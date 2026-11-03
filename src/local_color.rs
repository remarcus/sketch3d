//! Bounded local color painting for ordinary sampled strokes.
//!
//! A gesture is evaluated once against its complete screen-space path. Each
//! affected sample receives one strength/falloff blend, so repainting a preview
//! at a different pointer event rate does not change the result. Painted RGB is
//! a final linear albedo override; untouched samples continue to use the solid
//! color or two-stop gradient, and the material remains unchanged.

use crate::{
    document::{Document, Point, Stroke, MAX_LOCAL_COLOR_SAMPLES_PER_STROKE},
    editing::{Edit, Snapshot},
    geometry::Camera,
    gizmo,
    liquify::Falloff,
};
use eframe::egui::{Pos2, Rect};
use std::collections::{HashMap, HashSet};

pub const MAX_PAINT_PATH_POINTS: usize = 1024;
pub const MAX_PAINT_WORK: usize = 8_000_000;
pub const MAX_PAINT_RADIUS: f32 = 250.0;
const MAX_VISIBILITY_SAMPLES: usize = 2_000_000;
const VISIBILITY_CELL: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub tint: [f32; 3],
    pub radius_px: f32,
    pub strength: f32,
    pub falloff: Falloff,
    pub selected_only: bool,
    /// When enabled, projected centerline samples behind visible stroke
    /// samples are skipped. Hidden and locked layers are always protected.
    pub visible_only: bool,
}

impl Settings {
    pub fn validate(self) -> Result<(), String> {
        if self
            .tint
            .iter()
            .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
        {
            return Err("Tint components must be finite linear RGB values from 0 to 1".into());
        }
        if !self.radius_px.is_finite() || !(1.0..=MAX_PAINT_RADIUS).contains(&self.radius_px) {
            return Err(format!(
                "Tint radius must be between 1 and {MAX_PAINT_RADIUS} screen points"
            ));
        }
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err("Tint strength must be between 0 and 1".into());
        }
        Ok(())
    }
}

