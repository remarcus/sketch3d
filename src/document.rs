pub use crate::dynamics::{Dynamics, PressureMapping};
use crate::texture::TextureSettings;
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushKind {
    #[default]
    Round,
    Ribbon,
    Chisel,
}

/// Brush orientation is captured in world space when the stroke begins.
/// Thickness is a fraction of width, so uniform scaling only changes radius.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub kind: BrushKind,
    pub thickness: f32,
    pub angle_degrees: f32,
    pub normal: Vec3,
    pub tangent: Vec3,
    #[serde(default)]
    pub dynamics: Dynamics,
    #[serde(default)]
    pub texture: TextureSettings,
}
impl Default for Brush {
    fn default() -> Self {
        Self {
            kind: BrushKind::Round,
            thickness: 0.2,
            angle_degrees: 0.0,
            normal: Vec3::Z,
            tangent: Vec3::X,
            dynamics: Dynamics::default(),
            texture: TextureSettings::default(),
        }
    }
}
impl Brush {
    pub fn is_valid(&self) -> bool {
        self.dynamics.is_valid()
            && self.texture.is_valid()
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub position: Vec3,
    pub pressure: f32,
    #[serde(default)]
    pub tilt: Option<Vec3>,
}
impl Point {
    pub fn is_valid(&self) -> bool {
        self.position.is_finite()
            && self.position.abs().max_element() <= 10000.0
            && self.pressure.is_finite()
            && (0.0..=1.0).contains(&self.pressure)
            && self
                .tilt
                .is_none_or(|tilt| tilt.is_finite() && (tilt.length_squared() - 1.0).abs() <= 0.001)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
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
    }]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    #[serde(default = "default_layer_id")]
    pub layer_id: u64,
    pub id: u64,
    pub color: [f32; 3],
    pub radius: f32,
    #[serde(default)]
    pub brush: Brush,
    pub points: Vec<Point>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub layers: Vec<Layer>,
    pub active_layer: u64,
    pub version: u32,
    pub strokes: Vec<Stroke>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            layers: layers_default(),
            active_layer: 1,
            version: 5,
            strokes: Vec::new(),
        }
    }
}
impl Document {
    pub fn visible(&self, stroke: &Stroke) -> bool {
        self.layers
            .iter()
            .any(|l| l.id == stroke.layer_id && l.visible)
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
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if !matches!(version, Some(1..=5)) {
            return Err("Unsupported document version".into());
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
        doc.version = 5;
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
                || !names.insert(layer.name.to_lowercase())
            {
                return Err("Invalid or duplicate layer metadata".into());
            }
        }
        if !layer_ids.contains(&doc.active_layer) {
            return Err("Active layer does not exist".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut total = 0;
        for stroke in &doc.strokes {
            if !layer_ids.contains(&stroke.layer_id)
                || !ids.insert(stroke.id)
                || stroke.id == u64::MAX
            {
                return Err("Invalid stroke IDs".into());
            }
            total += stroke.points.len();
            if total > 2_000_000
                || !stroke.brush.is_valid()
                || !stroke.radius.is_finite()
                || !(0.001..=1.0).contains(&stroke.radius)
                || stroke
                    .color
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || stroke.points.iter().any(|p| !p.is_valid())
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
            layer_id: 1,
            id: 1,
            color: [0.5; 3],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::ONE,
                pressure: 0.7,
                tilt: None,
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
                angle_degrees: -42.0,
                normal: Vec3::new(1.0, 2.0, 3.0).normalize(),
                tangent: Vec3::new(2.0, -1.0, 0.0).normalize(),
                dynamics: Dynamics::default(),
                texture: TextureSettings::default(),
            };
            s.brush.dynamics.width.minimum = 0.05;
            s.brush.dynamics.width.curve = 2.0;
            s.brush.dynamics.thickness.enabled = false;
            s.brush.dynamics.taper_start = 0.4;
            s.brush.dynamics.taper_end = 0.7;
            s.brush.dynamics.tilt_orientation = true;
            s.points[0].tilt = Some(Vec3::X);
            let doc = Document {
                layers: layers_default(),
                active_layer: 1,
                version: 5,
                strokes: vec![s],
            };
            let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
            assert_eq!(loaded.version, 5);
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
        })
        .unwrap();
        // Even an unrecognized extension must not opt a version-1 stroke into
        // a new shape, or prevent an otherwise valid legacy file from opening.
        old["strokes"][0]["brush"] = serde_json::json!({"kind": "future_brush"});
        let loaded = Document::from_json(&old.to_string()).unwrap();
        assert_eq!(loaded.version, 5);
        assert_eq!(loaded.strokes, vec![original]);
        old["strokes"][0].as_object_mut().unwrap().remove("brush");
        let without_extension = Document::from_json(&old.to_string()).unwrap();
        assert_eq!(without_extension.strokes, loaded.strokes);
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
                version: 5,
                strokes: vec![s],
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
            version: 5,
            strokes: vec![stroke(), stroke()],
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
        })
        .unwrap();
        value["strokes"][0]["brush"]["dynamics"] = serde_json::json!({"future":true});
        value["strokes"][0]["points"][0]["tilt"] = serde_json::json!("unknown extension");
        let loaded = Document::from_json(&value.to_string()).unwrap();
        assert_eq!(loaded.version, 5);
        assert_eq!(loaded.strokes, vec![original]);
    }
    #[test]
    fn rejects_invalid_dynamics_and_tilt_metadata() {
        let original = Document {
            layers: layers_default(),
            active_layer: 1,
            version: 5,
            strokes: vec![stroke()],
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
            version: 5,
            strokes: vec![mark.clone()],
        };
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes, doc.strokes);
        let mut legacy = serde_json::to_value(doc).unwrap();
        legacy["version"] = serde_json::json!(3);
        legacy["strokes"][0]["brush"]["texture"] = serde_json::json!({"kind":"future_pattern"});
        let loaded = Document::from_json(&legacy.to_string()).unwrap();
        mark.brush.texture = TextureSettings::default();
        assert_eq!(loaded.version, 5);
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
                version: 5,
                strokes: vec![stroke()],
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
            id: 8,
            layer_id: 1,
            color: [0.2, 0.4, 0.7],
            radius: 0.05,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::new(2.0, 3.0, 4.0),
                pressure: 0.37,
                tilt: Some(Vec3::Y),
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
        assert_eq!(migrated.version, 5);
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
        });
        doc.active_layer = 2;
        let stroke = Stroke {
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
                }),
                "blank_name" => doc.layers[0].name = " ".into(),
                "active_missing" => doc.active_layer = 2,
                "orphan" => doc.strokes.push(Stroke {
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
