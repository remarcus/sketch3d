//! Bounded edits to sampled strokes; semantic curves retain their own editor.
use crate::document::{Document, Stroke};
use crate::editing::{snapshot, Edit};
use glam::Vec3;
use std::collections::HashSet;

pub const MAX_CANVAS_POINTS: usize = 4096;
pub const MAX_JOIN_TOLERANCE: f32 = 100.0;
const JOIN_COLOR_EPSILON: f32 = 1.0e-4;
const JOIN_DISTANCE_EPSILON: f32 = 1.0e-5;

/// A detached preview. Saving, picking and history see committed geometry until Apply.
pub struct Draft {
    pub original: Stroke,
    pub preview: Stroke,
    pub selected: usize,
}

pub struct SplitPlan {
    pub edit: Edit,
    pub next_id: u64,
    pub first_id: u64,
    pub second_id: u64,
}

impl Draft {
    pub fn new(doc: &Document, id: u64) -> Result<Self, String> {
        let before = snapshot(doc, &HashSet::from([id]));
        let source = &before
            .first()
            .ok_or("Select an editable ordinary stroke.")?
            .1;
        if source.fill.is_some() || source.curve.is_some() || doc.has_linked_source(id) {
            return Err("Use the curve or group editor for this stroke.".into());
        }
        if !(2..=MAX_CANVAS_POINTS).contains(&source.points.len()) {
            return Err(format!(
                "Canvas point editing supports 2–{MAX_CANVAS_POINTS} samples per stroke."
            ));
        }
        Ok(Self {
            original: source.clone(),
            preview: source.clone(),
            selected: 0,
        })
    }

    pub fn valid_for(&self, doc: &Document) -> bool {
        doc.strokes
            .iter()
            .any(|stroke| stroke == &self.original && doc.editable(stroke))
            && doc.linked_instance_for_stroke(self.original.id).is_none()
            && !doc.has_linked_source(self.original.id)
    }

