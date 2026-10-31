//! Bounded screen-space partial erasing for ordinary stroked curves.
//!
//! This module only calculates a reversible document edit. The caller should
//! retain the returned `Edit` as one history entry, apply it once on pointer-up,
//! and discard it on gesture cancellation. Filled marks intentionally use
//! whole-mark erasing because their contour is a single planar region.

use crate::{
    document::{Document, Point, Stroke},
    editing::{Edit, Snapshot},
    geometry::Camera,
    gizmo,
};
use eframe::egui::{Pos2, Rect};
use glam::Vec3;

const MAX_PATH_POINTS: usize = 1024;
const MAX_STROKES: usize = 100_000;
const MAX_DOCUMENT_POINTS: usize = 2_000_000;
const MAX_SEGMENT_PAIRS: usize = 8_000_000;
const BISECTION_STEPS: usize = 22;

/// A partial erase plan. `edit` replaces the affected strokes with their
/// retained fragments; `next_id` is the first unused ID after any new pieces.
pub struct PartialErasePlan {
    pub edit: Edit,
    pub next_id: u64,
    pub removed: usize,
    pub split: usize,
}

/// Calculate one reversible partial-erase operation.
///
/// The eraser radius and path use logical egui points. Stroke width is included
/// in hit tolerance, scaled at the local camera depth. Hidden and locked layers
/// are never modified. Filled marks are removed as whole marks if the path
/// touches their projected interior or outline. For ordinary strokes, projected
/// line segments are cut and all retained point attributes are interpolated.
///
/// Work is bounded by a point-pair budget; exceptionally large documents or
/// paths are rejected with an actionable error instead of stalling the UI.
pub fn plan_partial_erase(
    doc: &Document,
    camera: &Camera,
    canvas: Rect,
    path: &[Pos2],
    radius_px: f32,
    next_id: u64,
) -> Result<Option<PartialErasePlan>, String> {
    if path.is_empty() || path.len() > MAX_PATH_POINTS {
        return Err(format!(
            "Erase path must contain 1–{MAX_PATH_POINTS} points"
        ));
    }
    if path
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err("Erase path contains an invalid screen position".into());
    }
    if !radius_px.is_finite() || !(1.0..=250.0).contains(&radius_px) {
        return Err("Eraser radius must be between 1 and 250 screen points".into());
    }
    if canvas.width() <= 0.0 || canvas.height() <= 0.0 {
        return Err("Partial erasing needs a visible canvas".into());
    }
    let point_count: usize = doc.strokes.iter().map(|stroke| stroke.points.len()).sum();
    if doc.strokes.len() > MAX_STROKES || point_count > MAX_DOCUMENT_POINTS {
        return Err("Document exceeds the partial-eraser work limits".into());
    }
    let erase_segments = path_segments(path);
    let eligible_segments: usize = doc
        .strokes
        .iter()
        .filter(|stroke| doc.editable(stroke) && stroke.fill.is_none())
        .map(|stroke| stroke.points.len().saturating_sub(1))
        .sum();
    if eligible_segments.saturating_mul(erase_segments.len()) > MAX_SEGMENT_PAIRS {
        return Err(
            "Erase gesture is too long for this drawing; shorten the gesture or erase in parts"
                .into(),
        );
    }

    let mut before: Snapshot = Vec::new();
    let mut changed_fragments: Vec<(usize, Vec<Stroke>)> = Vec::new();
    for (index, stroke) in doc.strokes.iter().enumerate() {
        if !doc.editable(stroke) || stroke.points.is_empty() {
            continue;
        }
        if stroke.fill.is_some() {
            if fill_hit(stroke, camera, canvas, &erase_segments, radius_px) {
                before.push((index, stroke.clone()));
                changed_fragments.push((index, Vec::new()));
            }
            continue;
        }
        let Some(projected) = project_points(stroke, camera, canvas) else {
            continue;
        };
        if stroke.points.len() == 1 {
            let world = stroke.points[0].position;
            let Some((screen, _)) = projected.first() else {
                continue;
            };
            let width = crate::geometry::stroke_extent(stroke)
                / camera.world_per_pixel(world, canvas.height());
            if erase_segments
                .iter()
                .any(|(a, b)| point_segment_distance(*screen, *a, *b).0 <= radius_px + width)
            {
                before.push((index, stroke.clone()));
                changed_fragments.push((index, Vec::new()));
            }
            continue;
        }

        let mut cuts = Vec::with_capacity(stroke.points.len() - 1);
        let mut touched = false;
        for segment_index in 0..stroke.points.len() - 1 {
            let (screen_a, _) = projected[segment_index];
            let (screen_b, _) = projected[segment_index + 1];
            let world_a = stroke.points[segment_index].position;
            let world_b = stroke.points[segment_index + 1].position;
            let world_mid = world_a.lerp(world_b, 0.5);
            let threshold = radius_px
                + crate::geometry::stroke_extent(stroke)
                    / camera.world_per_pixel(world_mid, canvas.height());
            let intervals = erased_intervals(screen_a, screen_b, &erase_segments, threshold);
            touched |= !intervals.is_empty();
            cuts.push(intervals);
        }
        if touched {
            let fragments = retained_fragments(stroke, &cuts);
            before.push((index, stroke.clone()));
            changed_fragments.push((index, fragments));
        }
    }
    if before.is_empty() {
        return Ok(None);
    }

    let mut next_id = next_id;
    let mut after: Snapshot = Vec::new();
    let mut index_delta: isize = 0;
    let mut removed = 0;
    let mut split = 0;
    for (original_index, mut fragments) in changed_fragments {
        if fragments.is_empty() {
            removed += 1;
            index_delta -= 1;
            continue;
        }
        if fragments.len() > 1 {
            split += 1;
        }
        let base_index = original_index.saturating_add_signed(index_delta);
        for (fragment_index, stroke) in fragments.iter_mut().enumerate() {
            if fragment_index > 0 {
                if next_id == u64::MAX {
                    return Err("No more stroke IDs available for partial erase".into());
                }
                stroke.id = next_id;
                next_id += 1;
            }
            after.push((base_index + fragment_index, stroke.clone()));
        }
        index_delta += fragments.len() as isize - 1;
    }

    Ok(Some(PartialErasePlan {
        edit: Edit::Change { before, after },
        next_id,
        removed,
        split,
    }))
}

