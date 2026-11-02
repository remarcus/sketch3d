use super::*;
use crate::mixed_resources::{
    MoveDirection, MoveSnapshot, ResourceSelection, TransformMode, TransformSpec,
};

struct MixedMoveDraft {
    snapshot: MoveSnapshot,
    mode: TransformMode,
    translation: Vec3,
    rotation_degrees: Vec3,
    scale: f32,
    selection_center: Vec3,
    has_preview: bool,
}

#[derive(Default)]
pub(super) struct MixedResourcesUi {
    pub(super) selection: ResourceSelection,
    draft: Option<MixedMoveDraft>,
}

impl SketchApp {
    pub(super) fn mixed_transform_supported(mode: TransformMode) -> bool {
        matches!(
            mode,
            TransformMode::Translate | TransformMode::Rotate | TransformMode::Scale
        )
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
                ui.strong("MIXED RESOURCE TRANSFORM");
                let current_mode = self
                    .mixed_resources_ui
                    .draft
                    .as_ref()
                    .expect("mixed transform draft remains active")
                    .mode;
                let mut next_mode = current_mode;
                ui.horizontal(|ui| {
                    ui.label("Transform");
                    for (mode, label) in [
                        (TransformMode::Translate, "Translate"),
                        (TransformMode::Rotate, "Rotate"),
                        (TransformMode::Scale, "Uniform scale"),
                    ] {
                        if ui.selectable_label(current_mode == mode, label).clicked() {
                            next_mode = mode;
                        }
                    }
                });
                let mut changed = false;
                if next_mode != current_mode {
                    if let Some(draft) = self.mixed_resources_ui.draft.as_mut() {
                        draft.mode = next_mode;
                        draft.translation = Vec3::ZERO;
                        draft.rotation_degrees = Vec3::ZERO;
                        draft.scale = 1.0;
                    }
                    changed = true;
                }
                let mode = self
                    .mixed_resources_ui
                    .draft
                    .as_ref()
                    .expect("mixed transform draft remains active")
                    .mode;
                match mode {
                    TransformMode::Translate => {
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
                    }
                    TransformMode::Rotate | TransformMode::Scale => {
                        changed |= self.mixed_pivot_controls(ui);
                        if mode == TransformMode::Rotate {
                            ui.label("Rotation in degrees (X, then Y, then Z)");
                            for (index, axis) in ["X", "Y", "Z"].iter().enumerate() {
                                if let Some(draft) = self.mixed_resources_ui.draft.as_mut() {
                                    changed |= ui_style::numeric_row(
                                        ui,
                                        axis,
                                        ("mixed-rotation", index),
                                        &mut draft.rotation_degrees[index],
                                        Some(-36_000.0..=36_000.0),
                                        "°",
                                    )
                                    .changed();
                                }
                            }
                        } else if let Some(draft) = self.mixed_resources_ui.draft.as_mut() {
                            changed |= ui_style::numeric_row(
                                ui,
                                "Uniform scale",
                                "mixed-uniform-scale",
                                &mut draft.scale,
                                Some(0.001..=100.0),
                                "×",
                            )
                            .changed();
                            ui.small("Stroke frames and widths scale together. Limits and resource validity are checked before preview.");
                        }
                    }
                }
                ui.small("Preview is rebuilt from original records. Apply commits one undo step; Cancel or Escape restores the originals.");
                let mut apply = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(
                            self.mixed_resources_ui.draft.as_ref().is_some_and(|draft| draft.has_preview),
                            egui::Button::new("Apply transform"),
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
            ui.small("Rotation and uniform scale use a shared pivot. Non-uniform mixed scaling is disabled.");
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

            if ui.button("Transform selected resources…").clicked() {
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
        let selection_center = crate::mixed_resources::selection_bounds(
            &self.doc,
            &selection,
            &self.obj_reference_models,
        )
        .map_or(Vec3::ZERO, |(min, max)| (min + max) * 0.5);
        match MoveSnapshot::capture(
            &self.doc,
            selection.clone(),
            TransformMode::Translate,
            Vec3::ZERO,
        ) {
            Ok(snapshot) => {
                self.mixed_resources_ui.draft = Some(MixedMoveDraft {
                    snapshot,
                    mode: TransformMode::Translate,
                    translation: Vec3::ZERO,
                    rotation_degrees: Vec3::ZERO,
                    scale: 1.0,
                    selection_center,
                    has_preview: false,
                });
                self.status = "Mixed-resource transform preview; Apply or Cancel.".into();
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
        let pivot = match self.transform_pivot {
            super::selection::PivotMode::SelectionCenter => draft.selection_center,
            super::selection::PivotMode::WorldOrigin => Vec3::ZERO,
            super::selection::PivotMode::Custom => self.custom_pivot,
        };
        let transform = match draft.mode {
            TransformMode::Translate => TransformSpec::translate(draft.translation),
            TransformMode::Rotate => TransformSpec::rotate(
                pivot,
                glam::Quat::from_euler(
                    glam::EulerRot::ZYX,
                    draft.rotation_degrees.z.to_radians(),
                    draft.rotation_degrees.y.to_radians(),
                    draft.rotation_degrees.x.to_radians(),
                ),
            ),
            TransformMode::Scale => TransformSpec::uniform_scale(pivot, draft.scale, true),
        };
        match MoveSnapshot::capture_transform(&self.doc, selection, transform) {
            Ok(snapshot) => {
                if snapshot.after != snapshot.before {
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
                self.status = match draft.mode {
                    TransformMode::Translate => {
                        "Mixed-resource translation preview. Apply to keep it, or Cancel to restore."
                    }
                    TransformMode::Rotate => {
                        "Mixed-resource rotation preview. Apply to keep it, or Cancel to restore."
                    }
                    TransformMode::Scale => {
                        "Mixed-resource uniform-scale preview. Apply to keep it, or Cancel to restore."
                    }
                }
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
            self.status = "Mixed-resource transform applied as one undo step.".into();
        } else {
            if draft.has_preview {
                if let Err(error) = draft.snapshot.apply(&mut self.doc, MoveDirection::Restore) {
                    self.status = error.to_string();
                } else {
                    self.status =
                        "Mixed-resource transform cancelled; original records restored.".into();
                }
            }
        }
        self.renderer.invalidate();
        self.refresh_obj_reference_meshes();
    }
}

impl SketchApp {
    fn mixed_pivot_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Pivot");
            egui::ComboBox::from_id_salt("mixed-transform-pivot")
                .selected_text(match self.transform_pivot {
                    super::selection::PivotMode::SelectionCenter => "Selection center",
                    super::selection::PivotMode::WorldOrigin => "World origin",
                    super::selection::PivotMode::Custom => "Custom",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.transform_pivot,
                            super::selection::PivotMode::SelectionCenter,
                            "Selection center",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.transform_pivot,
                            super::selection::PivotMode::WorldOrigin,
                            "World origin",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.transform_pivot,
                            super::selection::PivotMode::Custom,
                            "Custom",
                        )
                        .changed();
                });
        });
        if self.transform_pivot == super::selection::PivotMode::Custom {
            for (index, axis) in ["Pivot X", "Pivot Y", "Pivot Z"].iter().enumerate() {
                changed |= ui_style::numeric_row(
                    ui,
                    axis,
                    ("mixed-custom-pivot", index),
                    &mut self.custom_pivot[index],
                    Some(-10_000.0..=10_000.0),
                    "wu",
                )
                .changed();
            }
        } else if self.transform_pivot == super::selection::PivotMode::SelectionCenter {
            if let Some(draft) = &self.mixed_resources_ui.draft {
                ui.small(format!(
                    "Bounds center: {:.3}, {:.3}, {:.3} wu",
                    draft.selection_center.x, draft.selection_center.y, draft.selection_center.z
                ));
            }
        }
        changed
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
    fn mixed_resource_ui_enables_translation_rotation_and_uniform_scale() {
        assert!(SketchApp::mixed_transform_supported(
            TransformMode::Translate
        ));
        assert!(SketchApp::mixed_transform_supported(TransformMode::Rotate));
        assert!(SketchApp::mixed_transform_supported(TransformMode::Scale));
    }
}
