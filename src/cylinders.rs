use super::guide_resource_ui::rotation_degrees;
use super::ui_style;
use super::*;
use crate::{editing, gizmo, scenes};

impl SketchApp {
    pub(super) fn guide_preview(&self) -> GuidePreview {
        if !self.guide_visible {
            GuidePreview::Hidden
        } else if let Some(preview) = self.saved_guide_preview() {
            preview
        } else if self.use_revolved {
            GuidePreview::Revolved(self.revolved)
        } else if self.use_cylinder {
            GuidePreview::Cylinder(self.cylinder)
        } else {
            GuidePreview::Plane(self.guide)
        }
    }
    pub(super) fn active_guide_label(&self) -> String {
        if let Some(guide) = self
            .doc
            .guide_resources
            .iter()
            .find(|guide| Some(guide.id) == self.doc.active_guide)
        {
            guide.name.clone()
        } else if self.use_revolved {
            "Revolved profile guide".into()
        } else if self.use_cylinder {
            "Cylinder guide".into()
        } else {
            self.guide.label().into()
        }
    }
    pub(super) fn active_guide_u(&self) -> Vec3 {
        if let Some((_, rotation, _)) = self.saved_guide_camera_frame() {
            rotation * Vec3::X
        } else if self.use_revolved {
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
        } else if self.doc.active_guide.is_some() {
            format!("Saved / {}", self.active_guide_label())
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
        if let Some((center, rotation, size)) = self.saved_guide_camera_frame() {
            self.camera.distance = self.camera.distance.max(size).clamp(0.3, 150.0);
            self.camera.align_guide(DrawingPlane {
                anchor: center,
                rotation_degrees: rotation_degrees(rotation),
                depth: 0.0,
            });
            return;
        } else if self.use_revolved {
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
                .color(ui_style::colors(ui.ctx()).accent),
        );
        ui_style::section(ui, "Cylinder geometry");
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui_style::field_row(ui, &format!("Rotation {axis}"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.cylinder.rotation_degrees[i])
                        .speed(0.5)
                        .range(-180.0..=180.0),
                );
                ui_style::numeric_field(
                    ui,
                    ("cylinder-rotation", i),
                    &mut self.cylinder.rotation_degrees[i],
                    Some(-180.0..=180.0),
                    1.0,
                    "°",
                )
            });
        }
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui_style::field_row(ui, &format!("Center {axis}"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.cylinder.center[i])
                        .speed(0.02)
                        .range(-1000.0..=1000.0),
                );
                ui_style::numeric_field(
                    ui,
                    ("cylinder-center", i),
                    &mut self.cylinder.center[i],
                    Some(-1000.0..=1000.0),
                    1.0,
                    "wu",
                )
            });
        }
        ui_style::logarithmic_slider_row(
            ui,
            "Radius",
            "cylinder-radius",
            &mut self.cylinder.radius,
            0.05..=10.0,
            1.0,
            "wu",
        );
        ui_style::logarithmic_slider_row(
            ui,
            "Height",
            "cylinder-height",
            &mut self.cylinder.height,
            0.1..=20.0,
            1.0,
            "wu",
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
        self.run_curved_construction_checks(rect);
        self.cancel_curve_tool();
        self.cancel_shape_draft();
        self.shape_tool.mode = None;
        self.load_lantern();
        self.stabilization = 0.35;
    }

    fn run_curved_construction_checks(&mut self, rect: Rect) {
        fn screen(app: &SketchApp, rect: Rect, cylinder: Cylinder, angle: f32, y: f32) -> Pos2 {
            gizmo::project(&app.camera, rect, cylinder.point(angle, y))
                .expect("cylinder contact projects into the verification view")
                .0
        }
        fn check_surface(stroke: &Stroke, cylinder: Cylinder) {
            let curve = stroke
                .curve
                .as_ref()
                .expect("curved stroke retains controls");
            assert!(curve.is_valid());
            assert!(curve.samples_match_stroke(stroke));
            for point in &stroke.points {
                let local = cylinder.to_local(point.position);
                assert!((local.x.hypot(local.z) - cylinder.radius).abs() < 0.001);
                assert!(local.y.abs() <= cylinder.height * 0.5 + 0.001);
            }
        }

        self.doc.active_image_plane = None;
        self.doc.active_guide = None;
        self.doc.active_model_reference = None;
        self.use_cylinder = true;
        self.use_revolved = false;
        self.curve_tool.tension = 0.0;
        let cylinder = self.cylinder;
        self.camera.target = cylinder.center;
        self.camera.yaw = std::f32::consts::PI;
        self.camera.pitch = 0.0;
        self.camera.distance = (cylinder.radius * 4.0).max(4.0);
        let before = self.doc.strokes.len();

        // Keep the captured cylinder while the camera orbits between seam anchors.
        self.start_curve_tool();
        let start = screen(self, rect, cylinder, std::f32::consts::PI - 0.07, -0.25);
        self.curve_input(start, 0.6, rect, true);
        self.curve_pointer_up();
        assert_eq!(self.curve_draft_anchor_count(), 1);
        self.camera.yaw += 0.18;
        self.camera.pitch = 0.04;
        self.camera.distance *= 1.08;
        self.camera.target += Vec3::new(0.03, 0.0, 0.0);
        let end = screen(self, rect, cylinder, -std::f32::consts::PI + 0.07, 0.25);
        self.curve_input(end, 0.9, rect, true);
        self.curve_pointer_up();
        let preview = self
            .curve_draft_preview()
            .expect("seam curve remains previewed after camera navigation")
            .clone();
        check_surface(&preview, cylinder);
        let anchors = &preview.curve.as_ref().unwrap().anchors;
        assert!(anchors[1].position.x > anchors[0].position.x);
        assert!(anchors[1].position.x - anchors[0].position.x < 0.25);
        assert_eq!(
            self.doc.strokes.len(),
            before,
            "preview does not mutate artwork"
        );
        self.cancel_curve();
        assert_eq!(
            self.doc.strokes.len(),
            before,
            "Escape cancels the curve preview"
        );

        self.start_curve_tool();
        let start = screen(self, rect, cylinder, std::f32::consts::PI - 0.07, -0.25);
        self.curve_input(start, 0.6, rect, true);
        self.curve_pointer_up();
        let end = screen(self, rect, cylinder, -std::f32::consts::PI + 0.07, 0.25);
        self.curve_input(end, 0.9, rect, true);
        self.curve_pointer_up();
        self.commit_curve();
        assert_eq!(self.doc.strokes.len(), before + 1);
        let curve = self.doc.strokes.last().unwrap().clone();
        check_surface(&curve, cylinder);
        self.undo();
        assert_eq!(self.doc.strokes.len(), before);
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&curve));

        let reloaded = Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
        assert_eq!(reloaded.strokes.last(), Some(&curve));
        self.doc = reloaded;
        self.undo.clear();
        self.redo.clear();
        self.selected = HashSet::from([curve.id]);
        self.cylinder.radius *= 1.5;
        self.start_curve_tool();
        self.open_selected_curve();
        let reopened = self
            .curve_draft_preview()
            .expect("saved cylinder curve reopens with a preview")
            .clone();
        check_surface(&reopened, cylinder);
        assert_eq!(
            reopened.curve.as_ref().unwrap().surface_target,
            curve.curve.as_ref().unwrap().surface_target
        );
        let source_curve = curve.curve.as_ref().unwrap();
        let first_anchor = source_curve.anchors[0].position;
        let first_world = cylinder.point_from_parameter(first_anchor).unwrap();
        let moved_world = cylinder
            .point_from_parameter(first_anchor + Vec3::new(0.0, 0.06, 0.0))
            .unwrap();
        let start = gizmo::project(&self.camera, rect, first_world).unwrap().0;
        let end = gizmo::project(&self.camera, rect, moved_world).unwrap().0;
        self.curve_input(start, 0.6, rect, true);
        self.curve_input(end, 0.6, rect, false);
        self.curve_pointer_up();
        let edited_preview = self.curve_draft_preview().unwrap().clone();
        check_surface(&edited_preview, cylinder);
        self.commit_curve();
        let edited = self.doc.strokes.last().unwrap().clone();
        check_surface(&edited, cylinder);
        self.undo();
        assert_eq!(self.doc.strokes.last(), Some(&curve));
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&edited));
        self.cancel_curve_tool();
        self.undo.clear();
        self.redo.clear();

        self.cylinder = cylinder;
        self.selected.clear();
        self.shape_tool.mode = Some(shape_tool::ShapeMode::Ellipse);
        self.shape_tool.precise = true;
        self.shape_tool.width = 0.24;
        self.shape_tool.height = 0.5;
        self.start_shape_tool(shape_tool::ShapeMode::Ellipse);
        let center = Vec3::new(std::f32::consts::PI * cylinder.radius - 0.02, 0.0, 0.0);
        let world_center = cylinder.point_from_parameter(center).unwrap();
        let center_screen = gizmo::project(&self.camera, rect, world_center).unwrap().0;
        let shape_count = self.doc.strokes.len();
        self.shape_input(center_screen, 0.8, rect, true);
        let shape_preview = self.shape_draft_preview().unwrap().clone();
        check_surface(&shape_preview, cylinder);
        assert_eq!(
            shape_preview.curve.as_ref().unwrap().shape.unwrap().kind,
            crate::document::CurvedShapeKind::Ellipse
        );
        self.cancel_shape_draft();
        assert_eq!(self.doc.strokes.len(), shape_count);
        self.shape_input(center_screen, 0.8, rect, true);
        self.release_shape(center_screen, 0.8, rect);
        assert_eq!(self.doc.strokes.len(), shape_count + 1);
        let shape = self.doc.strokes.last().unwrap().clone();
        check_surface(&shape, cylinder);

        self.doc = Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
        self.undo.clear();
        self.redo.clear();
        self.selected = HashSet::from([shape.id]);
        self.open_selected_curved_shape();
        assert!(self.shape_edit_active());
        self.shape_tool.width *= 1.5;
        self.refresh_curved_shape_edit();
        let shape_preview = self.shape_draft_preview().unwrap().clone();
        check_surface(&shape_preview, cylinder);
        assert_ne!(shape_preview, shape);
        self.commit_curved_shape_edit();
        let edited_shape = self.doc.strokes.last().unwrap().clone();
        assert_eq!(edited_shape.id, shape.id);
        assert_eq!(self.undo.len(), 1);
        self.undo();
        assert_eq!(self.doc.strokes.last(), Some(&shape));
        self.redo();
        assert_eq!(self.doc.strokes.last(), Some(&edited_shape));
        self.shape_tool.mode = None;
        self.selected.clear();
    }
}
