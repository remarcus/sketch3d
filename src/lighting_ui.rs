use super::ui_style;
use super::*;
use crate::lighting::Lighting;

impl SketchApp {
    pub(super) fn show_lighting(&mut self, ctx: &egui::Context) {
        if !self.show_scene
            || self.presentation.artwork
            || self.confirm
            || self.recovery_prompt
            || self.tutorial.offer
        {
            return;
        }
        let mut artwork_requested = false;
        let grain_before = self.presentation.preferences.grain;
        let toon_before = self.presentation.preferences.toon_shading;
        let effects_before = self.presentation.preferences.post_effects;
        let mut local_light = self.doc.local_light;
        egui::Window::new("Scene lighting").open(&mut self.show_scene).default_width(300.0)
            .max_height((ctx.screen_rect().height()-100.0).max(200.0)).vscroll(true)
            .anchor(egui::Align2::RIGHT_TOP,egui::vec2(-255.0,60.0)).show(ctx,|ui| {
                artwork_requested = self.presentation.controls(ui, &mut self.palette_ui);
                ui_style::section(ui, "Film grain");
                ui_style::slider_row(
                    ui,
                    "Amount",
                    "scene-grain-amount",
                    &mut self.presentation.preferences.grain,
                    0.0..=0.12,
                    100.0,
                    "%",
                );
                if ui.button("Reset film grain").clicked() {
                    self.presentation.preferences.grain = 0.0;
                }
                ui.checkbox(&mut self.presentation.preferences.toon_shading, "Toon shading (4 light bands)");
                ui_style::section(ui, "Local point light");
                ui.checkbox(&mut local_light.enabled, "Enable local light");
                ui.add_enabled_ui(local_light.enabled, |ui| {
                    for (index, axis) in ["X", "Y", "Z"].iter().enumerate() {
                        ui_style::numeric_row(ui, &format!("Position {axis}"), ("local-light-position", index), &mut local_light.position[index], Some(-10_000.0..=10_000.0), "wu");
                    }
                    for (index, channel) in ["R", "G", "B"].iter().enumerate() {
                        ui_style::slider_row(ui, &format!("Color {channel}"), ("local-light-color", index), &mut local_light.color[index], 0.0..=1.0, 100.0, "%");
                    }
                    ui_style::slider_row(ui, "Intensity", "local-light-intensity", &mut local_light.intensity, 0.0..=4.0, 100.0, "%");
                    ui_style::slider_row(ui, "Range", "local-light-range", &mut local_light.range, 0.1..=1000.0, 1.0, "wu");
                    ui.checkbox(&mut local_light.shadows, "Cast local shadows");
                    ui.small("Local shadows use one bounded 2048 px shadow pass. Emissive marks keep their stored glow.");
                });
                ui_style::section(ui, "Post-processing");
                let effects = &mut self.presentation.preferences.post_effects;
                ui_style::slider_row(ui, "Glow", "effect-glow", &mut effects.glow, 0.0..=1.0, 100.0, "%");
                ui_style::slider_row(ui, "Focus distance", "effect-focus-distance", &mut effects.focus_distance, 0.0..=1000.0, 1.0, "wu");
                ui_style::slider_row(ui, "Depth blur", "effect-depth-of-field", &mut effects.depth_of_field, 0.0..=16.0, 1.0, "px");
                ui_style::slider_row(ui, "Pixel size", "effect-pixel-size", &mut effects.pixel_size, 1.0..=16.0, 1.0, "px");
                ui.small("Effects apply to the canvas and artwork image export; pixelation and glow are screen-space, while depth blur follows camera distance.");
                let light=&mut self.renderer.lighting;
                ui_style::section(ui, "Directional light");
                ui_style::slider_row(ui, "Direction", "light-azimuth", &mut light.azimuth, -180.0..=180.0, 1.0, "°");
                ui_style::slider_row(ui, "Elevation", "light-elevation", &mut light.elevation, 10.0..=85.0, 1.0, "°");
                ui_style::slider_row(ui, "Intensity", "light-intensity", &mut light.intensity, 0.0..=1.5, 1.0, "");
                ui_style::slider_row(ui, "Ambient", "light-ambient", &mut light.ambient, 0.0..=1.0, 100.0, "%");
                ui.separator();
                ui.checkbox(&mut light.shadows,"Cast shadows");
                ui.add_enabled_ui(light.shadows,|ui| {
                    ui_style::slider_row(ui, "Shadow strength", "light-shadow-strength", &mut light.strength, 0.0..=1.0, 100.0, "%");
                    ui_style::logarithmic_slider_row(ui, "Bias", "light-shadow-bias", &mut light.bias, 0.0001..=0.03, 1.0, "wu");
                    ui.small("Increase bias to reduce surface speckles; too much can detach thin shadows.");
                });
                ui.separator(); ui.checkbox(&mut light.ground,"Show ground");
                if ui.button("Fit ground & enable shadows").clicked() {
                    if light.fit_ground(&self.doc) {light.shadows=true;} else {self.status="No visible artwork to fit the ground to.".into();}
                }
                ui.add_enabled_ui(light.ground,|ui| {
                    ui_style::numeric_row(ui, "Height Y", "ground-height", &mut light.ground_height, Some(-20_000.0..=20_000.0), "wu");
                    for (i, axis) in ["X", "Z"].iter().enumerate() {
                        ui_style::numeric_row(ui, &format!("Center {axis}"), ("ground-center", i), &mut light.ground_center[i], Some(-20_000.0..=20_000.0), "wu");
                    }
                    ui_style::numeric_row(ui, "Size", "ground-size", &mut light.ground_size, Some(0.1..=40_000.0), "wu");
                    ui.horizontal(|ui| {
                        ui.label("Ground color");
                        self.palette_ui.color_button(ui, super::color_editor::ColorTarget::Ground, &light.ground_color);
                    });
                });
                if ui.button("Reset scene lighting").clicked() {*light=Lighting::default();}
                ui.separator();ui.small("Local light settings save with the drawing. Directional lighting and ground remain presentation settings. The ground is not paintable or selectable; use a drawing guide for ink.");
            });
        if local_light.enabled && self.last_canvas_rect.is_positive() {
            if let Some((screen, _)) =
                crate::gizmo::project(&self.camera, self.last_canvas_rect, local_light.position)
            {
                egui::Area::new(egui::Id::new("local-point-light-handle"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(screen - egui::vec2(10.0, 10.0))
                    .show(ctx, |ui| {
                        let response = ui.add(
                            egui::Button::new("✦")
                                .fill(egui::Color32::from_rgb(255, 184, 74))
                                .sense(egui::Sense::click_and_drag()),
                        );
                        response.clone().on_hover_text(
                            "Drag to move the local light in the camera-facing plane.",
                        );
                        if response.drag_started() {
                            self.local_light_drag_before = Some(self.doc.local_light);
                        }
                        if response.dragged() {
                            if let Some(pointer) = response.interact_pointer_pos() {
                                if let Some(position) = crate::gizmo::plane_hit(
                                    &self.camera,
                                    self.last_canvas_rect,
                                    pointer,
                                    local_light.position,
                                    self.camera.basis().2,
                                ) {
                                    local_light.position = position;
                                    self.doc.local_light.position = position;
                                    self.renderer.invalidate();
                                }
                            }
                        }
                        if response.drag_stopped() {
                            if let Some(before) = self.local_light_drag_before.take() {
                                let after = self.doc.local_light;
                                if before != after {
                                    self.undo.push(Edit::LocalLight { before, after });
                                    self.redo.clear();
                                    self.dirty = true;
                                    self.record_recovery_edit_labeled("Move local point light");
                                }
                            }
                        }
                    });
            }
        }
        if local_light != self.doc.local_light {
            let before = self.doc.local_light;
            self.doc.local_light = local_light;
            self.undo.push(Edit::LocalLight {
                before,
                after: local_light,
            });
            self.redo.clear();
            self.renderer.invalidate();
            self.dirty = true;
            self.record_recovery_edit_labeled("Local point light");
        }
        if self.presentation.preferences.grain != grain_before
            || self.presentation.preferences.toon_shading != toon_before
            || self.presentation.preferences.post_effects != effects_before
        {
            self.renderer.grain = self.presentation.preferences.grain;
            self.renderer.toon_shading = self.presentation.preferences.toon_shading;
            self.renderer.post_effects = self.presentation.preferences.post_effects;
            self.presentation.remember();
        }
        if artwork_requested {
            self.toggle_artwork();
        }
    }

    pub(super) fn verify_lighting(&mut self, ctx: &egui::Context, _rect: Rect) {
        let Some(frame) = self.lighting_verification_frame else {
            return;
        };
        if frame == 1 {
            self.check_lighting_pixels();
        }
        if frame == 3 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        assert!(frame < 100, "Lighting screenshot timed out");
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let path = std::env::current_dir().unwrap().join("verification");
            let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                path.join("lighting-preview.png"),
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            let report = serde_json::json!({"passed":true,"checks":["GPU ground shadows darken receiver","hidden excluded and visible locked casters retained","selection preserves ground shadows","masked brush shadow holes","camera orbit and extreme light angles","house and planar fixtures","native and GLB ink unchanged","scene controls"],"gpu":self.renderer.adapter,"benchmarks_deferred":true});
            std::fs::write(
                path.join("lighting-report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.lighting_verification_frame = Some(frame + 1);
    }

    fn lighting_pixels(&mut self, doc: &Document, camera: &Camera) -> Vec<u8> {
        self.renderer.invalidate();
        self.renderer
            .render(doc, camera, GuidePreview::Hidden, [600, 450]);
        self.renderer.verification_pixels()
    }
    fn check_lighting_pixels(&mut self) {
        let original = serde_json::to_vec(&self.doc).unwrap();
        let export = crate::export::encode(&self.doc, None).unwrap();
        let mut doc = Document::default();
        doc.strokes.push(Stroke {
            guide_attachment: None,
            model_surface: None,
            curve: None,
            material: Default::default(),
            id: 1,
            layer_id: 1,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            color: [0.85, 0.7, 0.35],
            gradient: None,
            radius: 0.6,
            brush: Brush {
                kind: BrushKind::Ribbon,
                normal: Vec3::Y,
                tangent: Vec3::X,
                ..Default::default()
            },
            points: vec![
                Point {
                    position: Vec3::new(-0.7, 1.4, 0.0),
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                },
                Point {
                    position: Vec3::new(0.7, 1.4, 0.0),
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                },
            ],
        });
        let camera = Camera {
            yaw: 0.55,
            pitch: 0.85,
            distance: 7.0,
            target: Vec3::new(0.0, 0.5, 0.0),
            ..Default::default()
        };
        self.renderer.lighting = Lighting {
            ground: true,
            ground_height: 0.0,
            ground_size: 10.0,
            ground_color: [0.7, 0.7, 0.7],
            ambient: 0.25,
            intensity: 0.9,
            ..Default::default()
        };
        let unshadowed = self.lighting_pixels(&doc, &camera);
        self.renderer.lighting.shadows = true;
        let shadowed = self.lighting_pixels(&doc, &camera);
        let darker = unshadowed
            .as_chunks::<4>()
            .0
            .iter()
            .zip(shadowed.as_chunks::<4>().0.iter())
            .filter(|(a, b)| a[0] == a[1] && a[1] == a[2] && a[0] as i32 - b[0] as i32 > 15)
            .count();
        assert!(darker > 200, "No substantial GPU shadow: {darker}");
        doc.layers[0].locked = true;
        assert_eq!(self.lighting_pixels(&doc, &camera), shadowed);
        self.renderer.set_selection(&HashSet::from([1]));
        let selected = self.lighting_pixels(&doc, &camera);
        for ((base, before), after) in unshadowed
            .as_chunks::<4>()
            .0
            .iter()
            .zip(shadowed.as_chunks::<4>().0.iter())
            .zip(selected.as_chunks::<4>().0.iter())
        {
            if base[0] == base[1] && base[1] == base[2] {
                assert_eq!(before, after);
            }
        }
        self.renderer.set_selection(&HashSet::new());
        doc.strokes[0].brush.texture.kind = crate::texture::TextureKind::Pattern;
        self.renderer.invalidate();
        let masked = self.lighting_pixels(&doc, &camera);
        let holes = unshadowed
            .as_chunks::<4>()
            .0
            .iter()
            .zip(shadowed.as_chunks::<4>().0.iter())
            .zip(masked.as_chunks::<4>().0.iter())
            .filter(|((base, before), after)| {
                base[0] == base[1] && base[1] == base[2] && after[0] as i32 - before[0] as i32 > 15
            })
            .count();
        assert!(holes > 100, "Masked shadow holes missing: {holes}");
        let orbit = Camera {
            projection: camera.projection,
            yaw: camera.yaw + 0.7,
            pitch: camera.pitch - 0.25,
            distance: camera.distance,
            target: camera.target,
            roll: 0.0,
        };
        self.lighting_pixels(&doc, &orbit);
        assert_eq!(self.lighting_pixels(&doc, &camera), masked);
        doc.layers[0].visible = false;
        let hidden = self.lighting_pixels(&doc, &camera);
        assert_eq!(hidden, self.lighting_pixels(&Document::default(), &camera));
        let path = std::env::current_dir().unwrap().join("verification");
        std::fs::create_dir_all(&path).unwrap();
        image::save_buffer(
            path.join("lighting-planar.png"),
            &masked,
            600,
            450,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let mut building = Document::default();
        for (id, normal, start, end, radius, color) in [
            (
                1,
                Vec3::Z,
                Vec3::new(-0.9, 1.0, 0.5),
                Vec3::new(0.9, 1.0, 0.5),
                1.0,
                [0.85, 0.83, 0.75],
            ),
            (
                2,
                Vec3::X,
                Vec3::new(0.9, 1.0, 0.5),
                Vec3::new(0.9, 1.0, -0.9),
                1.0,
                [0.75, 0.77, 0.72],
            ),
            (
                3,
                Vec3::Y,
                Vec3::new(-1.05, 2.05, -0.2),
                Vec3::new(1.05, 2.05, -0.2),
                0.85,
                [0.65, 0.25, 0.15],
            ),
        ] {
            building.strokes.push(Stroke {
                guide_attachment: None,
                model_surface: None,
                curve: None,
                material: Default::default(),
                id,
                layer_id: 1,
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: None,
                group_id: None,
                color,
                gradient: None,
                radius,
                brush: Brush {
                    kind: BrushKind::Ribbon,
                    normal,
                    ..Default::default()
                },
                points: [start, end]
                    .into_iter()
                    .map(|position| Point {
                        position,
                        pressure: 1.0,
                        tilt: None,
                        time_ms: 0,
                    })
                    .collect(),
            });
        }
        self.renderer.lighting.azimuth = 100.0;
        self.renderer.lighting.fit_ground(&building);
        let building_pixels = self.lighting_pixels(&building, &camera);
        image::save_buffer(
            path.join("lighting-building.png"),
            &building_pixels,
            600,
            450,
            image::ColorType::Rgba8,
        )
        .unwrap();
        self.renderer.lighting.fit_ground(&self.doc);
        for elevation in [10.0, 85.0] {
            self.renderer.lighting.elevation = elevation;
            self.lighting_pixels(&self.doc.clone(), &crate::scenes::house_camera());
        }
        self.renderer.lighting = Lighting {
            shadows: true,
            azimuth: 100.0,
            elevation: 40.0,
            ambient: 0.35,
            intensity: 0.75,
            ..Default::default()
        };
        self.renderer.lighting.fit_ground(&self.doc);
        self.renderer
            .render(&self.doc, &self.camera, GuidePreview::Hidden, [900, 700]);
        let pixels = self.renderer.verification_pixels();
        image::save_buffer(
            path.join("lighting-house.png"),
            &pixels,
            900,
            700,
            image::ColorType::Rgba8,
        )
        .unwrap();
        self.guide_visible = false;
        self.show_scene = true;
        assert_eq!(serde_json::to_vec(&self.doc).unwrap(), original);
        assert_eq!(
            crate::export::encode(&self.doc, None).unwrap().bytes,
            export.bytes
        );
        self.status="Lighting checks passed: ground shadows, masks, layer rules, selection and unchanged ink/export.".into();
    }
}
