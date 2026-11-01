use super::*;
use crate::document::guide_resources::{GuideGeometry, GuideResource};

impl SketchApp {
    /// Exercise saved curved-guide live mirror through the canvas pointer handlers.
    pub(super) fn verify_mirror(&mut self, ctx: &egui::Context, rect: Rect) {
        if self.mirror_verification_frame != Some(0) {
            return;
        }

        self.doc = Document::default();
        self.doc.guide_resources.push(GuideResource {
            id: 9401,
            name: "Mirror verification cylinder".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Cylinder {
                center: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                radius: 1.0,
                height: 2.0,
            },
        });
        self.doc.active_guide = Some(9401);
        self.doc.active_image_plane = None;
        self.doc.active_model_reference = None;
        self.camera.align_guide(DrawingPlane {
            anchor: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            depth: 0.0,
        });
        self.camera.distance = 4.0;
        self.select_mode = false;
        self.eraser = false;
        self.line.enabled = false;
        self.curve_tool.enabled = false;
        self.fills.enabled = false;
        self.shape_tool.mode = None;
        self.mirror_ui.enabled = true;
        self.stabilization = 0.0;
        self.next_id = 1;
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;

        let world_points = [
            Vec3::new(0.6, -0.55, 0.8),
            Vec3::new(0.72, 0.0, (1.0_f32 - 0.72_f32.powi(2)).sqrt()),
            Vec3::new(0.8, 0.55, 0.6),
        ];
        let screen_points: Vec<_> = world_points
            .iter()
            .map(|point| {
                crate::gizmo::project(&self.camera, rect, *point)
                    .expect("saved-cylinder verification point projects")
                    .0
            })
            .collect();
        assert!(screen_points.iter().all(|point| rect.contains(*point)));

        let baseline = self.doc.strokes.clone();
        let history_before = self.undo.len();
        self.pointer_action(screen_points[0], 0.45, rect, true, false);
        self.pointer_action(screen_points[1], 0.75, rect, false, false);
        self.pointer_action(screen_points[2], 1.0, rect, false, false);
        assert!(
            self.active.is_some(),
            "pointer samples leave a live brush draft"
        );
        assert_eq!(self.doc.strokes.len(), 1);
        let draft = self.doc.strokes[0].clone();
        assert!(
            draft.points.len() >= 2,
            "curved-guide input records samples"
        );
        let geometry = self.saved_guide_geometry().expect("active cylinder");
        for point in &draft.points {
            let local = match geometry {
                GuideGeometry::Cylinder {
                    center, rotation, ..
                } => rotation.inverse() * (point.position - *center),
                _ => unreachable!(),
            };
            assert!((local.x.hypot(local.z) - 1.0).abs() < 0.003);
        }

        let preview = self
            .mirrored_runs_for(&draft)
            .expect("curved-guide mirror preview should build");
        assert_eq!(preview.len(), 1, "one continuous source run previews once");
        assert_eq!(
            self.doc.strokes.as_slice(),
            std::slice::from_ref(&draft),
            "preview does not mutate ink"
        );
        let preview_copy = preview[0].clone();

        self.finish();
        assert!(self.active.is_none());
        assert_eq!(self.undo.len(), history_before + 1, "pair is one undo edit");
        assert_eq!(
            self.doc.strokes.len(),
            2,
            "source and reflected copy commit"
        );
        assert_eq!(
            self.doc.strokes.iter().find(|stroke| stroke.id == draft.id),
            Some(&draft),
            "the original stroke stays unchanged"
        );
        assert_eq!(
            self.doc
                .strokes
                .iter()
                .find(|stroke| stroke.id == preview_copy.id),
            Some(&preview_copy),
            "commit matches preview geometry exactly"
        );
        let committed = self.doc.strokes.clone();
        let encoded = crate::export::encode(&self.doc, None).expect("mirrored GLB export");
        assert_eq!(encoded.summary.strokes, 2);

        self.undo();
        assert_eq!(self.doc.strokes, baseline, "one undo removes both strokes");
        self.redo();
        assert_eq!(
            self.doc.strokes, committed,
            "redo restores exact mirrored pair"
        );

        let history_after_commit = self.undo.len();
        self.pointer_action(screen_points[0], 0.5, rect, true, false);
        self.pointer_action(screen_points[2], 0.9, rect, false, false);
        assert_eq!(self.doc.strokes.len(), 3, "second source remains a draft");
        self.cancel();
        assert_eq!(
            self.doc.strokes, committed,
            "cancel removes only the in-progress source"
        );
        assert_eq!(
            self.undo.len(),
            history_after_commit,
            "cancel creates no history edit"
        );
        assert!(self.active.is_none());

        self.status = "Saved curved-guide mirror pointer checks passed.".into();
        self.dirty = false;
        self.renderer.invalidate();
        self.mirror_verification_frame = Some(1);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
}
