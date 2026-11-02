use super::*;
use crate::share_service::{
    OsCapabilityStore, ShareServiceClient, SharedArtwork, UpdatedArtwork, MAX_SHARE_BYTES,
};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ShareScope {
    InkOnly,
    WholeScene,
    Selected,
}

impl ShareScope {
    fn label(self) -> &'static str {
        match self {
            Self::InkOnly => "Whole drawing · ink only",
            Self::WholeScene => "Whole scene · ink + image planes + OBJ models",
            Self::Selected => "Selected strokes",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ShareRecord {
    model_id: String,
    viewer_url: String,
    expires_at: String,
    scope: ShareScope,
}

#[derive(Default, Serialize, Deserialize)]
struct SharePreferences {
    #[serde(default)]
    service_origin: String,
    #[serde(default)]
    records: Vec<ShareRecord>,
}

struct Preview {
    bytes: Vec<u8>,
    summary: crate::export::ExportSummary,
    material_count: usize,
    scope: ShareScope,
    target_id: Option<String>,
    acknowledge_public: bool,
    over_limit: bool,
}

struct ShareJob {
    receiver: Receiver<Result<ShareOutcome, String>>,
}

enum ShareOutcome {
    Preview(Preview),
    Published(ShareRecord),
    Updated(UpdatedArtwork, ShareScope),
    Unpublished(String),
}

#[derive(Default)]
pub(super) struct ShareUi {
    preferences: SharePreferences,
    loaded: bool,
    manager_open: bool,
    preview: Option<Preview>,
    unpublish_confirmation: Option<String>,
    job: Option<ShareJob>,
    notice: Option<String>,
    last_public_url: Option<String>,
}

impl ShareUi {
    pub(super) fn open_manager(&mut self) {
        self.load_preferences();
        self.manager_open = true;
    }

    fn load_preferences(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Ok(path) = preferences_path() {
            if let Ok(contents) = std::fs::read(path) {
                if let Ok(preferences) = serde_json::from_slice(&contents) {
                    self.preferences = preferences;
                }
            }
        }
    }

    fn save_preferences(&self) -> Result<(), String> {
        let path = preferences_path()?;
        let parent = path
            .parent()
            .ok_or("Local application-data folder is unavailable")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        serde_json::to_writer_pretty(temporary.as_file_mut(), &self.preferences)
            .map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        crate::atomic_file::replace(temporary, &path).map_err(|error| error.to_string())
    }

    fn begin_preview(
        &mut self,
        document: &Document,
        selection: &HashSet<u64>,
        image_root: &std::path::Path,
        scope: ShareScope,
        target_id: Option<String>,
        ctx: &egui::Context,
    ) {
        self.load_preferences();
        if self.job.is_some() {
            return;
        }
        let selected = (scope == ShareScope::Selected).then(|| selection.clone());
        let snapshot = export_ui::build_export_document_snapshot(
            document,
            selected.as_ref(),
            scope == ShareScope::WholeScene,
        );
        let image_root = image_root.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        let spawn = std::thread::Builder::new()
            .name("artwork-share-preview".into())
            .spawn(move || {
                let encoded = if scope == ShareScope::WholeScene {
                    crate::export::encode_with_images(
                        &snapshot,
                        selected.as_ref(),
                        Some(&image_root),
                    )
                } else {
                    crate::export::encode(&snapshot, selected.as_ref())
                }
                .map_err(|error| error.to_string())
                .map(|encoded| {
                    let over_limit = encoded.bytes.len() > MAX_SHARE_BYTES;
                    let material_count = snapshot
                        .strokes
                        .iter()
                        .map(|stroke| stroke.material)
                        .collect::<std::collections::HashSet<_>>()
                        .len();
                    ShareOutcome::Preview(Preview {
                        bytes: encoded.bytes,
                        summary: encoded.summary,
                        material_count,
                        scope,
                        target_id,
                        acknowledge_public: false,
                        over_limit,
                    })
                });
                let _ = sender.send(encoded);
                context.request_repaint();
            });
        match spawn {
            Ok(_) => {
                self.preview = None;
                self.job = Some(ShareJob { receiver });
                self.notice = Some("Preparing exact GLB bytes for review…".into());
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(error) => self.notice = Some(format!("Could not start sharing preview: {error}")),
        }
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context, status: &mut String) {
        let Some(job) = self.job.as_ref() else { return };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(TryRecvError::Disconnected) => Err("Sharing worker stopped unexpectedly.".into()),
        };
        self.job = None;
        match result {
            Ok(ShareOutcome::Preview(preview)) => {
                self.notice = None;
                self.preview = Some(preview);
            }
            Ok(ShareOutcome::Published(record)) => {
                self.last_public_url = Some(record.viewer_url.clone());
                let updated = self
                    .preferences
                    .records
                    .iter_mut()
                    .find(|r| r.model_id == record.model_id);
                if let Some(existing) = updated {
                    *existing = record.clone();
                } else {
                    self.preferences.records.push(record.clone());
                }
                match self.save_preferences() {
                    Ok(()) => {
                        self.notice = Some(format!(
                            "Shared link expires {} UTC.",
                            format_expiry(&record.expires_at)
                        ));
                        *status = "Artwork was shared successfully; copy the public link from Manage shared links.".into();
                    }
                    Err(error) => {
                        self.notice = Some(format!(
                            "Share succeeded, but local link metadata could not be saved: {error}"
                        ));
                    }
                }
                self.preview = None;
            }
            Ok(ShareOutcome::Updated(updated, scope)) => {
                let expiry = if let Some(record) = self
                    .preferences
                    .records
                    .iter_mut()
                    .find(|r| r.model_id == updated.model_id)
                {
                    record.expires_at = updated.expires_at;
                    record.scope = scope;
                    Some(record.expires_at.clone())
                } else {
                    None
                };
                if let Some(expiry) = expiry {
                    self.last_public_url = self
                        .preferences
                        .records
                        .iter()
                        .find(|record| record.model_id == updated.model_id)
                        .map(|record| record.viewer_url.clone());
                    match self.save_preferences() {
                        Ok(()) => {
                            self.notice = Some(format!(
                                "Share updated; the existing public URL remains valid until {}.",
                                format_expiry(&expiry)
                            ))
                        }
                        Err(error) => {
                            self.notice = Some(format!(
                                "Share updated, but local metadata could not be saved: {error}"
                            ))
                        }
                    }
                } else {
                    self.notice = Some("Share updated, but its local link record is missing. Reopen Share → Manage shared links.".into());
                }
                self.preview = None;
            }
            Ok(ShareOutcome::Unpublished(model_id)) => {
                self.last_public_url = None;
                self.preferences
                    .records
                    .retain(|record| record.model_id != model_id);
                self.notice = self.save_preferences().err().map_or_else(
                    || Some("Shared link was unpublished.".into()),
                    |error| {
                        Some(format!(
                            "Unpublished, but local metadata could not be saved: {error}"
                        ))
                    },
                );
                self.preview = None;
            }
            Err(error) => self.notice = Some(error),
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        document: &Document,
        selection: &HashSet<u64>,
        image_root: &std::path::Path,
        scope_enabled: [bool; 3],
    ) {
        self.load_preferences();
        if self.manager_open {
            let mut open = self.manager_open;
            egui::Window::new("Shared artwork links")
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label(
                        "Service URL (HTTPS required; HTTP is allowed only for localhost testing):",
                    );
                    let changed = ui
                        .text_edit_singleline(&mut self.preferences.service_origin)
                        .changed();
                    if changed {
                        if let Err(error) = self.save_preferences() {
                            self.notice = Some(format!("Could not save service URL: {error}"));
                        }
                    }
                    if self.preferences.service_origin.trim().is_empty() {
                        ui.colored_label(
                            ui_style::colors(ui.ctx()).error,
                            "Configure a sharing service URL before publishing.",
                        );
                    }
                    ui.separator();
                    if self.preferences.records.is_empty() {
                        ui.label("No locally managed share links.");
                    }
                    for record in self.preferences.records.clone() {
                        ui.group(|ui| {
                            ui.label(record.scope.label());
                            ui.hyperlink_to(&record.viewer_url, &record.viewer_url);
                            if ui.button("Copy public link").clicked() {
                                ui.ctx().copy_text(record.viewer_url.clone());
                                self.notice = Some("Public link copied to the clipboard.".into());
                            }
                            ui.small(format!(
                                "Expires: {} UTC",
                                format_expiry(&record.expires_at)
                            ));
                            ui.horizontal_wrapped(|ui| {
                                for (label, scope, enabled) in [
                                    (
                                        "Update with ink only…",
                                        ShareScope::InkOnly,
                                        scope_enabled[0],
                                    ),
                                    (
                                        "Update with whole scene…",
                                        ShareScope::WholeScene,
                                        scope_enabled[1],
                                    ),
                                    (
                                        "Update with selection…",
                                        ShareScope::Selected,
                                        scope_enabled[2],
                                    ),
                                ] {
                                    if ui
                                        .add_enabled(
                                            self.job.is_none() && enabled,
                                            egui::Button::new(label),
                                        )
                                        .clicked()
                                    {
                                        self.begin_preview(
                                            document,
                                            selection,
                                            image_root,
                                            scope,
                                            Some(record.model_id.clone()),
                                            ctx,
                                        );
                                    }
                                }
                                if ui
                                    .add_enabled(
                                        self.job.is_none(),
                                        egui::Button::new("Unpublish…"),
                                    )
                                    .clicked()
                                {
                                    self.unpublish_confirmation = Some(record.model_id.clone());
                                }
                            });
                        });
                    }
                });
            self.manager_open = open;
        }

        if let Some(model_id) = self.unpublish_confirmation.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Unpublish shared artwork?")
                .id(egui::Id::new(("unpublish-confirm", &model_id)))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("This immediately removes the public GLB. Anyone using the link will lose access.");
                    ui.horizontal(|ui| {
                        if ui.button("Unpublish now").clicked() {
                            confirm = true;
                        }
                        if ui.button("Keep link").clicked() {
                            cancel = true;
                        }
                    });
                });
            if confirm {
                self.unpublish_confirmation = None;
                self.start_unpublish(model_id);
            } else if cancel {
                self.unpublish_confirmation = None;
            }
        }

        if let Some(notice) = self.notice.clone() {
            egui::Window::new("Sharing")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 70.0))
                .show(ctx, |ui| {
                    ui.label(notice);
                    if let Some(url) = self.last_public_url.as_ref() {
                        ui.hyperlink_to(url, url);
                        if ui.button("Copy public link").clicked() {
                            ui.ctx().copy_text(url.clone());
                        }
                    }
                    if ui.button("Open link manager").clicked() {
                        self.manager_open = true;
                        self.notice = None;
                    }
                    if ui.button("Dismiss").clicked() {
                        self.notice = None;
                        self.last_public_url = None;
                    }
                });
        }

        if self.preview.is_some() {
            let mut cancel = false;
            let mut submit = false;
            let service_url_valid =
                crate::share_service::validate_service_origin(&self.preferences.service_origin)
                    .is_ok();
            egui::Window::new(if self.preview.as_ref().is_some_and(|p| p.target_id.is_some()) {
                "Review share update"
            } else { "Review public share" })
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let preview = self.preview.as_mut().unwrap();
                ui.strong(preview.scope.label());
                ui.label(format!("Exact GLB size: {} bytes", preview.bytes.len()));
                if preview.over_limit {
                    ui.colored_label(ui_style::colors(ui.ctx()).error, "This model exceeds the 10 MiB service upload limit and cannot be published.");
                }
                ui.label(format!("Strokes: {} · distinct materials: {}", preview.summary.strokes, preview.material_count));
                ui.label(format!("Image planes: {} · OBJ models: {}", preview.summary.image_planes, preview.summary.model_references));
                ui.separator();
                ui.label("Anyone with the public viewer URL can view or download this GLB until it expires. The link is not password protected.");
                ui.label("The upload includes only the reviewed GLB scope. Native drawing data, recovery files, brush library content, and unrelated local resources are excluded.");
                ui.checkbox(&mut preview.acknowledge_public, "I understand this creates a public link and want to continue.");
                if self.preferences.service_origin.trim().is_empty() {
                    ui.colored_label(ui_style::colors(ui.ctx()).error, "Set the service URL in Shared artwork links before continuing.");
                } else if !service_url_valid {
                    ui.colored_label(ui_style::colors(ui.ctx()).error, "Service URL must be an HTTPS origin (localhost HTTP is allowed for testing).");
                }
                ui.horizontal(|ui| {
                    let label = if preview.target_id.is_some() { "Replace public model" } else { "Publish public link" };
                    if ui.add_enabled(preview.acknowledge_public && !preview.over_limit && service_url_valid && self.job.is_none(), egui::Button::new(label)).clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() { cancel = true; }
                });
            });
            if submit {
                if let Some(preview) = self.preview.take() {
                    self.start_service_job(preview);
                }
            } else if cancel {
                self.preview = None;
            }
        }
    }

    fn start_service_job(&mut self, preview: Preview) {
        let origin = self.preferences.service_origin.clone();
        let (sender, receiver) = mpsc::channel();
        let result = std::thread::Builder::new()
            .name("artwork-share-request".into())
            .spawn(move || {
                let result = (|| {
                    let client =
                        ShareServiceClient::new(&origin, std::sync::Arc::new(OsCapabilityStore))
                            .map_err(|error| error.to_string())?;
                    if let Some(model_id) = preview.target_id {
                        client
                            .update(&model_id, &preview.bytes)
                            .map(|artwork| ShareOutcome::Updated(artwork, preview.scope))
                            .map_err(|error| error.to_string())
                    } else {
                        client
                            .publish(&preview.bytes)
                            .map(|artwork| {
                                ShareOutcome::Published(to_record(artwork, preview.scope))
                            })
                            .map_err(|error| error.to_string())
                    }
                })();
                let _ = sender.send(result);
            });
        match result {
            Ok(_) => self.job = Some(ShareJob { receiver }),
            Err(error) => self.notice = Some(format!("Could not start sharing request: {error}")),
        }
    }

    fn start_unpublish(&mut self, model_id: String) {
        let origin = self.preferences.service_origin.clone();
        let (sender, receiver) = mpsc::channel();
        let target = model_id.clone();
        let result = std::thread::Builder::new()
            .name("artwork-unpublish".into())
            .spawn(move || {
                let result =
                    ShareServiceClient::new(&origin, std::sync::Arc::new(OsCapabilityStore))
                        .map_err(|error| error.to_string())
                        .and_then(|client| {
                            client.unpublish(&target).map_err(|error| error.to_string())
                        })
                        .map(|()| ShareOutcome::Unpublished(target));
                let _ = sender.send(result);
            });
        match result {
            Ok(_) => self.job = Some(ShareJob { receiver }),
            Err(error) => self.notice = Some(format!("Could not start unpublish request: {error}")),
        }
    }
}

