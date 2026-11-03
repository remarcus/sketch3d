//! Wavefront OBJ/MTL export for visible editable stroke geometry.
use crate::{
    document::{Document, Point, Stroke, StrokeMaterial},
    export,
    texture::{self, TextureKind, TextureSettings},
};
use glam::Vec3;
use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

type MaterialKey = (
    u32,
    u32,
    u32,
    u32,
    TextureKind,
    u32,
    u32,
    StrokeMaterial,
    u64,
    u32,
    u64,
    bool,
);

/// Write a compact OBJ/MTL fixture with one stroke for each material mode.
pub fn write_material_verification_bundle(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let strokes = [
        StrokeMaterial::Shaded,
        StrokeMaterial::Unlit,
        StrokeMaterial::Emissive,
        StrokeMaterial::Cutout,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, material)| material_stroke(index as u64 + 1, material))
    .collect();
    let document = Document {
        strokes,
        ..Document::default()
    };
    let destination = directory.join("materials.obj");
    write_obj_bundle(&document, None, &destination, false)?;
    Ok(())
}

fn material_stroke(id: u64, material: StrokeMaterial) -> Stroke {
    Stroke {
        curve: None,
        material,
        point_thickness: None,
        modifiers: None,
        fill: None,
        group_id: None,
        layer_id: 1,
        id,
        color: [0.25, 0.5, 0.75],
        gradient: None,
        radius: 0.04,
        brush: Default::default(),
        points: vec![
            Point {
                position: Vec3::new(0.0, id as f32 * 0.2, 0.0),
                pressure: 0.8,
                tilt: None,
                time_ms: 0,
            },
            Point {
                position: Vec3::new(1.0, id as f32 * 0.2, 0.0),
                pressure: 0.8,
                tilt: None,
                time_ms: 0,
            },
        ],
    }
}

struct Material {
    name: String,
    diffuse: [f32; 3],
    gradient_fallback: bool,
    opacity: f32,
    texture: TextureSettings,
    mask_file: Option<String>,
    tip_file: Option<String>,
    tip_alpha_file: Option<String>,
    fill_texture_id: Option<u64>,
    fill_texture_repeat: bool,
    fill_texture_file: Option<String>,
    fill_texture_alpha_file: Option<String>,
    mode: StrokeMaterial,
}

