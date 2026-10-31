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
