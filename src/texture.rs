//! Original procedural brush masks; no external artwork or sidecars.
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use serde::{Deserialize, Serialize};

pub const SIZE: u32 = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureKind {
    #[default]
    Solid,
    Grain,
    DryBrush,
    Pattern,
    Graphite,
    Charcoal,
    Crosshatch,
}
impl TextureKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Grain => "Grain / chalk",
            Self::DryBrush => "Dry brush",
            Self::Pattern => "Diamond pattern",
            Self::Graphite => "Graphite",
            Self::Charcoal => "Charcoal",
            Self::Crosshatch => "Crosshatch",
        }
    }

    pub fn uses_continuous_alpha(self) -> bool {
        matches!(self, Self::Graphite | Self::Charcoal)
    }
}

/// Scale is a world-space tile period; strength controls removed coverage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureSettings {
    #[serde(default)]
    pub mapping: TextureMapping,
    pub kind: TextureKind,
    pub scale: f32,
    pub strength: f32,
    pub seed: u32,
}
impl Default for TextureSettings {
    fn default() -> Self {
        Self {
            mapping: TextureMapping::default(),
            kind: TextureKind::Solid,
            scale: 0.2,
            strength: 0.65,
            seed: 1,
        }
    }
}
impl TextureSettings {
    pub fn is_valid(self) -> bool {
        self.mapping.is_valid()
            && self.scale.is_finite()
            && (0.02..=10.0).contains(&self.scale)
            && self.strength.is_finite()
            && (0.0..=1.0).contains(&self.strength)
    }
}

/// Affine UV mapping; stored with the stroke and independent of the camera.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureMapping {
    pub rotation_degrees: f32,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}
impl Default for TextureMapping {
    fn default() -> Self {
        Self {
            rotation_degrees: 0.0,
            scale: [1.0; 2],
            offset: [0.0; 2],
        }
    }
}
impl TextureMapping {
    pub fn is_valid(self) -> bool {
        self.rotation_degrees.is_finite()
            && self.rotation_degrees.abs() <= 180.0
            && self
                .scale
                .iter()
                .all(|v| v.is_finite() && (0.05..=20.0).contains(v))
            && self
                .offset
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1000.0)
    }
    pub fn map(self, uv: [f32; 2]) -> [f32; 2] {
        if self == Self::default() {
            return uv;
        }
        let (sin, cos) = self.rotation_degrees.to_radians().sin_cos();
        let u = uv[0] / self.scale[0];
        let v = uv[1] / self.scale[1];
        [
            cos * u - sin * v + self.offset[0],
            sin * u + cos * v + self.offset[1],
        ]
    }
}

/// Scale affects UVs only and therefore does not create another mask image.
pub fn key(settings: TextureSettings) -> (TextureKind, u32, u32) {
    if settings.kind == TextureKind::Solid || settings.strength == 0.0 {
        (TextureKind::Solid, 0, 0)
    } else {
        (settings.kind, settings.strength.to_bits(), settings.seed)
    }
}

fn noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut value = x.wrapping_mul(0x9e37_79b9) ^ y.wrapping_mul(0x85eb_ca6b) ^ seed;
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    (value & 0x00ff_ffff) as f32 / 0x0100_0000 as f32
}

/// U runs along the stroke and V across its section. Masks repeat in UV space;
/// older textures remain binary while graphite and charcoal use soft alpha.
pub fn rgba(settings: TextureSettings) -> Vec<u8> {
    let mut result = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            result.extend_from_slice(&[255, 255, 255, texel_alpha(settings, x, y)]);
        }
    }
    result
}

/// Match the nearest, repeating texture sampler used by the renderer.
pub fn covered_at_uv(settings: TextureSettings, uv: [f32; 2]) -> bool {
    if !uv.into_iter().all(f32::is_finite) {
        return false;
    }
    let x = (uv[0].rem_euclid(1.0) * SIZE as f32).floor() as u32;
    let y = (uv[1].rem_euclid(1.0) * SIZE as f32).floor() as u32;
    texel_covered(settings, x.min(SIZE - 1), y.min(SIZE - 1))
}

fn texel_covered(settings: TextureSettings, x: u32, y: u32) -> bool {
    texel_alpha(settings, x, y) >= 128
}

