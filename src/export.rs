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

pub(crate) const MAX_BINARY_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const MAX_TRIANGLE_VERTICES_PER_STROKE: usize = 2_000_000;
#[derive(Clone, Debug, Serialize)]
pub struct ExportSummary {
    pub strokes: usize,
    pub image_planes: usize,
    pub model_references: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub bytes: usize,
}
pub struct EncodedGlb {
    pub bytes: Vec<u8>,
    pub summary: ExportSummary,
}
#[cfg(test)]
mod fill_tests {
    use super::*;
    #[test]
    fn selected_fill_exports_double_sided_offset_geometry_and_rejects_invalid_contours() {
        let mut doc = crate::polygon::study();
        let selected = HashSet::from([doc.strokes[0].id]);
        let encoded = encode(&doc, Some(&selected)).unwrap();
        assert_eq!(encoded.summary.strokes, 1);
        let parsed = gltf::Gltf::from_slice(&encoded.bytes).unwrap();
        let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
        assert!(primitive.material().double_sided());
        let positions: Vec<_> = primitive
            .reader(|_| parsed.blob.as_deref())
            .read_positions()
            .unwrap()
            .collect();
        assert!(positions.iter().all(|p| (p[2] - 0.897).abs() < 1e-6));
        let len = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&encoded.bytes[20..20 + len]).unwrap();
        assert_eq!(
            json["nodes"][0]["extras"]["spatialSketchGeometry"],
            "planar_fill"
        );
        doc.strokes[0].points[2].position.z += 0.2;
        assert!(encode(&doc, Some(&selected)).is_err());
    }
}
pub(crate) struct Mesh {
    pub(crate) vertices: Vec<[f32; 12]>,
    pub(crate) indices: Vec<u32>,
    pub(crate) min: Vec3,
    pub(crate) max: Vec3,
}

