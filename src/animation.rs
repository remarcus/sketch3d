//! Bounded keyframed animation of editable artwork strokes.
//!
//! Animation is separate from construction history and camera sequences. Each
//! keyframe stores an ordered stroke snapshot; frame seeking is deterministic
//! and never changes the keyframe list. When interpolation is enabled it only
//! blends positions and pressure for strokes with identical topology/settings.

use crate::document::Stroke;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const CURRENT_VERSION: u16 = 1;
pub const MAX_KEYFRAMES: usize = 120;
pub const MAX_FRAME: u32 = 100_000;
pub const MAX_TOTAL_POINTS: usize = 2_000_000;
pub const MAX_STROKES_PER_FRAME: usize = 100_000;
pub const MIN_FPS: u8 = 1;
pub const MAX_FPS: u8 = 60;
pub const MAX_TRACK_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtworkKeyframe {
    pub frame: u32,
    pub strokes: Vec<Stroke>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtworkAnimation {
    #[serde(default = "version_default")]
    pub version: u16,
    #[serde(default = "default_fps")]
    pub fps: u8,
    #[serde(default)]
    pub range_start: u32,
    #[serde(default = "default_range_end")]
    pub range_end: u32,
    #[serde(default)]
    pub current_frame: u32,
    #[serde(default)]
    pub interpolate: bool,
    #[serde(default)]
    pub onion_skins: bool,
    #[serde(default)]
    pub keyframes: Vec<ArtworkKeyframe>,
}

fn version_default() -> u16 {
    CURRENT_VERSION
}

fn default_range_end() -> u32 {
    120
}

fn default_fps() -> u8 {
    12
}

impl Default for ArtworkAnimation {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            fps: 12,
            range_start: 0,
            range_end: 120,
            current_frame: 0,
            interpolate: false,
            onion_skins: false,
            keyframes: Vec::new(),
        }
    }
}

impl ArtworkAnimation {
    pub fn is_empty(&self) -> bool {
        self.keyframes.is_empty()
    }

    pub fn frame_count(&self) -> usize {
        self.range_end
            .saturating_sub(self.range_start)
            .saturating_add(1) as usize
    }

    pub fn duration_seconds(&self) -> f32 {
        self.frame_count() as f32 / self.fps.max(1) as f32
    }

    pub fn validate(
        &self,
        layer_ids: &HashSet<u64>,
        tip_asset_ids: &HashSet<u64>,
    ) -> Result<(), String> {
        if self.version != CURRENT_VERSION
            || !(MIN_FPS..=MAX_FPS).contains(&self.fps)
            || self.range_start > self.range_end
            || self.range_end > MAX_FRAME
            || self.current_frame < self.range_start
            || self.current_frame > self.range_end
            || self.keyframes.len() > MAX_KEYFRAMES
        {
            return Err("Animation settings or keyframe count are outside supported limits".into());
        }
        let mut previous = None;
        let mut total_points = 0usize;
        for keyframe in &self.keyframes {
            if keyframe.frame < self.range_start
                || keyframe.frame > self.range_end
                || previous.is_some_and(|frame| frame >= keyframe.frame)
                || keyframe.strokes.len() > MAX_STROKES_PER_FRAME
            {
                return Err(
                    "Animation keyframes must be ordered, unique, and within the frame range"
                        .into(),
                );
            }
            previous = Some(keyframe.frame);
            let mut ids = HashSet::with_capacity(keyframe.strokes.len());
            for stroke in &keyframe.strokes {
                if stroke.id == 0
                    || stroke.id == u64::MAX
                    || !ids.insert(stroke.id)
                    || !layer_ids.contains(&stroke.layer_id)
                    || !stroke.radius.is_finite()
                    || !(0.001..=1.0).contains(&stroke.radius)
                    || stroke
                        .color
                        .iter()
                        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                    || stroke.gradient.is_some_and(|gradient| !gradient.is_valid())
                    || !stroke.brush.is_valid()
                    || (stroke.brush.tip_asset_id != 0
                        && !tip_asset_ids.contains(&stroke.brush.tip_asset_id))
                    || stroke.points.iter().any(|point| !point.is_valid())
                    || !stroke.point_thickness_is_valid()
                    || stroke.modifiers.as_ref().is_some_and(|stack| {
                        !stack.is_valid(stroke.points.len()) || !stack.is_compatible(stroke)
                    })
                    || stroke.curve.as_ref().is_some_and(|curve| !curve.is_valid())
                    || stroke.fill.is_some_and(|fill| {
                        !fill.offset.is_finite() || fill.offset.abs() > 10_000.0
                    })
                {
                    return Err("Animation contains an invalid or duplicate stroke record".into());
                }
                total_points = total_points.saturating_add(stroke.points.len());
                if total_points > MAX_TOTAL_POINTS {
                    return Err("Animation snapshots exceed the 2 million point budget".into());
                }
            }
        }
        // A conservative count bound is checked before JSON sizing, keeping
        // malformed imported animation from requesting unbounded allocation.
        let encoded = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        if encoded.len() > MAX_TRACK_BYTES {
            return Err("Animation exceeds the 64 MiB track size budget".into());
        }
        Ok(())
    }

