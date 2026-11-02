use super::*;
use crate::{
    geometry,
    presets::{Library, Preset},
};

impl SketchApp {
    pub(super) fn preset_controls(&mut self, ui: &mut egui::Ui) {
        let current = Preset {
            id: 0,
            name: "Current settings".into(),
            brush: self.brush,
            radius: self.radius,
            color: self.color,
            stabilization: self.stabilization,
        };
        if let Some(preset) = self.preset_ui.show(ui, current) {
            self.apply_preset(preset);
        }
    }
    fn apply_preset(&mut self, preset: Preset) {
        self.finish();
        if preset.brush.tip_asset_id != 0 {
            if let Some(asset) = self.preset_ui.library.tip_asset(preset.brush.tip_asset_id) {
                if let Err(error) = self.doc.add_brush_tip_asset(asset.clone()) {
                    self.status = format!("Cannot apply brush tip: {error}");
                    return;
                }
                self.dirty = true;
            } else {
                self.status = "Cannot apply brush tip: preset asset is missing".into();
                return;
            }
        }
        self.brush = preset.brush;
        if self.brush.kind == crate::document::BrushKind::Chisel {
            self.brush.softness = 0.0;
        }
        self.radius = preset.radius;
        self.color = preset.color;
        self.stabilization = preset.stabilization;
        self.eraser = false;
        self.select_mode = false;
        self.status = format!("{} / applied to future strokes", preset.name);
    }
    pub(super) fn verify_presets(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.preset_verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.preset_ui.library = Library::default();
                let path = self.preset_ui.path.clone().unwrap();
                self.preset_ui.library.save(&path).unwrap();
                self.preset_ui.reload();
                let original = self.doc.strokes.clone();
                let original_export = crate::export::encode(&self.doc, None).unwrap().bytes;
                let mut preset = self.preset_ui.library.get(4).unwrap().clone();
                preset.brush.dynamics.taper_end = 0.4;
                preset.brush.texture.seed = 981;
                preset.stabilization = 0.65;
                self.apply_preset(preset.clone());
                assert_eq!(self.brush, preset.brush);
                assert_eq!(self.radius, preset.radius);
                assert_eq!(self.stabilization, preset.stabilization);
                self.camera.align_guide(self.guide);
                let start = rect.center() - egui::vec2(90.0, 0.0);
                self.add_point(start, 0.4, rect, true);
                self.add_point(start + egui::vec2(180.0, 20.0), 0.9, rect, false);
                self.finish();
                let mark = self.doc.strokes.last().unwrap().clone();
                assert_eq!(mark.brush.texture, preset.brush.texture);
                let id = self
                    .preset_ui
                    .library
                    .create(
                        "Personal Chalk",
                        preset.brush,
                        preset.radius,
                        preset.color,
                        preset.stabilization,
                    )
                    .unwrap();
                let copy = self.preset_ui.library.duplicate(id, "Chalk copy").unwrap();
                self.preset_ui.library.rename(id, "Evening Chalk").unwrap();
                let mut changed = preset.brush;
                changed.texture.strength = 0.9;
                changed.dynamics.taper_start = 0.5;
                self.preset_ui
                    .library
                    .update(id, changed, 0.08, [0.9, 0.7, 0.4], 0.8)
                    .unwrap();
                self.preset_ui.library.delete(copy).unwrap();
                self.preset_ui.library.delete(1).unwrap();
                self.preset_ui.library.restore_defaults().unwrap();
                assert_eq!(self.doc.strokes.last().unwrap(), &mark);
                self.preset_ui.library.save(&path).unwrap();
                let expected = self.preset_ui.library.clone();
                self.preset_ui.reload();
                assert_eq!(self.preset_ui.library, expected);
                self.undo();
                assert_eq!(self.doc.strokes, original);
                assert_eq!(
                    crate::export::encode(&self.doc, None).unwrap().bytes,
                    original_export
                );
                // Failed library loading remains isolated from native drawing data.
                let corrupt = path.parent().unwrap().join("corrupt/brushes.json");
                std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
                let source = "{\"version\":99,\"presets\":[]}";
                std::fs::write(&corrupt, source).unwrap();
                self.preset_ui.path = Some(corrupt.clone());
                self.preset_ui.reload();
                assert_eq!(self.preset_ui.library, Library::default());
                assert!(!self.preset_ui.notice.is_empty());
                assert_eq!(std::fs::read_to_string(&corrupt).unwrap(), source);
                self.preset_ui.repair_library().unwrap();
                assert_eq!(Library::load(&corrupt).unwrap(), Library::default());
                assert!(std::fs::read_dir(corrupt.parent().unwrap())
                    .unwrap()
                    .filter_map(Result::ok)
                    .any(|e| e.path() != corrupt
                        && std::fs::read_to_string(e.path()).is_ok_and(|text| text == source)));
                self.preset_ui.path = Some(path);
                self.preset_ui.reload();
                assert_eq!(self.preset_ui.library, expected);
                self.preset_ui.choose(id);
                let chosen = self.preset_ui.library.get(id).unwrap().clone();
                self.apply_preset(chosen.clone());
                let preview = crate::presets_ui::preview_document(&chosen);
                assert_eq!(preview.strokes[0].brush, chosen.brush);
                assert!(!geometry::stroke_mesh(&preview.strokes[0]).is_empty());
                self.camera = Camera::default();
                self.camera.distance = 8.5;
                self.preset_ui.notice="Preset CRUD, restart, captured strokes, export preservation, and repair backup passed.".into();
                self.dirty = false;
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Preset screenshot timed out"),
            _ => {}
        }
        self.preset_verification_frame = Some(frame + 1);
    }
}
