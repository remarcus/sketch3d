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

pub const MAX_REGION_VERTICES: usize = 4096;

/// The operation a brush gesture performs on its hit set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushSelectionOperation {
    /// Add every hit stroke (and its selection group) to the current set.
    Select,
    /// Remove every hit stroke (and its selection group) from the current set.
    Deselect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionSelectionOperation {
    Replace,
    Add,
    Subtract,
}

/// Find strokes intersecting a screen-space polygon, or entirely contained by it.
pub fn region_hit_ids(
    doc: &Document,
    camera: &Camera,
    canvas: Rect,
    region: &[Pos2],
    fully_contained: bool,
    through_selection: bool,
    individual: bool,
) -> Result<HashSet<u64>, &'static str> {
    const MAX_VERTICES: usize = 4096;
    const MAX_WORK: usize = 5_000_000;
    if !(3..=MAX_VERTICES).contains(&region.len())
        || region.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
        || canvas.width() <= 0.0
        || canvas.height() <= 0.0
    {
        return Err("Selection region needs 3–4096 finite vertices on a visible canvas");
    }
    let mut hits = HashSet::new();
    let mut work = 0usize;
    for source in doc.strokes.iter().filter(|stroke| doc.editable(stroke)) {
        let evaluated_copies = crate::stroke_modifiers::evaluated_strokes(source)
            .map_err(|_| "Region selection cannot evaluate a stroke's modifier stack")?;
        for evaluated in evaluated_copies {
            let stroke = evaluated.as_ref();
            if stroke.points.is_empty() {
                continue;
            }
            work = work.saturating_add(stroke.points.len());
            if work > MAX_WORK {
                return Err("Selection query exceeded its 5,000,000 geometry budget");
            }
            let projected: Option<Vec<_>> = stroke
                .points
                .iter()
                .map(|point| {
                    gizmo::project(camera, canvas, point.position)
                        .map(|(screen, _)| (screen, point.position))
                })
                .collect();
            let Some(projected) = projected else {
                continue;
            };
            let contour: Vec<_> = projected.iter().map(|entry| entry.0).collect();
            let extent = crate::geometry::stroke_extent(stroke);
            let mut witnesses = Vec::new();

            if stroke.fill.is_some() {
                if fully_contained {
                    let mut contained = contour.iter().all(|p| point_in_polygon(*p, region));
                    if contained {
                        for i in 0..contour.len() {
                            for j in 0..region.len() {
                                work = work.saturating_add(1);
                                if work > MAX_WORK {
                                    return Err(
                                        "Selection query exceeded its 5,000,000 intersection budget",
                                    );
                                }
                                if closest_segments(
                                    contour[i],
                                    contour[(i + 1) % contour.len()],
                                    region[j],
                                    region[(j + 1) % region.len()],
                                )
                                .0 <= 0.01
                                {
                                    contained = false;
                                    break;
                                }
                            }
                            if !contained {
                                break;
                            }
                        }
                    }
                    if contained {
                        witnesses.extend(contour.iter().copied());
                    }
                } else {
                    witnesses.extend(
                        contour
                            .iter()
                            .copied()
                            .filter(|p| point_in_polygon(*p, region)),
                    );
                    witnesses.extend(
                        region
                            .iter()
                            .copied()
                            .filter(|p| point_in_polygon(*p, &contour)),
                    );
                    for i in 0..contour.len() {
                        for j in 0..region.len() {
                            work += 1;
                            if work > MAX_WORK {
                                return Err(
                                    "Selection query exceeded its 5,000,000 intersection budget",
                                );
                            }
                            let hit = closest_segments(
                                contour[i],
                                contour[(i + 1) % contour.len()],
                                region[j],
                                region[(j + 1) % region.len()],
                            );
                            if hit.0 <= 0.01 {
                                witnesses.push(hit.1);
                            }
                        }
                    }
                }
            } else if fully_contained {
                let mut contained = true;
                for (screen, world) in &projected {
                    let radius_px =
                        extent / camera.world_per_pixel(*world, canvas.height()).max(1e-6);
                    if !point_in_polygon(*screen, region) {
                        contained = false;
                        break;
                    }
                    for edge in 0..region.len() {
                        work = work.saturating_add(1);
                        if work > MAX_WORK {
                            return Err("Selection query exceeded its 5,000,000 geometry budget");
                        }
                        if closest_point(region[edge], region[(edge + 1) % region.len()], *screen).0
                            <= radius_px
                        {
                            contained = false;
                            break;
                        }
                    }
                    if !contained {
                        break;
                    }
                }
                for pair in projected.windows(2) {
                    if !contained {
                        break;
                    }
                    let radius_px = extent
                        / camera
                            .world_per_pixel(pair[0].1, canvas.height())
                            .min(camera.world_per_pixel(pair[1].1, canvas.height()))
                            .max(1e-6);
                    for edge in 0..region.len() {
                        work = work.saturating_add(1);
                        if work > MAX_WORK {
                            return Err("Selection query exceeded its 5,000,000 geometry budget");
                        }
                        if closest_segments(
                            pair[0].0,
                            pair[1].0,
                            region[edge],
                            region[(edge + 1) % region.len()],
                        )
                        .0 <= radius_px
                        {
                            contained = false;
                            break;
                        }
                    }
                }
                if contained {
                    witnesses.extend(contour.iter().copied());
                }
            } else {
                let edge_count = region.len();
                if projected.len() == 1 {
                    let radius_px = extent
                        / camera
                            .world_per_pixel(projected[0].1, canvas.height())
                            .max(1e-6);
                    if point_in_polygon(contour[0], region)
                        || (0..region.len()).any(|edge| {
                            closest_point(
                                region[edge],
                                region[(edge + 1) % region.len()],
                                contour[0],
                            )
                            .0 <= radius_px
                        })
                    {
                        witnesses.push(contour[0]);
                    }
                } else {
                    for pair in projected.windows(2) {
                        let world_per_pixel =
                            camera.world_per_pixel(pair[0].1, canvas.height()).max(1e-6);
                        let radius_px = extent / world_per_pixel;
                        if point_in_polygon(pair[0].0, region) {
                            witnesses.push(pair[0].0);
                        }
                        for edge in 0..edge_count {
                            work += 1;
                            if work > MAX_WORK {
                                return Err(
                                    "Selection query exceeded its 5,000,000 intersection budget",
                                );
                            }
                            let closest = closest_segments(
                                pair[0].0,
                                pair[1].0,
                                region[edge],
                                region[(edge + 1) % edge_count],
                            );
                            if closest.0 <= radius_px {
                                witnesses.push(closest.1);
                            }
                        }
                    }
                    if point_in_polygon(*contour.last().unwrap(), region) {
                        witnesses.push(*contour.last().unwrap());
                    }
                }
            }
            let mut visible_hit = through_selection;
            if !visible_hit {
                for point in witnesses {
                    work = work.saturating_add(1);
                    if work > MAX_WORK {
                        return Err("Selection query exceeded its 5,000,000 geometry budget");
                    }
                    if gizmo::pick_region_visible(doc, camera, canvas, point)? == Some(stroke.id) {
                        visible_hit = true;
                        break;
                    }
                }
            }
            if !visible_hit {
                continue;
            }
            if individual {
                hits.insert(stroke.id);
            } else {
                hits.extend(doc.selection_for_stroke(stroke.id, false));
            }
        }
    }
    Ok(hits)
}

