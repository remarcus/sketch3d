use super::*;
use crate::reprojection::{Draft, ProjectionMode, TargetChoice};

impl SketchApp {
    pub(super) fn begin_reprojection(&mut self) {
        if self.reprojection_draft.is_some() || self.selected_model_reference.is_some() {
            self.status = "Reprojection accepts stroke selections only.".into();
            return;
        }
        match Draft::new(&self.doc, &self.selected, self.camera) {
            Ok(mut draft) => {
                let active_plane = self.planar_drawing_target().map(|target| target.plane);
                if active_plane.is_some() {
                    draft.target = TargetChoice::ActivePlanarGuide;
                }
                if let Some(id) = self.doc.active_model_reference {
                    draft.target = TargetChoice::ObjReference(id);
                    let _ = rebuild_model_preview(
                        &mut draft,
                        &self.doc,
                        &self.obj_reference_models,
                        &mut self.obj_surface_indices,
                        id,
                        self.last_canvas_rect,
                    );
                } else if let Ok(plane) = draft.target_plane(&self.doc, active_plane) {
                    let _ = draft.rebuild(plane, self.last_canvas_rect);
                }
                self.finish();
                self.point_draft = None;
                self.point_multi_draft = None;
                self.brush_select_enabled = false;
                self.region_select_enabled = false;
                self.reprojection_draft = Some(draft);
                self.presentation.preferences.layout.inspector_open = true;
                self.presentation.preferences.layout.inspector_tab =
                    crate::presentation::InspectorTab::Properties;
                self.status = "Reprojection preview is detached. Choose a target and projection, then Apply or Escape.".into();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn finish_reprojection(&mut self, apply: bool) {
        if apply {
            let Some(draft) = self.reprojection_draft.as_ref() else {
                return;
            };
            if let Some(error) = &draft.error {
                self.status = error.clone();
                return;
            }
            if draft.preview == draft.before {
                self.status =
                    "Reprojection produced no changes; adjust its target or settings.".into();
                return;
            }
        }
        let Some(draft) = self.reprojection_draft.take() else {
            return;
        };
        if !apply {
            self.status = "Reprojection cancelled; the drawing is unchanged.".into();
            return;
        }
        match draft.edit(&self.doc) {
            Ok(edit) => {
                let ids: HashSet<_> = draft.before.iter().map(|stroke| stroke.id).collect();
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.renderer.invalidate_ids(&ids);
                self.dirty = true;
                self.record_recovery_edit();
                self.status = "Reprojection applied as one undo step.".into();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn reprojection_controls(&mut self, ui: &mut egui::Ui) {
        let active_plane = self.planar_drawing_target().map(|target| target.plane);
        let rect = self.last_canvas_rect;
        let doc = &self.doc;
        let saved_planes: Vec<_> = doc
            .guide_resources
            .iter()
            .filter_map(|guide| {
                matches!(
                    &guide.geometry,
                    crate::document::guide_resources::GuideGeometry::Plane { .. }
                )
                .then_some((guide.id, guide.name.clone()))
            })
            .collect();
        let saved_curved_guides: Vec<_> = doc
            .guide_resources
            .iter()
            .filter_map(|guide| {
                matches!(
                    &guide.geometry,
                    crate::document::guide_resources::GuideGeometry::Cylinder { .. }
                        | crate::document::guide_resources::GuideGeometry::Sphere { .. }
                )
                .then_some((guide.id, guide.name.clone()))
            })
            .collect();
        let obj_targets: Vec<_> = doc
            .model_references
            .iter()
            .filter(|resource| {
                resource.visible && self.obj_reference_models.contains_key(&resource.id)
            })
            .map(|resource| (resource.id, resource.name.clone()))
            .collect();
        let mut changed = false;
        let mut apply = false;
        let mut cancel = false;
        {
            let Some(draft) = self.reprojection_draft.as_mut() else {
                return;
            };
            ui.heading("Reproject selected strokes");
            ui.small("The preview projects stored samples onto a plane or saved analytic surface and rebuilds the brush frame. Source strokes remain unchanged until Apply.");
            egui::ComboBox::from_id_salt("reproject-target")
                .selected_text(target_label(
                    draft.target,
                    &saved_planes,
                    &saved_curved_guides,
                    &obj_targets,
                ))
                .show_ui(ui, |ui| {
                    for (target, label) in [
                        (TargetChoice::Front, "Front"),
                        (TargetChoice::Side, "Side"),
                        (TargetChoice::Ground, "Ground"),
                        (TargetChoice::ActivePlanarGuide, "Active planar guide"),
                        (TargetChoice::CapturedView, "Captured view plane"),
                    ] {
                        changed |= ui
                            .selectable_value(&mut draft.target, target, label)
                            .changed();
                    }
                    if !saved_planes.is_empty() {
                        ui.separator();
                    }
                    for (id, name) in &saved_planes {
                        changed |= ui
                            .selectable_value(
                                &mut draft.target,
                                TargetChoice::SavedPlane(*id),
                                name,
                            )
                            .changed();
                    }
                    if !saved_curved_guides.is_empty() {
                        ui.separator();
                        ui.label("Curved guides");
                    }
                    for (id, name) in &saved_curved_guides {
                        changed |= ui
                            .selectable_value(
                                &mut draft.target,
                                TargetChoice::SavedCurvedGuide(*id),
                                name,
                            )
                            .changed();
                    }
                    if !obj_targets.is_empty() {
                        ui.separator();
                        ui.label("OBJ references");
                    }
                    for (id, name) in &obj_targets {
                        changed |= ui
                            .selectable_value(
                                &mut draft.target,
                                TargetChoice::ObjReference(*id),
                                name,
                            )
                            .changed();
                    }
                });
            ui.horizontal(|ui| {
                changed |= ui
                    .selectable_value(&mut draft.mode, ProjectionMode::Orthogonal, "Orthogonal")
                    .changed();
                changed |= ui
                    .selectable_value(
                        &mut draft.mode,
                        ProjectionMode::CapturedViewRay,
                        "Captured view rays",
                    )
                    .changed();
            });
            let model_target = matches!(draft.target, TargetChoice::ObjReference(_));
            if model_target {
                changed |= ui
                    .checkbox(&mut draft.attach_to_model, "Attach projected strokes to model")
                    .changed();
            }
            let attach = model_target && draft.attach_to_model;
            if !attach {
                changed |= super::ui_style::numeric_row(
                    ui,
                    "Normal offset",
                    "reproject-offset",
                    &mut draft.offset,
                    Some(-100.0..=100.0),
                    "wu",
                )
                .changed();
            }
            if matches!(
                draft.target,
                TargetChoice::SavedCurvedGuide(_) | TargetChoice::ObjReference(_)
            ) {
                changed |= ui
                    .add(
                        egui::Slider::new(&mut draft.resample_spacing, 0.05..=2.0)
                            .logarithmic(true)
                            .text("Maximum sample spacing"),
                    )
                    .changed();
                ui.small("Surface targets add samples along the original path before projection, preserving pressure, tilt, timing, local color, and thickness. The preview is capped at 65,536 samples.");
            }
            if !attach {
                changed |= ui
                    .checkbox(
                        &mut draft.keep_original,
                        "Keep original samples when projection misses",
                    )
                    .changed();
            }
            if matches!(draft.target, TargetChoice::SavedCurvedGuide(_)) {
                ui.small("Cylinder and Sphere targets use the nearest surface point or the first forward hit along captured view rays. Samples through cylinder ends or ambiguous center points report a miss; keeping them can leave segments crossing the surface.");
            } else if attach {
                let skipped: Vec<_> = draft
                    .preview
                    .iter()
                    .filter(|stroke| stroke.attached_model().is_none())
                    .map(|stroke| stroke.id.to_string())
                    .collect();
                ui.small("Attachment needs a continuous projection across neighboring OBJ faces. Choose a smaller sample spacing for small faces.");
                if !skipped.is_empty() && !changed {
                    ui.colored_label(
                        ui_style::colors(ui.ctx()).warning,
                        format!("Strokes left in place (no continuous projection or unsupported brush): {}", skipped.join(", ")),
                    );
                }
            } else if model_target {
                ui.small("OBJ targets use the closest triangle or first forward ray hit. Projection rejects ambiguous nearest points and disconnected triangle transitions; tangent rays and mesh gaps count as misses.");
            } else if draft.mode == ProjectionMode::CapturedViewRay {
                ui.small("Samples outside the captured view or without a forward target hit report a miss.");
            }
            if draft.kept_original_samples > 0 {
                ui.colored_label(
                    ui_style::colors(ui.ctx()).warning,
                    format!(
                        "{} samples remain at their original positions because the target had no hit.",
                        draft.kept_original_samples
                    ),
                );
            }
            if draft.before.iter().any(|stroke| stroke.curve.is_some()) {
                changed |= ui
                    .checkbox(
                        &mut draft.convert_curves,
                        "Convert semantic curves to sampled strokes",
                    )
                    .changed();
                ui.small("Conversion keeps the existing evaluated samples and removes semantic control handles.");
            }
            if let Some(error) = &draft.error {
                ui.colored_label(ui_style::colors(ui.ctx()).warning, error);
            }
            ui.small(format!(
                "{} strokes · {} source → {} preview samples · target locked to this preview",
                draft.before.len(),
                draft
                    .before
                    .iter()
                    .map(|stroke| stroke.points.len())
                    .sum::<usize>(),
                draft
                    .preview
                    .iter()
                    .map(|stroke| stroke.points.len())
                    .sum::<usize>()
            ));
            ui.horizontal(|ui| {
                apply = ui
                    .add_enabled(
                        !changed && draft.error.is_none() && draft.preview != draft.before,
                        egui::Button::new("Apply reprojection"),
                    )
                    .clicked();
                cancel = ui.button("Cancel reprojection").clicked();
            });
        }
        if changed {
            if let Some(draft) = self.reprojection_draft.as_mut() {
                let result = match draft.target {
                    TargetChoice::SavedCurvedGuide(id) => {
                        draft.rebuild_on_curved_guide(doc, id, rect)
                    }
                    TargetChoice::ObjReference(id) => {
                        rebuild_model_preview(
                            draft,
                            doc,
                            &self.obj_reference_models,
                            &mut self.obj_surface_indices,
                            id,
                            rect,
                        )
                    }
                    _ => draft
                        .target_plane(doc, active_plane)
                        .and_then(|plane| draft.rebuild(plane, rect)),
                };
                if let Err(error) = result {
                    self.status = error;
                }
            }
        }
        if apply || cancel {
            self.finish_reprojection(apply);
        }
    }

    pub(super) fn reprojection_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(draft) = &self.reprojection_draft else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let color = ui_style::canvas_selection_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        );
        for stroke in &draft.preview {
            for pair in stroke.points.windows(2) {
                if let (Some((a, _)), Some((b, _))) = (
                    crate::gizmo::project(&self.camera, rect, pair[0].position),
                    crate::gizmo::project(&self.camera, rect, pair[1].position),
                ) {
                    painter.line_segment([a, b], egui::Stroke::new(2.0_f32, color));
                }
            }
        }
    }
}

fn target_label(
    target: TargetChoice,
    saved: &[(u64, String)],
    curved: &[(u64, String)],
    obj_targets: &[(u64, String)],
) -> String {
    match target {
        TargetChoice::Front => "Front".into(),
        TargetChoice::Side => "Side".into(),
        TargetChoice::Ground => "Ground".into(),
        TargetChoice::ActivePlanarGuide => "Active planar guide".into(),
        TargetChoice::CapturedView => "Captured view plane".into(),
        TargetChoice::SavedPlane(id) => saved
            .iter()
            .find(|entry| entry.0 == id)
            .map(|entry| entry.1.clone())
            .unwrap_or_else(|| "Missing saved plane".into()),
        TargetChoice::SavedCurvedGuide(id) => curved
            .iter()
            .find(|entry| entry.0 == id)
            .map(|entry| entry.1.clone())
            .unwrap_or_else(|| "Missing curved guide".into()),
        TargetChoice::ObjReference(id) => obj_targets
            .iter()
            .find(|entry| entry.0 == id)
            .map(|entry| entry.1.clone())
            .unwrap_or_else(|| "Missing OBJ reference".into()),
    }
}

fn rebuild_model_preview(
    draft: &mut Draft,
    doc: &Document,
    models: &std::collections::HashMap<u64, crate::obj_reference::ObjModel>,
    indices: &mut std::collections::HashMap<u64, crate::obj_mesh_index::ObjMeshIndex>,
    id: u64,
    rect: Rect,
) -> Result<(), String> {
    let result = (|| {
        let resource = doc
            .model_references
            .iter()
            .find(|resource| resource.id == id)
            .ok_or("Selected OBJ target no longer exists")?;
        let model = models.get(&id).ok_or("Selected OBJ target is not loaded")?;
        let index = match indices.entry(id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(crate::obj_mesh_index::ObjMeshIndex::new(model)?),
        };
        draft.rebuild_on_obj_reference(resource, model, index, rect)
    })();
    if let Err(error) = &result {
        draft.reject_preview(error);
    }
    result
}
