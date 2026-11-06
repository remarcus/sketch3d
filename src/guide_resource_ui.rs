use super::*;
use crate::{
    document::guide_resources::{
        FreeformSurface, GuideGeometry, GuideResource, LoftSection, LoftSurface,
        ProfilePoint as SavedProfilePoint,
    },
    revolved::{ProfilePoint, Revolved},
    surface::{ConeSurface, Cylinder, GuidePreview, RingSurface, SphereSurface, Surface},
};

type GuideEditState = (
    Vec<GuideResource>,
    Option<u64>,
    Vec<crate::image_planes::ImagePlane>,
    Option<u64>,
    Option<u64>,
);

#[derive(Clone, Copy, PartialEq)]
struct AttachmentFrame {
    origin: Vec3,
    rotation: glam::Quat,
    cylinder: Option<(f32, f32)>,
}

fn attachment_frame(guides: &[GuideResource], id: u64) -> Option<AttachmentFrame> {
    let guide = guides.iter().find(|guide| guide.id == id)?;
    match guide.geometry {
        GuideGeometry::Plane { origin, rotation } => Some(AttachmentFrame {
            origin,
            rotation,
            cylinder: None,
        }),
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } => Some(AttachmentFrame {
            origin: center,
            rotation,
            cylinder: Some((radius, height)),
        }),
        _ => None,
    }
}

impl AttachmentFrame {
    fn cylinder_point(
        self,
        position: Vec3,
        next: Self,
        radius_change: f32,
        next_height: f32,
    ) -> Result<Vec3, &'static str> {
        let local = self.rotation.inverse() * (position - self.origin);
        if local.y.abs() > next_height * 0.5 + 1e-5 {
            return Err("The cylinder would be too short for its attached ink. Move or detach that ink first.");
        }
        let radial = Vec3::new(local.x, 0.0, local.z);
        let distance = radial.length();
        let next_distance = distance + radius_change;
        if distance <= 1e-6 || next_distance <= 1e-6 {
            return Err(
                "Attached ink cannot cross the cylinder axis. Move or detach that ink first.",
            );
        }
        // Radial vectors preserve angle continuously across the angular seam.
        // Keep axial position and the signed offset from the cylinder surface.
        let next_local = radial * (next_distance / distance) + Vec3::Y * local.y;
        Ok(next.origin + next.rotation * next_local)
    }
}

fn transform_attached_stroke(
    index: usize,
    stroke: &crate::document::Stroke,
    old: Option<AttachmentFrame>,
    new: Option<AttachmentFrame>,
) -> Result<crate::document::Stroke, &'static str> {
    let mut changed = stroke.clone();
    if let Some((old, new)) = old
        .zip(new)
        .filter(|(old, new)| old.cylinder.is_some() == new.cylinder.is_some())
    {
        let rotation = if old.rotation == new.rotation {
            glam::Quat::IDENTITY
        } else {
            new.rotation * old.rotation.inverse()
        };
        let mut transformed = crate::editing::transformed(
            &vec![(index, changed)],
            old.origin,
            new.origin - old.origin,
            rotation,
            1.0,
        )?;
        changed = transformed.pop().expect("one transformed stroke").1;
        if let (Some((radius, _)), Some((next_radius, next_height))) = (old.cylinder, new.cylinder)
        {
            if matches!(
                changed.brush.dabs.surface_target,
                Some(crate::dynamics::DabSurfaceTarget::Cylinder { .. })
            ) {
                changed.brush.dabs.surface_target = Some(crate::dynamics::DabSurfaceTarget::Cylinder {
                    center: new.origin,
                    rotation: new.rotation,
                    radius: next_radius,
                    height: next_height,
                });
            }
            for (point, source) in changed.points.iter_mut().zip(&stroke.points) {
                let position =
                    old.cylinder_point(source.position, new, next_radius - radius, next_height)?;
                if next_radius != radius {
                    point.position = position;
                }
                if !point.is_valid() {
                    return Err("Cylinder edit moves attached ink outside the drawing bounds.");
                }
            }
            if !changed.brush.is_valid()
                || strand_vertex_budget_exceeded(&changed)
                || airbrush_vertex_budget_exceeded(&changed)
                || profile_tube_vertex_budget_exceeded(&changed)
            {
                return Err("Cylinder edit would exceed attached ink geometry limits.");
            }
        }
    } else {
        changed.guide_attachment = None;
    }
    Ok(changed)
}

fn reshape_attached_stroke(
    stroke: &Stroke,
    old: &GuideGeometry,
    new: &GuideGeometry,
) -> Result<Stroke, &'static str> {
    let mut changed = stroke.clone();
    for (index, point) in changed.points.iter_mut().enumerate() {
        let (u, v) = old.attachment_coordinates(point.position)
            .ok_or("An attached sample is off its guide. Detach the ink before reshaping.")?;
        let (previous, old_tangent, old_normal) = old.attachment_sample(u, v)
            .ok_or("The previous guide has no attachment frame.")?;
        let tolerance = 0.00015 + point.position.abs().max_element() * f32::EPSILON * 16.0;
        if previous.distance(point.position) > tolerance {
            return Err("An attached sample is off its guide. Detach the ink before reshaping.");
        }
        let (position, tangent, normal) = new.attachment_sample(u, v)
            .ok_or("The revised guide has no attachment frame.")?;
        let basis = |t: Vec3, n: Vec3| glam::Mat3::from_cols(t, n.cross(t), n);
        let old_basis = basis(old_tangent, old_normal);
        let new_basis = basis(tangent, normal);
        if old_basis.determinant() < 0.99 || new_basis.determinant() < 0.99 {
            return Err("The revised guide would collapse an attached ink frame.");
        }
        let rotation = new_basis * old_basis.transpose();
        point.position = position;
        point.tilt = point.tilt.map(|tilt| rotation * tilt);
        if !point.is_valid() {
            return Err("The guide edit would move attached ink outside the drawing bounds.");
        }
        if index == 0 {
            changed.brush.normal = (rotation * changed.brush.normal).normalize_or_zero();
            changed.brush.tangent = (rotation * changed.brush.tangent).normalize_or_zero();
        }
    }
    if !changed.brush.is_valid()
        || strand_vertex_budget_exceeded(&changed)
        || profile_tube_vertex_budget_exceeded(&changed)
    {
        return Err("The guide edit would exceed attached ink geometry limits.");
    }
    Ok(changed)
}

type GuideInkEdit = Box<(crate::editing::Snapshot, crate::editing::Snapshot)>;

fn attached_guide_changes(
    doc: &Document,
    before: &[GuideResource],
    after: &[GuideResource],
) -> Result<Option<GuideInkEdit>, &'static str> {
    let mut ink_before = Vec::new();
    let mut ink_after = Vec::new();
    let mut reshape_samples = 0;
    let mut dab_delta = 0_i64;
    for (index, stroke) in doc.strokes.iter().enumerate() {
        let Some(id) = stroke.guide_attachment else { continue };
        let old = before.iter().find(|guide| guide.id == id).map(|guide| &guide.geometry);
        let new = after.iter().find(|guide| guide.id == id).map(|guide| &guide.geometry);
        if old == new {
            continue;
        }
        if !doc.editable(stroke) {
            return Err("Unlock and show the attached ink before changing its guide.");
        }
        let parametric = old.into_iter().chain(new).any(GuideGeometry::supports_parametric_attachment);
        let changed = match (old, new) {
            (Some(old), Some(new)) if parametric => {
                if !old.attachment_compatible_with(new) {
                    return Err("Detach the ink before changing the guide's parameterization.");
                }
                reshape_samples += stroke.points.len();
                if reshape_samples > 65_536 {
                    return Err("This edit exceeds the 65,536-sample attached ink reshape limit.");
                }
                reshape_attached_stroke(stroke, old, new)?
            }
            _ => transform_attached_stroke(
                index, stroke, attachment_frame(before, id), attachment_frame(after, id),
            )?,
        };
        if changed.brush.dabs.enabled {
            let before = crate::dab_placement::point_count(stroke)
                .map_err(|_| "Attached ink exceeds the dab geometry limit.")?;
            let after = crate::dab_placement::point_count(&changed)
                .map_err(|_| "Guide edit would exceed the dab geometry limit.")?;
            dab_delta += after as i64 - before as i64;
        }
        ink_before.push((index, stroke.clone()));
        ink_after.push((index, changed));
    }
    if dab_delta > 0 && crate::dab_placement::document_count(doc)
        .map_err(|_| "Attached ink exceeds the document dab limit.")? + dab_delta as usize
        > crate::dab_placement::MAX_DOCUMENT_DABS
    {
        return Err("Guide edit would exceed the document dab limit.");
    }
    Ok((!ink_before.is_empty()).then(|| Box::new((ink_before, ink_after))))
}

pub(super) struct GuideBendDraft {
    pub guide_id: u64,
    pub center: Vec3,
    pub rotation: glam::Quat,
    pub depth: f32,
    pub bend: f32,
    pub controls: Vec<glam::Vec2>,
    pub preview: Option<FreeformSurface>,
    pub error: Option<String>,
}

impl GuideBendDraft {
    fn from_guide(guide_id: u64, geometry: &GuideGeometry) -> Result<Self, String> {
        let (center, rotation, depth, bend) = match geometry {
            GuideGeometry::Plane { origin, rotation } => (*origin, *rotation, 4.0, 0.0),
            GuideGeometry::Freeform { surface } => (
                surface.center,
                surface.rotation,
                surface.depth,
                surface.bend,
            ),
            _ => return Err("Bending currently supports saved Plane and Freeform guides.".into()),
        };
        if !center.is_finite()
            || !rotation.is_finite()
            || (rotation.length_squared() - 1.0).abs() > 0.001
        {
            return Err("The source guide transform is invalid.".into());
        }
        Ok(Self {
            guide_id,
            center,
            rotation,
            depth,
            bend,
            controls: Vec::with_capacity(4),
            preview: None,
            error: None,
        })
    }

    fn push_world_control(&mut self, world: Vec3) -> Result<(), String> {
        if self.controls.len() >= 4 {
            return Err("Four cubic control points are already set.".into());
        }
        if !world.is_finite()
            || world.abs().max_element()
                > crate::document::guide_resources::freeform_surface::MAX_COORDINATE
        {
            return Err("Bend control is outside the supported world bounds.".into());
        }
        let local = self.rotation.inverse() * (world - self.center);
        if !local.is_finite() {
            return Err("Bend control must be finite.".into());
        }
        self.controls.push(local.truncate());
        self.preview = None;
        self.error = None;
        if self.controls.len() == 4 {
            match self.build_surface() {
                Ok(surface) => self.preview = Some(surface),
                Err(error) => self.error = Some(error.clone()),
            }
        }
        Ok(())
    }

