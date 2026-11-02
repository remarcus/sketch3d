use super::*;
use crate::{
    document::{guide_resources::GuideGeometry, Brush},
    geometry::Projection,
    surface::Surface,
};

impl SketchApp {
    /// Exercise saved-ring placement, pointer drawing, misses, and cancellation.
    pub(super) fn verify_ring(&mut self, ctx: &egui::Context, rect: Rect) {
        if self.ring_verification_frame != Some(0) {
            return;
        }

        self.doc = Document::default();
        self.doc.strokes.clear();
        self.doc.guide_resources.clear();
        self.doc.active_guide = None;
        self.doc.active_image_plane = None;
        self.doc.active_model_reference = None;
        self.next_id = 1;
        self.undo.clear();
        self.redo.clear();
        self.select_mode = false;
        self.eraser = false;
        self.line.enabled = false;
        self.curve_tool.enabled = false;
        self.fills.enabled = false;
        self.shape_tool.mode = None;
        self.mirror_ui.enabled = false;
        self.stabilization = 0.0;
        self.camera.target = Vec3::new(0.35, -0.2, 0.6);
        self.camera.projection = Projection::Orthographic;
        self.camera.align_guide(DrawingPlane {
            anchor: self.camera.target,
            rotation_degrees: Vec3::ZERO,
            depth: 0.0,
        });
        self.camera.distance = 6.0;
        self.brush = Brush {
            dabs: crate::dynamics::DabSettings {
                enabled: true,
                spacing: 0.5,
                ..Default::default()
            },
            ..Brush::default()
        };
        self.radius = 0.1;

        let placed_center = self.camera.target;
        self.add_saved_guide(
            "Verification ring",
            GuideGeometry::Ring {
                center: self.camera.target,
                rotation: glam::Quat::IDENTITY,
                major_radius: 1.5,
                minor_radius: 0.4,
            },
        );
        let guide_id = self.doc.active_guide.expect("added ring becomes active");
        let guide = self
            .doc
            .guide_resources
            .iter()
            .find(|guide| guide.id == guide_id)
            .expect("saved ring exists");
        let GuideGeometry::Ring { center, .. } = &guide.geometry else {
            panic!("Add ring creates toroidal geometry");
        };
        assert_eq!(*center, placed_center);
        let guide_snapshot = self.doc.clone();
        self.undo();
        assert!(self.doc.guide_resources.is_empty(), "guide add is undoable");
        self.redo();
        assert_eq!(self.doc.guide_resources, guide_snapshot.guide_resources);
        assert_eq!(self.doc.active_guide, Some(guide_id));

        let (center, rotation, major_radius, minor_radius) =
            match self.saved_guide_geometry().expect("active ring geometry") {
                GuideGeometry::Ring {
                    center,
                    rotation,
                    major_radius,
                    minor_radius,
                } => (*center, *rotation, *major_radius, *minor_radius),
                _ => unreachable!(),
            };
        self.align_active_guide();
        self.camera.distance = 6.0;
        assert!(
            self.camera.basis().2.dot(rotation * Vec3::Y) > 0.999,
            "saved-ring alignment faces the torus hole"
        );
        let screen_at = |x: f32, y: f32| {
            crate::gizmo::project(&self.camera, rect, center + rotation * Vec3::new(x, 0.0, y))
                .expect("ring pointer sample projects")
                .0
        };
        let across_hole = [
            screen_at(-1.5, 0.0),
            screen_at(-1.3, 0.0),
            screen_at(0.0, 0.0),
            screen_at(1.3, 0.0),
            screen_at(1.5, 0.0),
        ];
        let cancel_start = screen_at(1.5, 0.3);
        let cancel_end = screen_at(1.55, 0.3);
        assert!(across_hole.iter().all(|point| rect.contains(*point)));

        self.brush.dabs.scatter = 0.25;
        let strokes_before_rejected_scatter = self.doc.strokes.clone();
        let history_before_rejected_scatter = self.undo.len();
        self.pointer_action(across_hole[0], 0.35, rect, true, false);
        assert!(
            self.active.is_none(),
            "ring scatter is rejected before drawing"
        );
        assert_eq!(self.doc.strokes, strokes_before_rejected_scatter);
        assert_eq!(self.undo.len(), history_before_rejected_scatter);
        self.brush.dabs.scatter = 0.0;

        self.pointer_action(across_hole[0], 0.35, rect, true, false);
        self.pointer_action(across_hole[1], 0.5, rect, false, false);
        self.pointer_action(across_hole[2], 0.65, rect, false, false);
        self.pointer_action(across_hole[3], 0.8, rect, false, false);
        self.pointer_action(across_hole[4], 1.0, rect, false, false);
        self.finish();
        assert_eq!(
            self.doc.strokes.len(),
            2,
            "the hole miss splits the gesture"
        );
        let ring = match self.saved_guide_surface().expect("ring surface") {
            Surface::Ring(ring) => ring,
            _ => unreachable!(),
        };
        for stroke in &self.doc.strokes {
            assert!(stroke.points.len() >= 2, "each side keeps a drawable run");
            for point in &stroke.points {
                let local = ring.rotation.inverse() * (point.position - ring.center);
                let radial = local.x.hypot(local.z);
                assert!(
                    ((radial - major_radius).hypot(local.y) - minor_radius).abs() < 0.003,
                    "stabilized ink remains on the torus surface"
                );
                assert!(radial > major_radius - minor_radius - 0.003);
            }
            let dabs = crate::dab_placement::sample(stroke).unwrap();
            assert!(!dabs.is_empty(), "saved-ring run resolves visible dabs");
            for dab in dabs {
                let local = ring.rotation.inverse() * (dab.position - ring.center);
                let radial = local.x.hypot(local.z);
                assert!(
                    ((radial - major_radius).hypot(local.y) - minor_radius).abs() < 0.003,
                    "dab centers follow the captured torus path within chordal tolerance"
                );
                assert!(radial > major_radius - minor_radius - 0.003);
            }
        }
        let committed = self.doc.strokes.clone();
        assert_eq!(
            crate::export::encode(&self.doc, None)
                .expect("ring strokes export")
                .summary
                .strokes,
            2
        );
        self.undo();
        assert_eq!(
            self.doc.strokes.len(),
            1,
            "undo removes the last surface run"
        );
        self.redo();
        assert_eq!(
            self.doc.strokes, committed,
            "redo restores both runs exactly"
        );

        let history_before_cancel = self.undo.len();
        let before_cancel = self.doc.strokes.clone();
        self.pointer_action(cancel_start, 0.4, rect, true, false);
        self.pointer_action(cancel_end, 0.9, rect, false, false);
        assert!(
            self.active.is_some(),
            "cancel sample creates an in-progress stroke"
        );
        self.cancel();
        assert_eq!(self.doc.strokes, before_cancel);
        assert_eq!(self.undo.len(), history_before_cancel);

        let native = serde_json::to_string(&self.doc).unwrap();
        let reopened = Document::from_json(&native).unwrap();
        assert_eq!(reopened.guide_resources, self.doc.guide_resources);
        assert_eq!(reopened.strokes, self.doc.strokes);
        self.status = "Saved-ring pointer checks passed.".into();
        self.dirty = false;
        self.renderer.invalidate();
        self.ring_verification_frame = Some(1);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
}
