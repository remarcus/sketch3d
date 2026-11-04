use super::ui_style;
use super::*;
use crate::{editing, gizmo};

#[derive(Default)]
pub(super) struct PlanePickUi {
    pub active: bool,
    pub hovered_guide: Option<u64>,
}

impl SketchApp {
    pub(super) fn plane_from_view(&mut self) {
        self.finish();
        self.guide = DrawingPlane::from_view(&self.camera);
        self.use_cylinder = false;
        self.use_revolved = false;
        self.status =
            "Drawing plane placed through the camera focus, facing the current view.".into();
    }
    pub(super) fn plane_controls(&mut self, ui: &mut egui::Ui) {
        self.saved_guide_controls(ui);
        let has_pickable_plane = self.doc.guide_resources.iter().any(|guide| {
            guide.visible
                && !guide.locked
                && matches!(
                    &guide.geometry,
                    crate::document::guide_resources::GuideGeometry::Plane { .. }
                )
        });
        if ui
            .add_enabled(
                has_pickable_plane || self.plane_picker.active,
                egui::Button::new(if self.plane_picker.active {
                    "Picking plane… [Escape]"
                } else {
                    "Pick plane on canvas [Shift+K]"
                }),
            )
            .clicked()
        {
            if self.plane_picker.active {
                self.cancel_plane_picker();
            } else {
                self.begin_plane_picker();
            }
        }
        if self.doc.active_guide.is_some() {
            return;
        }
        if ui.button("Place plane on artwork [K]").clicked() {
            self.begin_placement();
        }
        ui_style::field_row(ui, "Guide type", |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut kind = if self.use_revolved {
                    2
                } else if self.use_cylinder {
                    1
                } else {
                    0
                };
                for (value, label) in [(0, "Plane"), (1, "Cylinder"), (2, "Revolved")] {
                    ui.radio_value(&mut kind, value, label);
                }
                if kind
                    != if self.use_revolved {
                        2
                    } else if self.use_cylinder {
                        1
                    } else {
                        0
                    }
                {
                    self.finish();
                    self.use_cylinder = kind == 1;
                    self.use_revolved = kind == 2;
                }
            });
        });
        if ui.checkbox(&mut self.guide_visible, "Show guide").changed() {
            self.finish();
        }
        if self.use_revolved {
            self.revolved_controls(ui);
            return;
        }
        if self.use_cylinder {
            self.cylinder_controls(ui);
            return;
        }
        let before = self.guide;
        ui_style::field_row(ui, "Preset", |ui| {
            for (plane, label) in [
                (Plane::Front, "Front"),
                (Plane::Side, "Side"),
                (Plane::Ground, "Ground"),
            ] {
                if ui
                    .radio(self.guide.preset() == Some(plane), label)
                    .on_hover_text("Reset orientation and placement to this world plane")
                    .clicked()
                {
                    self.finish();
                    self.guide = DrawingPlane::from_preset(plane);
                }
            }
        });
        ui.label(
            RichText::new(self.guide.label())
                .small()
                .color(ui_style::colors(ui.ctx()).accent),
        );
        ui_style::section(ui, "Guide geometry");
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui_style::field_row(ui, &format!("Rotation {axis}"), |ui| {
                ui_style::numeric_field(
                    ui,
                    ("guide-rotation", i),
                    &mut self.guide.rotation_degrees[i],
                    Some(-180.0..=180.0),
                    1.0,
                    "°",
                )
            });
        }
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui_style::field_row(ui, &format!("Anchor {axis}"), |ui| {
                ui_style::numeric_field(
                    ui,
                    ("guide-anchor", i),
                    &mut self.guide.anchor[i],
                    Some(-20_000.0..=20_000.0),
                    1.0,
                    "wu",
                )
            });
        }
        ui_style::slider_row(
            ui,
            "Offset",
            "guide-depth",
            &mut self.guide.depth,
            -4.0..=4.0,
            1.0,
            "wu",
        )
        .on_hover_text(
            "Move the plane along its normal, relative to the anchor. Existing strokes stay fixed.",
        );
        if self.guide != before {
            self.finish();
        }
        if ui
            .button("Plane from view [Shift+F]")
            .on_hover_text(
                "Face the camera, through its focus point. Does not move existing strokes.",
            )
            .clicked()
        {
            self.plane_from_view();
        }
        if ui
            .add_enabled(
                !self.selected.is_empty(),
                egui::Button::new("Center on selection"),
            )
            .on_hover_text(
                "Move the anchor to the selection's center, retaining the plane orientation",
            )
            .clicked()
        {
            self.finish();
            if let Some((min, max)) = editing::bounds(&self.doc, &self.selected) {
                self.guide.anchor = (min + max) * 0.5;
                self.guide.depth = 0.0;
            }
        }
        if ui.button("Align view to plane [F]").clicked() {
            self.finish();
            self.camera.align_guide(self.guide);
        }
    }
    pub(super) fn plane_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let painter = ui.painter().with_clip_rect(rect);
        let active_color = ui_style::canvas_overlay_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        );
        let inactive_base = ui_style::canvas_selection_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        );
        let inactive_color = Color32::from_rgba_unmultiplied(
            inactive_base.r(),
            inactive_base.g(),
            inactive_base.b(),
            95,
        );

        // Saved planes are independent guide resources, so display every
        // visible planar one at once. Curved guides keep their existing
        // dedicated interaction and rendering paths.
        for guide in self
            .doc
            .guide_resources
            .iter()
            .filter(|guide| guide.visible)
        {
            let crate::document::guide_resources::GuideGeometry::Plane { origin, rotation } =
                &guide.geometry
            else {
                continue;
            };
            let active = self.doc.active_guide == Some(guide.id);
            let hovered =
                self.plane_picker.active && self.plane_picker.hovered_guide == Some(guide.id);
            let color = if hovered {
                Color32::from_rgb(255, 210, 90)
            } else if active {
                active_color
            } else {
                inactive_color
            };
            draw_plane_grid(
                &painter,
                &self.camera,
                rect,
                *origin,
                *rotation * Vec3::X,
                *rotation * Vec3::Y,
                PlaneGridStyle {
                    color,
                    active: active || hovered,
                    label: if hovered { "Pick" } else { &guide.name },
                },
            );
        }

        // The unsaved work plane is also visible when no saved guide is
        // active. This includes Front/Side/Ground presets, which were
        // previously hidden by the normal-axis-only overlay.
        if self.doc.active_guide.is_none()
            && self.guide_visible
            && !self.use_revolved
            && !self.use_cylinder
        {
            let (u, v, _) = self.guide.basis();
            draw_plane_grid(
                &painter,
                &self.camera,
                rect,
                self.guide.origin(),
                u,
                v,
                PlaneGridStyle {
                    color: active_color,
                    active: true,
                    label: self.guide.label(),
                },
            );
        }
    }

    pub(super) fn begin_plane_picker(&mut self) {
        if self.presentation.artwork {
            self.status = "Exit artwork-only view before picking a drawing plane.".into();
            return;
        }
        if self.placement.active || self.placement.stamp.is_some() {
            self.status = "Finish or cancel the current plane or stamp placement first.".into();
            return;
        }
        if !self.doc.guide_resources.iter().any(|guide| {
            guide.visible
                && !guide.locked
                && matches!(
                    &guide.geometry,
                    crate::document::guide_resources::GuideGeometry::Plane { .. }
                )
        }) {
            self.status = "There are no visible, unlocked saved planes to pick.".into();
            return;
        }
        self.finish();
        self.cancel_line_tool();
        self.cancel_curve_tool();
        self.cancel_shape_draft();
        self.cancel_fill_tool();
        self.plane_picker.active = true;
        self.plane_picker.hovered_guide = None;
        self.status = "Move over a visible saved plane and click to make it active. Shift+K starts picking; Escape cancels.".into();
    }

    pub(super) fn cancel_plane_picker(&mut self) {
        if self.plane_picker.active {
            self.plane_picker.active = false;
            self.plane_picker.hovered_guide = None;
            self.status = "Plane picking cancelled; the active drawing plane is unchanged.".into();
        }
    }

    pub(super) fn plane_picker_shortcuts(&mut self, ctx: &egui::Context) {
        let (pressed, modifiers) = ctx.input(|input| (input.key_pressed(Key::K), input.modifiers));
        if pressed
            && modifiers.shift
            && !modifiers.command
            && !modifiers.alt
            && !ctx.wants_keyboard_input()
        {
            if self.plane_picker.active {
                self.cancel_plane_picker();
            } else {
                self.begin_plane_picker();
            }
        }
        if self.plane_picker.active && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.cancel_plane_picker();
        }
    }

    pub(super) fn update_plane_picker_hover(&mut self, cursor: Option<Pos2>, rect: Rect) {
        if !self.plane_picker.active {
            return;
        }
        self.plane_picker.hovered_guide = cursor
            .filter(|position| rect.contains(*position))
            .and_then(|position| self.pick_saved_plane(position, rect));
    }

    fn pick_saved_plane(&self, cursor: Pos2, rect: Rect) -> Option<u64> {
        let (ray_origin, ray_direction) = gizmo::ray(&self.camera, rect, cursor);
        let mut best: Option<(f32, u64)> = None;
        for guide in self
            .doc
            .guide_resources
            .iter()
            .filter(|guide| guide.visible && !guide.locked)
        {
            let crate::document::guide_resources::GuideGeometry::Plane { origin, rotation } =
                &guide.geometry
            else {
                continue;
            };
            let plane = DrawingPlane {
                anchor: *origin,
                rotation_degrees: super::guide_resource_ui::rotation_degrees(*rotation),
                depth: 0.0,
            };
            let Some(hit) = intersect_plane(ray_origin, ray_direction, plane) else {
                continue;
            };
            if gizmo::project(&self.camera, rect, hit).is_none() {
                continue;
            }
            let distance = hit.distance(ray_origin);
            if !distance.is_finite() {
                continue;
            }
            let Some((best_distance, best_id)) = best else {
                best = Some((distance, guide.id));
                continue;
            };
            let tie_tolerance = 1.0e-5 * distance.max(best_distance).max(1.0);
            if distance < best_distance - tie_tolerance
                || ((distance - best_distance).abs() <= tie_tolerance && guide.id < best_id)
            {
                best = Some((distance, guide.id));
            }
        }
        best.map(|(_, id)| id)
    }

    pub(super) fn pick_saved_plane_at(&mut self, cursor: Pos2, rect: Rect) {
        let Some(guide_id) = self.pick_saved_plane(cursor, rect) else {
            self.status = "No visible saved plane intersects this canvas position.".into();
            self.plane_picker.hovered_guide = None;
            return;
        };
        self.finish();
        self.cancel_line_tool();
        self.cancel_curve_tool();
        self.cancel_shape_draft();
        self.cancel_fill_tool();

        let before = (
            self.doc.guide_resources.clone(),
            self.doc.active_guide,
            self.doc.image_planes.clone(),
            self.doc.active_image_plane,
            self.doc.active_model_reference,
        );
        let mut after = before.clone();
        after.1 = Some(guide_id);
        after.3 = None;
        after.4 = None;
        for image in &mut after.2 {
            image.drawable = false;
        }
        if before != after {
            let edit = Edit::Guides { before, after };
            edit.apply(&mut self.doc, true);
            self.undo.push(edit);
            self.redo.clear();
            self.dirty = true;
            self.record_recovery_edit();
            self.renderer.invalidate();
        }
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = true;
        self.shape_tool.mode = None;
        self.plane_picker.active = false;
        self.plane_picker.hovered_guide = None;
        self.status = format!(
            "{} is now the active drawing plane.",
            self.doc
                .guide_resources
                .iter()
                .find(|guide| guide.id == guide_id)
                .map_or("Saved plane", |guide| guide.name.as_str())
        );
    }
    pub(super) fn verify_planes(&mut self, rect: Rect) {
        let original = self.doc.strokes.clone();
        let selection = self.selected.clone();
        let strength = self.stabilization;
        self.select_mode = false;
        self.stabilization = 0.6;
        self.guide = DrawingPlane {
            anchor: Vec3::new(0.4, 0.3, -0.2),
            rotation_degrees: Vec3::new(32.0, -27.0, 18.0),
            depth: 0.35,
        };
        self.camera.align_guide(self.guide);
        let origin = self.guide.origin();
        let normal = self.guide.basis().2;
        for i in 0..70 {
            self.add_point(
                rect.center() + egui::vec2(i as f32 * 2.0 - 70.0, (i as f32 * 0.07).sin() * 30.0),
                0.7,
                rect,
                i == 0,
            );
        }
        self.finish();
        let custom = self.doc.strokes.last().unwrap();
        assert!(custom.points.len() > 30);
        for point in &custom.points {
            assert!((point.position - origin).dot(normal).abs() < 0.00001);
        }
        let first_plane = self.doc.strokes.clone();
        self.guide = DrawingPlane::from_preset(Plane::Ground);
        self.guide.depth = 1.5;
        assert_eq!(
            self.doc.strokes, first_plane,
            "Changing the work plane must not move existing ink"
        );
        self.camera.align_guide(self.guide);
        for i in 0..30 {
            self.add_point(
                rect.center() + egui::vec2(i as f32 * 3.0 - 45.0, 25.0),
                0.5,
                rect,
                i == 0,
            );
        }
        self.finish();
        assert!(self.camera.matrix(rect.aspect_ratio()).is_finite());
        assert!(self
            .doc
            .strokes
            .last()
            .unwrap()
            .points
            .iter()
            .all(|p| (p.position.y - 1.5).abs() < 0.00001));
        assert!(self.save(false));
        let loaded =
            Document::from_json(&std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap())
                .unwrap();
        assert_eq!(loaded.strokes, self.doc.strokes);
        self.undo();
        assert_eq!(self.doc.strokes, first_plane);
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.camera = Camera::default();
        self.camera.roll = 0.25;
        self.plane_from_view();
        let (_, up, n) = self.guide.basis();
        let (_, camera_up, camera_n) = self.camera.basis();
        assert!(up.dot(camera_up) > 0.9999 && n.dot(camera_n) > 0.9999);
        self.guide = DrawingPlane {
            anchor: Vec3::ZERO,
            rotation_degrees: Vec3::new(32.0, -22.0, 15.0),
            depth: 0.0,
        };
        self.camera = Camera::default();
        self.selected = selection;
        self.stabilization = strength;
        assert!(self.save(false));
        self.status="Custom planes: rotated drawing, vertical alignment, plane switching, and persistence checks passed.".into();
    }
}

