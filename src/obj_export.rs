//! Wavefront OBJ/MTL export for visible editable stroke geometry.
use crate::{
    document::{Document, Stroke, StrokeMaterial},
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

type MaterialKey = (u32, u32, u32, u32, TextureKind, u32, u32, StrokeMaterial);

struct Material {
    name: String,
    diffuse: [f32; 3],
    opacity: f32,
    texture: TextureSettings,
    mask_file: Option<String>,
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
    if strokes.is_empty() {
        return Err("No visible strokes to export".into());
    }

    let mut materials = Vec::<Material>::new();
    let mut material_ids = HashMap::<MaterialKey, usize>::new();
    for stroke in &strokes {
        export::stroke_mesh(stroke)?;
        let key = material_key(stroke);
        let textured = key.4 != TextureKind::Solid;
        if let std::collections::hash_map::Entry::Vacant(entry) = material_ids.entry(key) {
            if materials.len() >= 65_536 {
                return Err("OBJ export exceeds the 65,536 material limit".into());
            }
            let index = materials.len();
            let mask_file = textured.then(|| format!("mask-{index:05}.png"));
            entry.insert(index);
            materials.push(Material {
                name: format!("material_{index:05}"),
                diffuse: stroke.color,
                opacity: if stroke.material == StrokeMaterial::Cutout {
                    0.0
                } else {
                    stroke.brush.opacity
                },
                texture: stroke.brush.texture,
                mask_file,
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
    for material in &materials {
        writeln!(mtl, "newmtl {}", material.name).unwrap();
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
        if let Some(mask_file) = &material.mask_file {
            writeln!(mtl, "map_d sketch3d-{nonce}-{mask_file}").unwrap();
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
        stroke.material,
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

#[cfg(test)]
mod material_tests {
    use super::*;
    use crate::document::{Point, StrokeMaterial};
    use glam::Vec3;

    fn material_stroke(id: u64, material: StrokeMaterial) -> Stroke {
        Stroke {
            material,
            fill: None,
            group_id: None,
            layer_id: 1,
            id,
            color: [0.25, 0.5, 0.75],
            radius: 0.04,
            brush: Default::default(),
            points: vec![
                Point {
                    position: Vec3::new(0.0, id as f32 * 0.2, 0.0),
                    pressure: 0.8,
                    tilt: None,
                },
                Point {
                    position: Vec3::new(1.0, id as f32 * 0.2, 0.0),
                    pressure: 0.8,
                    tilt: None,
                },
            ],
        }
    }

    #[test]
    fn obj_mtl_preserves_material_modes_and_discloses_cutout_fallback() {
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let destination = root.path().join("materials.obj");
        let document = Document {
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
}
