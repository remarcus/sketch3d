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
pub const MAX_OBJ_TRIANGLE_TESTS: usize = 5_000_000;

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
    SavedCurvedGuide(u64),
    ObjReference(u64),
}

pub struct Draft {
    pub before: Vec<Stroke>,
    pub preview: Vec<Stroke>,
    pub target: TargetChoice,
    pub mode: ProjectionMode,
    pub offset: f32,
    pub resample_spacing: f32,
    pub keep_original: bool,
    pub attach_to_model: bool,
    pub convert_curves: bool,
    pub captured_camera: Camera,
    pub error: Option<String>,
    pub kept_original_samples: usize,
    target_guide_snapshot: Option<(u64, GuideGeometry)>,
    target_obj_snapshot: Option<crate::document::model_references::ObjReferenceResource>,
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
                    || stroke.model_surface.is_some()
                    || stroke.points.is_empty()
            })
        {
            return Err("Reprojection supports selected editable sampled strokes only; fills, linked artwork, model-bound dab strokes, and modifier stacks are unsupported.".into());
        }
        let count: usize = before.iter().map(|stroke| stroke.points.len()).sum();
        if count > MAX_POINTS {
            return Err(format!("Reprojection is limited to {MAX_POINTS} samples."));
        }
        if before.iter().any(|stroke| {
            !stroke.brush.is_valid()
                || !stroke.points.iter().all(|point| point.is_valid())
                || !stroke.point_thickness_is_valid()
                || !stroke.point_colors_are_valid()
        }) {
            return Err("Selected strokes contain invalid brush or point data.".into());
        }
        Ok(Self {
            preview: before.clone(),
            before,
            target: TargetChoice::Front,
            mode: ProjectionMode::Orthogonal,
            offset: 0.0,
            resample_spacing: 0.25,
            keep_original: false,
            attach_to_model: false,
            convert_curves: false,
            captured_camera: camera,
            error: None,
            kept_original_samples: 0,
            target_guide_snapshot: None,
            target_obj_snapshot: None,
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
            TargetChoice::SavedCurvedGuide(_) => {
                return Err(
                    "Choose a curved guide rebuild target instead of a planar target.".into(),
                );
            }
            TargetChoice::ObjReference(_) => {
                return Err("Choose an OBJ rebuild target instead of a planar target.".into());
            }
        })
    }

    pub fn rebuild_on_curved_guide(
        &mut self,
        doc: &Document,
        guide_id: u64,
        rect: Rect,
    ) -> Result<(), String> {
        let Some(guide) = doc
            .guide_resources
            .iter()
            .find(|guide| guide.id == guide_id)
        else {
            self.preview = self.before.clone();
            self.error = Some("Selected curved guide no longer exists".into());
            self.kept_original_samples = 0;
            self.target_guide_snapshot = None;
            self.target_obj_snapshot = None;
            return Err("Selected curved guide no longer exists".into());
        };
        let geometry = guide.geometry.clone();
        let result = match &geometry {
            GuideGeometry::Cylinder { .. } | GuideGeometry::Sphere { .. } => {
                self.build_curved_preview(&geometry, rect)
            }
            _ => Err("Reprojection currently supports saved Cylinder and Sphere guides.".into()),
        };
        match result {
            Ok((preview, kept_original_samples)) => {
                self.preview = preview;
                self.error = None;
                self.kept_original_samples = kept_original_samples;
                self.target_guide_snapshot = Some((guide_id, geometry));
                self.target_obj_snapshot = None;
                Ok(())
            }
            Err(error) => {
                self.preview = self.before.clone();
                self.error = Some(error.clone());
                self.kept_original_samples = 0;
                self.target_guide_snapshot = None;
                self.target_obj_snapshot = None;
                Err(error)
            }
        }
    }

    pub fn rebuild(&mut self, plane: DrawingPlane, rect: Rect) -> Result<(), String> {
        let result = self.build_preview(plane, rect);
        match result {
            Ok((preview, kept_original_samples)) => {
                self.preview = preview;
                self.error = None;
                self.kept_original_samples = kept_original_samples;
                self.target_guide_snapshot = None;
                self.target_obj_snapshot = None;
                Ok(())
            }
            Err(error) => {
                self.preview = self.before.clone();
                self.error = Some(error.clone());
                self.kept_original_samples = 0;
                self.target_guide_snapshot = None;
                self.target_obj_snapshot = None;
                Err(error)
            }
        }
    }

    pub fn rebuild_on_obj_reference(
        &mut self,
        resource: &crate::document::model_references::ObjReferenceResource,
        model: &crate::obj_reference::ObjModel,
        index: &crate::obj_mesh_index::ObjMeshIndex,
        rect: Rect,
    ) -> Result<(), String> {
        if let Err(error) = resource.validate() {
            self.reset_failed_preview(&error);
            return Err(error);
        }
        if !resource.visible {
            let error = "The selected OBJ target is hidden.".to_owned();
            self.reset_failed_preview(&error);
            return Err(error);
        }
        if self.attach_to_model
            && (resource.locked || model.source_format != crate::obj_reference::ObjSourceFormat::Obj)
        {
            let error = "Attachment requires a visible, unlocked OBJ reference.";
            self.reset_failed_preview(error);
            return Err(error.into());
        }
        let result = self.build_preview_with_target(
            ProjectionTarget::Obj(ObjProjectionTarget {
                resource,
                model,
                index,
            }),
            rect,
        );
        match result {
            Ok((preview, kept_original_samples)) => {
                self.preview = preview;
                self.error = None;
                self.kept_original_samples = kept_original_samples;
                self.target_guide_snapshot = None;
                self.target_obj_snapshot = Some(resource.clone());
                Ok(())
            }
            Err(error) => {
                self.reset_failed_preview(&error);
                Err(error)
            }
        }
    }

    fn reset_failed_preview(&mut self, error: &str) {
        self.preview = self.before.clone();
        self.error = Some(error.to_owned());
        self.kept_original_samples = 0;
        self.target_guide_snapshot = None;
        self.target_obj_snapshot = None;
    }

    pub fn reject_preview(&mut self, error: &str) {
        self.reset_failed_preview(error);
    }

    fn build_preview(
        &self,
        plane: DrawingPlane,
        rect: Rect,
    ) -> Result<(Vec<Stroke>, usize), String> {
        self.build_preview_with_target(ProjectionTarget::Plane(plane), rect)
    }

    fn build_curved_preview(
        &self,
        guide: &GuideGeometry,
        rect: Rect,
    ) -> Result<(Vec<Stroke>, usize), String> {
        self.build_preview_with_target(ProjectionTarget::Curved(guide), rect)
    }

    fn build_preview_with_target(
        &self,
        target: ProjectionTarget<'_>,
        rect: Rect,
    ) -> Result<(Vec<Stroke>, usize), String> {
        if !self.offset.is_finite() || !(-100.0..=100.0).contains(&self.offset) {
            return Err("Normal offset must be between −100 and 100 world units".into());
        }
        let conforming_target = matches!(
            target,
            ProjectionTarget::Curved(_) | ProjectionTarget::Obj(_)
        );
        let attach = self.attach_to_model && matches!(target, ProjectionTarget::Obj(_));
        if conforming_target
            && (!self.resample_spacing.is_finite()
                || !(0.05..=2.0).contains(&self.resample_spacing))
        {
            return Err("Maximum sample spacing must be between 0.05 and 2 world units".into());
        }
        let mut total = 0usize;
        for stroke in &self.before {
            total = total
                .checked_add(if conforming_target {
                    resampled_sample_count(&stroke.points, self.resample_spacing)?
                } else {
                    stroke.points.len()
                })
                .ok_or("Reprojection sample count overflow")?;
            if total > MAX_POINTS {
                return Err(format!(
                    "Surface reprojection would create more than {MAX_POINTS} samples; increase maximum sample spacing or select fewer strokes."
                ));
            }
        }
        let mut preview = Vec::with_capacity(self.before.len());
        let mut kept_original_samples = 0;
        let mut obj_triangle_tests = 0usize;
        'strokes: for source in &self.before {
            if attach
                && (source.curve.is_some() && !self.convert_curves
                    || source.brush.flow.enabled
                    || source.brush.dabs.enabled
                    || source.brush.tip_asset_id != 0
                    || source.brush.texture.anchor_mode
                        == crate::texture::TextureAnchorMode::CapturedPlanar)
            {
                preview.push(source.clone());
                continue;
            }
            let mut stroke = source.clone();
            if stroke.curve.is_some() && !self.convert_curves {
                return Err("Selected semantic curves need the explicit Convert curves option before reprojection.".into());
            }
            if stroke.curve.is_some() {
                stroke.curve = None;
            }
            if conforming_target {
                resample_stroke(&mut stroke, self.resample_spacing)?;
            }
            let mut first_surface_normal = None;
            let mut previous_mesh_triangle = None;
            let mut anchors = Vec::new();
            for point in &mut stroke.points {
                let original = point.position;
                let Some((projected, normal, mesh_hit)) =
                    self.project_sample(target, original, rect, &mut obj_triangle_tests)?
                else {
                    if attach {
                        preview.push(source.clone());
                        continue 'strokes;
                    }
                    if self.keep_original {
                        kept_original_samples += 1;
                        previous_mesh_triangle = None;
                        continue;
                    }
                    return Err(match target {
                        ProjectionTarget::Plane(_) => "A source sample could not be projected to the target plane.",
                        ProjectionTarget::Curved(_) => "A source sample misses the curved target or is ambiguous; enable Keep Original or choose another projection.",
                        ProjectionTarget::Obj(_) => "A source sample misses the OBJ target; enable Keep Original or choose another projection.",
                    }.into());
                };
                let mesh_triangle = mesh_hit.map(|hit| hit.triangle_index);
                if let (ProjectionTarget::Obj(target), Some(previous), Some(current)) =
                    (target, previous_mesh_triangle, mesh_triangle)
                {
                    let connected = if attach {
                        target.index.triangles_adjacent(previous, current)
                    } else {
                        target.index.triangles_connected(previous, current)
                    };
                    if !connected {
                        if attach {
                            preview.push(source.clone());
                            continue 'strokes;
                        }
                        if self.keep_original {
                            kept_original_samples += 1;
                            previous_mesh_triangle = Some(current);
                            continue;
                        }
                        return Err("Projection crosses to a disconnected OBJ component or seam; enable Keep Original or select a continuous region.".into());
                    }
                }
                let position = if attach {
                    projected
                } else {
                    projected + normal * self.offset
                };
                if !position.is_finite() || position.abs().max_element() > 10000.0 {
                    if attach {
                        preview.push(source.clone());
                        continue 'strokes;
                    }
                    return Err("Reprojected point exceeds the ±10000 world-unit limit".into());
                }
                if attach {
                    anchors.extend(
                        mesh_hit.map(crate::document::model_references::ModelInkAnchor::from),
                    );
                }
                point.position = position;
                first_surface_normal.get_or_insert(normal);
                previous_mesh_triangle = mesh_triangle;
            }
            if stroke.points.len() >= 2 {
                let length: f32 = stroke
                    .points
                    .windows(2)
                    .map(|pair| pair[0].position.distance(pair[1].position))
                    .sum();
                if !length.is_finite() || length <= 1.0e-6 {
                    if attach {
                        preview.push(source.clone());
                        continue 'strokes;
                    }
                    return Err("Projection collapses a stroke to a degenerate point; revise the target or keep the original samples.".into());
                }
            }
            match target {
                ProjectionTarget::Plane(plane) => {
                    let (tangent, _, normal) = plane.basis();
                    stroke.brush.normal = normal;
                    stroke.brush.tangent = tangent;
                }
                ProjectionTarget::Curved(_) | ProjectionTarget::Obj(_) => {
                    if let Some(normal) = first_surface_normal {
                        let tangent = stroke
                            .points
                            .windows(2)
                            .map(|pair| pair[1].position - pair[0].position)
                            .map(|delta| delta - normal * delta.dot(normal))
                            .find_map(Vec3::try_normalize)
                            .or_else(|| {
                                let tangent = stroke.brush.tangent;
                                (tangent - normal * tangent.dot(normal)).try_normalize()
                            })
                            .or_else(|| normal.cross(Vec3::X).try_normalize())
                            .or_else(|| normal.cross(Vec3::Y).try_normalize())
                            .unwrap_or(Vec3::Z);
                        stroke.brush.normal = normal;
                        stroke.brush.tangent = tangent;
                    }
                }
            }
            if !stroke.brush.is_valid() {
                return Err("Projected brush frame is invalid".into());
            }
            if let (true, ProjectionTarget::Obj(target)) = (attach, target) {
                stroke.guide_attachment = None;
                stroke.model_surface = Some(crate::document::ModelSurfacePatch {
                    triangles: Vec::new(),
                    attachment: Some(crate::document::model_references::ModelInkAttachment {
                        model_id: target.resource.id,
                        anchors,
                    }),
                });
            }
            preview.push(stroke);
        }
        Ok((preview, kept_original_samples))
    }

    fn project_sample(
        &self,
        target: ProjectionTarget<'_>,
        original: Vec3,
        rect: Rect,
        obj_triangle_tests: &mut usize,
    ) -> Result<Option<(Vec3, Vec3, Option<crate::obj_mesh_index::ObjMeshHit>)>, String> {
        match target {
            ProjectionTarget::Plane(plane) => {
                let (_, _, normal) = plane.basis();
                let origin = plane.origin();
                let projected = match self.mode {
                    ProjectionMode::Orthogonal => {
                        original - normal * (original - origin).dot(normal)
                    }
                    ProjectionMode::CapturedViewRay => {
                        let Some((screen, _)) =
                            crate::gizmo::project(&self.captured_camera, rect, original)
                        else {
                            return Ok(None);
                        };
                        let (ray_origin, ray_direction) =
                            crate::gizmo::ray(&self.captured_camera, rect, screen);
                        let denominator = ray_direction.dot(normal);
                        if denominator.abs() < 1.0e-6 {
                            return Err(
                                "A captured view ray is parallel to the target plane.".into()
                            );
                        }
                        let distance = (origin - ray_origin).dot(normal) / denominator;
                        if !distance.is_finite() || distance < 0.0 {
                            return Err("A captured view ray hits behind its camera; choose another target.".into());
                        }
                        ray_origin + ray_direction * distance
                    }
                };
                Ok(Some((projected, normal, None)))
            }
            ProjectionTarget::Curved(guide) => {
                let projected = match self.mode {
                    ProjectionMode::Orthogonal => orthogonal_guide_projection(guide, original),
                    ProjectionMode::CapturedViewRay => {
                        let Some((screen, _)) =
                            crate::gizmo::project(&self.captured_camera, rect, original)
                        else {
                            return Ok(None);
                        };
                        let (ray_origin, ray_direction) =
                            crate::gizmo::ray(&self.captured_camera, rect, screen);
                        guide.clone().hit(ray_origin, ray_direction)
                    }
                };
                let Some(projected) = projected else {
                    return Ok(None);
                };
                let Some(normal) = guide_normal(guide, projected) else {
                    return Ok(None);
                };
                Ok(Some((projected, normal, None)))
            }
            ProjectionTarget::Obj(target) => {
                let inverse = target.resource.rotation.inverse();
                let hit = match self.mode {
                    ProjectionMode::Orthogonal => {
                        let local_point =
                            inverse * (original - target.resource.center) / target.resource.scale;
                        let remaining = MAX_OBJ_TRIANGLE_TESTS.saturating_sub(*obj_triangle_tests);
                        let (hit, tested) = target.index.closest_point_bounded(
                            target.model,
                            local_point,
                            remaining,
                            self.attach_to_model,
                        )?;
                        *obj_triangle_tests = obj_triangle_tests
                            .checked_add(tested)
                            .ok_or("OBJ projection query count overflow")?;
                        hit
                    }
                    ProjectionMode::CapturedViewRay => {
                        let Some((screen, _)) =
                            crate::gizmo::project(&self.captured_camera, rect, original)
                        else {
                            return Ok(None);
                        };
                        let (ray_origin, ray_direction) =
                            crate::gizmo::ray(&self.captured_camera, rect, screen);
                        let local_origin =
                            inverse * (ray_origin - target.resource.center) / target.resource.scale;
                        let local_direction = inverse * ray_direction / target.resource.scale;
                        let remaining = MAX_OBJ_TRIANGLE_TESTS.saturating_sub(*obj_triangle_tests);
                        let (hit, tested) = target.index.hit_bounded(
                            target.model,
                            local_origin,
                            local_direction,
                            remaining,
                        )?;
                        *obj_triangle_tests = obj_triangle_tests
                            .checked_add(tested)
                            .ok_or("OBJ projection query count overflow")?;
                        hit
                    }
                };
                let Some(hit) = hit else {
                    return Ok(None);
                };
                let world_position = target.resource.center
                    + target.resource.rotation * (hit.position * target.resource.scale);
                let world_normal = (target.resource.rotation * hit.normal)
                    .try_normalize()
                    .ok_or("OBJ surface returned an invalid normal")?;
                Ok(Some((
                    world_position,
                    world_normal,
                    Some(hit),
                )))
            }
        }
    }

    pub fn edit(&self, doc: &Document) -> Result<Edit, String> {
        if self.preview == self.before {
            return Err("Reprojection produces no changes.".into());
        }
        if self.preview.iter().any(|stroke| stroke.attached_model().is_some())
            && (!doc.animation.is_empty() || doc.animation.two_bone_rig.is_some())
        {
            return Err("Model attachment requires static artwork.".into());
        }
        if let Some((guide_id, expected_geometry)) = &self.target_guide_snapshot {
            let target_is_unchanged = matches!(self.target, TargetChoice::SavedCurvedGuide(id) if id == *guide_id)
                && doc
                    .guide_resources
                    .iter()
                    .find(|guide| guide.id == *guide_id)
                    .is_some_and(|guide| guide.geometry == *expected_geometry);
            if !target_is_unchanged {
                return Err(
                    "The curved target changed or disappeared; reopen the reprojection preview."
                        .into(),
                );
            }
        }
        if let Some(expected_resource) = &self.target_obj_snapshot {
            let target_is_unchanged = matches!(
                self.target,
                TargetChoice::ObjReference(id) if id == expected_resource.id
            ) && doc
                .model_references
                .iter()
                .find(|resource| resource.id == expected_resource.id)
                .is_some_and(|resource| resource == expected_resource);
            if !target_is_unchanged {
                return Err(
                    "The OBJ target changed or disappeared; reopen the reprojection preview."
                        .into(),
                );
            }
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

#[derive(Clone, Copy)]
enum ProjectionTarget<'a> {
    Plane(DrawingPlane),
    Curved(&'a GuideGeometry),
    Obj(ObjProjectionTarget<'a>),
}

#[derive(Clone, Copy)]
struct ObjProjectionTarget<'a> {
    resource: &'a crate::document::model_references::ObjReferenceResource,
    model: &'a crate::obj_reference::ObjModel,
    index: &'a crate::obj_mesh_index::ObjMeshIndex,
}

fn orthogonal_guide_projection(guide: &GuideGeometry, point: Vec3) -> Option<Vec3> {
    match guide {
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } => {
            let local = rotation.inverse() * (point - *center);
            let radial = Vec3::new(local.x, 0.0, local.z).try_normalize()?;
            (local.y.abs() <= *height * 0.5).then_some(
                *center + *rotation * Vec3::new(radial.x * *radius, local.y, radial.z * *radius),
            )
        }
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => {
            let local = rotation.inverse() * (point - *center);
            let direction = local.try_normalize()?;
            Some(*center + *rotation * direction * *radius)
        }
        _ => None,
    }
}

fn guide_normal(guide: &GuideGeometry, point: Vec3) -> Option<Vec3> {
    match guide {
        GuideGeometry::Cylinder {
            center, rotation, ..
        } => {
            let local = rotation.inverse() * (point - *center);
            (*rotation * Vec3::new(local.x, 0.0, local.z).try_normalize()?).try_normalize()
        }
        GuideGeometry::Sphere {
            center, rotation, ..
        } => {
            (*rotation * (rotation.inverse() * (point - *center)).try_normalize()?).try_normalize()
        }
        _ => None,
    }
}

fn resampled_sample_count(
    points: &[crate::document::Point],
    spacing: f32,
) -> Result<usize, String> {
    if points.is_empty() {
        return Ok(0);
    }
    let mut count = 1usize;
    for pair in points.windows(2) {
        let length = pair[0].position.distance(pair[1].position);
        let segments = (length / spacing).ceil().max(1.0);
        if !segments.is_finite() || segments > MAX_POINTS as f32 {
            return Err("Surface reprojection exceeds its sample budget".into());
        }
        count = count
            .checked_add(segments as usize)
            .ok_or("Reprojection sample count overflow")?;
        if count > MAX_POINTS {
            return Err(format!(
                "Surface reprojection would create more than {MAX_POINTS} samples; increase maximum sample spacing or select fewer strokes."
            ));
        }
    }
    Ok(count)
}

fn resample_stroke(stroke: &mut Stroke, spacing: f32) -> Result<(), String> {
    let output_count = resampled_sample_count(&stroke.points, spacing)?;
    if output_count == stroke.points.len() || stroke.points.len() <= 1 {
        return Ok(());
    }
    let source_points = stroke.points.clone();
    let source_thickness = stroke.point_thickness.clone();
    let source_colors = stroke.point_colors.clone();
    let distances = crate::dynamics::arc_lengths(&source_points);
    let total_length = distances.last().copied().unwrap_or(0.0);
    let mut points = Vec::with_capacity(output_count);
    let mut thickness = source_thickness
        .as_ref()
        .map(|_| Vec::with_capacity(output_count));
    let mut colors = source_colors
        .as_ref()
        .map(|_| Vec::with_capacity(output_count));

    for index in 0..source_points.len() - 1 {
        points.push(source_points[index]);
        if let Some(values) = &mut thickness {
            values.push(source_thickness.as_ref().unwrap()[index]);
        }
        if let Some(values) = &mut colors {
            values.push(source_colors.as_ref().unwrap()[index]);
        }
        let start = source_points[index];
        let end = source_points[index + 1];
        let length = start.position.distance(end.position);
        let segment_count = ((length / spacing).ceil().max(1.0) as usize).min(MAX_POINTS);
        for segment in 1..=segment_count {
            let amount = segment as f32 / segment_count as f32;
            if segment == segment_count {
                points.push(end);
                if let Some(values) = &mut thickness {
                    values.push(source_thickness.as_ref().unwrap()[index + 1]);
                }
                if let Some(values) = &mut colors {
                    values.push(source_colors.as_ref().unwrap()[index + 1]);
                }
                continue;
            }
            points.push(crate::document::Point {
                position: start.position.lerp(end.position, amount),
                pressure: start.pressure + (end.pressure - start.pressure) * amount,
                tilt: crate::dynamics::interpolate_tilt(start.tilt, end.tilt, amount),
                time_ms: (start.time_ms as f32
                    + (end.time_ms as f32 - start.time_ms as f32) * amount)
                    .round() as u32,
            });
            if let Some(values) = &mut thickness {
                let start_scale = source_thickness.as_ref().unwrap()[index];
                let end_scale = source_thickness.as_ref().unwrap()[index + 1];
                values.push(start_scale + (end_scale - start_scale) * amount);
            }
            if let Some(values) = &mut colors {
                let source_colors = source_colors.as_ref().unwrap();
                let left = source_colors[index];
                let right = source_colors[index + 1];
                let color = match (left, right) {
                    (None, None) => None,
                    _ => {
                        let start_t = if total_length > 1.0e-8 {
                            distances[index] / total_length
                        } else {
                            0.0
                        };
                        let end_t = if total_length > 1.0e-8 {
                            distances[index + 1] / total_length
                        } else {
                            0.0
                        };
                        let start_color = left.unwrap_or_else(|| stroke.base_color_at(start_t));
                        let end_color = right.unwrap_or_else(|| stroke.base_color_at(end_t));
                        let resolved = std::array::from_fn(|channel| {
                            start_color[channel]
                                + (end_color[channel] - start_color[channel]) * amount
                        });
                        let sample_t = start_t + (end_t - start_t) * amount;
                        let base = stroke.base_color_at(sample_t);
                        (!resolved
                            .iter()
                            .zip(base)
                            .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-5))
                        .then_some(resolved)
                    }
                };
                values.push(color);
            }
        }
    }
    stroke.points = points;
    stroke.point_thickness = thickness;
    stroke.point_colors = colors;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{guide_resources::GuideResource, Brush, Point};
    use glam::Quat;

    fn sampled_stroke(id: u64) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id,
            color: [0.31, 0.52, 0.77],
            gradient: None,
            radius: 0.04,
            brush: Brush::default(),
            points: [
                (Vec3::new(-1.0, 0.4, 0.7), 0.2, 11),
                (Vec3::new(0.0, 1.2, -0.3), 0.8, 22),
                (Vec3::new(1.0, 0.7, 0.5), 0.6, 33),
            ]
            .into_iter()
            .map(|(position, pressure, time_ms)| Point {
                position,
                pressure,
                tilt: Some(Vec3::new(0.1, -0.2, 0.0)),
                time_ms,
            })
            .collect(),
        }
    }

    #[test]
    fn rotated_saved_plane_projection_preserves_samples_and_is_one_undoable_edit() {
        let mut doc = Document::default();
        let source = sampled_stroke(7);
        doc.strokes.push(source.clone());
        let rotation = Quat::from_rotation_y(0.42) * Quat::from_rotation_x(-0.2);
        doc.guide_resources.push(GuideResource {
            id: 19,
            name: "Offset target".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Plane {
                origin: Vec3::new(0.4, -0.3, 0.8),
                rotation,
            },
        });
        let original = doc.strokes[0].clone();
        let mut draft = Draft::new(&doc, &HashSet::from([source.id]), Camera::default()).unwrap();
        draft.target = TargetChoice::SavedPlane(19);
        draft.offset = 0.12;
        let plane = draft.target_plane(&doc, None).unwrap();
        let rect = Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(900.0, 700.0));
        draft.rebuild(plane, rect).unwrap();
        let (tangent, _, normal) = plane.basis();
        let origin = plane.origin();
        let projected = draft.preview[0].clone();
        for (before, after) in original.points.iter().zip(&projected.points) {
            let expected = before.position - normal * (before.position - origin).dot(normal)
                + normal * draft.offset;
            assert!(after.position.distance(expected) < 1e-5);
            assert_eq!(after.pressure, before.pressure);
            assert_eq!(after.tilt, before.tilt);
            assert_eq!(after.time_ms, before.time_ms);
        }
        assert_eq!(projected.color, original.color);
        assert_eq!(projected.radius, original.radius);
        assert_eq!(projected.brush.normal, normal);
        assert_eq!(projected.brush.tangent, tangent);

        let edit = draft.edit(&doc).unwrap();
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], projected);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes[0], original);
        edit.apply(&mut doc, true);
        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes[0], doc.strokes[0]);
        let glb = crate::export::encode(&reopened, Some(&HashSet::from([source.id]))).unwrap();
        assert_eq!(glb.summary.strokes, 1);
    }

    #[test]
    fn captured_view_rays_project_in_perspective_and_orthographic_modes() {
        let rect = Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(900.0, 700.0));
        for projection in [
            crate::geometry::Projection::Perspective,
            crate::geometry::Projection::Orthographic,
        ] {
            let camera = Camera {
                projection,
                yaw: 0.42,
                pitch: 0.24,
                distance: 5.0,
                target: Vec3::ZERO,
                ..Camera::default()
            };
            let mut doc = Document::default();
            let source = sampled_stroke(8);
            doc.strokes.push(source.clone());
            let mut draft = Draft::new(&doc, &HashSet::from([source.id]), camera).unwrap();
            draft.target = TargetChoice::CapturedView;
            draft.mode = ProjectionMode::CapturedViewRay;
            let plane = draft.target_plane(&doc, None).unwrap();
            draft.rebuild(plane, rect).unwrap();
            let normal = plane.basis().2;
            let origin = plane.origin();
            assert!(draft.preview[0]
                .points
                .iter()
                .all(|point| { (point.position - origin).dot(normal).abs() < 1e-4 }));
            assert_eq!(draft.preview[0].points.len(), source.points.len());
        }
    }

    #[test]
    fn captured_view_rays_reject_parallel_and_backward_plane_hits() {
        let camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 5.0,
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let rect = Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(900.0, 700.0));
        let source = sampled_stroke(9);
        let mut doc = Document::default();
        doc.strokes.push(source.clone());
        let mut draft = Draft::new(&doc, &HashSet::from([source.id]), camera).unwrap();
        draft.mode = ProjectionMode::CapturedViewRay;
        let mut tests = 0;
        let side = DrawingPlane::from_preset(Plane::Side);
        assert!(draft
            .project_sample(
                ProjectionTarget::Plane(side),
                Vec3::new(0.0, 0.0, 1.0),
                rect,
                &mut tests
            )
            .is_err());

        let behind = DrawingPlane {
            anchor: Vec3::new(0.0, 0.0, 10.0),
            rotation_degrees: Vec3::ZERO,
            depth: 0.0,
        };
        assert!(draft
            .project_sample(
                ProjectionTarget::Plane(behind),
                Vec3::new(0.0, 0.0, 1.0),
                rect,
                &mut tests
            )
            .is_err());
    }
}
