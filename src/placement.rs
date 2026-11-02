//! Exact, read-only artwork surface picking for drawing-plane placement.
use crate::{
    document::Document,
    geometry::{self, DrawingPlane},
    texture,
};
use glam::{Mat3, Quat, Vec2, Vec3};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct SurfaceHit {
    pub id: u64,
    pub position: Vec3,
    pub normal: Vec3,
    pub tangent: Vec3,
}
impl SurfaceHit {
    pub fn plane(self, flip: bool, offset: f32) -> DrawingPlane {
        let n = if flip { -self.normal } else { self.normal };
        let u = self.tangent;
        let v = n.cross(u).normalize();
        let rotation = Quat::from_mat3(&Mat3::from_cols(u, v, n));
        let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
        DrawingPlane {
            anchor: self.position,
            rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
            depth: offset,
        }
    }
}
/// Two-sided ray/triangle intersection in double precision, returning distance and barycentrics.
fn triangle(
    origin: Vec3,
    direction: Vec3,
    vertices: &[geometry::Vertex; 3],
) -> Option<(f64, f32, f32)> {
    let a = Vec3::from(vertices[0].position).as_dvec3();
    let e1 = Vec3::from(vertices[1].position).as_dvec3() - a;
    let e2 = Vec3::from(vertices[2].position).as_dvec3() - a;
    let d = direction.as_dvec3();
    let p = d.cross(e2);
    let determinant = e1.dot(p);
    if determinant.abs() < 1e-10 * e1.length() * e2.length() * d.length() || determinant == 0.0 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let tvec = origin.as_dvec3() - a;
    let u = tvec.dot(p) * inverse;
    let q = tvec.cross(e1);
    let v = d.dot(q) * inverse;
    if u < -1e-7 || v < -1e-7 || u + v > 1.0 + 1e-7 {
        return None;
    }
    let t = e2.dot(q) * inverse;
    (t >= 0.0 && t.is_finite()).then_some((t, u as f32, v as f32))
}
pub fn pick(doc: &Document, origin: Vec3, direction: Vec3) -> Option<SurfaceHit> {
    if !origin.is_finite() || !direction.is_finite() || direction.length_squared() < 1e-12 {
        return None;
    }
    let mut best: Option<(f64, SurfaceHit)> = None;
    let mut masks = HashMap::new();
    let mut tip_images = HashMap::new();
    for stroke in doc.strokes.iter().filter(|s| doc.visible(s)) {
        let vertices = geometry::stroke_mesh(stroke);
        let tip_image = if stroke.brush.tip_asset_id != 0 {
            let image = tip_images
                .entry(stroke.brush.tip_asset_id)
                .or_insert_with(|| {
                    doc.brush_tip_assets
                        .iter()
                        .find(|asset| asset.id == stroke.brush.tip_asset_id)
                        .and_then(|asset| asset.decoded().ok())
                });
            image.as_ref()
        } else {
            None
        };
        for face in vertices.as_chunks::<3>().0 {
            let Some((t, u, v)) = triangle(origin, direction, face) else {
                continue;
            };
            if best.is_some_and(|(distance, _)| t > distance) {
                continue;
            }
            let weights = [1.0 - u - v, u, v];
            if stroke.brush.flow.enabled {
                let opacity: f32 = face
                    .iter()
                    .zip(weights)
                    .map(|(vertex, weight)| vertex.opacity * weight)
                    .sum();
                if opacity <= 0.001 {
                    continue;
                }
                if tip_image.is_none()
                    && stroke.brush.tip_asset_id == 0
                    && texture::key(stroke.brush.texture).0 != texture::TextureKind::Solid
                {
                    let uv = face
                        .iter()
                        .zip(weights)
                        .map(|(vertex, weight)| Vec2::from(vertex.uv) * weight)
                        .sum::<Vec2>();
                    let x = ((uv.x.rem_euclid(1.0) * texture::SIZE as f32) as u32)
                        .min(texture::SIZE - 1);
                    let y = ((uv.y.rem_euclid(1.0) * texture::SIZE as f32) as u32)
                        .min(texture::SIZE - 1);
                    let mask = masks
                        .entry(texture::key(stroke.brush.texture))
                        .or_insert_with(|| texture::rgba(stroke.brush.texture));
                    if mask[((y * texture::SIZE + x) * 4 + 3) as usize] < 128 {
                        continue;
                    }
                }
            } else if let Some(tip) = tip_image {
                let uv = face
                    .iter()
                    .zip(weights)
                    .map(|(vertex, weight)| Vec2::from(vertex.uv) * weight)
                    .sum::<Vec2>();
                let x = (uv.x.clamp(0.0, 1.0) * tip.size[0] as f32)
                    .floor()
                    .min(tip.size[0].saturating_sub(1) as f32) as usize;
                let y = (uv.y.clamp(0.0, 1.0) * tip.size[1] as f32)
                    .floor()
                    .min(tip.size[1].saturating_sub(1) as f32) as usize;
                if tip.pixels[(y * tip.size[0] as usize + x) * 4 + 3] == 0 {
                    continue;
                }
            } else if stroke.brush.tip_asset_id != 0 {
                continue;
            } else if texture::key(stroke.brush.texture).0 != texture::TextureKind::Solid {
                let uv = face
                    .iter()
                    .zip(weights)
                    .map(|(p, w)| Vec2::from(p.uv) * w)
                    .sum::<Vec2>();
                let mask = masks
                    .entry(texture::key(stroke.brush.texture))
                    .or_insert_with(|| texture::rgba(stroke.brush.texture));
                let x =
                    ((uv.x.rem_euclid(1.0) * texture::SIZE as f32) as u32).min(texture::SIZE - 1);
                let y =
                    ((uv.y.rem_euclid(1.0) * texture::SIZE as f32) as u32).min(texture::SIZE - 1);
                if mask[((y * texture::SIZE + x) * 4 + 3) as usize] < 128 {
                    continue;
                }
            }
            let face_normal = (Vec3::from(face[1].position) - Vec3::from(face[0].position))
                .cross(Vec3::from(face[2].position) - Vec3::from(face[0].position));
            let normal = face
                .iter()
                .zip(weights)
                .map(|(p, w)| Vec3::from(p.normal) * w)
                .sum::<Vec3>();
            let Some(mut normal) = normal
                .try_normalize()
                .or_else(|| face_normal.try_normalize())
            else {
                continue;
            };
            if normal.dot(direction) > 0.0 {
                normal = -normal;
            }
            let edge = Vec3::from(face[1].position) - Vec3::from(face[0].position);
            let project_axis = |axis: Vec3| {
                let axis = axis - normal * axis.dot(normal);
                (axis.length_squared() > 1e-10).then(|| axis.normalize())
            };
            let tangent = project_axis(stroke.brush.tangent)
                .or_else(|| project_axis(edge))
                .unwrap_or_else(|| normal.any_orthonormal_vector());
            let position = (origin.as_dvec3() + direction.as_dvec3() * t).as_vec3();
            if position.is_finite() {
                best = Some((
                    t,
                    SurfaceHit {
                        id: stroke.id,
                        position,
                        normal,
                        tangent,
                    },
                ));
            }
        }
    }
    best.map(|(_, hit)| hit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, BrushKind, PlanarFill, Point, Stroke};
    fn panel(id: u64, z: f32) -> Stroke {
        Stroke {
            gradient: None,
            curve: None,
            material: Default::default(),
            id,
            point_thickness: None,
            modifiers: None,
            fill: Some(PlanarFill { offset: 0.0 }),
            group_id: None,
            layer_id: 1,
            color: [0.5; 3],
            radius: 0.1,
            brush: Brush::default(),
            points: [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
                .into_iter()
                .map(|p| Point {
                    position: Vec3::new(p[0], p[1], z),
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                })
                .collect(),
        }
    }
    #[test]
    fn nearest_visible_locked_surface_and_rotated_plane_frames() {
        let mut doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![panel(1, 0.0), panel(2, 1.0)],
            ..Default::default()
        };
        doc.layers[0].locked = true;
        let hit = pick(&doc, Vec3::new(0.2, 0.3, 5.0), -Vec3::Z).unwrap();
        assert_eq!(hit.id, 2);
        let plane = hit.plane(false, 0.02);
        assert!(plane.basis().2.dot(Vec3::Z) > 0.99999);
        assert!((plane.origin().z - 1.02).abs() < 1e-5);
        assert!(hit.plane(true, 0.02).basis().2.dot(Vec3::Z) < -0.99999);
        let back = pick(&doc, -Vec3::Z * 5.0, Vec3::Z).unwrap();
        assert_eq!(back.id, 1);
        assert!(back.normal.dot(-Vec3::Z) > 0.99999);
        let mut coplanar = doc.clone();
        for point in &mut coplanar.strokes[1].points {
            point.position.z = 0.0;
        }
        assert_eq!(pick(&coplanar, Vec3::Z * 5.0, -Vec3::Z).unwrap().id, 2);
        doc.layers[0].visible = false;
        assert!(pick(&doc, Vec3::Z * 5.0, -Vec3::Z).is_none());
        doc.layers[0].visible = true;
        let rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.7, 0.2);
        let snapshot = vec![(0, panel(1, 0.0))];
        let rotated =
            crate::editing::transformed(&snapshot, Vec3::ZERO, Vec3::X * 2.0, rotation, 1.0)
                .unwrap();
        doc.strokes = vec![rotated[0].1.clone()];
        let normal = rotation * Vec3::Z;
        let point = Vec3::X * 2.0;
        let hit = pick(&doc, point + normal * 4.0, -normal).unwrap();
        let plane = hit.plane(false, 0.0);
        assert!(plane.basis().2.dot(normal) > 0.99999);
        assert!(plane.origin().distance(point) < 1e-5);
        assert!(plane.basis().0.dot(plane.basis().2).abs() < 1e-5);
    }
    #[test]
    fn round_and_chisel_use_actual_surfaces_without_pick_halo() {
        let mut doc = Document::default();
        for kind in [BrushKind::Round, BrushKind::Chisel] {
            let mut mark = panel(1, 0.0);
            mark.fill = None;
            mark.brush.kind = kind;
            mark.brush.thickness = 1.0;
            mark.points = vec![
                Point {
                    position: Vec3::new(-1.0, 0.0, 0.0),
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                },
                Point {
                    position: Vec3::new(1.0, 0.0, 0.0),
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                },
            ];
            doc.strokes = vec![mark];
            let hit = pick(&doc, Vec3::Z * 5.0, -Vec3::Z).unwrap();
            assert!((hit.position.z - 0.1).abs() < 0.001);
            assert!(hit.normal.dot(Vec3::Z) > 0.99);
            let cap = pick(&doc, Vec3::X * 5.0, -Vec3::X).unwrap();
            let cap_plane = cap.plane(false, 0.0);
            assert!(cap_plane.basis().2.dot(Vec3::X) > 0.99);
            assert!(cap_plane.basis().0.dot(cap_plane.basis().2).abs() < 1e-5);
            assert!(pick(&doc, Vec3::new(0.0, 0.3, 5.0), -Vec3::Z).is_none());
        }
    }
    #[test]
    fn textured_holes_pick_the_visible_surface_behind_them() {
        let mut front = panel(1, 0.0);
        front.fill = None;
        front.brush.kind = BrushKind::Ribbon;
        front.radius = 0.5;
        front.points = vec![
            Point {
                position: Vec3::new(-1.0, 0.0, 0.0),
                pressure: 1.0,
                tilt: None,

                time_ms: 0,
            },
            Point {
                position: Vec3::new(1.0, 0.0, 0.0),
                pressure: 1.0,
                tilt: None,

                time_ms: 0,
            },
        ];
        front.brush.texture.kind = texture::TextureKind::Pattern;
        front.brush.texture.strength = 0.5;
        let doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![front, panel(2, -0.5)],
            ..Default::default()
        };
        let mut counts = [0; 2];
        for x in 0..12 {
            for y in 0..12 {
                let hit = pick(
                    &doc,
                    Vec3::new(-0.8 + x as f32 * 0.13, -0.4 + y as f32 * 0.065, 4.0),
                    -Vec3::Z,
                )
                .unwrap();
                counts[(hit.id - 1) as usize] += 1;
            }
        }
        assert!(counts[0] > 10 && counts[1] > 10, "{counts:?}");
    }
}
