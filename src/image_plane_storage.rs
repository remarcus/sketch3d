//! Drawing-local storage for image-plane source files.
use crate::{
    document::Document,
    image_planes::{self, ImageAsset},
};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn asset_root(path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or("Invalid drawing path")?;
    let mut folder = name.to_os_string();
    folder.push(".assets");
    Ok(path.with_file_name(folder))
}

pub fn load_assets(
    document: &Document,
    drawing: &Path,
) -> Result<HashMap<u64, ImageAsset>, String> {
    let root = asset_root(drawing)?;
    let mut loaded = HashMap::new();
    let mut pixels = 0_u64;
    let mut bytes_total = 0_u64;
    for plane in &document.image_planes {
        let Ok(Some(path)) = plane.resolve_asset(&root) else {
            continue;
        };
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        bytes_total = bytes_total.saturating_add(metadata.len());
        if bytes_total > image_planes::MAX_TOTAL_ASSET_BYTES {
            return Err("Image-plane assets exceed the 256 MiB managed-folder limit".into());
        }
        let Ok(asset) = image_planes::decode_asset(&path) else {
            continue;
        };
        if plane.pixel_size != Some(asset.size) {
            continue;
        }
        pixels = pixels.saturating_add(u64::from(asset.size[0]) * u64::from(asset.size[1]));
        if pixels > image_planes::MAX_CATALOG_PIXELS {
            return Err("Image-plane assets exceed the document pixel budget".into());
        }
        loaded.insert(plane.id, asset);
    }
    Ok(loaded)
}

/// Copy all available plane assets from the current managed folder into the
/// destination bundle. Missing assets remain missing so documents open with a
/// usable placeholder instead of losing their drawing metadata.
pub fn copy_bundle(document: &Document, source: &Path, destination: &Path) -> Result<(), String> {
    if document.image_planes.is_empty() {
        return Ok(());
    }
    if same_directory(source, destination) {
        return Ok(());
    }
    if fs::symlink_metadata(destination).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("The drawing image folder must not be a symbolic link".into());
    }
    fs::create_dir_all(destination)
        .map_err(|e| format!("Cannot create image asset folder: {e}"))?;
    if !fs::symlink_metadata(destination)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err("The drawing image folder is not a regular directory".into());
    }
    let mut bytes_total = 0_u64;
    for plane in &document.image_planes {
        let Some(source_file) = plane.resolve_asset(source)? else {
            continue;
        };
        let metadata = fs::metadata(&source_file).map_err(|e| e.to_string())?;
        bytes_total = bytes_total.saturating_add(metadata.len());
        if metadata.len() > image_planes::MAX_IMAGE_BYTES
            || bytes_total > image_planes::MAX_TOTAL_ASSET_BYTES
        {
            return Err("Image-plane assets exceed the managed storage limit".into());
        }
        let data = fs::read(source_file).map_err(|e| e.to_string())?;
        let target = destination.join(&plane.asset_file);
        if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("An image asset target must not be a symbolic link".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(destination).map_err(|e| e.to_string())?;
        temp.write_all(&data).map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist(&target).map_err(|e| e.error.to_string())?;
    }
    Ok(())
}

fn same_directory(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
