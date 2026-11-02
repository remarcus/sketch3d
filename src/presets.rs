//! Versioned local brush configurations, independent of drawing documents.
use crate::{
    document::{Brush, BrushKind},
    texture::{TextureKind, TextureSettings},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const BUILTIN_PRESET_COUNT: usize = 10;
const MAX_PERSONAL_PRESETS: usize = 250;
const MAX_PRESETS: usize = MAX_PERSONAL_PRESETS + BUILTIN_PRESET_COUNT;
const FIRST_PERSONAL_ID: u64 = 100;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 64 * 1024;
const PACKAGE_FORMAT: &str = "spatial-sketch-brush";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: u64,
    pub name: String,
    pub brush: Brush,
    pub radius: f32,
    pub color: [f32; 3],
    pub stabilization: f32,
}

/// Portable, self-contained brush definition. Version 1 supports built-in
/// procedural brush and texture features; it contains no external assets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushPackage {
    pub format: String,
    pub version: u32,
    pub preset: PackagePreset,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackagePreset {
    pub name: String,
    pub brush: Brush,
    pub radius: f32,
    pub color: [f32; 3],
    pub stabilization: f32,
}

impl BrushPackage {
    pub fn from_preset(preset: &Preset) -> Self {
        Self {
            format: PACKAGE_FORMAT.into(),
            version: 1,
            preset: PackagePreset {
                name: preset.name.clone(),
                brush: preset.brush,
                radius: preset.radius,
                color: preset.color,
                stabilization: preset.stabilization,
            },
        }
    }

    pub fn to_json(&self) -> Result<String, String> {
        let preset = self.as_preset()?;
        let canonical = Self::from_preset(&preset);
        let text = serde_json::to_string_pretty(&canonical).map_err(|e| e.to_string())?;
        if text.len() as u64 > MAX_PACKAGE_BYTES {
            return Err("Brush package is too large".into());
        }
        Ok(text)
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        if text.len() as u64 > MAX_PACKAGE_BYTES {
            return Err("Brush package is too large".into());
        }
        let package: Self =
            serde_json::from_str(text).map_err(|e| format!("Cannot read brush package: {e}"))?;
        package.as_preset()?;
        Ok(package)
    }

    fn as_preset(&self) -> Result<Preset, String> {
        if self.format != PACKAGE_FORMAT {
            return Err("This is not a Sketch3D brush package".into());
        }
        if self.version != 1 {
            return Err("Unsupported brush package version".into());
        }
        let preset = Preset {
            id: FIRST_PERSONAL_ID,
            name: valid_name(&self.preset.name)?,
            brush: self.preset.brush,
            radius: self.preset.radius,
            color: self.preset.color,
            stabilization: self.preset.stabilization,
        };
        if !preset.is_valid() {
            return Err("Brush package contains invalid brush settings".into());
        }
        Ok(preset)
    }

