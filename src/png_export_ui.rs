use super::*;
use image::ImageEncoder;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};

const MAX_PIXELS: u64 = 8_388_608;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StillFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum JpegBackground {
    Canvas,
    White,
    Black,
}

fn jpeg_background_color(background: JpegBackground, canvas: [f32; 3]) -> [f32; 3] {
    match background {
        JpegBackground::Canvas => canvas,
        JpegBackground::White => [1.0; 3],
        JpegBackground::Black => [0.0; 3],
    }
}

struct PngJob {
    receiver: Receiver<Result<(), String>>,
    destination: PathBuf,
}

pub(super) struct PngExportUi {
    dialog_open: bool,
    format: StillFormat,
    jpeg_background: JpegBackground,
    transparent_png: bool,
    width: u32,
    height: u32,
    overwrite: Option<(PathBuf, [u32; 2], StillFormat)>,
    request: Option<(PathBuf, [u32; 2], bool, StillFormat, JpegBackground, bool)>,
    job: Option<PngJob>,
}

impl Default for PngExportUi {
    fn default() -> Self {
        Self {
            dialog_open: false,
            format: StillFormat::Png,
            jpeg_background: JpegBackground::Canvas,
            transparent_png: false,
            width: 1920,
            height: 1080,
            overwrite: None,
            request: None,
            job: None,
        }
    }
}

impl PngExportUi {
    pub fn can_export(&self) -> bool {
        self.job.is_none() && self.request.is_none() && self.overwrite.is_none()
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn begin(&mut self) {
        if self.can_export() {
            self.dialog_open = true;
        }
    }

    fn dimensions_valid(&self) -> bool {
        self.width >= 1
            && self.height >= 1
            && self.width <= 8192
            && self.height <= 8192
            && (self.width as u64) * (self.height as u64) <= MAX_PIXELS
    }

    fn choose_destination(&mut self) {
        let (extension, filter) = match self.format {
            StillFormat::Png => ("png", "PNG image"),
            StillFormat::Jpeg => ("jpg", "JPEG image"),
        };
        let filename = format!("artwork-{}x{}.{}", self.width, self.height, extension);
        let Some(mut path) = rfd::FileDialog::new()
            .add_filter(filter, &[extension])
            .set_file_name(filename)
            .save_file()
        else {
            return;
        };
        path.set_extension(extension);
        let size = [self.width, self.height];
        self.dialog_open = false;
        if path.exists() {
            self.overwrite = Some((path, size, self.format));
        } else {
            self.request = Some((
                path,
                size,
                false,
                self.format,
                self.jpeg_background,
                self.transparent_png,
            ));
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if self.dialog_open {
            let mut cancel = false;
            let mut choose = false;
            egui::Modal::new(egui::Id::new("png-export-dimensions")).show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.heading("Export artwork image");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.format, StillFormat::Png, "PNG");
                    ui.selectable_value(&mut self.format, StillFormat::Jpeg, "JPEG");
                });
                if self.format == StillFormat::Jpeg {
                    ui.horizontal(|ui| {
                        ui.label("JPEG background");
                        egui::ComboBox::from_id_salt("jpeg-background")
                            .selected_text(match self.jpeg_background {
                                JpegBackground::Canvas => "Canvas color",
                                JpegBackground::White => "White",
                                JpegBackground::Black => "Black",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.jpeg_background, JpegBackground::Canvas, "Canvas color");
                                ui.selectable_value(&mut self.jpeg_background, JpegBackground::White, "White");
                                ui.selectable_value(&mut self.jpeg_background, JpegBackground::Black, "Black");
                            });
                    });
                } else {
                    ui.checkbox(&mut self.transparent_png, "Transparent background");
                    ui.small("Keeps stroke lighting and shadows; excludes ground and image planes.");
                }
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.width).range(1..=8192).prefix("Width "));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut self.height).range(1..=8192).prefix("Height "));
                });
                ui.label(format!("{} pixels", self.width as u64 * self.height as u64));
                ui.small("Keeps the current camera target and vertical framing; output aspect sets horizontal framing.");
                if !self.dimensions_valid() {
                    ui.colored_label(ui_style::colors(ui.ctx()).error, "Use 1–8192 pixels per side and at most 8 megapixels.");
                }
                ui.horizontal(|ui| {
                    choose = ui
                        .add_enabled(
                            self.dimensions_valid(),
                            egui::Button::new(match self.format {
                                StillFormat::Png => "Choose PNG file…",
                                StillFormat::Jpeg => "Choose JPEG file…",
                            }),
                        )
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if choose {
                self.choose_destination();
            }
            if cancel {
                self.dialog_open = false;
            }
        }
        if let Some((path, size, format)) = self.overwrite.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("png-export-overwrite")).show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.heading("Replace existing image?");
                ui.label(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("image file"),
                );
                ui.horizontal(|ui| {
                    confirm = ui.button("Overwrite").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if confirm {
                self.overwrite = None;
                self.request = Some((
                    path,
                    size,
                    true,
                    format,
                    self.jpeg_background,
                    self.transparent_png,
                ));
            } else if cancel {
                self.overwrite = None;
            }
        }
    }

    pub fn take_request(
        &mut self,
    ) -> Option<(PathBuf, [u32; 2], bool, StillFormat, JpegBackground, bool)> {
        self.request.take()
    }

    pub fn start_job(
        &mut self,
        destination: PathBuf,
        size: [u32; 2],
        allow_overwrite: bool,
        format: StillFormat,
        pixels: Vec<u8>,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if !self.can_export() {
            return Err("Another PNG export is already running".into());
        }
        if (size[0] as u64) * (size[1] as u64) > MAX_PIXELS
            || pixels.len() != size[0] as usize * size[1] as usize * 4
        {
            return Err("PNG pixel data does not match the validated dimensions".into());
        }
        if destination.exists() && !allow_overwrite {
            return Err(
                "Destination appeared after overwrite confirmation; choose it again to confirm."
                    .into(),
            );
        }
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        let path = destination.clone();
        std::thread::Builder::new()
            .name("png-export".into())
            .spawn(move || {
                let result = match format {
                    StillFormat::Png => write_png_atomic(&path, size, &pixels),
                    StillFormat::Jpeg => write_jpeg_atomic(&path, size, &pixels),
                };
                let _ = sender.send(result);
                context.request_repaint();
            })
            .map_err(|error| error.to_string())?;
        self.job = Some(PngJob {
            receiver,
            destination,
        });
        Ok(())
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Option<String> {
        let job = self.job.as_ref()?;
        match job.receiver.try_recv() {
            Ok(result) => {
                let destination = job.destination.clone();
                self.job = None;
                Some(match result {
                    Ok(()) => format!("Exported image to {}", destination.display()),
                    Err(error) => format!("Image export failed: {error}"),
                })
            }
            Err(TryRecvError::Disconnected) => {
                let destination = job.destination.clone();
                self.job = None;
                Some(format!(
                    "Image export worker stopped for {}",
                    destination.display()
                ))
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                None
            }
        }
    }
}

