use super::ui_style;
use super::*;
use crate::{document::PressureMapping, export_check};

fn mapping_controls(ui: &mut egui::Ui, label: &str, mapping: &mut PressureMapping) {
    ui.checkbox(&mut mapping.enabled, label);
    if mapping.enabled {
        ui_style::slider_row(
            ui,
            "Minimum",
            (label, "minimum"),
            &mut mapping.minimum,
            0.0..=1.0,
            100.0,
            "%",
        );
        ui_style::slider_row(
            ui,
            "Response",
            (label, "curve"),
            &mut mapping.curve,
            0.3..=3.0,
            1.0,
            "",
        );
    }
}

impl SketchApp {
    pub(super) fn dynamics_controls(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Pressure, tilt & taper").default_open(false).show(ui, |ui| {
            mapping_controls(ui, "Pressure: width", &mut self.brush.dynamics.width);
            if self.brush.kind == BrushKind::Chisel && self.brush.tip_asset_id == 0 {
                mapping_controls(ui, "Pressure: thickness", &mut self.brush.dynamics.thickness);
            }
            ui_style::slider_row(ui, "Opacity", "brush-opacity", &mut self.brush.opacity, 0.0..=1.0, 100.0, "%");
            if self.brush.kind != BrushKind::Chisel && self.brush.tip_asset_id == 0 {
                ui_style::slider_row(ui, "Soft edges", "brush-softness", &mut self.brush.softness, 0.0..=1.0, 100.0, "%");
            }
            mapping_controls(ui, "Pressure: opacity", &mut self.brush.dynamics.opacity);
            if (self.brush.kind != BrushKind::Round
                || (self.brush.tube_profile != crate::document::TubeProfile::Round
                    && !self.brush.flow.enabled
                    && !self.brush.strands.enabled
                    && !self.brush.dabs.enabled))
                && self.brush.tip_asset_id == 0
            {
                ui.checkbox(&mut self.brush.dynamics.tilt_orientation, "Tilt orientation").on_hover_text("Uses reported pen lean to orient the cross-section. Missing or upright tilt falls back to the manual frame.");
            }
            if self.brush.texture.kind.uses_continuous_alpha() {
                ui.checkbox(&mut self.brush.dynamics.tilt_shading, "Tilt broad shading")
                    .on_hover_text("Reported lean broadens graphite or charcoal. Its angle also sets the shading direction when tilt orientation is enabled.");
                if self.brush.dynamics.tilt_shading {
                    ui_style::slider_row(
                        ui,
                        "Tilt width",
                        "tilt-shading-width",
                        &mut self.brush.dynamics.tilt_shading_scale,
                        1.0..=6.0,
                        100.0,
                        "×",
                    );
                    ui_style::slider_row(
                        ui,
                        "No-tilt fallback",
                        "manual-shading-width",
                        &mut self.brush.dynamics.manual_shading_scale,
                        1.0..=6.0,
                        100.0,
                        "×",
                    );
                }
            }
            ui_style::slider_row(ui, "Start taper", "taper-start", &mut self.brush.dynamics.taper_start, 0.0..=10.0, 1.0, "wu");
            ui_style::slider_row(ui, "End taper", "taper-end", &mut self.brush.dynamics.taper_end, 0.0..=10.0, 1.0, "wu");
            ui.small("Taper length is in world units. Mouse pressure is constant; missing tilt uses the manual angle.");
        });
    }

    pub(super) fn load_dynamics(&mut self) {
        self.load_brushes();
        self.doc = export_check::dynamics_fixture();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.renderer.invalidate();
        self.status = "Dynamics study / pressure width, independent thickness, tilt orientation, and distance taper. V to edit; New to draw.".into();
    }

    pub(super) fn verify_dynamics(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.dynamics_verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.dirty = false;
                self.request(Pending::New);
                self.camera.align_guide(self.guide);
                self.brush.kind = BrushKind::Chisel;
                self.brush.opacity = 0.64;
                self.brush.softness = 0.0;
                self.brush.dynamics.width.minimum = 0.05;
                self.brush.dynamics.width.curve = 1.8;
                self.brush.dynamics.thickness.enabled = false;
                self.brush.dynamics.opacity = PressureMapping {
                    enabled: true,
                    minimum: 0.18,
                    curve: 1.6,
                };
                self.brush.dynamics.tilt_orientation = true;
                self.brush.dynamics.taper_start = 0.3;
                self.brush.dynamics.taper_end = 0.5;
                self.stabilization = 0.8;
                let scale = ctx.pixels_per_point();
                let start = rect.center() - egui::vec2(130.0, 0.0);
                let end = start + egui::vec2(260.0, 40.0);
                let sample = |position: Pos2, pressure, tilt, phase, contact| PenSample {
                    position: egui::pos2(position.x * scale, position.y * scale),
                    pressure: Some(pressure),
                    tilt,
                    phase,
                    contact,
                    eraser: false,
                    barrel: false,
                };
                self.canvas_input(
                    ctx,
                    rect,
                    vec![
                        sample(start, 0.15, [Some(20.0), Some(30.0)], Phase::Down, true),
                        sample(
                            start + egui::vec2(120.0, -40.0),
                            0.5,
                            [Some(30.0), Some(20.0)],
                            Phase::Move,
                            true,
                        ),
                        sample(end, 0.85, [Some(35.0), Some(25.0)], Phase::Move, true),
                        sample(end, 0.0, [Some(-50.0), Some(-50.0)], Phase::Up, false),
                    ],
                );
                assert_eq!(self.doc.strokes.len(), 1);
                let stroke = self.doc.strokes[0].clone();
                assert_eq!(stroke.brush.dynamics, self.brush.dynamics);
                assert_eq!(stroke.brush.opacity, self.brush.opacity);
                assert_eq!(stroke.brush.softness, self.brush.softness);
                assert_eq!(stroke.points.last().unwrap().pressure, 0.85);
                let tilt = crate::dynamics::screen_tilt(
                    [Some(35.0), Some(25.0)],
                    self.camera.basis().0,
                    self.camera.basis().1,
                )
                .unwrap();
                assert!(stroke
                    .points
                    .last()
                    .unwrap()
                    .tilt
                    .unwrap()
                    .abs_diff_eq(tilt, 1e-5));
                assert!(stroke.points.len() > 3, "release tail must be retained");
                assert_eq!(self.undo.len(), 1);
                self.undo();
                assert!(self.doc.strokes.is_empty());
                self.redo();
                assert_eq!(self.doc.strokes[0], stroke);
                // A canceled second stroke contributes neither geometry nor an undo item.
                self.canvas_input(
                    ctx,
                    rect,
                    vec![
                        sample(start, 0.6, [Some(20.0), Some(20.0)], Phase::Down, true),
                        sample(end, 0.0, [None, None], Phase::Cancel, false),
                    ],
                );
                assert_eq!(self.doc.strokes, vec![stroke.clone()]);
                assert_eq!(self.undo.len(), 1);
                self.canvas_input(
                    ctx,
                    rect,
                    vec![
                        sample(start, 0.7, [None, None], Phase::Down, true),
                        sample(end, 0.7, [None, None], Phase::Move, true),
                        sample(end, 0.0, [None, None], Phase::Up, false),
                    ],
                );
                assert!(self.doc.strokes[1].points.iter().all(|p| p.tilt.is_none()));
                self.brush.dynamics.width.minimum = 0.9;
                assert_eq!(self.doc.strokes[0], stroke);
                let path = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&path).unwrap();
                self.file = Some(path.join("dynamics.sketch.json"));
                assert!(self.save(false));
                let loaded = Document::from_json(
                    &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(loaded.version, 22);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.load_dynamics();
                self.brush.dynamics = Default::default();
                self.brush.dynamics.tilt_orientation = true;
                self.brush.dynamics.taper_start = 0.3;
                self.brush.dynamics.taper_end = 0.5;
            }
            3 => {
                self.verify_opacity_compositing();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            100 => panic!("Dynamics screenshot timed out"),
            _ => {}
        }
        self.dynamics_verification_frame = Some(frame + 1);
    }
}

