use super::*;

#[derive(Clone)]
struct SwatchDraft {
    id: u64,
    name: String,
    color: [f32; 3],
}

#[derive(Default)]
pub(super) struct SwatchesUi {
    pub(super) open: bool,
    new_name: String,
    draft: Option<SwatchDraft>,
}

impl SketchApp {
    fn color_state_with_resolved_swatches(
        &self,
        mut state: crate::editing::DocumentColorState,
    ) -> crate::editing::DocumentColorState {
        state.swatches.resolve_strokes(&mut state.strokes);
        if let Some(strokes) = &mut state.animation.base_strokes {
            state.swatches.resolve_strokes(strokes);
        }
        for key in &mut state.animation.keyframes {
            state.swatches.resolve_strokes(&mut key.strokes);
        }
        for track in &mut state.animation.layer_tracks {
            for key in &mut track.keyframes {
                state.swatches.resolve_strokes(&mut key.strokes);
            }
        }
        state
    }

    fn commit_document_color_state(
        &mut self,
        before: crate::editing::DocumentColorState,
        after: crate::editing::DocumentColorState,
        label: &str,
    ) -> Result<(), String> {
        if before.swatches == after.swatches
            && before.strokes == after.strokes
            && before.animation == after.animation
        {
            return Ok(());
        }
        let mut candidate = self.doc.clone();
        candidate.color_swatches = after.swatches.clone();
        candidate.strokes = after.strokes.clone();
        candidate.animation = after.animation.clone();
        candidate.color_swatches.validate(&candidate)?;
        let edit = Edit::DocumentColorSwatches { before, after };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.renderer.invalidate();
        self.dirty = true;
        self.status = label.into();
        self.record_recovery_edit_labeled("Document color swatches");
        Ok(())
    }

    fn edit_document_swatches(
        &mut self,
        mutate: impl FnOnce(&mut crate::editing::DocumentColorState) -> Result<(), String>,
        label: &str,
    ) {
        let before = crate::editing::DocumentColorState::capture(&self.doc);
        let mut after = before.clone();
        if let Err(error) = mutate(&mut after) {
            self.status = error;
            return;
        }
        after = self.color_state_with_resolved_swatches(after);
        if let Err(error) = self.commit_document_color_state(before, after, label) {
            self.status = error;
        }
    }

    fn add_document_swatch(&mut self, name: &str, color: [f32; 3]) {
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            self.status = "Swatch names must contain 1–64 printable characters.".into();
            return;
        }
        if self
            .doc
            .color_swatches
            .swatches
            .iter()
            .any(|swatch| swatch.name.eq_ignore_ascii_case(&name))
        {
            self.status = "A swatch already uses that name.".into();
            return;
        }
        let id = self
            .doc
            .color_swatches
            .swatches
            .iter()
            .map(|swatch| swatch.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        if id == u64::MAX {
            self.status = "No swatch IDs remain in this drawing.".into();
            return;
        }
        self.edit_document_swatches(
            move |state| {
                state
                    .swatches
                    .swatches
                    .push(crate::document::DocumentColorSwatch { id, name, color });
                Ok(())
            },
            "Added a document color swatch.",
        );
    }

    fn document_swatch_protected_layers(&self, swatch_id: u64) -> Vec<String> {
        let bound_ids: std::collections::HashSet<_> = self
            .doc
            .color_swatches
            .bindings
            .iter()
            .filter(|binding| binding.swatch_id == swatch_id)
            .map(|binding| binding.stroke_id)
            .collect();
        let mut names = std::collections::BTreeSet::new();
        let mut visit = |strokes: &[Stroke]| {
            for stroke in strokes
                .iter()
                .filter(|stroke| bound_ids.contains(&stroke.id))
            {
                if let Some(layer) = self
                    .doc
                    .layers
                    .iter()
                    .find(|layer| layer.id == stroke.layer_id)
                {
                    if !layer.visible || layer.locked {
                        names.insert(layer.name.clone());
                    }
                }
            }
        };
        visit(&self.doc.strokes);
        if let Some(strokes) = &self.doc.animation.base_strokes {
            visit(strokes);
        }
        for key in &self.doc.animation.keyframes {
            visit(&key.strokes);
        }
        for track in &self.doc.animation.layer_tracks {
            for key in &track.keyframes {
                visit(&key.strokes);
            }
        }
        names.into_iter().collect()
    }

