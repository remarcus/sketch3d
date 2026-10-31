//! Geometry helpers for mirrored stroke creation.
//!
//! This module does not mutate a document. Callers can preview the returned
//! stroke and add it in the same undo transaction as its source stroke.
use crate::document::{Point, Stroke};
use glam::Vec3;

/// A world-space mirror plane. The normal need not be unit length; public
/// operations normalize it before use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MirrorPlane {
    pub point: Vec3,
    pub normal: Vec3,
}

impl MirrorPlane {
    fn unit_normal(self) -> Result<Vec3, String> {
        if !self.point.is_finite() || !self.normal.is_finite() {
            return Err("Mirror plane contains non-finite coordinates".into());
        }
        self.normal
            .try_normalize()
            .ok_or_else(|| "Mirror plane normal must have a non-zero length".into())
    }

    /// Reflect a world-space point through the plane.
    pub fn reflect_point(self, point: Vec3) -> Result<Vec3, String> {
        if !point.is_finite() {
            return Err("Cannot reflect a non-finite point".into());
        }
        let normal = self.unit_normal()?;
        Ok(point - 2.0 * normal * (point - self.point).dot(normal))
    }

    /// Reflect a direction vector. This does not translate or normalize it.
    pub fn reflect_vector(self, vector: Vec3) -> Result<Vec3, String> {
        if !vector.is_finite() {
            return Err("Cannot reflect a non-finite direction".into());
        }
        let normal = self.unit_normal()?;
        Ok(vector - 2.0 * normal * vector.dot(normal))
    }

    /// Signed distance from a point to this plane, in world units.
    pub fn signed_distance(self, point: Vec3) -> Result<f32, String> {
        if !point.is_finite() {
            return Err("Cannot measure a non-finite point".into());
        }
        Ok((point - self.point).dot(self.unit_normal()?))
    }
}

/// True if every sampled centerline point lies within `tolerance` of the
/// mirror plane. Reflecting such a stroke would create coincident geometry on
/// the seam, so [`mirror_stroke`] returns `Ok(None)` for it.
pub fn is_seam_coincident(
    stroke: &Stroke,
    plane: MirrorPlane,
    tolerance: f32,
) -> Result<bool, String> {
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err("Mirror seam tolerance must be finite and non-negative".into());
    }
    if stroke.points.is_empty() {
        return Err("Cannot mirror a stroke without points".into());
    }
    for point in &stroke.points {
        if plane.signed_distance(point.position)?.abs() > tolerance {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Build an independent mirrored copy with a caller-reserved fresh ID.
///
/// Every point and optional tilt vector, plus the captured brush frame, is
/// reflected. Pressure, color, radius, texture, group, layer, fill offset,
/// opacity, dynamics, and softness retain their original values. Since a
/// plane reflection reverses handedness, the signed brush angle is negated.
/// The operation preserves all sampled points and never changes the source.
///
/// Returns `Ok(None)` if the centerline lies entirely on the mirror seam;
/// callers should keep only the original stroke in that case. This covers
/// exact seam duplication. A stroke that only partly follows the seam remains
/// a complete reflected copy; callers that need to split and deduplicate
/// partial seam runs should do that as a separate editing operation.
pub fn mirror_stroke(
    source: &Stroke,
    plane: MirrorPlane,
    fresh_id: u64,
    seam_tolerance: f32,
) -> Result<Option<Stroke>, String> {
    if fresh_id == u64::MAX || fresh_id == source.id {
        return Err("Mirrored stroke needs a distinct, available ID".into());
    }
    if source.points.is_empty() {
        return Err("Cannot mirror a stroke without points".into());
    }
    if !source.brush.is_valid()
        || !source.radius.is_finite()
        || !(0.001..=1.0).contains(&source.radius)
        || source
            .color
            .iter()
            .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
        || source.points.iter().any(|point| !point.is_valid())
    {
        return Err("Cannot mirror invalid stroke data".into());
    }
    if is_seam_coincident(source, plane, seam_tolerance)? {
        return Ok(None);
    }

    let mut mirrored = source.clone();
    mirrored.id = fresh_id;
    for point in &mut mirrored.points {
        *point = mirror_point(*point, plane)?;
        if !point.is_valid() {
            return Err("Reflected stroke exceeds document point limits".into());
        }
    }
    mirrored.brush.normal = plane.reflect_vector(source.brush.normal)?;
    mirrored.brush.tangent = plane.reflect_vector(source.brush.tangent)?;
    mirrored.brush.angle_degrees = -source.brush.angle_degrees;
    if !mirrored.brush.is_valid() {
        return Err("Reflected brush frame is invalid".into());
    }
    if mirrored.fill.is_some() && crate::polygon::triangulate(&mirrored).is_err() {
        return Err("Reflected fill would exceed planar fill limits".into());
    }
    Ok(Some(mirrored))
}

/// Reflect a point sample while retaining pressure and tilt exactly apart from
/// the geometric transform.
pub fn mirror_point(point: Point, plane: MirrorPlane) -> Result<Point, String> {
    let mirrored = Point {
        position: plane.reflect_point(point.position)?,
        pressure: point.pressure,
        tilt: point
            .tilt
            .map(|tilt| plane.reflect_vector(tilt))
            .transpose()?,
    };
    if !mirrored.is_valid() {
        return Err("Reflected point is outside the document limits".into());
    }
    Ok(mirrored)
}
