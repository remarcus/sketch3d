use super::ui_style;
use super::*;
use crate::{
    editing::{self, Snapshot},
    gizmo::{self, HandleKind, Layout, Mode},
};
use glam::{Mat3, Quat};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PivotMode {
    #[default]
    SelectionCenter,
    WorldOrigin,
    Custom,
}

pub(super) struct Drag {
    before: Snapshot,
    pivot: Vec3,
    kind: HandleKind,
    start: Pos2,
    view_normal: Vec3,
    translation: Vec3,
    scale_basis: Quat,
    snap_target: Option<super::guide_resource_ui::PlanarTarget>,
    snap_source_points: Vec<Vec3>,
}

pub(super) struct BrushSelectionDrag {
    starting_selection: HashSet<u64>,
    path: Vec<Pos2>,
}

impl BrushSelectionDrag {
    pub(super) fn path_len(&self) -> usize {
        self.path.len()
    }
}

pub(super) struct AppearanceDraft {
    before: Snapshot,
    after: Snapshot,
    color: [f32; 3],
    gradient_start: [f32; 3],
    gradient_end: [f32; 3],
    radius: f32,
    opacity: f32,
    brush_kind: crate::document::BrushKind,
    material: crate::document::StrokeMaterial,
    set_color: bool,
    set_gradient: bool,
    gradient_enabled: bool,
    set_radius: bool,
    set_opacity: bool,
    set_brush_kind: bool,
    set_material: bool,
    mixed_color: bool,
    mixed_gradient: bool,
    mixed_radius: bool,
    mixed_opacity: bool,
    mixed_brush_kind: bool,
    mixed_material: bool,
    has_preview: bool,
}

impl AppearanceDraft {
    fn new(before: Snapshot) -> Option<Self> {
        let first = before.first()?.1.clone();
        Some(Self {
            mixed_color: before.iter().any(|(_, stroke)| stroke.color != first.color),
            mixed_gradient: before
                .iter()
                .any(|(_, stroke)| stroke.gradient != first.gradient),
            mixed_radius: before
                .iter()
                .any(|(_, stroke)| stroke.radius != first.radius),
            mixed_opacity: before
                .iter()
                .any(|(_, stroke)| stroke.brush.opacity != first.brush.opacity),
            mixed_brush_kind: before
                .iter()
                .any(|(_, stroke)| stroke.brush.kind != first.brush.kind),
            mixed_material: before
                .iter()
                .any(|(_, stroke)| stroke.material != first.material),
            after: before.clone(),
            color: first.color,
            gradient_start: first
                .gradient
                .map_or(first.color, |gradient| gradient.start),
            gradient_end: first.gradient.map_or(first.color, |gradient| gradient.end),
            radius: first.radius,
            opacity: first.brush.opacity,
            brush_kind: first.brush.kind,
            material: first.material,
            before,
            set_color: false,
            set_gradient: false,
            gradient_enabled: first.gradient.is_some(),
            set_radius: false,
            set_opacity: false,
            set_brush_kind: false,
            set_material: false,
            has_preview: false,
        })
    }
    fn patch(&self) -> editing::AppearancePatch {
        editing::AppearancePatch {
            color: self.set_color.then_some(self.color),
            gradient: self.set_gradient.then_some(self.gradient_enabled.then_some(
                crate::document::StrokeGradient {
                    start: self.gradient_start,
                    end: self.gradient_end,
                },
            )),
            radius: self.set_radius.then_some(self.radius),
            opacity: self.set_opacity.then_some(self.opacity),
            brush_kind: self.set_brush_kind.then_some(self.brush_kind),
            material: self.set_material.then_some(self.material),
        }
    }
}

#[derive(Clone, Copy)]
struct StrokeJoinUiState {
    ids: [u64; 2],
    reverse_order: bool,
    tolerance: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RepeatSpace {
    World,
    Guide,
}

#[derive(Clone)]
pub(super) struct RepeatState {
    source: Snapshot,
    count: usize,
    spacing: [f32; 3],
    space: RepeatSpace,
}

const MAX_REPEAT_COPIES: usize = 100;
const MAX_DOCUMENT_STROKES: usize = 100_000;
const MAX_DOCUMENT_POINTS: usize = 2_000_000;

impl SketchApp {
    fn begin_appearance_edit(&mut self) {
        let before = editing::snapshot(&self.doc, &self.selected);
        if before.is_empty() || before.len() != self.selected.len() {
            self.status = "Appearance editing requires a complete editable selection.".into();
            return;
        }
        self.appearance_draft = AppearanceDraft::new(before);
    }

    fn refresh_appearance_preview(&mut self) {
        let Some(draft) = self.appearance_draft.as_mut() else {
            return;
        };
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview {
            editing::replace(&mut self.doc, &draft.after, &Vec::new(), false);
        }
        match editing::appearance_preview(&draft.before, draft.patch()) {
            Ok(after) => {
                if after != draft.before {
                    editing::replace(&mut self.doc, &draft.before, &after, true);
                    draft.after = after;
                    draft.has_preview = true;
                } else {
                    draft.after = draft.before.clone();
                    draft.has_preview = false;
                }
                self.renderer
                    .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
                self.status =
                    "Appearance preview only. Apply to keep it or Cancel to restore.".into();
            }
            Err(error) => {
                draft.after = draft.before.clone();
                draft.has_preview = false;
                self.renderer
                    .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
                self.status = error;
            }
        }
    }

    pub(super) fn finish_appearance_edit(&mut self, apply: bool) {
        let Some(draft) = self.appearance_draft.take() else {
            return;
        };
        let ids: HashSet<_> = draft.before.iter().map(|(_, stroke)| stroke.id).collect();
        if draft.has_preview && apply {
            editing::replace(&mut self.doc, &draft.after, &Vec::new(), false);
            match editing::appearance_edit(&self.doc, &self.selected, draft.patch()) {
                Ok(edit) => {
                    edit.apply(&mut self.doc, true);
                    self.undo.push(edit);
                    self.redo.clear();
                    self.dirty = true;
                    self.record_recovery_edit();
                    self.status =
                        "Selected stroke appearance updated. One undo restores the originals."
                            .into();
                }
                Err(error) => self.status = error,
            }
            self.renderer
                .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
        } else {
            if draft.has_preview {
                editing::replace(&mut self.doc, &draft.after, &Vec::new(), false);
                self.renderer
                    .invalidate_ids(&self.doc.linked_affected_stroke_ids(&ids));
            }
            self.status = if apply {
                "No appearance properties were changed.".into()
            } else {
                "Appearance edit cancelled.".into()
            };
        }
    }

    pub(super) fn update_brush_selection(&mut self, pos: Pos2, rect: Rect) {
        let Some(drag) = self.brush_select_drag.as_mut() else {
            return;
        };
        if drag
            .path
            .last()
            .is_none_or(|last| last.distance(pos) >= 1.0)
            && drag.path.len() < 4096
        {
            drag.path.push(pos);
        }
        let result = crate::brush_selection::hit_ids(
            &self.doc,
            &self.camera,
            rect,
            &drag.path,
            self.brush_select_radius,
            self.brush_select_through,
            self.individual_selection,
        );
        match result {
            Ok(hits) => {
                self.selected = crate::brush_selection::apply_brush_selection(
                    &drag.starting_selection,
                    &hits,
                    if self.brush_select_deselect {
                        crate::brush_selection::BrushSelectionOperation::Deselect
                    } else {
                        crate::brush_selection::BrushSelectionOperation::Select
                    },
                );
            }
            Err(error) => self.status = error.into(),
        }
    }

    pub(super) fn commit_brush_selection(&mut self) {
        if self.brush_select_drag.take().is_some() {
            self.status = format!(
                "Brush selection finished: {} stroke(s) selected.",
                self.selected.len()
            );
        }
    }

    pub(super) fn cancel_brush_selection(&mut self) {
        if let Some(drag) = self.brush_select_drag.take() {
            self.selected = drag.starting_selection;
            self.status = "Brush selection cancelled.".into();
        }
    }

    pub(super) fn sync_selection(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let present: HashSet<_> = self
            .doc
            .strokes
            .iter()
            .filter(|s| self.doc.editable(s))
            .map(|s| s.id)
            .collect();
        self.selected.retain(|id| present.contains(id));
    }
    pub(super) fn commit_transform(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.clear_planar_snap();
        let ids = drag.before.iter().map(|(_, s)| s.id).collect();
        let after = editing::snapshot(&self.doc, &ids);
        if drag.before != after {
            self.undo.push(Edit::Change {
                before: drag.before,
                after,
            });
            self.redo.clear();
            self.dirty = true;
            self.record_recovery_edit();
            self.status = "Transform applied. Undo restores the entire gesture.".into();
        }
    }
    pub(super) fn cancel_transform(&mut self) {
        if let Some(drag) = self.drag.take() {
            editing::replace(&mut self.doc, &drag.before, &Vec::new(), false);
            self.renderer
                .invalidate_ids(&self.doc.linked_affected_stroke_ids(&self.selected));
            self.status = "Transform cancelled".into();
        }
        self.clear_planar_snap();
    }
    fn layout(&self, rect: Rect) -> Option<Layout> {
        if !self.select_mode {
            return None;
        }
        if self
            .selected
            .iter()
            .any(|id| self.doc.linked_instance_for_stroke(*id).is_some())
        {
            return None;
        }
        let (min, max) = editing::bounds(&self.doc, &self.selected)?;
        let center = (min + max) * 0.5;
        let selected_pivot = self
            .drag
            .as_ref()
            .map(|d| d.pivot + d.translation)
            .unwrap_or_else(|| self.transform_pivot_value(center));
        let pivot = if self.drag.is_some() {
            selected_pivot
        } else {
            self.transform_pivot_value(center)
        };
        Layout::new(&self.camera, rect, pivot, self.transform_mode)
    }

