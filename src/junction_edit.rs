//! Bounded intersection edits for sampled strokes on one drawing plane.
use crate::{
    document::{Document, Point, Stroke},
    editing::Snapshot,
    geometry::DrawingPlane,
    partial_eraser::{interpolate_point, retained_fragments},
};
use glam::{DVec2, Vec3};

const PLANE_TOLERANCE: f32 = 0.0005;
const EPS: f64 = 1e-6;
const MAX_INTERSECTION_TESTS: usize = 500_000;

#[derive(Clone, Copy)]
pub enum Action {
    Trim,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trim => "Trim to intersection",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Target {
    pub plane: DrawingPlane,
    pub half_extent: Option<glam::Vec2>,
}

impl Target {
    fn chart(self, world: Vec3) -> DVec2 {
        let (u, v, _) = self.plane.basis();
        let delta = (world - self.plane.origin()).as_dvec3();
        DVec2::new(delta.dot(u.as_dvec3()), delta.dot(v.as_dvec3()))
    }

    fn contains(self, world: Vec3) -> bool {
        (world - self.plane.origin())
            .dot(self.plane.basis().2)
            .abs()
            <= PLANE_TOLERANCE
            && self.half_extent.is_none_or(|extent| {
                self.chart(world)
                    .abs()
                    .cmple(extent.as_dvec2() + DVec2::splat(EPS))
                    .all()
            })
    }
}

#[derive(Clone, PartialEq)]
pub struct Preview {
    pub after: Snapshot,
    pub next_id: u64,
    pub path: Vec<Vec3>,
    pub joins: Vec<Vec3>,
}

pub fn available(doc: &Document) -> bool {
    doc.animation.is_empty() && doc.animation.two_bone_rig.is_none()
}

fn sampled_on_plane(stroke: &Stroke, target: Target) -> bool {
    stroke.points.len() >= 2
        && stroke.fill.is_none()
        && stroke.curve.is_none()
        && stroke.modifiers.is_none()
        && stroke.model_surface.is_none()
        && stroke
            .points
            .iter()
            .all(|point| target.contains(point.position))
}

pub fn editable_source(doc: &Document, stroke: &Stroke, target: Target) -> bool {
    doc.editable(stroke)
        && sampled_on_plane(stroke, target)
        && !doc.has_linked_source(stroke.id)
        && doc.linked_instance_for_stroke(stroke.id).is_none()
}

pub fn preview(
    doc: &Document,
    action: Action,
    target: Target,
    index: usize,
    at: f64,
    next_id: u64,
) -> Result<Preview, String> {
    let stroke = doc
        .strokes
        .get(index)
        .filter(|stroke| editable_source(doc, stroke, target))
        .ok_or("The stroke is missing, protected, or off the active plane")?;
    if !available(doc) {
        return Err("Intersection edits require static artwork".into());
    }
    match action {
        Action::Trim => trim(doc, stroke, target, index, at, next_id),
    }
}

fn boundaries<'a>(
    doc: &'a Document,
    source: &Stroke,
    target: Target,
) -> impl Iterator<Item = [DVec2; 2]> + 'a {
    let source_id = source.id;
    doc.strokes
        .iter()
        .filter(move |stroke| {
            stroke.id != source_id && doc.editable(stroke) && sampled_on_plane(stroke, target)
        })
        .flat_map(move |stroke| {
            stroke.points.windows(2).map(move |pair| {
                [
                    target.chart(pair[0].position),
                    target.chart(pair[1].position),
                ]
            })
        })
}

// Return the parameter on the first line, bounded on the second line.
fn intersection(a: DVec2, ab: DVec2, c: DVec2, cd: DVec2) -> Option<f64> {
    let denominator = ab.perp_dot(cd);
    if denominator.abs() <= EPS * ab.length() * cd.length() || denominator == 0.0 {
        return None;
    }
    let offset = c - a;
    let u = offset.perp_dot(ab) / denominator;
    (-EPS..=1.0 + EPS)
        .contains(&u)
        .then(|| offset.perp_dot(cd) / denominator)
}

