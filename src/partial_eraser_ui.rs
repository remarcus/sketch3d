use super::*;

pub(super) struct PartialEraseDrag {
    path: Vec<Pos2>,
    preview: Option<crate::partial_eraser::PartialErasePlan>,
}

pub(super) struct PartialEraserUi {
    pub enabled: bool,
    pub radius: f32,
    drag: Option<PartialEraseDrag>,
}

impl Default for PartialEraserUi {
    fn default() -> Self {
        Self {
            enabled: false,
            radius: 12.0,
            drag: None,
        }
    }
}

impl SketchApp {
    pub(super) fn partial_erase_active(&self) -> bool {
        self.partial_eraser.drag.is_some()
    }

    pub(super) fn eraser_input(&mut self, position: Pos2, rect: Rect, begin: bool) {
        if self.partial_eraser.enabled {
            self.partial_erase_input(position, rect, begin);
        } else {
            self.erase(position, rect);
        }
    }

    pub(super) fn partial_erase_input(&mut self, position: Pos2, rect: Rect, begin: bool) {
        if begin {
            self.cancel_partial_erase();
            self.finish();
            self.partial_eraser.drag = Some(PartialEraseDrag {
                path: vec![position],
                preview: None,
            });
        } else if let Some(drag) = self.partial_eraser.drag.as_mut() {
            if drag
                .path
                .last()
                .is_none_or(|last| last.distance(position) >= 1.0)
                && drag.path.len() < 1024
            {
                drag.path.push(position);
            }
        } else {
            return;
        }

        let drag = self
            .partial_eraser
            .drag
            .as_mut()
            .expect("erase drag was initialized");
        if let Some(previous) = drag.preview.take() {
            previous.edit.apply(&mut self.doc, false);
        }
        match crate::partial_eraser::plan_partial_erase(
            &self.doc,
            &self.camera,
            rect,
            &drag.path,
            self.partial_eraser.radius,
            self.next_id,
        ) {
            Ok(Some(plan)) => {
                plan.edit.apply(&mut self.doc, true);
                self.status = if plan.split > 0 {
                    format!(
                        "Preview: split {} stroke(s), remove {} mark(s). Release to apply.",
                        plan.split, plan.removed
                    )
                } else {
                    format!(
                        "Preview: remove {} mark(s). Release to apply.",
                        plan.removed
                    )
                };
                drag.preview = Some(plan);
            }
            Ok(None) => {
                self.status = "No editable strokes under the eraser.".into();
            }
            Err(error) => self.status = error,
        }
        self.renderer.invalidate();
    }

    pub(super) fn finish_partial_erase(&mut self, commit: bool) {
        let Some(mut drag) = self.partial_eraser.drag.take() else {
            return;
        };
        if let Some(plan) = drag.preview.take() {
            if commit {
                self.next_id = plan.next_id;
                self.undo.push(plan.edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.status = "Partial erase applied as one undo step.".into();
            } else {
                plan.edit.apply(&mut self.doc, false);
                self.status = "Partial erase cancelled.".into();
            }
            self.renderer.invalidate();
            self.sync_selection();
        }
    }

    pub(super) fn cancel_partial_erase(&mut self) {
        self.finish_partial_erase(false);
    }

    pub(super) fn partial_eraser_controls(&mut self, ui: &mut egui::Ui) {
        if !self.eraser {
            return;
        }
        ui.checkbox(&mut self.partial_eraser.enabled, "Partial stroke eraser");
        if self.partial_eraser.enabled {
            ui.add(
                egui::Slider::new(&mut self.partial_eraser.radius, 2.0..=80.0)
                    .text("Screen radius"),
            );
            ui.small("Drag to split ordinary strokes. Filled marks erase as a whole. Escape cancels the gesture.");
        } else {
            ui.small("Erase whole strokes under the pointer.");
        }
    }
}
