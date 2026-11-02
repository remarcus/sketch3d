//! Local preset browsing and editing, with a real stroke-mesh preview.
use crate::{
    document::{Document, Point, Stroke},
    geometry::Camera,
    presets::{BrushPackage, Library, Preset},
    renderer::SceneRenderer,
    surface::GuidePreview,
};
use eframe::egui::WidgetInfo;
use eframe::{egui, egui_wgpu};
use glam::Vec3;
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct PresetUi {
    pub library: Library,
    pub selected: u64,
    pub notice: String,
    pub path: Option<PathBuf>,
    renderer: SceneRenderer,
    name: String,
    repair_needed: bool,
    preview_preset: Option<Preset>,
    preview_doc: Document,
    tip_previews: HashMap<u64, egui::TextureHandle>,
    search: String,
    browser_open: bool,
    focus_search: bool,
}
impl PresetUi {
    pub fn new(state: egui_wgpu::RenderState, path: Result<PathBuf, String>) -> Self {
        let (library, path, repair_needed, notice) = match path {
            Ok(path) => {
                let (library, repair, notice) = load_state(&path);
                (library, Some(path), repair, notice)
            }
            Err(error) => (
                Library::default(),
                None,
                false,
                format!("{error}. Presets are available in memory only."),
            ),
        };
        let selected = library.presets.first().map_or(0, |p| p.id);
        let name = library
            .get(selected)
            .map_or_else(|| "My brush".into(), |p| p.name.clone());
        Self {
            library,
            selected,
            notice,
            path,
            renderer: SceneRenderer::new(state),
            name,
            repair_needed,
            preview_preset: None,
            preview_doc: Document::default(),
            tip_previews: HashMap::new(),
            search: String::new(),
            browser_open: false,
            focus_search: false,
        }
    }

    /// Reload is also the restart/corruption boundary used by software verification.
    pub fn reload(&mut self) {
        let (library, repair, notice) = match self.path.as_deref() {
            Some(path) => load_state(path),
            None => (
                Library::default(),
                false,
                "Local application-data storage is unavailable. Presets are in memory only.".into(),
            ),
        };
        self.library = library;
        self.repair_needed = repair;
        self.notice = notice;
        self.choose(self.library.presets.first().map_or(0, |p| p.id));
        self.preview_preset = None;
        self.renderer.invalidate();
    }
    fn persist(&mut self, action: &str) {
        match &self.path {
            Some(path) => match self.library.save(path) {
                Ok(()) => self.notice = format!("{action}. Saved locally."),
                Err(error) => self.notice = format!("{action} in memory; saving failed: {error}"),
            },
            None => {
                self.notice = format!(
                    "{action} in memory only; local application-data storage is unavailable."
                )
            }
        }
    }
    pub fn choose(&mut self, id: u64) {
        self.selected = id;
        self.name = self
            .library
            .get(id)
            .map_or_else(|| "My brush".into(), |p| p.name.clone());
    }

    pub(super) fn browser_is_open(&self) -> bool {
        self.browser_open
    }
    pub fn repair_library(&mut self) -> Result<(), String> {
        if !self.repair_needed {
            return Ok(());
        }
        let path = self
            .path
            .as_ref()
            .ok_or("Local library path is unavailable")?;
        let backup = backup_original(path)?;
        self.library.save(path)?;
        self.repair_needed = false;
        self.notice = format!(
            "Library repaired. Original preserved at {}",
            backup.display()
        );
        Ok(())
    }

