use crate::document::{BrushKind, Stroke};
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
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
}
/// A work plane is interface state; existing stroke positions stay in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawingPlane {
    pub anchor: Vec3,
    pub rotation_degrees: Vec3,
    pub depth: f32,
}
impl Default for DrawingPlane {
    fn default() -> Self {
        Self::from_preset(Plane::Front)
    }
}
impl DrawingPlane {
    pub fn from_preset(plane: Plane) -> Self {
        let rotation_degrees = match plane {
            Plane::Front => Vec3::ZERO,
            Plane::Side => Vec3::new(0.0, 90.0, 0.0),
            Plane::Ground => Vec3::new(-90.0, 0.0, 0.0),
        };
        Self {
            anchor: Vec3::ZERO,
            rotation_degrees,
            depth: 0.0,
        }
    }
    pub fn rotation(self) -> Quat {
        let r = self.rotation_degrees * std::f32::consts::PI / 180.0;
        Quat::from_euler(glam::EulerRot::ZYX, r.z, r.y, r.x)
    }
    pub fn basis(self) -> (Vec3, Vec3, Vec3) {
        let q = self.rotation();
        (q * Vec3::X, q * Vec3::Y, q * Vec3::Z)
    }
    pub fn origin(self) -> Vec3 {
        self.anchor + self.basis().2 * self.depth
    }
    pub fn preset(self) -> Option<Plane> {
        [Plane::Front, Plane::Side, Plane::Ground]
            .into_iter()
            .find(|p| self.rotation().dot(Self::from_preset(*p).rotation()).abs() > 1.0 - 0.0000001)
    }
    pub fn label(self) -> &'static str {
        self.preset().map(|p| p.label()).unwrap_or("Custom plane")
    }
    pub fn from_view(camera: &Camera) -> Self {
        let (u, v, n) = camera.basis();
        let q = Quat::from_mat3(&Mat3::from_cols(u, v, n));
        let (z, y, x) = q.to_euler(glam::EulerRot::ZYX);
        Self {
            anchor: camera.target,
            rotation_degrees: Vec3::new(x, y, z) * 180.0 / std::f32::consts::PI,
            depth: 0.0,
        }
    }
}
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub distance: f32,
    pub target: Vec3,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.35,
            pitch: 0.22,
            roll: 0.0,
            distance: 9.0,
            target: Vec3::ZERO,
        }
    }
}
impl Camera {
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let n = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        );
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = n.cross(right);
        (
            right * self.roll.cos() + up * self.roll.sin(),
            -right * self.roll.sin() + up * self.roll.cos(),
            n,
        )
    }
    pub fn align_guide(&mut self, guide: DrawingPlane) {
        let (_, up, normal) = guide.basis();
        self.yaw = normal.x.atan2(normal.z);
        self.pitch = normal.y.clamp(-1.0, 1.0).asin();
        self.roll = 0.0;
        let (right, base_up, _) = self.basis();
        self.roll = (-up.dot(right)).atan2(up.dot(base_up));
        self.target = guide.origin();
    }
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
            * Mat4::look_at_rh(self.eye(), self.target, self.basis().1)
    }
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let inv = self.matrix(aspect).inverse();
        let near = inv.project_point3(ndc.extend(0.0));
        let far = inv.project_point3(ndc.extend(1.0));
        (near, (far - near).normalize())
    }
    pub fn pan(&mut self, delta: Vec2, height: f32) {
        let (right, up, _) = self.basis();
        self.target += (-right * delta.x + up * delta.y)
            * (2.0 * self.distance * 22.5f32.to_radians().tan() / height.max(1.0));
    }
    pub fn align(&mut self, plane: Plane) {
        self.roll = 0.0;
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
pub fn intersect_plane(origin: Vec3, direction: Vec3, plane: DrawingPlane) -> Option<Vec3> {
    let (_, _, normal) = plane.basis();
    let denom = direction.dot(normal);
    if denom.abs() < 0.025 {
        return None;
    }
    let t = (plane.origin() - origin).dot(normal) / denom;
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
/// Conservative brush radius, shared by selection and geometry bounds.
pub fn stroke_extent(stroke: &Stroke) -> f32 {
    let dot = !stroke.points.is_empty()
        && stroke
            .points
            .windows(2)
            .all(|pair| pair[0].position.distance_squared(pair[1].position) <= 1e-12);
    match stroke.brush.kind {
        BrushKind::Chisel => {
            stroke.radius * (if dot { 2.0 } else { 1.0 } + stroke.brush.thickness.powi(2)).sqrt()
        }
        BrushKind::Ribbon if dot => stroke.radius * std::f32::consts::SQRT_2,
        BrushKind::Round | BrushKind::Ribbon => stroke.radius,
    }
}

/// All ink is built in document space. Camera changes cannot rotate a brush mark.
pub fn stroke_mesh(stroke: &Stroke) -> Vec<Vertex> {
    if stroke.brush.kind != BrushKind::Round {
        return flat_brush(stroke);
    }
    // Retain the original round brush's positions and pressure profile. Give caps
    // their true axial normals and fix the legacy inward side winding here.
    let mut mesh = tube(stroke);
    if stroke.points.len() > 1 {
        let sides = (stroke.points.len() - 1) * 48;
        let first = (stroke.points[1].position - stroke.points[0].position)
            .try_normalize()
            .unwrap_or(Vec3::Y);
        let end = stroke.points.len() - 1;
        let last = (stroke.points[end].position - stroke.points[end - 1].position)
            .try_normalize()
            .unwrap_or(Vec3::Y);
        for (i, vertex) in mesh[sides..].iter_mut().enumerate() {
            vertex.normal = if i < 24 { -first } else { last }.to_array();
        }
    }
    for triangle in mesh.as_chunks_mut::<3>().0 {
        let a = Vec3::from(triangle[0].position);
        let b = Vec3::from(triangle[1].position);
        let c = Vec3::from(triangle[2].position);
        let normal: Vec3 = triangle.iter().map(|v| Vec3::from(v.normal)).sum();
        if (b - a).cross(c - a).dot(normal) < 0.0 {
            triangle.swap(1, 2);
        }
    }
    mesh
}

#[derive(Clone, Copy)]
struct BrushFrame {
    center: Vec3,
    width: Vec3,
    normal: Vec3,
    tangent: Vec3,
    radius: f32,
}

fn perpendicular(normal: Vec3) -> Vec3 {
    let helper = if normal.x.abs() < 0.8 {
        Vec3::X
    } else {
        Vec3::Y
    };
    (helper - normal * helper.dot(normal)).normalize()
}

fn frame_direction(vector: Vec3) -> Option<Vec3> {
    // Round-off after a rigid transform must not turn an almost-zero
    // projection into a new, arbitrary frame direction.
    (vector.length_squared() > 1e-8).then(|| vector.normalize())
}

fn brush_frames(stroke: &Stroke) -> Vec<BrushFrame> {
    let mut points: Vec<crate::document::Point> = Vec::with_capacity(stroke.points.len());
    for point in &stroke.points {
        if let Some(last) = points.last_mut() {
            if last.position.distance_squared(point.position) <= 1e-12 {
                // Repeated pressure samples do not create zero-length mesh strips.
                *last = *point;
                continue;
            }
        }
        points.push(*point);
    }
    let reference = stroke.brush.normal.try_normalize().unwrap_or(Vec3::Z);
    let mut frames: Vec<BrushFrame> = Vec::with_capacity(points.len());
    for (i, point) in points.iter().enumerate() {
        let before = points[i.saturating_sub(1)].position;
        let after = points[(i + 1).min(points.len() - 1)].position;
        let tangent = (after - before)
            .try_normalize()
            .or_else(|| (point.position - before).try_normalize())
            .or_else(|| (after - point.position).try_normalize())
            .unwrap_or(stroke.brush.tangent);
        let width = if let Some(previous) = frames.last() {
            // Projection preserves the width sign even at an exact reversal.
            frame_direction(previous.width - tangent * previous.width.dot(tangent)).unwrap_or_else(
                || {
                    let candidate = previous.normal.cross(tangent).normalize_or_zero();
                    if candidate.length_squared() > 0.5 {
                        if candidate.dot(previous.width) < 0.0 {
                            -candidate
                        } else {
                            candidate
                        }
                    } else {
                        perpendicular(tangent)
                    }
                },
            )
        } else {
            let normal = frame_direction(reference - tangent * reference.dot(tangent))
                .unwrap_or_else(|| {
                    frame_direction(
                        stroke.brush.tangent - tangent * stroke.brush.tangent.dot(tangent),
                    )
                    .unwrap_or_else(|| perpendicular(tangent))
                });
            let width = normal.cross(tangent).normalize();
            Quat::from_axis_angle(tangent, stroke.brush.angle_degrees.to_radians()) * width
        };
        frames.push(BrushFrame {
            center: point.position,
            width,
            normal: tangent.cross(width).normalize(),
            tangent,
            radius: stroke.radius * (0.12 + 0.88 * point.pressure),
        });
    }
    frames
}

fn flat_triangle(out: &mut Vec<Vertex>, mut positions: [Vec3; 3], outward: Vec3, color: [f32; 3]) {
    let cross = (positions[1] - positions[0]).cross(positions[2] - positions[0]);
    let Some(mut normal) = cross.try_normalize() else {
        return;
    };
    if normal.dot(outward) < 0.0 {
        positions.swap(1, 2);
        normal = -normal;
    }
    out.extend(positions.map(|position| Vertex {
        position: position.to_array(),
        normal: normal.to_array(),
        color,
    }));
}

fn flat_quad(out: &mut Vec<Vertex>, positions: [Vec3; 4], outward: Vec3, color: [f32; 3]) {
    flat_triangle(
        out,
        [positions[0], positions[1], positions[2]],
        outward,
        color,
    );
    flat_triangle(
        out,
        [positions[0], positions[2], positions[3]],
        outward,
        color,
    );
}

fn flat_brush(stroke: &Stroke) -> Vec<Vertex> {
    let frames = brush_frames(stroke);
    let mut out = Vec::new();
    let Some(first) = frames.first() else {
        return out;
    };
    if frames.len() == 1 {
        // A tap has a finite footprint in the guide plane, even for a thin ribbon.
        let f = first;
        let a = f.width * f.radius;
        let b = f.tangent * f.radius;
        let h = if stroke.brush.kind == BrushKind::Chisel {
            f.normal * f.radius * stroke.brush.thickness
        } else {
            Vec3::ZERO
        };
        let corners = [
            f.center + a + b,
            f.center - a + b,
            f.center - a - b,
            f.center + a - b,
        ];
        flat_quad(&mut out, corners.map(|p| p + h), f.normal, stroke.color);
        if stroke.brush.kind == BrushKind::Chisel {
            flat_quad(&mut out, corners.map(|p| p - h), -f.normal, stroke.color);
            for j in 0..4 {
                let k = (j + 1) % 4;
                let outward = (corners[j] + corners[k]) * 0.5 - f.center;
                flat_quad(
                    &mut out,
                    [
                        corners[j] + h,
                        corners[k] + h,
                        corners[k] - h,
                        corners[j] - h,
                    ],
                    outward,
                    stroke.color,
                );
            }
        }
        return out;
    }
    if stroke.brush.kind == BrushKind::Ribbon {
        for pair in frames.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            flat_quad(
                &mut out,
                [
                    a.center - a.width * a.radius,
                    a.center + a.width * a.radius,
                    b.center + b.width * b.radius,
                    b.center - b.width * b.radius,
                ],
                a.normal + b.normal,
                stroke.color,
            );
        }
    } else {
        let ring = |f: BrushFrame| {
            let a = f.width * f.radius;
            let b = f.normal * f.radius * stroke.brush.thickness;
            [
                f.center + a + b,
                f.center - a + b,
                f.center - a - b,
                f.center + a - b,
            ]
        };
        for pair in frames.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            let [ar, br] = [ring(a), ring(b)];
            for j in 0..4 {
                let k = (j + 1) % 4;
                let outward = (ar[j] + ar[k] - a.center * 2.0) + (br[j] + br[k] - b.center * 2.0);
                flat_quad(
                    &mut out,
                    [ar[j], br[j], br[k], ar[k]],
                    outward,
                    stroke.color,
                );
            }
        }
        flat_quad(&mut out, ring(*first), -first.tangent, stroke.color);
        let last = frames.last().unwrap();
        flat_quad(&mut out, ring(*last), last.tangent, stroke.color);
    }
    out
}

