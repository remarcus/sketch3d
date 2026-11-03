use super::guide_resource_ui::PlanarTarget;
use super::*;
use crate::{document::PlanarFill, polygon};

pub(super) struct FillDraft {
    pub mark: Stroke,
    target: PlanarTarget,
    pub(super) original: Option<Stroke>,
    selected: usize,
    pub(super) drag: Option<FillPointDrag>,
    pub(super) gradient_drag: Option<(usize, Camera, Rect, Vec3)>,
}
pub(super) struct FillPointDrag {
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
    pub draft: Option<FillDraft>,
    pub verification_frame: Option<u32>,
}
impl Default for FillUi {
    fn default() -> Self {
        Self {
            enabled: false,
            offset: 0.003,
            region_mode: false,
            gap_tolerance: 0.0,
            draft: None,
            verification_frame: None,
        }
    }
}
impl SketchApp {
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
            original: Some(source),
            selected: 0,
            drag: None,
            gradient_drag: None,
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
        ui.small("Move vertices in the fill's captured plane. Insert adds a midpoint after the selected vertex; the contour must remain simple and nondegenerate.");
        let count = draft.mark.points.len();
        ui.horizontal(|ui| {
            ui.label("Vertex (0-based)");
            ui.add(egui::DragValue::new(&mut draft.selected).range(0..=count - 1));
        });
        let mut coordinates = draft.mark.points[draft.selected].position.to_array();
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
        ui.horizontal(|ui| {
            insert = ui.button("Insert midpoint after vertex").clicked();
            remove = ui
                .add_enabled(count > 3, egui::Button::new("Remove vertex"))
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
        if let Some(fill) = draft.mark.fill.as_mut() {
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
                if ui.button("Reset to solid color").clicked() {
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
        if changed {
            self.update_fill_vertex(selected, Vec3::from_array(coordinates));
        }
        if insert {
            self.insert_fill_vertex();
        }
        if remove {
            self.remove_fill_vertex();
        }
        if apply || cancel {
            self.finish_fill_contour_edit(apply);
        }
    }

    fn update_fill_vertex(&mut self, index: usize, position: Vec3) {
        let Some(draft) = self.fills.draft.as_mut() else {
            return;
        };
        if draft.original.is_none() || index >= draft.mark.points.len() {
            return;
        }
        let previous = draft.mark.points[index];
        draft.mark.points[index].position = position;
        if let Err(error) = polygon::triangulate(&draft.mark) {
            draft.mark.points[index] = previous;
            self.status = error;
        }
    }

    fn insert_fill_vertex(&mut self) {
        let Some(draft) = self.fills.draft.as_mut() else {
            return;
        };
        if draft.original.is_none() || draft.mark.points.len() >= polygon::MAX_CORNERS {
            return;
        }
        let index = draft.selected;
        let next = (index + 1) % draft.mark.points.len();
        let mut point = draft.mark.points[index];
        point.position = (point.position + draft.mark.points[next].position) * 0.5;
        point.time_ms = 0;
        draft.mark.points.insert(index + 1, point);
        if let Err(error) = polygon::triangulate(&draft.mark) {
            draft.mark.points.remove(index + 1);
            self.status = error;
        } else {
            draft.selected = index + 1;
        }
    }

    fn remove_fill_vertex(&mut self) {
        let Some(draft) = self.fills.draft.as_mut() else {
            return;
        };
        if draft.original.is_none() || draft.mark.points.len() <= 3 {
            return;
        }
        let previous = draft.mark.points.remove(draft.selected);
        if let Err(error) = polygon::triangulate(&draft.mark) {
            draft.mark.points.insert(draft.selected, previous);
            self.status = error;
        } else {
            draft.selected = draft.selected.min(draft.mark.points.len() - 1);
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
            self.select_mode = true;
            self.status = "Fill contour edit cancelled; the drawing is unchanged.".into();
            return;
        }
        if draft.mark == original {
            self.select_mode = true;
            self.status = "No contour changes to apply.".into();
            return;
        }
        if !self.doc.editable(&original)
            || !self.doc.strokes.iter().any(|stroke| stroke == &original)
        {
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
                        nearest = Some((index, point.position));
                    }
                }
            }
            let Some((index, origin)) = nearest else {
                return;
            };
            let normal = draft.mark.brush.normal;
            let Some(start) = crate::gizmo::plane_hit(&self.camera, rect, pos, origin, normal)
            else {
                return;
            };
            draft.selected = index;
            draft.drag = Some(FillPointDrag {
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
                    draft.mark.fill,
                    draft.mark.fill.and_then(|fill| fill.gradient),
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
                                ..fill
                            });
                        }
                    }
                }
            }
        } else if let Some(drag) = &draft.drag {
            if let Some(position) = drag.position(pos) {
                let previous = draft.mark.points[drag.index];
                draft.mark.points[drag.index].position = position;
                if let Err(error) = polygon::triangulate(&draft.mark) {
                    draft.mark.points[drag.index] = previous;
                    self.status = error;
                }
            }
        }
    }

    pub(super) fn cancel_fill_tool(&mut self) {
        self.fills.draft = None;
        self.fills.enabled = false;
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
        ui.strong("PLANAR FILL");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.fills.region_mode, true, "Enclosed region");
            ui.selectable_value(&mut self.fills.region_mode, false, "Trace contour");
        });
        ui.horizontal(|ui| {
            ui.label("Fill color");
            ui.color_edit_button_rgb(&mut self.color);
        });
        if self.fills.region_mode {
            ui.label("Click inside a closed outline to preview its fill. Press Enter to commit or Escape to cancel.");
            ui.add(
                egui::Slider::new(&mut self.fills.gap_tolerance, 0.0..=0.05)
                    .text("Close outline gaps (wu)"),
            );
            ui.small("Visible coplanar strokes define boundaries, including strokes on locked layers. Hidden and non-coplanar strokes are ignored. Crossing strokes split regions; nested outlines choose the smallest region. Holes are unsupported.");
        } else {
            ui.label("Click corners on a plane. Click the first corner or press Enter to close and fill. Backspace removes a corner; Escape cancels.");
        }
        ui.add(egui::Slider::new(&mut self.fills.offset, 0.0..=0.05).text("Paint behind plane"));
        ui.small("Offset follows the captured plane normal and is saved/exported. Zero may cause z-fighting. Fill uses solid color; pressure and brush textures do not apply.");
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
            }
        });
        if let Some(draft) = &self.fills.draft {
            ui.small(format!("Captured layer {} and plane: finish/cancel before switching drawings. Guide changes do not move this contour.",draft.mark.layer_id));
        }
        if ui.button("Open painted-house study").clicked() {
            self.request(Pending::FilledStudy);
        }
    }
    pub(super) fn remove_fill_corner(&mut self) {
        if let Some(draft) = &mut self.fills.draft {
            draft.mark.points.pop();
            if draft.mark.points.is_empty() {
                self.fills.draft = None;
            }
        }
    }
    pub(super) fn fill_corner(&mut self, pos: Pos2, rect: Rect) {
        let Some(target) = self.planar_drawing_target() else {
            self.status =
                "Planar fills require a plane guide. Activate a planar guide first.".into();
            return;
        };
        if !self.doc.active_editable() {
            self.status = "Choose a visible unlocked layer for a fill.".into();
            return;
        }
        if self.fills.region_mode && self.fills.draft.is_none() {
            let (o, d) = crate::gizmo::ray(&self.camera, rect, pos);
            let Some(world) = target.hit(o, d) else {
                self.status =
                    "Plane is edge-on or the click is outside the active image plane.".into();
                return;
            };
            match crate::region_fill::find_region(
                &self.doc,
                target.plane,
                world,
                self.fills.gap_tolerance,
                target.half_extent(),
            ) {
                Ok(points) => {
                    let mark = Stroke {
                        curve: None,
                        material: Default::default(),
                        point_thickness: None,
                        modifiers: None,
                        fill: Some(PlanarFill {
                            offset: self.fills.offset,
                            gradient: None,
                        }),
                        group_id: None,
                        layer_id: self.doc.active_layer,
                        id: 0,
                        color: self.color,
                        gradient: None,
                        radius: 0.025,
                        brush: Brush {
                            normal: target.plane.basis().2,
                            tangent: target.plane.basis().0,
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
                        original: None,
                        selected: 0,
                        drag: None,
                        gradient_drag: None,
                    });
                    self.status = "Region preview ready. Enter commits; Escape cancels.".into();
                }
                Err(error) => self.status = error,
            }
            return;
        }
        if self.fills.region_mode && self.fills.draft.is_some() {
            self.status =
                "Region preview is active. Press Enter to commit or Escape to cancel.".into();
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
            original: None,
            selected: 0,
            drag: None,
            gradient_drag: None,
            mark: Stroke {
                curve: None,
                material: Default::default(),
                point_thickness: None,
                modifiers: None,
                fill: Some(PlanarFill {
                    offset: self.fills.offset,
                    gradient: None,
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
        mark.color = self.color;
        mark.fill = Some(PlanarFill {
            offset: self.fills.offset,
            gradient: None,
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
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status="Planar fill created. Draw outlines on the same guide plane; V selects the filled interior.".into();
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
        {
            self.status =
                "Planar fills require a plane guide. Activate a planar guide first.".into();
        } else if p && !ctx.input(|i| i.modifiers.command || i.modifiers.alt) {
            self.cancel_line_tool();
            self.finish();
            self.fills.enabled = true;
            self.select_mode = false;
            self.eraser = false;
        }
        if self.fills.enabled {
            if enter {
                self.commit_fill();
            }
            if back && !self.fills.region_mode {
                self.remove_fill_corner();
            }
            if escape {
                self.fills.draft = None;
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
            for tri in vertices.chunks_exact(3) {
                let points = tri
                    .iter()
                    .map(|vertex| {
                        crate::gizmo::project(&self.camera, rect, Vec3::from_array(vertex.position))
                            .map(|(screen, _)| screen)
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(points) = points {
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
            if let Some(fill) = draft.mark.fill {
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
        self.starter_scene = false;
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
