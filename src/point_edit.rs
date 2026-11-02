//! Bounded edits to sampled strokes; semantic curves retain their own editor.
use crate::document::{Document, Stroke};
use crate::editing::{snapshot, Edit};
use glam::Vec3;
use std::collections::HashSet;

pub const MAX_CANVAS_POINTS: usize = 4096;

/// A detached preview. Saving, picking and history see committed geometry until Apply.
pub struct Draft {
    pub original: Stroke,
    pub preview: Stroke,
    pub selected: usize,
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