impl SketchApp {
    pub(super) fn start_share_preview(
        &mut self,
        selected: bool,
        whole_scene: bool,
        ctx: &egui::Context,
    ) {
        if self.fills.draft.is_some() || self.line.draft.is_some() {
            self.status = "Close or cancel the draft contour before sharing.".into();
            return;
        }
        if !self.can_export_scope(selected, whole_scene) {
            self.status = "Finish editing and select drawable strokes before sharing.".into();
            return;
        }
        let scope = if selected {
            ShareScope::Selected
        } else if whole_scene {
            ShareScope::WholeScene
        } else {
            ShareScope::InkOnly
        };
        self.share_ui.begin_preview(
            &self.doc,
            &self.selected,
            &self.image_asset_root,
            scope,
            None,
            ctx,
        );
    }
}

fn to_record(artwork: SharedArtwork, scope: ShareScope) -> ShareRecord {
    ShareRecord {
        model_id: artwork.model_id,
        viewer_url: artwork.viewer_url,
        expires_at: artwork.expires_at,
        scope,
    }
}

fn format_expiry(epoch_millis: &str) -> String {
    let Ok(millis) = epoch_millis.parse::<i64>() else {
        return epoch_millis.to_owned();
    };
    let seconds = millis.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn preferences_path() -> Result<PathBuf, String> {
    let base = crate::presets::path()?;
    let parent = base
        .parent()
        .ok_or("Local application-data folder is unavailable")?;
    Ok(parent.join("shares.json"))
}
