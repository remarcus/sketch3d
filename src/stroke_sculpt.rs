//! Bounded local sculpt operations for selected sampled strokes.
//!
//! Smooth and Relax change centerline positions while keeping endpoints,
//! pressure, tilt, timing, brush settings, IDs, layers, and groups intact.
//! Thickness edits the existing per-sample pressure profile, which is the
//! document's explicit sampled width input; pressure-driven opacity is also
//! affected where a brush enables that mapping. Filled surfaces and semantic
//! curves are rejected until their own editable representations are defined.

use crate::{
    document::{Document, Point, Stroke},
    editing::{self, Edit, Snapshot},
    liquify::Falloff,
};
use glam::Vec3;
use std::collections::HashSet;

pub const MAX_STROKES: usize = 100_000;
pub const MAX_POINTS: usize = 2_000_000;
pub const MAX_RADIUS: f32 = 10_000.0;

/// Remap a source profile onto an ordered resampled fragment. The scan is
/// monotonic, so it is linear in source plus destination point count.
pub fn remap_thickness_profile(source: &Stroke, points: &[Point]) -> Option<Vec<f32>> {
    let values = source.point_thickness.as_ref()?;
    if values.len() != source.points.len() || source.points.is_empty() {
        return None;
    }
    if source.points.len() == 1 {
        return Some(vec![values[0]; points.len()]);
    }
    let mut result = Vec::with_capacity(points.len());
    let mut segment = 0usize;
    for point in points {
        while segment + 2 < source.points.len()
            && point
                .position
                .distance_squared(source.points[segment + 1].position)
                < point
                    .position
                    .distance_squared(source.points[segment].position)
        {
            segment += 1;
        }
        let start = source.points[segment].position;
        let end = source.points[segment + 1].position;
        let edge = end - start;
        let length_sq = edge.length_squared();
        let t = if length_sq > 1.0e-12 {
            ((point.position - start).dot(edge) / length_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        result.push(values[segment] + (values[segment + 1] - values[segment]) * t);
    }
    Some(result)
}

/// Remap the sparse local color field onto ordered resampled points. Painted
/// colors remain absolute resolved RGB values at inserted boundaries; samples
/// without paint keep their underlying solid or gradient color behavior.
pub fn remap_point_colors(source: &Stroke, points: &[Point]) -> Option<Vec<Option<[f32; 3]>>> {
    let values = source.point_colors.as_ref()?;
    if values.len() != source.points.len() || source.points.is_empty() {
        return None;
    }
    let distances = crate::dynamics::arc_lengths(&source.points);
    let total = distances.last().copied().unwrap_or(0.0);
    if source.points.len() == 1 {
        return Some(vec![values[0]; points.len()]);
    }
    let mut result = Vec::with_capacity(points.len());
    let mut segment = 0usize;
    for point in points {
        while segment + 2 < source.points.len()
            && point
                .position
                .distance_squared(source.points[segment + 1].position)
                < point
                    .position
                    .distance_squared(source.points[segment].position)
        {
            segment += 1;
        }
        let start = source.points[segment].position;
        let end = source.points[segment + 1].position;
        let edge = end - start;
        let length_sq = edge.length_squared();
        let t = if length_sq > 1.0e-12 {
            ((point.position - start).dot(edge) / length_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if values[segment].is_none() && values[segment + 1].is_none() {
            result.push(None);
            continue;
        }
        let sample_t = |index: usize| {
            if total > 1.0e-8 {
                distances[index] / total
            } else {
                0.0
            }
        };
        let a = source.point_color_at(segment, sample_t(segment));
        let b = source.point_color_at(segment + 1, sample_t(segment + 1));
        result.push(Some(std::array::from_fn(|channel| {
            a[channel] + (b[channel] - a[channel]) * t
        })));
    }
    Some(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Smooth,
    Relax,
    Thickness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThicknessDirection {
    Increase,
    Decrease,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub operation: Operation,
    pub center: Vec3,
    /// Mouse travel in the camera-facing working plane. Only its magnitude is
    /// used, so one long drag and the same drag sampled at different rates
    /// produce identical previews.
    pub travel: f32,
    pub radius: f32,
    pub strength: f32,
    pub falloff: Falloff,
    pub thickness_direction: ThicknessDirection,
}

impl Settings {
    pub fn validate(self) -> Result<(), String> {
        if !self.center.is_finite() || self.center.abs().max_element() > 10_000.0 {
            return Err("Sculpt brush center is outside document bounds".into());
        }
        if !self.travel.is_finite() || !(0.0..=MAX_RADIUS).contains(&self.travel) {
            return Err("Sculpt brush travel must be finite and no longer than 10000 units".into());
        }
        if !self.radius.is_finite() || !(0.001..=MAX_RADIUS).contains(&self.radius) {
            return Err("Sculpt radius must be between 0.001 and 10000 world units".into());
        }
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err("Sculpt strength must be between 0 and 1".into());
        }
        Ok(())
    }
}

/// Validate a complete selected-only operation. Hidden, locked, linked-source,
/// and generated instance strokes reject the whole selection.
pub fn edit(
    document: &Document,
    selected: &HashSet<u64>,
    settings: Settings,
) -> Result<Edit, String> {
    if selected.is_empty() {
        return Err("Select one or more strokes before sculpting".into());
    }
    if selected.iter().any(|id| {
        document.has_linked_source(*id) || document.linked_instance_for_stroke(*id).is_some()
    }) {
        return Err("Make linked strokes unique before sculpting".into());
    }
    let before = editing::snapshot(document, selected);
    if before.len() != selected.len() {
        return Err("Hidden, locked, or missing strokes cannot be sculpted".into());
    }
    let after = preview(&before, settings)?;
    Ok(Edit::Change { before, after })
}

/// Recompute a preview from its immutable original snapshot. Brush travel is
/// clamped to one radius, bounding the effect per mouse gesture.
pub fn preview(before: &Snapshot, settings: Settings) -> Result<Snapshot, String> {
    settings.validate()?;
    validate_snapshot(before)?;
    if settings.travel <= f32::EPSILON || settings.strength == 0.0 {
        return Ok(before.clone());
    }
    let travel_amount = (settings.travel / settings.radius).clamp(0.0, 1.0) * settings.strength;
    if travel_amount <= f32::EPSILON {
        return Ok(before.clone());
    }

    let mut after = before.clone();
    let mut changed = false;
    let radius_squared = settings.radius * settings.radius;
    for (_, stroke) in &mut after {
        let original = before
            .iter()
            .find(|(_, source)| source.id == stroke.id)
            .map(|(_, source)| source)
            .ok_or("Sculpt snapshot changed while building preview")?;
        let len = original.points.len();
        let mut stroke_changed = false;
        for index in 1..len.saturating_sub(1) {
            let point = original.points[index];
            let distance_squared = point.position.distance_squared(settings.center);
            if distance_squared >= radius_squared {
                continue;
            }
            let normalized = (distance_squared.sqrt() / settings.radius).clamp(0.0, 1.0);
            let influence = falloff(settings.falloff, normalized) * travel_amount;
            if influence <= f32::EPSILON {
                continue;
            }
            match settings.operation {
                Operation::Smooth | Operation::Relax => {
                    let previous = original.points[index - 1].position;
                    let next = original.points[index + 1].position;
                    let midpoint = (previous + next) * 0.5;
                    let target = if settings.operation == Operation::Smooth {
                        (previous + point.position * 2.0 + next) * 0.25
                    } else {
                        midpoint
                    };
                    let position = point.position.lerp(target, influence);
                    if !position.is_finite() || position.abs().max_element() > 10_000.0 {
                        return Err(
                            "Sculpt preview would move a point outside document bounds".into()
                        );
                    }
                    stroke.points[index].position = position;
                    stroke_changed |= position != point.position;
                }
                Operation::Thickness => {
                    let sign = if settings.thickness_direction == ThicknessDirection::Increase {
                        1.0
                    } else {
                        -1.0
                    };
                    let source_scale = original.point_thickness_scale(index);
                    let scale = (source_scale * (1.0 + sign * influence)).clamp(0.05, 20.0);
                    let values = stroke
                        .point_thickness
                        .get_or_insert_with(|| vec![1.0; stroke.points.len()]);
                    values[index] = scale;
                    stroke_changed |= scale != source_scale;
                }
            }
        }
        if stroke_changed {
            if matches!(settings.operation, Operation::Smooth | Operation::Relax) {
                stroke.curve = None;
            }
            changed = true;
        }
    }
    if !changed {
        return Err("The sculpt brush did not change any selected interior samples".into());
    }
    Ok(after)
}

fn falloff(falloff: Falloff, normalized_distance: f32) -> f32 {
    let t = normalized_distance.clamp(0.0, 1.0);
    match falloff {
        Falloff::Linear => 1.0 - t,
        Falloff::Smooth => 1.0 - t * t * (3.0 - 2.0 * t),
        Falloff::Sharp => (1.0 - t).powi(2),
    }
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.is_empty() || snapshot.len() > MAX_STROKES {
        return Err("Sculpt selection is empty or exceeds the stroke limit".into());
    }
    let mut point_count = 0usize;
    let mut ids = HashSet::with_capacity(snapshot.len());
    for (_, stroke) in snapshot {
        if !ids.insert(stroke.id) {
            return Err("Sculpt selection contains duplicate stroke IDs".into());
        }
        validate_stroke(stroke)?;
        point_count = point_count.saturating_add(stroke.points.len());
        if point_count > MAX_POINTS {
            return Err("Sculpt selection exceeds the 2 million point limit".into());
        }
    }
    Ok(())
}

fn validate_stroke(stroke: &Stroke) -> Result<(), String> {
    if stroke.id == 0
        || stroke.id == u64::MAX
        || stroke.fill.is_some()
        || stroke.curve.is_some()
        || !stroke.radius.is_finite()
        || !(0.001..=1.0).contains(&stroke.radius)
        || stroke
            .color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || !stroke.brush.is_valid()
        || stroke.points.len() < 3
        || stroke.points.iter().any(|point| !point.is_valid())
        || !stroke.point_thickness_is_valid()
    {
        return Err("Sculpt supports sampled strokes with at least three points; semantic curves and fills are not supported".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point};

    fn stroke(id: u64, y: f32) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id,
            color: [0.3, 0.5, 0.8],
            gradient: None,
            radius: 0.05,
            brush: Brush::default(),
            points: (0..5)
                .map(|i| Point {
                    position: Vec3::new(i as f32 * 0.25, if i == 2 { y } else { 0.0 }, 0.0),
                    pressure: 0.2 + i as f32 * 0.15,
                    tilt: Some(Vec3::Z),
                    time_ms: i * 7,
                })
                .collect(),
            curve: None,
        }
    }

    fn settings(operation: Operation) -> Settings {
        Settings {
            operation,
            center: Vec3::new(0.5, 0.3, 0.0),
            travel: 0.2,
            radius: 1.0,
            strength: 0.8,
            falloff: Falloff::Smooth,
            thickness_direction: ThicknessDirection::Increase,
        }
    }

    #[test]
    fn smooth_and_relax_preserve_endpoints_and_sample_attributes() {
        for operation in [Operation::Smooth, Operation::Relax] {
            let original = stroke(4, 0.8);
            let before = vec![(0, original.clone())];
            let after = preview(&before, settings(operation)).unwrap();
            let result = &after[0].1;
            assert_eq!(result.points[0], original.points[0]);
            assert_eq!(result.points[4], original.points[4]);
            assert_ne!(result.points[2].position, original.points[2].position);
            for index in 0..original.points.len() {
                assert_eq!(
                    result.points[index].pressure,
                    original.points[index].pressure
                );
                assert_eq!(result.points[index].tilt, original.points[index].tilt);
                assert_eq!(result.points[index].time_ms, original.points[index].time_ms);
            }
            assert_eq!(result.brush, original.brush);
        }
    }

    #[test]
    fn thickness_uses_a_dedicated_per_sample_profile_without_changing_pressure() {
        let original = stroke(4, 0.8);
        let before = vec![(0, original.clone())];
        let after = preview(&before, settings(Operation::Thickness)).unwrap();
        let result = &after[0].1;
        assert_eq!(result.points[0], original.points[0]);
        assert_eq!(result.points[4], original.points[4]);
        for index in 1..4 {
            assert_eq!(
                result.points[index].position,
                original.points[index].position
            );
            assert_eq!(
                result.points[index].pressure,
                original.points[index].pressure
            );
            assert_eq!(result.points[index].tilt, original.points[index].tilt);
            assert_eq!(result.points[index].time_ms, original.points[index].time_ms);
        }
        assert_eq!(result.gradient, original.gradient);
        assert_eq!(result.brush, original.brush);
        assert!(result
            .point_thickness
            .as_ref()
            .unwrap()
            .iter()
            .enumerate()
            .any(|(index, scale)| index > 0 && index < 4 && *scale > 1.0));
    }

    #[test]
    fn thickness_profile_changes_textured_mesh_without_changing_pressure_or_timing() {
        let mut original = stroke(4, 0.8);
        original.brush.texture.kind = crate::texture::TextureKind::DryBrush;
        original.brush.texture.strength = 0.5;
        let before = vec![(0, original.clone())];
        let before_mesh = crate::geometry::stroke_mesh(&original);
        let after = preview(&before, settings(Operation::Thickness)).unwrap();
        let result = &after[0].1;
        let after_mesh = crate::geometry::stroke_mesh(result);
        assert_eq!(result.points, original.points);
        assert_eq!(result.brush, original.brush);
        assert_ne!(before_mesh.len(), 0);
        assert!(before_mesh
            .iter()
            .zip(&after_mesh)
            .any(|(a, b)| { Vec3::from(a.position).distance(Vec3::from(b.position)) > 1.0e-5 }));
        assert!(result.point_thickness_is_valid());
    }

    #[test]
    fn profile_remapping_preserves_ordered_resampled_and_partial_paths() {
        let mut original = stroke(4, 0.8);
        original.point_thickness = Some(vec![0.5, 0.75, 1.0, 1.25, 1.5]);
        let remapped = remap_thickness_profile(&original, &original.points[1..4]).unwrap();
        assert_eq!(remapped, vec![0.75, 1.0, 1.25]);
        let resampled = vec![
            original.points[1],
            Point {
                position: (original.points[1].position + original.points[2].position) * 0.5,
                pressure: 0.0,
                tilt: None,
                time_ms: 0,
            },
            original.points[2],
        ];
        let remapped = remap_thickness_profile(&original, &resampled).unwrap();
        assert_eq!(remapped, vec![0.75, 0.875, 1.0]);
    }

    #[test]
    fn preview_is_stationary_bounded_and_frame_rate_independent() {
        let original = stroke(4, 0.8);
        let before = vec![(0, original.clone())];
        let mut stationary = settings(Operation::Smooth);
        stationary.travel = 0.0;
        assert_eq!(preview(&before, stationary).unwrap(), before);
        let mut long = settings(Operation::Relax);
        long.travel = 1000.0;
        let mut capped = long;
        capped.travel = long.radius;
        assert_eq!(
            preview(&before, long).unwrap(),
            preview(&before, capped).unwrap()
        );
    }

    #[test]
    fn selection_rejects_protected_linked_and_semantic_resources_atomically() {
        let document = Document {
            strokes: vec![stroke(4, 0.8)],
            ..Document::default()
        };
        assert!(edit(
            &document,
            &HashSet::from([4, 99]),
            settings(Operation::Smooth)
        )
        .is_err());
        let mut locked = document.clone();
        locked.layers[0].locked = true;
        assert!(edit(&locked, &HashSet::from([4]), settings(Operation::Smooth)).is_err());
        let mut fill = document.clone();
        fill.strokes[0].fill = Some(crate::document::PlanarFill {
            offset: 0.0,
            gradient: None,
            texture: None,
            holes: Vec::new(),
            cylinder: None,
        });
        assert!(edit(&fill, &HashSet::from([4]), settings(Operation::Smooth)).is_err());
        let mut curve = document;
        curve.strokes[0].curve = Some(crate::document::CurveData {
            anchors: vec![],
            frame_origin: Vec3::ZERO,
            frame_normal: Vec3::Z,
            frame_tangent: Vec3::X,
            half_extent: None,
            tension: 0.5,
            surface_target: None,
            shape: None,
        });
        assert!(edit(&curve, &HashSet::from([4]), settings(Operation::Smooth)).is_err());
        let mut linked = Document {
            linked_stroke_groups: crate::document::LinkedStrokeGroups {
                definitions: vec![crate::document::LinkedStrokeGroupDefinition {
                    id: 2,
                    source_stroke_ids: vec![4],
                    pivot: Vec3::ZERO,
                }],
                ..Default::default()
            },
            strokes: vec![stroke(4, 0.8)],
            ..Document::default()
        };
        assert!(edit(&linked, &HashSet::from([4]), settings(Operation::Smooth)).is_err());
        linked.linked_stroke_groups.definitions.clear();
        linked.layers[0].visible = false;
        assert!(edit(&linked, &HashSet::from([4]), settings(Operation::Smooth)).is_err());
    }

    #[test]
    fn thickness_profile_roundtrips_and_exports_the_same_geometry() {
        let mut marked = stroke(4, 0.8);
        marked.point_thickness = Some(vec![0.5, 0.75, 1.5, 1.25, 0.6]);
        marked.brush.texture.kind = crate::texture::TextureKind::DryBrush;
        marked.brush.texture.strength = 0.5;
        let document = Document {
            strokes: vec![marked],
            ..Document::default()
        };
        let encoded_native = serde_json::to_vec(&document).unwrap();
        let native = Document::from_json(std::str::from_utf8(&encoded_native).unwrap()).unwrap();
        assert_eq!(
            native.strokes[0].point_thickness,
            document.strokes[0].point_thickness
        );
        let mut malformed = serde_json::to_value(&document).unwrap();
        malformed["strokes"][0]["point_thickness"] = serde_json::json!([1.0]);
        assert!(Document::from_json(&malformed.to_string()).is_err());
        let mut out_of_range = serde_json::to_value(&document).unwrap();
        out_of_range["strokes"][0]["point_thickness"][2] = serde_json::json!(21.0);
        assert!(Document::from_json(&out_of_range.to_string()).is_err());

        let recovery_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "stroke-sculpt-profile".into(),
            None,
            native.clone(),
        );
        crate::recovery_store::write(recovery_dir.path(), &recovery).unwrap();
        let recovered = crate::recovery_store::scan(recovery_dir.path(), "fresh-reader").unwrap();
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes[0].point_thickness,
            document.strokes[0].point_thickness
        );

        let expected = crate::geometry::stroke_mesh(&native.strokes[0]);
        let bounds = |positions: Vec<[f32; 3]>| {
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for position in positions {
                min = min.min(Vec3::from(position));
                max = max.max(Vec3::from(position));
            }
            (min, max)
        };
        let expected_bounds = bounds(expected.iter().map(|vertex| vertex.position).collect());

        let glb = crate::export::encode(&native, None).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb.bytes).unwrap();
        let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
        let glb_bounds = bounds(
            primitive
                .reader(|_| parsed.blob.as_deref())
                .read_positions()
                .unwrap()
                .collect(),
        );
        assert!(glb_bounds.0.abs_diff_eq(expected_bounds.0, 1.0e-5));
        assert!(glb_bounds.1.abs_diff_eq(expected_bounds.1, 1.0e-5));

        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let obj_path = root.path().join("profile.obj");
        crate::obj_export::write_obj_bundle(&native, None, &obj_path, false).unwrap();
        let obj_positions: Vec<[f32; 3]> = std::fs::read_to_string(&obj_path)
            .unwrap()
            .lines()
            .filter_map(|line| {
                let values: Vec<f32> = line
                    .strip_prefix("v ")?
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<Vec<f32>, _>>()
                    .ok()?;
                (values.len() == 3).then(|| [values[0], values[1], values[2]])
            })
            .collect();
        let obj_bounds = bounds(obj_positions);
        assert!(obj_bounds.0.abs_diff_eq(expected_bounds.0, 1.0e-5));
        assert!(obj_bounds.1.abs_diff_eq(expected_bounds.1, 1.0e-5));

        let legacy = stroke(9, 0.0);
        let legacy_value = serde_json::to_value(legacy).unwrap();
        assert!(legacy_value.get("point_thickness").is_none());
        let legacy_loaded = Document::from_json(
            &serde_json::to_string(&Document {
                strokes: vec![serde_json::from_value(legacy_value).unwrap()],
                ..Document::default()
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(legacy_loaded.strokes[0].point_thickness, None);
    }
}
