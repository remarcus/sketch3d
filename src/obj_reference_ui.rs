use super::*;
use crate::document::model_references::ObjReferenceResource;
use std::path::Path;

fn line_art_point_visible(
    camera: &crate::geometry::Camera,
    canvas: egui::Rect,
    point: glam::Vec3,
    resources: &[ObjReferenceResource],
    models: &std::collections::HashMap<u64, crate::obj_reference::ObjModel>,
    indices: &mut std::collections::HashMap<u64, crate::obj_mesh_index::ObjMeshIndex>,
) -> bool {
    let Some((screen, _)) = crate::gizmo::project(camera, canvas, point) else {
        return false;
    };
    let (origin, direction) = crate::gizmo::ray(camera, canvas, screen);
    let expected = (point - origin).dot(direction);
    nearest_obj_reference_depth(origin, direction, resources, models, indices)
        .is_none_or(|depth| depth + 0.002_f32.max(expected.abs() * 1.0e-4) >= expected)
}

impl SketchApp {
    pub(super) fn process_line_art_command(&mut self, canvas: egui::Rect) {
        let Some(command) = self.line_art_command.take() else {
            return;
        };
        match command {
            LineArtCommand::Cancel => {
                self.line_art_draft = None;
                self.status = "Line-art preview canceled".into();
            }
            LineArtCommand::Create | LineArtCommand::Update => {
                let source_id = if matches!(command, LineArtCommand::Update) {
                    self.line_art_draft.as_ref().map(|draft| draft.source_id)
                } else {
                    self.selected_model_reference
                };
                let Some(source_id) = source_id else {
                    self.status = "Select an OBJ reference to preview line art".into();
                    return;
                };
                let Some(resource) = self
                    .doc
                    .model_references
                    .iter()
                    .find(|item| item.id == source_id && item.visible)
                    .cloned()
                else {
                    self.line_art_draft = None;
                    self.status = "The source model is hidden or unavailable".into();
                    return;
                };
                let Some(model) = self.obj_reference_models.get(&source_id) else {
                    self.status = "The source OBJ model is not loaded".into();
                    return;
                };
                let settings = crate::model_line_art::Settings {
                    crease_angle_degrees: self.line_art_crease_angle,
                    boundaries: self.line_art_boundaries,
                    creases: self.line_art_creases,
                    silhouettes: self.line_art_silhouettes,
                };
                match crate::model_line_art::extract(
                    model,
                    &self.camera,
                    resource.center,
                    resource.rotation,
                    resource.scale,
                    settings,
                ) {
                    Ok(extraction) => {
                        let extraction_truncated = extraction.truncated;
                        let local_segments = extraction.segments;
                        let world_segments = local_segments
                            .into_iter()
                            .map(|line| {
                                line.map(|point| {
                                    resource.center + resource.rotation * (point * resource.scale)
                                })
                            })
                            .collect::<Vec<_>>();
                        let mut indices = std::mem::take(&mut self.obj_surface_indices);
                        let visible =
                            crate::model_line_art::filter_visible(world_segments, |sample| {
                                line_art_point_visible(
                                    &self.camera,
                                    canvas,
                                    sample,
                                    &self.doc.model_references,
                                    &self.obj_reference_models,
                                    &mut indices,
                                )
                            });
                        self.obj_surface_indices = indices;
                        let truncated = extraction_truncated || visible.truncated;
                        let visible_count = visible.segments.len();
                        self.line_art_draft = Some(crate::model_line_art::Draft {
                            source_id,
                            segments: visible.segments,
                            truncated,
                        });
                        self.status = if truncated {
                            format!("Preview truncated at the 20,000-stroke limit ({visible_count} visible strokes)")
                        } else {
                            format!("Previewed {visible_count} visible line-art strokes")
                        };
                    }
                    Err(error) => self.status = format!("Line-art preview failed: {error}"),
                }
            }
            LineArtCommand::Bake => {
                let Some(draft) = self.line_art_draft.take() else {
                    return;
                };
                if draft.segments.is_empty() {
                    self.status = "There is no line art to bake".into();
                    return;
                }
                if self
                    .next_id
                    .checked_add(draft.segments.len() as u64)
                    .is_none_or(|end| end == u64::MAX)
                {
                    self.status = "No more stroke IDs are available".into();
                    self.line_art_draft = Some(draft);
                    return;
                }
                let mut id = self.next_id;
                let strokes = draft
                    .segments
                    .into_iter()
                    .map(|line| {
                        let stroke = crate::document::Stroke {
                            model_surface: None,
                            fill: None,
                            material: crate::document::StrokeMaterial::Unlit,
                            group_id: None,
                            layer_id: self.doc.active_layer,
                            id,
                            color: [0.035, 0.035, 0.04],
                            gradient: None,
                            radius: 0.006,
                            brush: crate::document::Brush::default(),
                            points: line
                                .into_iter()
                                .map(|position| crate::document::Point {
                                    position,
                                    pressure: 1.0,
                                    tilt: None,
                                    time_ms: 0,
                                })
                                .collect(),
                            point_thickness: None,
                            modifiers: None,
                            curve: None,
                        };
                        id += 1;
                        stroke
                    })
                    .collect::<Vec<_>>();
                let edit = Edit::AddBatch(strokes);
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.next_id = id;
                self.dirty = true;
                self.record_recovery_edit_labeled("Bake model line art");
                self.renderer.invalidate();
                self.status = "Baked line art as independent strokes (one undo step)".into();
            }
        }
    }

