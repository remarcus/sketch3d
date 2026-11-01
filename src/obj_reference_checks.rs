use super::*;
use crate::geometry::Plane;

impl SketchApp {
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
                self.obj_reference_verification_source = Some(source);
                self.obj_reference_verification_source_root = Some(source_root);
                self.obj_reference_window_open = true;

                let import_rect = self
                    .obj_reference_verification_import_rect
                    .expect("OBJ import button was rendered");
                assert!(rect.contains(import_rect.center()));
                self.import_obj_reference();
            }
            1 => {
                assert_eq!(
                    self.doc.model_references.len(),
                    1,
                    "the import action installs one OBJ: {}",
                    self.status
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

                self.status = "OBJ viewport import and picking checks passed.".into();
                self.dirty = false;
                self.renderer.invalidate();
                self.obj_reference_verification_frame = Some(2);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            _ => {}
        }
        self.obj_reference_verification_frame = Some(frame + 1);
    }
}
