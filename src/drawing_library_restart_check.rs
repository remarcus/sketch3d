//! Process-boundary verification for the on-disk recoverable drawing trash.
use crate::{document::Document, drawing_library};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Serialize, Deserialize)]
struct RestartFixture {
    source: String,
    backup: String,
    interrupted_stage: String,
}

/// First process: write a valid drawing and move it into the configured trash.
pub fn seed(marker_path: &Path) -> Result<(), String> {
    let parent = marker_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or("Restart marker must have a parent folder")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let source = parent.join("drawing-trash-restart-source.sketch.json");
    fs::write(
        &source,
        serde_json::to_vec_pretty(&Document::default()).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot create restart fixture drawing: {error}"))?;
    let trash_root = crate::presets::path()
        .map_err(|error| error.to_string())?
        .join("drawing-trash");
    fs::create_dir_all(&trash_root).map_err(|error| error.to_string())?;
    let interrupted_stage = trash_root.join(".sketch-backup-interrupted");
    fs::create_dir_all(&interrupted_stage).map_err(|error| error.to_string())?;
    fs::write(
        interrupted_stage.join("drawing.sketch.json"),
        b"partial backup left by a terminated process",
    )
    .map_err(|error| error.to_string())?;
    let backup = drawing_library::trash_native_drawing(&source)?;
    let fixture = RestartFixture {
        source: source.to_string_lossy().into_owned(),
        backup: backup.to_string_lossy().into_owned(),
        interrupted_stage: interrupted_stage.to_string_lossy().into_owned(),
    };
    fs::write(
        marker_path,
        serde_json::to_vec_pretty(&fixture).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot save restart fixture marker: {error}"))?;
    if source.exists() || !backup.is_dir() {
        return Err("First process did not move the drawing into recoverable trash".into());
    }
    Ok(())
}

/// Second process: rediscover the backup, reject a conflicting restore, and
/// restore the drawing into a different directory.
pub fn reopen(
    marker_path: &Path,
    destination_dir: &Path,
    report_path: &Path,
) -> Result<(), String> {
    let fixture: RestartFixture = serde_json::from_slice(
        &fs::read(marker_path).map_err(|error| format!("Cannot read restart marker: {error}"))?,
    )
    .map_err(|error| format!("Restart marker is invalid: {error}"))?;
    let source = Path::new(&fixture.source);
    let backup = Path::new(&fixture.backup);
    let interrupted_stage = Path::new(&fixture.interrupted_stage);
    if source.exists() || !backup.is_dir() {
        return Err("Trash state did not survive the first process exit".into());
    }
    if !interrupted_stage.is_dir() {
        return Err("Interrupted backup fixture did not survive the process restart".into());
    }

    let entries = drawing_library::list_trash()?;
    if entries.iter().any(|entry| entry.path == interrupted_stage) {
        return Err("An abandoned backup staging folder was exposed as a trash item".into());
    }
    let listed = entries
        .iter()
        .find(|entry| entry.path == backup && entry.issue.is_none())
        .ok_or("A fresh process could not rediscover the valid trash backup")?;
    if listed.name != "drawing-trash-restart-source.sketch.json" || listed.deleted.is_none() {
        return Err("Restarted trash entry has incorrect source name or deletion date".into());
    }

    fs::create_dir_all(destination_dir).map_err(|error| error.to_string())?;
    let conflict = destination_dir.join("existing.sketch.json");
    fs::write(&conflict, b"preserve this existing drawing").map_err(|error| error.to_string())?;
    if drawing_library::restore_from_trash(backup, &conflict).is_ok()
        || fs::read(&conflict).map_err(|error| error.to_string())?
            != b"preserve this existing drawing"
        || !backup.is_dir()
    {
        return Err("A conflict restore overwrote data or consumed the backup".into());
    }

    let destination = destination_dir.join("restored-after-restart.sketch.json");
    drawing_library::restore_from_trash(backup, &destination)?;
    let reopened =
        Document::from_json(&fs::read_to_string(&destination).map_err(|error| error.to_string())?)?;
    if !reopened.strokes.is_empty() || reopened.layers != Document::default().layers {
        return Err("Restored drawing content changed across the process restart".into());
    }
    if backup.exists() {
        return Err("Successfully restored backup remained in the trash".into());
    }

    let report = serde_json::json!({
        "passed": true,
        "checks": [
            "first application process moved a valid drawing into on-disk trash",
            "second application process ignores abandoned backup staging from an interrupted operation",
            "second application process rediscovered source name and deletion date after restart",
            "conflicting restore preserved existing data and retained the backup",
            "restore after restart recovered the drawing into a relocated folder"
        ],
        "physical_hardware_tested": false
    });
    fs::write(
        report_path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot write restart verification report: {error}"))?;
    Ok(())
}

/// Child process: create a valid drawing with a managed asset, then begin the
/// actual trash backup. The debug-only copy hook pauses after the first bytes
/// have reached the staging asset file so the parent can terminate this process.
pub fn interrupt_backup_copy(source_path: &Path) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("The deterministic copy interruption verifier requires a debug build".into());
    }
    if std::env::var_os("SKETCH3D_VERIFY_COPY_PAUSE_MARKER").is_none_or(|marker| marker.is_empty())
    {
        return Err("The copy interruption marker is not configured".into());
    }
    let parent = source_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or("Interruption fixture drawing must have a parent folder")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    fs::write(
        source_path,
        serde_json::to_vec_pretty(&Document::default()).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot create interruption fixture drawing: {error}"))?;
    let source_assets = crate::image_plane_storage::asset_root(source_path)?;
    fs::create_dir_all(&source_assets).map_err(|error| error.to_string())?;
    fs::write(
        source_assets.join("interrupt-probe.bin"),
        vec![0x5a; 1024 * 1024],
    )
    .map_err(|error| format!("Cannot create interruption fixture asset: {error}"))?;

    let _ = crate::drawing_library::trash_native_drawing(source_path)?;
    Err("The copy interruption hook did not pause the backup process".into())
}

/// Fresh process: confirm the killed copy left an ignored partial stage, retry
/// the real trash operation, and verify restore recovers the full asset bytes.
pub fn reopen_after_interrupted_copy(
    source_path: &Path,
    trash_root: &Path,
    destination_path: &Path,
    report_path: &Path,
) -> Result<(), String> {
    if !source_path.is_file() {
        return Err("The original drawing was removed before its backup completed".into());
    }
    let stages = fs::read_dir(trash_root)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".sketch-backup-")
                && entry.path().is_dir()
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    if stages.len() != 1 {
        return Err(format!(
            "Expected one abandoned backup stage after process termination, found {}",
            stages.len()
        ));
    }
    let partial_asset = stages[0]
        .join("drawing.sketch.json.assets")
        .join("interrupt-probe.bin");
    let partial_metadata = fs::metadata(&partial_asset)
        .map_err(|error| format!("Interrupted partial asset is missing: {error}"))?;
    if partial_metadata.len() != 8 {
        return Err(format!(
            "Expected exactly 8 bytes written before termination, found {}",
            partial_metadata.len()
        ));
    }
    if crate::drawing_library::list_trash()?
        .iter()
        .any(|entry| entry.path == stages[0])
    {
        return Err("The fresh process listed an abandoned partial stage as a backup".into());
    }

    let backup = crate::drawing_library::trash_native_drawing(source_path)
        .map_err(|error| format!("Retrying trash backup failed: {error}"))?;
    if source_path.exists() || !backup.is_dir() {
        return Err("Retry did not install a complete backup before removing the source".into());
    }
    fs::create_dir_all(
        destination_path
            .parent()
            .ok_or("Restore destination must have a parent folder")?,
    )
    .map_err(|error| format!("Creating restore destination parent failed: {error}"))?;
    crate::drawing_library::restore_from_trash(&backup, destination_path)
        .map_err(|error| format!("Restoring retried backup failed: {error}"))?;
    let restored = Document::from_json(
        &fs::read_to_string(destination_path).map_err(|error| error.to_string())?,
    )?;
    if !restored.strokes.is_empty() || restored.layers != Document::default().layers {
        return Err("The retried backup restored different drawing content".into());
    }
    let restored_asset_root = crate::image_plane_storage::asset_root(destination_path)?;
    let restored_bytes = fs::read(restored_asset_root.join("interrupt-probe.bin"))
        .map_err(|error| format!("Restored asset is missing: {error}"))?;
    if restored_bytes.len() != 1024 * 1024 || restored_bytes.iter().any(|byte| *byte != 0x5a) {
        return Err("Retry/restore did not recover the complete managed asset bytes".into());
    }
    fs::remove_dir_all(&stages[0]).map_err(|error| error.to_string())?;

    let report = serde_json::json!({
        "passed": true,
        "checks": [
            "the parent terminated a child process after it wrote eight asset bytes into the real backup staging directory",
            "the source drawing remained intact and the partial stage survived process termination",
            "a fresh process ignored the partial stage in the trash list",
            "retry completed the production trash backup before removing the source drawing",
            "restoring the retried backup recovered the complete 1 MiB managed asset byte-for-byte"
        ],
        "partial_asset_bytes_before_termination": partial_metadata.len(),
        "restored_asset_bytes": restored_bytes.len(),
        "physical_hardware_tested": false
    });
    fs::write(
        report_path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot write interruption verification report: {error}"))?;
    Ok(())
}
