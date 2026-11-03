//! Ordered background recovery writes, coalesced at committed-edit boundaries.
use crate::{
    document::Document,
    recovery_store::{self, RecoveryAsset, RecoverySnapshot, Scan},
};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const QUIET_INTERVAL: Duration = Duration::from_secs(2);
const MAX_INTERVAL: Duration = Duration::from_secs(15);
const RETRY_INTERVAL: Duration = Duration::from_secs(15);

enum Command {
    Save {
        epoch: u64,
        revision: u64,
        snapshot: Box<RecoverySnapshot>,
    },
    Clear {
        epoch: u64,
    },
    Scan,
    Dismiss(PathBuf),
}
enum Event {
    Unavailable(String),
    Saved {
        epoch: u64,
        revision: u64,
        result: Result<PathBuf, String>,
    },
    Cleared {
        epoch: u64,
        result: Result<(), String>,
    },
    Scanned(Result<Scan, String>),
    Dismissed {
        path: PathBuf,
        result: Result<(), String>,
    },
}

pub struct Recovery {
    sender: Option<Sender<Command>>,
    receiver: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
    session: String,
    own_path: Option<PathBuf>,
    epoch: u64,
    pending: usize,
    saving: Option<(u64, u64)>,
    scanning: bool,
    first_unsaved: Option<Instant>,
    last_edit: Option<Instant>,
    retry_after: Option<Instant>,
    last_path: Option<PathBuf>,
    pub revision: u64,
    pub protected_revision: u64,
    pub scan: Option<Scan>,
    pub notice: String,
    pub last_error: Option<String>,
    pub cleanup_error: Option<String>,
    operation_error: Option<String>,
    failed_dismissals: Vec<(PathBuf, String)>,
}
impl Recovery {
    pub fn new(dir: Result<PathBuf, String>, ctx: egui::Context) -> Self {
        let session = recovery_store::session_id();
        let (sender, commands) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let mut notice = "Recovery ready; committed edits are saved automatically.".to_owned();
        let mut last_error = None;
        let mut own_path = None;
        let worker = match dir {
            Ok(dir) => {
                own_path = Some(dir.join(format!("recovery-{session}.json")));
                let worker_session = session.clone();
                match thread::Builder::new()
                    .name("sketch-recovery".into())
                    .spawn(move || {
                        let session_lock = match recovery_store::lock_session(&dir, &worker_session)
                        {
                            Ok(lock) => lock,
                            Err(error) => {
                                let _ = events.send(Event::Unavailable(error));
                                ctx.request_repaint();
                                return;
                            }
                        };
                        while let Ok(command) = commands.recv() {
                            let event = match command {
                                Command::Save {
                                    epoch,
                                    revision,
                                    snapshot,
                                } => Event::Saved {
                                    epoch,
                                    revision,
                                    result: recovery_store::write(&dir, &snapshot),
                                },
                                Command::Clear { epoch } => Event::Cleared {
                                    epoch,
                                    result: recovery_store::remove(
                                        &dir,
                                        &dir.join(format!("recovery-{worker_session}.json")),
                                    ),
                                },
                                Command::Scan => {
                                    Event::Scanned(recovery_store::scan(&dir, &worker_session))
                                }
                                Command::Dismiss(path) => Event::Dismissed {
                                    result: recovery_store::remove(&dir, &path),
                                    path,
                                },
                            };
                            if events.send(event).is_err() {
                                break;
                            }
                            ctx.request_repaint();
                        }
                        drop(session_lock);
                        let _ = std::fs::remove_file(
                            dir.join(format!("recovery-{worker_session}.lock")),
                        );
                    }) {
                    Ok(worker) => Some(worker),
                    Err(error) => {
                        let error = format!("Could not start recovery worker: {error}");
                        notice = error.clone();
                        last_error = Some(error);
                        None
                    }
                }
            }
            Err(error) => {
                notice = format!("Recovery unavailable: {error}");
                last_error = Some(notice.clone());
                None
            }
        };
        Self {
            sender: worker.as_ref().map(|_| sender),
            receiver,
            worker,
            session,
            own_path,
            epoch: 0,
            pending: 0,
            saving: None,
            scanning: false,
            first_unsaved: None,
            last_edit: None,
            retry_after: None,
            last_path: None,
            revision: 0,
            protected_revision: 0,
            scan: None,
            notice,
            operation_error: last_error.clone(),
            last_error,
            cleanup_error: None,
            failed_dismissals: Vec::new(),
        }
    }
    pub fn enabled(&self) -> bool {
        self.sender.is_some()
    }
    pub fn busy(&self) -> bool {
        self.pending > 0
    }
    pub fn last_snapshot_path(&self) -> Option<&Path> {
        self.last_path.as_deref()
    }
    pub fn directory(&self) -> Option<&Path> {
        self.own_path.as_deref().and_then(Path::parent)
    }
    pub fn current_session_id(&self) -> &str {
        &self.session
    }
    fn send(&mut self, command: Command) -> bool {
        let Some(sender) = &self.sender else {
            return false;
        };
        if sender.send(command).is_err() {
            self.sender = None;
            self.pending = 0;
            self.saving = None;
            self.scanning = false;
            self.notice = "Recovery worker stopped; recent edits are not protected.".into();
            self.operation_error = Some(self.notice.clone());
            self.last_error = self.operation_error.clone();
            false
        } else {
            self.pending += 1;
            true
        }
    }
    /// A new epoch prevents late success messages from protecting another drawing.
    /// The single worker processes clear after any earlier snapshot write.
    pub fn begin_document(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.revision = 0;
        self.protected_revision = 0;
        self.first_unsaved = None;
        self.last_edit = None;
        self.retry_after = None;
        self.saving = None;
        if self.send(Command::Clear { epoch: self.epoch }) {
            self.notice = "Clearing the previous recovery copy…".into();
        }
    }
    pub fn mark_saved(&mut self) {
        self.begin_document();
    }
    pub fn discard(&mut self) {
        self.begin_document();
    }
    pub fn mark_edit(&mut self) {
        self.mark_edit_at(Instant::now());
    }
    pub fn mark_edit_at(&mut self, now: Instant) {
        if self.revision == self.protected_revision {
            self.first_unsaved = Some(now);
        }
        self.revision = self.revision.saturating_add(1);
        self.last_edit = Some(now);
        if self.enabled() && self.last_error.is_none() {
            self.notice = "Unsaved edits pending recovery.".into();
        }
    }
    #[allow(dead_code)] // Compatibility helper for documents without sidecar resources.
    pub fn tick(
        &mut self,
        document: &Document,
        original: Option<PathBuf>,
        active: bool,
        now: Instant,
    ) -> bool {
        self.tick_with_assets(document, original, Vec::new(), active, now)
    }

