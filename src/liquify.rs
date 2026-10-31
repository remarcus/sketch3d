//! Bounded, reversible point deformation for SKETCH-046.
//!
//! The helper edits only positions in an existing selection snapshot. Every
//! brush property, pressure sample, tilt sample, group, layer, and stroke ID
//! remains byte-for-byte unchanged. Call `preview` repeatedly from the same
//! original snapshot, then wrap its result in `Edit::Change` for one-step
//! apply/undo or discard it to cancel.

use crate::{
    document::{Document, Stroke},
    editing::{self, Edit, Snapshot},
};
use glam::Vec3;
use std::collections::HashSet;

pub const MAX_STROKES: usize = 100_000;
pub const MAX_POINTS: usize = 2_000_000;
pub const MAX_RADIUS: f32 = 10_000.0;
pub const MAX_DISPLACEMENT: f32 = 10_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Push,
    Pull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    Linear,
    Smooth,
    Sharp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiquifySettings {
    /// Brush center in world space.
    pub center: Vec3,
    /// Pointer or stylus displacement in world space. Push follows this vector;
    /// Pull uses its inverse.
    pub displacement: Vec3,
    /// Influence radius in world units.
    pub radius: f32,
    /// Maximum fraction of the pointer displacement applied at the center.
    pub strength: f32,
    pub direction: Direction,
    pub falloff: Falloff,
}

impl LiquifySettings {
    pub fn validate(self) -> Result<(), String> {
        if !self.center.is_finite() || self.center.abs().max_element() > 10_000.0 {
            return Err("Liquify brush center is outside document bounds".into());
        }
        if !self.displacement.is_finite() || self.displacement.length() > MAX_DISPLACEMENT {
            return Err(
                "Liquify displacement must be finite and no longer than 10000 units".into(),
            );
        }
        if !self.radius.is_finite() || !(0.001..=MAX_RADIUS).contains(&self.radius) {
            return Err("Liquify radius must be between 0.001 and 10000 world units".into());
        }
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err("Liquify strength must be between 0 and 1".into());
        }
        Ok(())
    }
}

/// Build the atomic edit for a set of visible, unlocked selected strokes.
pub fn edit(
    document: &Document,
    selected: &HashSet<u64>,
    settings: LiquifySettings,
) -> Result<Edit, String> {
    if selected.is_empty() {
        return Err("Select one or more editable strokes to liquify".into());
    }
    let before = editing::snapshot(document, selected);
    if before.len() != selected.len() {
        return Err("Hidden, locked, or missing strokes cannot be liquified".into());
    }
    let after = preview(&before, settings)?;
    Ok(Edit::Change { before, after })
}

/// Recompute a deformation preview from an immutable original selection.
/// Filled surfaces are intentionally excluded; they have no ordinary point
/// path to deform. The returned snapshot is all-or-nothing on invalid bounds.
pub fn preview(before: &Snapshot, settings: LiquifySettings) -> Result<Snapshot, String> {
    settings.validate()?;
    validate_snapshot(before)?;
    if settings.displacement.length_squared() <= f32::EPSILON || settings.strength == 0.0 {
        return Ok(before.clone());
    }

    let sign = if settings.direction == Direction::Push {
        1.0
    } else {
        -1.0
    };
    let delta = settings.displacement * (settings.strength * sign);
    let radius_squared = settings.radius * settings.radius;
    let mut result = before.clone();
    let mut changed = false;
    for (_, stroke) in &mut result {
        for point in &mut stroke.points {
            let distance_squared = point.position.distance_squared(settings.center);
            if distance_squared >= radius_squared {
                continue;
            }
            let normalized = (distance_squared.sqrt() / settings.radius).clamp(0.0, 1.0);
            let influence = match settings.falloff {
                Falloff::Linear => 1.0 - normalized,
                Falloff::Smooth => 1.0 - normalized * normalized * (3.0 - 2.0 * normalized),
                Falloff::Sharp => (1.0 - normalized).powi(2),
            };
            if influence <= 0.0 {
                continue;
            }
            let position = point.position + delta * influence;
            if !position.is_finite() || position.abs().max_element() > 10_000.0 {
                return Err("Liquify preview would move points outside document bounds".into());
            }
            changed |= position != point.position;
            point.position = position;
        }
    }
    if !changed {
        return Err("The liquify brush did not touch any selected stroke points".into());
    }
    Ok(result)
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.is_empty() || snapshot.len() > MAX_STROKES {
        return Err("Liquify selection is empty or exceeds the stroke limit".into());
    }
    let mut points = 0usize;
    let mut ids = HashSet::with_capacity(snapshot.len());
    for (_, stroke) in snapshot {
        if !ids.insert(stroke.id) {
            return Err("Liquify selection contains duplicate stroke IDs".into());
        }
        if stroke.fill.is_some() {
            return Err("Liquify currently supports ordinary strokes, not filled surfaces".into());
        }
        validate_stroke(stroke)?;
        points = points.saturating_add(stroke.points.len());
        if points > MAX_POINTS {
            return Err("Liquify selection exceeds the 2 million point limit".into());
        }
    }
    Ok(())
}

fn validate_stroke(stroke: &Stroke) -> Result<(), String> {
    if stroke.id == 0
        || stroke.id == u64::MAX
        || !stroke.radius.is_finite()
        || !(0.001..=1.0).contains(&stroke.radius)
        || stroke
            .color
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || !stroke.brush.is_valid()
        || stroke.points.is_empty()
        || stroke.points.iter().any(|point| !point.is_valid())
    {
        return Err("Liquify selection contains invalid stroke data".into());
    }
    Ok(())
}