fn texel_alpha(settings: TextureSettings, x: u32, y: u32) -> u8 {
    if settings.kind == TextureKind::Solid || settings.strength <= 0.0 {
        return 255;
    }
    let alpha = match settings.kind {
        TextureKind::Solid => 1.0,
        TextureKind::Grain => {
            if noise(x, y, settings.seed) >= settings.strength * 0.65 {
                1.0
            } else {
                0.0
            }
        }
        TextureKind::DryBrush => {
            let lane = noise(0, y / 3, settings.seed);
            let streak = noise(x / 16, y / 3, settings.seed ^ 0x7a21_eb37);
            if lane * 0.85 + streak * 0.15 >= settings.strength * 0.8 {
                1.0
            } else {
                0.0
            }
        }
        TextureKind::Pattern => {
            let phase_x = settings.seed & 31;
            let phase_y = (settings.seed >> 5) & 31;
            let dx = ((x + phase_x) % 32).abs_diff(16) as f32;
            let dy = ((y + phase_y) % 32).abs_diff(16) as f32;
            if 1.0 - (dx + dy) / 32.0 >= settings.strength * 0.95 {
                1.0
            } else {
                0.0
            }
        }
        TextureKind::Crosshatch => {
            // Integer-period diagonals join exactly across the repeating tile.
            let phase = settings.seed & 31;
            let forward = (x + y + phase) % 32;
            let backward = (x + SIZE - y + phase) % 32;
            let ink = forward < 4 || backward < 4;
            if ink {
                1.0
            } else {
                1.0 - settings.strength
            }
        }
        TextureKind::Graphite => {
            let coarse = periodic_value_noise(x, y, settings.seed, 5);
            let fine = periodic_value_noise(x, y, settings.seed ^ 0x6d2b_79f5, 29);
            let grit = periodic_value_noise(x, y, settings.seed ^ 0xa511_e9b3, 71);
            let grain = coarse * 0.24 + fine * 0.48 + grit * 0.28;
            1.0 - settings.strength * (1.0 - (0.24 + grain * 0.76))
        }
        TextureKind::Charcoal => {
            let broad = periodic_value_noise(x, y, settings.seed, 6);
            let mid = periodic_value_noise(x, y, settings.seed ^ 0x68bc_21eb, 19);
            let grit = periodic_value_noise(x, y, settings.seed ^ 0x02e5_be93, 47);
            let field = broad * 0.58 + mid * 0.3 + grit * 0.12;
            let density = smoothstep(0.18, 0.76, field);
            1.0 - settings.strength * (1.0 - (0.06 + density * 0.94))
        }
    };
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn periodic_value_noise(x: u32, y: u32, seed: u32, period: u32) -> f32 {
    let px = x as f32 / SIZE as f32 * period as f32;
    let py = y as f32 / SIZE as f32 * period as f32;
    let ix = px.floor() as u32;
    let iy = py.floor() as u32;
    let fx = fade(px - ix as f32);
    let fy = fade(py - iy as f32);
    let x0 = ix % period;
    let x1 = (ix + 1) % period;
    let y0 = iy % period;
    let y1 = (iy + 1) % period;
    let a = noise(x0, y0, seed);
    let b = noise(x1, y0, seed);
    let c = noise(x0, y1, seed);
    let d = noise(x1, y1, seed);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fy
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn png(settings: TextureSettings) -> Result<Vec<u8>, String> {
    if !settings.is_valid() {
        return Err("Invalid texture settings".into());
    }
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(&rgba(settings), SIZE, SIZE, ColorType::Rgba8.into())
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_changes_only_mesh_uvs_and_reuses_mask() {
        let doc = crate::export_check::textured_fixture();
        let mut stroke = doc
            .strokes
            .into_iter()
            .find(|s| s.brush.texture.kind == TextureKind::Grain)
            .unwrap();
        let original = crate::geometry::stroke_mesh(&stroke);
        let mapping = TextureMapping {
            rotation_degrees: 31.0,
            scale: [2.0, 0.5],
            offset: [0.2, -0.3],
        };
        stroke.brush.texture.mapping = mapping;
        let mapped = crate::geometry::stroke_mesh(&stroke);
        assert!(!mapped.is_empty());
        assert_eq!(mapped.len(), original.len());
        assert!(mapped.iter().zip(&original).any(|(a, b)| a.uv != b.uv));
        for (a, b) in mapped.iter().zip(&original) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.normal, b.normal);
            assert_eq!(a.color, b.color);
            assert_eq!(a.opacity, b.opacity);
            assert_eq!(a.uv, mapping.map(b.uv));
        }
        let reopened = crate::document::Document::from_json(
            &serde_json::to_string(&crate::document::Document {
                strokes: vec![stroke],
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(reopened.strokes[0].brush.texture.mapping, mapping);
    }
    #[test]
    fn mapping_is_affine_bounded_and_legacy_identity() {
        let legacy = r#"{"kind":"grain","scale":0.2,"strength":0.5,"seed":1}"#;
        let settings: TextureSettings = serde_json::from_str(legacy).unwrap();
        assert_eq!(settings.mapping.map([2.0, -3.0]), [2.0, -3.0]);
        let mapping = TextureMapping {
            rotation_degrees: 90.0,
            scale: [2.0, 4.0],
            offset: [0.25, -0.5],
        };
        let uv = mapping.map([4.0, 4.0]);
        assert!((uv[0] + 0.75).abs() < 1e-6);
        assert!((uv[1] - 1.5).abs() < 1e-6);
        assert!(mapping.is_valid());
        assert!(!TextureMapping {
            scale: [0.0, 1.0],
            ..mapping
        }
        .is_valid());
        assert!(!TextureMapping {
            rotation_degrees: f32::NAN,
            ..mapping
        }
        .is_valid());
        assert!(!TextureMapping {
            offset: [f32::INFINITY, 0.0],
            ..mapping
        }
        .is_valid());
        let mapped = TextureSettings {
            mapping,
            ..settings
        };
        assert_eq!(key(settings), key(mapped));
        assert_eq!(png(settings).unwrap(), png(mapped).unwrap());
        assert_eq!(
            serde_json::from_str::<TextureSettings>(&serde_json::to_string(&mapped).unwrap())
                .unwrap(),
            mapped
        );
    }
    #[test]
    fn crosshatch_repeats_both_diagonals_and_preserves_strength() {
        let mut texture = TextureSettings {
            kind: TextureKind::Crosshatch,
            strength: 1.0,
            seed: 0,
            ..TextureSettings::default()
        };
        assert_eq!(texel_alpha(texture, 8, 8), 255);
        assert_eq!(texel_alpha(texture, 8, 24), 255);
        assert_eq!(texel_alpha(texture, 8, 16), 0);
        for y in 0..SIZE {
            assert_eq!(texel_alpha(texture, 0, y), texel_alpha(texture, SIZE, y));
            assert_eq!(texel_alpha(texture, y, 0), texel_alpha(texture, y, SIZE));
        }
        texture.strength = 0.0;
        assert!(rgba(texture)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255));
        texture.strength = 0.5;
        assert_eq!(texel_alpha(texture, 8, 16), 128);
        let saved = serde_json::to_string(&texture).unwrap();
        assert_eq!(
            serde_json::from_str::<TextureSettings>(&saved).unwrap(),
            texture
        );
    }
    fn settings(kind: TextureKind) -> TextureSettings {
        TextureSettings {
            kind,
            ..TextureSettings::default()
        }
    }
    #[test]
    fn masks_are_deterministic_binary_original_patterns() {
        let masks: Vec<_> = [
            TextureKind::Grain,
            TextureKind::DryBrush,
            TextureKind::Pattern,
        ]
        .map(|kind| rgba(settings(kind)))
        .into();
        for (index, mask) in masks.iter().enumerate() {
            assert_eq!(mask.len(), (SIZE * SIZE * 4) as usize);
            let ink = mask
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] == 255)
                .count();
            assert!((2000..15000).contains(&ink));
            assert!(mask
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[..3] == [255; 3] && [0, 255].contains(&p[3])));
            assert_ne!(*mask, masks[(index + 1) % masks.len()]);
        }
        assert_eq!(rgba(settings(TextureKind::Grain)), masks[0]);
        for kind in [
            TextureKind::Grain,
            TextureKind::DryBrush,
            TextureKind::Pattern,
        ] {
            let mut changed = settings(kind);
            changed.seed = 12345;
            assert_ne!(rgba(changed), rgba(settings(kind)));
            changed.strength = 0.0;
            assert!(rgba(changed).iter().all(|v| *v == 255));
        }
    }

    #[test]
    fn graphite_and_charcoal_masks_have_distinct_deterministic_soft_alpha() {
        let graphite = settings(TextureKind::Graphite);
        let charcoal = settings(TextureKind::Charcoal);
        let graphite_mask = rgba(graphite);
        let charcoal_mask = rgba(charcoal);
        for (settings, mask) in [(graphite, &graphite_mask), (charcoal, &charcoal_mask)] {
            assert_eq!(rgba(settings).as_slice(), mask.as_slice());
            assert!(mask
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| (1..255).contains(&pixel[3])));
            let mut changed_seed = settings;
            changed_seed.seed = changed_seed.seed.wrapping_add(1);
            assert_ne!(mask.as_slice(), rgba(changed_seed).as_slice());
        }
        assert_ne!(graphite_mask, charcoal_mask);
        assert!(rgba(settings(TextureKind::Grain))
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| matches!(pixel[3], 0 | 255)));
    }

    #[test]
    fn uv_coverage_matches_nearest_repeating_mask_texels() {
        for kind in [
            TextureKind::Grain,
            TextureKind::DryBrush,
            TextureKind::Pattern,
        ] {
            let settings = settings(kind);
            let mask = rgba(settings);
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let uv = [
                        (x as f32 + 0.5) / SIZE as f32,
                        (y as f32 + 0.5) / SIZE as f32,
                    ];
                    let expected = mask[((y * SIZE + x) * 4 + 3) as usize] != 0;
                    assert_eq!(covered_at_uv(settings, uv), expected);
                }
            }
            assert_eq!(
                covered_at_uv(settings, [1.25, -0.25]),
                covered_at_uv(settings, [0.25, 0.75])
            );
        }
    }
    #[test]
    fn strength_removes_coverage_monotonically_and_dry_marks_form_streaks() {
        for kind in [
            TextureKind::Grain,
            TextureKind::DryBrush,
            TextureKind::Pattern,
        ] {
            let mut light = settings(kind);
            light.strength = 0.25;
            let mut heavy = light;
            heavy.strength = 0.9;
            assert!(rgba(light)
                .as_chunks::<4>()
                .0
                .iter()
                .zip(rgba(heavy).as_chunks::<4>().0.iter())
                .all(|(a, b)| b[3] <= a[3]));
        }
        let mask = rgba(settings(TextureKind::DryBrush));
        let matching = (0..SIZE)
            .flat_map(|y| (0..SIZE - 1).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                mask[((y * SIZE + x) * 4 + 3) as usize]
                    == mask[((y * SIZE + x + 1) * 4 + 3) as usize]
            })
            .count();
        assert!(matching > (SIZE * (SIZE - 1)) as usize * 9 / 10);
    }
    #[test]
    fn png_embeds_exact_mask_and_key_excludes_scale() {
        let original = settings(TextureKind::Grain);
        let encoded = png(original).unwrap();
        let decoded = image::load_from_memory(&encoded).unwrap().to_rgba8();
        assert_eq!(decoded.as_raw(), &rgba(original));
        let mut scaled = original;
        scaled.scale = 2.0;
        assert_eq!(key(scaled), key(original));
        assert_eq!(
            key(settings(TextureKind::Solid)),
            (TextureKind::Solid, 0, 0)
        );
    }
    #[test]
    fn settings_reject_nonfinite_and_out_of_range_values() {
        for invalid in [
            TextureSettings {
                scale: 0.0,
                ..Default::default()
            },
            TextureSettings {
                scale: 11.0,
                ..Default::default()
            },
            TextureSettings {
                scale: f32::NAN,
                ..Default::default()
            },
            TextureSettings {
                strength: -0.1,
                ..Default::default()
            },
            TextureSettings {
                strength: f32::INFINITY,
                ..Default::default()
            },
        ] {
            assert!(!invalid.is_valid());
            assert!(png(invalid).is_err());
        }
    }
}
