//! Rebuildable index of recently saved or opened native drawings.
use eframe::egui;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
};

const MAX_RECENT: usize = 40;
const MAX_INDEX_BYTES: u64 = 64 * 1024;
const MAX_DRAWING_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ASSET_FILES: usize = 4096;
const THUMBNAIL_SIZE: [u32; 2] = [192, 120];
const MAX_THUMBNAIL_BYTES: u64 = 128 * 1024;
const MAX_TRASH_ENTRIES: usize = 1000;
const MAX_THUMBNAIL_WORKERS: usize = 2;
const MAX_THUMBNAIL_SEGMENTS: usize = 20_000;
const MAX_CACHED_THUMBNAILS: usize = 128;

struct ThumbnailResult {
    path: PathBuf,
    generation: u64,
    rgba: Result<Vec<u8>, String>,
}

pub struct TrashDrawing {
    pub path: PathBuf,
    pub name: String,
    pub deleted: Option<std::time::SystemTime>,
    pub issue: Option<String>,
}

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
    thumbnail_dir: Option<PathBuf>,
    thumbnail_tx: Sender<ThumbnailResult>,
    thumbnail_rx: Receiver<ThumbnailResult>,
    thumbnail_pending: HashSet<PathBuf>,
    thumbnail_failed: HashSet<PathBuf>,
    thumbnail_textures: HashMap<PathBuf, egui::TextureHandle>,
    thumbnail_generations: HashMap<PathBuf, u64>,
    pub trash_open: bool,
    pub notice: Option<String>,
}