    fn assign_selected_to_swatch(&mut self, swatch_id: u64) {
        if self.selected.is_empty() {
            self.status = "Select editable strokes or fills to link to a swatch.".into();
            return;
        }
        let Some(swatch) = self.doc.color_swatches.swatch(swatch_id).cloned() else {
            self.status = "The selected swatch is no longer available.".into();
            return;
        };
        let selected = self.selected.clone();
        let strokes = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| selected.contains(&stroke.id))
            .collect::<Vec<_>>();
        if strokes.len() != selected.len()
            || strokes.iter().any(|stroke| !self.doc.editable(stroke))
        {
            self.status = "Hidden, locked, or missing artwork cannot be linked to a swatch.".into();
            return;
        }
        if strokes.iter().any(|stroke| {
            self.doc.has_linked_source(stroke.id)
                || self.doc.linked_instance_for_stroke(stroke.id).is_some()
                || stroke.gradient.is_some()
                || (stroke
                    .fill
                    .as_ref()
                    .is_some_and(|fill| fill.gradient.is_some() || fill.texture.is_some()))
        }) {
            self.status = "Swatches apply to solid stroke/fill colors. Make linked or gradient artwork unique first.".into();
            return;
        }
        let mut ids = selected;
        for id in ids.clone() {
            ids.extend(
                self.doc
                    .linked_affected_stroke_ids(&std::collections::HashSet::from([id])),
            );
        }
        self.edit_document_swatches(
            move |state| {
                state.swatches.bindings.retain(|binding| {
                    !(ids.contains(&binding.stroke_id)
                        && (binding.role == crate::document::SwatchRole::Stroke
                            || binding.role == crate::document::SwatchRole::Fill))
                });
                for id in ids {
                    let is_fill = state
                        .strokes
                        .iter()
                        .find(|stroke| stroke.id == id)
                        .or_else(|| {
                            state.animation.layer_tracks.iter().find_map(|track| {
                                track.keyframes.iter().find_map(|key| {
                                    key.strokes.iter().find(|stroke| stroke.id == id)
                                })
                            })
                        })
                        .is_some_and(|stroke| stroke.fill.is_some());
                    state
                        .swatches
                        .bindings
                        .push(crate::document::StrokeSwatchBinding {
                            stroke_id: id,
                            swatch_id,
                            role: if is_fill {
                                crate::document::SwatchRole::Fill
                            } else {
                                crate::document::SwatchRole::Stroke
                            },
                        });
                }
                Ok(())
            },
            &format!("Linked {} mark(s) to {}.", self.selected.len(), swatch.name),
        );
    }

    fn detach_selected_from_swatches(&mut self) {
        let ids = self.selected.clone();
        if ids.is_empty() {
            self.status = "Select swatch-linked marks to detach.".into();
            return;
        }
        if self
            .doc
            .strokes
            .iter()
            .any(|stroke| ids.contains(&stroke.id) && !self.doc.editable(stroke))
        {
            self.status = "Hidden or locked artwork cannot be detached from a swatch.".into();
            return;
        }
        self.edit_document_swatches(
            move |state| {
                state
                    .swatches
                    .bindings
                    .retain(|binding| !ids.contains(&binding.stroke_id));
                Ok(())
            },
            "Detached selected marks from their swatches; their displayed colors are retained.",
        );
    }

    fn delete_document_swatch(&mut self, swatch_id: u64) {
        let protected = self.document_swatch_protected_layers(swatch_id);
        if !protected.is_empty() {
            self.status = format!(
                "Unlock or show protected swatch uses before deleting it: {}.",
                protected.join(", ")
            );
            return;
        }
        self.edit_document_swatches(
            move |state| {
                state
                    .swatches
                    .swatches
                    .retain(|swatch| swatch.id != swatch_id);
                state
                    .swatches
                    .bindings
                    .retain(|binding| binding.swatch_id != swatch_id);
                Ok(())
            },
            "Deleted the swatch and detached its marks while retaining their literal colors.",
        );
    }

    pub(super) fn show_document_color_swatches(&mut self, ctx: &egui::Context) {
        if !self.swatches_ui.open {
            return;
        }
        let mut open = self.swatches_ui.open;
        let mut edit = None;
        let mut bind = None;
        let mut delete = None;
        let mut detach = false;
        let mut add_swatch = None;
        let mut cancel_edit = false;
        let swatches = self.doc.color_swatches.swatches.clone();
        egui::Window::new("Document color swatches")
            .id(egui::Id::new("document-color-swatches"))
            .open(&mut open)
            .resizable(true)
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.small("Marks without a swatch keep their literal colors. Linked colors update together and resolve consistently in animation and exports.");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.swatches_ui.new_name);
                    if ui.button("Add from brush color").clicked() {
                        let name = if self.swatches_ui.new_name.trim().is_empty() {
                            format!("Color {}", swatches.len() + 1)
                        } else {
                            self.swatches_ui.new_name.trim().to_string()
                        };
                        self.swatches_ui.new_name.clear();
                        add_swatch = Some((name, self.color));
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(310.0).show(ui, |ui| {
                    for swatch in &swatches {
                        ui.horizontal(|ui| {
                            let color = Color32::from_rgb(
                                (crate::presentation::linear_to_srgb(swatch.color[0]) * 255.0).round() as u8,
                                (crate::presentation::linear_to_srgb(swatch.color[1]) * 255.0).round() as u8,
                                (crate::presentation::linear_to_srgb(swatch.color[2]) * 255.0).round() as u8,
                            );
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 3.0, color);
                            let uses = self.doc.color_swatches.bindings.iter().filter(|binding| binding.swatch_id == swatch.id).count();
                            ui.label(format!("{} ({uses})", swatch.name));
                            if ui.small_button("Edit").clicked() {
                                edit = Some(SwatchDraft { id: swatch.id, name: swatch.name.clone(), color: swatch.color });
                            }
                            if ui.small_button("Use selected").clicked() { bind = Some(swatch.id); }
                            if ui.small_button("Delete").clicked() { delete = Some(swatch.id); }
                        });
                    }
                });
                if let Some(draft) = &mut self.swatches_ui.draft {
                    ui.separator();
                    ui.label("Edit linked swatch");
                    ui.text_edit_singleline(&mut draft.name);
                    let mut color_srgb = draft.color.map(crate::presentation::linear_to_srgb);
                    if ui.color_edit_button_rgb(&mut color_srgb).changed() {
                        draft.color = color_srgb.map(crate::presentation::srgb_to_linear);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Apply swatch edit").clicked() { edit = Some(draft.clone()); }
                        if ui.button("Cancel").clicked() { cancel_edit = true; }
                    });
                }
                if !self.selected.is_empty() && ui.button("Detach selected").clicked() { detach = true; }
            });
        self.swatches_ui.open = open;
        if cancel_edit {
            self.swatches_ui.draft = None;
        }
        if let Some((name, color)) = add_swatch {
            self.add_document_swatch(&name, color);
        }
        if let Some(draft) = edit {
            if self
                .swatches_ui
                .draft
                .as_ref()
                .is_some_and(|current| current.id == draft.id)
            {
                self.update_document_swatch(draft);
            } else {
                self.swatches_ui.draft = Some(draft);
            }
        }
        if let Some(id) = bind {
            self.assign_selected_to_swatch(id);
        }
        if detach {
            self.detach_selected_from_swatches();
        }
        if let Some(id) = delete {
            self.delete_document_swatch(id);
        }
    }

    fn update_document_swatch(&mut self, draft: SwatchDraft) {
        let name = draft.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            self.status = "Swatch names must contain 1–64 printable characters.".into();
            return;
        }
        if self
            .doc
            .color_swatches
            .swatches
            .iter()
            .any(|swatch| swatch.id != draft.id && swatch.name.eq_ignore_ascii_case(&name))
        {
            self.status = "A swatch already uses that name.".into();
            return;
        }
        let protected = self.document_swatch_protected_layers(draft.id);
        if !protected.is_empty() {
            self.status = format!(
                "Unlock or show protected swatch uses before editing it: {}.",
                protected.join(", ")
            );
            return;
        }
        let id = draft.id;
        self.edit_document_swatches(
            move |state| {
                let swatch = state
                    .swatches
                    .swatches
                    .iter_mut()
                    .find(|swatch| swatch.id == id)
                    .ok_or("The swatch was removed")?;
                swatch.name = name;
                swatch.color = draft.color;
                Ok(())
            },
            "Updated the named swatch and every linked mark.",
        );
        self.swatches_ui.draft = None;
    }
}
