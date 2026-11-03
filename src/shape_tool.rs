use super::guide_resource_ui::PlanarTarget;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShapeMode {
    Circle,
    Ellipse,
}

impl ShapeMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Circle => "Circle",
            Self::Ellipse => "Ellipse",
        }
    }
}

struct ShapeDraft {
    mode: ShapeMode,
    target: ShapeTarget,
    first: Vec3,
    current: Vec3,
    pressure: f32,
    color: [f32; 3],
    radius: f32,
    brush: Brush,
    preview: Option<Stroke>,
    editing_stroke_id: Option<u64>,
}

#[derive(Clone, Copy)]
enum ShapeTarget {
    Planar(PlanarTarget),
    Cylinder(crate::surface::Cylinder),
}

impl ShapeTarget {
    fn hit(self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        match self {
            Self::Planar(target) => target.hit(origin, direction),
            Self::Cylinder(cylinder) => cylinder.hit(origin, direction),
        }
    }

    fn contains(self, point: Vec3) -> bool {
        match self {
            Self::Planar(target) => target.contains(point),
            Self::Cylinder(cylinder) => cylinder
                .parameterize(point, None)
                .is_some_and(|parameter| parameter.y.abs() <= cylinder.height * 0.5),
        }
    }
}

pub(super) struct ShapeTool {
    pub mode: Option<ShapeMode>,
    pub precise: bool,
    pub radius: f32,
    pub width: f32,
    pub height: f32,
    draft: Option<ShapeDraft>,
}

impl Default for ShapeTool {
    fn default() -> Self {
        Self {
            mode: None,
            precise: false,
            radius: 0.5,
            width: 1.0,
            height: 0.75,
            draft: None,
        }
    }
}

impl SketchApp {
    pub(super) fn shape_draft_active(&self) -> bool {
        self.shape_tool.draft.is_some()
    }

    pub(super) fn shape_edit_active(&self) -> bool {
        self.shape_tool
            .draft
            .as_ref()
            .is_some_and(|draft| draft.editing_stroke_id.is_some())
    }

    pub(super) fn shape_draft_preview(&self) -> Option<&Stroke> {
        self.shape_tool
            .draft
            .as_ref()
            .and_then(|draft| draft.preview.as_ref())
    }

    fn has_selected_curved_shape(&self) -> bool {
        if self.selected.len() != 1 {
            return false;
        }
        self.selected.iter().any(|id| {
            self.doc.strokes.iter().any(|stroke| {
                stroke.id == *id
                    && self.doc.editable(stroke)
                    && stroke
                        .curve
                        .as_ref()
                        .is_some_and(|curve| curve.shape.is_some() && curve.is_valid())
            })
        })
    }

    pub(super) fn start_shape_tool(&mut self, mode: ShapeMode) {
        if self.shape_target().is_none() && !self.has_selected_curved_shape() {
            self.status =
                "Circles and ellipses require a plane or cylinder guide. Activate a supported guide first.".into();
            return;
        }
        self.finish();
        self.cancel_curve_tool();
        self.cancel_line_tool();
        self.cancel_fill_tool();
        self.shape_tool.draft = None;
        self.shape_tool.mode = Some(mode);
        self.select_mode = false;
        self.eraser = false;
        self.status = format!(
            "{} tool ready. Drag on the active plane or cylinder to preview.",
            mode.label()
        );
    }

    fn shape_target(&self) -> Option<ShapeTarget> {
        if self.doc.active_image_plane.is_some() {
            return self.planar_drawing_target().map(ShapeTarget::Planar);
        }
        if let Some(geometry) = self.saved_guide_geometry() {
            return match geometry {
                crate::document::guide_resources::GuideGeometry::Cylinder {
                    center,
                    rotation,
                    radius,
                    height,
                } => {
                    let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
                    Some(ShapeTarget::Cylinder(crate::surface::Cylinder {
                        center: *center,
                        rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
                        radius: *radius,
                        height: *height,
                    }))
                }
                crate::document::guide_resources::GuideGeometry::Plane { .. } => {
                    self.planar_drawing_target().map(ShapeTarget::Planar)
                }
                _ => None,
            };
        }
        if self.use_cylinder {
            Some(ShapeTarget::Cylinder(self.cylinder))
        } else {
            self.planar_drawing_target().map(ShapeTarget::Planar)
        }
    }

