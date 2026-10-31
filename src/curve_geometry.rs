//! Bounded cubic curve geometry for SKETCH-040.
//!
//! Curve controls are stored in world space. Callers should convert a desired
//! screen-space quality into world units with `Camera::world_per_pixel` and
//! pass that as `tolerance_world`; this keeps the tessellation stable across
//! viewport sizes and camera projections. Generated strokes are ordinary
//! editable ink. Pressure is interpolated linearly in Bézier parameter space;
//! width remains the stroke's captured radius multiplied by its normal brush
//! pressure dynamics. This module does not invent tilt samples.
#![allow(dead_code)] // Geometry helpers also define the core API for future curve editing.

use crate::document::{Point, Stroke};
use glam::Vec3;

pub const MIN_TOLERANCE_WORLD: f32 = 0.00001;
pub const MAX_TOLERANCE_WORLD: f32 = 100.0;
pub const MAX_CURVE_POINTS: usize = 4096;
const MAX_RECURSION_DEPTH: u8 = 18;

/// One cubic Bézier segment with absolute world-space control points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    pub p0: Vec3,
    pub p1: Vec3,
    pub p2: Vec3,
    pub p3: Vec3,
}

impl CubicBezier {
    pub fn is_valid(self) -> bool {
        [self.p0, self.p1, self.p2, self.p3]
            .into_iter()
            .all(|point| point.is_finite() && point.abs().max_element() <= 10_000.0)
    }

    pub fn evaluate(self, t: f32) -> Result<Vec3, String> {
        if !self.is_valid() || !t.is_finite() || !(0.0..=1.0).contains(&t) {
            return Err(
                "Curve evaluation needs finite controls and a parameter from 0 to 1".into(),
            );
        }
        let one_minus = 1.0 - t;
        Ok(self.p0 * one_minus.powi(3)
            + self.p1 * (3.0 * one_minus.powi(2) * t)
            + self.p2 * (3.0 * one_minus * t.powi(2))
            + self.p3 * t.powi(3))
    }

    pub fn derivative(self, t: f32) -> Result<Vec3, String> {
        if !self.is_valid() || !t.is_finite() || !(0.0..=1.0).contains(&t) {
            return Err("Curve tangent needs finite controls and a parameter from 0 to 1".into());
        }
        let one_minus = 1.0 - t;
        Ok((self.p1 - self.p0) * (3.0 * one_minus * one_minus)
            + (self.p2 - self.p1) * (6.0 * one_minus * t)
            + (self.p3 - self.p2) * (3.0 * t * t))
    }

    /// Exact De Casteljau split. The returned segments share one anchor.
    pub fn split(self, t: f32) -> Result<(Self, Self), String> {
        if !self.is_valid() || !t.is_finite() || !(0.0..=1.0).contains(&t) {
            return Err("Curve split needs finite controls and a parameter from 0 to 1".into());
        }
        let p01 = self.p0.lerp(self.p1, t);
        let p12 = self.p1.lerp(self.p2, t);
        let p23 = self.p2.lerp(self.p3, t);
        let p012 = p01.lerp(p12, t);
        let p123 = p12.lerp(p23, t);
        let anchor = p012.lerp(p123, t);
        Ok((
            Self {
                p0: self.p0,
                p1: p01,
                p2: p012,
                p3: anchor,
            },
            Self {
                p0: anchor,
                p1: p123,
                p2: p23,
                p3: self.p3,
            },
        ))
    }

    /// Approximate a pair of adjacent segments with one cubic while preserving
    /// the endpoint tangent directions. `tension` scales handle length: 1 is
    /// the usual one-third-chord estimate, 0 is a straight chord.
    pub fn join(left: Self, right: Self, tension: f32) -> Result<Self, String> {
        if !left.is_valid()
            || !right.is_valid()
            || !tension.is_finite()
            || !(0.0..=2.0).contains(&tension)
            || left.p3.distance(right.p0) > 0.001
        {
            return Err(
                "Joining curves needs adjacent valid segments and tension from 0 to 2".into(),
            );
        }
        let chord = right.p3 - left.p0;
        let handle = chord.length() * tension / 3.0;
        let start_tangent = (left.p1 - left.p0)
            .try_normalize()
            .or_else(|| (left.p3 - left.p0).try_normalize())
            .unwrap_or(Vec3::ZERO);
        let end_tangent = (right.p3 - right.p2)
            .try_normalize()
            .or_else(|| (right.p3 - right.p0).try_normalize())
            .unwrap_or(Vec3::ZERO);
        let joined = Self {
            p0: left.p0,
            p1: left.p0 + start_tangent * handle,
            p2: right.p3 - end_tangent * handle,
            p3: right.p3,
        };
        if joined.is_valid() {
            Ok(joined)
        } else {
            Err("Joined curve exceeds document bounds".into())
        }
    }
}

