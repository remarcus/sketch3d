use super::ui_style;
use super::*;
use crate::{document::PressureMapping, export_check};

fn airbrush_stroke(
    id: u64,
    color: [f32; 3],
    radius: f32,
    rate_per_second: f32,
    opacity: f32,
    points: impl IntoIterator<Item = (u32, glam::Vec3, f32)>,
) -> Stroke {
    Stroke {
        model_surface: None,
        curve: None,
        material: Default::default(),
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        fill: None,
        group_id: None,
        layer_id: 1,
        id,
        color,
        gradient: None,
        radius,
        brush: Brush {
            opacity,
            softness: 0.8,
            flow: crate::document::AirbrushSettings {
                enabled: true,
                rate_per_second,
                algorithm_version: 1,
            },
            ..Brush::default()
        },
        points: points
            .into_iter()
            .map(|(time_ms, position, pressure)| Point {
                position,
                pressure,
                tilt: None,
                time_ms,
            })
            .collect(),
    }
}

fn timed_line_points(step_ms: u32) -> Vec<(u32, glam::Vec3, f32)> {
    (0..=256)
        .step_by(step_ms as usize)
        .map(|time_ms| {
            let t = time_ms as f32 / 256.0;
            (
                time_ms,
                glam::Vec3::new(-1.0 + 2.0 * t, 0.15 * t, 0.0),
                0.2 + 0.7 * t,
            )
        })
        .collect()
}

fn canvas_flow_sample(
    position: Pos2,
    pixels_per_point: f32,
    phase: Phase,
    contact: bool,
    pressure: Option<f32>,
    barrel: bool,
) -> PenSample {
    PenSample {
        position: position * pixels_per_point,
        pressure,
        tilt: [None, None],
        phase,
        contact,
        eraser: false,
        barrel,
    }
}

