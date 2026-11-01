//! Headless GPU checks for screen-space presentation background images.
use crate::{
    document::Document,
    geometry::{Camera, Projection},
    lighting::Lighting,
    renderer::{BackgroundImageFit, SceneRenderer},
};
use eframe::egui_wgpu;
use image::AnimationDecoder;
use serde_json::json;
use std::{
    future::Future,
    path::Path,
    sync::atomic::AtomicBool,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    thread,
};

const LANDSCAPE: [u32; 2] = [384, 256];
const PORTRAIT: [u32; 2] = [256, 384];
const BACKGROUND: [f32; 3] = [0.08, 0.12, 0.2];
const SOURCE: [u8; 4] = [204, 72, 36, 255];

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

fn pixels(size: [u32; 2]) -> Vec<u8> {
    SOURCE.repeat((size[0] * size[1]) as usize)
}

fn pixel(data: &[u8], size: [u32; 2], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * size[0] + x) * 4) as usize;
    data[offset..offset + 4].try_into().unwrap()
}

fn check_rgb(name: &str, actual: [u8; 4], expected: [u8; 3], tolerance: u8) -> Result<(), String> {
    let error = actual[..3]
        .iter()
        .copied()
        .zip(expected)
        .map(|(actual, expected)| actual.abs_diff(expected))
        .max()
        .unwrap_or(0);
    if error > tolerance {
        return Err(format!(
            "{name} differs from expected RGB by {error} levels: {actual:?}"
        ));
    }
    Ok(())
}

