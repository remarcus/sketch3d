//! Headless visual checks for transparent PNG edges and JPEG backgrounds.
use crate::{
    document::{Document, Point, Stroke, StrokeMaterial},
    geometry::{Camera, Projection},
    lighting::Lighting,
    renderer::SceneRenderer,
};
use eframe::egui_wgpu;
use glam::Vec3;
use serde_json::json;
use std::{
    future::Future,
    path::Path,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    thread,
};

const SIZE: [u32; 2] = [384, 256];
const INK: [f32; 3] = [0.82, 0.24, 0.12];
const CANVAS: [f32; 3] = [0.08, 0.12, 0.2];

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

fn stroke() -> Stroke {
    Stroke {
        guide_attachment: None,
        model_surface: None,
        curve: None,
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        fill: None,
        material: StrokeMaterial::Unlit,
        group_id: None,
        layer_id: 1,
        id: 1,
        color: INK,
        gradient: None,
        radius: 0.16,
        brush: Default::default(),
        points: [-1.2, 1.2]
            .into_iter()
            .map(|x| Point {
                position: Vec3::new(x, 0.0, 0.0),
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .collect(),
    }
}

fn save_rgba(path: &Path, pixels: &[u8]) -> Result<(), String> {
    image::save_buffer(path, pixels, SIZE[0], SIZE[1], image::ColorType::Rgba8)
        .map_err(|error| error.to_string())
}

fn composite_preview(pixels: &[u8], background: [f32; 3]) -> Vec<u8> {
    let mut preview = pixels.to_vec();
    for pixel in preview.as_chunks_mut::<4>().0 {
        let alpha = f32::from(pixel[3]) / 255.0;
        for channel in 0..3 {
            let foreground = crate::presentation::srgb_to_linear(f32::from(pixel[channel]) / 255.0);
            let background = crate::presentation::srgb_to_linear(background[channel]);
            let composed = foreground * alpha + background * (1.0 - alpha);
            pixel[channel] = (crate::presentation::linear_to_srgb(composed) * 255.0).round() as u8;
        }
        pixel[3] = 255;
    }
    preview
}

pub fn run(output: &Path) -> Result<(), String> {
    let mut config = egui_wgpu::WgpuConfiguration::default();
    if let egui_wgpu::WgpuSetup::CreateNew(mut setup) = config.wgpu_setup {
        let default_device_descriptor = std::sync::Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut descriptor = default_device_descriptor(adapter);
            if adapter.limits().max_bind_groups >= 5 {
                descriptor.required_limits.max_bind_groups = 5;
            }
            descriptor
        });
        config.wgpu_setup = setup.into();
    }
    let instance = block_on(config.wgpu_setup.new_instance());
    let state = block_on(egui_wgpu::RenderState::create(
        &config, &instance, None, None, 1, false,
    ))
    .map_err(|error| format!("Could not create headless GPU renderer: {error}"))?;
    let adapter = state.adapter.get_info();
    let mut renderer = SceneRenderer::new(state);
    renderer.background = CANVAS;
    renderer.lighting = Lighting {
        ground: false,
        shadows: false,
        ..Lighting::default()
    };
    renderer.post_effects = Default::default();
    renderer.grain = 0.0;
    renderer.toon_shading = false;
    let lighting = renderer.lighting;
    let camera = Camera {
        projection: Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        distance: 5.0,
        target: Vec3::ZERO,
    };
    let document = Document {
        linked_stroke_groups: Default::default(),
        strokes: vec![stroke()],
        ..Document::default()
    };
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;

    let transparent = renderer.render_transparent_png_pixels(&document, &camera, SIZE, lighting)?;
    save_rgba(&output.join("transparent-edge.png"), &transparent)?;
    let mut zero_alpha_pixels = 0;
    let mut edge_pixels = 0;
    let mut opaque_pixels = 0;
    let mut max_edge_channel_error = 0_u8;
    let mut worst_edge = [0; 4];
    let expected_ink =
        INK.map(|channel| (crate::presentation::linear_to_srgb(channel) * 255.0).round() as u8);
    for pixel in transparent.as_chunks::<4>().0 {
        match pixel[3] {
            0 => {
                zero_alpha_pixels += 1;
                if pixel[..3] != [0, 0, 0] {
                    return Err(format!("Zero-alpha pixel retains color data: {pixel:?}"));
                }
            }
            1..=254 => {
                edge_pixels += 1;
                for channel in 0..3 {
                    let error = pixel[channel].abs_diff(expected_ink[channel]);
                    if error > max_edge_channel_error {
                        max_edge_channel_error = error;
                        worst_edge = *pixel;
                    }
                }
            }
            _ => opaque_pixels += 1,
        }
    }
    if edge_pixels < 8 {
        return Err(format!(
            "Too few partially covered edge pixels: {edge_pixels}"
        ));
    }
    if max_edge_channel_error > 8 {
        return Err(format!(
            "Transparent edge color differs from straight-alpha ink by {max_edge_channel_error} levels (RGBA {worst_edge:?}, expected RGB {expected_ink:?})"
        ));
    }
    if zero_alpha_pixels == 0 || opaque_pixels == 0 {
        return Err("Transparent export lacks clear or fully covered pixels".into());
    }
    let edge_white = composite_preview(&transparent, [1.0; 3]);
    let edge_black = composite_preview(&transparent, [0.0; 3]);
    save_rgba(&output.join("transparent-edge-on-white.png"), &edge_white)?;
    save_rgba(&output.join("transparent-edge-on-black.png"), &edge_black)?;

    let background_cases = [("canvas", CANVAS), ("white", [1.0; 3]), ("black", [0.0; 3])];
    let mut jpeg_checks = Vec::new();
    for (name, background) in background_cases {
        let pixels = renderer.render_png_pixels(&document, &camera, SIZE, background, lighting)?;
        let path = output.join(format!("jpeg-{name}.jpg"));
        crate::app::png_export_ui::write_jpeg_atomic(&path, SIZE, &pixels)?;
        let decoded = image::open(&path)
            .map_err(|error| error.to_string())?
            .to_rgb8();
        let actual = decoded.get_pixel(4, 4).0;
        let expected = background.map(|channel| (channel * 255.0).round() as u8);
        let max_error = actual
            .into_iter()
            .zip(expected)
            .map(|(actual, expected)| actual.abs_diff(expected))
            .max()
            .unwrap_or(0);
        if max_error > 5 {
            return Err(format!(
                "JPEG {name} background differs from requested color by {max_error} levels"
            ));
        }
        jpeg_checks.push(json!({
            "choice": name,
            "expected_corner_rgb": expected,
            "decoded_corner_rgb": actual,
            "maximum_channel_error": max_error,
            "artifact": path.file_name().unwrap().to_string_lossy(),
        }));
    }

    let report_path = output.join("still-export-report.json");
    let report = json!({
        "passed": true,
        "adapter": adapter.name,
        "backend": format!("{:?}", adapter.backend),
        "size": SIZE,
        "transparent_png": {
            "zero_alpha_pixels_have_zero_rgb": true,
            "zero_alpha_pixel_count": zero_alpha_pixels,
            "partially_covered_edge_pixels": edge_pixels,
            "fully_covered_pixels": opaque_pixels,
            "maximum_straight_color_channel_error": max_edge_channel_error,
            "artifact": "transparent-edge.png",
            "composite_previews": ["transparent-edge-on-white.png", "transparent-edge-on-black.png"],
        },
        "jpeg_backgrounds": jpeg_checks,
        "jpeg_uses_production_atomic_writer": true,
        "checks": [
            "transparent antialias pixels retain straight ink color after linear-space unpremultiplication",
            "zero-alpha pixels contain no residual color",
            "Canvas, White, and Black JPEG renders use the production atomic writer and decode with their requested corner background",
        ],
    });
    std::fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "Still export visual checks passed; report: {}",
        report_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_stroke_produces_srgb_alpha_edge_expectations() {
        let expected = INK.map(crate::presentation::linear_to_srgb);
        assert!(expected[0] > expected[1] && expected[1] > expected[2]);
        assert!(expected.iter().all(|channel| (0.0..=1.0).contains(channel)));
    }
}
