//! Bounded keyframed animation of editable artwork strokes.
//!
//! Animation is separate from construction history and camera sequences. Each
//! keyframe stores an ordered stroke snapshot; frame seeking is deterministic
//! and never changes the keyframe list. When interpolation is enabled it only
//! blends positions and pressure for strokes with identical topology/settings.

use crate::document::Stroke;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const CURRENT_VERSION: u16 = 2;
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

/// One layer's held artwork snapshot. Exposure is measured in frames starting
/// at `frame`; an expired exposure leaves the layer empty until its next key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerArtworkKeyframe {
    pub frame: u32,
    #[serde(default = "default_exposure")]
    pub exposure: u32,
    pub strokes: Vec<Stroke>,
}

fn default_exposure() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerAnimationTrack {
    pub layer_id: u64,
    pub keyframes: Vec<LayerArtworkKeyframe>,
}

impl LayerAnimationTrack {
    /// Half-open playback interval. The first key also supplies the lead-in.
    pub fn key_span(&self, index: usize, range_start: u32, range_end: u32) -> (u32, u32) {
        let key = &self.keyframes[index];
        let start = if index == 0 { range_start } else { key.frame };
        let end = key.frame.saturating_add(key.exposure).min(
            self.keyframes
                .get(index + 1)
                .map_or(range_end + 1, |next| next.frame),
        );
        (start, end)
    }
}

/// Small cutout-rig prototype for one layer. Each source stroke has one
/// normalized child-bone weight per sampled point; the first bone moves the
/// full layer and the second bone bends the weighted distal portions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TwoBoneRig {
    pub layer_id: u64,
    pub root: glam::Vec3,
    pub joint: glam::Vec3,
    pub point_weights: Vec<StrokeBoneWeights>,
    pub poses: Vec<TwoBonePoseKeyframe>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeBoneWeights {
    pub stroke_id: u64,
    pub child_weights: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TwoBonePoseKeyframe {
    pub frame: u32,
    pub root_angle: f32,
    pub child_angle: f32,
}

impl TwoBoneRig {
    pub fn from_layer(layer_id: u64, frame: u32, strokes: &[Stroke]) -> Result<Self, String> {
        let layer_strokes = strokes
            .iter()
            .filter(|stroke| stroke.layer_id == layer_id)
            .collect::<Vec<_>>();
        let total_points = layer_strokes
            .iter()
            .map(|stroke| stroke.points.len())
            .sum::<usize>();
        if layer_strokes.is_empty() || total_points == 0 || total_points > 262_144 {
            return Err(
                "The active layer needs between 1 and 262,144 sampled points for the rig prototype"
                    .into(),
            );
        }
        let mut min = glam::Vec3::splat(f32::INFINITY);
        let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
        for point in layer_strokes.iter().flat_map(|stroke| &stroke.points) {
            min = min.min(point.position);
            max = max.max(point.position);
        }
        let root = glam::Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
        let joint = glam::Vec3::new(root.x, min.y + (max.y - min.y) * 0.55, root.z);
        if !root.is_finite() || !joint.is_finite() || root.distance(joint) < 1.0e-4 {
            return Err(
                "The active layer needs a non-zero vertical span for the two-bone sample".into(),
            );
        }
        let point_weights = layer_strokes
            .iter()
            .map(|stroke| {
                let lengths = crate::dynamics::arc_lengths(&stroke.points);
                let total = lengths.last().copied().unwrap_or(0.0);
                let child_weights = lengths
                    .iter()
                    .map(|distance| {
                        if total <= 1.0e-6 {
                            0.0
                        } else {
                            let t = distance / total;
                            let x = ((t - 0.42) / 0.16).clamp(0.0, 1.0);
                            x * x * (3.0 - 2.0 * x)
                        }
                    })
                    .collect();
                StrokeBoneWeights {
                    stroke_id: stroke.id,
                    child_weights,
                }
            })
            .collect();
        Ok(Self {
            layer_id,
            root,
            joint,
            point_weights,
            poses: vec![TwoBonePoseKeyframe {
                frame,
                root_angle: 0.0,
                child_angle: 0.0,
            }],
        })
    }

    pub fn validate(
        &self,
        layer_ids: &HashSet<u64>,
        range_start: u32,
        range_end: u32,
    ) -> Result<(), String> {
        if !layer_ids.contains(&self.layer_id)
            || !self.root.is_finite()
            || !self.joint.is_finite()
            || self.root.abs().max_element() > 10_000.0
            || self.joint.abs().max_element() > 10_000.0
            || self.root.distance(self.joint) < 1.0e-4
            || self.point_weights.is_empty()
            || self.point_weights.len() > MAX_STROKES_PER_FRAME
            || self.poses.is_empty()
            || self.poses.len() > MAX_KEYFRAMES
        {
            return Err("Two-bone rig data is outside supported limits".into());
        }
        let mut ids = HashSet::new();
        let mut total = 0usize;
        for binding in &self.point_weights {
            if binding.stroke_id == 0
                || binding.stroke_id == u64::MAX
                || !ids.insert(binding.stroke_id)
                || binding.child_weights.is_empty()
                || binding.child_weights.len() > 65_536
                || binding
                    .child_weights
                    .iter()
                    .any(|weight| !weight.is_finite() || !(0.0..=1.0).contains(weight))
            {
                return Err("Two-bone stroke weights must be finite, unique and bounded".into());
            }
            total = total.saturating_add(binding.child_weights.len());
            if total > 262_144 {
                return Err("Two-bone rig exceeds the 262,144 point-weight limit".into());
            }
        }
        let mut previous = None;
        for pose in &self.poses {
            if pose.frame < range_start
                || pose.frame > range_end
                || previous.is_some_and(|frame| frame >= pose.frame)
                || !pose.root_angle.is_finite()
                || !pose.child_angle.is_finite()
                || pose.root_angle.abs() > std::f32::consts::TAU * 10.0
                || pose.child_angle.abs() > std::f32::consts::TAU * 10.0
            {
                return Err(
                    "Two-bone pose keys must be ordered and inside the animation range".into(),
                );
            }
            previous = Some(pose.frame);
        }
        Ok(())
    }

    pub fn pose_at(&self, frame: u32) -> TwoBonePoseKeyframe {
        let upper = self.poses.partition_point(|pose| pose.frame <= frame);
        let before = upper.checked_sub(1).map(|index| self.poses[index]);
        let after = self.poses.get(upper).copied();
        match (before, after) {
            (None, Some(next)) => next,
            (Some(previous), None) => previous,
            (Some(previous), Some(next)) => {
                let t = (frame - previous.frame) as f32 / (next.frame - previous.frame) as f32;
                TwoBonePoseKeyframe {
                    frame,
                    root_angle: previous.root_angle + (next.root_angle - previous.root_angle) * t,
                    child_angle: previous.child_angle
                        + (next.child_angle - previous.child_angle) * t,
                }
            }
            (None, None) => TwoBonePoseKeyframe {
                frame,
                root_angle: 0.0,
                child_angle: 0.0,
            },
        }
    }

    pub fn set_pose(&self, frame: u32, root_angle: f32, child_angle: f32) -> Result<Self, String> {
        if !root_angle.is_finite()
            || !child_angle.is_finite()
            || root_angle.abs() > std::f32::consts::TAU * 10.0
            || child_angle.abs() > std::f32::consts::TAU * 10.0
        {
            return Err("Rig angles must be finite and within ten turns".into());
        }
        let mut next = self.clone();
        let key = TwoBonePoseKeyframe {
            frame,
            root_angle,
            child_angle,
        };
        if let Some(existing) = next.poses.iter_mut().find(|pose| pose.frame == frame) {
            *existing = key;
        } else {
            if next.poses.len() >= MAX_KEYFRAMES {
                return Err("Two-bone rig already contains 120 pose keys".into());
            }
            let index = next.poses.partition_point(|pose| pose.frame < frame);
            next.poses.insert(index, key);
        }
        Ok(next)
    }

    pub fn deform(&self, frame: u32, strokes: &mut [Stroke]) {
        let pose = self.pose_at(frame);
        let upper_rotation = glam::Quat::from_rotation_z(pose.root_angle);
        let lower_rotation = glam::Quat::from_rotation_z(pose.child_angle);
        let transformed_joint = self.root + upper_rotation * (self.joint - self.root);
        let weights: HashMap<_, _> = self
            .point_weights
            .iter()
            .map(|binding| (binding.stroke_id, binding))
            .collect();
        for stroke in strokes
            .iter_mut()
            .filter(|stroke| stroke.layer_id == self.layer_id)
        {
            let Some(binding) = weights.get(&stroke.id) else {
                continue;
            };
            if binding.child_weights.len() != stroke.points.len() {
                continue;
            }
            for (point, child_weight) in stroke.points.iter_mut().zip(&binding.child_weights) {
                let upper = self.root + upper_rotation * (point.position - self.root);
                let lower = transformed_joint + lower_rotation * (upper - transformed_joint);
                point.position = upper.lerp(lower, *child_weight);
            }
        }
    }
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
    /// Static pose used as the base for independently animated layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_strokes: Option<Vec<Stroke>>,
    #[serde(default)]
    pub layer_tracks: Vec<LayerAnimationTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_bone_rig: Option<TwoBoneRig>,
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
            base_strokes: None,
            layer_tracks: Vec::new(),
            two_bone_rig: None,
        }
    }
}

