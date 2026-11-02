//! Deterministic, bounded extraction of line-art candidates from OBJ meshes.
use crate::{
    geometry::{Camera, Projection},
    obj_mesh_index::ObjMeshIndex,
    obj_reference::ObjModel,
};
use glam::Vec3;

pub const MAX_INPUT_TRIANGLES: usize = 250_000;
pub const MAX_CANDIDATE_EDGES: usize = 750_000;
pub const MAX_OUTPUT_STROKES: usize = 20_000;
pub const MAX_OUTPUT_POINTS: usize = MAX_OUTPUT_STROKES * 2;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub crease_angle_degrees: f32,
    pub boundaries: bool,
    pub creases: bool,
    pub silhouettes: bool,
}

#[derive(Clone)]
pub struct Draft {
    pub source_id: u64,
    pub segments: Vec<[Vec3; 2]>,
    pub truncated: bool,
}

pub struct CappedSegments {
    pub segments: Vec<[Vec3; 2]>,
    pub truncated: bool,
}

/// Apply conservative visibility to every candidate, including boundaries.
/// Quarter, midpoint, and three-quarter samples prevent a partially hidden
/// edge from leaking through the occluder. The callback uses the scene BVH.
pub fn filter_visible<F>(
    segments: impl IntoIterator<Item = [Vec3; 2]>,
    mut visible: F,
) -> CappedSegments
where
    F: FnMut(Vec3) -> bool,
{
    let mut output = Vec::new();
    let mut truncated = false;
    for segment in segments {
        let is_visible = [0.25_f32, 0.5, 0.75]
            .into_iter()
            .all(|t| visible(segment[0].lerp(segment[1], t)));
        if is_visible {
            if output.len() == MAX_OUTPUT_STROKES {
                truncated = true;
                break;
            }
            output.push(segment);
        }
    }
    CappedSegments {
        segments: output,
        truncated,
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            crease_angle_degrees: 35.0,
            boundaries: true,
            creases: true,
            silhouettes: true,
        }
    }
}

