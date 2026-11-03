//! Projection, depth-aware stroke picking, and screen-sized world-space handles.
use crate::{document::Document, geometry::Camera};
use eframe::egui::{self, Color32, Pos2, Rect};
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Move,
    Rotate,
    Scale,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HandleKind {
    Axis(usize),
    Free,
    Rotation(usize),
    Scale,
    ScaleAxis(usize),
    ScaleView,
}
pub struct Handle {
    pub kind: HandleKind,
    pub path: Vec<Pos2>,
    pub color: Color32,
}
pub struct Layout {
    pub pivot: Vec3,
    pub origin: Pos2,
    pub handles: Vec<Handle>,
}
pub fn axis(index: usize) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z][index]
}
pub fn project(camera: &Camera, rect: Rect, point: Vec3) -> Option<(Pos2, f32)> {
    let clip = camera.matrix(rect.aspect_ratio()) * point.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !(0.0..=1.0).contains(&ndc.z) {
        return None;
    }
    Some((
        egui::pos2(
            rect.left() + (ndc.x + 1.0) * 0.5 * rect.width(),
            rect.top() + (1.0 - ndc.y) * 0.5 * rect.height(),
        ),
        ndc.z,
    ))
}
pub fn ray(camera: &Camera, rect: Rect, pos: Pos2) -> (Vec3, Vec3) {
    camera.ray(
        glam::vec2(
            (pos.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (pos.y - rect.top()) / rect.height() * 2.0,
        ),
        rect.aspect_ratio(),
    )
}
pub fn plane_hit(
    camera: &Camera,
    rect: Rect,
    pos: Pos2,
    pivot: Vec3,
    normal: Vec3,
) -> Option<Vec3> {
    let (o, d) = ray(camera, rect, pos);
    let denominator = d.dot(normal);
    if denominator.abs() < 0.0001 {
        return None;
    }
    let t = (pivot - o).dot(normal) / denominator;
    (t >= 0.0).then_some(o + d * t)
}
pub fn axis_parameter(
    camera: &Camera,
    rect: Rect,
    pos: Pos2,
    pivot: Vec3,
    axis: Vec3,
) -> Option<f32> {
    let (o, d) = ray(camera, rect, pos);
    let dot = d.dot(axis);
    let denominator = 1.0 - dot * dot;
    if denominator < 0.005 {
        return None;
    }
    let r = o - pivot;
    Some((axis.dot(r) - dot * d.dot(r)) / denominator)
}
fn distance(pos: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let ab = b - a;
    let t = ((pos - a).dot(ab) / ab.length_sq().max(0.0001)).clamp(0.0, 1.0);
    (pos.distance(a + ab * t), t)
}
pub fn pick(doc: &Document, camera: &Camera, rect: Rect, pos: Pos2) -> Option<u64> {
    pick_filtered(doc, camera, rect, pos, true, 8.0, false)
        .ok()
        .flatten()
}

/// Pick visible artwork without requiring its layer to be editable.
/// Read-only tools such as color sampling can inspect locked layers.
pub fn pick_visible(doc: &Document, camera: &Camera, rect: Rect, pos: Pos2) -> Option<u64> {
    pick_filtered(doc, camera, rect, pos, false, 8.0, false)
        .ok()
        .flatten()
}

/// Resolve region witnesses without the cursor tolerance used for point picking.
pub fn pick_region_visible(
    doc: &Document,
    camera: &Camera,
    rect: Rect,
    pos: Pos2,
) -> Result<Option<u64>, &'static str> {
    pick_filtered(doc, camera, rect, pos, false, 0.0, true)
}

