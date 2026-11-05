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
    let mut cuts = vec![vec![(0.0, segments[0].a), (1.0, segments[0].b)]; segments.len()];
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
