use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NormalAxis {
    U,
    V,
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
        if self.use_cylinder || self.use_revolved {
            return Err("Live mirror drawing currently requires a plane guide.".into());
        }
        let (u, v, _) = self.guide.basis();
        let normal = if self.mirror_ui.axis == NormalAxis::U {
            u
        } else {
            v
        };
        Ok(crate::mirror::MirrorPlane {
            point: self.guide.origin() + normal * self.mirror_ui.offset,
            normal,
        })
    }

    pub(super) fn mirror_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("LIVE MIRROR");
        ui.checkbox(&mut self.mirror_ui.enabled, "Mirror brush and line strokes");
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
            ui.small("The mirror plane follows the active planar guide. Curved guide mirroring is not enabled yet.");
        }
    }

    pub(super) fn commit_drawn_stroke(&mut self, source: Stroke, already_present: bool) {
        let mut mirrored = None;
        let mut mirror_warning = None;
        if self.mirror_ui.enabled {
            match self.mirror_plane().and_then(|plane| {
                crate::mirror::mirror_stroke(&source, plane, self.next_id, 0.0001)
            }) {
                Ok(copy) => mirrored = copy,
                Err(error) => mirror_warning = Some(error),
            }
        }

        if let Some(copy) = mirrored {
            if self.next_id == u64::MAX
                || self.doc.strokes.len() + usize::from(!already_present) + 1 > 100_000
                || self
                    .doc
                    .strokes
                    .iter()
                    .map(|stroke| stroke.points.len())
                    .sum::<usize>()
                    + copy.points.len()
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
                let after = vec![(index, source), (index + 1, copy)];
                let edit = Edit::Change {
                    before: Vec::new(),
                    after,
                };
                edit.apply(&mut self.doc, true);
                self.next_id += 1;
                self.undo.push(edit);
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
                self.renderer.invalidate();
                self.status = "Stroke and mirror committed together. One undo removes both.".into();
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
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status = mirror_warning.unwrap_or_else(|| "Stroke created as editable ink.".into());
    }

    pub(super) fn mirror_preview_overlay(&self, ui: &egui::Ui, rect: Rect, source: &Stroke) {
        if !self.mirror_ui.enabled {
            return;
        }
        let Ok(plane) = self.mirror_plane() else {
            return;
        };
        let Ok(Some(mirrored)) = crate::mirror::mirror_stroke(source, plane, self.next_id, 0.0001)
        else {
            return;
        };
        let points: Vec<_> = mirrored
            .points
            .iter()
            .filter_map(|point| {
                crate::gizmo::project(&self.camera, rect, point.position).map(|(screen, _)| screen)
            })
            .collect();
        if points.len() < 2 {
            return;
        }
        let painter = ui.painter().with_clip_rect(rect);
        let color = Color32::from_rgba_unmultiplied(94, 222, 192, 170);
        for pair in points.windows(2) {
            painter.line_segment([pair[0], pair[1]], egui::Stroke::new(2.0_f32, color));
        }
        if mirrored.fill.is_some() {
            if let (Some(first), Some(last)) = (points.first(), points.last()) {
                painter.line_segment([*last, *first], egui::Stroke::new(2.0_f32, color));
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
