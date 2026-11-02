//! Open surfaces of revolution built from a strictly ordered piecewise-linear profile.
use crate::geometry::{DrawingPlane, Vertex};
use glam::{Quat, Vec3};
use std::f32::consts::{PI, TAU};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfilePoint {
    pub height: f32,
    pub radius: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revolved {
    pub center: Vec3,
    pub rotation_degrees: Vec3,
    pub profile: [ProfilePoint; 8],
    pub count: usize,
}
impl Default for Revolved {
    fn default() -> Self {
        let mut profile = [ProfilePoint {
            height: 0.0,
            radius: 1.0,
        }; 8];
        for (i, (height, radius)) in [
            (-1.5, 0.65),
            (-0.8, 1.05),
            (0.2, 0.9),
            (0.9, 0.55),
            (1.5, 0.7),
        ]
        .into_iter()
        .enumerate()
        {
            profile[i] = ProfilePoint { height, radius };
        }
        Self {
            center: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            profile,
            count: 5,
        }
    }
}
impl Revolved {
    pub fn valid(self) -> bool {
        (2..=8).contains(&self.count)
            && self.center.is_finite()
            && self.center.abs().max_element() <= 1000.0
            && self.rotation_degrees.is_finite()
            && self.profile[..self.count].iter().all(|p| {
                p.height.is_finite()
                    && p.height.abs() <= 1000.0
                    && p.radius.is_finite()
                    && (0.05..=10.0).contains(&p.radius)
            })
            && self.profile[..self.count]
                .windows(2)
                .all(|p| p[1].height - p[0].height >= 0.02 - 1e-6)
    }
    pub fn rotation(self) -> Quat {
        DrawingPlane {
            anchor: Vec3::ZERO,
            rotation_degrees: self.rotation_degrees,
            depth: 0.0,
        }
        .rotation()
    }
    pub fn to_local(self, p: Vec3) -> Vec3 {
        self.rotation().inverse() * (p - self.center)
    }
    pub fn to_world(self, p: Vec3) -> Vec3 {
        self.center + self.rotation() * p
    }
    pub fn min_height(self) -> f32 {
        self.profile[0].height
    }
    pub fn max_height(self) -> f32 {
        self.profile[self.count.clamp(2, 8) - 1].height
    }
    pub fn max_radius(self) -> f32 {
        self.profile[..self.count.min(8)]
            .iter()
            .map(|p| p.radius)
            .fold(0.05, f32::max)
    }
    pub fn reference_radius(self) -> f32 {
        self.max_radius()
    }
    fn segment(self, y: f32) -> usize {
        (0..self.count - 1)
            .find(|i| y <= self.profile[i + 1].height)
            .unwrap_or(self.count - 2)
    }
    pub fn radius_at(self, y: f32) -> f32 {
        let y = y.clamp(self.min_height(), self.max_height());
        let i = self.segment(y);
        let a = self.profile[i];
        let b = self.profile[i + 1];
        a.radius + (b.radius - a.radius) * (y - a.height) / (b.height - a.height)
    }
    pub fn point(self, angle: f32, y: f32) -> Vec3 {
        let y = y.clamp(self.min_height(), self.max_height());
        let r = self.radius_at(y);
        self.to_world(Vec3::new(r * angle.sin(), y, r * angle.cos()))
    }
    pub fn normal(self, world: Vec3) -> Vec3 {
        let p = self.to_local(world);
        let i = self.segment(p.y);
        let a = self.profile[i];
        let b = self.profile[i + 1];
        let radial = Vec3::new(p.x, 0.0, p.z).normalize_or_zero();
        self.rotation()
            * Vec3::new(
                radial.x,
                -(b.radius - a.radius) / (b.height - a.height),
                radial.z,
            )
            .normalize()
    }
    pub fn tangent(self, world: Vec3) -> Vec3 {
        let p = self.to_local(world);
        self.rotation() * Vec3::new(p.z, 0.0, -p.x).normalize_or_zero()
    }
    pub fn hit(self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        if !self.valid() || !origin.is_finite() || !direction.is_finite() {
            return None;
        }
        let o = self.to_local(origin);
        let d = self.rotation().inverse() * direction;
        let mut best = f64::INFINITY;
        for pair in self.profile[..self.count].windows(2) {
            let lo = pair[0];
            let hi = pair[1];
            let k = (hi.radius as f64 - lo.radius as f64) / (hi.height as f64 - lo.height as f64);
            let r = lo.radius as f64 + k * (o.y as f64 - lo.height as f64);
            let rd = k * d.y as f64;
            let a = (d.x as f64).powi(2) + (d.z as f64).powi(2) - rd * rd;
            let b = 2.0 * (o.x as f64 * d.x as f64 + o.z as f64 * d.z as f64 - r * rd);
            let c = (o.x as f64).powi(2) + (o.z as f64).powi(2) - r * r;
            let mut roots = [f64::INFINITY; 2];
            if a.abs() < 1e-12 {
                if b.abs() > 1e-12 {
                    roots[0] = -c / b;
                }
            } else {
                let discriminant = b * b - 4.0 * a * c;
                let tolerance = 1e-12 * (b * b + (4.0 * a * c).abs() + 1.0);
                if discriminant >= -tolerance {
                    let sqrt = discriminant.max(0.0).sqrt();
                    let q = -0.5 * (b + sqrt.copysign(b));
                    if q.abs() > 1e-16 {
                        roots = [q / a, c / q];
                    } else {
                        roots[0] = -b / (2.0 * a);
                    }
                }
            }
            for t in roots {
                if !(0.00001..1000.0).contains(&t) || t >= best {
                    continue;
                }
                let y = o.y as f64 + d.y as f64 * t;
                if y >= lo.height as f64 - 1e-6 && y <= hi.height as f64 + 1e-6 {
                    best = t;
                }
            }
        }
        if !best.is_finite() {
            return None;
        }
        let p = o + d * best as f32;
        Some(self.point(p.x.atan2(p.z), p.y))
    }
    pub fn meridian_knots(self) -> Vec<f32> {
        let mut out = vec![0.0];
        for p in self.profile[..self.count].windows(2) {
            out.push(
                out.last().unwrap() + (p[1].height - p[0].height).hypot(p[1].radius - p[0].radius),
            );
        }
        out
    }
    fn meridian_at(self, y: f32) -> f32 {
        let y = y.clamp(self.min_height(), self.max_height());
        let i = self.segment(y);
        let a = self.profile[i];
        let b = self.profile[i + 1];
        let knots = self.meridian_knots();
        knots[i] + (knots[i + 1] - knots[i]) * (y - a.height) / (b.height - a.height)
    }
    fn height_at(self, s: f32) -> f32 {
        let knots = self.meridian_knots();
        let s = s.clamp(0.0, *knots.last().unwrap());
        let i = (0..self.count - 1)
            .find(|i| s <= knots[i + 1])
            .unwrap_or(self.count - 2);
        self.profile[i].height
            + (self.profile[i + 1].height - self.profile[i].height) * (s - knots[i])
                / (knots[i + 1] - knots[i])
    }
    pub fn parameter(self, world: Vec3, previous: Option<Vec3>) -> Vec3 {
        let p = self.to_local(world);
        let mut a = p.x.atan2(p.z);
        if let Some(previous) = previous {
            let old = previous.x / self.reference_radius();
            a = old + (a - old + PI).rem_euclid(TAU) - PI;
        }
        Vec3::new(a * self.reference_radius(), self.meridian_at(p.y), 0.0)
    }
    pub fn decode(self, parameter: Vec3) -> Vec3 {
        self.point(
            parameter.x / self.reference_radius(),
            self.height_at(parameter.y),
        )
    }
    pub fn wireframe(self) -> Vec<Vertex> {
        if !self.valid() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut line = |a: Vec3, b: Vec3, color| {
            for p in [a, b] {
                out.push(Vertex {
                    material: 0.0,
                    gradient_t: 0.0,
                    position: p.to_array(),
                    normal: [0.0; 3],
                    color,
                    uv: [0.0; 2],
                    opacity: 1.0,
                });
            }
        };
        for p in &self.profile[..self.count] {
            for i in 0..96 {
                line(
                    self.point(i as f32 / 96.0 * TAU, p.height),
                    self.point((i + 1) as f32 / 96.0 * TAU, p.height),
                    [0.2, 0.58, 0.55],
                );
            }
        }
        for i in 0..16 {
            for p in self.profile[..self.count].windows(2) {
                line(
                    self.point(i as f32 / 16.0 * TAU, p[0].height),
                    self.point(i as f32 / 16.0 * TAU, p[1].height),
                    [0.12, 0.38, 0.39],
                );
            }
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ray_hits_rotated_profile_and_inside_without_caps() {
        let r = Revolved {
            center: Vec3::new(2.0, -1.0, 3.0),
            rotation_degrees: Vec3::new(25.0, 30.0, -15.0),
            ..Default::default()
        };
        for y in [-1.5, -0.8, 0.0, 0.9, 1.5] {
            let hit = r
                .hit(r.to_world(Vec3::new(0.0, y, 5.0)), r.rotation() * -Vec3::Z)
                .unwrap();
            assert!(hit.distance(r.point(0.0, y)) < 1e-5);
            assert!(r
                .hit(r.to_world(Vec3::new(0.0, y, 0.0)), r.rotation() * Vec3::X)
                .is_some());
        }
        assert!(r
            .hit(
                r.to_world(Vec3::new(0.0, 3.0, 5.0)),
                r.rotation() * -Vec3::Z
            )
            .is_none());
        assert!(r
            .hit(
                r.to_world(Vec3::new(0.0, 3.0, 0.0)),
                r.rotation() * -Vec3::Y
            )
            .is_none());
    }
    #[test]
    fn linear_cone_case_and_tangent_are_finite() {
        let mut r = Revolved {
            count: 2,
            ..Default::default()
        };
        r.profile[0] = ProfilePoint {
            height: 0.0,
            radius: 1.0,
        };
        r.profile[1] = ProfilePoint {
            height: 2.0,
            radius: 3.0,
        };
        assert!(r
            .hit(Vec3::new(0.0, 0.0, 3.0), Vec3::new(0.0, 1.0, -1.0))
            .is_some());
        r.profile[1].radius = 1.0;
        let hit = r.hit(Vec3::new(1.0, 1.0, 3.0), -Vec3::Z).unwrap();
        assert!(hit.distance(Vec3::new(1.0, 1.0, 0.0)) < 1e-5);
    }
    #[test]
    fn validation_and_parameter_roundtrip() {
        let r = Revolved::default();
        assert!(r.valid());
        for i in 0..31 {
            let y = r.min_height() + (r.max_height() - r.min_height()) * i as f32 / 30.0;
            let p = r.point(2.4, y);
            assert!(r.decode(r.parameter(p, None)).distance(p) < 1e-5);
            assert!(r.normal(p).dot(r.tangent(p)).abs() < 1e-5);
        }
        let mut bad = r;
        bad.count = 9;
        assert!(!bad.valid());
        bad = r;
        bad.profile[1].height = bad.profile[0].height;
        assert!(!bad.valid());
        bad = r;
        bad.profile[1].radius = 0.0;
        assert!(!bad.valid());
    }
}
