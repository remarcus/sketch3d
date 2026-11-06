//! Bounded planar arrangement used by the enclosed-region fill tool.
use crate::{
    document::{Document, Stroke},
    geometry::DrawingPlane,
};
use glam::{DVec2, Vec3};

const MAX_SEGMENTS: usize = 2_000;
const MAX_PAIR_CHECKS: usize = 1_000_000;
const MAX_FACES: usize = 20_000;

#[derive(Clone, Copy)]
struct Segment {
    a: DVec2,
    b: DVec2,
}

fn point_in_polygon(p: DVec2, ring: &[DVec2]) -> bool {
    let mut inside = false;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

fn intersection(a: Segment, b: Segment) -> Option<(f64, f64, DVec2)> {
    let r = a.b - a.a;
    let s = b.b - b.a;
    let den = r.perp_dot(s);
    if den.abs() < 1e-10 {
        return None;
    }
    let delta = b.a - a.a;
    let t = delta.perp_dot(s) / den;
    let u = delta.perp_dot(r) / den;
    ((-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u)).then_some((
        t.clamp(0.0, 1.0),
        u.clamp(0.0, 1.0),
        a.a + r * t,
    ))
}

fn project(stroke: &Stroke, plane: DrawingPlane) -> Option<Vec<DVec2>> {
    let (u, v, n) = plane.basis();
    let origin = plane.origin();
    if stroke.fill.is_some() || stroke.points.len() < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(stroke.points.len());
    for p in &stroke.points {
        if !p.is_valid() || (p.position - origin).dot(n).abs() > 0.002 {
            return None;
        }
        let d = (p.position - origin).as_dvec3();
        out.push(DVec2::new(d.dot(u.as_dvec3()), d.dot(v.as_dvec3())));
    }
    Some(out)
}

/// Finds the smallest bounded face containing `world`, with endpoints closer
/// than `gap` joined. Hidden layers are ignored; visible locked layers remain
/// useful as boundaries. When `active_layer` is set, only that layer contributes
/// boundaries. The returned contour lies exactly on the target plane.
pub fn find_region(
    doc: &Document,
    plane: DrawingPlane,
    world: Vec3,
    gap: f32,
    half_extent: Option<glam::Vec2>,
    active_layer: Option<u64>,
    closing_guides: &[(Vec3, Vec3)],
) -> Result<Vec<Vec3>, String> {
    if !world.is_finite() || !gap.is_finite() || !(0.0..=0.05).contains(&gap) {
        return Err("Invalid region point or gap tolerance".into());
    }
    let (u, v, _) = plane.basis();
    let origin = plane.origin();
    let d = (world - origin).as_dvec3();
    let click = DVec2::new(d.dot(u.as_dvec3()), d.dot(v.as_dvec3()));
    let mut segments = Vec::new();
    for stroke in &doc.strokes {
        if !doc.visible(stroke) || active_layer.is_some_and(|layer| stroke.layer_id != layer) {
            continue;
        }
        let Some(points) = project(stroke, plane) else {
            continue;
        };
        for pair in points.windows(2) {
            if pair[0].distance(pair[1]) > 1e-7 {
                segments.push(Segment {
                    a: pair[0],
                    b: pair[1],
                });
            }
            if segments.len() > MAX_SEGMENTS {
                return Err("Region boundary exceeds the 2,000-segment limit".into());
            }
        }
    }
    for &(a, b) in closing_guides {
        if !a.is_finite()
            || !b.is_finite()
            || (a - origin).dot(plane.basis().2).abs() > 0.002
            || (b - origin).dot(plane.basis().2).abs() > 0.002
        {
            return Err("Temporary closing guide is outside the active plane".into());
        }
        let da = (a - origin).as_dvec3();
        let db = (b - origin).as_dvec3();
        let (u, v, _) = plane.basis();
        segments.push(Segment {
            a: DVec2::new(da.dot(u.as_dvec3()), da.dot(v.as_dvec3())),
            b: DVec2::new(db.dot(u.as_dvec3()), db.dot(v.as_dvec3())),
        });
        if segments.len() > MAX_SEGMENTS {
            return Err("Region boundary exceeds the 2,000-segment limit".into());
        }
    }
    if let Some(extent) = half_extent {
        if !extent.is_finite() || extent.min_element() <= 0.0 {
            return Err("Active image plane has invalid bounds".into());
        }
        let corners = [
            DVec2::new(-extent.x as f64, -extent.y as f64),
            DVec2::new(extent.x as f64, -extent.y as f64),
            DVec2::new(extent.x as f64, extent.y as f64),
            DVec2::new(-extent.x as f64, extent.y as f64),
        ];
        for i in 0..4 {
            segments.push(Segment {
                a: corners[i],
                b: corners[(i + 1) % 4],
            });
        }
    }
    if segments.is_empty() {
        return Err("No visible coplanar stroke boundaries were found".into());
    }
    let mut cuts: Vec<_> = segments
        .iter()
        .map(|segment| vec![(0.0, segment.a), (1.0, segment.b)])
        .collect();
    let mut checks = 0usize;
    for i in 0..segments.len() {
        for j in i + 1..segments.len() {
            checks += 1;
            if checks > MAX_PAIR_CHECKS {
                return Err("Region boundary intersection budget exceeded".into());
            }
            if let Some((t, s, p)) = intersection(segments[i], segments[j]) {
                cuts[i].push((t, p));
                cuts[j].push((s, p));
            }
        }
    }
    // Join near endpoints. This explicit, world-unit tolerance is bounded by the UI.
    let tolerance = gap as f64;
    let mut nodes: Vec<DVec2> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let node_for = |p: DVec2, nodes: &mut Vec<DVec2>| -> usize {
        if let Some(i) = nodes
            .iter()
            .position(|q| q.distance(p) <= tolerance.max(1e-7))
        {
            i
        } else {
            nodes.push(p);
            nodes.len() - 1
        }
    };
    for list in &mut cuts {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        list.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-8);
        for pair in list.windows(2) {
            let a = node_for(pair[0].1, &mut nodes);
            let b = node_for(pair[1].1, &mut nodes);
            if a != b && !edges.contains(&(a, b)) && !edges.contains(&(b, a)) {
                edges.push((a, b));
            }
        }
    }
    // The edge graph is planar after intersection splitting. Walk the face on
    // the left of each directed edge by choosing the next clockwise neighbor.
    let mut adjacency = vec![Vec::<usize>::new(); nodes.len()];
    for &(a, b) in &edges {
        adjacency[a].push(b);
        adjacency[b].push(a);
    }
    for (i, neighbors) in adjacency.iter_mut().enumerate() {
        neighbors.sort_by(|a, b| {
            let pa = nodes[*a] - nodes[i];
            let pb = nodes[*b] - nodes[i];
            pa.y.atan2(pa.x).total_cmp(&pb.y.atan2(pb.x))
        });
    }
    let mut visited = std::collections::HashSet::<(usize, usize)>::new();
    let mut best: Option<Vec<DVec2>> = None;
    let mut face_count = 0;
    for &(a, b) in &edges {
        for (start, end) in [(a, b), (b, a)] {
            if visited.contains(&(start, end)) {
                continue;
            }
            let (mut x, mut y) = (start, end);
            let mut ring = Vec::new();
            loop {
                if !visited.insert((x, y)) {
                    break;
                }
                ring.push(nodes[x]);
                let Some(pos) = adjacency[y].iter().position(|n| *n == x) else {
                    break;
                };
                if adjacency[y].is_empty() {
                    break;
                }
                let next = adjacency[y][(pos + adjacency[y].len() - 1) % adjacency[y].len()];
                x = y;
                y = next;
                if (x, y) == (start, end) {
                    break;
                }
                if ring.len() > MAX_SEGMENTS * 2 {
                    break;
                }
            }
            face_count += 1;
            if face_count > MAX_FACES {
                return Err("Region face limit exceeded".into());
            }
            if (x, y) != (start, end) || ring.len() < 3 {
                continue;
            }
            let area: f64 = (0..ring.len())
                .map(|i| ring[i].perp_dot(ring[(i + 1) % ring.len()]))
                .sum();
            if area <= 1e-10 || !point_in_polygon(click, &ring) {
                continue;
            }
            if best.as_ref().is_none_or(|old| {
                area < (0..old.len())
                    .map(|i| old[i].perp_dot(old[(i + 1) % old.len()]))
                    .sum()
            }) {
                best = Some(ring);
            }
        }
    }
    let ring =
        best.ok_or("No enclosed region found. Close the outline or increase gap tolerance.")?;
    Ok(ring
        .into_iter()
        .map(|p| origin + u * p.x as f32 + v * p.y as f32)
        .collect())
}

