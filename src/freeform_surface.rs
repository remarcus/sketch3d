//! Bounded freeform ribbon guide built from a monotone cubic directrix.
//!
//! The directrix lives in local XY space and is swept across local Z. A small
//! sinusoidal cross-sweep bend adds depth variation. Strictly increasing cubic
//! X controls and a bend factor below one make the parameterization one-to-one,
//! which rejects loops and keeps ray hits deterministic.

use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub const MIN_DEPTH: f32 = 0.02;
pub const MAX_DEPTH: f32 = 10_000.0;
pub const MAX_BEND: f32 = 0.9;
pub const MAX_COORDINATE: f32 = 10_000.0;
const HIT_U_STEPS: usize = 64;
const HIT_V_STEPS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FreeformSurface {
    pub center: Vec3,
    pub rotation: Quat,
    /// Cubic Bézier controls for `(local X, local Y)` in world units.
    pub directrix: [Vec2; 4],
    /// Sweep extent along local Z.
    pub depth: f32,
    /// Signed, dimensionless cross-sweep bend in `[-0.9, 0.9]`.
    pub bend: f32,
}

impl FreeformSurface {
    pub fn validate(&self) -> Result<(), String> {
        if !self.center.is_finite()
            || self.center.abs().max_element() > MAX_COORDINATE
            || !self.rotation.is_finite()
            || (self.rotation.length_squared() - 1.0).abs() > 0.001
        {
            return Err("Freeform guide transform is invalid or outside document bounds".into());
        }
        if self
            .directrix
            .iter()
            .any(|point| !point.is_finite() || point.abs().max_element() > MAX_COORDINATE)
        {
            return Err("Freeform guide controls must be finite and within document bounds".into());
        }
        if self.directrix[0].x + 0.01 > self.directrix[1].x
            || self.directrix[1].x + 0.01 > self.directrix[2].x
            || self.directrix[2].x + 0.01 > self.directrix[3].x
        {
            return Err(
                "Freeform directrix X controls must increase by at least 0.01 units".into(),
            );
        }
        if !self.depth.is_finite() || !(MIN_DEPTH..=MAX_DEPTH).contains(&self.depth) {
            return Err(format!(
                "Freeform sweep depth must be between {MIN_DEPTH} and {MAX_DEPTH}"
            ));
        }
        if !self.bend.is_finite() || self.bend.abs() > MAX_BEND {
            return Err(format!(
                "Freeform bend must be between -{MAX_BEND} and {MAX_BEND}"
            ));
        }
        Ok(())
    }

    pub fn directrix_point(&self, u: f32) -> Vec2 {
        cubic(self.directrix, u.clamp(0.0, 1.0))
    }

    pub fn point(&self, u: f32, v: f32) -> Vec3 {
        let u = u.clamp(0.0, 1.0);
        let s = v.clamp(0.0, 1.0) - 0.5;
        let curve = self.directrix_point(u);
        let z = self.depth * (s + self.bend * (std::f32::consts::PI * u).sin() * s * s);
        self.center + self.rotation * Vec3::new(curve.x, curve.y, z)
    }

    /// World-space tangent and normal for a point on the guide.
    pub fn frame(&self, u: f32, v: f32) -> (Vec3, Vec3) {
        let u = u.clamp(0.0, 1.0);
        let s = v.clamp(0.0, 1.0) - 0.5;
        let derivative = cubic_derivative(self.directrix, u);
        let dz_du = self.depth
            * self.bend
            * std::f32::consts::PI
            * (std::f32::consts::PI * u).cos()
            * s
            * s;
        let dz_dv = self.depth * (1.0 + 2.0 * self.bend * (std::f32::consts::PI * u).sin() * s);
        let tangent_local = Vec3::new(derivative.x, derivative.y, dz_du).normalize_or_zero();
        let across_local = Vec3::Z * dz_dv;
        let normal_local = tangent_local.cross(across_local).normalize_or_zero();
        let tangent = self.rotation * tangent_local;
        let normal = self.rotation * normal_local;
        (tangent, normal)
    }

