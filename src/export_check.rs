//! Reproducible export fixtures for independent glTF and Blender verification.
use crate::{
    document::{Brush, BrushKind, Document, Point, Stroke},
    export, geometry, scenes,
};
use glam::Vec3;
use serde_json::json;
use std::collections::HashSet;

pub fn pressure_fixture() -> Document {
    Document {
        version: 2,
        strokes: vec![
            Stroke {
                id: 1001,
                color: [0.17, 0.82, 0.74],
                radius: 0.12,
                brush: Default::default(),
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
                brush: Default::default(),
                points: vec![Point {
                    position: Vec3::new(1.5, 0.25, 0.75),
                    pressure: 0.7,
                }],
            },
            Stroke {
                id: 1003,
                color: [0.35, 0.45, 0.9],
                radius: 0.045,
                brush: Default::default(),
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
/// A small pressure-shaped sampler that also exercises double-sided materials and dots.
pub fn brush_fixture() -> Document {
    let mut doc = Document::default();
    for (row, kind) in [BrushKind::Round, BrushKind::Ribbon, BrushKind::Chisel]
        .into_iter()
        .enumerate()
    {
        let brush = Brush {
            kind,
            thickness: 0.25,
            angle_degrees: if kind == BrushKind::Chisel { 35.0 } else { 0.0 },
            normal: Vec3::Z,
            tangent: Vec3::X,
        };
        let color = [[0.17, 0.82, 0.74], [0.94, 0.35, 0.14], [0.35, 0.45, 0.9]][row];
        let y = 1.2 - row as f32 * 1.2;
        doc.strokes.push(Stroke {
            id: 2001 + row as u64 * 2,
            color,
            radius: 0.16,
            brush,
            points: (0..=40)
                .map(|step| {
                    let t = step as f32 / 40.0;
                    Point {
                        position: Vec3::new(
                            -2.0 + 4.0 * t,
                            y + (t * std::f32::consts::TAU).sin() * 0.24,
                            0.0,
                        ),
                        pressure: 0.1 + 0.9 * t,
                    }
                })
                .collect(),
        });
        let dot_rotation =
            glam::Quat::from_rotation_arc(Vec3::Z, Vec3::new(0.3, 0.2, 1.0).normalize());
        doc.strokes.push(Stroke {
            id: 2002 + row as u64 * 2,
            color,
            radius: 0.16,
            brush: Brush {
                normal: dot_rotation * Vec3::Z,
                tangent: dot_rotation * Vec3::X,
                ..brush
            },
            points: vec![Point {
                position: Vec3::new(2.6, y, 0.0),
                pressure: 1.0,
            }],
        });
    }
    doc
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
        ("brushes", brush_fixture(), None),
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
            let geometry = geometry::stroke_mesh(stroke);
            let min = geometry.iter().fold(Vec3::splat(f32::INFINITY), |min, v| {
                min.min(Vec3::from(v.position))
            });
            let max = geometry
                .iter()
                .fold(Vec3::splat(f32::NEG_INFINITY), |max, v| {
                    max.max(Vec3::from(v.position))
                });
            strokes.push(json!({"name":format!("Stroke_{}",stroke.id),"id":stroke.id.to_string(),"color":stroke.color,"doubleSided":stroke.brush.kind == BrushKind::Ribbon,"min":min.to_array(),"max":max.to_array()}));
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
