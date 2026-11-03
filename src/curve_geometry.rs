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
                time_ms: 0,
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

/// Tessellate Bézier controls stored in an unwrapped cylinder chart while
/// measuring subdivision error in world space. Curve samples are mapped back
/// to the captured finite cylinder side and never cross its open ends.
pub fn tessellate_cylinder_stroke(
    segments: &[PressureSegment],
    prototype: &Stroke,
    cylinder: crate::surface::Cylinder,
    tolerance_world: f32,
    max_points: usize,
) -> Result<Stroke, String> {
    validate_tessellation(tolerance_world, max_points)?;
    if !cylinder.valid() || segments.is_empty() || segments.len() > max_points {
        return Err("Cylinder curve target or segment count is invalid".into());
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
    let mut points = Vec::new();
    for (index, segment) in segments.iter().copied().enumerate() {
        if !segment.is_valid()
            || segment.curve.p0.z.abs() > 1e-4
            || segment.curve.p1.z.abs() > 1e-4
            || segment.curve.p2.z.abs() > 1e-4
            || segment.curve.p3.z.abs() > 1e-4
            || [
                segment.curve.p0,
                segment.curve.p1,
                segment.curve.p2,
                segment.curve.p3,
            ]
            .iter()
            .any(|point| point.y.abs() > cylinder.height * 0.5)
        {
            return Err("Cylinder curve controls extend beyond the finite surface".into());
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
        let map = |point: Vec3| cylinder.point_from_parameter(point);
        let mut samples = vec![(
            0.0,
            map(segment.curve.p0).ok_or("Cylinder curve start is outside its surface")?,
        )];
        flatten_cylinder(
            segment.curve,
            cylinder,
            tolerance_world,
            0,
            0.0,
            1.0,
            max_points.saturating_sub(points.len()).max(2),
            &mut samples,
        )?;
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
                time_ms: 0,
            };
            if !point.is_valid() {
                return Err("Curve tessellation exceeds point or pressure limits".into());
            }
            points.push(point);
        }
    }
    if points.len() < 2
        || points
            .windows(2)
            .any(|pair| pair[0].position.distance_squared(pair[1].position) <= 1e-12)
    {
        return Err("Curve needs at least two distinct samples".into());
    }
    let mut stroke = prototype.clone();
    stroke.fill = None;
    stroke.points = points;
    Ok(stroke)
}

fn flatten_cylinder(
    curve: CubicBezier,
    cylinder: crate::surface::Cylinder,
    tolerance: f32,
    depth: u8,
    t0: f32,
    t1: f32,
    max_points: usize,
    output: &mut Vec<(f32, Vec3)>,
) -> Result<(), String> {
    let mapped = CubicBezier {
        p0: cylinder
            .point_from_parameter(curve.p0)
            .ok_or("Cylinder curve left its surface")?,
        p1: cylinder
            .point_from_parameter(curve.p1)
            .ok_or("Cylinder curve handle left its surface")?,
        p2: cylinder
            .point_from_parameter(curve.p2)
            .ok_or("Cylinder curve handle left its surface")?,
        p3: cylinder
            .point_from_parameter(curve.p3)
            .ok_or("Cylinder curve left its surface")?,
    };
    let curved_midpoint = cylinder
        .point_from_parameter(curve.evaluate(0.5)?)
        .ok_or("Cylinder curve midpoint left its surface")?;
    if flatness(mapped).max(point_line_distance(curved_midpoint, mapped.p0, mapped.p3)) <= tolerance
    {
        if output.len() >= max_points {
            return Err("Curve tessellation exceeds the point limit".into());
        }
        output.push((t1, mapped.p3));
        return Ok(());
    }
    if depth >= MAX_RECURSION_DEPTH {
        return Err(
            "Curve could not meet the requested tolerance within the subdivision limit".into(),
        );
    }
    let (left, right) = curve.split(0.5)?;
    let middle = (t0 + t1) * 0.5;
    flatten_cylinder(
        left,
        cylinder,
        tolerance,
        depth + 1,
        t0,
        middle,
        max_points,
        output,
    )?;
    flatten_cylinder(
        right,
        cylinder,
        tolerance,
        depth + 1,
        middle,
        t1,
        max_points,
        output,
    )
}

