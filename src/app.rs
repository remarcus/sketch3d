use crate::surface::{Cylinder, GuidePreview, Surface, SurfaceStabilizer};
use crate::{
    document::{Brush, BrushKind, Document, Point, Stroke},
    geometry::{intersect_plane, Camera, DrawingPlane, Plane},
    input::{PenInput, PenSample, Phase},
    renderer::SceneRenderer,
};
use crate::{editing::Edit, gizmo::Mode};
use color_editor::{ColorAction, ColorTarget};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, RichText, Vec2};
use glam::Vec3;
use std::collections::{hash_map::Entry, HashMap, HashSet};

type LoadedDrawing = (
    Document,
    HashMap<u64, crate::image_planes::ImageAsset>,
    HashMap<u64, crate::obj_reference::ObjModel>,
);
#[path = "brush_dynamics.rs"]
mod brush_dynamics;
#[path = "brush_textures.rs"]
mod brush_textures;
#[path = "brushes.rs"]
mod brushes;
#[path = "camera_ui.rs"]
mod camera_ui;
#[path = "color_editor.rs"]
mod color_editor;
#[path = "curve_tool.rs"]
mod curve_tool;
#[path = "cylinders.rs"]
mod cylinders;
#[path = "drawing_trash_ui.rs"]
mod drawing_trash_ui;
#[path = "export_ui.rs"]
mod export_ui;
#[path = "fills_checks.rs"]
mod fills_checks;
#[path = "fills_ui.rs"]
mod fills_ui;
#[path = "groups_ui.rs"]
mod groups_ui;
#[path = "guide_resource_ui.rs"]
mod guide_resource_ui;
#[path = "image_plane_tool_checks.rs"]
mod image_plane_tool_checks;
#[path = "image_plane_ui.rs"]
mod image_plane_ui;
#[path = "layers_ui.rs"]
mod layers_ui;
#[path = "lighting_ui.rs"]
mod lighting_ui;
#[path = "line_checks.rs"]
mod line_checks;
#[path = "line_tool.rs"]
mod line_tool;
#[path = "liquify_ui.rs"]
mod liquify_ui;
#[path = "mirror_checks.rs"]
mod mirror_checks;
#[path = "mirror_ui.rs"]
mod mirror_ui;
#[path = "mixed_resources_ui.rs"]
mod mixed_resources_ui;
#[path = "model_surface_checks.rs"]
mod model_surface_checks;
#[path = "native_merge.rs"]
mod native_merge;
#[path = "native_merge_check.rs"]
mod native_merge_check;
#[path = "obj_export_ui.rs"]
mod obj_export_ui;
#[path = "obj_reference_checks.rs"]
mod obj_reference_checks;
#[path = "obj_reference_ui.rs"]
mod obj_reference_ui;
#[path = "palette_ui.rs"]
mod palette_ui;
#[path = "partial_eraser_ui.rs"]
mod partial_eraser_ui;
#[path = "placement_checks.rs"]
mod placement_checks;
#[path = "placement_ui.rs"]
mod placement_ui;
#[path = "planes.rs"]
mod planes;
#[path = "png_export_ui.rs"]
pub(crate) mod png_export_ui;
#[path = "presentation_checks.rs"]
mod presentation_checks;
#[path = "presentation_ui.rs"]
mod presentation_ui;
#[path = "preset_checks.rs"]
mod preset_checks;
#[path = "recovery_ui.rs"]
mod recovery_ui;
#[path = "reference_board.rs"]
mod reference_board;
#[path = "revolved_ui.rs"]
mod revolved_ui;
#[path = "ring_checks.rs"]
mod ring_checks;
#[path = "selection.rs"]
mod selection;
#[path = "shape_tool.rs"]
mod shape_tool;
#[path = "starter.rs"]
mod starter;
#[path = "tutorial_ui.rs"]
mod tutorial_ui;
#[path = "ui_style.rs"]
pub(crate) mod ui_style;
#[path = "video_export_ui.rs"]
pub(crate) mod video_export_ui;

pub(crate) fn write_native_merge_fixture(output: &std::path::Path) -> Result<(), String> {
    native_merge_check::run(output)
}
#[path = "views.rs"]
mod views;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, mpsc::SyncSender},
    thread::JoinHandle,
    time::{Duration, Instant},
};

fn canvas_pointer_allowed(ctx: &egui::Context, canvas: Rect, position: Pos2) -> bool {
    canvas.contains(position)
        && ctx
            .layer_id_at(position)
            .is_none_or(|layer| layer.order == egui::Order::Background)
}

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
    curve_tool: curve_tool::CurveUi,
    shape_tool: shape_tool::ShapeTool,
    mirror_ui: mirror_ui::MirrorUi,
    drawing_library: crate::drawing_library::DrawingLibrary,
    show_drawing_library: bool,
    drawing_library_filter: String,
    library_folder_filter: String,
    new_library_folder: String,
    rename_library_source: Option<PathBuf>,
    rename_library_stem: String,
    delete_library_source: Option<PathBuf>,
    library_open_path: Option<PathBuf>,
    insert_preview: Option<(PathBuf, Document)>,
    insert_offset: [f32; 3],
    insert_as_group: bool,
    show_view: bool,
    view_verification_frame: Option<u32>,
    presentation: presentation_ui::PresentationUi,
    placement: placement_ui::PlacementUi,
    fills: fills_ui::FillUi,
    show_scene: bool,
    lighting_verification_frame: Option<u32>,
    tutorial: tutorial_ui::TutorialUi,
    renderer: SceneRenderer,
    camera_bookmarks: crate::camera_bookmarks::CameraBookmarkStore,
    camera_bookmark_path: Option<PathBuf>,
    camera_bookmark_error: Option<String>,
    camera_bookmark_restart_report: Option<PathBuf>,
    camera_ui: camera_ui::CameraBookmarkUi,
    camera_playback: Option<camera_ui::CameraPlayback>,
    liquify_ui: liquify_ui::LiquifyUi,
    mixed_resources_ui: mixed_resources_ui::MixedResourcesUi,
    image_plane_assets: std::collections::HashMap<u64, crate::image_planes::ImageAsset>,
    obj_reference_models: std::collections::HashMap<u64, crate::obj_reference::ObjModel>,
    obj_surface_indices: std::collections::HashMap<u64, crate::obj_mesh_index::ObjMeshIndex>,
    model_stroke_contact: Option<(u64, u32, Pos2)>,
    selected_model_reference: Option<u64>,
    obj_reference_window_open: bool,
    obj_reference_verification_frame: Option<u32>,
    obj_reference_verification_import_rect: Option<Rect>,
    obj_reference_verification_button_clicked: bool,
    obj_reference_verification_source: Option<Option<PathBuf>>,
    obj_reference_verification_source_root: Option<tempfile::TempDir>,
    obj_reference_verification_asset_root: Option<tempfile::TempDir>,
    appearance_layout_verification: Option<AppearanceLayoutVerification>,
    image_asset_root: PathBuf,
    guide_name: String,
    loft_preview: Option<crate::document::guide_resources::LoftSurface>,
    palette_ui: palette_ui::PaletteUi,
    reference_board: reference_board::ReferenceBoard,
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
    image_plane_verification_frame: Option<u32>,
    mirror_verification_frame: Option<u32>,
    ring_verification_frame: Option<u32>,
    model_surface_verification_frame: Option<u32>,
    image_plane_verification_changed_pixels: usize,
    liquify_brush_verification_frame: Option<u32>,
    liquify_brush_verification_before: Option<Vec<Stroke>>,
    mixed_resources_verification_frame: Option<u32>,
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
    partial_eraser: partial_eraser_ui::PartialEraserUi,
    select_mode: bool,
    selected: HashSet<u64>,
    transform_mode: Mode,
    transform_values: [f32; 3],
    scale_values: [f32; 3],
    scale_uniform_lock: bool,
    scale_brush_width: bool,
    transform_pivot: selection::PivotMode,
    custom_pivot: Vec3,
    drag: Option<selection::Drag>,
    brush_select_enabled: bool,
    brush_select_radius: f32,
    brush_select_through: bool,
    brush_select_deselect: bool,
    brush_select_drag: Option<selection::BrushSelectionDrag>,
    appearance_draft: Option<selection::AppearanceDraft>,
    active: Option<u64>,
    active_started_at: Option<Instant>,
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
    show_diagnostics: bool,
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
    video_export: video_export_ui::VideoExportUi,
    construction_history: crate::construction_history::ConstructionHistory,
    obj_export: obj_export_ui::ObjExportUi,
    png_verification_frame: Option<u32>,
    capability_replay: Option<CapabilityReplay>,
}

const CAPABILITY_REPLAY_FPS: u32 = 10;
const CAPABILITY_REPLAY_FRAMES: usize = 60;
const CAPABILITY_REPLAY_MAX_GIF_BYTES: u64 = 5 * 1024 * 1024;
const CAPABILITY_REPLAY_MAX_MP4_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapabilityClip {
    DrawOrbit,
    CurvedGuides,
    MirrorShapes,
    EditErase,
}

impl CapabilityClip {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "draw-orbit" => Some(Self::DrawOrbit),
            "curved-guides" => Some(Self::CurvedGuides),
            "mirror-shapes" => Some(Self::MirrorShapes),
            "edit-erase" => Some(Self::EditErase),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::DrawOrbit => "draw-orbit",
            Self::CurvedGuides => "curved-guides",
            Self::MirrorShapes => "mirror-shapes",
            Self::EditErase => "edit-erase",
        }
    }

    fn is_curved_guides(self) -> bool {
        self == Self::CurvedGuides
    }

    fn is_edit_erase(self) -> bool {
        self == Self::EditErase
    }

    fn is_mirror_shapes(self) -> bool {
        self == Self::MirrorShapes
    }
}

pub(crate) fn prepare_capability_demo_output(name: &str) -> Result<PathBuf, String> {
    let clip = CapabilityClip::parse(name)
        .ok_or_else(|| format!("Unsupported capability demo clip: {name}"))?;
    let output_dir = std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join("verification")
        .join("capability-demos")
        .join("drafts");
    std::fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    for extension in ["gif", "mp4", "png", "en.vtt", "recording.json"] {
        let path = output_dir.join(format!("{}.{}", clip.name(), extension));
        if path.exists() {
            return Err(format!(
                "Refusing to overwrite existing capability demo output: {}",
                path.display()
            ));
        }
    }
    Ok(output_dir)
}

struct CapabilityReplay {
    clip: CapabilityClip,
    output_dir: PathBuf,
    initial_stroke_count: usize,
    initial_camera: Camera,
    canvas_rect: Option<Rect>,
    captured_frames: usize,
    screenshot_pending: bool,
    screenshot_requested: bool,
    wait_updates: u32,
    frame_size: Option<[u32; 2]>,
    pixels_per_point: f32,
    pointer_cue: Option<Pos2>,
    contact: bool,
    barrel: bool,
    eraser: bool,
    setup_verified: bool,
    undo_applied: bool,
    gesture_start: Option<Pos2>,
    gesture_end: Option<Pos2>,
    expected_undo_strokes: Option<Vec<Stroke>>,
    sender: Option<SyncSender<Option<Vec<u8>>>>,
    encoder: Option<JoinHandle<Result<(), String>>>,
    mp4_sender: Option<SyncSender<Option<Vec<u8>>>>,
    mp4_encoder: Option<JoinHandle<Result<(), String>>>,
}

impl CapabilityReplay {
    fn path(&self, extension: &str) -> PathBuf {
        self.output_dir
            .join(format!("{}.{}", self.clip.name(), extension))
    }

    fn remove_draft_outputs(&self) {
        for extension in ["gif", "mp4", "png", "en.vtt", "recording.json"] {
            let _ = std::fs::remove_file(self.path(extension));
        }
    }

    fn abort(mut self) {
        self.sender.take();
        self.mp4_sender.take();
        if let Some(encoder) = self.encoder.take() {
            let _ = encoder.join();
        }
        if let Some(encoder) = self.mp4_encoder.take() {
            let _ = encoder.join();
        }
        self.remove_draft_outputs();
    }

    fn save_poster(&self, pixels: &[u8], dimensions: [u32; 2]) -> Result<(), String> {
        use image::ImageEncoder;
        let destination = self.path("png");
        let mut temporary = tempfile::Builder::new()
            .suffix(".png")
            .tempfile_in(&self.output_dir)
            .map_err(|error| error.to_string())?;
        image::codecs::png::PngEncoder::new(temporary.as_file_mut())
            .write_image(
                pixels,
                dimensions[0],
                dimensions[1],
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        temporary
            .persist(&destination)
            .map_err(|error| error.error.to_string())?;
        Ok(())
    }

    fn next_sample(&mut self, rect: Rect) -> Option<PenSample> {
        if self.screenshot_pending || self.captured_frames >= CAPABILITY_REPLAY_FRAMES {
            return None;
        }
        self.screenshot_pending = true;
        self.canvas_rect = Some(rect);
        self.wait_updates = 0;
        let frame = self.captured_frames;
        let orbit_sample = if self.clip.is_curved_guides() || self.clip.is_mirror_shapes() {
            let start = rect.center() + egui::vec2(0.0, 12.0);
            let end = start + egui::vec2(270.0, -55.0);
            match frame {
                0..=4 => (start, Phase::Move, false, false, false),
                5..=50 => {
                    let t = (frame - 5) as f32 / 45.0;
                    (
                        egui::pos2(
                            start.x + (end.x - start.x) * t,
                            start.y + (end.y - start.y) * t,
                        ),
                        if frame == 5 { Phase::Down } else { Phase::Move },
                        true,
                        true,
                        false,
                    )
                }
                51 => (end, Phase::Up, false, false, false),
                _ => (end, Phase::Move, false, false, false),
            }
        } else if self.clip.is_edit_erase() {
            let start = self
                .gesture_start
                .expect("edit-erase replay setup provides a gesture start");
            let end = self
                .gesture_end
                .expect("edit-erase replay setup provides a gesture end");
            match frame {
                0..=3 => (start, Phase::Move, false, false, false),
                4..=25 => {
                    let t = (frame - 4) as f32 / 21.0;
                    (
                        egui::pos2(
                            start.x + (end.x - start.x) * t,
                            start.y + (end.y - start.y) * t,
                        ),
                        if frame == 4 { Phase::Down } else { Phase::Move },
                        true,
                        false,
                        true,
                    )
                }
                26 => (end, Phase::Up, false, false, true),
                27..=33 => (end, Phase::Move, false, false, false),
                34..=36 => (end, Phase::Move, false, false, false),
                37..=50 => {
                    let t = (frame - 37) as f32 / 13.0;
                    (
                        rect.center() + egui::vec2(t * 220.0, -t * 45.0),
                        if frame == 37 {
                            Phase::Down
                        } else {
                            Phase::Move
                        },
                        true,
                        true,
                        false,
                    )
                }
                51 => (
                    rect.center() + egui::vec2(220.0, -45.0),
                    Phase::Up,
                    false,
                    false,
                    false,
                ),
                _ => (
                    rect.center() + egui::vec2(220.0, -45.0),
                    Phase::Move,
                    false,
                    false,
                    false,
                ),
            }
        } else {
            let start = rect.center() + egui::vec2(-150.0, -65.0);
            let end = rect.center() + egui::vec2(150.0, -35.0);
            match frame {
                0..=2 => (start, Phase::Move, false, false, false),
                3 => (start, Phase::Down, true, false, false),
                4..=25 => {
                    let t = (frame - 4) as f32 / 21.0;
                    (
                        egui::pos2(
                            start.x + (end.x - start.x) * t,
                            start.y
                                + (end.y - start.y) * t
                                + (t * std::f32::consts::TAU).sin() * 28.0,
                        ),
                        Phase::Move,
                        true,
                        false,
                        false,
                    )
                }
                26 => (end, Phase::Up, false, false, false),
                27..=33 => (end, Phase::Move, false, false, false),
                34..=50 => {
                    let t = (frame - 34) as f32 / 16.0;
                    (
                        egui::pos2(
                            end.x + t * 210.0,
                            end.y + (t * std::f32::consts::PI).sin() * 52.0,
                        ),
                        Phase::Move,
                        true,
                        true,
                        false,
                    )
                }
                51 => (end + egui::vec2(210.0, 0.0), Phase::Up, false, false, false),
                _ => (
                    end + egui::vec2(210.0, 0.0),
                    Phase::Move,
                    false,
                    false,
                    false,
                ),
            }
        };
        let (position, phase, contact, barrel, eraser) = orbit_sample;
        self.pointer_cue = Some(position);
        self.contact = contact;
        self.barrel = barrel;
        self.eraser = eraser;
        Some(PenSample {
            position,
            pressure: None,
            tilt: [None, None],
            eraser,
            barrel,
            contact,
            phase,
        })
    }

    fn paint_cursor_cue(&self, painter: &egui::Painter, canvas: Rect) {
        let disclosure = Rect::from_min_size(
            egui::pos2(canvas.right() - 300.0, canvas.top() + 12.0),
            egui::vec2(288.0, 28.0),
        );
        painter.rect_filled(disclosure, 4.0, Color32::from_black_alpha(210));
        painter.text(
            disclosure.center(),
            egui::Align2::CENTER_CENTER,
            "SYNTHETIC REPLAY  •  NO PRESSURE",
            egui::FontId::proportional(15.0),
            Color32::WHITE,
        );
        let Some(position) = self
            .pointer_cue
            .filter(|position| canvas.contains(*position))
        else {
            return;
        };
        let accent = Color32::from_rgb(60, 220, 210);
        painter.circle_stroke(
            position,
            if self.contact { 10.0 } else { 8.0 },
            egui::Stroke::new(2.0_f32, accent),
        );
        painter.circle_filled(position, 2.5, accent);
        if self.contact {
            let label = if self.barrel {
                "ORBIT"
            } else if self.eraser {
                "ERASE"
            } else {
                "DRAW"
            };
            let label_rect =
                Rect::from_min_size(position + egui::vec2(14.0, -20.0), egui::vec2(56.0, 18.0));
            painter.rect_filled(label_rect, 3.0, Color32::from_black_alpha(210));
            painter.text(
                label_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
    }

    fn finish(mut self) -> Result<(), String> {
        if let Some(sender) = self.sender.take() {
            drop(sender);
        }
        if let Some(sender) = self.mp4_sender.take() {
            drop(sender);
        }
        let gif_result = self
            .encoder
            .take()
            .map(|encoder| {
                encoder
                    .join()
                    .map_err(|_| "Capability demo GIF encoder panicked".to_owned())?
            })
            .unwrap_or(Ok(()));
        let mp4_result = self
            .mp4_encoder
            .take()
            .map(|encoder| {
                encoder
                    .join()
                    .map_err(|_| "Capability demo MP4 encoder panicked".to_owned())?
            })
            .unwrap_or(Ok(()));
        if let Err(error) = gif_result.and(mp4_result) {
            self.remove_draft_outputs();
            return Err(error);
        }
        let gif_path = self.path("gif");
        let mp4_path = self.path("mp4");
        let poster_path = self.path("png");
        let gif_metadata = match std::fs::metadata(&gif_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.remove_draft_outputs();
                return Err(error.to_string());
            }
        };
        let mp4_metadata = match std::fs::metadata(&mp4_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.remove_draft_outputs();
                return Err(error.to_string());
            }
        };
        let poster_metadata = match std::fs::metadata(&poster_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.remove_draft_outputs();
                return Err(error.to_string());
            }
        };
        if gif_metadata.len() == 0 || gif_metadata.len() > CAPABILITY_REPLAY_MAX_GIF_BYTES {
            self.remove_draft_outputs();
            return Err(format!(
                "Capability demo GIF is {} bytes, empty or over the {} byte limit",
                gif_metadata.len(),
                CAPABILITY_REPLAY_MAX_GIF_BYTES,
            ));
        }
        if mp4_metadata.len() <= 64 || mp4_metadata.len() > CAPABILITY_REPLAY_MAX_MP4_BYTES {
            self.remove_draft_outputs();
            return Err(format!(
                "Capability demo MP4 is {} bytes, empty or over the {} byte limit",
                mp4_metadata.len(),
                CAPABILITY_REPLAY_MAX_MP4_BYTES,
            ));
        }
        if poster_metadata.len() == 0 {
            self.remove_draft_outputs();
            return Err("Capability demo poster is empty".into());
        }
        let manifest_path = self.path("recording.json");
        let captions_path = self.path("en.vtt");
        let captions = if self.clip.is_edit_erase() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:00.400\n",
                "The editable stroke was selected and recolored before capture.\n\n",
                "00:00:00.400 --> 00:00:02.700\n",
                "Drag the partial eraser across the stroke to split it.\n\n",
                "00:00:02.700 --> 00:00:03.400\n",
                "Two editable fragments remain.\n\n",
                "00:00:03.400 --> 00:00:03.700\n",
                "Undo restores the edited stroke.\n\n",
                "00:00:03.700 --> 00:00:05.200\n",
                "A synthetic barrel gesture orbits the restored stroke.\n\n",
                "00:00:05.200 --> 00:00:06.000\n",
                "The replay uses synthetic input with no pressure data.\n"
            )
        } else if self.clip.is_mirror_shapes() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:00.500\n",
                "A mirrored line and ellipse were prepared on the front guide.\n\n",
                "00:00:00.500 --> 00:00:05.200\n",
                "A synthetic barrel gesture orbits to reveal the editable geometry.\n\n",
                "00:00:05.200 --> 00:00:06.000\n",
                "The replay uses synthetic input with no pressure data.\n"
            )
        } else if self.clip.is_curved_guides() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:00.500\n",
                "Deterministic app replay. Synthetic input has no pressure data.\n\n",
                "00:00:00.500 --> 00:00:05.200\n",
                "A simulated barrel gesture orbits to reveal the curved drawing.\n\n",
                "00:00:05.200 --> 00:00:06.000\n",
                "The cylinder and mirrored strokes remain editable.\n"
            )
        } else {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:00.300\n",
                "Deterministic replay. Input is synthetic; tablet pressure was not measured.\n\n",
                "00:00:00.300 --> 00:00:02.700\n",
                "Draw a short cyan stroke on the front plane.\n\n",
                "00:00:03.400 --> 00:00:05.100\n",
                "Orbit the camera with a simulated barrel gesture.\n\n",
                "00:00:05.100 --> 00:00:06.000\n",
                "The new stroke remains editable.\n"
            )
        };
        let mut caption_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&captions_path)
            .map_err(|error| error.to_string())?;
        use std::io::Write;
        if let Err(error) = caption_file
            .write_all(captions.as_bytes())
            .and_then(|()| caption_file.sync_all())
        {
            self.remove_draft_outputs();
            return Err(error.to_string());
        }
        let recorded_unix_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs();
        let captions_timing_checked = self.clip.is_curved_guides() || self.clip.is_mirror_shapes();
        let app_commit = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|commit| !commit.is_empty())
            .unwrap_or_else(|| "unknown".into());
        let mp4_backend = if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            "FFmpeg H.264"
        } else {
            "Blender H.264 fallback"
        };
        let source_tree_dirty = std::process::Command::new("git")
            .args([
                "status",
                "--porcelain",
                "--",
                "src/app.rs",
                "src/main.rs",
                "src/selection.rs",
                "src/mirror_checks.rs",
                "src/tutorial_ui.rs",
                "src/presentation_ui.rs",
                "src/video_export_ui.rs",
            ])
            .output()
            .map(|output| !output.stdout.is_empty())
            .unwrap_or(true);
        let dimensions = self
            .frame_size
            .ok_or_else(|| "Capability demo did not capture any frames".to_owned())?;
        let clip_name = self.clip.name();
        let (fixture, fixture_provenance, shot_steps, poster_alt) = if self.clip.is_edit_erase() {
            (
                "Selected editable stroke",
                "Procedurally generated from an empty Sketch3D document by the production pointer-action handler; no external artwork or assets.",
                vec![
                    "Create an editable stroke through the production pointer-action handler and change its appearance through the selection editor handler.",
                    "Drag the partial eraser through the stroke using synthetic input through canvas_input.",
                    "Undo restores the edited source stroke; a short orbit reveals the result.",
                ],
                "An orange editable stroke on the Sketch3D front plane with a synthetic-input disclosure.",
            )
        } else if self.clip.is_mirror_shapes() {
            (
                "Mirrored front-guide line and ellipse",
                "Procedurally generated from an empty Sketch3D document through the production line-mirror and shape handlers; no external artwork or assets.",
                vec![
                    "Create a straight line and its reflected run through the production live-mirror line handler.",
                    "Create an editable ellipse on the front drawing guide through the production shape handler.",
                    "Orbit the camera using synthetic barrel samples through canvas_input.",
                ],
                "An editable cyan mirrored line pair and ellipse on Sketch3D's front drawing guide.",
            )
        } else if self.clip.is_curved_guides() {
            (
                "Saved cylinder mirror study",
                "Procedurally generated from an empty Sketch3D document by the production pointer-action handlers; no external artwork or assets.",
                vec![
                    "Create a saved cylinder and mirrored curved strokes through the production pointer-action handlers.",
                    "Orbit the camera using a synthetic barrel gesture through canvas_input.",
                    "Pause on the editable cylinder and reflected strokes.",
                ],
                "A saved cylinder with editable curved and mirrored strokes in Sketch3D's dark workspace.",
            )
        } else {
            (
                "Japanese house",
                "Original Japanese garden house scene built into Sketch3D; no external artwork or assets.",
                vec![
                    "Draw a short stroke on the Japanese house drawing plane with pressure-free synthetic input.",
                    "Orbit the camera using a synthetic barrel gesture through canvas_input.",
                    "Pause on the edited view.",
                ],
                "Japanese garden house in Sketch3D's dark workspace before the scripted cyan stroke is added.",
            )
        };
        let manifest = serde_json::json!({
            "format": "spatial-sketch-capability-demo",
            "manifest_version": 1,
            "clip": clip_name,
            "recorded_unix_seconds": recorded_unix_seconds,
            "duration_seconds": 6,
            "app_commit": app_commit,
            "source_tree_dirty": source_tree_dirty,
            "fixture": fixture,
            "fixture_version": 1,
            "license_or_provenance": fixture_provenance,
            "viewport": [dimensions[0], dimensions[1]],
            "capture_size": [dimensions[0], dimensions[1]],
            "gif_size": [640, 360],
            "dpi_percent": (self.pixels_per_point * 100.0).round() as u32,
            "theme": "Dark",
            "capture_mode": "deterministic hidden in-app replay through canvas_input",
            "input_note": if self.clip.is_edit_erase() { "The fixture stroke is created through the production pointer-action handler and its appearance edit through the production selection handler. Captured PenSample input uses pressure=None and no tilt; no tablet pressure or latency was measured." } else if self.clip.is_mirror_shapes() { "The mirrored line and ellipse are prepared by production handlers before capture. Captured PenSample orbit input uses pressure=None and no tilt; no tablet pressure or latency was measured." } else { "Synthetic PenSample input uses pressure=None and no tilt. The app applies its fixed default mouse-like width; no tablet pressure or latency was measured." },
            "window_visible": false,
            "frame_rate": CAPABILITY_REPLAY_FPS,
            "shot_steps": shot_steps,
            "cursor_cue": if self.clip.is_edit_erase() { "A synthetic pointer ring is labeled ERASE or ORBIT during the captured workflow; the selected stroke and appearance are prepared by production handlers before capture, and a persistent banner discloses no pressure data." } else if self.clip.is_mirror_shapes() { "The mirrored line and ellipse are prepared by production handlers before capture; playback shows a synthetic ORBIT cursor and persistent no-pressure disclosure." } else if self.clip.is_curved_guides() { "The curved-guide strokes are prepared by scripted production handlers before capture; playback shows a synthetic ORBIT cursor and a persistent no-pressure disclosure." } else { "A synthetic pointer ring is labeled DRAW or ORBIT while input is active; a persistent banner discloses synthetic replay and no pressure data." },
            "encoding": {
                "mp4": format!("Captured full-viewport RGBA frames; {mp4_backend}; 10 fps."),
                "gif": "In-app screenshot frames downscaled to 640x360; image GIF encoder without a loop extension (plays once); 10 fps; 6 seconds; <=5 MiB.",
                "poster": "First full-resolution app frame encoded as PNG."
            },
            "outputs": {
                "mp4": { "file": format!("{clip_name}.mp4"), "bytes": mp4_metadata.len() },
                "gif": { "file": format!("{clip_name}.gif"), "bytes": gif_metadata.len() },
                "poster": {
                    "file": format!("{clip_name}.png"),
                    "bytes": poster_metadata.len(),
                    "alt": poster_alt
                },
                "captions": {
                    "file": format!("{clip_name}.en.vtt"),
                    "bytes": std::fs::metadata(&captions_path).map_err(|error| error.to_string())?.len(),
                    "format": "WebVTT",
                    "timing_reviewed": captions_timing_checked
                }
            },
            "review": {
                "reviewed": false,
                "loop_boundary_checked": false,
                "intended_readme_surface_checked": false,
                "private_content_checked": false,
                "captions_timing_checked": captions_timing_checked,
                "reviewer": null,
                "notes": if self.clip.is_edit_erase() { "Synthetic edit-erase draft. The selected stroke and appearance are prepared by production handlers before the captured eraser and undo sequence; review playback and readability before use." } else if self.clip.is_mirror_shapes() { "Synthetic mirror-shapes draft. The mirrored line and ellipse are prepared through production handlers before the captured orbit; review playback and readability before use." } else if self.clip.is_curved_guides() { "Synthetic curved-guide draft. The strokes are scripted through production handlers before the captured orbit; review playback and visual readability before use." } else { "Synthetic replay draft. The GIF plays once to avoid a false loop seam; review MP4 playback and visual readability before use." }
            }
        });
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest_path)
            .map_err(|error| error.to_string())?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            self.remove_draft_outputs();
            return Err(error.to_string());
        }
        Ok(())
    }
}