    fn transform_pivot_value(&self, selection_center: Vec3) -> Vec3 {
        match self.transform_pivot {
            PivotMode::SelectionCenter => selection_center,
            PivotMode::WorldOrigin => Vec3::ZERO,
            PivotMode::Custom => self.custom_pivot,
        }
    }
    pub(super) fn select_pointer(&mut self, pos: Pos2, rect: Rect, begin: bool, shift: bool) {
        if self.point_draft.is_some() {
            self.canvas_point_pointer(pos, rect, begin);
            return;
        }
        if self.appearance_draft.is_some() {
            return;
        }
        if begin && self.brush_select_enabled {
            self.finish();
            self.brush_select_drag = Some(BrushSelectionDrag {
                starting_selection: self.selected.clone(),
                path: vec![pos],
            });
            self.update_brush_selection(pos, rect);
            return;
        }
        if self.brush_select_drag.is_some() {
            self.update_brush_selection(pos, rect);
            return;
        }
        if begin {
            self.finish();
            if !shift {
                if let Some(layout) = self.layout(rect) {
                    if let Some(kind) = layout.hit(pos) {
                        let before = editing::snapshot(&self.doc, &self.selected);
                        let snap_target = (self.transform_mode == Mode::Move
                            && self.snapping.options.enabled)
                            .then(|| self.planar_drawing_target())
                            .flatten();
                        let snap_source_points = snap_target
                            .map_or_else(Vec::new, |target| selected_snap_points(&before, target));
                        if let Some(target) = snap_target {
                            let excluded = self.selected.clone();
                            self.prepare_planar_snap(target, rect, &excluded);
                        } else {
                            self.clear_planar_snap();
                        }
                        self.drag = Some(Drag {
                            before,
                            pivot: layout.pivot,
                            kind,
                            start: pos,
                            view_normal: (self.camera.eye() - layout.pivot).normalize(),
                            translation: Vec3::ZERO,
                            scale_basis: {
                                let (right, up, forward) = self.camera.basis();
                                Quat::from_mat3(&Mat3::from_cols(right, up, forward))
                            },
                            snap_target,
                            snap_source_points,
                        });
                        return;
                    }
                }
            }
            let hit = gizmo::pick(&self.doc, &self.camera, rect, pos);
            let model_hit = if hit.is_none() {
                self.pick_obj_reference(rect, pos)
            } else {
                None
            };
            if !shift {
                self.selected.clear();
                self.selected_model_reference = None;
            }
            if let Some(id) = hit {
                let ids = self.doc.selection_for_stroke(id, self.individual_selection);
                editing::toggle_selection(&mut self.selected, &ids, shift);
            } else if let Some(id) = model_hit {
                self.selected_model_reference =
                    if shift && self.selected_model_reference == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                self.status = format!(
                    "{} selected. Transform it in Drawing → OBJ references.",
                    self.doc
                        .model_references
                        .iter()
                        .find(|resource| resource.id == id)
                        .map(|resource| resource.name.as_str())
                        .unwrap_or("OBJ reference")
                );
                return;
            }
            self.status = format!(
                "{} stroke(s) selected. Shift-click toggles selection.",
                self.selected.len()
            );
        } else {
            self.preview_transform(pos, rect);
        }
    }
    fn preview_transform(&mut self, pos: Pos2, rect: Rect) {
        if let Some(target) = self.drag.as_ref().and_then(|drag| drag.snap_target) {
            let excluded = self.selected.clone();
            if self.planar_snap_index_stale(target, rect, &excluded) {
                self.prepare_planar_snap(target, rect, &excluded);
            }
        }
        let Some(drag) = self.drag.as_ref() else {
            return;
        };
        let mut translation = Vec3::ZERO;
        let mut rotation = Quat::IDENTITY;
        let mut scale = Vec3::ONE;
        let mut scale_basis = Quat::IDENTITY;
        match drag.kind {
            HandleKind::Axis(i) => {
                let axis = gizmo::axis(i);
                let (Some(start), Some(current)) = (
                    gizmo::axis_parameter(&self.camera, rect, drag.start, drag.pivot, axis),
                    gizmo::axis_parameter(&self.camera, rect, pos, drag.pivot, axis),
                ) else {
                    return;
                };
                translation = axis * (current - start);
            }
            HandleKind::Free => {
                let (Some(start), Some(current)) = (
                    gizmo::plane_hit(&self.camera, rect, drag.start, drag.pivot, drag.view_normal),
                    gizmo::plane_hit(&self.camera, rect, pos, drag.pivot, drag.view_normal),
                ) else {
                    return;
                };
                translation = current - start;
            }
            HandleKind::Rotation(i) => {
                let axis = gizmo::axis(i);
                let (Some(start), Some(current)) = (
                    gizmo::plane_hit(&self.camera, rect, drag.start, drag.pivot, axis),
                    gizmo::plane_hit(&self.camera, rect, pos, drag.pivot, axis),
                ) else {
                    return;
                };
                let (Some(a), Some(b)) = (
                    (start - drag.pivot).try_normalize(),
                    (current - drag.pivot).try_normalize(),
                ) else {
                    return;
                };
                let angle = axis.dot(a.cross(b)).atan2(a.dot(b));
                rotation = Quat::from_axis_angle(axis, angle);
            }
            HandleKind::Scale => {
                let delta = pos - drag.start;
                let min = if self.scale_brush_width {
                    drag.before
                        .iter()
                        .map(|(_, s)| 0.001 / s.radius)
                        .fold(0.01_f32, f32::max)
                } else {
                    0.01
                };
                let max = if self.scale_brush_width {
                    drag.before
                        .iter()
                        .map(|(_, s)| 1.0 / s.radius)
                        .fold(100.0_f32, f32::min)
                } else {
                    100.0
                };
                let uniform = ((delta.x - delta.y) * 0.01).exp().clamp(min, max);
                scale = Vec3::splat(uniform);
            }
            HandleKind::ScaleAxis(i) => {
                let axis = gizmo::axis(i);
                let length = 80.0 * self.camera.world_per_pixel(drag.pivot, rect.height());
                let Some((end, _)) = gizmo::project(&self.camera, rect, drag.pivot + axis * length)
                else {
                    return;
                };
                let Some((origin, _)) = gizmo::project(&self.camera, rect, drag.pivot) else {
                    return;
                };
                let direction = end - origin;
                if direction.length_sq() <= f32::EPSILON {
                    return;
                }
                let direction = direction.normalized();
                let factor = ((pos - drag.start).dot(direction) * 0.01)
                    .exp()
                    .clamp(0.01, 100.0);
                if self.scale_uniform_lock {
                    scale = Vec3::splat(factor);
                } else {
                    scale[i] = factor;
                }
            }
            HandleKind::ScaleView => {
                let delta = pos - drag.start;
                let x = (delta.x * 0.01).exp().clamp(0.01, 100.0);
                let y = (-delta.y * 0.01).exp().clamp(0.01, 100.0);
                if self.scale_uniform_lock {
                    scale = Vec3::splat((x * y).sqrt());
                } else {
                    scale = Vec3::new(x, y, 1.0);
                    scale_basis = drag.scale_basis;
                }
            }
        }
        let mut snap_cue = None;
        if let Some(target) = drag.snap_target {
            let options = self.snapping.options;
            if options.enabled && !options.bypass {
                if let Some(index) = self.snapping.index.as_ref() {
                    let mut best: Option<(f32, Vec3, Vec3, super::snapping::SnapCue)> = None;
                    for source in &drag.snap_source_points {
                        let moved = *source + translation;
                        let origin = target.plane.origin();
                        let normal = target.plane.basis().2;
                        if (moved - origin).dot(normal).abs() > 0.005 {
                            continue;
                        }
                        let Some((screen, _)) = gizmo::project(&self.camera, rect, moved) else {
                            continue;
                        };
                        let Some((snapped, cue)) =
                            index.query(&self.doc, screen, options, None, false)
                        else {
                            continue;
                        };
                        let distance = screen.distance(cue.screen);
                        if best.is_none_or(|(old_distance, _, _, old_cue)| {
                            distance < old_distance - 0.001
                                || ((distance - old_distance).abs() <= 0.001
                                    && cue.kind.priority() < old_cue.kind.priority())
                        }) {
                            best = Some((distance, snapped, snapped - *source, cue));
                        }
                    }
                    if let Some((_, _snapped, snapped_translation, cue)) =
                        best.filter(|(_, snapped, _, cue)| {
                            index.candidate_visible(&self.doc, &self.camera, rect, *cue)
                                && !super::snapping::snap_occluded_by_scene_resources(
                                    &self.doc,
                                    &self.obj_reference_models,
                                    &mut self.obj_surface_indices,
                                    &self.camera,
                                    rect,
                                    cue.screen,
                                    *snapped,
                                )
                        })
                    {
                        translation = snapped_translation;
                        snap_cue = Some(cue);
                    }
                }
            }
        }
        self.snapping.cue = snap_cue;
        match editing::transformed_with_scale(
            &drag.before,
            drag.pivot,
            translation,
            rotation,
            scale,
            scale_basis,
            self.scale_brush_width,
        ) {
            Ok(after) => {
                editing::replace(&mut self.doc, &drag.before, &after, true);
                self.renderer
                    .invalidate_ids(&self.doc.linked_affected_stroke_ids(&self.selected));
                if let Some(drag) = self.drag.as_mut() {
                    drag.translation = translation;
                }
            }
            Err(error) => self.status = error.into(),
        }
    }
    pub(super) fn duplicate_selected(&mut self) {
        self.finish();
        if self
            .selected
            .iter()
            .any(|id| self.doc.linked_instance_for_stroke(*id).is_some())
        {
            self.status =
                "Make the linked instance unique before duplicating or editing it.".into();
            return;
        }
        let before = editing::snapshot(&self.doc, &self.selected);
        if before.is_empty() {
            return;
        }
        let Some(next) = self
            .next_id
            .checked_add(before.len() as u64)
            .filter(|n| *n < u64::MAX)
        else {
            self.status = "No more stroke IDs available".into();
            return;
        };
        let point_count: usize = self.doc.strokes.iter().map(|s| s.points.len()).sum();
        let copied_points: usize = before.iter().map(|(_, s)| s.points.len()).sum();
        if point_count + copied_points > 2_000_000 {
            self.status = "Document point limit reached".into();
            return;
        }
        if self.doc.strokes.len() + before.len() > 100000 {
            self.status = "Document stroke limit reached".into();
            return;
        }
        let shift = self.active_guide_u() * 0.35;
        let Ok(mut after) = editing::transformed(&before, Vec3::ZERO, shift, Quat::IDENTITY, 1.0)
        else {
            self.status = "Cannot duplicate beyond document bounds".into();
            return;
        };
        if let Err(error) = editing::remap_duplicate_groups(&self.doc, &before, &mut after) {
            self.status = error;
            return;
        }
        self.selected.clear();
        for (index, (position, stroke)) in after.iter_mut().enumerate() {
            *position = self.doc.strokes.len() + index;
            stroke.id = self.next_id + index as u64;
            self.selected.insert(stroke.id);
        }
        self.next_id = next;
        let edit = Edit::Change {
            before: Vec::new(),
            after,
        };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.status =
            "Duplicated selection with a small offset. Move the copies with the handles.".into();
    }

    fn repeated_strokes(&self, repeat: &RepeatState) -> Result<Vec<(usize, Stroke)>, String> {
        if repeat.count == 0 || repeat.count > MAX_REPEAT_COPIES {
            return Err(format!("Repeat count must be 1–{MAX_REPEAT_COPIES}"));
        }
        let selected: HashSet<_> = repeat.source.iter().map(|(_, stroke)| stroke.id).collect();
        if selected.len() != self.selected.len()
            || repeat
                .source
                .iter()
                .any(|(_, stroke)| !self.doc.editable(stroke))
        {
            return Err("Repeat requires a visible selection on editable layers".into());
        }
        let spacing = Vec3::from(repeat.spacing);
        if !spacing.is_finite() || spacing.length_squared() <= 1.0e-10 {
            return Err("Enter a non-zero spacing offset".into());
        }
        let stroke_count = repeat
            .source
            .len()
            .checked_mul(repeat.count)
            .ok_or("Repeat would create too many strokes")?;
        if self
            .doc
            .strokes
            .len()
            .checked_add(stroke_count)
            .is_none_or(|count| count > MAX_DOCUMENT_STROKES)
        {
            return Err("Repeat exceeds the 100,000-stroke document limit".into());
        }
        let source_points: usize = repeat
            .source
            .iter()
            .map(|(_, stroke)| stroke.points.len())
            .sum();
        let copied_points = source_points
            .checked_mul(repeat.count)
            .ok_or("Repeat would create too many points")?;
        let total_points: usize = self
            .doc
            .strokes
            .iter()
            .map(|stroke| stroke.points.len())
            .sum();
        if total_points
            .checked_add(copied_points)
            .is_none_or(|count| count > MAX_DOCUMENT_POINTS)
        {
            return Err("Repeat exceeds the 2,000,000-point document limit".into());
        }
        let total_ids = u64::try_from(stroke_count).map_err(|_| "No more stroke IDs available")?;
        if self
            .next_id
            .checked_add(total_ids)
            .is_none_or(|next| next == u64::MAX)
        {
            return Err("No more stroke IDs available".into());
        }
        let local_offset = match repeat.space {
            RepeatSpace::World => spacing,
            RepeatSpace::Guide => {
                let rotation = if self.use_cylinder {
                    self.cylinder.rotation()
                } else if self.use_revolved {
                    self.revolved.rotation()
                } else {
                    self.guide.rotation()
                };
                rotation * spacing
            }
        };
        let mut next_group = self
            .doc
            .strokes
            .iter()
            .filter_map(|stroke| stroke.group_id)
            .max()
            .unwrap_or(0);
        let mut copies = Vec::with_capacity(stroke_count);
        for instance in 1..=repeat.count {
            let translation = local_offset * instance as f32;
            let mut copy =
                editing::transformed(&repeat.source, Vec3::ZERO, translation, Quat::IDENTITY, 1.0)
                    .map_err(str::to_owned)?;
            editing::remap_duplicate_groups_with_next(
                &self.doc,
                &repeat.source,
                &mut copy,
                &mut next_group,
            )?;
            for (_, stroke) in &mut copy {
                stroke.id = self.next_id + copies.len() as u64;
                copies.push((0, stroke.clone()));
            }
        }
        let base = self.doc.strokes.len();
        for (index, (position, _)) in copies.iter_mut().enumerate() {
            *position = base + index;
        }
        Ok(copies)
    }

