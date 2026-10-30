//! Self-contained glTF 2.0 binary export from semantic stroke data only.
use crate::{
    document::{BrushKind, Document, Stroke},
    geometry,
};
use glam::Vec3;
use serde::Serialize;
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::Path,
};

const MAX_BINARY_BYTES: usize = 128 * 1024 * 1024;
const MAX_TRIANGLE_VERTICES_PER_STROKE: usize = 2_000_000;
#[derive(Clone, Debug, Serialize)]
pub struct ExportSummary {
    pub strokes: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub bytes: usize,
}
pub struct EncodedGlb {
    pub bytes: Vec<u8>,
    pub summary: ExportSummary,
}
struct Mesh {
    vertices: Vec<[f32; 6]>,
    indices: Vec<u32>,
    min: Vec3,
    max: Vec3,
}

fn stroke_mesh(stroke: &Stroke) -> Result<Mesh, String> {
    if !stroke.brush.is_valid()
        || !stroke.radius.is_finite()
        || !(0.001..=1.0).contains(&stroke.radius)
        || stroke
            .color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || stroke.points.iter().any(|p| {
            !p.position.is_finite()
                || p.position.abs().max_element() > 10000.0
                || !p.pressure.is_finite()
                || !(0.0..=1.0).contains(&p.pressure)
                || p.tilt.is_some_and(|tilt| {
                    !tilt.is_finite() || (tilt.length_squared() - 1.0).abs() > 0.001
                })
        })
    {
        return Err(format!(
            "Stroke {} contains invalid geometry or color",
            stroke.id
        ));
    }
    let estimate = stroke
        .points
        .len()
        .saturating_add(3) // Distance taper may insert three tessellation landmarks.
        .saturating_sub(1)
        .saturating_mul(48)
        .saturating_add(48);
    if estimate > MAX_TRIANGLE_VERTICES_PER_STROKE {
        return Err(format!(
            "Stroke {} is too large to export in one piece",
            stroke.id
        ));
    }
    let source = geometry::stroke_mesh(stroke);
    if source.len() > MAX_TRIANGLE_VERTICES_PER_STROKE {
        return Err(format!(
            "Stroke {} exceeds the working mesh budget",
            stroke.id
        ));
    }
    let mut mesh = Mesh {
        vertices: Vec::new(),
        indices: Vec::new(),
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };
    let mut unique = HashMap::<[u32; 6], u32>::new();
    for triangle in source.as_chunks::<3>().0 {
        let mut triangle = *triangle;
        if triangle
            .iter()
            .any(|v| !Vec3::from(v.position).is_finite() || !Vec3::from(v.normal).is_finite())
        {
            return Err(format!(
                "Stroke {} generated non-finite geometry",
                stroke.id
            ));
        }
        let a = Vec3::from(triangle[0].position);
        let b = Vec3::from(triangle[1].position);
        let c = Vec3::from(triangle[2].position);
        let face = (b - a).cross(c - a);
        if face.length_squared() < 1e-18 {
            continue;
        }
        let expected = triangle.iter().map(|v| Vec3::from(v.normal)).sum::<Vec3>();
        // The viewport is two-sided. Export consistent outward winding for PBR tools.
        if face.dot(expected) < 0.0 {
            triangle.swap(1, 2);
        }
        for vertex in triangle {
            let p = Vec3::from(vertex.position);
            let n = Vec3::from(vertex.normal)
                .try_normalize()
                .unwrap_or_else(|| face.normalize());
            let data = [p.x, p.y, p.z, n.x, n.y, n.z];
            let key = data.map(f32::to_bits);
            let index = if let Some(index) = unique.get(&key) {
                *index
            } else {
                let index =
                    u32::try_from(mesh.vertices.len()).map_err(|_| "Too many mesh vertices")?;
                mesh.vertices.push(data);
                unique.insert(key, index);
                mesh.min = mesh.min.min(p);
                mesh.max = mesh.max.max(p);
                index
            };
            mesh.indices.push(index);
        }
    }
    Ok(mesh)
}
pub fn encode(doc: &Document, selected: Option<&HashSet<u64>>) -> Result<EncodedGlb, String> {
    if !matches!(doc.version, 1..=3) {
        return Err("Unsupported source document version".into());
    }
    let mut binary = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut materials = Vec::new();
    let mut colors = HashMap::new();
    let mut ids = HashSet::new();
    let mut summary = ExportSummary {
        strokes: 0,
        vertices: 0,
        triangles: 0,
        bytes: 0,
    };
    for stroke in &doc.strokes {
        if selected.is_some_and(|selection| !selection.contains(&stroke.id))
            || stroke.points.is_empty()
        {
            continue;
        }
        if !ids.insert(stroke.id) {
            return Err("Source document has duplicate stroke IDs".into());
        }
        let mesh = stroke_mesh(stroke)?;
        if mesh.indices.is_empty() {
            continue;
        }
        let extra = mesh
            .vertices
            .len()
            .checked_mul(24)
            .and_then(|n| n.checked_add(mesh.indices.len() * 4))
            .ok_or("Export size overflow")?;
        if binary.len().saturating_add(extra) > MAX_BINARY_BYTES {
            return Err(
                "Export exceeds the 128 MiB geometry limit; export smaller selections".into(),
            );
        }
        let vertex_offset = binary.len();
        for vertex in &mesh.vertices {
            for value in vertex {
                binary.extend_from_slice(&value.to_le_bytes());
            }
        }
        let vertex_view = views.len();
        views.push(json!({"buffer":0,"byteOffset":vertex_offset,"byteLength":mesh.vertices.len()*24,"byteStride":24,"target":34962}));
        let position = accessors.len();
        accessors.push(json!({"bufferView":vertex_view,"byteOffset":0,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3","min":mesh.min.to_array(),"max":mesh.max.to_array()}));
        let normal = accessors.len();
        accessors.push(json!({"bufferView":vertex_view,"byteOffset":12,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3"}));
        let index_offset = binary.len();
        for index in &mesh.indices {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        let index_view = views.len();
        views.push(json!({"buffer":0,"byteOffset":index_offset,"byteLength":mesh.indices.len()*4,"target":34963}));
        let indices = accessors.len();
        accessors.push(json!({"bufferView":index_view,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}));
        let double_sided = stroke.brush.kind == BrushKind::Ribbon;
        let key = (stroke.color.map(f32::to_bits), double_sided);
        let material = if let Some(index) = colors.get(&key) {
            *index
        } else {
            let index = materials.len();
            materials.push(json!({"name":format!("Ink {}",index+1),"pbrMetallicRoughness":{"baseColorFactor":[stroke.color[0],stroke.color[1],stroke.color[2],1.0],"metallicFactor":0.0,"roughnessFactor":0.8},"alphaMode":"OPAQUE","doubleSided":double_sided}));
            colors.insert(key, index);
            index
        };
        let mesh_index = meshes.len();
        let name = format!("Stroke_{}", stroke.id);
        meshes.push(json!({"name":name,"primitives":[{"attributes":{"POSITION":position,"NORMAL":normal},"indices":indices,"material":material,"mode":4}]}));
        nodes.push(json!({"name":name,"mesh":mesh_index,"extras":{"spatialSketchStrokeId":stroke.id.to_string()}}));
        summary.strokes += 1;
        summary.vertices += mesh.vertices.len();
        summary.triangles += mesh.indices.len() / 3;
    }
    if summary.strokes == 0 {
        return Err("No drawable strokes to export".into());
    }
    let scene_nodes: Vec<usize> = (0..nodes.len()).collect();
    let root = json!({"asset":{"version":"2.0","generator":concat!("Spatial Sketch ",env!("CARGO_PKG_VERSION"))},"scene":0,"scenes":[{"name":"Spatial Sketch drawing","nodes":scene_nodes}],"nodes":nodes,"meshes":meshes,"materials":materials,"accessors":accessors,"bufferViews":views,"buffers":[{"byteLength":binary.len()}],"extras":{"sourceFormat":format!("Spatial Sketch version {}",doc.version),"unitInMeters":1.0,"axes":"+X right, +Y up, +Z front","scope":if selected.is_some(){"selection"}else{"all strokes"}}});
    let mut json =
        serde_json::to_vec(&root).map_err(|e| format!("Could not encode export metadata: {e}"))?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let length = 12 + 8 + json.len() + 8 + binary.len();
    let total = u32::try_from(length).map_err(|_| "GLB exceeds its container size limit")?;
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&total.to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4E4F534A_u32.to_le_bytes());
    bytes.extend(json);
    bytes.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x004E4942_u32.to_le_bytes());
    bytes.extend(binary);
    summary.bytes = bytes.len();
    Ok(EncodedGlb { bytes, summary })
}
/// Protect an open native drawing even if it was given a nonstandard extension.
pub fn same_destination(a: &Path, b: &Path) -> bool {
    let resolve = |p: &Path| {
        std::fs::canonicalize(p)
            .or_else(|_| std::path::absolute(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let a = resolve(a);
    let b = resolve(b);
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
pub fn write_glb(
    doc: &Document,
    selected: Option<&HashSet<u64>>,
    path: &Path,
) -> Result<ExportSummary, String> {
    let encoded = encode(doc, selected)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("Could not create export file: {e}"))?;
    temp.write_all(&encoded.bytes)
        .map_err(|e| format!("Could not write export: {e}"))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| format!("Could not finish export: {e}"))?;
    temp.persist(path)
        .map_err(|e| format!("Could not replace export destination: {e}"))?;
    Ok(encoded.summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Point;
    fn stroke(id: u64, color: [f32; 3]) -> Stroke {
        Stroke {
            id,
            color,
            radius: 0.1,
            brush: Default::default(),
            points: vec![
                Point {
                    tilt: None,
                    position: Vec3::ZERO,
                    pressure: 0.2,
                },
                Point {
                    tilt: None,
                    position: Vec3::Y,
                    pressure: 1.0,
                },
            ],
        }
    }
    #[test]
    fn binary_roundtrip_preserves_pressure_geometry_material_and_outward_normals() {
        let doc = Document {
            version: 1,
            strokes: vec![stroke(7, [0.2, 0.6, 0.8])],
        };
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(exported.bytes.len() % 4, 0);
        assert_eq!(
            u32::from_le_bytes(exported.bytes[8..12].try_into().unwrap()) as usize,
            exported.bytes.len()
        );
        assert_eq!(gltf.nodes().count(), 1);
        let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
        let reader = primitive.reader(|_| gltf.blob.as_deref());
        let positions: Vec<Vec3> = reader.read_positions().unwrap().map(Vec3::from).collect();
        let normals: Vec<Vec3> = reader.read_normals().unwrap().map(Vec3::from).collect();
        let indices: Vec<u32> = reader.read_indices().unwrap().into_u32().collect();
        assert_eq!(
            primitive
                .material()
                .pbr_metallic_roughness()
                .base_color_factor(),
            [0.2, 0.6, 0.8, 1.0]
        );
        assert!(positions
            .iter()
            .filter(|p| p.y.abs() < 0.00001 && p.x.hypot(p.z) > 0.00001)
            .all(|p| (p.x.hypot(p.z) - 0.0296).abs() < 0.00001));
        assert!(positions
            .iter()
            .filter(|p| (p.y - 1.0).abs() < 0.00001)
            .any(|p| (p.x.hypot(p.z) - 0.1).abs() < 0.00001));
        for normal in &normals {
            assert!((normal.length() - 1.0).abs() < 0.00001);
        }
        for [a, b, c] in indices.as_chunks::<3>().0 {
            let face = (positions[*b as usize] - positions[*a as usize])
                .cross(positions[*c as usize] - positions[*a as usize]);
            assert!(
                face.dot(normals[*a as usize] + normals[*b as usize] + normals[*c as usize]) > 0.0
            );
        }
    }
    #[test]
    fn all_brush_types_export_with_shape_normals_and_distinct_sidedness() {
        let mut doc = crate::export_check::brush_fixture();
        // Equal colors must still have different materials when their sidedness differs.
        for stroke in &mut doc.strokes {
            stroke.color = [0.4; 3];
        }
        let source = doc.strokes.clone();
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(exported.summary.strokes, 6);
        assert_eq!(gltf.materials().count(), 2);
        for (mesh, stroke) in gltf.meshes().zip(&doc.strokes) {
            let primitive = mesh.primitives().next().unwrap();
            assert_eq!(
                primitive.material().double_sided(),
                stroke.brush.kind == BrushKind::Ribbon
            );
            let reader = primitive.reader(|_| gltf.blob.as_deref());
            let positions: Vec<Vec3> = reader.read_positions().unwrap().map(Vec3::from).collect();
            let normals: Vec<Vec3> = reader.read_normals().unwrap().map(Vec3::from).collect();
            let indices: Vec<u32> = reader.read_indices().unwrap().into_u32().collect();
            assert!(normals
                .iter()
                .all(|n| n.is_finite() && (n.length() - 1.0).abs() < 0.00001));
            for triangle in indices.as_chunks::<3>().0 {
                let [a, b, c] = triangle.map(|i| i as usize);
                let face = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
                assert!(face.dot(normals[a] + normals[b] + normals[c]) > 0.0);
            }
        }
        assert_eq!(doc.strokes, source);
        doc.strokes[0].brush.tangent = doc.strokes[0].brush.normal;
        assert!(encode(&doc, None).is_err());
        doc.strokes[0].brush.tangent = Vec3::X;
        doc.strokes[0].brush.thickness = 0.0;
        assert!(encode(&doc, None).is_err());
    }
    #[test]
    fn independent_pressure_dimensions_are_baked_into_glb() {
        let mut mark = stroke(9001, [0.2, 0.6, 0.8]);
        mark.brush.kind = BrushKind::Chisel;
        mark.brush.thickness = 0.5;
        mark.brush.dynamics.width.enabled = false;
        mark.brush.dynamics.thickness.minimum = 0.0;
        mark.brush.dynamics.thickness.curve = 2.0;
        let mut doc = Document::default();
        doc.strokes.push(mark);
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
        let positions: Vec<Vec3> = primitive
            .reader(|_| gltf.blob.as_deref())
            .read_positions()
            .unwrap()
            .map(Vec3::from)
            .collect();
        let first: Vec<_> = positions.iter().filter(|p| p.y.abs() < 1e-5).collect();
        let last: Vec<_> = positions
            .iter()
            .filter(|p| (p.y - 1.0).abs() < 1e-5)
            .collect();
        assert!(!first.is_empty() && !last.is_empty());
        assert!(first
            .iter()
            .all(|p| (p.x.abs() - 0.1).abs() < 1e-5 && (p.z.abs() - 0.002).abs() < 1e-5));
        assert!(last
            .iter()
            .all(|p| (p.x.abs() - 0.1).abs() < 1e-5 && (p.z.abs() - 0.05).abs() < 1e-5));
        doc.strokes[0].points[0].tilt = Some(Vec3::ZERO);
        assert!(encode(&doc, None).is_err());
        doc.strokes[0].points[0].tilt = None;
        doc.strokes[0].brush.dynamics.taper_start = f32::NAN;
        assert!(encode(&doc, None).is_err());
    }

    #[test]
    fn dynamics_survive_native_roundtrip_and_selected_export() {
        let source = crate::export_check::dynamics_fixture();
        let text = serde_json::to_string(&source).unwrap();
        let loaded = Document::from_json(&text).unwrap();
        assert_eq!(loaded.strokes, source.strokes);
        assert_eq!(loaded.version, 3);
        let exported = encode(&loaded, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 8);
        for (mesh, stroke) in gltf.meshes().zip(&loaded.strokes) {
            let primitive = mesh.primitives().next().unwrap();
            let positions: Vec<Vec3> = primitive
                .reader(|_| gltf.blob.as_deref())
                .read_positions()
                .unwrap()
                .map(Vec3::from)
                .collect();
            let expected = geometry::stroke_mesh(stroke);
            let bounds = |points: &[Vec3]| {
                points.iter().fold(
                    (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                    |(lo, hi), p| (lo.min(*p), hi.max(*p)),
                )
            };
            let expected_positions: Vec<_> =
                expected.iter().map(|v| Vec3::from(v.position)).collect();
            let (lo, hi) = bounds(&positions);
            let (expected_lo, expected_hi) = bounds(&expected_positions);
            assert!(lo.abs_diff_eq(expected_lo, 1e-5) && hi.abs_diff_eq(expected_hi, 1e-5));
        }
        let selected = encode(&loaded, Some(&HashSet::from([3005]))).unwrap();
        let parsed = gltf::Gltf::from_slice(&selected.bytes).unwrap();
        assert_eq!(parsed.meshes().count(), 1);
        assert_eq!(parsed.nodes().next().unwrap().name(), Some("Stroke_3005"));
        assert!(parsed.materials().next().unwrap().double_sided());
        assert_eq!(serde_json::to_string(&loaded).unwrap(), text);
    }
    #[test]
    fn selected_scope_uses_original_colors_and_excludes_other_strokes() {
        let doc = Document {
            version: 1,
            strokes: vec![stroke(1, [0.9, 0.1, 0.2]), stroke(2, [0.1, 0.8, 0.3])],
        };
        let original = doc.strokes.clone();
        let exported = encode(&doc, Some(&HashSet::from([2]))).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(exported.summary.strokes, 1);
        assert_eq!(gltf.nodes().next().unwrap().name(), Some("Stroke_2"));
        assert_eq!(
            gltf.materials()
                .next()
                .unwrap()
                .pbr_metallic_roughness()
                .base_color_factor(),
            [0.1, 0.8, 0.3, 1.0]
        );
        assert_eq!(doc.strokes, original);
    }
    #[test]
    fn dots_and_shared_materials_are_valid() {
        let mut a = stroke(1, [0.5; 3]);
        a.points.truncate(1);
        let doc = Document {
            version: 1,
            strokes: vec![a, stroke(2, [0.5; 3])],
        };
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 2);
        assert_eq!(gltf.materials().count(), 1);
    }
    #[test]
    fn failures_leave_existing_destination_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drawing.glb");
        std::fs::write(&path, b"keep this file").unwrap();
        assert!(same_destination(&path, &dir.path().join("./drawing.glb")));
        assert!(!same_destination(&path, &dir.path().join("other.glb")));
        assert!(write_glb(&Document::default(), None, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep this file");
        let mut doc = Document {
            version: 1,
            strokes: vec![stroke(1, [0.5; 3])],
        };
        doc.strokes[0].points[0].pressure = f32::NAN;
        assert!(write_glb(&doc, None, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep this file");
        doc.strokes[0].points[0].pressure = 0.3;
        let missing = dir.path().join("missing/drawing.glb");
        assert!(write_glb(&doc, None, &missing).is_err());
        assert!(!missing.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn success_atomically_replaces_destination_and_empty_selection_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drawing.glb");
        std::fs::write(&path, b"old").unwrap();
        let doc = Document {
            version: 1,
            strokes: vec![stroke(1, [0.5; 3])],
        };
        let result = write_glb(&doc, None, &path).unwrap();
        assert_eq!(
            result.bytes,
            std::fs::metadata(&path).unwrap().len() as usize
        );
        gltf::Gltf::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(encode(&doc, Some(&HashSet::new())).is_err());
    }
}
