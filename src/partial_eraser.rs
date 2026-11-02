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
    let distances = crate::dynamics::arc_lengths(&stroke.points);
    let mut fragments: Vec<(Vec<Point>, f32)> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    let mut current_origin = None;
    for (index, intervals) in cuts.iter().enumerate() {
        let mut cursor = 0.0;
        for &(cut_start, cut_end) in intervals {
            if cut_start > cursor + 0.000001 {
                append_interval(
                    &mut current,
                    &mut current_origin,
                    stroke.brush.dab_distance_origin + distances[index],
                    &stroke.points[index],
                    &stroke.points[index + 1],
                    cursor,
                    cut_start,
                );
            }
            if current.len() >= 2 {
                fragments.push((
                    std::mem::take(&mut current),
                    current_origin
                        .take()
                        .unwrap_or(stroke.brush.dab_distance_origin),
                ));
            } else {
                current.clear();
                current_origin = None;
            }
            cursor = cursor.max(cut_end);
        }
        if cursor < 1.0 - 0.000001 {
            append_interval(
                &mut current,
                &mut current_origin,
                stroke.brush.dab_distance_origin + distances[index],
                &stroke.points[index],
                &stroke.points[index + 1],
                cursor,
                1.0,
            );
        }
        if intervals.is_empty() && current.last() != Some(&stroke.points[index + 1]) {
            if current.is_empty() {
                current_origin = Some(stroke.brush.dab_distance_origin + distances[index + 1]);
            }
            current.push(stroke.points[index + 1]);
        }
    }
    if current.len() >= 2 {
        fragments.push((
            current,
            current_origin.unwrap_or(stroke.brush.dab_distance_origin),
        ));
    }
    fragments
        .into_iter()
        .filter_map(|(mut points, origin)| {
            deduplicate(&mut points);
            (points.len() >= 2).then(|| {
                let mut fragment = stroke.clone();
                fragment.points = points;
                fragment.curve = None;
                if fragment.brush.dabs.enabled {
                    fragment.brush.dab_distance_origin = origin;
                }
                fragment
            })
        })
        .collect()
}

fn append_interval(
    points: &mut Vec<Point>,
    source_origin: &mut Option<f32>,
    segment_origin: f32,
    a: &Point,
    b: &Point,
    start: f32,
    end: f32,
) {
    if end <= start + 0.000001 {
        return;
    }
    let first = interpolate_point(a, b, start);
    let last = interpolate_point(a, b, end);
    if points.is_empty() {
        let segment_length = a.position.distance(b.position);
        *source_origin = Some(segment_origin + segment_length * start);
    }
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
    let tilt = crate::dynamics::interpolate_tilt(a.tilt, b.tilt, t);
    Point {
        position: a.position.lerp(b.position, t),
        pressure: a.pressure + (b.pressure - a.pressure) * t,
        tilt,
        time_ms: (a.time_ms as f32 + (b.time_ms as f32 - a.time_ms as f32) * t).round() as u32,
    }
}

