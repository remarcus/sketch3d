use super::*;
use crate::junction_edit::{self, Action, Preview, Target};

pub(super) struct JunctionDraft {
    action: Action,
    target: guide_resource_ui::PlanarTarget,
    picked: Option<Picked>,
    redraw: RedrawState,
}

#[derive(Default)]
struct RedrawState {
    first: Option<(Stroke, f64)>,
    second: Option<f64>,
    contact: Option<Vec<Point>>,
    completed: Vec<Point>,
}

impl RedrawState {
    fn joins(&self) -> Option<(&Stroke, [f64; 2])> {
        let (stroke, first) = self.first.as_ref()?;
        Some((stroke, [*first, self.second?]))
    }
}

struct Picked {
    source: Stroke,
    at: f64,
    preview: Preview,
}

impl SketchApp {
    pub(super) fn junction_buttons(&mut self, ui: &mut egui::Ui) {
        for action in [Action::Trim, Action::Extend, Action::Redraw] {
            if ui.button(action.label()).clicked() {
                self.begin_junction_edit(action);
            }
        }
    }

    fn begin_junction_edit(&mut self, action: Action) {
        let Some(target) = self.planar_drawing_target() else {
            self.status = "Activate a planar drawing target first.".into();
            return;
        };
        if !junction_edit::available(&self.doc) {
            self.status = "Intersection edits require static artwork.".into();
            return;
        }
        self.finish();
        self.cancel();
        self.cancel_fill_tool();
        self.cancel_placement();
        self.select_mode = true;
        self.junction_edit = Some(JunctionDraft {
            action,
            target,
            picked: None,
            redraw: RedrawState::default(),
        });
        self.status = action_prompt(action).into();
    }

    pub(super) fn junction_pointer(
        &mut self,
        pos: Pos2,
        pressure: Option<f32>,
        tilt: Option<Vec3>,
        rect: Rect,
        phase: Phase,
    ) {
        let Some(draft) = &self.junction_edit else {
            return;
        };
        if self.planar_drawing_target() != Some(draft.target) {
            self.junction_edit = None;
            self.status = "The drawing plane changed. Start the intersection edit again.".into();
            return;
        }
        let target = core_target(draft.target);
        let action = draft.action;
        if action == Action::Redraw {
            self.redraw_pointer(pos, pressure, tilt, rect, phase);
            return;
        }
        if phase != Phase::Down {
            return;
        }
        let result = pick_stroke(&self.doc, action, target, &self.camera, rect, pos).and_then(
            |(index, at)| {
                let preview =
                    junction_edit::preview(&self.doc, action, target, index, at, self.next_id)?;
                Ok(Picked {
                    source: self.doc.strokes[index].clone(),
                    at,
                    preview,
                })
            },
        );
        let draft = self.junction_edit.as_mut().unwrap();
        match result {
            Ok(picked) => {
                draft.picked = Some(picked);
                self.status = "Preview ready. Enter applies and Escape cancels.".into();
            }
            Err(error) => {
                draft.picked = None;
                self.status = error;
            }
        }
    }

    pub(super) fn redraw_contact_active(&self) -> bool {
        self.junction_edit
            .as_ref()
            .is_some_and(|draft| draft.redraw.contact.is_some())
    }

    pub(super) fn cancel_redraw_contact(&mut self) {
        if let Some(draft) = &mut self.junction_edit {
            draft.redraw.contact = None;
        }
    }

