//! Validated insertion of portable native drawings into an open document.
use crate::{
    document::{Document, Layer},
    editing::MergeEdit,
    image_planes,
    obj_reference::ObjModel,
};
use glam::Vec3;
use std::{collections::HashMap, fs, io::Read, path::Path};

const MAX_NATIVE_BYTES: u64 = 64 * 1024 * 1024;

pub struct InsertedDrawing {
    pub edit: MergeEdit,
    pub next_id: u64,
    pub image_assets: HashMap<u64, image_planes::ImageAsset>,
    pub obj_models: HashMap<u64, ObjModel>,
    pub brush_tip_assets: Vec<crate::brush_tip::BrushTipAsset>,
}

/// Validate a native source and stage its content and dependent resources for
/// insertion. The document itself is not mutated until this returns success.
#[allow(clippy::too_many_arguments)]
pub fn prepare_insert(
    destination: &Document,
    source_path: &Path,
    destination_assets: &Path,
    first_id: u64,
    first_layer_id: u64,
    first_group_id: u64,
    offset: Vec3,
    group_as_one: bool,
    destination_models: &HashMap<u64, ObjModel>,
) -> Result<InsertedDrawing, String> {
    if !offset.is_finite() || offset.abs().max_element() > 10_000.0 {
        return Err("Insertion offset must be finite and within document bounds".into());
    }
    let metadata = fs::symlink_metadata(source_path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_NATIVE_BYTES
    {
        return Err("Choose a regular native drawing no larger than 64 MiB".into());
    }
    let mut text = String::with_capacity(metadata.len() as usize);
    fs::File::open(source_path)
        .map_err(|error| error.to_string())?
        .take(MAX_NATIVE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    if text.len() as u64 > MAX_NATIVE_BYTES {
        return Err("Native drawing exceeds the 64 MiB document limit".into());
    }
    let mut source = Document::from_json(&text)?;
    if group_as_one
        && (source.layers.len() != 1
            || source
                .strokes
                .iter()
                .any(|stroke| stroke.group_id.is_some()))
    {
        return Err(
            "Single-group insertion requires one source layer with no existing groups".into(),
        );
    }
    let source_assets = crate::image_plane_storage::asset_root(source_path)?;

    // Missing/corrupt dependencies abort the entire insertion. This avoids
    // installing strokes or metadata that only partly survives save/reload.
    let mut image_sources = HashMap::new();
    for plane in &source.image_planes {
        let path = plane
            .resolve_asset(&source_assets)?
            .ok_or_else(|| format!("Image resource for '{}' is missing", plane.name))?;
        let decoded = image_planes::decode_asset(&path)?;
        if plane.pixel_size != Some(decoded.size) {
            return Err(format!(
                "Image resource for '{}' does not match its drawing metadata",
                plane.name
            ));
        }
        image_sources.insert(plane.id, path);
    }
    let mut model_sources: HashMap<u64, ObjModel> = HashMap::new();
    for model in &source.model_references {
        let loaded = crate::obj_reference_storage::load_bundle(&source_assets, &model.asset)
            .map_err(|error| format!("OBJ resource '{}' is unavailable: {error}", model.name))?;
        model_sources.insert(model.id, loaded);
    }
    let destination_asset_bytes =
        catalog_asset_bytes(destination, destination_assets, destination_models)?;
    let mut inserted_asset_bytes = 0_u64;
    for path in image_sources.values() {
        inserted_asset_bytes = inserted_asset_bytes
            .saturating_add(fs::metadata(path).map_err(|error| error.to_string())?.len());
    }
    for model in model_sources.values() {
        inserted_asset_bytes = inserted_asset_bytes.saturating_add(model_bundle_bytes(model)?);
    }
    if destination_asset_bytes.saturating_add(inserted_asset_bytes)
        > crate::recovery_store::MAX_ASSET_BYTES as u64
    {
        return Err("Insertion would exceed the 256 MiB total managed/recovery asset limit".into());
    }

    if destination
        .strokes
        .len()
        .saturating_add(source.strokes.len())
        > 100_000
    {
        return Err("Insertion would exceed the 100,000-stroke document limit".into());
    }
    let current_points: usize = destination
        .strokes
        .iter()
        .map(|stroke| stroke.points.len())
        .sum();
    let inserted_points: usize = source
        .strokes
        .iter()
        .map(|stroke| stroke.points.len())
        .sum();
    if current_points.saturating_add(inserted_points) > 2_000_000 {
        return Err("Insertion would exceed the 2,000,000-control-point document limit".into());
    }
    if destination.layers.len().saturating_add(source.layers.len()) > 256 {
        return Err("Insertion would exceed the 256-layer document limit".into());
    }

    let mut merged_tip_assets = destination.brush_tip_assets.clone();
    for asset in &source.brush_tip_assets {
        if let Some(existing) = merged_tip_assets
            .iter()
            .find(|existing| existing.id == asset.id)
        {
            if existing.png != asset.png {
                return Err("Brush tip content identity collision during insertion".into());
            }
        } else {
            merged_tip_assets.push(asset.clone());
        }
    }
    crate::brush_tip::validate_catalog(&merged_tip_assets)?;

    let mut ids = Ids::new(first_id, destination_assets.to_path_buf());
    let mut layer_ids = Ids::new(first_layer_id, destination_assets.to_path_buf());
    let mut group_ids = Ids::new(first_group_id, destination_assets.to_path_buf());
    let layer_map: HashMap<u64, u64> = source
        .layers
        .iter()
        .map(|layer| Ok((layer.id, layer_ids.take()?)))
        .collect::<Result<_, String>>()?;
    let mut used_layer_names: Vec<String> = destination
        .layers
        .iter()
        .map(|layer| layer.name.clone())
        .collect();
    let inserted_layers: Vec<Layer> = source
        .layers
        .iter()
        .map(|layer| {
            Ok(Layer {
                id: layer_map[&layer.id],
                name: unique_name(&layer.name, &mut used_layer_names, 64),
                visible: layer.visible,
                locked: layer.locked,
                opacity: layer.opacity,
                blend_mode: layer.blend_mode,
            })
        })
        .collect::<Result<_, String>>()?;

    let mut group_map = HashMap::new();
    for group in source.strokes.iter().filter_map(|stroke| stroke.group_id) {
        if let std::collections::hash_map::Entry::Vacant(entry) = group_map.entry(group) {
            entry.insert(group_ids.take()?);
        }
    }
    let top_group = group_as_one.then(|| group_ids.take()).transpose()?;
    for stroke in &mut source.strokes {
        stroke.id = ids.take()?;
        stroke.layer_id = layer_map[&stroke.layer_id];
        stroke.group_id = top_group.or_else(|| stroke.group_id.map(|group| group_map[&group]));
        for point in &mut stroke.points {
            point.position += offset;
            if !point.is_valid() {
                return Err(
                    "Insertion offset moves stroke geometry outside document bounds".into(),
                );
            }
        }
        if let Some(curve) = &mut stroke.curve {
            curve.frame_origin += offset;
            for anchor in &mut curve.anchors {
                anchor.position += offset;
            }
            if !curve.is_valid() {
                return Err("Insertion offset moves curve controls outside document bounds".into());
            }
        }
    }

    let image_map: HashMap<u64, u64> = source
        .image_planes
        .iter()
        .map(|plane| Ok((plane.id, ids.take()?)))
        .collect::<Result<_, String>>()?;
    let image_copy_sources: Vec<(u64, std::path::PathBuf)> = source
        .image_planes
        .iter()
        .map(|plane| {
            Ok((
                image_map[&plane.id],
                image_sources
                    .get(&plane.id)
                    .cloned()
                    .ok_or("Missing validated image source")?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let guide_map: HashMap<u64, u64> = source
        .guide_resources
        .iter()
        .map(|guide| Ok((guide.id, ids.take()?)))
        .collect::<Result<_, String>>()?;
    let model_map: HashMap<u64, u64> = source
        .model_references
        .iter()
        .map(|model| Ok((model.id, ids.take()?)))
        .collect::<Result<_, String>>()?;
    let model_copy_sources: Vec<(u64, ObjModel)> = source
        .model_references
        .iter()
        .map(|model| {
            Ok((
                model_map[&model.id],
                model_sources
                    .remove(&model.id)
                    .ok_or_else(|| format!("Missing validated OBJ source for '{}'", model.name))?,
            ))
        })
        .collect::<Result<_, String>>()?;

    for plane in &mut source.image_planes {
        let old_id = plane.id;
        plane.id = image_map[&old_id];
        plane.origin += offset;
        let extension = Path::new(&plane.asset_file)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        plane.asset_file = format!("image-{}.{}", plane.id, extension);
        if !plane.origin.is_finite() || plane.origin.abs().max_element() > 10_000.0 {
            return Err("Insertion offset moves an image plane outside document bounds".into());
        }
    }
    for guide in &mut source.guide_resources {
        guide.id = guide_map[&guide.id];
        translate_guide(&mut guide.geometry, offset);
    }
    let mut used_model_names: Vec<String> = destination
        .model_references
        .iter()
        .map(|model| model.name.clone())
        .collect();
    for model in &mut source.model_references {
        let old_id = model.id;
        model.id = model_map[&old_id];
        model.name = unique_name(&model.name, &mut used_model_names, 96);
        model.center += offset;
        model.asset.id = model.id;
        model.asset.bundle_dir = format!("models/{}", model.id);
        if !model.center.is_finite() || model.center.abs().max_element() > 10_000.0 {
            return Err("Insertion offset moves an OBJ reference outside document bounds".into());
        }
    }

    let _source_active_layer = layer_map
        .get(&source.active_layer)
        .copied()
        .ok_or("Source active layer is invalid")?;
    let edit = MergeEdit {
        strokes: source.strokes.clone(),
        layers: inserted_layers,
        image_planes: source.image_planes.clone(),
        guides: source.guide_resources.clone(),
        model_references: source.model_references.clone(),
        previous_active_layer: destination.active_layer,
        inserted_active_layer: destination.active_layer,
    };

    let mut merged = destination.clone();
    merged.brush_tip_assets = merged_tip_assets;
    crate::editing::Edit::Merge(edit.clone()).apply(&mut merged, true);
    // Run the combined catalogs through the same schema and bounds checks used
    // by normal native open/save before writing any destination assets.
    let serialized = serde_json::to_string(&merged).map_err(|error| error.to_string())?;
    Document::from_json(&serialized)?;

    let (image_assets, obj_models) =
        copy_dependencies(&image_copy_sources, &model_copy_sources, destination_assets)?;
    Ok(InsertedDrawing {
        edit,
        next_id: ids.next,
        image_assets,
        obj_models,
        brush_tip_assets: source.brush_tip_assets,
    })
}

fn catalog_asset_bytes(
    document: &Document,
    asset_root: &Path,
    models: &HashMap<u64, ObjModel>,
) -> Result<u64, String> {
    let mut total = 0_u64;
    for plane in &document.image_planes {
        if let Some(path) = plane.resolve_asset(asset_root)? {
            total =
                total.saturating_add(fs::metadata(path).map_err(|error| error.to_string())?.len());
        }
    }
    for model in &document.model_references {
        let loaded = models.get(&model.id).ok_or_else(|| {
            format!(
                "OBJ resource '{}' is not loaded in the destination drawing",
                model.name
            )
        })?;
        total = total.saturating_add(model_bundle_bytes(loaded)?);
    }
    Ok(total)
}

fn model_bundle_bytes(model: &ObjModel) -> Result<u64, String> {
    let mut total = fs::metadata(&model.source_obj)
        .map_err(|error| error.to_string())?
        .len();
    for path in &model.source_materials {
        total = total.saturating_add(fs::metadata(path).map_err(|error| error.to_string())?.len());
    }
    Ok(total)
}

type CopiedDependencies = (
    HashMap<u64, image_planes::ImageAsset>,
    HashMap<u64, ObjModel>,
);

fn copy_dependencies(
    image_sources: &[(u64, std::path::PathBuf)],
    model_sources: &[(u64, ObjModel)],
    destination_root: &Path,
) -> Result<CopiedDependencies, String> {
    let existed = fs::symlink_metadata(destination_root)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
    if fs::symlink_metadata(destination_root)
        .is_ok_and(|metadata| !metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return Err("Destination resource path is not a regular directory".into());
    }
    fs::create_dir_all(destination_root).map_err(|error| error.to_string())?;
    let mut created_images = Vec::new();
    let mut created_models = Vec::new();
    let mut image_assets = HashMap::new();
    let mut obj_models = HashMap::new();
    let install = (|| -> Result<(), String> {
        for (new_id, source) in image_sources {
            let (file, asset) = image_planes::copy_asset(source, destination_root, *new_id)?;
            created_images.push(destination_root.join(file));
            image_assets.insert(*new_id, asset);
        }
        for (new_id, source) in model_sources {
            let asset =
                crate::obj_reference_storage::import_bundle(source, destination_root, *new_id)?;
            created_models.push(asset.clone());
            let loaded = crate::obj_reference_storage::load_bundle(destination_root, &asset)?;
            obj_models.insert(*new_id, loaded);
        }
        Ok(())
    })();
    if let Err(error) = install {
        for path in created_images {
            let _ = fs::remove_file(path);
        }
        for asset in created_models {
            let _ = crate::obj_reference_storage::remove_bundle(destination_root, &asset);
        }
        if !existed
            && fs::read_dir(destination_root).is_ok_and(|mut entries| entries.next().is_none())
        {
            let _ = fs::remove_dir(destination_root);
        }
        return Err(error);
    }
    Ok((image_assets, obj_models))
}

fn translate_guide(geometry: &mut crate::document::guide_resources::GuideGeometry, offset: Vec3) {
    use crate::document::guide_resources::GuideGeometry as G;
    match geometry {
        G::Plane { origin, .. } => *origin += offset,
        G::Cylinder { center, .. }
        | G::Revolved { center, .. }
        | G::Sphere { center, .. }
        | G::Ring { center, .. }
        | G::Cone { center, .. } => *center += offset,
        G::Freeform { surface } => surface.center += offset,
        G::Loft { surface } => surface.center += offset,
    }
}

fn unique_name(name: &str, existing: &mut Vec<String>, max_chars: usize) -> String {
    let mut base: String = name.chars().take(max_chars.saturating_sub(12)).collect();
    if base.is_empty() {
        base = "Imported".into();
    }
    let mut candidate = base.clone();
    let mut index = 2_u32;
    while existing
        .iter()
        .any(|known| known.eq_ignore_ascii_case(&candidate))
    {
        let suffix = format!(" (import {index})");
        let keep = max_chars.saturating_sub(suffix.chars().count());
        candidate = format!("{}{}", base.chars().take(keep).collect::<String>(), suffix);
        index = index.saturating_add(1);
    }
    existing.push(candidate.clone());
    candidate
}

struct Ids {
    next: u64,
    asset_root: std::path::PathBuf,
}

impl Ids {
    fn new(next: u64, asset_root: std::path::PathBuf) -> Self {
        Self {
            next: next.max(1),
            asset_root,
        }
    }
    fn take(&mut self) -> Result<u64, String> {
        for _ in 0..1_000_000 {
            if self.next >= u64::MAX - 1 {
                return Err("No more document IDs are available for insertion".into());
            }
            let current = self.next;
            self.next += 1;
            let models = self.asset_root.join("models").join(current.to_string());
            let png = self.asset_root.join(format!("image-{current}.png"));
            let jpg = self.asset_root.join(format!("image-{current}.jpg"));
            if fs::symlink_metadata(models).is_ok()
                || fs::symlink_metadata(png).is_ok()
                || fs::symlink_metadata(jpg).is_ok()
            {
                continue;
            }
            return Ok(current);
        }
        Err("Could not reserve collision-free IDs for inserted assets".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{model_references::ObjReferenceResource, Brush, Point, Stroke},
        image_planes::ImagePlane,
    };
    use glam::Quat;
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    #[test]
    fn prepared_insert_remaps_ids_offsets_and_undoes_as_one_merge() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source_path = root.path().join("source.sketch.json");
        let asset_root = root.path().join("destination-assets");
        fs::create_dir_all(&asset_root).unwrap();
        let mut source = Document::default();
        source.strokes.push(Stroke {
            curve: None,
            id: 1,
            layer_id: 1,
            group_id: None,
            material: Default::default(),
            fill: None,
            color: [0.4, 0.5, 0.6],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::new(1.0, 2.0, 3.0),
                pressure: 0.4,
                tilt: None,

                time_ms: 0,
            }],
        });
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();

        let destination = Document::default();
        let original_json = serde_json::to_value(&destination).unwrap();
        let inserted = prepare_insert(
            &destination,
            &source_path,
            &asset_root,
            100,
            10,
            200,
            Vec3::new(2.0, -1.0, 0.5),
            true,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(&destination).unwrap(), original_json);
        assert_eq!(inserted.edit.layers[0].id, 10);
        assert_eq!(inserted.edit.layers[0].name, "Layer 1 (import 2)");

        let edit = crate::editing::Edit::Merge(inserted.edit.clone());
        let mut merged = destination.clone();
        edit.apply(&mut merged, true);
        let stroke = &merged.strokes[0];
        assert_eq!(stroke.id, 100);
        assert_eq!(stroke.layer_id, 10);
        assert_eq!(stroke.group_id, Some(200));
        assert_eq!(stroke.points[0].position, Vec3::new(3.0, 1.0, 3.5));
        assert!(inserted.next_id > stroke.id);
        let reloaded = Document::from_json(&serde_json::to_string(&merged).unwrap()).unwrap();
        assert_eq!(reloaded.strokes[0].id, stroke.id);
        assert_eq!(reloaded.strokes[0].group_id, stroke.group_id);
        assert_eq!(
            reloaded.strokes[0].points[0].position,
            stroke.points[0].position
        );

        edit.apply(&mut merged, false);
        assert_eq!(serde_json::to_value(merged).unwrap(), original_json);
    }

    #[test]
    fn rejected_source_leaves_destination_document_and_assets_untouched() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source_path = root.path().join("corrupt.sketch.json");
        let asset_root = root.path().join("destination-assets");
        fs::create_dir_all(&asset_root).unwrap();
        fs::write(&source_path, "{not native json").unwrap();
        let destination = Document::default();
        let original_json = serde_json::to_value(&destination).unwrap();

        let result = prepare_insert(
            &destination,
            &source_path,
            &asset_root,
            100,
            10,
            200,
            Vec3::ZERO,
            false,
            &HashMap::new(),
        );

        assert!(result.is_err());
        assert_eq!(serde_json::to_value(&destination).unwrap(), original_json);
        assert_eq!(fs::read_dir(&asset_root).unwrap().count(), 0);
    }

    #[test]
    fn missing_or_corrupt_image_asset_rejects_insert_without_mutation() {
        for corrupt in [false, true] {
            let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
            let source_path = root.path().join("source.sketch.json");
            let source_assets = crate::image_plane_storage::asset_root(&source_path).unwrap();
            fs::create_dir_all(&source_assets).unwrap();
            if corrupt {
                fs::write(source_assets.join("image-7.png"), b"not a PNG image").unwrap();
            }
            let mut source = Document::default();
            source.image_planes.push(
                ImagePlane::from_asset(
                    7,
                    "Referenced image",
                    "image-7.png".into(),
                    [1, 1],
                    Vec3::ZERO,
                    Quat::IDENTITY,
                    1.0,
                )
                .unwrap(),
            );
            fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();

            let destination = Document::default();
            let original_json = serde_json::to_value(&destination).unwrap();
            let destination_assets = root.path().join("destination-assets");
            fs::create_dir_all(&destination_assets).unwrap();
            let result = prepare_insert(
                &destination,
                &source_path,
                &destination_assets,
                100,
                10,
                200,
                Vec3::ZERO,
                false,
                &HashMap::new(),
            );

            assert!(result.is_err(), "corrupt={corrupt}");
            assert_eq!(serde_json::to_value(&destination).unwrap(), original_json);
            assert_eq!(fs::read_dir(&destination_assets).unwrap().count(), 0);
        }
    }

    #[test]
    fn insert_copies_managed_image_and_obj_assets_for_relocated_source() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source_dir = root.path().join("relocated-source");
        fs::create_dir_all(&source_dir).unwrap();
        let source_path = source_dir.join("source.sketch.json");
        let source_assets = crate::image_plane_storage::asset_root(&source_path).unwrap();
        fs::create_dir_all(&source_assets).unwrap();

        let original_image = source_dir.join("original.png");
        let image = RgbaImage::from_pixel(3, 2, Rgba([24, 128, 220, 255]));
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        fs::write(&original_image, png.into_inner()).unwrap();
        let (image_file, source_image) =
            image_planes::copy_asset(&original_image, &source_assets, 7).unwrap();
        let mut plane = ImagePlane::from_asset(
            7,
            "Source image",
            image_file,
            source_image.size,
            Vec3::new(0.5, 1.0, 0.0),
            Quat::from_rotation_y(0.25),
            2.0,
        )
        .unwrap();
        plane.drawable = true;

        let source_obj = source_dir.join("tree.obj");
        let source_mtl = source_dir.join("tree.mtl");
        fs::write(
            &source_obj,
            "mtllib tree.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl leaf\nf 1 2 3\n",
        )
        .unwrap();
        fs::write(&source_mtl, "newmtl leaf\nKd 0.1 0.8 0.2\nd 0.75\n").unwrap();
        let parsed_model = crate::obj_reference::load(&source_obj).unwrap();
        let source_model_asset =
            crate::obj_reference_storage::import_bundle(&parsed_model, &source_assets, 8).unwrap();

        let mut source = Document::default();
        source.image_planes.push(plane);
        source.active_image_plane = Some(7);
        source.model_references.push(ObjReferenceResource {
            id: 8,
            name: "Tree".into(),
            visible: true,
            locked: false,
            center: Vec3::new(-1.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: 1.0,
            asset: source_model_asset,
        });
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();

        let destination_path = root.path().join("destination.sketch.json");
        let destination_assets = crate::image_plane_storage::asset_root(&destination_path).unwrap();
        let destination = Document::default();
        let inserted = prepare_insert(
            &destination,
            &source_path,
            &destination_assets,
            100,
            10,
            200,
            Vec3::new(2.0, -1.0, 0.5),
            false,
            &HashMap::new(),
        )
        .unwrap();

        let inserted_plane = inserted.edit.image_planes.first().unwrap();
        let inserted_model = inserted.edit.model_references.first().unwrap();
        assert_eq!(inserted_plane.id, 100);
        assert_eq!(inserted_model.id, 101);
        assert_eq!(inserted_plane.origin, Vec3::new(2.5, 0.0, 0.5));
        assert_eq!(inserted_model.center, Vec3::new(1.0, -1.0, 0.5));
        assert_eq!(inserted.image_assets[&inserted_plane.id].size, [3, 2]);
        assert_eq!(
            inserted.image_assets[&inserted_plane.id].pixels,
            source_image.pixels
        );
        let staged_model = &inserted.obj_models[&inserted_model.id];
        assert_eq!(staged_model.positions, parsed_model.positions);
        assert_eq!(staged_model.triangles, parsed_model.triangles);
        assert_eq!(staged_model.materials, parsed_model.materials);

        fs::remove_dir_all(&source_dir).unwrap();
        let mut merged = destination.clone();
        let edit = crate::editing::Edit::Merge(inserted.edit.clone());
        edit.apply(&mut merged, true);
        fs::write(&destination_path, serde_json::to_vec(&merged).unwrap()).unwrap();
        let reloaded =
            Document::from_json(&fs::read_to_string(&destination_path).unwrap()).unwrap();
        let loaded_images =
            crate::image_plane_storage::load_assets(&reloaded, &destination_path).unwrap();
        assert_eq!(
            loaded_images[&inserted_plane.id].pixels,
            source_image.pixels
        );
        let loaded_model =
            crate::obj_reference_storage::load_bundle(&destination_assets, &inserted_model.asset)
                .unwrap();
        assert_eq!(loaded_model.positions, parsed_model.positions);
        assert_eq!(loaded_model.materials, parsed_model.materials);
        let export =
            crate::export::encode_with_images(&reloaded, None, Some(&destination_assets)).unwrap();
        assert_eq!(export.summary.image_planes, 1);
        assert_eq!(export.summary.model_references, 1);
        assert!(export.summary.triangles > 1);
        let gltf = gltf::Gltf::from_slice(&export.bytes).unwrap();
        assert_eq!(gltf.images().count(), 1);
        assert_eq!(gltf.meshes().count(), 2);

        let mut recovery_assets = vec![crate::recovery_store::RecoveryAsset {
            asset_file: inserted_plane.asset_file.clone(),
            bytes: fs::read(destination_assets.join(&inserted_plane.asset_file)).unwrap(),
        }];
        let model_bundle = destination_assets.join(&inserted_model.asset.bundle_dir);
        for entry in fs::read_dir(&model_bundle).unwrap() {
            let entry = entry.unwrap();
            let relative = Path::new(&inserted_model.asset.bundle_dir).join(entry.file_name());
            recovery_assets.push(crate::recovery_store::RecoveryAsset {
                asset_file: relative.to_string_lossy().replace('\\', "/"),
                bytes: fs::read(entry.path()).unwrap(),
            });
        }
        let recovery_dir = root.path().join("recovery");
        let snapshot = crate::recovery_store::RecoverySnapshot::new_with_assets(
            "insert-session".into(),
            None,
            reloaded.clone(),
            recovery_assets,
        );
        crate::recovery_store::write(&recovery_dir, &snapshot).unwrap();
        let scan = crate::recovery_store::scan(&recovery_dir, "another-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(
            scan.candidates[0].snapshot.document.image_planes,
            reloaded.image_planes
        );
        assert_eq!(
            scan.candidates[0].snapshot.document.model_references,
            reloaded.model_references
        );
        assert_eq!(scan.candidates[0].snapshot.assets.len(), 3);
        let recovered_path = root.path().join("recovered.sketch.json");
        let recovered_assets = crate::image_plane_storage::asset_root(&recovered_path).unwrap();
        crate::recovery_store::restore_assets(
            &scan.candidates[0].snapshot.document,
            &recovered_assets,
            &scan.candidates[0].snapshot.assets,
        )
        .unwrap();
        let recovered_images =
            crate::image_plane_storage::load_assets(&reloaded, &recovered_path).unwrap();
        assert_eq!(
            recovered_images[&inserted_plane.id].pixels,
            source_image.pixels
        );
        let recovered_model =
            crate::obj_reference_storage::load_bundle(&recovered_assets, &inserted_model.asset)
                .unwrap();
        assert_eq!(recovered_model.positions, parsed_model.positions);
        assert_eq!(recovered_model.materials, parsed_model.materials);

        edit.apply(&mut merged, false);
        assert!(merged.image_planes.is_empty());
        assert!(merged.model_references.is_empty());
        assert!(destination_assets
            .join(&inserted_plane.asset_file)
            .is_file());
        assert!(destination_assets
            .join(&inserted_model.asset.bundle_dir)
            .is_dir());
        edit.apply(&mut merged, true);
        assert_eq!(merged.image_planes, reloaded.image_planes);
        assert_eq!(merged.model_references, reloaded.model_references);
    }
}
