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
const MAX_GIF_FPS: u32 = 50;

fn gif_delay_centiseconds(fps: u32) -> u32 {
    ((100.0 / f64::from(fps.max(1))).round() as u32).max(1)
}

fn encoded_frame_duration_seconds(frame_count: usize, fps: u32) -> f64 {
    frame_count as f64 / f64::from(fps.max(1))
}

fn mp4_input_frame_rate(fps: u32) -> String {
    fps.to_string()
}

fn valid_export_fps(mode: ExportMode, format: VideoFormat, fps: u32) -> bool {
    match mode {
        ExportMode::ArtworkAnimation => {
            (u32::from(crate::animation::MIN_FPS)..=u32::from(crate::animation::MAX_FPS))
                .contains(&fps)
                && (format != VideoFormat::Gif || fps <= MAX_GIF_FPS)
        }
        _ => (4..=24).contains(&fps),
    }
}

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

fn append_reveal_prefix(
    document: &mut Document,
    strokes: &[crate::document::Stroke],
    revealed_stroke_count: &mut usize,
    frame_index: usize,
    frame_count: usize,
) {
    let target_count = reveal_target_count(strokes.len(), frame_index, frame_count);
    document.strokes.extend(
        strokes[*revealed_stroke_count..target_count]
            .iter()
            .cloned(),
    );
    *revealed_stroke_count = target_count;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportMode {
    Turntable,
    Sequence(u64),
    ReconstructedReveal,
    RecordedHistory,
    ArtworkAnimation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    #[cfg(test)]
    test_save_path: Option<Option<PathBuf>>,
    #[cfg(test)]
    test_reconstructed_mode_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_choose_file_rect: Option<egui::Rect>,
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
            #[cfg(test)]
            test_save_path: None,
            #[cfg(test)]
            test_reconstructed_mode_rect: None,
            #[cfg(test)]
            test_choose_file_rect: None,
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
        document: &Document,
    ) -> Option<f32> {
        if self.mode == ExportMode::RecordedHistory {
            let (start_ms, end_ms) = trim_range(history_duration_ms, self.history_trim_percent)?;
            if !self.history_speed.is_finite() || self.history_speed <= 0.0 {
                return None;
            }
            let selected_ms = end_ms.saturating_sub(start_ms) as f32;
            return Some(selected_ms / 1000.0 / self.history_speed);
        }
        if self.mode == ExportMode::ArtworkAnimation {
            return (!document.animation.is_empty()).then(|| {
                encoded_frame_duration_seconds(
                    document.animation.frame_count(),
                    u32::from(document.animation.fps),
                ) as f32
            });
        }
        let seconds = match self.mode {
            ExportMode::Turntable | ExportMode::ReconstructedReveal => self.duration,
            ExportMode::RecordedHistory | ExportMode::ArtworkAnimation => unreachable!(),
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
        document: &Document,
    ) -> Option<usize> {
        if self.mode == ExportMode::ArtworkAnimation {
            let count = document.animation.frame_count();
            return (2..=MAX_FRAMES).contains(&count).then_some(count);
        }
        let seconds = self.export_duration_seconds(sequences, history_duration_ms, document)?;
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
                        let reconstructed_mode = ui.selectable_value(
                            &mut self.mode,
                            ExportMode::ReconstructedReveal,
                            "Reconstructed reveal",
                        );
                        #[cfg(test)]
                        {
                            self.test_reconstructed_mode_rect = Some(reconstructed_mode.rect);
                        }
                        #[cfg(not(test))]
                        let _ = reconstructed_mode;
                        if history.has_history() {
                            ui.selectable_value(
                                &mut self.mode,
                                ExportMode::RecordedHistory,
                                "Recorded history",
                            );
                        }
                        if !document.animation.is_empty() {
                            ui.selectable_value(
                                &mut self.mode,
                                ExportMode::ArtworkAnimation,
                                "Artwork animation",
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
                        && self.mode != ExportMode::ArtworkAnimation
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
                    } else if self.mode == ExportMode::ArtworkAnimation {
                        ui.small("Exports the saved integer frame range at the animation FPS. Onion skins and editor overlays are excluded.");
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
                    if self.mode == ExportMode::ArtworkAnimation {
                        self.fps = u32::from(document.animation.fps);
                        ui.label(format!("{} frames per second", document.animation.fps));
                    } else {
                        ui.add(egui::Slider::new(&mut self.fps, 4..=24).text("Frames per second"));
                    }
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.format, VideoFormat::Gif, "GIF");
                        ui.selectable_value(&mut self.format, VideoFormat::Mp4, "MP4 (H.264)");
                    });
                    if self.format == VideoFormat::Mp4 {
                        ui.small("MP4 encoding uses FFmpeg with libx264, or installed Blender when FFmpeg is unavailable. Odd dimensions are padded by one pixel for H.264.");
                    } else if self.mode == ExportMode::ArtworkAnimation {
                        let delay = gif_delay_centiseconds(self.fps);
                        let effective_fps = 100.0 / f64::from(delay);
                        ui.small(format!(
                            "GIF delays use 10 ms steps: requested {} fps is encoded as {:.2} fps ({} ms per frame). GIF export supports at most {MAX_GIF_FPS} fps.",
                            self.fps,
                            effective_fps,
                            delay * 10,
                        ));
                    }
                    ui.label(format!("{} pixels", self.width as u64 * self.height as u64));
                    if let Some(count) = self.frame_count(sequences, history.duration_ms(), document) {
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
                        let choose_file = ui.add_enabled(
                            self.frame_count(sequences, history.duration_ms(), document).is_some()
                                && !(self.mode == ExportMode::ArtworkAnimation
                                    && self.format == VideoFormat::Gif
                                    && self.fps > MAX_GIF_FPS)
                                && (self.mode != ExportMode::RecordedHistory
                                    || self.history_trim_percent[1]
                                        > self.history_trim_percent[0])
                                    && u64::from(self.width) * u64::from(self.height) <= MAX_PIXELS,
                            egui::Button::new(match self.format {
                                VideoFormat::Gif => "Choose GIF file…",
                                VideoFormat::Mp4 => "Choose MP4 file…",
                            }),
                        );
                        choose = choose_file.clicked();
                        #[cfg(test)]
                        {
                            self.test_choose_file_rect = Some(choose_file.rect);
                        }
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
                let picker_result: Option<Option<PathBuf>> = {
                    #[cfg(test)]
                    {
                        self.test_save_path.take()
                    }
                    #[cfg(not(test))]
                    {
                        None
                    }
                };
                let selected_path = picker_result.unwrap_or_else(|| {
                    rfd::FileDialog::new()
                        .add_filter(filter, &[extension])
                        .set_file_name(filename)
                        .save_file()
                });
                if let Some(mut path) = selected_path {
                    path.set_extension(extension);
                    let history_total_ms = history.duration_ms();
                    let request = VideoRequest {
                        path: path.clone(),
                        size: [self.width, self.height],
                        fps: self.fps,
                        mode: self.mode,
                        duration: self
                            .export_duration_seconds(sequences, history_total_ms, document)
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
            ExportMode::ArtworkAnimation => {
                if document.animation.is_empty() {
                    return Err("Artwork animation has no keyframes to export".into());
                }
                if !document.animation.current_pose_matches(&document.strokes) {
                    return Err("Set or update the current artwork keyframe before export".into());
                }
                if u32::from(document.animation.fps) != request.fps {
                    return Err("Artwork animation export rate must match its saved FPS".into());
                }
                document.animation.duration_seconds()
            }
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
        if request.mode == ExportMode::ArtworkAnimation {
            frame_count = document.animation.frame_count();
        }
        if request.mode == ExportMode::RecordedHistory && seconds.is_finite() && seconds > 0.0 {
            frame_count = frame_count.max(2);
        }
        let valid_fps = valid_export_fps(request.mode, request.format, request.fps);
        if request.size.contains(&0)
            || u64::from(request.size[0]) * u64::from(request.size[1]) > MAX_PIXELS
            || !valid_fps
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
                ExportMode::ArtworkAnimation => {
                    let frame = job
                        .document
                        .animation
                        .range_start
                        .saturating_add(job.next_frame as u32);
                    if let Some(strokes) = job.document.animation.resolve(frame) {
                        job.document.strokes = strokes;
                    } else {
                        valid_camera = false;
                    }
                }
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
                    append_reveal_prefix(
                        &mut job.document,
                        strokes,
                        &mut job.revealed_stroke_count,
                        job.next_frame,
                        job.frame_count,
                    );
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

pub(crate) fn encode_gif(
    receiver: Receiver<Option<Vec<u8>>>,
    destination: &std::path::Path,
    size: [u32; 2],
    fps: u32,
    frame_count: usize,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    encode_gif_with_repeat(
        receiver,
        destination,
        size,
        fps,
        frame_count,
        cancelled,
        Some(image::codecs::gif::Repeat::Infinite),
    )
}

pub(crate) fn encode_gif_once(
    receiver: Receiver<Option<Vec<u8>>>,
    destination: &std::path::Path,
    size: [u32; 2],
    fps: u32,
    frame_count: usize,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    encode_gif_with_repeat(
        receiver,
        destination,
        size,
        fps,
        frame_count,
        cancelled,
        None,
    )
}

fn encode_gif_with_repeat(
    receiver: Receiver<Option<Vec<u8>>>,
    destination: &std::path::Path,
    size: [u32; 2],
    fps: u32,
    frame_count: usize,
    cancelled: &AtomicBool,
    repeat: Option<image::codecs::gif::Repeat>,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    {
        let mut encoder = GifEncoder::new(&mut temporary);
        if let Some(repeat) = repeat {
            encoder
                .set_repeat(repeat)
                .map_err(|error| error.to_string())?;
        }
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
                    Delay::from_numer_denom_ms(gif_delay_centiseconds(fps) * 10, 1),
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

fn blender_executable() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("SPATIAL_SKETCH_BLENDER") {
        candidates.push(PathBuf::from(path));
    }
    #[cfg(windows)]
    let executable_name = "blender.exe";
    #[cfg(not(windows))]
    let executable_name = "blender";
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            candidates.push(directory.join(executable_name));
        }
    }
    #[cfg(windows)]
    for root in [
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramFiles(x86)"),
        std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("Programs").into_os_string()),
    ]
    .into_iter()
    .flatten()
    {
        let root = PathBuf::from(root).join("Blender Foundation");
        if let Ok(entries) = std::fs::read_dir(root) {
            let mut installs: Vec<_> = entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("Blender "))
                .map(|entry| entry.path().join(executable_name))
                .collect();
            installs.sort_by(|left, right| right.cmp(left));
            candidates.extend(installs);
        }
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn pad_rgba_frame(pixels: &[u8], size: [u32; 2], padded_size: [u32; 2]) -> Option<Vec<u8>> {
    if size.contains(&0) || padded_size[0] < size[0] || padded_size[1] < size[1] {
        return None;
    }
    let row_bytes = size[0] as usize * 4;
    if pixels.len() != row_bytes.checked_mul(size[1] as usize)? {
        return None;
    }
    let padded_row_bytes = padded_size[0] as usize * 4;
    let mut padded = vec![0; padded_row_bytes.checked_mul(padded_size[1] as usize)?];
    for row in 0..size[1] as usize {
        let source_start = row * row_bytes;
        let target_start = row * padded_row_bytes;
        padded[target_start..target_start + row_bytes]
            .copy_from_slice(&pixels[source_start..source_start + row_bytes]);
        if padded_size[0] > size[0] {
            padded[target_start + row_bytes + 3] = 255;
        }
    }
    if padded_size[1] > size[1] {
        let bottom_row_start = size[1] as usize * padded_row_bytes;
        for pixel in padded[bottom_row_start..].as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
    }
    Some(padded)
}

fn encode_mp4_with_blender(
    receiver: Receiver<Option<Vec<u8>>>,
    destination: &std::path::Path,
    size: [u32; 2],
    fps: u32,
    frame_count: usize,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let blender = blender_executable().ok_or_else(|| {
        "MP4 export requires FFmpeg with libx264 or Blender with its H.264 video encoder."
            .to_owned()
    })?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let work = tempfile::Builder::new()
        .prefix(".sketch3d-mp4-")
        .tempdir_in(parent)
        .map_err(|error| error.to_string())?;
    let frames = work.path().join("frames");
    std::fs::create_dir(&frames).map_err(|error| error.to_string())?;
    let script = work.path().join("encode_mp4.py");
    std::fs::write(&script, include_str!("../scripts/blender_mp4_encoder.py"))
        .map_err(|error| error.to_string())?;
    let encoded_path = work.path().join("encoded.mp4");
    let padded_size = [size[0] + size[0] % 2, size[1] + size[1] % 2];
    let expected_len = size[0] as usize * size[1] as usize * 4;

    for frame_index in 0..frame_count {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Export cancelled".into());
        }
        let pixels = match receiver.recv() {
            Ok(Some(pixels)) => pixels,
            Ok(None) | Err(_) => return Err("Export cancelled".into()),
        };
        if pixels.len() != expected_len {
            return Err("Rendered frame has invalid dimensions".into());
        }
        let padded;
        let frame = if padded_size == size {
            pixels.as_slice()
        } else {
            padded = pad_rgba_frame(&pixels, size, padded_size)
                .ok_or_else(|| "Could not pad an RGBA video frame".to_owned())?;
            padded.as_slice()
        };
        let frame_path = frames.join(format!("frame_{:06}.png", frame_index + 1));
        image::save_buffer(
            &frame_path,
            frame,
            padded_size[0],
            padded_size[1],
            image::ColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err("Export cancelled".into());
    }

    let stdout_log_path = work.path().join("blender.stdout.log");
    let stderr_log_path = work.path().join("blender.stderr.log");
    let stdout_log = std::fs::File::create(&stdout_log_path).map_err(|error| error.to_string())?;
    let stderr_log = std::fs::File::create(&stderr_log_path).map_err(|error| error.to_string())?;
    let mut command = Command::new(&blender);
    command
        .args([
            "--background",
            "--factory-startup",
            "--python-exit-code",
            "1",
            "--python",
        ])
        .arg(&script)
        .args(["--"])
        .arg(&frames)
        .arg(&encoded_path)
        .arg(padded_size[0].to_string())
        .arg(padded_size[1].to_string())
        .arg(fps.to_string())
        .arg(frame_count.to_string())
        .current_dir(work.path())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log));
    for (setting, directory) in [
        ("BLENDER_USER_CONFIG", "config"),
        ("BLENDER_USER_SCRIPTS", "scripts"),
        ("BLENDER_USER_DATAFILES", "datafiles"),
        ("BLENDER_USER_EXTENSIONS", "extensions"),
    ] {
        let path = work.path().join(directory);
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        command.env(setting, path);
    }
    let mut child = command
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start Blender MP4 encoder: {error}"))?;
    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Export cancelled".into());
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => break status,
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };
    if !status.success() {
        let detail = std::fs::read_to_string(&stderr_log_path)
            .unwrap_or_default()
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" ");
        return Err(if detail.is_empty() {
            "Blender could not encode MP4 with its H.264 video encoder".into()
        } else {
            format!("Blender MP4 encoding failed: {detail}")
        });
    }
    let mut movie = std::fs::File::open(&encoded_path)
        .map_err(|error| format!("Blender did not produce an MP4: {error}"))?;
    if movie.metadata().map_err(|error| error.to_string())?.len() <= 64 {
        return Err("Blender produced an empty MP4 file".into());
    }
    use std::io::{Read, Seek, SeekFrom};
    movie
        .seek(SeekFrom::Start(4))
        .map_err(|error| error.to_string())?;
    let mut signature = [0; 4];
    movie
        .read_exact(&mut signature)
        .map_err(|error| error.to_string())?;
    if &signature != b"ftyp" {
        return Err("Blender output is not an MP4 container".into());
    }
    movie
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut temporary = tempfile::Builder::new()
        .suffix(".mp4")
        .tempfile_in(parent)
        .map_err(|error| error.to_string())?;
    std::io::copy(&mut movie, temporary.as_file_mut()).map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(destination)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}