/// Endpoint pressure attached to one cubic segment. Tessellation linearly
/// interpolates this value in parameter space; normal brush dynamics then map
/// it to width/opacity consistently with hand-drawn strokes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressureSegment {
    pub curve: CubicBezier,
    pub pressure_start: f32,
    pub pressure_end: f32,
}

impl PressureSegment {
    fn is_valid(self) -> bool {
        self.curve.is_valid()
            && self.pressure_start.is_finite()
            && self.pressure_end.is_finite()
            && (0.0..=1.0).contains(&self.pressure_start)
            && (0.0..=1.0).contains(&self.pressure_end)
    }
}

/// Adaptive flattening for one cubic segment. The result includes both exact
/// endpoints, never exceeds `max_points`, and rejects controls that cannot
/// meet the requested tolerance within the bounded recursion depth.
pub fn tessellate_cubic(
    curve: CubicBezier,
    tolerance_world: f32,
    max_points: usize,
) -> Result<Vec<Vec3>, String> {
    validate_tessellation(tolerance_world, max_points)?;
    if !curve.is_valid() {
        return Err("Curve control points must be finite and inside document bounds".into());
    }
    let samples = tessellate_with_parameters(curve, tolerance_world, max_points)?;
    Ok(samples.into_iter().map(|(_, point)| point).collect())
}

/// Convert a continuous sequence of pressure-aware cubic segments into one
/// ordinary editable stroke. Adjacent anchors and pressure values must agree;
/// duplicate join samples are stored once.
pub fn tessellate_stroke(
    segments: &[PressureSegment],
    prototype: &Stroke,
    tolerance_world: f32,
    max_points: usize,
) -> Result<Stroke, String> {
    validate_tessellation(tolerance_world, max_points)?;
    if segments.is_empty() {
        return Err("Add at least one curve segment".into());
    }
    if segments.len() > max_points {
        return Err("Curve has too many segments".into());
    }
    if !prototype.brush.is_valid()
        || !prototype.radius.is_finite()
        || !(0.001..=1.0).contains(&prototype.radius)
        || prototype
            .color
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err("Curve stroke brush, radius, or color is invalid".into());
    }

    let mut points: Vec<Point> = Vec::new();
    for (index, segment) in segments.iter().copied().enumerate() {
        if !segment.is_valid() {
            return Err("Curve controls and endpoint pressures must be valid".into());
        }
        if index > 0 {
            let previous = segments[index - 1];
            if previous.curve.p3.distance(segment.curve.p0) > tolerance_world
                || (previous.pressure_end - segment.pressure_start).abs() > 0.001
            {
                return Err(
                    "Curve segments must share position and pressure at each anchor".into(),
                );
            }
        }
        let remaining = max_points.saturating_sub(points.len()).saturating_add(1);
        let samples = tessellate_with_parameters(segment.curve, tolerance_world, remaining.max(2))?;
        for (sample_index, (t, position)) in samples.into_iter().enumerate() {
            if index > 0 && sample_index == 0 {
                continue;
            }
            if points.len() >= max_points {
                return Err("Curve tessellation exceeds the point limit".into());
            }
            let pressure =
                segment.pressure_start + (segment.pressure_end - segment.pressure_start) * t;
            let point = Point {
                position,
                pressure,
                tilt: None,
            };
            if !point.is_valid() {
                return Err("Curve tessellation exceeds point or pressure limits".into());
            }
            points.push(point);
        }
    }
    if points.len() < 2 {
        return Err("Curve needs at least two distinct samples".into());
    }
    if points
        .windows(2)
        .any(|pair| pair[0].position.distance_squared(pair[1].position) <= 1e-12)
    {
        return Err("Curve contains adjacent duplicate samples".into());
    }
    let mut stroke = prototype.clone();
    stroke.fill = None;
    stroke.points = points;
    Ok(stroke)
}

