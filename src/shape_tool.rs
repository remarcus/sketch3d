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
    target: PlanarTarget,
    plane: DrawingPlane,
    first: Vec3,
    current: Vec3,
    pressure: f32,
    color: [f32; 3],
    radius: f32,
    brush: Brush,
    preview: Option<Stroke>,
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

    pub(super) fn start_shape_tool(&mut self, mode: ShapeMode) {
        if self.planar_drawing_target().is_none() {
            self.status =
                "Circles and ellipses require a plane guide. Activate a planar guide first.".into();
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
            "{} tool ready. Drag on the active planar target to preview.",
            mode.label()
        );
    }

    pub(super) fn cancel_shape_draft(&mut self) {
        if self.shape_tool.draft.take().is_some() {
            self.status = "Shape preview cancelled.".into();
        }
    }

    pub(super) fn shape_controls(&mut self, ui: &mut egui::Ui) {
        let Some(mode) = self.shape_tool.mode else {
            return;
        };
        ui.strong(format!("{} TOOL", mode.label().to_ascii_uppercase()));
        ui.label("Shapes follow the active plane or image plane and use uniform start pressure.");
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
        } else if mode == ShapeMode::Circle {
            ui.small("Drag from the circle center to its edge.");
        } else {
            ui.small("Drag opposite corners of the ellipse bounds.");
        }
        ui.small("Escape cancels the preview. Release creates one editable, undoable outline.");
    }

    pub(super) fn shape_input(&mut self, pos: Pos2, pressure: f32, rect: Rect, begin: bool) {
        if begin {
            self.finish();
            self.cancel_shape_draft();
            let Some(target) = self.planar_drawing_target() else {
                self.status =
                    "Circles and ellipses require a plane guide. Activate a planar guide first."
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
            let plane = target.plane;
            let mut brush = self.brush;
            brush.dynamics.taper_start = 0.0;
            brush.dynamics.taper_end = 0.0;
            self.shape_tool.draft = Some(ShapeDraft {
                mode: self.shape_tool.mode.expect("shape mode active"),
                target,
                plane,
                first: world,
                current: world,
                pressure: pressure.clamp(0.0, 1.0),
                color: self.color,
                radius: self.radius,
                brush,
                preview: None,
            });
        }
        let Some(draft) = self.shape_tool.draft.as_mut() else {
            return;
        };
        if !rect.contains(pos) {
            return;
        }
        let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
        if let Some(world) = draft.target.hit(origin, direction) {
            draft.current = world;
            let id = self.next_id;
            let result = match (draft.mode, self.shape_tool.precise) {
                (ShapeMode::Circle, true) => crate::shape_geometry::circle_from_center(
                    draft.plane,
                    draft.first,
                    self.shape_tool.radius,
                    id,
                    self.doc.active_layer,
                    draft.color,
                    draft.radius,
                    draft.pressure,
                    draft.brush,
                ),
                (ShapeMode::Circle, false) => crate::shape_geometry::circle_from_center_edge(
                    draft.plane,
                    draft.first,
                    draft.current,
                    id,
                    self.doc.active_layer,
                    draft.color,
                    draft.radius,
                    draft.pressure,
                    draft.brush,
                ),
                (ShapeMode::Ellipse, true) => crate::shape_geometry::ellipse_from_center(
                    draft.plane,
                    draft.first,
                    self.shape_tool.width * 0.5,
                    self.shape_tool.height * 0.5,
                    id,
                    self.doc.active_layer,
                    draft.color,
                    draft.radius,
                    draft.pressure,
                    draft.brush,
                ),
                (ShapeMode::Ellipse, false) => crate::shape_geometry::ellipse_from_bounds(
                    draft.plane,
                    draft.first,
                    draft.current,
                    id,
                    self.doc.active_layer,
                    draft.color,
                    draft.radius,
                    draft.pressure,
                    draft.brush,
                ),
            };
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
            return;
        };
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
                        ui_style::canvas_accent(self.presentation.preferences.background),
                    ),
                );
            }
            if let (Some(first), Some(last)) = (points.first(), points.last()) {
                painter.line_segment(
                    [*last, *first],
                    egui::Stroke::new(
                        2.0_f32,
                        ui_style::canvas_accent(self.presentation.preferences.background),
                    ),
                );
            }
        }
    }
}
