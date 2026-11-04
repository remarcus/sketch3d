//! Bounded Wavefront OBJ reference geometry import.
//!
//! The importer accepts positions, optional vertex normals/UVs, named objects
//! and groups, polygon faces, and an explicit relative MTL sidecar. Polygon
//! faces are triangulated with projected ear clipping; malformed, non-planar,
//! or self-intersecting faces fail the whole import. The supported material
//! subset is `newmtl`, diffuse `Kd`, and opacity `d`/`Tr`. Recognized texture
//! maps and other common MTL properties are ignored and listed in `warnings`.
//! MTL filenames are whitespace-delimited and cannot contain spaces. A declared
//! MTL file that is missing or escapes the OBJ directory is an error. An OBJ with
//! no `mtllib` uses a default white material. Importing an OBJ does not imply
//! that its triangles become editable strokes or GLB output.

use glam::{Vec2, Vec3};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const MAX_OBJ_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_MTL_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_POSITIONS: usize = 1_000_000;
pub const MAX_TEXCOORDS: usize = 1_000_000;
pub const MAX_NORMALS: usize = 1_000_000;
pub const MAX_FACES: usize = 500_000;
pub const MAX_TRIANGLES: usize = 1_000_000;
pub const MAX_MATERIALS: usize = 256;
pub const MAX_FACE_CORNERS: usize = 256;
pub const MAX_NAME_CHARS: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObjSourceFormat {
    #[default]
    Obj,
    Glb,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjMaterial {
    pub name: String,
    /// Linearized as written by MTL; callers can convert color space at draw time.
    pub diffuse: [f32; 3],
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjIndex {
    pub position: u32,
    pub texcoord: Option<u32>,
    pub normal: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjTriangle {
    pub corners: [ObjIndex; 3],
    /// Face normal used where a corner does not reference an explicit `vn`.
    pub face_normal: Vec3,
    pub object: u32,
    pub group: u32,
    pub material: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub texcoord: Option<Vec2>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjModel {
    pub source_format: ObjSourceFormat,
    pub positions: Vec<Vec3>,
    pub texcoords: Vec<Vec2>,
    pub normals: Vec<Vec3>,
    pub objects: Vec<String>,
    pub groups: Vec<String>,
    pub materials: Vec<ObjMaterial>,
    pub triangles: Vec<ObjTriangle>,
    pub bounds: [Vec3; 2],
    /// MTL features outside the supported diffuse-color/opacity subset.
    pub warnings: Vec<String>,
    /// Canonical source paths retained for UI diagnostics; callers should not
    /// depend on them after import unless they explicitly reload the resource.
    pub source_obj: PathBuf,
    pub source_materials: Vec<PathBuf>,
}

impl ObjModel {
    /// Resolve a triangle to render-ready vertices. Missing normals use the
    /// face normal. UVs remain optional and are not synthesized.
    pub fn render_vertices(&self, triangle: &ObjTriangle) -> Result<[RenderVertex; 3], String> {
        let mut result = [
            RenderVertex {
                position: Vec3::ZERO,
                normal: triangle.face_normal,
                texcoord: None,
            },
            RenderVertex {
                position: Vec3::ZERO,
                normal: triangle.face_normal,
                texcoord: None,
            },
            RenderVertex {
                position: Vec3::ZERO,
                normal: triangle.face_normal,
                texcoord: None,
            },
        ];
        for (out, index) in result.iter_mut().zip(triangle.corners) {
            out.position = *self
                .positions
                .get(index.position as usize)
                .ok_or("OBJ position index became invalid")?;
            if let Some(normal) = index.normal {
                out.normal = *self
                    .normals
                    .get(normal as usize)
                    .ok_or("OBJ normal index became invalid")?;
            }
            if let Some(uv) = index.texcoord {
                out.texcoord = Some(
                    *self
                        .texcoords
                        .get(uv as usize)
                        .ok_or("OBJ UV index became invalid")?,
                );
            }
        }
        Ok(result)
    }

    pub fn material(&self, triangle: &ObjTriangle) -> Option<&ObjMaterial> {
        triangle
            .material
            .and_then(|index| self.materials.get(index as usize))
    }
}

#[derive(Clone)]
struct PendingTriangle {
    corners: [ObjIndex; 3],
    face_normal: Vec3,
    object: u32,
    group: u32,
    material_name: Option<String>,
}

/// Read and validate an OBJ plus its explicitly referenced MTL sidecars.
pub fn load(path: &Path) -> Result<ObjModel, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Cannot inspect OBJ file: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_OBJ_BYTES {
        return Err("OBJ must be a regular file no larger than 64 MiB".into());
    }
    let canonical_obj = path
        .canonicalize()
        .map_err(|error| format!("Cannot resolve OBJ file: {error}"))?;
    let bytes = read_bounded(&canonical_obj, MAX_OBJ_BYTES, "OBJ")?;
    parse(&bytes, &canonical_obj)
}

/// Parse already bounded UTF-8 OBJ bytes. Relative MTL references resolve
/// against the parent directory of `source_obj` and cannot escape it.
pub fn parse(bytes: &[u8], source_obj: &Path) -> Result<ObjModel, String> {
    if bytes.len() as u64 > MAX_OBJ_BYTES {
        return Err("OBJ source exceeds the 64 MiB limit".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "OBJ must be UTF-8 text")?;
    let obj_path = source_obj
        .canonicalize()
        .unwrap_or_else(|_| source_obj.to_owned());
    let directory = obj_path
        .parent()
        .ok_or("OBJ path has no parent directory")?;
    let mut positions = Vec::new();
    let mut texcoords = Vec::new();
    let mut normals = Vec::new();
    let mut objects = Vec::new();
    let mut groups = Vec::new();
    let mut object_ids = HashMap::new();
    let mut group_ids = HashMap::new();
    let mut current_object = name_id("default", &mut objects, &mut object_ids)?;
    let mut current_group = name_id("default", &mut groups, &mut group_ids)?;
    let mut current_material: Option<String> = None;
    let mut material_libraries = Vec::new();
    let mut pending = Vec::new();
    let mut face_count = 0usize;

    for (line_index, raw_line) in text.lines().enumerate() {
        if raw_line.len() > 64 * 1024 {
            return Err(format!("OBJ line {} exceeds 64 KiB", line_index + 1));
        }
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let directive = fields.next().unwrap_or("");
        let rest: Vec<&str> = fields.collect();
        match directive {
            "v" => {
                if positions.len() >= MAX_POSITIONS {
                    return Err("OBJ exceeds one million positions".into());
                }
                if rest.len() < 3 || rest.len() > 4 {
                    return Err(line_error(line_index, "vertex needs x y z and optional w"));
                }
                let mut values = parse_floats(&rest, line_index, 3)?;
                let weight = if values.len() == 4 {
                    values.pop().unwrap()
                } else {
                    1.0
                };
                if weight.abs() <= 1e-12 {
                    return Err(line_error(
                        line_index,
                        "homogeneous vertex weight cannot be zero",
                    ));
                }
                let position =
                    Vec3::new(values[0] / weight, values[1] / weight, values[2] / weight);
                validate_position(position).map_err(|error| line_error(line_index, &error))?;
                positions.push(position);
            }
            "vt" => {
                if texcoords.len() >= MAX_TEXCOORDS {
                    return Err("OBJ exceeds one million texture coordinates".into());
                }
                if rest.len() < 2 || rest.len() > 3 {
                    return Err(line_error(
                        line_index,
                        "texture coordinate needs u v and optional w",
                    ));
                }
                let values = parse_floats(&rest, line_index, 2)?;
                texcoords.push(Vec2::new(values[0], values[1]));
            }
            "vn" => {
                if normals.len() >= MAX_NORMALS {
                    return Err("OBJ exceeds one million normals".into());
                }
                if rest.len() != 3 {
                    return Err(line_error(line_index, "normal needs x y z"));
                }
                let values = parse_floats(&rest, line_index, 3)?;
                let normal = Vec3::new(values[0], values[1], values[2]);
                let Some(normal) = normal.try_normalize() else {
                    return Err(line_error(line_index, "normal must be non-zero"));
                };
                normals.push(normal);
            }
            "o" => {
                let name = joined_name(&rest, "default")?;
                current_object = name_id(&name, &mut objects, &mut object_ids)?;
            }
            "g" => {
                let name = joined_name(&rest, "default")?;
                current_group = name_id(&name, &mut groups, &mut group_ids)?;
            }
            "usemtl" => current_material = Some(joined_name(&rest, "")?),
            "mtllib" => {
                if rest.is_empty() {
                    return Err(line_error(
                        line_index,
                        "mtllib needs a relative MTL filename",
                    ));
                }
                for name in rest {
                    let path = resolve_material_path(directory, name)?;
                    if !material_libraries.contains(&path) {
                        material_libraries.push(path);
                    }
                }
            }
            "f" => {
                face_count += 1;
                if face_count > MAX_FACES {
                    return Err("OBJ exceeds 500,000 faces".into());
                }
                if rest.len() < 3 {
                    return Err(line_error(line_index, "face needs at least three corners"));
                }
                let mut polygon = Vec::with_capacity(rest.len());
                for token in rest {
                    polygon.push(
                        parse_index(token, positions.len(), texcoords.len(), normals.len())
                            .map_err(|error| line_error(line_index, &error))?,
                    );
                }
                let triangulated = triangulate(&polygon, &positions)
                    .map_err(|error| line_error(line_index, &error))?;
                if pending.len().saturating_add(triangulated.len()) > MAX_TRIANGLES {
                    return Err("OBJ exceeds one million output triangles".into());
                }
                for tri in triangulated {
                    let [a, b, c] = [polygon[tri[0]], polygon[tri[1]], polygon[tri[2]]];
                    let face_normal = (positions[b.position as usize]
                        - positions[a.position as usize])
                        .cross(positions[c.position as usize] - positions[a.position as usize])
                        .try_normalize()
                        .ok_or_else(|| {
                            line_error(line_index, "face produced a degenerate triangle")
                        })?;
                    pending.push(PendingTriangle {
                        corners: [a, b, c],
                        face_normal,
                        object: current_object,
                        group: current_group,
                        material_name: current_material.clone(),
                    });
                }
            }
            // Lines, points, smoothing groups, and free-form extensions are not
            // geometry in this reference surface importer.
            _ => {}
        }
    }
    if positions.is_empty() || pending.is_empty() {
        return Err("OBJ must contain positions and at least one polygon face".into());
    }

    let mut materials = Vec::new();
    let mut warnings = Vec::new();
    for library in &material_libraries {
        let metadata = fs::metadata(library)
            .map_err(|error| format!("Cannot inspect MTL ‘{}’: {error}", library.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_MTL_BYTES {
            return Err(format!(
                "MTL ‘{}’ must be a regular file no larger than 8 MiB",
                library.display()
            ));
        }
        let mtl = read_bounded(library, MAX_MTL_BYTES, "MTL")?;
        let text = std::str::from_utf8(&mtl)
            .map_err(|_| format!("MTL ‘{}’ must be UTF-8 text", library.display()))?;
        parse_mtl(text, &mut materials, &mut warnings)?;
    }
    let material_ids: HashMap<String, u16> = materials
        .iter()
        .enumerate()
        .map(|(index, material)| (material.name.clone(), index as u16))
        .collect();
    let mut triangles = Vec::with_capacity(pending.len());
    for triangle in pending {
        let material = match triangle.material_name {
            None => None,
            Some(name) => Some(*material_ids.get(&name).ok_or_else(|| {
                format!("OBJ references material ‘{name}’, but no declared MTL defines it")
            })?),
        };
        triangles.push(ObjTriangle {
            corners: triangle.corners,
            face_normal: triangle.face_normal,
            object: triangle.object,
            group: triangle.group,
            material,
        });
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
        source_format: ObjSourceFormat::Obj,
        positions,
        texcoords,
        normals,
        objects,
        groups,
        materials,
        triangles,
        bounds: [min, max],
        warnings,
        source_obj: obj_path,
        source_materials: material_libraries,
    })
}

fn read_bounded(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>, String> {
    let file =
        fs::File::open(path).map_err(|error| format!("Cannot read {label} file: {error}"))?;
    let mut bytes = Vec::with_capacity(limit.min(1024 * 1024) as usize);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read {label} file: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label} exceeds the configured size limit"));
    }
    Ok(bytes)
}

fn parse_mtl(
    text: &str,
    materials: &mut Vec<ObjMaterial>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let mut current: Option<usize> = None;
    for (line_index, raw_line) in text.lines().enumerate() {
        if raw_line.len() > 64 * 1024 {
            return Err(format!("MTL line {} exceeds 64 KiB", line_index + 1));
        }
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let directive = fields.next().unwrap_or("");
        let rest: Vec<&str> = fields.collect();
        match directive {
            "newmtl" => {
                let name = joined_name(&rest, "")?;
                if materials.iter().any(|material| material.name == name) {
                    return Err(format!("Duplicate MTL material ‘{name}’"));
                }
                if materials.len() >= MAX_MATERIALS {
                    return Err("MTL exceeds 256 materials".into());
                }
                materials.push(ObjMaterial {
                    name,
                    diffuse: [1.0; 3],
                    opacity: 1.0,
                });
                current = Some(materials.len() - 1);
            }
            "Kd" => {
                let index =
                    current.ok_or_else(|| line_error(line_index, "Kd appears before newmtl"))?;
                if rest.len() != 3 {
                    return Err(line_error(line_index, "Kd needs three components"));
                }
                let values = parse_floats(&rest, line_index, 3)?;
                if values.iter().any(|value| !(0.0..=1.0).contains(value)) {
                    return Err(line_error(
                        line_index,
                        "Kd components must be between 0 and 1",
                    ));
                }
                materials[index].diffuse = [values[0], values[1], values[2]];
            }
            "d" | "Tr" => {
                let index = current
                    .ok_or_else(|| line_error(line_index, "opacity appears before newmtl"))?;
                if rest.len() != 1 {
                    return Err(line_error(line_index, "opacity needs one component"));
                }
                let value = rest[0]
                    .parse::<f32>()
                    .map_err(|_| line_error(line_index, "invalid opacity"))?;
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(line_error(line_index, "opacity must be between 0 and 1"));
                }
                materials[index].opacity = if directive == "Tr" {
                    1.0 - value
                } else {
                    value
                };
            }
            "map_Kd" | "map_Ka" | "map_Ks" | "map_d" | "bump" | "map_Bump" | "norm" => {
                push_warning(
                    warnings,
                    format!("Unsupported MTL texture directive ‘{directive}’ was ignored"),
                );
            }
            "Ka" | "Ks" | "Ns" | "Ni" | "illum" | "Tf" | "Ke" => {
                push_warning(
                    warnings,
                    format!("Unsupported MTL property ‘{directive}’ was ignored"),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn push_warning(warnings: &mut Vec<String>, message: String) {
    if warnings.len() < 64 && !warnings.contains(&message) {
        warnings.push(message);
    }
}

fn resolve_material_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    let relative = Path::new(&normalized);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "MTL path ‘{name}’ must be relative and stay within the OBJ folder"
        ));
    }
    let root = directory
        .canonicalize()
        .map_err(|error| format!("Cannot resolve OBJ folder: {error}"))?;
    let candidate = root.join(relative).canonicalize().map_err(|error| {
        format!("Declared MTL ‘{name}’ is missing or cannot be resolved: {error}")
    })?;
    if !candidate.starts_with(&root) {
        return Err(format!("MTL path ‘{name}’ resolves outside the OBJ folder"));
    }
    Ok(candidate)
}

fn parse_index(
    token: &str,
    positions: usize,
    texcoords: usize,
    normals: usize,
) -> Result<ObjIndex, String> {
    let parts: Vec<&str> = token.split('/').collect();
    if parts.is_empty() || parts.len() > 3 || parts[0].is_empty() {
        return Err(format!("invalid face corner ‘{token}’"));
    }
    let position = resolve_index(parts[0], positions, "position")?;
    let texcoord = if parts.len() >= 2 && !parts[1].is_empty() {
        Some(resolve_index(parts[1], texcoords, "texture coordinate")?)
    } else {
        None
    };
    let normal = if parts.len() == 3 && !parts[2].is_empty() {
        Some(resolve_index(parts[2], normals, "normal")?)
    } else {
        None
    };
    Ok(ObjIndex {
        position,
        texcoord,
        normal,
    })
}

fn resolve_index(value: &str, count: usize, label: &str) -> Result<u32, String> {
    let raw = value
        .parse::<i64>()
        .map_err(|_| format!("invalid {label} index ‘{value}’"))?;
    if raw == 0 {
        return Err(format!(
            "OBJ {label} indices are one-based and cannot be zero"
        ));
    }
    let index = if raw > 0 { raw - 1 } else { count as i64 + raw };
    if index < 0 || index >= count as i64 {
        return Err(format!("OBJ {label} index ‘{value}’ is out of range"));
    }
    u32::try_from(index).map_err(|_| format!("OBJ {label} index is too large"))
}

fn triangulate(polygon: &[ObjIndex], positions: &[Vec3]) -> Result<Vec<[usize; 3]>, String> {
    if polygon.len() == 3 {
        return Ok(vec![[0, 1, 2]]);
    }
    if polygon.len() > MAX_FACE_CORNERS {
        return Err(format!("face exceeds {MAX_FACE_CORNERS} corners"));
    }
    let points: Vec<Vec3> = polygon
        .iter()
        .map(|corner| positions[corner.position as usize])
        .collect();
    let normal = newell_normal(&points).ok_or("polygon is degenerate")?;
    let origin = points[0];
    let extent = points
        .iter()
        .map(|p| p.distance(origin))
        .fold(0.0_f32, f32::max);
    if points
        .iter()
        .any(|point| (point - origin).dot(normal).abs() > extent.max(1.0) * 1e-4)
    {
        return Err("non-planar polygon face is unsupported".into());
    }
    let axis = dominant_axis(normal);
    let projected: Vec<Vec2> = points
        .iter()
        .copied()
        .map(|point| project(point, axis))
        .collect();
    let epsilon = (extent as f64).max(1.0).powi(2) * 1e-12;
    if projected
        .windows(2)
        .any(|pair| distance_squared(pair[0], pair[1]) <= epsilon)
        || distance_squared(*projected.first().unwrap(), *projected.last().unwrap()) <= epsilon
    {
        return Err("polygon contains repeated adjacent corners".into());
    }
    if has_self_intersection(&projected, epsilon) {
        return Err("self-intersecting polygon face is unsupported".into());
    }
    let area = signed_area(&projected);
    if area.abs() <= epsilon {
        return Err("polygon has zero projected area".into());
    }
    let winding = area.signum();
    let mut remaining: Vec<usize> = (0..projected.len()).collect();
    let mut triangles = Vec::with_capacity(projected.len() - 2);
    while remaining.len() > 3 {
        let mut ear = None;
        for cursor in 0..remaining.len() {
            let a = remaining[(cursor + remaining.len() - 1) % remaining.len()];
            let b = remaining[cursor];
            let c = remaining[(cursor + 1) % remaining.len()];
            if orient2d(projected[a], projected[b], projected[c]) * winding <= epsilon {
                continue;
            }
            if remaining.iter().copied().any(|p| {
                p != a
                    && p != b
                    && p != c
                    && point_in_triangle(
                        projected[p],
                        projected[a],
                        projected[b],
                        projected[c],
                        winding,
                        epsilon,
                    )
            }) {
                continue;
            }
            ear = Some((cursor, [a, b, c]));
            break;
        }
        let Some((cursor, triangle)) = ear else {
            return Err("polygon could not be triangulated safely".into());
        };
        triangles.push(triangle);
        remaining.remove(cursor);
    }
    triangles.push([remaining[0], remaining[1], remaining[2]]);
    Ok(triangles)
}

fn newell_normal(points: &[Vec3]) -> Option<Vec3> {
    let mut normal = Vec3::ZERO;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        normal.x += (a.y - b.y) * (a.z + b.z);
        normal.y += (a.z - b.z) * (a.x + b.x);
        normal.z += (a.x - b.x) * (a.y + b.y);
    }
    normal.try_normalize()
}
fn dominant_axis(normal: Vec3) -> usize {
    let n = normal.abs();
    if n.x >= n.y && n.x >= n.z {
        0
    } else if n.y >= n.z {
        1
    } else {
        2
    }
}
fn project(point: Vec3, axis: usize) -> Vec2 {
    match axis {
        0 => Vec2::new(point.y, point.z),
        1 => Vec2::new(point.x, point.z),
        _ => Vec2::new(point.x, point.y),
    }
}
fn signed_area(points: &[Vec2]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        area += a.x as f64 * b.y as f64 - b.x as f64 * a.y as f64;
    }
    area * 0.5
}
fn orient2d(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    (b.x as f64 - a.x as f64) * (c.y as f64 - a.y as f64)
        - (b.y as f64 - a.y as f64) * (c.x as f64 - a.x as f64)
}
fn distance_squared(a: Vec2, b: Vec2) -> f64 {
    let x = a.x as f64 - b.x as f64;
    let y = a.y as f64 - b.y as f64;
    x * x + y * y
}
fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2, winding: f64, epsilon: f64) -> bool {
    orient2d(a, b, p) * winding >= -epsilon
        && orient2d(b, c, p) * winding >= -epsilon
        && orient2d(c, a, p) * winding >= -epsilon
}
fn has_self_intersection(points: &[Vec2], epsilon: f64) -> bool {
    let count = points.len();
    for a in 0..count {
        let a2 = (a + 1) % count;
        for b in a + 1..count {
            let b2 = (b + 1) % count;
            if a == b || a2 == b || b2 == a {
                continue;
            }
            if segments_intersect(points[a], points[a2], points[b], points[b2], epsilon) {
                return true;
            }
        }
    }
    false
}
fn segments_intersect(a: Vec2, b: Vec2, c: Vec2, d: Vec2, epsilon: f64) -> bool {
    let ab_c = orient2d(a, b, c);
    let ab_d = orient2d(a, b, d);
    let cd_a = orient2d(c, d, a);
    let cd_b = orient2d(c, d, b);
    if ((ab_c > epsilon && ab_d < -epsilon) || (ab_c < -epsilon && ab_d > epsilon))
        && ((cd_a > epsilon && cd_b < -epsilon) || (cd_a < -epsilon && cd_b > epsilon))
    {
        return true;
    }
    (ab_c.abs() <= epsilon && on_segment(a, b, c, epsilon))
        || (ab_d.abs() <= epsilon && on_segment(a, b, d, epsilon))
        || (cd_a.abs() <= epsilon && on_segment(c, d, a, epsilon))
        || (cd_b.abs() <= epsilon && on_segment(c, d, b, epsilon))
}
fn on_segment(a: Vec2, b: Vec2, p: Vec2, epsilon: f64) -> bool {
    let min_x = (a.x.min(b.x) - epsilon as f32)..=(a.x.max(b.x) + epsilon as f32);
    let min_y = (a.y.min(b.y) - epsilon as f32)..=(a.y.max(b.y) + epsilon as f32);
    min_x.contains(&p.x) && min_y.contains(&p.y)
}

fn parse_floats(fields: &[&str], line_index: usize, min_count: usize) -> Result<Vec<f32>, String> {
    if fields.len() < min_count {
        return Err(line_error(line_index, "not enough numeric fields"));
    }
    fields
        .iter()
        .map(|field| {
            let value = field
                .parse::<f32>()
                .map_err(|_| line_error(line_index, "invalid numeric field"))?;
            if !value.is_finite() {
                return Err(line_error(line_index, "non-finite numeric field"));
            }
            Ok(value)
        })
        .collect()
}
fn validate_position(position: Vec3) -> Result<(), String> {
    if !position.is_finite() || position.abs().max_element() > 1_000_000.0 {
        Err("position is outside the supported 1,000,000-unit bound".into())
    } else {
        Ok(())
    }
}
fn joined_name(fields: &[&str], fallback: &str) -> Result<String, String> {
    if fields.is_empty() {
        return Ok(fallback.to_owned());
    }
    let name = fields.join(" ");
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err("OBJ/MTL name is too long or contains control characters".into());
    }
    Ok(name)
}
fn name_id(
    name: &str,
    names: &mut Vec<String>,
    ids: &mut HashMap<String, u32>,
) -> Result<u32, String> {
    if let Some(index) = ids.get(name) {
        return Ok(*index);
    }
    if names.len() >= u32::MAX as usize {
        return Err("OBJ has too many named groups".into());
    }
    let index = names.len() as u32;
    names.push(name.to_owned());
    ids.insert(name.to_owned(), index);
    Ok(index)
}
fn line_error(line_index: usize, message: &str) -> String {
    format!("OBJ/MTL line {}: {message}", line_index + 1)
}