fn opacity_test_stroke(
    id: u64,
    color: [f32; 3],
    positions: [glam::Vec3; 2],
    tangent: glam::Vec3,
    z: f32,
    opacity: f32,
    softness: f32,
) -> Stroke {
    let mut brush = Brush {
        kind: BrushKind::Ribbon,
        normal: glam::Vec3::Z,
        tangent,
        opacity,
        softness,
        ..Brush::default()
    };
    brush.dynamics.opacity = PressureMapping {
        enabled: true,
        minimum: 0.2,
        curve: 1.0,
    };
    Stroke {
        curve: None,
        material: Default::default(),
        fill: None,
        group_id: None,
        layer_id: 1,
        id,
        color,
        radius: 0.32,
        brush,
        points: positions
            .into_iter()
            .map(|mut position| {
                position.z = z;
                Point {
                    position,
                    pressure: 0.8,
                    tilt: None,
                    time_ms: 0,
                }
            })
            .collect(),
    }
}

impl SketchApp {
    fn verify_opacity_compositing(&mut self) {
        let old_background = self.renderer.background;
        let old_lighting = self.renderer.lighting;
        let background = [0.10, 0.12, 0.16];
        self.renderer.background = background;
        self.renderer.lighting = crate::lighting::Lighting::default();
        let mut camera = Camera {
            distance: 5.0,
            ..Default::default()
        };
        camera.yaw = 0.0;
        camera.pitch = 0.0;
        camera.roll = 0.0;
        let red = opacity_test_stroke(
            8801,
            [0.95, 0.08, 0.06],
            [
                glam::Vec3::new(-1.2, 0.0, 0.0),
                glam::Vec3::new(1.2, 0.0, 0.0),
            ],
            glam::Vec3::X,
            0.0,
            0.65,
            0.75,
        );
        let blue = opacity_test_stroke(
            8802,
            [0.06, 0.18, 0.98],
            [
                glam::Vec3::new(0.0, -1.2, 0.0),
                glam::Vec3::new(0.0, 1.2, 0.0),
            ],
            glam::Vec3::Y,
            0.08,
            0.62,
            0.0,
        );
        let mut doc = Document {
            strokes: vec![red.clone(), blue.clone()],
            ..Default::default()
        };
        for index in 0..6 {
            let horizontal = index % 2 == 0;
            let position_pair = if horizontal {
                [
                    glam::Vec3::new(-1.2, 0.0, 0.0),
                    glam::Vec3::new(1.2, 0.0, 0.0),
                ]
            } else {
                [
                    glam::Vec3::new(0.0, -1.2, 0.0),
                    glam::Vec3::new(0.0, 1.2, 0.0),
                ]
            };
            let color = [
                [0.92, 0.58, 0.12],
                [0.35, 0.92, 0.26],
                [0.76, 0.24, 0.88],
                [0.16, 0.72, 0.82],
                [0.95, 0.42, 0.16],
                [0.52, 0.72, 0.98],
            ][index];
            doc.strokes.push(opacity_test_stroke(
                8810 + index as u64,
                color,
                position_pair,
                if horizontal {
                    glam::Vec3::X
                } else {
                    glam::Vec3::Y
                },
                -0.12 + index as f32 * 0.04,
                0.28,
                if index % 2 == 0 { 0.35 } else { 0.0 },
            ));
        }
        let render = |renderer: &mut SceneRenderer, doc: &Document, camera: &Camera| {
            let started = Instant::now();
            renderer.invalidate();
            renderer.render(doc, camera, GuidePreview::Hidden, [512, 512]);
            let pixels = renderer.verification_pixels();
            let offset = (256 * 512 + 256) * 4;
            (
                [
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ],
                started.elapsed().as_micros(),
                pixels,
            )
        };
        let (overlap, overlap_us, overlap_image) = render(&mut self.renderer, &doc, &camera);
        let output = std::env::current_dir().unwrap().join("verification");
        std::fs::create_dir_all(&output).unwrap();
        image::save_buffer(
            output.join("opacity-overlap.png"),
            &overlap_image,
            512,
            512,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let background_bytes = background.map(|channel| (channel * 255.0).round() as u8);
        assert!(
            overlap[0] > background_bytes[0].saturating_add(30),
            "red translucent contribution should remain visible: {overlap:?}"
        );
        assert!(
            overlap[2] > background_bytes[2].saturating_add(30),
            "blue translucent contribution should remain visible: {overlap:?}"
        );

        doc.strokes.reverse();
        let (reversed, reversed_us, _) = render(&mut self.renderer, &doc, &camera);
        for channel in 0..3 {
            assert!(overlap[channel].abs_diff(reversed[channel]) <= 2, "camera-depth sorting should not depend on document order: {overlap:?} vs {reversed:?}");
        }
        self.renderer.set_selection(&HashSet::from([red.id]));
        let (selected, _, _) = render(&mut self.renderer, &doc, &camera);
        self.renderer.set_selection(&HashSet::new());
        assert!(
            selected[2] > background_bytes[2].saturating_add(30),
            "selection tint must leave the other translucent color contribution visible"
        );
        let mut orbit = camera;
        orbit.yaw += 0.18;
        let (orbited, orbit_us, _) = render(&mut self.renderer, &doc, &orbit);
        assert!(orbited[0] > background_bytes[0].saturating_add(20));
        assert!(orbited[2] > background_bytes[2].saturating_add(20));

        let mut opaque = opacity_test_stroke(
            8803,
            [0.92, 0.82, 0.18],
            [
                glam::Vec3::new(-1.2, 0.0, 0.0),
                glam::Vec3::new(1.2, 0.0, 0.0),
            ],
            glam::Vec3::X,
            0.3,
            1.0,
            0.0,
        );
        opaque.brush.dynamics.opacity.enabled = false;
        let opaque_only = Document {
            strokes: vec![opaque.clone()],
            ..Default::default()
        };
        let mut occlusion = opaque_only.clone();
        let translucent_count = doc.strokes.len();
        occlusion.strokes.extend(doc.strokes);
        let (front, opaque_us, _) = render(&mut self.renderer, &opaque_only, &camera);
        let (combined, occlusion_us, _) = render(&mut self.renderer, &occlusion, &camera);
        assert_eq!(
            front, combined,
            "opaque front surfaces occlude transparent strokes behind them"
        );
        let report = serde_json::json!({
            "passed": true,
            "checks": ["pressure opacity and soft edge alpha", "overlap is independent of document order", "selection tint preserves overlapping color", "camera orbit preserves color contributions", "hidden guides stay out of artwork render", "opaque front surface occludes translucent marks"],
            "resolution": [512, 512],
            "overlapping_translucent_strokes": translucent_count,
            "center_pixels": {"overlap": overlap, "reversed": reversed, "orbit": orbited},
            "end_to_end_readback_microseconds": {"overlap": overlap_us, "reversed": reversed_us, "orbit": orbit_us, "opaque_only": opaque_us, "opaque_occlusion": occlusion_us},
            "timing_note": "Includes CPU submission and synchronous GPU readback; this is not a GPU-only timestamp.",
            "gpu": self.renderer.adapter
        });
        std::fs::write(
            output.join("opacity-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        self.renderer.background = old_background;
        self.renderer.lighting = old_lighting;
    }
}
