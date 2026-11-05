pub use crate::dynamics::{
    AirbrushSettings, DabColorVariation, DabSettings, Dynamics, PressureMapping, StrandSettings,
};
use crate::texture::TextureSettings;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[path = "guide_resources.rs"]
pub mod guide_resources;
#[path = "model_references.rs"]
pub mod model_references;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushKind {
    #[default]
    Round,
    Ribbon,
    Chisel,
}

/// Bounded polygon profiles for the ordinary round tube brush.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TubeProfile {
    #[default]
    Round,
    Square,
    Triangle,
    Star,
}
impl TubeProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Round => "Round",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Star => "Star",
        }
    }
}

/// How a stroke responds to scene lighting and exports to mesh formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeMaterial {
    /// Receives scene lighting and casts shadows.
    #[default]
    Shaded,
    /// Keeps its stored color regardless of scene lighting and does not cast shadows.
    Unlit,
    /// Unlit stored color with a fixed emissive contribution.
    Emissive,
    /// Erases previously composited canvas pixels within the stroke's coverage.
    Cutout,
}

pub const MAX_BRUSH_TIP_VARIANTS: usize = 8;

/// One content-addressed image in an optional multi-tip brush set.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushTipVariant {
    pub asset_id: u64,
    pub size: [u32; 2],
    pub aspect: f32,
    #[serde(default = "default_tip_variant_weight")]
    pub weight: u8,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
}

fn default_tip_variant_weight() -> u8 {
    1
}

impl BrushTipVariant {
    pub fn is_valid(self) -> bool {
        self.asset_id != 0
            && self.size[0] > 0
            && self.size[0] <= crate::brush_tip::MAX_DIMENSION
            && self.size[1] > 0
            && self.size[1] <= crate::brush_tip::MAX_DIMENSION
            && self.aspect.is_finite()
            && (1.0 / 512.0..=512.0).contains(&self.aspect)
            && (self.aspect - self.size[0] as f32 / self.size[1] as f32).abs() <= 1e-4
            && (1..=100).contains(&self.weight)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipSelectionMode {
    #[default]
    SeededRandom,
    Sequential,
}

/// Brush orientation is captured in world space when the stroke begins.
/// Thickness is a fraction of width, so uniform scaling only changes radius.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub kind: BrushKind,
    #[serde(default)]
    pub tube_profile: TubeProfile,
    /// Width, depth, and dot-axis scales for transformed profiled tubes.
    #[serde(default = "default_tube_profile_scale")]
    pub tube_profile_scale: Vec3,
    pub thickness: f32,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default)]
    pub softness: f32,
    pub angle_degrees: f32,
    pub normal: Vec3,
    pub tangent: Vec3,
    #[serde(default)]
    pub dynamics: Dynamics,
    #[serde(default)]
    pub dabs: DabSettings,
    /// Seeded per-mark color offsets applied after the stroke's color profile.
    #[serde(default)]
    pub dab_color_variation: DabColorVariation,
    #[serde(default)]
    pub strands: StrandSettings,
    #[serde(default)]
    pub flow: AirbrushSettings,
    /// Stable per-stroke random stream; drawing tools replace preset seeds.
    #[serde(default)]
    pub dab_seed: u64,
    /// Content fingerprint of an optional imported image-tip PNG.
    #[serde(default)]
    pub tip_asset_id: u64,
    /// Width-to-height ratio captured from the assigned tip image.
    #[serde(default = "default_tip_aspect")]
    pub tip_aspect: f32,
    /// Optional ordered set of image-tip variants. Empty retains the legacy
    /// single `tip_asset_id` representation.
    #[serde(default)]
    pub tip_variants: [Option<BrushTipVariant>; MAX_BRUSH_TIP_VARIANTS],
    #[serde(default)]
    pub tip_selection_mode: TipSelectionMode,
    /// Arc-length phase retained by fragments created through partial erasing.
    #[serde(default)]
    pub dab_distance_origin: f32,
    /// Original full-stroke arc length, retained by fragments for tapering.
    #[serde(default)]
    pub dab_total_length: f32,
    #[serde(default)]
    pub texture: TextureSettings,
}
impl Default for Brush {
    fn default() -> Self {
        Self {
            kind: BrushKind::Round,
            tube_profile: TubeProfile::default(),
            tube_profile_scale: Vec3::ONE,
            thickness: 0.2,
            opacity: 1.0,
            softness: 0.0,
            angle_degrees: 0.0,
            normal: Vec3::Z,
            tangent: Vec3::X,
            dynamics: Dynamics::default(),
            dabs: DabSettings::default(),
            dab_color_variation: DabColorVariation::default(),
            strands: StrandSettings::default(),
            flow: AirbrushSettings::default(),
            dab_seed: 0,
            tip_asset_id: 0,
            tip_aspect: 1.0,
            tip_variants: [None; MAX_BRUSH_TIP_VARIANTS],
            tip_selection_mode: TipSelectionMode::SeededRandom,
            dab_distance_origin: 0.0,
            dab_total_length: 0.0,
            texture: TextureSettings::default(),
        }
    }
}
fn default_opacity() -> f32 {
    1.0
}
fn default_tube_profile_scale() -> Vec3 {
    Vec3::ONE
}
impl Brush {
    pub fn is_valid(&self) -> bool {
        self.dynamics.is_valid()
            && self.tube_profile_scale.is_finite()
            && (0.05..=20.0).contains(&self.tube_profile_scale.x)
            && (0.05..=20.0).contains(&self.tube_profile_scale.y)
            && (0.05..=20.0).contains(&self.tube_profile_scale.z)
            && self.dabs.is_valid()
            && self.dab_color_variation.is_valid()
            && self.strands.is_valid()
            && self.flow.is_valid()
            && self.dab_distance_origin.is_finite()
            && self.dab_distance_origin >= 0.0
            && self.dab_total_length.is_finite()
            && self.dab_total_length >= 0.0
            && (self.dab_total_length == 0.0
                || self.dab_distance_origin <= self.dab_total_length + 1.0e-4)
            && self.texture.is_valid()
            && self.tip_aspect.is_finite()
            && (1.0 / 512.0..=512.0).contains(&self.tip_aspect)
            && self.tip_variants_are_valid()
            && (self.tip_asset_id == 0 || self.dabs.enabled)
            && (!self.strands.enabled || (!self.dabs.enabled && self.tip_asset_id == 0))
            && (!self.flow.enabled
                || (!self.strands.enabled && !self.dabs.enabled && self.tip_asset_id == 0))
            && self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
            && self.softness.is_finite()
            && (0.0..=1.0).contains(&self.softness)
            && self.thickness.is_finite()
            && (0.02..=1.0).contains(&self.thickness)
            && self.angle_degrees.is_finite()
            && (-180.0..=180.0).contains(&self.angle_degrees)
            && self.normal.is_finite()
            && (self.normal.length_squared() - 1.0).abs() <= 0.001
            && self.tangent.is_finite()
            && (self.tangent.length_squared() - 1.0).abs() <= 0.001
            && self.normal.dot(self.tangent).abs() <= 0.001
    }

    pub fn tip_variant_count(&self) -> usize {
        let explicit = self.tip_variants.iter().flatten().count();
        if explicit > 0 {
            explicit
        } else {
            usize::from(self.tip_asset_id != 0)
        }
    }

    pub fn explicit_tip_variants(&self) -> impl Iterator<Item = BrushTipVariant> + '_ {
        self.tip_variants.iter().flatten().copied()
    }

    pub fn tip_variant_ids(&self) -> Vec<u64> {
        if self.tip_variants[0].is_some() {
            self.explicit_tip_variants()
                .map(|variant| variant.asset_id)
                .collect()
        } else if self.tip_asset_id != 0 {
            vec![self.tip_asset_id]
        } else {
            Vec::new()
        }
    }

    pub fn uses_tip_atlas(&self) -> bool {
        self.tip_variant_count() > 1
    }

    pub fn max_tip_inverse_aspect(&self) -> f32 {
        self.explicit_tip_variants()
            .map(|variant| variant.aspect.recip())
            .reduce(f32::max)
            .unwrap_or_else(|| self.tip_aspect.recip())
    }

    fn tip_variants_are_valid(&self) -> bool {
        let mut saw_empty = false;
        let mut ids = std::collections::HashSet::new();
        for (index, variant) in self.tip_variants.iter().enumerate() {
            if let Some(variant) = variant {
                if saw_empty || !variant.is_valid() || !ids.insert(variant.asset_id) {
                    return false;
                }
                if index == 0
                    && (variant.asset_id != self.tip_asset_id
                        || (variant.aspect - self.tip_aspect).abs() > 1e-4)
                {
                    return false;
                }
            } else {
                saw_empty = true;
            }
        }
        let explicit = self.tip_variant_count() > 1;
        !explicit || self.tip_variants[0].is_some()
    }
}
fn default_tip_aspect() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub position: Vec3,
    pub pressure: f32,
    #[serde(default)]
    pub tilt: Option<Vec3>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub time_ms: u32,
}
fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}
pub const MAX_FLOW_DURATION_MS: u32 = 5 * 60 * 1000;
impl Point {
    pub fn is_valid(&self) -> bool {
        self.position.is_finite()
            && self.position.abs().max_element() <= 10000.0
            && self.pressure.is_finite()
            && (0.0..=1.0).contains(&self.pressure)
            && self.tilt.is_none_or(|tilt| {
                tilt.is_finite() && tilt.length_squared() > 1.0e-8 && tilt.length_squared() <= 1.002
            })
            && self.time_ms <= MAX_FLOW_DURATION_MS
    }
}

impl Stroke {
    pub fn point_thickness_scale(&self, index: usize) -> f32 {
        self.point_thickness
            .as_ref()
            .and_then(|values| values.get(index).copied())
            .unwrap_or(1.0)
    }

    pub fn point_thickness_is_valid(&self) -> bool {
        self.point_thickness.as_ref().is_none_or(|values| {
            values.len() == self.points.len()
                && values
                    .iter()
                    .all(|value| value.is_finite() && (0.05..=20.0).contains(value))
        })
    }

    pub fn point_colors_are_valid(&self) -> bool {
        self.point_colors.as_ref().is_none_or(|values| {
            values.len() == self.points.len()
                && self.points.len() <= MAX_LOCAL_COLOR_SAMPLES_PER_STROKE
                && values
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|component| component.is_finite() && (0.0..=1.0).contains(component))
        })
    }

    pub fn has_local_colors(&self) -> bool {
        self.point_colors
            .as_ref()
            .is_some_and(|values| values.iter().any(Option::is_some))
    }

    /// Base albedo at a normalized arc-length position, before local painting.
    pub fn base_color_at(&self, t: f32) -> [f32; 3] {
        self.gradient
            .map_or(self.color, |gradient| gradient.sample(t))
    }

    /// Resolve one sample to its visible albedo over the solid/gradient base.
    pub fn point_color_at(&self, index: usize, t: f32) -> [f32; 3] {
        self.point_colors
            .as_ref()
            .and_then(|values| values.get(index))
            .and_then(|value| *value)
            .unwrap_or_else(|| self.base_color_at(t))
    }
}

pub const MAX_LOCAL_COLOR_SAMPLES_PER_STROKE: usize = 65_536;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerBlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Add,
}
impl LayerBlendMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Add => "Add",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default)]
    pub blend_mode: LayerBlendMode,
    /// Restrict this layer's visible coverage to the immediately lower layer.
    #[serde(default)]
    pub clip_to_below: bool,
    /// Explicit mask source. The stable ID survives changes to layer order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_source: Option<u64>,
}