struct AppearanceLayoutVerification {
    theme: crate::presentation::UiTheme,
    zoom_factor: f32,
    zoom_percent: u32,
    viewport_size: [u32; 2],
    state: String,
    screenshot_requested: bool,
}

struct InsertedDrawingTarget<'a> {
    doc: &'a mut Document,
    selected: &'a mut HashSet<u64>,
    undo: &'a mut Vec<Edit>,
    redo: &'a mut Vec<Edit>,
    next_id: &'a mut u64,
    dirty: &'a mut bool,
    image_assets: &'a mut HashMap<u64, crate::image_planes::ImageAsset>,
    obj_models: &'a mut HashMap<u64, crate::obj_reference::ObjModel>,
}

fn apply_inserted_drawing_state(
    target: InsertedDrawingTarget<'_>,
    inserted: native_merge::InsertedDrawing,
) -> usize {
    let native_merge::InsertedDrawing {
        edit: merge,
        next_id: inserted_next_id,
        image_assets: inserted_images,
        obj_models: inserted_models,
    } = inserted;
    let stroke_ids = merge.strokes.iter().map(|stroke| stroke.id).collect();
    let stroke_count = merge.strokes.len();
    let edit = Edit::Merge(merge);
    edit.apply(target.doc, true);
    target.undo.push(edit);
    target.redo.clear();
    *target.next_id = inserted_next_id;
    target.image_assets.extend(inserted_images);
    target.obj_models.extend(inserted_models);
    *target.selected = stroke_ids;
    *target.dirty = true;
    stroke_count
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertPreviewAction {
    Apply,
    Cancel,
}

struct InsertPreviewUiResponse {
    action: Option<InsertPreviewAction>,
    #[cfg(test)]
    preview_rect: Rect,
    #[cfg(test)]
    insert_button_rect: Rect,
    #[cfg(test)]
    cancel_button_rect: Rect,
}

fn show_insert_preview_window(
    ctx: &egui::Context,
    path: &std::path::Path,
    source: &Document,
    offset: &mut [f32; 3],
    insert_as_group: &mut bool,
) -> Option<InsertPreviewUiResponse> {
    egui::Window::new("Insert drawing")
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.label(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Native drawing"),
            );
            let point_count: usize = source.strokes.iter().map(|stroke| stroke.points.len()).sum();
            ui.label(format!(
                "{} strokes · {} control points · {} layers",
                source.strokes.len(),
                point_count,
                source.layers.len()
            ));
            let (mut minimum, mut maximum) =
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for point in source.strokes.iter().flat_map(|stroke| &stroke.points) {
                minimum = minimum.min(point.position);
                maximum = maximum.max(point.position);
            }
            let (preview_rect, _) =
                ui.allocate_exact_size(Vec2::new(320.0, 150.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(preview_rect, 4.0, Color32::from_gray(27));
            ui.painter().rect_stroke(
                preview_rect,
                4.0,
                egui::Stroke::new(1.0_f32, Color32::from_gray(82)),
                egui::StrokeKind::Inside,
            );
            if minimum.is_finite() {
                let world_offset = Vec3::from_array(*offset);
                minimum += world_offset;
                maximum += world_offset;
                let span = (maximum - minimum).max(Vec3::splat(0.01));
                let scale = ((preview_rect.width() - 20.0) / span.x)
                    .min((preview_rect.height() - 20.0) / span.y)
                    .max(0.001);
                let center = (minimum + maximum) * 0.5;
                let to_screen = |world: Vec3| {
                    egui::pos2(
                        preview_rect.center().x + (world.x - center.x) * scale,
                        preview_rect.center().y - (world.y - center.y) * scale,
                    )
                };
                let painter = ui.painter().with_clip_rect(preview_rect);
                for stroke in &source.strokes {
                    let color = Color32::from_rgb(
                        (stroke.color[0].clamp(0.0, 1.0) * 255.0) as u8,
                        (stroke.color[1].clamp(0.0, 1.0) * 255.0) as u8,
                        (stroke.color[2].clamp(0.0, 1.0) * 255.0) as u8,
                    );
                    for pair in stroke.points.windows(2) {
                        let start = to_screen(pair[0].position + world_offset);
                        let end = to_screen(pair[1].position + world_offset);
                        painter.line_segment(
                            [start, end],
                            egui::Stroke::new((stroke.radius * scale).clamp(1.0, 5.0), color),
                        );
                    }
                }
            } else {
                ui.painter().text(
                    preview_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No stroke geometry",
                    egui::FontId::proportional(13.0),
                    Color32::GRAY,
                );
            }
            ui.small("Top-view placement preview; guides and reference models retain their saved transforms.");
            for (index, axis) in ["X", "Y", "Z"].iter().enumerate() {
                ui.add(
                    egui::DragValue::new(&mut offset[index])
                        .speed(0.05)
                        .range(-10000.0..=10000.0)
                        .prefix(format!("{axis} offset "))
                        .suffix(" wu"),
                );
            }
            let can_group = source.layers.len() == 1
                && source.strokes.iter().all(|stroke| stroke.group_id.is_none());
            ui.add_enabled_ui(can_group, |ui| {
                ui.checkbox(insert_as_group, "Wrap inserted strokes in one group");
            });
            if !can_group {
                ui.small("Single-group insertion is available only for a single-layer source with no existing groups; native group structure is preserved otherwise.");
            }
            let (insert_button_rect, cancel_button_rect, action) = ui
                .horizontal(|ui| {
                    let insert = ui.button("Insert");
                    let cancel = ui.button("Cancel");
                    let action = if insert.clicked() {
                        Some(InsertPreviewAction::Apply)
                    } else if cancel.clicked() {
                        Some(InsertPreviewAction::Cancel)
                    } else {
                        None
                    };
                    (insert.rect, cancel.rect, action)
                })
                .inner;
            #[cfg(not(test))]
            let _ = (insert_button_rect, cancel_button_rect);
            InsertPreviewUiResponse {
                action,
                #[cfg(test)]
                preview_rect,
                #[cfg(test)]
                insert_button_rect,
                #[cfg(test)]
                cancel_button_rect,
            }
        })
        .and_then(|response| response.inner)
}

impl SketchApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui_style::apply(&cc.egui_ctx, crate::presentation::UiTheme::Dark);
        let (pen, input_status) = match PenInput::install(cc) {
            Ok(p) => (Some(p), "Native pen listener ready".into()),
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
            let arguments = std::env::args().collect::<Vec<_>>();
            let relative = if arguments.iter().any(|arg| arg == "--verify-presentation") {
                "verification/presentation/recovery"
            } else if arguments
                .iter()
                .any(|arg| arg == "--verify-camera-bookmarks")
            {
                "verification/camera-bookmarks-recovery"
            } else if arguments.iter().any(|arg| arg == "--verify-obj-references") {
                "verification/obj-reference/recovery"
            } else if arguments
                .iter()
                .any(|arg| arg == "--verify-appearance-layout")
            {
                "verification/appearance-layout/recovery"
            } else if arguments.iter().any(|arg| arg == "--verify") {
                "verification/smoke/recovery"
            } else {
                "verification/recovery"
            };
            std::env::current_dir()
                .map(|p| p.join(relative))
                .map_err(|e| e.to_string())
        } else {
            crate::recovery_store::path()
        };
        let recovery = crate::recovery::Recovery::new(recovery_path, cc.egui_ctx.clone());
        let camera_bookmark_path = crate::presets::path().ok().and_then(|path| {
            path.parent()
                .map(|parent| parent.join("camera-bookmarks.json"))
        });
        let (camera_bookmarks, camera_bookmark_error) = camera_bookmark_path
            .as_ref()
            .map(
                |path| match crate::camera_bookmarks::CameraBookmarkStore::load(path) {
                    Ok(store) => (store, None),
                    Err(error) => (Default::default(), Some(error)),
                },
            )
            .unwrap_or_else(|| {
                (
                    Default::default(),
                    Some("Camera bookmark storage is unavailable".into()),
                )
            });
        let image_asset_root = std::env::temp_dir().join(format!(
            "sketch3d-image-planes-{}",
            crate::recovery_store::session_id()
        ));
        let mut app = Self {
            drawing_library: crate::drawing_library::DrawingLibrary::new(),
            show_drawing_library: false,
            drawing_library_filter: String::new(),
            library_folder_filter: "All folders".into(),
            new_library_folder: String::new(),
            rename_library_source: None,
            rename_library_stem: String::new(),
            delete_library_source: None,
            library_open_path: None,
            insert_preview: None,
            insert_offset: [0.0; 3],
            insert_as_group: false,
            line: Default::default(),
            curve_tool: Default::default(),
            shape_tool: Default::default(),
            mirror_ui: Default::default(),
            show_view: false,
            view_verification_frame: None,
            presentation: presentation_ui::PresentationUi::new(),
            placement: Default::default(),
            fills: Default::default(),
            show_scene: false,
            lighting_verification_frame: None,
            tutorial: tutorial_ui::TutorialUi::new(),
            renderer,
            camera_bookmarks,
            camera_bookmark_path,
            camera_bookmark_error,
            camera_bookmark_restart_report: std::env::args()
                .position(|argument| argument == "--verify-camera-bookmarks")
                .and_then(|index| std::env::args().nth(index + 1))
                .map(PathBuf::from),
            camera_ui: Default::default(),
            camera_playback: None,
            liquify_ui: Default::default(),
            mixed_resources_ui: Default::default(),
            image_plane_assets: std::collections::HashMap::new(),
            obj_reference_models: std::collections::HashMap::new(),
            obj_surface_indices: std::collections::HashMap::new(),
            model_stroke_contact: None,
            selected_model_reference: None,
            obj_reference_window_open: false,
            obj_reference_verification_frame: None,
            obj_reference_verification_import_rect: None,
            obj_reference_verification_button_clicked: false,
            obj_reference_verification_source: None,
            obj_reference_verification_source_root: None,
            obj_reference_verification_asset_root: None,
            appearance_layout_verification: None,
            image_asset_root,
            guide_name: String::new(),
            loft_preview: None,
            palette_ui: palette_ui::PaletteUi::new(),
            reference_board: reference_board::ReferenceBoard::new(&cc.egui_ctx),
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
            image_plane_verification_frame: None,
            mirror_verification_frame: None,
            ring_verification_frame: None,
            model_surface_verification_frame: None,
            image_plane_verification_changed_pixels: 0,
            liquify_brush_verification_frame: None,
            liquify_brush_verification_before: None,
            mixed_resources_verification_frame: None,
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
            partial_eraser: Default::default(),
            select_mode: false,
            selected: HashSet::new(),
            transform_mode: Mode::Move,
            transform_values: [0.0; 3],
            scale_values: [1.0; 3],
            scale_uniform_lock: true,
            scale_brush_width: true,
            transform_pivot: selection::PivotMode::SelectionCenter,
            custom_pivot: Vec3::ZERO,
            drag: None,
            brush_select_enabled: false,
            brush_select_radius: 18.0,
            brush_select_through: false,
            brush_select_deselect: false,
            brush_select_drag: None,
            appearance_draft: None,
            active: None,
            active_started_at: None,
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
            show_diagnostics: false,
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
            video_export: Default::default(),
            construction_history: crate::construction_history::ConstructionHistory::load(),
            obj_export: Default::default(),
            png_verification_frame: None,
            capability_replay: None,
        };
        if let Some(index) = std::env::args().position(|a| a == "--record-capability-demo") {
            let arguments = std::env::args().collect::<Vec<_>>();
            let clip_name = arguments
                .get(index + 1)
                .expect("capability demo clip name is validated before app launch");
            let clip = CapabilityClip::parse(clip_name)
                .expect("capability demo clip name is validated before app launch");
            app.verification_frame = None;
            app.capability_replay = Some(CapabilityReplay {
                clip,
                output_dir: prepare_capability_demo_output(clip_name)
                    .expect("capability demo output path must be prepared before app launch"),
                initial_stroke_count: 0,
                initial_camera: app.camera,
                canvas_rect: None,
                captured_frames: 0,
                screenshot_pending: false,
                screenshot_requested: false,
                wait_updates: 0,
                frame_size: None,
                pixels_per_point: cc.egui_ctx.pixels_per_point(),
                pointer_cue: None,
                contact: false,
                barrel: false,
                eraser: false,
                setup_verified: !clip.is_curved_guides()
                    && !clip.is_mirror_shapes()
                    && !clip.is_edit_erase(),
                undo_applied: false,
                gesture_start: None,
                gesture_end: None,
                expected_undo_strokes: None,
                sender: None,
                encoder: None,
                mp4_sender: None,
                mp4_encoder: None,
            });
            app.stabilization = 0.0;
            if clip.is_curved_guides() {
                app.mirror_verification_frame = Some(0);
            } else if !clip.is_edit_erase() && !clip.is_mirror_shapes() {
                app.load_house();
                app.guide = DrawingPlane::from_preset(Plane::Front);
                app.guide.depth = app.camera.target.z;
                if let Some(replay) = app.capability_replay.as_mut() {
                    replay.initial_stroke_count = app.doc.strokes.len();
                    replay.initial_camera = app.camera;
                }
            }
        } else if std::env::args().any(|a| a == "--verify-liquify-brush") {
            app.verification_frame = None;
            app.liquify_brush_verification_frame = Some(0);
            app.doc = crate::export_check::brush_fixture();
            app.selected = HashSet::from([app.doc.strokes[0].id]);
            app.select_mode = true;
            app.camera.align(crate::geometry::Plane::Front);
        } else if std::env::args().any(|a| a == "--verify-model-surfaces") {
            app.verification_frame = None;
            app.model_surface_verification_frame = Some(0);
        } else if std::env::args().any(|a| a == "--verify-obj-references") {
            app.verification_frame = None;
            app.obj_reference_verification_frame = Some(0);
            app.obj_reference_window_open = true;
            app.camera.align(crate::geometry::Plane::Front);
            app.camera.target = Vec3::ZERO;
            app.camera.distance = 6.0;
        } else if let Some(index) =
            std::env::args().position(|argument| argument == "--verify-appearance-layout")
        {
            let arguments = std::env::args().collect::<Vec<_>>();
            let theme = arguments
                .get(index + 1)
                .map(String::as_str)
                .expect("--verify-appearance-layout requires a theme");
            let zoom_factor = arguments
                .get(index + 2)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|zoom| zoom.is_finite() && (0.5..=2.0).contains(zoom))
                .expect("--verify-appearance-layout requires a zoom between 0.5 and 2.0");
            let viewport_size = [
                arguments
                    .get(index + 3)
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|dimension| (640..=3840).contains(dimension))
                    .unwrap_or(1440),
                arguments
                    .get(index + 4)
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|dimension| (480..=2160).contains(dimension))
                    .unwrap_or(940),
            ];
            let state = arguments
                .get(index + 5)
                .map(String::as_str)
                .unwrap_or("workspace");
            let supported_states = [
                "workspace",
                "brush",
                "select",
                "layers",
                "color-picker",
                "library",
                "recovery-error",
                "export",
                "scene-dark-background",
                "scene-light-background",
                "scene-custom-background",
                "scene-image-background",
                "scene-mirror-preview-dark",
                "scene-mirror-preview-light",
                "scene-mirror-preview-custom",
                "scene-mirror-preview-image",
                "scene-brush-selection-drag-light",
                "scene-brush-selection-drag-image",
                "scene-mixed-resource-disabled",
                "scene-liquify-cursor-light",
                "scene-liquify-cursor-image",
            ];
            assert!(
                supported_states.contains(&state),
                "Unsupported appearance capture state: {state}"
            );
            app.presentation.preferences.ui_theme = match theme {
                "dark" => crate::presentation::UiTheme::Dark,
                "light" => crate::presentation::UiTheme::Light,
                _ => panic!("Appearance verification theme must be dark or light"),
            };
            app.appearance_layout_verification = Some(AppearanceLayoutVerification {
                theme: app.presentation.preferences.ui_theme,
                zoom_factor,
                zoom_percent: (zoom_factor * 100.0).round() as u32,
                viewport_size,
                state: state.to_owned(),
                screenshot_requested: false,
            });
            match state {
                "workspace" | "brush" => {}
                "select" | "layers" => {
                    app.doc = crate::export_check::brush_fixture();
                    app.selected = app.doc.strokes.iter().map(|stroke| stroke.id).collect();
                    app.select_mode = state == "select";
                    if state == "layers" {
                        app.presentation.preferences.layout.inspector_tab =
                            crate::presentation::InspectorTab::Layers;
                    }
                    app.camera.align(Plane::Front);
                }
                "color-picker" => app
                    .palette_ui
                    .open_editor_for_capture(ColorTarget::Brush, app.color),
                "library" => {
                    let fixture_dir = std::env::current_dir()
                        .unwrap()
                        .join("verification/appearance-layout/library-fixture");
                    std::fs::create_dir_all(&fixture_dir)
                        .expect("appearance library fixture directory");
                    let fixture_path = fixture_dir.join("Sample drawing.sketch.json");
                    let fixture = crate::export_check::brush_fixture();
                    std::fs::write(
                        &fixture_path,
                        serde_json::to_vec(&fixture).expect("appearance library fixture JSON"),
                    )
                    .expect("appearance library fixture writes");
                    app.drawing_library.record(&fixture_path);
                    app.show_drawing_library = true;
                }
                "recovery-error" => {}
                "export" => app.png_export.begin(),
                "scene-dark-background"
                | "scene-light-background"
                | "scene-custom-background"
                | "scene-image-background"
                | "scene-mirror-preview-dark"
                | "scene-mirror-preview-light"
                | "scene-mirror-preview-custom"
                | "scene-mirror-preview-image"
                | "scene-brush-selection-drag-light"
                | "scene-brush-selection-drag-image"
                | "scene-mixed-resource-disabled"
                | "scene-liquify-cursor-light"
                | "scene-liquify-cursor-image" => {
                    app.doc = crate::export_check::brush_fixture();
                    app.selected = app.doc.strokes.iter().map(|stroke| stroke.id).collect();
                    let mirror_preview = matches!(
                        state,
                        "scene-mirror-preview-dark"
                            | "scene-mirror-preview-light"
                            | "scene-mirror-preview-custom"
                            | "scene-mirror-preview-image"
                    );
                    app.select_mode = !mirror_preview;
                    if matches!(
                        state,
                        "scene-brush-selection-drag-light" | "scene-brush-selection-drag-image"
                    ) {
                        app.selected.clear();
                        app.brush_select_enabled = true;
                        app.brush_select_radius = 22.0;
                    }
                    if state == "scene-mixed-resource-disabled" {
                        app.doc.image_planes.push(crate::image_planes::ImagePlane {
                            id: 9101,
                            name: "Appearance verification image".into(),
                            asset_file: "image-9101.png".into(),
                            pixel_size: Some([2, 2]),
                            origin: Vec3::new(0.0, 0.0, 0.25),
                            rotation: glam::Quat::IDENTITY,
                            size: glam::Vec2::ONE,
                            opacity: 0.8,
                            visible: true,
                            locked: false,
                            drawable: true,
                        });
                        app.doc.guide_resources.push(
                            crate::document::guide_resources::GuideResource {
                                id: 9301,
                                name: "Appearance verification guide".into(),
                                visible: true,
                                locked: false,
                                geometry: crate::document::guide_resources::GuideGeometry::Plane {
                                    origin: Vec3::new(0.0, 0.0, 0.5),
                                    rotation: glam::Quat::IDENTITY,
                                },
                            },
                        );
                        app.mixed_resources_ui.selection =
                            crate::mixed_resources::ResourceSelection {
                                stroke_ids: HashSet::new(),
                                image_plane_ids: HashSet::from([9101]),
                                obj_reference_ids: HashSet::new(),
                                guide_ids: HashSet::from([9301]),
                            };
                        app.begin_mixed_resource_move();
                        assert!(app.mixed_resource_preview_active());
                        assert!(!app.mixed_resource_has_preview());
                        assert!(app.undo.is_empty());
                        assert!(!Self::mixed_transform_supported(
                            crate::mixed_resources::TransformMode::Rotate
                        ));
                        assert!(!Self::mixed_transform_supported(
                            crate::mixed_resources::TransformMode::Scale
                        ));
                    }
                    app.camera.align(Plane::Front);
                    app.presentation.preferences.background = match state {
                        "scene-dark-background"
                        | "scene-mixed-resource-disabled"
                        | "scene-image-background"
                        | "scene-mirror-preview-dark"
                        | "scene-mirror-preview-image"
                        | "scene-brush-selection-drag-image"
                        | "scene-liquify-cursor-image" => [0.035, 0.045, 0.06],
                        "scene-light-background"
                        | "scene-mirror-preview-light"
                        | "scene-liquify-cursor-light"
                        | "scene-brush-selection-drag-light" => [0.94, 0.95, 0.97],
                        "scene-custom-background" | "scene-mirror-preview-custom" => {
                            [0.46, 0.31, 0.68]
                        }
                        _ => unreachable!("appearance background state was validated above"),
                    };
                    if matches!(
                        state,
                        "scene-image-background"
                            | "scene-mirror-preview-image"
                            | "scene-brush-selection-drag-image"
                            | "scene-liquify-cursor-image"
                    ) {
                        let light = [238, 236, 224, 255];
                        let dark = [31, 42, 55, 255];
                        let pixels = (0..8)
                            .flat_map(|y| {
                                (0..8).flat_map(move |x| {
                                    if (x / 4 + y / 4) % 2 == 0 {
                                        light
                                    } else {
                                        dark
                                    }
                                })
                            })
                            .collect();
                        app.reference_board.add_capture_background(
                            &cc.egui_ctx,
                            u64::MAX - 1,
                            pixels,
                            [8, 8],
                        );
                        app.presentation.preferences.background_image =
                            Some(crate::presentation::BackgroundImageSettings {
                                reference_id: u64::MAX - 1,
                                fit: crate::presentation::BackgroundImageFit::Fill,
                                alignment: [0.5, 0.5],
                                opacity: 1.0,
                            });
                    }
                    if matches!(
                        state,
                        "scene-dark-background"
                            | "scene-light-background"
                            | "scene-custom-background"
                            | "scene-image-background"
                            | "scene-mirror-preview-dark"
                            | "scene-mirror-preview-light"
                            | "scene-mirror-preview-custom"
                            | "scene-mirror-preview-image"
                    ) {
                        app.line.enabled = true;
                        app.mirror_ui.enabled = true;
                    } else if matches!(
                        state,
                        "scene-liquify-cursor-light" | "scene-liquify-cursor-image"
                    ) {
                        let stroke_id = app
                            .doc
                            .strokes
                            .iter()
                            .find(|stroke| stroke.fill.is_none())
                            .map(|stroke| stroke.id)
                            .expect("Liquify cursor fixture contains an ordinary stroke");
                        app.selected = HashSet::from([stroke_id]);
                        app.begin_liquify();
                        app.activate_liquify_brush();
                    }
                }
                _ => unreachable!("appearance capture states are validated above"),
            }
        } else if std::env::args().any(|a| a == "--verify-mixed-resources") {
            app.verification_frame = None;
            app.mixed_resources_verification_frame = Some(0);
            app.doc = crate::export_check::brush_fixture();
            app.selected = HashSet::from([app.doc.strokes[0].id]);
            app.select_mode = true;
            let plane = crate::image_planes::ImagePlane {
                id: 9101,
                name: "Mixed verification image".into(),
                asset_file: "image-9101.png".into(),
                pixel_size: Some([2, 2]),
                origin: Vec3::new(0.0, 0.0, 0.25),
                rotation: glam::Quat::IDENTITY,
                size: glam::Vec2::ONE,
                opacity: 0.8,
                visible: true,
                locked: false,
                drawable: true,
            };
            app.doc.image_planes.push(plane);
            app.doc.active_image_plane = Some(9101);
            app.doc.model_references.push(
                crate::document::model_references::ObjReferenceResource {
                    id: 9201,
                    name: "Mixed verification model".into(),
                    visible: true,
                    locked: false,
                    center: Vec3::new(0.0, 0.0, -0.25),
                    rotation: glam::Quat::IDENTITY,
                    scale: 1.0,
                    asset: crate::obj_reference_storage::ObjReferenceAsset {
                        id: 9201,
                        bundle_dir: "models/9201".into(),
                    },
                },
            );
            app.doc
                .guide_resources
                .push(crate::document::guide_resources::GuideResource {
                    id: 9301,
                    name: "Mixed verification guide".into(),
                    visible: true,
                    locked: false,
                    geometry: crate::document::guide_resources::GuideGeometry::Plane {
                        origin: Vec3::new(0.0, 0.0, 0.5),
                        rotation: glam::Quat::IDENTITY,
                    },
                });
            app.mixed_resources_ui.selection = crate::mixed_resources::ResourceSelection {
                stroke_ids: HashSet::new(),
                image_plane_ids: HashSet::from([9101]),
                obj_reference_ids: HashSet::from([9201]),
                guide_ids: HashSet::from([9301]),
            };
            app.camera.align(crate::geometry::Plane::Front);
        } else if std::env::args().any(|a| a == "--verify-mirror") {
            app.verification_frame = None;
            app.mirror_verification_frame = Some(0);
        } else if std::env::args().any(|a| a == "--verify-ring") {
            app.verification_frame = None;
            app.ring_verification_frame = Some(0);
        } else if std::env::args().any(|a| a == "--verify-image-planes") {
            app.verification_frame = None;
            app.image_plane_verification_frame = Some(0);
        } else if std::env::args().any(|a| a == "--verify-lines") {
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
        app.renderer.grain = app.presentation.preferences.grain;
        app.renderer.toon_shading = app.presentation.preferences.toon_shading;
        app.renderer.post_effects = app.presentation.preferences.post_effects;
        app.recovery.request_scan();
        app.sync_presentation_background();
        app
    }

    fn sync_presentation_background(&mut self) {
        let Some(settings) = self.presentation.preferences.background_image else {
            self.renderer.clear_screen_background_image();
            return;
        };
        let Some((rgba, size)) = self.reference_board.background_image(settings.reference_id)
        else {
            self.renderer.clear_screen_background_image();
            self.presentation.preferences.background_image = None;
            self.presentation.remember();
            self.status =
                "Presentation background image is missing; cleared the saved reference.".into();
            return;
        };
        let fit = match settings.fit {
            crate::presentation::BackgroundImageFit::Fit => {
                crate::renderer::BackgroundImageFit::Fit
            }
            crate::presentation::BackgroundImageFit::Fill => {
                crate::renderer::BackgroundImageFit::Fill
            }
        };
        if let Err(error) = self.renderer.set_screen_background_image(
            settings.reference_id,
            rgba,
            size,
            fit,
            settings.alignment,
            settings.opacity,
        ) {
            self.renderer.clear_screen_background_image();
            self.status = format!("Cannot use presentation background image: {error}");
        }
    }
    fn current_tool_settings(&self) -> crate::construction_history::ToolSettingsSnapshot {
        let tool = if self.select_mode {
            "select"
        } else if self.eraser {
            "eraser"
        } else if self.fills.enabled {
            "fill"
        } else if self.line.enabled {
            "line"
        } else if self.curve_tool.enabled {
            "curve"
        } else if self.shape_tool.mode.is_some() {
            "shape"
        } else {
            "brush"
        };
        crate::construction_history::ToolSettingsSnapshot {
            tool: tool.into(),
            radius: self.radius,
            color: self.color,
            stabilization: self.stabilization,
            brush: self.brush,
        }
    }

    fn finish(&mut self) {
        let model_surface_stroke = self.model_stroke_contact.is_some();
        self.finish_appearance_edit(false);
        self.finish_partial_erase(true);
        self.cancel_line();
        self.resume_ink = false;
        self.model_stroke_contact = None;
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
        let stroke_duration_ms = self
            .active_started_at
            .take()
            .map(|started| started.elapsed().as_millis().min(u64::MAX as u128) as u64)
            .unwrap_or(0);
        if let Some(id) = self.active.take() {
            if let Some(stroke) = self
                .doc
                .strokes
                .last()
                .filter(|stroke| stroke.id == id)
                .cloned()
            {
                if model_surface_stroke {
                    self.commit_model_surface_stroke(stroke, stroke_duration_ms);
                } else {
                    self.commit_drawn_stroke(stroke, true, stroke_duration_ms);
                }
            }
        }
    }

    fn commit_model_surface_stroke(&mut self, stroke: Stroke, duration_ms: u64) {
        let Some(index) = self
            .doc
            .strokes
            .iter()
            .position(|item| item.id == stroke.id)
        else {
            return;
        };
        let recorded_stroke = stroke.clone();
        let edit = Edit::Change {
            before: Vec::new(),
            after: vec![(index, stroke)],
        };
        edit.apply(&mut self.doc, true);
        self.undo.push(edit);
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_stroke_labeled("Draw on model", &recorded_stroke, duration_ms);
        self.renderer.invalidate();
    }
    fn cancel(&mut self) {
        self.cancel_brush_selection();
        self.cancel_partial_erase();
        self.cancel_shape_draft();
        self.cancel_line();
        self.cancel_curve();
        self.palette_ui.sampling = false;
        self.sampling_contact = false;
        self.resume_ink = false;
        self.cancel_transform();
        if self.liquify_active() {
            self.finish_liquify(false);
        }
        self.stabilizer.reset();
        self.mouse_down = false;
        if let Some(id) = self.active.take() {
            self.doc.strokes.retain(|s| s.id != id);
        }
        self.active_started_at = None;
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
        if self.doc.active_model_reference.is_some() {
            self.add_model_point_with_tilt(position, pressure, rect, begin, tilt);
            return;
        }
        let ndc = glam::vec2(
            (position.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (position.y - rect.top()) / rect.height() * 2.0,
        );
        let (origin, direction) = self.camera.ray(ndc, rect.aspect_ratio());
        let active_image_plane = self.doc.active_image_plane.and_then(|id| {
            self.doc
                .image_planes
                .iter()
                .find(|plane| plane.id == id && plane.visible && !plane.locked && plane.drawable)
                .map(|plane| (plane.id, plane.rotation))
        });
        let image_hit = self.image_plane_hit(origin, direction);
        let saved_geometry = self.saved_guide_geometry().cloned();
        let saved_guide_hit = saved_geometry
            .as_ref()
            .and_then(|geometry| geometry.clone().hit(origin, direction));
        let saved_surface = self.saved_guide_surface();
        let saved_guide_frame = saved_guide_hit.and_then(|world| self.saved_guide_frame(world));
        let hit = if self.doc.active_guide.is_some() {
            saved_guide_hit
        } else if self.doc.active_image_plane.is_some() {
            image_hit.map(|(world, _)| world)
        } else if self.use_revolved {
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
            self.status = if self.doc.active_image_plane.is_some() {
                "No hit on the active image plane. Draw within its visible bounds or adjust its transform."
            } else if self.doc.active_guide.is_some() {
                "No hit on the active saved guide. Draw on its visible surface or adjust its transform."
            } else if self.use_cylinder || self.use_revolved {
                "No curved guide side hit. Draw on its wireframe or orbit; missed regions split the stroke."
            } else {
                "Plane is edge-on. Use Align view or rotate the camera."
            }.into();
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
            self.active_started_at = Some(Instant::now());
            self.stabilizer.start(if let Some(surface) = saved_surface {
                surface
            } else if active_image_plane.is_some() {
                Surface::Plane
            } else if self.use_revolved {
                Surface::Revolved(self.revolved)
            } else if self.use_cylinder {
                Surface::Cylinder(self.cylinder)
            } else {
                Surface::Plane
            });
            self.doc.strokes.push(Stroke {
                material: Default::default(),
                fill: None,
                group_id: None,
                layer_id: self.doc.active_layer,
                id,
                color: self.color,
                radius: self.radius,
                brush: Brush {
                    normal: if let Some((normal, _)) = saved_guide_frame {
                        normal
                    } else if let Some((_, rotation)) = active_image_plane {
                        rotation * Vec3::Z
                    } else if self.use_revolved {
                        self.revolved.normal(world)
                    } else if self.use_cylinder {
                        let local = self.cylinder.to_local(world);
                        self.cylinder.rotation()
                            * Vec3::new(local.x, 0.0, local.z).normalize_or_zero()
                    } else {
                        self.guide.basis().2
                    },
                    tangent: if let Some((_, tangent)) = saved_guide_frame {
                        tangent
                    } else if let Some((_, rotation)) = active_image_plane {
                        rotation * Vec3::X
                    } else if self.use_revolved {
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

    fn add_model_point_with_tilt(
        &mut self,
        position: Pos2,
        pressure: f32,
        rect: Rect,
        begin: bool,
        tilt: Option<Vec3>,
    ) {
        if begin {
            self.finish();
            self.model_stroke_contact = None;
        }
        let ndc = glam::vec2(
            (position.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (position.y - rect.top()) / rect.height() * 2.0,
        );
        let (origin, direction) = self.camera.ray(ndc, rect.aspect_ratio());
        let Some(id) = self.doc.active_model_reference else {
            return;
        };
        let Some(resource) = self
            .doc
            .model_references
            .iter()
            .find(|resource| resource.id == id && resource.visible && !resource.locked)
            .cloned()
        else {
            self.finish();
            self.model_stroke_contact = None;
            self.resume_ink = begin;
            self.status = "The active OBJ surface is hidden, locked, or missing.".into();
            return;
        };
        let Some(model) = self.obj_reference_models.get(&id) else {
            self.finish();
            self.model_stroke_contact = None;
            self.resume_ink = begin;
            self.status = "The active OBJ surface is not loaded.".into();
            return;
        };
        let index_result = match self.obj_surface_indices.entry(id) {
            Entry::Occupied(_) => None,
            Entry::Vacant(_) => Some(crate::obj_mesh_index::ObjMeshIndex::new(model)),
        };
        if let Some(result) = index_result {
            match result {
                Ok(index) => {
                    self.obj_surface_indices.insert(id, index);
                }
                Err(error) => {
                    self.finish();
                    self.model_stroke_contact = None;
                    self.resume_ink = begin;
                    self.status = format!("Cannot index OBJ surface: {error}");
                    return;
                }
            }
        }
        let inverse = resource.rotation.inverse();
        let local_origin = inverse * (origin - resource.center) / resource.scale;
        let local_direction = inverse * direction / resource.scale;
        let hit = self
            .obj_surface_indices
            .get(&id)
            .and_then(|index| index.hit(model, local_origin, local_direction));
        let Some(hit) = hit else {
            let resume = begin || self.active.is_some() || self.resume_ink;
            self.finish();
            self.model_stroke_contact = None;
            self.resume_ink = resume;
            self.status = "No OBJ surface hit. The missed region split the stroke.".into();
            return;
        };
        let world = resource.center + resource.rotation * (hit.position * resource.scale);
        if !world.is_finite() || world.abs().max_element() > 10_000.0 {
            self.finish();
            self.model_stroke_contact = None;
            self.resume_ink = true;
            return;
        }
        let connected = self.model_stroke_contact.is_some_and(
            |(previous_id, previous_triangle, previous_screen)| {
                previous_id == id
                    && self.obj_surface_indices.get(&id).is_some_and(|index| {
                        index.stroke_contacts_connect(
                            previous_triangle,
                            hit.triangle_index,
                            position.distance(previous_screen),
                        )
                    })
            },
        );
        if self.active.is_some() && !connected {
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
            self.active_started_at = Some(Instant::now());
            let normal = (resource.rotation * hit.normal).normalize_or_zero();
            let view_right = self.camera.basis().0;
            let tangent = (view_right - normal * view_right.dot(normal)).normalize_or_zero();
            self.doc.strokes.push(Stroke {
                material: Default::default(),
                fill: None,
                group_id: None,
                layer_id: self.doc.active_layer,
                id,
                color: self.color,
                radius: self.radius,
                brush: Brush {
                    normal,
                    tangent,
                    ..self.brush
                },
                points: Vec::new(),
            });
        }
        let Some(stroke_id) = self.active else {
            return;
        };
        let point = Point {
            tilt,
            position: world,
            pressure: pressure.clamp(0.0, 1.0),
        };
        if let Some(stroke) = self
            .doc
            .strokes
            .last_mut()
            .filter(|stroke| stroke.id == stroke_id)
        {
            if stroke.points.last().is_none_or(|last| {
                last.position.distance(point.position) > 0.001
                    || (last.pressure - point.pressure).abs() > 0.025
                    || last.tilt != point.tilt
            }) {
                stroke.points.push(point);
            }
        }
        self.model_stroke_contact = Some((id, hit.triangle_index, position));
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
            self.record_recovery_edit_labeled("Erase");
        }
    }
    fn undo(&mut self) {
        if self.mixed_resource_preview_active() {
            self.status = "Apply or cancel the mixed-resource preview before undoing.".into();
            return;
        }
        if self.liquify_active() {
            self.status = "Apply or cancel the Liquify preview before undoing.".into();
            return;
        }
        if self.fills.draft.is_some() {
            self.remove_fill_corner();
            return;
        }
        self.finish();
        if let Some(edit) = self.undo.pop() {
            let image_planes_changed = matches!(
                edit,
                Edit::ImagePlanes { .. } | Edit::MixedResources { .. } | Edit::Merge(_)
            );
            let models_changed = matches!(
                edit,
                Edit::ModelReferences { .. } | Edit::MixedResources { .. } | Edit::Merge(_)
            );
            edit.apply(&mut self.doc, false);
            self.redo.push(edit);
            self.dirty = true;
            self.record_recovery_edit_labeled("Undo");
            self.renderer.invalidate();
            if image_planes_changed {
                self.refresh_image_plane_textures();
            }
            if models_changed {
                self.refresh_obj_reference_meshes();
                self.refresh_image_plane_textures();
            }
            self.sync_selection();
        }
    }
    fn redo(&mut self) {
        if self.mixed_resource_preview_active() {
            self.status = "Apply or cancel the mixed-resource preview before redoing.".into();
            return;
        }
        if self.liquify_active() {
            self.status = "Apply or cancel the Liquify preview before redoing.".into();
            return;
        }
        if self.fills.draft.is_some() {
            self.status = "Finish or cancel the fill contour before redoing document edits.".into();
            return;
        }
        self.finish();
        if let Some(edit) = self.redo.pop() {
            let image_planes_changed = matches!(
                edit,
                Edit::ImagePlanes { .. } | Edit::MixedResources { .. } | Edit::Merge(_)
            );
            let models_changed = matches!(
                edit,
                Edit::ModelReferences { .. } | Edit::MixedResources { .. } | Edit::Merge(_)
            );
            edit.apply(&mut self.doc, true);
            self.undo.push(edit);
            self.dirty = true;
            self.record_recovery_edit_labeled("Redo");
            self.renderer.invalidate();
            if image_planes_changed {
                self.refresh_image_plane_textures();
            }
            if models_changed {
                self.refresh_obj_reference_meshes();
                self.refresh_image_plane_textures();
            }
            self.sync_selection();
        }
    }
    fn save(&mut self, save_as: bool) -> bool {
        if self.mixed_resource_preview_active() {
            self.status = "Apply or cancel the mixed-resource preview before saving.".into();
            return false;
        }
        if self.liquify_active() {
            self.status = "Apply or cancel the Liquify preview before saving.".into();
            return false;
        }
        if self.curve_draft_active() {
            self.status = "Finish or cancel the curve preview before saving.".into();
            return false;
        }
        if self.shape_draft_active() {
            self.status = "Release or cancel the shape preview before saving.".into();
            return false;
        }
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
                .add_filter("Sketch3D", &["json"])
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
            let asset_root = crate::image_plane_storage::asset_root(&path)?;
            crate::image_plane_storage::copy_bundle(
                &self.doc,
                &self.image_asset_root,
                &asset_root,
            )?;
            self.copy_obj_reference_bundles(&asset_root)?;
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
                self.drawing_library.record(&path);
                self.file = Some(path);
                self.image_asset_root =
                    crate::image_plane_storage::asset_root(self.file.as_deref().unwrap())
                        .unwrap_or_else(|_| self.image_asset_root.clone());
                if let Ok(models) = self.load_obj_reference_models(&self.image_asset_root) {
                    self.obj_reference_models = models;
                    self.refresh_obj_reference_meshes();
                }
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
        if self.video_export.is_running() {
            self.status =
                "Wait for the animated export to finish or cancel it before switching drawings."
                    .into();
            return;
        }
        if self.liquify_active() {
            self.status = "Apply or cancel the Liquify preview before switching drawings.".into();
            return;
        }
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
                self.loft_preview = None;
                self.image_plane_assets.clear();
                self.obj_reference_models.clear();
                self.obj_surface_indices.clear();
                self.selected_model_reference = None;
                self.renderer.clear_image_plane_assets();
                self.renderer.clear_obj_reference_models();
                self.image_asset_root = std::env::temp_dir().join(format!(
                    "sketch3d-image-planes-{}",
                    crate::recovery_store::session_id()
                ));
                self.file = None;
                self.dirty = false;
                self.undo.clear();
                self.redo.clear();
                self.selected.clear();
                self.renderer.invalidate();
                self.status = "New drawing".into();
            }
            Some(Pending::Open) => {
                let selected_path = self.library_open_path.take().or_else(|| {
                    rfd::FileDialog::new()
                        .add_filter("Sketch3D", &["json"])
                        .pick_file()
                });
                if let Some(path) = selected_path {
                    let result = (|| -> Result<LoadedDrawing, String> {
                        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                        if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
                            return Err("Document exceeds the prototype's 64 MB limit".into());
                        }
                        use std::io::Read;
                        let mut text = String::new();
                        file.read_to_string(&mut text).map_err(|e| e.to_string())?;
                        let document = Document::from_json(&text)?;
                        let assets = crate::image_plane_storage::load_assets(&document, &path)?;
                        let model_root = crate::image_plane_storage::asset_root(&path)?;
                        let models = document
                            .model_references
                            .iter()
                            .map(|resource| {
                                crate::obj_reference_storage::load_bundle(
                                    &model_root,
                                    &resource.asset,
                                )
                                .map(|model| (resource.id, model))
                            })
                            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
                        Ok((document, assets, models))
                    })();
                    match result {
                        Ok((doc, assets, models)) => {
                            self.reset_recovery_document();
                            self.next_id = doc
                                .strokes
                                .iter()
                                .map(|s| s.id)
                                .chain(doc.image_planes.iter().map(|plane| plane.id))
                                .chain(doc.guide_resources.iter().map(|guide| guide.id))
                                .chain(doc.model_references.iter().map(|model| model.id))
                                .max()
                                .unwrap_or(0)
                                .saturating_add(1);
                            self.doc = doc;
                            self.loft_preview = None;
                            self.image_plane_assets = assets;
                            self.obj_reference_models = models;
                            self.obj_surface_indices.clear();
                            self.selected_model_reference = None;
                            self.image_asset_root =
                                crate::image_plane_storage::asset_root(&path).unwrap_or_default();
                            self.renderer.clear_image_plane_assets();
                            for (id, asset) in &self.image_plane_assets {
                                let _ = self.renderer.set_image_plane_asset(*id, asset);
                            }
                            self.refresh_obj_reference_meshes();
                            self.starter_scene = false;
                            self.cylinder_example = false;
                            self.file = Some(path);
                            if let Some(saved_path) = &self.file {
                                self.drawing_library.record(saved_path);
                            }
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
                material: Default::default(),
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
    fn choose_tool(&mut self, tool: &str) {
        if self.resource_target_active()
            && matches!(tool, "Line" | "Fill" | "Circle" | "Ellipse" | "Curve")
        {
            self.status = "The active image/saved guide currently accepts brush strokes. Switch to the temporary guide for line, fill, shape, or curve tools.".into();
            return;
        }
        match tool {
            "Brush" => {
                self.finish();
                self.cancel_curve_tool();
                self.shape_tool.mode = None;
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.eraser = false;
                self.select_mode = false;
            }
            "Erase" => {
                self.finish();
                self.cancel_curve_tool();
                self.shape_tool.mode = None;
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.eraser = true;
                self.select_mode = false;
            }
            "Select" => {
                self.finish();
                self.cancel_curve_tool();
                self.shape_tool.mode = None;
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.eraser = false;
                self.select_mode = true;
            }
            "Fill" => {
                self.finish();
                self.cancel_curve_tool();
                self.shape_tool.mode = None;
                self.cancel_line_tool();
                self.fills.enabled = true;
                self.eraser = false;
                self.select_mode = false;
            }
            "Line" => {
                self.cancel_curve_tool();
                self.shape_tool.mode = None;
                self.start_line_tool();
            }
            "Circle" => {
                self.cancel_curve_tool();
                self.start_shape_tool(shape_tool::ShapeMode::Circle)
            }
            "Ellipse" => {
                self.cancel_curve_tool();
                self.start_shape_tool(shape_tool::ShapeMode::Ellipse)
            }
            "Curve" => {
                self.shape_tool.mode = None;
                self.start_curve_tool();
            }
            _ => {}
        }
    }

    fn tool_rail(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("tool-rail-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("TOOLS")
                        .small()
                        .color(ui_style::colors(ui.ctx()).muted_text),
                );
                for (tool, label, selected) in [
                    (
                        "Brush",
                        "Brush [B]",
                        !self.eraser
                            && !self.select_mode
                            && !self.fills.enabled
                            && !self.line.enabled
                            && !self.curve_tool.enabled
                            && self.shape_tool.mode.is_none(),
                    ),
                    ("Line", "Line [L]", self.line.enabled && !self.select_mode),
                    (
                        "Curve",
                        "Curve [C]",
                        self.curve_tool.enabled && !self.select_mode,
                    ),
                    (
                        "Circle",
                        "Circle",
                        self.shape_tool.mode == Some(shape_tool::ShapeMode::Circle),
                    ),
                    (
                        "Ellipse",
                        "Ellipse",
                        self.shape_tool.mode == Some(shape_tool::ShapeMode::Ellipse),
                    ),
                    ("Fill", "Fill [P]", self.fills.enabled && !self.select_mode),
                    ("Erase", "Erase [E]", self.eraser && !self.select_mode),
                    ("Select", "Select [V]", self.select_mode),
                ] {
                    if ui
                        .add_sized(
                            [ui.available_width(), 38.0],
                            egui::Button::new(label).selected(selected),
                        )
                        .clicked()
                    {
                        self.choose_tool(tool);
                    }
                }
                ui.add_space(8.0);
                if ui.button("Artwork [Tab]").clicked() {
                    self.toggle_artwork();
                }
            });
    }

    fn contextual_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(mode) = self.shape_tool.mode {
                ui.label(RichText::new(mode.label()).strong());
                if self.shape_tool.precise {
                    ui.label("Typed dimensions");
                } else {
                    ui.label("Drag on plane guide");
                }
            } else if self.select_mode {
                ui.label(RichText::new("Select").strong());
                for (mode, label) in [
                    (Mode::Move, "Move"),
                    (Mode::Rotate, "Rotate"),
                    (Mode::Scale, "Scale"),
                ] {
                    if ui
                        .selectable_value(&mut self.transform_mode, mode, label)
                        .changed()
                    {
                        self.finish();
                    }
                }
                ui.separator();
                ui.label(format!("{} selected", self.selected.len()));
            } else if self.fills.enabled {
                ui.label(RichText::new("Planar fill").strong());
                ui.label("Click enclosed regions on the active plane");
            } else if self.curve_tool.enabled {
                ui.label(RichText::new("Smooth curve").strong());
                ui.label("Click to add anchors; drag to adjust tangents; Enter to finish");
            } else if self.line.enabled {
                ui.label(RichText::new("Straight line").strong());
                ui.label("Drag from start to end; release to commit");
            } else {
                ui.label(RichText::new(if self.eraser { "Eraser" } else { "Brush" }).strong());
                if !self.eraser {
                    let compact_width = (ui.available_width() * 0.30).clamp(250.0, 360.0);
                    let control_height = ui.spacing().interact_size.y;
                    ui.allocate_ui_with_layout(
                        Vec2::new(compact_width, control_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui_style::slider_row(
                                ui,
                                "Radius",
                                "workspace-radius",
                                &mut self.radius,
                                0.001..=1.0,
                                1.0,
                                "wu",
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        Vec2::new(compact_width, control_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui_style::slider_row(
                                ui,
                                "Stabilize",
                                "workspace-stabilization",
                                &mut self.stabilization,
                                0.0..=1.0,
                                100.0,
                                "%",
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        Vec2::new(220.0, control_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui_style::field_row(ui, "Color", |ui| {
                                self.palette_ui
                                    .color_button(ui, ColorTarget::Brush, &self.color)
                            });
                        },
                    );
                } else {
                    ui.label("Erase whole strokes");
                }
            }
        });
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        if self.mixed_resource_preview_active() {
            self.mixed_resource_controls(ui);
            return;
        }
        self.line_controls(ui);
        self.curve_controls(ui);
        self.fill_controls(ui);
        self.shape_controls(ui);
        self.mirror_controls(ui);
        ui.add_space(6.0);
        if self.select_mode {
            self.group_controls(ui);
            self.mixed_resource_controls(ui);
        }
        self.palette_ui.controls(ui, &mut self.color);
        self.tutorial_hint(ui, "Brush library");
        if !self.fills.enabled && self.shape_tool.mode.is_none() && !self.curve_tool.enabled {
            self.preset_controls(ui);
        }
        if self.select_mode {
            self.tutorial_hint(ui, "Select");
            self.selection_panel(ui);
        } else if !self.fills.enabled && self.shape_tool.mode.is_none() && !self.curve_tool.enabled
        {
            if self.eraser {
                self.partial_eraser_controls(ui);
            }
            self.tutorial_hint(ui, "Brush");
            if !self.fills.enabled {
                self.brush_controls(ui);
            }
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
                .color(ui_style::colors(ui.ctx()).muted_text),
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
                .color(ui_style::colors(ui.ctx()).muted_text),
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
                .color(ui_style::colors(ui.ctx()).muted_text),
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
            ui_style::colors(ui.ctx()).accent
        } else {
            ui_style::colors(ui.ctx()).muted_text
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
            .rect_filled(rect, 4.0, ui_style::colors(ui.ctx()).input);
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
                egui::Stroke::new(1.5_f32, ui_style::colors(ui.ctx()).accent),
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
                .color(ui_style::colors(ui.ctx()).muted_text),
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

    fn show_drawing_library(&mut self, ctx: &egui::Context) {
        if self.presentation.artwork {
            return;
        }
        if !self.show_drawing_library {
            self.show_drawing_trash(ctx);
            return;
        }
        let mut selected = None;
        let mut copy_source = None;
        let mut backup_source = None;
        let mut delete_source = None;
        let mut restore_paths = None;
        let mut import_paths = None;
        let mut rename_source = None;
        let mut browse = false;
        let mut folder_changes = Vec::new();
        let mut create_folder = false;
        egui::Window::new("Drawing library")
            .open(&mut self.show_drawing_library)
            .default_width((ctx.screen_rect().width() * 0.8).clamp(460.0, 800.0))
            .max_width((ctx.screen_rect().width() - 32.0).max(200.0))
            .max_height((ctx.screen_rect().height() * 0.75).max(160.0))
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label("Recently saved or opened drawings. Files and their resource folders stay in their current locations.");
                ui.horizontal_wrapped(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut self.drawing_library_filter);
                    egui::ComboBox::from_id_salt("drawing-library-folder-filter")
                        .selected_text(&self.library_folder_filter)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.library_folder_filter, "All folders".into(), "All folders");
                            ui.selectable_value(&mut self.library_folder_filter, "Unsorted".into(), "Unsorted");
                            for folder in self.drawing_library.folders() {
                                ui.selectable_value(&mut self.library_folder_filter, folder.clone(), folder);
                            }
                        });
                    if ui.button("Browse…").clicked() {
                        browse = true;
                    }
                    if ui.button("Trash…").clicked() {
                        self.drawing_library.trash_open = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Import drawing…").clicked() {
                        if let Some(source) = rfd::FileDialog::new()
                            .add_filter("Sketch3D", &["json"])
                            .pick_file()
                        {
                            let file_name = source
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("drawing.sketch.json");
                            if let Some(destination) = rfd::FileDialog::new()
                                .add_filter("Sketch3D", &["json"])
                                .set_file_name(file_name)
                                .save_file()
                            {
                                import_paths = Some((source, destination));
                            }
                        }
                    }
                    if ui.button("Restore backup…").clicked() {
                        if let Some(bundle) = rfd::FileDialog::new().pick_folder() {
                            if let Some(destination) = rfd::FileDialog::new()
                                .add_filter("Sketch3D", &["json"])
                                .set_file_name("restored.sketch.json")
                                .save_file()
                            {
                                restore_paths = Some((bundle, destination));
                            }
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.text_edit_singleline(&mut self.new_library_folder);
                    if ui.button("Create folder").clicked() {
                        create_folder = true;
                    }
                });
                ui.separator();
                let filter = self.drawing_library_filter.trim().to_lowercase();
                let folder_filter = self.library_folder_filter.clone();
                let recent_paths = self.drawing_library.recent().to_vec();
                let list_height = (ctx.screen_rect().height() - 240.0).clamp(100.0, 360.0);
                egui::ScrollArea::vertical().max_height(list_height).show(ui, |ui| {
                    for path in &recent_paths {
                        let label = path.file_name().and_then(|name| name.to_str()).unwrap_or("Untitled drawing");
                        if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                            continue;
                        }
                        let current_folder = self.drawing_library.folder_for(path).map(str::to_owned);
                        if folder_filter != "All folders"
                            && (folder_filter == "Unsorted" && current_folder.is_some()
                                || folder_filter != "Unsorted" && current_folder.as_deref() != Some(folder_filter.as_str()))
                        {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                if let Some(texture) =
                                    self.drawing_library.thumbnail(ctx, path.as_path())
                                {
                                    ui.add(egui::Image::new((
                                        texture.id(),
                                        egui::vec2(96.0, 60.0),
                                    )));
                                } else {
                                    ui.allocate_ui(egui::vec2(96.0, 60.0), |ui| {
                                        ui.centered_and_justified(|ui| {
                                            ui.weak("Preview unavailable")
                                                .on_hover_text("The drawing is still loading, missing, or invalid.");
                                        });
                                    });
                                }
                                ui.label(label);
                                ui.small(path.display().to_string());
                                if !path.is_file() {
                                    ui.colored_label(ui_style::colors(ui.ctx()).error, "File is missing");
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                let mut selected_folder = current_folder.clone();
                                egui::ComboBox::from_id_salt(("drawing-library-folder", path.to_string_lossy().to_string()))
                                    .selected_text(selected_folder.as_deref().unwrap_or("Unsorted"))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut selected_folder, None, "Unsorted");
                                        for folder in self.drawing_library.folders() {
                                            ui.selectable_value(&mut selected_folder, Some(folder.clone()), folder);
                                        }
                                    });
                                if selected_folder != current_folder {
                                    folder_changes.push((path.clone(), selected_folder));
                                }
                                if ui.add_enabled(path.is_file(), egui::Button::new("Open")).clicked() {
                                    selected = Some(path.clone());
                                }
                                if ui.add_enabled(path.is_file(), egui::Button::new("Copy saved file…")).clicked() {
                                    copy_source = Some(path.clone());
                                }
                                if ui.add_enabled(path.is_file(), egui::Button::new("Backup…")).clicked() {
                                    backup_source = Some(path.clone());
                                }
                                let is_current = self.file.as_ref().is_some_and(|file| {
                                    file.canonicalize().ok() == path.canonicalize().ok()
                                });
                                let can_rename = path.is_file()
                                    && !(is_current && self.dirty)
                                    && self.recovered_source.is_none();
                                if ui.add_enabled(can_rename, egui::Button::new("Rename…")).clicked() {
                                    rename_source = Some(path.clone());
                                }
                                if ui.add_enabled(path.is_file() && !is_current, egui::Button::new("Delete…")).clicked() {
                                    delete_source = Some(path.clone());
                                }
                            });
                        });
                        ui.separator();
                    }
                });
                if let Some(notice) = &self.drawing_library.notice {
                    ui.small(notice);
                }
            });
        if let Some((bundle, destination)) = restore_paths {
            match crate::drawing_library::restore_native_backup(&bundle, &destination) {
                Ok(()) => {
                    self.drawing_library.record(&destination);
                    self.status = format!("Restored drawing to {}", destination.display());
                }
                Err(error) => self.status = format!("Restore failed: {error}"),
            }
        }
        if let Some((source, destination)) = import_paths {
            match crate::drawing_library::copy_native_drawing(&source, &destination) {
                Ok(()) => {
                    self.drawing_library.record(&destination);
                    self.status = format!("Imported drawing to {}", destination.display());
                }
                Err(error) => self.status = format!("Import failed: {error}"),
            }
        }
        if let Some(path) = rename_source {
            self.rename_library_stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_owned();
            self.rename_library_source = Some(path);
        }
        if let Some(path) = delete_source {
            self.delete_library_source = Some(path);
        }
        let mut delete_confirmed = false;
        let mut delete_cancelled = false;
        if let Some(path) = &self.delete_library_source {
            egui::Window::new("Move drawing to recoverable trash?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!("A validated backup will be kept in local application data before the drawing and its managed resources are removed.\n{}", path.display()));
                    ui.horizontal(|ui| {
                        delete_confirmed = ui.button("Move to trash").clicked();
                        delete_cancelled = ui.button("Cancel").clicked();
                    });
                });
        }
        if delete_cancelled {
            self.delete_library_source = None;
        } else if delete_confirmed {
            if let Some(source) = self.delete_library_source.take() {
                match crate::drawing_library::trash_native_drawing(&source) {
                    Ok(backup) => {
                        self.drawing_library.forget(&source);
                        self.status = format!("Moved drawing to local trash: {}", backup.display());
                    }
                    Err(error) => self.status = format!("Could not move drawing to trash: {error}"),
                }
            }
        }
        let mut rename_confirmed = false;
        let mut rename_cancelled = false;
        if self.rename_library_source.is_some() {
            egui::Window::new("Rename saved drawing")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        "The drawing and its managed resource folder will be renamed together.",
                    );
                    ui.text_edit_singleline(&mut self.rename_library_stem);
                    ui.horizontal(|ui| {
                        rename_confirmed = ui.button("Rename").clicked();
                        rename_cancelled = ui.button("Cancel").clicked();
                    });
                });
        }
        if rename_cancelled {
            self.rename_library_source = None;
        } else if rename_confirmed {
            if let Some(source) = self.rename_library_source.take() {
                let current_file = self
                    .file
                    .as_ref()
                    .is_some_and(|file| file.canonicalize().ok() == source.canonicalize().ok());
                match self
                    .drawing_library
                    .rename_drawing(&source, &self.rename_library_stem)
                {
                    Ok(path) => {
                        let mut reload_issue = None;
                        if current_file {
                            self.file = Some(path.clone());
                            if let Ok(root) = crate::image_plane_storage::asset_root(&path) {
                                self.image_asset_root = root;
                            }
                            match self.load_obj_reference_models(&self.image_asset_root) {
                                Ok(models) => {
                                    self.obj_reference_models = models;
                                    self.refresh_obj_reference_meshes();
                                }
                                Err(error) => reload_issue = Some(error),
                            }
                        }
                        self.status = reload_issue.map_or_else(
                            || format!("Renamed drawing to {}", path.display()),
                            |error| format!("Drawing renamed, but an OBJ reference could not be reloaded: {error}"),
                        );
                    }
                    Err(error) => self.status = format!("Rename failed: {error}"),
                }
            }
        }
        for (path, folder) in folder_changes {
            if let Err(error) = self.drawing_library.assign_folder(&path, folder.as_deref()) {
                self.status = format!("Cannot organize drawing: {error}");
            }
        }
        if create_folder {
            match self.drawing_library.create_folder(&self.new_library_folder) {
                Ok(()) => {
                    self.library_folder_filter = self.new_library_folder.trim().to_owned();
                    self.new_library_folder.clear();
                }
                Err(error) => self.status = error,
            }
        }
        if let Some(path) = selected {
            self.library_open_path = Some(path);
            self.show_drawing_library = false;
            self.request(Pending::Open);
        } else if let Some(source) = backup_source {
            if let Some(parent) = rfd::FileDialog::new().pick_folder() {
                let stem = source
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("drawing");
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|value| value.as_secs())
                    .unwrap_or(0);
                let destination = parent.join(format!("{stem}-{stamp}.sketch-backup"));
                match crate::drawing_library::backup_native_drawing(&source, &destination) {
                    Ok(()) => {
                        self.status =
                            format!("Created portable backup at {}", destination.display())
                    }
                    Err(error) => self.status = format!("Backup failed: {error}"),
                }
            }
        } else if let Some(source) = copy_source {
            let file_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("drawing.sketch.json");
            if let Some(destination) = rfd::FileDialog::new()
                .add_filter("Sketch3D", &["json"])
                .set_file_name(file_name)
                .save_file()
            {
                match crate::drawing_library::copy_native_drawing(&source, &destination) {
                    Ok(()) => {
                        self.drawing_library.record(&destination);
                        self.status = format!("Copied drawing to {}", destination.display());
                    }
                    Err(error) => self.status = format!("Copy failed: {error}"),
                }
            }
        } else if browse {
            self.show_drawing_library = false;
            self.request(Pending::Open);
        }
        self.show_drawing_trash(ctx);
    }

    fn show_drawing_trash(&mut self, ctx: &egui::Context) {
        if !self.drawing_library.trash_open || self.presentation.artwork {
            return;
        }
        let (entries, read_error) = match crate::drawing_library::list_trash() {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let output = drawing_trash_ui::show(
            ctx,
            self.drawing_library.trash_open,
            &entries,
            read_error
                .as_deref()
                .or(self.drawing_library.notice.as_deref()),
        );
        self.drawing_library.trash_open = output.open;
        if let Some((bundle, original_name)) = output.restore {
            let destination = rfd::FileDialog::new()
                .add_filter("Sketch3D", &["json"])
                .set_file_name(original_name)
                .save_file();
            if let Some(destination) = destination {
                match crate::drawing_library::restore_from_trash(&bundle, &destination) {
                    Ok(()) => {
                        self.drawing_library.record(&destination);
                        self.drawing_library.notice = None;
                        self.status = format!("Restored drawing to {}", destination.display());
                    }
                    Err(error) => {
                        let notice = format!("Restore failed: {error}");
                        self.drawing_library.notice = Some(notice.clone());
                        self.status = notice;
                    }
                }
            }
        }
    }

    fn begin_insert_drawing(&mut self) {
        if self.liquify_active()
            || self.line.draft.is_some()
            || self.curve_draft_active()
            || self.shape_draft_active()
            || self.fills.draft.is_some()
            || self.video_export.is_running()
        {
            self.status =
                "Finish or cancel the current preview/export before inserting a drawing.".into();
            return;
        }
        self.finish();
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Sketch3D", &["json"])
            .pick_file()
        else {
            return;
        };
        if self
            .file
            .as_ref()
            .is_some_and(|current| current.canonicalize().ok() == path.canonicalize().ok())
        {
            self.status = "Choose a different drawing to insert".into();
            return;
        }
        let loaded = (|| -> Result<Document, String> {
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > 64 * 1024 * 1024
            {
                return Err("Choose a regular native drawing no larger than 64 MiB".into());
            }
            use std::io::Read;
            let mut text = String::new();
            std::fs::File::open(&path)
                .map_err(|error| error.to_string())?
                .take(64 * 1024 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|error| error.to_string())?;
            Document::from_json(&text)
        })();
        match loaded {
            Ok(document) => {
                self.insert_preview = Some((path, document));
                self.insert_offset = [0.0; 3];
                self.insert_as_group = false;
            }
            Err(error) => self.status = format!("Cannot insert drawing: {error}"),
        }
    }

    fn show_insert_preview(&mut self, ctx: &egui::Context) {
        let Some((path, source)) = &self.insert_preview else {
            return;
        };
        let path = path.clone();
        let source = source.clone();
        let response = show_insert_preview_window(
            ctx,
            &path,
            &source,
            &mut self.insert_offset,
            &mut self.insert_as_group,
        );
        let action = response.and_then(|response| response.action);
        if action == Some(InsertPreviewAction::Cancel) {
            self.insert_preview = None;
        } else if action == Some(InsertPreviewAction::Apply) {
            if self.liquify_active()
                || self.line.draft.is_some()
                || self.curve_draft_active()
                || self.shape_draft_active()
                || self.fills.draft.is_some()
            {
                self.status = "Finish or cancel the current tool preview before inserting.".into();
                return;
            }
            let same_file = self
                .file
                .as_ref()
                .is_some_and(|current| current.canonicalize().ok() == path.canonicalize().ok());
            if same_file {
                self.status = "Choose a different drawing to insert".into();
                return;
            }
            self.insert_preview = None;
            let source_group = self.insert_as_group;
            let next_layer_id = self
                .doc
                .layers
                .iter()
                .map(|layer| layer.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let next_group_id = self
                .doc
                .strokes
                .iter()
                .filter_map(|stroke| stroke.group_id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let offset = Vec3::from_array(self.insert_offset);
            match native_merge::prepare_insert(
                &self.doc,
                &path,
                &self.image_asset_root,
                self.next_id,
                next_layer_id,
                next_group_id,
                offset,
                source_group,
                &self.obj_reference_models,
            ) {
                Ok(inserted) => {
                    let stroke_count = apply_inserted_drawing_state(
                        InsertedDrawingTarget {
                            doc: &mut self.doc,
                            selected: &mut self.selected,
                            undo: &mut self.undo,
                            redo: &mut self.redo,
                            next_id: &mut self.next_id,
                            dirty: &mut self.dirty,
                            image_assets: &mut self.image_plane_assets,
                            obj_models: &mut self.obj_reference_models,
                        },
                        inserted,
                    );
                    self.record_recovery_edit_labeled("Insert drawing");
                    self.refresh_image_plane_textures();
                    self.refresh_obj_reference_meshes();
                    self.renderer.invalidate();
                    self.status =
                        format!("Inserted {stroke_count} strokes from {}", path.display());
                }
                Err(error) => self.status = format!("Insert failed: {error}"),
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("File", |ui| {
            if ui.button("New drawing…   Ctrl+N").clicked() {
                ui.close_menu();
                self.request(Pending::New);
            }
            if ui.button("Open…   Ctrl+O").clicked() {
                ui.close_menu();
                self.request(Pending::Open);
            }
            if ui.button("Insert drawing…").clicked() {
                ui.close_menu();
                self.begin_insert_drawing();
            }
            if ui.button("Drawing library…").clicked() {
                ui.close_menu();
                self.finish();
                self.show_drawing_library = true;
            }
            ui.separator();
            if ui.button("Save   Ctrl+S").clicked() {
                ui.close_menu();
                self.save(false);
            }
            if ui.button("Save as…   Ctrl+Shift+S").clicked() {
                ui.close_menu();
                self.save(true);
            }
            ui.menu_button("Export", |ui| {
                if let Some(action) = export_ui::glb_export_scope_items(
                    ui,
                    self.can_export(false),
                    self.can_export_scene(),
                    self.can_export(true),
                )
                .action
                {
                    ui.close_menu();
                    let (selected, include_images) = action.scope();
                    self.export_dialog_with_images(selected, include_images, ctx);
                }
                if ui
                    .add_enabled(
                        self.can_export_png(),
                        egui::Button::new("Artwork image (.png / .jpg)…"),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.png_export.begin();
                }
                if ui
                    .add_enabled(
                        self.can_export_png() && !self.video_export.is_running(),
                        egui::Button::new("Animated GIF (.gif)…"),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.video_export.begin();
                }
                if ui.button("Construction history…").clicked() {
                    ui.close_menu();
                    self.video_export.begin_history_panel();
                }
                if ui
                    .add_enabled(
                        self.can_export(false),
                        egui::Button::new("Whole drawing (.obj)…"),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.obj_export.begin(false);
                }
                if ui
                    .add_enabled(
                        self.can_export(true),
                        egui::Button::new("Selected strokes (.obj)…"),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.obj_export.begin(true);
                }
            });
            ui.menu_button("Examples", |ui| {
                for (label, pending) in [
                    ("Japanese garden house", Pending::House),
                    ("Cylindrical lantern", Pending::Lantern),
                    ("Brush sampler", Pending::Brushes),
                    ("Pressure and dynamics", Pending::Dynamics),
                    ("Textured brush study", Pending::Textures),
                    ("Revolved vase", Pending::Vase),
                ] {
                    if ui.button(label).clicked() {
                        ui.close_menu();
                        self.request(pending);
                    }
                }
            });
        });

        ui.menu_button("Drawing", |ui| {
            if ui.button("OBJ references…").clicked() {
                ui.close_menu();
                self.finish();
                self.obj_reference_window_open = true;
            }
        });

        ui.menu_button("Edit", |ui| {
            if ui
                .add_enabled(
                    !self.undo.is_empty() || self.active.is_some() || self.drag.is_some(),
                    egui::Button::new("Undo   Ctrl+Z"),
                )
                .clicked()
            {
                ui.close_menu();
                self.undo();
            }
            if ui
                .add_enabled(
                    !self.redo.is_empty(),
                    egui::Button::new("Redo   Ctrl+Shift+Z"),
                )
                .clicked()
            {
                ui.close_menu();
                self.redo();
            }
        });

        ui.menu_button("View", |ui| {
            let mut selected_theme = None;
            ui.menu_button("Appearance", |ui| {
                for (theme, label) in [
                    (crate::presentation::UiTheme::Dark, "Dark"),
                    (crate::presentation::UiTheme::Light, "Light"),
                    (crate::presentation::UiTheme::System, "System"),
                ] {
                    if ui
                        .selectable_label(self.presentation.preferences.ui_theme == theme, label)
                        .clicked()
                    {
                        selected_theme = Some(theme);
                        ui.close_menu();
                    }
                }
                ui.small("System follows the OS theme when available; otherwise it uses Dark.");
            });
            if let Some(theme) = selected_theme {
                self.presentation.set_ui_theme(theme);
                self.presentation.apply_theme(ui.ctx());
            }
            if ui.button("Camera and guide controls").clicked() {
                ui.close_menu();
                self.finish();
                self.show_view = !self.show_view;
            }
            if ui.button("Artwork-only view   Tab").clicked() {
                ui.close_menu();
                self.toggle_artwork();
            }
            if ui.button("Pick a drawing plane   K").clicked() {
                ui.close_menu();
                self.begin_placement();
            }
            if ui.button("Scene lighting and ground").clicked() {
                ui.close_menu();
                self.finish();
                self.show_scene = !self.show_scene;
            }
            if ui.button("Camera bookmarks and sequences…").clicked() {
                ui.close_menu();
                self.finish();
                self.camera_ui.open = true;
            }
            ui.separator();
            if ui
                .button(if self.presentation.preferences.layout.inspector_open {
                    "Hide inspector"
                } else {
                    "Show inspector"
                })
                .clicked()
            {
                ui.close_menu();
                let open = !self.presentation.preferences.layout.inspector_open;
                self.set_inspector_open(open);
            }
            if ui.button("Reset workspace layout").clicked() {
                ui.close_menu();
                self.presentation.reset_layout();
            }
            if ui.button("Reset camera").clicked() {
                ui.close_menu();
                self.finish();
                self.camera = Camera::default();
            }
        });

        ui.menu_button("Help", |ui| {
            if ui.button("Drawing guide and shortcuts").clicked() {
                ui.close_menu();
                self.finish();
                self.show_help = true;
            }
            if ui.button("Start guided practice").clicked() {
                ui.close_menu();
                self.request(Pending::TutorialStart);
            }
            if ui.button("Pen diagnostics").clicked() {
                ui.close_menu();
                self.show_diagnostics = true;
            }
        });

        ui.separator();
        ui.label(
            RichText::new("SPATIAL / SKETCH")
                .strong()
                .color(ui_style::colors(ui.ctx()).accent),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(if self.dirty {
                "Unsaved changes"
            } else {
                "Saved / blank"
            });
            if self.export_job.is_some() || self.png_export.is_running() {
                ui.spinner();
            }
        });
    }

    fn inspector(&mut self, ctx: &egui::Context, enabled: bool) {
        if !self.presentation.preferences.layout.inspector_open || self.presentation.artwork {
            return;
        }
        let width = self.presentation.preferences.layout.inspector_width;
        let panel = egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(width)
            .min_width(260.0)
            .max_width(650.0)
            .show(ctx, |ui| {
                let mut properties_focus = None;
                let mut close_references = false;
                ui.horizontal(|ui| {
                    let properties_response = ui.selectable_value(
                        &mut self.presentation.preferences.layout.inspector_tab,
                        crate::presentation::InspectorTab::Properties,
                        "Properties",
                    );
                    properties_focus = Some(properties_response.id);
                    let properties_changed = properties_response.changed();
                    let layers_changed = ui
                        .selectable_value(
                            &mut self.presentation.preferences.layout.inspector_tab,
                            crate::presentation::InspectorTab::Layers,
                            "Layers",
                        )
                        .changed();
                    let references_changed = ui
                        .selectable_value(
                            &mut self.presentation.preferences.layout.inspector_tab,
                            crate::presentation::InspectorTab::References,
                            "References",
                        )
                        .changed();
                    let image_planes_changed = ui
                        .selectable_value(
                            &mut self.presentation.preferences.layout.inspector_tab,
                            crate::presentation::InspectorTab::ImagePlanes,
                            "Images",
                        )
                        .changed();
                    if properties_changed
                        || layers_changed
                        || references_changed
                        || image_planes_changed
                    {
                        self.presentation.schedule_layout_save();
                    }
                    if ui.button("Hide").on_hover_text("Hide inspector").clicked() {
                        self.set_inspector_open(false);
                    }
                });
                ui.separator();
                if let Some(notice) = &self.presentation.notice {
                    ui.colored_label(ui_style::colors(ui.ctx()).warning, notice);
                    ui.add_space(4.0);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        if self.mixed_resource_preview_active()
                            && self.presentation.preferences.layout.inspector_tab
                                != crate::presentation::InspectorTab::Properties
                        {
                            ui.label("Finish or cancel the mixed-resource preview first.");
                        } else {
                            match self.presentation.preferences.layout.inspector_tab {
                                crate::presentation::InspectorTab::Properties => {
                                    self.properties(ui)
                                }
                                crate::presentation::InspectorTab::Layers => {
                                    self.layer_controls(ui)
                                }
                                crate::presentation::InspectorTab::References => {
                                    let (sampled, close, background_changed) =
                                        self.reference_board.show(
                                            ui,
                                            ctx,
                                            &mut self.presentation.preferences.background_image,
                                        );
                                    if background_changed {
                                        self.presentation.remember();
                                        self.sync_presentation_background();
                                    }
                                    close_references = close;
                                    if let Some(color) = sampled {
                                        self.color = color;
                                        self.palette_ui.set_color_from_sample(color);
                                        self.status =
                                            "Reference color sampled into the brush color.".into();
                                    }
                                }
                                crate::presentation::InspectorTab::ImagePlanes => {
                                    self.image_plane_controls(ui)
                                }
                            }
                        }
                    });
                });
                if close_references {
                    self.presentation.preferences.layout.inspector_tab =
                        crate::presentation::InspectorTab::Properties;
                    if let Some(id) = properties_focus {
                        ctx.memory_mut(|memory| memory.request_focus(id));
                    }
                }
            });
        let new_width = panel.response.rect.width().clamp(260.0, 650.0);
        if (new_width - width).abs() > 0.5 {
            self.presentation.preferences.layout.inspector_width = new_width;
            self.presentation.schedule_layout_save();
        }
    }

    fn set_inspector_open(&mut self, open: bool) {
        if !open {
            self.finish();
            self.cancel_line_tool();
            self.cancel_fill_tool();
            self.cancel_transform();
        }
        self.presentation.preferences.layout.inspector_open = open;
        self.presentation.schedule_layout_save();
    }

    fn show_diagnostics_window(&mut self, ctx: &egui::Context) {
        if self.show_diagnostics && !self.presentation.artwork {
            let mut open = self.show_diagnostics;
            egui::Window::new("Pen diagnostics")
                .open(&mut open)
                .default_width(300.0)
                .max_height((ctx.screen_rect().height() - 80.0).max(200.0))
                .vscroll(true)
                .show(ctx, |ui| self.diagnostics(ui));
            self.show_diagnostics = open;
        }
    }

    fn show_color_editor(&mut self, ctx: &egui::Context) {
        match self.palette_ui.show_color_editor(ctx) {
            ColorAction::None => {}
            ColorAction::Sample(target) => self.start_color_sampling_for(target),
            ColorAction::Apply(target, color) => {
                self.finish();
                color_editor::apply_target(
                    target,
                    color,
                    &mut self.color,
                    &mut self.presentation.preferences.background,
                    &mut self.renderer.lighting.ground_color,
                );
                self.status = format!("Applied {}.", target.label());
            }
        }
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
        self.start_color_sampling_inner(ColorTarget::Brush, false);
    }

    fn start_color_sampling_for(&mut self, target: ColorTarget) {
        self.start_color_sampling_inner(target, true);
    }

    fn start_color_sampling_inner(&mut self, target: ColorTarget, staged: bool) {
        self.finish();
        self.cancel_placement();
        self.cancel_line_tool();
        self.cancel_fill_tool();
        self.select_mode = false;
        self.eraser = false;
        self.sampling_contact = false;
        if staged {
            self.palette_ui.start_sampling(target);
        } else {
            self.palette_ui.sampling_target = target;
            self.palette_ui.staged_sampling = false;
            self.palette_ui.sampling = true;
        }
        self.status = format!(
            "Sampling {}. Click visible artwork or Escape to cancel.",
            target.label().to_lowercase()
        );
    }

    fn cancel_color_sampling(&mut self) {
        let staged = self.palette_ui.staged_sampling;
        self.palette_ui.cancel_editor_sampling();
        self.select_mode = false;
        self.eraser = false;
        self.line.enabled = false;
        self.fills.enabled = false;
        self.status = if staged {
            "Color sampling cancelled. The color editor is ready.".into()
        } else {
            "Color sampling cancelled. Brush ready.".into()
        };
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
                    if self.palette_ui.staged_sampling {
                        self.palette_ui.set_color_from_sample(color);
                        self.status =
                            "Artwork color staged. Apply it in the color editor or cancel.".into();
                    } else {
                        color_editor::apply_target(
                            self.palette_ui.sampling_target,
                            color,
                            &mut self.color,
                            &mut self.presentation.preferences.background,
                            &mut self.renderer.lighting.ground_color,
                        );
                        self.palette_ui.sampling = false;
                        self.status = "Artwork color sampled. Ready to draw with it.".into();
                    }
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
            self.eraser_input(pos, rect, begin);
        } else if self.shape_tool.mode.is_some() {
            self.shape_input(pos, pressure, rect, begin);
        } else if self.curve_tool.enabled {
            self.curve_input(pos, pressure, rect, begin);
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
        if self.video_export.is_running() {
            return;
        }
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
            && self.appearance_layout_verification.is_none()
            && self.verification_frame.is_none()
            && self.dynamics_verification_frame.is_none()
            && self.texture_verification_frame.is_none()
            && self.preset_verification_frame.is_none()
            && self.recovery_verification_frame.is_none()
            && self.layer_verification_frame.is_none()
            && self.image_plane_verification_frame.is_none()
            && self.ring_verification_frame.is_none()
            && self.model_surface_verification_frame.is_none()
            && self.obj_reference_verification_frame.is_none()
            && self.vase_verification_frame.is_none()
            && self.group_verification_frame.is_none()
            && self.tutorial.verification_frame.is_none()
            && self.lighting_verification_frame.is_none()
            && self.fills.verification_frame.is_none()
            && self.placement.verification_frame.is_none()
            && self.presentation.verification_frame.is_none()
            && self.view_verification_frame.is_none()
            && self.line.verification_frame.is_none()
            && self.liquify_brush_verification_frame.is_none()
            && self.mirror_verification_frame.is_none()
            && self.capability_replay.is_none()
        {
            self.cancel();
            return;
        }
        let blocked = self.confirm
            || self.show_help
            || self.tutorial.offer
            || self.recovery_prompt
            || self.close_after_recovery
            || self.preset_ui.browser_is_open()
            || self.palette_ui.color_editor_active()
            || self.camera_playback.is_some()
            || self.mixed_resource_preview_active()
            || (self.liquify_active() && !self.liquify_brush_active());
        let inside_canvas = |p| canvas_pointer_allowed(ctx, rect, p);
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
                self.cancel_brush_selection();
                self.cancel_partial_erase();
                self.cancel_shape_draft();
                self.finish();
                self.mouse_down = false;
                self.pen_previous = None;
                self.cancel_transform();
                continue;
            }
            if self.liquify_brush_active() {
                if sample.phase == Phase::Cancel || sample.eraser {
                    self.finish_liquify(false);
                } else if sample.phase != Phase::Move || sample.contact {
                    self.liquify_brush_input(
                        sample.position,
                        sample.pressure.unwrap_or(1.0),
                        rect,
                        sample.phase,
                    );
                }
                continue;
            }
            if sample.phase == Phase::Up {
                self.sampling_contact = false;
                let curve_contact = self.curve_tool.enabled;
                if curve_contact {
                    self.curve_input(sample.position, sample.pressure.unwrap_or(1.0), rect, false);
                    self.curve_pointer_up();
                }
                if self.partial_erase_active() {
                    self.partial_erase_input(sample.position, rect, false);
                    self.finish_partial_erase(true);
                }
                if self.line.draft.is_some() {
                    if inside {
                        self.release_line(sample.position, rect);
                    } else {
                        self.cancel_line();
                    }
                }
                if self.shape_draft_active() {
                    if inside {
                        self.release_shape(sample.position, sample.pressure.unwrap_or(1.0), rect);
                    } else {
                        self.cancel_shape_draft();
                    }
                }
                if self.drag.is_some() || self.brush_select_drag.is_some() {
                    self.select_pointer(sample.position, rect, false, shift);
                }
                self.commit_brush_selection();
                if !curve_contact {
                    self.finish();
                }
                continue;
            }
            if !inside
                && self.drag.is_none()
                && self.brush_select_drag.is_none()
                && !self.partial_erase_active()
                && !self.shape_draft_active()
            {
                self.finish();
                continue;
            }
            if space || sample.barrel {
                if sample.contact {
                    if self.liquify_brush_active() {
                        self.cancel_liquify_gesture();
                    }
                    if let Some(prev) = previous {
                        self.navigate(sample.position - prev, shift, alt, rect);
                    }
                }
                continue;
            }
            if sample.contact {
                if sample.eraser && !self.placement.active && !self.presentation.artwork {
                    self.eraser_input(sample.position, rect, sample.phase == Phase::Down);
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
            self.cancel_brush_selection();
            self.cancel_partial_erase();
            self.cancel_shape_draft();
            self.mouse_down = false;
            self.pen_previous = None;
            self.cancel_transform();
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
                self.cancel_liquify_for_navigation();
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
                            if self.liquify_brush_active() {
                                self.liquify_brush_input(pos, 1.0, rect, Phase::Down);
                            } else {
                                self.pointer_action(pos, 1.0, rect, true, modifiers.shift);
                            }
                        }
                        egui::Event::PointerMoved(pos) if self.mouse_down => {
                            if self.liquify_brush_active() {
                                self.liquify_brush_input(pos, 1.0, rect, Phase::Move);
                            } else if inside_canvas(pos)
                                || ((self.drag.is_some()
                                    || self.brush_select_drag.is_some()
                                    || self.partial_erase_active()
                                    || self.shape_draft_active())
                                    && !rect.contains(pos))
                            {
                                self.pointer_action(pos, 1.0, rect, false, shift);
                            } else {
                                self.mouse_down = false;
                                self.cancel_brush_selection();
                                self.cancel_partial_erase();
                                self.finish();
                            }
                        }
                        egui::Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            ..
                        } => {
                            // Inspector and toolbar clicks also produce a
                            // primary-button release event. Only finish a
                            // canvas gesture when its press began inside the
                            // canvas; otherwise this can cancel staged edits
                            // such as the selected-stroke appearance editor.
                            if !self.mouse_down {
                                continue;
                            }
                            if self.liquify_brush_active() {
                                self.liquify_brush_input(pos, 1.0, rect, Phase::Up);
                                self.mouse_down = false;
                                continue;
                            }
                            self.sampling_contact = false;
                            if self.partial_erase_active() {
                                self.partial_erase_input(pos, rect, false);
                                self.finish_partial_erase(true);
                            } else if self.shape_draft_active() {
                                if inside_canvas(pos) {
                                    self.release_shape(pos, 1.0, rect);
                                } else {
                                    self.cancel_shape_draft();
                                }
                            } else if self.drag.is_some() || self.brush_select_drag.is_some() {
                                self.select_pointer(pos, rect, false, shift);
                            }
                            self.commit_brush_selection();
                            self.mouse_down = false;
                            if self.line.draft.is_some() {
                                if inside_canvas(pos) {
                                    self.release_line(pos, rect);
                                } else {
                                    self.cancel_line();
                                }
                            }
                            if self.curve_tool.enabled {
                                if inside_canvas(pos) {
                                    self.curve_input(pos, 1.0, rect, false);
                                }
                                self.curve_pointer_up();
                            } else {
                                self.finish();
                            }
                        }
                        egui::Event::PointerGone => {
                            self.mouse_down = false;
                            if self.liquify_brush_active() {
                                self.finish_liquify(false);
                                continue;
                            }
                            self.sampling_contact = false;
                            self.cancel_brush_selection();
                            self.cancel_partial_erase();
                            self.cancel_shape_draft();
                            self.cancel_curve();
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
        if let Some(verification) = &self.appearance_layout_verification {
            ctx.set_zoom_factor(verification.zoom_factor);
        }
        self.presentation.apply_theme(ctx);
        self.presentation.poll_layout_save();
        self.poll_recovery(ctx);
        if self
            .appearance_layout_verification
            .as_ref()
            .is_some_and(|verification| verification.state == "recovery-error")
        {
            self.recovery.scan = Some(crate::recovery_store::Scan {
                candidates: Vec::new(),
                issues: vec![crate::recovery_store::Issue {
                    path: PathBuf::from("verification/unreadable-recovery.json"),
                    message: "Snapshot could not be read: invalid verification fixture.".into(),
                }],
            });
            self.recovery.notice = "One recovery snapshot could not be read.".into();
            self.recovery_scan_seen = true;
            self.recovery_prompt = true;
        }
        self.ensure_workspace();
        if let Some(report_path) = self.camera_bookmark_restart_report.take() {
            if let Err(error) = crate::camera_bookmark_restart_check::verify_loaded(
                &self.camera_bookmarks,
                &report_path,
            ) {
                eprintln!("Camera bookmark restart verification failed: {error}");
            }
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.poll_export(ctx);
        if let Some(status) = self.video_export.render_next(&mut self.renderer, ctx) {
            self.status = status;
        }
        if let Some(status) = self.obj_export.poll(ctx) {
            self.status = status;
        }
        if let Some(status) = self.png_export.poll(ctx) {
            self.status = status;
        }
        self.capture_verification(ctx);
        self.capture_capability_replay_screenshot(ctx);
        if let Some(verification) = &mut self.appearance_layout_verification {
            if !verification.screenshot_requested
                && (ctx.zoom_factor() - verification.zoom_factor).abs() < 0.001
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                verification.screenshot_requested = true;
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.leave_artwork();
        }
        if self.palette_ui.sampling && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.cancel_color_sampling();
        }
        if self.loft_preview.is_some() && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.loft_preview = None;
            self.renderer.invalidate();
            self.status = "Loft preview cancelled.".into();
        }
        self.partial_eraser_shortcuts(ctx);
        self.shape_shortcuts(ctx);
        self.tick_camera_sequence(ctx);
        if self.liquify_active() && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.finish_liquify(false);
        }
        if self.mixed_resource_preview_active() && ctx.input(|input| input.key_pressed(Key::Escape))
        {
            self.finish_mixed_resource_move(false);
        }
        let keep_mixed_preview_for_appearance_capture = self
            .appearance_layout_verification
            .as_ref()
            .is_some_and(|verification| verification.state == "scene-mixed-resource-disabled");
        if self.mixed_resource_preview_active()
            && !keep_mixed_preview_for_appearance_capture
            && !ctx.input(|input| input.focused)
        {
            self.finish_mixed_resource_move(false);
        }
        if !self.liquify_active() && self.camera_playback.is_none() {
            self.curve_shortcuts(ctx);
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.capability_replay.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if ctx.input(|i| i.viewport().close_requested())
            && self.capability_replay.is_none()
            && !self.allow_close
            && (self.export_job.is_some()
                || self.png_export.is_running()
                || self.video_export.is_running()
                || self.obj_export.is_running())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "An export is running. Finish or cancel it, then close again.".into();
        } else if ctx.input(|i| i.viewport().close_requested())
            && self.capability_replay.is_none()
            && !self.allow_close
        {
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
            && !self.liquify_active()
            && !self.mixed_resource_preview_active()
            && self.camera_playback.is_none()
            && self.repeat.is_none()
            && !self.palette_ui.sampling
            && !self.palette_ui.color_editor_active()
            && !self.preset_ui.browser_is_open()
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
            && !self.liquify_active()
            && !self.mixed_resource_preview_active()
            && self.camera_playback.is_none()
            && self.repeat.is_none()
            && !self.palette_ui.sampling
            && !self.palette_ui.color_editor_active()
            && !self.preset_ui.browser_is_open()
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
            && !self.liquify_active()
            && !self.mixed_resource_preview_active()
            && self.camera_playback.is_none()
            && self.repeat.is_none()
            && !self.palette_ui.sampling
            && !self.palette_ui.color_editor_active()
            && !self.preset_ui.browser_is_open()
            && !ctx.wants_keyboard_input()
        {
            self.line_shortcuts(ctx);
            self.fill_shortcuts(ctx);
            self.group_shortcuts(ctx);
            if self.presentation.preferences.layout.inspector_tab
                != crate::presentation::InspectorTab::References
            {
                self.selection_shortcuts(ctx);
            }
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
            let enabled = !self.confirm
                && !self.recovery_prompt
                && !self.close_after_recovery
                && !self.tutorial.offer
                && !self.placement.active
                && !self.video_export.is_running()
                && !self.mixed_resource_preview_active();
            egui::TopBottomPanel::top("menu-bar").show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    egui::menu::bar(ui, |ui| self.menu_bar(ui, ctx));
                });
            });
            egui::TopBottomPanel::top("context-options").show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| self.contextual_options(ui));
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
                        if ui.button("Pen diagnostics").clicked() {
                            self.show_diagnostics = true;
                        }
                        ui.label(self.active_guide_label());
                    });
                });
            });
            egui::SidePanel::left("tool-rail")
                .exact_width(116.0)
                .show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| self.tool_rail(ui));
                });
            self.inspector(ctx, enabled || self.mixed_resource_preview_active());
        } else {
            self.artwork_bar(ctx);
        }
        self.camera_bookmark_controls(ctx);
        self.show_obj_reference_window(ctx);
        self.repeat_modal(ctx);
        self.png_export.show(ctx);
        let history_source_key = self.file.as_ref().map_or_else(
            || "untitled".to_owned(),
            |path| {
                path.canonicalize()
                    .unwrap_or_else(|_| path.clone())
                    .to_string_lossy()
                    .into_owned()
            },
        );
        let history_tool_settings = self.current_tool_settings();
        self.video_export.show(
            ctx,
            &self.camera_bookmarks.sequences,
            &mut self.construction_history,
            &self.doc,
            video_export_ui::HistoryRecordingContext {
                source_key: &history_source_key,
                camera: &self.camera,
                tool_settings: &history_tool_settings,
            },
        );
        self.obj_export.show(ctx);
        let pen = self.pen.as_mut().map(|p| p.drain()).unwrap_or_default();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                let appearance_state = self
                    .appearance_layout_verification
                    .as_ref()
                    .map(|verification| verification.state.clone());
                if matches!(
                    appearance_state.as_deref(),
                    Some("scene-brush-selection-drag-light" | "scene-brush-selection-drag-image")
                ) && self.brush_select_drag.is_none()
                {
                    let stroke = self
                        .doc
                        .strokes
                        .first()
                        .expect("brush-selection appearance fixture contains a stroke");
                    let stroke_id = stroke.id;
                    let start =
                        crate::gizmo::project(&self.camera, rect, stroke.points[2].position)
                            .expect("brush-selection start projects into the canvas")
                            .0
                            + egui::vec2(0.0, 32.0);
                    let end = crate::gizmo::project(
                        &self.camera,
                        rect,
                        stroke.points[stroke.points.len() - 3].position,
                    )
                    .expect("brush-selection end projects into the canvas")
                    .0 - egui::vec2(0.0, 32.0);
                    self.select_pointer(start, rect, true, false);
                    self.select_pointer(end, rect, false, false);
                    assert!(
                        self.brush_select_drag
                            .as_ref()
                            .is_some_and(|drag| drag.path_len() >= 2),
                        "brush-selection appearance capture must keep a live drag path"
                    );
                    assert!(
                        self.selected.contains(&stroke_id),
                        "brush-selection appearance capture must select the crossed fixture stroke"
                    );
                }
                if matches!(
                    appearance_state.as_deref(),
                    Some(
                        "scene-mirror-preview-dark"
                            | "scene-mirror-preview-light"
                            | "scene-mirror-preview-custom"
                            | "scene-mirror-preview-image"
                    )
                ) && self.line.draft.is_none()
                {
                    let start = rect.center() + egui::vec2(-160.0, 45.0);
                    let end = rect.center() + egui::vec2(-50.0, -45.0);
                    self.line_input(start, 0.75, None, rect, true);
                    self.line_input(end, 0.75, None, rect, false);
                    let draft =
                        self.line.draft.as_ref().expect(
                            "appearance mirror preview line draft starts on the active guide",
                        );
                    let mirrors = self
                        .mirrored_runs_for(&draft.mark)
                        .expect("appearance mirror preview geometry is supported");
                    assert!(
                        !mirrors.is_empty(),
                        "appearance mirror preview must contain reflected geometry"
                    );
                }
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
                self.verify_image_planes(ctx, rect);
                self.verify_mirror(ctx, rect);
                if let Some(replay) = self
                    .capability_replay
                    .as_mut()
                    .filter(|replay| replay.clip.is_curved_guides() && !replay.setup_verified)
                {
                    let guide = self
                        .doc
                        .guide_resources
                        .iter()
                        .find(|guide| Some(guide.id) == self.doc.active_guide)
                        .expect("curved-guide replay verifier adds an active saved guide");
                    assert!(
                        matches!(
                            &guide.geometry,
                            crate::document::guide_resources::GuideGeometry::Cylinder { .. }
                        ) && self.doc.strokes.len() == 2,
                        "curved-guide replay setup retains the verified cylinder and mirrored stroke pair"
                    );
                    replay.initial_stroke_count = self.doc.strokes.len();
                    replay.initial_camera = self.camera;
                    replay.setup_verified = true;
                }
                let prepare_mirror_shapes = self
                    .capability_replay
                    .as_ref()
                    .is_some_and(|replay| {
                        replay.clip.is_mirror_shapes() && !replay.setup_verified
                    });
                if prepare_mirror_shapes {
                    let stroke_count = self.prepare_capability_mirror_shapes(rect);
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.initial_stroke_count = stroke_count;
                    replay.initial_camera = self.camera;
                    replay.setup_verified = stroke_count == 3
                        && self.mirror_ui.enabled
                        && self.doc.strokes.last().is_some_and(|stroke| {
                            stroke.points.len() > 8 && stroke.fill.is_none()
                        });
                    assert!(
                        replay.setup_verified,
                        "mirror-shapes setup retains mirrored line pair and editable ellipse"
                    );
                }
                let prepare_edit_erase = self
                    .capability_replay
                    .as_ref()
                    .is_some_and(|replay| {
                        replay.clip.is_edit_erase() && !replay.setup_verified
                    });
                if prepare_edit_erase {
                    let (gesture_start, gesture_end) =
                        self.prepare_capability_edit_erase(rect);
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.initial_stroke_count = self.doc.strokes.len();
                    replay.initial_camera = self.camera;
                    replay.gesture_start = Some(gesture_start);
                    replay.gesture_end = Some(gesture_end);
                    replay.expected_undo_strokes = Some(self.doc.strokes.clone());
                    replay.setup_verified = self.doc.strokes.len() == 1
                        && self.selected.len() == 1
                        && self.undo.len() == 1
                        && self.eraser
                        && self.partial_eraser.enabled;
                    assert!(
                        replay.setup_verified,
                        "edit-erase setup creates, selects, and changes one editable stroke"
                    );
                }
                let undo_edit_erase = self.capability_replay.as_ref().is_some_and(|replay| {
                    replay.clip.is_edit_erase()
                        && replay.setup_verified
                        && replay.captured_frames == 34
                        && !replay.undo_applied
                });
                if undo_edit_erase {
                    self.undo();
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.undo_applied = replay.expected_undo_strokes.as_ref()
                        == Some(&self.doc.strokes)
                        && self.undo.len() == 1;
                    assert!(replay.undo_applied, "undo restores the selected edited stroke");
                }
                self.verify_ring(ctx, rect);
                self.verify_model_surfaces(ctx, rect);
                self.verify_obj_references(ctx, rect);
                self.verify_liquify_brush(ctx, rect);
                self.verify_mixed_resources(ctx);
                self.verify_textures(ctx, rect);
                self.verify_dynamics(ctx, rect);
                self.verify_brushes(ctx, rect);
                self.verify_cylinder(ctx, rect);
                self.verify_house(ctx);
                self.verify_canvas(ctx, rect);
                let replay_pen = self
                    .capability_replay
                    .as_mut()
                    .and_then(|replay| replay.next_sample(rect));
                self.canvas_input(ctx, rect, replay_pen.map_or(pen, |sample| vec![sample]));
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
                let cursor_pointer = if matches!(
                    appearance_state.as_deref(),
                    Some("scene-liquify-cursor-light" | "scene-liquify-cursor-image")
                ) {
                    Some(rect.center())
                } else {
                    ctx.pointer_hover_pos()
                };
                let cursor_painted = self.paint_liquify_cursor(ui.painter(), rect, cursor_pointer);
                if matches!(
                    appearance_state.as_deref(),
                    Some("scene-liquify-cursor-light" | "scene-liquify-cursor-image")
                ) {
                    cursor_painted.unwrap_or_else(|reason| {
                        panic!("appearance capture could not paint the Liquify cursor: {reason}")
                    });
                }
                if let Some(replay) = &self.capability_replay {
                    replay.paint_cursor_cue(ui.painter(), rect);
                }
                if let Some((
                    destination,
                    size,
                    allow_overwrite,
                    format,
                    jpeg_background,
                    transparent_png,
                )) = self.png_export.take_request()
                {
                    self.start_png_export(
                        destination,
                        size,
                        allow_overwrite,
                        format,
                        jpeg_background,
                        transparent_png,
                        ctx,
                    );
                }
                if let Some(request) = self.video_export.take_request() {
                    if self.file.as_ref().is_some_and(|source| {
                        crate::export::same_destination(source, &request.path)
                    }) {
                        self.status =
                            "Choose a GIF destination different from the native drawing.".into();
                    } else if let Err(error) = self.video_export.start(
                        request,
                        &self.doc,
                        self.camera,
                        &self.camera_bookmarks,
                        self.presentation.preferences.background,
                        self.renderer.lighting,
                        &self.construction_history,
                        video_export_ui::ExportResourceAvailability {
                            image_plane_ids: self.image_plane_assets.keys().copied().collect(),
                            model_reference_ids: self
                                .obj_reference_models
                                .keys()
                                .copied()
                                .collect(),
                        },
                    ) {
                        self.status = format!("Could not start animated export: {error}");
                    }
                }
                if let Some((destination, selected, replace)) = self.obj_export.take_request() {
                    if self
                        .file
                        .as_ref()
                        .is_some_and(|source| crate::export::same_destination(source, &destination))
                    {
                        self.status =
                            "Choose an OBJ destination different from the native drawing.".into();
                    } else if let Err(error) = self.obj_export.start(
                        destination,
                        selected,
                        replace,
                        &self.doc,
                        &self.selected,
                        ctx,
                    ) {
                        self.status = format!("Could not start OBJ export: {error}");
                    }
                }
                self.verify_png_export(ctx);
                if !self.presentation.artwork {
                    self.line_overlay(ui, rect);
                    self.curve_overlay(ui, rect);
                    self.mirror_overlay(ui, rect);
                    self.fill_overlay(ui, rect);
                    self.shape_overlay(ui, rect);
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
                        let heading_size = (rect.width() * 0.075).clamp(14.0, 25.0);
                        let instruction_size = (rect.width() * 0.05).clamp(10.0, 14.0);
                        ui.painter().text(
                            rect.center() + egui::vec2(0.0, -35.0),
                            egui::Align2::CENTER_CENTER,
                            "Your next idea starts here.",
                            egui::FontId::proportional(heading_size),
                            ui_style::contrasting_canvas_text(
                                self.presentation.preferences.background,
                            ),
                        );
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Pen to draw   /   Space + drag to explore",
                            egui::FontId::proportional(instruction_size),
                            ui_style::contrasting_canvas_text(
                                self.presentation.preferences.background,
                            ),
                        );
                    }
                }
                if response.hovered() && !self.presentation.artwork {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                }
            });
        if let Some(replay) = self.capability_replay.as_mut() {
            if replay.screenshot_pending && !replay.screenshot_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                replay.screenshot_requested = true;
            }
        }
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
                            self.library_open_path = None;
                            self.recovery_to_restore = None;
                            self.confirm = false;
                        }
                    });
                });
        }
        self.show_view(ctx);
        self.show_diagnostics_window(ctx);
        self.show_placement(ctx);
        self.show_lighting(ctx);
        self.show_drawing_library(ctx);
        self.show_insert_preview(ctx);
        self.show_tutorial(ctx);
        if self.show_help {
            egui::Window::new("Drawing with a tablet").default_width(540.0).max_height((ctx.screen_rect().height()-80.0).max(200.0)).vscroll(true).show(ctx,|ui|{self.tutorial_help(ui);ui.separator();if ui.button("Close help").clicked(){self.show_help=false;}ui.label("V: select   Shift-click: toggle stroke selection\nG: move   R: rotate   S: uniform scale (in Select tool)\nDrag the visible handles; Escape cancels a transform.\nCtrl/Cmd+A: select all   Ctrl/Cmd+D: duplicate\nDelete / Backspace: delete selection\n\nB: brush   E: whole-mark eraser   P: planar fill   I: sample artwork color (Escape cancels)\nK: pick plane   F: align view to guide   Shift+F: plane from view\nCtrl/Cmd+Z: undo   Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y: redo\nCtrl/Cmd+S: save   Ctrl/Cmd+Shift+S: save as\nCtrl/Cmd+O: open   Ctrl/Cmd+N: new\n\nSpace + pen drag: orbit\nSpace + Shift + pen drag: pan\nSpace + Alt + pen drag: zoom\nPen barrel + drag: orbit (when reported by Windows Ink)\nRight mouse drag: orbit   Middle drag: pan   Wheel: zoom\n\nUse Front / Side / Ground or rotate and position a custom plane.\nShift+F creates a plane through the camera focus; F looks straight at it.\nExisting strokes stay in world space when you switch planes.\nEnable Windows Ink in your Wacom / Huion driver for pressure.\nA mouse works too, with fixed pressure.\n\nExport menu: whole drawing or selected strokes as GLB.\nPrototype limits: no Wintab, additional curved guides. Recovery saves completed edits automatically; Save keeps your native drawing.\nSave your work using the Save button.");});
        }
        self.show_recovery(ctx);
        self.show_color_editor(ctx);
        self.tick_recovery(ctx);
        self.construction_history.record_camera(&self.camera);
        self.construction_history
            .record_tool_settings(&self.current_tool_settings());
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
            || self.mirror_verification_frame.is_some()
            || self.ring_verification_frame.is_some()
            || self.model_surface_verification_frame.is_some()
            || self.obj_reference_verification_frame.is_some()
            || self.vase_verification_frame.is_some()
            || self.group_verification_frame.is_some()
            || self.tutorial.verification_frame.is_some()
            || self.lighting_verification_frame.is_some()
            || self.fills.verification_frame.is_some()
            || self.placement.verification_frame.is_some()
            || self.presentation.verification_frame.is_some()
            || self.view_verification_frame.is_some()
            || self.capability_replay.is_some()
            || self.line.verification_frame.is_some()
            || self.close_after_recovery
            || self.recovery.busy()
        {
            let repaint_delay = if self.capability_replay.is_some() {
                Duration::from_millis(1000 / CAPABILITY_REPLAY_FPS as u64)
            } else {
                Duration::from_millis(8)
            };
            ctx.request_repaint_after(repaint_delay);
        }
    }
}

