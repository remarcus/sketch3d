use super::*;
use crate::document::guide_resources::{GuideGeometry, GuideResource};

impl SketchApp {
    pub(super) fn prepare_capability_mirror_shapes(&mut self, rect: Rect) -> usize {
        self.doc = Document::default();
        self.next_id = 1;
        self.camera = Camera {
            projection: crate::geometry::Projection::Orthographic,
            distance: 5.0,
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        };
        self.guide = DrawingPlane::from_preset(Plane::Front);
        self.guide.depth = 0.0;
        self.selected.clear();
        self.undo.clear();
        self.redo.clear();
        self.mirror_ui.enabled = true;
        self.line.constraint = line_tool::Constraint::Free;
        self.brush = Brush::default();
        self.radius = 0.025;
        self.color = [0.08, 0.72, 0.78];
        self.stabilization = 0.0;
        self.select_mode = false;
        self.eraser = false;
        self.line.enabled = false;
        self.shape_tool.mode = None;
        self.curve_tool.enabled = false;
        self.fills.enabled = false;

        let camera = self.camera;
        let screen = |world: Vec3| {
            crate::gizmo::project(&camera, rect, world)
                .expect("mirror-shapes fixture projects into the canvas")
                .0
        };
        let line_start_world = Vec3::new(-1.35, -0.55, 0.0);
        let line_end_world = Vec3::new(-0.25, 0.5, 0.0);
        let line_start = screen(line_start_world);
        let line_end = screen(line_end_world);
        self.start_line_tool();
        self.line_input(line_start, 1.0, None, rect, true);
        self.line_input(line_end, 1.0, None, rect, false);
        self.release_line(line_end, rect);
        assert_eq!(self.doc.strokes.len(), 2, "mirrored line commits both runs");
        let mirror_plane = crate::mirror::MirrorPlane {
            point: Vec3::ZERO,
            normal: Vec3::X,
        };
        let (source, reflected) = (&self.doc.strokes[0], &self.doc.strokes[1]);
        assert_eq!(source.points.len(), 2);
        assert_eq!(reflected.points.len(), 2);
        for (source_point, reflected_point) in source.points.iter().zip(&reflected.points) {
            assert!(
                reflected_point
                    .position
                    .distance(mirror_plane.reflect_point(source_point.position).unwrap())
                    < 0.0001,
                "captured mirror line matches production reflection geometry"
            );
        }

        self.line.enabled = false;
        self.start_shape_tool(shape_tool::ShapeMode::Ellipse);
        let ellipse_start = screen(Vec3::new(0.48, -1.5, 0.0));
        let ellipse_end = screen(Vec3::new(1.25, -0.65, 0.0));
        self.shape_input(ellipse_start, 1.0, rect, true);
        self.shape_input(ellipse_end, 1.0, rect, false);
        assert!(self.shape_draft_active(), "ellipse creates a live preview");
        self.release_shape(ellipse_end, 1.0, rect);
        assert_eq!(
            self.doc.strokes.len(),
            3,
            "ellipse commits after the mirror pair"
        );
        let ellipse = self.doc.strokes.last().unwrap();
        assert!(ellipse.points.len() > 8);
        assert!(ellipse
            .points
            .iter()
            .all(|point| point.position.z.abs() < 0.001));
        self.shape_tool.mode = None;
        self.status =
            "Mirrored line and editable ellipse prepared on the front guide; synthetic orbit reveals the layout.".into();
        self.dirty = true;
        self.renderer.invalidate();
        self.doc.strokes.len()
    }

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

        self.configure_radial_verification(plane_origin, 3, 17.0);
        let radial_history_before = self.undo.len();
        replay_mouse(self, ctx, rect, line_start, line_end, false);
        assert!(
            self.line.draft.is_some(),
            "radial line drag previews its source"
        );
        assert_eq!(
            self.doc.strokes, line_committed,
            "radial preview leaves ink untouched"
        );
        let radial_source = self.line.draft.as_ref().unwrap().mark.clone();
        let radial_preview = self
            .symmetry_copies_for(&radial_source)
            .expect("radial copies build on the rotated active guide");
        assert_eq!(
            radial_preview.len(),
            3,
            "radial preview contains three additional marks"
        );
        assert!(radial_preview.iter().all(|copy| {
            let mut copy_brush = copy.brush;
            let mut source_brush = radial_source.brush;
            copy_brush.normal = source_brush.normal;
            copy_brush.tangent = source_brush.tangent;
            copy_brush.angle_degrees = source_brush.angle_degrees;
            source_brush.normal = copy_brush.normal;
            source_brush.tangent = copy_brush.tangent;
            source_brush.angle_degrees = copy_brush.angle_degrees;
            copy.points.len() == radial_source.points.len()
                && copy.color == radial_source.color
                && copy.radius == radial_source.radius
                && copy_brush == source_brush
                && (copy.brush.normal.length() - 1.0).abs() < 1.0e-4
                && (copy.brush.tangent.length() - 1.0).abs() < 1.0e-4
                && copy
                    .points
                    .iter()
                    .all(|point| (point.position - plane_origin).dot(normal).abs() < 0.0001)
        }));

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
        assert!(
            self.line.draft.is_none(),
            "Escape cancels the radial preview"
        );
        assert_eq!(
            self.doc.strokes, line_committed,
            "radial cancel leaves no copies"
        );
        assert_eq!(
            self.undo.len(),
            radial_history_before,
            "radial cancel adds no history"
        );

