//! Bounded detached stroke projection onto planar drawing guides.
use crate::{
    document::{guide_resources::GuideGeometry, Document, Stroke},
    editing::{snapshot, Edit},
    geometry::{Camera, DrawingPlane, Plane},
};
use eframe::egui::Rect;
use glam::Vec3;
use std::collections::HashSet;

pub const MAX_STROKES: usize = 128;
pub const MAX_POINTS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionMode {
    Orthogonal,
    CapturedViewRay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetChoice {
    Front,
    Side,
    Ground,
    ActivePlanarGuide,
    CapturedView,
    SavedPlane(u64),
}

pub struct Draft {
    pub before: Vec<Stroke>,
    pub preview: Vec<Stroke>,
    pub target: TargetChoice,
    pub mode: ProjectionMode,
    pub offset: f32,
    pub keep_original: bool,
    pub convert_curves: bool,
    pub captured_camera: Camera,
    pub error: Option<String>,
}

impl Draft {
    pub fn new(doc: &Document, ids: &HashSet<u64>, camera: Camera) -> Result<Self, String> {
        if ids.is_empty() || ids.len() > MAX_STROKES {
            return Err(format!("Select 1–{MAX_STROKES} strokes to reproject."));
        }
        let before: Vec<_> = doc
            .strokes
            .iter()
            .filter(|stroke| ids.contains(&stroke.id))
            .cloned()
            .collect();
        if before.len() != ids.len()
            || before.iter().any(|stroke| {
                !doc.editable(stroke)
                    || stroke.fill.is_some()
                    || doc.has_linked_source(stroke.id)
                    || doc.linked_instance_for_stroke(stroke.id).is_some()
                    || stroke.modifiers.is_some()
                    || stroke.points.is_empty()
            })
        {
            return Err("Reprojection supports selected editable sampled strokes only; fills, linked artwork, and modifier stacks are unsupported.".into());
        }
        let count: usize = before.iter().map(|stroke| stroke.points.len()).sum();
        if count > MAX_POINTS {
            return Err(format!("Reprojection is limited to {MAX_POINTS} samples."));
        }
        if before.iter().any(|stroke| {
            !stroke.brush.is_valid() || !stroke.points.iter().all(|point| point.is_valid())
        }) {
            return Err("Selected strokes contain invalid brush or point data.".into());
        }
        Ok(Self {
            preview: before.clone(),
            before,
            target: TargetChoice::Front,
            mode: ProjectionMode::Orthogonal,
            offset: 0.0,
            keep_original: false,
            convert_curves: false,
            captured_camera: camera,
            error: None,
        })
    }

    pub fn target_plane(
        &self,
        doc: &Document,
        active: Option<DrawingPlane>,
    ) -> Result<DrawingPlane, String> {
        Ok(match self.target {
            TargetChoice::Front => DrawingPlane::from_preset(Plane::Front),
            TargetChoice::Side => DrawingPlane::from_preset(Plane::Side),
            TargetChoice::Ground => DrawingPlane::from_preset(Plane::Ground),
            TargetChoice::ActivePlanarGuide => {
                active.ok_or("Choose an active planar guide first")?
            }
            TargetChoice::CapturedView => DrawingPlane::from_view(&self.captured_camera),
            TargetChoice::SavedPlane(id) => {
                let guide = doc
                    .guide_resources
                    .iter()
                    .find(|guide| guide.id == id)
                    .ok_or("Selected saved plane no longer exists")?;
                let GuideGeometry::Plane { origin, rotation } = &guide.geometry else {
                    return Err("Saved target must be a planar guide".into());
                };
                let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
                DrawingPlane {
                    anchor: *origin,
                    rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
                    depth: 0.0,
                }
            }
        })
    }

    pub fn rebuild(&mut self, plane: DrawingPlane, rect: Rect) -> Result<(), String> {
        let result = self.build_preview(plane, rect);
        match result {
            Ok(preview) => {
                self.preview = preview;
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.preview = self.before.clone();
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn build_preview(&self, plane: DrawingPlane, rect: Rect) -> Result<Vec<Stroke>, String> {
        if !self.offset.is_finite() || !(-100.0..=100.0).contains(&self.offset) {
            return Err("Normal offset must be between −100 and 100 world units".into());
        }
        let (u, _, n) = plane.basis();
        let origin = plane.origin();
        let camera = self.captured_camera;
        let mut preview = self.before.clone();
        let mut total = 0usize;
        for stroke in &mut preview {
            if stroke.curve.is_some() && !self.convert_curves {
                return Err("Selected semantic curves need the explicit Convert curves option before reprojection.".into());
            }
            if stroke.curve.is_some() {
                stroke.curve = None;
            }
            total = total.saturating_add(stroke.points.len());
            if total > MAX_POINTS {
                return Err("Reprojection exceeds its sample budget".into());
            }
            for point in &mut stroke.points {
                let original = point.position;
                let projected = match self.mode {
                    ProjectionMode::Orthogonal => original - n * (original - origin).dot(n),
                    ProjectionMode::CapturedViewRay => {
                        let Some((screen, _)) = crate::gizmo::project(&camera, rect, original)
                        else {
                            if self.keep_original {
                                continue;
                            }
                            return Err("A source sample is outside the captured view; enable Keep Original or adjust the view.".into());
                        };
                        let (ray_origin, ray_direction) = crate::gizmo::ray(&camera, rect, screen);
                        let denominator = ray_direction.dot(n);
                        if denominator.abs() < 1.0e-6 {
                            return Err(
                                "A captured view ray is parallel to the target plane.".into()
                            );
                        }
                        let distance = (origin - ray_origin).dot(n) / denominator;
                        if !distance.is_finite() || distance < 0.0 {
                            return Err("A captured view ray hits behind its camera; choose another target.".into());
                        }
                        ray_origin + ray_direction * distance
                    }
                };
                let position = projected + n * self.offset;
                if !position.is_finite() || position.abs().max_element() > 10000.0 {
                    return Err("Reprojected point exceeds the ±10000 world-unit limit".into());
                }
                point.position = position;
            }
            if stroke.points.len() >= 2 {
                let length: f32 = stroke
                    .points
                    .windows(2)
                    .map(|pair| pair[0].position.distance(pair[1].position))
                    .sum();
                if !length.is_finite() || length <= 1.0e-6 {
                    return Err("Projection collapses a stroke to a degenerate point; revise the target or keep the original samples.".into());
                }
            }
            stroke.brush.normal = n;
            stroke.brush.tangent = u;
            if !stroke.brush.is_valid() {
                return Err("Projected brush frame is invalid".into());
            }
        }
        Ok(preview)
    }

    pub fn edit(&self, doc: &Document) -> Result<Edit, String> {
        if self.preview == self.before {
            return Err("Reprojection produces no changes.".into());
        }
        for source in &self.before {
            if !doc.editable(source) || !doc.strokes.iter().any(|stroke| stroke == source) {
                return Err(
                    "A source stroke changed or became protected; reopen the reprojection preview."
                        .into(),
                );
            }
        }
        let ids: HashSet<_> = self.before.iter().map(|stroke| stroke.id).collect();
        let before = snapshot(doc, &ids);
        let mut after = before.clone();
        for (_, stroke) in &mut after {
            if let Some(projected) = self
                .preview
                .iter()
                .find(|candidate| candidate.id == stroke.id)
            {
                *stroke = projected.clone();
            }
        }
        Ok(Edit::Change { before, after })
    }
}
