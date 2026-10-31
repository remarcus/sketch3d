//! Bounded camera bookmarks and deterministic camera-sequence playback.
//!
//! Camera views are kept in separate local workspace data rather than drawing
//! documents: they describe how a person frames a scene, not the ink itself.
//! Sequence durations are stored on each frame as the transition time to the
//! next frame; the final frame must have a zero duration. Transitions ease
//! smoothly between views. When adjacent views use different projections, the
//! starting projection stays active through the transition and the next
//! projection takes effect exactly at the frame boundary.

use crate::geometry::{Camera, Projection};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::Write, path::Path};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_BOOKMARKS: usize = 64;
pub const MAX_SEQUENCES: usize = 32;
pub const MAX_SEQUENCE_FRAMES: usize = 64;
pub const MAX_NAME_CHARS: usize = 64;
pub const MIN_TRANSITION_SECONDS: f32 = 0.1;
pub const MAX_TRANSITION_SECONDS: f32 = 120.0;
const FILE_LIMIT: u64 = 512 * 1024;
const MIN_DISTANCE: f32 = 0.3;
const MAX_DISTANCE: f32 = 150.0;
const MAX_TARGET: f32 = 10_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedProjection {
    Perspective,
    Orthographic,
}

impl From<Projection> for SavedProjection {
    fn from(value: Projection) -> Self {
        match value {
            Projection::Perspective => Self::Perspective,
            Projection::Orthographic => Self::Orthographic,
        }
    }
}

impl From<SavedProjection> for Projection {
    fn from(value: SavedProjection) -> Self {
        match value {
            SavedProjection::Perspective => Self::Perspective,
            SavedProjection::Orthographic => Self::Orthographic,
        }
    }
}

/// Validated serializable snapshot of the camera's full view and framing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraView {
    pub projection: SavedProjection,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub distance: f32,
    pub target: [f32; 3],
}

impl CameraView {
    pub fn capture(camera: &Camera) -> Result<Self, String> {
        let view = Self {
            projection: camera.projection.into(),
            yaw: camera.yaw,
            pitch: camera.pitch,
            roll: camera.roll,
            distance: camera.distance,
            target: camera.target.to_array(),
        };
        view.validate()?;
        Ok(view)
    }

    pub fn apply(self, camera: &mut Camera) -> Result<(), String> {
        self.validate()?;
        camera.projection = self.projection.into();
        camera.yaw = self.yaw;
        camera.pitch = self.pitch;
        camera.roll = self.roll;
        camera.distance = self.distance;
        camera.target = Vec3::from_array(self.target);
        Ok(())
    }

    pub fn validate(self) -> Result<(), String> {
        if !self.yaw.is_finite()
            || !self.pitch.is_finite()
            || !self.roll.is_finite()
            || self.yaw.abs() > 1_000_000.0
            || self.roll.abs() > 1_000_000.0
            || !(-std::f32::consts::FRAC_PI_2..=std::f32::consts::FRAC_PI_2).contains(&self.pitch)
            || !self.distance.is_finite()
            || !(MIN_DISTANCE..=MAX_DISTANCE).contains(&self.distance)
            || !self.target.into_iter().all(f32::is_finite)
            || self.target.iter().any(|v| v.abs() > MAX_TARGET)
        {
            return Err(
                "Camera bookmark contains invalid position, angle, or distance values".into(),
            );
        }
        Ok(())
    }