    pub fn build_surface(&self) -> Result<FreeformSurface, String> {
        if self.controls.len() != 4 {
            return Err("Click four control points: start, two handles, and end.".into());
        }
        let chord = self.controls[3] - self.controls[0];
        if chord.length_squared() < 0.0001 {
            return Err("Bend endpoints must be at least 0.01 units apart.".into());
        }
        let angle = chord.y.atan2(chord.x);
        let align = glam::Quat::from_rotation_z(angle);
        let inverse = align.inverse();
        let directrix = std::array::from_fn(|index| {
            inverse
                .mul_vec3(Vec3::new(
                    self.controls[index].x,
                    self.controls[index].y,
                    0.0,
                ))
                .truncate()
        });
        let surface = FreeformSurface {
            center: self.center,
            rotation: self.rotation * align,
            directrix,
            depth: self.depth,
            bend: self.bend,
        };
        surface.validate()?;
        let extent = surface.directrix[3].x - surface.directrix[0].x;
        if !extent.is_finite()
            || extent > crate::document::guide_resources::freeform_surface::MAX_COORDINATE
        {
            return Err("Bend path exceeds the supported surface extent.".into());
        }
        Ok(surface)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PlanarTarget {
    pub plane: DrawingPlane,
    half_extent: Option<glam::Vec2>,
}

impl PlanarTarget {
    pub fn from_curve_frame(
        origin: Vec3,
        normal: Vec3,
        tangent: Vec3,
        half_extent: Option<glam::Vec2>,
    ) -> Option<Self> {
        if !origin.is_finite()
            || !normal.is_finite()
            || !tangent.is_finite()
            || (normal.length_squared() - 1.0).abs() > 0.001
            || (tangent.length_squared() - 1.0).abs() > 0.001
            || normal.dot(tangent).abs() > 0.001
            || half_extent.is_some_and(|extent| !extent.is_finite() || extent.min_element() <= 0.0)
        {
            return None;
        }
        let bitangent = normal.cross(tangent).normalize_or_zero();
        if bitangent == Vec3::ZERO {
            return None;
        }
        let rotation = glam::Quat::from_mat3(&glam::Mat3::from_cols(tangent, bitangent, normal));
        let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
        Some(Self {
            plane: DrawingPlane {
                anchor: origin,
                rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
                depth: 0.0,
            },
            half_extent,
        })
    }

    pub fn hit(self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        let point = intersect_plane(origin, direction, self.plane)?;
        self.contains(point).then_some(point)
    }

    pub fn contains(self, point: Vec3) -> bool {
        let Some(half_extent) = self.half_extent else {
            return true;
        };
        let (u, v, _) = self.plane.basis();
        let offset = point - self.plane.origin();
        let tolerance = 1e-5 * half_extent.max_element().max(1.0);
        offset.dot(u).abs() <= half_extent.x + tolerance
            && offset.dot(v).abs() <= half_extent.y + tolerance
    }

    pub fn half_extent(self) -> Option<glam::Vec2> {
        self.half_extent
    }
}

impl SketchApp {
    pub(super) fn drawn_guide_attachment(&self) -> Option<u64> {
        let id = self.doc.active_guide?;
        let geometry = self.saved_guide_geometry()?;
        let supported_brush = self.doc.active_image_plane.is_none()
            && self.brush.dabs.surface_target.is_none()
            && self.brush.texture.anchor_mode != crate::texture::TextureAnchorMode::CapturedPlanar
            && (!geometry.supports_parametric_attachment()
                || (!self.brush.flow.enabled && !self.brush.dabs.enabled && self.brush.tip_asset_id == 0));
        (geometry.supports_drawn_attachment()
            && supported_brush
            && self.doc.animation.is_empty()
            && self.doc.animation.two_bone_rig.is_none()).then_some(id)
    }

    pub(super) fn begin_guide_bend(&mut self, guide_id: u64) {
        let Some(guide) = self
            .doc
            .guide_resources
            .iter()
            .find(|guide| guide.id == guide_id && guide.visible && !guide.locked)
        else {
            self.status = "Choose a visible, unlocked Plane or Freeform guide to bend.".into();
            return;
        };
        match GuideBendDraft::from_guide(guide_id, &guide.geometry) {
            Ok(draft) => {
                self.guide_bend_draft = Some(draft);
                self.status = "Bend preview: click four points on the guide (start, two cubic handles, end). Existing strokes stay in world space.".into();
                self.renderer.invalidate();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn cancel_guide_bend(&mut self) {
        if self.guide_bend_draft.take().is_some() {
            self.status =
                "Guide bend cancelled; saved guide and existing strokes are unchanged.".into();
            self.renderer.invalidate();
        }
    }

    pub(super) fn apply_guide_bend(&mut self) {
        let Some(draft) = self.guide_bend_draft.take() else {
            return;
        };
        let surface = match draft.preview.or_else(|| draft.build_surface().ok()) {
            Some(surface) => surface,
            None => {
                self.status = draft
                    .error
                    .clone()
                    .unwrap_or_else(|| "Set a valid four-point bend curve first.".into());
                self.guide_bend_draft = Some(draft);
                return;
            }
        };
        let before = self.guide_edit_state();
        let Some(guide) = self
            .doc
            .guide_resources
            .iter_mut()
            .find(|guide| guide.id == draft.guide_id && guide.visible && !guide.locked)
        else {
            self.status =
                "The source guide changed or became protected; bend preview cancelled.".into();
            self.renderer.invalidate();
            return;
        };
        guide.geometry = GuideGeometry::Freeform { surface };
        if self.commit_guide_edit(before) {
            self.status = "Guide bend applied. Attached ink follows the sheet.".into();
        }
    }

    pub(super) fn guide_bend_canvas_input(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
        events: &[egui::Event],
        pen: &[PenSample],
    ) {
        if !ctx.input(|input| input.focused) {
            self.cancel_guide_bend();
            return;
        }
        if events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                }
            )
        }) {
            self.cancel_guide_bend();
            return;
        }
        let scale = ctx.pixels_per_point();
        let mut clicks = Vec::new();
        for event in events {
            if let egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                ..
            } = event
            {
                clicks.push(*pos);
            }
        }
        for sample in pen
            .iter()
            .filter(|sample| sample.phase == Phase::Down && sample.contact)
        {
            clicks.push(Pos2::new(
                sample.position.x / scale,
                sample.position.y / scale,
            ));
        }
        for position in clicks {
            if !canvas_pointer_allowed(ctx, rect, position) {
                continue;
            }
            let (origin, direction) = crate::gizmo::ray(&self.camera, rect, position);
            let guide_id = self.guide_bend_draft.as_ref().unwrap().guide_id;
            let hit = self
                .doc
                .guide_resources
                .iter()
                .find(|guide| guide.id == guide_id && guide.visible && !guide.locked)
                .and_then(|guide| guide.geometry.clone().hit(origin, direction));
            let Some(hit) = hit else {
                self.status =
                    "Bend control missed the saved guide; click on the visible surface.".into();
                continue;
            };
            let draft = self.guide_bend_draft.as_mut().unwrap();
            match draft.push_world_control(hit) {
                Ok(()) => {
                    self.status = if let Some(error) = &draft.error {
                        error.clone()
                    } else if draft.preview.is_some() {
                        "Four bend controls set. Review the guide preview, then Apply or Cancel."
                            .into()
                    } else {
                        format!("Bend control {} of 4 set.", draft.controls.len())
                    };
                    self.renderer.invalidate();
                }
                Err(error) => self.status = error,
            }
        }
    }

    pub(super) fn resource_target_active(&self) -> bool {
        self.doc.active_guide.is_some()
            || self.doc.active_image_plane.is_some()
            || self.doc.active_model_reference.is_some()
    }

    fn guide_edit_state(&self) -> GuideEditState {
        (
            self.doc.guide_resources.clone(),
            self.doc.active_guide,
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
            self.doc.active_model_reference,
        )
    }

    fn commit_guide_edit(&mut self, before: GuideEditState) -> bool {
        let after = (
            self.doc.guide_resources.clone(),
            self.doc.active_guide,
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
            self.doc.active_model_reference,
        );
        if before == after {
            return false;
        }
        if before.1.is_some() || after.1.is_some() {
            if self.active.is_some() {
                Edit::Guides {
                    before: before.clone(), after: after.clone(), ink: None,
                }.apply(&mut self.doc, false);
            }
            self.finish();
            self.cancel_line_tool();
            self.cancel_curve_tool();
            self.cancel_shape_draft();
            self.shape_tool.mode = None;
            self.cancel_fill_tool();
        }
        let ink = match attached_guide_changes(&self.doc, &before.0, &after.0) {
            Ok(ink) => ink,
            Err(error) => {
                Edit::Guides { before, after, ink: None }.apply(&mut self.doc, false);
                self.status = error.into();
                return false;
            }
        };
        let edit = Edit::Guides { before, after, ink };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        true
    }

    pub(super) fn saved_guide_geometry(&self) -> Option<&GuideGeometry> {
        self.doc
            .active_guide
            .and_then(|id| {
                self.doc
                    .guide_resources
                    .iter()
                    .find(|guide| guide.id == id && guide.visible && !guide.locked)
            })
            .map(|guide| &guide.geometry)
    }

    /// Plane snapshot used by planar drawing tools. Saved non-planar guides,
    /// image planes, and model surfaces stay exclusive to brush drawing.
    pub(super) fn planar_drawing_target(&self) -> Option<PlanarTarget> {
        if self.doc.active_model_reference.is_some() {
            return None;
        }
        if let Some(id) = self.doc.active_image_plane {
            let image =
                self.doc.image_planes.iter().find(|image| {
                    image.id == id && image.visible && !image.locked && image.drawable
                })?;
            return Some(PlanarTarget {
                plane: DrawingPlane {
                    anchor: image.origin,
                    rotation_degrees: rotation_degrees(image.rotation),
                    depth: 0.0,
                },
                half_extent: Some(image.size * 0.5),
            });
        }
        if self.doc.active_guide.is_some() {
            return match self.saved_guide_geometry()? {
                GuideGeometry::Plane { origin, rotation } => Some(PlanarTarget {
                    plane: DrawingPlane {
                        anchor: *origin,
                        rotation_degrees: rotation_degrees(*rotation),
                        depth: 0.0,
                    },
                    half_extent: None,
                }),
                _ => None,
            };
        }
        (!self.use_cylinder && !self.use_revolved).then_some(PlanarTarget {
            plane: self.guide,
            half_extent: None,
        })
    }

    pub(super) fn saved_guide_surface(&self) -> Option<Surface> {
        geometry_surface(self.saved_guide_geometry()?)
    }

    pub(super) fn saved_guide_frame(&self, world: Vec3) -> Option<(Vec3, Vec3)> {
        geometry_frame(self.saved_guide_geometry()?, world)
    }

    pub(super) fn saved_guide_preview(&self) -> Option<GuidePreview> {
        self.guide_bend_draft
            .as_ref()
            .and_then(|draft| draft.preview.map(GuidePreview::Freeform))
            .or_else(|| self.loft_preview.map(GuidePreview::Loft))
            .or_else(|| self.saved_guide_geometry().and_then(geometry_preview))
    }

    pub(super) fn saved_guide_camera_frame(&self) -> Option<(Vec3, glam::Quat, f32)> {
        let guide = self.doc.guide_resources.iter().find(|guide| {
            Some(guide.id) == self.doc.active_guide && guide.visible && !guide.locked
        })?;
        Some(match &guide.geometry {
            GuideGeometry::Plane { origin, rotation } => (*origin, *rotation, 2.0),
            GuideGeometry::Cylinder {
                center,
                rotation,
                radius,
                height,
            }
            | GuideGeometry::Cone {
                center,
                rotation,
                radius,
                height,
            } => (*center, *rotation, (radius * 3.0).max(height * 0.7)),
            GuideGeometry::Sphere {
                center,
                rotation,
                radius,
            } => (*center, *rotation, radius * 3.0),
            GuideGeometry::Ring {
                center,
                rotation,
                major_radius,
                minor_radius,
            } => (
                *center,
                *rotation * glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                (major_radius + minor_radius) * 1.8,
            ),
            GuideGeometry::Freeform { surface } => (
                surface.center,
                surface.rotation,
                (surface.depth * 0.8)
                    .max((surface.directrix[3].x - surface.directrix[0].x).abs() * 1.5),
            ),
            GuideGeometry::Loft { surface } => {
                let bounds = surface.bounds().unwrap_or([Vec3::ZERO, Vec3::splat(2.0)]);
                (
                    surface.center + surface.rotation * (bounds[0] + bounds[1]) * 0.5,
                    surface.rotation,
                    (bounds[1] - bounds[0]).length().max(2.0),
                )
            }
            GuideGeometry::Revolved {
                center,
                rotation,
                profile,
            } => {
                let max_radius = profile.iter().map(|p| p.radius).fold(0.0_f32, f32::max);
                let height = profile
                    .last()
                    .zip(profile.first())
                    .map_or(2.0, |(last, first)| last.height - first.height);
                (*center, *rotation, (max_radius * 3.0).max(height * 0.7))
            }
        })
    }

