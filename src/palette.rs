//! Named artwork colors saved separately from brush presets and document data.
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

pub const MAX_SWATCHES: usize = 32;
pub const MAX_NAME_CHARS: usize = 32;
const MAX_FILE_BYTES: u64 = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Swatch {
    pub name: String,
    pub color: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub version: u32,
    pub swatches: Vec<Swatch>,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            version: 1,
            swatches: Vec::new(),
        }
    }
}

impl Palette {
    fn valid(&self) -> bool {
        self.version == 1
            && self.swatches.len() <= MAX_SWATCHES
            && self.swatches.iter().enumerate().all(|(index, swatch)| {
                let name = swatch.name.trim();
                !name.is_empty()
                    && name.chars().count() <= MAX_NAME_CHARS
                    && swatch
                        .color
                        .iter()
                        .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
                    && self.swatches[..index]
                        .iter()
                        .all(|other| !other.name.eq_ignore_ascii_case(name))
            })
    }

    pub fn add(&mut self, name: &str, color: [f32; 3]) -> Result<usize, String> {
        let name = name.trim();
        if self.swatches.len() >= MAX_SWATCHES {
            return Err(format!("Palette is limited to {MAX_SWATCHES} swatches"));
        }
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(format!(
                "Swatch names must contain 1–{MAX_NAME_CHARS} characters"
            ));
        }
        if !color
            .iter()
            .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
        {
            return Err("Swatch colors must be finite RGB values from 0 to 1".into());
        }
        if self
            .swatches
            .iter()
            .any(|swatch| swatch.name.eq_ignore_ascii_case(name))
        {
            return Err("Swatch names must be unique".into());
        }
        self.swatches.push(Swatch {
            name: name.into(),
            color,
        });
        Ok(self.swatches.len() - 1)
    }

    pub fn rename(&mut self, index: usize, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(format!(
                "Swatch names must contain 1–{MAX_NAME_CHARS} characters"
            ));
        }
        if self
            .swatches
            .iter()
            .enumerate()
            .any(|(i, swatch)| i != index && swatch.name.eq_ignore_ascii_case(name))
        {
            return Err("Swatch names must be unique".into());
        }
        self.swatches
            .get_mut(index)
            .ok_or("Choose a swatch to rename")?
            .name = name.into();
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        if index >= self.swatches.len() {
            return Err("Choose a swatch to remove".into());
        }
        self.swatches.remove(index);
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_FILE_BYTES {
            return Err("Palette file exceeds the 32 KB size limit".into());
        }
        let palette: Self = serde_json::from_reader(file).map_err(|error| error.to_string())?;
        if !palette.valid() {
            return Err("Unsupported or invalid palette data".into());
        }
        Ok(palette)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if !self.valid() {
            return Err("Unsupported or invalid palette data".into());
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut temp =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        temp.write_all(&serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        temp.as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        crate::atomic_file::replace(temp, path).map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_swatch_crud_and_atomic_restart_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/palette.json");
        let mut palette = Palette::default();
        let index = palette.add("Awning Blue", [0.1, 0.3, 0.8]).unwrap();
        palette.rename(index, "Trim Blue").unwrap();
        palette.save(&path).unwrap();
        assert_eq!(Palette::load(&path).unwrap(), palette);
        palette.remove(index).unwrap();
        assert!(palette.swatches.is_empty());
    }

    #[test]
    fn palette_rejects_bad_names_colors_duplicates_and_oversized_files() {
        let mut palette = Palette::default();
        assert!(palette.add("  ", [0.0; 3]).is_err());
        assert!(palette.add("Bad", [f32::NAN, 0.0, 0.0]).is_err());
        palette.add("Trim", [0.5; 3]).unwrap();
        assert!(palette.add("trim", [0.2; 3]).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("palette.json");
        std::fs::write(&path, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(Palette::load(&path).is_err());
    }
}
