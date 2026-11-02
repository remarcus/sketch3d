//! Screen-space brush selection shared by mouse and pen input.
//!
//! The caller supplies the pointer path in canvas coordinates, so this module
//! does not depend on an input device or own gesture state. A selection gesture
//! should retain its starting ID set and pass that same set to
//! [`apply_brush_selection`] on every preview frame; this makes cancellation a
//! simple restoration of the starting set.

use crate::{document::Document, geometry::Camera, gizmo};
use eframe::egui::{Pos2, Rect};
use std::collections::HashSet;

/// The operation a brush gesture performs on its hit set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushSelectionOperation {
    /// Add every hit stroke (and its selection group) to the current set.
    Select,
    /// Remove every hit stroke (and its selection group) from the current set.
    Deselect,
}

/// Find editable strokes intersected by a screen-space brush path.
///
/// `path` and `radius_px` are in logical egui points. `through_selection` skips
/// occlusion checks; otherwise a hit only counts where the stroke is the
/// frontmost editable artwork according to the same picker used by click
/// selection. Hidden and locked layers are excluded in both modes. Filled
/// strokes are selectable along their contour, consistent with stroke picking.
///
/// The return set expands hits to their editable group members unless
/// `individual` is true. Invalid radii, empty paths, or non-finite coordinates
/// return a user-readable error instead of producing an accidental selection.
pub fn hit_ids(
    doc: &Document,
    camera: &Camera,
    canvas: Rect,
    path: &[Pos2],
    radius_px: f32,
    through_selection: bool,
    individual: bool,
) -> Result<HashSet<u64>, &'static str> {
    if path.is_empty() {
        return Err("Brush selection needs at least one pointer position");
    }
    if !radius_px.is_finite() || !(1.0..=250.0).contains(&radius_px) {
        return Err("Brush selection radius must be between 1 and 250 px");
    }
    if path.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err("Brush selection path contains an invalid position");
    }
    if canvas.width() <= 0.0 || canvas.height() <= 0.0 {
        return Err("Brush selection requires a visible canvas");
    }

    let mut hits = HashSet::new();
    for stroke in doc.strokes.iter().filter(|stroke| doc.editable(stroke)) {
        if stroke.points.is_empty() {
            continue;
        }
        let extent = crate::geometry::stroke_extent(stroke);
        let projected: Option<Vec<_>> = stroke
            .points
            .iter()
            .map(|point| {
                gizmo::project(camera, canvas, point.position)
                    .map(|(screen, _)| (screen, point.position))
            })
            .collect();
        // Skipping a partly clipped stroke is preferable to connecting points
        // across a camera-plane or clip-plane gap and selecting empty space.
        let Some(projected) = projected else { continue };
        if projected.is_empty() {
            continue;
        }

        let mut witness = None;
        if stroke.fill.is_some() {
            // A filled mark is treated as a selectable closed outline. The
            // interior also counts, using the candidate's ray hit as witness.
            let contour: Vec<_> = projected.iter().map(|(screen, _)| *screen).collect();
            for segment in path_segments(path) {
                if point_in_polygon(segment.0, &contour) {
                    witness = Some(segment.0);
                    break;
                }
                if let Some(candidate) = closest_path_to_polyline(segment, &projected, true) {
                    if candidate.distance <= radius_px {
                        witness = Some(candidate.stroke_screen);
                        break;
                    }
                }
            }
        } else if projected.len() == 1 {
            for (start, end) in path_segments(path) {
                let (distance, _) = closest_point(start, end, projected[0].0);
                let width = extent / camera.world_per_pixel(projected[0].1, canvas.height());
                if distance <= radius_px + width {
                    witness = Some(projected[0].0);
                    break;
                }
            }
        } else {
            'path: for segment in path_segments(path) {
                if let Some(candidate) = closest_path_to_polyline(segment, &projected, false) {
                    let world = candidate.stroke_world;
                    let width = extent / camera.world_per_pixel(world, canvas.height());
                    if candidate.distance <= radius_px + width {
                        witness = Some(candidate.stroke_screen);
                        break 'path;
                    }
                }
            }
        }

        let Some(witness) = witness else { continue };
        if through_selection || gizmo::pick(doc, camera, canvas, witness) == Some(stroke.id) {
            if individual {
                hits.insert(stroke.id);
            } else {
                hits.extend(doc.selection_for_stroke(stroke.id, false));
            }
        }
    }
    Ok(hits)
}