    pub(super) fn saved_guide_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Saved guides", |ui| {
            if !self.selected.is_empty() {
                ui.horizontal(|ui| {
                    if ui.button("Attach selection to active guide").clicked() {
                        self.set_guide_attachment(true);
                    }
                    if ui.button("Detach selection").clicked() {
                        self.set_guide_attachment(false);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.guide_name);
                if ui.button("Save current").clicked() { self.save_current_guide(); }
            });
            ui.horizontal(|ui| {
                if ui.button("Add sphere").clicked() {
                    self.add_saved_guide("Sphere", GuideGeometry::sphere(self.camera.target, 1.0));
                }
                if ui.button("Add cone").clicked() {
                    self.add_saved_guide("Cone", GuideGeometry::cone(self.camera.target, 1.0, 2.0));
                }
                if add_ring_button(ui).clicked() {
                    self.add_saved_guide(
                        "Ring",
                        GuideGeometry::Ring {
                            center: self.camera.target,
                            rotation: glam::Quat::IDENTITY,
                            major_radius: 1.5,
                            minor_radius: 0.4,
                        },
                    );
                }
                if ui.button("Add freeform").clicked() {
                    let surface = FreeformSurface {
                        center: self.camera.target,
                        rotation: glam::Quat::IDENTITY,
                        directrix: [
                            glam::Vec2::new(-1.5, 0.0),
                            glam::Vec2::new(-0.5, 0.0),
                            glam::Vec2::new(0.5, 0.0),
                            glam::Vec2::new(1.5, 0.0),
                        ],
                        depth: 2.0,
                        bend: 0.0,
                    };
                    self.add_saved_guide("Freeform", GuideGeometry::Freeform { surface });
                }
                let eligible = self.selected_stroke_count();
                if preview_loft_button(ui, eligible).clicked() {
                    self.add_loft_from_selection();
                }
            });
            ui.small("For a loft, select 2–8 open cross-section strokes drawn in the same endpoint direction.");
            if let Some(surface) = self.loft_preview {
                match loft_preview_controls(ui, &surface) {
                    Some(LoftPreviewAction::Apply) => {
                        self.loft_preview = None;
                        self.add_saved_guide("Loft", GuideGeometry::Loft { surface });
                        self.selected.clear();
                        self.status = "Loft guide added as an independent snapshot.".into();
                    }
                    Some(LoftPreviewAction::Cancel) => {
                        self.loft_preview = None;
                        self.renderer.invalidate();
                    }
                    None => {}
                }
            }
            if let Some(draft) = self.guide_bend_draft.as_mut() {
                ui.separator();
                ui.strong("Bend guide preview");
                ui.small("Click four points on the guide: start, two cubic handles, and end. Attached ink follows compatible bends.");
                let mut depth_changed = false;
                ui.horizontal(|ui| {
                    ui.label("Sweep depth");
                    depth_changed = ui
                        .add(
                            egui::DragValue::new(&mut draft.depth)
                                .range(
                                    crate::document::guide_resources::freeform_surface::MIN_DEPTH
                                        ..=crate::document::guide_resources::freeform_surface::MAX_DEPTH,
                                )
                                .speed(0.05)
                                .suffix(" wu"),
                        )
                        .changed();
                });
                if depth_changed && draft.controls.len() == 4 {
                    match draft.build_surface() {
                        Ok(surface) => {
                            draft.preview = Some(surface);
                            draft.error = None;
                        }
                        Err(error) => {
                            draft.preview = None;
                            draft.error = Some(error);
                        }
                    }
                }
                ui.label(format!("{} / 4 control points", draft.controls.len()));
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                let can_apply = draft.preview.is_some();
                let mut apply = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    apply = ui.add_enabled(can_apply, egui::Button::new("Apply bend")).clicked();
                    cancel = ui.button("Cancel bend").clicked();
                });
                if apply {
                    self.apply_guide_bend();
                } else if cancel {
                    self.cancel_guide_bend();
                }
            }
            if self.doc.guide_resources.is_empty() {
                ui.small("Save a plane, cylinder, or revolved guide, or add a sphere/cone/ring/freeform.");
            }
            let mut remove = None;
            let before = self.guide_edit_state();
            let mut changed = false;
            let mut activate = None;
            let mut bend = None;
            for index in 0..self.doc.guide_resources.len() {
                let id = self.doc.guide_resources[index].id;
                let was_active = self.doc.active_guide == Some(id);
                let row = ui.group(|ui| {
                    let guide = &mut self.doc.guide_resources[index];
                    let mut action = guide_row_controls(ui, guide, was_active);
                    ui.add_enabled_ui(!guide.locked, |ui| {
                        let edit = edit_geometry_with_feedback(ui, &mut guide.geometry);
                        action.changed |= edit.changed;
                    });
                    action
                }).inner;
                if row.remove { remove = Some(index); }
                if let Some(active) = row.activate { activate = Some((id, active)); }
                if row.bend { bend = Some(id); }
                changed |= row.changed;
            }
            if let Some(index) = remove {
                let guide = self.doc.guide_resources.remove(index);
                if self.doc.active_guide == Some(guide.id) { self.doc.active_guide = None; }
                changed = true;
            }
            if let Some((id, true)) = activate {
                if self.doc.guide_resources.iter().any(|guide| guide.id == id && guide.visible && !guide.locked) {
                    self.doc.active_guide = Some(id);
                    self.doc.active_image_plane = None;
                    self.doc.active_model_reference = None;
                    for plane in &mut self.doc.image_planes { plane.drawable = false; }
                    changed = true;
                }
            } else if let Some((id, false)) = activate {
                if self.doc.active_guide == Some(id) { self.doc.active_guide = None; changed = true; }
            }
            if let Some(id) = self.doc.active_guide {
                if self.doc.guide_resources.iter().any(|guide| guide.id == id && (!guide.visible || guide.locked)) {
                    self.doc.active_guide = None;
                    changed = true;
                }
            }
            if changed { self.commit_guide_edit(before); }
            if let Some(id) = bend {
                self.begin_guide_bend(id);
            }
            ui.small("Saved guide edits move attached ink. Free ink stays in place.");
        });
        self.obj_reference_controls(ui);
    }

    fn set_guide_attachment(&mut self, attach: bool) {
        self.finish();
        let guide = if attach {
            let Some(id) = self.doc.active_guide.filter(|&id| {
                attachment_frame(&self.doc.guide_resources, id).is_some()
                    && self
                        .doc
                        .guide_resources
                        .iter()
                        .any(|guide| guide.id == id && guide.visible && !guide.locked)
            }) else {
                self.status = "Activate a visible, unlocked saved plane or cylinder first.".into();
                return;
            };
            if !self.doc.animation.is_empty() || self.doc.animation.two_bone_rig.is_some() {
                self.status = "Guide attachment requires static artwork.".into();
                return;
            }
            Some(id)
        } else {
            None
        };
        let before = crate::editing::snapshot(&self.doc, &self.selected);
        if before.is_empty()
            || before.len() != self.selected.len()
            || before.iter().any(|(_, stroke)| {
                !self.doc.editable(stroke)
                    || (attach
                        && (stroke.fill.is_some()
                            || stroke.curve.is_some()
                            || stroke.model_surface.is_some()
                            || stroke.brush.dabs.surface_target.is_some()
                            || stroke.brush.texture.anchor_mode
                                == crate::texture::TextureAnchorMode::CapturedPlanar
                            || self.doc.linked_instance_for_stroke(stroke.id).is_some()
                            || self
                                .doc
                                .linked_stroke_groups
                                .definitions
                                .iter()
                                .any(|group| group.source_stroke_ids.contains(&stroke.id))))
            })
        {
            self.status =
                "Select editable, independent sampled strokes with stroke-following texture."
                    .into();
            return;
        }
        let mut after = before.clone();
        let cylinder = guide
            .and_then(|id| attachment_frame(&self.doc.guide_resources, id))
            .and_then(|frame| frame.cylinder.map(|(_, height)| (frame, height)));
        if let Some((frame, height)) = cylinder {
            for (_, stroke) in &after {
                for point in &stroke.points {
                    if let Err(error) = frame.cylinder_point(point.position, frame, 0.0, height) {
                        self.status = error.into();
                        return;
                    }
                }
            }
        }
        for (_, stroke) in &mut after {
            stroke.guide_attachment = guide;
            if !attach {
                stroke.detach_model();
            }
        }
        if before == after {
            return;
        }
        let edit = Edit::Change { before, after };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status = if attach {
            "Selection attached to the saved guide."
        } else {
            "Selection detached in place."
        }
        .into();
    }

    fn save_current_guide(&mut self) {
        let name = if self.guide_name.trim().is_empty() {
            match (self.use_revolved, self.use_cylinder) {
                (true, _) => "Revolved",
                (_, true) => "Cylinder",
                _ => "Plane",
            }
            .to_owned()
        } else {
            self.guide_name.trim().to_owned()
        };
        let geometry = if self.use_revolved {
            revolved_geometry(self.revolved)
        } else if self.use_cylinder {
            GuideGeometry::Cylinder {
                center: self.cylinder.center,
                rotation: self.cylinder.rotation(),
                radius: self.cylinder.radius,
                height: self.cylinder.height,
            }
        } else {
            GuideGeometry::Plane {
                origin: self.guide.origin(),
                rotation: self.guide.rotation(),
            }
        };
        self.add_saved_guide(name, geometry);
    }

    pub(super) fn add_saved_guide(&mut self, name: impl Into<String>, geometry: GuideGeometry) {
        if self.doc.guide_resources.len() >= crate::document::guide_resources::MAX_GUIDES {
            self.status = "This drawing already has the maximum number of saved guides.".into();
            return;
        }
        let id = self.next_id;
        if id == 0 || id == u64::MAX {
            self.status = "No more guide IDs are available.".into();
            return;
        }
        let guide = GuideResource {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            geometry,
        };
        if let Err(error) = guide.validate() {
            self.status = error;
            return;
        }
        let before = self.guide_edit_state();
        self.doc.guide_resources.push(guide);
        self.doc.active_guide = Some(id);
        self.doc.active_image_plane = None;
        self.doc.active_model_reference = None;
        for plane in &mut self.doc.image_planes {
            plane.drawable = false;
        }
        self.next_id += 1;
        self.guide_name.clear();
        self.commit_guide_edit(before);
        self.status = "Saved guide added and activated.".into();
    }

    fn selected_stroke_count(&self) -> usize {
        self.doc
            .strokes
            .iter()
            .filter(|stroke| self.selected.contains(&stroke.id) && stroke.fill.is_none())
            .count()
    }

    fn add_loft_from_selection(&mut self) {
        let strokes: Vec<_> = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| self.selected.contains(&stroke.id) && stroke.fill.is_none())
            .collect();
        let surface = match loft_from_strokes(&strokes) {
            Ok(surface) => surface,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        self.loft_preview = Some(surface);
        self.renderer.invalidate();
        self.status =
            "Loft preview ready. Apply to save it, or cancel to leave the drawing unchanged."
                .into();
    }
}

fn loft_from_strokes(strokes: &[&Stroke]) -> Result<LoftSurface, String> {
    if let Some(error) = loft_input_error(strokes) {
        return Err(error);
    }
    let centroids: Vec<Vec3> = strokes
        .iter()
        .map(|stroke| {
            stroke
                .points
                .iter()
                .map(|point| point.position)
                .sum::<Vec3>()
                / stroke.points.len() as f32
        })
        .collect();
    let x_axis =
        (strokes[0].points.last().unwrap().position - strokes[0].points[0].position).normalize();
    let center = centroids.iter().copied().sum::<Vec3>() / centroids.len() as f32;
    let progression = centroids.last().unwrap() - centroids[0];
    let z_axis = (progression - x_axis * progression.dot(x_axis)).normalize_or_zero();
    let y_axis = z_axis.cross(x_axis).normalize_or_zero();
    let x_axis = y_axis.cross(z_axis).normalize_or_zero();
    let rotation =
        glam::Quat::from_mat3(&glam::Mat3::from_cols(x_axis, y_axis, z_axis)).normalize();
    let inverse = rotation.inverse();
    let mut ordered: Vec<_> = strokes
        .iter()
        .map(|stroke| {
            let points: Vec<Vec3> = stroke
                .points
                .iter()
                .map(|point| inverse * (point.position - center))
                .collect();
            let z = points.iter().map(|point| point.z).sum::<f32>() / points.len() as f32;
            (z, points)
        })
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let sections: Vec<_> = ordered
        .iter()
        .map(|(_, points)| fit_loft_section(points))
        .collect();
    let section_count = sections.len();
    let mut section_storage = [LoftSection {
        controls: [Vec3::ZERO; 4],
    }; 8];
    section_storage[..section_count].copy_from_slice(&sections);
    let surface = LoftSurface {
        center,
        rotation,
        sections: section_storage,
        section_count: section_count as u8,
        tension: 0.0,
    };
    surface
        .validate()
        .map_err(|error| format!("Cannot create loft: {error}"))?;
    Ok(surface)
}

