use super::*;
use crate::mixed_resources::{MoveDirection, MoveSnapshot, ResourceSelection, TransformMode};

struct MixedMoveDraft {
    snapshot: MoveSnapshot,
    translation: Vec3,
    has_preview: bool,
}

#[derive(Default)]
pub(super) struct MixedResourcesUi {
    pub(super) selection: ResourceSelection,
    draft: Option<MixedMoveDraft>,
}

impl SketchApp {
    pub(super) fn mixed_transform_supported(mode: TransformMode) -> bool {
        mode == TransformMode::Translate
    }

    pub(super) fn mixed_resource_preview_active(&self) -> bool {
        self.mixed_resources_ui.draft.is_some()
    }

    pub(super) fn mixed_resource_has_preview(&self) -> bool {
        self.mixed_resources_ui
            .draft
            .as_ref()
            .is_some_and(|draft| draft.has_preview)
    }

    pub(super) fn mixed_resource_controls(&mut self, ui: &mut egui::Ui) {
        if self.mixed_resources_ui.draft.is_some() {
            ui.group(|ui| {
                ui.strong("MIXED RESOURCE TRANSLATION");
                ui.horizontal(|ui| {
                    ui.label("Transform");
                    ui.label("Translate");
                    for (mode, label) in [
                        (TransformMode::Rotate, "Rotate"),
                        (TransformMode::Scale, "Scale"),
                    ] {
                        ui.add_enabled(
                            Self::mixed_transform_supported(mode),
                            egui::Button::new(label),
                        );
                    }
                });
                let mut changed = false;
                let mut apply = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    ui.label("Offset (wu)");
                    if let Some(draft) = self.mixed_resources_ui.draft.as_mut() {
                        changed |= ui
                            .add(egui::DragValue::new(&mut draft.translation.x).speed(0.01).prefix("X "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut draft.translation.y).speed(0.01).prefix("Y "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut draft.translation.z).speed(0.01).prefix("Z "))
                            .changed();
                    }
                });
                ui.small("Preview is built from the original selection. Rotation and scaling are not supported for mixed resources.");
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(
                            self.mixed_resources_ui.draft.as_ref().is_some_and(|draft| draft.has_preview),
                            egui::Button::new("Apply movement"),
                        )
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if changed {
                    self.refresh_mixed_resource_move();
                }
                if apply {
                    self.finish_mixed_resource_move(true);
                }
                if cancel {
                    self.finish_mixed_resource_move(false);
                }
            });
            return;
        }
        egui::CollapsingHeader::new("Mixed resource selection")
            .default_open(true)
            .show(ui, |ui| {
            ui.small("Select strokes in the Select tool, then add visible, unlocked references and saved guides here.");
            ui.small("Moves use one shared world-space offset; no pivot is needed.");
            ui.label(format!("{} stroke(s) selected", self.selected.len()));

            for plane in &self.doc.image_planes {
                resource_checkbox(
                    ui,
                    &plane.name,
                    plane.visible && !plane.locked,
                    self.mixed_resources_ui
                        .selection
                        .image_plane_ids
                        .contains(&plane.id),
                    |selected| {
                        set_selection(
                            &mut self.mixed_resources_ui.selection.image_plane_ids,
                            plane.id,
                            selected,
                        );
                    },
                );
                if !plane.visible || plane.locked {
                    self.mixed_resources_ui
                        .selection
                        .image_plane_ids
                        .remove(&plane.id);
                }
            }
            for model in &self.doc.model_references {
                resource_checkbox(
                    ui,
                    &model.name,
                    model.visible && !model.locked,
                    self.mixed_resources_ui
                        .selection
                        .obj_reference_ids
                        .contains(&model.id)
                        || self.selected_model_reference == Some(model.id),
                    |selected| {
                        set_selection(
                            &mut self.mixed_resources_ui.selection.obj_reference_ids,
                            model.id,
                            selected,
                        );
                        if !selected && self.selected_model_reference == Some(model.id) {
                            self.selected_model_reference = None;
                        }
                    },
                );
                if !model.visible || model.locked {
                    self.mixed_resources_ui
                        .selection
                        .obj_reference_ids
                        .remove(&model.id);
                }
            }
            for guide in &self.doc.guide_resources {
                resource_checkbox(
                    ui,
                    &guide.name,
                    guide.visible && !guide.locked,
                    self.mixed_resources_ui
                        .selection
                        .guide_ids
                        .contains(&guide.id),
                    |selected| {
                        set_selection(
                            &mut self.mixed_resources_ui.selection.guide_ids,
                            guide.id,
                            selected,
                        );
                    },
                );
                if !guide.visible || guide.locked {
                    self.mixed_resources_ui
                        .selection
                        .guide_ids
                        .remove(&guide.id);
                }
            }

            if ui.button("Translate selected resources…").clicked() {
                self.begin_mixed_resource_move();
            }
            });
    }

    pub(super) fn begin_mixed_resource_move(&mut self) {
        self.finish();
        let mut selection = self.mixed_resources_ui.selection.clone();
        selection.stroke_ids = self.selected.clone();
        if let Some(id) = self.selected_model_reference {
            selection.obj_reference_ids.insert(id);
        }
        match MoveSnapshot::capture(&self.doc, selection, TransformMode::Translate, Vec3::ZERO) {
            Ok(snapshot) => {
                self.mixed_resources_ui.draft = Some(MixedMoveDraft {
                    snapshot,
                    translation: Vec3::ZERO,
                    has_preview: false,
                });
                self.status = "Mixed-resource translation preview; Apply or Cancel.".into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn set_mixed_resource_translation(&mut self, translation: Vec3) {
        if let Some(draft) = self.mixed_resources_ui.draft.as_mut() {
            draft.translation = translation;
            self.refresh_mixed_resource_move();
        }
    }

    fn refresh_mixed_resource_move(&mut self) {
        let Some(mut draft) = self.mixed_resources_ui.draft.take() else {
            return;
        };
        if draft.has_preview {
            if let Err(error) = draft.snapshot.apply(&mut self.doc, MoveDirection::Restore) {
                self.status = error.to_string();
                self.mixed_resources_ui.draft = Some(draft);
                return;
            }
            draft.has_preview = false;
        }
        let selection = draft.snapshot.selection.clone();
        match MoveSnapshot::capture(
            &self.doc,
            selection,
            TransformMode::Translate,
            draft.translation,
        ) {
            Ok(snapshot) => {
                if draft.translation != Vec3::ZERO {
                    match snapshot.apply(&mut self.doc, MoveDirection::Forward) {
                        Ok(()) => draft.has_preview = true,
                        Err(error) => {
                            self.status = error.to_string();
                            self.mixed_resources_ui.draft = Some(draft);
                            return;
                        }
                    }
                }
                draft.snapshot = snapshot;
                self.status =
                    "Mixed-resource translation preview. Apply to keep it, or Cancel to restore."
                        .into();
            }
            Err(error) => self.status = error.to_string(),
        }
        self.renderer.invalidate();
        self.refresh_obj_reference_meshes();
        self.mixed_resources_ui.draft = Some(draft);
    }

    pub(super) fn finish_mixed_resource_move(&mut self, apply: bool) {
        let Some(draft) = self.mixed_resources_ui.draft.take() else {
            return;
        };
        if draft.has_preview && apply {
            self.undo.push(Edit::MixedResources {
                before: draft.snapshot.before,
                after: draft.snapshot.after,
            });
            self.redo.clear();
            self.dirty = true;
            self.record_recovery_edit();
            self.status = "Mixed-resource movement applied as one undo step.".into();
        } else {
            if draft.has_preview {
                if let Err(error) = draft.snapshot.apply(&mut self.doc, MoveDirection::Restore) {
                    self.status = error.to_string();
                } else {
                    self.status =
                        "Mixed-resource movement cancelled; original transforms restored.".into();
                }
            }
        }
        self.renderer.invalidate();
        self.refresh_obj_reference_meshes();
    }
}

fn resource_checkbox(
    ui: &mut egui::Ui,
    name: &str,
    enabled: bool,
    mut selected: bool,
    mut set: impl FnMut(bool),
) {
    ui.add_enabled_ui(enabled, |ui| {
        if ui.checkbox(&mut selected, name).changed() {
            set(selected);
        }
    });
}

fn set_selection(selection: &mut HashSet<u64>, id: u64, selected: bool) {
    if selected {
        selection.insert(id);
    } else {
        selection.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_resource_ui_enables_translation_and_disables_rotation_and_scale() {
        assert!(SketchApp::mixed_transform_supported(
            TransformMode::Translate
        ));
        assert!(!SketchApp::mixed_transform_supported(TransformMode::Rotate));
        assert!(!SketchApp::mixed_transform_supported(TransformMode::Scale));
    }
}
