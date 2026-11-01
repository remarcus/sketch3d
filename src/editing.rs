//! Reversible edits and transforms, independent of the interface and GPU.
use crate::document::{BrushKind, Document, Layer, Stroke, StrokeMaterial};
use glam::{Mat3, Quat, Vec3};
use std::collections::HashSet;

pub type Snapshot = Vec<(usize, Stroke)>;
pub type ImagePlaneState = (
    Vec<crate::image_planes::ImagePlane>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
);
pub type GuideState = (
    Vec<crate::document::guide_resources::GuideResource>,
    Option<u64>,
    Vec<crate::image_planes::ImagePlane>,
    Option<u64>,
    Option<u64>,
);
pub type ModelReferenceState = (
    Vec<crate::document::model_references::ObjReferenceResource>,
    Option<u64>,
    Vec<crate::image_planes::ImagePlane>,
    Option<u64>,
    Vec<crate::document::guide_resources::GuideResource>,
    Option<u64>,
);

/// Content introduced by one native-drawing insertion. Undo removes document
/// references while deliberately leaving copied managed assets in place; a
/// later merge may share those files and must not lose them on undo.
#[derive(Clone)]
pub struct MergeEdit {
    pub strokes: Vec<Stroke>,
    pub layers: Vec<Layer>,
    pub image_planes: Vec<crate::image_planes::ImagePlane>,
    pub guides: Vec<crate::document::guide_resources::GuideResource>,
    pub model_references: Vec<crate::document::model_references::ObjReferenceResource>,
    pub previous_active_layer: u64,
    pub inserted_active_layer: u64,
}

/// A partial appearance update. `None` means leave that property exactly as-is,
/// including when the selected strokes have mixed values.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppearancePatch {
    pub color: Option<[f32; 3]>,
    pub radius: Option<f32>,
    pub opacity: Option<f32>,
    pub brush_kind: Option<BrushKind>,
    pub material: Option<StrokeMaterial>,
}

impl AppearancePatch {
    fn is_empty(self) -> bool {
        self.color.is_none()
            && self.radius.is_none()
            && self.opacity.is_none()
            && self.brush_kind.is_none()
            && self.material.is_none()
    }
}

/// Builds the preview snapshot for an appearance edit without mutating the
/// document. Passing the original snapshot on each preview keeps untouched
/// values exact and prevents cumulative rounding/drift.
///
/// Filled strokes keep their captured contour. Color and opacity may be
/// changed, but radius and brush kind are rejected for a selection containing
/// fills because those surface-brush properties do not alter fill geometry.
pub fn appearance_preview(before: &Snapshot, patch: AppearancePatch) -> Result<Snapshot, String> {
    if before.is_empty() {
        return Err("Select at least one editable stroke".into());
    }
    if patch.is_empty() {
        return Err("Change at least one appearance property".into());
    }
    if patch.color.is_some_and(|color| {
        color
            .iter()
            .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
    }) {
        return Err("Color components must be finite values from 0 to 1".into());
    }
    if patch
        .radius
        .is_some_and(|radius| !radius.is_finite() || !(0.001..=1.0).contains(&radius))
    {
        return Err("Radius must be a finite value from 0.001 to 1".into());
    }
    if patch
        .opacity
        .is_some_and(|opacity| !opacity.is_finite() || !(0.0..=1.0).contains(&opacity))
    {
        return Err("Opacity must be a finite value from 0 to 1".into());
    }
    if before.iter().any(|(_, stroke)| stroke.fill.is_some())
        && (patch.radius.is_some() || patch.brush_kind.is_some())
    {
        return Err("Filled strokes keep their contour; radius and brush changes apply only to non-filled strokes".into());
    }

    let mut after = before.clone();
    for (_, stroke) in &mut after {
        if let Some(color) = patch.color {
            stroke.color = color;
        }
        if let Some(radius) = patch.radius {
            stroke.radius = radius;
        }
        if let Some(opacity) = patch.opacity {
            stroke.brush.opacity = opacity;
        }
        if let Some(kind) = patch.brush_kind {
            stroke.brush.kind = kind;
        }
        if let Some(material) = patch.material {
            stroke.material = material;
        }
        if !stroke.brush.is_valid() {
            return Err("Appearance change would make brush settings invalid".into());
        }
        if !stroke.radius.is_finite() || !(0.001..=1.0).contains(&stroke.radius) {
            return Err("Appearance change would exceed the valid radius range".into());
        }
    }
    Ok(after)
}

