//! Persistent guide resources and bounded primitive intersection helpers.
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[path = "freeform_surface.rs"]
pub mod freeform_surface;
pub use freeform_surface::FreeformSurface;

#[path = "loft_surface.rs"]
pub mod loft_surface;
pub use loft_surface::{LoftSection, LoftSurface};

pub const MAX_GUIDES: usize = 64;
pub const MAX_GUIDE_NAME_CHARS: usize = 96;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // Resource variants stay directly serializable without boxing.
pub enum GuideGeometry {
    Plane {
        origin: Vec3,
        rotation: Quat,
    },
    Cylinder {
        center: Vec3,
        rotation: Quat,
        radius: f32,
        height: f32,
    },
    Revolved {
        center: Vec3,
        rotation: Quat,
        profile: Vec<ProfilePoint>,
    },
    Sphere {
        center: Vec3,
        rotation: Quat,
        radius: f32,
    },
    Ring {
        center: Vec3,
        rotation: Quat,
        major_radius: f32,
        minor_radius: f32,
    },
    Cone {
        center: Vec3,
        rotation: Quat,
        radius: f32,
        height: f32,
    },
    Freeform {
        surface: FreeformSurface,
    },
    Loft {
        surface: LoftSurface,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfilePoint {
    pub height: f32,
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuideResource {
    pub id: u64,
    pub name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    pub geometry: GuideGeometry,
}

fn default_true() -> bool {
    true
}

impl GuideGeometry {
    pub fn sphere(center: Vec3, radius: f32) -> Self {
        Self::Sphere {
            center,
            rotation: Quat::IDENTITY,
            radius,
        }
    }

    pub fn cone(center: Vec3, radius: f32, height: f32) -> Self {
        Self::Cone {
            center,
            rotation: Quat::IDENTITY,
            radius,
            height,
        }
    }

    /// Nearest positive intersection with an open guide surface. Plane resources
    /// are infinite; cylinders, cones, spheres, and revolved profiles have no caps.
    pub fn hit(self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        if !origin.is_finite() || !direction.is_finite() || direction.length_squared() < 1e-12 {
            return None;
        }
        match self {
            Self::Plane {
                origin: center,
                rotation,
            } => {
                if !valid_transform(center, rotation) {
                    return None;
                }
                let inverse = rotation.inverse();
                let local_origin = inverse * (origin - center);
                let local_direction = inverse * direction;
                if local_direction.z.abs() < 1e-8 {
                    return None;
                }
                let t = -local_origin.z / local_direction.z;
                (t > 1e-6 && t.is_finite()).then_some(origin + direction * t)
            }
            Self::Sphere {
                center,
                rotation,
                radius,
            } => {
                if !valid_transform(center, rotation) || !valid_dimension(radius) {
                    return None;
                }
                let o = rotation.inverse() * (origin - center);
                let d = rotation.inverse() * direction;
                let a = d.length_squared();
                let b = 2.0 * o.dot(d);
                let c = o.length_squared() - radius * radius;
                let discriminant = b * b - 4.0 * a * c;
                if discriminant < 0.0 {
                    return None;
                }
                let root = discriminant.sqrt();
                let t0 = (-b - root) / (2.0 * a);
                let t1 = (-b + root) / (2.0 * a);
                let t = [t0, t1]
                    .into_iter()
                    .filter(|t| *t > 1e-6 && t.is_finite())
                    .min_by(f32::total_cmp)?;
                Some(origin + direction * t)
            }
            Self::Ring {
                center,
                rotation,
                major_radius,
                minor_radius,
            } => {
                if !valid_ring(center, rotation, major_radius, minor_radius) {
                    return None;
                }
                let inverse = rotation.inverse();
                let o = inverse * (origin - center);
                let d = inverse * direction;
                let bound = major_radius + minor_radius;
                let a = d.length_squared() as f64;
                let b = (2.0 * o.dot(d)) as f64;
                let c = (o.length_squared() - bound * bound) as f64;
                let discriminant = b * b - 4.0 * a * c;
                let discriminant_tolerance = (b * b).abs().max((4.0 * a * c).abs()).max(1.0) * 1e-6;
                if !discriminant.is_finite() || discriminant < -discriminant_tolerance || a <= 1e-14
                {
                    return None;
                }
                let root = discriminant.max(0.0).sqrt();
                // Expand the enclosing-sphere interval slightly. Transformed
                // rays can land just outside an exact tangent endpoint due to
                // f32 rotation roundoff, which otherwise drops the first torus
                // surface root and incorrectly returns the farther tube hit.
                let endpoint_tolerance = 1e-5 * bound.max(1.0) as f64;
                let t0 = ((-b - root) / (2.0 * a) - endpoint_tolerance).max(1e-6);
                let t1 = (-b + root) / (2.0 * a) + endpoint_tolerance;
                if !t1.is_finite() || t1 < t0 {
                    return None;
                }
                let r = major_radius as f64;
                let tube = minor_radius as f64;
                let x = o.x as f64;
                let y = o.y as f64;
                let z = o.z as f64;
                let dx = d.x as f64;
                let dy = d.y as f64;
                let dz = d.z as f64;
                let q0 = x * x + y * y + z * z + r * r - tube * tube;
                let q1 = 2.0 * (x * dx + y * dy + z * dz);
                let q2 = dx * dx + dy * dy + dz * dz;
                let coeff = [
                    q0 * q0 - 4.0 * r * r * (x * x + z * z),
                    2.0 * q0 * q1 - 8.0 * r * r * (x * dx + z * dz),
                    q1 * q1 + 2.0 * q0 * q2 - 4.0 * r * r * (dx * dx + dz * dz),
                    2.0 * q1 * q2,
                    q2 * q2,
                ];
                let roots = polynomial_roots_in_interval(&coeff, t0, t1);
                let t = roots
                    .into_iter()
                    .filter(|t| *t > 1e-6)
                    .min_by(f64::total_cmp)?;
                Some(origin + direction * t as f32)
            }
            Self::Cylinder {
                center,
                rotation,
                radius,
                height,
            } => {
                if !valid_transform(center, rotation)
                    || !valid_dimension(radius)
                    || !valid_dimension(height)
                {
                    return None;
                }
                let o = rotation.inverse() * (origin - center);
                let d = rotation.inverse() * direction;
                let a = d.x * d.x + d.z * d.z;
                if a < 1e-12 {
                    return None;
                }
                let b = 2.0 * (o.x * d.x + o.z * d.z);
                let c = o.x * o.x + o.z * o.z - radius * radius;
                let disc = b * b - 4.0 * a * c;
                if disc < 0.0 {
                    return None;
                }
                let root = disc.sqrt();
                let mut hits = [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)];
                hits.sort_by(f32::total_cmp);
                let t = hits.into_iter().find(|t| {
                    *t > 1e-6 && t.is_finite() && (o.y + d.y * *t).abs() <= height * 0.5
                })?;
                Some(origin + direction * t)
            }
            Self::Cone {
                center,
                rotation,
                radius,
                height,
            } => {
                if !valid_transform(center, rotation)
                    || !valid_dimension(radius)
                    || !valid_dimension(height)
                {
                    return None;
                }
                let o = rotation.inverse() * (origin - center);
                let d = rotation.inverse() * direction;
                let k = radius / height;
                let k2 = k * k;
                let a = d.x * d.x + d.z * d.z - k2 * d.y * d.y;
                let b = 2.0 * (o.x * d.x + o.z * d.z + k2 * (height - o.y) * d.y);
                let c = o.x * o.x + o.z * o.z - k2 * (height - o.y) * (height - o.y);
                let roots = quadratic_roots(a, b, c)?;
                let t = roots
                    .into_iter()
                    .filter(|t| {
                        *t > 1e-6 && t.is_finite() && (0.0..=height).contains(&(o.y + d.y * *t))
                    })
                    .min_by(f32::total_cmp)?;
                Some(origin + direction * t)
            }
            Self::Revolved {
                center,
                rotation,
                profile,
            } => {
                if !valid_transform(center, rotation)
                    || !(2..=8).contains(&profile.len())
                    || profile.iter().any(|p| {
                        !p.height.is_finite()
                            || p.height.abs() > 10000.0
                            || !valid_dimension(p.radius)
                    })
                    || !profile
                        .windows(2)
                        .all(|p| p[1].height - p[0].height >= 0.02)
                {
                    return None;
                }
                let inverse = rotation.inverse();
                let o = inverse * (origin - center);
                let d = inverse * direction;
                let mut nearest: Option<f32> = None;
                for pair in profile.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let slope = (b.radius - a.radius) / (b.height - a.height);
                    let intercept = a.radius - slope * a.height;
                    let qa = d.x * d.x + d.z * d.z - slope * slope * d.y * d.y;
                    let qb =
                        2.0 * (o.x * d.x + o.z * d.z - (slope * o.y + intercept) * slope * d.y);
                    let qc = o.x * o.x + o.z * o.z - (slope * o.y + intercept).powi(2);
                    let Some(roots) = quadratic_roots(qa, qb, qc) else {
                        continue;
                    };
                    for t in roots {
                        let y = o.y + d.y * t;
                        if t > 1e-6
                            && t.is_finite()
                            && (a.height..=b.height).contains(&y)
                            && nearest.is_none_or(|previous| t < previous)
                        {
                            nearest = Some(t);
                        }
                    }
                }
                nearest.map(|t| origin + direction * t)
            }
            Self::Freeform { surface } => surface.hit(origin, direction),
            Self::Loft { surface } => surface.hit(origin, direction),
        }
    }
}

impl GuideResource {
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 || self.id == u64::MAX {
            return Err("Guide ID is invalid".into());
        }
        let length = self.name.chars().count();
        if self.name.trim() != self.name
            || length == 0
            || length > MAX_GUIDE_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(format!(
                "Guide name must contain 1–{MAX_GUIDE_NAME_CHARS} printable characters"
            ));
        }
        match &self.geometry {
            GuideGeometry::Plane { origin, rotation } => validate_transform(*origin, *rotation),
            GuideGeometry::Cylinder {
                center,
                rotation,
                radius,
                height,
            }
            | GuideGeometry::Cone {
                center,
                rotation,
                radius,
                height,
            } => {
                validate_transform(*center, *rotation)?;
                validate_dimension(*radius, "Guide radius")?;
                validate_dimension(*height, "Guide height")
            }
            GuideGeometry::Revolved {
                center,
                rotation,
                profile,
            } => {
                validate_transform(*center, *rotation)?;
                if !(2..=8).contains(&profile.len()) {
                    return Err("A revolved guide requires 2–8 profile points".into());
                }
                if profile.iter().any(|p| {
                    !p.height.is_finite() || p.height.abs() > 10000.0 || !valid_dimension(p.radius)
                }) || !profile
                    .windows(2)
                    .all(|p| p[1].height - p[0].height >= 0.02)
                {
                    return Err(
                        "Revolved guide profile must be bounded, positive, and strictly ordered"
                            .into(),
                    );
                }
                Ok(())
            }
            GuideGeometry::Sphere {
                center,
                rotation,
                radius,
            } => {
                validate_transform(*center, *rotation)?;
                validate_dimension(*radius, "Guide radius")
            }
            GuideGeometry::Ring {
                center,
                rotation,
                major_radius,
                minor_radius,
            } => {
                if valid_ring(*center, *rotation, *major_radius, *minor_radius) {
                    Ok(())
                } else {
                    Err("Ring major radius must be greater than its valid tube radius".into())
                }
            }
            GuideGeometry::Freeform { surface } => surface.validate(),
            GuideGeometry::Loft { surface } => surface.validate(),
        }
    }
}