/// Solves a region in the unwrapped chart of one finite saved cylinder. Only
/// sampled strokes explicitly attached to that guide contribute boundaries.
pub fn find_cylinder_region(
    doc: &Document,
    guide_id: u64,
    surface: crate::document::CylinderFillSurface,
    world: Vec3,
    gap: f32,
    active_layer: Option<u64>,
    closing_guides: &[(Vec3, Vec3)],
) -> Result<Vec<Vec3>, String> {
    let click = surface
        .chart_point(world)
        .ok_or("Click on the finite saved cylinder surface")?;
    let mut chart_doc = doc.clone();
    chart_doc.strokes.clear();
    for source in &doc.strokes {
        if source.guide_attachment != Some(guide_id)
            || source.fill.is_some()
            || source.curve.is_some()
            || !doc.visible(source)
        {
            continue;
        }
        let mut stroke = source.clone();
        let mut valid = true;
        for point in &mut stroke.points {
            if let Some(chart) = surface.chart_point(point.position) {
                point.position = Vec3::new(chart.x, chart.y, 0.0);
            } else {
                valid = false;
                break;
            }
        }
        if valid {
            stroke.guide_attachment = None;
            stroke.model_surface = None;
            chart_doc.strokes.push(stroke);
        }
    }
    let chart_guides = closing_guides
        .iter()
        .map(|(a, b)| {
            let a = surface
                .chart_point(*a)
                .ok_or("Temporary closing guide leaves the cylinder bounds")?;
            let b = surface
                .chart_point(*b)
                .ok_or("Temporary closing guide leaves the cylinder bounds")?;
            Ok((Vec3::new(a.x, a.y, 0.0), Vec3::new(b.x, b.y, 0.0)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let chart = find_region(
        &chart_doc,
        DrawingPlane::default(),
        Vec3::new(click.x, click.y, 0.0),
        gap,
        None,
        active_layer,
        &chart_guides,
    )?;
    chart
        .into_iter()
        .map(|point| {
            surface
                .world_point(glam::vec2(point.x, point.y), 0.0)
                .ok_or_else(|| "Enclosed region crosses the finite cylinder height".to_owned())
        })
        .collect()
}

#[cfg(test)]
mod boundary_scope_tests {
    use super::*;
    use crate::document::{Brush, Layer, Point, Stroke};

    fn line(id: u64, layer_id: u64, a: Vec3, b: Vec3) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id,
            id,
            color: [0.2, 0.4, 0.6],
            gradient: None,
            radius: 0.01,
            brush: Brush::default(),
            points: [a, b]
                .into_iter()
                .map(|position| Point {
                    position,
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                })
                .collect(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
        }
    }

    fn area(points: &[Vec3]) -> f32 {
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum::<f32>()
            .abs()
            * 0.5
    }

    fn plane_area(points: &[Vec3], plane: DrawingPlane) -> f32 {
        let origin = plane.origin();
        let (u, v, _) = plane.basis();
        let local: Vec<_> = points
            .iter()
            .map(|point| {
                let relative = *point - origin;
                glam::Vec2::new(relative.dot(u), relative.dot(v))
            })
            .collect();
        local
            .iter()
            .zip(local.iter().cycle().skip(1))
            .take(local.len())
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum::<f32>()
            .abs()
            * 0.5
    }

    #[test]
    fn concave_nearly_closed_region_solves_on_a_rotated_offset_plane() {
        let plane = DrawingPlane {
            anchor: Vec3::new(1.0, -2.0, 0.5),
            rotation_degrees: Vec3::new(23.0, 31.0, -17.0),
            depth: 0.8,
        };
        let (u, v, _) = plane.basis();
        let origin = plane.origin();
        let point = |x: f32, y: f32| origin + u * x + v * y;
        let corners = [
            point(-1.0, -1.0),
            point(1.0, -1.0),
            point(1.0, 0.0),
            point(0.0, 0.0),
            point(0.0, 1.0),
            point(-1.0, 1.0),
        ];
        let doc = crate::document::Document {
            strokes: (0..corners.len())
                .map(|index| {
                    let start = if index == 0 {
                        corners[index] - v * 0.01
                    } else {
                        corners[index]
                    };
                    let end = if index == 5 {
                        corners[(index + 1) % corners.len()] + v * 0.01
                    } else {
                        corners[(index + 1) % corners.len()]
                    };
                    line(index as u64 + 1, 1, start, end)
                })
                .collect(),
            ..Default::default()
        };
        let before = doc.strokes.clone();
        let seed = point(-0.5, 0.5);
        assert!(find_region(&doc, plane, seed, 0.0, None, None, &[]).is_err());
        let region = find_region(&doc, plane, seed, 0.021, None, None, &[]).unwrap();
        assert!(
            (plane_area(&region, plane) - 3.0).abs() < 0.02,
            "expected concave area 3, got {} from {region:?}",
            plane_area(&region, plane)
        );
        assert!(region
            .iter()
            .all(|point| (point - origin).dot(plane.basis().2).abs() < 1e-4));
        assert_eq!(
            doc.strokes, before,
            "region solving leaves boundary art untouched"
        );
    }

    #[test]
    fn visible_layer_boundaries_split_a_region_but_active_layer_scope_ignores_them() {
        let mut doc = crate::document::Document::default();
        doc.layers.push(Layer {
            id: 2,
            name: "Detail".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: Default::default(),
            hide_onion_skin: false,
            mask_inverted: false,
            mask_source: None,
            clip_to_below: false,
        });
        doc.strokes = vec![
            line(1, 1, Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0)),
            line(2, 1, Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)),
            line(3, 1, Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)),
            line(4, 1, Vec3::new(-1.0, 1.0, 0.0), Vec3::new(-1.0, -1.0, 0.0)),
            line(5, 2, Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        ];
        let plane = DrawingPlane::default();
        let click = Vec3::new(0.0, 0.5, 0.0);
        let visible = find_region(&doc, plane, click, 0.0, None, None, &[]).unwrap();
        let active = find_region(&doc, plane, click, 0.0, None, Some(1), &[]).unwrap();
        assert!(
            (area(&visible) - 2.0).abs() < 1e-4,
            "visible region area {}: {visible:?}",
            area(&visible)
        );
        assert!(
            (area(&active) - 4.0).abs() < 1e-4,
            "active region area {}: {active:?}",
            area(&active)
        );

        doc.layers[1].visible = false;
        let hidden = find_region(&doc, plane, click, 0.0, None, None, &[]).unwrap();
        assert!((area(&hidden) - 4.0).abs() < 1e-4);
    }

    #[test]
    fn image_plane_edges_close_region_around_an_interior_boundary() {
        let plane = DrawingPlane::default();
        let doc = crate::document::Document {
            strokes: vec![line(
                1,
                1,
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            )],
            ..Default::default()
        };
        let seed = Vec3::new(-0.5, 0.0, 0.0);
        assert!(find_region(&doc, plane, seed, 0.0, None, None, &[]).is_err());

        let region = find_region(&doc, plane, seed, 0.0, Some(glam::Vec2::ONE), None, &[]).unwrap();
        assert!((area(&region) - 2.0).abs() < 1e-4);
        assert!(region.iter().all(|point| point.x <= 1e-4));
        assert!(region.iter().all(|point| point.x >= -1.0 - 1e-4));
        assert!(region.iter().all(|point| point.y.abs() <= 1.0 + 1e-4));
    }

    #[test]
    fn crossing_boundary_strokes_split_the_clicked_face() {
        let doc = crate::document::Document {
            strokes: vec![
                line(1, 1, Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0)),
                line(2, 1, Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)),
                line(3, 1, Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)),
                line(4, 1, Vec3::new(-1.0, 1.0, 0.0), Vec3::new(-1.0, -1.0, 0.0)),
                line(5, 1, Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
                line(6, 1, Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)),
            ],
            ..Default::default()
        };
        let original = doc.strokes.clone();
        let region = find_region(
            &doc,
            DrawingPlane::default(),
            Vec3::new(0.5, 0.5, 0.0),
            0.0,
            None,
            None,
            &[],
        )
        .unwrap();
        assert!((area(&region) - 1.0).abs() < 1e-4);
        assert!(region
            .iter()
            .all(|point| point.x >= -1e-4 && point.y >= -1e-4));
        assert_eq!(
            doc.strokes, original,
            "crossing boundaries remain unchanged"
        );
    }

    #[test]
    fn temporary_closing_segment_solves_a_gap_without_mutating_boundary_art() {
        let doc = crate::document::Document {
            strokes: vec![
                line(1, 1, Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0)),
                line(2, 1, Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)),
                line(3, 1, Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)),
                line(4, 1, Vec3::new(-1.0, 1.0, 0.0), Vec3::new(-1.0, 0.1, 0.0)),
                line(5, 1, Vec3::new(-1.0, -0.1, 0.0), Vec3::new(-1.0, -1.0, 0.0)),
            ],
            ..Default::default()
        };
        let before = doc.strokes.clone();
        let plane = DrawingPlane::default();
        let click = Vec3::ZERO;
        assert!(find_region(&doc, plane, click, 0.0, None, None, &[]).is_err());
        let guides = [(Vec3::new(-1.0, 0.1, 0.0), Vec3::new(-1.0, -0.1, 0.0))];
        let closed = find_region(&doc, plane, click, 0.0, None, None, &guides).unwrap();
        assert!((area(&closed) - 4.0).abs() < 1e-4);
        assert_eq!(
            doc.strokes, before,
            "temporary guides are solver input, never document artwork"
        );
    }
}
