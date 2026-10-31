//! A local, workspace-level reference image board. Imported source files are
//! copied into managed application storage so the board survives restarts.
use super::*;
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

const MAX_REFERENCE_COUNT: usize = 32;
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PIXELS: u64 = 12_000_000;
const MAX_TOTAL_PIXELS: u64 = 24_000_000;

#[derive(Clone, Serialize, Deserialize)]
struct ReferenceRecord {
    id: u64,
    name: String,
    file: String,
}

struct ReferenceImage {
    record: ReferenceRecord,
    pixels: Vec<u8>,
    size: [usize; 2],
    texture: egui::TextureHandle,
}

pub(super) struct ReferenceBoard {
    root: Result<PathBuf, String>,
    images: Vec<ReferenceImage>,
    selected: Option<u64>,
    notice: Option<String>,
    zoom: f32,
    pan: Vec2,
    pan_origin: Vec2,
    pan_start: Option<Pos2>,
}

impl ReferenceBoard {
    pub fn new(ctx: &egui::Context) -> Self {
        let root = if std::env::args().any(|arg| arg.starts_with("--verify")) {
            std::env::current_dir()
                .map(|path| path.join("verification/reference-board"))
                .map_err(|error| error.to_string())
        } else {
            crate::presets::path().map(|path| path.with_file_name("reference-board"))
        };
        let mut board = Self {
            root,
            images: Vec::new(),
            selected: None,
            notice: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            pan_origin: Vec2::ZERO,
            pan_start: None,
        };
        board.load(ctx);
        board
    }

    fn index_path(&self) -> Result<PathBuf, String> {
        Ok(self.root.as_ref().map_err(Clone::clone)?.join("index.json"))
    }