    /// Invert world position to approximate stable directrix/sweep coordinates.
    pub fn coordinates(&self, world: Vec3) -> Option<(f32, f32)> {
        if !world.is_finite() || self.validate().is_err() {
            return None;
        }
        let local = self.rotation.inverse() * (world - self.center);
        let x0 = self.directrix[0].x;
        let x1 = self.directrix[3].x;
        if local.x < x0 - 1e-4 || local.x > x1 + 1e-4 {
            return None;
        }
        let mut low = 0.0_f32;
        let mut high = 1.0_f32;
        for _ in 0..24 {
            let mid = (low + high) * 0.5;
            if self.directrix_point(mid).x < local.x {
                low = mid;
            } else {
                high = mid;
            }
        }
        let u = (low + high) * 0.5;
        let curve = self.directrix_point(u);
        if (curve.y - local.y).abs() > self.depth.max(1.0) * 0.02 {
            return None;
        }
        let target = local.z / self.depth;
        let mut v_low = -0.5_f32;
        let mut v_high = 0.5_f32;
        let bend = self.bend * (std::f32::consts::PI * u).sin();
        for _ in 0..32 {
            let s = (v_low + v_high) * 0.5;
            let value = s + bend * s * s;
            if value < target {
                v_low = s;
            } else {
                v_high = s;
            }
        }
        let s = (v_low + v_high) * 0.5;
        let v = s + 0.5;
        (-1e-4..=1.0001)
            .contains(&v)
            .then_some((u, v.clamp(0.0, 1.0)))
    }

    /// Nearest positive ray hit against a deterministic, bounded triangle grid.
    pub fn hit(&self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        if !origin.is_finite()
            || !direction.is_finite()
            || direction.length_squared() < 1e-12
            || self.validate().is_err()
        {
            return None;
        }
        let inverse = self.rotation.inverse();
        let local_origin = inverse * (origin - self.center);
        let local_direction = inverse * direction;
        let mut nearest: Option<f32> = None;
        for iu in 0..HIT_U_STEPS {
            let u0 = iu as f32 / HIT_U_STEPS as f32;
            let u1 = (iu + 1) as f32 / HIT_U_STEPS as f32;
            for iv in 0..HIT_V_STEPS {
                let v0 = iv as f32 / HIT_V_STEPS as f32;
                let v1 = (iv + 1) as f32 / HIT_V_STEPS as f32;
                let p00 = self.local_point(u0, v0);
                let p10 = self.local_point(u1, v0);
                let p11 = self.local_point(u1, v1);
                let p01 = self.local_point(u0, v1);
                for (a, b, c) in [(p00, p10, p11), (p00, p11, p01)] {
                    if let Some(t) = ray_triangle(local_origin, local_direction, a, b, c) {
                        if nearest.is_none_or(|old| t < old) {
                            nearest = Some(t);
                        }
                    }
                }
            }
        }
        nearest.map(|t| origin + direction * t)
    }

    /// A lightweight triangle mesh for preview/picking integrations.
    #[allow(dead_code)]
    pub fn triangles(&self, u_steps: usize, v_steps: usize) -> Option<Vec<[Vec3; 3]>> {
        if self.validate().is_err()
            || u_steps == 0
            || v_steps == 0
            || u_steps.saturating_mul(v_steps) > 4096
        {
            return None;
        }
        let mut triangles = Vec::with_capacity(u_steps * v_steps * 2);
        for iu in 0..u_steps {
            let u0 = iu as f32 / u_steps as f32;
            let u1 = (iu + 1) as f32 / u_steps as f32;
            for iv in 0..v_steps {
                let v0 = iv as f32 / v_steps as f32;
                let v1 = (iv + 1) as f32 / v_steps as f32;
                let p00 = self.point(u0, v0);
                let p10 = self.point(u1, v0);
                let p11 = self.point(u1, v1);
                let p01 = self.point(u0, v1);
                triangles.push([p00, p10, p11]);
                triangles.push([p00, p11, p01]);
            }
        }
        Some(triangles)
    }

    fn local_point(&self, u: f32, v: f32) -> Vec3 {
        let s = v - 0.5;
        let curve = cubic(self.directrix, u);
        let z = self.depth * (s + self.bend * (std::f32::consts::PI * u).sin() * s * s);
        Vec3::new(curve.x, curve.y, z)
    }
}

fn cubic(control: [Vec2; 4], t: f32) -> Vec2 {
    let s = 1.0 - t;
    control[0] * (s * s * s)
        + control[1] * (3.0 * s * s * t)
        + control[2] * (3.0 * s * t * t)
        + control[3] * (t * t * t)
}

fn cubic_derivative(control: [Vec2; 4], t: f32) -> Vec2 {
    let s = 1.0 - t;
    (control[1] - control[0]) * (3.0 * s * s)
        + (control[2] - control[1]) * (6.0 * s * t)
        + (control[3] - control[2]) * (3.0 * t * t)
}

