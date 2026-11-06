//! Region fill on a bounded, locally unfoldable patch of an imported OBJ.
use std::collections::{HashMap, HashSet, VecDeque};

use glam::{DVec2, DVec3, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    document::{
        model_references::{ModelInkAnchor, ObjReferenceResource},
        Document, Stroke,
    },
    geometry::Vertex,
    obj_mesh_index::ObjMeshIndex,
    obj_reference::ObjModel,
    polygon,
};

const MAX_BOUNDARY_SAMPLES: usize = 2_000;
const MAX_FACES: usize = 512;
pub const MAX_TRIANGLES: usize = 4_096;
pub const MAX_DOCUMENT_TRIANGLES: usize = 200_000;
const EPS: f64 = 1e-8;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshFillSource {
    pub model_id: u64,
    pub seed: ModelInkAnchor,
    pub boundary_layer: Option<u64>,
    pub gap_tolerance: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshFill {
    pub triangles: Vec<[Vec3; 3]>,
    pub source: Option<MeshFillSource>,
}

impl MeshFill {
    pub fn validate(&self, stroke: &Stroke) -> Result<(), String> {
        let fill = stroke.fill.as_ref().ok_or("Missing mesh fill")?;
        if !(3..=polygon::MAX_CORNERS).contains(&stroke.points.len())
            || stroke.points.iter().any(|point| !point.is_valid())
            || !stroke.brush.is_valid()
            || fill.offset != 0.0
            || fill.cylinder.is_some()
            || fill.region_source.is_some()
            || !fill.holes.is_empty()
            || fill.gradient.is_some()
            || fill.texture.is_some()
            || self.source.is_some_and(|source| {
                !source.seed.is_valid() || !(0.0..=0.05).contains(&source.gap_tolerance)
            })
            || !(1..=MAX_TRIANGLES).contains(&self.triangles.len())
            || self.triangles.iter().any(|triangle| {
                triangle
                    .iter()
                    .any(|p| !p.is_finite() || p.abs().max_element() > 10_000.0)
                    || face_normal(*triangle).is_none()
            })
        {
            return Err("Invalid or oversized mesh fill".into());
        }
        Ok(())
    }

    pub fn ray_hit(&self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        self.triangles
            .iter()
            .filter_map(|triangle| {
                crate::document::ray_triangle_distance(origin, direction, *triangle)
            })
            .min_by(f32::total_cmp)
            .map(|distance| origin + direction * distance)
    }

    pub fn vertices(&self, stroke: &Stroke) -> Vec<Vertex> {
        self.triangles
            .iter()
            .filter_map(|triangle| {
                let normal = face_normal(*triangle)?;
                Some(triangle.map(|position| Vertex {
                    // Lift paint just above the opaque reference to avoid depth ties.
                    position: (position + normal * 0.0002).to_array(),
                    normal: normal.to_array(),
                    color: stroke.color,
                    uv: [0.0; 2],
                    opacity: stroke.brush.opacity,
                    material: 0.0,
                    grain_uv: [0.0; 2],
                    grain_dynamics: [1.0; 2],
                    gradient_t: 0.0,
                    emission: [0.0; 3],
                }))
            })
            .flatten()
            .collect()
    }
}

fn face_normal([a, b, c]: [Vec3; 3]) -> Option<Vec3> {
    (b - a).cross(c - a).try_normalize()
}

type Chart = [DVec2; 3];

struct Face {
    corners: [u32; 3],
    positions: [DVec3; 3],
}

impl Face {
    fn new(model: &ObjModel, id: u32) -> Result<Self, String> {
        let triangle = model.triangles.get(id as usize).ok_or("Missing OBJ face")?;
        let corners = triangle.corners.map(|corner| corner.position);
        let positions = corners.map(|corner| model.positions[corner as usize].as_dvec3());
        Ok(Self { corners, positions })
    }

    fn across(&self, other: &Self, chart: Chart, edge: usize) -> Result<Chart, String> {
        let next = (edge + 1) % 3;
        let first = other
            .corners
            .iter()
            .position(|&id| id == self.corners[edge])
            .ok_or("OBJ faces do not share an edge")?;
        let second = other
            .corners
            .iter()
            .position(|&id| id == self.corners[next])
            .ok_or("OBJ faces do not share an edge")?;
        let third = 3 - first - second;
        let side = cross(chart[edge], chart[next], chart[(edge + 2) % 3]).signum();
        let mut result = [DVec2::ZERO; 3];
        result[first] = chart[edge];
        result[second] = chart[next];
        result[third] = unfold_corner(
            result[first],
            result[second],
            other.positions[third].distance_squared(other.positions[first]),
            other.positions[third].distance_squared(other.positions[second]),
            -side,
        )?;
        Ok(result)
    }
}

fn cross(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    (b - a).perp_dot(c - a)
}

fn unfold_corner(a: DVec2, b: DVec2, da: f64, db: f64, side: f64) -> Result<DVec2, String> {
    let edge = b - a;
    let length = edge.length();
    if length <= EPS {
        return Err("OBJ patch contains a degenerate edge".into());
    }
    let along = (da - db + length * length) / (2.0 * length);
    let height = (da - along * along).max(0.0).sqrt();
    if height <= EPS {
        return Err("OBJ patch contains a degenerate face".into());
    }
    Ok(a + (edge * along + edge.perp() * (side * height)) / length)
}

fn extend_chart(
    model: &ObjModel,
    charts: &mut HashMap<u32, Chart>,
    id: u32,
    next: u32,
    edge: usize,
) -> Result<(), String> {
    let chart = Face::new(model, id)?.across(&Face::new(model, next)?, charts[&id], edge)?;
    if let Some(existing) = charts.get(&next) {
        if existing
            .iter()
            .zip(chart)
            .any(|(a, b)| a.distance(b) > 1e-5)
        {
            return Err(
                "The enclosed OBJ patch cannot be unfolded without overlap or distortion".into(),
            );
        }
    } else {
        if charts.len() >= MAX_FACES {
            return Err("Mesh fill exceeds the 512-face work limit".into());
        }
        charts.insert(next, chart);
    }
    Ok(())
}

fn chart_point(chart: Chart, anchor: ModelInkAnchor) -> DVec2 {
    let w = anchor.barycentric.as_dvec3();
    chart[0] * w.x + chart[1] * w.y + chart[2] * w.z
}

fn local_point(model: &ObjModel, anchor: ModelInkAnchor) -> Result<DVec3, String> {
    let face = Face::new(model, anchor.triangle)?;
    let w = anchor.barycentric.as_dvec3();
    Ok(face.positions[0] * w.x + face.positions[1] * w.y + face.positions[2] * w.z)
}

fn contour(
    doc: &Document,
    model: &ObjModel,
    index: &ObjMeshIndex,
    resource: &ObjReferenceResource,
    source: MeshFillSource,
) -> Result<Vec<ModelInkAnchor>, String> {
    let mut paths = Vec::new();
    let mut count = 0;
    let mut remaining_queries = 500_000usize;
    for stroke in &doc.strokes {
        if !doc.visible(stroke)
            || source
                .boundary_layer
                .is_some_and(|id| stroke.layer_id != id)
        {
            continue;
        }
        let Some(attachment) = stroke
            .model_surface
            .as_ref()
            .and_then(|patch| patch.attachment.as_ref())
        else {
            continue;
        };
        if attachment.model_id != resource.id
            || !attachment.anchors.first().is_some_and(|anchor| {
                index.triangles_connected(source.seed.triangle, anchor.triangle)
            })
            || stroke.points.len() < 2
        {
            continue;
        }
        count += stroke.points.len();
        if count > MAX_BOUNDARY_SAMPLES {
            return Err("Mesh fill exceeds the 2,000-sample boundary limit".into());
        }
        let mut path = Vec::with_capacity(stroke.points.len());
        for (i, point) in stroke.points.iter().enumerate() {
            let local =
                resource.rotation.inverse() * (point.position - resource.center) / resource.scale;
            let anchor = attachment.anchors.get(i).copied().filter(|&anchor| {
                local_point(model, anchor)
                    .is_ok_and(|saved| saved.distance(local.as_dvec3()) <= 1e-5)
            });
            let anchor = match anchor {
                Some(anchor) => anchor,
                None => {
                    let (hit, tested) =
                        index.closest_point_bounded(model, local, remaining_queries, true)?;
                    remaining_queries -= tested;
                    hit.filter(|hit| hit.position.distance(local) <= 1e-5)
                        .map(ModelInkAnchor::from)
                        .ok_or("An edited boundary sample is no longer on the OBJ surface")?
                }
            };
            if path.last().is_none_or(|last| *last != anchor) {
                path.push(anchor);
            }
        }
        if path.len() > 1 {
            paths.push(path);
        }
    }
    let mut ring = paths
        .pop()
        .ok_or("No visible attached outline on the clicked OBJ component")?;
    let tolerance = f64::from(source.gap_tolerance / resource.scale).max(1e-5);
    let joins = |a: ModelInkAnchor, b: ModelInkAnchor| {
        index.triangles_adjacent(a.triangle, b.triangle)
            && local_point(model, a)
                .ok()
                .zip(local_point(model, b).ok())
                .is_some_and(|(a, b)| a.distance(b) <= tolerance)
    };
    while !joins(*ring.last().unwrap(), ring[0]) {
        let end = *ring.last().unwrap();
        let mut candidates = paths
            .iter()
            .enumerate()
            .flat_map(|(i, path)| [(i, false, path[0]), (i, true, *path.last().unwrap())])
            .filter(|(_, _, start)| joins(end, *start));
        let (i, reverse, _) = candidates
            .next()
            .ok_or("The attached OBJ outline is open")?;
        if candidates.next().is_some() {
            return Err("The attached OBJ outline has an ambiguous junction".into());
        }
        let mut path = paths.swap_remove(i);
        if reverse {
            path.reverse();
        }
        ring.extend(path);
    }
    if !paths.is_empty() {
        return Err("Mesh fill needs one outline on the clicked component. Limit boundary layers to isolate it.".into());
    }
    Ok(ring)
}

fn clip(mut polygon: Vec<DVec2>, triangle: Chart) -> Vec<DVec2> {
    let sign = cross(triangle[0], triangle[1], triangle[2]).signum();
    for i in 0..3 {
        let input = std::mem::take(&mut polygon);
        let Some(&last) = input.last() else { break };
        let a = triangle[i];
        let b = triangle[(i + 1) % 3];
        let mut previous = last;
        let mut previous_distance = sign * cross(a, b, previous);
        for current in input {
            let distance = sign * cross(a, b, current);
            if (distance >= 0.0) != (previous_distance >= 0.0) {
                polygon.push(
                    previous.lerp(current, previous_distance / (previous_distance - distance)),
                );
            }
            if distance >= 0.0 {
                polygon.push(current);
            }
            previous = current;
            previous_distance = distance;
        }
    }
    polygon
}

fn chart_weights(chart: Chart, point: DVec2) -> DVec3 {
    let area = cross(chart[0], chart[1], chart[2]);
    DVec3::new(
        cross(chart[1], chart[2], point),
        cross(chart[2], chart[0], point),
        cross(chart[0], chart[1], point),
    ) / area
}

fn world_point(resource: &ObjReferenceResource, local: DVec3) -> Vec3 {
    resource.center + resource.rotation * (local.as_vec3() * resource.scale)
}

pub fn solve(
    doc: &Document,
    resource: &ObjReferenceResource,
    model: &ObjModel,
    index: &ObjMeshIndex,
    source: MeshFillSource,
) -> Result<(MeshFill, Vec<Vec3>), String> {
    if source
        .boundary_layer
        .is_some_and(|id| !doc.layers.iter().any(|layer| layer.id == id))
    {
        return Err("The original boundary layer is unavailable".into());
    }
    let anchors = contour(doc, model, index, resource, source)?;
    let first = anchors[0].triangle;
    let [a, b, c] = Face::new(model, first)?.positions;
    let edge = DVec2::new(a.distance(b), 0.0);
    let third = unfold_corner(
        DVec2::ZERO,
        edge,
        c.distance_squared(a),
        c.distance_squared(b),
        1.0,
    )?;
    let mut charts = HashMap::from([(first, [DVec2::ZERO, edge, third])]);
    let mut outline = Vec::new();
    for (&a, &b) in anchors.iter().zip(anchors.iter().cycle().skip(1)) {
        let point = chart_point(charts[&a.triangle], a);
        if outline
            .last()
            .is_none_or(|&(last, _): &(DVec2, ModelInkAnchor)| last.distance(point) > 1e-6)
        {
            outline.push((point, a));
        }
        if a.triangle != b.triangle {
            let edge = index
                .triangle_neighbors(a.triangle)
                .and_then(|neighbors| {
                    neighbors
                        .iter()
                        .position(|neighbor| *neighbor == Some(b.triangle))
                })
                .ok_or("The attached outline jumps across non-neighboring OBJ faces")?;
            extend_chart(model, &mut charts, a.triangle, b.triangle, edge)?;
            let face = charts[&a.triangle];
            let next = chart_point(charts[&b.triangle], b);
            let start = face[edge];
            let end = face[(edge + 1) % 3];
            let side_a = cross(point, next, start);
            let side_b = cross(point, next, end);
            if (point.distance(next) > EPS) && side_a * side_b > EPS * EPS {
                return Err("An outline segment misses its shared OBJ edge".into());
            }
        }
    }
    if outline.len() > 1 && outline[0].0.distance(outline.last().unwrap().0) <= 1e-6 {
        outline.pop();
    }
    // Reduce straight runs before applying the usual contour corner limit.
    let mut i = 0;
    while outline.len() > 3 && i < outline.len() {
        let len = outline.len();
        let a = outline[(i + len - 1) % len].0;
        let b = outline[i].0;
        let c = outline[(i + 1) % len].0;
        if cross(a, b, c).abs() <= EPS && (b - a).dot(b - c) <= 0.0 {
            outline.remove(i);
            i = i.saturating_sub(1);
        } else {
            i += 1;
        }
    }
    let ring: Vec<_> = outline.iter().map(|&(point, _)| point).collect();
    polygon::validate_simple_ring(&ring, 1e-10)?;
    let (vertices, indices) = polygon::lyon_mesh(std::slice::from_ref(&ring))?;
    let chart_triangles: Vec<_> = indices
        .into_iter()
        .map(|ids| ids.map(|i| vertices[i]))
        .collect();
    let expected_area: f64 = chart_triangles
        .iter()
        .map(|&[a, b, c]| cross(a, b, c).abs())
        .sum();
    let mut covered_area = 0.0;
    let mut triangles = Vec::new();
    let mut queue = VecDeque::from([first]);
    let mut visited = HashSet::from([first]);
    while let Some(id) = queue.pop_front() {
        let chart = charts[&id];
        let face = Face::new(model, id)?;
        let winding = cross(chart[0], chart[1], chart[2]).signum();
        let mut crosses_edge = [false; 3];
        for part in &chart_triangles {
            let clipped = clip(part.to_vec(), chart);
            if clipped.len() < 3 {
                continue;
            }
            for i in 1..clipped.len() - 1 {
                let mut triangle = [clipped[0], clipped[i], clipped[i + 1]];
                let area = cross(triangle[0], triangle[1], triangle[2]);
                if area.abs() <= EPS {
                    continue;
                }
                covered_area += area.abs();
                if area.signum() != winding {
                    triangle.swap(1, 2);
                }
                let triangle = triangle.map(|point| {
                    let w = chart_weights(chart, point);
                    let local =
                        face.positions[0] * w.x + face.positions[1] * w.y + face.positions[2] * w.z;
                    world_point(resource, local)
                });
                if face_normal(triangle).is_some() {
                    triangles.push(triangle);
                }
                if triangles.len() > MAX_TRIANGLES {
                    return Err("Mesh fill exceeds the tessellation limit".into());
                }
            }
            for (edge, crosses) in crosses_edge.iter_mut().enumerate() {
                let a = chart[edge];
                let b = chart[(edge + 1) % 3];
                *crosses |= clipped
                    .iter()
                    .zip(clipped.iter().cycle().skip(1))
                    .any(|(&p, &q)| {
                        cross(a, b, p).abs() <= EPS
                            && cross(a, b, q).abs() <= EPS
                            && p.distance(q) > EPS
                    });
            }
        }
        for (edge, neighbor) in index
            .triangle_neighbors(id)
            .unwrap_or([None; 3])
            .into_iter()
            .enumerate()
        {
            if let (true, Some(next)) = (crosses_edge[edge], neighbor) {
                extend_chart(model, &mut charts, id, next, edge)?;
                if visited.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }
    if triangles.is_empty()
        || (covered_area - expected_area).abs() > (expected_area * 1e-4).max(EPS)
    {
        return Err("The enclosed region crosses an OBJ border, seam, or overlapping chart".into());
    }
    if !visited.contains(&source.seed.triangle)
        || !polygon::point_in_ring(
            chart_point(charts[&source.seed.triangle], source.seed),
            &ring,
        )
    {
        return Err("Click inside the attached OBJ outline".into());
    }
    let points = outline
        .into_iter()
        .map(|(_, anchor)| local_point(model, anchor).map(|point| world_point(resource, point)))
        .collect::<Result<_, _>>()?;
    Ok((
        MeshFill {
            triangles,
            source: Some(source),
        },
        points,
    ))
}

pub fn replace_geometry(
    doc: &Document,
    original: &Stroke,
    mesh: MeshFill,
    points: Vec<crate::document::Point>,
) -> Result<Stroke, String> {
    let other_strokes = doc.strokes.iter().filter(|stroke| stroke.id != original.id);
    let triangles = other_strokes
        .clone()
        .filter_map(|stroke| stroke.fill.as_ref()?.mesh.as_ref())
        .map(|mesh| mesh.triangles.len())
        .sum::<usize>();
    let samples = other_strokes
        .map(|stroke| stroke.points.len())
        .sum::<usize>();
    if triangles + mesh.triangles.len() > MAX_DOCUMENT_TRIANGLES
        || samples + points.len() > 2_000_000
    {
        return Err("The refreshed mesh fill would exceed the document geometry limit".into());
    }
    let mut mark = original.clone();
    let fill = mark.fill.as_mut().ok_or("Missing original mesh fill")?;
    fill.mesh = Some(mesh);
    mark.points = points;
    polygon::validate(&mark)?;
    Ok(mark)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{model_references::ModelInkAttachment, ModelSurfacePatch, Point};

    #[test]
    fn fills_and_refreshes_a_crease_and_rejects_an_open_boundary() {
        let model = crate::obj_reference::parse(
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 -1\nf 1 2 3\nf 2 1 4\n",
            std::path::Path::new("crease.obj"),
        )
        .unwrap();
        let index = ObjMeshIndex::new(&model).unwrap();
        let resource = ObjReferenceResource {
            id: 5,
            name: "Crease".into(),
            visible: true,
            locked: false,
            center: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: 1.0,
            live_line_art: None,
            asset: crate::obj_reference_storage::ObjReferenceAsset {
                id: 5,
                bundle_dir: "models/5".into(),
            },
        };
        let top = |x, y| ModelInkAnchor {
            triangle: 0,
            barycentric: Vec3::new(1.0 - x - y, x, y),
        };
        let side = |x, depth| ModelInkAnchor {
            triangle: 1,
            barycentric: Vec3::new(x, 1.0 - x - depth, depth),
        };
        let anchors = vec![
            top(0.1, 0.15),
            top(0.3, 0.15),
            side(0.3, 0.15),
            side(0.1, 0.15),
            top(0.1, 0.15),
        ];
        let points = anchors
            .iter()
            .map(|&anchor| Point {
                position: local_point(&model, anchor).unwrap().as_vec3(),
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .collect();
        let mut doc = Document::default();
        doc.model_references.push(resource.clone());
        doc.strokes.push(Stroke {
            guide_attachment: None,
            model_surface: Some(ModelSurfacePatch {
                triangles: Vec::new(),
                attachment: Some(ModelInkAttachment {
                    model_id: 5,
                    anchors,
                }),
            }),
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id: 6,
            color: [0.0; 3],
            gradient: None,
            radius: 0.01,
            brush: Default::default(),
            points,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
        });
        let source = MeshFillSource {
            model_id: 5,
            seed: top(0.2, 0.05),
            boundary_layer: Some(1),
            gap_tolerance: 0.0,
        };
        let (fill, points) = solve(&doc, &resource, &model, &index, source).unwrap();
        assert!(fill.triangles.iter().all(|triangle| {
            triangle.iter().all(|point| point.y.abs() < 1e-6)
                || triangle.iter().all(|point| point.z.abs() < 1e-6)
        }));
        assert!(fill.triangles.iter().flatten().any(|point| point.z < -0.1));
        assert!(fill.triangles.iter().flatten().any(|point| point.y > 0.1));
        let as_points = |positions: Vec<Vec3>| {
            positions
                .into_iter()
                .map(|position| Point {
                    position,
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                })
                .collect()
        };
        let mut original = doc.strokes[0].clone();
        original.id = 7;
        original.color = [0.2, 0.4, 0.6];
        original.model_surface = None;
        original.fill = Some(crate::document::PlanarFill {
            offset: 0.0,
            mesh: Some(fill),
            ..Default::default()
        });
        original.points = as_points(points);
        doc.strokes.push(original.clone());
        for point in &mut doc.strokes[0].points[2..4] {
            point.position.z = -0.2;
        }
        let (fill, points) = solve(&doc, &resource, &model, &index, source).unwrap();
        let refreshed = replace_geometry(&doc, &original, fill, as_points(points)).unwrap();
        assert_eq!(
            (refreshed.id, refreshed.layer_id, refreshed.color),
            (original.id, original.layer_id, original.color)
        );
        assert_ne!(refreshed.points, original.points);
        let edit = crate::editing::Edit::Change {
            before: vec![(1, original.clone())],
            after: vec![(1, refreshed.clone())],
        };
        edit.apply(&mut doc, true);
        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes[1], refreshed);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes[1], original);
        doc.strokes[0].points.pop();
        assert!(solve(&doc, &resource, &model, &index, source)
            .unwrap_err()
            .contains("open"));
        assert_eq!(doc.strokes[1], original);
    }
}