    pub(super) fn paint_line_art_preview(&self, painter: &egui::Painter, canvas: egui::Rect) {
        let Some(draft) = &self.line_art_draft else {
            return;
        };
        if !self
            .doc
            .model_references
            .iter()
            .any(|resource| resource.id == draft.source_id && resource.visible)
        {
            return;
        }
        for line in &draft.segments {
            if let (Some((a, _)), Some((b, _))) = (
                crate::gizmo::project(&self.camera, canvas, line[0]),
                crate::gizmo::project(&self.camera, canvas, line[1]),
            ) {
                painter.line_segment(
                    [a, b],
                    egui::Stroke::new(
                        1.5_f32,
                        egui::Color32::from_rgba_unmultiplied(255, 185, 55, 210),
                    ),
                );
            }
        }
    }

    pub(super) fn show_obj_reference_window(&mut self, ctx: &egui::Context) {
        if !self.obj_reference_window_open {
            return;
        }
        let mut open = self.obj_reference_window_open;
        egui::Window::new("OBJ references")
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| self.obj_reference_controls(ui));
        self.obj_reference_window_open = open;
    }

    pub(super) fn pick_obj_reference(
        &mut self,
        rect: egui::Rect,
        position: egui::Pos2,
    ) -> Option<u64> {
        let (origin, direction) = crate::gizmo::ray(&self.camera, rect, position);
        nearest_obj_reference_hit(
            origin,
            direction,
            &self.doc.model_references,
            &self.obj_reference_models,
            &mut self.obj_surface_indices,
        )
    }

    pub(super) fn selected_obj_bounds(&self) -> Option<[glam::Vec3; 8]> {
        let id = self.selected_model_reference?;
        let resource = self
            .doc
            .model_references
            .iter()
            .find(|resource| resource.id == id && resource.visible)?;
        let model = self.obj_reference_models.get(&id)?;
        Some(std::array::from_fn(|index| {
            let local = glam::Vec3::new(
                if index & 1 == 0 {
                    model.bounds[0].x
                } else {
                    model.bounds[1].x
                },
                if index & 2 == 0 {
                    model.bounds[0].y
                } else {
                    model.bounds[1].y
                },
                if index & 4 == 0 {
                    model.bounds[0].z
                } else {
                    model.bounds[1].z
                },
            );
            resource.center + resource.rotation * (local * resource.scale)
        }))
    }

    pub(super) fn obj_reference_controls(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("OBJ references")
            .default_open(true)
            .show(ui, |ui| {
            let import_response = import_obj_reference_button(ui);
            let capability_import = self
                .capability_replay
                .as_ref()
                .is_some_and(|replay| replay.clip.is_reference_export());
            if self.obj_reference_verification_frame.is_some() || capability_import {
                self.obj_reference_verification_import_rect = Some(import_response.rect);
            }
            if import_response.clicked() {
                if self.obj_reference_verification_frame.is_some() || capability_import {
                    self.obj_reference_verification_button_clicked = true;
                }
                self.import_obj_reference();
            }
            ui.small("OBJ models are read-only references. Whole-scene GLB includes visible models; selected-stroke export remains ink-only. Supported MTL properties: Kd diffuse color and d/Tr opacity. Texture maps and other MTL properties are ignored and listed in the GLB metadata.");

            ui.separator();
            ui.strong("Model line art");
            ui.horizontal(|ui| {
                ui.label("Crease angle");
                ui.add(egui::Slider::new(&mut self.line_art_crease_angle, 0.0..=180.0).suffix("°"));
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.line_art_boundaries, "Boundaries");
                ui.checkbox(&mut self.line_art_creases, "Creases");
                ui.checkbox(&mut self.line_art_silhouettes, "Silhouettes");
            });
            ui.horizontal(|ui| {
                let preview = ui.button("Preview line art");
                if self.obj_reference_verification_frame.is_some() {
                    self.line_art_verification_preview_rect = Some(preview.rect);
                }
                if preview.clicked() {
                    self.line_art_command = Some(LineArtCommand::Create);
                }
                if self.line_art_draft.is_some() {
                    let update = ui.button("Update preview");
                    let bake = ui.button("Bake");
                    if self.obj_reference_verification_frame.is_some() {
                        self.line_art_verification_update_rect = Some(update.rect);
                        self.line_art_verification_bake_rect = Some(bake.rect);
                    }
                    if update.clicked() {
                        self.line_art_command = Some(LineArtCommand::Update);
                    }
                    if bake.clicked() {
                        self.line_art_command = Some(LineArtCommand::Bake);
                    }
                    if ui.button("Cancel").clicked() {
                        self.line_art_command = Some(LineArtCommand::Cancel);
                    }
                }
            });
            if self.line_art_draft.is_some() {
                ui.small("Preview is detached. Update recomputes from the source model and current camera. Bake creates independent strokes in one undo step; removing the model keeps baked strokes.");
                if self.line_art_draft.as_ref().is_some_and(|draft| draft.truncated) {
                    ui.colored_label(ui_style::colors(ui.ctx()).warning, "This preview reached the 20,000-stroke cap and is incomplete.");
                }
            } else {
                ui.small("Visible models only; extraction is capped at 250,000 triangles, 750,000 candidate edges, and 20,000 output strokes.");
            }

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
                        ui.colored_label(ui_style::colors(ui.ctx()).error, error);
                    }
                });
                changed |= resource_changed;
                geometry_changed |= resource_changed;
            }
            if let Some(index) = remove {
                let resource = self.doc.model_references.remove(index);
                if self.line_art_draft.as_ref().is_some_and(|draft| draft.source_id == resource.id) {
                    self.line_art_draft = None;
                }
                if self.selected_model_reference == Some(resource.id) {
                    self.selected_model_reference = None;
                }
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

    pub(super) fn import_obj_reference(&mut self) {
        let path = self
            .obj_reference_verification_source
            .take()
            .unwrap_or_else(|| {
                rfd::FileDialog::new()
                    .add_filter("Wavefront OBJ", &["obj"])
                    .pick_file()
            });
        let Some(path) = path else {
            return;
        };
        self.import_obj_reference_from_path(&path);
    }

    fn import_obj_reference_from_path(&mut self, path: &Path) {
        if self.doc.model_references.len()
            >= crate::document::model_references::MAX_MODEL_REFERENCES
        {
            self.status = "This drawing already has the maximum number of OBJ references.".into();
            return;
        }
        let mut id = self.next_id;
        let model_root = self.image_asset_root.join("models");
        while id != 0 && id != u64::MAX && model_root.join(id.to_string()).exists() {
            id = id.saturating_add(1);
        }
        if id == 0 || id == u64::MAX {
            self.status = "No more resource IDs are available.".into();
            return;
        }
        let (asset, model) = match crate::obj_reference_storage::import_source_bundle(
            path,
            &self.image_asset_root,
            id,
        ) {
            Ok(imported) => imported,
            Err(error) => {
                self.status = format!("OBJ import failed: {error}");
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
        let triangle_count = model.triangles.len();
        let material_count = model.materials.len();
        let warning_count = model.warnings.len();
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
        self.obj_reference_models.insert(id, model);
        self.next_id = id + 1;
        self.refresh_obj_reference_meshes();
        self.undo.push(Edit::ModelReferences {
            before,
            after: self.model_reference_edit_state(),
        });
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit_labeled("Import OBJ reference");
        self.renderer.invalidate();
        self.status = format!(
            "Imported {} triangles and {} materials. Unsupported MTL features: {}.",
            triangle_count, material_count, warning_count
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

fn nearest_obj_reference_hit(
    origin: glam::Vec3,
    direction: glam::Vec3,
    resources: &[ObjReferenceResource],
    models: &std::collections::HashMap<u64, crate::obj_reference::ObjModel>,
    indices: &mut std::collections::HashMap<u64, crate::obj_mesh_index::ObjMeshIndex>,
) -> Option<u64> {
    nearest_obj_reference_hit_with_depth(origin, direction, resources, models, indices)
        .map(|(_, id)| id)
}

pub(super) fn nearest_obj_reference_depth(
    origin: glam::Vec3,
    direction: glam::Vec3,
    resources: &[ObjReferenceResource],
    models: &std::collections::HashMap<u64, crate::obj_reference::ObjModel>,
    indices: &mut std::collections::HashMap<u64, crate::obj_mesh_index::ObjMeshIndex>,
) -> Option<f32> {
    nearest_obj_reference_hit_with_depth(origin, direction, resources, models, indices)
        .map(|(distance, _)| distance)
}

fn nearest_obj_reference_hit_with_depth(
    origin: glam::Vec3,
    direction: glam::Vec3,
    resources: &[ObjReferenceResource],
    models: &std::collections::HashMap<u64, crate::obj_reference::ObjModel>,
    indices: &mut std::collections::HashMap<u64, crate::obj_mesh_index::ObjMeshIndex>,
) -> Option<(f32, u64)> {
    let mut nearest: Option<(f32, u64)> = None;
    for resource in resources.iter().filter(|resource| resource.visible) {
        let id = resource.id;
        let Some(model) = models.get(&id) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(entry) = indices.entry(id) {
            let Ok(index) = crate::obj_mesh_index::ObjMeshIndex::new(model) else {
                continue;
            };
            entry.insert(index);
        }
        let inverse = resource.rotation.inverse();
        let local_origin = inverse * (origin - resource.center) / resource.scale;
        let local_direction = inverse * direction / resource.scale;
        let hit = indices
            .get(&id)
            .and_then(|index| index.hit(model, local_origin, local_direction));
        let Some(hit) = hit else {
            continue;
        };
        let world = resource.center + resource.rotation * (hit.position * resource.scale);
        let ray_t = (world - origin).dot(direction);
        if ray_t > 0.0 && nearest.is_none_or(|(best, _)| ray_t < best) {
            nearest = Some((ray_t, id));
        }
    }
    nearest
}

fn import_obj_reference_button(ui: &mut egui::Ui) -> egui::Response {
    ui.button("Import OBJ reference…")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_depth_hides_back_model_and_ignores_hidden_occluder() {
        let camera = crate::geometry::Camera {
            yaw: 0.0,
            pitch: 0.0,
            target: glam::Vec3::ZERO,
            distance: 5.0,
            ..Default::default()
        };
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));
        let plane = |z: f32| {
            let obj = format!("v -1 -1 {z}\nv 1 -1 {z}\nv 1 1 {z}\nv -1 1 {z}\nf 1 2 3\nf 1 3 4\n");
            crate::obj_reference::parse(obj.as_bytes(), Path::new("plane.obj")).unwrap()
        };
        let models = [(1, plane(2.0)), (2, plane(0.0))].into_iter().collect();
        let resource = |id, visible| ObjReferenceResource {
            id,
            name: format!("model {id}"),
            visible,
            locked: false,
            center: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: 1.0,
            asset: crate::obj_reference_storage::ObjReferenceAsset {
                id,
                bundle_dir: format!("models/{id}"),
            },
        };
        let mut resources = vec![resource(1, true), resource(2, true)];
        let mut indices = std::collections::HashMap::new();
        assert!(!line_art_point_visible(
            &camera,
            canvas,
            glam::vec3(0.0, 0.0, 0.0),
            &resources,
            &models,
            &mut indices
        ));
        assert!(line_art_point_visible(
            &camera,
            canvas,
            glam::vec3(0.0, 0.0, 2.0),
            &resources,
            &models,
            &mut indices
        ));
        resources[0].visible = false;
        assert!(line_art_point_visible(
            &camera,
            canvas,
            glam::vec3(0.0, 0.0, 0.0),
            &resources,
            &models,
            &mut indices
        ));
    }

    fn model() -> crate::obj_reference::ObjModel {
        crate::obj_reference::parse(
            b"v -1 -1 0\nv 1 -1 0\nv 0 1 0\nf 1 2 3\n",
            Path::new("test.obj"),
        )
        .unwrap()
    }

    fn render_import_button(ctx: &egui::Context, events: Vec<egui::Event>) -> (Rect, bool) {
        let mut response = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(640.0, 480.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let button = import_obj_reference_button(ui);
                    response = Some((button.rect, button.clicked()));
                });
            },
        );
        response.expect("import button renders")
    }

    #[test]
    fn import_obj_reference_button_responds_to_pointer_click() {
        let ctx = egui::Context::default();
        let (rect, _) = render_import_button(&ctx, Vec::new());
        let position = rect.center();
        let _ = render_import_button(&ctx, vec![egui::Event::PointerMoved(position)]);
        let _ = render_import_button(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        let (_, clicked) = render_import_button(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert!(clicked, "the import control responds to a pointer click");
    }

    fn resource(id: u64, center: glam::Vec3, scale: f32, visible: bool) -> ObjReferenceResource {
        ObjReferenceResource {
            id,
            name: format!("Model {id}"),
            visible,
            locked: false,
            center,
            rotation: glam::Quat::IDENTITY,
            scale,
            asset: crate::obj_reference_storage::ObjReferenceAsset {
                id,
                bundle_dir: format!("models/{id}"),
            },
        }
    }

    #[test]
    fn picking_uses_transformed_meshes_skips_hidden_models_and_selects_nearest_hit() {
        let mut resources = [
            {
                let mut resource = resource(1, glam::Vec3::new(2.0, 0.0, 0.0), 2.0, true);
                resource.rotation = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
                resource
            },
            {
                let mut resource = resource(2, glam::Vec3::new(4.0, 0.0, 0.0), 2.0, false);
                resource.rotation = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
                resource
            },
            {
                let mut resource = resource(3, glam::Vec3::new(0.0, 0.0, 0.0), 2.0, true);
                resource.rotation = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
                resource
            },
        ];
        let models = [(1, model()), (2, model()), (3, model())]
            .into_iter()
            .collect();
        let mut indices = std::collections::HashMap::new();

        let picked = nearest_obj_reference_hit(
            glam::Vec3::new(5.0, 0.0, 0.0),
            glam::Vec3::NEG_X,
            &resources,
            &models,
            &mut indices,
        );

        assert_eq!(picked, Some(1));
        assert_eq!(indices.len(), 2, "hidden model should not be indexed");

        resources[1].visible = true;
        assert_eq!(
            nearest_obj_reference_hit(
                glam::Vec3::new(5.0, 0.0, 0.0),
                glam::Vec3::NEG_X,
                &resources,
                &models,
                &mut indices,
            ),
            Some(2),
            "the closest visible transformed surface should win"
        );
        for resource in &mut resources {
            resource.visible = false;
        }
        assert_eq!(
            nearest_obj_reference_hit(
                glam::Vec3::new(5.0, 0.0, 0.0),
                glam::Vec3::NEG_X,
                &resources,
                &models,
                &mut indices,
            ),
            None
        );
    }
}
