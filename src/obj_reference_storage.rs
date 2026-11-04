//! Drawing-local persistence for imported OBJ and GLB model reference resources.
//!
//! A document stores only the generated `bundle_dir` metadata. The source model
//! and its dependencies are normalized into a per-resource folder below the
//! document's managed `.assets` directory, so reload does not depend on the
//! original files remaining in place.

use crate::obj_reference::{self, ObjModel};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const MAX_MODEL_OBJ_BYTES: u64 = obj_reference::MAX_OBJ_BYTES;
pub const MAX_MODEL_MTL_BYTES: u64 = obj_reference::MAX_MTL_BYTES;
pub const MAX_MODEL_MATERIAL_FILES: usize = 64;
pub const MAX_MODEL_BUNDLE_BYTES: u64 = 128 * 1024 * 1024;
const MODELS_DIRECTORY: &str = "models";

/// Safe, relative asset reference suitable for embedding in a drawing record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjReferenceAsset {
    pub id: u64,
    /// Generated path such as `models/42`, always relative to the drawing's
    /// managed asset root. It never contains an imported source path.
    pub bundle_dir: String,
}

impl ObjReferenceAsset {
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 || self.id == u64::MAX {
            return Err("OBJ reference ID is invalid".into());
        }
        let expected = PathBuf::from(MODELS_DIRECTORY).join(self.id.to_string());
        let actual = Path::new(&self.bundle_dir);
        if actual != expected
            || actual.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err("OBJ reference bundle path is invalid".into());
        }
        Ok(())
    }
}

/// Store a parsed/imported model and its dependencies under `asset_root`.
/// Returns generated document metadata; caller should append it to its model
/// catalog only after this function succeeds.
pub fn import_bundle(
    model: &ObjModel,
    asset_root: &Path,
    id: u64,
) -> Result<ObjReferenceAsset, String> {
    if id == 0 || id == u64::MAX {
        return Err("OBJ reference ID is invalid".into());
    }
    if model.source_format == crate::obj_reference::ObjSourceFormat::Glb {
        return import_glb_bundle(model, asset_root, id);
    }
    if model.source_materials.len() > MAX_MODEL_MATERIAL_FILES {
        return Err(format!(
            "OBJ references more than {MAX_MODEL_MATERIAL_FILES} MTL files"
        ));
    }
    let models_root = safe_models_root(asset_root, true)?;
    let bundle_name = id.to_string();
    let destination = models_root.join(&bundle_name);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err("An OBJ reference asset with this ID already exists".into());
    }

    let source_obj = regular_canonical_file(&model.source_obj, "OBJ source")?;
    let source_dir = source_obj
        .parent()
        .ok_or("OBJ source has no parent directory")?;
    let obj_metadata = fs::metadata(&source_obj).map_err(|error| error.to_string())?;
    if obj_metadata.len() > MAX_MODEL_OBJ_BYTES {
        return Err("OBJ source exceeds the 64 MiB bundle limit".into());
    }
    let original_obj = read_bounded(&source_obj, MAX_MODEL_OBJ_BYTES, "OBJ")?;
    let normalized_obj =
        normalize_obj_material_libraries(&original_obj, model.source_materials.len())?;
    if normalized_obj.len() as u64 > MAX_MODEL_OBJ_BYTES {
        return Err("Normalized OBJ source exceeds the 64 MiB bundle limit".into());
    }

    let staging = tempfile::Builder::new()
        .prefix(".obj-import-")
        .tempdir_in(&models_root)
        .map_err(|error| format!("Cannot create temporary OBJ bundle: {error}"))?;
    let mut bundle_size = normalized_obj.len() as u64;
    write_new_file(&staging.path().join("model.obj"), &normalized_obj)?;
    for (index, source) in model.source_materials.iter().enumerate() {
        let source = regular_canonical_file(source, "MTL source")?;
        if !source.starts_with(source_dir) {
            return Err("MTL source resolves outside the imported OBJ folder".into());
        }
        let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
        if metadata.len() > MAX_MODEL_MTL_BYTES {
            return Err("An MTL source exceeds the 8 MiB bundle limit".into());
        }
        bundle_size = bundle_size.saturating_add(metadata.len());
        if bundle_size > MAX_MODEL_BUNDLE_BYTES {
            return Err("OBJ and MTL sources exceed the 128 MiB per-model bundle limit".into());
        }
        let bytes = read_bounded(&source, MAX_MODEL_MTL_BYTES, "MTL")?;
        write_new_file(
            &staging.path().join(format!("material-{index:03}.mtl")),
            &bytes,
        )?;
    }

    fs::rename(staging.path(), &destination)
        .map_err(|error| format!("Cannot install OBJ asset bundle: {error}"))?;
    let metadata = ObjReferenceAsset {
        id,
        bundle_dir: format!("{MODELS_DIRECTORY}/{bundle_name}"),
    };
    metadata.validate()?;
    Ok(metadata)
}