pub(crate) fn stroke_mesh(stroke: &Stroke) -> Result<Mesh, String> {
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
    if stroke.fill.is_some() {
        crate::polygon::triangulate(stroke)?;
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
    let mut unique = HashMap::<[u32; 12], u32>::new();
    for triangle in source.as_chunks::<3>().0 {
        let mut triangle = *triangle;
        if triangle.iter().any(|v| {
            !Vec3::from(v.position).is_finite()
                || !Vec3::from(v.normal).is_finite()
                || v.uv.iter().any(|u| !u.is_finite())
                || !v.opacity.is_finite()
                || !(0.0..=1.0).contains(&v.opacity)
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
            let data = [
                p.x,
                p.y,
                p.z,
                n.x,
                n.y,
                n.z,
                vertex.uv[0],
                vertex.uv[1],
                1.0,
                1.0,
                1.0,
                vertex.opacity,
            ];
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
fn ensure_binary_budget(current: usize, additional: usize, label: &str) -> Result<(), String> {
    if current
        .checked_add(additional)
        .is_none_or(|size| size > MAX_BINARY_BYTES)
    {
        Err(format!(
            "Export exceeds the 128 MiB binary limit including {label}"
        ))
    } else {
        Ok(())
    }
}

pub fn encode(doc: &Document, selected: Option<&HashSet<u64>>) -> Result<EncodedGlb, String> {
    encode_with_images(doc, selected, None)
}

/// Encode visible managed image planes as textured scene geometry when a bundle
/// root is supplied. Selected-stroke export always omits images.
pub fn encode_with_images(
    doc: &Document,
    selected: Option<&HashSet<u64>>,
    image_root: Option<&Path>,
) -> Result<EncodedGlb, String> {
    if !matches!(doc.version, 1..=13) {
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
    let mut group_layers = HashMap::new();
    for stroke in &doc.strokes {
        if let Some(group) = stroke.group_id {
            if group == 0
                || group == u64::MAX
                || group_layers
                    .insert(group, stroke.layer_id)
                    .is_some_and(|layer| layer != stroke.layer_id)
            {
                return Err("Invalid source group membership".into());
            }
        }
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
    let mut image_samplers = false;
    let mut uses_unlit = false;
    let mut ids = HashSet::new();
    let mut summary = ExportSummary {
        strokes: 0,
        image_planes: 0,
        model_references: 0,
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
            .checked_mul(48)
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
        views.push(json!({"buffer":0,"byteOffset":vertex_offset,"byteLength":mesh.vertices.len()*48,"byteStride":48,"target":34962}));
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
        let double_sided = stroke.fill.is_some() || stroke.brush.kind == BrushKind::Ribbon;
        let settings = stroke.brush.texture;
        let texture_key = (settings.kind != TextureKind::Solid).then(|| texture::key(settings));
        let alpha_blended = stroke.brush.opacity < 1.0
            || (stroke.fill.is_none() && stroke.brush.softness > 0.0)
            || stroke.material == crate::document::StrokeMaterial::Cutout
            || (stroke.brush.dynamics.opacity.enabled
                && stroke.brush.dynamics.opacity.minimum < 1.0);
        let key = (
            stroke.color.map(f32::to_bits),
            double_sided,
            texture_key,
            alpha_blended,
            stroke.material,
        );
        let material = if let Some(index) = colors.get(&key) {
            *index
        } else {
            let index = materials.len();
            let material_alpha = if stroke.material == crate::document::StrokeMaterial::Cutout {
                0.0
            } else {
                1.0
            };
            let mut material = json!({"name":format!("Ink {}",index+1),"pbrMetallicRoughness":{"baseColorFactor":[stroke.color[0],stroke.color[1],stroke.color[2],material_alpha],"metallicFactor":0.0,"roughnessFactor":0.8},"alphaMode":if alpha_blended {"BLEND"} else if texture_key.is_some() {"MASK"} else {"OPAQUE"},"doubleSided":double_sided});
            match stroke.material {
                crate::document::StrokeMaterial::Shaded => {}
                crate::document::StrokeMaterial::Unlit => {
                    uses_unlit = true;
                    material["extensions"] = json!({"KHR_materials_unlit":{}});
                }
                crate::document::StrokeMaterial::Emissive => {
                    uses_unlit = true;
                    material["extensions"] = json!({"KHR_materials_unlit":{}});
                    material["emissiveFactor"] = json!(stroke.color);
                }
                crate::document::StrokeMaterial::Cutout => {}
            }
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
                if !alpha_blended {
                    material["alphaCutoff"] = json!(0.5);
                }
                material["pbrMetallicRoughness"]["baseColorTexture"] =
                    json!({"index":image_index,"texCoord":0});
            }
            materials.push(material);
            colors.insert(key, index);
            index
        };
        let mesh_index = meshes.len();
        let name = format!("Stroke_{}", stroke.id);
        let color = accessors.len();
        accessors.push(json!({"bufferView":vertex_view,"byteOffset":32,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC4"}));
        meshes.push(json!({"name":name,"primitives":[{"attributes":{"POSITION":position,"NORMAL":normal,"TEXCOORD_0":uv,"COLOR_0":color},"indices":indices,"material":material,"mode":4}]}));
        let mut node = json!({"name":name,"mesh":mesh_index,"extras":{"spatialSketchStrokeId":stroke.id.to_string(),"spatialSketchLayerId":stroke.layer_id.to_string(),"spatialSketchLayerName":doc.layers.iter().find(|l|l.id==stroke.layer_id).unwrap().name}});
        node["extras"]["spatialSketchMaterial"] = json!(match stroke.material {
            crate::document::StrokeMaterial::Shaded => "shaded",
            crate::document::StrokeMaterial::Unlit => "unlit",
            crate::document::StrokeMaterial::Emissive => "emissive",
            crate::document::StrokeMaterial::Cutout => "cutout",
        });
        node["extras"]["spatialSketchCastsShadows"] =
            json!(stroke.material == crate::document::StrokeMaterial::Shaded);
        if stroke.material == crate::document::StrokeMaterial::Cutout {
            node["extras"]["spatialSketchCompositingFallback"] =
                json!("fully_transparent_geometry_destination_not_erased");
        }
        if let Some(fill) = stroke.fill {
            node["extras"]["spatialSketchGeometry"] = json!("planar_fill");
            node["extras"]["spatialSketchFillOffset"] = json!(fill.offset);
        }
        if let Some(group) = stroke.group_id {
            node["extras"]["spatialSketchGroupId"] = json!(group.to_string());
        }
        nodes.push(node);
        summary.strokes += 1;
        summary.vertices += mesh.vertices.len();
        summary.triangles += mesh.indices.len() / 3;
    }
    if selected.is_none() {
        if let Some(image_root) = image_root {
            for plane in doc.image_planes.iter().filter(|plane| plane.visible) {
                plane.validate()?;
                let path = plane.resolve_asset(image_root)?.ok_or_else(|| {
                    format!(
                        "Visible image '{}' is missing from its asset bundle",
                        plane.name
                    )
                })?;
                let asset = crate::image_planes::decode_asset(&path)?;
                if plane.pixel_size != Some(asset.size) {
                    return Err(format!(
                        "Image '{}' dimensions do not match the drawing metadata",
                        plane.name
                    ));
                }
                let rgba = image::RgbaImage::from_raw(asset.size[0], asset.size[1], asset.pixels)
                    .ok_or("Could not construct decoded image plane")?;
                let has_alpha = plane.opacity < 1.0 || rgba.pixels().any(|pixel| pixel[3] < 255);
                let mut png = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(rgba)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|error| format!("Could not encode image '{}': {error}", plane.name))?;
                let png = png.into_inner();
                while !binary.len().is_multiple_of(4) {
                    binary.push(0);
                }
                if binary.len().saturating_add(png.len()).saturating_add(320) > MAX_BINARY_BYTES {
                    return Err(
                        "Export exceeds the 128 MiB binary limit including image planes".into(),
                    );
                }
                let png_offset = binary.len();
                binary.extend_from_slice(&png);
                let png_view = views.len();
                views.push(json!({"buffer":0,"byteOffset":png_offset,"byteLength":png.len()}));
                while !binary.len().is_multiple_of(4) {
                    binary.push(0);
                }
                let image_index = images.len();
                images.push(json!({"name":format!("Image {}",plane.name),"bufferView":png_view,"mimeType":"image/png"}));
                let texture_index = textures.len();
                textures.push(json!({"sampler":1,"source":image_index}));
                image_samplers = true;
                let material_index = materials.len();
                materials.push(json!({
                    "name": format!("Image {}", plane.name),
                    "pbrMetallicRoughness": {
                        "baseColorFactor": [1.0, 1.0, 1.0, plane.opacity],
                        "baseColorTexture": {"index": texture_index, "texCoord": 0},
                        "metallicFactor": 0.0,
                        "roughnessFactor": 1.0
                    },
                    "alphaMode": if has_alpha {"BLEND"} else {"OPAQUE"},
                    "doubleSided": true
                }));

                let half = plane.size * 0.5;
                let corners = [
                    (Vec3::new(-half.x, half.y, 0.0), [0.0_f32, 0.0_f32]),
                    (Vec3::new(-half.x, -half.y, 0.0), [0.0, 1.0]),
                    (Vec3::new(half.x, -half.y, 0.0), [1.0, 1.0]),
                    (Vec3::new(half.x, half.y, 0.0), [1.0, 0.0]),
                ];
                let normal = plane.rotation * Vec3::Z;
                let vertex_offset = binary.len();
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for (local, uv) in corners {
                    let position = plane.origin + plane.rotation * local;
                    if !position.is_finite() || position.abs().max_element() > 20_000.0 {
                        return Err(format!(
                            "Image '{}' exceeds export coordinate bounds",
                            plane.name
                        ));
                    }
                    min = min.min(position);
                    max = max.max(position);
                    for value in [
                        position.x, position.y, position.z, normal.x, normal.y, normal.z, uv[0],
                        uv[1],
                    ] {
                        binary.extend_from_slice(&value.to_le_bytes());
                    }
                }
                let vertex_view = views.len();
                views.push(json!({"buffer":0,"byteOffset":vertex_offset,"byteLength":128,"byteStride":32,"target":34962}));
                let position_accessor = accessors.len();
                accessors.push(json!({"bufferView":vertex_view,"byteOffset":0,"componentType":5126,"count":4,"type":"VEC3","min":min.to_array(),"max":max.to_array()}));
                let normal_accessor = accessors.len();
                accessors.push(json!({"bufferView":vertex_view,"byteOffset":12,"componentType":5126,"count":4,"type":"VEC3"}));
                let uv_accessor = accessors.len();
                accessors.push(json!({"bufferView":vertex_view,"byteOffset":24,"componentType":5126,"count":4,"type":"VEC2"}));
                let index_offset = binary.len();
                for index in [0_u32, 1, 2, 0, 2, 3] {
                    binary.extend_from_slice(&index.to_le_bytes());
                }
                let index_view = views.len();
                views.push(
                    json!({"buffer":0,"byteOffset":index_offset,"byteLength":24,"target":34963}),
                );
                let index_accessor = accessors.len();
                accessors.push(
                    json!({"bufferView":index_view,"componentType":5125,"count":6,"type":"SCALAR"}),
                );
                let mesh_index = meshes.len();
                meshes.push(json!({"name":format!("Image_{}",plane.id),"primitives":[{"attributes":{"POSITION":position_accessor,"NORMAL":normal_accessor,"TEXCOORD_0":uv_accessor},"indices":index_accessor,"material":material_index,"mode":4}]}));
                nodes.push(json!({"name":format!("Image_{}",plane.name),"mesh":mesh_index,"extras":{"spatialSketchImagePlaneId":plane.id.to_string(),"spatialSketchAsset":plane.asset_file,"spatialSketchOpacity":plane.opacity}}));
                summary.image_planes += 1;
                summary.vertices += 4;
                summary.triangles += 2;
            }

            crate::document::model_references::validate_catalog(
                &doc.model_references,
                doc.active_model_reference,
            )?;
            for resource in doc
                .model_references
                .iter()
                .filter(|resource| resource.visible)
            {
                let model = crate::obj_reference_storage::load_bundle(image_root, &resource.asset)
                    .map_err(|error| {
                        format!("Cannot load visible OBJ model '{}': {error}", resource.name)
                    })?;
                let mut material_groups = std::collections::BTreeMap::<
                    Option<u16>,
                    Vec<&crate::obj_reference::ObjTriangle>,
                >::new();
                for triangle in &model.triangles {
                    material_groups
                        .entry(triangle.material)
                        .or_default()
                        .push(triangle);
                }
                let mut primitives = Vec::with_capacity(material_groups.len());
                for (material_id, triangles) in material_groups {
                    let triangle_count = triangles.len();
                    let vertex_count = triangle_count
                        .checked_mul(3)
                        .ok_or("OBJ export vertex count overflow")?;
                    let vertex_bytes = vertex_count
                        .checked_mul(32)
                        .ok_or("OBJ export vertex size overflow")?;
                    let index_bytes = vertex_count
                        .checked_mul(4)
                        .ok_or("OBJ export index size overflow")?;
                    let required = vertex_bytes
                        .checked_add(index_bytes)
                        .and_then(|bytes| bytes.checked_add(192))
                        .ok_or("OBJ export size overflow")?;
                    ensure_binary_budget(binary.len(), required, "OBJ models")?;

                    let mut min = Vec3::splat(f32::INFINITY);
                    let mut max = Vec3::splat(f32::NEG_INFINITY);
                    let vertex_offset = binary.len();
                    for triangle in &triangles {
                        let vertices = model.render_vertices(triangle)?;
                        for source in &vertices {
                            let position = resource.center
                                + resource.rotation * (source.position * resource.scale);
                            let normal = (resource.rotation * source.normal).normalize_or_zero();
                            let uv = source.texcoord.unwrap_or(glam::Vec2::ZERO);
                            if !position.is_finite()
                                || position.abs().max_element() > 10_000.0
                                || !normal.is_finite()
                                || normal.length_squared() < 0.99
                                || !uv.is_finite()
                            {
                                return Err(format!(
                                    "OBJ model '{}' has invalid transformed geometry",
                                    resource.name
                                ));
                            }
                            min = min.min(position);
                            max = max.max(position);
                            for value in [
                                position.x, position.y, position.z, normal.x, normal.y, normal.z,
                                uv.x, uv.y,
                            ] {
                                binary.extend_from_slice(&value.to_le_bytes());
                            }
                        }
                    }
                    let vertex_view = views.len();
                    views.push(json!({"buffer":0,"byteOffset":vertex_offset,"byteLength":vertex_bytes,"byteStride":32,"target":34962}));
                    let position_accessor = accessors.len();
                    accessors.push(json!({"bufferView":vertex_view,"byteOffset":0,"componentType":5126,"count":vertex_count,"type":"VEC3","min":min.to_array(),"max":max.to_array()}));
                    let normal_accessor = accessors.len();
                    accessors.push(json!({"bufferView":vertex_view,"byteOffset":12,"componentType":5126,"count":vertex_count,"type":"VEC3"}));
                    let uv_accessor = accessors.len();
                    accessors.push(json!({"bufferView":vertex_view,"byteOffset":24,"componentType":5126,"count":vertex_count,"type":"VEC2"}));

                    let index_offset = binary.len();
                    for index in 0..vertex_count {
                        let index =
                            u32::try_from(index).map_err(|_| "OBJ export has too many vertices")?;
                        binary.extend_from_slice(&index.to_le_bytes());
                    }
                    let index_view = views.len();
                    views.push(json!({"buffer":0,"byteOffset":index_offset,"byteLength":index_bytes,"target":34963}));
                    let index_accessor = accessors.len();
                    accessors.push(json!({"bufferView":index_view,"componentType":5125,"count":vertex_count,"type":"SCALAR"}));

                    let material = model
                        .materials
                        .get(material_id.unwrap_or(u16::MAX) as usize);
                    let material_name =
                        material.map_or_else(|| "Default".to_owned(), |m| m.name.clone());
                    let diffuse = material.map_or([1.0; 3], |m| m.diffuse);
                    let opacity = material.map_or(1.0, |m| m.opacity);
                    let material_index = materials.len();
                    materials.push(json!({
                        "name": format!("OBJ {} - {}", resource.name, material_name),
                        "pbrMetallicRoughness": {
                            "baseColorFactor": [diffuse[0], diffuse[1], diffuse[2], opacity],
                            "metallicFactor": 0.0,
                            "roughnessFactor": 0.8
                        },
                        "alphaMode": if opacity < 0.999_999 {"BLEND"} else {"OPAQUE"},
                        "doubleSided": true
                    }));
                    let mesh_index = meshes.len();
                    meshes.push(json!({"name":format!("OBJ_{}_{}",resource.id,material_name),"primitives":[{"attributes":{"POSITION":position_accessor,"NORMAL":normal_accessor,"TEXCOORD_0":uv_accessor},"indices":index_accessor,"material":material_index,"mode":4}]}));
                    primitives.push(mesh_index);
                    summary.vertices += vertex_count;
                    summary.triangles += triangle_count;
                }
                let mesh = if primitives.len() == 1 {
                    primitives[0]
                } else {
                    let primitive_values: Vec<_> = primitives
                        .iter()
                        .map(|index| meshes[*index]["primitives"][0].clone())
                        .collect();
                    for index in primitives.iter().rev() {
                        meshes.remove(*index);
                    }
                    let mesh_index = meshes.len();
                    meshes.push(
                        json!({"name":format!("OBJ_{}",resource.id),"primitives":primitive_values}),
                    );
                    mesh_index
                };
                let mut extras = json!({
                    "spatialSketchObjReferenceId": resource.id.to_string(),
                    "spatialSketchAsset": resource.asset.bundle_dir,
                    "spatialSketchObjMaterials": "MTL Kd diffuse color and d/Tr opacity"
                });
                if !model.warnings.is_empty() {
                    extras["spatialSketchIgnoredObjMaterialProperties"] = json!(model.warnings);
                }
                nodes.push(
                    json!({"name":format!("OBJ_{}",resource.name),"mesh":mesh,"extras":extras}),
                );
                summary.model_references += 1;
            }
        }
    }
    if summary.strokes == 0 && summary.image_planes == 0 && summary.model_references == 0 {
        return Err("No drawable scene geometry to export".into());
    }
    let scene_nodes: Vec<usize> = (0..nodes.len()).collect();
    let scope = if selected.is_some() {
        "selected strokes (image planes and OBJ models omitted)"
    } else if image_root.is_some() {
        "all visible strokes, image planes, and OBJ models"
    } else {
        "all strokes"
    };
    let mut root = json!({"asset":{"version":"2.0","generator":concat!("Spatial Sketch ",env!("CARGO_PKG_VERSION"))},"scene":0,"scenes":[{"name":"Spatial Sketch drawing","nodes":scene_nodes}],"nodes":nodes,"meshes":meshes,"materials":materials,"accessors":accessors,"bufferViews":views,"buffers":[{"byteLength":binary.len()}],"extras":{"sourceFormat":format!("Spatial Sketch version {}",doc.version),"unitInMeters":1.0,"axes":"+X right, +Y up, +Z front","scope":scope}});
    if uses_unlit {
        root["extensionsUsed"] = json!(["KHR_materials_unlit"]);
    }
    if !images.is_empty() {
        root["images"] = json!(images);
        root["textures"] = json!(textures);
        let mut samplers =
            vec![json!({"magFilter":9728,"minFilter":9728,"wrapS":10497,"wrapT":10497})];
        if image_samplers {
            samplers.push(json!({"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071}));
        }
        root["samplers"] = json!(samplers);
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
    write_encoded_glb(encoded, path)
}

pub fn write_glb_with_images(
    doc: &Document,
    selected: Option<&HashSet<u64>>,
    image_root: &Path,
    path: &Path,
) -> Result<ExportSummary, String> {
    let encoded = encode_with_images(doc, selected, Some(image_root))?;
    write_encoded_glb(encoded, path)
}

fn write_encoded_glb(encoded: EncodedGlb, path: &Path) -> Result<ExportSummary, String> {
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
    crate::atomic_file::replace(temp, path)
        .map_err(|e| format!("Could not replace export destination: {e}"))?;
    Ok(encoded.summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Point, StrokeMaterial};
    fn stroke(id: u64, color: [f32; 3]) -> Stroke {
        Stroke {
            material: Default::default(),
            id,
            fill: None,
            group_id: None,
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
    fn material_modes_export_with_explicit_cutout_fallback_metadata() {
        let materials = [
            StrokeMaterial::Shaded,
            StrokeMaterial::Unlit,
            StrokeMaterial::Emissive,
            StrokeMaterial::Cutout,
        ];
        let doc = Document {
            version: 13,
            strokes: materials
                .into_iter()
                .enumerate()
                .map(|(index, material)| {
                    let mut mark = stroke(index as u64 + 1, [0.25, 0.5, 0.75]);
                    mark.material = material;
                    mark
                })
                .collect(),
            ..Document::default()
        };
        let encoded = encode(&doc, None).unwrap();
        let json_len = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&encoded.bytes[20..20 + json_len]).unwrap();
        assert_eq!(json["extensionsUsed"], json!(["KHR_materials_unlit"]));

        let mut nodes_by_material = std::collections::HashMap::new();
        for node in json["nodes"].as_array().unwrap() {
            nodes_by_material.insert(
                node["extras"]["spatialSketchMaterial"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                node,
            );
        }
        assert_eq!(nodes_by_material.len(), 4);
        assert_eq!(
            nodes_by_material["shaded"]["extras"]["spatialSketchCastsShadows"],
            true
        );
        assert_eq!(
            nodes_by_material["unlit"]["extras"]["spatialSketchCastsShadows"],
            false
        );
        assert_eq!(
            nodes_by_material["emissive"]["extras"]["spatialSketchCastsShadows"],
            false
        );
        assert_eq!(
            nodes_by_material["cutout"]["extras"]["spatialSketchCastsShadows"],
            false
        );
        assert_eq!(
            nodes_by_material["cutout"]["extras"]["spatialSketchCompositingFallback"],
            "fully_transparent_geometry_destination_not_erased"
        );
        let cutout_mesh = json["meshes"]
            [nodes_by_material["cutout"]["mesh"].as_u64().unwrap() as usize]["primitives"][0]
            ["material"]
            .as_u64()
            .unwrap() as usize;
        assert_eq!(
            json["materials"][cutout_mesh]["pbrMetallicRoughness"]["baseColorFactor"][3],
            0.0
        );
        for mode in ["unlit", "emissive"] {
            let mesh = json["meshes"][nodes_by_material[mode]["mesh"].as_u64().unwrap() as usize]
                ["primitives"][0]["material"]
                .as_u64()
                .unwrap() as usize;
            assert!(json["materials"][mesh]["extensions"]["KHR_materials_unlit"].is_object());
        }
        assert_eq!(
            json["materials"][2]["emissiveFactor"],
            json!([0.25, 0.5, 0.75])
        );

        let roundtrip = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(
            roundtrip
                .strokes
                .iter()
                .map(|mark| mark.material)
                .collect::<Vec<_>>(),
            materials
        );
        assert_eq!(encoded.summary.strokes, materials.len());
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
    fn opacity_dynamics_and_soft_coverage_export_as_vertex_alpha() {
        let mut a = stroke(9101, [0.2, 0.6, 0.8]);
        a.brush.opacity = 0.5;
        a.brush.softness = 0.4;
        a.brush.dynamics.opacity = crate::dynamics::PressureMapping {
            enabled: true,
            minimum: 0.2,
            curve: 1.0,
        };
        let mut b = a.clone();
        b.id += 1;
        b.brush.dynamics.opacity.minimum = 0.35;
        let mut c = a.clone();
        c.id += 2;
        c.brush.opacity = 0.7;
        let mut d = a.clone();
        d.id += 3;
        d.brush.opacity = 1.0;
        d.brush.softness = 0.0;
        d.brush.dynamics.opacity = crate::dynamics::PressureMapping {
            enabled: false,
            minimum: 1.0,
            curve: 1.0,
        };
        let doc = Document {
            strokes: vec![a, b, c, d],
            ..Document::default()
        };
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        assert_eq!(gltf.materials().count(), 2);
        let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
        let reader = primitive.reader(|_| gltf.blob.as_deref());
        let alpha: Vec<_> = reader
            .read_colors(0)
            .expect("COLOR_0 accessor")
            .into_rgba_f32()
            .collect();
        assert_eq!(
            primitive.material().alpha_mode(),
            gltf::material::AlphaMode::Blend
        );
        assert_eq!(
            primitive
                .material()
                .pbr_metallic_roughness()
                .base_color_factor(),
            [0.2, 0.6, 0.8, 1.0]
        );
        let alpha: Vec<_> = alpha.iter().map(|rgba| rgba[3]).collect();
        assert!(alpha.iter().all(|value| (0.0..=1.0).contains(value)));
        assert!(alpha.iter().any(|value| *value < 0.2));
        assert!(alpha.iter().any(|value| *value > 0.3));
        let next_primitive = gltf.meshes().nth(1).unwrap().primitives().next().unwrap();
        let next_alpha: Vec<_> = next_primitive
            .reader(|_| gltf.blob.as_deref())
            .read_colors(0)
            .unwrap()
            .into_rgba_f32()
            .collect();
        assert!(next_alpha.iter().any(|rgba| rgba[3] < 1.0));
        assert_eq!(
            gltf.meshes()
                .nth(3)
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .material()
                .alpha_mode(),
            gltf::material::AlphaMode::Opaque
        );
        assert!(gltf
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .attributes()
            .any(
                |(semantic, accessor)| semantic == gltf::mesh::Semantic::Colors(0)
                    && accessor.offset() == 32
                    && accessor.size() == 16
            ));
    }

    #[test]
    fn binary_texture_mask_combines_with_opacity_blend() {
        let mut mark = stroke(9201, [0.7, 0.4, 0.2]);
        mark.brush.opacity = 0.65;
        mark.brush.texture.kind = TextureKind::Grain;
        mark.brush.dynamics.opacity = crate::dynamics::PressureMapping {
            enabled: true,
            minimum: 0.25,
            curve: 1.0,
        };
        let doc = Document {
            strokes: vec![mark],
            ..Document::default()
        };
        let exported = encode(&doc, None).unwrap();
        let gltf = gltf::Gltf::from_slice(&exported.bytes).unwrap();
        let material = gltf.materials().next().unwrap();
        assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Blend);
        assert_eq!(
            material.pbr_metallic_roughness().base_color_factor(),
            [0.7, 0.4, 0.2, 1.0]
        );
        assert!(material
            .pbr_metallic_roughness()
            .base_color_texture()
            .is_some());
        assert_eq!(gltf.images().count(), 1);
        let length = u32::from_le_bytes(exported.bytes[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&exported.bytes[20..20 + length]).unwrap();
        assert_eq!(json["materials"][0]["alphaMode"], "BLEND");
        assert!(json["materials"][0].get("alphaCutoff").is_none());
    }

    #[test]
    fn dynamics_survive_native_roundtrip_and_selected_export() {
        let source = crate::export_check::dynamics_fixture();
        let text = serde_json::to_string(&source).unwrap();
        let loaded = Document::from_json(&text).unwrap();
        assert_eq!(loaded.strokes, source.strokes);
        assert_eq!(loaded.version, 13);
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
        let mut normalized_source = doc.clone();
        normalized_source.version = loaded.version;
        assert_eq!(
            encode(&loaded, None).unwrap().bytes,
            encode(&normalized_source, None).unwrap().bytes
        );
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

#[cfg(test)]
mod group_export_tests {
    use super::*;
    #[test]
    fn group_metadata_roundtrips_without_merging_or_reinterpreting_strokes() {
        let doc = crate::export_check::grouped_fixture();
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.version, 13);
        assert_eq!(loaded.strokes, doc.strokes);
        let encoded = encode(&doc, None).unwrap();
        let parsed = gltf::Gltf::from_slice(&encoded.bytes).unwrap();
        assert_eq!(parsed.meshes().count(), 3);
        let length = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&encoded.bytes[20..20 + length]).unwrap();
        assert_eq!(json["nodes"][0]["extras"]["spatialSketchGroupId"], "999");
        assert_eq!(json["nodes"][1]["extras"]["spatialSketchGroupId"], "999");
        assert!(json["nodes"][2]["extras"]
            .get("spatialSketchGroupId")
            .is_none());
        let selected = encode(&doc, Some(&HashSet::from([doc.strokes[0].id]))).unwrap();
        assert_eq!(selected.summary.strokes, 1);
        let mut independent = doc.clone();
        for stroke in &mut independent.strokes {
            stroke.group_id = None;
        }
        let base = encode(&independent, None).unwrap();
        assert_eq!(encoded.summary.vertices, base.summary.vertices);
        assert_eq!(encoded.summary.triangles, base.summary.triangles);
        assert_eq!(loaded.strokes, doc.strokes);
    }
    #[test]
    fn invalid_group_links_are_rejected_even_when_outside_export_selection() {
        let mut doc = crate::export_check::grouped_fixture();
        let mut layer = doc.layers[0].clone();
        layer.id = 2;
        layer.name = "Other".into();
        doc.layers.push(layer);
        doc.strokes[1].layer_id = 2;
        assert!(encode(&doc, Some(&HashSet::from([doc.strokes[2].id]))).is_err());
        doc.strokes[1].layer_id = 1;
        doc.strokes[0].group_id = Some(0);
        assert!(encode(&doc, None).is_err());
    }
}

#[cfg(test)]
mod image_plane_export_tests {
    use super::*;
    use crate::image_planes::ImagePlane;
    use glam::{Quat, Vec2};
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    #[test]
    fn whole_scene_glb_embeds_visible_image_plane_and_keeps_selected_export_ink_only() {
        let directory = tempfile::tempdir().unwrap();
        let image_root = directory.path().join("drawing.sketch.json.assets");
        let source_path = directory.path().join("source.png");
        let source_image = RgbaImage::from_pixel(2, 1, Rgba([40, 120, 200, 128]));
        let mut source_bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source_image)
            .write_to(&mut source_bytes, ImageFormat::Png)
            .unwrap();
        std::fs::write(&source_path, source_bytes.into_inner()).unwrap();
        let (asset_file, asset) =
            crate::image_planes::copy_asset(&source_path, &image_root, 7).unwrap();
        let rotation = Quat::from_rotation_y(0.35);
        let mut plane = ImagePlane::from_asset(
            7,
            "Facade",
            asset_file,
            asset.size,
            Vec3::new(1.0, 2.0, 3.0),
            rotation,
            3.0,
        )
        .unwrap();
        plane.opacity = 0.75;
        let document = Document {
            image_planes: vec![plane.clone()],
            ..Document::default()
        };

        let encoded = encode_with_images(&document, None, Some(&image_root)).unwrap();
        assert_eq!(encoded.summary.image_planes, 1);
        assert_eq!(encoded.summary.vertices, 4);
        assert!(encoded.summary.triangles > 1);
        let parsed = gltf::Gltf::from_slice(&encoded.bytes).unwrap();
        assert_eq!(parsed.meshes().count(), 1);
        assert_eq!(parsed.images().count(), 1);
        assert!(parsed
            .blob
            .as_ref()
            .is_some_and(|blob| blob.windows(8).any(|window| window == b"\x89PNG\r\n\x1a\n")));

        let json_length = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&encoded.bytes[20..20 + json_length]).unwrap();
        assert_eq!(
            json["extras"]["scope"],
            "all visible strokes, image planes, and OBJ models"
        );
        assert_eq!(json["nodes"][0]["extras"]["spatialSketchImagePlaneId"], "7");
        assert_eq!(
            json["nodes"][0]["extras"]["spatialSketchAsset"],
            plane.asset_file
        );
        assert_eq!(json["materials"][0]["alphaMode"], "BLEND");
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"][3],
            0.75
        );

        let position_accessor = json["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .unwrap() as usize;
        let half = plane.size * 0.5;
        let corners = [
            Vec3::new(-half.x, half.y, 0.0),
            Vec3::new(-half.x, -half.y, 0.0),
            Vec3::new(half.x, -half.y, 0.0),
            Vec3::new(half.x, half.y, 0.0),
        ];
        let points: Vec<_> = corners
            .iter()
            .map(|corner| plane.origin + plane.rotation * *corner)
            .collect();
        let minimum = points.iter().copied().reduce(Vec3::min).unwrap();
        let maximum = points.iter().copied().reduce(Vec3::max).unwrap();
        let gltf_min = json["accessors"][position_accessor]["min"]
            .as_array()
            .unwrap();
        let gltf_max = json["accessors"][position_accessor]["max"]
            .as_array()
            .unwrap();
        for axis in 0..3 {
            assert!((gltf_min[axis].as_f64().unwrap() as f32 - minimum[axis]).abs() < 1e-5);
            assert!((gltf_max[axis].as_f64().unwrap() as f32 - maximum[axis]).abs() < 1e-5);
        }

        let mut selected_document = crate::scenes::japanese_house();
        selected_document.image_planes.push(plane.clone());
        let selected_id = HashSet::from([selected_document.strokes[0].id]);
        let selected_encoded =
            encode_with_images(&selected_document, Some(&selected_id), Some(&image_root)).unwrap();
        assert_eq!(selected_encoded.summary.image_planes, 0);
        assert!(gltf::Gltf::from_slice(&selected_encoded.bytes)
            .unwrap()
            .images()
            .next()
            .is_none());
        plane.visible = false;
        let mut hidden = crate::scenes::japanese_house();
        hidden.image_planes.push(plane);
        let hidden_encoded = encode_with_images(&hidden, None, Some(&image_root)).unwrap();
        assert_eq!(hidden_encoded.summary.image_planes, 0);
        assert!(gltf::Gltf::from_slice(&hidden_encoded.bytes)
            .unwrap()
            .images()
            .next()
            .is_none());
    }

    #[test]
    fn whole_scene_image_export_rejects_missing_visible_assets() {
        let directory = tempfile::tempdir().unwrap();
        let plane = ImagePlane::from_asset(
            3,
            "Missing",
            "image-3.png".into(),
            [2, 1],
            Vec3::ZERO,
            Quat::IDENTITY,
            2.0,
        )
        .unwrap();
        assert_eq!(plane.size, Vec2::new(2.0, 1.0));
        let document = Document {
            image_planes: vec![plane],
            ..Document::default()
        };
        let error = encode_with_images(&document, None, Some(directory.path()))
            .err()
            .unwrap();
        assert!(error.contains("is missing from its asset bundle"));
    }
}

#[cfg(test)]
mod obj_reference_export_tests {
    use super::*;
    use crate::document::Point;
    use glam::Quat;

    fn imported_model(
        root: &Path,
        id: u64,
    ) -> crate::document::model_references::ObjReferenceResource {
        let source_root = root.join("source");
        std::fs::create_dir_all(&source_root).unwrap();
        let obj = source_root.join("sample.obj");
        let mtl = source_root.join("sample.mtl");
        std::fs::write(
            &mtl,
            "newmtl Brick\nKd 0.8 0.2 0.1\nd 0.4\nNs 32\nmap_Kd ignored.png\n",
        )
        .unwrap();
        std::fs::write(
            &obj,
            "mtllib sample.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nusemtl Brick\nf 1/1/1 2/2/1 3/3/1\n",
        )
        .unwrap();
        let asset_root = root.join("managed");
        let bundle = asset_root.join("models").join(id.to_string());
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            bundle.join("model.obj"),
            "mtllib material-000.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nusemtl Brick\nf 1/1/1 2/2/1 3/3/1\n",
        )
        .unwrap();
        std::fs::copy(&mtl, bundle.join("material-000.mtl")).unwrap();
        let asset = crate::obj_reference_storage::ObjReferenceAsset {
            id,
            bundle_dir: format!("models/{id}"),
        };
        crate::document::model_references::ObjReferenceResource {
            id,
            name: "Brick sample".into(),
            visible: true,
            locked: false,
            center: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: 2.0,
            asset,
        }
    }

    fn sample_stroke() -> Stroke {
        Stroke {
            fill: None,
            material: Default::default(),
            group_id: None,
            layer_id: 1,
            id: 9,
            color: [0.2, 0.7, 0.9],
            radius: 0.05,
            brush: Default::default(),
            points: vec![
                Point {
                    tilt: None,
                    position: Vec3::ZERO,
                    pressure: 1.0,
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
    fn whole_scene_glb_exports_transformed_obj_geometry_materials_and_selected_ink_stays_ink_only()
    {
        let directory = tempfile::tempdir().unwrap();
        let resource = imported_model(directory.path(), 77);
        let asset_root = directory.path().join("managed");
        let document = Document {
            strokes: vec![sample_stroke()],
            model_references: vec![resource.clone()],
            ..Document::default()
        };

        let encoded = encode_with_images(&document, None, Some(&asset_root)).unwrap();
        assert_eq!(encoded.summary.strokes, 1);
        assert_eq!(encoded.summary.model_references, 1);
        assert!(encoded.summary.triangles > 1);
        let parsed = gltf::Gltf::from_slice(&encoded.bytes).unwrap();
        assert_eq!(parsed.meshes().count(), 2);
        assert_eq!(parsed.materials().count(), 2);
        let model_node = parsed
            .nodes()
            .find(|node| node.name() == Some("OBJ_Brick sample"))
            .unwrap();
        let primitive = model_node.mesh().unwrap().primitives().next().unwrap();
        let positions: Vec<_> = primitive
            .reader(|_| parsed.blob.as_deref())
            .read_positions()
            .unwrap()
            .collect();
        assert_eq!(positions.len(), 3);
        let expected = [[1.0, 2.0, 3.0], [1.0, 4.0, 3.0], [-1.0, 2.0, 3.0]];
        for (actual, expected) in positions.iter().zip(expected) {
            assert!(glam::Vec3::from(*actual).distance(Vec3::from(expected)) < 1e-5);
        }
        let normals: Vec<_> = primitive
            .reader(|_| parsed.blob.as_deref())
            .read_normals()
            .unwrap()
            .collect();
        assert!(normals
            .iter()
            .all(|normal| { glam::Vec3::from(*normal).distance(Vec3::Z) < 1e-5 }));
        let uvs: Vec<_> = primitive
            .reader(|_| parsed.blob.as_deref())
            .read_tex_coords(0)
            .unwrap()
            .into_f32()
            .collect();
        assert_eq!(uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);

        let json_length = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&encoded.bytes[20..20 + json_length]).unwrap();
        let node = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["extras"]["spatialSketchObjReferenceId"] == "77")
            .unwrap();
        assert_eq!(
            node["extras"]["spatialSketchAsset"],
            resource.asset.bundle_dir
        );
        assert_eq!(
            node["extras"]["spatialSketchObjMaterials"],
            "MTL Kd diffuse color and d/Tr opacity"
        );
        assert_eq!(
            node["extras"]["spatialSketchIgnoredObjMaterialProperties"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let material_index = json["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"]
            [0]["material"]
            .as_u64()
            .unwrap() as usize;
        let factors = json["materials"][material_index]["pbrMetallicRoughness"]["baseColorFactor"]
            .as_array()
            .unwrap();
        for (actual, expected) in factors.iter().zip([0.8_f64, 0.2, 0.1, 0.4]) {
            assert!((actual.as_f64().unwrap() - expected).abs() < 1e-6);
        }
        assert_eq!(json["materials"][material_index]["alphaMode"], "BLEND");
        assert_eq!(json["materials"][material_index]["doubleSided"], true);
        assert_eq!(
            json["extras"]["scope"],
            "all visible strokes, image planes, and OBJ models"
        );

        let translation = Vec3::new(-0.25, 0.5, 1.25);
        let mut moved_document = document.clone();
        let movement = crate::mixed_resources::MoveSnapshot::capture(
            &moved_document,
            crate::mixed_resources::ResourceSelection {
                stroke_ids: HashSet::from([9]),
                obj_reference_ids: HashSet::from([77]),
                ..Default::default()
            },
            crate::mixed_resources::TransformMode::Translate,
            translation,
        )
        .unwrap();
        movement
            .apply(
                &mut moved_document,
                crate::mixed_resources::MoveDirection::Forward,
            )
            .unwrap();
        let moved_glb = encode_with_images(&moved_document, None, Some(&asset_root)).unwrap();
        let moved_parsed = gltf::Gltf::from_slice(&moved_glb.bytes).unwrap();
        let moved_node = moved_parsed
            .nodes()
            .find(|node| node.name() == Some("OBJ_Brick sample"))
            .unwrap();
        let moved_primitive = moved_node.mesh().unwrap().primitives().next().unwrap();
        let moved_positions: Vec<_> = moved_primitive
            .reader(|_| moved_parsed.blob.as_deref())
            .read_positions()
            .unwrap()
            .collect();
        for (actual, original) in moved_positions.iter().zip(expected) {
            assert!(glam::Vec3::from(*actual).distance(Vec3::from(original) + translation) < 1e-5);
        }

        let model_only = Document {
            model_references: vec![resource.clone()],
            ..Document::default()
        };
        let model_only_glb = encode_with_images(&model_only, None, Some(&asset_root)).unwrap();
        assert_eq!(model_only_glb.summary.strokes, 0);
        assert_eq!(model_only_glb.summary.model_references, 1);
        assert_eq!(
            gltf::Gltf::from_slice(&model_only_glb.bytes)
                .unwrap()
                .meshes()
                .count(),
            1
        );

        let selected =
            encode_with_images(&document, Some(&HashSet::from([9])), Some(&asset_root)).unwrap();
        assert_eq!(selected.summary.strokes, 1);
        assert_eq!(selected.summary.model_references, 0);
        let selected_json_length =
            u32::from_le_bytes(selected.bytes[12..16].try_into().unwrap()) as usize;
        let selected_json: serde_json::Value =
            serde_json::from_slice(&selected.bytes[20..20 + selected_json_length]).unwrap();
        assert_eq!(selected_json["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(
            selected_json["extras"]["scope"],
            "selected strokes (image planes and OBJ models omitted)"
        );
    }

    #[test]
    fn hidden_models_are_omitted_and_missing_visible_models_do_not_replace_output() {
        let directory = tempfile::tempdir().unwrap();
        let resource = imported_model(directory.path(), 88);
        let asset_root = directory.path().join("managed");
        let mut hidden = resource.clone();
        hidden.visible = false;
        hidden.id = 89;
        hidden.asset.bundle_dir = "models/89".into();
        hidden.asset.id = 89;
        let document = Document {
            strokes: vec![sample_stroke()],
            model_references: vec![hidden.clone()],
            ..Document::default()
        };
        let encoded = encode_with_images(&document, None, Some(&asset_root)).unwrap();
        assert_eq!(encoded.summary.model_references, 0);
        assert_eq!(
            gltf::Gltf::from_slice(&encoded.bytes)
                .unwrap()
                .meshes()
                .count(),
            1
        );

        let too_many_models = (0..=crate::document::model_references::MAX_MODEL_REFERENCES)
            .map(|index| {
                let id = 1000 + index as u64;
                let mut resource = hidden.clone();
                resource.id = id;
                resource.name = format!("Hidden {index}");
                resource.asset.id = id;
                resource.asset.bundle_dir = format!("models/{id}");
                resource
            })
            .collect();
        let oversized_catalog = Document {
            strokes: vec![sample_stroke()],
            model_references: too_many_models,
            ..Document::default()
        };
        assert!(
            encode_with_images(&oversized_catalog, None, Some(&asset_root))
                .err()
                .unwrap()
                .contains("at most 64 OBJ references")
        );

        let mut out_of_bounds = resource.clone();
        out_of_bounds.center = Vec3::splat(10_000.0);
        let invalid_geometry = Document {
            strokes: vec![sample_stroke()],
            model_references: vec![out_of_bounds],
            ..Document::default()
        };
        assert!(
            encode_with_images(&invalid_geometry, None, Some(&asset_root))
                .err()
                .unwrap()
                .contains("invalid transformed geometry")
        );

        let missing = Document {
            strokes: vec![sample_stroke()],
            model_references: vec![resource],
            ..Document::default()
        };
        let bundle = asset_root.join("models/88");
        std::fs::remove_dir_all(bundle).unwrap();
        let destination = directory.path().join("existing.glb");
        std::fs::write(&destination, b"keep previous export").unwrap();
        let error = write_glb_with_images(&missing, None, &asset_root, &destination).unwrap_err();
        assert!(error.contains("Cannot load visible OBJ model"));
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"keep previous export"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
    }
}