/// Write the OBJ last so consumers never see a main file whose companions are
/// incomplete. Companions have unique names; on failure only files created by
/// this attempt are removed.
pub fn write_obj_bundle(
    doc: &Document,
    selected: Option<&std::collections::HashSet<u64>>,
    destination: &Path,
    replace: bool,
) -> Result<usize, String> {
    let strokes: Vec<&Stroke> = doc
        .strokes
        .iter()
        .filter(|stroke| {
            doc.visible(stroke)
                && !stroke.points.is_empty()
                && selected.is_none_or(|ids| ids.contains(&stroke.id))
        })
        .collect();
    crate::brush_tip::validate_catalog(&doc.brush_tip_assets)?;
    let tip_assets: HashMap<_, _> = doc
        .brush_tip_assets
        .iter()
        .map(|asset| (asset.id, asset))
        .collect();
    if strokes.iter().any(|stroke| {
        stroke.brush.tip_asset_id != 0 && !tip_assets.contains_key(&stroke.brush.tip_asset_id)
    }) {
        return Err("An exported stroke refers to a missing brush tip".into());
    }
    if strokes.iter().any(|stroke| {
        stroke
            .fill
            .and_then(|fill| fill.texture)
            .is_some_and(|texture| !tip_assets.contains_key(&texture.asset_id))
    }) {
        return Err("An exported fill refers to a missing image texture".into());
    }
    if strokes.is_empty() {
        return Err("No visible strokes to export".into());
    }
    if doc
        .layers
        .iter()
        .any(|layer| !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity))
    {
        return Err("A layer has an invalid opacity value".into());
    }

    let mut materials = Vec::<Material>::new();
    let mut material_ids = HashMap::<MaterialKey, usize>::new();
    for stroke in &strokes {
        export::stroke_mesh(stroke)?;
        let layer_opacity = doc.layer_opacity(stroke.layer_id);
        let key = material_key(stroke, layer_opacity);
        let has_tip = stroke.brush.tip_asset_id != 0;
        let textured = !has_tip && key.4 != TextureKind::Solid;
        if let std::collections::hash_map::Entry::Vacant(entry) = material_ids.entry(key) {
            if materials.len() >= 65_536 {
                return Err("OBJ export exceeds the 65,536 material limit".into());
            }
            let index = materials.len();
            let mask_file = textured.then(|| format!("mask-{index:05}.png"));
            let tip_file = has_tip.then(|| format!("tip-{:016x}.png", stroke.brush.tip_asset_id));
            let tip_alpha_file =
                has_tip.then(|| format!("tip-alpha-{:016x}.png", stroke.brush.tip_asset_id));
            let fill_texture_id = stroke
                .fill
                .and_then(|fill| fill.texture)
                .map(|t| t.asset_id);
            let fill_texture_repeat = stroke
                .fill
                .and_then(|fill| fill.texture)
                .is_some_and(|texture| texture.repeat);
            let fill_texture_alpha_file =
                fill_texture_id.map(|id| format!("fill-alpha-{id:016x}.png"));
            let fill_texture_file = fill_texture_id.map(|id| format!("fill-{id:016x}.png"));
            entry.insert(index);
            materials.push(Material {
                name: format!("material_{index:05}"),
                diffuse: stroke.fill.and_then(|fill| fill.texture).map_or_else(
                    || {
                        stroke.gradient.map_or_else(
                            || {
                                stroke
                                    .fill
                                    .and_then(|fill| fill.gradient)
                                    .map_or(stroke.color, |gradient| {
                                        gradient.sample(glam::Vec2::splat(0.5))
                                    })
                            },
                            |gradient| gradient.sample(0.5),
                        )
                    },
                    |texture| {
                        let base = stroke
                            .fill
                            .and_then(|fill| fill.gradient)
                            .map_or([1.0; 3], |gradient| gradient.sample(glam::Vec2::splat(0.5)));
                        std::array::from_fn(|i| base[i] * texture.tint[i])
                    },
                ),
                gradient_fallback: stroke.gradient.is_some()
                    || stroke.fill.and_then(|fill| fill.gradient).is_some(),
                opacity: if stroke.material == StrokeMaterial::Cutout {
                    0.0
                } else {
                    stroke.brush.opacity
                        * layer_opacity
                        * stroke
                            .fill
                            .and_then(|fill| fill.texture)
                            .map_or(1.0, |t| t.opacity)
                },
                texture: stroke.brush.texture,
                mask_file,
                tip_file,
                tip_alpha_file,
                fill_texture_id,
                fill_texture_repeat,
                fill_texture_file,
                fill_texture_alpha_file,
                mode: stroke.material,
            });
        }
    }

    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let nonce = unique_nonce();
    let mtl_name = format!("sketch3d-{nonce}.mtl");
    let mtl_path = parent.join(&mtl_name);
    let mut companion_files = Vec::<(PathBuf, Vec<u8>)>::new();
    let mut mtl = String::new();
    mtl.push_str("# Sketch3D cutout strokes export as fully transparent geometry; destination erasure is not represented by OBJ/MTL.\n");
    if strokes.iter().any(|stroke| stroke.brush.flow.enabled) {
        mtl.push_str("# Airbrush timing and per-vertex soft opacity are baked into mesh placement only approximately; OBJ/MTL cannot preserve per-vertex alpha.\n");
    }
    if strokes
        .iter()
        .any(|stroke| doc.layer_opacity(stroke.layer_id) < 1.0)
    {
        mtl.push_str("# Layer opacity is folded into MTL material opacity; OBJ/MTL cannot preserve per-point alpha or layer compositing.\n");
    }
    if strokes.iter().any(|stroke| {
        doc.layer_blend_mode(stroke.layer_id) != crate::document::LayerBlendMode::Normal
    }) {
        mtl.push_str("# Layer Multiply, Screen, and Add modes fall back to Normal in OBJ/MTL.\n");
    }
    if strokes.iter().any(|stroke| {
        stroke.gradient.is_some() || stroke.fill.and_then(|fill| fill.gradient).is_some()
    }) {
        mtl.push_str(
            "# OBJ/MTL cannot store per-vertex color; stroke and fill gradients use their evaluated midpoint color.\n",
        );
    }
    for material in &materials {
        writeln!(mtl, "newmtl {}", material.name).unwrap();
        if material.gradient_fallback {
            mtl.push_str("# Two-stop gradient approximated by its linear RGB midpoint; radial fills sample normalized center.\n");
        }
        writeln!(
            mtl,
            "Kd {:.6} {:.6} {:.6}",
            material.diffuse[0], material.diffuse[1], material.diffuse[2]
        )
        .unwrap();
        writeln!(mtl, "d {:.6}", material.opacity).unwrap();
        if material.mode == StrokeMaterial::Emissive {
            writeln!(
                mtl,
                "Ke {:.6} {:.6} {:.6}",
                material.diffuse[0], material.diffuse[1], material.diffuse[2]
            )
            .unwrap();
        }
        writeln!(
            mtl,
            "illum {}",
            if matches!(
                material.mode,
                StrokeMaterial::Shaded | StrokeMaterial::Cutout
            ) {
                2
            } else {
                0
            }
        )
        .unwrap();
        if let Some(fill_file) = &material.fill_texture_file {
            writeln!(
                mtl,
                "map_Kd {}sketch3d-{nonce}-{fill_file}",
                if material.fill_texture_repeat {
                    ""
                } else {
                    "-clamp on "
                }
            )
            .unwrap();
            if let Some(alpha_file) = &material.fill_texture_alpha_file {
                writeln!(mtl, "map_d sketch3d-{nonce}-{alpha_file}").unwrap();
            }
        } else if let Some(tip_file) = &material.tip_file {
            writeln!(mtl, "map_Kd sketch3d-{nonce}-{tip_file}").unwrap();
            writeln!(
                mtl,
                "map_d sketch3d-{nonce}-{}",
                material.tip_alpha_file.as_deref().unwrap_or_default()
            )
            .unwrap();
        } else if let Some(mask_file) = &material.mask_file {
            writeln!(mtl, "map_d sketch3d-{nonce}-{mask_file}").unwrap();
        }
        mtl.push('\n');
    }
    if mtl.len() > export::MAX_BINARY_BYTES {
        return Err("OBJ material file exceeds the 128 MiB export limit".into());
    }
    let mut companion_size = mtl.len();
    let mut emitted_tips = std::collections::HashSet::new();
    let mut emitted_fill_textures = std::collections::HashSet::new();
    for (index, material) in materials.iter().enumerate() {
        if let Some(id) = material.fill_texture_id {
            if emitted_fill_textures.insert(id) {
                let asset = tip_assets
                    .get(&id)
                    .ok_or("Missing validated fill texture")?;
                let alpha_png = tip_alpha_png(asset)?;
                companion_size = companion_size
                    .checked_add(asset.png.len())
                    .and_then(|total| total.checked_add(alpha_png.len()))
                    .ok_or("OBJ fill texture companion size overflow")?;
                if companion_size >= export::MAX_BINARY_BYTES {
                    return Err("OBJ fill textures exceed the 128 MiB export limit".into());
                }
                companion_files.push((
                    parent.join(format!("sketch3d-{nonce}-fill-{:016x}.png", id)),
                    asset.png.clone(),
                ));
                companion_files.push((
                    parent.join(format!("sketch3d-{nonce}-fill-alpha-{:016x}.png", id)),
                    alpha_png,
                ));
            }
        }
        if let Some(tip_file) = &material.tip_file {
            let id = u64::from_str_radix(
                tip_file.trim_start_matches("tip-").trim_end_matches(".png"),
                16,
            )
            .map_err(|_| "Invalid internal brush tip filename")?;
            if emitted_tips.insert(id) {
                let asset = tip_assets.get(&id).ok_or("Missing validated brush tip")?;
                let alpha_png = tip_alpha_png(asset)?;
                companion_size = companion_size
                    .checked_add(asset.png.len())
                    .and_then(|total| total.checked_add(alpha_png.len()))
                    .ok_or("OBJ companion size overflow")?;
                if companion_size >= export::MAX_BINARY_BYTES {
                    return Err("OBJ companions exceed the 128 MiB export limit".into());
                }
                companion_files.push((
                    parent.join(format!("sketch3d-{nonce}-{tip_file}")),
                    asset.png.clone(),
                ));
                companion_files.push((
                    parent.join(format!(
                        "sketch3d-{nonce}-{}",
                        material
                            .tip_alpha_file
                            .as_deref()
                            .ok_or("Missing brush tip alpha map")?
                    )),
                    alpha_png,
                ));
            }
        }
        if material.mask_file.is_some() {
            let bytes = texture::png(material.texture)?;
            companion_size = companion_size
                .checked_add(bytes.len())
                .ok_or("OBJ companion size overflow")?;
            if companion_size >= export::MAX_BINARY_BYTES {
                return Err("OBJ companions exceed the 128 MiB export limit".into());
            }
            let name = format!("sketch3d-{nonce}-mask-{index:05}.png");
            companion_files.push((parent.join(name), bytes));
        }
    }
    companion_files.push((mtl_path.clone(), mtl.into_bytes()));
    let total_companion_bytes = companion_files
        .iter()
        .map(|(_, bytes)| bytes.len())
        .try_fold(0_usize, |total, length| total.checked_add(length))
        .ok_or("OBJ companion size overflow")?;
    if total_companion_bytes >= export::MAX_BINARY_BYTES {
        return Err("OBJ companions exceed the 128 MiB export limit".into());
    }

    let mut installed = Vec::<PathBuf>::new();
    let result = (|| {
        for (path, bytes) in &companion_files {
            if path.exists() {
                return Err("OBJ companion name collision; please export again".into());
            }
            let mut temporary =
                tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
            temporary
                .write_all(bytes)
                .map_err(|error| error.to_string())?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|error| error.to_string())?;
            temporary
                .persist_noclobber(path)
                .map_err(|error| error.error.to_string())?;
            installed.push(path.clone());
        }

        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        let mut writer = BufWriter::new(BudgetWriter {
            inner: &mut temporary,
            remaining: export::MAX_BINARY_BYTES - total_companion_bytes,
        });
        writeln!(writer, "# Sketch3D OBJ export").map_err(|error| error.to_string())?;
        writeln!(
            writer,
            "# Coordinates use document world units; axes are +X right, +Y up, +Z front."
        )
        .map_err(|error| error.to_string())?;
        writeln!(writer, "mtllib {mtl_name}").map_err(|error| error.to_string())?;
        let mut vertex_offset = 0_usize;
        for stroke in &strokes {
            let mesh = export::stroke_mesh(stroke)?;
            if mesh.indices.is_empty() {
                continue;
            }
            let material = material_ids[&material_key(stroke, doc.layer_opacity(stroke.layer_id))];
            writeln!(
                writer,
                "# layer {} blend {} fallback normal",
                stroke.layer_id,
                doc.layer_blend_mode(stroke.layer_id).label()
            )
            .map_err(|error| error.to_string())?;
            if let Some(stack) = &stroke.modifiers {
                writeln!(
                    writer,
                    "# stroke {} modifier stack v{} ({} entries) is evaluated in the exported mesh; editable records are native-only",
                    stroke.id,
                    stack.version,
                    stack.modifiers.len()
                )
                .map_err(|error| error.to_string())?;
            }
            writeln!(writer, "\ng stroke_{}", stroke.id).map_err(|error| error.to_string())?;
            writeln!(writer, "usemtl material_{material:05}").map_err(|error| error.to_string())?;
            for vertex in &mesh.vertices {
                writeln!(
                    writer,
                    "v {:.9} {:.9} {:.9}",
                    vertex[0], vertex[1], vertex[2]
                )
                .map_err(|error| error.to_string())?;
            }
            for vertex in &mesh.vertices {
                writeln!(writer, "vt {:.9} {:.9}", vertex[6], vertex[7])
                    .map_err(|error| error.to_string())?;
            }
            for vertex in &mesh.vertices {
                writeln!(
                    writer,
                    "vn {:.9} {:.9} {:.9}",
                    vertex[3], vertex[4], vertex[5]
                )
                .map_err(|error| error.to_string())?;
            }
            for triangle in mesh.indices.as_chunks::<3>().0 {
                let index = |local: u32| vertex_offset + local as usize + 1;
                writeln!(
                    writer,
                    "f {0}/{0}/{0} {1}/{1}/{1} {2}/{2}/{2}",
                    index(triangle[0]),
                    index(triangle[1]),
                    index(triangle[2]),
                )
                .map_err(|error| error.to_string())?;
            }
            vertex_offset = vertex_offset
                .checked_add(mesh.vertices.len())
                .ok_or("OBJ vertex index overflow")?;
        }
        let output_size = {
            let budget = writer.into_inner().map_err(|error| error.to_string())?;
            export::MAX_BINARY_BYTES - total_companion_bytes - budget.remaining
        };
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        if replace {
            temporary
                .persist(destination)
                .map_err(|error| error.error.to_string())?;
        } else {
            temporary
                .persist_noclobber(destination)
                .map_err(|error| error.error.to_string())?;
        }
        Ok(output_size + total_companion_bytes)
    })();
    if result.is_err() {
        for path in installed {
            let _ = std::fs::remove_file(path);
        }
    }
    result
}

