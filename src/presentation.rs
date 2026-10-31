//! Separate local canvas preferences; no artwork or document fields.
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};
pub const BLUE: [f32; 3] = [0.16, 0.44, 0.74];
pub const LIGHT: [f32; 3] = [0.93, 0.94, 0.96];
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectorTab {
    #[default]
    Properties,
    Layers,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPreferences {
    pub inspector_open: bool,
    pub inspector_width: f32,
    #[serde(default)]
    pub inspector_tab: InspectorTab,
}

impl Default for LayoutPreferences {
    fn default() -> Self {
        Self {
            inspector_open: true,
            inspector_width: 320.0,
            inspector_tab: InspectorTab::Properties,
        }
    }
}

impl LayoutPreferences {
    fn valid(&self) -> bool {
        self.inspector_width.is_finite() && (260.0..=720.0).contains(&self.inspector_width)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub background: [f32; 3],
    #[serde(default)]
    pub layout: LayoutPreferences,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 2,
            background: [0.018, 0.029, 0.046].map(linear_to_srgb),
            layout: LayoutPreferences::default(),
        }
    }
}
impl Preferences {
    fn valid(&self) -> bool {
        (self.version == 1 || self.version == 2)
            && self
                .background
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.layout.valid()
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        if file.metadata().map_err(|e| e.to_string())?.len() > 16_384 {
            return Err("Presentation preferences exceed the size limit".into());
        }
        let mut prefs: Self = serde_json::from_reader(file).map_err(|e| e.to_string())?;
        if !prefs.valid() {
            return Err("Unsupported or invalid presentation preferences".into());
        }
        // Version 1 stored only the background; missing layout fields use the
        // new defaults and are written as version 2 on the next preference change.
        prefs.version = 2;
        Ok(prefs)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if !self.valid() {
            return Err("Unsupported or invalid presentation preferences".into());
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_preference_roundtrips_without_persisting_focus_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        assert_eq!(Preferences::load(&path).unwrap(), Preferences::default());
        let prefs = Preferences {
            background: BLUE,
            ..Default::default()
        };
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), prefs);
        assert!(serde_json::to_value(&prefs)
            .unwrap()
            .get("artwork")
            .is_none());
    }
    #[test]
    fn invalid_future_and_oversized_preferences_are_retained() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        for bytes in [
            b"broken".to_vec(),
            br#"{"version":99,"background":[0,0,0]}"#.to_vec(),
            br#"{"version":1,"background":[-1,0,0]}"#.to_vec(),
            vec![b' '; 16_385],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(Preferences::load(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        assert!(Preferences {
            background: [f32::NAN, 0.0, 0.0],
            ..Default::default()
        }
        .save(&path)
        .is_err());
    }

    #[test]
    fn version_one_preferences_migrate_to_default_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        std::fs::write(&path, br#"{"version":1,"background":[0.1,0.2,0.3]}"#).unwrap();
        let prefs = Preferences::load(&path).unwrap();
        assert_eq!(prefs.version, 2);
        assert_eq!(prefs.background, [0.1, 0.2, 0.3]);
        assert_eq!(prefs.layout, LayoutPreferences::default());
    }

    #[test]
    fn workspace_layout_roundtrips_open_width_and_inspector_tab() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        let prefs = Preferences {
            layout: LayoutPreferences {
                inspector_open: false,
                inspector_width: 410.0,
                inspector_tab: InspectorTab::Layers,
            },
            ..Default::default()
        };
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), prefs);
    }

    #[test]
    fn invalid_layout_preferences_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        let mut prefs = Preferences::default();
        prefs.layout.inspector_width = f32::NAN;
        assert!(prefs.save(&path).is_err());
        std::fs::write(
            &path,
            br#"{"version":2,"background":[0.1,0.2,0.3],"layout":{"inspector_open":true,"inspector_width":80.0,"inspector_tab":"properties"}}"#,
        )
        .unwrap();
        assert!(Preferences::load(&path).is_err());
    }
    #[test]
    fn srgb_conversion_preserves_picker_values_and_original_clear_color() {
        for c in [0.0, 0.02, 0.16, 0.44, 0.74, 0.93, 1.0] {
            assert!((linear_to_srgb(srgb_to_linear(c)) - c).abs() < 1e-6);
        }
        for (actual, expected) in Preferences::default()
            .background
            .map(srgb_to_linear)
            .into_iter()
            .zip([0.018, 0.029, 0.046])
        {
            assert!((actual - expected).abs() < 1e-7);
        }
    }
}