    /// Imports by allocating a fresh local ID and resolving name collisions
    /// with an explicit, deterministic suffix.
    pub fn import_into(&self, library: &mut Library) -> Result<u64, String> {
        let preset = self.as_preset()?;
        let base = format!("{} (imported)", preset.name);
        let mut name = base.clone();
        let mut suffix = 2_u32;
        while library
            .presets
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            name = format!("{base} {suffix}");
            suffix = suffix
                .checked_add(1)
                .ok_or("No unique preset names available")?;
        }
        library.create(
            &name,
            preset.brush,
            preset.radius,
            preset.color,
            preset.stabilization,
        )
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path).map_err(|e| format!("Cannot read brush package: {e}"))?;
        if metadata.len() > MAX_PACKAGE_BYTES {
            return Err("Brush package is too large".into());
        }
        let text =
            fs::read_to_string(path).map_err(|e| format!("Cannot read brush package: {e}"))?;
        Self::from_json(&text)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = self.to_json()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create brush package folder: {e}"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        crate::atomic_file::replace(temporary, path)
            .map_err(|e| format!("Cannot save brush package: {e}"))?;
        Ok(())
    }
}
impl Preset {
    pub fn is_builtin(&self) -> bool {
        (1..=BUILTIN_PRESET_COUNT as u64).contains(&self.id)
    }
    fn is_valid(&self) -> bool {
        valid_name(&self.name).is_ok()
            && self.brush.is_valid()
            && self.radius.is_finite()
            && (0.001..=1.0).contains(&self.radius)
            && self
                .color
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.stabilization.is_finite()
            && (0.0..=1.0).contains(&self.stabilization)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub version: u32,
    pub presets: Vec<Preset>,
    pub next_id: u64,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            presets: defaults(),
            next_id: FIRST_PERSONAL_ID,
        }
    }
}
fn valid_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 64 || trimmed.chars().any(char::is_control) {
        return Err("Preset name must contain 1–64 characters without control characters".into());
    }
    Ok(trimmed.to_owned())
}
fn defaults() -> Vec<Preset> {
    [
        ("Round Ink", BrushKind::Round, TextureKind::Solid, 0.025),
        ("Ribbon", BrushKind::Ribbon, TextureKind::Solid, 0.055),
        ("Flat Marker", BrushKind::Chisel, TextureKind::Solid, 0.08),
        ("Chalk", BrushKind::Ribbon, TextureKind::Grain, 0.075),
        ("Dry Brush", BrushKind::Chisel, TextureKind::DryBrush, 0.09),
        (
            "Diamond Pattern",
            BrushKind::Ribbon,
            TextureKind::Pattern,
            0.09,
        ),
        ("Graphite", BrushKind::Ribbon, TextureKind::Graphite, 0.055),
        ("Charcoal", BrushKind::Chisel, TextureKind::Charcoal, 0.09),
        ("Stipple", BrushKind::Round, TextureKind::Solid, 0.035),
        ("Spray", BrushKind::Round, TextureKind::Solid, 0.024),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, kind, texture, radius))| {
        let mut brush = Brush {
            kind,
            texture: TextureSettings {
                kind: texture,
                ..Default::default()
            },
            ..Default::default()
        };
        if name == "Stipple" {
            brush.dabs.enabled = true;
            brush.dabs.spacing = 2.4;
            brush.dabs.size_variation = 0.08;
            brush.dabs.pressure_size = crate::document::PressureMapping {
                enabled: true,
                minimum: 0.28,
                curve: 1.15,
            };
            brush.dynamics.width.enabled = false;
            brush.dynamics.opacity = crate::document::PressureMapping {
                enabled: true,
                minimum: 0.06,
                curve: 1.2,
            };
            brush.opacity = 0.86;
        } else if name == "Spray" {
            brush.dabs.enabled = true;
            brush.dabs.spacing = 0.7;
            brush.dabs.scatter = 2.8;
            brush.dabs.size_variation = 0.62;
            brush.dabs.angle_variation_degrees = 180.0;
            brush.dabs.pressure_size = crate::document::PressureMapping {
                enabled: true,
                minimum: 0.08,
                curve: 1.35,
            };
            brush.dynamics.width.enabled = false;
            brush.dynamics.opacity = crate::document::PressureMapping {
                enabled: true,
                minimum: 0.025,
                curve: 1.1,
            };
            brush.opacity = 0.62;
        }
        let color = match texture {
            TextureKind::Graphite => {
                brush.dynamics.opacity = crate::document::PressureMapping {
                    enabled: true,
                    minimum: 0.04,
                    curve: 1.5,
                };
                brush.dynamics.tilt_orientation = true;
                brush.dynamics.tilt_shading = true;
                brush.dynamics.tilt_shading_scale = 3.5;
                brush.dynamics.manual_shading_scale = 1.45;
                brush.opacity = 0.88;
                [0.23, 0.22, 0.2]
            }
            TextureKind::Charcoal => {
                brush.dynamics.opacity = crate::document::PressureMapping {
                    enabled: true,
                    minimum: 0.03,
                    curve: 1.2,
                };
                brush.dynamics.tilt_orientation = true;
                brush.dynamics.tilt_shading = true;
                brush.dynamics.tilt_shading_scale = 5.0;
                brush.dynamics.manual_shading_scale = 2.1;
                brush.opacity = 0.92;
                [0.14, 0.13, 0.12]
            }
            _ => match name {
                "Stipple" => [0.12, 0.11, 0.1],
                "Spray" => [0.24, 0.25, 0.29],
                _ => [0.17, 0.82, 0.74],
            },
        };
        Preset {
            id: i as u64 + 1,
            name: name.into(),
            brush,
            radius,
            color,
            stabilization: 0.35,
        }
    })
    .collect()
}

