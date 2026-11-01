use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NormalAxis {
    U,
    V,
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
    ui.add_enabled(
        available,
        egui::Checkbox::new(enabled, "Mirror brush and line strokes"),
    )
}

pub(super) struct MirrorUi {
    pub enabled: bool,
    axis: NormalAxis,
    offset: f32,
}

impl Default for MirrorUi {
    fn default() -> Self {
        Self {
            enabled: false,
            axis: NormalAxis::U,
            offset: 0.0,
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

    pub(super) fn mirror_controls(&mut self, ui: &mut egui::Ui) {
        let availability = self.mirror_availability();
        ui.separator();
        ui.strong("LIVE MIRROR");
        mirror_checkbox(ui, &mut self.mirror_ui.enabled, availability.is_ok());
        if let Err(reason) = availability {
            ui.small(reason);
        }
        if self.mirror_ui.enabled {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mirror_ui.axis, NormalAxis::U, "Plane normal U");
                ui.selectable_value(&mut self.mirror_ui.axis, NormalAxis::V, "Plane normal V");
            });
            ui_style::numeric_row(
                ui,
                "Plane offset",
                "mirror-offset",
                &mut self.mirror_ui.offset,
                Some(-10000.0..=10000.0),
                "wu",
            );
            ui.small("On curved guides, the reflected ink follows a world-space symmetry plane and does not snap back to the guide.");
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
            match self.mirror_availability().and_then(|plane| {
                crate::mirror::mirror_stroke_runs(&source, plane, self.next_id, 0.0001)
            }) {
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
                if !already_present {
                    after.push((index, source));
                }
                for (copy_index, copy) in mirrored.into_iter().enumerate() {
                    after.push((index + usize::from(!already_present) + copy_index, copy));
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
                    "Stroke and {copy_count} mirrored run(s) committed together. One undo removes them all."
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
        let Ok(plane) = self.mirror_availability() else {
            return;
        };
        let Ok(mirrored) = crate::mirror::mirror_stroke_runs(source, plane, self.next_id, 0.0001)
        else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let color =
            ui_style::canvas_accent(self.presentation.preferences.background).linear_multiply(0.72);
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
