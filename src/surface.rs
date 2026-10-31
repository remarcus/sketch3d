//! Side-surface guides and pressure-preserving stabilization in surface coordinates.
use crate::revolved::Revolved;
use crate::{
    document::Point,
    geometry::{self, DrawingPlane, Vertex},
    stabilizer::Stabilizer,
};
use glam::{Quat, Vec3};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cylinder {
    pub center: Vec3,
    pub rotation_degrees: Vec3,
    pub radius: f32,
    pub height: f32,
}
impl Default for Cylinder {
    fn default() -> Self {
        Self {
            center: Vec3::new(0.0, 1.5, 0.0),
            rotation_degrees: Vec3::ZERO,
            radius: 1.0,
            height: 3.0,
        }
    }
}
impl Cylinder {
    pub fn rotation(self) -> Quat {
        DrawingPlane {
            anchor: Vec3::ZERO,
            rotation_degrees: self.rotation_degrees,
            depth: 0.0,
        }
        .rotation()
    }
    pub fn to_local(self, world: Vec3) -> Vec3 {
        self.rotation().inverse() * (world - self.center)
    }
    pub fn to_world(self, local: Vec3) -> Vec3 {
        self.center + self.rotation() * local
    }
    pub fn point(self, angle: f32, y: f32) -> Vec3 {
        self.to_world(Vec3::new(
            self.radius * angle.sin(),
            y,
            self.radius * angle.cos(),
        ))
    }
    pub fn valid(self) -> bool {
        self.center.is_finite()
            && self.rotation_degrees.is_finite()
            && self.radius.is_finite()
            && self.height.is_finite()
            && self.radius > 0.0
            && self.height > 0.0
    }
    /// Open cylinder: choose the nearest forward side hit within the finite height.
    pub fn hit(self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        if !self.valid() {
            return None;
        }
        let o = self.to_local(origin);
        let d = self.rotation().inverse() * direction;
        let a = d.x as f64 * d.x as f64 + d.z as f64 * d.z as f64;
        if a < 1e-10 {
            return None;
        }
        let b = 2.0 * (o.x as f64 * d.x as f64 + o.z as f64 * d.z as f64);
        let c = o.x as f64 * o.x as f64 + o.z as f64 * o.z as f64 - (self.radius as f64).powi(2);
        let discriminant = b * b - 4.0 * a * c;
        if !discriminant.is_finite() || discriminant < 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        for t in [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)] {
            if !(0.00001..1000.0).contains(&t) {
                continue;
            }
            let p = o + d * t as f32;
            if p.y.abs() > self.height * 0.5 + 0.00001 {
                continue;
            }
            return Some(self.point(
                p.x.atan2(p.z),
                p.y.clamp(-self.height * 0.5, self.height * 0.5),
            ));
        }
        None
    }
    fn parameter(self, world: Vec3, previous: Option<Vec3>) -> Vec3 {
        let p = self.to_local(world);
        let mut angle = p.x.atan2(p.z);
        if let Some(previous) = previous {
            let old = previous.x / self.radius;
            angle = old + (angle - old + PI).rem_euclid(TAU) - PI;
        }
        Vec3::new(angle * self.radius, p.y, 0.0)
    }
    fn decode(self, parameter: Vec3) -> Vec3 {
        self.point(parameter.x / self.radius, parameter.y)
    }
    pub fn wireframe(self) -> Vec<Vertex> {
        let mut vertices = Vec::new();
        let color = [0.12, 0.38, 0.39];
        let mut segment = |a: Vec3, b: Vec3, color| {
            for p in [a, b] {
                vertices.push(Vertex {
                    position: p.to_array(),
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                    color,
                    opacity: 1.0,
                });
            }
        };
        for j in 0..=6 {
            let y = -self.height * 0.5 + self.height * j as f32 / 6.0;
            for i in 0..96 {
                let t = i as f32 / 96.0 * TAU;
                let next = (i + 1) as f32 / 96.0 * TAU;
                segment(
                    self.point(t, y),
                    self.point(next, y),
                    if j == 0 || j == 6 {
                        [0.22, 0.62, 0.59]
                    } else {
                        color
                    },
                );
            }
        }
        for i in 0..16 {
            let t = i as f32 / 16.0 * TAU;
            segment(
                self.point(t, -self.height * 0.5),
                self.point(t, self.height * 0.5),
                color,
            );
        }
        vertices
    }
}
#[derive(Clone, Copy, PartialEq)]
pub enum GuidePreview {
    Plane(DrawingPlane),
    Cylinder(Cylinder),
    Revolved(Revolved),
    Hidden,
}
impl GuidePreview {
    pub fn vertices(self) -> Vec<Vertex> {
        match self {
            Self::Plane(plane) => geometry::grid(plane),
            Self::Cylinder(cylinder) => cylinder.wireframe(),
            Self::Revolved(r) => r.wireframe(),
            Self::Hidden => Vec::new(),
        }
    }
}
#[derive(Clone, Copy, Default)]
pub enum Surface {
    #[default]
    Plane,
    Cylinder(Cylinder),
    Revolved(Revolved),
}
impl Surface {
    fn encode(self, world: Vec3, previous: Option<Vec3>) -> Vec3 {
        match self {
            Self::Plane => world,
            Self::Cylinder(c) => c.parameter(world, previous),
            Self::Revolved(r) => r.parameter(world, previous),
        }
    }
    fn decode(self, point: Point) -> Point {
        Point {
            tilt: point.tilt,
            position: match self {
                Self::Plane => point.position,
                Self::Cylinder(c) => c.decode(point.position),
                Self::Revolved(r) => r.decode(point.position),
            },
            pressure: point.pressure,
        }
    }
    fn steps(self, start: Vec3, end: Vec3) -> usize {
        match self {
            Self::Plane => 1,
            Self::Revolved(r) => (((end.x - start.x).abs() / r.reference_radius() / 0.035).ceil()
                as usize)
                .clamp(1, 128),
            Self::Cylinder(c) => {
                (((end.x - start.x).abs() / c.radius / 0.035).ceil() as usize).clamp(1, 128)
            }
        }
    }
}
#[derive(Default)]
pub struct SurfaceStabilizer {
    surface: Surface,
    filter: Stabilizer,
    last_raw: Option<Vec3>,
    last_filtered: Option<Point>,
}
impl SurfaceStabilizer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn start(&mut self, surface: Surface) {
        self.reset();
        self.surface = surface;
    }
    /// A large discontinuous side change should become a new stroke, not a bridge.
    pub fn discontinuity(&self, world: Vec3) -> bool {
        match (self.surface, self.last_raw) {
            (Surface::Revolved(r), Some(previous)) => {
                (r.parameter(world, Some(previous)).x - previous.x).abs() / r.reference_radius()
                    > std::f32::consts::FRAC_PI_2
            }
            (Surface::Cylinder(c), Some(previous)) => {
                (c.parameter(world, Some(previous)).x - previous.x).abs() / c.radius
                    > std::f32::consts::FRAC_PI_2
            }
            _ => false,
        }
    }
    fn emit(&mut self, filtered: Point) -> Vec<Point> {
        let mut points = Vec::new();
        if let Some(previous) = self.last_filtered {
            let steps = self.surface.steps(previous.position, filtered.position);
            let mut fractions: Vec<f32> = (1..=steps).map(|i| i as f32 / steps as f32).collect();
            if let Surface::Revolved(r) = self.surface {
                let delta = filtered.position.y - previous.position.y;
                if delta.abs() > 1e-8 {
                    for knot in r.meridian_knots() {
                        let t = (knot - previous.position.y) / delta;
                        if t > 1e-6 && t < 1.0 - 1e-6 {
                            fractions.push(t);
                        }
                    }
                }
            }
            fractions.sort_by(f32::total_cmp);
            fractions.dedup_by(|a, b| *a == *b);
            for t in fractions {
                let p = if t == 1.0 {
                    filtered
                } else {
                    Point {
                        tilt: crate::dynamics::interpolate_tilt(previous.tilt, filtered.tilt, t),
                        position: previous.position.lerp(filtered.position, t),
                        pressure: previous.pressure + (filtered.pressure - previous.pressure) * t,
                    }
                };
                points.push(self.surface.decode(p));
            }
        } else {
            points.push(self.surface.decode(filtered));
        }
        self.last_filtered = Some(filtered);
        points
    }
    pub fn push(&mut self, world: Point, radius: f32) -> Vec<Point> {
        let parameter = self.surface.encode(world.position, self.last_raw);
        self.last_raw = Some(parameter);
        let filtered = self.filter.push(
            Point {
                tilt: world.tilt,
                position: parameter,
                pressure: world.pressure,
            },
            radius,
        );
        self.emit(filtered)
    }
    pub fn finish(&mut self) -> Vec<Point> {
        let mut points = Vec::new();
        for p in self.filter.finish() {
            points.extend(self.emit(p));
        }
        self.reset();
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cylinder() -> Cylinder {
        Cylinder {
            center: Vec3::new(2.0, 0.5, -1.0),
            rotation_degrees: Vec3::new(35.0, -20.0, 65.0),
            radius: 1.2,
            height: 3.0,
        }
    }
    #[test]
    fn nearest_rotated_side_hit_and_miss_cases() {
        let c = cylinder();
        let o = c.to_world(Vec3::new(0.0, 0.4, 5.0));
        let d = c.rotation() * -Vec3::Z;
        let hit = c.hit(o, d).unwrap();
        assert!(hit.distance(c.point(0.0, 0.4)) < 0.00001);
        assert!(c.hit(o, -d).is_none());
        assert!(c.hit(c.to_world(Vec3::new(0.0, 3.0, 5.0)), d).is_none());
        assert!(c
            .hit(
                c.to_world(Vec3::new(0.0, 3.0, 0.0)),
                c.rotation() * -Vec3::Y
            )
            .is_none());
        assert!(
            c.hit(c.center, c.rotation() * Vec3::X)
                .unwrap()
                .distance(c.point(PI * 0.5, 0.0))
                < 0.00001
        );
    }
    #[test]
    fn angular_seam_is_continuous_with_surface_bound_stabilization() {
        let c = cylinder();
        let mut filter = SurfaceStabilizer::default();
        filter.start(Surface::Cylinder(c));
        let mut out = Vec::new();
        let mut last = Vec3::ZERO;
        for i in 0..80 {
            let angle = 3.0 + i as f32 * 0.004;
            let world = c.point(angle, (i as f32 * 0.4).sin() * 0.04);
            assert!(!filter.discontinuity(world));
            last = world;
            out.extend(filter.push(
                Point {
                    tilt: None,
                    position: world,
                    pressure: i as f32 / 80.0,
                },
                0.08,
            ));
        }
        out.extend(filter.finish());
        assert!(out.last().unwrap().position.distance(last) < 0.00001);
        for p in &out {
            let local = c.to_local(p.position);
            assert!((Vec3::new(local.x, 0.0, local.z).length() - c.radius).abs() < 0.00001);
            assert!((0.0..=1.0).contains(&p.pressure));
        }
        assert!(out
            .windows(2)
            .all(|p| p[0].position.distance(p[1].position) < 0.1));
    }
    #[test]
    fn sparse_samples_are_resampled_on_surface_and_large_jumps_are_detected() {
        let c = cylinder();
        let mut filter = SurfaceStabilizer::default();
        filter.start(Surface::Cylinder(c));
        filter.push(
            Point {
                tilt: None,
                position: c.point(0.0, 0.0),
                pressure: 0.0,
            },
            0.0,
        );
        assert!(filter.discontinuity(c.point(2.0, 0.0)));
        let points = filter.push(
            Point {
                tilt: None,
                position: c.point(1.0, 0.5),
                pressure: 1.0,
            },
            0.0,
        );
        assert!(points.len() > 20);
        for point in points {
            let p = c.to_local(point.position);
            assert!((p.x.hypot(p.z) - c.radius).abs() < 0.00001);
        }
    }
    #[test]
    fn finishing_uses_the_captured_surface_after_guide_changes() {
        let c = cylinder();
        let mut filter = SurfaceStabilizer::default();
        filter.start(Surface::Cylinder(c));
        filter.push(
            Point {
                tilt: None,
                position: c.point(0.0, 0.0),
                pressure: 0.5,
            },
            0.2,
        );
        let target = c.point(0.4, 0.3);
        filter.push(
            Point {
                tilt: None,
                position: target,
                pressure: 0.7,
            },
            0.2,
        );
        let mut changed = c;
        changed.radius = 2.0;
        assert_ne!(changed, c);
        let tail = filter.finish();
        assert!(tail.last().unwrap().position.distance(target) < 0.00001);
        assert_eq!(tail.last().unwrap().pressure, 0.7);
    }
}

