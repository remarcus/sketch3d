//! Bounded planar circle and ellipse geometry for ordinary editable strokes.
#![allow(clippy::too_many_arguments)] // Explicit geometry parameters keep the public builders unambiguous.
use crate::{
    document::{Brush, Point, Stroke},
    geometry::DrawingPlane,
};
use glam::Vec3;
use std::f32::consts::TAU;

const MIN_SEGMENTS: usize = 24;
const MAX_SEGMENTS: usize = 512;
const DEFAULT_SEGMENTS: usize = 128;
const MAX_WORLD_RADIUS: f32 = 10_000.0;

/// Builds a circle on `plane`, projecting `center` onto that plane first.
pub fn circle_from_center(
    plane: DrawingPlane,
    center: Vec3,
    radius: f32,
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
) -> Result<Stroke, String> {
    validate_common(plane, color, stroke_radius, pressure, brush)?;
    if !radius.is_finite() || !(0.001..=MAX_WORLD_RADIUS).contains(&radius) {
        return Err("Circle radius must be between 0.001 and 10000 world units".into());
    }
    let (origin, u, v, normal) = plane_frame(plane)?;
    let center = project_to_plane(center, origin, u, v)?;
    create_ellipse(
        center,
        radius,
        radius,
        u,
        v,
        normal,
        id,
        layer_id,
        color,
        stroke_radius,
        pressure,
        brush,
        DEFAULT_SEGMENTS,
    )
}

/// Builds an ellipse on `plane`, projecting its center onto that plane.
pub fn ellipse_from_center(
    plane: DrawingPlane,
    center: Vec3,
    radius_u: f32,
    radius_v: f32,
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
) -> Result<Stroke, String> {
    validate_common(plane, color, stroke_radius, pressure, brush)?;
    validate_extent(radius_u, "Horizontal")?;
    validate_extent(radius_v, "Vertical")?;
    let (origin, u, v, normal) = plane_frame(plane)?;
    let center = project_to_plane(center, origin, u, v)?;
    create_ellipse(
        center,
        radius_u,
        radius_v,
        u,
        v,
        normal,
        id,
        layer_id,
        color,
        stroke_radius,
        pressure,
        brush,
        DEFAULT_SEGMENTS,
    )
}

/// Builds a circle using the distance between `center` and a point on its edge.
pub fn circle_from_center_edge(
    plane: DrawingPlane,
    center: Vec3,
    edge: Vec3,
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
) -> Result<Stroke, String> {
    let (origin, u, v, _) = plane_frame(plane)?;
    let center_on_plane = project_to_plane(center, origin, u, v)?;
    let edge_on_plane = project_to_plane(edge, origin, u, v)?;
    let radius = center_on_plane.distance(edge_on_plane);
    circle_from_center(
        plane,
        center_on_plane,
        radius,
        id,
        layer_id,
        color,
        stroke_radius,
        pressure,
        brush,
    )
}

/// Builds an ellipse from opposite corners of a bounding box expressed in
/// world space. The corners are projected to the drawing plane and the ellipse
/// is aligned with the plane's local U/V axes.
pub fn ellipse_from_bounds(
    plane: DrawingPlane,
    first: Vec3,
    opposite: Vec3,
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
) -> Result<Stroke, String> {
    validate_common(plane, color, stroke_radius, pressure, brush)?;
    let (origin, u, v, normal) = plane_frame(plane)?;
    let first = project_to_plane(first, origin, u, v)?;
    let opposite = project_to_plane(opposite, origin, u, v)?;
    let delta = opposite - first;
    let center = (first + opposite) * 0.5;
    let radius_u = delta.dot(u).abs() * 0.5;
    let radius_v = delta.dot(v).abs() * 0.5;
    validate_extent(radius_u, "Horizontal")?;
    validate_extent(radius_v, "Vertical")?;
    create_ellipse(
        center,
        radius_u,
        radius_v,
        u,
        v,
        normal,
        id,
        layer_id,
        color,
        stroke_radius,
        pressure,
        brush,
        DEFAULT_SEGMENTS,
    )
}

