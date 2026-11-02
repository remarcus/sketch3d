use super::ui_style;
use super::*;
use crate::{
    editing, export_check, geometry,
    texture::{TextureKind, TextureSettings},
};

impl SketchApp {
    pub(super) fn texture_controls(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Surface texture").default_open(true).show(ui,|ui| {
            ui_style::field_row(ui, "Pattern", |ui| {
                ui.horizontal_wrapped(|ui| {
                for kind in [TextureKind::Solid,TextureKind::Grain,TextureKind::DryBrush,TextureKind::Pattern,TextureKind::Graphite,TextureKind::Charcoal] {
                    ui.radio_value(&mut self.brush.texture.kind,kind,kind.label());
                }
                });
            });
            if self.brush.texture.kind!=TextureKind::Solid {
                ui.add(egui::Slider::new(&mut self.brush.texture.scale,0.02..=10.0).logarithmic(true).text("Spacing"));
                ui.add(egui::Slider::new(&mut self.brush.texture.strength,0.0..=1.0).text("Strength"));
                if ui.button("Vary pattern").clicked() {self.brush.texture.seed=self.brush.texture.seed.wrapping_add(1);}
                if self.brush.texture.kind.uses_continuous_alpha() {
                    ui.small("Soft alpha grain stays fixed in world space. Pressure opacity and tilt broadening shape graphite/charcoal coverage.");
                } else {
                    ui.small("Spacing is in world units. Gaps remain part of the selectable stroke footprint.");
                }
            }
        });
    }
    pub(super) fn load_textures(&mut self) {
        self.load_brushes();
        self.doc = export_check::textured_fixture();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.renderer.invalidate();
        self.status="Texture study / Chalk, Graphite, Charcoal, Dry brush, and Pattern. Pressure runs vary across each row; charcoal also shows synthetic tilt broadening.".into();
    }
    pub(super) fn verify_textures(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.texture_verification_frame else {
            return;
        };
        match frame {
            1 => {
                let original = self.doc.strokes.clone();
                let stroke = original
                    .iter()
                    .find(|s| s.brush.texture.kind == TextureKind::Grain && s.points.len() > 1)
                    .unwrap();
                self.selected = HashSet::from([stroke.id]);
                let before = editing::snapshot(&self.doc, &self.selected);
                let after = editing::transformed(
                    &before,
                    Vec3::ZERO,
                    Vec3::new(0.1, 0.2, 0.3),
                    glam::Quat::from_rotation_y(0.4),
                    1.5,
                )
                .unwrap();
                let a = geometry::stroke_mesh(&before[0].1);
                let b = geometry::stroke_mesh(&after[0].1);
                assert_eq!(a.len(), b.len());
                assert!(a
                    .iter()
                    .zip(&b)
                    .all(|(a, b)| glam::Vec2::from(a.uv).distance(glam::Vec2::from(b.uv)) < 1e-4));
                self.duplicate_selected();
                assert_eq!(
                    self.doc.strokes.last().unwrap().brush.texture,
                    stroke.brush.texture
                );
                self.undo();
                assert_eq!(self.doc.strokes, original);
                self.camera.align_guide(self.guide);
                self.brush.kind = BrushKind::Ribbon;
                self.brush.texture = TextureSettings {
                    kind: TextureKind::DryBrush,
                    ..Default::default()
                };
                let start = rect.center() - egui::vec2(100.0, 0.0);
                self.add_point(start, 0.8, rect, true);
                self.add_point(start + egui::vec2(200.0, 0.0), 0.8, rect, false);
                self.finish();
                let added = self.doc.strokes.last().unwrap().clone();
                assert_eq!(added.brush.texture, self.brush.texture);
                self.erase(start + egui::vec2(100.0, 0.0), rect);
                assert!(!self.doc.strokes.iter().any(|s| s.id == added.id));
                self.undo();
                assert_eq!(self.doc.strokes.last().unwrap(), &added);
                let path = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&path).unwrap();
                self.file = Some(path.join("textures.sketch.json"));
                assert!(self.save(false));
                let loaded = Document::from_json(
                    &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(loaded.version, 17);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.load_textures();
                self.brush.texture = TextureSettings {
                    kind: TextureKind::Grain,
                    ..Default::default()
                };
                // Highlight one masked stroke without filling its gaps.
                let id = self
                    .doc
                    .strokes
                    .iter()
                    .find(|s| s.brush.texture.kind == TextureKind::DryBrush && s.points.len() > 1)
                    .unwrap()
                    .id;
                self.selected = HashSet::from([id]);
                self.select_mode = true;
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Texture screenshot timed out"),
            _ => {}
        }
        self.texture_verification_frame = Some(frame + 1);
    }
}
