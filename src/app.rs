use crate::surface::{Cylinder, GuidePreview, Surface, SurfaceStabilizer};
use crate::{
    document::{Brush, BrushKind, Document, Point, Stroke},
    geometry::{intersect_plane, Camera, DrawingPlane, Plane},
    input::{PenInput, PenSample, Phase},
    renderer::SceneRenderer,
};
use crate::{editing::Edit, gizmo::Mode};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, RichText, Vec2};
use glam::Vec3;
use std::collections::HashSet;
#[path = "brush_dynamics.rs"]
mod brush_dynamics;
#[path = "brush_textures.rs"]
mod brush_textures;
#[path = "brushes.rs"]
mod brushes;
#[path = "cylinders.rs"]
mod cylinders;
#[path = "export_ui.rs"]
mod export_ui;
#[path = "fills_checks.rs"]
mod fills_checks;
#[path = "fills_ui.rs"]
mod fills_ui;
#[path = "groups_ui.rs"]
mod groups_ui;
#[path = "layers_ui.rs"]
mod layers_ui;
#[path = "lighting_ui.rs"]
mod lighting_ui;
#[path = "line_checks.rs"]
mod line_checks;
#[path = "line_tool.rs"]
mod line_tool;
#[path = "palette_ui.rs"]
mod palette_ui;
#[path = "placement_checks.rs"]
mod placement_checks;
#[path = "placement_ui.rs"]
mod placement_ui;
#[path = "planes.rs"]
mod planes;
#[path = "png_export_ui.rs"]
mod png_export_ui;
#[path = "presentation_checks.rs"]
mod presentation_checks;
#[path = "presentation_ui.rs"]
mod presentation_ui;
#[path = "preset_checks.rs"]
mod preset_checks;
#[path = "recovery_ui.rs"]
mod recovery_ui;
#[path = "revolved_ui.rs"]
mod revolved_ui;
#[path = "selection.rs"]
mod selection;
#[path = "starter.rs"]
mod starter;
#[path = "tutorial_ui.rs"]
mod tutorial_ui;
#[path = "views.rs"]
mod views;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Pending {
    FilledStudy,
    TutorialStart,
    TutorialEnd,
    TutorialClose,
    New,
    House,
    Lantern,
    Brushes,
    Dynamics,
    Textures,
    Recover,
    Vase,
    Open,
    Close,
}