pub(crate) fn layer_mask_order(layers: &[Layer]) -> Result<Vec<u64>, &'static str> {
    use std::collections::{HashMap, HashSet};
    let sources: HashMap<_, _> = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| {
            (
                layer.id,
                layer
                    .mask_source
                    .or_else(|| (layer.clip_to_below && index > 0).then(|| layers[index - 1].id)),
            )
        })
        .collect();
    let needed: HashSet<_> = sources.values().flatten().copied().collect();
    let mut done = HashSet::new();
    let mut visiting = HashSet::new();
    let mut path = Vec::new();
    let mut order = Vec::new();
    for layer in layers {
        let mut current = Some(layer.id);
        while let Some(id) = current {
            if done.contains(&id) {
                break;
            }
            if !visiting.insert(id) {
                return Err("Layer masks cannot refer to themselves or form a cycle.");
            }
            path.push(id);
            current = *sources.get(&id).ok_or("Layer mask source is missing.")?;
        }
        while let Some(id) = path.pop() {
            visiting.remove(&id);
            done.insert(id);
            if needed.contains(&id) {
                order.push(id);
            }
        }
    }
    Ok(order)
}
/// A UI grouping for layers. Folder membership never changes a layer's stable
/// ID, animation track, or position in the flat compositing stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerFolder {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub layer_ids: Vec<u64>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default = "default_folder_visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
}
fn default_folder_visible() -> bool {
    true
}
pub fn default_layer_id() -> u64 {
    1
}
pub fn layers_default() -> Vec<Layer> {
    vec![Layer {
        id: 1,
        name: "Layer 1".into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: LayerBlendMode::Normal,
        mask_source: None,
        clip_to_below: false,
    }]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Saved plane or cylinder that carries this stroke when its geometry changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide_attachment: Option<u64>,
    /// Bounded world-space triangle patch captured for image-tip and scattered
    /// dab placement on an imported model surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_surface: Option<ModelSurfacePatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<PlanarFill>,
    #[serde(default)]
    pub material: StrokeMaterial,
    #[serde(default)]
    pub group_id: Option<u64>,
    #[serde(default = "default_layer_id")]
    pub layer_id: u64,
    pub id: u64,
    pub color: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<StrokeGradient>,
    pub radius: f32,
    #[serde(default)]
    pub brush: Brush,
    pub points: Vec<Point>,
    /// Optional per-sample multiplier for brush thickness. `None` is the
    /// legacy identity profile and preserves pre-profile documents exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_thickness: Option<Vec<f32>>,
    /// Optional sparse per-sample color overrides. A painted sample stores its
    /// final linear RGB albedo; empty entries use `color` or `gradient`.
    /// Overrides preserve the stroke's existing material mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_colors: Option<Vec<Option<[f32; 3]>>>,
    /// Optional ordered, versioned modifiers evaluated without rewriting the
    /// stored centerline. `None` preserves the legacy rendering path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<crate::stroke_modifiers::StrokeModifierStack>,
    /// Exact controls for semantic curves and curved circle/ellipse tools.
    /// The sampled points remain authoritative for rendering and export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<CurveData>,
}

pub const MAX_MODEL_SURFACE_TRIANGLES_PER_STROKE: usize = 16_384;
pub const MAX_MODEL_SURFACE_TRIANGLES_PER_DOCUMENT: usize = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelSurfaceTriangle {
    pub source_id: u32,
    pub positions: [Vec3; 3],
    pub normal: Vec3,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelSurfacePatch {
    pub triangles: Vec<ModelSurfaceTriangle>,
}

impl ModelSurfacePatch {
    pub fn insert(&mut self, triangle: ModelSurfaceTriangle) -> Result<bool, String> {
        match self
            .triangles
            .binary_search_by_key(&triangle.source_id, |item| item.source_id)
        {
            Ok(_) => Ok(false),
            Err(index) => {
                if self.triangles.len() >= MAX_MODEL_SURFACE_TRIANGLES_PER_STROKE {
                    return Err("Model dab target exceeds the per-stroke triangle limit".into());
                }
                self.triangles.insert(index, triangle);
                Ok(true)
            }
        }
    }

    pub fn project(&self, point: Vec3, max_distance: f32) -> Option<(Vec3, Vec3, u32, f32)> {
        if !point.is_finite() || !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        self.triangles
            .iter()
            .filter_map(|triangle| {
                let nearest = closest_point_on_triangle(point, triangle.positions)?;
                let distance = nearest.distance(point);
                (distance <= max_distance).then_some((
                    nearest,
                    triangle.normal,
                    triangle.source_id,
                    distance,
                ))
            })
            .min_by(|a, b| a.3.total_cmp(&b.3))
    }

    /// Drop captured triangles that cannot contribute to marks near a retained
    /// stroke fragment. Keeping patches local prevents repeated partial erases
    /// from multiplying unrelated model geometry into every fragment.
    pub fn retain_near_points(&mut self, points: &[Point], margin: f32) {
        if points.is_empty() || !margin.is_finite() || margin < 0.0 || self.triangles.len() <= 1 {
            return;
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for point in points {
            min = min.min(point.position);
            max = max.max(point.position);
        }
        min -= Vec3::splat(margin);
        max += Vec3::splat(margin);
        let retained: Vec<_> = self
            .triangles
            .iter()
            .copied()
            .filter(|triangle| {
                let triangle_min = triangle.positions[0]
                    .min(triangle.positions[1])
                    .min(triangle.positions[2]);
                let triangle_max = triangle.positions[0]
                    .max(triangle.positions[1])
                    .max(triangle.positions[2]);
                (0..3)
                    .all(|axis| triangle_max[axis] >= min[axis] && triangle_min[axis] <= max[axis])
            })
            .collect();
        if !retained.is_empty() {
            self.triangles = retained;
        }
    }

    pub fn offset_tangent(
        &self,
        origin: Vec3,
        normal: Vec3,
        offset: Vec3,
        max_distance: f32,
    ) -> Option<(Vec3, Vec3)> {
        if !origin.is_finite()
            || !normal.is_finite()
            || !offset.is_finite()
            || !max_distance.is_finite()
            || max_distance <= 0.0
        {
            return None;
        }
        let normal = normal.try_normalize()?;
        let query = origin + offset;
        let ray_origin = query + normal * max_distance;
        let direction = -normal;
        self.triangles
            .iter()
            .filter_map(|triangle| {
                if triangle.normal.dot(normal) < 0.5 {
                    return None;
                }
                let distance = ray_triangle_distance(ray_origin, direction, triangle.positions)?;
                if !(0.0..=max_distance * 2.0).contains(&distance) {
                    return None;
                }
                let position = ray_origin + direction * distance;
                Some((position, triangle.normal, (distance - max_distance).abs()))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(position, normal, _)| (position, normal))
    }
}

fn ray_triangle_distance(origin: Vec3, direction: Vec3, triangle: [Vec3; 3]) -> Option<f32> {
    let edge_a = triangle[1] - triangle[0];
    let edge_b = triangle[2] - triangle[0];
    let p = direction.cross(edge_b);
    let determinant = edge_a.dot(p);
    if determinant.abs() < 1e-8 {
        return None;
    }
    let inverse = determinant.recip();
    let from_vertex = origin - triangle[0];
    let u = from_vertex.dot(p) * inverse;
    let q = from_vertex.cross(edge_a);
    let v = direction.dot(q) * inverse;
    if u < -1e-5 || v < -1e-5 || u + v > 1.0 + 1e-5 {
        return None;
    }
    let distance = edge_b.dot(q) * inverse;
    (distance.is_finite() && distance >= 0.0).then_some(distance)
}

fn closest_point_on_triangle(point: Vec3, triangle: [Vec3; 3]) -> Option<Vec3> {
    let [a, b, c] = triangle;
    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    let result = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = point - b;
        let d3 = ab.dot(bp);
        let d4 = ac.dot(bp);
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else {
            let vc = d1 * d4 - d3 * d2;
            if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                a + ab * (d1 / (d1 - d3))
            } else {
                let cp = point - c;
                let d5 = ab.dot(cp);
                let d6 = ac.dot(cp);
                if d6 >= 0.0 && d5 <= d6 {
                    c
                } else {
                    let vb = d5 * d2 - d1 * d6;
                    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                        a + ac * (d2 / (d2 - d6))
                    } else {
                        let va = d3 * d6 - d5 * d4;
                        if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
                            b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)))
                        } else {
                            let denominator = 1.0 / (va + vb + vc);
                            a + ab * (vb * denominator) + ac * (vc * denominator)
                        }
                    }
                }
            }
        }
    };
    result.is_finite().then_some(result)
}

/// Optional two-stop stroke color ramp. Colors use linear RGB and are
/// interpolated by normalized world-space arc length.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeGradient {
    pub start: [f32; 3],
    pub end: [f32; 3],
}

impl StrokeGradient {
    pub fn is_valid(self) -> bool {
        self.start
            .into_iter()
            .chain(self.end)
            .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
    }

    pub fn sample(self, t: f32) -> [f32; 3] {
        let t = t.clamp(0.0, 1.0);
        std::array::from_fn(|i| self.start[i] + (self.end[i] - self.start[i]) * t)
    }

    pub fn subrange(self, start: f32, end: f32) -> Self {
        Self {
            start: self.sample(start),
            end: self.sample(end),
        }
    }
}

/// Editable source data retained alongside a curve's sampled stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveData {
    pub anchors: Vec<CurveAnchor>,
    pub frame_origin: Vec3,
    pub frame_normal: Vec3,
    pub frame_tangent: Vec3,
    #[serde(default)]
    pub half_extent: Option<glam::Vec2>,
    pub tension: f32,
    /// Optional analytic target snapshot. When present, anchors and handles
    /// are stored in the target's unwrapped surface chart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_target: Option<crate::dynamics::DabSurfaceTarget>,
    /// Curved circle/ellipse dimensions retained for reopening shape controls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<CurvedShapeData>,
}

/// A non-recursive linked definition backed by regular source strokes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkedStrokeGroupDefinition {
    pub id: u64,
    pub source_stroke_ids: Vec<u64>,
    pub pivot: Vec3,
}

