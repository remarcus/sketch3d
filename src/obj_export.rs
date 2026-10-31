//! Wavefront OBJ/MTL export for visible editable stroke geometry.
use crate::{
    document::{Document, Stroke},
    export,
    texture::{self, TextureKind, TextureSettings},
};
use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

type MaterialKey = (u32, u32, u32, u32, TextureKind, u32, u32);

struct Material {
    name: String,
    diffuse: [f32; 3],
    opacity: f32,
    texture: TextureSettings,
    mask_file: Option<String>,
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
    if strokes.is_empty() {
        return Err("No visible strokes to export".into());
    }

    let mut materials = Vec::<Material>::new();
    let mut material_ids = HashMap::<MaterialKey, usize>::new();
    for stroke in &strokes {
        export::stroke_mesh(stroke)?;
        let key = material_key(stroke);
        if !material_ids.contains_key(&key) {
            if materials.len() >= 65_536 {
                return Err("OBJ export exceeds the 65,536 material limit".into());
            }
            let index = materials.len();
            let mask_file = (key.4 != TextureKind::Solid).then(|| format!("mask-{index:05}.png"));
            material_ids.insert(key, index);
            materials.push(Material {
                name: format!("material_{index:05}"),
                diffuse: stroke.color,
                opacity: stroke.brush.opacity,
                texture: stroke.brush.texture,
                mask_file,
            });
        }
    }

    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let nonce = unique_nonce();
    let mtl_name = format!("spatial-sketch-{nonce}.mtl");
    let mtl_path = parent.join(&mtl_name);
    let mut companion_files = Vec::<(PathBuf, Vec<u8>)>::new();
    let mut mtl = String::new();
    for material in &materials {
        writeln!(mtl, "newmtl {}", material.name).unwrap();
        writeln!(
            mtl,
            "Kd {:.6} {:.6} {:.6}",
            material.diffuse[0], material.diffuse[1], material.diffuse[2]
        )
        .unwrap();
        writeln!(mtl, "d {:.6}", material.opacity).unwrap();
        if let Some(mask_file) = &material.mask_file {
            writeln!(mtl, "map_d spatial-sketch-{nonce}-{mask_file}").unwrap();
        }
        mtl.push('\n');
    }
    if mtl.len() > export::MAX_BINARY_BYTES {
        return Err("OBJ material file exceeds the 128 MiB export limit".into());
    }
    let mut companion_size = mtl.len();
    for (index, material) in materials.iter().enumerate() {
        if material.mask_file.is_some() {
            let bytes = texture::png(material.texture)?;
            companion_size = companion_size
                .checked_add(bytes.len())
                .ok_or("OBJ companion size overflow")?;
            if companion_size >= export::MAX_BINARY_BYTES {
                return Err("OBJ companions exceed the 128 MiB export limit".into());
            }
            let name = format!("spatial-sketch-{nonce}-mask-{index:05}.png");
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
        writeln!(writer, "# Spatial Sketch OBJ export").map_err(|error| error.to_string())?;
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
            let material = material_ids[&material_key(stroke)];
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
            for triangle in mesh.indices.chunks_exact(3) {
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
        let budget = writer.into_inner().map_err(|error| error.to_string())?;
        let output_size = export::MAX_BINARY_BYTES - total_companion_bytes - budget.remaining;
        drop(budget);
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

fn material_key(stroke: &Stroke) -> MaterialKey {
    let texture = texture::key(stroke.brush.texture);
    (
        stroke.color[0].to_bits(),
        stroke.color[1].to_bits(),
        stroke.color[2].to_bits(),
        stroke.brush.opacity.to_bits(),
        texture.0,
        texture.1,
        texture.2,
    )
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