pub fn validate_catalog(guides: &[GuideResource], active: Option<u64>) -> Result<(), String> {
    if guides.len() > MAX_GUIDES {
        return Err(format!(
            "A document can contain at most {MAX_GUIDES} saved guides"
        ));
    }
    let mut ids = HashSet::with_capacity(guides.len());
    for guide in guides {
        guide.validate()?;
        if !ids.insert(guide.id) {
            return Err("Saved guide IDs must be unique".into());
        }
    }
    if active.is_some_and(|id| {
        !guides
            .iter()
            .any(|guide| guide.id == id && guide.visible && !guide.locked)
    }) {
        return Err("Active saved guide must exist, be visible, and be unlocked".into());
    }
    Ok(())
}

fn valid_transform(center: Vec3, rotation: Quat) -> bool {
    center.is_finite()
        && center.abs().max_element() <= 10000.0
        && rotation.is_finite()
        && (rotation.length_squared() - 1.0).abs() <= 0.001
}
fn validate_transform(center: Vec3, rotation: Quat) -> Result<(), String> {
    if valid_transform(center, rotation) {
        Ok(())
    } else {
        Err("Guide transform is invalid or outside document bounds".into())
    }
}
fn valid_dimension(value: f32) -> bool {
    value.is_finite() && (0.001..=10000.0).contains(&value)
}
fn validate_dimension(value: f32, label: &str) -> Result<(), String> {
    if valid_dimension(value) {
        Ok(())
    } else {
        Err(format!(
            "{label} must be between 0.001 and 10000 world units"
        ))
    }
}

