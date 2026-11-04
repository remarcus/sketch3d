use super::*;
use crate::placement::{self, SurfaceHit};
pub(super) struct PlacementUi {
    pub active: bool,
    pub hit: Option<SurfaceHit>,
    pub offset: f32,
    pub flip: bool,
    pub verification_frame: Option<u32>,
    pub stamp: Option<StampPlacement>,
}

pub(super) struct StampPlacement {
    pub definition_id: u64,
    pub position: Option<Vec3>,
    pub rotation_degrees: f32,
    pub scale: f32,
    pub linked: bool,
    pub snap: bool,
    pub repeat: bool,
}
impl Default for PlacementUi {
    fn default() -> Self {
        Self {
            active: false,
            hit: None,
            offset: 0.005,
            flip: false,
            verification_frame: None,
            stamp: None,
        }
    }
}
impl SketchApp {
    pub(super) fn begin_placement(&mut self) {
        if self.fills.draft.is_some() || self.line.draft.is_some() {
            self.status = "Finish or cancel the fill contour before placing a plane.".into();
            return;
        }
        self.finish();
        self.placement.active = true;
        self.placement.hit = None;
        self.placement.flip = false;
        self.status="Pick a visible artwork surface. Locked artwork is a read-only reference; Space-drag still navigates.".into();
    }
    pub(super) fn cancel_placement(&mut self) {
        self.placement.active = false;
        self.placement.hit = None;
    }
    pub(super) fn placement_preview(&self) -> Option<DrawingPlane> {
        self.placement.active.then_some(())?;
        self.placement
            .hit
            .map(|hit| hit.plane(self.placement.flip, self.placement.offset))
    }
    pub(super) fn pick_placement(&mut self, pos: Pos2, rect: Rect) {
        let (o, d) = crate::gizmo::ray(&self.camera, rect, pos);
        if let Some(hit) = placement::pick(&self.doc, o, d)
            .filter(|hit| crate::gizmo::project(&self.camera, rect, hit.position).is_some())
        {
            self.placement.hit = Some(hit);
            self.placement.flip = false;
            self.status="Plane preview ready. Adjust offset/normal, then Apply, or Escape to keep the previous guide.".into();
        } else {
            self.status="No visible artwork surface here. Zoom for thin marks; previous guide and preview are unchanged.".into();
        }
    }
    pub(super) fn accept_placement(&mut self, align: bool) {
        let Some(plane) = self.placement_preview() else {
            return;
        };
        let id = self.placement.hit.unwrap().id;
        if !self
            .doc
            .strokes
            .iter()
            .any(|s| s.id == id && self.doc.visible(s))
        {
            self.status = "Reference surface is no longer visible; pick again.".into();
            return;
        }
        self.finish();
        self.guide = plane;
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = true;
        if align {
            self.camera.align_guide(plane);
        }
        self.cancel_placement();
        self.status="Drawing plane placed on artwork. F faces the plane; drawing stays on this plane until you change it.".into();
    }
    pub(super) fn placement_shortcuts(&mut self, ctx: &egui::Context) {
        let (k, enter, escape, mods) = ctx.input(|i| {
            (
                i.key_pressed(Key::K),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.modifiers,
            )
        });
        if k && !mods.shift && !mods.command && !mods.alt {
            self.begin_placement();
        }
        if self.placement.active {
            if escape {
                self.cancel_placement();
                self.status = "Plane placement cancelled; previous guide retained.".into();
            } else if enter {
                self.accept_placement(false);
            }
        }
    }
    pub(super) fn show_placement(&mut self, ctx: &egui::Context) {
        if !self.placement.active
            || self.confirm
            || self.recovery_prompt
            || self.show_help
            || self.tutorial.offer
        {
            return;
        }
        let mut apply = false;
        let mut align = false;
        let mut cancel = false;
        egui::Window::new("Place drawing plane").collapsible(false).resizable(true).default_width(310.0)
            .max_height((ctx.screen_rect().height()-100.0).max(200.0)).vscroll(true)
            .anchor(egui::Align2::LEFT_TOP,egui::vec2(255.0,60.0)).show(ctx,|ui| {
                ui.label("Click/tap artwork to preview a plane. Space-drag or mouse navigation still works. Click another surface to repick.");
                if let Some(hit)=self.placement.hit {
                    let mark=self.doc.strokes.iter().find(|s|s.id==hit.id).unwrap();
                    let layer=self.doc.layers.iter().find(|l|l.id==mark.layer_id).unwrap();
                    ui.strong(format!("Reference: mark {} · {}{}",hit.id,layer.name,if layer.locked {" (locked, read-only)"} else {""}));
                    ui.checkbox(&mut self.placement.flip,"Flip normal");
                    ui.horizontal(|ui| {ui.label("Normal offset");ui.add(egui::DragValue::new(&mut self.placement.offset).speed(0.001).range(-1.0..=1.0).max_decimals(4));});
                    ui.small("Positive offset lifts toward the preview normal. Curved strokes give a local tangent plane, not a fitted wall. Thin strokes need an exact surface hit.");
                } else {ui.label("No surface picked yet.");}
                ui.small("Hidden ink, guides, and the presentation ground are excluded. Locked visible ink can be referenced. This changes workspace guides only, not artwork or history.");
                ui.add_enabled_ui(self.placement.hit.is_some(),|ui| {ui.horizontal(|ui| {apply=ui.button("Apply [Enter]").clicked();align=ui.button("Apply & face plane").clicked();});});
                cancel=ui.button("Cancel [Escape]").clicked();
            });
        if apply || align {
            self.accept_placement(align);
        } else if cancel {
            self.cancel_placement();
        }
    }
    pub(super) fn placement_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(plane) = self.placement_preview() else {
            return;
        };
        let Some((point, _)) = crate::gizmo::project(&self.camera, rect, plane.origin()) else {
            return;
        };
        let length = 70.0 * self.camera.world_per_pixel(plane.origin(), rect.height());
        let color = Color32::from_rgb(255, 175, 225);
        let painter = ui.painter().with_clip_rect(rect);
        painter.circle_stroke(point, 7.0, egui::Stroke::new(2.0_f32, color));
        if let Some((end, _)) = crate::gizmo::project(
            &self.camera,
            rect,
            plane.origin() + plane.basis().2 * length,
        ) {
            painter.arrow(point, end - point, egui::Stroke::new(2.0_f32, color));
            painter.text(
                end,
                egui::Align2::LEFT_BOTTOM,
                "Preview normal",
                egui::FontId::proportional(13.0),
                color,
            );
        }
    }
}
