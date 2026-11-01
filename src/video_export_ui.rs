use super::*;
use image::{codecs::gif::GifEncoder, Delay, Frame, RgbaImage};
use std::{
    collections::HashSet,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc,
    },
};

const MAX_FRAMES: usize = 480;
const MAX_PIXELS: u64 = 921_600;

fn trim_range(total_ms: u64, percentages: [f32; 2]) -> Option<(u64, u64)> {
    let [start, end] = percentages;
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end > 100.0 || end <= start {
        return None;
    }
    let scale = |percent: f32| (total_ms as f64 * f64::from(percent) / 100.0).round() as u64;
    let start_ms = scale(start).min(total_ms);
    let end_ms = scale(end).min(total_ms);
    (end_ms > start_ms).then_some((start_ms, end_ms))
}

fn recorded_document_at(timeline: &[(u64, Document)], elapsed_ms: u64) -> Option<&Document> {
    timeline
        .iter()
        .take_while(|(at, _)| *at <= elapsed_ms)
        .last()
        .map(|(_, document)| document)
}

fn recorded_camera_at(
    timeline: &[(u64, crate::camera_bookmarks::CameraView)],
    elapsed_ms: u64,
) -> Option<crate::camera_bookmarks::CameraView> {
    timeline
        .iter()
        .take_while(|(at, _)| *at <= elapsed_ms)
        .last()
        .map(|(_, view)| *view)
}

fn recorded_tool_settings_at(
    timeline: &[(u64, crate::construction_history::ToolSettingsSnapshot)],
    elapsed_ms: u64,
) -> Option<crate::construction_history::ToolSettingsSnapshot> {
    timeline
        .iter()
        .take_while(|(at, _)| *at <= elapsed_ms)
        .last()
        .map(|(_, settings)| settings.clone())
}

fn recorded_document_with_stroke_at(
    timeline: &[(u64, Document)],
    stroke_samples: &[(u64, crate::construction_history::StrokeReplaySamples)],
    elapsed_ms: u64,
) -> Option<Document> {
    let snapshot = recorded_document_at(timeline, elapsed_ms)?;
    for (end_ms, samples) in stroke_samples {
        if samples.start_elapsed_ms > elapsed_ms || elapsed_ms >= *end_ms {
            continue;
        }
        let Some((_, previous)) = timeline.iter().take_while(|(at, _)| *at < *end_ms).last() else {
            continue;
        };
        if previous
            .strokes
            .iter()
            .any(|stroke| stroke.id == samples.stroke_id)
        {
            continue;
        }
        let Some(final_stroke) = timeline
            .iter()
            .find(|(at, document)| {
                *at >= *end_ms
                    && document
                        .strokes
                        .iter()
                        .any(|stroke| stroke.id == samples.stroke_id)
            })
            .and_then(|(_, document)| {
                document
                    .strokes
                    .iter()
                    .find(|stroke| stroke.id == samples.stroke_id)
            })
        else {
            continue;
        };
        let duration = end_ms.saturating_sub(samples.start_elapsed_ms).max(1);
        let progress = elapsed_ms
            .saturating_sub(samples.start_elapsed_ms)
            .min(duration) as f64
            / duration as f64;
        let point_count = ((samples.points.len() as f64 * progress).ceil() as usize)
            .max(1)
            .min(samples.points.len());
        let mut stroke = final_stroke.clone();
        stroke.points = samples.points[..point_count].to_vec();
        let mut partial = previous.clone();
        partial.strokes.push(stroke);
        return Some(partial);
    }
    Some(snapshot.clone())
}

fn visible_reveal_strokes(document: &Document) -> Result<Vec<crate::document::Stroke>, String> {
    let strokes: Vec<_> = document
        .strokes
        .iter()
        .filter(|stroke| document.visible(stroke))
        .cloned()
        .collect();
    if strokes.is_empty() {
        return Err("The drawing has no visible strokes to reveal".into());
    }
    Ok(strokes)
}