pub struct SketchApp {
    line: line_tool::LineUi,
    show_view: bool,
    view_verification_frame: Option<u32>,
    presentation: presentation_ui::PresentationUi,
    placement: placement_ui::PlacementUi,
    fills: fills_ui::FillUi,
    show_scene: bool,
    lighting_verification_frame: Option<u32>,
    tutorial: tutorial_ui::TutorialUi,
    renderer: SceneRenderer,
    palette_ui: palette_ui::PaletteUi,
    sampling_contact: bool,
    preset_ui: crate::presets_ui::PresetUi,
    recovery: crate::recovery::Recovery,
    recovery_prompt: bool,
    recovery_scan_seen: bool,
    recovery_to_restore: Option<crate::recovery_store::Candidate>,
    recovery_retire: Option<PathBuf>,
    recovered_source: Option<PathBuf>,
    close_after_recovery: bool,
    close_cleanup_failed: bool,
    allow_close: bool,
    recovery_verification_frame: Option<u32>,
    recovery_verify_stage: u8,
    recovery_verify_since: u32,
    recovery_verify_crash: bool,
    recovery_offer_capture: bool,
    recovery_offer_requested: bool,
    layer_name: String,
    layer_target: u64,
    layer_verification_frame: Option<u32>,
    group_verification_frame: Option<u32>,
    repeat_verification_frame: Option<u32>,
    repeat: Option<selection::RepeatState>,
    individual_selection: bool,
    pen: Option<PenInput>,
    input_status: String,
    doc: Document,
    camera: Camera,
    guide: DrawingPlane,
    cylinder: Cylinder,
    use_cylinder: bool,
    use_revolved: bool,
    revolved: crate::revolved::Revolved,
    vase_verification_frame: Option<u32>,
    guide_visible: bool,
    resume_ink: bool,
    color: [f32; 3],
    radius: f32,
    brush: Brush,
    stabilization: f32,
    stabilizer: SurfaceStabilizer,
    eraser: bool,
    select_mode: bool,
    selected: HashSet<u64>,
    transform_mode: Mode,
    transform_values: [f32; 3],
    scale_value: f32,
    drag: Option<selection::Drag>,
    active: Option<u64>,
    mouse_down: bool,
    next_id: u64,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    file: Option<PathBuf>,
    dirty: bool,
    status: String,
    last_pen: Option<PenSample>,
    last_pen_at: Option<Instant>,
    pen_previous: Option<Pos2>,
    samples: u64,
    last_batch: usize,
    pressure_history: std::collections::VecDeque<f32>,
    pending: Option<Pending>,
    confirm: bool,
    show_help: bool,
    frame_ms: f32,
    verification_frame: Option<u32>,
    brush_verification_frame: Option<u32>,
    dynamics_verification_frame: Option<u32>,
    texture_verification_frame: Option<u32>,
    preset_verification_frame: Option<u32>,
    house_verification_frame: Option<u32>,
    starter_scene: bool,
    cylinder_example: bool,
    cylinder_verification_frame: Option<u32>,
    export_job: Option<export_ui::ExportJob>,
    last_export_result: Option<Result<crate::export::ExportSummary, String>>,
    export_snapshot: Option<export_ui::ExportSnapshot>,
    png_export: png_export_ui::PngExportUi,
    png_verification_frame: Option<u32>,
}
impl SketchApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(20, 27, 37);
        style.visuals.window_fill = Color32::from_rgb(25, 34, 46);
        style.visuals.selection.bg_fill = Color32::from_rgb(27, 105, 111);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(32, 124, 132);
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        cc.egui_ctx.set_style(style);
        let (pen, input_status) = match PenInput::install(cc) {
            Ok(p) => (Some(p), "Windows Ink listener ready".into()),
            Err(e) => (None, e),
        };
        let renderer = SceneRenderer::new(
            cc.wgpu_render_state
                .clone()
                .expect("wgpu renderer required"),
        );
        let library_path = if std::env::args().any(|arg| arg.starts_with("--verify")) {
            std::env::current_dir()
                .map(|p| p.join("verification/preset-library/brushes.json"))
                .map_err(|e| e.to_string())
        } else {
            crate::presets::path()
        };
        let preset_ui = crate::presets_ui::PresetUi::new(
            cc.wgpu_render_state
                .clone()
                .expect("wgpu renderer required"),
            library_path,
        );
        let recovery_path = if std::env::args().any(|arg| arg.starts_with("--verify")) {
            std::env::current_dir()
                .map(|p| p.join("verification/recovery"))
                .map_err(|e| e.to_string())
        } else {
            crate::recovery_store::path()
        };
        let recovery = crate::recovery::Recovery::new(recovery_path, cc.egui_ctx.clone());
        let mut app = Self {
            line: Default::default(),
            show_view: false,
            view_verification_frame: None,
            presentation: presentation_ui::PresentationUi::new(),
            placement: Default::default(),
            fills: Default::default(),
            show_scene: false,
            lighting_verification_frame: None,
            tutorial: tutorial_ui::TutorialUi::new(),
            renderer,
            palette_ui: palette_ui::PaletteUi::new(),
            sampling_contact: false,
            preset_ui,
            recovery,
            recovery_prompt: false,
            recovery_scan_seen: false,
            recovery_to_restore: None,
            recovery_retire: None,
            recovered_source: None,
            close_after_recovery: false,
            close_cleanup_failed: false,
            allow_close: false,
            recovery_verification_frame: None,
            recovery_verify_stage: 0,
            recovery_verify_since: 0,
            recovery_verify_crash: false,
            recovery_offer_capture: false,
            recovery_offer_requested: false,
            layer_name: String::new(),
            layer_target: 1,
            layer_verification_frame: None,
            group_verification_frame: None,
            repeat_verification_frame: None,
            repeat: None,
            individual_selection: false,
            pen,
            input_status,
            doc: Document::default(),
            camera: Camera::default(),
            guide: DrawingPlane::default(),
            cylinder: Cylinder::default(),
            use_cylinder: false,
            use_revolved: false,
            revolved: Default::default(),
            vase_verification_frame: None,
            guide_visible: true,
            resume_ink: false,
            color: [0.17, 0.82, 0.74],
            radius: 0.025,
            brush: Brush::default(),
            stabilization: 0.35,
            stabilizer: SurfaceStabilizer::default(),
            eraser: false,
            select_mode: false,
            selected: HashSet::new(),
            transform_mode: Mode::Move,
            transform_values: [0.0; 3],
            scale_value: 1.0,
            drag: None,
            active: None,
            mouse_down: false,
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            file: None,
            dirty: false,
            status: "Ready. Draw on the highlighted plane.".into(),
            last_pen: None,
            last_pen_at: None,
            pen_previous: None,
            samples: 0,
            last_batch: 0,
            pressure_history: Default::default(),
            pending: None,
            confirm: false,
            show_help: false,
            frame_ms: 0.0,
            verification_frame: std::env::args().any(|a| a == "--verify").then_some(0),
            brush_verification_frame: None,
            dynamics_verification_frame: None,
            texture_verification_frame: None,
            preset_verification_frame: None,
            house_verification_frame: None,
            starter_scene: false,
            cylinder_example: false,
            cylinder_verification_frame: None,
            export_job: None,
            last_export_result: None,
            export_snapshot: None,
            png_export: Default::default(),
            png_verification_frame: None,
        };
        if std::env::args().any(|a| a == "--verify-lines") {
            app.verification_frame = None;
            app.line.verification_frame = Some(0);
            app.load_house();
        } else if std::env::args().any(|a| a == "--verify-views") {
            app.verification_frame = None;
            app.view_verification_frame = Some(0);
            app.load_house();
        } else if std::env::args().any(|a| a == "--verify-presentation") {
            app.verification_frame = None;
            app.load_house();
        } else if std::env::args().any(|a| a == "--verify-placement") {
            app.verification_frame = None;
            app.placement.verification_frame = Some(0);
            app.load_fill_study();
        } else if std::env::args().any(|a| a == "--verify-fills") {
            app.verification_frame = None;
            app.fills.verification_frame = Some(0);
            app.load_fill_study();
        } else if std::env::args().any(|a| a == "--verify-lighting") {
            app.verification_frame = None;
            app.lighting_verification_frame = Some(0);
            app.load_house();
        } else if std::env::args().any(|a| a == "--verify-tutorial") {
            app.verification_frame = None;
            app.load_house();
        } else if std::env::args().any(|a| a == "--verify-groups") {
            app.verification_frame = None;
            app.group_verification_frame = Some(0);
            app.load_house();
        } else if std::env::args().any(|a| a == "--verify-repeat") {
            app.verification_frame = None;
            app.repeat_verification_frame = Some(0);
            app.load_fill_study();
        } else if std::env::args().any(|a| a == "--verify-palette") {
            app.verification_frame = None;
            app.load_fill_study();
        } else if std::env::args().any(|a| a == "--verify-png") {
            app.verification_frame = None;
            app.load_fill_study();
            app.png_verification_frame = Some(0);
        } else if std::env::args().any(|a| a == "--verify-vase") {
            app.verification_frame = None;
            app.vase_verification_frame = Some(0);
            app.load_vase();
        } else if std::env::args().any(|a| a == "--verify-layers") {
            app.verification_frame = None;
            app.layer_verification_frame = Some(0);
            app.load_textures();
        } else if std::env::args()
            .any(|a| a == "--verify-recovery-crash" || a == "--verify-recovery")
        {
            app.verification_frame = None;
            app.recovery_verification_frame = Some(0);
            app.recovery_verify_crash = std::env::args().any(|a| a == "--verify-recovery-crash");
            if app.recovery_verify_crash {
                app.load_textures();
            } else {
                app.load_house();
            }
        } else if std::env::args().any(|a| a == "--verify-presets") {
            app.verification_frame = None;
            app.preset_verification_frame = Some(0);
            app.load_textures();
        } else if std::env::args().any(|a| a == "--verify-textures") {
            app.verification_frame = None;
            app.texture_verification_frame = Some(0);
            app.load_textures();
        } else if std::env::args().any(|a| a == "--verify-dynamics") {
            app.verification_frame = None;
            app.dynamics_verification_frame = Some(0);
            app.load_dynamics();
        } else if std::env::args().any(|a| a == "--verify-brushes") {
            app.verification_frame = None;
            app.brush_verification_frame = Some(0);
            app.load_brushes();
        } else if std::env::args().any(|a| a == "--verify-cylinder") {
            app.verification_frame = None;
            app.cylinder_verification_frame = Some(0);
            app.load_lantern();
        } else if std::env::args().any(|a| a == "--verify-house") {
            app.verification_frame = None;
            app.house_verification_frame = Some(0);
            app.load_house();
        } else if app.verification_frame.is_none() {
            if std::env::args().any(|a| a == "--vase") {
                app.load_vase();
            } else if std::env::args().any(|a| a == "--textures") {
                app.load_textures();
            } else if std::env::args().any(|a| a == "--dynamics") {
                app.load_dynamics();
            } else if std::env::args().any(|a| a == "--brushes") {
                app.load_brushes();
            } else if std::env::args().any(|a| a == "--demo") {
                app.demo();
            } else if std::env::args().any(|a| a == "--lantern") {
                app.load_lantern();
            } else if !std::env::args().any(|a| a == "--blank") {
                app.load_house();
            }
        }
        app.recovery.request_scan();
        app
    }
    fn finish(&mut self) {
        self.cancel_line();
        self.resume_ink = false;
        self.commit_transform();
        let tail = self.stabilizer.finish();
        if let Some(id) = self.active {
            if let Some(stroke) = self.doc.strokes.last_mut().filter(|s| s.id == id) {
                for point in tail {
                    if stroke.points.last().is_none_or(|p| {
                        p.position != point.position
                            || p.pressure != point.pressure
                            || p.tilt != point.tilt
                    }) {
                        stroke.points.push(point);
                    }
                }
            }
        }
        if let Some(id) = self.active.take() {
            if let Some(s) = self.doc.strokes.last().filter(|s| s.id == id) {
                self.undo.push(Edit::Add(s.clone()));
                self.redo.clear();
                self.dirty = true;
                self.record_recovery_edit();
            }
        }
    }
    fn cancel(&mut self) {
        self.cancel_line();
        self.palette_ui.sampling = false;
        self.sampling_contact = false;
        self.resume_ink = false;
        self.cancel_transform();
        self.stabilizer.reset();
        self.mouse_down = false;
        if let Some(id) = self.active.take() {
            self.doc.strokes.retain(|s| s.id != id);
        }
    }
    fn add_point(&mut self, position: Pos2, pressure: f32, rect: Rect, begin: bool) {
        self.add_point_with_tilt(position, pressure, rect, begin, None);
    }
    fn add_point_with_tilt(
        &mut self,
        position: Pos2,
        pressure: f32,
        rect: Rect,
        begin: bool,
        tilt: Option<Vec3>,
    ) {
        if begin {
            self.finish();
        }
        if !self.doc.active_editable() {
            self.status =
                "The active layer is hidden or locked. Choose an editable layer to draw.".into();
            return;
        }
        let ndc = glam::vec2(
            (position.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (position.y - rect.top()) / rect.height() * 2.0,
        );
        let (origin, direction) = self.camera.ray(ndc, rect.aspect_ratio());
        let hit = if self.use_revolved {
            self.revolved.hit(origin, direction)
        } else if self.use_cylinder {
            self.cylinder.hit(origin, direction)
        } else {
            intersect_plane(origin, direction, self.guide)
        };
        let Some(world) = hit else {
            let resume = begin || self.active.is_some() || self.resume_ink;
            self.finish();
            self.resume_ink = resume;
            self.status=if self.use_cylinder || self.use_revolved{"No curved guide side hit. Draw on its wireframe or orbit; missed regions split the stroke."}else{"Plane is edge-on. Use Align view or rotate the camera."}.into();
            return;
        };
        if world.abs().max_element() > 10000.0 {
            return;
        }
        if self.active.is_some() && self.stabilizer.discontinuity(world) {
            self.finish();
            self.resume_ink = true;
        }
        if begin || (self.resume_ink && self.active.is_none()) {
            self.finish();
            if self.next_id == u64::MAX {
                self.status = "No more stroke IDs available".into();
                return;
            }
            let id = self.next_id;
            self.next_id += 1;
            self.active = Some(id);
            self.stabilizer.start(if self.use_revolved {
                Surface::Revolved(self.revolved)
            } else if self.use_cylinder {
                Surface::Cylinder(self.cylinder)
            } else {
                Surface::Plane
            });
            self.doc.strokes.push(Stroke {
                fill: None,
                group_id: None,
                layer_id: self.doc.active_layer,
                id,
                color: self.color,
                radius: self.radius,
                brush: Brush {
                    normal: if self.use_revolved {
                        self.revolved.normal(world)
                    } else if self.use_cylinder {
                        let local = self.cylinder.to_local(world);
                        self.cylinder.rotation()
                            * Vec3::new(local.x, 0.0, local.z).normalize_or_zero()
                    } else {
                        self.guide.basis().2
                    },
                    tangent: if self.use_revolved {
                        self.revolved.tangent(world)
                    } else if self.use_cylinder {
                        let local = self.cylinder.to_local(world);
                        self.cylinder.rotation()
                            * Vec3::new(local.z, 0.0, -local.x).normalize_or_zero()
                    } else {
                        self.guide.basis().0
                    },
                    ..self.brush
                },
                points: Vec::new(),
            });
        }
        let Some(id) = self.active else {
            return;
        };
        let pressure = pressure.clamp(0.0, 1.0);
        let world_per_pixel = self.camera.world_per_pixel(world, rect.height());
        let points = self.stabilizer.push(
            Point {
                tilt,
                position: world,
                pressure,
            },
            self.stabilization * 24.0 * world_per_pixel,
        );
        if let Some(stroke) = self.doc.strokes.last_mut().filter(|s| s.id == id) {
            for point in points {
                if stroke.points.last().is_none_or(|p| {
                    p.position.distance(point.position) > 0.001
                        || (p.pressure - point.pressure).abs() > 0.025
                        || p.tilt != point.tilt
                }) {
                    stroke.points.push(point);
                }
            }
        }
    }
    fn erase(&mut self, position: Pos2, rect: Rect) {
        self.finish();
        let picked = crate::gizmo::pick(&self.doc, &self.camera, rect, position);
        let index = self
            .doc
            .strokes
            .iter()
            .position(|stroke| Some(stroke.id) == picked);
        if let Some(i) = index {
            let stroke = self.doc.strokes.remove(i);
            self.undo.push(Edit::Remove(i, stroke));
            self.redo.clear();
            self.dirty = true;
            self.record_recovery_edit();
        }
    }
    fn undo(&mut self) {
        if self.fills.draft.is_some() {
            self.remove_fill_corner();
            return;
        }
        self.finish();
        if let Some(edit) = self.undo.pop() {
            edit.apply(&mut self.doc, false);
            self.redo.push(edit);
            self.dirty = true;
            self.record_recovery_edit();
            self.renderer.invalidate();
            self.sync_selection();
        }
    }
    fn redo(&mut self) {
        if self.fills.draft.is_some() {
            self.status = "Finish or cancel the fill contour before redoing document edits.".into();
            return;
        }
        self.finish();
        if let Some(edit) = self.redo.pop() {
            edit.apply(&mut self.doc, true);
            self.undo.push(edit);
            self.dirty = true;
            self.record_recovery_edit();
            self.renderer.invalidate();
            self.sync_selection();
        }
    }
    fn save(&mut self, save_as: bool) -> bool {
        if self.line.draft.is_some() {
            self.status = "Release or cancel the line preview before saving.".into();
            return false;
        }
        self.cancel_placement();
        if self.fills.draft.is_some() {
            self.status = "Close or cancel the draft contour before saving.".into();
            return false;
        }
        self.finish();
        let path = if save_as || self.file.is_none() {
            rfd::FileDialog::new()
                .add_filter("Spatial Sketch", &["json"])
                .set_file_name("drawing.sketch.json")
                .save_file()
        } else {
            self.file.clone()
        };
        let Some(path) = path else {
            return false;
        };
        if self.reserved_recovery_destination(&path) {
            self.leave_artwork();
            self.status = "Choose a native save location outside the recovery folder.".into();
            return false;
        }
        let result = (|| -> Result<(), String> {
            let bytes = serde_json::to_vec_pretty(&self.doc).map_err(|e| e.to_string())?;
            let parent = path.parent().ok_or("Invalid save location")?;
            let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            use std::io::Write;
            temp.write_all(&bytes).map_err(|e| e.to_string())?;
            temp.as_file().sync_all().map_err(|e| e.to_string())?;
            temp.persist(&path).map_err(|e| e.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                if self.tutorial.practice.is_none() {
                    self.close_cleanup_failed = false;
                    self.recovery.mark_saved();
                    self.retire_recovered_source();
                    self.recovered_source = None;
                }
                self.status = format!("Saved {}", path.display());
                self.file = Some(path);
                self.dirty = false;
                true
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                self.leave_artwork();
                false
            }
        }
    }
    fn request(&mut self, mut pending: Pending) {
        if self.line.draft.is_some() {
            self.status = "Release or cancel the line preview before switching drawings.".into();
            return;
        }
        self.leave_artwork();
        self.cancel_placement();
        if self.fills.draft.is_some() {
            self.status =
                "Close or cancel the draft contour before switching drawings or leaving practice."
                    .into();
            return;
        }
        self.finish();
        if self.tutorial.practice.is_some()
            && !matches!(
                pending,
                Pending::TutorialEnd | Pending::TutorialClose | Pending::Close
            )
        {
            self.status = "Exit practice before opening another drawing or recovery copy.".into();
            return;
        }
        if self.tutorial.practice.is_some() && matches!(pending, Pending::Close) {
            pending = Pending::TutorialClose;
        }
        self.pending = Some(pending);
        if self.dirty {
            self.confirm = true;
        } else {
            self.execute_pending();
        }
    }
    fn execute_pending(&mut self) {
        self.confirm = false;
        match self.pending.take() {
            Some(Pending::FilledStudy) => self.load_fill_study(),
            Some(Pending::TutorialStart) => self.start_practice(),
            Some(Pending::TutorialEnd) => self.end_practice(),
            Some(Pending::TutorialClose) => {
                self.end_practice();
                self.request(Pending::Close);
            }
            Some(Pending::House) => self.load_house(),
            Some(Pending::Lantern) => self.load_lantern(),
            Some(Pending::Brushes) => self.load_brushes(),
            Some(Pending::Dynamics) => self.load_dynamics(),
            Some(Pending::Textures) => self.load_textures(),
            Some(Pending::New) => {
                self.reset_recovery_document();
                self.starter_scene = false;
                self.cylinder_example = false;
                self.use_cylinder = false;
                self.use_revolved = false;
                self.guide_visible = true;
                self.next_id = 1;
                self.camera = Camera::default();
                self.guide = DrawingPlane::default();
                self.doc = Document::default();
                self.file = None;
                self.dirty = false;
                self.undo.clear();
                self.redo.clear();
                self.selected.clear();
                self.renderer.invalidate();
                self.status = "New drawing".into();
            }
            Some(Pending::Open) => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Spatial Sketch", &["json"])
                    .pick_file()
                {
                    let result = (|| -> Result<Document, String> {
                        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                        if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
                            return Err("Document exceeds the prototype's 64 MB limit".into());
                        }
                        use std::io::Read;
                        let mut text = String::new();
                        file.read_to_string(&mut text).map_err(|e| e.to_string())?;
                        Document::from_json(&text)
                    })();
                    match result {
                        Ok(doc) => {
                            self.reset_recovery_document();
                            self.next_id = doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
                            self.doc = doc;
                            self.starter_scene = false;
                            self.cylinder_example = false;
                            self.file = Some(path);
                            self.dirty = false;
                            self.undo.clear();
                            self.redo.clear();
                            self.selected.clear();
                            self.renderer.invalidate();
                            self.status = "Drawing opened".into();
                        }
                        Err(e) => self.status = format!("Open failed: {e}"),
                    }
                }
            }
            Some(Pending::Recover) => self.install_recovery(),
            Some(Pending::Vase) => self.load_vase(),
            Some(Pending::Close) => self.approve_recovery_close(),
            None => {}
        }
    }
    fn demo(&mut self) {
        for j in 0..3 {
            let mut points = Vec::new();
            for i in 0..160 {
                let t = i as f32 / 159.0 * std::f32::consts::TAU;
                let pos = match j {
                    0 => Vec3::new(t.cos() * 1.6, t.sin() * 1.6, 0.0),
                    1 => Vec3::new(0.0, t.sin() * 1.6, t.cos() * 1.6),
                    _ => Vec3::new(t.cos() * 1.6, 0.0, t.sin() * 1.6),
                };
                points.push(Point {
                    tilt: None,
                    position: pos,
                    pressure: 0.5 + 0.5 * (t * 2.0).sin().abs(),
                });
            }
            self.doc.strokes.push(Stroke {
                fill: None,
                group_id: None,
                layer_id: self.doc.active_layer,
                id: self.next_id,
                color: [[0.17, 0.82, 0.74], [0.96, 0.55, 0.30], [0.47, 0.58, 0.95]][j],
                radius: 0.035,
                brush: Brush::default(),
                points,
            });
            self.next_id += 1;
        }
        self.status = "Demo scene. New starts a blank drawing.".into();
    }
    fn tools(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(
            RichText::new("WORKSPACE")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.heading("Draw in space");
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(
                    !self.eraser && !self.select_mode && !self.fills.enabled && !self.line.enabled,
                    "Brush [B]",
                )
                .clicked()
            {
                self.finish();
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.eraser = false;
                self.select_mode = false;
            }
            if ui
                .selectable_label(self.eraser && !self.select_mode, "Erase [E]")
                .clicked()
            {
                self.finish();
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.eraser = true;
                self.select_mode = false;
            }
            if ui
                .selectable_label(self.select_mode, "Select [V]")
                .clicked()
            {
                self.finish();
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.eraser = false;
                self.select_mode = true;
            }
        });
        if ui
            .selectable_label(self.fills.enabled && !self.select_mode, "Planar fill [P]")
            .clicked()
        {
            self.finish();
            self.cancel_line_tool();
            self.fills.enabled = true;
            self.eraser = false;
            self.select_mode = false;
        }
        if ui
            .selectable_label(self.line.enabled && !self.select_mode, "Straight line [L]")
            .clicked()
        {
            self.start_line_tool();
        }
        self.line_controls(ui);
        self.fill_controls(ui);
        ui.add_space(6.0);
        if self.select_mode {
            self.group_controls(ui);
        }
        self.tutorial_hint(ui, "Drawing layers");
        self.layer_controls(ui);
        match self.palette_ui.controls(ui, &mut self.color) {
            palette_ui::PaletteAction::StartSampling => self.start_color_sampling(),
            palette_ui::PaletteAction::CancelSampling => self.cancel_color_sampling(),
            palette_ui::PaletteAction::None => {}
        }
        self.tutorial_hint(ui, "Brush library");
        if !self.fills.enabled {
            self.preset_controls(ui);
        }
        if self.select_mode {
            self.tutorial_hint(ui, "Select");
            self.selection_panel(ui);
        } else if !self.fills.enabled {
            if self.eraser {
                ui.label("Eraser removes whole strokes.");
            }
            self.tutorial_hint(ui, "Brush");
            if !self.fills.enabled {
                self.brush_controls(ui);
            }
            ui.label("INK COLOR");
            ui.color_edit_button_rgb(&mut self.color);
            ui.horizontal_wrapped(|ui| {
                for color in [
                    [0.17, 0.82, 0.74],
                    [0.96, 0.55, 0.30],
                    [0.47, 0.58, 0.95],
                    [0.94, 0.80, 0.39],
                    [0.92, 0.94, 0.97],
                ] {
                    let fill = Color32::from_rgb(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    );
                    if ui.add(egui::Button::new("  ").fill(fill)).clicked() {
                        self.color = color;
                    }
                }
            });
            ui.add(
                egui::Slider::new(&mut self.radius, 0.001..=1.0)
                    .text(if self.brush.kind == BrushKind::Round {
                        "Radius"
                    } else {
                        "Half width"
                    })
                    .logarithmic(true),
            );
            ui.add(egui::Slider::new(&mut self.stabilization, 0.0..=1.0).text("Stabilize").custom_formatter(|v, _| format!("{:.0}%", v*100.0))).on_hover_text("0% gives raw input. Higher values reduce jitter and let the line trail the pen. It catches up on release; pressure is preserved.");
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label(
            RichText::new("DRAWING GUIDE")
                .small()
                .color(Color32::from_gray(140)),
        );
        self.tutorial_hint(ui, "Drawing guide");
        self.plane_controls(ui);
        if ui.button("Reset camera").clicked() {
            self.finish();
            self.camera = Camera::default();
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label(
            RichText::new("NAVIGATION")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.label("Space + drag    Orbit\nSpace + Shift    Pan\nSpace + Alt       Zoom\nRight drag        Orbit\nMiddle drag      Pan\nScroll                Zoom");
        self.tutorial_hint(ui, "Navigation");
        if ui.button("All shortcuts / help").clicked() {
            self.finish();
            self.show_help = true;
        }
    }
    fn diagnostics(&mut self, ui: &mut egui::Ui) {
        self.tutorial_hint(ui, "Input lab");
        ui.add_space(8.0);
        ui.label(
            RichText::new("INPUT LAB")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.heading("Tablet diagnostics");
        ui.label(&self.input_status);
        ui.add_space(12.0);
        let recent = self
            .last_pen_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
        let label = if recent {
            "PEN DETECTED"
        } else {
            "WAITING FOR PEN"
        };
        ui.label(RichText::new(label).color(if recent {
            Color32::from_rgb(76, 220, 190)
        } else {
            Color32::from_gray(150)
        }));
        if let Some(p) = self.last_pen {
            match p.pressure {
                Some(v) => {
                    ui.add(egui::ProgressBar::new(v).text(format!("Pressure  {:.1}%", v * 100.0)));
                }
                None => {
                    ui.label("Pressure: not reported");
                }
            }
            ui.label(format!(
                "Tilt X: {}\nTilt Y: {}",
                p.tilt[0]
                    .map(|v| format!("{v:.0} degrees"))
                    .unwrap_or("not reported".into()),
                p.tilt[1]
                    .map(|v| format!("{v:.0} degrees"))
                    .unwrap_or("not reported".into())
            ));
            ui.label(format!(
                "Contact: {}\nBarrel button: {}\nEraser: {}",
                p.contact, p.barrel, p.eraser
            ));
        } else {
            ui.label("Touch the tablet with your pen.\nMouse strokes use fixed width;\nthey do not count as pen input.");
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 70.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 4.0, Color32::from_rgb(13, 20, 29));
        let points: Vec<Pos2> = self
            .pressure_history
            .iter()
            .enumerate()
            .map(|(i, p)| {
                egui::pos2(
                    rect.left() + i as f32 / 179.0 * rect.width(),
                    rect.bottom() - p * rect.height(),
                )
            })
            .collect();
        if points.len() > 1 {
            ui.painter().add(egui::Shape::line(
                points,
                egui::Stroke::new(1.5_f32, Color32::from_rgb(76, 220, 190)),
            ));
        }
        ui.label(format!(
            "Pen samples: {}\nLast batch: {}",
            self.samples, self.last_batch
        ));
        ui.add_space(12.0);
        ui.separator();
        ui.label(
            RichText::new("SCENE")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.label(format!(
            "{} strokes\n{} control points",
            self.doc.strokes.len(),
            self.doc
                .strokes
                .iter()
                .map(|s| s.points.len())
                .sum::<usize>()
        ));
        ui.label(format!("UI + canvas CPU: {:.2} ms", self.frame_ms));
        ui.label(RichText::new(&self.renderer.adapter).small());
        ui.add_space(12.0);
        ui.separator();
        ui.label("Windows tablet setup");
        ui.label(
            "Enable Windows Ink in your tablet driver. Map the tablet to the intended monitor.",
        );
        ui.label("Wintab and native macOS / Linux pen backends are future work.");
        ui.separator();
        ui.label("RECOVERY");
        ui.label(&self.recovery.notice);
    }
    fn navigate(&mut self, delta: Vec2, pan: bool, zoom: bool, rect: Rect) {
        self.finish();
        if zoom {
            self.camera.distance =
                (self.camera.distance * (delta.y * 0.012).exp()).clamp(0.3, 150.0);
        } else if pan {
            self.camera.pan(glam::vec2(delta.x, delta.y), rect.height());
        } else {
            self.camera.yaw -= delta.x * 0.008;
            self.camera.pitch = (self.camera.pitch + delta.y * 0.008).clamp(-1.55, 1.55);
        }
    }
    fn pointer_action(&mut self, pos: Pos2, pressure: f32, rect: Rect, begin: bool, shift: bool) {
        self.pointer_action_with_tilt(pos, pressure, rect, begin, shift, None);
    }

    fn start_color_sampling(&mut self) {
        self.finish();
        self.cancel_placement();
        self.cancel_line_tool();
        self.cancel_fill_tool();
        self.select_mode = false;
        self.eraser = false;
        self.sampling_contact = false;
        self.palette_ui.sampling = true;
        self.status = "Color sample active. Click visible artwork or Escape to cancel.".into();
    }

    fn cancel_color_sampling(&mut self) {
        self.palette_ui.sampling = false;
        self.select_mode = false;
        self.eraser = false;
        self.line.enabled = false;
        self.fills.enabled = false;
        self.status = "Color sampling cancelled. Brush ready.".into();
    }
    fn pointer_action_with_tilt(
        &mut self,
        pos: Pos2,
        pressure: f32,
        rect: Rect,
        begin: bool,
        shift: bool,
        tilt: Option<Vec3>,
    ) {
        if self.presentation.artwork {
            return;
        }
        if self.sampling_contact {
            return;
        }
        if self.palette_ui.sampling {
            if begin {
                self.sampling_contact = true;
                let sampled = crate::gizmo::pick_visible(&self.doc, &self.camera, rect, pos)
                    .and_then(|id| self.doc.strokes.iter().find(|stroke| stroke.id == id))
                    .map(|stroke| stroke.color);
                if let Some(color) = sampled {
                    self.color = color;
                    self.palette_ui.sampling = false;
                    self.status = "Artwork color sampled. Ready to draw with it.".into();
                } else {
                    self.status =
                        "No visible artwork at that point. Click a stroke or fill.".into();
                }
            }
            return;
        }
        if self.placement.active {
            if begin {
                self.pick_placement(pos, rect);
            }
        } else if self.select_mode {
            self.select_pointer(pos, rect, begin, shift);
        } else if self.eraser {
            self.erase(pos, rect);
        } else if self.line.enabled {
            self.line_input(pos, pressure, tilt, rect, begin);
        } else if self.fills.enabled {
            if begin {
                self.fill_corner(pos, rect);
            }
        } else {
            self.add_point_with_tilt(pos, pressure, rect, begin, tilt);
        }
    }
    fn canvas_input(&mut self, ctx: &egui::Context, rect: Rect, mut pen: Vec<PenSample>) {
        let (space, shift, alt, events, pointer, scroll) = ctx.input(|i| {
            (
                i.key_down(Key::Space),
                i.modifiers.shift,
                i.modifiers.alt,
                i.events.clone(),
                i.pointer.clone(),
                i.smooth_scroll_delta.y,
            )
        });
        if !ctx.input(|i| i.focused)
            && self.verification_frame.is_none()
            && self.dynamics_verification_frame.is_none()
            && self.texture_verification_frame.is_none()
            && self.preset_verification_frame.is_none()
            && self.recovery_verification_frame.is_none()
            && self.layer_verification_frame.is_none()
            && self.vase_verification_frame.is_none()
            && self.group_verification_frame.is_none()
            && self.tutorial.verification_frame.is_none()
            && self.lighting_verification_frame.is_none()
            && self.fills.verification_frame.is_none()
            && self.placement.verification_frame.is_none()
            && self.presentation.verification_frame.is_none()
            && self.view_verification_frame.is_none()
            && self.line.verification_frame.is_none()
        {
            self.cancel();
            return;
        }
        let blocked = self.confirm
            || self.show_help
            || self.tutorial.offer
            || self.recovery_prompt
            || self.close_after_recovery;
        let inside_canvas = |p| {
            rect.contains(p)
                && ctx
                    .layer_id_at(p)
                    .is_none_or(|layer| layer.order == egui::Order::Background)
        };
        let scale = ctx.pixels_per_point();
        for sample in &mut pen {
            sample.position.x /= scale;
            sample.position.y /= scale;
        }
        self.last_batch = pen.len();
        for sample in pen {
            if sample.phase == Phase::Cancel {
                self.cancel();
                self.pen_previous = None;
                continue;
            }
            self.last_pen = Some(sample);
            self.last_pen_at = Some(Instant::now());
            self.samples += 1;
            if let Some(p) = sample.pressure {
                self.pressure_history.push_back(p);
                if self.pressure_history.len() > 180 {
                    self.pressure_history.pop_front();
                }
            }
            let inside = inside_canvas(sample.position);
            let previous = self.pen_previous.replace(sample.position);
            if blocked {
                self.finish();
                continue;
            }
            if sample.phase == Phase::Up {
                self.sampling_contact = false;
                if self.line.draft.is_some() {
                    if inside {
                        self.release_line(sample.position, rect);
                    } else {
                        self.cancel_line();
                    }
                }
                if self.drag.is_some() {
                    self.select_pointer(sample.position, rect, false, shift);
                }
                self.finish();
                continue;
            }
            if !inside && self.drag.is_none() {
                self.finish();
                continue;
            }
            if space || sample.barrel {
                if sample.contact {
                    if let Some(prev) = previous {
                        self.navigate(sample.position - prev, shift, alt, rect);
                    }
                }
                continue;
            }
            if sample.contact {
                if sample.eraser && !self.placement.active && !self.presentation.artwork {
                    self.erase(sample.position, rect);
                } else {
                    self.pointer_action_with_tilt(
                        sample.position,
                        sample.pressure.unwrap_or(1.0),
                        rect,
                        sample.phase == Phase::Down,
                        shift,
                        crate::dynamics::screen_tilt(
                            sample.tilt,
                            self.camera.basis().0,
                            self.camera.basis().1,
                        ),
                    );
                }
            }
        }
        if blocked {
            return;
        }
        let native_recent = self
            .last_pen_at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(200));
        if native_recent {
            self.mouse_down = false;
        }
        if !native_recent {
            let nav = pointer.button_down(PointerButton::Secondary)
                || pointer.button_down(PointerButton::Middle)
                || (space && pointer.primary_down());
            if nav && pointer.interact_pos().is_some_and(inside_canvas) {
                self.mouse_down = false;
                self.navigate(
                    pointer.delta(),
                    shift || pointer.button_down(PointerButton::Middle),
                    alt,
                    rect,
                );
            }
            if !nav && !space {
                for event in events {
                    match event {
                        egui::Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers,
                        } if inside_canvas(pos) => {
                            self.mouse_down = true;
                            self.pointer_action(pos, 1.0, rect, true, modifiers.shift);
                        }
                        egui::Event::PointerMoved(pos) if self.mouse_down => {
                            if inside_canvas(pos) || (self.drag.is_some() && !rect.contains(pos)) {
                                self.pointer_action(pos, 1.0, rect, false, shift);
                            } else {
                                self.mouse_down = false;
                                self.finish();
                            }
                        }
                        egui::Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            ..
                        } => {
                            self.sampling_contact = false;
                            if self.drag.is_some() {
                                self.select_pointer(pos, rect, false, shift);
                            }
                            self.mouse_down = false;
                            if self.line.draft.is_some() {
                                if inside_canvas(pos) {
                                    self.release_line(pos, rect);
                                } else {
                                    self.cancel_line();
                                }
                            }
                            self.finish();
                        }
                        egui::Event::PointerGone => {
                            self.mouse_down = false;
                            self.sampling_contact = false;
                            self.finish();
                        }
                        _ => {}
                    }
                }
            }
        }
        if pointer.hover_pos().is_some_and(inside_canvas) && scroll != 0.0 {
            self.finish();
            self.camera.distance =
                (self.camera.distance * (-scroll * 0.002).exp()).clamp(0.3, 150.0);
        }
        self.sync_selection();
    }
}
impl eframe::App for SketchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let start = Instant::now();
        self.poll_recovery(ctx);
        self.ensure_workspace();
        self.poll_export(ctx);
        if let Some(status) = self.png_export.poll(ctx) {
            self.status = status;
        }
        self.capture_verification(ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.leave_artwork();
        }
        if self.palette_ui.sampling && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.cancel_color_sampling();
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && (self.export_job.is_some() || self.png_export.is_running())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "An image export is finishing. Close again when it completes.".into();
        } else if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.close_cleanup_failed {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            self.finish();
            self.request(Pending::Close);
        }
        let was_artwork = self.presentation.artwork;
        if !self.confirm
            && !self.show_help
            && !self.recovery_prompt
            && !self.tutorial.offer
            && !self.close_after_recovery
            && self.repeat.is_none()
            && !self.palette_ui.sampling
            && !ctx.wants_keyboard_input()
        {
            self.view_shortcuts(ctx);
            self.presentation_shortcuts(ctx);
        }
        let was_placing = self.placement.active;
        if !self.confirm
            && !self.show_help
            && !self.recovery_prompt
            && !self.tutorial.offer
            && !self.close_after_recovery
            && self.repeat.is_none()
            && !self.palette_ui.sampling
            && !ctx.wants_keyboard_input()
            && !self.presentation.artwork
            && !was_artwork
        {
            self.placement_shortcuts(ctx);
        }
        if !self.confirm
            && !self.placement.active
            && !was_placing
            && !self.presentation.artwork
            && !was_artwork
            && !self.show_help
            && !self.tutorial.offer
            && !self.recovery_prompt
            && !self.close_after_recovery
            && self.repeat.is_none()
            && !self.palette_ui.sampling
            && !ctx.wants_keyboard_input()
        {
            self.line_shortcuts(ctx);
            self.fill_shortcuts(ctx);
            self.group_shortcuts(ctx);
            self.selection_shortcuts(ctx);
            let (mods, b, e, f, i_key, z, y, s, o, n) = ctx.input(|i| {
                (
                    i.modifiers,
                    i.key_pressed(Key::B),
                    i.key_pressed(Key::E),
                    i.key_pressed(Key::F),
                    i.key_pressed(Key::I),
                    i.key_pressed(Key::Z),
                    i.key_pressed(Key::Y),
                    i.key_pressed(Key::S),
                    i.key_pressed(Key::O),
                    i.key_pressed(Key::N),
                )
            });
            if mods.command {
                if z {
                    if mods.shift {
                        self.redo()
                    } else {
                        self.undo()
                    }
                }
                if y {
                    self.redo()
                }
                if s {
                    self.save(mods.shift);
                }
                if o {
                    self.request(Pending::Open)
                }
                if n {
                    self.request(Pending::New)
                }
            } else {
                if i_key {
                    self.start_color_sampling();
                }
                if b {
                    self.cancel_line_tool();
                    self.cancel_fill_tool();
                    self.finish();
                    self.eraser = false;
                    self.select_mode = false;
                }
                if e {
                    self.cancel_line_tool();
                    self.cancel_fill_tool();
                    self.finish();
                    self.eraser = true;
                    self.select_mode = false;
                }
                if f {
                    self.finish();
                    if mods.shift {
                        self.plane_from_view();
                    } else {
                        self.align_active_guide();
                    }
                }
            }
        }
        self.ensure_workspace();
        if !self.presentation.artwork {
            egui::TopBottomPanel::top("header").show(ctx, |ui| {
                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SPATIAL / SKETCH")
                            .strong()
                            .size(20.0)
                            .color(Color32::from_rgb(99, 229, 207)),
                    );
                    ui.label(
                        RichText::new("0.1  /  RUST PROTOTYPE")
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                    ui.separator();
                    let enabled = !self.confirm
                        && !self.recovery_prompt
                        && !self.close_after_recovery
                        && !self.tutorial.offer
                        && !self.placement.active;
                    if ui.add_enabled(enabled, egui::Button::new("View")).clicked() {
                        self.finish();
                        self.show_view = !self.show_view;
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Artwork [Tab]"))
                        .clicked()
                    {
                        self.toggle_artwork();
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Pick plane [K]"))
                        .clicked()
                    {
                        self.begin_placement();
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Scene"))
                        .clicked()
                    {
                        self.finish();
                        self.show_scene = !self.show_scene;
                    }
                    if ui.add_enabled(enabled, egui::Button::new("Help")).clicked() {
                        self.finish();
                        self.show_help = true;
                    }
                    if ui.add_enabled(enabled, egui::Button::new("New")).clicked() {
                        self.request(Pending::New);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("House"))
                        .on_hover_text("Open the editable Japanese garden house sample")
                        .clicked()
                    {
                        self.request(Pending::House);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Lantern"))
                        .on_hover_text("Open the editable cylindrical drawing-guide example")
                        .clicked()
                    {
                        self.request(Pending::Lantern);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Brushes"))
                        .on_hover_text("Open the editable brush sampler")
                        .clicked()
                    {
                        self.request(Pending::Brushes);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Dynamics"))
                        .on_hover_text("Open the editable pressure, tilt, and taper study")
                        .clicked()
                    {
                        self.request(Pending::Dynamics);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Textures"))
                        .on_hover_text("Open the editable texture study")
                        .clicked()
                    {
                        self.request(Pending::Textures);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Vase"))
                        .on_hover_text("Editable revolved-surface painting study")
                        .clicked()
                    {
                        self.request(Pending::Vase);
                    }
                    if ui.add_enabled(enabled, egui::Button::new("Open")).clicked() {
                        self.request(Pending::Open);
                    }
                    if ui.add_enabled(enabled, egui::Button::new("Save")).clicked() {
                        self.save(false);
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Save as"))
                        .clicked()
                    {
                        self.save(true);
                    }
                    ui.add_enabled_ui(self.can_export(false) || self.can_export_png(), |ui| {
                        ui.menu_button("Export", |ui| {
                            if ui.button("Whole drawing (.glb)").clicked() {
                                ui.close_menu();
                                self.export_dialog(false, ctx);
                            }
                            if ui
                                .add_enabled(
                                    self.can_export(true),
                                    egui::Button::new("Selected strokes (.glb)"),
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.export_dialog(true, ctx);
                            }
                            if ui
                                .add_enabled(
                                    self.can_export_png(),
                                    egui::Button::new("Artwork image (.png)…"),
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.png_export.begin();
                            }
                        });
                    });
                    if self.export_job.is_some() || self.png_export.is_running() {
                        ui.spinner();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            enabled
                                && (!self.undo.is_empty()
                                    || self.active.is_some()
                                    || self.drag.is_some()),
                            egui::Button::new("Undo"),
                        )
                        .clicked()
                    {
                        self.undo();
                    }
                    if ui
                        .add_enabled(enabled && !self.redo.is_empty(), egui::Button::new("Redo"))
                        .clicked()
                    {
                        self.redo();
                    }
                    ui.label(if self.dirty {
                        "Unsaved changes"
                    } else {
                        "Saved / blank"
                    });
                });
                ui.add_space(7.0);
            });
            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.tutorial.practice.is_none()
                                && !self.tutorial.offer
                                && !self.placement.active,
                            egui::Button::new("Recovery"),
                        )
                        .clicked()
                    {
                        self.recovery_prompt = true;
                        self.recovery.request_scan();
                    }
                    ui.label(&self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(self.active_guide_label());
                    });
                });
            });
            egui::SidePanel::left("tools")
                .exact_width(240.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_enabled_ui(
                            !self.confirm
                                && !self.recovery_prompt
                                && !self.close_after_recovery
                                && !self.tutorial.offer
                                && !self.placement.active,
                            |ui| self.tools(ui),
                        );
                    });
                });
            egui::SidePanel::right("diagnostics")
                .exact_width(245.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.diagnostics(ui));
                });
        } else {
            self.artwork_bar(ctx);
        }
        self.repeat_modal(ctx);
        self.png_export.show(ctx);
        let pen = self.pen.as_ref().map(|p| p.drain()).unwrap_or_default();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                self.verify_lines(ctx, rect);
                self.verify_views(ctx, rect);
                self.verify_presentation(ctx, rect);
                self.verify_placement(ctx, rect);
                self.verify_fills(ctx, rect);
                self.verify_lighting(ctx, rect);
                self.verify_tutorial(ctx, rect);
                self.verify_groups(ctx, rect);
                self.verify_repeat(ctx);
                self.verify_palette(ctx, rect);
                self.verify_vase(ctx, rect);
                self.verify_layers(ctx, rect);
                self.verify_recovery(ctx, rect);
                self.verify_presets(ctx, rect);
                self.verify_textures(ctx, rect);
                self.verify_dynamics(ctx, rect);
                self.verify_brushes(ctx, rect);
                self.verify_cylinder(ctx, rect);
                self.verify_house(ctx);
                self.verify_canvas(ctx, rect);
                self.canvas_input(ctx, rect, pen);
                let scale = ctx.pixels_per_point();
                if self.select_mode && !self.presentation.artwork {
                    self.renderer.set_selection(&self.selected);
                } else {
                    self.renderer.set_selection(&HashSet::new());
                }
                self.renderer.background = self.presentation.preferences.background;
                let id = self.renderer.render(
                    &self.doc,
                    &self.camera,
                    if self.presentation.artwork {
                        GuidePreview::Hidden
                    } else {
                        self.placement_preview()
                            .map(GuidePreview::Plane)
                            .unwrap_or_else(|| self.guide_preview())
                    },
                    [
                        (rect.width() * scale).round() as u32,
                        (rect.height() * scale).round() as u32,
                    ],
                );
                ui.painter().image(
                    id,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if let Some((destination, size, allow_overwrite)) = self.png_export.take_request() {
                    self.start_png_export(destination, size, allow_overwrite, ctx);
                }
                self.verify_png_export(ctx);
                if !self.presentation.artwork {
                    self.line_overlay(ui, rect);
                    self.fill_overlay(ui, rect);
                    self.selection_overlay(ui, rect);
                    self.plane_overlay(ui, rect);
                    self.placement_overlay(ui, rect);
                    let badge = Rect::from_min_size(
                        rect.min + egui::vec2(20.0, 20.0),
                        egui::vec2(200.0, 32.0),
                    );
                    ui.painter()
                        .rect_filled(badge, 5.0, Color32::from_black_alpha(110));
                    ui.painter().text(
                        badge.center(),
                        egui::Align2::CENTER_CENTER,
                        self.guide_badge(),
                        egui::FontId::proportional(13.0),
                        Color32::from_rgb(173, 210, 213),
                    );
                    if self.starter_scene || self.cylinder_example {
                        ui.painter().text(
                            rect.min + egui::vec2(22.0, 76.0),
                            egui::Align2::LEFT_TOP,
                            if self.cylinder_example {
                                "Cylindrical lantern"
                            } else {
                                "Japanese garden house"
                            },
                            egui::FontId::proportional(24.0),
                            Color32::from_rgb(240, 217, 172),
                        );
                        ui.painter().text(
                            rect.min + egui::vec2(23.0, 110.0),
                            egui::Align2::LEFT_TOP,
                            if self.cylinder_example {
                                "Curved guide study / draw on the side, orbit to reach the back"
                            } else {
                                "Editable starter drawing / New for a blank canvas"
                            },
                            egui::FontId::proportional(13.0),
                            Color32::from_rgb(171, 190, 182),
                        );
                    }
                    if self.doc.strokes.is_empty() {
                        ui.painter().text(
                            rect.center() + egui::vec2(0.0, -35.0),
                            egui::Align2::CENTER_CENTER,
                            "Your next idea starts here.",
                            egui::FontId::proportional(25.0),
                            Color32::from_gray(190),
                        );
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Pen to draw   /   Space + drag to explore",
                            egui::FontId::proportional(14.0),
                            Color32::from_gray(125),
                        );
                    }
                }
                if response.hovered() && !self.presentation.artwork {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                }
            });
        if self.confirm {
            egui::Window::new("Save your drawing?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    if self.tutorial.practice.is_some() {
                        ui.label("Practice has unsaved changes. Save or discard practice to return to your original drawing, or cancel to keep practicing.");
                    } else if matches!(self.pending, Some(Pending::TutorialStart)) {
                        ui.label("Your drawing has unsaved changes. Save a disk copy first, or continue without saving; it will stay in memory during practice.");
                    } else {
                        ui.label("There are unsaved changes.");
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Save and continue").clicked() && self.save(false) {
                            let close = matches!(self.pending, Some(Pending::Close));
                            self.execute_pending();
                            if close {
                                self.close_after_recovery = true;
                            }
                        }
                        if ui.button("Discard").clicked() {
                            let close = matches!(self.pending, Some(Pending::Close));
                            self.execute_pending();
                            if close {
                                self.dirty = false;
                                self.close_after_recovery = true;
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending = None;
                            self.recovery_to_restore = None;
                            self.confirm = false;
                        }
                    });
                });
        }
        self.show_view(ctx);
        self.show_placement(ctx);
        self.show_lighting(ctx);
        self.show_tutorial(ctx);
        if self.show_help {
            egui::Window::new("Drawing with a tablet").default_width(540.0).max_height((ctx.screen_rect().height()-80.0).max(200.0)).vscroll(true).show(ctx,|ui|{self.tutorial_help(ui);ui.separator();if ui.button("Close help").clicked(){self.show_help=false;}ui.label("V: select   Shift-click: toggle stroke selection\nG: move   R: rotate   S: uniform scale (in Select tool)\nDrag the visible handles; Escape cancels a transform.\nCtrl/Cmd+A: select all   Ctrl/Cmd+D: duplicate\nDelete / Backspace: delete selection\n\nB: brush   E: whole-mark eraser   P: planar fill   I: sample artwork color (Escape cancels)\nK: pick plane   F: align view to guide   Shift+F: plane from view\nCtrl/Cmd+Z: undo   Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y: redo\nCtrl/Cmd+S: save   Ctrl/Cmd+Shift+S: save as\nCtrl/Cmd+O: open   Ctrl/Cmd+N: new\n\nSpace + pen drag: orbit\nSpace + Shift + pen drag: pan\nSpace + Alt + pen drag: zoom\nPen barrel + drag: orbit (when reported by Windows Ink)\nRight mouse drag: orbit   Middle drag: pan   Wheel: zoom\n\nUse Front / Side / Ground or rotate and position a custom plane.\nShift+F creates a plane through the camera focus; F looks straight at it.\nExisting strokes stay in world space when you switch planes.\nEnable Windows Ink in your Wacom / Huion driver for pressure.\nA mouse works too, with fixed pressure.\n\nExport menu: whole drawing or selected strokes as GLB.\nPrototype limits: no Wintab, additional curved guides. Recovery saves completed edits automatically; Save keeps your native drawing.\nSave your work using the Save button.");});
        }
        self.show_recovery(ctx);
        self.tick_recovery(ctx);
        self.frame_ms = start.elapsed().as_secs_f32() * 1000.0;
        if self.active.is_some()
            || self.drag.is_some()
            || self.verification_frame.is_some()
            || self.house_verification_frame.is_some()
            || self.cylinder_verification_frame.is_some()
            || self.brush_verification_frame.is_some()
            || self.dynamics_verification_frame.is_some()
            || self.texture_verification_frame.is_some()
            || self.preset_verification_frame.is_some()
            || self.recovery_verification_frame.is_some()
            || self.layer_verification_frame.is_some()
            || self.vase_verification_frame.is_some()
            || self.group_verification_frame.is_some()
            || self.tutorial.verification_frame.is_some()
            || self.lighting_verification_frame.is_some()
            || self.fills.verification_frame.is_some()
            || self.placement.verification_frame.is_some()
            || self.presentation.verification_frame.is_some()
            || self.view_verification_frame.is_some()
            || self.line.verification_frame.is_some()
            || self.close_after_recovery
            || self.recovery.busy()
        {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
    }
}

