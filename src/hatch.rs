//! Clip parallel centerlines against the same even-odd contours as a fill.
use crate::{
    document::{Brush, Document, Point, Stroke},
    polygon,
    texture::{TextureAnchorMode, TextureFrame},
};
use glam::DVec2;

pub const MIN_SPACING: f32 = 0.005;
pub const MAX_SPACING: f32 = 10.0;
const MAX_ROWS: usize = 256;
const MAX_STROKES: usize = 512;
const MAX_POINTS: usize = 16_384;
const MAX_VERTICES: usize = 1_000_000;

#[derive(Clone, Copy)]
pub struct Settings {
    pub angle: f32,
    pub spacing: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            angle: 45.0,
            spacing: 0.12,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Ink {
    pub brush: Brush,
    pub color: [f32; 3],
    pub radius: f32,
    pub layer: u64,
}

pub fn supported(source: &Stroke) -> bool {
    source
        .fill
        .as_ref()
        .is_some_and(|fill| fill.cylinder.is_none() && fill.mesh.is_none())
}

pub fn build(
    doc: &Document,
    source: &Stroke,
    settings: Settings,
    ink: Ink,
    next_id: u64,
) -> Result<Vec<Stroke>, String> {
    if !supported(source) || !doc.editable(source) {
        return Err("Select one editable planar fill to hatch".into());
    }
    if !doc.layer_is_editable(ink.layer) {
        return Err("Choose a visible, unlocked layer for the hatch strokes".into());
    }
    if ink.brush.flow.enabled {
        return Err("Hatching requires a brush without timed airbrush flow".into());
    }
    if !(0.001..=1.0).contains(&ink.radius)
        || ink.color.iter().any(|color| !(0.0..=1.0).contains(color))
        || next_id == 0
    {
        return Err("Invalid hatch brush size, color, or stroke ID".into());
    }
    if !settings.angle.is_finite() || !(MIN_SPACING..=MAX_SPACING).contains(&settings.spacing) {
        return Err("Invalid hatch angle or spacing".into());
    }
    polygon::triangulate(source)?;
    let mut rings = vec![polygon::projected(source)];
    rings.extend(polygon::projected_holes(source));
    let segments = clipped_segments(rings, settings)?;
    if doc.strokes.len() + segments.len() > 100_000
        || next_id.checked_add(segments.len() as u64).is_none()
    {
        return Err("The drawing has no room for this hatch pass".into());
    }
    let origin = source.points[0].position;
    let normal = source.brush.normal;
    let tangent = source.brush.tangent;
    let across = normal.cross(tangent);
    let mut brush = ink.brush;
    brush.normal = normal;
    brush.tangent = tangent;
    brush.dabs.surface_target = None;
    brush.dab_distance_origin = 0.0;
    brush.dab_total_length = 0.0;
    brush.texture.pressure_samples_available = false;
    if brush.texture.anchor_mode == TextureAnchorMode::CapturedPlanar {
        brush.texture.frame = Some(TextureFrame {
            origin: origin.to_array(),
            axis_u: tangent.to_array(),
            axis_v: across.to_array(),
        });
    }
    let mut strokes = Vec::with_capacity(segments.len());
    let mut point_count = 0;
    let mut dab_count = 0;
    let mut vertex_count = 0;
    for (i, segment) in segments.into_iter().enumerate() {
        let points = segment
            .map(|p| Point {
                position: origin + tangent * p.x as f32 + across * p.y as f32,
                pressure: 1.0,
                tilt: None,
                time_ms: 0,
            })
            .to_vec();
        let id = next_id + i as u64;
        brush.dab_seed = crate::dab_placement::seed_for_stroke(id);
        let stroke = Stroke {
            id,
            layer_id: ink.layer,
            color: ink.color,
            radius: ink.radius,
            brush,
            points,
            guide_attachment: None,
            model_surface: None,
            fill: None,
            material: Default::default(),
            group_id: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
        };
        if !stroke.brush.is_valid() || stroke.points.iter().any(|point| !point.is_valid()) {
            return Err("The selected brush or hatch geometry is outside drawing limits".into());
        }
        point_count += stroke.points.len();
        let dabs = crate::dab_placement::point_count(&stroke)?;
        dab_count += dabs;
        vertex_count += vertex_estimate(&stroke, dabs);
        if point_count > MAX_POINTS || vertex_count > MAX_VERTICES {
            return Err(
                "Hatch pass exceeds its geometry budget. Increase spacing or simplify the brush."
                    .into(),
            );
        }
        strokes.push(stroke);
    }
    if doc
        .strokes
        .iter()
        .map(|stroke| stroke.points.len())
        .sum::<usize>()
        + point_count
        > 2_000_000
        || (dab_count > 0
            && crate::dab_placement::document_count(doc)? + dab_count
                > crate::dab_placement::MAX_DOCUMENT_DABS)
    {
        return Err("Hatch pass exceeds the document geometry limit".into());
    }
    Ok(strokes)
}

fn vertex_estimate(stroke: &Stroke, dabs: usize) -> usize {
    if stroke.brush.tip_asset_id != 0 || stroke.brush.dabs.enabled {
        // Bound even an imported tip preset with dense distance placements.
        dabs.max(stroke.points.len()) * 216
    } else if stroke.brush.strands.enabled {
        stroke.brush.strands.vertex_estimate(stroke.points.len())
    } else {
        crate::geometry::profile_tube_vertex_estimate(stroke).max(stroke.points.len() * 216)
    }
}

fn clipped_segments(rings: Vec<Vec<DVec2>>, settings: Settings) -> Result<Vec<[DVec2; 2]>, String> {
    let angle = f64::from(settings.angle).to_radians();
    let along = DVec2::new(angle.cos(), angle.sin());
    let across = along.perp();
    let chart: Vec<Vec<_>> = rings
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|p| DVec2::new(p.dot(along), p.dot(across)))
                .collect()
        })
        .collect();
    let (min, max) = chart[0]
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), p| {
            (min.min(p.y), max.max(p.y))
        });
    let spacing = f64::from(settings.spacing);
    let rows = ((max - min) / spacing).ceil().max(1.0) as usize;
    if rows > MAX_ROWS {
        return Err("Hatch pass exceeds 256 rows. Increase spacing.".into());
    }
    let first_row = (min + max - (rows - 1) as f64 * spacing) * 0.5;
    let mut segments = Vec::new();
    let mut crossings = Vec::new();
    for row in 0..rows {
        let y = first_row + row as f64 * spacing;
        crossings.clear();
        for ring in &chart {
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                if (a.y > y) != (b.y > y) {
                    crossings.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
                }
            }
        }
        crossings.sort_unstable_by(f64::total_cmp);
        for &[a, b] in crossings.as_chunks::<2>().0 {
            if b - a <= 1e-5 {
                continue;
            }
            if segments.len() == MAX_STROKES {
                return Err("Hatch pass exceeds 512 strokes. Increase spacing.".into());
            }
            segments.push([along * a + across * y, along * b + across * y]);
        }
    }
    if segments.is_empty() {
        return Err("The fill is too small for a usable hatch stroke".into());
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::PlanarFill;
    use glam::{Quat, Vec3};

    fn fill(points: Vec<Vec3>, brush: Brush, fill: PlanarFill) -> Stroke {
        Stroke {
            id: 1,
            layer_id: 1,
            color: [0.7; 3],
            radius: 0.02,
            brush,
            points: points
                .into_iter()
                .map(|position| Point {
                    position,
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                })
                .collect(),
            guide_attachment: None,
            model_surface: None,
            fill: Some(fill),
            material: Default::default(),
            group_id: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
        }
    }

    fn ink() -> Ink {
        Ink {
            brush: Brush::default(),
            color: [0.2, 0.3, 0.5],
            radius: 0.02,
            layer: 1,
        }
    }

    #[test]
    fn planar_hatching_clips_concavity_and_hole_and_undoes_as_one_pass() {
        let rotation = Quat::from_rotation_y(0.4) * Quat::from_rotation_x(0.3);
        let origin = Vec3::new(1.0, 2.0, 0.5);
        let world = |(x, y)| origin + rotation * Vec3::new(x, y, 0.0);
        let ring = [
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 1.0),
            (2.0, 1.0),
            (2.0, 4.0),
            (0.0, 4.0),
        ];
        let hole = [(0.5, 1.4), (1.5, 1.4), (1.5, 2.6), (0.5, 2.6)];
        let source = fill(
            ring.into_iter().map(world).collect(),
            Brush {
                normal: rotation * Vec3::Z,
                tangent: rotation * Vec3::X,
                ..Default::default()
            },
            PlanarFill {
                holes: vec![hole.into_iter().map(world).collect()],
                ..Default::default()
            },
        );
        let mut doc = Document::default();
        doc.strokes.push(source.clone());
        let settings = Settings {
            angle: 0.0,
            spacing: 0.5,
        };
        let strokes = build(&doc, &source, settings, ink(), 2).unwrap();
        assert!(strokes.len() > 8);
        for stroke in &strokes {
            assert_eq!(stroke.color, ink().color);
            assert_eq!(stroke.guide_attachment, None);
            let local = [stroke.points[0], stroke.points[1]]
                .map(|p| rotation.inverse() * (p.position - origin));
            assert!((local[0].y - local[1].y).abs() < 1e-5);
            for i in 1..10 {
                let p = local[0].lerp(local[1], i as f32 / 10.0);
                assert!(p.z.abs() < 1e-5 && (p.y <= 1.0 || p.x <= 2.0 + 1e-5));
                assert!(!(p.x > 0.5 && p.x < 1.5 && p.y > 1.4 && p.y < 2.6));
            }
        }
        let edit = crate::editing::Edit::AddBatch(strokes.clone());
        edit.apply(&mut doc, true);
        let crosshatch = build(
            &doc,
            &source,
            Settings {
                angle: 90.0,
                ..settings
            },
            ink(),
            2 + strokes.len() as u64,
        )
        .unwrap();
        assert!(crosshatch[0].id > strokes.last().unwrap().id);
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_ok());
        edit.apply(&mut doc, false);
        assert_eq!(doc.strokes, vec![source.clone()]);
        assert!(
            build(
                &doc,
                &source,
                Settings {
                    spacing: MIN_SPACING,
                    ..settings
                },
                ink(),
                2
            )
            .is_err()
        );
    }
}
