use super::line_tool::Constraint;
use super::*;
use crate::geometry::Projection;
impl SketchApp {
    pub(super) fn verify_lines(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.line.verification_frame else {
            return;
        };
        if frame == 1 {
            self.request(Pending::New);
            self.guide.rotation_degrees = Vec3::new(20.0, 35.0, 15.0);
            self.camera.align_guide(self.guide);
            self.camera.projection = Projection::Orthographic;
            self.start_line_tool();
            self.line.constraint = Constraint::Horizontal;
            ctx.input_mut(|i| {
                i.events.extend([
                    egui::Event::PointerButton {
                        pos: rect.center() - egui::vec2(100.0, 0.0),
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerMoved(rect.center() + egui::vec2(20.0, 40.0)),
                ])
            });
        }
        if frame == 2 {
            assert!(self.doc.strokes.is_empty() && self.line.draft.is_some());
            ctx.input_mut(|i| {
                i.events.push(egui::Event::PointerButton {
                    pos: rect.center() + egui::vec2(140.0, 70.0),
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                })
            });
        }
        if frame == 3 {
            assert_eq!(self.doc.strokes.len(), 1);
            let line = &self.doc.strokes[0];
            let (u, v, n) = self.guide.basis();
            let delta = line.points[1].position - line.points[0].position;
            assert!(delta.dot(v).abs() < 1e-4 && delta.dot(n).abs() < 1e-4);
            assert!(
                (delta.dot(u)
                    - 240.0
                        * self
                            .camera
                            .world_per_pixel(self.guide.origin(), rect.height()))
                .abs()
                    < 0.001
            );
            assert!(line.points.iter().all(|p| p.pressure == 1.0));
            self.line.constraint = Constraint::Angle;
            self.line.step = 45.0;
            self.brush.kind = BrushKind::Ribbon;
            self.brush.dynamics.taper_end = 0.15;
            let start = rect.center() - egui::vec2(60.0, 80.0);
            let end = rect.center() + egui::vec2(90.0, -30.0);
            self.line_input(start, 0.2, Some(Vec3::Y), rect, true);
            assert!(!self.save(false));
            self.request(Pending::New);
            assert_eq!(self.doc.strokes.len(), 1);
            self.line_input(end, 0.85, Some(Vec3::Z), rect, false);
            self.release_line(end, rect);
            assert_eq!(self.doc.strokes.len(), 2);
            let points = &self.doc.strokes[1].points;
            assert_eq!(points[0].pressure, 0.2);
            assert_eq!(points[1].pressure, 0.85);
            assert_eq!(points[1].tilt, Some(Vec3::Z));
            let original = self.doc.strokes.clone();
            self.undo();
            self.redo();
            assert_eq!(self.doc.strokes, original);
            let loaded = Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
            assert_eq!(loaded.strokes, original);
            assert_eq!(
                crate::export::encode(&loaded, None)
                    .unwrap()
                    .summary
                    .strokes,
                2
            );
            self.line_input(start, 0.5, None, rect, true);
            self.cancel();
            assert!(self.line.draft.is_none());
            assert_eq!(self.doc.strokes, original);
            self.line_input(start, 0.5, None, rect, true);
            self.release_line(rect.min - egui::vec2(20.0, 20.0), rect);
            assert_eq!(self.doc.strokes, original);
            self.line_input(start, 0.5, None, rect, true);
            self.doc.layers[0].locked = true;
            self.release_line(end, rect);
            assert_eq!(self.doc.strokes, original);
            self.doc.layers[0].locked = false;
            self.use_cylinder = true;
            self.line_input(start, 0.5, None, rect, true);
            assert!(self.line.draft.is_none());
            self.use_cylinder = false;
            self.dirty = false;
            self.load_fill_study();
            self.camera.projection = Projection::Orthographic;
            self.guide.depth = 0.94;
            self.brush = Brush::default();
            self.radius = 0.012;
            self.color = [0.1, 0.17, 0.20];
            self.start_line_tool();
            for x in [-0.5, 0.5] {
                for (constraint, a, b) in [
                    (
                        Constraint::Horizontal,
                        Vec3::new(x - 0.24, 1.35, 0.94),
                        Vec3::new(x + 0.24, 1.35, 0.94),
                    ),
                    (
                        Constraint::Vertical,
                        Vec3::new(x, 1.14, 0.94),
                        Vec3::new(x, 1.56, 0.94),
                    ),
                ] {
                    self.line.constraint = constraint;
                    let (a, _) = crate::gizmo::project(&self.camera, rect, a).unwrap();
                    let (b, _) = crate::gizmo::project(&self.camera, rect, b).unwrap();
                    self.line_input(a, 1.0, None, rect, true);
                    self.line_input(b, 1.0, None, rect, false);
                    self.release_line(b, rect);
                }
            }
            self.dirty = false;
            self.recovery.discard();
            self.line.constraint = Constraint::Free;
            self.status="Line tool passed: release endpoints, plane-local constraints, pressure/tilt/taper, cancel/limits, history, native reload and export.".into();
        }
        if frame == 24 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        assert!(frame < 150, "Line screenshot timed out");
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = shot {
            let root = std::env::current_dir().unwrap().join("verification");
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                root.join("lines-preview.png"),
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            std::fs::write(root.join("lines-report.json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"checks":["mouse press drag release endpoints","rotated orthographic plane constraints","contact pressure tilt and taper capture","draft save and document guards","cancel outside release locked layers and curved guide exclusion","undo native reload and GLB export","GPU architectural line study"],"gpu":self.renderer.adapter,"physical_tablet_tested":false})).unwrap()).unwrap();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.line.verification_frame = Some(frame + 1);
    }
}
