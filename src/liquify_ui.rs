use super::*;
use crate::{
    editing::Snapshot,
    liquify::{Direction, Falloff, LiquifySettings},
};

pub(super) struct LiquifyDraft {
    before: Snapshot,
    after: Snapshot,
    settings: LiquifySettings,
    has_preview: bool,
}

pub(super) struct LiquifyGesture {
    pub(super) start_screen: Pos2,
    pub(super) last_screen: Pos2,
    pub(super) start_world: Vec3,
    pub(super) plane_origin: Vec3,
    pub(super) plane_normal: Vec3,
    pub(super) base_strength: f32,
}

#[derive(Default)]
pub(super) struct LiquifyUi {
    draft: Option<LiquifyDraft>,
    brush_mode: bool,
    gesture: Option<LiquifyGesture>,
}

impl SketchApp {
    pub(super) fn liquify_active(&self) -> bool {
        self.liquify_ui.draft.is_some()
    }

    pub(super) fn liquify_brush_active(&self) -> bool {
        self.liquify_ui.draft.is_some() && self.liquify_ui.brush_mode
    }

    pub(super) fn activate_liquify_brush(&mut self) {
        if self.liquify_ui.draft.is_some() {
            self.cancel_liquify_gesture();
            self.liquify_ui.brush_mode = true;
        }
    }

    pub(super) fn begin_liquify(&mut self) {
        if self.selected.is_empty() {
            self.status = "Select ordinary strokes before starting Liquify.".into();
            return;
        }
        let before = crate::editing::snapshot(&self.doc, &self.selected);
        if before.is_empty() || before.len() != self.selected.len() {
            self.status = "Liquify requires a complete visible, unlocked selection.".into();
            return;
        }
        if before.iter().any(|(_, stroke)| stroke.fill.is_some()) {
            self.status =
                "Liquify currently supports ordinary strokes; deselect filled surfaces first."
                    .into();
            return;
        }
        let mut center = Vec3::ZERO;
        let mut count = 0_u64;
        for (_, stroke) in &before {
            for point in &stroke.points {
                center += point.position;
                count += 1;
            }
        }
        if count > 0 {
            center /= count as f32;
        }
        let settings = LiquifySettings {
            center,
            displacement: Vec3::ZERO,
            radius: 0.5,
            strength: 0.75,
            direction: Direction::Push,
            falloff: Falloff::Smooth,
        };
        self.liquify_ui.draft = Some(LiquifyDraft {
            after: before.clone(),
            before,
            settings,
            has_preview: false,
        });
        self.status =
            "Liquify preview. Choose Brush on canvas or adjust numeric controls, then Apply or Cancel."
                .into();
    }

