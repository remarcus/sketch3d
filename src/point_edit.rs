//! Bounded edits to sampled strokes; semantic curves retain their own editor.
use crate::document::{Document, Stroke};
use crate::editing::{snapshot, Edit};
use glam::{Quat, Vec3};
use std::collections::HashSet;

pub const MAX_MULTI_STROKES: usize = 64;
pub const MAX_MULTI_POINTS: usize = 4096;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct PointRef {
    pub stroke_id: u64,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointTransform {
    #[default]
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointConstraint {
    #[default]
    Free,
    X,
    Y,
    Z,
    ViewPlane,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointPivot {
    #[default]
    SelectionCenter,
    WorldOrigin,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProportionalNeighborhood {
    #[default]
    AlongStroke,
    Spatial,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointFalloff {
    Linear,
    #[default]
    Smooth,
}

/// Detached multi-stroke point preview. Each update is rebuilt from `original`.
pub struct MultiDraft {
    pub original: Vec<Stroke>,
    pub preview: Vec<Stroke>,
    pub selected: HashSet<PointRef>,
    pub transform: PointTransform,
    pub constraint: PointConstraint,
    pub pivot_mode: PointPivot,
    pub custom_pivot: Vec3,
    pub values: Vec3,
    pub proportional: bool,
    pub radius: f32,
    pub neighborhood: ProportionalNeighborhood,
    pub falloff: PointFalloff,
}

impl MultiDraft {
    pub fn new(doc: &Document, ids: &HashSet<u64>) -> Result<Self, String> {
        if ids.is_empty() || ids.len() > MAX_MULTI_STROKES {
            return Err(format!(
                "Select 1–{MAX_MULTI_STROKES} strokes for multi-point editing."
            ));
        }
        let source: Vec<_> = doc
            .strokes
            .iter()
            .filter(|stroke| ids.contains(&stroke.id))
            .cloned()
            .collect();
        if source.len() != ids.len()
            || source.iter().any(|stroke| {
                !doc.editable(stroke)
                    || stroke.fill.is_some()
                    || stroke.curve.is_some()
                    || doc.has_linked_source(stroke.id)
                    || doc.linked_instance_for_stroke(stroke.id).is_some()
                    || stroke.points.is_empty()
            })
        {
            return Err("Multi-point editing supports selected editable sampled strokes only; fills, semantic curves, and linked artwork are excluded.".into());
        }
        let count: usize = source.iter().map(|stroke| stroke.points.len()).sum();
        if count > MAX_MULTI_POINTS {
            return Err(format!("Multi-point editing is limited to {MAX_MULTI_POINTS} samples across the selection."));
        }
        if source
            .iter()
            .any(|stroke| !stroke.points.iter().all(|point| point.is_valid()))
        {
            return Err("Selected strokes contain invalid point data.".into());
        }
        Ok(Self {
            preview: source.clone(),
            original: source,
            selected: HashSet::new(),
            transform: PointTransform::Move,
            constraint: PointConstraint::Free,
            pivot_mode: PointPivot::SelectionCenter,
            custom_pivot: Vec3::ZERO,
            values: Vec3::ZERO,
            proportional: false,
            radius: 1.0,
            neighborhood: ProportionalNeighborhood::AlongStroke,
            falloff: PointFalloff::Smooth,
        })
    }

    pub fn select_points(&mut self, selected: HashSet<PointRef>) {
        self.selected = selected;
        if self.selected.is_empty() {
            self.preview = self.original.clone();
        }
    }

    pub fn rebuild(&mut self, view_right: Vec3, view_up: Vec3) -> Result<(), String> {
        match self.rebuild_inner(view_right, view_up) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.preview = self.original.clone();
                Err(error)
            }
        }
    }

    fn rebuild_inner(&mut self, view_right: Vec3, view_up: Vec3) -> Result<(), String> {
        if self.selected.is_empty() {
            self.preview = self.original.clone();
            return Ok(());
        }
        if !self.values.is_finite() || !self.custom_pivot.is_finite() {
            return Err("Transform values and pivot must be finite.".into());
        }
        if self.proportional
            && (!self.radius.is_finite() || !(0.001..=1000.0).contains(&self.radius))
        {
            return Err("Proportional radius must be 0.001–1000 world units.".into());
        }
        let point_count: usize = self.original.iter().map(|stroke| stroke.points.len()).sum();
        if self.proportional
            && self.neighborhood == ProportionalNeighborhood::Spatial
            && point_count.saturating_mul(self.selected.len()) > 4_000_000
        {
            return Err(
                "Spatial proportional edit exceeds the 4,000,000 distance-check budget.".into(),
            );
        }
        let selected_positions: Vec<_> = self
            .original
            .iter()
            .flat_map(|stroke| {
                stroke
                    .points
                    .iter()
                    .enumerate()
                    .filter_map(|(index, point)| {
                        self.selected
                            .contains(&PointRef {
                                stroke_id: stroke.id,
                                index,
                            })
                            .then_some((stroke.id, index, point.position))
                    })
            })
            .collect();
        if selected_positions.is_empty() {
            return Err("Select one or more sample points first.".into());
        }
        let center = selected_positions.iter().map(|entry| entry.2).sum::<Vec3>()
            / selected_positions.len() as f32;
        let pivot = match self.pivot_mode {
            PointPivot::SelectionCenter => center,
            PointPivot::WorldOrigin => Vec3::ZERO,
            PointPivot::Custom => self.custom_pivot,
        };
        let delta = match self.constraint {
            PointConstraint::Free => self.values,
            PointConstraint::X => Vec3::X * self.values.x,
            PointConstraint::Y => Vec3::Y * self.values.y,
            PointConstraint::Z => Vec3::Z * self.values.z,
            PointConstraint::ViewPlane => view_right * self.values.x + view_up * self.values.y,
        };
        let rotation = Quat::from_euler(
            glam::EulerRot::XYZ,
            self.values.x.to_radians(),
            self.values.y.to_radians(),
            self.values.z.to_radians(),
        );
        if !rotation.is_finite() {
            return Err("Rotation values are invalid.".into());
        }
        let scale = self.values.clamp(Vec3::splat(0.01), Vec3::splat(100.0));
        let mut result = self.original.clone();
        let mut work = 0usize;
        for stroke in &mut result {
            let source = self
                .original
                .iter()
                .find(|source| source.id == stroke.id)
                .unwrap();
            let cumulative = cumulative_lengths(&source.points);
            let selected_along: Vec<_> = selected_positions
                .iter()
                .filter(|entry| entry.0 == stroke.id)
                .map(|entry| cumulative[entry.1])
                .collect();
            for (index, point) in stroke.points.iter_mut().enumerate() {
                let reference = PointRef {
                    stroke_id: stroke.id,
                    index,
                };
                let weight = if self.selected.contains(&reference) {
                    1.0
                } else if !self.proportional {
                    0.0
                } else {
                    work += selected_positions.len();
                    if work > 4_000_000 {
                        return Err(
                            "Proportional edit exceeds the 4,000,000 influence-check budget."
                                .into(),
                        );
                    }
                    let distance = match self.neighborhood {
                        ProportionalNeighborhood::Spatial => selected_positions
                            .iter()
                            .map(|entry| point.position.distance(entry.2))
                            .fold(f32::INFINITY, f32::min),
                        ProportionalNeighborhood::AlongStroke => selected_along
                            .iter()
                            .map(|position| (cumulative[index] - position).abs())
                            .fold(f32::INFINITY, f32::min),
                    };
                    if distance >= self.radius {
                        0.0
                    } else {
                        let t = 1.0 - distance / self.radius;
                        match self.falloff {
                            PointFalloff::Linear => t,
                            PointFalloff::Smooth => t * t * (3.0 - 2.0 * t),
                        }
                    }
                };
                if weight == 0.0 {
                    continue;
                }
                let position = match self.transform {
                    PointTransform::Move => point.position + delta * weight,
                    PointTransform::Rotate => point
                        .position
                        .lerp(pivot + rotation * (point.position - pivot), weight),
                    PointTransform::Scale => point
                        .position
                        .lerp(pivot + (point.position - pivot) * scale, weight),
                };
                if !position.is_finite() || position.abs().max_element() > 10000.0 {
                    return Err(
                        "Transform moves a sample outside the ±10000 world-unit bound.".into(),
                    );
                }
                point.position = position;
            }
        }
        self.preview = result;
        Ok(())
    }

    pub fn edit(&self, doc: &Document) -> Result<Edit, String> {
        if self.preview == self.original {
            return Err("No multi-point changes to apply.".into());
        }
        for source in &self.original {
            if !doc.editable(source) || !doc.strokes.iter().any(|stroke| stroke == source) {
                return Err(
                    "A source stroke changed or became protected; reopen multi-point editing."
                        .into(),
                );
            }
        }
        let ids: HashSet<_> = self.original.iter().map(|stroke| stroke.id).collect();
        let before = snapshot(doc, &ids);
        let mut after = before.clone();
        for (_, stroke) in &mut after {
            if let Some(preview) = self.preview.iter().find(|preview| preview.id == stroke.id) {
                *stroke = preview.clone();
            }
        }
        Ok(Edit::Change { before, after })
    }
}

fn cumulative_lengths(points: &[crate::document::Point]) -> Vec<f32> {
    let mut total = 0.0;
    let mut values = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        if index > 0 {
            total += point.position.distance(points[index - 1].position);
        }
        values.push(total);
    }
    values
}

pub const MAX_CANVAS_POINTS: usize = 4096;
pub const MAX_JOIN_TOLERANCE: f32 = 100.0;
pub const MAX_CLEANUP_POINTS: usize = 65_536;
pub const MAX_CLEANUP_WORK: usize = 16_000_000;
const MAX_CLEANUP_ERROR: f32 = 100.0;
const MAX_CLEANUP_SPACING: f32 = 1000.0;
const GRADIENT_LENGTH_ERROR: f32 = 0.01;
const JOIN_COLOR_EPSILON: f32 = 1.0e-4;
const JOIN_DISTANCE_EPSILON: f32 = 1.0e-5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CleanupStats {
    pub input_points: usize,
    pub output_points: usize,
    pub maximum_deviation: f32,
    pub source_length: f32,
    pub result_length: f32,
    pub elapsed_micros: u128,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionCleanupStats {
    pub strokes: usize,
    pub input_points: usize,
    pub output_points: usize,
    pub maximum_deviation: f32,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub elapsed_micros: u128,
}

/// Detached cleanup preview for a bounded set of selected strokes.
pub struct SelectionCleanupDraft {
    pub before: crate::editing::Snapshot,
    pub after: crate::editing::Snapshot,
    pub stats: SelectionCleanupStats,
}

impl SelectionCleanupDraft {
    pub fn cleanup(
        doc: &Document,
        ids: &HashSet<u64>,
        spacing: f32,
        max_error: f32,
    ) -> Result<Self, String> {
        use crate::editing::snapshot;
        if ids.is_empty() || ids.len() > MAX_MULTI_STROKES {
            return Err(format!(
                "Select 1–{MAX_MULTI_STROKES} editable ordinary strokes."
            ));
        }
        let before = snapshot(doc, ids);
        if before.len() != ids.len() {
            return Err("Cleanup requires every selected stroke to be editable.".into());
        }
        if before.iter().any(|(_, stroke)| {
            stroke.fill.is_some()
                || stroke.curve.is_some()
                || doc.has_linked_source(stroke.id)
                || doc.linked_instance_for_stroke(stroke.id).is_some()
        }) {
            return Err("Cleanup supports editable ordinary strokes; fills, curves and linked strokes are preserved.".into());
        }
        let total_points: usize = before.iter().map(|(_, stroke)| stroke.points.len()).sum();
        if total_points > MAX_CLEANUP_POINTS {
            return Err(format!(
                "Selection cleanup is limited to {MAX_CLEANUP_POINTS} input points."
            ));
        }
        let estimated_work = before
            .iter()
            .try_fold(0_usize, |total, (_, stroke)| {
                total.checked_add(stroke.points.len().saturating_mul(stroke.points.len()))
            })
            .unwrap_or(usize::MAX);
        if estimated_work > MAX_CLEANUP_WORK {
            return Err(format!("Selection cleanup is limited to {MAX_CLEANUP_WORK} estimated geometry comparisons."));
        }
        let started = std::time::Instant::now();
        let mut after = Vec::with_capacity(before.len());
        let mut input_points = 0;
        let mut output_points = 0;
        let mut maximum_deviation: f32 = 0.0;
        let mut input_bytes = 0;
        let mut output_bytes = 0;
        for (index, source) in &before {
            let (preview, stats) = cleanup_stroke(source, spacing, max_error)?;
            input_points += stats.input_points;
            output_points += stats.output_points;
            maximum_deviation = maximum_deviation.max(stats.maximum_deviation);
            input_bytes += serde_json::to_vec(source)
                .map_err(|error| error.to_string())?
                .len();
            output_bytes += serde_json::to_vec(&preview)
                .map_err(|error| error.to_string())?
                .len();
            after.push((*index, preview));
        }
        if before.iter().zip(&after).all(|((_, a), (_, b))| a == b) {
            return Err("Cleanup produced no changes for these settings.".into());
        }
        Ok(Self {
            before,
            after,
            stats: SelectionCleanupStats {
                strokes: ids.len(),
                input_points,
                output_points,
                maximum_deviation,
                input_bytes,
                output_bytes,
                elapsed_micros: started.elapsed().as_micros(),
            },
        })
    }

    pub fn valid_for(&self, doc: &Document) -> bool {
        let ids: HashSet<_> = self.before.iter().map(|(_, stroke)| stroke.id).collect();
        crate::editing::snapshot(doc, &ids) == self.before
            && self.before.iter().all(|(_, stroke)| {
                !doc.has_linked_source(stroke.id)
                    && doc.linked_instance_for_stroke(stroke.id).is_none()
            })
    }

    pub fn edit(&self, doc: &Document) -> Result<Edit, String> {
        if !self.valid_for(doc) {
            return Err("A source stroke changed; reopen the cleanup preview.".into());
        }
        Ok(Edit::Change {
            before: self.before.clone(),
            after: self.after.clone(),
        })
    }
}

/// A detached preview. Saving, picking and history see committed geometry until Apply.
pub struct Draft {
    pub original: Stroke,
    pub preview: Stroke,
    pub selected: usize,
    pub cleanup_stats: Option<CleanupStats>,
}

pub struct SplitPlan {
    pub edit: Edit,
    pub next_id: u64,
    pub first_id: u64,
    pub second_id: u64,
}

impl Draft {
    pub fn new(doc: &Document, id: u64) -> Result<Self, String> {
        let before = snapshot(doc, &HashSet::from([id]));
        let source = &before
            .first()
            .ok_or("Select an editable ordinary stroke.")?
            .1;
        if source.fill.is_some() || source.curve.is_some() || doc.has_linked_source(id) {
            return Err("Use the curve or group editor for this stroke.".into());
        }
        if !(2..=MAX_CANVAS_POINTS).contains(&source.points.len()) {
            return Err(format!(
                "Canvas point editing supports 2–{MAX_CANVAS_POINTS} samples per stroke."
            ));
        }
        Ok(Self {
            original: source.clone(),
            preview: source.clone(),
            selected: 0,
            cleanup_stats: None,
        })
    }

    pub fn cleanup(doc: &Document, id: u64, spacing: f32, max_error: f32) -> Result<Self, String> {
        let selected = HashSet::from([id]);
        let before = snapshot(doc, &selected);
        let source = before
            .first()
            .map(|(_, stroke)| stroke)
            .ok_or("Select one editable ordinary stroke.")?;
        if before.len() != 1
            || source.fill.is_some()
            || source.curve.is_some()
            || doc.has_linked_source(id)
            || doc.linked_instance_for_stroke(id).is_some()
        {
            return Err(
                "Cleanup supports one editable ordinary stroke; use its existing editor otherwise."
                    .into(),
            );
        }
        let (preview, cleanup_stats) = cleanup_stroke(source, spacing, max_error)?;
        if preview == *source {
            return Err("Cleanup produced no changes for these settings.".into());
        }
        Ok(Self {
            original: source.clone(),
            preview,
            selected: 0,
            cleanup_stats: Some(cleanup_stats),
        })
    }

    pub fn valid_for(&self, doc: &Document) -> bool {
        doc.strokes
            .iter()
            .any(|stroke| stroke == &self.original && doc.editable(stroke))
            && doc.linked_instance_for_stroke(self.original.id).is_none()
            && !doc.has_linked_source(self.original.id)
    }

    pub fn move_selected(&mut self, position: Vec3) -> Result<(), String> {
        let mut point = *self
            .preview
            .points
            .get(self.selected)
            .ok_or("Choose a point first.")?;
        point.position = position;
        if !point.is_valid() {
            return Err("Point coordinates must be finite and within ±10000 world units.".into());
        }
        let previous = self.preview.points[self.selected];
        self.preview.points[self.selected] = point;
        if let Err(error) = validate_geometry(&self.preview) {
            self.preview.points[self.selected] = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn delete_selected(&mut self) -> Result<(), String> {
        if self.preview.points.len() <= 2 || self.selected >= self.preview.points.len() {
            return Err("Keep at least two points.".into());
        }
        if let Some(colors) = &mut self.preview.point_colors {
            colors.remove(self.selected);
            if colors.iter().all(Option::is_none) {
                self.preview.point_colors = None;
            }
        }
        self.preview.points.remove(self.selected);
        self.selected = self.selected.min(self.preview.points.len() - 1);
        Ok(())
    }

    pub fn edit(&self, doc: &Document) -> Result<Edit, String> {
        if !self.valid_for(doc) {
            return Err("The source stroke changed; reopen point editing.".into());
        }
        if self.original == self.preview {
            return Err("No point changes to apply.".into());
        }
        validate_geometry(&self.preview)?;
        let before = snapshot(doc, &HashSet::from([self.original.id]));
        let mut after = before.clone();
        after[0].1 = self.preview.clone();
        Ok(Edit::Change { before, after })
    }
}

/// Rebuild a bounded sampled stroke from an immutable source. The simplifier
/// preserves sharp turns and meaningful pressure/tilt extrema; resampling then
/// interpolates the retained path at a uniform world-space spacing.
pub fn cleanup_stroke(
    source: &Stroke,
    spacing: f32,
    max_error: f32,
) -> Result<(Stroke, CleanupStats), String> {
    let started = std::time::Instant::now();
    if !spacing.is_finite() || !(0.001..=MAX_CLEANUP_SPACING).contains(&spacing) {
        return Err(format!(
            "Resample spacing must be 0.001–{MAX_CLEANUP_SPACING} world units."
        ));
    }
    if !max_error.is_finite() || !(0.0..=MAX_CLEANUP_ERROR).contains(&max_error) {
        return Err(format!(
            "Maximum error must be 0–{MAX_CLEANUP_ERROR} world units."
        ));
    }
    if source.fill.is_some() || source.curve.is_some() {
        return Err("Fills and semantic curves use their existing editors.".into());
    }
    if source.brush.flow.enabled {
        return Err("Timed Airbrush samples are preserved exactly because timestamps drive deposited coverage.".into());
    }
    if !(2..=MAX_CLEANUP_POINTS).contains(&source.points.len()) {
        return Err(format!(
            "Cleanup supports 2–{MAX_CLEANUP_POINTS} samples per stroke."
        ));
    }
    if !source.points.iter().all(|point| point.is_valid()) {
        return Err("The source stroke contains invalid point data.".into());
    }

    let source_length = points_length(&source.points);
    let anchors = protected_cleanup_indices(&source.points);
    let retained = simplify_indices(&source.points, &anchors, max_error)?;
    let reduced: Vec<_> = retained.iter().map(|index| source.points[*index]).collect();
    let points = resample_points(&reduced, spacing)?;
    if points.len() > MAX_CLEANUP_POINTS {
        return Err(format!(
            "Cleanup would exceed the {MAX_CLEANUP_POINTS}-point limit."
        ));
    }
    let mut preview = source.clone();
    preview.point_thickness = crate::stroke_sculpt::remap_thickness_profile(source, &points);
    preview.point_colors = crate::stroke_sculpt::remap_point_colors(source, &points);
    preview.points = points;
    validate_geometry(&preview)?;
    let result_length = points_length(&preview.points);
    if source.gradient.is_some()
        && source_length > 1.0e-5
        && ((result_length - source_length).abs() / source_length) > GRADIENT_LENGTH_ERROR
    {
        return Err("Cleanup would shift the gradient's arc-length ramp by more than 1%; lower the maximum error.".into());
    }
    let maximum_deviation = maximum_polyline_deviation(&source.points, &preview.points)?;
    if maximum_deviation > max_error + 1.0e-4 {
        return Err("Cleanup exceeded the requested geometric error bound.".into());
    }
    let output_points = preview.points.len();
    let input_bytes = serde_json::to_vec(source)
        .map_err(|error| error.to_string())?
        .len();
    let output_bytes = serde_json::to_vec(&preview)
        .map_err(|error| error.to_string())?
        .len();
    Ok((
        preview,
        CleanupStats {
            input_points: source.points.len(),
            output_points,
            maximum_deviation,
            source_length,
            result_length,
            elapsed_micros: started.elapsed().as_micros(),
            input_bytes,
            output_bytes,
        },
    ))
}

fn points_length(points: &[crate::document::Point]) -> f32 {
    points
        .windows(2)
        .map(|pair| pair[0].position.distance(pair[1].position))
        .sum()
}

fn protected_cleanup_indices(points: &[crate::document::Point]) -> Vec<usize> {
    let mut protected = Vec::with_capacity(points.len().min(1024));
    protected.push(0);
    for index in 1..points.len().saturating_sub(1) {
        let previous = points[index - 1];
        let current = points[index];
        let next = points[index + 1];
        let incoming = current.position - previous.position;
        let outgoing = next.position - current.position;
        let corner = if incoming.length_squared() <= 1.0e-12 || outgoing.length_squared() <= 1.0e-12
        {
            true
        } else {
            let cosine = incoming
                .normalize()
                .dot(outgoing.normalize())
                .clamp(-1.0, 1.0);
            cosine.acos() >= 30.0_f32.to_radians()
        };
        let pressure_extremum =
            (current.pressure - (previous.pressure + next.pressure) * 0.5).abs() >= 0.08;
        let tilt_extremum = match (previous.tilt, current.tilt, next.tilt) {
            (Some(before), Some(now), Some(after)) => {
                before.normalize().dot(now.normalize()) < 0.965_925_8
                    || now.normalize().dot(after.normalize()) < 0.965_925_8
            }
            (None, Some(_), Some(_)) | (Some(_), None, Some(_)) | (Some(_), Some(_), None) => true,
            _ => false,
        };
        if corner || pressure_extremum || tilt_extremum {
            protected.push(index);
        }
    }
    protected.push(points.len() - 1);
    protected
}

fn simplify_indices(
    points: &[crate::document::Point],
    anchors: &[usize],
    max_error: f32,
) -> Result<Vec<usize>, String> {
    let mut retained = vec![false; points.len()];
    for &index in anchors {
        retained[index] = true;
    }
    if max_error > 0.0 {
        let mut work = Vec::new();
        let mut operations = 0usize;
        for pair in anchors.windows(2) {
            work.push((pair[0], pair[1]));
        }
        while let Some((start, end)) = work.pop() {
            if end <= start + 1 {
                continue;
            }
            let origin = points[start].position;
            let segment = points[end].position - origin;
            let segment_length_sq = segment.length_squared();
            let mut farthest = None;
            let mut farthest_distance_sq = max_error * max_error;
            for (index, point) in points.iter().enumerate().take(end).skip(start + 1) {
                operations = operations.saturating_add(1);
                if operations > MAX_CLEANUP_WORK {
                    return Err(format!(
                        "Simplification reached its bounded {MAX_CLEANUP_WORK}-operation limit."
                    ));
                }
                let offset = point.position - origin;
                let t = if segment_length_sq > 1.0e-12 {
                    (offset.dot(segment) / segment_length_sq).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance_sq = point.position.distance_squared(origin + segment * t);
                if distance_sq > farthest_distance_sq {
                    farthest_distance_sq = distance_sq;
                    farthest = Some(index);
                }
            }
            if let Some(index) = farthest {
                retained[index] = true;
                work.push((start, index));
                work.push((index, end));
            }
        }
    }
    Ok(retained
        .into_iter()
        .enumerate()
        .filter_map(|(index, keep)| keep.then_some(index))
        .collect())
}

fn resample_points(
    anchors: &[crate::document::Point],
    spacing: f32,
) -> Result<Vec<crate::document::Point>, String> {
    let mut output = Vec::with_capacity(anchors.len());
    output.push(anchors[0]);
    let mut work = 0usize;
    for pair in anchors.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let distance = start.position.distance(end.position);
        if !distance.is_finite() {
            return Err("Cleanup encountered a non-finite segment length.".into());
        }
        if distance <= 1.0e-6 {
            // Coincident samples can carry pressure/time changes; keep both.
            output.push(end);
            continue;
        }
        let steps = (distance / spacing).ceil().max(1.0) as usize;
        work = work.saturating_add(steps);
        if work > MAX_CLEANUP_POINTS || output.len().saturating_add(steps) > MAX_CLEANUP_POINTS {
            return Err(format!(
                "Resampling would exceed the {MAX_CLEANUP_POINTS}-point limit."
            ));
        }
        for step in 1..=steps {
            output.push(interpolate_point(start, end, step as f32 / steps as f32));
        }
    }
    Ok(output)
}

fn interpolate_point(
    start: crate::document::Point,
    end: crate::document::Point,
    t: f32,
) -> crate::document::Point {
    let tilt = match (start.tilt, end.tilt) {
        (Some(a), Some(b)) => {
            let interpolated = a.lerp(b, t);
            (interpolated.length_squared() > 1.0e-8).then(|| interpolated.normalize())
        }
        (Some(a), None) => (t < 0.5).then_some(a),
        (None, Some(b)) => (t >= 0.5).then_some(b),
        (None, None) => None,
    };
    crate::document::Point {
        position: start.position.lerp(end.position, t),
        pressure: (start.pressure + (end.pressure - start.pressure) * t).clamp(0.0, 1.0),
        tilt,
        time_ms: (start.time_ms as f32 + (end.time_ms as f32 - start.time_ms as f32) * t)
            .round()
            .clamp(0.0, crate::document::MAX_FLOW_DURATION_MS as f32) as u32,
    }
}

fn maximum_polyline_deviation(
    source: &[crate::document::Point],
    result: &[crate::document::Point],
) -> Result<f32, String> {
    let mut operations = 0usize;
    let source_to_result = directed_polyline_deviation(source, result, &mut operations)?;
    let result_to_source = directed_polyline_deviation(result, source, &mut operations)?;
    Ok(source_to_result.max(result_to_source))
}

fn directed_polyline_deviation(
    source: &[crate::document::Point],
    target: &[crate::document::Point],
    operations: &mut usize,
) -> Result<f32, String> {
    let mut maximum = 0.0_f32;
    for point in source {
        let mut nearest = f32::INFINITY;
        for pair in target.windows(2) {
            *operations = operations.saturating_add(1);
            if *operations > MAX_CLEANUP_WORK {
                return Err(format!(
                    "Geometric error measurement reached its bounded {MAX_CLEANUP_WORK}-comparison limit."
                ));
            }
            let edge = pair[1].position - pair[0].position;
            let length_sq = edge.length_squared();
            let t = if length_sq > 1.0e-12 {
                ((point.position - pair[0].position).dot(edge) / length_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            nearest = nearest.min(point.position.distance(pair[0].position + edge * t));
        }
        maximum = maximum.max(nearest);
    }
    Ok(maximum)
}

fn validate_geometry(stroke: &Stroke) -> Result<(), String> {
    crate::dab_placement::point_count(stroke)?;
    if stroke.brush.flow.enabled
        && crate::geometry::airbrush_vertex_estimate(stroke)
            > crate::geometry::MAX_AIRBRUSH_VERTICES
    {
        return Err("Point edit exceeds the airbrush geometry budget.".into());
    }
    Ok(())
}

/// Split an ordinary sampled stroke after `point_index`. The shared sample is
/// retained by both pieces, so the pair covers the same centerline as the
/// original and keeps its captured pressure, tilt, and time values.
pub fn split_stroke(
    doc: &Document,
    stroke_id: u64,
    point_index: usize,
    next_id: u64,
) -> Result<SplitPlan, String> {
    let before = snapshot_for_point_operation(doc, &[stroke_id])?;
    let (index, source) = &before[0];
    if source.points.len() < 4 {
        return Err("A split needs at least four samples.".into());
    }
    if point_index == 0 || point_index >= source.points.len() - 1 {
        return Err("Choose an interior point so each piece keeps at least two samples.".into());
    }
    if next_id == 0 || next_id == u64::MAX || doc.strokes.iter().any(|stroke| stroke.id == next_id)
    {
        return Err("No available stroke ID for the split piece.".into());
    }
    if doc.strokes.len() >= 100_000 {
        return Err("Document stroke limit reached.".into());
    }
    let Some(after_next_id) = next_id.checked_add(1).filter(|id| *id < u64::MAX) else {
        return Err("No more stroke IDs available.".into());
    };

    let arc_lengths = crate::dynamics::arc_lengths(&source.points);
    let split_distance = arc_lengths[point_index];
    let total_length = *arc_lengths.last().unwrap_or(&0.0);
    let mut first = source.clone();
    let mut second = source.clone();
    first.points = source.points[..=point_index].to_vec();
    second.points = source.points[point_index..].to_vec();
    if let Some(profile) = &source.point_thickness {
        first.point_thickness = Some(profile[..=point_index].to_vec());
        second.point_thickness = Some(profile[point_index..].to_vec());
    }
    if let Some(profile) = &source.point_colors {
        first.point_colors = Some(profile[..=point_index].to_vec());
        second.point_colors = Some(profile[point_index..].to_vec());
    }
    second.id = next_id;
    if let Some(gradient) = source.gradient {
        let t = if total_length > 1.0e-8 {
            (split_distance / total_length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut first_gradient = gradient.subrange(0.0, t);
        first_gradient.start = gradient.start;
        let mut second_gradient = gradient.subrange(t, 1.0);
        second_gradient.end = gradient.end;
        first.gradient = Some(first_gradient);
        second.gradient = Some(second_gradient);
    }
    if source.brush.dabs.enabled {
        let total = if source.brush.dab_total_length > 0.0 {
            source.brush.dab_total_length
        } else {
            source.brush.dab_distance_origin + total_length
        };
        first.brush.dab_total_length = total;
        second.brush.dab_total_length = total;
        second.brush.dab_distance_origin += split_distance;
    }
    validate_geometry(&first)?;
    validate_geometry(&second)?;
    let after = vec![(*index, first), (*index + 1, second)];
    Ok(SplitPlan {
        edit: Edit::Change { before, after },
        next_id: after_next_id,
        first_id: stroke_id,
        second_id: next_id,
    })
}

/// Join two compatible ordinary strokes in the requested point order. The
/// endpoint tolerance is in world units; a nonzero gap is retained as a short
/// connecting segment, while an identical seam sample is stored only once.
pub fn join_strokes(
    doc: &Document,
    first_id: u64,
    second_id: u64,
    endpoint_tolerance: f32,
) -> Result<Edit, String> {
    if first_id == second_id {
        return Err("Choose two different strokes to join.".into());
    }
    if !endpoint_tolerance.is_finite() || !(0.0..=MAX_JOIN_TOLERANCE).contains(&endpoint_tolerance)
    {
        return Err(format!(
            "Endpoint tolerance must be from 0 to {MAX_JOIN_TOLERANCE} world units."
        ));
    }
    let before = snapshot_for_point_operation(doc, &[first_id, second_id])?;
    if before.len() != 2 {
        return Err("Select two editable ordinary strokes.".into());
    }
    let first_entry = before
        .iter()
        .find(|(_, stroke)| stroke.id == first_id)
        .ok_or("The first stroke is unavailable.")?;
    let second_entry = before
        .iter()
        .find(|(_, stroke)| stroke.id == second_id)
        .ok_or("The second stroke is unavailable.")?;
    let first = &first_entry.1;
    let second = &second_entry.1;
    if first.points.len() < 2 || second.points.len() < 2 {
        return Err("Both strokes need at least two samples.".into());
    }
    let duplicate_seam_sample = first.points.last() == second.points.first();
    let combined_point_count =
        first.points.len() + second.points.len() - usize::from(duplicate_seam_sample);
    if combined_point_count > MAX_CANVAS_POINTS {
        return Err(format!(
            "A joined stroke supports at most {MAX_CANVAS_POINTS} samples."
        ));
    }
    if first.layer_id != second.layer_id || first.group_id != second.group_id {
        return Err("Strokes must belong to the same layer and group.".into());
    }
    if first.material != second.material
        || first.color != second.color
        || first.radius != second.radius
    {
        return Err("Strokes must have matching color, radius, and material.".into());
    }
    if !brush_settings_match(&first.brush, &second.brush) {
        return Err("Strokes must use matching brush settings and dab seeds.".into());
    }
    let endpoint_gap = first
        .points
        .last()
        .unwrap()
        .position
        .distance(second.points[0].position);
    if !endpoint_gap.is_finite() || endpoint_gap > endpoint_tolerance {
        return Err("The chosen stroke endpoints are farther apart than the tolerance.".into());
    }
    if first.brush.flow.enabled
        && (first
            .points
            .windows(2)
            .chain(second.points.windows(2))
            .any(|pair| pair[0].time_ms > pair[1].time_ms)
            || first.points.last().unwrap().time_ms > second.points[0].time_ms)
    {
        return Err("Timed strokes must continue forward in sample time.".into());
    }
    let combined_gradient = match (first.gradient, second.gradient) {
        (None, None) => None,
        (Some(a), Some(b))
            if a.end
                .iter()
                .zip(b.start)
                .all(|(left, right)| (*left - right).abs() <= JOIN_COLOR_EPSILON) =>
        {
            Some(crate::document::StrokeGradient {
                start: a.start,
                end: b.end,
            })
        }
        (Some(_), Some(_)) => {
            return Err("Gradient colors must meet at the join endpoint.".into());
        }
        _ => return Err("Both strokes must use the same gradient mode.".into()),
    };
    if first.has_local_colors() || second.has_local_colors() {
        let first_end = first.point_color_at(first.points.len() - 1, 1.0);
        let second_start = second.point_color_at(0, 0.0);
        if first_end
            .iter()
            .zip(second_start)
            .any(|(left, right)| (left - right).abs() > JOIN_COLOR_EPSILON)
        {
            return Err("Painted colors must meet at the join endpoint.".into());
        }
    }

    if first.brush.dabs.enabled {
        if endpoint_gap > JOIN_DISTANCE_EPSILON {
            return Err("Dab strokes can join only at coincident endpoints.".into());
        }
        let first_length = stroke_length(first);
        let expected_origin = first.brush.dab_distance_origin + first_length;
        let origin_epsilon = 1.0e-4_f32.max(first_length.abs() * 1.0e-5);
        if (second.brush.dab_distance_origin - expected_origin).abs() > origin_epsilon
            || (effective_dab_total(first) - effective_dab_total(second)).abs() > origin_epsilon
        {
            return Err("Dab placement phases must be continuous across the join.".into());
        }
    }

    let mut joined = first.clone();
    joined.gradient = combined_gradient;
    if first.point_thickness.is_some() || second.point_thickness.is_some() {
        let first_profile = first
            .point_thickness
            .clone()
            .unwrap_or_else(|| vec![1.0; first.points.len()]);
        let second_profile = second
            .point_thickness
            .as_ref()
            .cloned()
            .unwrap_or_else(|| vec![1.0; second.points.len()]);
        joined.point_thickness = Some(first_profile);
        if let Some(joined_profile) = joined.point_thickness.as_mut() {
            joined_profile.extend(
                second_profile
                    .iter()
                    .skip(usize::from(duplicate_seam_sample))
                    .copied(),
            );
        }
    }
    if first.point_colors.is_some() || second.point_colors.is_some() {
        let first_profile = first
            .point_colors
            .clone()
            .unwrap_or_else(|| vec![None; first.points.len()]);
        let second_profile = second
            .point_colors
            .as_ref()
            .cloned()
            .unwrap_or_else(|| vec![None; second.points.len()]);
        joined.point_colors = Some(first_profile);
        if let Some(joined_profile) = joined.point_colors.as_mut() {
            joined_profile.extend(
                second_profile
                    .iter()
                    .skip(usize::from(duplicate_seam_sample))
                    .copied(),
            );
            if joined_profile.iter().all(Option::is_none) {
                joined.point_colors = None;
            }
        }
    }
    joined.points.extend(
        second
            .points
            .iter()
            .skip(usize::from(duplicate_seam_sample))
            .copied(),
    );
    if joined.brush.dabs.enabled {
        let expected_total = joined.brush.dab_distance_origin + stroke_length(&joined);
        let total = effective_dab_total(first);
        joined.brush.dab_total_length = if (total - expected_total).abs() <= 1.0e-4 {
            0.0
        } else {
            total
        };
    }
    validate_geometry(&joined)?;
    let insertion_index = first_entry.0.min(second_entry.0);
    Ok(Edit::Change {
        before,
        after: vec![(insertion_index, joined)],
    })
}

fn snapshot_for_point_operation(
    doc: &Document,
    ids: &[u64],
) -> Result<Vec<(usize, Stroke)>, String> {
    let selected: HashSet<_> = ids.iter().copied().collect();
    let before = snapshot(doc, &selected);
    if before.len() != selected.len() {
        return Err("Missing, hidden, locked, or generated strokes cannot be edited.".into());
    }
    for (_, stroke) in &before {
        if stroke.fill.is_some() || stroke.curve.is_some() {
            return Err("Fills and semantic curves use their existing editors.".into());
        }
        if doc.has_linked_source(stroke.id) || doc.linked_instance_for_stroke(stroke.id).is_some() {
            return Err("Make linked strokes unique before editing their samples.".into());
        }
        if !(2..=MAX_CANVAS_POINTS).contains(&stroke.points.len()) {
            return Err(format!(
                "Point operations support 2–{MAX_CANVAS_POINTS} samples per stroke."
            ));
        }
    }
    Ok(before)
}

fn brush_settings_match(first: &crate::document::Brush, second: &crate::document::Brush) -> bool {
    let mut first = *first;
    let mut second = *second;
    first.dab_distance_origin = 0.0;
    second.dab_distance_origin = 0.0;
    first.dab_total_length = 0.0;
    second.dab_total_length = 0.0;
    first == second
}

fn stroke_length(stroke: &Stroke) -> f32 {
    crate::dynamics::arc_lengths(&stroke.points)
        .last()
        .copied()
        .unwrap_or(0.0)
}

fn effective_dab_total(stroke: &Stroke) -> f32 {
    if stroke.brush.dab_total_length > 0.0 {
        stroke.brush.dab_total_length
    } else {
        stroke.brush.dab_distance_origin + stroke_length(stroke)
    }
}

pub fn edit_point(
    doc: &Document,
    stroke_id: u64,
    index: usize,
    position: Option<Vec3>,
) -> Result<Edit, String> {
    let before = snapshot(doc, &HashSet::from([stroke_id]));
    let Some((_, source)) = before.first() else {
        return Err("Select an editable ordinary stroke.".into());
    };
    if source.fill.is_some() || source.curve.is_some() || doc.has_linked_source(stroke_id) {
        return Err(
            "Use the curve or group editor for this stroke; fills are not supported.".into(),
        );
    }
    if index >= source.points.len() {
        return Err("Point index is outside the stroke.".into());
    }
    let mut after = before.clone();
    let stroke = &mut after[0].1;
    if let Some(position) = position {
        let mut point = stroke.points[index];
        point.position = position;
        if !point.is_valid() {
            return Err("Point coordinates must be finite and within ±10000 world units.".into());
        }
        stroke.points[index] = point;
    } else {
        if stroke.points.len() <= 2 {
            return Err(
                "Keep at least two points; delete the stroke to remove it entirely.".into(),
            );
        }
        stroke.points.remove(index);
        if let Some(colors) = &mut stroke.point_colors {
            colors.remove(index);
            if colors.iter().all(Option::is_none) {
                stroke.point_colors = None;
            }
        }
    }
    if before == after {
        return Err("Point position is unchanged.".into());
    }
    validate_geometry(&after[0].1)?;
    Ok(Edit::Change { before, after })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point, Stroke, StrokeMaterial};
    fn sample() -> Document {
        let mut doc = Document::default();
        doc.strokes.push(Stroke {
            guide_attachment: None,
            model_surface: None,
            id: 1,
            layer_id: 1,
            group_id: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            curve: None,
            material: StrokeMaterial::Shaded,
            color: [0.2, 0.4, 0.6],
            gradient: None,
            radius: 0.02,
            brush: Brush::default(),
            points: (0..3)
                .map(|i| Point {
                    position: Vec3::X * i as f32,
                    pressure: 0.25 * (i + 1) as f32,
                    tilt: Some(Vec3::Y),
                    time_ms: i * 10,
                })
                .collect(),
        });
        doc
    }

    fn four_point_sample() -> Document {
        let mut doc = sample();
        let mut fourth = doc.strokes[0].points[2];
        fourth.position = Vec3::X * 3.0;
        fourth.pressure = 0.9;
        fourth.tilt = Some(Vec3::new(0.0, 0.6, 0.8));
        fourth.time_ms = 30;
        doc.strokes[0].points.push(fourth);
        doc
    }

    #[test]
    fn split_and_join_roundtrip_preserves_samples_gradient_dabs_and_history() {
        let mut doc = four_point_sample();
        doc.strokes[0].point_thickness = Some(vec![0.5, 0.75, 1.25, 1.5]);
        doc.strokes[0].gradient = Some(crate::document::StrokeGradient {
            start: [0.0, 0.1, 0.2],
            end: [0.8, 0.7, 0.6],
        });
        doc.strokes[0].brush.dabs.enabled = true;
        doc.strokes[0].brush.dabs.spacing = 0.4;
        doc.strokes[0].brush.dab_seed = 41;
        let original = doc.strokes[0].clone();
        let dabs = crate::dab_placement::sample(&original).unwrap();
        let original_document = Document {
            strokes: vec![original.clone()],
            ..Document::default()
        };
        let original_glb = crate::export::encode(&original_document, None)
            .unwrap()
            .bytes;
        let plan = split_stroke(&doc, 1, 1, 2).unwrap();
        assert_eq!(plan.next_id, 3);
        assert_eq!((plan.first_id, plan.second_id), (1, 2));
        plan.edit.apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 2);
        assert_eq!(doc.strokes[0].points, original.points[..=1]);
        assert_eq!(doc.strokes[1].points, original.points[1..]);
        assert_eq!(doc.strokes[0].points[1], doc.strokes[1].points[0]);
        assert_eq!(doc.strokes[0].point_thickness, Some(vec![0.5, 0.75]));
        assert_eq!(doc.strokes[1].point_thickness, Some(vec![0.75, 1.25, 1.5]));
        let first_dabs = crate::dab_placement::sample(&doc.strokes[0]).unwrap();
        let second_dabs = crate::dab_placement::sample(&doc.strokes[1]).unwrap();
        let split_dabs: Vec<_> = first_dabs.iter().chain(&second_dabs).collect();
        assert_eq!(
            split_dabs.len(),
            dabs.len(),
            "the shared split seam emits one dab"
        );
        assert!(first_dabs.last().unwrap().position.distance(Vec3::X) < 1.0e-5);
        assert!(second_dabs[0].position.distance(Vec3::X) > 1.0e-5);
        for (split_dab, original_dab) in split_dabs.iter().zip(&dabs) {
            assert!(split_dab.position.distance(original_dab.position) < 1.0e-4);
            assert!((split_dab.pressure - original_dab.pressure).abs() < 1.0e-5);
            assert!((split_dab.size_scale - original_dab.size_scale).abs() < 1.0e-5);
        }
        assert!(doc.strokes[0]
            .gradient
            .unwrap()
            .end
            .iter()
            .zip(doc.strokes[1].gradient.unwrap().start)
            .all(|(a, b)| (a - b).abs() < 1.0e-6));
        assert!(doc.strokes[1].brush.dab_distance_origin > 0.0);
        let selected = HashSet::from([1, 2]);
        let split_bounds = crate::editing::bounds(&doc, &selected).unwrap();
        let original_bounds = crate::editing::bounds(
            &Document {
                strokes: vec![original.clone()],
                ..Document::default()
            },
            &HashSet::from([1]),
        )
        .unwrap();
        assert!(split_bounds.0.x > original_bounds.0.x);
        assert_eq!(split_bounds.1, original_bounds.1);

        let recovery_dir = tempfile::tempdir().unwrap();
        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "point-split-join-test".into(),
            None,
            doc.clone(),
        );
        crate::recovery_store::write(recovery_dir.path(), &recovery).unwrap();
        let recovered =
            crate::recovery_store::scan(recovery_dir.path(), "point-split-join-reader").unwrap();
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes,
            doc.strokes
        );
        let glb = crate::export::encode(&doc, None).unwrap();
        assert_eq!(glb.summary.strokes, 2);
        let obj_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let obj_path = obj_dir.path().join("split.obj");
        assert!(crate::obj_export::write_obj_bundle(&doc, None, &obj_path, false).unwrap() > 0);
        assert!(std::fs::read_to_string(obj_path).unwrap().contains("v "));

        let join = join_strokes(&doc, 1, 2, 0.0).unwrap();
        join.apply(&mut doc, true);
        assert_eq!(doc.strokes, vec![original.clone()]);
        assert_eq!(doc.strokes[0].point_thickness, original.point_thickness);
        assert_eq!(crate::dab_placement::sample(&doc.strokes[0]).unwrap(), dabs);
        assert_eq!(
            crate::export::encode(&doc, None).unwrap().bytes,
            original_glb
        );
        assert_eq!(
            Document::from_json(&serde_json::to_string(&doc).unwrap())
                .unwrap()
                .strokes,
            doc.strokes
        );
        join.apply(&mut doc, false);
        assert_eq!(doc.strokes.len(), 2);
        plan.edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, vec![original]);
    }

    #[test]
    fn join_uses_explicit_order_and_world_endpoint_tolerance() {
        let mut doc = four_point_sample();
        let mut first = doc.strokes[0].clone();
        first.id = 10;
        first.points = vec![doc.strokes[0].points[2], doc.strokes[0].points[3]];
        let mut second = doc.strokes[0].clone();
        second.id = 11;
        second.points = vec![doc.strokes[0].points[0], doc.strokes[0].points[1]];
        doc.strokes = vec![first.clone(), second.clone()];
        let join = join_strokes(&doc, 11, 10, 1.0).unwrap();
        join.apply(&mut doc, true);
        assert_eq!(doc.strokes.len(), 1);
        assert_eq!(doc.strokes[0].id, 11);
        assert_eq!(doc.strokes[0].points, four_point_sample().strokes[0].points);
        join.apply(&mut doc, false);
        assert_eq!(doc.strokes, vec![first, second]);

        let mut near = four_point_sample();
        near.strokes.truncate(1);
        let mut tail = near.strokes[0].clone();
        tail.id = 2;
        tail.points = vec![
            crate::document::Point {
                position: Vec3::new(2.05, 0.0, 0.0),
                ..near.strokes[0].points[2]
            },
            crate::document::Point {
                position: Vec3::new(3.0, 0.0, 0.0),
                ..near.strokes[0].points[3]
            },
        ];
        near.strokes[0].points = near.strokes[0].points[..=2].to_vec();
        near.strokes.push(tail);
        assert!(join_strokes(&near, 1, 2, 0.04).is_err());
        let joined = join_strokes(&near, 1, 2, 0.051).unwrap();
        joined.apply(&mut near, true);
        assert_eq!(
            near.strokes[0].points.len(),
            5,
            "the accepted gap stays explicit"
        );
    }

    #[test]
    fn join_rejects_incompatible_attributes_and_protected_or_semantic_strokes() {
        let base = four_point_sample();
        assert!(split_stroke(&base, 1, usize::MAX, 2).is_err());
        let mut compatible = base.clone();
        let mut tail = compatible.strokes[0].clone();
        tail.id = 2;
        tail.points = vec![
            compatible.strokes[0].points[2],
            compatible.strokes[0].points[3],
        ];
        compatible.strokes[0].points.truncate(3);
        compatible.strokes.push(tail);
        compatible.strokes[1].material = crate::document::StrokeMaterial::Unlit;
        assert!(join_strokes(&compatible, 1, 2, 0.0).is_err());
        compatible.strokes[1].material = compatible.strokes[0].material;
        compatible.strokes[1].brush.softness = 0.2;
        assert!(join_strokes(&compatible, 1, 2, 0.0).is_err());
        compatible.strokes[1].brush.softness = compatible.strokes[0].brush.softness;
        compatible.strokes[1].layer_id = 999;
        assert!(join_strokes(&compatible, 1, 2, 0.0).is_err());

        let mut fill = base.clone();
        fill.strokes[0].fill = Some(crate::document::PlanarFill {
            offset: 0.0,
            gradient: None,
            texture: None,
            holes: Vec::new(),
        });
        assert!(split_stroke(&fill, 1, 1, 2).is_err());
        let mut locked = base;
        locked.layers[0].locked = true;
        assert!(split_stroke(&locked, 1, 1, 2).is_err());
    }

    #[test]
    fn split_and_join_preserve_timed_airbrush_samples_in_order() {
        let mut doc = four_point_sample();
        doc.strokes[0].brush.flow.enabled = true;
        for (index, point) in doc.strokes[0].points.iter_mut().enumerate() {
            point.time_ms = index as u32 * 30;
        }
        let original = doc.strokes[0].clone();
        let split = split_stroke(&doc, 1, 1, 2).unwrap();
        split.edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0].points.last().unwrap().time_ms, 30);
        assert_eq!(doc.strokes[1].points[0].time_ms, 30);
        assert!(doc.strokes[1]
            .points
            .windows(2)
            .all(|pair| pair[0].time_ms <= pair[1].time_ms));
        let join = join_strokes(&doc, 1, 2, 0.0).unwrap();
        join.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], original);
        let native = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(native.strokes, vec![original]);
        assert!(crate::export::encode(&native, None).is_ok());
    }
    #[test]
    fn move_delete_roundtrip_and_exact_history() {
        let mut doc = sample();
        let original = doc.strokes.clone();
        let edit = edit_point(&doc, 1, 1, Some(Vec3::Y)).unwrap();
        edit.apply(&mut doc, true);
        let mut expected = original[0].points[1];
        expected.position = Vec3::Y;
        assert_eq!(doc.strokes[0].points[1], expected);
        let json = serde_json::to_string(&doc).unwrap();
        assert_eq!(Document::from_json(&json).unwrap().strokes, doc.strokes);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
        edit.apply(&mut doc, true);
        let delete = edit_point(&doc, 1, 1, None).unwrap();
        delete.apply(&mut doc, true);
        assert_eq!(
            doc.strokes[0].points,
            vec![original[0].points[0], original[0].points[2]]
        );
        assert!(edit_point(&doc, 1, 0, None).is_err());
        delete.apply(&mut doc, false);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
    }
    #[test]
    fn rejects_invalid_and_protected_edits_without_mutation() {
        let mut doc = sample();
        let original = doc.strokes.clone();
        for position in [
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
            Vec3::X * 10001.0,
        ] {
            assert!(edit_point(&doc, 1, 0, Some(position)).is_err());
        }
        assert!(edit_point(&doc, 1, 99, None).is_err());
        assert!(edit_point(&doc, 999, 0, None).is_err());
        assert!(edit_point(&doc, 1, 0, Some(Vec3::ZERO)).is_err());
        doc.layers[0].locked = true;
        assert!(edit_point(&doc, 1, 0, Some(Vec3::Y)).is_err());
        doc.layers[0].locked = false;
        doc.layers[0].visible = false;
        assert!(edit_point(&doc, 1, 0, None).is_err());
        assert_eq!(doc.strokes, original);
        doc.layers[0].visible = true;
        doc.strokes[0].fill = Some(crate::document::PlanarFill {
            offset: 0.0,
            gradient: None,
            texture: None,
            holes: Vec::new(),
        });
        assert!(edit_point(&doc, 1, 0, Some(Vec3::Y)).is_err());
    }

    #[test]
    fn detached_session_batches_changes_and_rejects_stale_source() {
        let mut doc = sample();
        let original = doc.strokes.clone();
        let mut draft = Draft::new(&doc, 1).unwrap();
        assert!(draft.edit(&doc).is_err());
        draft.selected = 1;
        draft.move_selected(Vec3::new(1.0, 0.4, 0.0)).unwrap();
        draft.move_selected(Vec3::new(1.0, 0.8, 0.0)).unwrap();
        assert_eq!(doc.strokes, original, "preview never changes the document");
        let preview = draft.preview.clone();
        let edit = draft.edit(&doc).unwrap();
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], preview);
        assert!(
            draft.edit(&doc).is_err(),
            "a stale preview cannot overwrite an edit"
        );
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original);
        doc.layers[0].locked = true;
        assert!(draft.edit(&doc).is_err());
        doc.layers[0].locked = false;
        draft.delete_selected().unwrap();
        assert_eq!(draft.preview.points.len(), 2);
        assert!(draft.delete_selected().is_err());
        drop(draft);
        assert_eq!(
            doc.strokes, original,
            "discarding a preview is exact cancellation"
        );
    }

    #[test]
    fn edited_geometry_survives_recovery_and_exports() {
        let mut doc = sample();
        let mut draft = Draft::new(&doc, 1).unwrap();
        draft.selected = 1;
        draft.move_selected(Vec3::new(1.0, 0.6, 0.2)).unwrap();
        draft.edit(&doc).unwrap().apply(&mut doc, true);
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "point-edit-test".into(),
            None,
            doc.clone(),
        );
        let json = serde_json::to_vec(&snapshot).unwrap();
        let loaded: crate::recovery_store::RecoverySnapshot =
            serde_json::from_slice(&json).unwrap();
        assert_eq!(loaded.document.strokes, doc.strokes);
        let glb = crate::export::encode(&doc, None).unwrap().bytes;
        assert_eq!(
            crate::export::encode(&loaded.document, None).unwrap().bytes,
            glb
        );
        assert!(
            crate::editing::bounds(&doc, &HashSet::from([1]))
                .unwrap()
                .1
                .y
                >= 0.6
        );
    }

    #[test]
    fn canvas_point_budget_and_dab_expansion_are_bounded() {
        let mut doc = sample();
        doc.strokes[0].points = vec![doc.strokes[0].points[0]; MAX_CANVAS_POINTS + 1];
        assert!(Draft::new(&doc, 1).is_err());
        let mut doc = sample();
        doc.strokes[0].brush.dabs.enabled = true;
        doc.strokes[0].brush.dabs.spacing = 0.1;
        let mut draft = Draft::new(&doc, 1).unwrap();
        let previous = draft.preview.clone();
        draft.selected = 1;
        assert!(draft.move_selected(Vec3::X * 9999.0).is_err());
        assert_eq!(draft.preview, previous);
        assert!(edit_point(&doc, 1, 1, Some(Vec3::X * 9999.0)).is_err());
    }

    #[test]
    fn cleanup_preserves_endpoints_corners_pressure_and_tilt_within_error() {
        let mut stroke = sample().strokes.remove(0);
        stroke.points = (0_i32..401)
            .map(|index| {
                let t = index as f32 / 400.0;
                let (x, y) = if t < 0.5 {
                    (t * 10.0, (t * 20.0).sin() * 0.01)
                } else {
                    (5.0, (1.0 - t) * -8.0)
                };
                crate::document::Point {
                    position: Vec3::new(x, y, 0.0),
                    pressure: if (index - 210).abs() < 2 {
                        0.95
                    } else {
                        0.4 + 0.1 * t
                    },
                    tilt: Some(Vec3::new((t * 0.4).sin(), 0.0, 1.0).normalize()),
                    time_ms: index as u32,
                }
            })
            .collect();
        stroke.point_thickness = Some(
            (0..stroke.points.len())
                .map(|index| 0.5 + index as f32 / stroke.points.len() as f32)
                .collect(),
        );
        stroke.points[0].position = Vec3::new(0.0, 0.0, 0.0);
        stroke.points[200].position = Vec3::new(5.0, 0.0, 0.0);
        stroke.points[201].position = Vec3::new(5.0, -0.02, 0.0);
        stroke.points[400].position = Vec3::new(5.0, -4.0, 0.0);
        let source = stroke.clone();
        let (cleaned, stats) = cleanup_stroke(&stroke, 0.25, 0.06).unwrap();
        assert_eq!(cleaned.points.first(), source.points.first());
        assert_eq!(cleaned.points.last(), source.points.last());
        let cleaned_profile = cleaned.point_thickness.as_ref().unwrap();
        assert_eq!(cleaned_profile.len(), cleaned.points.len());
        assert!((cleaned_profile[0] - 0.5).abs() < 1.0e-5);
        assert!(
            (cleaned_profile.last().unwrap()
                - source.point_thickness.as_ref().unwrap().last().unwrap())
            .abs()
                < 1.0e-4
        );
        assert!(stats.output_points < stats.input_points);
        assert!(stats.maximum_deviation <= 0.0601, "{stats:?}");
        assert!(cleaned.points.windows(2).all(|pair| {
            pair[0].position.distance(pair[1].position) <= 0.2501
                || pair[0].position.distance(pair[1].position) <= 1.0e-6
        }));
        assert!(cleaned.points.iter().any(|point| point.pressure > 0.9));
        assert!(cleaned.points.iter().all(|point| point
            .tilt
            .is_none_or(|tilt| (tilt.length() - 1.0).abs() < 1.0e-4)));
        assert!(cleaned
            .points
            .iter()
            .any(|point| point.position.distance(Vec3::X * 5.0) < 1.0e-5));
    }

    #[test]
    fn cleanup_is_reversible_and_keeps_gradient_dabs_native_recovery_and_exports() {
        let mut doc = sample();
        let source = &mut doc.strokes[0];
        source.points = (0..41)
            .map(|index| crate::document::Point {
                position: Vec3::new(index as f32 * 0.05, 0.0, 0.0),
                pressure: 0.2 + index as f32 / 80.0,
                tilt: Some(Vec3::Y),
                time_ms: index * 7,
            })
            .collect();
        source.gradient = Some(crate::document::StrokeGradient {
            start: [0.1, 0.2, 0.3],
            end: [0.8, 0.7, 0.6],
        });
        source.point_thickness = Some(
            (0..source.points.len())
                .map(|index| 0.6 + index as f32 * 0.02)
                .collect(),
        );
        source.brush.dabs.enabled = true;
        source.brush.dabs.spacing = 0.125;
        source.brush.dab_seed = 777;
        let original = doc.strokes.clone();
        let original_dabs = crate::dab_placement::sample(&doc.strokes[0]).unwrap();
        let draft = Draft::cleanup(&doc, 1, 0.2, 0.01).unwrap();
        let preview = draft.preview.clone();
        let stats = draft.cleanup_stats.unwrap();
        assert!(stats.maximum_deviation <= 0.01);
        assert_eq!(doc.strokes, original, "preview reads an immutable source");
        let edit = draft.edit(&doc).unwrap();
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0].gradient, original[0].gradient);
        assert_eq!(
            doc.strokes[0].point_thickness.as_ref().unwrap().len(),
            doc.strokes[0].points.len()
        );
        assert_eq!(doc.strokes[0].brush.dab_seed, original[0].brush.dab_seed);
        let cleaned_dabs = crate::dab_placement::sample(&doc.strokes[0]).unwrap();
        assert_eq!(cleaned_dabs.len(), original_dabs.len());
        assert!(cleaned_dabs
            .iter()
            .zip(&original_dabs)
            .all(|(a, b)| a.position.distance(b.position) <= 0.01));
        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "stroke-cleanup-test".into(),
            None,
            doc.clone(),
        );
        let recovered: crate::recovery_store::RecoverySnapshot =
            serde_json::from_slice(&serde_json::to_vec(&recovery).unwrap()).unwrap();
        assert_eq!(recovered.document.strokes, doc.strokes);
        let native = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(native.strokes, doc.strokes);
        let glb = crate::export::encode(&doc, None).unwrap().bytes;
        assert_eq!(crate::export::encode(&native, None).unwrap().bytes, glb);
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, original, "cancel/undo restores exact samples");
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes[0], preview);
    }

    #[test]
    fn cleanup_bounds_expansion_rejects_airbrush_and_handles_tiny_strokes() {
        let mut stroke = sample().strokes.remove(0);
        stroke.points = vec![
            crate::document::Point {
                position: Vec3::ZERO,
                pressure: 0.4,
                tilt: None,
                time_ms: 0,
            },
            crate::document::Point {
                position: Vec3::X * 0.002,
                pressure: 0.8,
                tilt: None,
                time_ms: 1,
            },
        ];
        let (tiny, stats) = cleanup_stroke(&stroke, 1.0, 0.0).unwrap();
        assert_eq!(tiny.points, stroke.points);
        assert_eq!(stats.maximum_deviation, 0.0);
        assert!(cleanup_stroke(&stroke, f32::NAN, 0.0).is_err());
        assert!(cleanup_stroke(&stroke, 0.01, f32::INFINITY).is_err());
        stroke.brush.flow.enabled = true;
        assert!(cleanup_stroke(&stroke, 0.1, 0.1)
            .unwrap_err()
            .contains("timestamps drive deposited coverage"));

        let mut long = sample().strokes.remove(0);
        long.points = vec![
            long.points[0],
            crate::document::Point {
                position: Vec3::X * 1000.0,
                pressure: 1.0,
                tilt: None,
                time_ms: 1,
            },
        ];
        assert!(cleanup_stroke(&long, 0.001, 0.0)
            .unwrap_err()
            .contains("point limit"));
    }

    #[test]
    fn cleanup_large_fixture_reports_error_and_cpu_cost() {
        let mut stroke = sample().strokes.remove(0);
        stroke.points = (0..20_000)
            .map(|index| {
                let t = index as f32 / 19_999.0;
                crate::document::Point {
                    position: Vec3::new(
                        10.0 * t,
                        0.05 * (t * 8.0 * std::f32::consts::PI).sin(),
                        0.0,
                    ),
                    pressure: 0.5,
                    tilt: None,
                    time_ms: index as u32,
                }
            })
            .collect();
        let (cleaned, stats) = cleanup_stroke(&stroke, 0.05, 0.015).unwrap();
        assert!(stats.output_points < stats.input_points);
        assert!(stats.maximum_deviation <= 0.0151, "{stats:?}");
        assert_eq!(cleaned.points.first(), stroke.points.first());
        assert_eq!(cleaned.points.last(), stroke.points.last());
        eprintln!(
            "CLEANUP_FIXTURE input={} output={} max_deviation={:.6} source_length={:.6} result_length={:.6} elapsed_us={}",
            stats.input_points,
            stats.output_points,
            stats.maximum_deviation,
            stats.source_length,
            stats.result_length,
            stats.elapsed_micros
        );
    }
}