/// Compute symmetric handle offsets for a smooth anchor. Endpoints use the
/// available one-sided direction; interior handles lie along the chord from
/// previous to next. Offsets use the same coordinate space as the controls.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, BrushKind, Document, StrokeMaterial};

    fn prototype() -> Stroke {
        Stroke {
            model_surface: None,
            gradient: None,
            point_thickness: None,
            modifiers: None,
            curve: None,
            id: 42,
            color: [0.15, 0.45, 0.8],
            radius: 0.035,
            brush: Brush {
                kind: BrushKind::Ribbon,
                angle_degrees: 23.0,
                ..Brush::default()
            },
            points: Vec::new(),
            fill: None,
            material: StrokeMaterial::Emissive,
            group_id: Some(8),
            layer_id: 1,
        }
    }

    fn curve(scale: f32) -> CubicBezier {
        CubicBezier {
            p0: Vec3::ZERO,
            p1: Vec3::new(0.0, scale, 0.0),
            p2: Vec3::new(scale, scale, 0.0),
            p3: Vec3::new(scale, 0.0, 0.0),
        }
    }

    #[test]
    fn adaptive_tessellation_scales_with_world_tolerance_and_preserves_endpoints() {
        let unit = tessellate_cubic(curve(1.0), 0.01, 4096).unwrap();
        let scaled = tessellate_cubic(curve(10.0), 0.1, 4096).unwrap();
        let finer = tessellate_cubic(curve(1.0), 0.002, 4096).unwrap();
        assert_eq!(unit.len(), scaled.len());
        assert!(finer.len() > unit.len());
        assert_eq!(unit.first(), Some(&curve(1.0).p0));
        assert_eq!(unit.last(), Some(&curve(1.0).p3));
        for (unit_point, scaled_point) in unit.iter().zip(&scaled) {
            assert!(unit_point.distance(*scaled_point / 10.0) < 0.00001);
        }
    }

    #[test]
    fn cylinder_curve_tessellation_wraps_the_seam_and_rejects_out_of_domain_handles() {
        let cylinder = crate::surface::Cylinder {
            center: Vec3::new(0.3, -0.4, 0.8),
            rotation_degrees: Vec3::new(13.0, 27.0, -8.0),
            radius: 1.2,
            height: 2.0,
        };
        let segment = PressureSegment {
            curve: CubicBezier {
                p0: Vec3::new(3.62, -0.2, 0.0),
                p1: Vec3::new(3.72, -0.1, 0.0),
                p2: Vec3::new(3.82, 0.1, 0.0),
                p3: Vec3::new(3.92, 0.2, 0.0),
            },
            pressure_start: 0.2,
            pressure_end: 0.9,
        };
        let stroke =
            tessellate_cylinder_stroke(&[segment], &prototype(), cylinder, 0.002, MAX_CURVE_POINTS)
                .unwrap();
        let rotation = cylinder.rotation();
        let angle = segment.curve.p0.x / cylinder.radius;
        let curve = crate::document::CurveData {
            anchors: vec![
                crate::document::CurveAnchor {
                    position: segment.curve.p0,
                    pressure: segment.pressure_start,
                    incoming: Vec3::ZERO,
                    outgoing: segment.curve.p1 - segment.curve.p0,
                    manual: true,
                },
                crate::document::CurveAnchor {
                    position: segment.curve.p3,
                    pressure: segment.pressure_end,
                    incoming: segment.curve.p2 - segment.curve.p3,
                    outgoing: Vec3::ZERO,
                    manual: true,
                },
            ],
            frame_origin: cylinder.center,
            frame_normal: rotation * Vec3::new(angle.sin(), 0.0, angle.cos()),
            frame_tangent: rotation * Vec3::new(angle.cos(), 0.0, -angle.sin()),
            half_extent: None,
            tension: 1.0,
            surface_target: Some(crate::dynamics::DabSurfaceTarget::Cylinder {
                center: cylinder.center,
                rotation,
                radius: cylinder.radius,
                height: cylinder.height,
            }),
            shape: None,
        };
        assert!(curve.is_valid());
        let mut stroke = stroke;
        stroke.curve = Some(curve.clone());
        assert!(stroke.points.len() > 2);
        assert!(stroke.points.iter().all(|point| {
            let local = cylinder.to_local(point.position);
            (local.x.hypot(local.z) - cylinder.radius).abs() < 1.0e-4
                && (0.2..=0.9).contains(&point.pressure)
        }));
        assert!(stroke
            .points
            .windows(2)
            .all(|pair| { pair[0].position.distance(pair[1].position) < 0.2 }));

        let document = Document {
            strokes: vec![stroke.clone()],
            ..Document::default()
        };
        let reopened = Document::from_json(&serde_json::to_string(&document).unwrap()).unwrap();
        assert_eq!(reopened.strokes[0].curve.as_ref(), Some(&curve));
        let recovery_root = tempfile::tempdir().unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "cylinder-curve-test".into(),
            None,
            reopened.clone(),
        );
        crate::recovery_store::write(recovery_root.path(), &snapshot).unwrap();
        let recovered = crate::recovery_store::scan(recovery_root.path(), "test-reader").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes[0]
                .curve
                .as_ref(),
            Some(&curve)
        );
        let exported = crate::export::encode(&reopened, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 1);
        let mut camera = crate::geometry::Camera::default();
        let angle = segment.curve.p3.x / cylinder.radius;
        let facing = rotation * Vec3::new(angle.sin(), 0.0, angle.cos());
        camera.yaw = facing.x.atan2(facing.z);
        camera.pitch = facing.y.asin();
        camera.target = cylinder.center;
        camera.distance = 5.0;
        let canvas = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(500.0, 400.0),
        );
        let point = stroke.points[stroke.points.len() / 2].position;
        let screen = crate::gizmo::project(&camera, canvas, point).unwrap().0;
        assert_eq!(
            crate::gizmo::pick(&reopened, &camera, canvas, screen),
            Some(stroke.id)
        );

        let invalid = PressureSegment {
            curve: CubicBezier {
                p0: Vec3::new(0.0, 0.0, 0.0),
                p1: Vec3::new(0.1, 1.1, 0.0),
                p2: Vec3::new(0.2, 0.0, 0.0),
                p3: Vec3::new(0.3, 0.0, 0.0),
            },
            pressure_start: 1.0,
            pressure_end: 1.0,
        };
        assert!(tessellate_cylinder_stroke(
            &[invalid],
            &prototype(),
            cylinder,
            0.01,
            MAX_CURVE_POINTS,
        )
        .is_err());
    }

    #[test]
    fn pressure_aware_segments_join_once_and_preserve_brush_metadata() {
        let left = CubicBezier {
            p0: Vec3::ZERO,
            p1: Vec3::new(0.0, 0.3, 0.0),
            p2: Vec3::new(0.7, 0.3, 0.0),
            p3: Vec3::X,
        };
        let right = CubicBezier {
            p0: Vec3::X,
            p1: Vec3::new(1.3, -0.3, 0.0),
            p2: Vec3::new(1.7, 0.3, 0.0),
            p3: Vec3::new(2.0, 0.0, 0.0),
        };
        let mut source = prototype();
        source.points = vec![Point {
            position: Vec3::new(10.0, 20.0, 30.0),
            pressure: 1.0,
            tilt: Some(Vec3::Z),

            time_ms: 0,
        }];
        let stroke = tessellate_stroke(
            &[
                PressureSegment {
                    curve: left,
                    pressure_start: 0.2,
                    pressure_end: 0.5,
                },
                PressureSegment {
                    curve: right,
                    pressure_start: 0.5,
                    pressure_end: 0.9,
                },
            ],
            &source,
            0.01,
            4096,
        )
        .unwrap();
        assert_eq!(stroke.id, source.id);
        assert_eq!(stroke.color, source.color);
        assert_eq!(stroke.radius, source.radius);
        assert_eq!(stroke.brush, source.brush);
        assert_eq!(stroke.material, source.material);
        assert_eq!(stroke.group_id, source.group_id);
        assert_eq!(stroke.layer_id, source.layer_id);
        assert_eq!(stroke.fill, None);
        assert_eq!(stroke.points.first().unwrap().position, left.p0);
        assert_eq!(stroke.points.last().unwrap().position, right.p3);
        assert_eq!(stroke.points.first().unwrap().pressure, 0.2);
        assert_eq!(stroke.points.last().unwrap().pressure, 0.9);
        assert_eq!(
            stroke
                .points
                .iter()
                .filter(|point| point.position == Vec3::X)
                .count(),
            1,
            "a shared anchor is stored once"
        );
        assert!(stroke
            .points
            .windows(2)
            .all(|pair| pair[0].pressure <= pair[1].pressure));

        let mut doc = Document::default();
        let edit = crate::editing::Edit::Add(stroke.clone());
        edit.apply(&mut doc, true);
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let reopened = Document::from_json(&std::fs::read_to_string(file.path()).unwrap()).unwrap();
        assert_eq!(reopened.strokes, doc.strokes);
        let glb = crate::export::encode(&reopened, None).unwrap();
        assert_eq!(
            gltf::Gltf::from_slice(&glb.bytes).unwrap().meshes().count(),
            1
        );
        edit.apply(&mut doc, false);
        assert!(doc.strokes.is_empty());
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes, vec![stroke]);
    }

    #[test]
    fn cubic_split_and_join_keep_endpoints_and_tangent_directions() {
        let original = curve(1.0);
        let (left, right) = original.split(0.37).unwrap();
        assert!(left.p3.distance(right.p0) < 1e-6);
        assert!(left.p3.distance(original.evaluate(0.37).unwrap()) < 1e-6);
        let joined = CubicBezier::join(left, right, 1.0).unwrap();
        assert_eq!(joined.p0, original.p0);
        assert_eq!(joined.p3, original.p3);
        assert!(
            joined
                .derivative(0.0)
                .unwrap()
                .dot(original.derivative(0.0).unwrap())
                > 0.0
        );
        assert!(
            joined
                .derivative(1.0)
                .unwrap()
                .dot(original.derivative(1.0).unwrap())
                > 0.0
        );
    }
}
