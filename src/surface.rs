//! Side-surface guides and pressure-preserving stabilization in surface coordinates.
use crate::revolved::Revolved;
use crate::{
    document::guide_resources::{FreeformSurface, LoftSurface},
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
    /// Map a world-space cylinder contact into the unwrapped surface chart.
    /// The first coordinate is circumferential arc length and the second is
    /// axial distance. Supplying a previous parameter keeps seam crossings
    /// continuous.
    pub fn parameterize(self, world: Vec3, previous: Option<Vec3>) -> Option<Vec3> {
        if !self.valid() || !world.is_finite() {
            return None;
        }
        let parameter = self.parameter(world, previous);
        (parameter.is_finite() && parameter.y.abs() <= self.height * 0.5 + 1.0e-5)
            .then_some(parameter)
    }

    /// Map an unwrapped cylinder chart coordinate onto the finite side.
    pub fn point_from_parameter(self, parameter: Vec3) -> Option<Vec3> {
        if !self.valid()
            || !parameter.is_finite()
            || parameter.y.abs() > self.height * 0.5 + 1.0e-5
            || parameter.x.abs() > 1_000_000.0
        {
            return None;
        }
        let point = self.decode(parameter);
        point.is_finite().then_some(point)
    }
    pub fn wireframe(self) -> Vec<Vertex> {
        let mut vertices = Vec::new();
        let color = [0.12, 0.38, 0.39];
        let mut segment = |a: Vec3, b: Vec3, color| {
            for p in [a, b] {
                vertices.push(Vertex {
                    material: 0.0,
                    emission: [0.0; 3],
                    grain_uv: [0.0; 2],
                    grain_dynamics: [1.0; 2],
                    gradient_t: 0.0,
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

/// Finite smooth sphere used by persistent guide resources.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SphereSurface {
    pub center: Vec3,
    pub rotation: Quat,
    pub radius: f32,
}

impl SphereSurface {
    pub fn point(self, longitude: f32, latitude: f32) -> Vec3 {
        let c = latitude.cos();
        self.center
            + self.rotation
                * Vec3::new(
                    self.radius * c * longitude.sin(),
                    self.radius * latitude.sin(),
                    self.radius * c * longitude.cos(),
                )
    }

    fn parameter(self, world: Vec3, previous: Option<Vec3>) -> Vec3 {
        let local = self.rotation.inverse() * (world - self.center);
        let unit = local.normalize_or_zero();
        let horizontal = unit.x.hypot(unit.z);
        let mut longitude = if horizontal < 1e-5 {
            previous.map_or(0.0, |p| p.x / self.radius)
        } else {
            unit.x.atan2(unit.z)
        };
        if let Some(previous) = previous {
            longitude = unwrap_angle(longitude, previous.x / self.radius);
        }
        Vec3::new(
            longitude * self.radius,
            unit.y.clamp(-1.0, 1.0).asin() * self.radius,
            0.0,
        )
    }

    fn decode(self, parameter: Vec3) -> Vec3 {
        self.point(parameter.x / self.radius, parameter.y / self.radius)
    }

    pub fn wireframe(self) -> Vec<Vertex> {
        let mut vertices = Vec::new();
        let color = [0.12, 0.38, 0.39];
        let mut segment = |a: Vec3, b: Vec3| {
            for position in [a, b] {
                vertices.push(Vertex {
                    material: 0.0,
                    emission: [0.0; 3],
                    grain_uv: [0.0; 2],
                    grain_dynamics: [1.0; 2],
                    gradient_t: 0.0,
                    position: position.to_array(),
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                    color,
                    opacity: 1.0,
                });
            }
        };
        for latitude_index in 1..8 {
            let latitude = -PI * 0.5 + PI * latitude_index as f32 / 8.0;
            for i in 0..64 {
                let a = i as f32 / 64.0 * TAU;
                let b = (i + 1) as f32 / 64.0 * TAU;
                segment(self.point(a, latitude), self.point(b, latitude));
            }
        }
        for longitude_index in 0..12 {
            let longitude = longitude_index as f32 / 12.0 * TAU;
            for i in 0..32 {
                let a = -PI * 0.5 + PI * i as f32 / 32.0;
                let b = -PI * 0.5 + PI * (i + 1) as f32 / 32.0;
                segment(self.point(longitude, a), self.point(longitude, b));
            }
        }
        vertices
    }
}

/// Smooth toroidal ring surface. Both periodic parameters are stored in world
/// units so stabilization can cross each seam without jumping back to zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingSurface {
    pub center: Vec3,
    pub rotation: Quat,
    pub major_radius: f32,
    pub minor_radius: f32,
}

impl RingSurface {
    pub fn point(self, major_angle: f32, minor_angle: f32) -> Vec3 {
        let tube_radius = self.major_radius + self.minor_radius * minor_angle.cos();
        self.center
            + self.rotation
                * Vec3::new(
                    tube_radius * major_angle.sin(),
                    self.minor_radius * minor_angle.sin(),
                    tube_radius * major_angle.cos(),
                )
    }

    fn parameter(self, world: Vec3, previous: Option<Vec3>) -> Vec3 {
        let local = self.rotation.inverse() * (world - self.center);
        let radial = local.x.hypot(local.z);
        let mut major = local.x.atan2(local.z);
        let mut minor = local.y.atan2(radial - self.major_radius);
        if let Some(previous) = previous {
            major = unwrap_angle(major, previous.x / self.major_radius);
            minor = unwrap_angle(minor, previous.y / self.minor_radius);
        }
        Vec3::new(major * self.major_radius, minor * self.minor_radius, 0.0)
    }

    fn decode(self, parameter: Vec3) -> Vec3 {
        self.point(
            parameter.x / self.major_radius,
            parameter.y / self.minor_radius,
        )
    }

    pub fn wireframe(self) -> Vec<Vertex> {
        let mut vertices = Vec::new();
        let color = [0.12, 0.38, 0.39];
        let mut segment = |a: Vec3, b: Vec3| {
            for position in [a, b] {
                vertices.push(Vertex {
                    material: 0.0,
                    emission: [0.0; 3],
                    grain_uv: [0.0; 2],
                    grain_dynamics: [1.0; 2],
                    gradient_t: 0.0,
                    position: position.to_array(),
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                    color,
                    opacity: 1.0,
                });
            }
        };
        for minor_index in 0..=8 {
            let minor = minor_index as f32 / 8.0 * TAU;
            for major_index in 0..64 {
                let a = major_index as f32 / 64.0 * TAU;
                let b = (major_index + 1) as f32 / 64.0 * TAU;
                segment(self.point(a, minor), self.point(b, minor));
            }
        }
        for major_index in 0..=12 {
            let major = major_index as f32 / 12.0 * TAU;
            for minor_index in 0..32 {
                let a = minor_index as f32 / 32.0 * TAU;
                let b = (minor_index + 1) as f32 / 32.0 * TAU;
                segment(self.point(major, a), self.point(major, b));
            }
        }
        vertices
    }
}

/// Finite cone side with a circular base at local Y=0 and a tip at Y=height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeSurface {
    pub center: Vec3,
    pub rotation: Quat,
    pub radius: f32,
    pub height: f32,
}

impl ConeSurface {
    pub fn point(self, longitude: f32, y: f32) -> Vec3 {
        let y = y.clamp(0.0, self.height);
        let radius = self.radius * (1.0 - y / self.height);
        self.center
            + self.rotation * Vec3::new(radius * longitude.sin(), y, radius * longitude.cos())
    }

    fn parameter(self, world: Vec3, previous: Option<Vec3>) -> Vec3 {
        let local = self.rotation.inverse() * (world - self.center);
        let ring_radius = self.radius * (1.0 - local.y / self.height).max(0.0);
        let mut longitude = if ring_radius < 1e-5 {
            previous.map_or(0.0, |p| p.x / self.radius)
        } else {
            local.x.atan2(local.z)
        };
        if let Some(previous) = previous {
            longitude = unwrap_angle(longitude, previous.x / self.radius);
        }
        let slant_scale = (1.0 + (self.radius / self.height).powi(2)).sqrt();
        Vec3::new(longitude * self.radius, local.y * slant_scale, 0.0)
    }

    fn decode(self, parameter: Vec3) -> Vec3 {
        let slant_scale = (1.0 + (self.radius / self.height).powi(2)).sqrt();
        self.point(parameter.x / self.radius, parameter.y / slant_scale)
    }

    pub fn wireframe(self) -> Vec<Vertex> {
        let mut vertices = Vec::new();
        let color = [0.12, 0.38, 0.39];
        let mut segment = |a: Vec3, b: Vec3| {
            for position in [a, b] {
                vertices.push(Vertex {
                    material: 0.0,
                    emission: [0.0; 3],
                    grain_uv: [0.0; 2],
                    grain_dynamics: [1.0; 2],
                    gradient_t: 0.0,
                    position: position.to_array(),
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                    color,
                    opacity: 1.0,
                });
            }
        };
        for ring_index in 0..=4 {
            let y = self.height * ring_index as f32 / 5.0;
            for i in 0..64 {
                let a = i as f32 / 64.0 * TAU;
                let b = (i + 1) as f32 / 64.0 * TAU;
                segment(self.point(a, y), self.point(b, y));
            }
        }
        for i in 0..12 {
            let longitude = i as f32 / 12.0 * TAU;
            segment(
                self.point(longitude, 0.0),
                self.point(longitude, self.height),
            );
        }
        vertices
    }
}

fn unwrap_angle(angle: f32, previous: f32) -> f32 {
    previous + (angle - previous + PI).rem_euclid(TAU) - PI
}

#[derive(Clone, Copy, PartialEq)]
#[allow(clippy::large_enum_variant)] // Preview values are copied through the active-guide UI.
pub enum GuidePreview {
    Plane(DrawingPlane),
    Cylinder(Cylinder),
    Revolved(Revolved),
    Sphere(SphereSurface),
    Ring(RingSurface),
    Cone(ConeSurface),
    Freeform(FreeformSurface),
    Loft(LoftSurface),
    Hidden,
}
impl GuidePreview {
    pub fn vertices(self) -> Vec<Vertex> {
        match self {
            Self::Plane(plane) => geometry::grid(plane),
            Self::Cylinder(cylinder) => cylinder.wireframe(),
            Self::Revolved(r) => r.wireframe(),
            Self::Sphere(sphere) => sphere.wireframe(),
            Self::Ring(ring) => ring.wireframe(),
            Self::Cone(cone) => cone.wireframe(),
            Self::Freeform(surface) => {
                let mut vertices = Vec::new();
                let color = [0.12, 0.38, 0.39];
                let mut segment = |a: Vec3, b: Vec3| {
                    for position in [a, b] {
                        vertices.push(Vertex {
                            material: 0.0,
                            emission: [0.0; 3],
                            grain_uv: [0.0; 2],
                            grain_dynamics: [1.0; 2],
                            gradient_t: 0.0,
                            position: position.to_array(),
                            normal: [0.0; 3],
                            uv: [0.0; 2],
                            color,
                            opacity: 1.0,
                        });
                    }
                };
                for row in 0..=8 {
                    let v = row as f32 / 8.0;
                    for column in 0..32 {
                        let u0 = column as f32 / 32.0;
                        let u1 = (column + 1) as f32 / 32.0;
                        segment(surface.point(u0, v), surface.point(u1, v));
                    }
                }
                for column in 0..=16 {
                    let u = column as f32 / 16.0;
                    for row in 0..8 {
                        let v0 = row as f32 / 8.0;
                        let v1 = (row + 1) as f32 / 8.0;
                        segment(surface.point(u, v0), surface.point(u, v1));
                    }
                }
                vertices
            }
            Self::Loft(surface) => {
                let mut vertices = Vec::new();
                let color = [0.12, 0.38, 0.39];
                let mut segment = |a: Vec3, b: Vec3| {
                    for position in [a, b] {
                        vertices.push(Vertex {
                            material: 0.0,
                            emission: [0.0; 3],
                            grain_uv: [0.0; 2],
                            grain_dynamics: [1.0; 2],
                            gradient_t: 0.0,
                            position: position.to_array(),
                            normal: [0.0; 3],
                            uv: [0.0; 2],
                            color,
                            opacity: 1.0,
                        });
                    }
                };
                let v_steps = (surface.sections.len() - 1) * 8;
                for row in 0..=v_steps {
                    let v = row as f32 / v_steps as f32;
                    for column in 0..32 {
                        segment(
                            surface.point(column as f32 / 32.0, v),
                            surface.point((column + 1) as f32 / 32.0, v),
                        );
                    }
                }
                for column in 0..=16 {
                    let u = column as f32 / 16.0;
                    for row in 0..v_steps {
                        segment(
                            surface.point(u, row as f32 / v_steps as f32),
                            surface.point(u, (row + 1) as f32 / v_steps as f32),
                        );
                    }
                }
                vertices
            }
            Self::Hidden => Vec::new(),
        }
    }
}
#[derive(Clone, Copy, Default)]
#[allow(clippy::large_enum_variant)] // Surface values are Copy geometry adapters used by tool hot paths.
pub enum Surface {
    #[default]
    Plane,
    Cylinder(Cylinder),
    Revolved(Revolved),
    Sphere(SphereSurface),
    Ring(RingSurface),
    Cone(ConeSurface),
    Freeform(FreeformSurface),
    Loft(LoftSurface),
}
impl Surface {
    fn parameter_in_domain(self, parameter: Vec3) -> bool {
        match self {
            Self::Cylinder(cylinder) => parameter.y.abs() <= cylinder.height * 0.5,
            Self::Revolved(revolved) => {
                (0.0..=*revolved.meridian_knots().last().unwrap_or(&0.0)).contains(&parameter.y)
            }
            Self::Sphere(sphere) => {
                parameter.y.abs() <= sphere.radius * std::f32::consts::FRAC_PI_2
            }
            Self::Cone(cone) => {
                let slant = (1.0 + (cone.radius / cone.height).powi(2)).sqrt();
                (0.0..=cone.height * slant).contains(&parameter.y)
            }
            Self::Freeform(_) | Self::Loft(_) => {
                (0.0..=1.0).contains(&parameter.x) && (0.0..=1.0).contains(&parameter.y)
            }
            _ => true,
        }
    }

    pub fn project_to_surface(self, world: Vec3) -> Option<Vec3> {
        if !world.is_finite() {
            return None;
        }
        let parameter = self.encode(world, None);
        if !self.parameter_in_domain(parameter) {
            return None;
        }
        let projected = self
            .decode(Point {
                position: parameter,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position;
        if !projected.is_finite() {
            return None;
        }
        match self {
            Self::Revolved(revolved) => {
                let local = revolved.to_local(projected);
                (local.y >= revolved.min_height() && local.y <= revolved.max_height())
                    .then_some(projected)
            }
            Self::Cylinder(cylinder)
                if cylinder.to_local(projected).y.abs() > cylinder.height * 0.5 =>
            {
                None
            }
            Self::Cone(cone) => {
                let local = cone.rotation.inverse() * (projected - cone.center);
                (local.y >= 0.0 && local.y <= cone.height).then_some(projected)
            }
            _ => Some(projected),
        }
    }

    pub fn normal_at(self, world: Vec3) -> Option<Vec3> {
        let parameter = self.encode(world, None);
        let epsilon = 1.0e-3;
        let base = self
            .decode(Point {
                position: parameter,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position;
        let du = self
            .decode(Point {
                position: parameter + Vec3::X * epsilon,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position
            - base;
        let dv = self
            .decode(Point {
                position: parameter + Vec3::Y * epsilon,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position
            - base;
        let mut normal = du.cross(dv).try_normalize()?;
        if normal.dot(world - base) < 0.0 {
            normal = -normal;
        }
        Some(normal)
    }

    /// Map a small tangent-plane offset to the same analytic surface using its
    /// local metric. The returned point is rejected when it leaves a finite
    /// cylinder or cone boundary.
    pub fn offset_tangent(self, world: Vec3, tangent_offset: Vec3) -> Option<Vec3> {
        if !world.is_finite() || !tangent_offset.is_finite() {
            return None;
        }
        let parameter = self.encode(world, None);
        let base = self
            .decode(Point {
                position: parameter,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position;
        let epsilon = 1.0e-3;
        let du = (self
            .decode(Point {
                position: parameter + Vec3::X * epsilon,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position
            - base)
            / epsilon;
        let dv = (self
            .decode(Point {
                position: parameter + Vec3::Y * epsilon,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position
            - base)
            / epsilon;
        let (a, b, c) = (du.dot(du), du.dot(dv), dv.dot(dv));
        let determinant = a * c - b * b;
        if determinant <= 1.0e-10 || !determinant.is_finite() {
            return None;
        }
        let (x, y) = (du.dot(tangent_offset), dv.dot(tangent_offset));
        let delta = Vec3::new(
            (c * x - b * y) / determinant,
            (a * y - b * x) / determinant,
            0.0,
        );
        if !self.parameter_in_domain(parameter + delta) {
            return None;
        }
        let projected = self
            .decode(Point {
                position: parameter + delta,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .position;
        if !projected.is_finite() {
            return None;
        }
        let base_normal = self.normal_at(world)?;
        let projected_normal = self.normal_at(projected)?;
        if base_normal.dot(projected_normal) < 0.5 {
            return None;
        }
        Some(projected)
    }

    fn encode(self, world: Vec3, previous: Option<Vec3>) -> Vec3 {
        match self {
            Self::Plane => world,
            Self::Cylinder(c) => c.parameter(world, previous),
            Self::Revolved(r) => r.parameter(world, previous),
            Self::Sphere(sphere) => sphere.parameter(world, previous),
            Self::Ring(ring) => ring.parameter(world, previous),
            Self::Cone(cone) => cone.parameter(world, previous),
            Self::Freeform(surface) => surface
                .coordinates(world)
                .map_or(Vec3::ZERO, |(u, v)| Vec3::new(u, v, 0.0)),
            Self::Loft(surface) => surface
                .coordinates(world)
                .map_or(Vec3::ZERO, |(u, v)| Vec3::new(u, v, 0.0)),
        }
    }
    fn decode(self, point: Point) -> Point {
        Point {
            tilt: point.tilt,
            position: match self {
                Self::Plane => point.position,
                Self::Cylinder(c) => c.decode(point.position),
                Self::Revolved(r) => r.decode(point.position),
                Self::Sphere(sphere) => sphere.decode(point.position),
                Self::Ring(ring) => ring.decode(point.position),
                Self::Cone(cone) => cone.decode(point.position),
                Self::Freeform(surface) => surface.point(point.position.x, point.position.y),
                Self::Loft(surface) => surface.point(point.position.x, point.position.y),
            },
            pressure: point.pressure,
            time_ms: point.time_ms,
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
            Self::Sphere(sphere) => {
                (((end - start).length() / sphere.radius / 0.035).ceil() as usize).clamp(1, 128)
            }
            Self::Ring(ring) => {
                let major = (end.x - start.x).abs() / ring.major_radius;
                let minor = (end.y - start.y).abs() / ring.minor_radius;
                (((major + minor) / 0.035).ceil() as usize).clamp(1, 128)
            }
            Self::Cone(cone) => {
                (((end - start).length() / cone.radius / 0.035).ceil() as usize).clamp(1, 128)
            }
            Self::Freeform(_) => (((end - start).length() / 0.02).ceil() as usize).clamp(1, 128),
            Self::Loft(_) => (((end - start).length() / 0.02).ceil() as usize).clamp(1, 128),
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
            (Surface::Sphere(sphere), Some(previous)) => {
                let current = sphere.parameter(world, Some(previous));
                (current.x - previous.x).abs() / sphere.radius > PI * 0.5
                    || (current.y - previous.y).abs() / sphere.radius > PI * 0.5
            }
            (Surface::Ring(ring), Some(previous)) => {
                let current = ring.parameter(world, Some(previous));
                (current.x - previous.x).abs() / ring.major_radius > PI * 0.5
                    || (current.y - previous.y).abs() / ring.minor_radius > PI * 0.5
            }
            (Surface::Cone(cone), Some(previous)) => {
                let current = cone.parameter(world, Some(previous));
                (current.x - previous.x).abs() / cone.radius > PI * 0.5
                    || (current.y - previous.y).abs() > cone.height * 0.5
            }
            (Surface::Freeform(_), Some(previous)) => {
                let current = self.surface.encode(world, Some(previous));
                (current - previous).length() > 0.25
            }
            (Surface::Loft(_), Some(previous)) => {
                let current = self.surface.encode(world, Some(previous));
                (current - previous).length() > 0.25
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
                        time_ms: (previous.time_ms as f32
                            + (filtered.time_ms as f32 - previous.time_ms as f32) * t)
                            .round() as u32,
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
                time_ms: world.time_ms,
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

                    time_ms: 0,
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

                time_ms: 0,
            },
            0.0,
        );
        assert!(filter.discontinuity(c.point(2.0, 0.0)));
        let points = filter.push(
            Point {
                tilt: None,
                position: c.point(1.0, 0.5),
                pressure: 1.0,

                time_ms: 0,
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

                time_ms: 0,
            },
            0.2,
        );
        let target = c.point(0.4, 0.3);
        filter.push(
            Point {
                tilt: None,
                position: target,
                pressure: 0.7,

                time_ms: 0,
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
mod ring_tests {
    use super::*;

    #[test]
    fn ring_crosses_both_periodic_seams_without_leaving_captured_surface() {
        let ring = RingSurface {
            center: Vec3::new(0.4, -0.3, 0.8),
            rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.2),
            major_radius: 2.0,
            minor_radius: 0.5,
        };
        let mut stabilizer = SurfaceStabilizer::default();
        stabilizer.start(Surface::Ring(ring));
        let start = ring.point(PI - 0.08, PI - 0.08);
        stabilizer.push(
            Point {
                position: start,
                pressure: 0.2,
                tilt: Some(Vec3::Y),

                time_ms: 0,
            },
            0.0,
        );
        let target = ring.point(-PI + 0.08, -PI + 0.08);
        assert!(!stabilizer.discontinuity(target));
        let mut points = stabilizer.push(
            Point {
                position: target,
                pressure: 0.8,
                tilt: Some(Vec3::X),

                time_ms: 0,
            },
            0.0,
        );
        points.extend(stabilizer.finish());

        assert!(points.len() > 3, "cross-seam input is resampled");
        assert!(points.last().unwrap().position.distance(target) < 1e-5);
        assert_eq!(points.last().unwrap().pressure, 0.8);
        assert_eq!(points.last().unwrap().tilt, Some(Vec3::X));
        for point in &points {
            let local = ring.rotation.inverse() * (point.position - ring.center);
            let radial = local.x.hypot(local.z);
            let torus_distance = (radial - ring.major_radius).hypot(local.y);
            assert!((torus_distance - ring.minor_radius).abs() < 1e-4);
        }
        assert!(points
            .windows(2)
            .all(|pair| pair[0].position.distance(pair[1].position) < 0.2));

        let document = crate::document::Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![crate::document::Stroke {
                model_surface: None,
                gradient: None,
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                curve: None,
                material: Default::default(),
                fill: None,
                group_id: None,
                layer_id: 1,
                id: 1,
                color: [0.2, 0.6, 0.8],
                radius: 0.02,
                brush: Default::default(),
                points,
            }],
            ..crate::document::Document::default()
        };
        let glb = crate::export::encode(&document, None).unwrap();
        assert_eq!(glb.summary.strokes, 1);
        assert_eq!(
            gltf::Gltf::from_slice(&glb.bytes).unwrap().meshes().count(),
            1
        );
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

                time_ms: 0,
            },
            0.0,
        );
        let points = filter.push(
            Point {
                position: cylinder.point(0.4, 0.2),
                pressure: 0.8,
                tilt: Some(Vec3::Y),

                time_ms: 0,
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

                time_ms: 0,
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
mod sphere_cone_guide_tests {
    use super::*;

    #[test]
    fn sphere_stabilization_crosses_longitude_seam_and_remains_stable_at_poles() {
        let sphere = SphereSurface {
            center: Vec3::new(0.4, -0.3, 0.8),
            rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.2),
            radius: 1.5,
        };
        let mut stabilizer = SurfaceStabilizer::default();
        stabilizer.start(Surface::Sphere(sphere));
        let seam_world = [sphere.point(PI - 0.08, 0.2), sphere.point(-PI + 0.08, 0.21)];
        let mut output = Vec::new();
        for (index, position) in seam_world.into_iter().enumerate() {
            if index > 0 {
                assert!(!stabilizer.discontinuity(position));
            }
            output.extend(stabilizer.push(
                Point {
                    position,
                    pressure: 0.4 + index as f32 * 0.2,
                    tilt: None,

                    time_ms: 0,
                },
                0.05,
            ));
        }
        output.extend(stabilizer.finish());
        assert!(output
            .iter()
            .all(|point| ((point.position - sphere.center).length() - sphere.radius).abs() < 1e-4));
        assert!(output.last().unwrap().position.distance(seam_world[1]) < 1e-5);

        stabilizer.start(Surface::Sphere(sphere));
        let north = sphere.point(0.0, PI * 0.5);
        for longitude in [0.0, 1.0, 2.0, 3.0] {
            let near_pole = sphere.point(longitude, PI * 0.5 - 0.002);
            assert!(!stabilizer.discontinuity(near_pole));
            stabilizer.push(
                Point {
                    position: near_pole,
                    pressure: 0.7,
                    tilt: None,

                    time_ms: 0,
                },
                0.05,
            );
        }
        let tail = stabilizer.finish();
        assert!(tail.iter().all(|point| {
            ((point.position - sphere.center).length() - sphere.radius).abs() < 1e-4
        }));
        assert!(north.is_finite());
    }

    #[test]
    fn cone_stabilization_crosses_seam_and_never_leaves_open_side() {
        let cone = ConeSurface {
            center: Vec3::new(-0.7, 0.5, 1.2),
            rotation: Quat::from_rotation_z(0.3) * Quat::from_rotation_x(-0.2),
            radius: 1.1,
            height: 2.4,
        };
        let mut stabilizer = SurfaceStabilizer::default();
        stabilizer.start(Surface::Cone(cone));
        let points = [
            cone.point(PI - 0.1, 0.4),
            cone.point(-PI + 0.1, 0.45),
            cone.point(-PI + 0.2, 0.5),
        ];
        let mut output = Vec::new();
        for (index, position) in points.into_iter().enumerate() {
            if index > 0 {
                assert!(!stabilizer.discontinuity(position));
            }
            output.extend(stabilizer.push(
                Point {
                    position,
                    pressure: 0.3 + index as f32 * 0.2,
                    tilt: None,

                    time_ms: 0,
                },
                0.04,
            ));
        }
        output.extend(stabilizer.finish());
        assert!(output.iter().all(|point| {
            let local = cone.rotation.inverse() * (point.position - cone.center);
            let expected_radius = cone.radius * (1.0 - local.y / cone.height);
            (local.x.hypot(local.z) - expected_radius).abs() < 1e-4
                && (0.0..=cone.height).contains(&local.y)
        }));
        assert!(output.last().unwrap().position.distance(points[2]) < 1e-5);
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

                time_ms: 0,
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

                time_ms: 0,
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
