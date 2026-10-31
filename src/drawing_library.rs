//! Rebuildable index of recently saved or opened native drawings.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_RECENT: usize = 40;
const MAX_INDEX_BYTES: u64 = 64 * 1024;
const MAX_DRAWING_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ASSET_FILES: usize = 4096;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    #[serde(default)]
    recent: Vec<PathBuf>,
    #[serde(default)]
    folders: Vec<String>,
    #[serde(default)]
    assignments: HashMap<String, String>,
}

pub struct DrawingLibrary {
    path: Option<PathBuf>,
    recent: Vec<PathBuf>,
    folders: Vec<String>,
    assignments: HashMap<String, String>,
    pub notice: Option<String>,
}

impl DrawingLibrary {
    pub fn new() -> Self {
        let path = match crate::presets::path() {
            Ok(path) => path.with_file_name("drawing-library.json"),
            Err(error) => {
                return Self {
                    path: None,
                    recent: Vec::new(),
                    folders: Vec::new(),
                    assignments: HashMap::new(),
                    notice: Some(format!("Drawing library: {error}")),
                };
            }
        };
        let mut library = Self {
            path: Some(path),
            recent: Vec::new(),
            folders: Vec::new(),
            assignments: HashMap::new(),
            notice: None,
        };
        match Self::load(library.path.as_deref().unwrap()) {
            Ok(index) => {
                library.recent = index.recent;
                library.folders = index.folders;
                library.assignments = index.assignments;
            }
            Err(error) => library.notice = Some(format!("Drawing library: {error}")),
        }
        library
    }