fn textured_surface_covered(
    stroke: &crate::document::Stroke,
    tip_asset: Option<&crate::brush_tip::BrushTipAsset>,
    origin: Vec3,
    direction: Vec3,
) -> Option<bool> {
    let mut nearest: Option<(f32, [f32; 2])> = None;
    for triangle in crate::geometry::stroke_mesh(stroke).as_chunks::<3>().0 {
        let a = Vec3::from(triangle[0].position);
        let b = Vec3::from(triangle[1].position);
        let c = Vec3::from(triangle[2].position);
        let edge_ab = b - a;
        let edge_ac = c - a;
        let cross = direction.cross(edge_ac);
        let determinant = edge_ab.dot(cross);
        if determinant.abs() < 1.0e-7 {
            continue;
        }
        let inverse = determinant.recip();
        let offset = origin - a;
        let u = offset.dot(cross) * inverse;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = offset.cross(edge_ab);
        let v = direction.dot(q) * inverse;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let distance = edge_ac.dot(q) * inverse;
        if distance < 0.0 || nearest.is_some_and(|(best, _)| distance >= best) {
            continue;
        }
        let w = 1.0 - u - v;
        let uv = [
            triangle[0].uv[0] * w + triangle[1].uv[0] * u + triangle[2].uv[0] * v,
            triangle[0].uv[1] * w + triangle[1].uv[1] * u + triangle[2].uv[1] * v,
        ];
        let opacity = triangle[0].opacity * w + triangle[1].opacity * u + triangle[2].opacity * v;
        if stroke.brush.flow.enabled
            && (opacity <= 0.001
                || (stroke.brush.texture.kind != crate::texture::TextureKind::Solid
                    && !crate::texture::covered_at_uv(stroke.brush.texture, uv)))
        {
            continue;
        }
        nearest = Some((distance, uv));
    }
    let Some((_, uv)) = nearest else {
        return Some(false);
    };
    if stroke.brush.flow.enabled && stroke.brush.texture.image.is_none() {
        return Some(true);
    }
    if stroke.brush.tip_asset_id != 0 {
        return Some(
            tip_asset
                .and_then(|asset| asset.alpha_at_uv(uv).ok())
                .is_some_and(|alpha| alpha > 0.001),
        );
    }
    if let Some(settings) = stroke.brush.texture.image {
        return Some(
            tip_asset
                .and_then(|asset| asset.repeating_coverage_at_uv(settings, uv).ok())
                .is_some_and(|coverage| coverage > 0.001),
        );
    }
    Some(crate::texture::covered_at_uv(stroke.brush.texture, uv))
}