    fn shape_target_from_snapshot(
        target: crate::dynamics::DabSurfaceTarget,
    ) -> Option<ShapeTarget> {
        let crate::dynamics::DabSurfaceTarget::Cylinder {
            center,
            rotation,
            radius,
            height,
        } = target
        else {
            return None;
        };
        let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
        Some(ShapeTarget::Cylinder(crate::surface::Cylinder {
            center,
            rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
            radius,
            height,
        }))
    }

    pub(super) fn open_selected_curved_shape(&mut self) {
        if self.shape_tool.draft.is_some() {
            self.status = "Apply or cancel the current shape preview first.".into();
            return;
        }
        let Some(stroke) = self.doc.strokes.iter().find(|stroke| {
            self.selected.contains(&stroke.id)
                && self.doc.editable(stroke)
                && stroke
                    .curve
                    .as_ref()
                    .is_some_and(|curve| curve.shape.is_some() && curve.is_valid())
        }) else {
            self.status = "Select one visible, unlocked cylinder shape to edit.".into();
            return;
        };
        let Some(curve) = stroke.curve.as_ref() else {
            return;
        };
        let (Some(shape), Some(snapshot)) = (curve.shape, curve.surface_target) else {
            return;
        };
        let Some(target @ ShapeTarget::Cylinder(cylinder)) =
            Self::shape_target_from_snapshot(snapshot)
        else {
            self.status = "The selected shape has an unsupported captured target.".into();
            return;
        };
        let Some(center) = cylinder.point_from_parameter(shape.center) else {
            self.status = "The saved shape center is outside its cylinder.".into();
            return;
        };
        self.shape_tool.mode = Some(match shape.kind {
            crate::document::CurvedShapeKind::Circle => ShapeMode::Circle,
            crate::document::CurvedShapeKind::Ellipse => ShapeMode::Ellipse,
        });
        self.shape_tool.precise = true;
        self.shape_tool.radius = shape.radius_u;
        self.shape_tool.width = shape.radius_u * 2.0;
        self.shape_tool.height = shape.radius_v * 2.0;
        self.shape_tool.draft = Some(ShapeDraft {
            mode: self.shape_tool.mode.unwrap(),
            target,
            first: center,
            current: center,
            pressure: stroke.points.first().map_or(1.0, |point| point.pressure),
            color: stroke.color,
            radius: stroke.radius,
            brush: stroke.brush,
            preview: Some(stroke.clone()),
            editing_stroke_id: Some(stroke.id),
        });
        self.status =
            "Cylinder shape edit preview ready. Apply to commit once or Escape to cancel.".into();
    }