fn valid_ring(center: Vec3, rotation: Quat, major_radius: f32, minor_radius: f32) -> bool {
    center.is_finite()
        && center.abs().max_element() <= 10000.0
        && rotation.is_finite()
        && (rotation.length_squared() - 1.0).abs() <= 0.001
        && major_radius.is_finite()
        && minor_radius.is_finite()
        && major_radius > minor_radius
        && minor_radius >= 0.001
        && major_radius <= 10000.0
        && major_radius + minor_radius <= 10000.0
}

fn polynomial_value(coefficients: &[f64], x: f64) -> f64 {
    coefficients
        .iter()
        .rev()
        .fold(0.0, |value, coefficient| value.mul_add(x, *coefficient))
}

/// Isolate every real root in a bounded interval by recursively finding the
/// derivative roots, then bisecting each monotone section. This handles the
/// repeated roots produced by tangent silhouette hits as well as crossings.
fn polynomial_roots_in_interval(coefficients: &[f64], low: f64, high: f64) -> Vec<f64> {
    let mut coefficients = coefficients.to_vec();
    while coefficients.len() > 1 && coefficients.last().is_some_and(|value| value.abs() < 1e-18) {
        coefficients.pop();
    }
    if coefficients.len() <= 1 || !low.is_finite() || !high.is_finite() || high < low {
        return Vec::new();
    }
    if coefficients.len() == 2 {
        let root = -coefficients[0] / coefficients[1];
        return (root >= low && root <= high && root.is_finite())
            .then_some(root)
            .into_iter()
            .collect();
    }
    let derivative: Vec<_> = coefficients
        .iter()
        .enumerate()
        .skip(1)
        .map(|(power, coefficient)| *coefficient * power as f64)
        .collect();
    let mut partitions = vec![low];
    partitions.extend(polynomial_roots_in_interval(&derivative, low, high));
    partitions.push(high);
    partitions.sort_by(f64::total_cmp);
    partitions.dedup_by(|a, b| (*a - *b).abs() <= 1e-10 * a.abs().max(b.abs()).max(1.0));
    let scale = coefficients
        .iter()
        .enumerate()
        .map(|(power, coefficient)| {
            coefficient.abs() * high.abs().max(low.abs()).max(1.0).powi(power as i32)
        })
        .sum::<f64>()
        .max(1.0);
    let tolerance = scale * 1e-12;
    let mut roots = Vec::new();
    for &point in &partitions {
        if polynomial_value(&coefficients, point).abs() <= tolerance {
            roots.push(point);
        }
    }
    for interval in partitions.windows(2) {
        let (mut a, mut b) = (interval[0], interval[1]);
        let mut fa = polynomial_value(&coefficients, a);
        let fb = polynomial_value(&coefficients, b);
        if !fa.is_finite()
            || !fb.is_finite()
            || fa == 0.0
            || fb == 0.0
            || fa.signum() == fb.signum()
        {
            continue;
        }
        for _ in 0..64 {
            let midpoint = (a + b) * 0.5;
            let fm = polynomial_value(&coefficients, midpoint);
            if !fm.is_finite() {
                break;
            }
            if fm.abs() <= tolerance || b - a <= 1e-10 * midpoint.abs().max(1.0) {
                a = midpoint;
                b = midpoint;
                break;
            }
            if fa.signum() == fm.signum() {
                a = midpoint;
                fa = fm;
            } else {
                b = midpoint;
            }
        }
        roots.push((a + b) * 0.5);
    }
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|a, b| (*a - *b).abs() <= 1e-7 * a.abs().max(b.abs()).max(1.0));
    roots
}
fn quadratic_roots(a: f32, b: f32, c: f32) -> Option<Vec<f32>> {
    if !a.is_finite() || !b.is_finite() || !c.is_finite() {
        return None;
    }
    if a.abs() <= 1e-10 {
        if b.abs() <= 1e-10 {
            return None;
        }
        return Some(vec![-c / b]);
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 || !discriminant.is_finite() {
        return None;
    }
    let root = discriminant.sqrt();
    Some(vec![(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)])
}

#[cfg(test)]
mod ring_hit_tests {
    use super::*;

    fn ring(rotation: Quat) -> GuideGeometry {
        GuideGeometry::Ring {
            center: Vec3::new(1.0, -0.5, 2.0),
            rotation,
            major_radius: 2.0,
            minor_radius: 0.5,
        }
    }

    #[test]
    fn ring_selects_nearest_tube_hit_and_accepts_tangent_silhouette() {
        let rotation = Quat::from_rotation_y(0.63) * Quat::from_rotation_x(-0.28);
        let center = Vec3::new(1.0, -0.5, 2.0);
        let geometry = ring(rotation);

        let origin = center + rotation * Vec3::new(0.0, 0.0, 5.2);
        let direction = rotation * -Vec3::Z;
        let hit = geometry.clone().hit(origin, direction).unwrap();
        let expected = center + rotation * Vec3::new(0.0, 0.0, 2.5);
        assert!(hit.distance(expected) < 1e-4, "nearest outer hit: {hit:?}");

        // A ray tangent to the torus's outer silhouette has a repeated quartic
        // root; this guards against dropping tangencies in root isolation.
        let tangent_geometry = ring(Quat::IDENTITY);
        let tangent_origin = center + Vec3::new(2.5, 0.0, -5.0);
        let tangent_direction = Vec3::Z;
        let tangent_hit = tangent_geometry
            .hit(tangent_origin, tangent_direction)
            .unwrap();
        let tangent_expected = center + Vec3::new(2.5, 0.0, 0.0);
        assert!(
            tangent_hit.distance(tangent_expected) < 2e-3,
            "tangent hit: {tangent_hit:?}"
        );

        let hole_origin = center + rotation * Vec3::new(0.0, -5.0, 0.0);
        let hole_direction = rotation * Vec3::Y;
        assert!(
            geometry.hit(hole_origin, hole_direction).is_none(),
            "a ray through the center hole must not hit the torus"
        );

        let invalid = GuideResource {
            id: 1,
            name: "Self-intersecting ring".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Ring {
                center,
                rotation,
                major_radius: 0.5,
                minor_radius: 0.5,
            },
        };
        assert!(invalid.validate().is_err());
    }
}

#[cfg(test)]
mod saved_guide_tests {
    use super::*;

    fn guide(id: u64, name: &str, geometry: GuideGeometry) -> GuideResource {
        GuideResource {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            geometry,
        }
    }

    #[test]
    fn guide_catalog_validates_each_new_primitive_and_active_protection() {
        let guides = vec![
            guide(1, "Sphere", GuideGeometry::sphere(Vec3::ZERO, 1.0)),
            guide(2, "Cone", GuideGeometry::cone(Vec3::X, 0.8, 2.0)),
            guide(
                3,
                "Ring",
                GuideGeometry::Ring {
                    center: Vec3::Y,
                    rotation: Quat::IDENTITY,
                    major_radius: 2.0,
                    minor_radius: 0.5,
                },
            ),
        ];
        assert!(validate_catalog(&guides, Some(2)).is_ok());

        let mut locked = guides.clone();
        locked[1].locked = true;
        assert!(validate_catalog(&locked, Some(2)).is_err());
        let mut hidden = guides.clone();
        hidden[1].visible = false;
        assert!(validate_catalog(&hidden, Some(2)).is_err());
        let mut duplicate = guides.clone();
        duplicate[2].id = duplicate[0].id;
        assert!(validate_catalog(&duplicate, None).is_err());
    }

    #[test]
    fn transformed_sphere_and_cone_intersections_choose_near_hits_and_reject_misses() {
        let center = Vec3::new(1.0, -0.5, 2.0);
        let rotation = Quat::from_rotation_y(0.63) * Quat::from_rotation_x(-0.28);
        let sphere = GuideGeometry::Sphere {
            center,
            rotation,
            radius: 1.0,
        };
        let sphere_origin = center + rotation * Vec3::new(0.0, 0.0, 3.0);
        let sphere_direction = rotation * -Vec3::Z;
        let sphere_hit = sphere.clone().hit(sphere_origin, sphere_direction).unwrap();
        assert!(sphere_hit.distance(center + rotation * Vec3::Z) < 1e-5);
        assert!(sphere.hit(sphere_origin, rotation * Vec3::X).is_none());

        let cone = GuideGeometry::Cone {
            center,
            rotation,
            radius: 1.0,
            height: 2.0,
        };
        let cone_origin = center + rotation * Vec3::new(0.0, 1.0, 3.0);
        let cone_direction = rotation * -Vec3::Z;
        let cone_hit = cone.clone().hit(cone_origin, cone_direction).unwrap();
        assert!(cone_hit.distance(center + rotation * Vec3::new(0.0, 1.0, 0.5)) < 1e-5);
        assert!(cone
            .hit(center + rotation * Vec3::new(0.0, 3.0, 3.0), cone_direction)
            .is_none());
    }
}
