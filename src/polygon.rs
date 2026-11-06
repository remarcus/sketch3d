//! Bounded simple planar polygons: validation, ear clipping, and exact interior picking.
use crate::document::Stroke;
use glam::{DVec2, Vec3};
use lyon_tessellation::{
    geometry_builder::{simple_builder, VertexBuffers},
    math::point,
    path::Path,
    FillOptions, FillRule, FillTessellator,
};
pub const MAX_CORNERS: usize = 256;
pub const MAX_HOLES: usize = 64;
pub const MAX_TOTAL_CONTOUR_POINTS: usize = 4096;
const MAX_FILL_MESH_VERTICES: usize = 8192;
const MAX_FILL_MESH_INDICES: usize = 24576;
fn cross(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    (b - a).perp_dot(c - a)
}
fn on_segment(p: DVec2, a: DVec2, b: DVec2, eps: f64) -> bool {
    cross(a, b, p).abs() <= eps && (p - a).dot(p - b) <= eps
}
fn intersects(a: DVec2, b: DVec2, c: DVec2, d: DVec2, eps: f64) -> bool {
    let [ab_c, ab_d, cd_a, cd_b] = [
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    ];
    (ab_c * ab_d < -eps * eps && cd_a * cd_b < -eps * eps)
        || on_segment(c, a, b, eps)
        || on_segment(d, a, b, eps)
        || on_segment(a, c, d, eps)
        || on_segment(b, c, d, eps)
}
fn projected(stroke: &Stroke) -> Vec<DVec2> {
    if let Some(surface) = stroke.fill.as_ref().and_then(|fill| fill.cylinder) {
        return stroke
            .points
            .iter()
            .map(|point| {
                surface
                    .chart_point(point.position)
                    .map(|point| DVec2::new(point.x as f64, point.y as f64))
                    .unwrap_or(DVec2::splat(f64::NAN))
            })
            .collect();
    }
    let origin = stroke.points[0].position.as_dvec3();
    let u = stroke.brush.tangent.as_dvec3();
    let v = stroke.brush.normal.as_dvec3().cross(u);
    stroke
        .points
        .iter()
        .map(|p| {
            let d = p.position.as_dvec3() - origin;
            DVec2::new(d.dot(u), d.dot(v))
        })
        .collect()
}

fn bounds(points: &[DVec2]) -> (DVec2, DVec2) {
    let mut min = DVec2::splat(f64::INFINITY);
    let mut max = DVec2::splat(f64::NEG_INFINITY);
    for point in points {
        min = min.min(*point);
        max = max.max(*point);
    }
    (min, max)
}

fn normalized_uv(point: DVec2, min: DVec2, max: DVec2) -> glam::Vec2 {
    let size = (max - min).max(DVec2::splat(1e-8));
    ((point - min) / size).as_vec2()
}

