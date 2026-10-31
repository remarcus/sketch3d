//! Portable metadata and bounded asset helpers for image planes.
//!
//! `asset_file` is a generated, drawing-local filename rather than an absolute
//! source path. The document integration should keep these files in a managed
//! sibling folder (for example `drawing.sketch.assets/`) and resolve every
//! filename relative to that folder. Copying the drawing and its asset folder
//! together preserves the references; missing files are represented as absent
//! assets and must not prevent the rest of the document from loading.

use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
};

pub const MAX_IMAGE_PLANES: usize = 32;
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 12_000_000;
/// Caps decoded catalog size to roughly 96 MiB of RGBA pixels.
pub const MAX_CATALOG_PIXELS: u64 = 24_000_000;
/// Caps the total on-disk image payload managed for one drawing.
pub const MAX_TOTAL_ASSET_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_NAME_CHARS: usize = 96;

/// Serializable image-plane state. Width and height are world-space lengths
/// along the rotated local X/Y axes, so their ratio is the displayed aspect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImagePlane {
    pub id: u64,
    pub name: String,
    /// Generated local filename such as `image-42.png`; never an external path.
    pub asset_file: String,
    /// Pixel dimensions recorded at import so document validation can bound
    /// the aggregate decoded texture footprint without opening every file.
    #[serde(default)]
    pub pixel_size: Option<[u32; 2]>,
    pub origin: Vec3,
    pub rotation: Quat,
    pub size: Vec2,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// Only the active drawable plane may receive drawing input.
    #[serde(default)]
    pub drawable: bool,
}

fn default_opacity() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}

impl ImagePlane {
    /// Build metadata for a newly copied PNG/JPEG asset while preserving its
    /// aspect ratio at the requested world-space width.
    pub fn from_asset(
        id: u64,
        name: impl Into<String>,
        asset_file: String,
        pixel_size: [u32; 2],
        origin: Vec3,
        rotation: Quat,
        width: f32,
    ) -> Result<Self, String> {
        if pixel_size[0] == 0 || pixel_size[1] == 0 {
            return Err("Image dimensions must be non-zero".into());
        }
        if !width.is_finite() || !(0.001..=10_000.0).contains(&width) {
            return Err("Image-plane width must be between 0.001 and 10000 world units".into());
        }
        let height = width * pixel_size[1] as f32 / pixel_size[0] as f32;
        let plane = Self {
            id,
            name: name.into(),
            asset_file,
            pixel_size: Some(pixel_size),
            origin,
            rotation,
            size: Vec2::new(width, height),
            opacity: 1.0,
            visible: true,
            locked: false,
            drawable: false,
        };
        plane.validate()?;
        Ok(plane)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 || self.id == u64::MAX {
            return Err("Image-plane ID is invalid".into());
        }
        let name_chars = self.name.chars().count();
        if self.name.trim() != self.name
            || name_chars == 0
            || name_chars > MAX_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(format!(
                "Image-plane name must contain 1–{MAX_NAME_CHARS} printable characters"
            ));
        }
        if !is_asset_name(&self.asset_file, self.id) {
            return Err(
                "Image-plane asset name is invalid; assets must use the generated local filename"
                    .into(),
            );
        }
        if self.pixel_size.is_some_and(|[width, height]| {
            width == 0
                || height == 0
                || u64::from(width).saturating_mul(u64::from(height)) > MAX_IMAGE_PIXELS
        }) {
            return Err("Image-plane pixel dimensions are invalid or exceed 12 megapixels".into());
        }
        if !self.origin.is_finite() || self.origin.abs().max_element() > 10_000.0 {
            return Err("Image-plane position is outside document bounds".into());
        }
        if !self.rotation.is_finite() || (self.rotation.length_squared() - 1.0).abs() > 0.001 {
            return Err("Image-plane rotation must be a normalized finite quaternion".into());
        }
        if !self.size.is_finite()
            || self.size.min_element() < 0.001
            || self.size.max_element() > 10_000.0
        {
            return Err(
                "Image-plane dimensions must be between 0.001 and 10000 world units".into(),
            );
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err("Image-plane opacity must be between 0 and 1".into());
        }
        Ok(())
    }

    /// Resolve this record inside the managed asset root. A missing asset is
    /// returned as `Ok(None)`, allowing artwork to load with a placeholder.
    pub fn resolve_asset(&self, root: &Path) -> Result<Option<PathBuf>, String> {
        self.validate()?;
        let candidate = root.join(&self.asset_file);
        let metadata = match std::fs::metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Cannot inspect image-plane asset: {error}")),
        };
        if !metadata.is_file() {
            return Err("Image-plane asset is not a regular file".into());
        }
        let canonical_root = root
            .canonicalize()
            .map_err(|error| format!("Cannot resolve managed image folder: {error}"))?;
        let canonical_file = candidate
            .canonicalize()
            .map_err(|error| format!("Cannot resolve image-plane asset: {error}"))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err("Image-plane asset resolves outside its managed folder".into());
        }
        Ok(Some(canonical_file))
    }
}

