use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NormalAxis {
    U,
    V,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SymmetryMode {
    #[default]
    Mirror,
    Radial,
}

fn mirror_tool_supported(
    selecting: bool,
    erasing: bool,
    filling: bool,
    shaping: bool,
    curving: bool,
) -> bool {
    !(selecting || erasing || filling || shaping || curving)
}

fn mirror_target_supported(drawing_on_model_surface: bool) -> bool {
    !drawing_on_model_surface
}

fn mirror_checkbox(ui: &mut egui::Ui, enabled: &mut bool, available: bool) -> egui::Response {
    symmetry_checkbox(ui, enabled, available, "Mirror brush and line strokes")
}

fn symmetry_checkbox(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    available: bool,
    label: &str,
) -> egui::Response {
    ui.add_enabled(available, egui::Checkbox::new(enabled, label))
}

pub(super) struct MirrorUi {
    pub enabled: bool,
    mode: SymmetryMode,
    axis: NormalAxis,
    offset: f32,
    radial_copy_count: usize,
    radial_offset_degrees: f32,
    radial_center: Option<Vec3>,
}

impl Default for MirrorUi {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: SymmetryMode::Mirror,
            axis: NormalAxis::U,
            offset: 0.0,
            radial_copy_count: 5,
            radial_offset_degrees: 0.0,
            radial_center: None,
        }
    }
}

impl SketchApp {
    fn mirror_plane(&self) -> Result<crate::mirror::MirrorPlane, String> {
        let (origin, rotation) = if let Some(plane) = self.planar_drawing_target() {
            (plane.plane.origin(), plane.plane.rotation())
        } else if let Some((origin, rotation, _)) = self.saved_guide_camera_frame() {
            (origin, rotation)
        } else if self.use_cylinder {
            (self.cylinder.center, self.cylinder.rotation())
        } else if self.use_revolved {
            (self.revolved.center, self.revolved.rotation())
        } else {
            return Err("Live mirror needs an active plane or curved drawing guide.".into());
        };
        let u = rotation * Vec3::X;
        let v = rotation * Vec3::Y;
        let normal = if self.mirror_ui.axis == NormalAxis::U {
            u
        } else {
            v
        };
        Ok(crate::mirror::MirrorPlane {
            point: origin + normal * self.mirror_ui.offset,
            normal,
        })
    }

    fn mirror_availability(&self) -> Result<crate::mirror::MirrorPlane, String> {
        if !mirror_target_supported(self.doc.active_model_reference.is_some()) {
            return Err("Live mirror is unavailable while drawing on a model surface.".into());
        }
        if !mirror_tool_supported(
            self.select_mode,
            self.eraser,
            self.fills.enabled,
            self.shape_tool.mode.is_some(),
            self.curve_tool.enabled,
        ) {
            return Err(
                "Live mirror is available for brush and straight-line strokes only.".into(),
            );
        }
        self.mirror_plane()
    }

    fn radial_axis_center(&self) -> Result<(Vec3, Vec3), String> {
        if !mirror_tool_supported(
            self.select_mode,
            self.eraser,
            self.fills.enabled,
            self.shape_tool.mode.is_some(),
            self.curve_tool.enabled,
        ) {
            return Err(
                "Radial symmetry is available for brush and straight-line strokes only.".into(),
            );
        }
        if !mirror_target_supported(self.doc.active_model_reference.is_some()) {
            return Err("Radial symmetry is unavailable while drawing on a model surface.".into());
        }
        let target = self.planar_drawing_target().ok_or_else(|| {
            "Radial symmetry needs an active planar drawing guide; curved and model surfaces are unsupported.".to_owned()
        })?;
        let (_, _, axis) = target.plane.basis();
        let center = self
            .mirror_ui
            .radial_center
            .unwrap_or_else(|| target.plane.origin());
        if !center.is_finite()
            || center.abs().max_element() > 10_000.0
            || (center - target.plane.origin()).dot(axis).abs() > 0.001
        {
            return Err("Radial center must be a valid point on the active drawing plane.".into());
        }
        Ok((center, axis))
    }

