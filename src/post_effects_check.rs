//! Headless visual and GPU-backed latency checks for optional post effects.
use crate::{
    document::{Document, Point, Stroke, StrokeMaterial},
    geometry::{Camera, Projection},
    lighting::Lighting,
    presentation::PostEffects,
    renderer::SceneRenderer,
    surface::GuidePreview,
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
    time::{Duration, Instant},
};

const SIZES: [[u32; 2]; 2] = [[384, 256], [768, 512]];
const BACKGROUND: [f32; 3] = [0.025, 0.035, 0.055];

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

fn stroke(id: u64, material: StrokeMaterial, color: [f32; 3], points: Vec<Vec3>) -> Stroke {
    Stroke {
        fill: None,
        material,
        group_id: None,
        layer_id: 1,
        id,
        color,
        radius: 0.075,
        brush: Default::default(),
        points: points
            .into_iter()
            .map(|position| Point {
                position,
                pressure: 1.0,
                tilt: None,
            })
            .collect(),
    }
}

fn fixture() -> Document {
    let curve = (-20..=20)
        .map(|step| {
            let x = step as f32 * 0.075;
            Vec3::new(x, 0.45 + (x * 2.0).sin() * 0.12, 0.0)
        })
        .collect();
    let emissive = (-12..=12)
        .map(|step| Vec3::new(step as f32 * 0.09, -0.45, 0.0))
        .collect();
    Document {
        strokes: vec![
            stroke(1, StrokeMaterial::Shaded, [0.72, 0.68, 0.26], curve),
            stroke(2, StrokeMaterial::Emissive, [0.98, 0.2, 0.08], emissive),
        ],
        ..Document::default()
    }
}

fn changed_pixels(before: &[u8], after: &[u8]) -> (usize, u8) {
    let mut count = 0;
    let mut max_delta = 0;
    for (a, b) in before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0)
    {
        let delta = a[..3]
            .iter()
            .copied()
            .zip(&b[..3])
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .unwrap_or(0);
        if delta > 2 {
            count += 1;
        }
        max_delta = max_delta.max(delta);
    }
    (count, max_delta)
}

fn render(
    renderer: &mut SceneRenderer,
    doc: &Document,
    camera: &Camera,
    size: [u32; 2],
) -> Result<Vec<u8>, String> {
    renderer.render_png_pixels(doc, camera, size, BACKGROUND, renderer.lighting)
}

fn render_canvas(
    renderer: &mut SceneRenderer,
    doc: &Document,
    camera: &Camera,
    size: [u32; 2],
) -> Vec<u8> {
    renderer.render(doc, camera, GuidePreview::Hidden, size);
    renderer.verification_pixels()
}

fn median(mut durations: Vec<Duration>) -> Duration {
    durations.sort_unstable();
    durations[durations.len() / 2]
}

