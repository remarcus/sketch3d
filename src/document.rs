use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Point {
    pub position: Vec3,
    pub pressure: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub id: u64,
    pub color: [f32; 3],
    pub radius: f32,
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
            version: 1,
            strokes: Vec::new(),
        }
    }
}
impl Document {
    pub fn from_json(text: &str) -> Result<Self, String> {
        let doc: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if doc.version != 1 {
            return Err("Unsupported document version".into());
        }
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
            points: vec![Point {
                position: Vec3::ONE,
                pressure: 0.7,
            }],
        }
    }
    #[test]
    fn round_trip() {
        let doc = Document {
            version: 1,
            strokes: vec![stroke()],
        };
        let loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(loaded.strokes[0].points[0].position, Vec3::ONE);
    }
    #[test]
    fn rejects_invalid_documents() {
        assert!(Document::from_json("{\"version\":99,\"strokes\":[]}").is_err());
        let mut doc = Document {
            version: 1,
            strokes: vec![stroke(), stroke()],
        };
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        doc.strokes.pop();
        doc.strokes[0].points[0].pressure = 1.1;
        assert!(Document::from_json(&serde_json::to_string(&doc).unwrap()).is_err());
    }
}