fn save(path: &Path, size: [u32; 2], rgba: &[u8]) -> Result<(), String> {
    image::save_buffer(path, rgba, size[0], size[1], image::ColorType::Rgba8)
        .map_err(|error| error.to_string())
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
    let camera = Camera {
        projection: Projection::Orthographic,
        distance: 5.0,
        ..Camera::default()
    };
    let document = Document::default();
    let source_size = [4, 4];
    let source_pixels = pixels(source_size);
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;

    renderer.set_screen_background_image(
        5101,
        &source_pixels,
        source_size,
        BackgroundImageFit::Fit,
        [0.5, 0.5],
        1.0,
    )?;
    let fit_landscape =
        renderer.render_png_pixels(&document, &camera, LANDSCAPE, BACKGROUND, renderer.lighting)?;
    check_rgb(
        "landscape Fit image center",
        pixel(
            &fit_landscape,
            LANDSCAPE,
            LANDSCAPE[0] / 2,
            LANDSCAPE[1] / 2,
        ),
        SOURCE[..3].try_into().unwrap(),
        2,
    )?;
    let canvas_rgb = BACKGROUND.map(|channel| (channel * 255.0).round() as u8);
    check_rgb(
        "landscape Fit margin",
        pixel(&fit_landscape, LANDSCAPE, 4, LANDSCAPE[1] / 2),
        canvas_rgb,
        2,
    )?;
    save(
        &output.join("background-fit-landscape.png"),
        LANDSCAPE,
        &fit_landscape,
    )?;

    let fit_portrait =
        renderer.render_png_pixels(&document, &camera, PORTRAIT, BACKGROUND, renderer.lighting)?;
    check_rgb(
        "portrait Fit image center",
        pixel(&fit_portrait, PORTRAIT, PORTRAIT[0] / 2, PORTRAIT[1] / 2),
        SOURCE[..3].try_into().unwrap(),
        2,
    )?;
    check_rgb(
        "portrait Fit margin",
        pixel(&fit_portrait, PORTRAIT, PORTRAIT[0] / 2, 4),
        canvas_rgb,
        2,
    )?;
    save(
        &output.join("background-fit-portrait.png"),
        PORTRAIT,
        &fit_portrait,
    )?;

    renderer.set_screen_background_image(
        5101,
        &source_pixels,
        source_size,
        BackgroundImageFit::Fill,
        [0.5, 0.5],
        1.0,
    )?;
    let fill_landscape =
        renderer.render_png_pixels(&document, &camera, LANDSCAPE, BACKGROUND, renderer.lighting)?;
    check_rgb(
        "landscape Fill corner",
        pixel(&fill_landscape, LANDSCAPE, 4, LANDSCAPE[1] / 2),
        SOURCE[..3].try_into().unwrap(),
        2,
    )?;
    save(
        &output.join("background-fill-landscape.png"),
        LANDSCAPE,
        &fill_landscape,
    )?;

    let fill_portrait =
        renderer.render_png_pixels(&document, &camera, PORTRAIT, BACKGROUND, renderer.lighting)?;
    check_rgb(
        "portrait Fill corner",
        pixel(&fill_portrait, PORTRAIT, PORTRAIT[0] / 2, 4),
        SOURCE[..3].try_into().unwrap(),
        2,
    )?;
    save(
        &output.join("background-fill-portrait.png"),
        PORTRAIT,
        &fill_portrait,
    )?;

    renderer.set_screen_background_image(
        5101,
        &source_pixels,
        source_size,
        BackgroundImageFit::Fit,
        [0.5, 0.5],
        0.5,
    )?;
    let opacity =
        renderer.render_png_pixels(&document, &camera, LANDSCAPE, BACKGROUND, renderer.lighting)?;
    let expected_opacity_rgb = std::array::from_fn(|channel| {
        let source = crate::presentation::srgb_to_linear(f32::from(SOURCE[channel]) / 255.0);
        let canvas = crate::presentation::srgb_to_linear(BACKGROUND[channel]);
        ((crate::presentation::linear_to_srgb(source * 0.5 + canvas * 0.5) * 255.0).round()) as u8
    });
    check_rgb(
        "half-opacity background",
        pixel(&opacity, LANDSCAPE, LANDSCAPE[0] / 2, LANDSCAPE[1] / 2),
        expected_opacity_rgb,
        3,
    )?;
    save(
        &output.join("background-opacity-half.png"),
        LANDSCAPE,
        &opacity,
    )?;

    let transparent =
        renderer.render_transparent_png_pixels(&document, &camera, LANDSCAPE, renderer.lighting)?;
    if transparent
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] != 0)
    {
        return Err("Transparent PNG included the screen-space background image".into());
    }
    save(
        &output.join("background-excluded-transparent.png"),
        LANDSCAPE,
        &transparent,
    )?;

    let gif_path = output.join("background-fit.gif");
    let (frame_sender, frame_receiver) = std::sync::mpsc::channel();
    frame_sender
        .send(Some(fit_landscape.clone()))
        .map_err(|error| error.to_string())?;
    crate::app::video_export_ui::encode_gif(
        frame_receiver,
        &gif_path,
        LANDSCAPE,
        1,
        1,
        &AtomicBool::new(false),
    )?;
    drop(frame_sender);
    let frames = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
        std::fs::File::open(&gif_path).map_err(|error| error.to_string())?,
    ))
    .map_err(|error| error.to_string())?
    .into_frames()
    .collect_frames()
    .map_err(|error| error.to_string())?;
    let gif_frame = frames
        .first()
        .ok_or("GIF background verification produced no frames")?
        .buffer();
    let gif_pixel = gif_frame.get_pixel(LANDSCAPE[0] / 2, LANDSCAPE[1] / 2).0;
    check_rgb(
        "GIF presentation background",
        gif_pixel,
        SOURCE[..3].try_into().unwrap(),
        5,
    )?;

    let report = json!({
        "passed": true,
        "adapter": adapter.name,
        "backend": format!("{:?}", adapter.backend),
        "checks": [
            "Fit leaves canvas-color margins and keeps image centered at landscape and portrait sizes",
            "Fill covers the viewport at landscape and portrait sizes",
            "background opacity blends in linear light over the canvas color",
            "opaque PNG rendering includes the presentation image",
            "production GIF encoder preserves the opaque presentation background",
            "transparent PNG rendering excludes the presentation image",
        ],
        "source_color": SOURCE,
        "canvas_color": BACKGROUND,
        "expected_half_opacity_rgb": expected_opacity_rgb,
        "gif_background": {
            "frames": frames.len(),
            "center_rgb": [gif_pixel[0], gif_pixel[1], gif_pixel[2]],
            "artifact": "background-fit.gif",
        },
        "artifacts": [
            "background-fit-landscape.png",
            "background-fit-portrait.png",
            "background-fill-landscape.png",
            "background-fill-portrait.png",
            "background-opacity-half.png",
            "background-fit.gif",
            "background-excluded-transparent.png",
        ],
    });
    let report_path = output.join("presentation-background-report.json");
    std::fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "Presentation background visual checks passed; report: {}",
        report_path.display()
    );
    Ok(())
}
