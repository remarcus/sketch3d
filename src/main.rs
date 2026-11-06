#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod animation;
mod animation_check;
mod app;
mod atomic_file;
mod brush_selection;
mod brush_tip;
mod camera_bookmark_restart_check;
mod camera_bookmarks;
mod construction_history;
mod curve_geometry;
mod dab_placement;
mod document;
mod drawing_library;
mod drawing_library_restart_check;
mod dynamics;
mod editing;
mod export;
mod export_check;
mod geometry;
mod gizmo;
mod glb_texture_roundtrip_check;
mod gltf_reference;
mod grease_pencil_handoff;
mod image_plane_storage;
mod image_planes;
mod input;
mod lighting;
mod liquify;
mod live_mcp;
mod local_color;
mod material_render_check;
mod mirror;
mod mixed_resources;
mod model_line_art;
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
mod point_edit;
mod junction_edit;
mod hatch;
mod polygon;
mod post_effects_check;
mod presentation;
mod presentation_background_check;
mod presets;
mod presets_ui;
mod recovery;
mod recovery_store;
mod region_fill;
mod mesh_fill;
mod renderer;
mod reprojection;
mod revolved;
mod scenes;
mod shape_geometry;
mod share_service;
mod stabilizer;
mod still_export_check;
mod stroke_modifiers;
mod stroke_sculpt;
mod surface;
mod svg_export;
mod svg_import;
mod texture;
mod texture_check;
mod tutorial;
mod verification_mcp;

fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-glb-texture-roundtrip")
    {
        let Some(directory) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-glb-texture-roundtrip <output-directory>");
            std::process::exit(2);
        };
        if let Err(error) = glb_texture_roundtrip_check::run(std::path::Path::new(directory)) {
            eprintln!("GLB texture round-trip verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--mcp-verification-server")
    {
        let Some(root) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --mcp-verification-server <verification-directory>");
            std::process::exit(2);
        };
        if let Err(error) = verification_mcp::serve(std::path::Path::new(root)) {
            eprintln!("Verification MCP server stopped: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--mcp-verification-execution-server")
    {
        let Some(root) = args.get(index + 1) else {
            eprintln!(
                "Usage: sketch3d --mcp-verification-execution-server <verification-directory>"
            );
            std::process::exit(2);
        };
        if let Err(error) = verification_mcp::serve_with_execution(std::path::Path::new(root)) {
            eprintln!("Verification MCP server stopped: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--mcp-drawing-server")
    {
        let (Some(input), Some(output)) = (args.get(index + 1), args.get(index + 2)) else {
            eprintln!("Usage: sketch3d --mcp-drawing-server <native-input-directory> <glb-output-directory>");
            std::process::exit(2);
        };
        if let Err(error) = verification_mcp::serve_drawings(
            std::path::Path::new(input),
            std::path::Path::new(output),
        ) {
            eprintln!("Drawing MCP server stopped: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-artwork-animation")
    {
        let Some(report) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-artwork-animation <report-path>");
            std::process::exit(2);
        };
        if let Err(error) = animation_check::run(std::path::Path::new(report)) {
            eprintln!("Artwork animation verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--benchmark-stroke-modifiers")
    {
        let Some(output) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --benchmark-stroke-modifiers <report-path>");
            std::process::exit(2);
        };
        if let Err(error) = stroke_modifiers::benchmark(std::path::Path::new(output)) {
            eprintln!("Stroke modifier benchmark failed: {error}");
            std::process::exit(1);
        }
        println!("Stroke modifier benchmark report: {output}");
        return Ok(());
    }
    if args
        .iter()
        .any(|argument| argument == "--verify-camera-bookmarks-seed")
    {
        if let Err(error) = camera_bookmark_restart_check::seed() {
            eprintln!("Camera bookmark restart seed failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-camera-bookmarks")
    {
        if args.get(index + 1).is_none() {
            eprintln!("Usage: sketch3d --verify-camera-bookmarks <report-path>");
            std::process::exit(2);
        }
    }
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
        .position(|argument| argument == "--verify-library-backup-interrupt-child")
    {
        let Some(source) = args.get(index + 1) else {
            eprintln!("Usage: sketch3d --verify-library-backup-interrupt-child <source-drawing>");
            std::process::exit(2);
        };
        if let Err(error) =
            drawing_library_restart_check::interrupt_backup_copy(std::path::Path::new(source))
        {
            eprintln!("Library backup interruption child failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--verify-library-backup-interrupt-reopen")
    {
        let (Some(source), Some(destination), Some(report)) = (
            args.get(index + 1),
            args.get(index + 2),
            args.get(index + 3),
        ) else {
            eprintln!("Usage: sketch3d --verify-library-backup-interrupt-reopen <source-drawing> <restore-destination> <report-path>");
            std::process::exit(2);
        };
        let result = presets::path()
            .map(|path| path.join("drawing-trash"))
            .and_then(|trash_root| {
                drawing_library_restart_check::reopen_after_interrupted_copy(
                    std::path::Path::new(source),
                    &trash_root,
                    std::path::Path::new(destination),
                    std::path::Path::new(report),
                )
            });
        if let Err(error) = result {
            eprintln!("Library backup interruption verification failed: {error}");
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
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--record-capability-demo")
    {
        let name = args.get(index + 1).map(String::as_str).unwrap_or_default();
        if !matches!(
            name,
            "draw-orbit"
                | "curved-guides"
                | "mirror-shapes"
                | "edit-erase"
                | "reference-export"
                | "overview"
                | "house-tutorial"
                | "dusk-house"
                | "curved-tutorial"
        ) || args.len() != index + 2
        {
            eprintln!(
                "Usage: sketch3d --record-capability-demo <draw-orbit|curved-guides|mirror-shapes|edit-erase|reference-export|overview|house-tutorial|dusk-house|curved-tutorial>"
            );
            std::process::exit(2);
        }
        if let Err(error) = app::prepare_capability_demo_output(name) {
            eprintln!("Could not prepare capability demo output: {error}");
            std::process::exit(2);
        }
    }
    let hidden_verification = args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "--verify-model-surfaces"
                | "--verify-point-edit"
                | "--verify-liquify-brush"
                | "--verify-obj-references"
                | "--verify-camera-bookmarks"
                | "--verify-artwork-animation"
                | "--verify-fills-no-capture"
                | "--verify-house-no-capture"
                | "--verify-textures-no-capture"
                | "--verify-appearance-layout"
                | "--record-capability-demo"
        )
    });
    let viewport_size = if args
        .iter()
        .any(|argument| argument == "--record-capability-demo")
    {
        [1280.0, 720.0]
    } else {
        args.iter()
            .position(|argument| argument == "--verify-appearance-layout")
            .map(|index| {
                let width = args
                    .get(index + 3)
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|dimension| (640..=3840).contains(dimension))
                    .unwrap_or(1440);
                let height = args
                    .get(index + 4)
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|dimension| (480..=2160).contains(dimension))
                    .unwrap_or(940);
                [width as f32, height as f32]
            })
            .unwrap_or([1440.0, 940.0])
    };
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(mut setup) = wgpu_options.wgpu_setup {
        let default_device_descriptor = std::sync::Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut descriptor = default_device_descriptor(adapter);
            if adapter.limits().max_bind_groups >= 5 {
                descriptor.required_limits.max_bind_groups = 5;
            }
            descriptor
        });
        wgpu_options.wgpu_setup = setup.into();
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Sketch3D")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/sketch3d.png"))
                    .expect("embedded application icon must be valid PNG"),
            )
            .with_inner_size(viewport_size)
            .with_min_inner_size([960.0, 640.0])
            .with_visible(!hidden_verification),
        ..Default::default()
    };
    let live_mcp_enabled = args
        .iter()
        .any(|argument| argument == "--mcp-live-app-server");
    eframe::run_native(
        "Sketch3D",
        options,
        Box::new(move |cc| {
            let app = if live_mcp_enabled {
                let (server, requests) = live_mcp::channel();
                let egui_context = cc.egui_ctx.clone();
                std::thread::Builder::new()
                    .name("sketch3d-live-mcp".into())
                    .spawn(move || {
                        if let Err(error) = live_mcp::serve(server, egui_context) {
                            eprintln!("Live session MCP server stopped: {error}");
                        }
                    })
                    .expect("live session MCP thread must start");
                app::SketchApp::new_with_live_mcp(cc, requests)
            } else {
                app::SketchApp::new(cc)
            };
            Ok(Box::new(app))
        }),
    )
}