#[cfg(test)]
mod tilt_tests {
    use super::*;
    #[test]
    fn cylinder_resampling_interpolates_optional_tilt_and_preserves_surface() {
        let cylinder = Cylinder::default();
        let mut filter = SurfaceStabilizer::default();
        filter.start(Surface::Cylinder(cylinder));
        filter.push(
            Point {
                position: cylinder.point(0.0, 0.0),
                pressure: 0.2,
                tilt: Some(Vec3::X),
            },
            0.0,
        );
        let points = filter.push(
            Point {
                position: cylinder.point(0.4, 0.2),
                pressure: 0.8,
                tilt: Some(Vec3::Y),
            },
            0.0,
        );
        assert!(points.len() > 5);
        assert!(points
            .iter()
            .all(|p| p.tilt.is_some_and(|t| (t.length() - 1.0).abs() < 1e-6)));
        assert_eq!(points.last().unwrap().tilt, Some(Vec3::Y));
        let points = filter.push(
            Point {
                position: cylinder.point(0.8, 0.3),
                pressure: 0.8,
                tilt: None,
            },
            0.0,
        );
        assert!(points.iter().all(|p| p.tilt.is_none()));
        for point in points {
            let local = cylinder.to_local(point.position);
            assert!((local.x.hypot(local.z) - cylinder.radius).abs() < 1e-5);
        }
    }
}