    /// Browsing and editing library entries never applies them to the active brush.
    /// Only the explicit Use preset button returns a resolved value to the app.
    pub fn show(&mut self, ui: &mut egui::Ui, current: Preset) -> Option<Preset> {
        let mut chosen = None;
        egui::CollapsingHeader::new("Brush library")
            .default_open(true)
            .show(ui, |ui| {
                let selected_name = self
                    .library
                    .get(self.selected)
                    .map_or("No presets", |preset| preset.name.as_str());
                let was_open = self.browser_open;
                let trigger = ui.add(
                    egui::Button::new(format!("Browse brushes  ·  {selected_name}"))
                        .selected(self.browser_open),
                );
                let trigger_id = trigger.id;
                if trigger.clicked() {
                    self.browser_open = !self.browser_open;
                    self.focus_search = self.browser_open;
                }
                if self.browser_open {
                    if let Some(preset) = self.show_browser(ui.ctx()) {
                        self.notice =
                            format!("Using {}. Existing marks keep their settings.", preset.name);
                        chosen = Some(preset);
                    }
                }
                if was_open && !self.browser_open {
                    ui.memory_mut(|memory| memory.request_focus(trigger_id));
                }
                if let Some(preset) = self.library.get(self.selected).cloned() {
                    self.preview(ui, &preset);
                    self.tip_preview(ui, &preset);
                    if ui.button("Import PNG tip…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PNG image", &["png"])
                            .pick_file()
                        {
                            match self.import_tip(&path, preset.id) {
                                Ok(file_name) => self.persist(&format!("Imported {file_name}")),
                                Err(error) => {
                                    self.notice = format!("Brush tip import failed: {error}")
                                }
                            }
                        }
                    }
                    let shape = match preset.brush.kind {
                        crate::document::BrushKind::Round => "Round",
                        crate::document::BrushKind::Ribbon => "Ribbon",
                        crate::document::BrushKind::Chisel => "Flat/chisel",
                    };
                    ui.small(format!(
                        "{shape} · {} · radius {:.3}",
                        preset.brush.texture.kind.label(),
                        preset.radius
                    ));
                    ui.small(format!(
                        "Pressure width: {}",
                        if preset.brush.dynamics.width.enabled {
                            "on"
                        } else {
                            "off"
                        }
                    ));
                    if preset.brush.kind == crate::document::BrushKind::Chisel {
                        ui.small(format!(
                            "Thickness {:.0}% · pressure {}",
                            preset.brush.thickness * 100.0,
                            if preset.brush.dynamics.thickness.enabled {
                                "on"
                            } else {
                                "off"
                            }
                        ));
                    }
                    ui.small(format!(
                        "Taper {:.2} / {:.2} · stabilize {:.0}%",
                        preset.brush.dynamics.taper_start,
                        preset.brush.dynamics.taper_end,
                        preset.stabilization * 100.0
                    ));
                    if preset.brush.kind != crate::document::BrushKind::Round {
                        ui.small(format!(
                            "Angle {:.0}° · tilt {}",
                            preset.brush.angle_degrees,
                            if preset.brush.dynamics.tilt_orientation {
                                "on"
                            } else {
                                "off"
                            }
                        ));
                    }
                    if preset.brush.texture.kind != crate::texture::TextureKind::Solid {
                        ui.small(format!(
                            "Pattern period {:.2} · strength {:.0}%",
                            preset.brush.texture.scale,
                            preset.brush.texture.strength * 100.0
                        ));
                    }
                    if preset.brush.dynamics.width.enabled {
                        ui.small(format!(
                            "Width minimum {:.0}% · response {:.2}",
                            preset.brush.dynamics.width.minimum * 100.0,
                            preset.brush.dynamics.width.curve
                        ));
                    }
                    if preset.brush.kind == crate::document::BrushKind::Chisel
                        && preset.brush.dynamics.thickness.enabled
                    {
                        ui.small(format!(
                            "Thickness minimum {:.0}% · response {:.2}",
                            preset.brush.dynamics.thickness.minimum * 100.0,
                            preset.brush.dynamics.thickness.curve
                        ));
                    }
                    if ui.button("Use preset").clicked() {
                        chosen = Some(preset.clone());
                        self.notice =
                            format!("Using {}. Existing marks keep their settings.", preset.name);
                    }
                }
                if self.repair_needed {
                    ui.small(
                        "The saved library needs repair. Original data will be backed up first.",
                    );
                    if ui.button("Repair library (keep backup)").clicked() {
                        if let Err(error) = self.repair_library() {
                            self.notice = format!("Repair failed: {error}");
                        }
                    }
                }
                ui.menu_button("Manage presets", |ui| {
                    ui.label("Preset name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.name)
                            .hint_text("Preset name")
                            .char_limit(64),
                    );
                    ui.add_enabled_ui(!self.repair_needed, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button("Save as new")
                                .on_hover_text(
                                    "Save the current brush controls as a personal preset",
                                )
                                .clicked()
                            {
                                let name = if self
                                    .library
                                    .get(self.selected)
                                    .is_some_and(|p| p.name == self.name)
                                {
                                    unique_name(&self.library, "My brush")
                                } else {
                                    self.name.clone()
                                };
                                match self.library.create(
                                    &name,
                                    current.brush,
                                    current.radius,
                                    current.color,
                                    current.stabilization,
                                ) {
                                    Ok(id) => {
                                        self.choose(id);
                                        self.persist("Preset created");
                                        ui.close_menu();
                                    }
                                    Err(error) => self.notice = error,
                                }
                            }
                            if ui
                                .add_enabled(
                                    self.library.get(self.selected).is_some(),
                                    egui::Button::new("Duplicate"),
                                )
                                .clicked()
                            {
                                let name = if self
                                    .library
                                    .get(self.selected)
                                    .is_some_and(|p| p.name == self.name)
                                {
                                    unique_name(&self.library, &format!("{} copy", self.name))
                                } else {
                                    self.name.clone()
                                };
                                match self.library.duplicate(self.selected, &name) {
                                    Ok(id) => {
                                        self.choose(id);
                                        self.persist("Preset duplicated");
                                        ui.close_menu();
                                    }
                                    Err(error) => self.notice = error,
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            let exists = self.library.get(self.selected).is_some();
                            if ui
                                .add_enabled(exists, egui::Button::new("Rename"))
                                .clicked()
                            {
                                match self.library.rename(self.selected, &self.name) {
                                    Ok(()) => {
                                        self.choose(self.selected);
                                        self.persist("Preset renamed");
                                        ui.close_menu();
                                    }
                                    Err(error) => self.notice = error,
                                }
                            }
                            if ui
                                .add_enabled(exists, egui::Button::new("Update"))
                                .on_hover_text(
                                    "Replace this preset with the current brush controls",
                                )
                                .clicked()
                            {
                                match self.library.update(
                                    self.selected,
                                    current.brush,
                                    current.radius,
                                    current.color,
                                    current.stabilization,
                                ) {
                                    Ok(()) => {
                                        self.persist("Preset updated");
                                        ui.close_menu();
                                    }
                                    Err(error) => self.notice = error,
                                }
                            }
                            if ui
                                .add_enabled(exists, egui::Button::new("Delete"))
                                .clicked()
                            {
                                match self.library.delete(self.selected) {
                                    Ok(()) => {
                                        self.choose(
                                            self.library.presets.first().map_or(0, |p| p.id),
                                        );
                                        self.persist("Preset deleted");
                                        ui.close_menu();
                                    }
                                    Err(error) => self.notice = error,
                                }
                            }
                        });
                        if ui
                            .button("Restore built-in defaults")
                            .on_hover_text(
                                "Restore ten built-in brushes while keeping personal presets",
                            )
                            .clicked()
                        {
                            match self.library.restore_defaults() {
                                Ok(()) => {
                                    self.choose(self.library.presets.first().map_or(0, |p| p.id));
                                    self.persist("Built-in defaults restored");
                                    ui.close_menu();
                                }
                                Err(error) => self.notice = format!("Restore failed: {error}"),
                            }
                        }
                    });
                    if self.path.is_some()
                        && !self.repair_needed
                        && ui
                            .button("Save library")
                            .on_hover_text(
                                "Retry saving the current library without changing presets",
                            )
                            .clicked()
                    {
                        self.persist("Library saved");
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.repair_needed,
                            egui::Button::new("Import brush package…"),
                        )
                        .on_hover_text("Import a Sketch3D .brush.json file as a personal preset")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Sketch3D brush", &["brush.json", "json"])
                            .pick_file()
                        {
                            match BrushPackage::load(&path)
                                .and_then(|package| package.import_into(&mut self.library))
                            {
                                Ok(id) => {
                                    self.choose(id);
                                    self.persist("Brush imported");
                                }
                                Err(error) => {
                                    self.notice = format!("Brush import failed: {error}");
                                }
                            }
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            !self.repair_needed && self.library.get(self.selected).is_some(),
                            egui::Button::new("Export selected brush…"),
                        )
                        .on_hover_text("Save the selected brush as a portable .brush.json package")
                        .clicked()
                    {
                        if let Some(preset) = self.library.get(self.selected) {
                            let suggested = format!("{}.brush.json", safe_file_stem(&preset.name));
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Sketch3D brush", &["brush.json"])
                                .set_file_name(&suggested)
                                .save_file()
                            {
                                let tip_asset =
                                    self.library.tip_asset(preset.brush.tip_asset_id).cloned();
                                let result =
                                    BrushPackage::from_preset(preset, tip_asset).save(&path);
                                self.notice = match result {
                                    Ok(()) => {
                                        format!("Brush package exported to {}", path.display())
                                    }
                                    Err(error) => format!("Brush export failed: {error}"),
                                };
                            }
                        }
                        ui.close_menu();
                    }
                });
                if !self.notice.is_empty() {
                    ui.small(&self.notice);
                }
            });
        chosen
    }

    fn show_browser(&mut self, ctx: &egui::Context) -> Option<Preset> {
        let mut open = true;
        let mut close = false;
        let mut selected_id = self.selected;
        let mut use_id = None;
        let viewport = ctx.screen_rect().size();
        let dialog_size = egui::vec2(
            (viewport.x - 32.0).clamp(420.0, 760.0),
            (viewport.y - 48.0).clamp(360.0, 560.0),
        );
        egui::Window::new("Brush browser")
            .id(egui::Id::new("brush-browser-window"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(dialog_size)
            .resizable(false)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search brushes");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Name, shape, or pattern")
                            .id_salt("brush-browser-search")
                            .desired_width(ui.available_width().clamp(360.0, 440.0)),
                    );
                    response.widget_info(|| {
                        let mut info =
                            WidgetInfo::text_edit(ui.is_enabled(), &self.search, &self.search);
                        info.label = Some("Search brushes by name, shape, or pattern".into());
                        info
                    });
                    if self.focus_search {
                        response.request_focus();
                        self.focus_search = false;
                    }
                });
                ui.small("Choose a card to preview it. Browsing never changes the active brush.");
                ui.separator();

                let query = self.search.trim().to_lowercase();
                let filtered: Vec<Preset> = self
                    .library
                    .presets
                    .iter()
                    .filter(|preset| preset_matches(preset, &query))
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    ui.label("No brushes match this search.");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("brush-browser-results")
                        .auto_shrink([true, false])
                        .show(ui, |ui| {
                            egui::Grid::new("brush-browser-grid")
                                .num_columns(2)
                                .spacing(egui::vec2(12.0, 10.0))
                                .show(ui, |ui| {
                                    for (index, preset) in filtered.iter().enumerate() {
                                        let selected = selected_id == preset.id;
                                        egui::Frame::group(ui.style())
                                            .stroke(if selected {
                                                ui.visuals().selection.stroke
                                            } else {
                                                ui.visuals().widgets.noninteractive.bg_stroke
                                            })
                                            .show(ui, |ui| {
                                                ui.set_min_width(245.0);
                                                let response = ui.radio(selected, &preset.name);
                                                if response.clicked() {
                                                    selected_id = preset.id;
                                                }
                                                brush_thumbnail(ui, preset);
                                                ui.small(format!(
                                                    "{} · {}",
                                                    brush_kind_label(preset),
                                                    preset.brush.texture.kind.label()
                                                ));
                                            });
                                        if index % 2 == 1 {
                                            ui.end_row();
                                        }
                                    }
                                });
                        });
                }

                if let Some(preset) = self.library.get(selected_id).cloned() {
                    ui.separator();
                    ui.strong(format!("Preview: {}", preset.name));
                    self.preview(ui, &preset);
                    self.tip_preview(ui, &preset);
                    describe_preset(ui, &preset);
                    if ui.button("Use selected preset").clicked() {
                        use_id = Some(selected_id);
                        close = true;
                    }
                }
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        self.browser_open = open && !close;
        if selected_id != self.selected {
            self.choose(selected_id);
        }
        use_id.and_then(|id| self.library.get(id).cloned())
    }

    fn preview(&mut self, ui: &mut egui::Ui, preset: &Preset) {
        if self.preview_preset.as_ref() != Some(preset) {
            self.preview_doc = preview_document(preset);
            self.preview_preset = Some(preset.clone());
            self.renderer.invalidate();
        }
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(210.0), 64.0),
            egui::Sense::hover(),
        );
        if ui.is_rect_visible(rect) {
            let scale = ui.ctx().pixels_per_point();
            let camera = Camera {
                distance: 1.5,
                yaw: 0.3,
                pitch: 0.2,
                ..Default::default()
            };
            let id = self.renderer.render(
                &self.preview_doc,
                &camera,
                GuidePreview::Hidden,
                [
                    (rect.width() * scale).round() as u32,
                    (rect.height() * scale).round() as u32,
                ],
            );
            ui.painter().image(
                id,
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        ui.small("Preview uses synthetic pressure and tilt.");
    }

    fn tip_preview(&mut self, ui: &mut egui::Ui, preset: &Preset) {
        let id = preset.brush.tip_asset_id;
        if id == 0 {
            return;
        }
        let Some(asset) = self.library.tip_asset(id) else {
            ui.small("Image tip asset is unavailable.");
            return;
        };
        if !self.tip_previews.contains_key(&id) {
            let Ok(decoded) = asset.decoded() else {
                ui.small("Image tip preview could not be decoded.");
                return;
            };
            let size = [decoded.size[0] as usize, decoded.size[1] as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &decoded.pixels);
            let texture = ui.ctx().load_texture(
                format!("brush-tip-{id:016x}"),
                image,
                egui::TextureOptions::NEAREST,
            );
            self.tip_previews.insert(id, texture);
        }
        if let Some(texture) = self.tip_previews.get(&id) {
            ui.label(format!(
                "PNG tip · {} × {} · alpha preserved",
                asset.size[0], asset.size[1]
            ));
            ui.image((texture.id(), egui::vec2(96.0, 96.0)));
        }
        if let Some(provenance) = &asset.provenance {
            ui.small(format!("Source: {provenance}"));
        }
    }

    fn import_tip(&mut self, path: &Path, preset_id: u64) -> Result<String, String> {
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Choose a regular PNG image file".into());
        }
        let mut bytes =
            Vec::with_capacity(metadata.len().min(crate::brush_tip::MAX_PNG_BYTES as u64) as usize);
        fs::File::open(path)
            .map_err(|error| error.to_string())?
            .take(crate::brush_tip::MAX_PNG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > crate::brush_tip::MAX_PNG_BYTES {
            return Err("Brush tip PNG must be no larger than 512 KiB".into());
        }
        let provenance = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| name.chars().count() <= 240 && !name.chars().any(char::is_control))
            .map(|name| format!("Imported from {name}"));
        let asset = crate::brush_tip::BrushTipAsset::from_png(bytes, provenance)?;
        let file_name = asset.file_name();
        self.library.assign_tip_asset(preset_id, asset)?;
        Ok(file_name)
    }
}

