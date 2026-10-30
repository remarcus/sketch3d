#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod app;
mod document;
mod dynamics;
mod editing;
mod export;
mod export_check;
mod geometry;
mod gizmo;
mod input;
mod presets;
mod presets_ui;
mod recovery;
mod recovery_store;
mod renderer;
mod revolved;
mod scenes;
mod stabilizer;
mod surface;
mod texture;
mod texture_check;

fn main() -> eframe::Result {
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
            .with_title("Spatial Sketch")
            .with_inner_size([1440.0, 940.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Spatial Sketch",
        options,
        Box::new(|cc| Ok(Box::new(app::SketchApp::new(cc)))),
    )
}