#[cfg(test)]
mod revolved_tests {
    use super::*;
    #[test]
    fn seam_release_and_profile_knots_remain_on_captured_surface() {
        let r = Revolved::default();
        let mut filter = SurfaceStabilizer::default();
        filter.start(Surface::Revolved(r));
        let mut out = filter.push(
            Point {
                position: r.point(PI - 0.1, -1.4),
                pressure: 0.3,
                tilt: Some(Vec3::Y),
            },
            0.1,
        );
        let target = r.point(-PI + 0.1, 1.4);
        assert!(!filter.discontinuity(target));
        out.extend(filter.push(
            Point {
                position: target,
                pressure: 0.8,
                tilt: Some(Vec3::Y),
            },
            0.1,
        ));
        out.extend(filter.finish());
        assert!(out.last().unwrap().position.distance(target) < 1e-5);
        for p in &out {
            let q = r.to_local(p.position);
            assert!((q.x.hypot(q.z) - r.radius_at(q.y)).abs() < 1e-5);
            assert!(p.tilt.unwrap().is_finite());
        }
        for knot in &r.profile[1..r.count - 1] {
            assert!(out
                .iter()
                .any(|p| (r.to_local(p.position).y - knot.height).abs() < 1e-5));
        }
        assert_eq!(out.last().unwrap().pressure, 0.8);
    }
}