    /// Like `tick`, while capturing the current drawing-local image bytes.
    /// The worker validates filename references and the combined 256 MiB cap
    /// before writing the recovery envelope.
    pub fn tick_with_assets(
        &mut self,
        document: &Document,
        original: Option<PathBuf>,
        assets: Vec<RecoveryAsset>,
        active: bool,
        now: Instant,
    ) -> bool {
        if !self.snapshot_due(active, now) {
            return false;
        }
        self.submit(document, original, assets)
    }

    /// Lets callers defer expensive sidecar reads until a recovery write is due.
    pub fn snapshot_due(&self, active: bool, now: Instant) -> bool {
        if active
            || self.revision == 0
            || self.revision <= self.protected_revision
            || self.saving.is_some()
            || self.retry_after.is_some_and(|at| now < at)
            || !self.enabled()
        {
            return false;
        }
        self.last_edit
            .is_some_and(|at| now.saturating_duration_since(at) >= QUIET_INTERVAL)
            || self
                .first_unsaved
                .is_some_and(|at| now.saturating_duration_since(at) >= MAX_INTERVAL)
    }
    /// Explicit software-verification/foreground lifecycle snapshot request.
    /// A clean starter or blank session remains excluded even when forced.
    pub fn force_snapshot(&mut self, document: &Document, original: Option<PathBuf>) -> bool {
        self.force_snapshot_with_assets(document, original, Vec::new())
    }