    fn load(path: &Path) -> Result<Index, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Index::default())
            }
            Err(error) => return Err(error.to_string()),
        };
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_INDEX_BYTES {
            return Err("index exceeds its size limit".into());
        }
        let index: Index = serde_json::from_reader(file).map_err(|error| error.to_string())?;
        if index.recent.len() > MAX_RECENT
            || index.recent.iter().any(|path| path.as_os_str().is_empty())
            || index.folders.len() > 100
            || index.folders.iter().any(|folder| !valid_folder(folder))
            || index.assignments.len() > MAX_RECENT
            || index
                .assignments
                .values()
                .any(|folder| !valid_folder(folder))
        {
            return Err("index contains invalid paths or folder metadata".into());
        }
        let mut unique = Vec::new();
        for path in &index.recent {
            if !unique.contains(path) {
                unique.push(path.clone());
            }
        }
        let known_folders: HashSet<_> = index.folders.iter().collect();
        let mut unique_folders = HashSet::new();
        if index
            .folders
            .iter()
            .any(|folder| !unique_folders.insert(folder.to_lowercase()))
        {
            return Err("index contains duplicate folder names".into());
        }
        if index
            .assignments
            .values()
            .any(|folder| !known_folders.contains(folder))
        {
            return Err("index assigns a drawing to an unknown folder".into());
        }
        Ok(Index {
            recent: unique,
            ..index
        })
    }

    pub fn recent(&self) -> &[PathBuf] {
        &self.recent
    }

    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    pub fn folder_for(&self, path: &Path) -> Option<&str> {
        self.assignments.get(&path_key(path)).map(String::as_str)
    }

    pub fn create_folder(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if !valid_folder(name) {
            return Err("Folder names must be 1–40 characters and cannot contain slashes or control characters".into());
        }
        if self
            .folders
            .iter()
            .any(|folder| folder.eq_ignore_ascii_case(name))
        {
            return Err("A folder with that name already exists".into());
        }
        if self.folders.len() >= 100 {
            return Err("The library supports at most 100 folders".into());
        }
        self.folders.push(name.to_owned());
        self.save_index();
        Ok(())
    }

    pub fn assign_folder(&mut self, path: &Path, folder: Option<&str>) -> Result<(), String> {
        let key = path_key(path);
        match folder {
            Some(folder) if self.folders.iter().any(|known| known == folder) => {
                self.assignments.insert(key, folder.to_owned());
            }
            Some(_) => return Err("Choose a folder that exists in this library".into()),
            None => {
                self.assignments.remove(&key);
            }
        }
        self.save_index();
        Ok(())
    }

    pub fn rename_drawing(&mut self, source: &Path, new_stem: &str) -> Result<PathBuf, String> {
        let stem = new_stem.trim();
        if !valid_file_stem(stem) {
            return Err(
                "Use a file name of 1–96 characters without path separators or reserved characters"
                    .into(),
            );
        }
        let source_metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
        if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
            return Err("The drawing must be a regular file".into());
        }
        if source_metadata.len() > MAX_DRAWING_BYTES {
            return Err("The drawing exceeds the 64 MiB document limit".into());
        }
        let source_file = File::open(source).map_err(|error| error.to_string())?;
        let mut document_bytes = Vec::with_capacity(source_metadata.len() as usize);
        source_file
            .take(MAX_DRAWING_BYTES + 1)
            .read_to_end(&mut document_bytes)
            .map_err(|error| error.to_string())?;
        if document_bytes.len() as u64 > MAX_DRAWING_BYTES {
            return Err("The drawing exceeds the 64 MiB document limit".into());
        }
        crate::document::Document::from_json(
            std::str::from_utf8(&document_bytes).map_err(|error| error.to_string())?,
        )?;
        let extension = source
            .extension()
            .ok_or("The drawing has no file extension")?;
        let mut new_file_name = stem.to_owned();
        new_file_name.push('.');
        new_file_name.push_str(&extension.to_string_lossy());
        let destination = source.with_file_name(new_file_name);
        if source
            .file_name()
            .and_then(|name| name.to_str())
            .zip(destination.file_name().and_then(|name| name.to_str()))
            .is_some_and(|(old, new)| old.eq_ignore_ascii_case(new))
        {
            return Err("Choose a different drawing name".into());
        }
        if fs::symlink_metadata(&destination).is_ok() {
            return Err("A drawing with that name already exists".into());
        }
        let source_assets = crate::image_plane_storage::asset_root(source)?;
        let destination_assets = crate::image_plane_storage::asset_root(&destination)?;
        let old_key = path_key(source);
        let assets_exist = match fs::symlink_metadata(&source_assets) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
            Ok(_) => return Err("The drawing resource folder is not a regular directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.to_string()),
        };
        if fs::symlink_metadata(&destination_assets).is_ok() {
            return Err("A resource folder with that name already exists".into());
        }
        if assets_exist {
            fs::create_dir(&destination_assets)
                .map_err(|error| format!("Cannot create renamed resource folder: {error}"))?;
            let mut asset_bytes = 0;
            let mut asset_files = 0;
            if let Err(error) = copy_asset_tree(
                &source_assets,
                &destination_assets,
                &mut asset_bytes,
                &mut asset_files,
            ) {
                let cleanup = fs::remove_dir_all(&destination_assets);
                return Err(cleanup.err().map_or(error.clone(), |cleanup| {
                    format!("{error}; resource-folder cleanup also failed: {cleanup}")
                }));
            }
        }
        let install = (|| -> Result<(), String> {
            let parent = destination.parent().ok_or("Invalid drawing path")?;
            let mut temp =
                tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
            temp.write_all(&document_bytes)
                .map_err(|error| error.to_string())?;
            temp.as_file()
                .sync_all()
                .map_err(|error| error.to_string())?;
            temp.persist_noclobber(&destination)
                .map_err(|error| error.error.to_string())?;
            Ok(())
        })();
        if let Err(error) = install {
            if assets_exist {
                let _ = fs::remove_dir_all(&destination_assets);
            }
            return Err(format!("Cannot install renamed drawing: {error}"));
        }
        if let Err(error) = fs::remove_file(source) {
            let file_cleanup = fs::remove_file(&destination);
            let assets_cleanup = if assets_exist {
                fs::remove_dir_all(&destination_assets)
            } else {
                Ok(())
            };
            let cleanup = file_cleanup
                .err()
                .or_else(|| assets_cleanup.err())
                .map(|cleanup| format!("; rollback also failed: {cleanup}"))
                .unwrap_or_default();
            return Err(format!(
                "Cannot remove old drawing after copying: {error}{cleanup}"
            ));
        }
        let asset_cleanup_error = if assets_exist {
            fs::remove_dir_all(&source_assets).err()
        } else {
            None
        };
        let new_path = destination.canonicalize().unwrap_or(destination);
        let new_key = path_key(&new_path);
        self.recent.retain(|path| path_key(path) != old_key);
        self.recent.insert(0, new_path.clone());
        self.recent.truncate(MAX_RECENT);
        if let Some(folder) = self.assignments.remove(&old_key) {
            self.assignments.insert(new_key, folder);
        }
        self.save_index();
        if let Some(error) = asset_cleanup_error {
            self.notice = Some(format!(
                "Drawing renamed, but old resource cleanup failed: {error}"
            ));
        }
        Ok(new_path)
    }

    pub fn record(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|recent| recent != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        let known: HashSet<_> = self.recent.iter().map(|path| path_key(path)).collect();
        self.assignments.retain(|key, _| known.contains(key));
        self.save_index();
    }

    fn save_index(&mut self) {
        if let Some(index_path) = &self.path {
            let result = (|| -> Result<(), String> {
                let parent = index_path
                    .parent()
                    .ok_or("Invalid library index location")?;
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                let mut temp =
                    tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
                serde_json::to_writer_pretty(
                    &mut temp,
                    &Index {
                        recent: self.recent.clone(),
                        folders: self.folders.clone(),
                        assignments: self.assignments.clone(),
                    },
                )
                .map_err(|error| error.to_string())?;
                temp.as_file()
                    .sync_all()
                    .map_err(|error| error.to_string())?;
                temp.persist(index_path)
                    .map_err(|error| error.to_string())?;
                Ok(())
            })();
            self.notice = result
                .err()
                .map(|error| format!("Cannot save drawing library: {error}"));
        }
    }
}

