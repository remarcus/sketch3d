//! Bounded planar and cylinder circle/ellipse geometry for editable strokes.
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

/// Builds a circle in the cylinder's unwrapped surface chart. `center` and
/// dimensions are chart coordinates: X is circumferential arc length and Y
/// is axial distance. The result wraps continuously across the cylinder seam.
pub fn cylinder_circle(
    cylinder: crate::surface::Cylinder,
    center: Vec3,
    radius: f32,
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
    tolerance_world: f32,
) -> Result<Stroke, String> {
    let mut stroke = cylinder_ellipse(
        cylinder,
        center,
        radius,
        radius,
        id,
        layer_id,
        color,
        stroke_radius,
        pressure,
        brush,
        tolerance_world,
    )?;
    if let Some(shape) = stroke.curve.as_mut().and_then(|curve| curve.shape.as_mut()) {
        shape.kind = crate::document::CurvedShapeKind::Circle;
    }
    Ok(stroke)
}

/// Builds an ellipse in the cylinder's unwrapped surface chart. Its perimeter
/// is sampled adaptively in world space, with a bounded 24–512 segment count.
pub fn cylinder_ellipse(
    cylinder: crate::surface::Cylinder,
    center: Vec3,
    radius_u: f32,
    radius_v: f32,
    id: u64,
    layer_id: u64,
    color: [f32; 3],
    stroke_radius: f32,
    pressure: f32,
    brush: Brush,
    tolerance_world: f32,
) -> Result<Stroke, String> {
    validate_common_values(color, stroke_radius, pressure, brush)?;
    if !cylinder.valid()
        || !center.is_finite()
        || center.z.abs() > 1.0e-5
        || center.y.abs() > cylinder.height * 0.5
    {
        return Err("Cylinder shape center is outside the captured surface".into());
    }
    validate_extent(radius_u, "Horizontal")?;
    validate_extent(radius_v, "Vertical")?;
    if radius_u > std::f32::consts::PI * cylinder.radius
        || center.y - radius_v < -cylinder.height * 0.5
        || center.y + radius_v > cylinder.height * 0.5
    {
        return Err("Cylinder shape extends beyond the unwrapped surface bounds".into());
    }
    if !tolerance_world.is_finite()
        || !(crate::curve_geometry::MIN_TOLERANCE_WORLD
            ..=crate::curve_geometry::MAX_TOLERANCE_WORLD)
            .contains(&tolerance_world)
    {
        return Err("Cylinder shape tessellation tolerance is outside supported bounds".into());
    }
    let extent = radius_u.max(radius_v).max(cylinder.radius);
    let segments = (std::f32::consts::TAU * (extent / (8.0 * tolerance_world)).sqrt())
        .ceil()
        .clamp(MIN_SEGMENTS as f32, MAX_SEGMENTS as f32) as usize;
    let angle = center.x / cylinder.radius;
    let anchor = cylinder
        .point_from_parameter(center)
        .ok_or("Invalid cylinder shape center")?;
    let rotation = cylinder.rotation();
    let normal = rotation * Vec3::new(angle.sin(), 0.0, angle.cos());
    let tangent = rotation * Vec3::new(angle.cos(), 0.0, -angle.sin());
    let mut frame_brush = Brush {
        normal,
        tangent,
        ..brush
    };
    frame_brush.dabs.enabled = false;
    if !frame_brush.is_valid() {
        return Err("Brush settings are invalid for the cylinder surface".into());
    }
    let points: Vec<Point> = (0..=segments)
        .map(|index| {
            let theta = TAU * index as f32 / segments as f32;
            let parameter = Vec3::new(
                center.x + radius_u * theta.cos(),
                center.y + radius_v * theta.sin(),
                0.0,
            );
            Point {
                position: cylinder
                    .point_from_parameter(parameter)
                    .unwrap_or(Vec3::splat(f32::NAN)),
                pressure,
                tilt: None,
                time_ms: 0,
            }
        })
        .collect();
    if points.iter().any(|point| !point.is_valid()) {
        return Err("Cylinder shape exceeds the valid document bounds".into());
    }
    let surface_target = crate::dynamics::DabSurfaceTarget::Cylinder {
        center: cylinder.center,
        rotation,
        radius: cylinder.radius,
        height: cylinder.height,
    };
    let chart_anchor = |position| crate::document::CurveAnchor {
        position,
        pressure,
        incoming: Vec3::ZERO,
        outgoing: Vec3::ZERO,
        manual: true,
    };
    let curve = crate::document::CurveData {
        anchors: vec![
            chart_anchor(center),
            chart_anchor(Vec3::new(center.x + radius_u, center.y, 0.0)),
        ],
        frame_origin: anchor,
        frame_normal: normal,
        frame_tangent: tangent,
        half_extent: None,
        tension: 1.0,
        surface_target: Some(surface_target),
        shape: Some(crate::document::CurvedShapeData {
            kind: crate::document::CurvedShapeKind::Ellipse,
            center,
            radius_u,
            radius_v,
        }),
    };
    if !curve.is_valid() {
        return Err("Cylinder shape controls exceed the captured surface bounds".into());
    }
    Ok(Stroke {
        model_surface: None,
        curve: Some(curve),
        material: Default::default(),
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        fill: None,
        group_id: None,
        layer_id,
        id,
        color,
        gradient: None,
        radius: stroke_radius,
        brush: frame_brush,
        points,
    })
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
        model_surface: None,
        curve: None,
        material: Default::default(),
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        fill: None,
        group_id: None,
        layer_id,
        id,
        color,
        gradient: None,
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
    fn cylinder_circle_crosses_the_unwrapped_seam_and_rejects_finite_height_overflow() {
        let cylinder = crate::surface::Cylinder {
            center: Vec3::new(0.5, -0.2, 1.0),
            rotation_degrees: Vec3::new(17.0, 31.0, -9.0),
            radius: 1.25,
            height: 3.0,
        };
        let before_seam = cylinder
            .parameterize(cylinder.point(std::f32::consts::PI - 0.02, 0.0), None)
            .unwrap();
        let after_seam = cylinder
            .parameterize(
                cylinder.point(-std::f32::consts::PI + 0.02, 0.0),
                Some(before_seam),
            )
            .unwrap();
        assert!((after_seam.x - before_seam.x - 0.04 * cylinder.radius).abs() < 1.0e-4);
        let center = Vec3::new((std::f32::consts::PI - 0.015) * cylinder.radius, 0.0, 0.0);
        let stroke = cylinder_circle(
            cylinder,
            center,
            0.12,
            19,
            1,
            [0.2, 0.4, 0.6],
            0.02,
            0.7,
            Brush::default(),
            0.001,
        )
        .unwrap();
        assert!(stroke.points.len() > MIN_SEGMENTS);
        assert!(
            stroke
                .points
                .first()
                .unwrap()
                .position
                .distance(stroke.points.last().unwrap().position)
                < 1.0e-5
        );
        assert!(stroke.points.iter().all(|point| point.is_valid()));
        let semantic = stroke
            .curve
            .as_ref()
            .expect("cylinder circles retain controls");
        assert!(semantic.is_valid());
        assert_eq!(
            semantic.shape.unwrap().kind,
            crate::document::CurvedShapeKind::Circle
        );
        assert!(stroke
            .points
            .windows(2)
            .all(|pair| { pair[0].position.distance(pair[1].position) < 0.04 }));
        assert_eq!(stroke.brush.normal.length(), 1.0);
        assert!((stroke.brush.normal.dot(stroke.brush.tangent)).abs() < 1.0e-5);

        let too_tall = cylinder_circle(
            cylinder,
            Vec3::new(0.0, 1.45, 0.0),
            0.1,
            20,
            1,
            [0.2, 0.4, 0.6],
            0.02,
            0.7,
            Brush::default(),
            0.001,
        );
        assert!(too_tall.is_err());
    }

    #[test]
    fn cylinder_ellipse_retains_semantics_through_native_recovery_picking_and_glb_export() {
        let cylinder = crate::surface::Cylinder {
            center: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            radius: 1.25,
            height: 3.0,
        };
        let center = Vec3::new(std::f32::consts::PI * cylinder.radius - 0.02, 0.1, 0.0);
        let make_ellipse = |tolerance| {
            cylinder_ellipse(
                cylinder,
                center,
                0.12,
                0.3,
                31,
                1,
                [0.8, 0.3, 0.2],
                0.025,
                0.9,
                Brush::default(),
                tolerance,
            )
            .unwrap()
        };
        let ellipse = make_ellipse(0.002);
        let coarse = make_ellipse(0.08);
        assert!(ellipse.points.len() > coarse.points.len());
        assert!(ellipse.points.iter().all(|point| {
            let local = cylinder.to_local(point.position);
            (local.x.hypot(local.z) - cylinder.radius).abs() < 1.0e-4
        }));
        let semantic = ellipse.curve.as_ref().expect("ellipse retains dimensions");
        assert!(semantic.is_valid());
        let shape = semantic.shape.unwrap();
        assert_eq!(shape.kind, crate::document::CurvedShapeKind::Ellipse);
        assert_eq!(shape.center, center);
        assert_eq!((shape.radius_u, shape.radius_v), (0.12, 0.3));

        let document = Document {
            strokes: vec![ellipse.clone()],
            ..Document::default()
        };
        let selected = std::collections::HashSet::from([ellipse.id]);
        let snapshot = crate::editing::snapshot(&document, &selected);
        let uniformly_scaled = crate::editing::transformed(
            &snapshot,
            Vec3::ZERO,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.2,
        )
        .unwrap();
        assert!(uniformly_scaled[0]
            .1
            .curve
            .as_ref()
            .unwrap()
            .samples_match_stroke(&uniformly_scaled[0].1));
        let nonuniform = crate::editing::transformed_with_scale(
            &snapshot,
            Vec3::ZERO,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::new(1.2, 1.0, 1.0),
            glam::Quat::IDENTITY,
            true,
        )
        .unwrap_err();
        assert!(nonuniform.contains("Reproject with Convert semantic curves"));

        let serialized = serde_json::to_string(&document).unwrap();
        let reopened = Document::from_json(&serialized).unwrap();
        assert_eq!(reopened.strokes, document.strokes);
        assert_eq!(reopened.version, 28);

        let recovery_root = tempfile::tempdir().unwrap();
        let snapshot = crate::recovery_store::RecoverySnapshot::new(
            "cylinder-curve-shape-test".into(),
            None,
            reopened.clone(),
        );
        crate::recovery_store::write(recovery_root.path(), &snapshot).unwrap();
        let recovered = crate::recovery_store::scan(recovery_root.path(), "test-reader").unwrap();
        assert_eq!(recovered.candidates.len(), 1);
        assert_eq!(
            recovered.candidates[0].snapshot.document.strokes,
            reopened.strokes
        );

        let mut camera = crate::geometry::Camera::default();
        camera.yaw = std::f32::consts::PI;
        camera.pitch = 0.0;
        camera.target = cylinder.center;
        camera.distance = 5.0;
        let canvas = Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(500.0, 400.0));
        let screen = crate::gizmo::project(&camera, canvas, ellipse.points[0].position)
            .unwrap()
            .0;
        assert_eq!(
            crate::gizmo::pick(&reopened, &camera, canvas, screen),
            Some(ellipse.id)
        );

        let exported = crate::export::encode(&reopened, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 1);
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
