#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod app;
mod atomic_file;
mod brush_selection;
mod camera_bookmarks;
mod construction_history;
mod curve_geometry;
mod document;
mod drawing_library;
mod drawing_library_restart_check;
mod dynamics;
mod editing;
mod export;
mod export_check;
mod geometry;
mod gizmo;
mod image_plane_storage;
mod image_planes;
mod input;
mod lighting;
mod liquify;
mod material_render_check;
mod mirror;
mod mixed_resources;
mod obj_export;
mod obj_mesh_index;
#[allow(dead_code)]
mod obj_reference;
#[allow(dead_code)]
mod obj_reference_storage;
mod palette;
mod partial_eraser;
mod performance_benchmark;
mod placement;
mod polygon;
mod post_effects_check;
mod presentation;
mod presentation_background_check;
mod presets;
mod presets_ui;
mod recovery;
mod recovery_store;
mod renderer;
mod revolved;
mod scenes;
mod shape_geometry;
mod stabilizer;
mod still_export_check;
mod surface;
mod texture;
mod texture_check;
mod tutorial;

fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-drawing-trash-seed")
    {
        let Some(marker) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-drawing-trash-seed <marker-path>");
            std::process::exit(2);
        };
        if let Err(error) = drawing_library_restart_check::seed(std::path::Path::new(marker)) {
            eprintln!("Drawing-trash restart seed failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-drawing-trash-reopen")
    {
        let (Some(marker), Some(destination), Some(report)) = (
            args.get(index + 1),
            args.get(index + 2),
            args.get(index + 3),
        ) else {
            eprintln!("Usage: sketch3d --verify-drawing-trash-reopen <marker-path> <destination-folder> <report-path>");
            std::process::exit(2);
        };
        if let Err(error) = drawing_library_restart_check::reopen(
            std::path::Path::new(marker),
            std::path::Path::new(destination),
            std::path::Path::new(report),
        ) {
            eprintln!("Drawing-trash restart verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--write-capability-fixtures")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --write-capability-fixtures <directory>");
            std::process::exit(2);
        };
        match scenes::write_capability_fixtures(std::path::Path::new(directory)) {
            Ok(paths) => {
                for path in paths {
                    println!("Wrote {}", path.display());
                }
            }
            Err(error) => {
                eprintln!("Could not write capability fixtures: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--write-obj-material-fixture")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --write-obj-material-fixture <directory>");
            std::process::exit(2);
        };
        match obj_export::write_material_verification_bundle(std::path::Path::new(directory)) {
            Ok(()) => println!("Wrote OBJ material verification fixture to {directory}"),
            Err(error) => {
                eprintln!("Could not write OBJ material verification fixture: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-material-render")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-material-render <output-directory>");
            std::process::exit(2);
        };
        if let Err(error) = material_render_check::run(std::path::Path::new(directory)) {
            eprintln!("Material rendering verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-still-export")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-still-export <output-directory>");
            std::process::exit(2);
        };
        if let Err(error) = still_export_check::run(std::path::Path::new(directory)) {
            eprintln!("Still export visual verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-presentation-background")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-presentation-background <output-directory>");
            std::process::exit(2);
        };
        if let Err(error) = presentation_background_check::run(std::path::Path::new(directory)) {
            eprintln!("Presentation background verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-post-effects")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-post-effects <output-directory>");
            std::process::exit(2);
        };
        if let Err(error) = post_effects_check::run(std::path::Path::new(directory)) {
            eprintln!("Post-processing verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--write-native-merge-fixture")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --write-native-merge-fixture <output-directory>");
            std::process::exit(2);
        };
        if let Err(error) = app::write_native_merge_fixture(std::path::Path::new(directory)) {
            eprintln!("Native merge fixture generation failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().any(|a| a == "--benchmark-performance") {
        if let Err(error) = performance_benchmark::run() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().any(|a| a == "--benchmark-textures") {
        if let Err(error) = texture_check::run() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().any(|a| a == "--verify-export") {
        if let Err(error) = export_check::run() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let hidden_verification = args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "--verify-model-surfaces" | "--verify-liquify-brush" | "--verify-obj-references"
        )
    });
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Sketch3D")
            .with_inner_size([1440.0, 940.0])
            .with_min_inner_size([960.0, 640.0])
            .with_visible(!hidden_verification),
        ..Default::default()
    };
    eframe::run_native(
        "Sketch3D",
        options,
        Box::new(|cc| Ok(Box::new(app::SketchApp::new(cc)))),
    )
}