fn material_key(stroke: &Stroke, layer_opacity: f32) -> MaterialKey {
    let texture = texture::key(stroke.brush.texture);
    let fill_texture = stroke.fill.and_then(|fill| fill.texture);
    let diffuse = fill_texture.map_or_else(
        || {
            stroke.gradient.map_or_else(
                || {
                    stroke
                        .fill
                        .and_then(|fill| fill.gradient)
                        .map_or(stroke.color, |gradient| {
                            gradient.sample(glam::Vec2::splat(0.5))
                        })
                },
                |gradient| gradient.sample(0.5),
            )
        },
        |texture| {
            let base = stroke
                .fill
                .and_then(|fill| fill.gradient)
                .map_or([1.0; 3], |gradient| gradient.sample(glam::Vec2::splat(0.5)));
            std::array::from_fn(|i| base[i] * texture.tint[i])
        },
    );
    (
        diffuse[0].to_bits(),
        diffuse[1].to_bits(),
        diffuse[2].to_bits(),
        (stroke.brush.opacity * fill_texture.map_or(1.0, |texture| texture.opacity)).to_bits(),
        texture.0,
        texture.1,
        texture.2,
        stroke.material,
        stroke.brush.tip_asset_id,
        layer_opacity.to_bits(),
        fill_texture.map_or(0, |texture| texture.asset_id),
        fill_texture.is_none_or(|texture| texture.repeat),
    )
}

