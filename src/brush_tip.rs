//! Bounded, content-addressed PNG tips embedded in brush libraries and drawing bundles.
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub const MAX_PNG_BYTES: usize = 512 * 1024;
pub const MAX_DIMENSION: u32 = 512;
pub const MAX_PIXELS: u64 = 262_144;
pub const MAX_ASSETS: usize = 64;
pub const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
/// Four bytes per texel keeps worst-case decoded RGBA memory at 16 MiB.
pub const MAX_CATALOG_PIXELS: u64 = MAX_CATALOG_BYTES as u64 / 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushTipAsset {
    /// FNV-1a 64-bit fingerprint of the original PNG bytes; collisions are
    /// detected by comparing the bytes before deduplicating.
    pub id: u64,
    #[serde(with = "base64_png")]
    pub png: Vec<u8>,
    pub size: [u32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

impl BrushTipAsset {
    pub fn from_png(png: Vec<u8>, provenance: Option<String>) -> Result<Self, String> {
        let size = validate_png(&png)?;
        let asset = Self {
            id: content_id(&png),
            png,
            size,
            provenance,
        };
        asset.validate()?;
        Ok(asset)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 || self.id != content_id(&self.png) {
            return Err("Brush tip content identity does not match its PNG".into());
        }
        if validate_png(&self.png)? != self.size {
            return Err("Brush tip dimensions do not match its PNG".into());
        }
        if self
            .provenance
            .as_ref()
            .is_some_and(|value| value.chars().count() > 256 || value.chars().any(char::is_control))
        {
            return Err("Brush tip provenance is invalid".into());
        }
        Ok(())
    }

    pub fn decoded(&self) -> Result<crate::image_planes::ImageAsset, String> {
        self.validate()?;
        let image = image::load_from_memory_with_format(&self.png, image::ImageFormat::Png)
            .map_err(|error| format!("Cannot decode brush tip PNG: {error}"))?
            .to_rgba8();
        Ok(crate::image_planes::ImageAsset {
            size: [image.width(), image.height()],
            pixels: image.into_raw(),
        })
    }

    pub fn alpha_at_uv(&self, uv: [f32; 2]) -> Result<f32, String> {
        if uv.iter().any(|value| !value.is_finite()) {
            return Err("Brush tip UV coordinates are invalid".into());
        }
        let decoded = self.decoded()?;
        let x = (uv[0].clamp(0.0, 1.0) * decoded.size[0] as f32)
            .floor()
            .min(decoded.size[0].saturating_sub(1) as f32) as usize;
        let y = (uv[1].clamp(0.0, 1.0) * decoded.size[1] as f32)
            .floor()
            .min(decoded.size[1].saturating_sub(1) as f32) as usize;
        Ok(f32::from(decoded.pixels[(y * decoded.size[0] as usize + x) * 4 + 3]) / 255.0)
    }

    /// Prepare bounded repeating-texture pixels from this shared image asset.
    /// Mask mode discards source RGB and writes interpreted coverage to alpha;
    /// RGBA mode preserves source color while applying contrast, inversion, and
    /// strength to its coverage channel.
    pub fn repeating_texture_pixels(
        &self,
        settings: crate::texture::ImageTextureSettings,
    ) -> Result<crate::image_planes::ImageAsset, String> {
        use crate::texture::{ImageMaskSource, ImageTextureMode};

        if !settings.is_valid() || settings.asset_id != self.id {
            return Err("Repeating texture settings do not match the image asset".into());
        }
        let mut decoded = self.decoded()?;
        for pixel in decoded.pixels.as_chunks_mut::<4>().0 {
            let alpha = f32::from(pixel[3]) / 255.0;
            let luminance = (0.2126 * f32::from(pixel[0])
                + 0.7152 * f32::from(pixel[1])
                + 0.0722 * f32::from(pixel[2]))
                / 255.0;
            let source_coverage = match settings.mask_source {
                ImageMaskSource::Alpha => alpha,
                ImageMaskSource::Luminance => luminance,
                ImageMaskSource::LuminanceTimesAlpha => luminance * alpha,
            };
            let mut coverage = (source_coverage - 0.5) * settings.contrast + 0.5;
            if settings.invert {
                coverage = 1.0 - coverage;
            }
            coverage = coverage.clamp(0.0, 1.0);
            match settings.mode {
                ImageTextureMode::CoverageMask => {
                    let strength = settings.strength;
                    let alpha = 1.0 + (coverage - 1.0) * strength;
                    pixel[..3].fill(255);
                    pixel[3] = (alpha * 255.0).round() as u8;
                }
                ImageTextureMode::RgbaWithAlpha => {
                    let alpha = 1.0 + (coverage - 1.0) * settings.strength;
                    pixel[3] = (alpha * 255.0).round() as u8;
                }
            }
        }
        Ok(decoded)
    }

    pub fn repeating_coverage_at_uv(
        &self,
        settings: crate::texture::ImageTextureSettings,
        uv: [f32; 2],
    ) -> Result<f32, String> {
        if uv.iter().any(|value| !value.is_finite()) {
            return Err("Repeating texture UV coordinates are invalid".into());
        }
        let image = self.repeating_texture_pixels(settings)?;
        let x = (uv[0].rem_euclid(1.0) * image.size[0] as f32).floor() as usize;
        let y = (uv[1].rem_euclid(1.0) * image.size[1] as f32).floor() as usize;
        let index = (y * image.size[0] as usize + x) * 4 + 3;
        Ok(f32::from(image.pixels[index]) / 255.0)
    }

    /// Build a small tiled checkerboard preview with transparent gutters so
    /// transparent texels and repeat seams remain visible in the editor.
    pub fn repeating_preview_pixels(
        &self,
        settings: crate::texture::ImageTextureSettings,
        tint: [f32; 3],
    ) -> Result<crate::image_planes::ImageAsset, String> {
        if tint
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("Repeating texture preview tint is invalid".into());
        }
        let source = self.repeating_texture_pixels(settings)?;
        let gutter = 2_u32;
        let columns = 3_u32;
        let rows = 3_u32;
        let width = source.size[0] * columns + gutter * (columns + 1);
        let height = source.size[1] * rows + gutter * (rows + 1);
        let mut pixels = vec![0; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let checker = if (x / 8 + y / 8).is_multiple_of(2) {
                    [0.72_f32; 3]
                } else {
                    [0.88_f32; 3]
                };
                let mut color = checker;
                if x >= gutter && y >= gutter {
                    let tile_x = (x - gutter) / (source.size[0] + gutter);
                    let tile_y = (y - gutter) / (source.size[1] + gutter);
                    let local_x = (x - gutter) % (source.size[0] + gutter);
                    let local_y = (y - gutter) % (source.size[1] + gutter);
                    if tile_x < columns
                        && tile_y < rows
                        && local_x < source.size[0]
                        && local_y < source.size[1]
                    {
                        let index = ((local_y * source.size[0] + local_x) * 4) as usize;
                        let alpha = f32::from(source.pixels[index + 3]) / 255.0;
                        let source_rgb = std::array::from_fn(|channel| {
                            let encoded = f32::from(source.pixels[index + channel]) / 255.0;
                            let source_linear = crate::presentation::srgb_to_linear(encoded);
                            let textured_linear = if settings.mode
                                == crate::texture::ImageTextureMode::CoverageMask
                                || settings.tint_with_brush
                            {
                                source_linear * tint[channel]
                            } else {
                                source_linear
                            };
                            let background_linear =
                                crate::presentation::srgb_to_linear(checker[channel]);
                            crate::presentation::linear_to_srgb(
                                textured_linear * alpha + background_linear * (1.0 - alpha),
                            )
                        });
                        color = source_rgb;
                    }
                }
                let index = ((y * width + x) * 4) as usize;
                for channel in 0..3 {
                    pixels[index + channel] = (color[channel] * 255.0).round() as u8;
                }
                pixels[index + 3] = 255;
            }
        }
        Ok(crate::image_planes::ImageAsset {
            size: [width, height],
            pixels,
        })
    }

    pub fn file_name(&self) -> String {
        format!("brush-tip-{:016x}.png", self.id)
    }
}