    fn refresh_liquify(&mut self) {
        let Some(draft) = self.liquify_ui.draft.as_mut() else {
            return;
        };
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
        }
        match crate::liquify::preview(&draft.before, draft.settings) {
            Ok(after) if after != draft.before => {
                crate::editing::replace(&mut self.doc, &draft.before, &after, true);
                draft.after = after;
                draft.has_preview = true;
                self.status =
                    "Liquify preview only. Apply to keep the deformation, or Cancel to restore."
                        .into();
            }
            Ok(_) => {
                draft.after = draft.before.clone();
                draft.has_preview = false;
            }
            Err(error) => {
                draft.after = draft.before.clone();
                draft.has_preview = false;
                self.status = error;
            }
        }
        self.renderer.invalidate_ids(&ids);
    }

    pub(super) fn finish_liquify(&mut self, apply: bool) {
        let Some(draft) = self.liquify_ui.draft.take() else {
            return;
        };
        self.liquify_ui.brush_mode = false;
        self.liquify_ui.gesture = None;
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview && apply {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
            match crate::liquify::edit(&self.doc, &ids, draft.settings) {
                Ok(edit) => {
                    edit.apply(&mut self.doc, true);
                    self.undo.push(edit);
                    self.redo.clear();
                    self.dirty = true;
                    self.record_recovery_edit();
                    self.status =
                        "Liquify deformation applied. One undo restores the original points."
                            .into();
                }
                Err(error) => self.status = error,
            }
        } else if draft.has_preview {
            crate::editing::replace(&mut self.doc, &draft.before, &draft.after, false);
            self.status = if apply {
                "Liquify did not change the selected strokes."
            } else {
                "Liquify cancelled; original points restored."
            }
            .into();
        }
        self.renderer.invalidate_ids(&ids);
    }

    pub(super) fn liquify_controls(&mut self, ui: &mut egui::Ui) {
        if self.liquify_ui.draft.is_none() {
            if ui
                .add_enabled(
                    !self.selected.is_empty(),
                    egui::Button::new("Liquify selected strokes…"),
                )
                .clicked()
            {
                self.begin_liquify();
            }
            return;
        }
        let mut changed = false;
        let mut apply = false;
        let mut cancel = false;
        let mut enter_brush = false;
        let mut leave_brush = false;
        ui.group(|ui| {
            ui.strong("LIQUIFY PREVIEW");
            if self.liquify_ui.brush_mode {
                ui.label("Drag across the canvas to preview selected-ink deformation.");
                ui.small("Working plane: camera-facing plane through the selected ink centroid; depth stays fixed.");
                if ui.button("Use numeric controls").clicked() {
                    leave_brush = true;
                }
            } else if ui.button("Brush on canvas…").clicked() {
                enter_brush = true;
            }
            if let Some(draft) = self.liquify_ui.draft.as_mut() {
                if !self.liquify_ui.brush_mode {
                    ui.label("Brush center (world units)");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.center.x)
                                    .speed(0.01)
                                    .prefix("X "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.center.y)
                                    .speed(0.01)
                                    .prefix("Y "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.center.z)
                                    .speed(0.01)
                                    .prefix("Z "),
                            )
                            .changed();
                    });
                    ui.label("Brush displacement");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.displacement.x)
                                    .speed(0.01)
                                    .prefix("X "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.displacement.y)
                                    .speed(0.01)
                                    .prefix("Y "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut draft.settings.displacement.z)
                                    .speed(0.01)
                                    .prefix("Z "),
                            )
                            .changed();
                    });
                }
                changed |= ui
                    .add(
                        egui::Slider::new(&mut draft.settings.radius, 0.001..=10.0)
                            .text("Radius (wu)"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut draft.settings.strength, 0.0..=1.0).text("Strength"),
                    )
                    .changed();
                egui::ComboBox::from_label("Direction")
                    .selected_text(format!("{:?}", draft.settings.direction))
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.direction,
                                Direction::Push,
                                "Push",
                            )
                            .changed();
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.direction,
                                Direction::Pull,
                                "Pull",
                            )
                            .changed();
                    });
                egui::ComboBox::from_label("Falloff")
                    .selected_text(format!("{:?}", draft.settings.falloff))
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.falloff,
                                Falloff::Linear,
                                "Linear",
                            )
                            .changed();
                        changed |= ui
                            .selectable_value(
                                &mut draft.settings.falloff,
                                Falloff::Smooth,
                                "Smooth",
                            )
                            .changed();
                        changed |= ui
                            .selectable_value(&mut draft.settings.falloff, Falloff::Sharp, "Sharp")
                            .changed();
                    });
            }
            ui.horizontal(|ui| {
                apply = ui.button("Apply deformation").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if enter_brush {
            self.cancel_liquify_gesture();
            self.liquify_ui.brush_mode = true;
            self.status =
                "Liquify brush ready. Drag selected ink on its camera-facing working plane.".into();
        }
        if leave_brush {
            self.cancel_liquify_gesture();
            self.liquify_ui.brush_mode = false;
        }
        if changed {
            self.refresh_liquify();
        }
        if apply {
            self.finish_liquify(true);
        }
        if cancel {
            self.finish_liquify(false);
        }
    }

    pub(super) fn liquify_brush_input(
        &mut self,
        pos: Pos2,
        pressure: f32,
        rect: Rect,
        phase: crate::input::Phase,
    ) {
        if !self.liquify_brush_active() {
            return;
        }
        match phase {
            crate::input::Phase::Down => {
                if !rect.contains(pos) {
                    return;
                }
                let Some(draft) = self.liquify_ui.draft.as_ref() else {
                    return;
                };
                let mut plane_origin = Vec3::ZERO;
                let mut point_count = 0usize;
                for (_, stroke) in &draft.before {
                    for point in &stroke.points {
                        plane_origin += point.position;
                        point_count += 1;
                    }
                }
                if point_count == 0 {
                    return;
                }
                plane_origin /= point_count as f32;
                let plane_normal = self.camera.basis().2;
                let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let denominator = ray_direction.dot(plane_normal);
                if denominator.abs() < 1.0e-5 {
                    self.status =
                        "Cannot start Liquify: camera ray is parallel to the working plane.".into();
                    return;
                }
                let distance = (plane_origin - ray_origin).dot(plane_normal) / denominator;
                if !distance.is_finite() || distance <= 0.0 {
                    self.status = "Cannot start Liquify behind the camera.".into();
                    return;
                }
                let start_world = ray_origin + ray_direction * distance;
                let base_strength = self
                    .liquify_ui
                    .draft
                    .as_ref()
                    .map_or(0.75, |draft| draft.settings.strength);
                self.liquify_ui.gesture = Some(LiquifyGesture {
                    start_screen: pos,
                    last_screen: pos,
                    start_world,
                    plane_origin,
                    plane_normal,
                    base_strength,
                });
                self.update_liquify_gesture(pos, pressure, rect);
            }
            crate::input::Phase::Move => {
                if self.liquify_ui.gesture.is_some() {
                    if rect.contains(pos) {
                        self.update_liquify_gesture(pos, pressure, rect);
                    } else {
                        self.cancel_liquify_gesture();
                        self.status = "Liquify stroke cancelled outside the canvas.".into();
                    }
                }
            }
            crate::input::Phase::Up => {
                if self.liquify_ui.gesture.is_some() {
                    if rect.contains(pos) {
                        self.update_liquify_gesture(pos, pressure, rect);
                        self.liquify_ui.gesture = None;
                        self.status =
                            "Liquify preview ready. Apply to keep the drag, or Cancel to restore."
                                .into();
                    } else {
                        self.cancel_liquify_gesture();
                    }
                }
            }
            crate::input::Phase::Cancel => self.finish_liquify(false),
        }
    }

    fn update_liquify_gesture(&mut self, pos: Pos2, pressure: f32, rect: Rect) {
        let Some(gesture) = self.liquify_ui.gesture.as_mut() else {
            return;
        };
        let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pos);
        let denominator = ray_direction.dot(gesture.plane_normal);
        if denominator.abs() < 1.0e-5 {
            return;
        }
        let distance = (gesture.plane_origin - ray_origin).dot(gesture.plane_normal) / denominator;
        if !distance.is_finite() || distance <= 0.0 {
            return;
        }
        let current_world = ray_origin + ray_direction * distance;
        gesture.last_screen = pos;
        if let Some(draft) = self.liquify_ui.draft.as_mut() {
            draft.settings.center = gesture.start_world;
            draft.settings.displacement = current_world - gesture.start_world;
            draft.settings.strength = pressure_strength(gesture.base_strength, pressure);
        }
        self.refresh_liquify();
    }

    pub(super) fn cancel_liquify_gesture(&mut self) {
        self.liquify_ui.gesture = None;
        let restore = self.liquify_ui.draft.as_mut().map(|draft| {
            let should_restore = draft.has_preview;
            let after = draft.after.clone();
            let before = draft.before.clone();
            let ids: HashSet<_> = before.iter().map(|(_, stroke)| stroke.id).collect();
            draft.settings.displacement = Vec3::ZERO;
            draft.settings.center = snapshot_center(&draft.before);
            draft.has_preview = false;
            draft.after = before.clone();
            (should_restore, after, before, ids)
        });
        if let Some((true, after, before, ids)) = restore {
            crate::editing::replace(&mut self.doc, &before, &after, false);
            self.renderer.invalidate_ids(&ids);
        }
    }

    pub(super) fn cancel_liquify_for_navigation(&mut self) {
        if self.liquify_brush_active() && self.mouse_down {
            self.cancel_liquify_gesture();
        }
    }

    pub(super) fn paint_liquify_cursor(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        pointer: Option<Pos2>,
    ) {
        if !self.liquify_brush_active() {
            return;
        }
        let Some(draft) = self.liquify_ui.draft.as_ref() else {
            return;
        };
        let Some(pointer) = pointer.filter(|pos| rect.contains(*pos)) else {
            return;
        };
        let (cursor_center, plane_origin) = self
            .liquify_ui
            .gesture
            .as_ref()
            .map_or((pointer, draft.settings.center), |gesture| {
                (gesture.start_screen, gesture.plane_origin)
            });
        let normal = self.camera.basis().2;
        let (ray_origin, ray_direction) = crate::gizmo::ray(&self.camera, rect, pointer);
        let denominator = ray_direction.dot(normal);
        if denominator.abs() < 1.0e-5 {
            return;
        }
        let distance = (plane_origin - ray_origin).dot(normal) / denominator;
        if distance <= 0.0 {
            return;
        }
        let world = ray_origin + ray_direction * distance;
        let radius_pixels =
            draft.settings.radius / self.camera.world_per_pixel(world, rect.height());
        let painter = painter.with_clip_rect(rect);
        painter.circle_stroke(
            cursor_center,
            radius_pixels,
            egui::Stroke::new(1.5_f32, Color32::WHITE),
        );
        painter.circle_stroke(
            cursor_center,
            radius_pixels + 1.0,
            egui::Stroke::new(0.7_f32, Color32::BLACK),
        );
        let depth = (self.camera.eye() - plane_origin).dot(self.camera.basis().2);
        painter.text(
            cursor_center + Vec2::new(radius_pixels + 8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("r {:.2} wu · depth {depth:.2} wu", draft.settings.radius),
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
        if let Some(gesture) = &self.liquify_ui.gesture {
            painter.arrow(
                gesture.start_screen,
                gesture.last_screen - gesture.start_screen,
                egui::Stroke::new(2.0_f32, Color32::from_rgb(255, 205, 50)),
            );
        }
    }
}

fn snapshot_center(snapshot: &Snapshot) -> Vec3 {
    let (sum, count) = snapshot
        .iter()
        .flat_map(|(_, stroke)| stroke.points.iter())
        .fold((Vec3::ZERO, 0usize), |(sum, count), point| {
            (sum + point.position, count + 1)
        });
    if count == 0 {
        Vec3::ZERO
    } else {
        sum / count as f32
    }
}

fn pressure_strength(base: f32, pressure: f32) -> f32 {
    (base * pressure.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::pressure_strength;

    #[test]
    fn pen_pressure_scales_the_configured_liquify_strength() {
        assert_eq!(pressure_strength(0.8, 0.5), 0.4);
        assert_eq!(pressure_strength(0.8, 2.0), 0.8);
        assert_eq!(pressure_strength(0.8, -1.0), 0.0);
    }
}