    /// Resolve a frame without mutating the stored keyframes.
    pub fn resolve(&self, frame: u32) -> Option<Vec<Stroke>> {
        if self.keyframes.is_empty() {
            return None;
        }
        let frame = frame.clamp(self.range_start, self.range_end);
        let upper = self.keyframes.partition_point(|key| key.frame <= frame);
        let before = upper.checked_sub(1).map(|index| &self.keyframes[index]);
        let after = self.keyframes.get(upper);
        match (before, after) {
            (None, Some(next)) => Some(next.strokes.clone()),
            (Some(previous), None) => Some(previous.strokes.clone()),
            (Some(previous), Some(_next)) if previous.frame == frame => {
                Some(previous.strokes.clone())
            }
            (Some(previous), Some(next)) if self.interpolate => {
                let t = (frame - previous.frame) as f32 / (next.frame - previous.frame) as f32;
                interpolate_compatible(&previous.strokes, &next.strokes, t)
                    .or_else(|| Some(previous.strokes.clone()))
            }
            (Some(previous), Some(_)) => Some(previous.strokes.clone()),
            (None, None) => None,
        }
    }

    pub fn current_pose_matches(&self, strokes: &[Stroke]) -> bool {
        self.resolve(self.current_frame)
            .is_none_or(|resolved| resolved == strokes)
    }

    pub fn set_keyframe(&self, frame: u32, strokes: &[Stroke]) -> Result<Self, String> {
        let mut next = self.clone();
        next.current_frame = frame;
        if frame < next.range_start || frame > next.range_end {
            return Err("Keyframe must be inside the animation range".into());
        }
        if let Some(existing) = next.keyframes.iter_mut().find(|key| key.frame == frame) {
            existing.strokes = strokes.to_vec();
        } else {
            if next.keyframes.len() >= MAX_KEYFRAMES {
                return Err("Animation already contains 120 keyframes".into());
            }
            let index = next.keyframes.partition_point(|key| key.frame < frame);
            next.keyframes.insert(
                index,
                ArtworkKeyframe {
                    frame,
                    strokes: strokes.to_vec(),
                },
            );
        }
        Ok(next)
    }

    pub fn duplicate_keyframe(&self, source_frame: u32, target_frame: u32) -> Result<Self, String> {
        if target_frame < self.range_start || target_frame > self.range_end {
            return Err("Duplicate frame must be inside the animation range".into());
        }
        if self.keyframes.iter().any(|key| key.frame == target_frame) {
            return Err("A keyframe already exists at the destination frame".into());
        }
        let source = self
            .keyframes
            .iter()
            .find(|key| key.frame == source_frame)
            .ok_or("Choose an existing keyframe to duplicate")?;
        let mut next = self.clone();
        if next.keyframes.len() >= MAX_KEYFRAMES {
            return Err("Animation already contains 120 keyframes".into());
        }
        let index = next
            .keyframes
            .partition_point(|key| key.frame < target_frame);
        next.keyframes.insert(
            index,
            ArtworkKeyframe {
                frame: target_frame,
                strokes: source.strokes.clone(),
            },
        );
        next.current_frame = target_frame;
        Ok(next)
    }

    pub fn delete_keyframe(&self, frame: u32) -> Result<Self, String> {
        let mut next = self.clone();
        let Some(index) = next.keyframes.iter().position(|key| key.frame == frame) else {
            return Err("There is no keyframe at the current frame".into());
        };
        next.keyframes.remove(index);
        Ok(next)
    }
}

