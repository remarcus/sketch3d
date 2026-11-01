//! End-to-end portable insert fixture for independent GLB viewer checks.
use super::native_merge;
use crate::{
    document::{Document, Point, Stroke},
    editing::Edit,
    image_plane_storage,
    image_planes::{self, ImagePlane},
    obj_reference::{self, ObjModel},
    obj_reference_storage,
};
use glam::{Quat, Vec3};
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::json;
use std::{collections::HashMap, io::Cursor, path::Path};

fn stroke(id: u64, color: [f32; 3], start_x: f32, end_x: f32) -> Stroke {
    Stroke {
        fill: None,
        material: Default::default(),
        group_id: None,
        layer_id: 1,
        id,
        color,
        radius: 0.08,
        brush: Default::default(),
        points: vec![
            Point {
                position: Vec3::new(start_x, 0.0, 0.0),
                pressure: 0.8,
                tilt: None,
            },
            Point {
                position: Vec3::new(end_x, 0.0, 0.0),
                pressure: 0.8,
                tilt: None,
            },
        ],
    }
}

pub fn run(output: &Path) -> Result<(), String> {
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let drawing_path = output.join("inserted.sketch.json");
    let glb_path = output.join("inserted.glb");
    let manifest_path = output.join("inserted-manifest.json");
    let destination_assets = image_plane_storage::asset_root(&drawing_path)?;
    if drawing_path.exists()
        || glb_path.exists()
        || manifest_path.exists()
        || destination_assets.exists()
    {
        return Err(format!(
            "Refusing to overwrite existing native-merge verification output in {}",
            output.display()
        ));
    }

    let source_workspace = tempfile::tempdir_in(output).map_err(|error| error.to_string())?;
    let source_path = source_workspace.path().join("portable-source.sketch.json");
    let source_assets = image_plane_storage::asset_root(&source_path)?;
    std::fs::create_dir_all(&source_assets).map_err(|error| error.to_string())?;

    let original_png = source_workspace.path().join("reference.png");
    let pixels = RgbaImage::from_fn(8, 4, |x, y| {
        Rgba([
            32 + x as u8 * 20,
            48 + y as u8 * 35,
            220,
            if x < 4 { 255 } else { 150 },
        ])
    });
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    std::fs::write(&original_png, png.into_inner()).map_err(|error| error.to_string())?;
    let (image_file, image) = image_planes::copy_asset(&original_png, &source_assets, 7)?;
    let mut plane = ImagePlane::from_asset(
        7,
        "Inserted reference",
        image_file,
        image.size,
        Vec3::new(0.5, 1.0, 0.0),
        Quat::from_rotation_y(0.2),
        2.0,
    )?;
    plane.drawable = true;

    let source_obj = source_workspace.path().join("tree.obj");
    std::fs::write(
        &source_obj,
        "mtllib tree.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Leaf\nf 1 2 3\n",
    )
    .map_err(|error| error.to_string())?;
    std::fs::write(
        source_workspace.path().join("tree.mtl"),
        "newmtl Leaf\nKd 0.1 0.8 0.2\nd 0.75\n",
    )
    .map_err(|error| error.to_string())?;
    let parsed_model: ObjModel = obj_reference::load(&source_obj)?;
    let model_asset = obj_reference_storage::import_bundle(&parsed_model, &source_assets, 8)?;

    let mut source = Document {
        strokes: vec![stroke(1, [0.1, 0.7, 0.3], -0.8, 0.8)],
        image_planes: vec![plane],
        active_image_plane: Some(7),
        ..Document::default()
    };
    source
        .model_references
        .push(crate::document::model_references::ObjReferenceResource {
            id: 8,
            name: "Tree".into(),
            visible: true,
            locked: false,
            center: Vec3::new(-1.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: 1.0,
            asset: model_asset,
        });
    std::fs::write(
        &source_path,
        serde_json::to_vec(&source).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let mut destination = Document {
        strokes: vec![stroke(1, [0.2, 0.3, 0.9], -1.5, -0.8)],
        ..Document::default()
    };
    let inserted = native_merge::prepare_insert(
        &destination,
        &source_path,
        &destination_assets,
        2,
        2,
        1,
        Vec3::new(2.0, -1.0, 0.5),
        false,
        &HashMap::new(),
    )?;
    let inserted_stroke_id = inserted.edit.strokes[0].id;
    let inserted_layer_id = inserted.edit.layers[0].id;
    let inserted_plane = inserted.edit.image_planes[0].clone();
    let inserted_model = inserted.edit.model_references[0].clone();
    let edit = Edit::Merge(inserted.edit);
    edit.apply(&mut destination, true);
    source_workspace
        .close()
        .map_err(|error| error.to_string())?;

    std::fs::write(
        &drawing_path,
        serde_json::to_vec_pretty(&destination).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let reloaded = Document::from_json(
        &std::fs::read_to_string(&drawing_path).map_err(|error| error.to_string())?,
    )?;
    let image_assets = image_plane_storage::load_assets(&reloaded, &drawing_path)?;
    if image_assets.get(&inserted_plane.id).map(|asset| asset.size) != Some([8, 4]) {
        return Err("Inserted image asset did not reload after removing the source bundle".into());
    }
    let model = obj_reference_storage::load_bundle(&destination_assets, &inserted_model.asset)?;
    if model.triangles.len() != 1 {
        return Err("Inserted OBJ asset did not reload after removing the source bundle".into());
    }
    let summary =
        crate::export::write_glb_with_images(&reloaded, None, &destination_assets, &glb_path)?;
    if summary.strokes != 2 || summary.image_planes != 1 || summary.model_references != 1 {
        return Err(format!("Unexpected merged GLB summary: {summary:?}"));
    }

    let manifest = json!({
        "passed": true,
        "glb": glb_path.file_name().unwrap().to_string_lossy(),
        "native": drawing_path.file_name().unwrap().to_string_lossy(),
        "source_bundle_removed_before_reload_and_export": true,
        "destination_stroke_id": "1",
        "inserted_stroke_id": inserted_stroke_id.to_string(),
        "inserted_layer_id": inserted_layer_id.to_string(),
        "inserted_image_plane_id": inserted_plane.id.to_string(),
        "inserted_image_size": image.size,
        "inserted_image_name": format!("Image_{}", inserted_plane.name),
        "inserted_model_id": inserted_model.id.to_string(),
        "inserted_model_name": format!("OBJ_{}", inserted_model.name),
        "inserted_model_triangle_count": model.triangles.len(),
        "export": summary,
    });
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "Native insert GLB fixture and manifest written to {}",
        output.display()
    );
    Ok(())
}