/// Plan a complete paint gesture as one reversible document edit. Radius and
/// pointer coordinates use logical screen points; color values use linear RGB.
pub fn paint(
    doc: &Document,
    camera: &Camera,
    canvas: Rect,
    path: &[Pos2],
    selected: &HashSet<u64>,
    settings: Settings,
) -> Result<Option<Edit>, String> {
    settings.validate()?;
    if path.is_empty() || path.len() > MAX_PAINT_PATH_POINTS {
        return Err(format!(
            "Tint path must contain 1–{MAX_PAINT_PATH_POINTS} points"
        ));
    }
    if path
        .iter()
        .any(|position| !position.x.is_finite() || !position.y.is_finite())
    {
        return Err("Tint path contains an invalid screen position".into());
    }
    if canvas.width() <= 0.0 || canvas.height() <= 0.0 {
        return Err("Tint painting needs a visible canvas".into());
    }
    if settings.strength == 0.0 || (settings.selected_only && selected.is_empty()) {
        return Ok(None);
    }
    let work: usize = doc
        .strokes
        .iter()
        .filter(|stroke| paint_eligible(doc, stroke, selected, settings.selected_only))
        .map(|stroke| {
            stroke
                .points
                .len()
                .saturating_mul(path.len().saturating_sub(1).max(1))
        })
        .sum();
    if work > MAX_PAINT_WORK {
        return Err("Tint gesture exceeds the bounded stroke-sample work limit".into());
    }
    let visible_depth = if settings.visible_only {
        Some(visible_depth_grid(doc, camera, canvas)?)
    } else {
        None
    };
    let linked_ids = doc.linked_generated_ids();
    let spacing = settings.radius_px.clamp(1.0, 4.0);
    let mut before: Snapshot = Vec::new();
    let mut after: Snapshot = Vec::new();
    let mut document_point_count: usize =
        doc.strokes.iter().map(|stroke| stroke.points.len()).sum();
    let mut resampled_work = 0usize;

    for (stroke_index, stroke) in doc.strokes.iter().enumerate() {
        if !paint_eligible(doc, stroke, selected, settings.selected_only)
            || linked_ids.contains(&stroke.id)
            || doc.has_linked_source(stroke.id)
        {
            continue;
        }
        if stroke.points.len() > MAX_LOCAL_COLOR_SAMPLES_PER_STROKE {
            continue;
        }
        let points = resample_for_paint(stroke, camera, canvas, spacing)?;
        if points.is_empty() || points.len() > MAX_LOCAL_COLOR_SAMPLES_PER_STROKE {
            continue;
        }
        resampled_work = resampled_work.saturating_add(
            points
                .len()
                .saturating_mul(path.len().saturating_sub(1).max(1)),
        );
        if resampled_work > MAX_PAINT_WORK {
            return Err("Tint gesture exceeds the bounded resampled-point work limit".into());
        }
        let added_points = points.len().saturating_sub(stroke.points.len());
        if document_point_count.saturating_add(added_points) > 2_000_000 {
            return Err("Tint painting would exceed the 2,000,000 document sample limit".into());
        }
        let mut painted = stroke.clone();
        painted.point_thickness = crate::stroke_sculpt::remap_thickness_profile(stroke, &points);
        painted.point_colors = crate::stroke_sculpt::remap_point_colors(stroke, &points);
        painted.points = points;
        let original_colors = painted.point_colors.clone();
        let mut colors = painted
            .point_colors
            .take()
            .unwrap_or_else(|| vec![None; painted.points.len()]);
        let distances = crate::dynamics::arc_lengths(&painted.points);
        let total = distances.last().copied().unwrap_or(0.0);
        let mut touched = false;
        for (point_index, point) in painted.points.iter().enumerate() {
            let Some((position, depth)) = gizmo::project(camera, canvas, point.position) else {
                continue;
            };
            if visible_depth
                .as_ref()
                .is_some_and(|grid| occluded_by_visible_sample(grid, position, depth))
            {
                continue;
            }
            let distance = path_distance(position, path);
            let mark_radius_px = crate::geometry::stroke_extent(stroke)
                / camera.world_per_pixel(point.position, canvas.height());
            let influence_radius = settings.radius_px + mark_radius_px.max(0.0);
            if distance >= influence_radius {
                continue;
            }
            let influence =
                settings.strength * falloff(settings.falloff, distance / influence_radius);
            if influence <= f32::EPSILON {
                continue;
            }
            let t = if total > 1.0e-8 {
                distances[point_index] / total
            } else {
                0.0
            };
            let current = colors[point_index].unwrap_or_else(|| painted.base_color_at(t));
            let blended = std::array::from_fn(|channel| {
                current[channel] + (settings.tint[channel] - current[channel]) * influence
            });
            if colors[point_index] != Some(blended) {
                colors[point_index] = Some(blended);
                touched = true;
            }
        }
        if !touched {
            continue;
        }
        if colors.iter().all(Option::is_none) {
            painted.point_colors = None;
        } else {
            painted.point_colors = Some(colors);
        }
        if painted == *stroke {
            painted.point_colors = original_colors;
            continue;
        }
        document_point_count = document_point_count.saturating_add(added_points);
        before.push((stroke_index, stroke.clone()));
        after.push((stroke_index, painted));
    }
    Ok((!before.is_empty()).then_some(Edit::Change { before, after }))
}

/// Clear local paint on selected strokes or all editable visible strokes.
/// Solid color and gradient values remain unchanged.
pub fn reset(doc: &Document, selected: &HashSet<u64>, selected_only: bool) -> Option<Edit> {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let linked_ids = doc.linked_generated_ids();
    for (index, stroke) in doc.strokes.iter().enumerate() {
        if !paint_eligible(doc, stroke, selected, selected_only)
            || linked_ids.contains(&stroke.id)
            || doc.has_linked_source(stroke.id)
            || !stroke.has_local_colors()
        {
            continue;
        }
        let mut cleared = stroke.clone();
        cleared.point_colors = None;
        before.push((index, stroke.clone()));
        after.push((index, cleared));
    }
    (!before.is_empty()).then_some(Edit::Change { before, after })
}

fn paint_eligible(
    doc: &Document,
    stroke: &Stroke,
    selected: &HashSet<u64>,
    selected_only: bool,
) -> bool {
    doc.editable(stroke)
        && (!selected_only || selected.contains(&stroke.id))
        && stroke.fill.is_none()
        && stroke.curve.is_none()
}