    fn redraw_pointer(
        &mut self,
        pos: Pos2,
        pressure: Option<f32>,
        tilt: Option<Vec3>,
        rect: Rect,
        phase: Phase,
    ) {
        let draft = self.junction_edit.as_mut().unwrap();
        let (Some((source, first)), Some(second)) = (&draft.redraw.first, draft.redraw.second)
        else {
            if phase != Phase::Down {
                return;
            }
            let candidate = pick_stroke(
                &self.doc,
                Action::Redraw,
                core_target(draft.target),
                &self.camera,
                rect,
                pos,
            );
            match candidate {
                Ok((index, at)) => {
                    let stroke = &self.doc.strokes[index];
                    if let Some((first, start)) = &draft.redraw.first {
                        if stroke != first || (at - *start).abs() < 1e-5 {
                            self.status = "Pick a different point on the same stroke.".into();
                            return;
                        }
                        draft.redraw.second = Some(at);
                        self.status =
                            "Draw from the first join to the second, then release.".into();
                    } else {
                        draft.redraw.first = Some((stroke.clone(), at));
                        self.status = "Pick the second join on the same stroke.".into();
                    }
                }
                Err(error) => self.status = error,
            }
            return;
        };
        let joins = [*first, second];
        let [start, end] = joins.map(|at| junction_edit::point_at(source, at));
        let near = |point: Point| {
            crate::gizmo::project(&self.camera, rect, point.position)
                .is_some_and(|(screen, _)| screen.distance(pos) <= 12.0)
        };
        match phase {
            Phase::Down => {
                draft.picked = None;
                draft.redraw.completed.clear();
                if near(start) {
                    draft.redraw.contact = Some(vec![start]);
                    self.status = "Draw to the second join and release.".into();
                } else {
                    draft.redraw.contact = None;
                    self.status = "Start the gesture at the first join.".into();
                }
            }
            Phase::Move => {
                let Some(samples) = &mut draft.redraw.contact else {
                    return;
                };
                let (origin, direction) = crate::gizmo::ray(&self.camera, rect, pos);
                let Some(position) = draft.target.hit(origin, direction) else {
                    draft.redraw.contact = None;
                    self.status =
                        "The gesture left the drawing plane. Start again at the first join.".into();
                    return;
                };
                if samples.len() >= junction_edit::MAX_REDRAW_SAMPLES - 1 {
                    draft.redraw.contact = None;
                    self.status =
                        "The redraw exceeded its sample limit. Try a shorter gesture.".into();
                    return;
                }
                if samples
                    .last()
                    .is_none_or(|point| point.position.distance_squared(position) > 1e-8)
                {
                    samples.push(Point {
                        position,
                        pressure: pressure.unwrap_or(1.0).clamp(0.0, 1.0),
                        tilt,
                        time_ms: 0,
                    });
                }
            }
            Phase::Up => {
                let Some(mut gesture) = draft.redraw.contact.take() else {
                    return;
                };
                if !near(end) {
                    self.status =
                        "Release at the second join. The original stroke is intact.".into();
                    return;
                }
                gesture.push(end);
                let result = self
                    .doc
                    .strokes
                    .iter()
                    .position(|stroke| stroke == source)
                    .ok_or_else(|| "The source stroke changed. Pick the joins again.".to_owned())
                    .and_then(|index| {
                        junction_edit::redraw(
                            &self.doc,
                            core_target(draft.target),
                            index,
                            joins,
                            gesture.clone(),
                            self.next_id,
                        )
                    });
                match result {
                    Ok(preview) => {
                        draft.picked = Some(Picked {
                            source: source.clone(),
                            at: 0.0,
                            preview,
                        });
                        draft.redraw.completed = gesture;
                        self.status =
                            "Redraw preview ready. Enter applies and Escape cancels.".into();
                    }
                    Err(error) => self.status = error,
                }
            }
            Phase::Cancel => draft.redraw.contact = None,
        }
    }

    pub(super) fn junction_controls(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = &self.junction_edit else {
            return;
        };
        ui.heading(draft.action.label());
        ui.label(action_prompt(draft.action));
        if draft.action == Action::Redraw && ui.button("Pick new joins").clicked() {
            let draft = self.junction_edit.as_mut().unwrap();
            draft.redraw = RedrawState::default();
            draft.picked = None;
            return;
        }
        let ready = draft.picked.is_some();
        let (apply, cancel) = ui
            .horizontal(|ui| {
                (
                    ui.add_enabled(ready, egui::Button::new("Apply")).clicked(),
                    ui.button("Cancel").clicked(),
                )
            })
            .inner;
        if apply || cancel {
            self.finish_junction_edit(apply);
        }
    }

    pub(super) fn junction_shortcuts(&mut self, ctx: &egui::Context) {
        if self.junction_edit.is_none() {
            return;
        }
        let (cancel, apply) = ctx.input_mut(|input| {
            (
                !input.focused || input.consume_key(egui::Modifiers::NONE, Key::Escape),
                input.consume_key(egui::Modifiers::NONE, Key::Enter),
            )
        });
        if cancel || apply {
            self.finish_junction_edit(apply && !cancel);
        }
    }