    fn update_repeat_preview(&mut self) -> Result<Vec<Stroke>, String> {
        let Some(repeat) = &self.repeat else {
            self.renderer.set_repeat_preview(&[], &self.doc);
            return Ok(Vec::new());
        };
        match self.repeated_strokes(repeat) {
            Ok(copies) => {
                let strokes: Vec<_> = copies.into_iter().map(|(_, stroke)| stroke).collect();
                self.renderer.set_repeat_preview(&strokes, &self.doc);
                Ok(strokes)
            }
            Err(error) => {
                self.renderer.set_repeat_preview(&[], &self.doc);
                Err(error)
            }
        }
    }

    fn begin_repeat(&mut self) {
        self.finish();
        let source = editing::snapshot(&self.doc, &self.selected);
        if source.is_empty() || source.len() != self.selected.len() {
            self.status = "Select visible strokes on unlocked layers to repeat".into();
            return;
        }
        let guide_rotation = if self.use_cylinder {
            self.cylinder.rotation()
        } else if self.use_revolved {
            self.revolved.rotation()
        } else {
            self.guide.rotation()
        };
        let axis = guide_rotation * Vec3::X;
        let (minimum, maximum) = source
            .iter()
            .flat_map(|(_, stroke)| {
                let extent = crate::geometry::stroke_extent(stroke);
                stroke.points.iter().map(move |point| {
                    let projection = point.position.dot(axis);
                    (projection - extent, projection + extent)
                })
            })
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), range| {
                (min.min(range.0), max.max(range.1))
            });
        let spacing = (maximum - minimum + 0.2).max(0.35);
        self.repeat = Some(RepeatState {
            source,
            count: 3,
            spacing: [spacing, 0.0, 0.0],
            space: RepeatSpace::Guide,
        });
        if let Err(error) = self.update_repeat_preview() {
            self.status = error;
        }
    }

    fn cancel_repeat(&mut self) {
        self.repeat = None;
        self.renderer.set_repeat_preview(&[], &self.doc);
    }

    fn apply_repeat(&mut self) {
        let Some(repeat) = &self.repeat else {
            return;
        };
        let copies = match self.repeated_strokes(repeat) {
            Ok(copies) => copies,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if copies.is_empty() {
            self.status = "Repeat has no copies to apply".into();
            return;
        }
        let new_selection: HashSet<_> = copies.iter().map(|(_, stroke)| stroke.id).collect();
        let edit = Edit::Change {
            before: Vec::new(),
            after: copies,
        };
        self.next_id += new_selection.len() as u64;
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.selected = new_selection;
        self.cancel_repeat();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer.invalidate();
        self.status = "Repeated copies added. Undo removes the entire repeat.".into();
    }

    pub(super) fn repeat_modal(&mut self, ctx: &egui::Context) {
        if self.repeat.is_none() {
            return;
        }
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.cancel_repeat();
            return;
        }
        let preview = self
            .repeat
            .as_ref()
            .map(|repeat| self.repeated_strokes(repeat));
        let mut apply = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("repeat-details-modal")).show(ctx, |ui| {
            ui.set_min_width(340.0);
            ui.heading("Repeat details");
            let repeat = self.repeat.as_mut().unwrap();
            ui_style::integer_slider_row(
                ui,
                "Copies",
                "repeat-count",
                &mut repeat.count,
                1..=MAX_REPEAT_COPIES,
            );
            for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                ui_style::numeric_row(
                    ui,
                    &format!("Spacing {label}"),
                    ("repeat-spacing", axis),
                    &mut repeat.spacing[axis],
                    Some(-1000.0..=1000.0),
                    "wu",
                );
            }
            ui.horizontal(|ui| {
                ui.label("Offset axes");
                ui.radio_value(&mut repeat.space, RepeatSpace::World, "World");
                ui.radio_value(&mut repeat.space, RepeatSpace::Guide, "Guide");
            });
            match preview.as_ref().unwrap() {
                Ok(copies) => {
                    ui.label(format!("Previewing {} strokes", copies.len()));
                    ui.label("Copies remain individually editable; Apply is one undo step.");
                }
                Err(error) => {
                    ui.colored_label(ui_style::colors(ui.ctx()).error, error);
                }
            }
            ui.horizontal(|ui| {
                apply = ui
                    .add_enabled(
                        preview.as_ref().is_some_and(Result::is_ok),
                        egui::Button::new("Apply"),
                    )
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        let _ = self.update_repeat_preview();
        if cancel {
            self.cancel_repeat();
        } else if apply {
            self.apply_repeat();
        }
    }

    pub(super) fn verify_repeat(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.repeat_verification_frame else {
            return;
        };
        if frame == 1 {
            self.guide.rotation_degrees = Vec3::new(0.0, 37.0, 0.0);
            let source_ids: HashSet<_> = self
                .doc
                .strokes
                .iter()
                .filter(|stroke| self.doc.editable(stroke))
                .map(|stroke| stroke.id)
                .collect();
            assert!(self.doc.strokes.iter().any(|stroke| stroke.fill.is_some()));
            let grouped = editing::group_selected(&self.doc, &source_ids).unwrap();
            grouped.apply(&mut self.doc, true);
            self.undo.push(grouped);
            self.selected = source_ids.clone();
            self.select_mode = true;
            let group = self.doc.strokes[0].group_id.unwrap();
            let grouped_doc = self.doc.strokes.clone();
            let history_depth = self.undo.len();
            let repeat = RepeatState {
                source: editing::snapshot(&self.doc, &source_ids),
                count: 3,
                spacing: [2.0, 0.0, 0.0],
                space: RepeatSpace::Guide,
            };
            self.repeat = Some(repeat.clone());
            let copies = self
                .repeated_strokes(self.repeat.as_ref().unwrap())
                .unwrap();
            assert_eq!(copies.len(), source_ids.len() * 3);
            let basis = self.guide.rotation() * Vec3::X;
            for instance in 0..3 {
                let copy_group = copies[instance * source_ids.len()].1.group_id.unwrap();
                assert_ne!(copy_group, group);
                assert!(
                    copies[instance * source_ids.len()..(instance + 1) * source_ids.len()]
                        .iter()
                        .all(|(_, stroke)| stroke.group_id == Some(copy_group))
                );
                for (source_index, (_, stroke)) in copies
                    [instance * source_ids.len()..(instance + 1) * source_ids.len()]
                    .iter()
                    .enumerate()
                {
                    let source = &self.repeat.as_ref().unwrap().source[source_index].1;
                    match (stroke.fill, source.fill) {
                        (Some(actual), Some(expected)) => {
                            assert!((actual.offset - expected.offset).abs() < 1e-6);
                        }
                        (None, None) => {}
                        _ => panic!("Repeated stroke changed its fill kind"),
                    }
                    assert_eq!(stroke.color, source.color);
                    assert!((stroke.brush.normal - source.brush.normal).length() < 1e-6);
                    assert!((stroke.brush.tangent - source.brush.tangent).length() < 1e-6);
                    let mut expected_brush = source.brush;
                    expected_brush.normal = stroke.brush.normal;
                    expected_brush.tangent = stroke.brush.tangent;
                    assert_eq!(stroke.brush, expected_brush);
                    assert!((stroke.radius - source.radius).abs() < 1e-6);
                    assert_eq!(stroke.layer_id, source.layer_id);
                    assert!(
                        (stroke.points[0].position
                            - source.points[0].position
                            - basis * (2.0 * (instance + 1) as f32))
                            .length()
                            < 1.0e-4
                    );
                }
            }
            let expected_copy_count = copies.len();
            let copied_ids: HashSet<_> = copies.iter().map(|(_, stroke)| stroke.id).collect();
            assert_eq!(copied_ids.len(), expected_copy_count);
            assert!(copied_ids.is_disjoint(&source_ids));
            self.renderer.set_repeat_preview(
                &copies
                    .iter()
                    .map(|(_, stroke)| stroke.clone())
                    .collect::<Vec<_>>(),
                &self.doc,
            );
            self.cancel_repeat();
            assert_eq!(self.doc.strokes, grouped_doc);
            assert_eq!(self.undo.len(), history_depth);
            self.repeat = Some(repeat);
            self.update_repeat_preview().unwrap();
            assert_eq!(expected_copy_count, source_ids.len() * 3);
        }
        if frame == 2 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let root = std::env::current_dir().unwrap().join("verification");
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                root.join("repeat-preview.png"),
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            let grouped_doc = self.doc.strokes.clone();
            let history_depth = self.undo.len();
            self.doc.layers[0].locked = true;
            assert!(self
                .repeated_strokes(self.repeat.as_ref().unwrap())
                .is_err());
            self.doc.layers[0].locked = false;
            let expected = self
                .repeated_strokes(self.repeat.as_ref().unwrap())
                .unwrap();
            let expected_after: Vec<_> =
                expected.iter().map(|(_, stroke)| stroke.clone()).collect();
            self.apply_repeat();
            assert_eq!(self.undo.len(), history_depth + 1);
            let applied_doc = self.doc.strokes.clone();
            assert_eq!(applied_doc.len(), grouped_doc.len() + expected_after.len());
            assert_eq!(&applied_doc[grouped_doc.len()..], expected_after.as_slice());
            let groups: HashSet<_> = expected_after.iter().filter_map(|s| s.group_id).collect();
            assert_eq!(groups.len(), 3);
            let serialized = serde_json::to_string(&self.doc).unwrap();
            let native_path = root.join("repeat.sketch.json");
            std::fs::write(&native_path, serialized).unwrap();
            let loaded =
                Document::from_json(&std::fs::read_to_string(native_path).unwrap()).unwrap();
            assert_eq!(loaded.strokes, self.doc.strokes);
            let export = crate::export::encode(&loaded, None).unwrap();
            assert_eq!(export.summary.strokes as usize, self.doc.strokes.len());
            std::fs::create_dir_all(root.join("exports")).unwrap();
            std::fs::write(root.join("exports/repeat.glb"), &export.bytes).unwrap();
            self.undo();
            assert_eq!(self.doc.strokes, grouped_doc);
            self.redo();
            assert_eq!(self.doc.strokes, applied_doc);
            std::fs::write(
                root.join("repeat-report.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed": true,
                    "checks": [
                        "rotated guide-local spacing",
                        "live preview and cancel preserve artwork/history",
                        "fresh independent group and stroke IDs",
                        "fill, outline, color, brush, and layer metadata preserved",
                        "locked source layer rejected",
                        "apply is one undo step; redo restores exact copies",
                        "native save/reload and GLB output include copies"
                    ],
                    "copies": expected_after.len(),
                    "source_strokes": grouped_doc.len(),
                    "gpu": self.renderer.adapter
                }))
                .unwrap(),
            )
            .unwrap();
            self.cancel_repeat();
            self.recovery.discard();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        assert!(frame < 150, "Repeat screenshot timed out");
        self.repeat_verification_frame = Some(frame + 1);
    }
    pub(super) fn delete_selected(&mut self) {
        self.finish();
        let linked_instances: HashSet<_> = self
            .selected
            .iter()
            .filter_map(|id| self.doc.linked_instance_for_stroke(*id).map(|i| i.id))
            .collect();
        if !linked_instances.is_empty() {
            if linked_instances.len() != 1
                || self
                    .selected
                    .iter()
                    .any(|id| self.doc.linked_instance_for_stroke(*id).is_none())
            {
                self.status =
                    "Delete linked instances one at a time, or make them unique first.".into();
                return;
            }
            let instance_id = *linked_instances.iter().next().unwrap();
            match editing::remove_linked_instance(&self.doc, instance_id, false) {
                Ok(edit) => {
                    edit.apply(&mut self.doc, true);
                    self.undo.push(edit);
                    self.redo.clear();
                    self.selected.clear();
                    self.dirty = true;
                    self.record_recovery_edit();
                    self.renderer.invalidate();
                    self.status = "Linked instance deleted. Undo restores the link.".into();
                }
                Err(error) => self.status = error,
            }
            return;
        }
        if self
            .selected
            .iter()
            .any(|id| self.doc.has_linked_source(*id))
        {
            self.status = "A linked source cannot be deleted while instances use it. Delete or make unique its instances first.".into();
            return;
        }
        let before = editing::snapshot(&self.doc, &self.selected);
        if before.is_empty() {
            return;
        }
        let edit = Edit::Change {
            before,
            after: Vec::new(),
        };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.selected.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.status = "Deleted selection".into();
    }
    pub(super) fn numeric_transform(&mut self) {
        self.finish();
        if self
            .selected
            .iter()
            .any(|id| self.doc.linked_instance_for_stroke(*id).is_some())
        {
            self.status = "Use the linked instance controls to transform this copy.".into();
            return;
        }
        let before = editing::snapshot(&self.doc, &self.selected);
        let Some((min, max)) = editing::bounds(&self.doc, &self.selected) else {
            return;
        };
        let mut translation = Vec3::ZERO;
        let mut rotation = Quat::IDENTITY;
        let mut scale = Vec3::ONE;
        match self.transform_mode {
            Mode::Move => translation = Vec3::from(self.transform_values),
            Mode::Rotate => {
                rotation = Quat::from_euler(
                    glam::EulerRot::ZYX,
                    self.transform_values[2].to_radians(),
                    self.transform_values[1].to_radians(),
                    self.transform_values[0].to_radians(),
                );
            }
            Mode::Scale => {
                scale = Vec3::from(self.scale_values);
                if self.scale_uniform_lock {
                    scale = Vec3::splat(scale.x);
                }
            }
        }
        let pivot = self.transform_pivot_value((min + max) * 0.5);
        match editing::transformed_with_scale(
            &before,
            pivot,
            translation,
            rotation,
            scale,
            Quat::IDENTITY,
            self.scale_brush_width,
        ) {
            Ok(after) if after != before => {
                let edit = Edit::Change { before, after };
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.renderer
                    .invalidate_ids(&self.doc.linked_affected_stroke_ids(&self.selected));
                self.dirty = true;
                self.record_recovery_edit();
                self.transform_values = [0.0; 3];
                self.scale_values = [1.0; 3];
                self.status = "Transform applied".into();
            }
            Ok(_) => {}
            Err(error) => self.status = error.into(),
        }
    }
    pub(super) fn selection_shortcuts(&mut self, ctx: &egui::Context) {
        if self.mixed_resource_preview_active() {
            if ctx.input(|input| input.key_pressed(Key::Escape)) {
                self.finish_mixed_resource_move(false);
            }
            return;
        }
        let (mods, v, g, r, s, a, d, delete, escape) = ctx.input(|i| {
            (
                i.modifiers,
                i.key_pressed(Key::V),
                i.key_pressed(Key::G),
                i.key_pressed(Key::R),
                i.key_pressed(Key::S),
                i.key_pressed(Key::A),
                i.key_pressed(Key::D),
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Escape),
            )
        });
        if self.appearance_draft.is_some() {
            if escape {
                self.finish_appearance_edit(false);
            }
            return;
        }
        if mods.command {
            if a {
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.finish();
                self.select_mode = true;
                self.selected = self
                    .doc
                    .strokes
                    .iter()
                    .filter(|s| self.doc.editable(s))
                    .map(|s| s.id)
                    .collect();
            }
            if d && self.select_mode {
                self.duplicate_selected();
            }
        } else {
            if v {
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.finish();
                self.select_mode = true;
                self.eraser = false;
            }
            if self.select_mode {
                let mode = if g {
                    Some(Mode::Move)
                } else if r {
                    Some(Mode::Rotate)
                } else if s {
                    Some(Mode::Scale)
                } else {
                    None
                };
                if let Some(mode) = mode {
                    self.finish();
                    self.transform_mode = mode;
                    self.transform_values = [0.0; 3];
                }
                if delete {
                    self.delete_selected();
                }
                if escape {
                    if self.brush_select_drag.is_some() {
                        self.cancel_brush_selection();
                    } else if self.drag.is_some() {
                        self.cancel_transform();
                    } else {
                        self.selected.clear();
                    }
                }
            }
        }
    }
    pub(super) fn split_ordinary_stroke_after(&mut self, stroke_id: u64, point_index: usize) {
        match crate::point_edit::split_stroke(&self.doc, stroke_id, point_index, self.next_id) {
            Ok(plan) => {
                plan.edit.apply(&mut self.doc, true);
                self.undo.push(plan.edit);
                self.redo.clear();
                self.next_id = plan.next_id;
                self.selected = HashSet::from([plan.first_id, plan.second_id]);
                self.renderer.invalidate();
                self.dirty = true;
                self.record_recovery_edit();
                self.status =
                    "Stroke split after the selected point. Undo restores the original.".into();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn join_ordinary_strokes(&mut self, first_id: u64, second_id: u64, tolerance: f32) {
        match crate::point_edit::join_strokes(&self.doc, first_id, second_id, tolerance) {
            Ok(edit) => {
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.selected = HashSet::from([first_id]);
                self.renderer.invalidate();
                self.dirty = true;
                self.record_recovery_edit();
                self.status =
                    "Strokes joined in the chosen order. Undo restores both strokes.".into();
            }
            Err(error) => self.status = error,
        }
    }

    fn stroke_join_controls(&mut self, ui: &mut egui::Ui) {
        if self.selected.len() != 2
            || self.drag.is_some()
            || self.repeat.is_some()
            || self.curve_draft_active()
            || self.liquify_active()
        {
            return;
        }
        let ids: Vec<_> = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| self.selected.contains(&stroke.id))
            .map(|stroke| stroke.id)
            .collect();
        if ids.len() != 2 {
            return;
        }
        let state_id = ui.make_persistent_id("stroke-join-controls");
        let mut state = ui
            .ctx()
            .data_mut(|data| data.get_temp::<StrokeJoinUiState>(state_id))
            .filter(|state| state.ids == [ids[0], ids[1]])
            .unwrap_or(StrokeJoinUiState {
                ids: [ids[0], ids[1]],
                reverse_order: false,
                tolerance: 0.02,
            });
        ui.collapsing("Join strokes", |ui| {
            let (first_id, second_id) = if state.reverse_order {
                (state.ids[1], state.ids[0])
            } else {
                (state.ids[0], state.ids[1])
            };
            ui.label(format!("Order: #{first_id} → #{second_id}"));
            if ui.button("Swap stroke order").clicked() {
                state.reverse_order = !state.reverse_order;
            }
            ui.small("Connect the first stroke’s end to the second stroke’s start. A small gap remains visible as a connecting segment.");
            super::ui_style::numeric_row(
                ui,
                "Endpoint tolerance",
                "stroke-join-tolerance",
                &mut state.tolerance,
                Some(0.0..=crate::point_edit::MAX_JOIN_TOLERANCE),
                "wu",
            );
            if ui.button("Join selected strokes").clicked() {
                let (first_id, second_id) = if state.reverse_order {
                    (state.ids[1], state.ids[0])
                } else {
                    (state.ids[0], state.ids[1])
                };
                self.join_ordinary_strokes(first_id, second_id, state.tolerance);
            }
        });
        ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
    }

    fn point_edit_controls(&mut self, ui: &mut egui::Ui) {
        if self.selected.len() != 1
            || self.drag.is_some()
            || self.repeat.is_some()
            || self.curve_draft_active()
        {
            return;
        }
        let id = *self.selected.iter().next().unwrap();
        let Some(stroke) = self.doc.strokes.iter().find(|stroke| stroke.id == id) else {
            return;
        };
        if !self.doc.editable(stroke)
            || stroke.fill.is_some()
            || stroke.curve.is_some()
            || self.doc.has_linked_source(id)
            || self.doc.linked_instance_for_stroke(id).is_some()
            || stroke.points.is_empty()
        {
            return;
        }
        let points = &stroke.points;
        let state_id = ui.make_persistent_id(("stroke-point-edit", id));
        let mut state = ui
            .ctx()
            .data_mut(|data| data.get_temp::<(usize, [f32; 3], Vec3)>(state_id))
            .unwrap_or((0, points[0].position.to_array(), points[0].position));
        state.0 = state.0.min(points.len() - 1);
        if points[state.0].position != state.2 {
            state.1 = points[state.0].position.to_array();
            state.2 = points[state.0].position;
        }
        let mut move_point = false;
        let mut delete_point = false;
        let mut split_after_point = false;
        let mut canvas_edit = false;
        let cleanup_state_id = ui.make_persistent_id(("stroke-cleanup", id));
        let mut cleanup = ui
            .ctx()
            .data_mut(|data| data.get_temp::<(f32, f32)>(cleanup_state_id))
            .unwrap_or((0.2, 0.02));
        let mut preview_cleanup = false;
        ui.collapsing("Stroke points", |ui| {
            canvas_edit = ui.button("Edit points in canvas…").clicked();
            ui.small("Choose a sampled point (0-based). Coordinates are world units. Apply is one undo step.");
            ui.horizontal(|ui| {
                ui.label("Point");
                if ui.add(egui::DragValue::new(&mut state.0).range(0..=points.len()-1)).changed() {
                    state.1 = points[state.0].position.to_array();
                    state.2 = points[state.0].position;
                }
                ui.label(format!("of {}", points.len()));
            });
            for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                ui_style::numeric_row(ui, label, ("stroke-point-coordinate", axis), &mut state.1[axis], Some(-10000.0..=10000.0), "wu");
            }
            ui.horizontal(|ui| {
                move_point = ui.button("Apply point position").clicked();
                if ui.button("Reset").clicked() { state.1 = state.2.to_array(); }
            });
            delete_point = ui.add_enabled(points.len() > 2, egui::Button::new("Delete point")).clicked();
            split_after_point = ui
                .add_enabled(points.len() >= 4, egui::Button::new("Split after selected point"))
                .clicked();
        });
        ui.collapsing("Resample and simplify", |ui| {
            ui.small("Uniform world-space spacing and a maximum path error. Sharp corners and significant pressure/tilt changes are retained. Timed Airbrush marks stay unchanged because their timestamps drive deposited coverage.");
            ui.horizontal(|ui| {
                ui.label("Spacing");
                ui.add(
                    egui::DragValue::new(&mut cleanup.0)
                        .range(0.001..=1000.0)
                        .speed(0.01)
                        .suffix(" wu"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Max error");
                ui.add(
                    egui::DragValue::new(&mut cleanup.1)
                        .range(0.0..=100.0)
                        .speed(0.005)
                        .suffix(" wu"),
                );
            });
            ui.small("Gradient strokes are rejected if cleanup changes arc length by more than 1%. Dab seeds and settings are retained.");
            preview_cleanup = ui.button("Preview cleanup…").clicked();
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(cleanup_state_id, cleanup));
        ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
        if preview_cleanup {
            self.preview_selected_cleanup(cleanup.0, cleanup.1);
            return;
        }
        if canvas_edit {
            self.begin_canvas_point_edit();
            return;
        }
        if move_point || delete_point {
            let position = if delete_point {
                None
            } else {
                Some(Vec3::from_array(state.1))
            };
            match crate::point_edit::edit_point(&self.doc, id, state.0, position) {
                Ok(edit) => {
                    edit.apply(&mut self.doc, true);
                    self.undo.push(edit);
                    self.redo.clear();
                    self.renderer.invalidate_ids(&self.selected);
                    self.dirty = true;
                    self.record_recovery_edit();
                    ui.ctx()
                        .data_mut(|data| data.remove::<(usize, [f32; 3], Vec3)>(state_id));
                    self.status = "Stroke point updated. Undo restores the original stroke.".into();
                }
                Err(error) => self.status = error,
            }
        }
        if split_after_point {
            ui.ctx()
                .data_mut(|data| data.remove::<(usize, [f32; 3], Vec3)>(state_id));
            self.split_ordinary_stroke_after(id, state.0);
        }
    }

    pub(super) fn selection_panel(&mut self, ui: &mut egui::Ui) {
        self.sync_canvas_point_edit();
        if self.point_draft.is_some() {
            self.canvas_point_controls(ui);
            return;
        }
        ui.label(format!("{} stroke(s) selected", self.selected.len()));
        self.liquify_controls(ui);
        if self.liquify_active() {
            return;
        }
        if self.appearance_draft.is_none() {
            self.point_edit_controls(ui);
            self.stroke_modifier_controls(ui);
            self.stroke_join_controls(ui);
            if ui
                .add_enabled(
                    !self.selected.is_empty(),
                    egui::Button::new("Edit appearance…"),
                )
                .clicked()
            {
                self.begin_appearance_edit();
            }
        } else {
            let mut changed = false;
            let mut apply = false;
            let mut cancel = false;
            ui.group(|ui| {
                ui.strong("EDIT APPEARANCE");
                if let Some(draft) = self.appearance_draft.as_mut() {
                    let contains_fill = draft.before.iter().any(|(_, stroke)| stroke.fill.is_some());
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut draft.set_color, "Color").changed();
                        ui.small(if draft.mixed_color { "Mixed; unchanged" } else { "Same" });
                    });
                    if draft.set_color {
                        let mut display_color = draft.color.map(crate::presentation::linear_to_srgb);
                        if ui.color_edit_button_rgb(&mut display_color).changed() {
                            draft.color = display_color.map(crate::presentation::srgb_to_linear);
                            changed = true;
                        }
                    }
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add_enabled(
                                !contains_fill,
                                egui::Checkbox::new(&mut draft.set_gradient, "Color gradient"),
                            )
                            .changed();
                        ui.small(if draft.mixed_gradient {
                            "Mixed; unchanged"
                        } else {
                            "Same"
                        });
                    });
                    if draft.set_gradient && !contains_fill {
                        changed |= ui
                            .checkbox(&mut draft.gradient_enabled, "Use two-stop gradient")
                            .changed();
                        if draft.gradient_enabled {
                            ui.label("Start stop");
                            let mut start = draft
                                .gradient_start
                                .map(crate::presentation::linear_to_srgb);
                            if ui.color_edit_button_rgb(&mut start).changed() {
                                draft.gradient_start =
                                    start.map(crate::presentation::srgb_to_linear);
                                changed = true;
                            }
                            ui.label("End stop");
                            let mut end = draft
                                .gradient_end
                                .map(crate::presentation::linear_to_srgb);
                            if ui.color_edit_button_rgb(&mut end).changed() {
                                draft.gradient_end =
                                    end.map(crate::presentation::srgb_to_linear);
                                changed = true;
                            }
                            ui.small("Stops interpolate in linear RGB along stroke length.");
                        }
                    }
                    if draft.set_color || draft.set_gradient {
                        ui.small("Applying one color clears a gradient. Gradient stops take precedence when both are enabled.");
                    }
                    ui.horizontal(|ui| {
                        changed |= ui.add_enabled(!contains_fill, egui::Checkbox::new(&mut draft.set_radius, "Radius")).changed();
                        ui.small(if draft.mixed_radius { "Mixed; unchanged" } else { "Same" });
                    });
                    if draft.set_radius && !contains_fill { changed |= ui.add(egui::Slider::new(&mut draft.radius, 0.001..=1.0).text("wu")).changed(); }
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut draft.set_opacity, "Opacity").changed();
                        ui.small(if draft.mixed_opacity { "Mixed; unchanged" } else { "Same" });
                    });
                    if draft.set_opacity { changed |= ui.add(egui::Slider::new(&mut draft.opacity, 0.0..=1.0).text("Opacity")).changed(); }
                    ui.horizontal(|ui| {
                        changed |= ui.add_enabled(!contains_fill, egui::Checkbox::new(&mut draft.set_brush_kind, "Brush type")).changed();
                        ui.small(if draft.mixed_brush_kind { "Mixed; unchanged" } else { "Same" });
                    });
                    if draft.set_brush_kind && !contains_fill {
                        egui::ComboBox::from_id_salt("appearance-brush-kind")
                            .selected_text(format!("{:?}", draft.brush_kind))
                            .show_ui(ui, |ui| {
                                for kind in [crate::document::BrushKind::Round, crate::document::BrushKind::Ribbon, crate::document::BrushKind::Chisel] {
                                    changed |= ui.selectable_value(&mut draft.brush_kind, kind, format!("{:?}", kind)).changed();
                                }
                            });
                    }
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut draft.set_material, "Material").changed();
                        ui.small(if draft.mixed_material { "Mixed; unchanged" } else { "Same" });
                    });
                    if draft.set_material {
                        egui::ComboBox::from_id_salt("appearance-stroke-material")
                            .selected_text(format!("{:?}", draft.material))
                            .show_ui(ui, |ui| {
                                for material in [crate::document::StrokeMaterial::Shaded, crate::document::StrokeMaterial::Unlit, crate::document::StrokeMaterial::Emissive, crate::document::StrokeMaterial::Cutout] {
                                    changed |= ui.selectable_value(&mut draft.material, material, format!("{:?}", material)).changed();
                                }
                            });
                        ui.small("Unlit and emissive strokes do not cast shadows. Emissive strokes alone feed glow. Cutout coverage erases prior color and glow to transparency; later strokes remain visible.");
                    }
                    if contains_fill {
                        ui.small("Filled marks keep their contour; radius and brush type cannot change while any fill is selected.");
                    }
                }
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if changed {
                self.refresh_appearance_preview();
            }
            if apply {
                self.finish_appearance_edit(true);
            }
            if cancel {
                self.finish_appearance_edit(false);
            }
            ui.small("Mixed or untouched values stay exact until you enable that property.");
            return;
        }
        ui.label("Click a stroke. Shift-click adds or removes one. Empty space clears.");
        ui.horizontal(|ui| {
            if ui.button("Select all").clicked() {
                self.finish();
                self.selected = self
                    .doc
                    .strokes
                    .iter()
                    .filter(|s| self.doc.editable(s))
                    .map(|s| s.id)
                    .collect();
            }
            if ui.button("Clear").clicked() {
                self.finish();
                self.selected.clear();
            }
        });
        ui.separator();
        ui.checkbox(&mut self.brush_select_enabled, "Drag brush selection");
        if self.brush_select_enabled {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.brush_select_deselect, false, "Select");
                ui.selectable_value(&mut self.brush_select_deselect, true, "Deselect");
            });
            ui.add(
                egui::Slider::new(&mut self.brush_select_radius, 1.0..=120.0).text("Radius (px)"),
            );
            ui.checkbox(&mut self.brush_select_through, "Select through occlusion");
            ui.small(
                "Hidden and locked layers are always skipped. Escape cancels an active sweep.",
            );
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_enabled_ui(!self.selected.is_empty(),|ui|{
            ui.horizontal(|ui|{for(mode,label)in[(Mode::Move,"Move [G]"),(Mode::Rotate,"Rotate [R]"),(Mode::Scale,"Scale [S]")]{if ui.selectable_label(self.transform_mode==mode,label).clicked(){self.finish();self.transform_mode=mode;self.transform_values=[0.0;3];}}});
            ui.horizontal(|ui| {
                ui.label("Pivot");
                egui::ComboBox::from_id_salt("transform-pivot")
                    .selected_text(match self.transform_pivot {
                        PivotMode::SelectionCenter => "Selection center",
                        PivotMode::WorldOrigin => "World origin",
                        PivotMode::Custom => "Custom",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.transform_pivot, PivotMode::SelectionCenter, "Selection center");
                        ui.selectable_value(&mut self.transform_pivot, PivotMode::WorldOrigin, "World origin");
                        ui.selectable_value(&mut self.transform_pivot, PivotMode::Custom, "Custom");
                    });
            });
            if self.transform_pivot == PivotMode::Custom {
                for (i, axis) in ["Pivot X", "Pivot Y", "Pivot Z"].iter().enumerate() {
                    ui_style::numeric_row(ui, axis, ("transform-custom-pivot", i), &mut self.custom_pivot[i], Some(-10000.0..=10000.0), "wu");
                }
            }
            ui.label(match self.transform_mode{Mode::Move=>"Drag an axis arrow, or the center square to move in the view plane.",Mode::Rotate=>"Drag a colored ring to rotate around its world axis. Orbit to expose other rings.",Mode::Scale=>"Drag the gold square for uniform scale, colored endpoints for one axis, or View for independent view-plane scale."});
            ui.add_space(4.0);
            if self.transform_mode==Mode::Scale {
                ui.checkbox(&mut self.scale_uniform_lock, "Lock proportions");
                ui.checkbox(&mut self.scale_brush_width, "Scale brush width");
                for (i, axis) in ["Scale X", "Scale Y", "Scale Z"].iter().enumerate() {
                    let response = ui_style::numeric_row(ui, axis, ("transform-scale", i), &mut self.scale_values[i], Some(0.01..=100.0), "×");
                    if self.scale_uniform_lock && response.changed() {
                        let value = self.scale_values[i];
                        self.scale_values = [value; 3];
                    }
                }
                ui.small("Proportion lock affects numeric fields and handles. Scaling stays positive; brush width follows the geometric mean of each stroke's transformed cross-section when enabled.");
            } else {
                let unit = if self.transform_mode == Mode::Rotate { "°" } else { "wu" };
                ui.label(if self.transform_mode==Mode::Rotate{"Rotation in degrees (X, then Y, then Z)"}else{"Offset in world units"});
                for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui_style::numeric_row(ui, axis, ("transform-axis", i), &mut self.transform_values[i], None, unit);
                }
            }
            if ui.button("Reset numeric values").clicked() {
                self.transform_values = [0.0; 3];
                self.scale_values = [1.0; 3];
            }
            if ui.button("Apply values").clicked(){self.numeric_transform();}
            ui.horizontal(|ui|{if ui.button("Duplicate").on_hover_text("Ctrl/Cmd+D").clicked(){self.duplicate_selected();}
if ui.button("Repeat…").on_hover_text("Create evenly spaced independent copies with a live preview").clicked(){self.begin_repeat();}
if ui.button("Delete").on_hover_text("Delete / Backspace").clicked(){self.delete_selected();}});
        });
        ui.label(RichText::new("One drag = one undo step. Escape cancels a drag.").small());
    }
    pub(super) fn selection_overlay(&self, ui: &egui::Ui, rect: Rect) {
        if self.point_draft.is_some() {
            self.canvas_point_overlay(ui, rect);
            return;
        }
        if !self.select_mode {
            return;
        }
        let painter = ui.painter().with_clip_rect(rect);
        if let Some(brush) = &self.brush_select_drag {
            let accent = self.ui_style_selection_color(255);
            for pair in brush.path.windows(2) {
                painter.line_segment([pair[0], pair[1]], egui::Stroke::new(2.0_f32, accent));
            }
            if let Some(last) = brush.path.last() {
                painter.circle_stroke(
                    *last,
                    self.brush_select_radius,
                    egui::Stroke::new(1.5_f32, self.ui_style_selection_color(255)),
                );
            }
        }
        if let Some((min, max)) = editing::bounds(&self.doc, &self.selected) {
            let corners: [Vec3; 8] = std::array::from_fn(|i| {
                Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                )
            });
            paint_selection_box(
                &painter,
                &self.camera,
                rect,
                &corners,
                self.ui_style_selection_color(110),
            );
            if let Some(layout) = self.layout(rect) {
                layout.paint(&painter, self.drag.as_ref().map(|d| d.kind));
            }
        }
        if let Some(corners) = self.selected_obj_bounds() {
            paint_selection_box(
                &painter,
                &self.camera,
                rect,
                &corners,
                self.ui_style_selection_color(220),
            );
        }
    }

    fn ui_style_selection_color(&self, alpha: u8) -> Color32 {
        super::ui_style::canvas_selection_accent(
            self.presentation.preferences.background,
            self.presentation.preferences.background_image.is_some(),
        )
        .gamma_multiply(alpha as f32 / 255.0)
    }
}

