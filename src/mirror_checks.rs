use super::*;
use crate::document::guide_resources::{GuideGeometry, GuideResource};

impl SketchApp {
    /// Exercise saved curved-guide live mirror through the canvas pointer handlers.
    pub(super) fn verify_mirror(&mut self, ctx: &egui::Context, rect: Rect) {
        fn replay_mouse(
            app: &mut SketchApp,
            ctx: &egui::Context,
            rect: Rect,
            start: Pos2,
            end: Pos2,
            release: bool,
        ) {
            let mut events = vec![egui::Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }];
            events.push(egui::Event::PointerMoved(end));
            if release {
                events.push(egui::Event::PointerButton {
                    pos: end,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
            }
            ctx.input_mut(|input| input.events = events);
            app.canvas_input(ctx, rect, Vec::new());
            ctx.input_mut(|input| input.events.clear());
        }

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

        let plane_rotation = glam::Quat::from_rotation_z(0.37) * glam::Quat::from_rotation_x(0.21);
        let plane_origin = Vec3::ZERO;
        self.doc.guide_resources.push(GuideResource {
            id: 9402,
            name: "Mirror verification line plane".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Plane {
                origin: plane_origin,
                rotation: plane_rotation,
            },
        });
        self.doc.active_guide = Some(9402);
        self.use_cylinder = false;
        self.use_revolved = false;
        let plane = DrawingPlane {
            anchor: plane_origin,
            rotation_degrees: super::guide_resource_ui::rotation_degrees(plane_rotation),
            depth: 0.0,
        };
        self.camera.align_guide(plane);
        self.camera.distance = 4.0;
        self.line.enabled = true;
        self.line.constraint = line_tool::Constraint::Horizontal;
        self.mirror_ui.enabled = true;
        self.mouse_down = false;

        let (u, v, normal) = plane.basis();
        let line_start_world = plane_origin + u * 0.45 + v * 0.35;
        let line_end_world = plane_origin + u * 1.15 + v * 0.35;
        let line_start = crate::gizmo::project(&self.camera, rect, line_start_world)
            .expect("rotated-plane line start projects")
            .0;
        let line_end = crate::gizmo::project(&self.camera, rect, line_end_world)
            .expect("rotated-plane line end projects")
            .0;
        assert!(rect.contains(line_start) && rect.contains(line_end));
        let line_baseline = self.doc.strokes.clone();
        let line_history_before = self.undo.len();

        replay_mouse(self, ctx, rect, line_start, line_end, false);
        assert!(
            self.line.draft.is_some(),
            "mouse drag previews a straight line"
        );
        assert_eq!(
            self.doc.strokes, line_baseline,
            "line preview leaves ink untouched"
        );
        let line_draft = self.line.draft.as_ref().unwrap().mark.clone();
        assert_eq!(line_draft.points.len(), 2);
        for point in &line_draft.points {
            assert!((point.position - plane_origin).dot(normal).abs() < 0.0001);
        }
        assert!(
            (line_draft.points[1].position - line_draft.points[0].position)
                .dot(v)
                .abs()
                < 0.0001,
            "horizontal line constraint follows the active guide's U axis"
        );
        let line_preview = self
            .mirrored_runs_for(&line_draft)
            .expect("rotated active-guide line mirror preview builds");
        assert_eq!(line_preview.len(), 1);
        assert_eq!(
            self.doc.strokes, line_baseline,
            "mirror preview does not commit ink"
        );

        ctx.input_mut(|input| {
            input.events = vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        });
        self.line_shortcuts(ctx);
        ctx.input_mut(|input| input.events.clear());
        assert!(self.line.draft.is_none(), "Escape cancels the line preview");
        assert_eq!(self.doc.strokes, line_baseline);
        assert_eq!(self.undo.len(), line_history_before);
        assert!(!self.mouse_down);

        replay_mouse(self, ctx, rect, line_start, line_end, true);
        assert!(self.line.draft.is_none(), "mouse release commits the line");
        assert_eq!(self.doc.strokes.len(), line_baseline.len() + 2);
        assert_eq!(self.undo.len(), line_history_before + 1);
        let source_line = &self.doc.strokes[line_baseline.len()];
        let mirrored_line = &self.doc.strokes[line_baseline.len() + 1];
        assert_eq!(source_line.points, line_draft.points);
        let mirror_plane = crate::mirror::MirrorPlane {
            point: plane_origin,
            normal: u,
        };
        for (source, mirrored) in source_line.points.iter().zip(&mirrored_line.points) {
            assert!(
                mirrored
                    .position
                    .distance(mirror_plane.reflect_point(source.position).unwrap())
                    < 0.0001
            );
        }
        let line_committed = self.doc.strokes.clone();
        let line_export = crate::export::encode(&self.doc, None).expect("line mirror GLB export");
        assert_eq!(line_export.summary.strokes, line_committed.len());
        self.undo();
        assert_eq!(
            self.doc.strokes, line_baseline,
            "one undo removes line and mirror"
        );
        self.redo();
        assert_eq!(
            self.doc.strokes, line_committed,
            "redo restores the exact line pair"
        );

        // Keep the curved-guide replay focused on the saved-cylinder workflow
        // after the verifier has also exercised the rotated-plane line path.
        self.doc.strokes = committed;
        self.doc.guide_resources.retain(|guide| guide.id == 9401);
        self.doc.active_guide = Some(9401);
        self.line.enabled = false;
        self.camera.align_guide(DrawingPlane {
            anchor: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            depth: 0.0,
        });
        self.camera.distance = 4.0;

        self.status =
            "Saved-guide brush and rotated-plane line mirror pointer checks passed.".into();
        self.dirty = false;
        self.renderer.invalidate();
        self.mirror_verification_frame = Some(1);
        if self
            .capability_replay
            .as_ref()
            .is_none_or(|replay| !replay.clip.is_curved_guides())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
    }
}
