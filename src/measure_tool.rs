use super::guide_resource_ui::PlanarTarget;
use super::*;

#[derive(Clone, Copy)]
struct Draft {
    target: PlanarTarget,
    start: Vec3,
    end: Vec3,
    locked: bool,
}

#[derive(Default)]
pub(super) struct MeasureUi {
    pub enabled: bool,
    draft: Option<Draft>,
}

impl SketchApp {
    pub(super) fn start_measure_tool(&mut self) {
        self.finish();
        self.cancel_line_tool();
        self.cancel_curve_tool();
        self.cancel_shape_draft();
        self.cancel_fill_tool();
        self.cancel_placement();
        self.measure.enabled = true;
        self.measure.draft = None;
        self.eraser = false;
        self.select_mode = false;
        self.local_color_brush.enabled = false;
        self.clear_planar_snap();
        self.status = "Measure: click a start point, move to preview, then click an endpoint. Pin the annotation to keep it.".into();
    }

    pub(super) fn cancel_measure_tool(&mut self) {
        self.measure.enabled = false;
        self.measure.draft = None;
        self.clear_planar_snap();
    }

    fn measurement_point(&mut self, pos: Pos2, rect: Rect, target: PlanarTarget) -> Option<Vec3> {
        let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
        let world = target.hit(origin, direction)?;
        if self.planar_snap_index_stale(target, rect, &HashSet::new()) {
            self.prepare_planar_snap(target, rect, &HashSet::new());
        }
        Some(self.snap_planar_point(world, pos, target, rect, None))
    }

    pub(super) fn measure_input(&mut self, pos: Pos2, rect: Rect, begin: bool) {
        if !self.measure.enabled {
            return;
        }
        let Some(target) = self
            .measure
            .draft
            .map(|draft| draft.target)
            .or_else(|| self.planar_drawing_target())
        else {
            self.status = "Measurement needs an active planar guide or image plane.".into();
            return;
        };
        let Some(world) = self.measurement_point(pos, rect, target) else {
            return;
        };
        if !begin {
            if let Some(draft) = &mut self.measure.draft {
                if !draft.locked {
                    draft.end = world;
                }
            }
            return;
        }
        if let Some(draft) = &mut self.measure.draft {
            if !draft.locked {
                draft.end = world;
                draft.locked = true;
            }
        } else {
            self.measure.draft = Some(Draft {
                target,
                start: world,
                end: world,
                locked: false,
            });
        }
    }

    fn measurement_values(start: Vec3, end: Vec3, axis_u: Vec3, axis_v: Vec3) -> (f32, f32) {
        let delta = end - start;
        let angle = delta.dot(axis_v).atan2(delta.dot(axis_u)).to_degrees();
        (delta.length(), angle)
    }

    pub(super) fn measure_controls(&mut self, ui: &mut egui::Ui) {
        if !self.measure.enabled {
            return;
        }
        ui.strong("MEASURE");
        ui.label("Click two snapped points on the active plane. Distance is in world units; angle is measured from its U axis.");
        if let Some(draft) = self.measure.draft {
            let (u, v, _) = draft.target.plane.basis();
            let (distance, angle) = Self::measurement_values(draft.start, draft.end, u, v);
            ui.label(format!("{distance:.4} wu  ·  {angle:.2}°"));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(draft.locked, egui::Button::new("Pin annotation"))
                    .clicked()
                {
                    let id = self
                        .doc
                        .measurements
                        .iter()
                        .map(|item| item.id)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    let annotation = crate::document::MeasurementAnnotation {
                        id,
                        start: draft.start,
                        end: draft.end,
                        axis_u: u,
                        axis_v: v,
                    };
                    if annotation.is_valid() && self.doc.measurements.len() < 10_000 {
                        let edit = Edit::Measurement { annotation };
                        edit.apply(&mut self.doc, true);
                        self.undo.push(edit);
                        self.redo.clear();
                        self.dirty = true;
                        self.record_recovery_edit_labeled("Pin measurement annotation");
                        self.renderer.invalidate();
                        self.status = "Measurement annotation pinned.".into();
                        self.measure.draft = None;
                    } else {
                        self.status = "Cannot pin more measurement annotations.".into();
                    }
                }
                if ui.button("Clear measurement").clicked() {
                    self.measure.draft = None;
                    self.clear_planar_snap();
                }
            });
        }
    }

    pub(super) fn measure_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let painter = ui.painter();
        let draw_annotation = |start: Vec3, end: Vec3, u: Vec3, v: Vec3, color: Color32| {
            let (Some((a, _)), Some((b, _))) = (
                crate::gizmo::project(&self.camera, rect, start),
                crate::gizmo::project(&self.camera, rect, end),
            ) else {
                return;
            };
            painter.line_segment([a, b], egui::Stroke::new(1.8_f32, color));
            painter.circle_filled(a, 3.5, color);
            painter.circle_filled(b, 3.5, color);
            let (distance, angle) = Self::measurement_values(start, end, u, v);
            painter.text(
                a.lerp(b, 0.5) + egui::vec2(0.0, -12.0),
                egui::Align2::CENTER_BOTTOM,
                format!("{distance:.4} wu  ·  {angle:.2}°"),
                egui::FontId::proportional(13.0),
                color,
            );
        };
        for annotation in &self.doc.measurements {
            if annotation.is_valid() {
                draw_annotation(
                    annotation.start,
                    annotation.end,
                    annotation.axis_u,
                    annotation.axis_v,
                    Color32::from_rgb(108, 224, 233),
                );
            }
        }
        if let Some(draft) = self.measure.draft {
            let (u, v, _) = draft.target.plane.basis();
            draw_annotation(
                draft.start,
                draft.end,
                u,
                v,
                Color32::from_rgb(255, 219, 112),
            );
        }
    }
}