impl SketchApp {
    fn shape_shortcuts(&mut self, ctx: &egui::Context) {
        if self.shape_draft_active() && ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.cancel_shape_draft();
        }
    }

    /// Exercise image-plane rendering and brush targeting with deterministic mouse events.
    fn verify_image_planes(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.image_plane_verification_frame else {
            return;
        };
        match frame {
            0 => {
                let root = std::env::current_dir()
                    .unwrap()
                    .join("verification")
                    .join(format!(
                        "image-plane-{}",
                        crate::recovery_store::session_id()
                    ));
                std::fs::create_dir_all(&root).unwrap();
                let source = root.join("asymmetric-source.png");
                let image = image::RgbaImage::from_fn(32, 24, |x, y| {
                    let color = match (x >= 16, y >= 12) {
                        (false, false) => [235, 40, 35, 255],
                        (true, false) => [35, 210, 65, 255],
                        (false, true) => [35, 70, 235, 255],
                        (true, true) => [235, 210, 35, 255],
                    };
                    image::Rgba(color)
                });
                image.save(&source).unwrap();
                self.image_asset_root = root.join("drawing.sketch.assets");
                let (asset_file, asset) =
                    crate::image_planes::copy_asset(&source, &self.image_asset_root, 42).unwrap();
                let rotation =
                    glam::Quat::from_rotation_z(0.32) * glam::Quat::from_rotation_y(0.12);
                let origin = Vec3::new(0.35, -0.25, 0.8);
                let mut plane = crate::image_planes::ImagePlane::from_asset(
                    42,
                    "Asymmetric verification image",
                    asset_file,
                    asset.size,
                    origin,
                    rotation,
                    6.0,
                )
                .unwrap();
                plane.drawable = true;
                self.renderer
                    .set_image_plane_asset(plane.id, &asset)
                    .unwrap();
                self.image_plane_assets.clear();
                self.image_plane_assets.insert(plane.id, asset);
                self.doc = Document::default();
                self.doc.image_planes.push(plane);
                self.doc.active_image_plane = Some(42);
                self.next_id = 43;
                self.undo.clear();
                self.redo.clear();
                self.camera.align(crate::geometry::Plane::Front);
                self.camera.target = origin;
                self.camera.distance = 9.0;
                self.stabilization = 0.0;
                self.guide = DrawingPlane::default();
                self.renderer.background = [0.12, 0.12, 0.12];
                self.renderer.invalidate();
            }
            1 => {
                let plane = &self.doc.image_planes[0];
                let a = plane.origin + plane.rotation * Vec3::new(-1.0, -0.3, 0.0);
                let b = plane.origin + plane.rotation * Vec3::new(1.0, 0.3, 0.0);
                let start = crate::gizmo::project(&self.camera, rect, a).unwrap().0;
                let end = crate::gizmo::project(&self.camera, rect, b).unwrap().0;
                assert!(rect.contains(start) && rect.contains(end));
                self.add_point_with_tilt(start, 0.3, rect, true, None);
                self.add_point_with_tilt(end, 0.8, rect, false, None);
                self.finish();
            }
            2 => {
                assert_eq!(
                    self.doc.strokes.len(),
                    1,
                    "Image-plane brush contact commits one stroke"
                );
                let stroke = self.doc.strokes[0].clone();
                assert!(
                    stroke.points.len() >= 2,
                    "Pointer movement reaches the image-plane target"
                );
                let plane = &self.doc.image_planes[0];
                let inverse = plane.rotation.inverse();
                for point in &stroke.points {
                    let local = inverse * (point.position - plane.origin);
                    assert!(
                        local.z.abs() < 0.0001,
                        "Drawn point remains on transformed image plane"
                    );
                    assert!(local.x.abs() <= plane.size.x * 0.5 + 0.001);
                    assert!(local.y.abs() <= plane.size.y * 0.5 + 0.001);
                }
                assert!((stroke.brush.normal - plane.rotation * Vec3::Z).length() < 0.0001);
                assert!((stroke.brush.tangent - plane.rotation * Vec3::X).length() < 0.0001);
                assert_eq!(self.undo.len(), 1, "One contact is one undoable stroke");
                self.undo();
                assert!(
                    self.doc.strokes.is_empty(),
                    "Undo removes the image-plane stroke"
                );
                self.redo();
                assert_eq!(
                    self.doc.strokes,
                    vec![stroke],
                    "Redo restores exact stroke data"
                );

                let plane = &self.doc.image_planes[0];
                let outside_world = plane.origin + plane.rotation * Vec3::new(3.3, 0.0, 0.0);
                let outside = crate::gizmo::project(&self.camera, rect, outside_world)
                    .unwrap()
                    .0;
                assert!(
                    rect.contains(outside),
                    "Outside-bounds test remains within the canvas"
                );
                self.add_point(outside, 0.5, rect, true);
                self.finish();
            }
            3 => {
                assert_eq!(
                    self.doc.strokes.len(),
                    1,
                    "Out-of-bounds contact creates no image-plane stroke"
                );
                let plane = &mut self.doc.image_planes[0];
                plane.drawable = false;
                self.doc.active_image_plane = None;
                let world = plane.origin + plane.rotation * Vec3::new(-0.8, 0.0, 0.0);
                let pos = crate::gizmo::project(&self.camera, rect, world).unwrap().0;
                self.add_point(pos, 0.5, rect, true);
                self.finish();
            }
            4 => {
                assert_eq!(
                    self.doc.strokes.len(),
                    2,
                    "Inactive plane falls back to the regular drawing guide"
                );
                let plane = &self.doc.image_planes[0];
                let fallback = self.doc.strokes.last().unwrap();
                assert!(
                    fallback.points.iter().all(|point| {
                        (plane.rotation.inverse() * (point.position - plane.origin))
                            .z
                            .abs()
                            > 0.1
                    }),
                    "Inactive image plane does not intercept brush input"
                );

                self.verify_image_plane_planar_tools(ctx, rect);

                let with_plane = self
                    .renderer
                    .render_png_pixels(
                        &self.doc,
                        &self.camera,
                        [256, 256],
                        [0.12, 0.12, 0.12],
                        self.renderer.lighting,
                    )
                    .unwrap();
                let mut without_plane_doc = self.doc.clone();
                without_plane_doc.image_planes.clear();
                without_plane_doc.active_image_plane = None;
                let without_plane = self
                    .renderer
                    .render_png_pixels(
                        &without_plane_doc,
                        &self.camera,
                        [256, 256],
                        [0.12, 0.12, 0.12],
                        self.renderer.lighting,
                    )
                    .unwrap();
                let (with_plane_pixels, _) = with_plane.as_chunks::<4>();
                let (without_plane_pixels, _) = without_plane.as_chunks::<4>();
                let changed_pixels = with_plane_pixels
                    .iter()
                    .zip(without_plane_pixels.iter())
                    .filter(|(a, b)| {
                        a.iter()
                            .zip(b.iter())
                            .take(3)
                            .any(|(x, y)| x.abs_diff(*y) > 20)
                    })
                    .count();
                assert!(
                    changed_pixels > 500,
                    "Artwork PNG includes the image plane texture"
                );
                self.image_plane_verification_changed_pixels = changed_pixels;
                let output = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&output).unwrap();
                image::save_buffer(
                    output.join("image-plane-png-with.png"),
                    &with_plane,
                    256,
                    256,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                image::save_buffer(
                    output.join("image-plane-png-without.png"),
                    &without_plane,
                    256,
                    256,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            _ => {}
        }
        self.image_plane_verification_frame = Some(frame + 1);
    }

    fn verify_liquify_brush(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.liquify_brush_verification_frame else {
            return;
        };
        match frame {
            0 | 1 => {
                if frame == 0 {
                    // Keep unrelated recovery offers out of this isolated
                    // verification workflow without changing stored snapshots.
                    self.recovery_scan_seen = true;
                    self.recovery_prompt = false;
                    self.liquify_brush_verification_before = Some(self.doc.strokes.clone());
                    let selected = &self.doc.strokes[0];
                    let center = selected
                        .points
                        .iter()
                        .map(|point| point.position)
                        .sum::<Vec3>()
                        / selected.points.len() as f32;
                    self.camera.target = center;
                    self.camera.distance = 7.0;
                    self.begin_liquify();
                } else {
                    let before = self
                        .liquify_brush_verification_before
                        .as_ref()
                        .unwrap()
                        .clone();
                    assert_ne!(self.doc.strokes, before, "Canvas drag previews deformation");
                    assert!(self.undo.is_empty(), "Preview does not enter history");
                    self.finish_liquify(false);
                    assert_eq!(self.doc.strokes, before, "Cancel restores exact points");
                    self.begin_liquify();
                }
                self.activate_liquify_brush();
                let stroke = &self.liquify_brush_verification_before.as_ref().unwrap()[0];
                let center = stroke
                    .points
                    .iter()
                    .map(|point| point.position)
                    .sum::<Vec3>()
                    / stroke.points.len() as f32;
                let start_world = center - Vec3::X * 0.25;
                let end_world = start_world + Vec3::Y * 0.6;
                let start = crate::gizmo::project(&self.camera, rect, start_world)
                    .unwrap()
                    .0;
                let end = crate::gizmo::project(&self.camera, rect, end_world)
                    .unwrap()
                    .0;
                assert!(rect.contains(start) && rect.contains(end));
                if frame == 0 {
                    ctx.input_mut(|input| {
                        input.events = vec![
                            egui::Event::PointerButton {
                                pos: start,
                                button: PointerButton::Primary,
                                pressed: true,
                                modifiers: Default::default(),
                            },
                            egui::Event::PointerMoved(end),
                            egui::Event::PointerButton {
                                pos: end,
                                button: PointerButton::Primary,
                                pressed: false,
                                modifiers: Default::default(),
                            },
                        ];
                    });
                } else {
                    self.liquify_brush_input(start, 1.0, rect, crate::input::Phase::Down);
                    self.liquify_brush_input(end, 1.0, rect, crate::input::Phase::Move);
                    self.liquify_brush_input(end, 1.0, rect, crate::input::Phase::Up);
                }
            }
            2 => {
                let before = self
                    .liquify_brush_verification_before
                    .as_ref()
                    .unwrap()
                    .clone();
                let applied = self.doc.strokes.clone();
                assert_ne!(applied, before, "Second drag previews deformation");
                assert!(self.undo.is_empty());
                let original_selected = before
                    .iter()
                    .find(|stroke| self.selected.contains(&stroke.id))
                    .expect("Liquify source selection exists");
                let deformed_selected = applied
                    .iter()
                    .find(|stroke| self.selected.contains(&stroke.id))
                    .expect("Liquify preview selection exists");
                let (moved_point_index, maximum_displacement) = original_selected
                    .points
                    .iter()
                    .zip(&deformed_selected.points)
                    .enumerate()
                    .map(|(index, (original, deformed))| {
                        (index, original.position.distance(deformed.position))
                    })
                    .max_by(|left, right| left.1.total_cmp(&right.1))
                    .expect("Liquify source contains points");
                assert!(
                    maximum_displacement > 0.05,
                    "Canvas drag moves selected geometry"
                );
                self.finish_liquify(true);
                assert_eq!(self.undo.len(), 1, "Brush gesture is one undo step");
                assert_eq!(self.doc.strokes, applied, "Commit matches the preview");
                self.undo();
                assert_eq!(
                    self.doc.strokes, before,
                    "Undo restores exact source geometry"
                );
                self.redo();
                assert_eq!(
                    self.doc.strokes, applied,
                    "Redo restores committed deformation"
                );
                let moved_before = original_selected.points[moved_point_index].position;
                let moved_after = deformed_selected.points[moved_point_index].position;
                let new_screen = crate::gizmo::project(&self.camera, rect, moved_after)
                    .expect("deformed point projects")
                    .0;
                let old_screen = crate::gizmo::project(&self.camera, rect, moved_before)
                    .expect("original point projects")
                    .0;
                assert_eq!(
                    crate::gizmo::pick(&self.doc, &self.camera, rect, new_screen),
                    Some(deformed_selected.id),
                    "Updated geometry remains pickable at the deformed position"
                );
                assert_ne!(
                    crate::gizmo::pick(&self.doc, &self.camera, rect, old_screen),
                    Some(deformed_selected.id),
                    "Picking no longer follows the original geometry"
                );

                let native = Document::from_json(&serde_json::to_string(&self.doc).unwrap())
                    .expect("Liquify output survives native serialization");
                assert_eq!(
                    native.strokes, applied,
                    "Native reload preserves Liquify geometry"
                );
                let recovery_dir = tempfile::tempdir().unwrap();
                let recovery = crate::recovery_store::RecoverySnapshot::new(
                    "liquify-verification-session".into(),
                    None,
                    native,
                );
                crate::recovery_store::write(recovery_dir.path(), &recovery).unwrap();
                let recovered =
                    crate::recovery_store::scan(recovery_dir.path(), "liquify-verification-reader")
                        .unwrap();
                assert_eq!(
                    recovered.candidates[0].snapshot.document.strokes, applied,
                    "Recovery preserves Liquify geometry"
                );
                let whole_scene = crate::export::encode(&self.doc, None).unwrap();
                let selected = crate::export::encode(&self.doc, Some(&self.selected)).unwrap();
                assert_eq!(whole_scene.summary.strokes, applied.len());
                assert_eq!(selected.summary.strokes, 1);
                assert!(selected.summary.triangles > 0);

                let camera_before = self.camera;
                for view in [
                    crate::geometry::Plane::Front,
                    crate::geometry::Plane::Side,
                    crate::geometry::Plane::Ground,
                ] {
                    let baseline = self.doc.strokes.clone();
                    let selected_stroke = baseline
                        .iter()
                        .find(|stroke| self.selected.contains(&stroke.id))
                        .unwrap();
                    let center = selected_stroke
                        .points
                        .iter()
                        .map(|point| point.position)
                        .sum::<Vec3>()
                        / selected_stroke.points.len() as f32;
                    self.camera.align(view);
                    self.camera.target = center;
                    self.camera.distance = 7.0;
                    self.begin_liquify();
                    self.activate_liquify_brush();
                    let (right, _, normal) = self.camera.basis();
                    let start = crate::gizmo::project(&self.camera, rect, center - right * 0.25)
                        .unwrap()
                        .0;
                    let end = crate::gizmo::project(&self.camera, rect, center + right * 0.35)
                        .unwrap()
                        .0;
                    self.liquify_brush_input(start, 1.0, rect, Phase::Down);
                    self.liquify_brush_input(end, 1.0, rect, Phase::Move);
                    self.liquify_brush_input(end, 1.0, rect, Phase::Up);
                    let deformed = &self.doc.strokes;
                    assert_ne!(
                        deformed, &baseline,
                        "Drag previews from each tested camera view"
                    );
                    for (before_stroke, after_stroke) in baseline.iter().zip(deformed) {
                        if self.selected.contains(&before_stroke.id) {
                            for (before_point, after_point) in
                                before_stroke.points.iter().zip(&after_stroke.points)
                            {
                                assert!(
                                    (after_point.position - before_point.position)
                                        .dot(normal)
                                        .abs()
                                        < 1e-4,
                                    "Camera-facing working plane preserves selected-ink depth"
                                );
                            }
                        } else {
                            assert_eq!(
                                before_stroke, after_stroke,
                                "Occluded/unselected ink stays fixed"
                            );
                        }
                    }
                    self.finish_liquify(false);
                    assert_eq!(
                        self.doc.strokes, baseline,
                        "Cancel restores camera-view preview"
                    );
                }
                let navigation_baseline = self.doc.strokes.clone();
                let selected_stroke = self
                    .doc
                    .strokes
                    .iter()
                    .find(|stroke| self.selected.contains(&stroke.id))
                    .unwrap();
                let center = selected_stroke
                    .points
                    .iter()
                    .map(|point| point.position)
                    .sum::<Vec3>()
                    / selected_stroke.points.len() as f32;
                self.begin_liquify();
                self.activate_liquify_brush();
                let (right, _, _) = self.camera.basis();
                let start = crate::gizmo::project(&self.camera, rect, center - right * 0.25)
                    .unwrap()
                    .0;
                let end = crate::gizmo::project(&self.camera, rect, center + right * 0.35)
                    .unwrap()
                    .0;
                self.liquify_brush_input(start, 1.0, rect, Phase::Down);
                self.liquify_brush_input(end, 1.0, rect, Phase::Move);
                assert_ne!(self.doc.strokes, navigation_baseline);
                let navigation_history_len = self.undo.len();
                self.mouse_down = true;
                self.cancel_liquify_for_navigation();
                self.mouse_down = false;
                self.navigate(Vec2::new(12.0, -8.0), false, false, rect);
                assert_eq!(self.doc.strokes, navigation_baseline);
                assert_eq!(self.undo.len(), navigation_history_len);
                self.camera = camera_before;
                self.dirty = false;
                self.renderer.invalidate();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            _ => {}
        }
        self.liquify_brush_verification_frame = Some(frame + 1);
    }

    fn verify_mixed_resources(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.mixed_resources_verification_frame else {
            return;
        };
        if frame == 0 {
            self.undo.clear();
            self.redo.clear();
            assert!(Self::mixed_transform_supported(
                crate::mixed_resources::TransformMode::Translate
            ));
            assert!(!Self::mixed_transform_supported(
                crate::mixed_resources::TransformMode::Rotate
            ));
            assert!(!Self::mixed_transform_supported(
                crate::mixed_resources::TransformMode::Scale
            ));
            let mut selection = self.mixed_resources_ui.selection.clone();
            selection.stroke_ids = self.selected.clone();
            let original = crate::mixed_resources::MoveSnapshot::capture(
                &self.doc,
                selection.clone(),
                crate::mixed_resources::TransformMode::Translate,
                Vec3::ZERO,
            )
            .unwrap()
            .before;
            let active_plane = self.doc.active_image_plane;

            self.begin_mixed_resource_move();
            assert!(self.mixed_resource_preview_active());
            self.set_mixed_resource_translation(Vec3::new(0.3, -0.2, 0.15));
            let preview = crate::mixed_resources::MoveSnapshot::capture(
                &self.doc,
                selection.clone(),
                crate::mixed_resources::TransformMode::Translate,
                Vec3::ZERO,
            )
            .unwrap()
            .before;
            assert_ne!(preview, original, "All selected resources preview together");
            assert!(self.undo.is_empty(), "Preview does not enter history");
            self.finish_mixed_resource_move(false);
            let cancelled = crate::mixed_resources::MoveSnapshot::capture(
                &self.doc,
                selection.clone(),
                crate::mixed_resources::TransformMode::Translate,
                Vec3::ZERO,
            )
            .unwrap()
            .before;
            assert_eq!(cancelled, original, "Cancel restores all source records");

            self.begin_mixed_resource_move();
            self.set_mixed_resource_translation(Vec3::new(0.3, -0.2, 0.15));
            let applied = crate::mixed_resources::MoveSnapshot::capture(
                &self.doc,
                selection.clone(),
                crate::mixed_resources::TransformMode::Translate,
                Vec3::ZERO,
            )
            .unwrap()
            .before;
            self.finish_mixed_resource_move(true);
            assert_eq!(self.undo.len(), 1, "Mixed movement is one history edit");
            assert_eq!(self.doc.active_image_plane, active_plane);
            self.undo();
            let undone = crate::mixed_resources::MoveSnapshot::capture(
                &self.doc,
                selection.clone(),
                crate::mixed_resources::TransformMode::Translate,
                Vec3::ZERO,
            )
            .unwrap()
            .before;
            assert_eq!(undone, original, "Undo restores every selected resource");
            self.redo();
            let redone = crate::mixed_resources::MoveSnapshot::capture(
                &self.doc,
                selection,
                crate::mixed_resources::TransformMode::Translate,
                Vec3::ZERO,
            )
            .unwrap()
            .before;
            assert_eq!(redone, applied, "Redo reapplies every selected resource");
            self.dirty = false;
            self.renderer.invalidate();
            self.refresh_obj_reference_meshes();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        self.mixed_resources_verification_frame = Some(frame + 1);
    }

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
            && self.image_plane_verification_frame.is_none()
            && self.mirror_verification_frame.is_none()
            && self.ring_verification_frame.is_none()
            && self.model_surface_verification_frame.is_none()
            && self.obj_reference_verification_frame.is_none()
            && self.liquify_brush_verification_frame.is_none()
            && self.mixed_resources_verification_frame.is_none()
            && self.vase_verification_frame.is_none()
            && self.group_verification_frame.is_none()
            && self.appearance_layout_verification.is_none()
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
            if let Some(verification) = self.appearance_layout_verification.take() {
                let theme = match verification.theme {
                    crate::presentation::UiTheme::Dark => "dark",
                    crate::presentation::UiTheme::Light => "light",
                    crate::presentation::UiTheme::System => unreachable!(),
                };
                let output = path.join("appearance-layout");
                std::fs::create_dir_all(&output).expect("appearance screenshot folder");
                let filename = if verification.viewport_size == [1440, 940] {
                    format!(
                        "{theme}-{}-{}-percent.png",
                        verification.state, verification.zoom_percent
                    )
                } else {
                    format!(
                        "{theme}-{}-viewport-{}x{}-{}-percent.png",
                        verification.state,
                        verification.viewport_size[0],
                        verification.viewport_size[1],
                        verification.zoom_percent
                    )
                };
                let screenshot_path = output.join(&filename);
                image::save_buffer(
                    &screenshot_path,
                    &pixels,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .expect("appearance screenshot saves");
                let mut checks = vec![
                    format!(
                        "hidden app rendered the {} UI state at the requested egui zoom factor",
                        verification.state
                    ),
                    "the requested appearance theme was applied in memory without saving user preferences".into(),
                ];
                if matches!(
                    verification.state.as_str(),
                    "scene-mirror-preview-dark"
                        | "scene-mirror-preview-light"
                        | "scene-mirror-preview-custom"
                        | "scene-mirror-preview-image"
                ) {
                    checks.push(
                        "production live line draft and mirror preview overlays were active for this capture".into(),
                    );
                }
                if verification.state == "scene-mixed-resource-disabled" {
                    assert!(self.mixed_resource_preview_active());
                    assert!(!self.mixed_resource_has_preview());
                    assert!(self.undo.is_empty());
                    assert!(!Self::mixed_transform_supported(
                        crate::mixed_resources::TransformMode::Rotate
                    ));
                    assert!(!Self::mixed_transform_supported(
                        crate::mixed_resources::TransformMode::Scale
                    ));
                    checks.push(
                        "mixed-resource controls show disabled Rotate and Scale plus disabled Apply before movement".into(),
                    );
                }
                if matches!(
                    verification.state.as_str(),
                    "scene-brush-selection-drag-light" | "scene-brush-selection-drag-image"
                ) {
                    let drag = self
                        .brush_select_drag
                        .as_ref()
                        .expect("brush-selection drag remains active through screenshot capture");
                    assert!(drag.path_len() >= 2);
                    assert!(
                        self.selected
                            .iter()
                            .any(|id| { self.doc.strokes.iter().any(|stroke| stroke.id == *id) }),
                        "brush-selection capture requires selected fixture geometry"
                    );
                    checks.push(
                        "production brush-selection input crossed the fixture stroke and kept its live drag path visible".into(),
                    );
                }
                if matches!(
                    verification.state.as_str(),
                    "scene-liquify-cursor-light" | "scene-liquify-cursor-image"
                ) {
                    checks.push(
                        "painted Liquify cursor received a deterministic synthetic in-canvas pointer position".into(),
                    );
                }
                let report = serde_json::json!({
                    "passed": true,
                    "theme": theme,
                    "state": verification.state,
                    "egui_zoom_percent": verification.zoom_percent,
                    "requested_viewport_points": verification.viewport_size,
                    "screenshot": filename,
                    "screenshot_size_pixels": image.size,
                    "checks": checks,
                    "physical_tablet_tested": false
                });
                std::fs::write(
                    output.join(filename.replace(".png", ".json")),
                    serde_json::to_vec_pretty(&report).unwrap(),
                )
                .expect("appearance report writes");
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
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
                } else if self.image_plane_verification_frame.is_some() {
                    "image-plane-preview.png"
                } else if self.mirror_verification_frame.is_some() {
                    "mirror-preview.png"
                } else if self.ring_verification_frame.is_some() {
                    "ring-preview.png"
                } else if self.model_surface_verification_frame.is_some() {
                    "model-surface-preview.png"
                } else if self.obj_reference_verification_frame.is_some() {
                    "obj-reference-preview.png"
                } else if self.liquify_brush_verification_frame.is_some() {
                    "liquify-brush-preview.png"
                } else if self.mixed_resources_verification_frame.is_some() {
                    "mixed-resources-preview.png"
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
            let report = serde_json::json!({"passed":true,"checks":["single-frame mouse sample order","3D geometry and GPU pipeline","undo/redo","whole-stroke eraser","atomic save","document reload","UI screenshot","Shift multi-selection","mouse and synthetic pen brush select/deselect through canvas handlers at near perspective and far orthographic zoom without geometry or history edits","partial eraser mouse/pen preview, Escape cancel, and single-undo release through canvas input","axis move and view-plane cancel","rotation and uniform scale handles","rotated-selection non-uniform X/Y/Z handle and numeric scaling agree for selection-center, world-origin, and custom pivots with one-step undo/redo","Escape cancels non-uniform axis scaling and restores exact document/history","view-plane scaling applies separate camera-plane X/Y factors and preserves depth","transformed selection bounds refresh after view-plane scaling","scaled stroke remains pickable at transformed geometry","numeric non-uniform scale survives native reload and produces byte-identical GLB output","non-uniform scale survives recovery write, reload, and recovery scan","group duplicate/delete undo","numeric transform","transformed document reload","stabilized stroke endpoint and undo","custom plane drawing and grid","plane switching preserves geometry","plane alignment and view placement","oriented stroke save/reload","background GLB export preserves document state"],"gpu":self.renderer.adapter,"strokes":self.doc.strokes.len(),"physical_tablet_tested":false});
            let report = if self.group_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["house component grouping","group and individual selection","duplicate independent group","shared pivot transforms","ungroup undo and native persistence","layer protection","GPU groups"],"gpu":self.renderer.adapter})
            } else if self.image_plane_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["generated asymmetric image texture uploaded to GPU","transformed image plane brush input handler commits a stroke","stroke points remain within transformed plane bounds","captured brush frame follows plane rotation","one-step stroke undo and exact redo","out-of-bounds image-plane contact rejected","inactive image plane does not intercept brush input","transformed finite image-plane line drag and undo/redo","image-plane fill click-to-close, clipping, and undo/redo","rotated image-plane circle drag, clipping, and undo/redo","rotated image-plane ellipse drag, camera orbit, clipping, and undo/redo","image-plane curve anchor clicks, insertion, tangent drag, Backspace removal, Enter commit, clipping, and undo/redo","line, fill, circle, and curve Escape cancel without document edits","shape Escape cancels after camera orbit without document edits","hidden and locked image planes reject all planar tools","artwork PNG includes image-plane pixels"],"image_plane_count":self.doc.image_planes.len(),"stroke_count":self.doc.strokes.len(),"png_changed_pixels_vs_no_plane":self.image_plane_verification_changed_pixels,"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.mirror_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic pointer-handler samples draw on an active saved cylinder guide","curved-guide brush points remain on the guide surface","mirror preview geometry does not mutate the document","committed mirrored geometry matches its preview exactly","source and mirrored run commit as one undo edit","undo/redo preserve the exact mirrored pair","cancel discards an in-progress source without history","whole-drawing GLB includes the mirrored pair","mouse canvas events preview, cancel, and commit a straight line on a rotated active guide","active guide horizontal constraint and mirrored points remain on the rotated plane","line source and mirror commit as one undo edit and survive undo/redo"],"stroke_count":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.ring_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["Add ring action creates and activates a saved torus at the camera target","active ring camera alignment faces the torus hole","synthetic pointer samples draw on the saved ring surface","a center-hole miss splits the drawing into two surface strokes","ring strokes remain on the torus instead of cutting through its hole","cancel removes an in-progress ring stroke without history","undo and redo preserve split ring strokes","ring strokes export to GLB"],"guide_count":self.doc.guide_resources.len(),"stroke_count":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.model_surface_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic pointer samples cross a shared manifold triangle edge in one model-surface stroke","world-space samples remain on the OBJ surface","model-surface commit creates one ordinary undo edit","undo and redo preserve exact model-surface samples","a pointer miss commits the first surface run","recontact after a miss starts a second surface run without bridging the gap","split model-surface runs undo and redo independently"],"stroke_count":self.doc.strokes.len(),"point_count":self.doc.strokes.iter().map(|stroke|stroke.points.len()).sum::<usize>(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.obj_reference_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["OBJ References window displays its import control","synthetic pointer move, press, and release clicks the window's import control","canceling the picker leaves the document, history, and dirty state unchanged and keeps the source fixture available","the clicked control invokes the production import handler with an injected file-picker result","the imported drawing-local bundle reloads","imported model is registered with the scene renderer","synthetic viewport selection picks the imported model","selected-model bounds match the normalized OBJ bounds"],"native_file_picker_tested":false,"model_count":self.doc.model_references.len(),"triangle_count":self.obj_reference_models.values().map(|model|model.triangles.len()).sum::<usize>(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.liquify_brush_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic canvas drag creates a non-destructive preview","front, side, and ground camera views preserve the working-plane depth","unselected and occluding strokes remain unchanged","cancel restores exact source strokes","navigation interruption cancels brush preview without history","apply commits preview as one history edit","undo restores source geometry","redo restores committed deformation","updated stroke remains pickable at the deformed position and is no longer picked at its original position","native serialization and recovery scan preserve the deformed geometry","whole-scene and selected-stroke GLB encode the committed result","brush cursor indicates radius and working-plane depth"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.mixed_resources_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["mixed stroke, image plane, OBJ reference, and guide translation preview","preview does not enter history","cancel restores all exact source records","apply commits one mixed-resource history edit","undo restores every selected record","redo reapplies every selected record","active image-plane drawable binding is preserved","mixed-resource UI enables translation and disables rotation and scale","rotation and scaling are rejected before mutation"],"resource_counts":{"strokes":self.doc.strokes.len(),"image_planes":self.doc.image_planes.len(),"obj_references":self.doc.model_references.len(),"guides":self.doc.guide_resources.len()},"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.vase_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["revolved profile drawing","captured guide and surface release tail","pressure tilt brush frames","guide editing preserves ink","selection duplicate undo","native persistence","GPU painted vase"],"gpu":self.renderer.adapter})
            } else if self.layer_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["layer drawing and locking","visibility picking and erasing","layer reassignment undo redo","nonempty deletion protection","version 5 persistence","GPU layers"],"gpu":self.renderer.adapter})
            } else if self.recovery_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["forced process termination","startup recovery offer","inserted native stroke and remapped layer survive recovery after source deletion","exact fill contour offset pressure tilt dynamics texture restoration","image-plane recovery asset reinstalls the renderer texture","recovered copy is untitled and dirty","native source unchanged","new session snapshot durable before old retirement","Save New and close cleanup ordering","corrupt and unsupported issue visibility","GPU recovery workflow"],"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.preset_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["preset application captures resolved settings","create duplicate rename update delete and defaults","restart persistence","live GPU previews","library edits preserve existing ink and export","corrupt library fallback and explicit repair backup"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.texture_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["masked GPU rendering","selection retains coverage","texture transforms preserve UVs","stroke duplication and undo","textured drawing and erasing","version 13 save/reload","original procedural mask sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.dynamics_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic Windows Ink contact and tilt","independent pressure settings","pressure-to-opacity capture","soft-edge geometry coverage","overlap compositing independent of document order","camera-orbit stability","opaque-surface occlusion","stabilized release tail","cancellation discards dynamics","missing tilt fallback","captured settings","version 13 save/reload","GPU dynamics sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.brush_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["all brush types through drawing input","captured guide orientation","camera orbit preserves mesh","shape-aware picking and erasing","duplication","scale and rotation preserve brush shape","cylinder frames and stabilized release tails","undo/redo","version 13 save/reload","GPU brush sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
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
                } else if self.image_plane_verification_frame.is_some() {
                    "image-plane-report.json"
                } else if self.mirror_verification_frame.is_some() {
                    "mirror-report.json"
                } else if self.ring_verification_frame.is_some() {
                    "ring-report.json"
                } else if self.model_surface_verification_frame.is_some() {
                    "model-surface-report.json"
                } else if self.obj_reference_verification_frame.is_some() {
                    "obj-reference-report.json"
                } else if self.vase_verification_frame.is_some() {
                    "vase-report.json"
                } else if self.layer_verification_frame.is_some() {
                    "layer-report.json"
                } else if self.image_plane_verification_frame.is_some() {
                    "image-plane-report.json"
                } else if self.liquify_brush_verification_frame.is_some() {
                    "liquify-brush-report.json"
                } else if self.mixed_resources_verification_frame.is_some() {
                    "mixed-resources-report.json"
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
    fn capture_capability_replay_screenshot(&mut self, ctx: &egui::Context) {
        let screenshot_pending = self
            .capability_replay
            .as_ref()
            .is_some_and(|replay| replay.screenshot_pending && replay.screenshot_requested);
        if !screenshot_pending {
            return;
        }
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = screenshot else {
            if let Some(replay) = self.capability_replay.as_mut() {
                replay.wait_updates = replay.wait_updates.saturating_add(1);
                if replay.wait_updates > 120 {
                    eprintln!("Capability demo capture timed out waiting for a screenshot event");
                    std::process::exit(1);
                }
            }
            return;
        };
        let dimensions = [image.size[0] as u32, image.size[1] as u32];
        let pixel_count = (dimensions[0] as usize).saturating_mul(dimensions[1] as usize);
        if dimensions[0] == 0
            || dimensions[1] == 0
            || dimensions[0] > 3840
            || dimensions[1] > 2160
            || pixel_count > 3840usize * 2160usize
        {
            eprintln!("Capability demo screenshot dimensions exceed the capture limit");
            std::process::exit(1);
        }
        let replay = self.capability_replay.as_mut().unwrap();
        if replay.frame_size.is_some_and(|size| size != dimensions) {
            eprintln!("Capability demo screenshots changed size during capture");
            std::process::exit(1);
        }
        if replay.sender.is_none() {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            let gif_output = replay.path("gif");
            replay.encoder = Some(std::thread::spawn(move || {
                let cancelled = AtomicBool::new(false);
                video_export_ui::encode_gif_once(
                    receiver,
                    &gif_output,
                    [640, 360],
                    CAPABILITY_REPLAY_FPS,
                    CAPABILITY_REPLAY_FRAMES,
                    &cancelled,
                )
            }));
            replay.sender = Some(sender);
            let (mp4_sender, mp4_receiver) = std::sync::mpsc::sync_channel(1);
            let mp4_output = replay.path("mp4");
            let mp4_size = dimensions;
            replay.mp4_encoder = Some(std::thread::spawn(move || {
                let cancelled = AtomicBool::new(false);
                video_export_ui::encode_mp4(
                    mp4_receiver,
                    &mp4_output,
                    mp4_size,
                    CAPABILITY_REPLAY_FPS,
                    CAPABILITY_REPLAY_FRAMES,
                    &cancelled,
                )
            }));
            replay.mp4_sender = Some(mp4_sender);
            replay.frame_size = Some(dimensions);
        }
        replay.pixels_per_point = ctx.pixels_per_point();
        let pixels = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_array())
            .collect::<Vec<_>>();
        let Some(rgba) = image::RgbaImage::from_raw(dimensions[0], dimensions[1], pixels) else {
            eprintln!("Could not assemble capability demo screenshot pixels");
            std::process::exit(1);
        };
        if replay.captured_frames == 0 {
            if let Err(error) = replay.save_poster(rgba.as_raw(), dimensions) {
                self.capability_replay.take().unwrap().abort();
                eprintln!("Could not save capability demo poster: {error}");
                std::process::exit(1);
            }
        }
        let resized =
            image::imageops::resize(&rgba, 640, 360, image::imageops::FilterType::Lanczos3);
        let sender = replay.sender.as_ref().unwrap();
        if sender.send(Some(resized.into_raw())).is_err() {
            eprintln!("Capability demo GIF encoder stopped before all frames were captured");
            std::process::exit(1);
        }
        let mp4_sender = replay.mp4_sender.as_ref().unwrap();
        if mp4_sender.send(Some(rgba.into_raw())).is_err() {
            eprintln!("Capability demo MP4 encoder stopped before all frames were captured");
            std::process::exit(1);
        }
        replay.captured_frames += 1;
        if replay.captured_frames.is_multiple_of(10) {
            eprintln!(
                "Capability demo captured frame {}/{}",
                replay.captured_frames, CAPABILITY_REPLAY_FRAMES
            );
        }
        replay.screenshot_pending = false;
        replay.screenshot_requested = false;
        if replay.captured_frames == 27 {
            let committed = if replay.clip.is_curved_guides() {
                replay.setup_verified
                    && self.doc.strokes.len() == replay.initial_stroke_count
                    && self.doc.active_guide.is_some()
            } else if replay.clip.is_mirror_shapes() {
                replay.setup_verified
                    && self.doc.strokes.len() == replay.initial_stroke_count
                    && self.mirror_ui.enabled
            } else if replay.clip.is_edit_erase() {
                let split_strokes = self.doc.strokes.as_slice();
                replay.setup_verified
                    && replay.eraser
                    && split_strokes.len() == replay.initial_stroke_count + 1
                    && split_strokes.first().is_some_and(|left| {
                        split_strokes.last().is_some_and(|right| {
                            left.points.len() > 2
                                && right.points.len() > 2
                                && left.points.last().unwrap().position.x
                                    < right.points.first().unwrap().position.x
                        })
                    })
                    && self.undo.len() == 2
            } else {
                self.doc.strokes.len() > replay.initial_stroke_count
                    && self.doc.strokes.last().is_some_and(|stroke| {
                        stroke.points.len() > 10
                            && replay.canvas_rect.is_some_and(|rect| {
                                stroke.points.iter().any(|point| {
                                    crate::gizmo::project(&self.camera, rect, point.position)
                                        .is_some_and(|(position, _)| rect.contains(position))
                                })
                            })
                    })
            };
            if !committed {
                self.capability_replay.take().unwrap().abort();
                eprintln!("Capability demo replay did not retain its verified drawing workflow");
                std::process::exit(1);
            }
        }
        if replay.clip.is_edit_erase()
            && replay.captured_frames == 35
            && (!replay.undo_applied
                || replay.expected_undo_strokes.as_ref() != Some(&self.doc.strokes)
                || self.undo.len() != 1)
        {
            self.capability_replay.take().unwrap().abort();
            eprintln!("Capability demo undo did not restore the edited source stroke");
            std::process::exit(1);
        }
        if replay.captured_frames == 51 {
            let camera_changed = (self.camera.yaw - replay.initial_camera.yaw).abs() > 0.001
                || (self.camera.pitch - replay.initial_camera.pitch).abs() > 0.001
                || (self.camera.roll - replay.initial_camera.roll).abs() > 0.001
                || (self.camera.distance - replay.initial_camera.distance).abs() > 0.001
                || self.camera.target.distance(replay.initial_camera.target) > 0.001;
            if !camera_changed {
                self.capability_replay.take().unwrap().abort();
                eprintln!("Capability demo replay did not orbit the camera through canvas_input");
                std::process::exit(1);
            }
        }
        if replay.captured_frames == CAPABILITY_REPLAY_FRAMES {
            let replay = self.capability_replay.take().unwrap();
            let clip_name = replay.clip.name();
            if let Err(error) = replay.finish() {
                eprintln!("Capability demo capture failed: {error}");
                std::process::exit(1);
            }
            eprintln!(
                "Wrote verification/capability-demos/drafts/{}.gif and manifest",
                clip_name
            );
            self.allow_close = true;
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

#[cfg(test)]
mod native_merge_apply_tests {
    use super::*;
    use crate::{
        document::{Layer, Point},
        editing::MergeEdit,
    };

    fn stroke(id: u64, layer_id: u64, x: f32) -> Stroke {
        Stroke {
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id,
            id,
            color: [0.2, 0.6, 0.9],
            radius: 0.04,
            brush: Default::default(),
            points: vec![
                Point {
                    position: Vec3::new(x, 0.0, 0.0),
                    pressure: 0.8,
                    tilt: None,
                },
                Point {
                    position: Vec3::new(x + 0.5, 0.0, 0.0),
                    pressure: 0.8,
                    tilt: None,
                },
            ],
        }
    }

    #[test]
    fn insert_preview_renders_placement_and_routes_cancel_and_apply_clicks() {
        fn frame(
            context: &egui::Context,
            path: &std::path::Path,
            source: &Document,
            offset: &mut [f32; 3],
            insert_as_group: &mut bool,
            events: Vec<egui::Event>,
        ) -> Option<InsertPreviewUiResponse> {
            let mut response = None;
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    response =
                        show_insert_preview_window(ctx, path, source, offset, insert_as_group);
                },
            );
            response
        }

        fn click(
            context: &egui::Context,
            path: &std::path::Path,
            source: &Document,
            offset: &mut [f32; 3],
            insert_as_group: &mut bool,
            position: Pos2,
        ) -> Option<InsertPreviewUiResponse> {
            let _ = frame(
                context,
                path,
                source,
                offset,
                insert_as_group,
                vec![egui::Event::PointerMoved(position)],
            );
            let _ = frame(
                context,
                path,
                source,
                offset,
                insert_as_group,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            frame(
                context,
                path,
                source,
                offset,
                insert_as_group,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            )
        }

        let source = Document {
            strokes: vec![stroke(1, 1, -0.5)],
            ..Document::default()
        };
        let path = std::path::Path::new("insert-source.sketch.json");

        let context = egui::Context::default();
        let mut offset = [0.0; 3];
        let mut insert_as_group = false;
        let first = frame(
            &context,
            path,
            &source,
            &mut offset,
            &mut insert_as_group,
            Vec::new(),
        )
        .expect("insert preview should render");
        assert!(first.preview_rect.width() >= 300.0);
        assert!(first.insert_button_rect.is_positive());
        assert!(first.cancel_button_rect.is_positive());
        assert_eq!(offset, [0.0; 3]);
        let canceled = click(
            &context,
            path,
            &source,
            &mut offset,
            &mut insert_as_group,
            first.cancel_button_rect.center(),
        )
        .expect("insert preview should remain visible until its action is handled");
        assert_eq!(canceled.action, Some(InsertPreviewAction::Cancel));

        let context = egui::Context::default();
        let mut offset = [0.0; 3];
        let mut insert_as_group = false;
        let first = frame(
            &context,
            path,
            &source,
            &mut offset,
            &mut insert_as_group,
            Vec::new(),
        )
        .expect("insert preview should render");
        let applied = click(
            &context,
            path,
            &source,
            &mut offset,
            &mut insert_as_group,
            first.insert_button_rect.center(),
        )
        .expect("insert preview should remain visible until its action is handled");
        assert_eq!(applied.action, Some(InsertPreviewAction::Apply));
    }

    #[test]
    fn applying_insert_is_one_edit_and_preserves_camera_and_presentation_state() {
        let mut doc = Document {
            strokes: vec![stroke(1, 1, -1.0)],
            ..Document::default()
        };
        let mut selected = HashSet::from([1]);
        let mut undo = Vec::new();
        let mut redo = vec![Edit::Add(stroke(99, 1, 2.0))];
        let mut next_id = 2;
        let mut dirty = false;
        let mut image_assets = HashMap::new();
        let mut obj_models = HashMap::new();
        let camera = Camera::default();
        let camera_before = format!("{camera:?}");
        let preferences = crate::presentation::Preferences {
            background: [0.81, 0.63, 0.42],
            ..Default::default()
        };
        let preferences_before = preferences.clone();
        let guide = DrawingPlane::default();
        let guide_before = guide;

        let inserted = native_merge::InsertedDrawing {
            edit: MergeEdit {
                strokes: vec![stroke(2, 2, 0.0)],
                layers: vec![Layer {
                    id: 2,
                    name: "Layer 1 (import 2)".into(),
                    visible: true,
                    locked: false,
                }],
                image_planes: Vec::new(),
                guides: Vec::new(),
                model_references: Vec::new(),
                previous_active_layer: 1,
                inserted_active_layer: 2,
            },
            next_id: 3,
            image_assets: HashMap::new(),
            obj_models: HashMap::new(),
        };

        let inserted_count = apply_inserted_drawing_state(
            InsertedDrawingTarget {
                doc: &mut doc,
                selected: &mut selected,
                undo: &mut undo,
                redo: &mut redo,
                next_id: &mut next_id,
                dirty: &mut dirty,
                image_assets: &mut image_assets,
                obj_models: &mut obj_models,
            },
            inserted,
        );

        assert_eq!(inserted_count, 1);
        assert_eq!(
            doc.strokes
                .iter()
                .map(|stroke| stroke.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(doc.layers[1].name, "Layer 1 (import 2)");
        assert_eq!(doc.active_layer, 2);
        assert_eq!(selected, HashSet::from([2]));
        assert_eq!(undo.len(), 1);
        assert!(redo.is_empty());
        assert_eq!(next_id, 3);
        assert!(dirty);
        assert_eq!(format!("{camera:?}"), camera_before);
        assert_eq!(preferences, preferences_before);
        assert_eq!(guide, guide_before);
    }
}
