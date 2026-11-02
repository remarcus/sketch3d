use crate::document::{BrushKind, Point, Stroke, StrokeMaterial};
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub uv: [f32; 2],
    /// Final per-vertex alpha, including base opacity, pressure, and soft coverage.
    pub opacity: f32,
    /// Encoded material mode: shaded=0, unlit=1, emissive=2.
    pub material: f32,
    /// Normalized centerline arc-length parameter used to resolve stroke gradients.
    pub gradient_t: f32,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub projection: Projection,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub distance: f32,
    pub target: Vec3,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: Projection::Perspective,
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
        let h = self.distance * 22.5f32.to_radians().tan();
        let aspect = aspect.max(0.01);
        let projection = match self.projection {
            Projection::Perspective => {
                Mat4::perspective_rh(45f32.to_radians(), aspect, 0.01, 1000.0)
            }
            Projection::Orthographic => Mat4::orthographic_rh(
                -h * aspect,
                h * aspect,
                -h,
                h,
                self.distance - 500.0,
                self.distance + 500.0,
            ),
        };
        projection * Mat4::look_at_rh(self.eye(), self.target, self.basis().1)
    }
    pub fn world_per_pixel(&self, point: Vec3, height: f32) -> f32 {
        let distance = match self.projection {
            Projection::Perspective => (self.eye() - point).dot(self.basis().2).max(0.01),
            Projection::Orthographic => self.distance,
        };
        2.0 * distance * 22.5f32.to_radians().tan() / height.max(1.0)
    }
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> (Vec3, Vec3) {
        if self.projection == Projection::Orthographic {
            let (u, v, n) = self.basis();
            let h = self.distance * 22.5f32.to_radians().tan();
            return (
                self.target + u * ndc.x * h * aspect.max(0.01) + v * ndc.y * h + n * 500.0,
                -n,
            );
        }
        let inv = self.matrix(aspect).inverse();
        let near = inv.project_point3(ndc.extend(0.0));
        let far = inv.project_point3(ndc.extend(1.0));
        (near, (far - near).normalize())
    }
    pub fn pan(&mut self, delta: Vec2, height: f32) {
        let (right, up, _) = self.basis();
        self.target +=
            (-right * delta.x + up * delta.y) * self.world_per_pixel(self.target, height);
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
    let point_count = stroke.points.len();
    let closed = is_closed_stroke(stroke);
    let mut out = Vec::with_capacity(if point_count <= 1 {
        36
    } else {
        (point_count - 1) * 48 + 48
    });
    if stroke.points.is_empty() {
        return out;
    }
    // A round brush ring always has eight vertices. Fixed-size arrays avoid
    // one heap allocation per sample on long strokes.
    let mut rings: Vec<[Vertex; 8]> = Vec::with_capacity(point_count);
    let mut previous_axis: Option<Vec3> = None;
    let distances = crate::dynamics::arc_lengths(&stroke.points);
    let total = distances.last().copied().unwrap_or(0.0);
    for (i, p) in stroke.points.iter().enumerate() {
        let prev = if closed && i == 0 {
            stroke.points[point_count - 2].position
        } else {
            stroke.points[i.saturating_sub(1)].position
        };
        let next = stroke.points[(i + 1).min(point_count - 1)].position;
        let tangent = (next - prev).try_normalize().unwrap_or(Vec3::Y);
        let helper = if tangent.dot(Vec3::Y).abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        // Transport the last ring axis along the curve instead of switching bases.
        let a = previous_axis
            .and_then(|axis| (axis - tangent * axis.dot(tangent)).try_normalize())
            .unwrap_or_else(|| {
                if stroke.brush.texture.kind == crate::texture::TextureKind::Solid {
                    tangent.cross(helper).normalize()
                } else {
                    frame_direction(
                        stroke.brush.normal - tangent * stroke.brush.normal.dot(tangent),
                    )
                    .or_else(|| {
                        frame_direction(
                            stroke.brush.tangent - tangent * stroke.brush.tangent.dot(tangent),
                        )
                    })
                    .unwrap_or_else(|| tangent.cross(helper).normalize())
                }
            });
        previous_axis = Some(a);
        let b = tangent.cross(a).normalize();
        let radius = stroke.radius
            * stroke.point_thickness_scale(i)
            * crate::dynamics::pressure_scale(stroke.brush.dynamics.width, p.pressure)
            * crate::dynamics::shading_width_scale(
                stroke.brush.dynamics,
                stroke.brush.texture.kind,
                p.tilt,
            )
            * crate::dynamics::taper_factor(
                distances[i],
                total,
                stroke.brush.dynamics.taper_start,
                stroke.brush.dynamics.taper_end,
            );
        let mut ring = std::array::from_fn(|j| {
            let angle = j as f32 * std::f32::consts::TAU / 8.0;
            let normal = a * angle.cos() + b * angle.sin();
            Vertex {
                material: 0.0,
                gradient_t: if total > 1.0e-8 {
                    (distances[i] / total).clamp(0.0, 1.0)
                } else {
                    0.0
                },
                position: (p.position + normal * radius).to_array(),
                normal: normal.to_array(),
                color: stroke.color,
                uv: [distances[i] / stroke.brush.texture.scale, j as f32 / 8.0],
                opacity: vertex_opacity(stroke, p.pressure, normal, stroke.brush.normal),
            }
        });
        if closed && i + 1 == point_count {
            ring = rings[0];
            for vertex in &mut ring {
                vertex.uv[0] = distances[i] / stroke.brush.texture.scale;
                vertex.gradient_t = if total > 1.0e-8 {
                    (distances[i] / total).clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        rings.push(ring);
    }
    if rings.len() == 1 {
        let p = stroke.points[0];
        let r = stroke.radius
            * stroke.point_thickness_scale(0)
            * crate::dynamics::pressure_scale(stroke.brush.dynamics.width, p.pressure)
            * crate::dynamics::shading_width_scale(
                stroke.brush.dynamics,
                stroke.brush.texture.kind,
                p.tilt,
            );
        let textured = stroke.brush.texture.kind != crate::texture::TextureKind::Solid;
        let frame = if textured {
            Mat3::from_cols(
                stroke.brush.tangent,
                stroke.brush.normal.cross(stroke.brush.tangent),
                stroke.brush.normal,
            )
        } else {
            Mat3::IDENTITY
        };
        let axes = [Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z];
        for up in [Vec3::Y, -Vec3::Y] {
            for i in 0..4 {
                for local in [up, axes[i], axes[(i + 1) % 4]] {
                    let normal = frame * local;
                    out.push(Vertex {
                        material: 0.0,
                        gradient_t: 0.0,
                        position: (p.position + normal * r).to_array(),
                        normal: normal.to_array(),
                        color: stroke.color,
                        uv: [
                            0.5 + local.x * r / stroke.brush.texture.scale,
                            0.5 + local.y * r / stroke.brush.texture.scale,
                        ],
                        opacity: vertex_opacity(stroke, p.pressure, normal, stroke.brush.normal),
                    });
                }
            }
        }
        return out;
    }
    for pair in rings.windows(2) {
        for j in 0..8 {
            let k = (j + 1) % 8;
            let mut next0 = pair[0][k];
            let mut next1 = pair[1][k];
            // The same spatial seam vertex needs both V=0 and V=1. Do not
            // interpolate across the complete tile on the final ring segment.
            if k == 0 {
                next0.uv[1] = 1.0;
                next1.uv[1] = 1.0;
            }
            out.extend_from_slice(&[pair[0][j], pair[1][j], next1, pair[0][j], next1, next0]);
        }
    }
    if !closed {
        for (index, ring) in [(0, &rings[0]), (rings.len() - 1, rings.last().unwrap())] {
            let center = Vertex {
                material: 0.0,
                gradient_t: if total > 1.0e-8 {
                    (distances[index] / total).clamp(0.0, 1.0)
                } else {
                    0.0
                },
                position: stroke.points[index].position.to_array(),
                normal: [0.0, 0.0, 1.0],
                color: stroke.color,
                uv: [0.5, 0.5],
                opacity: vertex_opacity(
                    stroke,
                    stroke.points[index].pressure,
                    Vec3::Z,
                    stroke.brush.normal,
                ),
            };
            let radius = Vec3::from(ring[0].position).distance(stroke.points[index].position);
            let cap_vertex = |j: usize| {
                let mut vertex = ring[j % 8];
                let angle = j as f32 * std::f32::consts::TAU / 8.0;
                vertex.uv = [
                    0.5 + angle.cos() * radius / stroke.brush.texture.scale,
                    0.5 + angle.sin() * radius / stroke.brush.texture.scale,
                ];
                vertex
            };
            for j in 0..8 {
                out.extend_from_slice(&[center, cap_vertex(j), cap_vertex(j + 1)]);
            }
        }
    }
    out
}

pub(crate) const MAX_PROFILE_TUBE_VERTICES: usize = 1_000_000;

fn profile_sides(profile: crate::document::TubeProfile) -> usize {
    match profile {
        crate::document::TubeProfile::Round => 8,
        crate::document::TubeProfile::Square => 4,
        crate::document::TubeProfile::Triangle => 3,
        crate::document::TubeProfile::Star => 10,
    }
}

pub(crate) fn profile_tube_vertex_estimate(stroke: &Stroke) -> usize {
    if stroke.brush.kind != BrushKind::Round
        || stroke.brush.tube_profile == crate::document::TubeProfile::Round
        || stroke.brush.flow.enabled
        || stroke.brush.strands.enabled
        || stroke.brush.dabs.enabled
        || stroke.brush.tip_asset_id != 0
        || stroke.points.is_empty()
    {
        return 0;
    }
    let sides = profile_sides(stroke.brush.tube_profile);
    let segments = stroke.points.len().saturating_sub(1).max(1);
    let side_vertices = segments.saturating_mul(sides).saturating_mul(6);
    let cap_vertices = if is_closed_stroke(stroke) {
        0
    } else {
        sides.saturating_mul(6)
    };
    side_vertices.saturating_add(cap_vertices)
}

fn profile_tube_mesh(stroke: &Stroke) -> Vec<Vertex> {
    let capacity = profile_tube_vertex_estimate(stroke);
    if capacity == 0 || capacity > MAX_PROFILE_TUBE_VERTICES {
        return Vec::new();
    }
    let sides = profile_sides(stroke.brush.tube_profile);
    let angle_offset = match stroke.brush.tube_profile {
        crate::document::TubeProfile::Triangle => -std::f32::consts::FRAC_PI_2,
        crate::document::TubeProfile::Square => std::f32::consts::FRAC_PI_4,
        crate::document::TubeProfile::Round | crate::document::TubeProfile::Star => 0.0,
    };
    let mut frames = brush_frames(stroke);
    if frames.is_empty() {
        return Vec::new();
    }
    let closed = is_closed_stroke(stroke);
    if frames.len() == 1 {
        let frame = frames[0];
        let half = (frame.radius * stroke.brush.tube_profile_scale.z).max(0.001) * 0.65;
        let mut start = frame;
        start.center -= frame.tangent * half;
        start.u -= half / stroke.brush.texture.scale;
        let mut end = frame;
        end.center += frame.tangent * half;
        end.u += half / stroke.brush.texture.scale;
        frames = vec![start, end];
    }
    let rings: Vec<Vec<(Vec3, [f32; 2])>> = frames
        .iter()
        .map(|frame| {
            (0..sides)
                .map(|side| {
                    let angle = side as f32 * std::f32::consts::TAU / sides as f32 + angle_offset;
                    let factor = if stroke.brush.tube_profile == crate::document::TubeProfile::Star
                        && side % 2 == 1
                    {
                        0.42
                    } else {
                        1.0
                    };
                    (
                        frame.center
                            + frame.width
                                * (angle.cos()
                                    * frame.radius
                                    * factor
                                    * stroke.brush.tube_profile_scale.x)
                            + frame.normal
                                * (angle.sin()
                                    * frame.radius
                                    * factor
                                    * stroke.brush.tube_profile_scale.y),
                        [frame.u, side as f32 / sides as f32],
                    )
                })
                .collect()
        })
        .collect();
    let mut out = Vec::with_capacity(capacity);
    for (frame_pair, ring_pair) in frames.windows(2).zip(rings.windows(2)) {
        let (start, end) = (frame_pair[0], frame_pair[1]);
        for side in 0..sides {
            let next = (side + 1) % sides;
            let edge = ring_pair[0][next].0 - ring_pair[0][side].0;
            let outward = edge
                .cross(end.tangent + start.tangent)
                .try_normalize()
                .unwrap_or(start.width);
            let uv_start_next = [start.u, (side + 1) as f32 / sides as f32];
            let uv_end_next = [end.u, (side + 1) as f32 / sides as f32];
            flat_quad_gradient(
                &mut out,
                [
                    ring_pair[0][side].0,
                    ring_pair[1][side].0,
                    ring_pair[1][next].0,
                    ring_pair[0][next].0,
                ],
                [
                    ring_pair[0][side].1,
                    ring_pair[1][side].1,
                    uv_end_next,
                    uv_start_next,
                ],
                outward,
                stroke.color,
                [start.opacity, end.opacity, end.opacity, start.opacity],
                (start.softness + end.softness) * 0.5,
                [
                    start.gradient_t,
                    end.gradient_t,
                    end.gradient_t,
                    start.gradient_t,
                ],
            );
        }
    }
    if !closed {
        for (frame, ring, outward) in [
            (frames[0], &rings[0], -frames[0].tangent),
            (
                *frames.last().unwrap(),
                rings.last().unwrap(),
                frames.last().unwrap().tangent,
            ),
        ] {
            for side in 0..sides {
                let next = (side + 1) % sides;
                let radial_uv = |position: Vec3| {
                    let offset = position - frame.center;
                    [
                        0.5 + offset.dot(frame.width) / stroke.brush.texture.scale,
                        0.5 + offset.dot(frame.normal) / stroke.brush.texture.scale,
                    ]
                };
                flat_triangle_gradient(
                    &mut out,
                    [frame.center, ring[side].0, ring[next].0],
                    [[0.5, 0.5], radial_uv(ring[side].0), radial_uv(ring[next].0)],
                    outward,
                    stroke.color,
                    [frame.opacity; 3],
                    frame.softness,
                    [frame.gradient_t; 3],
                );
            }
        }
    }
    out
}
/// Conservative brush radius, shared by selection and geometry bounds.
pub fn stroke_extent(stroke: &Stroke) -> f32 {
    let evaluated = match crate::stroke_modifiers::evaluated_stroke(stroke) {
        Ok(stroke) => stroke,
        Err(_) => return 0.0,
    };
    let stroke = evaluated.as_ref();
    if stroke.fill.is_some() {
        return 0.0;
    }
    let dot = !stroke.points.is_empty()
        && stroke
            .points
            .windows(2)
            .all(|pair| pair[0].position.distance_squared(pair[1].position) <= 1e-12);
    let point_scale = stroke
        .point_thickness
        .as_ref()
        .and_then(|values| values.iter().copied().reduce(f32::max))
        .unwrap_or(1.0);
    let mut shape_extent = (if stroke.brush.flow.enabled {
        stroke.radius
            * if stroke.brush.dynamics.tilt_shading {
                stroke
                    .brush
                    .dynamics
                    .tilt_shading_scale
                    .max(stroke.brush.dynamics.manual_shading_scale)
            } else {
                1.0
            }
    } else if stroke.brush.kind == BrushKind::Round
        && stroke.brush.tube_profile != crate::document::TubeProfile::Round
        && !stroke.brush.flow.enabled
        && !stroke.brush.strands.enabled
        && !stroke.brush.dabs.enabled
        && stroke.brush.tip_asset_id == 0
    {
        let radial = stroke.radius
            * stroke
                .brush
                .tube_profile_scale
                .x
                .max(stroke.brush.tube_profile_scale.y);
        if dot {
            radial.hypot(stroke.radius * stroke.brush.tube_profile_scale.z * 0.65)
        } else {
            radial
        }
    } else if stroke.brush.strands.enabled {
        let settings = stroke.brush.strands;
        stroke.radius
            * (settings.spread * (1.0 + settings.tilt_spread) + if dot { 0.65 } else { 0.0 })
    } else if stroke.brush.tip_asset_id != 0 {
        stroke.radius * (1.0 + stroke.brush.tip_aspect.recip().powi(2)).sqrt()
    } else {
        match stroke.brush.kind {
            BrushKind::Chisel => {
                stroke.radius
                    * (if dot { 2.0 } else { 1.0 } + stroke.brush.thickness.powi(2)).sqrt()
            }
            BrushKind::Ribbon if dot => stroke.radius * std::f32::consts::SQRT_2,
            BrushKind::Round | BrushKind::Ribbon => stroke.radius,
        }
    }) * point_scale;
    if stroke.brush.dynamics.tilt_shading && stroke.brush.texture.kind.uses_continuous_alpha() {
        shape_extent *= stroke
            .brush
            .dynamics
            .tilt_shading_scale
            .max(stroke.brush.dynamics.manual_shading_scale);
    }
    if stroke.brush.dabs.enabled {
        shape_extent * (1.0 + stroke.brush.dabs.size_variation)
            + stroke.radius * point_scale * stroke.brush.dabs.scatter
    } else {
        shape_extent
    }
}

/// Insert distance landmarks so a sparse two-sample stroke still reaches its
/// full middle width. This changes only tessellation, never document samples.
fn taper_points(stroke: &Stroke) -> Vec<crate::document::Point> {
    let settings = stroke.brush.dynamics;
    let distances = crate::dynamics::arc_lengths(&stroke.points);
    let total = distances.last().copied().unwrap_or(0.0);
    if total <= 1e-8 || (settings.taper_start == 0.0 && settings.taper_end == 0.0) {
        return stroke.points.clone();
    }
    let mut landmarks = vec![settings.taper_start, total - settings.taper_end];
    if settings.taper_start > 0.0 && settings.taper_end > 0.0 {
        landmarks.push(total * settings.taper_start / (settings.taper_start + settings.taper_end));
    }
    landmarks.retain(|s| *s > 0.0 && *s < total);
    landmarks.sort_by(f32::total_cmp);
    landmarks.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    let mut out = Vec::with_capacity(stroke.points.len() + landmarks.len());
    if let Some(point) = stroke.points.first() {
        out.push(*point);
    }
    for (i, pair) in stroke.points.windows(2).enumerate() {
        let start = distances[i];
        let end = distances[i + 1];
        for &at in &landmarks {
            if at > start + 1e-7 && at < end - 1e-7 {
                let t = (at - start) / (end - start);
                out.push(crate::document::Point {
                    position: pair[0].position.lerp(pair[1].position, t),
                    pressure: pair[0].pressure + (pair[1].pressure - pair[0].pressure) * t,
                    tilt: crate::dynamics::interpolate_tilt(pair[0].tilt, pair[1].tilt, t),
                    time_ms: (pair[0].time_ms as f32
                        + (pair[1].time_ms as f32 - pair[0].time_ms as f32) * t)
                        .round() as u32,
                });
            }
        }
        out.push(pair[1]);
    }
    out
}
/// All ink is built in document space. Camera changes cannot rotate a brush mark.
pub fn stroke_mesh(stroke: &Stroke) -> Vec<Vertex> {
    let mut vertices = stroke_mesh_unmapped(stroke);
    if stroke.fill.is_none() && stroke.brush.tip_asset_id == 0 {
        for vertex in &mut vertices {
            vertex.uv = stroke.brush.texture.mapping.map(vertex.uv);
        }
    }
    vertices
}

fn stroke_mesh_unmapped(stroke: &Stroke) -> Vec<Vertex> {
    let evaluated = match crate::stroke_modifiers::evaluated_stroke(stroke) {
        Ok(stroke) => stroke,
        Err(_) => return Vec::new(),
    };
    let stroke = evaluated.as_ref();
    let material = match stroke.material {
        StrokeMaterial::Shaded => 0.0,
        StrokeMaterial::Unlit => 1.0,
        StrokeMaterial::Emissive => 2.0,
        StrokeMaterial::Cutout => 3.0,
    } + if stroke.fill.is_none() && stroke.brush.tip_asset_id != 0 {
        4.0
    } else {
        0.0
    };
    let mut vertices = if stroke.fill.is_some() {
        crate::polygon::mesh(stroke)
    } else if stroke.brush.flow.enabled {
        airbrush_mesh(stroke)
    } else if stroke.brush.tip_asset_id != 0 {
        tip_mesh(stroke)
    } else if stroke.brush.dabs.enabled {
        dab_mesh(stroke)
    } else if stroke.brush.strands.enabled {
        strand_mesh(stroke)
    } else {
        let prepared;
        let stroke =
            if stroke.brush.dynamics.taper_start > 0.0 || stroke.brush.dynamics.taper_end > 0.0 {
                prepared = Stroke {
                    curve: None,
                    points: taper_points(stroke),
                    ..stroke.clone()
                };
                &prepared
            } else {
                stroke
            };
        if stroke.brush.kind == BrushKind::Round
            && stroke.brush.tube_profile != crate::document::TubeProfile::Round
        {
            profile_tube_mesh(stroke)
        } else if stroke.brush.kind != BrushKind::Round {
            flat_brush(stroke)
        } else {
            // Retain the original round brush's positions and pressure profile. Give caps
            // their true axial normals and fix the legacy inward side winding here.
            let mut mesh = tube(stroke);
            if stroke.points.len() > 1 && !is_closed_stroke(stroke) {
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
    };
    for vertex in &mut vertices {
        vertex.material = material;
        if let Some(gradient) = stroke.gradient {
            vertex.color = gradient.sample(vertex.gradient_t);
        }
    }
    vertices
}

fn tip_mesh(stroke: &Stroke) -> Vec<Vertex> {
    let Ok(dabs) = crate::dab_placement::sample(stroke) else {
        return Vec::new();
    };
    let mut vertices = Vec::with_capacity(dabs.len().saturating_mul(6));
    for dab in dabs {
        let tangent = dab.tangent.try_normalize().unwrap_or(Vec3::Y);
        let side = dab.normal.cross(tangent).try_normalize().unwrap_or(Vec3::X);
        let angle = (stroke.brush.angle_degrees + dab.angle_degrees).to_radians();
        let along = (tangent * angle.cos() + side * angle.sin())
            .try_normalize()
            .unwrap_or(tangent);
        let across = (side * angle.cos() - tangent * angle.sin())
            .try_normalize()
            .unwrap_or(side);
        let half_width = stroke.radius * dab.size_scale;
        let half_height = half_width / stroke.brush.tip_aspect;
        let positions = [
            dab.position - along * half_width + across * half_height,
            dab.position + along * half_width + across * half_height,
            dab.position + along * half_width - across * half_height,
            dab.position - along * half_width - across * half_height,
        ];
        let opacity = vertex_opacity(stroke, dab.pressure, dab.normal, dab.normal);
        flat_quad_gradient(
            &mut vertices,
            positions,
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            dab.normal,
            stroke.color,
            [opacity; 4],
            0.0,
            [dab.gradient_t; 4],
        );
    }
    vertices
}

fn dab_mesh(stroke: &Stroke) -> Vec<Vertex> {
    let Ok(dabs) = crate::dab_placement::sample(stroke) else {
        return Vec::new();
    };
    let mut vertices = Vec::with_capacity(dabs.len().saturating_mul(24));
    for dab in dabs {
        let tangent = dab.tangent.try_normalize().unwrap_or(Vec3::Y);
        let frame_side = dab.normal.cross(tangent).try_normalize().unwrap_or(Vec3::X);
        let angle = (stroke.brush.angle_degrees + dab.angle_degrees).to_radians();
        let width_axis = frame_side * angle.cos() + dab.normal * angle.sin();
        let normal_axis = dab.normal * angle.cos() - frame_side * angle.sin();
        let radius = stroke.radius * dab.size_scale;
        let normal_radius = if stroke.brush.kind == BrushKind::Round {
            radius
        } else {
            radius * stroke.brush.thickness
        };
        let axes = [
            width_axis * radius,
            normal_axis * normal_radius,
            tangent * radius,
        ];
        for sx in [-1.0_f32, 1.0] {
            for sy in [-1.0_f32, 1.0] {
                for sz in [-1.0_f32, 1.0] {
                    let offsets = [axes[0] * sx, axes[1] * sy, axes[2] * sz];
                    let outward = (offsets[0] + offsets[1] + offsets[2])
                        .try_normalize()
                        .unwrap_or(normal_axis);
                    let mut positions = offsets.map(|offset| dab.position + offset);
                    let mut face_normal = (positions[1] - positions[0])
                        .cross(positions[2] - positions[0])
                        .try_normalize()
                        .unwrap_or(outward);
                    if face_normal.dot(outward) < 0.0 {
                        positions.swap(1, 2);
                        face_normal = -face_normal;
                    }
                    for position in positions {
                        let offset = position - dab.position;
                        vertices.push(Vertex {
                            material: 0.0,
                            gradient_t: dab.gradient_t,
                            position: position.to_array(),
                            normal: face_normal.to_array(),
                            color: stroke.color,
                            uv: [
                                0.5 + offset.dot(width_axis) / stroke.brush.texture.scale,
                                0.5 + offset.dot(normal_axis) / stroke.brush.texture.scale,
                            ],
                            opacity: vertex_opacity(
                                stroke,
                                dab.pressure,
                                face_normal,
                                stroke.brush.normal,
                            ),
                        });
                    }
                }
            }
        }
    }
    vertices
}

fn is_closed_stroke(stroke: &Stroke) -> bool {
    stroke.points.len() >= 4
        && stroke
            .points
            .first()
            .zip(stroke.points.last())
            .is_some_and(|(first, last)| first.position.distance_squared(last.position) <= 1e-12)
}

#[derive(Clone, Copy)]
struct BrushFrame {
    center: Vec3,
    width: Vec3,
    normal: Vec3,
    tangent: Vec3,
    radius: f32,
    half_thickness: f32,
    u: f32,
    gradient_t: f32,
    opacity: f32,
    softness: f32,
    tilt: Option<Vec3>,
    thickness_scale: f32,
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

fn vertex_opacity(stroke: &Stroke, pressure: f32, normal: Vec3, reference: Vec3) -> f32 {
    stroke.brush.opacity
        * crate::dynamics::opacity_scale(stroke.brush.dynamics.opacity, pressure)
        * soft_coverage(normal, reference, stroke.brush.softness)
}

fn soft_coverage(normal: Vec3, reference: Vec3, softness: f32) -> f32 {
    let alignment = normal
        .try_normalize()
        .zip(reference.try_normalize())
        .map_or(1.0, |(a, b)| a.dot(b).abs().clamp(0.0, 1.0));
    let t = (alignment / 0.85).clamp(0.0, 1.0);
    let edge = t * t * (3.0 - 2.0 * t);
    1.0 - softness * (1.0 - edge)
}

fn ribbon_edge_coverage(across: f32, softness: f32) -> f32 {
    if softness <= 0.0 {
        return 1.0;
    }
    let distance = across.min(1.0 - across).clamp(0.0, 1.0);
    let t = (distance / (softness * 0.5)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn brush_frames(stroke: &Stroke) -> Vec<BrushFrame> {
    let mut points: Vec<crate::document::Point> = Vec::with_capacity(stroke.points.len());
    let mut point_scales: Vec<f32> = Vec::with_capacity(stroke.points.len());
    for (source_index, point) in stroke.points.iter().enumerate() {
        if let Some(last) = points.last_mut() {
            if last.position.distance_squared(point.position) <= 1e-12 {
                // Repeated pressure samples do not create zero-length mesh strips.
                *last = *point;
                if let Some(scale) = point_scales.last_mut() {
                    *scale = stroke.point_thickness_scale(source_index);
                }
                continue;
            }
        }
        points.push(*point);
        point_scales.push(stroke.point_thickness_scale(source_index));
    }
    let closed = is_closed_stroke(stroke)
        && points.len() >= 4
        && points
            .first()
            .zip(points.last())
            .is_some_and(|(first, last)| first.position.distance_squared(last.position) <= 1e-12);
    if closed {
        points.pop();
        point_scales.pop();
    }
    let distances = crate::dynamics::arc_lengths(&points);
    let closing_length = if closed {
        points
            .first()
            .zip(points.last())
            .map_or(0.0, |(first, last)| first.position.distance(last.position))
    } else {
        0.0
    };
    let total = distances.last().copied().unwrap_or(0.0) + closing_length;
    let reference = stroke.brush.normal.try_normalize().unwrap_or(Vec3::Z);
    let mut previous_manual: Option<(Vec3, Vec3)> = None;
    let mut frames: Vec<BrushFrame> = Vec::with_capacity(points.len());
    for (i, point) in points.iter().enumerate() {
        let before = if closed {
            points[(i + points.len() - 1) % points.len()].position
        } else {
            points[i.saturating_sub(1)].position
        };
        let after = if closed {
            points[(i + 1) % points.len()].position
        } else {
            points[(i + 1).min(points.len() - 1)].position
        };
        let tangent = (after - before)
            .try_normalize()
            .or_else(|| (point.position - before).try_normalize())
            .or_else(|| (after - point.position).try_normalize())
            .unwrap_or(stroke.brush.tangent);
        let manual_width = if let Some((previous_width, previous_normal)) = previous_manual {
            // Projection preserves the width sign even at an exact reversal.
            frame_direction(previous_width - tangent * previous_width.dot(tangent)).unwrap_or_else(
                || {
                    let candidate = previous_normal.cross(tangent).normalize_or_zero();
                    if candidate.length_squared() > 0.5 {
                        if candidate.dot(previous_width) < 0.0 {
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
        previous_manual = Some((manual_width, tangent.cross(manual_width).normalize()));
        let mut width = if stroke.brush.dynamics.tilt_orientation {
            point
                .tilt
                .and_then(|lean| frame_direction(lean - tangent * lean.dot(tangent)))
                .map(|axis| {
                    Quat::from_axis_angle(tangent, stroke.brush.angle_degrees.to_radians()) * axis
                })
                .unwrap_or(manual_width)
        } else {
            manual_width
        };
        // Width is an unoriented axis: choose the nearest hemisphere instead of
        // twisting 180 degrees as reported tilt passes an azimuth boundary.
        if frames
            .last()
            .is_some_and(|previous| width.dot(previous.width) < 0.0)
        {
            width = -width;
        }
        let taper = if closed {
            1.0
        } else {
            crate::dynamics::taper_factor(
                distances[i],
                total,
                stroke.brush.dynamics.taper_start,
                stroke.brush.dynamics.taper_end,
            )
        };
        frames.push(BrushFrame {
            center: point.position,
            u: distances[i] / stroke.brush.texture.scale,
            gradient_t: if total > 1.0e-8 {
                (distances[i] / total).clamp(0.0, 1.0)
            } else {
                0.0
            },
            width,
            normal: tangent.cross(width).normalize(),
            tangent,
            radius: stroke.radius
                * point_scales[i]
                * crate::dynamics::pressure_scale(stroke.brush.dynamics.width, point.pressure)
                * crate::dynamics::shading_width_scale(
                    stroke.brush.dynamics,
                    stroke.brush.texture.kind,
                    point.tilt,
                )
                * taper,
            half_thickness: stroke.radius
                * point_scales[i]
                * stroke.brush.thickness
                * crate::dynamics::pressure_scale(stroke.brush.dynamics.thickness, point.pressure)
                * taper,
            opacity: stroke.brush.opacity
                * crate::dynamics::opacity_scale(stroke.brush.dynamics.opacity, point.pressure),
            softness: stroke.brush.softness,
            tilt: point.tilt,
            thickness_scale: crate::dynamics::pressure_scale(
                stroke.brush.dynamics.thickness,
                point.pressure,
            ) * point_scales[i],
        });
    }
    if closed && !frames.is_empty() {
        let mut seam = frames[0];
        seam.u = total / stroke.brush.texture.scale;
        frames.push(seam);
    }
    frames
}

pub(crate) const MAX_STRAND_VERTICES: usize = 400_000;
const STRAND_SIDES: usize = 6;
pub(crate) const MAX_AIRBRUSH_POINTS: usize = 65_536;
pub(crate) const MAX_AIRBRUSH_DEPOSITS: usize = 8_192;
pub(crate) const MAX_AIRBRUSH_VERTICES: usize = MAX_AIRBRUSH_DEPOSITS * 72;
const AIRBRUSH_CELL_MS: u32 = 32;
pub(crate) const AIRBRUSH_TICK_MS: u32 = AIRBRUSH_CELL_MS;
const AIRBRUSH_DISC_SIDES: usize = 8;

fn strand_mesh(stroke: &Stroke) -> Vec<Vertex> {
    let settings = stroke.brush.strands;
    let frames = brush_frames(stroke);
    if frames.is_empty() || settings.vertex_estimate(stroke.points.len()) > MAX_STRAND_VERTICES {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(settings.vertex_estimate(stroke.points.len()));
    let mut path = frames.clone();
    if path.len() == 1 {
        let frame = path[0];
        let half = frame.radius.max(0.001) * 0.65;
        let mut start = frame;
        start.center -= frame.tangent * half;
        start.u -= half / stroke.brush.texture.scale;
        let mut end = frame;
        end.center += frame.tangent * half;
        end.u += half / stroke.brush.texture.scale;
        path = vec![start, end];
    }
    let scale_at = |frame: BrushFrame| {
        1.0 + settings.tilt_spread * frame.tilt.map_or(0.0, |tilt| tilt.length().clamp(0.0, 1.0))
    };
    for lane in 0..settings.count {
        let lane_position = (lane as f32 + 0.5) / settings.count as f32 * 2.0 - 1.0;
        let ring = |frame: BrushFrame| {
            let spread = scale_at(frame);
            let pitch_radius = frame.radius * settings.spread * spread / settings.count as f32;
            let center = frame.center
                + frame.width * (lane_position * frame.radius * settings.spread * spread);
            let radius = pitch_radius * settings.thickness * frame.thickness_scale;
            std::array::from_fn::<_, STRAND_SIDES, _>(|side| {
                let angle = side as f32 * std::f32::consts::TAU / STRAND_SIDES as f32;
                let normal = frame.width * angle.cos() + frame.normal * angle.sin();
                (center + normal * radius, normal)
            })
        };
        for pair in path.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            let (ring_a, ring_b) = (ring(a), ring(b));
            for side in 0..STRAND_SIDES {
                let next = (side + 1) % STRAND_SIDES;
                let outward = ring_a[side].1 + ring_a[next].1 + ring_b[side].1 + ring_b[next].1;
                let v0 = side as f32 / STRAND_SIDES as f32;
                let v1 = (side + 1) as f32 / STRAND_SIDES as f32;
                flat_quad_gradient(
                    &mut out,
                    [
                        ring_a[side].0,
                        ring_b[side].0,
                        ring_b[next].0,
                        ring_a[next].0,
                    ],
                    [[a.u, v0], [b.u, v0], [b.u, v1], [a.u, v1]],
                    outward,
                    stroke.color,
                    [a.opacity, b.opacity, b.opacity, a.opacity],
                    (a.softness + b.softness) * 0.5,
                    [a.gradient_t, b.gradient_t, b.gradient_t, a.gradient_t],
                );
            }
        }
        for (frame, outward) in [
            (path[0], -path[0].tangent),
            (*path.last().unwrap(), path.last().unwrap().tangent),
        ] {
            let vertices = ring(frame);
            for side in 0..STRAND_SIDES {
                let next = (side + 1) % STRAND_SIDES;
                flat_triangle_gradient(
                    &mut out,
                    [
                        frame.center + (vertices[side].0 - frame.center),
                        frame.center + (vertices[next].0 - frame.center),
                        frame.center,
                    ],
                    [
                        [frame.u, side as f32 / STRAND_SIDES as f32],
                        [frame.u, (side + 1) as f32 / STRAND_SIDES as f32],
                        [frame.u, 0.5],
                    ],
                    outward,
                    stroke.color,
                    [frame.opacity; 3],
                    frame.softness,
                    [frame.gradient_t; 3],
                );
            }
        }
    }
    out
}

fn airbrush_timeline(stroke: &Stroke) -> Vec<Point> {
    if stroke.points.is_empty() || stroke.points.len() > MAX_AIRBRUSH_POINTS {
        return Vec::new();
    }
    let first_time = stroke.points[0].time_ms;
    let last_time = stroke.points.last().unwrap().time_ms;
    if stroke
        .points
        .windows(2)
        .any(|pair| pair[0].time_ms > pair[1].time_ms)
    {
        return Vec::new();
    }
    if last_time > first_time {
        return stroke.points.clone();
    }

    // Shape tools and very short untimed input still produce one deterministic
    // pass. Duration is derived from path length rather than point count.
    let lengths = crate::dynamics::arc_lengths(&stroke.points);
    let total = lengths.last().copied().unwrap_or(0.0);
    if total <= 1e-8 {
        let mut start = stroke.points[0];
        start.time_ms = 0;
        let mut end = *stroke.points.last().unwrap();
        end.time_ms = AIRBRUSH_CELL_MS;
        return vec![start, end];
    }
    let spacing = (stroke.radius * 0.5).max(0.001);
    let cells = (total / spacing).ceil().max(1.0) as u32;
    let duration = cells
        .saturating_mul(AIRBRUSH_CELL_MS)
        .min(crate::document::MAX_FLOW_DURATION_MS);
    stroke
        .points
        .iter()
        .zip(lengths)
        .map(|(point, distance)| {
            let mut point = *point;
            point.time_ms = ((distance / total) * duration as f32).round() as u32;
            point
        })
        .collect()
}

fn point_at_time(points: &[Point], time_ms: u32) -> Point {
    let index = points
        .partition_point(|point| point.time_ms <= time_ms)
        .saturating_sub(1);
    let a = points[index];
    let Some(b) = points.get(index + 1).copied() else {
        return a;
    };
    if time_ms <= a.time_ms || b.time_ms <= a.time_ms {
        return a;
    }
    let t = (time_ms - a.time_ms) as f32 / (b.time_ms - a.time_ms) as f32;
    Point {
        position: a.position.lerp(b.position, t),
        pressure: a.pressure + (b.pressure - a.pressure) * t,
        tilt: crate::dynamics::interpolate_tilt(a.tilt, b.tilt, t),
        time_ms,
    }
}

fn path_between_times(points: &[Point], start: u32, end: u32) -> Vec<Point> {
    let mut path = vec![point_at_time(points, start)];
    let interior_start = points.partition_point(|point| point.time_ms <= start);
    let interior_end = points.partition_point(|point| point.time_ms < end);
    path.extend(points[interior_start..interior_end].iter().copied());
    let last = point_at_time(points, end);
    if path.len() == 1
        || path
            .last()
            .is_none_or(|previous| previous.position.distance_squared(last.position) > 1e-12)
    {
        path.push(last);
    } else if let Some(previous) = path.last_mut() {
        *previous = last;
    }
    path
}

fn path_length(points: &[Point]) -> f32 {
    points
        .windows(2)
        .map(|pair| pair[0].position.distance(pair[1].position))
        .sum()
}

fn path_length_between_times(points: &[Point], start: u32, end: u32) -> f32 {
    let first = point_at_time(points, start);
    let last = point_at_time(points, end);
    let interior_start = points.partition_point(|point| point.time_ms <= start);
    let interior_end = points.partition_point(|point| point.time_ms < end);
    let mut previous = first.position;
    let mut length = 0.0;
    for point in &points[interior_start..interior_end] {
        length += previous.distance(point.position);
        previous = point.position;
    }
    length + previous.distance(last.position)
}

fn sample_path_distance(points: &[Point], distance: f32) -> (Point, Vec3) {
    let mut walked = 0.0;
    for pair in points.windows(2) {
        let delta = pair[1].position - pair[0].position;
        let length = delta.length();
        if length > 1e-8 && distance <= walked + length {
            let t = ((distance - walked) / length).clamp(0.0, 1.0);
            return (
                Point {
                    position: pair[0].position.lerp(pair[1].position, t),
                    pressure: pair[0].pressure + (pair[1].pressure - pair[0].pressure) * t,
                    tilt: crate::dynamics::interpolate_tilt(pair[0].tilt, pair[1].tilt, t),
                    time_ms: (pair[0].time_ms as f32
                        + (pair[1].time_ms as f32 - pair[0].time_ms as f32) * t)
                        .round() as u32,
                },
                delta / length,
            );
        }
        walked += length;
    }
    (
        points.last().copied().unwrap_or(Point {
            position: Vec3::ZERO,
            pressure: 1.0,
            tilt: None,
            time_ms: 0,
        }),
        points
            .windows(2)
            .rev()
            .find_map(|pair| (pair[1].position - pair[0].position).try_normalize())
            .unwrap_or(Vec3::Y),
    )
}

fn airbrush_deposits(
    stroke: &Stroke,
    timeline: &[Point],
    visit: &mut impl FnMut(Point, Vec3, f32, f32),
) -> usize {
    let (Some(first), Some(last)) = (timeline.first(), timeline.last()) else {
        return 0;
    };
    let mut cursor = first.time_ms;
    let mut count = 0usize;
    let mut walked = 0.0;
    let spacing = (stroke.radius * 0.5).max(0.001);
    while cursor < last.time_ms {
        let next_boundary = (cursor / AIRBRUSH_CELL_MS + 1).saturating_mul(AIRBRUSH_CELL_MS);
        let end = next_boundary.min(last.time_ms);
        if end <= cursor {
            break;
        }
        let path = path_between_times(timeline, cursor, end);
        let length = path_length(&path);
        let samples = (length / spacing).ceil().max(1.0) as usize;
        let seconds_per_sample = (end - cursor) as f32 / 1000.0 / samples as f32;
        for index in 0..samples {
            let distance = length * (index as f32 + 0.5) / samples as f32;
            let (point, tangent) = sample_path_distance(&path, distance);
            visit(point, tangent, seconds_per_sample, walked + distance);
            count = count.saturating_add(1);
            if count > MAX_AIRBRUSH_DEPOSITS {
                return count;
            }
        }
        walked += length;
        cursor = end;
    }
    count
}

pub(crate) fn airbrush_vertex_estimate(stroke: &Stroke) -> usize {
    if stroke.points.is_empty() || stroke.points.len() > MAX_AIRBRUSH_POINTS {
        return MAX_AIRBRUSH_VERTICES.saturating_add(1);
    }
    let synthetic;
    let timestamps_valid = stroke
        .points
        .windows(2)
        .all(|pair| pair[0].time_ms <= pair[1].time_ms);
    let timeline = if timestamps_valid
        && stroke.points.first().unwrap().time_ms < stroke.points.last().unwrap().time_ms
    {
        stroke.points.as_slice()
    } else {
        synthetic = airbrush_timeline(stroke);
        synthetic.as_slice()
    };
    if timeline.is_empty() {
        return MAX_AIRBRUSH_VERTICES.saturating_add(1);
    }
    let (Some(first), Some(last)) = (timeline.first(), timeline.last()) else {
        return 0;
    };
    let mut cursor = first.time_ms;
    let mut deposits = 0usize;
    let spacing = (stroke.radius * 0.5).max(0.001);
    while cursor < last.time_ms {
        let next_boundary = (cursor / AIRBRUSH_CELL_MS + 1).saturating_mul(AIRBRUSH_CELL_MS);
        let end = next_boundary.min(last.time_ms);
        if end <= cursor {
            break;
        }
        let length = path_length_between_times(timeline, cursor, end);
        deposits = deposits.saturating_add((length / spacing).ceil().max(1.0) as usize);
        if deposits > MAX_AIRBRUSH_DEPOSITS {
            return MAX_AIRBRUSH_VERTICES.saturating_add(1);
        }
        cursor = end;
    }
    if deposits == 0 {
        deposits = 1;
    }
    deposits.saturating_mul(72)
}

fn airbrush_mesh(stroke: &Stroke) -> Vec<Vertex> {
    let timeline = airbrush_timeline(stroke);
    let capacity = airbrush_vertex_estimate(stroke);
    if timeline.is_empty() || capacity > MAX_AIRBRUSH_VERTICES {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(capacity);
    let total_length = path_length(&timeline);
    let mut previous_width = None;
    let _ = airbrush_deposits(
        stroke,
        &timeline,
        &mut |point, tangent, seconds, traveled| {
            let tangent = tangent
                .try_normalize()
                .unwrap_or_else(|| stroke.brush.tangent.try_normalize().unwrap_or(Vec3::Y));
            let reference = stroke.brush.normal.try_normalize().unwrap_or(Vec3::Z);
            let manual_width = frame_direction(reference - tangent * reference.dot(tangent))
                .or_else(|| {
                    frame_direction(
                        stroke.brush.tangent - tangent * stroke.brush.tangent.dot(tangent),
                    )
                })
                .unwrap_or_else(|| perpendicular(tangent));
            let mut width = if stroke.brush.dynamics.tilt_orientation {
                point
                    .tilt
                    .and_then(|lean| frame_direction(lean - tangent * lean.dot(tangent)))
                    .map(|axis| {
                        Quat::from_axis_angle(tangent, stroke.brush.angle_degrees.to_radians())
                            * axis
                    })
                    .unwrap_or(manual_width)
            } else {
                Quat::from_axis_angle(tangent, stroke.brush.angle_degrees.to_radians())
                    * manual_width
            };
            if previous_width.is_some_and(|previous: Vec3| width.dot(previous) < 0.0) {
                width = -width;
            }
            previous_width = Some(width);
            let normal = tangent.cross(width).try_normalize().unwrap_or(reference);
            let taper = crate::dynamics::taper_factor(
                traveled,
                total_length,
                stroke.brush.dynamics.taper_start,
                stroke.brush.dynamics.taper_end,
            );
            let radius = stroke.radius
                * point_thickness_at_time(stroke, point.time_ms)
                * crate::dynamics::pressure_scale(stroke.brush.dynamics.width, point.pressure)
                * crate::dynamics::shading_width_scale(
                    stroke.brush.dynamics,
                    stroke.brush.texture.kind,
                    point.tilt,
                )
                * taper;
            let pressure = point.pressure.clamp(0.0, 1.0);
            let dose = 1.0 - (-stroke.brush.flow.rate_per_second * pressure * seconds).exp();
            let opacity = stroke.brush.opacity
                * crate::dynamics::opacity_scale(stroke.brush.dynamics.opacity, point.pressure)
                * dose;
            emit_airbrush_disc(
                &mut out,
                stroke,
                AirbrushDiscParameters {
                    center: point.position,
                    width,
                    normal,
                    radius,
                    opacity,
                    softness: stroke.brush.softness,
                    gradient_t: if total_length > 1.0e-8 {
                        (traveled / total_length).clamp(0.0, 1.0)
                    } else {
                        0.0
                    },
                },
            );
        },
    );
    out
}

fn point_thickness_at_time(stroke: &Stroke, time_ms: u32) -> f32 {
    let Some(values) = stroke.point_thickness.as_ref() else {
        return 1.0;
    };
    if values.len() != stroke.points.len() || values.is_empty() {
        return 1.0;
    }
    if values.len() == 1 || time_ms <= stroke.points[0].time_ms {
        return values[0];
    }
    let last = stroke.points.len() - 1;
    if time_ms >= stroke.points[last].time_ms {
        return values[last];
    }
    let upper = stroke
        .points
        .partition_point(|point| point.time_ms < time_ms);
    let segment = upper.saturating_sub(1).min(last - 1);
    let start = stroke.points[segment].time_ms;
    let end = stroke.points[segment + 1].time_ms;
    let t = if end > start {
        (time_ms - start) as f32 / (end - start) as f32
    } else {
        0.0
    };
    values[segment] + (values[segment + 1] - values[segment]) * t
}

struct AirbrushDiscParameters {
    center: Vec3,
    width: Vec3,
    normal: Vec3,
    radius: f32,
    opacity: f32,
    softness: f32,
    gradient_t: f32,
}

fn emit_airbrush_disc(out: &mut Vec<Vertex>, stroke: &Stroke, parameters: AirbrushDiscParameters) {
    let AirbrushDiscParameters {
        center,
        width,
        normal,
        radius,
        opacity,
        softness,
        gradient_t,
    } = parameters;
    if radius <= 1e-8 || opacity <= 1e-8 {
        return;
    }
    let inner_radius = radius * (0.45 - softness.clamp(0.0, 1.0) * 0.38);
    let inner = std::array::from_fn::<_, AIRBRUSH_DISC_SIDES, _>(|index| {
        let angle = index as f32 * std::f32::consts::TAU / AIRBRUSH_DISC_SIDES as f32;
        let axis = width * angle.cos() + normal * angle.sin();
        (center + axis * inner_radius, axis)
    });
    let outer = std::array::from_fn::<_, AIRBRUSH_DISC_SIDES, _>(|index| {
        let angle = index as f32 * std::f32::consts::TAU / AIRBRUSH_DISC_SIDES as f32;
        let axis = width * angle.cos() + normal * angle.sin();
        (center + axis * radius, axis)
    });
    for index in 0..AIRBRUSH_DISC_SIDES {
        let next = (index + 1) % AIRBRUSH_DISC_SIDES;
        let uv = |offset: Vec3| {
            [
                0.5 + offset.dot(width) / stroke.brush.texture.scale,
                0.5 + offset.dot(normal) / stroke.brush.texture.scale,
            ]
        };
        flat_triangle_gradient(
            out,
            [center, inner[index].0, inner[next].0],
            [
                [0.5, 0.5],
                uv(inner[index].0 - center),
                uv(inner[next].0 - center),
            ],
            normal,
            stroke.color,
            [opacity; 3],
            0.0,
            [gradient_t; 3],
        );
        flat_quad_gradient(
            out,
            [inner[index].0, outer[index].0, outer[next].0, inner[next].0],
            [
                uv(inner[index].0 - center),
                uv(outer[index].0 - center),
                uv(outer[next].0 - center),
                uv(inner[next].0 - center),
            ],
            normal,
            stroke.color,
            [opacity, 0.0, 0.0, opacity],
            0.0,
            [gradient_t; 4],
        );
    }
}

fn flat_triangle(
    out: &mut Vec<Vertex>,
    mut positions: [Vec3; 3],
    mut uv: [[f32; 2]; 3],
    outward: Vec3,
    color: [f32; 3],
    opacity: [f32; 3],
    softness: f32,
) {
    let cross = (positions[1] - positions[0]).cross(positions[2] - positions[0]);
    let Some(mut normal) = cross.try_normalize() else {
        return;
    };
    if normal.dot(outward) < 0.0 {
        positions.swap(1, 2);
        uv.swap(1, 2);
        normal = -normal;
    }
    let coverage = soft_coverage(normal, outward, softness);
    out.extend(
        positions
            .into_iter()
            .zip(uv)
            .zip(opacity)
            .map(|((position, uv), opacity)| Vertex {
                material: 0.0,
                gradient_t: 0.0,
                position: position.to_array(),
                normal: normal.to_array(),
                color,
                uv,
                opacity: opacity * coverage,
            }),
    );
}

fn flat_quad(
    out: &mut Vec<Vertex>,
    positions: [Vec3; 4],
    uv: [[f32; 2]; 4],
    outward: Vec3,
    color: [f32; 3],
    opacity: [f32; 4],
    softness: f32,
) {
    flat_triangle(
        out,
        [positions[0], positions[1], positions[2]],
        [uv[0], uv[1], uv[2]],
        outward,
        color,
        [opacity[0], opacity[1], opacity[2]],
        softness,
    );
    flat_triangle(
        out,
        [positions[0], positions[2], positions[3]],
        [uv[0], uv[2], uv[3]],
        outward,
        color,
        [opacity[0], opacity[2], opacity[3]],
        softness,
    );
}

fn apply_gradient_parameters(vertices: &mut [Vertex], positions: &[Vec3], parameters: &[f32]) {
    for vertex in vertices {
        let position = Vec3::from(vertex.position);
        let nearest = positions
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                position
                    .distance_squared(**a)
                    .total_cmp(&position.distance_squared(**b))
            })
            .map_or(0, |(index, _)| index);
        vertex.gradient_t = parameters[nearest];
    }
}

// These helpers mirror the fixed per-vertex arrays of `flat_triangle`; keeping
// each vertex attribute explicit makes the mesh emission call sites auditable.
#[allow(clippy::too_many_arguments)]
fn flat_triangle_gradient(
    out: &mut Vec<Vertex>,
    positions: [Vec3; 3],
    uv: [[f32; 2]; 3],
    outward: Vec3,
    color: [f32; 3],
    opacity: [f32; 3],
    softness: f32,
    gradient_t: [f32; 3],
) {
    let start = out.len();
    flat_triangle(out, positions, uv, outward, color, opacity, softness);
    apply_gradient_parameters(&mut out[start..], &positions, &gradient_t);
}

// See `flat_triangle_gradient`: the four-vertex attributes intentionally stay
// parallel fixed-size arrays at this rendering boundary.
#[allow(clippy::too_many_arguments)]
fn flat_quad_gradient(
    out: &mut Vec<Vertex>,
    positions: [Vec3; 4],
    uv: [[f32; 2]; 4],
    outward: Vec3,
    color: [f32; 3],
    opacity: [f32; 4],
    softness: f32,
    gradient_t: [f32; 4],
) {
    let start = out.len();
    flat_quad(out, positions, uv, outward, color, opacity, softness);
    apply_gradient_parameters(&mut out[start..], &positions, &gradient_t);
}

fn flat_brush(stroke: &Stroke) -> Vec<Vertex> {
    let frames = brush_frames(stroke);
    let mut out = Vec::new();
    let Some(first) = frames.first() else {
        return out;
    };
    if frames.len() == 1 {
        // A tap has a finite footprint in its frozen frame. Its longitudinal
        // texture scale follows the same world period as a full stroke.
        let f = first;
        let a = f.width * f.radius;
        let b = f.tangent * f.radius;
        let h = if stroke.brush.kind == BrushKind::Chisel {
            f.normal * f.half_thickness
        } else {
            Vec3::ZERO
        };
        let corners = [
            f.center + a + b,
            f.center - a + b,
            f.center - a - b,
            f.center + a - b,
        ];
        let r = f.radius / stroke.brush.texture.scale;
        let uv = [
            [0.5 + r, 1.0],
            [0.5 + r, 0.0],
            [0.5 - r, 0.0],
            [0.5 - r, 1.0],
        ];
        flat_quad_gradient(
            &mut out,
            corners.map(|p| p + h),
            uv,
            f.normal,
            stroke.color,
            [f.opacity; 4],
            f.softness,
            [f.gradient_t; 4],
        );
        if stroke.brush.kind == BrushKind::Chisel {
            flat_quad_gradient(
                &mut out,
                corners.map(|p| p - h),
                uv,
                -f.normal,
                stroke.color,
                [f.opacity; 4],
                f.softness,
                [f.gradient_t; 4],
            );
            for j in 0..4 {
                let k = (j + 1) % 4;
                let outward = (corners[j] + corners[k]) * 0.5 - f.center;
                flat_quad_gradient(
                    &mut out,
                    [
                        corners[j] + h,
                        corners[k] + h,
                        corners[k] - h,
                        corners[j] - h,
                    ],
                    [uv[j], uv[k], uv[k], uv[j]],
                    outward,
                    stroke.color,
                    [f.opacity; 4],
                    f.softness,
                    [f.gradient_t; 4],
                );
            }
        }
        return out;
    }
    if stroke.brush.kind == BrushKind::Ribbon {
        for pair in frames.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            let left_a = a.center - a.width * a.radius;
            let right_a = a.center + a.width * a.radius;
            let left_b = b.center - b.width * b.radius;
            let right_b = b.center + b.width * b.radius;
            let softness = (a.softness + b.softness) * 0.5;
            let bands = if softness > 0.0 { 4 } else { 1 };
            for index in 0..bands {
                let t0 = index as f32 / bands as f32;
                let t1 = (index + 1) as f32 / bands as f32;
                let coverage = [
                    ribbon_edge_coverage(t0, softness),
                    ribbon_edge_coverage(t1, softness),
                    ribbon_edge_coverage(t1, softness),
                    ribbon_edge_coverage(t0, softness),
                ];
                flat_quad_gradient(
                    &mut out,
                    [
                        left_a.lerp(right_a, t0),
                        left_a.lerp(right_a, t1),
                        left_b.lerp(right_b, t1),
                        left_b.lerp(right_b, t0),
                    ],
                    [[a.u, t0], [a.u, t1], [b.u, t1], [b.u, t0]],
                    a.normal + b.normal,
                    stroke.color,
                    [
                        a.opacity * coverage[0],
                        a.opacity * coverage[1],
                        b.opacity * coverage[2],
                        b.opacity * coverage[3],
                    ],
                    softness,
                    [a.gradient_t, a.gradient_t, b.gradient_t, b.gradient_t],
                );
            }
        }
    } else {
        let ring = |f: BrushFrame| {
            let a = f.width * f.radius;
            let b = f.normal * f.half_thickness;
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
                // Unwrap the complete rectangular perimeter, with a duplicated
                // seam at V=1. Adjacent faces agree at every shared edge.
                let v = j as f32 / 4.0;
                let next = (j + 1) as f32 / 4.0;
                flat_quad_gradient(
                    &mut out,
                    [ar[j], br[j], br[k], ar[k]],
                    [[a.u, v], [b.u, v], [b.u, next], [a.u, next]],
                    outward,
                    stroke.color,
                    [a.opacity, b.opacity, b.opacity, a.opacity],
                    (a.softness + b.softness) * 0.5,
                    [a.gradient_t, b.gradient_t, b.gradient_t, a.gradient_t],
                );
            }
        }
        let cap_uv = |f: &BrushFrame| {
            let width = f.radius / stroke.brush.texture.scale;
            let height = f.half_thickness / stroke.brush.texture.scale;
            [
                [0.5 + width, 0.5 + height],
                [0.5 - width, 0.5 + height],
                [0.5 - width, 0.5 - height],
                [0.5 + width, 0.5 - height],
            ]
        };
        if !is_closed_stroke(stroke) {
            flat_quad_gradient(
                &mut out,
                ring(*first),
                cap_uv(first),
                -first.tangent,
                stroke.color,
                [first.opacity; 4],
                first.softness,
                [first.gradient_t; 4],
            );
            let last = frames.last().unwrap();
            flat_quad_gradient(
                &mut out,
                ring(*last),
                cap_uv(last),
                last.tangent,
                stroke.color,
                [last.opacity; 4],
                last.softness,
                [last.gradient_t; 4],
            );
        }
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
                    material: 0.0,
                    gradient_t: 0.0,
                    position: (origin + a * i as f32 * 0.5 + b * 6.0 * sign).to_array(),
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                    color,
                    opacity: 1.0,
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
            gradient: None,
            curve: None,
            material: Default::default(),
            id: 1,
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            color: [0.5; 3],
            radius: 0.1,
            brush: Default::default(),
            points: vec![
                Point {
                    tilt: None,
                    position: Vec3::ZERO,
                    pressure,

                    time_ms: 0,
                },
                Point {
                    tilt: None,
                    position: Vec3::Y,
                    pressure,

                    time_ms: 0,
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
                    tilt: None,
                    position: Vec3::new(t.cos(), t.sin(), 0.0),
                    pressure: 1.0,

                    time_ms: 0,
                }
            })
            .collect();
        let stroke = Stroke {
            gradient: None,
            curve: None,
            material: Default::default(),
            id: 1,
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
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
            gradient: None,
            curve: None,
            material: Default::default(),
            id: 7,
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            color: [0.3, 0.6, 0.9],
            radius: 0.1,
            brush: Brush {
                kind,
                ..Default::default()
            },
            points: positions
                .iter()
                .map(|&position| Point {
                    tilt: None,
                    position,
                    pressure: 1.0,

                    time_ms: 0,
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

    fn strand_mark(name: &str, positions: &[Vec3]) -> Stroke {
        let preset = crate::presets::Library::default()
            .presets
            .into_iter()
            .find(|preset| preset.name == name)
            .unwrap_or_else(|| panic!("missing built-in {name} preset"));
        let mut stroke = mark(BrushKind::Ribbon, positions);
        stroke.radius = preset.radius;
        stroke.brush = preset.brush;
        stroke
    }

    fn profiled_mark(profile: crate::document::TubeProfile, positions: &[Vec3]) -> Stroke {
        let mut stroke = mark(BrushKind::Round, positions);
        stroke.brush.tube_profile = profile;
        stroke
    }

    fn profile_corners(mesh: &[Vertex], sides: usize) -> Vec<Vec2> {
        let mut corners = Vec::new();
        for vertex in &mesh[..sides * 6] {
            let position = Vec3::from(vertex.position);
            if position.x.abs() <= 1.0e-6 {
                let corner = Vec2::new(position.y, position.z);
                if corners
                    .iter()
                    .all(|existing: &Vec2| existing.distance(corner) > 1.0e-5)
                {
                    corners.push(corner);
                }
            }
        }
        corners
    }

    fn profile_radius_at(mesh: &[Vertex], x: f32) -> f32 {
        mesh.iter()
            .filter_map(|vertex| {
                let position = Vec3::from(vertex.position);
                ((position.x - x).abs() <= 1.0e-5)
                    .then(|| Vec2::new(position.y, position.z).length())
            })
            .fold(0.0, f32::max)
    }

    fn strand_centroid(vertices: &[Vertex]) -> Vec3 {
        vertices
            .iter()
            .map(|vertex| Vec3::from(vertex.position))
            .sum::<Vec3>()
            / vertices.len() as f32
    }

    fn assert_same_mesh(left: &[Vertex], right: &[Vertex], context: &str) {
        assert_eq!(left.len(), right.len(), "{context} vertex count");
        for (left, right) in left.iter().zip(right) {
            assert_eq!(left.position, right.position, "{context} positions");
            assert_eq!(left.normal, right.normal, "{context} normals");
            assert_eq!(left.color, right.color, "{context} colors");
            assert_eq!(left.uv, right.uv, "{context} UVs");
            assert_eq!(left.opacity, right.opacity, "{context} opacity");
            assert_eq!(left.material, right.material, "{context} materials");
            assert_eq!(left.gradient_t, right.gradient_t, "{context} gradients");
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

    #[test]
    fn square_triangle_and_star_profiles_have_distinct_silhouettes_and_side_counts() {
        for (profile, sides) in [
            (crate::document::TubeProfile::Square, 4),
            (crate::document::TubeProfile::Triangle, 3),
            (crate::document::TubeProfile::Star, 10),
        ] {
            let stroke = profiled_mark(profile, &[Vec3::ZERO, Vec3::X]);
            let mesh = stroke_mesh(&stroke);
            assert_eq!(mesh.len(), 12 * sides);
            assert_eq!(profile_tube_vertex_estimate(&stroke), mesh.len());
            valid(&mesh);

            let corners = profile_corners(&mesh, sides);
            assert_eq!(corners.len(), sides, "{profile:?} cross-section corners");
            let mut radii: Vec<_> = corners.iter().map(|corner| corner.length()).collect();
            radii.sort_by(f32::total_cmp);
            if profile == crate::document::TubeProfile::Star {
                assert!(radii[..5]
                    .iter()
                    .all(|radius| (radius - 0.42 * stroke.radius).abs() < 1e-5));
                assert!(radii[5..]
                    .iter()
                    .all(|radius| (radius - stroke.radius).abs() < 1e-5));
            } else {
                assert!(radii
                    .iter()
                    .all(|radius| (radius - stroke.radius).abs() < 1e-5));
            }
            if profile == crate::document::TubeProfile::Square {
                assert!(corners
                    .iter()
                    .all(|corner| (corner.x.abs() - corner.y.abs()).abs() < 1e-5));
            }
        }
    }

    #[test]
    fn profiled_tubes_have_finite_wound_faces_and_oriented_caps_for_degenerate_paths() {
        let paths = [
            vec![Vec3::ZERO],
            vec![Vec3::ZERO, Vec3::ZERO, Vec3::ZERO],
            vec![Vec3::ZERO, Vec3::X * 1.0e-8, Vec3::X],
            vec![Vec3::ZERO, Vec3::X, Vec3::ZERO, Vec3::new(-1.0, 0.2, 0.0)],
        ];
        for profile in [
            crate::document::TubeProfile::Square,
            crate::document::TubeProfile::Triangle,
            crate::document::TubeProfile::Star,
        ] {
            let sides = profile_sides(profile);
            for path in &paths {
                let stroke = profiled_mark(profile, path);
                let mesh = stroke_mesh(&stroke);
                valid(&mesh);

                let frames = brush_frames(&stroke);
                assert!(!frames.is_empty());
                let side_segments = frames.len().saturating_sub(1).max(1);
                let side_vertices = side_segments * sides * 6;
                let cap_vertices_per_end = sides * 3;
                assert_eq!(
                    mesh.len(),
                    side_vertices + cap_vertices_per_end * 2,
                    "{profile:?} capped path geometry"
                );
                let start_cap = &mesh[side_vertices..side_vertices + cap_vertices_per_end];
                let end_cap = &mesh[side_vertices + cap_vertices_per_end..];
                let start_normal = -frames[0].tangent;
                let end_normal = frames.last().unwrap().tangent;
                assert!(start_cap
                    .iter()
                    .all(|vertex| { Vec3::from(vertex.normal).dot(start_normal) > 0.9999 }));
                assert!(end_cap
                    .iter()
                    .all(|vertex| Vec3::from(vertex.normal).dot(end_normal) > 0.9999));
            }
        }
    }

    #[test]
    fn profiled_tubes_apply_pressure_and_taper_to_the_cross_section() {
        for profile in [
            crate::document::TubeProfile::Square,
            crate::document::TubeProfile::Triangle,
            crate::document::TubeProfile::Star,
        ] {
            let positions = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0];
            let mut low_pressure = profiled_mark(profile, &positions);
            low_pressure.brush.dynamics.width = crate::document::PressureMapping {
                enabled: true,
                minimum: 0.3,
                curve: 1.0,
            };
            for point in &mut low_pressure.points {
                point.pressure = 0.0;
            }
            let mut high_pressure = low_pressure.clone();
            for point in &mut high_pressure.points {
                point.pressure = 1.0;
            }
            let low_mesh = stroke_mesh(&low_pressure);
            let high_mesh = stroke_mesh(&high_pressure);
            let low_radius = profile_radius_at(&low_mesh, 1.0);
            let high_radius = profile_radius_at(&high_mesh, 1.0);
            assert!(low_radius > 0.0 && high_radius > low_radius);
            assert!((low_radius / high_radius - 0.3).abs() < 1.0e-5);

            let mut tapered = high_pressure;
            tapered.brush.dynamics.taper_end = 0.5;
            let tapered_mesh = stroke_mesh(&tapered);
            let middle_radius = profile_radius_at(&tapered_mesh, 1.0);
            let end_radius = profile_radius_at(&tapered_mesh, 2.0);
            assert!(
                end_radius < middle_radius * 0.02,
                "{profile:?} taper reaches its end"
            );
        }
    }

    #[test]
    fn transformed_profile_mesh_stays_within_pick_extent_and_vertex_budget() {
        let mut source = profiled_mark(
            crate::document::TubeProfile::Square,
            &[Vec3::new(-0.5, 0.0, 0.0), Vec3::new(0.5, 0.0, 0.0)],
        );
        source.id = 713;
        source.radius = 0.7;
        let rotation = Quat::from_rotation_z(0.6);
        let translation = Vec3::new(0.1, -0.2, 0.0);
        let scale = 1.2;
        let before_mesh = stroke_mesh(&source);
        let before_extent = stroke_extent(&source);
        let before = vec![(0, source.clone())];
        let transformed =
            crate::editing::transformed(&before, Vec3::ZERO, translation, rotation, scale)
                .unwrap()
                .remove(0)
                .1;
        let transformed_mesh = stroke_mesh(&transformed);
        assert_eq!(before_mesh.len(), transformed_mesh.len());
        assert!(transformed_mesh.len() <= MAX_PROFILE_TUBE_VERTICES);
        assert!(
            (stroke_extent(&transformed) - before_extent * scale).abs() < 1.0e-5,
            "profile extent scales with the transformed cross-section"
        );
        for (before, after) in before_mesh.iter().zip(&transformed_mesh) {
            assert!(
                (rotation * Vec3::from(before.position) * scale + translation)
                    .distance(Vec3::from(after.position))
                    < 1.0e-5
            );
            assert!(
                (rotation * Vec3::from(before.normal)).distance(Vec3::from(after.normal)) < 1.0e-3
            );
        }

        let camera = Camera {
            distance: 3.0,
            ..Camera::default()
        };
        let canvas = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(640.0, 480.0),
        );
        let midpoint = transformed.points[0]
            .position
            .lerp(transformed.points[1].position, 0.5);
        let width = transformed
            .brush
            .normal
            .cross(transformed.brush.tangent)
            .normalize();
        let pick_world = midpoint + width * stroke_extent(&transformed) * 0.9;
        let pick_screen = crate::gizmo::project(&camera, canvas, pick_world)
            .unwrap()
            .0;
        let document = crate::document::Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![transformed.clone()],
            ..Default::default()
        };
        assert_eq!(
            crate::gizmo::pick(&document, &camera, canvas, pick_screen),
            Some(transformed.id),
            "profile-aware extent keeps the transformed stroke pickable near its outer bound"
        );
    }

    #[test]
    fn profiled_tube_vertex_limit_rejects_before_large_mesh_allocation() {
        let mut stroke = profiled_mark(crate::document::TubeProfile::Star, &[Vec3::ZERO, Vec3::X]);
        stroke.points = (0..16_667)
            .map(|index| Point {
                tilt: None,
                position: Vec3::X * (index as f32 * 0.001),
                pressure: 1.0,
                time_ms: 0,
            })
            .collect();
        assert!(profile_tube_vertex_estimate(&stroke) > MAX_PROFILE_TUBE_VERTICES);
        assert!(profile_tube_mesh(&stroke).is_empty());
        assert!(stroke_mesh(&stroke).is_empty());
    }

    #[test]
    fn rake_and_bristle_fan_strands_stay_finite_for_dots_bends_and_reversals() {
        let paths = [
            vec![Vec3::ZERO],
            vec![Vec3::ZERO, Vec3::ZERO, Vec3::ZERO],
            vec![
                Vec3::ZERO,
                Vec3::X,
                Vec3::X + Vec3::Y * 0.15,
                Vec3::X * 1.8 + Vec3::Y * 0.15,
            ],
            vec![
                Vec3::ZERO,
                Vec3::X,
                Vec3::X,
                Vec3::ZERO,
                -Vec3::X + Vec3::Y * 0.2,
            ],
        ];

        for name in ["Rake", "Bristle Fan"] {
            for path in &paths {
                let stroke = strand_mark(name, path);
                assert!(stroke.brush.strands.enabled, "{name} uses strands");
                assert!(!stroke.brush.dabs.enabled, "{name} is one strand stroke");
                assert!(
                    crate::dab_placement::sample(&stroke).unwrap().is_empty(),
                    "{name} does not emit separate dab records"
                );

                let frame_count = brush_frames(&stroke).len();
                let expected_vertices = stroke.brush.strands.vertex_estimate(frame_count);
                let mesh = strand_mesh(&stroke);
                assert_eq!(mesh.len(), expected_vertices, "{name} lane mesh size");
                assert_same_mesh(&stroke_mesh(&stroke), &mesh, name);
                valid(&mesh);
            }
        }
    }

    #[test]
    fn strand_presets_emit_their_expected_number_of_distinct_lanes() {
        for (name, expected_count) in [("Rake", 14), ("Bristle Fan", 6)] {
            let stroke = strand_mark(name, &[Vec3::ZERO, Vec3::X]);
            let count = usize::from(stroke.brush.strands.count);
            assert_eq!(count, expected_count, "{name} strand count");
            let mesh = stroke_mesh(&stroke);
            let per_strand = mesh.len() / count;
            assert_eq!(mesh.len() % count, 0);

            let centers: Vec<_> = mesh.chunks_exact(per_strand).map(strand_centroid).collect();
            assert_eq!(centers.len(), expected_count);
            for (index, center) in centers.iter().enumerate() {
                for other in &centers[index + 1..] {
                    assert!(
                        center.distance(*other) > 0.01,
                        "{name} lanes should occupy distinct positions"
                    );
                }
            }
        }
    }

    #[test]
    fn strand_mesh_uses_manual_frames_when_tilt_is_missing() {
        let positions = [
            Vec3::ZERO,
            Vec3::X,
            Vec3::X + Vec3::Y * 0.12,
            Vec3::new(0.45, 0.12, 0.0),
        ];
        for name in ["Rake", "Bristle Fan"] {
            let tilted_preset = strand_mark(name, &positions);
            assert!(tilted_preset.brush.dynamics.tilt_orientation);
            assert!(tilted_preset
                .points
                .iter()
                .all(|point| point.tilt.is_none()));

            let mut manual = tilted_preset.clone();
            manual.brush.dynamics.tilt_orientation = false;
            assert_same_mesh(
                &strand_mesh(&tilted_preset),
                &strand_mesh(&manual),
                &format!("missing tilt for {name}"),
            );
        }
    }

    #[test]
    fn strand_mesh_transforms_with_its_captured_frame() {
        let rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.35, -0.7, 1.1);
        let translation = Vec3::ZERO;
        let positions = [
            Vec3::ZERO,
            Vec3::X,
            Vec3::X + Vec3::Y * 0.15,
            Vec3::new(1.8, 0.35, 0.0),
        ];

        for name in ["Rake", "Bristle Fan"] {
            let mut stroke = strand_mark(name, &positions);
            let before = strand_mesh(&stroke);
            for point in &mut stroke.points {
                point.position = rotation * point.position + translation;
            }
            stroke.brush.normal = rotation * stroke.brush.normal;
            stroke.brush.tangent = rotation * stroke.brush.tangent;
            let after = strand_mesh(&stroke);
            assert_eq!(before.len(), after.len(), "{name} transformed mesh size");
            for (source, transformed) in before.iter().zip(&after) {
                assert!(
                    (rotation * Vec3::from(source.position) + translation)
                        .distance(Vec3::from(transformed.position))
                        < 1e-5,
                    "{name} position follows its captured frame"
                );
                assert!(
                    (rotation * Vec3::from(source.normal))
                        .distance(Vec3::from(transformed.normal))
                        < 1e-3,
                    "{name} normal follows its captured frame: source={:?}, expected={:?}, actual={:?}",
                    Vec3::from(source.normal),
                    rotation * Vec3::from(source.normal),
                    Vec3::from(transformed.normal),
                );
            }
        }
    }

    #[test]
    fn over_budget_strand_stroke_is_rejected_before_mesh_allocation() {
        let mut stroke = strand_mark("Rake", &[Vec3::ZERO, Vec3::X]);
        stroke.brush.strands.count = 16;
        stroke.points = (0..695)
            .map(|index| Point {
                tilt: None,
                position: Vec3::X * (index as f32 * 0.01),
                pressure: 1.0,
                time_ms: 0,
            })
            .collect();
        let estimate = stroke.brush.strands.vertex_estimate(stroke.points.len());
        assert!(estimate > MAX_STRAND_VERTICES);
        assert!(strand_mesh(&stroke).is_empty());
        assert!(stroke_mesh(&stroke).is_empty());
    }
}

#[cfg(test)]
mod dynamics_tests {
    use super::*;
    use crate::document::{Brush, Point};

    fn line(kind: BrushKind, xs: &[f32]) -> Stroke {
        Stroke {
            gradient: None,
            curve: None,
            material: Default::default(),
            id: 100,
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            color: [0.5; 3],
            radius: 0.1,
            brush: Brush {
                kind,
                ..Brush::default()
            },
            points: xs
                .iter()
                .map(|&x| Point {
                    position: Vec3::X * x,
                    pressure: 0.0,
                    tilt: None,

                    time_ms: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn chisel_pressure_width_and_thickness_are_independent() {
        let mut stroke = line(BrushKind::Chisel, &[0.0, 1.0]);
        stroke.brush.dynamics.thickness.enabled = false;
        let frames = brush_frames(&stroke);
        assert!((frames[0].radius - 0.012).abs() < 1e-6);
        assert!((frames[0].half_thickness - 0.02).abs() < 1e-6);
        stroke.brush.dynamics.width.enabled = false;
        stroke.brush.dynamics.thickness.enabled = true;
        let frames = brush_frames(&stroke);
        assert!((frames[0].radius - 0.1).abs() < 1e-6);
        assert!((frames[0].half_thickness - 0.0024).abs() < 1e-6);
    }

    #[test]
    fn taper_adds_distance_landmarks_and_matches_denser_sampling() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            let mut sparse = line(kind, &[0.0, 1.0]);
            sparse.brush.dynamics.width.enabled = false;
            sparse.brush.dynamics.thickness.enabled = false;
            sparse.brush.dynamics.taper_start = 0.25;
            sparse.brush.dynamics.taper_end = 0.25;
            let prepared = Stroke {
                curve: None,
                points: taper_points(&sparse),
                ..sparse.clone()
            };
            let mut dense = sparse.clone();
            dense.points = (0..=20)
                .map(|i| Point {
                    position: Vec3::X * i as f32 * 0.05,
                    pressure: 0.0,
                    tilt: None,

                    time_ms: 0,
                })
                .collect();
            let sparse_frames = brush_frames(&prepared);
            let dense_frames = brush_frames(&dense);
            for frame in &sparse_frames {
                let matching = dense_frames
                    .iter()
                    .find(|f| f.center.distance(frame.center) < 1e-6)
                    .unwrap();
                assert!((frame.radius - matching.radius).abs() < 1e-6);
                assert!((frame.half_thickness - matching.half_thickness).abs() < 1e-6);
            }
            assert!(sparse_frames.iter().any(|f| (f.radius - 0.1).abs() < 1e-6));
            let mesh = stroke_mesh(&sparse);
            assert!(mesh
                .iter()
                .all(|v| Vec3::from(v.position).is_finite() && Vec3::from(v.normal).is_finite()));
            assert!(mesh.iter().map(|v| v.position[1].abs()).fold(0.0, f32::max) > 0.099);
        }
    }

    #[test]
    fn overlapping_tapers_keep_short_strokes_and_dots_finite() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            for xs in [vec![0.0], vec![0.0, 0.001], vec![0.0, 0.1]] {
                let mut stroke = line(kind, &xs);
                stroke.brush.dynamics.taper_start = 1.0;
                stroke.brush.dynamics.taper_end = 1.0;
                let mesh = stroke_mesh(&stroke);
                assert!(!mesh.is_empty());
                assert!(mesh.iter().all(
                    |v| Vec3::from(v.position).is_finite() && Vec3::from(v.normal).is_finite()
                ));
                if xs.len() == 1 {
                    assert!((brush_frames(&stroke)[0].radius - 0.012).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn tilt_uses_captured_directions_and_falls_back_to_manual_frame() {
        let mut stroke = line(BrushKind::Chisel, &[0.0, 0.5, 1.0, 1.5]);
        stroke.brush.dynamics.tilt_orientation = true;
        stroke.points[0].tilt = Some(Vec3::Z);
        stroke.points[1].tilt = None;
        stroke.points[2].tilt = Some(Vec3::X); // Parallel to the path is undefined.
        stroke.points[3].tilt = Some(-Vec3::Y);
        let frames = brush_frames(&stroke);
        assert!(frames[0].width.abs_diff_eq(Vec3::Z, 1e-6));
        assert!(frames[1].width.abs_diff_eq(Vec3::Y, 1e-6));
        assert!(frames[2].width.abs_diff_eq(Vec3::Y, 1e-6));
        assert!(frames[3].width.abs_diff_eq(Vec3::Y, 1e-6));
        assert!(frames.windows(2).all(|f| f[0].width.dot(f[1].width) >= 0.0));
        stroke.brush.dynamics.tilt_orientation = false;
        let disabled = stroke_mesh(&stroke);
        for p in &mut stroke.points {
            p.tilt = None;
        }
        let manual = stroke_mesh(&stroke);
        assert_eq!(
            disabled.iter().map(|v| v.position).collect::<Vec<_>>(),
            manual.iter().map(|v| v.position).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod dynamics_boundary_tests {
    use super::*;
    use crate::document::{Brush, Point};
    #[test]
    fn zero_pressure_dimensions_and_tilt_azimuth_wrap_stay_finite() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            let mut stroke = Stroke {
                gradient: None,
                curve: None,
                material: Default::default(),
                id: 101,
                point_thickness: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: [0.5; 3],
                radius: 0.1,
                brush: Brush {
                    kind,
                    ..Brush::default()
                },
                points: (0..4)
                    .map(|i| Point {
                        position: Vec3::X * i as f32,
                        pressure: 0.0,
                        tilt: Some(if i % 2 == 0 {
                            Vec3::new(0.0, 0.001, -1.0).normalize()
                        } else {
                            Vec3::new(0.0, -0.001, 1.0).normalize()
                        }),

                        time_ms: 0,
                    })
                    .collect(),
            };
            stroke.brush.dynamics.width.minimum = 0.0;
            stroke.brush.dynamics.thickness.minimum = 0.0;
            stroke.brush.dynamics.tilt_orientation = true;
            stroke.brush.dynamics.taper_start = 0.4;
            stroke.brush.dynamics.taper_end = 0.4;
            let frames = brush_frames(&stroke);
            assert!(frames
                .iter()
                .all(|f| f.radius == 0.0 && f.half_thickness == 0.0));
            assert!(frames.windows(2).all(|f| f[0].width.dot(f[1].width) > 0.99));
            let mesh = stroke_mesh(&stroke);
            assert!(mesh
                .iter()
                .all(|v| Vec3::from(v.position).is_finite() && Vec3::from(v.normal).is_finite()));
            for point in &mut stroke.points {
                point.pressure = 0.01;
            }
            let mesh = stroke_mesh(&stroke);
            assert!(!mesh.is_empty());
            assert!(mesh
                .iter()
                .all(|v| Vec3::from(v.position).is_finite() && Vec3::from(v.normal).is_finite()));
        }
    }
}

#[cfg(test)]
mod seam_dynamics_tests {
    use super::*;
    use crate::{
        document::{Brush, Point},
        surface::{Cylinder, Surface, SurfaceStabilizer},
    };
    #[test]
    fn tapered_tilted_marks_cross_cylinder_seam_and_finish_on_surface() {
        let cylinder = Cylinder::default();
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            let mut filter = SurfaceStabilizer::default();
            filter.start(Surface::Cylinder(cylinder));
            let mut points = Vec::new();
            for angle in [std::f32::consts::PI - 0.12, -std::f32::consts::PI + 0.12] {
                let position = cylinder.point(angle, 0.2);
                assert!(!filter.discontinuity(position));
                points.extend(filter.push(
                    Point {
                        position,
                        pressure: 0.75,
                        tilt: Some(Vec3::Y),

                        time_ms: 0,
                    },
                    0.05,
                ));
            }
            points.extend(filter.finish());
            let target = cylinder.point(-std::f32::consts::PI + 0.12, 0.2);
            assert!(points.last().unwrap().position.distance(target) < 1e-5);
            assert!(points
                .windows(2)
                .all(|p| p[0].position.distance(p[1].position) < 0.036));
            assert!(points.iter().all(|p| {
                let local = cylinder.to_local(p.position);
                (local.x.hypot(local.z) - cylinder.radius).abs() < 1e-5
                    && p.pressure == 0.75
                    && p.tilt == Some(Vec3::Y)
            }));
            let mut stroke = Stroke {
                gradient: None,
                curve: None,
                material: Default::default(),
                id: 1,
                point_thickness: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: [0.5; 3],
                radius: 0.1,
                brush: Brush {
                    kind,
                    ..Brush::default()
                },
                points,
            };
            stroke.brush.dynamics.tilt_orientation = true;
            stroke.brush.dynamics.taper_start = 0.08;
            stroke.brush.dynamics.taper_end = 0.1;
            let prepared = Stroke {
                curve: None,
                points: taper_points(&stroke),
                ..stroke.clone()
            };
            let frames = brush_frames(&prepared);
            let maximum = frames.iter().map(|f| f.radius).fold(0.0, f32::max);
            assert!(maximum > 0.07);
            assert!(frames.first().unwrap().radius < maximum * 0.02);
            assert!(frames.last().unwrap().radius < maximum * 0.02);
            let mesh = stroke_mesh(&stroke);
            assert!(!mesh.is_empty());
            assert!(mesh
                .iter()
                .all(|v| Vec3::from(v.position).is_finite() && Vec3::from(v.normal).is_finite()));
        }
    }
}

#[cfg(test)]
mod texture_uv_tests {
    use super::*;
    use crate::{
        document::{Brush, Point},
        texture::TextureKind,
    };

    fn mark(kind: BrushKind, positions: Vec<Vec3>) -> Stroke {
        let mut brush = Brush {
            kind,
            ..Brush::default()
        };
        brush.texture.kind = TextureKind::Grain;
        brush.texture.scale = 0.2;
        Stroke {
            gradient: None,
            curve: None,
            material: Default::default(),
            id: 1,
            point_thickness: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            color: [0.5; 3],
            radius: 0.1,
            brush,
            points: positions
                .into_iter()
                .map(|position| Point {
                    position,
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn side_uvs_use_path_distance_and_duplicate_perimeter_seams() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            for xs in [vec![0.0, 0.5, 1.0], vec![0.0, 0.25, 0.5, 0.75, 1.0]] {
                let stroke = mark(kind, xs.into_iter().map(|x| Vec3::X * x).collect());
                let mesh = stroke_mesh(&stroke);
                let per_segment = match kind {
                    BrushKind::Round => 48,
                    BrushKind::Ribbon => 6,
                    BrushKind::Chisel => 24,
                };
                let sides = &mesh[..(stroke.points.len() - 1) * per_segment];
                for v in sides {
                    assert!((v.uv[0] - v.position[0] / stroke.brush.texture.scale).abs() < 1e-6);
                    assert!((0.0..=1.0).contains(&v.uv[1]));
                }
                assert!(sides.iter().any(|v| v.uv[1] == 0.0));
                assert!(sides.iter().any(|v| v.uv[1] == 1.0));
                if kind == BrushKind::Round {
                    for triangle in sides.as_chunks::<3>().0 {
                        let min = triangle
                            .iter()
                            .map(|v| v.uv[1])
                            .fold(f32::INFINITY, f32::min);
                        let max = triangle
                            .iter()
                            .map(|v| v.uv[1])
                            .fold(f32::NEG_INFINITY, f32::max);
                        assert!(
                            max - min <= 0.125 + 1e-6,
                            "ring seam must not span the tile"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn opacity_mapping_and_soft_edges_are_baked_into_stroke_mesh_alpha() {
        let mut stroke = mark(BrushKind::Round, vec![Vec3::ZERO, Vec3::X]);
        stroke.brush.opacity = 0.4;
        stroke.brush.dynamics.opacity = crate::document::PressureMapping {
            enabled: true,
            minimum: 0.2,
            curve: 1.0,
        };
        stroke.points[0].pressure = 0.0;
        stroke.points[1].pressure = 1.0;
        let mesh = stroke_mesh(&stroke);
        assert!(mesh
            .iter()
            .any(|vertex| (vertex.opacity - 0.08).abs() < 1e-5));
        assert!(mesh
            .iter()
            .any(|vertex| (vertex.opacity - 0.4).abs() < 1e-5));

        stroke.brush.dynamics.opacity.enabled = false;
        stroke.brush.softness = 1.0;
        let mesh = stroke_mesh(&stroke);
        assert!(mesh.iter().any(|vertex| vertex.opacity < 0.05));
        assert!(mesh
            .iter()
            .any(|vertex| (vertex.opacity - 0.4).abs() < 1e-5));

        let mut ribbon = mark(BrushKind::Ribbon, vec![Vec3::ZERO, Vec3::X]);
        ribbon.brush.softness = 1.0;
        let mesh = stroke_mesh(&ribbon);
        assert_eq!(
            mesh.len(),
            24,
            "soft ribbon subdivides its width into four bands"
        );
        assert!(mesh.iter().any(|vertex| vertex.opacity < 0.1));
        assert!(mesh.iter().any(|vertex| vertex.opacity > 0.9));
    }

    #[test]
    fn frozen_uvs_and_textured_geometry_survive_rigid_transform_and_uniform_scale() {
        let rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.6, 1.1);
        let translation = Vec3::new(1.0, -0.4, 0.2);
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            for positions in [
                vec![Vec3::ZERO],
                vec![Vec3::ZERO, Vec3::X, Vec3::new(1.4, 0.6, 0.2)],
            ] {
                let stroke = mark(kind, positions);
                let before = stroke_mesh(&stroke);
                let transformed = crate::editing::transformed(
                    &vec![(0, stroke)],
                    Vec3::ZERO,
                    translation,
                    rotation,
                    1.6,
                )
                .unwrap();
                let after = stroke_mesh(&transformed[0].1);
                assert_eq!(before.len(), after.len());
                for (a, b) in before.iter().zip(&after) {
                    assert!(Vec2::from(a.uv).distance(Vec2::from(b.uv)) < 1e-5);
                    assert!(
                        (rotation * (Vec3::from(a.position) * 1.6) + translation)
                            .distance(Vec3::from(b.position))
                            < 1e-5
                    );
                }
            }
        }
    }

    #[test]
    fn cylinder_seam_uvs_have_continuous_distance_without_angular_reset() {
        use crate::surface::{Cylinder, Surface, SurfaceStabilizer};
        let cylinder = Cylinder::default();
        let mut filter = SurfaceStabilizer::default();
        filter.start(Surface::Cylinder(cylinder));
        let mut points = Vec::new();
        for angle in [std::f32::consts::PI - 0.2, -std::f32::consts::PI + 0.2] {
            points.extend(filter.push(
                Point {
                    position: cylinder.point(angle, 0.2),
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                },
                0.0,
            ));
        }
        points.extend(filter.finish());
        let mut stroke = mark(BrushKind::Ribbon, Vec::new());
        stroke.points = points;
        let frames = brush_frames(&stroke);
        assert!(frames
            .windows(2)
            .all(|f| f[1].u >= f[0].u && f[1].u - f[0].u < 0.18));
        assert!((frames.last().unwrap().u - 2.0).abs() < 0.001);
        for triangle in stroke_mesh(&stroke).as_chunks::<3>().0 {
            let min = triangle
                .iter()
                .map(|v| v.uv[0])
                .fold(f32::INFINITY, f32::min);
            let max = triangle
                .iter()
                .map(|v| v.uv[0])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(max - min < 0.18);
            assert!(triangle.iter().all(|v| Vec2::from(v.uv).is_finite()));
        }
    }
}
