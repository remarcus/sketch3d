use super::*;
use crate::{
    editing::{self, LayerAction},
    gizmo,
};
impl SketchApp {
    pub(super) fn apply_layer_action(&mut self, action: LayerAction) {
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
    pub(super) fn layer_controls(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        egui::CollapsingHeader::new("Drawing layers").default_open(true).show(ui,|ui| {
            for layer in &self.doc.layers {
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.doc.active_layer==layer.id,&layer.name).clicked(){action=Some(LayerAction::Activate(layer.id));}
                    let mut visible=layer.visible;let mut locked=layer.locked;
                    if ui.checkbox(&mut visible,"Show").changed(){action=Some(LayerAction::Visible(layer.id,visible));}
                    if ui.checkbox(&mut locked,"Lock").changed(){action=Some(LayerAction::Lock(layer.id,locked));}
                });
            }
            ui.add(egui::TextEdit::singleline(&mut self.layer_name).hint_text("Layer name").char_limit(64));
            ui.horizontal(|ui| {
                if ui.button("Add").clicked(){action=Some(LayerAction::Add(if self.layer_name.trim().is_empty(){format!("Layer {}",self.doc.layers.iter().map(|l|l.id).max().unwrap_or(0)+1)}else{self.layer_name.clone()}));}
                if ui.button("Rename").clicked(){action=Some(LayerAction::Rename(self.doc.active_layer,self.layer_name.clone()));}
                if ui.button("Delete empty").clicked(){action=Some(LayerAction::Delete(self.doc.active_layer));}
            });
            if !self.doc.layers.iter().any(|l|l.id==self.layer_target){self.layer_target=self.doc.active_layer;}
            egui::ComboBox::from_id_salt("layer-target").selected_text(self.doc.layers.iter().find(|l|l.id==self.layer_target).map_or("Layer",|l|l.name.as_str())).show_ui(ui,|ui|{for layer in &self.doc.layers{ui.selectable_value(&mut self.layer_target,layer.id,&layer.name);}});
            if ui.add_enabled(!self.selected.is_empty(),egui::Button::new("Move selected to layer")).clicked(){action=Some(LayerAction::MoveSelection(self.selected.clone(),self.layer_target));}
            ui.small("Hidden ink is excluded from view/export. Locked ink stays visible but cannot be picked or edited. Delete requires an empty layer.");
        });
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
                assert_eq!(loaded.version, 7);
                assert_eq!(loaded.layers, self.doc.layers);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.status="Layer drawing, lock/visibility, reassignment, undo, and persistence checks passed.".into();
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Layer screenshot timed out"),
            _ => {}
        }
        self.layer_verification_frame = Some(frame + 1);
    }
}
