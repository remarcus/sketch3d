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
    target: PlanarTarget,
    stroke: Stroke,
    anchors: Vec<Anchor>,
    preview: Option<Stroke>,
    dragging: Option<HandleRef>,
    selected_anchor: Option<usize>,
    editing_stroke_id: Option<u64>,
    viewport_height: f32,
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
        if self.planar_drawing_target().is_none() && !self.has_selected_editable_curve() {
            self.status =
                "Smooth curves require a plane guide. Activate a planar guide first.".into();
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
        ui.label("Click to append anchors or click near a segment to insert one. Reopened curves retain their captured frame and bounds.");
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
        if self.curve_tool.draft.is_some() && self.snapping.options.enabled {
            let (target, excluded) = self
                .curve_tool
                .draft
                .as_ref()
                .map(|draft| {
                    (
                        draft.target,
                        draft.editing_stroke_id.into_iter().collect::<HashSet<_>>(),
                    )
                })
                .expect("curve draft remains active");
            if self.planar_snap_index_stale(target, rect, &excluded) {
                self.prepare_planar_snap(target, rect, &excluded);
            }
        }
        if begin {
            if self.curve_tool.draft.is_none() {
                let Some(target) = self.planar_drawing_target() else {
                    self.status =
                        "Smooth curves require a plane guide. Activate a planar guide first."
                            .into();
                    return;
                };
                let plane = target.plane;
                if !self.doc.active_editable() {
                    self.status = "Choose a visible unlocked layer for the curve.".into();
                    return;
                }
                let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let Some(_) = target.hit(origin, direction) else {
                    return;
                };
                let (u, _, normal) = plane.basis();
                let stroke = Stroke {
                    curve: None,
                    material: Default::default(),
                    fill: None,
                    group_id: None,
                    layer_id: self.doc.active_layer,
                    id: 0,
                    color: self.color,
                    gradient: None,
                    radius: self.radius,
                    brush: Brush {
                        normal,
                        tangent: u,
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
                if self.planar_snap_index_stale(target, rect, &excluded) {
                    self.prepare_planar_snap(target, rect, &excluded);
                }
            }
            let Some(world) = self.curve_world(pos, rect) else {
                return;
            };
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
                let world = self.snap_planar_point(world, pos, target, rect, None);
                let insertion_pos = crate::gizmo::project(&self.camera, rect, world)
                    .map_or(pos, |(screen, _)| screen);
                let insertion = self.curve_tool.draft.as_ref().and_then(|draft| {
                    if draft.anchors.len() < 2 {
                        return None;
                    }
                    let mut nearest: (f32, usize) = (14.0, draft.anchors.len());
                    for pair_index in 0..draft.anchors.len() - 1 {
                        let Some((a, _)) = crate::gizmo::project(
                            &self.camera,
                            rect,
                            draft.anchors[pair_index].position,
                        ) else {
                            continue;
                        };
                        let Some((b, _)) = crate::gizmo::project(
                            &self.camera,
                            rect,
                            draft.anchors[pair_index + 1].position,
                        ) else {
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
                if let Some(draft) = &mut self.curve_tool.draft {
                    let idx = insertion.unwrap_or(draft.anchors.len());
                    draft.anchors.insert(
                        idx,
                        Anchor {
                            position: world,
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
            let world = if let HandleRef::Anchor(_) = handle {
                let target = self
                    .curve_tool
                    .draft
                    .as_ref()
                    .expect("curve draft remains active")
                    .target;
                self.snap_planar_point(world, pos, target, rect, None)
            } else {
                world
            };
            self.curve_move_handle(handle, world);
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
            .or_else(|| self.planar_drawing_target())?;
        let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
        target.hit(origin, direction)
    }

    fn curve_handle_at(&self, draft: &CurveDraft, pos: Pos2, rect: Rect) -> Option<HandleRef> {
        let mut nearest_anchor = (10.0_f32, None);
        let mut nearest_handle = (12.0_f32, None);
        for (index, anchor) in draft.anchors.iter().enumerate() {
            if let Some((screen, _)) = crate::gizmo::project(&self.camera, rect, anchor.position) {
                let distance = screen.distance(pos);
                if distance < nearest_anchor.0 {
                    nearest_anchor = (distance, Some(HandleRef::Anchor(index)));
                }
            }
            for (endpoint, handle) in [
                (
                    anchor.position + anchor.incoming,
                    HandleRef::Incoming(index),
                ),
                (
                    anchor.position + anchor.outgoing,
                    HandleRef::Outgoing(index),
                ),
            ] {
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
        let scale = self
            .camera
            .world_per_pixel(draft.anchors[0].position, draft.viewport_height.max(1.0));
        let tolerance = (scale * 0.75).clamp(
            crate::curve_geometry::MIN_TOLERANCE_WORLD,
            crate::curve_geometry::MAX_TOLERANCE_WORLD,
        );
        match crate::curve_geometry::tessellate_stroke(
            &segments,
            &draft.stroke,
            tolerance,
            crate::curve_geometry::MAX_CURVE_POINTS,
        ) {
            Ok(stroke)
                if stroke
                    .points
                    .iter()
                    .all(|point| draft.target.contains(point.position)) =>
            {
                let (tangent, _, normal) = draft.target.plane.basis();
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
                    frame_origin: draft.target.plane.origin(),
                    frame_normal: normal,
                    frame_tangent: tangent,
                    half_extent: draft.target.half_extent(),
                    tension: self.curve_tool.tension,
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
                self.curve_tool.error =
                    Some("Curve extends beyond the active image bounds.".into());
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
                && stroke.curve.as_ref().is_some_and(CurveData::is_valid)
                && self.doc.editable(stroke)
        })
    }

    fn open_selected_curve(&mut self) {
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
        let Some(target) = PlanarTarget::from_curve_frame(
            curve.frame_origin,
            curve.frame_normal,
            curve.frame_tangent,
            curve.half_extent,
        ) else {
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
            if let Some((point, _)) = crate::gizmo::project(&self.camera, rect, anchor.position) {
                painter.circle_filled(point, 4.5, color);
            }
            for endpoint in [
                anchor.position + anchor.incoming,
                anchor.position + anchor.outgoing,
            ] {
                if let (Some((a, _)), Some((b, _))) = (
                    crate::gizmo::project(&self.camera, rect, anchor.position),
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
