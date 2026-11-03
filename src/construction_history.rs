//! Optional, bounded, durable construction-state recording for timelapse export.
//!
//! Each committed event stores a validated document snapshot. That makes replay
//! independent of today's selection or stroke ordering and records undo/imports
//! as they actually happened. Recording excludes unfinished gestures.
use crate::{
    camera_bookmarks::CameraView,
    document::{Brush, Document, Point, Stroke},
    geometry::Camera,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const FORMAT_VERSION: u32 = 3;
const MAX_HISTORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_HISTORY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EVENTS: usize = 200;
const CAMERA_SAMPLE_INTERVAL_MS: u64 = 100;
const TOOL_SETTINGS_SAMPLE_INTERVAL_MS: u64 = 100;
const MAX_STROKE_SAMPLE_POINTS: usize = 4096;
const MAX_ESTIMATED_BYTES: usize = 56 * 1024 * 1024;
const MAX_EVENT_NAME_CHARS: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub elapsed_ms: u64,
    pub kind: String,
    #[serde(default)]
    pub action: HistoryAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_samples: Option<StrokeReplaySamples>,
    pub document: Document,
}

/// A bounded sample of the committed world-space stroke geometry, not raw pen input.
#[derive(Clone, Serialize, Deserialize)]
pub struct StrokeReplaySamples {
    pub start_elapsed_ms: u64,
    pub stroke_id: u64,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSettingsSnapshot {
    pub tool: String,
    pub radius: f32,
    pub color: [f32; 3],
    pub stabilization: f32,
    pub brush: Brush,
}

impl ToolSettingsSnapshot {
    fn is_valid(&self) -> bool {
        matches!(
            self.tool.as_str(),
            "brush" | "line" | "curve" | "shape" | "fill" | "eraser" | "select"
        ) && self.radius.is_finite()
            && self.radius > 0.0
            && self.radius <= 10_000.0
            && self
                .color
                .iter()
                .all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel))
            && self.stabilization.is_finite()
            && (0.0..=1.0).contains(&self.stabilization)
            && self.brush.is_valid()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    #[default]
    SceneEdit,
    Draw,
    ModelDraw,
    Erase,
    Undo,
    Redo,
    Import,
}

impl HistoryAction {
    fn from_label(label: &str) -> Self {
        match label {
            "Draw" => Self::Draw,
            "Draw on model" => Self::ModelDraw,
            "Erase" => Self::Erase,
            "Undo" => Self::Undo,
            "Redo" => Self::Redo,
            "Insert drawing" | "Import image plane" | "Import OBJ reference" => Self::Import,
            _ => Self::SceneEdit,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RecordingOverhead {
    pub events: u64,
    total_nanoseconds: u128,
    pub max_nanoseconds: u64,
}

impl RecordingOverhead {
    fn add(&mut self, elapsed: std::time::Duration) {
        self.events = self.events.saturating_add(1);
        let nanos = elapsed.as_nanos();
        self.total_nanoseconds = self.total_nanoseconds.saturating_add(nanos);
        self.max_nanoseconds = self.max_nanoseconds.max(nanos.min(u64::MAX as u128) as u64);
    }

    pub fn average_microseconds(self) -> f64 {
        if self.events == 0 {
            0.0
        } else {
            self.total_nanoseconds as f64 / self.events as f64 / 1_000.0
        }
    }
}

// Persisted history records intentionally carry bounded document snapshots.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum Line {
    Header {
        version: u32,
        recording_id: String,
        source_key: String,
        started_at_ms: u64,
        baseline: Document,
        #[serde(default)]
        baseline_camera: Option<CameraView>,
        #[serde(default)]
        baseline_tool_settings: Option<ToolSettingsSnapshot>,
    },
    Event(HistoryEvent),
    Camera {
        elapsed_ms: u64,
        view: CameraView,
    },
    ToolSettings {
        elapsed_ms: u64,
        settings: ToolSettingsSnapshot,
    },
    Pause {
        elapsed_ms: u64,
    },
    Resume {
        elapsed_ms: u64,
    },
    End {
        ended_at_ms: u64,
        #[serde(default)]
        elapsed_ms: Option<u64>,
    },
}

// Queue entries intentionally carry one bounded document snapshot.
#[allow(clippy::large_enum_variant)]
enum WriteCommand {
    Event(HistoryEvent),
    Camera {
        elapsed_ms: u64,
        view: CameraView,
    },
    ToolSettings {
        elapsed_ms: u64,
        settings: ToolSettingsSnapshot,
    },
    Pause(u64),
    Resume(u64),
    End(u64),
}

pub struct ConstructionHistory {
    storage_dir: Option<PathBuf>,
    path: Option<PathBuf>,
    recording_id: Option<String>,
    source_key: Option<String>,
    started_at_ms: u64,
    last_elapsed_ms: u64,
    paused_total_ms: u64,
    pause_started_at_ms: Option<u64>,
    elapsed_duration_ms: u64,
    baseline: Option<Document>,
    baseline_camera: Option<CameraView>,
    camera_events: Vec<(u64, CameraView)>,
    last_camera_sample_ms: u64,
    baseline_tool_settings: Option<ToolSettingsSnapshot>,
    tool_settings_events: Vec<(u64, ToolSettingsSnapshot)>,
    last_tool_settings_sample_ms: u64,
    events: Vec<HistoryEvent>,
    estimated_bytes: usize,
    overhead: RecordingOverhead,
    recording: bool,
    paused: bool,
    interrupted: bool,
    writer: Option<SyncSender<WriteCommand>>,
    writer_errors: Option<Receiver<String>>,
    pub notice: Option<String>,
}

impl ConstructionHistory {
    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            storage_dir: None,
            path: None,
            recording_id: None,
            source_key: None,
            started_at_ms: 0,
            last_elapsed_ms: 0,
            paused_total_ms: 0,
            pause_started_at_ms: None,
            elapsed_duration_ms: 0,
            baseline: None,
            baseline_camera: None,
            camera_events: Vec::new(),
            last_camera_sample_ms: 0,
            baseline_tool_settings: None,
            tool_settings_events: Vec::new(),
            last_tool_settings_sample_ms: 0,
            events: Vec::new(),
            estimated_bytes: 0,
            overhead: RecordingOverhead::default(),
            recording: false,
            paused: false,
            interrupted: false,
            writer: None,
            writer_errors: None,
            notice: None,
        }
    }

