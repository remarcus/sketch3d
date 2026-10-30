//! Reversible edits and transforms, independent of the interface and GPU.
use crate::document::{Document, Stroke};
use glam::{Quat, Vec3};
use std::collections::HashSet;

pub type Snapshot = Vec<(usize, Stroke)>;
#[derive(Clone)]
pub enum Edit {
    Add(Stroke),
    Remove(usize, Stroke),
    Change { before: Snapshot, after: Snapshot },
}
impl Edit {
    pub fn apply(&self, doc: &mut Document, forward: bool) {
        match self {
            Self::Add(s) => {
                if forward {
                    doc.strokes.push(s.clone());
                } else {
                    doc.strokes.retain(|x| x.id != s.id);
                }
            }
            Self::Remove(index, s) => {
                if forward {
                    doc.strokes.retain(|x| x.id != s.id);
                } else {
                    doc.strokes
                        .insert((*index).min(doc.strokes.len()), s.clone());
                }
            }
            Self::Change { before, after } => replace(doc, before, after, forward),
        }
    }
}
pub fn snapshot(doc: &Document, selected: &HashSet<u64>) -> Snapshot {
    doc.strokes
        .iter()
        .enumerate()
        .filter(|(_, s)| selected.contains(&s.id))
        .map(|(i, s)| (i, s.clone()))
        .collect()
}
pub fn replace(doc: &mut Document, before: &Snapshot, after: &Snapshot, forward: bool) {
    let ids: HashSet<u64> = before.iter().chain(after).map(|(_, s)| s.id).collect();
    let source = if forward { after } else { before };
    let mut ordered: Vec<_> = source.iter().collect();
    ordered.sort_by_key(|(index, _)| *index);
    // Merge instead of repeatedly inserting into a Vec: large group gestures
    // should not shift every remaining stroke once per selected stroke.
    let capacity = doc.strokes.len() + source.len();
    let mut remaining = std::mem::take(&mut doc.strokes)
        .into_iter()
        .filter(|s| !ids.contains(&s.id));
    let mut result = Vec::with_capacity(capacity);
    for (index, stroke) in ordered {
        while result.len() < *index {
            let Some(unchanged) = remaining.next() else {
                break;
            };
            result.push(unchanged);
        }
        result.push(stroke.clone());
    }
    result.extend(remaining);
    doc.strokes = result;
}
pub fn bounds(doc: &Document, selected: &HashSet<u64>) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for stroke in doc.strokes.iter().filter(|s| selected.contains(&s.id)) {
        for point in &stroke.points {
            min = min.min(point.position);
            max = max.max(point.position);
        }
    }
    min.is_finite().then_some((min, max))
}
/// Always transform the gesture's original snapshot to avoid cumulative drift.
pub fn transformed(
    before: &Snapshot,
    pivot: Vec3,
    translation: Vec3,
    rotation: Quat,
    scale: f32,
) -> Result<Snapshot, &'static str> {
    if !scale.is_finite() || scale <= 0.0 || !translation.is_finite() || !rotation.is_finite() {
        return Err("Invalid transform");
    }
    // Preserve exact data for a click or a drag returned to its starting point.
    if translation == Vec3::ZERO && rotation == Quat::IDENTITY && scale == 1.0 {
        return Ok(before.clone());
    }
    let mut after = before.clone();
    for (_, stroke) in &mut after {
        stroke.radius *= scale;
        if !(0.001..=1.0).contains(&stroke.radius) {
            return Err("Scale would exceed brush radius limits");
        }
        for point in &mut stroke.points {
            point.position = pivot + rotation * ((point.position - pivot) * scale) + translation;
            if !point.position.is_finite() || point.position.abs().max_element() > 10000.0 {
                return Err("Transform exceeds document bounds");
            }
        }
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Point;
    fn stroke(id: u64, x: f32) -> Stroke {
        Stroke {
            id,
            color: [0.4, 0.5, 0.6],
            radius: 0.02,
            points: vec![Point {
                position: Vec3::new(x, 0.0, 0.0),
                pressure: 0.3,
            }],
        }
    }
    #[test]
    fn group_rotation_and_scale_preserve_pressure_and_color() {
        let before = vec![(0, stroke(1, 0.0)), (1, stroke(2, 2.0))];
        let after = transformed(
            &before,
            Vec3::X,
            Vec3::Z,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            2.0,
        )
        .unwrap();
        assert!(
            after[0].1.points[0]
                .position
                .distance(Vec3::new(1.0, -2.0, 1.0))
                < 0.0001
        );
        assert!(
            after[1].1.points[0]
                .position
                .distance(Vec3::new(1.0, 2.0, 1.0))
                < 0.0001
        );
        assert_eq!(after[0].1.radius, 0.04);
        assert_eq!(after[0].1.points[0].pressure, 0.3);
        assert_eq!(after[0].1.color, before[0].1.color);
    }
    #[test]
    fn batch_delete_duplicate_and_transform_restore_exact_order() {
        let mut doc = Document {
            version: 1,
            strokes: (1..=5).map(|id| stroke(id, id as f32)).collect(),
        };
        let original = doc.strokes.clone();
        let ids = HashSet::from([2, 4]);
        let before = snapshot(&doc, &ids);
        let after = transformed(&before, Vec3::ZERO, Vec3::Y, Quat::IDENTITY, 1.0).unwrap();
        let edit = Edit::Change {
            before: before.clone(),
            after,
        };
        edit.apply(&mut doc, true);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
        let delete = Edit::Change {
            before,
            after: Vec::new(),
        };
        delete.apply(&mut doc, true);
        assert_eq!(
            doc.strokes.iter().map(|s| s.id).collect::<Vec<_>>(),
            [1, 3, 5]
        );
        delete.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
        let duplicate = Edit::Change {
            before: Vec::new(),
            after: vec![(5, stroke(6, 2.0)), (6, stroke(7, 4.0))],
        };
        duplicate.apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 7);
        duplicate.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
    }
    #[test]
    fn invalid_transforms_do_not_change_original_data() {
        let before = vec![(0, stroke(1, 1.0))];
        let original = before.clone();
        assert!(transformed(&before, Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY, 100.0).is_err());
        assert!(transformed(
            &before,
            Vec3::ZERO,
            Vec3::splat(f32::NAN),
            Quat::IDENTITY,
            1.0
        )
        .is_err());
        assert_eq!(before, original);
    }
}
