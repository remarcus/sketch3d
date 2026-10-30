use super::*;
use crate::{editing, gizmo, scenes};

impl SketchApp {
    pub(super) fn guide_preview(&self) -> GuidePreview {
        if !self.guide_visible {
            GuidePreview::Hidden
        } else if self.use_revolved {
            GuidePreview::Revolved(self.revolved)
        } else if self.use_cylinder {
            GuidePreview::Cylinder(self.cylinder)
        } else {
            GuidePreview::Plane(self.guide)
        }
    }
    pub(super) fn active_guide_label(&self) -> &'static str {
        if self.use_revolved {
            "Revolved profile guide"
        } else if self.use_cylinder {
            "Cylinder guide"
        } else {
            self.guide.label()
        }
    }
    pub(super) fn active_guide_u(&self) -> Vec3 {
        if self.use_revolved {
            self.revolved.rotation() * Vec3::X
        } else if self.use_cylinder {
            self.cylinder.rotation() * Vec3::X
        } else {
            self.guide.basis().0
        }
    }
    pub(super) fn guide_badge(&self) -> String {
        if !self.guide_visible {
            format!("{} / hidden", self.active_guide_label())
        } else if self.use_revolved {
            format!("Revolved / {} profile points", self.revolved.count)
        } else if self.use_cylinder {
            format!(
                "Cylinder / R {:.2} / H {:.2}",
                self.cylinder.radius, self.cylinder.height
            )
        } else {
            format!("{} / {:+.2}", self.guide.label(), self.guide.depth)
        }
    }
    pub(super) fn align_active_guide(&mut self) {
        self.finish();
        if self.use_revolved {
            self.camera.distance = self
                .camera
                .distance
                .max(self.revolved.max_radius() * 3.0)
                .max((self.revolved.max_height() - self.revolved.min_height()) * 1.6);
            self.camera.align_guide(DrawingPlane {
                anchor: self.revolved.center,
                rotation_degrees: self.revolved.rotation_degrees,
                depth: 0.0,
            });
            return;
        }
        if self.use_cylinder {
            self.camera.distance = self
                .camera
                .distance
                .max(self.cylinder.radius * 3.0)
                .max(self.cylinder.height * 1.6);
            self.camera.align_guide(DrawingPlane {
                anchor: self.cylinder.center,
                rotation_degrees: self.cylinder.rotation_degrees,
                depth: 0.0,
            });
        } else {
            self.camera.align_guide(self.guide);
        }
    }
    pub(super) fn cylinder_controls(&mut self, ui: &mut egui::Ui) {
        let before = self.cylinder;
        ui.label(
            RichText::new("Open cylinder / side surface")
                .small()
                .color(Color32::from_rgb(119, 203, 195)),
        );
        ui.label("Rotation in degrees");
        ui.horizontal(|ui| {
            for (i, label) in ["X ", "Y ", "Z "].iter().enumerate() {
                ui.add(
                    egui::DragValue::new(&mut self.cylinder.rotation_degrees[i])
                        .prefix(*label)
                        .speed(0.5)
                        .range(-180.0..=180.0),
                );
            }
        });
        ui.label("Center in world units");
        ui.horizontal(|ui| {
            for (i, label) in ["X ", "Y ", "Z "].iter().enumerate() {
                ui.add(
                    egui::DragValue::new(&mut self.cylinder.center[i])
                        .prefix(*label)
                        .speed(0.02)
                        .range(-1000.0..=1000.0),
                );
            }
        });
        ui.add(
            egui::Slider::new(&mut self.cylinder.radius, 0.05..=10.0)
                .logarithmic(true)
                .text("Radius"),
        );
        ui.add(
            egui::Slider::new(&mut self.cylinder.height, 0.1..=20.0)
                .logarithmic(true)
                .text("Height"),
        );
        if self.cylinder != before {
            self.finish();
        }
        if ui
            .button("Cylinder from view")
            .on_hover_text(
                "Place the cylinder at the camera focus with its axis vertical in the current view",
            )
            .clicked()
        {
            self.finish();
            let plane = DrawingPlane::from_view(&self.camera);
            self.cylinder.center = plane.anchor;
            self.cylinder.rotation_degrees = plane.rotation_degrees;
        }
        if ui
            .add_enabled(
                !self.selected.is_empty(),
                egui::Button::new("Center on selection"),
            )
            .clicked()
        {
            self.finish();
            if let Some((min, max)) = editing::bounds(&self.doc, &self.selected) {
                self.cylinder.center = (min + max) * 0.5;
            }
        }
        if ui.button("Align view to guide [F]").clicked() {
            self.align_active_guide();
        }
        ui.label(RichText::new("Draw on the side. Orbit to reach the back. Missing the guide splits the stroke; caps are not drawing surfaces.").small());
    }
    pub(super) fn load_lantern(&mut self) {
        self.reset_recovery_document();
        self.doc = scenes::cylindrical_lantern();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.cylinder = scenes::lantern_cylinder();
        self.camera = scenes::lantern_camera();
        self.use_cylinder = true;
        self.use_revolved = false;
        self.guide_visible = true;
        self.resume_ink = false;
        self.file = None;
        self.dirty = false;
        self.active = None;
        self.drag = None;
        self.mouse_down = false;
        self.stabilizer.reset();
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.renderer.invalidate();
        self.starter_scene = false;
        self.cylinder_example = true;
        self.select_mode = false;
        self.eraser = false;
        self.color = [1.0, 0.76, 0.29];
        self.radius = 0.025;
        self.status="Cylindrical lantern / editable study. Draw around its side, orbit for the back, or hide the guide.".into();
    }
    pub(super) fn verify_cylinder(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.cylinder_verification_frame else {
            return;
        };
        match frame {
            1 => self.run_cylinder_checks(rect),
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Cylinder screenshot timed out"),
            _ => {}
        }
        self.cylinder_verification_frame = Some(frame + 1);
    }
    fn run_cylinder_checks(&mut self, rect: Rect) {
        fn check(stroke: &Stroke, cylinder: Cylinder) {
            assert!(stroke.points.len() > 5);
            for point in &stroke.points {
                let p = cylinder.to_local(point.position);
                assert!(
                    (p.x.hypot(p.z) - cylinder.radius).abs() < 0.00002,
                    "All stored ink must stay on the cylinder side"
                );
                assert!(p.y.abs() <= cylinder.height * 0.5 + 0.00002);
            }
        }
        fn draw(app: &mut SketchApp, rect: Rect, angle: f32, y: f32, begin: bool) {
            let point = app.cylinder.point(angle, y);
            let pos = gizmo::project(&app.camera, rect, point).unwrap().0;
            app.add_point(pos, 0.65, rect, begin);
        }
        let original = self.doc.strokes.clone();
        let cylinder = self.cylinder;
        self.stabilization = 0.8;
        for i in 0..80 {
            draw(
                self,
                rect,
                -0.4 + i as f32 * 0.017,
                -0.7 + i as f32 * 0.017,
                i == 0,
            );
        }
        self.finish();
        check(self.doc.strokes.last().unwrap(), cylinder);
        let finished = self.doc.strokes.clone();
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.redo();
        assert_eq!(self.doc.strokes, finished);
        self.undo();
        draw(self, rect, 0.1, -0.2, true);
        draw(self, rect, 0.3, 0.1, false);
        self.add_point(
            egui::pos2(rect.left() + 3.0, rect.center().y),
            0.65,
            rect,
            false,
        );
        assert!(self.active.is_none());
        assert!(self.resume_ink);
        draw(self, rect, 0.5, 0.3, false);
        draw(self, rect, 0.7, 0.6, false);
        self.finish();
        assert_eq!(self.doc.strokes.len(), original.len() + 2);
        self.undo();
        self.undo();
        assert_eq!(self.doc.strokes, original);
        draw(self, rect, -0.85, 0.0, true);
        draw(self, rect, 1.0, 0.1, false);
        self.finish();
        assert_eq!(
            self.doc.strokes.len(),
            original.len() + 2,
            "A large side jump must split ink rather than bridge the silhouette"
        );
        self.undo();
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.camera.yaw = std::f32::consts::PI;
        self.camera.pitch = 0.1;
        for i in 0..80 {
            draw(
                self,
                rect,
                3.0 + i as f32 * 0.004,
                (i as f32 * 0.07).sin() * 0.3,
                i == 0,
            );
        }
        self.finish();
        assert_eq!(self.doc.strokes.len(), original.len() + 1);
        let seam = self.doc.strokes.last().unwrap();
        check(seam, cylinder);
        assert!(seam
            .points
            .windows(2)
            .all(|p| p[0].position.distance(p[1].position) < 0.15));
        self.undo();
        self.cylinder = Cylinder {
            center: Vec3::new(0.3, 1.75, 0.1),
            rotation_degrees: Vec3::new(25.0, 10.0, -15.0),
            radius: 0.8,
            height: 3.0,
        };
        self.align_active_guide();
        let captured = self.cylinder;
        for i in 0..60 {
            draw(
                self,
                rect,
                -0.55 + i as f32 * 0.018,
                -0.6 + i as f32 * 0.02,
                i == 0,
            );
        }
        self.cylinder.radius = 1.3;
        self.finish();
        check(self.doc.strokes.last().unwrap(), captured);
        self.cylinder = captured;
        let ink = self.doc.strokes.clone();
        self.guide_visible = false;
        assert!(matches!(self.guide_preview(), GuidePreview::Hidden));
        assert_eq!(self.doc.strokes, ink);
        self.guide_visible = true;
        self.use_cylinder = false;
        assert_eq!(self.doc.strokes, ink);
        self.use_cylinder = true;
        self.use_revolved = false;
        let id = self.doc.strokes.last().unwrap().id;
        self.selected = HashSet::from([id]);
        self.duplicate_selected();
        assert_eq!(self.doc.strokes.len(), ink.len() + 1);
        self.delete_selected();
        assert_eq!(self.doc.strokes, ink);
        self.undo();
        self.undo();
        assert_eq!(self.doc.strokes, ink);
        let path = std::env::current_dir().unwrap().join("verification");
        std::fs::create_dir_all(&path).unwrap();
        self.file = Some(path.join("cylinder.sketch.json"));
        assert!(self.save(false));
        let loaded =
            Document::from_json(&std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap())
                .unwrap();
        assert_eq!(loaded.strokes, self.doc.strokes);
        self.load_lantern();
        self.stabilization = 0.35;
    }
}