fn selected_snap_points(
    snapshot: &Snapshot,
    target: super::guide_resource_ui::PlanarTarget,
) -> Vec<Vec3> {
    const MAX_POINTS: usize = 4096;
    let origin = target.plane.origin();
    let normal = target.plane.basis().2;
    let on_plane = |point: Vec3| (point - origin).dot(normal).abs() <= 0.005;
    let mut result = Vec::new();
    for (_, stroke) in snapshot {
        for point in [stroke.points.first(), stroke.points.last()]
            .into_iter()
            .flatten()
        {
            if on_plane(point.position) {
                result.push(point.position);
            }
        }
        for pair in stroke.points.windows(2) {
            if on_plane(pair[0].position) && on_plane(pair[1].position) {
                result.push(pair[0].position.lerp(pair[1].position, 0.5));
            }
            if result.len() >= MAX_POINTS {
                result.truncate(MAX_POINTS);
                return result;
            }
        }
        if result.len() >= MAX_POINTS {
            result.truncate(MAX_POINTS);
            return result;
        }
    }
    result
}

fn paint_selection_box(
    painter: &egui::Painter,
    camera: &crate::geometry::Camera,
    rect: Rect,
    corners: &[Vec3; 8],
    color: Color32,
) {
    for i in 0..8 {
        for bit in [1, 2, 4] {
            let j = i ^ bit;
            if j > i {
                if let (Some((a, _)), Some((b, _))) = (
                    gizmo::project(camera, rect, corners[i]),
                    gizmo::project(camera, rect, corners[j]),
                ) {
                    painter.line_segment([a, b], egui::Stroke::new(1.5_f32, color));
                }
            }
        }
    }
}

