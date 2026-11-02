//! Bounded edits to sampled strokes; semantic curves retain their own editor.
use crate::document::Document;
use crate::editing::{snapshot, Edit};
use glam::Vec3;
use std::collections::HashSet;

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
}
