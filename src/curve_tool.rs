use super::ui_style;
use super::*;
use crate::curve_geometry::{CubicBezier, PressureSegment};

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
    Incoming(usize),
    Outgoing(usize),
    NewAnchor(usize),
}

struct CurveDraft {
    plane: DrawingPlane,
    stroke: Stroke,
    anchors: Vec<Anchor>,
    preview: Option<Stroke>,
    dragging: Option<HandleRef>,
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

    pub(super) fn start_curve_tool(&mut self) {
        if self.resource_target_active() {
            self.status = "Curve tool currently uses the temporary plane guide. Switch to it before drawing a curve.".into();
            return;
        }
        self.finish();
        self.cancel_line_tool();
        self.cancel_fill_tool();
        self.cancel_shape_draft();
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
        ui.label("Click to append anchors or click near a segment to insert one. Drag an anchor to adjust its tangent. Planar guides only.");
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
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(count > 0, egui::Button::new("Remove last point"))
                .clicked()
            {
                self.curve_remove_last();
            }
            if ui
                .add_enabled(count >= 2, egui::Button::new("Finish curve"))
                .clicked()
            {
                self.commit_curve();
            }
            if count > 0 && ui.button("Cancel").clicked() {
                self.cancel_curve();
            }
        });
        ui.small("Enter finishes; Backspace removes the last point; Escape cancels. Commit creates one editable stroke and one undo step.");
    }

    pub(super) fn curve_input(&mut self, pos: Pos2, pressure: f32, rect: Rect, begin: bool) {
        if !rect.contains(pos) {
            return;
        }
        if begin {
            if self.curve_tool.draft.is_none() {
                if self.use_cylinder || self.use_revolved {
                    self.status =
                        "Smooth curves currently require a plane guide. Choose Plane first.".into();
                    return;
                }
                if !self.doc.active_editable() {
                    self.status = "Choose a visible unlocked layer for the curve.".into();
                    return;
                }
                let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let Some(_) = intersect_plane(origin, direction, self.guide) else {
                    return;
                };
                let (u, _, normal) = self.guide.basis();
                let stroke = Stroke {
                    fill: None,
                    group_id: None,
                    layer_id: self.doc.active_layer,
                    id: 0,
                    color: self.color,
                    radius: self.radius,
                    brush: Brush {
                        normal,
                        tangent: u,
                        ..self.brush
                    },
                    points: Vec::new(),
                };
                self.curve_tool.draft = Some(CurveDraft {
                    plane: self.guide,
                    stroke,
                    anchors: Vec::new(),
                    preview: None,
                    dragging: None,
                    viewport_height: rect.height().max(1.0),
                });
            }
            let Some(world) = self.curve_world(pos, rect) else {
                return;
            };
            let target = self
                .curve_tool
                .draft
                .as_ref()
                .and_then(|draft| self.curve_handle_at(draft, pos, rect));
            if let Some(handle) = target {
                if let Some(draft) = &mut self.curve_tool.draft {
                    draft.dragging = Some(handle);
                }
            } else {
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
                        let distance = point_segment_distance(pos, a, b);
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
            self.curve_move_handle(handle, world);
            self.curve_refresh(rect);
        }
    }

    pub(super) fn curve_pointer_up(&mut self) {
        if let Some(draft) = &mut self.curve_tool.draft {
            draft.dragging = None;
        }
    }

    fn curve_world(&self, pos: Pos2, rect: Rect) -> Option<Vec3> {
        let plane = self
            .curve_tool
            .draft
            .as_ref()
            .map_or(self.guide, |draft| draft.plane);
        let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
        intersect_plane(origin, direction, plane)
    }

    fn curve_handle_at(&self, draft: &CurveDraft, pos: Pos2, rect: Rect) -> Option<HandleRef> {
        let mut nearest = (12.0_f32, None);
        for (index, anchor) in draft.anchors.iter().enumerate() {
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
                    if distance < nearest.0 {
                        nearest = (distance, Some(handle));
                    }
                }
            }
        }
        nearest.1
    }

    fn curve_move_handle(&mut self, handle: HandleRef, world: Vec3) {
        let Some(draft) = &mut self.curve_tool.draft else {
            return;
        };
        let index = match handle {
            HandleRef::Incoming(i) | HandleRef::Outgoing(i) | HandleRef::NewAnchor(i) => i,
        };
        let Some(anchor) = draft.anchors.get_mut(index) else {
            return;
        };
        let offset = world - anchor.position;
        match handle {
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
            Ok(stroke) => {
                draft.preview = Some(stroke);
                self.curve_tool.error = None;
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
        }
        self.curve_tool.error = None;
        let height = self
            .curve_tool
            .draft
            .as_ref()
            .map_or(900.0, |draft| draft.viewport_height);
        self.curve_refresh(Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, height)));
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
        self.commit_drawn_stroke(stroke, false);
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
                self.curve_remove_last();
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
        let color = Color32::from_rgb(76, 220, 190);
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
                        egui::Stroke::new(1.0_f32, Color32::from_rgb(145, 170, 165)),
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
