use super::*;
use crate::hatch::{self, Ink};

pub(super) struct HatchDraft {
    source: Stroke,
    ink: Ink,
    preview: Result<Vec<Stroke>, String>,
}

impl SketchApp {
    pub(super) fn begin_hatch(&mut self) {
        if self.repeat.is_some()
            || self.selected.len() != 1
            || self.animation_ui.playing
            || self.camera_playback.is_some()
        {
            return;
        }
        let Some(source) = self
            .doc
            .strokes
            .iter()
            .find(|stroke| {
                self.selected.contains(&stroke.id)
                    && self.doc.editable(stroke)
                    && hatch::supported(stroke)
            })
            .cloned()
        else {
            self.status = "Select one visible, unlocked planar or cylinder fill to hatch.".into();
            return;
        };
        self.finish();
        self.cancel();
        self.select_mode = true;
        let ink = Ink {
            brush: self.brush,
            color: self.color,
            radius: self.radius,
            layer: self.doc.active_layer,
        };
        self.hatch_draft = Some(HatchDraft {
            source,
            ink,
            preview: Ok(Vec::new()),
        });
        self.refresh_hatch_preview();
        self.status = "Adjust hatch angle and spacing, then Apply or Cancel.".into();
    }

    pub(super) fn cancel_hatch(&mut self) {
        if self.hatch_draft.take().is_some() {
            self.renderer.set_stroke_preview(&[], &self.doc);
        }
    }

    fn refresh_hatch_preview(&mut self) {
        let Some(draft) = &mut self.hatch_draft else {
            return;
        };
        draft.preview = hatch::build(
            &self.doc,
            &draft.source,
            self.hatch_settings,
            draft.ink,
            self.next_id,
        );
        let strokes = draft.preview.as_deref().unwrap_or_default();
        self.renderer.set_stroke_preview(strokes, &self.doc);
    }

    fn apply_hatch(&mut self) {
        let Some(draft) = &self.hatch_draft else {
            return;
        };
        if !self
            .doc
            .strokes
            .iter()
            .any(|stroke| stroke == &draft.source)
        {
            self.cancel_hatch();
            self.status =
                "The source fill changed. Select it again to preview a new hatch pass.".into();
            return;
        }
        match hatch::build(
            &self.doc,
            &draft.source,
            self.hatch_settings,
            draft.ink,
            self.next_id,
        ) {
            Ok(strokes) => {
                if draft.preview.as_ref().ok() != Some(&strokes) {
                    self.refresh_hatch_preview();
                    self.status =
                        "The hatch changed. Review the updated preview before applying.".into();
                    return;
                }
                let count = strokes.len();
                let edit = Edit::AddBatch(strokes);
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.next_id += count as u64;
                self.cancel_hatch();
                self.dirty = true;
                self.record_recovery_edit_labeled("Hatch fill");
                self.renderer.invalidate();
                self.status = format!("Added {count} editable hatch strokes as one undo step.");
            }
            Err(error) => {
                self.refresh_hatch_preview();
                self.status = error;
            }
        }
    }

    pub(super) fn hatch_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Hatch fill");
        ui.small("The preview uses the brush captured at the start of this pass. Hatch strokes go on the active layer captured at the same time.");
        let angle_changed = ui
            .add(
                egui::Slider::new(&mut self.hatch_settings.angle, 0.0..=180.0)
                    .text("Angle")
                    .suffix("°"),
            )
            .changed();
        let spacing_changed = ui
            .add(
                egui::Slider::new(
                    &mut self.hatch_settings.spacing,
                    hatch::MIN_SPACING..=hatch::MAX_SPACING,
                )
                .logarithmic(true)
                .text("Spacing")
                .suffix(" wu"),
            )
            .changed();
        if angle_changed || spacing_changed {
            self.refresh_hatch_preview();
        }
        let Some(draft) = &self.hatch_draft else {
            return;
        };
        let ready = match &draft.preview {
            Ok(strokes) => {
                ui.label(format!("{} hatch strokes", strokes.len()));
                !strokes.is_empty()
            }
            Err(error) => {
                ui.colored_label(ui_style::colors(ui.ctx()).warning, error);
                false
            }
        };
        let (apply, cancel) = ui
            .horizontal(|ui| {
                (
                    ui.add_enabled(ready, egui::Button::new("Apply")).clicked(),
                    ui.button("Cancel").clicked(),
                )
            })
            .inner;
        if cancel {
            self.cancel_hatch();
            self.status = "Hatch preview cancelled.".into();
        } else if apply {
            self.apply_hatch();
        }
    }

    pub(super) fn hatch_shortcuts(&mut self, ctx: &egui::Context) {
        if self.hatch_draft.is_none() {
            return;
        }
        let (cancel, apply) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, Key::Escape),
                input.consume_key(egui::Modifiers::NONE, Key::Enter),
            )
        });
        if cancel {
            self.cancel_hatch();
            self.status = "Hatch preview cancelled.".into();
        } else if apply && !ctx.wants_keyboard_input() {
            self.apply_hatch();
        }
    }
}
