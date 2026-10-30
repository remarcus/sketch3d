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
}
impl Default for Brush {
    fn default() -> Self {
        Self {
            kind: BrushKind::Round,
            thickness: 0.2,
            angle_degrees: 0.0,
            normal: Vec3::Z,
            tangent: Vec3::X,
        }
    }
}
impl Brush {
    pub fn is_valid(&self) -> bool {
        self.thickness.is_finite()
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
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: u64,
    pub color: [f32; 3],
    pub radius: f32,
    #[serde(default)]
    pub brush: Brush,
    pub points: Vec<Point>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    pub strokes: Vec<Stroke>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 2,
            strokes: Vec::new(),
        }
    }
}
impl Document {
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if !matches!(version, Some(1 | 2)) {
            return Err("Unsupported document version".into());
        }
        // Version 1 has no brush metadata. Ignore an unknown extension with
        // that name rather than changing the appearance of a legacy drawing.
        if version == Some(1) {
            if let Some(strokes) = value.get_mut("strokes").and_then(|v| v.as_array_mut()) {
                for stroke in strokes {
                    if let Some(fields) = stroke.as_object_mut() {
                        fields.remove("brush");
                    }
                }
            }
        }
        let mut doc: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        doc.version = 2;
        if doc.strokes.len() > 100_000 {
            return Err("Too many strokes".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut total = 0;
        for stroke in &doc.strokes {
            if !ids.insert(stroke.id) || stroke.id == u64::MAX {
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
                || stroke.points.iter().any(|p| {
                    !p.position.is_finite()
                        || p.position.abs().max_element() > 10000.0
                        || !p.pressure.is_finite()
                        || !(0.0..=1.0).contains(&p.pressure)
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
            id: 1,
            color: [0.5; 3],
            radius: 0.02,
            brush: Brush::default(),
            points: vec![Point {
                position: Vec3::ONE,
                pressure: 0.7,
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
            };
            let doc = Document {
                version: 2,
                strokes: vec![s],
            };
            let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
            assert_eq!(loaded.version, 2);
            assert_eq!(loaded.strokes, doc.strokes);
        }
    }
    #[test]
    fn legacy_documents_upgrade_without_changing_ink() {
        let original = stroke();
        let mut old = serde_json::to_value(Document {
            version: 1,
            strokes: vec![original.clone()],
        })
        .unwrap();
        // Even an unrecognized extension must not opt a version-1 stroke into
        // a new shape, or prevent an otherwise valid legacy file from opening.
        old["strokes"][0]["brush"] = serde_json::json!({"kind": "future_brush"});
        let loaded = Document::from_json(&old.to_string()).unwrap();
        assert_eq!(loaded.version, 2);
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
                version: 2,
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
            version: 2,
            strokes: vec![stroke(), stroke()],
        };
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        doc.strokes.pop();
        doc.strokes[0].points[0].pressure = 1.1;
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
    }
}