fn path_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn valid_folder(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= 40
        && !name.eq_ignore_ascii_case("all folders")
        && !name.eq_ignore_ascii_case("unsorted")
        && name
            .chars()
            .all(|character| !character.is_control() && character != '/' && character != '\\')
}

fn valid_file_stem(name: &str) -> bool {
    if name.trim().is_empty()
        || name.chars().count() > 96
        || name.ends_with([' ', '.'])
        || name == "."
        || name == ".."
        || name.chars().any(|character| {
            character.is_control()
                || ['/', '\\', ':', '*', '?', '"', '<', '>', '|'].contains(&character)
        })
    {
        return false;
    }
    let device_name = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    !matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(device_name.len() == 4
            && (device_name.starts_with("COM") || device_name.starts_with("LPT"))
            && device_name.as_bytes()[3].is_ascii_digit())
}

/// Copy a validated native drawing and its complete managed resource folder.
/// Installs assets first and the JSON file last, so a failed copy never leaves
/// a discoverable drawing that points at a partial bundle.
pub fn copy_native_drawing(source: &Path, destination: &Path) -> Result<(), String> {
    let source_metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
        return Err("The source drawing must be a regular file".into());
    }
    if source_metadata.len() > MAX_DRAWING_BYTES {
        return Err("The source drawing exceeds the 64 MiB document limit".into());
    }
    let source_file = File::open(source).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(source_metadata.len() as usize);
    source_file
        .take(MAX_DRAWING_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_DRAWING_BYTES {
        return Err("The source drawing exceeds the 64 MiB document limit".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
    let document = crate::document::Document::from_json(text)?;

    if destination.exists() {
        return Err("The copy destination already exists".into());
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or("Invalid copy destination")?;
    if !parent.is_dir() {
        return Err("The copy destination folder does not exist".into());
    }
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    let destination_root = crate::image_plane_storage::asset_root(destination)?;
    let destination_normalized = parent
        .canonicalize()
        .map_err(|error| error.to_string())?
        .join(destination.file_name().ok_or("Invalid copy destination")?);
    if source == destination_normalized {
        return Err("Choose a different destination from the source drawing".into());
    }

    let source_root = crate::image_plane_storage::asset_root(&source)?;
    for plane in &document.image_planes {
        let Some(path) = plane.resolve_asset(&source_root)? else {
            return Err(format!("Image resource for '{}' is missing", plane.name));
        };
        let image = crate::image_planes::decode_asset(&path)?;
        if plane.pixel_size.is_some_and(|size| size != image.size) {
            return Err(format!(
                "Image resource for '{}' does not match its drawing metadata",
                plane.name
            ));
        }
    }
    for model in &document.model_references {
        crate::obj_reference_storage::load_bundle(&source_root, &model.asset)?;
    }
    let has_assets = match fs::symlink_metadata(&source_root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
        Ok(_) => return Err("The source resource folder is not a regular directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.to_string()),
    };
    if fs::symlink_metadata(&destination_root).is_ok() {
        return Err("The destination resource folder already exists".into());
    }
    if has_assets {
        fs::create_dir(&destination_root)
            .map_err(|error| format!("Cannot create copy resource folder: {error}"))?;
        let mut asset_bytes = 0;
        let mut asset_files = 0;
        let copy_result = copy_asset_tree(
            &source_root,
            &destination_root,
            &mut asset_bytes,
            &mut asset_files,
        );
        if let Err(error) = copy_result {
            let cleanup = fs::remove_dir_all(&destination_root);
            return Err(cleanup.err().map_or(error.clone(), |cleanup| {
                format!("{error}; resource-folder cleanup also failed: {cleanup}")
            }));
        }
    }

    let mut destination_created = false;
    let install_result = (|| -> Result<(), String> {
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| error.to_string())?;
        destination_created = true;
        target
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        target.sync_all().map_err(|error| error.to_string())?;
        Ok(())
    })();
    if let Err(error) = install_result {
        let file_cleanup = if destination_created {
            fs::remove_file(destination)
        } else {
            Ok(())
        };
        let asset_cleanup = if has_assets {
            fs::remove_dir_all(&destination_root)
        } else {
            Ok(())
        };
        let cleanup_error = file_cleanup
            .err()
            .or_else(|| asset_cleanup.err())
            .map(|cleanup| format!("; copy cleanup also failed: {cleanup}"))
            .unwrap_or_default();
        return Err(format!("{error}{cleanup_error}"));
    }
    Ok(())
}

fn copy_asset_tree(
    source: &Path,
    destination: &Path,
    bytes_total: &mut u64,
    files_total: &mut usize,
) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("Resource folders cannot contain symbolic links".into());
        }
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|error| error.to_string())?;
            copy_asset_tree(&entry.path(), &target, bytes_total, files_total)?;
        } else if metadata.is_file() {
            *files_total = files_total.saturating_add(1);
            *bytes_total = bytes_total.saturating_add(metadata.len());
            if *files_total > MAX_ASSET_FILES || *bytes_total > MAX_ASSET_BYTES {
                return Err("Resource folder exceeds the copy limits".into());
            }
            let input = File::open(entry.path()).map_err(|error| error.to_string())?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|error| error.to_string())?;
            let remaining =
                MAX_ASSET_BYTES.saturating_sub(bytes_total.saturating_sub(metadata.len()));
            let mut bounded = input.take(remaining.saturating_add(1));
            let copied =
                std::io::copy(&mut bounded, &mut output).map_err(|error| error.to_string())?;
            if copied > remaining {
                return Err("Resource folder exceeds the copy limits".into());
            }
            output.sync_all().map_err(|error| error.to_string())?;
        } else {
            return Err("Resource folder contains an unsupported filesystem entry".into());
        }
    }
    Ok(())
}
