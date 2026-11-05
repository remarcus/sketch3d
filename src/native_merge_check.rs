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
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    path::Path,
};

fn stroke(id: u64, color: [f32; 3], start_x: f32, end_x: f32) -> Stroke {
    Stroke {
        guide_attachment: None,
        model_surface: None,
        curve: None,
        point_thickness: None,
        point_colors: None,
        modifiers: None,
        fill: None,
        material: Default::default(),
        group_id: None,
        layer_id: 1,
        id,
        color,
        gradient: None,
        radius: 0.08,
        brush: Default::default(),
        points: vec![
            Point {
                position: Vec3::new(start_x, 0.0, 0.0),
                pressure: 0.8,
                tilt: None,
                time_ms: 0,
            },
            Point {
                position: Vec3::new(end_x, 0.0, 0.0),
                pressure: 0.8,
                tilt: None,
                time_ms: 0,
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
        linked_stroke_groups: Default::default(),
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
            live_line_art: None,
            asset: model_asset,
        });
    source
        .guide_resources
        .push(crate::document::guide_resources::GuideResource {
            id: 9,
            name: "Mixed transform guide".into(),
            visible: true,
            locked: false,
            geometry: crate::document::guide_resources::GuideGeometry::Cylinder {
                center: Vec3::new(0.0, 0.5, 0.25),
                rotation: Quat::from_rotation_y(0.15),
                radius: 0.35,
                height: 0.9,
            },
        });
    let selection = crate::mixed_resources::ResourceSelection {
        stroke_ids: HashSet::from([1]),
        image_plane_ids: HashSet::from([7]),
        obj_reference_ids: HashSet::from([8]),
        guide_ids: HashSet::from([9]),
    };
    let pivot = Vec3::new(0.1, -0.2, 0.35);
    let rotation = Quat::from_euler(glam::EulerRot::ZYX, 0.31, -0.18, 0.12);
    for transform in [
        crate::mixed_resources::TransformSpec::rotate(pivot, rotation),
        crate::mixed_resources::TransformSpec::uniform_scale(pivot, 1.35, true),
    ] {
        let snapshot = crate::mixed_resources::MoveSnapshot::capture_transform(
            &source,
            selection.clone(),
            transform,
        )
        .map_err(|error| error.to_string())?;
        snapshot
            .apply(&mut source, crate::mixed_resources::MoveDirection::Forward)
            .map_err(|error| error.to_string())?;
    }
    let transformed_source = source.clone();
    std::fs::write(
        &source_path,
        serde_json::to_vec(&source).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let mut destination = Document {
        linked_stroke_groups: Default::default(),
        strokes: vec![stroke(1, [0.2, 0.3, 0.9], -1.5, -0.8)],
        ..Document::default()
    };
    let insertion_offset = Vec3::new(2.0, -1.0, 0.5);
    let inserted = native_merge::prepare_insert(
        &destination,
        &source_path,
        &destination_assets,
        2,
        2,
        1,
        insertion_offset,
        false,
        &HashMap::new(),
    )?;
    let inserted_stroke_id = inserted.edit.strokes[0].id;
    let inserted_layer_id = inserted.edit.layers[0].id;
    let inserted_plane = inserted.edit.image_planes[0].clone();
    let inserted_model = inserted.edit.model_references[0].clone();
    let inserted_guide = inserted.edit.guides[0].clone();
    for (actual, expected) in inserted.edit.strokes[0]
        .points
        .iter()
        .zip(&transformed_source.strokes[0].points)
    {
        if actual
            .position
            .distance(expected.position + insertion_offset)
            > 1.0e-5
        {
            return Err("Merged rotated/scaled stroke differs from its transformed source".into());
        }
    }
    let transformed_plane = &transformed_source.image_planes[0];
    let transformed_model = &transformed_source.model_references[0];
    if inserted_plane
        .origin
        .distance(transformed_plane.origin + insertion_offset)
        > 1.0e-5
        || inserted_plane.rotation != transformed_plane.rotation
        || inserted_plane.size != transformed_plane.size
        || inserted_model
            .center
            .distance(transformed_model.center + insertion_offset)
            > 1.0e-5
        || inserted_model.rotation != transformed_model.rotation
        || (inserted_model.scale - transformed_model.scale).abs() > 1.0e-5
    {
        return Err("Merged image or OBJ transform differs from transformed source".into());
    }
    let transformed_guide_center = match &transformed_source.guide_resources[0].geometry {
        crate::document::guide_resources::GuideGeometry::Cylinder { center, .. } => *center,
        _ => return Err("Mixed merge guide fixture must be cylindrical".into()),
    };
    let inserted_guide_center = match &inserted_guide.geometry {
        crate::document::guide_resources::GuideGeometry::Cylinder { center, .. } => *center,
        _ => return Err("Merged mixed guide changed geometry kind".into()),
    };
    if inserted_guide_center.distance(transformed_guide_center + insertion_offset) > 1.0e-5 {
        return Err("Merged guide differs from transformed source".into());
    }
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
    if serde_json::to_value(&destination).map_err(|error| error.to_string())?
        != serde_json::to_value(&reloaded).map_err(|error| error.to_string())?
    {
        return Err("Native reload changed the merged rotated/scaled document".into());
    }
    let reloaded_plane = reloaded
        .image_planes
        .iter()
        .find(|resource| resource.id == inserted_plane.id)
        .ok_or("Merged image plane ID was not preserved by native reload")?;
    let reloaded_model = reloaded
        .model_references
        .iter()
        .find(|resource| resource.id == inserted_model.id)
        .ok_or("Merged OBJ reference ID was not preserved by native reload")?;
    let reloaded_guide = reloaded
        .guide_resources
        .iter()
        .find(|resource| resource.id == inserted_guide.id)
        .ok_or("Merged guide ID was not preserved by native reload")?;
    if reloaded_plane.origin != inserted_plane.origin
        || reloaded_plane.rotation != inserted_plane.rotation
        || reloaded_plane.size != inserted_plane.size
        || reloaded_model.center != inserted_model.center
        || reloaded_model.rotation != inserted_model.rotation
        || reloaded_model.scale != inserted_model.scale
        || reloaded_guide.geometry != inserted_guide.geometry
    {
        return Err("Native reload changed a rotated/scaled mixed resource".into());
    }
    let recovery_dir = tempfile::tempdir_in(output).map_err(|error| error.to_string())?;
    let recovery = crate::recovery_store::RecoverySnapshot::new(
        "mixed-transform-merge-check".into(),
        Some(drawing_path.clone()),
        reloaded.clone(),
    );
    crate::recovery_store::write(recovery_dir.path(), &recovery)
        .map_err(|error| error.to_string())?;
    let recovery_scan = crate::recovery_store::scan(recovery_dir.path(), "fresh-process")
        .map_err(|error| error.to_string())?;
    let recovered = recovery_scan
        .candidates
        .iter()
        .find(|candidate| candidate.snapshot.session_id == "mixed-transform-merge-check")
        .ok_or("Mixed-transform merged recovery snapshot was not rediscovered")?;
    if serde_json::to_value(&recovered.snapshot.document).map_err(|error| error.to_string())?
        != serde_json::to_value(&reloaded).map_err(|error| error.to_string())?
    {
        return Err("Recovery scan changed the merged rotated/scaled scene".into());
    }
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

    let glb_bytes = std::fs::read(&glb_path).map_err(|error| error.to_string())?;
    if glb_bytes.len() < 20
        || &glb_bytes[..4] != b"glTF"
        || u32::from_le_bytes(glb_bytes[4..8].try_into().unwrap()) != 2
        || u32::from_le_bytes(glb_bytes[8..12].try_into().unwrap()) as usize != glb_bytes.len()
        || &glb_bytes[16..20] != b"JSON"
    {
        return Err("Merged export is not a complete GLB 2.0 JSON chunk".into());
    }
    let glb_json_length = u32::from_le_bytes(glb_bytes[12..16].try_into().unwrap()) as usize;
    let glb_json_end = 20usize
        .checked_add(glb_json_length)
        .filter(|end| *end <= glb_bytes.len() && glb_json_length <= crate::export::MAX_BINARY_BYTES)
        .ok_or("Merged GLB JSON chunk exceeds its bounded file length")?;
    let glb_json: serde_json::Value =
        serde_json::from_slice(&glb_bytes[20..glb_json_end]).map_err(|error| error.to_string())?;
    if glb_json["extras"]["axes"] != "+X right, +Y up, +Z front" {
        return Err("Merged GLB does not declare the expected world axes".into());
    }
    let nodes = glb_json["nodes"]
        .as_array()
        .ok_or("Merged GLB node list is missing")?;
    for (id, key) in [
        (inserted_stroke_id.to_string(), "spatialSketchStrokeId"),
        (inserted_plane.id.to_string(), "spatialSketchImagePlaneId"),
        (inserted_model.id.to_string(), "spatialSketchObjReferenceId"),
    ] {
        if !nodes.iter().any(|node| node["extras"][key] == id) {
            return Err(format!("Merged GLB omitted transformed resource ID {id}"));
        }
    }
    let accessor_bounds = |accessor: &serde_json::Value| -> Result<(Vec3, Vec3, usize), String> {
        let vec3 = |value: &serde_json::Value| -> Result<Vec3, String> {
            let values = value.as_array().ok_or("GLB position bounds are missing")?;
            if values.len() != 3 {
                return Err("GLB position bounds are not VEC3".into());
            }
            Ok(Vec3::new(
                values[0].as_f64().ok_or("Invalid GLB bound")? as f32,
                values[1].as_f64().ok_or("Invalid GLB bound")? as f32,
                values[2].as_f64().ok_or("Invalid GLB bound")? as f32,
            ))
        };
        Ok((
            vec3(&accessor["min"])?,
            vec3(&accessor["max"])?,
            accessor["count"]
                .as_u64()
                .ok_or("GLB accessor count is missing")? as usize,
        ))
    };
    let accessor_for_id = |id: &str, id_key: &str| -> Result<&serde_json::Value, String> {
        let node = nodes
            .iter()
            .find(|node| node["extras"][id_key] == id)
            .ok_or_else(|| format!("GLB resource ID {id} is missing"))?;
        let mesh = node["mesh"].as_u64().ok_or("GLB node mesh is missing")? as usize;
        let primitive = &glb_json["meshes"][mesh]["primitives"][0];
        let position = primitive["attributes"]["POSITION"]
            .as_u64()
            .ok_or("GLB POSITION accessor is missing")? as usize;
        Ok(&glb_json["accessors"][position])
    };
    let model_accessor = accessor_for_id(
        &inserted_model.id.to_string(),
        "spatialSketchObjReferenceId",
    )?;
    let (model_min, model_max, model_count) = accessor_bounds(model_accessor)?;
    let source_vertices = model
        .render_vertices(&model.triangles[0])
        .map_err(|error| error.to_string())?;
    let expected_model_positions: Vec<Vec3> = source_vertices
        .iter()
        .map(|vertex| {
            inserted_model.center
                + inserted_model
                    .rotation
                    .mul_vec3(vertex.position * inserted_model.scale)
        })
        .collect();
    let expected_model_min = expected_model_positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let expected_model_max = expected_model_positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    if model_count != expected_model_positions.len()
        || model_min.distance(expected_model_min) > 1.0e-5
        || model_max.distance(expected_model_max) > 1.0e-5
    {
        return Err(
            "Merged GLB OBJ positions/bounds do not match the transformed reference".into(),
        );
    }
    let image_accessor =
        accessor_for_id(&inserted_plane.id.to_string(), "spatialSketchImagePlaneId")?;
    let (image_min, image_max, image_count) = accessor_bounds(image_accessor)?;
    let expected_image_corners: Vec<Vec3> = [-0.5, 0.5]
        .into_iter()
        .flat_map(|x| {
            [-0.5, 0.5].into_iter().map(move |y| {
                inserted_plane.origin
                    + inserted_plane.rotation
                        * Vec3::new(inserted_plane.size.x * x, inserted_plane.size.y * y, 0.0)
            })
        })
        .collect();
    let expected_image_min = expected_image_corners
        .iter()
        .copied()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let expected_image_max = expected_image_corners
        .iter()
        .copied()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    if image_count != expected_image_corners.len()
        || image_min.distance(expected_image_min) > 1.0e-5
        || image_max.distance(expected_image_max) > 1.0e-5
    {
        return Err("Merged GLB image bounds do not match the transformed plane".into());
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
        "inserted_guide_id": inserted_guide.id.to_string(),
        "mixed_transform": {"rotation_euler_zyx_radians":[0.31,-0.18,0.12],"uniform_scale":1.35,"pivot":pivot.to_array(),"guide_reloaded":true,"recovery_scan_preserved_scene":true,"glb_axes":"+X right, +Y up, +Z front"},
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
