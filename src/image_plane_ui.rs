use super::*;

impl SketchApp {
    pub(super) fn refresh_image_plane_textures(&mut self) {
        self.renderer.clear_image_plane_assets();
        for plane in &self.doc.image_planes {
            if let Some(asset) = self.image_plane_assets.get(&plane.id) {
                let _ = self.renderer.set_image_plane_asset(plane.id, asset);
            }
        }
        self.renderer.invalidate();
    }

    fn commit_image_plane_edit(
        &mut self,
        before: (
            Vec<crate::image_planes::ImagePlane>,
            Option<u64>,
            Option<u64>,
        ),
    ) {
        let after = (
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
            self.doc.active_guide,
        );
        if before == after {
            return;
        }
        if self.doc.active_image_plane.is_some() {
            self.finish();
            self.cancel_line_tool();
            self.cancel_curve_tool();
            self.cancel_shape_draft();
            self.shape_tool.mode = None;
            self.cancel_fill_tool();
        }
        self.undo.push(Edit::ImagePlanes { before, after });
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
    }

    pub(super) fn image_recovery_assets(&self) -> Vec<crate::recovery_store::RecoveryAsset> {
        let mut result = Vec::new();
        let mut total = 0_u64;
        for plane in &self.doc.image_planes {
            let Ok(Some(path)) = plane.resolve_asset(&self.image_asset_root) else {
                continue;
            };
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            total = total.saturating_add(metadata.len());
            if metadata.len() > crate::image_planes::MAX_IMAGE_BYTES
                || total > crate::image_planes::MAX_TOTAL_ASSET_BYTES
            {
                return Vec::new();
            }
            if let Ok(bytes) = std::fs::read(path) {
                result.push(crate::recovery_store::RecoveryAsset {
                    asset_file: plane.asset_file.clone(),
                    bytes,
                });
            }
        }
        result
    }

    pub(super) fn install_recovery_image_assets(
        &mut self,
        assets: &[crate::recovery_store::RecoveryAsset],
    ) {
        for saved in assets {
            if let Some(plane) = self
                .doc
                .image_planes
                .iter()
                .find(|plane| plane.asset_file == saved.asset_file)
            {
                let path = self.image_asset_root.join(&plane.asset_file);
                if std::fs::create_dir_all(&self.image_asset_root).is_err() {
                    continue;
                }
                let Ok(mut temp) = tempfile::NamedTempFile::new_in(&self.image_asset_root) else {
                    continue;
                };
                if std::io::Write::write_all(&mut temp, &saved.bytes).is_err()
                    || temp.as_file().sync_all().is_err()
                    || temp.persist(&path).is_err()
                {
                    continue;
                }
                if let Ok(asset) = crate::image_planes::decode_asset(&path) {
                    if self
                        .renderer
                        .set_image_plane_asset(plane.id, &asset)
                        .is_ok()
                    {
                        self.image_plane_assets.insert(plane.id, asset);
                    }
                }
                continue;
            }
            let Some(resource) = self.doc.model_references.iter().find(|resource| {
                saved
                    .asset_file
                    .starts_with(&format!("{}/", resource.asset.bundle_dir))
            }) else {
                continue;
            };
            let relative = std::path::Path::new(&saved.asset_file);
            let components: Vec<_> = relative.components().collect();
            if components.len() != 3
                || !matches!(components[0], std::path::Component::Normal(name) if name == "models")
                || !matches!(components[1], std::path::Component::Normal(name) if name.to_string_lossy() == resource.id.to_string())
                || !matches!(components[2], std::path::Component::Normal(_))
            {
                continue;
            }
            let name = components[2].as_os_str().to_string_lossy();
            if name != "model.obj" && !(name.starts_with("material-") && name.ends_with(".mtl")) {
                continue;
            }
            let path = self.image_asset_root.join(relative);
            let Some(parent) = path.parent() else {
                continue;
            };
            if std::fs::create_dir_all(parent).is_err() {
                continue;
            }
            let Ok(mut temp) = tempfile::NamedTempFile::new_in(parent) else {
                continue;
            };
            if std::io::Write::write_all(&mut temp, &saved.bytes).is_err()
                || temp.as_file().sync_all().is_err()
                || temp.persist(&path).is_err()
            {
                continue;
            }
        }
        if let Ok(models) = self.load_obj_reference_models(&self.image_asset_root) {
            self.obj_reference_models = models;
            self.refresh_obj_reference_meshes();
        }
    }