/// Apply a brush hit set to a gesture's original selection snapshot.
///
/// Use `starting_selection` rather than a prior preview result on each frame;
/// doing so prevents repeated preview calls from accumulating stale hits. A
/// cancelled gesture should simply discard this result and restore that same
/// starting set. This function only changes IDs and never edits document data.
pub fn apply_brush_selection(
    starting_selection: &HashSet<u64>,
    hit_ids: &HashSet<u64>,
    operation: BrushSelectionOperation,
) -> HashSet<u64> {
    let mut result = starting_selection.clone();
    match operation {
        BrushSelectionOperation::Select => result.extend(hit_ids.iter().copied()),
        BrushSelectionOperation::Deselect => result.retain(|id| !hit_ids.contains(id)),
    }
    result
}

#[derive(Clone, Copy)]
struct Candidate {
    distance: f32,
    stroke_screen: Pos2,
    stroke_world: glam::Vec3,
}

fn path_segments(path: &[Pos2]) -> Vec<(Pos2, Pos2)> {
    if path.len() == 1 {
        return vec![(path[0], path[0])];
    }
    path.windows(2).map(|pair| (pair[0], pair[1])).collect()
}

fn closest_path_to_polyline(
    path: (Pos2, Pos2),
    stroke: &[(Pos2, glam::Vec3)],
    close: bool,
) -> Option<Candidate> {
    let mut best: Option<Candidate> = None;
    let edge_count = stroke.len().saturating_sub(1) + usize::from(close && stroke.len() > 2);
    for index in 0..edge_count {
        let a = &stroke[index];
        let b = &stroke[(index + 1) % stroke.len()];
        let (distance, stroke_screen, t) = closest_segments(path.0, path.1, a.0, b.0);
        if best.is_none_or(|old| distance < old.distance) {
            best = Some(Candidate {
                distance,
                stroke_screen,
                stroke_world: a.1.lerp(b.1, t),
            });
        }
    }
    best
}

fn closest_point(a: Pos2, b: Pos2, point: Pos2) -> (f32, Pos2) {
    let ab = b - a;
    let t = ((point - a).dot(ab) / ab.length_sq().max(0.0001)).clamp(0.0, 1.0);
    let closest = a + ab * t;
    (point.distance(closest), closest)
}

fn closest_segments(a0: Pos2, a1: Pos2, b0: Pos2, b1: Pos2) -> (f32, Pos2, f32) {
    if let Some((ta, tb)) = intersection_parameters(a0, a1, b0, b1) {
        let _ = ta;
        return (0.0, b0 + (b1 - b0) * tb, tb);
    }
    let candidates = [
        {
            let (d, _) = closest_point(a0, a1, b0);
            (d, b0, 0.0)
        },
        {
            let (d, _) = closest_point(a0, a1, b1);
            (d, b1, 1.0)
        },
        {
            let (d, p) = closest_point(b0, b1, a0);
            (d, p, parameter(b0, b1, p))
        },
        {
            let (d, p) = closest_point(b0, b1, a1);
            (d, p, parameter(b0, b1, p))
        },
    ];
    let (distance, point, t) = candidates
        .into_iter()
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .unwrap();
    (distance, point, t)
}

fn parameter(a: Pos2, b: Pos2, point: Pos2) -> f32 {
    let delta = b - a;
    ((point - a).dot(delta) / delta.length_sq().max(0.0001)).clamp(0.0, 1.0)
}