fn import_glb_bundle(
    model: &ObjModel,
    asset_root: &Path,
    id: u64,
) -> Result<ObjReferenceAsset, String> {
    let source = regular_canonical_file(&model.source_obj, "GLB source")?;
    let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
    if metadata.len() > crate::gltf_reference::MAX_GLB_BYTES {
        return Err("GLB source exceeds the 64 MiB bundle limit".into());
    }
    let bytes = read_bounded(&source, crate::gltf_reference::MAX_GLB_BYTES, "GLB")?;
    crate::gltf_reference::parse(&bytes, &source)?;

    let models_root = safe_models_root(asset_root, true)?;
    let destination = models_root.join(id.to_string());
    if fs::symlink_metadata(&destination).is_ok() {
        return Err("A model reference asset with this ID already exists".into());
    }
    let staging = tempfile::Builder::new()
        .prefix(".glb-import-")
        .tempdir_in(&models_root)
        .map_err(|error| format!("Cannot create temporary GLB bundle: {error}"))?;
    write_new_file(&staging.path().join("model.glb"), &bytes)?;
    fs::rename(staging.path(), &destination)
        .map_err(|error| format!("Cannot install GLB asset bundle: {error}"))?;
    let asset = ObjReferenceAsset {
        id,
        bundle_dir: format!("{MODELS_DIRECTORY}/{id}"),
    };
    asset.validate()?;
    Ok(asset)
}

/// Parse a model source, install its managed bundle, then reload the installed
/// bytes before returning the model that the viewport should use. A bundle
/// that cannot be reopened is removed so failed imports do not leave an
/// orphaned resource directory.
pub fn import_source_bundle(
    source_obj: &Path,
    asset_root: &Path,
    id: u64,
) -> Result<(ObjReferenceAsset, ObjModel), String> {
    let source_model = match source_obj
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("glb") => crate::gltf_reference::load(source_obj)?,
        Some("obj") => obj_reference::load(source_obj)?,
        _ => return Err("Choose a Wavefront OBJ or binary glTF (.glb) file".into()),
    };
    let asset = import_bundle(&source_model, asset_root, id)?;
    match load_bundle(asset_root, &asset) {
        Ok(model) => {
            let extent = (model.bounds[1] - model.bounds[0]).max_element();
            if !extent.is_finite() || extent <= 1.0e-5 {
                let _ = remove_bundle(asset_root, &asset);
                return Err("OBJ geometry has no usable bounds.".into());
            }
            Ok((asset, model))
        }
        Err(error) => {
            let _ = remove_bundle(asset_root, &asset);
            Err(error)
        }
    }
}

