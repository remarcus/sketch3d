use super::*;

struct TintDrag {
    path: Vec<Pos2>,
    preview: Option<crate::editing::Edit>,
}

pub(super) struct LocalColorUi {
    pub enabled: bool,
    pub radius: f32,
    pub strength: f32,
    pub falloff: crate::liquify::Falloff,
    pub selected_only: bool,
    pub visible_only: bool,
    drag: Option<TintDrag>,
}

impl Default for LocalColorUi {
    fn default() -> Self {
        Self {
            enabled: false,
            radius: 18.0,
            strength: 0.65,
            falloff: crate::liquify::Falloff::Smooth,
            selected_only: false,
            visible_only: true,
            drag: None,
        }
    }
}

impl SketchApp {
    pub(super) fn local_color_active(&self) -> bool {
        self.local_color_brush.drag.is_some()
    }

    pub(super) fn local_color_input(&mut self, position: Pos2, rect: Rect, begin: bool) {
        if begin {
            self.cancel_local_color();
            self.finish();
            self.local_color_brush.drag = Some(TintDrag {
                path: vec![position],
                preview: None,
            });
        } else if let Some(drag) = self.local_color_brush.drag.as_mut() {
            if drag
                .path
                .last()
                .is_none_or(|last| last.distance(position) >= 0.5)
            {
                if drag.path.len() < crate::local_color::MAX_PAINT_PATH_POINTS {
                    drag.path.push(position);
                } else {
                    self.status = "Tint gesture reached its 1,024 point limit.".into();
                }
            }
        } else {
            return;
        }

        let drag = self
            .local_color_brush
            .drag
            .as_mut()
            .expect("tint gesture was initialized");
        if let Some(previous) = drag.preview.take() {
            previous.apply(&mut self.doc, false);
        }
        let settings = crate::local_color::Settings {
            tint: self.color,
            radius_px: self.local_color_brush.radius,
            strength: self.local_color_brush.strength,
            falloff: self.local_color_brush.falloff,
            selected_only: self.local_color_brush.selected_only,
            visible_only: self.local_color_brush.visible_only,
        };
        match crate::local_color::paint(
            &self.doc,
            &self.camera,
            rect,
            &drag.path,
            &self.selected,
            settings,
        ) {
            Ok(Some(edit)) => {
                edit.apply(&mut self.doc, true);
                drag.preview = Some(edit);
                self.status = "Tint preview. Release to apply; Escape cancels.".into();
            }
            Ok(None) => self.status = "No editable strokes under the tint brush.".into(),
            Err(error) => self.status = error,
        }
        self.renderer.invalidate();
    }

    pub(super) fn finish_local_color(&mut self, commit: bool) {
        let Some(mut drag) = self.local_color_brush.drag.take() else {
            return;
        };
        if let Some(edit) = drag.preview.take() {
            if commit {
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.status = "Tint applied as one undo step.".into();
            } else {
                edit.apply(&mut self.doc, false);
                self.status = "Tint cancelled.".into();
            }
            self.renderer.invalidate();
            self.sync_selection();
        }
    }

    pub(super) fn cancel_local_color(&mut self) {
        self.finish_local_color(false);
    }

    pub(super) fn local_color_shortcuts(&mut self, ctx: &egui::Context) {
        if self.local_color_active() && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.cancel_local_color();
        }
    }

    pub(super) fn local_color_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tint brush");
        ui.add(
            egui::Slider::new(&mut self.local_color_brush.radius, 1.0..=250.0)
                .text("Screen radius"),
        );
        ui.add(egui::Slider::new(&mut self.local_color_brush.strength, 0.0..=1.0).text("Strength"));
        egui::ComboBox::from_id_salt("local-color-falloff")
            .selected_text(match self.local_color_brush.falloff {
                crate::liquify::Falloff::Linear => "Linear falloff",
                crate::liquify::Falloff::Smooth => "Smooth falloff",
                crate::liquify::Falloff::Sharp => "Sharp falloff",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.local_color_brush.falloff,
                    crate::liquify::Falloff::Linear,
                    "Linear",
                );
                ui.selectable_value(
                    &mut self.local_color_brush.falloff,
                    crate::liquify::Falloff::Smooth,
                    "Smooth",
                );
                ui.selectable_value(
                    &mut self.local_color_brush.falloff,
                    crate::liquify::Falloff::Sharp,
                    "Sharp",
                );
            });
        ui.checkbox(
            &mut self.local_color_brush.selected_only,
            "Selected strokes only",
        );
        ui.checkbox(
            &mut self.local_color_brush.visible_only,
            "Visible surface only",
        );
        ui.small("Tint color comes from the Color control above. Painted RGB is linear albedo; shading and material stay unchanged.");
        if ui.button("Reset painted colors").clicked() {
            self.finish();
            if let Some(edit) = crate::local_color::reset(
                &self.doc,
                &self.selected,
                self.local_color_brush.selected_only,
            ) {
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.renderer.invalidate();
                self.status = "Painted colors reset as one undo step.".into();
            } else {
                self.status = "No painted colors to reset.".into();
            }
        }
    }
}