    pub fn move_selected(&mut self, position: Vec3) -> Result<(), String> {
        let mut point = *self
            .preview
            .points
            .get(self.selected)
            .ok_or("Choose a point first.")?;
        point.position = position;
        if !point.is_valid() {
            return Err("Point coordinates must be finite and within ±10000 world units.".into());
        }
        let previous = self.preview.points[self.selected];
        self.preview.points[self.selected] = point;
        if let Err(error) = validate_geometry(&self.preview) {
            self.preview.points[self.selected] = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn delete_selected(&mut self) -> Result<(), String> {
        if self.preview.points.len() <= 2 || self.selected >= self.preview.points.len() {
            return Err("Keep at least two points.".into());
        }
        self.preview.points.remove(self.selected);
        self.selected = self.selected.min(self.preview.points.len() - 1);
        Ok(())
    }

    pub fn edit(&self, doc: &Document) -> Result<Edit, String> {
        if !self.valid_for(doc) {
            return Err("The source stroke changed; reopen point editing.".into());
        }
        if self.original == self.preview {
            return Err("No point changes to apply.".into());
        }
        validate_geometry(&self.preview)?;
        let before = snapshot(doc, &HashSet::from([self.original.id]));
        let mut after = before.clone();
        after[0].1 = self.preview.clone();
        Ok(Edit::Change { before, after })
    }
}

fn validate_geometry(stroke: &Stroke) -> Result<(), String> {
    crate::dab_placement::point_count(stroke)?;
    if stroke.brush.flow.enabled
        && crate::geometry::airbrush_vertex_estimate(stroke)
            > crate::geometry::MAX_AIRBRUSH_VERTICES
    {
        return Err("Point edit exceeds the airbrush geometry budget.".into());
    }
    Ok(())
}

/// Split an ordinary sampled stroke after `point_index`. The shared sample is
/// retained by both pieces, so the pair covers the same centerline as the
/// original and keeps its captured pressure, tilt, and time values.
pub fn split_stroke(
    doc: &Document,
    stroke_id: u64,
    point_index: usize,
    next_id: u64,
) -> Result<SplitPlan, String> {
    let before = snapshot_for_point_operation(doc, &[stroke_id])?;
    let (index, source) = &before[0];
    if source.points.len() < 4 {
        return Err("A split needs at least four samples.".into());
    }
    if point_index == 0 || point_index >= source.points.len() - 1 {
        return Err("Choose an interior point so each piece keeps at least two samples.".into());
    }
    if next_id == 0 || next_id == u64::MAX || doc.strokes.iter().any(|stroke| stroke.id == next_id)
    {
        return Err("No available stroke ID for the split piece.".into());
    }
    if doc.strokes.len() >= 100_000 {
        return Err("Document stroke limit reached.".into());
    }
    let Some(after_next_id) = next_id.checked_add(1).filter(|id| *id < u64::MAX) else {
        return Err("No more stroke IDs available.".into());
    };

    let arc_lengths = crate::dynamics::arc_lengths(&source.points);
    let split_distance = arc_lengths[point_index];
    let total_length = *arc_lengths.last().unwrap_or(&0.0);
    let mut first = source.clone();
    let mut second = source.clone();
    first.points = source.points[..=point_index].to_vec();
    second.points = source.points[point_index..].to_vec();
    second.id = next_id;
    if let Some(gradient) = source.gradient {
        let t = if total_length > 1.0e-8 {
            (split_distance / total_length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut first_gradient = gradient.subrange(0.0, t);
        first_gradient.start = gradient.start;
        let mut second_gradient = gradient.subrange(t, 1.0);
        second_gradient.end = gradient.end;
        first.gradient = Some(first_gradient);
        second.gradient = Some(second_gradient);
    }
    if source.brush.dabs.enabled {
        let total = if source.brush.dab_total_length > 0.0 {
            source.brush.dab_total_length
        } else {
            source.brush.dab_distance_origin + total_length
        };
        first.brush.dab_total_length = total;
        second.brush.dab_total_length = total;
        second.brush.dab_distance_origin += split_distance;
    }
    validate_geometry(&first)?;
    validate_geometry(&second)?;
    let after = vec![(*index, first), (*index + 1, second)];
    Ok(SplitPlan {
        edit: Edit::Change { before, after },
        next_id: after_next_id,
        first_id: stroke_id,
        second_id: next_id,
    })
}

/// Join two compatible ordinary strokes in the requested point order. The
/// endpoint tolerance is in world units; a nonzero gap is retained as a short
/// connecting segment, while an identical seam sample is stored only once.
pub fn join_strokes(
    doc: &Document,
    first_id: u64,
    second_id: u64,
    endpoint_tolerance: f32,
) -> Result<Edit, String> {
    if first_id == second_id {
        return Err("Choose two different strokes to join.".into());
    }
    if !endpoint_tolerance.is_finite() || !(0.0..=MAX_JOIN_TOLERANCE).contains(&endpoint_tolerance)
    {
        return Err(format!(
            "Endpoint tolerance must be from 0 to {MAX_JOIN_TOLERANCE} world units."
        ));
    }
    let before = snapshot_for_point_operation(doc, &[first_id, second_id])?;
    if before.len() != 2 {
        return Err("Select two editable ordinary strokes.".into());
    }
    let first_entry = before
        .iter()
        .find(|(_, stroke)| stroke.id == first_id)
        .ok_or("The first stroke is unavailable.")?;
    let second_entry = before
        .iter()
        .find(|(_, stroke)| stroke.id == second_id)
        .ok_or("The second stroke is unavailable.")?;
    let first = &first_entry.1;
    let second = &second_entry.1;
    if first.points.len() < 2 || second.points.len() < 2 {
        return Err("Both strokes need at least two samples.".into());
    }
    let duplicate_seam_sample = first.points.last() == second.points.first();
    let combined_point_count =
        first.points.len() + second.points.len() - usize::from(duplicate_seam_sample);
    if combined_point_count > MAX_CANVAS_POINTS {
        return Err(format!(
            "A joined stroke supports at most {MAX_CANVAS_POINTS} samples."
        ));
    }
    if first.layer_id != second.layer_id || first.group_id != second.group_id {
        return Err("Strokes must belong to the same layer and group.".into());
    }
    if first.material != second.material
        || first.color != second.color
        || first.radius != second.radius
    {
        return Err("Strokes must have matching color, radius, and material.".into());
    }
    if !brush_settings_match(&first.brush, &second.brush) {
        return Err("Strokes must use matching brush settings and dab seeds.".into());
    }
    let endpoint_gap = first
        .points
        .last()
        .unwrap()
        .position
        .distance(second.points[0].position);
    if !endpoint_gap.is_finite() || endpoint_gap > endpoint_tolerance {
        return Err("The chosen stroke endpoints are farther apart than the tolerance.".into());
    }
    if first.brush.flow.enabled
        && (first
            .points
            .windows(2)
            .chain(second.points.windows(2))
            .any(|pair| pair[0].time_ms > pair[1].time_ms)
            || first.points.last().unwrap().time_ms > second.points[0].time_ms)
    {
        return Err("Timed strokes must continue forward in sample time.".into());
    }
    let combined_gradient = match (first.gradient, second.gradient) {
        (None, None) => None,
        (Some(a), Some(b))
            if a.end
                .iter()
                .zip(b.start)
                .all(|(left, right)| (*left - right).abs() <= JOIN_COLOR_EPSILON) =>
        {
            Some(crate::document::StrokeGradient {
                start: a.start,
                end: b.end,
            })
        }
        (Some(_), Some(_)) => {
            return Err("Gradient colors must meet at the join endpoint.".into());
        }
        _ => return Err("Both strokes must use the same gradient mode.".into()),
    };

    if first.brush.dabs.enabled {
        if endpoint_gap > JOIN_DISTANCE_EPSILON {
            return Err("Dab strokes can join only at coincident endpoints.".into());
        }
        let first_length = stroke_length(first);
        let expected_origin = first.brush.dab_distance_origin + first_length;
        let origin_epsilon = 1.0e-4_f32.max(first_length.abs() * 1.0e-5);
        if (second.brush.dab_distance_origin - expected_origin).abs() > origin_epsilon
            || (effective_dab_total(first) - effective_dab_total(second)).abs() > origin_epsilon
        {
            return Err("Dab placement phases must be continuous across the join.".into());
        }
    }

    let mut joined = first.clone();
    joined.gradient = combined_gradient;
    joined.points.extend(
        second
            .points
            .iter()
            .skip(usize::from(duplicate_seam_sample))
            .copied(),
    );
    if joined.brush.dabs.enabled {
        let expected_total = joined.brush.dab_distance_origin + stroke_length(&joined);
        let total = effective_dab_total(first);
        joined.brush.dab_total_length = if (total - expected_total).abs() <= 1.0e-4 {
            0.0
        } else {
            total
        };
    }
    validate_geometry(&joined)?;
    let insertion_index = first_entry.0.min(second_entry.0);
    Ok(Edit::Change {
        before,
        after: vec![(insertion_index, joined)],
    })
}

fn snapshot_for_point_operation(
    doc: &Document,
    ids: &[u64],
) -> Result<Vec<(usize, Stroke)>, String> {
    let selected: HashSet<_> = ids.iter().copied().collect();
    let before = snapshot(doc, &selected);
    if before.len() != selected.len() {
        return Err("Missing, hidden, locked, or generated strokes cannot be edited.".into());
    }
    for (_, stroke) in &before {
        if stroke.fill.is_some() || stroke.curve.is_some() {
            return Err("Fills and semantic curves use their existing editors.".into());
        }
        if doc.has_linked_source(stroke.id) || doc.linked_instance_for_stroke(stroke.id).is_some() {
            return Err("Make linked strokes unique before editing their samples.".into());
        }
        if !(2..=MAX_CANVAS_POINTS).contains(&stroke.points.len()) {
            return Err(format!(
                "Point operations support 2–{MAX_CANVAS_POINTS} samples per stroke."
            ));
        }
    }
    Ok(before)
}

fn brush_settings_match(first: &crate::document::Brush, second: &crate::document::Brush) -> bool {
    let mut first = *first;
    let mut second = *second;
    first.dab_distance_origin = 0.0;
    second.dab_distance_origin = 0.0;
    first.dab_total_length = 0.0;
    second.dab_total_length = 0.0;
    first == second
}

fn stroke_length(stroke: &Stroke) -> f32 {
    crate::dynamics::arc_lengths(&stroke.points)
        .last()
        .copied()
        .unwrap_or(0.0)
}

fn effective_dab_total(stroke: &Stroke) -> f32 {
    if stroke.brush.dab_total_length > 0.0 {
        stroke.brush.dab_total_length
    } else {
        stroke.brush.dab_distance_origin + stroke_length(stroke)
    }
}

pub fn edit_point(
    doc: &Document,
    stroke_id: u64,
    index: usize,
    position: Option<Vec3>,
) -> Result<Edit, String> {
    let before = snapshot(doc, &HashSet::from([stroke_id]));
    let Some((_, source)) = before.first() else {
        return Err("Select an editable ordinary stroke.".into());
    };
    if source.fill.is_some() || source.curve.is_some() || doc.has_linked_source(stroke_id) {
        return Err(
            "Use the curve or group editor for this stroke; fills are not supported.".into(),
        );
    }
    if index >= source.points.len() {
        return Err("Point index is outside the stroke.".into());
    }
    let mut after = before.clone();
    let stroke = &mut after[0].1;
    if let Some(position) = position {
        let mut point = stroke.points[index];
        point.position = position;
        if !point.is_valid() {
            return Err("Point coordinates must be finite and within ±10000 world units.".into());
        }
        stroke.points[index] = point;
    } else {
        if stroke.points.len() <= 2 {
            return Err(
                "Keep at least two points; delete the stroke to remove it entirely.".into(),
            );
        }
        stroke.points.remove(index);
    }
    if before == after {
        return Err("Point position is unchanged.".into());
    }
    validate_geometry(&after[0].1)?;
    Ok(Edit::Change { before, after })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point, Stroke, StrokeMaterial};
    fn sample() -> Document {
        let mut doc = Document::default();
        doc.strokes.push(Stroke {
            id: 1,
            layer_id: 1,
            group_id: None,
            fill: None,
            curve: None,
            material: StrokeMaterial::Shaded,
            color: [0.2, 0.4, 0.6],
            gradient: None,
            radius: 0.02,
            brush: Brush::default(),
            points: (0..3)
                .map(|i| Point {
                    position: Vec3::X * i as f32,
                    pressure: 0.25 * (i + 1) as f32,
                    tilt: Some(Vec3::Y),
                    time_ms: i * 10,
                })
                .collect(),
        });
        doc
    }

    fn four_point_sample() -> Document {
        let mut doc = sample();
        let mut fourth = doc.strokes[0].points[2];
        fourth.position = Vec3::X * 3.0;
        fourth.pressure = 0.9;
        fourth.tilt = Some(Vec3::new(0.0, 0.6, 0.8));
        fourth.time_ms = 30;
        doc.strokes[0].points.push(fourth);
        doc
    }

    #[test]
    fn split_and_join_roundtrip_preserves_samples_gradient_dabs_and_history() {
        let mut doc = four_point_sample();
        doc.strokes[0].gradient = Some(crate::document::StrokeGradient {
            start: [0.0, 0.1, 0.2],
            end: [0.8, 0.7, 0.6],
        });
        doc.strokes[0].brush.dabs.enabled = true;
        doc.strokes[0].brush.dabs.spacing = 0.4;
        doc.strokes[0].brush.dab_seed = 41;
        let original = doc.strokes[0].clone();
        let dabs = crate::dab_placement::sample(&original).unwrap();
        let original_document = Document {
            strokes: vec![original.clone()],
            ..Document::default()
        };
        let original_glb = crate::export::encode(&original_document, None)
            .unwrap()
            .bytes;
        let plan = split_stroke(&doc, 1, 1, 2).unwrap();
        assert_eq!(plan.next_id, 3);
        assert_eq!((plan.first_id, plan.second_id), (1, 2));
        plan.edit.apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 2);
        assert_eq!(doc.strokes[0].points, original.points[..=1]);
        assert_eq!(doc.strokes[1].points, original.points[1..]);
        assert_eq!(doc.strokes[0].points[1], doc.strokes[1].points[0]);
        let first_dabs = crate::dab_placement::sample(&doc.strokes[0]).unwrap();
        let second_dabs = crate::dab_placement::sample(&doc.strokes[1]).unwrap();
        let split_dabs: Vec<_> = first_dabs.iter().chain(&second_dabs).collect();
        assert_eq!(
            split_dabs.len(),
            dabs.len(),
            "the shared split seam emits one dab"
        );
        assert!(first_dabs.last().unwrap().position.distance(Vec3::X) < 1.0e-5);
        assert!(second_dabs[0].position.distance(Vec3::X) > 1.0e-5);
        for (split_dab, original_dab) in split_dabs.iter().zip(&dabs) {
            assert!(split_dab.position.distance(original_dab.position) < 1.0e-4);
            assert!((split_dab.pressure - original_dab.pressure).abs() < 1.0e-5);
            assert!((split_dab.size_scale - original_dab.size_scale).abs() < 1.0e-5);
        }
        assert!(doc.strokes[0]
            .gradient
            .unwrap()
            .end
            .iter()
            .zip(doc.strokes[1].gradient.unwrap().start)
            .all(|(a, b)| (a - b).abs() < 1.0e-6));
        assert!(doc.strokes[1].brush.dab_distance_origin > 0.0);
        let selected = HashSet::from([1, 2]);
        let split_bounds = crate::editing::bounds(&doc, &selected).unwrap();
        let original_bounds = crate::editing::bounds(
            &Document {
                strokes: vec![original.clone()],
                ..Document::default()
            },
            &HashSet::from([1]),
        )
        .unwrap();
        assert_eq!(split_bounds, original_bounds);

        let recovery_dir = tempfile::tempdir().unwrap();
        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "point-split-join-test".into(),
            None,
            doc.clone(),
        );
        crate::recovery_store::write(recovery_dir.path(), &recovery).unwrap();
        let recovered =
            crate::recovery_store::scan(recovery_dir.path(), "point-split-join-reader").unwrap();
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes,
            doc.strokes
        );
        let glb = crate::export::encode(&doc, None).unwrap();
        assert_eq!(glb.summary.strokes, 2);
        let obj_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let obj_path = obj_dir.path().join("split.obj");
        assert!(crate::obj_export::write_obj_bundle(&doc, None, &obj_path, false).unwrap() > 0);
        assert!(std::fs::read_to_string(obj_path).unwrap().contains("v "));

        let join = join_strokes(&doc, 1, 2, 0.0).unwrap();
        join.apply(&mut doc, true);
        assert_eq!(doc.strokes, vec![original.clone()]);
        assert_eq!(crate::dab_placement::sample(&doc.strokes[0]).unwrap(), dabs);
        assert_eq!(
            crate::export::encode(&doc, None).unwrap().bytes,
            original_glb
        );
        assert_eq!(
            Document::from_json(&serde_json::to_string(&doc).unwrap())
                .unwrap()
                .strokes,
            doc.strokes
        );
        join.apply(&mut doc, false);
        assert_eq!(doc.strokes.len(), 2);
        plan.edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, vec![original]);
    }

    #[test]
    fn join_uses_explicit_order_and_world_endpoint_tolerance() {
        let mut doc = four_point_sample();
        let mut first = doc.strokes[0].clone();
        first.id = 10;
        first.points = vec![doc.strokes[0].points[2], doc.strokes[0].points[3]];
        let mut second = doc.strokes[0].clone();
        second.id = 11;
        second.points = vec![doc.strokes[0].points[0], doc.strokes[0].points[1]];
        doc.strokes = vec![first.clone(), second.clone()];
        let join = join_strokes(&doc, 11, 10, 1.0).unwrap();
        join.apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 1);
        assert_eq!(doc.strokes[0].id, 11);
        assert_eq!(doc.strokes[0].points, four_point_sample().strokes[0].points);
        join.apply(&mut doc, false);
        assert_eq!(doc.strokes, vec![first, second]);

        let mut near = four_point_sample();
        near.strokes.truncate(1);
        let mut tail = near.strokes[0].clone();
        tail.id = 2;
        tail.points = vec![
            crate::document::Point {
                position: Vec3::new(2.05, 0.0, 0.0),
                ..near.strokes[0].points[2]
            },
            crate::document::Point {
                position: Vec3::new(3.0, 0.0, 0.0),
                ..near.strokes[0].points[3]
            },
        ];
        near.strokes[0].points = near.strokes[0].points[..=2].to_vec();
        near.strokes.push(tail);
        assert!(join_strokes(&near, 1, 2, 0.04).is_err());
        let joined = join_strokes(&near, 1, 2, 0.051).unwrap();
        joined.apply(&mut near, true);
        assert_eq!(
            near.strokes[0].points.len(),
            5,
            "the accepted gap stays explicit"
        );
    }

    #[test]
    fn join_rejects_incompatible_attributes_and_protected_or_semantic_strokes() {
        let base = four_point_sample();
        assert!(split_stroke(&base, 1, usize::MAX, 2).is_err());
        let mut compatible = base.clone();
        let mut tail = compatible.strokes[0].clone();
        tail.id = 2;
        tail.points = vec![
            compatible.strokes[0].points[2],
            compatible.strokes[0].points[3],
        ];
        compatible.strokes[0].points.truncate(3);
        compatible.strokes.push(tail);
        compatible.strokes[1].material = crate::document::StrokeMaterial::Unlit;
        assert!(join_strokes(&compatible, 1, 2, 0.0).is_err());
        compatible.strokes[1].material = compatible.strokes[0].material;
        compatible.strokes[1].brush.softness = 0.2;
        assert!(join_strokes(&compatible, 1, 2, 0.0).is_err());
        compatible.strokes[1].brush.softness = compatible.strokes[0].brush.softness;
        compatible.strokes[1].layer_id = 999;
        assert!(join_strokes(&compatible, 1, 2, 0.0).is_err());

        let mut fill = base.clone();
        fill.strokes[0].fill = Some(crate::document::PlanarFill { offset: 0.0 });
        assert!(split_stroke(&fill, 1, 1, 2).is_err());
        let mut locked = base;
        locked.layers[0].locked = true;
        assert!(split_stroke(&locked, 1, 1, 2).is_err());
    }

    #[test]
    fn split_and_join_preserve_timed_airbrush_samples_in_order() {
        let mut doc = four_point_sample();
        doc.strokes[0].brush.flow.enabled = true;
        for (index, point) in doc.strokes[0].points.iter_mut().enumerate() {
            point.time_ms = index as u32 * 30;
        }
        let original = doc.strokes[0].clone();
        let split = split_stroke(&doc, 1, 1, 2).unwrap();
        split.edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0].points.last().unwrap().time_ms, 30);
        assert_eq!(doc.strokes[1].points[0].time_ms, 30);
        assert!(doc.strokes[1]
            .points
            .windows(2)
            .all(|pair| pair[0].time_ms <= pair[1].time_ms));
        let join = join_strokes(&doc, 1, 2, 0.0).unwrap();
        join.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], original);
        let native = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(native.strokes, vec![original]);
        assert!(crate::export::encode(&native, None).is_ok());
    }
    #[test]
    fn move_delete_roundtrip_and_exact_history() {
        let mut doc = sample();
        let original = doc.strokes.clone();
        let edit = edit_point(&doc, 1, 1, Some(Vec3::Y)).unwrap();
        edit.apply(&mut doc, true);
        let mut expected = original[0].points[1];
        expected.position = Vec3::Y;
        assert_eq!(doc.strokes[0].points[1], expected);
        let json = serde_json::to_string(&doc).unwrap();
        assert_eq!(Document::from_json(&json).unwrap().strokes, doc.strokes);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
        edit.apply(&mut doc, true);
        let delete = edit_point(&doc, 1, 1, None).unwrap();
        delete.apply(&mut doc, true);
        assert_eq!(
            doc.strokes[0].points,
            vec![original[0].points[0], original[0].points[2]]
        );
        assert!(edit_point(&doc, 1, 0, None).is_err());
        delete.apply(&mut doc, false);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
    }
    #[test]
    fn rejects_invalid_and_protected_edits_without_mutation() {
        let mut doc = sample();
        let original = doc.strokes.clone();
        for position in [
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
            Vec3::X * 10001.0,
        ] {
            assert!(edit_point(&doc, 1, 0, Some(position)).is_err());
        }
        assert!(edit_point(&doc, 1, 99, None).is_err());
        assert!(edit_point(&doc, 999, 0, None).is_err());
        assert!(edit_point(&doc, 1, 0, Some(Vec3::ZERO)).is_err());
        doc.layers[0].locked = true;
        assert!(edit_point(&doc, 1, 0, Some(Vec3::Y)).is_err());
        doc.layers[0].locked = false;
        doc.layers[0].visible = false;
        assert!(edit_point(&doc, 1, 0, None).is_err());
        assert_eq!(doc.strokes, original);
        doc.layers[0].visible = true;
        doc.strokes[0].fill = Some(crate::document::PlanarFill { offset: 0.0 });
        assert!(edit_point(&doc, 1, 0, Some(Vec3::Y)).is_err());
    }

    #[test]
    fn detached_session_batches_changes_and_rejects_stale_source() {
        let mut doc = sample();
        let original = doc.strokes.clone();
        let mut draft = Draft::new(&doc, 1).unwrap();
        assert!(draft.edit(&doc).is_err());
        draft.selected = 1;
        draft.move_selected(Vec3::new(1.0, 0.4, 0.0)).unwrap();
        draft.move_selected(Vec3::new(1.0, 0.8, 0.0)).unwrap();
        assert_eq!(doc.strokes, original, "preview never changes the document");
        let preview = draft.preview.clone();
        let edit = draft.edit(&doc).unwrap();
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], preview);
        assert!(
            draft.edit(&doc).is_err(),
            "a stale preview cannot overwrite an edit"
        );
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
        doc.layers[0].locked = true;
        assert!(draft.edit(&doc).is_err());
        doc.layers[0].locked = false;
        draft.delete_selected().unwrap();
        assert_eq!(draft.preview.points.len(), 2);
        assert!(draft.delete_selected().is_err());
        drop(draft);
        assert_eq!(
            doc.strokes, original,
            "discarding a preview is exact cancellation"
        );
    }

    #[test]
    fn edited_geometry_survives_recovery_and_exports() {
        let mut doc = sample();
        let mut draft = Draft::new(&doc, 1).unwrap();
        draft.selected = 1;
        draft.move_selected(Vec3::new(1.0, 0.6, 0.2)).unwrap();
        draft.edit(&doc).unwrap().apply(&mut doc, true);
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "point-edit-test".into(),
            None,
            doc.clone(),
        );
        let json = serde_json::to_vec(&snapshot).unwrap();
        let loaded: crate::recovery_store::RecoverySnapshot =
            serde_json::from_slice(&json).unwrap();
        assert_eq!(loaded.document.strokes, doc.strokes);
        let glb = crate::export::encode(&doc, None).unwrap().bytes;
        assert_eq!(
            crate::export::encode(&loaded.document, None).unwrap().bytes,
            glb
        );
        assert!(
            crate::editing::bounds(&doc, &HashSet::from([1]))
                .unwrap()
                .1
                .y
                >= 0.6
        );
    }

    #[test]
    fn canvas_point_budget_and_dab_expansion_are_bounded() {
        let mut doc = sample();
        doc.strokes[0].points = vec![doc.strokes[0].points[0]; MAX_CANVAS_POINTS + 1];
        assert!(Draft::new(&doc, 1).is_err());
        let mut doc = sample();
        doc.strokes[0].brush.dabs.enabled = true;
        doc.strokes[0].brush.dabs.spacing = 0.1;
        let mut draft = Draft::new(&doc, 1).unwrap();
        let previous = draft.preview.clone();
        draft.selected = 1;
        assert!(draft.move_selected(Vec3::X * 9999.0).is_err());
        assert_eq!(draft.preview, previous);
        assert!(edit_point(&doc, 1, 1, Some(Vec3::X * 9999.0)).is_err());
    }
}