fn loft_input_error(strokes: &[&Stroke]) -> Option<String> {
    if !(2..=8).contains(&strokes.len()) {
        return Some("Select between two and eight open strokes for a loft.".into());
    }
    if strokes.iter().any(|stroke| stroke.points.len() < 2) {
        return Some("Each loft section needs at least two points.".into());
    }
    let Some(tangent) =
        (strokes[0].points.last()?.position - strokes[0].points.first()?.position).try_normalize()
    else {
        return Some("The first loft section has no usable direction.".into());
    };
    let centroids: Vec<Vec3> = strokes
        .iter()
        .map(|stroke| {
            stroke
                .points
                .iter()
                .map(|point| point.position)
                .sum::<Vec3>()
                / stroke.points.len() as f32
        })
        .collect();
    let progression = centroids.last()? - centroids[0];
    if (progression - tangent * progression.dot(tangent))
        .try_normalize()
        .is_none()
    {
        return Some("Loft sections need a distinct progression direction.".into());
    }
    None
}

fn add_ring_button(ui: &mut egui::Ui) -> egui::Response {
    ui.button("Add ring")
}

fn preview_loft_button(ui: &mut egui::Ui, eligible_strokes: usize) -> egui::Response {
    ui.add_enabled(
        (2..=8).contains(&eligible_strokes),
        egui::Button::new("Preview loft from selected strokes"),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoftPreviewAction {
    Apply,
    Cancel,
}

fn loft_preview_controls(ui: &mut egui::Ui, surface: &LoftSurface) -> Option<LoftPreviewAction> {
    let mut action = None;
    ui.group(|ui| {
        ui.strong("Loft preview");
        ui.small(format!(
            "{} sections, ordered along +Z; each stroke keeps its original endpoint direction.",
            surface.section_count
        ));
        for section in 0..surface.section_count {
            ui.small(loft_section_label(section));
        }
        let (apply, cancel) = loft_preview_action_buttons(ui);
        if apply.clicked() {
            action = Some(LoftPreviewAction::Apply);
        }
        if cancel.clicked() {
            action = Some(LoftPreviewAction::Cancel);
        }
    });
    action
}

fn loft_section_label(index: u8) -> String {
    format!("Section {}", index + 1)
}

fn loft_preview_action_buttons(ui: &mut egui::Ui) -> (egui::Response, egui::Response) {
    ui.horizontal(|ui| (ui.button("Apply loft"), ui.button("Cancel preview")))
        .inner
}

#[derive(Default)]
struct GuideRowAction {
    activate: Option<bool>,
    changed: bool,
    remove: bool,
    bend: bool,
    use_rect: Option<Rect>,
    visible_rect: Option<Rect>,
    locked_rect: Option<Rect>,
    remove_rect: Option<Rect>,
}

fn guide_row_controls(
    ui: &mut egui::Ui,
    guide: &mut GuideResource,
    is_active: bool,
) -> GuideRowAction {
    let mut action = GuideRowAction::default();
    ui.horizontal(|ui| {
        ui.strong(&guide.name);
        let mut active = is_active;
        let use_response = ui.checkbox(&mut active, "Use");
        action.use_rect = Some(use_response.rect);
        if use_response.changed() {
            action.activate = Some(active);
        }
        let visible_response = ui.checkbox(&mut guide.visible, "Visible");
        action.visible_rect = Some(visible_response.rect);
        action.changed |= visible_response.changed();
        let locked_response = ui.checkbox(&mut guide.locked, "Locked");
        action.locked_rect = Some(locked_response.rect);
        action.changed |= locked_response.changed();
        let remove_response = ui.small_button("Remove");
        action.remove_rect = Some(remove_response.rect);
        action.remove = remove_response.clicked();
        if !guide.locked
            && matches!(
                &guide.geometry,
                GuideGeometry::Plane { .. } | GuideGeometry::Freeform { .. }
            )
        {
            action.bend = ui.small_button("Bend…").clicked();
        }
    });
    action
}

fn fit_loft_section(points: &[Vec3]) -> LoftSection {
    let sample = |fraction: f32| {
        let mut lengths = Vec::with_capacity(points.len());
        lengths.push(0.0_f32);
        for pair in points.windows(2) {
            lengths.push(lengths.last().copied().unwrap() + pair[0].distance(pair[1]));
        }
        let total = *lengths.last().unwrap();
        let target = total * fraction;
        let index = lengths
            .partition_point(|length| *length < target)
            .clamp(1, points.len() - 1);
        let span = lengths[index] - lengths[index - 1];
        let t = if span <= 1e-8 {
            0.0
        } else {
            (target - lengths[index - 1]) / span
        };
        points[index - 1].lerp(points[index], t)
    };
    let [p0, q1, q2, p3] = [
        sample(0.0),
        sample(1.0 / 3.0),
        sample(2.0 / 3.0),
        sample(1.0),
    ];
    let a = q1 * 27.0 - p0 * 8.0 - p3;
    let b = q2 * 27.0 - p0 - p3 * 8.0;
    LoftSection {
        controls: [p0, a / 9.0 - b / 18.0, -a / 18.0 + b / 9.0, p3],
    }
}

pub(super) fn rotation_degrees(rotation: glam::Quat) -> Vec3 {
    let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}
fn rotation_from_degrees(degrees: Vec3) -> glam::Quat {
    let r = Vec3::new(
        degrees.x.to_radians(),
        degrees.y.to_radians(),
        degrees.z.to_radians(),
    );
    glam::Quat::from_euler(glam::EulerRot::ZYX, r.z, r.y, r.x).normalize()
}
fn revolved_geometry(value: Revolved) -> GuideGeometry {
    GuideGeometry::Revolved {
        center: value.center,
        rotation: value.rotation(),
        profile: value.profile[..value.count]
            .iter()
            .map(|p| SavedProfilePoint {
                height: p.height,
                radius: p.radius,
            })
            .collect(),
    }
}
fn to_revolved(center: Vec3, rotation: glam::Quat, profile: &[SavedProfilePoint]) -> Revolved {
    let mut result = Revolved {
        center,
        rotation_degrees: rotation_degrees(rotation),
        count: profile.len().min(8),
        ..Revolved::default()
    };
    for (dest, source) in result.profile.iter_mut().zip(profile.iter()).take(8) {
        *dest = ProfilePoint {
            height: source.height,
            radius: source.radius,
        };
    }
    result
}
fn to_cylinder(center: Vec3, rotation: glam::Quat, radius: f32, height: f32) -> Cylinder {
    Cylinder {
        center,
        rotation_degrees: rotation_degrees(rotation),
        radius,
        height,
    }
}

fn geometry_surface(geometry: &GuideGeometry) -> Option<Surface> {
    Some(match geometry {
        GuideGeometry::Plane { .. } => Surface::Plane,
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } => Surface::Cylinder(to_cylinder(*center, *rotation, *radius, *height)),
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => Surface::Revolved(to_revolved(*center, *rotation, profile)),
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => Surface::Sphere(SphereSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
        }),
        GuideGeometry::Ring {
            center,
            rotation,
            major_radius,
            minor_radius,
        } => Surface::Ring(RingSurface {
            center: *center,
            rotation: *rotation,
            major_radius: *major_radius,
            minor_radius: *minor_radius,
        }),
        GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => Surface::Cone(ConeSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
            height: *height,
        }),
        GuideGeometry::Freeform { surface } => Surface::Freeform(*surface),
        GuideGeometry::Loft { surface } => Surface::Loft(*surface),
    })
}
fn geometry_preview(geometry: &GuideGeometry) -> Option<GuidePreview> {
    Some(match geometry {
        GuideGeometry::Plane { origin, rotation } => GuidePreview::Plane(DrawingPlane {
            anchor: *origin,
            rotation_degrees: rotation_degrees(*rotation),
            depth: 0.0,
        }),
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } => GuidePreview::Cylinder(to_cylinder(*center, *rotation, *radius, *height)),
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => GuidePreview::Revolved(to_revolved(*center, *rotation, profile)),
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => GuidePreview::Sphere(SphereSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
        }),
        GuideGeometry::Ring {
            center,
            rotation,
            major_radius,
            minor_radius,
        } => GuidePreview::Ring(RingSurface {
            center: *center,
            rotation: *rotation,
            major_radius: *major_radius,
            minor_radius: *minor_radius,
        }),
        GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => GuidePreview::Cone(ConeSurface {
            center: *center,
            rotation: *rotation,
            radius: *radius,
            height: *height,
        }),
        GuideGeometry::Freeform { surface } => GuidePreview::Freeform(*surface),
        GuideGeometry::Loft { surface } => GuidePreview::Loft(*surface),
    })
}
fn geometry_frame(geometry: &GuideGeometry, world: Vec3) -> Option<(Vec3, Vec3)> {
    Some(match geometry {
        GuideGeometry::Plane { rotation, .. } => (*rotation * Vec3::Z, *rotation * Vec3::X),
        GuideGeometry::Cylinder {
            center, rotation, ..
        } => {
            let local = rotation.inverse() * (world - *center);
            (
                *rotation * Vec3::new(local.x, 0.0, local.z).normalize_or_zero(),
                *rotation * Vec3::new(local.z, 0.0, -local.x).normalize_or_zero(),
            )
        }
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => {
            let surface = to_revolved(*center, *rotation, profile);
            (surface.normal(world), surface.tangent(world))
        }
        GuideGeometry::Sphere {
            center, rotation, ..
        } => {
            let local = rotation.inverse() * (world - *center);
            let tangent = Vec3::new(local.z, 0.0, -local.x).normalize_or_zero();
            (
                *rotation * local.normalize_or_zero(),
                *rotation
                    * if tangent.length_squared() > 0.5 {
                        tangent
                    } else {
                        Vec3::X
                    },
            )
        }
        GuideGeometry::Ring {
            center,
            rotation,
            major_radius,
            minor_radius: _,
        } => {
            let local = rotation.inverse() * (world - *center);
            let major = local.x.atan2(local.z);
            let tube_center = Vec3::new(
                *major_radius * major.sin(),
                0.0,
                *major_radius * major.cos(),
            );
            let normal = (local - tube_center).normalize_or_zero();
            let tangent = Vec3::new(major.cos(), 0.0, -major.sin());
            (*rotation * normal, *rotation * tangent)
        }
        GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => {
            let local = rotation.inverse() * (world - *center);
            let k = *radius / *height;
            let tangent = Vec3::new(local.z, 0.0, -local.x).normalize_or_zero();
            (
                *rotation
                    * Vec3::new(local.x, k * k * (*height - local.y), local.z).normalize_or_zero(),
                *rotation
                    * if tangent.length_squared() > 0.5 {
                        tangent
                    } else {
                        Vec3::X
                    },
            )
        }
        GuideGeometry::Freeform { surface } => {
            let (u, v) = surface.coordinates(world).unwrap_or((0.5, 0.5));
            let (tangent, normal) = surface.frame(u, v);
            (normal, tangent)
        }
        GuideGeometry::Loft { surface } => {
            let (u, v) = surface.coordinates(world).unwrap_or((0.5, 0.5));
            let (tangent, normal) = surface.frame(u, v);
            (normal, tangent)
        }
    })
}

#[derive(Default)]
struct GeometryEditResponse {
    changed: bool,
    invalid_input: bool,
    bend_rect: Option<Rect>,
    directrix_x_rects: [Option<Rect>; 4],
}

fn edit_geometry_with_feedback(
    ui: &mut egui::Ui,
    geometry: &mut GuideGeometry,
) -> GeometryEditResponse {
    let before = geometry.clone();
    let mut response = edit_geometry(ui, geometry);
    let is_valid = || {
        GuideResource {
            id: 1,
            name: "Inspector validation".into(),
            visible: true,
            locked: false,
            geometry: geometry.clone(),
        }
        .validate()
    };
    if response.changed && is_valid().is_err() {
        *geometry = before;
        response.changed = false;
        response.invalid_input = true;
        ui.colored_label(
            ui_style::colors(ui.ctx()).error,
            "Invalid guide geometry; the last valid values were restored.",
        );
    }
    response
}

