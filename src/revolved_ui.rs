use super::ui_style;
use super::*;
use crate::{
    gizmo,
    revolved::{ProfilePoint, Revolved},
};
impl SketchApp {
    pub(super) fn revolved_controls(&mut self, ui: &mut egui::Ui) {
        let mut next = self.revolved;
        ui.label("Revolved profile / open side surface");
        ui.horizontal(|ui| {
            if ui.button("Vase").clicked() {
                next.profile = Revolved::default().profile;
                next.count = Revolved::default().count;
            }
            if ui.button("Bowl").clicked() {
                next.count = 3;
                next.profile[0] = ProfilePoint {
                    height: -1.0,
                    radius: 0.25,
                };
                next.profile[1] = ProfilePoint {
                    height: -0.3,
                    radius: 0.85,
                };
                next.profile[2] = ProfilePoint {
                    height: 0.5,
                    radius: 1.3,
                };
            }
            if ui.button("Cylinder").clicked() {
                next.count = 2;
                next.profile[0] = ProfilePoint {
                    height: -1.5,
                    radius: 0.8,
                };
                next.profile[1] = ProfilePoint {
                    height: 1.5,
                    radius: 0.8,
                };
            }
        });
        ui.label("Drag profile points, or edit radius / height");
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 160.0),
            egui::Sense::hover(),
        );
        let max_radius = next.max_radius().max(1.5) * 1.15;
        let min = next.min_height() - 0.2;
        let max = next.max_height() + 0.2;
        let pos = |p: ProfilePoint| {
            egui::pos2(
                rect.left() + p.radius / max_radius * rect.width(),
                rect.bottom() - (p.height - min) / (max - min) * rect.height(),
            )
        };
        ui.painter()
            .rect_filled(rect, 3.0, ui_style::colors(ui.ctx()).surface);
        for i in 0..next.count {
            let point = pos(next.profile[i]);
            if i > 0 {
                ui.painter().line_segment(
                    [pos(next.profile[i - 1]), point],
                    egui::Stroke::new(2.0_f32, ui_style::colors(ui.ctx()).accent),
                );
            }
            let response = ui.interact(
                Rect::from_center_size(point, egui::vec2(18.0, 18.0)),
                ui.id().with(("profile", i)),
                egui::Sense::drag(),
            );
            ui.painter()
                .circle_filled(point, 4.0, ui_style::colors(ui.ctx()).focus);
            if response.dragged() {
                if let Some(mouse) = response.interact_pointer_pos() {
                    next.profile[i].radius =
                        ((mouse.x - rect.left()) / rect.width() * max_radius).clamp(0.05, 10.0);
                    next.profile[i].height =
                        min + (rect.bottom() - mouse.y) / rect.height() * (max - min);
                }
            }
        }
        let mut remove = None;
        for i in 0..next.count {
            ui.horizontal(|ui| {
                ui.strong(format!("Profile {}", i + 1));
                if ui
                    .add_enabled(next.count > 2, egui::Button::new("−"))
                    .clicked()
                {
                    remove = Some(i);
                }
            });
            ui_style::numeric_row(
                ui,
                &format!("Profile {} · Radius", i + 1),
                ("revolved-radius", i),
                &mut next.profile[i].radius,
                Some(0.05..=10.0),
                "wu",
            );
            ui_style::numeric_row(
                ui,
                &format!("Profile {} · Height", i + 1),
                ("revolved-height", i),
                &mut next.profile[i].height,
                Some(-20.0..=20.0),
                "wu",
            );
        }
        if let Some(i) = remove {
            for j in i..next.count - 1 {
                next.profile[j] = next.profile[j + 1];
            }
            next.count -= 1;
        }
        if ui
            .add_enabled(next.count < 8, egui::Button::new("Insert profile point"))
            .clicked()
        {
            let i = (0..next.count - 1)
                .max_by(|a, b| {
                    (next.profile[a + 1].height - next.profile[*a].height)
                        .total_cmp(&(next.profile[b + 1].height - next.profile[*b].height))
                })
                .unwrap();
            let p = ProfilePoint {
                height: (next.profile[i].height + next.profile[i + 1].height) * 0.5,
                radius: (next.profile[i].radius + next.profile[i + 1].radius) * 0.5,
            };
            for j in (i + 1..next.count).rev() {
                next.profile[j + 1] = next.profile[j];
            }
            next.profile[i + 1] = p;
            next.count += 1;
        }
        ui_style::section(ui, "Guide placement");
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui_style::field_row(ui, &format!("Center {axis}"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut next.center[i])
                        .speed(0.02)
                        .range(-1000.0..=1000.0),
                );
                ui_style::numeric_field(
                    ui,
                    ("revolved-center", i),
                    &mut next.center[i],
                    Some(-1000.0..=1000.0),
                    1.0,
                    "wu",
                )
            });
        }
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui_style::field_row(ui, &format!("Rotation {axis}"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut next.rotation_degrees[i])
                        .speed(0.5)
                        .range(-180.0..=180.0),
                );
                ui_style::numeric_field(
                    ui,
                    ("revolved-rotation", i),
                    &mut next.rotation_degrees[i],
                    Some(-180.0..=180.0),
                    1.0,
                    "°",
                )
            });
        }
        if ui.button("Profile from view").clicked() {
            let plane = DrawingPlane::from_view(&self.camera);
            next.center = plane.anchor;
            next.rotation_degrees = plane.rotation_degrees;
        }
        if next != self.revolved {
            if next.valid() {
                self.finish();
                self.revolved = next;
            } else {
                self.status = "Profile points need increasing heights and positive radii.".into();
            }
        }
        if ui.button("Align view [F]").clicked() {
            self.align_active_guide();
        }
        ui.small("Edit the guide before drawing. Existing ink stays fixed. Open ends are not drawing surfaces.");
    }
    pub(super) fn load_vase(&mut self) {
        self.reset_recovery_document();
        self.cancel();
        self.doc = crate::export_check::vase_fixture();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.revolved = Revolved::default();
        self.use_revolved = true;
        self.use_cylinder = false;
        self.guide_visible = true;
        self.camera = Camera {
            target: self.revolved.center,
            distance: 8.0,
            yaw: 0.45,
            pitch: 0.15,
            ..Default::default()
        };
        self.file = None;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.renderer.invalidate();
        self.starter_scene = false;
        self.cylinder_example = false;
        self.select_mode = false;
        self.eraser = false;
        self.status="Revolved vase / editable ink. Change profile for new marks; hide the guide to view finished strokes.".into();
    }
    pub(super) fn verify_vase(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.vase_verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.revolved.rotation_degrees = Vec3::new(15.0, -20.0, 10.0);
                self.align_active_guide();
                self.brush.kind = BrushKind::Chisel;
                self.brush.dynamics.tilt_orientation = true;
                self.stabilization = 0.8;
                let original = self.doc.strokes.clone();
                for (i, (a, y)) in [(-0.3, -0.9), (0.0, 0.0), (0.3, 0.8)]
                    .into_iter()
                    .enumerate()
                {
                    let p = self.revolved.point(a, y);
                    let screen = gizmo::project(&self.camera, rect, p).unwrap().0;
                    self.add_point_with_tilt(
                        screen,
                        0.4 + i as f32 * 0.2,
                        rect,
                        i == 0,
                        Some(Vec3::Y),
                    );
                }
                let captured = self.revolved;
                self.revolved.profile[2].radius += 0.1;
                self.finish();
                self.revolved = captured;
                let stroke = self.doc.strokes.last().unwrap().clone();
                assert!(stroke.points.last().unwrap().tilt.is_some());
                assert!((stroke.points.last().unwrap().pressure - 0.8).abs() < 1e-6);
                assert!(self.doc.strokes.len() > original.len());
                for p in &stroke.points {
                    let local = self.revolved.to_local(p.position);
                    assert!(
                        (local.x.hypot(local.z) - self.revolved.radius_at(local.y)).abs() < 1e-4
                    );
                }
                self.undo();
                assert_eq!(self.doc.strokes, original);
                self.redo();
                assert_eq!(self.doc.strokes.last().unwrap(), &stroke);
                let ink = self.doc.strokes.clone();
                self.selected = HashSet::from([stroke.id]);
                self.duplicate_selected();
                let duplicated_brush = &self.doc.strokes.last().unwrap().brush;
                assert!((duplicated_brush.normal - stroke.brush.normal).length() < 1e-6);
                assert!((duplicated_brush.tangent - stroke.brush.tangent).length() < 1e-6);
                let mut expected_brush = stroke.brush;
                expected_brush.normal = duplicated_brush.normal;
                expected_brush.tangent = duplicated_brush.tangent;
                assert_eq!(*duplicated_brush, expected_brush);
                self.undo();
                assert_eq!(self.doc.strokes, ink);
                self.selected.clear();
                self.guide_visible = false;
                assert!(matches!(self.guide_preview(), GuidePreview::Hidden));
                self.guide_visible = true;
                self.revolved.profile[2].radius += 0.1;
                assert_eq!(self.doc.strokes, ink);
                let loaded =
                    Document::from_json(&serde_json::to_string(&self.doc).unwrap()).unwrap();
                assert_eq!(loaded.strokes, ink);
                self.load_vase();
            }
            3 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Vase screenshot timed out"),
            _ => {}
        }
        self.vase_verification_frame = Some(frame + 1);
    }
}