        replay_mouse(self, ctx, rect, line_start, line_end, true);
        assert!(
            self.line.draft.is_none(),
            "mouse release commits the radial fan"
        );
        assert_eq!(self.doc.strokes.len(), line_committed.len() + 4);
        assert_eq!(
            self.undo.len(),
            radial_history_before + 1,
            "fan is one undo edit"
        );
        let mut committed_radial_source = self.doc.strokes[line_committed.len()].clone();
        committed_radial_source.id = radial_source.id;
        assert_eq!(committed_radial_source, radial_source);
        let mut committed_radial_copies = self.doc.strokes[line_committed.len() + 1..].to_vec();
        for (committed, preview) in committed_radial_copies.iter_mut().zip(&radial_preview) {
            committed.id = preview.id;
        }
        assert_eq!(
            committed_radial_copies, radial_preview,
            "committed copies exactly match preview geometry"
        );
        let radial_committed = self.doc.clone();
        let radial_glb = crate::export::encode(&self.doc, None).expect("radial GLB export");
        assert_eq!(radial_glb.summary.strokes, radial_committed.strokes.len());
        let radial_dir = tempfile::tempdir_in(std::env::current_dir().unwrap())
            .expect("workspace-local radial verification directory");
        let recovery_dir = radial_dir.path().join("recovery");
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "radial-verification".into(),
            None,
            radial_committed.clone(),
        );
        crate::recovery_store::write(&recovery_dir, &snapshot)
            .expect("radial recovery snapshot writes");
        let recovered = crate::recovery_store::scan(&recovery_dir, "verifier-reader")
            .expect("radial recovery snapshot scans");
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            serde_json::to_value(&recovered.candidates[0].snapshot.document).unwrap(),
            serde_json::to_value(&radial_committed).unwrap(),
            "recovery preserves the complete radial drawing"
        );
        let obj_path = radial_dir.path().join("radial.obj");
        crate::obj_export::write_obj_bundle(&self.doc, None, &obj_path, false)
            .expect("radial OBJ export");
        let obj = std::fs::read_to_string(obj_path).expect("radial OBJ reads");
        assert_eq!(obj.matches("\ng stroke_").count(), self.doc.strokes.len());

        // Generated copies are ordinary strokes after commit: use the normal
        // canvas picker to select one copy, then change only its appearance.
        let pick_doc = &self.doc;
        let pick_camera = self.camera;
        let pick_candidates: Vec<_> = radial_committed
            .strokes
            .iter()
            .skip(line_committed.len() + 1)
            .take(radial_preview.len())
            .flat_map(|copy| {
                copy.points
                    .windows(2)
                    .flat_map(|pair| {
                        (1..10).map(move |step| {
                            pair[0].position.lerp(pair[1].position, step as f32 / 10.0)
                        })
                    })
                    .filter_map(move |world| {
                        let (screen, _) = crate::gizmo::project(&pick_camera, rect, world)?;
                        Some((
                            copy.id,
                            screen,
                            crate::gizmo::pick(pick_doc, &pick_camera, rect, screen),
                        ))
                    })
            })
            .collect();
        let (edited_copy_id, pick_screen) = pick_candidates
            .iter()
            .find_map(|(id, screen, picked)| (*picked == Some(*id)).then_some((*id, *screen)))
            .unwrap_or_else(|| {
                panic!("no generated copy was independently picked: {pick_candidates:?}")
            });
        self.line.enabled = false;
        self.select_mode = true;
        self.individual_selection = true;
        replay_mouse(self, ctx, rect, pick_screen, pick_screen, true);
        assert_eq!(
            self.selected,
            HashSet::from([edited_copy_id]),
            "canvas picking selects one radial copy independently"
        );
        let before_edit = crate::editing::snapshot(&self.doc, &self.selected);
        assert_eq!(before_edit.len(), 1);
        let changed_color = [0.91, 0.17, 0.33];
        let after_edit = crate::editing::appearance_preview(
            &before_edit,
            crate::editing::AppearancePatch {
                color: Some(changed_color),
                ..Default::default()
            },
        )
        .expect("selected radial copy accepts normal appearance editing");
        let appearance_edit = crate::editing::Edit::Change {
            before: before_edit,
            after: after_edit,
        };
        appearance_edit.apply(&mut self.doc, true);
        self.undo.push(appearance_edit);
        self.redo.clear();
        assert_eq!(
            self.doc
                .strokes
                .iter()
                .find(|stroke| stroke.id == edited_copy_id)
                .unwrap()
                .color,
            changed_color,
            "individual radial copy appearance edit applies"
        );
        assert!(radial_committed
            .strokes
            .iter()
            .filter(|stroke| stroke.id != edited_copy_id)
            .all(|expected| self
                .doc
                .strokes
                .iter()
                .find(|actual| actual.id == expected.id)
                == Some(expected)));
        self.undo();
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&radial_committed).unwrap(),
            "undo restores the radial drawing after an individual copy edit"
        );
        self.redo();
        self.undo();
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&radial_committed).unwrap(),
            "individual edit redo and undo preserve the original radial fan"
        );
        self.select_mode = false;

        self.undo();
        assert_eq!(
            self.doc.strokes, line_committed,
            "one undo removes the entire fan"
        );
        self.redo();
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&radial_committed).unwrap(),
            "redo restores all radial copies"
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
            "Saved-guide mirror, rotated-plane line, and radial symmetry pointer checks passed."
                .into();
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
