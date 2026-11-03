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
        let mut stroke_changed = false;
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
            stroke_changed |= position != point.position;
            point.position = position;
        }
        if stroke_changed {
            stroke.curve = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point};

    fn sample_document() -> Document {
        Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![Stroke {
                gradient: None,
                point_thickness: None,
                modifiers: None,
                curve: None,
                fill: None,
                material: Default::default(),
                group_id: None,
                layer_id: 1,
                id: 7,
                color: [0.2, 0.4, 0.8],
                radius: 0.05,
                brush: Brush::default(),
                points: vec![
                    Point {
                        position: Vec3::new(-0.2, 0.0, 0.0),
                        pressure: 0.3,
                        tilt: None,

                        time_ms: 0,
                    },
                    Point {
                        position: Vec3::new(0.0, 0.0, 0.0),
                        pressure: 0.7,
                        tilt: None,

                        time_ms: 0,
                    },
                    Point {
                        position: Vec3::new(0.2, 0.0, 0.0),
                        pressure: 1.0,
                        tilt: None,

                        time_ms: 0,
                    },
                ],
            }],
            ..Document::default()
        }
    }

    fn settings() -> LiquifySettings {
        LiquifySettings {
            center: Vec3::ZERO,
            displacement: Vec3::new(0.3, 0.2, 0.0),
            radius: 0.5,
            strength: 0.75,
            direction: Direction::Push,
            falloff: Falloff::Smooth,
        }
    }

    #[test]
    fn preview_is_pure_and_atomic_edit_roundtrips_exact_ink() {
        let mut document = sample_document();
        let before = document.strokes.clone();
        let ids = HashSet::from([7]);
        let preview = edit(&document, &ids, settings()).unwrap();
        assert_eq!(
            document.strokes, before,
            "Preview must not mutate the source"
        );
        let Edit::Change {
            before: original,
            after,
        } = preview.clone()
        else {
            panic!("Liquify must produce one change edit");
        };
        assert_eq!(original[0].1, before[0]);
        assert_ne!(after[0].1.points, before[0].points);
        assert_eq!(after[0].1.brush, before[0].brush);
        assert_eq!(after[0].1.color, before[0].color);
        assert_eq!(after[0].1.radius, before[0].radius);
        preview.apply(&mut document, true);
        assert_eq!(document.strokes[0], after[0].1);
        preview.apply(&mut document, false);
        assert_eq!(document.strokes, before);
    }

    #[test]
    fn hidden_locked_and_filled_ink_cannot_be_liquified() {
        let ids = HashSet::from([7]);
        let mut document = sample_document();
        document.layers[0].locked = true;
        assert!(edit(&document, &ids, settings()).is_err());
        document.layers[0].locked = false;
        document.layers[0].visible = false;
        assert!(edit(&document, &ids, settings()).is_err());
        document.layers[0].visible = true;
        document.strokes[0].fill = Some(crate::document::PlanarFill {
            offset: 0.01,
            gradient: None,
            texture: None,
        });
        assert!(edit(&document, &ids, settings()).is_err());
    }

    #[test]
    fn falloffs_and_direction_change_only_point_positions() {
        let document = sample_document();
        let before = editing::snapshot(&document, &HashSet::from([7]));
        let displacement = Vec3::X * 0.4;
        let positions: Vec<Vec3> = [Falloff::Linear, Falloff::Smooth, Falloff::Sharp]
            .into_iter()
            .map(|falloff| {
                let mut config = settings();
                config.displacement = displacement;
                config.strength = 1.0;
                config.falloff = falloff;
                preview(&before, config).unwrap()[0].1.points[0].position
            })
            .collect();
        let linear_amount = positions[0].x - before[0].1.points[0].position.x;
        let smooth_amount = positions[1].x - before[0].1.points[0].position.x;
        let sharp_amount = positions[2].x - before[0].1.points[0].position.x;
        assert!(sharp_amount < linear_amount && linear_amount < smooth_amount);

        let mut pull = settings();
        pull.direction = Direction::Pull;
        let pulled = preview(&before, pull).unwrap();
        assert_eq!(
            pulled[0].1.points[1].position,
            before[0].1.points[1].position - pull.displacement * pull.strength
        );
        assert_eq!(pulled[0].1.brush, before[0].1.brush);
        assert_eq!(
            pulled[0].1.points[1].pressure,
            before[0].1.points[1].pressure
        );
        assert_eq!(pulled[0].1.points[1].tilt, before[0].1.points[1].tilt);
    }

    #[test]
    fn deformed_ink_roundtrips_through_native_recovery_and_export() {
        let mut document = sample_document();
        let ids = HashSet::from([7]);
        let edit = edit(&document, &ids, settings()).unwrap();
        edit.apply(&mut document, true);

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        let reopened = Document::from_json(&std::fs::read_to_string(file.path()).unwrap()).unwrap();
        assert_eq!(reopened.strokes, document.strokes);

        let recovery_dir = tempfile::tempdir().unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "liquify-session".into(),
            None,
            document.clone(),
        );
        crate::recovery_store::write(recovery_dir.path(), &snapshot).unwrap();
        let recovered = crate::recovery_store::scan(recovery_dir.path(), "other-session").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes,
            document.strokes
        );

        let exported = crate::export::encode(&document, None).unwrap();
        let mesh_count = gltf::Gltf::from_slice(&exported.bytes)
            .unwrap()
            .meshes()
            .count();
        assert_eq!(mesh_count, 1);

        edit.apply(&mut document, false);
        assert_eq!(document.strokes, sample_document().strokes);
        edit.apply(&mut document, true);
        assert_eq!(document.strokes, reopened.strokes);
    }

    #[test]
    fn invalid_preview_is_atomic_and_bounded() {
        let mut before = editing::snapshot(&sample_document(), &HashSet::from([7]));
        before[0].1.points[1].position = Vec3::new(9_999.9, 0.0, 0.0);
        let mut config = settings();
        config.center = before[0].1.points[1].position;
        config.displacement = Vec3::X;
        config.strength = 1.0;
        config.radius = 1.0;
        let original = before.clone();
        assert!(preview(&before, config).is_err());
        assert_eq!(before, original);

        config.center = Vec3::ZERO;
        config.radius = 0.0;
        assert!(preview(&before, config).is_err());
    }
}
