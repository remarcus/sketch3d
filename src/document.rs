pub use crate::dynamics::{
    AirbrushSettings, DabSettings, Dynamics, PressureMapping, StrandSettings,
};
use crate::texture::TextureSettings;
use glam::Vec3;
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
            strands: StrandSettings::default(),
            flow: AirbrushSettings::default(),
            dab_seed: 0,
            tip_asset_id: 0,
            tip_aspect: 1.0,
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
    }]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
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
    pub radius: f32,
    #[serde(default)]
    pub brush: Brush,
    pub points: Vec<Point>,
    /// Exact control points for curves created by the smooth curve tool.
    /// The sampled points remain authoritative for rendering and export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<CurveData>,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveAnchor {
    pub position: Vec3,
    pub pressure: f32,
    pub incoming: Vec3,
    pub outgoing: Vec3,
    pub manual: bool,
}

impl CurveData {
    pub fn is_valid(&self) -> bool {
        let unit = |v: Vec3| v.is_finite() && (v.length_squared() - 1.0).abs() <= 0.001;
        self.anchors.len() >= 2
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
            })
            && self.anchors.iter().all(|anchor| {
                anchor.position.is_finite()
                    && anchor.position.abs().max_element() <= 10_000.0
                    && anchor.pressure.is_finite()
                    && (0.0..=1.0).contains(&anchor.pressure)
                    && anchor.incoming.is_finite()
                    && anchor.incoming.abs().max_element() <= 10_000.0
                    && anchor.outgoing.is_finite()
                    && anchor.outgoing.abs().max_element() <= 10_000.0
                    && anchor.incoming.dot(self.frame_normal).abs() <= 0.01
                    && anchor.outgoing.dot(self.frame_normal).abs() <= 0.01
                    && (anchor.position - self.frame_origin)
                        .dot(self.frame_normal)
                        .abs()
                        <= 0.01
            })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanarFill {
    /// Physical separation behind the captured guide normal, in world units.
    pub offset: f32,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub layers: Vec<Layer>,
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
}
impl Default for Document {
    fn default() -> Self {
        Self {
            layers: layers_default(),
            active_layer: 1,
            version: 22,
            strokes: Vec::new(),
            image_planes: Vec::new(),
            active_image_plane: None,
            guide_resources: Vec::new(),
            active_guide: None,
            model_references: Vec::new(),
            active_model_reference: None,
            brush_tip_assets: Vec::new(),
        }
    }
}
impl Document {
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
            .any(|l| l.id == stroke.layer_id && l.visible)
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
    pub fn editable(&self, stroke: &Stroke) -> bool {
        self.layers
            .iter()
            .any(|l| l.id == stroke.layer_id && l.visible && !l.locked)
    }
    pub fn active_editable(&self) -> bool {
        self.layers
            .iter()
            .any(|l| l.id == self.active_layer && l.visible && !l.locked)
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
        if !matches!(version, Some(1..=22)) {
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
        doc.version = 22;
        if doc.strokes.len() > 100_000 {
            return Err("Too many strokes".into());
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
        model_references::validate_catalog(&doc.model_references, doc.active_model_reference)?;
        let mut ids = std::collections::HashSet::new();
        let mut total = 0;
        let mut total_dabs = 0usize;
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
                crate::polygon::triangulate(stroke)?;
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
                || stroke.points.iter().any(|p| !p.is_valid())
                || stroke.curve.as_ref().is_some_and(|curve| {
                    !curve.is_valid()
                        || stroke.points.len() > crate::curve_geometry::MAX_CURVE_POINTS
                        || stroke
                            .points
                            .first()
                            .is_none_or(|point| point.position != curve.anchors[0].position)
                        || stroke.points.last().is_none_or(|point| {
                            point.position != curve.anchors[curve.anchors.len() - 1].position
                        })
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
            };
            let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
            assert_eq!(loaded.version, 22);
            assert_eq!(loaded.strokes, doc.strokes);
        }
    }
    #[test]
    fn legacy_documents_upgrade_without_changing_ink() {
        let original = stroke();
        let mut old = serde_json::to_value(Document {
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
        })
        .unwrap();
        // Even an unrecognized extension must not opt a version-1 stroke into
        // a new shape, or prevent an otherwise valid legacy file from opening.
        old["strokes"][0]["brush"] = serde_json::json!({"kind": "future_brush"});
        let loaded = Document::from_json(&old.to_string()).unwrap();
        assert_eq!(loaded.version, 22);
        assert_eq!(loaded.strokes, vec![original]);
        old["strokes"][0].as_object_mut().unwrap().remove("brush");
        let without_extension = Document::from_json(&old.to_string()).unwrap();
        assert_eq!(without_extension.strokes, loaded.strokes);
    }
    #[test]
    fn version_seven_migration_defaults_missing_opacity_to_opaque() {
        let mut value = serde_json::to_value(Document {
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
        assert_eq!(loaded.version, 22);
        assert_eq!(loaded.strokes[0].brush.opacity, 1.0);
        assert_eq!(loaded.strokes[0].brush.softness, 0.0);
        assert_eq!(
            loaded.strokes[0].brush.dynamics.opacity,
            Dynamics::default().opacity
        );

        let saved = serde_json::to_string(&loaded).unwrap();
        let reopened = Document::from_json(&saved).unwrap();
        assert_eq!(reopened.version, 22);
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
        })
        .unwrap();
        value["strokes"][0]["brush"]["dynamics"] = serde_json::json!({"future":true});
        value["strokes"][0]["points"][0]["tilt"] = serde_json::json!("unknown extension");
        let loaded = Document::from_json(&value.to_string()).unwrap();
        assert_eq!(loaded.version, 22);
        assert_eq!(loaded.strokes, vec![original]);
    }
    #[test]
    fn rejects_invalid_dynamics_and_tilt_metadata() {
        let original = Document {
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
            strength: 0.82,
            seed: 1234,
        };
        let doc = Document {
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
        };
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, doc.strokes);
        let mut legacy = serde_json::to_value(doc).unwrap();
        legacy["version"] = serde_json::json!(3);
        legacy["strokes"][0]["brush"]["texture"] = serde_json::json!({"kind":"future_pattern"});
        let loaded = Document::from_json(&legacy.to_string()).unwrap();
        mark.brush.texture = TextureSettings::default();
        assert_eq!(loaded.version, 22);
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
            strokes: vec![stroke],
            ..Default::default()
        })
        .unwrap();
        value["version"] = serde_json::json!(4);
        value["layers"] = serde_json::json!("unknown extension");
        value["active_layer"] = serde_json::json!(99);
        value["strokes"][0]["layer_id"] = serde_json::json!("unknown");
        let migrated = Document::from_json(&value.to_string()).unwrap();
        assert_eq!(migrated.version, 22);
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
        });
        doc.active_layer = 2;
        let stroke = Stroke {
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
                }),
                "blank_name" => doc.layers[0].name = " ".into(),
                "active_missing" => doc.active_layer = 2,
                "orphan" => doc.strokes.push(Stroke {
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
        assert_eq!(loaded.version, 22);
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
    fn legacy_five_preserves_resolved_ink_and_layers_but_ignores_group_extensions() {
        let mut doc = Document {
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
        assert_eq!(loaded.version, 22);
        assert_eq!(loaded.strokes, doc.strokes);
        assert_eq!(loaded.layers, doc.layers);
    }
    #[test]
    fn groups_roundtrip_select_as_a_unit_and_allow_individual_access() {
        let mut doc = Document {
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
        });
        let mut second = mark(2);
        second.layer_id = 2;
        bad.strokes.push(second);
        assert!(Document::from_json(&serde_json::to_string(&bad).unwrap()).is_err());
    }
}
