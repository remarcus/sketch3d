use super::*;
use crate::{editing, gizmo};
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
                assert_eq!(loaded.version, 23);
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
