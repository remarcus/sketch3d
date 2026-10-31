use super::*;
use image::{codecs::gif::GifEncoder, Delay, Frame, RgbaImage};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc,
    },
};

const MAX_FRAMES: usize = 480;
const MAX_PIXELS: u64 = 921_600;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportMode {
    Turntable,
    Sequence(u64),
}

pub(super) struct VideoRequest {
    pub(super) path: PathBuf,
    size: [u32; 2],
    fps: u32,
    mode: ExportMode,
    replace: bool,
}

struct VideoJob {
    request: VideoRequest,
    document: Document,
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
    mode: ExportMode,
    width: u32,
    height: u32,
    fps: u32,
    duration: f32,
    overwrite: Option<VideoRequest>,
    request: Option<VideoRequest>,
    job: Option<VideoJob>,
}

impl Default for VideoExportUi {
    fn default() -> Self {
        Self {
            dialog_open: false,
            mode: ExportMode::Turntable,
            width: 640,
            height: 360,
            fps: 12,
            duration: 5.0,
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

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn frame_count(&self, sequences: &[crate::camera_bookmarks::CameraSequence]) -> Option<usize> {
        let seconds = match self.mode {
            ExportMode::Turntable => self.duration,
            ExportMode::Sequence(id) => sequences
                .iter()
                .find(|sequence| sequence.id == id)?
                .duration_seconds(),
        };
        let count = (seconds * self.fps as f32).ceil() as usize;
        (seconds.is_finite() && seconds > 0.0 && (2..=MAX_FRAMES).contains(&count)).then_some(count)
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        sequences: &[crate::camera_bookmarks::CameraSequence],
    ) {
        if self.dialog_open {
            let mut open = true;
            let mut choose = false;
            egui::Window::new("Animated GIF export")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.mode, ExportMode::Turntable, "Turntable");
                        for sequence in sequences {
                            ui.selectable_value(
                                &mut self.mode,
                                ExportMode::Sequence(sequence.id),
                                &sequence.name,
                            );
                        }
                    });
                    if self.mode == ExportMode::Turntable {
                        ui.add(
                            egui::Slider::new(&mut self.duration, 1.0..=40.0)
                                .text("Duration (seconds)"),
                        );
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
                    ui.label(format!("{} pixels", self.width as u64 * self.height as u64));
                    if let Some(count) = self.frame_count(sequences) {
                        ui.label(format!(
                            "{count} frames · GIF animation · overlays excluded"
                        ));
                    } else {
                        ui.colored_label(
                            Color32::LIGHT_RED,
                            "Choose a sequence or reduce the duration to fit the 480-frame limit.",
                        );
                    }
                    if u64::from(self.width) * u64::from(self.height) > MAX_PIXELS {
                        ui.colored_label(
                            Color32::LIGHT_RED,
                            "Output is limited to 921,600 pixels per frame.",
                        );
                    }
                    ui.horizontal(|ui| {
                        choose = ui
                            .add_enabled(
                                self.frame_count(sequences).is_some()
                                    && u64::from(self.width) * u64::from(self.height) <= MAX_PIXELS,
                                egui::Button::new("Choose GIF file…"),
                            )
                            .clicked();
                        if ui.button("Cancel").clicked() {
                            self.dialog_open = false;
                        }
                    });
                });
            self.dialog_open &= open;
            if choose {
                if let Some(mut path) = rfd::FileDialog::new()
                    .add_filter("Animated GIF", &["gif"])
                    .set_file_name("animation.gif")
                    .save_file()
                {
                    path.set_extension("gif");
                    let request = VideoRequest {
                        path: path.clone(),
                        size: [self.width, self.height],
                        fps: self.fps,
                        mode: self.mode,
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
            egui::Modal::new(egui::Id::new("gif-export-overwrite")).show(ctx, |ui| {
                ui.heading("Replace existing GIF?");
                ui.label(
                    request
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("GIF file"),
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
            egui::Window::new("Rendering animated GIF")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.add(
                        egui::ProgressBar::new(job.next_frame as f32 / job.frame_count as f32)
                            .text(format!("{} / {} frames", job.next_frame, job.frame_count)),
                    );
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

    pub fn start(
        &mut self,
        request: VideoRequest,
        document: &Document,
        camera: Camera,
        bookmarks: &crate::camera_bookmarks::CameraBookmarkStore,
        background: [f32; 3],
        lighting: crate::lighting::Lighting,
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
            ExportMode::Turntable => self.duration,
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
        let frame_count = (seconds * request.fps as f32).ceil() as usize;
        if request.size.contains(&0)
            || u64::from(request.size[0]) * u64::from(request.size[1]) > MAX_PIXELS
            || !(4..=24).contains(&request.fps)
            || !(2..=MAX_FRAMES).contains(&frame_count)
        {
            return Err("Animated export exceeds its frame, rate, or dimension limits".into());
        }
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
        std::thread::Builder::new()
            .name("gif-export".into())
            .spawn(move || {
                let result = encode_gif(
                    frame_receiver,
                    &output_path,
                    size,
                    fps,
                    frame_count,
                    &worker_cancelled,
                );
                let _ = result_sender.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.job = Some(VideoJob {
            request,
            document: document.clone(),
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
                    return Some("Animated GIF encoder stopped unexpectedly.".into());
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
                Some(format!("Exported animated GIF to {path}"))
            }
            Ok(Err(error)) => {
                self.job = None;
                Some(if error == "Export cancelled" {
                    "Animated GIF export cancelled.".into()
                } else {
                    format!("Animated GIF export failed: {error}")
                })
            }
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                Some("Animated GIF export worker stopped unexpectedly.".into())
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