    pub(super) fn image_plane_hit(
        &self,
        origin: Vec3,
        direction: Vec3,
    ) -> Option<(Vec3, glam::Quat)> {
        let id = self.doc.active_image_plane?;
        let plane = self
            .doc
            .image_planes
            .iter()
            .find(|plane| plane.id == id && plane.visible && !plane.locked && plane.drawable)?;
        let normal = plane.rotation * Vec3::Z;
        let denominator = normal.dot(direction);
        if denominator.abs() < 1e-6 {
            return None;
        }
        let distance = normal.dot(plane.origin - origin) / denominator;
        if distance < 0.0 {
            return None;
        }
        let world = origin + direction * distance;
        let local = plane.rotation.inverse() * (world - plane.origin);
        (local.x.abs() <= plane.size.x * 0.5 && local.y.abs() <= plane.size.y * 0.5)
            .then_some((world, plane.rotation))
    }

    pub(super) fn image_plane_controls(&mut self, ui: &mut egui::Ui) {
        let before = (
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
            self.doc.active_guide,
        );
        ui.horizontal(|ui| {
            if ui.button("Import image plane…").clicked() {
                self.import_image_plane();
            }
            ui.label(format!(
                "{}/{}",
                self.doc.image_planes.len(),
                crate::image_planes::MAX_IMAGE_PLANES
            ));
        });
        ui.small("Image planes are saved with the drawing and shown in PNG renders. GLB export contains strokes only.");
        if self.doc.image_planes.is_empty() {
            ui.add_space(8.0);
            ui.label("Import a PNG or JPEG to place it as a reference in 3D space.");
            return;
        }

        let mut remove = None;
        let mut changed_any = false;
        let mut active_change = None;
        for index in 0..self.doc.image_planes.len() {
            let id = self.doc.image_planes[index].id;
            let loaded = self.image_plane_assets.contains_key(&id);
            let mut plane_changed = false;
            ui.group(|ui| {
                let plane = &mut self.doc.image_planes[index];
                ui.horizontal(|ui| {
                    ui.strong(&plane.name);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
                if !loaded {
                    ui.colored_label(Color32::YELLOW, "Image file is missing or unreadable");
                }
                ui.horizontal(|ui| {
                    plane_changed |= ui.checkbox(&mut plane.visible, "Visible").changed();
                    plane_changed |= ui.checkbox(&mut plane.locked, "Locked").changed();
                    let mut active = self.doc.active_image_plane == Some(id);
                    if ui.checkbox(&mut active, "Draw here").changed() {
                        active_change = Some((id, active));
                    }
                });
                plane_changed |= ui
                    .add(egui::Slider::new(&mut plane.opacity, 0.0..=1.0).text("Opacity"))
                    .changed();
                ui.horizontal(|ui| {
                    ui.label("Position");
                    plane_changed |= ui
                        .add(
                            egui::DragValue::new(&mut plane.origin.x)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("X "),
                        )
                        .changed();
                    plane_changed |= ui
                        .add(
                            egui::DragValue::new(&mut plane.origin.y)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("Y "),
                        )
                        .changed();
                    plane_changed |= ui
                        .add(
                            egui::DragValue::new(&mut plane.origin.z)
                                .speed(0.01)
                                .range(-10000.0..=10000.0)
                                .prefix("Z "),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Size");
                    let aspect = plane
                        .pixel_size
                        .map_or(plane.size.y / plane.size.x, |[w, h]| h as f32 / w as f32);
                    let min_width = (0.001 / aspect).max(0.001);
                    let max_width = (10000.0 / aspect).min(10000.0);
                    plane_changed |= ui
                        .add(
                            egui::DragValue::new(&mut plane.size.x)
                                .speed(0.01)
                                .range(min_width..=max_width)
                                .prefix("W "),
                        )
                        .changed();
                    let height = plane.size.x * aspect;
                    plane_changed |= (height - plane.size.y).abs() > 1e-5;
                    plane.size.y = height;
                    ui.label(format!("H {height:.3} (aspect locked)"));
                });
                let (mut rx, mut ry, mut rz) = plane.rotation.to_euler(glam::EulerRot::XYZ);
                ui.horizontal(|ui| {
                    ui.label("Rotation°");
                    let mut x = rx.to_degrees();
                    let mut y = ry.to_degrees();
                    let mut z = rz.to_degrees();
                    let changed = ui
                        .add(
                            egui::DragValue::new(&mut x)
                                .speed(0.5)
                                .range(-3600.0..=3600.0)
                                .prefix("X "),
                        )
                        .changed()
                        | ui.add(
                            egui::DragValue::new(&mut y)
                                .speed(0.5)
                                .range(-3600.0..=3600.0)
                                .prefix("Y "),
                        )
                        .changed()
                        | ui.add(
                            egui::DragValue::new(&mut z)
                                .speed(0.5)
                                .range(-3600.0..=3600.0)
                                .prefix("Z "),
                        )
                        .changed();
                    if changed {
                        rx = x.to_radians();
                        ry = y.to_radians();
                        rz = z.to_radians();
                        plane.rotation =
                            glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz).normalize();
                        plane_changed = true;
                    }
                });
            });
            changed_any |= plane_changed;
        }
        if let Some(index) = remove {
            let plane = self.doc.image_planes.remove(index);
            self.renderer.remove_image_plane_asset(plane.id);
            if self.doc.active_image_plane == Some(plane.id) {
                self.doc.active_image_plane = None;
            }
            changed_any = true;
        }
        if let Some((id, true)) = active_change {
            for plane in &mut self.doc.image_planes {
                plane.drawable = plane.id == id;
            }
            self.doc.active_guide = None;
            self.doc.active_image_plane = Some(id);
            changed_any = true;
        } else if let Some((id, false)) = active_change {
            if self.doc.active_image_plane == Some(id) {
                self.doc.active_image_plane = None;
            }
            if let Some(plane) = self.doc.image_planes.iter_mut().find(|p| p.id == id) {
                plane.drawable = false;
            }
            changed_any = true;
        }
        if let Some(id) = self.doc.active_image_plane {
            if let Some(plane) = self.doc.image_planes.iter_mut().find(|p| p.id == id) {
                if !plane.visible || plane.locked || !plane.drawable {
                    plane.drawable = false;
                    self.doc.active_image_plane = None;
                    changed_any = true;
                }
            }
        }
        if changed_any {
            self.commit_image_plane_edit(before);
        }
    }

    fn import_image_plane(&mut self) {
        let Some(source) = rfd::FileDialog::new()
            .add_filter("PNG or JPEG image", &["png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };
        let before = (
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
            self.doc.active_guide,
        );
        if self.doc.image_planes.len() >= crate::image_planes::MAX_IMAGE_PLANES {
            self.status = "This drawing already has the maximum number of image planes.".into();
            return;
        }
        let id = self.next_id;
        if id == 0 || id == u64::MAX {
            self.status = "No more image plane IDs are available".into();
            return;
        }
        if std::fs::symlink_metadata(&self.image_asset_root)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            self.status = "Image asset folder must not be a symbolic link.".into();
            return;
        }
        if let Err(error) = std::fs::create_dir_all(&self.image_asset_root) {
            self.status = format!("Cannot create image asset folder: {error}");
            return;
        }
        let (asset_file, asset) =
            match crate::image_planes::copy_asset(&source, &self.image_asset_root, id) {
                Ok(value) => value,
                Err(error) => {
                    self.status = format!("Image import failed: {error}");
                    return;
                }
            };
        let total_pixels = self
            .doc
            .image_planes
            .iter()
            .filter_map(|plane| plane.pixel_size)
            .map(|[w, h]| u64::from(w) * u64::from(h))
            .sum::<u64>()
            .saturating_add(u64::from(asset.size[0]) * u64::from(asset.size[1]));
        if total_pixels > crate::image_planes::MAX_CATALOG_PIXELS {
            let _ = std::fs::remove_file(self.image_asset_root.join(&asset_file));
            self.status = "Image import exceeds the drawing's aggregate pixel budget.".into();
            return;
        }
        let name = source
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Image")
            .chars()
            .filter(|character| !character.is_control())
            .take(crate::image_planes::MAX_NAME_CHARS)
            .collect::<String>();
        let name = name.trim().to_owned();
        let name = if name.is_empty() {
            "Image".to_owned()
        } else {
            name
        };
        let plane = match crate::image_planes::ImagePlane::from_asset(
            id,
            name,
            asset_file.clone(),
            asset.size,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            2.0,
        ) {
            Ok(plane) => plane,
            Err(error) => {
                let _ = std::fs::remove_file(self.image_asset_root.join(&asset_file));
                self.status = format!("Image import failed: {error}");
                return;
            }
        };
        if let Err(error) = self.renderer.set_image_plane_asset(id, &asset) {
            let _ = std::fs::remove_file(self.image_asset_root.join(&plane.asset_file));
            self.status = format!("Image import failed: {error}");
            return;
        }
        self.image_plane_assets.insert(id, asset);
        self.doc.image_planes.push(plane);
        self.next_id += 1;
        self.commit_image_plane_edit(before);
        self.status =
            "Image plane imported. Position and size it, then enable Draw here to ink on it."
                .into();
    }
}
