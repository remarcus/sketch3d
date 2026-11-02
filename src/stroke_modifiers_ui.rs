use super::*;

#[derive(Clone)]
struct ModifierDraft {
    before: crate::editing::Snapshot,
    after: crate::editing::Snapshot,
    stack: crate::stroke_modifiers::StrokeModifierStack,
    changed: bool,
}

impl SketchApp {
    pub(super) fn stroke_modifier_controls(&mut self, ui: &mut egui::Ui) {
        if self.selected.len() != 1 || self.drag.is_some() || self.repeat.is_some() {
            return;
        }
        let stroke_id = *self.selected.iter().next().unwrap();
        let Some(stroke) = self
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.id == stroke_id)
        else {
            return;
        };
        if stroke.fill.is_some() {
            ui.collapsing("Non-destructive modifiers", |ui| {
                ui.small("Stroke modifiers do not apply to filled surfaces; edit the fill contour instead.");
            });
            return;
        }
        if !self.doc.editable(stroke)
            || self.doc.has_linked_source(stroke_id)
            || self.doc.linked_instance_for_stroke(stroke_id).is_some()
            || stroke.points.len() > crate::stroke_modifiers::MAX_POINTS
        {
            return;
        }
        let state_id = ui.make_persistent_id(("stroke-modifiers", stroke_id));
        let mut draft = ui
            .ctx()
            .data_mut(|data| data.get_temp::<ModifierDraft>(state_id));
        if draft.as_ref().is_some_and(|state| {
            crate::editing::snapshot(&self.doc, &HashSet::from([stroke_id])) != state.after
        }) {
            ui.ctx()
                .data_mut(|data| data.remove::<ModifierDraft>(state_id));
            draft = None;
            self.status =
                "Modifier preview was cancelled because its source stroke changed.".into();
        }
        let mut start = false;
        let mut cancel = false;
        let mut apply = false;
        let mut stack_changed = false;
        let mut bake = false;
        let mut close_draft = false;