impl Library {
    pub fn get(&self, id: u64) -> Option<&Preset> {
        self.presets.iter().find(|p| p.id == id)
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported brush library version".into());
        }
        if self.presets.len() > MAX_PRESETS {
            return Err(format!("Brush library exceeds {MAX_PRESETS} presets"));
        }
        if self.presets.iter().filter(|p| !p.is_builtin()).count() > MAX_PERSONAL_PRESETS {
            return Err(format!(
                "Brush library exceeds {MAX_PERSONAL_PRESETS} personal presets; {BUILTIN_PRESET_COUNT} slots are reserved for defaults"
            ));
        }
        if self.next_id < FIRST_PERSONAL_ID || self.next_id == u64::MAX {
            return Err("Invalid next preset ID".into());
        }
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        for preset in &self.presets {
            if !preset.is_valid() || preset.name != preset.name.trim() {
                return Err("Invalid preset settings or name".into());
            }
            if !preset.is_builtin() && !(FIRST_PERSONAL_ID..self.next_id).contains(&preset.id) {
                return Err("Invalid preset ID".into());
            }
            if !ids.insert(preset.id) {
                return Err("Duplicate preset ID".into());
            }
            if !names.insert(preset.name.to_lowercase()) {
                return Err("Preset names must be unique".into());
            }
        }
        Ok(())
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err("Brush library file is too large".into());
        }
        let library: Self =
            serde_json::from_str(text).map_err(|e| format!("Cannot read brush library: {e}"))?;
        library.validate()?;
        Ok(library)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path).map_err(|e| format!("Cannot read brush library: {e}"))?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err("Brush library file is too large".into());
        }
        Self::from_json(
            &fs::read_to_string(path).map_err(|e| format!("Cannot read brush library: {e}"))?,
        )
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("Brush library file is too large".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create brush library folder: {e}"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temporary.write_all(&bytes).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        crate::atomic_file::replace(temporary, path)
            .map_err(|e| format!("Cannot save brush library: {e}"))?;
        Ok(())
    }
    pub fn create(
        &mut self,
        name: &str,
        brush: Brush,
        radius: f32,
        color: [f32; 3],
        stabilization: f32,
    ) -> Result<u64, String> {
        let name = valid_name(name)?;
        let id = self.next_id;
        let mut changed = self.clone();
        changed.next_id = id.checked_add(1).ok_or("No preset IDs available")?;
        changed.presets.push(Preset {
            id,
            name,
            brush,
            radius,
            color,
            stabilization,
        });
        changed.validate()?;
        *self = changed;
        Ok(id)
    }
    pub fn duplicate(&mut self, id: u64, name: &str) -> Result<u64, String> {
        let preset = self.get(id).cloned().ok_or("Preset no longer exists")?;
        self.create(
            name,
            preset.brush,
            preset.radius,
            preset.color,
            preset.stabilization,
        )
    }
    pub fn rename(&mut self, id: u64, name: &str) -> Result<(), String> {
        let name = valid_name(name)?;
        let mut changed = self.clone();
        changed
            .presets
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("Preset no longer exists")?
            .name = name;
        changed.validate()?;
        *self = changed;
        Ok(())
    }
    pub fn update(
        &mut self,
        id: u64,
        brush: Brush,
        radius: f32,
        color: [f32; 3],
        stabilization: f32,
    ) -> Result<(), String> {
        let mut changed = self.clone();
        let preset = changed
            .presets
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("Preset no longer exists")?;
        preset.brush = brush;
        preset.radius = radius;
        preset.color = color;
        preset.stabilization = stabilization;
        changed.validate()?;
        *self = changed;
        Ok(())
    }
    pub fn delete(&mut self, id: u64) -> Result<(), String> {
        if self.get(id).is_none() {
            return Err("Preset no longer exists".into());
        }
        let mut changed = self.clone();
        changed.presets.retain(|p| p.id != id);
        changed.validate()?;
        *self = changed;
        Ok(())
    }
    /// Built-in IDs are restored without deleting any personal preset. Resolve
    /// a conflicting personal name first rather than silently renaming it.
    pub fn restore_defaults(&mut self) -> Result<(), String> {
        let mut changed = self.clone();
        changed.presets.retain(|p| !p.is_builtin());
        changed.presets.splice(0..0, defaults());
        changed.validate()?;
        *self = changed;
        Ok(())
    }
}

