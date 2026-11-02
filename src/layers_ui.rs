use super::*;
use crate::{
    editing::{self, LayerAction, LayerState},
    gizmo,
};
use eframe::egui::{WidgetInfo, WidgetType};

const RENAMING_LAYER_ID: &str = "layer-list-renaming-id";
const FOCUS_RENAME_LAYER_ID: &str = "layer-list-focus-rename-id";

pub(super) struct LayerPropertiesDraft {
    layer_id: u64,
    before: LayerState,
}

fn layer_actions_menu(
    ui: &mut egui::Ui,
    selected: &HashSet<u64>,
    layer_id: u64,
) -> (Option<LayerAction>, bool) {
    let mut action = None;
    let mut rename = false;
    if ui.button("Rename layer…").clicked() {
        rename = true;
        ui.close_menu();
    }
    ui.separator();
    if ui
        .add_enabled(
            !selected.is_empty(),
            egui::Button::new("Move selected artwork here"),
        )
        .clicked()
    {
        action = Some(LayerAction::MoveSelection(selected.clone(), layer_id));
        ui.close_menu();
    }
    if ui.button("Delete if empty").clicked() {
        action = Some(LayerAction::Delete(layer_id));
        ui.close_menu();
    }
    (action, rename)
}

impl SketchApp {
    pub(super) fn apply_layer_action(&mut self, action: LayerAction) {
        self.finish_layer_properties_edit(true);
        self.finish();
        match editing::layer_edit(&self.doc, action) {
            Ok(edit) => {
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.sync_selection();
                self.renderer.invalidate();
                self.status = "Layer updated".into();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn finish_layer_properties_edit(&mut self, apply: bool) {
        let Some(draft) = self.layer_properties_draft.take() else {
            return;
        };
        let edit = editing::Edit::Layers {
            before: draft.before,
            after: LayerState::capture(&self.doc),
        };
        if apply {
            let changed = match &edit {
                editing::Edit::Layers { before, after } => before != after,
                _ => false,
            };
            if changed {
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.status =
                    "Layer properties updated. One undo restores the previous values.".into();
            }
        } else {
            edit.apply(&mut self.doc, false);
            self.status = "Layer property preview cancelled.".into();
        }
        self.renderer.invalidate();
    }

    fn begin_layer_properties_preview(&mut self, layer_id: u64) {
        if self
            .layer_properties_draft
            .as_ref()
            .is_some_and(|draft| draft.layer_id == layer_id)
        {
            return;
        }
        self.finish_layer_properties_edit(true);
        self.finish();
        self.layer_properties_draft = Some(LayerPropertiesDraft {
            layer_id,
            before: LayerState::capture(&self.doc),
        });
    }

    fn preview_layer_action(&mut self, action: LayerAction) {
        match editing::layer_edit(&self.doc, action) {
            Ok(edit) => {
                edit.apply(&mut self.doc, true);
                self.renderer.invalidate();
                self.status =
                    "Layer property preview. Apply to keep it or Cancel to restore.".into();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn layer_controls(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let mut opacity_responses = Vec::new();
        let mut blend_changes = Vec::new();
        let mut property_buttons = Vec::new();
        let rename_id = egui::Id::new(RENAMING_LAYER_ID);
        let focus_rename_id = egui::Id::new(FOCUS_RENAME_LAYER_ID);
        let renaming = ui.memory(|memory| memory.data.get_temp::<u64>(rename_id));
        let focus_rename = ui.memory(|memory| memory.data.get_temp::<u64>(focus_rename_id));
        let f2 = ui.input(|input| input.key_pressed(Key::F2)) && !ui.ctx().wants_keyboard_input();
        let begin_rename = f2.then_some(self.doc.active_layer);
        let mut requested_rename = begin_rename;

        egui::CollapsingHeader::new("Drawing layers")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Layer list");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small("Active layer is the drawing target");
                    });
                });
                ui.separator();

                for layer in &self.doc.layers {
                    let layer_id = layer.id;
                    let active = self.doc.active_layer == layer_id;
                    let selected_artwork = self
                        .selected
                        .iter()
                        .any(|stroke_id| self.doc.strokes.iter().any(|s| s.id == *stroke_id && s.layer_id == layer_id));
                    let is_renaming = renaming == Some(layer_id);
                    let layer_name = layer.name.clone();
                    let mut begin_inline_rename = false;

                    ui.push_id(("layer-row", layer_id), |ui| {
                        egui::Frame::group(ui.style())
                            .inner_margin(egui::Margin::symmetric(6, 4))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if is_renaming {
                                        let edit = ui.add(
                                            egui::TextEdit::singleline(&mut self.layer_name)
                                                .id_salt(("layer-name-edit", layer_id))
                                                .desired_width(ui.available_width().max(60.0))
                                                .char_limit(64),
                                        );
                                        edit.widget_info(|| {
                                            let mut info = WidgetInfo::text_edit(
                                                ui.is_enabled(),
                                                &self.layer_name,
                                                &self.layer_name,
                                            );
                                            info.label = Some(format!("Rename layer {layer_name}"));
                                            info
                                        });
                                        if focus_rename == Some(layer_id) {
                                            edit.request_focus();
                                            ui.memory_mut(|memory| {
                                                memory.data.remove::<u64>(focus_rename_id)
                                            });
                                        }
                                        let escape = ui.input(|input| input.key_pressed(Key::Escape));
                                        let enter = ui.input(|input| input.key_pressed(Key::Enter));
                                        if escape {
                                            ui.memory_mut(|memory| {
                                                memory.data.remove::<u64>(rename_id);
                                                memory.data.remove::<u64>(focus_rename_id);
                                            });
                                            self.layer_name.clear();
                                        } else if enter || (edit.lost_focus() && enter) {
                                            action = Some(LayerAction::Rename(layer_id, self.layer_name.clone()));
                                            ui.memory_mut(|memory| {
                                                memory.data.remove::<u64>(rename_id);
                                                memory.data.remove::<u64>(focus_rename_id);
                                            });
                                            self.layer_name.clear();
                                        }
                                    } else {
                                        let label = if active {
                                            format!("●  {}", layer_name)
                                        } else {
                                            format!("   {}", layer_name)
                                        };
                                        let name_response = ui
                                            .selectable_label(active, label)
                                            .on_hover_text(if active {
                                                "Active drawing layer"
                                            } else {
                                                "Make this the active drawing layer"
                                            });
                                        if name_response.clicked() {
                                            action = Some(LayerAction::Activate(layer_id));
                                        }
                                        name_response.context_menu(|ui| {
                                            let (menu_action, rename) =
                                                layer_actions_menu(ui, &self.selected, layer_id);
                                            if let Some(menu_action) = menu_action {
                                                action = Some(menu_action);
                                            }
                                            if rename {
                                                begin_inline_rename = true;
                                            }
                                        });
                                    }

                                    let mut visible = layer.visible;
                                    let visibility_label = if visible { "Hide" } else { "Show" };
                                    if ui
                                        .button(visibility_label)
                                        .on_hover_text(format!("{visibility_label} layer {layer_name}"))
                                        .clicked()
                                    {
                                        visible = !visible;
                                        action = Some(LayerAction::Visible(layer_id, visible));
                                    }

                                    let mut locked = layer.locked;
                                    let lock_label = if locked { "Unlock" } else { "Lock" };
                                    if ui
                                        .button(lock_label)
                                        .on_hover_text(format!("{lock_label} layer {layer_name}"))
                                        .clicked()
                                    {
                                        locked = !locked;
                                        action = Some(LayerAction::Lock(layer_id, locked));
                                    }
                                });

                                ui.horizontal(|ui| {
                                    ui.add_space(6.0);
                                    if selected_artwork {
                                        ui.small("Contains selected artwork");
                                    } else if active {
                                        ui.small("Drawing target");
                                    }
                                    if !layer.visible {
                                        ui.small("Hidden");
                                    }
                                    if layer.locked {
                                        ui.small("Locked");
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.menu_button("Actions", |ui| {
                                                let (menu_action, rename) =
                                                    layer_actions_menu(ui, &self.selected, layer_id);
                                                if let Some(menu_action) = menu_action {
                                                    action = Some(menu_action);
                                                }
                                                if rename {
                                                    begin_inline_rename = true;
                                                }
                                            });
                                        },
                                    );
                                });

                                ui.horizontal(|ui| {
                                    let mut opacity = layer.opacity;
                                    let response = ui
                                        .add(
                                            egui::Slider::new(&mut opacity, 0.0..=1.0)
                                                .text("Opacity")
                                                .custom_formatter(|value, _| {
                                                    format!("{:.0}%", value * 100.0)
                                                })
                                                .custom_parser(|text| {
                                                    text.trim_end_matches('%')
                                                        .parse::<f64>()
                                                        .ok()
                                                        .map(|value| value / 100.0)
                                                }),
                                        )
                                        .on_hover_text(
                                            "Multiplies the opacity of each mark in this layer",
                                        );
                                    opacity_responses.push((
                                        layer_id,
                                        opacity,
                                        response.changed(),
                                        response.drag_stopped(),
                                        response.lost_focus(),
                                    ));
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Blend mode");
                                    let mut blend_mode = layer.blend_mode;
                                    let mut changed = false;
                                    egui::ComboBox::from_id_salt(("layer-blend-mode", layer_id))
                                        .selected_text(blend_mode.label())
                                        .show_ui(ui, |ui| {
                                            for mode in [
                                                crate::document::LayerBlendMode::Normal,
                                                crate::document::LayerBlendMode::Multiply,
                                                crate::document::LayerBlendMode::Screen,
                                                crate::document::LayerBlendMode::Add,
                                            ] {
                                                changed |= ui
                                                    .selectable_value(
                                                        &mut blend_mode,
                                                        mode,
                                                        mode.label(),
                                                    )
                                                    .changed();
                                            }
                                        });
                                    if changed {
                                        blend_changes.push((layer_id, blend_mode));
                                    }
                                    if self
                                        .layer_properties_draft
                                        .as_ref()
                                        .is_some_and(|draft| draft.layer_id == layer_id)
                                    {
                                        let apply = ui.button("Apply").clicked();
                                        let cancel = ui.button("Cancel").clicked();
                                        property_buttons.push((layer_id, apply, cancel));
                                    }
                                });
                            });
                    });

                    if begin_inline_rename {
                        self.layer_name = layer_name;
                        requested_rename = Some(layer_id);
                    }
                }

                if requested_rename.is_some() {
                    let target = requested_rename.take().unwrap();
                    if let Some(layer) = self.doc.layers.iter().find(|layer| layer.id == target) {
                        self.layer_name = layer.name.clone();
                    }
                    ui.memory_mut(|memory| memory.data.insert_temp(rename_id, target));
                    ui.memory_mut(|memory| memory.data.insert_temp(focus_rename_id, target));
                }

                if !self.doc.layers.iter().any(|layer| layer.id == self.layer_target) {
                    self.layer_target = self.doc.active_layer;
                }

                if renaming.is_none() {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("New layer");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.layer_name)
                                .id_salt("new-layer-name")
                                .hint_text("Optional name")
                                .desired_width(95.0)
                                .char_limit(64),
                        );
                        if ui.button("Add").clicked() {
                            let name = if self.layer_name.trim().is_empty() {
                                format!(
                                    "Layer {}",
                                    self.doc.layers.iter().map(|l| l.id).max().unwrap_or(0) + 1
                                )
                            } else {
                                self.layer_name.clone()
                            };
                            action = Some(LayerAction::Add(name));
                            self.layer_name.clear();
                        }
                    });
                }

                ui.horizontal(|ui| {
                    ui.label("Move artwork to");
                    let selected_name = self
                        .doc
                        .layers
                        .iter()
                        .find(|layer| layer.id == self.layer_target)
                        .map_or("Choose layer", |layer| layer.name.as_str());
                    let combo = egui::ComboBox::from_id_salt("layer-target")
                        .selected_text(
                            selected_name,
                        )
                        .show_ui(ui, |ui| {
                            for layer in &self.doc.layers {
                                ui.selectable_value(&mut self.layer_target, layer.id, &layer.name);
                            }
                        });
                    combo.response.widget_info(|| {
                        WidgetInfo::labeled(
                            WidgetType::ComboBox,
                            combo.response.enabled(),
                            format!("Move artwork to: {selected_name}"),
                        )
                    });
                });
                if ui
                    .add_enabled(
                        !self.selected.is_empty(),
                        egui::Button::new("Move selected artwork"),
                    )
                    .clicked()
                {
                    action = Some(LayerAction::MoveSelection(
                        self.selected.clone(),
                        self.layer_target,
                    ));
                }
                ui.small("F2 renames the active layer. Hidden ink is excluded from view and export; locked ink stays visible but cannot be edited. Only empty layers can be deleted.");
            });

        let cancel_properties = ui.input(|input| input.key_pressed(Key::Escape))
            && self.layer_properties_draft.is_some();
        if cancel_properties {
            self.finish_layer_properties_edit(false);
        } else {
            for (layer_id, opacity, changed, drag_stopped, lost_focus) in opacity_responses {
                if changed {
                    self.begin_layer_properties_preview(layer_id);
                    self.preview_layer_action(LayerAction::Opacity(layer_id, opacity));
                }
                if drag_stopped {
                    if self
                        .layer_properties_draft
                        .as_ref()
                        .is_some_and(|draft| draft.layer_id == layer_id)
                    {
                        self.finish_layer_properties_edit(true);
                    }
                }
                if lost_focus
                    && self
                        .layer_properties_draft
                        .as_ref()
                        .is_some_and(|draft| draft.layer_id == layer_id)
                {
                    self.finish_layer_properties_edit(true);
                }
            }
            for (layer_id, blend_mode) in blend_changes {
                self.begin_layer_properties_preview(layer_id);
                self.preview_layer_action(LayerAction::BlendMode(layer_id, blend_mode));
            }
            for (layer_id, apply, cancel) in property_buttons {
                if cancel
                    && self
                        .layer_properties_draft
                        .as_ref()
                        .is_some_and(|draft| draft.layer_id == layer_id)
                {
                    self.finish_layer_properties_edit(false);
                } else if apply
                    && self
                        .layer_properties_draft
                        .as_ref()
                        .is_some_and(|draft| draft.layer_id == layer_id)
                {
                    self.finish_layer_properties_edit(true);
                }
            }
        }

        if let Some(action) = action {
            self.apply_layer_action(action);
        }
    }

    pub(super) fn verify_layers(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.layer_verification_frame else {
            return;
        };
        match frame {
            1 => {
                let original = self.doc.strokes.clone();
                self.apply_layer_action(LayerAction::Add("Details".into()));
                let layer = self.doc.active_layer;
                self.camera.align_guide(self.guide);
                let start = rect.center();
                self.add_point(start, 0.6, rect, true);
                self.add_point(start + egui::vec2(90.0, 25.0), 0.9, rect, false);
                self.finish();
                let stroke = self.doc.strokes.last().unwrap().clone();
                assert_eq!(stroke.layer_id, layer);
                self.apply_layer_action(LayerAction::Lock(layer, true));
                let count = self.doc.strokes.len();
                self.add_point(start, 0.5, rect, true);
                self.finish();
                assert_eq!(self.doc.strokes.len(), count);
                assert_ne!(
                    gizmo::pick(&self.doc, &self.camera, rect, start),
                    Some(stroke.id)
                );
                self.erase(start, rect);
                assert!(self.doc.strokes.iter().any(|s| s.id == stroke.id));
                self.apply_layer_action(LayerAction::Lock(layer, false));
                self.selected = HashSet::from([stroke.id]);
                self.apply_layer_action(LayerAction::MoveSelection(self.selected.clone(), 1));
                assert_eq!(self.doc.strokes.last().unwrap().layer_id, 1);
                self.undo();
                assert_eq!(self.doc.strokes.last().unwrap().layer_id, layer);
                self.redo();
                self.apply_layer_action(LayerAction::Delete(layer));
                assert!(!self.doc.layers.iter().any(|l| l.id == layer));
                self.undo();
                self.apply_layer_action(LayerAction::Visible(1, false));
                assert!(editing::snapshot(&self.doc, &HashSet::from([original[0].id])).is_empty());
                self.undo();
                assert!(editing::layer_edit(&self.doc, LayerAction::Delete(1)).is_err());
                let loaded =
                    Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
                assert_eq!(loaded.version, 23);
                assert_eq!(loaded.layers, self.doc.layers);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.status = "Layer drawing, lock/visibility, reassignment, undo, and persistence checks passed.".into();
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Layer screenshot timed out"),
            _ => {}
        }
        self.layer_verification_frame = Some(frame + 1);
    }
}