/// Load a resource from the managed bundle. The original source files may
/// have been deleted or moved after import.
pub fn load_bundle(asset_root: &Path, asset: &ObjReferenceAsset) -> Result<ObjModel, String> {
    asset.validate()?;
    let models_root = safe_models_root(asset_root, false)?;
    let bundle = models_root.join(asset.id.to_string());
    let canonical_root = models_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve model asset folder: {error}"))?;
    let canonical_bundle = bundle
        .canonicalize()
        .map_err(|error| format!("Cannot resolve OBJ reference bundle: {error}"))?;
    if !canonical_bundle.starts_with(&canonical_root)
        || !fs::symlink_metadata(&bundle)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err(
            "OBJ reference bundle must be a regular folder inside the managed model directory"
                .into(),
        );
    }
    let glb = canonical_bundle.join("model.glb");
    let obj = canonical_bundle.join("model.obj");
    if fs::symlink_metadata(&glb).is_ok() {
        if fs::symlink_metadata(&obj).is_ok() {
            return Err("Model bundle contains ambiguous OBJ and GLB sources".into());
        }
        let metadata = fs::symlink_metadata(&glb)
            .map_err(|error| format!("Cannot inspect bundled GLB: {error}"))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > crate::gltf_reference::MAX_GLB_BYTES
        {
            return Err("Bundled GLB is missing or exceeds the 64 MiB limit".into());
        }
        let bytes = read_bounded(&glb, crate::gltf_reference::MAX_GLB_BYTES, "bundled GLB")?;
        return crate::gltf_reference::parse(&bytes, &glb);
    }
    let metadata = fs::symlink_metadata(&obj)
        .map_err(|error| format!("Cannot inspect bundled OBJ: {error}"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_MODEL_OBJ_BYTES
    {
        return Err("Bundled OBJ is missing or exceeds the 64 MiB limit".into());
    }
    let bytes = read_bounded(&obj, MAX_MODEL_OBJ_BYTES, "bundled OBJ")?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "Bundled OBJ must be UTF-8 text")?;
    let mut material_count = 0usize;
    let mut bundle_size = metadata.len();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        if fields.next() != Some("mtllib") {
            continue;
        }
        for name in fields {
            if material_count >= MAX_MODEL_MATERIAL_FILES
                || name != format!("material-{material_count:03}.mtl")
            {
                return Err("Bundled OBJ contains invalid MTL references".into());
            }
            material_count += 1;
            let material_path = canonical_bundle.join(name);
            let material_metadata = fs::symlink_metadata(&material_path)
                .map_err(|error| format!("Cannot inspect bundled MTL: {error}"))?;
            let canonical_material = material_path
                .canonicalize()
                .map_err(|error| format!("Cannot resolve bundled MTL: {error}"))?;
            if !material_metadata.is_file()
                || material_metadata.file_type().is_symlink()
                || !canonical_material.starts_with(&canonical_bundle)
                || material_metadata.len() > MAX_MODEL_MTL_BYTES
            {
                return Err("Bundled MTL is invalid or outside its resource folder".into());
            }
            bundle_size = bundle_size.saturating_add(material_metadata.len());
            if bundle_size > MAX_MODEL_BUNDLE_BYTES {
                return Err("OBJ and MTL sources exceed the 128 MiB per-model bundle limit".into());
            }
        }
    }
    obj_reference::load(&obj)
}

/// Remove only the per-resource bundle named by validated document metadata.
pub fn remove_bundle(asset_root: &Path, asset: &ObjReferenceAsset) -> Result<(), String> {
    asset.validate()?;
    let models_root = safe_models_root(asset_root, false)?;
    let bundle = models_root.join(asset.id.to_string());
    if !fs::symlink_metadata(&bundle)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err("OBJ reference bundle is missing or is not a regular folder".into());
    }
    let canonical_root = models_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let canonical_bundle = bundle.canonicalize().map_err(|error| error.to_string())?;
    if !canonical_bundle.starts_with(&canonical_root) {
        return Err("OBJ reference bundle resolves outside the managed model directory".into());
    }
    fs::remove_dir_all(canonical_bundle)
        .map_err(|error| format!("Cannot remove OBJ bundle: {error}"))
}

fn safe_models_root(asset_root: &Path, create: bool) -> Result<PathBuf, String> {
    if create {
        fs::create_dir_all(asset_root)
            .map_err(|error| format!("Cannot create drawing asset folder: {error}"))?;
    }
    if !fs::symlink_metadata(asset_root)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err("Drawing asset root must be a regular directory".into());
    }
    let models_root = asset_root.join(MODELS_DIRECTORY);
    if create {
        fs::create_dir_all(&models_root)
            .map_err(|error| format!("Cannot create model asset folder: {error}"))?;
    }
    if !fs::symlink_metadata(&models_root)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err("Model asset root must be a regular directory".into());
    }
    let root = asset_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve drawing asset folder: {error}"))?;
    let models = models_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve model asset folder: {error}"))?;
    if !models.starts_with(root) {
        return Err("Model asset folder resolves outside the drawing asset root".into());
    }
    Ok(models)
}

