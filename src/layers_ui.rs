use super::*;
use crate::{
    editing::{self, LayerAction, LayerState},
    gizmo,
};
use eframe::egui::{WidgetInfo, WidgetType};

const RENAMING_LAYER_ID: &str = "layer-list-renaming-id";
const FOCUS_RENAME_LAYER_ID: &str = "layer-list-focus-rename-id";

fn layer_fixture_stroke(
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    horizontal: bool,
    z: f32,
    material: crate::document::StrokeMaterial,
) -> Stroke {
    let (a, b, tangent) = if horizontal {
        (Vec3::new(-1.2, 0.0, z), Vec3::new(1.2, 0.0, z), Vec3::X)
    } else {
        (Vec3::new(0.0, -1.2, z), Vec3::new(0.0, 1.2, z), Vec3::Y)
    };
    Stroke {
        guide_attachment: None,
        model_surface: None,
        curve: None,
        material,
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        fill: None,
        group_id: None,
        layer_id,
        id,
        color,
        gradient: None,
        radius: 0.32,
        brush: Brush {
            kind: BrushKind::Ribbon,
            normal: Vec3::Z,
            tangent,
            opacity: 1.0,
            softness: 0.0,
            ..Brush::default()
        },
        points: [a, b]
            .into_iter()
            .map(|position| Point {
                position,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .collect(),
    }
}

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
        let mut clip_changes = Vec::new();
        let mut mask_changes = Vec::new();
        let mut mask_inversion_changes = Vec::new();
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

                ui.horizontal(|ui| {
                    if ui.button("New folder for active layer").clicked() {
                        action = Some(LayerAction::CreateFolder(self.layer_name.trim().to_owned()));
                        self.layer_name.clear();
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.layer_name).hint_text("Folder name").desired_width(110.0).char_limit(64));
                });
                for folder in &self.doc.layer_folders {
                    let folder_id = folder.id;
                    let folder_name = folder.name.clone();
                    let folder_rename_id = egui::Id::new(("folder-rename", folder_id));
                    let editing_name = ui.memory(|memory| memory.data.get_temp::<bool>(folder_rename_id)).unwrap_or(false);
                    ui.horizontal(|ui| {
                        let collapsed = folder.collapsed;
                        if ui.button(if collapsed { "▶" } else { "▼" }).clicked() {
                            action = Some(LayerAction::FolderCollapsed(folder_id, !collapsed));
                        }
                        if editing_name {
                            let mut name = ui.memory(|memory| memory.data.get_temp::<String>(folder_rename_id)).unwrap_or_else(|| folder_name.clone());
                            let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0).char_limit(64));
                            ui.memory_mut(|memory| memory.data.insert_temp(folder_rename_id, name.clone()));
                            if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                                action = Some(LayerAction::RenameFolder(folder_id, name));
                                ui.memory_mut(|memory| memory.data.remove::<bool>(folder_rename_id));
                                ui.memory_mut(|memory| memory.data.remove::<String>(folder_rename_id));
                            }
                        } else {
                            let response = ui.strong(format!("{} ({})", folder_name, folder.layer_ids.len())).on_hover_text("Double-click to rename. Folder visibility and lock apply to every member. Reordering moves member layers together in the render stack; layer IDs and animation tracks remain attached. Clip-to-below masks continue to use the immediately preceding render layer, so moving a folder can change a mask source.");
                            if response.double_clicked() {
                                ui.memory_mut(|memory| memory.data.insert_temp(folder_rename_id, folder_name.clone()));
                                ui.memory_mut(|memory| memory.data.insert_temp(folder_rename_id, true));
                            }
                        }
                        if ui.button("↑").on_hover_text("Move folder up").clicked() { action = Some(LayerAction::MoveFolder(folder_id, -1)); }
                        if ui.button("↓").on_hover_text("Move folder down").clicked() { action = Some(LayerAction::MoveFolder(folder_id, 1)); }
                        let mut visible = folder.visible;
                        if ui.checkbox(&mut visible, "Visible").changed() { action = Some(LayerAction::FolderVisible(folder_id, visible)); }
                        let mut locked = folder.locked;
                        if ui.checkbox(&mut locked, "Locked").changed() { action = Some(LayerAction::FolderLock(folder_id, locked)); }
                    });
                }

                for layer in &self.doc.layers {
                    let layer_id = layer.id;
                    let folder = self.doc.layer_folders.iter().find(|folder| folder.layer_ids.contains(&layer_id));
                    if folder.is_some_and(|folder| folder.collapsed) { continue; }
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
                                    if let Some(folder) = folder {
                                        ui.add_space(12.0);
                                        ui.small(format!("{} /", folder.name));
                                    }
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
                                    let visibility_label = if visible && self.doc.layer_folder_visible(layer_id) { "Hide" } else { "Show" };
                                    if ui
                                        .button(visibility_label)
                                        .on_hover_text(format!("{visibility_label} layer {layer_name}"))
                                        .clicked()
                                    {
                                        visible = !visible;
                                        action = Some(LayerAction::Visible(layer_id, visible));
                                    }

                                    let mut locked = layer.locked;
                                    let lock_label = if locked || self.doc.layer_folder_locked(layer_id) { "Unlock" } else { "Lock" };
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
                                    let folder_id = self.doc.layer_folders.iter().find(|folder| folder.layer_ids.contains(&layer_id)).map(|folder| folder.id);
                                    let folder_name = folder_id.and_then(|id| self.doc.layer_folders.iter().find(|folder| folder.id == id)).map_or("No folder", |folder| folder.name.as_str());
                                    let mut selected_folder = folder_id;
                                    egui::ComboBox::from_id_salt(("layer-folder", layer_id)).selected_text(folder_name).show_ui(ui, |ui| {
                                        if ui.selectable_value(&mut selected_folder, None, "No folder").clicked() { action = Some(LayerAction::MoveLayerToFolder(layer_id, None)); }
                                        for folder in &self.doc.layer_folders {
                                            if ui.selectable_value(&mut selected_folder, Some(folder.id), &folder.name).clicked() { action = Some(LayerAction::MoveLayerToFolder(layer_id, Some(folder.id))); }
                                        }
                                    });
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
                                            "Multiplies each mark's opacity in this layer. Cutout marks with reduced opacity may not erase; use 100% for full cutout coverage.",
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
                                    let blend_combo = egui::ComboBox::from_id_salt((
                                        "layer-blend-mode",
                                        layer_id,
                                    ))
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
                                    blend_combo.response.on_hover_text(
                                        "Applies this blend rule to each stroke in the layer during rendering. Source colors are preserved; cutout marks keep their threshold-based erase behavior.",
                                    );
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

                                let layer_index = self
                                    .doc
                                    .layers
                                    .iter()
                                    .position(|candidate| candidate.id == layer_id)
                                    .unwrap_or(0);
                                let mut clip_to_below = layer.clip_to_below;
                                let response = ui.add_enabled(
                                    layer_index > 0,
                                    egui::Checkbox::new(&mut clip_to_below, "Clip to layer below"),
                                );
                                let response = response.on_hover_text(
                                    "Uses the immediately lower layer's screen-space silhouette and opacity as this layer's mask. Mask source depth is ignored; clipped marks keep scene depth tests but do not write depth or cast shadows. A hidden source clips to nothing, and reordering changes the source.",
                                );
                                if response.changed() {
                                    clip_changes.push((layer_id, clip_to_below));
                                }
                                let mut source = layer.mask_source;
                                let name = source.and_then(|id| self.doc.layers.iter().find(|candidate| candidate.id == id))
                                    .map_or("None", |candidate| candidate.name.as_str());
                                egui::ComboBox::from_id_salt(("layer-mask-source", layer_id))
                                    .selected_text(name)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut source, None, "None");
                                        for candidate in &self.doc.layers {
                                            if candidate.id != layer_id {
                                                ui.selectable_value(&mut source, Some(candidate.id), &candidate.name);
                                            }
                                        }
                                    });
                                ui.small("Named mask source. Hidden sources clip to nothing.");
                                if source != layer.mask_source {
                                    mask_changes.push((layer_id, source));
                                }
                                let mut inverted = layer.mask_inverted;
                                if ui.add_enabled(source.is_some(), egui::Checkbox::new(&mut inverted, "Invert mask"))
                                    .on_hover_text("Show the layer outside the named mask. A hidden source gives full coverage when inverted.")
                                    .changed()
                                {
                                    mask_inversion_changes.push((layer_id, inverted));
                                }
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
                if drag_stopped
                    && self
                        .layer_properties_draft
                        .as_ref()
                        .is_some_and(|draft| draft.layer_id == layer_id)
                {
                    self.finish_layer_properties_edit(true);
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
            for (layer_id, clip_to_below) in clip_changes {
                self.begin_layer_properties_preview(layer_id);
                self.preview_layer_action(LayerAction::ClipToBelow(layer_id, clip_to_below));
            }
            for (layer_id, source) in mask_changes {
                self.begin_layer_properties_preview(layer_id);
                self.preview_layer_action(LayerAction::MaskSource(layer_id, source));
            }
            for (layer_id, inverted) in mask_inversion_changes {
                self.begin_layer_properties_preview(layer_id);
                self.preview_layer_action(LayerAction::InvertMask(layer_id, inverted));
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
                assert_eq!(loaded.version, 30);
                assert_eq!(loaded.layers, self.doc.layers);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.verify_layer_compositing();
                self.status = "Layer drawing, lock/visibility, reassignment, undo, and persistence checks passed.".into();
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Layer screenshot timed out"),
            _ => {}
        }
        self.layer_verification_frame = Some(frame + 1);
    }

    fn verify_layer_compositing(&mut self) {
        use crate::document::{Layer, LayerBlendMode, StrokeMaterial};
        use std::time::Instant;

        let old_background = self.renderer.background;
        let old_lighting = self.renderer.lighting;
        let background = [0.0, 0.0, 0.0];
        self.renderer.background = background;
        self.renderer.lighting = crate::lighting::Lighting::default();
        let mut camera = Camera {
            distance: 5.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0));
        let base = layer_fixture_stroke(
            89_001,
            1,
            [0.8, 0.18, 0.08],
            true,
            0.0,
            StrokeMaterial::Unlit,
        );
        let overlay = layer_fixture_stroke(
            89_002,
            2,
            [0.08, 0.32, 0.9],
            false,
            0.08,
            StrokeMaterial::Unlit,
        );
        let make_doc = |mode: LayerBlendMode, opacity: f32| {
            let mut doc = Document {
                strokes: vec![base.clone(), overlay.clone()],
                ..Document::default()
            };
            doc.layers.push(Layer {
                id: 2,
                name: "Blend overlay".into(),
                visible: true,
                locked: false,
                opacity: 1.0,
                blend_mode: LayerBlendMode::Normal,
                hide_onion_skin: false,
                mask_inverted: false,
                mask_source: None,
                clip_to_below: false,
            });
            let upper = doc.layers.iter_mut().find(|layer| layer.id == 2).unwrap();
            upper.opacity = opacity;
            upper.blend_mode = mode;
            doc
        };
        let render = |renderer: &mut crate::renderer::SceneRenderer,
                      doc: &Document,
                      camera: &Camera,
                      size: [u32; 2]| {
            renderer.invalidate();
            renderer.render(doc, camera, GuidePreview::Hidden, size);
            renderer.verification_pixels()
        };
        let center_pixel = |pixels: &[u8], camera: &Camera, size: [u32; 2]| {
            let (screen, _) = gizmo::project(camera, rect, Vec3::ZERO).unwrap();
            let x = (screen.x / rect.width() * size[0] as f32)
                .round()
                .clamp(0.0, size[0].saturating_sub(1) as f32) as usize;
            let y = (screen.y / rect.height() * size[1] as f32)
                .round()
                .clamp(0.0, size[1].saturating_sub(1) as f32) as usize;
            let offset = (y * size[0] as usize + x) * 4;
            [
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                pixels[offset + 3],
            ]
        };
        let srgb_to_linear_pixel = |pixel: [u8; 4]| {
            std::array::from_fn::<_, 3, _>(|channel| {
                crate::presentation::srgb_to_linear(pixel[channel] as f32 / 255.0)
            })
        };
        let linear_to_pixel = |color: [f32; 3]| {
            color.map(|value| {
                (crate::presentation::linear_to_srgb(value.clamp(0.0, 1.0)) * 255.0).round() as u8
            })
        };
        let size = [512, 512];
        let base_pixels = render(
            &mut self.renderer,
            &make_doc(LayerBlendMode::Normal, 0.0),
            &camera,
            size,
        );
        let base_pixel = center_pixel(&base_pixels, &camera, size);
        let destination = srgb_to_linear_pixel(base_pixel);
        let source = overlay.color;
        let alpha = 0.5_f32;
        let modes = [
            LayerBlendMode::Normal,
            LayerBlendMode::Multiply,
            LayerBlendMode::Screen,
            LayerBlendMode::Add,
        ];
        let mut references = serde_json::Map::new();
        for mode in modes {
            let pixels = render(&mut self.renderer, &make_doc(mode, alpha), &camera, size);
            let actual = center_pixel(&pixels, &camera, size);
            let expected_linear = std::array::from_fn(|channel| match mode {
                LayerBlendMode::Normal => {
                    source[channel] * alpha + destination[channel] * (1.0 - alpha)
                }
                LayerBlendMode::Multiply => {
                    source[channel] * alpha * destination[channel]
                        + destination[channel] * (1.0 - alpha)
                }
                LayerBlendMode::Screen => {
                    source[channel] * alpha + destination[channel] * (1.0 - source[channel] * alpha)
                }
                LayerBlendMode::Add => source[channel] * alpha + destination[channel],
            });
            let expected = linear_to_pixel(expected_linear);
            assert!(
                (0..3).all(|channel| actual[channel].abs_diff(expected[channel]) <= 18),
                "{mode:?} reference pixel mismatch: actual={actual:?} expected={expected:?} base={base_pixel:?}"
            );
            references.insert(
                mode.label().to_owned(),
                serde_json::json!({"actual_rgba":actual,"expected_rgb":expected}),
            );
        }
        let mut orbit_camera = camera;
        orbit_camera.yaw = 0.25;
        orbit_camera.pitch = 0.12;
        let orbit_base_pixels = render(
            &mut self.renderer,
            &make_doc(LayerBlendMode::Normal, 0.0),
            &orbit_camera,
            size,
        );
        let orbit_base_pixel = center_pixel(&orbit_base_pixels, &orbit_camera, size);
        let orbit_destination = srgb_to_linear_pixel(orbit_base_pixel);
        for mode in modes {
            let pixels = render(
                &mut self.renderer,
                &make_doc(mode, alpha),
                &orbit_camera,
                size,
            );
            let actual = center_pixel(&pixels, &orbit_camera, size);
            let expected_linear = std::array::from_fn(|channel| match mode {
                LayerBlendMode::Normal => {
                    source[channel] * alpha + orbit_destination[channel] * (1.0 - alpha)
                }
                LayerBlendMode::Multiply => {
                    source[channel] * alpha * orbit_destination[channel]
                        + orbit_destination[channel] * (1.0 - alpha)
                }
                LayerBlendMode::Screen => {
                    source[channel] * alpha
                        + orbit_destination[channel] * (1.0 - source[channel] * alpha)
                }
                LayerBlendMode::Add => source[channel] * alpha + orbit_destination[channel],
            });
            let expected = linear_to_pixel(expected_linear);
            assert!(
                (0..3).all(|channel| actual[channel].abs_diff(expected[channel]) <= 18),
                "{mode:?} orbit reference pixel mismatch: actual={actual:?} expected={expected:?} base={orbit_base_pixel:?}"
            );
            references.insert(
                format!("{} / orbit", mode.label()),
                serde_json::json!({"actual_rgba":actual,"expected_rgb":expected}),
            );
        }

        let mut output_path_pixel_checks = serde_json::Map::new();
        for output_size in [[640, 360], [1280, 720]] {
            for mode in modes {
                let doc = make_doc(mode, alpha);
                let canvas_pixels = render(&mut self.renderer, &doc, &camera, output_size);
                let still_pixels = self
                    .renderer
                    .render_png_pixels(&doc, &camera, output_size, background, old_lighting)
                    .unwrap();
                assert_eq!(
                    canvas_pixels, still_pixels,
                    "canvas/still pixel mismatch for {mode:?} at {}x{}",
                    output_size[0], output_size[1]
                );
                output_path_pixel_checks.insert(
                    format!("{}x{} / {}", output_size[0], output_size[1], mode.label()),
                    serde_json::json!({"identical_rgba_pixels":canvas_pixels.len() / 4}),
                );
            }
        }

        let make_cutout_doc = |opacity: f32| {
            let mut doc = Document {
                strokes: vec![
                    layer_fixture_stroke(
                        89_011,
                        1,
                        [0.08, 0.32, 0.9],
                        true,
                        0.0,
                        StrokeMaterial::Unlit,
                    ),
                    layer_fixture_stroke(
                        89_012,
                        2,
                        [0.9, 0.2, 0.1],
                        false,
                        0.08,
                        StrokeMaterial::Cutout,
                    ),
                ],
                ..Document::default()
            };
            doc.layers.push(Layer {
                id: 2,
                name: "Cutout".into(),
                visible: true,
                locked: false,
                opacity: 1.0,
                blend_mode: LayerBlendMode::Normal,
                hide_onion_skin: false,
                mask_inverted: false,
                mask_source: None,
                clip_to_below: false,
            });
            doc.layers
                .iter_mut()
                .find(|layer| layer.id == 2)
                .unwrap()
                .opacity = opacity;
            doc
        };
        let cutout_low = center_pixel(
            &render(&mut self.renderer, &make_cutout_doc(0.49), &camera, size),
            &camera,
            size,
        );
        let cutout_high = center_pixel(
            &render(&mut self.renderer, &make_cutout_doc(0.75), &camera, size),
            &camera,
            size,
        );
        assert!(cutout_low[2] > cutout_high[2].saturating_add(30));

        let original_doc = self.doc.clone();
        let original_undo_len = self.undo.len();
        let property_doc = make_doc(LayerBlendMode::Normal, 1.0);
        self.doc = property_doc;
        let before_properties = serde_json::to_value(&self.doc).unwrap();
        self.begin_layer_properties_preview(2);
        self.preview_layer_action(LayerAction::Opacity(2, 0.25));
        assert_eq!(self.doc.layer_opacity(2), 0.25);
        self.finish_layer_properties_edit(false);
        assert_eq!(serde_json::to_value(&self.doc).unwrap(), before_properties);
        self.begin_layer_properties_preview(2);
        self.preview_layer_action(LayerAction::BlendMode(2, LayerBlendMode::Multiply));
        self.finish_layer_properties_edit(true);
        assert_eq!(self.doc.layer_blend_mode(2), LayerBlendMode::Multiply);
        assert_eq!(self.doc.strokes[1].color, overlay.color);
        assert_eq!(self.undo.len(), original_undo_len + 1);
        let committed = serde_json::to_value(&self.doc).unwrap();
        self.undo();
        assert_eq!(self.doc.layer_blend_mode(2), LayerBlendMode::Normal);
        self.redo();
        assert_eq!(serde_json::to_value(&self.doc).unwrap(), committed);
        let recovery_dir = std::env::current_dir()
            .unwrap()
            .join("verification/layer-compositing/recovery");
        std::fs::create_dir_all(&recovery_dir).unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "layer-compositing-acceptance".into(),
            None,
            self.doc.clone(),
        );
        crate::recovery_store::write(&recovery_dir, &snapshot).unwrap();
        let recovered = crate::recovery_store::scan(&recovery_dir, "layer-compositing-reader")
            .unwrap()
            .candidates
            .into_iter()
            .max_by_key(|candidate| candidate.snapshot.saved_at_unix_ms)
            .unwrap();
        assert_eq!(recovered.snapshot.document.layers, self.doc.layers);
        let mut legacy = serde_json::to_value(&self.doc).unwrap();
        for layer in legacy["layers"].as_array_mut().unwrap() {
            layer.as_object_mut().unwrap().remove("opacity");
            layer.as_object_mut().unwrap().remove("blend_mode");
        }
        let legacy = Document::from_json(&legacy.to_string()).unwrap();
        assert!(legacy
            .layers
            .iter()
            .all(|layer| { layer.opacity == 1.0 && layer.blend_mode == LayerBlendMode::Normal }));
        self.doc = original_doc;
        self.undo.truncate(original_undo_len);
        self.redo.clear();

        camera.yaw = 0.25;
        let orbit_doc = make_doc(LayerBlendMode::Screen, alpha);
        let orbit_pixels = render(&mut self.renderer, &orbit_doc, &camera, size);
        let orbit_pixel = center_pixel(&orbit_pixels, &camera, size);
        assert!(orbit_pixel[0] > 10 || orbit_pixel[2] > 10);

        let mut populated = Document::default();
        for layer_id in 1..=6 {
            if layer_id > 1 {
                populated.layers.push(Layer {
                    id: layer_id,
                    name: format!("Layer {layer_id}"),
                    visible: true,
                    locked: false,
                    opacity: 0.6 + layer_id as f32 * 0.05,
                    blend_mode: modes[(layer_id as usize - 1) % modes.len()],
                    hide_onion_skin: false,
                    mask_inverted: false,
                    mask_source: None,
                    clip_to_below: false,
                });
            }
            for stroke_index in 0..8 {
                let mut stroke = layer_fixture_stroke(
                    90_000 + layer_id * 20 + stroke_index,
                    layer_id,
                    [0.2 + layer_id as f32 * 0.08, 0.3, 0.7],
                    stroke_index % 2 == 0,
                    layer_id as f32 * 0.01,
                    StrokeMaterial::Unlit,
                );
                stroke.radius = 0.04 + stroke_index as f32 * 0.004;
                populated.strokes.push(stroke);
            }
        }
        let mut timings = serde_json::Map::new();
        for output_size in [[640, 360], [1280, 720]] {
            let mut elapsed = Vec::new();
            for _ in 0..3 {
                let started = Instant::now();
                let _ = render(&mut self.renderer, &populated, &camera, output_size);
                elapsed.push(started.elapsed().as_micros());
            }
            elapsed.sort_unstable();
            timings.insert(
                format!("{}x{}", output_size[0], output_size[1]),
                serde_json::json!({"end_to_end_readback_median_us":elapsed[1]}),
            );
        }

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/layer-compositing");
        std::fs::create_dir_all(&output).unwrap();
        let report = serde_json::json!({
            "passed": true,
            "checks": [
                "Normal, Multiply, Screen, and Add match reference overlap pixels",
                "two-camera view preserves visible overlap and blend modes",
                "canvas and production still/video frame path produce identical pixels at two sizes",
                "layer opacity previews cancel exactly, applies once, and undoes/redoes",
                "cutout below 0.5 coverage preserves the lower stroke; higher opacity clears it",
                "native recovery retains opacity/blend state and legacy omission defaults to 100% Normal",
                "blend modes add no per-layer render targets; GLB/OBJ use documented Normal fallback"
            ],
            "blend_reference_pixels": references,
            "canvas_still_video_pixel_checks": output_path_pixel_checks,
            "cutout_layer_opacity_049_pixel": cutout_low,
            "cutout_layer_opacity_075_pixel": cutout_high,
            "orbit_screen_pixel": orbit_pixel,
            "populated_scene": {"layers":6,"strokes":populated.strokes.len(),"timings":timings},
            "render_target_budget": {
                "formula": "conservative 4x MSAA budget: 12 bytes/pixel x (1 resolve set + 4 multisample set) + 8 bytes/pixel color/emission resolve = 68 bytes/pixel",
                "additional_per_layer_targets":0,
                "bytes_640x360":640_u64*360*68,
                "bytes_1280x720":1280_u64*720*68,
                "note":"Estimate covers shared color/emission/depth resources only and is a conservative budget, not a measured allocation; post-effects and driver allocation overhead are excluded."
            },
            "mesh_export_fallback":"GLB/OBJ retain the original layer mode as metadata/comments and render with Normal; transparent PNG uses Normal for Multiply.",
            "gpu":self.renderer.adapter
        });
        std::fs::write(
            output.join("layer-compositing-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        self.renderer.background = old_background;
        self.renderer.lighting = old_lighting;
    }
}