/// Creates a valid ordinary stroke with uniformly sampled pressure. The first
/// point is repeated once as a closed-loop marker; the mesh builder joins the
/// final segment without adding overlapping endpoint caps.
pub fn create_ellipse(
    center: Vec3,
    radius_u: f32,
    radius_v: f32,
    axis_u: Vec3,
    axis_v: Vec3,
    normal: Vec3,
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
    segments: usize,
) -> Result<Stroke, String> {
    if !center.is_finite() || center.abs().max_element() > MAX_WORLD_RADIUS {
        return Err("Shape center is outside document bounds".into());
    }
    validate_extent(radius_u, "Horizontal")?;
    validate_extent(radius_v, "Vertical")?;
    if !axis_u.is_finite()
        || !axis_v.is_finite()
        || !normal.is_finite()
        || (axis_u.length_squared() - 1.0).abs() > 0.001
        || (axis_v.length_squared() - 1.0).abs() > 0.001
        || (normal.length_squared() - 1.0).abs() > 0.001
        || axis_u.dot(axis_v).abs() > 0.001
        || axis_u.dot(normal).abs() > 0.001
        || axis_v.dot(normal).abs() > 0.001
    {
        return Err("Shape axes must form a finite orthonormal frame".into());
    }
    validate_common_values(color, stroke_radius, pressure, brush)?;
    let count = segments.clamp(MIN_SEGMENTS, MAX_SEGMENTS);
    let mut frame_brush = Brush {
        normal,
        tangent: axis_u,
        ..brush
    };
    frame_brush.dabs.enabled = false;
    if !frame_brush.is_valid() {
        return Err("Brush settings are invalid for the shape's drawing plane".into());
    }
    let points: Vec<Point> = (0..=count)
        .map(|index| {
            let angle = TAU * index as f32 / count as f32;
            Point {
                position: center
                    + axis_u * (radius_u * angle.cos())
                    + axis_v * (radius_v * angle.sin()),
                pressure,
                tilt: None,
                time_ms: 0,
            }
        })
        .collect();
    if points.iter().any(|point| !point.is_valid()) {
        return Err("Shape geometry exceeds the valid document bounds".into());
    }
    Ok(Stroke {
        material: Default::default(),
        fill: None,
        group_id: None,
        layer_id,
        id,
        color,
        radius: stroke_radius,
        brush: frame_brush,
        points,
    })
}

fn plane_frame(plane: DrawingPlane) -> Result<(Vec3, Vec3, Vec3, Vec3), String> {
    if !plane.anchor.is_finite()
        || plane.anchor.abs().max_element() > MAX_WORLD_RADIUS
        || !plane.rotation_degrees.is_finite()
        || plane.rotation_degrees.abs().max_element() > 1.0e6
        || !plane.depth.is_finite()
        || plane.depth.abs() > MAX_WORLD_RADIUS
    {
        return Err("Drawing plane has invalid geometry".into());
    }
    let (u, v, normal) = plane.basis();
    let origin = plane.origin();
    if !origin.is_finite() || !u.is_finite() || !v.is_finite() || !normal.is_finite() {
        return Err("Drawing plane frame is not finite".into());
    }
    Ok((origin, u, v, normal))
}

fn project_to_plane(point: Vec3, origin: Vec3, u: Vec3, v: Vec3) -> Result<Vec3, String> {
    if !point.is_finite() || point.abs().max_element() > MAX_WORLD_RADIUS {
        return Err("Shape point is outside document bounds".into());
    }
    let projected = origin + u * (point - origin).dot(u) + v * (point - origin).dot(v);
    if !projected.is_finite() || projected.abs().max_element() > MAX_WORLD_RADIUS {
        return Err("Projected shape exceeds document bounds".into());
    }
    Ok(projected)
}

fn validate_extent(value: f32, label: &str) -> Result<(), String> {
    if !value.is_finite() || !(0.001..=MAX_WORLD_RADIUS).contains(&value) {
        Err(format!(
            "{label} radius must be between 0.001 and 10000 world units"
        ))
    } else {
        Ok(())
    }
}

fn validate_common(
    plane: DrawingPlane,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
) -> Result<(), String> {
    let _ = plane_frame(plane)?;
    validate_common_values(color, stroke_radius, pressure, brush)
}

