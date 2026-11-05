use super::guide_resource_ui::PlanarTarget;
use super::ui_style;
use super::*;
use crate::curve_geometry::{CubicBezier, PressureSegment};
use crate::document::{CurveAnchor, CurveData};

#[derive(Clone, Copy, Debug)]
struct Anchor {
    position: Vec3,
    pressure: f32,
    incoming: Vec3,
    outgoing: Vec3,
    manual: bool,
}

#[derive(Clone, Copy, Debug)]
enum HandleRef {
    Anchor(usize),
    Incoming(usize),
    Outgoing(usize),
    NewAnchor(usize),
}

struct CurveDraft {
    target: CurveTarget,
    stroke: Stroke,
    anchors: Vec<Anchor>,
    preview: Option<Stroke>,
    dragging: Option<HandleRef>,
    selected_anchor: Option<usize>,
    editing_stroke_id: Option<u64>,
    viewport_height: f32,
}

#[derive(Clone, Copy)]
enum CurveTarget {
    Planar(PlanarTarget),
    Cylinder(crate::surface::Cylinder),
}

impl CurveTarget {
    fn hit(self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        match self {
            Self::Planar(target) => target.hit(origin, direction),
            Self::Cylinder(cylinder) => cylinder.hit(origin, direction),
        }
    }

    fn world_to_control(self, world: Vec3, previous: Option<Vec3>) -> Option<Vec3> {
        match self {
            Self::Planar(_) => world.is_finite().then_some(world),
            Self::Cylinder(cylinder) => cylinder.parameterize(world, previous),
        }
    }

    fn control_to_world(self, control: Vec3) -> Option<Vec3> {
        match self {
            Self::Planar(_) => control.is_finite().then_some(control),
            Self::Cylinder(cylinder) => cylinder.point_from_parameter(control),
        }
    }

    fn contains_world(self, world: Vec3) -> bool {
        match self {
            Self::Planar(target) => target.contains(world),
            Self::Cylinder(cylinder) => cylinder.parameterize(world, None).is_some(),
        }
    }

    fn planar(self) -> Option<PlanarTarget> {
        match self {
            Self::Planar(target) => Some(target),
            Self::Cylinder(_) => None,
        }
    }

    fn cylinder_surface(self) -> Option<crate::dynamics::DabSurfaceTarget> {
        let Self::Cylinder(cylinder) = self else {
            return None;
        };
        Some(crate::dynamics::DabSurfaceTarget::Cylinder {
            center: cylinder.center,
            rotation: cylinder.rotation(),
            radius: cylinder.radius,
            height: cylinder.height,
        })
    }

    fn frame(self, control: Vec3) -> Option<(Vec3, Vec3, Vec3)> {
        match self {
            Self::Planar(target) => {
                let (tangent, _, normal) = target.plane.basis();
                Some((target.plane.origin(), normal, tangent))
            }
            Self::Cylinder(cylinder) => {
                let angle = control.x / cylinder.radius;
                let rotation = cylinder.rotation();
                Some((
                    cylinder.center,
                    rotation * Vec3::new(angle.sin(), 0.0, angle.cos()),
                    rotation * Vec3::new(angle.cos(), 0.0, -angle.sin()),
                ))
            }
        }
    }
}

pub(super) struct CurveUi {
    pub enabled: bool,
    pub tension: f32,
    pub aligned_handles: bool,
    draft: Option<CurveDraft>,
    error: Option<String>,
}

impl Default for CurveUi {
    fn default() -> Self {
        Self {
            enabled: false,
            tension: 1.0,
            aligned_handles: true,
            draft: None,
            error: None,
        }
    }
}

impl SketchApp {
    pub(super) fn curve_draft_active(&self) -> bool {
        self.curve_tool.draft.is_some()
    }

    pub(super) fn curve_draft_anchor_count(&self) -> usize {
        self.curve_tool
            .draft
            .as_ref()
            .map_or(0, |draft| draft.anchors.len())
    }

    pub(super) fn curve_draft_anchor_is_manual(&self, index: usize) -> bool {
        self.curve_tool
            .draft
            .as_ref()
            .and_then(|draft| draft.anchors.get(index))
            .is_some_and(|anchor| anchor.manual)
    }

    pub(super) fn curve_draft_preview(&self) -> Option<&Stroke> {
        self.curve_tool
            .draft
            .as_ref()
            .and_then(|draft| draft.preview.as_ref())
    }

    pub(super) fn start_curve_tool(&mut self) {
        if self.curve_target().is_none() && !self.has_selected_editable_curve() {
            self.status =
                "Smooth curves require a plane or cylinder guide. Activate a supported guide first.".into();
            return;
        }
        self.finish();
        self.cancel_line_tool();
        self.cancel_fill_tool();
        self.cancel_shape_draft();
        self.clear_planar_snap();
        self.curve_tool.draft = None;
        self.curve_tool.enabled = true;
        self.select_mode = false;
        self.eraser = false;
        self.status =
            "Smooth curve tool ready. Click to add anchors; drag an anchor to set its tangent."
                .into();
    }