fn path_segments(path: &[Pos2]) -> Vec<(Pos2, Pos2)> {
    if path.len() == 1 {
        vec![(path[0], path[0])]
    } else {
        path.windows(2).map(|pair| (pair[0], pair[1])).collect()
    }
}

fn project_points(stroke: &Stroke, camera: &Camera, canvas: Rect) -> Option<Vec<(Pos2, f32)>> {
    stroke
        .points
        .iter()
        .map(|point| gizmo::project(camera, canvas, point.position))
        .collect()
}

fn erased_intervals(a: Pos2, b: Pos2, eraser: &[(Pos2, Pos2)], radius: f32) -> Vec<(f32, f32)> {
    let mut intervals = Vec::new();
    for &(c, d) in eraser {
        let (minimum, closest) = closest_segments(a, b, c, d);
        if minimum > radius {
            continue;
        }
        let left = if distance_at(a, b, c, d, closest) <= radius {
            let mut lo = 0.0;
            let mut hi = closest;
            for _ in 0..BISECTION_STEPS {
                let mid = (lo + hi) * 0.5;
                if distance_at(a, b, c, d, mid) <= radius {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            hi
        } else {
            closest
        };
        let right = if distance_at(a, b, c, d, closest) <= radius {
            let mut lo = closest;
            let mut hi = 1.0;
            for _ in 0..BISECTION_STEPS {
                let mid = (lo + hi) * 0.5;
                if distance_at(a, b, c, d, mid) <= radius {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        } else {
            closest
        };
        intervals.push((left.clamp(0.0, 1.0), right.clamp(0.0, 1.0)));
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::with_capacity(intervals.len());
    for interval in intervals {
        if let Some(last) = merged
            .last_mut()
            .filter(|last| interval.0 <= last.1 + 0.00001)
        {
            last.1 = last.1.max(interval.1);
        } else {
            merged.push(interval);
        }
    }
    merged
}

fn distance_at(a: Pos2, b: Pos2, c: Pos2, d: Pos2, t: f32) -> f32 {
    let point = a + (b - a) * t;
    point_segment_distance(point, c, d).0
}

/// Return (distance, parameter on the first segment) for two 2D segments.
fn closest_segments(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> (f32, f32) {
    if let Some((ta, _)) = intersection_parameters(a, b, c, d) {
        return (0.0, ta);
    }
    let candidates = [
        {
            let (distance, t) = point_segment_distance(c, a, b);
            (distance, t)
        },
        {
            let (distance, t) = point_segment_distance(d, a, b);
            (distance, t)
        },
        {
            let (distance, _) = point_segment_distance(a, c, d);
            (distance, 0.0)
        },
        {
            let (distance, _) = point_segment_distance(b, c, d);
            (distance, 1.0)
        },
    ];
    candidates
        .into_iter()
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .unwrap_or((f32::INFINITY, 0.0))
}

fn point_segment_distance(point: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let delta = b - a;
    let t = ((point - a).dot(delta) / delta.length_sq().max(0.0001)).clamp(0.0, 1.0);
    (point.distance(a + delta * t), t)
}

fn intersection_parameters(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> Option<(f32, f32)> {
    let ab = b - a;
    let cd = d - c;
    let denominator = ab.x * cd.y - ab.y * cd.x;
    if denominator.abs() < 0.00001 {
        return None;
    }
    let offset = c - a;
    let ta = (offset.x * cd.y - offset.y * cd.x) / denominator;
    let tb = (offset.x * ab.y - offset.y * ab.x) / denominator;
    ((0.0..=1.0).contains(&ta) && (0.0..=1.0).contains(&tb)).then_some((ta, tb))
}

fn retained_fragments(stroke: &Stroke, cuts: &[Vec<(f32, f32)>]) -> Vec<Stroke> {
    let mut fragments: Vec<Vec<Point>> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    for (index, intervals) in cuts.iter().enumerate() {
        let mut cursor = 0.0;
        for &(cut_start, cut_end) in intervals {
            if cut_start > cursor + 0.000001 {
                append_interval(
                    &mut current,
                    &stroke.points[index],
                    &stroke.points[index + 1],
                    cursor,
                    cut_start,
                );
            }
            if current.len() >= 2 {
                fragments.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            cursor = cursor.max(cut_end);
        }
        if cursor < 1.0 - 0.000001 {
            append_interval(
                &mut current,
                &stroke.points[index],
                &stroke.points[index + 1],
                cursor,
                1.0,
            );
        }
        if intervals.is_empty() && current.last() != Some(&stroke.points[index + 1]) {
            current.push(stroke.points[index + 1]);
        }
    }
    if current.len() >= 2 {
        fragments.push(current);
    }
    fragments
        .into_iter()
        .filter_map(|mut points| {
            deduplicate(&mut points);
            (points.len() >= 2).then(|| {
                let mut fragment = stroke.clone();
                fragment.points = points;
                fragment
            })
        })
        .collect()
}

fn append_interval(points: &mut Vec<Point>, a: &Point, b: &Point, start: f32, end: f32) {
    if end <= start + 0.000001 {
        return;
    }
    let first = interpolate_point(a, b, start);
    let last = interpolate_point(a, b, end);
    if points.last() != Some(&first) {
        points.push(first);
    }
    if points.last() != Some(&last) {
        points.push(last);
    }
}

fn interpolate_point(a: &Point, b: &Point, t: f32) -> Point {
    if t <= 0.000001 {
        return *a;
    }
    if t >= 0.999999 {
        return *b;
    }
    let tilt = match (a.tilt, b.tilt) {
        (Some(left), Some(right)) => (left.lerp(right, t)).try_normalize(),
        _ => None,
    };
    Point {
        position: a.position.lerp(b.position, t),
        pressure: a.pressure + (b.pressure - a.pressure) * t,
        tilt,
    }
}

fn deduplicate(points: &mut Vec<Point>) {
    points.dedup_by(|right, left| {
        right.position.distance_squared(left.position) <= 1.0e-12
            && (right.pressure - left.pressure).abs() <= 1.0e-6
            && right.tilt == left.tilt
    });
}

fn fill_hit(
    stroke: &Stroke,
    camera: &Camera,
    canvas: Rect,
    eraser: &[(Pos2, Pos2)],
    radius: f32,
) -> bool {
    let Some(projected) = project_points(stroke, camera, canvas) else {
        return false;
    };
    let polygon: Vec<_> = projected.iter().map(|(position, _)| *position).collect();
    let world = stroke
        .points
        .first()
        .map(|point| point.position)
        .unwrap_or(Vec3::ZERO);
    let width =
        crate::geometry::stroke_extent(stroke) / camera.world_per_pixel(world, canvas.height());
    let threshold = radius + width;
    eraser.iter().any(|(a, b)| {
        point_in_polygon(*a, &polygon)
            || point_in_polygon(*b, &polygon)
            || polygon
                .windows(2)
                .any(|edge| segment_distance(*a, *b, edge[0], edge[1]) <= threshold)
            || (polygon.len() > 2
                && segment_distance(*a, *b, polygon[polygon.len() - 1], polygon[0]) <= threshold)
    })
}

fn segment_distance(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> f32 {
    closest_segments(a, b, c, d).0
}

fn point_in_polygon(point: Pos2, polygon: &[Pos2]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}