    pub(super) fn mirrored_runs_for(&self, source: &Stroke) -> Result<Vec<Stroke>, String> {
        self.mirror_availability().and_then(|plane| {
            crate::mirror::mirror_stroke_runs(source, plane, self.next_id, 0.0001)
        })
    }

    pub(super) fn symmetry_copies_for(&self, source: &Stroke) -> Result<Vec<Stroke>, String> {
        match self.mirror_ui.mode {
            SymmetryMode::Mirror => self.mirrored_runs_for(source),
            SymmetryMode::Radial => {
                let (center, axis) = self.radial_axis_center()?;
                let already_present = self.doc.strokes.iter().any(|stroke| stroke.id == source.id);
                let current_points: usize = self
                    .doc
                    .strokes
                    .iter()
                    .map(|stroke| stroke.points.len())
                    .sum();
                let proposed_points = source
                    .points
                    .len()
                    .checked_mul(self.mirror_ui.radial_copy_count)
                    .and_then(|count| {
                        count.checked_add(usize::from(!already_present) * source.points.len())
                    })
                    .and_then(|count| count.checked_add(current_points));
                let proposed_strokes = self
                    .doc
                    .strokes
                    .len()
                    .checked_add(self.mirror_ui.radial_copy_count)
                    .and_then(|count| count.checked_add(usize::from(!already_present)));
                if proposed_points.is_none_or(|count| count > 2_000_000)
                    || proposed_strokes.is_none_or(|count| count > 100_000)
                {
                    return Err("Radial copies would exceed document stroke or point limits".into());
                }
                crate::mirror::radial_copies(
                    source,
                    center,
                    axis,
                    self.mirror_ui.radial_copy_count,
                    self.mirror_ui.radial_offset_degrees,
                    self.next_id,
                )
            }
        }
    }