fn apply_synthetic_pointer_samples(app: &mut SketchApp, rect: Rect, samples: Vec<PenSample>) {
    for sample in samples {
        match sample.phase {
            Phase::Cancel => app.cancel(),
            Phase::Up => app.finish(),
            phase if sample.contact => {
                let tilt = crate::dynamics::screen_tilt(
                    sample.tilt,
                    app.camera.basis().0,
                    app.camera.basis().1,
                );
                app.add_point_with_tilt(
                    sample.position,
                    sample.pressure.unwrap_or(1.0),
                    rect,
                    phase == Phase::Down,
                    tilt,
                );
            }
            _ => {}
        }
    }
}

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
                self.brush.dabs.enabled = true;
                self.brush.dabs.spacing = 0.5;
                self.brush.dabs.scatter = 0.0;
                self.radius = 0.1;
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
                let start = rect.center() - egui::vec2(130.0, 0.0);
                let end = start + egui::vec2(260.0, 40.0);
                let sample = |position: Pos2, pressure, tilt, phase, contact| PenSample {
                    position,
                    pressure: Some(pressure),
                    tilt,
                    phase,
                    contact,
                    eraser: false,
                    barrel: false,
                };
                apply_synthetic_pointer_samples(
                    self,
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
                let stroke_length = crate::dynamics::arc_lengths(&stroke.points)
                    .last()
                    .copied()
                    .unwrap_or(0.0);
                let dab_spacing = stroke.radius * stroke.brush.dabs.spacing;
                let resolved_dabs = crate::dab_placement::sample(&stroke).unwrap();
                assert!(resolved_dabs.len() > 1);
                assert!(resolved_dabs.iter().all(|dab| {
                    let distance = dab.gradient_t * stroke_length;
                    (distance / dab_spacing - (distance / dab_spacing).round()).abs() < 1.0e-4
                }));
                let final_dab_distance = resolved_dabs.last().unwrap().gradient_t * stroke_length;
                assert!(stroke_length - final_dab_distance < dab_spacing + 1.0e-5);
                assert_eq!(self.undo.len(), 1);
                self.undo();
                assert!(self.doc.strokes.is_empty());
                self.redo();
                assert_eq!(self.doc.strokes[0], stroke);
                // A canceled second stroke contributes neither geometry nor an undo item.
                apply_synthetic_pointer_samples(
                    self,
                    rect,
                    vec![
                        sample(start, 0.6, [Some(20.0), Some(20.0)], Phase::Down, true),
                        sample(end, 0.0, [None, None], Phase::Cancel, false),
                    ],
                );
                assert_eq!(self.doc.strokes, vec![stroke.clone()]);
                assert_eq!(self.undo.len(), 1);
                apply_synthetic_pointer_samples(
                    self,
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
                assert_eq!(loaded.version, 30);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.load_dynamics();
                self.brush.dynamics = Default::default();
                self.brush.dynamics.tilt_orientation = true;
                self.brush.dynamics.taper_start = 0.3;
                self.brush.dynamics.taper_end = 0.5;
            }
            2 => self.verify_airbrush_flow(ctx, rect),
            3 => {
                self.verify_opacity_compositing();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            100 => panic!("Dynamics screenshot timed out"),
            _ => {}
        }
        self.dynamics_verification_frame = Some(frame + 1);
    }

    fn verify_airbrush_flow(&mut self, ctx: &egui::Context, rect: Rect) {
        let saved_doc = std::mem::take(&mut self.doc);
        let saved_brush = self.brush;
        let saved_radius = self.radius;
        let saved_color = self.color;
        let saved_stabilization = self.stabilization;
        let saved_active = self.active;
        let saved_started_at = self.active_started_at;
        let saved_next_id = self.next_id;
        let saved_undo = std::mem::take(&mut self.undo);
        let saved_redo = std::mem::take(&mut self.redo);
        let saved_dirty = self.dirty;
        let saved_mouse_down = self.mouse_down;
        let saved_pen_previous = self.pen_previous;
        let saved_resume_ink = self.resume_ink;
        let saved_model_contact = self.model_stroke_contact;
        let saved_verification_frame = self.dynamics_verification_frame;
        self.show_help = false;
        self.tutorial.offer = false;
        self.recovery_prompt = false;
        self.close_after_recovery = false;

        let coarse_mesh = crate::geometry::stroke_mesh(&airbrush_stroke(
            7701,
            [0.2, 0.7, 0.95],
            0.16,
            3.7,
            0.84,
            timed_line_points(64),
        ));
        let medium_mesh = crate::geometry::stroke_mesh(&airbrush_stroke(
            7702,
            [0.2, 0.7, 0.95],
            0.16,
            3.7,
            0.84,
            timed_line_points(32),
        ));
        let fine_mesh = crate::geometry::stroke_mesh(&airbrush_stroke(
            7703,
            [0.2, 0.7, 0.95],
            0.16,
            3.7,
            0.84,
            timed_line_points(8),
        ));
        assert_eq!(coarse_mesh.len(), medium_mesh.len());
        assert_eq!(coarse_mesh.len(), fine_mesh.len());
        let coarse_discs = coarse_mesh.as_chunks::<72>().0;
        let medium_discs = medium_mesh.as_chunks::<72>().0;
        let fine_discs = fine_mesh.as_chunks::<72>().0;
        let mut max_sampling_position_error = 0.0_f32;
        let mut max_sampling_alpha_error = 0.0_f32;
        for ((coarse_disc, medium_disc), fine_disc) in
            coarse_discs.iter().zip(medium_discs).zip(fine_discs)
        {
            // Compare the center vertex for each triangle-fan wedge. The
            // captured source-point subdivision can change the disc frame's
            // interpolated tangent slightly; center positions and deposited
            // opacity measure temporal flow invariance without conflating it
            // with the eight-sided coverage tessellation.
            for wedge in (0..72).step_by(9) {
                for (a, b) in [
                    (coarse_disc[wedge].position, medium_disc[wedge].position),
                    (coarse_disc[wedge].position, fine_disc[wedge].position),
                ] {
                    max_sampling_position_error = max_sampling_position_error
                        .max(glam::Vec3::from(a).distance(glam::Vec3::from(b)));
                }
            }
            for index in 0..72 {
                max_sampling_alpha_error = max_sampling_alpha_error
                    .max((coarse_disc[index].opacity - medium_disc[index].opacity).abs())
                    .max((coarse_disc[index].opacity - fine_disc[index].opacity).abs());
            }
        }
        assert!(
            max_sampling_position_error < 1e-3,
            "sample-rate geometry error is {max_sampling_position_error} world units"
        );
        assert!(
            max_sampling_alpha_error < 1e-3,
            "sample-rate alpha error is {max_sampling_alpha_error}"
        );

        let stationary_samples = |step_ms: u32| {
            (0..=1024)
                .step_by(step_ms as usize)
                .map(|time_ms| (time_ms, glam::Vec3::ZERO, 0.85))
                .collect::<Vec<_>>()
        };
        let repaint_30hz = crate::geometry::stroke_mesh(&airbrush_stroke(
            7710,
            [0.9, 0.35, 0.12],
            0.2,
            3.5,
            0.82,
            stationary_samples(32),
        ));
        let repaint_8hz = crate::geometry::stroke_mesh(&airbrush_stroke(
            7711,
            [0.9, 0.35, 0.12],
            0.2,
            3.5,
            0.82,
            stationary_samples(128),
        ));
        assert_eq!(repaint_30hz.len(), 32 * 72);
        assert_eq!(repaint_30hz.len(), repaint_8hz.len());
        for (a, b) in repaint_30hz.iter().zip(&repaint_8hz) {
            assert!(glam::Vec3::from(a.position).distance(glam::Vec3::from(b.position)) < 1e-6);
            assert!((a.opacity - b.opacity).abs() < 1e-6);
        }

        let mut stationary_coverage = Vec::new();
        for duration_ms in [32, 64, 128, 256, 512, 1024] {
            let mesh = crate::geometry::stroke_mesh(&airbrush_stroke(
                7720 + u64::from(duration_ms),
                [0.9, 0.35, 0.12],
                0.2,
                3.5,
                0.82,
                [
                    (0, glam::Vec3::ZERO, 0.85),
                    (duration_ms, glam::Vec3::ZERO, 0.85),
                ],
            ));
            let deposit_count = (duration_ms / crate::geometry::AIRBRUSH_TICK_MS) as usize;
            assert_eq!(mesh.len(), deposit_count * 72);
            let center_alpha = mesh
                .iter()
                .find(|vertex| glam::Vec3::from(vertex.position).length_squared() < 1e-10)
                .expect("each stationary deposit has a center vertex")
                .opacity;
            let accumulated = 1.0 - (1.0 - center_alpha).powi(deposit_count as i32);
            assert!((0.0..=1.0).contains(&accumulated));
            stationary_coverage.push(accumulated);
        }
        assert!(stationary_coverage.windows(2).all(|pair| pair[1] > pair[0]));
        assert!(stationary_coverage.last().unwrap() > &0.8);

        let large_radius = airbrush_stroke(
            7730,
            [0.3, 0.85, 0.35],
            0.8,
            3.0,
            0.7,
            (0..=10_000)
                .step_by(64)
                .map(|time_ms| {
                    (
                        time_ms,
                        glam::Vec3::new(time_ms as f32 / 1000.0, 0.0, 0.0),
                        0.9,
                    )
                })
                .chain(std::iter::once((10_000, glam::Vec3::X * 10.0, 0.9))),
        );
        let large_radius_started = Instant::now();
        let large_radius_mesh = crate::geometry::stroke_mesh(&large_radius);
        let large_radius_elapsed_us = large_radius_started.elapsed().as_micros();
        let large_radius_estimate = crate::geometry::airbrush_vertex_estimate(&large_radius);
        assert_eq!(large_radius_mesh.len(), large_radius_estimate);
        assert!(large_radius_estimate <= crate::geometry::MAX_AIRBRUSH_VERTICES);

        let stationary_limit = crate::document::MAX_FLOW_DURATION_MS
            .min(crate::geometry::MAX_AIRBRUSH_DEPOSITS as u32 * crate::geometry::AIRBRUSH_TICK_MS);
        let at_budget = airbrush_stroke(
            7740,
            [0.4, 0.55, 0.95],
            0.1,
            3.0,
            0.8,
            [
                (0, glam::Vec3::ZERO, 1.0),
                (stationary_limit, glam::Vec3::ZERO, 1.0),
            ],
        );
        assert_eq!(
            crate::geometry::airbrush_vertex_estimate(&at_budget),
            crate::geometry::MAX_AIRBRUSH_VERTICES
        );
        let over_budget = airbrush_stroke(
            7741,
            [0.4, 0.55, 0.95],
            0.1,
            3.0,
            0.8,
            [
                (0, glam::Vec3::ZERO, 1.0),
                (
                    stationary_limit + crate::geometry::AIRBRUSH_TICK_MS,
                    glam::Vec3::ZERO,
                    1.0,
                ),
            ],
        );
        assert!(
            crate::geometry::airbrush_vertex_estimate(&over_budget)
                > crate::geometry::MAX_AIRBRUSH_VERTICES
        );
        let over_point_budget = airbrush_stroke(
            7742,
            [0.4, 0.55, 0.95],
            0.1,
            3.0,
            0.8,
            (0..=crate::geometry::MAX_AIRBRUSH_POINTS).map(|index| {
                (
                    (index as u32).min(crate::document::MAX_FLOW_DURATION_MS),
                    glam::Vec3::ZERO,
                    1.0,
                )
            }),
        );
        assert!(
            crate::geometry::airbrush_vertex_estimate(&over_point_budget)
                > crate::geometry::MAX_AIRBRUSH_VERTICES
        );

        let horizontal = airbrush_stroke(
            7801,
            [0.96, 0.18, 0.08],
            0.24,
            3.5,
            0.82,
            [
                (0, glam::Vec3::new(-0.9, 0.0, 0.0), 0.85),
                (1024, glam::Vec3::new(0.9, 0.0, 0.0), 0.85),
            ],
        );
        let vertical = airbrush_stroke(
            7802,
            [0.08, 0.35, 0.98],
            0.24,
            3.5,
            0.78,
            [
                (0, glam::Vec3::new(0.0, -0.9, 0.05), 0.8),
                (1024, glam::Vec3::new(0.0, 0.9, 0.05), 0.8),
            ],
        );
        let flow_doc = Document {
            version: 24,
            strokes: vec![horizontal, vertical],
            ..Document::default()
        };
        let pick_vertices = crate::geometry::stroke_mesh(&flow_doc.strokes[0]);
        let pick_face = &pick_vertices[..3];
        let pick_position = pick_face
            .iter()
            .map(|vertex| glam::Vec3::from(vertex.position))
            .sum::<glam::Vec3>()
            / 3.0;
        let pick_normal = (glam::Vec3::from(pick_face[1].position)
            - glam::Vec3::from(pick_face[0].position))
        .cross(glam::Vec3::from(pick_face[2].position) - glam::Vec3::from(pick_face[0].position))
        .normalize();
        assert!(
            crate::placement::pick(&flow_doc, pick_position + pick_normal * 3.0, -pick_normal)
                .is_some(),
            "an opaque Airbrush deposit remains pickable"
        );
        let transparent_miss = Document {
            version: 24,
            strokes: vec![airbrush_stroke(
                7803,
                [0.96, 0.18, 0.08],
                0.24,
                0.05,
                0.82,
                [(0, glam::Vec3::ZERO, 0.001), (128, glam::Vec3::ZERO, 0.001)],
            )],
            ..Document::default()
        };
        assert!(crate::placement::pick(
            &transparent_miss,
            glam::Vec3::new(0.0, 0.0, 3.0),
            -glam::Vec3::Z
        )
        .is_none());

        let native_bytes = serde_json::to_vec_pretty(&flow_doc).unwrap();
        let loaded = Document::from_json(std::str::from_utf8(&native_bytes).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(&flow_doc).unwrap()
        );
        let encoded = crate::export::encode(&flow_doc, None).unwrap();
        let reloaded_encoded = crate::export::encode(&loaded, None).unwrap();
        assert_eq!(encoded.bytes, reloaded_encoded.bytes);
        assert!(encoded.summary.vertices <= 2 * crate::geometry::MAX_AIRBRUSH_VERTICES);
        let json_len = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap()) as usize;
        let glb_json: serde_json::Value =
            serde_json::from_slice(&encoded.bytes[20..20 + json_len]).unwrap();
        assert!(glb_json["materials"]
            .as_array()
            .unwrap()
            .iter()
            .any(|material| material["alphaMode"] == "BLEND"));
        let glb_position_vertices = glb_json["meshes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
            .map(|primitive| {
                let accessor = primitive["attributes"]["POSITION"].as_u64().unwrap() as usize;
                glb_json["accessors"][accessor]["count"].as_u64().unwrap() as usize
            })
            .sum::<usize>();
        assert_eq!(glb_position_vertices, encoded.summary.vertices);

        let run_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let output = std::env::current_dir()
            .unwrap()
            .join("verification/airbrush-flow")
            .join(format!("run-{run_id}"));
        std::fs::create_dir_all(&output).unwrap();
        let native_path = output.join("flow.sketch.json");
        std::fs::write(&native_path, &native_bytes).unwrap();
        let glb_path = output.join("flow.glb");
        std::fs::write(&glb_path, &encoded.bytes).unwrap();
        let obj_path = output.join("flow.obj");
        let obj_bundle_bytes =
            crate::obj_export::write_obj_bundle(&loaded, None, &obj_path, false).unwrap();
        let obj_text = std::fs::read_to_string(&obj_path).unwrap();
        let obj_vertex_count = obj_text
            .lines()
            .filter(|line| line.starts_with("v "))
            .count();
        let obj_face_count = obj_text
            .lines()
            .filter(|line| line.starts_with("f "))
            .count();
        assert_eq!(obj_vertex_count, encoded.summary.vertices);
        assert!(obj_face_count > 0);
        let mtl_name = obj_text
            .lines()
            .find_map(|line| line.strip_prefix("mtllib "))
            .expect("OBJ names its MTL companion");
        let mtl_text = std::fs::read_to_string(output.join(mtl_name)).unwrap();
        assert!(mtl_text.contains(
            "Airbrush timing and per-vertex soft opacity are baked into mesh placement only approximately"
        ));

        let flow_camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            target: glam::Vec3::ZERO,
            distance: 5.0,
            ..Camera::default()
        };
        let flow_orbit = Camera {
            yaw: 0.72,
            pitch: 0.38,
            ..flow_camera
        };
        let render = |renderer: &mut SceneRenderer, doc: &Document, camera: &Camera| {
            renderer.invalidate();
            renderer.render(doc, camera, GuidePreview::Hidden, [640, 360]);
            renderer.verification_pixels()
        };
        let front_pixels = render(&mut self.renderer, &flow_doc, &flow_camera);
        let orbit_pixels = render(&mut self.renderer, &flow_doc, &flow_orbit);
        let blank_pixels = render(&mut self.renderer, &Document::default(), &flow_camera);
        let changed_pixels = |pixels: &[u8]| {
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(blank_pixels.as_chunks::<4>().0)
                .filter(|(pixel, blank)| {
                    pixel[..3]
                        .iter()
                        .zip(&blank[..3])
                        .map(|(a, b)| a.abs_diff(*b))
                        .sum::<u8>()
                        > 8
                })
                .count()
        };
        let front_ink_pixels = changed_pixels(&front_pixels);
        let orbit_ink_pixels = changed_pixels(&orbit_pixels);
        assert!(
            front_ink_pixels > 20,
            "front render contains airbrush marks"
        );
        assert!(
            orbit_ink_pixels > 20,
            "orbit render contains airbrush marks"
        );
        image::save_buffer(
            output.join("airbrush-front.png"),
            &front_pixels,
            640,
            360,
            image::ColorType::Rgba8,
        )
        .unwrap();
        image::save_buffer(
            output.join("airbrush-orbit.png"),
            &orbit_pixels,
            640,
            360,
            image::ColorType::Rgba8,
        )
        .unwrap();

        let flow_brush = Brush {
            opacity: 0.82,
            softness: 0.8,
            flow: crate::document::AirbrushSettings {
                enabled: true,
                rate_per_second: 3.5,
                algorithm_version: 1,
            },
            ..Brush::default()
        };
        let prepare_contact = |app: &mut Self| {
            app.doc = Document::default();
            app.active = None;
            app.active_started_at = None;
            app.next_id = 1;
            app.undo.clear();
            app.redo.clear();
            app.mouse_down = false;
            app.pen_previous = None;
            app.resume_ink = false;
            app.model_stroke_contact = None;
            app.stabilizer.reset();
            app.brush = flow_brush;
            app.radius = 0.12;
            app.color = [0.8, 0.4, 0.2];
            app.stabilization = 0.0;
            app.dynamics_verification_frame = Some(2);
        };
        let start_contact = |app: &mut Self, pressure: Option<f32>| {
            prepare_contact(app);
            let scale = ctx.pixels_per_point();
            let contact_position = (1..=9)
                .flat_map(|row| {
                    (1..=9).map(move |column| {
                        egui::pos2(
                            rect.left() + rect.width() * column as f32 / 10.0,
                            rect.top() + rect.height() * row as f32 / 10.0,
                        )
                    })
                })
                .find(|position| super::canvas_pointer_allowed(ctx, rect, *position))
                .unwrap_or_else(|| panic!("no unobstructed canvas point in {rect:?}"));
            app.canvas_input(
                ctx,
                rect,
                vec![canvas_flow_sample(
                    contact_position,
                    scale,
                    Phase::Down,
                    true,
                    pressure,
                    false,
                )],
            );
            assert!(app.active.is_some(), "canvas down starts a flow contact");
            assert_eq!(app.doc.strokes.len(), 1);
        };

        start_contact(self, None);
        let fallback_pressure = self.doc.strokes[0].points[0].pressure;
        assert_eq!(fallback_pressure, 1.0, "missing pointer pressure uses 1.0");
        self.active_started_at = Some(Instant::now() - Duration::from_millis(128));
        self.tick_airbrush_contact(ctx);
        assert_eq!(self.doc.strokes[0].points.len(), 2);
        assert!(self.doc.strokes[0].points[1].time_ms >= crate::geometry::AIRBRUSH_TICK_MS);
        assert_eq!(
            self.doc.strokes[0].points[0].position, self.doc.strokes[0].points[1].position,
            "stationary timer records a same-position timestamped sample"
        );
        let stationary_mesh = crate::geometry::stroke_mesh(&self.doc.strokes[0]);
        assert!(!stationary_mesh.is_empty());
        let stationary_tick_point_count = self.doc.strokes[0].points.len();
        let scale = ctx.pixels_per_point();
        self.canvas_input(
            ctx,
            rect,
            vec![canvas_flow_sample(
                rect.center() + egui::vec2(rect.width() * 2.0, 0.0),
                scale,
                Phase::Move,
                false,
                None,
                false,
            )],
        );
        assert!(self.active.is_none(), "hover ends the active flow contact");
        let stopped_points = self.doc.strokes.last().unwrap().points.clone();
        self.active_started_at = Some(Instant::now() - Duration::from_secs(2));
        self.tick_airbrush_contact(ctx);
        assert_eq!(self.doc.strokes.last().unwrap().points, stopped_points);
        assert_eq!(stopped_points.len(), stationary_tick_point_count);

        start_contact(self, Some(0.75));
        self.canvas_input(
            ctx,
            rect,
            vec![canvas_flow_sample(
                rect.center(),
                scale,
                Phase::Up,
                false,
                None,
                false,
            )],
        );
        assert!(self.active.is_none(), "pen-up ends the active flow contact");
        assert_eq!(self.doc.strokes.len(), 1);
        let up_points = self.doc.strokes[0].points.clone();
        self.active_started_at = Some(Instant::now() - Duration::from_secs(2));
        self.tick_airbrush_contact(ctx);
        assert_eq!(self.doc.strokes[0].points, up_points);

        start_contact(self, Some(0.75));
        self.canvas_input(
            ctx,
            rect,
            vec![canvas_flow_sample(
                rect.center(),
                scale,
                Phase::Cancel,
                false,
                None,
                false,
            )],
        );
        assert!(
            self.active.is_none(),
            "cancel stops the active flow contact"
        );
        assert!(
            self.doc.strokes.is_empty(),
            "cancel removes the uncommitted flow stroke"
        );
        self.active_started_at = Some(Instant::now() - Duration::from_secs(2));
        self.tick_airbrush_contact(ctx);
        assert!(self.doc.strokes.is_empty());

        start_contact(self, Some(0.75));
        self.canvas_input(
            ctx,
            rect,
            vec![canvas_flow_sample(
                rect.center() + egui::vec2(18.0, 0.0),
                scale,
                Phase::Move,
                true,
                Some(0.75),
                true,
            )],
        );
        assert!(
            self.active.is_none(),
            "navigation stops the active flow contact"
        );
        assert_eq!(self.doc.strokes.len(), 1);

        start_contact(self, Some(0.75));
        let prior_verification_frame = self.dynamics_verification_frame.take();
        ctx.input_mut(|input| input.focused = false);
        self.canvas_input(ctx, rect, Vec::new());
        ctx.input_mut(|input| input.focused = true);
        self.dynamics_verification_frame = prior_verification_frame;
        assert!(
            self.active.is_none(),
            "focus loss stops the active flow contact"
        );
        assert!(
            self.doc.strokes.is_empty(),
            "focus loss cancels the uncommitted flow stroke"
        );

        let gpu = self.renderer.adapter.clone();
        let report = serde_json::json!({
            "passed": true,
            "checks": [
                "same timed line at 15 Hz, 30 Hz, and 125 Hz input samples produces the same density and geometry",
                "stationary 30 Hz and 8 Hz timestamp recordings produce identical 32 ms deposits",
                "stationary center coverage rises smoothly and remains at or below full opacity",
                "stationary contact timer records timestamped same-position samples",
                "canvas hover, pen-up, barrel navigation, cancel, and focus loss stop flow emission",
                "missing pen pressure follows the 1.0 mouse fallback through canvas input",
                "65,536 point, 8,192 deposit, and 589,824 vertex limits are enforced",
                "opacity-aware picking hits visible flow and misses near-transparent flow",
                "native roundtrip preserves flow rate, algorithm version, pressure, and timestamps",
                "GLB retains blended alpha and roundtrips to byte-identical output within vertex budget",
                "OBJ mesh is budgeted and MTL records its per-vertex opacity approximation limit",
                "overlapping transparent flow marks render from front and orbit cameras"
            ],
            "metrics": {
                "moving_vertices": coarse_mesh.len(),
                "moving_max_position_error": max_sampling_position_error,
                "moving_max_alpha_error": max_sampling_alpha_error,
                "stationary_deposits_at_1024ms": repaint_30hz.len() / 72,
                "stationary_coverage_at_1024ms": stationary_coverage.last().copied().unwrap_or(0.0),
                "large_radius_vertices": large_radius_estimate,
                "large_radius_mesh_us": large_radius_elapsed_us,
                "native_bytes": native_bytes.len(),
                "glb_bytes": encoded.bytes.len(),
                "glb_vertices": encoded.summary.vertices,
                "obj_bundle_bytes": obj_bundle_bytes,
                "obj_vertices": obj_vertex_count,
                "obj_triangles": obj_face_count,
                "front_ink_pixels": front_ink_pixels,
                "orbit_ink_pixels": orbit_ink_pixels,
                "missing_pressure_fallback": fallback_pressure,
                "stationary_live_points_before_hover": stationary_tick_point_count
            },
            "artifacts": {
                "native": native_path.file_name().unwrap().to_string_lossy(),
                "glb": glb_path.file_name().unwrap().to_string_lossy(),
                "obj": obj_path.file_name().unwrap().to_string_lossy(),
                "front_png": "airbrush-front.png",
                "orbit_png": "airbrush-orbit.png",
            },
            "gpu": gpu,
            "physical_tablet_tested": false
        });
        std::fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();

        self.doc = saved_doc;
        self.brush = saved_brush;
        self.radius = saved_radius;
        self.color = saved_color;
        self.stabilization = saved_stabilization;
        self.active = saved_active;
        self.active_started_at = saved_started_at;
        self.next_id = saved_next_id;
        self.undo = saved_undo;
        self.redo = saved_redo;
        self.dirty = saved_dirty;
        self.mouse_down = saved_mouse_down;
        self.pen_previous = saved_pen_previous;
        self.resume_ink = saved_resume_ink;
        self.model_stroke_contact = saved_model_contact;
        self.dynamics_verification_frame = saved_verification_frame;
        self.renderer.invalidate();
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
        model_surface: None,
        curve: None,
        material: Default::default(),
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        fill: None,
        group_id: None,
        layer_id: 1,
        id,
        color,
        gradient: None,
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
            linked_stroke_groups: Default::default(),
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
            linked_stroke_groups: Default::default(),
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