    fn interpolate(self, next: Self, t: f32) -> Self {
        let t = smoothstep(t.clamp(0.0, 1.0));
        Self {
            projection: self.projection,
            yaw: self.yaw + shortest_angle(self.yaw, next.yaw) * t,
            pitch: self.pitch + (next.pitch - self.pitch) * t,
            roll: self.roll + shortest_angle(self.roll, next.roll) * t,
            distance: self.distance + (next.distance - self.distance) * t,
            target: [
                self.target[0] + (next.target[0] - self.target[0]) * t,
                self.target[1] + (next.target[1] - self.target[1]) * t,
                self.target[2] + (next.target[2] - self.target[2]) * t,
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraBookmark {
    pub id: u64,
    pub name: String,
    pub view: CameraView,
}

impl CameraBookmark {
    pub fn new(id: u64, name: impl Into<String>, camera: &Camera) -> Result<Self, String> {
        let bookmark = Self {
            id,
            name: name.into(),
            view: CameraView::capture(camera)?,
        };
        bookmark.validate()?;
        Ok(bookmark)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_id(self.id)?;
        validate_name(&self.name)?;
        self.view.validate()
    }
}

/// Transition duration from this bookmark to the next sequence frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SequenceFrame {
    pub bookmark_id: u64,
    pub transition_seconds: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraSequence {
    pub id: u64,
    pub name: String,
    pub frames: Vec<SequenceFrame>,
}

impl CameraSequence {
    pub fn validate(&self, bookmarks: &[CameraBookmark]) -> Result<(), String> {
        validate_id(self.id)?;
        validate_name(&self.name)?;
        if self.frames.len() < 2 || self.frames.len() > MAX_SEQUENCE_FRAMES {
            return Err(format!(
                "A camera sequence needs 2–{MAX_SEQUENCE_FRAMES} frames"
            ));
        }
        let ids: HashSet<_> = bookmarks.iter().map(|bookmark| bookmark.id).collect();
        let mut frame_ids = HashSet::new();
        for (index, frame) in self.frames.iter().enumerate() {
            if !ids.contains(&frame.bookmark_id) || !frame_ids.insert(frame.bookmark_id) {
                return Err("Camera sequence refers to a missing bookmark".into());
            }
            let last = index + 1 == self.frames.len();
            if last {
                if frame.transition_seconds != 0.0 {
                    return Err(
                        "The last camera sequence frame must have zero transition time".into(),
                    );
                }
            } else if !frame.transition_seconds.is_finite()
                || !(MIN_TRANSITION_SECONDS..=MAX_TRANSITION_SECONDS)
                    .contains(&frame.transition_seconds)
            {
                return Err(format!(
                    "Camera transitions must be between {MIN_TRANSITION_SECONDS} and {MAX_TRANSITION_SECONDS} seconds"
                ));
            }
        }
        Ok(())
    }

    pub fn duration_seconds(&self) -> f32 {
        self.frames
            .iter()
            .map(|frame| frame.transition_seconds)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraBookmarkStore {
    pub version: u32,
    pub next_id: u64,
    pub bookmarks: Vec<CameraBookmark>,
    pub sequences: Vec<CameraSequence>,
}

impl Default for CameraBookmarkStore {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            next_id: 1,
            bookmarks: Vec::new(),
            sequences: Vec::new(),
        }
    }
}

impl CameraBookmarkStore {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != FORMAT_VERSION {
            return Err("Unsupported camera bookmark format version".into());
        }
        if self.bookmarks.len() > MAX_BOOKMARKS || self.sequences.len() > MAX_SEQUENCES {
            return Err("Camera bookmark library exceeds its item limit".into());
        }
        validate_id(self.next_id)?;
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut max_id = 0;
        for bookmark in &self.bookmarks {
            bookmark.validate()?;
            if !ids.insert(bookmark.id) {
                return Err("Camera bookmark IDs must be unique".into());
            }
            if !names.insert(bookmark.name.to_lowercase()) {
                return Err("Camera bookmark names must be unique".into());
            }
            max_id = max_id.max(bookmark.id);
        }
        for sequence in &self.sequences {
            sequence.validate(&self.bookmarks)?;
            if !ids.insert(sequence.id) {
                return Err("Camera bookmark and sequence IDs must be unique".into());
            }
            if !names.insert(sequence.name.to_lowercase()) {
                return Err("Camera bookmark and sequence names must be unique".into());
            }
            max_id = max_id.max(sequence.id);
        }
        if self.next_id <= max_id {
            return Err("Camera bookmark next ID must be greater than all stored IDs".into());
        }
        Ok(())
    }

    pub fn allocate_id(&mut self) -> Result<u64, String> {
        validate_id(self.next_id)?;
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .filter(|value| *value < u64::MAX)
            .ok_or_else(|| "Camera bookmark ID space is exhausted".to_owned())?;
        Ok(id)
    }

    pub fn add_bookmark(
        &mut self,
        name: impl Into<String>,
        camera: &Camera,
    ) -> Result<u64, String> {
        if self.bookmarks.len() >= MAX_BOOKMARKS {
            return Err(format!(
                "A library can contain at most {MAX_BOOKMARKS} camera bookmarks"
            ));
        }
        let name = name.into();
        self.ensure_name_available(&name, None)?;
        let id = self.allocate_id()?;
        let bookmark = CameraBookmark::new(id, name, camera)?;
        self.bookmarks.push(bookmark);
        self.validate()?;
        Ok(id)
    }

    pub fn rename_bookmark(&mut self, id: u64, name: impl Into<String>) -> Result<(), String> {
        let name = name.into();
        self.ensure_name_available(&name, Some(id))?;
        let bookmark = self
            .bookmarks
            .iter_mut()
            .find(|bookmark| bookmark.id == id)
            .ok_or_else(|| "Camera bookmark was not found".to_owned())?;
        bookmark.name = name;
        validate_name(&bookmark.name)
    }

    pub fn remove_bookmark(&mut self, id: u64) -> Result<(), String> {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|bookmark| bookmark.id != id);
        if self.bookmarks.len() == before {
            return Err("Camera bookmark was not found".into());
        }
        for sequence in &mut self.sequences {
            sequence.frames.retain(|frame| frame.bookmark_id != id);
            if let Some(last) = sequence.frames.last_mut() {
                last.transition_seconds = 0.0;
            }
        }
        self.sequences.retain(|sequence| sequence.frames.len() >= 2);
        Ok(())
    }

    pub fn reorder_bookmarks(&mut self, order: &[u64]) -> Result<(), String> {
        if order.len() != self.bookmarks.len() {
            return Err("Bookmark order must include every saved camera exactly once".into());
        }
        let mut reordered = Vec::with_capacity(order.len());
        for id in order {
            let Some(index) = self
                .bookmarks
                .iter()
                .position(|bookmark| bookmark.id == *id)
            else {
                return Err("Bookmark order contains a missing or duplicate ID".into());
            };
            if reordered
                .iter()
                .any(|bookmark: &CameraBookmark| bookmark.id == *id)
            {
                return Err("Bookmark order contains a missing or duplicate ID".into());
            }
            reordered.push(self.bookmarks[index].clone());
        }
        self.bookmarks = reordered;
        Ok(())
    }

    pub fn rename_sequence(&mut self, id: u64, name: impl Into<String>) -> Result<(), String> {
        let name = name.into();
        self.ensure_name_available(&name, Some(id))?;
        let sequence = self
            .sequences
            .iter_mut()
            .find(|sequence| sequence.id == id)
            .ok_or_else(|| "Camera sequence was not found".to_owned())?;
        sequence.name = name;
        validate_name(&sequence.name)
    }

    pub fn remove_sequence(&mut self, id: u64) -> Result<(), String> {
        let before = self.sequences.len();
        self.sequences.retain(|sequence| sequence.id != id);
        if self.sequences.len() == before {
            return Err("Camera sequence was not found".into());
        }
        Ok(())
    }

    pub fn reorder_sequences(&mut self, order: &[u64]) -> Result<(), String> {
        if order.len() != self.sequences.len() {
            return Err("Sequence order must include every saved sequence exactly once".into());
        }
        let mut reordered = Vec::with_capacity(order.len());
        for id in order {
            let Some(index) = self
                .sequences
                .iter()
                .position(|sequence| sequence.id == *id)
            else {
                return Err("Sequence order contains a missing or duplicate ID".into());
            };
            if reordered
                .iter()
                .any(|sequence: &CameraSequence| sequence.id == *id)
            {
                return Err("Sequence order contains a missing or duplicate ID".into());
            }
            reordered.push(self.sequences[index].clone());
        }
        self.sequences = reordered;
        Ok(())
    }

    pub fn add_sequence(
        &mut self,
        name: impl Into<String>,
        frames: Vec<SequenceFrame>,
    ) -> Result<u64, String> {
        if self.sequences.len() >= MAX_SEQUENCES {
            return Err(format!(
                "A library can contain at most {MAX_SEQUENCES} camera sequences"
            ));
        }
        let name = name.into();
        self.ensure_name_available(&name, None)?;
        let id = self.allocate_id()?;
        let sequence = CameraSequence { id, name, frames };
        sequence.validate(&self.bookmarks)?;
        self.sequences.push(sequence);
        self.validate()?;
        Ok(id)
    }

    pub fn reorder_sequence_frames(
        &mut self,
        sequence_id: u64,
        bookmark_ids: &[u64],
    ) -> Result<(), String> {
        let sequence = self
            .sequences
            .iter_mut()
            .find(|sequence| sequence.id == sequence_id)
            .ok_or_else(|| "Camera sequence was not found".to_owned())?;
        if bookmark_ids.len() != sequence.frames.len() {
            return Err("Sequence order must include every frame exactly once".into());
        }
        let mut reordered = Vec::with_capacity(bookmark_ids.len());
        for id in bookmark_ids {
            let Some(index) = sequence
                .frames
                .iter()
                .position(|frame| frame.bookmark_id == *id)
            else {
                return Err("Sequence order contains a missing or duplicate bookmark".into());
            };
            if reordered
                .iter()
                .any(|frame: &SequenceFrame| frame.bookmark_id == *id)
            {
                return Err("Sequence order contains a missing or duplicate bookmark".into());
            }
            reordered.push(sequence.frames[index]);
        }
        sequence.frames = reordered;
        let transition_count = sequence.frames.len().saturating_sub(1);
        for frame in sequence.frames.iter_mut().take(transition_count) {
            if frame.transition_seconds < MIN_TRANSITION_SECONDS {
                frame.transition_seconds = 2.0;
            }
        }
        if let Some(last) = sequence.frames.last_mut() {
            last.transition_seconds = 0.0;
        }
        sequence.validate(&self.bookmarks)
    }

    pub fn sample_sequence(
        &self,
        sequence_id: u64,
        elapsed_seconds: f32,
    ) -> Result<CameraView, String> {
        self.validate()?;
        if !elapsed_seconds.is_finite() || elapsed_seconds < 0.0 {
            return Err("Sequence playback time must be finite and non-negative".into());
        }
        let sequence = self
            .sequences
            .iter()
            .find(|sequence| sequence.id == sequence_id)
            .ok_or_else(|| "Camera sequence was not found".to_owned())?;
        let mut elapsed = elapsed_seconds.min(sequence.duration_seconds());
        for (index, frame) in sequence
            .frames
            .iter()
            .take(sequence.frames.len() - 1)
            .enumerate()
        {
            let duration = frame.transition_seconds;
            let start = bookmark_view(&self.bookmarks, frame.bookmark_id)?;
            let next_frame = sequence
                .frames
                .get(index + 1)
                .ok_or_else(|| "Camera sequence frame is missing its successor".to_owned())?;
            let end = bookmark_view(&self.bookmarks, next_frame.bookmark_id)?;
            if elapsed < duration {
                return Ok(start.interpolate(end, elapsed / duration));
            }
            elapsed -= duration;
        }
        let last = sequence
            .frames
            .last()
            .ok_or_else(|| "Camera sequence is empty".to_owned())?;
        bookmark_view(&self.bookmarks, last.bookmark_id)
    }

    fn ensure_name_available(&self, name: &str, except_id: Option<u64>) -> Result<(), String> {
        validate_name(name)?;
        if self.bookmarks.iter().any(|bookmark| {
            Some(bookmark.id) != except_id && bookmark.name.eq_ignore_ascii_case(name)
        }) || self.sequences.iter().any(|sequence| {
            Some(sequence.id) != except_id && sequence.name.eq_ignore_ascii_case(name)
        }) {
            return Err("Camera bookmark and sequence names must be unique".into());
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error.to_string()),
        };
        if file.metadata().map_err(|error| error.to_string())?.len() > FILE_LIMIT {
            return Err("Camera bookmark file exceeds the 512 KiB limit".into());
        }
        let store: Self = serde_json::from_reader(file).map_err(|error| error.to_string())?;
        store.validate()?;
        Ok(store)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > FILE_LIMIT {
            return Err("Camera bookmark file exceeds the 512 KiB limit".into());
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut temp =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        temp.write_all(&bytes).map_err(|error| error.to_string())?;
        temp.as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        temp.persist(path).map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn bookmark_view(bookmarks: &[CameraBookmark], id: u64) -> Result<CameraView, String> {
    bookmarks
        .iter()
        .find(|bookmark| bookmark.id == id)
        .map(|bookmark| bookmark.view)
        .ok_or_else(|| "Camera sequence refers to a missing bookmark".to_owned())
}

fn validate_id(id: u64) -> Result<(), String> {
    if id == 0 || id == u64::MAX {
        Err("Camera bookmark ID is invalid".into())
    } else {
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    let chars = name.chars().count();
    if name.trim() != name
        || chars == 0
        || chars > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(format!(
            "Camera names must contain 1–{MAX_NAME_CHARS} printable characters"
        ));
    }
    Ok(())
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn shortest_angle(from: f32, to: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (to - from + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}