fn point_in_ring(point: DVec2, ring: &[DVec2]) -> bool {
    let mut inside = false;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

fn projected_holes(stroke: &Stroke) -> Vec<Vec<DVec2>> {
    if let Some(surface) = stroke.fill.as_ref().and_then(|fill| fill.cylinder) {
        return stroke
            .fill
            .as_ref()
            .map(|fill| {
                fill.holes
                    .iter()
                    .map(|hole| {
                        hole.iter()
                            .map(|point| {
                                surface
                                    .chart_point(*point)
                                    .map(|point| DVec2::new(point.x as f64, point.y as f64))
                                    .unwrap_or(DVec2::splat(f64::NAN))
                            })
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let origin = stroke.points[0].position;
    let u = stroke.brush.tangent.as_dvec3();
    let v = stroke.brush.normal.as_dvec3().cross(u);
    stroke
        .fill
        .as_ref()
        .map(|fill| {
            fill.holes
                .iter()
                .map(|hole| {
                    hole.iter()
                        .map(|point| {
                            let delta = (*point - origin).as_dvec3();
                            DVec2::new(delta.dot(u), delta.dot(v))
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn validate_simple_ring(points: &[DVec2], eps: f64) -> Result<(), String> {
    if !(3..=MAX_CORNERS).contains(&points.len()) {
        return Err(format!("A contour needs 3–{MAX_CORNERS} corners"));
    }
    for i in 0..points.len() {
        let next = (i + 1) % points.len();
        if points[i].distance_squared(points[next]) <= eps {
            return Err("Remove duplicate or extremely close contour corners".into());
        }
        for j in i + 1..points.len() {
            let end = (j + 1) % points.len();
            if next == j || end == i {
                continue;
            }
            if intersects(points[i], points[next], points[j], points[end], eps) {
                return Err("Contour intersects itself; move or remove corners".into());
            }
        }
    }
    let area: f64 = (0..points.len())
        .map(|i| points[i].perp_dot(points[(i + 1) % points.len()]))
        .sum();
    if area.abs() <= eps {
        return Err("Contour has no usable area".into());
    }
    Ok(())
}

fn lyon_mesh(contours: &[Vec<DVec2>]) -> Result<(Vec<DVec2>, Vec<[usize; 3]>), String> {
    let mut builder = Path::builder();
    for contour in contours {
        let Some(first) = contour.first() else {
            return Err("Fill contains an empty contour".into());
        };
        builder.begin(point(first.x as f32, first.y as f32));
        for vertex in &contour[1..] {
            builder.line_to(point(vertex.x as f32, vertex.y as f32));
        }
        builder.end(true);
    }
    let path = builder.build();
    let mut buffers: VertexBuffers<lyon_tessellation::math::Point, u16> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            &path,
            &FillOptions::default().with_fill_rule(FillRule::EvenOdd),
            &mut simple_builder(&mut buffers),
        )
        .map_err(|error| format!("Could not triangulate fill contours: {error:?}"))?;
    if buffers.vertices.len() > MAX_FILL_MESH_VERTICES
        || buffers.indices.len() > MAX_FILL_MESH_INDICES
        || !buffers.indices.len().is_multiple_of(3)
    {
        return Err("Fill contour tessellation exceeds the mesh budget".into());
    }
    let vertices: Vec<_> = buffers
        .vertices
        .into_iter()
        .map(|vertex| DVec2::new(vertex.x as f64, vertex.y as f64))
        .collect();
    let triangles = buffers
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| {
            [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ]
        })
        .collect();
    Ok((vertices, triangles))
}

fn gradient_world(stroke: &Stroke, uv: glam::Vec2) -> Vec3 {
    let points = projected(stroke);
    let (min, max) = bounds(&points);
    let local = min + (max - min) * uv.as_dvec2();
    let origin = stroke.points[0].position;
    let tangent = stroke.brush.tangent;
    let bitangent = stroke.brush.normal.cross(tangent);
    origin + tangent * local.x as f32 + bitangent * local.y as f32
        - stroke.brush.normal * stroke.fill.as_ref().map_or(0.0, |fill| fill.offset)
}

pub fn gradient_handle_positions(stroke: &Stroke) -> Option<[Vec3; 2]> {
    let gradient = stroke.fill.as_ref()?.gradient?;
    Some([
        gradient_world(stroke, gradient.start),
        gradient_world(stroke, gradient.end),
    ])
}

pub fn gradient_uv_from_world(stroke: &Stroke, world: Vec3) -> Option<glam::Vec2> {
    let origin = stroke.points.first()?.position;
    let delta = (world - origin).as_dvec3();
    let tangent = stroke.brush.tangent.as_dvec3();
    let bitangent = stroke.brush.normal.as_dvec3().cross(tangent);
    let points = projected(stroke);
    let (min, max) = bounds(&points);
    Some(normalized_uv(
        DVec2::new(delta.dot(tangent), delta.dot(bitangent)),
        min,
        max,
    ))
}

pub fn mapped_fill_uv(stroke: &Stroke, world: Vec3) -> Option<glam::Vec2> {
    let origin = stroke.points.first()?.position;
    let delta = (world - origin).as_dvec3();
    let tangent = stroke.brush.tangent.as_dvec3();
    let bitangent = stroke.brush.normal.as_dvec3().cross(tangent);
    let points = projected(stroke);
    let (min, max) = bounds(&points);
    let uv = normalized_uv(
        DVec2::new(delta.dot(tangent), delta.dot(bitangent)),
        min,
        max,
    );
    Some(
        stroke
            .fill
            .as_ref()?
            .texture
            .map_or(uv, |texture| texture.map_uv(uv)),
    )
}
pub fn triangulate(stroke: &Stroke) -> Result<Vec<[usize; 3]>, String> {
    let fill = stroke
        .fill
        .as_ref()
        .ok_or("This mark is not a planar fill")?;
    if !(3..=MAX_CORNERS).contains(&stroke.points.len()) {
        return Err(format!("A fill needs 3–{MAX_CORNERS} corners"));
    }
    if !stroke.brush.is_valid()
        || !fill.offset.is_finite()
        || !(0.0..=1.0).contains(&fill.offset)
        || fill.gradient.is_some_and(|gradient| !gradient.is_valid())
        || fill.texture.is_some_and(|texture| !texture.is_valid())
        || stroke.brush.texture.kind != crate::texture::TextureKind::Solid
        || stroke.points.iter().any(|p| {
            !p.is_valid()
                || (p.position - stroke.brush.normal * fill.offset)
                    .abs()
                    .max_element()
                    > 10000.0
        })
    {
        return Err("Invalid fill frame, offset, color mask, or corners".into());
    }
    if let Some(surface) = fill.cylinder {
        if !surface.valid()
            || fill.offset >= surface.radius
            || stroke
                .points
                .iter()
                .any(|point| surface.chart_point(point.position).is_none())
        {
            return Err("Curved fill corners must lie on the finite saved cylinder".into());
        }
    }
    let points = projected(stroke);
    let size = points.iter().map(|p| p.length()).fold(1.0_f64, f64::max);
    let eps = (1e-10 * size * size).max(1e-8);
    let origin = stroke.points[0].position;
    let tolerance = 1e-5 * size + 2e-7 * origin.abs().max_element() as f64;
    if fill.cylinder.is_none()
        && stroke.points.iter().any(|p| {
            ((p.position - origin)
                .as_dvec3()
                .dot(stroke.brush.normal.as_dvec3()))
            .abs()
                > tolerance
        })
    {
        return Err("Fill corners must lie on one plane".into());
    }
    if fill.holes.len() > MAX_HOLES
        || stroke.points.len() + fill.holes.iter().map(Vec::len).sum::<usize>()
            > MAX_TOTAL_CONTOUR_POINTS
    {
        return Err("Fill contours exceed the 64-hole / 4,096-point limit".into());
    }
    let count = points.len();
    for i in 0..count {
        let next = (i + 1) % count;
        if points[i].distance_squared(points[next]) <= eps {
            return Err("Remove duplicate or extremely close fill corners".into());
        }
        for j in i + 1..count {
            let end = (j + 1) % count;
            if next == j || end == i {
                continue;
            }
            if intersects(points[i], points[next], points[j], points[end], eps) {
                return Err("Fill contour intersects itself; move or remove corners".into());
            }
        }
    }
    let area: f64 = (0..count)
        .map(|i| points[i].perp_dot(points[(i + 1) % count]))
        .sum();
    if area.abs() <= eps {
        return Err("Fill contour has no usable area".into());
    }
    let sign = area.signum();
    let mut ring: Vec<_> = (0..count).collect();
    loop {
        let remove = (0..ring.len()).find(|&i| {
            let a = points[ring[(i + ring.len() - 1) % ring.len()]];
            let b = points[ring[i]];
            let c = points[ring[(i + 1) % ring.len()]];
            cross(a, b, c).abs() <= eps && on_segment(b, a, c, eps)
        });
        if let Some(i) = remove {
            if ring.len() <= 3 {
                return Err("Fill contour has no usable area".into());
            }
            ring.remove(i);
        } else {
            break;
        }
    }
    let mut triangles = Vec::with_capacity(ring.len() - 2);
    while ring.len() > 3 {
        let ear = (0..ring.len())
            .find(|&i| {
                let [a, b, c] = [
                    ring[(i + ring.len() - 1) % ring.len()],
                    ring[i],
                    ring[(i + 1) % ring.len()],
                ];
                cross(points[a], points[b], points[c]) * sign > eps
                    && !ring.iter().any(|&p| {
                        p != a
                            && p != b
                            && p != c
                            && cross(points[a], points[b], points[p]) * sign >= -eps
                            && cross(points[b], points[c], points[p]) * sign >= -eps
                            && cross(points[c], points[a], points[p]) * sign >= -eps
                    })
            })
            .ok_or("Could not triangulate this contour; remove overlapping or redundant corners")?;
        triangles.push([
            ring[(ear + ring.len() - 1) % ring.len()],
            ring[ear],
            ring[(ear + 1) % ring.len()],
        ]);
        ring.remove(ear);
    }
    triangles.push([ring[0], ring[1], ring[2]]);
    if sign < 0.0 {
        for triangle in &mut triangles {
            triangle.swap(1, 2);
        }
    }
    if !fill.holes.is_empty() {
        let holes = projected_holes(stroke);
        for (index, (world, hole)) in fill.holes.iter().zip(&holes).enumerate() {
            if world.iter().any(|point| {
                !point.is_finite()
                    || (point - stroke.brush.normal * fill.offset)
                        .abs()
                        .max_element()
                        > 10_000.0
                    || ((*point - origin)
                        .as_dvec3()
                        .dot(stroke.brush.normal.as_dvec3()))
                    .abs()
                        > tolerance
            }) {
                return Err("Cutout corners must be finite and lie on the fill plane".into());
            }
            validate_simple_ring(hole, eps)?;
            if !point_in_ring(hole[0], &points)
                || (0..hole.len()).any(|i| {
                    let j = (i + 1) % hole.len();
                    (0..points.len()).any(|k| {
                        intersects(
                            hole[i],
                            hole[j],
                            points[k],
                            points[(k + 1) % points.len()],
                            eps,
                        )
                    })
                })
            {
                return Err(format!(
                    "Cutout {} must stay strictly inside the outer contour",
                    index + 1
                ));
            }
            for prior in 0..index {
                if (0..hole.len()).any(|i| {
                    let j = (i + 1) % hole.len();
                    (0..holes[prior].len()).any(|k| {
                        intersects(
                            hole[i],
                            hole[j],
                            holes[prior][k],
                            holes[prior][(k + 1) % holes[prior].len()],
                            eps,
                        )
                    })
                }) {
                    return Err("Cutout contours may nest but must not cross".into());
                }
            }
        }
        let mut contours = Vec::with_capacity(holes.len() + 1);
        contours.push(points.clone());
        contours.extend(holes);
        lyon_mesh(&contours)?;
    }
    Ok(triangles)
}
pub fn ray_hit(stroke: &Stroke, origin: Vec3, direction: Vec3) -> Option<Vec3> {
    let fill = stroke.fill.as_ref()?;
    let first = stroke.points.first()?.position;
    let (hit, p) = if let Some(surface) = fill.cylinder {
        let rotation = surface.rotation.normalize();
        let local_origin = rotation.inverse() * (origin - surface.center);
        let local_direction = rotation.inverse() * direction;
        let radius = surface.radius - fill.offset;
        if radius <= 0.0 {
            return None;
        }
        let a = local_direction.x * local_direction.x + local_direction.z * local_direction.z;
        if a < 1e-10 {
            return None;
        }
        let b = 2.0 * (local_origin.x * local_direction.x + local_origin.z * local_direction.z);
        let c = local_origin.x * local_origin.x + local_origin.z * local_origin.z - radius * radius;
        let discriminant = b * b - 4.0 * a * c;
        if discriminant < 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        let t = [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
            .into_iter()
            .find(|t| {
                *t >= 0.0 && (local_origin.y + local_direction.y * *t).abs() <= surface.height * 0.5
            })?;
        let local = local_origin + local_direction * t;
        let angle = local.x.atan2(local.z);
        let delta = (angle - surface.angle_origin + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        (
            origin + direction * t,
            DVec2::new((delta * surface.radius) as f64, local.y as f64),
        )
    } else {
        let n = stroke.brush.normal;
        let dot = direction.dot(n);
        if dot.abs() < 1e-6 {
            return None;
        }
        let t = (first - n * fill.offset - origin).dot(n) / dot;
        if t < 0.0 {
            return None;
        }
        let hit = origin + direction * t;
        let d = (hit + n * fill.offset - first).as_dvec3();
        let u = stroke.brush.tangent.as_dvec3();
        let v = n.as_dvec3().cross(u);
        (hit, DVec2::new(d.dot(u), d.dot(v)))
    };
    let points = projected(stroke);
    let mut inside = false;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        if on_segment(p, a, b, 1e-10) {
            return Some(hit);
        }
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    if !inside {
        return None;
    }
    for hole in projected_holes(stroke) {
        for i in 0..hole.len() {
            if on_segment(p, hole[i], hole[(i + 1) % hole.len()], 1e-10) {
                return None;
            }
        }
        if point_in_ring(p, &hole) {
            inside = !inside;
        }
    }
    inside.then_some(hit)
}
pub fn mesh(stroke: &Stroke) -> Vec<crate::geometry::Vertex> {
    let Ok(triangles) = triangulate(stroke) else {
        return Vec::new();
    };
    let Some(fill) = stroke.fill.as_ref() else {
        return Vec::new();
    };
    let offset = stroke.brush.normal * fill.offset;
    let outer = projected(stroke);
    let (min, max) = bounds(&outer);
    let (mesh_points, mesh_coordinates, triangles) = if fill.holes.is_empty() {
        (
            stroke.points.iter().map(|point| point.position).collect(),
            outer,
            triangles,
        )
    } else {
        let mut contours = Vec::with_capacity(fill.holes.len() + 1);
        contours.push(outer);
        contours.extend(projected_holes(stroke));
        let Ok((coordinates, triangles)) = lyon_mesh(&contours) else {
            return Vec::new();
        };
        let origin = stroke.points[0].position;
        let u = stroke.brush.tangent;
        let v = stroke.brush.normal.cross(u);
        let positions: Vec<Vec3> = coordinates
            .iter()
            .map(|point| origin + u * point.x as f32 + v * point.y as f32)
            .collect();
        (positions, coordinates, triangles)
    };
    let gradient = fill.gradient;
    let texture = fill.texture;
    let make_vertex = |position: Vec3, uv: glam::Vec2, pressure: f32| {
        let mut color = gradient.map_or(stroke.color, |gradient| gradient.sample(uv));
        if let Some(texture) = texture {
            if gradient.is_none() {
                color = texture.tint;
            } else {
                for (component, tint) in color.iter_mut().zip(texture.tint) {
                    *component *= tint;
                }
            }
        }
        let (position, normal) = if let Some(surface) = fill.cylinder {
            let chart = surface.chart_point(position);
            match chart.and_then(|chart| {
                surface.world_point(chart, fill.offset).map(|position| {
                    let angle = surface.angle_origin + chart.x / surface.radius;
                    (
                        position,
                        surface.rotation * glam::Vec3::new(angle.sin(), 0.0, angle.cos()),
                    )
                })
            }) {
                Some((position, normal)) => (position, normal),
                None => (position - offset, stroke.brush.normal),
            }
        } else {
            (position - offset, stroke.brush.normal)
        };
        crate::geometry::Vertex {
            material: 0.0,
            emission: [0.0; 3],
            grain_uv: [0.0; 2],
            grain_dynamics: [1.0; 2],
            gradient_t: 0.0,
            position: position.to_array(),
            normal: normal.to_array(),
            color,
            uv: texture.map_or(uv, |texture| texture.map_uv(uv)).to_array(),
            opacity: stroke.brush.opacity
                * texture.map_or(1.0, |texture| texture.opacity)
                * crate::dynamics::opacity_scale(stroke.brush.dynamics.opacity, pressure),
        }
    };
    let mut output = Vec::new();
    for [a, b, c] in triangles {
        if gradient
            .is_some_and(|gradient| gradient.kind == crate::document::FillGradientKind::Radial)
        {
            // Radial color fields are nonlinear; subdividing keeps their vertex-color
            // interpolation smooth in both the viewport and exported meshes.
            const DIVISIONS: usize = 8;
            let [pa, pb, pc] = [a, b, c].map(|i| mesh_points[i]);
            let [ua, ub, uc] = [a, b, c].map(|i| normalized_uv(mesh_coordinates[i], min, max));
            let [qa, qb, qc] = if fill.holes.is_empty() {
                [a, b, c].map(|i| stroke.points[i].pressure)
            } else {
                [1.0; 3]
            };
            let sample = |i: usize, j: usize| {
                let wb = i as f32 / DIVISIONS as f32;
                let wc = j as f32 / DIVISIONS as f32;
                let wa = 1.0 - wb - wc;
                (
                    pa * wa + pb * wb + pc * wc,
                    ua * wa + ub * wb + uc * wc,
                    qa * wa + qb * wb + qc * wc,
                )
            };
            for i in 0..DIVISIONS {
                for j in 0..(DIVISIONS - i) {
                    let p0 = sample(i, j);
                    let p1 = sample(i + 1, j);
                    let p2 = sample(i, j + 1);
                    for p in [p0, p1, p2] {
                        output.push(make_vertex(p.0, p.1, p.2));
                    }
                    if i + j + 1 < DIVISIONS {
                        let p3 = sample(i + 1, j + 1);
                        for p in [p1, p3, p2] {
                            output.push(make_vertex(p.0, p.1, p.2));
                        }
                    }
                }
            }
        } else {
            for i in [a, b, c] {
                output.push(make_vertex(
                    mesh_points[i],
                    normalized_uv(mesh_coordinates[i], min, max),
                    if fill.holes.is_empty() {
                        stroke.points[i].pressure
                    } else {
                        1.0
                    },
                ));
            }
        }
    }
    output
}

pub fn study() -> crate::document::Document {
    use crate::document::{Brush, Document, PlanarFill, Point};
    fn face(doc: &mut Document, corners: Vec<Vec3>, normal: Vec3, color: [f32; 3]) {
        let tangent = (corners[1] - corners[0]).normalize();
        let points: Vec<_> = corners
            .iter()
            .map(|&position| Point {
                position,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .collect();
        doc.strokes.push(Stroke {
            guide_attachment: None,
            model_surface: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: Some(PlanarFill {
                offset: 0.003,
                gradient: None,
                texture: None,
                holes: Vec::new(),
                cylinder: None,
                region_source: None,
            }),
            group_id: None,
            layer_id: 1,
            id: doc.strokes.len() as u64 + 1,
            color,
            gradient: None,
            radius: 0.025,
            brush: Brush {
                normal,
                tangent,
                ..Default::default()
            },
            points: points.clone(),
        });
        let mut outline = points;
        outline.push(outline[0]);
        doc.strokes.push(Stroke {
            guide_attachment: None,
            model_surface: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id: doc.strokes.len() as u64 + 1,
            color: [0.12, 0.17, 0.20],
            gradient: None,
            radius: 0.012,
            brush: Brush {
                normal,
                tangent,
                ..Default::default()
            },
            points: outline,
        });
    }
    let mut doc = Document::default();
    face(
        &mut doc,
        vec![
            Vec3::new(-1.0, 0.0, 0.9),
            Vec3::new(1.0, 0.0, 0.9),
            Vec3::new(1.0, 2.0, 0.9),
            Vec3::new(0.0, 2.8, 0.9),
            Vec3::new(-1.0, 2.0, 0.9),
        ],
        Vec3::Z,
        [0.88, 0.82, 0.67],
    );
    face(
        &mut doc,
        vec![
            Vec3::new(1.0, 0.0, 0.9),
            Vec3::new(1.0, 0.0, -0.9),
            Vec3::new(1.0, 1.97, -0.9),
            Vec3::new(1.0, 1.97, 0.9),
        ],
        Vec3::X,
        [0.78, 0.73, 0.60],
    );
    for side in [-1.0, 1.0] {
        let outer = side * 1.15;
        let normal = Vec3::new(side * 0.8, 1.0, 0.0).normalize();
        face(
            &mut doc,
            vec![
                Vec3::new(0.0, 2.8, 1.05),
                Vec3::new(outer, 1.88, 1.05),
                Vec3::new(outer, 1.88, -1.05),
                Vec3::new(0.0, 2.8, -1.05),
            ],
            normal,
            [0.18, 0.38, 0.43],
        );
    }
    for (x, y, width, height, color) in [
        (-0.5, 1.35, 0.55, 0.5, [0.22, 0.44, 0.56]),
        (0.5, 1.35, 0.55, 0.5, [0.22, 0.44, 0.56]),
        (0.45, 0.48, 0.48, 0.96, [0.43, 0.25, 0.13]),
    ] {
        face(
            &mut doc,
            vec![
                Vec3::new(x - width * 0.5, y - height * 0.5, 0.915),
                Vec3::new(x + width * 0.5, y - height * 0.5, 0.915),
                Vec3::new(x + width * 0.5, y + height * 0.5, 0.915),
                Vec3::new(x - width * 0.5, y + height * 0.5, 0.915),
            ],
            Vec3::Z,
            color,
        );
    }
    face(
        &mut doc,
        vec![
            Vec3::new(-1.35, 0.08, -0.9),
            Vec3::new(-1.05, 0.08, -0.9),
            Vec3::new(-1.05, 0.08, 0.98),
            Vec3::new(1.3, 0.08, 0.98),
            Vec3::new(1.3, 0.08, 1.35),
            Vec3::new(-1.35, 0.08, 1.35),
        ],
        Vec3::Y,
        [0.58, 0.43, 0.28],
    );
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Document, PlanarFill, Point};
    fn mark(points: &[[f32; 2]]) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: Some(PlanarFill {
                offset: 0.003,
                gradient: None,
                texture: None,
                holes: Vec::new(),
                cylinder: None,
                region_source: None,
            }),
            group_id: None,
            layer_id: 1,
            id: 1,
            color: [0.8, 0.6, 0.3],
            radius: 0.025,
            brush: Brush::default(),
            points: points
                .iter()
                .map(|p| Point {
                    position: Vec3::new(p[0], p[1], 0.0),
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                })
                .collect(),
        }
    }
    #[test]
    fn concave_winding_collinear_and_interior_picking() {
        for reverse in [false, true] {
            let mut fill = mark(&[
                [0.0, 0.0],
                [1.0, 0.0],
                [2.0, 0.0],
                [2.0, 1.0],
                [1.0, 1.0],
                [1.0, 2.0],
                [0.0, 2.0],
            ]);
            if reverse {
                fill.points.reverse();
            }
            let mesh = mesh(&fill);
            let area: f32 = mesh
                .as_chunks::<3>()
                .0
                .iter()
                .map(|triangle| {
                    let a = Vec3::from(triangle[0].position);
                    let b = Vec3::from(triangle[1].position);
                    let c = Vec3::from(triangle[2].position);
                    let cross = (b - a).cross(c - a);
                    assert!(cross.z > 0.0);
                    cross.length() * 0.5
                })
                .sum();
            assert!((area - 3.0).abs() < 1e-5);
            assert!(ray_hit(&fill, Vec3::new(0.5, 1.5, 5.0), -Vec3::Z).is_some());
            assert!(ray_hit(&fill, Vec3::new(1.5, 1.5, 5.0), -Vec3::Z).is_none());
            assert!(ray_hit(&fill, Vec3::new(0.0, 0.0, 5.0), -Vec3::Z).is_some());
        }
    }
    #[test]
    fn invalid_contours_fail_without_mutation() {
        for points in [
            vec![[0.0, 0.0], [1.0, 1.0], [0.0, 1.0], [1.0, 0.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        ] {
            let fill = mark(&points);
            let before = fill.clone();
            assert!(triangulate(&fill).is_err());
            assert_eq!(fill, before);
        }
        let mut fill = mark(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        fill.points[2].position.z = 0.1;
        assert!(triangulate(&fill).is_err());
        fill.points[2].position.z = 0.0;
        fill.fill.as_mut().unwrap().offset = f32::NAN;
        assert!(triangulate(&fill).is_err());
        fill.fill.as_mut().unwrap().offset = 0.0;
        fill.points = vec![fill.points[0]; MAX_CORNERS + 1];
        assert!(triangulate(&fill).is_err());
    }
    #[test]
    fn native_migration_history_transforms_and_groups_preserve_fill_geometry() {
        let doc = study();
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, doc.strokes);
        let mut legacy = serde_json::to_value(&doc).unwrap();
        legacy["version"] = serde_json::json!(6);
        legacy["strokes"][0]["fill"] = serde_json::json!("future extension");
        let migrated = Document::from_json(&legacy.to_string()).unwrap();
        assert!(migrated.strokes.iter().all(|s| s.fill.is_none()));
        let mut changed = doc.clone();
        let ids = changed.strokes.iter().map(|s| s.id).collect();
        let group = crate::editing::group_selected(&changed, &ids).unwrap();
        group.apply(&mut changed, true);
        let before = crate::editing::snapshot(&changed, &ids);
        let after = crate::editing::transformed(
            &before,
            Vec3::ZERO,
            Vec3::X,
            glam::Quat::from_rotation_y(0.7),
            1.5,
        )
        .unwrap();
        let edit = crate::editing::Edit::Change { before, after };
        edit.apply(&mut changed, true);
        Document::from_json(&serde_json::to_string(&changed).unwrap()).unwrap();
        edit.apply(&mut changed, false);
        group.apply(&mut changed, false);
        assert_eq!(changed.strokes, doc.strokes);
    }
}
