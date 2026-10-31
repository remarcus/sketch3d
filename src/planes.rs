use super::*;
use crate::{editing, gizmo};

impl SketchApp {
    pub(super) fn plane_from_view(&mut self) {
        self.finish();
        self.guide = DrawingPlane::from_view(&self.camera);
        self.use_cylinder = false;
        self.use_revolved = false;
        self.status =
            "Drawing plane placed through the camera focus, facing the current view.".into();
    }
    pub(super) fn plane_controls(&mut self, ui: &mut egui::Ui) {
        if ui.button("Place plane on artwork [K]").clicked() {
            self.begin_placement();
        }
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!self.use_cylinder && !self.use_revolved, "Plane")
                .clicked()
            {
                self.finish();
                self.use_cylinder = false;
                self.use_revolved = false;
            }
            if ui
                .selectable_label(self.use_cylinder && !self.use_revolved, "Cylinder")
                .clicked()
            {
                self.finish();
                self.use_cylinder = true;
                self.use_revolved = false;
            }
        });
        if ui
            .selectable_label(self.use_revolved, "Revolved profile")
            .clicked()
        {
            self.finish();
            self.use_revolved = true;
            self.use_cylinder = false;
        }
        if ui.checkbox(&mut self.guide_visible, "Show guide").changed() {
            self.finish();
        }
        if self.use_revolved {
            self.revolved_controls(ui);
            return;
        }
        if self.use_cylinder {
            self.cylinder_controls(ui);
            return;
        }
        let before = self.guide;
        ui.horizontal(|ui| {
            for (plane, label) in [
                (Plane::Front, "Front"),
                (Plane::Side, "Side"),
                (Plane::Ground, "Ground"),
            ] {
                if ui
                    .selectable_label(self.guide.preset() == Some(plane), label)
                    .on_hover_text("Reset orientation and placement to this world plane")
                    .clicked()
                {
                    self.finish();
                    self.guide = DrawingPlane::from_preset(plane);
                }
            }
        });
        ui.label(
            RichText::new(self.guide.label())
                .small()
                .color(Color32::from_rgb(119, 203, 195)),
        );
        ui.label("Rotation in degrees");
        ui.horizontal(|ui| {
            for (i, label) in ["X ", "Y ", "Z "].iter().enumerate() {
                ui.add(
                    egui::DragValue::new(&mut self.guide.rotation_degrees[i])
                        .prefix(*label)
                        .speed(0.5)
                        .range(-180.0..=180.0),
                );
            }
        });
        ui.label("Anchor in world units");
        ui.horizontal(|ui| {
            for (i, label) in ["X ", "Y ", "Z "].iter().enumerate() {
                ui.add(
                    egui::DragValue::new(&mut self.guide.anchor[i])
                        .prefix(*label)
                        .speed(0.02)
                        .range(-20000.0..=20000.0),
                );
            }
        });
        ui.add(egui::Slider::new(&mut self.guide.depth,-4.0..=4.0).text("Offset")).on_hover_text("Move the plane along its normal, relative to the anchor. Existing strokes stay fixed.");
        if self.guide != before {
            self.finish();
        }
        if ui
            .button("Plane from view [Shift+F]")
            .on_hover_text(
                "Face the camera, through its focus point. Does not move existing strokes.",
            )
            .clicked()
        {
            self.plane_from_view();
        }
        if ui
            .add_enabled(
                !self.selected.is_empty(),
                egui::Button::new("Center on selection"),
            )
            .on_hover_text(
                "Move the anchor to the selection's center, retaining the plane orientation",
            )
            .clicked()
        {
            self.finish();
            if let Some((min, max)) = editing::bounds(&self.doc, &self.selected) {
                self.guide.anchor = (min + max) * 0.5;
                self.guide.depth = 0.0;
            }
        }
        if ui.button("Align view to plane [F]").clicked() {
            self.finish();
            self.camera.align_guide(self.guide);
        }
    }
    pub(super) fn plane_overlay(&self, ui: &egui::Ui, rect: Rect) {
        if self.placement.active
            || self.use_revolved
            || self.use_cylinder
            || !self.guide_visible
            || self.guide.preset().is_some()
        {
            return;
        }
        let origin = self.guide.origin();
        let normal = self.guide.basis().2;
        let Some((start, _)) = gizmo::project(&self.camera, rect, origin) else {
            return;
        };
        let length = 60.0 * self.camera.world_per_pixel(origin, rect.height());
        let Some((end, _)) = gizmo::project(&self.camera, rect, origin + normal * length) else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let color = Color32::from_rgb(87, 215, 207);
        painter.circle_stroke(start, 5.0, egui::Stroke::new(1.5_f32, color));
        if start.distance(end) > 10.0 {
            painter.arrow(start, end - start, egui::Stroke::new(1.5_f32, color));
            painter.text(
                end + egui::vec2(6.0, -10.0),
                egui::Align2::LEFT_CENTER,
                "N",
                egui::FontId::proportional(12.0),
                color,
            );
        }
    }
    pub(super) fn verify_planes(&mut self, rect: Rect) {
        let original = self.doc.strokes.clone();
        let selection = self.selected.clone();
        let strength = self.stabilization;
        self.select_mode = false;
        self.stabilization = 0.6;
        self.guide = DrawingPlane {
            anchor: Vec3::new(0.4, 0.3, -0.2),
            rotation_degrees: Vec3::new(32.0, -27.0, 18.0),
            depth: 0.35,
        };
        self.camera.align_guide(self.guide);
        let origin = self.guide.origin();
        let normal = self.guide.basis().2;
        for i in 0..70 {
            self.add_point(
                rect.center() + egui::vec2(i as f32 * 2.0 - 70.0, (i as f32 * 0.07).sin() * 30.0),
                0.7,
                rect,
                i == 0,
            );
        }
        self.finish();
        let custom = self.doc.strokes.last().unwrap();
        assert!(custom.points.len() > 30);
        for point in &custom.points {
            assert!((point.position - origin).dot(normal).abs() < 0.00001);
        }
        let first_plane = self.doc.strokes.clone();
        self.guide = DrawingPlane::from_preset(Plane::Ground);
        self.guide.depth = 1.5;
        assert_eq!(
            self.doc.strokes, first_plane,
            "Changing the work plane must not move existing ink"
        );
        self.camera.align_guide(self.guide);
        for i in 0..30 {
            self.add_point(
                rect.center() + egui::vec2(i as f32 * 3.0 - 45.0, 25.0),
                0.5,
                rect,
                i == 0,
            );
        }
        self.finish();
        assert!(self.camera.matrix(rect.aspect_ratio()).is_finite());
        assert!(self
            .doc
            .strokes
            .last()
            .unwrap()
            .points
            .iter()
            .all(|p| (p.position.y - 1.5).abs() < 0.00001));
        assert!(self.save(false));
        let loaded =
            Document::from_json(&std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap())
                .unwrap();
        assert_eq!(loaded.strokes, self.doc.strokes);
        self.undo();
        assert_eq!(self.doc.strokes, first_plane);
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.camera = Camera::default();
        self.camera.roll = 0.25;
        self.plane_from_view();
        let (_, up, n) = self.guide.basis();
        let (_, camera_up, camera_n) = self.camera.basis();
        assert!(up.dot(camera_up) > 0.9999 && n.dot(camera_n) > 0.9999);
        self.guide = DrawingPlane {
            anchor: Vec3::ZERO,
            rotation_degrees: Vec3::new(32.0, -22.0, 15.0),
            depth: 0.0,
        };
        self.camera = Camera::default();
        self.selected = selection;
        self.stabilization = strength;
        assert!(self.save(false));
        self.status="Custom planes: rotated drawing, vertical alignment, plane switching, and persistence checks passed.".into();
    }
}