pub fn validate_catalog(assets: &[BrushTipAsset]) -> Result<(), String> {
    if assets.len() > MAX_ASSETS {
        return Err(format!(
            "A brush library/drawing can contain at most {MAX_ASSETS} portable PNG assets"
        ));
    }
    let mut ids = std::collections::HashMap::with_capacity(assets.len());
    let mut total = 0usize;
    let mut total_pixels = 0u64;
    for asset in assets {
        asset.validate()?;
        total = total.saturating_add(asset.png.len());
        if total > MAX_CATALOG_BYTES {
            return Err("Portable PNG assets exceed the 16 MiB encoded catalog budget".into());
        }
        total_pixels = total_pixels
            .saturating_add(u64::from(asset.size[0]).saturating_mul(u64::from(asset.size[1])));
        if total_pixels > MAX_CATALOG_PIXELS {
            return Err("Portable PNG assets exceed the 16 MiB decoded RGBA memory budget".into());
        }
        if let Some(previous) = ids.insert(asset.id, asset.png.as_slice()) {
            if previous != asset.png {
                return Err("Brush tip content identity collision".into());
            }
            return Err("Brush tip assets must be deduplicated".into());
        }
    }
    Ok(())
}

pub fn content_id(bytes: &[u8]) -> u64 {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    if hash == 0 {
        1
    } else {
        hash
    }
}

