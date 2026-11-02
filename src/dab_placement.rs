//! Deterministic, distance-based dab placement for editable strokes.
//!
//! Version 1 measures spacing along the captured world-space centerline in
//! multiples of the stroke's base radius. Dab indices are global to the
//! original stroke, so splitting a stroke preserves its spacing and random
//! sequence. The default endpoint rule places dabs at index zero and each
//! following spacing interval; it does not snap an extra dab to pen-up.

use crate::{
    document::{Document, Stroke},
    dynamics,
};
use glam::Vec3;

pub const MAX_DABS_PER_STROKE: usize = 8_192;
pub const MAX_DOCUMENT_DABS: usize = 50_000;
const DISTANCE_EPSILON: f32 = 1.0e-5;

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
    let first = ((origin - DISTANCE_EPSILON).max(0.0) / step).ceil();
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
            point.position,
            stroke.brush.tangent,
            point.pressure,
            point.tilt,
            0.0,
            total,
            0.0,
            0,
        ));
        return Ok(out);
    }
    let first = ((origin - DISTANCE_EPSILON).max(0.0) / step).ceil() as u64;
    let last = ((origin + local_length + DISTANCE_EPSILON) / step).floor() as u64;
    for index in first..=last {
        let global_distance = index as f32 * step;
        let local_distance = (global_distance - origin).clamp(0.0, local_length);
        let (position, pressure, tangent, tilt) = sample_path(stroke, &distances, local_distance);
        out.push(resolve_dab(
            stroke,
            position,
            tangent,
            pressure,
            tilt,
            global_distance,
            total,
            if local_length > DISTANCE_EPSILON {
                (local_distance / local_length).clamp(0.0, 1.0)
            } else {
                0.0
            },
            index,
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

fn resolve_dab(
    stroke: &Stroke,
    center: Vec3,
    tangent: Vec3,
    pressure: f32,
    tilt: Option<Vec3>,
    global_distance: f32,
    total: f32,
    gradient_t: f32,
    index: u64,
) -> ResolvedDab {
    let normal = (stroke.brush.normal - tangent * stroke.brush.normal.dot(tangent))
        .try_normalize()
        .or_else(|| {
            (stroke.brush.tangent - tangent * stroke.brush.tangent.dot(tangent)).try_normalize()
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
        // The brush frame is fixed to the captured drawing plane. The input
        // handler disables scatter on curved and model-surface targets.
        let plane_tangent = (stroke.brush.tangent
            - stroke.brush.normal * stroke.brush.tangent.dot(stroke.brush.normal))
        .try_normalize()
        .unwrap_or(tangent);
        let plane_side = stroke
            .brush
            .normal
            .cross(plane_tangent)
            .try_normalize()
            .unwrap_or(cross);
        (plane_tangent * scatter_angle.cos() + plane_side * scatter_angle.sin()) * scatter_radius
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
        size_scale: random_size * pressure_size * pressure_width * tilt_width * taper,
        angle_degrees: angle,
        gradient_t,
    }
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
