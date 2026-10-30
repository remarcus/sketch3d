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
        let extent = Vec3::splat(crate::geometry::stroke_extent(stroke));
        for point in &stroke.points {
            min = min.min(point.position - extent);
            max = max.max(point.position + extent);
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
        stroke.brush.normal = rotation * stroke.brush.normal;
        stroke.brush.tangent = rotation * stroke.brush.tangent;
        if !stroke.brush.is_valid() {
            return Err("Transform would invalidate brush orientation");
        }
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
    use crate::document::{Brush, BrushKind, Point};
    fn stroke(id: u64, x: f32) -> Stroke {
        Stroke {
            id,
            color: [0.4, 0.5, 0.6],
            radius: 0.02,
            brush: Brush::default(),
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
    #[test]
    fn brush_shape_orientation_and_undo_survive_transforms() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            let mut s = stroke(1, 1.0);
            s.brush = Brush {
                kind,
                thickness: 0.3,
                angle_degrees: 35.0,
                normal: Vec3::Z,
                tangent: Vec3::Y,
            };
            let mut doc = Document {
                version: 2,
                strokes: vec![s],
            };
            let original = doc.strokes.clone();
            let before = snapshot(&doc, &HashSet::from([1]));
            let rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
            let after = transformed(&before, Vec3::ZERO, Vec3::Y, rotation, 2.0).unwrap();
            assert_eq!(after[0].1.brush.kind, kind);
            assert_eq!(after[0].1.brush.thickness, 0.3);
            assert_eq!(after[0].1.brush.angle_degrees, 35.0);
            assert!(after[0].1.brush.normal.distance(-Vec3::Y) < 0.0001);
            assert!(after[0].1.brush.tangent.distance(Vec3::Z) < 0.0001);
            assert_eq!(after[0].1.radius, 0.04);
            let edit = Edit::Change { before, after };
            edit.apply(&mut doc, true);
            let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
            assert_eq!(loaded.strokes, doc.strokes);
            edit.apply(&mut doc, false);
            assert_eq!(doc.strokes, original);
            edit.apply(&mut doc, true);
            assert_eq!(doc.strokes, loaded.strokes);
        }
    }
    #[test]
    fn bounds_include_brush_cross_sections() {
        let mut s = stroke(1, 1.0);
        s.brush.kind = BrushKind::Chisel;
        s.brush.thickness = 1.0;
        let doc = Document {
            version: 2,
            strokes: vec![s],
        };
        let (min, max) = bounds(&doc, &HashSet::from([1])).unwrap();
        // A chisel dot has a square footprint and thickness. Bounds cover its
        // three-dimensional corners even at low pressure and any orientation.
        let corner_radius = 0.02 * 3.0f32.sqrt();
        assert!((min.x - (1.0 - corner_radius)).abs() < 0.00001);
        assert!((max.z - corner_radius).abs() < 0.00001);
        assert_eq!((min + max) * 0.5, Vec3::X);
        assert!(bounds(&doc, &HashSet::from([999])).is_none());
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            for angle in [0.0, 0.7, 1.5] {
                let rotation = Quat::from_euler(glam::EulerRot::XYZ, angle, 0.6, -0.4);
                let mut dot = stroke(1, 1.0);
                dot.points[0].pressure = 1.0;
                dot.brush = Brush {
                    kind,
                    thickness: 1.0,
                    angle_degrees: 37.0,
                    normal: rotation * Vec3::Z,
                    tangent: rotation * Vec3::X,
                };
                let sample = Document {
                    version: 2,
                    strokes: vec![dot],
                };
                let (low, high) = bounds(&sample, &HashSet::from([1])).unwrap();
                for vertex in crate::geometry::stroke_mesh(&sample.strokes[0]) {
                    let position = Vec3::from(vertex.position);
                    assert!(position.cmpge(low - Vec3::splat(0.000001)).all());
                    assert!(position.cmple(high + Vec3::splat(0.000001)).all());
                }
            }
        }
    }
}