/// Compute symmetric handle offsets for a smooth anchor. Endpoints use the
/// available one-sided direction; interior handles lie along the chord from
/// previous to next. Offsets are world-space vectors relative to `position`.
pub fn smooth_handle_offsets(
    previous: Option<Vec3>,
    position: Vec3,
    next: Option<Vec3>,
    tension: f32,
) -> Result<(Vec3, Vec3), String> {
    if !position.is_finite()
        || previous.is_some_and(|point| !point.is_finite())
        || next.is_some_and(|point| !point.is_finite())
        || !tension.is_finite()
        || !(0.0..=2.0).contains(&tension)
    {
        return Err("Smooth anchor needs finite points and tension from 0 to 2".into());
    }
    let direction = match (previous, next) {
        (Some(before), Some(after)) => (after - before).try_normalize(),
        (Some(before), None) => (position - before).try_normalize(),
        (None, Some(after)) => (after - position).try_normalize(),
        (None, None) => None,
    }
    .unwrap_or(Vec3::ZERO);
    let scale = match (previous, next) {
        (Some(before), Some(after)) => before.distance(position).min(position.distance(after)),
        (Some(before), None) => before.distance(position),
        (None, Some(after)) => position.distance(after),
        (None, None) => 0.0,
    } * tension
        / 3.0;
    let handle = direction * scale;
    Ok((-handle, handle))
}

/// Move one relative handle and optionally keep the opposite handle aligned.
/// The aligned mode preserves the opposite handle's existing length.
pub fn move_handle(
    incoming: Vec3,
    outgoing: Vec3,
    move_incoming: bool,
    new_offset: Vec3,
    aligned: bool,
) -> Result<(Vec3, Vec3), String> {
    if !incoming.is_finite() || !outgoing.is_finite() || !new_offset.is_finite() {
        return Err("Curve handles must be finite".into());
    }
    if move_incoming {
        let opposite_length = outgoing.length();
        let new_outgoing = if aligned {
            new_offset
                .try_normalize()
                .map_or(outgoing, |axis| -axis * opposite_length)
        } else {
            outgoing
        };
        Ok((new_offset, new_outgoing))
    } else {
        let opposite_length = incoming.length();
        let new_incoming = if aligned {
            new_offset
                .try_normalize()
                .map_or(incoming, |axis| -axis * opposite_length)
        } else {
            incoming
        };
        Ok((new_incoming, new_offset))
    }
}

fn validate_tessellation(tolerance_world: f32, max_points: usize) -> Result<(), String> {
    if !tolerance_world.is_finite()
        || !(MIN_TOLERANCE_WORLD..=MAX_TOLERANCE_WORLD).contains(&tolerance_world)
    {
        return Err(format!(
            "Curve tolerance must be {MIN_TOLERANCE_WORLD}–{MAX_TOLERANCE_WORLD} world units"
        ));
    }
    if !(2..=MAX_CURVE_POINTS).contains(&max_points) {
        return Err(format!("Curve point limit must be 2–{MAX_CURVE_POINTS}"));
    }
    Ok(())
}

fn flatten(
    curve: CubicBezier,
    tolerance: f32,
    depth: u8,
    t0: f32,
    t1: f32,
    max_points: usize,
    output: &mut Vec<(f32, Vec3)>,
) -> Result<(), String> {
    if flatness(curve) <= tolerance {
        if output.len() >= max_points {
            return Err("Curve tessellation exceeds the point limit".into());
        }
        output.push((t1, curve.p3));
        return Ok(());
    }
    if depth >= MAX_RECURSION_DEPTH {
        return Err(
            "Curve could not meet the requested tolerance within the subdivision limit".into(),
        );
    }
    let (left, right) = curve.split(0.5)?;
    let middle = (t0 + t1) * 0.5;
    flatten(left, tolerance, depth + 1, t0, middle, max_points, output)?;
    flatten(right, tolerance, depth + 1, middle, t1, max_points, output)
}

fn tessellate_with_parameters(
    curve: CubicBezier,
    tolerance_world: f32,
    max_points: usize,
) -> Result<Vec<(f32, Vec3)>, String> {
    let mut result = vec![(0.0, curve.p0)];
    flatten(curve, tolerance_world, 0, 0.0, 1.0, max_points, &mut result)?;
    Ok(result)
}

fn flatness(curve: CubicBezier) -> f32 {
    point_line_distance(curve.p1, curve.p0, curve.p3)
        .max(point_line_distance(curve.p2, curve.p0, curve.p3))
}

fn point_line_distance(point: Vec3, start: Vec3, end: Vec3) -> f32 {
    let chord = end - start;
    if chord.length_squared() <= 1e-12 {
        return point.distance(start);
    }
    let t = ((point - start).dot(chord) / chord.length_squared()).clamp(0.0, 1.0);
    point.distance(start + chord * t)
}