    pub fn force_snapshot_with_assets(
        &mut self,
        document: &Document,
        original: Option<PathBuf>,
        assets: Vec<RecoveryAsset>,
    ) -> bool {
        if self.revision == 0 || self.revision <= self.protected_revision || self.saving.is_some() {
            return false;
        }
        self.submit(document, original, assets)
    }
    fn submit(
        &mut self,
        document: &Document,
        original: Option<PathBuf>,
        assets: Vec<RecoveryAsset>,
    ) -> bool {
        if !self.enabled() {
            return false;
        }
        let revision = self.revision;
        let command = Command::Save {
            epoch: self.epoch,
            revision,
            snapshot: Box::new(RecoverySnapshot::new_with_assets(
                self.session.clone(),
                original,
                document.clone(),
                assets,
            )),
        };
        if self.send(command) {
            self.saving = Some((self.epoch, revision));
            self.retry_after = None;
            self.notice = "Saving recovery snapshot…".into();
            true
        } else {
            false
        }
    }
    pub fn request_scan(&mut self) {
        if !self.scanning && self.send(Command::Scan) {
            self.scanning = true;
        }
    }
    pub fn dismiss(&mut self, path: &Path) {
        self.send(Command::Dismiss(path.to_path_buf()));
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let event = match self.receiver.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.sender.take().is_some() {
                        self.notice =
                            "Recovery worker stopped; recent edits are not protected.".into();
                        self.operation_error = Some(self.notice.clone());
                        self.last_error = self.operation_error.clone();
                        self.pending = 0;
                        self.saving = None;
                        self.scanning = false;
                        changed = true;
                    }
                    break;
                }
            };
            changed = true;
            self.pending = self.pending.saturating_sub(1);
            match event {
                Event::Unavailable(error) => {
                    self.sender = None;
                    self.pending = 0;
                    self.saving = None;
                    self.scanning = false;
                    self.notice = format!("Recovery unavailable: {error}");
                    self.operation_error = Some(self.notice.clone());
                    self.last_error = self.operation_error.clone();
                }
                Event::Saved {
                    epoch,
                    revision,
                    result,
                } => {
                    if self.saving == Some((epoch, revision)) {
                        self.saving = None;
                    }
                    if epoch != self.epoch {
                        continue;
                    }
                    match result {
                        Ok(path) => {
                            self.last_path = Some(path);
                            self.protected_revision = self.protected_revision.max(revision);
                            self.operation_error = None;
                            self.last_error = None;
                            self.retry_after = None;
                            if self.protected_revision == self.revision {
                                self.first_unsaved = None;
                                self.last_edit = None;
                                self.notice = "Recovery snapshot saved.".into();
                            } else {
                                self.first_unsaved = Some(Instant::now());
                                self.notice =
                                    "Recovery snapshot saved; newer edits are pending.".into();
                            }
                        }
                        Err(error) => {
                            self.notice=format!("Recovery save failed: {error}. Recent edits are not protected; save your drawing manually.");
                            self.operation_error = Some(self.notice.clone());
                            self.last_error = self.operation_error.clone();
                            self.retry_after = Some(Instant::now() + RETRY_INTERVAL);
                        }
                    }
                }
                Event::Cleared { epoch, result } => {
                    if epoch != self.epoch {
                        continue;
                    }
                    match result {
                        Ok(()) => {
                            self.last_path = None;
                            self.operation_error = None;
                            self.last_error = None;
                            self.notice = if self.revision == 0 {
                                "Recovery ready."
                            } else {
                                "Unsaved edits pending recovery."
                            }
                            .into();
                        }
                        Err(error) => {
                            self.last_path = self.own_path.clone();
                            self.notice = format!("Could not clear recovery copy: {error}");
                            self.operation_error = Some(self.notice.clone());
                            self.last_error = self.operation_error.clone();
                        }
                    }
                }
                Event::Scanned(result) => {
                    self.scanning = false;
                    match result {
                        Ok(scan) => self.scan = Some(scan),
                        Err(error) => {
                            self.notice = format!("Could not inspect recovery copies: {error}");
                            self.operation_error = Some(self.notice.clone());
                            self.last_error = self.operation_error.clone();
                        }
                    }
                }
                Event::Dismissed { path, result } => match result {
                    Ok(()) => {
                        if let Some(scan) = self.scan.as_mut() {
                            scan.candidates
                                .retain(|c| !same_recovery_name(&c.path, &path));
                            scan.issues.retain(|i| !same_recovery_name(&i.path, &path));
                        }
                        self.failed_dismissals
                            .retain(|(failed, _)| !same_recovery_name(failed, &path));
                        self.refresh_cleanup_error();
                        if self.cleanup_error.is_none() && self.operation_error.is_none() {
                            self.notice = "Recovery copy dismissed.".into();
                        }
                    }
                    Err(error) => {
                        self.failed_dismissals
                            .retain(|(failed, _)| !same_recovery_name(failed, &path));
                        self.failed_dismissals
                            .push((path, format!("Could not dismiss recovery copy: {error}")));
                        self.refresh_cleanup_error();
                    }
                },
            }
        }
        self.last_error = self
            .cleanup_error
            .clone()
            .or_else(|| self.operation_error.clone());
        if let Some(error) = &self.cleanup_error {
            self.notice = error.clone();
        }
        changed
    }
    fn refresh_cleanup_error(&mut self) {
        self.cleanup_error = self.failed_dismissals.last().map(|(_, error)| {
            if self.failed_dismissals.len() > 1 {
                format!(
                    "{error} ({} recovery copies could not be removed.)",
                    self.failed_dismissals.len()
                )
            } else {
                error.clone()
            }
        });
    }
}
// Every scan entry is a direct child of the same recovery directory, and a
// successful remove has already validated that boundary on the worker. Match
// its filename without filesystem access after removal; canonicalization would
// fail for the deleted file and add I/O to the UI poll path.
fn same_recovery_name(a: &Path, b: &Path) -> bool {
    match (a.file_name(), b.file_name()) {
        (Some(a), Some(b)) => {
            #[cfg(windows)]
            {
                a.to_string_lossy()
                    .eq_ignore_ascii_case(&b.to_string_lossy())
            }
            #[cfg(not(windows))]
            {
                a == b
            }
        }
        _ => false,
    }
}
impl Drop for Recovery {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point, Stroke};
    use glam::Vec3;
    fn document() -> Document {
        Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![Stroke {
                model_surface: None,
                gradient: None,
                curve: None,
                material: Default::default(),
                id: 1,
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: [0.2, 0.7, 0.6],
                radius: 0.03,
                brush: Brush::default(),
                points: vec![Point {
                    position: Vec3::X,
                    pressure: 0.42,
                    tilt: Some(Vec3::Y),

                    time_ms: 0,
                }],
            }],
            ..Document::default()
        }
    }
    fn wait(recovery: &mut Recovery) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while recovery.busy() && Instant::now() < deadline {
            recovery.poll();
            thread::sleep(Duration::from_millis(2));
        }
        recovery.poll();
        assert!(!recovery.busy(), "background worker timed out");
    }
    #[test]
    fn coalesces_committed_edits_and_bounds_repeated_edit_delay() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        let doc = document();
        let now = Instant::now();
        assert!(!recovery.force_snapshot(&doc, None));
        assert!(!recovery.tick(&doc, None, false, now + MAX_INTERVAL));
        recovery.mark_edit_at(now);
        assert!(!recovery.tick(&doc, None, false, now + Duration::from_secs(1)));
        recovery.mark_edit_at(now + Duration::from_secs(1));
        assert!(!recovery.tick(&doc, None, true, now + Duration::from_secs(3)));
        assert!(recovery.tick(&doc, None, false, now + Duration::from_secs(3)));
        assert!(!recovery.tick(&doc, None, false, now + Duration::from_secs(4)));
        wait(&mut recovery);
        assert_eq!(recovery.protected_revision, 2);
        assert!(recovery.last_snapshot_path().unwrap().exists());
        recovery.mark_edit_at(now + Duration::from_secs(5));
        for seconds in 6..=20 {
            recovery.mark_edit_at(now + Duration::from_secs(seconds));
        }
        assert!(recovery.tick(&doc, None, false, now + Duration::from_secs(20)));
        wait(&mut recovery);
        assert_eq!(recovery.protected_revision, recovery.revision);
    }
    #[test]
    fn ordered_clear_cannot_be_undone_by_a_late_writer() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        let doc = document();
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, None));
        recovery.discard();
        wait(&mut recovery);
        assert_eq!(recovery.revision, 0);
        assert_eq!(recovery.protected_revision, 0);
        assert!(recovery.last_snapshot_path().is_none());
        assert!(!recovery.own_path.as_ref().unwrap().exists());
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, None));
        recovery.mark_saved();
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, Some(PathBuf::from("source.sketch.json"))));
        wait(&mut recovery);
        let snapshot: RecoverySnapshot = serde_json::from_str(
            &std::fs::read_to_string(recovery.last_snapshot_path().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            snapshot.original_path,
            Some(PathBuf::from("source.sketch.json"))
        );
        assert_eq!(recovery.protected_revision, 1);
    }

    #[test]
    fn appearance_edit_is_restored_from_recovery_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        let mut doc = document();
        let edit = crate::editing::appearance_edit(
            &doc,
            &std::collections::HashSet::from([1]),
            crate::editing::AppearancePatch {
                color: Some([0.9, 0.25, 0.1]),
                radius: Some(0.08),
                opacity: Some(0.65),
                ..crate::editing::AppearancePatch::default()
            },
        )
        .unwrap();
        edit.apply(&mut doc, true);
        let expected = doc.strokes.clone();

        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, None));
        wait(&mut recovery);
        drop(recovery);
        let scan = recovery_store::scan(dir.path(), "another-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].snapshot.document.strokes, expected);
    }

    #[test]
    fn brush_tip_survives_native_relocation_and_forced_recovery_scan() {
        let pixels = image::RgbaImage::from_fn(5, 3, |x, _y| {
            image::Rgba([218, 73, 41, [0, 64, 128, 255, 0][x as usize]])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let asset = crate::brush_tip::BrushTipAsset::from_png(
            png.into_inner(),
            Some("native relocation recovery fixture".into()),
        )
        .unwrap();

        let mut document = document();
        document.strokes[0].brush.tip_asset_id = asset.id;
        document.strokes[0].brush.tip_aspect = asset.size[0] as f32 / asset.size[1] as f32;
        document.strokes[0].brush.dabs.enabled = true;
        document.add_brush_tip_asset(asset.clone()).unwrap();

        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source_dir = root.path().join("source-drawing");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source_path = source_dir.join("drawing.sketch.json");
        std::fs::write(&source_path, serde_json::to_vec(&document).unwrap()).unwrap();
        let relocated_dir = root.path().join("relocated-drawing");
        std::fs::rename(&source_dir, &relocated_dir).unwrap();
        let relocated_path = relocated_dir.join("drawing.sketch.json");
        let reopened =
            Document::from_json(&std::fs::read_to_string(&relocated_path).unwrap()).unwrap();
        assert_eq!(reopened.strokes[0].brush.tip_asset_id, asset.id);
        assert_eq!(reopened.brush_tip_assets, vec![asset.clone()]);
        let relocated_asset = reopened
            .brush_tip_assets
            .iter()
            .find(|candidate| candidate.id == asset.id)
            .unwrap();
        assert_eq!(relocated_asset.png, asset.png);
        assert_eq!(relocated_asset.alpha_at_uv([0.1, 0.5]).unwrap(), 0.0);
        assert!((relocated_asset.alpha_at_uv([0.3, 0.5]).unwrap() - 64.0 / 255.0).abs() < 1e-6);
        assert!((relocated_asset.alpha_at_uv([0.5, 0.5]).unwrap() - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(relocated_asset.alpha_at_uv([0.7, 0.5]).unwrap(), 1.0);
        assert_eq!(relocated_asset.alpha_at_uv([0.9, 0.5]).unwrap(), 0.0);

        let recovery_dir = root.path().join("recovery");
        let mut recovery = Recovery::new(Ok(recovery_dir.clone()), egui::Context::default());
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&reopened, Some(relocated_path.clone())));
        wait(&mut recovery);
        assert!(recovery.last_error.is_none());
        drop(recovery);

        let mut fresh_reader = Recovery::new(Ok(recovery_dir), egui::Context::default());
        fresh_reader.request_scan();
        wait(&mut fresh_reader);
        assert!(fresh_reader.last_error.is_none());
        let scan = fresh_reader.scan.as_ref().unwrap();
        assert!(scan.issues.is_empty());
        assert_eq!(scan.candidates.len(), 1);
        let snapshot = &scan.candidates[0].snapshot;
        assert_eq!(snapshot.original_path, Some(relocated_path));
        let recovered = &snapshot.document;
        assert_eq!(recovered.strokes[0].brush.tip_asset_id, asset.id);
        let recovered_asset = recovered
            .brush_tip_assets
            .iter()
            .find(|candidate| candidate.id == asset.id)
            .unwrap();
        assert_eq!(recovered_asset.png, asset.png);
        assert_eq!(recovered_asset.id, asset.id);
        assert_eq!(recovered_asset.alpha_at_uv([0.1, 0.5]).unwrap(), 0.0);
        assert!((recovered_asset.alpha_at_uv([0.3, 0.5]).unwrap() - 64.0 / 255.0).abs() < 1e-6);
        assert!((recovered_asset.alpha_at_uv([0.5, 0.5]).unwrap() - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(recovered_asset.alpha_at_uv([0.7, 0.5]).unwrap(), 1.0);
        assert_eq!(recovered_asset.alpha_at_uv([0.9, 0.5]).unwrap(), 0.0);
    }

    #[test]
    fn saved_guides_and_active_target_survive_recovery_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        let mut doc = document();
        use crate::document::guide_resources::{
            GuideGeometry, GuideResource, LoftSection, LoftSurface,
        };
        let loft_section = |z: f32| LoftSection {
            controls: [
                Vec3::new(-1.0, 0.0, z),
                Vec3::new(-0.3, 0.2, z),
                Vec3::new(0.3, -0.1, z),
                Vec3::new(1.0, 0.0, z),
            ],
        };
        let mut sections = [loft_section(0.0); 8];
        sections[0] = loft_section(-1.0);
        sections[1] = loft_section(0.0);
        sections[2] = loft_section(1.0);
        doc.guide_resources = vec![GuideResource {
            id: 23,
            name: "Recovery loft".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Loft {
                surface: LoftSurface {
                    center: Vec3::new(1.0, 2.0, 3.0),
                    rotation: glam::Quat::IDENTITY,
                    sections,
                    section_count: 3,
                    tension: -0.2,
                },
            },
        }];
        doc.active_guide = Some(23);
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, None));
        wait(&mut recovery);
        drop(recovery);
        let scan = recovery_store::scan(dir.path(), "another-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(
            scan.candidates[0].snapshot.document.guide_resources,
            doc.guide_resources
        );
        assert_eq!(
            scan.candidates[0].snapshot.document.active_guide,
            doc.active_guide
        );
    }

    #[test]
    fn ring_guide_and_active_target_survive_recovery_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        let mut doc = document();
        use crate::document::guide_resources::{GuideGeometry, GuideResource};
        let ring = GuideResource {
            id: 31,
            name: "Recovery ring".into(),
            visible: true,
            locked: false,
            geometry: GuideGeometry::Ring {
                center: Vec3::new(1.5, -0.75, 2.25),
                rotation: glam::Quat::from_rotation_y(0.37) * glam::Quat::from_rotation_x(-0.21),
                major_radius: 2.4,
                minor_radius: 0.6,
            },
        };
        doc.guide_resources = vec![ring];
        doc.active_guide = Some(31);

        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, None));
        wait(&mut recovery);
        drop(recovery);

        let scan = recovery_store::scan(dir.path(), "another-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(
            scan.candidates[0].snapshot.document.guide_resources,
            doc.guide_resources
        );
        assert_eq!(scan.candidates[0].snapshot.document.active_guide, Some(31));
    }

    #[test]
    fn failed_background_write_never_claims_protection_and_retries_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        // A directory at the final filename makes atomic replacement fail,
        // while allowing the worker's per-session lock to initialize normally.
        std::fs::create_dir(recovery.own_path.as_ref().unwrap()).unwrap();
        let doc = document();
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&doc, None));
        wait(&mut recovery);
        assert_eq!(recovery.protected_revision, 0);
        assert!(recovery.last_error.is_some());
        assert!(recovery.notice.contains("failed"));
        assert!(!recovery.tick(&doc, None, false, Instant::now() + QUIET_INTERVAL));
        assert!(recovery.tick(
            &doc,
            None,
            false,
            Instant::now() + RETRY_INTERVAL + QUIET_INTERVAL
        ));
        wait(&mut recovery);
        assert_eq!(recovery.protected_revision, 0);
    }
    #[test]
    fn scan_and_dismiss_leave_other_sessions_alone_until_requested() {
        let dir = tempfile::tempdir().unwrap();
        let doc = document();
        let snapshot = RecoverySnapshot::new("interrupted-session".into(), None, doc);
        let path = recovery_store::write(dir.path(), &snapshot).unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        recovery.request_scan();
        wait(&mut recovery);
        assert_eq!(recovery.scan.as_ref().unwrap().candidates.len(), 1);
        assert!(path.exists());
        recovery.dismiss(&path);
        wait(&mut recovery);
        assert!(!path.exists());
        assert!(recovery.scan.as_ref().unwrap().candidates.is_empty());
    }
    #[test]
    fn live_session_is_not_offered_until_its_worker_exits() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&document(), None));
        wait(&mut recovery);
        assert!(recovery_store::scan(dir.path(), "another-session")
            .unwrap()
            .candidates
            .is_empty());
        drop(recovery);
        let scan = recovery_store::scan(dir.path(), "another-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(
            scan.candidates[0].snapshot.document.strokes,
            document().strokes
        );
    }
    #[test]
    fn unavailable_lock_disables_worker_without_claiming_protection() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let mut recovery = Recovery::new(Ok(blocked), egui::Context::default());
        recovery.mark_edit();
        recovery.force_snapshot(&document(), None);
        wait(&mut recovery);
        assert!(!recovery.enabled());
        assert_eq!(recovery.protected_revision, 0);
        assert!(recovery.last_error.is_some());
        assert!(recovery.last_snapshot_path().is_none());
    }
    #[test]
    fn older_checkpoint_does_not_turn_continuous_edits_into_a_write_storm() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        let (events, receiver) = mpsc::channel();
        recovery.receiver = receiver;
        recovery.revision = 2;
        recovery.pending = 1;
        recovery.saving = Some((0, 1));
        let now = Instant::now();
        recovery.first_unsaved = Some(now - MAX_INTERVAL - MAX_INTERVAL);
        recovery.last_edit = Some(now);
        events
            .send(Event::Saved {
                epoch: 0,
                revision: 1,
                result: Ok(dir.path().join("checkpoint.json")),
            })
            .unwrap();
        recovery.poll();
        assert_eq!(recovery.protected_revision, 1);
        assert!(!recovery.tick(&document(), None, false, now));
    }
    #[test]
    fn failed_retirement_survives_unrelated_save_and_clear_until_each_retry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("recovery-stuck-a.json");
        let b = dir.path().join("recovery-stuck-b.json");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let mut recovery = Recovery::new(Ok(dir.path().into()), egui::Context::default());
        recovery.dismiss(&a);
        wait(&mut recovery);
        assert!(recovery.cleanup_error.is_some());
        recovery.mark_edit();
        assert!(recovery.force_snapshot(&document(), None));
        wait(&mut recovery);
        assert_eq!(recovery.protected_revision, recovery.revision);
        assert!(recovery.cleanup_error.is_some());
        assert_eq!(recovery.last_error, recovery.cleanup_error);
        recovery.mark_saved();
        wait(&mut recovery);
        assert!(recovery.last_snapshot_path().is_none());
        assert!(recovery.cleanup_error.is_some());
        assert_eq!(recovery.last_error, recovery.cleanup_error);
        recovery.dismiss(&b);
        wait(&mut recovery);
        std::fs::remove_dir(&a).unwrap();
        std::fs::write(&a, b"{corrupt record}").unwrap();
        recovery.dismiss(&a);
        wait(&mut recovery);
        assert!(
            recovery.cleanup_error.is_some(),
            "the second retained copy still needs attention"
        );
        std::fs::remove_dir(&b).unwrap();
        std::fs::write(&b, b"{corrupt record}").unwrap();
        recovery.dismiss(&b);
        wait(&mut recovery);
        assert!(recovery.cleanup_error.is_none());
        assert!(recovery.last_error.is_none());
    }
}