fn ray_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let edge1 = b - a;
    let edge2 = c - a;
    let p = direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() <= 1e-9 {
        return None;
    }
    let inverse = determinant.recip();
    let offset = origin - a;
    let u = offset.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = offset.cross(edge1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge2.dot(q) * inverse;
    (t > 1e-6 && t.is_finite()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Point;

    fn surface() -> FreeformSurface {
        FreeformSurface {
            center: Vec3::new(0.7, -0.3, 1.2),
            rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.25),
            directrix: [
                Vec2::new(-1.0, 0.0),
                Vec2::new(-0.4, 0.8),
                Vec2::new(0.4, -0.5),
                Vec2::new(1.0, 0.2),
            ],
            depth: 2.5,
            bend: 0.4,
        }
    }

    #[test]
    fn freeform_sheet_validates_hits_deterministically_and_inverts_coordinates() {
        let surface = surface();
        assert!(surface.validate().is_ok());
        assert_eq!(
            surface.triangles(HIT_U_STEPS, HIT_V_STEPS).unwrap().len(),
            1024
        );
        let (u, v) = (0.37, 0.62);
        let point = surface.point(u, v);
        let (_, normal) = surface.frame(u, v);
        let hit = surface.hit(point + normal * 3.0, -normal).unwrap();
        assert!(
            hit.distance(point) < 0.06,
            "mesh hit should be near analytic sheet"
        );
        let coordinates = surface.coordinates(hit).unwrap();
        assert!((coordinates.0 - u).abs() < 0.02);
        assert!((coordinates.1 - v).abs() < 0.02);

        let miss_origin = surface.center + surface.rotation * Vec3::new(4.0, 4.0, 5.0);
        assert!(surface
            .hit(miss_origin, surface.rotation * Vec3::Z)
            .is_none());

        let mut invalid = surface;
        invalid.directrix[1].x = invalid.directrix[2].x + 0.1;
        assert!(invalid.validate().is_err());
        invalid = surface;
        invalid.bend = MAX_BEND + 0.01;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn freeform_stabilization_stays_on_the_single_sheet() {
        let surface = surface();
        let mut stabilizer = crate::surface::SurfaceStabilizer::default();
        stabilizer.start(crate::surface::Surface::Freeform(surface));
        let mut output = Vec::new();
        for index in 0..12 {
            let u = 0.1 + index as f32 * 0.065;
            let v = 0.2 + (index as f32 * 0.27).sin() * 0.2;
            let position = surface.point(u, v);
            if index > 0 {
                assert!(!stabilizer.discontinuity(position));
            }
            output.extend(stabilizer.push(
                Point {
                    position,
                    pressure: 0.2 + index as f32 * 0.05,
                    tilt: None,

                    time_ms: 0,
                },
                0.08,
            ));
        }
        output.extend(stabilizer.finish());
        assert!(output.len() > 12);
        assert!(output.iter().all(|point| {
            surface
                .coordinates(point.position)
                .is_some_and(|(u, v)| (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v))
        }));
    }

    #[test]
    fn freeform_guide_parameters_save_transform_delete_and_undo_exactly() {
        let source = surface();
        let guide = crate::document::guide_resources::GuideResource {
            id: 31,
            name: "Canopy".into(),
            visible: true,
            locked: false,
            geometry: crate::document::guide_resources::GuideGeometry::Freeform { surface: source },
        };
        let mut doc = crate::document::Document {
            linked_stroke_groups: Default::default(),
            guide_resources: vec![guide.clone()],
            active_guide: Some(31),
            ..Default::default()
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let reopened =
            crate::document::Document::from_json(&std::fs::read_to_string(file.path()).unwrap())
                .unwrap();
        assert_eq!(reopened.guide_resources, doc.guide_resources);
        assert_eq!(reopened.active_guide, doc.active_guide);

        let before = (
            doc.guide_resources.clone(),
            doc.active_guide,
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let mut transformed = guide.clone();
        if let crate::document::guide_resources::GuideGeometry::Freeform { surface } =
            &mut transformed.geometry
        {
            surface.center += Vec3::new(0.5, -0.2, 1.0);
            surface.rotation = Quat::from_rotation_y(0.7);
            surface.depth = 3.2;
            surface.bend = -0.25;
        }
        let after = (
            vec![transformed],
            Some(31),
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let transform = crate::editing::Edit::Guides {
            ink: None,
            before: before.clone(),
            after: after.clone(),
        };
        transform.apply(&mut doc, true);
        transform.apply(&mut doc, false);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            before
        );
        transform.apply(&mut doc, true);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            after
        );

        let deleted = (
            Vec::new(),
            None,
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let remove = crate::editing::Edit::Guides {
            ink: None,
            before: after.clone(),
            after: deleted,
        };
        remove.apply(&mut doc, true);
        assert!(doc.guide_resources.is_empty());
        remove.apply(&mut doc, false);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            after
        );
    }
}
