//! Bounded, self-contained GLB model reference import.

use crate::obj_reference::{ObjIndex, ObjMaterial, ObjModel, ObjSourceFormat, ObjTriangle};
use glam::{Mat4, Vec2, Vec3};
use std::{fs, io::Read, path::Path};

pub const MAX_GLB_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_EMBEDDED_IMAGE_BYTES: usize = 64 * 1024 * 1024;

pub fn load(path: &Path) -> Result<ObjModel, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Cannot inspect GLB file: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_GLB_BYTES {
        return Err("GLB must be a regular file no larger than 64 MiB".into());
    }
    let source = path
        .canonicalize()
        .map_err(|error| format!("Cannot resolve GLB file: {error}"))?;
    let file = fs::File::open(&source).map_err(|error| format!("Cannot read GLB file: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_GLB_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read GLB file: {error}"))?;
    if bytes.len() as u64 > MAX_GLB_BYTES {
        return Err("GLB source exceeds the 64 MiB limit".into());
    }
    parse(&bytes, &source)
}

pub fn parse(bytes: &[u8], source: &Path) -> Result<ObjModel, String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_GLB_BYTES {
        return Err("GLB source must contain 1 byte–64 MiB".into());
    }
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|error| format!("Invalid GLB: {error}"))?;
    let blob = gltf
        .blob
        .as_deref()
        .ok_or("GLB must contain its binary buffer")?;
    let buffers: Vec<_> = gltf.buffers().collect();
    if buffers.len() != 1
        || !matches!(buffers[0].source(), gltf::buffer::Source::Bin)
        || buffers[0].length() > blob.len()
    {
        return Err("GLB must use one self-contained binary buffer".into());
    }
    let mut image_bytes = 0usize;
    let mut decoded_images: Vec<Option<crate::image_planes::ImageAsset>> = Vec::new();
    let mut total_image_pixels = 0u64;
    for image in gltf.images() {
        let gltf::image::Source::View { view, mime_type } = image.source() else {
            return Err(
                "GLB images must be embedded in buffer views; external image URIs are unsupported"
                    .into(),
            );
        };
        if !matches!(view.buffer().source(), gltf::buffer::Source::Bin)
            || view
                .offset()
                .checked_add(view.length())
                .is_none_or(|end| end > buffers[0].length())
            || !matches!(mime_type, "image/png" | "image/jpeg" | "image/webp")
        {
            return Err("GLB contains an invalid or unsupported embedded image".into());
        }
        image_bytes = image_bytes.saturating_add(view.length());
        if image_bytes > MAX_EMBEDDED_IMAGE_BYTES {
            return Err("GLB embedded images exceed the 64 MiB limit".into());
        }
        let format = match mime_type {
            "image/png" => image::ImageFormat::Png,
            "image/jpeg" => image::ImageFormat::Jpeg,
            "image/webp" => {
                decoded_images.push(None);
                continue;
            }
            _ => unreachable!(),
        };
        let image_data = &blob[view.offset()..view.offset() + view.length()];
        let (width, height) =
            image::ImageReader::with_format(std::io::Cursor::new(image_data), format)
                .into_dimensions()
                .map_err(|error| format!("Cannot read embedded GLB image dimensions: {error}"))?;
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        total_image_pixels = total_image_pixels.saturating_add(pixels);
        if width == 0
            || height == 0
            || pixels > crate::image_planes::MAX_IMAGE_PIXELS
            || total_image_pixels > crate::image_planes::MAX_CATALOG_PIXELS
        {
            return Err("GLB embedded images exceed the 24 megapixel decoded limit".into());
        }
        let rgba = image::load_from_memory_with_format(image_data, format)
            .map_err(|error| format!("Cannot decode embedded GLB image: {error}"))?
            .to_rgba8();
        if rgba.width() != width || rgba.height() != height {
            return Err("Decoded GLB image dimensions changed unexpectedly".into());
        }
        decoded_images.push(Some(crate::image_planes::ImageAsset {
            pixels: rgba.into_raw(),
            size: [width, height],
        }));
    }

    let mut materials = vec![ObjMaterial {
        name: "Default".into(),
        diffuse: [1.0; 3],
        opacity: 1.0,
        metallic: 1.0,
        roughness: 1.0,
        emissive_factor: [0.0; 3],
        base_color_texture: None,
        base_color_tex_coord: None,
        texture_sampler: None,
        alpha_mode: crate::obj_reference::ObjMaterialAlphaMode::Opaque,
        alpha_cutoff: 0.5,
    }];
    let mut warnings = Vec::new();
    for material in gltf.materials() {
        let pbr = material.pbr_metallic_roughness();
        let color = pbr.base_color_factor();
        let metallic = pbr.metallic_factor();
        let roughness = pbr.roughness_factor();
        let emissive_factor = material.emissive_factor();
        if !metallic.is_finite()
            || !(0.0..=1.0).contains(&metallic)
            || !roughness.is_finite()
            || !(0.0..=1.0).contains(&roughness)
            || emissive_factor
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("GLB PBR material factors must be finite and between 0 and 1".into());
        }
        let alpha_cutoff = material.alpha_cutoff().unwrap_or(0.5);
        if material.alpha_mode() == gltf::material::AlphaMode::Mask
            && (!alpha_cutoff.is_finite() || !(0.0..=1.0).contains(&alpha_cutoff))
        {
            return Err("GLB alpha-mask cutoff must be between 0 and 1".into());
        }
        materials.push(ObjMaterial {
            name: material
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Material {}", material.index().unwrap_or(0) + 1)),
            diffuse: [color[0], color[1], color[2]],
            opacity: if material.alpha_mode() == gltf::material::AlphaMode::Opaque {
                1.0
            } else {
                color[3]
            },
            metallic,
            roughness,
            emissive_factor,
            base_color_texture: pbr.base_color_texture().and_then(|info| {
                decoded_images
                    .get(info.texture().source().index())
                    .cloned()
                    .flatten()
            }),
            base_color_tex_coord: pbr.base_color_texture().map(|info| info.tex_coord()),
            texture_sampler: pbr.base_color_texture().map(|info| {
                let sampler = info.texture().sampler();
                crate::obj_reference::ObjTextureSampler {
                    wrap_s: match sampler.wrap_s() {
                        gltf::texture::WrappingMode::Repeat => {
                            crate::obj_reference::ObjTextureWrap::Repeat
                        }
                        gltf::texture::WrappingMode::ClampToEdge => {
                            crate::obj_reference::ObjTextureWrap::Clamp
                        }
                        gltf::texture::WrappingMode::MirroredRepeat => {
                            crate::obj_reference::ObjTextureWrap::MirroredRepeat
                        }
                    },
                    wrap_t: match sampler.wrap_t() {
                        gltf::texture::WrappingMode::Repeat => {
                            crate::obj_reference::ObjTextureWrap::Repeat
                        }
                        gltf::texture::WrappingMode::ClampToEdge => {
                            crate::obj_reference::ObjTextureWrap::Clamp
                        }
                        gltf::texture::WrappingMode::MirroredRepeat => {
                            crate::obj_reference::ObjTextureWrap::MirroredRepeat
                        }
                    },
                    mag_filter: match sampler
                        .mag_filter()
                        .unwrap_or(gltf::texture::MagFilter::Linear)
                    {
                        gltf::texture::MagFilter::Nearest => {
                            crate::obj_reference::ObjTextureFilter::Nearest
                        }
                        gltf::texture::MagFilter::Linear => {
                            crate::obj_reference::ObjTextureFilter::Linear
                        }
                    },
                    min_filter: match sampler
                        .min_filter()
                        .unwrap_or(gltf::texture::MinFilter::LinearMipmapLinear)
                    {
                        gltf::texture::MinFilter::Nearest => {
                            crate::obj_reference::ObjTextureFilter::Nearest
                        }
                        gltf::texture::MinFilter::Linear => {
                            crate::obj_reference::ObjTextureFilter::Linear
                        }
                        gltf::texture::MinFilter::NearestMipmapNearest => {
                            crate::obj_reference::ObjTextureFilter::NearestMipmapNearest
                        }
                        gltf::texture::MinFilter::LinearMipmapNearest => {
                            crate::obj_reference::ObjTextureFilter::LinearMipmapNearest
                        }
                        gltf::texture::MinFilter::NearestMipmapLinear => {
                            crate::obj_reference::ObjTextureFilter::NearestMipmapLinear
                        }
                        gltf::texture::MinFilter::LinearMipmapLinear => {
                            crate::obj_reference::ObjTextureFilter::LinearMipmapLinear
                        }
                    },
                }
            }),
            alpha_mode: match material.alpha_mode() {
                gltf::material::AlphaMode::Opaque => {
                    crate::obj_reference::ObjMaterialAlphaMode::Opaque
                }
                gltf::material::AlphaMode::Mask => crate::obj_reference::ObjMaterialAlphaMode::Mask,
                gltf::material::AlphaMode::Blend => {
                    crate::obj_reference::ObjMaterialAlphaMode::Blend
                }
            },
            alpha_cutoff,
        });
        if pbr.base_color_texture().is_some() {
            if pbr.base_color_texture().is_some_and(|info| {
                decoded_images
                    .get(info.texture().source().index())
                    .is_none_or(Option::is_none)
            }) {
                warnings.push("WebP base-color textures are retained in the bundled GLB; viewport shading uses the material base-color factor because WebP decoding is unavailable.".into());
            }
        }
        if metallic > 0.0 || roughness < 1.0 {
            warnings.push("GLB metallic and roughness factors are preserved in whole-scene GLB export; the viewport uses a diffuse lighting approximation. Emissive factors render in the viewport and are preserved on export.".into());
        }
    }
    if materials.len() > crate::obj_reference::MAX_MATERIALS {
        return Err("GLB exceeds the 256 material limit".into());
    }

    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or("GLB contains no scene")?;
    let mut positions = Vec::new();
    let mut texcoords = Vec::new();
    let mut normals = Vec::new();
    let mut objects = Vec::new();
    let mut groups = Vec::new();
    let mut triangles = Vec::new();
    for node in scene.nodes() {
        visit_node(
            node,
            Mat4::IDENTITY,
            blob,
            &mut positions,
            &mut texcoords,
            &mut normals,
            &mut objects,
            &mut groups,
            &materials,
            &mut warnings,
            &mut triangles,
        )?;
    }
    if triangles.is_empty() {
        return Err("GLB scene contains no triangle geometry".into());
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for triangle in &triangles {
        for corner in triangle.corners {
            let position = positions[corner.position as usize];
            min = min.min(position);
            max = max.max(position);
        }
    }
    Ok(ObjModel {
        source_format: ObjSourceFormat::Glb,
        positions,
        texcoords,
        normals,
        objects,
        groups,
        materials,
        triangles,
        bounds: [min, max],
        warnings,
        source_obj: source.to_owned(),
        source_materials: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn visit_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    blob: &[u8],
    positions: &mut Vec<Vec3>,
    texcoords: &mut Vec<Vec2>,
    normals: &mut Vec<Vec3>,
    objects: &mut Vec<String>,
    groups: &mut Vec<String>,
    materials: &[ObjMaterial],
    warnings: &mut Vec<String>,
    triangles: &mut Vec<ObjTriangle>,
) -> Result<(), String> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let transform = parent * local;
    let normal_transform = transform.inverse().transpose();
    if !transform.is_finite() || !normal_transform.is_finite() {
        return Err("GLB node transform is invalid".into());
    }
    if let Some(mesh) = node.mesh() {
        let object = push_name(
            objects,
            node.name()
                .unwrap_or_else(|| mesh.name().unwrap_or("Object")),
        );
        let group = push_name(groups, mesh.name().unwrap_or("Mesh"));
        for primitive in mesh.primitives() {
            if primitive.mode() == gltf::mesh::Mode::Points
                || primitive.mode() == gltf::mesh::Mode::Lines
                || primitive.mode() == gltf::mesh::Mode::LineLoop
                || primitive.mode() == gltf::mesh::Mode::LineStrip
            {
                continue;
            }
            let reader = primitive.reader(|buffer| match buffer.source() {
                gltf::buffer::Source::Bin => Some(blob),
                gltf::buffer::Source::Uri(_) => None,
            });
            let material = primitive
                .material()
                .index()
                .map(|index| index as u16 + 1)
                .unwrap_or(0);
            if material as usize >= materials.len() {
                return Err("GLB material index is invalid".into());
            }
            let local_positions: Vec<_> = reader
                .read_positions()
                .ok_or("GLB triangle primitive is missing positions")?
                .map(|point| transform.transform_point3(Vec3::from_array(point)))
                .collect();
            if local_positions.is_empty()
                || positions.len().saturating_add(local_positions.len())
                    > crate::obj_reference::MAX_POSITIONS
            {
                return Err("GLB exceeds the one million vertex limit".into());
            }
            if local_positions
                .iter()
                .any(|point| !point.is_finite() || point.abs().max_element() > 10_000.0)
            {
                return Err("GLB transformed geometry exceeds the 10,000-unit limit".into());
            }
            let position_base = positions.len() as u32;
            positions.extend(local_positions);

            let local_normals: Vec<_> = reader
                .read_normals()
                .map(|normals| {
                    normals
                        .map(|normal| {
                            normal_transform
                                .transform_vector3(Vec3::from_array(normal))
                                .normalize_or_zero()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let normal_base = normals.len() as u32;
            normals.extend(local_normals);

            let local_uvs: Vec<_> = reader
                .read_tex_coords(
                    materials[material as usize]
                        .base_color_tex_coord
                        .unwrap_or(0),
                )
                .map(|uvs| uvs.into_f32().map(Vec2::from_array).collect())
                .unwrap_or_default();
            let has_local_uvs = !local_uvs.is_empty();
            let uv_base = texcoords.len() as u32;
            texcoords.extend(local_uvs);
            let position_count = positions.len() as u32 - position_base;
            let indices: Vec<u32> = reader
                .read_indices()
                .map(|indices| indices.into_u32().collect())
                .unwrap_or_else(|| (0..position_count).collect());
            let primitive_indices = triangulate_indices(&indices, primitive.mode())?;
            if triangles.len().saturating_add(primitive_indices.len())
                > crate::obj_reference::MAX_TRIANGLES
            {
                return Err("GLB exceeds the one million triangle limit".into());
            }
            if !has_local_uvs
                && materials[material as usize].base_color_texture.is_some()
                && !warnings
                    .iter()
                    .any(|warning| warning.contains("without its requested texture coordinate set"))
            {
                warnings.push("A textured primitive without its requested texture coordinate set samples a single texture corner in the viewport; the bundled GLB retains the source primitive.".into());
            }
            for triangle in primitive_indices {
                if triangle.iter().any(|index| *index >= position_count) {
                    return Err("GLB primitive contains an invalid index".into());
                }
                let p0 = positions[(position_base + triangle[0]) as usize];
                let p1 = positions[(position_base + triangle[1]) as usize];
                let p2 = positions[(position_base + triangle[2]) as usize];
                let face_normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                if face_normal.length_squared() < 0.5 {
                    continue;
                }
                let corners = triangle.map(|index| ObjIndex {
                    position: position_base + index,
                    texcoord: ((index as usize) < texcoords.len() - uv_base as usize)
                        .then_some(uv_base + index),
                    normal: ((index as usize) < normals.len() - normal_base as usize)
                        .then_some(normal_base + index),
                });
                triangles.push(ObjTriangle {
                    corners,
                    face_normal,
                    object,
                    group,
                    material: Some(material),
                });
            }
        }
    }
    for child in node.children() {
        visit_node(
            child, transform, blob, positions, texcoords, normals, objects, groups, materials,
            warnings, triangles,
        )?;
    }
    Ok(())
}

fn push_name(names: &mut Vec<String>, name: &str) -> u32 {
    let index = names.len() as u32;
    names.push(
        name.chars()
            .take(crate::obj_reference::MAX_NAME_CHARS)
            .collect(),
    );
    index
}

fn triangulate_indices(indices: &[u32], mode: gltf::mesh::Mode) -> Result<Vec<[u32; 3]>, String> {
    let mut triangles = Vec::new();
    match mode {
        gltf::mesh::Mode::Triangles => {
            if indices.len() % 3 != 0 {
                return Err("GLB triangle index count is not divisible by three".into());
            }
            triangles.extend(indices.chunks_exact(3).map(|tri| [tri[0], tri[1], tri[2]]));
        }
        gltf::mesh::Mode::TriangleStrip => {
            for index in 2..indices.len() {
                triangles.push(if index % 2 == 0 {
                    [indices[index - 2], indices[index - 1], indices[index]]
                } else {
                    [indices[index - 1], indices[index - 2], indices[index]]
                });
            }
        }
        gltf::mesh::Mode::TriangleFan => {
            for index in 2..indices.len() {
                triangles.push([indices[0], indices[index - 1], indices[index]]);
            }
        }
        _ => return Ok(Vec::new()),
    }
    Ok(triangles)
}
