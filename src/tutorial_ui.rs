use super::*;
use crate::tutorial::{Preferences, LESSONS};

pub(super) struct TutorialUi {
    pub offer: bool,
    startup: bool,
    pub step: usize,
    preferences: Preferences,
    path: Result<PathBuf, String>,
    error: Option<String>,
    pub practice: Option<Practice>,
    pub verification_frame: Option<u32>,
}
pub(super) struct Practice {
    lighting: crate::lighting::Lighting,
    pub doc: Document,
    camera: Camera,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    selected: HashSet<u64>,
    next_id: u64,
    guide: DrawingPlane,
    cylinder: Cylinder,
    revolved: crate::revolved::Revolved,
    use_cylinder: bool,
    use_revolved: bool,
    guide_visible: bool,
    starter_scene: bool,
    cylinder_example: bool,
    brush: Brush,
    color: [f32; 3],
    radius: f32,
    stabilization: f32,
    select_mode: bool,
    eraser: bool,
    individual: bool,
    transform_mode: Mode,
    transform_values: [f32; 3],
    scale_value: f32,
    layer_target: u64,
    layer_name: String,
}
impl TutorialUi {
    pub fn new() -> Self {
        let verify = std::env::args().any(|a| a.starts_with("--verify"));
        let tutorial_verify = std::env::args().any(|a| a == "--verify-tutorial");
        let path = if verify {
            std::env::current_dir()
                .map(|p| p.join("verification/tutorial/onboarding.json"))
                .map_err(|e| e.to_string())
        } else {
            crate::presets::path().map(|p| p.with_file_name("onboarding.json"))
        };
        let loaded = path
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|p| Preferences::load(p));
        let error = loaded.as_ref().err().cloned();
        let preferences = loaded.unwrap_or_default();
        let startup = (!verify || tutorial_verify)
            && error.is_none()
            && (!preferences.seen || preferences.show_on_startup);
        Self {
            offer: false,
            startup,
            step: 0,
            preferences,
            path,
            error,
            practice: None,
            verification_frame: tutorial_verify.then_some(0),
        }
    }
    fn save_preferences(&mut self) {
        self.error = self
            .path
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|p| self.preferences.save(p))
            .err();
    }
}
impl SketchApp {
    fn tutorial_step(&mut self, delta: isize) {
        self.finish();
        self.tutorial.step = self
            .tutorial
            .step
            .saturating_add_signed(delta)
            .min(LESSONS.len() - 1);
    }
    pub(super) fn start_practice(&mut self) {
        if self.tutorial.practice.is_some() {
            return;
        }
        if !self.recovery_scan_seen || self.recovery_prompt {
            self.tutorial.offer = true;
            self.status = "Finish checking recovery before starting practice.".into();
            return;
        }
        self.finish();
        self.cancel();
        // Protect the suspended drawing, not the disposable lesson document.
        if self.dirty {
            self.recovery.force_snapshot(
                &self.doc,
                self.file.clone().or_else(|| self.recovered_source.clone()),
            );
        }
        self.tutorial.practice = Some(Practice {
            lighting: self.renderer.lighting,
            doc: std::mem::take(&mut self.doc),
            camera: std::mem::take(&mut self.camera),
            undo: std::mem::take(&mut self.undo),
            redo: std::mem::take(&mut self.redo),
            file: self.file.take(),
            dirty: self.dirty,
            selected: std::mem::take(&mut self.selected),
            next_id: self.next_id,
            guide: self.guide,
            cylinder: self.cylinder,
            revolved: self.revolved,
            use_cylinder: self.use_cylinder,
            use_revolved: self.use_revolved,
            guide_visible: self.guide_visible,
            starter_scene: self.starter_scene,
            cylinder_example: self.cylinder_example,
            brush: self.brush,
            color: self.color,
            radius: self.radius,
            stabilization: self.stabilization,
            select_mode: self.select_mode,
            eraser: self.eraser,
            individual: self.individual_selection,
            transform_mode: self.transform_mode,
            transform_values: self.transform_values,
            scale_value: self.scale_value,
            layer_target: self.layer_target,
            layer_name: std::mem::take(&mut self.layer_name),
        });
        self.renderer.lighting = Default::default();
        self.guide = DrawingPlane::from_preset(Plane::Front);
        self.camera.align(Plane::Front);
        self.use_cylinder = false;
        self.use_revolved = false;
        self.guide_visible = true;
        self.brush = Brush::default();
        self.radius = 0.025;
        self.color = [0.17, 0.82, 0.74];
        self.stabilization = 0.35;
        self.select_mode = false;
        self.eraser = false;
        self.individual_selection = false;
        self.starter_scene = false;
        self.cylinder_example = false;
        self.layer_target = 1;
        self.next_id = 1;
        self.dirty = false;
        self.tutorial.step = 0;
        self.tutorial.offer = false;
        self.tutorial.startup = false;
        self.show_help = false;
        self.tutorial.preferences.seen = true;
        self.tutorial.save_preferences();
        self.renderer.invalidate();
        self.status = "Tutorial practice: original drawing is suspended. Save as to keep practice; Exit restores the original.".into();
    }
    pub(super) fn end_practice(&mut self) {
        self.finish();
        self.cancel();
        let Some(p) = self.tutorial.practice.take() else {
            return;
        };
        self.renderer.lighting = p.lighting;
        self.doc = p.doc;
        self.camera = p.camera;
        self.undo = p.undo;
        self.redo = p.redo;
        self.file = p.file;
        self.dirty = p.dirty;
        self.selected = p.selected;
        self.next_id = p.next_id;
        self.guide = p.guide;
        self.cylinder = p.cylinder;
        self.revolved = p.revolved;
        self.use_cylinder = p.use_cylinder;
        self.use_revolved = p.use_revolved;
        self.guide_visible = p.guide_visible;
        self.starter_scene = p.starter_scene;
        self.cylinder_example = p.cylinder_example;
        self.brush = p.brush;
        self.color = p.color;
        self.radius = p.radius;
        self.stabilization = p.stabilization;
        self.select_mode = p.select_mode;
        self.eraser = p.eraser;
        self.individual_selection = p.individual;
        self.transform_mode = p.transform_mode;
        self.transform_values = p.transform_values;
        self.scale_value = p.scale_value;
        self.layer_target = p.layer_target;
        self.layer_name = p.layer_name;
        self.renderer.invalidate();
        self.status = "Tutorial finished. Original drawing and edit history restored.".into();
    }
    pub(super) fn tutorial_hint(&self, ui: &mut egui::Ui, target: &str) {
        if self.tutorial.practice.is_some() && LESSONS[self.tutorial.step].target == target {
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.5_f32, Color32::from_rgb(76, 220, 190)))
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.colored_label(
                        Color32::from_rgb(76, 220, 190),
                        "Tutorial: try these controls",
                    );
                });
        }
    }
    pub(super) fn tutorial_help(&mut self, ui: &mut egui::Ui) {
        if ui.button("Start / replay tutorial").clicked() {
            self.replay_tutorial();
        }
        if ui
            .checkbox(
                &mut self.tutorial.preferences.show_on_startup,
                "Offer tutorial on every startup",
            )
            .changed()
        {
            self.tutorial.preferences.seen = true;
            self.tutorial.save_preferences();
        }
        if let Some(error) = &self.tutorial.error {
            ui.colored_label(Color32::LIGHT_RED, format!("Tutorial preferences: {error}"));
        }
    }
    fn replay_tutorial(&mut self) {
        self.finish();
        self.show_help = false;
        self.tutorial.offer = self.tutorial.practice.is_none();
        if self.tutorial.practice.is_some() {
            self.tutorial.step = 0;
        }
    }
    pub(super) fn show_tutorial(&mut self, ctx: &egui::Context) {
        if self.recovery_scan_seen
            && !self.recovery_prompt
            && !self.confirm
            && self.tutorial.practice.is_none()
            && self.tutorial.startup
        {
            self.tutorial.startup = false;
            self.tutorial.offer = true;
        }
        if self.recovery_prompt || self.confirm || self.close_after_recovery {
            return;
        }
        if self.tutorial.offer && self.tutorial.practice.is_none() {
            let mut start = false;
            let mut skip = false;
            egui::Window::new("Welcome to Spatial Sketch").collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).default_width(430.0).show(ctx, |ui| {
                ui.label("Learn to draw in 3D with a mouse, a screenless tablet, or a pen display.");
                ui.label("Nine short lessons with optional practice. Your current drawing stays in memory and returns when you exit. Save it first for a deliberate disk copy.");
                ui.small("Practice has no automatic recovery. You can Save as to keep it. You can replay the tutorial from Help anytime.");
                if ui.checkbox(&mut self.tutorial.preferences.show_on_startup, "Offer tutorial on every startup").changed() { self.tutorial.save_preferences(); }
                if let Some(error) = &self.tutorial.error { ui.colored_label(Color32::LIGHT_RED, format!("Preference save: {error}")); }
                ui.horizontal(|ui| { start = ui.button("Start practice").clicked(); skip = ui.button("Skip").clicked(); });
            });
            if skip {
                self.tutorial.offer = false;
                self.tutorial.preferences.seen = true;
                self.tutorial.save_preferences();
            }
            if start {
                self.tutorial.offer = false;
                self.request(Pending::TutorialStart);
            }
        }
        if self.tutorial.practice.is_some() {
            let lesson = &LESSONS[self.tutorial.step];
            let mut exit = false;
            egui::Window::new("Guided practice").collapsible(false).resizable(true)
                .default_width(360.0).anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-255.0,-38.0)).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(format!("Step {} of {} · {}", self.tutorial.step+1, LESSONS.len(), lesson.target));
                ui.heading(lesson.title); ui.label(lesson.text); ui.separator();
                ui.colored_label(Color32::from_rgb(76,220,190), lesson.practice);
                if self.tutorial.step == 1 { ui.small(if self.samples > 0 { "Pen samples received: inspect the Input lab pressure graph. This does not certify physical tablet behavior." } else { "No pen samples received yet. Mouse practice is fully supported." }); }
                ui.small("All steps are optional. Original drawing is suspended; practice has no automatic recovery.");
                if let Some(error) = &self.tutorial.error { ui.colored_label(Color32::LIGHT_RED, format!("Tutorial preferences: {error}")); }
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.tutorial.step > 0, egui::Button::new("Back")).clicked() { self.tutorial_step(-1); }
                    if self.tutorial.step+1 < LESSONS.len() {
                        if ui.button("Next").clicked() { self.tutorial_step(1); }
                    } else if ui.button("Finish").clicked() { exit = true; }
                    if ui.button("Exit practice").clicked() { exit = true; }
                });
            });
            if exit {
                self.request(Pending::TutorialEnd);
            }
        }
    }

    pub(super) fn verify_tutorial(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.tutorial.verification_frame else {
            return;
        };
        if frame == 0 && (!self.recovery_scan_seen || self.recovery.busy()) {
            return;
        }
        if frame == 2 && self.recovery.busy() {
            return;
        }
        match frame {
            0 => {
                self.tutorial.offer = false;
                self.tutorial.startup = true;
                self.recovery_prompt = true;
                self.show_tutorial(ctx);
                assert!(!self.tutorial.offer && self.tutorial.startup);
                self.recovery_prompt = false;
                self.show_tutorial(ctx);
                assert!(self.tutorial.offer);
                self.tutorial.offer = false;
                self.dirty = true;
                self.renderer.lighting = crate::lighting::Lighting {
                    azimuth: -85.0,
                    ground: true,
                    ..Default::default()
                };
                self.record_recovery_edit();
                self.selected = self.doc.strokes.iter().take(2).map(|s| s.id).collect();
                self.duplicate_selected();
                self.undo();
                assert!(!self.redo.is_empty());
                self.selected = self.doc.strokes.iter().take(2).map(|s| s.id).collect();
                let source = std::env::current_dir()
                    .unwrap()
                    .join("verification/tutorial/original.sketch.json");
                std::fs::create_dir_all(source.parent().unwrap()).unwrap();
                std::fs::write(&source, serde_json::to_vec_pretty(&self.doc).unwrap()).unwrap();
                self.file = Some(source);
                self.request(Pending::TutorialStart);
                assert!(self.confirm && self.tutorial.practice.is_none());
                self.pending = None;
                self.confirm = false;
                assert_eq!(self.doc.strokes.len(), 497);
                self.request(Pending::TutorialStart);
                self.execute_pending();
                assert!(self.doc.strokes.is_empty() && self.tutorial.practice.is_some());
                self.tutorial.preferences.show_on_startup = false;
                self.tutorial.save_preferences();
                assert!(!TutorialUi::new().startup);
                self.tutorial.preferences.show_on_startup = true;
                self.tutorial.save_preferences();
                assert!(TutorialUi::new().startup);
                self.tutorial.preferences.show_on_startup = false;
                self.tutorial.save_preferences();
            }
            1 => {
                let revision = self.recovery.revision;
                self.add_point(rect.center() - egui::vec2(130.0, 90.0), 0.5, rect, true);
                self.add_point(rect.center() - egui::vec2(20.0, 70.0), 0.8, rect, false);
                self.finish();
                assert_eq!(self.doc.strokes.len(), 1);
                assert_eq!(self.recovery.revision, revision);
                let original = self.tutorial.practice.as_ref().unwrap();
                let source_path = original.file.clone().unwrap();
                let source_bytes = std::fs::read(&source_path).unwrap();
                self.file = Some(source_path.with_file_name("practice.sketch.json"));
                assert!(self.save(false));
                assert_eq!(std::fs::read(source_path).unwrap(), source_bytes);
                assert_eq!(self.recovery.revision, revision);
                self.undo();
                assert!(self.doc.strokes.is_empty());
                self.redo();
                for _ in 0..LESSONS.len() + 2 {
                    self.tutorial_step(1);
                }
                assert_eq!(self.tutorial.step, LESSONS.len() - 1);
                self.replay_tutorial();
                assert_eq!(self.tutorial.step, 0);
                assert!(!self.tutorial.offer);
                self.tutorial_step(LESSONS.len() as isize);
                for _ in 0..LESSONS.len() + 2 {
                    self.tutorial_step(-1);
                }
                assert_eq!(self.tutorial.step, 0);
                self.request(Pending::New);
                assert!(self.pending.is_none() && self.tutorial.practice.is_some());
                self.request(Pending::Close);
                assert!(self.confirm && self.tutorial.practice.is_some());
                self.confirm = false;
                self.pending = None;
                self.request(Pending::TutorialEnd);
                assert!(self.confirm);
                self.confirm = false;
                self.pending = None;
                assert_eq!(self.doc.strokes.len(), 1);
            }
            2 => {
                let p = self.tutorial.practice.as_ref().unwrap();
                let original = serde_json::to_string(&p.doc).unwrap();
                let snapshot: crate::recovery_store::RecoverySnapshot = serde_json::from_slice(
                    &std::fs::read(self.recovery.last_snapshot_path().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(serde_json::to_string(&snapshot.document).unwrap(), original);
                let selected = p.selected.clone();
                let history = (p.undo.len(), p.redo.len());
                let lighting = p.lighting;
                let file = p.file.clone();
                self.request(Pending::TutorialEnd);
                self.execute_pending();
                assert_eq!(serde_json::to_string(&self.doc).unwrap(), original);
                assert_eq!(self.selected, selected);
                assert_eq!((self.undo.len(), self.redo.len()), history);
                assert_eq!(self.file, file);
                assert_eq!(self.renderer.lighting, lighting);
                assert!(self.dirty && self.tutorial.practice.is_none());
                self.request(Pending::TutorialStart);
                self.execute_pending();
                self.request(Pending::Close);
                assert!(self.confirm && self.tutorial.practice.is_none());
                assert_eq!(serde_json::to_string(&self.doc).unwrap(), original);
                self.confirm = false;
                self.pending = None;
                self.request(Pending::TutorialStart);
                self.execute_pending();
                self.tutorial.step = 3;
                self.add_point(rect.center() - egui::vec2(160.0, 100.0), 0.4, rect, true);
                self.add_point(rect.center() - egui::vec2(60.0, 50.0), 0.8, rect, false);
                self.finish();
            }
            4 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            100 => panic!("Tutorial screenshot timed out"),
            _ => {}
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let path = std::env::current_dir().unwrap().join("verification");
            let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                path.join("tutorial-preview.png"),
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            self.end_practice();
            assert_eq!(self.doc.strokes.len(), 497);
            let report = serde_json::json!({"passed":true,"checks":["recovery precedence","first launch and replay preferences","entry cancel and unsaved guard","separate practice undo redo","practice does not mutate original recovery revision","Back Next and optional steps","exit cancel and exact document restoration","close restores original before unsaved prompt","GPU lesson and highlighted controls"],"gpu":self.renderer.adapter,"physical_tablet_tested":false});
            std::fs::write(
                path.join("tutorial-report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            self.dirty = false;
            self.recovery.discard();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.tutorial.verification_frame = Some(frame + 1);
    }
}