/// Extract local-space line segments in triangle/edge order. The adjacency
/// index is built once and queried in O(T); no all-pairs triangle scan occurs.
pub fn extract(
    model: &ObjModel,
    camera: &Camera,
    center: Vec3,
    rotation: glam::Quat,
    scale: f32,
    settings: Settings,
) -> Result<CappedSegments, String> {
    if model.triangles.len() > MAX_INPUT_TRIANGLES {
        return Err(format!(
            "Line art is limited to {MAX_INPUT_TRIANGLES} triangles per model"
        ));
    }
    if !settings.crease_angle_degrees.is_finite()
        || !(0.0..=180.0).contains(&settings.crease_angle_degrees)
    {
        return Err("Crease angle must be between 0 and 180 degrees".into());
    }
    let index = ObjMeshIndex::new(model)?;
    let mut lines = Vec::new();
    let crease_cos = settings.crease_angle_degrees.to_radians().cos();
    // Classification happens in model-local coordinates, matching OBJ normals.
    let inverse_rotation = rotation.inverse();
    let to_eye = inverse_rotation * (camera.eye() - center) / scale;
    let ortho_to_eye = inverse_rotation * camera.basis().2;
    let mut candidates = 0usize;
    for (triangle_index, triangle) in model.triangles.iter().enumerate() {
        let neighbors = index
            .triangle_neighbors(triangle_index as u32)
            .ok_or("Missing triangle adjacency")?;
        for edge in 0..3 {
            let neighbor = neighbors[edge];
            if neighbor.is_some_and(|other| other < triangle_index as u32) {
                continue;
            }
            candidates += 1;
            if candidates > MAX_CANDIDATE_EDGES {
                return Err("Line-art candidate edge limit exceeded".into());
            }
            let first_index = triangle.corners[edge].position as usize;
            let second_index = triangle.corners[(edge + 1) % 3].position as usize;
            let first = *model
                .positions
                .get(first_index)
                .ok_or("Invalid OBJ edge position")?;
            let second = *model
                .positions
                .get(second_index)
                .ok_or("Invalid OBJ edge position")?;
            let selected = match neighbor {
                None => settings.boundaries,
                Some(other) => {
                    let other_triangle = &model.triangles[other as usize];
                    let a = triangle.face_normal.normalize_or_zero();
                    let b = other_triangle.face_normal.normalize_or_zero();
                    let crease = settings.creases && a.dot(b) <= crease_cos;
                    let center_a = triangle
                        .corners
                        .iter()
                        .map(|corner| model.positions[corner.position as usize])
                        .sum::<Vec3>()
                        / 3.0;
                    let center_b = other_triangle
                        .corners
                        .iter()
                        .map(|corner| model.positions[corner.position as usize])
                        .sum::<Vec3>()
                        / 3.0;
                    let face_a = if camera.projection == Projection::Perspective {
                        a.dot(to_eye - center_a)
                    } else {
                        a.dot(ortho_to_eye)
                    };
                    let face_b = if camera.projection == Projection::Perspective {
                        b.dot(to_eye - center_b)
                    } else {
                        b.dot(ortho_to_eye)
                    };
                    crease
                        || (settings.silhouettes
                            && face_a.is_sign_positive() != face_b.is_sign_positive())
                }
            };
            if selected {
                if lines.len() >= MAX_OUTPUT_STROKES
                    || lines.len().saturating_mul(2) + 2 > MAX_OUTPUT_POINTS
                {
                    return Ok(CappedSegments {
                        segments: lines,
                        truncated: true,
                    });
                }
                lines.push([first, second]);
            }
        }
    }
    Ok(CappedSegments {
        segments: lines,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn open_model() -> ObjModel {
        crate::obj_reference::parse(
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
            Path::new("open.obj"),
        )
        .unwrap()
    }
    #[test]
    fn open_mesh_emits_bounded_boundaries_deterministically() {
        let model = open_model();
        let settings = Settings {
            creases: false,
            silhouettes: false,
            ..Settings::default()
        };
        let first = extract(
            &model,
            &Camera::default(),
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
            settings,
        )
        .unwrap();
        assert_eq!(first.segments.len(), 3);
        assert_eq!(
            first.segments,
            extract(
                &model,
                &Camera::default(),
                Vec3::ZERO,
                glam::Quat::IDENTITY,
                1.0,
                settings
            )
            .unwrap()
            .segments
        );
    }

    #[test]
    fn cube_and_curved_mesh_are_stable_across_cameras() {
        let cube = crate::obj_reference::parse(b"v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\nf 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n", Path::new("cube.obj")).unwrap();
        let settings = Settings {
            boundaries: false,
            creases: false,
            silhouettes: true,
            ..Settings::default()
        };
        let a = extract(
            &cube,
            &Camera::default(),
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
            settings,
        )
        .unwrap();
        let mut other_camera = Camera::default();
        other_camera.yaw += 2.1;
        let b = extract(
            &cube,
            &other_camera,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
            settings,
        )
        .unwrap();
        assert!(!a.segments.is_empty() && !b.segments.is_empty());
        assert_ne!(
            a.segments, b.segments,
            "view-dependent silhouettes change with camera"
        );
        let curved = crate::obj_reference::parse(
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\nf 2 1 4\n",
            Path::new("curved.obj"),
        )
        .unwrap();
        let lines = extract(
            &curved,
            &Camera::default(),
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
            Settings::default(),
        )
        .unwrap();
        assert!(
            lines.segments.len() >= 5,
            "open boundaries and the curved crease are included"
        );
    }

    #[test]
    fn occlusion_filter_includes_boundary_candidates_and_is_bounded() {
        let candidates = vec![[Vec3::ZERO, Vec3::X], [Vec3::Y, Vec3::ONE]];
        let visible = filter_visible(candidates, |sample| sample.y == 0.0);
        assert_eq!(visible.segments.len(), 1);
        assert_eq!(visible.segments[0], [Vec3::ZERO, Vec3::X]);
        let partly_hidden = filter_visible(vec![[Vec3::ZERO, Vec3::X]], |sample| {
            sample.x < 0.4 || sample.x > 0.6
        });
        assert!(partly_hidden.segments.is_empty());
    }

    #[test]
    fn extraction_stays_bounded_and_fast_on_a_grid_fixture() {
        let mut obj = String::new();
        const SIDE: usize = 32;
        for y in 0..=SIDE {
            for x in 0..=SIDE {
                obj.push_str(&format!("v {x} {y} 0\n"));
            }
        }
        for y in 0..SIDE {
            for x in 0..SIDE {
                let a = y * (SIDE + 1) + x + 1;
                let b = a + 1;
                let c = a + SIDE + 1;
                let d = c + 1;
                obj.push_str(&format!("f {a} {b} {d}\nf {a} {d} {c}\n"));
            }
        }
        let model = crate::obj_reference::parse(obj.as_bytes(), Path::new("grid.obj")).unwrap();
        let start = std::time::Instant::now();
        let lines = extract(
            &model,
            &Camera::default(),
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
            Settings::default(),
        )
        .unwrap();
        assert!(lines.segments.len() <= MAX_OUTPUT_STROKES);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "2048 triangles should extract in bounded near-linear time"
        );
    }

    #[test]
    fn output_stroke_and_point_limits_are_enforced() {
        let mut model = open_model();
        let triangle = model.triangles[0].clone();
        model.triangles.resize(MAX_OUTPUT_STROKES + 1, triangle);
        let output = extract(
            &model,
            &Camera::default(),
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
            Settings {
                creases: false,
                silhouettes: false,
                ..Settings::default()
            },
        )
        .unwrap();
        assert_eq!(output.segments.len(), MAX_OUTPUT_STROKES);
        assert!(output.segments.len() * 2 <= MAX_OUTPUT_POINTS);
        assert!(output.truncated);
    }

    #[test]
    fn baked_line_art_roundtrips_and_appears_in_glb_and_obj_exports() {
        use crate::document::{Brush, Document, Point, Stroke, StrokeMaterial};
        let baked = Stroke {
            fill: None,
            material: StrokeMaterial::Unlit,
            group_id: None,
            layer_id: 1,
            id: 700,
            color: [0.03, 0.03, 0.04],
            gradient: None,
            radius: 0.006,
            brush: Brush::default(),
            points: vec![
                Point {
                    position: Vec3::ZERO,
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                },
                Point {
                    position: Vec3::X,
                    pressure: 1.0,
                    tilt: None,
                    time_ms: 0,
                },
            ],
            point_thickness: None,
            modifiers: None,
            curve: None,
        };
        let document = Document {
            strokes: vec![baked.clone()],
            ..Document::default()
        };

        let native = serde_json::to_vec(&document).unwrap();
        let reloaded: Document = serde_json::from_slice(&native).unwrap();
        assert_eq!(reloaded.strokes, vec![baked.clone()]);

        let recovery_root = tempfile::tempdir().unwrap();
        let recovery = crate::recovery_store::RecoverySnapshot::new(
            "line-art-bake-test".into(),
            None,
            document.clone(),
        );
        crate::recovery_store::write(recovery_root.path(), &recovery).unwrap();
        let scan = crate::recovery_store::scan(recovery_root.path(), "different-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(
            scan.candidates[0].snapshot.document.strokes,
            vec![baked.clone()]
        );

        let glb = crate::export::encode(&document, None).unwrap();
        assert_eq!(glb.summary.strokes, 1);
        let parsed = gltf::Gltf::from_slice(&glb.bytes).unwrap();
        assert!(
            parsed.meshes().next().is_some(),
            "baked line art is ordinary exported mesh geometry"
        );

        let workspace = std::env::current_dir().unwrap();
        let export_root = tempfile::tempdir_in(workspace.join("verification")).unwrap();
        let obj_path = export_root.path().join("line-art.obj");
        crate::obj_export::write_obj_bundle(&document, None, &obj_path, false).unwrap();
        let obj_text = std::fs::read_to_string(obj_path).unwrap();
        assert!(obj_text.contains("g stroke_700"));
        assert!(obj_text.lines().any(|line| line.starts_with("v ")));
    }
}