fn reveal_target_count(stroke_count: usize, frame_index: usize, frame_count: usize) -> usize {
    if stroke_count == 0 || frame_count == 0 {
        return 0;
    }
    (stroke_count
        .saturating_mul(frame_index.saturating_add(1).min(frame_count))
        .saturating_add(frame_count - 1)
        / frame_count)
        .min(stroke_count)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportMode {
    Turntable,
    Sequence(u64),
    ReconstructedReveal,
    RecordedHistory,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VideoFormat {
    Gif,
    Mp4,
}

pub(super) struct VideoRequest {
    pub(super) path: PathBuf,
    size: [u32; 2],
    fps: u32,
    mode: ExportMode,
    duration: f32,
    history_total_ms: u64,
    history_trim_percent: [f32; 2],
    history_speed: f32,
    format: VideoFormat,
    replace: bool,
}

pub(super) struct ExportResourceAvailability {
    pub(super) image_plane_ids: HashSet<u64>,
    pub(super) model_reference_ids: HashSet<u64>,
}

fn validate_history_assets(
    timeline: &[(u64, Document)],
    available: &ExportResourceAvailability,
) -> Result<(), String> {
    let mut missing_images = HashSet::new();
    let mut missing_models = HashSet::new();
    for (_, document) in timeline {
        missing_images.extend(
            document
                .image_planes
                .iter()
                .filter(|plane| plane.visible && !available.image_plane_ids.contains(&plane.id))
                .map(|plane| plane.name.clone()),
        );
        missing_models.extend(
            document
                .model_references
                .iter()
                .filter(|model| model.visible && !available.model_reference_ids.contains(&model.id))
                .map(|model| model.name.clone()),
        );
    }
    if missing_images.is_empty() && missing_models.is_empty() {
        return Ok(());
    }
    let mut missing = Vec::new();
    if !missing_images.is_empty() {
        missing.push(format!(
            "image planes: {}",
            summarize_missing_names(missing_images)
        ));
    }
    if !missing_models.is_empty() {
        missing.push(format!(
            "OBJ references: {}",
            summarize_missing_names(missing_models)
        ));
    }
    Err(format!(
        "Recorded-history export needs unavailable visible assets ({}). Reopen the drawing or restore the missing resources, then retry.",
        missing.join("; ")
    ))
}

fn summarize_missing_names(names: HashSet<String>) -> String {
    let mut names: Vec<_> = names.into_iter().collect();
    names.sort_unstable();
    let omitted = names.len().saturating_sub(8);
    names.truncate(8);
    if omitted > 0 {
        names.push(format!("and {omitted} more"));
    }
    names.join(", ")
}

struct VideoJob {
    request: VideoRequest,
    document: Document,
    reveal_strokes: Option<Vec<crate::document::Stroke>>,
    history_timeline: Option<Vec<(u64, Document)>>,
    history_camera_timeline: Option<Vec<(u64, crate::camera_bookmarks::CameraView)>>,
    history_stroke_samples: Option<Vec<(u64, crate::construction_history::StrokeReplaySamples)>>,
    history_tool_settings: Option<Vec<(u64, crate::construction_history::ToolSettingsSnapshot)>>,
    active_history_tool_settings: Option<crate::construction_history::ToolSettingsSnapshot>,
    history_range_ms: Option<(u64, u64)>,
    revealed_stroke_count: usize,
    start_camera: Camera,
    bookmarks: crate::camera_bookmarks::CameraBookmarkStore,
    background: [f32; 3],
    lighting: crate::lighting::Lighting,
    frame_count: usize,
    next_frame: usize,
    pending_frame: Option<Vec<u8>>,
    frame_sender: Option<SyncSender<Option<Vec<u8>>>>,
    result_receiver: Receiver<Result<(), String>>,
    cancelled: Arc<AtomicBool>,
}

pub(super) struct VideoExportUi {
    dialog_open: bool,
    history_window_open: bool,
    mode: ExportMode,
    history_trim_percent: [f32; 2],
    history_speed: f32,
    width: u32,
    height: u32,
    fps: u32,
    duration: f32,
    format: VideoFormat,
    overwrite: Option<VideoRequest>,
    request: Option<VideoRequest>,
    job: Option<VideoJob>,
}

pub(super) struct HistoryRecordingContext<'a> {
    pub(super) source_key: &'a str,
    pub(super) camera: &'a crate::geometry::Camera,
    pub(super) tool_settings: &'a crate::construction_history::ToolSettingsSnapshot,
}

impl Default for VideoExportUi {
    fn default() -> Self {
        Self {
            dialog_open: false,
            history_window_open: false,
            mode: ExportMode::Turntable,
            history_trim_percent: [0.0, 100.0],
            history_speed: 1.0,
            width: 640,
            height: 360,
            fps: 12,
            duration: 5.0,
            format: VideoFormat::Gif,
            overwrite: None,
            request: None,
            job: None,
        }
    }
}

impl VideoExportUi {
    pub fn begin(&mut self) {
        if self.job.is_none() && self.request.is_none() {
            self.dialog_open = true;
        }
    }

    pub fn begin_history_panel(&mut self) {
        self.history_window_open = true;
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn export_duration_seconds(
        &self,
        sequences: &[crate::camera_bookmarks::CameraSequence],
        history_duration_ms: u64,
    ) -> Option<f32> {
        if self.mode == ExportMode::RecordedHistory {
            let (start_ms, end_ms) = trim_range(history_duration_ms, self.history_trim_percent)?;
            if !self.history_speed.is_finite() || self.history_speed <= 0.0 {
                return None;
            }
            let selected_ms = end_ms.saturating_sub(start_ms) as f32;
            return Some(selected_ms / 1000.0 / self.history_speed);
        }
        let seconds = match self.mode {
            ExportMode::Turntable | ExportMode::ReconstructedReveal => self.duration,
            ExportMode::RecordedHistory => unreachable!(),
            ExportMode::Sequence(id) => sequences
                .iter()
                .find(|sequence| sequence.id == id)?
                .duration_seconds(),
        };
        Some(seconds)
    }

    fn frame_count(
        &self,
        sequences: &[crate::camera_bookmarks::CameraSequence],
        history_duration_ms: u64,
    ) -> Option<usize> {
        let seconds = self.export_duration_seconds(sequences, history_duration_ms)?;
        let mut count = (seconds * self.fps as f32).ceil() as usize;
        if self.mode == ExportMode::RecordedHistory && seconds.is_finite() && seconds > 0.0 {
            count = count.max(2);
        }
        (seconds.is_finite() && seconds > 0.0 && (2..=MAX_FRAMES).contains(&count)).then_some(count)
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        sequences: &[crate::camera_bookmarks::CameraSequence],
        history: &mut crate::construction_history::ConstructionHistory,
        document: &Document,
        recording_context: HistoryRecordingContext<'_>,
    ) {
        history.poll_writer();
        history.stop_if_source_changed(recording_context.source_key);
        if self.history_window_open {
            let mut open = true;
            egui::Window::new("Construction history")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    if history.recording() {
                        if history.paused() {
                            ui.label("Recording paused. Edits during the pause are not captured individually; they appear in the next snapshot after resume.");
                        } else {
                        ui.label("Recording committed document states, camera views, and tool settings. Unfinished gestures are not recorded.");
                        }
                        ui.horizontal(|ui| {
                            if history.paused() {
                                if ui.button("Resume recording").clicked() {
                                    history.resume();
                                }
                            } else if ui.button("Pause recording").clicked() {
                                history.pause();
                            }
                            if ui.button("Stop recording").clicked() {
                                history.stop();
                            }
                        });
                    } else {
                        ui.label("Record future edits as ordered snapshots, committed brush-stroke geometry samples, camera views, and tool settings. Erases, undo/redo, and imports appear at their committed snapshots.");
                        if ui.button("Start new recording").clicked() {
                            if let Err(error) = history.start(
                                document,
                                recording_context.source_key,
                                recording_context.camera,
                                recording_context.tool_settings,
                            ) {
                                history.notice = Some(format!("Could not start history recording: {error}"));
                            }
                        }
                    }
                    if history.has_history() {
                        ui.label(format!("{} recorded edits · {} camera views · {} tool-setting changes · up to 200 of each / 64 MiB per recording · 256 MiB total", history.event_count(), history.camera_event_count(), history.tool_settings_event_count()));
                        let overhead = history.overhead();
                        if overhead.events > 0 {
                            ui.small(format!(
                                "Synchronous capture overhead: {:.1} µs average, {:.1} µs maximum across {} edits. JSONL encoding and disk sync run in the background.",
                                overhead.average_microseconds(),
                                overhead.max_nanoseconds as f64 / 1_000.0,
                                overhead.events,
                            ));
                        }
                        if let Some(source) = history.source_key() {
                            ui.small(format!("Recording source: {source}"));
                        }
                        if history.interrupted() {
                            ui.colored_label(ui_style::colors(ui.ctx()).warning, "The last session ended without a stop marker. Its completed edits are preserved for export; recording cannot resume after restart.");
                        }
                        ui.small("Brush and model strokes replay from committed, stabilized world-space geometry with their resolved release tails; these are not raw pen samples. No synthetic or reconstructed pen input is recorded. Other drawing edits appear at their committed snapshot time. Unfinished gestures, selection, and document changes that bypass the edit recorder are not captured. Camera views and tool settings are sampled at most 10 times per second. Older drawings have no recorded history; use Reconstructed reveal for those.");
                        if history.snapshot_only() {
                            ui.small("This is a legacy snapshot-only recording; it has no camera or tool-settings history.");
                        }
                    } else {
                        ui.label("No recorded edit history is available yet.");
                        ui.small("History storage is capped at 256 MiB total; older recordings are retained until you remove them from the application data folder.");
                    }
                    if let Some(notice) = &history.notice {
                        ui.small(notice);
                    }
                });
            self.history_window_open = open;
        }
        if self.dialog_open {
            let mut open = true;
            let mut choose = false;
            egui::Window::new("Animated export")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.mode, ExportMode::Turntable, "Turntable");
                        ui.selectable_value(
                            &mut self.mode,
                            ExportMode::ReconstructedReveal,
                            "Reconstructed reveal",
                        );
                        if history.has_history() {
                            ui.selectable_value(
                                &mut self.mode,
                                ExportMode::RecordedHistory,
                                "Recorded history",
                            );
                        }
                        for sequence in sequences {
                            ui.selectable_value(
                                &mut self.mode,
                                ExportMode::Sequence(sequence.id),
                                &sequence.name,
                            );
                        }
                    });
                    if !matches!(self.mode, ExportMode::Sequence(_))
                        && self.mode != ExportMode::RecordedHistory
                    {
                        ui.add(
                            egui::Slider::new(&mut self.duration, 1.0..=20.0)
                                .text("Duration (seconds)"),
                        );
                    }
                    if self.mode == ExportMode::ReconstructedReveal {
                        ui.small("Reconstructed from the current visible stroke order. This is not recorded history and has no historical timing.");
                    } else if self.mode == ExportMode::RecordedHistory {
                        let total_ms = history.duration_ms();
                        let max_start = (self.history_trim_percent[1] - 1.0).max(0.0);
                        ui.add(
                            egui::Slider::new(&mut self.history_trim_percent[0], 0.0..=max_start)
                                .text("Trim start (%)"),
                        );
                        let min_end = (self.history_trim_percent[0] + 1.0).min(100.0);
                        ui.add(
                            egui::Slider::new(&mut self.history_trim_percent[1], min_end..=100.0)
                                .text("Trim end (%)"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.history_speed, 0.25..=4.0)
                                .text("Playback speed (×)"),
                        );
                        let [trim_start, trim_end] = self.history_trim_percent;
                        let start_seconds = total_ms as f32 * trim_start / 100_000.0;
                        let end_seconds = total_ms as f32 * trim_end / 100_000.0;
                        let output_seconds = (end_seconds - start_seconds) / self.history_speed;
                        ui.label(format!(
                            "Recorded range {start_seconds:.2}–{end_seconds:.2}s · output {output_seconds:.2}s"
                        ));
                        ui.small("Snapshots are replayed at recorded times. Speed changes the export duration; trim percentages select a portion of the session. A very short selection exports at least two frames.");
                    }
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.width)
                                .range(160..=1280)
                                .prefix("Width "),
                        );
                        ui.label("×");
                        ui.add(
                            egui::DragValue::new(&mut self.height)
                                .range(90..=720)
                                .prefix("Height "),
                        );
                    });
                    ui.add(egui::Slider::new(&mut self.fps, 4..=24).text("Frames per second"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.format, VideoFormat::Gif, "GIF");
                        ui.selectable_value(&mut self.format, VideoFormat::Mp4, "MP4 (FFmpeg)");
                    });
                    if self.format == VideoFormat::Mp4 {
                        ui.small("MP4 encoding requires FFmpeg with libx264 on PATH. Odd dimensions are padded by one pixel for H.264.");
                    }
                    ui.label(format!("{} pixels", self.width as u64 * self.height as u64));
                    if let Some(count) = self.frame_count(sequences, history.duration_ms()) {
                        ui.label(format!(
                            "{count} frames · {} · overlays excluded",
                            match self.format { VideoFormat::Gif => "GIF animation", VideoFormat::Mp4 => "MP4 video" }
                        ));
                    } else {
                        ui.colored_label(
                            ui_style::colors(ui.ctx()).error,
                            if self.mode == ExportMode::RecordedHistory {
                                "Increase playback speed or shorten the trim range to fit the 480-frame limit."
                            } else {
                                "Choose a sequence or reduce the duration to fit the 480-frame limit."
                            },
                        );
                    }
                    if u64::from(self.width) * u64::from(self.height) > MAX_PIXELS {
                        ui.colored_label(
                            ui_style::colors(ui.ctx()).error,
                            "Output is limited to 921,600 pixels per frame.",
                        );
                    }
                    ui.horizontal(|ui| {
                        choose = ui
                            .add_enabled(
                            self.frame_count(sequences, history.duration_ms()).is_some()
                                && (self.mode != ExportMode::RecordedHistory
                                    || self.history_trim_percent[1]
                                        > self.history_trim_percent[0])
                                    && u64::from(self.width) * u64::from(self.height) <= MAX_PIXELS,
                                egui::Button::new(match self.format {
                                    VideoFormat::Gif => "Choose GIF file…",
                                    VideoFormat::Mp4 => "Choose MP4 file…",
                                }),
                            )
                            .clicked();
                        if ui.button("Cancel").clicked() {
                            self.dialog_open = false;
                        }
                    });
                });
            self.dialog_open &= open;
            if choose {
                let (extension, filter, filename) = match self.format {
                    VideoFormat::Gif => ("gif", "Animated GIF", "animation.gif"),
                    VideoFormat::Mp4 => ("mp4", "MP4 video", "animation.mp4"),
                };
                if let Some(mut path) = rfd::FileDialog::new()
                    .add_filter(filter, &[extension])
                    .set_file_name(filename)
                    .save_file()
                {
                    path.set_extension(extension);
                    let history_total_ms = history.duration_ms();
                    let request = VideoRequest {
                        path: path.clone(),
                        size: [self.width, self.height],
                        fps: self.fps,
                        mode: self.mode,
                        duration: self
                            .export_duration_seconds(sequences, history_total_ms)
                            .unwrap_or(self.duration),
                        history_total_ms,
                        history_trim_percent: self.history_trim_percent,
                        history_speed: self.history_speed,
                        format: self.format,
                        replace: false,
                    };
                    self.dialog_open = false;
                    if path.exists() {
                        self.overwrite = Some(request);
                    } else {
                        self.request = Some(request);
                    }
                }
            }
        }
        if let Some(request) = self.overwrite.as_ref() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("video-export-overwrite")).show(ctx, |ui| {
                ui.heading("Replace existing video?");
                ui.label(
                    request
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("video file"),
                );
                ui.horizontal(|ui| {
                    confirm = ui.button("Overwrite").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if confirm {
                let mut request = self.overwrite.take().unwrap();
                request.replace = true;
                self.request = Some(request);
            } else if cancel {
                self.overwrite = None;
            }
        }
        if let Some(job) = self.job.as_mut() {
            let mut cancel = false;
            egui::Window::new(match job.request.format {
                VideoFormat::Gif => "Rendering animated GIF",
                VideoFormat::Mp4 => "Rendering MP4 video",
            })
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add(
                    egui::ProgressBar::new(job.next_frame as f32 / job.frame_count as f32)
                        .text(format!("{} / {} frames", job.next_frame, job.frame_count)),
                );
                if let Some(settings) = &job.active_history_tool_settings {
                    ui.small(format!(
                        "Recorded tool: {} · radius {:.3} wu · stabilization {:.0}%",
                        settings.tool,
                        settings.radius,
                        settings.stabilization * 100.0,
                    ));
                }
                cancel = ui.button("Cancel export").clicked();
            });
            if cancel {
                job.cancelled.store(true, Ordering::Relaxed);
                job.frame_sender.take();
            }
        }
    }

    pub fn take_request(&mut self) -> Option<VideoRequest> {
        self.request.take()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        request: VideoRequest,
        document: &Document,
        camera: Camera,
        bookmarks: &crate::camera_bookmarks::CameraBookmarkStore,
        background: [f32; 3],
        lighting: crate::lighting::Lighting,
        history: &crate::construction_history::ConstructionHistory,
        available_resources: ExportResourceAvailability,
    ) -> Result<(), String> {
        if self.job.is_some() {
            return Err("An animated export is already running".into());
        }
        if request.path.exists() && !request.replace {
            return Err(
                "Destination appeared after overwrite confirmation; choose it again to confirm."
                    .into(),
            );
        }
        let seconds = match request.mode {
            ExportMode::Turntable
            | ExportMode::ReconstructedReveal
            | ExportMode::RecordedHistory => request.duration,
            ExportMode::Sequence(id) => {
                let duration = bookmarks
                    .sequences
                    .iter()
                    .find(|sequence| sequence.id == id)
                    .ok_or("Camera sequence no longer exists")?
                    .duration_seconds();
                bookmarks.sample_sequence(id, 0.0)?;
                duration
            }
        };
        let mut frame_count = (seconds * request.fps as f32).ceil() as usize;
        if request.mode == ExportMode::RecordedHistory && seconds.is_finite() && seconds > 0.0 {
            frame_count = frame_count.max(2);
        }
        if request.size.contains(&0)
            || u64::from(request.size[0]) * u64::from(request.size[1]) > MAX_PIXELS
            || !(4..=24).contains(&request.fps)
            || !(2..=MAX_FRAMES).contains(&frame_count)
        {
            return Err("Animated export exceeds its frame, rate, or dimension limits".into());
        }
        let history_timeline =
            (request.mode == ExportMode::RecordedHistory).then(|| history.timeline());
        let history_camera_timeline =
            (request.mode == ExportMode::RecordedHistory).then(|| history.camera_timeline());
        let history_stroke_samples =
            (request.mode == ExportMode::RecordedHistory).then(|| history.stroke_sample_timeline());
        let history_tool_settings =
            (request.mode == ExportMode::RecordedHistory).then(|| history.tool_settings_timeline());
        if history_timeline
            .as_ref()
            .is_some_and(|timeline| timeline.len() < 2)
        {
            return Err("A recorded history with at least one committed edit is required".into());
        }
        if let Some(timeline) = &history_timeline {
            validate_history_assets(timeline, &available_resources)?;
        }
        let history_range_ms = if request.mode == ExportMode::RecordedHistory {
            if !request.history_speed.is_finite() || request.history_speed <= 0.0 {
                return Err("Choose a valid recorded-history trim range and playback speed".into());
            }
            let Some(range) = trim_range(request.history_total_ms, request.history_trim_percent)
            else {
                return Err("The selected trim range is shorter than one millisecond".into());
            };
            Some(range)
        } else {
            None
        };
        let mut document = document.clone();
        let reveal_strokes = if request.mode == ExportMode::ReconstructedReveal {
            let visible = visible_reveal_strokes(&document)?;
            document.strokes.clear();
            Some(visible)
        } else {
            None
        };
        let parent = request
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let (frame_sender, frame_receiver) = mpsc::sync_channel::<Option<Vec<u8>>>(1);
        let (result_sender, result_receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let output_path = request.path.clone();
        let size = request.size;
        let fps = request.fps;
        let format = request.format;
        std::thread::Builder::new()
            .name(
                match format {
                    VideoFormat::Gif => "gif-export",
                    VideoFormat::Mp4 => "mp4-export",
                }
                .into(),
            )
            .spawn(move || {
                let result = match format {
                    VideoFormat::Gif => encode_gif(
                        frame_receiver,
                        &output_path,
                        size,
                        fps,
                        frame_count,
                        &worker_cancelled,
                    ),
                    VideoFormat::Mp4 => encode_mp4(
                        frame_receiver,
                        &output_path,
                        size,
                        fps,
                        frame_count,
                        &worker_cancelled,
                    ),
                };
                let _ = result_sender.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.job = Some(VideoJob {
            request,
            document,
            reveal_strokes,
            history_timeline,
            history_camera_timeline,
            history_stroke_samples,
            history_tool_settings,
            active_history_tool_settings: None,
            history_range_ms,
            revealed_stroke_count: 0,
            start_camera: camera,
            bookmarks: bookmarks.clone(),
            background,
            lighting,
            frame_count,
            next_frame: 0,
            pending_frame: None,
            frame_sender: Some(frame_sender),
            result_receiver,
            cancelled,
        });
        Ok(())
    }

    pub fn render_next(
        &mut self,
        renderer: &mut SceneRenderer,
        ctx: &egui::Context,
    ) -> Option<String> {
        let job = self.job.as_mut()?;
        if job.pending_frame.is_none() && job.next_frame < job.frame_count {
            let mut camera = job.start_camera;
            let mut valid_camera = true;
            match job.request.mode {
                ExportMode::Turntable => {
                    camera.yaw = job.start_camera.yaw
                        + std::f32::consts::TAU * job.next_frame as f32 / job.frame_count as f32;
                }
                ExportMode::Sequence(id) => {
                    let seconds = job
                        .bookmarks
                        .sequences
                        .iter()
                        .find(|sequence| sequence.id == id)
                        .map(|sequence| sequence.duration_seconds())
                        .unwrap_or(0.0);
                    let elapsed = (job.next_frame as f32 / job.request.fps as f32).min(seconds);
                    match job
                        .bookmarks
                        .sample_sequence(id, elapsed)
                        .and_then(|view| view.apply(&mut camera))
                    {
                        Ok(()) => {}
                        Err(error) => {
                            job.cancelled.store(true, Ordering::Relaxed);
                            job.frame_sender.take();
                            let _ = error;
                            valid_camera = false;
                        }
                    }
                }
                ExportMode::ReconstructedReveal => {}
                ExportMode::RecordedHistory => {
                    if let (Some(timeline), Some((start_ms, end_ms))) =
                        (&job.history_timeline, job.history_range_ms)
                    {
                        let elapsed_offset = end_ms
                            .saturating_sub(start_ms)
                            .saturating_mul(job.next_frame as u64)
                            / job.frame_count.saturating_sub(1).max(1) as u64;
                        let elapsed = start_ms.saturating_add(elapsed_offset);
                        if let Some(document) = recorded_document_with_stroke_at(
                            timeline,
                            job.history_stroke_samples.as_deref().unwrap_or_default(),
                            elapsed,
                        ) {
                            job.document = document;
                        }
                        if let Some(camera_timeline) = &job.history_camera_timeline {
                            if let Some(view) = recorded_camera_at(camera_timeline, elapsed) {
                                if view.apply(&mut camera).is_err() {
                                    valid_camera = false;
                                }
                            }
                        }
                        if let Some(settings_timeline) = &job.history_tool_settings {
                            job.active_history_tool_settings =
                                recorded_tool_settings_at(settings_timeline, elapsed);
                        }
                    } else {
                        valid_camera = false;
                    }
                }
            }
            if job.request.mode == ExportMode::ReconstructedReveal {
                if let Some(strokes) = &job.reveal_strokes {
                    let target_count =
                        reveal_target_count(strokes.len(), job.next_frame, job.frame_count);
                    job.document.strokes.extend(
                        strokes[job.revealed_stroke_count..target_count]
                            .iter()
                            .cloned(),
                    );
                    job.revealed_stroke_count = target_count;
                }
            }
            if valid_camera {
                match renderer.render_png_pixels(
                    &job.document,
                    &camera,
                    job.request.size,
                    job.background,
                    job.lighting,
                ) {
                    Ok(pixels) => job.pending_frame = Some(pixels),
                    Err(error) => {
                        job.cancelled.store(true, Ordering::Relaxed);
                        job.frame_sender.take();
                        let _ = error;
                    }
                }
            }
        }
        if let (Some(sender), Some(frame)) = (job.frame_sender.as_ref(), job.pending_frame.take()) {
            match sender.try_send(Some(frame)) {
                Ok(()) => job.next_frame += 1,
                Err(TrySendError::Full(Some(frame))) => job.pending_frame = Some(frame),
                Err(TrySendError::Disconnected(_)) => {
                    job.frame_sender.take();
                    return Some("Video encoder stopped unexpectedly.".into());
                }
                Err(TrySendError::Full(None)) => unreachable!(),
            }
        }
        if job.next_frame == job.frame_count && job.pending_frame.is_none() {
            job.frame_sender.take();
        }
        match job.result_receiver.try_recv() {
            Ok(Ok(())) => {
                let path = job.request.path.display().to_string();
                self.job = None;
                Some(format!("Exported video to {path}"))
            }
            Ok(Err(error)) => {
                self.job = None;
                Some(if error == "Export cancelled" {
                    "Video export cancelled.".into()
                } else {
                    format!("Video export failed: {error}")
                })
            }
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                Some("Video export worker stopped unexpectedly.".into())
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(16));
                None
            }
        }
    }
}