fn edit_geometry(ui: &mut egui::Ui, geometry: &mut GuideGeometry) -> GeometryEditResponse {
    let mut changed = false;
    let mut response = GeometryEditResponse::default();
    match geometry {
        GuideGeometry::Plane { origin, rotation } => {
            changed |= edit_transform(ui, origin, rotation);
        }
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        }
        | GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => {
            changed |= edit_transform(ui, center, rotation);
            changed |= ui
                .add(egui::Slider::new(radius, 0.001..=10000.0).text("Radius (wu)"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(height, 0.001..=10000.0).text("Height (wu)"))
                .changed();
        }
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => {
            changed |= edit_transform(ui, center, rotation);
            changed |= ui
                .add(egui::Slider::new(radius, 0.001..=10000.0).text("Radius (wu)"))
                .changed();
        }
        GuideGeometry::Ring {
            center,
            rotation,
            major_radius,
            minor_radius,
        } => {
            changed |= edit_transform(ui, center, rotation);
            changed |= ui
                .add(egui::Slider::new(major_radius, 0.002..=10000.0).text("Major radius (wu)"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(minor_radius, 0.001..=9999.0).text("Tube radius (wu)"))
                .changed();
            ui.small("Tube radius must stay smaller than major radius to keep a non-self-intersecting ring.");
        }
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => {
            changed |= edit_transform(ui, center, rotation);
            let mut remove = None;
            let can_remove = profile.len() > 2;
            for (i, point) in profile.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("P{}", i + 1));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut point.height)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("Y "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut point.radius)
                                .speed(0.01)
                                .range(0.001..=10000.0)
                                .prefix("R "),
                        )
                        .changed();
                    if can_remove
                        && ui_style::small_icon_button(
                            ui,
                            "−",
                            &format!("Remove guide profile point {}", i + 1),
                            true,
                        )
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            }

            if let Some(index) = remove {
                profile.remove(index);
                changed = true;
            }
            if profile.len() < 8 && ui.small_button("Add profile point").clicked() {
                let last = *profile.last().unwrap();
                profile.push(SavedProfilePoint {
                    height: last.height + 1.0,
                    radius: last.radius,
                });
                changed = true;
            }
        }
        GuideGeometry::Freeform { surface } => {
            changed |= edit_transform(ui, &mut surface.center, &mut surface.rotation);
            changed |= ui
                .add(egui::Slider::new(&mut surface.depth, 0.02..=10000.0).text("Sweep depth (wu)"))
                .changed();
            let bend = ui.add(egui::Slider::new(&mut surface.bend, -0.9..=0.9).text("Sweep bend"));
            response.bend_rect = Some(bend.rect);
            changed |= bend.changed();
            ui.small("Edit the four monotone directrix control points in local XY.");
            for (index, point) in surface.directrix.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("C{}", index + 1));
                    let x = ui.add(
                        egui::DragValue::new(&mut point.x)
                            .speed(0.01)
                            .range(-10000.0..=10000.0)
                            .prefix("X "),
                    );
                    response.directrix_x_rects[index] = Some(x.rect);
                    changed |= x.changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut point.y)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("Y "),
                        )
                        .changed();
                });
            }
        }
        GuideGeometry::Loft { surface } => {
            changed |= edit_transform(ui, &mut surface.center, &mut surface.rotation);
            changed |= ui
                .add(egui::Slider::new(&mut surface.tension, -1.0..=1.0).text("Loft tension"))
                .changed();
            ui.collapsing("Loft section controls", |ui| {
                ui.small("Edit the cubic controls in loft-local coordinates. Sections retain their order.");
                for (index, section) in surface.sections.iter_mut()
                    .take(usize::from(surface.section_count)).enumerate()
                {
                    ui.collapsing(loft_section_label(index as u8), |ui| {
                        for (control, point) in section.controls.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("C{}", control + 1));
                                for (axis, label) in ["X ", "Y ", "Z "].into_iter().enumerate() {
                                    changed |= ui.add(egui::DragValue::new(&mut point[axis])
                                        .speed(0.01).range(-10_000.0..=10_000.0).prefix(label))
                                        .changed();
                                }
                            });
                        }
                    });
                }
            });
        }
    }
    response.changed = changed;
    response
}
fn edit_transform(ui: &mut egui::Ui, center: &mut Vec3, rotation: &mut glam::Quat) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Center");
        for axis in 0..3 {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut center[axis])
                        .speed(0.01)
                        .range(-10000.0..=10000.0),
                )
                .changed();
        }
    });
    let mut degrees = rotation_degrees(*rotation);
    ui.horizontal(|ui| {
        ui.label("Rotation°");
        for axis in 0..3 {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut degrees[axis])
                        .speed(0.5)
                        .range(-3600.0..=3600.0),
                )
                .changed();
        }
    });
    if changed {
        *rotation = rotation_from_degrees(degrees);
    }
    changed
}

#[cfg(test)]
mod ring_button_tests {
    use super::add_ring_button;
    use eframe::egui::{self, Pos2, Rect, Vec2};

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (bool, Rect) {
        let mut result = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 480.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = add_ring_button(ui);
                    result = Some((response.clicked(), response.rect));
                });
            },
        );
        result.expect("ring guide button renders")
    }

    #[test]
    fn add_ring_button_responds_to_pointer_click() {
        let ctx = egui::Context::default();
        let (_, rect) = frame(&ctx, Vec::new());
        let position = rect.center();
        let _ = frame(&ctx, vec![egui::Event::PointerMoved(position)]);
        let _ = frame(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        let (clicked, _) = frame(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert!(
            clicked,
            "the production Add ring button dispatches on click"
        );
    }
}

#[cfg(test)]
mod guide_row_tests {
    use super::{guide_row_controls, GuideRowAction};
    use crate::document::guide_resources::{GuideGeometry, GuideResource};
    use eframe::egui::{self, Event, PointerButton, Pos2, Rect, Vec2};
    use glam::Vec3;

    fn frame(
        ctx: &egui::Context,
        guide: &mut GuideResource,
        active: bool,
        events: Vec<Event>,
    ) -> GuideRowAction {
        let mut result = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 480.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    result = Some(ui.group(|ui| guide_row_controls(ui, guide, active)).inner);
                });
            },
        );
        result.expect("saved guide inspector row renders")
    }

    fn click(
        ctx: &egui::Context,
        guide: &mut GuideResource,
        active: bool,
        position: Pos2,
    ) -> GuideRowAction {
        let _ = frame(ctx, guide, active, vec![Event::PointerMoved(position)]);
        let _ = frame(
            ctx,
            guide,
            active,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        frame(
            ctx,
            guide,
            active,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        )
    }

    #[test]
    fn saved_guide_inspector_use_visibility_lock_and_remove_controls_respond_to_mouse() {
        let ctx = egui::Context::default();
        let mut guide = GuideResource {
            id: 42,
            name: "Interaction guide".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Plane {
                origin: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
            },
        };
        let controls = frame(&ctx, &mut guide, false, Vec::new());

        let use_action = click(&ctx, &mut guide, false, controls.use_rect.unwrap().center());
        assert_eq!(use_action.activate, Some(true), "Use activates the guide");

        let visibility_action = click(
            &ctx,
            &mut guide,
            true,
            controls.visible_rect.unwrap().center(),
        );
        assert!(visibility_action.changed);
        assert!(!guide.visible, "Visible toggles the guide independently");

        guide.visible = true;
        let lock_action = click(
            &ctx,
            &mut guide,
            true,
            controls.locked_rect.unwrap().center(),
        );
        assert!(lock_action.changed);
        assert!(guide.locked, "Locked toggles the guide independently");

        let remove_action = click(
            &ctx,
            &mut guide,
            false,
            controls.remove_rect.unwrap().center(),
        );
        assert!(remove_action.remove, "Remove dispatches a distinct action");
    }
}

#[cfg(test)]
mod guide_bend_tests {
    use super::*;
    use crate::document::{Document, Point, Stroke};
    use glam::Vec3;
    use std::collections::HashMap;

    fn plane() -> GuideGeometry {
        GuideGeometry::Plane {
            origin: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
        }
    }

    fn bend_from(geometry: &GuideGeometry, controls: [Vec3; 4]) -> FreeformSurface {
        let mut draft = GuideBendDraft::from_guide(7, geometry).unwrap();
        for point in controls {
            draft.push_world_control(point).unwrap();
        }
        draft
            .preview
            .expect("four controls produce a validated preview")
    }

    #[test]
    fn mouse_control_points_produce_a_bounded_bend_and_reject_degenerate_curves() {
        let source = plane();
        let controls = [
            Vec3::new(-2.0, 0.0, 0.0),
            Vec3::new(-0.5, 1.2, 0.0),
            Vec3::new(0.5, 1.2, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ];
        let surface = bend_from(&source, controls);
        assert!(surface.validate().is_ok());
        assert_ne!(surface.point(0.5, 0.5).y, 0.0);
        assert!(surface.depth >= crate::document::guide_resources::freeform_surface::MIN_DEPTH);

        let mut degenerate = GuideBendDraft::from_guide(7, &source).unwrap();
        for point in [Vec3::ZERO, Vec3::X, Vec3::X * 0.5, Vec3::X * 2.0] {
            degenerate.push_world_control(point).unwrap();
        }
        assert!(
            degenerate.preview.is_none(),
            "non-monotone handles do not preview"
        );
        assert!(degenerate.error.is_some());

        let mut bad_depth = GuideBendDraft::from_guide(7, &source).unwrap();
        bad_depth.depth = f32::NAN;
        bad_depth.controls = controls.map(|point| point.truncate()).to_vec();
        assert!(bad_depth.build_surface().is_err());
    }

    #[test]
    fn repeated_bends_keep_surface_hits_and_world_strokes_persist_merge_and_export() {
        let first = bend_from(
            &plane(),
            [
                Vec3::new(-2.0, 0.0, 0.0),
                Vec3::new(-0.6, 1.0, 0.0),
                Vec3::new(0.6, 1.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ],
        );
        let second_controls = [0.0, 0.33, 0.66, 1.0].map(|u| first.point(u, 0.5));
        let repeated = bend_from(&GuideGeometry::Freeform { surface: first }, second_controls);
        assert!(repeated.validate().is_ok());
        let triangles = repeated.triangles(64, 8).unwrap();
        let [a, b, c] = triangles[64 * 4 + 4];
        let point = (a + b + c) / 3.0;
        let normal = (b - a).cross(c - a).normalize();
        let hit = repeated
            .hit(point + normal * 2.0, -normal)
            .expect("repeated bend triangle remains hittable");
        assert!(hit.distance(point) < 0.04, "hit={hit:?} point={point:?}");
        let (tangent, frame_normal) = repeated.frame(0.5, 0.5);
        assert!(tangent.is_finite() && frame_normal.is_finite());

        // Existing artwork remains world-space. A new stroke sampled on the
        // bent centerline is ordinary editable/exportable ink.
        let stroke = Stroke {
            guide_attachment: None,
            model_surface: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id: 11,
            color: [0.8, 0.25, 0.1],
            gradient: None,
            radius: 0.04,
            brush: Default::default(),
            points: [0.1, 0.5, 0.9]
                .map(|u| Point {
                    position: repeated.point(u, 0.5),
                    pressure: 0.8,
                    tilt: None,
                    time_ms: 0,
                })
                .to_vec(),
            curve: None,
        };
        let mut document = Document {
            guide_resources: vec![GuideResource {
                id: 7,
                name: "Bend test".into(),
                visible: true,
                locked: false,
                geometry: GuideGeometry::Freeform { surface: repeated },
            }],
            active_guide: Some(7),
            strokes: vec![stroke.clone()],
            ..Document::default()
        };
        let before_stroke = document.strokes.clone();
        let native = serde_json::to_string(&document).unwrap();
        let reloaded = Document::from_json(&native).unwrap();
        assert_eq!(reloaded.guide_resources, document.guide_resources);
        assert_eq!(reloaded.strokes, before_stroke);

        let recovery_root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "guide-bend-test".into(),
            None,
            reloaded.clone(),
        );
        crate::recovery_store::write(recovery_root.path(), &snapshot).unwrap();
        let recovered =
            crate::recovery_store::scan(recovery_root.path(), "guide-bend-reader").unwrap();
        assert_eq!(
            recovered.candidates[0].snapshot.document.guide_resources,
            document.guide_resources
        );
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes,
            before_stroke
        );

        let source_path = recovery_root.path().join("source.sketch.json");
        std::fs::write(&source_path, native).unwrap();
        let assets = recovery_root.path().join("merged-assets");
        std::fs::create_dir_all(&assets).unwrap();
        let inserted = super::super::native_merge::prepare_insert(
            &Document::default(),
            &source_path,
            &assets,
            100,
            10,
            1000,
            Vec3::new(1.0, 0.0, 0.0),
            false,
            &HashMap::new(),
        )
        .unwrap();
        let edit = crate::editing::Edit::Merge(inserted.edit);
        edit.apply(&mut document, true);
        assert!(document.guide_resources.len() >= 2);
        assert_eq!(document.strokes.len(), 2);
        let merged_native =
            Document::from_json(&serde_json::to_string(&document).unwrap()).unwrap();
        assert_eq!(merged_native.strokes, document.strokes);
        let export = crate::export::encode(&merged_native, None).unwrap();
        assert!(export.summary.vertices > 0);
        edit.apply(&mut document, false);
        assert_eq!(document.strokes, before_stroke);
    }
}