struct PlaneGridStyle<'a> {
    color: Color32,
    active: bool,
    label: &'a str,
}

fn draw_plane_grid(
    painter: &egui::Painter,
    camera: &Camera,
    rect: Rect,
    origin: Vec3,
    u: Vec3,
    v: Vec3,
    style: PlaneGridStyle<'_>,
) {
    if !origin.is_finite() || !u.is_finite() || !v.is_finite() {
        return;
    }
    let world_per_pixel = camera.world_per_pixel(origin, rect.height());
    if !world_per_pixel.is_finite() || world_per_pixel <= 0.0 {
        return;
    }
    // Keep the guide footprint legible across zoom levels while bounding the
    // world-space coordinates sent through projection.
    let extent = (world_per_pixel * 55.0).clamp(0.25, 500.0);
    let corners = [
        origin - u * extent - v * extent,
        origin + u * extent - v * extent,
        origin + u * extent + v * extent,
        origin - u * extent + v * extent,
    ];
    let Some(projected) = corners
        .into_iter()
        .map(|point| gizmo::project(camera, rect, point).map(|(screen, _)| screen))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };

    let edge_width = if style.active { 1.8_f32 } else { 1.0_f32 };
    let edge = egui::Stroke::new(edge_width, style.color);
    for index in 0..4 {
        painter.line_segment([projected[index], projected[(index + 1) % 4]], edge);
    }

    // Three interior divisions in each direction make plane orientation
    // visible without flooding the viewport with a dense infinite grid.
    let grid_color = Color32::from_rgba_unmultiplied(
        style.color.r(),
        style.color.g(),
        style.color.b(),
        if style.active { 90 } else { 44 },
    );
    let grid = egui::Stroke::new(0.8_f32, grid_color);
    for step in 1..4 {
        let offset = -extent + extent * 0.5 * step as f32;
        let u_start = origin - u * extent + v * offset;
        let u_end = origin + u * extent + v * offset;
        let v_start = origin + u * offset - v * extent;
        let v_end = origin + u * offset + v * extent;
        if let (Some((a, _)), Some((b, _))) = (
            gizmo::project(camera, rect, u_start),
            gizmo::project(camera, rect, u_end),
        ) {
            painter.line_segment([a, b], grid);
        }
        if let (Some((a, _)), Some((b, _))) = (
            gizmo::project(camera, rect, v_start),
            gizmo::project(camera, rect, v_end),
        ) {
            painter.line_segment([a, b], grid);
        }
    }

    if let Some((screen, _)) = gizmo::project(camera, rect, origin) {
        painter.circle_filled(screen, if style.active { 4.0 } else { 2.5 }, style.color);
        painter.text(
            screen + egui::vec2(7.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            style.label,
            egui::FontId::proportional(if style.active { 12.0 } else { 10.0 }),
            style.color,
        );
    }
}