fn tip_alpha_png(asset: &crate::brush_tip::BrushTipAsset) -> Result<Vec<u8>, String> {
    let decoded = asset.decoded()?;
    let mut image = image::RgbaImage::new(decoded.size[0], decoded.size[1]);
    for (pixel, source) in image.pixels_mut().zip(decoded.pixels.as_chunks::<4>().0) {
        pixel.0 = [source[3], source[3], source[3], 255];
    }
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| format!("Could not encode brush tip alpha map: {error}"))?;
    Ok(output.into_inner())
}

fn unique_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

struct BudgetWriter<W> {
    inner: W,
    remaining: usize,
}

impl<W: Write> Write for BudgetWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.len() > self.remaining {
            return Err(std::io::Error::other(
                "OBJ exceeds the 128 MiB output limit",
            ));
        }
        let written = self.inner.write(buffer)?;
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod material_tests {
    use super::*;

    #[test]
    fn obj_mtl_preserves_material_modes_and_discloses_cutout_fallback() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let destination = root.path().join("materials.obj");
        let document = Document {
            linked_stroke_groups: Default::default(),
            strokes: [
                StrokeMaterial::Shaded,
                StrokeMaterial::Unlit,
                StrokeMaterial::Emissive,
                StrokeMaterial::Cutout,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, material)| material_stroke(index as u64 + 1, material))
            .collect(),
            ..Document::default()
        };

        assert!(write_obj_bundle(&document, None, &destination, false).unwrap() > 0);
        let obj = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(obj.matches("usemtl ").count(), 4);
        let mtl_name = obj
            .lines()
            .find_map(|line| line.strip_prefix("mtllib "))
            .unwrap();
        let mtl = std::fs::read_to_string(root.path().join(mtl_name)).unwrap();
        let section = |name: &str| {
            mtl.split_once(&format!("newmtl {name}\n"))
                .unwrap()
                .1
                .split("\nnewmtl ")
                .next()
                .unwrap()
        };

        assert!(section("material_00000").contains("illum 2"));
        assert!(section("material_00001").contains("illum 0"));
        assert!(section("material_00002").contains("illum 0"));
        assert!(section("material_00002").contains("Ke 0.250000 0.500000 0.750000"));
        assert!(section("material_00003").contains("d 0.000000"));
        assert!(section("material_00003").contains("illum 2"));
        assert!(mtl.contains(
            "cutout strokes export as fully transparent geometry; destination erasure is not represented"
        ));
    }

    #[test]
    fn graphite_and_charcoal_obj_bundles_embed_continuous_alpha_masks() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let destination = root.path().join("textures.obj");
        let strokes = [TextureKind::Graphite, TextureKind::Charcoal]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                let mut stroke = material_stroke(index as u64 + 1, StrokeMaterial::Shaded);
                stroke.brush.texture = TextureSettings {
                    kind,
                    strength: 0.8,
                    seed: 41 + index as u32,
                    ..Default::default()
                };
                stroke
            })
            .collect();
        let document = Document {
            linked_stroke_groups: Default::default(),
            strokes,
            ..Document::default()
        };
        assert!(write_obj_bundle(&document, None, &destination, false).unwrap() > 0);

        let obj = std::fs::read_to_string(&destination).unwrap();
        let mtl_name = obj
            .lines()
            .find_map(|line| line.strip_prefix("mtllib "))
            .unwrap();
        let mtl = std::fs::read_to_string(root.path().join(mtl_name)).unwrap();
        let masks: Vec<_> = mtl
            .lines()
            .filter_map(|line| line.strip_prefix("map_d "))
            .collect();
        assert_eq!(masks.len(), 2);
        for path in masks {
            let mask = image::open(root.path().join(path)).unwrap().to_rgba8();
            assert_eq!(
                [mask.width(), mask.height()],
                [texture::SIZE, texture::SIZE]
            );
            assert!(mask.pixels().any(|pixel| (1..255).contains(&pixel[3])));
        }
        assert!(obj.contains("vt "));
    }
}