    pub(super) fn mirror_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("LIVE SYMMETRY");
        egui::ComboBox::from_id_salt("live-symmetry-mode")
            .selected_text(match self.mirror_ui.mode {
                SymmetryMode::Mirror => "Mirror",
                SymmetryMode::Radial => "Radial",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.mirror_ui.mode, SymmetryMode::Mirror, "Mirror");
                ui.selectable_value(&mut self.mirror_ui.mode, SymmetryMode::Radial, "Radial");
            });
        let availability = match self.mirror_ui.mode {
            SymmetryMode::Mirror => self.mirror_availability().map(|_| ()),
            SymmetryMode::Radial => self.radial_axis_center().map(|_| ()),
        };
        let label = match self.mirror_ui.mode {
            SymmetryMode::Mirror => "Mirror brush and line strokes",
            SymmetryMode::Radial => "Repeat brush and line strokes radially",
        };
        if self.mirror_ui.mode == SymmetryMode::Mirror {
            mirror_checkbox(ui, &mut self.mirror_ui.enabled, availability.is_ok());
        } else {
            symmetry_checkbox(ui, &mut self.mirror_ui.enabled, availability.is_ok(), label);
        }
        if let Err(reason) = availability {
            ui.small(reason);
        }
        if self.mirror_ui.enabled {
            match self.mirror_ui.mode {
                SymmetryMode::Mirror => {
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.mirror_ui.axis,
                            NormalAxis::U,
                            "Plane normal U",
                        );
                        ui.selectable_value(
                            &mut self.mirror_ui.axis,
                            NormalAxis::V,
                            "Plane normal V",
                        );
                    });
                    ui_style::numeric_row(
                        ui,
                        "Plane offset",
                        "mirror-offset",
                        &mut self.mirror_ui.offset,
                        Some(-10000.0..=10000.0),
                        "wu",
                    );
                    ui.small("On curved guides, reflected ink follows a world-space plane and does not snap back to the guide.");
                }
                SymmetryMode::Radial => {
                    let mut center = self.mirror_ui.radial_center.unwrap_or_else(|| {
                        self.planar_drawing_target()
                            .map_or(Vec3::ZERO, |target| target.plane.origin())
                    });
                    ui.horizontal(|ui| {
                        ui.label("World center");
                        ui.add(
                            egui::DragValue::new(&mut center.x)
                                .speed(0.05)
                                .range(-10000.0..=10000.0)
                                .prefix("X "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut center.y)
                                .speed(0.05)
                                .range(-10000.0..=10000.0)
                                .prefix("Y "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut center.z)
                                .speed(0.05)
                                .range(-10000.0..=10000.0)
                                .prefix("Z "),
                        );
                    });
                    self.mirror_ui.radial_center = Some(center);
                    ui.add(
                        egui::Slider::new(
                            &mut self.mirror_ui.radial_copy_count,
                            1..=crate::mirror::MAX_RADIAL_COPIES,
                        )
                        .text("Additional copies"),
                    );
                    ui.add(
                        egui::Slider::new(
                            &mut self.mirror_ui.radial_offset_degrees,
                            -180.0..=180.0,
                        )
                        .text("Copy fan offset"),
                    );
                    if let Ok((_, axis)) = self.radial_axis_center() {
                        ui.small(format!(
                            "World rotation axis: {:.3}, {:.3}, {:.3}. Change the active plane to reorient it.",
                            axis.x, axis.y, axis.z
                        ));
                    }
                    ui.small("The source remains fixed; the offset rotates only the evenly spaced duplicate fan. Planar guides only.");
                }
            }
        }
    }

    pub(super) fn commit_drawn_stroke(
        &mut self,
        source: Stroke,
        already_present: bool,
        duration_ms: u64,
    ) {
        let recorded_stroke = source.clone();
        let mut mirrored = Vec::new();
        let mut mirror_warning = None;
        if self.mirror_ui.enabled {
            match self.symmetry_copies_for(&source) {
                Ok(copies) => mirrored = copies,
                Err(error) => mirror_warning = Some(error),
            }
        }

        if !mirrored.is_empty() {
            let copy_count = mirrored.len();
            let added_strokes = copy_count + usize::from(!already_present);
            let added_points: usize = mirrored.iter().map(|copy| copy.points.len()).sum();
            let current_points: usize = self
                .doc
                .strokes
                .iter()
                .map(|stroke| stroke.points.len())
                .sum();
            let id_end = self
                .next_id
                .checked_add(copy_count as u64)
                .filter(|_| self.next_id != u64::MAX);
            if id_end.is_none()
                || self.next_id == u64::MAX
                || self.doc.strokes.len() + added_strokes > 100_000
                || current_points
                    + added_points
                    + usize::from(!already_present) * source.points.len()
                    > 2_000_000
            {
                mirror_warning = Some("Mirror copy would exceed document limits".into());
            } else {
                let index = if already_present {
                    self.doc
                        .strokes
                        .iter()
                        .position(|stroke| stroke.id == source.id)
                        .unwrap_or(self.doc.strokes.len())
                } else {
                    self.doc.strokes.len()
                };
                let mut after = Vec::with_capacity(added_strokes);
                // The live source exists in the document while its pointer
                // gesture is in progress. Include it in the transaction so
                // one undo removes the source and every reflected run.
                after.push((index, source.clone()));
                for (copy_index, copy) in mirrored.into_iter().enumerate() {
                    after.push((index + 1 + copy_index, copy));
                }
                let edit = Edit::Change {
                    before: Vec::new(),
                    after,
                };
                edit.apply(&mut self.doc, true);
                if let Some(id_end) = id_end {
                    self.next_id = id_end;
                }
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_stroke_labeled("Draw", &recorded_stroke, duration_ms);
                self.renderer.invalidate();
                self.status = format!(
                    "Stroke and {copy_count} symmetry copy/copies committed together. One undo removes them all."
                );
                return;
            }
        }

        if already_present {
            self.undo.push(Edit::Add(source));
        } else {
            let edit = Edit::Add(source);
            edit.apply(&mut self.doc, true);
            self.undo.push(edit);
        }
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_stroke_labeled("Draw", &recorded_stroke, duration_ms);
        self.renderer.invalidate();
        self.status = mirror_warning.unwrap_or_else(|| "Stroke created as editable ink.".into());
    }

    pub(super) fn mirror_preview_overlay(&self, ui: &egui::Ui, rect: Rect, source: &Stroke) {
        if !self.mirror_ui.enabled {
            return;
        }
        let Ok(mirrored) = self.symmetry_copies_for(source) else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let color = ui_style::canvas_overlay_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        )
        .linear_multiply(0.72);
        if self.mirror_ui.mode == SymmetryMode::Radial {
            if let Ok((center, axis)) = self.radial_axis_center() {
                let axis_half_length = self.camera.world_per_pixel(center, rect.height()) * 40.0;
                if let (Some((center_screen, _)), Some((axis_start, _)), Some((axis_end, _))) = (
                    crate::gizmo::project(&self.camera, rect, center),
                    crate::gizmo::project(&self.camera, rect, center - axis * axis_half_length),
                    crate::gizmo::project(&self.camera, rect, center + axis * axis_half_length),
                ) {
                    painter.line_segment([axis_start, axis_end], egui::Stroke::new(1.5_f32, color));
                    painter.circle_stroke(
                        center_screen,
                        5.0_f32,
                        egui::Stroke::new(2.0_f32, color),
                    );
                }
            }
        }
        for mirrored in mirrored {
            let points: Vec<_> = mirrored
                .points
                .iter()
                .filter_map(|point| {
                    crate::gizmo::project(&self.camera, rect, point.position)
                        .map(|(screen, _)| screen)
                })
                .collect();
            for pair in points.windows(2) {
                painter.line_segment([pair[0], pair[1]], egui::Stroke::new(2.0_f32, color));
            }
            if mirrored.fill.is_some() {
                if let (Some(first), Some(last)) = (points.first(), points.last()) {
                    painter.line_segment([*last, *first], egui::Stroke::new(2.0_f32, color));
                }
            }
        }
    }

    pub(super) fn mirror_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(id) = self.active else {
            return;
        };
        if let Some(source) = self.doc.strokes.iter().find(|stroke| stroke.id == id) {
            self.mirror_preview_overlay(ui, rect, source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mirror_checkbox, mirror_target_supported, mirror_tool_supported};
    use eframe::egui::{self, Pos2, Rect, Vec2};

    #[test]
    fn live_mirror_is_limited_to_brush_and_line_modes() {
        assert!(mirror_tool_supported(false, false, false, false, false));
        for unsupported_mode in 0..5 {
            let mut modes = [false; 5];
            modes[unsupported_mode] = true;
            assert!(
                !mirror_tool_supported(modes[0], modes[1], modes[2], modes[3], modes[4]),
                "mode {unsupported_mode} must disable live mirror"
            );
        }
    }

    #[test]
    fn live_mirror_is_disabled_for_model_surface_strokes() {
        assert!(mirror_target_supported(false));
        assert!(!mirror_target_supported(true));
    }

    #[test]
    fn mirror_checkbox_rejects_clicks_when_unavailable() {
        fn frame(
            ctx: &egui::Context,
            events: Vec<egui::Event>,
            available: bool,
            enabled: &mut bool,
        ) -> egui::Response {
            let mut response = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 480.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        response = Some(mirror_checkbox(ui, enabled, available));
                    });
                },
            );
            response.expect("mirror checkbox renders")
        }

        fn click(ctx: &egui::Context, position: Pos2, available: bool, enabled: &mut bool) {
            let _ = frame(
                ctx,
                vec![egui::Event::PointerMoved(position)],
                available,
                enabled,
            );
            let _ = frame(
                ctx,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
                available,
                enabled,
            );
            let _ = frame(
                ctx,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                available,
                enabled,
            );
        }

        let ctx = egui::Context::default();
        let mut enabled = false;
        let initial = frame(&ctx, Vec::new(), false, &mut enabled);
        assert!(!initial.enabled());
        click(&ctx, initial.rect.center(), false, &mut enabled);
        assert!(!enabled, "an unavailable mirror option cannot turn on");

        let available = frame(&ctx, Vec::new(), true, &mut enabled);
        assert!(available.enabled());
        click(&ctx, available.rect.center(), true, &mut enabled);
        assert!(enabled, "the option can be turned on for supported modes");
    }
}
