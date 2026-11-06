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
use glam::{Quat, Vec3};
use std::collections::{hash_map::Entry, HashMap, HashSet};

type LoadedDrawing = (
    Document,
    HashMap<u64, crate::image_planes::ImageAsset>,
    HashMap<u64, crate::obj_reference::ObjModel>,
);

fn tool_icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    name: &str,
    detail: &str,
    selected: bool,
) -> egui::Response {
    let response = ui.add_sized([44.0, 44.0], egui::Button::new("").selected(selected));
    let accessible_name = if icon == "shape" {
        "Shape tool, choose Circle, Ellipse, Rectangle, or Rounded rectangle".to_owned()
    } else if detail.starts_with("Shortcut:") {
        format!("{name}, {}", detail.trim_start_matches("Shortcut: ").trim())
    } else {
        name.to_owned()
    };
    let response = ui_style::accessible_button(response, &accessible_name, Some(selected));
    let response = response.on_hover_text(format!("{name} — {detail}"));
    let painter = ui.painter_at(response.rect);
    let color = if selected {
        ui_style::colors(ui.ctx()).focus
    } else {
        ui_style::colors(ui.ctx()).text
    };
    let stroke = egui::Stroke::new(1.8_f32, color);
    let center = response.rect.center();
    let point = |x: f32, y: f32| center + egui::vec2(x, y);
    let line = |a, b| painter.line_segment([a, b], stroke);
    match icon {
        "brush" => {
            line(point(-9.0, 8.0), point(7.0, -8.0));
            line(point(7.0, -8.0), point(10.0, -4.0));
            line(point(10.0, -4.0), point(-6.0, 12.0));
            line(point(-6.0, 12.0), point(-11.0, 13.0));
            line(point(-11.0, 13.0), point(-9.0, 8.0));
            line(point(-2.0, 2.0), point(3.0, 7.0));
        }
        "line" => {
            line(point(-11.0, 9.0), point(11.0, -9.0));
            painter.circle_filled(point(-11.0, 9.0), 2.5, color);
            painter.circle_filled(point(11.0, -9.0), 2.5, color);
        }
        "measure" => {
            line(point(-11.0, 8.0), point(11.0, -8.0));
            line(point(-8.0, 5.0), point(-5.0, 8.0));
            line(point(-3.0, 0.0), point(0.0, 3.0));
            line(point(2.0, -5.0), point(5.0, -2.0));
            painter.circle_filled(point(-11.0, 8.0), 2.0, color);
            painter.circle_filled(point(11.0, -8.0), 2.0, color);
        }
        "curve" => {
            let p0 = point(-11.0, 7.0);
            let p1 = point(-4.0, -10.0);
            let p2 = point(4.0, 10.0);
            let p3 = point(11.0, -7.0);
            let mut previous = p0;
            for step in 1..=16 {
                let t = step as f32 / 16.0;
                let s = 1.0 - t;
                let next = egui::pos2(
                    s.powi(3) * p0.x
                        + 3.0 * s.powi(2) * t * p1.x
                        + 3.0 * s * t.powi(2) * p2.x
                        + t.powi(3) * p3.x,
                    s.powi(3) * p0.y
                        + 3.0 * s.powi(2) * t * p1.y
                        + 3.0 * s * t.powi(2) * p2.y
                        + t.powi(3) * p3.y,
                );
                line(previous, next);
                previous = next;
            }
        }
        "shape" => {
            painter.circle_stroke(center, 9.0, stroke);
            line(point(-12.0, 12.0), point(12.0, 12.0));
            line(point(-12.0, 12.0), point(-12.0, -12.0));
            line(point(-12.0, -12.0), point(12.0, -12.0));
            line(point(12.0, -12.0), point(12.0, 12.0));
        }
        "fill" => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    point(-10.0, 7.0),
                    point(-4.0, -8.0),
                    point(10.0, -5.0),
                    point(7.0, 9.0),
                ],
                color.gamma_multiply(0.32),
                stroke,
            ));
        }
        "erase" => {
            let corners = [
                point(-10.0, -2.0),
                point(-2.0, -10.0),
                point(10.0, 2.0),
                point(2.0, 10.0),
            ];
            for i in 0..4 {
                line(corners[i], corners[(i + 1) % 4]);
            }
            line(point(-4.0, 4.0), point(4.0, -4.0));
        }
        "tint" => {
            painter.circle_filled(point(0.0, 0.0), 8.0, color.gamma_multiply(0.24));
            painter.circle_stroke(point(0.0, 0.0), 8.0, stroke);
            line(point(-3.0, 0.0), point(3.0, 0.0));
            line(point(0.0, -3.0), point(0.0, 3.0));
        }
        "select" => {
            let r = response.rect.shrink(13.0);
            let dash = 4.0;
            let mut x = r.left();
            while x < r.right() {
                let end = (x + dash).min(r.right());
                line(egui::pos2(x, r.top()), egui::pos2(end, r.top()));
                line(egui::pos2(x, r.bottom()), egui::pos2(end, r.bottom()));
                x += dash * 1.7;
            }
            let mut y = r.top();
            while y < r.bottom() {
                let end = (y + dash).min(r.bottom());
                line(egui::pos2(r.left(), y), egui::pos2(r.left(), end));
                line(egui::pos2(r.right(), y), egui::pos2(r.right(), end));
                y += dash * 1.7;
            }
        }
        _ => {}
    }
    response
}

fn inspector_rail_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    selected: bool,
    enabled: bool,
    tooltip: Option<&str>,
) -> egui::Response {
    inspector_icon_button(ui, icon, label, selected, enabled, [44.0, 44.0], tooltip)
}