fn intersection_parameters(a0: Pos2, a1: Pos2, b0: Pos2, b1: Pos2) -> Option<(f32, f32)> {
    let a = a1 - a0;
    let b = b1 - b0;
    let denominator = a.x * b.y - a.y * b.x;
    if denominator.abs() < 0.00001 {
        return None;
    }
    let offset = b0 - a0;
    let ta = (offset.x * b.y - offset.y * b.x) / denominator;
    let tb = (offset.x * a.y - offset.y * a.x) / denominator;
    (0.0..=1.0)
        .contains(&ta)
        .then_some((ta, tb))
        .filter(|(_, tb)| (0.0..=1.0).contains(tb))
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
    use crate::document::{Brush, Layer, Point, Stroke};
    use crate::geometry::Projection;
    use eframe::egui::Vec2;
    use glam::Vec3;

    fn stroke(id: u64, layer_id: u64, group_id: Option<u64>, position: Vec3) -> Stroke {
        Stroke {
            curve: None,
            id,
            layer_id,
            group_id,
            fill: None,
            material: Default::default(),
            color: [0.2, 0.3, 0.4],
            radius: 0.001,
            brush: Brush::default(),
            points: vec![Point {
                position,
                pressure: 1.0,
                tilt: None,

                time_ms: 0,
            }],
        }
    }

    fn canvas() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))
    }

    fn screen_point(camera: &Camera, canvas: Rect, position: Vec3) -> Pos2 {
        crate::gizmo::project(camera, canvas, position)
            .expect("test points should be in the camera frustum")
            .0
    }

    #[test]
    fn visible_only_and_through_modes_respect_occlusion_and_protected_layers() {
        let camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        };
        let canvas = canvas();
        let mut doc = Document::default();
        doc.layers.extend([
            Layer {
                id: 2,
                name: "Locked".into(),
                visible: true,
                locked: true,
            },
            Layer {
                id: 3,
                name: "Hidden".into(),
                visible: false,
                locked: false,
            },
        ]);
        let point = Vec3::ZERO;
        doc.strokes = vec![
            stroke(1, 1, None, point - Vec3::Z),
            stroke(2, 1, None, point + Vec3::Z),
            stroke(3, 2, None, point + Vec3::Z * 2.0),
            stroke(4, 3, None, point + Vec3::Z * 3.0),
        ];
        let path = [screen_point(&camera, canvas, point)];

        assert_eq!(
            hit_ids(&doc, &camera, canvas, &path, 6.0, false, true).unwrap(),
            HashSet::from([2])
        );
        assert_eq!(
            hit_ids(&doc, &camera, canvas, &path, 6.0, true, true).unwrap(),
            HashSet::from([1, 2])
        );
    }

    #[test]
    fn screen_radius_is_stable_across_projection_modes_and_zoom_levels() {
        let canvas = canvas();
        for projection in [Projection::Perspective, Projection::Orthographic] {
            for distance in [4.0, 20.0] {
                let camera = Camera {
                    projection,
                    yaw: 0.0,
                    pitch: 0.0,
                    distance,
                    ..Camera::default()
                };
                let center = camera.target;
                let scale = camera.world_per_pixel(center, canvas.height());
                let right = camera.basis().0;
                let inside = center + right * (scale * 4.0);
                let outside = center + right * (scale * 12.0);
                let path = [screen_point(&camera, canvas, center)];
                let doc = Document {
                    strokes: vec![stroke(1, 1, None, inside), stroke(2, 1, None, outside)],
                    ..Document::default()
                };

                assert_eq!(
                    hit_ids(&doc, &camera, canvas, &path, 6.0, true, true).unwrap(),
                    HashSet::from([1]),
                    "radius should be interpreted in logical screen points for {projection:?} at distance {distance}"
                );
            }
        }
    }

    #[test]
    fn select_deselect_and_repeated_previews_only_change_ids_from_the_starting_set() {
        let doc = Document {
            strokes: vec![
                stroke(1, 1, Some(7), Vec3::ZERO),
                stroke(2, 1, Some(7), Vec3::X),
            ],
            ..Document::default()
        };
        let before = doc.strokes.clone();
        let starting = HashSet::from([9]);
        let hits = HashSet::from([1, 2]);

        let selected = apply_brush_selection(&starting, &hits, BrushSelectionOperation::Select);
        assert_eq!(selected, HashSet::from([1, 2, 9]));
        let deselected = apply_brush_selection(
            &HashSet::from([1, 2, 9]),
            &hits,
            BrushSelectionOperation::Deselect,
        );
        assert_eq!(deselected, HashSet::from([9]));
        assert_eq!(
            apply_brush_selection(&starting, &hits, BrushSelectionOperation::Select),
            selected,
            "each preview is derived from the original selection, not the previous preview"
        );
        assert_eq!(
            doc.strokes, before,
            "selection gestures must not edit geometry"
        );
        assert_eq!(
            starting,
            HashSet::from([9]),
            "cancel restores this starting set"
        );
    }
}