/// Check catalog size, individual records, and duplicate IDs before saving or
/// using image-plane metadata from a native document.
pub fn validate_catalog(planes: &[ImagePlane]) -> Result<(), String> {
    if planes.len() > MAX_IMAGE_PLANES {
        return Err(format!(
            "A document can contain at most {MAX_IMAGE_PLANES} image planes"
        ));
    }
    let mut ids = HashSet::with_capacity(planes.len());
    let mut total_pixels = 0_u64;
    for plane in planes {
        plane.validate()?;
        let [width, height] = plane.pixel_size.ok_or_else(|| {
            "Image-plane pixel dimensions are required for catalog validation".to_owned()
        })?;
        total_pixels = total_pixels.saturating_add(u64::from(width) * u64::from(height));
        if total_pixels > MAX_CATALOG_PIXELS {
            return Err(format!(
                "Image-plane catalog exceeds the {MAX_CATALOG_PIXELS} pixel budget"
            ));
        }
        if !ids.insert(plane.id) {
            return Err("Image-plane IDs must be unique".into());
        }
    }
    Ok(())
}

/// Decoded, premultiplied-independent RGBA asset data for a renderer texture.
pub struct ImageAsset {
    pub pixels: Vec<u8>,
    pub size: [u32; 2],
}

/// Load a bounded PNG/JPEG. Dimensions are checked from the image header before
/// allocating the decoded RGBA buffer.
pub fn decode_asset(path: &Path) -> Result<ImageAsset, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
        return Err("Image-plane asset must be a regular file no larger than 32 MiB".into());
    }
    let format = format_from_path(path)?;
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("Image-plane asset grew beyond the 32 MiB limit".into());
    }
    decode_bytes(&bytes, format)
}

/// Validate and copy a source image to the generated document-local asset name.
/// Uses a same-directory temporary file so interrupted copies do not leave a
/// partial asset under a valid filename.
pub fn copy_asset(
    source: &Path,
    managed_root: &Path,
    id: u64,
) -> Result<(String, ImageAsset), String> {
    if id == 0 || id == u64::MAX {
        return Err("Image-plane ID is invalid".into());
    }
    let metadata = std::fs::metadata(source).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
        return Err("Choose a regular PNG/JPEG image no larger than 32 MiB".into());
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    let format = format_from_extension(extension.as_deref())?;
    let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("Image grew beyond the 32 MiB import limit".into());
    }
    let image = decode_bytes(&bytes, format)?;
    std::fs::create_dir_all(managed_root).map_err(|error| error.to_string())?;
    let ext = if format == image::ImageFormat::Png {
        "png"
    } else {
        "jpg"
    };
    let file_name = format!("image-{id}.{ext}");
    let destination = managed_root.join(&file_name);
    if destination.exists() {
        return Err("An image-plane asset with this ID already exists".into());
    }
    let existing_bytes = std::fs::read_dir(managed_root)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .try_fold(0_u64, |total, metadata| total.checked_add(metadata.len()))
        .ok_or_else(|| "Managed image folder is too large".to_owned())?;
    if existing_bytes.saturating_add(bytes.len() as u64) > MAX_TOTAL_ASSET_BYTES {
        return Err("Managed image assets exceed the 256 MiB per-document limit".into());
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(managed_root).map_err(|error| error.to_string())?;
    temp.write_all(&bytes).map_err(|error| error.to_string())?;
    temp.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temp.persist(&destination)
        .map_err(|error| error.to_string())?;
    Ok((file_name, image))
}

fn decode_bytes(bytes: &[u8], format: image::ImageFormat) -> Result<ImageAsset, String> {
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|error| format!("Cannot read image dimensions: {error}"))?;
    if width == 0 || height == 0 || (width as u64).saturating_mul(height as u64) > MAX_IMAGE_PIXELS
    {
        return Err("Image dimensions exceed the 12 megapixel limit".into());
    }
    let rgba = image::load_from_memory_with_format(bytes, format)
        .map_err(|error| format!("Cannot decode image: {error}"))?
        .to_rgba8();
    if rgba.width() != width || rgba.height() != height {
        return Err("Decoded image dimensions changed unexpectedly".into());
    }
    Ok(ImageAsset {
        pixels: rgba.into_raw(),
        size: [width, height],
    })
}

fn format_from_path(path: &Path) -> Result<image::ImageFormat, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    format_from_extension(extension.as_deref())
}

fn format_from_extension(extension: Option<&str>) -> Result<image::ImageFormat, String> {
    match extension {
        Some("png") => Ok(image::ImageFormat::Png),
        Some("jpg" | "jpeg") => Ok(image::ImageFormat::Jpeg),
        _ => Err("Only PNG and JPEG image-plane assets are supported".into()),
    }
}

fn is_asset_name(value: &str, id: u64) -> bool {
    value == format!("image-{id}.png") || value == format!("image-{id}.jpg")
}
