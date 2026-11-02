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

const BUILTIN_PRESET_COUNT: usize = 19;
const MAX_PERSONAL_PRESETS: usize = 250;
const MAX_PRESETS: usize = MAX_PERSONAL_PRESETS + BUILTIN_PRESET_COUNT;
const FIRST_PERSONAL_ID: u64 = 100;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024;
const PACKAGE_FORMAT: &str = "spatial-sketch-brush";
const LIBRARY_VERSION: u32 = 5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: u64,
    pub name: String,
    pub brush: Brush,
    pub radius: f32,
    pub color: [f32; 3],
    pub stabilization: f32,
}

/// Portable, self-contained brush definition. Version 1 is settings-only;
/// version 2 can carry a bounded PNG tip without a source path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushPackage {
    pub format: String,
    pub version: u32,
    pub preset: PackagePreset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip_asset: Option<crate::brush_tip::BrushTipAsset>,
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
    pub fn from_preset(
        preset: &Preset,
        tip_asset: Option<crate::brush_tip::BrushTipAsset>,
    ) -> Self {
        Self {
            format: PACKAGE_FORMAT.into(),
            version: if tip_asset.is_some() { 2 } else { 1 },
            preset: PackagePreset {
                name: preset.name.clone(),
                brush: preset.brush,
                radius: preset.radius,
                color: preset.color,
                stabilization: preset.stabilization,
            },
            tip_asset,
        }
    }

    pub fn to_json(&self) -> Result<String, String> {
        let preset = self.as_preset()?;
        let canonical = Self::from_preset(&preset, self.tip_asset.clone());
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
        let mut value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("Cannot read brush package: {e}"))?;
        // Early v2 packages embedded tips before their resolved aspect ratio
        // field existed. Recover that value from the self-contained PNG.
        if value.get("version").and_then(serde_json::Value::as_u64) == Some(2) {
            let size = value
                .get("tip_asset")
                .and_then(|asset| asset.get("size"))
                .and_then(serde_json::Value::as_array)
                .filter(|size| size.len() == 2)
                .and_then(|size| Some([size[0].as_u64()?, size[1].as_u64()?]));
            if let Some([width, height]) = size {
                if let Some(brush) = value
                    .get_mut("preset")
                    .and_then(|preset| preset.get_mut("brush"))
                    .and_then(serde_json::Value::as_object_mut)
                {
                    if !brush.contains_key("tip_aspect") {
                        brush.insert(
                            "tip_aspect".into(),
                            serde_json::json!(width as f32 / height as f32),
                        );
                        brush.insert(
                            "texture".into(),
                            serde_json::to_value(TextureSettings::default())
                                .map_err(|error| error.to_string())?,
                        );
                        let dabs = brush.entry("dabs").or_insert_with(|| serde_json::json!({}));
                        dabs["enabled"] = serde_json::json!(true);
                        dabs["spacing"] = serde_json::json!(0.55);
                        let dynamics = brush
                            .entry("dynamics")
                            .or_insert_with(|| serde_json::json!({}));
                        dynamics["tilt_orientation"] = serde_json::json!(false);
                        dynamics["tilt_shading"] = serde_json::json!(false);
                    }
                }
            }
        }
        let package: Self =
            serde_json::from_value(value).map_err(|e| format!("Cannot read brush package: {e}"))?;
        package.as_preset()?;
        Ok(package)
    }

    fn as_preset(&self) -> Result<Preset, String> {
        if self.format != PACKAGE_FORMAT {
            return Err("This is not a Sketch3D brush package".into());
        }
        if !matches!(self.version, 1 | 2) {
            return Err("Unsupported brush package version".into());
        }
        match (
            &self.tip_asset,
            self.version,
            self.preset.brush.tip_asset_id,
        ) {
            (None, 1, 0) | (None, 2, 0) => {}
            (Some(asset), 2, id) if id == asset.id => {
                asset.validate()?;
                if !self.preset.brush.dabs.enabled
                    || self.preset.brush.texture.kind != TextureKind::Solid
                    || self.preset.brush.dynamics.tilt_orientation
                    || self.preset.brush.dynamics.tilt_shading
                    || (self.preset.brush.tip_aspect - asset.size[0] as f32 / asset.size[1] as f32)
                        .abs()
                        > 1e-4
                {
                    return Err(
                        "Brush package image-tip settings are invalid or do not match its asset"
                            .into(),
                    );
                }
            }
            _ => return Err("Brush package tip asset reference is missing or inconsistent".into()),
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
        let mut preset = self.as_preset()?;
        let mut changed = library.clone();
        if let Some(asset) = &self.tip_asset {
            changed.add_tip_asset(asset.clone())?;
            preset.brush.tip_asset_id = asset.id;
        }
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
        let id = changed.create(
            &name,
            preset.brush,
            preset.radius,
            preset.color,
            preset.stabilization,
        )?;
        *library = changed;
        Ok(id)
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
    #[serde(default)]
    pub tip_assets: Vec<crate::brush_tip::BrushTipAsset>,
}
impl Default for Library {
    fn default() -> Self {
        let tip_assets = default_tip_assets();
        Self {
            version: LIBRARY_VERSION,
            presets: defaults(&tip_assets),
            next_id: FIRST_PERSONAL_ID,
            tip_assets,
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
fn defaults(tip_assets: &[crate::brush_tip::BrushTipAsset]) -> Vec<Preset> {
    let mut presets: Vec<Preset> = [
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
    .collect();
    for (index, asset) in tip_assets.iter().enumerate() {
        let (name, radius, color, angle, variation) = match index {
            0 => ("Leaf Scatter", 0.07, [0.58, 0.95, 0.52], 24.0, 34.0),
            1 => ("Grass Scatter", 0.055, [0.42, 0.88, 0.35], 22.0, 28.0),
            _ => ("Star Stamp", 0.06, [1.0, 0.77, 0.24], 0.0, 24.0),
        };
        let mut brush = Brush {
            tip_asset_id: asset.id,
            tip_aspect: asset.size[0] as f32 / asset.size[1] as f32,
            ..Brush::default()
        };
        brush.dabs.enabled = true;
        brush.dabs.spacing = 0.9;
        brush.dabs.scatter = if index == 2 { 0.0 } else { 1.4 };
        brush.dabs.size_variation = 0.2;
        brush.angle_degrees = angle;
        brush.dabs.pressure_size = crate::document::PressureMapping {
            enabled: true,
            minimum: 0.48,
            curve: 1.1,
        };
        brush.dynamics.width.enabled = false;
        brush.dynamics.opacity = crate::document::PressureMapping {
            enabled: true,
            minimum: 0.42,
            curve: 1.0,
        };
        brush.dabs.angle_variation_degrees = variation;
        presets.push(Preset {
            id: 11 + index as u64,
            name: name.into(),
            brush,
            radius,
            color,
            stabilization: 0.35,
        });
    }
    for (index, (name, count, spread, thickness, tilt_spread, radius, color)) in [
        ("Rake", 14, 2.0, 0.72, 0.25, 0.075, [0.22, 0.39, 0.72]),
        (
            "Bristle Fan",
            6,
            0.85,
            0.95,
            0.75,
            0.085,
            [0.72, 0.29, 0.16],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut brush = Brush {
            kind: BrushKind::Ribbon,
            ..Brush::default()
        };
        brush.strands = crate::document::StrandSettings {
            enabled: true,
            count,
            spread,
            thickness,
            tilt_spread,
        };
        brush.dynamics.tilt_orientation = true;
        brush.dynamics.width.minimum = 0.28;
        brush.dynamics.width.curve = 1.2;
        brush.dynamics.opacity.minimum = 0.42;
        brush.dynamics.opacity.curve = 1.0;
        presets.push(Preset {
            id: 14 + index as u64,
            name: name.into(),
            brush,
            radius,
            color,
            stabilization: 0.35,
        });
    }
    let mut airbrush = Brush {
        kind: BrushKind::Round,
        ..Brush::default()
    };
    airbrush.flow.enabled = true;
    airbrush.flow.rate_per_second = 3.5;
    airbrush.opacity = 0.82;
    airbrush.softness = 0.9;
    airbrush.dynamics.width.minimum = 0.2;
    airbrush.dynamics.opacity.minimum = 0.12;
    presets.push(Preset {
        id: 16,
        name: "Airbrush".into(),
        brush: airbrush,
        radius: 0.12,
        color: [0.82, 0.28, 0.19],
        stabilization: 0.35,
    });
    for (id, name, profile, radius, color, tilt_orientation) in [
        (
            17,
            "Square Tube",
            crate::document::TubeProfile::Square,
            0.07,
            [0.26, 0.62, 0.85],
            false,
        ),
        (
            18,
            "Triangular Tube",
            crate::document::TubeProfile::Triangle,
            0.07,
            [0.46, 0.78, 0.34],
            true,
        ),
        (
            19,
            "Star Tube",
            crate::document::TubeProfile::Star,
            0.075,
            [0.9, 0.54, 0.19],
            false,
        ),
    ] {
        let mut brush = Brush {
            kind: BrushKind::Round,
            tube_profile: profile,
            ..Brush::default()
        };
        brush.dynamics.tilt_orientation = tilt_orientation;
        presets.push(Preset {
            id,
            name: name.into(),
            brush,
            radius,
            color,
            stabilization: 0.35,
        });
    }
    presets
}

fn default_tip_assets() -> Vec<crate::brush_tip::BrushTipAsset> {
    [TipShape::Leaf, TipShape::Grass, TipShape::Star]
        .into_iter()
        .map(|shape| {
            let (size, provenance) = match shape {
                TipShape::Leaf => ([64, 96], "Original Sketch3D leaf tip"),
                TipShape::Grass => ([64, 96], "Original Sketch3D grass tip"),
                TipShape::Star => ([96, 96], "Original Sketch3D star tip"),
            };
            let mut image = image::RgbaImage::new(size[0], size[1]);
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let fx = (x as f32 + 0.5) / size[0] as f32;
                    let fy = (y as f32 + 0.5) / size[1] as f32;
                    let pixel = match shape {
                        TipShape::Leaf => {
                            let center = 0.5 + 0.055 * (fy * std::f32::consts::PI).sin();
                            let half = 0.43 * (std::f32::consts::PI * fy).sin().powf(0.72);
                            let edge = half - (fx - center).abs();
                            let alpha = coverage(edge, 1.2 / size[0] as f32);
                            let vein = (fx - center).abs() < 0.014 + fy * 0.012;
                            if vein {
                                image::Rgba([194, 230, 134, alpha])
                            } else {
                                image::Rgba([120, 184, 74, alpha])
                            }
                        }
                        TipShape::Grass => {
                            let center = 0.5 - 0.27 * fy + 0.05 * (fy * 4.0).sin();
                            let half = (0.04 + 0.3 * fy) * (1.0 - 0.55 * fy);
                            let alpha = coverage(half - (fx - center).abs(), 1.2 / size[0] as f32);
                            image::Rgba([83, 167, 58, alpha])
                        }
                        TipShape::Star => {
                            let dx = fx - 0.5;
                            let dy = fy - 0.5;
                            let radius = dx.hypot(dy);
                            let angle = dy.atan2(dx);
                            let edge = 0.43 * (0.56 + 0.44 * (5.0 * angle).cos()) - radius;
                            let alpha = coverage(edge, 1.2 / size[0] as f32);
                            image::Rgba([255, 205, 58, alpha])
                        }
                    };
                    image.put_pixel(x, y, pixel);
                }
            }
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut png, image::ImageFormat::Png)
                .expect("built-in brush tip PNG encoding is infallible");
            crate::brush_tip::BrushTipAsset::from_png(png.into_inner(), Some(provenance.into()))
                .expect("built-in brush tip satisfies its fixed resource bounds")
        })
        .collect()
}

#[derive(Clone, Copy)]
enum TipShape {
    Leaf,
    Grass,
    Star,
}

fn coverage(distance_inside: f32, antialias: f32) -> u8 {
    ((distance_inside / antialias + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8
}

impl Library {
    pub fn get(&self, id: u64) -> Option<&Preset> {
        self.presets.iter().find(|p| p.id == id)
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != LIBRARY_VERSION {
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
        crate::brush_tip::validate_catalog(&self.tip_assets)?;
        let tip_assets: std::collections::HashMap<_, _> = self
            .tip_assets
            .iter()
            .map(|asset| (asset.id, asset))
            .collect();
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        for preset in &self.presets {
            if !preset.is_valid() || preset.name != preset.name.trim() {
                return Err("Invalid preset settings or name".into());
            }
            if preset.brush.tip_asset_id != 0 {
                let asset = tip_assets
                    .get(&preset.brush.tip_asset_id)
                    .ok_or("Preset refers to a missing brush tip asset")?;
                if preset.brush.texture.kind != TextureKind::Solid
                    || preset.brush.dynamics.tilt_orientation
                    || preset.brush.dynamics.tilt_shading
                    || (preset.brush.tip_aspect - asset.size[0] as f32 / asset.size[1] as f32).abs()
                        > 1e-4
                {
                    return Err(
                        "Preset image-tip settings are invalid or do not match its asset".into(),
                    );
                }
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

    pub fn tip_asset(&self, id: u64) -> Option<&crate::brush_tip::BrushTipAsset> {
        self.tip_assets.iter().find(|asset| asset.id == id)
    }

    fn add_tip_asset(&mut self, asset: crate::brush_tip::BrushTipAsset) -> Result<(), String> {
        asset.validate()?;
        if let Some(existing) = self.tip_asset(asset.id) {
            if existing.png != asset.png {
                return Err("Brush tip content identity collision".into());
            }
            return Ok(());
        }
        self.tip_assets.push(asset);
        crate::brush_tip::validate_catalog(&self.tip_assets)
    }
    pub fn assign_tip_asset(
        &mut self,
        preset_id: u64,
        asset: crate::brush_tip::BrushTipAsset,
    ) -> Result<(), String> {
        let mut changed = self.clone();
        changed.add_tip_asset(asset.clone())?;
        let preset = changed
            .presets
            .iter_mut()
            .find(|preset| preset.id == preset_id)
            .ok_or("Selected brush preset no longer exists")?;
        preset.brush.tip_asset_id = asset.id;
        preset.brush.tip_aspect = asset.size[0] as f32 / asset.size[1] as f32;
        preset.brush.strands.enabled = false;
        preset.brush.flow.enabled = false;
        preset.brush.dabs.enabled = true;
        preset.brush.dabs.spacing = 0.55;
        preset.brush.texture = TextureSettings::default();
        preset.brush.dynamics.tilt_orientation = false;
        preset.brush.dynamics.tilt_shading = false;
        changed.validate()?;
        *self = changed;
        Ok(())
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err("Brush library file is too large".into());
        }
        let mut library: Self =
            serde_json::from_str(text).map_err(|e| format!("Cannot read brush library: {e}"))?;
        if matches!(library.version, 1..=4) {
            let previous_version = library.version;
            let defaults = Library::default();
            let first_new_id = match previous_version {
                1 => 11,
                2 => 14,
                3 => 16,
                _ => 17,
            };
            for preset in defaults
                .presets
                .into_iter()
                .filter(|preset| preset.id >= first_new_id)
            {
                if !library
                    .presets
                    .iter()
                    .any(|existing| existing.id == preset.id)
                {
                    let mut preset = preset;
                    let base = preset.name.clone();
                    let mut name = base.clone();
                    let mut suffix = 2_u32;
                    while library
                        .presets
                        .iter()
                        .any(|existing| existing.name.eq_ignore_ascii_case(&name))
                    {
                        name = format!("{base} (built-in {suffix})");
                        suffix = suffix
                            .checked_add(1)
                            .ok_or("No unique preset names available")?;
                    }
                    preset.name = name;
                    library.presets.push(preset);
                }
            }
            if previous_version == 1 {
                for asset in defaults.tip_assets {
                    if !library
                        .tip_assets
                        .iter()
                        .any(|existing| existing.id == asset.id)
                    {
                        library.tip_assets.push(asset);
                    }
                }
                for preset in &mut library.presets {
                    if preset.brush.tip_asset_id != 0 {
                        if let Some(asset) = library
                            .tip_assets
                            .iter()
                            .find(|asset| asset.id == preset.brush.tip_asset_id)
                        {
                            preset.brush.tip_aspect = asset.size[0] as f32 / asset.size[1] as f32;
                            preset.brush.dabs.enabled = true;
                            preset.brush.texture = TextureSettings::default();
                            preset.brush.dynamics.tilt_orientation = false;
                            preset.brush.dynamics.tilt_shading = false;
                        }
                    }
                }
            }
            library.version = LIBRARY_VERSION;
        }
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
        let default_tips = default_tip_assets();
        changed.presets.splice(0..0, defaults(&default_tips));
        for asset in default_tips {
            if !changed
                .tip_assets
                .iter()
                .any(|existing| existing.id == asset.id)
            {
                changed.tip_assets.push(asset);
            }
        }
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
        assert_eq!(library.presets.len(), 269);
    }

    #[test]
    fn brush_package_roundtrip_allocates_local_id_and_resolves_name_collisions() {
        let mut source = Library::default();
        let id = source.duplicate(4, "Custom Chalk").unwrap();
        let package = BrushPackage::from_preset(source.get(id).unwrap(), None);
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
    fn graphite_and_charcoal_presets_roundtrip_through_brush_packages() {
        let source = Library::default();
        let mut destination = Library::default();
        for name in ["Graphite", "Charcoal"] {
            let preset = source
                .presets
                .iter()
                .find(|preset| preset.name == name)
                .expect("built-in texture preset exists");
            let package = BrushPackage::from_preset(preset, None);
            let decoded = BrushPackage::from_json(&package.to_json().unwrap()).unwrap();
            let imported_id = decoded.import_into(&mut destination).unwrap();
            let imported = destination.get(imported_id).unwrap();
            assert_eq!(imported.brush, preset.brush);
            assert_eq!(imported.radius, preset.radius);
            assert_eq!(imported.color, preset.color);
            assert_eq!(
                imported.brush.texture.kind,
                match name {
                    "Graphite" => TextureKind::Graphite,
                    "Charcoal" => TextureKind::Charcoal,
                    _ => unreachable!(),
                }
            );
        }
    }

    #[test]
    fn brush_package_file_roundtrip_is_atomic_and_portable() {
        let package = BrushPackage::from_preset(&Library::default().presets[0], None);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/Round Ink.brush.json");
        package.save(&path).unwrap();
        assert_eq!(BrushPackage::load(&path).unwrap(), package);
    }

    #[test]
    fn brush_package_rejects_wrong_format_version_invalid_settings_and_oversize() {
        let package = BrushPackage::from_preset(&Library::default().presets[0], None);
        let mut value = serde_json::to_value(&package).unwrap();
        value["format"] = serde_json::json!("other-brush-format");
        assert!(BrushPackage::from_json(&value.to_string()).is_err());
        value = serde_json::to_value(&package).unwrap();
        value["version"] = serde_json::json!(99);
        assert!(BrushPackage::from_json(&value.to_string()).is_err());
        value = serde_json::to_value(&package).unwrap();
        value["preset"]["radius"] = serde_json::json!(f32::NAN);
        assert!(BrushPackage::from_json(&value.to_string()).is_err());
        assert!(BrushPackage::from_json(&" ".repeat(MAX_PACKAGE_BYTES as usize + 1)).is_err());
    }
}
