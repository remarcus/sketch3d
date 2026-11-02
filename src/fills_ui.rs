use super::guide_resource_ui::PlanarTarget;
use super::*;
use crate::{document::PlanarFill, polygon};

pub(super) struct FillDraft {
    pub mark: Stroke,
    target: PlanarTarget,
}
pub(super) struct FillUi {
    pub enabled: bool,
    pub offset: f32,
    pub draft: Option<FillDraft>,
    pub verification_frame: Option<u32>,
}
impl Default for FillUi {
    fn default() -> Self {
        Self {
            enabled: false,
            offset: 0.003,
            draft: None,
            verification_frame: None,
        }
    }
}
impl SketchApp {
    pub(super) fn cancel_fill_tool(&mut self) {
        self.fills.draft = None;
        self.fills.enabled = false;
    }
    pub(super) fn fill_controls(&mut self, ui: &mut egui::Ui) {
        if !self.fills.enabled {
            return;
        }
        ui.strong("PLANAR FILL");
        ui.horizontal(|ui| {
            ui.label("Fill color");
            ui.color_edit_button_rgb(&mut self.color);
        });
        ui.label("Click corners on a plane. Click the first corner or press Enter to close and fill. Backspace removes a corner; Escape cancels.");
        ui.add(egui::Slider::new(&mut self.fills.offset, 0.0..=0.05).text("Paint behind plane"));
        ui.small("Offset follows the captured plane normal and is saved/exported. Zero may cause z-fighting. Fill uses solid color; pressure and brush textures do not apply.");
        let count = self.fills.draft.as_ref().map_or(0, |d| d.mark.points.len());
        ui.label(format!(
            "{count}/{} draft corners (not saved)",
            polygon::MAX_CORNERS
        ));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(count >= 3, egui::Button::new("Close & fill"))
                .clicked()
            {
                self.commit_fill();
            }
            if ui
                .add_enabled(count > 0, egui::Button::new("Undo corner"))
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
            mark: Stroke {
                curve: None,
                material: Default::default(),
                fill: Some(PlanarFill {
                    offset: self.fills.offset,
                }),
                group_id: None,
                layer_id: self.doc.active_layer,
                id: 0,
                color: self.color,
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
            if back {
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
