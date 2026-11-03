use super::guide_resource_ui::PlanarTarget;
use super::ui_style;
use super::*;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Constraint {
    #[default]
    Free,
    Horizontal,
    Vertical,
    Angle,
}
pub(super) struct LineDraft {
    target: PlanarTarget,
    pub(super) mark: Stroke,
    constraint: Constraint,
    step: f32,
}
pub(super) struct LineUi {
    pub enabled: bool,
    pub constraint: Constraint,
    pub step: f32,
    pub draft: Option<LineDraft>,
    pub verification_frame: Option<u32>,
}
impl Default for LineUi {
    fn default() -> Self {
        Self {
            enabled: false,
            constraint: Constraint::Free,
            step: 15.0,
            draft: None,
            verification_frame: None,
        }
    }
}
fn constrain(
    start: Vec3,
    end: Vec3,
    plane: DrawingPlane,
    constraint: Constraint,
    step: f32,
) -> Vec3 {
    let (u, v, _) = plane.basis();
    let delta = end - start;
    let mut x = delta.dot(u);
    let mut y = delta.dot(v);
    match constraint {
        Constraint::Free => {}
        Constraint::Horizontal => y = 0.0,
        Constraint::Vertical => x = 0.0,
        Constraint::Angle => {
            let length = x.hypot(y);
            let step = step.clamp(1.0, 90.0).to_radians();
            let angle = (y.atan2(x) / step).round() * step;
            x = length * angle.cos();
            y = length * angle.sin();
        }
    }
    start + u * x + v * y
}
impl SketchApp {
    pub(super) fn cancel_line(&mut self) {
        if self.line.draft.take().is_some() {
            self.resume_ink = false;
        }
        self.clear_planar_snap();
    }
    pub(super) fn cancel_line_tool(&mut self) {
        self.cancel_line();
        self.line.enabled = false;
    }
    pub(super) fn start_line_tool(&mut self) {
        if self.planar_drawing_target().is_none() {
            self.status =
                "Straight lines require a plane guide. Activate a planar guide first.".into();
            return;
        }
        self.finish();
        self.cancel_fill_tool();
        self.line.enabled = true;
        self.eraser = false;
        self.select_mode = false;
        self.resume_ink = false;
    }
    pub(super) fn line_controls(&mut self, ui: &mut egui::Ui) {
        if !self.line.enabled {
            return;
        }
        ui.strong("STRAIGHT LINE");
        ui.label("Press, drag to preview, then release to draw. Escape cancels. Uses plane or image-plane targets.");
        ui_style::field_row(ui, "Constraint", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (constraint, label) in [
                    (Constraint::Free, "Free angle"),
                    (Constraint::Horizontal, "Horizontal"),
                    (Constraint::Vertical, "Vertical"),
                    (Constraint::Angle, "Angle snap"),
                ] {
                    ui.radio_value(&mut self.line.constraint, constraint, label);
                }
            });
        });
        if self.line.constraint == Constraint::Angle {
            ui_style::slider_row(
                ui,
                "Snap step",
                "line-snap-step",
                &mut self.line.step,
                1.0..=90.0,
                1.0,
                "°",
            );
        }
        ui.small("Constraints follow the captured plane axes. Start/end contact pressure and tilt are retained; mouse uses fixed pressure. Taper and textures use the normal brush settings. Stabilization is bypassed.");
        if self.line.draft.is_some() && ui.button("Cancel line").clicked() {
            self.cancel_line();
        }
    }
    pub(super) fn line_input(
        &mut self,
        pos: Pos2,
        pressure: f32,
        tilt: Option<Vec3>,
        rect: Rect,
        begin: bool,
    ) {
        if begin {
            self.finish();
            self.cancel_line();
            let Some(target) = self.planar_drawing_target() else {
                self.status =
                    "Straight lines require a plane guide. Activate a planar guide first.".into();
                return;
            };
            if !self.doc.active_editable() {
                self.status = "Choose a visible unlocked layer for a line.".into();
                return;
            }
            let (o, d) = crate::gizmo::ray(&self.camera, rect, pos);
            let Some(world) = target.hit(o, d) else {
                return;
            };
            self.prepare_planar_snap(target, rect, &HashSet::new());
            let world = self.snap_planar_point(world, pos, target, rect, None);
            let plane = target.plane;
            let point = Point {
                position: world,
                pressure: pressure.clamp(0.0, 1.0),
                tilt,
                time_ms: 0,
            };
            if !point.is_valid() {
                return;
            }
            self.line.draft = Some(LineDraft {
                target,
                constraint: self.line.constraint,
                step: self.line.step,
                mark: Stroke {
                    model_surface: None,
                    curve: None,
                    material: Default::default(),
                    point_thickness: None,
                    modifiers: None,
                    fill: None,
                    group_id: None,
                    layer_id: self.doc.active_layer,
                    id: 0,
                    color: self.color,
                    gradient: None,
                    radius: self.radius,
                    brush: Brush {
                        normal: plane.basis().2,
                        tangent: plane.basis().0,
                        dabs: crate::dynamics::DabSettings {
                            enabled: false,
                            ..self.brush.dabs
                        },
                        ..self.brush
                    },
                    points: vec![point, point],
                },
            });
        }
        if let Some(draft) = &mut self.line.draft {
            draft.mark.points[1].pressure = pressure.clamp(0.0, 1.0);
            draft.mark.points[1].tilt = tilt;
        }
        self.line_position(pos, rect);
    }
    fn line_position(&mut self, pos: Pos2, rect: Rect) -> bool {
        let Some(draft) = self.line.draft.as_ref() else {
            return false;
        };
        if !rect.contains(pos) {
            return false;
        }
        let target = draft.target;
        let plane = target.plane;
        let constraint = draft.constraint;
        let step = draft.step;
        let start = draft.mark.points[0].position;
        let (o, d) = crate::gizmo::ray(&self.camera, rect, pos);
        let Some(world) = target.hit(o, d) else {
            return false;
        };
        let constrained_end = constrain(start, world, plane, constraint, step);
        let constrained = constraint != Constraint::Free;
        let snap_cursor = if constrained {
            crate::gizmo::project(&self.camera, rect, constrained_end)
                .map_or(pos, |(screen, _)| screen)
        } else {
            pos
        };
        let excluded = HashSet::new();
        if self.planar_snap_index_stale(target, rect, &excluded) {
            self.prepare_planar_snap(target, rect, &excluded);
        }
        let end = self.snap_planar_point(
            constrained_end,
            snap_cursor,
            target,
            rect,
            constrained.then_some((start, constrained_end)),
        );
        if !end.is_finite() || end.abs().max_element() > 10000.0 || !target.contains(end) {
            return false;
        }
        let draft = self.line.draft.as_mut().expect("line draft remains active");
        draft.mark.points[1].position = end;
        true
    }
    pub(super) fn release_line(&mut self, pos: Pos2, rect: Rect) {
        if !self.line_position(pos, rect) {
            self.cancel_line();
            return;
        }
        let Some(draft) = self.line.draft.take() else {
            self.clear_planar_snap();
            return;
        };
        self.clear_planar_snap();
        let mut mark = draft.mark;
        if mark.points[0].position.distance(mark.points[1].position) < 0.00001 {
            self.status = "Zero-length line cancelled.".into();
            return;
        }
        if !self.doc.editable(&mark)
            || !mark.brush.is_valid()
            || !(0.001..=1.0).contains(&mark.radius)
            || mark.points.iter().any(|p| !p.is_valid())
            || self.next_id == u64::MAX
            || self.doc.strokes.len() >= 100_000
            || self
                .doc
                .strokes
                .iter()
                .map(|s| s.points.len())
                .sum::<usize>()
                + 2
                > 2_000_000
        {
            self.status = "Line could not be committed: protected layer or document limits.".into();
            return;
        }
        mark.id = self.next_id;
        self.next_id += 1;
        self.commit_drawn_stroke(mark, false, 0);
        self.clear_planar_snap();
    }
    pub(super) fn line_shortcuts(&mut self, ctx: &egui::Context) {
        let (l, escape, mods) = ctx.input(|i| {
            (
                i.key_pressed(Key::L),
                i.key_pressed(Key::Escape),
                i.modifiers,
            )
        });
        if l && !mods.command && !mods.alt {
            self.start_line_tool();
        }
        if escape && self.line.draft.is_some() {
            self.cancel_line();
            self.mouse_down = false;
        }
    }
    pub(super) fn line_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(draft) = &self.line.draft else {
            return;
        };
        if let (Some((a, _)), Some((b, _))) = (
            crate::gizmo::project(&self.camera, rect, draft.mark.points[0].position),
            crate::gizmo::project(&self.camera, rect, draft.mark.points[1].position),
        ) {
            let painter = ui.painter().with_clip_rect(rect);
            let color = ui_style::canvas_overlay_accent(
                self.presentation.preferences.background,
                self.presentation.preferences.background_image.is_some(),
            );
            painter.line_segment([a, b], egui::Stroke::new(2.0_f32, color));
            for p in [a, b] {
                painter.circle_stroke(p, 5.0, egui::Stroke::new(1.5_f32, color));
            }
            self.mirror_preview_overlay(ui, rect, &draft.mark);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotated_plane_constraints_preserve_plane_and_snap_axes() {
        let plane = DrawingPlane {
            anchor: Vec3::new(2.0, 1.0, -3.0),
            rotation_degrees: Vec3::new(30.0, -45.0, 17.0),
            depth: 0.2,
        };
        let (u, v, n) = plane.basis();
        let start = plane.origin();
        let end = start + u * 2.0 + v * 0.7;
        let horizontal = constrain(start, end, plane, Constraint::Horizontal, 15.0);
        assert!((horizontal - start).dot(v).abs() < 1e-5);
        let vertical = constrain(start, end, plane, Constraint::Vertical, 15.0);
        assert!((vertical - start).dot(u).abs() < 1e-5);
        let snapped = constrain(start, end, plane, Constraint::Angle, 45.0);
        assert!((snapped - start).dot(v).abs() < 1e-5);
        for p in [horizontal, vertical, snapped] {
            assert!((p - start).dot(n).abs() < 1e-5);
        }
        assert!(((snapped - start).length() - (end - start).length()).abs() < 1e-5);
    }
}