        ui.collapsing("Non-destructive modifiers", |ui| {
            ui.small("Thickness, taper and smooth are evaluated from the saved source points. Apply stores the ordered stack; Bake commits its evaluated geometry.");
            if draft.is_none() {
                start = ui.button("Edit modifiers…").clicked();
                return;
            }
            let state = draft.as_mut().unwrap();
            let mut working = state.stack.clone();
            ui.label(format!("{} / {} modifiers", working.modifiers.len(), crate::stroke_modifiers::MAX_MODIFIERS));
            for index in 0..working.modifiers.len() {
                let mut modifier = working.modifiers[index];
                let mut move_up = false;
                let mut move_down = false;
                let mut remove = false;
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut modifier.enabled, modifier_name(modifier.kind).to_string());
                        move_up = ui.button("↑").on_hover_text("Move modifier earlier").clicked();
                        move_down = ui.button("↓").on_hover_text("Move modifier later").clicked();
                        remove = ui.button("Remove").clicked();
                    });
                    match &mut modifier.kind {
                        crate::stroke_modifiers::ModifierKind::Thickness { factor } => {
                            stack_changed |= ui
                                .add(egui::Slider::new(factor, 0.05..=20.0).text("Scale"))
                                .changed();
                        }
                        crate::stroke_modifiers::ModifierKind::Taper { start, end } => {
                            stack_changed |= ui
                                .add(egui::Slider::new(start, 0.0..=0.5).text("Start"))
                                .changed();
                            stack_changed |= ui
                                .add(egui::Slider::new(end, 0.0..=0.5).text("End"))
                                .changed();
                        }
                        crate::stroke_modifiers::ModifierKind::Smooth { amount, iterations } => {
                            stack_changed |= ui
                                .add(egui::Slider::new(amount, 0.0..=1.0).text("Amount"))
                                .changed();
                            stack_changed |= ui
                                .add(egui::Slider::new(
                                    iterations,
                                    1..=crate::stroke_modifiers::MAX_SMOOTH_ITERATIONS,
                                )
                                .text("Passes"))
                                .changed();
                        }
                    }
                });
                if modifier != working.modifiers[index] {
                    working.modifiers[index] = modifier;
                    stack_changed = true;
                }
                if remove {
                    working.modifiers.remove(index);
                    stack_changed = true;
                    break;
                }
                if move_up && index > 0 {
                    working.modifiers.swap(index, index - 1);
                    stack_changed = true;
                    break;
                }
                if move_down && index + 1 < working.modifiers.len() {
                    working.modifiers.swap(index, index + 1);
                    stack_changed = true;
                    break;
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        working.modifiers.len() < crate::stroke_modifiers::MAX_MODIFIERS,
                        egui::Button::new("+ Thickness"),
                    )
                    .clicked()
                {
                    working.modifiers.push(crate::stroke_modifiers::StrokeModifier {
                        enabled: true,
                        kind: crate::stroke_modifiers::ModifierKind::Thickness { factor: 1.25 },
                    });
                    stack_changed = true;
                }
                if ui
                    .add_enabled(
                        working.modifiers.len() < crate::stroke_modifiers::MAX_MODIFIERS,
                        egui::Button::new("+ Taper"),
                    )
                    .clicked()
                {
                    working.modifiers.push(crate::stroke_modifiers::StrokeModifier {
                        enabled: true,
                        kind: crate::stroke_modifiers::ModifierKind::Taper {
                            start: 0.15,
                            end: 0.15,
                        },
                    });
                    stack_changed = true;
                }
                if ui
                    .add_enabled(
                        working.modifiers.len() < crate::stroke_modifiers::MAX_MODIFIERS,
                        egui::Button::new("+ Smooth"),
                    )
                    .clicked()
                {
                    working.modifiers.push(crate::stroke_modifiers::StrokeModifier {
                        enabled: true,
                        kind: crate::stroke_modifiers::ModifierKind::Smooth {
                            amount: 0.25,
                            iterations: 1,
                        },
                    });
                    stack_changed = true;
                }
            });
            ui.horizontal(|ui| {
                apply = ui.button("Apply stack").clicked();
                bake = ui.button("Bake modifiers").clicked();
                cancel = ui.button("Cancel").clicked();
            });
            if ui.input(|input| input.key_pressed(Key::Escape)) {
                cancel = true;
                ui.ctx().input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, Key::Escape);
                });
            }
            if stack_changed {
                if !working.is_valid(state.before[0].1.points.len()) {
                    self.status = "Modifier values exceed the supported limits.".into();
                } else {
                    match make_after(&state.before, Some(working.clone())) {
                        Ok(after) => {
                            state.stack = working;
                            state.changed = true;
                            crate::editing::replace(&mut self.doc, &state.before, &after, true);
                            state.after = after;
                            self.renderer.invalidate();
                        }
                        Err(error) => self.status = error,
                    }
                }
            }
            if bake {
                match crate::stroke_modifiers::bake(&state.after[0].1) {
                    Ok(mut baked) => {
                        baked.modifiers = None;
                        let after = vec![(state.after[0].0, baked)];
                        crate::editing::replace(&mut self.doc, &state.after, &after, true);
                        let edit = crate::editing::Edit::Change {
                            before: state.before.clone(),
                            after,
                        };
                        self.undo.push(edit);
                        self.redo.clear();
                        self.dirty = true;
                        self.record_recovery_edit();
                        ui.ctx()
                            .data_mut(|data| data.remove::<ModifierDraft>(state_id));
                        self.status = "Modifiers baked into the stroke as one undo edit.".into();
                        close_draft = true;
                        self.renderer.invalidate();
                    }
                    Err(error) => self.status = error,
                }
            }
            if apply && !close_draft {
                if state.changed {
                    let edit = crate::editing::Edit::Change {
                        before: state.before.clone(),
                        after: state.after.clone(),
                    };
                    self.undo.push(edit);
                    self.redo.clear();
                    self.dirty = true;
                    self.record_recovery_edit();
                    self.status = "Stroke modifier stack applied as one undo edit.".into();
                }
                ui.ctx().data_mut(|data| data.remove::<ModifierDraft>(state_id));
                self.renderer.invalidate();
                close_draft = true;
            }
            if cancel && !close_draft {
                if state.changed {
                    crate::editing::replace(&mut self.doc, &state.before, &state.after, false);
                    self.renderer.invalidate();
                }
                ui.ctx().data_mut(|data| data.remove::<ModifierDraft>(state_id));
                self.status = "Stroke modifier preview cancelled.".into();
                close_draft = true;
            }
        });

        if let Some(state) = draft.filter(|_| !close_draft) {
            ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
        }

        if start {
            let selected = HashSet::from([stroke_id]);
            let before = crate::editing::snapshot(&self.doc, &selected);
            if before.len() == 1 {
                let stack = before[0].1.modifiers.clone().unwrap_or_default();
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        state_id,
                        ModifierDraft {
                            before: before.clone(),
                            after: before,
                            stack,
                            changed: false,
                        },
                    )
                });
            }
        }
    }
}