impl ArtworkAnimation {
    pub fn is_empty(&self) -> bool {
        self.keyframes.is_empty() && self.layer_tracks.is_empty()
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
            || self.layer_tracks.len() > 128
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
                    || stroke
                        .brush
                        .explicit_tip_variants()
                        .any(|variant| !tip_asset_ids.contains(&variant.asset_id))
                    || stroke.points.iter().any(|point| !point.is_valid())
                    || !stroke.point_thickness_is_valid()
                    || stroke.modifiers.as_ref().is_some_and(|stack| {
                        !stack.is_valid(stroke.points.len()) || !stack.is_compatible(stroke)
                    })
                    || stroke.curve.as_ref().is_some_and(|curve| !curve.is_valid())
                    || stroke.fill.as_ref().is_some_and(|fill| {
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
        if let Some(base) = &self.base_strokes {
            validate_snapshot(base, None, layer_ids, tip_asset_ids, &mut total_points)?;
        }
        let mut tracked_layers = HashSet::new();
        for track in &self.layer_tracks {
            if !layer_ids.contains(&track.layer_id)
                || !tracked_layers.insert(track.layer_id)
                || track.keyframes.len() > MAX_KEYFRAMES
                || track.keyframes.is_empty()
            {
                return Err("Animation layer tracks must be unique, valid, and bounded".into());
            }
            let mut previous = None;
            for keyframe in &track.keyframes {
                if keyframe.frame < self.range_start
                    || keyframe.frame > self.range_end
                    || previous.is_some_and(|frame| frame >= keyframe.frame)
                    || keyframe.exposure == 0
                    || keyframe.exposure > self.range_end.saturating_sub(keyframe.frame) + 1
                {
                    return Err("Layer keyframes must be ordered and have a valid exposure".into());
                }
                previous = Some(keyframe.frame);
                validate_snapshot(
                    &keyframe.strokes,
                    Some(track.layer_id),
                    layer_ids,
                    tip_asset_ids,
                    &mut total_points,
                )?;
            }
        }
        if !self.layer_tracks.is_empty() && self.base_strokes.is_none() {
            return Err("Layer animation is missing its static base pose".into());
        }
        if !self.layer_tracks.is_empty() && (!self.keyframes.is_empty() || self.interpolate) {
            return Err(
                "Layer animation cannot be combined with whole-drawing keys or interpolation"
                    .into(),
            );
        }
        if let Some(rig) = &self.two_bone_rig {
            rig.validate(layer_ids, self.range_start, self.range_end)?;
            let track = self
                .layer_tracks
                .iter()
                .find(|track| track.layer_id == rig.layer_id)
                .ok_or(
                    "Two-bone prototype requires an independent animation track for its layer",
                )?;
            for binding in &rig.point_weights {
                if !track.keyframes.iter().any(|key| {
                    key.strokes.iter().any(|stroke| {
                        stroke.id == binding.stroke_id
                            && stroke.points.len() == binding.child_weights.len()
                    })
                }) {
                    return Err(
                        "Two-bone weight data must match a stroke topology in the active layer"
                            .into(),
                    );
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
        if self.keyframes.is_empty() && self.layer_tracks.is_empty() {
            return None;
        }
        let frame = frame.clamp(self.range_start, self.range_end);
        let mut strokes = if !self.layer_tracks.is_empty() {
            let mut strokes = self.base_strokes.clone().unwrap_or_default();
            for track in &self.layer_tracks {
                strokes.retain(|stroke| stroke.layer_id != track.layer_id);
                let index = track.keyframes.partition_point(|key| key.frame <= frame);
                let index = index.saturating_sub(1);
                let (_, end) = track.key_span(index, self.range_start, self.range_end);
                if frame < end {
                    strokes.extend(track.keyframes[index].strokes.iter().cloned());
                }
            }
            Some(strokes)
        } else {
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
        }?;
        if let Some(rig) = &self.two_bone_rig {
            rig.deform(frame, &mut strokes);
        }
        Some(strokes)
    }

    pub fn current_pose_matches(&self, strokes: &[Stroke]) -> bool {
        self.resolve(self.current_frame)
            .is_none_or(|resolved| resolved == strokes)
    }

    pub fn set_keyframe(&self, frame: u32, strokes: &[Stroke]) -> Result<Self, String> {
        if !self.layer_tracks.is_empty() {
            return Err("Use layer keyframes while independent layer tracks are active".into());
        }
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

    #[cfg(test)]
    pub fn duplicate_keyframe(&self, source_frame: u32, target_frame: u32) -> Result<Self, String> {
        if !self.layer_tracks.is_empty() {
            return Err("Use layer exposures while independent layer tracks are active".into());
        }
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

    #[cfg(test)]
    pub fn delete_keyframe(&self, frame: u32) -> Result<Self, String> {
        if !self.layer_tracks.is_empty() {
            return Err("Delete a keyframe from the active layer track".into());
        }
        let mut next = self.clone();
        let Some(index) = next.keyframes.iter().position(|key| key.frame == frame) else {
            return Err("There is no keyframe at the current frame".into());
        };
        next.keyframes.remove(index);
        Ok(next)
    }

    pub fn set_layer_keyframe(
        &self,
        frame: u32,
        layer_id: u64,
        layer_strokes: &[Stroke],
        base: &[Stroke],
    ) -> Result<Self, String> {
        if frame < self.range_start || frame > self.range_end {
            return Err("Keyframe must be inside the animation range".into());
        }
        if layer_id == 0
            || layer_id == u64::MAX
            || layer_strokes
                .iter()
                .any(|stroke| stroke.layer_id != layer_id)
        {
            return Err("Layer keyframe must contain artwork from one valid layer".into());
        }
        let mut next = self.clone();
        if next.layer_tracks.is_empty() {
            next.base_strokes = Some(base.to_vec());
            if !next.keyframes.is_empty() {
                if next.interpolate {
                    return Err("Turn off whole-drawing interpolation before creating independent layer tracks".into());
                }
                let mut layers = base
                    .iter()
                    .map(|stroke| stroke.layer_id)
                    .chain(
                        next.keyframes
                            .iter()
                            .flat_map(|key| key.strokes.iter().map(|stroke| stroke.layer_id)),
                    )
                    .collect::<Vec<_>>();
                layers.sort_unstable();
                layers.dedup();
                let old_keys = std::mem::take(&mut next.keyframes);
                for id in layers {
                    let keyframes = old_keys
                        .iter()
                        .enumerate()
                        .map(|(index, key)| LayerArtworkKeyframe {
                            frame: key.frame,
                            exposure: old_keys
                                .get(index + 1)
                                .map(|next| next.frame.saturating_sub(key.frame))
                                .unwrap_or_else(|| {
                                    next.range_end
                                        .saturating_sub(key.frame)
                                        .saturating_add(1)
                                        .max(1)
                                }),
                            strokes: key
                                .strokes
                                .iter()
                                .filter(|stroke| stroke.layer_id == id)
                                .cloned()
                                .collect(),
                        })
                        .collect::<Vec<_>>();
                    if !keyframes.is_empty() {
                        next.layer_tracks.push(LayerAnimationTrack {
                            layer_id: id,
                            keyframes,
                        });
                    }
                }
            } else {
                next.interpolate = false;
            }
        }
        let track_index = match next
            .layer_tracks
            .iter()
            .position(|track| track.layer_id == layer_id)
        {
            Some(index) => index,
            None => {
                if next.layer_tracks.len() >= 128 {
                    return Err("Animation already contains 128 layer tracks".into());
                }
                next.layer_tracks.push(LayerAnimationTrack {
                    layer_id,
                    keyframes: Vec::new(),
                });
                next.layer_tracks.len() - 1
            }
        };
        let range_end = next.range_end;
        let track = &mut next.layer_tracks[track_index];
        if let Some(key) = track.keyframes.iter_mut().find(|key| key.frame == frame) {
            key.strokes = layer_strokes.to_vec();
        } else {
            if track.keyframes.len() >= MAX_KEYFRAMES {
                return Err("Layer track already contains 120 keyframes".into());
            }
            let exposure = track
                .keyframes
                .iter()
                .find(|key| key.frame > frame)
                .map(|key| key.frame - frame)
                .unwrap_or_else(|| range_end.saturating_sub(frame).saturating_add(1));
            let index = track.keyframes.partition_point(|key| key.frame < frame);
            track.keyframes.insert(
                index,
                LayerArtworkKeyframe {
                    frame,
                    exposure: exposure.max(1),
                    strokes: layer_strokes.to_vec(),
                },
            );
        }
        next.current_frame = frame;
        Ok(next)
    }

    pub fn place_layer_key(
        &self,
        layer_id: u64,
        from: u32,
        to: u32,
        duplicate: bool,
    ) -> Result<Self, String> {
        let mut next = self.clone();
        let track = next
            .layer_tracks
            .iter_mut()
            .find(|track| track.layer_id == layer_id)
            .ok_or("Layer has no animation track")?;
        let index = track
            .keyframes
            .iter()
            .position(|key| key.frame == from)
            .ok_or("Layer key no longer exists")?;
        if from == to && !duplicate {
            return Ok(next);
        }
        let mut key = if duplicate {
            if track.keyframes.len() >= MAX_KEYFRAMES {
                return Err("Layer track already contains 120 keyframes".into());
            }
            track.keyframes[index].clone()
        } else {
            track.keyframes.remove(index)
        };
        if to < self.range_start || to > self.range_end || key.exposure > self.range_end - to + 1 {
            return Err("The held exposure must fit inside the animation range".into());
        }
        let end = to + key.exposure;
        if track
            .keyframes
            .iter()
            .any(|other| other.frame < end && to < other.frame + other.exposure)
        {
            return Err("The destination would overlap an existing exposure".into());
        }
        key.frame = to;
        let index = track.keyframes.partition_point(|other| other.frame < to);
        track.keyframes.insert(index, key);
        if duplicate || next.current_frame == from {
            next.current_frame = to;
        }
        Ok(next)
    }

    /// Move or copy a selected set of layer keys by one signed frame offset.
    /// Every destination is validated against the complete track before any
    /// key is changed, so conflicts reject the whole batch.
    pub fn place_layer_keys(
        &self,
        layer_id: u64,
        source_frames: &[u32],
        offset: i32,
        duplicate: bool,
    ) -> Result<Self, String> {
        if source_frames.is_empty() || offset == 0 {
            return Err("Select layer keys and choose a nonzero frame offset".into());
        }
        let source_set: HashSet<u32> = source_frames.iter().copied().collect();
        if source_set.len() != source_frames.len() {
            return Err("The selected layer keys contain duplicates".into());
        }
        let mut next = self.clone();
        let track_index = next
            .layer_tracks
            .iter()
            .position(|track| track.layer_id == layer_id)
            .ok_or("The active layer has no animation track")?;
        let track = &next.layer_tracks[track_index];
        let mut selected = Vec::with_capacity(source_set.len());
        for frame in &source_set {
            selected.push(
                track
                    .keyframes
                    .iter()
                    .find(|key| key.frame == *frame)
                    .cloned()
                    .ok_or("A selected layer key no longer exists")?,
            );
        }
        if duplicate && track.keyframes.len() + selected.len() > MAX_KEYFRAMES {
            return Err("The selected keys would exceed the 120-key limit".into());
        }
        let mut destinations = Vec::with_capacity(selected.len());
        for mut key in selected {
            let destination = i64::from(key.frame) + i64::from(offset);
            if destination < i64::from(self.range_start)
                || destination > i64::from(self.range_end)
                || destination + i64::from(key.exposure) - 1 > i64::from(self.range_end)
            {
                return Err(
                    "Every destination and held exposure must fit inside the animation range"
                        .into(),
                );
            }
            key.frame = destination as u32;
            destinations.push(key);
        }
        destinations.sort_by_key(|key| key.frame);
        for pair in destinations.windows(2) {
            if pair[0].frame.saturating_add(pair[0].exposure) > pair[1].frame {
                return Err(
                    "Selected destinations overlap each other; no keys were changed".into(),
                );
            }
        }
        let track = &mut next.layer_tracks[track_index];
        let remaining: Vec<_> = track
            .keyframes
            .iter()
            .filter(|key| duplicate || !source_set.contains(&key.frame))
            .cloned()
            .collect();
        for destination in &destinations {
            let destination_end = destination.frame.saturating_add(destination.exposure);
            if remaining.iter().any(|other| {
                let other_end = other.frame.saturating_add(other.exposure);
                other.frame < destination_end && destination.frame < other_end
            }) {
                return Err(
                    "A destination overlaps an existing exposure; no keys were changed".into(),
                );
            }
        }
        let mut keyframes = remaining;
        keyframes.extend(destinations);
        keyframes.sort_by_key(|key| key.frame);
        track.keyframes = keyframes;
        Ok(next)
    }

    pub fn set_layer_exposure(
        &self,
        layer_id: u64,
        frame: u32,
        exposure: u32,
    ) -> Result<Self, String> {
        let mut next = self.clone();
        let range_end = next.range_end;
        let key = next
            .layer_tracks
            .iter_mut()
            .find(|track| track.layer_id == layer_id)
            .and_then(|track| track.keyframes.iter_mut().find(|key| key.frame == frame))
            .ok_or("There is no keyframe for this layer at the current frame")?;
        let max = range_end.saturating_sub(frame).saturating_add(1);
        if exposure == 0 || exposure > max {
            return Err("Exposure must fit inside the animation range".into());
        }
        key.exposure = exposure;
        Ok(next)
    }

    pub fn create_two_bone_rig(
        &self,
        frame: u32,
        layer_id: u64,
        layer_strokes: &[Stroke],
        base_strokes: &[Stroke],
    ) -> Result<Self, String> {
        if self.two_bone_rig.is_some() {
            return Err("Remove the existing two-bone prototype before creating another".into());
        }
        if self
            .layer_tracks
            .iter()
            .any(|track| track.layer_id == layer_id)
        {
            // A key already exists. Bind against its source snapshot so the
            // displayed rig pose is never baked into its own input geometry.
            let key = self
                .layer_tracks
                .iter()
                .find(|track| track.layer_id == layer_id)
                .and_then(|track| track.keyframes.iter().find(|key| key.frame == frame))
                .ok_or("Key the active layer before creating its two-bone rig")?;
            let rig = TwoBoneRig::from_layer(layer_id, frame, &key.strokes)?;
            let mut next = self.clone();
            next.two_bone_rig = Some(rig);
            next.current_frame = frame;
            return Ok(next);
        }
        let mut next = self.set_layer_keyframe(frame, layer_id, layer_strokes, base_strokes)?;
        let rig = TwoBoneRig::from_layer(layer_id, frame, layer_strokes)?;
        next.two_bone_rig = Some(rig);
        Ok(next)
    }

    pub fn set_two_bone_pose(
        &self,
        frame: u32,
        root_angle: f32,
        child_angle: f32,
    ) -> Result<Self, String> {
        if frame < self.range_start || frame > self.range_end {
            return Err("Rig pose key must be inside the animation range".into());
        }
        let mut next = self.clone();
        let rig = next
            .two_bone_rig
            .as_ref()
            .ok_or("Create a two-bone prototype first")?;
        let updated = rig.set_pose(frame, root_angle, child_angle)?;
        updated.validate(
            &HashSet::from([updated.layer_id]),
            self.range_start,
            self.range_end,
        )?;
        next.two_bone_rig = Some(updated);
        next.current_frame = frame;
        Ok(next)
    }

    pub fn remove_two_bone_rig(&self) -> Result<Self, String> {
        if self.two_bone_rig.is_none() {
            return Err("There is no two-bone prototype to remove".into());
        }
        let mut next = self.clone();
        next.two_bone_rig = None;
        Ok(next)
    }

    /// Apply the same point and appearance edit to the strokes whose stable
    /// IDs occur in every selected key of one layer. Held exposures are driven
    /// by these source keys, so they update automatically without creating
    /// synthetic keys at intermediate frames.
    pub fn edit_layer_keys(
        &self,
        layer_id: u64,
        frames: &[u32],
        translation: glam::Vec3,
        point_scale: f32,
        radius_scale: f32,
        tint: [f32; 3],
    ) -> Result<Self, String> {
        if frames.len() < 2 || frames.len() > 16 {
            return Err("Select between 2 and 16 keys on the active layer".into());
        }
        if !translation.is_finite()
            || translation.abs().max_element() > 10_000.0
            || !point_scale.is_finite()
            || !(0.01..=100.0).contains(&point_scale)
            || !radius_scale.is_finite()
            || !(0.01..=10.0).contains(&radius_scale)
            || tint
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=2.0).contains(value))
        {
            return Err("Multi-key edit values are outside supported limits".into());
        }
        let mut unique_frames = frames.to_vec();
        unique_frames.sort_unstable();
        unique_frames.dedup();
        if unique_frames.len() != frames.len() {
            return Err("Selected animation keys must be unique".into());
        }
        let track = self
            .layer_tracks
            .iter()
            .find(|track| track.layer_id == layer_id)
            .ok_or("The active layer has no independent animation track")?;
        let keys = unique_frames
            .iter()
            .map(|frame| {
                track
                    .keyframes
                    .iter()
                    .find(|key| key.frame == *frame)
                    .ok_or("Selected frames must be actual keys, not held exposures")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut shared_ids: HashSet<u64> = keys[0].strokes.iter().map(|stroke| stroke.id).collect();
        for key in &keys[1..] {
            let ids: HashSet<u64> = key.strokes.iter().map(|stroke| stroke.id).collect();
            shared_ids.retain(|id| ids.contains(id));
        }
        if shared_ids.is_empty() {
            return Err("Selected keys have no strokes with matching IDs".into());
        }
        let point_count = keys
            .iter()
            .flat_map(|key| key.strokes.iter())
            .filter(|stroke| shared_ids.contains(&stroke.id))
            .map(|stroke| stroke.points.len())
            .sum::<usize>();
        if point_count > 262_144 {
            return Err("Selected keys exceed the 262,144 point edit limit".into());
        }

        let mut next = self.clone();
        let next_track = next
            .layer_tracks
            .iter_mut()
            .find(|track| track.layer_id == layer_id)
            .expect("track was found above");
        for key in next_track
            .keyframes
            .iter_mut()
            .filter(|key| unique_frames.binary_search(&key.frame).is_ok())
        {
            for stroke in key
                .strokes
                .iter_mut()
                .filter(|stroke| shared_ids.contains(&stroke.id))
            {
                let center = stroke
                    .points
                    .iter()
                    .fold(glam::Vec3::ZERO, |sum, point| sum + point.position)
                    / stroke.points.len().max(1) as f32;
                for point in &mut stroke.points {
                    point.position = center + (point.position - center) * point_scale + translation;
                }
                stroke.radius = (stroke.radius * radius_scale).clamp(0.001, 1.0);
                for (channel, tint_channel) in tint.iter().enumerate() {
                    stroke.color[channel] = (stroke.color[channel] * tint_channel).clamp(0.0, 1.0);
                }
                if let Some(gradient) = &mut stroke.gradient {
                    for (channel, tint_channel) in tint.iter().enumerate() {
                        gradient.start[channel] =
                            (gradient.start[channel] * tint_channel).clamp(0.0, 1.0);
                        gradient.end[channel] =
                            (gradient.end[channel] * tint_channel).clamp(0.0, 1.0);
                    }
                }
                if let Some(colors) = &mut stroke.point_colors {
                    for color in colors.iter_mut().flatten() {
                        for (channel, tint_channel) in tint.iter().enumerate() {
                            color[channel] = (color[channel] * tint_channel).clamp(0.0, 1.0);
                        }
                    }
                }
            }
        }
        Ok(next)
    }

    pub fn duplicate_layer_keyframe(
        &self,
        layer_id: u64,
        source_frame: u32,
        target_frame: u32,
    ) -> Result<Self, String> {
        if target_frame < self.range_start || target_frame > self.range_end {
            return Err("Duplicate frame must be inside the animation range".into());
        }
        let mut next = self.clone();
        let range_end = next.range_end;
        let track = next
            .layer_tracks
            .iter_mut()
            .find(|track| track.layer_id == layer_id)
            .ok_or("The active layer has no animation track")?;
        if track.keyframes.iter().any(|key| key.frame == target_frame) {
            return Err("A layer keyframe already exists at the destination frame".into());
        }
        if track.keyframes.len() >= MAX_KEYFRAMES {
            return Err("Layer track already contains 120 keyframes".into());
        }
        let source = track
            .keyframes
            .iter()
            .find(|key| key.frame == source_frame)
            .cloned()
            .ok_or("Choose an existing layer keyframe to duplicate")?;
        let exposure = track
            .keyframes
            .iter()
            .find(|key| key.frame > target_frame)
            .map(|key| key.frame - target_frame)
            .unwrap_or_else(|| range_end.saturating_sub(target_frame).saturating_add(1));
        if let Some(previous) = track.keyframes.iter_mut().find(|key| {
            key.frame < target_frame && target_frame < key.frame.saturating_add(key.exposure)
        }) {
            previous.exposure = target_frame - previous.frame;
        }
        let index = track
            .keyframes
            .partition_point(|key| key.frame < target_frame);
        track.keyframes.insert(
            index,
            LayerArtworkKeyframe {
                frame: target_frame,
                exposure: exposure.max(1),
                strokes: source.strokes,
            },
        );
        next.current_frame = target_frame;
        Ok(next)
    }

    pub fn delete_layer_keyframe(&self, layer_id: u64, frame: u32) -> Result<Self, String> {
        let mut next = self.clone();
        let range_end = next.range_end;
        let track = next
            .layer_tracks
            .iter_mut()
            .find(|track| track.layer_id == layer_id)
            .ok_or("The active layer has no animation track")?;
        let index = track
            .keyframes
            .iter()
            .position(|key| key.frame == frame)
            .ok_or("There is no layer keyframe at the current frame")?;
        track.keyframes.remove(index);
        if index > 0 {
            let next_frame = track.keyframes.get(index).map(|key| key.frame);
            let previous = &mut track.keyframes[index - 1];
            if let Some(next_frame) = next_frame {
                previous.exposure = next_frame.saturating_sub(previous.frame).max(1);
            } else {
                previous.exposure = range_end
                    .saturating_sub(previous.frame)
                    .saturating_add(1)
                    .max(1);
            }
        }
        if track.keyframes.is_empty() {
            next.layer_tracks.retain(|track| track.layer_id != layer_id);
        }
        if next.layer_tracks.is_empty() {
            next.base_strokes = None;
        }
        Ok(next)
    }
}

fn validate_snapshot(
    strokes: &[Stroke],
    expected_layer: Option<u64>,
    layer_ids: &HashSet<u64>,
    tip_asset_ids: &HashSet<u64>,
    total_points: &mut usize,
) -> Result<(), String> {
    if strokes.len() > MAX_STROKES_PER_FRAME {
        return Err("Animation snapshot exceeds the 100,000 stroke limit".into());
    }
    let mut ids = HashSet::with_capacity(strokes.len());
    for stroke in strokes {
        if stroke.id == 0
            || stroke.id == u64::MAX
            || !ids.insert(stroke.id)
            || !layer_ids.contains(&stroke.layer_id)
            || expected_layer.is_some_and(|id| stroke.layer_id != id)
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
            || stroke
                .brush
                .explicit_tip_variants()
                .any(|variant| !tip_asset_ids.contains(&variant.asset_id))
            || stroke.points.iter().any(|point| !point.is_valid())
            || !stroke.point_thickness_is_valid()
            || stroke.modifiers.as_ref().is_some_and(|stack| {
                !stack.is_valid(stroke.points.len()) || !stack.is_compatible(stroke)
            })
            || stroke.curve.as_ref().is_some_and(|curve| !curve.is_valid())
            || stroke
                .fill
                .as_ref()
                .is_some_and(|fill| !fill.offset.is_finite() || fill.offset.abs() > 10_000.0)
        {
            return Err("Animation contains an invalid or duplicate stroke record".into());
        }
        *total_points = total_points.saturating_add(stroke.points.len());
        if *total_points > MAX_TOTAL_POINTS {
            return Err("Animation snapshots exceed the 2 million point budget".into());
        }
    }
    Ok(())
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

    #[test]
    fn duplicate_layer_key_preserves_the_source_hold_and_rejects_collisions() {
        let strokes = vec![stroke(1, 0.0, 2)];
        let animation = ArtworkAnimation::default()
            .set_layer_keyframe(3, 1, &strokes, &strokes)
            .unwrap()
            .set_layer_exposure(1, 3, 4)
            .unwrap();
        assert!(animation.place_layer_key(1, 3, 5, true).is_err());
        assert!(animation
            .place_layer_key(1, 3, animation.range_end, true)
            .is_err());
        let copied = animation.place_layer_key(1, 3, 9, true).unwrap();
        let keys = &copied.layer_tracks[0].keyframes;
        assert_eq!(keys[0], animation.layer_tracks[0].keyframes[0]);
        assert_eq!((keys[1].frame, keys[1].exposure), (9, 4));
        assert_eq!(keys[1].strokes, strokes);
        assert_eq!(copied.current_frame, 9);
    }

    #[test]
    fn timeline_retime_preserves_static_layers_holds_blank_gaps_and_undo_state() {
        let static_stroke = stroke(1, 0.0, 2);
        let mut animated_stroke = stroke(2, 0.0, 2);
        animated_stroke.layer_id = 2;
        let mut animation = ArtworkAnimation {
            range_end: 20,
            fps: 24,
            ..ArtworkAnimation::default()
        };
        animation = animation
            .set_layer_keyframe(0, 2, &[animated_stroke.clone()], &[static_stroke.clone()])
            .unwrap();
        animation = animation.set_layer_exposure(2, 0, 3).unwrap();
        let mut later_stroke = animated_stroke.clone();
        later_stroke.points[0].position.x = 5.0;
        animation = animation
            .set_layer_keyframe(5, 2, &[later_stroke], &[static_stroke.clone()])
            .unwrap()
            .set_layer_exposure(2, 5, 4)
            .unwrap();
        assert_eq!(animation.resolve(2).unwrap().len(), 2);
        assert_eq!(
            animation.resolve(3).unwrap().len(),
            1,
            "expired exposure leaves a blank interval"
        );
        assert_eq!(animation.resolve(4).unwrap().len(), 1);
        assert_eq!(animation.resolve(5).unwrap().len(), 2);

        let before = animation.clone();
        animation = animation.place_layer_key(2, 5, 7, false).unwrap();
        assert_eq!(animation.current_frame, 7);
        assert_eq!(animation.resolve(6).unwrap().len(), 1);
        assert_eq!(animation.resolve(7).unwrap().len(), 2);
        assert_eq!(
            animation.resolve(10).unwrap().len(),
            2,
            "retimed key keeps its hold"
        );
        assert_eq!(
            animation.resolve(11).unwrap().len(),
            1,
            "held span ends at the same relative offset"
        );
        assert!(animation.place_layer_key(2, 7, 1, false).is_err());

        let selected_key_edit = animation
            .edit_layer_keys(2, &[0, 7], Vec3::X, 1.0, 1.0, [1.0; 3])
            .unwrap();
        assert_eq!(
            selected_key_edit.layer_tracks[0]
                .keyframes
                .iter()
                .map(|key| key.frame)
                .collect::<Vec<_>>(),
            vec![0, 7]
        );
        assert!(selected_key_edit.resolve(7).unwrap().iter().any(|stroke| {
            stroke.layer_id == 2 && (stroke.points[0].position.x - 6.0).abs() < 1e-6
        }));

        let mut original_doc = Document::default();
        original_doc.layers.push(crate::document::Layer {
            id: 2,
            name: "Animated".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: Default::default(),
            hide_onion_skin: false,
            mask_inverted: false,
            mask_source: None,
            clip_to_below: false,
        });
        original_doc.strokes = before.resolve(before.current_frame).unwrap();
        original_doc.animation = before.clone();
        let mut changed_doc = original_doc.clone();
        changed_doc.strokes = selected_key_edit
            .resolve(selected_key_edit.current_frame)
            .unwrap();
        changed_doc.animation = selected_key_edit.clone();
        let edit = crate::editing::Edit::ArtworkAnimation {
            before: crate::editing::ArtworkAnimationState {
                animation: original_doc.animation.clone(),
                strokes: original_doc.strokes.clone(),
            },
            after: crate::editing::ArtworkAnimationState {
                animation: changed_doc.animation.clone(),
                strokes: changed_doc.strokes.clone(),
            },
        };
        let mut doc = original_doc.clone();
        edit.apply(&mut doc, true);
        assert_eq!(doc.animation, changed_doc.animation);
        assert_eq!(doc.strokes, changed_doc.strokes);
        edit.apply(&mut doc, false);
        assert_eq!(doc.animation, original_doc.animation);
        assert_eq!(doc.strokes, original_doc.strokes);
        edit.apply(&mut doc, true);
        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.animation, doc.animation);
        assert_eq!(
            reopened.animation.resolve(7).unwrap(),
            doc.animation.resolve(7).unwrap()
        );
        assert_eq!(reopened.animation.fps, 24);
    }

    #[test]
    fn batch_layer_key_move_and_duplicate_preserve_spacing_holds_and_rollback() {
        let base = vec![stroke(1, 0.0, 2)];
        let mut animation = ArtworkAnimation {
            range_end: 30,
            ..ArtworkAnimation::default()
        };
        for (frame, x) in [(0, 0.0), (5, 5.0), (10, 10.0)] {
            let mut animated = stroke(2, x, 2);
            animated.layer_id = 2;
            animation = animation
                .set_layer_keyframe(frame, 2, &[animated], &base)
                .unwrap();
            animation = animation.set_layer_exposure(2, frame, 2).unwrap();
        }

        let moved = animation.place_layer_keys(2, &[0, 5], 3, false).unwrap();
        let moved_track = &moved.layer_tracks[0].keyframes;
        assert_eq!(
            moved_track.iter().map(|key| key.frame).collect::<Vec<_>>(),
            vec![3, 8, 10]
        );
        assert_eq!(
            moved_track
                .iter()
                .map(|key| key.exposure)
                .collect::<Vec<_>>(),
            vec![2, 2, 2]
        );
        assert_eq!(moved.resolve(4).unwrap().len(), 2);
        assert_eq!(moved.resolve(5).unwrap().len(), 1);
        assert_eq!(moved.resolve(8).unwrap().len(), 2);
        assert_eq!(
            moved
                .edit_layer_keys(2, &[3, 8], Vec3::Y, 1.0, 1.0, [1.0; 3])
                .unwrap()
                .layer_tracks[0]
                .keyframes[0]
                .frame,
            3
        );

        let duplicated = animation.place_layer_keys(2, &[0, 5], 15, true).unwrap();
        assert_eq!(
            duplicated.layer_tracks[0]
                .keyframes
                .iter()
                .map(|key| key.frame)
                .collect::<Vec<_>>(),
            vec![0, 5, 10, 15, 20]
        );
        assert_eq!(
            duplicated.layer_tracks[0].keyframes[0].strokes,
            duplicated.layer_tracks[0].keyframes[3].strokes
        );
        assert_eq!(
            duplicated.layer_tracks[0].keyframes[1].exposure,
            duplicated.layer_tracks[0].keyframes[4].exposure
        );

        assert!(animation.place_layer_keys(2, &[0, 5], 1, true).is_err());
        assert!(animation.place_layer_keys(2, &[0, 5], 25, true).is_err());
        assert_eq!(
            animation.layer_tracks[0]
                .keyframes
                .iter()
                .map(|key| key.frame)
                .collect::<Vec<_>>(),
            vec![0, 5, 10]
        );

        let mut doc = Document::default();
        doc.layers.push(crate::document::Layer {
            id: 2,
            name: "Animated".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: Default::default(),
            hide_onion_skin: false,
            mask_inverted: false,
            mask_source: None,
            clip_to_below: false,
        });
        doc.animation = animation.clone();
        doc.strokes = animation.resolve(animation.current_frame).unwrap();
        let before = crate::editing::ArtworkAnimationState {
            animation: doc.animation.clone(),
            strokes: doc.strokes.clone(),
        };
        let after = crate::editing::ArtworkAnimationState {
            animation: moved.clone(),
            strokes: moved.resolve(moved.current_frame).unwrap(),
        };
        let edit = crate::editing::Edit::ArtworkAnimation { before, after };
        edit.apply(&mut doc, true);
        assert_eq!(doc.animation, moved);
        edit.apply(&mut doc, false);
        assert_eq!(doc.animation, animation);
    }

    fn stroke(id: u64, x: f32, points: usize) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
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
            point_colors: None,
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
        let mut animation = ArtworkAnimation {
            range_end: 20,
            keyframes: vec![keyframe(0, 0.0), keyframe(10, 1.0)],
            ..ArtworkAnimation::default()
        };
        assert_eq!(animation.resolve(6).unwrap()[0].points[1].position.x, 0.1);
        animation.interpolate = true;
        let first = animation.resolve(6).unwrap();
        assert!((first[0].points[1].position.x - 0.7).abs() < 1.0e-6);
        assert_eq!(first, animation.resolve(6).unwrap());
    }

    #[test]
    fn incompatible_topology_holds_previous_keyframe() {
        let animation = ArtworkAnimation {
            range_end: 20,
            interpolate: true,
            keyframes: vec![
                keyframe(0, 0.0),
                ArtworkKeyframe {
                    frame: 10,
                    strokes: vec![stroke(7, 1.0, 4)],
                },
            ],
            ..ArtworkAnimation::default()
        };
        assert_eq!(
            animation.resolve(5).unwrap(),
            animation.keyframes[0].strokes
        );
    }

    #[test]
    fn keyframe_operations_are_sorted_bounded_and_roundtrip_legacy_defaults() {
        let mut animation = ArtworkAnimation {
            range_end: 20,
            ..ArtworkAnimation::default()
        };
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
        let document = Document {
            animation: animation.clone(),
            ..Document::default()
        };
        let bytes = serde_json::to_vec(&document).unwrap();
        let reopened = Document::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(reopened.animation, animation);
        let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        legacy.as_object_mut().unwrap().remove("animation");
        legacy["version"] = serde_json::json!(24);
        let legacy = Document::from_json(&legacy.to_string()).unwrap();
        assert!(legacy.animation.is_empty());
        assert_eq!(legacy.version, 30);
    }
}