pub fn grid(plane: DrawingPlane) -> Vec<Vertex> {
    let (u, v, _) = plane.basis();
    let origin = plane.origin();
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
                    position: (origin + a * i as f32 * 0.5 + b * 6.0 * sign).to_array(),
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
        assert!(
            intersect_plane(o, d, DrawingPlane::default())
                .unwrap()
                .length()
                < 0.001
        );
        assert!(intersect_plane(Vec3::Z, Vec3::X, DrawingPlane::default()).is_none());
        assert!(intersect_plane(Vec3::Z, Vec3::Z, DrawingPlane::default()).is_none());
    }
    #[test]
    fn pressure_geometry() {
        let make = |pressure| Stroke {
            id: 1,
            color: [0.5; 3],
            radius: 0.1,
            brush: Default::default(),
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
            brush: Default::default(),
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

#[cfg(test)]
mod work_plane_tests {
    use super::*;
    fn custom() -> DrawingPlane {
        DrawingPlane {
            anchor: Vec3::new(1.2, -0.4, 2.0),
            rotation_degrees: Vec3::new(31.0, -42.0, 23.0),
            depth: 0.7,
        }
    }
    #[test]
    fn rotated_translated_plane_intersections_and_grid_agree() {
        let plane = custom();
        let (u, v, n) = plane.basis();
        let expected = plane.origin() + u * 1.3 - v * 0.8;
        let hit = intersect_plane(expected + n * 4.0, -n, plane).unwrap();
        assert!(hit.distance(expected) < 0.00001);
        assert!(intersect_plane(expected + n, u, plane).is_none());
        assert!(intersect_plane(expected + n, n, plane).is_none());
        for vertex in grid(plane) {
            assert!((Vec3::from(vertex.position) - plane.origin()).dot(n).abs() < 0.00001);
        }
    }
    #[test]
    fn align_view_preserves_plane_axes_including_vertical_and_roll() {
        for plane in [
            custom(),
            DrawingPlane::from_preset(Plane::Front),
            DrawingPlane::from_preset(Plane::Side),
            DrawingPlane::from_preset(Plane::Ground),
        ] {
            let mut camera = Camera::default();
            camera.align_guide(plane);
            let (u, v, n) = plane.basis();
            let (right, up, normal) = camera.basis();
            assert!(right.dot(u) > 0.9999);
            assert!(up.dot(v) > 0.9999);
            assert!(normal.dot(n) > 0.9999);
            assert_eq!(camera.target, plane.origin());
            let matrix = camera.matrix(1.4);
            assert!(matrix.is_finite());
            assert!(matrix.project_point3(plane.origin()).truncate().length() < 0.0001);
        }
    }
    #[test]
    fn plane_from_view_matches_rolled_camera_and_focus_point() {
        let camera = Camera {
            yaw: -0.8,
            pitch: 0.6,
            roll: 0.45,
            target: Vec3::new(2.0, 1.0, -3.0),
            ..Default::default()
        };
        let plane = DrawingPlane::from_view(&camera);
        assert_eq!(plane.origin(), camera.target);
        let (u, v, n) = plane.basis();
        let (right, up, normal) = camera.basis();
        assert!(u.dot(right) > 0.9999 && v.dot(up) > 0.9999 && n.dot(normal) > 0.9999);
    }
}

#[cfg(test)]
mod brush_tests {
    use super::*;
    use crate::document::{Brush, Point};

    fn mark(kind: BrushKind, positions: &[Vec3]) -> Stroke {
        Stroke {
            id: 7,
            color: [0.3, 0.6, 0.9],
            radius: 0.1,
            brush: Brush {
                kind,
                ..Default::default()
            },
            points: positions
                .iter()
                .map(|&position| Point {
                    position,
                    pressure: 1.0,
                })
                .collect(),
        }
    }

    fn valid(mesh: &[Vertex]) {
        assert!(!mesh.is_empty());
        for triangle in mesh.as_chunks::<3>().0 {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|v| Vec3::from(v.position));
            let face = (b - a).cross(c - a);
            for v in triangle {
                assert!(Vec3::from(v.position).is_finite());
                let n = Vec3::from(v.normal);
                assert!(n.is_finite() && (n.length() - 1.0).abs() < 1e-5);
            }
            let mean_normal: Vec3 = triangle.iter().map(|v| Vec3::from(v.normal)).sum();
            assert!(
                face.dot(mean_normal) > -1e-7,
                "triangle winding must face along its mean shading normal"
            );
        }
    }

    #[test]
    fn all_brushes_have_finite_dots_duplicates_and_reversal_meshes() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            for positions in [
                vec![Vec3::ZERO],
                vec![Vec3::ZERO; 3],
                vec![Vec3::ZERO, Vec3::X, Vec3::ZERO, -Vec3::X],
            ] {
                valid(&stroke_mesh(&mark(kind, &positions)));
            }
        }
    }

    #[test]
    fn flat_cross_sections_keep_pressure_thickness_and_manual_angle() {
        let mut stroke = mark(BrushKind::Chisel, &[Vec3::ZERO, Vec3::X]);
        stroke.brush.thickness = 0.25;
        let mesh = stroke_mesh(&stroke);
        valid(&mesh);
        let max_y = mesh.iter().map(|v| v.position[1].abs()).fold(0.0, f32::max);
        let max_z = mesh.iter().map(|v| v.position[2].abs()).fold(0.0, f32::max);
        assert!((max_y - 0.1).abs() < 1e-6 && (max_z - 0.025).abs() < 1e-6);
        stroke.brush.angle_degrees = 90.0;
        let mesh = stroke_mesh(&stroke);
        let rotated_y = mesh.iter().map(|v| v.position[1].abs()).fold(0.0, f32::max);
        let rotated_z = mesh.iter().map(|v| v.position[2].abs()).fold(0.0, f32::max);
        assert!((rotated_y - max_z).abs() < 1e-6 && (rotated_z - max_y).abs() < 1e-6);
        for p in &mut stroke.points {
            p.pressure = 0.0;
        }
        let frames = brush_frames(&stroke);
        assert!((frames[0].radius - 0.012).abs() < 1e-6);
        assert!(stroke_extent(&stroke) > stroke.radius);
    }

    #[test]
    fn transported_flat_widths_stay_continuous_on_tight_3d_curves() {
        let positions: Vec<Vec3> = (0..200)
            .map(|i| {
                let t = i as f32 * 0.04;
                Vec3::new(t.cos(), t.sin(), t * 0.1)
            })
            .collect();
        let stroke = mark(BrushKind::Ribbon, &positions);
        let frames = brush_frames(&stroke);
        for pair in frames.windows(2) {
            assert!(pair[0].width.dot(pair[1].width) > 0.99);
            assert!(pair[0].normal.dot(pair[1].normal) > 0.99);
        }
        valid(&stroke_mesh(&stroke));
    }

    #[test]
    fn frozen_frames_transform_with_both_dots_and_paths() {
        let rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.6, -0.8, 1.2);
        let translation = Vec3::new(2.0, -0.5, 1.0);
        for kind in [BrushKind::Ribbon, BrushKind::Chisel] {
            for positions in [
                vec![Vec3::ZERO],
                vec![Vec3::ZERO, Vec3::X, Vec3::new(1.2, 0.8, 0.3)],
                vec![Vec3::ZERO, Vec3::Z],
            ] {
                let mut stroke = mark(kind, &positions);
                stroke.brush.angle_degrees = 32.0;
                let before = stroke_mesh(&stroke);
                for p in &mut stroke.points {
                    p.position = rotation * p.position + translation;
                }
                stroke.brush.normal = rotation * stroke.brush.normal;
                stroke.brush.tangent = rotation * stroke.brush.tangent;
                let after = stroke_mesh(&stroke);
                assert_eq!(before.len(), after.len());
                for (a, b) in before.iter().zip(&after) {
                    assert!(
                        (rotation * Vec3::from(a.position) + translation)
                            .distance(Vec3::from(b.position))
                            < 1e-5
                    );
                    assert!(
                        (rotation * Vec3::from(a.normal)).distance(Vec3::from(b.normal)) < 1e-5
                    );
                }
            }
        }
    }
    #[test]
    fn round_dispatch_preserves_legacy_positions_and_repairs_caps() {
        let stroke = mark(BrushKind::Round, &[Vec3::ZERO, Vec3::X]);
        let old = tube(&stroke);
        let new = stroke_mesh(&stroke);
        for (a, b) in old.as_chunks::<3>().0.iter().zip(new.as_chunks::<3>().0) {
            for vertex in a {
                assert!(b.iter().any(|v| v.position == vertex.position));
            }
        }
        for v in &new[48..72] {
            assert_eq!(v.normal, (-Vec3::X).to_array());
        }
        for v in &new[72..] {
            assert_eq!(v.normal, Vec3::X.to_array());
        }
        valid(&new);
    }
}