/// One transformed, baked view of a linked definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkedStrokeGroupInstance {
    pub id: u64,
    pub definition_id: u64,
    pub layer_id: u64,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: f32,
    /// Stable IDs for the regenerated strokes; removed when the instance is deleted.
    pub stroke_ids: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinkedStrokeGroups {
    #[serde(default)]
    pub definitions: Vec<LinkedStrokeGroupDefinition>,
    #[serde(default)]
    pub instances: Vec<LinkedStrokeGroupInstance>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwatchRole {
    Stroke,
    Fill,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentColorSwatch {
    pub id: u64,
    pub name: String,
    pub color: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrokeSwatchBinding {
    pub stroke_id: u64,
    pub swatch_id: u64,
    pub role: SwatchRole,
}

/// Named document colors and stable stroke bindings. Strokes without bindings
/// continue to use their literal stored colors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentColorSwatches {
    #[serde(default)]
    pub swatches: Vec<DocumentColorSwatch>,
    #[serde(default)]
    pub bindings: Vec<StrokeSwatchBinding>,
}

impl DocumentColorSwatches {
    pub fn swatch(&self, id: u64) -> Option<&DocumentColorSwatch> {
        self.swatches.iter().find(|swatch| swatch.id == id)
    }

    pub fn binding(&self, stroke_id: u64, role: SwatchRole) -> Option<StrokeSwatchBinding> {
        self.bindings
            .iter()
            .copied()
            .find(|binding| binding.stroke_id == stroke_id && binding.role == role)
    }

    /// Resolve linked colors into the literal base-color fields consumed by
    /// renderers and exporters. Unbound marks retain their stored colors.
    pub fn resolve_strokes(&self, strokes: &mut [Stroke]) {
        let colors: std::collections::HashMap<_, _> = self
            .swatches
            .iter()
            .map(|swatch| (swatch.id, swatch.color))
            .collect();
        for stroke in strokes {
            let role = if stroke.fill.is_some() {
                SwatchRole::Fill
            } else {
                SwatchRole::Stroke
            };
            if let Some(binding) = self.binding(stroke.id, role) {
                if let Some(color) = colors.get(&binding.swatch_id) {
                    stroke.color = *color;
                }
            }
        }
    }

    pub fn validate(&self, document: &Document) -> Result<(), String> {
        if self.swatches.len() > 512 || self.bindings.len() > 100_000 {
            return Err("Document color swatches exceed their item limits".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for swatch in &self.swatches {
            if swatch.id == 0
                || swatch.id == u64::MAX
                || !ids.insert(swatch.id)
                || swatch.name.trim() != swatch.name
                || swatch.name.is_empty()
                || swatch.name.chars().count() > 64
                || swatch.name.chars().any(char::is_control)
                || !names.insert(swatch.name.to_lowercase())
                || swatch
                    .color
                    .iter()
                    .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            {
                return Err("Invalid or duplicate document color swatch".into());
            }
        }
        let mut bound = std::collections::HashSet::new();
        for binding in &self.bindings {
            if binding.stroke_id == 0
                || binding.stroke_id == u64::MAX
                || !ids.contains(&binding.swatch_id)
                || !bound.insert((binding.stroke_id, binding.role))
            {
                return Err("Invalid or duplicate document color swatch binding".into());
            }
            let stroke = document
                .strokes
                .iter()
                .find(|stroke| stroke.id == binding.stroke_id)
                .or_else(|| {
                    document
                        .animation
                        .base_strokes
                        .as_ref()
                        .and_then(|strokes| {
                            strokes.iter().find(|stroke| stroke.id == binding.stroke_id)
                        })
                })
                .or_else(|| {
                    document.animation.keyframes.iter().find_map(|key| {
                        key.strokes
                            .iter()
                            .find(|stroke| stroke.id == binding.stroke_id)
                    })
                })
                .or_else(|| {
                    document.animation.layer_tracks.iter().find_map(|track| {
                        track.keyframes.iter().find_map(|key| {
                            key.strokes
                                .iter()
                                .find(|stroke| stroke.id == binding.stroke_id)
                        })
                    })
                })
                .ok_or("Swatch binding refers to a missing stroke")?;
            if matches!(binding.role, SwatchRole::Fill) != stroke.fill.is_some() {
                return Err("Swatch binding color role does not match its stroke".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveAnchor {
    pub position: Vec3,
    pub pressure: f32,
    pub incoming: Vec3,
    pub outgoing: Vec3,
    pub manual: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurvedShapeKind {
    Circle,
    Ellipse,
    Rectangle,
    RoundedRectangle,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvedShapeData {
    pub kind: CurvedShapeKind,
    /// Center in the captured target's local chart.
    pub center: Vec3,
    pub radius_u: f32,
    pub radius_v: f32,
    #[serde(default)]
    pub corner_radius: f32,
    #[serde(default)]
    pub aspect_locked: bool,
}

impl CurveData {
    pub fn is_valid(&self) -> bool {
        let unit = |v: Vec3| v.is_finite() && (v.length_squared() - 1.0).abs() <= 0.001;
        let common = self.anchors.len() >= 2
            && self.anchors.len() <= 256
            && self.frame_origin.is_finite()
            && self.frame_origin.abs().max_element() <= 10_000.0
            && unit(self.frame_normal)
            && unit(self.frame_tangent)
            && self.frame_normal.dot(self.frame_tangent).abs() <= 0.001
            && self.tension.is_finite()
            && (0.0..=2.0).contains(&self.tension)
            && self.half_extent.is_none_or(|extent| {
                extent.is_finite() && extent.min_element() > 0.0 && extent.max_element() <= 10_000.0
            });
        let anchors_valid = self.anchors.iter().all(|anchor| {
            anchor.position.is_finite()
                && anchor.position.abs().max_element() <= 10_000.0
                && anchor.pressure.is_finite()
                && (0.0..=1.0).contains(&anchor.pressure)
                && anchor.incoming.is_finite()
                && anchor.incoming.abs().max_element() <= 10_000.0
                && anchor.outgoing.is_finite()
                && anchor.outgoing.abs().max_element() <= 10_000.0
        });
        let shape_valid = self.shape.is_none_or(|shape| {
            shape.center.is_finite()
                && shape.center.abs().max_element() <= 10_000.0
                && shape.center.z.abs() <= 1.0e-4
                && (0.001..=10_000.0).contains(&shape.radius_u)
                && (0.001..=10_000.0).contains(&shape.radius_v)
                && shape.corner_radius.is_finite()
                && (0.0..=shape.radius_u.min(shape.radius_v)).contains(&shape.corner_radius)
                && match shape.kind {
                    CurvedShapeKind::Circle => {
                        shape.corner_radius == 0.0
                            && (shape.radius_u - shape.radius_v).abs() <= 1.0e-5
                    }
                    CurvedShapeKind::Ellipse | CurvedShapeKind::Rectangle => {
                        shape.corner_radius == 0.0
                    }
                    CurvedShapeKind::RoundedRectangle => shape.corner_radius > 0.0,
                }
        });
        common
            && anchors_valid
            && shape_valid
            && match self.surface_target {
                None => self.anchors.iter().all(|anchor| {
                    anchor.incoming.dot(self.frame_normal).abs() <= 0.01
                        && anchor.outgoing.dot(self.frame_normal).abs() <= 0.01
                        && (anchor.position - self.frame_origin)
                            .dot(self.frame_normal)
                            .abs()
                            <= 0.01
                }),
                Some(target) => {
                    let crate::dynamics::DabSurfaceTarget::Cylinder {
                        center,
                        rotation,
                        radius,
                        height,
                    } = target
                    else {
                        return false;
                    };
                    target.is_valid()
                        && self.half_extent.is_none()
                        && self.anchors.iter().all(|anchor| {
                            anchor.position.z.abs() <= 1.0e-4
                                && anchor.incoming.z.abs() <= 1.0e-4
                                && anchor.outgoing.z.abs() <= 1.0e-4
                                && anchor.position.y.abs() <= height * 0.5
                                && (anchor.position.y + anchor.incoming.y).abs() <= height * 0.5
                                && (anchor.position.y + anchor.outgoing.y).abs() <= height * 0.5
                                && anchor.position.x.abs() <= 1_000_000.0
                        })
                        && self.shape.is_none_or(|shape| {
                            shape.center.is_finite()
                                && shape.center.abs().max_element() <= 10_000.0
                                && shape.center.z.abs() <= 1.0e-4
                                && shape.center.y.abs() <= height * 0.5
                                && (0.001..=std::f32::consts::PI * radius).contains(&shape.radius_u)
                                && (0.001..=height).contains(&shape.radius_v)
                                && shape.center.y - shape.radius_v >= -height * 0.5
                                && shape.center.y + shape.radius_v <= height * 0.5
                                && (shape.kind != CurvedShapeKind::Circle
                                    || (shape.radius_u - shape.radius_v).abs() <= 1.0e-5)
                        })
                        && center.abs().max_element() <= 10_000.0
                        && rotation.is_finite()
                }
            }
    }

    /// Ensure sampled geometry agrees with its semantic controls. Planar curve
    /// endpoints are world positions; cylinder curve endpoints are chart
    /// positions. Curved shapes retain dimensions instead of endpoint anchors.
    pub fn samples_match_stroke(&self, stroke: &Stroke) -> bool {
        if !self.is_valid() {
            return false;
        }
        let (Some(first), Some(last)) = (stroke.points.first(), stroke.points.last()) else {
            return false;
        };
        if stroke.points.len() < 2 {
            return false;
        }
        match self.surface_target {
            None => {
                if let Some(shape) = self.shape {
                    let bitangent = self.frame_normal.cross(self.frame_tangent);
                    stroke.points.iter().all(|point| {
                        let offset = point.position - self.frame_origin;
                        let local =
                            glam::Vec2::new(offset.dot(self.frame_tangent), offset.dot(bitangent));
                        offset.dot(self.frame_normal).abs() <= 0.002
                            && (local.x - shape.center.x).abs() <= shape.radius_u + 0.002
                            && (local.y - shape.center.y).abs() <= shape.radius_v + 0.002
                    }) && (stroke.fill.is_some() || first.position.distance(last.position) <= 0.002)
                } else {
                    first.position == self.anchors[0].position
                        && last.position == self.anchors[self.anchors.len() - 1].position
                }
            }
            Some(crate::dynamics::DabSurfaceTarget::Cylinder {
                center,
                rotation,
                radius,
                height,
            }) => {
                let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
                let cylinder = crate::surface::Cylinder {
                    center,
                    rotation_degrees: glam::Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
                    radius,
                    height,
                };
                let samples_on_target = stroke.points.iter().all(|point| {
                    let local = cylinder.to_local(point.position);
                    (local.x.hypot(local.z) - radius).abs() <= 0.001
                        && local.y.abs() <= height * 0.5 + 0.001
                });
                let endpoints_match = self.shape.is_some()
                    || (cylinder
                        .point_from_parameter(self.anchors[0].position)
                        .is_some_and(|position| position.distance(first.position) <= 0.001)
                        && cylinder
                            .point_from_parameter(self.anchors[self.anchors.len() - 1].position)
                            .is_some_and(|position| position.distance(last.position) <= 0.001));
                samples_on_target && endpoints_match
            }
            Some(_) => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanarFill {
    /// Physical separation behind the captured guide normal, in world units.
    pub offset: f32,
    /// Optional color field in normalized coordinates of the current contour bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<FillGradient>,
    /// Optional portable image and fill-local UV mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<FillImageTexture>,
    /// Nested subtractive loops in world space, captured on the fill plane.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Vec3>>,
}

impl Default for PlanarFill {
    fn default() -> Self {
        Self {
            offset: 0.003,
            gradient: None,
            texture: None,
            holes: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FillImageTexture {
    pub asset_id: u64,
    #[serde(default = "default_fill_texture_scale")]
    pub scale: glam::Vec2,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub offset: glam::Vec2,
    #[serde(default = "default_fill_texture_repeat")]
    pub repeat: bool,
    #[serde(default = "default_fill_texture_tint")]
    pub tint: [f32; 3],
    #[serde(default = "default_true_f32")]
    pub opacity: f32,
}

fn default_fill_texture_scale() -> glam::Vec2 {
    glam::Vec2::ONE
}
fn default_fill_texture_tint() -> [f32; 3] {
    [1.0; 3]
}
fn default_fill_texture_repeat() -> bool {
    true
}
fn default_true_f32() -> f32 {
    1.0
}

impl FillImageTexture {
    pub fn is_valid(self) -> bool {
        self.asset_id != 0
            && self.asset_id != u64::MAX
            && self.scale.is_finite()
            && self.scale.min_element() >= 0.001
            && self.scale.max_element() <= 100.0
            && self.rotation.is_finite()
            && self.rotation.abs() <= std::f32::consts::TAU * 100.0
            && self.offset.is_finite()
            && self.offset.abs().max_element() <= 10_000.0
            && self
                .tint
                .into_iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
            && self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
    }

    pub fn map_uv(self, uv: glam::Vec2) -> glam::Vec2 {
        let scaled = (uv - glam::Vec2::splat(0.5)) * self.scale;
        let (sin, cos) = self.rotation.sin_cos();
        glam::Vec2::new(
            cos * scaled.x - sin * scaled.y,
            sin * scaled.x + cos * scaled.y,
        ) + glam::Vec2::splat(0.5)
            + self.offset
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillGradientKind {
    #[default]
    Linear,
    Radial,
}

/// Two-stop fill gradient. Coordinates are normalized in the fill's captured plane
/// and relative to the contour bounds, so the gradient follows contour transforms.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FillGradient {
    #[serde(default)]
    pub kind: FillGradientKind,
    pub start_color: [f32; 3],
    pub end_color: [f32; 3],
    pub start: glam::Vec2,
    pub end: glam::Vec2,
}

impl FillGradient {
    pub fn is_valid(self) -> bool {
        let color_valid = |color: [f32; 3]| {
            color
                .into_iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
        };
        self.start.is_finite()
            && self.end.is_finite()
            && self.start.min_element() >= 0.0
            && self.start.max_element() <= 1.0
            && self.end.min_element() >= 0.0
            && self.end.max_element() <= 1.0
            && self.start.distance(self.end) >= 0.01
            && color_valid(self.start_color)
            && color_valid(self.end_color)
    }

    pub fn sample(self, uv: glam::Vec2) -> [f32; 3] {
        let t = match self.kind {
            FillGradientKind::Linear => {
                let axis = self.end - self.start;
                (uv - self.start).dot(axis) / axis.length_squared().max(1e-8)
            }
            FillGradientKind::Radial => uv.distance(self.start) / self.start.distance(self.end),
        }
        .clamp(0.0, 1.0);
        std::array::from_fn(|i| self.start_color[i] + (self.end_color[i] - self.start_color[i]) * t)
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub color_swatches: DocumentColorSwatches,
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub layer_folders: Vec<LayerFolder>,
    pub active_layer: u64,
    pub version: u32,
    pub strokes: Vec<Stroke>,
    #[serde(default)]
    pub image_planes: Vec<crate::image_planes::ImagePlane>,
    #[serde(default)]
    pub active_image_plane: Option<u64>,
    #[serde(default)]
    pub guide_resources: Vec<guide_resources::GuideResource>,
    #[serde(default)]
    pub active_guide: Option<u64>,
    #[serde(default)]
    pub model_references: Vec<model_references::ObjReferenceResource>,
    #[serde(default)]
    pub active_model_reference: Option<u64>,
    /// Self-contained imported tip images keyed by their content fingerprint.
    #[serde(default)]
    pub brush_tip_assets: Vec<crate::brush_tip::BrushTipAsset>,
    #[serde(default)]
    pub linked_stroke_groups: LinkedStrokeGroups,
    /// Editable artwork keyframes. This is separate from construction history.
    #[serde(default)]
    pub animation: crate::animation::ArtworkAnimation,
    /// Pinned world-space measurements shown as drawing annotations.
    #[serde(default)]
    pub measurements: Vec<MeasurementAnnotation>,
}

/// A persisted distance and angle annotation captured in a planar frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasurementAnnotation {
    pub id: u64,
    pub start: glam::Vec3,
    pub end: glam::Vec3,
    pub axis_u: glam::Vec3,
    pub axis_v: glam::Vec3,
}

impl MeasurementAnnotation {
    pub fn is_valid(self) -> bool {
        self.id != 0
            && self.start.is_finite()
            && self.end.is_finite()
            && self.axis_u.is_finite()
            && self.axis_v.is_finite()
            && (self.axis_u.length_squared() - 1.0).abs() < 0.01
            && (self.axis_v.length_squared() - 1.0).abs() < 0.01
            && self.axis_u.dot(self.axis_v).abs() < 0.01
    }
}

impl Default for Document {
    fn default() -> Self {
        Self {
            color_swatches: DocumentColorSwatches::default(),
            layers: layers_default(),
            layer_folders: Vec::new(),
            active_layer: 1,
            version: 30,
            strokes: Vec::new(),
            image_planes: Vec::new(),
            active_image_plane: None,
            guide_resources: Vec::new(),
            active_guide: None,
            model_references: Vec::new(),
            active_model_reference: None,
            brush_tip_assets: Vec::new(),
            linked_stroke_groups: LinkedStrokeGroups::default(),
            animation: crate::animation::ArtworkAnimation::default(),
            measurements: Vec::new(),
        }
    }
}
impl Document {
    pub fn linked_instance_for_stroke(&self, stroke_id: u64) -> Option<&LinkedStrokeGroupInstance> {
        self.linked_stroke_groups
            .instances
            .iter()
            .find(|instance| instance.stroke_ids.contains(&stroke_id))
    }

    pub fn linked_generated_ids(&self) -> std::collections::HashSet<u64> {
        self.linked_stroke_groups
            .instances
            .iter()
            .flat_map(|instance| instance.stroke_ids.iter().copied())
            .collect()
    }

    pub fn has_linked_source(&self, stroke_id: u64) -> bool {
        self.linked_stroke_groups
            .definitions
            .iter()
            .any(|definition| definition.source_stroke_ids.contains(&stroke_id))
    }

    pub fn linked_affected_stroke_ids(
        &self,
        changed_source_ids: &std::collections::HashSet<u64>,
    ) -> std::collections::HashSet<u64> {
        let changed_definitions: std::collections::HashSet<u64> = self
            .linked_stroke_groups
            .definitions
            .iter()
            .filter(|definition| {
                definition
                    .source_stroke_ids
                    .iter()
                    .any(|id| changed_source_ids.contains(id))
            })
            .map(|definition| definition.id)
            .collect();
        let mut affected = changed_source_ids.clone();
        for instance in &self.linked_stroke_groups.instances {
            if changed_definitions.contains(&instance.definition_id) {
                affected.extend(instance.stroke_ids.iter().copied());
            }
        }
        affected
    }

    pub fn next_group_resource_id(&self) -> u64 {
        self.strokes
            .iter()
            .filter_map(|stroke| stroke.group_id)
            .chain(self.color_swatches.swatches.iter().map(|swatch| swatch.id))
            .chain(
                self.linked_stroke_groups
                    .definitions
                    .iter()
                    .map(|definition| definition.id),
            )
            .chain(
                self.linked_stroke_groups
                    .instances
                    .iter()
                    .map(|instance| instance.id),
            )
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// Rebuild baked instance strokes from their editable sources. The stored
    /// stroke IDs keep selection, renderer caches, and exported geometry stable.
    pub fn refresh_linked_instances(&mut self) -> Result<(), String> {
        let groups = &self.linked_stroke_groups;
        if groups.definitions.len() > 10_000 || groups.instances.len() > 10_000 {
            return Err("Linked group metadata exceeds the 10,000 item limit".into());
        }
        let generated_ids: Vec<u64> = groups
            .instances
            .iter()
            .flat_map(|instance| instance.stroke_ids.iter().copied())
            .collect();
        let generated_set: std::collections::HashSet<u64> = generated_ids.iter().copied().collect();
        if generated_ids.len() > 100_000
            || generated_ids.iter().any(|id| *id == 0 || *id == u64::MAX)
            || generated_set.len() != generated_ids.len()
        {
            return Err("Invalid or over-budget linked instance stroke IDs".into());
        }
        let mut definition_map = std::collections::HashMap::new();
        let mut group_resource_ids = std::collections::HashSet::new();
        let source_ids: std::collections::HashSet<u64> = groups
            .definitions
            .iter()
            .flat_map(|definition| definition.source_stroke_ids.iter().copied())
            .collect();
        for definition in &groups.definitions {
            if definition.id == 0
                || definition.id == u64::MAX
                || definition.source_stroke_ids.is_empty()
                || definition.source_stroke_ids.len() > 100_000
                || !definition.pivot.is_finite()
                || definition.pivot.abs().max_element() > 10_000.0
                || definition.source_stroke_ids.contains(&0)
                || definition
                    .source_stroke_ids
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != definition.source_stroke_ids.len()
                || !group_resource_ids.insert(definition.id)
                || definition_map.insert(definition.id, definition).is_some()
            {
                return Err("Invalid linked group definition".into());
            }
        }
        let mut referenced_definitions = std::collections::HashSet::new();
        for instance in &groups.instances {
            let definition = definition_map
                .get(&instance.definition_id)
                .ok_or("Linked instance refers to a missing definition")?;
            if instance.id == 0
                || instance.id == u64::MAX
                || !group_resource_ids.insert(instance.id)
                || instance.layer_id == 0
                || !instance.translation.is_finite()
                || instance.translation.abs().max_element() > 10_000.0
                || !instance.rotation.is_finite()
                || instance.rotation.length_squared() < 1e-8
                || !(0.001..=100.0).contains(&instance.scale)
                || instance.stroke_ids.len() != definition.source_stroke_ids.len()
            {
                return Err("Invalid linked group instance transform or ownership".into());
            }
            referenced_definitions.insert(instance.definition_id);
        }
        if referenced_definitions.len() != groups.definitions.len() {
            return Err("Linked group definition has no instances".into());
        }
        if source_ids.iter().any(|id| generated_set.contains(id)) {
            return Err("Linked group definitions cannot reference instance strokes".into());
        }
        let source_map: std::collections::HashMap<u64, Stroke> = self
            .strokes
            .iter()
            .filter(|stroke| !generated_set.contains(&stroke.id))
            .map(|stroke| (stroke.id, stroke.clone()))
            .collect();
        for definition in &groups.definitions {
            let mut layer_id = None;
            for id in &definition.source_stroke_ids {
                let source = source_map
                    .get(id)
                    .ok_or("Linked group source stroke is missing")?;
                if source
                    .modifiers
                    .as_ref()
                    .is_some_and(|stack| stack.has_enabled_array())
                {
                    return Err(
                        "Array modifier strokes must be baked before they can enter a linked group"
                            .into(),
                    );
                }
                if layer_id.is_some_and(|layer| layer != source.layer_id) {
                    return Err("Linked group sources must remain on one layer".into());
                }
                layer_id = Some(source.layer_id);
            }
        }
        let known_ids: std::collections::HashSet<u64> = self
            .strokes
            .iter()
            .filter(|stroke| !generated_set.contains(&stroke.id))
            .map(|stroke| stroke.id)
            .collect();
        if generated_set.iter().any(|id| known_ids.contains(id)) {
            return Err("Linked instance stroke ID collides with ordinary artwork".into());
        }
        let base_strokes: Vec<Stroke> = self
            .strokes
            .iter()
            .filter(|stroke| !generated_set.contains(&stroke.id))
            .cloned()
            .collect();
        let mut expanded = Vec::new();
        for instance in &self.linked_stroke_groups.instances {
            if !self
                .layers
                .iter()
                .any(|layer| layer.id == instance.layer_id)
            {
                return Err("Linked instance target layer is missing".into());
            }
            let definition = definition_map[&instance.definition_id];
            let before: crate::editing::Snapshot = definition
                .source_stroke_ids
                .iter()
                .map(|id| {
                    let stroke = source_map
                        .get(id)
                        .ok_or("Linked group source stroke is missing")?;
                    Ok((0, stroke.clone()))
                })
                .collect::<Result<_, &str>>()?;
            let mut after = crate::editing::transformed(
                &before,
                definition.pivot,
                instance.translation,
                instance.rotation.normalize(),
                instance.scale,
            )
            .map_err(|_| "Linked group transform exceeds document bounds")?;
            for (index, (_, stroke)) in after.iter_mut().enumerate() {
                stroke.id = instance.stroke_ids[index];
                stroke.layer_id = instance.layer_id;
                stroke.group_id = None;
            }
            expanded.extend(after.into_iter().map(|(_, stroke)| stroke));
        }
        if base_strokes.len().saturating_add(expanded.len()) > 100_000
            || base_strokes
                .iter()
                .chain(&expanded)
                .map(|stroke| stroke.points.len())
                .sum::<usize>()
                > 2_000_000
        {
            return Err("Expanded linked groups exceed document stroke or point limits".into());
        }
        self.strokes = base_strokes;
        self.strokes.extend(expanded);
        Ok(())
    }

    pub fn add_brush_tip_asset(
        &mut self,
        asset: crate::brush_tip::BrushTipAsset,
    ) -> Result<(), String> {
        asset.validate()?;
        if let Some(existing) = self
            .brush_tip_assets
            .iter()
            .find(|existing| existing.id == asset.id)
        {
            if existing.png != asset.png {
                return Err("Brush tip content identity collision".into());
            }
            return Ok(());
        }
        let mut changed = self.brush_tip_assets.clone();
        changed.push(asset);
        crate::brush_tip::validate_catalog(&changed)?;
        self.brush_tip_assets = changed;
        Ok(())
    }

    pub fn visible(&self, stroke: &Stroke) -> bool {
        self.layers
            .iter()
            .any(|l| l.id == stroke.layer_id && l.visible && self.layer_folder_visible(l.id))
    }
    pub fn layer_folder_visible(&self, layer_id: u64) -> bool {
        self.layer_folders
            .iter()
            .find(|folder| folder.layer_ids.contains(&layer_id))
            .is_none_or(|folder| folder.visible)
    }
    pub fn layer_folder_locked(&self, layer_id: u64) -> bool {
        self.layer_folders
            .iter()
            .any(|folder| folder.locked && folder.layer_ids.contains(&layer_id))
    }
    pub fn layer_is_editable(&self, layer_id: u64) -> bool {
        self.layers
            .iter()
            .any(|l| l.id == layer_id && l.visible && !l.locked)
            && self.layer_folder_visible(layer_id)
            && !self.layer_folder_locked(layer_id)
    }
    pub fn layer_opacity(&self, layer_id: u64) -> f32 {
        self.layers
            .iter()
            .find(|layer| layer.id == layer_id)
            .map_or(0.0, |layer| layer.opacity)
    }
    pub fn layer_blend_mode(&self, layer_id: u64) -> LayerBlendMode {
        self.layers
            .iter()
            .find(|layer| layer.id == layer_id)
            .map_or(LayerBlendMode::Normal, |layer| layer.blend_mode)
    }
    pub fn layer_clip_source(&self, layer_id: u64) -> Option<u64> {
        let index = self.layers.iter().position(|layer| layer.id == layer_id)?;
        let layer = &self.layers[index];
        layer
            .mask_source
            .or_else(|| (layer.clip_to_below && index > 0).then(|| self.layers[index - 1].id))
    }
    pub fn editable(&self, stroke: &Stroke) -> bool {
        !self
            .animation
            .two_bone_rig
            .as_ref()
            .is_some_and(|rig| rig.layer_id == stroke.layer_id)
            && self.layer_is_editable(stroke.layer_id)
    }
    pub fn active_editable(&self) -> bool {
        !self
            .animation
            .two_bone_rig
            .as_ref()
            .is_some_and(|rig| rig.layer_id == self.active_layer)
            && self.layer_is_editable(self.active_layer)
    }
    pub fn group_members(&self, id: u64) -> std::collections::HashSet<u64> {
        self.strokes
            .iter()
            .filter(|s| s.group_id == Some(id))
            .map(|s| s.id)
            .collect()
    }
    /// Group selection respects layer protection; individual mode deliberately bypasses expansion.
    pub fn selection_for_stroke(
        &self,
        id: u64,
        individual: bool,
    ) -> std::collections::HashSet<u64> {
        let Some(stroke) = self.strokes.iter().find(|s| s.id == id && self.editable(s)) else {
            return Default::default();
        };
        if let Some(instance) = self.linked_instance_for_stroke(id) {
            return instance
                .stroke_ids
                .iter()
                .filter(|stroke_id| {
                    self.strokes
                        .iter()
                        .find(|stroke| stroke.id == **stroke_id)
                        .is_some_and(|stroke| self.editable(stroke))
                })
                .copied()
                .collect();
        }
        if individual || stroke.group_id.is_none() {
            return std::collections::HashSet::from([id]);
        }
        self.strokes
            .iter()
            .filter(|s| s.group_id == stroke.group_id && self.editable(s))
            .map(|s| s.id)
            .collect()
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if !matches!(version, Some(1..=30)) {
            return Err("Unsupported document version".into());
        }
        if matches!(version, Some(1..=6)) {
            if let Some(strokes) = value.get_mut("strokes").and_then(|v| v.as_array_mut()) {
                for stroke in strokes {
                    if let Some(fields) = stroke.as_object_mut() {
                        fields.remove("fill");
                    }
                }
            }
        }
        if matches!(version, Some(1..=5)) {
            if let Some(strokes) = value.get_mut("strokes").and_then(|v| v.as_array_mut()) {
                for stroke in strokes {
                    if let Some(fields) = stroke.as_object_mut() {
                        fields.remove("group_id");
                    }
                }
            }
        }
        if matches!(version, Some(1..=4)) {
            value["layers"] = serde_json::to_value(layers_default()).map_err(|e| e.to_string())?;
            value["active_layer"] = serde_json::json!(1);
            if let Some(strokes) = value.get_mut("strokes").and_then(|v| v.as_array_mut()) {
                for stroke in strokes {
                    if let Some(fields) = stroke.as_object_mut() {
                        fields.insert("layer_id".into(), serde_json::json!(1));
                    }
                }
            }
        }
        // Versions before 4 rendered every brush as solid. Ignore unknown
        // texture extensions while preserving all earlier shape/dynamics data.
        if matches!(version, Some(1..=3)) {
            if let Some(strokes) = value.get_mut("strokes").and_then(|v| v.as_array_mut()) {
                for stroke in strokes {
                    if let Some(brush) = stroke.get_mut("brush").and_then(|b| b.as_object_mut()) {
                        brush.remove("texture");
                    }
                }
            }
        }
        // Legacy pressure values already include the old response curve.
        // Default dynamics reproduce that profile; ignore unknown newer fields.
        if matches!(version, Some(1 | 2)) {
            if let Some(strokes) = value.get_mut("strokes").and_then(|v| v.as_array_mut()) {
                for stroke in strokes {
                    if let Some(fields) = stroke.as_object_mut() {
                        if version == Some(1) {
                            fields.remove("brush");
                        } else if let Some(brush) =
                            fields.get_mut("brush").and_then(|b| b.as_object_mut())
                        {
                            brush.remove("dynamics");
                        }
                        if let Some(points) =
                            fields.get_mut("points").and_then(|p| p.as_array_mut())
                        {
                            for point in points {
                                if let Some(point) = point.as_object_mut() {
                                    point.remove("tilt");
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut doc: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        let swatches = doc.color_swatches.clone();
        swatches.resolve_strokes(&mut doc.strokes);
        if let Some(strokes) = &mut doc.animation.base_strokes {
            swatches.resolve_strokes(strokes);
        }
        for key in &mut doc.animation.keyframes {
            swatches.resolve_strokes(&mut key.strokes);
        }
        for track in &mut doc.animation.layer_tracks {
            for key in &mut track.keyframes {
                swatches.resolve_strokes(&mut key.strokes);
            }
        }
        if doc.animation.version == 1 {
            // Version 2 adds independent layer tracks and exposure lengths;
            // both fields default empty so existing whole-drawing tracks stay intact.
            doc.animation.version = crate::animation::CURRENT_VERSION;
        }
        if version == Some(16) {
            for stroke in &mut doc.strokes {
                if stroke.brush.tip_asset_id != 0 {
                    if let Some(asset) = doc
                        .brush_tip_assets
                        .iter()
                        .find(|asset| asset.id == stroke.brush.tip_asset_id)
                    {
                        stroke.brush.tip_aspect = asset.size[0] as f32 / asset.size[1] as f32;
                        stroke.brush.dabs.enabled = true;
                    }
                }
            }
        }
        doc.version = 30;
        doc.refresh_linked_instances()?;
        if doc.strokes.len() > 100_000 {
            return Err("Too many strokes".into());
        }
        if doc.measurements.len() > 10_000 {
            return Err("Too many measurement annotations".into());
        }
        let mut measurement_ids = std::collections::HashSet::new();
        if doc
            .measurements
            .iter()
            .any(|annotation| !annotation.is_valid() || !measurement_ids.insert(annotation.id))
        {
            return Err("Invalid or duplicate measurement annotation".into());
        }
        if doc.layers.is_empty() || doc.layers.len() > 256 {
            return Err("A document requires 1–256 layers".into());
        }
        let mut layer_ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for layer in &doc.layers {
            if layer.id == 0
                || layer.id == u64::MAX
                || !layer_ids.insert(layer.id)
                || layer.name.trim() != layer.name
                || layer.name.is_empty()
                || layer.name.chars().count() > 64
                || layer.name.chars().any(char::is_control)
                || !layer.opacity.is_finite()
                || !(0.0..=1.0).contains(&layer.opacity)
                || !names.insert(layer.name.to_lowercase())
            {
                return Err("Invalid or duplicate layer metadata".into());
            }
        }
        if doc.layer_folders.len() > 128 {
            return Err("Too many layer folders".into());
        }
        let mut folder_ids = std::collections::HashSet::new();
        let mut folder_names = std::collections::HashSet::new();
        let mut folder_members = std::collections::HashSet::new();
        for folder in &doc.layer_folders {
            if folder.id == 0
                || folder.id == u64::MAX
                || !folder_ids.insert(folder.id)
                || folder.name.trim() != folder.name
                || folder.name.is_empty()
                || folder.name.chars().count() > 64
                || folder.name.chars().any(char::is_control)
                || !folder_names.insert(folder.name.to_lowercase())
            {
                return Err("Invalid or duplicate layer folder".into());
            }
            for layer_id in &folder.layer_ids {
                if !layer_ids.contains(layer_id) || !folder_members.insert(*layer_id) {
                    return Err(
                        "Layer folders must reference existing layers without duplicates".into(),
                    );
                }
            }
        }
        layer_mask_order(&doc.layers)?;
        if !layer_ids.contains(&doc.active_layer) {
            return Err("Active layer does not exist".into());
        }
        crate::image_planes::validate_catalog(&doc.image_planes)?;
        crate::brush_tip::validate_catalog(&doc.brush_tip_assets)?;
        let tip_assets: std::collections::HashMap<_, _> = doc
            .brush_tip_assets
            .iter()
            .map(|asset| (asset.id, asset))
            .collect();
        doc.animation
            .validate(&layer_ids, &tip_assets.keys().copied().collect())?;
        doc.color_swatches.validate(&doc)?;
        if doc.active_image_plane.is_some_and(|id| {
            !doc.image_planes
                .iter()
                .any(|plane| plane.id == id && plane.visible && !plane.locked && plane.drawable)
        }) {
            return Err(
                "Active image plane must refer to a visible, unlocked drawable plane".into(),
            );
        }
        if [
            doc.active_image_plane.is_some(),
            doc.active_guide.is_some(),
            doc.active_model_reference.is_some(),
        ]
        .into_iter()
        .filter(|active| *active)
        .count()
            > 1
        {
            return Err(
                "Only one image plane, saved guide, or OBJ model can be the active drawing target"
                    .into(),
            );
        }
        guide_resources::validate_catalog(&doc.guide_resources, doc.active_guide)?;
        for stroke in &doc.strokes {
            if let Some(id) = stroke.guide_attachment {
                if !doc.guide_resources.iter().any(|guide| {
                    guide.id == id
                        && matches!(
                            guide.geometry,
                            guide_resources::GuideGeometry::Plane { .. }
                                | guide_resources::GuideGeometry::Cylinder { .. }
                        )
                }) {
                    return Err("Stroke attachment references a missing plane or cylinder".into());
                }
            }
        }
        model_references::validate_catalog(&doc.model_references, doc.active_model_reference)?;
        let mut ids = std::collections::HashSet::new();
        let mut total = 0;
        let mut total_dabs = 0usize;
        let mut total_model_surface_triangles = 0usize;
        let mut groups = std::collections::HashMap::new();
        for stroke in &doc.strokes {
            if let Some(group) = stroke.group_id {
                if group == 0
                    || group == u64::MAX
                    || groups
                        .insert(group, stroke.layer_id)
                        .is_some_and(|layer| layer != stroke.layer_id)
                {
                    return Err("Invalid group identity or cross-layer membership".into());
                }
            }
            if !layer_ids.contains(&stroke.layer_id)
                || !ids.insert(stroke.id)
                || stroke.id == u64::MAX
            {
                return Err("Invalid stroke IDs".into());
            }
            if stroke.fill.is_some() {
                if stroke.gradient.is_some()
                    || stroke.fill.as_ref().is_some_and(|fill| {
                        fill.gradient.is_some_and(|gradient| !gradient.is_valid())
                            || fill.texture.is_some_and(|texture| {
                                !texture.is_valid() || !tip_assets.contains_key(&texture.asset_id)
                            })
                    })
                {
                    return Err(
                        "Invalid gradient or missing image texture for a planar fill".into(),
                    );
                }
                crate::polygon::triangulate(stroke)?;
            }
            if let Some(image_texture) = stroke.brush.texture.image {
                if !image_texture.is_valid() || !tip_assets.contains_key(&image_texture.asset_id) {
                    return Err("Invalid or missing repeating brush texture image".into());
                }
            }
            for variant in stroke.brush.explicit_tip_variants() {
                let asset = tip_assets
                    .get(&variant.asset_id)
                    .ok_or("Stroke refers to a missing brush tip variant")?;
                if asset.size != variant.size {
                    return Err("Brush tip variant dimensions do not match its PNG".into());
                }
            }
            if stroke.brush.texture.anchor_mode == crate::texture::TextureAnchorMode::CapturedPlanar
                && stroke.brush.texture.frame.is_none()
            {
                return Err("Planar-anchored stroke texture is missing its captured frame".into());
            }
            total += stroke.points.len();
            if stroke.brush.dabs.enabled {
                total_dabs = total_dabs
                    .checked_add(crate::dab_placement::point_count(stroke)?)
                    .ok_or("Dab placement count overflow")?;
                if total_dabs > crate::dab_placement::MAX_DOCUMENT_DABS {
                    return Err("Document exceeds the dab geometry budget".into());
                }
            }
            if let Some(patch) = &stroke.model_surface {
                total_model_surface_triangles = total_model_surface_triangles
                    .checked_add(patch.triangles.len())
                    .ok_or("Model surface triangle count overflow")?;
                if patch.triangles.is_empty()
                    || patch.triangles.len() > MAX_MODEL_SURFACE_TRIANGLES_PER_STROKE
                    || total_model_surface_triangles > MAX_MODEL_SURFACE_TRIANGLES_PER_DOCUMENT
                    || (!stroke.brush.dabs.enabled && stroke.brush.tip_asset_id == 0)
                    || stroke.brush.dabs.surface_target.is_some()
                    || patch
                        .triangles
                        .windows(2)
                        .any(|pair| pair[0].source_id >= pair[1].source_id)
                    || patch.triangles.iter().any(|triangle| {
                        !triangle.normal.is_finite()
                            || (triangle.normal.length_squared() - 1.0).abs() > 1e-3
                            || triangle.positions.iter().any(|point| {
                                !point.is_finite() || point.abs().max_element() > 10_000.0
                            })
                            || (triangle.positions[1] - triangle.positions[0])
                                .cross(triangle.positions[2] - triangle.positions[0])
                                .length_squared()
                                <= 1e-12
                    })
                {
                    return Err("Invalid or oversized model dab surface patch".into());
                }
            }
            if total > 2_000_000
                || !stroke.brush.is_valid()
                || (stroke.brush.strands.enabled
                    && stroke.brush.strands.vertex_estimate(stroke.points.len())
                        > crate::geometry::MAX_STRAND_VERTICES)
                || (stroke.brush.kind == BrushKind::Round
                    && stroke.brush.tube_profile != TubeProfile::Round
                    && !stroke.brush.flow.enabled
                    && !stroke.brush.strands.enabled
                    && !stroke.brush.dabs.enabled
                    && stroke.brush.tip_asset_id == 0
                    && crate::geometry::profile_tube_vertex_estimate(stroke)
                        > crate::geometry::MAX_PROFILE_TUBE_VERTICES)
                || (stroke.brush.flow.enabled
                    && (stroke.points.len() > crate::geometry::MAX_AIRBRUSH_POINTS
                        || crate::geometry::airbrush_vertex_estimate(stroke)
                            > crate::geometry::MAX_AIRBRUSH_VERTICES
                        || stroke
                            .points
                            .windows(2)
                            .any(|pair| pair[0].time_ms > pair[1].time_ms)))
                || (stroke.brush.tip_asset_id != 0
                    && tip_assets
                        .get(&stroke.brush.tip_asset_id)
                        .is_none_or(|asset| {
                            (stroke.brush.tip_aspect - asset.size[0] as f32 / asset.size[1] as f32)
                                .abs()
                                > 1e-4
                        }))
                || !stroke.radius.is_finite()
                || !(0.001..=1.0).contains(&stroke.radius)
                || stroke
                    .color
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || stroke.gradient.is_some_and(|gradient| !gradient.is_valid())
                || stroke.points.iter().any(|p| !p.is_valid())
                || !stroke.point_thickness_is_valid()
                || !stroke.point_colors_are_valid()
                || (stroke.fill.is_some() && stroke.has_local_colors())
                || stroke.modifiers.as_ref().is_some_and(|stack| {
                    !stack.is_valid(stroke.points.len()) || !stack.is_compatible(stroke)
                })
                || stroke.curve.as_ref().is_some_and(|curve| {
                    !curve.is_valid()
                        || stroke.points.len() > crate::curve_geometry::MAX_CURVE_POINTS
                        || !curve.samples_match_stroke(stroke)
                })
            {
                return Err("Invalid or oversized stroke data".into());
            }
        }
        Ok(doc)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn stroke() -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
            material: Default::default(),
            fill: None,
            group_id: None,
            layer_id: 1,
            id: 1,
            color: [0.5; 3],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::ONE,
                pressure: 0.7,
                tilt: None,

                time_ms: 0,
            }],
        }
    }
    #[test]
    fn round_trip_preserves_each_brush() {
        for kind in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel] {
            let mut s = stroke();
            s.brush = Brush {
                kind,
                thickness: 0.35,
                opacity: 0.64,
                softness: 0.35,
                angle_degrees: -42.0,
                normal: Vec3::new(1.0, 2.0, 3.0).normalize(),
                tangent: Vec3::new(2.0, -1.0, 0.0).normalize(),
                dynamics: Dynamics::default(),
                texture: TextureSettings::default(),
                ..Brush::default()
            };
            s.brush.dynamics.width.minimum = 0.05;
            s.brush.dynamics.width.curve = 2.0;
            s.brush.dynamics.thickness.enabled = false;
            s.brush.dynamics.opacity = PressureMapping {
                enabled: true,
                minimum: 0.18,
                curve: 1.7,
            };
            s.brush.dynamics.taper_start = 0.4;
            s.brush.dynamics.taper_end = 0.7;
            s.brush.dynamics.tilt_orientation = true;
            s.points[0].tilt = Some(Vec3::X);
            let doc = Document {
                linked_stroke_groups: Default::default(),
                color_swatches: Default::default(),
                layer_folders: Vec::new(),
                layers: layers_default(),
                active_layer: 1,
                version: 8,
                strokes: vec![s],
                image_planes: Vec::new(),
                active_image_plane: None,
                guide_resources: Vec::new(),
                active_guide: None,
                model_references: Vec::new(),
                active_model_reference: None,
                brush_tip_assets: Vec::new(),
                animation: Default::default(),
                measurements: Vec::new(),
            };
            let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
            assert_eq!(loaded.version, 30);
            assert_eq!(loaded.strokes, doc.strokes);
        }
    }
    #[test]
    fn legacy_documents_upgrade_without_changing_ink() {
        let original = stroke();
        let mut old = serde_json::to_value(Document {
            linked_stroke_groups: Default::default(),
            color_swatches: Default::default(),
            layer_folders: Vec::new(),
            layers: layers_default(),
            active_layer: 1,
            version: 1,
            strokes: vec![original.clone()],
            image_planes: Vec::new(),
            active_image_plane: None,
            guide_resources: Vec::new(),
            active_guide: None,
            model_references: Vec::new(),
            active_model_reference: None,
            brush_tip_assets: Vec::new(),
            animation: Default::default(),
            measurements: Vec::new(),
        })
        .unwrap();
        // Even an unrecognized extension must not opt a version-1 stroke into
        // a new shape, or prevent an otherwise valid legacy file from opening.
        old["strokes"][0]["brush"] = serde_json::json!({"kind": "future_brush"});
        let loaded = Document::from_json(&old.to_string()).unwrap();
        assert_eq!(loaded.version, 30);
        assert_eq!(loaded.strokes, vec![original]);
        old["strokes"][0].as_object_mut().unwrap().remove("brush");
        let without_extension = Document::from_json(&old.to_string()).unwrap();
        assert_eq!(without_extension.strokes, loaded.strokes);
    }
    #[test]
    fn version_seven_migration_defaults_missing_opacity_to_opaque() {
        let mut value = serde_json::to_value(Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![stroke()],
            ..Default::default()
        })
        .unwrap();
        value["version"] = serde_json::json!(7);
        value["strokes"][0]["brush"]
            .as_object_mut()
            .unwrap()
            .remove("opacity");
        value["strokes"][0]["brush"]
            .as_object_mut()
            .unwrap()
            .remove("softness");
        value["strokes"][0]["brush"]["dynamics"]
            .as_object_mut()
            .unwrap()
            .remove("opacity");

        let loaded = Document::from_json(&value.to_string()).unwrap();
        assert_eq!(loaded.version, 30);
        assert_eq!(loaded.strokes[0].brush.opacity, 1.0);
        assert_eq!(loaded.strokes[0].brush.softness, 0.0);
        assert_eq!(
            loaded.strokes[0].brush.dynamics.opacity,
            Dynamics::default().opacity
        );

        let saved = serde_json::to_string(&loaded).unwrap();
        let reopened = Document::from_json(&saved).unwrap();
        assert_eq!(reopened.version, 30);
        assert_eq!(reopened.strokes, loaded.strokes);
    }
    #[test]
    fn rejects_invalid_brush_metadata() {
        for brush in [
            Brush {
                thickness: 0.0,
                ..Brush::default()
            },
            Brush {
                thickness: 1.1,
                ..Brush::default()
            },
            Brush {
                opacity: -0.1,
                ..Brush::default()
            },
            Brush {
                opacity: 1.1,
                ..Brush::default()
            },
            Brush {
                opacity: f32::NAN,
                ..Brush::default()
            },
            Brush {
                softness: -0.1,
                ..Brush::default()
            },
            Brush {
                softness: 1.1,
                ..Brush::default()
            },
            Brush {
                softness: f32::INFINITY,
                ..Brush::default()
            },
            Brush {
                angle_degrees: 181.0,
                ..Brush::default()
            },
            Brush {
                normal: Vec3::ZERO,
                ..Brush::default()
            },
            Brush {
                normal: Vec3::splat(1.0),
                ..Brush::default()
            },
            Brush {
                tangent: Vec3::ZERO,
                ..Brush::default()
            },
            Brush {
                tangent: Vec3::Z,
                ..Brush::default()
            },
        ] {
            let mut s = stroke();
            s.brush = brush;
            let doc = Document {
                linked_stroke_groups: Default::default(),
                color_swatches: Default::default(),
                layer_folders: Vec::new(),
                layers: layers_default(),
                active_layer: 1,
                version: 7,
                strokes: vec![s],
                image_planes: Vec::new(),
                active_image_plane: None,
                guide_resources: Vec::new(),
                active_guide: None,
                model_references: Vec::new(),
                active_model_reference: None,
                brush_tip_assets: Vec::new(),
                animation: Default::default(),
                measurements: Vec::new(),
            };
            assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        }
        assert!(!Brush {
            normal: Vec3::splat(f32::NAN),
            ..Brush::default()
        }
        .is_valid());
        assert!(!Brush {
            thickness: f32::INFINITY,
            ..Brush::default()
        }
        .is_valid());
        assert!(!Brush {
            angle_degrees: f32::NAN,
            ..Brush::default()
        }
        .is_valid());
    }
    #[test]
    fn rejects_invalid_documents() {
        assert!(Document::from_json("{\"version\":99,\"strokes\":[]}").is_err());
        let mut doc = Document {
            linked_stroke_groups: Default::default(),
            color_swatches: Default::default(),
            layer_folders: Vec::new(),
            layers: layers_default(),
            active_layer: 1,
            version: 7,
            strokes: vec![stroke(), stroke()],
            image_planes: Vec::new(),
            active_image_plane: None,
            guide_resources: Vec::new(),
            active_guide: None,
            model_references: Vec::new(),
            active_model_reference: None,
            brush_tip_assets: Vec::new(),
            animation: Default::default(),
            measurements: Vec::new(),
        };
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        doc.strokes.pop();
        doc.strokes[0].points[0].pressure = 1.1;
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
    }
    #[test]
    fn version_two_migration_preserves_brush_shape_and_baked_pressure() {
        let mut original = stroke();
        original.brush.kind = BrushKind::Chisel;
        original.brush.thickness = 0.6;
        original.brush.angle_degrees = 43.0;
        let mut value = serde_json::to_value(Document {
            linked_stroke_groups: Default::default(),
            color_swatches: Default::default(),
            layer_folders: Vec::new(),
            layers: layers_default(),
            active_layer: 1,
            version: 2,
            strokes: vec![original.clone()],
            image_planes: Vec::new(),
            active_image_plane: None,
            guide_resources: Vec::new(),
            active_guide: None,
            model_references: Vec::new(),
            active_model_reference: None,
            brush_tip_assets: Vec::new(),
            animation: Default::default(),
            measurements: Vec::new(),
        })
        .unwrap();
        value["strokes"][0]["brush"]["dynamics"] = serde_json::json!({"future":true});
        value["strokes"][0]["points"][0]["tilt"] = serde_json::json!("unknown extension");
        let loaded = Document::from_json(&value.to_string()).unwrap();
        assert_eq!(loaded.version, 30);
        assert_eq!(loaded.strokes, vec![original]);
    }
    #[test]
    fn rejects_invalid_dynamics_and_tilt_metadata() {
        let original = Document {
            linked_stroke_groups: Default::default(),
            color_swatches: Default::default(),
            layer_folders: Vec::new(),
            layers: layers_default(),
            active_layer: 1,
            version: 7,
            strokes: vec![stroke()],
            image_planes: Vec::new(),
            active_image_plane: None,
            guide_resources: Vec::new(),
            active_guide: None,
            model_references: Vec::new(),
            active_model_reference: None,
            brush_tip_assets: Vec::new(),
            animation: Default::default(),
            measurements: Vec::new(),
        };
        for dynamics in [
            Dynamics {
                width: PressureMapping {
                    minimum: -0.1,
                    ..PressureMapping::default()
                },
                ..Dynamics::default()
            },
            Dynamics {
                thickness: PressureMapping {
                    curve: 4.0,
                    ..PressureMapping::default()
                },
                ..Dynamics::default()
            },
            Dynamics {
                opacity: PressureMapping {
                    minimum: 1.1,
                    ..Dynamics::default().opacity
                },
                ..Dynamics::default()
            },
            Dynamics {
                taper_start: -0.1,
                ..Dynamics::default()
            },
            Dynamics {
                taper_end: 10.1,
                ..Dynamics::default()
            },
        ] {
            let mut doc = original.clone();
            doc.strokes[0].brush.dynamics = dynamics;
            assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        }
        for tilt in [Vec3::ZERO, Vec3::splat(1.0), Vec3::X * 2.0] {
            let mut doc = original.clone();
            doc.strokes[0].points[0].tilt = Some(tilt);
            assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        }
        let mut point = stroke().points[0];
        point.tilt = Some(Vec3::splat(f32::NAN));
        assert!(!point.is_valid());
        assert!(!Dynamics {
            taper_start: f32::INFINITY,
            ..Dynamics::default()
        }
        .is_valid());
    }
    #[test]
    fn texture_settings_roundtrip_and_legacy_three_remains_solid() {
        let mut mark = stroke();
        mark.brush.kind = BrushKind::Ribbon;
        mark.brush.dynamics.taper_end = 0.7;
        mark.brush.texture = TextureSettings {
            kind: crate::texture::TextureKind::DryBrush,
            scale: 0.37,
            mapping: Default::default(),
            strength: 0.82,
            seed: 1234,
            ..TextureSettings::default()
        };
        let doc = Document {
            linked_stroke_groups: Default::default(),
            color_swatches: Default::default(),
            layer_folders: Vec::new(),
            layers: layers_default(),
            active_layer: 1,
            version: 7,
            strokes: vec![mark.clone()],
            image_planes: Vec::new(),
            active_image_plane: None,
            guide_resources: Vec::new(),
            active_guide: None,
            model_references: Vec::new(),
            active_model_reference: None,
            brush_tip_assets: Vec::new(),
            animation: Default::default(),
            measurements: Vec::new(),
        };
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, doc.strokes);
        let mut legacy = serde_json::to_value(doc).unwrap();
        legacy["version"] = serde_json::json!(3);
        legacy["strokes"][0]["brush"]["texture"] = serde_json::json!({"kind":"future_pattern"});
        let loaded = Document::from_json(&legacy.to_string()).unwrap();
        mark.brush.texture = TextureSettings::default();
        assert_eq!(loaded.version, 30);
        assert_eq!(loaded.strokes, vec![mark]);
    }
    #[test]
    fn document_rejects_invalid_texture_metadata() {
        for (field, value) in [
            ("scale", -0.1),
            ("scale", 11.0),
            ("strength", 1.1),
            ("strength", -0.1),
        ] {
            let mut doc = serde_json::to_value(Document {
                linked_stroke_groups: Default::default(),
                color_swatches: Default::default(),
                layer_folders: Vec::new(),
                layers: layers_default(),
                active_layer: 1,
                version: 7,
                strokes: vec![stroke()],
                image_planes: Vec::new(),
                active_image_plane: None,
                guide_resources: Vec::new(),
                active_guide: None,
                model_references: Vec::new(),
                active_model_reference: None,
                brush_tip_assets: Vec::new(),
                animation: Default::default(),
                measurements: Vec::new(),
            })
            .unwrap();
            doc["strokes"][0]["brush"]["texture"][field] = serde_json::json!(value);
            assert!(Document::from_json(&doc.to_string()).is_err());
        }
    }
}

#[cfg(test)]
mod layer_tests {
    use super::*;
    #[test]
    fn version_four_migration_preserves_all_resolved_ink() {
        let mut stroke = Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
            material: Default::default(),
            fill: None,
            group_id: None,
            id: 8,
            layer_id: 1,
            color: [0.2, 0.4, 0.7],
            radius: 0.05,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::new(2.0, 3.0, 4.0),
                pressure: 0.37,
                tilt: Some(Vec3::Y),

                time_ms: 0,
            }],
        };
        stroke.brush.kind = BrushKind::Chisel;
        stroke.brush.dynamics.taper_end = 0.5;
        stroke.brush.texture.kind = crate::texture::TextureKind::Grain;
        stroke.brush.texture.seed = 321;
        let original = stroke.clone();
        let mut value = serde_json::to_value(Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![stroke],
            ..Default::default()
        })
        .unwrap();
        value["version"] = serde_json::json!(4);
        value["layers"] = serde_json::json!("unknown extension");
        value["active_layer"] = serde_json::json!(99);
        value["strokes"][0]["layer_id"] = serde_json::json!("unknown");
        let migrated = Document::from_json(&value.to_string()).unwrap();
        assert_eq!(migrated.version, 30);
        assert_eq!(migrated.strokes, vec![original]);
        assert_eq!(migrated.layers, layers_default());
        assert_eq!(migrated.active_layer, 1);
    }
    #[test]
    fn layers_roundtrip_and_visibility_locking_are_independent() {
        let mut doc = Document::default();
        doc.layers.push(Layer {
            id: 2,
            name: "Roof".into(),
            visible: true,
            locked: true,
            opacity: 1.0,
            blend_mode: LayerBlendMode::Normal,
            mask_source: None,
            clip_to_below: false,
        });
        doc.active_layer = 2;
        let stroke = Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
            material: Default::default(),
            fill: None,
            group_id: None,
            id: 1,
            layer_id: 2,
            color: [0.5; 3],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![],
        };
        assert!(doc.visible(&stroke));
        assert!(!doc.editable(&stroke));
        assert!(!doc.active_editable());
        doc.layers[1].locked = false;
        assert!(doc.editable(&stroke));
        assert!(doc.active_editable());
        doc.layers[1].visible = false;
        assert!(!doc.visible(&stroke));
        assert!(!doc.editable(&stroke));
        doc.strokes.push(stroke);
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.layers, doc.layers);
        assert_eq!(loaded.active_layer, 2);
        assert_eq!(loaded.strokes, doc.strokes);
    }
    #[test]
    fn invalid_layers_and_orphan_strokes_are_rejected() {
        for field in [
            "empty",
            "duplicate_id",
            "duplicate_name",
            "blank_name",
            "active_missing",
            "orphan",
            "zero_id",
            "long_name",
        ] {
            let mut doc = Document::default();
            match field {
                "empty" => doc.layers.clear(),
                "duplicate_id" => doc.layers.push(doc.layers[0].clone()),
                "duplicate_name" => doc.layers.push(Layer {
                    id: 2,
                    name: "layer 1".into(),
                    visible: true,
                    locked: false,
                    opacity: 1.0,
                    blend_mode: LayerBlendMode::Normal,
                    mask_source: None,
                    clip_to_below: false,
                }),
                "blank_name" => doc.layers[0].name = " ".into(),
                "active_missing" => doc.active_layer = 2,
                "orphan" => doc.strokes.push(Stroke {
                    guide_attachment: None,
                    model_surface: None,
                    gradient: None,
                    point_thickness: None,
                    point_colors: None,
                    modifiers: None,
                    curve: None,
                    material: Default::default(),
                    fill: None,
                    group_id: None,
                    id: 1,
                    layer_id: 2,
                    color: [0.5; 3],
                    radius: 0.02,
                    brush: Brush::default(),
                    points: vec![],
                }),
                "zero_id" => doc.layers[0].id = 0,
                _ => doc.layers[0].name = "x".repeat(65),
            }
            assert!(
                Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err(),
                "{field}"
            );
        }
    }
}

#[cfg(test)]
mod saved_guide_document_tests {
    use super::*;
    use guide_resources::{GuideGeometry, GuideResource};

    fn guide(id: u64, name: &str, geometry: GuideGeometry) -> GuideResource {
        GuideResource {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            geometry,
        }
    }

    fn sample_stroke() -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
            material: Default::default(),
            fill: None,
            group_id: None,
            layer_id: 1,
            id: 1,
            color: [0.5; 3],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::ONE,
                pressure: 0.7,
                tilt: None,

                time_ms: 0,
            }],
        }
    }

    fn sample_loft() -> GuideGeometry {
        let section = |z: f32, lift: f32| guide_resources::LoftSection {
            controls: [
                Vec3::new(-1.0, 0.0, z),
                Vec3::new(-0.3, lift + 0.2, z),
                Vec3::new(0.3, lift - 0.1, z),
                Vec3::new(1.0, lift, z),
            ],
        };
        let mut sections = [section(0.0, 0.0); 8];
        sections[0] = section(-1.0, 0.0);
        sections[1] = section(0.0, 0.35);
        sections[2] = section(1.0, -0.1);
        GuideGeometry::Loft {
            surface: guide_resources::LoftSurface {
                center: Vec3::new(2.0, -1.0, 0.5),
                rotation: glam::Quat::from_rotation_y(0.3),
                sections,
                section_count: 3,
                tension: 0.4,
            },
        }
    }

    #[test]
    fn loft_snapshot_roundtrips_and_guide_edit_undoes_without_changing_ink() {
        let ink = vec![sample_stroke()];
        let loft = guide(27, "Canopy loft", sample_loft());
        let mut doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: ink.clone(),
            guide_resources: vec![loft.clone()],
            active_guide: Some(27),
            ..Document::default()
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let reopened = Document::from_json(&std::fs::read_to_string(file.path()).unwrap()).unwrap();
        assert_eq!(reopened.guide_resources, vec![loft.clone()]);
        assert_eq!(reopened.strokes, ink);

        let before = (
            doc.guide_resources.clone(),
            doc.active_guide,
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let mut changed = loft;
        if let GuideGeometry::Loft { surface } = &mut changed.geometry {
            surface.tension = -0.6;
            surface.center += Vec3::new(0.5, 0.0, -0.2);
        }
        let after = (
            vec![changed],
            Some(27),
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let edit = crate::editing::Edit::Guides {
            ink: None,
            before: before.clone(),
            after: after.clone(),
        };
        edit.apply(&mut doc, true);
        assert_eq!(doc.strokes, ink);
        edit.apply(&mut doc, false);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            before
        );
        edit.apply(&mut doc, true);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            after
        );
        assert_eq!(doc.strokes, ink);
    }

    #[test]
    fn primitive_guides_roundtrip_transform_undo_without_moving_existing_ink() {
        let mut doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![sample_stroke()],
            guide_resources: vec![
                guide(
                    10,
                    "Plane",
                    GuideGeometry::Plane {
                        origin: Vec3::new(1.0, 2.0, 3.0),
                        rotation: glam::Quat::from_rotation_y(0.4),
                    },
                ),
                guide(11, "Sphere", GuideGeometry::sphere(Vec3::X, 1.2)),
                guide(12, "Cone", GuideGeometry::cone(Vec3::Y, 0.8, 2.0)),
                guide(
                    13,
                    "Ring",
                    GuideGeometry::Ring {
                        center: Vec3::Z,
                        rotation: glam::Quat::IDENTITY,
                        major_radius: 2.0,
                        minor_radius: 0.5,
                    },
                ),
            ],
            active_guide: Some(10),
            ..Document::default()
        };
        let original_strokes = doc.strokes.clone();
        let before = (
            doc.guide_resources.clone(),
            doc.active_guide,
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let mut after_guides = doc.guide_resources.clone();
        if let GuideGeometry::Plane { origin, rotation } = &mut after_guides[0].geometry {
            *origin = Vec3::new(-2.0, 0.5, 4.0);
            *rotation = glam::Quat::from_rotation_x(-0.6);
        }
        after_guides[0].visible = false;
        after_guides[0].locked = true;
        let after = (
            after_guides,
            None,
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let edit = crate::editing::Edit::Guides {
            ink: None,
            before: before.clone(),
            after: after.clone(),
        };
        edit.apply(&mut doc, true);
        assert_eq!(
            doc.strokes, original_strokes,
            "guide changes must not move ink"
        );
        assert_eq!(doc.active_guide, None);

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let reopened = Document::from_json(&std::fs::read_to_string(file.path()).unwrap()).unwrap();
        assert_eq!(reopened.guide_resources, doc.guide_resources);
        assert_eq!(reopened.strokes, original_strokes);
        assert_eq!(reopened.active_guide, None);
        let glb = crate::export::encode(&reopened, None).unwrap();
        assert_eq!(
            gltf::Gltf::from_slice(&glb.bytes).unwrap().meshes().count(),
            1,
            "saved guide resources are not exported as artwork meshes"
        );

        edit.apply(&mut doc, false);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            before
        );
        edit.apply(&mut doc, true);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            after
        );

        let removed = (
            Vec::new(),
            None,
            doc.image_planes.clone(),
            doc.active_image_plane,
            doc.active_model_reference,
        );
        let remove = crate::editing::Edit::Guides {
            ink: None,
            before: after.clone(),
            after: removed.clone(),
        };
        remove.apply(&mut doc, true);
        assert!(doc.guide_resources.is_empty());
        assert_eq!(doc.strokes, original_strokes);
        remove.apply(&mut doc, false);
        assert_eq!(
            (
                doc.guide_resources.clone(),
                doc.active_guide,
                doc.image_planes.clone(),
                doc.active_image_plane,
                doc.active_model_reference,
            ),
            after
        );
    }

    #[test]
    fn pre_guide_native_file_migration_keeps_ink_and_uses_empty_catalog() {
        let doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![sample_stroke()],
            guide_resources: vec![guide(10, "Sphere", GuideGeometry::sphere(Vec3::ZERO, 1.0))],
            active_guide: Some(10),
            ..Document::default()
        };
        let mut legacy = serde_json::to_value(doc.clone()).unwrap();
        legacy["version"] = serde_json::json!(10);
        legacy.as_object_mut().unwrap().remove("guide_resources");
        legacy.as_object_mut().unwrap().remove("active_guide");
        let loaded = Document::from_json(&legacy.to_string()).unwrap();
        assert_eq!(loaded.version, 30);
        assert!(loaded.guide_resources.is_empty());
        assert_eq!(loaded.active_guide, None);
        assert_eq!(loaded.strokes, doc.strokes);
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    fn mark(id: u64) -> Stroke {
        Stroke {
            guide_attachment: None,
            model_surface: None,
            gradient: None,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            curve: None,
            material: Default::default(),
            id,
            fill: None,
            group_id: Some(7),
            layer_id: 1,
            color: [0.3; 3],
            radius: 0.03,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::X * id as f32,
                pressure: 0.37,
                tilt: Some(Vec3::Y),

                time_ms: 0,
            }],
        }
    }

    #[test]
    fn model_surface_patch_roundtrips_and_rejects_invalid_triangles() {
        let mut stroke = mark(1);
        stroke.brush.dabs.enabled = true;
        stroke.model_surface = Some(ModelSurfacePatch {
            triangles: vec![ModelSurfaceTriangle {
                source_id: 4,
                positions: [Vec3::ZERO, Vec3::X, Vec3::Y],
                normal: Vec3::Z,
            }],
        });
        let doc = Document {
            strokes: vec![stroke],
            ..Default::default()
        };
        let reopened = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(reopened.strokes, doc.strokes);
        assert_eq!(reopened.version, 30);

        let mut invalid = doc;
        invalid.strokes[0].model_surface.as_mut().unwrap().triangles[0].normal = Vec3::ZERO;
        assert!(Document::from_json(&serde_json::to_string(&invalid).unwrap()).is_err());
    }
    #[test]
    fn legacy_five_preserves_resolved_ink_and_layers_but_ignores_group_extensions() {
        let mut doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![mark(1), mark(2)],
            ..Default::default()
        };
        doc.layers.push(Layer {
            id: 2,
            name: "Roof".into(),
            visible: false,
            locked: true,
            opacity: 1.0,
            blend_mode: LayerBlendMode::Normal,
            mask_source: None,
            clip_to_below: false,
        });
        doc.active_layer = 2;
        for stroke in &mut doc.strokes {
            stroke.layer_id = 2;
        }
        doc.strokes[0].brush.texture.kind = crate::texture::TextureKind::Grain;
        doc.strokes[0].brush.dynamics.taper_end = 0.4;
        let mut value = serde_json::to_value(&doc).unwrap();
        value["version"] = serde_json::json!(5);
        value["strokes"][0]["group_id"] = serde_json::json!("future extension");
        for s in &mut doc.strokes {
            s.group_id = None;
        }
        let loaded = Document::from_json(&value.to_string()).unwrap();
        assert_eq!(loaded.version, 30);
        assert_eq!(loaded.strokes, doc.strokes);
        assert_eq!(loaded.layers, doc.layers);
    }
    #[test]
    fn groups_roundtrip_select_as_a_unit_and_allow_individual_access() {
        let mut doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![mark(1), mark(2)],
            ..Default::default()
        };
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, doc.strokes);
        assert_eq!(
            doc.group_members(7),
            std::collections::HashSet::from([1, 2])
        );
        assert_eq!(
            doc.selection_for_stroke(1, false),
            std::collections::HashSet::from([1, 2])
        );
        assert_eq!(
            doc.selection_for_stroke(1, true),
            std::collections::HashSet::from([1])
        );
        doc.layers[0].locked = true;
        assert!(doc.selection_for_stroke(1, false).is_empty());
        doc.layers[0].locked = false;
        doc.layers[0].visible = false;
        assert!(doc.selection_for_stroke(1, true).is_empty());
        assert!(doc.selection_for_stroke(999, false).is_empty());
    }
    #[test]
    fn invalid_group_ids_and_cross_layer_groups_are_rejected_but_singletons_are_valid() {
        let doc = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![mark(1)],
            ..Default::default()
        };
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_ok());
        for id in [0, u64::MAX] {
            let mut bad = doc.clone();
            bad.strokes[0].group_id = Some(id);
            assert!(Document::from_json(&serde_json::to_string(&bad).unwrap()).is_err());
        }
        let mut bad = doc;
        bad.layers.push(Layer {
            id: 2,
            name: "Other".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: LayerBlendMode::Normal,
            mask_source: None,
            clip_to_below: false,
        });
        let mut second = mark(2);
        second.layer_id = 2;
        bad.strokes.push(second);
        assert!(Document::from_json(&serde_json::to_string(&bad).unwrap()).is_err());
    }
}
