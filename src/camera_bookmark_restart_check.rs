//! Two-process verification that camera bookmarks and sequences reload through
//! the application's workspace-settings path.
use crate::{
    camera_bookmarks::{CameraBookmarkStore, SequenceFrame},
    geometry::{Camera, Projection},
};
use glam::Vec3;
use std::{fs, path::Path};

fn fixture() -> Result<CameraBookmarkStore, String> {
    let mut store = CameraBookmarkStore::default();
    let mut front = Camera {
        yaw: 1.2,
        pitch: 0.3,
        roll: -0.2,
        distance: 5.25,
        target: Vec3::new(2.0, -1.0, 0.5),
        ..Camera::default()
    };
    front.projection = Projection::Perspective;
    let front_id = store.add_bookmark("Restart front", &front)?;

    let side = Camera {
        projection: Projection::Orthographic,
        yaw: -2.1,
        pitch: -0.4,
        roll: 0.15,
        distance: 12.0,
        target: Vec3::new(-3.0, 4.0, 1.25),
    };
    let side_id = store.add_bookmark("Restart detail", &side)?;
    store.add_sequence(
        "Restart walkthrough",
        vec![
            SequenceFrame {
                bookmark_id: front_id,
                transition_seconds: 1.75,
            },
            SequenceFrame {
                bookmark_id: side_id,
                transition_seconds: 0.0,
            },
        ],
    )?;
    Ok(store)
}

fn storage_path() -> Result<std::path::PathBuf, String> {
    crate::presets::path()?
        .parent()
        .map(|parent| parent.join("camera-bookmarks.json"))
        .ok_or_else(|| "Workspace settings path has no parent folder".to_owned())
}

/// First process writes a representative camera library using its production
/// serializer and the same workspace path the app uses.
pub fn seed() -> Result<(), String> {
    let expected = fixture()?;
    expected.save(&storage_path()?)
}

/// The second process calls this with the store loaded by `SketchApp::new`.
pub fn verify_loaded(store: &CameraBookmarkStore, report_path: &Path) -> Result<(), String> {
    let expected = fixture()?;
    if store != &expected {
        return Err("Application startup did not reload the saved camera library exactly".into());
    }
    let path = storage_path()?;
    if !path.is_file() {
        return Err("Camera bookmark file disappeared before the second process loaded it".into());
    }
    let report = serde_json::json!({
        "passed": true,
        "checks": [
            "first process saved two full camera views and a timed ordered sequence through CameraBookmarkStore::save",
            "second application process loaded the exact saved library through SketchApp startup",
            "bookmark projection, framing, target, sequence order, and transition duration survived restart"
        ],
        "bookmark_count": store.bookmarks.len(),
        "sequence_count": store.sequences.len(),
        "physical_hardware_tested": false
    });
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(report_path, bytes).map_err(|error| error.to_string())
}
