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

/// Mirror only the portions of a stroke that are not coincident with the
/// symmetry plane. Consecutive seam samples are removed from the reflected
/// copy and split it into independent runs, so the original seam ink is never
/// overdrawn. Isolated seam samples remain shared endpoints when a stroke
/// simply crosses the plane.
pub fn mirror_stroke_runs(
    source: &Stroke,
    plane: MirrorPlane,
    first_fresh_id: u64,
    seam_tolerance: f32,
) -> Result<Vec<Stroke>, String> {
    if source.points.len() < 2 {
        return Err("Cannot mirror a stroke without at least two points".into());
    }
    if !seam_tolerance.is_finite() || seam_tolerance < 0.0 {
        return Err("Mirror seam tolerance must be finite and non-negative".into());
    }
    let normal = plane.unit_normal()?;
    let seam: Vec<bool> = source
        .points
        .iter()
        .map(|point| {
            plane
                .signed_distance(point.position)
                .map(|d| d.abs() <= seam_tolerance)
        })
        .collect::<Result<_, _>>()?;
    if seam.iter().all(|is_seam| *is_seam) {
        return Ok(Vec::new());
    }

    let snap = |mut point: Point| {
        let distance = (point.position - plane.point).dot(normal);
        if distance.abs() <= seam_tolerance {
            point.position -= normal * distance;
        }
        point
    };
    let mut runs: Vec<Vec<Point>> = Vec::new();
    let mut current = Vec::new();
    for index in 0..source.points.len() - 1 {
        let (a, b) = (source.points[index], source.points[index + 1]);
        if seam[index] && seam[index + 1] {
            if current.len() >= 2 {
                runs.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            continue;
        }
        let a = snap(a);
        let b = snap(b);
        if current.last().is_none_or(|last: &Point| *last != a) {
            current.push(a);
        }
        current.push(b);
    }
    if current.len() >= 2 {
        runs.push(current);
    }

    if runs.len() > 100_000 || first_fresh_id == u64::MAX {
        return Err("Mirrored runs exceed stroke or ID limits".into());
    }
    let mut mirrored = Vec::with_capacity(runs.len());
    for (index, points) in runs.into_iter().enumerate() {
        let id = first_fresh_id
            .checked_add(index as u64)
            .filter(|id| *id != u64::MAX)
            .ok_or("No more stroke IDs available")?;
        let mut run = source.clone();
        run.points = points;
        if let Some(copy) = mirror_stroke(&run, plane, id, seam_tolerance)? {
            mirrored.push(copy);
        }
    }
    Ok(mirrored)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, BrushKind, StrokeMaterial};

    fn source(points: Vec<Point>) -> Stroke {
        let brush = Brush {
            kind: BrushKind::Ribbon,
            angle_degrees: 28.0,
            normal: Vec3::Z,
            tangent: Vec3::X,
            ..Brush::default()
        };
        Stroke {
            id: 7,
            color: [0.2, 0.4, 0.8],
            radius: 0.08,
            brush,
            points,
            fill: None,
            material: StrokeMaterial::Emissive,
            group_id: Some(3),
            layer_id: 2,
        }
    }

    fn point(x: f32) -> Point {
        Point {
            position: Vec3::new(x, 0.25, 0.0),
            pressure: 0.3 + (x.abs() * 0.1),
            tilt: Some(Vec3::Y),
        }
    }

    #[test]
    fn partial_seam_overlap_splits_reflected_runs_without_losing_attributes() {
        let original = source(vec![
            point(-1.0),
            point(0.0),
            point(0.0),
            point(1.0),
            point(2.0),
        ]);
        let plane = MirrorPlane {
            point: Vec3::ZERO,
            normal: Vec3::X,
        };
        let untouched = original.clone();
        let runs = mirror_stroke_runs(&original, plane, 100, 1e-5).unwrap();

        assert_eq!(original, untouched, "mirroring must be immutable");
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs.iter().map(|stroke| stroke.id).collect::<Vec<_>>(),
            [100, 101]
        );
        assert_eq!(runs[0].points.len(), 2);
        assert_eq!(runs[1].points.len(), 3);
        assert_eq!(runs[0].points[0].position, Vec3::new(1.0, 0.25, 0.0));
        assert_eq!(runs[0].points[1].position, Vec3::new(0.0, 0.25, 0.0));
        assert_eq!(runs[1].points[0].position, Vec3::new(0.0, 0.25, 0.0));
        assert_eq!(runs[1].points[2].position, Vec3::new(-2.0, 0.25, 0.0));
        assert_eq!(runs[0].material, StrokeMaterial::Emissive);
        assert_eq!(runs[0].group_id, Some(3));
        assert_eq!(runs[0].layer_id, 2);
        assert_eq!(runs[0].color, original.color);
        assert_eq!(runs[0].radius, original.radius);
        assert_eq!(runs[0].brush.kind, BrushKind::Ribbon);
        assert_eq!(runs[0].brush.angle_degrees, -original.brush.angle_degrees);
        assert_eq!(
            runs[0].brush.normal,
            plane.reflect_vector(original.brush.normal).unwrap()
        );
        assert_eq!(
            runs[0].brush.tangent,
            plane.reflect_vector(original.brush.tangent).unwrap()
        );
        assert_eq!(runs[0].points[0].pressure, original.points[0].pressure);
        assert_eq!(runs[0].points[0].tilt, original.points[0].tilt);
        assert_eq!(runs[0].points[1].pressure, original.points[1].pressure);
        assert_eq!(runs[0].points[1].tilt, original.points[1].tilt);
    }

    #[test]
    fn fully_seam_coincident_stroke_has_no_reflected_run() {
        let original = source(vec![point(0.0), point(0.0), point(0.0)]);
        let plane = MirrorPlane {
            point: Vec3::ZERO,
            normal: Vec3::X,
        };
        assert!(mirror_stroke_runs(&original, plane, 100, 1e-5)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn transformed_plane_reflection_is_atomic_reversible_persistent_and_exportable() {
        let mut original = source(vec![point(-1.0), point(-0.25), point(0.5)]);
        original.layer_id = 1;
        let plane = MirrorPlane {
            point: Vec3::new(0.5, -0.25, 0.75),
            normal: Vec3::new(2.0, 4.0, 1.0),
        };
        let copies = mirror_stroke_runs(&original, plane, 100, 1e-5).unwrap();
        assert_eq!(copies.len(), 1);
        let reflected = &copies[0];
        for (before, after) in original.points.iter().zip(&reflected.points) {
            assert!(
                (plane.signed_distance(before.position).unwrap()
                    + plane.signed_distance(after.position).unwrap())
                .abs()
                    < 0.00001
            );
            assert_eq!(before.pressure, after.pressure);
            assert_eq!(
                after.tilt,
                before.tilt.map(|tilt| plane.reflect_vector(tilt).unwrap())
            );
        }
        assert_eq!(
            reflected.brush.normal,
            plane.reflect_vector(original.brush.normal).unwrap()
        );
        assert_eq!(
            reflected.brush.tangent,
            plane.reflect_vector(original.brush.tangent).unwrap()
        );
        assert_eq!(reflected.color, original.color);
        assert_eq!(reflected.radius, original.radius);
        assert_eq!(reflected.group_id, original.group_id);
        assert_eq!(reflected.layer_id, original.layer_id);

        let mut doc = crate::document::Document::default();
        let edit = crate::editing::Edit::Change {
            before: Vec::new(),
            after: vec![(0, original.clone()), (1, reflected.clone())],
        };
        edit.apply(&mut doc, true);
        let committed = doc.strokes.clone();
        assert_eq!(committed.len(), 2, "source and mirror form one atomic edit");
        edit.apply(&mut doc, false);
        assert!(doc.strokes.is_empty());
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes, committed);

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let reopened =
            crate::document::Document::from_json(&std::fs::read_to_string(file.path()).unwrap())
                .unwrap();
        assert_eq!(reopened.strokes, doc.strokes);

        let glb = crate::export::encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&glb.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 2);
    }
}
