//! Local preset browsing and editing, with a real stroke-mesh preview.
use crate::{
    document::{Document, Point, Stroke},
    geometry::Camera,
    presets::{Library, Preset},
    renderer::SceneRenderer,
    surface::GuidePreview,
};
use eframe::{egui, egui_wgpu};
use glam::Vec3;
use std::{
    fs,
    io::{self, Write},
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
                let before = self.selected;
                let text = self
                    .library
                    .get(self.selected)
                    .map_or("No presets", |p| p.name.as_str());
                egui::ComboBox::from_id_salt("brush-preset-picker")
                    .width(ui.available_width() - 12.0)
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        for preset in &self.library.presets {
                            ui.selectable_value(&mut self.selected, preset.id, &preset.name);
                        }
                    });
                if self.selected != before {
                    self.choose(self.selected);
                }
                if let Some(preset) = self.library.get(self.selected).cloned() {
                    self.preview(ui, &preset);
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
                ui.add(
                    egui::TextEdit::singleline(&mut self.name)
                        .hint_text("Preset name")
                        .char_limit(64),
                );
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
                ui.add_enabled_ui(!self.repair_needed, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .button("Save as new")
                            .on_hover_text("Save the current brush controls as a personal preset")
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
                                }
                                Err(error) => self.notice = error,
                            }
                        }
                        if ui
                            .add_enabled(exists, egui::Button::new("Update"))
                            .on_hover_text("Replace this preset with the current brush controls")
                            .clicked()
                        {
                            match self.library.update(
                                self.selected,
                                current.brush,
                                current.radius,
                                current.color,
                                current.stabilization,
                            ) {
                                Ok(()) => self.persist("Preset updated"),
                                Err(error) => self.notice = error,
                            }
                        }
                        if ui
                            .add_enabled(exists, egui::Button::new("Delete"))
                            .clicked()
                        {
                            match self.library.delete(self.selected) {
                                Ok(()) => {
                                    self.choose(self.library.presets.first().map_or(0, |p| p.id));
                                    self.persist("Preset deleted");
                                }
                                Err(error) => self.notice = error,
                            }
                        }
                    });
                    if ui
                        .button("Restore built-in defaults")
                        .on_hover_text(
                            "Restore six built-in brushes while keeping personal presets",
                        )
                        .clicked()
                    {
                        match self.library.restore_defaults() {
                            Ok(()) => {
                                self.choose(self.library.presets.first().map_or(0, |p| p.id));
                                self.persist("Built-in defaults restored");
                            }
                            Err(error) => self.notice = format!("Restore failed: {error}"),
                        }
                    }
                });
                if self.path.is_some()
                    && !self.repair_needed
                    && ui
                        .button("Save library")
                        .on_hover_text("Retry saving the current library without changing presets")
                        .clicked()
                {
                    self.persist("Library saved");
                }
                if !self.notice.is_empty() {
                    ui.small(&self.notice);
                }
            });
        chosen
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
}

/// A preview copies resolved settings and never refers back to a library ID.
pub fn preview_document(preset: &Preset) -> Document {
    Document {
        strokes: vec![Stroke {
            id: 1,
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