fn resample_for_paint(
    stroke: &Stroke,
    camera: &Camera,
    canvas: Rect,
    spacing: f32,
) -> Result<Vec<Point>, String> {
    if stroke.points.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = Vec::with_capacity(stroke.points.len().min(4096));
    output.push(stroke.points[0]);
    for pair in stroke.points.windows(2) {
        let steps = gizmo::project(camera, canvas, pair[0].position)
            .zip(gizmo::project(camera, canvas, pair[1].position))
            .map_or(1, |(start, end)| {
                ((start.0.distance(end.0) / spacing).ceil() as usize).max(1)
            });
        if output.len().saturating_add(steps) > MAX_LOCAL_COLOR_SAMPLES_PER_STROKE {
            return Err(format!(
                "Tint painting is limited to {MAX_LOCAL_COLOR_SAMPLES_PER_STROKE} samples per stroke"
            ));
        }
        for step in 1..=steps {
            output.push(interpolate_point(
                pair[0],
                pair[1],
                step as f32 / steps as f32,
            ));
        }
    }
    Ok(output)
}

fn interpolate_point(start: Point, end: Point, t: f32) -> Point {
    let tilt = match (start.tilt, end.tilt) {
        (Some(a), Some(b)) => {
            let value = a.lerp(b, t);
            (value.length_squared() > 1.0e-8).then(|| value.normalize())
        }
        (Some(a), None) => (t < 0.5).then_some(a),
        (None, Some(b)) => (t >= 0.5).then_some(b),
        (None, None) => None,
    };
    Point {
        position: start.position.lerp(end.position, t),
        pressure: start.pressure + (end.pressure - start.pressure) * t,
        tilt,
        time_ms: (start.time_ms as f32 + (end.time_ms as f32 - start.time_ms as f32) * t)
            .round()
            .clamp(0.0, crate::document::MAX_FLOW_DURATION_MS as f32) as u32,
    }
}

fn path_distance(point: Pos2, path: &[Pos2]) -> f32 {
    if path.len() == 1 {
        return point.distance(path[0]);
    }
    path.windows(2)
        .map(|pair| {
            let edge = pair[1] - pair[0];
            let length_squared = edge.length_sq();
            let t = if length_squared > 1.0e-8 {
                ((point - pair[0]).dot(edge) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            point.distance(pair[0] + edge * t)
        })
        .fold(f32::INFINITY, f32::min)
}

fn falloff(kind: Falloff, normalized: f32) -> f32 {
    let t = normalized.clamp(0.0, 1.0);
    match kind {
        Falloff::Linear => 1.0 - t,
        Falloff::Smooth => 1.0 - t * t * (3.0 - 2.0 * t),
        Falloff::Sharp => (1.0 - t) * (1.0 - t),
    }
}

fn visible_depth_grid(
    doc: &Document,
    camera: &Camera,
    canvas: Rect,
) -> Result<HashMap<(i32, i32), f32>, String> {
    let mut grid = HashMap::new();
    let mut work = 0usize;
    for stroke in doc.strokes.iter().filter(|stroke| doc.visible(stroke)) {
        if stroke.points.len() == 1 {
            if let Some((position, depth)) =
                gizmo::project(camera, canvas, stroke.points[0].position)
            {
                insert_depth(&mut grid, position, depth);
                work += 1;
            }
            continue;
        }
        for pair in stroke.points.windows(2) {
            let Some((a, depth_a)) = gizmo::project(camera, canvas, pair[0].position) else {
                continue;
            };
            let Some((b, depth_b)) = gizmo::project(camera, canvas, pair[1].position) else {
                continue;
            };
            let steps = ((a.distance(b) / VISIBILITY_CELL).ceil() as usize).max(1);
            work = work.saturating_add(steps + 1);
            if work > MAX_VISIBILITY_SAMPLES {
                return Err("Visible-only tint exceeds the bounded visibility sample limit".into());
            }
            for step in 0..=steps {
                let t = step as f32 / steps as f32;
                insert_depth(
                    &mut grid,
                    a + (b - a) * t,
                    depth_a + (depth_b - depth_a) * t,
                );
            }
        }
    }
    Ok(grid)
}

fn insert_depth(grid: &mut HashMap<(i32, i32), f32>, position: Pos2, depth: f32) {
    let key = cell(position);
    grid.entry(key)
        .and_modify(|current| *current = current.min(depth))
        .or_insert(depth);
}

fn cell(position: Pos2) -> (i32, i32) {
    (
        (position.x / VISIBILITY_CELL).floor() as i32,
        (position.y / VISIBILITY_CELL).floor() as i32,
    )
}

fn occluded_by_visible_sample(grid: &HashMap<(i32, i32), f32>, position: Pos2, depth: f32) -> bool {
    let (x, y) = cell(position);
    (-1..=1).any(|dx| {
        (-1..=1).any(|dy| {
            grid.get(&(x + dx, y + dy))
                .is_some_and(|front| *front + 0.002 < depth)
        })
    })
}