/// Stream raw RGBA frames to FFmpeg; if it is absent, encode a temporary PNG
/// sequence through Blender's bundled FFmpeg video writer. The final file is
/// installed only after a clean encoder exit.
pub(crate) fn encode_mp4(
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
    let ffmpeg = Command::new("ffmpeg")
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
        .args([
            "-framerate",
            &mp4_input_frame_rate(fps),
            "-i",
            "pipe:0",
            "-an",
            "-vf",
        ])
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
        .spawn();
    let mut child = match ffmpeg {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            drop(temporary);
            return encode_mp4_with_blender(
                receiver,
                destination,
                size,
                fps,
                frame_count,
                cancelled,
            );
        }
        Err(error) => return Err(format!("Could not start FFmpeg: {error}")),
    };
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("MP4 encoder did not open its input stream".into());
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
    fn gif_animation_uses_reportable_centisecond_delay_and_caps_at_50_fps() {
        assert_eq!(gif_delay_centiseconds(24), 4);
        assert_eq!(100.0 / f64::from(gif_delay_centiseconds(24)), 25.0);
        assert_eq!(gif_delay_centiseconds(50), 2);
        assert!(valid_export_fps(
            ExportMode::ArtworkAnimation,
            VideoFormat::Gif,
            50
        ));
        assert!(!valid_export_fps(
            ExportMode::ArtworkAnimation,
            VideoFormat::Gif,
            51
        ));
    }

    #[test]
    fn mp4_animation_keeps_exact_integer_frame_rate_and_duration() {
        assert!(valid_export_fps(
            ExportMode::ArtworkAnimation,
            VideoFormat::Mp4,
            60
        ));
        assert_eq!(encoded_frame_duration_seconds(60, 24), 2.5);
        assert_eq!(encoded_frame_duration_seconds(60, 60), 1.0);
        assert_eq!(mp4_input_frame_rate(60), "60");
        assert!(!valid_export_fps(
            ExportMode::Turntable,
            VideoFormat::Mp4,
            60
        ));
    }

    #[test]
    fn production_artwork_animation_mode_renders_and_encodes_saved_gif_track() {
        use image::AnimationDecoder;

        const SIZE: [u32; 2] = [64, 64];
        const FPS: u32 = 24;
        let Some(state) = headless_render_state_for_test() else {
            return;
        };
        let mut renderer = crate::renderer::SceneRenderer::new(state);
        let mut document = crate::export_check::brush_fixture();
        let initial_strokes = document.strokes.clone();
        let mut moved_strokes = initial_strokes.clone();
        for stroke in &mut moved_strokes {
            for point in &mut stroke.points {
                point.position.x += 0.8;
            }
        }
        let mut animation = crate::animation::ArtworkAnimation {
            range_end: 1,
            fps: FPS as u8,
            ..crate::animation::ArtworkAnimation::default()
        };
        animation = animation.set_keyframe(0, &initial_strokes).unwrap();
        animation = animation.set_keyframe(1, &moved_strokes).unwrap();
        animation.current_frame = 0;
        document.animation = animation;
        let original = document.clone();
        let output_root = std::env::current_dir().unwrap().join("verification");
        std::fs::create_dir_all(&output_root).unwrap();
        let temp = tempfile::tempdir_in(output_root).unwrap();
        let path = temp.path().join("artwork-track.gif");
        let request = VideoRequest {
            path: path.clone(),
            size: SIZE,
            fps: FPS,
            mode: ExportMode::ArtworkAnimation,
            duration: 2.0 / FPS as f32,
            history_total_ms: 0,
            history_trim_percent: [0.0, 100.0],
            history_speed: 1.0,
            format: VideoFormat::Gif,
            replace: false,
        };
        let camera = crate::geometry::Camera {
            projection: crate::geometry::Projection::Orthographic,
            distance: 7.0,
            pitch: 0.38,
            ..Default::default()
        };
        let bookmarks = crate::camera_bookmarks::CameraBookmarkStore::default();
        let history = crate::construction_history::ConstructionHistory::empty_for_test();
        let mut export_ui = VideoExportUi::default();
        export_ui
            .start(
                request,
                &document,
                camera,
                &bookmarks,
                [0.08, 0.12, 0.2],
                crate::lighting::Lighting {
                    ground: false,
                    shadows: false,
                    ..Default::default()
                },
                &history,
                ExportResourceAvailability {
                    image_plane_ids: HashSet::new(),
                    model_reference_ids: HashSet::new(),
                },
            )
            .unwrap();
        let context = egui::Context::default();
        let status = (0..10_000)
            .find_map(|_| {
                let status = export_ui.render_next(&mut renderer, &context);
                if status.is_none() {
                    thread::sleep(Duration::from_millis(2));
                }
                status
            })
            .expect("GIF encoder finishes within polling window");
        assert!(status.starts_with("Exported video to"), "{status}");
        assert_eq!(
            document.strokes, original.strokes,
            "editor strokes are unchanged"
        );
        assert_eq!(
            document.animation, original.animation,
            "editor track is unchanged"
        );

        let frames = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(path).unwrap(),
        ))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
        assert_eq!(frames.len(), 2);
        let (numerator, denominator) = frames[0].delay().numer_denom_ms();
        assert_eq!(numerator / denominator, gif_delay_centiseconds(FPS) * 10);
        let changed_pixels = frames[0]
            .buffer()
            .pixels()
            .zip(frames[1].buffer().pixels())
            .filter(|(left, right)| left.0 != right.0)
            .count();
        assert!(
            changed_pixels > 10,
            "keyframe motion changes rendered pixels"
        );
    }

    #[test]
    fn production_artwork_animation_mode_exports_mp4_with_exact_rate_when_available() {
        if !ffmpeg_is_available() && blender_executable().is_none() {
            eprintln!(
                "Skipping production artwork MP4 export: neither FFmpeg nor Blender is installed."
            );
            return;
        }
        let Some(state) = headless_render_state_for_test() else {
            return;
        };
        const SIZE: [u32; 2] = [64, 64];
        const FPS: u32 = 24;
        let mut renderer = crate::renderer::SceneRenderer::new(state);
        let mut document = crate::export_check::brush_fixture();
        let initial_strokes = document.strokes.clone();
        let mut moved_strokes = initial_strokes.clone();
        for stroke in &mut moved_strokes {
            for point in &mut stroke.points {
                point.position.x += 0.8;
            }
        }
        let mut animation = crate::animation::ArtworkAnimation {
            range_end: 1,
            fps: FPS as u8,
            ..crate::animation::ArtworkAnimation::default()
        };
        animation = animation.set_keyframe(0, &initial_strokes).unwrap();
        animation = animation.set_keyframe(1, &moved_strokes).unwrap();
        animation.current_frame = 0;
        document.animation = animation;
        let original = document.clone();
        let output_root = std::env::current_dir().unwrap().join("verification");
        std::fs::create_dir_all(&output_root).unwrap();
        let temp = tempfile::tempdir_in(output_root).unwrap();
        let path = temp.path().join("artwork-track.mp4");
        let request = VideoRequest {
            path: path.clone(),
            size: SIZE,
            fps: FPS,
            mode: ExportMode::ArtworkAnimation,
            duration: 2.0 / FPS as f32,
            history_total_ms: 0,
            history_trim_percent: [0.0, 100.0],
            history_speed: 1.0,
            format: VideoFormat::Mp4,
            replace: false,
        };
        let camera = crate::geometry::Camera {
            projection: crate::geometry::Projection::Orthographic,
            distance: 7.0,
            pitch: 0.38,
            ..Default::default()
        };
        let bookmarks = crate::camera_bookmarks::CameraBookmarkStore::default();
        let history = crate::construction_history::ConstructionHistory::empty_for_test();
        let mut export_ui = VideoExportUi::default();
        export_ui
            .start(
                request,
                &document,
                camera,
                &bookmarks,
                [0.08, 0.12, 0.2],
                crate::lighting::Lighting {
                    ground: false,
                    shadows: false,
                    ..Default::default()
                },
                &history,
                ExportResourceAvailability {
                    image_plane_ids: HashSet::new(),
                    model_reference_ids: HashSet::new(),
                },
            )
            .unwrap();
        let context = egui::Context::default();
        let status = (0..10_000)
            .find_map(|_| {
                let status = export_ui.render_next(&mut renderer, &context);
                if status.is_none() {
                    thread::sleep(Duration::from_millis(2));
                }
                status
            })
            .expect("MP4 encoder finishes within polling window");
        assert!(status.starts_with("Exported video to"), "{status}");
        assert_eq!(document.strokes, original.strokes);
        assert_eq!(document.animation, original.animation);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(mp4_sample_count(&bytes), Some(2));
        let mp4_fps = mp4_average_video_fps(&bytes)
            .expect("MP4 media timescale and sample table expose encoded frame rate");
        assert!(
            (mp4_fps - f64::from(FPS)).abs() < 0.001,
            "encoded FPS: {mp4_fps}"
        );

        match Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=r_frame_rate,nb_frames",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(&path)
            .output()
        {
            Ok(output) if output.status.success() => {
                let metadata = String::from_utf8_lossy(&output.stdout);
                assert!(
                    metadata.lines().any(|line| line == "r_frame_rate=24/1"),
                    "{metadata}"
                );
                assert!(
                    metadata.lines().any(|line| line == "nb_frames=2"),
                    "{metadata}"
                );
            }
            _ if ffmpeg_is_available() => {
                let output = Command::new("ffmpeg")
                    .args(["-hide_banner", "-i"])
                    .arg(&path)
                    .output()
                    .expect("installed FFmpeg inspects encoded MP4 metadata");
                let metadata = String::from_utf8_lossy(&output.stderr);
                assert!(metadata.contains("24 fps"), "{metadata}");
            }
            _ => eprintln!("ffprobe/FFmpeg metadata commands are absent; frame count and FPS were verified from MP4 sample tables."),
        }
    }
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
        thread,
    };

    struct ThreadWake(thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => thread::park(),
            }
        }
    }

    fn headless_render_state_for_test() -> Option<eframe::egui_wgpu::RenderState> {
        let config = eframe::egui_wgpu::WgpuConfiguration::default();
        let instance = block_on(config.wgpu_setup.new_instance());
        match block_on(eframe::egui_wgpu::RenderState::create(
            &config, &instance, None, None, 1, false,
        )) {
            Ok(state) => Some(state),
            Err(eframe::egui_wgpu::WgpuError::NoSuitableAdapterFound(reason)) => {
                eprintln!(
                    "skipping GPU-backed video export test: no suitable WGPU adapter: {reason}"
                );
                None
            }
            Err(error) => panic!("headless WGPU render state initializes: {error}"),
        }
    }

    fn mp4_sample_count(bytes: &[u8]) -> Option<u32> {
        bytes
            .windows(4)
            .enumerate()
            .filter(|(_, window)| *window == b"stsz")
            .filter_map(|(index, _)| {
                bytes
                    .get(index + 12..index + 16)
                    .and_then(|sample_count| sample_count.try_into().ok())
                    .map(u32::from_be_bytes)
            })
            .max()
    }

    fn mp4_average_video_fps(bytes: &[u8]) -> Option<f64> {
        let mdhd = bytes.windows(4).position(|window| window == b"mdhd")?;
        let version = *bytes.get(mdhd + 4)?;
        let timescale_at = mdhd + if version == 1 { 24 } else { 16 };
        let timescale =
            u32::from_be_bytes(bytes.get(timescale_at..timescale_at + 4)?.try_into().ok()?);
        if timescale == 0 {
            return None;
        }
        let stts = bytes.windows(4).position(|window| window == b"stts")?;
        let entry_count =
            u32::from_be_bytes(bytes.get(stts + 8..stts + 12)?.try_into().ok()?) as usize;
        let mut offset = stts + 12;
        let mut samples = 0u64;
        let mut ticks = 0u64;
        for _ in 0..entry_count {
            let count = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as u64;
            let delta =
                u32::from_be_bytes(bytes.get(offset + 4..offset + 8)?.try_into().ok()?) as u64;
            samples = samples.checked_add(count)?;
            ticks = ticks.checked_add(count.checked_mul(delta)?)?;
            offset += 8;
        }
        (ticks > 0).then_some(samples as f64 * f64::from(timescale) / ticks as f64)
    }

    fn mp4_media_data(bytes: &[u8]) -> Option<&[u8]> {
        let mut offset = 0usize;
        while offset.checked_add(8)? <= bytes.len() {
            let box_size = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?);
            let kind = bytes.get(offset + 4..offset + 8)?;
            let (box_size, header_size) = match box_size {
                0 => (bytes.len() - offset, 8usize),
                1 => (
                    usize::try_from(u64::from_be_bytes(
                        bytes.get(offset + 8..offset + 16)?.try_into().ok()?,
                    ))
                    .ok()?,
                    16usize,
                ),
                size => (size as usize, 8usize),
            };
            let end = offset.checked_add(box_size)?;
            if box_size < header_size || end > bytes.len() {
                return None;
            }
            if kind == b"mdat" {
                return bytes.get(offset + header_size..end);
            }
            offset = end;
        }
        None
    }

    fn ffmpeg_is_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    fn vlc_executable() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                candidates.push(directory.join(if cfg!(windows) { "vlc.exe" } else { "vlc" }));
            }
        }
        #[cfg(windows)]
        for root in [
            std::env::var_os("ProgramFiles"),
            std::env::var_os("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        {
            candidates.push(PathBuf::from(root).join("VideoLAN/VLC/vlc.exe"));
        }
        candidates.into_iter().find(|candidate| candidate.is_file())
    }

    fn decode_mp4_with_vlc(vlc: &std::path::Path, movie: &std::path::Path, raw: &std::path::Path) {
        let sout = format!(
            "#transcode{{vcodec=RV24}}:standard{{access=file,mux=raw,dst=\"{}\"}}",
            raw.display()
        );
        let mut decoder = Command::new(vlc);
        decoder
            .args([
                "-I",
                "dummy",
                "--ignore-config",
                "--no-media-library",
                "--no-one-instance",
                "--no-audio",
            ])
            .arg(format!("--sout={sout}"))
            .arg(movie)
            .arg("--play-and-exit")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = decoder.spawn().expect("VLC decoder starts");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            match child.try_wait().expect("poll VLC decoder") {
                Some(status) => break status,
                None if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("VLC MP4 decoder timed out");
                }
            }
        };
        assert!(status.success(), "VLC decodes the MP4 without errors");
    }

    fn render_video_dialog(
        ctx: &egui::Context,
        ui: &mut VideoExportUi,
        history: &mut crate::construction_history::ConstructionHistory,
        document: &Document,
        camera: &crate::geometry::Camera,
        tool_settings: &crate::construction_history::ToolSettingsSnapshot,
        events: Vec<egui::Event>,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(900.0, 700.0),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                ui.show(
                    ctx,
                    &[],
                    history,
                    document,
                    HistoryRecordingContext {
                        source_key: "video-export-ui-test",
                        camera,
                        tool_settings,
                    },
                );
            },
        );
    }

    fn click_video_dialog(
        ctx: &egui::Context,
        ui: &mut VideoExportUi,
        history: &mut crate::construction_history::ConstructionHistory,
        document: &Document,
        camera: &crate::geometry::Camera,
        tool_settings: &crate::construction_history::ToolSettingsSnapshot,
        position: egui::Pos2,
    ) {
        render_video_dialog(
            ctx,
            ui,
            history,
            document,
            camera,
            tool_settings,
            vec![egui::Event::PointerMoved(position)],
        );
        render_video_dialog(
            ctx,
            ui,
            history,
            document,
            camera,
            tool_settings,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        render_video_dialog(
            ctx,
            ui,
            history,
            document,
            camera,
            tool_settings,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
    }

    #[test]
    fn blender_padded_frame_adds_opaque_black_right_and_bottom_edges() {
        let source = [10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255];
        let padded = pad_rgba_frame(&source, [1, 3], [2, 4]).unwrap();
        assert_eq!(
            padded,
            [
                10, 20, 30, 255, 0, 0, 0, 255, // first row and right edge
                40, 50, 60, 255, 0, 0, 0, 255, // second row and right edge
                70, 80, 90, 255, 0, 0, 0, 255, // third row and right edge
                0, 0, 0, 255, 0, 0, 0, 255, // opaque black padded bottom row
            ]
        );
        assert!(pad_rgba_frame(&source, [1, 3], [1, 4]).is_some());
        assert!(pad_rgba_frame(&source, [1, 3], [0, 4]).is_none());
    }

    #[test]
    fn production_mp4_fallback_encodes_and_decodes_when_blender_is_installed() {
        let Some(blender) = blender_executable() else {
            eprintln!("Skipping Blender MP4 fallback check: Blender is not installed.");
            return;
        };
        if ffmpeg_is_available() {
            eprintln!("Skipping Blender fallback check: FFmpeg is installed and takes priority.");
            return;
        }

        const SIZE: [u32; 2] = [63, 47];
        const PADDED_SIZE: [u32; 2] = [64, 48];
        const FPS: u32 = 8;
        const FRAME_COUNT: usize = 8;
        let output_root = std::env::current_dir()
            .expect("workspace directory")
            .join("verification");
        std::fs::create_dir_all(&output_root).expect("verification directory exists");
        let temporary = tempfile::tempdir_in(&output_root).expect("MP4 test temp directory");
        let movie_path = temporary.path().join("odd size H264 preview.mp4");
        let (sender, receiver) = mpsc::channel();
        for frame_index in 0..FRAME_COUNT {
            let mut frame = vec![0; SIZE[0] as usize * SIZE[1] as usize * 4];
            for pixel in frame.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[220, 80, 20 + frame_index as u8, 255]);
            }
            sender.send(Some(frame)).expect("send rendered frame");
        }
        drop(sender);
        encode_mp4(
            receiver,
            &movie_path,
            SIZE,
            FPS,
            FRAME_COUNT,
            &AtomicBool::new(false),
        )
        .expect("installed Blender encodes the MP4 stream");

        let movie = std::fs::read(&movie_path).expect("encoded MP4 exists");
        assert!(movie.len() > 64, "encoded MP4 is nonempty");
        assert_eq!(&movie[4..8], b"ftyp", "output is an MP4 container");
        assert_eq!(mp4_sample_count(&movie), Some(FRAME_COUNT as u32));
        assert!(mp4_media_data(&movie).is_some_and(|payload| !payload.is_empty()));

        let mut decoded_frame_count = None;
        if let Some(vlc) = vlc_executable() {
            let decoded_path = temporary.path().join("decoded RGB frames.raw");
            decode_mp4_with_vlc(&vlc, &movie_path, &decoded_path);
            let decoded = std::fs::read(&decoded_path).expect("VLC produces decoded frames");
            let frame_bytes = PADDED_SIZE[0] as usize * PADDED_SIZE[1] as usize * 3;
            assert!(decoded.len() >= frame_bytes);
            let center = (PADDED_SIZE[1] as usize / 2 * PADDED_SIZE[0] as usize
                + PADDED_SIZE[0] as usize / 2)
                * 3;
            for (actual, expected) in decoded[center..center + 3].iter().zip([220u8, 80, 20]) {
                assert!(
                    actual.abs_diff(expected) <= 20,
                    "decoded RGB color is preserved: actual={actual}, expected={expected}"
                );
            }
            decoded_frame_count = Some(decoded.len() / frame_bytes);
        }

        let report = serde_json::json!({
            "passed": true,
            "backend": "Blender H.264 fallback",
            "encoder_path": blender,
            "input_dimensions": SIZE,
            "encoded_dimensions": PADDED_SIZE,
            "fps": FPS,
            "frame_count": FRAME_COUNT,
            "mp4_sample_count": mp4_sample_count(&movie),
            "decoded_frame_count": decoded_frame_count,
            "mp4_bytes": movie.len(),
            "checks": ["odd RGBA frames are padded to even H.264 dimensions", "Blender writes the requested number of H.264 MP4 samples", "VLC playback decode preserves RGB color when VLC is installed"],
            "physical_hardware_tested": false
        });
        let report_directory = output_root.join("video-export-check");
        std::fs::create_dir_all(&report_directory).expect("video export report directory exists");
        let report_path = report_directory.join("mp4-runtime-report.json");
        std::fs::write(
            report_path,
            serde_json::to_vec_pretty(&report).expect("MP4 report serializes"),
        )
        .expect("MP4 runtime report writes");
    }

    #[test]
    fn reconstructed_reveal_dialog_click_queues_gif_request() {
        let ctx = egui::Context::default();
        let mut ui = VideoExportUi::default();
        ui.begin();
        let mut history = crate::construction_history::ConstructionHistory::empty_for_test();
        let document = Document::default();
        let camera = crate::geometry::Camera::default();
        let tool_settings = crate::construction_history::ToolSettingsSnapshot {
            tool: "brush".into(),
            radius: 0.1,
            color: [0.2, 0.4, 0.8],
            stabilization: 0.0,
            brush: Default::default(),
        };

        render_video_dialog(
            &ctx,
            &mut ui,
            &mut history,
            &document,
            &camera,
            &tool_settings,
            Vec::new(),
        );
        let mode_position = ui
            .test_reconstructed_mode_rect
            .expect("reconstructed reveal choice is visible")
            .center();
        click_video_dialog(
            &ctx,
            &mut ui,
            &mut history,
            &document,
            &camera,
            &tool_settings,
            mode_position,
        );
        assert_eq!(ui.mode, ExportMode::ReconstructedReveal);
        assert_eq!(
            ui.frame_count(&[], history.duration_ms(), &document),
            Some(60)
        );

        let output = tempfile::tempdir().unwrap();
        let choose_position = ui
            .test_choose_file_rect
            .expect("GIF file selection control is visible")
            .center();
        ui.test_save_path = Some(None);
        click_video_dialog(
            &ctx,
            &mut ui,
            &mut history,
            &document,
            &camera,
            &tool_settings,
            choose_position,
        );
        assert!(
            ui.take_request().is_none(),
            "canceling the save picker queues no export"
        );
        assert!(
            ui.dialog_open,
            "canceling the save picker leaves the export dialog open"
        );

        let selected_path = output.path().join("reconstructed.preview.png");
        let expected_path = output.path().join("reconstructed.preview.gif");
        ui.test_save_path = Some(Some(selected_path));
        let choose_position = ui
            .test_choose_file_rect
            .expect("GIF file selection control remains available after picker cancel")
            .center();
        click_video_dialog(
            &ctx,
            &mut ui,
            &mut history,
            &document,
            &camera,
            &tool_settings,
            choose_position,
        );

        let request = ui
            .take_request()
            .expect("clicking Choose GIF file queues the export request");
        assert_eq!(
            request.path, expected_path,
            "the selected path uses the requested GIF extension"
        );
        assert_eq!(request.mode, ExportMode::ReconstructedReveal);
        assert_eq!(request.format, VideoFormat::Gif);
        assert_eq!(request.size, [640, 360]);
        assert_eq!(request.fps, 12);
        assert_eq!(request.duration, 5.0);
    }

    #[test]
    fn production_renderer_turntable_gif_decodes_as_moving_playback() {
        use image::AnimationDecoder;

        const SIZE: [u32; 2] = [320, 240];
        const FRAME_COUNT: usize = 8;
        const FPS: u32 = 8;
        let Some(state) = headless_render_state_for_test() else {
            return;
        };
        let adapter = state.adapter.get_info();
        let mut renderer = crate::renderer::SceneRenderer::new(state);
        let lighting = crate::lighting::Lighting {
            ground: false,
            shadows: false,
            ..Default::default()
        };
        let document = crate::export_check::brush_fixture();
        let background = [0.08, 0.12, 0.2];
        let start_camera = crate::geometry::Camera {
            projection: crate::geometry::Projection::Orthographic,
            distance: 7.0,
            pitch: 0.38,
            ..Default::default()
        };
        let ui_context = egui::Context::default();
        let render_turntable =
            |renderer: &mut crate::renderer::SceneRenderer, theme: crate::presentation::UiTheme| {
                crate::app::ui_style::apply(&ui_context, theme);
                let mut rendered = Vec::with_capacity(FRAME_COUNT);
                for frame_index in 0..FRAME_COUNT {
                    let mut camera = start_camera;
                    camera.yaw = start_camera.yaw
                        + std::f32::consts::TAU * frame_index as f32 / FRAME_COUNT as f32;
                    rendered.push(
                        renderer
                            .render_png_pixels(&document, &camera, SIZE, background, lighting)
                            .expect("turntable frame renders through the production renderer"),
                    );
                }
                rendered
            };
        let rendered = render_turntable(&mut renderer, crate::presentation::UiTheme::Dark);
        let light_theme_rendered =
            render_turntable(&mut renderer, crate::presentation::UiTheme::Light);
        assert_eq!(
            rendered, light_theme_rendered,
            "fixed video frames are independent of the live Dark/Light UI theme"
        );

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/video-export-check");
        std::fs::create_dir_all(&output).unwrap();
        let gif_path = output.join("turntable.gif");
        let (sender, receiver) = mpsc::channel();
        for frame in &rendered {
            sender.send(Some(frame.clone())).unwrap();
        }
        drop(sender);
        encode_gif(
            receiver,
            &gif_path,
            SIZE,
            FPS,
            FRAME_COUNT,
            &AtomicBool::new(false),
        )
        .expect("production GIF encoder writes rendered turntable frames");

        let light_theme_output = tempfile::tempdir_in(&output).unwrap();
        let light_gif_path = light_theme_output.path().join("turntable.gif");
        let (sender, receiver) = mpsc::channel();
        for frame in &light_theme_rendered {
            sender.send(Some(frame.clone())).unwrap();
        }
        drop(sender);
        encode_gif(
            receiver,
            &light_gif_path,
            SIZE,
            FPS,
            FRAME_COUNT,
            &AtomicBool::new(false),
        )
        .expect("production GIF encoder writes the Light-theme comparison frames");

        let frames = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&gif_path).unwrap(),
        ))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
        assert_eq!(frames.len(), FRAME_COUNT, "playback retains every frame");
        let light_theme_frames = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&light_gif_path).unwrap(),
        ))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
        assert_eq!(light_theme_frames.len(), FRAME_COUNT);
        for (dark, light) in frames.iter().zip(&light_theme_frames) {
            assert_eq!(
                dark.buffer(),
                light.buffer(),
                "decoded GIF playback frames are identical across Dark/Light themes"
            );
        }
        assert!(frames.iter().all(|frame| {
            frame.buffer().width() == SIZE[0] && frame.buffer().height() == SIZE[1]
        }));
        assert!(
            frames.iter().all(|frame| {
                let (numerator, denominator) = frame.delay().numer_denom_ms();
                (numerator as f64 / denominator as f64 - 1000.0 / f64::from(FPS)).abs() <= 5.0
            }),
            "playback frame delays match the requested {FPS} FPS"
        );

        let first = frames[0].buffer();
        let opposite = frames[FRAME_COUNT / 2].buffer();
        let (delay_numerator, delay_denominator) = frames[0].delay().numer_denom_ms();
        let frame_delay_ms = delay_numerator / delay_denominator;
        let changed_pixels = first
            .pixels()
            .zip(opposite.pixels())
            .filter(|(left, right)| left.0 != right.0)
            .count();
        assert!(
            changed_pixels > (SIZE[0] * SIZE[1] / 20) as usize,
            "opposite turntable views visibly differ, changed pixels: {changed_pixels}"
        );

        let mp4_theme_check = if ffmpeg_is_available() || blender_executable().is_some() {
            let dark_mp4 = output.join("turntable-dark.mp4");
            let light_mp4 = output.join("turntable-light.mp4");
            for path in [&dark_mp4, &light_mp4] {
                if path.is_file() {
                    std::fs::remove_file(path).expect("remove prior generated MP4 artifact");
                }
            }
            for (theme_frames, path) in
                [(&rendered, &dark_mp4), (&light_theme_rendered, &light_mp4)]
            {
                let (sender, receiver) = mpsc::channel();
                for frame in theme_frames {
                    sender.send(Some(frame.clone())).expect("send MP4 frame");
                }
                drop(sender);
                encode_mp4(
                    receiver,
                    path,
                    SIZE,
                    FPS,
                    FRAME_COUNT,
                    &AtomicBool::new(false),
                )
                .expect("production H.264 encoder writes the turntable");
            }
            let dark_bytes = std::fs::read(&dark_mp4).expect("Dark MP4 exists");
            let light_bytes = std::fs::read(&light_mp4).expect("Light MP4 exists");
            let dark_samples = mp4_sample_count(&dark_bytes).expect("Dark MP4 sample table");
            let light_samples = mp4_sample_count(&light_bytes).expect("Light MP4 sample table");
            assert_eq!(dark_samples, FRAME_COUNT as u32);
            assert_eq!(light_samples, FRAME_COUNT as u32);
            let dark_video = mp4_media_data(&dark_bytes).expect("Dark MP4 media samples");
            let light_video = mp4_media_data(&light_bytes).expect("Light MP4 media samples");
            assert_eq!(dark_video, light_video);
            serde_json::json!({
                "passed": true,
                "backend": if ffmpeg_is_available() { "FFmpeg H.264" } else { "Blender H.264 fallback" },
                "sample_count": dark_samples,
                "encoded_dimensions": SIZE,
                "fps": FPS,
                "theme_video_payloads_identical": true,
                "video_payload_bytes": dark_video.len(),
                "physical_hardware_tested": false,
            })
        } else {
            serde_json::json!({
                "passed": false,
                "skipped": "FFmpeg and Blender H.264 encoders are unavailable",
            })
        };

        let mut contact_sheet = RgbaImage::new(SIZE[0] * 4, SIZE[1] * 2);
        for (index, frame) in frames.iter().enumerate() {
            let x = (index % 4) as i64 * i64::from(SIZE[0]);
            let y = (index / 4) as i64 * i64::from(SIZE[1]);
            image::imageops::replace(&mut contact_sheet, frame.buffer(), x, y);
        }
        contact_sheet
            .save(output.join("turntable-contact.png"))
            .unwrap();
        std::fs::write(
            output.join("turntable-report.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed": true,
                "adapter": adapter.name,
                "backend": format!("{:?}", adapter.backend),
                "checks": [
                    "all turntable frames render through the production offscreen PNG renderer",
                    "production GIF encoder emits all requested frames at the requested dimensions",
                    "GIF playback decoding preserves visible camera motion between opposing views",
                    "production turntable GIF decoded frames are identical across Dark and Light UI themes",
                    "production H.264 MP4 samples are identical across Dark and Light UI themes",
                ],
                "mp4": mp4_theme_check,
                "frame_count": frames.len(),
                "size": SIZE,
                "fps": FPS,
                "frame_delay_ms": frame_delay_ms,
                "changed_pixels_first_to_half_turn": changed_pixels,
                "physical_hardware_tested": false,
                "artifacts": ["turntable.gif", "turntable-contact.png", "turntable-dark.mp4", "turntable-light.mp4"],
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn production_renderer_reconstructed_reveal_gif_decodes_in_visible_stroke_order() {
        use image::AnimationDecoder;

        const SIZE: [u32; 2] = [320, 240];
        const FRAME_COUNT: usize = 4;
        const FPS: u32 = 4;
        let Some(state) = headless_render_state_for_test() else {
            return;
        };
        let adapter = state.adapter.get_info();
        let mut renderer = crate::renderer::SceneRenderer::new(state);
        let lighting = crate::lighting::Lighting {
            ground: false,
            shadows: false,
            ..Default::default()
        };
        let mut source = crate::export_check::brush_fixture();
        let mut hidden_layer = source.layers[0].clone();
        hidden_layer.id = 2;
        hidden_layer.name = "Hidden".into();
        hidden_layer.visible = false;
        source.layers.push(hidden_layer);
        source.strokes.push(reveal_stroke(99, 2));
        let visible_strokes = visible_reveal_strokes(&source)
            .expect("visible source strokes are available")
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(visible_strokes.len(), source.strokes.len() - 1);
        assert!(
            visible_strokes.iter().all(|stroke| stroke.layer_id != 2),
            "hidden-layer strokes are excluded"
        );

        let mut document = source.clone();
        document.strokes.clear();
        let mut revealed = 0;
        let camera = crate::geometry::Camera {
            projection: crate::geometry::Projection::Orthographic,
            distance: 7.0,
            pitch: 0.38,
            ..Default::default()
        };
        let background = [0.08, 0.12, 0.2];
        let mut rendered = Vec::with_capacity(FRAME_COUNT);
        let mut prefix_counts = Vec::with_capacity(FRAME_COUNT);
        for frame_index in 0..FRAME_COUNT {
            append_reveal_prefix(
                &mut document,
                &visible_strokes,
                &mut revealed,
                frame_index,
                FRAME_COUNT,
            );
            prefix_counts.push(document.strokes.len());
            rendered.push(
                renderer
                    .render_png_pixels(&document, &camera, SIZE, background, lighting)
                    .expect("reconstructed reveal frame renders through production renderer"),
            );
        }
        assert_eq!(prefix_counts, [2, 3, 5, visible_strokes.len()]);

        let output = std::env::current_dir()
            .unwrap()
            .join("verification/construction-reveal-check");
        std::fs::create_dir_all(&output).unwrap();
        let gif_path = output.join("reconstructed-reveal.gif");
        let (sender, receiver) = mpsc::channel();
        for frame in &rendered {
            sender.send(Some(frame.clone())).unwrap();
        }
        encode_gif(
            receiver,
            &gif_path,
            SIZE,
            FPS,
            FRAME_COUNT,
            &AtomicBool::new(false),
        )
        .expect("production GIF encoder writes reconstructed reveal frames");

        let frames = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&gif_path).unwrap(),
        ))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
        assert_eq!(frames.len(), FRAME_COUNT, "playback retains every frame");
        assert!(frames.iter().all(|frame| {
            frame.buffer().width() == SIZE[0] && frame.buffer().height() == SIZE[1]
        }));
        let (numerator, denominator) = frames[0].delay().numer_denom_ms();
        assert!((numerator as f64 / denominator as f64 - 1000.0 / FPS as f64).abs() <= 5.0);
        let frame_changed_pixels = |left: usize, right: usize| {
            frames[left]
                .buffer()
                .pixels()
                .zip(frames[right].buffer().pixels())
                .filter(|(a, b)| a.0 != b.0)
                .count()
        };
        let first_to_second_changed = frame_changed_pixels(0, 1);
        let second_to_third_changed = frame_changed_pixels(1, 2);
        assert!(first_to_second_changed > 100);
        assert!(second_to_third_changed > 100);
        let third_to_fourth_changed = frame_changed_pixels(2, 3);
        assert!(
            third_to_fourth_changed > 100,
            "final prefix completes the reveal"
        );
        let full_scene_pixels = renderer
            .render_png_pixels(&source, &camera, SIZE, background, lighting)
            .expect("complete visible source renders through production renderer");
        assert_eq!(
            rendered.last(),
            Some(&full_scene_pixels),
            "final reveal frame matches the complete visible source"
        );

        let mut contact_sheet = RgbaImage::new(SIZE[0] * 2, SIZE[1] * 2);
        for (index, frame) in frames.iter().enumerate() {
            let x = (index % 2) as i64 * i64::from(SIZE[0]);
            let y = (index / 2) as i64 * i64::from(SIZE[1]);
            image::imageops::replace(&mut contact_sheet, frame.buffer(), x, y);
        }
        contact_sheet
            .save(output.join("reconstructed-reveal-contact.png"))
            .unwrap();
        std::fs::write(
            output.join("reconstructed-reveal-report.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed": true,
                "adapter": adapter.name,
                "backend": format!("{:?}", adapter.backend),
                "checks": [
                    "each frame renders through the production offscreen renderer with the visible stroke prefix",
                    "hidden-layer strokes are omitted",
                    "GIF decoding preserves four frames, output dimensions, and requested frame delay",
                    "decoded frames change as strokes are revealed and the complete scene remains visible",
                ],
                "frame_count": frames.len(),
                "visible_prefix_counts": prefix_counts,
                "hidden_stroke_excluded": true,
                "size": SIZE,
                "fps": FPS,
                "frame_delay_ms": numerator / denominator,
                "changed_pixels_first_to_second": first_to_second_changed,
                "changed_pixels_second_to_third": second_to_third_changed,
                "changed_pixels_third_to_fourth": third_to_fourth_changed,
                "physical_hardware_tested": false,
                "artifacts": ["reconstructed-reveal.gif", "reconstructed-reveal-contact.png"],
            }))
            .unwrap(),
        )
        .unwrap();
    }

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

                time_ms: 0,
            },
            crate::document::Point {
                position: glam::Vec3::X,
                pressure: 0.6,
                tilt: None,

                time_ms: 0,
            },
            crate::document::Point {
                position: glam::Vec3::Y,
                pressure: 1.0,
                tilt: None,

                time_ms: 0,
            },
        ];
        let stroke = crate::document::Stroke {
            model_surface: None,
            gradient: None,
            curve: None,
            point_thickness: None,
            modifiers: None,
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
        let document = Document::default();
        assert_eq!(
            ui.export_duration_seconds(&[], 10_000, &document),
            Some(2.5)
        );
        assert_eq!(ui.frame_count(&[], 10_000, &document), Some(30));
        ui.history_speed = f32::NAN;
        assert_eq!(ui.export_duration_seconds(&[], 10_000, &document), None);
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
        sender.send(Some([255, 0, 0, 255].repeat(4))).unwrap();
        sender.send(Some([0, 255, 0, 255].repeat(4))).unwrap();
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
        sender.send(Some([0, 0, 255, 255].repeat(4))).unwrap();
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

    #[test]
    fn single_play_gif_preview_omits_the_infinite_loop_extension() {
        use image::AnimationDecoder;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("single-play.gif");
        let (sender, receiver) = mpsc::channel();
        sender.send(Some([20, 40, 60, 255].repeat(4))).unwrap();
        encode_gif_once(
            receiver,
            &destination,
            [2, 2],
            10,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();

        let bytes = std::fs::read(&destination).unwrap();
        assert!(!bytes
            .windows(b"NETSCAPE2.0".len())
            .any(|window| window == b"NETSCAPE2.0"));
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&destination).unwrap(),
        ))
        .unwrap();
        assert_eq!(decoder.into_frames().collect_frames().unwrap().len(), 1);
    }

    fn reveal_stroke(id: u64, layer_id: u64) -> crate::document::Stroke {
        crate::document::Stroke {
            model_surface: None,
            gradient: None,
            curve: None,
            point_thickness: None,
            modifiers: None,
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

                time_ms: 0,
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
