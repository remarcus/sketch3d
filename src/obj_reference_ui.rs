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
    fn current_line_art_settings(&self) -> crate::model_line_art::Settings {
        crate::model_line_art::Settings {
            crease_angle_degrees: self.line_art_crease_angle,
            boundaries: self.line_art_boundaries,
            creases: self.line_art_creases,
            silhouettes: self.line_art_silhouettes,
        }
    }

    pub(super) fn process_line_art_command(&mut self, canvas: egui::Rect, ctx: &egui::Context) {
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
                let settings = self.current_line_art_settings();
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
                match self.bake_line_art_segments(&draft.segments) {
                    Ok(count) => {
                        self.status = format!(
                            "Baked {count} line-art strokes as independent strokes (one undo step)"
                        )
                    }
                    Err(error) => {
                        self.line_art_draft = Some(draft);
                        self.status = error;
                    }
                }
            }
            LineArtCommand::SetLive(source_id, settings) => {
                self.set_live_line_art_settings(source_id, settings);
            }
            LineArtCommand::BakeLive(source_id) => {
                self.refresh_live_line_art(canvas, ctx, true);
                let Some(cached) = self.live_line_art_cache.get(&source_id) else {
                    self.status = "No live line-art output is available for this model".into();
                    return;
                };
                if let Some(error) = &cached.error {
                    self.status = format!("Cannot bake live line art: {error}");
                    return;
                }
                let segments = cached.segments.clone();
                match self.bake_live_line_art(source_id, &segments) {
                    Ok(count) => self.status = format!(
                        "Baked {count} live line-art strokes and detached the source (one undo step)"
                    ),
                    Err(error) => self.status = error,
                }
            }
        }
    }

    fn set_live_line_art_settings(
        &mut self,
        source_id: u64,
        settings: Option<crate::model_line_art::Settings>,
    ) {
        if settings.is_some_and(|settings| settings.validate().is_err()) {
            self.status = "Live line-art settings are invalid".into();
            return;
        }
        let Some(index) = self
            .doc
            .model_references
            .iter()
            .position(|resource| resource.id == source_id)
        else {
            self.status = "The live line-art model no longer exists".into();
            return;
        };
        let resource = &self.doc.model_references[index];
        if resource.locked {
            self.status =
                "Unlock the OBJ reference before changing its live line-art source".into();
            return;
        }
        if settings.is_some() && !resource.visible {
            self.status = "Show the OBJ reference before enabling or updating live line art".into();
            return;
        }
        if resource.live_line_art == settings {
            self.status = "Live line-art settings are unchanged".into();
            return;
        }
        if settings.is_some()
            && resource.live_line_art.is_none()
            && self
                .doc
                .model_references
                .iter()
                .filter(|resource| resource.live_line_art.is_some())
                .count()
                >= crate::model_line_art::MAX_LIVE_SOURCES
        {
            self.status = format!(
                "A drawing can contain at most {} live line-art sources",
                crate::model_line_art::MAX_LIVE_SOURCES
            );
            return;
        }
        let before = self.model_reference_edit_state();
        self.doc.model_references[index].live_line_art = settings;
        if let Err(error) = crate::document::model_references::validate_catalog(
            &self.doc.model_references,
            self.doc.active_model_reference,
        ) {
            self.doc.model_references = before.0.clone();
            self.status = error;
            return;
        }
        let after = self.model_reference_edit_state();
        self.undo.push(Edit::ModelReferences { before, after });
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit_labeled("Change live model line art");
        self.live_line_art_cache.remove(&source_id);
        self.live_line_art_pending.remove(&source_id);
        self.renderer.invalidate();
        self.status = if settings.is_some() {
            "Live model line-art source enabled".into()
        } else {
            "Live model line-art source detached".into()
        };
    }

    fn bake_line_art_segments(&mut self, segments: &[[glam::Vec3; 2]]) -> Result<usize, String> {
        if segments.is_empty() {
            return Err("There is no line art to bake".into());
        }
        if self
            .doc
            .layers
            .iter()
            .find(|layer| layer.id == self.doc.active_layer)
            .is_none_or(|layer| !layer.visible || layer.locked)
        {
            return Err("Unlock and show the active layer before baking line art".into());
        }
        let count =
            u64::try_from(segments.len()).map_err(|_| "No more stroke IDs are available")?;
        let next_id = self
            .next_id
            .checked_add(count)
            .filter(|end| *end != u64::MAX)
            .ok_or("No more stroke IDs are available")?;
        let strokes = self.make_line_art_strokes(segments, self.next_id)?;
        let edit = Edit::AddBatch(strokes);
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.next_id = next_id;
        self.dirty = true;
        self.record_recovery_edit_labeled("Bake model line art");
        self.renderer.invalidate();
        Ok(segments.len())
    }

    fn bake_live_line_art(
        &mut self,
        source_id: u64,
        segments: &[[glam::Vec3; 2]],
    ) -> Result<usize, String> {
        if segments.is_empty() {
            return Err("There is no live line art to bake".into());
        }
        let Some(index) = self
            .doc
            .model_references
            .iter()
            .position(|resource| resource.id == source_id && resource.live_line_art.is_some())
        else {
            return Err("The live line-art source no longer exists".into());
        };
        let resource = &self.doc.model_references[index];
        if resource.locked || !resource.visible {
            return Err("Unlock and show the OBJ reference before baking live line art".into());
        }
        if self
            .doc
            .layers
            .iter()
            .find(|layer| layer.id == self.doc.active_layer)
            .is_none_or(|layer| !layer.visible || layer.locked)
        {
            return Err("Unlock and show the active layer before baking line art".into());
        }
        let count =
            u64::try_from(segments.len()).map_err(|_| "No more stroke IDs are available")?;
        let next_id = self
            .next_id
            .checked_add(count)
            .filter(|end| *end != u64::MAX)
            .ok_or("No more stroke IDs are available")?;
        let strokes = self.make_line_art_strokes(segments, self.next_id)?;
        let current_points: usize = self
            .doc
            .strokes
            .iter()
            .map(|stroke| stroke.points.len())
            .sum();
        if self.doc.strokes.len().saturating_add(strokes.len()) > 100_000
            || current_points.saturating_add(strokes.len().saturating_mul(2)) > 2_000_000
        {
            return Err("Baked line art would exceed the document stroke or point limit".into());
        }
        let before = self.model_reference_edit_state();
        self.doc.model_references[index].live_line_art = None;
        if let Err(error) = crate::document::model_references::validate_catalog(
            &self.doc.model_references,
            self.doc.active_model_reference,
        ) {
            self.doc.model_references = before.0.clone();
            return Err(error);
        }
        let after = self.model_reference_edit_state();
        let edit = Edit::BakeLiveLineArt {
            strokes,
            before,
            after,
        };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.next_id = next_id;
        self.dirty = true;
        self.record_recovery_edit_labeled("Bake live model line art");
        self.live_line_art_cache.remove(&source_id);
        self.live_line_art_pending.remove(&source_id);
        self.renderer.invalidate();
        Ok(segments.len())
    }

    fn make_line_art_strokes(
        &self,
        segments: &[[glam::Vec3; 2]],
        first_id: u64,
    ) -> Result<Vec<crate::document::Stroke>, String> {
        let count =
            u64::try_from(segments.len()).map_err(|_| "No more stroke IDs are available")?;
        let end_id = first_id
            .checked_add(count)
            .filter(|end| *end != u64::MAX)
            .ok_or("No more stroke IDs are available")?;
        let _ = end_id;
        Ok(segments
            .iter()
            .enumerate()
            .map(|(index, line)| crate::document::Stroke {
                model_surface: None,
                fill: None,
                material: crate::document::StrokeMaterial::Unlit,
                group_id: None,
                layer_id: self.doc.active_layer,
                id: first_id + index as u64,
                color: [0.035, 0.035, 0.04],
                gradient: None,
                radius: 0.006,
                brush: crate::document::Brush::default(),
                points: line
                    .iter()
                    .copied()
                    .map(|position| crate::document::Point {
                        position,
                        pressure: 1.0,
                        tilt: None,
                        time_ms: 0,
                    })
                    .collect(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                curve: None,
            })
            .collect())
    }

    fn live_line_art_key(
        &self,
        resource: &ObjReferenceResource,
        canvas: egui::Rect,
    ) -> Option<crate::model_line_art::LiveLineArtKey> {
        let settings = resource.live_line_art?;
        Some(crate::model_line_art::LiveLineArtKey {
            source_id: resource.id,
            asset_bundle: resource.asset.bundle_dir.clone(),
            settings,
            camera_projection: self.camera.projection,
            camera_angles: [self.camera.yaw, self.camera.pitch, self.camera.roll],
            camera_distance: self.camera.distance,
            camera_target: self.camera.target,
            canvas_size: [canvas.width(), canvas.height()],
            resource_transforms: self
                .doc
                .model_references
                .iter()
                .map(|item| {
                    (
                        item.id,
                        item.visible,
                        item.center,
                        item.rotation,
                        item.scale,
                    )
                })
                .collect(),
        })
    }

    pub(super) fn refresh_live_line_art(
        &mut self,
        canvas: egui::Rect,
        ctx: &egui::Context,
        force: bool,
    ) {
        if canvas.width() <= 0.0 || canvas.height() <= 0.0 {
            return;
        }
        self.line_art_canvas_size = Some([canvas.width(), canvas.height()]);
        let resources: Vec<_> = self
            .doc
            .model_references
            .iter()
            .filter(|resource| resource.visible && resource.live_line_art.is_some())
            .cloned()
            .collect();
        let active_ids: HashSet<_> = resources.iter().map(|resource| resource.id).collect();
        self.live_line_art_cache
            .retain(|id, _| active_ids.contains(id));
        self.live_line_art_pending
            .retain(|id, _| active_ids.contains(id));
        let now = Instant::now();
        let mut repaint_for_debounce = false;
        for resource in resources {
            let Some(key) = self.live_line_art_key(&resource, canvas) else {
                continue;
            };
            if self
                .live_line_art_cache
                .get(&resource.id)
                .is_some_and(|entry| entry.key == key)
            {
                self.live_line_art_pending.remove(&resource.id);
                continue;
            }
            if !force {
                let pending = self
                    .live_line_art_pending
                    .entry(resource.id)
                    .or_insert_with(|| crate::model_line_art::PendingLiveLineArt {
                        key: key.clone(),
                        started: now,
                    });
                if pending.key != key {
                    pending.key = key.clone();
                    pending.started = now;
                }
                if pending.started.elapsed()
                    < Duration::from_millis(crate::model_line_art::LIVE_REGENERATION_DEBOUNCE_MS)
                {
                    repaint_for_debounce = true;
                    continue;
                }
            }
            let result = if let Some(model) = self.obj_reference_models.get(&resource.id) {
                match crate::model_line_art::extract(
                    model,
                    &self.camera,
                    resource.center,
                    resource.rotation,
                    resource.scale,
                    resource.live_line_art.unwrap_or_default(),
                ) {
                    Ok(extraction) => {
                        let world_segments = extraction
                            .segments
                            .into_iter()
                            .map(|line| {
                                line.map(|point| {
                                    resource.center + resource.rotation * (point * resource.scale)
                                })
                            })
                            .collect::<Vec<_>>();
                        let mut indices = std::mem::take(&mut self.obj_surface_indices);
                        let visible = crate::model_line_art::filter_visible_limited(
                            world_segments,
                            crate::model_line_art::MAX_LIVE_OUTPUT_PER_SOURCE,
                            |sample| {
                                line_art_point_visible(
                                    &self.camera,
                                    canvas,
                                    sample,
                                    &self.doc.model_references,
                                    &self.obj_reference_models,
                                    &mut indices,
                                )
                            },
                        );
                        self.obj_surface_indices = indices;
                        Ok(crate::model_line_art::CachedLiveLineArt {
                            key: key.clone(),
                            segments: visible.segments,
                            truncated: extraction.truncated || visible.truncated,
                            error: None,
                        })
                    }
                    Err(error) => Err(error),
                }
            } else {
                Err("The model bundle is missing or not loaded".into())
            };
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => crate::model_line_art::CachedLiveLineArt {
                    key,
                    segments: Vec::new(),
                    truncated: false,
                    error: Some(error),
                },
            };
            self.live_line_art_cache.insert(resource.id, entry);
            self.live_line_art_pending.remove(&resource.id);
        }
        if repaint_for_debounce {
            ctx.request_repaint_after(Duration::from_millis(
                crate::model_line_art::LIVE_REGENERATION_DEBOUNCE_MS,
            ));
        }
    }

    pub(super) fn clear_live_line_art_cache(&mut self) {
        self.live_line_art_cache.clear();
        self.live_line_art_pending.clear();
    }

    pub(super) fn append_live_line_art_to_export(
        &mut self,
        document: &mut crate::document::Document,
        ctx: &egui::Context,
    ) -> Result<usize, String> {
        let [width, height] = self
            .line_art_canvas_size
            .ok_or("The drawing canvas has not been initialized for live line art")?;
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        self.refresh_live_line_art(canvas, ctx, true);
        let mut segments = Vec::new();
        for resource in self
            .doc
            .model_references
            .iter()
            .filter(|resource| resource.visible && resource.live_line_art.is_some())
        {
            let Some(entry) = self.live_line_art_cache.get(&resource.id) else {
                continue;
            };
            if let Some(error) = &entry.error {
                return Err(format!("{}: {error}", resource.name));
            }
            segments.extend(entry.segments.iter().copied());
        }
        let strokes = self.make_line_art_strokes(&segments, self.next_id)?;
        let count = strokes.len();
        let current_points: usize = document
            .strokes
            .iter()
            .map(|stroke| stroke.points.len())
            .sum();
        if document.strokes.len().saturating_add(count) > 100_000
            || current_points.saturating_add(count.saturating_mul(2)) > 2_000_000
        {
            return Err("Live line-art export exceeds the document stroke or point limit".into());
        }
        document.strokes.extend(strokes);
        Ok(count)
    }

    pub(super) fn paint_line_art_preview(&self, painter: &egui::Painter, canvas: egui::Rect) {
        let paint_segments = |segments: &[[glam::Vec3; 2]], color| {
            for line in segments {
                if let (Some((a, _)), Some((b, _))) = (
                    crate::gizmo::project(&self.camera, canvas, line[0]),
                    crate::gizmo::project(&self.camera, canvas, line[1]),
                ) {
                    painter.line_segment([a, b], egui::Stroke::new(1.5_f32, color));
                }
            }
        };
        if let Some(draft) = &self.line_art_draft {
            if self
                .doc
                .model_references
                .iter()
                .any(|resource| resource.id == draft.source_id && resource.visible)
            {
                paint_segments(
                    &draft.segments,
                    egui::Color32::from_rgba_unmultiplied(255, 185, 55, 210),
                );
            }
        }
        for resource in self
            .doc
            .model_references
            .iter()
            .filter(|resource| resource.visible && resource.live_line_art.is_some())
        {
            let Some(key) = self.live_line_art_key(resource, canvas) else {
                continue;
            };
            let Some(entry) = self.live_line_art_cache.get(&resource.id) else {
                continue;
            };
            if entry.key == key && entry.error.is_none() {
                paint_segments(
                    &entry.segments,
                    egui::Color32::from_rgba_unmultiplied(93, 222, 166, 210),
                );
            }
        }
    }

    pub(super) fn show_obj_reference_window(&mut self, ctx: &egui::Context) {
        if !self.obj_reference_window_open {
            return;
        }
        let mut open = self.obj_reference_window_open;
        egui::Window::new("Model references")
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
        egui::CollapsingHeader::new("Model references")
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
            ui.small("OBJ and self-contained GLB models are read-only references. Whole-scene GLB includes visible models; selected-stroke export remains ink-only. GLB node transforms, base-color materials, and embedded PNG/JPEG textures are imported and kept with the drawing. WebP textures, nonzero UV sets, and alpha-mask cutoffs fall back to the material color. External dependencies are rejected.");

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
            ui.small("Line-art settings apply only when you choose Preview, Make live, or Update settings. Cancel discards the preview and leaves live settings unchanged.");
            ui.small("Live sources refresh after the camera or model transform settles. Camera-only refreshes do not add undo history; Bake view converts the overlay to ordinary strokes and detaches it in one undo step. Detach removes the overlay without adding strokes.");

            let before = self.model_reference_edit_state();
            let mut remove = None;
            let mut changed = false;
            let mut geometry_changed = false;
            let mut activate = None;
            let mut live_action = None;
            let current_live_settings = self.current_line_art_settings();
            let active_layer_bakeable = self
                .doc
                .layers
                .iter()
                .any(|layer| layer.id == self.doc.active_layer && layer.visible && !layer.locked);
            let active_model_reference = self.doc.active_model_reference;
            for index in 0..self.doc.model_references.len() {
                let original = self.doc.model_references[index].clone();
                let live_cache_info = self
                    .live_line_art_cache
                    .get(&original.id)
                    .map(|entry| (entry.error.clone(), entry.truncated));
                let model_available = self.obj_reference_models.contains_key(&original.id);
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
                    ui.horizontal(|ui| {
                        if resource.live_line_art.is_some() {
                            ui.label("Live line art");
                            if ui
                                .add_enabled(
                                    resource.visible && !resource.locked,
                                    egui::Button::new("Update settings"),
                                )
                                .clicked()
                            {
                                live_action = Some(LineArtCommand::SetLive(
                                    resource.id,
                                    Some(current_live_settings),
                                ));
                            }
                            if ui
                                .add_enabled(
                                    resource.visible && model_available && active_layer_bakeable,
                                    egui::Button::new("Bake view"),
                                )
                                .clicked()
                            {
                                live_action = Some(LineArtCommand::BakeLive(resource.id));
                            }
                            if ui
                                .add_enabled(!resource.locked, egui::Button::new("Detach"))
                                .clicked()
                            {
                                live_action = Some(LineArtCommand::SetLive(resource.id, None));
                            }
                        } else if ui
                            .add_enabled(
                                resource.visible && !resource.locked && model_available,
                                egui::Button::new("Make live"),
                            )
                            .clicked()
                        {
                            live_action = Some(LineArtCommand::SetLive(
                                resource.id,
                                Some(current_live_settings),
                            ));
                        }
                    });
                    if resource.live_line_art.is_some() {
                        if let Some((Some(error), _)) = &live_cache_info {
                            ui.colored_label(ui_style::colors(ui.ctx()).warning, error);
                        }
                        if live_cache_info
                            .as_ref()
                            .is_some_and(|(_, truncated)| *truncated)
                        {
                            ui.colored_label(
                                ui_style::colors(ui.ctx()).warning,
                                "Live line art is truncated by the per-source output limit.",
                            );
                        }
                    }
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
                self.live_line_art_cache.remove(&resource.id);
                self.live_line_art_pending.remove(&resource.id);
                if self.selected_model_reference == Some(resource.id) {
                    self.selected_model_reference = None;
                }
                if self.doc.active_model_reference == Some(resource.id) {
                    self.doc.active_model_reference = None;
                }
                changed = true;
            }
            if let Some(command) = live_action {
                self.line_art_command = Some(command);
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
                    .add_filter("3D model reference", &["obj", "glb"])
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
            self.status = "This drawing already has the maximum number of model references.".into();
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
        let format = if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
        {
            "GLB"
        } else {
            "OBJ"
        };
        let (asset, model) = match crate::obj_reference_storage::import_source_bundle(
            path,
            &self.image_asset_root,
            id,
        ) {
            Ok(imported) => imported,
            Err(error) => {
                self.status = format!("{format} import failed: {error}");
                return;
            }
        };
        let size = model.bounds[1] - model.bounds[0];
        let extent = size.max_element();
        if !extent.is_finite() || extent <= 1.0e-5 {
            self.status = "Model geometry has no usable bounds.".into();
            return;
        }
        let scale = (2.0 / extent).clamp(0.001, 1000.0);
        let rotation = glam::Quat::IDENTITY;
        let center =
            self.camera.target - rotation * ((model.bounds[0] + model.bounds[1]) * 0.5 * scale);
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Model reference")
            .chars()
            .take(crate::document::model_references::MAX_MODEL_NAME_CHARS)
            .collect::<String>()
            .trim()
            .to_owned();
        let mut name = if name.is_empty() {
            format!("Model {id}")
        } else {
            name
        };
        if self
            .doc
            .model_references
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            name = format!("Model {id}");
        }
        let resource = ObjReferenceResource {
            id,
            name,
            visible: true,
            locked: false,
            center,
            rotation,
            scale,
            live_line_art: None,
            asset,
        };
        let triangle_count = model.triangles.len();
        let material_count = model.materials.len();
        let warning_count = model.warnings.len();
        if let Err(error) = resource.validate() {
            self.status = format!("Model reference is invalid: {error}");
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
        self.record_recovery_edit_labeled("Import model reference");
        self.renderer.invalidate();
        self.status = format!(
            "Imported {format}: {triangle_count} triangles, {material_count} materials, {warning_count} import notes."
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
            let names = match model.source_format {
                crate::obj_reference::ObjSourceFormat::Obj => {
                    let mut names = vec!["model.obj".to_owned()];
                    names.extend(
                        (0..model.source_materials.len())
                            .map(|index| format!("material-{index:03}.mtl")),
                    );
                    names
                }
                crate::obj_reference::ObjSourceFormat::Glb => vec!["model.glb".to_owned()],
            };
            for name in names {
                let asset_file = format!("{}/{name}", resource.asset.bundle_dir);
                let path = directory.join(&name);
                let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                    return false;
                };
                let Ok(canonical_path) = path.canonicalize() else {
                    return false;
                };
                let limit = match name.as_str() {
                    "model.obj" => crate::obj_reference_storage::MAX_MODEL_OBJ_BYTES,
                    "model.glb" => crate::gltf_reference::MAX_GLB_BYTES,
                    _ => crate::obj_reference_storage::MAX_MODEL_MTL_BYTES,
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
    ui.button("Import model reference…")
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
            live_line_art: None,
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
            live_line_art: None,
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