    pub fn load() -> Self {
        let storage_dir = crate::presets::path()
            .ok()
            .map(|path| path.with_file_name("construction-history"));
        let path = storage_dir.as_deref().and_then(latest_recording);
        let mut history = Self {
            storage_dir,
            path,
            recording_id: None,
            source_key: None,
            started_at_ms: 0,
            last_elapsed_ms: 0,
            paused_total_ms: 0,
            pause_started_at_ms: None,
            elapsed_duration_ms: 0,
            baseline: None,
            baseline_camera: None,
            camera_events: Vec::new(),
            last_camera_sample_ms: 0,
            baseline_tool_settings: None,
            tool_settings_events: Vec::new(),
            last_tool_settings_sample_ms: 0,
            events: Vec::new(),
            estimated_bytes: 0,
            overhead: RecordingOverhead::default(),
            recording: false,
            paused: false,
            interrupted: false,
            writer: None,
            writer_errors: None,
            notice: None,
        };
        if let Some(path) = history.path.clone() {
            if let Err(error) = history.read(&path) {
                history.notice = Some(format!("Construction history: {error}"));
            }
        }
        history
    }

    fn read(&mut self, path: &Path) -> Result<(), String> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_HISTORY_BYTES {
            return Err("recording exceeds its 64 MiB storage limit".into());
        }
        let mut lines = BufReader::new(file).lines();
        let header_line = lines
            .next()
            .ok_or("recording header is missing")?
            .map_err(|error| error.to_string())?;
        let Line::Header {
            version,
            recording_id,
            source_key,
            started_at_ms,
            baseline,
            baseline_camera,
            baseline_tool_settings,
        } = serde_json::from_str(&header_line)
            .map_err(|error| format!("invalid recording header: {error}"))?
        else {
            return Err("recording does not start with a header".into());
        };
        if !(1..=FORMAT_VERSION).contains(&version)
            || recording_id.is_empty()
            || source_key.is_empty()
        {
            return Err("recording header has an unsupported version or identity".into());
        }
        validate_document(&baseline)?;
        self.estimated_bytes = estimate_document(&baseline);
        if self.estimated_bytes > MAX_ESTIMATED_BYTES {
            return Err("recording baseline exceeds its in-memory replay limit".into());
        }
        self.recording_id = Some(recording_id);
        self.source_key = Some(source_key);
        self.started_at_ms = started_at_ms;
        self.baseline = Some(baseline);
        self.baseline_camera = baseline_camera;
        self.camera_events.clear();
        self.last_camera_sample_ms = 0;
        if baseline_tool_settings
            .as_ref()
            .is_some_and(|settings| !settings.is_valid())
        {
            return Err("recording baseline tool settings are invalid".into());
        }
        self.baseline_tool_settings = baseline_tool_settings;
        self.tool_settings_events.clear();
        self.last_tool_settings_sample_ms = 0;
        let mut ended = false;
        let mut paused = false;
        for line in lines {
            let line = line.map_err(|error| error.to_string())?;
            if line.is_empty() {
                continue;
            }
            let record: Line = match serde_json::from_str(&line) {
                Ok(record) => record,
                Err(_) => {
                    // A process interruption may leave only the final line
                    // incomplete. Earlier malformed data is not silently used.
                    if ended {
                        return Err("data follows the recording end marker".into());
                    }
                    self.interrupted = true;
                    break;
                }
            };
            match record {
                Line::Event(event) if !ended => {
                    if paused {
                        return Err("recording contains an edit while paused".into());
                    }
                    if event.elapsed_ms < self.last_elapsed_ms {
                        return Err("recorded edit time moves backwards".into());
                    }
                    if self.events.len() >= MAX_EVENTS {
                        return Err("recording contains more than 200 events".into());
                    }
                    validate_document(&event.document)?;
                    if let Some(samples) = &event.stroke_samples {
                        if samples.start_elapsed_ms > event.elapsed_ms
                            || samples.points.len() < 2
                            || samples.points.len() > MAX_STROKE_SAMPLE_POINTS
                            || !samples.points.iter().all(Point::is_valid)
                            || !event.document.strokes.iter().any(|stroke| {
                                stroke.id == samples.stroke_id
                                    && samples_are_subsequence(&samples.points, &stroke.points)
                            })
                        {
                            return Err("recorded stroke samples are invalid".into());
                        }
                    }
                    let sample_bytes = event.stroke_samples.as_ref().map_or(0, |samples| {
                        samples.points.len().saturating_mul(64).saturating_add(256)
                    });
                    self.estimated_bytes = self
                        .estimated_bytes
                        .saturating_add(estimate_document(&event.document))
                        .saturating_add(sample_bytes);
                    if self.estimated_bytes > MAX_ESTIMATED_BYTES {
                        return Err("recording exceeds its in-memory replay limit".into());
                    }
                    self.events.push(event);
                    self.last_elapsed_ms = self
                        .last_elapsed_ms
                        .max(self.events.last().map_or(0, |event| event.elapsed_ms));
                    self.elapsed_duration_ms = self.elapsed_duration_ms.max(self.last_elapsed_ms);
                }
                Line::Camera { elapsed_ms, view } if !ended => {
                    if paused {
                        return Err("recording contains a camera change while paused".into());
                    }
                    if elapsed_ms < self.last_elapsed_ms {
                        return Err("recorded camera time moves backwards".into());
                    }
                    view.validate()?;
                    if self.camera_events.len() >= MAX_EVENTS {
                        return Err("recording contains more than 200 camera changes".into());
                    }
                    self.camera_events.push((elapsed_ms, view));
                    self.last_camera_sample_ms = elapsed_ms;
                    self.last_elapsed_ms = elapsed_ms;
                    self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
                }
                Line::ToolSettings {
                    elapsed_ms,
                    settings,
                } if !ended => {
                    if paused {
                        return Err("recording contains tool settings while paused".into());
                    }
                    if elapsed_ms < self.last_elapsed_ms {
                        return Err("recorded tool-settings time moves backwards".into());
                    }
                    if !settings.is_valid() {
                        return Err("recorded tool settings are invalid".into());
                    }
                    if self.tool_settings_events.len() >= MAX_EVENTS {
                        return Err("recording contains more than 200 tool-settings changes".into());
                    }
                    self.tool_settings_events.push((elapsed_ms, settings));
                    self.last_tool_settings_sample_ms = elapsed_ms;
                    self.last_elapsed_ms = elapsed_ms;
                    self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
                }
                Line::Pause { elapsed_ms } if !ended && !paused => {
                    if elapsed_ms < self.last_elapsed_ms {
                        return Err("recording pause time moves backwards".into());
                    }
                    paused = true;
                    self.last_elapsed_ms = elapsed_ms;
                    self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
                }
                Line::Resume { elapsed_ms } if !ended && paused => {
                    if elapsed_ms < self.last_elapsed_ms {
                        return Err("recording resume time moves backwards".into());
                    }
                    paused = false;
                    self.last_elapsed_ms = elapsed_ms;
                    self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
                }
                Line::End { elapsed_ms, .. } if !ended => {
                    ended = true;
                    if let Some(elapsed_ms) = elapsed_ms {
                        self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
                        self.last_elapsed_ms = self.last_elapsed_ms.max(elapsed_ms);
                    }
                }
                _ => return Err("recording has an invalid event order".into()),
            }
        }
        self.interrupted |= !ended;
        self.paused = false;
        Ok(())
    }

    pub fn start(
        &mut self,
        document: &Document,
        source_key: &str,
        camera: &Camera,
        tool_settings: &ToolSettingsSnapshot,
    ) -> Result<(), String> {
        self.stop();
        validate_document(document)?;
        let baseline_camera = CameraView::capture(camera)?;
        if !tool_settings.is_valid() {
            return Err("The current tool settings are invalid".into());
        }
        if estimate_document(document) > MAX_ESTIMATED_BYTES {
            return Err("The current document exceeds the 56 MiB replay limit".into());
        }
        let storage_dir = self
            .storage_dir
            .clone()
            .ok_or("Application data folder is unavailable")?;
        fs::create_dir_all(&storage_dir).map_err(|error| error.to_string())?;
        let stored_bytes = history_directory_bytes(&storage_dir);
        if stored_bytes.saturating_add(MAX_HISTORY_BYTES) > MAX_TOTAL_HISTORY_BYTES {
            return Err("Construction-history storage is full (256 MiB total); remove older recording files from the application data folder before starting another recording".into());
        }
        let started_at_ms = now_ms();
        let recording_id = format!("{}-{}", started_at_ms, std::process::id());
        let mut path = storage_dir.join(format!("{recording_id}.jsonl"));
        let mut suffix = 1_u32;
        while path.exists() {
            path = storage_dir.join(format!("{recording_id}-{suffix}.jsonl"));
            suffix = suffix.saturating_add(1);
        }
        let source_key = bounded_source_key(source_key);
        let header = Line::Header {
            version: FORMAT_VERSION,
            recording_id: recording_id.clone(),
            source_key: source_key.clone(),
            started_at_ms,
            baseline: document.clone(),
            baseline_camera: Some(baseline_camera),
            baseline_tool_settings: Some(tool_settings.clone()),
        };
        let mut temp =
            tempfile::NamedTempFile::new_in(&storage_dir).map_err(|error| error.to_string())?;
        write_json_line(&mut temp, &header)?;
        temp.as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        temp.persist_noclobber(&path)
            .map_err(|error| error.error.to_string())?;
        let file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|error| error.to_string())?;
        let (sender, receiver) = mpsc::sync_channel::<WriteCommand>(8);
        let worker_path = path.clone();
        let (error_sender, error_receiver) = mpsc::channel();
        thread::Builder::new()
            .name("construction-history".into())
            .spawn(move || {
                if let Err(error) = writer_loop(file, &worker_path, receiver) {
                    let _ = error_sender.send(error);
                }
            })
            .map_err(|error| format!("Cannot start history writer: {error}"))?;

        self.path = Some(path);
        self.recording_id = Some(recording_id);
        self.source_key = Some(source_key);
        self.started_at_ms = started_at_ms;
        self.last_elapsed_ms = 0;
        self.paused_total_ms = 0;
        self.pause_started_at_ms = None;
        self.elapsed_duration_ms = 0;
        self.baseline = Some(document.clone());
        self.baseline_camera = Some(baseline_camera);
        self.camera_events.clear();
        self.last_camera_sample_ms = 0;
        self.baseline_tool_settings = Some(tool_settings.clone());
        self.tool_settings_events.clear();
        self.last_tool_settings_sample_ms = 0;
        self.events.clear();
        self.estimated_bytes = estimate_document(document);
        self.overhead = RecordingOverhead::default();
        self.recording = true;
        self.paused = false;
        self.interrupted = false;
        self.writer = Some(sender);
        self.writer_errors = Some(error_receiver);
        self.notice = Some(
            "Construction recording started. Committed brush geometry, camera views, and tool settings are captured.".into(),
        );
        Ok(())
    }

    pub fn stop(&mut self) {
        if !self.recording {
            return;
        }
        let elapsed_ms = self.duration_ms();
        self.recording = false;
        if let Some(writer) = self.writer.take() {
            let _ = writer.send(WriteCommand::End(elapsed_ms));
        }
        self.paused = false;
        self.pause_started_at_ms = None;
        self.notice = Some("Construction recording stopped; recorded history is preserved.".into());
    }

    pub fn record(&mut self, document: &Document, kind: &str) {
        if !self.recording || self.paused {
            return;
        }
        let started = Instant::now();
        self.record_active(document, kind, None, 0);
        self.overhead.add(started.elapsed());
    }

    pub fn record_stroke(
        &mut self,
        document: &Document,
        kind: &str,
        stroke: &Stroke,
        gesture_duration_ms: u64,
    ) {
        if !self.recording || self.paused {
            return;
        }
        let started = Instant::now();
        self.record_active(document, kind, Some(stroke), gesture_duration_ms);
        self.overhead.add(started.elapsed());
    }

    fn record_active(
        &mut self,
        document: &Document,
        kind: &str,
        stroke: Option<&Stroke>,
        gesture_duration_ms: u64,
    ) {
        if self.events.len() >= MAX_EVENTS {
            self.fail("Recording stopped at the 200-event limit.");
            return;
        }
        let elapsed_ms = self
            .last_elapsed_ms
            .max(self.active_elapsed_ms())
            .max(self.last_elapsed_ms.saturating_add(1));
        let stroke_samples = stroke
            .filter(|stroke| gesture_duration_ms > 0 && stroke.points.len() >= 2)
            .filter(|stroke| document.strokes.iter().any(|item| item.id == stroke.id))
            .map(|stroke| StrokeReplaySamples {
                start_elapsed_ms: elapsed_ms.saturating_sub(gesture_duration_ms),
                stroke_id: stroke.id,
                points: bounded_stroke_points(&stroke.points),
            });
        let sample_estimate = stroke_samples.as_ref().map_or(0, |samples| {
            samples.points.len().saturating_mul(64).saturating_add(256)
        });
        let estimate = estimate_document(document).saturating_add(sample_estimate);
        if self.estimated_bytes.saturating_add(estimate) > MAX_ESTIMATED_BYTES {
            self.fail("Recording stopped at the 56 MiB in-memory history limit.");
            return;
        }
        let event = HistoryEvent {
            elapsed_ms,
            kind: bounded_name(kind),
            action: HistoryAction::from_label(kind),
            stroke_samples,
            document: document.clone(),
        };
        let Some(writer) = &self.writer else {
            self.fail("History writer is unavailable; recorded history was preserved.");
            return;
        };
        match writer.try_send(WriteCommand::Event(event.clone())) {
            Ok(()) => {
                self.last_elapsed_ms = event.elapsed_ms;
                self.elapsed_duration_ms = self.elapsed_duration_ms.max(event.elapsed_ms);
                self.estimated_bytes += estimate;
                self.events.push(event);
            }
            Err(TrySendError::Full(_)) => {
                self.fail("History writer could not keep up; recording stopped safely.");
            }
            Err(TrySendError::Disconnected(_)) => {
                self.fail("History writer stopped; recording stopped safely.");
            }
        }
    }

    pub fn record_camera(&mut self, camera: &Camera) {
        if !self.recording || self.paused {
            return;
        }
        let started = Instant::now();
        let Ok(view) = CameraView::capture(camera) else {
            return;
        };
        let previous = self
            .camera_events
            .last()
            .map(|(_, view)| *view)
            .or(self.baseline_camera);
        if previous.is_some_and(|previous| previous == view) {
            return;
        }
        let elapsed_ms = self.active_elapsed_ms().max(self.last_elapsed_ms);
        if elapsed_ms.saturating_sub(self.last_camera_sample_ms) < CAMERA_SAMPLE_INTERVAL_MS {
            return;
        }
        if self.camera_events.len() >= MAX_EVENTS {
            self.fail("Recording stopped at the 200-camera-change limit.");
            return;
        }
        let Some(writer) = &self.writer else {
            self.fail("History writer is unavailable; recorded history was preserved.");
            return;
        };
        match writer.try_send(WriteCommand::Camera { elapsed_ms, view }) {
            Ok(()) => {
                self.camera_events.push((elapsed_ms, view));
                self.last_camera_sample_ms = elapsed_ms;
                self.last_elapsed_ms = elapsed_ms;
                self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
            }
            Err(TrySendError::Full(_)) => {
                self.fail("History writer could not keep up; recording stopped safely.");
            }
            Err(TrySendError::Disconnected(_)) => {
                self.fail("History writer stopped; recording stopped safely.");
            }
        }
        self.overhead.add(started.elapsed());
    }

    pub fn record_tool_settings(&mut self, settings: &ToolSettingsSnapshot) {
        if !self.recording || self.paused || !settings.is_valid() {
            return;
        }
        let started = Instant::now();
        let previous = self
            .tool_settings_events
            .last()
            .map(|(_, settings)| settings)
            .or(self.baseline_tool_settings.as_ref());
        if previous.is_some_and(|previous| previous == settings) {
            return;
        }
        let elapsed_ms = self.active_elapsed_ms().max(self.last_elapsed_ms);
        if elapsed_ms.saturating_sub(self.last_tool_settings_sample_ms)
            < TOOL_SETTINGS_SAMPLE_INTERVAL_MS
        {
            return;
        }
        if self.tool_settings_events.len() >= MAX_EVENTS {
            self.fail("Recording stopped at the 200-tool-settings-change limit.");
            return;
        }
        let Some(writer) = &self.writer else {
            self.fail("History writer is unavailable; recorded history was preserved.");
            return;
        };
        match writer.try_send(WriteCommand::ToolSettings {
            elapsed_ms,
            settings: settings.clone(),
        }) {
            Ok(()) => {
                self.tool_settings_events
                    .push((elapsed_ms, settings.clone()));
                self.last_tool_settings_sample_ms = elapsed_ms;
                self.last_elapsed_ms = elapsed_ms;
                self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
            }
            Err(TrySendError::Full(_)) => {
                self.fail("History writer could not keep up; recording stopped safely.");
            }
            Err(TrySendError::Disconnected(_)) => {
                self.fail("History writer stopped; recording stopped safely.");
            }
        }
        self.overhead.add(started.elapsed());
    }

    pub fn overhead(&self) -> RecordingOverhead {
        self.overhead
    }

    fn fail(&mut self, message: &str) {
        self.recording = false;
        self.paused = false;
        self.pause_started_at_ms = None;
        self.writer = None;
        self.interrupted = true;
        self.notice = Some(message.into());
    }

    pub fn recording(&self) -> bool {
        self.recording
    }

    pub fn paused(&self) -> bool {
        self.recording && self.paused
    }

    pub fn pause(&mut self) {
        if !self.recording || self.paused {
            return;
        }
        let elapsed_ms = self.active_elapsed_ms().max(self.last_elapsed_ms);
        let Some(writer) = &self.writer else {
            self.fail("History writer is unavailable; recording stopped safely.");
            return;
        };
        if writer.send(WriteCommand::Pause(elapsed_ms)).is_err() {
            self.fail("History writer stopped; recording stopped safely.");
            return;
        }
        self.paused = true;
        self.pause_started_at_ms = Some(now_ms());
        self.last_elapsed_ms = elapsed_ms;
        self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
        self.notice = Some("Construction recording paused.".into());
    }

    pub fn resume(&mut self) {
        if !self.recording || !self.paused {
            return;
        }
        let now = now_ms();
        let paused_total_ms = self
            .paused_total_ms
            .saturating_add(now.saturating_sub(self.pause_started_at_ms.unwrap_or(now)));
        let elapsed_ms = now
            .saturating_sub(self.started_at_ms)
            .saturating_sub(paused_total_ms)
            .max(self.last_elapsed_ms);
        let Some(writer) = &self.writer else {
            self.fail("History writer is unavailable; recording stopped safely.");
            return;
        };
        if writer.send(WriteCommand::Resume(elapsed_ms)).is_err() {
            self.fail("History writer stopped; recording stopped safely.");
            return;
        }
        self.paused_total_ms = paused_total_ms;
        self.pause_started_at_ms = None;
        self.paused = false;
        self.last_elapsed_ms = elapsed_ms;
        self.elapsed_duration_ms = self.elapsed_duration_ms.max(elapsed_ms);
        self.notice = Some("Construction recording resumed.".into());
    }

    fn active_elapsed_ms(&self) -> u64 {
        let now = self.pause_started_at_ms.unwrap_or_else(now_ms);
        now.saturating_sub(self.started_at_ms)
            .saturating_sub(self.paused_total_ms)
    }

    pub fn duration_ms(&self) -> u64 {
        let current = if self.recording && !self.paused {
            self.active_elapsed_ms()
        } else {
            self.elapsed_duration_ms
        };
        self.elapsed_duration_ms
            .max(self.last_elapsed_ms)
            .max(current)
    }

    pub fn stop_if_source_changed(&mut self, source_key: &str) {
        if self.recording && self.source_key.as_deref() != Some(source_key) {
            self.stop();
            self.notice = Some("Construction recording stopped because the active drawing changed. Open and import actions are not recorded as edits.".into());
        }
    }

    /// Surface asynchronous disk and encoding failures and reload only the
    /// events that actually reached durable storage.
    pub fn poll_writer(&mut self) {
        let Some(receiver) = &self.writer_errors else {
            return;
        };
        let Ok(error) = receiver.try_recv() else {
            return;
        };
        self.recording = false;
        self.paused = false;
        self.writer = None;
        self.writer_errors = None;
        self.events.clear();
        self.camera_events.clear();
        self.tool_settings_events.clear();
        self.estimated_bytes = 0;
        self.baseline = None;
        self.baseline_camera = None;
        self.baseline_tool_settings = None;
        self.recording_id = None;
        self.source_key = None;
        self.started_at_ms = 0;
        self.last_elapsed_ms = 0;
        self.last_camera_sample_ms = 0;
        self.last_tool_settings_sample_ms = 0;
        self.paused_total_ms = 0;
        self.pause_started_at_ms = None;
        self.elapsed_duration_ms = 0;
        self.interrupted = true;
        if let Some(path) = self.path.clone() {
            if let Err(read_error) = self.read(&path) {
                self.notice = Some(format!("History write failed ({error}); saved events could not be recovered ({read_error})."));
                return;
            }
        }
        self.interrupted = true;
        self.notice = Some(format!("History recording stopped after a storage failure: {error}. Only durably saved edits are retained."));
    }

    pub fn has_history(&self) -> bool {
        self.baseline.is_some() && !self.events.is_empty()
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn camera_event_count(&self) -> usize {
        self.camera_events.len()
    }

    pub fn tool_settings_event_count(&self) -> usize {
        self.tool_settings_events.len()
    }

    pub fn interrupted(&self) -> bool {
        self.interrupted
    }

    /// Timeline snapshots used by replay. The baseline is at time zero and
    /// each committed state keeps the elapsed time captured by the recorder.
    pub fn timeline(&self) -> Vec<(u64, Document)> {
        let Some(baseline) = &self.baseline else {
            return Vec::new();
        };
        std::iter::once((0, baseline.clone()))
            .chain(
                self.events
                    .iter()
                    .map(|event| (event.elapsed_ms, event.document.clone())),
            )
            .collect()
    }

    pub fn camera_timeline(&self) -> Vec<(u64, CameraView)> {
        self.baseline_camera
            .iter()
            .map(|view| (0, *view))
            .chain(self.camera_events.iter().copied())
            .collect()
    }

    pub fn stroke_sample_timeline(&self) -> Vec<(u64, StrokeReplaySamples)> {
        self.events
            .iter()
            .filter_map(|event| {
                event
                    .stroke_samples
                    .as_ref()
                    .map(|samples| (event.elapsed_ms, samples.clone()))
            })
            .collect()
    }

    pub fn tool_settings_timeline(&self) -> Vec<(u64, ToolSettingsSnapshot)> {
        self.baseline_tool_settings
            .iter()
            .map(|settings| (0, settings.clone()))
            .chain(self.tool_settings_events.iter().cloned())
            .collect()
    }

    pub fn snapshot_only(&self) -> bool {
        self.baseline_camera.is_none()
    }

    pub fn source_key(&self) -> Option<&str> {
        self.source_key.as_deref()
    }
}