    pub(super) fn refresh_curved_shape_edit(&mut self) {
        let Some(draft) = self.shape_tool.draft.as_ref() else {
            return;
        };
        let (Some(id), ShapeTarget::Cylinder(cylinder)) = (draft.editing_stroke_id, draft.target)
        else {
            return;
        };
        let Some(layer_id) = self
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.id == id)
            .map(|stroke| stroke.layer_id)
        else {
            self.status = "The source shape no longer exists. Reopen it to edit.".into();
            return;
        };
        let Some(center) = self
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.id == id)
            .and_then(|stroke| stroke.curve.as_ref())
            .and_then(|curve| curve.shape)
            .map(|shape| shape.center)
        else {
            self.status = "The source shape no longer has semantic dimensions.".into();
            return;
        };
        let tolerance = (self.camera.world_per_pixel(draft.first, 900.0) * 0.75).clamp(
            crate::curve_geometry::MIN_TOLERANCE_WORLD,
            crate::curve_geometry::MAX_TOLERANCE_WORLD,
        );
        let preview = match draft.mode {
            ShapeMode::Circle => crate::shape_geometry::cylinder_circle(
                cylinder,
                center,
                self.shape_tool.radius,
                id,
                layer_id,
                draft.color,
                draft.radius,
                draft.pressure,
                draft.brush,
                tolerance,
            ),
            ShapeMode::Ellipse => crate::shape_geometry::cylinder_ellipse(
                cylinder,
                center,
                self.shape_tool.width * 0.5,
                self.shape_tool.height * 0.5,
                id,
                layer_id,
                draft.color,
                draft.radius,
                draft.pressure,
                draft.brush,
                tolerance,
            ),
        };
        match preview {
            Ok(preview) => {
                if let Some(draft) = &mut self.shape_tool.draft {
                    draft.preview = Some(preview);
                }
            }
            Err(error) => {
                if let Some(draft) = &mut self.shape_tool.draft {
                    draft.preview = None;
                }
                self.status = error;
            }
        }
    }

    pub(super) fn commit_curved_shape_edit(&mut self) {
        let Some(mut draft) = self.shape_tool.draft.take() else {
            return;
        };
        let Some(id) = draft.editing_stroke_id else {
            self.shape_tool.draft = Some(draft);
            return;
        };
        let Some(mut after) = draft.preview.take() else {
            self.status = "Shape edit is outside its captured cylinder bounds.".into();
            self.shape_tool.draft = Some(draft);
            return;
        };
        let Some(index) = self.doc.strokes.iter().position(|stroke| stroke.id == id) else {
            self.status = "The source shape no longer exists. Reopen it to edit.".into();
            return;
        };
        let before = self.doc.strokes[index].clone();
        if !self.doc.editable(&before)
            || after.points.len()
                + self
                    .doc
                    .strokes
                    .iter()
                    .map(|stroke| stroke.points.len())
                    .sum::<usize>()
                    .saturating_sub(before.points.len())
                > 2_000_000
        {
            self.status = "Shape edit exceeds the editable layer or document limits.".into();
            self.shape_tool.draft = Some(draft);
            return;
        }
        after.id = id;
        if after == before {
            self.status = "Shape is unchanged.".into();
            return;
        }
        let edit = Edit::Change {
            before: vec![(index, before)],
            after: vec![(index, after)],
        };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit_labeled("Edit cylinder shape");
        self.renderer.invalidate();
        self.status = "Cylinder shape edit applied as one undo step.".into();
    }

    pub(super) fn cancel_shape_draft(&mut self) {
        if self.shape_tool.draft.take().is_some() {
            self.status = "Shape preview cancelled.".into();
        }
        self.clear_planar_snap();
    }

    pub(super) fn shape_controls(&mut self, ui: &mut egui::Ui) {
        let Some(mode) = self.shape_tool.mode else {
            return;
        };
        let cylinder_target = self
            .shape_tool
            .draft
            .as_ref()
            .is_some_and(|draft| matches!(draft.target, ShapeTarget::Cylinder(_)))
            || matches!(self.shape_target(), Some(ShapeTarget::Cylinder(_)));
        let editing = self
            .shape_tool
            .draft
            .as_ref()
            .is_some_and(|draft| draft.editing_stroke_id.is_some());
        ui.strong(format!("{} TOOL", mode.label().to_ascii_uppercase()));
        ui.label("Shapes follow the active plane or cylinder and use uniform start pressure.");
        if self.shape_tool.draft.is_none()
            && self.has_selected_curved_shape()
            && ui.button("Edit selected cylinder shape").clicked()
        {
            self.open_selected_curved_shape();
            return;
        }
        let mut dimensions_changed = false;
        if !editing {
            ui.checkbox(&mut self.shape_tool.precise, "Use typed dimensions");
        }
        if self.shape_tool.precise {
            match mode {
                ShapeMode::Circle => {
                    dimensions_changed |= ui_style::numeric_row(
                        ui,
                        "Radius",
                        "shape-radius",
                        &mut self.shape_tool.radius,
                        Some(0.001..=10000.0),
                        "wu",
                    )
                    .changed();
                }
                ShapeMode::Ellipse => {
                    dimensions_changed |= ui_style::numeric_row(
                        ui,
                        "Width",
                        "shape-width",
                        &mut self.shape_tool.width,
                        Some(0.002..=20000.0),
                        "wu",
                    )
                    .changed();
                    dimensions_changed |= ui_style::numeric_row(
                        ui,
                        "Height",
                        "shape-height",
                        &mut self.shape_tool.height,
                        Some(0.002..=20000.0),
                        "wu",
                    )
                    .changed();
                }
            }
            ui.small("Click a center point to place the typed shape.");
            if cylinder_target {
                ui.small("Cylinder dimensions use the unwrapped surface chart: circumferential arc length and axial distance. Shapes wrap across the angular seam and stop at the finite ends.");
            }
        } else if mode == ShapeMode::Circle {
            ui.small("Drag from the circle center to its edge.");
            if cylinder_target {
                ui.small("Radius is measured in the cylinder surface chart and may cross the angular seam.");
            }
        } else {
            ui.small("Drag opposite corners of the ellipse bounds.");
            if cylinder_target {
                ui.small("Ellipse width follows circumferential arc length; height follows the cylinder axis.");
            }
        }
        if editing && dimensions_changed {
            self.refresh_curved_shape_edit();
        }
        if editing {
            ui.horizontal(|ui| {
                if ui.button("Apply edit").clicked() {
                    self.commit_curved_shape_edit();
                }
                if ui.button("Cancel").clicked() {
                    self.cancel_shape_draft();
                }
            });
            ui.small("The source remains unchanged until Apply; Escape cancels the preview.");
            return;
        }
        ui.small("Escape cancels the preview. Release creates one editable, undoable outline.");
    }

    pub(super) fn shape_input(&mut self, pos: Pos2, pressure: f32, rect: Rect, begin: bool) {
        if self
            .shape_tool
            .draft
            .as_ref()
            .is_some_and(|draft| draft.editing_stroke_id.is_some())
        {
            return;
        }
        if begin {
            self.finish();
            self.cancel_shape_draft();
            let Some(target) = self.shape_target() else {
                self.status =
                    "Circles and ellipses require a plane or cylinder guide. Activate a supported guide first."
                        .into();
                return;
            };
            if !self.doc.active_editable() {
                self.status = "Choose a visible unlocked layer for the shape.".into();
                return;
            }
            let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
            let Some(world) = target.hit(origin, direction) else {
                return;
            };
            let world = if let ShapeTarget::Planar(planar) = target {
                self.prepare_planar_snap(planar, rect, &HashSet::new());
                self.snap_planar_point(world, pos, planar, rect, None)
            } else {
                world
            };
            let mut brush = self.brush;
            brush.dynamics.taper_start = 0.0;
            brush.dynamics.taper_end = 0.0;
            self.shape_tool.draft = Some(ShapeDraft {
                mode: self.shape_tool.mode.expect("shape mode active"),
                target,
                first: world,
                current: world,
                pressure: pressure.clamp(0.0, 1.0),
                color: self.color,
                radius: self.radius,
                brush,
                preview: None,
                editing_stroke_id: None,
            });
        }
        let Some(draft) = self.shape_tool.draft.as_ref() else {
            return;
        };
        if !rect.contains(pos) {
            return;
        }
        let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
        let target = draft.target;
        let first = draft.first;
        let mode = draft.mode;
        let color = draft.color;
        let radius = draft.radius;
        let pressure = draft.pressure;
        let brush = draft.brush;
        if let Some(mut world) = target.hit(origin, direction) {
            if let ShapeTarget::Planar(planar) = target {
                let excluded = HashSet::new();
                if self.planar_snap_index_stale(planar, rect, &excluded) {
                    self.prepare_planar_snap(planar, rect, &excluded);
                }
                world = self.snap_planar_point(world, pos, planar, rect, None);
            }
            let id = self.next_id;
            let result = match (target, mode, self.shape_tool.precise) {
                (ShapeTarget::Cylinder(cylinder), ShapeMode::Circle, true) => {
                    let Some(center) = cylinder.parameterize(first, None) else {
                        return;
                    };
                    crate::shape_geometry::cylinder_circle(
                        cylinder,
                        center,
                        self.shape_tool.radius,
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                        (self.camera.world_per_pixel(first, rect.height().max(1.0)) * 0.75).clamp(
                            crate::curve_geometry::MIN_TOLERANCE_WORLD,
                            crate::curve_geometry::MAX_TOLERANCE_WORLD,
                        ),
                    )
                }
                (ShapeTarget::Cylinder(cylinder), ShapeMode::Circle, false) => {
                    let Some(center) = cylinder.parameterize(first, None) else {
                        return;
                    };
                    let Some(edge) = cylinder.parameterize(world, Some(center)) else {
                        return;
                    };
                    crate::shape_geometry::cylinder_circle(
                        cylinder,
                        center,
                        center.distance(edge),
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                        self.camera
                            .world_per_pixel(first, rect.height().max(1.0))
                            .clamp(
                                crate::curve_geometry::MIN_TOLERANCE_WORLD,
                                crate::curve_geometry::MAX_TOLERANCE_WORLD,
                            ),
                    )
                }
                (ShapeTarget::Cylinder(cylinder), ShapeMode::Ellipse, true) => {
                    let Some(center) = cylinder.parameterize(first, None) else {
                        return;
                    };
                    crate::shape_geometry::cylinder_ellipse(
                        cylinder,
                        center,
                        self.shape_tool.width * 0.5,
                        self.shape_tool.height * 0.5,
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                        self.camera
                            .world_per_pixel(first, rect.height().max(1.0))
                            .clamp(
                                crate::curve_geometry::MIN_TOLERANCE_WORLD,
                                crate::curve_geometry::MAX_TOLERANCE_WORLD,
                            ),
                    )
                }
                (ShapeTarget::Cylinder(cylinder), ShapeMode::Ellipse, false) => {
                    let Some(first_parameter) = cylinder.parameterize(first, None) else {
                        return;
                    };
                    let Some(opposite) = cylinder.parameterize(world, Some(first_parameter)) else {
                        return;
                    };
                    let center = (first_parameter + opposite) * 0.5;
                    crate::shape_geometry::cylinder_ellipse(
                        cylinder,
                        center,
                        (opposite.x - first_parameter.x).abs() * 0.5,
                        (opposite.y - first_parameter.y).abs() * 0.5,
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                        self.camera
                            .world_per_pixel(first, rect.height().max(1.0))
                            .clamp(
                                crate::curve_geometry::MIN_TOLERANCE_WORLD,
                                crate::curve_geometry::MAX_TOLERANCE_WORLD,
                            ),
                    )
                }
                (ShapeTarget::Planar(target), ShapeMode::Circle, true) => {
                    crate::shape_geometry::circle_from_center(
                        target.plane,
                        first,
                        self.shape_tool.radius,
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                    )
                }
                (ShapeTarget::Planar(target), ShapeMode::Ellipse, true) => {
                    crate::shape_geometry::ellipse_from_center(
                        target.plane,
                        first,
                        self.shape_tool.width * 0.5,
                        self.shape_tool.height * 0.5,
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                    )
                }
                (ShapeTarget::Planar(target), ShapeMode::Circle, false) => {
                    crate::shape_geometry::circle_from_center_edge(
                        target.plane,
                        first,
                        world,
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                    )
                }
                (ShapeTarget::Planar(target), ShapeMode::Ellipse, false) => {
                    crate::shape_geometry::ellipse_from_bounds(
                        target.plane,
                        first,
                        world,
                        id,
                        self.doc.active_layer,
                        color,
                        radius,
                        pressure,
                        brush,
                    )
                }
            };
            let draft = self
                .shape_tool
                .draft
                .as_mut()
                .expect("shape draft remains active");
            draft.current = world;
            match result {
                Ok(stroke)
                    if stroke
                        .points
                        .iter()
                        .all(|point| draft.target.contains(point.position)) =>
                {
                    draft.preview = Some(stroke)
                }
                Ok(_) => {
                    draft.preview = None;
                    self.status = "Shape extends beyond the active drawing surface bounds.".into();
                }
                Err(error) => {
                    draft.preview = None;
                    self.status = error;
                }
            }
        }
    }

    pub(super) fn release_shape(&mut self, pos: Pos2, pressure: f32, rect: Rect) {
        if self
            .shape_tool
            .draft
            .as_ref()
            .is_some_and(|draft| draft.editing_stroke_id.is_some())
        {
            return;
        }
        self.shape_input(pos, pressure, rect, false);
        let Some(mut draft) = self.shape_tool.draft.take() else {
            self.clear_planar_snap();
            return;
        };
        self.clear_planar_snap();
        let Some(mut stroke) = draft.preview.take() else {
            self.status = "Shape was too small or outside the valid document range.".into();
            return;
        };
        if self.next_id == u64::MAX
            || self.doc.strokes.len() >= 100_000
            || self
                .doc
                .strokes
                .iter()
                .map(|item| item.points.len())
                .sum::<usize>()
                + stroke.points.len()
                > 2_000_000
        {
            self.status = "Shape could not be committed: document limits reached.".into();
            return;
        }
        stroke.id = self.next_id;
        self.next_id += 1;
        let edit = Edit::Add(stroke);
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status = format!("{} created as editable outline.", draft.mode.label());
    }

    pub(super) fn shape_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(preview) = self
            .shape_tool
            .draft
            .as_ref()
            .and_then(|draft| draft.preview.as_ref())
        else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let points: Vec<_> = preview
            .points
            .iter()
            .filter_map(|point| {
                crate::gizmo::project(&self.camera, rect, point.position).map(|(screen, _)| screen)
            })
            .collect();
        if points.len() > 1 {
            for pair in points.windows(2) {
                painter.line_segment(
                    [pair[0], pair[1]],
                    egui::Stroke::new(
                        2.0_f32,
                        ui_style::canvas_overlay_accent(
                            self.presentation.preferences.background,
                            self.presentation.preferences.background_image.is_some(),
                        ),
                    ),
                );
            }
            if let (Some(first), Some(last)) = (points.first(), points.last()) {
                painter.line_segment(
                    [*last, *first],
                    egui::Stroke::new(
                        2.0_f32,
                        ui_style::canvas_overlay_accent(
                            self.presentation.preferences.background,
                            self.presentation.preferences.background_image.is_some(),
                        ),
                    ),
                );
            }
        }
    }
}
