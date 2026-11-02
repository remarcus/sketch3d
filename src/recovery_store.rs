//! Atomic per-session recovery files, separate from deliberate native saves.
use crate::document::Document;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
// Drawing-local assets are encoded as base64 in the recovery JSON. Their
// decoded payload remains capped at 256 MiB; allow the encoding overhead here.
const MAX_BYTES: u64 = 384 * 1024 * 1024;
pub const MAX_ASSET_BYTES: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 8192;
const STALE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize, Deserialize)]
pub struct RecoverySnapshot {
    pub version: u32,
    pub session_id: String,
    pub saved_at_unix_ms: u64,
    pub original_path: Option<PathBuf>,
    pub document: Document,
    /// Drawing-local image files captured with the document so a recovered
    /// copy can be restored even if the original drawing folder moved.
    #[serde(default)]
    pub assets: Vec<RecoveryAsset>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecoveryAsset {
    /// A generated image filename or a path inside a referenced OBJ bundle.
    pub asset_file: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

impl RecoverySnapshot {
    #[allow(dead_code)] // Keeps the resource-free constructor available for callers.
    pub fn new(session_id: String, original_path: Option<PathBuf>, document: Document) -> Self {
        Self::new_with_assets(session_id, original_path, document, Vec::new())
    }

    pub fn new_with_assets(
        session_id: String,
        original_path: Option<PathBuf>,
        document: Document,
        assets: Vec<RecoveryAsset>,
    ) -> Self {
        Self {
            version: 1,
            session_id,
            saved_at_unix_ms: now_ms(),
            original_path,
            document,
            assets,
        }
    }
}

/// Small local base64 codec avoids turning every image byte into a JSON number
/// (which would make snapshots several times larger). Input size is bounded by
/// the recovery envelope before deserialization.
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut encoded = String::with_capacity(bytes.len().saturating_add(2) / 3 * 4);
        for chunk in bytes.chunks(3) {
            let a = chunk[0];
            let b = *chunk.get(1).unwrap_or(&0);
            let c = *chunk.get(2).unwrap_or(&0);
            encoded.push(TABLE[(a >> 2) as usize] as char);
            encoded.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
            encoded.push(if chunk.len() > 1 {
                TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char
            } else {
                '='
            });
            encoded.push(if chunk.len() > 2 {
                TABLE[(c & 63) as usize] as char
            } else {
                '='
            });
        }
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() % 4 != 0 {
            return Err(serde::de::Error::custom("invalid base64 image payload"));
        }
        let estimated = encoded.len() / 4 * 3;
        if estimated > crate::image_planes::MAX_IMAGE_BYTES as usize + 2 {
            return Err(serde::de::Error::custom("image payload exceeds 32 MiB"));
        }
        let mut decoded = Vec::with_capacity(estimated);
        for (index, chunk) in encoded.as_bytes().as_chunks::<4>().0.iter().enumerate() {
            let last = index + 1 == encoded.len() / 4;
            let a = sextet(chunk[0])
                .ok_or_else(|| serde::de::Error::custom("invalid base64 image payload"))?;
            let b = sextet(chunk[1])
                .ok_or_else(|| serde::de::Error::custom("invalid base64 image payload"))?;
            let c = if chunk[2] == b'=' {
                0
            } else {
                sextet(chunk[2])
                    .ok_or_else(|| serde::de::Error::custom("invalid base64 image payload"))?
            };
            let d = if chunk[3] == b'=' {
                0
            } else {
                sextet(chunk[3])
                    .ok_or_else(|| serde::de::Error::custom("invalid base64 image payload"))?
            };
            if (!last && (chunk[2] == b'=' || chunk[3] == b'='))
                || (chunk[2] == b'=' && chunk[3] != b'=')
            {
                return Err(serde::de::Error::custom("invalid base64 image padding"));
            }
            decoded.push((a << 2) | (b >> 4));
            if chunk[2] != b'=' {
                decoded.push((b << 4) | (c >> 2));
            }
            if chunk[3] != b'=' {
                decoded.push((c << 6) | d);
            }
        }
        Ok(decoded)
    }