impl SketchApp {
    pub(super) fn prepare_capability_edit_erase(&mut self, rect: Rect) -> (Pos2, Pos2, Pos2) {
        self.doc = Document::default();
        self.selected.clear();
        self.undo.clear();
        self.redo.clear();
        self.next_id = 1;
        self.camera = Camera {
            projection: crate::geometry::Projection::Orthographic,
            distance: 5.0,
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        };
        self.guide = DrawingPlane::from_preset(Plane::Front);
        self.guide.depth = 0.0;
        self.stabilization = 0.0;
        self.radius = 0.035;
        self.brush = Brush::default();
        self.color = [0.25, 0.55, 0.85];
        self.select_mode = false;
        self.eraser = false;
        self.partial_eraser.enabled = false;
        self.line.enabled = false;
        self.shape_tool.mode = None;
        self.curve_tool.enabled = false;
        self.fills.enabled = false;

        let source_at = |t: f32| {
            Vec3::new(
                -1.8 + 3.6 * t,
                0.12 + 0.15 * (t * std::f32::consts::TAU).sin(),
                0.0,
            )
        };
        for index in 0..=48 {
            let t = index as f32 / 48.0;
            let position = gizmo::project(&self.camera, rect, source_at(t))
                .expect("edit-erase fixture samples project")
                .0;
            assert!(rect.contains(position));
            self.pointer_action(position, 0.8, rect, index == 0, false);
        }
        self.finish();
        assert_eq!(self.doc.strokes.len(), 1);
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        let selection_world = self.doc.strokes[0].points[24].position;
        let selection_point = gizmo::project(&self.camera, rect, selection_world)
            .expect("edit-erase stroke center projects")
            .0;
        let crossing_t = 0.52;
        let crossing = source_at(crossing_t);
        let start = gizmo::project(&self.camera, rect, crossing + Vec3::new(0.0, -0.55, 0.0))
            .expect("edit-erase gesture start projects")
            .0;
        let end = gizmo::project(&self.camera, rect, crossing + Vec3::new(0.0, 0.55, 0.0))
            .expect("edit-erase gesture end projects")
            .0;
        assert!(rect.contains(start) && rect.contains(end));
        self.status = "Select the stroke and change its appearance, then erase a segment.".into();
        self.renderer.invalidate();
        (selection_point, start, end)
    }