    fn curve_target(&self) -> Option<CurveTarget> {
        if self.doc.active_image_plane.is_some() {
            return self.planar_drawing_target().map(CurveTarget::Planar);
        }
        if let Some(geometry) = self.saved_guide_geometry() {
            return match geometry {
                crate::document::guide_resources::GuideGeometry::Cylinder {
                    center,
                    rotation,
                    radius,
                    height,
                } => {
                    let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
                    Some(CurveTarget::Cylinder(crate::surface::Cylinder {
                        center: *center,
                        rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
                        radius: *radius,
                        height: *height,
                    }))
                }
                crate::document::guide_resources::GuideGeometry::Plane { .. } => {
                    self.planar_drawing_target().map(CurveTarget::Planar)
                }
                _ => None,
            };
        }
        if self.use_cylinder {
            Some(CurveTarget::Cylinder(self.cylinder))
        } else {
            self.planar_drawing_target().map(CurveTarget::Planar)
        }
    }

    fn curve_target_from_snapshot(
        target: crate::dynamics::DabSurfaceTarget,
    ) -> Option<CurveTarget> {
        let crate::dynamics::DabSurfaceTarget::Cylinder {
            center,
            rotation,
            radius,
            height,
        } = target
        else {
            return None;
        };
        let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
        Some(CurveTarget::Cylinder(crate::surface::Cylinder {
            center,
            rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
            radius,
            height,
        }))
    }

    pub(super) fn cancel_curve(&mut self) {
        if self.curve_tool.draft.take().is_some() {
            self.status = "Curve preview cancelled.".into();
        }
        self.clear_planar_snap();
    }

    pub(super) fn cancel_curve_tool(&mut self) {
        self.cancel_curve();
        self.curve_tool.enabled = false;
    }

