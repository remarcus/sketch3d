use crate::document::Stroke;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    Front,
    Side,
    Ground,
}
impl Plane {
    pub fn label(self) -> &'static str {
        match self {
            Self::Front => "Front / XY",
            Self::Side => "Side / YZ",
            Self::Ground => "Ground / XZ",
        }
    }
    pub fn basis(self) -> (Vec3, Vec3, Vec3) {
        match self {
            Self::Front => (Vec3::X, Vec3::Y, Vec3::Z),
            Self::Side => (-Vec3::Z, Vec3::Y, Vec3::X),
            Self::Ground => (Vec3::X, -Vec3::Z, Vec3::Y),
        }
    }
}
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target: Vec3,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.35,
            pitch: 0.22,
            distance: 9.0,
            target: Vec3::ZERO,
        }
    }
}
impl Camera {
    pub fn eye(&self) -> Vec3 {
        self.target
            + Vec3::new(
                self.yaw.sin() * self.pitch.cos(),
                self.pitch.sin(),
                self.yaw.cos() * self.pitch.cos(),
            ) * self.distance
    }
    pub fn matrix(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(45f32.to_radians(), aspect.max(0.01), 0.01, 1000.0)
            * Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let inv = self.matrix(aspect).inverse();
        let near = inv.project_point3(ndc.extend(0.0));
        let far = inv.project_point3(ndc.extend(1.0));
        (near, (far - near).normalize())
    }
    pub fn pan(&mut self, delta: Vec2, height: f32) {
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        self.target += (-right * delta.x + up * delta.y)
            * (2.0 * self.distance * 22.5f32.to_radians().tan() / height.max(1.0));
    }
    pub fn align(&mut self, plane: Plane) {
        match plane {
            Plane::Front => {
                self.yaw = 0.0;
                self.pitch = 0.0
            }
            Plane::Side => {
                self.yaw = std::f32::consts::FRAC_PI_2;
                self.pitch = 0.0
            }
            Plane::Ground => {
                self.yaw = 0.0;
                self.pitch = 1.569
            }
        }
    }
}
pub fn intersect_plane(origin: Vec3, direction: Vec3, plane: Plane, offset: f32) -> Option<Vec3> {
    let (_, _, normal) = plane.basis();
    let denom = direction.dot(normal);
    if denom.abs() < 0.025 {
        return None;
    }
    let t = (offset - origin.dot(normal)) / denom;
    if !(0.0..1000.0).contains(&t) {
        return None;
    }
    Some(origin + direction * t)
}
pub fn tube(stroke: &Stroke) -> Vec<Vertex> {
    let mut out = Vec::new();
    if stroke.points.is_empty() {
        return out;
    }
    let mut rings = Vec::with_capacity(stroke.points.len());
    let mut previous_axis: Option<Vec3> = None;
    for (i, p) in stroke.points.iter().enumerate() {
        let prev = stroke.points[i.saturating_sub(1)].position;
        let next = stroke.points[(i + 1).min(stroke.points.len() - 1)].position;
        let tangent = (next - prev).try_normalize().unwrap_or(Vec3::Y);
        let helper = if tangent.dot(Vec3::Y).abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        // Transport the last ring axis along the curve instead of switching bases.
        let a = previous_axis
            .and_then(|axis| (axis - tangent * axis.dot(tangent)).try_normalize())
            .unwrap_or_else(|| tangent.cross(helper).normalize());
        previous_axis = Some(a);
        let b = tangent.cross(a).normalize();
        let radius = stroke.radius * (0.12 + 0.88 * p.pressure);
        let ring: Vec<Vertex> = (0..8)
            .map(|j| {
                let angle = j as f32 * std::f32::consts::TAU / 8.0;
                let normal = a * angle.cos() + b * angle.sin();
                Vertex {
                    position: (p.position + normal * radius).to_array(),
                    normal: normal.to_array(),
                    color: stroke.color,
                }
            })
            .collect();
        rings.push(ring);
    }
    if rings.len() == 1 {
        let p = stroke.points[0];
        let r = stroke.radius * (0.12 + 0.88 * p.pressure);
        let axes = [Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z];
        for up in [Vec3::Y, -Vec3::Y] {
            for i in 0..4 {
                for normal in [up, axes[i], axes[(i + 1) % 4]] {
                    out.push(Vertex {
                        position: (p.position + normal * r).to_array(),
                        normal: normal.to_array(),
                        color: stroke.color,
                    });
                }
            }
        }
        return out;
    }
    for pair in rings.windows(2) {
        for j in 0..8 {
            let k = (j + 1) % 8;
            out.extend_from_slice(&[
                pair[0][j], pair[1][j], pair[1][k], pair[0][j], pair[1][k], pair[0][k],
            ]);
        }
    }
    for (index, ring) in [(0, &rings[0]), (rings.len() - 1, rings.last().unwrap())] {
        let center = Vertex {
            position: stroke.points[index].position.to_array(),
            normal: [0.0, 0.0, 1.0],
            color: stroke.color,
        };
        for j in 0..8 {
            out.extend_from_slice(&[center, ring[j], ring[(j + 1) % 8]]);
        }
    }
    out
}
pub fn grid(plane: Plane, offset: f32) -> Vec<Vertex> {
    let (u, v, n) = plane.basis();
    let mut vertices = Vec::new();
    for i in -12..=12 {
        for swap in [false, true] {
            let (a, b) = if swap { (v, u) } else { (u, v) };
            let color = if i == 0 {
                [0.22, 0.44, 0.46]
            } else {
                [0.09, 0.145, 0.19]
            };
            for sign in [-1.0, 1.0] {
                vertices.push(Vertex {
                    position: (n * offset + a * i as f32 * 0.5 + b * 6.0 * sign).to_array(),
                    normal: [0.0; 3],
                    color,
                });
            }
        }
    }
    vertices
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Point;
    #[test]
    fn rays_and_planes() {
        let mut c = Camera::default();
        c.align(Plane::Front);
        let (o, d) = c.ray(Vec2::ZERO, 1.5);
        assert!(intersect_plane(o, d, Plane::Front, 0.0).unwrap().length() < 0.001);
        assert!(intersect_plane(Vec3::Z, Vec3::X, Plane::Front, 0.0).is_none());
        assert!(intersect_plane(Vec3::Z, Vec3::Z, Plane::Front, 0.0).is_none());
    }
    #[test]
    fn pressure_geometry() {
        let make = |pressure| Stroke {
            id: 1,
            color: [0.5; 3],
            radius: 0.1,
            points: vec![
                Point {
                    position: Vec3::ZERO,
                    pressure,
                },
                Point {
                    position: Vec3::Y,
                    pressure,
                },
            ],
        };
        let low = tube(&make(0.0));
        let high = tube(&make(1.0));
        assert_eq!(low.len(), high.len());
        assert!(Vec3::from(high[0].position).length() > Vec3::from(low[0].position).length() * 5.0);
        assert!(high.iter().all(|v| Vec3::from(v.position).is_finite()));
    }
}

#[cfg(test)]
mod curve_regression {
    use super::*;
    use crate::document::Point;
    #[test]
    fn curve_frames_do_not_flip() {
        let points: Vec<Point> = (0..160)
            .map(|i| {
                let t = i as f32 / 159.0 * std::f32::consts::TAU;
                Point {
                    position: Vec3::new(t.cos(), t.sin(), 0.0),
                    pressure: 1.0,
                }
            })
            .collect();
        let stroke = Stroke {
            id: 1,
            color: [0.5; 3],
            radius: 0.03,
            points,
        };
        let mesh = tube(&stroke);
        for segment in mesh[..159 * 48].as_chunks::<48>().0 {
            let a = Vec3::from(segment[0].normal);
            let b = Vec3::from(segment[1].normal);
            assert!(
                a.dot(b) > 0.95,
                "Tube orientation flipped between adjacent rings"
            );
        }
    }
}
