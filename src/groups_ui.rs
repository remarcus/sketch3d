use super::*;
use crate::{editing, gizmo};
use glam::Quat;
impl SketchApp {
    fn apply_group_action(&mut self, ungroup: bool) {
        self.finish();
        let result = if ungroup {
            editing::ungroup_selected(&self.doc, &self.selected)
        } else {
            editing::group_selected(&self.doc, &self.selected)
        };
        match result {
            Ok(edit) => {
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.renderer.invalidate();
                self.sync_selection();
                self.status = if ungroup {
                    "Selection ungrouped"
                } else {
                    "Selection grouped"
                }
                .into();
            }
            Err(error) => self.status = error,
        }
    }
    pub(super) fn group_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.selected.len() >= 2, egui::Button::new("Group"))
                .on_hover_text("Group selected strokes on one editable layer (Ctrl/Cmd+G)")
                .clicked()
            {
                self.apply_group_action(false);
            }
            let grouped = self
                .doc
                .strokes
                .iter()
                .any(|s| self.selected.contains(&s.id) && s.group_id.is_some());
            if ui
                .add_enabled(grouped, egui::Button::new("Ungroup"))
                .on_hover_text("Ungroup complete selected groups (Ctrl/Cmd+Shift+G)")
                .clicked()
            {
                self.apply_group_action(true);
            }
        });
        let mut individual = self.individual_selection;
        if ui
            .checkbox(&mut individual, "Select individual strokes")
            .changed()
        {
            self.set_individual_selection(individual);
        }
        ui.small("Click selects a whole group; Shift-click toggles it. Individual mode edits members. Groups stay on one layer.");
        ui.separator();
        self.linked_group_controls(ui);
    }

    fn linked_group_controls(&mut self, ui: &mut egui::Ui) {
        let selected_instance = self.selected_linked_instance();
        ui.label("Linked stroke groups");
        if ui
            .add_enabled(
                self.selected.len() >= 2 && selected_instance.is_none(),
                egui::Button::new("Create linked instance"),
            )
            .on_hover_text("Create a linked definition from a complete regular stroke group")
            .clicked()
        {
            self.create_linked_group();
        }

        let definitions: Vec<_> = self
            .doc
            .linked_stroke_groups
            .definitions
            .iter()
            .map(|definition| {
                let instances = self
                    .doc
                    .linked_stroke_groups
                    .instances
                    .iter()
                    .filter(|instance| instance.definition_id == definition.id)
                    .count();
                (definition.id, definition.source_stroke_ids.len(), instances)
            })
            .collect();
        for (definition_id, strokes, instances) in definitions {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Group {definition_id}: {strokes} strokes, {instances} copies"
                ));
                if ui.button("Add copy").clicked() {
                    self.add_linked_group_copy(definition_id, instances);
                }
            });
        }

        if let Some(instance_id) = selected_instance {
            ui.horizontal(|ui| {
                if ui.button("Move −U").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        -self.active_guide_u() * 0.25,
                        Quat::IDENTITY,
                        1.0,
                    );
                }
                if ui.button("Move +U").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        self.active_guide_u() * 0.25,
                        Quat::IDENTITY,
                        1.0,
                    );
                }
                let normal = self
                    .planar_drawing_target()
                    .map_or(Vec3::Z, |target| target.plane.basis().2);
                if ui.button("Rotate −15°").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        Vec3::ZERO,
                        Quat::from_axis_angle(normal, -15.0_f32.to_radians()),
                        1.0,
                    );
                }
                if ui.button("Rotate +15°").clicked() {
                    self.transform_linked_instance(
                        instance_id,
                        Vec3::ZERO,
                        Quat::from_axis_angle(normal, 15.0_f32.to_radians()),
                        1.0,
                    );
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Scale 90%").clicked() {
                    self.transform_linked_instance(instance_id, Vec3::ZERO, Quat::IDENTITY, 0.9);
                }
                if ui.button("Scale 110%").clicked() {
                    self.transform_linked_instance(instance_id, Vec3::ZERO, Quat::IDENTITY, 1.1);
                }
                if ui.button("Make unique").clicked() {
                    self.remove_linked_group_instance(instance_id, true);
                }
                if ui.button("Delete instance").clicked() {
                    self.remove_linked_group_instance(instance_id, false);
                }
            });
            ui.small("Linked copies update when their source group changes. Make a copy unique before editing or erasing it directly.");
        }
    }

    fn selected_linked_instance(&self) -> Option<u64> {
        if self.selected.is_empty() {
            return None;
        }
        let instances: HashSet<_> = self
            .selected
            .iter()
            .filter_map(|id| {
                self.doc
                    .linked_instance_for_stroke(*id)
                    .map(|instance| instance.id)
            })
            .collect();
        (instances.len() == 1
            && self
                .selected
                .iter()
                .all(|id| self.doc.linked_instance_for_stroke(*id).is_some()))
        .then(|| *instances.iter().next().unwrap())
    }

    fn apply_linked_group_edit(
        &mut self,
        edit: editing::Edit,
        select_ids: Option<HashSet<u64>>,
        consumed_ids: usize,
        status: &str,
    ) {
        if let Some(end) = self.next_id.checked_add(consumed_ids as u64) {
            self.next_id = end;
        }
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        if let Some(ids) = select_ids {
            self.selected = ids;
        }
        self.sync_selection();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status = status.into();
    }

    fn create_linked_group(&mut self) {
        let count = self.selected.len();
        match editing::create_linked_group(
            &self.doc,
            &self.selected,
            self.next_id,
            self.doc.active_layer,
            self.active_guide_u() * 0.5,
        ) {
            Ok(edit) => {
                let (ids, count) = match &edit {
                    editing::Edit::LinkedStrokeGroups { after, .. } => after
                        .instances
                        .last()
                        .map(|instance| {
                            (
                                Some(instance.stroke_ids.iter().copied().collect()),
                                instance.stroke_ids.len(),
                            )
                        })
                        .unwrap_or((None, count)),
                    _ => (None, count),
                };
                self.apply_linked_group_edit(edit, ids, count, "Linked stroke group created.");
            }
            Err(error) => self.status = error,
        }
    }

    fn add_linked_group_copy(&mut self, definition_id: u64, existing_copies: usize) {
        let count = self
            .doc
            .linked_stroke_groups
            .definitions
            .iter()
            .find(|definition| definition.id == definition_id)
            .map_or(0, |definition| definition.source_stroke_ids.len());
        let offset = self.active_guide_u() * (0.5 * (existing_copies as f32 + 1.0));
        match editing::add_linked_instance(
            &self.doc,
            definition_id,
            self.next_id,
            self.doc.active_layer,
            offset,
        ) {
            Ok(edit) => {
                let ids = match &edit {
                    editing::Edit::LinkedStrokeGroups { after, .. } => after
                        .instances
                        .last()
                        .map(|instance| instance.stroke_ids.iter().copied().collect()),
                    _ => None,
                };
                self.apply_linked_group_edit(edit, ids, count, "Linked copy created.");
            }
            Err(error) => self.status = error,
        }
    }

    fn transform_linked_instance(
        &mut self,
        instance_id: u64,
        translation: Vec3,
        rotation: Quat,
        scale: f32,
    ) {
        match editing::transform_linked_instance(
            &self.doc,
            instance_id,
            translation,
            rotation,
            scale,
        ) {
            Ok(edit) => {
                self.apply_linked_group_edit(edit, None, 0, "Linked instance transform applied.")
            }
            Err(error) => self.status = error,
        }
    }

    fn remove_linked_group_instance(&mut self, instance_id: u64, make_unique: bool) {
        match editing::remove_linked_instance(&self.doc, instance_id, make_unique) {
            Ok(edit) => {
                if make_unique {
                    let ids: HashSet<_> = self.selected.clone();
                    self.apply_linked_group_edit(
                        edit,
                        Some(ids),
                        0,
                        "Instance made unique. It can now be edited independently.",
                    );
                } else {
                    self.apply_linked_group_edit(
                        edit,
                        Some(HashSet::new()),
                        0,
                        "Linked instance deleted. Undo restores the link.",
                    );
                }
            }
            Err(error) => self.status = error,
        }
    }
    fn set_individual_selection(&mut self, individual: bool) {
        self.finish();
        self.individual_selection = individual;
        if !individual {
            let groups: HashSet<_> = self
                .doc
                .strokes
                .iter()
                .filter(|s| self.selected.contains(&s.id))
                .filter_map(|s| s.group_id)
                .collect();
            self.selected.extend(
                self.doc
                    .strokes
                    .iter()
                    .filter(|s| {
                        s.group_id.is_some_and(|id| groups.contains(&id)) && self.doc.editable(s)
                    })
                    .map(|s| s.id),
            );
        }
    }
    pub(super) fn group_shortcuts(&mut self, ctx: &egui::Context) {
        let action = ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(Key::G) {
                Some(i.modifiers.shift)
            } else {
                None
            }
        });
        if self.select_mode {
            if let Some(ungroup) = action {
                self.apply_group_action(ungroup);
            }
        }
    }
    pub(super) fn verify_groups(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.group_verification_frame else {
            return;
        };
        match frame {
            1 => {
                let original = self.doc.strokes.clone();
                let ids: HashSet<_> = original.iter().take(18).map(|s| s.id).collect();
                self.select_mode = true;
                self.selected = ids.clone();
                self.apply_group_action(false);
                let grouped = self.doc.strokes.clone();
                let group = self.doc.strokes[0].group_id.unwrap();
                assert_eq!(self.doc.group_members(group), ids);
                assert_eq!(self.doc.selection_for_stroke(original[0].id, false), ids);
                assert_eq!(
                    self.doc.selection_for_stroke(original[0].id, true),
                    HashSet::from([original[0].id])
                );
                self.undo();
                assert_eq!(self.doc.strokes, original);
                self.redo();
                assert_eq!(self.doc.strokes, grouped);
                self.selected = ids.clone();
                self.duplicate_selected();
                let copies = self.selected.clone();
                let copy_group = self.doc.strokes.last().unwrap().group_id.unwrap();
                assert_ne!(copy_group, group);
                assert_eq!(self.doc.group_members(copy_group), copies);
                self.transform_mode = Mode::Move;
                self.transform_values = [0.4, 0.3, 0.0];
                self.numeric_transform();
                let moved = self.doc.strokes.clone();
                self.undo();
                self.redo();
                assert_eq!(self.doc.strokes, moved);
                self.apply_group_action(true);
                assert!(self
                    .doc
                    .strokes
                    .iter()
                    .filter(|s| copies.contains(&s.id))
                    .all(|s| s.group_id.is_none()));
                self.undo();
                assert_eq!(self.doc.group_members(copy_group), copies);
                let loaded =
                    Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
                assert_eq!(loaded.version, 24);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.apply_layer_action(editing::LayerAction::Lock(1, true));
                assert!(self
                    .doc
                    .selection_for_stroke(original[0].id, false)
                    .is_empty());
                self.undo();
                self.selected = copies;
                self.select_mode = true;
                self.transform_mode = Mode::Move;
                assert!(
                    gizmo::project(&self.camera, rect, self.doc.strokes[0].points[0].position)
                        .is_some()
                );
                self.status="Grouped house detail / duplicate, transform, ungroup, undo, layers and persistence passed.".into();
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Group screenshot timed out"),
            _ => {}
        }
        self.group_verification_frame = Some(frame + 1);
    }
}
