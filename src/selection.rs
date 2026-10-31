use super::*;
use crate::{
    editing::{self, Snapshot},
    gizmo::{self, HandleKind, Layout, Mode},
};
use glam::Quat;

pub(super) struct Drag {
    before: Snapshot,
    pivot: Vec3,
    kind: HandleKind,
    start: Pos2,
    view_normal: Vec3,
    translation: Vec3,
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
            self.renderer.invalidate_ids(&self.selected);
            self.status = "Transform cancelled".into();
        }
    }
    fn layout(&self, rect: Rect) -> Option<Layout> {
        if !self.select_mode {
            return None;
        }
        let (min, max) = editing::bounds(&self.doc, &self.selected)?;
        let pivot = self
            .drag
            .as_ref()
            .map(|d| d.pivot + d.translation)
            .unwrap_or((min + max) * 0.5);
        Layout::new(&self.camera, rect, pivot, self.transform_mode)
    }
    pub(super) fn select_pointer(&mut self, pos: Pos2, rect: Rect, begin: bool, shift: bool) {
        if begin {
            self.finish();
            if !shift {
                if let Some(layout) = self.layout(rect) {
                    if let Some(kind) = layout.hit(pos) {
                        self.drag = Some(Drag {
                            before: editing::snapshot(&self.doc, &self.selected),
                            pivot: layout.pivot,
                            kind,
                            start: pos,
                            view_normal: (self.camera.eye() - layout.pivot).normalize(),
                            translation: Vec3::ZERO,
                        });
                        return;
                    }
                }
            }
            let hit = gizmo::pick(&self.doc, &self.camera, rect, pos);
            if !shift {
                self.selected.clear();
            }
            if let Some(id) = hit {
                let ids = self.doc.selection_for_stroke(id, self.individual_selection);
                editing::toggle_selection(&mut self.selected, &ids, shift);
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
        let Some(drag) = self.drag.as_ref() else {
            return;
        };
        let mut translation = Vec3::ZERO;
        let mut rotation = Quat::IDENTITY;
        let mut scale = 1.0;
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
                let min = drag
                    .before
                    .iter()
                    .map(|(_, s)| 0.001 / s.radius)
                    .fold(0.01_f32, f32::max);
                let max = drag
                    .before
                    .iter()
                    .map(|(_, s)| 1.0 / s.radius)
                    .fold(100.0_f32, f32::min);
                scale = ((delta.x - delta.y) * 0.01).exp().clamp(min, max);
            }
        }
        match editing::transformed(&drag.before, drag.pivot, translation, rotation, scale) {
            Ok(after) => {
                editing::replace(&mut self.doc, &drag.before, &after, true);
                self.renderer.invalidate_ids(&self.selected);
                if let Some(drag) = self.drag.as_mut() {
                    drag.translation = translation;
                }
            }
            Err(error) => self.status = error.into(),
        }
    }
    pub(super) fn duplicate_selected(&mut self) {
        self.finish();
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
            self.renderer.set_repeat_preview(&[]);
            return Ok(Vec::new());
        };
        match self.repeated_strokes(repeat) {
            Ok(copies) => {
                let strokes: Vec<_> = copies.into_iter().map(|(_, stroke)| stroke).collect();
                self.renderer.set_repeat_preview(&strokes);
                Ok(strokes)
            }
            Err(error) => {
                self.renderer.set_repeat_preview(&[]);
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
        self.renderer.set_repeat_preview(&[]);
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
            ui.add(egui::Slider::new(&mut repeat.count, 1..=MAX_REPEAT_COPIES).text("Copies"));
            ui.horizontal(|ui| {
                for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                    ui.add(
                        egui::DragValue::new(&mut repeat.spacing[axis])
                            .speed(0.02)
                            .range(-1000.0..=1000.0)
                            .prefix(format!("{label} ")),
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut repeat.space, RepeatSpace::World, "World axes");
                ui.selectable_value(&mut repeat.space, RepeatSpace::Guide, "Guide axes");
            });
            match preview.as_ref().unwrap() {
                Ok(copies) => {
                    ui.label(format!("Previewing {} strokes", copies.len()));
                    ui.label("Copies remain individually editable; Apply is one undo step.");
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
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
                    assert_eq!(stroke.fill, source.fill);
                    assert_eq!(stroke.color, source.color);
                    assert_eq!(stroke.brush, source.brush);
                    assert_eq!(stroke.radius, source.radius);
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
        let before = editing::snapshot(&self.doc, &self.selected);
        let Some((min, max)) = editing::bounds(&self.doc, &self.selected) else {
            return;
        };
        let (translation, rotation, scale) = match self.transform_mode {
            Mode::Move => (Vec3::from(self.transform_values), Quat::IDENTITY, 1.0),
            Mode::Rotate => (
                Vec3::ZERO,
                Quat::from_euler(
                    glam::EulerRot::ZYX,
                    self.transform_values[2].to_radians(),
                    self.transform_values[1].to_radians(),
                    self.transform_values[0].to_radians(),
                ),
                1.0,
            ),
            Mode::Scale => (Vec3::ZERO, Quat::IDENTITY, self.scale_value),
        };
        match editing::transformed(&before, (min + max) * 0.5, translation, rotation, scale) {
            Ok(after) if after != before => {
                let edit = Edit::Change { before, after };
                edit.apply(&mut self.doc, true);
                self.undo.push(edit);
                self.redo.clear();
                self.renderer.invalidate_ids(&self.selected);
                self.dirty = true;
                self.record_recovery_edit();
                self.transform_values = [0.0; 3];
                self.scale_value = 1.0;
                self.status = "Transform applied".into();
            }
            Ok(_) => {}
            Err(error) => self.status = error.into(),
        }
    }
    pub(super) fn selection_shortcuts(&mut self, ctx: &egui::Context) {
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
                    if self.drag.is_some() {
                        self.cancel_transform();
                    } else {
                        self.selected.clear();
                    }
                }
            }
        }
    }
    pub(super) fn selection_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(format!("{} stroke(s) selected", self.selected.len()));
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
        ui.add_space(6.0);
        ui.separator();
        ui.add_enabled_ui(!self.selected.is_empty(),|ui|{
            ui.horizontal(|ui|{for(mode,label)in[(Mode::Move,"Move [G]"),(Mode::Rotate,"Rotate [R]"),(Mode::Scale,"Scale [S]")]{if ui.selectable_label(self.transform_mode==mode,label).clicked(){self.finish();self.transform_mode=mode;self.transform_values=[0.0;3];}}});
            ui.label(match self.transform_mode{Mode::Move=>"Drag an axis arrow, or the center square to move in the view plane.",Mode::Rotate=>"Drag a colored ring to rotate around its world axis. Orbit to expose other rings.",Mode::Scale=>"Drag the gold square to scale the group and its brush widths uniformly."});
            ui.add_space(4.0);
            if self.transform_mode==Mode::Scale{ui.add(egui::DragValue::new(&mut self.scale_value).speed(0.01).range(0.01..=100.0).prefix("Factor "));}
            else{ui.label(if self.transform_mode==Mode::Rotate{"Rotation in degrees (X, then Y, then Z)"}else{"Offset in world units"});ui.horizontal(|ui|{for(i,label)in["X ","Y ","Z "].iter().enumerate(){ui.add(egui::DragValue::new(&mut self.transform_values[i]).speed(if self.transform_mode==Mode::Rotate{0.5}else{0.02}).prefix(*label));}});}
            if ui.button("Apply values").clicked(){self.numeric_transform();}
            ui.horizontal(|ui|{if ui.button("Duplicate").on_hover_text("Ctrl/Cmd+D").clicked(){self.duplicate_selected();}
if ui.button("Repeat…").on_hover_text("Create evenly spaced independent copies with a live preview").clicked(){self.begin_repeat();}
if ui.button("Delete").on_hover_text("Delete / Backspace").clicked(){self.delete_selected();}});
        });
        ui.label(RichText::new("One drag = one undo step. Escape cancels a drag.").small());
    }
    pub(super) fn selection_overlay(&self, ui: &egui::Ui, rect: Rect) {
        if !self.select_mode {
            return;
        }
        let Some((min, max)) = editing::bounds(&self.doc, &self.selected) else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        let corners: [Vec3; 8] = std::array::from_fn(|i| {
            Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        });
        for i in 0..8 {
            for bit in [1, 2, 4] {
                let j = i ^ bit;
                if j > i {
                    if let (Some((a, _)), Some((b, _))) = (
                        gizmo::project(&self.camera, rect, corners[i]),
                        gizmo::project(&self.camera, rect, corners[j]),
                    ) {
                        painter.line_segment(
                            [a, b],
                            egui::Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(255, 201, 104, 110),
                            ),
                        );
                    }
                }
            }
        }
        if let Some(layout) = self.layout(rect) {
            layout.paint(&painter, self.drag.as_ref().map(|d| d.kind));
        }
    }
}

impl SketchApp {
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
        self.transform_mode = Mode::Scale;
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
        assert!(self.save(false));
        let loaded =
            Document::from_json(&std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap())
                .unwrap();
        assert_eq!(loaded.strokes, self.doc.strokes);
        self.doc = loaded;
        self.renderer.invalidate();
        self.status="Selection, handles, grouped undo, duplicate/delete, cancel, and persistence checks passed.".into();
    }
}