#[cfg(test)]
mod loft_ui_tests {
    use super::{
        loft_from_strokes, loft_preview_action_buttons, loft_section_label, preview_loft_button,
        LoftPreviewAction,
    };
    use crate::document::{Brush, Point, Stroke};
    use eframe::egui::{self, Event, PointerButton, Pos2, Rect, Vec2};
    use glam::Vec3;

    fn line(id: u64, start: Vec3, end: Vec3) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id,
            color: [0.2, 0.4, 0.6],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![
                Point {
                    position: start,
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                },
                Point {
                    position: end,
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                },
            ],
        }
    }

    fn button_frame(
        ctx: &egui::Context,
        eligible_strokes: usize,
        events: Vec<Event>,
    ) -> (bool, Rect) {
        let mut result = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(720.0, 480.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = preview_loft_button(ui, eligible_strokes);
                    result = Some((response.clicked(), response.rect));
                });
            },
        );
        result.expect("loft preview button renders")
    }

    fn action_buttons_frame(ctx: &egui::Context, events: Vec<Event>) -> (bool, Rect, bool, Rect) {
        let mut result = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(720.0, 480.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (apply, cancel) = loft_preview_action_buttons(ui);
                    result = Some((apply.clicked(), apply.rect, cancel.clicked(), cancel.rect));
                });
            },
        );
        result.expect("loft preview action controls render")
    }

    fn click_selection_button(
        ctx: &egui::Context,
        eligible_strokes: usize,
        position: Pos2,
    ) -> bool {
        let _ = button_frame(ctx, eligible_strokes, vec![Event::PointerMoved(position)]);
        let _ = button_frame(
            ctx,
            eligible_strokes,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        button_frame(
            ctx,
            eligible_strokes,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        )
        .0
    }

    fn click_preview_action(
        ctx: &egui::Context,
        position: Pos2,
        action: LoftPreviewAction,
    ) -> LoftPreviewAction {
        let _ = action_buttons_frame(ctx, vec![Event::PointerMoved(position)]);
        let _ = action_buttons_frame(
            ctx,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        let (apply_clicked, _, cancel_clicked, _) = action_buttons_frame(
            ctx,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let actual = match (apply_clicked, cancel_clicked) {
            (true, false) => LoftPreviewAction::Apply,
            (false, true) => LoftPreviewAction::Cancel,
            _ => panic!("the requested preview action must dispatch by itself"),
        };
        assert_eq!(actual, action);
        actual
    }

    #[test]
    fn loft_preview_apply_cancel_and_selection_controls_respond_to_mouse() {
        let ctx = egui::Context::default();
        let (disabled_clicked, disabled_rect) = button_frame(&ctx, 1, Vec::new());
        assert!(!click_selection_button(&ctx, 1, disabled_rect.center()));
        assert!(!disabled_clicked);
        let (_, preview_rect) = button_frame(&ctx, 3, Vec::new());
        assert!(click_selection_button(&ctx, 3, preview_rect.center()));

        assert_eq!(
            (0..3).map(loft_section_label).collect::<Vec<_>>(),
            vec!["Section 1", "Section 2", "Section 3"]
        );
        let (_, apply_rect, _, cancel_rect) = action_buttons_frame(&ctx, Vec::new());
        assert_eq!(
            click_preview_action(&ctx, cancel_rect.center(), LoftPreviewAction::Cancel),
            LoftPreviewAction::Cancel
        );
        assert_eq!(
            click_preview_action(&ctx, apply_rect.center(), LoftPreviewAction::Apply),
            LoftPreviewAction::Apply
        );
    }

    #[test]
    fn loft_selection_rejects_parallel_progression_with_user_feedback() {
        let strokes = [
            line(1, Vec3::ZERO, Vec3::X),
            line(2, Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0)),
        ];
        let selected: Vec<_> = strokes.iter().collect();
        assert_eq!(
            loft_from_strokes(&selected).unwrap_err(),
            "Loft sections need a distinct progression direction."
        );
    }

    #[test]
    fn generated_loft_orders_source_sections_and_keeps_an_independent_snapshot() {
        let mut strokes = [
            line(1, Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0)),
            line(2, Vec3::new(-1.0, 0.4, 1.0), Vec3::new(1.0, 0.4, 1.0)),
            line(3, Vec3::new(-1.0, -0.2, 0.0), Vec3::new(1.0, -0.2, 0.0)),
        ];
        let selected: Vec<_> = strokes.iter().collect();
        let surface = loft_from_strokes(&selected).unwrap();
        assert_eq!(surface.section_count, 3);
        assert!(surface.validate().is_ok());
        assert!(surface.sections[..3]
            .windows(2)
            .all(|pair| pair[0].controls[0].z < pair[1].controls[0].z));
        assert!(surface.sections[..3]
            .iter()
            .all(|section| section.controls[0].x < section.controls[3].x));

        let snapshot = surface;
        strokes[1].points[0].position.y += 5.0;
        assert_eq!(
            snapshot, surface,
            "source edits do not change the generated loft snapshot"
        );
    }
}

#[cfg(test)]
mod freeform_inspector_tests {
    use super::{edit_geometry_with_feedback, GeometryEditResponse};
    use crate::document::guide_resources::{FreeformSurface, GuideGeometry};
    use eframe::egui::{self, Event, PointerButton, Pos2, Rect, Vec2};
    use glam::{Quat, Vec3};

    fn surface_geometry() -> GuideGeometry {
        GuideGeometry::Freeform {
            surface: FreeformSurface {
                center: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                directrix: [
                    glam::Vec2::new(-1.5, 0.0),
                    glam::Vec2::new(-0.5, 0.0),
                    glam::Vec2::new(0.5, 0.0),
                    glam::Vec2::new(1.5, 0.0),
                ],
                depth: 2.0,
                bend: 0.0,
            },
        }
    }

