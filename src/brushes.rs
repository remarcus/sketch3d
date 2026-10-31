use super::*;
use crate::{editing, export_check, geometry, gizmo};

impl SketchApp {
    pub(super) fn brush_controls(&mut self, ui: &mut egui::Ui) {
        ui.label("BRUSH SHAPE");
        let before = self.brush;
        ui.horizontal_wrapped(|ui| {
            for (kind, label) in [
                (BrushKind::Round, "Round"),
                (BrushKind::Ribbon, "Ribbon"),
                (BrushKind::Chisel, "Flat / chisel"),
            ] {
                ui.selectable_value(&mut self.brush.kind, kind, label);
            }
        });
        if self.brush.kind != BrushKind::Round {
            ui.add(
                egui::Slider::new(&mut self.brush.angle_degrees, -180.0..=180.0).text("Angle °"),
            );
            if self.brush.kind == BrushKind::Chisel {
                ui.add(
                    egui::Slider::new(&mut self.brush.thickness, 0.02..=1.0)
                        .text("Thickness")
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                )
                .on_hover_text("Thickness as a percentage of brush width.");
            }
            ui.small("Orientation starts from the guide, then follows the stroke. Orbit to view it from either side.");
        }
        self.texture_controls(ui);
        self.dynamics_controls(ui);
        if self.brush != before {
            self.finish();
        }
    }

    pub(super) fn load_brushes(&mut self) {
        self.reset_recovery_document();
        self.cancel();
        self.doc = export_check::brush_fixture();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.camera = Camera::default();
        self.camera.distance = 8.5;
        self.guide = DrawingPlane::default();
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = false;
        self.starter_scene = false;
        self.cylinder_example = false;
        self.file = None;
        self.dirty = false;
        self.select_mode = false;
        self.eraser = false;
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.renderer.invalidate();
        self.status = "Brush sampler / top: Round, middle: Ribbon, bottom: Flat/chisel. Editable pressure curves and dots.".into();
    }

    pub(super) fn verify_brushes(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.brush_verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.dirty = false;
                self.request(Pending::New);
                self.stabilization = 0.0;
                self.camera.align_guide(self.guide);
                for (i, kind) in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel]
                    .into_iter()
                    .enumerate()
                {
                    self.brush.kind = kind;
                    self.brush.angle_degrees = 25.0;
                    self.radius = 0.1;
                    let start = rect.center() + egui::vec2(-120.0, (i as f32 - 1.0) * 80.0);
                    self.add_point(start, 0.15, rect, true);
                    self.add_point(start + egui::vec2(120.0, -30.0), 1.0, rect, false);
                    self.add_point(start + egui::vec2(240.0, 0.0), 0.4, rect, false);
                    self.finish();
                    let stroke = self.doc.strokes.last().unwrap().clone();
                    assert_eq!(stroke.brush.kind, kind);
                    assert!(stroke.brush.normal.abs_diff_eq(self.guide.basis().2, 1e-6));
                    let midpoint = stroke.points[1].position;
                    let screen = gizmo::project(&self.camera, rect, midpoint).unwrap().0;
                    assert_eq!(
                        gizmo::pick(&self.doc, &self.camera, rect, screen),
                        Some(stroke.id)
                    );
                    self.erase(screen, rect);
                    assert!(!self.doc.strokes.iter().any(|s| s.id == stroke.id));
                    self.undo();
                    assert_eq!(self.doc.strokes.last().unwrap(), &stroke);
                    self.selected = HashSet::from([stroke.id]);
                    self.duplicate_selected();
                    assert_eq!(self.doc.strokes.last().unwrap().brush, stroke.brush);
                    self.undo();
                    assert_eq!(self.doc.strokes.last().unwrap(), &stroke);
                    self.redo();
                    self.undo();
                    self.selected = HashSet::from([stroke.id]);
                    let before = editing::snapshot(&self.doc, &self.selected);
                    let after = editing::transformed(
                        &before,
                        Vec3::ZERO,
                        Vec3::ZERO,
                        glam::Quat::from_rotation_x(0.5),
                        1.4,
                    )
                    .unwrap();
                    let edit = Edit::Change {
                        before: before.clone(),
                        after,
                    };
                    edit.apply(&mut self.doc, true);
                    assert!(
                        (self.doc.strokes.last().unwrap().radius - stroke.radius * 1.4).abs()
                            < 1e-6
                    );
                    assert_eq!(
                        self.doc.strokes.last().unwrap().brush.thickness,
                        stroke.brush.thickness
                    );
                    edit.apply(&mut self.doc, false);
                    assert_eq!(editing::snapshot(&self.doc, &self.selected), before);
                }
                let mesh = geometry::stroke_mesh(&self.doc.strokes[1]);
                self.camera.yaw += 1.0;
                assert_eq!(
                    mesh.iter().map(|v| v.position).collect::<Vec<_>>(),
                    geometry::stroke_mesh(&self.doc.strokes[1])
                        .iter()
                        .map(|v| v.position)
                        .collect::<Vec<_>>()
                );
                // Exercise captured cylinder frames and the stabilized release tail.
                self.use_cylinder = true;
                self.cylinder = Cylinder {
                    center: Vec3::ZERO,
                    ..Cylinder::default()
                };
                self.camera = Camera {
                    yaw: 0.0,
                    pitch: 0.0,
                    ..Camera::default()
                };
                self.stabilization = 0.8;
                for kind in [BrushKind::Ribbon, BrushKind::Chisel] {
                    self.brush.kind = kind;
                    for (i, angle) in [-0.4, 0.0, 0.4].into_iter().enumerate() {
                        let world = self.cylinder.point(angle, 0.3 * i as f32);
                        let screen = gizmo::project(&self.camera, rect, world).unwrap().0;
                        self.add_point(screen, 0.3 + i as f32 * 0.3, rect, i == 0);
                    }
                    self.finish();
                    let stroke = self.doc.strokes.last().unwrap();
                    assert!(stroke.brush.is_valid());
                    assert!(stroke.points.iter().all(|p| {
                        let local = self.cylinder.to_local(p.position);
                        ((local.x * local.x + local.z * local.z).sqrt() - self.cylinder.radius)
                            .abs()
                            < 1e-4
                    }));
                    assert!(!geometry::stroke_mesh(stroke).is_empty());
                }
                // Changing the current guide cannot rotate existing brush frames.
                let original = self.doc.strokes.clone();
                self.guide = DrawingPlane::from_preset(Plane::Side);
                assert_eq!(self.doc.strokes, original);
                let path = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&path).unwrap();
                self.file = Some(path.join("brushes.sketch.json"));
                assert!(self.save(false));
                let loaded = Document::from_json(
                    &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(loaded.version, 7);
                assert_eq!(loaded.strokes, original);
                self.load_brushes();
                self.brush = Brush {
                    kind: BrushKind::Chisel,
                    angle_degrees: 35.0,
                    ..Brush::default()
                };
                self.radius = 0.1;
                self.stabilization = 0.35;
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Brush screenshot timed out"),
            _ => {}
        }
        self.brush_verification_frame = Some(frame + 1);
    }
}