impl SketchApp {
    pub(super) fn can_export_png(&self) -> bool {
        self.png_export.can_export()
            && !self.video_export.is_running()
            && self.active.is_none()
            && self.drag.is_none()
            && self.repeat.is_none()
            && self.fills.draft.is_none()
            && self.line.draft.is_none()
            && !self.confirm
            && !self.recovery_prompt
            && self
                .doc
                .strokes
                .iter()
                .any(|stroke| self.doc.visible(stroke) && !stroke.points.is_empty())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_png_export(
        &mut self,
        destination: PathBuf,
        size: [u32; 2],
        allow_overwrite: bool,
        format: StillFormat,
        jpeg_background: JpegBackground,
        transparent_png: bool,
        ctx: &egui::Context,
    ) {
        if !self.can_export_png() {
            self.status =
                "Finish editing and make sure a visible stroke is available before exporting."
                    .into();
            return;
        }
        if self
            .file
            .as_ref()
            .is_some_and(|source| crate::export::same_destination(source, &destination))
        {
            self.status =
                "Choose an export destination different from the open native drawing.".into();
            return;
        }
        self.status = "Rendering artwork image…".into();
        let camera = self.camera;
        let background = if format == StillFormat::Jpeg {
            jpeg_background_color(jpeg_background, self.presentation.preferences.background)
        } else {
            self.presentation.preferences.background
        };
        let lighting = self.renderer.lighting;
        let rendered = if format == StillFormat::Png && transparent_png {
            self.renderer
                .render_transparent_png_pixels(&self.doc, &camera, size, lighting)
        } else {
            self.renderer
                .render_png_pixels(&self.doc, &camera, size, background, lighting)
        };
        let pixels = match rendered {
            Ok(pixels) => pixels,
            Err(error) => {
                self.status = format!("PNG render failed: {error}");
                return;
            }
        };
        if let Err(error) =
            self.png_export
                .start_job(destination, size, allow_overwrite, format, pixels, ctx)
        {
            self.status = format!("Could not start PNG export: {error}");
        }
    }

    pub(super) fn verify_png_export(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.png_verification_frame else {
            return;
        };
        if frame > 0 {
            return;
        }
        append_storefront_details(&mut self.doc);
        let doc_before = serde_json::to_value(&self.doc).unwrap();
        let camera_before = format!("{:?}", self.camera);
        let undo_before = self.undo.len();
        let redo_before = self.redo.len();
        let selected_before = self.selected.clone();
        let dirty_before = self.dirty;
        let file_before = self.file.clone();
        let background = self.presentation.preferences.background;
        let lighting = self.renderer.lighting;
        let sizes = [[1024, 768], [640, 480]];
        let output_dir = std::env::current_dir().unwrap().join("verification");
        std::fs::create_dir_all(&output_dir).expect("create PNG verification directory");
        let mut dimensions = Vec::new();
        for (index, size) in sizes.into_iter().enumerate() {
            let pixels = self
                .renderer
                .render_png_pixels(&self.doc, &self.camera, size, background, lighting)
                .expect("offscreen PNG render");
            let path = output_dir.join(format!("artwork-{}x{}.png", size[0], size[1]));
            image::save_buffer(&path, &pixels, size[0], size[1], image::ColorType::Rgba8)
                .expect("save verification PNG");
            let image = image::open(&path)
                .expect("reopen verification PNG")
                .to_rgba8();
            assert_eq!(image.dimensions(), (size[0], size[1]));
            let first = image.get_pixel(0, 0);
            let varied = image.pixels().filter(|pixel| **pixel != *first).count();
            assert!(varied > 100, "offscreen PNG should contain visible artwork");
            if index == 0 {
                let overwrite_path = output_dir.join("atomic-overwrite-check.png");
                std::fs::write(&overwrite_path, b"previous destination").unwrap();
                write_png_atomic(&overwrite_path, size, &pixels)
                    .expect("atomically replace existing PNG");
                assert_eq!(
                    image::open(overwrite_path).unwrap().to_rgba8().dimensions(),
                    (size[0], size[1])
                );
            }
            dimensions.push(size);
        }
        assert_eq!(serde_json::to_value(&self.doc).unwrap(), doc_before);
        assert_eq!(format!("{:?}", self.camera), camera_before);
        assert_eq!(self.undo.len(), undo_before);
        assert_eq!(self.redo.len(), redo_before);
        assert_eq!(self.selected, selected_before);
        assert_eq!(self.dirty, dirty_before);
        assert_eq!(self.file, file_before);
        assert_eq!(self.presentation.preferences.background, background);
        assert_eq!(self.renderer.lighting, lighting);
        let report = serde_json::json!({
            "passed": true,
                "checks": ["offscreen PNG dimensions and decoding at two sizes", "dense storefront fixture pixels", "atomic replacement of an existing PNG", "document, camera, selection, file identity, dirty state, and history unchanged"],
            "sizes": dimensions,
            "strokes": self.doc.strokes.len(),
            "gpu": self.renderer.adapter
        });
        std::fs::write(
            output_dir.join("png-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        self.status =
            "Offscreen PNG export verified at two sizes; artwork state is unchanged.".into();
        self.png_verification_frame = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn append_storefront_details(doc: &mut Document) {
    fn rectangle(doc: &mut Document, x0: f32, y0: f32, x1: f32, y1: f32, z: f32, color: [f32; 3]) {
        let id = doc
            .strokes
            .iter()
            .map(|stroke| stroke.id)
            .max()
            .unwrap_or(0)
            + 1;
        let corners = [
            glam::Vec3::new(x0, y0, z),
            glam::Vec3::new(x1, y0, z),
            glam::Vec3::new(x1, y1, z),
            glam::Vec3::new(x0, y1, z),
        ];
        let points: Vec<_> = corners
            .iter()
            .map(|&position| Point {
                position,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .collect();
        let tangent = (corners[1] - corners[0]).normalize();
        let brush = Brush {
            normal: glam::Vec3::Z,
            tangent,
            ..Default::default()
        };
        doc.strokes.push(Stroke {
            model_surface: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: Some(crate::document::PlanarFill {
                offset: 0.004,
                gradient: None,
                texture: None,
                holes: Vec::new(),
            }),
            group_id: None,
            layer_id: doc.active_layer,
            id,
            color,
            gradient: None,
            radius: 0.012,
            brush,
            points: points.clone(),
        });
        let mut outline = points;
        outline.push(outline[0]);
        doc.strokes.push(Stroke {
            model_surface: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: doc.active_layer,
            id: id + 1,
            color: [0.12, 0.17, 0.20],
            gradient: None,
            radius: 0.009,
            brush,
            points: outline,
        });
    }

    // A projecting striped canopy, shop sign, and small display goods on the facade.
    let stripe_colors = [
        [0.78, 0.20, 0.16],
        [0.94, 0.86, 0.67],
        [0.16, 0.42, 0.48],
        [0.94, 0.86, 0.67],
        [0.78, 0.20, 0.16],
        [0.94, 0.86, 0.67],
    ];
    for (index, color) in stripe_colors.into_iter().enumerate() {
        let x0 = -0.93 + index as f32 * 0.31;
        rectangle(doc, x0, 1.70, x0 + 0.31, 1.96, 1.10, color);
    }
    rectangle(doc, -0.50, 1.99, 0.50, 2.20, 1.10, [0.16, 0.28, 0.31]);
    rectangle(doc, -0.38, 2.04, 0.38, 2.15, 1.11, [0.93, 0.83, 0.60]);
    for (index, color) in [
        [0.83, 0.36, 0.20],
        [0.93, 0.71, 0.24],
        [0.22, 0.52, 0.38],
        [0.39, 0.55, 0.72],
        [0.76, 0.39, 0.55],
    ]
    .into_iter()
    .enumerate()
    {
        let x0 = -0.78 + index as f32 * 0.30;
        rectangle(doc, x0, 1.08, x0 + 0.18, 1.27, 1.105, color);
    }
}

pub(crate) fn write_png_atomic(path: &Path, size: [u32; 2], pixels: &[u8]) -> Result<(), String> {
    if size.contains(&0)
        || size[0] > 8192
        || size[1] > 8192
        || size[0] as u64 * size[1] as u64 > MAX_PIXELS
        || pixels.len() != size[0] as usize * size[1] as usize * 4
    {
        return Err("Invalid PNG dimensions or pixel data".into());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    image::codecs::png::PngEncoder::new(&mut temp)
        .write_image(pixels, size[0], size[1], image::ExtendedColorType::Rgba8)
        .map_err(|error| error.to_string())?;
    temp.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temp.persist(path).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn write_jpeg_atomic(path: &Path, size: [u32; 2], pixels: &[u8]) -> Result<(), String> {
    if size.contains(&0)
        || size[0] > 8192
        || size[1] > 8192
        || size[0] as u64 * size[1] as u64 > MAX_PIXELS
        || pixels.len() != size[0] as usize * size[1] as usize * 4
    {
        return Err("Invalid JPEG dimensions or pixel data".into());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    let rgb: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|rgba| [rgba[0], rgba[1], rgba[2]])
        .collect();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut temp, 92)
        .write_image(&rgb, size[0], size[1], image::ExtendedColorType::Rgb8)
        .map_err(|error| error.to_string())?;
    temp.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temp.persist(path).map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_background_choice_resolves_canvas_white_and_black_colors() {
        let canvas = [0.13, 0.28, 0.46];
        assert_eq!(
            jpeg_background_color(JpegBackground::Canvas, canvas),
            canvas
        );
        assert_eq!(
            jpeg_background_color(JpegBackground::White, canvas),
            [1.0; 3]
        );
        assert_eq!(
            jpeg_background_color(JpegBackground::Black, canvas),
            [0.0; 3]
        );
    }

    #[test]
    fn png_and_jpeg_files_decode_and_invalid_frames_preserve_existing_output() {
        let dir = tempfile::tempdir().unwrap();
        let size = [32, 32];
        let pixels = [40, 100, 220, 128].repeat((size[0] * size[1]) as usize);
        let png = dir.path().join("art.png");
        write_png_atomic(&png, size, &pixels).unwrap();
        let decoded_png = image::open(&png).unwrap().to_rgba8();
        assert_eq!(decoded_png.dimensions(), (size[0], size[1]));
        assert_eq!(decoded_png.get_pixel(16, 16).0, [40, 100, 220, 128]);

        let jpeg = dir.path().join("art.jpg");
        write_jpeg_atomic(&jpeg, size, &pixels).unwrap();
        let decoded_jpeg = image::open(&jpeg).unwrap().to_rgb8();
        assert_eq!(decoded_jpeg.dimensions(), (size[0], size[1]));
        let decoded = decoded_jpeg.get_pixel(16, 16).0;
        for (actual, expected) in decoded.into_iter().zip([40_u8, 100, 220]) {
            assert!(actual.abs_diff(expected) <= 8);
        }

        std::fs::write(&png, b"preserve png").unwrap();
        assert!(write_png_atomic(&png, size, &[0; 4]).is_err());
        assert_eq!(std::fs::read(&png).unwrap(), b"preserve png");
        std::fs::write(&jpeg, b"preserve jpeg").unwrap();
        assert!(write_jpeg_atomic(&jpeg, size, &[0; 4]).is_err());
        assert_eq!(std::fs::read(&jpeg).unwrap(), b"preserve jpeg");
    }
}
