use super::*;
use crate::document::model_references::ObjReferenceResource;
use std::path::Path;

impl SketchApp {
    pub(super) fn obj_reference_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("OBJ references", |ui| {
            if ui.button("Import OBJ reference…").clicked() {
                self.import_obj_reference();
            }
            ui.small("OBJ models are read-only references. They do not become strokes or GLB export geometry.");

            let before = self.model_reference_edit_state();
            let mut remove = None;
            let mut changed = false;
            let mut geometry_changed = false;
            let mut activate = None;
            let active_model_reference = self.doc.active_model_reference;
            for index in 0..self.doc.model_references.len() {
                let original = self.doc.model_references[index].clone();
                let mut resource_changed = false;
                ui.group(|ui| {
                    let resource = &mut self.doc.model_references[index];
                    ui.horizontal(|ui| {
                        ui.strong(&resource.name);
                        let mut active = active_model_reference == Some(resource.id);
                        if ui.checkbox(&mut active, "Draw here").changed() {
                            activate = Some((resource.id, active));
                        }
                        resource_changed |= ui.checkbox(&mut resource.visible, "Visible").changed();
                        resource_changed |= ui.checkbox(&mut resource.locked, "Locked").changed();
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                    });
                    ui.add_enabled_ui(!resource.locked, |ui| {
                        resource_changed |= transform_controls(ui, resource);
                    });
                    if let Err(error) = resource.validate() {
                        *resource = original.clone();
                        resource_changed = false;
                        ui.colored_label(Color32::LIGHT_RED, error);
                    }
                });
                changed |= resource_changed;
                geometry_changed |= resource_changed;
            }
            if let Some(index) = remove {
                let resource = self.doc.model_references.remove(index);
                if self.doc.active_model_reference == Some(resource.id) {
                    self.doc.active_model_reference = None;
                }
                changed = true;
            }
            if let Some((id, true)) = activate {
                if self.doc.model_references.iter().any(|resource| {
                    resource.id == id && resource.visible && !resource.locked
                }) {
                    self.doc.active_model_reference = Some(id);
                    self.doc.active_image_plane = None;
                    self.doc.active_guide = None;
                    for plane in &mut self.doc.image_planes {
                        plane.drawable = false;
                    }
                    changed = true;
                }
            } else if let Some((id, false)) = activate {
                if self.doc.active_model_reference == Some(id) {
                    self.doc.active_model_reference = None;
                    changed = true;
                }
            }
            if let Some(id) = self.doc.active_model_reference {
                if self.doc.model_references.iter().any(|resource| {
                    resource.id == id && (!resource.visible || resource.locked)
                }) {
                    self.doc.active_model_reference = None;
                    changed = true;
                }
            }
            if changed {
                self.undo.push(Edit::ModelReferences {
                    before,
                    after: self.model_reference_edit_state(),
                });
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.refresh_image_plane_textures();
                if geometry_changed {
                    self.refresh_obj_reference_meshes();
                }
                self.renderer.invalidate();
            }
        });
    }

    fn import_obj_reference(&mut self) {
        if self.doc.model_references.len()
            >= crate::document::model_references::MAX_MODEL_REFERENCES
        {
            self.status = "This drawing already has the maximum number of OBJ references.".into();
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Wavefront OBJ", &["obj"])
            .pick_file()
        else {
            return;
        };
        let model = match crate::obj_reference::load(&path) {
            Ok(model) => model,
            Err(error) => {
                self.status = format!("OBJ import failed: {error}");
                return;
            }
        };
        let mut id = self.next_id;
        let model_root = self.image_asset_root.join("models");
        while id != 0 && id != u64::MAX && model_root.join(id.to_string()).exists() {
            id = id.saturating_add(1);
        }
        if id == 0 || id == u64::MAX {
            self.status = "No more resource IDs are available.".into();
            return;
        }
        let asset =
            match crate::obj_reference_storage::import_bundle(&model, &self.image_asset_root, id) {
                Ok(asset) => asset,
                Err(error) => {
                    self.status = format!("OBJ asset import failed: {error}");
                    return;
                }
            };
        let bundled_model =
            match crate::obj_reference_storage::load_bundle(&self.image_asset_root, &asset) {
                Ok(model) => model,
                Err(error) => {
                    self.status = format!("OBJ asset verification failed: {error}");
                    let _ =
                        crate::obj_reference_storage::remove_bundle(&self.image_asset_root, &asset);
                    return;
                }
            };
        let size = model.bounds[1] - model.bounds[0];
        let extent = size.max_element();
        if !extent.is_finite() || extent <= 1.0e-5 {
            self.status = "OBJ geometry has no usable bounds.".into();
            return;
        }
        let scale = (2.0 / extent).clamp(0.001, 1000.0);
        let rotation = glam::Quat::IDENTITY;
        let center =
            self.camera.target - rotation * ((model.bounds[0] + model.bounds[1]) * 0.5 * scale);
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("OBJ reference")
            .chars()
            .take(crate::document::model_references::MAX_MODEL_NAME_CHARS)
            .collect::<String>()
            .trim()
            .to_owned();
        let mut name = if name.is_empty() {
            format!("OBJ {id}")
        } else {
            name
        };
        if self
            .doc
            .model_references
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            name = format!("OBJ {id}");
        }
        let resource = ObjReferenceResource {
            id,
            name,
            visible: true,
            locked: false,
            center,
            rotation,
            scale,
            asset,
        };
        if let Err(error) = resource.validate() {
            self.status = format!("OBJ resource is invalid: {error}");
            let _ = crate::obj_reference_storage::remove_bundle(
                &self.image_asset_root,
                &resource.asset,
            );
            return;
        }
        let before = self.model_reference_edit_state();
        self.doc.model_references.push(resource.clone());
        self.obj_reference_models.insert(id, bundled_model);
        self.next_id = id + 1;
        self.refresh_obj_reference_meshes();
        self.undo.push(Edit::ModelReferences {
            before,
            after: self.model_reference_edit_state(),
        });
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status = format!(
            "Imported {} triangles and {} materials. Unsupported MTL features: {}.",
            model.triangles.len(),
            model.materials.len(),
            model.warnings.len()
        );
    }

    pub(super) fn load_obj_reference_models(
        &self,
        asset_root: &Path,
    ) -> Result<std::collections::HashMap<u64, crate::obj_reference::ObjModel>, String> {
        self.doc
            .model_references
            .iter()
            .map(|resource| {
                crate::obj_reference_storage::load_bundle(asset_root, &resource.asset)
                    .map(|model| (resource.id, model))
            })
            .collect()
    }

    pub(super) fn copy_obj_reference_bundles(&self, destination: &Path) -> Result<(), String> {
        if same_asset_root(&self.image_asset_root, destination) {
            return Ok(());
        }
        for resource in &self.doc.model_references {
            let model = self
                .obj_reference_models
                .get(&resource.id)
                .ok_or_else(|| format!("OBJ reference ‘{}’ is not loaded", resource.name))?;
            crate::obj_reference_storage::import_bundle(model, destination, resource.id)?;
        }
        Ok(())
    }

    pub(super) fn append_obj_recovery_assets(
        &self,
        assets: &mut Vec<crate::recovery_store::RecoveryAsset>,
    ) -> bool {
        let mut total: usize = assets.iter().map(|asset| asset.bytes.len()).sum();
        for resource in &self.doc.model_references {
            let Some(model) = self.obj_reference_models.get(&resource.id) else {
                return false;
            };
            let directory = self.image_asset_root.join(&resource.asset.bundle_dir);
            let Ok(root) = self.image_asset_root.canonicalize() else {
                return false;
            };
            let Ok(directory_metadata) = std::fs::symlink_metadata(&directory) else {
                return false;
            };
            let Ok(canonical_directory) = directory.canonicalize() else {
                return false;
            };
            if !directory_metadata.is_dir()
                || directory_metadata.file_type().is_symlink()
                || !canonical_directory.starts_with(&root)
            {
                return false;
            }
            let mut names = vec!["model.obj".to_owned()];
            names.extend(
                (0..model.source_materials.len()).map(|index| format!("material-{index:03}.mtl")),
            );
            for name in names {
                let asset_file = format!("{}/{name}", resource.asset.bundle_dir);
                let path = directory.join(&name);
                let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                    return false;
                };
                let Ok(canonical_path) = path.canonicalize() else {
                    return false;
                };
                let limit = if name.ends_with(".obj") {
                    crate::obj_reference_storage::MAX_MODEL_OBJ_BYTES
                } else {
                    crate::obj_reference_storage::MAX_MODEL_MTL_BYTES
                };
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || !canonical_path.starts_with(&canonical_directory)
                    || metadata.len() > limit
                {
                    return false;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    return false;
                };
                total = total.saturating_add(bytes.len());
                if total > crate::recovery_store::MAX_ASSET_BYTES {
                    return false;
                }
                assets.push(crate::recovery_store::RecoveryAsset { asset_file, bytes });
            }
        }
        true
    }

    pub(super) fn refresh_obj_reference_meshes(&mut self) {
        self.renderer.clear_obj_reference_models();
        for resource in &self.doc.model_references {
            let Some(model) = self.obj_reference_models.get(&resource.id) else {
                continue;
            };
            if let Err(error) = self.renderer.set_obj_reference_model(
                resource.id,
                model,
                resource.center,
                resource.rotation,
                resource.scale,
            ) {
                self.status = format!("Cannot render OBJ reference: {error}");
            }
        }
    }

    fn model_reference_edit_state(&self) -> crate::editing::ModelReferenceState {
        (
            self.doc.model_references.clone(),
            self.doc.active_model_reference,
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
            self.doc.guide_resources.clone(),
            self.doc.active_guide,
        )
    }
}

fn transform_controls(ui: &mut egui::Ui, resource: &mut ObjReferenceResource) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Position");
        for axis in 0..3 {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut resource.center[axis])
                        .speed(0.01)
                        .range(-10_000.0..=10_000.0),
                )
                .changed();
        }
    });
    let mut degrees = super::guide_resource_ui::rotation_degrees(resource.rotation);
    ui.horizontal(|ui| {
        ui.label("Rotation°");
        for axis in 0..3 {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut degrees[axis])
                        .speed(0.5)
                        .range(-3600.0..=3600.0),
                )
                .changed();
        }
    });
    changed |= ui
        .add(egui::Slider::new(&mut resource.scale, 0.001..=10_000.0).text("Scale"))
        .changed();
    if changed {
        resource.rotation = glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            degrees.z.to_radians(),
            degrees.y.to_radians(),
            degrees.x.to_radians(),
        )
        .normalize();
    }
    changed
}

fn same_asset_root(first: &Path, second: &Path) -> bool {
    match (first.canonicalize(), second.canonicalize()) {
        (Ok(first), Ok(second)) => first == second,
        _ => first == second,
    }
}