impl DrawingLibrary {
    pub fn new() -> Self {
        let (thumbnail_tx, thumbnail_rx) = mpsc::channel();
        let path = match crate::presets::path() {
            Ok(path) => path.with_file_name("drawing-library.json"),
            Err(error) => {
                return Self {
                    path: None,
                    recent: Vec::new(),
                    folders: Vec::new(),
                    assignments: HashMap::new(),
                    thumbnail_dir: None,
                    thumbnail_tx,
                    thumbnail_rx,
                    thumbnail_pending: HashSet::new(),
                    thumbnail_failed: HashSet::new(),
                    thumbnail_textures: HashMap::new(),
                    thumbnail_generations: HashMap::new(),
                    trash_open: false,
                    notice: Some(format!("Drawing library: {error}")),
                };
            }
        };
        let mut library = Self {
            path: Some(path.clone()),
            recent: Vec::new(),
            folders: Vec::new(),
            assignments: HashMap::new(),
            thumbnail_dir: Some(path.with_file_name("drawing-thumbnails")),
            thumbnail_tx,
            thumbnail_rx,
            thumbnail_pending: HashSet::new(),
            thumbnail_failed: HashSet::new(),
            thumbnail_textures: HashMap::new(),
            thumbnail_generations: HashMap::new(),
            trash_open: false,
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

    /// Return a cached thumbnail or queue its bounded generation on a worker.
    /// Parsing and rasterization never run in the canvas/UI frame.
    pub fn thumbnail(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        self.poll_thumbnails(ctx);
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !self.thumbnail_textures.contains_key(&key)
            && !self.thumbnail_pending.contains(&key)
            && !self.thumbnail_failed.contains(&key)
            && self.thumbnail_pending.len() < MAX_THUMBNAIL_WORKERS
        {
            self.thumbnail_pending.insert(key.clone());
            let sender = self.thumbnail_tx.clone();
            let repaint = ctx.clone();
            let cache_dir = self.thumbnail_dir.clone();
            let worker_path = key.clone();
            let generation = *self.thumbnail_generations.entry(key.clone()).or_default();
            if let Err(error) = std::thread::Builder::new()
                .name("drawing-thumbnail".into())
                .spawn(move || {
                    let rgba = generate_thumbnail(&worker_path, cache_dir.as_deref());
                    let _ = sender.send(ThumbnailResult {
                        path: worker_path,
                        generation,
                        rgba,
                    });
                    repaint.request_repaint();
                })
            {
                self.thumbnail_pending.remove(&key);
                self.thumbnail_failed.insert(key.clone());
                self.notice = Some(format!("Cannot start thumbnail worker: {error}"));
            }
        }
        self.thumbnail_textures.get(&key).cloned()
    }

    fn poll_thumbnails(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.thumbnail_rx.try_recv() {
            self.thumbnail_pending.remove(&result.path);
            if self
                .thumbnail_generations
                .get(&result.path)
                .copied()
                .unwrap_or_default()
                != result.generation
            {
                continue;
            }
            match result.rgba {
                Ok(rgba) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        THUMBNAIL_SIZE.map(|dimension| dimension as usize),
                        &rgba,
                    );
                    let texture = ctx.load_texture(
                        format!("drawing-thumbnail-{}", stable_path_hash(&result.path)),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.thumbnail_textures.insert(result.path, texture);
                }
                Err(_) => {
                    self.thumbnail_failed.insert(result.path);
                }
            }
        }
    }

    /// Clear stale preview state after a successful save/import/rename so the
    /// next library view renders the new document contents.
    pub fn invalidate_thumbnail(&mut self, path: &Path) {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.thumbnail_textures.remove(&key);
        self.thumbnail_failed.remove(&key);
        *self.thumbnail_generations.entry(key.clone()).or_default() += 1;
        if let Some(directory) = &self.thumbnail_dir {
            remove_thumbnail_files(directory, stable_path_hash(&key), None);
        }
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
        self.invalidate_thumbnail(source);
        self.invalidate_thumbnail(&new_path);
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
        self.invalidate_thumbnail(&path);
        let previous_recent = self.recent.clone();
        self.recent.retain(|recent| recent != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        for old_path in previous_recent {
            if !self.recent.contains(&old_path) {
                self.invalidate_thumbnail(&old_path);
            }
        }
        let known: HashSet<_> = self.recent.iter().map(|path| path_key(path)).collect();
        self.assignments.retain(|key, _| known.contains(key));
        self.save_index();
    }

    pub fn forget(&mut self, path: &Path) {
        let key = path_key(path);
        self.recent.retain(|recent| path_key(recent) != key);
        self.assignments.remove(&key);
        self.invalidate_thumbnail(path);
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
    let source_metadata = fs::symlink_metadata(source).map_err(|error| {
        format!(
            "Cannot inspect source drawing {}: {error}",
            source.display()
        )
    })?;
    if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
        return Err("The source drawing must be a regular file".into());
    }
    if source_metadata.len() > MAX_DRAWING_BYTES {
        return Err("The source drawing exceeds the 64 MiB document limit".into());
    }
    let source_file = File::open(source)
        .map_err(|error| format!("Cannot open source drawing {}: {error}", source.display()))?;
    let mut bytes = Vec::with_capacity(source_metadata.len() as usize);
    source_file
        .take(MAX_DRAWING_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read source drawing {}: {error}", source.display()))?;
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
    let source = source.canonicalize().map_err(|error| {
        format!(
            "Cannot canonicalize source drawing {}: {error}",
            source.display()
        )
    })?;
    let destination_root = crate::image_plane_storage::asset_root(destination)
        .map_err(|error| format!("Cannot resolve copy asset destination: {error}"))?;
    let destination_normalized = parent
        .canonicalize()
        .map_err(|error| {
            format!(
                "Cannot canonicalize copy destination folder {}: {error}",
                parent.display()
            )
        })?
        .join(destination.file_name().ok_or("Invalid copy destination")?);
    if source == destination_normalized {
        return Err("Choose a different destination from the source drawing".into());
    }

    let source_root = crate::image_plane_storage::asset_root(&source)
        .map_err(|error| format!("Cannot resolve source asset folder: {error}"))?;
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
        Err(error) => {
            return Err(format!(
                "Cannot inspect source asset folder {}: {error}",
                source_root.display()
            ))
        }
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

/// Create a self-contained, portable directory backup. The final directory is
/// installed with one rename after the drawing and its complete asset bundle
/// have been validated and copied into a staging directory.
pub fn backup_native_drawing(source: &Path, destination: &Path) -> Result<(), String> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err("The backup destination already exists".into());
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or("Invalid backup destination")?;
    if !parent.is_dir() {
        return Err("The backup parent folder does not exist".into());
    }
    let stage = tempfile::Builder::new()
        .prefix(".sketch-backup-")
        .tempdir_in(parent)
        .map_err(|error| format!("Cannot create backup staging folder: {error}"))?;
    let drawing_path = stage.path().join("drawing.sketch.json");
    copy_native_drawing(source, &drawing_path)
        .map_err(|error| format!("Copying drawing into backup staging failed: {error}"))?;
    let original_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("drawing.sketch.json");
    let manifest = serde_json::json!({"format":"spatial-sketch-backup", "version":1, "original_file":original_name});
    let mut manifest_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.path().join("manifest.json"))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer_pretty(&mut manifest_file, &manifest)
        .map_err(|error| error.to_string())?;
    manifest_file
        .sync_all()
        .map_err(|error| error.to_string())?;
    drop(manifest_file);
    let stage_path = stage.keep();
    if let Err(error) = fs::rename(&stage_path, destination) {
        let cleanup = fs::remove_dir_all(&stage_path)
            .err()
            .map(|error| format!("; staging cleanup failed: {error}"))
            .unwrap_or_default();
        return Err(format!(
            "Cannot install backup from {} to {}: {error}{cleanup}",
            stage_path.display(),
            destination.display()
        ));
    }
    Ok(())
}

/// Restore a portable backup into a new native drawing. Validation and asset
/// checks are shared with the regular copy path; destination creation is
/// no-clobber and rolls back its asset folder if installation fails.
pub fn restore_native_backup(bundle: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(bundle).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Choose a regular backup folder".into());
    }
    let manifest_path = bundle.join("manifest.json");
    let manifest_metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|_| "Backup manifest is missing or unreadable")?;
    if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
        return Err("Backup manifest must be a regular file".into());
    }
    let manifest_file =
        File::open(&manifest_path).map_err(|_| "Backup manifest is missing or unreadable")?;
    if manifest_file
        .metadata()
        .map_err(|error| error.to_string())?
        .len()
        > 4096
    {
        return Err("Backup manifest exceeds its size limit".into());
    }
    let manifest: serde_json::Value = serde_json::from_reader(manifest_file)
        .map_err(|error| format!("Invalid backup manifest: {error}"))?;
    if manifest.get("format").and_then(serde_json::Value::as_str) != Some("spatial-sketch-backup")
        || manifest.get("version").and_then(serde_json::Value::as_u64) != Some(1)
    {
        return Err("Unsupported backup format".into());
    }
    copy_native_drawing(&bundle.join("drawing.sketch.json"), destination)
}