/// Insert/update snapshots in the same vector order as the source. If any
/// record differs beyond ID/topology/appearance compatibility, interpolation
/// is refused for the entire scene and the preceding keyframe is held.
fn interpolate_compatible(before: &[Stroke], after: &[Stroke], t: f32) -> Option<Vec<Stroke>> {
    if before.len() != after.len() {
        return None;
    }
    let mut result = before.to_vec();
    for ((result, left), right) in result.iter_mut().zip(before).zip(after) {
        if !compatible_stroke(left, right) {
            return None;
        }
        for ((output, a), b) in result
            .points
            .iter_mut()
            .zip(&left.points)
            .zip(&right.points)
        {
            output.position = a.position.lerp(b.position, t);
            output.pressure = a.pressure + (b.pressure - a.pressure) * t;
        }
    }
    Some(result)
}

fn compatible_stroke(left: &Stroke, right: &Stroke) -> bool {
    left.id == right.id
        && left.layer_id == right.layer_id
        && left.group_id == right.group_id
        && left.material == right.material
        && left.color == right.color
        && left.radius == right.radius
        && left.brush == right.brush
        && left.gradient == right.gradient
        && left.fill == right.fill
        && left.curve == right.curve
        && left.point_thickness == right.point_thickness
        && left.modifiers == right.modifiers
        // Semantic curve anchors are tied to endpoint coordinates. Keep such
        // strokes held until both geometry and anchor metadata can be blended.
        && left.curve.is_none()
        && right.curve.is_none()
        && left.points.len() == right.points.len()
        && left
            .points
            .iter()
            .zip(&right.points)
            .all(|(a, b)| a.tilt == b.tilt && a.time_ms == b.time_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Document, Point};
    use glam::Vec3;

    fn stroke(id: u64, x: f32, points: usize) -> Stroke {
        Stroke {
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id,
            color: [0.2, 0.4, 0.6],
            gradient: None,
            radius: 0.05,
            brush: Brush::default(),
            points: (0..points)
                .map(|index| Point {
                    position: Vec3::new(index as f32 * 0.1 + x, 0.0, 0.0),
                    pressure: 0.5,
                    tilt: None,
                    time_ms: index as u32,
                })
                .collect(),
            point_thickness: None,
            modifiers: None,
            curve: None,
        }
    }

    fn keyframe(frame: u32, x: f32) -> ArtworkKeyframe {
        ArtworkKeyframe {
            frame,
            strokes: vec![stroke(7, x, 3)],
        }
    }

    #[test]
    fn held_and_compatible_interpolated_seeking_is_deterministic() {
        let mut animation = ArtworkAnimation::default();
        animation.range_end = 20;
        animation.keyframes = vec![keyframe(0, 0.0), keyframe(10, 1.0)];
        assert_eq!(animation.resolve(6).unwrap()[0].points[1].position.x, 0.1);
        animation.interpolate = true;
        let first = animation.resolve(6).unwrap();
        assert!((first[0].points[1].position.x - 0.7).abs() < 1.0e-6);
        assert_eq!(first, animation.resolve(6).unwrap());
    }

    #[test]
    fn incompatible_topology_holds_previous_keyframe() {
        let mut animation = ArtworkAnimation::default();
        animation.range_end = 20;
        animation.interpolate = true;
        animation.keyframes = vec![
            keyframe(0, 0.0),
            ArtworkKeyframe {
                frame: 10,
                strokes: vec![stroke(7, 1.0, 4)],
            },
        ];
        assert_eq!(
            animation.resolve(5).unwrap(),
            animation.keyframes[0].strokes
        );
    }

    #[test]
    fn keyframe_operations_are_sorted_bounded_and_roundtrip_legacy_defaults() {
        let mut animation = ArtworkAnimation::default();
        animation.range_end = 20;
        animation = animation.set_keyframe(10, &[stroke(7, 1.0, 3)]).unwrap();
        animation = animation.set_keyframe(0, &[stroke(7, 0.0, 3)]).unwrap();
        assert_eq!(
            animation
                .keyframes
                .iter()
                .map(|key| key.frame)
                .collect::<Vec<_>>(),
            [0, 10]
        );
        animation = animation.duplicate_keyframe(10, 15).unwrap();
        assert_eq!(animation.current_frame, 15);
        animation = animation.delete_keyframe(15).unwrap();
        assert_eq!(animation.keyframes.len(), 2);
        let mut document = Document::default();
        document.animation = animation.clone();
        let bytes = serde_json::to_vec(&document).unwrap();
        let reopened = Document::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(reopened.animation, animation);
        let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        legacy.as_object_mut().unwrap().remove("animation");
        legacy["version"] = serde_json::json!(24);
        let legacy = Document::from_json(&legacy.to_string()).unwrap();
        assert!(legacy.animation.is_empty());
        assert_eq!(legacy.version, 25);
    }
}
