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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub background: [f32; 3],
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            background: [0.018, 0.029, 0.046].map(linear_to_srgb),
        }
    }
}
impl Preferences {
    fn valid(&self) -> bool {
        self.version == 1
            && self
                .background
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
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
        let prefs: Self = serde_json::from_reader(file).map_err(|e| e.to_string())?;
        if !prefs.valid() {
            return Err("Unsupported or invalid presentation preferences".into());
        }
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
