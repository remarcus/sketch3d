use super::ui_style;
use super::*;
use crate::{
    editing, export_check, geometry, gizmo,
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
                    ui.small("Soft alpha grain stays fixed in world space. Pressure controls coverage; reported tilt broadens graphite/charcoal. Mouse or missing tilt uses the No-tilt fallback width.");
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
                self.recovery_scan_seen = true;
                self.recovery_prompt = false;
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

                for kind in [TextureKind::Graphite, TextureKind::Charcoal] {
                    let stroke = original
                        .iter()
                        .find(|stroke| stroke.brush.texture.kind == kind && stroke.points.len() > 1)
                        .expect("texture study contains the requested continuous-alpha brush");
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
                    let transformed_texture = after[0].1.brush.texture;
                    assert_eq!(transformed_texture.kind, stroke.brush.texture.kind);
                    assert_eq!(transformed_texture.strength, stroke.brush.texture.strength);
                    assert_eq!(transformed_texture.seed, stroke.brush.texture.seed);
                    assert!(
                        (transformed_texture.scale - stroke.brush.texture.scale * 1.5).abs()
                            < 1.0e-5,
                        "uniform scaling carries the world-space grain scale"
                    );
                    let before_mesh = geometry::stroke_mesh(&before[0].1);
                    let after_mesh = geometry::stroke_mesh(&after[0].1);
                    assert_eq!(before_mesh.len(), after_mesh.len());
                    assert!(before_mesh.iter().zip(&after_mesh).all(|(a, b)| {
                        glam::Vec2::from(a.uv).distance(glam::Vec2::from(b.uv)) < 1.0e-4
                    }));
                }

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
                let erase_position = geometry::stroke_mesh(&added)
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .find_map(|triangle| {
                        let world = triangle
                            .iter()
                            .map(|vertex| Vec3::from(vertex.position))
                            .sum::<Vec3>()
                            / 3.0;
                        let screen = gizmo::project(&self.camera, rect, world)?.0;
                        (gizmo::pick(&self.doc, &self.camera, rect, screen) == Some(added.id))
                            .then_some(screen)
                    })
                    .expect("textured stroke contains a covered point for erasing");
                self.erase(erase_position, rect);
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
                assert_eq!(loaded.version, 24);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.load_textures();
                self.brush.texture = TextureSettings {
                    kind: TextureKind::Grain,
                    ..Default::default()
                };
                self.selected.clear();
                self.select_mode = false;
                self.camera.align_guide(self.guide);

                for kind in [TextureKind::Graphite, TextureKind::Charcoal] {
                    let stroke = self
                        .doc
                        .strokes
                        .iter()
                        .find(|stroke| stroke.brush.texture.kind == kind && stroke.points.len() > 1)
                        .expect("texture study contains the requested pick target")
                        .clone();
                    let covered_point = geometry::stroke_mesh(&stroke)
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .find_map(|triangle| {
                            let world = triangle
                                .iter()
                                .map(|vertex| Vec3::from(vertex.position))
                                .sum::<Vec3>()
                                / 3.0;
                            let screen = gizmo::project(&self.camera, rect, world)?.0;
                            (gizmo::pick(&self.doc, &self.camera, rect, screen) == Some(stroke.id))
                                .then_some(screen)
                        })
                        .expect("continuous-alpha texture has a pickable covered point");
                    self.erase(covered_point, rect);
                    assert!(!self.doc.strokes.iter().any(|item| item.id == stroke.id));
                    self.undo();
                    assert!(self.doc.strokes.iter().any(|item| item == &stroke));
                }

                let output = std::env::current_dir()
                    .unwrap()
                    .join("verification/sketch-072");
                std::fs::create_dir_all(&output).unwrap();
                let camera = self.camera;
                let lighting = crate::lighting::Lighting {
                    ground: false,
                    shadows: false,
                    ..Default::default()
                };
                let background = [0.94, 0.93, 0.89];
                let mut rendered = Vec::new();
                for size in [[640, 360], [1280, 720], [2560, 1440]] {
                    for (view, yaw_offset) in [("front", 0.0), ("orbit", 0.65)] {
                        let mut view_camera = camera;
                        view_camera.yaw += yaw_offset;
                        let pixels = self
                            .renderer
                            .render_png_pixels(&self.doc, &view_camera, size, background, lighting)
                            .expect("texture study renders at each export size");
                        let image = image::RgbaImage::from_raw(size[0], size[1], pixels)
                            .expect("texture export dimensions match pixel count");
                        let file_name = format!("textures-{view}-{}x{}.png", size[0], size[1]);
                        image.save(output.join(&file_name)).unwrap();
                        rendered.push(serde_json::json!({
                            "view": view,
                            "yaw_offset": yaw_offset,
                            "size": size,
                            "file": file_name,
                        }));
                    }
                }
                std::fs::write(
                    output.join("texture-render-report.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "passed": true,
                        "fixture": "textured_fixture",
                        "background_linear_rgb": background,
                        "renders": rendered,
                        "physical_tablet_tested": false,
                    }))
                    .unwrap(),
                )
                .unwrap();
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Texture screenshot timed out"),
            _ => {}
        }
        self.texture_verification_frame = Some(frame + 1);
    }
}