/// Stages a reversible appearance edit for the complete editable selection.
/// The returned `Edit::Change` can be applied for preview, committed as one
/// undo step, or reversed to cancel. Hidden/locked/missing selected strokes
/// reject the whole operation, so callers cannot accidentally partially edit
/// protected content. See [`appearance_preview`] for fill compatibility.
pub fn appearance_edit(
    doc: &Document,
    selected: &HashSet<u64>,
    patch: AppearancePatch,
) -> Result<Edit, String> {
    if selected.is_empty() {
        return Err("Select at least one editable stroke".into());
    }
    let before = snapshot(doc, selected);
    if before.len() != selected.len() {
        return Err("Hidden, locked, or missing strokes cannot be edited".into());
    }
    let after = appearance_preview(&before, patch)?;
    Ok(Edit::Change { before, after })
}

#[derive(Clone)]
pub enum Edit {
    Add(Stroke),
    Remove(usize, Stroke),
    Change {
        before: Snapshot,
        after: Snapshot,
    },
    Layers {
        before: LayerState,
        after: LayerState,
    },
    ImagePlanes {
        before: ImagePlaneState,
        after: ImagePlaneState,
    },
    Guides {
        before: GuideState,
        after: GuideState,
    },
    ModelReferences {
        before: ModelReferenceState,
        after: ModelReferenceState,
    },
    MixedResources {
        before: crate::mixed_resources::ResourceState,
        after: crate::mixed_resources::ResourceState,
    },
    Merge(MergeEdit),
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
            Self::Layers { before, after } => {
                let state = if forward { after } else { before };
                doc.layers = state.layers.clone();
                doc.active_layer = state.active_layer;
                let assignments: std::collections::HashMap<_, _> =
                    state.assignments.iter().copied().collect();
                for stroke in &mut doc.strokes {
                    if let Some(layer) = assignments.get(&stroke.id) {
                        stroke.layer_id = *layer;
                    }
                }
            }
            Self::ImagePlanes { before, after } => {
                let (planes, active, active_guide, active_model) =
                    if forward { after } else { before };
                doc.image_planes = planes.clone();
                doc.active_image_plane = *active;
                doc.active_guide = *active_guide;
                doc.active_model_reference = *active_model;
            }
            Self::Guides { before, after } => {
                let (guides, active_guide, planes, active_image_plane, active_model) =
                    if forward { after } else { before };
                doc.guide_resources = guides.clone();
                doc.active_guide = *active_guide;
                doc.image_planes = planes.clone();
                doc.active_image_plane = *active_image_plane;
                doc.active_model_reference = *active_model;
            }
            Self::ModelReferences { before, after } => {
                let (models, active_model, planes, active_plane, guides, active_guide) =
                    if forward { after } else { before };
                doc.model_references = models.clone();
                doc.active_model_reference = *active_model;
                doc.image_planes = planes.clone();
                doc.active_image_plane = *active_plane;
                doc.guide_resources = guides.clone();
                doc.active_guide = *active_guide;
            }
            Self::MixedResources { before, after } => {
                crate::mixed_resources::restore_state(doc, if forward { after } else { before });
            }
            Self::Merge(merge) => {
                if forward {
                    doc.strokes.extend(merge.strokes.iter().cloned());
                    doc.layers.extend(merge.layers.iter().cloned());
                    doc.image_planes.extend(merge.image_planes.iter().cloned());
                    doc.guide_resources.extend(merge.guides.iter().cloned());
                    doc.model_references
                        .extend(merge.model_references.iter().cloned());
                    doc.active_layer = merge.inserted_active_layer;
                } else {
                    let stroke_ids: HashSet<_> = merge.strokes.iter().map(|s| s.id).collect();
                    let layer_ids: HashSet<_> = merge.layers.iter().map(|l| l.id).collect();
                    let plane_ids: HashSet<_> = merge.image_planes.iter().map(|p| p.id).collect();
                    let guide_ids: HashSet<_> = merge.guides.iter().map(|g| g.id).collect();
                    let model_ids: HashSet<_> =
                        merge.model_references.iter().map(|m| m.id).collect();
                    doc.strokes.retain(|s| !stroke_ids.contains(&s.id));
                    doc.layers.retain(|l| !layer_ids.contains(&l.id));
                    doc.image_planes.retain(|p| !plane_ids.contains(&p.id));
                    doc.guide_resources.retain(|g| !guide_ids.contains(&g.id));
                    doc.model_references.retain(|m| !model_ids.contains(&m.id));
                    doc.active_layer = merge.previous_active_layer;
                }
            }
        }
    }
}
pub fn snapshot(doc: &Document, selected: &HashSet<u64>) -> Snapshot {
    doc.strokes
        .iter()
        .enumerate()
        .filter(|(_, s)| selected.contains(&s.id) && doc.editable(s))
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
    for stroke in doc
        .strokes
        .iter()
        .filter(|s| selected.contains(&s.id) && doc.editable(s))
    {
        let extent = Vec3::splat(crate::geometry::stroke_extent(stroke));
        for point in &stroke.points {
            let position = point.position
                - stroke
                    .fill
                    .map_or(Vec3::ZERO, |fill| stroke.brush.normal * fill.offset);
            min = min.min(position - extent);
            max = max.max(position + extent);
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
    transformed_with_scale(
        before,
        pivot,
        translation,
        rotation,
        Vec3::splat(scale),
        Quat::IDENTITY,
        true,
    )
}

/// Apply a positive scale along the axes of `scale_basis`, optionally scaling
/// each stroke's isotropic brush width by the geometric mean of its two
/// transformed cross-section axes. Existing callers use the uniform wrapper.
pub fn transformed_with_scale(
    before: &Snapshot,
    pivot: Vec3,
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
    scale_basis: Quat,
    scale_brush_width: bool,
) -> Result<Snapshot, &'static str> {
    if !scale.is_finite()
        || scale.min_element() <= 0.0
        || scale.max_element() > 100.0
        || !pivot.is_finite()
        || !translation.is_finite()
        || !rotation.is_finite()
        || !scale_basis.is_finite()
        || scale_basis.length_squared() < 1e-8
    {
        return Err("Invalid transform");
    }
    // Preserve exact data for a click or a drag returned to its starting point.
    if translation == Vec3::ZERO && rotation == Quat::IDENTITY && scale == Vec3::ONE {
        return Ok(before.clone());
    }
    let basis = Mat3::from_quat(scale_basis.normalize());
    let linear = Mat3::from_quat(rotation.normalize())
        * basis
        * Mat3::from_diagonal(scale)
        * basis.transpose();
    let normal_matrix = linear.inverse().transpose();
    let average_scale = scale.x * scale.y * scale.z;
    let average_scale = average_scale.cbrt();
    if !linear.is_finite() || !normal_matrix.is_finite() || !average_scale.is_finite() {
        return Err("Invalid transform");
    }
    let mut after = before.clone();
    for (_, stroke) in &mut after {
        let old_normal = stroke.brush.normal;
        let old_tangent = stroke.brush.tangent;
        let old_side = old_normal.cross(old_tangent).normalize_or_zero();
        if old_side == Vec3::ZERO {
            return Err("Stroke brush frame is invalid");
        }
        let transformed_normal = normal_matrix * old_normal;
        let transformed_tangent = linear * old_tangent;
        let normal_factor = transformed_normal.length();
        let next_normal = transformed_normal
            .try_normalize()
            .ok_or("Brush frame collapsed")?;
        let next_tangent = (transformed_tangent
            - next_normal * transformed_tangent.dot(next_normal))
        .try_normalize()
        .ok_or("Brush frame collapsed")?;
        let width_scale = if scale_brush_width {
            ((linear * old_tangent).length() * (linear * old_side).length()).sqrt()
        } else {
            1.0
        };
        if let Some(fill) = &mut stroke.fill {
            fill.offset *= normal_factor.recip();
        }
        stroke.radius *= width_scale;
        stroke.brush.texture.scale *= average_scale;
        stroke.brush.dynamics.taper_start *= average_scale;
        stroke.brush.dynamics.taper_end *= average_scale;
        stroke.brush.normal = next_normal;
        stroke.brush.tangent = next_tangent;
        if !stroke.brush.is_valid() {
            return Err("Transform would exceed brush dynamics or orientation limits");
        }
        if !(0.001..=1.0).contains(&stroke.radius) {
            return Err("Scale would exceed brush radius limits");
        }
        for point in &mut stroke.points {
            point.tilt = point
                .tilt
                .map(|tilt| (linear * tilt).try_normalize().unwrap_or(tilt));
            point.position = pivot + linear * (point.position - pivot) + translation;
            if !point.is_valid() {
                return Err("Transform exceeds document bounds");
            }
        }
        if stroke.fill.is_some() && crate::polygon::triangulate(stroke).is_err() {
            return Err("Transform would exceed planar fill limits");
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
            material: Default::default(),
            fill: None,
            group_id: None,
            layer_id: 1,
            id,
            color: [0.4, 0.5, 0.6],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::new(x, 0.0, 0.0),
                pressure: 0.3,
                tilt: None,
            }],
        }
    }
    #[test]
    fn image_plane_import_edit_and_removal_undo_redo_restore_exact_state() {
        let plane = crate::image_planes::ImagePlane::from_asset(
            17,
            "Facade",
            "image-17.png".into(),
            [4, 3],
            Vec3::ZERO,
            Quat::IDENTITY,
            2.0,
        )
        .unwrap();
        let empty = (Vec::new(), None, None, None);
        let mut imported_plane = plane.clone();
        imported_plane.drawable = true;
        let imported = (vec![imported_plane.clone()], Some(17), None, None);
        let mut doc = Document::default();

        let import = Edit::ImagePlanes {
            before: empty.clone(),
            after: imported.clone(),
        };
        import.apply(&mut doc, true);
        assert_eq!(
            (
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_guide,
                doc.active_model_reference,
            ),
            imported
        );
        import.apply(&mut doc, false);
        assert_eq!(
            (
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_guide,
                doc.active_model_reference,
            ),
            empty
        );
        import.apply(&mut doc, true);

        let mut edited_plane = imported_plane;
        edited_plane.origin = Vec3::new(1.25, -0.5, 2.0);
        edited_plane.opacity = 0.35;
        edited_plane.visible = false;
        edited_plane.locked = true;
        edited_plane.drawable = false;
        let edited = (vec![edited_plane], None, None, None);
        let edit = Edit::ImagePlanes {
            before: imported.clone(),
            after: edited.clone(),
        };
        edit.apply(&mut doc, true);
        assert_eq!(
            (
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_guide,
                doc.active_model_reference,
            ),
            edited
        );
        edit.apply(&mut doc, false);
        assert_eq!(
            (
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_guide,
                doc.active_model_reference,
            ),
            imported
        );
        edit.apply(&mut doc, true);

        let remove = Edit::ImagePlanes {
            before: edited.clone(),
            after: empty.clone(),
        };
        remove.apply(&mut doc, true);
        assert_eq!(
            (
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_guide,
                doc.active_model_reference,
            ),
            empty
        );
        remove.apply(&mut doc, false);
        assert_eq!(
            (
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_guide,
                doc.active_model_reference,
            ),
            edited
        );
        remove.apply(&mut doc, true);
        assert!(doc.image_planes.is_empty());
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
        assert!((after[0].1.radius - 0.04).abs() < 1e-6);
        assert_eq!(after[0].1.points[0].pressure, 0.3);
        assert_eq!(after[0].1.color, before[0].1.color);
    }
    #[test]
    fn batch_delete_duplicate_and_transform_restore_exact_order() {
        let mut doc = Document {
            version: 1,
            strokes: (1..=5).map(|id| stroke(id, id as f32)).collect(),
            ..Document::default()
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
    fn non_uniform_scale_uses_explicit_pivot_and_optional_width_scaling() {
        let before = vec![(0, stroke(1, 2.0))];
        let pivot = Vec3::new(1.0, 0.0, 0.0);
        let fixed_width = transformed_with_scale(
            &before,
            pivot,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(2.0, 3.0, 1.0),
            Quat::IDENTITY,
            false,
        )
        .unwrap();
        assert_eq!(
            fixed_width[0].1.points[0].position,
            Vec3::new(3.0, 0.0, 0.0)
        );
        assert_eq!(fixed_width[0].1.radius, before[0].1.radius);
        assert!((fixed_width[0].1.brush.normal.length() - 1.0).abs() < 1e-6);

        let scaled_width = transformed_with_scale(
            &before,
            pivot,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(2.0, 3.0, 1.0),
            Quat::IDENTITY,
            true,
        )
        .unwrap();
        assert!((scaled_width[0].1.radius - before[0].1.radius * 6.0_f32.sqrt()).abs() < 1e-6);
        assert_eq!(before[0].1.points[0].position, Vec3::new(2.0, 0.0, 0.0));
    }
    #[test]
    fn brush_shape_orientation_and_undo_survive_transforms() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            let mut s = stroke(1, 1.0);
            s.brush = Brush {
                kind,
                thickness: 0.3,
                opacity: 1.0,
                softness: 0.0,
                angle_degrees: 35.0,
                normal: Vec3::Z,
                tangent: Vec3::Y,
                dynamics: Default::default(),
                texture: Default::default(),
            };
            let mut doc = Document {
                version: 7,
                strokes: vec![s],
                ..Document::default()
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
            assert!((after[0].1.radius - 0.04).abs() < 1e-6);
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
            version: 7,
            strokes: vec![s],
            ..Document::default()
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
                    opacity: 1.0,
                    softness: 0.0,
                    angle_degrees: 37.0,
                    normal: rotation * Vec3::Z,
                    tangent: rotation * Vec3::X,
                    dynamics: Default::default(),
                    texture: Default::default(),
                };
                let sample = Document {
                    version: 7,
                    strokes: vec![dot],
                    ..Document::default()
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
    #[test]
    fn tilt_and_taper_transform_with_the_mark_and_restore_exactly() {
        let mut s = stroke(1, 1.0);
        s.points[0].tilt = Some(Vec3::Y);
        s.brush.dynamics.tilt_orientation = true;
        s.brush.dynamics.taper_start = 0.4;
        s.brush.dynamics.taper_end = 0.7;
        let mut doc = Document {
            version: 7,
            strokes: vec![s],
            ..Document::default()
        };
        let original = doc.strokes.clone();
        let before = snapshot(&doc, &HashSet::from([1]));
        let rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let after = transformed(&before, Vec3::ZERO, Vec3::Y, rotation, 2.0).unwrap();
        assert!(after[0].1.points[0].tilt.unwrap().distance(Vec3::Z) < 1e-6);
        assert_eq!(after[0].1.brush.dynamics.taper_start, 0.8);
        assert_eq!(after[0].1.brush.dynamics.taper_end, 1.4);
        assert_eq!(
            after[0].1.points[0].pressure,
            original[0].points[0].pressure
        );
        let edit = Edit::Change { before, after };
        edit.apply(&mut doc, true);
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, doc.strokes);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes, loaded.strokes);
        let mut too_long = original[0].clone();
        too_long.brush.dynamics.taper_start = 6.0;
        assert!(transformed(
            &vec![(0, too_long)],
            Vec3::ZERO,
            Vec3::ZERO,
            Quat::IDENTITY,
            2.0
        )
        .is_err());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerState {
    pub layers: Vec<Layer>,
    pub active_layer: u64,
    pub assignments: Vec<(u64, u64)>,
}
impl LayerState {
    pub fn capture(doc: &Document) -> Self {
        Self {
            layers: doc.layers.clone(),
            active_layer: doc.active_layer,
            assignments: doc.strokes.iter().map(|s| (s.id, s.layer_id)).collect(),
        }
    }
}
pub enum LayerAction {
    Add(String),
    Rename(u64, String),
    Visible(u64, bool),
    Lock(u64, bool),
    Activate(u64),
    MoveSelection(HashSet<u64>, u64),
    Delete(u64),
}
pub fn layer_edit(doc: &Document, action: LayerAction) -> Result<Edit, String> {
    let before = LayerState::capture(doc);
    let mut after = before.clone();
    match action {
        LayerAction::Add(name) => {
            if after.layers.len() >= 256 {
                return Err("Layer limit reached".into());
            }
            let id = after
                .layers
                .iter()
                .map(|l| l.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .filter(|id| *id != u64::MAX)
                .ok_or("No layer IDs available")?;
            after.layers.push(Layer {
                id,
                name: name.trim().into(),
                visible: true,
                locked: false,
            });
            after.active_layer = id;
        }
        LayerAction::Rename(id, name) => {
            after
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Layer not found")?
                .name = name.trim().into()
        }
        LayerAction::Visible(id, value) => {
            after
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Layer not found")?
                .visible = value
        }
        LayerAction::Lock(id, value) => {
            after
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Layer not found")?
                .locked = value
        }
        LayerAction::Activate(id) => {
            if !after.layers.iter().any(|l| l.id == id) {
                return Err("Layer not found".into());
            }
            after.active_layer = id;
        }
        LayerAction::MoveSelection(ids, target) => {
            let layer = after
                .layers
                .iter()
                .find(|l| l.id == target)
                .ok_or("Target layer not found")?;
            if !layer.visible || layer.locked {
                return Err("Target layer must be visible and unlocked".into());
            }
            if ids.is_empty() {
                return Err("Select strokes to move".into());
            }
            let mut touched = HashSet::new();
            let mut found = 0;
            for stroke in doc.strokes.iter().filter(|s| ids.contains(&s.id)) {
                found += 1;
                if !doc.editable(stroke) {
                    return Err("Hidden or locked strokes cannot be moved".into());
                }
                if stroke.layer_id != target {
                    if let Some(group) = stroke.group_id {
                        touched.insert(group);
                    }
                }
            }
            if found != ids.len() {
                return Err("Selected stroke not found".into());
            }
            if doc
                .strokes
                .iter()
                .any(|s| !ids.contains(&s.id) && s.group_id.is_some_and(|g| touched.contains(&g)))
            {
                return Err("Move the whole group, or ungroup it before changing layers".into());
            }
            for (id, layer) in &mut after.assignments {
                if ids.contains(id) {
                    *layer = target;
                }
            }
        }
        LayerAction::Delete(id) => {
            if after.layers.len() == 1 {
                return Err("Keep at least one layer".into());
            }
            if !after.layers.iter().any(|l| l.id == id) {
                return Err("Layer not found".into());
            }
            if doc.strokes.iter().any(|s| s.layer_id == id) {
                return Err("Move or delete this layer's strokes before deleting the layer".into());
            }
            after.layers.retain(|l| l.id != id);
            if after.active_layer == id {
                after.active_layer = after.layers[0].id;
            }
        }
    }
    let mut names = HashSet::new();
    for layer in &after.layers {
        if layer.name.is_empty()
            || layer.name.chars().count() > 64
            || layer.name.chars().any(char::is_control)
        {
            return Err("Layer name must contain 1–64 characters".into());
        }
        if !names.insert(layer.name.to_lowercase()) {
            return Err("Layer names must be unique".into());
        }
    }
    Ok(Edit::Layers { before, after })
}

#[cfg(test)]
mod layer_tests {
    use super::*;
    use crate::document::{Brush, Point};
    fn sample() -> Document {
        Document {
            strokes: vec![Stroke {
                material: Default::default(),
                id: 1,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: [0.3; 3],
                radius: 0.03,
                brush: Brush::default(),
                points: vec![Point {
                    position: Vec3::ZERO,
                    pressure: 0.7,
                    tilt: None,
                }],
            }],
            ..Document::default()
        }
    }
    #[test]
    fn layer_metadata_and_reassignment_undo_without_changing_ink() {
        let mut doc = sample();
        let original = doc.strokes[0].clone();
        let add = layer_edit(&doc, LayerAction::Add("Details".into())).unwrap();
        add.apply(&mut doc, true);
        let target = doc.active_layer;
        let change =
            layer_edit(&doc, LayerAction::MoveSelection(HashSet::from([1]), target)).unwrap();
        change.apply(&mut doc, true);
        assert_eq!(doc.strokes[0].layer_id, target);
        assert_eq!(doc.strokes[0].points, original.points);
        assert_eq!(doc.strokes[0].brush, original.brush);
        let text = serde_json::to_string(&doc).unwrap();
        let loaded = Document::from_json(&text).unwrap();
        assert_eq!(loaded.layers, doc.layers);
        assert_eq!(loaded.strokes, doc.strokes);
        change.apply(&mut doc, false);
        assert_eq!(doc.strokes[0], original);
        add.apply(&mut doc, false);
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.active_layer, 1);
        add.apply(&mut doc, true);
        change.apply(&mut doc, true);
        assert_eq!(doc.strokes, loaded.strokes);
    }
    #[test]
    fn protected_layers_cannot_be_selected_moved_or_deleted_with_ink() {
        let mut doc = sample();
        let ids = HashSet::from([1]);
        assert!(layer_edit(&doc, LayerAction::Delete(1)).is_err());
        layer_edit(&doc, LayerAction::Add("Target".into()))
            .unwrap()
            .apply(&mut doc, true);
        let target = doc.active_layer;
        assert!(layer_edit(&doc, LayerAction::Delete(1)).is_err());
        for action in [LayerAction::Lock(1, true), LayerAction::Visible(1, false)] {
            let edit = layer_edit(&doc, action).unwrap();
            edit.apply(&mut doc, true);
            assert!(snapshot(&doc, &ids).is_empty());
            assert!(bounds(&doc, &ids).is_none());
            assert!(layer_edit(&doc, LayerAction::MoveSelection(ids.clone(), target)).is_err());
            edit.apply(&mut doc, false);
        }
        layer_edit(&doc, LayerAction::Lock(target, true))
            .unwrap()
            .apply(&mut doc, true);
        assert!(layer_edit(&doc, LayerAction::MoveSelection(ids, target)).is_err());
        assert!(layer_edit(&doc, LayerAction::Add(" target ".into())).is_err());
        assert!(layer_edit(&doc, LayerAction::Rename(1, "\n".into())).is_err());
        layer_edit(&doc, LayerAction::Delete(target))
            .unwrap()
            .apply(&mut doc, true);
        assert_eq!(doc.active_layer, 1);
    }
}

/// Shift toggles the entire hit unit, including a partially selected group.
pub fn toggle_selection(selected: &mut HashSet<u64>, ids: &HashSet<u64>, shift: bool) {
    if shift && ids.is_subset(selected) {
        selected.retain(|id| !ids.contains(id));
    } else {
        selected.extend(ids);
    }
}
pub fn group_selected(doc: &Document, ids: &HashSet<u64>) -> Result<Edit, String> {
    if ids.len() < 2 {
        return Err("Select at least two editable strokes to group".into());
    }
    let before = snapshot(doc, ids);
    if before.len() != ids.len() {
        return Err("Hidden or locked strokes cannot be grouped".into());
    }
    let layer = before[0].1.layer_id;
    if before.iter().any(|(_, s)| s.layer_id != layer) {
        return Err("A group must stay on one layer".into());
    }
    let touched: HashSet<_> = before.iter().filter_map(|(_, s)| s.group_id).collect();
    if doc
        .strokes
        .iter()
        .any(|s| !ids.contains(&s.id) && s.group_id.is_some_and(|g| touched.contains(&g)))
    {
        return Err("Select the whole existing group before regrouping".into());
    }
    let group = doc
        .strokes
        .iter()
        .filter_map(|s| s.group_id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id != u64::MAX)
        .ok_or("No group IDs available")?;
    let mut after = before.clone();
    for (_, stroke) in &mut after {
        stroke.group_id = Some(group);
    }
    Ok(Edit::Change { before, after })
}
pub fn ungroup_selected(doc: &Document, ids: &HashSet<u64>) -> Result<Edit, String> {
    let groups: HashSet<_> = doc
        .strokes
        .iter()
        .filter(|s| ids.contains(&s.id))
        .filter_map(|s| s.group_id)
        .collect();
    if groups.is_empty() {
        return Err("Select a group to ungroup".into());
    }
    let members: HashSet<_> = doc
        .strokes
        .iter()
        .filter(|s| s.group_id.is_some_and(|g| groups.contains(&g)))
        .map(|s| s.id)
        .collect();
    let before = snapshot(doc, &members);
    if before.len() != members.len() {
        return Err("Hidden or locked groups cannot be changed".into());
    }
    let mut after = before.clone();
    for (_, stroke) in &mut after {
        stroke.group_id = None;
    }
    Ok(Edit::Change { before, after })
}
/// Whole copied groups receive a new identity; individual copies are detached.
pub fn remap_duplicate_groups(
    doc: &Document,
    before: &Snapshot,
    after: &mut Snapshot,
) -> Result<(), String> {
    let mut next = doc
        .strokes
        .iter()
        .filter_map(|stroke| stroke.group_id)
        .max()
        .unwrap_or(0);
    remap_duplicate_groups_with_next(doc, before, after, &mut next)
}

pub fn remap_duplicate_groups_with_next(
    doc: &Document,
    before: &Snapshot,
    after: &mut Snapshot,
    next: &mut u64,
) -> Result<(), String> {
    let copied: HashSet<_> = before.iter().map(|(_, s)| s.id).collect();
    let mut counts = std::collections::HashMap::<u64, (usize, usize)>::new();
    for stroke in &doc.strokes {
        if let Some(group) = stroke.group_id {
            let count = counts.entry(group).or_default();
            count.0 += 1;
            if copied.contains(&stroke.id) {
                count.1 += 1;
            }
        }
    }
    let mut groups = std::collections::HashMap::new();
    for (_, stroke) in after {
        if let Some(group) = stroke.group_id {
            if counts
                .get(&group)
                .is_none_or(|(total, selected)| total != selected)
            {
                stroke.group_id = None;
                continue;
            }
            let new = if let Some(id) = groups.get(&group) {
                *id
            } else {
                *next = next
                    .checked_add(1)
                    .filter(|id| *id != u64::MAX)
                    .ok_or("No group IDs available")?;
                groups.insert(group, *next);
                *next
            };
            stroke.group_id = Some(new);
        }
    }
    Ok(())
}
#[cfg(test)]
mod group_tests {
    use super::*;
    use crate::document::{Brush, Point};
    fn sample() -> Document {
        Document {
            strokes: (1..=3)
                .map(|id| Stroke {
                    material: Default::default(),
                    id,
                    fill: None,
                    group_id: None,
                    layer_id: 1,
                    color: [0.5; 3],
                    radius: 0.02,
                    brush: Brush::default(),
                    points: vec![Point {
                        position: Vec3::X * id as f32,
                        pressure: 0.7,
                        tilt: None,
                    }],
                })
                .collect(),
            ..Default::default()
        }
    }
    #[test]
    fn grouping_transform_and_undo_preserve_exact_ink_and_order() {
        let mut doc = sample();
        let original = doc.strokes.clone();
        let ids = HashSet::from([1, 2]);
        let edit = group_selected(&doc, &ids).unwrap();
        edit.apply(&mut doc, true);
        let grouped = doc.strokes.clone();
        assert_eq!(doc.group_members(doc.strokes[0].group_id.unwrap()), ids);
        let before = snapshot(&doc, &ids);
        let after = transformed(
            &before,
            Vec3::ZERO,
            Vec3::Y,
            Quat::from_rotation_x(0.4),
            1.3,
        )
        .unwrap();
        let transform = Edit::Change { before, after };
        transform.apply(&mut doc, true);
        transform.apply(&mut doc, false);
        assert_eq!(doc.strokes, grouped);
        let ungroup = ungroup_selected(&doc, &HashSet::from([1])).unwrap();
        ungroup.apply(&mut doc, true);
        assert!(doc.strokes.iter().all(|s| s.group_id.is_none()));
        ungroup.apply(&mut doc, false);
        assert_eq!(doc.strokes, grouped);
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, grouped);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
    }
    #[test]
    fn copied_groups_are_independent_and_partial_copies_detach() {
        let mut doc = sample();
        group_selected(&doc, &HashSet::from([1, 2]))
            .unwrap()
            .apply(&mut doc, true);
        let original = doc.strokes[0].group_id;
        let before = snapshot(&doc, &HashSet::from([1, 2]));
        let mut after = before.clone();
        remap_duplicate_groups(&doc, &before, &mut after).unwrap();
        assert_ne!(after[0].1.group_id, original);
        assert_eq!(after[0].1.group_id, after[1].1.group_id);
        let before = snapshot(&doc, &HashSet::from([1]));
        let mut after = before.clone();
        remap_duplicate_groups(&doc, &before, &mut after).unwrap();
        assert_eq!(after[0].1.group_id, None);
        assert_eq!(doc.strokes[0].group_id, original);
    }
    #[test]
    fn shift_selection_and_layer_guards_keep_groups_coherent() {
        let mut doc = sample();
        let ids = HashSet::from([1, 2]);
        group_selected(&doc, &ids).unwrap().apply(&mut doc, true);
        let mut selected = HashSet::from([1]);
        toggle_selection(&mut selected, &ids, true);
        assert_eq!(selected, ids);
        toggle_selection(&mut selected, &ids, true);
        assert!(selected.is_empty());
        assert!(group_selected(&doc, &HashSet::from([1, 3])).is_err());
        layer_edit(&doc, LayerAction::Add("Other".into()))
            .unwrap()
            .apply(&mut doc, true);
        assert!(layer_edit(&doc, LayerAction::MoveSelection(HashSet::from([1]), 2)).is_err());
        layer_edit(&doc, LayerAction::MoveSelection(ids.clone(), 2))
            .unwrap()
            .apply(&mut doc, true);
        assert_eq!(doc.strokes[0].layer_id, 2);
        assert_eq!(doc.strokes[1].layer_id, 2);
        doc.layers[1].locked = true;
        assert!(ungroup_selected(&doc, &ids).is_err());
        assert!(group_selected(&doc, &ids).is_err());
    }
}

#[cfg(test)]
mod large_group_tests {
    use super::*;
    use crate::document::{Brush, Point};
    #[test]
    fn large_group_regroup_copy_and_move_preserve_membership() {
        let mut doc = Document {
            strokes: (1..=10000)
                .map(|id| Stroke {
                    material: Default::default(),
                    id,
                    fill: None,
                    group_id: Some(1 + (id - 1) / 10),
                    layer_id: 1,
                    color: [0.5; 3],
                    radius: 0.02,
                    brush: Brush::default(),
                    points: vec![Point {
                        position: Vec3::X,
                        pressure: 0.8,
                        tilt: None,
                    }],
                })
                .collect(),
            ..Default::default()
        };
        let ids: HashSet<_> = doc.strokes.iter().map(|s| s.id).collect();
        group_selected(&doc, &ids).unwrap().apply(&mut doc, true);
        let original = doc.strokes[0].group_id.unwrap();
        assert!(doc.strokes.iter().all(|s| s.group_id == Some(original)));
        let before = snapshot(&doc, &ids);
        let mut after = before.clone();
        remap_duplicate_groups(&doc, &before, &mut after).unwrap();
        let copied = after[0].1.group_id;
        assert_ne!(copied, Some(original));
        assert!(after.iter().all(|(_, s)| s.group_id == copied));
        layer_edit(&doc, LayerAction::Add("Target".into()))
            .unwrap()
            .apply(&mut doc, true);
        layer_edit(&doc, LayerAction::MoveSelection(ids, 2))
            .unwrap()
            .apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 10000);
        assert!(doc
            .strokes
            .iter()
            .all(|s| s.layer_id == 2 && s.group_id == Some(original)));
    }
}