fn trim(
    doc: &Document,
    stroke: &Stroke,
    target: Target,
    index: usize,
    at: f64,
    mut next_id: u64,
) -> Result<Preview, String> {
    let last = (stroke.points.len() - 1) as f64;
    if !(0.0..=last).contains(&at) {
        return Err("Pick a section of the sampled stroke".into());
    }
    let mut low = 0.0_f64;
    let mut high = last;
    let mut tests = 0;
    let source: Vec<_> = stroke
        .points
        .iter()
        .map(|p| target.chart(p.position))
        .collect();
    for [c, d] in boundaries(doc, stroke, target) {
        for (segment, pair) in source.windows(2).enumerate() {
            tests += 1;
            if tests > MAX_INTERSECTION_TESTS {
                return Err("Trim exceeds the intersection work limit".into());
            }
            let ab = pair[1] - pair[0];
            let cd = d - c;
            if ab.length_squared() <= EPS * EPS || cd.length_squared() <= EPS * EPS {
                continue;
            }
            let Some(t) = intersection(pair[0], ab, c, cd) else {
                if overlapping(pair[0], ab, c, d, 1.0) {
                    return Err("Overlapping strokes make this trim ambiguous".into());
                }
                continue;
            };
            if !(-EPS..=1.0 + EPS).contains(&t) {
                continue;
            }
            let hit = segment as f64 + t.clamp(0.0, 1.0);
            if hit <= EPS || hit >= last - EPS {
                continue;
            }
            if (at - hit).abs() <= EPS {
                return Err("Click to one side of the intersection".into());
            }
            if hit < at {
                low = low.max(hit);
            } else {
                high = high.min(hit);
            }
        }
    }
    if low == 0.0 && high == last {
        return Err("No eligible intersection on the active plane".into());
    }
    let cuts: Vec<_> = (0..stroke.points.len() - 1)
        .map(|i| {
            let start = (low - i as f64).clamp(0.0, 1.0);
            let end = (high - i as f64).clamp(0.0, 1.0);
            if end - start > EPS {
                vec![(start as f32, end as f32)]
            } else {
                Vec::new()
            }
        })
        .collect();
    let mut after = Vec::new();
    for (offset, mut fragment) in retained_fragments(stroke, &cuts).into_iter().enumerate() {
        if offset != 0 {
            if next_id == u64::MAX || doc.strokes.len() >= 100_000 {
                return Err("The document has no room for a second stroke fragment".into());
            }
            fragment.id = next_id;
            next_id += 1;
        }
        after.push((index + offset, fragment));
    }
    if after.is_empty() {
        return Err("The trim would leave no usable stroke".into());
    }
    let retained_points = after
        .iter()
        .map(|(_, fragment)| fragment.points.len())
        .sum::<usize>();
    if retained_points > stroke.points.len()
        && doc
            .strokes
            .iter()
            .map(|stroke| stroke.points.len())
            .sum::<usize>()
            - stroke.points.len()
            + retained_points
            > 2_000_000
    {
        return Err("The trim would exceed the document point limit".into());
    }
    let left = point_at(stroke, low).position;
    let right = point_at(stroke, high).position;
    let mut path = vec![left];
    path.extend(
        stroke.points.iter().enumerate().filter_map(|(i, point)| {
            (low < i as f64 && (i as f64) < high).then_some(point.position)
        }),
    );
    path.push(right);
    let joins = [(low > 0.0, left), (high < last, right)]
        .into_iter()
        .filter_map(|(is_join, point)| is_join.then_some(point))
        .collect();
    Ok(Preview {
        after,
        next_id,
        path,
        joins,
    })
}

fn overlapping(a: DVec2, ab: DVec2, c: DVec2, d: DVec2, length: f64) -> bool {
    if (c - a).perp_dot(ab).abs().max((d - a).perp_dot(ab).abs()) > EPS * ab.length() {
        return false;
    }
    let first = (c - a).dot(ab) / ab.length_squared();
    let second = (d - a).dot(ab) / ab.length_squared();
    first.max(second).min(length) - first.min(second).max(0.0) > EPS
}

fn point_at(stroke: &Stroke, at: f64) -> Point {
    let i = (at.floor() as usize).min(stroke.points.len() - 2);
    interpolate_point(
        &stroke.points[i],
        &stroke.points[i + 1],
        (at - i as f64) as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: u64, target: Target, a: Vec3, b: Vec3) -> Stroke {
        let (u, v, n) = target.plane.basis();
        Stroke {
            id,
            layer_id: 1,
            color: [0.2; 3],
            radius: 0.01,
            brush: Default::default(),
            points: [a, b]
                .into_iter()
                .zip([0.2, 0.8])
                .map(|(p, pressure)| Point {
                    position: target.plane.origin() + u * p.x + v * p.y + n * p.z,
                    pressure,
                    tilt: None,
                    time_ms: 0,
                })
                .collect(),
            guide_attachment: None,
            model_surface: None,
            curve: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
        }
    }

    #[test]
    fn trim_preserves_interpolated_fragments_and_ignores_other_depths() {
        let target = Target {
            plane: DrawingPlane {
                anchor: Vec3::new(2.0, 1.0, 0.0),
                rotation_degrees: Vec3::new(25.0, 40.0, 15.0),
                depth: 0.0,
            },
            half_extent: None,
        };
        let mut doc = Document::default();
        let source = line(1, target, Vec3::ZERO, Vec3::X * 4.0);
        doc.strokes.push(source.clone());
        for (id, x, depth) in [(2, 1.0, 0.0), (3, 3.0, 0.0), (4, 1.9, 0.01)] {
            doc.strokes.push(line(
                id,
                target,
                Vec3::new(x, -1.0, depth),
                Vec3::new(x, 1.0, depth),
            ));
        }
        let middle = preview(&doc, Action::Trim, target, 0, 0.5, 5).unwrap();
        assert_eq!(middle.after.len(), 2);
        assert_eq!(middle.after[0].1.id, 1);
        assert_eq!(middle.after[1].1.id, 5);
        assert!((middle.after[0].1.points[1].pressure - 0.35).abs() < 1e-5);
        assert!((middle.after[1].1.points[0].pressure - 0.65).abs() < 1e-5);
        let end = preview(&doc, Action::Trim, target, 0, 0.1, 5).unwrap();
        assert_eq!(end.after.len(), 1);
        assert_eq!(end.after[0].1.points[1], source.points[1]);
        let edit = crate::editing::Edit::Change {
            before: vec![(0, source.clone())],
            after: middle.after,
        };
        edit.apply(&mut doc, true);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes[0], source);
    }
}
