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

    pub(super) fn shape_draft_preview(&self) -> Option<&Stroke> {
        self.shape_tool
            .draft
            .as_ref()
            .and_then(|draft| draft.preview.as_ref())
    }

    pub(super) fn start_shape_tool(&mut self, mode: ShapeMode) {
        if self.shape_target().is_none() {
            self.status =
                "Circles and ellipses require a plane or cylinder guide. Activate a supported guide first.".into();
            return;
        }
        self.finish();
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
        let cylinder_target = matches!(self.shape_target(), Some(ShapeTarget::Cylinder(_)));
        ui.strong(format!("{} TOOL", mode.label().to_ascii_uppercase()));
        ui.label("Shapes follow the active plane or cylinder and use uniform start pressure.");
        ui.checkbox(&mut self.shape_tool.precise, "Use typed dimensions");
        if self.shape_tool.precise {
            match mode {
                ShapeMode::Circle => {
                    ui_style::numeric_row(
                        ui,
                        "Radius",
                        "shape-radius",
                        &mut self.shape_tool.radius,
                        Some(0.001..=10000.0),
                        "wu",
                    );
                }
                ShapeMode::Ellipse => {
                    ui_style::numeric_row(
                        ui,
                        "Width",
                        "shape-width",
                        &mut self.shape_tool.width,
                        Some(0.002..=20000.0),
                        "wu",
                    );
                    ui_style::numeric_row(
                        ui,
                        "Height",
                        "shape-height",
                        &mut self.shape_tool.height,
                        Some(0.002..=20000.0),
                        "wu",
                    );
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
        ui.small("Escape cancels the preview. Release creates one editable, undoable outline.");
    }

    pub(super) fn shape_input(&mut self, pos: Pos2, pressure: f32, rect: Rect, begin: bool) {
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
                    self.status = "Shape extends beyond the active image bounds.".into();
                }
                Err(error) => {
                    draft.preview = None;
                    self.status = error;
                }
            }
        }
    }

    pub(super) fn release_shape(&mut self, pos: Pos2, pressure: f32, rect: Rect) {
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