    pub(super) fn curve_controls(&mut self, ui: &mut egui::Ui) {
        if !self.curve_tool.enabled {
            return;
        }
        ui.strong("SMOOTH CURVE");
        ui.label("Click to append anchors or click near a segment to insert one. Curves capture their plane or unwrapped cylinder target.");
        let can_edit_selected = self.has_selected_editable_curve();
        if ui
            .add_enabled(can_edit_selected, egui::Button::new("Edit selected curve"))
            .clicked()
        {
            self.open_selected_curve();
        }
        let tension_changed = ui_style::slider_row(
            ui,
            "Auto tension",
            "curve-tension",
            &mut self.curve_tool.tension,
            0.0..=2.0,
            100.0,
            "",
        );
        if tension_changed.changed() {
            self.curve_refresh(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(
                    1.0,
                    self.curve_tool
                        .draft
                        .as_ref()
                        .map_or(900.0, |draft| draft.viewport_height),
                ),
            ));
        }
        ui.checkbox(
            &mut self.curve_tool.aligned_handles,
            "Keep tangent handles aligned",
        );
        let count = self
            .curve_tool
            .draft
            .as_ref()
            .map_or(0, |draft| draft.anchors.len());
        ui.label(format!(
            "{count} control points · {}",
            if count < 2 {
                "add at least two"
            } else {
                "ready to commit"
            }
        ));
        if let Some(error) = &self.curve_tool.error {
            ui.colored_label(ui_style::colors(ui.ctx()).error, error);
        }
        ui.horizontal(|ui| {
            let selected_anchor = self
                .curve_tool
                .draft
                .as_ref()
                .and_then(|draft| draft.selected_anchor);
            if ui
                .add_enabled(
                    selected_anchor.is_some(),
                    egui::Button::new("Remove selected point"),
                )
                .clicked()
            {
                if let Some(index) = selected_anchor {
                    self.curve_remove_anchor(index);
                }
            }
            if ui
                .add_enabled(count > 0, egui::Button::new("Remove last point"))
                .clicked()
            {
                self.curve_remove_last();
            }
            if ui
                .add_enabled(
                    count >= 2,
                    egui::Button::new(
                        if self
                            .curve_tool
                            .draft
                            .as_ref()
                            .is_some_and(|draft| draft.editing_stroke_id.is_some())
                        {
                            "Apply edit"
                        } else {
                            "Finish curve"
                        },
                    ),
                )
                .clicked()
            {
                self.commit_curve();
            }
            if count > 0 && ui.button("Cancel").clicked() {
                self.cancel_curve();
            }
        });
        ui.small("Enter applies; Backspace removes the selected point or last point; Escape cancels. Apply is one undo step.");
    }

    pub(super) fn curve_input(&mut self, pos: Pos2, pressure: f32, rect: Rect, begin: bool) {
        if !rect.contains(pos) {
            return;
        }
        if begin {
            if self.curve_tool.draft.is_none() {
                let Some(target) = self.curve_target() else {
                    self.status =
                        "Smooth curves require a plane or cylinder guide. Activate a supported guide first."
                            .into();
                    return;
                };
                if !self.doc.active_editable() {
                    self.status = "Choose a visible unlocked layer for the curve.".into();
                    return;
                }
                let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let Some(world) = target.hit(origin, direction) else {
                    return;
                };
                let Some(control) = target.world_to_control(world, None) else {
                    return;
                };
                let Some((_, normal, tangent)) = target.frame(control) else {
                    return;
                };
                let stroke = Stroke {
                    guide_attachment: None,
                    model_surface: None,
                    curve: None,
                    material: Default::default(),
                    point_thickness: None,
                    point_colors: None,
                    modifiers: None,
                    fill: None,
                    group_id: None,
                    layer_id: self.doc.active_layer,
                    id: 0,
                    color: self.color,
                    gradient: None,
                    radius: self.radius,
                    brush: Brush {
                        normal,
                        tangent,
                        dabs: crate::dynamics::DabSettings {
                            enabled: false,
                            ..self.brush.dabs
                        },
                        ..self.brush
                    },
                    points: Vec::new(),
                };
                self.curve_tool.draft = Some(CurveDraft {
                    target,
                    stroke,
                    anchors: Vec::new(),
                    preview: None,
                    dragging: None,
                    selected_anchor: None,
                    editing_stroke_id: None,
                    viewport_height: rect.height().max(1.0),
                });
            }
            if self.snapping.options.enabled {
                let (target, excluded) = self
                    .curve_tool
                    .draft
                    .as_ref()
                    .map(|draft| {
                        let excluded = draft.editing_stroke_id.into_iter().collect::<HashSet<_>>();
                        (draft.target, excluded)
                    })
                    .expect("curve draft was initialized");
                if let Some(planar) = target.planar() {
                    if self.planar_snap_index_stale(planar, rect, &excluded) {
                        self.prepare_planar_snap(planar, rect, &excluded);
                    }
                }
            }
            let Some(mut world) = self.curve_world(pos, rect) else {
                return;
            };
            let target = self.curve_tool.draft.as_ref().unwrap().target;
            if let Some(planar) = target.planar() {
                world = self.snap_planar_point(world, pos, planar, rect, None);
            }
            let handle = self
                .curve_tool
                .draft
                .as_ref()
                .and_then(|draft| self.curve_handle_at(draft, pos, rect));
            if let Some(handle) = handle {
                if let Some(draft) = &mut self.curve_tool.draft {
                    draft.dragging = Some(handle);
                    draft.selected_anchor = Some(match handle {
                        HandleRef::Anchor(i)
                        | HandleRef::Incoming(i)
                        | HandleRef::Outgoing(i)
                        | HandleRef::NewAnchor(i) => i,
                    });
                }
            } else {
                let target = self
                    .curve_tool
                    .draft
                    .as_ref()
                    .expect("curve draft remains active")
                    .target;
                let insertion_pos = crate::gizmo::project(&self.camera, rect, world)
                    .map_or(pos, |(screen, _)| screen);
                let insertion = self.curve_tool.draft.as_ref().and_then(|draft| {
                    if draft.anchors.len() < 2 {
                        return None;
                    }
                    let mut nearest: (f32, usize) = (14.0, draft.anchors.len());
                    for pair_index in 0..draft.anchors.len() - 1 {
                        let Some(world_a) = draft
                            .target
                            .control_to_world(draft.anchors[pair_index].position)
                        else {
                            continue;
                        };
                        let Some(world_b) = draft
                            .target
                            .control_to_world(draft.anchors[pair_index + 1].position)
                        else {
                            continue;
                        };
                        let Some((a, _)) = crate::gizmo::project(&self.camera, rect, world_a)
                        else {
                            continue;
                        };
                        let Some((b, _)) = crate::gizmo::project(&self.camera, rect, world_b)
                        else {
                            continue;
                        };
                        let distance = point_segment_distance(insertion_pos, a, b);
                        if distance < nearest.0 {
                            nearest = (distance, pair_index + 1);
                        }
                    }
                    (nearest.1 < draft.anchors.len()).then_some(nearest.1)
                });
                let count = self
                    .curve_tool
                    .draft
                    .as_ref()
                    .map_or(0, |draft| draft.anchors.len());
                if count >= 256 {
                    self.curve_tool.error =
                        Some("Curves are limited to 256 control points.".into());
                    return;
                }
                let previous = self.curve_tool.draft.as_ref().and_then(|draft| {
                    insertion
                        .unwrap_or(draft.anchors.len())
                        .checked_sub(1)
                        .and_then(|index| draft.anchors.get(index))
                        .map(|anchor| anchor.position)
                });
                let Some(control) = target.world_to_control(world, previous) else {
                    self.curve_tool.error =
                        Some("Point is outside the captured cylinder surface.".into());
                    return;
                };
                if let Some(draft) = &mut self.curve_tool.draft {
                    let idx = insertion.unwrap_or(draft.anchors.len());
                    draft.anchors.insert(
                        idx,
                        Anchor {
                            position: control,
                            pressure: pressure.clamp(0.0, 1.0),
                            incoming: Vec3::ZERO,
                            outgoing: Vec3::ZERO,
                            manual: false,
                        },
                    );
                    draft.dragging = Some(HandleRef::NewAnchor(idx));
                    draft.selected_anchor = Some(idx);
                }
                self.curve_refresh(rect);
            }
        } else {
            let Some(handle) = self
                .curve_tool
                .draft
                .as_ref()
                .and_then(|draft| draft.dragging)
            else {
                return;
            };
            let Some(world) = self.curve_world(pos, rect) else {
                return;
            };
            let anchor_index = match handle {
                HandleRef::Anchor(index)
                | HandleRef::Incoming(index)
                | HandleRef::Outgoing(index)
                | HandleRef::NewAnchor(index) => index,
            };
            let (target, previous) = {
                let draft = self.curve_tool.draft.as_ref().unwrap();
                (draft.target, draft.anchors[anchor_index].position)
            };
            let mut world = world;
            if matches!(handle, HandleRef::Anchor(_)) {
                if let Some(planar) = target.planar() {
                    world = self.snap_planar_point(world, pos, planar, rect, None);
                }
            }
            let Some(control) = target.world_to_control(world, Some(previous)) else {
                return;
            };
            self.curve_move_handle(handle, control);
            self.curve_refresh(rect);
        }
    }

    pub(super) fn curve_pointer_up(&mut self) {
        if let Some(draft) = &mut self.curve_tool.draft {
            draft.dragging = None;
        }
        self.clear_planar_snap_cue();
    }

    fn curve_world(&self, pos: Pos2, rect: Rect) -> Option<Vec3> {
        let target = self
            .curve_tool
            .draft
            .as_ref()
            .map(|draft| draft.target)
            .or_else(|| self.curve_target())?;
        let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
        target.hit(origin, direction)
    }

    fn curve_handle_at(&self, draft: &CurveDraft, pos: Pos2, rect: Rect) -> Option<HandleRef> {
        let mut nearest_anchor = (10.0_f32, None);
        let mut nearest_handle = (12.0_f32, None);
        for (index, anchor) in draft.anchors.iter().enumerate() {
            let Some(anchor_world) = draft.target.control_to_world(anchor.position) else {
                continue;
            };
            if let Some((screen, _)) = crate::gizmo::project(&self.camera, rect, anchor_world) {
                let distance = screen.distance(pos);
                if distance < nearest_anchor.0 {
                    nearest_anchor = (distance, Some(HandleRef::Anchor(index)));
                }
            }
            for (endpoint_control, handle) in [
                (
                    anchor.position + anchor.incoming,
                    HandleRef::Incoming(index),
                ),
                (
                    anchor.position + anchor.outgoing,
                    HandleRef::Outgoing(index),
                ),
            ] {
                let Some(endpoint) = draft.target.control_to_world(endpoint_control) else {
                    continue;
                };
                if let Some((screen, _)) = crate::gizmo::project(&self.camera, rect, endpoint) {
                    let distance = screen.distance(pos);
                    if distance < nearest_handle.0 {
                        nearest_handle = (distance, Some(handle));
                    }
                }
            }
        }
        nearest_anchor.1.or(nearest_handle.1)
    }

    fn curve_move_handle(&mut self, handle: HandleRef, world: Vec3) {
        let Some(draft) = &mut self.curve_tool.draft else {
            return;
        };
        let index = match handle {
            HandleRef::Anchor(i)
            | HandleRef::Incoming(i)
            | HandleRef::Outgoing(i)
            | HandleRef::NewAnchor(i) => i,
        };
        let Some(anchor) = draft.anchors.get_mut(index) else {
            return;
        };
        let offset = world - anchor.position;
        match handle {
            HandleRef::Anchor(_) => anchor.position = world,
            HandleRef::NewAnchor(_) => {
                let result = crate::curve_geometry::move_handle(
                    anchor.incoming,
                    anchor.outgoing,
                    false,
                    offset,
                    self.curve_tool.aligned_handles,
                );
                if let Ok((incoming, outgoing)) = result {
                    anchor.incoming = incoming;
                    anchor.outgoing = outgoing;
                    anchor.manual = true;
                }
            }
            HandleRef::Incoming(_) => {
                if let Ok((incoming, outgoing)) = crate::curve_geometry::move_handle(
                    anchor.incoming,
                    anchor.outgoing,
                    true,
                    offset,
                    self.curve_tool.aligned_handles,
                ) {
                    anchor.incoming = incoming;
                    anchor.outgoing = outgoing;
                    anchor.manual = true;
                }
            }
            HandleRef::Outgoing(_) => {
                if let Ok((incoming, outgoing)) = crate::curve_geometry::move_handle(
                    anchor.incoming,
                    anchor.outgoing,
                    false,
                    offset,
                    self.curve_tool.aligned_handles,
                ) {
                    anchor.incoming = incoming;
                    anchor.outgoing = outgoing;
                    anchor.manual = true;
                }
            }
        }
    }

    fn curve_refresh(&mut self, rect: Rect) {
        let Some(draft) = &mut self.curve_tool.draft else {
            return;
        };
        if rect.height() > 0.0 {
            draft.viewport_height = rect.height();
        }
        for index in 0..draft.anchors.len() {
            if draft.anchors[index].manual {
                continue;
            }
            let previous = index.checked_sub(1).map(|i| draft.anchors[i].position);
            let next = draft.anchors.get(index + 1).map(|a| a.position);
            if let Ok((incoming, outgoing)) = crate::curve_geometry::smooth_handle_offsets(
                previous,
                draft.anchors[index].position,
                next,
                self.curve_tool.tension,
            ) {
                draft.anchors[index].incoming = incoming;
                draft.anchors[index].outgoing = outgoing;
            }
        }
        if draft.anchors.len() < 2 {
            draft.preview = None;
            return;
        }
        let segments: Vec<_> = draft
            .anchors
            .windows(2)
            .map(|pair| PressureSegment {
                curve: CubicBezier {
                    p0: pair[0].position,
                    p1: pair[0].position + pair[0].outgoing,
                    p2: pair[1].position + pair[1].incoming,
                    p3: pair[1].position,
                },
                pressure_start: pair[0].pressure,
                pressure_end: pair[1].pressure,
            })
            .collect();
        let Some(first_world) = draft.target.control_to_world(draft.anchors[0].position) else {
            draft.preview = None;
            self.curve_tool.error = Some("Curve anchor is outside the captured surface.".into());
            return;
        };
        let scale = self
            .camera
            .world_per_pixel(first_world, draft.viewport_height.max(1.0));
        let tolerance = (scale * 0.75).clamp(
            crate::curve_geometry::MIN_TOLERANCE_WORLD,
            crate::curve_geometry::MAX_TOLERANCE_WORLD,
        );
        let tessellated = match draft.target {
            CurveTarget::Planar(_) => crate::curve_geometry::tessellate_stroke(
                &segments,
                &draft.stroke,
                tolerance,
                crate::curve_geometry::MAX_CURVE_POINTS,
            ),
            CurveTarget::Cylinder(cylinder) => crate::curve_geometry::tessellate_cylinder_stroke(
                &segments,
                &draft.stroke,
                cylinder,
                tolerance,
                crate::curve_geometry::MAX_CURVE_POINTS,
            ),
        };
        match tessellated {
            Ok(stroke)
                if stroke
                    .points
                    .iter()
                    .all(|point| draft.target.contains_world(point.position)) =>
            {
                let (frame_origin, normal, tangent) =
                    draft.target.frame(draft.anchors[0].position).unwrap();
                let curve = CurveData {
                    anchors: draft
                        .anchors
                        .iter()
                        .map(|anchor| CurveAnchor {
                            position: anchor.position,
                            pressure: anchor.pressure,
                            incoming: anchor.incoming,
                            outgoing: anchor.outgoing,
                            manual: anchor.manual,
                        })
                        .collect(),
                    frame_origin,
                    frame_normal: normal,
                    frame_tangent: tangent,
                    half_extent: draft.target.planar().and_then(PlanarTarget::half_extent),
                    tension: self.curve_tool.tension,
                    surface_target: draft.target.cylinder_surface(),
                    shape: None,
                };
                if curve.is_valid() {
                    let mut stroke = stroke;
                    stroke.curve = Some(curve);
                    draft.preview = Some(stroke);
                    self.curve_tool.error = None;
                } else {
                    draft.preview = None;
                    self.curve_tool.error =
                        Some("Curve control data is outside supported bounds.".into());
                }
            }
            Ok(_) => {
                draft.preview = None;
                self.curve_tool.error = Some(match draft.target {
                    CurveTarget::Planar(_) => {
                        "Curve extends beyond the active image bounds.".into()
                    }
                    CurveTarget::Cylinder(_) => {
                        "Curve extends beyond the captured cylinder bounds.".into()
                    }
                });
            }
            Err(error) => {
                draft.preview = None;
                self.curve_tool.error = Some(error);
            }
        }
    }

    pub(super) fn curve_remove_last(&mut self) {
        if let Some(draft) = &mut self.curve_tool.draft {
            draft.anchors.pop();
            draft.dragging = None;
            draft.selected_anchor = draft.anchors.len().checked_sub(1);
        }
        self.curve_tool.error = None;
        let height = self
            .curve_tool
            .draft
            .as_ref()
            .map_or(900.0, |draft| draft.viewport_height);
        self.curve_refresh(Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, height)));
    }

    fn curve_remove_anchor(&mut self, index: usize) {
        let Some(draft) = &mut self.curve_tool.draft else {
            return;
        };
        if index >= draft.anchors.len() {
            return;
        }
        draft.anchors.remove(index);
        draft.dragging = None;
        draft.selected_anchor = if draft.anchors.is_empty() {
            None
        } else {
            Some(index.min(draft.anchors.len() - 1))
        };
        self.curve_tool.error = None;
        let height = draft.viewport_height;
        self.curve_refresh(Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, height)));
    }

    fn has_selected_editable_curve(&self) -> bool {
        if self.selected.len() != 1 {
            return false;
        }
        self.doc.strokes.iter().any(|stroke| {
            self.selected.contains(&stroke.id)
                && stroke
                    .curve
                    .as_ref()
                    .is_some_and(|curve| curve.shape.is_none() && curve.is_valid())
                && self.doc.editable(stroke)
        })
    }

    pub(super) fn open_selected_curve(&mut self) {
        if self.curve_tool.draft.is_some() {
            self.status = "Apply or cancel the current curve preview first.".into();
            return;
        }
        self.clear_planar_snap();
        let Some(stroke) = self
            .doc
            .strokes
            .iter()
            .find(|stroke| self.selected.contains(&stroke.id) && self.doc.editable(stroke))
        else {
            self.status = "Select one visible, unlocked curve to edit.".into();
            return;
        };
        let Some(curve) = stroke.curve.as_ref().filter(|curve| curve.is_valid()) else {
            self.status = "The selected stroke has no editable curve anchors.".into();
            return;
        };
        let target = if let Some(surface) = curve.surface_target {
            Self::curve_target_from_snapshot(surface)
        } else {
            PlanarTarget::from_curve_frame(
                curve.frame_origin,
                curve.frame_normal,
                curve.frame_tangent,
                curve.half_extent,
            )
            .map(CurveTarget::Planar)
        };
        let Some(target) = target else {
            self.status = "The selected curve has an invalid captured drawing frame.".into();
            return;
        };
        self.curve_tool.tension = curve.tension;
        self.curve_tool.draft = Some(CurveDraft {
            target,
            stroke: stroke.clone(),
            anchors: curve
                .anchors
                .iter()
                .map(|anchor| Anchor {
                    position: anchor.position,
                    pressure: anchor.pressure,
                    incoming: anchor.incoming,
                    outgoing: anchor.outgoing,
                    manual: anchor.manual,
                })
                .collect(),
            preview: Some(stroke.clone()),
            dragging: None,
            selected_anchor: None,
            editing_stroke_id: Some(stroke.id),
            viewport_height: 900.0,
        });
        self.curve_refresh(Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 900.0)));
        self.status = "Curve edit preview ready. Apply to commit once or Escape to cancel.".into();
    }

    pub(super) fn verify_curve_edit_persistence(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
    ) -> Vec<&'static str> {
        let source = self
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.curve.as_ref().is_some_and(CurveData::is_valid))
            .cloned()
            .expect("curve verifier has a committed semantic curve");
        let source_curve = source.curve.as_ref().unwrap().clone();

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/sketch-079");
        std::fs::create_dir_all(&output).unwrap();
        let native_path = output.join("curve-edit.sketch.json");
        self.file = Some(native_path.clone());
        assert!(self.save(false), "semantic curve native save succeeds");
        let reloaded = Document::from_json(&std::fs::read_to_string(&native_path).unwrap())
            .expect("native curve reload validates");
        let reloaded_source = reloaded
            .strokes
            .iter()
            .find(|stroke| stroke.id == source.id)
            .expect("native reload retains the source curve");
        assert_eq!(reloaded_source.curve.as_ref(), Some(&source_curve));
        assert_eq!(reloaded_source.points, source.points);
        self.doc = reloaded;
        self.renderer.invalidate();
        self.undo.clear();
        self.redo.clear();
        self.selected = HashSet::from([source.id]);
        let saved_document = self.doc.clone();

        let sampled_stroke = self
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.curve.is_none())
            .expect("older sampled strokes remain available without semantic controls");
        self.selected = HashSet::from([sampled_stroke.id]);
        self.start_curve_tool();
        self.open_selected_curve();
        assert!(!self.curve_draft_active());
        assert!(self.status.contains("no editable curve anchors"));
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&saved_document).unwrap(),
            "opening an older sampled stroke does not fabricate curve handles"
        );

        self.selected = HashSet::from([source.id]);
        self.start_curve_tool();
        self.open_selected_curve();
        let draft = self
            .curve_tool
            .draft
            .as_ref()
            .expect("selected curve reopens for editing");
        assert_eq!(draft.editing_stroke_id, Some(source.id));
        assert_eq!(draft.anchors.len(), source_curve.anchors.len());
        assert_eq!(self.curve_tool.tension, source_curve.tension);
        for (draft_anchor, saved_anchor) in draft.anchors.iter().zip(&source_curve.anchors) {
            assert_eq!(draft_anchor.position, saved_anchor.position);
            assert_eq!(draft_anchor.pressure, saved_anchor.pressure);
            assert_eq!(draft_anchor.incoming, saved_anchor.incoming);
            assert_eq!(draft_anchor.outgoing, saved_anchor.outgoing);
            assert_eq!(draft_anchor.manual, saved_anchor.manual);
        }
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&saved_document).unwrap(),
            "opening the edit is non-mutating"
        );

        let drag_anchor = |app: &mut SketchApp, original: &CurveData| {
            let anchor = original.anchors[1].position;
            let destination = anchor + original.frame_tangent * 0.08;
            let start = crate::gizmo::project(&app.camera, rect, anchor)
                .expect("curve anchor projects to the canvas")
                .0;
            let end = crate::gizmo::project(&app.camera, rect, destination)
                .expect("edited anchor remains inside the canvas")
                .0;
            app.curve_input(start, original.anchors[1].pressure, rect, true);
            app.curve_input(end, original.anchors[1].pressure, rect, false);
            app.curve_pointer_up();
            destination
        };

        let expected_anchor = drag_anchor(self, &source_curve);
        let escaped = self.curve_draft_preview().unwrap();
        assert_ne!(escaped.curve.as_ref().unwrap(), &source_curve);
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&saved_document).unwrap(),
            "edit preview leaves the drawing intact"
        );
        assert_eq!(self.undo.len(), 0, "preview adds no history entry");
        ctx.input_mut(|input| {
            input.events = vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        });
        self.curve_shortcuts(ctx);
        ctx.input_mut(|input| input.events.clear());
        assert!(!self.curve_draft_active(), "Escape cancels an edit preview");
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&saved_document).unwrap()
        );
        assert_eq!(self.undo.len(), 0);

        self.open_selected_curve();
        let _ = drag_anchor(self, &source_curve);
        assert!(self.curve_draft_active());
        let old_frame = self.image_plane_verification_frame;
        self.image_plane_verification_frame = None;
        let was_focused = ctx.input(|input| input.focused);
        ctx.input_mut(|input| input.focused = false);
        self.canvas_input(ctx, rect, Vec::new());
        ctx.input_mut(|input| input.focused = was_focused);
        self.image_plane_verification_frame = old_frame;
        assert!(
            !self.curve_draft_active(),
            "focus loss cancels the edit preview"
        );
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&saved_document).unwrap()
        );
        assert_eq!(self.undo.len(), 0);

        self.open_selected_curve();
        let _ = drag_anchor(self, &source_curve);
        let preview = self.curve_draft_preview().unwrap().clone();
        assert!(
            preview.curve.as_ref().unwrap().anchors[1]
                .position
                .distance(expected_anchor)
                < 1.0e-4,
            "the re-opened preview uses the exact expected world anchor"
        );
        self.commit_curve();
        let edited = self
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.id == source.id)
            .unwrap()
            .clone();
        assert_ne!(edited, source);
        assert_eq!(self.undo.len(), 1, "Apply commits one edit");
        assert_eq!(self.redo.len(), 0);
        assert_eq!(edited.brush, source.brush);
        assert_eq!(edited.radius, source.radius);
        assert_eq!(edited.color, source.color);
        assert_eq!(
            edited.curve.as_ref().unwrap().anchors.len(),
            source_curve.anchors.len()
        );
        self.undo();
        assert_eq!(
            serde_json::to_value(&self.doc).unwrap(),
            serde_json::to_value(&saved_document).unwrap(),
            "undo restores the exact original curve"
        );
        self.redo();
        assert_eq!(
            self.doc
                .strokes
                .iter()
                .find(|stroke| stroke.id == source.id),
            Some(&edited),
            "redo restores the applied edit exactly"
        );

        assert!(self.save(false), "edited semantic curve saves natively");
        let edited_reload = Document::from_json(&std::fs::read_to_string(&native_path).unwrap())
            .expect("edited curve native reload validates");
        let edited_reloaded_stroke = edited_reload
            .strokes
            .iter()
            .find(|stroke| stroke.id == source.id)
            .unwrap();
        assert_eq!(edited_reloaded_stroke, &edited);

        let only_curve = Document {
            strokes: vec![edited.clone()],
            ..Document::default()
        };
        let source_for_merge = output.join("curve-merge-source.sketch.json");
        std::fs::write(&source_for_merge, serde_json::to_vec(&only_curve).unwrap()).unwrap();
        let merge_assets = output.join("merge-assets");
        std::fs::create_dir_all(&merge_assets).unwrap();
        let merged = super::native_merge::prepare_insert(
            &Document::default(),
            &source_for_merge,
            &merge_assets,
            100,
            10,
            200,
            Vec3::ZERO,
            false,
            &HashMap::new(),
        )
        .expect("semantic curve native insert succeeds");
        assert_eq!(merged.edit.strokes.len(), 1);
        assert_eq!(merged.edit.strokes[0].curve.as_ref(), edited.curve.as_ref());
        let mut recovered_doc = Document {
            strokes: vec![edited.clone()],
            ..Document::default()
        };
        recovered_doc.refresh_linked_instances().unwrap();
        let recovery_root = output.join("recovery");
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "curve-edit-verification".into(),
            None,
            recovered_doc,
        );
        crate::recovery_store::write(&recovery_root, &snapshot).unwrap();
        let recovered = crate::recovery_store::scan(&recovery_root, "curve-edit-reader").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes[0].curve, edited.curve,
            "recovery retains curve anchors and frame"
        );

        let before = crate::editing::snapshot(&self.doc, &self.selected);
        let rotation = glam::Quat::from_rotation_y(0.25);
        let transformed =
            crate::editing::transformed(&before, Vec3::ZERO, Vec3::ZERO, rotation, 1.2)
                .expect("valid curve transforms keep semantic data");
        let transformed_curve = transformed[0].1.curve.as_ref().unwrap();
        let edited_curve = edited.curve.as_ref().unwrap();
        assert_eq!(transformed_curve.anchors.len(), edited_curve.anchors.len());
        assert!(
            (transformed_curve.frame_normal - rotation * edited_curve.frame_normal).length()
                < 1.0e-5
        );
        assert!(
            (transformed_curve.frame_tangent - rotation * edited_curve.frame_tangent).length()
                < 1.0e-5
        );
        for (before_anchor, after_anchor) in
            edited_curve.anchors.iter().zip(&transformed_curve.anchors)
        {
            let expected = rotation * before_anchor.position * 1.2;
            assert!((expected - after_anchor.position).length() < 1.0e-5);
        }
        self.cancel_curve_tool();

        vec![
            "older sampled strokes do not receive fabricated semantic curve controls",
            "curve anchors and tangent controls survive native save/reopen exactly",
            "curve edit preview preserves the source and applies as one undo/redo edit",
            "Escape and focus loss cancel committed-curve editing without document changes",
            "curve edits survive native merge and fresh recovery scan",
            "selection transforms preserve semantic curve anchors and captured frame",
        ]
    }

    pub(super) fn commit_curve(&mut self) {
        if self
            .curve_tool
            .draft
            .as_ref()
            .is_some_and(|draft| draft.preview.is_none())
        {
            self.status = self
                .curve_tool
                .error
                .clone()
                .unwrap_or_else(|| "Add at least two curve points before finishing.".into());
            return;
        }
        let Some(draft) = self.curve_tool.draft.take() else {
            return;
        };
        let Some(mut stroke) = draft.preview else {
            self.status = "Add at least two curve points before finishing.".into();
            self.curve_tool.draft = Some(draft);
            return;
        };
        if let Some(id) = draft.editing_stroke_id {
            let Some(index) = self.doc.strokes.iter().position(|item| item.id == id) else {
                self.curve_tool.draft = Some(CurveDraft {
                    preview: Some(stroke),
                    ..draft
                });
                self.status =
                    "The curve changed while its preview was open. Reopen it to edit.".into();
                return;
            };
            let before_stroke = self.doc.strokes[index].clone();
            let total_points = self
                .doc
                .strokes
                .iter()
                .map(|item| item.points.len())
                .sum::<usize>();
            if !self.doc.editable(&before_stroke)
                || !stroke.curve.as_ref().is_some_and(CurveData::is_valid)
                || total_points.saturating_sub(before_stroke.points.len()) + stroke.points.len()
                    > 2_000_000
            {
                self.curve_tool.draft = Some(CurveDraft {
                    preview: Some(stroke),
                    ..draft
                });
                self.status = "Curve edit exceeds the layer or document limits.".into();
                return;
            }
            stroke.id = id;
            if stroke == before_stroke {
                self.status = "Curve is unchanged.".into();
                return;
            }
            let edit = Edit::Change {
                before: vec![(index, before_stroke)],
                after: vec![(index, stroke)],
            };
            edit.apply(&mut self.doc, true);
            self.undo.push(edit);
            self.redo.clear();
            self.dirty = true;
            self.record_recovery_edit_labeled("Edit curve");
            self.renderer.invalidate();
            self.curve_tool.error = None;
            self.clear_planar_snap();
            self.status = "Curve edit applied as one undo step.".into();
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
                + stroke.points.len()
                > 2_000_000
            || !self.doc.active_editable()
        {
            self.status =
                "Curve could not be committed: protected layer or document limits.".into();
            self.curve_tool.draft = Some(CurveDraft {
                preview: Some(stroke),
                ..draft
            });
            return;
        }
        stroke.id = self.next_id;
        self.next_id += 1;
        self.commit_drawn_stroke(stroke, false, 0);
        self.clear_planar_snap();
        self.curve_tool.error = None;
        self.status = "Smooth curve committed as one editable stroke.".into();
    }

    pub(super) fn curve_shortcuts(&mut self, ctx: &egui::Context) {
        let (enter, backspace, escape, c, mods) = ctx.input(|i| {
            (
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::C),
                i.modifiers,
            )
        });
        if c && !mods.command && !mods.alt {
            self.start_curve_tool();
        }
        if self.curve_tool.draft.is_some() {
            if escape {
                self.cancel_curve();
                self.mouse_down = false;
            } else if backspace {
                if let Some(index) = self
                    .curve_tool
                    .draft
                    .as_ref()
                    .and_then(|draft| draft.selected_anchor)
                {
                    self.curve_remove_anchor(index);
                } else {
                    self.curve_remove_last();
                }
            } else if enter {
                self.commit_curve();
            }
        }
    }

    pub(super) fn curve_overlay(&self, ui: &egui::Ui, rect: Rect) {
        let Some(draft) = &self.curve_tool.draft else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let color = ui_style::canvas_overlay_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        );
        if let Some(stroke) = &draft.preview {
            let points: Vec<_> = stroke
                .points
                .iter()
                .filter_map(|point| {
                    crate::gizmo::project(&self.camera, rect, point.position).map(|(pos, _)| pos)
                })
                .collect();
            for pair in points.windows(2) {
                painter.line_segment([pair[0], pair[1]], egui::Stroke::new(2.0_f32, color));
            }
        }
        for anchor in &draft.anchors {
            let Some(anchor_world) = draft.target.control_to_world(anchor.position) else {
                continue;
            };
            if let Some((point, _)) = crate::gizmo::project(&self.camera, rect, anchor_world) {
                painter.circle_filled(point, 4.5, color);
            }
            for endpoint_control in [
                anchor.position + anchor.incoming,
                anchor.position + anchor.outgoing,
            ] {
                let Some(endpoint) = draft.target.control_to_world(endpoint_control) else {
                    continue;
                };
                if let (Some((a, _)), Some((b, _))) = (
                    crate::gizmo::project(&self.camera, rect, anchor_world),
                    crate::gizmo::project(&self.camera, rect, endpoint),
                ) {
                    painter.line_segment(
                        [a, b],
                        egui::Stroke::new(
                            1.0_f32,
                            ui_style::contrasting_canvas_text(
                                self.presentation.preferences.background,
                            ),
                        ),
                    );
                    painter.circle_stroke(b, 3.5, egui::Stroke::new(1.0_f32, color));
                }
            }
        }
    }
}

fn point_segment_distance(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let delta = b - a;
    let length = delta.length_sq();
    if length <= 1e-6 {
        return point.distance(a);
    }
    let t = ((point - a).dot(delta) / length).clamp(0.0, 1.0);
    point.distance(a + delta * t)
}