    fn frame(
        ctx: &egui::Context,
        geometry: &mut GuideGeometry,
        events: Vec<Event>,
    ) -> GeometryEditResponse {
        let mut result = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 720.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    result = Some(edit_geometry_with_feedback(ui, geometry));
                });
            },
        );
        result.expect("freeform guide inspector controls render")
    }

    fn click(
        ctx: &egui::Context,
        geometry: &mut GuideGeometry,
        position: Pos2,
    ) -> GeometryEditResponse {
        let _ = frame(ctx, geometry, vec![Event::PointerMoved(position)]);
        let mut response = frame(
            ctx,
            geometry,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        let released = frame(
            ctx,
            geometry,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        response.changed |= released.changed;
        response.invalid_input |= released.invalid_input;
        response
    }

    #[test]
    fn freeform_inspector_edits_parameters_and_restores_invalid_directrix_with_feedback() {
        let ctx = egui::Context::default();
        let mut geometry = surface_geometry();
        let controls = frame(&ctx, &mut geometry, Vec::new());
        let bend_rect = controls.bend_rect.expect("freeform bend slider renders");
        let bend_click = Pos2::new(
            bend_rect.left() + bend_rect.width() * 0.4,
            bend_rect.center().y,
        );
        let edit = click(&ctx, &mut geometry, bend_click);
        let GuideGeometry::Freeform { surface } = &geometry else {
            unreachable!()
        };
        assert!(
            edit.changed,
            "bend slider dispatches an edit (rect {bend_rect:?}, value {})",
            surface.bend
        );
        assert!(
            surface.bend > 0.1,
            "bend slider changes the saved parameter"
        );
        assert!(
            surface.validate().is_ok(),
            "valid inspector values are retained"
        );

        let last_valid_geometry = geometry.clone();
        let controls = frame(&ctx, &mut geometry, Vec::new());
        let x_rect = controls.directrix_x_rects[1].expect("second directrix X control renders");
        let start = x_rect.center();
        let end = start + Vec2::new(-120.0, 0.0);
        let _ = frame(&ctx, &mut geometry, vec![Event::PointerMoved(start)]);
        let _ = frame(
            &ctx,
            &mut geometry,
            vec![Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        let invalid = frame(&ctx, &mut geometry, vec![Event::PointerMoved(end)]);
        let _ = frame(
            &ctx,
            &mut geometry,
            vec![Event::PointerButton {
                pos: end,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert!(
            invalid.invalid_input,
            "crossing X controls displays validation feedback"
        );
        assert!(
            !invalid.changed,
            "rejected values do not enter the document edit"
        );
        assert_eq!(
            geometry, last_valid_geometry,
            "invalid input restores the last valid geometry"
        );
    }
}

#[cfg(test)]
mod planar_target_tests {
    use super::*;
    use crate::{
        curve_geometry::{tessellate_stroke, CubicBezier, PressureSegment},
        document::{Brush, Document, Point, Stroke, StrokeMaterial},
        shape_geometry,
    };

    #[test]
    fn rotated_bounded_planar_target_accepts_edges_and_rejects_outside_rays() {
        let target = PlanarTarget {
            plane: DrawingPlane {
                anchor: Vec3::new(0.7, -0.4, 1.2),
                rotation_degrees: Vec3::new(23.0, -31.0, 42.0),
                depth: 0.0,
            },
            half_extent: Some(glam::Vec2::new(1.5, 0.75)),
        };
        let (u, v, normal) = target.plane.basis();
        let origin = target.plane.origin();
        let edge = origin + u * 1.5 + v * 0.75;
        assert!(target.contains(edge), "the image boundary is inclusive");
        let edge_hit = target.hit(edge + normal * 2.0, -normal).unwrap();
        assert!(edge_hit.distance(edge) < 1e-5);

        let outside = origin + u * 1.501;
        assert!(!target.contains(outside));
        assert!(target.hit(outside + normal * 2.0, -normal).is_none());

        let infinite_guide = PlanarTarget {
            plane: target.plane,
            half_extent: None,
        };
        assert!(infinite_guide.contains(outside));
        assert!(infinite_guide
            .hit(outside + normal * 2.0, -normal)
            .is_some());
    }

    #[test]
    fn planar_tool_geometry_respects_rotated_image_bounds() {
        let target = PlanarTarget {
            plane: DrawingPlane {
                anchor: Vec3::new(-0.7, 0.4, 1.2),
                rotation_degrees: Vec3::new(23.0, -31.0, 42.0),
                depth: 0.0,
            },
            half_extent: Some(glam::Vec2::new(1.5, 0.75)),
        };
        let (u, v, normal) = target.plane.basis();
        let origin = target.plane.origin();

        let fill_positions = [
            origin - u * 0.5 - v * 0.25,
            origin + u * 0.5 - v * 0.25,
            origin + u * 0.5 + v * 0.25,
            origin - u * 0.5 + v * 0.25,
        ];
        let fill = Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            curve: None,
            material: StrokeMaterial::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: Some(crate::document::PlanarFill {
                offset: 0.01,
                gradient: None,
                texture: None,
                holes: Vec::new(),
                cylinder: None,
                mesh: None,
                region_source: None,
            }),
            group_id: None,
            layer_id: 1,
            id: 1,
            color: [0.3, 0.5, 0.7],
            radius: 0.02,
            brush: Brush {
                normal,
                tangent: u,
                ..Brush::default()
            },
            points: fill_positions
                .iter()
                .map(|position| Point {
                    position: *position,
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                })
                .collect(),
        };
        assert!(fill
            .points
            .iter()
            .all(|point| target.contains(point.position)));
        assert!(crate::polygon::triangulate(&fill).is_ok());

        let circle = shape_geometry::circle_from_center(
            target.plane,
            origin + u * 0.2 + v * 0.1,
            0.25,
            2,
            1,
            [0.2, 0.4, 0.6],
            0.02,
            0.7,
            Brush::default(),
        )
        .unwrap();
        assert!(circle
            .points
            .iter()
            .all(|point| target.contains(point.position)));

        let outside_ellipse = shape_geometry::ellipse_from_center(
            target.plane,
            origin + u * 1.4,
            0.25,
            0.2,
            3,
            1,
            [0.2, 0.4, 0.6],
            0.02,
            0.7,
            Brush::default(),
        )
        .unwrap();
        assert!(outside_ellipse
            .points
            .iter()
            .any(|point| !target.contains(point.position)));

        let prototype = Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            curve: None,
            material: StrokeMaterial::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id: 4,
            color: [0.2, 0.4, 0.6],
            radius: 0.02,
            brush: Brush::default(),
            points: Vec::new(),
        };
        let curve = CubicBezier {
            p0: origin + u * 1.2,
            p1: origin + u * 2.4,
            p2: origin + u * 2.4 + v * 0.1,
            p3: origin + u * 1.2 + v * 0.1,
        };
        let curve = tessellate_stroke(
            &[PressureSegment {
                curve,
                pressure_start: 0.5,
                pressure_end: 0.8,
            }],
            &prototype,
            0.005,
            4096,
        )
        .unwrap();
        assert!(target.contains(curve.points.first().unwrap().position));
        assert!(target.contains(curve.points.last().unwrap().position));
        assert!(curve
            .points
            .iter()
            .any(|point| !target.contains(point.position)));

        let ellipse = shape_geometry::ellipse_from_center(
            target.plane,
            origin - u * 0.2 - v * 0.1,
            0.35,
            0.2,
            5,
            1,
            [0.2, 0.4, 0.6],
            0.02,
            0.7,
            Brush::default(),
        )
        .unwrap();
        assert!(ellipse
            .points
            .iter()
            .all(|point| target.contains(point.position)));

        let inside_curve = CubicBezier {
            p0: origin - u * 0.5,
            p1: origin - u * 0.2 + v * 0.3,
            p2: origin + u * 0.2 + v * 0.3,
            p3: origin + u * 0.5,
        };
        let inside_curve = tessellate_stroke(
            &[PressureSegment {
                curve: inside_curve,
                pressure_start: 0.3,
                pressure_end: 0.9,
            }],
            &prototype,
            0.005,
            4096,
        )
        .unwrap();
        assert!(inside_curve
            .points
            .iter()
            .all(|point| target.contains(point.position)));

        let strokes = vec![fill, circle, ellipse, inside_curve];
        let mut edit_document = Document::default();
        for stroke in &strokes {
            let before = edit_document.strokes.clone();
            let edit = crate::editing::Edit::Add(stroke.clone());
            edit.apply(&mut edit_document, true);
            assert_eq!(edit_document.strokes.len(), before.len() + 1);
            assert_eq!(edit_document.strokes.last(), Some(stroke));
            edit.apply(&mut edit_document, false);
            assert_eq!(edit_document.strokes, before);
            edit.apply(&mut edit_document, true);
        }
        assert_eq!(edit_document.strokes, strokes);

        let native = serde_json::to_string(&edit_document).unwrap();
        let reloaded = Document::from_json(&native).unwrap();
        assert_eq!(reloaded.strokes, strokes);
        let recovery_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "planar-tool-session".into(),
            None,
            reloaded.clone(),
        );
        crate::recovery_store::write(recovery_dir.path(), &snapshot).unwrap();
        let recovered =
            crate::recovery_store::scan(recovery_dir.path(), "another-session").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(recovered.candidates[0].snapshot.document.strokes, strokes);

        let export = crate::export::encode(&reloaded, None).unwrap();
        assert_eq!(export.summary.strokes, strokes.len());
        assert!(
            gltf::Gltf::from_slice(&export.bytes)
                .unwrap()
                .meshes()
                .count()
                >= strokes.len()
        );
    }
}

#[cfg(test)]
mod planar_attachment_tests {
    use super::*;
    use crate::document::{guide_resources::GuideResource, Brush, Document, Point, Stroke};

    fn attached_stroke() -> Stroke {
        Stroke {
            guide_attachment: Some(7),
            model_surface: None,
            fill: None,
            material: crate::document::StrokeMaterial::Unlit,
            group_id: None,
            layer_id: 1,
            id: 31,
            color: [0.2, 0.4, 0.8],
            gradient: None,
            radius: 0.04,
            brush: Brush {
                normal: Vec3::Z,
                tangent: Vec3::X,
                ..Brush::default()
            },
            points: vec![
                Point {
                    position: Vec3::ZERO,
                    pressure: 0.35,
                    tilt: Some(Vec3::Y),
                    time_ms: 4,
                },
                Point {
                    position: Vec3::X,
                    pressure: 0.9,
                    tilt: Some(Vec3::Z),
                    time_ms: 19,
                },
            ],
            point_thickness: Some(vec![0.6, 1.2]),
            point_colors: Some(vec![None, Some([0.8, 0.3, 0.1])]),
            modifiers: None,
            curve: None,
        }
    }

    #[test]
    fn freeform_attachment_follows_control_edits_and_roundtrips() {
        let surface = FreeformSurface {
            center: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            directrix: [glam::vec2(-1.0, 0.0), glam::vec2(-0.3, 0.25),
                glam::vec2(0.3, -0.25), glam::vec2(1.0, 0.0)],
            depth: 2.0,
            bend: 0.15,
        };
        let mut old = guide(Vec3::ZERO, glam::Quat::IDENTITY);
        old.geometry = GuideGeometry::Freeform { surface };
        let mut next = surface;
        next.directrix[1].y += 0.65;
        next.depth = 2.75;
        next.bend = -0.35;
        next.rotation = glam::Quat::from_rotation_z(0.3);
        let mut new = old.clone();
        new.geometry = GuideGeometry::Freeform { surface: next };
        let mut stroke = attached_stroke();
        let parameters = [(0.18, 0.22), (0.83, 0.67)];
        for (sample, (u, v)) in stroke.points.iter_mut().zip(parameters) {
            sample.position = surface.point(u, v);
        }
        let mut free = stroke.clone();
        free.id += 1;
        free.guide_attachment = None;
        let mut doc = Document::default();
        doc.guide_resources.push(old.clone());
        doc.strokes = vec![stroke.clone(), free.clone()];
        let before = (vec![old.clone()], None, Vec::new(), None, None);
        let after = (vec![new], None, Vec::new(), None, None);
        let ink = attached_guide_changes(&doc, &before.0, &after.0).unwrap();
        let edit = Edit::Guides { before: before.clone(), after: after.clone(), ink };
        edit.apply(&mut doc, true);
        for (sample, (u, v)) in doc.strokes[0].points.iter().zip(parameters) {
            assert!(sample.position.distance(next.point(u, v)) < 1e-5);
        }
        assert_eq!(doc.strokes[1], free);
        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes, doc.strokes);
        let incompatible = vec![guide(Vec3::ZERO, glam::Quat::IDENTITY)];
        assert!(attached_guide_changes(&doc, &after.0, &incompatible).is_err());
        let detached = attached_guide_changes(&doc, &after.0, &[]).unwrap().unwrap().1;
        assert_eq!(detached[0].1.guide_attachment, None);
        assert_eq!(detached[0].1.points, doc.strokes[0].points);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes[0], stroke);
    }

    #[test]
    fn loft_attachment_spans_a_section_seam_and_rejects_topology_changes() {
        let section = |height: f32, z: f32| LoftSection {
            controls: [
                Vec3::new(-1.0, height, z),
                Vec3::new(-0.3, height + 0.2, z + 0.04),
                Vec3::new(0.3, height - 0.15, z - 0.04),
                Vec3::new(1.0, height, z),
            ],
        };
        let mut sections = [section(0.0, 0.0); 8];
        sections[1] = section(0.5, 1.0);
        sections[2] = section(-0.3, 2.0);
        let surface = LoftSurface {
            center: Vec3::new(0.3, -0.7, 0.5),
            rotation: glam::Quat::from_rotation_y(0.4),
            sections,
            section_count: 3,
            tension: 0.35,
        };
        let mut stabilizer = crate::surface::SurfaceStabilizer::default();
        stabilizer.start(Surface::Loft(surface));
        let mut stroke = attached_stroke();
        stroke.points.clear();
        stroke.point_thickness = None;
        stroke.point_colors = None;
        for v in [0.41, 0.47, 0.53, 0.62] {
            stroke.points.extend(stabilizer.push(Point {
                position: surface.point(0.37, v),
                pressure: 0.4 + v * 0.3,
                tilt: None,
                time_ms: 0,
            }, 0.0));
        }
        stroke.points.extend(stabilizer.finish());
        assert!(stroke.points.iter().any(|point| point.position.distance(surface.point(0.37, 0.5)) < 1e-5));
        let parameters: Vec<_> = stroke.points.iter()
            .map(|point| surface.coordinates(point.position).unwrap()).collect();
        let mut old = guide(Vec3::ZERO, glam::Quat::IDENTITY);
        old.geometry = GuideGeometry::Loft { surface };
        let mut next = surface;
        next.sections[1].controls[1].y += 0.45;
        next.tension = -0.5;
        next.center += Vec3::new(0.1, 0.3, -0.25);
        let mut new = old.clone();
        new.geometry = GuideGeometry::Loft { surface: next };
        assert!(new.validate().is_ok());
        let mut doc = Document::default();
        doc.guide_resources.push(old.clone());
        doc.strokes.push(stroke.clone());
        let before = (vec![old], None, Vec::new(), None, None);
        let after = (vec![new.clone()], None, Vec::new(), None, None);
        let ink = attached_guide_changes(&doc, &before.0, &after.0).unwrap();
        let edit = Edit::Guides { before, after, ink };
        edit.apply(&mut doc, true);
        for (sample, &(u, v)) in doc.strokes[0].points.iter().zip(&parameters) {
            assert!(sample.position.distance(next.point(u, v)) < 1e-5);
        }
        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes, doc.strokes);
        assert_eq!(crate::export::encode(&reopened, None).unwrap().summary.strokes, 1);
        let mut incompatible = new.clone();
        next.section_count = 2;
        incompatible.geometry = GuideGeometry::Loft { surface: next };
        assert!(attached_guide_changes(&doc, &[new], &[incompatible]).is_err());
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes[0], stroke);
    }

    #[test]
    fn cylinder_hatch_crosses_the_seam_and_follows_a_radius_edit() {
        use crate::document::{CylinderFillRegionSource, CylinderFillSurface, FillRegionSource, PlanarFill};

        let chart = CylinderFillSurface {
            center: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            radius: 1.0,
            height: 4.0,
            angle_origin: std::f32::consts::PI,
        };
        let mut source = attached_stroke();
        source.guide_attachment = None;
        source.point_colors = None;
        source.point_thickness = None;
        source.points = [(-0.45, -0.3), (0.45, -0.3), (0.45, 0.3), (-0.45, 0.3)]
            .map(|(x, y)| Point {
                position: chart.world_point(glam::vec2(x, y), 0.0).unwrap(),
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            }).to_vec();
        source.fill = Some(PlanarFill {
            cylinder: Some(chart),
            region_source: Some(FillRegionSource::Cylinder(CylinderFillRegionSource {
                guide_id: 7, seed_angle: chart.angle_origin, seed_height: 0.0,
                boundary_layer: None, gap_tolerance: 0.0,
            })),
            ..Default::default()
        });
        let mut doc = Document::default();
        doc.guide_resources.push(cylinder_guide(1.0, 4.0));
        doc.strokes.push(source.clone());
        let settings = crate::hatch::Settings { angle: 0.0, spacing: 0.2 };
        let ink = crate::hatch::Ink {
            brush: Brush::default(), color: [0.3; 3], radius: 0.02, layer: 1,
        };
        let hatches = crate::hatch::build(&doc, &source, settings, ink, 50).unwrap();
        assert!(hatches.len() >= 3);
        for stroke in &hatches {
            assert_eq!(stroke.guide_attachment, Some(7));
            assert!(stroke.points.first().unwrap().position.x > 0.0);
            assert!(stroke.points.last().unwrap().position.x < 0.0);
            assert!(stroke.points.windows(2).all(|pair| pair[0].position.distance(pair[1].position) <= 0.036));
            assert!(stroke.points.iter().all(|point| chart.chart_point(point.position).is_some()));
        }
        let add = Edit::AddBatch(hatches.clone());
        add.apply(&mut doc, true);
        let before = (doc.guide_resources.clone(), None, Vec::new(), None, None);
        let after = (vec![cylinder_guide(1.6, 4.0)], None, Vec::new(), None, None);
        let ink = attached_guide_changes(&doc, &before.0, &after.0).unwrap();
        let edit = Edit::Guides { before, after, ink };
        edit.apply(&mut doc, true);
        for stroke in &doc.strokes[1..] {
            assert!(stroke.points.iter().all(|point| {
                let radial = glam::vec2(point.position.x, point.position.z).length();
                (radial - 1.6).abs() < 1e-5
            }));
            assert!(matches!(stroke.brush.dabs.surface_target,
                Some(crate::dynamics::DabSurfaceTarget::Cylinder { radius, .. }) if radius == 1.6));
        }
        assert_eq!(doc.strokes[0], source);
        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes, doc.strokes);
        assert_eq!(crate::export::encode(&reopened, None).unwrap().summary.strokes, 1 + hatches.len());
        edit.apply(&mut doc, false);
        assert_eq!(&doc.strokes[1..], hatches.as_slice());
        add.apply(&mut doc, false);
        assert_eq!(doc.strokes, vec![source]);
    }

    fn guide(origin: Vec3, rotation: glam::Quat) -> GuideResource {
        GuideResource {
            id: 7,
            name: "Attached plane".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Plane { origin, rotation },
        }
    }

    fn cylinder_guide(radius: f32, height: f32) -> GuideResource {
        GuideResource {
            id: 7,
            name: "Attached cylinder".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Cylinder {
                center: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                radius,
                height,
            },
        }
    }

    #[test]
    fn planar_guide_motion_is_one_undoable_edit_and_survives_reopen_recovery_and_glb() {
        let old_guide = guide(Vec3::ZERO, glam::Quat::IDENTITY);
        let new_origin = Vec3::new(1.5, -0.25, 0.75);
        let new_rotation = glam::Quat::from_rotation_y(0.4);
        let new_guide = guide(new_origin, new_rotation);
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            1,
            image::Rgba([180, 110, 40, 200]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let grain = crate::brush_tip::BrushTipAsset::from_png(png.into_inner(), None).unwrap();
        let mut source = attached_stroke();
        source.brush.texture.mapping = crate::texture::TextureMapping {
            offset: [0.15, -0.25],
            rotation_degrees: 23.0,
            scale: [0.8, 1.3],
        };
        source.brush.texture.image = Some(crate::texture::ImageTextureSettings {
            asset_id: grain.id,
            mode: crate::texture::ImageTextureMode::CoverageMask,
            mask_source: crate::texture::ImageMaskSource::Alpha,
            tint_with_brush: true,
            ..Default::default()
        });
        let before_snapshot = vec![(0, source.clone())];
        let rotation = new_rotation * glam::Quat::IDENTITY.inverse();
        let old_frame = attachment_frame(std::slice::from_ref(&old_guide), 7).unwrap();
        let new_frame = attachment_frame(std::slice::from_ref(&new_guide), 7).unwrap();
        let after_snapshot = vec![(
            0,
            transform_attached_stroke(0, &source, Some(old_frame), Some(new_frame)).unwrap(),
        )];
        let mut doc = Document {
            guide_resources: vec![old_guide.clone()],
            active_guide: Some(7),
            strokes: vec![source.clone()],
            brush_tip_assets: vec![grain],
            ..Document::default()
        };
        let original_json = serde_json::to_value(&doc).unwrap();
        let edit = crate::editing::Edit::Guides {
            before: (vec![old_guide], Some(7), Vec::new(), None, None),
            after: (vec![new_guide], Some(7), Vec::new(), None, None),
            ink: Some(Box::new((before_snapshot, after_snapshot.clone()))),
        };

        edit.apply(&mut doc, true);
        let moved = doc.strokes[0].clone();
        assert_eq!(moved.guide_attachment, Some(7));
        assert_eq!(moved.points[0].position, new_origin);
        assert!((moved.points[1].position - (new_origin + rotation * Vec3::X)).length() < 1e-5);
        assert!((moved.brush.normal - rotation * source.brush.normal).length() < 1e-6);
        assert!((moved.brush.tangent - rotation * source.brush.tangent).length() < 1e-6);
        assert_eq!(moved.color, source.color);
        assert_eq!(moved.radius, source.radius);
        assert_eq!(moved.material, source.material);
        assert_eq!(moved.point_thickness, source.point_thickness);
        assert_eq!(moved.point_colors, source.point_colors);
        assert_eq!(moved.brush.texture, source.brush.texture);
        for (moved_point, source_point) in moved.points.iter().zip(&source.points) {
            assert_eq!(moved_point.pressure, source_point.pressure);
            assert_eq!(moved_point.time_ms, source_point.time_ms);
            match (moved_point.tilt, source_point.tilt) {
                (Some(moved_tilt), Some(source_tilt)) => {
                    assert!((moved_tilt - rotation * source_tilt).length() < 1e-6);
                }
                (None, None) => {}
                _ => panic!("guide rotation preserves optional tilt samples"),
            }
        }

        edit.apply(&mut doc, false);
        assert_eq!(serde_json::to_value(&doc).unwrap(), original_json);
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], after_snapshot[0].1);

        let native = serde_json::to_string(&doc).unwrap();
        let reopened = Document::from_json(&native).unwrap();
        assert_eq!(reopened.guide_resources, doc.guide_resources);
        assert_eq!(reopened.strokes, doc.strokes);
        let camera = crate::geometry::Camera::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let pick_position = (moved.points[0].position + moved.points[1].position) * 0.5;
        let cursor = crate::gizmo::project(&camera, rect, pick_position)
            .unwrap()
            .0;
        assert_eq!(
            crate::gizmo::pick(&reopened, &camera, rect, cursor),
            Some(31)
        );

        let recovery_root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "planar-guide-attachment".into(),
            None,
            reopened.clone(),
        );
        crate::recovery_store::write(recovery_root.path(), &snapshot).unwrap();
        let recovered =
            crate::recovery_store::scan(recovery_root.path(), "attachment-reader").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes,
            doc.strokes
        );

        let encoded = crate::export::encode(&reopened, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&encoded.bytes).unwrap();
        let exported_positions = gltf
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .reader(|_| gltf.blob.as_deref())
            .read_positions()
            .unwrap()
            .collect::<Vec<_>>();
        assert!(!exported_positions.is_empty());
        assert!(exported_positions
            .iter()
            .all(|position| position.iter().all(|value| value.is_finite())));
    }

    #[test]
    fn native_merge_remaps_planar_attachment_and_undo_removes_inserted_ink() {
        let source = Document {
            guide_resources: vec![guide(Vec3::ZERO, glam::Quat::IDENTITY)],
            active_guide: Some(7),
            strokes: vec![attached_stroke()],
            ..Document::default()
        };
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source_path = root.path().join("attached-source.sketch.json");
        std::fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();
        let assets = root.path().join("insert-assets");
        std::fs::create_dir_all(&assets).unwrap();
        let mut destination = Document::default();
        let inserted = crate::app::native_merge::prepare_insert(
            &destination,
            &source_path,
            &assets,
            100,
            10,
            1_000,
            Vec3::new(2.0, 0.0, 0.0),
            false,
            &HashMap::new(),
        )
        .unwrap();
        let edit = crate::editing::Edit::Merge(inserted.edit);
        edit.apply(&mut destination, true);
        let inserted_guide = destination.guide_resources.last().unwrap().id;
        let inserted_stroke = destination.strokes.last().unwrap();
        assert_ne!(inserted_guide, 7);
        assert_eq!(inserted_stroke.guide_attachment, Some(inserted_guide));
        assert_eq!(inserted_stroke.points[0].position, Vec3::new(2.0, 0.0, 0.0));

        edit.apply(&mut destination, false);
        assert!(destination.guide_resources.is_empty());
        assert!(destination.strokes.is_empty());
    }

    #[test]
    fn cylinder_radius_edit_preserves_seam_ink_and_detach_is_reversible_in_place() {
        let old_guide = cylinder_guide(1.0, 4.0);
        let new_guide = cylinder_guide(1.2, 4.0);
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            1,
            image::Rgba([80, 170, 210, 160]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let grain = crate::brush_tip::BrushTipAsset::from_png(png.into_inner(), None).unwrap();
        let mut source = attached_stroke();
        source.points[0].position = Vec3::new(-1.05, -0.2, -0.02);
        source.points[1].position = Vec3::new(-1.05, 0.2, 0.02);
        source.brush.texture.mapping = crate::texture::TextureMapping {
            offset: [-0.1, 0.3],
            rotation_degrees: -16.0,
            scale: [1.2, 0.7],
        };
        source.brush.texture.image = Some(crate::texture::ImageTextureSettings {
            asset_id: grain.id,
            mode: crate::texture::ImageTextureMode::CoverageMask,
            mask_source: crate::texture::ImageMaskSource::LuminanceTimesAlpha,
            contrast: 1.4,
            strength: 0.8,
            ..Default::default()
        });
        let before_snapshot = vec![(0, source.clone())];
        let old_frame = attachment_frame(std::slice::from_ref(&old_guide), 7).unwrap();
        let new_frame = attachment_frame(std::slice::from_ref(&new_guide), 7).unwrap();
        let after_snapshot = vec![(
            0,
            transform_attached_stroke(0, &source, Some(old_frame), Some(new_frame)).unwrap(),
        )];
        let mut doc = Document {
            guide_resources: vec![old_guide.clone()],
            active_guide: Some(7),
            strokes: vec![source.clone()],
            brush_tip_assets: vec![grain],
            ..Document::default()
        };
        let original = serde_json::to_value(&doc).unwrap();
        let guide_edit = crate::editing::Edit::Guides {
            before: (vec![old_guide], Some(7), Vec::new(), None, None),
            after: (vec![new_guide], Some(7), Vec::new(), None, None),
            ink: Some(Box::new((before_snapshot, after_snapshot.clone()))),
        };
        guide_edit.apply(&mut doc, true);
        let moved = doc.strokes[0].clone();
        assert_eq!(moved.guide_attachment, Some(7));
        assert!(
            (moved.points[0].position.x.hypot(moved.points[0].position.z) - 1.25019).abs() < 1e-4
        );
        assert!(
            (moved.points[1].position.x.hypot(moved.points[1].position.z) - 1.25019).abs() < 1e-4
        );
        assert!(moved.points[0].position.distance(moved.points[1].position) < 0.41);
        assert_eq!(
            moved
                .points
                .iter()
                .map(|point| (point.pressure, point.time_ms))
                .collect::<Vec<_>>(),
            source
                .points
                .iter()
                .map(|point| (point.pressure, point.time_ms))
                .collect::<Vec<_>>()
        );
        assert_eq!(moved.brush.normal, source.brush.normal);
        assert_eq!(moved.brush.tangent, source.brush.tangent);
        assert_eq!(moved.brush.texture, source.brush.texture);

        guide_edit.apply(&mut doc, false);
        assert_eq!(serde_json::to_value(&doc).unwrap(), original);
        guide_edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], after_snapshot[0].1);

        let detach_before = vec![(0, doc.strokes[0].clone())];
        let mut detached_stroke = doc.strokes[0].clone();
        detached_stroke.guide_attachment = None;
        let detach_after = vec![(0, detached_stroke)];
        let detach = crate::editing::Edit::Change {
            before: detach_before,
            after: detach_after,
        };
        detach.apply(&mut doc, true);
        assert_eq!(doc.strokes[0].guide_attachment, None);
        assert_eq!(
            doc.strokes[0].points, moved.points,
            "detaching leaves world-space geometry in place"
        );
        detach.apply(&mut doc, false);
        assert_eq!(doc.strokes[0], moved);

        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes[0], moved);
        let camera = crate::geometry::Camera::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let midpoint = (moved.points[0].position + moved.points[1].position) * 0.5;
        let cursor = crate::gizmo::project(&camera, rect, midpoint).unwrap().0;
        assert_eq!(
            crate::gizmo::pick(&reopened, &camera, rect, cursor),
            Some(31)
        );

        let recovery_root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "cylinder-guide-attachment".into(),
            None,
            reopened.clone(),
        );
        crate::recovery_store::write(recovery_root.path(), &snapshot).unwrap();
        let recovered =
            crate::recovery_store::scan(recovery_root.path(), "cylinder-reader").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(recovered.candidates[0].snapshot.document.strokes[0], moved);

        let export = crate::export::encode(&reopened, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&export.bytes).unwrap();
        let positions = gltf
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .reader(|_| gltf.blob.as_deref())
            .read_positions()
            .unwrap()
            .collect::<Vec<_>>();
        assert!(positions.iter().any(|position| {
            let radial = position[0].hypot(position[2]);
            (1.20..=1.30).contains(&radial)
        }));
    }
}
