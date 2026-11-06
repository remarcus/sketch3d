//! Separate local canvas preferences; no artwork or document fields.
use eframe::egui;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    /// Keep the appearance used before theme preferences were introduced.
    #[default]
    Dark,
    Light,
    System,
}

impl UiTheme {
    pub fn egui(self) -> egui::ThemePreference {
        match self {
            Self::Dark => egui::ThemePreference::Dark,
            Self::Light => egui::ThemePreference::Light,
            Self::System => egui::ThemePreference::System,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectorTab {
    #[default]
    Properties,
    Layers,
    References,
    ImagePlanes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundImageFit {
    #[default]
    Fit,
    Fill,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackgroundImageSettings {
    pub reference_id: u64,
    pub fit: BackgroundImageFit,
    pub alignment: [f32; 2],
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostEffects {
    /// Bright-pass bloom contribution. Zero disables glow.
    pub glow: f32,
    /// Focus distance in world units; zero disables depth of field.
    pub focus_distance: f32,
    /// Blur radius in pixels, bounded to keep GPU cost predictable.
    pub depth_of_field: f32,
    /// Pixel block width in pixels; 1 disables pixelation.
    pub pixel_size: f32,
}

impl Default for PostEffects {
    fn default() -> Self {
        Self {
            glow: 0.0,
            focus_distance: 0.0,
            depth_of_field: 0.0,
            pixel_size: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPreferences {
    pub inspector_open: bool,
    #[serde(default)]
    pub inspector_collapsed: bool,
    pub inspector_width: f32,
    #[serde(default)]
    pub inspector_tab: InspectorTab,
    #[serde(default = "default_canvas_strip_visible")]
    pub canvas_strip_visible: bool,
}

fn default_canvas_strip_visible() -> bool {
    true
}

impl Default for LayoutPreferences {
    fn default() -> Self {
        Self {
            inspector_open: true,
            inspector_collapsed: false,
            inspector_width: 320.0,
            inspector_tab: InspectorTab::Properties,
            canvas_strip_visible: true,
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
    #[serde(default)]
    pub ui_theme: UiTheme,
    pub background: [f32; 3],
    #[serde(default)]
    pub grain: f32,
    #[serde(default)]
    pub toon_shading: bool,
    #[serde(default)]
    pub post_effects: PostEffects,
    #[serde(default)]
    pub background_image: Option<BackgroundImageSettings>,
    #[serde(default)]
    pub layout: LayoutPreferences,
    #[serde(default)]
    pub pen_barrel_erases: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 7,
            ui_theme: UiTheme::Dark,
            background: [0.018, 0.029, 0.046].map(linear_to_srgb),
            grain: 0.0,
            toon_shading: false,
            post_effects: PostEffects::default(),
            background_image: None,
            layout: LayoutPreferences::default(),
            pen_barrel_erases: false,
        }
    }
}
impl Preferences {
    fn valid(&self) -> bool {
        (1..=7).contains(&self.version)
            && self
                .background
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.grain.is_finite()
            && (0.0..=0.12).contains(&self.grain)
            && self.post_effects.glow.is_finite()
            && (0.0..=1.0).contains(&self.post_effects.glow)
            && self.post_effects.focus_distance.is_finite()
            && (0.0..=1000.0).contains(&self.post_effects.focus_distance)
            && self.post_effects.depth_of_field.is_finite()
            && (0.0..=16.0).contains(&self.post_effects.depth_of_field)
            && self.post_effects.pixel_size.is_finite()
            && (1.0..=16.0).contains(&self.post_effects.pixel_size)
            && self.layout.valid()
            && self.background_image.is_none_or(|image| {
                image.reference_id != 0
                    && image.reference_id != u64::MAX
                    && image
                        .alignment
                        .into_iter()
                        .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
                    && image.opacity.is_finite()
                    && (0.0..=1.0).contains(&image.opacity)
            })
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
        // Older versions have no appearance mode; it defaults to Dark.
        prefs.version = 7;
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
        crate::atomic_file::replace(temp, path).map_err(|e| e.to_string())?;
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
            background_image: Some(BackgroundImageSettings {
                reference_id: 19,
                fit: BackgroundImageFit::Fill,
                alignment: [0.2, 0.8],
                opacity: 0.63,
            }),
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
        assert_eq!(prefs.version, 7);
        assert_eq!(prefs.ui_theme, UiTheme::Dark);
        assert_eq!(prefs.background, [0.1, 0.2, 0.3]);
        assert_eq!(prefs.layout, LayoutPreferences::default());
    }

    #[test]
    fn workspace_layout_roundtrips_expanded_collapsed_and_hidden_states() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        for (inspector_open, inspector_collapsed) in [(true, false), (true, true), (false, false)] {
            for inspector_tab in [
                InspectorTab::Properties,
                InspectorTab::Layers,
                InspectorTab::References,
                InspectorTab::ImagePlanes,
            ] {
                let prefs = Preferences {
                    layout: LayoutPreferences {
                        inspector_open,
                        inspector_collapsed,
                        inspector_width: 410.0,
                        inspector_tab,
                        canvas_strip_visible: false,
                    },
                    ..Default::default()
                };
                prefs.save(&path).unwrap();
                assert_eq!(Preferences::load(&path).unwrap(), prefs);
            }
        }
    }

    #[test]
    fn older_workspace_layout_defaults_canvas_strip_to_visible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        std::fs::write(
            &path,
            br#"{"version":7,"background":[0.1,0.2,0.3],"layout":{"inspector_open":true,"inspector_collapsed":false,"inspector_width":320.0,"inspector_tab":"properties"}}"#,
        )
        .unwrap();
        assert!(
            Preferences::load(&path)
                .unwrap()
                .layout
                .canvas_strip_visible
        );
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

    #[test]
    fn post_effect_preferences_default_off_and_roundtrip_their_bounded_ranges() {
        assert_eq!(Preferences::default().post_effects, PostEffects::default());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presentation.json");
        let prefs = Preferences {
            grain: 0.12,
            toon_shading: true,
            post_effects: PostEffects {
                glow: 1.0,
                focus_distance: 1000.0,
                depth_of_field: 16.0,
                pixel_size: 16.0,
            },
            ..Preferences::default()
        };
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), prefs);

        let mut invalid = prefs.clone();
        invalid.post_effects.glow = 1.01;
        assert!(invalid.save(&path).is_err());
        invalid = prefs.clone();
        invalid.post_effects.focus_distance = -0.01;
        assert!(invalid.save(&path).is_err());
        invalid = prefs.clone();
        invalid.post_effects.depth_of_field = 16.01;
        assert!(invalid.save(&path).is_err());
        invalid = prefs.clone();
        invalid.post_effects.pixel_size = 0.5;
        assert!(invalid.save(&path).is_err());
    }
}