fn inspector_icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    selected: bool,
    enabled: bool,
    size: [f32; 2],
    tooltip: Option<&str>,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .selected(selected)
            .min_size(size.into()),
    );
    let response = ui_style::accessible_button(response, label, Some(selected));
    let response = response.on_hover_text(tooltip.unwrap_or(label));
    let painter = ui.painter_at(response.rect);
    let colors = ui_style::colors(ui.ctx());
    let color = if !enabled {
        colors.muted_text
    } else if selected {
        colors.focus
    } else {
        colors.text
    };
    let stroke = egui::Stroke::new(1.8_f32, color);
    let center = response.rect.center();
    let p = |x: f32, y: f32| center + egui::vec2(x, y);
    match icon {
        "properties" => {
            for (y, knob_x) in [(-8.0, -4.0), (0.0, 5.0), (8.0, -1.0)] {
                painter.line_segment([p(-10.0, y), p(10.0, y)], stroke);
                painter.circle_filled(p(knob_x, y), 2.6, color);
            }
        }
        "layers" => {
            for (x, y) in [(-3.0, -8.0), (0.0, -2.0), (3.0, 4.0)] {
                painter.rect_stroke(
                    egui::Rect::from_center_size(p(x, y), egui::vec2(16.0, 9.0)),
                    1.5,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
        }
        "references" => {
            painter.rect_stroke(
                egui::Rect::from_center_size(p(-2.0, -2.0), egui::vec2(17.0, 19.0)),
                1.5,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.circle_stroke(p(5.0, 5.0), 4.0, stroke);
            painter.line_segment([p(8.0, 8.0), p(11.0, 11.0)], stroke);
        }
        "images" => {
            painter.rect_stroke(
                egui::Rect::from_center_size(p(0.0, 0.0), egui::vec2(22.0, 18.0)),
                1.5,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.circle_filled(p(6.0, -4.0), 2.0, color);
            painter.add(egui::Shape::line(
                vec![
                    p(-9.0, 6.0),
                    p(-3.0, 0.0),
                    p(1.0, 4.0),
                    p(5.0, 0.0),
                    p(10.0, 6.0),
                ],
                stroke,
            ));
        }
        "collapse" => {
            painter.add(egui::Shape::line(
                vec![p(-3.5, -7.0), p(3.5, 0.0), p(-3.5, 7.0)],
                stroke,
            ));
        }
        "expand" => {
            painter.add(egui::Shape::line(
                vec![p(3.5, -7.0), p(-3.5, 0.0), p(3.5, 7.0)],
                stroke,
            ));
        }
        "hide" => {
            painter.rect_stroke(
                egui::Rect::from_center_size(p(0.0, 0.0), egui::vec2(22.0, 18.0)),
                1.5,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([p(-4.0, -9.0), p(-4.0, 9.0)], stroke);
            painter.line_segment([p(1.0, -3.0), p(7.0, 3.0)], stroke);
            painter.line_segment([p(7.0, -3.0), p(1.0, 3.0)], stroke);
        }
        "sidebar" => {
            painter.rect_stroke(
                egui::Rect::from_center_size(p(0.0, 0.0), egui::vec2(22.0, 18.0)),
                1.5,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([p(5.0, -8.0), p(5.0, 8.0)], stroke);
            painter.add(egui::Shape::line(
                vec![p(-7.0, -4.0), p(-2.0, 0.0), p(-7.0, 4.0)],
                stroke,
            ));
        }
        _ => {}
    }
    response
}

fn inspector_menu_action_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width().max(160.0), 30.0],
        egui::Button::new(""),
    );
    let response = ui_style::accessible_button(response, label, None).on_hover_text(label);
    let painter = ui.painter_at(response.rect);
    let colors = ui_style::colors(ui.ctx());
    let color = colors.text;
    let stroke = egui::Stroke::new(1.6_f32, color);
    let center = egui::pos2(response.rect.left() + 16.0, response.rect.center().y);
    let p = |x: f32, y: f32| center + egui::vec2(x, y);
    painter.rect_stroke(
        egui::Rect::from_center_size(center, egui::vec2(20.0, 16.0)),
        1.5,
        stroke,
        egui::StrokeKind::Middle,
    );
    if icon == "sidebar" {
        painter.line_segment([p(4.0, -7.0), p(4.0, 7.0)], stroke);
        painter.add(egui::Shape::line(
            vec![p(-7.0, -3.0), p(-3.0, 0.0), p(-7.0, 3.0)],
            stroke,
        ));
    } else {
        painter.line_segment([p(-4.0, -7.0), p(-4.0, 7.0)], stroke);
        painter.line_segment([p(0.0, -3.0), p(6.0, 3.0)], stroke);
        painter.line_segment([p(6.0, -3.0), p(0.0, 3.0)], stroke);
    }
    painter.text(
        egui::pos2(response.rect.left() + 34.0, response.rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        color,
    );
    response
}

#[path = "animation_ui.rs"]
mod animation_ui;
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
#[path = "guide_bend_checks.rs"]
mod guide_bend_checks;
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
#[path = "local_color_ui.rs"]
mod local_color_ui;
#[path = "measure_tool.rs"]
mod measure_tool;
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
mod share_ui;
#[path = "swatches_ui.rs"]
mod swatches_ui;
#[derive(Clone, Copy)]
enum LineArtCommand {
    Create,
    Update,
    Bake,
    Cancel,
    SetLive(u64, Option<crate::model_line_art::Settings>),
    BakeLive(u64),
}
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
#[path = "point_edit_ui.rs"]
mod point_edit_ui;
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
#[path = "reference_export_checks.rs"]
mod reference_export_checks;
#[path = "reprojection_ui.rs"]
mod reprojection_ui;
#[path = "revolved_ui.rs"]
mod revolved_ui;
#[path = "ring_checks.rs"]
mod ring_checks;
#[path = "selection.rs"]
mod selection;
#[path = "shape_tool.rs"]
mod shape_tool;
#[path = "snapping.rs"]
mod snapping;
#[path = "starter.rs"]
mod starter;
#[path = "stroke_modifiers_ui.rs"]
mod stroke_modifiers_ui;
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

fn canvas_contact_allowed(
    ctx: &egui::Context,
    canvas: Rect,
    position: Pos2,
    continue_active_contact: bool,
) -> bool {
    canvas_pointer_allowed(ctx, canvas, position)
        || (continue_active_contact && canvas.contains(position))
}

fn filter_quick_palette_items(items: Vec<QuickPaletteItem>, query: &str) -> Vec<QuickPaletteItem> {
    let query = query.trim().to_lowercase();
    items
        .into_iter()
        .filter(|item| {
            query.is_empty()
                || item.label.to_lowercase().contains(&query)
                || item.group.to_lowercase().contains(&query)
                || item.shortcut.to_lowercase().contains(&query)
        })
        .collect()
}

#[derive(Clone, Copy)]
enum QuickPaletteCommand {
    Tool(&'static str),
    Undo,
    Redo,
    Shape(shape_tool::ShapeMode),
    Transform(Mode),
    Deselect,
}

#[derive(Clone)]
struct QuickPaletteItem {
    group: &'static str,
    label: &'static str,
    shortcut: &'static str,
    command: QuickPaletteCommand,
    enabled: bool,
}

struct QuickPaletteState {
    position: Pos2,
    selected: usize,
    query: String,
    search_focus_pending: bool,
}

#[derive(Clone, Copy)]
enum Pending {
    FilledStudy,
    TutorialStart,
    TutorialEnd,
    TutorialClose,
    New,
    House,
    PencilHouse,
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
    measure: measure_tool::MeasureUi,
    curve_tool: curve_tool::CurveUi,
    shape_tool: shape_tool::ShapeTool,
    snapping: snapping::SnapUi,
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
    capability_save_path: Option<PathBuf>,
    insert_preview: Option<(PathBuf, Document)>,
    insert_offset: [f32; 3],
    insert_as_group: bool,
    show_view: bool,
    view_verification_frame: Option<u32>,
    presentation: presentation_ui::PresentationUi,
    placement: placement_ui::PlacementUi,
    plane_picker: planes::PlanePickUi,
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
    animation_ui: animation_ui::AnimationUi,
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
    line_art_draft: Option<crate::model_line_art::Draft>,
    live_line_art_cache: HashMap<u64, crate::model_line_art::CachedLiveLineArt>,
    live_line_art_pending: HashMap<u64, crate::model_line_art::PendingLiveLineArt>,
    line_art_canvas_size: Option<[f32; 2]>,
    line_art_crease_angle: f32,
    line_art_boundaries: bool,
    line_art_creases: bool,
    line_art_silhouettes: bool,
    line_art_command: Option<LineArtCommand>,
    line_art_verification_preview_rect: Option<Rect>,
    line_art_verification_update_rect: Option<Rect>,
    line_art_verification_bake_rect: Option<Rect>,
    obj_reference_verification_asset_root: Option<tempfile::TempDir>,
    appearance_layout_verification: Option<AppearanceLayoutVerification>,
    image_asset_root: PathBuf,
    guide_name: String,
    loft_preview: Option<crate::document::guide_resources::LoftSurface>,
    guide_bend_draft: Option<guide_resource_ui::GuideBendDraft>,
    guide_bend_verification: bool,
    palette_ui: palette_ui::PaletteUi,
    swatches_ui: swatches_ui::SwatchesUi,
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
    layer_properties_draft: Option<layers_ui::LayerPropertiesDraft>,
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
    attach_drawn_strokes: bool,
    stabilization: f32,
    stabilizer: SurfaceStabilizer,
    eraser: bool,
    partial_eraser: partial_eraser_ui::PartialEraserUi,
    local_color_brush: local_color_ui::LocalColorUi,
    select_mode: bool,
    selected: HashSet<u64>,
    quick_palette: Option<QuickPaletteState>,
    suppress_canvas_input_once: bool,
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
    region_select_enabled: bool,
    region_select_shape: selection::RegionSelectionShape,
    region_select_operation: crate::brush_selection::RegionSelectionOperation,
    region_select_contained: bool,
    region_select_drag: Option<selection::RegionSelectionDrag>,
    appearance_draft: Option<selection::AppearanceDraft>,
    alignment_preview: Option<(crate::editing::Snapshot, crate::editing::Snapshot)>,
    alignment_axis: usize,
    alignment_guide_axis: bool,
    alignment_target: usize,
    alignment_distribute: bool,
    point_draft: Option<crate::point_edit::Draft>,
    selection_cleanup_draft: Option<crate::point_edit::SelectionCleanupDraft>,
    point_drag: Option<point_edit_ui::PointDrag>,
    point_multi_draft: Option<crate::point_edit::MultiDraft>,
    point_multi_drag: Option<point_edit_ui::MultiPointRegionDrag>,
    reprojection_draft: Option<crate::reprojection::Draft>,
    last_canvas_rect: Rect,
    point_verification_frame: Option<u32>,
    active: Option<u64>,
    active_started_at: Option<Instant>,
    mouse_down: bool,
    next_id: u64,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    file: Option<PathBuf>,
    dirty: bool,
    live_mcp_requests: Option<std::sync::mpsc::Receiver<crate::live_mcp::LiveUiRequest>>,
    live_mcp_capture_reply: Option<std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>>,
    live_mcp_preview: Option<LiveMcpPreview>,
    status: String,
    last_pen: Option<PenSample>,
    last_pen_at: Option<Instant>,
    pen_previous: Option<Pos2>,
    pen_barrel_contact: Option<bool>,
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
    repeating_texture_preview: Option<(
        (crate::texture::ImageTextureCacheKey, [u32; 3]),
        egui::TextureHandle,
    )>,
    procedural_texture_preview: Option<(crate::texture::TextureCacheKey, egui::TextureHandle)>,
    preset_verification_frame: Option<u32>,
    house_verification_frame: Option<u32>,
    starter_scene: Option<&'static str>,
    cylinder_example: bool,
    cylinder_verification_frame: Option<u32>,
    export_job: Option<export_ui::ExportJob>,
    last_export_result: Option<Result<crate::export::ExportSummary, String>>,
    export_snapshot: Option<export_ui::ExportSnapshot>,
    share_ui: share_ui::ShareUi,
    png_export: png_export_ui::PngExportUi,
    video_export: video_export_ui::VideoExportUi,
    construction_history: crate::construction_history::ConstructionHistory,
    obj_export: obj_export_ui::ObjExportUi,
    png_verification_frame: Option<u32>,
    capability_replay: Option<CapabilityReplay>,
}

struct LiveMcpPreview {
    description: &'static str,
    preview_id: String,
    base_revision: String,
    preview_revision: String,
    selected_ids: HashSet<u64>,
    action: LiveMcpPreviewAction,
}

enum LiveMcpPreviewAction {
    Artwork(Box<Edit>),
    Selection,
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
    ReferenceExport,
    Overview,
    HouseTutorial,
    DuskHouse,
    CurvedTutorial,
}

impl CapabilityClip {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "draw-orbit" => Some(Self::DrawOrbit),
            "curved-guides" => Some(Self::CurvedGuides),
            "mirror-shapes" => Some(Self::MirrorShapes),
            "edit-erase" => Some(Self::EditErase),
            "reference-export" => Some(Self::ReferenceExport),
            "overview" => Some(Self::Overview),
            "house-tutorial" => Some(Self::HouseTutorial),
            "dusk-house" => Some(Self::DuskHouse),
            "curved-tutorial" => Some(Self::CurvedTutorial),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::DrawOrbit => "draw-orbit",
            Self::CurvedGuides => "curved-guides",
            Self::MirrorShapes => "mirror-shapes",
            Self::EditErase => "edit-erase",
            Self::ReferenceExport => "reference-export",
            Self::Overview => "overview",
            Self::HouseTutorial => "house-tutorial",
            Self::DuskHouse => "dusk-house",
            Self::CurvedTutorial => "curved-tutorial",
        }
    }

    fn is_curved_guides(self) -> bool {
        self == Self::CurvedGuides || self == Self::CurvedTutorial
    }

    fn is_curved_tutorial(self) -> bool {
        self == Self::CurvedTutorial
    }

    fn is_house_tutorial(self) -> bool {
        self == Self::HouseTutorial || self == Self::DuskHouse
    }

    fn is_dusk_house(self) -> bool {
        self == Self::DuskHouse
    }

    fn is_overview(self) -> bool {
        self == Self::Overview
    }

    fn is_long_form(self) -> bool {
        self.is_house_tutorial() || self.is_curved_tutorial()
    }

    fn has_tutorial_edit_steps(self) -> bool {
        self.is_long_form() || self.is_overview()
    }

    fn has_gif(self) -> bool {
        true
    }

    fn mp4_hold_frames(self) -> usize {
        if self.is_dusk_house() {
            2
        } else if self.is_long_form() {
            10
        } else if self.is_overview() {
            3
        } else {
            1
        }
    }

    fn is_draw_orbit(self) -> bool {
        self == Self::DrawOrbit
    }

    fn is_edit_erase(self) -> bool {
        self == Self::EditErase
    }

    fn is_mirror_shapes(self) -> bool {
        self == Self::MirrorShapes
    }

    fn is_reference_export(self) -> bool {
        self == Self::ReferenceExport
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
    let extensions: &[&str] = if clip.has_gif() {
        &["gif", "mp4", "png", "en.vtt", "recording.json", "scene.glb"]
    } else {
        &[
            "mp4",
            "png",
            "en.vtt",
            "recording.json",
            "scene.glb",
            "sketch.json",
        ]
    };
    for extension in extensions {
        let path = output_dir.join(format!("{}.{}", clip.name(), extension));
        if path.exists() {
            return Err(format!(
                "Refusing to overwrite existing capability demo output: {}",
                path.display()
            ));
        }
    }
    if clip.is_long_form() {
        let sample = output_dir.join(format!("{}-saved.sketch.json", clip.name()));
        if sample.exists() {
            return Err(format!(
                "Refusing to overwrite capability tutorial sample: {}",
                sample.display()
            ));
        }
    }
    if clip.is_reference_export() && output_dir.join("reference-export").exists() {
        return Err(format!(
            "Refusing to overwrite reference-export fixture directory: {}",
            output_dir.join("reference-export").display()
        ));
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
    selecting: bool,
    setup_verified: bool,
    undo_applied: bool,
    gesture_start: Option<Pos2>,
    gesture_end: Option<Pos2>,
    edit_selection_point: Option<Pos2>,
    edit_processed_frame: Option<usize>,
    expected_undo_strokes: Option<Vec<Stroke>>,
    mirror_points: Option<[Pos2; 4]>,
    curved_points: Option<[Pos2; 3]>,
    curved_world_points: Option<[Vec3; 3]>,
    curved_processed_frame: Option<usize>,
    long_processed_frame: Option<usize>,
    long_saved: bool,
    long_reopened: bool,
    long_edit_verified: bool,
    reference_export_artifact: Option<serde_json::Value>,
    reference_export_action: Option<serde_json::Value>,
    reference_export_action_path: Option<PathBuf>,
    reference_export_action_started: bool,
    reference_insert_path: Option<PathBuf>,
    reference_import_action: bool,
    reference_insert_action: bool,
    reference_inserted_stroke_count: usize,
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
        for extension in [
            "gif",
            "mp4",
            "png",
            "en.vtt",
            "recording.json",
            "scene.glb",
            "sketch.json",
        ] {
            let _ = std::fs::remove_file(self.path(extension));
        }
        if self.clip.is_long_form() {
            let _ = std::fs::remove_file(
                self.output_dir
                    .join(format!("{}-saved.sketch.json", self.clip.name())),
            );
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
        let orbit_sample = if self.clip.is_mirror_shapes() {
            let [line_start, line_end, ellipse_start, ellipse_end] = self
                .mirror_points
                .expect("mirror-shapes replay projects action points onto the front guide");
            match frame {
                0..=2 => (line_start, Phase::Move, false, false, false),
                3 => (line_start, Phase::Down, true, false, false),
                4..=12 => {
                    let t = (frame - 3) as f32 / 9.0;
                    (
                        line_start + (line_end - line_start) * t,
                        Phase::Move,
                        true,
                        false,
                        false,
                    )
                }
                13 => (line_end, Phase::Up, false, false, false),
                14..=16 => (line_end, Phase::Move, false, false, false),
                17 => (ellipse_start, Phase::Move, false, false, false),
                18..=27 => {
                    let t = (frame - 17) as f32 / 10.0;
                    (
                        ellipse_start + (ellipse_end - ellipse_start) * t,
                        Phase::Move,
                        false,
                        false,
                        false,
                    )
                }
                28 => (ellipse_end, Phase::Move, false, false, false),
                29..=32 => (ellipse_end, Phase::Move, false, false, false),
                33..=50 => {
                    let t = (frame - 33) as f32 / 17.0;
                    let start = rect.center() + egui::vec2(0.0, 12.0);
                    let end = start + egui::vec2(270.0, -55.0);
                    (
                        start + (end - start) * t,
                        if frame == 33 {
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
                    rect.center() + egui::vec2(270.0, -43.0),
                    Phase::Up,
                    false,
                    false,
                    false,
                ),
                _ => (
                    rect.center() + egui::vec2(270.0, -43.0),
                    Phase::Move,
                    false,
                    false,
                    false,
                ),
            }
        } else if self.clip.is_curved_guides() {
            let [start, middle, end] = self
                .curved_points
                .expect("curved-guide replay projects action points onto the cylinder");
            match frame {
                0..=3 => (start, Phase::Move, false, false, false),
                4 => (start, Phase::Down, true, false, false),
                5..=11 => {
                    let t = (frame - 4) as f32 / 7.0;
                    (
                        start + (middle - start) * t,
                        Phase::Move,
                        true,
                        false,
                        false,
                    )
                }
                12..=18 => {
                    let t = (frame - 11) as f32 / 7.0;
                    (middle + (end - middle) * t, Phase::Move, true, false, false)
                }
                19 => (end, Phase::Up, false, false, false),
                20..=32 => (end, Phase::Move, false, false, false),
                33..=50 => {
                    let t = (frame - 33) as f32 / 17.0;
                    let orbit_start = rect.center() + egui::vec2(0.0, 12.0);
                    let orbit_end = orbit_start + egui::vec2(270.0, -55.0);
                    (
                        orbit_start + (orbit_end - orbit_start) * t,
                        if frame == 33 {
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
                    rect.center() + egui::vec2(270.0, -43.0),
                    Phase::Up,
                    false,
                    false,
                    false,
                ),
                _ => (end, Phase::Move, false, false, false),
            }
        } else if self.clip.is_reference_export() {
            let start = rect.center() + egui::vec2(0.0, 12.0);
            let end = start + egui::vec2(270.0, -55.0);
            match frame {
                0..=13 => (start, Phase::Move, false, false, false),
                14..=50 => {
                    let t = (frame - 14) as f32 / 36.0;
                    (
                        egui::pos2(
                            start.x + (end.x - start.x) * t,
                            start.y + (end.y - start.y) * t,
                        ),
                        if frame == 14 {
                            Phase::Down
                        } else {
                            Phase::Move
                        },
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
                0..=13 => (
                    self.edit_selection_point.unwrap_or(start),
                    Phase::Move,
                    false,
                    false,
                    false,
                ),
                14..=35 => {
                    let t = (frame - 14) as f32 / 21.0;
                    (
                        egui::pos2(
                            start.x + (end.x - start.x) * t,
                            start.y + (end.y - start.y) * t,
                        ),
                        if frame == 14 {
                            Phase::Down
                        } else {
                            Phase::Move
                        },
                        true,
                        false,
                        true,
                    )
                }
                36 => (end, Phase::Up, false, false, true),
                37..=42 => (end, Phase::Move, false, false, false),
                43..=56 => {
                    let t = (frame - 43) as f32 / 13.0;
                    (
                        rect.center() + egui::vec2(t * 220.0, -t * 45.0),
                        if frame == 43 {
                            Phase::Down
                        } else {
                            Phase::Move
                        },
                        true,
                        true,
                        false,
                    )
                }
                57 => (
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
            let (start, end) = if self.clip.is_dusk_house() {
                let project = |position| {
                    crate::gizmo::project(&self.initial_camera, rect, position)
                        .map(|(screen, _)| screen)
                };
                let start = project(Vec3::new(-0.42, 1.55, 1.68))
                    .unwrap_or(rect.center() + egui::vec2(-45.0, 18.0));
                let end = project(Vec3::new(0.42, 1.55, 1.68))
                    .unwrap_or(rect.center() + egui::vec2(45.0, 18.0));
                (start, end)
            } else if self.clip.is_house_tutorial() || self.clip.is_overview() {
                (
                    rect.center() + egui::vec2(170.0, -190.0),
                    rect.center() + egui::vec2(260.0, -170.0),
                )
            } else {
                (
                    rect.center() + egui::vec2(-150.0, -65.0),
                    rect.center() + egui::vec2(150.0, -35.0),
                )
            };
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
                                + (t * std::f32::consts::TAU).sin()
                                    * if self.clip.is_dusk_house() { 5.0 } else { 28.0 },
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
                _ if self.clip.is_overview() => (rect.center(), Phase::Move, false, false, false),
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
        self.selecting = self.clip.is_edit_erase() && frame <= 12;
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
            egui::pos2(canvas.center().x - 170.0, canvas.top() + 12.0),
            egui::vec2(340.0, 28.0),
        );
        painter.rect_filled(disclosure, 4.0, Color32::from_black_alpha(210));
        painter.text(
            disclosure.center(),
            egui::Align2::CENTER_CENTER,
            "SYNTHETIC REPLAY  •  NO PRESSURE",
            egui::FontId::proportional(12.0),
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
        if self.contact || self.selecting {
            let label = if self.barrel {
                "ORBIT"
            } else if self.selecting {
                "SELECT"
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
        let captions = if self.clip.is_dusk_house() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:00.600\n",
                "A pencil house at dusk: pale walls, cool shadows, and warm windows.\n\n",
                "00:00:00.600 --> 00:00:05.400\n",
                "Draw a graphite detail on the saved front-wall guide.\n\n",
                "00:00:05.400 --> 00:00:06.400\n",
                "Select the new detail, move it, then undo and redo.\n\n",
                "00:00:06.400 --> 00:00:10.200\n",
                "Orbit to reveal the shaded side and roof depth.\n\n",
                "00:00:10.200 --> 00:00:11.400\n",
                "Export the artwork still, save, and reopen the editable drawing.\n\n",
                "00:00:11.400 --> 00:00:12.000\n",
                "Finish with the artwork-only view. Input is synthetic and has no pressure data.\n"
            )
        } else if self.clip.is_house_tutorial() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:03.000\n",
                "Explore the editable garden-house scene. This is a deterministic synthetic-input tutorial.\n\n",
                "00:00:03.000 --> 00:00:27.000\n",
                "Draw a short detail on the front plane and watch the stroke build.\n\n",
                "00:00:27.000 --> 00:00:31.000\n",
                "Select the new stroke, move it, then undo and redo the transform.\n\n",
                "00:00:31.000 --> 00:00:52.000\n",
                "Orbit the camera to inspect the drawing in depth.\n\n",
                "00:00:52.000 --> 00:00:56.000\n",
                "Save the native document through the production save path.\n\n",
                "00:00:56.000 --> 00:01:00.000\n",
                "Reopen the saved native document through the production open path. The scene remains editable; input is synthetic with no pressure data.\n"
            )
        } else if self.clip.is_curved_tutorial() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:04.000\n",
                "Inspect the saved cylinder and its editable drawing guide.\n\n",
                "00:00:04.000 --> 00:00:20.000\n",
                "Draw across the cylinder surface; mirror adds the reflected stroke.\n\n",
                "00:00:20.000 --> 00:00:27.000\n",
                "The source stroke and reflected copy remain editable around the saved cylinder.\n\n",
                "00:00:27.000 --> 00:00:33.000\n",
                "Select the curved stroke, move it, then undo and redo the transform.\n\n",
                "00:00:33.000 --> 00:00:52.000\n",
                "Continue orbiting to inspect the curved result and seam.\n\n",
                "00:00:52.000 --> 00:00:56.000\n",
                "Save the native cylinder drawing through the production save path.\n\n",
                "00:00:56.000 --> 00:01:00.000\n",
                "Reopen the saved native drawing through the production open path. Curved strokes remain editable; no tablet pressure was measured.\n"
            )
        } else if self.clip.is_overview() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:00.900\n",
                "Sketch3D opens on an editable garden-house scene.\n\n",
                "00:00:00.900 --> 00:00:07.800\n",
                "Draw a cyan detail on the front drawing plane.\n\n",
                "00:00:07.800 --> 00:00:10.200\n",
                "Select and move the detail, then undo and redo the transform.\n\n",
                "00:00:10.200 --> 00:00:15.300\n",
                "Orbit the camera to inspect the stroke in 3D.\n\n",
                "00:00:15.300 --> 00:00:18.000\n",
                "Open the production export options. Synthetic input has no pressure data.\n"
            )
        } else if self.clip.is_edit_erase() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:01.400\n",
                "Select the editable stroke and change its color.\n\n",
                "00:00:01.400 --> 00:00:03.700\n",
                "Drag the partial eraser across the stroke to split it.\n\n",
                "00:00:03.700 --> 00:00:04.000\n",
                "Two editable fragments remain.\n\n",
                "00:00:04.000 --> 00:00:04.300\n",
                "Undo restores the edited stroke.\n\n",
                "00:00:04.300 --> 00:00:05.700\n",
                "A synthetic barrel gesture orbits the restored stroke.\n\n",
                "00:00:05.700 --> 00:00:06.000\n",
                "The replay uses synthetic input with no pressure data.\n"
            )
        } else if self.clip.is_reference_export() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:00.800\n",
                "Open the OBJ references controls; this replay uses a generated source file.\n\n",
                "00:00:00.800 --> 00:00:02.000\n",
                "The production Import OBJ button runs with an injected file-picker result; the native picker is not shown.\n\n",
                "00:00:02.000 --> 00:00:03.200\n",
                "Insert the lantern with the production native-merge path; its preview dialog is not shown.\n\n",
                "00:00:03.200 --> 00:00:05.200\n",
                "Start a whole-scene GLB export through the production worker, then orbit the combined scene.\n\n",
                "00:00:05.200 --> 00:00:06.000\n",
                "This replay uses synthetic input; no tablet pressure was measured.\n"
            )
        } else if self.clip.is_mirror_shapes() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:01.300\n",
                "Draw a line on the front guide; mirror creates the reflected stroke.\n\n",
                "00:00:01.300 --> 00:00:02.900\n",
                "Place an editable ellipse on the same guide.\n\n",
                "00:00:03.300 --> 00:00:05.200\n",
                "Orbit to inspect the mirrored line and ellipse.\n\n",
                "00:00:05.200 --> 00:00:06.000\n",
                "Synthetic replay uses no pressure data.\n"
            )
        } else if self.clip.is_curved_guides() {
            concat!(
                "WEBVTT\n\n",
                "00:00:00.000 --> 00:00:02.000\n",
                "Draw across the saved cylinder; the line is reflected around its axis.\n\n",
                "00:00:02.000 --> 00:00:03.300\n",
                "The source stroke and reflected copy are now editable.\n\n",
                "00:00:03.300 --> 00:00:05.200\n",
                "Orbit to inspect the curved result.\n\n",
                "00:00:05.200 --> 00:00:06.000\n",
                "Synthetic replay uses no pressure data.\n"
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
        let captions_timing_checked = false;
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
                "src/reference_export_checks.rs",
                "src/obj_reference_checks.rs",
                "src/obj_reference_ui.rs",
            ])
            .output()
            .map(|output| !output.stdout.is_empty())
            .unwrap_or(true);
        let tutorial_sample = if self.clip.is_long_form() {
            let path = self
                .output_dir
                .join(format!("{}-saved.sketch.json", self.clip.name()));
            let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
            Some(serde_json::json!({
                "file": path.file_name().and_then(|name| name.to_str()).unwrap_or("sample.sketch.json"),
                "bytes": metadata.len(),
                "saved_and_reopened_through_production_handlers": true
            }))
        } else {
            None
        };
        let dimensions = self
            .frame_size
            .ok_or_else(|| "Capability demo did not capture any frames".to_owned())?;
        let clip_name = self.clip.name();
        let (fixture, fixture_provenance, shot_steps, poster_alt) = if self.clip.is_dusk_house() {
            (
                "Editable dusk house starter scene",
                "Original procedural Sketch3D scene with editable planar fills, graphite linework, window-light fills, a ground shadow, and saved wall, roof, and chimney guides.",
                vec![
                    "Draw a graphite accent on the saved front-wall guide through production canvas input.",
                    "Save and reopen the edited native scene through production document handlers, then orbit to reveal its depth.",
                    "Export a full-resolution still through the artwork image exporter and finish in artwork-only view.",
                ],
                "Editable dusk house with warm windows, a cool blue side and roof, ground shadow, and a graphite accent.",
            )
        } else if self.clip.is_house_tutorial() {
            (
                "Japanese house scene",
                "Original Japanese garden-house scene built into Sketch3D; the stroke and transforms are created during capture by production editing handlers.",
                vec![
                    "Draw a detail on the front plane, then select and move it through the production selection handlers.",
                    "Undo and redo the transform, orbit the camera, then save and reopen through production native document paths.",
                    "The native save is stored next to the recording as a sample document.",
                ],
                "A cyan detail stroke selected in clear canvas above the garden-house, with the move gizmo visible.",
            )
        } else if self.clip.is_curved_tutorial() {
            (
                "Saved cylinder drawing guide",
                "An empty saved cylinder guide and reflected strokes are generated by the production guide and pointer handlers; no external assets are used.",
                vec![
                    "Draw across the saved cylinder surface and create the reflected copy through the production pointer handler.",
                    "Select and move the stroke, undo and redo, orbit the camera, then save and reopen through native document paths.",
                    "The native save is stored next to the recording as a sample document.",
                ],
                "A saved cylinder guide with curved editable source and reflected strokes.",
            )
        } else if self.clip.is_overview() {
            (
                "Japanese garden house scene",
                "Original Japanese garden-house scene built into Sketch3D; the detail stroke and camera orbit are created during capture.",
                vec![
                    "Show the editable garden-house scene.",
                    "Draw a detail, select and move it, then undo and redo the transform.",
                    "Orbit to inspect the result and show production GLB export options.",
                ],
                "The selected cyan detail stroke and move gizmo sit in open canvas above the garden-house, away from the title and scene.",
            )
        } else if self.clip.is_reference_export() {
            (
                    "Imported reference, inserted lantern, and exported scene",
                    "The garden-house and lantern drawings are generated from built-in original editable artwork. The reference image and OBJ mesh are generated procedurally for this clip; no external artwork or assets.",
                    vec![
                    "During capture, click the production Import OBJ control with a generated file-picker result; the native picker is not shown.",
                    "During capture, insert the saved cylindrical-lantern drawing with the production native-merge path; the preview dialog is not shown.",
                    "During capture, start a whole-scene GLB export through the production export worker, then orbit using synthetic canvas input.",
                    ],
                    "A garden-house scene with an inserted cylindrical lantern, a reference image plane, and a small OBJ model.",
                )
        } else if self.clip.is_edit_erase() {
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
            "duration_seconds": 6 * self.clip.mp4_hold_frames(),
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
            "input_note": if self.clip.is_reference_export() { "The image-plane reference and first OBJ are prepared through production storage/render paths before capture. During capture the production OBJ Import control receives a generated path through the existing picker-injection hook; the native picker is not shown. The lantern is inserted through the production native-merge handler without showing its preview dialog, then the production whole-scene export worker writes a GLB. Captured orbit samples use pressure=None and no tilt; no tablet pressure or latency was measured." } else if self.clip.is_edit_erase() { "The fixture stroke is created through the production pointer-action handler and its appearance edit through the production selection handler. Captured PenSample input uses pressure=None and no tilt; no tablet pressure or latency was measured." } else if self.clip.is_mirror_shapes() { "A pressure-free synthetic line gesture creates the mirrored pair through the production line handler; a second gesture places the ellipse through the production shape handler. Synthetic barrel samples then orbit through canvas_input. No tablet pressure or latency was measured." } else if self.clip.is_curved_guides() { "Pressure-free synthetic screen positions sampled from the saved cylinder surface create the mirrored stroke through the production pointer-action handler. Synthetic barrel samples then orbit through canvas_input; no tablet pressure or latency was measured." } else if self.clip.is_long_form() { "This deterministic tutorial uses synthetic pressure=None pointer samples. Drawing, selection, transform, undo/redo, native save, and native reopen use production app handlers. MP4 checkpoints are held to make the action steps readable; no tablet pressure or latency was measured." } else { "Synthetic PenSample input uses pressure=None and no tilt. The app applies its fixed default mouse-like width; no tablet pressure or latency was measured." },
            "window_visible": false,
            "frame_rate": CAPABILITY_REPLAY_FPS,
            "mp4_frame_count": CAPABILITY_REPLAY_FRAMES * self.clip.mp4_hold_frames(),
            "shot_steps": shot_steps,
            "cursor_cue": if self.clip.is_edit_erase() { "A synthetic cursor ring shows SELECT over the stroke, ERASE during the partial erase, and ORBIT during camera navigation; a persistent banner discloses no pressure data." } else if self.clip.is_mirror_shapes() { "The synthetic pointer ring is labeled DRAW while drawing the reflected line and ellipse, then ORBIT during camera navigation; a banner discloses no pressure data." } else if self.clip.is_curved_guides() { "The synthetic pointer ring is labeled DRAW while drawing across the cylinder and ORBIT during camera navigation; a persistent banner discloses no pressure data." } else if self.clip.is_long_form() || self.clip.is_overview() { "A synthetic pointer ring is labeled DRAW while drawing, SELECT while editing, and ORBIT during camera navigation; persistent text discloses synthetic input and no pressure data." } else { "A synthetic pointer ring is labeled DRAW or ORBIT while input is active; a persistent banner discloses synthetic replay and no pressure data." },
            "encoding": {
                "mp4": format!("Captured full-viewport RGBA frames; {mp4_backend}; 10 fps; repeated frame holds preserve the {duration}s duration.", duration = 6 * self.clip.mp4_hold_frames()),
                "gif": if self.clip.is_long_form() || self.clip.is_overview() { "Accelerated six-second preview of the same 60 replay checkpoints; plays once without a loop extension. Use the captioned MP4 master for normal-speed instruction." } else { "In-app screenshot frames downscaled to 640x360; image GIF encoder without a loop extension (plays once); 10 fps; <=5 MiB." },
                "poster": "First full-resolution app frame encoded as PNG."
            },
            "outputs": {
                "mp4": { "file": format!("{clip_name}.mp4"), "bytes": mp4_metadata.len() },
                "gif": { "file": format!("{clip_name}.gif"), "bytes": gif_metadata.len() },
                "artwork_still": if self.clip.is_dusk_house() {
                    let path = self.output_dir.join("dusk-house-still.png");
                    let metadata = std::fs::metadata(&path)
                        .map_err(|error| format!("Dusk-house still export is missing: {error}"))?;
                    if metadata.len() == 0 {
                        return Err("Dusk-house still export is empty".into());
                    }
                    Some(serde_json::json!({"file":"dusk-house-still.png","bytes":metadata.len(),"dimensions":[1920,1080],"exported_by":"production artwork image export"}))
                } else {
                    None
                },
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
                },
                "native_sample": tutorial_sample
            },
                "prepared_scene_export": self.reference_export_artifact,
                "capture_export_action": self.reference_export_action,
                "capture_import_action": self.reference_import_action,
                "capture_insert_action": self.reference_insert_action,
            "review": {
                "reviewed": false,
                "loop_boundary_checked": false,
                "intended_readme_surface_checked": false,
                "private_content_checked": false,
                "captions_timing_checked": captions_timing_checked,
                "reviewer": null,
                "notes": if self.clip.is_dusk_house() { "Synthetic 12-second dusk-house walkthrough. The production canvas handler draws on the saved wall guide; production native save/reopen and artwork image export are included. Review playback and caption timing before public release. No tablet pressure was measured." } else if self.clip.is_reference_export() { "Synthetic reference-export draft. During capture the production OBJ import control uses an injected generated file result, the native-merge handler inserts the lantern, and the export worker writes the combined GLB. The native file picker and insert preview are not shown. Full playback, caption-timing, privacy, and intended-surface review are still pending." } else if self.clip.is_house_tutorial() || self.clip.is_curved_tutorial() { "Synthetic 60-second tutorial draft with captured draw/select/move/undo/redo and production native save/reopen steps. The GIF is an accelerated six-second preview; use the MP4 and VTT for instruction. Verify actions, caption timing, playback, privacy, and intended README size before use." } else if self.clip.is_overview() { "Synthetic 18-second overview draft. Review playback, caption timing, privacy, action coverage, and intended README size before use." } else if self.clip.is_edit_erase() { "Synthetic edit-erase draft. Selection and appearance editing run through production handlers during capture, followed by a partial erase, undo and orbit." } else if self.clip.is_mirror_shapes() { "Synthetic mirror-shapes draft. The mirrored line and ellipse are created through production handlers during capture, then reviewed during camera orbit." } else if self.clip.is_curved_guides() { "Synthetic curved-guide draft. The cylinder stroke and reflected copy are created through production canvas input during capture; review playback and visual readability before use." } else { "Synthetic replay draft. The GIF plays once to avoid a false loop seam; review MP4 playback and visual readability before use." }
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
        brush_tip_assets: inserted_tips,
    } = inserted;
    let stroke_ids = merge.strokes.iter().map(|stroke| stroke.id).collect();
    let stroke_count = merge.strokes.len();
    let edit = Edit::Merge(merge);
    edit.apply(target.doc, true);
    for asset in inserted_tips {
        target
            .doc
            .add_brush_tip_asset(asset)
            .expect("prepared insertion already validated brush tip assets");
    }
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

fn dab_target_from_geometry(
    geometry: &crate::document::guide_resources::GuideGeometry,
) -> Option<crate::dynamics::DabSurfaceTarget> {
    use crate::{document::guide_resources::GuideGeometry, dynamics::DabSurfaceTarget};
    Some(match geometry {
        GuideGeometry::Cylinder {
            center,
            rotation,
            radius,
            height,
        } => DabSurfaceTarget::Cylinder {
            center: *center,
            rotation: *rotation,
            radius: *radius,
            height: *height,
        },
        GuideGeometry::Sphere {
            center,
            rotation,
            radius,
        } => DabSurfaceTarget::Sphere {
            center: *center,
            rotation: *rotation,
            radius: *radius,
        },
        GuideGeometry::Ring {
            center,
            rotation,
            major_radius,
            minor_radius,
        } => DabSurfaceTarget::Ring {
            center: *center,
            rotation: *rotation,
            major_radius: *major_radius,
            minor_radius: *minor_radius,
        },
        GuideGeometry::Cone {
            center,
            rotation,
            radius,
            height,
        } => DabSurfaceTarget::Cone {
            center: *center,
            rotation: *rotation,
            radius: *radius,
            height: *height,
        },
        GuideGeometry::Revolved {
            center,
            rotation,
            profile,
        } => {
            let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
            let mut points = [crate::revolved::ProfilePoint {
                height: 0.0,
                radius: 1.0,
            }; 8];
            for (destination, point) in points.iter_mut().zip(profile) {
                *destination = crate::revolved::ProfilePoint {
                    height: point.height,
                    radius: point.radius,
                };
            }
            crate::dynamics::DabSurfaceTarget::Revolved {
                surface: crate::revolved::Revolved {
                    center: *center,
                    rotation_degrees: glam::Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
                    profile: points,
                    count: profile.len(),
                },
            }
        }
        GuideGeometry::Freeform { surface } => {
            crate::dynamics::DabSurfaceTarget::Freeform { surface: *surface }
        }
        GuideGeometry::Loft { surface } => {
            crate::dynamics::DabSurfaceTarget::Loft { surface: *surface }
        }
        _ => return None,
    })
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
        let startup_pencil = crate::presets::startup_pencil();
        let mut preset_ui = crate::presets_ui::PresetUi::new(
            cc.wgpu_render_state
                .clone()
                .expect("wgpu renderer required"),
            library_path,
        );
        preset_ui.choose(startup_pencil.id);
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
            capability_save_path: None,
            insert_preview: None,
            insert_offset: [0.0; 3],
            insert_as_group: false,
            line: Default::default(),
            measure: Default::default(),
            curve_tool: Default::default(),
            shape_tool: Default::default(),
            snapping: Default::default(),
            mirror_ui: Default::default(),
            show_view: false,
            view_verification_frame: None,
            presentation: presentation_ui::PresentationUi::new(),
            placement: Default::default(),
            plane_picker: Default::default(),
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
            animation_ui: Default::default(),
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
            line_art_draft: None,
            live_line_art_cache: HashMap::new(),
            live_line_art_pending: HashMap::new(),
            line_art_canvas_size: None,
            line_art_crease_angle: 35.0,
            line_art_boundaries: true,
            line_art_creases: true,
            line_art_silhouettes: true,
            line_art_command: None,
            line_art_verification_preview_rect: None,
            line_art_verification_update_rect: None,
            line_art_verification_bake_rect: None,
            obj_reference_verification_source_root: None,
            obj_reference_verification_asset_root: None,
            appearance_layout_verification: None,
            image_asset_root,
            guide_name: String::new(),
            loft_preview: None,
            guide_bend_draft: None,
            guide_bend_verification: false,
            palette_ui: palette_ui::PaletteUi::new(),
            swatches_ui: Default::default(),
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
            layer_properties_draft: None,
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
            color: startup_pencil.color,
            radius: startup_pencil.radius,
            brush: startup_pencil.brush,
            attach_drawn_strokes: false,
            stabilization: startup_pencil.stabilization,
            stabilizer: SurfaceStabilizer::default(),
            eraser: false,
            partial_eraser: Default::default(),
            local_color_brush: Default::default(),
            select_mode: false,
            selected: HashSet::new(),
            quick_palette: None,
            suppress_canvas_input_once: false,
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
            region_select_enabled: false,
            region_select_shape: selection::RegionSelectionShape::Box,
            region_select_operation: crate::brush_selection::RegionSelectionOperation::Replace,
            region_select_contained: false,
            region_select_drag: None,
            appearance_draft: None,
            alignment_preview: None,
            alignment_axis: 0,
            alignment_guide_axis: false,
            alignment_target: 0,
            alignment_distribute: false,
            point_draft: None,
            selection_cleanup_draft: None,
            point_drag: None,
            point_multi_draft: None,
            point_multi_drag: None,
            reprojection_draft: None,
            last_canvas_rect: Rect::NOTHING,
            point_verification_frame: std::env::args()
                .any(|arg| arg == "--verify-point-edit")
                .then_some(0),
            active: None,
            active_started_at: None,
            mouse_down: false,
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            file: None,
            dirty: false,
            live_mcp_requests: None,
            live_mcp_capture_reply: None,
            live_mcp_preview: None,
            status: "Ready. Draw on the highlighted plane.".into(),
            last_pen: None,
            last_pen_at: None,
            pen_previous: None,
            pen_barrel_contact: None,
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
            repeating_texture_preview: None,
            procedural_texture_preview: None,
            preset_verification_frame: None,
            house_verification_frame: None,
            starter_scene: None,
            cylinder_example: false,
            cylinder_verification_frame: None,
            export_job: None,
            last_export_result: None,
            export_snapshot: None,
            share_ui: share_ui::ShareUi::default(),
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
                selecting: false,
                setup_verified: !clip.is_curved_guides()
                    && !clip.is_mirror_shapes()
                    && !clip.is_reference_export()
                    && !clip.is_edit_erase(),
                undo_applied: false,
                gesture_start: None,
                gesture_end: None,
                expected_undo_strokes: None,
                edit_selection_point: None,
                edit_processed_frame: None,
                mirror_points: None,
                curved_points: None,
                curved_world_points: None,
                curved_processed_frame: None,
                long_processed_frame: None,
                long_saved: false,
                long_reopened: false,
                long_edit_verified: false,
                reference_export_artifact: None,
                reference_export_action: None,
                reference_export_action_path: None,
                reference_export_action_started: false,
                reference_insert_path: None,
                reference_import_action: false,
                reference_insert_action: false,
                reference_inserted_stroke_count: 0,
                sender: None,
                encoder: None,
                mp4_sender: None,
                mp4_encoder: None,
            });
            if clip.is_long_form() {
                app.capability_save_path = Some(
                    app.capability_replay
                        .as_ref()
                        .unwrap()
                        .output_dir
                        .join(format!("{}-saved.sketch.json", clip.name())),
                );
            }
            app.stabilization = 0.0;
            if clip.is_curved_guides() {
                app.mirror_verification_frame = Some(0);
            } else if !clip.is_edit_erase()
                && !clip.is_mirror_shapes()
                && !clip.is_reference_export()
            {
                if clip.is_dusk_house() {
                    app.load_pencil_house();
                    app.doc.active_guide = Some(1);
                    if let Some(guide) = app
                        .doc
                        .guide_resources
                        .iter_mut()
                        .find(|guide| guide.id == 1)
                    {
                        guide.visible = true;
                    }
                    app.attach_drawn_strokes = true;
                    app.brush = crate::presets::startup_pencil().brush;
                    app.color = crate::presets::startup_pencil().color;
                    app.radius = 0.025;
                } else {
                    app.load_house();
                    app.guide = DrawingPlane::from_preset(Plane::Front);
                    app.guide.depth = app.camera.target.z;
                }
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
                        assert!(Self::mixed_transform_supported(
                            crate::mixed_resources::TransformMode::Rotate
                        ));
                        assert!(Self::mixed_transform_supported(
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
                    live_line_art: None,
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
        } else if std::env::args().any(|a| a == "--verify-guide-bend") {
            app.verification_frame = None;
            app.guide_bend_verification = true;
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
        } else if std::env::args()
            .any(|a| a == "--verify-fills" || a == "--verify-fills-no-capture")
        {
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
            app.load_pencil_house();
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
                app.load_pencil_house();
            }
        }
        app.renderer.grain = app.presentation.preferences.grain;
        app.renderer.toon_shading = app.presentation.preferences.toon_shading;
        app.renderer.post_effects = app.presentation.preferences.post_effects;
        app.recovery.request_scan();
        app.sync_presentation_background();
        app
    }

    pub fn new_with_live_mcp(
        cc: &eframe::CreationContext<'_>,
        requests: std::sync::mpsc::Receiver<crate::live_mcp::LiveUiRequest>,
    ) -> Self {
        let mut app = Self::new(cc);
        app.live_mcp_requests = Some(requests);
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
        } else if self.local_color_brush.enabled {
            "tint"
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
        self.point_drag = None;
        self.finish_layer_properties_edit(true);
        self.finish_local_color(true);
        let model_surface_stroke = self.model_stroke_contact.is_some();
        self.finish_appearance_edit(false);
        self.finish_partial_erase(true);
        self.cancel_line();
        self.resume_ink = false;
        self.model_stroke_contact = None;
        self.commit_transform();
        let tail = self.stabilizer.finish();
        let flow_finish_time = self.current_flow_time_ms();
        let stroke_finish_time = self.current_stroke_time_ms();
        if let Some(id) = self.active {
            if let Some(stroke) = self.doc.strokes.last_mut().filter(|s| s.id == id) {
                for point in tail {
                    if stroke.points.last().is_none_or(|p| {
                        p.position != point.position
                            || p.pressure != point.pressure
                            || p.tilt != point.tilt
                    }) {
                        stroke.points.push(point);
                        if crate::dab_placement::point_count(stroke).is_err()
                            || strand_vertex_budget_exceeded(stroke)
                            || profile_tube_vertex_budget_exceeded(stroke)
                        {
                            stroke.points.pop();
                            self.status = "Stroke reached its dab, strand, or tube-profile geometry limit; the remaining release tail was omitted.".into();
                            break;
                        }
                    }
                }
                if stroke.brush.flow.enabled
                    && stroke
                        .points
                        .last()
                        .is_some_and(|last| last.time_ms < flow_finish_time)
                {
                    let mut point = *stroke.points.last().unwrap();
                    point.time_ms = flow_finish_time;
                    stroke.points.push(point);
                    if airbrush_vertex_budget_exceeded(stroke) {
                        stroke.points.pop();
                        self.status = "Airbrush reached its 8,192-deposit limit; the remaining contact was omitted.".into();
                    }
                }
                if !stroke.brush.flow.enabled && stroke.brush.texture.dynamics.speed_enabled() {
                    if let Some(last) = stroke.points.last_mut() {
                        last.time_ms = last.time_ms.max(stroke_finish_time);
                    }
                }
                if stroke.brush.dabs.enabled && stroke.brush.dab_total_length == 0.0 {
                    stroke.brush.dab_total_length = crate::dynamics::arc_lengths(&stroke.points)
                        .last()
                        .copied()
                        .unwrap_or(0.0);
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

    fn dab_scatter_supported(&self) -> bool {
        if let Some(model_id) = self.doc.active_model_reference {
            return self.obj_reference_models.contains_key(&model_id)
                && self.doc.model_references.iter().any(|resource| {
                    resource.id == model_id && resource.visible && !resource.locked
                });
        }
        self.saved_guide_geometry().is_none_or(|geometry| {
            matches!(
                geometry,
                crate::document::guide_resources::GuideGeometry::Plane { .. }
            ) || dab_target_from_geometry(geometry).is_some()
        })
    }

    fn captured_dab_surface(
        &self,
        geometry: Option<&crate::document::guide_resources::GuideGeometry>,
    ) -> Option<crate::dynamics::DabSurfaceTarget> {
        if let Some(geometry) = geometry {
            return dab_target_from_geometry(geometry);
        }
        if self.use_cylinder {
            Some(crate::dynamics::DabSurfaceTarget::Cylinder {
                center: self.cylinder.center,
                rotation: self.cylinder.rotation(),
                radius: self.cylinder.radius,
                height: self.cylinder.height,
            })
        } else if self.use_revolved {
            Some(crate::dynamics::DabSurfaceTarget::Revolved {
                surface: self.revolved,
            })
        } else {
            None
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

    fn current_flow_time_ms(&self) -> u32 {
        if !self
            .doc
            .strokes
            .last()
            .is_some_and(|stroke| stroke.brush.flow.enabled)
        {
            return 0;
        }
        self.current_stroke_time_ms()
    }

    fn current_stroke_time_ms(&self) -> u32 {
        self.active_started_at.map_or(0, |started| {
            started
                .elapsed()
                .as_millis()
                .min(crate::document::MAX_FLOW_DURATION_MS as u128) as u32
        })
    }

    fn current_texture_time_ms(&self) -> u32 {
        if self
            .doc
            .strokes
            .last()
            .is_some_and(|stroke| stroke.brush.texture.dynamics.speed_enabled())
        {
            self.current_stroke_time_ms()
        } else {
            self.current_flow_time_ms()
        }
    }

    fn tick_airbrush_contact(&mut self, ctx: &egui::Context) {
        let Some(id) = self.active else {
            return;
        };
        let elapsed = self.current_flow_time_ms();
        let mut limit_reached = false;
        let Some(stroke) = self.doc.strokes.last_mut().filter(|stroke| stroke.id == id) else {
            return;
        };
        if !stroke.brush.flow.enabled {
            return;
        }
        let Some(last) = stroke.points.last().copied() else {
            return;
        };
        let due_at = last
            .time_ms
            .saturating_add(crate::geometry::AIRBRUSH_TICK_MS);
        if elapsed >= due_at && elapsed > last.time_ms {
            let mut point = last;
            point.time_ms = elapsed;
            stroke.points.push(point);
            if airbrush_vertex_budget_exceeded(stroke) {
                stroke.points.pop();
                limit_reached = true;
            } else {
                self.renderer.invalidate();
            }
        } else if elapsed >= crate::document::MAX_FLOW_DURATION_MS {
            limit_reached = true;
        } else {
            ctx.request_repaint_after(Duration::from_millis(
                due_at.saturating_sub(elapsed).max(1) as u64
            ));
        }
        if limit_reached {
            self.status = "Airbrush contact reached its time or geometry limit.".into();
            self.finish();
        } else {
            ctx.request_repaint_after(Duration::from_millis(
                crate::geometry::AIRBRUSH_TICK_MS as u64,
            ));
        }
    }
    fn cancel(&mut self) {
        self.pen_barrel_contact = None;
        self.placement.stamp = None;
        self.plane_picker.active = false;
        self.plane_picker.hovered_guide = None;
        self.finish_canvas_point_edit(false);
        self.finish_multi_point_edit(false);
        self.cancel_multi_point_region();
        self.finish_reprojection(false);
        self.finish_fill_contour_edit(false);
        self.cancel_brush_selection();
        self.cancel_region_selection();
        self.cancel_partial_erase();
        self.cancel_local_color();
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
        self.add_point_with_tilt_source(position, pressure, false, rect, begin, tilt);
    }

    fn add_point_with_tilt_source(
        &mut self,
        position: Pos2,
        pressure: f32,
        pressure_samples_available: bool,
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
        if (self.brush.tip_asset_id != 0 && !self.dab_scatter_supported())
            || (self.brush.dabs.enabled
                && self.brush.dabs.scatter > 0.0
                && !self.dab_scatter_supported())
        {
            self.finish();
            self.resume_ink = begin;
            self.status = if self.brush.tip_asset_id != 0 {
                "Image stamps support planes, saved curved guides, and captured OBJ surface patches.".into()
            } else {
                "Dab scatter supports planes, saved curved guides, and captured OBJ surface patches.".into()
            };
            return;
        }
        if self.brush.texture.anchor_mode == crate::texture::TextureAnchorMode::CapturedPlanar
            && self.doc.active_model_reference.is_some()
        {
            self.finish();
            self.resume_ink = false;
            self.status = "Captured planar texture anchoring supports flat work, saved planes, and image planes; use stroke-following mapping on OBJ surfaces.".into();
            return;
        }
        if self.doc.active_model_reference.is_some() {
            self.add_model_point_with_tilt(
                position,
                pressure,
                pressure_samples_available,
                rect,
                begin,
                tilt,
            );
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
        let saved_planar_frame = saved_geometry.as_ref().and_then(|geometry| match geometry {
            crate::document::guide_resources::GuideGeometry::Plane { origin, rotation } => {
                Some(crate::texture::TextureFrame::from_plane(*origin, *rotation))
            }
            _ => None,
        });
        let saved_guide_hit = saved_geometry
            .as_ref()
            .and_then(|geometry| geometry.clone().hit(origin, direction));
        let saved_surface = self.saved_guide_surface();
        let saved_guide_frame = saved_guide_hit.and_then(|world| self.saved_guide_frame(world));
        let unsupported_texture_surface = if self.doc.active_guide.is_some() {
            saved_planar_frame.is_none()
        } else if self.doc.active_image_plane.is_some() {
            active_image_plane.is_none()
        } else {
            self.use_cylinder || self.use_revolved
        };
        if self.brush.texture.anchor_mode == crate::texture::TextureAnchorMode::CapturedPlanar
            && unsupported_texture_surface
        {
            self.finish();
            self.resume_ink = false;
            self.status = "Captured planar texture anchoring supports flat work, saved planes, and drawable image planes; use stroke-following mapping on curved surfaces.".into();
            return;
        }
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
            let texture = if self.brush.texture.anchor_mode
                == crate::texture::TextureAnchorMode::CapturedPlanar
            {
                let current_frame = active_image_plane
                    .and_then(|(image_plane_id, rotation)| {
                        self.doc
                            .image_planes
                            .iter()
                            .find(|plane| plane.id == image_plane_id)
                            .map(|plane| {
                                crate::texture::TextureFrame::from_plane(plane.origin, rotation)
                            })
                    })
                    .or(saved_planar_frame)
                    .unwrap_or_else(|| {
                        crate::texture::TextureFrame::from_drawing_plane(self.guide)
                    });
                let frame = self.brush.texture.frame.unwrap_or(current_frame);
                self.brush.texture.frame = Some(frame);
                crate::texture::TextureSettings {
                    frame: Some(frame),
                    ..self.brush.texture
                }
            } else {
                self.brush.texture
            };
            let texture = crate::texture::TextureSettings {
                pressure_samples_available,
                ..texture
            };
            let attachable_brush = self.doc.active_image_plane.is_none()
                && self.brush.dabs.surface_target.is_none()
                && self.brush.texture.anchor_mode
                    != crate::texture::TextureAnchorMode::CapturedPlanar;
            let guide_attachment = self.attach_drawn_strokes
                .then(|| {
                    self.doc.active_guide.filter(|_| {
                        saved_geometry.as_ref().is_some_and(|geometry| {
                            matches!(
                                geometry,
                                crate::document::guide_resources::GuideGeometry::Plane { .. }
                                    | crate::document::guide_resources::GuideGeometry::Cylinder { .. }
                            )
                        })
                    })
                })
                .flatten()
                .filter(|_| attachable_brush);
            if self.attach_drawn_strokes && guide_attachment.is_none() {
                self.status =
                    "This active guide or brush cannot attach the new stroke. It was drawn freely."
                        .into();
            }
            self.doc.strokes.push(Stroke {
                guide_attachment,
                model_surface: None,
                curve: None,
                material: Default::default(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: self.doc.active_layer,
                id,
                color: self.color,
                gradient: None,
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
                    dab_seed: crate::dab_placement::seed_for_stroke(id),
                    dab_distance_origin: 0.0,
                    dab_total_length: 0.0,
                    dabs: crate::dynamics::DabSettings {
                        surface_target: self.captured_dab_surface(saved_geometry.as_ref()),
                        ..self.brush.dabs
                    },
                    texture,
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
        let point_time_ms = self.current_texture_time_ms();
        let points = self.stabilizer.push(
            Point {
                tilt,
                position: world,
                pressure,
                time_ms: point_time_ms,
            },
            self.stabilization * 24.0 * world_per_pixel,
        );
        if let Some(stroke) = self.doc.strokes.last_mut().filter(|s| s.id == id) {
            stroke.brush.texture.pressure_samples_available |= pressure_samples_available;
            let mut over_geometry_limit = false;
            for point in points {
                if stroke.points.last().is_none_or(|p| {
                    p.position.distance(point.position) > 0.001
                        || (p.pressure - point.pressure).abs() > 0.025
                        || p.tilt != point.tilt
                        || (stroke.brush.flow.enabled
                            && point.time_ms.saturating_sub(p.time_ms)
                                >= crate::geometry::AIRBRUSH_TICK_MS)
                }) {
                    stroke.points.push(point);
                    if crate::dab_placement::point_count(stroke).is_err()
                        || strand_vertex_budget_exceeded(stroke)
                        || airbrush_vertex_budget_exceeded(stroke)
                        || profile_tube_vertex_budget_exceeded(stroke)
                    {
                        stroke.points.pop();
                        over_geometry_limit = true;
                        break;
                    }
                }
            }
            if over_geometry_limit {
                self.status = "Stroke reached its airbrush, dab, strand, or tube-profile geometry limit; further input was ignored.".into();
            }
        }
    }

    fn add_model_point_with_tilt(
        &mut self,
        position: Pos2,
        pressure: f32,
        pressure_samples_available: bool,
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
        let capture_model_surface = self.brush.tip_asset_id != 0 || self.brush.dabs.enabled;
        let contact_patch = if capture_model_surface {
            match self
                .obj_surface_indices
                .get(&id)
                .expect("model surface index was built")
                .surface_contact_patch(
                    model,
                    hit.triangle_index,
                    resource.center,
                    resource.rotation,
                    resource.scale,
                ) {
                Ok(patch) => Some(patch),
                Err(error) => {
                    self.finish();
                    self.model_stroke_contact = None;
                    self.resume_ink = true;
                    self.status = format!("Cannot capture OBJ dab surface: {error}");
                    return;
                }
            }
        } else {
            None
        };
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
                guide_attachment: None,
                model_surface: capture_model_surface
                    .then(crate::document::ModelSurfacePatch::default),
                curve: None,
                material: Default::default(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: self.doc.active_layer,
                id,
                color: self.color,
                gradient: None,
                radius: self.radius,
                brush: Brush {
                    normal,
                    tangent,
                    dab_seed: crate::dab_placement::seed_for_stroke(id),
                    dab_distance_origin: 0.0,
                    dab_total_length: 0.0,
                    dabs: crate::dynamics::DabSettings {
                        surface_target: None,
                        ..self.brush.dabs
                    },
                    texture: crate::texture::TextureSettings {
                        pressure_samples_available,
                        ..self.brush.texture
                    },
                    ..self.brush
                },
                points: Vec::new(),
            });
        }
        let Some(stroke_id) = self.active else {
            return;
        };
        let point_time_ms = self.current_texture_time_ms();
        let point = Point {
            tilt,
            position: world,
            pressure: pressure.clamp(0.0, 1.0),
            time_ms: point_time_ms,
        };
        let active_patch = self
            .doc
            .strokes
            .last()
            .filter(|stroke| stroke.id == stroke_id)
            .and_then(|stroke| stroke.model_surface.as_ref());
        let new_patch_triangles = contact_patch.as_ref().map_or(0, |contact| {
            contact
                .iter()
                .filter(|triangle| {
                    !active_patch.is_some_and(|patch| {
                        patch
                            .triangles
                            .binary_search_by_key(&triangle.source_id, |item| item.source_id)
                            .is_ok()
                    })
                })
                .count()
        });
        let document_patch_triangles = if new_patch_triangles == 0 {
            0
        } else {
            self.doc
                .strokes
                .iter()
                .filter_map(|stroke| stroke.model_surface.as_ref())
                .map(|patch| patch.triangles.len())
                .sum()
        };
        let patch_limit_reached = active_patch.is_some_and(|patch| {
            patch.triangles.len().saturating_add(new_patch_triangles)
                > crate::document::MAX_MODEL_SURFACE_TRIANGLES_PER_STROKE
        }) || document_patch_triangles
            .saturating_add(new_patch_triangles)
            > crate::document::MAX_MODEL_SURFACE_TRIANGLES_PER_DOCUMENT;
        if patch_limit_reached {
            self.status = "Model dab target reached its per-stroke or document triangle limit; further input was ignored.".into();
            self.finish();
            self.resume_ink = true;
            self.model_stroke_contact = None;
            return;
        }
        if let Some(stroke) = self
            .doc
            .strokes
            .last_mut()
            .filter(|stroke| stroke.id == stroke_id)
        {
            stroke.brush.texture.pressure_samples_available |= pressure_samples_available;
            if let (Some(surface), Some(contact)) =
                (&mut stroke.model_surface, contact_patch.as_ref())
            {
                for triangle in contact {
                    let _ = surface.insert(*triangle);
                }
            }
            if stroke.points.last().is_none_or(|last| {
                last.position.distance(point.position) > 0.001
                    || (last.pressure - point.pressure).abs() > 0.025
                    || last.tilt != point.tilt
                    || (stroke.brush.flow.enabled
                        && point.time_ms.saturating_sub(last.time_ms)
                            >= crate::geometry::AIRBRUSH_TICK_MS)
            }) {
                stroke.points.push(point);
                if crate::dab_placement::point_count(stroke).is_err()
                    || strand_vertex_budget_exceeded(stroke)
                    || airbrush_vertex_budget_exceeded(stroke)
                    || profile_tube_vertex_budget_exceeded(stroke)
                {
                    stroke.points.pop();
                    self.status = "Stroke reached its airbrush, dab, strand, or tube-profile geometry limit; further input was ignored.".into();
                }
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
            let stroke_id = self.doc.strokes[i].id;
            if self.doc.linked_instance_for_stroke(stroke_id).is_some() {
                self.status = "Delete the linked instance from the selection controls, or make it unique before erasing.".into();
                return;
            }
            if self.doc.has_linked_source(stroke_id) {
                self.status = "A linked source cannot be erased while instances use it. Delete or make unique its instances first.".into();
                return;
            }
            let stroke = self.doc.strokes.remove(i);
            self.undo.push(Edit::Remove(i, stroke));
            self.redo.clear();
            self.dirty = true;
            self.record_recovery_edit_labeled("Erase");
        }
    }
    fn undo(&mut self) {
        if self.alignment_preview.is_some() {
            self.status = "Apply or cancel the alignment preview before undoing.".into();
            return;
        }
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
        if self.curve_draft_active() {
            self.cancel_curve();
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
                Edit::ModelReferences { .. }
                    | Edit::BakeLiveLineArt { .. }
                    | Edit::MixedResources { .. }
                    | Edit::Merge(_)
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
        if self.alignment_preview.is_some() {
            self.status = "Apply or cancel the alignment preview before redoing.".into();
            return;
        }
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
        if self.curve_draft_active() {
            self.cancel_curve();
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
                Edit::ModelReferences { .. }
                    | Edit::BakeLiveLineArt { .. }
                    | Edit::MixedResources { .. }
                    | Edit::Merge(_)
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
        if !self.doc.animation.is_empty()
            && !self.doc.animation.current_pose_matches(&self.doc.strokes)
        {
            self.status = "Set or update the current artwork keyframe before saving.".into();
            return false;
        }
        if self.selection_cleanup_draft.is_some()
            || self.point_draft.is_some()
            || self.point_multi_draft.is_some()
            || self.reprojection_draft.is_some()
        {
            self.status = "Apply or cancel point editing before saving.".into();
            return false;
        }
        if self.mixed_resource_preview_active() {
            self.status = "Apply or cancel the mixed-resource preview before saving.".into();
            return false;
        }
        if self.alignment_preview.is_some() {
            self.status = "Apply or cancel the alignment preview before saving.".into();
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
        if let Err(error) = crate::dab_placement::document_count(&self.doc) {
            self.status = format!("Save failed: {error}");
            return false;
        }
        let path = if save_as || self.file.is_none() {
            if self
                .capability_replay
                .as_ref()
                .is_some_and(|replay| replay.clip.is_long_form())
            {
                self.capability_save_path.take()
            } else {
                rfd::FileDialog::new()
                    .add_filter("Sketch3D", &["json"])
                    .set_file_name("drawing.sketch.json")
                    .save_file()
            }
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
                    self.clear_live_line_art_cache();
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
            Some(Pending::PencilHouse) => self.load_pencil_house(),
            Some(Pending::Lantern) => self.load_lantern(),
            Some(Pending::Brushes) => self.load_brushes(),
            Some(Pending::Dynamics) => self.load_dynamics(),
            Some(Pending::Textures) => self.load_textures(),
            Some(Pending::New) => {
                self.reset_recovery_document();
                self.starter_scene = None;
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
                self.clear_live_line_art_cache();
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
                                .chain(
                                    doc.animation
                                        .keyframes
                                        .iter()
                                        .flat_map(|keyframe| keyframe.strokes.iter().map(|s| s.id)),
                                )
                                .chain(doc.animation.base_strokes.iter().flatten().map(|s| s.id))
                                .chain(doc.animation.layer_tracks.iter().flat_map(|track| {
                                    track
                                        .keyframes
                                        .iter()
                                        .flat_map(|keyframe| keyframe.strokes.iter().map(|s| s.id))
                                }))
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
                            self.clear_live_line_art_cache();
                            self.obj_surface_indices.clear();
                            self.selected_model_reference = None;
                            self.image_asset_root =
                                crate::image_plane_storage::asset_root(&path).unwrap_or_default();
                            self.renderer.clear_image_plane_assets();
                            for (id, asset) in &self.image_plane_assets {
                                let _ = self.renderer.set_image_plane_asset(*id, asset);
                            }
                            self.refresh_obj_reference_meshes();
                            self.starter_scene = None;
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
                    time_ms: 0,
                });
            }
            self.doc.strokes.push(Stroke {
                guide_attachment: None,
                model_surface: None,
                curve: None,
                material: Default::default(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: None,
                group_id: None,
                layer_id: self.doc.active_layer,
                id: self.next_id,
                color: [[0.17, 0.82, 0.74], [0.96, 0.55, 0.30], [0.47, 0.58, 0.95]][j],
                gradient: None,
                radius: 0.035,
                brush: Brush::default(),
                points,
            });
            self.next_id += 1;
        }
        self.status = "Demo scene. New starts a blank drawing.".into();
    }
    fn choose_tool(&mut self, tool: &str) {
        if tool != "Measure" {
            self.cancel_measure_tool();
        }
        let saved_cylinder_fill = tool == "Fill" && self.active_fill_cylinder().is_some();
        if self.resource_target_active()
            && matches!(tool, "Line" | "Fill" | "Circle" | "Ellipse" | "Curve")
            && !saved_cylinder_fill
        {
            self.status = "The active image/saved guide currently accepts brush strokes. Switch to the temporary guide for line, fill, shape, or curve tools.".into();
            return;
        }
        if saved_cylinder_fill {
            self.fills.region_mode = true;
        }
        if tool != "Paint" && self.local_color_brush.enabled {
            self.finish_local_color(true);
            self.local_color_brush.enabled = false;
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
            "Paint" => {
                self.finish();
                self.cancel_curve_tool();
                self.shape_tool.mode = None;
                self.cancel_line_tool();
                self.cancel_fill_tool();
                self.eraser = false;
                self.select_mode = false;
                self.local_color_brush.enabled = true;
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
            "Measure" => self.start_measure_tool(),
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

    fn quick_palette_items(&self) -> Vec<QuickPaletteItem> {
        let unsupported_surface = self.resource_target_active();
        let tools = [
            ("Brush", "B", QuickPaletteCommand::Tool("Brush")),
            ("Line", "L", QuickPaletteCommand::Tool("Line")),
            ("Measure", "M", QuickPaletteCommand::Tool("Measure")),
            ("Curve", "C", QuickPaletteCommand::Tool("Curve")),
            ("Shape", "", QuickPaletteCommand::Tool("Circle")),
            ("Fill", "P", QuickPaletteCommand::Tool("Fill")),
            ("Erase", "E", QuickPaletteCommand::Tool("Erase")),
            ("Tint", "T", QuickPaletteCommand::Tool("Paint")),
            ("Select", "V", QuickPaletteCommand::Tool("Select")),
        ];
        let mut items: Vec<_> = tools
            .into_iter()
            .map(|(label, shortcut, command)| {
                let unsupported_tool = matches!(label, "Line" | "Curve" | "Shape")
                    || (label == "Fill" && self.active_fill_cylinder().is_none());
                let enabled = !(unsupported_surface && unsupported_tool);
                QuickPaletteItem {
                    group: "Tools",
                    label,
                    shortcut,
                    command,
                    enabled,
                }
            })
            .collect();
        items.extend([
            QuickPaletteItem {
                group: "History",
                label: "Undo",
                shortcut: "Ctrl/Cmd+Z",
                command: QuickPaletteCommand::Undo,
                enabled: !self.undo.is_empty(),
            },
            QuickPaletteItem {
                group: "History",
                label: "Redo",
                shortcut: "Ctrl/Cmd+Shift+Z",
                command: QuickPaletteCommand::Redo,
                enabled: !self.redo.is_empty(),
            },
        ]);
        if self.shape_tool.mode.is_some() {
            items.extend(
                [
                    (shape_tool::ShapeMode::Circle, "Circle"),
                    (shape_tool::ShapeMode::Ellipse, "Ellipse"),
                    (shape_tool::ShapeMode::Rectangle, "Rectangle"),
                    (shape_tool::ShapeMode::RoundedRectangle, "Rounded rectangle"),
                ]
                .into_iter()
                .map(|(mode, label)| QuickPaletteItem {
                    group: "Shape options",
                    label,
                    shortcut: "",
                    command: QuickPaletteCommand::Shape(mode),
                    enabled: !unsupported_surface,
                }),
            );
        }
        if self.select_mode {
            items.extend(
                [
                    (Mode::Move, "Move"),
                    (Mode::Rotate, "Rotate"),
                    (Mode::Scale, "Scale"),
                ]
                .into_iter()
                .map(|(mode, label)| QuickPaletteItem {
                    group: "Selection options",
                    label,
                    shortcut: "",
                    command: QuickPaletteCommand::Transform(mode),
                    enabled: true,
                }),
            );
        }
        if !self.selected.is_empty() {
            items.push(QuickPaletteItem {
                group: "Selection",
                label: "Deselect all",
                shortcut: "",
                command: QuickPaletteCommand::Deselect,
                enabled: true,
            });
        }
        items
    }

    fn filtered_quick_palette_items(&self) -> Vec<QuickPaletteItem> {
        let query = self
            .quick_palette
            .as_ref()
            .map(|state| state.query.trim().to_lowercase())
            .unwrap_or_default();
        filter_quick_palette_items(self.quick_palette_items(), &query)
    }

    fn quick_palette_active_tool(&self) -> &'static str {
        if self.select_mode {
            "Select"
        } else if self.local_color_brush.enabled {
            "Tint"
        } else if self.eraser {
            "Erase"
        } else if self.fills.enabled {
            "Fill"
        } else if self.line.enabled {
            "Line"
        } else if self.curve_tool.enabled {
            "Curve"
        } else if self.shape_tool.mode.is_some() {
            "Shape"
        } else {
            "Brush"
        }
    }

    fn can_open_quick_palette(&self, ctx: &egui::Context) -> bool {
        !self.confirm
            && !self.show_help
            && !self.recovery_prompt
            && !self.tutorial.offer
            && !self.close_after_recovery
            && !self.presentation.artwork
            && !self.placement.active
            && self.active.is_none()
            && self.drag.is_none()
            && self.line.draft.is_none()
            && !self.curve_draft_active()
            && !self.shape_draft_active()
            && self.fills.draft.is_none()
            && !self.liquify_active()
            && !self.partial_erase_active()
            && !self.local_color_active()
            && self.point_draft.is_none()
            && self.selection_cleanup_draft.is_none()
            && self.point_multi_draft.is_none()
            && self.reprojection_draft.is_none()
            && !self.mixed_resource_preview_active()
            && self.camera_playback.is_none()
            && self.repeat.is_none()
            && !self.palette_ui.sampling
            && !self.palette_ui.color_editor_active()
            && !self.preset_ui.browser_is_open()
            && !ctx.wants_keyboard_input()
            && !ctx.input(|input| input.pointer.any_down())
    }

    fn quick_palette_shortcuts(&mut self, ctx: &egui::Context) {
        let (escape, q, up, down, enter, modifiers, pointer, focused) = ctx.input(|input| {
            (
                input.key_pressed(Key::Escape),
                input.key_pressed(Key::Q),
                input.key_pressed(Key::ArrowUp),
                input.key_pressed(Key::ArrowDown),
                input.key_pressed(Key::Enter),
                input.modifiers,
                input.pointer.hover_pos(),
                input.focused,
            )
        });
        if self.quick_palette.is_some() {
            if !focused || escape {
                self.quick_palette = None;
                self.suppress_canvas_input_once = true;
                return;
            }
            let items = self.filtered_quick_palette_items();
            if items.is_empty() {
                return;
            }
            if let Some(state) = self.quick_palette.as_mut() {
                let direction = if up {
                    -1
                } else if down {
                    1
                } else {
                    0
                };
                if direction != 0 {
                    for offset in 1..=items.len() {
                        let index = (state.selected as isize + direction * offset as isize)
                            .rem_euclid(items.len() as isize)
                            as usize;
                        if items[index].enabled {
                            state.selected = index;
                            break;
                        }
                    }
                }
            }
            if enter {
                let selected = self
                    .quick_palette
                    .as_ref()
                    .and_then(|state| items.get(state.selected))
                    .filter(|item| item.enabled)
                    .map(|item| item.command);
                self.quick_palette = None;
                self.suppress_canvas_input_once = true;
                if let Some(command) = selected {
                    self.run_quick_palette_command(command);
                }
            }
        } else if q && !modifiers.any() && self.can_open_quick_palette(ctx) {
            let screen = ctx.screen_rect();
            let position = pointer.unwrap_or_else(|| screen.center());
            let menu_size = egui::vec2(300.0, 410.0);
            let position = egui::pos2(
                position.x.clamp(
                    screen.left() + 8.0,
                    (screen.right() - menu_size.x - 8.0).max(screen.left() + 8.0),
                ),
                position.y.clamp(
                    screen.top() + 8.0,
                    (screen.bottom() - menu_size.y - 8.0).max(screen.top() + 8.0),
                ),
            );
            let items = self.quick_palette_items();
            let active_tool = self.quick_palette_active_tool();
            let selected = items
                .iter()
                .position(|item| item.group == "Tools" && item.label == active_tool)
                .unwrap_or(0);
            self.quick_palette = Some(QuickPaletteState {
                position,
                selected,
                query: String::new(),
                search_focus_pending: true,
            });
        }
    }

    fn run_quick_palette_command(&mut self, command: QuickPaletteCommand) {
        match command {
            QuickPaletteCommand::Tool(tool) => self.choose_tool(tool),
            QuickPaletteCommand::Undo => self.undo(),
            QuickPaletteCommand::Redo => self.redo(),
            QuickPaletteCommand::Shape(mode) => {
                self.cancel_curve_tool();
                self.start_shape_tool(mode);
            }
            QuickPaletteCommand::Transform(mode) => {
                self.finish();
                self.transform_mode = mode;
            }
            QuickPaletteCommand::Deselect => {
                self.selected.clear();
                self.sync_selection();
            }
        }
    }

    fn quick_palette_ui(&mut self, ctx: &egui::Context) {
        let Some(state) = self.quick_palette.as_ref() else {
            return;
        };
        let position = state.position;
        let selected = state.selected;
        let mut query = state.query.clone();
        let focus_search = state.search_focus_pending;
        let items = self.filtered_quick_palette_items();
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::new("quick-tool-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(220.0);
                    ui.label(RichText::new("Quick palette").strong());
                    ui.small("Search tools and actions · ↑/↓ choose · Enter run · Esc close");
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text("Search by name or shortcut")
                            .desired_width(ui.available_width()),
                    );
                    if focus_search {
                        search.request_focus();
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height((ui.ctx().screen_rect().height() - 140.0).max(140.0))
                        .show(ui, |ui| {
                            if items.is_empty() {
                                ui.weak("No matching commands.");
                            } else {
                                let mut group = "";
                                for (index, item) in items.iter().enumerate() {
                                    if item.group != group {
                                        ui.add_space(3.0);
                                        ui.label(
                                            RichText::new(item.group.to_uppercase())
                                                .small()
                                                .color(ui_style::colors(ui.ctx()).muted_text),
                                        );
                                        group = item.group;
                                    }
                                    let text = if item.shortcut.is_empty() {
                                        item.label.to_owned()
                                    } else {
                                        format!("{}    {}", item.label, item.shortcut)
                                    };
                                    let response = ui
                                        .add_enabled(
                                            item.enabled,
                                            egui::Button::new(text).selected(index == selected),
                                        )
                                        .on_hover_text(item.label);
                                    if index == selected {
                                        ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
                                    }
                                    if response.clicked() {
                                        chosen = Some(item.command);
                                    }
                                }
                            }
                        });
                });
            });
        let pointer = ctx.input(|input| {
            input
                .pointer
                .button_pressed(PointerButton::Primary)
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        let clicked_outside =
            pointer.is_some_and(|position| !area.response.rect.contains(position));
        if let Some(state) = self.quick_palette.as_mut() {
            state.search_focus_pending = false;
            if state.query != query {
                state.query = query;
                state.selected = items.iter().position(|item| item.enabled).unwrap_or(0);
            }
            if !items.is_empty() {
                state.selected = state.selected.min(items.len() - 1);
            }
        }
        if chosen.is_some() || clicked_outside {
            self.quick_palette = None;
        }
        if let Some(command) = chosen {
            self.run_quick_palette_command(command);
        }
    }

    fn selection_canvas_controls(&mut self, ctx: &egui::Context, rect: Rect) {
        if self.presentation.artwork || self.quick_palette.is_some() {
            return;
        }
        let has_selection = self.select_mode && !self.selected.is_empty();
        if !has_selection
            || self.drag.is_some()
            || self.brush_select_drag.is_some()
            || self.point_drag.is_some()
            || self.point_multi_drag.is_some()
            || self.active.is_some()
        {
            return;
        }
        let position = egui::pos2(rect.center().x, rect.top() + 10.0);
        let compact = rect.width() < 430.0;
        let mut chosen_mode = None;
        let mut duplicate = false;
        let mut deselect = false;
        egui::Area::new(egui::Id::new("selection-canvas-strip"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .pivot(egui::Align2::CENTER_TOP)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if compact {
                                ui.menu_button("Selection", |ui| {
                                    for (mode, label) in [
                                        (Mode::Move, "Move"),
                                        (Mode::Rotate, "Rotate"),
                                        (Mode::Scale, "Scale"),
                                    ] {
                                        if ui
                                            .selectable_label(self.transform_mode == mode, label)
                                            .clicked()
                                        {
                                            chosen_mode = Some(mode);
                                            ui.close_menu();
                                        }
                                    }
                                    ui.separator();
                                    if ui.button("Duplicate").clicked() {
                                        duplicate = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Deselect").clicked() {
                                        deselect = true;
                                        ui.close_menu();
                                    }
                                });
                            } else {
                                for (mode, label) in [
                                    (Mode::Move, "Move"),
                                    (Mode::Rotate, "Rotate"),
                                    (Mode::Scale, "Scale"),
                                ] {
                                    if ui
                                        .selectable_label(self.transform_mode == mode, label)
                                        .clicked()
                                    {
                                        chosen_mode = Some(mode);
                                    }
                                }
                                ui.separator();
                                if ui.button("Duplicate").on_hover_text("Ctrl/Cmd+D").clicked() {
                                    duplicate = true;
                                }
                                if ui.button("Deselect").clicked() {
                                    deselect = true;
                                }
                            }
                        });
                    });
            });
        if let Some(mode) = chosen_mode {
            self.finish();
            self.transform_mode = mode;
        }
        if duplicate {
            self.duplicate_selected();
        }
        if deselect {
            self.selected.clear();
            self.sync_selection();
        }
    }

    fn canvas_controls(&mut self, ctx: &egui::Context, rect: Rect) {
        if self.presentation.artwork || self.quick_palette.is_some() {
            return;
        }
        let drawing_brush = !self.eraser
            && !self.select_mode
            && !self.local_color_brush.enabled
            && !self.fills.enabled
            && !self.measure.enabled
            && self.shape_tool.mode.is_none()
            && !self.curve_tool.enabled;
        if !drawing_brush {
            return;
        }
        if !self.presentation.preferences.layout.canvas_strip_visible {
            return;
        }
        let position = egui::pos2(rect.center().x, rect.top() + 10.0);
        let mut hide = false;
        let saved_guides: Vec<_> = self
            .doc
            .guide_resources
            .iter()
            .map(|guide| (guide.id, guide.name.clone(), guide.visible && !guide.locked))
            .collect();
        egui::Area::new(egui::Id::new("canvas-controls-strip"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .pivot(egui::Align2::CENTER_TOP)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        let available = ui.ctx().screen_rect().width().min(rect.width()) - 24.0;
                        let compact = available < 680.0;
                        ui.set_max_width(available.max(180.0));
                        ui.horizontal(|ui| {
                            if !compact {
                                ui.label(RichText::new("Brush").strong());
                            }
                            let radius_width = if compact {
                                (available * 0.40).clamp(72.0, 170.0)
                            } else {
                                (available * 0.34).clamp(115.0, 190.0)
                            };
                            ui.add_sized(
                                [radius_width, ui.spacing().interact_size.y],
                                egui::Slider::new(&mut self.radius, 0.001..=1.0)
                                    .logarithmic(true)
                                    .show_value(true)
                                    .custom_formatter(|value, _| format!("{value:.3}")),
                            )
                            .on_hover_text("Brush radius in world units");
                            self.palette_ui
                                .color_button(ui, ColorTarget::Brush, &self.color);
                            let guide_label = if compact {
                                "Guide".to_owned()
                            } else {
                                format!("Guide · {}", self.active_guide_label())
                            };
                            ui.menu_button(guide_label, |ui| {
                                if ui.button("Pick on canvas  K").clicked() {
                                    self.begin_plane_picker();
                                    ui.close_menu();
                                }
                                ui.separator();
                                ui.checkbox(&mut self.guide_visible, "Show guide");
                                ui.separator();
                                for (plane, label) in [
                                    (Plane::Front, "Front"),
                                    (Plane::Side, "Side"),
                                    (Plane::Ground, "Ground"),
                                ] {
                                    if ui.button(label).clicked() {
                                        self.finish();
                                        self.doc.active_guide = None;
                                        self.doc.active_image_plane = None;
                                        self.doc.active_model_reference = None;
                                        for image in &mut self.doc.image_planes {
                                            image.drawable = false;
                                        }
                                        self.use_cylinder = false;
                                        self.use_revolved = false;
                                        self.guide = DrawingPlane::from_preset(plane);
                                        self.guide_visible = true;
                                        ui.close_menu();
                                    }
                                }
                                if !saved_guides.is_empty() {
                                    ui.separator();
                                    for (id, name, enabled) in &saved_guides {
                                        if ui
                                            .add_enabled(*enabled, egui::Button::new(name))
                                            .clicked()
                                        {
                                            self.finish();
                                            self.doc.active_guide = Some(*id);
                                            self.doc.active_image_plane = None;
                                            self.doc.active_model_reference = None;
                                            for image in &mut self.doc.image_planes {
                                                image.drawable = false;
                                            }
                                            self.use_cylinder = false;
                                            self.use_revolved = false;
                                            self.guide_visible = true;
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                            if !compact {
                                ui.add_sized(
                                    [150.0, ui.spacing().interact_size.y],
                                    egui::Slider::new(&mut self.stabilization, 0.0..=1.0)
                                        .text("Stabilize")
                                        .custom_formatter(|value, _| {
                                            format!("{:.0}%", value * 100.0)
                                        }),
                                );
                            }
                            ui.menu_button(if compact { "More" } else { "⋯" }, |ui| {
                                if compact {
                                    ui.add(
                                        egui::Slider::new(&mut self.stabilization, 0.0..=1.0)
                                            .text("Stabilize")
                                            .custom_formatter(|value, _| {
                                                format!("{:.0}%", value * 100.0)
                                            }),
                                    );
                                    ui.separator();
                                }
                                if ui.button("Hide canvas strip").clicked() {
                                    hide = true;
                                    ui.close_menu();
                                }
                            });
                        });
                    });
            });
        if hide {
            self.presentation.preferences.layout.canvas_strip_visible = false;
            self.presentation.schedule_workspace_save();
        }
    }

    fn tool_rail(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("tool-rail-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(6.0);
                let active_tool = if self.select_mode {
                    "Select"
                } else if self.local_color_brush.enabled {
                    "Tint"
                } else if self.eraser {
                    "Erase"
                } else if self.fills.enabled {
                    "Fill"
                } else if self.line.enabled {
                    "Line"
                } else if self.measure.enabled {
                    "Measure"
                } else if self.curve_tool.enabled {
                    "Curve"
                } else if let Some(mode) = self.shape_tool.mode {
                    mode.label()
                } else {
                    "Brush"
                };
                ui.label(
                    RichText::new("TOOLS")
                        .small()
                        .color(ui_style::colors(ui.ctx()).muted_text),
                );
                ui.label(RichText::new(format!("Active: {active_tool}")).strong());
                ui.add_space(4.0);
                let selected = [
                    !self.eraser
                        && !self.local_color_brush.enabled
                        && !self.select_mode
                        && !self.fills.enabled
                        && !self.line.enabled
                        && !self.measure.enabled
                        && !self.curve_tool.enabled
                        && self.shape_tool.mode.is_none(),
                    self.line.enabled && !self.select_mode,
                    self.measure.enabled && !self.select_mode,
                    self.curve_tool.enabled && !self.select_mode,
                    self.shape_tool.mode.is_some(),
                    self.fills.enabled && !self.select_mode,
                    self.eraser && !self.select_mode,
                    self.local_color_brush.enabled && !self.select_mode,
                    self.select_mode,
                ];
                let tools = [
                    ("Brush", "Brush", "B", "brush"),
                    ("Line", "Line", "L", "line"),
                    ("Measure", "Measure", "M", "measure"),
                    ("Curve", "Curve", "C", "curve"),
                    ("Circle", "Shape", "", "shape"),
                    ("Fill", "Fill", "P", "fill"),
                    ("Erase", "Erase", "E", "erase"),
                    ("Paint", "Tint", "T", "tint"),
                    ("Select", "Select", "V", "select"),
                ];
                egui::Grid::new("compact-tool-grid")
                    .num_columns(2)
                    .spacing(Vec2::new(6.0, 6.0))
                    .show(ui, |ui| {
                        for (index, (command, name, shortcut, icon)) in
                            tools.into_iter().enumerate()
                        {
                            let suffix = if shortcut.is_empty() {
                                "Choose Circle, Ellipse, Rectangle, or Rounded rectangle from the contextual options".to_owned()
                            } else {
                                format!("Shortcut: {shortcut}")
                            };
                            if tool_icon_button(ui, icon, name, &suffix, selected[index]).clicked()
                            {
                                self.choose_tool(command);
                            }
                            if index % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(6.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 32.0],
                        egui::Button::new("Artwork [Tab]"),
                    )
                    .clicked()
                {
                    self.toggle_artwork();
                }
            });
    }

    fn contextual_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(mode) = self.shape_tool.mode {
                ui.label(RichText::new(mode.label()).strong());
                ui.separator();
                for (shape, label) in [
                    (shape_tool::ShapeMode::Circle, "Circle"),
                    (shape_tool::ShapeMode::Ellipse, "Ellipse"),
                    (shape_tool::ShapeMode::Rectangle, "Rectangle"),
                    (shape_tool::ShapeMode::RoundedRectangle, "Rounded rectangle"),
                ] {
                    if ui
                        .selectable_value(&mut self.shape_tool.mode, Some(shape), label)
                        .changed()
                    {
                        self.cancel_shape_draft();
                        self.shape_tool.mode = Some(shape);
                    }
                }
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
                let editable_shape = self.selected.len() == 1
                    && self.doc.strokes.iter().any(|stroke| {
                        self.selected.contains(&stroke.id)
                            && self.doc.editable(stroke)
                            && stroke
                                .curve
                                .as_ref()
                                .is_some_and(|curve| curve.shape.is_some() && curve.is_valid())
                    });
                if ui
                    .add_enabled(editable_shape, egui::Button::new("Edit shape…"))
                    .clicked()
                {
                    self.open_selected_curved_shape();
                }
                let editable_points = self.selected.len() == 1
                    && self.doc.strokes.iter().any(|stroke| {
                        self.selected.contains(&stroke.id)
                            && self.doc.editable(stroke)
                            && stroke.fill.is_none()
                            && stroke.curve.is_none()
                            && !self.doc.has_linked_source(stroke.id)
                            && self.doc.linked_instance_for_stroke(stroke.id).is_none()
                            && (2..=crate::point_edit::MAX_CANVAS_POINTS)
                                .contains(&stroke.points.len())
                    });
                if ui
                    .add_enabled(editable_points, egui::Button::new("Edit points…"))
                    .clicked()
                {
                    self.begin_canvas_point_edit();
                }
                let editable_fill = self.selected.len() == 1
                    && self.doc.strokes.iter().any(|stroke| {
                        self.selected.contains(&stroke.id)
                            && stroke.fill.is_some()
                            && self.doc.editable(stroke)
                    });
                if ui
                    .add_enabled(editable_fill, egui::Button::new("Edit fill contour…"))
                    .clicked()
                {
                    self.begin_fill_contour_edit(false);
                }
                let refreshable_fill = editable_fill && self.doc.strokes.iter().any(|stroke| {
                    self.selected.contains(&stroke.id) && stroke.fill.as_ref().is_some_and(|fill| {
                        fill.region_source.is_some() && fill.holes.is_empty()
                    })
                });
                if ui.add_enabled(refreshable_fill, egui::Button::new("Refresh enclosed fill…"))
                    .on_disabled_hover_text("Requires an enclosed fill saved with its seed, without cutouts or temporary closing guides.")
                    .clicked()
                {
                    self.begin_fill_contour_edit(true);
                }
            } else if self.fills.enabled {
                ui.label(RichText::new("Planar fill").strong());
                ui.label("Click enclosed regions on the active plane");
            } else if self.curve_tool.enabled {
                ui.label(RichText::new("Smooth curve").strong());
                ui.label("Click to add anchors; drag to adjust tangents; Enter to finish");
            } else if self.line.enabled {
                ui.label(RichText::new("Straight line").strong());
                ui.label("Drag from start to end; release to commit");
            } else if self.measure.enabled {
                ui.label(RichText::new("Measure").strong());
            } else {
                let tool_label = if self.eraser {
                    "Eraser"
                } else if self.local_color_brush.enabled {
                    "Tint brush"
                } else {
                    "Brush"
                };
                ui.label(RichText::new(tool_label).strong());
                if !self.eraser && !self.local_color_brush.enabled {
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
                } else if self.local_color_brush.enabled {
                    self.palette_ui
                        .color_button(ui, ColorTarget::Brush, &self.color);
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
        let common_brush_settings = !self.select_mode
            && !self.fills.enabled
            && !self.measure.enabled
            && self.shape_tool.mode.is_none()
            && !self.curve_tool.enabled
            && !self.local_color_brush.enabled
            && !self.eraser;
        if common_brush_settings {
            self.palette_ui.controls(ui, &mut self.color);
            self.tutorial_hint(ui, "Brush library");
            self.preset_controls(ui);
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
            ui.add_space(8.0);
        }
        self.snapping.controls(ui);
        self.line_controls(ui);
        self.measure_controls(ui);
        self.curve_controls(ui);
        self.fill_controls(ui);
        self.shape_controls(ui);
        self.mirror_controls(ui);
        ui.add_space(6.0);
        if self.select_mode {
            self.group_controls(ui);
            self.mixed_resource_controls(ui);
        }
        if !common_brush_settings {
            self.palette_ui.controls(ui, &mut self.color);
            self.tutorial_hint(ui, "Brush library");
            if !self.fills.enabled && self.shape_tool.mode.is_none() && !self.curve_tool.enabled {
                self.preset_controls(ui);
            }
        }
        if self.select_mode {
            self.tutorial_hint(ui, "Select");
            self.selection_panel(ui);
        } else if !self.fills.enabled && self.shape_tool.mode.is_none() && !self.curve_tool.enabled
        {
            if self.local_color_brush.enabled {
                self.local_color_controls(ui);
            } else if self.eraser {
                self.partial_eraser_controls(ui);
            } else {
                self.tutorial_hint(ui, "Brush");
                if !self.fills.enabled {
                    let brush_summary = if self.brush.tip_asset_id != 0 {
                        "Image tip"
                    } else if self.brush.flow.enabled {
                        "Airbrush"
                    } else if self.brush.strands.enabled {
                        "Bristle / rake"
                    } else {
                        match self.brush.kind {
                            BrushKind::Round => "Round",
                            BrushKind::Ribbon => "Ribbon",
                            BrushKind::Chisel => "Flat / chisel",
                        }
                    };
                    egui::CollapsingHeader::new(format!("Brush details · {brush_summary}"))
                        .id_salt("properties-brush-details")
                        .default_open(false)
                        .show(ui, |ui| self.brush_controls(ui));
                }
            }
        }
        ui.add_space(8.0);
        egui::CollapsingHeader::new(format!("Drawing guide · {}", self.active_guide_label()))
            .id_salt("properties-drawing-guide")
            .default_open(true)
            .show(ui, |ui| {
                self.tutorial_hint(ui, "Drawing guide");
                self.plane_controls(ui);
                if ui.button("Reset camera").clicked() {
                    self.finish();
                    self.camera = Camera::default();
                }
            });
        ui.add_space(8.0);
        ui.collapsing("Navigation and help", |ui| {
            ui.label("Space + drag    Orbit\nSpace + Shift    Pan\nSpace + Alt       Zoom\nRight drag        Orbit\nMiddle drag      Pan\nScroll                Zoom");
            self.tutorial_hint(ui, "Navigation");
            if ui.button("All shortcuts / help").clicked() {
                self.finish();
                self.show_help = true;
            }
        });
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
        ui.label("Tablet setup");
        ui.label("Windows: enable Windows Ink in the tablet driver. Linux X11: XInput2. Linux Wayland: compositor tablet-v2. macOS: AppKit.");
        ui.label("Native pen setup errors appear above; mouse drawing remains available. Linux/macOS runtime and physical tablet checks remain open (SKETCH-006/008).");
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
                                    self.clear_live_line_art_cache();
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
            let next_group_id = self.doc.next_group_resource_id();
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
            if ui.button("Import planar SVG…").clicked() {
                ui.close_menu();
                self.import_planar_svg();
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
                if ui
                    .add_enabled(
                        self.can_export(false),
                        egui::Button::new("Active plane as SVG (.svg)…"),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.active_plane_svg_export_dialog();
                }
                if ui.button("Editable Grease Pencil (.py)…").clicked() {
                    ui.close_menu();
                    self.grease_pencil_handoff_dialog();
                }
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
            ui.menu_button("Share", |ui| {
                if ui.button("Publish whole drawing · ink only…").clicked() {
                    ui.close_menu();
                    self.start_share_preview(false, false, ctx);
                }
                if ui
                    .button("Publish whole scene · ink + images + OBJ…")
                    .clicked()
                {
                    ui.close_menu();
                    self.start_share_preview(false, true, ctx);
                }
                if ui.button("Publish selected strokes…").clicked() {
                    ui.close_menu();
                    self.start_share_preview(true, false, ctx);
                }
                ui.separator();
                if ui.button("Manage shared links…").clicked() {
                    ui.close_menu();
                    self.share_ui.open_manager();
                }
            });
            ui.menu_button("Examples", |ui| {
                for (label, pending) in [
                    ("Pencil and pastel house", Pending::PencilHouse),
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
            if ui.checkbox(&mut self.presentation.preferences.pen_barrel_erases, "Hold pen button to erase")
                .on_hover_text("Hold the barrel button before touching the canvas to erase part of a stroke. When off, the button orbits.")
                .changed()
            {
                self.cancel();
                self.presentation.schedule_workspace_save();
            }
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
            if ui.button("Document color swatches…").clicked() {
                ui.close_menu();
                self.finish();
                self.swatches_ui.open = true;
            }
            if ui.button("Camera and guide controls").clicked() {
                ui.close_menu();
                self.finish();
                self.show_view = !self.show_view;
            }
            if ui
                .checkbox(
                    &mut self.presentation.preferences.layout.canvas_strip_visible,
                    "Canvas controls strip",
                )
                .changed()
            {
                self.presentation.schedule_workspace_save();
            }
            if ui.button("Artwork-only view   Tab").clicked() {
                ui.close_menu();
                self.toggle_artwork();
            }
            if ui.button("Pick a drawing plane   K").clicked() {
                ui.close_menu();
                self.begin_placement();
            }
            if ui.button("Pick saved guide on canvas   Shift+K").clicked() {
                ui.close_menu();
                self.begin_plane_picker();
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
            let open = !self.presentation.preferences.layout.inspector_open;
            let label = if open {
                "Show inspector"
            } else {
                "Hide inspector"
            };
            let icon = if open { "sidebar" } else { "hide" };
            if inspector_menu_action_button(ui, icon, label).clicked() {
                ui.close_menu();
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
            RichText::new("Sketch3D")
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

    fn import_planar_svg(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("SVG artwork", &["svg"])
            .pick_file()
        else {
            return;
        };
        let Some(target) = self.planar_drawing_target() else {
            self.status = "Activate a planar guide before importing SVG artwork.".into();
            return;
        };
        if !self.doc.active_editable() {
            self.status = "Choose a visible unlocked layer before importing SVG artwork.".into();
            return;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = format!("SVG import failed: {error}");
                return;
            }
        };
        let artwork = match crate::svg_import::parse(&bytes) {
            Ok(artwork) => artwork,
            Err(error) => {
                self.status = format!("SVG import failed: {error}");
                return;
            }
        };
        self.finish();
        self.cancel_line_tool();
        self.cancel_curve_tool();
        self.cancel_shape_draft();
        self.cancel_fill_tool();
        let before_id = self.next_id;
        let strokes = match crate::svg_import::to_strokes(
            &artwork,
            target.plane,
            self.doc.active_layer,
            before_id,
        ) {
            Ok(strokes) => strokes,
            Err(error) => {
                self.status = format!("SVG import failed: {error}");
                return;
            }
        };
        let Some(next_id) = before_id.checked_add(strokes.len() as u64) else {
            self.status = "No more stroke IDs are available".into();
            return;
        };
        if next_id == u64::MAX || strokes.is_empty() {
            self.status = "SVG import produced no artwork or exhausted stroke IDs".into();
            return;
        }
        self.doc.strokes.extend(strokes.iter().cloned());
        self.undo.push(Edit::AddBatch(strokes.clone()));
        self.redo.clear();
        self.next_id = next_id;
        self.dirty = true;
        self.record_recovery_edit_labeled("Import SVG artwork");
        self.renderer.invalidate();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("SVG");
        self.status = if artwork.skipped > 0 {
            format!("Imported {} editable strokes from {name}; skipped {} unsupported SVG elements or effects.", strokes.len(), artwork.skipped)
        } else {
            format!(
                "Imported {} editable strokes from {name} onto the active plane.",
                strokes.len()
            )
        };
    }

    fn inspector(&mut self, ctx: &egui::Context, enabled: bool) {
        if !self.presentation.preferences.layout.inspector_open || self.presentation.artwork {
            return;
        }
        if self.presentation.preferences.layout.inspector_collapsed {
            egui::SidePanel::right("inspector")
                .exact_width(64.0)
                .show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.vertical_centered(|ui| {
                        for (tab, glyph, label) in [
                            (
                                crate::presentation::InspectorTab::Properties,
                                "properties",
                                "Properties",
                            ),
                            (
                                crate::presentation::InspectorTab::Layers,
                                "layers",
                                "Layers",
                            ),
                            (
                                crate::presentation::InspectorTab::References,
                                "references",
                                "References",
                            ),
                            (
                                crate::presentation::InspectorTab::ImagePlanes,
                                "images",
                                "Images",
                            ),
                        ] {
                            let response = inspector_rail_button(
                                ui,
                                glyph,
                                label,
                                self.presentation.preferences.layout.inspector_tab == tab,
                                true,
                                None,
                            );
                            if response.clicked()
                                && self.presentation.preferences.layout.inspector_tab != tab
                            {
                                self.presentation.preferences.layout.inspector_tab = tab;
                                self.presentation.schedule_workspace_save();
                            }
                        }
                        ui.separator();
                        if inspector_rail_button(
                            ui,
                            "expand",
                            "Expand inspector",
                            false,
                            true,
                            None,
                        )
                        .clicked()
                        {
                            self.set_inspector_collapsed(false);
                        }
                        let can_hide = self.point_draft.is_none()
                            && self.point_multi_draft.is_none()
                            && self.reprojection_draft.is_none();
                        let hide = inspector_rail_button(
                            ui,
                            "hide",
                            "Hide inspector",
                            false,
                            can_hide,
                            (!can_hide).then_some("Finish the active edit before hiding"),
                        );
                        if hide.clicked() {
                            self.set_inspector_open(false);
                        }
                    });
                });
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
                        self.presentation.schedule_workspace_save();
                    }
                    let can_change_visibility = self.point_draft.is_none()
                        && self.point_multi_draft.is_none()
                        && self.reprojection_draft.is_none();
                    if inspector_rail_button(
                        ui,
                        "collapse",
                        "Collapse inspector to its tab rail",
                        false,
                        can_change_visibility,
                        None,
                    )
                    .clicked()
                    {
                        self.set_inspector_collapsed(true);
                    }
                    if inspector_rail_button(
                        ui,
                        "hide",
                        "Hide inspector",
                        false,
                        can_change_visibility,
                        None,
                    )
                    .clicked()
                    {
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
                        if self.selection_cleanup_draft.is_some() {
                            self.selection_cleanup_controls(ui);
                        } else if self.point_draft.is_some() {
                            self.canvas_point_controls(ui);
                        } else if self.point_multi_draft.is_some() {
                            self.multi_point_controls(ui);
                        } else if self.reprojection_draft.is_some() {
                            self.reprojection_controls(ui);
                        } else if self.mixed_resource_preview_active()
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
            self.presentation.schedule_workspace_save();
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
        self.presentation.schedule_workspace_save();
    }

    fn set_inspector_collapsed(&mut self, collapsed: bool) {
        self.presentation.preferences.layout.inspector_collapsed = collapsed;
        self.presentation.schedule_workspace_save();
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
    fn pointer_action(&mut self, pos: Pos2, _pressure: f32, rect: Rect, begin: bool, shift: bool) {
        if self.selection_cleanup_draft.is_some() {
            return;
        }
        self.pointer_action_with_tilt(pos, None, rect, begin, shift, None);
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
        pressure: Option<f32>,
        rect: Rect,
        begin: bool,
        shift: bool,
        tilt: Option<Vec3>,
    ) {
        if self.selection_cleanup_draft.is_some() {
            return;
        }
        if self.plane_picker.active {
            if self.presentation.artwork {
                self.cancel_plane_picker();
                return;
            }
            if begin {
                self.pick_saved_plane_at(pos, rect);
                self.mouse_down = false;
            }
            return;
        }
        let pressure_samples_available = pressure.is_some();
        let pressure = pressure.unwrap_or(1.0);
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
        if self
            .fills
            .draft
            .as_ref()
            .is_some_and(|draft| draft.original.is_some())
        {
            self.fill_contour_pointer(pos, rect, begin);
            return;
        }
        if self.measure.enabled {
            self.measure_input(pos, rect, begin);
            return;
        }
        if self.placement.stamp.is_some() {
            self.stamp_pointer_action(pos, rect, begin);
            return;
        }
        if self.placement.active {
            if begin {
                self.pick_placement(pos, rect);
            }
        } else if self.select_mode {
            self.select_pointer(pos, rect, begin, shift);
        } else if self.local_color_brush.enabled {
            self.local_color_input(pos, rect, begin);
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
            self.add_point_with_tilt_source(
                pos,
                pressure,
                pressure_samples_available,
                rect,
                begin,
                tilt,
            );
        }
    }
    fn canvas_input(&mut self, ctx: &egui::Context, rect: Rect, mut pen: Vec<PenSample>) {
        self.sync_canvas_point_edit();
        if self.video_export.is_running()
            || self.quick_palette.is_some()
            || std::mem::take(&mut self.suppress_canvas_input_once)
        {
            self.pen_barrel_contact = None;
            self.cancel_partial_erase();
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
        if self.guide_bend_draft.is_some() {
            self.guide_bend_canvas_input(ctx, rect, &events, &pen);
            return;
        }
        self.snapping.options.bypass = alt;
        if alt {
            self.snapping.cue = None;
        }
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
            && self.point_verification_frame.is_none()
            && self.house_verification_frame.is_none()
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
        let continue_active_contact = self.active.is_some();
        let inside_canvas = |p| canvas_contact_allowed(ctx, rect, p, continue_active_contact);
        self.update_plane_picker_hover(
            pointer
                .hover_pos()
                .filter(|position| inside_canvas(*position)),
            rect,
        );
        if self.measure.enabled {
            if let Some(position) = pointer
                .hover_pos()
                .filter(|position| inside_canvas(*position))
            {
                self.measure_input(position, rect, false);
            }
            if ctx.input(|input| input.key_pressed(Key::Escape)) {
                self.cancel_measure_tool();
                self.status = "Measurement cancelled.".into();
            }
        }
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
            if self.measure.enabled && inside && sample.phase == Phase::Move {
                self.measure_input(sample.position, rect, false);
            }
            if self.plane_picker.active && sample.phase == Phase::Move && !sample.contact {
                self.update_plane_picker_hover(inside.then_some(sample.position), rect);
            }
            if blocked {
                self.pen_barrel_contact = None;
                self.cancel_brush_selection();
                self.cancel_partial_erase();
                self.cancel_local_color();
                self.cancel_shape_draft();
                self.finish();
                self.mouse_down = false;
                self.pen_previous = None;
                self.cancel_transform();
                continue;
            }
            let barrel_erases = self.presentation.preferences.pen_barrel_erases;
            if barrel_erases {
                if sample.phase == Phase::Down {
                    self.pen_barrel_contact = Some(sample.barrel);
                } else if sample.phase == Phase::Move
                    && sample.contact
                    && self.pen_barrel_contact != Some(sample.barrel)
                {
                    if self.pen_barrel_contact.is_some() {
                        self.cancel();
                    }
                    continue;
                }
                let contact = if sample.phase == Phase::Up {
                    self.pen_barrel_contact.take()
                } else {
                    self.pen_barrel_contact
                };
                if contact == Some(true) {
                    if sample.phase == Phase::Up {
                        if inside {
                            self.partial_erase_input(sample.position, rect, false);
                        }
                        self.finish_partial_erase(true);
                    } else if space
                        || self.presentation.artwork
                        || self.plane_picker.active
                        || self.placement.active
                        || self.placement.stamp.is_some()
                        || self.liquify_active()
                        || self.point_draft.is_some()
                    {
                        self.cancel_partial_erase();
                    } else if inside && sample.contact {
                        self.partial_erase_input(
                            sample.position,
                            rect,
                            sample.phase == Phase::Down,
                        );
                    }
                    continue;
                }
                if sample.phase == Phase::Up && contact.is_none() {
                    continue;
                }
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
                if self.point_multi_drag.is_some() {
                    self.multi_point_pointer(sample.position, rect, false);
                    self.finish_multi_point_region();
                    continue;
                }
                if self.region_select_drag.is_some() {
                    self.select_pointer(sample.position, rect, false, shift);
                    self.commit_region_selection();
                    continue;
                }
                if self
                    .fills
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.original.is_some())
                {
                    self.fill_contour_pointer(sample.position, rect, false);
                    if let Some(draft) = &mut self.fills.draft {
                        draft.drag = None;
                        draft.gradient_drag = None;
                    }
                    continue;
                }
                if self.point_draft.is_some() {
                    self.canvas_point_pointer(sample.position, rect, false);
                    self.point_drag = None;
                    continue;
                }
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
                if self.local_color_active() {
                    self.local_color_input(sample.position, rect, false);
                    self.finish_local_color(true);
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
            if !sample.contact
                && sample.phase == Phase::Move
                && self.doc.strokes.last().is_some_and(|stroke| {
                    stroke.brush.flow.enabled && Some(stroke.id) == self.active
                })
            {
                self.finish();
                continue;
            }
            if !inside
                && self.drag.is_none()
                && self.point_drag.is_none()
                && self.point_multi_drag.is_none()
                && !self
                    .fills
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.drag.is_some())
                && self.region_select_drag.is_none()
                && self.brush_select_drag.is_none()
                && !self.partial_erase_active()
                && !self.local_color_active()
                && !self.shape_draft_active()
            {
                self.finish();
                continue;
            }
            if space || (sample.barrel && !barrel_erases) {
                if sample.contact {
                    if self
                        .doc
                        .strokes
                        .last()
                        .is_some_and(|stroke| stroke.brush.flow.enabled)
                    {
                        self.finish();
                    }
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
                        sample.pressure,
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
            self.cancel_local_color();
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
                if self
                    .doc
                    .strokes
                    .last()
                    .is_some_and(|stroke| stroke.brush.flow.enabled)
                {
                    self.finish();
                }
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
                                    || self.point_drag.is_some()
                                    || self.point_multi_drag.is_some()
                                    || self
                                        .fills
                                        .draft
                                        .as_ref()
                                        .is_some_and(|draft| draft.drag.is_some())
                                    || self.region_select_drag.is_some()
                                    || self.brush_select_drag.is_some()
                                    || self.partial_erase_active()
                                    || self.local_color_active()
                                    || self.shape_draft_active())
                                    && !rect.contains(pos))
                            {
                                self.pointer_action(pos, 1.0, rect, false, shift);
                            } else {
                                self.mouse_down = false;
                                self.cancel_brush_selection();
                                self.cancel_partial_erase();
                                self.cancel_local_color();
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
                            if self.point_multi_drag.is_some() {
                                self.multi_point_pointer(pos, rect, false);
                                self.finish_multi_point_region();
                                self.mouse_down = false;
                                continue;
                            }
                            if self.point_draft.is_some() {
                                self.canvas_point_pointer(pos, rect, false);
                                self.point_drag = None;
                                self.mouse_down = false;
                                continue;
                            }
                            if self
                                .fills
                                .draft
                                .as_ref()
                                .is_some_and(|draft| draft.original.is_some())
                            {
                                self.fill_contour_pointer(pos, rect, false);
                                if let Some(draft) = &mut self.fills.draft {
                                    draft.drag = None;
                                    draft.gradient_drag = None;
                                }
                                self.mouse_down = false;
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
                            } else if self.local_color_active() {
                                self.local_color_input(pos, rect, false);
                                self.finish_local_color(true);
                            } else if self.shape_draft_active() {
                                if inside_canvas(pos) {
                                    self.release_shape(pos, 1.0, rect);
                                } else {
                                    self.cancel_shape_draft();
                                }
                            } else if self.drag.is_some()
                                || self.brush_select_drag.is_some()
                                || self.region_select_drag.is_some()
                            {
                                self.select_pointer(pos, rect, false, shift);
                            }
                            self.commit_region_selection();
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
                            self.cancel_multi_point_region();
                            self.cancel_region_selection();
                            self.cancel_partial_erase();
                            self.cancel_local_color();
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
        self.tick_airbrush_contact(ctx);
        self.sync_selection();
    }
}

fn strand_vertex_budget_exceeded(stroke: &Stroke) -> bool {
    stroke.brush.strands.enabled
        && stroke.brush.strands.vertex_estimate(stroke.points.len())
            > crate::geometry::MAX_STRAND_VERTICES
}

fn airbrush_vertex_budget_exceeded(stroke: &Stroke) -> bool {
    stroke.brush.flow.enabled
        && (stroke.points.len() > crate::geometry::MAX_AIRBRUSH_POINTS
            || crate::geometry::airbrush_vertex_estimate(stroke)
                > crate::geometry::MAX_AIRBRUSH_VERTICES)
}

fn profile_tube_vertex_budget_exceeded(stroke: &Stroke) -> bool {
    stroke.brush.kind == BrushKind::Round
        && stroke.brush.tube_profile != crate::document::TubeProfile::Round
        && !stroke.brush.flow.enabled
        && !stroke.brush.strands.enabled
        && !stroke.brush.dabs.enabled
        && stroke.brush.tip_asset_id == 0
        && crate::geometry::profile_tube_vertex_estimate(stroke)
            > crate::geometry::MAX_PROFILE_TUBE_VERTICES
}

impl SketchApp {
    fn service_live_mcp(&mut self, ctx: &egui::Context) {
        let Some(requests) = self.live_mcp_requests.take() else {
            return;
        };
        for _ in 0..8 {
            let request = match requests.try_recv() {
                Ok(request) => request,
                Err(std::sync::mpsc::TryRecvError::Empty)
                | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            };
            match request.command {
                crate::live_mcp::LiveUiCommand::Snapshot => {
                    let revision = match self.live_mcp_revision() {
                        Ok(revision) => revision,
                        Err(error) => {
                            let _ = request.reply.send(Err(error));
                            continue;
                        }
                    };
                    let mut selected: Vec<_> = self.selected.iter().copied().collect();
                    selected.sort_unstable();
                    let file_name =
                        self.file
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| {
                                name.to_string_lossy()
                                    .chars()
                                    .filter(|character| !character.is_control())
                                    .collect::<String>()
                            });
                    let snapshot = serde_json::json!({
                        "document": {
                            "identity": file_name.unwrap_or_else(|| "untitled".into()),
                            "saved": self.file.is_some(),
                            "unsaved_changes": self.dirty,
                            "strokes": self.doc.strokes.len(),
                            "layers": self.doc.layers.len(),
                            "guides": self.doc.guide_resources.len(),
                            "image_planes": self.doc.image_planes.len()
                        },
                        "selection": {"stroke_ids": selected},
                        "live_edit_preview": self.live_mcp_preview.is_some(),
                        "revision": revision
                    });
                    let _ = request.reply.send(Ok(snapshot));
                }
                crate::live_mcp::LiveUiCommand::SceneOutline { cursor, page_size } => {
                    let result = self.live_mcp_scene_outline(cursor.as_deref(), page_size);
                    let _ = request.reply.send(result);
                }
                crate::live_mcp::LiveUiCommand::StrokeIndex {
                    layer_id,
                    cursor,
                    page_size,
                } => {
                    let result = self.live_mcp_stroke_index(layer_id, cursor.as_deref(), page_size);
                    let _ = request.reply.send(result);
                }
                crate::live_mcp::LiveUiCommand::CaptureViewport => {
                    if self.live_mcp_capture_reply.is_some() {
                        let _ = request
                            .reply
                            .send(Err("A viewport capture is already in progress".into()));
                    } else {
                        self.live_mcp_capture_reply = Some(request.reply);
                        ctx.send_viewport_cmd(
                            egui::ViewportCommand::Screenshot(Default::default()),
                        );
                        ctx.request_repaint();
                    }
                }
                crate::live_mcp::LiveUiCommand::PreviewColor { revision, color } => {
                    let result = self.preview_live_mcp_color(revision, color);
                    let _ = request.reply.send(result);
                }
                crate::live_mcp::LiveUiCommand::PreviewMove {
                    revision,
                    translation,
                } => {
                    let result = self.preview_live_mcp_move(revision, translation);
                    let _ = request.reply.send(result);
                }
                crate::live_mcp::LiveUiCommand::PreviewSelection {
                    revision,
                    stroke_ids,
                } => {
                    let result = self.preview_live_mcp_selection(revision, stroke_ids);
                    let _ = request.reply.send(result);
                }
                crate::live_mcp::LiveUiCommand::ApplyEdit {
                    preview_id,
                    revision,
                } => {
                    let result = self.apply_live_mcp_edit(&preview_id, &revision);
                    let _ = request.reply.send(result);
                }
                crate::live_mcp::LiveUiCommand::CancelEdit { preview_id } => {
                    let result = self.cancel_live_mcp_edit(&preview_id);
                    let _ = request.reply.send(result);
                }
            }
        }
        self.live_mcp_requests = Some(requests);
    }

    fn live_mcp_revision(&self) -> Result<String, String> {
        let bytes = serde_json::to_vec(&self.doc)
            .map_err(|error| format!("Could not encode document revision: {error}"))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("Live document revision exceeds the 16 MiB bound".into());
        }
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        if let Some(file) = &self.file {
            for byte in file.to_string_lossy().bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        let mut selected: Vec<_> = self.selected.iter().copied().collect();
        selected.sort_unstable();
        for id in selected {
            for byte in id.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        hash ^= u64::from(self.dirty);
        hash = hash.wrapping_mul(0x100000001b3);
        Ok(format!("fnv1a64:{hash:016x}"))
    }

    fn live_mcp_scene_outline(
        &self,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<serde_json::Value, String> {
        const MAX_NAME_CHARS: usize = 96;
        let revision = self.live_mcp_revision()?;
        let offset = if let Some(cursor) = cursor {
            let Some((cursor_revision, raw_offset)) = cursor.split_once('/') else {
                return Err("Invalid scene outline cursor; request the first page again".into());
            };
            if cursor_revision != revision {
                return Err(
                    "Scene changed; this cursor is stale. Request the first page again".into(),
                );
            }
            raw_offset
                .parse::<usize>()
                .ok()
                .filter(|offset| *offset <= self.doc.layers.len() + self.doc.guide_resources.len())
                .ok_or_else(|| {
                    "Invalid scene outline cursor; request the first page again".to_owned()
                })?
        } else {
            0
        };

        let safe_name = |name: &str| -> String {
            name.chars()
                .filter(|character| !character.is_control())
                .take(MAX_NAME_CHARS)
                .collect()
        };
        let mut items = Vec::with_capacity(self.doc.layers.len() + self.doc.guide_resources.len());
        for layer in &self.doc.layers {
            items.push(serde_json::json!({
                "kind": "layer",
                "id": layer.id,
                "name": safe_name(&layer.name),
                "active": layer.id == self.doc.active_layer,
                "visible": layer.visible && self.doc.layer_folder_visible(layer.id),
                "locked": layer.locked || self.doc.layer_folder_locked(layer.id)
            }));
        }
        for guide in &self.doc.guide_resources {
            let guide_type = match &guide.geometry {
                crate::document::guide_resources::GuideGeometry::Plane { .. } => "plane",
                crate::document::guide_resources::GuideGeometry::Cylinder { .. } => "cylinder",
                crate::document::guide_resources::GuideGeometry::Revolved { .. } => "revolved",
                crate::document::guide_resources::GuideGeometry::Sphere { .. } => "sphere",
                crate::document::guide_resources::GuideGeometry::Ring { .. } => "ring",
                crate::document::guide_resources::GuideGeometry::Cone { .. } => "cone",
                crate::document::guide_resources::GuideGeometry::Freeform { .. } => "freeform",
                crate::document::guide_resources::GuideGeometry::Loft { .. } => "loft",
            };
            items.push(serde_json::json!({
                "kind": "guide",
                "id": guide.id,
                "name": safe_name(&guide.name),
                "guide_type": guide_type,
                "active": self.doc.active_guide == Some(guide.id),
                "visible": guide.visible,
                "locked": guide.locked
            }));
        }
        let end = offset.saturating_add(page_size).min(items.len());
        let next_cursor = (end < items.len()).then(|| format!("{revision}/{end}"));
        Ok(serde_json::json!({
            "revision": revision,
            "active_layer_id": self.doc.active_layer,
            "active_guide_id": self.doc.active_guide,
            "items": items[offset..end],
            "next_cursor": next_cursor
        }))
    }

    fn live_mcp_stroke_index(
        &self,
        layer_id: u64,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<serde_json::Value, String> {
        let revision = self.live_mcp_revision()?;
        if !self.doc.layers.iter().any(|layer| layer.id == layer_id) {
            return Err("The requested layer does not exist".into());
        }
        let prefix = format!("{revision}/{layer_id}/");
        let after = cursor
            .map(|cursor| {
                cursor
                    .strip_prefix(&prefix)
                    .and_then(|id| id.parse::<u64>().ok())
                    .ok_or("Invalid or stale stroke cursor. Request the first page for this layer.")
            })
            .transpose()?;
        let mut strokes: Vec<_> = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| stroke.layer_id == layer_id)
            .collect();
        strokes.sort_unstable_by_key(|stroke| stroke.id);
        let start = after.map_or(0, |id| strokes.partition_point(|stroke| stroke.id <= id));
        let end = (start + page_size).min(strokes.len());
        let generated = self.doc.linked_generated_ids();
        let items: Vec<_> = strokes[start..end]
            .iter()
            .map(|stroke| {
                let kind = if stroke.fill.is_some() {
                    "fill"
                } else if stroke.curve.is_some() {
                    "curve"
                } else if stroke.model_surface.is_some() {
                    "model_surface"
                } else {
                    "sampled"
                };
                let can_color = self.doc.editable(stroke) && !generated.contains(&stroke.id);
                let bounds = crate::editing::stroke_bounds(stroke).map(
                    |(min, max)| serde_json::json!({"min":min.to_array(),"max":max.to_array()}),
                );
                serde_json::json!({
                    "id":stroke.id, "kind":kind, "world_bounds":bounds,
                    "can_edit_color":can_color,
                    "can_move":can_color && self.live_mcp_stroke_move_supported(stroke)
                })
            })
            .collect();
        let next_cursor = (end < strokes.len()).then(|| format!("{prefix}{}", strokes[end - 1].id));
        Ok(serde_json::json!({
            "revision": revision, "layer_id": layer_id, "items": items, "next_cursor": next_cursor
        }))
    }

    fn live_mcp_stroke_move_supported(&self, stroke: &Stroke) -> bool {
        stroke.fill.is_none()
            && stroke.guide_attachment.is_none()
            && !self.doc.has_linked_source(stroke.id)
    }

    fn live_mcp_gesture_active(&self) -> bool {
        self.active.is_some()
            || self.animation_ui.playing
            || self.mouse_down
            || self.drag.is_some()
            || self.brush_select_drag.is_some()
            || self.region_select_drag.is_some()
            || self.point_drag.is_some()
            || self.point_multi_drag.is_some()
            || self.partial_erase_active()
            || self.appearance_draft.is_some()
            || self.alignment_preview.is_some()
            || self.point_draft.is_some()
            || self.point_multi_draft.is_some()
            || self.selection_cleanup_draft.is_some()
            || self.reprojection_draft.is_some()
            || self.fills.draft.is_some()
            || self.curve_draft_active()
            || self.shape_draft_active()
            || self.line.draft.is_some()
            || self.line_art_draft.is_some()
            || self.line_art_command.is_some()
            || self.guide_bend_draft.is_some()
            || self.loft_preview.is_some()
            || self.camera_playback.is_some()
            || self.placement.active
            || self.repeat.is_some()
            || self.insert_preview.is_some()
            || self.liquify_active()
            || self.mixed_resource_preview_active()
            || self.pending.is_some()
            || self.confirm
            || self.recovery_prompt
            || self.quick_palette.is_some()
            || self.show_help
    }

    fn check_live_mcp_preview(&self, revision: &str) -> Result<(), String> {
        if self.live_mcp_preview.is_some() {
            return Err("A live MCP edit preview is already active".into());
        }
        if self.live_mcp_gesture_active() {
            return Err(
                "Finish or cancel the active edit before requesting a live MCP preview".into(),
            );
        }
        if self.live_mcp_revision()? != revision {
            return Err(
                "The document or selection changed; request a fresh live_document_snapshot".into(),
            );
        }
        Ok(())
    }

    fn preview_live_mcp_color(
        &mut self,
        revision: String,
        color: [f32; 3],
    ) -> Result<serde_json::Value, String> {
        self.check_live_mcp_preview(&revision)?;
        let edit = crate::editing::appearance_edit(
            &self.doc,
            &self.selected,
            crate::editing::AppearancePatch {
                color: Some(color),
                ..Default::default()
            },
        )?;
        if let Edit::Change { before, after } = &edit {
            if before == after {
                return Err("The selected strokes already have that color".into());
            }
        }
        let mut result = self.stage_live_mcp_edit(revision, edit, "a color change")?;
        result["color"] = serde_json::json!(color);
        Ok(result)
    }

    fn preview_live_mcp_move(
        &mut self,
        revision: String,
        translation: [f32; 3],
    ) -> Result<serde_json::Value, String> {
        self.check_live_mcp_preview(&revision)?;
        let before = crate::editing::snapshot(&self.doc, &self.selected);
        if before.is_empty()
            || before.len() != self.selected.len()
            || before
                .iter()
                .any(|(_, stroke)| !self.live_mcp_stroke_move_supported(stroke))
        {
            return Err("Select editable, independent strokes to move".into());
        }
        let after = crate::editing::transformed_with_scale(
            &before,
            Vec3::ZERO,
            Vec3::from_array(translation),
            Quat::IDENTITY,
            Vec3::ONE,
            Quat::IDENTITY,
            false,
        )?;
        if before == after {
            return Err("The translation does not change the selected strokes".into());
        }
        let mut result =
            self.stage_live_mcp_edit(revision, Edit::Change { before, after }, "a move")?;
        result["translation"] = serde_json::json!(translation);
        Ok(result)
    }

    fn stage_live_mcp_edit(
        &mut self,
        revision: String,
        edit: Edit,
        description: &'static str,
    ) -> Result<serde_json::Value, String> {
        let selected_ids = self.selected.clone();
        edit.apply(&mut self.doc, true);
        self.renderer
            .invalidate_ids(&self.doc.linked_affected_stroke_ids(&selected_ids));
        let preview_revision = match self.live_mcp_revision() {
            Ok(revision) => revision,
            Err(error) => {
                edit.apply(&mut self.doc, false);
                self.renderer
                    .invalidate_ids(&self.doc.linked_affected_stroke_ids(&selected_ids));
                return Err(error);
            }
        };
        Ok(self.register_live_mcp_preview(
            description,
            revision,
            preview_revision,
            selected_ids,
            LiveMcpPreviewAction::Artwork(Box::new(edit)),
        ))
    }

    fn preview_live_mcp_selection(
        &mut self,
        revision: String,
        stroke_ids: Vec<u64>,
    ) -> Result<serde_json::Value, String> {
        self.check_live_mcp_preview(&revision)?;
        let selected_ids: HashSet<_> = stroke_ids.iter().copied().collect();
        if selected_ids.len() != stroke_ids.len() {
            return Err("Stroke IDs must be unique".into());
        }
        let generated = self.doc.linked_generated_ids();
        let editable_count = self
            .doc
            .strokes
            .iter()
            .filter(|stroke| {
                selected_ids.contains(&stroke.id)
                    && self.doc.editable(stroke)
                    && !generated.contains(&stroke.id)
            })
            .count();
        if editable_count != selected_ids.len() {
            return Err(
                "Missing, hidden, protected, or linked instance strokes cannot be selected".into(),
            );
        }
        let mut result = self.register_live_mcp_preview(
            "a replacement selection",
            revision.clone(),
            revision,
            selected_ids,
            LiveMcpPreviewAction::Selection,
        );
        result["stroke_ids"] = serde_json::json!(stroke_ids);
        Ok(result)
    }

    fn register_live_mcp_preview(
        &mut self,
        description: &'static str,
        revision: String,
        preview_revision: String,
        selected_ids: HashSet<u64>,
        action: LiveMcpPreviewAction,
    ) -> serde_json::Value {
        let preview_id = crate::live_mcp::preview_id();
        let selected_count = selected_ids.len();
        self.live_mcp_preview = Some(LiveMcpPreview {
            description,
            preview_id: preview_id.clone(),
            base_revision: revision.clone(),
            preview_revision: preview_revision.clone(),
            selected_ids,
            action,
        });
        self.status =
            "MCP edit preview active. Apply or cancel it from the MCP client or preview panel."
                .into();
        serde_json::json!({
            "preview_id": preview_id,
            "revision": revision,
            "preview_revision": preview_revision,
            "selected_count": selected_count,
            "preview_active": true
        })
    }

    fn apply_live_mcp_edit(
        &mut self,
        preview_id: &str,
        revision: &str,
    ) -> Result<serde_json::Value, String> {
        let preview = self
            .live_mcp_preview
            .as_ref()
            .ok_or_else(|| "No live MCP edit preview is active".to_owned())?;
        if preview.preview_id != preview_id || preview.base_revision != revision {
            return Err("Preview ID or source revision does not match the active preview".into());
        }
        if self.live_mcp_revision()? != preview.preview_revision {
            return Err("The document changed after preview; the edit was not applied".into());
        }
        let preview = self.live_mcp_preview.take().expect("preview checked above");
        let count = preview.selected_ids.len();
        let LiveMcpPreviewAction::Artwork(edit) = preview.action else {
            self.selected = preview.selected_ids;
            self.status = format!("MCP selected {count} strokes.");
            return Ok(serde_json::json!({
                "applied":true,"selected_count":count,"undo_steps":0,"revision":self.live_mcp_revision()?
            }));
        };
        self.undo.push(*edit);
        self.redo.clear();
        self.dirty = true;
        self.record_recovery_edit();
        self.renderer
            .invalidate_ids(&self.doc.linked_affected_stroke_ids(&preview.selected_ids));
        self.status = "MCP edit applied as one undoable change.".into();
        Ok(serde_json::json!({"applied":true,"selected_count":count,"undo_steps":1}))
    }

    fn cancel_live_mcp_edit(&mut self, preview_id: &str) -> Result<serde_json::Value, String> {
        let preview = self
            .live_mcp_preview
            .as_ref()
            .ok_or_else(|| "No live MCP edit preview is active".to_owned())?;
        if preview.preview_id != preview_id {
            return Err("Preview ID does not match the active preview".into());
        }
        if self.live_mcp_revision()? != preview.preview_revision {
            return Err(
                "The document changed after preview; cancel was refused to protect the newer edit"
                    .into(),
            );
        }
        let preview = self.live_mcp_preview.take().expect("preview checked above");
        if let LiveMcpPreviewAction::Artwork(edit) = preview.action {
            edit.apply(&mut self.doc, false);
            self.renderer
                .invalidate_ids(&self.doc.linked_affected_stroke_ids(&preview.selected_ids));
        }
        self.status = "MCP edit cancelled. Source values restored.".into();
        Ok(serde_json::json!({"cancelled":true,"document_unchanged":true}))
    }

    fn show_live_mcp_preview(&mut self, ctx: &egui::Context) {
        let Some(preview) = self.live_mcp_preview.as_ref() else {
            return;
        };
        let preview_id = preview.preview_id.clone();
        let base_revision = preview.base_revision.clone();
        let selected_count = preview.selected_ids.len();
        let description = preview.description;
        let response = egui::Modal::new(egui::Id::new("live-mcp-edit-preview")).show(ctx, |ui| {
            ui.heading("MCP edit preview");
            ui.label(format!(
                "Previewing {description} on {selected_count} selected strokes."
            ));
            ui.label(
                "The drawing is temporarily locked until this preview is applied or cancelled.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let apply = ui.button("Apply").clicked();
                let cancel = ui.button("Cancel").clicked();
                (apply, cancel)
            })
            .inner
        });
        let (apply, cancel) = response.inner;
        if apply {
            let _ = self.apply_live_mcp_edit(&preview_id, &base_revision);
        } else if cancel {
            let _ = self.cancel_live_mcp_edit(&preview_id);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    fn finish_live_mcp_capture(&mut self, ctx: &egui::Context) {
        if self.live_mcp_capture_reply.is_none() {
            return;
        }
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = screenshot else {
            return;
        };
        let result = crate::live_mcp::encode_viewport_png(&image);
        let Some(reply) = self.live_mcp_capture_reply.take() else {
            return;
        };
        let _ = reply.send(result);
    }
}

impl eframe::App for SketchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let start = Instant::now();
        self.service_live_mcp(ctx);
        self.finish_live_mcp_capture(ctx);
        if let Some(preview) = self.live_mcp_preview.as_ref() {
            if ctx.input(|input| input.viewport().close_requested()) {
                let preview_id = preview.preview_id.clone();
                let _ = self.cancel_live_mcp_edit(&preview_id);
            }
        }
        let live_mcp_edit_locked = self.live_mcp_preview.is_some();
        if !live_mcp_edit_locked {
            self.canvas_point_shortcuts(ctx);
        }
        if let Some(verification) = &self.appearance_layout_verification {
            ctx.set_zoom_factor(verification.zoom_factor);
        }
        self.presentation.apply_theme(ctx);
        self.presentation.poll_layout_save();
        self.poll_recovery(ctx);
        if self.house_verification_frame.is_some() {
            self.recovery_prompt = false;
        }
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
        if !live_mcp_edit_locked {
            self.tick_artwork_animation(ctx);
        }
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
        self.share_ui.poll(ctx, &mut self.status);
        if self
            .capability_replay
            .as_ref()
            .is_some_and(|replay| replay.clip.is_reference_export())
            && matches!(self.last_export_result.as_ref(), Some(Ok(_)))
        {
            self.status =
                "Whole-scene GLB export completed; file details are in the recording manifest."
                    .into();
        }
        let completed_reference_export = self
            .capability_replay
            .as_ref()
            .filter(|replay| {
                replay.clip.is_reference_export()
                    && replay.reference_export_action_started
                    && replay.reference_export_action.is_none()
            })
            .and_then(|replay| {
                let path = replay.reference_export_action_path.as_ref()?;
                let summary = self.last_export_result.as_ref()?.as_ref().ok()?.clone();
                let bytes = std::fs::metadata(path).ok()?.len();
                Some((path.clone(), summary, bytes))
            });
        if let Some((path, summary, bytes)) = completed_reference_export {
            if let Some(replay) = self.capability_replay.as_mut() {
                replay.reference_export_action = Some(serde_json::json!({
                    "file": path
                        .strip_prefix(&replay.output_dir)
                        .unwrap_or(&path)
                        .to_string_lossy(),
                    "bytes": bytes,
                    "summary": summary,
                }));
            }
        }
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
        if !live_mcp_edit_locked {
            if ctx.input(|i| i.viewport().close_requested()) {
                self.leave_artwork();
            }
            if self.palette_ui.sampling && ctx.input(|input| input.key_pressed(Key::Escape)) {
                self.cancel_color_sampling();
            }
            if self.quick_palette.is_some() {
                self.quick_palette_shortcuts(ctx);
            } else {
                self.plane_picker_shortcuts(ctx);
                if self.loft_preview.is_some() && ctx.input(|input| input.key_pressed(Key::Escape))
                {
                    self.loft_preview = None;
                    self.renderer.invalidate();
                    self.status = "Loft preview cancelled.".into();
                }
                self.partial_eraser_shortcuts(ctx);
                self.stamp_shortcuts(ctx);
                self.local_color_shortcuts(ctx);
                self.shape_shortcuts(ctx);
                self.quick_palette_shortcuts(ctx);
            }
            self.tick_camera_sequence(ctx);
            if self.liquify_active() && ctx.input(|input| input.key_pressed(Key::Escape)) {
                self.finish_liquify(false);
            }
            if self.mixed_resource_preview_active()
                && ctx.input(|input| input.key_pressed(Key::Escape))
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
        }
        let was_artwork = self.presentation.artwork;
        if !self.confirm
            && !live_mcp_edit_locked
            && self.quick_palette.is_none()
            && !self.show_help
            && !self.recovery_prompt
            && !self.tutorial.offer
            && !self.close_after_recovery
            && !self.liquify_active()
            && self.point_draft.is_none()
            && self.point_multi_draft.is_none()
            && self.reprojection_draft.is_none()
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
            && !live_mcp_edit_locked
            && self.quick_palette.is_none()
            && !self.show_help
            && !self.recovery_prompt
            && !self.tutorial.offer
            && !self.close_after_recovery
            && !self.liquify_active()
            && self.point_draft.is_none()
            && self.point_multi_draft.is_none()
            && self.reprojection_draft.is_none()
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
            && !live_mcp_edit_locked
            && self.quick_palette.is_none()
            && !self.placement.active
            && !was_placing
            && !self.presentation.artwork
            && !was_artwork
            && !self.show_help
            && !self.tutorial.offer
            && !self.recovery_prompt
            && !self.close_after_recovery
            && !self.liquify_active()
            && self.point_draft.is_none()
            && self.point_multi_draft.is_none()
            && self.reprojection_draft.is_none()
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
            let (mods, b, e, f, i_key, t_key, m_key, z, y, s, o, n) = ctx.input(|i| {
                (
                    i.modifiers,
                    i.key_pressed(Key::B),
                    i.key_pressed(Key::E),
                    i.key_pressed(Key::F),
                    i.key_pressed(Key::I),
                    i.key_pressed(Key::T),
                    i.key_pressed(Key::M),
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
                if m_key {
                    self.choose_tool("Measure");
                }
                if b {
                    self.cancel_line_tool();
                    self.cancel_fill_tool();
                    self.finish();
                    self.eraser = false;
                    self.select_mode = false;
                    self.local_color_brush.enabled = false;
                }
                if e {
                    self.cancel_line_tool();
                    self.cancel_fill_tool();
                    self.finish();
                    self.eraser = true;
                    self.select_mode = false;
                    self.local_color_brush.enabled = false;
                }
                if t_key {
                    self.choose_tool("Paint");
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
                    ui.add_enabled_ui(
                        self.point_draft.is_none()
                            && self.point_multi_draft.is_none()
                            && self.reprojection_draft.is_none(),
                        |ui| {
                            egui::menu::bar(ui, |ui| self.menu_bar(ui, ctx));
                        },
                    );
                });
            });
            egui::TopBottomPanel::top("context-options").show(ctx, |ui| {
                ui.add_enabled_ui(
                    enabled
                        && self.point_draft.is_none()
                        && self.point_multi_draft.is_none()
                        && self.reprojection_draft.is_none(),
                    |ui| self.contextual_options(ui),
                );
            });
            self.artwork_animation_timeline(ctx);
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
                    ui.add_enabled_ui(
                        enabled
                            && self.point_draft.is_none()
                            && self.point_multi_draft.is_none()
                            && self.reprojection_draft.is_none(),
                        |ui| self.tool_rail(ui),
                    );
                });
            self.inspector(ctx, enabled || self.mixed_resource_preview_active());
        } else {
            self.artwork_bar(ctx);
        }
        self.camera_bookmark_controls(ctx);
        self.show_obj_reference_window(ctx);
        self.repeat_modal(ctx);
        self.png_export.show(ctx);
        let share_scope_enabled = [
            self.can_export_scope(false, false),
            self.can_export_scope(false, true),
            self.can_export_scope(true, false),
        ];
        self.share_ui.show(
            ctx,
            &self.doc,
            &self.selected,
            &self.image_asset_root,
            share_scope_enabled,
        );
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
        let show_overview_export_options = self.capability_replay.as_ref().is_some_and(|replay| {
            replay.clip.is_overview()
                && (51..CAPABILITY_REPLAY_FRAMES).contains(&replay.captured_frames)
        });
        if show_overview_export_options {
            egui::Window::new("Export options")
                .id(egui::Id::new("capability-overview-export-options"))
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18.0, 82.0))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Choose what to export as GLB:");
                    if let Some(action) = export_ui::glb_export_scope_items(
                        ui,
                        self.can_export(false),
                        self.can_export_scene(),
                        self.can_export(true),
                    )
                    .action
                    {
                        let (selected, include_images) = action.scope();
                        self.export_dialog_with_images(selected, include_images, ctx);
                    }
                });
        }
        self.obj_export.show(ctx);
        let pen = self.pen.as_mut().map(|p| p.drain()).unwrap_or_default();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                self.last_canvas_rect = rect;
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                self.canvas_controls(ctx, rect);
                self.selection_canvas_controls(ctx, rect);
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
                self.verify_guide_bend(ctx, rect);
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
                        ) && self.doc.strokes.is_empty(),
                        "curved-guide replay setup starts with an empty saved cylinder"
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
                    replay.setup_verified = stroke_count == 0
                        && self.mirror_ui.enabled
                        && self.line.enabled
                        && self.doc.strokes.is_empty();
                    assert!(
                        replay.setup_verified,
                        "mirror-shapes replay starts with an enabled mirror and line tool"
                    );
                }
                let reference_export_output = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| {
                        replay.clip.is_reference_export() && !replay.setup_verified
                    })
                    .map(|replay| replay.output_dir.clone());
                if let Some(output_dir) = reference_export_output {
                    let artifact = self
                        .prepare_capability_reference_export(rect, &output_dir)
                        .unwrap_or_else(|error| {
                            eprintln!("Could not prepare reference-export demo: {error}");
                            std::process::exit(1);
                        });
                    let artifact_value = serde_json::to_value(&artifact)
                        .expect("reference-export artifact serializes into its manifest");
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.initial_stroke_count = self.doc.strokes.len();
                    replay.initial_camera = self.camera;
                    replay.reference_export_artifact = Some(artifact_value);
                    replay.setup_verified = !self.doc.strokes.is_empty()
                        && self.doc.image_planes.iter().any(|plane| plane.visible)
                        && self.doc.model_references.iter().any(|model| model.visible);
                    assert!(
                        replay.setup_verified,
                        "reference-export setup keeps inserted ink, image plane, and OBJ reference visible"
                    );
                }
                let prepare_edit_erase = self
                    .capability_replay
                    .as_ref()
                    .is_some_and(|replay| {
                        replay.clip.is_edit_erase() && !replay.setup_verified
                    });
                if prepare_edit_erase {
                    let (selection_point, gesture_start, gesture_end) =
                        self.prepare_capability_edit_erase(rect);
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.initial_stroke_count = self.doc.strokes.len();
                    replay.initial_camera = self.camera;
                    replay.gesture_start = Some(gesture_start);
                    replay.gesture_end = Some(gesture_end);
                    replay.edit_selection_point = Some(selection_point);
                    replay.setup_verified = self.doc.strokes.len() == 1
                        && self.selected.is_empty()
                        && self.undo.is_empty()
                        && !self.eraser;
                    assert!(
                        replay.setup_verified,
                        "edit-erase setup creates one editable stroke for captured selection and erasing"
                    );
                }
                let undo_edit_erase = self.capability_replay.as_ref().is_some_and(|replay| {
                    replay.clip.is_edit_erase()
                        && replay.setup_verified
                        && replay.captured_frames == 40
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
                let import_reference_obj = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| replay.clip.is_reference_export())
                    .is_some_and(|replay| replay.captured_frames == 8 && !replay.reference_import_action);
                if import_reference_obj {
                    let screen_rect = ctx.input(|input| input.screen_rect);
                    self.verify_obj_reference_window_import_click(screen_rect);
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.reference_import_action = self.obj_reference_verification_button_clicked
                        && self.doc.model_references.len() == 2;
                    assert!(replay.reference_import_action, "capture clicks the production OBJ import control with its generated picker result");
                }
                if self
                    .capability_replay
                    .as_ref()
                    .is_some_and(|replay| replay.clip.is_reference_export() && replay.captured_frames == 12)
                {
                    self.obj_reference_window_open = false;
                }
                let insert_reference_lantern = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| replay.clip.is_reference_export())
                    .is_some_and(|replay| replay.captured_frames == 20 && !replay.reference_insert_action);
                if insert_reference_lantern {
                    let inserted_strokes = self.insert_capability_lantern().unwrap_or_else(|error| {
                        eprintln!("Could not insert reference-export lantern during capture: {error}");
                        std::process::exit(1);
                    });
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.reference_inserted_stroke_count = inserted_strokes;
                    replay.reference_insert_action = inserted_strokes > 0;
                    assert!(replay.reference_insert_action, "capture inserts the lantern through the production native-merge handler");
                }
                let export_reference_scene = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| replay.clip.is_reference_export())
                    .is_some_and(|replay| replay.captured_frames == 32 && !replay.reference_export_action_started);
                if export_reference_scene {
                    let output_path = self
                        .capability_replay
                        .as_ref()
                        .unwrap()
                        .output_dir
                        .join("reference-export")
                        .join("captured-reference-export.glb");
                    self.start_export_with_images(output_path.clone(), false, true, ctx);
                    let replay = self.capability_replay.as_mut().unwrap();
                    replay.reference_export_action_path = Some(output_path);
                    replay.reference_export_action_started = self.export_job.is_some();
                    assert!(replay.reference_export_action_started, "whole-scene export handler starts during capture");
                }
                let edit_action_frame = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| replay.clip.is_edit_erase())
                    .and_then(|replay| {
                        [2, 8, 12]
                            .contains(&replay.captured_frames)
                            .then_some(replay.captured_frames)
                    })
                    .filter(|frame| {
                        self.capability_replay
                            .as_ref()
                            .is_some_and(|replay| replay.edit_processed_frame != Some(*frame))
                    });
                if let Some(frame) = edit_action_frame {
                    match frame {
                        2 => {
                            let point = crate::gizmo::project(
                                &self.camera,
                                rect,
                                self.doc.strokes[0].points[24].position,
                            )
                            .expect("edit-erase stroke selection point projects at capture time")
                            .0;
                            self.select_mode = true;
                            self.select_pointer(point, rect, true, false);
                            assert_eq!(self.selected.len(), 1, "capture selects the editable stroke");
                            self.status = "Stroke selected. Change its appearance, then erase a segment.".into();
                        }
                        8 => {
                            self.apply_capability_appearance_edit([0.92, 0.22, 0.045]);
                            let replay = self.capability_replay.as_mut().unwrap();
                            replay.expected_undo_strokes = Some(self.doc.strokes.clone());
                            assert_eq!(self.undo.len(), 1, "appearance edit is one history step");
                            self.status = "Stroke appearance changed. Erase the middle; undo restores it.".into();
                        }
                        12 => {
                            self.select_mode = false;
                            self.eraser = true;
                            self.partial_eraser.enabled = true;
                            self.partial_eraser.radius = 16.0;
                        }
                        _ => unreachable!(),
                    }
                    self.capability_replay
                        .as_mut()
                        .unwrap()
                        .edit_processed_frame = Some(frame);
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
                self.verify_house(ctx, rect);
                self.verify_canvas(ctx, rect);
                let start_mirror_ellipse = self
                    .capability_replay
                    .as_ref()
                    .is_some_and(|replay| {
                        replay.clip.is_mirror_shapes() && replay.captured_frames == 17
                    });
                if start_mirror_ellipse {
                    self.line.enabled = false;
                    self.start_shape_tool(shape_tool::ShapeMode::Ellipse);
                }
                let mirror_shape_frame = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| replay.clip.is_mirror_shapes())
                    .map(|replay| replay.captured_frames);
                if let Some(frame @ 17..=28) = mirror_shape_frame {
                    let [_, _, ellipse_start, ellipse_end] = self
                        .capability_replay
                        .as_ref()
                        .and_then(|replay| replay.mirror_points)
                        .expect("mirror-shapes replay projects ellipse points");
                    let t = (frame - 17) as f32 / 11.0;
                    let position = ellipse_start + (ellipse_end - ellipse_start) * t;
                    if frame == 17 {
                        self.shape_input(ellipse_start, 1.0, rect, true);
                    } else if frame == 28 {
                        self.release_shape(ellipse_end, 1.0, rect);
                    } else {
                        self.shape_input(position, 1.0, rect, false);
                    }
                }
                let curved_action = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| {
                        replay.clip.is_curved_guides()
                            && replay.curved_processed_frame != Some(replay.captured_frames)
                    })
                    .map(|replay| replay.captured_frames);
                if let Some(frame @ 4..=19) = curved_action {
                    let [start, middle, end] = self
                        .capability_replay
                        .as_ref()
                        .and_then(|replay| replay.curved_world_points)
                        .expect("curved-guide replay stores cylinder-surface points");
                    let surface_screen = |from: Vec3, to: Vec3, t: f32| {
                        let from_angle = from.x.atan2(from.z);
                        let to_angle = to.x.atan2(to.z);
                        let angle = from_angle + (to_angle - from_angle) * t;
                        let world = Vec3::new(
                            angle.sin(),
                            from.y + (to.y - from.y) * t,
                            angle.cos(),
                        );
                        crate::gizmo::project(&self.camera, rect, world)
                            .expect("interpolated cylinder point projects onto the canvas")
                            .0
                    };
                    if frame == 4 {
                        let point = crate::gizmo::project(&self.camera, rect, start)
                            .expect("curved stroke start projects")
                            .0;
                        self.pointer_action(point, 1.0, rect, true, false);
                    } else if frame <= 11 {
                        let t = (frame - 4) as f32 / 7.0;
                        self.pointer_action(surface_screen(start, middle, t), 1.0, rect, false, false);
                    } else if frame <= 18 {
                        let t = (frame - 11) as f32 / 7.0;
                        self.pointer_action(surface_screen(middle, end, t), 1.0, rect, false, false);
                    } else {
                        let point = crate::gizmo::project(&self.camera, rect, end)
                            .expect("curved stroke end projects")
                            .0;
                        self.pointer_action(point, 1.0, rect, false, false);
                        self.finish();
                    }
                    self.capability_replay
                        .as_mut()
                        .unwrap()
                        .curved_processed_frame = Some(frame);
                }
                let long_action = self
                    .capability_replay
                    .as_ref()
                    .filter(|replay| {
                        replay.clip.has_tutorial_edit_steps()
                            && replay.long_processed_frame != Some(replay.captured_frames)
                    })
                    .map(|replay| replay.captured_frames);
                if let Some(frame) = long_action {
                    match frame {
                        27 => {
                            if let Some(stroke) = self.doc.strokes.last() {
                                let point = stroke.points[stroke.points.len() / 2].position;
                                if let Some((screen, _)) = crate::gizmo::project(&self.camera, rect, point) {
                                    self.select_mode = true;
                                    self.select_pointer(screen, rect, true, false);
                                }
                            }
                        }
                        28 => {
                            if self.selected.is_empty() {
                                self.capability_replay.take().unwrap().abort();
                                eprintln!("Capability tutorial could not select its captured stroke through the production picker");
                                std::process::exit(1);
                            }
                            self.transform_mode = Mode::Move;
                            self.transform_values = [0.12, 0.0, 0.0];
                            self.numeric_transform();
                            self.capability_replay.as_mut().unwrap().long_edit_verified =
                                self.undo.len() >= 2;
                        }
                        29 => self.undo(),
                        30 => self.redo(),
                        51 if self
                            .capability_replay
                            .as_ref()
                            .is_some_and(|replay| replay.clip.is_dusk_house()) => {
                            let orbit_camera = self.camera;
                            self.camera = self
                                .capability_replay
                                .as_ref()
                                .unwrap()
                                .initial_camera;
                            let path = self
                                .capability_replay
                                .as_ref()
                                .unwrap()
                                .output_dir
                                .join("dusk-house-still.png");
                            self.start_png_export(
                                path,
                                [1920, 1080],
                                false,
                                png_export_ui::StillFormat::Png,
                                png_export_ui::JpegBackground::Canvas,
                                false,
                                ctx,
                            );
                            self.camera = orbit_camera;
                        }
                        52 if self
                            .capability_replay
                            .as_ref()
                            .is_some_and(|replay| replay.clip.is_long_form()) => {
                            if self
                                .capability_replay
                                .as_ref()
                                .is_some_and(|replay| replay.clip.is_dusk_house())
                            {
                                for guide in &mut self.doc.guide_resources {
                                    guide.visible = false;
                                }
                            }
                            let saved = self.save(true);
                            self.capability_replay.as_mut().unwrap().long_saved = saved;
                            if !saved {
                                self.capability_replay.take().unwrap().abort();
                                eprintln!("Capability tutorial failed to save its native document");
                                std::process::exit(1);
                            }
                            self.status = "Native tutorial copy saved.".into();
                        }
                        56 if self
                            .capability_replay
                            .as_ref()
                            .is_some_and(|replay| replay.clip.is_long_form()) => {
                            let path = self.file.clone().expect("tutorial save establishes a native path");
                            self.library_open_path = Some(path);
                            self.pending = Some(Pending::Open);
                            self.execute_pending();
                            self.capability_replay.as_mut().unwrap().long_reopened = true;
                            self.status = "Saved native tutorial copy reopened.".into();
                        }
                        57 if self
                            .capability_replay
                            .as_ref()
                            .is_some_and(|replay| replay.clip.is_dusk_house()) => {
                            self.select_mode = false;
                            self.camera = self
                                .capability_replay
                                .as_ref()
                                .unwrap()
                                .initial_camera;
                            if !self.presentation.artwork {
                                self.toggle_artwork();
                            }
                        }
                        _ => {}
                    }
                    self.capability_replay.as_mut().unwrap().long_processed_frame = Some(frame);
                }
                let replay_pen = self
                    .capability_replay
                    .as_mut()
                    .and_then(|replay| replay.next_sample(rect));
                if let Some(replay) = self.capability_replay.as_mut() {
                    if replay.clip.has_tutorial_edit_steps()
                        && (27..=30).contains(&replay.captured_frames)
                    {
                        replay.selecting = true;
                    }
                }
                let curved_drawing_frame = self.capability_replay.as_ref().is_some_and(|replay| {
                    replay.clip.is_curved_guides() && (4..=19).contains(&replay.captured_frames)
                });
                let replay_pen = if curved_drawing_frame {
                    None
                } else {
                    replay_pen
                };
                if !curved_drawing_frame && !live_mcp_edit_locked {
                    self.canvas_input(ctx, rect, replay_pen.map_or(pen, |sample| vec![sample]));
                }
                self.process_line_art_command(rect, ctx);
                self.refresh_live_line_art(rect, ctx, false);
                self.verify_canvas_point_workflow(ctx, rect);
                let scale = ctx.pixels_per_point();
                let selection_preview = self.live_mcp_preview.as_ref()
                    .filter(|preview| matches!(preview.action, LiveMcpPreviewAction::Selection));
                if let Some(preview) = selection_preview {
                    self.renderer.set_selection(&preview.selected_ids);
                } else if self.select_mode && !self.presentation.artwork {
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
                self.paint_line_art_preview(ui.painter(), rect);
                self.paint_animation_onion_skins(ui.painter(), rect);
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
                    } else {
                        let mut export_document = self.doc.clone();
                        let line_art_result = if !selected
                            && self
                                .doc
                                .model_references
                                .iter()
                                .any(|resource| resource.visible && resource.live_line_art.is_some())
                        {
                            self.append_live_line_art_to_export(&mut export_document, ctx)
                        } else {
                            Ok(0)
                        };
                        if let Err(error) = line_art_result {
                            self.status = format!("Cannot snapshot live model line art: {error}");
                        } else if let Err(error) = self.obj_export.start(
                            destination,
                            selected,
                            replace,
                            &export_document,
                            &self.selected,
                            ctx,
                        ) {
                            self.status = format!("Could not start OBJ export: {error}");
                        }
                    }
                }
                self.verify_png_export(ctx);
                if !self.presentation.artwork {
                    self.line_overlay(ui, rect);
                    self.measure_overlay(ui, rect);
                    self.curve_overlay(ui, rect);
                    self.mirror_overlay(ui, rect);
                    self.fill_overlay(ui, rect);
                    self.shape_overlay(ui, rect);
                    snapping::overlay(ui, self.snapping.cue, rect);
                    self.selection_overlay(ui, rect);
                    self.plane_overlay(ui, rect);
                    self.placement_overlay(ui, rect);
                    self.stamp_overlay(ui, rect);
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
                    if let Some(title) = if self.cylinder_example {
                        Some("Cylindrical lantern")
                    } else {
                        self.starter_scene
                    } {
                        ui.painter().text(
                            rect.min + egui::vec2(22.0, 76.0),
                            egui::Align2::LEFT_TOP,
                            title,
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
        self.show_stamp_placement(ctx);
        self.show_lighting(ctx);
        self.show_drawing_library(ctx);
        self.show_insert_preview(ctx);
        self.show_tutorial(ctx);
        if self.show_help {
            egui::Window::new("Drawing with a tablet").default_width(540.0).max_height((ctx.screen_rect().height()-80.0).max(200.0)).vscroll(true).show(ctx,|ui|{self.tutorial_help(ui);ui.separator();if ui.button("Close help").clicked(){self.show_help=false;}ui.label("V: select   Shift-click: toggle stroke selection\nG: move   R: rotate   S: uniform scale (in Select tool)\nDrag the visible handles; Escape cancels a transform.\nCtrl/Cmd+A: select all   Ctrl/Cmd+D: duplicate\nDelete / Backspace: delete selection\n\nB: brush   E: whole-mark eraser   P: planar fill   I: sample artwork color (Escape cancels)\nQ: quick palette for tools, Undo/Redo and contextual commands (Escape closes)\nK: pick plane   F: align view to guide   Shift+F: plane from view\nCtrl/Cmd+Z: undo   Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y: redo\nCtrl/Cmd+S: save   Ctrl/Cmd+Shift+S: save as\nCtrl/Cmd+O: open   Ctrl/Cmd+N: new\n\nSpace + pen drag: orbit\nSpace + Shift + pen drag: pan\nSpace + Alt + pen drag: zoom\nPen barrel + drag: orbit (when reported by Windows Ink)\nRight mouse drag: orbit   Middle drag: pan   Wheel: zoom\n\nUse Front / Side / Ground or rotate and position a custom plane.\nShift+F creates a plane through the camera focus; F looks straight at it.\nExisting strokes stay in world space when you switch planes.\nEnable Windows Ink in your Wacom / Huion driver for pressure.\nA mouse works too, with fixed pressure.\n\nExport menu: whole drawing or selected strokes as GLB.\nPrototype limits: no Wintab, additional curved guides. Recovery saves completed edits automatically; Save keeps your native drawing.\nSave your work using the Save button.");});
        }
        self.show_recovery(ctx);
        self.show_color_editor(ctx);
        self.show_document_color_swatches(ctx);
        self.quick_palette_ui(ctx);
        self.show_live_mcp_preview(ctx);
        if !live_mcp_edit_locked {
            self.tick_recovery(ctx);
        }
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
            || self.guide_bend_verification
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
        if !self.shape_draft_active() {
            return;
        }
        let (escape, enter) = ctx.input(|input| {
            (
                input.key_pressed(Key::Escape),
                input.key_pressed(Key::Enter),
            )
        });
        let editing = self.shape_edit_active();
        if escape {
            self.cancel_shape_draft();
        } else if enter && editing {
            self.commit_curved_shape_edit();
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
                self.verify_stroke_sculpt_workflow(rect);
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
            assert!(Self::mixed_transform_supported(
                crate::mixed_resources::TransformMode::Rotate
            ));
            assert!(Self::mixed_transform_supported(
                crate::mixed_resources::TransformMode::Scale
            ));
            let mut selection = self.mixed_resources_ui.selection.clone();
            selection.stroke_ids = self.selected.clone();
            let original_document = self.doc.clone();
            let document_json = |document: &Document| serde_json::to_value(document).unwrap();
            let original = crate::mixed_resources::MoveSnapshot::capture_transform(
                &self.doc,
                selection.clone(),
                crate::mixed_resources::TransformSpec::translate(Vec3::ZERO),
            )
            .unwrap()
            .before;
            let active_plane = self.doc.active_image_plane;
            let selection_center = crate::mixed_resources::selection_bounds(
                &self.doc,
                &selection,
                &self.obj_reference_models,
            )
            .map_or(Vec3::ZERO, |(min, max)| (min + max) * 0.5);

            self.begin_mixed_resource_move();
            assert!(self.mixed_resource_preview_active());
            self.set_mixed_resource_translation(Vec3::new(0.3, -0.2, 0.15));
            let preview = crate::mixed_resources::MoveSnapshot::capture_transform(
                &self.doc,
                selection.clone(),
                crate::mixed_resources::TransformSpec::translate(Vec3::ZERO),
            )
            .unwrap()
            .before;
            assert_ne!(preview, original, "All selected resources preview together");
            assert!(self.undo.is_empty(), "Preview does not enter history");
            self.finish_mixed_resource_move(false);
            assert_eq!(
                document_json(&self.doc),
                document_json(&original_document),
                "Translation cancel is exact"
            );

            let custom_pivot = Vec3::new(-0.4, 0.25, 0.6);
            for (pivot_mode, pivot) in [
                (selection::PivotMode::SelectionCenter, selection_center),
                (selection::PivotMode::WorldOrigin, Vec3::ZERO),
                (selection::PivotMode::Custom, custom_pivot),
            ] {
                self.transform_pivot = pivot_mode;
                self.custom_pivot = custom_pivot;
                self.begin_mixed_resource_move();
                self.set_mixed_resource_rotation_degrees(Vec3::new(0.0, 0.0, 45.0));
                let first_expected = crate::mixed_resources::MoveSnapshot::capture_transform(
                    &original_document,
                    selection.clone(),
                    crate::mixed_resources::TransformSpec::rotate(
                        pivot,
                        glam::Quat::from_rotation_z(45.0_f32.to_radians()),
                    ),
                )
                .unwrap()
                .after;
                let first_preview = crate::mixed_resources::MoveSnapshot::capture_transform(
                    &self.doc,
                    selection.clone(),
                    crate::mixed_resources::TransformSpec::translate(Vec3::ZERO),
                )
                .unwrap()
                .before;
                assert_eq!(
                    first_preview, first_expected,
                    "rotation preview uses {pivot_mode:?}"
                );

                self.set_mixed_resource_rotation_degrees(Vec3::new(20.0, -10.0, 35.0));
                let expected = crate::mixed_resources::MoveSnapshot::capture_transform(
                    &original_document,
                    selection.clone(),
                    crate::mixed_resources::TransformSpec::rotate(
                        pivot,
                        glam::Quat::from_euler(
                            glam::EulerRot::ZYX,
                            35.0_f32.to_radians(),
                            -10.0_f32.to_radians(),
                            20.0_f32.to_radians(),
                        ),
                    ),
                )
                .unwrap()
                .after;
                let rebuilt = crate::mixed_resources::MoveSnapshot::capture_transform(
                    &self.doc,
                    selection.clone(),
                    crate::mixed_resources::TransformSpec::translate(Vec3::ZERO),
                )
                .unwrap()
                .before;
                assert_eq!(rebuilt, expected, "repeated rotation previews do not drift");
                assert!(
                    self.undo.is_empty(),
                    "Rotation preview does not enter history"
                );
                self.finish_mixed_resource_move(false);
                assert_eq!(
                    document_json(&self.doc),
                    document_json(&original_document),
                    "Rotation cancel restores all data"
                );

                self.begin_mixed_resource_move();
                self.set_mixed_resource_uniform_scale(1.25);
                let expected_scale = crate::mixed_resources::MoveSnapshot::capture_transform(
                    &original_document,
                    selection.clone(),
                    crate::mixed_resources::TransformSpec::uniform_scale(pivot, 1.25, true),
                )
                .unwrap()
                .after;
                let scale_preview = crate::mixed_resources::MoveSnapshot::capture_transform(
                    &self.doc,
                    selection.clone(),
                    crate::mixed_resources::TransformSpec::translate(Vec3::ZERO),
                )
                .unwrap()
                .before;
                assert_eq!(
                    scale_preview, expected_scale,
                    "scale preview uses {pivot_mode:?}"
                );
                self.finish_mixed_resource_move(false);
                assert_eq!(
                    document_json(&self.doc),
                    document_json(&original_document),
                    "Scale cancel restores all data"
                );
            }

            self.transform_pivot = selection::PivotMode::Custom;
            self.custom_pivot = custom_pivot;
            self.begin_mixed_resource_move();
            self.set_mixed_resource_rotation_degrees(Vec3::new(0.0, 30.0, 0.0));
            let applied_rotation = self.doc.clone();
            self.finish_mixed_resource_move(true);
            assert_eq!(self.undo.len(), 1, "Mixed rotation is one history edit");
            self.undo();
            assert_eq!(
                document_json(&self.doc),
                document_json(&original_document),
                "Rotation undo restores every resource"
            );
            self.redo();
            assert_eq!(
                document_json(&self.doc),
                document_json(&applied_rotation),
                "Rotation redo restores the preview"
            );

            self.transform_pivot = selection::PivotMode::WorldOrigin;
            self.begin_mixed_resource_move();
            self.set_mixed_resource_uniform_scale(1.1);
            let applied_scale = self.doc.clone();
            self.finish_mixed_resource_move(true);
            assert_eq!(
                self.undo.len(),
                2,
                "Mixed scale is one additional history edit"
            );
            self.undo();
            assert_eq!(
                document_json(&self.doc),
                document_json(&applied_rotation),
                "Scale undo preserves prior rotation"
            );
            self.redo();
            assert_eq!(
                document_json(&self.doc),
                document_json(&applied_scale),
                "Scale redo reapplies every resource"
            );
            assert_eq!(self.doc.active_image_plane, active_plane);

            let before_rejection = self.doc.clone();
            let mut invalid_selection = selection.clone();
            invalid_selection.obj_reference_ids.insert(u64::MAX - 10);
            assert!(crate::mixed_resources::MoveSnapshot::capture_transform(
                &self.doc,
                invalid_selection,
                crate::mixed_resources::TransformSpec::uniform_scale(Vec3::ZERO, 2.0, true),
            )
            .is_err());
            assert_eq!(
                document_json(&self.doc),
                document_json(&before_rejection),
                "invalid selection is atomic"
            );
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
                // Keep this synthetic hit test independent of the startup brush's grain.
                self.brush = Brush::default();
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
                // Hit the middle sample of the synthetic stroke instead of
                // depending on endpoint rasterization or picking.
                let stroke = &self.doc.strokes[0];
                let erase_point = crate::gizmo::project(
                    &self.camera,
                    rect,
                    stroke.points[stroke.points.len() / 2].position,
                )
                .expect("Smoke fixture midpoint should project onto the canvas")
                .0;
                self.erase(erase_point, rect);
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
                    assert!(Self::mixed_transform_supported(
                        crate::mixed_resources::TransformMode::Rotate
                    ));
                    assert!(Self::mixed_transform_supported(
                        crate::mixed_resources::TransformMode::Scale
                    ));
                    checks.push(
                        "mixed-resource controls enable Rotate and Scale and keep Apply disabled before a transform preview".into(),
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
                serde_json::json!({"passed":true,"checks":["generated asymmetric image texture uploaded to GPU","transformed image plane brush input handler commits a stroke","stroke points remain within transformed plane bounds","captured brush frame follows plane rotation","one-step stroke undo and exact redo","out-of-bounds image-plane contact rejected","inactive image plane does not intercept brush input","transformed finite image-plane line drag and undo/redo","image-plane fill click-to-close, clipping, and undo/redo","rotated image-plane circle drag, clipping, and undo/redo","rotated image-plane ellipse drag, camera orbit, clipping, and undo/redo","image-plane curve anchor clicks, insertion, tangent drag, Backspace removal, Enter commit, clipping, and undo/redo","older sampled strokes do not receive fabricated semantic curve controls","curve anchors and tangent controls survive native save/reopen exactly","reopened curve edit preview preserves source; Apply is one undo/redo change","Escape and focus loss cancel curve edit previews without document changes","curve edits survive native merge and fresh recovery scan","selection transforms preserve semantic curve anchors and captured frame","image-plane line preview displays endpoint snap cue and commits the snapped target","snapped image-plane endpoint survives undo/redo","Alt bypasses endpoint snapping and Escape cancels without an edit","line, fill, circle, and curve Escape cancel without document edits","shape Escape cancels after camera orbit without document edits","hidden and locked image planes reject all planar tools","artwork PNG includes image-plane pixels"],"image_plane_count":self.doc.image_planes.len(),"stroke_count":self.doc.strokes.len(),"png_changed_pixels_vs_no_plane":self.image_plane_verification_changed_pixels,"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.mirror_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic pointer-handler samples draw on an active saved cylinder guide","curved-guide brush points remain on the guide surface","mirror preview geometry does not mutate the document","committed mirrored geometry matches its preview exactly","source and mirrored run commit as one undo edit","undo/redo preserve the exact mirrored pair","cancel discards an in-progress source without history","whole-drawing GLB includes the mirrored pair","mouse canvas events preview, cancel, and commit a straight line on a rotated active guide","active guide horizontal constraint and mirrored points remain on the rotated plane","line source and mirror commit as one undo edit and survive undo/redo","radial line preview on a rotated planar guide leaves the document unchanged","Escape cancels a radial preview without history","one radial gesture commits its source and copies in a single undo edit","radial copies preserve the preview's geometry and brush settings","radial strokes survive recovery scan and whole-document GLB/OBJ export","canvas picking selects one radial copy independently","one radial copy accepts an isolated appearance edit; undo and redo preserve other copies","one undo/redo removes/restores the complete radial fan"],"stroke_count":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.ring_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["Add ring action creates and activates a saved torus at the camera target","active ring camera alignment faces the torus hole","synthetic pointer samples draw on the saved ring surface","a center-hole miss splits the drawing into two surface strokes","ring strokes remain on the torus instead of cutting through its hole","cancel removes an in-progress ring stroke without history","undo and redo preserve split ring strokes","ring strokes export to GLB"],"guide_count":self.doc.guide_resources.len(),"stroke_count":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.model_surface_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic pointer samples cross a shared manifold triangle edge in one model-surface stroke","world-space samples remain on the OBJ surface","seeded model-surface scatter is deterministic and projected","generated dab mesh remains on the captured OBJ surface","model-surface commit creates one ordinary undo edit","undo and redo preserve exact model-surface samples","a pointer miss commits the first surface run","recontact after a miss starts a second surface run without bridging the gap","scattered placements remain within disconnected model components","split model-surface runs undo and redo independently","captured model patches survive native save/reload"],"stroke_count":self.doc.strokes.len(),"point_count":self.doc.strokes.iter().map(|stroke|stroke.points.len()).sum::<usize>(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.obj_reference_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["OBJ References window displays its import control","synthetic pointer move, press, and release clicks the window's import control","canceling the picker leaves the document, history, and dirty state unchanged and keeps the source fixture available","the clicked control invokes the production import handler with an injected file-picker result","the imported drawing-local bundle reloads","imported model is registered with the scene renderer","synthetic viewport selection picks the imported OBJ","selected-model bounds match the normalized OBJ bounds","production Preview button creates detached line art","Update button recomputes line art for the current camera","Bake button creates ordinary strokes in one undo edit","undo and redo remove and restore the baked batch","removing the source model leaves baked strokes intact"],"native_file_picker_tested":false,"model_count":self.doc.model_references.len(),"triangle_count":self.obj_reference_models.values().map(|model|model.triangles.len()).sum::<usize>(),"stroke_count":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.liquify_brush_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic canvas drag creates a non-destructive Liquify preview","Liquify front, side, and ground camera views preserve working-plane depth","Liquify leaves unselected and occluding strokes unchanged","Liquify cancel restores exact source strokes","navigation interruption cancels Liquify preview without history","Liquify Apply commits one history edit; undo and redo restore geometry","updated Liquify stroke remains pickable at its deformed position","Liquify native serialization and recovery scan preserve geometry","Liquify whole-scene and selected-stroke GLB contain committed geometry","Liquify cursor indicates radius and working-plane depth","synthetic mouse Smooth preview preserves endpoints and Cancel restores the source","synthetic mouse Thickness preview changes a dedicated per-point profile while pressure and centerline stay exact","Sculpt Apply is one undo edit; undo/redo and updated-geometry picking pass","textured sculpt profile survives native reload, fresh recovery scan, GLB, and OBJ export"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.mixed_resources_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["mixed stroke, image plane, OBJ reference, and guide previews use the production transform path","translation, XYZ rotation, and positive uniform scale preview from immutable source records","rotation and scale pivot modes: selection center, world origin, and custom point","repeated rotation previews rebuild without cumulative drift","translation, rotation, and scale cancellation restores exact document records","rotation and scale previews do not enter history","rotation and scale Apply each create one undo edit","mixed-resource undo and redo restore all selected records exactly","invalid selected member is rejected before document mutation","active image-plane drawable binding survives preview, Apply, undo, and redo","OBJ reference renderer cache refreshes after preview and history transitions","non-uniform mixed scaling remains disabled"],"resource_counts":{"strokes":self.doc.strokes.len(),"image_planes":self.doc.image_planes.len(),"obj_references":self.doc.model_references.len(),"guides":self.doc.guide_resources.len()},"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.vase_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["revolved profile drawing","captured guide and surface release tail","pressure tilt brush frames","guide editing preserves ink","selection duplicate undo","native persistence","GPU painted vase"],"gpu":self.renderer.adapter})
            } else if self.layer_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["layer drawing and locking","visibility picking and erasing","layer reassignment undo redo","nonempty deletion protection","version 5 persistence","GPU layers"],"gpu":self.renderer.adapter})
            } else if self.recovery_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["forced process termination","startup recovery offer","inserted native stroke and remapped layer survive recovery after source deletion","exact fill contour offset pressure tilt dynamics texture restoration","image-plane recovery asset reinstalls the renderer texture","recovered copy is untitled and dirty","native source unchanged","new session snapshot durable before old retirement","Save New and close cleanup ordering","corrupt and unsupported issue visibility","GPU recovery workflow"],"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.preset_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["preset application captures resolved settings","create duplicate rename update delete and defaults","restart persistence","live GPU previews","library edits preserve existing ink and export","corrupt library fallback and explicit repair backup"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.texture_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["masked GPU rendering","selection retains coverage","Chalk/Graphite/Charcoal sampler includes pressure and width variation","Graphite and Charcoal world-grain scale and UVs remain stable under uniform transform","Graphite and Charcoal visible coverage is pickable and erases as one undoable stroke","front and orbit renders at 640x360, 1280x720, and 2560x1440","stroke duplication and undo","textured drawing and erasing at an alpha-covered point","version 30 save/reload","original procedural mask sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.dynamics_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["synthetic Windows Ink contact and tilt","independent pressure settings","pressure-to-opacity capture","soft-edge geometry coverage","overlap compositing independent of document order","camera-orbit stability","opaque-surface occlusion","stabilized release tail","cancellation discards dynamics","missing tilt fallback","captured settings","version 30 save/reload","GPU dynamics sampler"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.brush_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["all brush types through drawing input","captured guide orientation","camera orbit preserves mesh","shape-aware picking and erasing","duplication","scale and rotation preserve brush shape","cylinder frames and stabilized release tails","undo/redo","version 30 save/reload","GPU brush sampler","Stipple spacing and Spray density/spread sampler","scattered dab picking and partial erase","dab placement, render geometry, and GLB budgets","front/orbit Stipple and Spray renders at three output sizes","Leaf/Grass/Star image-tip taps and drags","transparent-edge image-tip picking and misses","image-tip native roundtrip","GLB original tip images, UVs, tint and alpha","OBJ diffuse and alpha companions","image-tip export sizes and CPU timings","front/orbit image-stamp renders at three output sizes","Rake and Bristle Fan strand geometry against Dry Brush","strand-aware picking and partial-erase fragments","native strand settings roundtrip","GLB and OBJ strand meshes parsed and budgeted","front/orbit strand renders at three output sizes","Square, Triangular, and Star Tube pressure/taper profiles","profiled mesh picking and native roundtrip","GLB and OBJ profile normals and faces parsed and budgeted","front/orbit profile renders at three output sizes"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.cylinder_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["cylinder side drawing","surface-constrained stabilization and release tail","miss and silhouette segmentation","angular seam continuity","rotated guide and captured surface","hide and switch guides preserve ink","selection duplicate/delete undo","cylinder stroke save/reload","seam-crossing cylinder curves preview across camera orbit and cancel cleanly","committed cylinder curves remain on their captured surface after native reload","cylinder curve edit remains on the captured surface and applies as one undo/redo step","cylinder ellipse preview, cancel, commit, semantic reopen, and undo/redo","editable lantern example","GPU screenshot"],"strokes":self.doc.strokes.len(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
            } else if self.house_verification_frame.is_some() {
                serde_json::json!({"passed":true,"checks":["editable dusk house with broad color faces and independent shadow, linework, and warm window layers","duplicate and undo","New clears starter and retains graphite","quick palette keyboard selection, Escape and focus dismissal","both house examples remain available","three-quarter opening view","orbit, pick and draw on wall, roof and chimney with hidden and locked filtering, cancel, undo and native reopen","synthetic pen barrel erases on lift, cancels on button change, and restores drawing on next contact","unsaved-change guard","sample save/reload","GPU screenshot"],"strokes":self.doc.strokes.len(),"points":self.doc.strokes.iter().map(|s|s.points.len()).sum::<usize>(),"gpu":self.renderer.adapter,"physical_tablet_tested":false})
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
            let mp4_frame_count = CAPABILITY_REPLAY_FRAMES * replay.clip.mp4_hold_frames();
            replay.mp4_encoder = Some(std::thread::spawn(move || {
                let cancelled = AtomicBool::new(false);
                video_export_ui::encode_mp4(
                    mp4_receiver,
                    &mp4_output,
                    mp4_size,
                    CAPABILITY_REPLAY_FPS,
                    mp4_frame_count,
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
        let poster_frame = if replay.clip.is_mirror_shapes() {
            30
        } else if replay.clip.is_curved_guides() {
            27
        } else if replay.clip.is_reference_export() {
            35
        } else if replay.clip.is_draw_orbit() {
            28
        } else if replay.clip.is_edit_erase() {
            39
        } else if replay.clip.is_dusk_house() {
            57
        } else if replay.clip.is_house_tutorial() || replay.clip.is_curved_tutorial() {
            if replay.clip.is_house_tutorial() {
                30
            } else {
                56
            }
        } else if replay.clip.is_overview() {
            30
        } else {
            0
        };
        if replay.captured_frames == poster_frame {
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
        let mp4_rgba = rgba.into_raw();
        for _ in 0..replay.clip.mp4_hold_frames() {
            if mp4_sender.send(Some(mp4_rgba.clone())).is_err() {
                eprintln!("Capability demo MP4 encoder stopped before all frames were captured");
                std::process::exit(1);
            }
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
        let workflow_check_frame = if replay.clip.is_mirror_shapes() {
            32
        } else if replay.clip.is_edit_erase() {
            39
        } else if replay.clip.is_overview() {
            31
        } else {
            27
        };
        if replay.captured_frames == workflow_check_frame {
            let committed = if replay.clip.is_curved_guides() {
                replay.setup_verified
                    && self.doc.strokes.len() == replay.initial_stroke_count + 2
                    && self.doc.active_guide.is_some()
            } else if replay.clip.is_mirror_shapes() {
                replay.setup_verified
                    && self.mirror_ui.enabled
                    && self.doc.strokes.len() == replay.initial_stroke_count + 3
                    && self
                        .doc
                        .strokes
                        .last()
                        .is_some_and(|stroke| stroke.points.len() > 8 && stroke.fill.is_none())
            } else if replay.clip.is_reference_export() {
                replay.setup_verified
                    && replay.reference_import_action
                    && replay.reference_insert_action
                    && self.doc.strokes.len()
                        == replay.initial_stroke_count + replay.reference_inserted_stroke_count
                    && self.doc.image_planes.iter().any(|plane| plane.visible)
                    && self
                        .doc
                        .model_references
                        .iter()
                        .filter(|model| model.visible)
                        .count()
                        == 2
            } else if replay.clip.is_edit_erase() {
                let split_strokes = self.doc.strokes.as_slice();
                let ordered_fragments =
                    split_strokes
                        .first()
                        .zip(split_strokes.get(1))
                        .map(|(first, second)| {
                            if first.points.first().unwrap().position.x
                                <= second.points.first().unwrap().position.x
                            {
                                (first, second)
                            } else {
                                (second, first)
                            }
                        });
                replay.setup_verified
                    && self.eraser
                    && self.partial_eraser.enabled
                    && split_strokes.len() == replay.initial_stroke_count + 1
                    && ordered_fragments.is_some_and(|(left, right)| {
                        left.points.len() > 2
                            && right.points.len() > 2
                            && left.points.last().unwrap().position.x
                                < right.points.first().unwrap().position.x
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
            && replay.captured_frames == 41
            && (!replay.undo_applied
                || replay.expected_undo_strokes.as_ref() != Some(&self.doc.strokes)
                || self.undo.len() != 1)
        {
            self.capability_replay.take().unwrap().abort();
            eprintln!("Capability demo undo did not restore the edited source stroke");
            std::process::exit(1);
        }
        if replay.clip.is_overview() && replay.captured_frames == 31 && !replay.long_edit_verified {
            self.capability_replay.take().unwrap().abort();
            eprintln!(
                "Capability overview did not complete its selection transform and undo/redo steps"
            );
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
            if replay.clip.is_long_form()
                && (!replay.long_edit_verified || !replay.long_saved || !replay.long_reopened)
            {
                self.capability_replay.take().unwrap().abort();
                eprintln!("Capability tutorial did not complete its verified edit, production save, and reopen steps");
                std::process::exit(1);
            }
            if replay.clip.is_dusk_house()
                && (!self.presentation.artwork
                    || self.png_export.is_running()
                    || !replay.output_dir.join("dusk-house-still.png").is_file())
            {
                self.capability_replay.take().unwrap().abort();
                eprintln!(
                    "Dusk-house replay did not finish the still export and artwork-only view"
                );
                std::process::exit(1);
            }
            if replay.clip.is_reference_export() && replay.reference_export_action.is_none() {
                self.capability_replay.take().unwrap().abort();
                eprintln!("Reference-export replay did not complete the production whole-scene export action");
                std::process::exit(1);
            }
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

    #[test]
    fn quick_palette_search_matches_names_groups_and_shortcuts() {
        let items = vec![
            QuickPaletteItem {
                group: "Tools",
                label: "Brush",
                shortcut: "B",
                command: QuickPaletteCommand::Tool("Brush"),
                enabled: true,
            },
            QuickPaletteItem {
                group: "History",
                label: "Undo",
                shortcut: "Ctrl/Cmd+Z",
                command: QuickPaletteCommand::Undo,
                enabled: false,
            },
        ];
        assert_eq!(filter_quick_palette_items(items.clone(), "  bru").len(), 1);
        assert_eq!(
            filter_quick_palette_items(items.clone(), "history").len(),
            1
        );
        assert_eq!(filter_quick_palette_items(items.clone(), "cmd+z").len(), 1);
        assert!(filter_quick_palette_items(items, "no match").is_empty());
    }

    #[test]
    fn canvas_controls_block_new_contact_but_continue_active_stroke() {
        let context = egui::Context::default();
        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
        let strip_point = Pos2::new(200.0, 40.0);
        for _ in 0..2 {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(canvas),
                    focused: true,
                    events: vec![egui::Event::PointerMoved(strip_point)],
                    ..Default::default()
                },
                |ctx| {
                    egui::Area::new(egui::Id::new("test-canvas-strip"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(egui::pos2(160.0, 25.0))
                        .show(ctx, |ui| {
                            ui.allocate_exact_size(Vec2::new(80.0, 30.0), egui::Sense::click());
                        });
                },
            );
        }
        assert!(!canvas_contact_allowed(
            &context,
            canvas,
            strip_point,
            false
        ));
        assert!(canvas_contact_allowed(&context, canvas, strip_point, true));
        assert!(!canvas_contact_allowed(
            &context,
            canvas,
            Pos2::new(410.0, 40.0),
            true
        ));
    }

    fn stroke(id: u64, layer_id: u64, x: f32) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            curve: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
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

                    time_ms: 0,
                },
                Point {
                    position: Vec3::new(x + 0.5, 0.0, 0.0),
                    pressure: 0.8,
                    tilt: None,

                    time_ms: 0,
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
            linked_stroke_groups: Default::default(),
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
            linked_stroke_groups: Default::default(),
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
                    opacity: 1.0,
                    blend_mode: crate::document::LayerBlendMode::Normal,
                    hide_onion_skin: false,
                    mask_inverted: false,
                    mask_source: None,
                    clip_to_below: false,
                }],
                folders: Vec::new(),
                image_planes: Vec::new(),
                guides: Vec::new(),
                model_references: Vec::new(),
                linked_stroke_groups: Default::default(),
                color_swatches: Default::default(),
                previous_active_layer: 1,
                inserted_active_layer: 2,
            },
            next_id: 3,
            image_assets: HashMap::new(),
            obj_models: HashMap::new(),
            brush_tip_assets: Vec::new(),
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