fn modifier_name(kind: crate::stroke_modifiers::ModifierKind) -> &'static str {
    match kind {
        crate::stroke_modifiers::ModifierKind::Thickness { .. } => "Thickness",
        crate::stroke_modifiers::ModifierKind::Taper { .. } => "Taper",
        crate::stroke_modifiers::ModifierKind::Smooth { .. } => "Smooth",
    }
}

fn make_after(
    before: &crate::editing::Snapshot,
    stack: Option<crate::stroke_modifiers::StrokeModifierStack>,
) -> Result<crate::editing::Snapshot, String> {
    let mut after = before.clone();
    if after.len() != 1 {
        return Err("Select exactly one editable ordinary stroke.".into());
    }
    let stroke = &mut after[0].1;
    stroke.modifiers = stack.filter(|stack| !stack.modifiers.is_empty());
    if let Some(stack) = stroke.modifiers.as_ref() {
        let _ = stack.evaluate(&before[0].1)?;
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point, Stroke};
    use glam::Vec3;

    fn document() -> Document {
        Document {
            strokes: vec![Stroke {
                fill: None,
                material: Default::default(),
                group_id: None,
                layer_id: 1,
                id: 8,
                color: [0.25, 0.5, 0.75],
                gradient: None,
                radius: 0.04,
                brush: Brush::default(),
                points: vec![
                    Point {
                        position: Vec3::ZERO,
                        pressure: 0.3,
                        tilt: None,
                        time_ms: 0,
                    },
                    Point {
                        position: Vec3::new(0.5, 0.2, 0.0),
                        pressure: 0.7,
                        tilt: None,
                        time_ms: 10,
                    },
                    Point {
                        position: Vec3::X,
                        pressure: 0.9,
                        tilt: None,
                        time_ms: 20,
                    },
                ],
                point_thickness: None,
                modifiers: None,
                curve: None,
            }],
            ..Document::default()
        }
    }

    #[test]
    fn modifier_preview_cancel_apply_undo_and_bake_are_exact() {
        let mut document = document();
        let original = serde_json::to_value(&document).unwrap();
        let selected = HashSet::from([8]);
        let before = crate::editing::snapshot(&document, &selected);
        let stack = crate::stroke_modifiers::StrokeModifierStack {
            version: crate::stroke_modifiers::CURRENT_VERSION,
            modifiers: vec![crate::stroke_modifiers::StrokeModifier {
                enabled: true,
                kind: crate::stroke_modifiers::ModifierKind::Smooth {
                    amount: 0.5,
                    iterations: 2,
                },
            }],
        };
        let after = make_after(&before, Some(stack)).unwrap();
        crate::editing::replace(&mut document, &before, &after, true);
        assert_eq!(document.strokes[0].points, before[0].1.points);
        let preview_mesh = crate::geometry::stroke_mesh(&document.strokes[0]);
        let source_mesh = crate::geometry::stroke_mesh(&before[0].1);
        assert_eq!(preview_mesh.len(), source_mesh.len());
        assert!(preview_mesh
            .iter()
            .zip(&source_mesh)
            .any(|(preview, source)| preview.position != source.position));
        crate::editing::replace(&mut document, &before, &after, false);
        assert_eq!(serde_json::to_value(&document).unwrap(), original);

        crate::editing::replace(&mut document, &before, &after, true);
        let edit = Edit::Change { before, after };
        edit.apply(&mut document, false);
        assert_eq!(serde_json::to_value(&document).unwrap(), original);
        edit.apply(&mut document, true);
        let baked = crate::stroke_modifiers::bake(&document.strokes[0]).unwrap();
        assert!(baked.modifiers.is_none());
        assert_ne!(baked.points, document.strokes[0].points);
    }
}