fn validate_png(bytes: &[u8]) -> Result<[u32; 2], String> {
    if bytes.is_empty() || bytes.len() > MAX_PNG_BYTES {
        return Err("Brush tip PNG must be no larger than 512 KiB".into());
    }
    if image::guess_format(bytes).map_err(|error| error.to_string())? != image::ImageFormat::Png {
        return Err("Brush tips must be PNG files".into());
    }
    let dimensions = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png)
        .into_dimensions()
        .map_err(|error| format!("Cannot read brush tip PNG dimensions: {error}"))?;
    let pixels = u64::from(dimensions.0).saturating_mul(u64::from(dimensions.1));
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > MAX_DIMENSION
        || dimensions.1 > MAX_DIMENSION
        || pixels > MAX_PIXELS
    {
        return Err("Brush tip PNG dimensions exceed 512 × 512 pixels".into());
    }
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Cannot decode brush tip PNG: {error}"))?;
    Ok([dimensions.0, dimensions.1])
}

mod base64_png {
    use super::MAX_PNG_BYTES;
    use serde::{Deserialize, Deserializer, Serializer};

    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut encoded = String::with_capacity(bytes.len().saturating_add(2) / 3 * 4);
        for chunk in bytes.chunks(3) {
            let a = chunk[0];
            let b = *chunk.get(1).unwrap_or(&0);
            let c = *chunk.get(2).unwrap_or(&0);
            encoded.push(TABLE[(a >> 2) as usize] as char);
            encoded.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
            encoded.push(if chunk.len() > 1 {
                TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char
            } else {
                '='
            });
            encoded.push(if chunk.len() > 2 {
                TABLE[(c & 63) as usize] as char
            } else {
                '='
            });
        }
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() % 4 != 0 || encoded.len() > MAX_PNG_BYTES.div_ceil(3) * 4 + 4 {
            return Err(serde::de::Error::custom(
                "invalid or oversized brush tip PNG",
            ));
        }
        let estimated = encoded.len() / 4 * 3;
        let mut decoded = Vec::with_capacity(estimated);
        for (index, chunk) in encoded.as_bytes().as_chunks::<4>().0.iter().enumerate() {
            let last = index + 1 == encoded.len() / 4;
            let a = sextet(chunk[0])
                .ok_or_else(|| serde::de::Error::custom("invalid brush tip base64"))?;
            let b = sextet(chunk[1])
                .ok_or_else(|| serde::de::Error::custom("invalid brush tip base64"))?;
            let c = if chunk[2] == b'=' {
                0
            } else {
                sextet(chunk[2])
                    .ok_or_else(|| serde::de::Error::custom("invalid brush tip base64"))?
            };
            let d = if chunk[3] == b'=' {
                0
            } else {
                sextet(chunk[3])
                    .ok_or_else(|| serde::de::Error::custom("invalid brush tip base64"))?
            };
            if (!last && (chunk[2] == b'=' || chunk[3] == b'='))
                || (chunk[2] == b'=' && chunk[3] != b'=')
            {
                return Err(serde::de::Error::custom("invalid brush tip base64 padding"));
            }
            decoded.push((a << 2) | (b >> 4));
            if chunk[2] != b'=' {
                decoded.push((b << 4) | (c >> 2));
            }
            if chunk[3] != b'=' {
                decoded.push((c << 6) | d);
            }
        }
        if decoded.len() > MAX_PNG_BYTES {
            return Err(serde::de::Error::custom("brush tip PNG exceeds 512 KiB"));
        }
        Ok(decoded)
    }

    fn sextet(value: u8) -> Option<u8> {
        match value {
            b'A'..=b'Z' => Some(value - b'A'),
            b'a'..=b'z' => Some(value - b'a' + 26),
            b'0'..=b'9' => Some(value - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
}
