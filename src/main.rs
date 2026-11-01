#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod app;
mod atomic_file;
mod brush_selection;
mod camera_bookmarks;
mod construction_history;
mod curve_geometry;
mod document;
mod drawing_library;
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
mod presentation;
mod presets;
mod presets_ui;
mod recovery;
mod recovery_store;
mod renderer;
mod revolved;
mod scenes;
mod shape_geometry;
mod stabilizer;
mod surface;
mod texture;
mod texture_check;
mod tutorial;

fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args().collect();
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
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Sketch3D")
            .with_inner_size([1440.0, 940.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Sketch3D",
        options,
        Box::new(|cc| Ok(Box::new(app::SketchApp::new(cc)))),
    )
}
