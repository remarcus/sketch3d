use super::*;
use crate::document::BrushKind;
use std::time::{SystemTime, UNIX_EPOCH};

fn age_label(time: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let seconds = now.saturating_sub(time) / 1000;
    if seconds < 60 {
        format!("{seconds} seconds ago")
    } else if seconds < 3600 {
        format!("{} minutes ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{} hours ago", seconds / 3600)
    } else {
        format!("{} days ago", seconds / 86400)
    }
}
impl SketchApp {
    pub(super) fn record_recovery_edit(&mut self) {
        if self.tutorial.practice.is_some() {
            return;
        }
        self.close_cleanup_failed = false;
        self.recovery.mark_edit();
    }
    pub(super) fn reset_recovery_document(&mut self) {
        self.close_cleanup_failed = false;
        self.retire_recovered_source();
        self.recovery.begin_document();
        self.recovered_source = None;
    }
    pub(super) fn retire_recovered_source(&mut self) {
        if let Some(path) = self.recovery_retire.take() {
            self.recovery.dismiss(&path);
        }
    }
    pub(super) fn reserved_recovery_destination(&self, path: &std::path::Path) -> bool {
        let Some(dir) = self.recovery.directory() else {
            return false;
        };
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        let reserved = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
            .starts_with(reserved)
    }
    pub(super) fn approve_recovery_close(&mut self) {
        self.retire_recovered_source();
        self.recovery.discard();
        self.close_after_recovery = true;
    }
    pub(super) fn poll_recovery(&mut self, ctx: &egui::Context) {
        self.recovery.poll();
        if !self.recovery_scan_seen {
            if let Some(scan) = &self.recovery.scan {
                self.recovery_scan_seen = true;
                self.recovery_prompt = !scan.candidates.is_empty() || !scan.issues.is_empty();
            }
        }
        if self.recovery.protected_revision > 0 && self.recovery.last_snapshot_path().is_some() {
            self.retire_recovered_source();
        }
        if self.close_after_recovery && !self.recovery.busy() {
            self.close_after_recovery = false;
            if self.recovery.cleanup_error.is_some()
                || (self.recovery.last_error.is_some()
                    && self.recovery.last_snapshot_path().is_some())
            {
                self.close_cleanup_failed = true;
                self.status="Recovery cleanup failed. The copy is retained. Close again to exit, or keep working.".into();
            } else {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    pub(super) fn tick_recovery(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(practice) = &self.tutorial.practice {
            self.recovery.tick_with_assets(
                &practice.doc,
                practice
                    .file
                    .clone()
                    .or_else(|| self.recovered_source.clone()),
                Vec::new(),
                !practice.dirty,
                now,
            );
            if practice.dirty && self.recovery.revision > self.recovery.protected_revision {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
            return;
        }
        let blocked = self.active.is_some()
            || self.drag.is_some()
            || !self.dirty
            || self.close_after_recovery
            || self.close_cleanup_failed
            || self.recovery_prompt
            || self.confirm;
        let assets = if self.recovery.snapshot_due(blocked, now) {
            let mut assets = self.image_recovery_assets();
            if !self.append_obj_recovery_assets(&mut assets) {
                assets.clear();
            }
            assets
        } else {
            Vec::new()
        };
        self.recovery.tick_with_assets(
            &self.doc,
            self.file.clone().or_else(|| self.recovered_source.clone()),
            assets,
            blocked,
            now,
        );
        if self.recovery.enabled()
            && self.dirty
            && self.recovery.revision > self.recovery.protected_revision
        {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
    fn choose_recovery(&mut self, index: usize) {
        self.recovery_to_restore = self
            .recovery
            .scan
            .as_ref()
            .and_then(|s| s.candidates.get(index))
            .cloned();
        if self.recovery_to_restore.is_some() {
            self.recovery_prompt = false;
            self.request(Pending::Recover);
        }
    }
    pub(super) fn install_recovery(&mut self) {
        let Some(candidate) = self.recovery_to_restore.take() else {
            return;
        };
        self.reset_recovery_document();
        self.cancel();
        self.image_asset_root = std::env::temp_dir().join(format!(
            "3dsketch-image-planes-recovered-{}",
            crate::recovery_store::session_id()
        ));
        let mut candidate = candidate;
        let recovered_assets = std::mem::take(&mut candidate.snapshot.assets);
        self.doc = candidate.snapshot.document;
        self.obj_reference_models.clear();
        self.next_id = self
            .doc
            .strokes
            .iter()
            .map(|s| s.id)
            .chain(self.doc.image_planes.iter().map(|plane| plane.id))
            .chain(self.doc.guide_resources.iter().map(|guide| guide.id))
            .chain(self.doc.model_references.iter().map(|model| model.id))
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.file = None;
        self.recovered_source = candidate.snapshot.original_path;
        self.recovery_retire = Some(candidate.path);
        self.dirty = true;
        self.record_recovery_edit();
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.camera = Camera::default();
        self.guide = DrawingPlane::default();
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = true;
        self.starter_scene = false;
        self.cylinder_example = false;
        self.renderer.invalidate();
        self.image_plane_assets.clear();
        self.renderer.clear_image_plane_assets();
        self.renderer.clear_obj_reference_models();
        self.install_recovery_image_assets(&recovered_assets);
        self.recovery.force_snapshot_with_assets(
            &self.doc,
            self.recovered_source.clone(),
            recovered_assets,
        );
        self.status = "Recovered an unsaved copy. Save as to choose a native drawing file.".into();
    }
    pub(super) fn show_recovery(&mut self, ctx: &egui::Context) {
        if !self.recovery_prompt {
            return;
        }
        let mut restore = None;
        let mut discard = None;
        let mut later = false;
        let busy = self.recovery.busy();
        egui::Window::new("Recover interrupted drawings").collapsible(false).resizable(true).default_width(540.0).anchor(egui::Align2::CENTER_CENTER,Vec2::ZERO).show(ctx,|ui| {
            ui.label("Recover opens an unsaved copy. Original drawing files stay unchanged.");
            if busy {ui.horizontal(|ui|{ui.spinner();ui.label("Checking recovery files...");});}
            if let Some(scan)=&self.recovery.scan {
                egui::ScrollArea::vertical().max_height(360.0).show(ui,|ui| {
                    for (index,candidate) in scan.candidates.iter().enumerate() {
                        ui.separator();
                        ui.strong(candidate.snapshot.original_path.as_ref().map_or_else(||"Untitled drawing".into(),|p|p.display().to_string()));
                        ui.label(format!("{} · {} strokes",age_label(candidate.snapshot.saved_at_unix_ms),candidate.snapshot.document.strokes.len()));
                        if candidate.stale {ui.small("This snapshot is over 30 days old or has a future timestamp.");}
                        if candidate.source_newer {ui.small("The original file has a newer save. This snapshot will still recover as a separate copy.");}
                        ui.add_enabled_ui(!busy,|ui|{ui.horizontal(|ui| {
                            if ui.button("Recover copy").clicked(){restore=Some(index);}
                            if ui.button("Discard snapshot").clicked(){discard=Some(candidate.path.clone());}
                        });});
                    }
                    for issue in &scan.issues {
                        ui.separator();ui.label(issue.path.display().to_string());ui.label(&issue.message);
                        if ui.add_enabled(!busy,egui::Button::new("Discard unreadable snapshot")).clicked(){discard=Some(issue.path.clone());}
                    }
                    if scan.candidates.is_empty() && scan.issues.is_empty(){ui.label("No interrupted drawings found.");}
                });
            }
            if self.recovery.last_error.is_some(){ui.label(&self.recovery.notice);}
            ui.separator();
            if ui.button("Continue without recovery").clicked(){later=true;}
            ui.small("Continue keeps these snapshots available from the Recovery button.");
        });
        if let Some(index) = restore {
            self.choose_recovery(index);
        }
        if let Some(path) = discard {
            self.recovery.dismiss(&path);
        }
        if later {
            self.recovery_prompt = false;
        }
    }

    pub(super) fn verify_recovery(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.recovery_verification_frame else {
            return;
        };
        let root = std::env::current_dir().unwrap().join("verification");
        let source = root.join("recovery-native.sketch.json");
        let expected_path = root.join("recovery-expected.sketch.json");
        if self.recovery_verify_crash {
            match self.recovery_verify_stage {
                0 => {
                    self.recovery_prompt = false;
                    std::fs::create_dir_all(&root).unwrap();
                    self.file = Some(source.clone());
                    assert!(self.save(false));
                    let mut fill = crate::polygon::study().strokes.remove(0);
                    fill.id = self.next_id;
                    self.next_id += 1;
                    self.doc.strokes.push(fill.clone());
                    self.undo.push(Edit::Add(fill));
                    self.dirty = true;
                    self.record_recovery_edit();
                    self.camera.align_guide(self.guide);
                    self.start_line_tool();
                    self.line.constraint = super::line_tool::Constraint::Horizontal;
                    let a = rect.center() + egui::vec2(-70.0, -65.0);
                    let b = rect.center() + egui::vec2(70.0, -45.0);
                    self.line_input(a, 0.3, None, rect, true);
                    self.line_input(b, 0.8, None, rect, false);
                    self.release_line(b, rect);
                    self.cancel_line_tool();
                    self.brush.kind = BrushKind::Chisel;
                    self.brush.texture.kind = crate::texture::TextureKind::Grain;
                    self.brush.texture.seed = 739;
                    self.brush.dynamics.width.minimum = 0.05;
                    self.brush.dynamics.width.curve = 1.8;
                    self.brush.dynamics.thickness.enabled = false;
                    self.brush.dynamics.tilt_orientation = true;
                    self.brush.dynamics.taper_start = 0.2;
                    self.brush.dynamics.taper_end = 0.4;
                    self.stabilization = 0.8;
                    let start = rect.center() - egui::vec2(110.0, 0.0);
                    for (i, (pressure, offset, tilt)) in [
                        (0.2, egui::vec2(0.0, 0.0), Vec3::Y),
                        (
                            0.9,
                            egui::vec2(110.0, -30.0),
                            Vec3::new(0.0, 1.0, 1.0).normalize(),
                        ),
                        (0.6, egui::vec2(220.0, 20.0), Vec3::Z),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        self.add_point_with_tilt(
                            start + offset,
                            pressure,
                            rect,
                            i == 0,
                            Some(tilt),
                        );
                    }
                    self.finish();
                    std::fs::write(&expected_path, serde_json::to_vec(&self.doc).unwrap()).unwrap();
                    assert!(self.recovery.force_snapshot(&self.doc, self.file.clone()));
                    self.recovery_verify_stage = 1;
                }
                1 if !self.recovery.busy()
                    && self.recovery.protected_revision == self.recovery.revision =>
                {
                    let snapshot = self.recovery.last_snapshot_path().unwrap();
                    std::fs::write(root.join("recovery-crash-ready.json"),serde_json::to_vec_pretty(&serde_json::json!({"snapshotPath":snapshot,"sourcePath":source,"sessionId":self.recovery.current_session_id(),"pid":std::process::id()})).unwrap()).unwrap();
                    self.recovery_verify_stage = 2;
                }
                _ => {}
            }
        } else {
            match self.recovery_verify_stage {
                0 if !self.recovery.busy() && self.recovery.scan.is_some() => {
                    assert!(
                        self.recovery_prompt,
                        "startup must offer the interrupted drawing"
                    );
                    self.recovery_verify_since = frame;
                    self.recovery_verify_stage = 1;
                }
                1 if frame > self.recovery_verify_since + 30 && !self.recovery_offer_capture => {
                    if !self.recovery_offer_requested {
                        self.recovery_offer_requested = true;
                        self.recovery_offer_capture = true;
                        ctx.send_viewport_cmd(
                            egui::ViewportCommand::Screenshot(Default::default()),
                        );
                    } else {
                        let marker: serde_json::Value = serde_json::from_str(
                            &std::fs::read_to_string(root.join("recovery-crash-ready.json"))
                                .unwrap(),
                        )
                        .unwrap();
                        let scan = self.recovery.scan.as_ref().unwrap();
                        let index = scan
                            .candidates
                            .iter()
                            .position(|c| {
                                c.snapshot.session_id == marker["sessionId"].as_str().unwrap()
                            })
                            .unwrap();
                        self.choose_recovery(index);
                        assert!(self.file.is_none() && self.dirty);
                        assert_eq!(self.recovered_source.as_ref(), Some(&source));
                        let expected =
                            Document::from_json(&std::fs::read_to_string(&expected_path).unwrap())
                                .unwrap();
                        assert_eq!(self.doc.strokes, expected.strokes);
                        assert_eq!(
                            self.doc
                                .strokes
                                .last()
                                .unwrap()
                                .points
                                .last()
                                .unwrap()
                                .pressure,
                            0.6
                        );
                        self.recovery_verify_stage = 2;
                    }
                }
                2 if !self.recovery.busy() && self.recovery_retire.is_none() => {
                    assert!(self.recovery.last_snapshot_path().unwrap().exists());
                    let original = std::fs::read(&source).unwrap();
                    // Saving a recovered copy is deliberate and leaves the original unchanged.
                    let recovered = self.doc.strokes.clone();
                    self.file = Some(root.join("recovery-restored-copy.sketch.json"));
                    assert!(self.save(false));
                    let saved = Document::from_json(
                        &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(saved.strokes, recovered);
                    assert_eq!(std::fs::read(&source).unwrap(), original);
                    let point = rect.center() + egui::vec2(40.0, 60.0);
                    self.add_point(point, 0.5, rect, true);
                    self.finish();
                    assert!(self.recovery.force_snapshot(&self.doc, self.file.clone()));
                    self.recovery_verify_stage = 3;
                }
                3 if !self.recovery.busy() => {
                    assert!(self.recovery.last_snapshot_path().unwrap().exists());
                    // New uses the same approved-discard flow as the interface.
                    self.request(Pending::New);
                    assert!(self.confirm);
                    self.execute_pending();
                    self.recovery_verify_stage = 4;
                }
                4 if !self.recovery.busy() => {
                    assert!(self.doc.strokes.is_empty());
                    assert!(self.recovery.last_snapshot_path().is_none());
                    // Add an invalid interrupted file and confirm it remains visible as an issue.
                    let folder = self.recovery.directory().unwrap();
                    let bad = folder.join("recovery-invalid-fixture.json");
                    std::fs::write(&bad, b"{incomplete").unwrap();
                    self.recovery.request_scan();
                    self.recovery_verify_stage = 5;
                }
                5 if !self.recovery.busy() => {
                    let issue = self
                        .recovery
                        .scan
                        .as_ref()
                        .unwrap()
                        .issues
                        .iter()
                        .find(|i| i.path.file_name().unwrap() == "recovery-invalid-fixture.json")
                        .unwrap()
                        .path
                        .clone();
                    self.recovery.dismiss(&issue);
                    self.recovery_verify_stage = 6;
                }
                6 if !self.recovery.busy() => {
                    self.load_textures();
                    self.recovery_prompt = false;
                    self.status="Forced crash, recovered copy, source preservation, and ordered cleanup passed.".into();
                    self.recovery_verify_stage = 7;
                }
                7 if !self.recovery.busy() => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                    self.recovery_verify_stage = 8;
                }
                _ => {}
            }
        }
        assert!(
            frame < 5000,
            "Recovery verification timed out: {}",
            self.recovery.notice
        );
        self.recovery_verification_frame = Some(frame + 1);
    }
}