fn measure(
    renderer: &mut SceneRenderer,
    doc: &Document,
    camera: &Camera,
    size: [u32; 2],
    effects_on: bool,
) -> Result<Duration, String> {
    renderer.grain = if effects_on { 0.12 } else { 0.0 };
    renderer.toon_shading = effects_on;
    renderer.post_effects = if effects_on {
        PostEffects {
            glow: 0.75,
            focus_distance: 50.0,
            depth_of_field: 16.0,
            pixel_size: 8.0,
        }
    } else {
        PostEffects::default()
    };
    for _ in 0..2 {
        let _ = render(renderer, doc, camera, size)?;
    }
    let mut durations = Vec::with_capacity(7);
    for _ in 0..7 {
        let started = Instant::now();
        let _ = render(renderer, doc, camera, size)?;
        durations.push(started.elapsed());
    }
    Ok(median(durations))
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
    let camera = Camera {
        projection: Projection::Orthographic,
        distance: 5.0,
        ..Camera::default()
    };
    let document = fixture();
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;

    let mut size_reports = Vec::new();
    for size in SIZES {
        renderer.grain = 0.0;
        renderer.toon_shading = false;
        renderer.post_effects = PostEffects::default();
        let baseline = render(&mut renderer, &document, &camera, size)?;
        let repeated = render(&mut renderer, &document, &camera, size)?;
        if baseline != repeated {
            return Err(format!(
                "Effect-off render is not deterministic at {size:?}"
            ));
        }
        let canvas_baseline = render_canvas(&mut renderer, &document, &camera, size);
        if canvas_baseline != baseline {
            return Err(format!(
                "Effect-off canvas and PNG output differ at {size:?}"
            ));
        }
        image::save_buffer(
            output.join(format!("effects-off-{}x{}.png", size[0], size[1])),
            &baseline,
            size[0],
            size[1],
            image::ColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;

        // Use a no-op screen pass as the reference for screen-space effects so
        // MSAA changes do not count as glow/blur/pixelation output.
        renderer.post_effects = PostEffects {
            focus_distance: 0.0,
            depth_of_field: 16.0,
            ..PostEffects::default()
        };
        let screen_pass_baseline = render(&mut renderer, &document, &camera, size)?;
        if screen_pass_baseline != render(&mut renderer, &document, &camera, size)? {
            return Err(format!(
                "No-op screen pass is not deterministic at {size:?}"
            ));
        }

        let cases = [
            ("grain", 0.12, false, PostEffects::default()),
            ("toon", 0.0, true, PostEffects::default()),
            (
                "glow",
                0.0,
                false,
                PostEffects {
                    glow: 1.0,
                    ..PostEffects::default()
                },
            ),
            (
                "depth-of-field",
                0.0,
                false,
                PostEffects {
                    focus_distance: 50.0,
                    depth_of_field: 16.0,
                    ..PostEffects::default()
                },
            ),
            (
                "pixelation",
                0.0,
                false,
                PostEffects {
                    pixel_size: 8.0,
                    ..PostEffects::default()
                },
            ),
        ];
        let mut effects_report = Vec::new();
        for (name, grain, toon, effects) in cases {
            renderer.grain = grain;
            renderer.toon_shading = toon;
            renderer.post_effects = effects;
            let pixels = render(&mut renderer, &document, &camera, size)?;
            let reference = if matches!(name, "glow" | "depth-of-field" | "pixelation") {
                &screen_pass_baseline
            } else {
                &baseline
            };
            let (changed, max_delta) = changed_pixels(reference, &pixels);
            if changed < 8 {
                return Err(format!(
                    "{name} changed only {changed} RGB pixels at {size:?}"
                ));
            }
            let repeat = render(&mut renderer, &document, &camera, size)?;
            if pixels != repeat {
                return Err(format!("{name} render is not deterministic at {size:?}"));
            }
            let canvas_pixels = render_canvas(&mut renderer, &document, &camera, size);
            if canvas_pixels != pixels {
                return Err(format!("{name} canvas and PNG output differ at {size:?}"));
            }
            image::save_buffer(
                output.join(format!("{name}-{}x{}.png", size[0], size[1])),
                &pixels,
                size[0],
                size[1],
                image::ColorType::Rgba8,
            )
            .map_err(|error| error.to_string())?;
            effects_report.push(json!({
                "effect": name,
                "changed_pixels_over_two_levels": changed,
                "max_rgb_channel_delta": max_delta,
                "repeat_exact": true,
            }));
        }

        renderer.grain = 0.12;
        renderer.toon_shading = true;
        renderer.post_effects = PostEffects {
            glow: 0.75,
            focus_distance: 50.0,
            depth_of_field: 16.0,
            pixel_size: 8.0,
        };
        let all_effects = render(&mut renderer, &document, &camera, size)?;
        let canvas_all_effects = render_canvas(&mut renderer, &document, &camera, size);
        let repeat = render(&mut renderer, &document, &camera, size)?;
        if all_effects == baseline || all_effects != repeat || canvas_all_effects != all_effects {
            return Err(format!(
                "Combined effects must change output deterministically and match canvas/PNG at {size:?}"
            ));
        }
        image::save_buffer(
            output.join(format!("all-effects-{}x{}.png", size[0], size[1])),
            &all_effects,
            size[0],
            size[1],
            image::ColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;

        let baseline_time = measure(&mut renderer, &document, &camera, size, false)?;
        let effects_time = measure(&mut renderer, &document, &camera, size, true)?;
        let delta_us = effects_time.as_micros() as i128 - baseline_time.as_micros() as i128;
        let percent = if baseline_time.is_zero() {
            0.0
        } else {
            (delta_us as f64 / baseline_time.as_micros() as f64) * 100.0
        };
        size_reports.push(json!({
            "size": size,
            "effects": effects_report,
            "combined_effects_changed_pixels": changed_pixels(&baseline, &all_effects).0,
            "combined_repeat_exact": true,
            "render_readback_median_us_effects_off": baseline_time.as_micros(),
            "render_readback_median_us_effects_on": effects_time.as_micros(),
            "render_readback_delta_us": delta_us,
            "render_readback_delta_percent": percent,
        }));
    }

    let report = json!({
        "passed": true,
        "checks": [
            "grain and toon each visibly change the multisampled baseline",
            "glow, depth-of-field, and pixelation each visibly change a no-op single-sample screen-pass reference",
            "each individual effect produces deterministic repeat output at both sizes",
            "effect-off and effect-on canvas pixels exactly match artwork PNG pixels at both sizes",
            "combined effects produce deterministic output at both sizes",
            "GPU-backed render and readback latency is measured at two resolutions with effects off and on",
        ],
        "adapter": adapter.name,
        "backend": format!("{:?}", adapter.backend),
        "measurement_method": "Median CPU wall time around render_png_pixels, including render setup, GPU queue work, and synchronous pixel readback; hardware-specific end-to-end latency, not isolated GPU shader time.",
        "samples_per_measurement": 7,
        "warmup_renders_per_measurement": 2,
        "timing_note": "Effect-on rendering enables grain/toon shading, disables the baseline multisample target, and adds a screen-space pass, so the measured delta reflects the actual renderer configuration rather than the isolated cost of only the screen-space pass.",
        "physical_hardware_tested": false,
        "sizes": size_reports,
    });
    std::fs::write(
        output.join("post-effects-report.json"),
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