fn validate_common_values(
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
) -> Result<(), String> {
    if color
        .iter()
        .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
    {
        return Err("Stroke color components must be from 0 to 1".into());
    }
    if !stroke_radius.is_finite() || !(0.001..=1.0).contains(&stroke_radius) {
        return Err("Brush radius must be between 0.001 and 1 world unit".into());
    }
    if !pressure.is_finite() || !(0.0..=1.0).contains(&pressure) {
        return Err("Pressure must be from 0 to 1".into());
    }
    if !brush.is_valid() {
        return Err("Brush settings are invalid".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, geometry::Camera};
    use eframe::egui::{Pos2, Rect};

    fn rotated_plane() -> DrawingPlane {
        DrawingPlane {
            anchor: Vec3::new(1.0, -2.0, 0.5),
            rotation_degrees: Vec3::new(23.0, 41.0, -17.0),
            depth: 0.7,
        }
    }

    fn check_plane_and_closed_shape(stroke: &Stroke, plane: DrawingPlane, pressure: f32) {
        let (origin, _, _, normal) = plane_frame(plane).unwrap();
        assert!(
            stroke
                .points
                .first()
                .unwrap()
                .position
                .distance_squared(stroke.points.last().unwrap().position)
                <= 1.0e-12,
            "closed marker must repeat the first point within the geometry seam tolerance"
        );
        assert!(stroke.points.len() > MIN_SEGMENTS);
        assert!(stroke.points.iter().all(|point| {
            ((point.position - origin).dot(normal)).abs() < 0.0001 && point.pressure == pressure
        }));
        assert_eq!(stroke.brush.normal, normal);
        assert_eq!(
            crate::geometry::stroke_mesh(stroke).len(),
            (stroke.points.len() - 1) * 48,
            "closed round outline should join its seam without endpoint-cap geometry"
        );
    }

    #[test]
    fn typed_circle_and_drag_ellipse_use_the_rotated_plane_and_uniform_pressure() {
        let plane = rotated_plane();
        let (origin, u, v, _) = plane_frame(plane).unwrap();
        let center = origin + u * 0.45 + v * 0.3;
        let pressure = 0.62;
        let circle = circle_from_center(
            plane,
            center + plane.basis().2 * 0.4,
            0.75,
            11,
            1,
            [0.1, 0.2, 0.3],
            0.03,
            pressure,
            Brush::default(),
        )
        .unwrap();
        check_plane_and_closed_shape(&circle, plane, pressure);
        for point in &circle.points {
            assert!((point.position.distance(center) - 0.75).abs() < 0.0001);
        }

        let first = center - u * 1.2 - v * 0.6 + plane.basis().2;
        let opposite = center + u * 1.2 + v * 0.6 - plane.basis().2;
        let ellipse = ellipse_from_bounds(
            plane,
            first,
            opposite,
            12,
            1,
            [0.3, 0.5, 0.7],
            0.02,
            pressure,
            Brush::default(),
        )
        .unwrap();
        check_plane_and_closed_shape(&ellipse, plane, pressure);
        let (radius_u, radius_v) = ellipse
            .points
            .iter()
            .fold((0.0f32, 0.0f32), |radii, point| {
                let delta = point.position - center;
                (
                    radii.0.max(delta.dot(u).abs()),
                    radii.1.max(delta.dot(v).abs()),
                )
            });
        assert!((radius_u - 1.2).abs() < 0.0001);
        assert!((radius_v - 0.6).abs() < 0.0001);
    }

    #[test]
    fn typed_shape_is_pickable_persistent_exportable_and_one_reversible_edit() {
        let plane = rotated_plane();
        let (origin, u, v, _) = plane_frame(plane).unwrap();
        let center = origin + u * 0.2 - v * 0.1;
        let stroke = ellipse_from_center(
            plane,
            center,
            0.8,
            0.45,
            21,
            1,
            [0.8, 0.4, 0.2],
            0.025,
            0.9,
            Brush::default(),
        )
        .unwrap();
        let mut doc = Document::default();
        let edit = crate::editing::Edit::Add(stroke.clone());
        edit.apply(&mut doc, true);

        let mut camera = Camera::default();
        camera.align_guide(plane);
        let canvas = Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(400.0, 300.0));
        let screen = crate::gizmo::project(&camera, canvas, stroke.points[0].position)
            .unwrap()
            .0;
        assert_eq!(
            crate::gizmo::pick(&doc, &camera, canvas, screen),
            Some(stroke.id)
        );

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let reopened = Document::from_json(&std::fs::read_to_string(file.path()).unwrap()).unwrap();
        assert_eq!(reopened.strokes, doc.strokes);
        let glb = crate::export::encode(&reopened, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&glb.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 1);

        edit.apply(&mut doc, false);
        assert!(doc.strokes.is_empty());
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes, vec![stroke]);
    }

    #[test]
    fn shape_builders_reject_zero_or_nonfinite_dimensions() {
        let plane = DrawingPlane::default();
        assert!(ellipse_from_center(
            plane,
            Vec3::ZERO,
            0.0,
            0.2,
            1,
            1,
            [0.0, 0.0, 0.0],
            0.02,
            1.0,
            Brush::default(),
        )
        .is_err());
        assert!(circle_from_center(
            plane,
            Vec3::ZERO,
            f32::INFINITY,
            1,
            1,
            [0.0, 0.0, 0.0],
            0.02,
            1.0,
            Brush::default(),
        )
        .is_err());
    }
}
