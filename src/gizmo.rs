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
    let matrix = camera.matrix(rect.aspect_ratio());
    let mut best: Option<(f32, f32, u64)> = None;
    for stroke in &doc.strokes {
        let extent = crate::geometry::stroke_extent(stroke);
        let consider = |point: Vec3| -> Option<(Pos2, f32, f32)> {
            let (p, z) = project(camera, rect, point)?;
            let w = (matrix * point.extend(1.0)).w;
            let width = extent * rect.height() / (2.0 * 22.5f32.to_radians().tan() * w);
            Some((p, z, width))
        };
        let mut hit = |a: (Pos2, f32, f32), b: (Pos2, f32, f32)| {
            let (dist, t) = distance(pos, a.0, b.0);
            if dist > 8.0 + a.2.max(b.2) {
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
            if let (Some(a), Some(b)) = (consider(pair[0].position), consider(pair[1].position)) {
                hit(a, b);
            }
        }
    }
    best.map(|(_, _, id)| id)
}
impl Layout {
    pub fn new(camera: &Camera, rect: Rect, pivot: Vec3, mode: Mode) -> Option<Self> {
        let (origin, _) = project(camera, rect, pivot)?;
        let clip = camera.matrix(rect.aspect_ratio()) * pivot.extend(1.0);
        let length = 80.0 * 2.0 * 22.5f32.to_radians().tan() * clip.w / rect.height();
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
                handles.push(Handle {
                    kind: HandleKind::Scale,
                    path: vec![origin, origin + egui::vec2(58.0, -58.0)],
                    color: Color32::from_rgb(255, 210, 100),
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
                } else if h.kind == HandleKind::Scale {
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
            id,
            color: [0.5; 3],
            radius: 0.02,
            brush: Default::default(),
            points: vec![
                Point {
                    position: Vec3::new(-1.0, 0.0, z),
                    pressure: 1.0,
                },
                Point {
                    position: Vec3::new(1.0, 0.0, z),
                    pressure: 1.0,
                },
            ],
        };
        let doc = Document {
            version: 1,
            strokes: vec![stroke(1, 1.0), stroke(2, 0.0)],
        };
        assert_eq!(pick(&doc, &camera, rect, rect.center()), Some(1));
        assert_eq!(pick(&doc, &camera, rect, rect.min), None);
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
