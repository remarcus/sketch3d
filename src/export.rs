//! Self-contained glTF 2.0 binary export from semantic stroke data only.
use crate::{
    document::{BrushKind, Document, Stroke},
    geometry,
    texture::{self, TextureKind},
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
    vertices: Vec<[f32; 8]>,
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
    let mut unique = HashMap::<[u32; 8], u32>::new();
    for triangle in source.as_chunks::<3>().0 {
        let mut triangle = *triangle;
        if triangle.iter().any(|v| {
            !Vec3::from(v.position).is_finite()
                || !Vec3::from(v.normal).is_finite()
                || v.uv.iter().any(|u| !u.is_finite())
        }) {
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
            let data = [p.x, p.y, p.z, n.x, n.y, n.z, vertex.uv[0], vertex.uv[1]];
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
    if !matches!(doc.version, 1..=5) {
        return Err("Unsupported source document version".into());
    }
    let mut layer_ids = HashSet::new();
    let mut layer_names = HashSet::new();
    if doc.layers.is_empty()
        || doc.layers.len() > 256
        || doc.layers.iter().any(|l| {
            l.id == 0
                || l.id == u64::MAX
                || l.name.trim() != l.name
                || l.name.is_empty()
                || l.name.chars().count() > 64
                || l.name.chars().any(char::is_control)
                || !layer_names.insert(l.name.to_lowercase())
        })
        || doc.layers.iter().any(|l| !layer_ids.insert(l.id))
        || !layer_ids.contains(&doc.active_layer)
        || doc.strokes.iter().any(|s| !layer_ids.contains(&s.layer_id))
    {
        return Err("Invalid source layer references".into());
    }
    let mut binary = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut materials = Vec::new();
    let mut colors = HashMap::new();
    let mut image_keys = HashMap::new();
    let mut images = Vec::new();
    let mut textures = Vec::new();
    let mut ids = HashSet::new();
    let mut summary = ExportSummary {
        strokes: 0,
        vertices: 0,
        triangles: 0,
        bytes: 0,
    };
    for stroke in &doc.strokes {
        if !doc.visible(stroke)
            || selected.is_some_and(|selection| !selection.contains(&stroke.id))
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
            .checked_mul(32)
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
        views.push(json!({"buffer":0,"byteOffset":vertex_offset,"byteLength":mesh.vertices.len()*32,"byteStride":32,"target":34962}));
        let position = accessors.len();
        accessors.push(json!({"bufferView":vertex_view,"byteOffset":0,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3","min":mesh.min.to_array(),"max":mesh.max.to_array()}));
        let normal = accessors.len();
        accessors.push(json!({"bufferView":vertex_view,"byteOffset":12,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3"}));
        let uv = accessors.len();
        accessors.push(json!({"bufferView":vertex_view,"byteOffset":24,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC2"}));
        let index_offset = binary.len();
        for index in &mesh.indices {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        let index_view = views.len();
        views.push(json!({"buffer":0,"byteOffset":index_offset,"byteLength":mesh.indices.len()*4,"target":34963}));
        let indices = accessors.len();
        accessors.push(json!({"bufferView":index_view,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}));
        let double_sided = stroke.brush.kind == BrushKind::Ribbon;
        let settings = stroke.brush.texture;
        let texture_key = (settings.kind != TextureKind::Solid).then(|| texture::key(settings));
        let key = (stroke.color.map(f32::to_bits), double_sided, texture_key);
        let material = if let Some(index) = colors.get(&key) {
            *index
        } else {
            let index = materials.len();
            let mut material = json!({"name":format!("Ink {}",index+1),"pbrMetallicRoughness":{"baseColorFactor":[stroke.color[0],stroke.color[1],stroke.color[2],1.0],"metallicFactor":0.0,"roughnessFactor":0.8},"alphaMode":"OPAQUE","doubleSided":double_sided});
            if let Some(texture_key) = texture_key {
                let image_index = if let Some(index) = image_keys.get(&texture_key) {
                    *index
                } else {
                    let png = texture::png(settings)?;
                    while !binary.len().is_multiple_of(4) {
                        binary.push(0);
                    }
                    if binary.len().saturating_add(png.len()).saturating_add(3) > MAX_BINARY_BYTES {
                        return Err(
                            "Export exceeds the 128 MiB binary limit including textures".into()
                        );
                    }
                    let offset = binary.len();
                    binary.extend_from_slice(&png);
                    let view = views.len();
                    views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":png.len()}));
                    while !binary.len().is_multiple_of(4) {
                        binary.push(0);
                    }
                    let index = images.len();
                    images.push(json!({"name":format!("{} mask",settings.kind.label()),"bufferView":view,"mimeType":"image/png"}));
                    textures.push(json!({"sampler":0,"source":index}));
                    image_keys.insert(texture_key, index);
                    index
                };
                material["alphaMode"] = json!("MASK");
                material["alphaCutoff"] = json!(0.5);
                material["pbrMetallicRoughness"]["baseColorTexture"] =
                    json!({"index":image_index,"texCoord":0});
            }
            materials.push(material);
            colors.insert(key, index);
            index
        };
        let mesh_index = meshes.len();
        let name = format!("Stroke_{}", stroke.id);
        meshes.push(json!({"name":name,"primitives":[{"attributes":{"POSITION":position,"NORMAL":normal,"TEXCOORD_0":uv},"indices":indices,"material":material,"mode":4}]}));
        nodes.push(json!({"name":name,"mesh":mesh_index,"extras":{"spatialSketchStrokeId":stroke.id.to_string(),"spatialSketchLayerId":stroke.layer_id.to_string(),"spatialSketchLayerName":doc.layers.iter().find(|l|l.id==stroke.layer_id).unwrap().name}}));
        summary.strokes += 1;
        summary.vertices += mesh.vertices.len();
        summary.triangles += mesh.indices.len() / 3;
    }
    if summary.strokes == 0 {
        return Err("No drawable strokes to export".into());
    }
    let scene_nodes: Vec<usize> = (0..nodes.len()).collect();
    let mut root = json!({"asset":{"version":"2.0","generator":concat!("Spatial Sketch ",env!("CARGO_PKG_VERSION"))},"scene":0,"scenes":[{"name":"Spatial Sketch drawing","nodes":scene_nodes}],"nodes":nodes,"meshes":meshes,"materials":materials,"accessors":accessors,"bufferViews":views,"buffers":[{"byteLength":binary.len()}],"extras":{"sourceFormat":format!("Spatial Sketch version {}",doc.version),"unitInMeters":1.0,"axes":"+X right, +Y up, +Z front","scope":if selected.is_some(){"selection"}else{"all strokes"}}});
    if !images.is_empty() {
        root["images"] = json!(images);
        root["textures"] = json!(textures);
        root["samplers"] = json!([{"magFilter":9728,"minFilter":9728,"wrapS":10497,"wrapT":10497}]);
    }
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
            layer_id: 1,
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
            ..Document::default()
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
        assert_eq!(loaded.version, 5);
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
            ..Document::default()
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
            ..Document::default()
        };
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 2);
        assert_eq!(gltf.materials().count(), 1);
    }
    #[test]
    fn textured_glb_embeds_masks_and_preserves_uvs_and_coverage() {
        let doc = crate::export_check::textured_fixture();
        let original = doc.strokes.clone();
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.images().count(), 3);
        assert_eq!(gltf.textures().count(), 3);
        for image in gltf.images() {
            let gltf::image::Source::View { view, mime_type } = image.source() else {
                panic!("texture must be embedded")
            };
            assert_eq!(mime_type, "image/png");
            let bytes = &gltf.blob.as_ref().unwrap()[view.offset()..view.offset() + view.length()];
            let pixels = image::load_from_memory(bytes).unwrap().to_rgba8();
            assert_eq!(pixels.dimensions(), (texture::SIZE, texture::SIZE));
            let mut coverage = [0usize; 2];
            for pixel in pixels.pixels() {
                assert_eq!(&pixel.0[..3], &[255; 3]);
                assert!(matches!(pixel.0[3], 0 | 255));
                coverage[usize::from(pixel.0[3] == 255)] += 1;
            }
            assert!(coverage.iter().all(|n| *n > 0));
        }
        for (mesh, stroke) in gltf.meshes().zip(&doc.strokes) {
            let primitive = mesh.primitives().next().unwrap();
            let reader = primitive.reader(|_| gltf.blob.as_deref());
            let uvs: Vec<_> = reader.read_tex_coords(0).unwrap().into_f32().collect();
            assert!(!uvs.is_empty() && uvs.iter().flatten().all(|u| u.is_finite()));
            let material = primitive.material();
            assert_eq!(
                material.pbr_metallic_roughness().base_color_factor(),
                [stroke.color[0], stroke.color[1], stroke.color[2], 1.0]
            );
            if stroke.brush.texture.kind == TextureKind::Solid {
                assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Opaque);
                assert!(material
                    .pbr_metallic_roughness()
                    .base_color_texture()
                    .is_none());
            } else {
                assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Mask);
                assert_eq!(material.alpha_cutoff(), Some(0.5));
                let tex = material
                    .pbr_metallic_roughness()
                    .base_color_texture()
                    .unwrap();
                assert_eq!(tex.tex_coord(), 0);
                let sampler = tex.texture().sampler();
                assert_eq!(sampler.wrap_s(), gltf::texture::WrappingMode::Repeat);
                assert_eq!(sampler.wrap_t(), gltf::texture::WrappingMode::Repeat);
                assert_eq!(
                    sampler.mag_filter(),
                    Some(gltf::texture::MagFilter::Nearest)
                );
                assert_eq!(
                    sampler.min_filter(),
                    Some(gltf::texture::MinFilter::Nearest)
                );
            }
            let expected = geometry::stroke_mesh(stroke);
            for uv in &uvs {
                assert!(expected.iter().any(|v| v.uv == *uv));
            }
        }
        assert_eq!(doc.strokes, original);
    }
    #[test]
    fn masks_share_images_across_uv_scale_but_not_seeds_and_selection_is_self_contained() {
        let mut a = stroke(1, [0.5; 3]);
        a.brush.texture.kind = TextureKind::Grain;
        let mut b = a.clone();
        b.id = 2;
        b.brush.texture.scale *= 2.0;
        let mut c = a.clone();
        c.id = 3;
        c.brush.texture.seed += 1;
        let doc = Document {
            strokes: vec![a, b, c],
            ..Document::default()
        };
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.images().count(), 2);
        assert_eq!(gltf.materials().count(), 2);
        let selected = encode(&doc, Some(&HashSet::from([2]))).unwrap();
        let gltf = gltf::Gltf::from_slice(&selected.bytes).unwrap();
        assert_eq!(gltf.meshes().count(), 1);
        assert_eq!(gltf.images().count(), 1);
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, doc.strokes);
        assert_eq!(encode(&loaded, None).unwrap().bytes, exported.bytes);
        let mut legacy = doc.clone();
        legacy.version = 3;
        let migrated = Document::from_json(&serde_json::to_string(&legacy).unwrap()).unwrap();
        assert!(migrated
            .strokes
            .iter()
            .all(|s| s.brush.texture.kind == TextureKind::Solid));
        assert_eq!(
            gltf::Gltf::from_slice(&encode(&migrated, None).unwrap().bytes)
                .unwrap()
                .images()
                .count(),
            0
        );
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
            ..Document::default()
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
            ..Document::default()
        };
        let result = write_glb(&doc, None, &path).unwrap();
        assert_eq!(
            result.bytes,
            std::fs::metadata(&path).unwrap().len() as usize
        );
        gltf::Gltf::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(encode(&doc, Some(&HashSet::new())).is_err());
    }
    #[test]
    fn layer_visibility_filters_whole_and_selected_exports_but_locks_do_not() {
        let doc = crate::export_check::layered_fixture();
        let source = serde_json::to_string(&doc).unwrap();
        let encoded = encode(&doc, None).unwrap();
        let parsed = gltf::Gltf::from_slice(&encoded.bytes).unwrap();
        assert_eq!(encoded.summary.strokes, 2);
        assert_eq!(
            parsed
                .nodes()
                .map(|n| n.name().unwrap())
                .collect::<Vec<_>>(),
            vec!["Stroke_1001", "Stroke_1003"]
        );
        let hidden = HashSet::from([1002]);
        assert!(encode(&doc, Some(&hidden)).is_err());
        let locked = HashSet::from([1003]);
        assert_eq!(encode(&doc, Some(&locked)).unwrap().summary.strokes, 1);
        let loaded = Document::from_json(&source).unwrap();
        assert_eq!(loaded.layers, doc.layers);
        assert_eq!(loaded.strokes, doc.strokes);
        assert_eq!(encode(&loaded, None).unwrap().bytes, encoded.bytes);
        assert_eq!(serde_json::to_string(&doc).unwrap(), source);
        let mut invalid = doc;
        invalid.strokes[0].layer_id = 999;
        assert!(encode(&invalid, None).is_err());
    }
    #[test]
    fn revolved_vase_exports_as_independent_editable_ink() {
        let source = crate::export_check::vase_fixture();
        assert_eq!(source.strokes.len(), 73);
        let guide = crate::revolved::Revolved::default();
        for stroke in &source.strokes {
            assert!(stroke.brush.is_valid());
            for point in &stroke.points {
                assert!(point.is_valid());
                let local = guide.to_local(point.position);
                assert!((local.x.hypot(local.z) - guide.radius_at(local.y)).abs() < 1e-4);
            }
        }
        let native = serde_json::to_string(&source).unwrap();
        let loaded = Document::from_json(&native).unwrap();
        assert_eq!(loaded.strokes, source.strokes);
        let encoded = encode(&loaded, None).unwrap();
        let parsed = gltf::Gltf::from_slice(&encoded.bytes).unwrap();
        assert_eq!(parsed.meshes().count(), 73);
        assert!(encoded.summary.triangles > 10000);
        assert_eq!(encode(&source, None).unwrap().bytes, encoded.bytes);
        assert_eq!(serde_json::to_string(&source).unwrap(), native);
    }
}