    fn finish_junction_edit(&mut self, apply: bool) {
        if !apply {
            self.junction_edit = None;
            self.status = "Intersection edit cancelled.".into();
            return;
        }
        let Some(draft) = &self.junction_edit else {
            return;
        };
        let Some(picked) = &draft.picked else { return };
        let target = draft.target;
        let action = draft.action;
        let result = self
            .doc
            .strokes
            .iter()
            .position(|stroke| stroke == &picked.source)
            .filter(|_| self.planar_drawing_target() == Some(target))
            .ok_or_else(|| "The stroke or active plane changed. Pick the stroke again.".to_owned())
            .and_then(|index| {
                if action == Action::Redraw {
                    let (_, joins) = draft
                        .redraw
                        .joins()
                        .ok_or("The redraw joins are unavailable")?;
                    return junction_edit::redraw(
                        &self.doc,
                        core_target(target),
                        index,
                        joins,
                        draft.redraw.completed.clone(),
                        self.next_id,
                    )
                    .map(|preview| (index, preview));
                }
                junction_edit::preview(
                    &self.doc,
                    draft.action,
                    core_target(target),
                    index,
                    picked.at,
                    self.next_id,
                )
                .map(|preview| (index, preview))
            });
        let draft = self.junction_edit.as_mut().unwrap();
        let (index, preview) = match result {
            Ok(result) => result,
            Err(error) => {
                draft.picked = None;
                self.status = error;
                return;
            }
        };
        let picked = draft.picked.as_mut().unwrap();
        if picked.preview != preview {
            picked.preview = preview;
            self.status =
                "The intersection changed. Review the updated preview before applying.".into();
            return;
        }
        let before = vec![(index, picked.source.clone())];
        self.selected = preview.after.iter().map(|(_, stroke)| stroke.id).collect();
        let edit = Edit::Change {
            before,
            after: preview.after,
        };
        edit.apply(&mut self.doc, true);
        self.next_id = preview.next_id;
        self.undo.push(edit);
        self.redo.clear();
        self.junction_edit = None;
        self.dirty = true;
        self.record_recovery_edit_labeled(action.label());
        self.renderer.invalidate();
        self.status = format!("{} applied as one undo step.", action.label());
    }

    pub(super) fn junction_overlay(&self, ui: &egui::Ui, rect: Rect) {
        if let Some(draft) = self
            .junction_edit
            .as_ref()
            .filter(|draft| draft.action == Action::Redraw)
        {
            let painter = ui.painter().with_clip_rect(rect);
            let color = ui.visuals().selection.stroke.color;
            let project =
                |point| crate::gizmo::project(&self.camera, rect, point).map(|(pos, _)| pos);
            if let Some((stroke, first)) = &draft.redraw.first {
                for (i, at) in [Some(*first), draft.redraw.second].into_iter().enumerate() {
                    if let Some(screen) =
                        at.and_then(|at| project(junction_edit::point_at(stroke, at).position))
                    {
                        painter.circle_stroke(screen, 6.0, egui::Stroke::new(2.0_f32, color));
                        painter.text(
                            screen + egui::vec2(8.0, -8.0),
                            egui::Align2::LEFT_BOTTOM,
                            i + 1,
                            egui::FontId::proportional(12.0),
                            color,
                        );
                    }
                }
            }
            let mut previous = None;
            let path = draft
                .redraw
                .contact
                .as_deref()
                .unwrap_or(&draft.redraw.completed);
            for sample in path {
                let current = project(sample.position);
                if let (Some(a), Some(b)) = (previous, current) {
                    painter.line_segment([a, b], egui::Stroke::new(3.0_f32, color));
                }
                previous = current;
            }
            return;
        }
        let Some(picked) = self
            .junction_edit
            .as_ref()
            .and_then(|draft| draft.picked.as_ref())
        else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let color = if picked.preview.snap_to.is_some() {
            ui.visuals().selection.stroke.color
        } else {
            ui_style::colors(ui.ctx()).warning
        };
        let project = |point| crate::gizmo::project(&self.camera, rect, point).map(|(pos, _)| pos);
        for pair in picked.preview.path.windows(2) {
            if let (Some(a), Some(b)) = (project(pair[0]), project(pair[1])) {
                painter.line_segment([a, b], egui::Stroke::new(4.0_f32, color));
            }
        }
        for &join in &picked.preview.joins {
            if let Some(point) = project(join) {
                if let Some(id) = picked.preview.snap_to {
                    snapping::overlay(
                        ui,
                        Some(snapping::SnapCue {
                            screen: point,
                            kind: snapping::SnapKind::Intersection,
                            target_ids: [picked.source.id, id],
                        }),
                        rect,
                    );
                } else {
                    painter.circle_stroke(point, 6.0, egui::Stroke::new(2.0_f32, color));
                }
            }
        }
    }
}

