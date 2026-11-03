use super::ui_style;
use super::*;
use crate::{
    editing, export_check, geometry, gizmo, texture,
    texture::{
        ImageMaskSource, ImageTextureMode, ImageTextureSettings, TextureKind, TextureSettings,
    },
};
use std::io::Read;

impl SketchApp {
    pub(super) fn texture_controls(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Surface texture")
            .default_open(true)
            .show(ui, |ui| {
                let mut anchor_mode = self.brush.texture.anchor_mode;
                egui::ComboBox::from_label("Texture anchor")
                    .selected_text(match anchor_mode {
                        crate::texture::TextureAnchorMode::StrokeFollowing => "Stroke following",
                        crate::texture::TextureAnchorMode::CapturedPlanar => "Captured planar surface",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut anchor_mode,
                            crate::texture::TextureAnchorMode::StrokeFollowing,
                            "Stroke following",
                        );
                        ui.selectable_value(
                            &mut anchor_mode,
                            crate::texture::TextureAnchorMode::CapturedPlanar,
                            "Captured planar surface",
                        );
                    });
                if anchor_mode != self.brush.texture.anchor_mode {
                    self.brush.texture.anchor_mode = anchor_mode;
                    self.brush.texture.frame = None;
                }
                ui.checkbox(
                    &mut self.brush.texture.scale_with_brush,
                    "Scale texture with brush",
                );
                if self.brush.texture.anchor_mode
                    == crate::texture::TextureAnchorMode::CapturedPlanar
                {
                    ui.small("Strokes from this brush share a captured flat plane frame. Reanchor when drawing on a different plane; curved and OBJ surfaces are unsupported.");
                    ui.small("Captured grain stays on its world-space plane when strokes move or rotate. Scale texture with brush changes its repeat period during stroke scaling.");
                    if ui.button("Reanchor next stroke to current plane").clicked() {
                        self.brush.texture.frame = None;
                    }
                }
                if self.brush.texture.image.is_none() {
                    ui_style::field_row(ui, "Pattern", |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for kind in [
                                TextureKind::Solid,
                                TextureKind::Grain,
                                TextureKind::DryBrush,
                                TextureKind::Pattern,
                                TextureKind::Graphite,
                                TextureKind::Charcoal,
                                TextureKind::Crosshatch,
                            ] {
                                ui.radio_value(&mut self.brush.texture.kind, kind, kind.label());
                            }
                        });
                    });
                    if ui.button("Import repeating PNG texture…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PNG image", &["png"])
                            .pick_file()
                        {
                            match import_repeating_texture(&path) {
                                Ok(asset) => match self.doc.add_brush_tip_asset(asset.clone()) {
                                    Ok(()) => {
                                        self.brush.texture = TextureSettings {
                                            image: Some(ImageTextureSettings {
                                                asset_id: asset.id,
                                                ..Default::default()
                                            }),
                                            kind: TextureKind::Solid,
                                            ..self.brush.texture
                                        };
                                        self.dirty = true;
                                        self.renderer.invalidate();
                                        self.status = format!(
                                            "Repeating texture {} imported",
                                            asset.file_name()
                                        );
                                    }
                                    Err(error) => {
                                        self.status = format!("Repeating texture import failed: {error}");
                                    }
                                },
                                Err(error) => {
                                    self.status = format!("Repeating texture import failed: {error}");
                                }
                            }
                        }
                    }
                }

                if let Some(mut image) = self.brush.texture.image {
                    egui::ComboBox::from_label("Image mode")
                        .selected_text(match image.mode {
                            ImageTextureMode::CoverageMask => "Coverage mask",
                            ImageTextureMode::RgbaWithAlpha => "RGB with alpha",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut image.mode,
                                ImageTextureMode::CoverageMask,
                                "Coverage mask",
                            );
                            ui.selectable_value(
                                &mut image.mode,
                                ImageTextureMode::RgbaWithAlpha,
                                "RGB with alpha",
                            );
                        });
                    egui::ComboBox::from_label("Coverage source")
                        .selected_text(match image.mask_source {
                            ImageMaskSource::Alpha => "Alpha",
                            ImageMaskSource::Luminance => "Luminance",
                            ImageMaskSource::LuminanceTimesAlpha => "Luminance × alpha",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut image.mask_source,
                                ImageMaskSource::Alpha,
                                "Alpha",
                            );
                            ui.selectable_value(
                                &mut image.mask_source,
                                ImageMaskSource::Luminance,
                                "Luminance",
                            );
                            ui.selectable_value(
                                &mut image.mask_source,
                                ImageMaskSource::LuminanceTimesAlpha,
                                "Luminance × alpha",
                            );
                        });
                    if image.mode == ImageTextureMode::RgbaWithAlpha {
                        ui.checkbox(&mut image.tint_with_brush, "Tint RGB with brush color");
                    } else {
                        ui.small("Coverage masks are tinted with the brush color.");
                    }
                    ui.checkbox(&mut image.invert, "Invert coverage");
                    ui.add(egui::Slider::new(&mut image.contrast, 0.0..=4.0).text("Contrast"));
                    ui.add(egui::Slider::new(&mut image.strength, 0.0..=1.0).text("Strength"));
                    self.brush.texture.image = Some(image);
                    ui.add(egui::Slider::new(&mut self.brush.texture.scale, 0.02..=10.0).logarithmic(true).text("Repeat spacing"));
                    ui.add(egui::Slider::new(&mut self.brush.texture.mapping.rotation_degrees, -180.0..=180.0).text("Texture rotation"));
                    for axis in 0..2 {
                        ui.add(egui::Slider::new(&mut self.brush.texture.mapping.scale[axis], 0.05..=20.0).logarithmic(true).text(if axis == 0 { "Texture U scale" } else { "Texture V scale" }));
                        ui.add(egui::DragValue::new(&mut self.brush.texture.mapping.offset[axis]).speed(0.01).range(-1000.0..=1000.0).prefix(if axis == 0 { "U offset " } else { "V offset " }));
                    }
                    if ui.button("Reset texture mapping").clicked() {
                        self.brush.texture.mapping = Default::default();
                    }
                    let preview_key = (image.cache_key(), self.color.map(f32::to_bits));
                    let refresh_preview = self
                        .repeating_texture_preview
                        .as_ref()
                        .is_none_or(|(key, _)| *key != preview_key);
                    if refresh_preview {
                        let preview = self
                            .doc
                            .brush_tip_assets
                            .iter()
                            .find(|asset| asset.id == image.asset_id)
                            .and_then(|asset| {
                                asset
                                    .repeating_preview_pixels(image, self.color)
                                    .ok()
                            });
                        self.repeating_texture_preview = preview.map(|preview| {
                            let size = [preview.size[0] as usize, preview.size[1] as usize];
                            let image = egui::ColorImage::from_rgba_unmultiplied(size, &preview.pixels);
                            (
                                preview_key,
                                ui.ctx().load_texture(
                                    format!("repeating-brush-texture-{:016x}", preview_key.0.0),
                                    image,
                                    egui::TextureOptions::NEAREST,
                                ),
                            )
                        });
                    }
                    if let Some((_, texture)) = &self.repeating_texture_preview {
                        ui.label("Repeat preview · transparent gutters");
                        ui.image((texture.id(), egui::vec2(96.0, 96.0)));
                    }
                    if ui.button("Clear repeating image texture").clicked() {
                        self.brush.texture.image = None;
                        self.brush.texture.kind = TextureKind::Solid;
                    }
                    ui.small("Image colors repeat along the stroke. Transparent texels leave open gaps.");
                    ui.small("Repeat spacing uses document world units; U/V scale and offsets use tile coordinates.");
                    if self.brush.tip_asset_id != 0 && image.strength > 0.0 {
                        if self.brush.flow.enabled {
                            ui.small("Image-tip grain cannot be exported with Airbrush flow geometry yet.");
                        } else {
                            ui.small("Image-tip alpha and repeating image coverage multiply; tip color and tint stay intact. GLB/OBJ export bakes the result into a per-stamp atlas.");
                        }
                    }
                } else if self.brush.texture.kind != TextureKind::Solid {
                    if matches!(
                        self.brush.texture.kind,
                        TextureKind::Grain | TextureKind::DryBrush | TextureKind::Crosshatch
                    ) {
                        let kind = self.brush.texture.kind;
                        let mut generator = self.brush.texture.generator;
                        ui.separator();
                        ui.strong(match kind {
                            TextureKind::Grain => "Paper grain generator",
                            TextureKind::DryBrush => "Directional fiber generator",
                            _ => "Crosshatch generator",
                        });
                        ui.add(
                            egui::Slider::new(&mut generator.frequency, 1..=8)
                                .text("Frequency"),
                        );
                        ui.add(
                            egui::Slider::new(&mut generator.coarseness, 1..=4)
                                .text(match kind {
                                    TextureKind::Grain => "Coarse scale",
                                    TextureKind::DryBrush => "Streak length",
                                    _ => "Line coarseness",
                                }),
                        );
                        ui.add(
                            egui::Slider::new(&mut generator.direction_degrees, -180.0..=180.0)
                                .text(match kind {
                                    TextureKind::Grain => "Grain direction",
                                    TextureKind::DryBrush => "Fiber direction",
                                    _ => "Direction",
                                }),
                        );
                        ui.add(
                            egui::Slider::new(&mut generator.anisotropy, 0.5..=2.0)
                                .text(match kind {
                                    TextureKind::Grain => "Directional anisotropy",
                                    TextureKind::DryBrush => "Fiber spacing ratio",
                                    _ => "Hatch spacing ratio",
                                }),
                        );
                        ui.horizontal_wrapped(|ui| {
                            match kind {
                                TextureKind::Grain => {
                                    if ui.button("Fine paper").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 8,
                                            coarseness: 1,
                                            ..Default::default()
                                        };
                                    }
                                    if ui.button("Soft paper").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 2,
                                            coarseness: 4,
                                            ..Default::default()
                                        };
                                    }
                                    if ui.button("Directional paper").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 6,
                                            coarseness: 2,
                                            direction_degrees: 30.0,
                                            anisotropy: 1.5,
                                            ..Default::default()
                                        };
                                    }
                                }
                                TextureKind::DryBrush => {
                                    if ui.button("Long fibers").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 4,
                                            coarseness: 4,
                                            direction_degrees: 0.0,
                                            anisotropy: 1.25,
                                            ..Default::default()
                                        };
                                    }
                                    if ui.button("Short fibers").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 8,
                                            coarseness: 1,
                                            direction_degrees: 0.0,
                                            anisotropy: 1.0,
                                            ..Default::default()
                                        };
                                    }
                                    if ui.button("Angled fibers").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 6,
                                            coarseness: 2,
                                            direction_degrees: 35.0,
                                            anisotropy: 1.5,
                                            ..Default::default()
                                        };
                                    }
                                }
                                _ => {
                                    if ui.button("Fine hatch").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 8,
                                            coarseness: 1,
                                            ..Default::default()
                                        };
                                    }
                                    if ui.button("Bold hatch").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 2,
                                            coarseness: 4,
                                            ..Default::default()
                                        };
                                    }
                                    if ui.button("Directional hatch").clicked() {
                                        generator = crate::texture::TextureGeneratorSettings {
                                            frequency: 6,
                                            coarseness: 2,
                                            direction_degrees: 30.0,
                                            anisotropy: 1.5,
                                            ..Default::default()
                                        };
                                    }
                                }
                            }
                            if ui.button("Reset generator").clicked() {
                                generator = Default::default();
                            }
                        });
                        ui.small("Procedural coverage changes stroke gaps; it does not simulate physical paint.");
                        self.brush.texture.generator = generator;
                    }
                    ui.add(egui::Slider::new(&mut self.brush.texture.scale, 0.02..=10.0).logarithmic(true).text("Spacing"));
                    ui.add(egui::Slider::new(&mut self.brush.texture.strength, 0.0..=1.0).text("Strength"));
                    ui.add(egui::Slider::new(&mut self.brush.texture.threshold, 0.0..=0.95).text("Threshold"));
                    ui.add(egui::Slider::new(&mut self.brush.texture.contrast, 0.0..=4.0).text("Contrast"));
                    ui.small("Threshold clears lower coverage values; contrast reshapes the remaining grain alpha.");
                    ui.add(egui::Slider::new(&mut self.brush.texture.mapping.rotation_degrees, -180.0..=180.0).text("Texture rotation"));
                    for axis in 0..2 {
                        ui.add(egui::Slider::new(&mut self.brush.texture.mapping.scale[axis], 0.05..=20.0).logarithmic(true).text(if axis == 0 { "Texture U scale" } else { "Texture V scale" }));
                        ui.add(egui::DragValue::new(&mut self.brush.texture.mapping.offset[axis]).speed(0.01).range(-1000.0..=1000.0).prefix(if axis == 0 { "U offset " } else { "V offset " }));
                    }
                    if ui.button("Reset texture mapping").clicked() { self.brush.texture.mapping = Default::default(); }
                    if ui.button("Vary pattern").clicked() {self.brush.texture.seed=self.brush.texture.seed.wrapping_add(1);}
                    if self.brush.texture.kind.uses_continuous_alpha() {
                        ui.small("Soft alpha grain stays fixed in world space. Pressure controls coverage; reported tilt broadens graphite/charcoal. Mouse or missing tilt uses the No-tilt fallback width.");
                    } else {
                        ui.small("Spacing is in world units. Gaps remain part of the selectable stroke footprint.");
                    }
                    if self.brush.tip_asset_id != 0 && self.brush.texture.strength > 0.0 {
                        if self.brush.flow.enabled {
                            ui.small("Image-tip grain cannot be exported with Airbrush flow geometry yet.");
                        } else {
                            ui.small("Image-tip alpha and grain coverage multiply; tip color and tint stay intact. GLB/OBJ export bakes the result into a per-stamp atlas.");
                        }
                    }
                    ui.small("U/V scale and offsets use tile coordinates; repeat spacing uses document world units.");
                }
                if self.brush.texture.image.is_none()
                    && self.brush.texture.kind != TextureKind::Solid
                {
                    let key = texture::key(self.brush.texture);
                    if self
                        .procedural_texture_preview
                        .as_ref()
                        .is_none_or(|(cached_key, _)| *cached_key != key)
                    {
                        let pixels = texture::rgba(self.brush.texture);
                        let size = [texture::SIZE as usize, texture::SIZE as usize];
                        let image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                        let preview = ui.ctx().load_texture(
                            format!("procedural-mask-{key:?}"),
                            image,
                            egui::TextureOptions::NEAREST,
                        );
                        self.procedural_texture_preview = Some((key, preview));
                    }
                    ui.horizontal(|ui| {
                        ui.small("Coverage preview · white keeps the mark, gaps remove it");
                        if let Some((_, preview)) = &self.procedural_texture_preview {
                            ui.image((preview.id(), egui::vec2(64.0, 64.0)));
                        }
                    });
                }
                if self.brush.texture.image.is_some()
                    || self.brush.texture.kind != TextureKind::Solid
                {
                    egui::CollapsingHeader::new("Grain dynamics")
                        .default_open(false)
                        .show(ui, |ui| {
                            let dynamics = &mut self.brush.texture.dynamics;
                            texture_response_controls(
                                ui,
                                "Pressure → grain strength",
                                &mut dynamics.pressure_strength,
                                1.0,
                            );
                            texture_response_controls(
                                ui,
                                "Pressure → grain contrast",
                                &mut dynamics.pressure_contrast,
                                4.0,
                            );
                            texture_response_controls(
                                ui,
                                "Speed → grain strength",
                                &mut dynamics.speed_strength,
                                1.0,
                            );
                            texture_response_controls(
                                ui,
                                "Speed → grain contrast",
                                &mut dynamics.speed_contrast,
                                4.0,
                            );
                            if dynamics.speed_enabled() {
                                ui.label("Speed is measured from stored world distance and elapsed contact time.");
                                ui.add(egui::Slider::new(
                                    &mut dynamics.speed_minimum,
                                    0.0..=50.0,
                                ).text("Slow speed (world units/s)"));
                                ui.add(egui::Slider::new(
                                    &mut dynamics.speed_maximum,
                                    0.1..=100.0,
                                ).text("Fast speed (world units/s)"));
                                if dynamics.speed_maximum <= dynamics.speed_minimum {
                                    dynamics.speed_maximum =
                                        (dynamics.speed_minimum + 0.01).min(1000.0);
                                }
                            }
                            if dynamics.pressure_strength.enabled
                                || dynamics.pressure_contrast.enabled
                            {
                                ui.add(egui::Slider::new(
                                    &mut dynamics.manual_pressure,
                                    0.0..=1.0,
                                ).text("Manual pressure fallback"));
                            }
                            if dynamics.speed_enabled() {
                                ui.add(egui::Slider::new(
                                    &mut dynamics.manual_speed,
                                    0.0..=1.0,
                                ).text("Manual speed fallback"));
                            }
                            ui.small("Pressure and speed affect grain only; width, opacity, and Airbrush flow remain independent. Missing input uses the manual fallback values.");
                            if dynamics.is_enabled() && self.brush.tip_asset_id == 0 {
                                ui.small("GLB/OBJ export currently blocks dynamic grain on ordinary brush geometry; image-tip strokes bake it into the atlas.");
                            }
                        });
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
                assert_eq!(loaded.version, 30);
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

fn texture_response_controls(
    ui: &mut egui::Ui,
    label: &str,
    response: &mut crate::texture::TextureResponse,
    max_scale: f32,
) {
    ui.checkbox(&mut response.enabled, label);
    if response.enabled {
        ui.add(egui::Slider::new(&mut response.minimum, 0.0..=max_scale).text("Minimum scale"));
        ui.add(egui::Slider::new(&mut response.maximum, 0.0..=max_scale).text("Maximum scale"));
        if response.minimum > response.maximum {
            std::mem::swap(&mut response.minimum, &mut response.maximum);
        }
        ui.add(egui::Slider::new(&mut response.curve, 0.25..=4.0).text("Response curve"));
    }
}

fn import_repeating_texture(
    path: &std::path::Path,
) -> Result<crate::brush_tip::BrushTipAsset, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Choose a regular PNG image file".into());
    }
    let mut bytes =
        Vec::with_capacity(metadata.len().min(crate::brush_tip::MAX_PNG_BYTES as u64) as usize);
    std::fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(crate::brush_tip::MAX_PNG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > crate::brush_tip::MAX_PNG_BYTES {
        return Err("Repeating texture PNG must be no larger than 512 KiB".into());
    }
    let provenance = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.chars().count() <= 240 && !name.chars().any(char::is_control))
        .map(|name| format!("Imported from {name}"));
    crate::brush_tip::BrushTipAsset::from_png(bytes, provenance)
}