impl SketchApp {
    /// Deterministic software-input smoke test; never labels synthetic input as hardware input.
    fn verify_canvas(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.stabilization = 0.0;
                self.camera.align(Plane::Front);
                let start = rect.center() - egui::vec2(120.0, 0.0);
                let mut events = vec![egui::Event::PointerButton {
                    pos: start,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }];
                for i in 0..100 {
                    events.push(egui::Event::PointerMoved(
                        start + egui::vec2(i as f32 * 2.4, (i as f32 * 0.08).sin() * 65.0),
                    ));
                }
                events.push(egui::Event::PointerButton {
                    pos: start + egui::vec2(240.0, 0.0),
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
                ctx.input_mut(|i| i.events = events);
            }
            2 => {
                assert_eq!(self.doc.strokes.len(), 1, "Mouse event stroke must commit");
                assert!(
                    self.doc.strokes[0].points.len() > 80,
                    "Intermediate mouse samples must survive a single frame"
                );
                self.undo();
                assert!(self.doc.strokes.is_empty());
                self.redo();
                assert_eq!(self.doc.strokes.len(), 1);
                self.erase(rect.center() - egui::vec2(120.0, 0.0), rect);
                assert!(self.doc.strokes.is_empty(), "Whole-stroke eraser");
                self.undo();
                assert_eq!(self.doc.strokes.len(), 1);
                self.doc = Document::default();
                self.undo.clear();
                self.redo.clear();
                self.selected.clear();
                self.renderer.invalidate();
                self.demo();
                let path = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&path).unwrap();
                self.file = Some(path.join("smoke.sketch.json"));
                assert!(self.save(false));
                assert!(self.save(false), "Replacing an existing save must work");
                let loaded = Document::from_json(
                    &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(loaded.strokes.len(), 3);
                assert_eq!(loaded.strokes[0].points.len(), 160);
                self.doc = loaded;
                self.renderer.invalidate();
                self.camera = Camera::default();
                self.status =
                    "Software smoke checks passed. Physical tablet validation is still required."
                        .into();
                // Clear synthetic events before the regular input handler sees them again.
                ctx.input_mut(|i| i.events.clear());
            }
            3 => self.verify_selection(ctx, rect),
            4 => self.verify_stabilization(rect),
            5 => self.verify_planes(rect),
            6 => self.verify_export_start(ctx),
            7..=119 => {
                if self.verify_export_done() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
            }
            120 => panic!("Verification screenshot timed out"),
            _ => {}
        }
        self.verification_frame = Some(frame + 1);
    }
    fn capture_verification(&mut self, ctx: &egui::Context) {
        if self.verification_frame.is_none()
            && self.house_verification_frame.is_none()
            && self.cylinder_verification_frame.is_none()
            && self.brush_verification_frame.is_none()
            && self.dynamics_verification_frame.is_none()
            && self.texture_verification_frame.is_none()
            && self.preset_verification_frame.is_none()
            && self.recovery_verification_frame.is_none()
            && self.layer_verification_frame.is_none()
            && self.vase_verification_frame.is_none()
            && self.group_verification_frame.is_none()
        {
            return;
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let path = std::env::current_dir().unwrap().join("verification");
            let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            if self.recovery_offer_capture {
                image::save_buffer(
                    path.join("recovery-offer.png"),
                    &pixels,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.recovery_offer_capture = false;
                return;
            }
            image::save_buffer(
                path.join(if self.group_verification_frame.is_some() {
                    "group-preview.png"
                } else if self.vase_verification_frame.is_some() {
                    "vase-preview.png"
                } else if self.layer_verification_frame.is_some() {
                    "layer-preview.png"
                } else if self.recovery_verification_frame.is_some() {
                    "recovery-preview.png"
                } else if self.preset_verification_frame.is_some() {
                    "preset-preview.png"
                } else if self.texture_verification_frame.is_some() {
                    "texture-preview.png"
                } else if self.dynamics_verification_frame.is_some() {
                    "dynamics-preview.png"
                } else if self.brush_verification_frame.is_some() {
                    "brush-preview.png"
                } else if self.cylinder_verification_frame.is_some() {
                    "cylinder-preview.png"
                } else if self.house_verification_frame.is_some() {
                    "house-preview.png"
                } else {
                    "preview.png"
                }),
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            let report = serde_json::json!({"passed":true,"checks":["single-frame mouse sample order","3D geometry and GPU pipeline","undo/redo","whole-stroke eraser","atomic save","document reload","UI screenshot","Shift multi-selection","axis move and view-plane cancel","rotation and uniform scale handles","group duplicate/delete undo","numeric transform","transformed document reload","stabilized stroke endpoint and undo","custom plane drawing and grid","plane switching preserves geometry","plane alignment and view placement","oriented stroke save/reload","background GLB export preserves document state"],"gpu":self.renderer.adapter,"strokes":self.doc.strokes.len(),"physical_tablet_tested":false});
            let report = if self.group_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["house component grouping","group and individual selection","duplicate independent group","shared pivot transforms","ungroup undo and native persistence","layer protection","GPU groups"],"gpu":self.renderer.adapter})
            } else if self.vase_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["revolved profile drawing","captured guide and surface release tail","pressure tilt brush frames","guide editing preserves ink","selection duplicate undo","native persistence","GPU painted vase"],"gpu":self.renderer.adapter})
            } else if self.layer_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["layer drawing and locking","visibility picking and erasing","layer reassignment undo redo","nonempty deletion protection","version 5 persistence","GPU layers"],"gpu":self.renderer.adapter})
            } else if self.recovery_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["forced process termination","startup recovery offer","exact fill contour offset pressure tilt dynamics texture restoration","recovered copy is untitled and dirty","native source unchanged","new session snapshot durable before old retirement","Save New and close cleanup ordering","corrupt and unsupported issue visibility","GPU recovery workflow"],"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.preset_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["preset application captures resolved settings","create duplicate rename update delete and defaults","restart persistence","live GPU previews","library edits preserve existing ink and export","corrupt library fallback and explicit repair backup"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.texture_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["masked GPU rendering","selection retains coverage","texture transforms preserve UVs","stroke duplication and undo","textured drawing and erasing","version 8 save/reload","original procedural mask sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.dynamics_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic Windows Ink contact and tilt","independent pressure settings","pressure-to-opacity capture","soft-edge geometry coverage","overlap compositing independent of document order","camera-orbit stability","opaque-surface occlusion","stabilized release tail","cancellation discards dynamics","missing tilt fallback","captured settings","version 8 save/reload","GPU dynamics sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.brush_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["all brush types through drawing input","captured guide orientation","camera orbit preserves mesh","shape-aware picking and erasing","duplication","scale and rotation preserve brush shape","cylinder frames and stabilized release tails","undo/redo","version 8 save/reload","GPU brush sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.cylinder_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["cylinder side drawing","surface-constrained stabilization and release tail","miss and silhouette segmentation","angular seam continuity","rotated guide and captured surface","hide and switch guides preserve ink","selection duplicate/delete undo","cylinder stroke save/reload","editable lantern example","GPU screenshot"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.house_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["editable Japanese house strokes","duplicate and undo","New clears starter","House reloads starter","unsaved-change guard","sample save/reload","GPU screenshot"],"strokes":self.doc.strokes.len(),"points":self.doc.strokes.iter().map(|s|s.points.len()).sum::<usize>(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else {
                report
            };
            std::fs::write(
                path.join(if self.group_verification_frame.is_some() {
                    "group-report.json"
                } else if self.vase_verification_frame.is_some() {
                    "vase-report.json"
                } else if self.layer_verification_frame.is_some() {
                    "layer-report.json"
                } else if self.recovery_verification_frame.is_some() {
                    "recovery-report.json"
                } else if self.preset_verification_frame.is_some() {
                    "preset-report.json"
                } else if self.texture_verification_frame.is_some() {
                    "texture-report.json"
                } else if self.dynamics_verification_frame.is_some() {
                    "dynamics-report.json"
                } else if self.brush_verification_frame.is_some() {
                    "brush-report.json"
                } else if self.cylinder_verification_frame.is_some() {
                    "cylinder-report.json"
                } else if self.house_verification_frame.is_some() {
                    "house-report.json"
                } else {
                    "report.json"
                }),
                serde_json::to_string_pretty(&report).unwrap(),
            )
            .unwrap();
            self.dirty = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl SketchApp {
    fn verify_stabilization(&mut self, rect: Rect) {
        let original = self.doc.strokes.clone();
        let selection = self.selected.clone();
        let steps = self.undo.len();
        self.select_mode = false;
        self.stabilization = 0.8;
        let start = rect.center() - egui::vec2(100.0, 60.0);
        let mut endpoint = start;
        for i in 0..100 {
            endpoint = start + egui::vec2(i as f32 * 2.0, if i % 2 == 0 { 3.0 } else { -3.0 });
            self.add_point(endpoint, 0.6, rect, i == 0);
        }
        let ndc = glam::vec2(
            (endpoint.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (endpoint.y - rect.top()) / rect.height() * 2.0,
        );
        let (o, d) = self.camera.ray(ndc, rect.aspect_ratio());
        let expected = intersect_plane(o, d, self.guide).unwrap();
        self.finish();
        let stroke = self.doc.strokes.last().unwrap();
        assert!(stroke.points.last().unwrap().position.distance(expected) < 0.00001);
        assert_eq!(stroke.points.last().unwrap().pressure, 0.6);
        assert_eq!(self.undo.len(), steps + 1);
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.redo();
        assert_eq!(self.doc.strokes.len(), original.len() + 1);
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.selected = selection;
        self.select_mode = true;
        self.stabilization = 0.35;
        assert!(self.save(false));
        self.status =
            "Selection, transforms, stabilization, endpoint, and undo checks passed.".into();
    }
}