pub fn apply_region_selection(
    starting: &HashSet<u64>,
    hits: &HashSet<u64>,
    operation: RegionSelectionOperation,
) -> HashSet<u64> {
    let mut result = starting.clone();
    match operation {
        RegionSelectionOperation::Replace => result = hits.clone(),
        RegionSelectionOperation::Add => result.extend(hits.iter().copied()),
        RegionSelectionOperation::Subtract => result.retain(|id| !hits.contains(id)),
    }
    result
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
    for source in doc.strokes.iter().filter(|stroke| doc.editable(stroke)) {
        let Ok(evaluated_copies) = crate::stroke_modifiers::evaluated_strokes(source) else {
            continue;
        };
        for evaluated in evaluated_copies {
            let stroke = evaluated.as_ref();
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
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
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
                opacity: 1.0,
                blend_mode: crate::document::LayerBlendMode::Normal,
                mask_source: None,
                clip_to_below: false,
            },
            Layer {
                id: 3,
                name: "Hidden".into(),
                visible: false,
                locked: false,
                opacity: 1.0,
                blend_mode: crate::document::LayerBlendMode::Normal,
                mask_source: None,
                clip_to_below: false,
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
                    linked_stroke_groups: Default::default(),
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
            linked_stroke_groups: Default::default(),
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
