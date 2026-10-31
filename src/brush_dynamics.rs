use super::*;
use crate::{document::PressureMapping, export_check};

fn mapping_controls(ui: &mut egui::Ui, label: &str, mapping: &mut PressureMapping) {
    ui.checkbox(&mut mapping.enabled, label);
    if mapping.enabled {
        ui.add(
            egui::Slider::new(&mut mapping.minimum, 0.0..=1.0)
                .text("Minimum")
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        );
        ui.add(egui::Slider::new(&mut mapping.curve, 0.3..=3.0).text("Response"));
    }
}

impl SketchApp {
    pub(super) fn dynamics_controls(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Pressure, tilt & taper").default_open(false).show(ui, |ui| {
            mapping_controls(ui, "Pressure: width", &mut self.brush.dynamics.width);
            if self.brush.kind == BrushKind::Chisel {
                mapping_controls(ui, "Pressure: thickness", &mut self.brush.dynamics.thickness);
            }
            if self.brush.kind != BrushKind::Round {
                ui.checkbox(&mut self.brush.dynamics.tilt_orientation, "Tilt orientation").on_hover_text("Uses reported pen lean to orient the cross-section. Missing or upright tilt falls back to the manual frame.");
            }
            ui.add(egui::Slider::new(&mut self.brush.dynamics.taper_start, 0.0..=10.0).text("Start taper"));
            ui.add(egui::Slider::new(&mut self.brush.dynamics.taper_end, 0.0..=10.0).text("End taper"));
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
                self.brush.dynamics.width.minimum = 0.05;
                self.brush.dynamics.width.curve = 1.8;
                self.brush.dynamics.thickness.enabled = false;
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
                assert_eq!(loaded.version, 7);
                assert_eq!(loaded.strokes, self.doc.strokes);
                self.load_dynamics();
                self.brush.dynamics = Default::default();
                self.brush.dynamics.tilt_orientation = true;
                self.brush.dynamics.taper_start = 0.3;
                self.brush.dynamics.taper_end = 0.5;
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Dynamics screenshot timed out"),
            _ => {}
        }
        self.dynamics_verification_frame = Some(frame + 1);
    }
}