fn regular_canonical_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Cannot resolve {label}: {error}"))?;
    if !fs::symlink_metadata(&canonical)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        return Err(format!("{label} must be a regular file"));
    }
    Ok(canonical)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Cannot create bundled file: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("Cannot write bundled file: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Cannot sync bundled file: {error}"))
}

fn read_bounded(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|error| format!("Cannot read {label}: {error}"))?;
    let mut bytes = Vec::with_capacity(limit.min(1024 * 1024) as usize);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read {label}: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label} exceeds its size limit"));
    }
    Ok(bytes)
}

fn normalize_obj_material_libraries(
    bytes: &[u8],
    material_count: usize,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "OBJ must be UTF-8 text")?;
    let mut output = String::with_capacity(bytes.len().saturating_add(1024));
    if material_count > 0 {
        output.push_str("mtllib");
        for index in 0..material_count {
            output.push_str(&format!(" material-{index:03}.mtl"));
        }
        output.push('\n');
    }
    for line in text.lines() {
        let directive = line
            .split('#')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .next();
        if directive == Some("mtllib") {
            continue;
        }
        output.push_str(line);
        output.push('\n');
        if output.len() as u64 > MAX_MODEL_OBJ_BYTES {
            return Err("Normalized OBJ source exceeds the 64 MiB bundle limit".into());
        }
    }
    Ok(output.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_bundle_reloads_after_original_obj_and_mtl_are_removed() {
        let source = tempfile::tempdir().unwrap();
        let obj_path = source.path().join("source.obj");
        let mtl_path = source.path().join("source.mtl");
        fs::write(
            &obj_path,
            "mtllib source.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl red\nf 1 2 3\n",
        )
        .unwrap();
        fs::write(&mtl_path, "newmtl red\nKd 0.8 0.1 0.2\nd 0.7\n").unwrap();
        let asset_root = tempfile::tempdir().unwrap();
        let (asset, imported) = import_source_bundle(&obj_path, asset_root.path(), 42).unwrap();
        assert_eq!(asset.bundle_dir, "models/42");
        assert_eq!(imported.triangles.len(), 1);
        assert_eq!(imported.materials[0].name, "red");

        fs::remove_file(&obj_path).unwrap();
        fs::remove_file(&mtl_path).unwrap();
        let reopened = load_bundle(asset_root.path(), &asset).unwrap();
        assert_eq!(reopened.positions, imported.positions);
        assert_eq!(reopened.triangles, imported.triangles);
        assert_eq!(reopened.materials, imported.materials);
        assert_eq!(reopened.source_materials.len(), 1);

        remove_bundle(asset_root.path(), &asset).unwrap();
        assert!(load_bundle(asset_root.path(), &asset).is_err());
    }

    #[test]
    fn rejected_source_does_not_create_a_model_bundle() {
        let source = tempfile::tempdir().unwrap();
        let obj_path = source.path().join("invalid.obj");
        fs::write(&obj_path, "v NaN 0 0\n").unwrap();
        let asset_root = tempfile::tempdir().unwrap();

        assert!(import_source_bundle(&obj_path, asset_root.path(), 43).is_err());
        assert!(!asset_root.path().join("models").exists());
    }

    #[test]
    fn unusable_geometry_does_not_leave_an_imported_bundle() {
        let source = tempfile::tempdir().unwrap();
        let obj_path = source.path().join("point.obj");
        fs::write(&obj_path, "v 0 0 0\nv 0 0 0\nv 0 0 0\nf 1 2 3\n").unwrap();
        let asset_root = tempfile::tempdir().unwrap();

        assert!(import_source_bundle(&obj_path, asset_root.path(), 44).is_err());
        assert!(!asset_root.path().join("models/44").exists());
    }
}