    pub(super) fn apply_capability_appearance_edit(&mut self, color: [f32; 3]) {
        self.begin_appearance_edit();
        let appearance = self
            .appearance_draft
            .as_mut()
            .expect("selected edit-erase stroke opens appearance editing");
        appearance.color = color;
        appearance.set_color = true;
        self.refresh_appearance_preview();
        self.finish_appearance_edit(true);
        assert_ne!(self.doc.strokes[0].color, [0.25, 0.55, 0.85]);
    }

    pub(super) fn verify_selection(&mut self, ctx: &egui::Context, rect: Rect) {
        fn replay(
            app: &mut SketchApp,
            ctx: &egui::Context,
            rect: Rect,
            start: Pos2,
            end: Option<Pos2>,
            shift: bool,
            release: bool,
        ) {
            let modifiers = egui::Modifiers {
                shift,
                ..Default::default()
            };
            let mut events = vec![egui::Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            }];
            if let Some(end) = end {
                events.push(egui::Event::PointerMoved(end));
            }
            if release {
                events.push(egui::Event::PointerButton {
                    pos: end.unwrap_or(start),
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers,
                });
            }
            ctx.input_mut(|i| i.events = events);
            app.canvas_input(ctx, rect, Vec::new());
            ctx.input_mut(|i| i.events.clear());
        }
        fn replay_pen(
            app: &mut SketchApp,
            ctx: &egui::Context,
            rect: Rect,
            start: Pos2,
            end: Pos2,
        ) {
            let scale = ctx.pixels_per_point();
            for (position, phase) in [(start, Phase::Down), (end, Phase::Move), (end, Phase::Up)] {
                ctx.input_mut(|input| input.events.clear());
                app.canvas_input(
                    ctx,
                    rect,
                    vec![PenSample {
                        position: egui::pos2(position.x * scale, position.y * scale),
                        pressure: Some(0.65),
                        tilt: [None, None],
                        eraser: false,
                        barrel: false,
                        contact: true,
                        phase,
                    }],
                );
            }
            ctx.input_mut(|input| input.events.clear());
        }
        fn verify_partial_eraser_input(app: &mut SketchApp, ctx: &egui::Context, rect: Rect) {
            let original_doc = app.doc.clone();
            let original_selection = app.selected.clone();
            let original_camera = app.camera;
            let original_undo = app.undo.clone();
            let original_redo = app.redo.clone();
            let original_next_id = app.next_id;
            let original_dirty = app.dirty;
            let original_eraser = app.eraser;
            let original_select_mode = app.select_mode;
            let original_enabled = app.partial_eraser.enabled;
            let original_radius = app.partial_eraser.radius;
            let original_last_pen = app.last_pen;
            let original_last_pen_at = app.last_pen_at;
            let original_pen_previous = app.pen_previous;
            let original_mouse_down = app.mouse_down;
            let original_status = app.status.clone();
            let source = Stroke {
                curve: None,
                material: Default::default(),
                point_thickness: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                id: 1,
                color: [0.3, 0.65, 0.8],
                gradient: None,
                radius: 0.02,
                brush: Default::default(),
                points: (0..=32)
                    .map(|index| {
                        let t = index as f32 / 32.0;
                        Point {
                            position: Vec3::new(-2.0 + 4.0 * t, 0.0, 0.0),
                            pressure: 0.2 + 0.8 * t,
                            tilt: Some(Vec3::new(0.0, 0.25, 1.0).normalize()),
                            time_ms: 0,
                        }
                    })
                    .collect(),
            };
            app.doc = Document {
                linked_stroke_groups: Default::default(),
                strokes: vec![source.clone()],
                ..Document::default()
            };
            app.selected.clear();
            app.undo.clear();
            app.redo.clear();
            app.next_id = 2;
            app.camera = Camera {
                projection: crate::geometry::Projection::Perspective,
                yaw: 0.0,
                pitch: 0.0,
                ..Camera::default()
            };
            app.eraser = true;
            app.select_mode = false;
            app.partial_eraser.enabled = true;
            app.partial_eraser.radius = 8.0;
            let original_strokes = app.doc.strokes.clone();
            let start = gizmo::project(&app.camera, rect, Vec3::new(0.0, -0.45, 0.0))
                .unwrap()
                .0;
            let end = gizmo::project(&app.camera, rect, Vec3::new(0.0, 0.45, 0.0))
                .unwrap()
                .0;

            replay(app, ctx, rect, start, Some(end), false, false);
            assert!(
                app.partial_erase_active(),
                "mouse drag starts an erase preview"
            );
            assert_ne!(app.doc.strokes, original_strokes);
            ctx.input_mut(|input| {
                input.events = vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }]
            });
            app.partial_eraser_shortcuts(ctx);
            ctx.input_mut(|input| {
                input.events = vec![egui::Event::PointerButton {
                    pos: end,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }]
            });
            app.canvas_input(ctx, rect, Vec::new());
            ctx.input_mut(|input| input.events.clear());
            assert_eq!(app.doc.strokes, original_strokes);
            assert!(
                app.undo.is_empty(),
                "Escape cancellation creates no undo step"
            );

            replay(app, ctx, rect, start, Some(end), false, true);
            assert_eq!(
                app.doc.strokes.len(),
                2,
                "mouse release keeps both fragments"
            );
            assert_eq!(app.undo.len(), 1, "one mouse release creates one undo step");
            let mouse_after = app.doc.strokes.clone();
            assert_eq!(mouse_after[0].id, source.id);
            assert_ne!(mouse_after[0].id, mouse_after[1].id);
            assert!(
                mouse_after[0].points.last().unwrap().position.x
                    < mouse_after[1].points.first().unwrap().position.x
            );
            app.undo();
            assert_eq!(app.doc.strokes, original_strokes);
            app.redo();
            assert_eq!(app.doc.strokes, mouse_after);

            app.undo.clear();
            app.redo.clear();
            app.doc.strokes = original_strokes.clone();
            app.next_id = 2;
            replay_pen(app, ctx, rect, start, end);
            assert_eq!(app.doc.strokes.len(), 2, "pen release keeps both fragments");
            assert_eq!(app.undo.len(), 1, "one pen release creates one undo step");
            let pen_after = app.doc.strokes.clone();
            assert!(
                pen_after[0].points.last().unwrap().position.x
                    < pen_after[1].points.first().unwrap().position.x
            );
            app.undo();
            assert_eq!(app.doc.strokes, original_strokes);
            app.redo();
            assert_eq!(app.doc.strokes, pen_after);

            app.doc = original_doc;
            app.selected = original_selection;
            app.camera = original_camera;
            app.undo = original_undo;
            app.redo = original_redo;
            app.next_id = original_next_id;
            app.dirty = original_dirty;
            app.eraser = original_eraser;
            app.select_mode = original_select_mode;
            app.partial_eraser.enabled = original_enabled;
            app.partial_eraser.radius = original_radius;
            app.last_pen = original_last_pen;
            app.last_pen_at = original_last_pen_at;
            app.pen_previous = original_pen_previous;
            app.mouse_down = original_mouse_down;
            app.status = original_status;
            app.renderer.invalidate();
        }
        fn verify_mouse_pen_brush_selection(app: &mut SketchApp, ctx: &egui::Context, rect: Rect) {
            let original_doc = app.doc.clone();
            let original_selection = app.selected.clone();
            let original_camera = app.camera;
            let original_undo = app.undo.len();
            let original_redo = app.redo.len();
            let original_last_pen = app.last_pen;
            let original_last_pen_at = app.last_pen_at;
            let original_pen_previous = app.pen_previous;
            let original_mouse_down = app.mouse_down;
            let original_select_mode = app.select_mode;
            let original_brush_select_enabled = app.brush_select_enabled;
            let original_brush_select_deselect = app.brush_select_deselect;
            let original_brush_select_radius = app.brush_select_radius;
            let original_brush_select_through = app.brush_select_through;
            let make_stroke = |id, y| Stroke {
                curve: None,
                material: Default::default(),
                point_thickness: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                id,
                color: [0.4, 0.7, 0.9],
                gradient: None,
                radius: 0.08,
                brush: Default::default(),
                points: [-0.8, 0.8]
                    .into_iter()
                    .map(|x| Point {
                        position: Vec3::new(x, y, 0.0),
                        pressure: 1.0,
                        tilt: None,
                        time_ms: 0,
                    })
                    .collect(),
            };
            app.doc = Document {
                linked_stroke_groups: Default::default(),
                strokes: vec![make_stroke(501, 0.8), make_stroke(502, -0.8)],
                ..Document::default()
            };
            app.camera = Camera {
                projection: crate::geometry::Projection::Perspective,
                distance: 5.0,
                yaw: 0.0,
                pitch: 0.0,
                ..Camera::default()
            };
            app.selected.clear();
            app.select_mode = true;
            app.eraser = false;
            app.brush_select_enabled = true;
            app.brush_select_deselect = false;
            app.brush_select_radius = 6.0;
            app.brush_select_through = false;
            let original_geometry = app.doc.strokes.clone();
            let first = app.doc.strokes[0].id;
            let second = app.doc.strokes[1].id;
            let first_start =
                gizmo::project(&app.camera, rect, app.doc.strokes[0].points[0].position)
                    .unwrap()
                    .0;
            let first_end =
                gizmo::project(&app.camera, rect, app.doc.strokes[0].points[1].position)
                    .unwrap()
                    .0;
            replay(app, ctx, rect, first_start, Some(first_end), false, true);
            assert_eq!(app.selected, HashSet::from([first]));
            app.brush_select_deselect = true;
            replay(app, ctx, rect, first_start, Some(first_end), false, true);
            assert!(app.selected.is_empty());

            app.camera.projection = crate::geometry::Projection::Orthographic;
            app.camera.distance = 30.0;
            let second_start =
                gizmo::project(&app.camera, rect, app.doc.strokes[1].points[0].position)
                    .unwrap()
                    .0;
            let second_end =
                gizmo::project(&app.camera, rect, app.doc.strokes[1].points[1].position)
                    .unwrap()
                    .0;
            app.brush_select_deselect = false;
            replay_pen(app, ctx, rect, second_start, second_end);
            assert_eq!(app.selected, HashSet::from([second]));
            app.brush_select_deselect = true;
            replay_pen(app, ctx, rect, second_start, second_end);
            assert!(app.selected.is_empty());
            assert_eq!(app.doc.strokes, original_geometry);
            assert_eq!(app.undo.len(), original_undo);
            assert_eq!(app.redo.len(), original_redo);
            assert!(app.brush_select_drag.is_none());

            app.doc = original_doc;
            app.selected = original_selection;
            app.camera = original_camera;
            app.last_pen = original_last_pen;
            app.last_pen_at = original_last_pen_at;
            app.pen_previous = original_pen_previous;
            app.mouse_down = original_mouse_down;
            app.select_mode = original_select_mode;
            app.brush_select_enabled = original_brush_select_enabled;
            app.brush_select_deselect = original_brush_select_deselect;
            app.brush_select_radius = original_brush_select_radius;
            app.brush_select_through = original_brush_select_through;
            app.renderer.invalidate();
        }
        fn compare_scale_handle_and_numeric(
            app: &mut SketchApp,
            ctx: &egui::Context,
            rect: Rect,
            pivot_mode: PivotMode,
            custom_pivot: Vec3,
            axis: usize,
            factor: f32,
        ) {
            app.transform_mode = Mode::Scale;
            app.transform_pivot = pivot_mode;
            app.custom_pivot = custom_pivot;
            app.scale_uniform_lock = false;
            let original = app.doc.strokes.clone();
            let undo_depth = app.undo.len();
            let layout = app.layout(rect).unwrap();
            let handle = layout
                .handles
                .iter()
                .find(|handle| handle.kind == HandleKind::ScaleAxis(axis))
                .unwrap_or_else(|| panic!("scale axis {axis} handle is available"));
            let start = handle.path[1];
            let direction = (start - layout.origin).normalized();
            replay(
                app,
                ctx,
                rect,
                start,
                Some(start + direction * (factor.ln() * 100.0)),
                false,
                true,
            );
            assert_eq!(app.undo.len(), undo_depth + 1);
            let handle_scaled = app.doc.strokes.clone();
            app.undo();
            assert_eq!(app.doc.strokes, original);

            let mut factors = [1.0; 3];
            factors[axis] = factor;
            app.scale_values = factors;
            app.numeric_transform();
            let numeric_scaled = app.doc.strokes.clone();
            assert_eq!(app.undo.len(), undo_depth + 1);
            assert_eq!(handle_scaled.len(), numeric_scaled.len());
            for (handle_stroke, numeric_stroke) in handle_scaled.iter().zip(&numeric_scaled) {
                assert_eq!(handle_stroke.id, numeric_stroke.id);
                assert_eq!(handle_stroke.fill, numeric_stroke.fill);
                assert_eq!(handle_stroke.color, numeric_stroke.color);
                assert_eq!(handle_stroke.material, numeric_stroke.material);
                assert_eq!(handle_stroke.group_id, numeric_stroke.group_id);
                assert_eq!(handle_stroke.layer_id, numeric_stroke.layer_id);
                assert!((handle_stroke.radius - numeric_stroke.radius).abs() < 0.0001);
                assert_eq!(handle_stroke.brush.kind, numeric_stroke.brush.kind);
                assert_eq!(handle_stroke.brush.dynamics, numeric_stroke.brush.dynamics);
                assert_eq!(
                    handle_stroke.brush.texture.kind,
                    numeric_stroke.brush.texture.kind
                );
                assert_eq!(
                    handle_stroke.brush.texture.seed,
                    numeric_stroke.brush.texture.seed
                );
                assert!(
                    (handle_stroke.brush.thickness - numeric_stroke.brush.thickness).abs() < 0.0001
                );
                assert!(
                    (handle_stroke.brush.opacity - numeric_stroke.brush.opacity).abs() < 0.0001
                );
                assert!(
                    (handle_stroke.brush.softness - numeric_stroke.brush.softness).abs() < 0.0001
                );
                assert!(
                    handle_stroke
                        .brush
                        .normal
                        .distance(numeric_stroke.brush.normal)
                        < 0.0001
                );
                assert!(
                    handle_stroke
                        .brush
                        .tangent
                        .distance(numeric_stroke.brush.tangent)
                        < 0.0001
                );
                assert!(
                    (handle_stroke.brush.texture.scale - numeric_stroke.brush.texture.scale).abs()
                        < 0.0001
                );
                assert_eq!(handle_stroke.points.len(), numeric_stroke.points.len());
                for (handle_point, numeric_point) in
                    handle_stroke.points.iter().zip(&numeric_stroke.points)
                {
                    assert!(
                        handle_point.position.distance(numeric_point.position) < 0.0001,
                        "numeric and axis {axis} handle scaling must agree around {pivot_mode:?}"
                    );
                    assert_eq!(handle_point.pressure, numeric_point.pressure);
                    assert_eq!(handle_point.tilt.is_some(), numeric_point.tilt.is_some());
                    if let (Some(handle_tilt), Some(numeric_tilt)) =
                        (handle_point.tilt, numeric_point.tilt)
                    {
                        assert!(handle_tilt.distance(numeric_tilt) < 0.0001);
                    }
                }
            }
            app.undo();
            assert_eq!(app.doc.strokes, original);
            app.redo();
            assert_eq!(app.doc.strokes, numeric_scaled);
            app.undo();
            assert_eq!(app.doc.strokes, original);
        }
        verify_partial_eraser_input(self, ctx, rect);
        verify_mouse_pen_brush_selection(self, ctx, rect);
        self.select_mode = true;
        self.eraser = false;
        self.selected.clear();
        let first = self.doc.strokes[0].id;
        let second = self.doc.strokes[1].id;
        let first_pos = gizmo::project(&self.camera, rect, self.doc.strokes[0].points[20].position)
            .unwrap()
            .0;
        let second_pos =
            gizmo::project(&self.camera, rect, self.doc.strokes[1].points[20].position)
                .unwrap()
                .0;
        replay(self, ctx, rect, first_pos, None, false, true);
        assert_eq!(self.selected, HashSet::from([first]));
        replay(self, ctx, rect, second_pos, None, true, true);
        assert_eq!(self.selected, HashSet::from([first, second]));
        replay(self, ctx, rect, second_pos, None, true, true);
        assert_eq!(self.selected, HashSet::from([first]));
        replay(self, ctx, rect, second_pos, None, true, true);
        let original = self.doc.strokes.clone();
        self.duplicate_selected();
        let copies = self.selected.clone();
        assert_eq!(self.doc.strokes.len(), 5);
        assert_eq!(copies.len(), 2);
        assert!(!copies.contains(&first));
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.redo();
        assert_eq!(self.doc.strokes.len(), 5);
        self.selected = copies;
        self.delete_selected();
        assert_eq!(self.doc.strokes, original);
        self.undo();
        assert_eq!(self.doc.strokes.len(), 5);
        self.redo();
        assert_eq!(self.doc.strokes, original);
        self.selected.clear();
        self.transform_mode = Mode::Move;
        replay(self, ctx, rect, first_pos, None, false, true);
        let before = self.doc.strokes.clone();
        let layout = self.layout(rect).unwrap();
        let click = layout
            .handles
            .iter()
            .find(|h| h.kind == HandleKind::Axis(0))
            .unwrap()
            .path[1];
        let click_steps = self.undo.len();
        replay(self, ctx, rect, click, None, false, true);
        assert_eq!(
            self.doc.strokes, before,
            "Clicking a handle must not change geometry"
        );
        assert_eq!(
            self.undo.len(),
            click_steps,
            "Clicking a handle must not create an undo entry"
        );
        let steps = self.undo.len();
        let layout = self.layout(rect).unwrap();
        let start = layout
            .handles
            .iter()
            .find(|h| h.kind == HandleKind::Axis(0))
            .unwrap()
            .path[1];
        replay(
            self,
            ctx,
            rect,
            start,
            Some(start + egui::vec2(35.0, 0.0)),
            false,
            true,
        );
        assert_eq!(
            self.undo.len(),
            steps + 1,
            "One gesture must add one undo entry"
        );
        let moved = self.doc.strokes.clone();
        assert_ne!(moved, before);
        let delta = moved[0].points[0].position - before[0].points[0].position;
        assert!(
            delta.x.abs() > 0.01 && delta.y.abs() < 0.0001 && delta.z.abs() < 0.0001,
            "X handle must constrain translation"
        );
        self.undo();
        assert_eq!(self.doc.strokes, before);
        self.redo();
        assert_eq!(self.doc.strokes, moved);
        let pivot = self.layout(rect).unwrap().origin;
        let steps = self.undo.len();
        replay(
            self,
            ctx,
            rect,
            pivot,
            Some(pivot + egui::vec2(30.0, 20.0)),
            false,
            false,
        );
        assert_ne!(self.doc.strokes, moved);
        ctx.input_mut(|i| {
            i.events = vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        });
        self.selection_shortcuts(ctx);
        ctx.input_mut(|i| i.events.clear());
        assert_eq!(
            self.doc.strokes, moved,
            "Escape must restore the original gesture snapshot"
        );
        assert_eq!(
            self.undo.len(),
            steps,
            "Cancelled gesture must not enter undo history"
        );
        self.transform_mode = Mode::Rotate;
        let layout = self.layout(rect).unwrap();
        let ring = layout
            .handles
            .iter()
            .find(|h| h.kind == HandleKind::Rotation(2))
            .unwrap();
        let index = (0..64)
            .find(|i| layout.hit(ring.path[*i]) == Some(HandleKind::Rotation(2)))
            .unwrap();
        let start = ring.path[index];
        let end = ring.path[(index + 13) % 64];
        let pivot = layout.pivot;
        replay(self, ctx, rect, start, Some(end), false, true);
        let rotated = self.doc.strokes.clone();
        assert_ne!(rotated, moved);
        assert!(
            (rotated[0].points[0].position.distance(pivot)
                - moved[0].points[0].position.distance(pivot))
            .abs()
                < 0.0001
        );
        self.undo();
        assert_eq!(self.doc.strokes, moved);
        self.redo();
        assert_eq!(self.doc.strokes, rotated);
        let custom_pivot = Vec3::new(0.35, -0.2, 0.4);
        for (pivot_mode, pivot) in [
            (PivotMode::SelectionCenter, Vec3::ZERO),
            (PivotMode::WorldOrigin, Vec3::ZERO),
            (PivotMode::Custom, custom_pivot),
        ] {
            for axis in 0..3 {
                compare_scale_handle_and_numeric(self, ctx, rect, pivot_mode, pivot, axis, 1.35);
            }
        }
        self.transform_mode = Mode::Scale;
        self.transform_pivot = PivotMode::Custom;
        self.custom_pivot = custom_pivot;
        self.scale_uniform_lock = false;
        let before_cancel = self.doc.strokes.clone();
        let cancel_history = self.undo.len();
        let layout = self.layout(rect).unwrap();
        let handle = layout
            .handles
            .iter()
            .find(|handle| handle.kind == HandleKind::ScaleAxis(1))
            .expect("Y scale handle is available for cancellation");
        let start = handle.path[1];
        let direction = (start - layout.origin).normalized();
        replay(
            self,
            ctx,
            rect,
            start,
            Some(start + direction * (0.4_f32.ln() * 100.0)),
            false,
            false,
        );
        assert!(self.drag.is_some(), "scale preview remains cancellable");
        assert_ne!(self.doc.strokes, before_cancel);
        assert_eq!(self.undo.len(), cancel_history);
        ctx.input_mut(|input| {
            input.events = vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        });
        self.selection_shortcuts(ctx);
        ctx.input_mut(|input| input.events.clear());
        assert!(self.drag.is_none());
        assert_eq!(self.doc.strokes, before_cancel);
        assert_eq!(self.undo.len(), cancel_history);
        self.transform_mode = Mode::Scale;
        self.transform_pivot = PivotMode::Custom;
        self.custom_pivot = custom_pivot;
        self.scale_uniform_lock = false;
        let before_view_scale = self.doc.strokes.clone();
        let view_scale_history = self.undo.len();
        let layout = self.layout(rect).unwrap();
        let handle = layout
            .handles
            .iter()
            .find(|handle| handle.kind == HandleKind::ScaleView)
            .expect("view-plane scale handle is available");
        let start = *handle.path.last().unwrap();
        let scale_x = 1.3_f32;
        let scale_y = 0.75_f32;
        replay(
            self,
            ctx,
            rect,
            start,
            Some(start + egui::vec2(scale_x.ln() * 100.0, -scale_y.ln() * 100.0)),
            false,
            true,
        );
        let view_scaled = self.doc.strokes.clone();
        assert_ne!(view_scaled, before_view_scale);
        assert_eq!(self.undo.len(), view_scale_history + 1);
        let (view_right, view_up, view_forward) = self.camera.basis();
        let view_basis = Quat::from_mat3(&Mat3::from_cols(view_right, view_up, view_forward));
        let pivot = self.custom_pivot;
        for (before_stroke, after_stroke) in before_view_scale.iter().zip(&view_scaled) {
            if !self.selected.contains(&before_stroke.id) {
                assert_eq!(before_stroke, after_stroke);
                continue;
            }
            for (before_point, after_point) in before_stroke.points.iter().zip(&after_stroke.points)
            {
                let before_local = view_basis.inverse() * (before_point.position - pivot);
                let after_local = view_basis.inverse() * (after_point.position - pivot);
                assert!((after_local.x - before_local.x * scale_x).abs() < 0.0001);
                assert!((after_local.y - before_local.y * scale_y).abs() < 0.0001);
                assert!((after_local.z - before_local.z).abs() < 0.0001);
            }
        }
        let picked_scaled_stroke = view_scaled
            .iter()
            .find(|stroke| self.selected.contains(&stroke.id))
            .expect("a scaled stroke remains selected");
        assert!(
            picked_scaled_stroke
                .points
                .iter()
                .filter_map(|point| gizmo::project(&self.camera, rect, point.position))
                .any(|(position, _)| {
                    gizmo::pick(&self.doc, &self.camera, rect, position)
                        == Some(picked_scaled_stroke.id)
                }),
            "the transformed stroke remains pickable at its new geometry"
        );
        let scaled_bounds = editing::bounds(&self.doc, &self.selected)
            .expect("scaled selection bounds are available");
        let original_bounds = editing::bounds(
            &Document {
                linked_stroke_groups: Default::default(),
                strokes: before_view_scale.clone(),
                ..Document::default()
            },
            &self.selected,
        )
        .expect("original selection bounds are available");
        assert_ne!(scaled_bounds, original_bounds);
        self.transform_pivot = PivotMode::SelectionCenter;
        let scaled_layout = self
            .layout(rect)
            .expect("scaled selection gizmo layout is available");
        assert!(
            scaled_layout
                .pivot
                .distance((scaled_bounds.0 + scaled_bounds.1) * 0.5)
                < 0.0001
        );
        self.transform_pivot = PivotMode::Custom;
        self.undo();
        assert_eq!(self.doc.strokes, before_view_scale);
        self.redo();
        assert_eq!(self.doc.strokes, view_scaled);
        self.undo();
        assert_eq!(self.doc.strokes, before_view_scale);
        self.transform_pivot = PivotMode::SelectionCenter;
        self.scale_values = [1.0; 3];
        let layout = self.layout(rect).unwrap();
        let start = layout.handles[0].path[1];
        replay(
            self,
            ctx,
            rect,
            start,
            Some(start + egui::vec2(12.0, -12.0)),
            false,
            true,
        );
        let scaled = self.doc.strokes.clone();
        assert!(scaled[0].radius > rotated[0].radius);
        assert_eq!(scaled[0].points[0].pressure, rotated[0].points[0].pressure);
        self.undo();
        assert_eq!(self.doc.strokes, rotated);
        self.redo();
        assert_eq!(self.doc.strokes, scaled);
        self.transform_mode = Mode::Move;
        self.transform_values = [0.1, 0.0, 0.0];
        self.numeric_transform();
        assert_ne!(self.doc.strokes, scaled);
        self.undo();
        assert_eq!(self.doc.strokes, scaled);
        self.redo();
        let moved = self.doc.strokes.clone();
        self.transform_mode = Mode::Scale;
        self.transform_pivot = PivotMode::Custom;
        self.custom_pivot = Vec3::new(-0.25, 0.3, 0.15);
        self.scale_uniform_lock = false;
        self.scale_values = [1.2, 0.8, 1.1];
        let steps = self.undo.len();
        self.numeric_transform();
        let non_uniform = self.doc.strokes.clone();
        assert_eq!(self.undo.len(), steps + 1);
        assert_ne!(non_uniform, moved);
        self.undo();
        assert_eq!(self.doc.strokes, moved);
        self.redo();
        assert_eq!(self.doc.strokes, non_uniform);
        let exported = crate::export::encode(&self.doc, None).unwrap().bytes;
        assert!(self.save(false));
        let loaded =
            Document::from_json(&std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap())
                .unwrap();
        assert_eq!(loaded.strokes, non_uniform);
        assert_eq!(
            crate::export::encode(&loaded, None).unwrap().bytes,
            exported,
            "native reload preserves non-uniform scale in the GLB export"
        );
        self.doc = loaded;
        self.renderer.invalidate();
        self.transform_mode = Mode::Scale;
        self.transform_pivot = PivotMode::Custom;
        self.custom_pivot = Vec3::new(-0.25, 0.3, 0.15);
        self.scale_uniform_lock = false;
        self.scale_values = [1.1, 1.0, 1.0];
        self.numeric_transform();
        let recovery_document = self.doc.strokes.clone();
        assert!(
            self.recovery.force_snapshot(&self.doc, self.file.clone()),
            "the scaled document should be eligible for a recovery snapshot"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.recovery.busy() && std::time::Instant::now() < deadline {
            self.recovery.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.recovery.poll();
        assert!(!self.recovery.busy(), "recovery write should finish");
        let recovery_path = self
            .recovery
            .last_snapshot_path()
            .expect("recovery snapshot path is recorded")
            .to_path_buf();
        let recovery_bytes = std::fs::read(&recovery_path).unwrap();
        let recovery_snapshot: crate::recovery_store::RecoverySnapshot =
            serde_json::from_slice(&recovery_bytes).unwrap();
        assert_eq!(recovery_snapshot.document.strokes, recovery_document);
        let recovery_scan_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let scan_copy = recovery_scan_dir
            .path()
            .join(recovery_path.file_name().unwrap());
        std::fs::write(&scan_copy, recovery_bytes).unwrap();
        let scan = crate::recovery_store::scan(
            recovery_scan_dir.path(),
            "selection-verifier-recovery-reader",
        )
        .unwrap();
        assert!(scan.issues.is_empty());
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(
            scan.candidates[0].snapshot.document.strokes,
            recovery_document
        );
        self.status="Selection and scaling checks passed: axes/pivots, cancel, undo/redo, native reload, GLB export and recovery scan.".into();
    }
}