    fn sextet(value: u8) -> Option<u8> {
        match value {
            b'A'..=b'Z' => Some(value - b'A'),
            b'a'..=b'z' => Some(value - b'a' + 26),
            b'0'..=b'9' => Some(value - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
}
#[derive(Clone)]
pub struct Candidate {
    pub path: PathBuf,
    pub snapshot: RecoverySnapshot,
    pub stale: bool,
    pub source_newer: bool,
}
#[derive(Clone)]
pub struct Issue {
    pub path: PathBuf,
    pub message: String,
}
#[derive(Clone, Default)]
pub struct Scan {
    pub candidates: Vec<Candidate>,
    pub issues: Vec<Issue>,
}

pub fn path() -> Result<PathBuf, String> {
    crate::presets::path()?
        .parent()
        .map(|p| p.join("recovery"))
        .ok_or_else(|| "Local recovery folder is unavailable".into())
}
/// The OS releases this exclusive lock when a process exits, including a crash.
/// Keep the returned handle alive for the entire recovery worker lifetime.
pub fn lock_session(dir: &Path, session: &str) -> Result<fs::File, String> {
    file_name(session)?;
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create recovery folder: {e}"))?;
    let lock_path = dir.join(format!("recovery-{session}.lock"));
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Recovery session lock is a symbolic link".into());
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| format!("Cannot open recovery session lock: {e}"))?;
    lock.try_lock()
        .map_err(|e| format!("Cannot lock recovery session: {e}"))?;
    Ok(lock)
}
pub fn session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Restore the bounded, document-local resources carried by a recovery
/// snapshot. Every asset name must refer to a resource in `document`; parent
/// directories and existing targets are checked to reject symlink escapes.
pub fn restore_assets(
    document: &Document,
    asset_root: &Path,
    assets: &[RecoveryAsset],
) -> Result<(), String> {
    validate_assets(document, assets)?;
    fs::create_dir_all(asset_root)
        .map_err(|error| format!("Cannot create recovered asset folder: {error}"))?;
    let root_metadata = fs::symlink_metadata(asset_root)
        .map_err(|error| format!("Cannot inspect recovered asset folder: {error}"))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err("Recovered asset folder must be a regular directory".into());
    }
    let canonical_root = asset_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve recovered asset folder: {error}"))?;

    for asset in assets {
        let relative = Path::new(&asset.asset_file);
        let target = asset_root.join(relative);
        let parent = target
            .parent()
            .ok_or("Recovered asset path has no parent directory")?;
        let mut directory = asset_root.to_path_buf();
        if let Some(relative_parent) = relative.parent() {
            for component in relative_parent.components() {
                let std::path::Component::Normal(name) = component else {
                    return Err("Recovered asset path contains an invalid directory".into());
                };
                directory.push(name);
                match fs::symlink_metadata(&directory) {
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                    Ok(_) => {
                        return Err("Recovered resource folder must be a regular directory".into())
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        fs::create_dir(&directory).map_err(|error| {
                            format!("Cannot create recovered resource folder: {error}")
                        })?;
                    }
                    Err(error) => {
                        return Err(format!("Cannot inspect recovered resource folder: {error}"))
                    }
                }
            }
        }
        let parent_metadata = fs::symlink_metadata(parent)
            .map_err(|error| format!("Cannot inspect recovered resource folder: {error}"))?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|error| format!("Cannot resolve recovered resource folder: {error}"))?;
        if !parent_metadata.is_dir()
            || parent_metadata.file_type().is_symlink()
            || !canonical_parent.starts_with(&canonical_root)
        {
            return Err("Recovered resource folder must stay inside its asset folder".into());
        }
        if fs::symlink_metadata(&target)
            .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
        {
            return Err("Recovered asset target must be a regular file".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("Cannot stage recovered asset: {error}"))?;
        temp.write_all(&asset.bytes)
            .map_err(|error| format!("Cannot write recovered asset: {error}"))?;
        temp.as_file()
            .sync_all()
            .map_err(|error| format!("Cannot sync recovered asset: {error}"))?;
        crate::atomic_file::replace(temp, &target)
            .map_err(|error| format!("Cannot install recovered asset: {error}"))?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn valid_session(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn file_name(session: &str) -> Result<String, String> {
    if !valid_session(session) {
        return Err("Invalid recovery session identity".into());
    }
    Ok(format!("recovery-{session}.json"))
}
fn valid_file_name(file: &Path) -> bool {
    file.file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("recovery-"))
        .and_then(|s| s.strip_suffix(".json"))
        .is_some_and(valid_session)
}
fn validate_metadata(snapshot: &RecoverySnapshot) -> Result<(), String> {
    if snapshot.version != 1 {
        return Err("Unsupported recovery format version".into());
    }
    file_name(&snapshot.session_id)?;
    if snapshot.saved_at_unix_ms == 0 {
        return Err("Invalid recovery snapshot time".into());
    }
    if snapshot
        .original_path
        .as_ref()
        .is_some_and(|p| p.as_os_str().is_empty() || p.as_os_str().len() > 32768)
    {
        return Err("Invalid original drawing path".into());
    }
    validate_assets(&snapshot.document, &snapshot.assets)
}

fn validate_assets(document: &Document, assets: &[RecoveryAsset]) -> Result<(), String> {
    let mut total = 0usize;
    let mut seen = std::collections::HashSet::new();
    for asset in assets {
        let image_reference = document
            .image_planes
            .iter()
            .any(|plane| plane.asset_file == asset.asset_file);
        let model_reference = model_recovery_asset_referenced(document, &asset.asset_file);
        if (!image_reference && !model_reference)
            || (!valid_asset_file(&asset.asset_file) && !valid_model_asset_file(&asset.asset_file))
            || !seen.insert(&asset.asset_file)
        {
            return Err("Invalid or duplicate recovery asset reference".into());
        }
        let per_file_limit = if image_reference {
            crate::image_planes::MAX_IMAGE_BYTES as usize
        } else if asset.asset_file.ends_with(".obj") {
            crate::obj_reference_storage::MAX_MODEL_OBJ_BYTES as usize
        } else {
            crate::obj_reference_storage::MAX_MODEL_MTL_BYTES as usize
        };
        if asset.bytes.is_empty() || asset.bytes.len() > per_file_limit {
            return Err("Recovery asset exceeds its per-file limit".into());
        }
        total = total.saturating_add(asset.bytes.len());
        if total > MAX_ASSET_BYTES {
            return Err("Recovery assets exceed the 256 MiB total limit".into());
        }
    }
    Ok(())
}

fn valid_asset_file(value: &str) -> bool {
    let Some(stem) = value.strip_prefix("image-") else {
        return false;
    };
    let Some((id, ext)) = stem.rsplit_once('.') else {
        return false;
    };
    id.parse::<u64>().is_ok_and(|id| id != 0 && id != u64::MAX) && matches!(ext, "png" | "jpg")
}
fn valid_model_asset_file(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() != 3 || parts[0] != "models" {
        return false;
    }
    if !parts[1]
        .parse::<u64>()
        .is_ok_and(|id| id != 0 && id != u64::MAX)
    {
        return false;
    }
    if parts[2] == "model.obj" {
        return true;
    }
    let Some(number) = parts[2]
        .strip_prefix("material-")
        .and_then(|name| name.strip_suffix(".mtl"))
    else {
        return false;
    };
    number.parse::<u8>().is_ok_and(|index| index < 64)
        && number.len() == 3
        && number.bytes().all(|byte| byte.is_ascii_digit())
}
fn model_recovery_asset_referenced(document: &Document, path: &str) -> bool {
    let mut parts = path.split('/');
    let (Some("models"), Some(id), Some(_file), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Ok(id) = id.parse::<u64>() else {
        return false;
    };
    document
        .model_references
        .iter()
        .any(|resource| resource.id == id && resource.asset.bundle_dir == format!("models/{id}"))
}
fn normalized(snapshot: &RecoverySnapshot) -> Result<RecoverySnapshot, String> {
    validate_metadata(snapshot)?;
    let document = Document::from_json(
        &serde_json::to_string(&snapshot.document).map_err(|e| e.to_string())?,
    )?;
    Ok(RecoverySnapshot {
        document,
        ..snapshot.clone()
    })
}
fn alias_path(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    absolute.canonicalize().ok().or_else(|| {
        Some(
            absolute
                .parent()?
                .canonicalize()
                .ok()?
                .join(absolute.file_name()?),
        )
    })
}
fn aliases(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (alias_path(a), alias_path(b)) {
        (Some(a), Some(b)) => {
            #[cfg(windows)]
            {
                a.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
            }
            #[cfg(not(windows))]
            {
                a == b
            }
        }
        _ => false,
    }
}
pub fn write(dir: &Path, snapshot: &RecoverySnapshot) -> Result<PathBuf, String> {
    let snapshot = normalized(snapshot)?;
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create recovery folder: {e}"))?;
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let destination = dir.join(file_name(&snapshot.session_id)?);
    if snapshot
        .original_path
        .as_ref()
        .is_some_and(|p| aliases(p, &destination))
    {
        return Err("Recovery destination must not replace the native drawing".into());
    }
    if fs::symlink_metadata(&destination).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Recovery destination is a symbolic link".into());
    }
    let bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Recovery snapshot exceeds 384 MiB".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    temporary
        .write_all(&bytes)
        .map_err(|e| format!("Cannot write recovery snapshot: {e}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| format!("Cannot sync recovery snapshot: {e}"))?;
    crate::atomic_file::replace(temporary, &destination)
        .map_err(|e| format!("Cannot replace recovery snapshot: {e}"))?;
    Ok(destination)
}

/// Removal is restricted to a regular recovery file directly in this folder.
pub fn remove(dir: &Path, file: &Path) -> Result<(), String> {
    if !valid_file_name(file) {
        return Err("Not a recovery filename".into());
    }
    let dir = match dir.canonicalize() {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let parent = file
        .parent()
        .ok_or("Recovery file has no parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent != dir {
        return Err("Recovery file is outside the recovery folder".into());
    }
    let metadata = match fs::symlink_metadata(file) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Recovery file must be a regular file".into());
    }
    let canonical = file.canonicalize().map_err(|e| e.to_string())?;
    if canonical.parent() != Some(dir.as_path()) {
        return Err("Recovery file resolves outside the recovery folder".into());
    }
    // A deliberately named native drawing is never a disposable recovery record.
    if metadata.len() <= MAX_BYTES {
        if let Ok(text) = fs::read_to_string(file) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if value.get("strokes").is_some() && value.get("document").is_none() {
                    return Err("Refusing to remove a native drawing".into());
                }
                if value
                    .get("original_path")
                    .and_then(|p| p.as_str())
                    .is_some_and(|p| aliases(Path::new(p), file))
                {
                    return Err("Recovery record aliases its native drawing".into());
                }
            }
        }
    }
    fs::remove_file(file).map_err(|e| format!("Cannot remove recovery file: {e}"))
}
fn read(file: &Path) -> Result<RecoverySnapshot, String> {
    let metadata = fs::symlink_metadata(file).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Recovery entry is not a regular file".into());
    }
    if metadata.len() > MAX_BYTES {
        return Err("Recovery file exceeds 384 MiB".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Recovery file grew beyond 384 MiB".into());
    }
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Incomplete or corrupt recovery file: {e}"))?;
    let version = value.get("version").and_then(|v| v.as_u64());
    if version != Some(1) {
        return Err("Unsupported recovery format version".into());
    }
    let raw_document = value
        .get_mut("document")
        .ok_or("Recovery document is missing")?
        .take();
    let document = Document::from_json(&raw_document.to_string())
        .map_err(|e| format!("Invalid recovery drawing: {e}"))?;
    // Decode envelope independently, allowing native schema migrations first.
    value["document"] = serde_json::to_value(Document::default()).map_err(|e| e.to_string())?;
    let mut snapshot: RecoverySnapshot =
        serde_json::from_value(value).map_err(|e| format!("Invalid recovery metadata: {e}"))?;
    snapshot.document = document;
    validate_metadata(&snapshot)?;
    if file.file_name().and_then(|s| s.to_str()) != Some(file_name(&snapshot.session_id)?.as_str())
    {
        return Err("Recovery filename and session identity disagree".into());
    }
    if snapshot
        .original_path
        .as_ref()
        .is_some_and(|p| aliases(p, file))
    {
        return Err("Recovery record aliases its native drawing".into());
    }
    Ok(snapshot)
}

pub fn scan(dir: &Path, own_session: &str) -> Result<Scan, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Scan::default()),
        Err(e) => return Err(format!("Cannot scan recovery folder: {e}")),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if valid_file_name(&entry.path())
            && entry.file_name() != file_name(own_session).unwrap_or_default().as_str()
        {
            files.push(entry.path());
        }
    }
    files.sort();
    let mut result = Scan::default();
    let mut total_bytes = 0u64;
    for (index, file) in files.into_iter().enumerate() {
        let lock_path = file.with_extension("lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
            result.issues.push(Issue {
                path: file,
                message: "Recovery session lock is a symbolic link".into(),
            });
            continue;
        }
        // A live instance owns its lock; leave its recovery snapshot untouched.
        let _lock_guard = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(lock) => match lock.try_lock() {
                Ok(()) => Some(lock),
                Err(fs::TryLockError::WouldBlock) => continue,
                Err(fs::TryLockError::Error(error)) => {
                    result.issues.push(Issue {
                        path: file,
                        message: format!("Cannot inspect recovery session lock: {error}"),
                    });
                    continue;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                result.issues.push(Issue {
                    path: file,
                    message: format!("Cannot inspect recovery session lock: {error}"),
                });
                continue;
            }
        };
        let size = fs::symlink_metadata(&file).map(|m| m.len()).unwrap_or(0);
        total_bytes = total_bytes.saturating_add(size);
        if index >= MAX_FILES || total_bytes > MAX_BYTES {
            result.issues.push(Issue {
                path: file,
                message: "Recovery scan limit reached; this file was left untouched".into(),
            });
            continue;
        }
        match read(&file) {
            Ok(snapshot) => {
                let source_newer = snapshot
                    .original_path
                    .as_ref()
                    .and_then(|p| fs::metadata(p).ok())
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .is_some_and(|d| d.as_millis() > snapshot.saved_at_unix_ms as u128);
                let now = now_ms();
                let stale = snapshot.saved_at_unix_ms > now
                    || now.saturating_sub(snapshot.saved_at_unix_ms) > STALE_MS;
                result.candidates.push(Candidate {
                    path: file,
                    snapshot,
                    stale,
                    source_newer,
                });
            }
            Err(message) => result.issues.push(Issue {
                path: file,
                message,
            }),
        }
    }
    result.candidates.sort_by(|a, b| {
        b.snapshot
            .saved_at_unix_ms
            .cmp(&a.snapshot.saved_at_unix_ms)
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Brush, BrushKind, Point, Stroke},
        texture::TextureKind,
    };
    use glam::{Quat, Vec3};