fn writer_loop(
    mut file: impl DurableWrite,
    _path: &Path,
    receiver: mpsc::Receiver<WriteCommand>,
) -> Result<(), String> {
    let mut bytes = file.len().unwrap_or(0);
    while let Ok(command) = receiver.recv() {
        let line = match command {
            WriteCommand::Event(event) => Line::Event(event),
            WriteCommand::Camera { elapsed_ms, view } => Line::Camera { elapsed_ms, view },
            WriteCommand::ToolSettings {
                elapsed_ms,
                settings,
            } => Line::ToolSettings {
                elapsed_ms,
                settings,
            },
            WriteCommand::Pause(elapsed_ms) => Line::Pause { elapsed_ms },
            WriteCommand::Resume(elapsed_ms) => Line::Resume { elapsed_ms },
            WriteCommand::End(ended_at_ms) => Line::End {
                ended_at_ms: now_ms(),
                elapsed_ms: Some(ended_at_ms),
            },
        };
        let encoded = match serde_json::to_vec(&line) {
            Ok(encoded) => encoded,
            Err(error) => return Err(format!("cannot encode event: {error}")),
        };
        if bytes.saturating_add(encoded.len() as u64 + 1) > MAX_HISTORY_BYTES {
            return Err("64 MiB recording storage limit reached".into());
        }
        file.write_all(&encoded)
            .map_err(|error| error.to_string())?;
        file.write_all(b"\n").map_err(|error| error.to_string())?;
        file.sync_data().map_err(|error| error.to_string())?;
        bytes += encoded.len() as u64 + 1;
        if matches!(line, Line::End { .. }) {
            return Ok(());
        }
    }
    Ok(())
}

