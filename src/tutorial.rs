//! Local onboarding preferences and the guided-practice lesson content.
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub seen: bool,
    pub show_on_startup: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            seen: false,
            show_on_startup: false,
        }
    }
}
impl Preferences {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 16_384 {
            return Err("Tutorial preferences exceed the size limit".into());
        }
        let preferences: Self = serde_json::from_reader(file).map_err(|e| e.to_string())?;
        if preferences.version != 1 {
            return Err("Unsupported tutorial preference version".into());
        }
        Ok(preferences)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported tutorial preference version".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct Lesson {
    pub title: &'static str,
    pub target: &'static str,
    pub text: &'static str,
    pub practice: &'static str,
}
pub const LESSONS: &[Lesson] = &[
    Lesson { title: "Make your first mark", target: "Brush", text: "B selects Brush. Draw inside the canvas on the highlighted plane. A mouse uses fixed pressure. With a tablet, enable Windows Ink in its driver and map it to your intended display.", practice: "Draw two strokes, then try Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z. E erases a whole stroke." },
    Lesson { title: "Pressure and pen feedback", target: "Input lab", text: "The right-hand Input lab reports actual pen samples and pressure. No samples means pressure is not verified. Tilt is optional; missing tilt uses the manual brush orientation. Neither a screen nor touch is required.", practice: "With a pen, vary pressure and watch the graph. With a mouse, continue using fixed width. Physical Wacom/Huion behavior still needs device testing." },
    Lesson { title: "Explore the drawing", target: "Navigation", text: "Hold Space and drag to orbit, Space+Shift to pan, or Space+Alt to zoom. Mouse alternatives: right drag, middle drag, and wheel. A reported pen barrel button can orbit too.", practice: "Orbit your marks, pan, then zoom. Press F to face the active guide again." },
    Lesson { title: "Shape, texture, and presets", target: "Brush library", text: "Choose Round, Ribbon, or Flat/chisel under Brush shape. Grain, Dry brush, and Pattern are built-in masks. Brush library presets combine settings; existing marks keep their resolved appearance.", practice: "Try a ribbon and a textured stroke. Choose a library entry and Use preset. Personal preset changes affect your real local library." },
    Lesson { title: "Smooth strokes and tapered ends", target: "Brush", text: "Stabilization smooths new input. Pressure width/thickness, response, taper, and optional tilt orientation change new strokes. Brush radius is measured in world units, so zoom changes its apparent size.", practice: "Compare low/high stabilization and start/end taper. No pressure or tilt hardware is required to try taper." },
    Lesson { title: "Draw on a different surface", target: "Drawing guide", text: "Use Front, Side, Ground, or a rotated plane. Shift+F makes a plane from the view; F aligns the camera. Cylinder and Revolved profile constrain ink to curved guide sides. Guides are temporary workspace settings.", practice: "Try another plane, then a cylinder. Draw on its wireframe and orbit. Missed or edge-on surfaces do not accept strokes." },
    Lesson { title: "Select and transform", target: "Select", text: "V selects; Shift-click adds or removes a hit. G moves, R rotates, S scales. Drag visible handles or use numeric values. Escape cancels an active transform. Ctrl/Cmd+D duplicates selected strokes.", practice: "Select a mark, duplicate it, and move the copy. Undo restores edits. B returns to drawing." },
    Lesson { title: "Organize layers and groups", target: "Drawing layers", text: "Layers control visibility and locking. In Select mode, Group (Ctrl/Cmd+G) combines selected strokes on one layer. Clicking a member selects its group; Select individual strokes edits members. Ctrl/Cmd+Shift+G ungroups.", practice: "Group two marks, duplicate the group, then undo. Try hiding or locking a layer: protected ink cannot be edited." },
    Lesson { title: "Keep and share your work", target: "Save / Export", text: "Save keeps editable .sketch.json data; GLB Export makes mesh artwork for other 3D apps. Recovery protects completed edits automatically, separately from deliberate saves. Original artwork stays suspended during practice; practice itself has no automatic recovery.", practice: "Use Save as if you want to keep this practice drawing. Finish returns to your original drawing and history. Unsaved practice prompts before exit. You can replay this tutorial from Help." },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_launch_skip_replay_preference_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        let mut prefs = Preferences::load(&path).unwrap();
        assert!(!prefs.seen && !prefs.show_on_startup);
        prefs.seen = true;
        prefs.show_on_startup = false;
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), prefs);
        prefs.show_on_startup = true;
        prefs.save(&path).unwrap();
        assert!(Preferences::load(&path).unwrap().show_on_startup);
    }
    #[test]
    fn invalid_preferences_are_not_overwritten_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        for bytes in [
            b"broken".to_vec(),
            br#"{"version":99,"seen":true,"show_on_startup":false}"#.to_vec(),
            vec![b' '; 16_385],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(Preferences::load(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}
