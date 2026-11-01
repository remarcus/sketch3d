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