/// Move a saved drawing into the local recoverable trash after creating and
/// validating a self-contained backup. The original remains untouched if the
/// backup cannot be completed.
pub fn trash_native_drawing(source: &Path) -> Result<PathBuf, String> {
    let trash_root = crate::presets::path()
        .map_err(|error| error.to_string())?
        .join("drawing-trash");
    trash_native_drawing_in(source, &trash_root)
}

fn trash_native_drawing_in(source: &Path, trash_root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(trash_root)
        .map_err(|error| format!("Cannot create drawing trash: {error}"))?;
    let stem = source
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("drawing");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let mut destination = trash_root.join(format!("{stem}-{timestamp}.sketch-backup"));
    let mut suffix = 1;
    while fs::symlink_metadata(&destination).is_ok() {
        destination = trash_root.join(format!("{stem}-{timestamp}-{suffix}.sketch-backup"));
        suffix += 1;
    }
    backup_native_drawing(source, &destination)?;
    let assets = crate::image_plane_storage::asset_root(source)?;
    if let Err(error) = fs::remove_file(source) {
        return Err(format!(
            "Drawing was backed up to {} but could not be removed: {error}",
            destination.display()
        ));
    }
    match fs::symlink_metadata(&assets) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            if let Err(error) = fs::remove_dir_all(&assets) {
                return Err(format!("Drawing was moved to trash, but its original resource folder could not be removed: {error}"));
            }
        }
        Ok(_) => {
            return Err(
                "Drawing moved to trash, but its resource folder is not a regular directory".into(),
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Drawing moved to trash, but its resource folder could not be inspected: {error}"
            ))
        }
    }
    Ok(destination)
}

fn trash_root() -> Result<PathBuf, String> {
    Ok(crate::presets::path()
        .map_err(|error| error.to_string())?
        .join("drawing-trash"))
}

/// List recoverable backups, retaining unreadable entries in the view with an
/// issue label so corruption is visible instead of silently hiding data.
pub fn list_trash() -> Result<Vec<TrashDrawing>, String> {
    let root = trash_root()?;
    list_trash_in(&root)
}

fn list_trash_in(root: &Path) -> Result<Vec<TrashDrawing>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Cannot read drawing trash: {error}")),
    };
    let mut results = Vec::new();
    for entry in entries.take(MAX_TRASH_ENTRIES) {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        // A terminated backup may leave its private staging directory behind.
        // It is not an installed trash item and should not appear as a corrupt
        // user backup in the trash browser.
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".sketch-backup-"))
        {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => metadata,
            _ => continue,
        };
        let deleted = metadata.modified().ok();
        match read_backup_name(&path) {
            Ok(name) => results.push(TrashDrawing {
                path,
                name,
                deleted,
                issue: None,
            }),
            Err(error) => results.push(TrashDrawing {
                path,
                name: "Unreadable backup".into(),
                deleted,
                issue: Some(error),
            }),
        }
    }
    results.sort_by_key(|result| std::cmp::Reverse(result.deleted));
    Ok(results)
}

fn read_backup_name(bundle: &Path) -> Result<String, String> {
    let manifest_path = bundle.join("manifest.json");
    let metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|_| "Backup manifest is missing".to_owned())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return Err("Backup manifest is invalid or exceeds its 4 KiB limit".into());
    }
    let manifest: serde_json::Value =
        serde_json::from_reader(File::open(manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("Backup manifest is corrupt: {error}"))?;
    if manifest.get("format").and_then(serde_json::Value::as_str) != Some("spatial-sketch-backup")
        || manifest.get("version").and_then(serde_json::Value::as_u64) != Some(1)
    {
        return Err("Backup format is unsupported".into());
    }
    let name = manifest
        .get("original_file")
        .and_then(serde_json::Value::as_str)
        .ok_or("Backup manifest has no source file name")?;
    if name.is_empty()
        || name.len() > 255
        || Path::new(name).file_name().and_then(|part| part.to_str()) != Some(name)
    {
        return Err("Backup source file name is invalid".into());
    }
    Ok(name.to_owned())
}

/// Restore a trash item without overwriting existing work, then remove the
/// backup only after the restored drawing and managed resources are installed.
pub fn restore_from_trash(bundle: &Path, destination: &Path) -> Result<(), String> {
    let root = trash_root()?;
    restore_from_trash_in(bundle, destination, &root)
}

fn restore_from_trash_in(bundle: &Path, destination: &Path, root: &Path) -> Result<(), String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Cannot locate drawing trash: {error}"))?;
    let canonical_bundle = bundle
        .canonicalize()
        .map_err(|error| format!("Cannot locate trash item: {error}"))?;
    if !canonical_bundle.starts_with(&canonical_root) {
        return Err("Selected backup is outside the recoverable drawing trash".into());
    }
    restore_native_backup(&canonical_bundle, destination)?;
    if let Err(error) = fs::remove_dir_all(&canonical_bundle) {
        return Err(format!(
            "Drawing restored, but its trash copy could not be removed: {error}"
        ));
    }
    Ok(())
}