    #[test]
    fn asset_restore_rejects_unreferenced_files_before_creating_destination() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let destination = directory.path().join("recovered-assets");
        let assets = vec![RecoveryAsset {
            asset_file: "models/42/model.obj".into(),
            bytes: b"v 0 0 0\n".to_vec(),
        }];

        assert!(restore_assets(&Document::default(), &destination, &assets).is_err());
        assert!(!destination.exists());
    }

    fn sample(session: &str) -> RecoverySnapshot {
        let mut brush = Brush {
            kind: BrushKind::Chisel,
            ..Brush::default()
        };
        brush.dynamics.tilt_orientation = true;
        brush.dynamics.width.curve = 1.8;
        brush.dynamics.taper_end = 0.7;
        brush.texture.kind = TextureKind::Pattern;
        brush.texture.seed = 987;
        RecoverySnapshot::new(
            session.into(),
            None,
            Document {
                strokes: vec![Stroke {
                    curve: None,
                    material: Default::default(),
                    id: 123,
                    fill: None,
                    group_id: None,
                    layer_id: 1,
                    color: [0.3, 0.6, 0.8],
                    radius: 0.08,
                    brush,
                    points: vec![
                        Point {
                            position: Vec3::ZERO,
                            pressure: 0.23,
                            tilt: Some(Vec3::Y),

                            time_ms: 0,
                        },
                        Point {
                            position: Vec3::X,
                            pressure: 0.91,
                            tilt: Some(Vec3::Z),

                            time_ms: 0,
                        },
                    ],
                }],
                ..Document::default()
            },
        )
    }
    #[test]
    fn atomic_roundtrip_preserves_pressure_tilt_textures_and_source_identity() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("native.sketch.json");
        fs::write(&source, "unchanged deliberate save").unwrap();
        let mut snapshot = sample("session-a");
        snapshot.original_path = Some(source.clone());
        let written = write(&dir.path().join("recovery"), &snapshot).unwrap();
        let scanned = scan(&dir.path().join("recovery"), "other").unwrap();
        assert_eq!(scanned.candidates.len(), 1);
        assert!(scanned.issues.is_empty());
        let loaded = &scanned.candidates[0].snapshot;
        assert_eq!(loaded.document.strokes, snapshot.document.strokes);
        assert_eq!(loaded.original_path, Some(source.clone()));
        assert_eq!(loaded.saved_at_unix_ms, snapshot.saved_at_unix_ms);
        assert_eq!(
            fs::read_to_string(source).unwrap(),
            "unchanged deliberate save"
        );
        assert_eq!(written, scanned.candidates[0].path.canonicalize().unwrap());
        remove(&dir.path().join("recovery"), &written).unwrap();
        remove(&dir.path().join("recovery"), &written).unwrap();
        assert!(scan(&dir.path().join("recovery"), "other")
            .unwrap()
            .candidates
            .is_empty());
    }
    #[test]
    fn image_plane_assets_survive_recovery_snapshot_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let plane = crate::image_planes::ImagePlane::from_asset(
            7,
            "Facade",
            "image-7.png".into(),
            [2, 1],
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(0.35),
            2.0,
        )
        .unwrap();
        let bytes = vec![137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3];
        let snapshot = RecoverySnapshot::new_with_assets(
            "image-session".into(),
            None,
            Document {
                image_planes: vec![plane.clone()],
                ..Document::default()
            },
            vec![RecoveryAsset {
                asset_file: plane.asset_file.clone(),
                bytes: bytes.clone(),
            }],
        );
        write(dir.path(), &snapshot).unwrap();
        let scan = scan(dir.path(), "another-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        let recovered = &scan.candidates[0].snapshot;
        assert_eq!(recovered.document.image_planes, vec![plane]);
        assert_eq!(recovered.assets[0].asset_file, "image-7.png");
        assert_eq!(recovered.assets[0].bytes, bytes);
    }
    #[test]
    fn obj_model_and_material_bundles_survive_recovery_snapshot_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let obj = b"mtllib material-000.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec();
        let mtl = b"newmtl default\nKd 0.7 0.8 0.9\n".to_vec();
        let resource = crate::document::model_references::ObjReferenceResource {
            id: 42,
            name: "Recovery model".into(),
            visible: true,
            locked: false,
            center: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::IDENTITY,
            scale: 1.25,
            asset: crate::obj_reference_storage::ObjReferenceAsset {
                id: 42,
                bundle_dir: "models/42".into(),
            },
        };
        let assets = vec![
            RecoveryAsset {
                asset_file: "models/42/model.obj".into(),
                bytes: obj.clone(),
            },
            RecoveryAsset {
                asset_file: "models/42/material-000.mtl".into(),
                bytes: mtl.clone(),
            },
        ];
        let surface_stroke = crate::document::Stroke {
            curve: None,
            material: Default::default(),
            fill: None,
            group_id: None,
            layer_id: 1,
            id: 7,
            color: [0.2, 0.6, 0.9],
            radius: 0.01,
            brush: crate::document::Brush {
                normal: Vec3::Z,
                tangent: Vec3::X,
                ..Default::default()
            },
            points: [Vec3::new(0.2, 0.2, 0.0), Vec3::new(0.6, 0.2, 0.0)]
                .into_iter()
                .map(|position| crate::document::Point {
                    position: resource.center + resource.rotation * (position * resource.scale),
                    pressure: 0.8,
                    tilt: None,

                    time_ms: 0,
                })
                .collect(),
        };
        let mut document = Document {
            model_references: vec![resource.clone()],
            active_model_reference: Some(42),
            ..Document::default()
        };
        let stroke_edit = crate::editing::Edit::Change {
            before: Vec::new(),
            after: vec![(0, surface_stroke.clone())],
        };
        stroke_edit.apply(&mut document, true);
        assert_eq!(document.strokes, vec![surface_stroke.clone()]);
        stroke_edit.apply(&mut document, false);
        assert!(document.strokes.is_empty());
        stroke_edit.apply(&mut document, true);
        assert_eq!(document.strokes, vec![surface_stroke.clone()]);

        let before_model_edit = (
            document.model_references.clone(),
            document.active_model_reference,
            document.image_planes.clone(),
            document.active_image_plane,
            document.guide_resources.clone(),
            document.active_guide,
        );
        let mut moved_model = resource.clone();
        moved_model.center += Vec3::new(0.5, -0.25, 0.75);
        moved_model.rotation = Quat::from_rotation_y(0.3);
        let after_model_edit = (
            vec![moved_model],
            document.active_model_reference,
            document.image_planes.clone(),
            document.active_image_plane,
            document.guide_resources.clone(),
            document.active_guide,
        );
        let transform_edit = crate::editing::Edit::ModelReferences {
            before: before_model_edit.clone(),
            after: after_model_edit.clone(),
        };
        transform_edit.apply(&mut document, true);
        assert_eq!(document.strokes, vec![surface_stroke.clone()]);
        transform_edit.apply(&mut document, false);
        assert_eq!(document.model_references, vec![resource.clone()]);
        assert_eq!(document.strokes, vec![surface_stroke.clone()]);
        transform_edit.apply(&mut document, true);
        assert_eq!(document.model_references, after_model_edit.0);
        assert_eq!(document.strokes, vec![surface_stroke.clone()]);

        let native_reloaded = Document::from_json(&serde_json::to_string(&document).unwrap())
            .expect("native document reload preserves model surface ink");
        assert_eq!(native_reloaded.strokes, vec![surface_stroke.clone()]);
        let snapshot = RecoverySnapshot::new_with_assets(
            "model-session".into(),
            None,
            native_reloaded,
            assets,
        );
        write(dir.path(), &snapshot).unwrap();
        let scan = scan(dir.path(), "another-session").unwrap();
        assert_eq!(scan.candidates.len(), 1);
        let recovered = &scan.candidates[0].snapshot;
        assert_eq!(recovered.document.strokes, vec![surface_stroke]);
        assert_eq!(recovered.document.model_references, after_model_edit.0);
        assert_eq!(recovered.document.active_model_reference, Some(42));
        assert_eq!(recovered.assets.len(), 2);
        assert_eq!(recovered.assets[0].asset_file, "models/42/model.obj");
        assert_eq!(recovered.assets[0].bytes, obj);
        assert_eq!(recovered.assets[1].asset_file, "models/42/material-000.mtl");
        assert_eq!(recovered.assets[1].bytes, mtl);

        let restored_root = dir.path().join("recovered-drawing.assets");
        restore_assets(&recovered.document, &restored_root, &recovered.assets).unwrap();
        let restored_model =
            crate::obj_reference_storage::load_bundle(&restored_root, &resource.asset).unwrap();
        assert_eq!(restored_model.triangles.len(), 1);
        assert_eq!(restored_model.materials.len(), 1);
        assert_eq!(restored_model.materials[0].name, "default");
        assert_eq!(restored_model.materials[0].diffuse, [0.7, 0.8, 0.9]);
        assert_eq!(restored_model.materials[0].opacity, 1.0);
        assert_eq!(
            restored_model.bounds,
            [Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0)]
        );
    }
    #[test]
    fn failed_replacement_retains_complete_previous_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let valid = sample("session-a");
        let destination = write(dir.path(), &valid).unwrap();
        let original = fs::read(&destination).unwrap();
        let mut invalid = valid.clone();
        invalid.document.strokes[0].points[0].pressure = 2.0;
        assert!(write(dir.path(), &invalid).is_err());
        assert_eq!(fs::read(&destination).unwrap(), original);
        invalid = valid.clone();
        invalid.version = 99;
        assert!(write(dir.path(), &invalid).is_err());
        assert_eq!(fs::read(&destination).unwrap(), original);
        let blocked = dir.path().join("not-a-folder");
        fs::write(&blocked, "occupied").unwrap();
        assert!(write(&blocked, &valid).is_err());
        assert_eq!(fs::read(&destination).unwrap(), original);
    }
    #[test]
    fn interrupted_corrupt_and_unsupported_entries_are_reported_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &sample("valid")).unwrap();
        fs::write(dir.path().join(".tmp-interrupted"), "{partial").unwrap();
        fs::write(dir.path().join("recovery-incomplete.json"), "{partial").unwrap();
        let mut unsupported = serde_json::to_value(sample("unsupported")).unwrap();
        unsupported["version"] = 99.into();
        fs::write(
            dir.path().join("recovery-unsupported.json"),
            unsupported.to_string(),
        )
        .unwrap();
        let mut native = serde_json::to_value(sample("future-native")).unwrap();
        native["document"]["version"] = 99.into();
        fs::write(
            dir.path().join("recovery-future-native.json"),
            native.to_string(),
        )
        .unwrap();
        let result = scan(dir.path(), "other").unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.issues.len(), 3);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Incomplete or corrupt")));
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Unsupported recovery")));
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Invalid recovery drawing")));
        assert_eq!(
            fs::read_to_string(dir.path().join(".tmp-interrupted")).unwrap(),
            "{partial"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("recovery-incomplete.json")).unwrap(),
            "{partial"
        );
        remove(dir.path(), &dir.path().join("recovery-incomplete.json")).unwrap();
    }
    #[test]
    fn old_future_and_newer_native_snapshots_are_flagged_but_retained() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = sample("old");
        old.saved_at_unix_ms = now_ms() - STALE_MS - 1000;
        write(dir.path(), &old).unwrap();
        let mut future = sample("future");
        future.saved_at_unix_ms = now_ms() + 60000;
        write(dir.path(), &future).unwrap();
        let native = dir.path().join("source.sketch.json");
        fs::write(&native, "newer native version").unwrap();
        let mut newer = sample("source-newer");
        newer.original_path = Some(native);
        newer.saved_at_unix_ms = now_ms() - 1000;
        write(dir.path(), &newer).unwrap();
        let result = scan(dir.path(), "other").unwrap();
        assert_eq!(result.candidates.len(), 3);
        assert_eq!(result.candidates[0].snapshot.session_id, "future");
        assert_eq!(result.candidates.iter().filter(|c| c.stale).count(), 2);
        assert!(
            result
                .candidates
                .iter()
                .find(|c| c.snapshot.session_id == "source-newer")
                .unwrap()
                .source_newer
        );
        assert!(result.candidates.iter().all(|c| c.path.exists()));
    }
    #[test]
    fn separate_sessions_do_not_replace_one_another_and_own_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let a = session_id();
        let b = session_id();
        assert_ne!(a, b);
        let first = write(dir.path(), &sample(&a)).unwrap();
        let second = write(dir.path(), &sample(&b)).unwrap();
        assert_ne!(first, second);
        let result = scan(dir.path(), &a).unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].snapshot.session_id, b);
        remove(dir.path(), &first).unwrap();
        assert!(second.exists());
    }
    #[test]
    fn path_boundaries_and_native_drawings_are_protected() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outsider = write(outside.path(), &sample("outside")).unwrap();
        assert!(remove(dir.path(), &outsider).is_err());
        assert!(outsider.exists());
        let nested = dir.path().join("nested");
        let child = write(&nested, &sample("child")).unwrap();
        assert!(remove(dir.path(), &child).is_err());
        let native = dir.path().join("recovery-native.json");
        fs::write(&native, serde_json::to_vec(&Document::default()).unwrap()).unwrap();
        assert!(remove(dir.path(), &native).is_err());
        assert!(native.exists());
        let mut traversal = sample("../outside");
        assert!(write(dir.path(), &traversal).is_err());
        traversal = sample("alias");
        traversal.original_path = Some(dir.path().join("recovery-alias.json"));
        assert!(write(dir.path(), &traversal).is_err());
        let absent = dir.path().join("missing");
        remove(&absent, &absent.join("recovery-gone.json")).unwrap();
    }
    #[test]
    fn legacy_native_payload_migrates_and_identity_mismatch_is_not_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = serde_json::to_value(sample("legacy")).unwrap();
        old["document"]["version"] = 3.into();
        old["document"]["strokes"][0]["brush"]
            .as_object_mut()
            .unwrap()
            .remove("texture");
        fs::write(dir.path().join("recovery-legacy.json"), old.to_string()).unwrap();
        let result = scan(dir.path(), "other").unwrap();
        assert_eq!(result.candidates[0].snapshot.document.version, 16);
        assert_eq!(
            result.candidates[0].snapshot.document.strokes[0]
                .brush
                .texture
                .kind,
            TextureKind::Solid
        );
        fs::write(dir.path().join("recovery-wrong.json"), old.to_string()).unwrap();
        let result = scan(dir.path(), "other").unwrap();
        assert_eq!(result.issues.len(), 1);
        assert!(result.issues[0].message.contains("identity disagree"));
    }
    #[test]
    fn live_session_lock_excludes_snapshot_until_process_handle_closes() {
        let dir = tempfile::tempdir().unwrap();
        let held = lock_session(dir.path(), "live").unwrap();
        write(dir.path(), &sample("live")).unwrap();
        assert!(scan(dir.path(), "other").unwrap().candidates.is_empty());
        assert!(lock_session(dir.path(), "live").is_err());
        drop(held);
        let scanned = scan(dir.path(), "other").unwrap();
        assert_eq!(scanned.candidates.len(), 1);
        assert!(scanned.issues.is_empty());
    }
}