fn pick_filtered(
    doc: &Document,
    camera: &Camera,
    rect: Rect,
    pos: Pos2,
    editable_only: bool,
    pick_slop_px: f32,
    require_visible_coverage: bool,
) -> Result<Option<u64>, &'static str> {
    let mut best: Option<(f32, f32, u64)> = None;
    let (origin, direction) = ray(camera, rect, pos);
    for stroke in doc.strokes.iter().filter(|stroke| {
        let selectable = if editable_only {
            doc.editable(stroke)
        } else {
            doc.visible(stroke)
        };
        selectable
            && (!require_visible_coverage
                || (stroke.brush.opacity > 0.001 && doc.layer_opacity(stroke.layer_id) > 0.001))
    }) {
        let evaluated_copies = match crate::stroke_modifiers::evaluated_strokes(stroke) {
            Ok(copies) => copies,
            Err(_) if require_visible_coverage => {
                return Err("Region selection cannot evaluate visible occlusion geometry")
            }
            Err(_) => continue,
        };
        for evaluated in evaluated_copies {
            let stroke = evaluated.as_ref();
            let mesh_covered = stroke.fill.is_none()
                && (stroke.brush.flow.enabled
                    || stroke.brush.strands.enabled
                    || stroke.brush.dabs.enabled
                    || stroke.brush.tip_asset_id != 0
                    || stroke.brush.texture.image.is_some()
                    || (stroke.brush.texture.kind != crate::texture::TextureKind::Solid
                        && stroke.brush.texture.strength > 0.0));
            if mesh_covered {
                let extent = crate::geometry::stroke_extent(stroke);
                let close = stroke.points.iter().any(|point| {
                    project(camera, rect, point.position).is_some_and(|(screen, _)| {
                        let width = extent / camera.world_per_pixel(point.position, rect.height());
                        screen.distance(pos) <= pick_slop_px + width
                    })
                }) || stroke.points.windows(2).any(|pair| {
                    let (Some((a, _)), Some((b, _))) = (
                        project(camera, rect, pair[0].position),
                        project(camera, rect, pair[1].position),
                    ) else {
                        return false;
                    };
                    let width = extent
                        / camera
                            .world_per_pixel(pair[0].position, rect.height())
                            .min(camera.world_per_pixel(pair[1].position, rect.height()));
                    distance(pos, a, b).0 <= pick_slop_px + width
                });
                let tip_asset = doc.brush_tip_assets.iter().find(|asset| {
                    asset.id
                        == if stroke.brush.tip_asset_id != 0 {
                            stroke.brush.tip_asset_id
                        } else {
                            stroke.brush.texture.image.map_or(0, |image| image.asset_id)
                        }
                });
                if !close
                    || textured_surface_covered(stroke, tip_asset, origin, direction) == Some(false)
                {
                    continue;
                }
            }
            if stroke.fill.is_some() {
                if let Some(hit) = crate::polygon::ray_hit(stroke, origin, direction) {
                    let texture_visible =
                        stroke
                            .fill
                            .and_then(|fill| fill.texture)
                            .is_none_or(|texture| {
                                let Some(asset) = doc
                                    .brush_tip_assets
                                    .iter()
                                    .find(|asset| asset.id == texture.asset_id)
                                else {
                                    return false;
                                };
                                let Some(mut uv) = crate::polygon::mapped_fill_uv(stroke, hit)
                                else {
                                    return false;
                                };
                                if texture.repeat {
                                    uv = uv - uv.floor();
                                } else {
                                    uv = uv.clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
                                }
                                asset
                                    .alpha_at_uv(uv.to_array())
                                    .is_ok_and(|alpha| alpha > 0.01)
                            });
                    if !texture_visible {
                        continue;
                    }
                    if let Some((_, depth)) = project(camera, rect, hit) {
                        if best.is_none_or(|old| depth < old.0) {
                            best = Some((depth, 0.0, stroke.id));
                        }
                    }
                }
                continue;
            }
            let extent = crate::geometry::stroke_extent(stroke);
            let consider = |point: Vec3| -> Option<(Pos2, f32, f32)> {
                let (p, z) = project(camera, rect, point)?;
                let width = extent / camera.world_per_pixel(point, rect.height());
                Some((p, z, width))
            };
            let mut hit = |a: (Pos2, f32, f32), b: (Pos2, f32, f32)| {
                let (dist, t) = distance(pos, a.0, b.0);
                if dist > pick_slop_px + a.2.max(b.2) {
                    return;
                }
                let depth = a.1 + (b.1 - a.1) * t;
                let candidate = (depth, dist, stroke.id);
                if best.is_none_or(|old| {
                    candidate.0 < old.0 - 0.000001
                        || ((candidate.0 - old.0).abs() < 0.000001 && candidate.1 < old.1)
                }) {
                    best = Some(candidate);
                }
            };
            if stroke.points.len() == 1 {
                if let Some(a) = consider(stroke.points[0].position) {
                    hit(a, a);
                }
            }
            for pair in stroke.points.windows(2) {
                if let (Some(a), Some(b)) = (consider(pair[0].position), consider(pair[1].position))
                {
                    hit(a, b);
                }
            }
        }
    }
    Ok(best.map(|(_, _, id)| id))
}
impl Layout {
    pub fn new(camera: &Camera, rect: Rect, pivot: Vec3, mode: Mode) -> Option<Self> {
        let (origin, _) = project(camera, rect, pivot)?;
        let length = 80.0 * camera.world_per_pixel(pivot, rect.height());
        let colors = [
            Color32::from_rgb(250, 104, 104),
            Color32::from_rgb(115, 224, 139),
            Color32::from_rgb(110, 164, 255),
        ];
        let mut handles = Vec::new();
        match mode {
            Mode::Move => {
                for (i, color) in colors.into_iter().enumerate() {
                    if let Some((end, _)) = project(camera, rect, pivot + axis(i) * length) {
                        let delta = end - origin;
                        if delta.length() > 18.0 {
                            let start = origin + delta.normalized() * 13.0;
                            handles.push(Handle {
                                kind: HandleKind::Axis(i),
                                path: vec![start, end],
                                color,
                            });
                        }
                    }
                }
                handles.push(Handle {
                    kind: HandleKind::Free,
                    path: vec![origin],
                    color: Color32::WHITE,
                });
            }
            Mode::Rotate => {
                let facing = (camera.eye() - pivot).normalize();
                for (i, color) in colors.into_iter().enumerate() {
                    if facing.dot(axis(i)).abs() < 0.12 {
                        continue;
                    }
                    let u = axis((i + 1) % 3);
                    let v = axis(i).cross(u);
                    let path = (0..=64)
                        .filter_map(|j| {
                            let angle = j as f32 / 64.0 * std::f32::consts::TAU;
                            project(
                                camera,
                                rect,
                                pivot + (u * angle.cos() + v * angle.sin()) * length * 0.85,
                            )
                            .map(|(p, _)| p)
                        })
                        .collect();
                    handles.push(Handle {
                        kind: HandleKind::Rotation(i),
                        path,
                        color,
                    });
                }
            }
            Mode::Scale => {
                // Keep the established uniform handle first for stable hit
                // ordering and compatibility with saved interaction scripts.
                handles.push(Handle {
                    kind: HandleKind::Scale,
                    path: vec![origin, origin + egui::vec2(58.0, -58.0)],
                    color: Color32::from_rgb(255, 210, 100),
                });
                for (i, color) in colors.into_iter().enumerate() {
                    if let Some((end, _)) = project(camera, rect, pivot + axis(i) * length) {
                        let delta = end - origin;
                        if delta.length() > 18.0 {
                            handles.push(Handle {
                                kind: HandleKind::ScaleAxis(i),
                                path: vec![origin + delta.normalized() * 12.0, end],
                                color,
                            });
                        }
                    }
                }
                handles.push(Handle {
                    kind: HandleKind::ScaleView,
                    path: vec![origin, origin + egui::vec2(-34.0, -34.0)],
                    color: Color32::from_rgb(115, 224, 220),
                });
            }
        }
        Some(Self {
            pivot,
            origin,
            handles,
        })
    }
    pub fn hit(&self, pos: Pos2) -> Option<HandleKind> {
        self.handles
            .iter()
            .filter_map(|h| {
                let d = if h.path.len() == 1 {
                    h.path[0].distance(pos)
                } else if matches!(h.kind, HandleKind::Scale | HandleKind::ScaleView) {
                    h.path.last()?.distance(pos)
                } else {
                    h.path
                        .windows(2)
                        .map(|p| distance(pos, p[0], p[1]).0)
                        .fold(f32::INFINITY, f32::min)
                };
                (d < 10.0).then_some((d, h.kind))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, kind)| kind)
    }
    pub fn paint(&self, painter: &egui::Painter, active: Option<HandleKind>) {
        for handle in &self.handles {
            let color = if active == Some(handle.kind) {
                Color32::WHITE
            } else {
                handle.color
            };
            let stroke = egui::Stroke::new(2.5_f32, color);
            match handle.kind {
                HandleKind::Axis(i) => {
                    let a = handle.path[0];
                    let b = handle.path[1];
                    painter.arrow(a, b - a, stroke);
                    painter.text(
                        b + egui::vec2(8.0, -8.0),
                        egui::Align2::CENTER_CENTER,
                        ["X", "Y", "Z"][i],
                        egui::FontId::proportional(14.0),
                        color,
                    );
                }
                HandleKind::Free => {
                    painter.rect_filled(
                        Rect::from_center_size(self.origin, egui::vec2(12.0, 12.0)),
                        2.0,
                        Color32::from_rgb(31, 45, 58),
                    );
                    painter.rect_stroke(
                        Rect::from_center_size(self.origin, egui::vec2(12.0, 12.0)),
                        2.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                HandleKind::Rotation(i) => {
                    painter.add(egui::Shape::line(handle.path.clone(), stroke));
                    if let Some(p) = handle.path.get(8) {
                        painter.text(
                            *p,
                            egui::Align2::CENTER_CENTER,
                            ["X", "Y", "Z"][i],
                            egui::FontId::proportional(14.0),
                            color,
                        );
                    }
                }
                HandleKind::Scale => {
                    painter.line_segment([self.origin, handle.path[1]], stroke);
                    painter.rect_filled(
                        Rect::from_center_size(handle.path[1], egui::vec2(13.0, 13.0)),
                        1.0,
                        color,
                    );
                    painter.text(
                        handle.path[1] + egui::vec2(0.0, -18.0),
                        egui::Align2::CENTER_CENTER,
                        "Uniform scale",
                        egui::FontId::proportional(12.0),
                        color,
                    );
                }
                HandleKind::ScaleAxis(i) => {
                    let a = handle.path[0];
                    let b = handle.path[1];
                    painter.line_segment([a, b], stroke);
                    painter.circle_filled(b, 5.0, color);
                    painter.text(
                        b + egui::vec2(8.0, -8.0),
                        egui::Align2::CENTER_CENTER,
                        ["SX", "SY", "SZ"][i],
                        egui::FontId::proportional(12.0),
                        color,
                    );
                }
                HandleKind::ScaleView => {
                    painter.line_segment([self.origin, handle.path[1]], stroke);
                    painter.rect_filled(
                        Rect::from_center_size(handle.path[1], egui::vec2(11.0, 11.0)),
                        1.0,
                        color,
                    );
                    painter.text(
                        handle.path[1] + egui::vec2(-6.0, -16.0),
                        egui::Align2::CENTER_CENTER,
                        "View",
                        egui::FontId::proportional(11.0),
                        color,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Point, Stroke},
        geometry::Plane,
    };
    #[test]
    fn picking_prefers_front_stroke_not_insertion_order() {
        let mut camera = Camera::default();
        camera.align(Plane::Front);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let stroke = |id, z| Stroke {
            model_surface: None,
            gradient: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id,
            color: [0.5; 3],
            radius: 0.02,
            brush: Default::default(),
            points: vec![
                Point {
                    tilt: None,
                    position: Vec3::new(-1.0, 0.0, z),
                    pressure: 1.0,

                    time_ms: 0,
                },
                Point {
                    tilt: None,
                    position: Vec3::new(1.0, 0.0, z),
                    pressure: 1.0,

                    time_ms: 0,
                },
            ],
        };
        let doc = Document {
            linked_stroke_groups: Default::default(),
            version: 1,
            strokes: vec![stroke(1, 1.0), stroke(2, 0.0)],
            ..Document::default()
        };
        assert_eq!(pick(&doc, &camera, rect, rect.center()), Some(1));
        assert_eq!(pick(&doc, &camera, rect, rect.min), None);
    }
    #[test]
    fn textured_stroke_picker_skips_transparent_mask_holes() {
        let mut camera = Camera::default();
        camera.align(Plane::Front);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut mark = Stroke {
            model_surface: None,
            gradient: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id: 41,
            color: [0.5; 3],
            radius: 0.12,
            brush: Default::default(),
            points: vec![
                Point {
                    tilt: None,
                    position: Vec3::new(-1.0, 0.0, 0.0),
                    pressure: 1.0,

                    time_ms: 0,
                },
                Point {
                    tilt: None,
                    position: Vec3::new(1.0, 0.0, 0.0),
                    pressure: 1.0,

                    time_ms: 0,
                },
            ],
        };
        mark.brush.texture = crate::texture::TextureSettings {
            kind: crate::texture::TextureKind::Grain,
            strength: 0.9,
            seed: 37,
            ..Default::default()
        };
        for point in &mut mark.points {
            point.position.z = 1.0;
        }
        let mut behind = mark.clone();
        behind.id = 42;
        behind.brush.texture = Default::default();
        for point in &mut behind.points {
            point.position.z = 0.0;
        }
        let doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![mark.clone(), behind.clone()],
            ..Document::default()
        };
        let mut transparent_pick = false;
        let mut covered_pick = false;
        for triangle in crate::geometry::stroke_mesh(&mark).as_chunks::<3>().0 {
            let position = (Vec3::from(triangle[0].position)
                + Vec3::from(triangle[1].position)
                + Vec3::from(triangle[2].position))
                / 3.0;
            let screen = project(&camera, rect, position).unwrap().0;
            let (origin, direction) = ray(&camera, rect, screen);
            match textured_surface_covered(&mark, None, origin, direction) {
                Some(false) => {
                    transparent_pick |= pick(&doc, &camera, rect, screen) == Some(behind.id);
                }
                Some(true) => {
                    covered_pick |= pick(&doc, &camera, rect, screen) == Some(mark.id);
                }
                None => {}
            }
            if transparent_pick && covered_pick {
                break;
            }
        }
        assert!(
            transparent_pick,
            "transparent mask holes reveal the stroke behind them"
        );
        assert!(covered_pick, "covered textured surface remains pickable");
    }

    #[test]
    fn dab_stroke_picker_skips_gaps_between_visible_marks() {
        let mut camera = Camera::default();
        camera.align(Plane::Front);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let line = |id, z| Stroke {
            model_surface: None,
            gradient: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id,
            color: [0.5; 3],
            radius: 0.1,
            brush: Default::default(),
            points: vec![
                Point {
                    position: Vec3::new(-1.0, 0.0, z),
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                },
                Point {
                    position: Vec3::new(1.0, 0.0, z),
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                },
            ],
        };
        let mut dabs = line(1, 1.0);
        dabs.brush.dabs.enabled = true;
        dabs.brush.dabs.spacing = 4.0;
        dabs.brush.dab_seed = crate::dab_placement::seed_for_stroke(dabs.id);
        let behind = line(2, 0.0);
        let doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![dabs, behind],
            ..Document::default()
        };

        let gap = project(&camera, rect, Vec3::new(-0.8, 0.0, 1.0)).unwrap().0;
        let covered = project(&camera, rect, Vec3::new(-0.95, 0.0, 1.0))
            .unwrap()
            .0;
        assert_eq!(pick(&doc, &camera, rect, gap), Some(2));
        assert_eq!(pick(&doc, &camera, rect, covered), Some(1));
    }
    #[test]
    fn axis_drag_parameters_follow_world_coordinates() {
        let mut camera = Camera::default();
        camera.align(Plane::Front);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let (pos, _) = project(&camera, rect, Vec3::new(2.0, 0.0, 0.0)).unwrap();
        assert!(
            (axis_parameter(&camera, rect, pos, Vec3::ZERO, Vec3::X).unwrap() - 2.0).abs() < 0.001
        );
        assert!(axis_parameter(&camera, rect, rect.center(), Vec3::ZERO, Vec3::Z).is_none());
    }
}

#[cfg(test)]
mod layer_picking_tests {
    use super::*;
    use crate::document::{Brush, Layer, Point, Stroke};
    use crate::geometry::Plane;
    #[test]
    fn protected_front_layers_do_not_intercept_editable_ink() {
        let mut camera = Camera::default();
        camera.align(Plane::Front);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mark = |id, layer_id, z| Stroke {
            model_surface: None,
            gradient: None,
            curve: None,
            material: Default::default(),
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            id,
            layer_id,
            color: [0.5; 3],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![
                Point {
                    position: Vec3::new(-1.0, 0.0, z),
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                },
                Point {
                    position: Vec3::new(1.0, 0.0, z),
                    pressure: 1.0,
                    tilt: None,

                    time_ms: 0,
                },
            ],
        };
        let mut doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![mark(1, 1, 1.0), mark(2, 2, 0.0)],
            ..Document::default()
        };
        doc.layers.push(Layer {
            id: 2,
            name: "Back".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: crate::document::LayerBlendMode::Normal,
        });
        assert_eq!(pick(&doc, &camera, rect, rect.center()), Some(1));
        assert_eq!(pick_visible(&doc, &camera, rect, rect.center()), Some(1));
        doc.layers[0].locked = true;
        assert_eq!(pick(&doc, &camera, rect, rect.center()), Some(2));
        assert_eq!(pick_visible(&doc, &camera, rect, rect.center()), Some(1));
        doc.layers[0].visible = false;
        assert_eq!(pick(&doc, &camera, rect, rect.center()), Some(2));
        assert_eq!(pick_visible(&doc, &camera, rect, rect.center()), Some(2));
        doc.layers[1].locked = true;
        assert_eq!(pick(&doc, &camera, rect, rect.center()), None);
        assert_eq!(pick_visible(&doc, &camera, rect, rect.center()), Some(2));
    }
}