fn deduplicate(points: &mut Vec<Point>) {
    points.dedup_by(|right, left| {
        right.position.distance_squared(left.position) <= 1.0e-12
            && (right.pressure - left.pressure).abs() <= 1.0e-6
            && right.tilt == left.tilt
            && right.time_ms == left.time_ms
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, StrokeMaterial};
    use crate::geometry::Projection;
    use eframe::egui::Vec2;

    fn canvas() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))
    }

    fn point(camera: &Camera, canvas: Rect, world: Vec3) -> Pos2 {
        gizmo::project(camera, canvas, world).unwrap().0
    }

    fn source_stroke(points: Vec<Point>) -> Stroke {
        let mut brush = Brush {
            kind: crate::document::BrushKind::Ribbon,
            normal: Vec3::Y,
            tangent: Vec3::X,
            ..Brush::default()
        };
        brush.texture.kind = crate::texture::TextureKind::Pattern;
        brush.texture.seed = 29;
        Stroke {
            curve: None,
            id: 5,
            color: [0.2, 0.4, 0.7],
            radius: 0.025,
            brush,
            points,
            fill: None,
            material: StrokeMaterial::Emissive,
            group_id: Some(13),
            layer_id: 1,
        }
    }

    fn line(points: [Vec3; 2]) -> Stroke {
        source_stroke(
            points
                .into_iter()
                .enumerate()
                .map(|(index, position)| Point {
                    position,
                    pressure: if index == 0 { 0.25 } else { 0.75 },
                    tilt: Some(if index == 0 { Vec3::X } else { Vec3::Y }),

                    time_ms: 0,
                })
                .collect(),
        )
    }

    fn front_camera() -> Camera {
        Camera {
            projection: Projection::Perspective,
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        }
    }

    #[test]
    fn split_fragments_keep_interpolated_contacts_and_stroke_metadata_through_history() {
        let camera = front_camera();
        let canvas = canvas();
        let source = line([Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)]);
        let mut doc = crate::document::Document {
            strokes: vec![source.clone()],
            ..Default::default()
        };
        let center = point(&camera, canvas, Vec3::ZERO);
        let path = [center - Vec2::Y * 2.0, center + Vec2::Y * 2.0];
        let plan = plan_partial_erase(&doc, &camera, canvas, &path, 5.0, 100)
            .unwrap()
            .unwrap();
        assert_eq!(plan.split, 1);
        assert_eq!(plan.removed, 0);
        assert_eq!(plan.next_id, 101);
        let expected_after = match &plan.edit {
            Edit::Change { after, .. } => after.clone(),
            _ => panic!("partial erasing must produce one reversible change"),
        };

        plan.edit.apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 2);
        assert_eq!(doc.strokes[0].id, source.id);
        assert_eq!(doc.strokes[1].id, 100);
        for fragment in &doc.strokes {
            assert_eq!(fragment.color, source.color);
            assert_eq!(fragment.radius, source.radius);
            assert_eq!(fragment.brush, source.brush);
            assert_eq!(fragment.material, source.material);
            assert_eq!(fragment.group_id, source.group_id);
            assert_eq!(fragment.layer_id, source.layer_id);
            assert_eq!(fragment.fill, source.fill);
        }
        let left = &doc.strokes[0];
        let right = &doc.strokes[1];
        assert!(left.points.first().unwrap().position.x < -1.9);
        assert!(left.points.last().unwrap().position.x < 0.0);
        assert!(right.points.first().unwrap().position.x > 0.0);
        assert!(right.points.last().unwrap().position.x > 1.9);
        assert!(
            right.points.first().unwrap().position.x - left.points.last().unwrap().position.x > 0.1
        );
        for boundary in [left.points.last().unwrap(), right.points.first().unwrap()] {
            assert!((0.25..=0.75).contains(&boundary.pressure));
            let tilt = boundary
                .tilt
                .expect("tilt should interpolate at split boundaries");
            assert!(tilt.length() > 0.0 && tilt.length() <= 1.0);
        }

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let reopened =
            crate::document::Document::from_json(&std::fs::read_to_string(file.path()).unwrap())
                .unwrap();
        assert_eq!(reopened.strokes, doc.strokes);

        let glb = crate::export::encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&glb.bytes).unwrap();
        let exported: Vec<_> = gltf
            .meshes()
            .map(|mesh| {
                let primitive = mesh.primitives().next().unwrap();
                primitive
                    .reader(|_| gltf.blob.as_deref())
                    .read_indices()
                    .unwrap()
                    .into_u32()
                    .count()
            })
            .collect();
        let retained: Vec<_> = doc
            .strokes
            .iter()
            .map(|stroke| crate::geometry::stroke_mesh(stroke).len())
            .collect();
        assert_eq!(
            exported, retained,
            "GLB should contain both exact retained fragments"
        );

        let recovery_dir = tempfile::tempdir().unwrap();
        let mut recovery = crate::recovery::Recovery::new(
            Ok(recovery_dir.path().into()),
            eframe::egui::Context::default(),
        );
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, None));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while recovery.busy() && std::time::Instant::now() < deadline {
            recovery.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        recovery.poll();
        assert!(
            !recovery.busy(),
            "recovery worker should finish the snapshot"
        );
        drop(recovery);
        let recovered = crate::recovery_store::scan(recovery_dir.path(), "next-session").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes,
            doc.strokes
        );

        plan.edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, vec![source.clone()]);
        plan.edit.apply(&mut doc, true);
        assert_eq!(
            doc.strokes,
            expected_after
                .iter()
                .map(|(_, stroke)| stroke.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn taps_repeated_cuts_and_complete_erasure_have_exact_results() {
        let camera = front_camera();
        let canvas = canvas();
        let center = point(&camera, canvas, Vec3::ZERO);

        let dot = source_stroke(vec![Point {
            position: Vec3::ZERO,
            pressure: 0.6,
            tilt: Some(Vec3::Z),

            time_ms: 0,
        }]);
        let mut dot_doc = crate::document::Document {
            strokes: vec![dot.clone()],
            ..Default::default()
        };
        let tap = plan_partial_erase(&dot_doc, &camera, canvas, &[center], 5.0, 100)
            .unwrap()
            .unwrap();
        assert_eq!(tap.removed, 1);
        tap.edit.apply(&mut dot_doc, true);
        assert!(dot_doc.strokes.is_empty());
        tap.edit.apply(&mut dot_doc, false);
        assert_eq!(dot_doc.strokes, vec![dot]);

        let source = line([Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)]);
        let mut repeated_doc = crate::document::Document {
            strokes: vec![source],
            ..Default::default()
        };
        let left = point(&camera, canvas, Vec3::new(-0.6, 0.0, 0.0));
        let right = point(&camera, canvas, Vec3::new(0.6, 0.0, 0.0));
        let repeated_path = [
            left - Vec2::Y * 10.0,
            left + Vec2::Y * 10.0,
            right + Vec2::Y * 10.0,
            right - Vec2::Y * 10.0,
        ];
        let repeated = plan_partial_erase(&repeated_doc, &camera, canvas, &repeated_path, 3.0, 100)
            .unwrap()
            .unwrap();
        assert_eq!(repeated.split, 1);
        assert_eq!(repeated.next_id, 102);
        repeated.edit.apply(&mut repeated_doc, true);
        assert_eq!(repeated_doc.strokes.len(), 3);
        assert!(repeated_doc.strokes.windows(2).all(|fragments| {
            fragments[0].points.last().unwrap().position.x
                < fragments[1].points.first().unwrap().position.x
        }));

        let short = line([Vec3::new(-0.02, 0.0, 0.0), Vec3::new(0.02, 0.0, 0.0)]);
        let mut short_doc = crate::document::Document {
            strokes: vec![short],
            ..Default::default()
        };
        let complete = plan_partial_erase(&short_doc, &camera, canvas, &[center], 10.0, 100)
            .unwrap()
            .unwrap();
        assert_eq!(complete.removed, 1);
        complete.edit.apply(&mut short_doc, true);
        assert!(short_doc.strokes.is_empty());
        complete.edit.apply(&mut short_doc, false);
        assert_eq!(short_doc.strokes.len(), 1);
    }
}
