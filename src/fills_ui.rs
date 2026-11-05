use super::guide_resource_ui::PlanarTarget;
use super::*;
use crate::{document::PlanarFill, polygon};

pub(super) struct FillDraft {
    pub mark: Stroke,
    target: PlanarTarget,
    cylinder: Option<(u64, crate::document::CylinderFillSurface)>,
    pub(super) original: Option<Stroke>,
    selected_contour: usize,
    selected: usize,
    pub(super) drag: Option<FillPointDrag>,
    pub(super) gradient_drag: Option<(usize, Camera, Rect, Vec3)>,
    imported_texture_ids: Vec<u64>,
}
pub(super) struct FillPointDrag {
    contour: usize,
    index: usize,
    origin: Vec3,
    start: Vec3,
    normal: Vec3,
    camera: Camera,
    rect: Rect,
}
impl FillPointDrag {
    fn position(&self, pos: Pos2) -> Option<Vec3> {
        crate::gizmo::plane_hit(&self.camera, self.rect, pos, self.origin, self.normal)
            .map(|hit| self.origin + hit - self.start)
    }
}
pub(super) struct FillUi {
    pub enabled: bool,
    pub offset: f32,
    pub region_mode: bool,
    pub gap_tolerance: f32,
    pub active_layer_boundaries: bool,
    pub region_anchor: Option<Vec3>,
    pub closing_guides: Vec<(Vec3, Vec3)>,
    pub closing_start: Option<Vec3>,
    pub draft: Option<FillDraft>,
    pub verification_frame: Option<u32>,
    texture_preview: Option<(u64, bool, egui::TextureHandle)>,
}
impl Default for FillUi {
    fn default() -> Self {
        Self {
            enabled: false,
            offset: 0.003,
            region_mode: false,
            gap_tolerance: 0.0,
            active_layer_boundaries: false,
            region_anchor: None,
            closing_guides: Vec::new(),
            closing_start: None,
            draft: None,
            verification_frame: None,
            texture_preview: None,
        }
    }
}
impl SketchApp {
    pub(super) fn active_fill_cylinder(
        &self,
    ) -> Option<(u64, crate::document::CylinderFillSurface)> {
        let id = self.doc.active_guide?;
        let guide = self
            .doc
            .guide_resources
            .iter()
            .find(|guide| guide.id == id)?;
        if !guide.visible || guide.locked {
            return None;
        }
        let crate::document::guide_resources::GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } = &guide.geometry
        else {
            return None;
        };
        Some((
            id,
            crate::document::CylinderFillSurface {
                center: *center,
                rotation: *rotation,
                radius: *radius,
                height: *height,
                angle_origin: 0.0,
            },
        ))
    }

    pub(super) fn begin_fill_contour_edit(&mut self) {
        if self.fills.draft.is_some() || self.selected.len() != 1 {
            return;
        }
        let id = *self.selected.iter().next().unwrap();
        let Some(source) = self
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.id == id)
            .cloned()
        else {
            return;
        };
        if source.fill.is_none() || !self.doc.editable(&source) {
            self.status = "Select one visible, unlocked planar fill to edit its contour.".into();
            return;
        }
        if source
            .fill
            .as_ref()
            .is_some_and(|fill| fill.cylinder.is_some())
        {
            self.status = "Curved cylinder fills can be selected and exported; contour editing is not supported yet.".into();
            return;
        }
        if let Err(error) = polygon::triangulate(&source) {
            self.status = error;
            return;
        }
        let Some(target) = PlanarTarget::from_curve_frame(
            source.points[0].position,
            source.brush.normal,
            source.brush.tangent,
            None,
        ) else {
            self.status = "The fill's captured plane frame is invalid.".into();
            return;
        };
        self.finish();
        self.fills.enabled = false;
        self.fills.draft = Some(FillDraft {
            mark: source.clone(),
            target,
            cylinder: None,
            original: Some(source),
            selected_contour: 0,
            selected: 0,
            drag: None,
            gradient_drag: None,
            imported_texture_ids: Vec::new(),
        });
        self.select_mode = false;
        self.brush_select_enabled = false;
        self.presentation.preferences.layout.inspector_open = true;
        self.presentation.preferences.layout.inspector_tab =
            crate::presentation::InspectorTab::Properties;
        self.status = "Edit fill contour: drag handles or use numeric coordinates. Apply commits one edit; Escape cancels.".into();
    }

    fn fill_contour_controls(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = self.fills.draft.as_mut() else {
            return;
        };
        ui.heading("Edit fill contour");
        ui.small("Fill contours use the even-odd rule. Nested cutouts alternate between empty and filled regions.");
        let hole_count = draft.mark.fill.as_ref().map_or(0, |fill| fill.holes.len());
        let mut selected_contour = draft.selected_contour.min(hole_count);
        egui::ComboBox::from_label("Contour")
            .selected_text(if selected_contour == 0 {
                "Outer boundary".to_owned()
            } else {
                format!("Cutout {}", selected_contour)
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected_contour, 0, "Outer boundary");
                for index in 1..=hole_count {
                    ui.selectable_value(&mut selected_contour, index, format!("Cutout {index}"));
                }
            });
        draft.selected_contour = selected_contour;
        let count = if selected_contour == 0 {
            draft.mark.points.len()
        } else {
            draft.mark.fill.as_ref().unwrap().holes[selected_contour - 1].len()
        };
        ui.horizontal(|ui| {
            ui.label("Vertex (0-based)");
            ui.add(egui::DragValue::new(&mut draft.selected).range(0..=count - 1));
        });
        draft.selected = draft.selected.min(count - 1);
        let mut coordinates = if selected_contour == 0 {
            draft.mark.points[draft.selected].position.to_array()
        } else {
            draft.mark.fill.as_ref().unwrap().holes[selected_contour - 1][draft.selected].to_array()
        };
        let mut changed = false;
        for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
            changed |= super::ui_style::numeric_row(
                ui,
                label,
                ("fill-contour", axis),
                &mut coordinates[axis],
                Some(-10000.0..=10000.0),
                "wu",
            )
            .changed();
        }
        let selected = draft.selected;
        let original = draft.original.as_ref().unwrap().clone();
        let apply_enabled = draft.mark != original;
        let mut apply = false;
        let mut cancel = false;
        let mut insert = false;
        let mut remove = false;
        let mut add_hole = false;
        let mut remove_hole = false;
        let mut preview_texture_id = None;
        ui.horizontal(|ui| {
            insert = ui.button("Insert midpoint after vertex").clicked();
            remove = ui
                .add_enabled(count > 3, egui::Button::new("Remove vertex"))
                .clicked();
        });
        ui.horizontal(|ui| {
            add_hole = ui
                .add_enabled(
                    draft.original.is_some() && hole_count < polygon::MAX_HOLES,
                    egui::Button::new("Add cutout"),
                )
                .on_hover_text("Adds a small nested rectangular cutout to the fill")
                .clicked();
            remove_hole = ui
                .add_enabled(
                    draft.original.is_some() && selected_contour > 0,
                    egui::Button::new("Remove cutout"),
                )
                .clicked();
        });
        ui.horizontal(|ui| {
            apply = ui
                .add_enabled(apply_enabled, egui::Button::new("Apply contour edits"))
                .clicked();
            cancel = ui.button("Cancel contour edits").clicked();
        });
        ui.separator();
        ui.strong("Fill appearance");
        let import_texture = ui.button("Import fill texture PNG…").clicked();
        if let Some(fill) = draft.mark.fill.as_mut() {
            if let Some(mut texture) = fill.texture {
                ui.horizontal(|ui| {
                    ui.label("Fill texture");
                    ui.checkbox(&mut texture.repeat, "Repeat");
                });
                ui.small("Uncheck Repeat to clamp the texture at its borders.");
                ui.horizontal(|ui| {
                    ui.label("Scale U / V");
                    ui.add(egui::DragValue::new(&mut texture.scale.x).range(0.001..=100.0));
                    ui.add(egui::DragValue::new(&mut texture.scale.y).range(0.001..=100.0));
                });
                let mut rotation_degrees = texture.rotation.to_degrees();
                ui.horizontal(|ui| {
                    ui.label("Rotation");
                    if ui
                        .add(
                            egui::DragValue::new(&mut rotation_degrees)
                                .range(-36000.0..=36000.0)
                                .suffix("°"),
                        )
                        .changed()
                    {
                        texture.rotation = rotation_degrees.to_radians();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Offset U / V");
                    ui.add(egui::DragValue::new(&mut texture.offset.x).range(-100.0..=100.0));
                    ui.add(egui::DragValue::new(&mut texture.offset.y).range(-100.0..=100.0));
                });
                let mut tint = texture.tint.map(crate::presentation::linear_to_srgb);
                ui.horizontal(|ui| {
                    ui.label("Tint");
                    if ui.color_edit_button_rgb(&mut tint).changed() {
                        texture.tint = tint.map(crate::presentation::srgb_to_linear);
                    }
                });
                ui.add(egui::Slider::new(&mut texture.opacity, 0.0..=1.0).text("Texture opacity"));
                if texture.is_valid() {
                    fill.texture = Some(texture);
                } else {
                    self.status = "Texture mapping values exceed the supported limits.".into();
                }
                if ui.button("Remove image texture").clicked() {
                    fill.texture = None;
                    self.fills.texture_preview = None;
                }
                let texture_id = fill.texture.map(|texture| texture.asset_id);
                if let Some(texture_id) = texture_id {
                    preview_texture_id = Some((texture_id, texture.repeat));
                }
            }
            if let Some(mut gradient) = fill.gradient {
                let old_kind = gradient.kind;
                ui.horizontal(|ui| {
                    ui.label("Gradient type");
                    egui::ComboBox::from_id_salt("fill-gradient-kind")
                        .selected_text(match gradient.kind {
                            crate::document::FillGradientKind::Linear => "Linear",
                            crate::document::FillGradientKind::Radial => "Radial",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut gradient.kind,
                                crate::document::FillGradientKind::Linear,
                                "Linear",
                            );
                            ui.selectable_value(
                                &mut gradient.kind,
                                crate::document::FillGradientKind::Radial,
                                "Radial",
                            );
                        });
                });
                if gradient.kind != old_kind {
                    match gradient.kind {
                        crate::document::FillGradientKind::Linear => {
                            gradient.start = glam::Vec2::new(0.0, 0.5);
                            gradient.end = glam::Vec2::new(1.0, 0.5);
                        }
                        crate::document::FillGradientKind::Radial => {
                            gradient.start = glam::Vec2::splat(0.5);
                            gradient.end = glam::Vec2::new(1.0, 0.5);
                        }
                    }
                }
                let mut start = gradient
                    .start_color
                    .map(crate::presentation::linear_to_srgb);
                let mut end = gradient.end_color.map(crate::presentation::linear_to_srgb);
                ui.horizontal(|ui| {
                    ui.label("Start stop");
                    if ui.color_edit_button_rgb(&mut start).changed() {
                        gradient.start_color = start.map(crate::presentation::srgb_to_linear);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("End stop");
                    if ui.color_edit_button_rgb(&mut end).changed() {
                        gradient.end_color = end.map(crate::presentation::srgb_to_linear);
                    }
                });
                for (label, position) in [
                    ("Start", &mut gradient.start),
                    ("End / radius", &mut gradient.end),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(
                            egui::DragValue::new(&mut position.x)
                                .range(0.0..=1.0)
                                .prefix("U "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut position.y)
                                .range(0.0..=1.0)
                                .prefix("V "),
                        );
                    });
                }
                if gradient.is_valid() {
                    fill.gradient = Some(gradient);
                } else {
                    self.status =
                        "Gradient handles must stay distinct and within the fill bounds.".into();
                }
                ui.small("Drag the on-canvas handles; coordinates follow the fill's normalized bounds and interpolate in linear RGB.");
                if ui
                    .button(if fill.texture.is_some() {
                        "Remove gradient"
                    } else {
                        "Reset to solid color"
                    })
                    .clicked()
                {
                    fill.gradient = None;
                }
            } else {
                let mut color = draft.mark.color.map(crate::presentation::linear_to_srgb);
                ui.horizontal(|ui| {
                    ui.label("Solid color");
                    if ui.color_edit_button_rgb(&mut color).changed() {
                        draft.mark.color = color.map(crate::presentation::srgb_to_linear);
                    }
                });
                if ui.button("Add linear gradient").clicked() {
                    fill.gradient = Some(crate::document::FillGradient {
                        kind: crate::document::FillGradientKind::Linear,
                        start_color: draft.mark.color,
                        end_color: [1.0; 3],
                        start: glam::Vec2::new(0.0, 0.5),
                        end: glam::Vec2::new(1.0, 0.5),
                    });
                }
            }
        }
        let _ = draft;
        if let Some((id, repeat)) = preview_texture_id {
            self.ensure_fill_texture_preview(ui.ctx(), id, repeat);
            if let Some((_, _, preview)) = &self.fills.texture_preview {
                ui.image((preview.id(), egui::vec2(160.0, 100.0)));
            }
        }
        if import_texture {
            self.import_fill_texture(ui.ctx());
        }
        if changed {
            self.update_fill_vertex(selected_contour, selected, Vec3::from_array(coordinates));
        }
        if insert {
            self.insert_fill_vertex(selected_contour, selected);
        }
        if remove {
            self.remove_fill_vertex(selected_contour, selected);
        }
        if add_hole {
            self.add_fill_hole();
        }
        if remove_hole {
            self.remove_fill_hole(selected_contour - 1);
        }
        if apply || cancel {
            self.finish_fill_contour_edit(apply);
        }
    }

    fn ensure_fill_texture_preview(&mut self, ctx: &egui::Context, id: u64, repeat: bool) {
        if self
            .fills
            .texture_preview
            .as_ref()
            .is_some_and(|(preview_id, preview_repeat, _)| {
                *preview_id == id && *preview_repeat == repeat
            })
        {
            return;
        }
        let Some(asset) = self
            .doc
            .brush_tip_assets
            .iter()
            .find(|asset| asset.id == id)
        else {
            self.fills.texture_preview = None;
            return;
        };
        let Ok(decoded) = asset.decoded() else {
            self.fills.texture_preview = None;
            return;
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [decoded.size[0] as usize, decoded.size[1] as usize],
            &decoded.pixels,
        );
        self.fills.texture_preview = Some((
            id,
            repeat,
            ctx.load_texture(
                format!("fill-texture-{id}-{repeat}"),
                image,
                if repeat {
                    egui::TextureOptions::LINEAR_REPEAT
                } else {
                    egui::TextureOptions::LINEAR
                },
            ),
        ));
    }

    fn import_fill_texture(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .pick_file()
        else {
            return;
        };
        match std::fs::metadata(&path) {
            Ok(metadata)
                if metadata.is_file()
                    && metadata.len() <= crate::brush_tip::MAX_PNG_BYTES as u64 => {}
            Ok(_) => {
                self.status = format!(
                    "Fill textures must be PNG files no larger than {} KiB.",
                    crate::brush_tip::MAX_PNG_BYTES / 1024
                );
                return;
            }
            Err(error) => {
                self.status = format!("Could not inspect fill texture: {error}");
                return;
            }
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = format!("Could not read fill texture: {error}");
                return;
            }
        };
        let provenance = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        let asset = match crate::brush_tip::BrushTipAsset::from_png(bytes, provenance) {
            Ok(asset) => asset,
            Err(error) => {
                self.status = format!("Fill texture import failed: {error}");
                return;
            }
        };
        let id = asset.id;
        let is_new = !self
            .doc
            .brush_tip_assets
            .iter()
            .any(|existing| existing.id == id);
        if let Err(error) = self.doc.add_brush_tip_asset(asset) {
            self.status = format!("Fill texture import failed: {error}");
            return;
        }
        let Some(draft) = self.fills.draft.as_mut() else {
            if is_new {
                self.doc.brush_tip_assets.retain(|asset| asset.id != id);
            }
            return;
        };
        let Some(fill) = draft.mark.fill.as_mut() else {
            return;
        };
        fill.texture = Some(crate::document::FillImageTexture {
            asset_id: id,
            scale: glam::Vec2::ONE,
            rotation: 0.0,
            offset: glam::Vec2::ZERO,
            repeat: true,
            tint: [1.0; 3],
            opacity: 1.0,
        });
        if is_new {
            draft.imported_texture_ids.push(id);
        }
        self.ensure_fill_texture_preview(ctx, id, true);
        self.status = "Fill texture imported into the detached preview. Apply commits it; Cancel removes newly added unused image data.".into();
    }

    fn update_fill_vertex(&mut self, contour: usize, index: usize, position: Vec3) {
        let Some(draft) = self.fills.draft.as_mut() else {
            return;
        };
        if draft.original.is_none() {
            return;
        }
        let previous = if contour == 0 {
            let Some(point) = draft.mark.points.get_mut(index) else {
                return;
            };
            let previous = point.position;
            point.position = position;
            previous
        } else {
            let Some(point) = draft
                .mark
                .fill
                .as_mut()
                .and_then(|fill| fill.holes.get_mut(contour - 1))
                .and_then(|hole| hole.get_mut(index))
            else {
                return;
            };
            let previous = *point;
            *point = position;
            previous
        };
        if let Err(error) = polygon::triangulate(&draft.mark) {
            if contour == 0 {
                draft.mark.points[index].position = previous;
            } else {
                draft.mark.fill.as_mut().unwrap().holes[contour - 1][index] = previous;
            }
            self.status = error;
        }
    }

    fn insert_fill_vertex(&mut self, contour: usize, index: usize) {
        let Some(draft) = self.fills.draft.as_mut() else {
            return;
        };
        if draft.original.is_none() {
            return;
        }
        if contour == 0 {
            if draft.mark.points.len() >= polygon::MAX_CORNERS {
                return;
            }
            let next = (index + 1) % draft.mark.points.len();
            let mut point = draft.mark.points[index];
            point.position = (point.position + draft.mark.points[next].position) * 0.5;
            point.time_ms = 0;
            draft.mark.points.insert(index + 1, point);
        } else {
            let Some(hole) = draft
                .mark
                .fill
                .as_mut()
                .and_then(|fill| fill.holes.get_mut(contour - 1))
            else {
                return;
            };
            if hole.len() >= polygon::MAX_CORNERS {
                return;
            }
            let next = (index + 1) % hole.len();
            hole.insert(index + 1, (hole[index] + hole[next]) * 0.5);
        }
        if let Err(error) = polygon::triangulate(&draft.mark) {
            if contour == 0 {
                draft.mark.points.remove(index + 1);
            } else {
                draft.mark.fill.as_mut().unwrap().holes[contour - 1].remove(index + 1);
            }
            self.status = error;
        } else {
            draft.selected = index + 1;
        }
    }

    fn remove_fill_vertex(&mut self, contour: usize, index: usize) {
        let Some(draft) = self.fills.draft.as_mut() else {
            return;
        };
        if draft.original.is_none() {
            return;
        }
        let (previous_point, previous_hole) = if contour == 0 {
            if draft.mark.points.len() <= 3 {
                return;
            }
            (Some(draft.mark.points.remove(index)), None)
        } else {
            let hole = &mut draft.mark.fill.as_mut().unwrap().holes[contour - 1];
            if hole.len() <= 3 {
                return;
            }
            (None, Some(hole.remove(index)))
        };
        if let Err(error) = polygon::triangulate(&draft.mark) {
            if contour == 0 {
                draft.mark.points.insert(index, previous_point.unwrap());
            } else {
                draft.mark.fill.as_mut().unwrap().holes[contour - 1]
                    .insert(index, previous_hole.unwrap());
            }
            self.status = error;
        } else {
            let count = if contour == 0 {
                draft.mark.points.len()
            } else {
                draft.mark.fill.as_ref().unwrap().holes[contour - 1].len()
            };
            draft.selected = draft.selected.min(count - 1);
        }
    }

    fn add_fill_hole(&mut self) {
        let Some(draft) = self
            .fills
            .draft
            .as_mut()
            .filter(|draft| draft.original.is_some())
        else {
            return;
        };
        let fill = draft.mark.fill.as_ref().unwrap();
        if fill.holes.len() >= polygon::MAX_HOLES {
            self.status = "This fill already has the maximum 64 cutouts.".into();
            return;
        }
        let (u, v, normal) = draft.target.plane.basis();
        let origin = draft.target.plane.origin();
        let projected: Vec<_> = draft
            .mark
            .points
            .iter()
            .map(|point| {
                let offset = point.position - origin;
                glam::Vec2::new(offset.dot(u), offset.dot(v))
            })
            .collect();
        let min = projected.iter().copied().reduce(glam::Vec2::min).unwrap();
        let max = projected.iter().copied().reduce(glam::Vec2::max).unwrap();
        let span = max - min;
        let half = span.min_element() * (0.18 / (fill.holes.len() as f32 + 1.0));
        if half < 0.002 {
            self.status = "The fill is too small for another cutout.".into();
            return;
        }
        let candidate = polygon::triangulate(&draft.mark)
            .ok()
            .into_iter()
            .flatten()
            .find_map(|triangle| {
                let center = triangle
                    .into_iter()
                    .map(|index| draft.mark.points[index].position)
                    .sum::<Vec3>()
                    / 3.0;
                polygon::ray_hit(&draft.mark, center + normal * 2.0, -normal).map(|_| center)
            });
        let Some(center) = candidate else {
            self.status = "Could not find open fill area for a new cutout.".into();
            return;
        };
        let hole = vec![
            center - u * half - v * half,
            center + u * half - v * half,
            center + u * half + v * half,
            center - u * half + v * half,
        ];
        draft.mark.fill.as_mut().unwrap().holes.push(hole);
        match polygon::triangulate(&draft.mark) {
            Ok(_) => {
                draft.selected_contour = draft.mark.fill.as_ref().unwrap().holes.len();
                draft.selected = 0;
            }
            Err(error) => {
                draft.mark.fill.as_mut().unwrap().holes.pop();
                self.status = error;
            }
        }
    }

    fn remove_fill_hole(&mut self, hole_index: usize) {
        let Some(draft) = self
            .fills
            .draft
            .as_mut()
            .filter(|draft| draft.original.is_some())
        else {
            return;
        };
        let holes = &mut draft.mark.fill.as_mut().unwrap().holes;
        if hole_index < holes.len() {
            holes.remove(hole_index);
            draft.selected_contour = 0;
            draft.selected = 0;
        }
    }

    pub(super) fn finish_fill_contour_edit(&mut self, apply: bool) {
        let Some(draft) = self.fills.draft.take() else {
            return;
        };
        let Some(original) = draft.original.as_ref().cloned() else {
            self.fills.draft = Some(draft);
            return;
        };
        if !apply {
            self.cleanup_imported_fill_textures(&draft.imported_texture_ids);
            self.select_mode = true;
            self.status = "Fill contour edit cancelled; the drawing is unchanged.".into();
            return;
        }
        if draft.mark == original {
            self.cleanup_imported_fill_textures(&draft.imported_texture_ids);
            self.select_mode = true;
            self.status = "No contour changes to apply.".into();
            return;
        }
        if !self.doc.editable(&original)
            || !self.doc.strokes.iter().any(|stroke| stroke == &original)
        {
            self.cleanup_imported_fill_textures(&draft.imported_texture_ids);
            self.select_mode = true;
            self.status =
                "The fill changed or its layer is now protected; contour edit cancelled.".into();
            return;
        }
        if let Err(error) = polygon::triangulate(&draft.mark) {
            self.fills.draft = Some(draft);
            self.status = error;
            return;
        }
        let before = crate::editing::snapshot(&self.doc, &HashSet::from([original.id]));
        let mut after = before.clone();
        after[0].1 = draft.mark;
        let edit = Edit::Change { before, after };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.cleanup_imported_fill_textures(&draft.imported_texture_ids);
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate_ids(&self.selected);
        self.select_mode = true;
        self.status = "Fill contour edits applied as one undo step.".into();
    }

    pub(super) fn fill_contour_pointer(&mut self, pos: Pos2, rect: Rect, begin: bool) {
        let Some(draft) = self
            .fills
            .draft
            .as_mut()
            .filter(|draft| draft.original.is_some())
        else {
            return;
        };
        if begin {
            if let Some(handles) = polygon::gradient_handle_positions(&draft.mark) {
                let nearest = handles
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, world)| {
                        crate::gizmo::project(&self.camera, rect, world)
                            .map(|(screen, _)| (index, world, screen.distance(pos)))
                    })
                    .filter(|(_, _, distance)| *distance < 13.0)
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                if let Some((index, origin, _)) = nearest {
                    draft.gradient_drag = Some((index, self.camera, rect, origin));
                    return;
                }
            }
            let mut nearest = None;
            let mut distance = 10.0_f32;
            for (index, point) in draft.mark.points.iter().enumerate() {
                if let Some((screen, _)) = crate::gizmo::project(&self.camera, rect, point.position)
                {
                    let d = screen.distance(pos);
                    if d < distance {
                        distance = d;
                        nearest = Some((0, index, point.position));
                    }
                }
            }
            if let Some(fill) = draft.mark.fill.as_ref() {
                for (hole_index, hole) in fill.holes.iter().enumerate() {
                    for (index, point) in hole.iter().enumerate() {
                        if let Some((screen, _)) = crate::gizmo::project(&self.camera, rect, *point)
                        {
                            let d = screen.distance(pos);
                            if d < distance {
                                distance = d;
                                nearest = Some((hole_index + 1, index, *point));
                            }
                        }
                    }
                }
            }
            let Some((contour, index, origin)) = nearest else {
                return;
            };
            let normal = draft.mark.brush.normal;
            let Some(start) = crate::gizmo::plane_hit(&self.camera, rect, pos, origin, normal)
            else {
                return;
            };
            draft.selected_contour = contour;
            draft.selected = index;
            draft.drag = Some(FillPointDrag {
                contour,
                index,
                origin,
                start,
                normal,
                camera: self.camera,
                rect,
            });
        } else if let Some((index, camera, drag_rect, origin)) = draft.gradient_drag {
            if let Some(world) =
                crate::gizmo::plane_hit(&camera, drag_rect, pos, origin, draft.mark.brush.normal)
            {
                if let (Some(fill), Some(mut gradient)) = (
                    draft.mark.fill.as_ref(),
                    draft.mark.fill.as_ref().and_then(|fill| fill.gradient),
                ) {
                    if let Some(uv) = polygon::gradient_uv_from_world(&draft.mark, world) {
                        if index == 0 {
                            gradient.start = uv;
                        } else {
                            gradient.end = uv;
                        }
                        if gradient.is_valid() {
                            draft.mark.fill = Some(crate::document::PlanarFill {
                                gradient: Some(gradient),
                                ..fill.clone()
                            });
                        }
                    }
                }
            }
        } else if let Some(drag) = &draft.drag {
            if let Some(position) = drag.position(pos) {
                let previous = if drag.contour == 0 {
                    draft.mark.points[drag.index].position
                } else {
                    draft.mark.fill.as_ref().unwrap().holes[drag.contour - 1][drag.index]
                };
                if drag.contour == 0 {
                    draft.mark.points[drag.index].position = position;
                } else {
                    draft.mark.fill.as_mut().unwrap().holes[drag.contour - 1][drag.index] =
                        position;
                }
                if let Err(error) = polygon::triangulate(&draft.mark) {
                    if drag.contour == 0 {
                        draft.mark.points[drag.index].position = previous;
                    } else {
                        draft.mark.fill.as_mut().unwrap().holes[drag.contour - 1][drag.index] =
                            previous;
                    }
                    self.status = error;
                }
            }
        }
    }

    pub(super) fn cancel_fill_tool(&mut self) {
        if let Some(draft) = self.fills.draft.take() {
            self.cleanup_imported_fill_textures(&draft.imported_texture_ids);
        }
        self.fills.enabled = false;
    }

    fn cleanup_imported_fill_textures(&mut self, ids: &[u64]) {
        for id in ids {
            let referenced = self.doc.strokes.iter().any(|stroke| {
                stroke.brush.tip_asset_id == *id
                    || stroke
                        .fill
                        .as_ref()
                        .and_then(|fill| fill.texture)
                        .is_some_and(|texture| texture.asset_id == *id)
            });
            if !referenced {
                self.doc.brush_tip_assets.retain(|asset| asset.id != *id);
            }
        }
        self.fills.texture_preview = None;
    }
    pub(super) fn fill_controls(&mut self, ui: &mut egui::Ui) {
        if self
            .fills
            .draft
            .as_ref()
            .is_some_and(|draft| draft.original.is_some())
        {
            self.fill_contour_controls(ui);
            return;
        }
        if !self.fills.enabled {
            return;
        }
        ui.strong("FILL");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.fills.region_mode, true, "Enclosed region");
            ui.selectable_value(&mut self.fills.region_mode, false, "Trace contour");
        });
        ui.horizontal(|ui| {
            ui.label("Fill color");
            ui.color_edit_button_rgb(&mut self.color);
        });
        if self.fills.region_mode {
            ui.label(if self.active_fill_cylinder().is_some() {
                "Click inside a closed outline on the saved cylinder. Boundaries must be attached to this guide; the finite height is enforced. Enter commits, Escape cancels."
            } else {
                "Click inside a closed outline to preview its fill. Press Enter to commit or Escape to cancel."
            });
            let scope_changed = ui
                .horizontal(|ui| {
                    let visible_changed = ui
                        .selectable_value(
                            &mut self.fills.active_layer_boundaries,
                            false,
                            "Visible layers",
                        )
                        .changed();
                    let active_changed = ui
                        .selectable_value(
                            &mut self.fills.active_layer_boundaries,
                            true,
                            "Active layer",
                        )
                        .changed();
                    visible_changed || active_changed
                })
                .inner;
            if scope_changed
                && self
                    .fills
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.original.is_none())
            {
                self.rebuild_region_preview();
            }
            let tolerance_changed = ui
                .add(
                    egui::Slider::new(&mut self.fills.gap_tolerance, 0.0..=0.05)
                        .text("Close outline gaps (wu)"),
                )
                .changed();
            if tolerance_changed
                && self
                    .fills
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.original.is_none())
            {
                self.rebuild_region_preview();
            }
            if self
                .fills
                .draft
                .as_ref()
                .is_some_and(|draft| draft.original.is_none())
            {
                ui.small("Click twice on the guide to add a temporary closing segment. It affects this preview only.");
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.fills.closing_guides.is_empty(),
                            egui::Button::new("Undo closing guide"),
                        )
                        .clicked()
                    {
                        self.fills.closing_guides.pop();
                        self.fills.closing_start = None;
                        self.rebuild_region_preview();
                    }
                    if ui
                        .add_enabled(
                            !self.fills.closing_guides.is_empty(),
                            egui::Button::new("Clear closing guides"),
                        )
                        .clicked()
                    {
                        self.fills.closing_guides.clear();
                        self.fills.closing_start = None;
                        self.rebuild_region_preview();
                    }
                });
            }
            ui.small(if self.active_fill_cylinder().is_some() && self.fills.active_layer_boundaries {
                "Visible sampled strokes attached to the active cylinder on the active layer define boundaries. Hidden and unrelated strokes are ignored."
            } else if self.active_fill_cylinder().is_some() {
                "Visible sampled strokes attached to the active cylinder define boundaries, including strokes on locked layers. Other guides and free strokes are ignored."
            } else if self.fills.active_layer_boundaries {
                "Only visible coplanar strokes on the active layer define boundaries. Hidden and non-coplanar strokes are ignored. Crossing strokes split regions; nested outlines choose the smallest region. Holes are unsupported."
            } else {
                "Visible coplanar strokes define boundaries, including strokes on locked layers. Hidden and non-coplanar strokes are ignored. Crossing strokes split regions; nested outlines choose the smallest region. Holes are unsupported."
            });
        } else {
            ui.label("Click corners on a plane. Click the first corner or press Enter to close and fill. Backspace removes a corner; Escape cancels.");
        }
        ui.add(egui::Slider::new(&mut self.fills.offset, 0.0..=0.05).text("Paint behind guide"));
        ui.small("Offset follows the captured guide surface and is saved/exported. Zero may cause z-fighting. Fill uses solid color; pressure and brush textures do not apply.");
        let count = self.fills.draft.as_ref().map_or(0, |d| d.mark.points.len());
        ui.label(format!(
            "{count}/{} draft corners (not saved)",
            polygon::MAX_CORNERS
        ));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    count >= 3 && !self.fills.region_mode,
                    egui::Button::new("Close & fill"),
                )
                .clicked()
            {
                self.commit_fill();
            }
            if ui
                .add_enabled(
                    count > 0 && !self.fills.region_mode,
                    egui::Button::new("Undo corner"),
                )
                .clicked()
            {
                self.remove_fill_corner();
            }
            if ui
                .add_enabled(count > 0, egui::Button::new("Cancel contour"))
                .clicked()
            {
                self.fills.draft = None;
                self.fills.region_anchor = None;
                self.fills.closing_guides.clear();
                self.fills.closing_start = None;
            }
        });
        if let Some(draft) = &self.fills.draft {
            ui.small(format!("Captured layer {} and surface: finish/cancel before switching drawings. Guide changes do not move this contour.",draft.mark.layer_id));
        }
        if ui.button("Open painted-house study").clicked() {
            self.request(Pending::FilledStudy);
        }
    }
    pub(super) fn remove_fill_corner(&mut self) {
        if self.fills.region_mode && self.fills.draft.is_some() {
            if self.fills.closing_start.take().is_none() {
                self.fills.closing_guides.pop();
                self.rebuild_region_preview();
            }
            self.status = "Temporary closing guide removed. It will not be saved.".into();
            return;
        }
        if let Some(draft) = &mut self.fills.draft {
            draft.mark.points.pop();
            if draft.mark.points.is_empty() {
                self.fills.draft = None;
            }
        }
    }
    fn rebuild_region_preview(&mut self) {
        let Some(anchor) = self.fills.region_anchor else {
            return;
        };
        let Some(target) = self
            .fills
            .draft
            .as_ref()
            .filter(|draft| draft.original.is_none())
            .map(|draft| draft.target)
        else {
            return;
        };
        let cylinder = self.fills.draft.as_ref().and_then(|draft| draft.cylinder);
        let region = if let Some((guide_id, surface)) = cylinder {
            crate::region_fill::find_cylinder_region(
                &self.doc,
                guide_id,
                surface,
                anchor,
                self.fills.gap_tolerance,
                self.fills
                    .active_layer_boundaries
                    .then_some(self.doc.active_layer),
                &self.fills.closing_guides,
            )
        } else {
            crate::region_fill::find_region(
                &self.doc,
                target.plane,
                anchor,
                self.fills.gap_tolerance,
                target.half_extent(),
                self.fills
                    .active_layer_boundaries
                    .then_some(self.doc.active_layer),
                &self.fills.closing_guides,
            )
        };
        match region {
            Ok(points) => {
                if let Some(draft) = self.fills.draft.as_mut() {
                    draft.mark.points = points
                        .into_iter()
                        .map(|position| Point {
                            position,
                            pressure: 1.0,
                            tilt: None,
                            time_ms: 0,
                        })
                        .collect();
                    if let Err(error) = polygon::triangulate(&draft.mark) {
                        draft.mark.points.clear();
                        self.status = error;
                    }
                }
            }
            Err(error) => {
                if let Some(draft) = self.fills.draft.as_mut() {
                    draft.mark.points.clear();
                }
                self.status = error;
            }
        }
        self.renderer.invalidate();
    }
    pub(super) fn fill_corner(&mut self, pos: Pos2, rect: Rect) {
        let cylinder = self.active_fill_cylinder();
        let chart_target = || PlanarTarget::from_curve_frame(Vec3::ZERO, Vec3::Z, Vec3::X, None);
        let Some(target) = self.planar_drawing_target().or_else(|| {
            (self.fills.region_mode && cylinder.is_some())
                .then(chart_target)
                .flatten()
        }) else {
            self.status =
                "Fills require a plane guide or an active saved cylinder for enclosed-region mode."
                    .into();
            return;
        };
        if !self.doc.active_editable() {
            self.status = "Choose a visible unlocked layer for a fill.".into();
            return;
        }
        if self.fills.region_mode && self.fills.draft.is_none() {
            let (o, d) = crate::gizmo::ray(&self.camera, rect, pos);
            let world = match cylinder {
                Some((_, surface)) => cylinder_fill_hit(surface, o, d),
                None => target.hit(o, d),
            };
            let Some(world) = world else {
                self.status =
                    "No hit on the active fill surface. Click the guide surface or orbit the view."
                        .into();
                return;
            };
            let mut cylinder = cylinder;
            let region = if let Some((guide_id, mut surface)) = cylinder {
                let local = surface.rotation.inverse() * (world - surface.center);
                surface.angle_origin = local.x.atan2(local.z);
                cylinder = Some((guide_id, surface));
                crate::region_fill::find_cylinder_region(
                    &self.doc,
                    guide_id,
                    surface,
                    world,
                    self.fills.gap_tolerance,
                    self.fills
                        .active_layer_boundaries
                        .then_some(self.doc.active_layer),
                    &self.fills.closing_guides,
                )
            } else {
                crate::region_fill::find_region(
                    &self.doc,
                    target.plane,
                    world,
                    self.fills.gap_tolerance,
                    target.half_extent(),
                    self.fills
                        .active_layer_boundaries
                        .then_some(self.doc.active_layer),
                    &self.fills.closing_guides,
                )
            };
            match region {
                Ok(points) => {
                    let cylinder_frame = cylinder.map(|(_, surface)| surface);
                    let (normal, tangent) = cylinder_frame.map_or(
                        (target.plane.basis().2, target.plane.basis().0),
                        |surface| {
                            let local = surface.rotation.inverse() * (world - surface.center);
                            let angle = local.x.atan2(local.z);
                            (
                                surface.rotation
                                    * Vec3::new(local.x, 0.0, local.z).normalize_or_zero(),
                                surface.rotation * Vec3::new(angle.cos(), 0.0, -angle.sin()),
                            )
                        },
                    );
                    let mark = Stroke {
                        guide_attachment: None,
                        model_surface: None,
                        curve: None,
                        material: Default::default(),
                        point_thickness: None,
                        point_colors: None,
                        modifiers: None,
                        fill: Some(PlanarFill {
                            offset: self.fills.offset,
                            gradient: None,
                            texture: None,
                            holes: Vec::new(),
                            cylinder: cylinder_frame,
                        }),
                        group_id: None,
                        layer_id: self.doc.active_layer,
                        id: 0,
                        color: self.color,
                        gradient: None,
                        radius: 0.025,
                        brush: Brush {
                            normal,
                            tangent,
                            ..Default::default()
                        },
                        points: points
                            .into_iter()
                            .map(|position| Point {
                                position,
                                pressure: 1.0,
                                tilt: None,
                                time_ms: 0,
                            })
                            .collect(),
                    };
                    if let Err(error) = polygon::triangulate(&mark) {
                        self.status = error;
                        return;
                    }
                    self.fills.draft = Some(FillDraft {
                        mark,
                        target,
                        cylinder,
                        original: None,
                        selected_contour: 0,
                        selected: 0,
                        drag: None,
                        gradient_drag: None,
                        imported_texture_ids: Vec::new(),
                    });
                    self.fills.region_anchor = Some(world);
                    self.fills.closing_guides.clear();
                    self.fills.closing_start = None;
                    self.status = "Region preview ready. Enter commits; Escape cancels.".into();
                }
                Err(error) => self.status = error,
            }
            return;
        }
        if self.fills.region_mode && self.fills.draft.is_some() {
            let Some(draft) = self.fills.draft.as_ref() else {
                return;
            };
            let (o, d) = crate::gizmo::ray(&self.camera, rect, pos);
            let world = match draft.cylinder {
                Some((_, surface)) => cylinder_fill_hit(surface, o, d),
                None => draft.target.hit(o, d),
            };
            let Some(world) = world else {
                self.status =
                    "Click on the active guide to add a temporary closing segment.".into();
                return;
            };
            if let Some(start) = self.fills.closing_start.take() {
                self.fills.closing_guides.push((start, world));
                self.rebuild_region_preview();
                self.status =
                    "Temporary closing guide added. Enter commits only the fill; Escape cancels."
                        .into();
            } else {
                self.fills.closing_start = Some(world);
                self.status = "Closing guide start set. Click its endpoint; Backspace removes the last guide.".into();
            }
            return;
        }
        if let Some(draft) = &self.fills.draft {
            if draft.mark.points.len() >= 3
                && crate::gizmo::project(&self.camera, rect, draft.mark.points[0].position)
                    .is_some_and(|(p, _)| p.distance(pos) <= 10.0)
            {
                self.commit_fill();
                return;
            }
            if draft.mark.points.len() >= polygon::MAX_CORNERS {
                self.status = "Fill corner limit reached; close or remove a corner.".into();
                return;
            }
        }
        let target = self
            .fills
            .draft
            .as_ref()
            .map_or(target, |draft| draft.target);
        let plane = target.plane;
        let (o, d) = crate::gizmo::ray(&self.camera, rect, pos);
        let Some(world) = target.hit(o, d) else {
            self.status = "Plane is edge-on; align the view with F.".into();
            return;
        };
        if world.abs().max_element() > 10000.0 {
            self.status = "Fill corner exceeds document bounds.".into();
            return;
        }
        let draft = self.fills.draft.get_or_insert_with(|| FillDraft {
            target,
            cylinder: None,
            original: None,
            selected_contour: 0,
            selected: 0,
            drag: None,
            gradient_drag: None,
            imported_texture_ids: Vec::new(),
            mark: Stroke {
                guide_attachment: None,
                model_surface: None,
                curve: None,
                material: Default::default(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: Some(PlanarFill {
                    offset: self.fills.offset,
                    gradient: None,
                    texture: None,
                    holes: Vec::new(),
                    cylinder: None,
                }),
                group_id: None,
                layer_id: self.doc.active_layer,
                id: 0,
                color: self.color,
                gradient: None,
                radius: 0.025,
                brush: Brush {
                    normal: plane.basis().2,
                    tangent: plane.basis().0,
                    ..Default::default()
                },
                points: Vec::new(),
            },
        });
        if draft
            .mark
            .points
            .last()
            .is_some_and(|p| p.position.distance(world) < 0.00001)
        {
            return;
        }
        draft.mark.points.push(Point {
            position: world,
            pressure: 1.0,
            tilt: None,
            time_ms: 0,
        });
        self.status =
            "Fill draft: add corners, then Enter or click the first corner. Escape cancels.".into();
    }
    pub(super) fn commit_fill(&mut self) {
        if self
            .fills
            .draft
            .as_ref()
            .is_some_and(|draft| draft.original.is_some())
        {
            self.finish_fill_contour_edit(true);
            return;
        }
        let Some(draft) = &self.fills.draft else {
            return;
        };
        if !self.doc.editable(&draft.mark) {
            self.status = "The captured fill layer is hidden, locked, or missing.".into();
            return;
        }
        let mut mark = draft.mark.clone();
        let cylinder = draft.mark.fill.as_ref().and_then(|fill| fill.cylinder);
        mark.color = self.color;
        mark.fill = Some(PlanarFill {
            offset: self.fills.offset,
            gradient: None,
            texture: None,
            holes: Vec::new(),
            cylinder,
        });
        if let Err(error) = polygon::triangulate(&mark) {
            self.status = error;
            return;
        }
        if self.next_id == u64::MAX
            || self.doc.strokes.len() >= 100_000
            || self
                .doc
                .strokes
                .iter()
                .map(|s| s.points.len())
                .sum::<usize>()
                + mark.points.len()
                > 2_000_000
        {
            self.status = "Document mark limit reached.".into();
            return;
        }
        mark.id = self.next_id;
        self.next_id += 1;
        let edit = Edit::Add(mark);
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.fills.draft = None;
        self.fills.region_anchor = None;
        self.fills.closing_guides.clear();
        self.fills.closing_start = None;
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status =
            "Fill created. Draw outlines on the same guide surface; V selects the filled interior."
                .into();
    }
    pub(super) fn fill_shortcuts(&mut self, ctx: &egui::Context) {
        let (p, enter, back, escape) = ctx.input(|i| {
            (
                i.key_pressed(Key::P),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Escape),
            )
        });
        if self
            .fills
            .draft
            .as_ref()
            .is_some_and(|draft| draft.original.is_some())
        {
            if enter {
                self.finish_fill_contour_edit(true);
            }
            if escape {
                self.finish_fill_contour_edit(false);
            }
            return;
        }
        if p && !ctx.input(|i| i.modifiers.command || i.modifiers.alt)
            && self.planar_drawing_target().is_none()
            && self.active_fill_cylinder().is_none()
        {
            self.status =
                "Fills require a plane guide or a saved cylinder in enclosed-region mode.".into();
        } else if p && !ctx.input(|i| i.modifiers.command || i.modifiers.alt) {
            self.cancel_line_tool();
            self.finish();
            self.fills.enabled = true;
            if self.planar_drawing_target().is_none() {
                self.fills.region_mode = true;
            }
            self.select_mode = false;
            self.eraser = false;
        }
        if self.fills.enabled {
            if enter {
                self.commit_fill();
            }
            if back {
                self.remove_fill_corner();
            }
            if escape {
                self.fills.draft = None;
                self.fills.region_anchor = None;
                self.fills.closing_guides.clear();
                self.fills.closing_start = None;
            }
        }
    }
    pub(super) fn fill_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(draft) = &self.fills.draft else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let color = ui_style::canvas_overlay_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        );
        let projected: Vec<_> = draft
            .mark
            .points
            .iter()
            .map(|p| crate::gizmo::project(&self.camera, rect, p.position))
            .collect();
        if draft.mark.fill.is_some() {
            let vertices = polygon::mesh(&draft.mark);
            let preview_texture = draft
                .mark
                .fill
                .as_ref()
                .and_then(|fill| fill.texture)
                .and_then(|texture| {
                    self.fills
                        .texture_preview
                        .as_ref()
                        .filter(|(id, repeat, _)| {
                            *id == texture.asset_id && *repeat == texture.repeat
                        })
                });
            let mut textured_mesh =
                preview_texture.map(|(_, _, texture)| egui::Mesh::with_texture(texture.id()));
            for tri in vertices.as_chunks::<3>().0 {
                let projected_triangle = tri
                    .iter()
                    .map(|vertex| {
                        crate::gizmo::project(&self.camera, rect, Vec3::from_array(vertex.position))
                            .map(|(screen, _)| (screen, vertex))
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(projected_triangle) = projected_triangle {
                    if let Some(mesh) = &mut textured_mesh {
                        let base = mesh.vertices.len() as u32;
                        for (position, vertex) in projected_triangle {
                            let rgb = vertex.color.map(crate::presentation::linear_to_srgb);
                            let alpha = (vertex.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                            mesh.vertices.push(egui::epaint::Vertex {
                                pos: position,
                                uv: egui::pos2(vertex.uv[0], vertex.uv[1]),
                                color: Color32::from_rgba_unmultiplied(
                                    (rgb[0] * 255.0).round() as u8,
                                    (rgb[1] * 255.0).round() as u8,
                                    (rgb[2] * 255.0).round() as u8,
                                    alpha,
                                ),
                            });
                        }
                        mesh.add_triangle(base, base + 1, base + 2);
                        continue;
                    }
                    let points = projected_triangle
                        .into_iter()
                        .map(|(position, _)| position)
                        .collect::<Vec<_>>();
                    let rgb = tri[0].color.map(crate::presentation::linear_to_srgb);
                    let fill = Color32::from_rgba_unmultiplied(
                        (rgb[0] * 255.0).round() as u8,
                        (rgb[1] * 255.0).round() as u8,
                        (rgb[2] * 255.0).round() as u8,
                        (150.0
                            * tri[0].opacity
                            * self.doc.layer_opacity(draft.mark.layer_id).clamp(0.0, 1.0))
                        .round() as u8,
                    );
                    painter.add(egui::Shape::convex_polygon(
                        points,
                        fill,
                        egui::Stroke::NONE,
                    ));
                }
            }
            if let Some(mesh) = textured_mesh {
                painter.add(egui::Shape::mesh(mesh));
            }
            if let Some(fill) = draft.mark.fill.as_ref() {
                if let Some(gradient) = fill.gradient {
                    if let Some([start, end]) = polygon::gradient_handle_positions(&draft.mark) {
                        if let (Some((a, _)), Some((b, _))) = (
                            crate::gizmo::project(&self.camera, rect, start),
                            crate::gizmo::project(&self.camera, rect, end),
                        ) {
                            painter.line_segment([a, b], egui::Stroke::new(2.0_f32, color));
                            for (point, stop, active) in
                                [(a, gradient.start_color, 0), (b, gradient.end_color, 1)]
                            {
                                let rgb = stop.map(crate::presentation::linear_to_srgb);
                                let stop_color = Color32::from_rgb(
                                    (rgb[0] * 255.0).round() as u8,
                                    (rgb[1] * 255.0).round() as u8,
                                    (rgb[2] * 255.0).round() as u8,
                                );
                                painter.circle_filled(point, 7.0, stop_color);
                                painter.circle_stroke(
                                    point,
                                    8.0,
                                    egui::Stroke::new(
                                        if draft.gradient_drag.is_some_and(|drag| drag.0 == active)
                                        {
                                            3.0_f32
                                        } else {
                                            1.5_f32
                                        },
                                        Color32::WHITE,
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
        for pair in projected.windows(2) {
            if let (Some((a, _)), Some((b, _))) = (pair[0], pair[1]) {
                painter.line_segment([a, b], egui::Stroke::new(2.0_f32, color));
            }
        }
        if let Some(fill) = draft.mark.fill.as_ref() {
            for (hole_index, hole) in fill.holes.iter().enumerate() {
                let hole_projected: Vec<_> = hole
                    .iter()
                    .map(|p| crate::gizmo::project(&self.camera, rect, *p))
                    .collect();
                if !hole_projected.is_empty() {
                    for i in 0..hole_projected.len() {
                        if let (Some((a, _)), Some((b, _))) = (
                            hole_projected[i],
                            hole_projected[(i + 1) % hole_projected.len()],
                        ) {
                            painter.line_segment([a, b], egui::Stroke::new(2.0_f32, color));
                        }
                    }
                }
                if draft.original.is_some() {
                    for (index, point) in hole_projected.iter().enumerate() {
                        if let Some((position, _)) = point {
                            if rect.contains(*position) {
                                let selected = draft.selected_contour == hole_index + 1
                                    && draft.selected == index;
                                painter.circle_filled(
                                    *position,
                                    if selected { 6.0 } else { 4.0 },
                                    if selected { color } else { Color32::WHITE },
                                );
                                painter.circle_stroke(
                                    *position,
                                    6.0,
                                    egui::Stroke::new(1.0_f32, color),
                                );
                            }
                        }
                    }
                }
            }
        }
        for (i, p) in projected.iter().enumerate() {
            if let Some((p, _)) = p {
                painter.circle_stroke(
                    *p,
                    if i == 0 { 7.0 } else { 3.5 },
                    egui::Stroke::new(1.5_f32, color),
                );
                painter.text(
                    *p + egui::vec2(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    (i + 1).to_string(),
                    egui::FontId::proportional(12.0),
                    color,
                );
            }
        }
        if draft.original.is_some() {
            for (index, point) in projected.iter().enumerate() {
                if let Some((position, _)) = point {
                    if rect.contains(*position) {
                        painter.circle_filled(
                            *position,
                            if index == draft.selected { 6.0 } else { 4.0 },
                            if index == draft.selected {
                                color
                            } else {
                                Color32::WHITE
                            },
                        );
                        painter.circle_stroke(*position, 6.0, egui::Stroke::new(1.0_f32, color));
                    }
                }
            }
        }
        for &(a, b) in &self.fills.closing_guides {
            if let (Some((a, _)), Some((b, _))) = (
                crate::gizmo::project(&self.camera, rect, a),
                crate::gizmo::project(&self.camera, rect, b),
            ) {
                painter.line_segment([a, b], egui::Stroke::new(2.5_f32, Color32::YELLOW));
            }
        }
        if let Some(start) = self.fills.closing_start {
            if let Some((start, _)) = crate::gizmo::project(&self.camera, rect, start) {
                painter.circle_filled(start, 4.0, Color32::YELLOW);
            }
        }
    }
    pub(super) fn load_fill_study(&mut self) {
        self.reset_recovery_document();
        self.cancel();
        self.cancel_line_tool();
        self.cancel_fill_tool();
        self.doc = polygon::study();
        self.next_id = self.doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.file = None;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.camera = Camera {
            yaw: 0.55,
            pitch: 0.4,
            distance: 8.0,
            target: Vec3::new(0.0, 1.1, 0.0),
            ..Default::default()
        };
        self.guide = DrawingPlane::from_preset(Plane::Front);
        self.guide.depth = 0.9;
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = false;
        self.starter_scene = None;
        self.cylinder_example = false;
        self.select_mode = false;
        self.eraser = false;
        self.renderer.invalidate();
        self.renderer.lighting = crate::lighting::Lighting {
            shadows: true,
            azimuth: 100.0,
            elevation: 40.0,
            ambient: 0.4,
            intensity: 0.7,
            ..Default::default()
        };
        self.renderer.lighting.fit_ground(&self.doc);
        self.status="Painted house: editable planar walls/roof and separate outlines. P draws another fill; V selects faces.".into();
    }
}

fn cylinder_fill_hit(
    surface: crate::document::CylinderFillSurface,
    origin: Vec3,
    direction: Vec3,
) -> Option<Vec3> {
    if !surface.valid() {
        return None;
    }
    let rotation = surface.rotation.normalize();
    let origin = rotation.inverse() * (origin - surface.center);
    let direction = rotation.inverse() * direction;
    let a = direction.x * direction.x + direction.z * direction.z;
    if a < 1e-10 {
        return None;
    }
    let b = 2.0 * (origin.x * direction.x + origin.z * direction.z);
    let c = origin.x * origin.x + origin.z * origin.z - surface.radius * surface.radius;
    let discriminant = b * b - 4.0 * a * c;
    if !discriminant.is_finite() || discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let t = [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
        .into_iter()
        .find(|t| *t >= 0.0 && (origin.y + direction.y * *t).abs() <= surface.height * 0.5)?;
    Some(surface.center + rotation * (origin + direction * t))
}