fn copy_asset_tree(
    source: &Path,
    destination: &Path,
    bytes_total: &mut u64,
    files_total: &mut usize,
) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|error| format!("Cannot read asset folder {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "Cannot inspect copied asset {}: {error}",
                entry.path().display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err("Resource folders cannot contain symbolic links".into());
        }
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|error| {
                format!("Cannot create asset folder {}: {error}", target.display())
            })?;
            copy_asset_tree(&entry.path(), &target, bytes_total, files_total)?;
        } else if metadata.is_file() {
            *files_total = files_total.saturating_add(1);
            *bytes_total = bytes_total.saturating_add(metadata.len());
            if *files_total > MAX_ASSET_FILES || *bytes_total > MAX_ASSET_BYTES {
                return Err("Resource folder exceeds the copy limits".into());
            }
            let input = File::open(entry.path()).map_err(|error| {
                format!(
                    "Cannot open source asset {}: {error}",
                    entry.path().display()
                )
            })?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|error| {
                    format!("Cannot create copied asset {}: {error}", target.display())
                })?;
            let remaining =
                MAX_ASSET_BYTES.saturating_sub(bytes_total.saturating_sub(metadata.len()));
            let mut bounded = input.take(remaining.saturating_add(1));
            #[cfg(debug_assertions)]
            let copied = if let Some(marker) = std::env::var_os("SKETCH3D_VERIFY_COPY_PAUSE_MARKER")
                .filter(|marker| !marker.is_empty())
            {
                let mut writer = PauseAfterFirstCopyWrite {
                    output: &mut output,
                    marker: marker.into(),
                    paused: false,
                };
                std::io::copy(&mut bounded, &mut writer).map_err(|error| error.to_string())?
            } else {
                std::io::copy(&mut bounded, &mut output).map_err(|error| error.to_string())?
            };
            #[cfg(not(debug_assertions))]
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

#[cfg(debug_assertions)]
struct PauseAfterFirstCopyWrite<'a> {
    output: &'a mut File,
    marker: PathBuf,
    paused: bool,
}

#[cfg(debug_assertions)]
impl std::io::Write for PauseAfterFirstCopyWrite<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let count = if self.paused {
            self.output.write(buffer)?
        } else {
            self.output.write(&buffer[..buffer.len().min(8)])?
        };
        if count > 0 && !self.paused {
            fs::write(&self.marker, b"partial asset bytes written")?;
            self.paused = true;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

fn generate_thumbnail(path: &Path, cache_dir: Option<&Path>) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Drawing is missing or is not a regular file".into());
    }
    if metadata.len() > MAX_DRAWING_BYTES {
        return Err("Drawing exceeds the thumbnail size limit".into());
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos() as u64);
    let mut fingerprint = stable_path_hash(path) ^ metadata.len().rotate_left(17) ^ modified;
    fingerprint ^= fingerprint >> 30;
    fingerprint = fingerprint.wrapping_mul(0xbf58476d1ce4e5b9);
    fingerprint ^= fingerprint >> 27;
    let cache_path = cache_dir.map(|directory| {
        directory.join(format!(
            "{:016x}-{:016x}.png",
            stable_path_hash(path),
            fingerprint
        ))
    });
    if let Some(cache_path) = &cache_path {
        if let Ok(metadata) = fs::symlink_metadata(cache_path) {
            if metadata.is_file() && !metadata.file_type().is_symlink() {
                if metadata.len() <= MAX_THUMBNAIL_BYTES {
                    if let Ok(bytes) = fs::read(cache_path) {
                        if valid_thumbnail_png(&bytes) {
                            if let Ok(rgba) = png_to_rgba(&bytes) {
                                if let Some(directory) = cache_dir {
                                    prune_thumbnail_cache(directory);
                                }
                                return Ok(rgba);
                            }
                        }
                    }
                }
                // A malformed cache entry must not block its own replacement.
                let _ = fs::remove_file(cache_path);
            }
        }
    }

    let mut text = String::with_capacity(metadata.len() as usize);
    File::open(path)
        .map_err(|error| error.to_string())?
        .take(MAX_DRAWING_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("Cannot read drawing for preview: {error}"))?;
    if text.len() as u64 > MAX_DRAWING_BYTES {
        return Err("Drawing grew beyond the 64 MiB thumbnail limit".into());
    }
    let document = crate::document::Document::from_json(&text)
        .map_err(|error| format!("Cannot preview invalid drawing: {error}"))?;
    let image = raster_thumbnail(&document);
    let rgba = image.as_raw().clone();
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| format!("Cannot encode drawing preview: {error}"))?;
    let bytes = encoded.into_inner();
    if bytes.len() as u64 > MAX_THUMBNAIL_BYTES {
        return Err("Generated preview exceeded its cache limit".into());
    }
    if let (Some(cache_dir), Some(cache_path)) = (cache_dir, cache_path.as_deref()) {
        if fs::create_dir_all(cache_dir).is_ok() {
            if let Ok(mut temporary) = tempfile::NamedTempFile::new_in(cache_dir) {
                if temporary.write_all(&bytes).is_ok() && temporary.as_file().sync_all().is_ok() {
                    let _ = temporary.persist_noclobber(cache_path);
                }
            }
            remove_thumbnail_files(
                cache_dir,
                stable_path_hash(path),
                cache_path.file_name().and_then(|name| name.to_str()),
            );
            prune_thumbnail_cache(cache_dir);
        }
    }
    Ok(rgba)
}

