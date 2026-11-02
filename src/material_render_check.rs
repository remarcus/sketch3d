//! Headless GPU verification that canvas and artwork PNG material rendering agree.
use crate::{
    document::{Document, Point, Stroke, StrokeMaterial},
    geometry::{Camera, Projection},
    lighting::Lighting,
    renderer::SceneRenderer,
    surface::GuidePreview,
};
use eframe::egui_wgpu;
use glam::Vec3;
use serde_json::json;
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    thread,
};

const SIZE: [u32; 2] = [256, 256];
const BACKGROUND: [f32; 3] = [0.08, 0.12, 0.2];
const INK: [f32; 3] = [0.92, 0.24, 0.12];

struct ThreadWake(thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

fn stroke(id: u64, material: StrokeMaterial, radius: f32, z: f32) -> Stroke {
    stroke_segment(id, material, radius, -0.9, 0.9, z)
}

fn stroke_segment(
    id: u64,
    material: StrokeMaterial,
    radius: f32,
    start_x: f32,
    end_x: f32,
    z: f32,
) -> Stroke {
    Stroke {
        curve: None,
        fill: None,
        material,
        group_id: None,
        layer_id: 1,
        id,
        color: INK,
        gradient: None,
        radius,
        brush: Default::default(),
        points: vec![
            Point {
                position: Vec3::new(start_x, 0.0, z),
                pressure: 0.8,
                tilt: None,
                time_ms: 0,
            },
            Point {
                position: Vec3::new(end_x, 0.0, z),
                pressure: 0.8,
                tilt: None,
                time_ms: 0,
            },
        ],
    }
}

fn document(material: StrokeMaterial) -> Document {
    Document {
        strokes: vec![stroke(1, material, 0.22, 0.0)],
        ..Document::default()
    }
}

fn count_visible(pixels: &[u8], background: [u8; 4]) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| **pixel != background)
        .count()
}

fn pixel_at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * SIZE[0] + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().unwrap()
}

fn render_pair(
    renderer: &mut SceneRenderer,
    doc: &Document,
    camera: &Camera,
    lighting: Lighting,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    renderer.render(doc, camera, GuidePreview::Hidden, SIZE);
    let canvas = renderer.verification_pixels();
    let png = renderer.render_png_pixels(doc, camera, SIZE, BACKGROUND, lighting)?;
    Ok((canvas, png))
}

pub fn run(output: &Path) -> Result<(), String> {
    let config = egui_wgpu::WgpuConfiguration::default();
    let instance = block_on(config.wgpu_setup.new_instance());
    let state = block_on(egui_wgpu::RenderState::create(
        &config, &instance, None, None, 1, false,
    ))
    .map_err(|error| format!("Could not create headless GPU renderer: {error}"))?;
    let adapter = state.adapter.get_info();
    let mut renderer = SceneRenderer::new(state);
    renderer.background = BACKGROUND;
    renderer.lighting = Lighting {
        ground: false,
        shadows: false,
        ..Lighting::default()
    };
    renderer.post_effects = Default::default();
    renderer.grain = 0.0;
    let lighting = renderer.lighting;
    let camera = Camera {
        projection: Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        distance: 5.0,
        target: Vec3::ZERO,
    };

    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let background_pixel = [
        (BACKGROUND[0] * 255.0).round() as u8,
        (BACKGROUND[1] * 255.0).round() as u8,
        (BACKGROUND[2] * 255.0).round() as u8,
        255,
    ];
    let mut modes = Vec::new();
    for (name, material) in [
        ("shaded", StrokeMaterial::Shaded),
        ("unlit", StrokeMaterial::Unlit),
        ("emissive", StrokeMaterial::Emissive),
        ("cutout", StrokeMaterial::Cutout),
    ] {
        let doc = document(material);
        let (canvas, png) = render_pair(&mut renderer, &doc, &camera, lighting)?;
        if canvas != png {
            let (canvas_pixels, _) = canvas.as_chunks::<4>();
            let (png_pixels, _) = png.as_chunks::<4>();
            let differing_pixels = canvas_pixels
                .iter()
                .zip(png_pixels)
                .filter(|(left, right)| left != right)
                .count();
            return Err(format!(
                "{name} canvas and PNG differ at {differing_pixels} pixels"
            ));
        }
        let visible_pixels = count_visible(&canvas, background_pixel);
        if material == StrokeMaterial::Cutout {
            if visible_pixels > 0 {
                return Err("isolated cutout changed the configured opaque background".into());
            }
        } else if visible_pixels < 20 {
            return Err(format!("{name} material produced too few visible pixels"));
        }
        let png_path = output.join(format!("{name}.png"));
        image::save_buffer(&png_path, &png, SIZE[0], SIZE[1], image::ColorType::Rgba8)
            .map_err(|error| error.to_string())?;
        modes.push(json!({
            "mode": name,
            "canvas_png_exact_pixel_match": true,
            "visible_pixels": visible_pixels,
            "png": png_path.file_name().unwrap().to_string_lossy(),
        }));
    }

    let layered = Document {
        strokes: vec![
            stroke(1, StrokeMaterial::Shaded, 0.3, 0.0),
            stroke_segment(2, StrokeMaterial::Cutout, 0.3, -0.25, 0.25, 0.0),
        ],
        ..Document::default()
    };
    let (canvas, png) = render_pair(&mut renderer, &layered, &camera, lighting)?;
    if canvas != png {
        return Err("cutout-over-ink canvas and PNG pixels differ".into());
    }
    let center = pixel_at(&png, SIZE[0] / 2, SIZE[1] / 2);
    if center != background_pixel {
        return Err(format!(
            "cutout did not restore background at stroke center: {center:?}"
        ));
    }
    let outside = pixel_at(&png, SIZE[0] / 2 + 32, SIZE[1] / 2);
    if outside == background_pixel {
        return Err("cutout removed ink outside its coverage".into());
    }
    let transparent = renderer.render_transparent_png_pixels(&layered, &camera, SIZE, lighting)?;
    image::save_buffer(
        output.join("cutout-over-ink-transparent.png"),
        &transparent,
        SIZE[0],
        SIZE[1],
        image::ColorType::Rgba8,
    )
    .map_err(|error| error.to_string())?;
    let transparent_center = pixel_at(&transparent, SIZE[0] / 2, SIZE[1] / 2);
    if transparent_center[3] != 0 {
        return Err(format!(
            "cutout did not clear alpha in transparent PNG output: {transparent_center:?}"
        ));
    }
    if pixel_at(&transparent, SIZE[0] / 2 + 32, SIZE[1] / 2)[3] == 0 {
        return Err("transparent PNG cutout cleared ink outside its coverage".into());
    }
    image::save_buffer(
        output.join("cutout-over-ink.png"),
        &png,
        SIZE[0],
        SIZE[1],
        image::ColorType::Rgba8,
    )
    .map_err(|error| error.to_string())?;

    let report = json!({
        "passed": true,
        "adapter": adapter.name,
        "backend": format!("{:?}", adapter.backend),
        "canvas_sample_count": renderer.sample_count(),
        "size": SIZE,
        "modes": modes,
        "cutout_over_ink": {
            "canvas_png_exact_pixel_match": true,
            "opaque_png_restores_background_at_cutout": true,
            "transparent_png_clears_alpha_at_cutout": true,
            "ink_outside_cutout_survives": true,
        },
    });
    let report_path: PathBuf = output.join("material-render-report.json");
    std::fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "Material render parity verified; report: {}",
        report_path.display()
    );
    Ok(())
}
