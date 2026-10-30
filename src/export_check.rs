//! Reproducible export fixtures for independent glTF and Blender verification.
use crate::{
    document::{Document, Point, Stroke},
    export, geometry, scenes,
};
use glam::Vec3;
use serde_json::json;
use std::collections::HashSet;

pub fn pressure_fixture() -> Document {
    Document {
        version: 1,
        strokes: vec![
            Stroke {
                id: 1001,
                color: [0.17, 0.82, 0.74],
                radius: 0.12,
                points: vec![
                    Point {
                        position: Vec3::ZERO,
                        pressure: 0.0,
                    },
                    Point {
                        position: Vec3::new(0.0, 0.5, 0.0),
                        pressure: 0.3,
                    },
                    Point {
                        position: Vec3::new(0.0, 1.2, 0.0),
                        pressure: 1.0,
                    },
                ],
            },
            Stroke {
                id: 1002,
                color: [0.94, 0.35, 0.14],
                radius: 0.08,
                points: vec![Point {
                    position: Vec3::new(1.5, 0.25, 0.75),
                    pressure: 0.7,
                }],
            },
            Stroke {
                id: 1003,
                color: [0.35, 0.45, 0.9],
                radius: 0.045,
                points: vec![
                    Point {
                        position: Vec3::new(0.0, 2.0, -1.0),
                        pressure: 1.0,
                    },
                    Point {
                        position: Vec3::new(1.2, 2.0, -1.0),
                        pressure: 1.0,
                    },
                ],
            },
        ],
    }
}
pub fn run() -> Result<(), String> {
    let directory = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("verification/exports");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let pressure = pressure_fixture();
    let fixtures = vec![
        ("house", scenes::japanese_house(), None),
        ("lantern", scenes::cylindrical_lantern(), None),
        ("pressure", pressure.clone(), None),
        ("selection", pressure, Some(HashSet::from([1002]))),
    ];
    let mut cases = Vec::new();
    for (name, doc, selected) in fixtures {
        let original = serde_json::to_string(&doc).map_err(|e| e.to_string())?;
        let file = format!("{name}.glb");
        let summary = export::write_glb(&doc, selected.as_ref(), &directory.join(&file))?;
        if serde_json::to_string(&doc).map_err(|e| e.to_string())? != original {
            return Err("Export modified its source document".into());
        }
        let mut strokes = Vec::new();
        for stroke in doc.strokes.iter().filter(|s| {
            !s.points.is_empty() && selected.as_ref().is_none_or(|ids| ids.contains(&s.id))
        }) {
            let geometry = geometry::tube(stroke);
            let min = geometry.iter().fold(Vec3::splat(f32::INFINITY), |min, v| {
                min.min(Vec3::from(v.position))
            });
            let max = geometry
                .iter()
                .fold(Vec3::splat(f32::NEG_INFINITY), |max, v| {
                    max.max(Vec3::from(v.position))
                });
            strokes.push(json!({"name":format!("Stroke_{}",stroke.id),"id":stroke.id.to_string(),"color":stroke.color,"min":min.to_array(),"max":max.to_array()}));
        }
        cases.push(json!({"name":name,"file":file,"summary":summary,"strokes":strokes}));
    }
    let report = json!({"passed":true,"axes":"+X right, +Y up, +Z front","unitInMeters":1.0,"cases":cases,"independentValidationRequired":["Khronos glTF Validator","Blender import"]});
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
