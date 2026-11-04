//! Planar SVG export for the active drawing plane.
//!
//! The SVG keeps the plane's world-unit scale and the document's layer order.
//! It exports solid vector paths and fill contours. Tube profiles, brush grain,
//! clipping masks, and depth compositing have no exact 2D SVG representation and
//! are intentionally reduced to centerline strokes and ordinary stacked paths.

use crate::{document::{Document, Stroke}, geometry::DrawingPlane};
use glam::{Vec2, Vec3};

pub const MAX_SVG_OUTPUT_POINTS: usize = 1_000_000;

#[derive(Clone, Copy)]
struct Frame {
    origin: Vec3,
    u: Vec3,
    v: Vec3,
}

impl Frame {
    fn new(plane: DrawingPlane) -> Self {
        let (u, v, _) = plane.basis();
        Self { origin: plane.origin(), u, v }
    }

    fn project(self, point: Vec3) -> Vec2 {
        let relative = point - self.origin;
        Vec2::new(relative.dot(self.u), -relative.dot(self.v))
    }
}

#[derive(Clone, Copy)]
struct Bounds { min: Vec2, max: Vec2 }

impl Bounds {
    fn include(&mut self, p: Vec2) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
}

/// Create an SVG document from visible strokes projected onto `plane`.
/// Projection is orthogonal to the plane; it does not depend on camera zoom.
pub fn build(document: &Document, plane: DrawingPlane) -> Result<String, String> {
    let frame = Frame::new(plane);
    let visible_layers: Vec<_> = document.layers.iter().filter(|layer| layer.visible && document.layer_folder_visible(layer.id)).collect();
    if visible_layers.is_empty() { return Err("The drawing has no visible layers to export.".into()); }
    let mut bounds = Bounds { min: Vec2::splat(f32::INFINITY), max: Vec2::splat(f32::NEG_INFINITY) };
    let mut point_count = 0usize;
    let mut stroke_margin = 0.0f32;
    for stroke in &document.strokes {
        if !visible_layers.iter().any(|layer| layer.id == stroke.layer_id) { continue; }
        if stroke.points.is_empty() { continue; }
        point_count = point_count.checked_add(stroke.points.len()).ok_or("SVG point count overflow")?;
        if point_count > MAX_SVG_OUTPUT_POINTS { return Err("SVG export exceeds the one million point limit.".into()); }
        for point in &stroke.points { bounds.include(frame.project(point.position)); }
        for hole in stroke.fill.iter().flat_map(|fill| fill.holes.iter()) {
            point_count = point_count.checked_add(hole.len()).ok_or("SVG point count overflow")?;
            if point_count > MAX_SVG_OUTPUT_POINTS { return Err("SVG export exceeds the one million point limit.".into()); }
            for point in hole { bounds.include(frame.project(*point)); }
        }
        stroke_margin = stroke_margin.max((stroke.radius * stroke.brush.thickness).max(0.0));
    }
    if !bounds.min.is_finite() || !bounds.max.is_finite() { return Err("There is no visible artwork to export.".into()); }
    bounds.min -= Vec2::splat(stroke_margin);
    bounds.max += Vec2::splat(stroke_margin);
    let mut size = bounds.max - bounds.min;
    if size.x < 1e-4 { bounds.min.x -= 0.5; size.x = 1.0; }
    if size.y < 1e-4 { bounds.min.y -= 0.5; size.y = 1.0; }

    let mut out = String::with_capacity(point_count.saturating_mul(32).min(16 * 1024 * 1024));
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" viewBox=\"{} {} {} {}\" width=\"{}\" height=\"{}\">\n", number(bounds.min.x), number(bounds.min.y), number(size.x), number(size.y), number(size.x), number(size.y)));
    out.push_str("<title>Sketch3D active-plane artwork</title>\n");
    for layer in visible_layers {
        out.push_str(&format!("<g id=\"layer-{}\" data-layer-name=\"{}\" opacity=\"{}\">\n", layer.id, xml_escape(&layer.name), number(layer.opacity)));
        for stroke in document.strokes.iter().filter(|stroke| stroke.layer_id == layer.id && !stroke.points.is_empty()) {
            write_stroke(&mut out, stroke, frame, bounds.min)?;
        }
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    Ok(out)
}

fn write_stroke(out: &mut String, stroke: &Stroke, frame: Frame, min: Vec2) -> Result<(), String> {
    let mut path = String::new();
    let mut put_contour = |points: &[Vec3], close: bool| {
        if points.is_empty() { return; }
        for (index, point) in points.iter().enumerate() {
            let p = frame.project(*point) - min;
            path.push_str(if index == 0 { "M " } else { " L " });
            path.push_str(&number(p.x)); path.push(' '); path.push_str(&number(p.y));
        }
        if close { path.push_str(" Z"); }
    };
    let points: Vec<Vec3> = stroke.points.iter().map(|point| point.position).collect();
    put_contour(&points, stroke.fill.is_some());
    if let Some(fill) = &stroke.fill {
        for hole in &fill.holes { put_contour(hole, true); }
        let color = mean_fill_color(stroke);
        out.push_str(&format!("<path d=\"{}\" fill=\"{}\" fill-opacity=\"{}\" fill-rule=\"evenodd\" stroke=\"none\"/>\n", path, rgb(color), number(stroke.brush.opacity)));
    } else {
        let width = (stroke.radius * 2.0 * stroke.brush.thickness).max(0.001);
        let color = mean_stroke_color(stroke);
        let opacity = mean_opacity(stroke);
        let cap = "round";
        out.push_str(&format!("<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"{}\" stroke-linecap=\"{}\" stroke-linejoin=\"round\"/>\n", path, rgb(color), number(opacity), number(width), cap));
    }
    Ok(())
}

fn mean_stroke_color(stroke: &Stroke) -> [f32; 3] {
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for (index, _point) in stroke.points.iter().enumerate() {
        let t = if stroke.points.len() <= 1 { 0.0 } else { index as f32 / (stroke.points.len() - 1) as f32 };
        let mut color = stroke.gradient.map_or(stroke.color, |gradient| gradient.sample(t));
        if let Some(Some(override_color)) = stroke.point_colors.as_ref().and_then(|values| values.get(index)) { color = *override_color; }
        for axis in 0..3 { sum[axis] += color[axis]; }
        count += 1.0;
    }
    std::array::from_fn(|axis| (sum[axis] / count.max(1.0)).clamp(0.0, 1.0))
}

fn mean_fill_color(stroke: &Stroke) -> [f32; 3] { mean_stroke_color(stroke) }

fn mean_opacity(stroke: &Stroke) -> f32 {
    let mapping = stroke.brush.dynamics.opacity;
    let mean = if stroke.points.is_empty() { 1.0 } else {
        stroke.points.iter().map(|point| {
            let pressure = point.pressure.clamp(0.0, 1.0);
            let response = if mapping.enabled {
                mapping.minimum + (1.0 - mapping.minimum) * pressure.powf(mapping.curve)
            } else { 1.0 };
            response
        }).sum::<f32>() / stroke.points.len() as f32
    };
    (stroke.brush.opacity * mean).clamp(0.0, 1.0)
}

fn rgb(color: [f32; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", (color[0].clamp(0.0, 1.0) * 255.0).round() as u8, (color[1].clamp(0.0, 1.0) * 255.0).round() as u8, (color[2].clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn number(value: f32) -> String {
    if !value.is_finite() { return "0".into(); }
    let value = if value.abs() < 0.0000005 { 0.0 } else { value };
    format!("{value:.6}").trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn xml_escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}