trait DurableWrite: Write {
    fn len(&self) -> std::io::Result<u64>;
    fn sync_data(&self) -> std::io::Result<()>;
}

impl DurableWrite for File {
    fn len(&self) -> std::io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn sync_data(&self) -> std::io::Result<()> {
        File::sync_data(self)
    }
}

fn latest_recording(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let is_history = path
                .extension()
                .is_some_and(|extension| extension == "jsonl");
            if !is_history {
                return None;
            }
            let metadata = fs::symlink_metadata(&path).ok()?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                None
            } else {
                metadata.modified().ok().map(|modified| (modified, path))
            }
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

fn history_directory_bytes(directory: &Path) -> u64 {
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                return None;
            }
            let metadata = fs::symlink_metadata(path).ok()?;
            (!metadata.file_type().is_symlink() && metadata.is_file()).then_some(metadata.len())
        })
        .fold(0_u64, u64::saturating_add)
}

fn write_json_line(file: &mut impl Write, line: &Line) -> Result<(), String> {
    serde_json::to_writer(&mut *file, line).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())
}

fn estimate_document(document: &Document) -> usize {
    4096_usize
        .saturating_add(document.strokes.len().saturating_mul(256))
        .saturating_add(
            document
                .strokes
                .iter()
                .map(|stroke| stroke.points.len().saturating_mul(64))
                .sum::<usize>(),
        )
        .saturating_add(document.layers.len().saturating_mul(128))
        .saturating_add(document.image_planes.len().saturating_mul(512))
        .saturating_add(document.guide_resources.len().saturating_mul(1024))
        .saturating_add(document.model_references.len().saturating_mul(1024))
}