fn valid_thumbnail_png(bytes: &[u8]) -> bool {
    let Ok(reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format() else {
        return false;
    };
    reader
        .into_dimensions()
        .is_ok_and(|(width, height)| width == THUMBNAIL_SIZE[0] && height == THUMBNAIL_SIZE[1])
}

fn png_to_rgba(bytes: &[u8]) -> Result<Vec<u8>, String> {
    image::load_from_memory(bytes)
        .map(|image| image.to_rgba8().into_raw())
        .map_err(|error| format!("Cached drawing preview is corrupt: {error}"))
}

fn raster_thumbnail(document: &crate::document::Document) -> image::RgbaImage {
    let mut image = image::RgbaImage::from_pixel(
        THUMBNAIL_SIZE[0],
        THUMBNAIL_SIZE[1],
        image::Rgba([21, 31, 39, 255]),
    );
    let strokes: Vec<_> = document
        .strokes
        .iter()
        .filter(|stroke| document.visible(stroke) && !stroke.points.is_empty())
        .collect();
    let Some(first) = strokes.iter().flat_map(|stroke| &stroke.points).next() else {
        return image;
    };
    let (mut min, mut max) = (first.position, first.position);
    for point in strokes.iter().flat_map(|stroke| &stroke.points) {
        min = min.min(point.position);
        max = max.max(point.position);
    }
    let spans = max - min;
    let pairs = [(0_usize, 1_usize), (0, 2), (1, 2)];
    let (axis_x, axis_y) = pairs
        .into_iter()
        .max_by(|(ax, ay), (bx, by)| {
            (spans[*ax] * spans[*ay]).total_cmp(&(spans[*bx] * spans[*by]))
        })
        .unwrap_or((0, 1));
    let span_x = spans[axis_x].max(0.001);
    let span_y = spans[axis_y].max(0.001);
    let scale = ((THUMBNAIL_SIZE[0] as f32 - 20.0) / span_x)
        .min((THUMBNAIL_SIZE[1] as f32 - 20.0) / span_y);
    let center_x = (min[axis_x] + max[axis_x]) * 0.5;
    let center_y = (min[axis_y] + max[axis_y]) * 0.5;
    let project = |position: Vec3| -> (f32, f32) {
        (
            THUMBNAIL_SIZE[0] as f32 * 0.5 + (position[axis_x] - center_x) * scale,
            THUMBNAIL_SIZE[1] as f32 * 0.5 - (position[axis_y] - center_y) * scale,
        )
    };
    let mut segment_count = 0;
    for stroke in strokes {
        let color = image::Rgba([
            linear_to_byte(stroke.color[0]),
            linear_to_byte(stroke.color[1]),
            linear_to_byte(stroke.color[2]),
            255,
        ]);
        let thickness = (stroke.radius * scale * 2.0).round().clamp(1.0, 5.0) as i32;
        for segment in stroke.points.windows(2) {
            if segment_count >= MAX_THUMBNAIL_SEGMENTS {
                return image;
            }
            segment_count += 1;
            let (x0, y0) = project(segment[0].position);
            let (x1, y1) = project(segment[1].position);
            draw_preview_line(&mut image, x0, y0, x1, y1, color, thickness);
        }
        if stroke.points.len() == 1 {
            let (x, y) = project(stroke.points[0].position);
            draw_preview_line(&mut image, x, y, x, y, color, thickness);
        }
    }
    image
}

fn draw_preview_line(
    image: &mut image::RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    color: image::Rgba<u8>,
    thickness: i32,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
    let radius = (thickness / 2).max(0);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (x0 + dx * t).round() as i32;
        let y = (y0 + dy * t).round() as i32;
        for oy in -radius..=radius {
            for ox in -radius..=radius {
                let px = x + ox;
                let py = y + oy;
                if px >= 0 && py >= 0 && (px as u32) < image.width() && (py as u32) < image.height()
                {
                    image.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}

fn linear_to_byte(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round() as u8
}

fn stable_path_hash(path: &Path) -> u64 {
    path.to_string_lossy()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn remove_thumbnail_files(directory: &Path, path_hash: u64, keep: Option<&str>) {
    let prefix = format!("{path_hash:016x}-");
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(&prefix)
            && keep.is_none_or(|keep| name.to_string_lossy() != keep)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn prune_thumbnail_cache(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let text = name.to_str()?;
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            (metadata.is_file()
                && !metadata.file_type().is_symlink()
                && text.ends_with(".png")
                && text.len() == 37)
                .then(|| (metadata.modified().ok(), entry.path()))
        })
        .collect();
    if files.len() <= MAX_CACHED_THUMBNAILS {
        return;
    }
    files.sort_by_key(|entry| entry.0);
    let remove_count = files.len() - MAX_CACHED_THUMBNAILS;
    for (_, path) in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawing_library_for_test(thumbnail_dir: PathBuf) -> DrawingLibrary {
        let (thumbnail_tx, thumbnail_rx) = mpsc::channel();
        DrawingLibrary {
            path: None,
            recent: Vec::new(),
            folders: Vec::new(),
            assignments: HashMap::new(),
            thumbnail_dir: Some(thumbnail_dir),
            thumbnail_tx,
            thumbnail_rx,
            thumbnail_pending: HashSet::new(),
            thumbnail_failed: HashSet::new(),
            thumbnail_textures: HashMap::new(),
            thumbnail_generations: HashMap::new(),
            trash_open: false,
            notice: None,
        }
    }

    #[test]
    fn invalidating_a_running_thumbnail_keeps_its_worker_slot_until_result_arrives() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let drawing = root.path().join("running.sketch.json");
        fs::write(
            &drawing,
            serde_json::to_vec(&crate::document::Document::default()).unwrap(),
        )
        .unwrap();
        let key = drawing.canonicalize().unwrap();
        let mut library = drawing_library_for_test(root.path().join("thumbnails"));
        library.thumbnail_pending.insert(key.clone());
        library.thumbnail_generations.insert(key.clone(), 0);

        library.invalidate_thumbnail(&drawing);
        assert!(library.thumbnail_pending.contains(&key));
        library
            .thumbnail_tx
            .send(ThumbnailResult {
                path: key.clone(),
                generation: 0,
                rgba: Ok(vec![
                    0;
                    THUMBNAIL_SIZE[0] as usize
                        * THUMBNAIL_SIZE[1] as usize
                        * 4
                ]),
            })
            .unwrap();
        library.poll_thumbnails(&egui::Context::default());

        assert!(!library.thumbnail_pending.contains(&key));
        assert!(!library.thumbnail_textures.contains_key(&key));
        assert_eq!(library.thumbnail_generations.get(&key), Some(&1));
    }

    #[test]
    fn drawing_thumbnail_is_bounded_and_cached_as_a_fixed_size_png() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let drawing = root.path().join("thumbnail.sketch.json");
        let cache = root.path().join("thumbnails");
        fs::write(
            &drawing,
            serde_json::to_vec(&crate::document::Document::default()).unwrap(),
        )
        .unwrap();

        let first = generate_thumbnail(&drawing, Some(&cache)).unwrap();
        assert_eq!(
            first.len(),
            THUMBNAIL_SIZE[0] as usize * THUMBNAIL_SIZE[1] as usize * 4
        );
        let cached: Vec<_> = fs::read_dir(&cache).unwrap().map(Result::unwrap).collect();
        assert_eq!(cached.len(), 1);
        let bytes = fs::read(cached[0].path()).unwrap();
        assert!(bytes.len() as u64 <= MAX_THUMBNAIL_BYTES);
        assert!(valid_thumbnail_png(&bytes));
        let second = generate_thumbnail(&drawing, Some(&cache)).unwrap();
        assert_eq!(second, first);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);

        let cached_path = fs::read_dir(&cache)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        fs::write(&cached_path, b"corrupt thumbnail").unwrap();
        let recovered = generate_thumbnail(&drawing, Some(&cache)).unwrap();
        assert_eq!(recovered, first);
        assert!(valid_thumbnail_png(&fs::read(&cached_path).unwrap()));
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
    }

    #[test]
    fn successful_library_record_invalidates_and_regenerates_the_preview() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let drawing = root.path().join("saved.sketch.json");
        let cache = root.path().join("thumbnails");
        fs::write(
            &drawing,
            serde_json::to_vec(&crate::document::Document::default()).unwrap(),
        )
        .unwrap();
        let key = drawing.canonicalize().unwrap();
        generate_thumbnail(&key, Some(&cache)).unwrap();
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);

        let mut library = drawing_library_for_test(cache.clone());
        library.thumbnail_failed.insert(key.clone());
        library.record(&drawing);
        assert!(library.recent().contains(&key));
        assert!(!library.thumbnail_failed.contains(&key));
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);

        generate_thumbnail(&key, Some(&cache)).unwrap();
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
        assert!(library.thumbnail_generations.get(&key).unwrap() > &0);
    }

    #[test]
    fn missing_or_corrupt_drawings_never_write_thumbnail_cache_entries() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let cache = root.path().join("thumbnails");
        let missing = root.path().join("missing.sketch.json");
        assert!(generate_thumbnail(&missing, Some(&cache)).is_err());

        let corrupt = root.path().join("corrupt.sketch.json");
        fs::write(&corrupt, b"not a native drawing").unwrap();
        assert!(generate_thumbnail(&corrupt, Some(&cache)).is_err());
        assert!(!cache.exists());
    }

    #[test]
    fn portable_backup_restore_roundtrips_native_file_without_overwrite() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source = root.path().join("source.sketch.json");
        let backup = root.path().join("source.sketch-backup");
        let restored = root.path().join("restored.sketch.json");
        let original = crate::document::Document::default();
        fs::write(&source, serde_json::to_vec(&original).unwrap()).unwrap();
        fs::create_dir(&backup).unwrap();
        fs::copy(&source, backup.join("drawing.sketch.json")).unwrap();
        fs::write(
            backup.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "format": "spatial-sketch-backup",
                "version": 1,
                "original_file": "source.sketch.json"
            }))
            .unwrap(),
        )
        .unwrap();
        restore_native_backup(&backup, &restored).unwrap();
        let reopened = fs::read_to_string(&restored).unwrap();
        assert_eq!(
            serde_json::to_value(crate::document::Document::from_json(&reopened).unwrap()).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        assert!(restore_native_backup(&backup, &restored).is_err());
        assert!(restored.is_file());
    }

    #[test]
    fn rename_rejects_duplicate_drawing_names_without_changing_either_file() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source = root.path().join("study.sketch.json");
        let destination = root.path().join("study-copy.sketch.json");
        let source_bytes = serde_json::to_vec(&crate::document::Document::default()).unwrap();
        let destination_bytes = b"keep the existing drawing";
        fs::write(&source, &source_bytes).unwrap();
        fs::write(&destination, destination_bytes).unwrap();
        let mut library = drawing_library_for_test(root.path().join("thumbnails"));

        assert!(library
            .rename_drawing(&source, "study-copy.sketch")
            .is_err());
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        assert_eq!(fs::read(&destination).unwrap(), destination_bytes);
    }

    #[test]
    fn abandoned_backup_staging_directory_is_not_listed_or_consumed_on_retry() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let trash = root.path().join("trash");
        fs::create_dir(&trash).unwrap();
        let source = root.path().join("recovered.sketch.json");
        fs::write(
            &source,
            serde_json::to_vec(&crate::document::Document::default()).unwrap(),
        )
        .unwrap();

        // This is the state left if a process stops while backup_native_drawing
        // is still building its private tempdir in the trash parent.
        let abandoned_stage = trash.join(".sketch-backup-interrupted");
        fs::create_dir(&abandoned_stage).unwrap();
        fs::write(abandoned_stage.join("drawing.sketch.json"), b"partial").unwrap();

        let visible_before_retry = list_trash_in(&trash).unwrap();
        assert!(visible_before_retry.is_empty());
        assert!(
            abandoned_stage.is_dir(),
            "listing must not mutate crash residue"
        );
        assert!(
            source.is_file(),
            "the original remains safe until backup install"
        );

        let installed = trash.join("recovered.sketch-backup");
        backup_native_drawing(&source, &installed).unwrap();
        assert!(source.is_file());
        assert!(installed.is_dir());
        let visible_after_retry = list_trash_in(&trash).unwrap();
        assert_eq!(visible_after_retry.len(), 1);
        assert_eq!(visible_after_retry[0].name, "recovered.sketch.json");
        assert!(visible_after_retry[0].issue.is_none());
        assert!(
            abandoned_stage.is_dir(),
            "retry does not delete unknown leftovers"
        );
    }

    #[test]
    fn portable_backup_restores_image_plane_assets_after_sources_are_removed() {
        use image::ImageEncoder;

        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source = root.path().join("with-image.sketch.json");
        let backup = root.path().join("portable.sketch-backup");
        let restored = root.path().join("restored.sketch.json");
        let image = crate::image_planes::ImagePlane::from_asset(
            8,
            "Reference photo",
            "image-8.png".into(),
            [1, 1],
            glam::Vec3::new(1.0, 2.0, 3.0),
            glam::Quat::IDENTITY,
            1.0,
        )
        .unwrap();
        let document = crate::document::Document {
            linked_stroke_groups: Default::default(),
            image_planes: vec![image.clone()],
            ..Default::default()
        };
        fs::write(&source, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        let source_assets = crate::image_plane_storage::asset_root(&source).unwrap();
        fs::create_dir_all(&source_assets).unwrap();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[12, 34, 56, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        fs::write(source_assets.join("image-8.png"), &png).unwrap();

        backup_native_drawing(&source, &backup).unwrap();
        fs::remove_file(&source).unwrap();
        fs::remove_dir_all(&source_assets).unwrap();
        restore_native_backup(&backup, &restored).unwrap();

        let restored_doc =
            crate::document::Document::from_json(&fs::read_to_string(&restored).unwrap()).unwrap();
        assert_eq!(restored_doc.image_planes, vec![image]);
        let restored_assets = crate::image_plane_storage::asset_root(&restored).unwrap();
        let restored_image =
            crate::image_planes::decode_asset(&restored_assets.join("image-8.png")).unwrap();
        assert_eq!(restored_image.pixels, [12, 34, 56, 255]);

        let missing_source = root.path().join("missing-asset.sketch.json");
        fs::write(&missing_source, serde_json::to_vec(&document).unwrap()).unwrap();
        let missing_backup = root.path().join("missing-asset.sketch-backup");
        assert!(backup_native_drawing(&missing_source, &missing_backup).is_err());
        assert!(!missing_backup.exists());

        let corrupt = root.path().join("corrupt.sketch-backup");
        fs::create_dir(&corrupt).unwrap();
        fs::write(corrupt.join("manifest.json"), b"not json").unwrap();
        let rejected = root.path().join("should-not-exist.sketch.json");
        assert!(restore_native_backup(&corrupt, &rejected).is_err());
        assert!(!rejected.exists());
    }

    #[test]
    fn portable_backup_retains_embedded_transparent_image_tip_after_relocation() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source_dir = root.path().join("original-location");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("image-tip.sketch.json");
        let backup = root.path().join("image-tip.sketch-backup");
        let relocated_dir = root.path().join("relocated");
        fs::create_dir_all(&relocated_dir).unwrap();
        let restored = relocated_dir.join("image-tip.sketch.json");
        let tip_image = image::RgbaImage::from_fn(3, 1, |x, _| {
            image::Rgba([220, 80, 35, [0, 128, 255][x as usize]])
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(tip_image)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let tip = crate::brush_tip::BrushTipAsset::from_png(
            encoded.into_inner(),
            Some("portable backup transparent edge fixture".into()),
        )
        .unwrap();
        let brush = crate::document::Brush {
            tip_asset_id: tip.id,
            tip_aspect: 3.0,
            dabs: crate::dynamics::DabSettings {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let document = crate::document::Document {
            brush_tip_assets: vec![tip.clone()],
            strokes: vec![crate::document::Stroke {
                model_surface: None,
                gradient: None,
                curve: None,
                id: 1,
                layer_id: 1,
                group_id: None,
                material: Default::default(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                fill: None,
                color: [0.3, 0.6, 0.9],
                radius: 0.05,
                brush,
                points: vec![crate::document::Point {
                    position: glam::Vec3::ZERO,
                    pressure: 0.75,
                    tilt: None,
                    time_ms: 0,
                }],
            }],
            ..Default::default()
        };
        fs::write(&source, serde_json::to_vec(&document).unwrap()).unwrap();

        backup_native_drawing(&source, &backup).unwrap();
        fs::remove_file(&source).unwrap();
        fs::remove_dir(&source_dir).unwrap();
        restore_native_backup(&backup, &restored).unwrap();

        let reopened =
            crate::document::Document::from_json(&fs::read_to_string(&restored).unwrap()).unwrap();
        assert_eq!(
            reopened.brush_tip_assets.as_slice(),
            std::slice::from_ref(&tip)
        );
        assert_eq!(reopened.strokes[0].brush.tip_asset_id, tip.id);
        let pixels = reopened.brush_tip_assets[0].decoded().unwrap().pixels;
        assert_eq!([pixels[3], pixels[7], pixels[11]], [0, 128, 255]);
    }

    #[test]
    fn drawing_trash_lists_corrupt_entries_and_restores_assets_without_clobbering() {
        use image::ImageEncoder;

        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source = root.path().join("garden.sketch.json");
        let trash = root.path().join("trash");
        let image = crate::image_planes::ImagePlane::from_asset(
            19,
            "Garden reference",
            "image-19.png".into(),
            [1, 1],
            glam::Vec3::ZERO,
            glam::Quat::IDENTITY,
            1.0,
        )
        .unwrap();
        let source_assets = crate::image_plane_storage::asset_root(&source).unwrap();
        fs::create_dir_all(&source_assets).unwrap();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[43, 87, 129, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        fs::write(source_assets.join("image-19.png"), png).unwrap();

        let external_model = root.path().join("external-model");
        fs::create_dir(&external_model).unwrap();
        let obj_path = external_model.join("source.obj");
        let mtl_path = external_model.join("source.mtl");
        fs::write(
            &obj_path,
            "mtllib source.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl red\nf 1 2 3\n",
        )
        .unwrap();
        fs::write(&mtl_path, "newmtl red\nKd 0.8 0.1 0.2\nd 0.7\n").unwrap();
        let parsed_model = crate::obj_reference::load(&obj_path).unwrap();
        let model_asset =
            crate::obj_reference_storage::import_bundle(&parsed_model, &source_assets, 23).unwrap();
        fs::remove_file(&obj_path).unwrap();
        fs::remove_file(&mtl_path).unwrap();
        let model_reference = crate::document::model_references::ObjReferenceResource {
            id: 23,
            name: "Garden study model".into(),
            visible: true,
            locked: false,
            center: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: 1.0,
            asset: model_asset,
        };
        let document = crate::document::Document {
            linked_stroke_groups: Default::default(),
            image_planes: vec![image.clone()],
            model_references: vec![model_reference.clone()],
            ..Default::default()
        };
        fs::write(&source, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

        let bundle = trash_native_drawing_in(&source, &trash).unwrap();
        assert!(!source.exists());
        assert!(!source_assets.exists());

        let corrupt = trash.join("corrupt.sketch-backup");
        fs::create_dir(&corrupt).unwrap();
        fs::write(corrupt.join("manifest.json"), b"not json").unwrap();
        let entries = list_trash_in(&trash).unwrap();
        let listed = entries.iter().find(|entry| entry.path == bundle).unwrap();
        assert_eq!(listed.name, "garden.sketch.json");
        assert!(listed.deleted.is_some());
        assert!(listed.issue.is_none());
        assert!(entries
            .iter()
            .any(|entry| entry.path == corrupt && entry.issue.is_some()));
        drop(entries);
        let entries_after_view_reopen = list_trash_in(&trash).unwrap();
        assert!(entries_after_view_reopen
            .iter()
            .any(|entry| entry.path == bundle && entry.name == "garden.sketch.json"));

        let conflict = root.path().join("existing.sketch.json");
        fs::write(&conflict, b"keep existing drawing").unwrap();
        assert!(restore_from_trash_in(&bundle, &conflict, &trash).is_err());
        assert_eq!(fs::read(&conflict).unwrap(), b"keep existing drawing");
        assert!(bundle.is_dir(), "failed restore must retain the backup");

        let destination_dir = root.path().join("relocated");
        fs::create_dir(&destination_dir).unwrap();
        let destination = destination_dir.join("restored.sketch.json");
        restore_from_trash_in(&bundle, &destination, &trash).unwrap();
        assert!(!bundle.exists());
        let restored =
            crate::document::Document::from_json(&fs::read_to_string(&destination).unwrap())
                .unwrap();
        assert_eq!(restored.image_planes, vec![image]);
        assert_eq!(restored.model_references, vec![model_reference.clone()]);
        let restored_assets = crate::image_plane_storage::asset_root(&destination).unwrap();
        let restored_image =
            crate::image_planes::decode_asset(&restored_assets.join("image-19.png")).unwrap();
        assert_eq!(restored_image.pixels, [43, 87, 129, 255]);
        let restored_model = crate::obj_reference_storage::load_bundle(
            &restored_assets,
            &restored.model_references[0].asset,
        )
        .unwrap();
        assert_eq!(restored_model.positions, parsed_model.positions);
        assert_eq!(restored_model.triangles, parsed_model.triangles);
        assert_eq!(restored_model.materials, parsed_model.materials);
    }
}