fn action_prompt(action: Action) -> &'static str {
    match action {
        Action::Trim => {
            "Click the stroke section to remove. The preview highlights the removed span."
        }
        Action::Extend => "Click an endpoint to extend along its tangent, up to 1 world unit.",
        Action::Redraw => "Pick two joins on one stroke, then draw from join 1 to join 2.",
    }
}

fn core_target(target: guide_resource_ui::PlanarTarget) -> Target {
    Target {
        plane: target.plane,
        half_extent: target.half_extent(),
    }
}

fn pick_stroke(
    doc: &Document,
    action: Action,
    target: Target,
    camera: &Camera,
    rect: Rect,
    pos: Pos2,
) -> Result<(usize, f64), String> {
    let mut picked = PickCandidate::default();
    let mut segment_count = 0;
    let matrix = camera.matrix(rect.aspect_ratio());
    for (index, stroke) in doc.strokes.iter().enumerate() {
        if !junction_edit::editable_source(doc, stroke, target)
            || (action == Action::Redraw && !junction_edit::redraw_source(stroke))
        {
            continue;
        }
        if action == Action::Extend {
            for endpoint in [0, stroke.points.len() - 1] {
                if let Some((screen, _)) =
                    crate::gizmo::project(camera, rect, stroke.points[endpoint].position)
                {
                    picked.consider(pos.distance(screen), index, endpoint as f64, 0.0);
                }
            }
            continue;
        }
        for (segment, pair) in stroke.points.windows(2).enumerate() {
            segment_count += 1;
            if segment_count > 100_000 {
                return Err("Stroke picking exceeds the intersection tool's work limit.".into());
            }
            let [a, b] = [pair[0].position, pair[1].position];
            let (Some((start, _)), Some((end, _))) = (
                crate::gizmo::project(camera, rect, a),
                crate::gizmo::project(camera, rect, b),
            ) else {
                continue;
            };
            let delta = end - start;
            if delta.length_sq() <= 1e-6 {
                continue;
            }
            let t = ((pos - start).dot(delta) / delta.length_sq()).clamp(0.0, 1.0);
            let distance = pos.distance(start + delta * t);
            if distance > 10.0 {
                continue;
            }
            // Convert a screen-space fraction back to the perspective-correct fraction on the ink.
            let wa = (matrix * a.extend(1.0)).w;
            let wb = (matrix * b.extend(1.0)).w;
            let at = segment as f64 + f64::from(t * wa / (wb * (1.0 - t) + wa * t));
            picked.consider(distance, index, at, 1.0);
        }
    }
    if picked.ambiguous {
        return Err(
            "Several stroke sections overlap here. Click farther from the crossing.".into(),
        );
    }
    picked
        .best
        .map(|(_, index, at)| (index, at))
        .ok_or_else(|| action_prompt(action).into())
}

#[derive(Default)]
struct PickCandidate {
    best: Option<(f32, usize, f64)>,
    ambiguous: bool,
}

impl PickCandidate {
    fn consider(&mut self, distance: f32, index: usize, at: f64, same_stroke_slop: f64) {
        if distance > 10.0 {
            return;
        }
        if self.best.is_none_or(|(old, _, _)| distance < old - 0.25) {
            self.best = Some((distance, index, at));
            self.ambiguous = false;
        } else if self.best.is_some_and(|(old, old_index, old_at)| {
            (old - distance).abs() <= 0.25
                && (old_index != index || (old_at - at).abs() > same_stroke_slop)
        }) {
            self.ambiguous = true;
        }
    }
}