fn preset_matches(preset: &Preset, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let kind = brush_kind_label(preset).to_lowercase();
    preset.name.to_lowercase().contains(query)
        || kind.contains(query)
        || preset
            .brush
            .texture
            .kind
            .label()
            .to_lowercase()
            .contains(query)
}

fn safe_file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim();
    if stem.is_empty() {
        "brush".into()
    } else {
        stem.to_owned()
    }
}

fn brush_kind_label(preset: &Preset) -> &'static str {
    match preset.brush.kind {
        crate::document::BrushKind::Round => "Round",
        crate::document::BrushKind::Ribbon => "Ribbon",
        crate::document::BrushKind::Chisel => "Chisel",
    }
}

/// Compact vector sample for each card; the selected preset also gets the full
/// renderer-backed preview below the result grid.
fn brush_thumbnail(ui: &mut egui::Ui, preset: &Preset) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(225.0, 48.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, ui.visuals().faint_bg_color);
    let color = egui::Color32::from_rgb(
        (preset.color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (preset.color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (preset.color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    let left = rect.left() + 14.0;
    let right = rect.right() - 14.0;
    let center = rect.center().y;
    let width = (4.0 + preset.radius.clamp(0.0, 0.25) * 32.0).clamp(4.0, 12.0);
    let points = vec![
        egui::pos2(left, center + 5.0),
        egui::pos2(egui::lerp(left..=right, 0.33), center - 4.0),
        egui::pos2(egui::lerp(left..=right, 0.67), center + 3.0),
        egui::pos2(right, center - 5.0),
    ];
    painter.add(egui::Shape::line(points, egui::Stroke::new(width, color)));
    if preset.brush.kind != crate::document::BrushKind::Round {
        painter.circle_filled(egui::pos2(left, center + 5.0), width * 0.45, color);
        painter.circle_filled(egui::pos2(right, center - 5.0), width * 0.45, color);
    }
}

fn describe_preset(ui: &mut egui::Ui, preset: &Preset) {
    ui.small(format!(
        "{} · {} · radius {:.3}",
        brush_kind_label(preset),
        preset.brush.texture.kind.label(),
        preset.radius
    ));
    ui.small(format!(
        "Pressure width: {} · taper {:.2}/{:.2} · stabilize {:.0}%",
        if preset.brush.dynamics.width.enabled {
            "on"
        } else {
            "off"
        },
        preset.brush.dynamics.taper_start,
        preset.brush.dynamics.taper_end,
        preset.stabilization * 100.0
    ));
}

/// A preview copies resolved settings and never refers back to a library ID.
pub fn preview_document(preset: &Preset) -> Document {
    Document {
        strokes: vec![Stroke {
            material: Default::default(),
            id: 1,
            fill: None,
            group_id: None,
            layer_id: 1,
            color: preset.color,
            radius: preset.radius,
            brush: preset.brush,
            points: (0..48)
                .map(|i| {
                    let t = i as f32 / 47.0;
                    Point {
                        position: Vec3::new(
                            -1.5 + 3.0 * t,
                            (t * std::f32::consts::TAU).sin() * 0.08,
                            0.0,
                        ),
                        pressure: 0.15 + 0.85 * (t * std::f32::consts::PI).sin(),
                        tilt: preset.brush.dynamics.tilt_orientation.then_some(
                            Vec3::new(0.0, 1.0, (t * std::f32::consts::PI).sin()).normalize(),
                        ),
                    }
                })
                .collect(),
        }],
        ..Document::default()
    }
}

fn load_state(path: &Path) -> (Library, bool, String) {
    match fs::metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            (Library::default(), false, String::new())
        }
        _ => match Library::load(path) {
            Ok(library) => (library, false, String::new()),
            Err(error) => (
                Library::default(),
                true,
                format!("{error}. Using temporary defaults. Repair keeps a backup before saving."),
            ),
        },
    }
}
fn unique_name(library: &Library, base: &str) -> String {
    let base: String = base.trim().chars().take(54).collect();
    for suffix in 0..=256 {
        let candidate = if suffix == 0 {
            base.clone()
        } else {
            format!("{base} {suffix}")
        };
        if !library
            .presets
            .iter()
            .any(|p| p.name.to_lowercase() == candidate.to_lowercase())
        {
            return candidate;
        }
    }
    format!("{base} new")
}

fn backup_original(path: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let mut backup_name = path.as_os_str().to_os_string();
    backup_name.push(format!(".corrupt-{stamp}.backup"));
    let backup = PathBuf::from(backup_name);
    let mut source = fs::File::open(path)
        .map_err(|e| format!("Cannot read original library for backup: {e}"))?;
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .map_err(|e| format!("Cannot create library backup: {e}"))?;
    io::copy(&mut source, &mut destination)
        .map_err(|e| format!("Cannot copy library backup: {e}"))?;
    destination.flush().map_err(|e| e.to_string())?;
    destination.sync_all().map_err(|e| e.to_string())?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_search_matches_names_shapes_and_patterns_without_applying() {
        let library = Library::default();
        let preset = &library.presets[0];
        assert!(preset_matches(preset, ""));
        assert!(preset_matches(preset, &preset.name.to_lowercase()));
        assert!(preset_matches(
            preset,
            &brush_kind_label(preset).to_lowercase()
        ));
        assert!(preset_matches(
            preset,
            &preset.brush.texture.kind.label().to_lowercase()
        ));
        assert!(!preset_matches(preset, "no matching brush"));
        assert_eq!(library.get(preset.id).unwrap().name, preset.name);
    }

    #[test]
    fn preview_preserves_resolved_settings_and_sample_pressure_is_valid() {
        let preset = Library::default().get(4).unwrap().clone();
        let preview = preview_document(&preset);
        assert_eq!(preview.strokes[0].brush, preset.brush);
        assert_eq!(preview.strokes[0].radius, preset.radius);
        assert_eq!(preview.strokes[0].color, preset.color);
        assert_eq!(preview.strokes[0].points.len(), 48);
        assert!(preview.strokes[0].points.iter().all(Point::is_valid));
        let loaded = Document::from_json(&serde_json::to_string(&preview).unwrap()).unwrap();
        assert_eq!(loaded.strokes, preview.strokes);
    }
    #[test]
    fn repair_backup_preserves_original_and_unique_copy_names_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brushes.json");
        let original = b"{invalid original data";
        fs::write(&path, original).unwrap();
        let backup = backup_original(&path).unwrap();
        Library::default().save(&path).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert!(Library::load(&path).is_ok());
        let mut library = Library::default();
        assert_eq!(unique_name(&library, "Round Ink"), "Round Ink 1");
        library.duplicate(1, "Round Ink 1").unwrap();
        assert_eq!(unique_name(&library, "Round Ink"), "Round Ink 2");
    }
}