pub fn path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|s| !s.is_empty())
                .map(|p| PathBuf::from(p).join("AppData/Local"))
        });
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|p| PathBuf::from(p).join(".config"))
        });
    base.map(|p| p.join("SpatialSketch/brushes.json"))
        .ok_or_else(|| "Local application-data folder is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_presets_survive_atomic_save_restart_and_crud() {
        let mut library = Library::default();
        let id = library.duplicate(4, " My Chalk ").unwrap();
        assert_eq!(library.get(id).unwrap().name, "My Chalk");
        let mut brush = library.get(id).unwrap().brush;
        brush.texture.seed = 123;
        brush.dynamics.taper_end = 0.6;
        library
            .update(id, brush, 0.1, [0.2, 0.3, 0.4], 0.7)
            .unwrap();
        library.rename(id, "Warm Chalk").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/brushes.json");
        library.save(&path).unwrap();
        assert_eq!(Library::load(&path).unwrap(), library);
        library.rename(id, "Renamed Chalk").unwrap();
        library.save(&path).unwrap();
        let mut restarted = Library::load(&path).unwrap();
        assert_eq!(restarted, library);
        restarted.delete(id).unwrap();
        assert!(restarted.get(id).is_none());
        assert!(restarted.delete(id).is_err());
    }
    #[test]
    fn builtins_restore_without_modifying_personal_resolved_settings() {
        let mut library = Library::default();
        let personal = library.duplicate(5, "Personal Dry Brush").unwrap();
        let resolved = library.get(personal).unwrap().clone();
        library.rename(5, "Modified Builtin").unwrap();
        library.delete(4).unwrap();
        library.restore_defaults().unwrap();
        assert_eq!(library.get(4).unwrap().name, "Chalk");
        assert_eq!(library.get(5).unwrap().name, "Dry Brush");
        assert_eq!(library.get(personal).unwrap(), &resolved);
        // Drawing stores a value copy, independent of later preset changes.
        let ink_brush = resolved.brush;
        let mut modified = resolved.brush;
        modified.texture.strength = 0.1;
        library
            .update(personal, modified, 0.04, [1.0; 3], 0.1)
            .unwrap();
        assert_ne!(library.get(personal).unwrap().brush, ink_brush);
        assert_eq!(ink_brush, resolved.brush);
    }
    #[test]
    fn invalid_changes_are_transactional_and_names_are_unique() {
        let mut library = Library::default();
        let original = library.clone();
        for name in ["", "  ", "round ink", "bad\nname"] {
            assert!(library.duplicate(1, name).is_err());
            assert_eq!(library, original);
        }
        assert!(library.rename(2, "ROUND INK").is_err());
        assert_eq!(library, original);
        assert!(library
            .update(1, Brush::default(), f32::NAN, [0.2; 3], 0.2)
            .is_err());
        assert_eq!(library, original);
        library.delete(4).unwrap();
        library.duplicate(1, "Chalk").unwrap();
        let before_restore = library.clone();
        assert!(library.restore_defaults().is_err());
        assert_eq!(library, before_restore);
    }
    #[test]
    fn corrupt_unsupported_and_invalid_library_files_remain_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brushes.json");
        for text in ["{broken", "{\"version\":99,\"presets\":[],\"next_id\":100}"] {
            fs::write(&path, text).unwrap();
            assert!(Library::load(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
        let mut value = serde_json::to_value(Library::default()).unwrap();
        value["presets"][0]["brush"]["texture"]["scale"] = serde_json::json!(0.0);
        assert!(Library::from_json(&value.to_string()).is_err());
        value = serde_json::to_value(Library::default()).unwrap();
        value["next_id"] = serde_json::json!(3);
        assert!(Library::from_json(&value.to_string()).is_err());
        value = serde_json::to_value(Library::default()).unwrap();
        value["presets"][1]["id"] = serde_json::json!(1);
        assert!(Library::from_json(&value.to_string()).is_err());
    }
    #[test]
    fn capacity_keeps_room_to_restore_default_brushes() {
        let mut library = Library::default();
        for index in 0..250 {
            library.duplicate(1, &format!("Personal {index}")).unwrap();
        }
        let full = library.clone();
        assert!(library.duplicate(1, "Overflow").is_err());
        assert_eq!(library, full);
        library.delete(6).unwrap();
        assert!(library.duplicate(1, "Still overflow").is_err());
        library.restore_defaults().unwrap();
        assert_eq!(library.presets.len(), 260);
    }

    #[test]
    fn brush_package_roundtrip_allocates_local_id_and_resolves_name_collisions() {
        let mut source = Library::default();
        let id = source.duplicate(4, "Custom Chalk").unwrap();
        let package = BrushPackage::from_preset(source.get(id).unwrap());
        let json = package.to_json().unwrap();
        let decoded = BrushPackage::from_json(&json).unwrap();
        let mut destination = Library::default();
        let imported = decoded.import_into(&mut destination).unwrap();
        assert_eq!(imported, 100);
        assert_eq!(
            destination.get(imported).unwrap().name,
            "Custom Chalk (imported)"
        );
        assert_eq!(
            destination.get(imported).unwrap().brush,
            source.get(id).unwrap().brush
        );
        let imported_again = decoded.import_into(&mut destination).unwrap();
        assert_eq!(
            destination.get(imported_again).unwrap().name,
            "Custom Chalk (imported) 2"
        );
    }

    #[test]
    fn brush_package_file_roundtrip_is_atomic_and_portable() {
        let package = BrushPackage::from_preset(&Library::default().presets[0]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/Round Ink.brush.json");
        package.save(&path).unwrap();
        assert_eq!(BrushPackage::load(&path).unwrap(), package);
    }

    #[test]
    fn brush_package_rejects_wrong_format_version_invalid_settings_and_oversize() {
        let package = BrushPackage::from_preset(&Library::default().presets[0]);
        let mut value = serde_json::to_value(&package).unwrap();
        value["format"] = serde_json::json!("other-brush-format");
        assert!(BrushPackage::from_json(&value.to_string()).is_err());
        value = serde_json::to_value(&package).unwrap();
        value["version"] = serde_json::json!(2);
        assert!(BrushPackage::from_json(&value.to_string()).is_err());
        value = serde_json::to_value(&package).unwrap();
        value["preset"]["radius"] = serde_json::json!(f32::NAN);
        assert!(BrushPackage::from_json(&value.to_string()).is_err());
        assert!(BrushPackage::from_json(&" ".repeat(MAX_PACKAGE_BYTES as usize + 1)).is_err());
    }
}