fn encode_gif(
    receiver: Receiver<Option<Vec<u8>>>,
    destination: &std::path::Path,
    size: [u32; 2],
    fps: u32,
    frame_count: usize,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    {
        let mut encoder = GifEncoder::new(&mut temporary);
        encoder
            .set_repeat(image::codecs::gif::Repeat::Infinite)
            .map_err(|error| error.to_string())?;
        for _ in 0..frame_count {
            if cancelled.load(Ordering::Relaxed) {
                return Err("Export cancelled".into());
            }
            let pixels = receiver
                .recv()
                .map_err(|_| "Export cancelled".to_owned())?
                .ok_or_else(|| "Export cancelled".to_owned())?;
            if pixels.len() != size[0] as usize * size[1] as usize * 4 {
                return Err("Rendered frame has invalid dimensions".into());
            }
            let image = RgbaImage::from_raw(size[0], size[1], pixels)
                .ok_or_else(|| "Could not assemble rendered frame".to_owned())?;
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(1000, fps),
                ))
                .map_err(|error| error.to_string())?;
        }
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err("Export cancelled".into());
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(destination)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}

/// Stream raw RGBA frames to FFmpeg so encoding stays off the UI/drawing path.
/// The temporary file uses an .mp4 suffix because FFmpeg selects its muxer from
/// the output filename. The final destination is installed only after a clean
/// encoder exit, so cancellation and encoder errors leave no partial movie.
fn encode_mp4(
    receiver: Receiver<Option<Vec<u8>>>,
    destination: &std::path::Path,
    size: [u32; 2],
    fps: u32,
    frame_count: usize,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let temporary = tempfile::Builder::new()
        .suffix(".mp4")
        .tempfile_in(parent)
        .map_err(|error| error.to_string())?;
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
        ])
        .arg(format!("{}x{}", size[0], size[1]))
        .args(["-framerate", &fps.to_string(), "-i", "pipe:0", "-an", "-vf"])
        .arg("pad=ceil(iw/2)*2:ceil(ih/2)*2")
        .args([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(temporary.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "MP4 export requires FFmpeg with libx264 installed and available on PATH".to_owned()
            } else {
                format!("Could not start FFmpeg: {error}")
            }
        })?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("FFmpeg did not open its input stream".into());
    };
    let expected_len = size[0] as usize * size[1] as usize * 4;
    for _ in 0..frame_count {
        if cancelled.load(Ordering::Relaxed) {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            return Err("Export cancelled".into());
        }
        let pixels = match receiver.recv() {
            Ok(Some(pixels)) => pixels,
            Ok(None) | Err(_) => {
                drop(stdin);
                let _ = child.kill();
                let _ = child.wait();
                return Err("Export cancelled".into());
            }
        };
        if pixels.len() != expected_len {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            return Err("Rendered frame has invalid dimensions".into());
        }
        if let Err(error) = stdin.write_all(&pixels) {
            drop(stdin);
            let status = child.wait().map_err(|wait_error| wait_error.to_string())?;
            return Err(if status.success() {
                format!("Could not stream a rendered frame: {error}")
            } else {
                "FFmpeg could not encode MP4 with the libx264 encoder".into()
            });
        }
    }
    drop(stdin);
    let status = child.wait().map_err(|error| error.to_string())?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("Export cancelled".into());
    }
    if !status.success() {
        return Err(
            "FFmpeg failed to encode MP4; confirm it was built with libx264 support".into(),
        );
    }
    if temporary
        .as_file()
        .metadata()
        .map_err(|error| error.to_string())?
        .len()
        == 0
    {
        return Err("FFmpeg produced an empty MP4 file".into());
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(destination)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_history_samples_the_latest_state_at_each_timeline_time() {
        let baseline = Document::default();
        let mut first_edit = baseline.clone();
        first_edit.active_layer = 2;
        let mut second_edit = baseline.clone();
        second_edit.active_layer = 3;
        let timeline = vec![(0, baseline), (120, first_edit), (300, second_edit)];

        assert_eq!(recorded_document_at(&timeline, 0).unwrap().active_layer, 1);
        assert_eq!(
            recorded_document_at(&timeline, 119).unwrap().active_layer,
            1
        );
        assert_eq!(
            recorded_document_at(&timeline, 120).unwrap().active_layer,
            2
        );
        assert_eq!(
            recorded_document_at(&timeline, 299).unwrap().active_layer,
            2
        );
        assert_eq!(
            recorded_document_at(&timeline, 300).unwrap().active_layer,
            3
        );
        assert_eq!(
            recorded_document_at(&timeline, 900).unwrap().active_layer,
            3
        );
    }

    #[test]
    fn recorded_camera_samples_the_latest_view_and_keeps_legacy_fallback() {
        let camera = crate::geometry::Camera::default();
        let baseline = crate::camera_bookmarks::CameraView::capture(&camera).unwrap();
        let changed = crate::camera_bookmarks::CameraView {
            yaw: baseline.yaw + 0.5,
            ..baseline
        };
        let timeline = vec![(0, baseline), (120, changed)];
        assert_eq!(recorded_camera_at(&timeline, 0), Some(baseline));
        assert_eq!(recorded_camera_at(&timeline, 119), Some(baseline));
        assert_eq!(recorded_camera_at(&timeline, 120), Some(changed));
        assert_eq!(recorded_camera_at(&timeline, 900), Some(changed));
        assert_eq!(recorded_camera_at(&[], 10), None);
    }

    #[test]
    fn recorded_stroke_geometry_replays_progressively_without_mutating_snapshots() {
        let baseline = Document::default();
        let points = vec![
            crate::document::Point {
                position: glam::Vec3::ZERO,
                pressure: 0.3,
                tilt: None,
            },
            crate::document::Point {
                position: glam::Vec3::X,
                pressure: 0.6,
                tilt: None,
            },
            crate::document::Point {
                position: glam::Vec3::Y,
                pressure: 1.0,
                tilt: None,
            },
        ];
        let stroke = crate::document::Stroke {
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: baseline.active_layer,
            id: 2,
            color: [0.1, 0.6, 0.9],
            radius: 0.025,
            brush: Default::default(),
            points: points.clone(),
        };
        let mut final_document = baseline.clone();
        final_document.strokes.push(stroke);
        let timeline = vec![(0, baseline), (1000, final_document)];
        let samples = vec![(
            1000,
            crate::construction_history::StrokeReplaySamples {
                start_elapsed_ms: 0,
                stroke_id: 2,
                points: points.clone(),
            },
        )];

        let partial = recorded_document_with_stroke_at(&timeline, &samples, 500).unwrap();
        assert_eq!(partial.strokes.len(), 1);
        assert_eq!(partial.strokes[0].points, points[..2]);
        assert!(timeline[0].1.strokes.is_empty());
        assert_eq!(timeline[1].1.strokes[0].points, points);
        assert_eq!(
            recorded_document_with_stroke_at(&timeline, &samples, 1000)
                .unwrap()
                .strokes[0]
                .points,
            points
        );
    }

    #[test]
    fn recorded_history_trim_rejects_invalid_ranges_and_preserves_bounds() {
        assert_eq!(trim_range(10_000, [25.0, 75.0]), Some((2_500, 7_500)));
        assert_eq!(trim_range(1_000, [0.0, 100.0]), Some((0, 1_000)));
        assert_eq!(trim_range(1_000, [50.0, 50.0]), None);
        assert_eq!(trim_range(1_000, [f32::NAN, 100.0]), None);
        assert_eq!(trim_range(1_000, [-1.0, 50.0]), None);
        assert_eq!(trim_range(1_000, [20.0, 101.0]), None);
        assert_eq!(trim_range(0, [0.0, 100.0]), None);
    }

    #[test]
    fn recorded_history_speed_scales_the_trimmed_export_duration() {
        let mut ui = VideoExportUi {
            mode: ExportMode::RecordedHistory,
            history_trim_percent: [25.0, 75.0],
            history_speed: 2.0,
            fps: 12,
            ..Default::default()
        };
        assert_eq!(ui.export_duration_seconds(&[], 10_000), Some(2.5));
        assert_eq!(ui.frame_count(&[], 10_000), Some(30));
        ui.history_speed = f32::NAN;
        assert_eq!(ui.export_duration_seconds(&[], 10_000), None);
    }

    #[test]
    fn recorded_history_export_rejects_missing_visible_assets_and_ignores_hidden_ones() {
        let mut document = Document::default();
        let mut image = crate::image_planes::ImagePlane::from_asset(
            1,
            "Visible reference",
            "image-1.png".into(),
            [2, 2],
            glam::Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
        )
        .unwrap();
        let mut hidden_image = image.clone();
        hidden_image.id = 2;
        hidden_image.name = "Hidden reference".into();
        hidden_image.asset_file = "image-2.png".into();
        hidden_image.visible = false;
        image.visible = true;
        document.image_planes.extend([image, hidden_image]);
        document
            .model_references
            .push(crate::document::model_references::ObjReferenceResource {
                id: 3,
                name: "Visible model".into(),
                visible: true,
                locked: false,
                center: glam::Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                scale: 1.0,
                asset: crate::obj_reference_storage::ObjReferenceAsset {
                    id: 3,
                    bundle_dir: "models/3".into(),
                },
            });
        let timeline = vec![(0, document)];
        let unavailable = ExportResourceAvailability {
            image_plane_ids: HashSet::new(),
            model_reference_ids: HashSet::new(),
        };
        let error = validate_history_assets(&timeline, &unavailable).unwrap_err();
        assert!(error.contains("Visible reference"));
        assert!(error.contains("Visible model"));
        assert!(!error.contains("Hidden reference"));

        let available = ExportResourceAvailability {
            image_plane_ids: HashSet::from([1]),
            model_reference_ids: HashSet::from([3]),
        };
        validate_history_assets(&timeline, &available).unwrap();
    }

    #[test]
    fn gif_encoder_writes_decodable_frames_and_preserves_destination_on_cancel() {
        use image::AnimationDecoder;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("preview.gif");
        let (sender, receiver) = mpsc::channel();
        sender.send(Some(vec![255, 0, 0, 255].repeat(4))).unwrap();
        sender.send(Some(vec![0, 255, 0, 255].repeat(4))).unwrap();
        encode_gif(
            receiver,
            &destination,
            [2, 2],
            2,
            2,
            &AtomicBool::new(false),
        )
        .unwrap();

        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&destination).unwrap(),
        ))
        .unwrap();
        let frames = decoder.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].buffer().get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(frames[1].buffer().get_pixel(0, 0).0, [0, 255, 0, 255]);

        std::fs::write(&destination, b"previous export").unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(Some(vec![0, 0, 255, 255].repeat(4))).unwrap();
        assert!(encode_gif(receiver, &destination, [2, 2], 2, 1, &AtomicBool::new(true),).is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"previous export");

        let destination = dir.path().join("invalid.gif");
        std::fs::write(&destination, b"keep on frame failure").unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(Some(vec![0, 0, 255])).unwrap();
        assert!(encode_gif(
            receiver,
            &destination,
            [2, 2],
            2,
            1,
            &AtomicBool::new(false),
        )
        .is_err());
        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"keep on frame failure"
        );
    }

    fn reveal_stroke(id: u64, layer_id: u64) -> crate::document::Stroke {
        crate::document::Stroke {
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id,
            id,
            color: [0.2, 0.4, 0.8],
            radius: 0.04,
            brush: Default::default(),
            points: vec![crate::document::Point {
                position: glam::Vec3::new(id as f32, 0.0, 0.0),
                pressure: 0.7,
                tilt: None,
            }],
        }
    }

    #[test]
    fn reconstructed_reveal_keeps_visible_source_order_and_distributes_prefixes() {
        let mut document = Document::default();
        let mut hidden_layer = document.layers[0].clone();
        hidden_layer.id = 2;
        hidden_layer.name = "Hidden".into();
        hidden_layer.visible = false;
        document.layers.push(hidden_layer);
        document.strokes = vec![
            reveal_stroke(9, 1),
            reveal_stroke(3, 2),
            reveal_stroke(7, 1),
        ];

        let saved = serde_json::to_string(&document).unwrap();
        let reopened = Document::from_json(&saved).unwrap();
        let visible = visible_reveal_strokes(&reopened).unwrap();
        assert_eq!(
            visible.iter().map(|stroke| stroke.id).collect::<Vec<_>>(),
            [9, 7],
            "reconstructed output follows saved visible stroke order"
        );
        assert_eq!(
            (0..4)
                .map(|frame| reveal_target_count(3, frame, 4))
                .collect::<Vec<_>>(),
            [1, 2, 3, 3]
        );
        assert_eq!(reveal_target_count(3, 100, 4), 3);
        assert_eq!(reveal_target_count(0, 0, 4), 0);
        assert_eq!(reveal_target_count(3, 0, 0), 0);

        let mut hidden = reopened;
        for layer in &mut hidden.layers {
            layer.visible = false;
        }
        assert!(visible_reveal_strokes(&hidden).is_err());
    }
}
