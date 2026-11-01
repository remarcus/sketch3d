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
    let frame_brush = Brush {
        normal,
        tangent: axis_u,
        ..brush
    };
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
