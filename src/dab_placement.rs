//! Deterministic, distance-based dab placement for editable strokes.
//!
//! Version 1 measures spacing along the captured world-space centerline in
//! multiples of the stroke's base radius. Dab indices are global to the
//! original stroke, so splitting a stroke preserves its spacing and random
//! sequence. The default endpoint rule places dabs at index zero and each
//! following spacing interval; fragments exclude repeated dabs at exact phase-aligned starts.

use crate::{
    document::{Document, Stroke},
    dynamics,
};
use glam::Vec3;

pub const MAX_DABS_PER_STROKE: usize = 8_192;
pub const MAX_DOCUMENT_DABS: usize = 50_000;
const DISTANCE_EPSILON: f32 = 1.0e-5;
const PHASE_INDEX_EPSILON: f32 = 1.0e-4;

fn first_dab_index(origin: f32, step: f32) -> f32 {
    if origin <= DISTANCE_EPSILON {
        return 0.0;
    }
    let phase = origin / step;
    let nearest = phase.round();
    if (phase - nearest).abs() <= PHASE_INDEX_EPSILON {
        nearest + 1.0
    } else {
        phase.ceil()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedDab {
    pub position: Vec3,
    pub tangent: Vec3,
    pub normal: Vec3,
    pub pressure: f32,
    pub tilt: Option<Vec3>,
    /// Includes pressure response, taper, and deterministic size variation.
    pub size_scale: f32,
    pub angle_degrees: f32,
    /// Normalized centerline distance for stroke color ramps.
    pub gradient_t: f32,
}

struct DabParameters {
    center: Vec3,
    tangent: Vec3,
    pressure: f32,
    tilt: Option<Vec3>,
    global_distance: f32,
    total: f32,
    gradient_t: f32,
    point_thickness: f32,
    index: u64,
}

/// A stable seed derived from a unique stroke ID. Split fragments retain it.
pub fn seed_for_stroke(id: u64) -> u64 {
    splitmix64(id ^ 0x534b_4554_4348_3301)
}

pub fn document_count(document: &Document) -> Result<usize, String> {
    let mut total = 0usize;
    for stroke in &document.strokes {
        total = total
            .checked_add(point_count(stroke)?)
            .ok_or("Dab placement count overflow")?;
        if total > MAX_DOCUMENT_DABS {
            return Err("Document exceeds the dab geometry budget".into());
        }
    }
    Ok(total)
}

pub fn point_count(stroke: &Stroke) -> Result<usize, String> {
    if !stroke.brush.dabs.enabled || stroke.points.is_empty() {
        return Ok(0);
    }
    if !stroke.brush.dabs.is_valid()
        || !stroke.radius.is_finite()
        || stroke.radius <= 0.0
        || !stroke.brush.dab_distance_origin.is_finite()
        || stroke.brush.dab_distance_origin < 0.0
    {
        return Err("Invalid dab placement settings".into());
    }
    let distances = dynamics::arc_lengths(&stroke.points);
    let local_length = distances.last().copied().unwrap_or(0.0);
    let origin = stroke.brush.dab_distance_origin;
    let total = if stroke.brush.dab_total_length > 0.0 {
        stroke.brush.dab_total_length
    } else {
        origin + local_length
    };
    let step = stroke.radius * stroke.brush.dabs.spacing;
    if !local_length.is_finite() || !total.is_finite() || !step.is_finite() || step <= 0.0 {
        return Err("Dab path exceeds the supported distance range".into());
    }
    if local_length <= DISTANCE_EPSILON {
        return Ok(usize::from(origin <= DISTANCE_EPSILON));
    }
    let first = first_dab_index(origin, step);
    let last = ((origin + local_length + DISTANCE_EPSILON) / step).floor();
    if !first.is_finite() || !last.is_finite() || last < first {
        return Ok(0);
    }
    let count = (last - first + 1.0) as usize;
    if count > MAX_DABS_PER_STROKE {
        return Err(format!(
            "Dab stroke exceeds the {MAX_DABS_PER_STROKE} placement limit"
        ));
    }
    Ok(count)
}

/// Resolve one deterministic dab at every global spacing landmark.
pub fn sample(stroke: &Stroke) -> Result<Vec<ResolvedDab>, String> {
    let count = point_count(stroke)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let distances = dynamics::arc_lengths(&stroke.points);
    let local_length = distances.last().copied().unwrap_or(0.0);
    let origin = stroke.brush.dab_distance_origin;
    let total = if stroke.brush.dab_total_length > 0.0 {
        stroke.brush.dab_total_length
    } else {
        origin + local_length
    };
    let step = stroke.radius * stroke.brush.dabs.spacing;
    let mut out = Vec::with_capacity(count);
    if local_length <= DISTANCE_EPSILON {
        let point = stroke.points[0];
        out.push(resolve_dab(
            stroke,
            DabParameters {
                center: point.position,
                tangent: stroke.brush.tangent,
                pressure: point.pressure,
                tilt: point.tilt,
                global_distance: 0.0,
                total,
                gradient_t: 0.0,
                point_thickness: stroke.point_thickness_scale(0),
                index: 0,
            },
        ));
        return Ok(out);
    }
    let first = first_dab_index(origin, step) as u64;
    let last = ((origin + local_length + DISTANCE_EPSILON) / step).floor() as u64;
    for index in first..=last {
        let global_distance = index as f32 * step;
        let local_distance = (global_distance - origin).clamp(0.0, local_length);
        let (position, pressure, tangent, tilt) = sample_path(stroke, &distances, local_distance);
        out.push(resolve_dab(
            stroke,
            DabParameters {
                center: position,
                tangent,
                pressure,
                tilt,
                global_distance,
                total,
                gradient_t: if local_length > DISTANCE_EPSILON {
                    (local_distance / local_length).clamp(0.0, 1.0)
                } else {
                    0.0
                },
                point_thickness: point_thickness_at_distance(stroke, &distances, local_distance),
                index,
            },
        ));
    }
    Ok(out)
}

fn sample_path(
    stroke: &Stroke,
    distances: &[f32],
    distance: f32,
) -> (Vec3, f32, Vec3, Option<Vec3>) {
    let points = &stroke.points;
    if points.len() == 1 {
        return (
            points[0].position,
            points[0].pressure,
            stroke.brush.tangent,
            points[0].tilt,
        );
    }
    let upper = distances.partition_point(|value| *value < distance);
    let segment = upper.saturating_sub(1).min(points.len() - 2);
    let length = distances[segment + 1] - distances[segment];
    let t = if length > DISTANCE_EPSILON {
        ((distance - distances[segment]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let a = points[segment];
    let b = points[segment + 1];
    let mut tangent = (b.position - a.position)
        .try_normalize()
        .unwrap_or(stroke.brush.tangent);
    if (distance - distances[segment + 1]).abs() <= DISTANCE_EPSILON && segment + 2 < points.len() {
        let outgoing = (points[segment + 2].position - b.position)
            .try_normalize()
            .unwrap_or(tangent);
        let bisector = tangent + outgoing;
        // At a 180-degree reversal there is no stable bisector. Keep the
        // incoming direction so the dab axis does not flip arbitrarily.
        tangent = bisector.try_normalize().unwrap_or(tangent);
    }
    (
        a.position.lerp(b.position, t),
        a.pressure + (b.pressure - a.pressure) * t,
        tangent,
        dynamics::interpolate_tilt(a.tilt, b.tilt, t),
    )
}

fn resolve_dab(stroke: &Stroke, parameters: DabParameters) -> ResolvedDab {
    let DabParameters {
        center,
        tangent,
        pressure,
        tilt,
        global_distance,
        total,
        gradient_t,
        point_thickness,
        index,
    } = parameters;
    let target = stroke
        .brush
        .dabs
        .surface_target
        .map(|target| target.surface());
    let normal = target
        .and_then(|surface| surface.normal_at(center))
        .or_else(|| {
            (stroke.brush.normal - tangent * stroke.brush.normal.dot(tangent))
                .try_normalize()
                .or_else(|| {
                    (stroke.brush.tangent - tangent * stroke.brush.tangent.dot(tangent))
                        .try_normalize()
                })
        })
        .unwrap_or(Vec3::Z);
    let tangent = (tangent - normal * tangent.dot(normal))
        .try_normalize()
        .unwrap_or(stroke.brush.tangent);
    let cross = normal.cross(tangent).try_normalize().unwrap_or(Vec3::Y);

    let size_random = random01(stroke.brush.dab_seed, index, 0);
    let angle_random = random01(stroke.brush.dab_seed, index, 1);
    let scatter_radius = stroke.radius
        * stroke.brush.dabs.scatter
        * random01(stroke.brush.dab_seed, index, 2).sqrt();
    let scatter_angle = random01(stroke.brush.dab_seed, index, 3) * std::f32::consts::TAU;
    let scatter = if stroke.brush.dabs.scatter > 0.0 {
        let plane_tangent = (stroke.brush.tangent
            - stroke.brush.normal * stroke.brush.tangent.dot(stroke.brush.normal))
            - normal
                * ((stroke.brush.tangent
                    - stroke.brush.normal * stroke.brush.tangent.dot(stroke.brush.normal))
                .dot(normal));
        let plane_tangent = plane_tangent.try_normalize().unwrap_or(tangent);
        let plane_side = stroke
            .brush
            .normal
            .cross(plane_tangent)
            .try_normalize()
            .unwrap_or(cross);
        let offset = (plane_tangent * scatter_angle.cos() + plane_side * scatter_angle.sin())
            * scatter_radius;
        if let Some(surface) = target {
            surface.offset_tangent(center, offset).unwrap_or(center) - center
        } else {
            offset
        }
    } else {
        Vec3::ZERO
    };

    let variation = stroke.brush.dabs.size_variation;
    let random_size = (1.0 - variation) + size_random * variation * 2.0;
    let pressure_size = dynamics::pressure_scale(stroke.brush.dabs.pressure_size, pressure);
    let pressure_width = dynamics::pressure_scale(stroke.brush.dynamics.width, pressure);
    let tilt_width =
        dynamics::shading_width_scale(stroke.brush.dynamics, stroke.brush.texture.kind, tilt);
    let taper = dynamics::taper_factor(
        global_distance,
        total,
        stroke.brush.dynamics.taper_start,
        stroke.brush.dynamics.taper_end,
    );
    let angle = (angle_random * 2.0 - 1.0) * stroke.brush.dabs.angle_variation_degrees;
    ResolvedDab {
        position: center + scatter,
        tangent,
        normal,
        pressure,
        tilt,
        size_scale: random_size
            * pressure_size
            * pressure_width
            * tilt_width
            * taper
            * point_thickness,
        angle_degrees: angle,
        gradient_t,
    }
}

fn point_thickness_at_distance(stroke: &Stroke, distances: &[f32], distance: f32) -> f32 {
    let Some(values) = stroke.point_thickness.as_ref() else {
        return 1.0;
    };
    if values.len() != stroke.points.len() || values.is_empty() {
        return 1.0;
    }
    if values.len() == 1 {
        return values[0];
    }
    let upper = distances.partition_point(|value| *value < distance);
    let segment = upper.saturating_sub(1).min(values.len() - 2);
    let span = distances[segment + 1] - distances[segment];
    let t = if span > 1.0e-8 {
        ((distance - distances[segment]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    values[segment] + (values[segment + 1] - values[segment]) * t
}

fn random01(seed: u64, index: u64, channel: u64) -> f32 {
    let bits = splitmix64(seed ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ channel);
    (bits >> 40) as f32 * (1.0 / 16_777_216.0)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut mixed = value;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Brush, Point, Stroke},
        dynamics::DabSettings,
    };

    fn stroke(samples: &[(Vec3, f32)], radius: f32, spacing: f32) -> Stroke {
        let brush = Brush {
            tangent: Vec3::X,
            normal: Vec3::Z,
            dabs: DabSettings {
                enabled: true,
                spacing,
                scatter: 0.0,
                size_variation: 0.0,
                angle_variation_degrees: 0.0,
                ..Default::default()
            },
            dab_seed: seed_for_stroke(17),
            ..Default::default()
        };
        Stroke {
            id: 17,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            color: [0.2, 0.6, 0.9],
            gradient: None,
            radius,
            brush,
            points: samples
                .iter()
                .map(|(position, pressure)| Point {
                    position: *position,
                    pressure: *pressure,
                    tilt: None,
                    time_ms: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn taps_and_sub_spacing_strokes_place_only_the_start_dab() {
        let tap = stroke(&[(Vec3::new(2.0, 3.0, 4.0), 0.7)], 0.1, 0.5);
        let tap_dabs = sample(&tap).unwrap();
        assert_eq!(tap_dabs.len(), 1);
        assert_eq!(tap_dabs[0].position, Vec3::new(2.0, 3.0, 4.0));

        let short = stroke(
            &[(Vec3::ZERO, 1.0), (Vec3::new(0.04, 0.0, 0.0), 1.0)],
            0.1,
            0.5,
        );
        let short_dabs = sample(&short).unwrap();
        assert_eq!(short_dabs.len(), 1, "no extra endpoint dab is forced");
        assert_eq!(short_dabs[0].position, Vec3::ZERO);
    }

    #[test]
    fn stabilized_release_tail_adds_only_crossed_spacing_landmarks() {
        let stroke_with_tail = stroke(
            &[
                (Vec3::ZERO, 1.0),
                (Vec3::new(0.04, 0.0, 0.0), 1.0),
                (Vec3::new(0.11, 0.0, 0.0), 1.0),
            ],
            0.1,
            0.5,
        );
        let dabs = sample(&stroke_with_tail).unwrap();
        assert_eq!(dabs.len(), 3);
        assert!(dabs[0].position.distance(Vec3::ZERO) <= 1.0e-6);
        assert!(dabs[1].position.distance(Vec3::new(0.05, 0.0, 0.0)) <= 1.0e-6);
        assert!(dabs[2].position.distance(Vec3::new(0.1, 0.0, 0.0)) <= 1.0e-6);
        assert!(dabs.iter().all(|dab| dab.position.x < 0.11));
    }

    #[test]
    fn equivalent_resampling_preserves_dab_centers_and_random_values() {
        let corners = [
            (Vec3::ZERO, 0.2),
            (Vec3::new(0.5, 0.0, 0.0), 0.9),
            (Vec3::new(0.5, 0.5, 0.0), 0.4),
        ];
        let mut sparse = stroke(&corners, 0.25, 0.5);
        sparse.brush.dabs.scatter = 0.3;
        sparse.brush.dabs.size_variation = 0.4;
        sparse.brush.dabs.angle_variation_degrees = 35.0;
        let mut dense_samples = Vec::new();
        for segment in corners.windows(2) {
            for step in 0..4 {
                let t = step as f32 / 4.0;
                dense_samples.push((
                    segment[0].0.lerp(segment[1].0, t),
                    segment[0].1 + (segment[1].1 - segment[0].1) * t,
                ));
            }
        }
        dense_samples.push(*corners.last().unwrap());
        let mut dense = stroke(&dense_samples, 0.25, 0.5);
        dense.brush.dabs.scatter = sparse.brush.dabs.scatter;
        dense.brush.dabs.size_variation = sparse.brush.dabs.size_variation;
        dense.brush.dabs.angle_variation_degrees = sparse.brush.dabs.angle_variation_degrees;

        let sparse_dabs = sample(&sparse).unwrap();
        let dense_dabs = sample(&dense).unwrap();
        assert_eq!(sparse_dabs.len(), dense_dabs.len());
        for (left, right) in sparse_dabs.iter().zip(&dense_dabs) {
            assert!(left.position.distance(right.position) <= 1.0e-5);
            assert!((left.pressure - right.pressure).abs() <= 1.0e-5);
            assert!((left.size_scale - right.size_scale).abs() <= 1.0e-5);
            assert!((left.angle_degrees - right.angle_degrees).abs() <= 1.0e-5);
            assert!((left.gradient_t - right.gradient_t).abs() <= 1.0e-5);
        }
    }

    #[test]
    fn exact_reversal_keeps_the_incoming_tangent_and_duplicate_samples_are_safe() {
        let path = stroke(
            &[
                (Vec3::ZERO, 1.0),
                (Vec3::ZERO, 1.0),
                (Vec3::new(0.3, 0.0, 0.0), 1.0),
                (Vec3::new(0.3, 0.0, 0.0), 1.0),
                (Vec3::ZERO, 1.0),
            ],
            0.1,
            3.0,
        );
        let dabs = sample(&path).unwrap();
        assert_eq!(dabs.len(), 3);
        assert!(dabs.iter().all(|dab| dab.position.is_finite()));
        assert!(dabs[1].tangent.dot(Vec3::X) > 0.999);
    }

    #[test]
    fn native_roundtrip_keeps_the_resolved_dab_sequence() {
        let mut original = stroke(
            &[(Vec3::ZERO, 0.3), (Vec3::new(0.75, 0.0, 0.0), 0.8)],
            0.1,
            0.5,
        );
        original.brush.dabs.scatter = 0.25;
        original.brush.dabs.size_variation = 0.2;
        original.brush.dabs.angle_variation_degrees = 20.0;
        let before = sample(&original).unwrap();
        let bytes = serde_json::to_vec(&original).unwrap();
        let reloaded: Stroke = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(before, sample(&reloaded).unwrap());
    }

    #[test]
    fn generated_mesh_uses_exactly_the_resolved_dab_count() {
        let path = stroke(
            &[(Vec3::ZERO, 1.0), (Vec3::new(0.3, 0.0, 0.0), 1.0)],
            0.1,
            0.5,
        );
        let dabs = sample(&path).unwrap();
        let mesh = crate::geometry::stroke_mesh(&path);
        assert_eq!(mesh.len(), dabs.len() * 24);
        assert!(mesh.iter().all(|vertex| {
            Vec3::from(vertex.position).is_finite() && Vec3::from(vertex.normal).is_finite()
        }));
    }
}
