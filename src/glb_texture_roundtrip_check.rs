//! End-to-end GLB texture import, native persistence, and scene export check.

use crate::{
    document::{model_references::ObjReferenceResource, Document},
    obj_reference_storage,
};
use glam::{Quat, Vec3};
use serde_json::{json, Value};
use std::{fs, io::Cursor, path::Path};

const FIXTURE_ID: &str = "textured-glb-managed-native-roundtrip-v1";

pub(crate) fn fixture_glb() -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut rgba = image::RgbaImage::new(2, 2);
    rgba.put_pixel(0, 0, image::Rgba([240, 30, 20, 255]));
    rgba.put_pixel(1, 0, image::Rgba([20, 220, 40, 255]));
    rgba.put_pixel(0, 1, image::Rgba([30, 60, 240, 255]));
    rgba.put_pixel(1, 1, image::Rgba([230, 210, 40, 255]));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| format!("Could not encode texture fixture: {error}"))?;
    let png = png.into_inner();

    let positions: [[f32; 3]; 3] = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.5, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 1.0], [1.0, 1.0], [0.5, 0.0]];
    let mut bin = Vec::new();
    for row in positions {
        for value in row {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    for row in normals {
        for value in row {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    for row in uvs {
        for value in row {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    let index_offset = bin.len();
    for value in [0u16, 1, 2] {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let image_offset = bin.len();
    bin.extend_from_slice(&png);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    let doc = json!({
        "asset":{"version":"2.0","generator":"Sketch3D GLB texture round-trip fixture"},
        "scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0,"name":"Textured triangle"}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"indices":3,"material":0}]}],
        "materials":[{"name":"Four-color checker","pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,1.0],"baseColorTexture":{"index":0},"metallicFactor":0.0,"roughnessFactor":0.8}}],
        "textures":[{"source":0}],"images":[{"bufferView":4,"mimeType":"image/png"}],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":36,"byteStride":12,"target":34962},
            {"buffer":0,"byteOffset":36,"byteLength":36,"byteStride":12,"target":34962},
            {"buffer":0,"byteOffset":72,"byteLength":24,"byteStride":8,"target":34962},
            {"buffer":0,"byteOffset":index_offset,"byteLength":6,"target":34963},
            {"buffer":0,"byteOffset":image_offset,"byteLength":png.len()}
        ],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[-1.0,0.0,0.0],"max":[1.0,1.5,0.0]},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":2,"componentType":5126,"count":3,"type":"VEC2"},
            {"bufferView":3,"componentType":5123,"count":3,"type":"SCALAR"}
        ]
    });
    let mut json_chunk = serde_json::to_vec(&doc).map_err(|error| error.to_string())?;
    while !json_chunk.len().is_multiple_of(4) {
        json_chunk.push(b' ');
    }
    let total = 12 + 8 + json_chunk.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x46546c67u32.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4e4f534au32.to_le_bytes());
    glb.extend_from_slice(&json_chunk);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004e4942u32.to_le_bytes());
    glb.extend_from_slice(&bin);
    Ok((glb, png))
}

fn write_preview(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| format!("Could not write PNG preview: {error}"))
}

pub fn run(output: &Path) -> Result<(), String> {
    fs::create_dir_all(output)
        .map_err(|error| format!("Could not create output directory: {error}"))?;
    let fixture_path = output.join("textured-glb-fixture.glb");
    let native_path = output.join("roundtrip.sketch.json");
    let exported_path = output.join("whole-scene.glb");
    let report_path = output.join("glb-texture-roundtrip-report.json");
    let preview_path = output.join("texture-preview.png");
    let (fixture, expected_png) = fixture_glb()?;
    fs::write(&fixture_path, &fixture).map_err(|error| error.to_string())?;
    write_preview(&preview_path, &expected_png)?;

    let (asset, imported) = obj_reference_storage::import_source_bundle(
        &fixture_path,
        &crate::image_plane_storage::asset_root(&native_path)?,
        42,
    )?;
    let imported_texture = imported
        .materials
        .iter()
        .find_map(|material| material.base_color_texture.as_ref())
        .ok_or("Fixture import did not retain a decoded base-color texture")?;
    if imported_texture.size != [2, 2] || imported_texture.pixels.len() != 16 {
        return Err("Imported texture dimensions or pixels do not match the fixture".into());
    }
    if imported.materials.iter().all(|material| {
        material.base_color_embedded_image.is_none() && material.base_color_texture.is_none()
    }) {
        return Err("Imported textured material metadata is missing".into());
    }
    let resource = ObjReferenceResource {
        id: 42,
        name: "Four-color GLB".into(),
        visible: true,
        locked: false,
        center: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: 1.0,
        live_line_art: None,
        asset,
    };
    let mut document = Document::default();
    document.model_references.push(resource);
    document.active_model_reference = Some(42);
    let native_bytes = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
    fs::write(&native_path, native_bytes)
        .map_err(|error| format!("Could not save native fixture: {error}"))?;
    let reopened: Document =
        serde_json::from_slice(&fs::read(&native_path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("Could not reopen native fixture: {error}"))?;
    if reopened.model_references != document.model_references
        || reopened.active_model_reference != Some(42)
    {
        return Err("Native save/reopen changed the imported GLB reference".into());
    }

    let root = crate::image_plane_storage::asset_root(&native_path)?;
    let summary = crate::export::write_glb_with_images(&reopened, None, &root, &exported_path)?;
    let output_bytes = fs::read(&exported_path).map_err(|error| error.to_string())?;
    let output_gltf = gltf::Gltf::from_slice(&output_bytes)
        .map_err(|error| format!("Whole-scene export is invalid: {error}"))?;
    let textures: Vec<_> = output_gltf
        .materials()
        .filter_map(|m| m.pbr_metallic_roughness().base_color_texture())
        .collect();
    if summary.model_references != 1 || textures.is_empty() {
        return Err("Whole-scene GLB did not include the textured imported model".into());
    }
    let output_png = output_gltf
        .images()
        .next()
        .and_then(|image| match image.source() {
            gltf::image::Source::View {
                view,
                mime_type: "image/png",
            } => output_gltf
                .blob
                .as_ref()
                .map(|blob| blob[view.offset()..view.offset() + view.length()].to_vec()),
            _ => None,
        })
        .ok_or("Whole-scene export did not embed the PNG texture")?;
    let source_image = image::load_from_memory(&expected_png)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let exported_image = image::load_from_memory(&output_png)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    if source_image != exported_image {
        return Err("Whole-scene export changed the fixture texture pixels".into());
    }

    let checks = ["textured GLB imported with UVs and decoded base-color pixels", "managed GLB asset saved and reopened through native JSON persistence", "whole-scene GLB export retained model geometry, material texture, and exact texture pixels"];
    let report: Value = json!({"passed":true,"structural_outcome":"passed","rendering_outcome":"not_run","rendering_note":"Interactive viewport rendering is tracked separately from this structural import/save/export check.","fixture_id":FIXTURE_ID,"checks":checks,"imported_texture_size":[2,2],"exported_models":summary.model_references,"exported_textures":textures.len(),"texture_pixels_exact":true,"artifacts":["texture-preview.png","whole-scene.glb","roundtrip.sketch.json"]});
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!("GLB texture round-trip passed: import, native save/reopen, whole-scene textured export; report {}", report_path.display());
    Ok(())
}
