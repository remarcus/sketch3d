use super::*;
use crate::geometry::Plane;

impl SketchApp {
    fn verify_line_art_button_click(&mut self, canvas: Rect, action: super::LineArtCommand) {
        let ui_context = egui::Context::default();
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 940.0));
        let raw_input = |events| egui::RawInput {
            screen_rect: Some(screen_rect),
            focused: true,
            events,
            ..Default::default()
        };
        let _ = ui_context.run(raw_input(Vec::new()), |ctx| {
            self.show_obj_reference_window(ctx)
        });
        let position = match action {
            super::LineArtCommand::Create => self.line_art_verification_preview_rect,
            super::LineArtCommand::Update => self.line_art_verification_update_rect,
            super::LineArtCommand::Bake => self.line_art_verification_bake_rect,
            super::LineArtCommand::Cancel => unreachable!(),
        }
        .expect("line-art action button is laid out in the production OBJ panel")
        .center();
        for events in [
            vec![egui::Event::PointerMoved(position)],
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            let _ = ui_context.run(raw_input(events), |ctx| self.show_obj_reference_window(ctx));
        }
        let command = self
            .line_art_command
            .take()
            .expect("synthetic pointer click reaches the line-art action");
        assert!(
            matches!(
                (action, command),
                (super::LineArtCommand::Create, super::LineArtCommand::Create)
                    | (super::LineArtCommand::Update, super::LineArtCommand::Update)
                    | (super::LineArtCommand::Bake, super::LineArtCommand::Bake)
            ),
            "the clicked line-art button dispatches the requested action"
        );
        self.line_art_command = Some(command);
        self.process_line_art_command(canvas);
    }

    pub(super) fn verify_obj_reference_window_import_click(&mut self, screen_rect: Rect) {
        let ui_context = egui::Context::default();
        let raw_input = |events| egui::RawInput {
            screen_rect: Some(screen_rect),
            focused: true,
            events,
            ..Default::default()
        };
        let _ = ui_context.run(raw_input(Vec::new()), |ctx| {
            self.show_obj_reference_window(ctx);
        });
        let import_rect = self
            .obj_reference_verification_import_rect
            .expect("OBJ import control is rendered in the actual references window");
        assert!(screen_rect.contains(import_rect.center()));
        let position = import_rect.center();

        for events in [
            vec![egui::Event::PointerMoved(position)],
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            let _ = ui_context.run(raw_input(events), |ctx| {
                self.show_obj_reference_window(ctx);
            });
        }
        assert!(
            self.obj_reference_verification_button_clicked,
            "synthetic pointer move, press, and release reaches the OBJ import handler"
        );
    }

    pub(super) fn verify_obj_references(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.obj_reference_verification_frame else {
            return;
        };
        match frame {
            0 => {
                self.recovery_scan_seen = true;
                self.recovery_prompt = false;
                self.doc = Document::default();
                self.undo.clear();
                self.redo.clear();
                self.selected.clear();
                self.selected_model_reference = None;
                self.select_mode = true;
                self.camera.target = Vec3::ZERO;
                self.camera.align(Plane::Front);
                self.camera.distance = 6.0;

                let asset_root = tempfile::tempdir_in(
                    std::env::current_dir()
                        .expect("workspace directory")
                        .join("verification"),
                )
                .expect("OBJ drawing asset root");
                self.image_asset_root = asset_root.path().to_owned();
                self.obj_reference_verification_asset_root = Some(asset_root);

                let source_root = tempfile::tempdir().expect("OBJ source fixture directory");
                let source = source_root.path().join("viewport-fixture.obj");
                std::fs::write(
                    &source,
                    "o Viewport fixture\nv 2 1 0\nv 6 1 0\nv 4 5 0\nf 1 2 3\n",
                )
                .expect("OBJ source fixture writes");
                self.obj_reference_verification_source_root = Some(source_root);
                self.obj_reference_window_open = true;

                let document_before = serde_json::to_vec(&self.doc).unwrap();
                let history_before = (self.undo.len(), self.redo.len());
                let dirty_before = self.dirty;
                let status_before = self.status.clone();
                self.obj_reference_verification_source = Some(None);
                self.verify_obj_reference_window_import_click(ctx.input(|input| input.screen_rect));
                assert_eq!(serde_json::to_vec(&self.doc).unwrap(), document_before);
                assert_eq!((self.undo.len(), self.redo.len()), history_before);
                assert_eq!(self.dirty, dirty_before);
                assert_eq!(self.status, status_before);
                assert!(self.doc.model_references.is_empty());
                assert!(self.obj_reference_models.is_empty());
                assert!(self.obj_reference_window_open);
                assert!(self.obj_reference_verification_source.is_none());
                assert!(
                    self.obj_reference_verification_source_root.is_some(),
                    "canceling the picker leaves its source fixture available for the retry"
                );
                self.obj_reference_verification_button_clicked = false;
                self.obj_reference_verification_source = Some(Some(source));
                self.verify_obj_reference_window_import_click(ctx.input(|input| input.screen_rect));
            }
            1 => {
                assert_eq!(
                    self.doc.model_references.len(),
                    1,
                    "the clicked import button installs one OBJ: {}",
                    self.status
                );
                assert!(
                    self.obj_reference_verification_button_clicked,
                    "synthetic pointer press/release reaches the OBJ import button handler"
                );
                assert_eq!(
                    self.obj_reference_models.len(),
                    1,
                    "imported mesh is loaded"
                );
                assert!(self.obj_reference_verification_source.is_none());
                assert!(self.obj_reference_verification_source_root.is_some());
                assert!(self.obj_reference_verification_asset_root.is_some());
                let resource = self.doc.model_references[0].clone();
                let local_bounds = {
                    let model = self
                        .obj_reference_models
                        .get(&resource.id)
                        .expect("imported model remains available to the viewport");
                    assert_eq!(model.triangles.len(), 1);
                    model.bounds
                };
                let reloaded = crate::obj_reference_storage::load_bundle(
                    &self.image_asset_root,
                    &resource.asset,
                )
                .expect("drawing-local OBJ bundle reloads after import");
                assert_eq!(reloaded.triangles.len(), 1);
                assert_eq!(reloaded.bounds, local_bounds);
                assert!(
                    self.renderer.has_obj_reference_model(resource.id),
                    "the import is installed in the scene renderer"
                );

                let source_center = (local_bounds[0] + local_bounds[1]) * 0.5;
                let hit = resource.center
                    + resource.rotation
                        * ((source_center + Vec3::new(0.0, -1.0, 0.0)) * resource.scale);
                let screen = crate::gizmo::project(&self.camera, rect, hit)
                    .expect("OBJ hit projects into the viewport")
                    .0;
                self.select_pointer(screen, rect, true, false);
                assert_eq!(
                    self.selected_model_reference,
                    Some(resource.id),
                    "the canvas selection handler picks the imported OBJ"
                );
                let bounds = self
                    .selected_obj_bounds()
                    .expect("selected visible OBJ has transform bounds");
                assert_eq!(bounds.len(), 8);
                assert!(bounds.iter().all(|point| point.is_finite()));
                let expected_min =
                    resource.center + resource.rotation * (local_bounds[0] * resource.scale);
                let expected_max =
                    resource.center + resource.rotation * (local_bounds[1] * resource.scale);
                let actual_min = bounds.iter().copied().reduce(Vec3::min).unwrap();
                let actual_max = bounds.iter().copied().reduce(Vec3::max).unwrap();
                assert!(actual_min.abs_diff_eq(expected_min.min(expected_max), 1.0e-5));
                assert!(actual_max.abs_diff_eq(expected_min.max(expected_max), 1.0e-5));

                let history_len = self.undo.len();
                self.verify_line_art_button_click(rect, super::LineArtCommand::Create);
                let preview_count = self
                    .line_art_draft
                    .as_ref()
                    .expect("Preview button creates a draft")
                    .segments
                    .len();
                assert!(preview_count > 0);
                assert!(
                    self.doc.strokes.is_empty(),
                    "preview remains detached from the document"
                );
                self.camera.yaw += 0.15;
                self.verify_line_art_button_click(rect, super::LineArtCommand::Update);
                assert_eq!(self.line_art_draft.as_ref().unwrap().source_id, resource.id);
                self.verify_line_art_button_click(rect, super::LineArtCommand::Bake);
                assert_eq!(self.doc.strokes.len(), preview_count);
                assert_eq!(self.undo.len(), history_len + 1, "Bake is one undo edit");
                let baked_strokes = self.doc.strokes.clone();
                self.undo();
                assert!(
                    self.doc.strokes.is_empty(),
                    "one undo removes the complete baked batch"
                );
                self.redo();
                assert_eq!(self.doc.strokes, baked_strokes);
                let remaining_models = std::mem::take(&mut self.doc.model_references);
                assert_eq!(
                    self.doc.strokes, baked_strokes,
                    "removing the source leaves baked strokes intact"
                );
                self.doc.model_references = remaining_models;

                self.status =
                    "OBJ import and production line-art preview, update, bake, undo, and source-removal checks passed.".into();
                self.dirty = false;
                self.renderer.invalidate();
                self.obj_reference_verification_frame = Some(4);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            _ => {}
        }
        self.obj_reference_verification_frame = Some(frame + 1);
    }
}