fn bounded_stroke_points(points: &[Point]) -> Vec<Point> {
    if points.len() <= MAX_STROKE_SAMPLE_POINTS {
        return points.to_vec();
    }
    let final_index = points.len() - 1;
    (0..MAX_STROKE_SAMPLE_POINTS)
        .map(|index| index * final_index / (MAX_STROKE_SAMPLE_POINTS - 1))
        .map(|index| points[index])
        .collect()
}

fn samples_are_subsequence(samples: &[Point], full: &[Point]) -> bool {
    let mut full_points = full.iter();
    for sample in samples {
        let Some(_) = full_points.find(|point| *point == sample) else {
            return false;
        };
    }
    true
}

fn validate_document(document: &Document) -> Result<(), String> {
    let encoded = serde_json::to_string(document).map_err(|error| error.to_string())?;
    Document::from_json(&encoded).map(|_| ())
}

fn bounded_name(name: &str) -> String {
    let mut bounded: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_EVENT_NAME_CHARS)
        .collect();
    if bounded.is_empty() {
        bounded.push_str("Edit");
    }
    bounded
}

fn bounded_source_key(source_key: &str) -> String {
    source_key.chars().take(4096).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u64::MAX as u128) as u64
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_history(storage_dir: &Path) -> ConstructionHistory {
        ConstructionHistory {
            storage_dir: Some(storage_dir.to_path_buf()),
            path: None,
            recording_id: None,
            source_key: None,
            started_at_ms: 0,
            last_elapsed_ms: 0,
            paused_total_ms: 0,
            pause_started_at_ms: None,
            elapsed_duration_ms: 0,
            baseline: None,
            baseline_camera: None,
            camera_events: Vec::new(),
            last_camera_sample_ms: 0,
            baseline_tool_settings: None,
            tool_settings_events: Vec::new(),
            last_tool_settings_sample_ms: 0,
            events: Vec::new(),
            estimated_bytes: 0,
            overhead: RecordingOverhead::default(),
            recording: false,
            paused: false,
            interrupted: false,
            writer: None,
            writer_errors: None,
            notice: None,
        }
    }

    fn default_tool_settings() -> ToolSettingsSnapshot {
        ToolSettingsSnapshot {
            tool: "brush".into(),
            radius: 0.025,
            color: [0.2, 0.4, 0.8],
            stabilization: 0.35,
            brush: Brush::default(),
        }
    }

    #[test]
    fn history_actions_are_typed_and_old_events_still_load() {
        let mut legacy = serde_json::json!({
            "elapsed_ms": 12,
            "kind": "Undo",
            "document": Document::default(),
        });
        let old_event: HistoryEvent = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(old_event.kind, "Undo");
        assert_eq!(old_event.action, HistoryAction::SceneEdit);

        legacy["action"] = serde_json::json!(HistoryAction::Undo);
        let event: HistoryEvent = serde_json::from_value(legacy).unwrap();
        assert_eq!(event.action, HistoryAction::Undo);
        assert_eq!(
            HistoryAction::from_label("Insert drawing"),
            HistoryAction::Import
        );
        assert_eq!(
            HistoryAction::from_label("Import OBJ reference"),
            HistoryAction::Import
        );
        assert_eq!(
            HistoryAction::from_label("Import image plane"),
            HistoryAction::Import
        );
        assert_eq!(
            HistoryAction::from_label("Draw on model"),
            HistoryAction::ModelDraw
        );
        assert_eq!(
            HistoryAction::from_label("Scene edit"),
            HistoryAction::SceneEdit
        );
    }

    #[test]
    fn synchronous_recording_overhead_reports_average_and_maximum() {
        let mut overhead = RecordingOverhead::default();
        overhead.add(std::time::Duration::from_micros(8));
        overhead.add(std::time::Duration::from_micros(12));
        assert_eq!(overhead.events, 2);
        assert_eq!(overhead.average_microseconds(), 10.0);
        assert_eq!(overhead.max_nanoseconds, 12_000);
        assert_eq!(RecordingOverhead::default().average_microseconds(), 0.0);
    }

    #[test]
    fn camera_views_are_sampled_bounded_and_survive_restart() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut recording = isolated_history(directory.path());
        let baseline = Document::default();
        let mut camera = Camera::default();
        recording
            .start(
                &baseline,
                "camera-drawing",
                &camera,
                &default_tool_settings(),
            )
            .unwrap();
        thread::sleep(std::time::Duration::from_millis(
            CAMERA_SAMPLE_INTERVAL_MS + 20,
        ));
        camera.yaw += 0.25;
        recording.record_camera(&camera);
        assert_eq!(recording.camera_events.len(), 1);
        camera.yaw += 0.25;
        recording.record_camera(&camera);
        assert_eq!(
            recording.camera_events.len(),
            1,
            "Changes inside the 100 ms sampling window are coalesced"
        );
        thread::sleep(std::time::Duration::from_millis(
            CAMERA_SAMPLE_INTERVAL_MS + 20,
        ));
        recording.record_camera(&camera);
        assert_eq!(recording.camera_events.len(), 2);
        let recording_path = recording.path.clone().unwrap();
        recording.stop();

        for _ in 0..100 {
            let finished = fs::read_to_string(&recording_path)
                .is_ok_and(|contents| contents.contains("\"record\":\"end\""));
            if finished {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }

        let mut restarted = isolated_history(directory.path());
        restarted.read(&recording_path).unwrap();
        assert!(!restarted.snapshot_only());
        assert_eq!(restarted.camera_timeline().len(), 3);
        assert_eq!(restarted.camera_timeline()[2].1.yaw, camera.yaw);
    }

    #[test]
    fn committed_stroke_geometry_samples_survive_restart() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut recording = isolated_history(directory.path());
        let baseline = Document::default();
        recording
            .start(
                &baseline,
                "stroke-drawing",
                &Camera::default(),
                &default_tool_settings(),
            )
            .unwrap();
        thread::sleep(std::time::Duration::from_millis(80));
        let stroke = Stroke {
            model_surface: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: baseline.active_layer,
            id: 2,
            color: [0.8, 0.2, 0.1],
            radius: 0.025,
            brush: Default::default(),
            points: vec![
                Point {
                    position: glam::Vec3::ZERO,
                    pressure: 0.4,
                    tilt: None,

                    time_ms: 0,
                },
                Point {
                    position: glam::Vec3::X,
                    pressure: 0.9,
                    tilt: Some(glam::Vec3::Y),

                    time_ms: 0,
                },
            ],
        };
        let mut document = baseline.clone();
        document.strokes.push(stroke.clone());
        recording.record_stroke(&document, "Draw", &stroke, 50);
        let recording_path = recording.path.clone().unwrap();
        recording.stop();

        for _ in 0..100 {
            let finished = fs::read_to_string(&recording_path)
                .is_ok_and(|contents| contents.contains("\"record\":\"end\""));
            if finished {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }

        let mut restarted = isolated_history(directory.path());
        restarted.read(&recording_path).unwrap();
        let samples = restarted.stroke_sample_timeline();
        assert_eq!(samples.len(), 1);
        assert!(samples[0].1.start_elapsed_ms < samples[0].0);
        assert_eq!(samples[0].1.stroke_id, stroke.id);
        assert_eq!(samples[0].1.points, stroke.points);
    }

    #[test]
    fn tool_settings_are_bounded_coalesced_and_survive_restart() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut recording = isolated_history(directory.path());
        let baseline = Document::default();
        let camera = Camera::default();
        let mut settings = default_tool_settings();
        recording
            .start(&baseline, "settings-drawing", &camera, &settings)
            .unwrap();
        thread::sleep(std::time::Duration::from_millis(
            TOOL_SETTINGS_SAMPLE_INTERVAL_MS + 20,
        ));
        settings.radius = 0.08;
        recording.record_tool_settings(&settings);
        assert_eq!(recording.tool_settings_events.len(), 1);

        settings.color = [0.9, 0.1, 0.3];
        recording.record_tool_settings(&settings);
        assert_eq!(recording.tool_settings_events.len(), 1);
        thread::sleep(std::time::Duration::from_millis(
            TOOL_SETTINGS_SAMPLE_INTERVAL_MS + 20,
        ));
        recording.record_tool_settings(&settings);
        assert_eq!(recording.tool_settings_events.len(), 2);
        let recording_path = recording.path.clone().unwrap();
        recording.stop();

        for _ in 0..100 {
            let finished = fs::read_to_string(&recording_path)
                .is_ok_and(|contents| contents.contains("\"record\":\"end\""));
            if finished {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }

        let mut restarted = isolated_history(directory.path());
        restarted.read(&recording_path).unwrap();
        assert_eq!(restarted.tool_settings_event_count(), 2);
        assert_eq!(
            restarted.baseline_tool_settings.as_ref().unwrap().radius,
            0.025
        );
        assert_eq!(restarted.tool_settings_events[0].1.radius, 0.08);
        assert_eq!(restarted.tool_settings_events[1].1.color, settings.color);
    }

    #[test]
    fn stroke_sample_point_budget_preserves_path_endpoints() {
        let points: Vec<_> = (0..MAX_STROKE_SAMPLE_POINTS + 100)
            .map(|index| Point {
                position: glam::Vec3::new(index as f32, 0.0, 0.0),
                pressure: 0.5,
                tilt: None,

                time_ms: 0,
            })
            .collect();
        let bounded = bounded_stroke_points(&points);
        assert_eq!(bounded.len(), MAX_STROKE_SAMPLE_POINTS);
        assert_eq!(bounded.first(), points.first());
        assert_eq!(bounded.last(), points.last());
    }

    #[test]
    fn legacy_snapshot_history_remains_readable_and_labeled() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let recording_path = directory.path().join("legacy-v2.jsonl");
        let baseline = Document::default();
        let header = serde_json::json!({
            "record": "header",
            "version": 2,
            "recording_id": "legacy-v2",
            "source_key": "legacy-drawing",
            "started_at_ms": 1,
            "baseline": baseline,
        });
        let event = serde_json::json!({
            "record": "event",
            "elapsed_ms": 10,
            "kind": "Draw",
            "action": "draw",
            "document": Document::default(),
        });
        fs::write(&recording_path, format!("{}\n{}\n", header, event)).unwrap();

        let mut restarted = isolated_history(directory.path());
        restarted.read(&recording_path).unwrap();
        assert!(restarted.snapshot_only());
        assert_eq!(restarted.event_count(), 1);
        assert!(restarted.camera_timeline().is_empty());
    }

    #[test]
    fn typed_events_and_baseline_survive_a_recording_restart() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut recording = isolated_history(directory.path());
        let baseline = Document::default();
        recording
            .start(
                &baseline,
                "test-drawing",
                &Camera::default(),
                &default_tool_settings(),
            )
            .unwrap();
        recording.pause();
        recording.record(&baseline, "Erase");
        assert_eq!(recording.event_count(), 0, "Paused edits are coalesced");
        recording.resume();
        recording.record(&baseline, "Draw");
        assert_eq!(recording.overhead().events, 1);
        assert!(recording.overhead().max_nanoseconds > 0);
        let recording_path = recording.path.clone().unwrap();
        recording.stop();

        for _ in 0..100 {
            let finished = fs::read_to_string(&recording_path)
                .is_ok_and(|contents| contents.contains("\"record\":\"end\""));
            if finished {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }

        let mut restarted = isolated_history(directory.path());
        restarted.read(&recording_path).unwrap();
        assert_eq!(restarted.source_key(), Some("test-drawing"));
        assert_eq!(restarted.event_count(), 1);
        assert_eq!(restarted.events[0].action, HistoryAction::Draw);
        assert_eq!(
            serde_json::to_value(&restarted.events[0].document).unwrap(),
            serde_json::to_value(&baseline).unwrap()
        );
        assert!(!restarted.interrupted());
    }

    #[test]
    fn pause_suppresses_v3_events_and_resume_records_them_again() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut recording = isolated_history(directory.path());
        let baseline = Document::default();
        let mut camera = Camera::default();
        let mut settings = default_tool_settings();
        recording
            .start(&baseline, "pause-v3", &camera, &settings)
            .unwrap();

        thread::sleep(std::time::Duration::from_millis(
            CAMERA_SAMPLE_INTERVAL_MS + 20,
        ));
        camera.yaw = 0.4;
        settings.radius = 0.08;
        recording.record_camera(&camera);
        recording.record_tool_settings(&settings);
        assert_eq!(recording.camera_event_count(), 1);
        assert_eq!(recording.tool_settings_event_count(), 1);

        recording.pause();
        thread::sleep(std::time::Duration::from_millis(30));
        camera.yaw = 0.8;
        settings.radius = 0.12;
        recording.record_camera(&camera);
        recording.record_tool_settings(&settings);
        recording.record(&baseline, "Draw");
        assert_eq!(recording.camera_event_count(), 1);
        assert_eq!(recording.tool_settings_event_count(), 1);
        assert_eq!(recording.event_count(), 0);

        recording.resume();
        thread::sleep(std::time::Duration::from_millis(
            CAMERA_SAMPLE_INTERVAL_MS + 20,
        ));
        recording.record_camera(&camera);
        recording.record_tool_settings(&settings);
        recording.record(&baseline, "Draw");
        assert_eq!(recording.camera_event_count(), 2);
        assert_eq!(recording.tool_settings_event_count(), 2);
        assert_eq!(recording.event_count(), 1);
        let recording_path = recording.path.clone().unwrap();
        recording.stop();

        for _ in 0..100 {
            if fs::read_to_string(&recording_path)
                .is_ok_and(|contents| contents.contains("\"record\":\"end\""))
            {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut restarted = isolated_history(directory.path());
        restarted.read(&recording_path).unwrap();
        assert_eq!(restarted.camera_timeline().len(), 3);
        assert_eq!(restarted.camera_timeline()[2].1.yaw, camera.yaw);
        assert_eq!(restarted.tool_settings_timeline().len(), 3);
        assert_eq!(
            restarted.tool_settings_timeline()[2].1.radius,
            settings.radius
        );
        assert_eq!(restarted.event_count(), 1);
        assert!(!restarted.interrupted());
    }

    #[test]
    fn history_storage_limits_reject_oversized_recordings_and_full_directories() {
        let read_directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let oversized = read_directory.path().join("oversized.jsonl");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&oversized)
            .unwrap()
            .set_len(MAX_HISTORY_BYTES + 1)
            .unwrap();
        let error = isolated_history(read_directory.path())
            .read(&oversized)
            .unwrap_err();
        assert!(error.contains("64 MiB storage limit"));

        let full_directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let retained = full_directory.path().join("retained.jsonl");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(retained)
            .unwrap()
            .set_len(MAX_TOTAL_HISTORY_BYTES - MAX_HISTORY_BYTES + 1)
            .unwrap();
        let mut history = isolated_history(full_directory.path());
        let error = history
            .start(
                &Document::default(),
                "full-storage",
                &Camera::default(),
                &default_tool_settings(),
            )
            .unwrap_err();
        assert!(error.contains("256 MiB total"));
        assert!(!history.recording());
    }

    #[test]
    fn interrupted_recording_retains_its_last_durable_event() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let mut recording = isolated_history(directory.path());
        let baseline = Document::default();
        recording
            .start(
                &baseline,
                "interrupted-drawing",
                &Camera::default(),
                &default_tool_settings(),
            )
            .unwrap();
        recording.record(&baseline, "Undo");
        let recording_path = recording.path.clone().unwrap();
        drop(recording);

        for _ in 0..100 {
            let durable = fs::read_to_string(&recording_path)
                .is_ok_and(|contents| contents.contains("\"record\":\"event\""));
            if durable {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }

        let mut restarted = isolated_history(directory.path());
        restarted.read(&recording_path).unwrap();
        assert!(restarted.interrupted());
        assert_eq!(restarted.event_count(), 1);
        assert_eq!(restarted.events[0].action, HistoryAction::Undo);
        assert!(!restarted.recording());
    }

    struct FailAfter {
        file: File,
        remaining: u64,
    }

    impl Write for FailAfter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Err(std::io::Error::other("simulated storage failure"));
            }
            let allowed = bytes.len().min(self.remaining as usize);
            let written = self.file.write(&bytes[..allowed])?;
            self.remaining = self.remaining.saturating_sub(written as u64);
            Ok(written)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }

    impl DurableWrite for FailAfter {
        fn len(&self) -> std::io::Result<u64> {
            self.file.metadata().map(|metadata| metadata.len())
        }

        fn sync_data(&self) -> std::io::Result<()> {
            self.file.sync_data()
        }
    }

    #[test]
    fn storage_failure_recovers_only_the_last_durable_edit() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let recording_path = directory.path().join("failure.jsonl");
        let baseline = Document::default();
        let header = Line::Header {
            version: FORMAT_VERSION,
            recording_id: "failure".into(),
            source_key: "test-drawing".into(),
            started_at_ms: now_ms(),
            baseline: baseline.clone(),
            baseline_camera: Some(CameraView::capture(&Camera::default()).unwrap()),
            baseline_tool_settings: Some(default_tool_settings()),
        };
        let mut seed = File::create(&recording_path).unwrap();
        write_json_line(&mut seed, &header).unwrap();
        seed.sync_all().unwrap();
        let header_bytes = seed.metadata().unwrap().len();
        drop(seed);

        let first = HistoryEvent {
            elapsed_ms: 10,
            kind: "Draw".into(),
            action: HistoryAction::Draw,
            stroke_samples: None,
            document: baseline.clone(),
        };
        let second = HistoryEvent {
            elapsed_ms: 20,
            kind: "Erase".into(),
            action: HistoryAction::Erase,
            stroke_samples: None,
            document: baseline,
        };
        let first_line = serde_json::to_vec(&Line::Event(first)).unwrap().len() as u64 + 1;
        let file = OpenOptions::new()
            .append(true)
            .open(&recording_path)
            .unwrap();
        let failing_writer = FailAfter {
            file,
            remaining: first_line,
        };
        let (commands, receiver) = mpsc::sync_channel(4);
        commands
            .send(WriteCommand::Event(HistoryEvent {
                elapsed_ms: 10,
                kind: "Draw".into(),
                action: HistoryAction::Draw,
                stroke_samples: None,
                document: Document::default(),
            }))
            .unwrap();
        commands.send(WriteCommand::Event(second)).unwrap();
        drop(commands);

        let (error_sender, error_receiver) = mpsc::channel();
        let path = recording_path.clone();
        let worker = thread::spawn(move || {
            let error = writer_loop(failing_writer, &path, receiver).unwrap_err();
            error_sender.send(error).unwrap();
        });

        let mut recording = isolated_history(directory.path());
        recording.path = Some(recording_path.clone());
        recording.read(&recording_path).unwrap();
        recording.recording = true;
        recording.writer_errors = Some(error_receiver);
        worker.join().unwrap();
        recording.poll_writer();

        assert!(!recording.recording());
        assert!(recording.interrupted());
        assert_eq!(recording.event_count(), 1);
        assert_eq!(recording.events[0].action, HistoryAction::Draw);
        assert!(recording.notice.as_deref().is_some_and(|notice| {
            notice.contains("storage failure") && notice.contains("durably saved edits")
        }));
        assert!(fs::metadata(recording_path).unwrap().len() > header_bytes);
    }
}