    fn load(&mut self, ctx: &egui::Context) {
        let index = match self.index_path() {
            Ok(path) => path,
            Err(error) => {
                self.notice = Some(format!("Reference board unavailable: {error}"));
                return;
            }
        };
        let bytes = match std::fs::read(&index) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.notice = Some(format!("Cannot read reference board: {error}"));
                return;
            }
        };
        if bytes.len() > 64 * 1024 {
            self.notice = Some("Reference board index exceeds the size limit.".into());
            return;
        }
        let records: Vec<ReferenceRecord> =
            match serde_json::from_slice::<Vec<ReferenceRecord>>(&bytes) {
                Ok(records) if records.len() <= MAX_REFERENCE_COUNT => records,
                Ok(_) => {
                    self.notice = Some(format!(
                        "A reference board can contain up to {MAX_REFERENCE_COUNT} images."
                    ));
                    return;
                }
                Err(error) => {
                    self.notice = Some(format!("Cannot read reference board index: {error}"));
                    return;
                }
            };
        let root = match self.root.as_ref() {
            Ok(root) => root,
            Err(_) => return,
        };
        let mut skipped = 0;
        let mut total_bytes = 0_u64;
        let mut total_pixels = 0_u64;
        for record in records {
            if !valid_record(&record) {
                skipped += 1;
                continue;
            }
            let path = root.join(&record.file);
            let file_bytes = std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(u64::MAX);
            if total_bytes.saturating_add(file_bytes) > MAX_TOTAL_BYTES {
                skipped += 1;
                continue;
            }
            match decode(&path) {
                Ok((pixels, size)) => {
                    let pixel_count = (size[0] as u64).saturating_mul(size[1] as u64);
                    if total_pixels.saturating_add(pixel_count) > MAX_TOTAL_PIXELS {
                        skipped += 1;
                        continue;
                    }
                    total_bytes += file_bytes;
                    total_pixels += pixel_count;
                    self.images.push(make_image(ctx, record, pixels, size));
                }
                Err(_) => skipped += 1,
            }
        }
        self.selected = self.images.first().map(|image| image.record.id);
        if skipped > 0 {
            self.notice = Some(format!("{skipped} missing or unreadable reference image(s) were skipped. Artwork files are unaffected."));
        }
    }

    fn persist(&self) -> Result<(), String> {
        let path = self.index_path()?;
        let root = path.parent().ok_or("Invalid reference storage location")?;
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
        let records: Vec<_> = self
            .images
            .iter()
            .map(|image| image.record.clone())
            .collect();
        let bytes = serde_json::to_vec_pretty(&records).map_err(|error| error.to_string())?;
        if bytes.len() > 64 * 1024 {
            return Err("Reference board index exceeds the size limit".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(root).map_err(|error| error.to_string())?;
        temp.write_all(&bytes).map_err(|error| error.to_string())?;
        temp.as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        temp.persist(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn import(&mut self, ctx: &egui::Context) {
        if self.images.len() >= MAX_REFERENCE_COUNT {
            self.notice = Some(format!(
                "A reference board can contain up to {MAX_REFERENCE_COUNT} images."
            ));
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Reference images", &["png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };
        let result = (|| -> Result<ReferenceImage, String> {
            let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
            if metadata.len() > MAX_SOURCE_BYTES {
                return Err("Image file is larger than the 32 MiB import limit.".into());
            }
            let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
            if bytes.len() as u64 > MAX_SOURCE_BYTES {
                return Err("Image file grew beyond the 32 MiB import limit.".into());
            }
            let format = match path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("png") => image::ImageFormat::Png,
                Some("jpg" | "jpeg") => image::ImageFormat::Jpeg,
                _ => return Err("Choose a PNG or JPEG image.".into()),
            };
            let rgba = decode_bytes(&bytes, format)
                .map_err(|error| format!("Cannot decode image: {error}"))?;
            let size = [rgba.width() as usize, rgba.height() as usize];
            let current_pixels: u64 = self
                .images
                .iter()
                .map(|image| (image.size[0] as u64) * (image.size[1] as u64))
                .sum();
            if current_pixels.saturating_add((size[0] as u64) * (size[1] as u64)) > MAX_TOTAL_PIXELS
            {
                return Err(
                    "The reference board's 24 megapixel total memory limit would be exceeded."
                        .into(),
                );
            }
            let root = self.root.as_ref().map_err(Clone::clone)?;
            let current_bytes: u64 = self
                .images
                .iter()
                .map(|image| {
                    root.join(&image.record.file)
                        .metadata()
                        .map(|metadata| metadata.len())
                        .unwrap_or(0)
                })
                .sum();
            if current_bytes.saturating_add(bytes.len() as u64) > MAX_TOTAL_BYTES {
                return Err(
                    "The reference board's 256 MiB managed-storage limit would be exceeded.".into(),
                );
            }
            std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
            let id = self
                .images
                .iter()
                .map(|image| image.record.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("No more reference image IDs available")?;
            let extension = if format == image::ImageFormat::Png {
                "png"
            } else {
                "jpg"
            };
            let file = format!("ref-{id}.{extension}");
            let destination = root.join(&file);
            let mut temp =
                tempfile::NamedTempFile::new_in(root).map_err(|error| error.to_string())?;
            temp.write_all(&bytes).map_err(|error| error.to_string())?;
            temp.as_file()
                .sync_all()
                .map_err(|error| error.to_string())?;
            temp.persist(&destination)
                .map_err(|error| error.to_string())?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Reference image")
                .chars()
                .take(100)
                .collect();
            let record = ReferenceRecord { id, name, file };
            Ok(make_image(ctx, record, rgba.into_raw(), size))
        })();
        match result {
            Ok(image) => {
                self.selected = Some(image.record.id);
                self.images.push(image);
                self.changed("Reference image copied into local managed storage.");
            }
            Err(error) => self.notice = Some(format!("Image import failed: {error}")),
        }
    }

    fn changed(&mut self, success: &str) {
        self.notice = Some(match self.persist() {
            Ok(()) => success.into(),
            Err(error) => format!("Board changed for this session; cannot save it: {error}"),
        });
    }

    pub fn show(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> (Option<[f32; 3]>, bool) {
        let mut sampled = None;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.strong("REFERENCE BOARD");
            if ui.button("Import…").clicked() {
                self.import(ctx);
            }
            close = ui.button("Done").clicked();
        });
        ui.label("Local workspace references. Pan by dragging; wheel or buttons zoom. Click the image to sample its color into the brush. Esc or Done returns to Properties.");
        ui.horizontal(|ui| {
            if ui.button("−").clicked() {
                self.zoom = (self.zoom / 1.25).clamp(0.1, 8.0);
            }
            ui.label(format!("{:.0}%", self.zoom * 100.0));
            if ui.button("+").clicked() {
                self.zoom = (self.zoom * 1.25).clamp(0.1, 8.0);
            }
            if ui.button("Fit").clicked() {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
        });
        if self.images.is_empty() {
            ui.add_space(12.0);
            ui.label("Import a PNG or JPEG to keep it beside your sketch.");
        } else {
            let mut selected = self.selected;
            egui::ComboBox::from_id_salt("reference-image-list")
                .selected_text(
                    self.images
                        .iter()
                        .find(|image| Some(image.record.id) == self.selected)
                        .map(|image| image.record.name.as_str())
                        .unwrap_or("Choose an image"),
                )
                .show_ui(ui, |ui| {
                    for image in &self.images {
                        ui.selectable_value(
                            &mut selected,
                            Some(image.record.id),
                            &image.record.name,
                        );
                    }
                });
            if selected != self.selected {
                self.selected = selected;
                self.pan = Vec2::ZERO;
                self.zoom = 1.0;
            }
            let mut move_up = false;
            let mut move_down = false;
            let mut remove = false;
            ui.horizontal(|ui| {
                move_up = ui
                    .add_enabled(
                        self.selected_index().is_some_and(|index| index > 0),
                        egui::Button::new("Move up"),
                    )
                    .clicked();
                move_down = ui
                    .add_enabled(
                        self.selected_index()
                            .is_some_and(|index| index + 1 < self.images.len()),
                        egui::Button::new("Move down"),
                    )
                    .clicked();
                remove = ui.button("Remove").clicked();
            });
            if move_up || move_down {
                if let Some(index) = self.selected_index() {
                    let target = if move_up { index - 1 } else { index + 1 };
                    self.images.swap(index, target);
                    self.changed("Reference order saved.");
                }
            }
            if remove {
                self.remove_selected();
            }

            let available = ui.available_size_before_wrap();
            let viewer_size = Vec2::new(available.x.max(140.0), available.y.max(180.0))
                .min(Vec2::new(1000.0, 1000.0));
            let (viewer, response) =
                ui.allocate_exact_size(viewer_size, egui::Sense::click_and_drag());
            ui.painter()
                .rect_filled(viewer, 3.0, Color32::from_rgb(24, 29, 36));
            if let Some(index) = self.selected_index() {
                let image = &self.images[index];
                let base_scale = (viewer.width() / image.size[0] as f32)
                    .min(viewer.height() / image.size[1] as f32);
                let size =
                    Vec2::new(image.size[0] as f32, image.size[1] as f32) * base_scale * self.zoom;
                let center = viewer.center() + self.pan;
                let image_rect = Rect::from_center_size(center, size);
                ui.painter().image(
                    image.texture.id(),
                    image_rect,
                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if response.drag_started() {
                    self.pan_start = response.interact_pointer_pos();
                    self.pan_origin = self.pan;
                }
                if response.dragged() {
                    if let (Some(start), Some(now)) =
                        (self.pan_start, response.interact_pointer_pos())
                    {
                        self.pan = self.pan_origin + (now - start);
                    }
                } else if response.drag_stopped() {
                    self.pan_start = None;
                }
                if response.hovered() {
                    let scroll = ui.input(|input| input.raw_scroll_delta.y);
                    if scroll != 0.0 {
                        self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.1, 8.0);
                    }
                    if response.clicked() {
                        if let Some(pos) = response
                            .interact_pointer_pos()
                            .filter(|pos| image_rect.contains(*pos))
                        {
                            let uv = (pos - image_rect.min) / image_rect.size();
                            let x = ((uv.x.clamp(0.0, 0.999999) * image.size[0] as f32) as usize)
                                .min(image.size[0] - 1);
                            let y = ((uv.y.clamp(0.0, 0.999999) * image.size[1] as f32) as usize)
                                .min(image.size[1] - 1);
                            let offset = (y * image.size[0] + x) * 4;
                            sampled = Some([
                                crate::presentation::srgb_to_linear(
                                    image.pixels[offset] as f32 / 255.0,
                                ),
                                crate::presentation::srgb_to_linear(
                                    image.pixels[offset + 1] as f32 / 255.0,
                                ),
                                crate::presentation::srgb_to_linear(
                                    image.pixels[offset + 2] as f32 / 255.0,
                                ),
                            ]);
                        }
                    }
                }
            }
        }
        if let Some(notice) = &self.notice {
            ui.small(notice);
        }
        close |= ctx.input(|input| {
            input.key_pressed(Key::Escape) || (input.modifiers.command && input.key_pressed(Key::W))
        });
        (sampled, close)
    }

    fn selected_index(&self) -> Option<usize> {
        self.images
            .iter()
            .position(|image| Some(image.record.id) == self.selected)
    }

    fn remove_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let old_selection = self.selected;
        let old_pan = self.pan;
        let old_zoom = self.zoom;
        let image = self.images.remove(index);
        self.selected = self
            .images
            .get(index.min(self.images.len().saturating_sub(1)))
            .map(|image| image.record.id);
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        match self.persist() {
            Ok(()) => {
                if let Ok(root) = &self.root {
                    let _ = std::fs::remove_file(root.join(&image.record.file));
                }
                self.notice = Some("Reference removed.".into());
            }
            Err(error) => {
                self.images.insert(index, image);
                self.selected = old_selection;
                self.pan = old_pan;
                self.zoom = old_zoom;
                self.notice = Some(format!("Cannot save reference removal: {error}"));
            }
        }
    }
}

fn valid_record(record: &ReferenceRecord) -> bool {
    let expected_png = format!("ref-{}.png", record.id);
    let expected_jpg = format!("ref-{}.jpg", record.id);
    !record.name.is_empty()
        && record.name.len() <= 400
        && (record.file == expected_png || record.file == expected_jpg)
}

fn decode(path: &Path) -> Result<(Vec<u8>, [usize; 2]), String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err("Reference exceeds the file-size limit".into());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let format = match path.extension().and_then(|extension| extension.to_str()) {
        Some("png") => image::ImageFormat::Png,
        Some("jpg") => image::ImageFormat::Jpeg,
        _ => return Err("Unsupported stored image type".into()),
    };
    let rgba = decode_bytes(&bytes, format)?;
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok((rgba.into_raw(), size))
}

fn decode_bytes(bytes: &[u8], format: image::ImageFormat) -> Result<image::RgbaImage, String> {
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|error| error.to_string())?;
    if width == 0 || height == 0 || (width as u64).saturating_mul(height as u64) > MAX_PIXELS {
        return Err("Image dimensions exceed the 12 megapixel limit".into());
    }
    image::load_from_memory_with_format(bytes, format)
        .map(|decoded| decoded.to_rgba8())
        .map_err(|error| error.to_string())
}

fn make_image(
    ctx: &egui::Context,
    record: ReferenceRecord,
    pixels: Vec<u8>,
    size: [usize; 2],
) -> ReferenceImage {
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    let texture = ctx.load_texture(
        format!("reference-{}", record.id),
        color,
        egui::TextureOptions::LINEAR,
    );
    ReferenceImage {
        record,
        pixels,
        size,
        texture,
    }
}
