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

#[derive(Default)]
pub(super) struct LiquifyUi {
    draft: Option<LiquifyDraft>,
}

impl SketchApp {
    pub(super) fn liquify_active(&self) -> bool {
        self.liquify_ui.draft.is_some()
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
            "Liquify preview. Adjust the brush center and displacement, then Apply or Cancel."
                .into();
    }

    fn refresh_liquify(&mut self) {
        let Some(draft) = self.liquify_ui.draft.as_mut() else {
            return;
        };
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview {
            crate::editing::replace(&mut self.doc, &draft.after, &Vec::new(), false);
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
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview && apply {
            crate::editing::replace(&mut self.doc, &draft.after, &Vec::new(), false);
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
            crate::editing::replace(&mut self.doc, &draft.after, &Vec::new(), false);
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
        ui.group(|ui| {
            ui.strong("LIQUIFY PREVIEW");
            if let Some(draft) = self.liquify_ui.draft.as_mut() {
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
}
