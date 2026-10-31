//! Reproducible export fixtures for independent glTF and Blender verification.
use crate::{
    document::{Brush, BrushKind, Document, Point, Stroke},
    export, geometry, scenes,
    texture::{self, TextureKind, TextureSettings},
};
use glam::Vec3;
use serde_json::json;
use std::collections::HashSet;

pub fn pressure_fixture() -> Document {
    Document {
        version: 7,
        strokes: vec![
            Stroke {
                id: 1001,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: [0.17, 0.82, 0.74],
                radius: 0.12,
                brush: Default::default(),
                points: vec![
                    Point {
                        tilt: None,
                        position: Vec3::ZERO,
                        pressure: 0.0,
                    },
                    Point {
                        tilt: None,
                        position: Vec3::new(0.0, 0.5, 0.0),
                        pressure: 0.3,
                    },
                    Point {
                        tilt: None,
                        position: Vec3::new(0.0, 1.2, 0.0),
                        pressure: 1.0,
                    },
                ],
            },
            Stroke {
                id: 1002,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: [0.94, 0.35, 0.14],
                radius: 0.08,
                brush: Default::default(),
                points: vec![Point {
                    tilt: None,
                    position: Vec3::new(1.5, 0.25, 0.75),
                    pressure: 0.7,
                }],
            },
            Stroke {
                id: 1003,
                fill: None,
                group_id: None,
                layer_id: 1,
                color: [0.35, 0.45, 0.9],
                radius: 0.045,
                brush: Default::default(),
                points: vec![
                    Point {
                        tilt: None,
                        position: Vec3::new(0.0, 2.0, -1.0),
                        pressure: 1.0,
                    },
                    Point {
                        tilt: None,
                        position: Vec3::new(1.2, 2.0, -1.0),
                        pressure: 1.0,
                    },
                ],
            },
        ],
        ..Document::default()
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
            dynamics: Default::default(),
            texture: Default::default(),
        };
        let color = [[0.17, 0.82, 0.74], [0.94, 0.35, 0.14], [0.35, 0.45, 0.9]][row];
        let y = 1.2 - row as f32 * 1.2;
        doc.strokes.push(Stroke {
            id: 2001 + row as u64 * 2,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius: 0.16,
            brush,
            points: (0..=40)
                .map(|step| {
                    let t = step as f32 / 40.0;
                    Point {
                        tilt: None,
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
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius: 0.16,
            brush: Brush {
                normal: dot_rotation * Vec3::Z,
                tangent: dot_rotation * Vec3::X,
                ..brush
            },
            points: vec![Point {
                tilt: None,
                position: Vec3::new(2.6, y, 0.0),
                pressure: 1.0,
            }],
        });
    }
    doc
}
/// Pressure, tilt, taper, and dot examples used by both the app and export checks.
pub fn dynamics_fixture() -> Document {
    let mut doc = Document::default();
    for row in 0..4 {
        let mut brush = Brush {
            kind: match row {
                1 => BrushKind::Chisel,
                2 => BrushKind::Ribbon,
                _ => BrushKind::Round,
            },
            thickness: 0.7,
            ..Brush::default()
        };
        match row {
            0 => {
                brush.dynamics.width.minimum = 0.05;
                brush.dynamics.width.curve = 1.6;
            }
            1 => {
                brush.dynamics.width.enabled = false;
                brush.dynamics.thickness.minimum = 0.04;
                brush.dynamics.thickness.curve = 2.0;
            }
            2 => {
                brush.dynamics.tilt_orientation = true;
                brush.dynamics.width.minimum = 0.3;
                brush.dynamics.taper_start = 0.4;
                brush.dynamics.taper_end = 0.6;
            }
            _ => {
                brush.dynamics.width.enabled = false;
                brush.dynamics.taper_start = 0.8;
                brush.dynamics.taper_end = 1.0;
            }
        }
        let color = [
            [0.17, 0.82, 0.74],
            [0.96, 0.55, 0.3],
            [0.47, 0.58, 0.95],
            [0.94, 0.80, 0.39],
        ][row];
        let y = 1.8 - row as f32 * 1.2;
        doc.strokes.push(Stroke {
            id: 3001 + row as u64 * 2,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius: 0.18,
            brush,
            points: (0..=60)
                .map(|i| {
                    let t = i as f32 / 60.0;
                    let angle = t * std::f32::consts::PI * 1.3 - 0.4;
                    Point {
                        position: Vec3::new(
                            -2.0 + 4.0 * t,
                            y + (t * std::f32::consts::TAU).sin() * 0.18,
                            0.0,
                        ),
                        pressure: 0.05 + 0.95 * t,
                        tilt: (row == 2).then_some(Vec3::new(0.0, angle.cos(), angle.sin())),
                    }
                })
                .collect(),
        });
        doc.strokes.push(Stroke {
            id: 3002 + row as u64 * 2,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius: 0.18,
            brush,
            points: vec![Point {
                position: Vec3::new(2.6, y, 0.0),
                pressure: 1.0,
                tilt: (row == 2).then_some(Vec3::new(0.0, 0.7, 0.7).normalize()),
            }],
        });
    }
    doc
}
/// Original procedural masks sampled on ribbon, chisel, round, and dot geometry.
pub fn textured_fixture() -> Document {
    let mut doc = Document::default();
    for (row, texture_kind) in [
        TextureKind::Solid,
        TextureKind::Grain,
        TextureKind::DryBrush,
        TextureKind::Pattern,
    ]
    .into_iter()
    .enumerate()
    {
        let brush = Brush {
            kind: if row == 2 {
                BrushKind::Chisel
            } else {
                BrushKind::Ribbon
            },
            thickness: 0.3,
            texture: TextureSettings {
                kind: texture_kind,
                scale: 0.3,
                strength: 0.8,
                seed: 42 + row as u32,
            },
            ..Brush::default()
        };
        let color = [
            [0.17, 0.82, 0.74],
            [0.96, 0.55, 0.3],
            [0.47, 0.58, 0.95],
            [0.94, 0.80, 0.39],
        ][row];
        let y = 1.8 - row as f32 * 1.2;
        doc.strokes.push(Stroke {
            id: 4001 + row as u64 * 3,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius: 0.18,
            brush,
            points: (0..=60)
                .map(|i| {
                    let t = i as f32 / 60.0;
                    Point {
                        position: Vec3::new(
                            -2.0 + 4.0 * t,
                            y + (t * std::f32::consts::TAU).sin() * 0.2,
                            0.0,
                        ),
                        pressure: 0.8 + 0.2 * t,
                        tilt: None,
                    }
                })
                .collect(),
        });
        doc.strokes.push(Stroke {
            id: 4002 + row as u64 * 3,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius: 0.18,
            brush,
            points: vec![Point {
                position: Vec3::new(2.6, y, 0.0),
                pressure: 1.0,
                tilt: None,
            }],
        });
        // A short depth-oriented round companion exercises circumference UV seams.
        doc.strokes.push(Stroke {
            id: 4003 + row as u64 * 3,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius: 0.11,
            brush: Brush {
                kind: BrushKind::Round,
                ..brush
            },
            points: (0..=12)
                .map(|i| Point {
                    position: Vec3::new(-2.0 + i as f32 / 12.0 * 4.0, y - 0.37, -0.15),
                    pressure: 1.0,
                    tilt: None,
                })
                .collect(),
        });
    }
    doc
}
/// Visible locked artwork exports; hidden artwork is excluded even if selected.
pub fn layered_fixture() -> Document {
    let mut doc = pressure_fixture();
    doc.layers.push(crate::document::Layer {
        id: 2,
        name: "Hidden study".into(),
        visible: false,
        locked: false,
    });
    doc.layers.push(crate::document::Layer {
        id: 3,
        name: "Protected ink".into(),
        visible: true,
        locked: true,
    });
    doc.strokes[1].layer_id = 2;
    doc.strokes[1].color = [1.0, 0.0, 1.0];
    doc.strokes[2].layer_id = 3;
    doc
}
/// Original painted pottery study made only from editable strokes on a vase guide.
pub fn vase_fixture() -> Document {
    let guide = crate::revolved::Revolved::default();
    let mut doc = Document::default();
    let mut add = |coordinates: Vec<(f32, f32)>, color: [f32; 3], radius: f32, kind: BrushKind| {
        let first = guide.point(coordinates[0].0, coordinates[0].1);
        let brush = Brush {
            kind,
            thickness: 0.18,
            normal: guide.normal(first),
            tangent: guide.tangent(first),
            ..Default::default()
        };
        let points = coordinates
            .into_iter()
            .enumerate()
            .map(|(i, (theta, height))| Point {
                position: guide.point(theta, height),
                pressure: 0.9 + 0.08 * (i as f32 * 0.3).sin().abs(),
                tilt: None,
            })
            .collect();
        doc.strokes.push(Stroke {
            id: 6001 + doc.strokes.len() as u64,
            fill: None,
            group_id: None,
            layer_id: 1,
            color,
            radius,
            brush,
            points,
        });
    };
    for i in 0..56 {
        let theta = i as f32 / 56.0 * std::f32::consts::TAU;
        let shade = 0.035 * (i as f32 * 1.7).sin();
        add(
            (0..=80)
                .map(|j| {
                    (
                        theta,
                        guide.min_height()
                            + (guide.max_height() - guide.min_height()) * j as f32 / 80.0,
                    )
                })
                .collect(),
            [0.43 + shade, 0.20 + shade * 0.5, 0.095],
            0.095,
            BrushKind::Ribbon,
        );
    }
    for height in [-0.05, 0.1, 0.25] {
        add(
            (0..=128)
                .map(|i| (i as f32 / 128.0 * std::f32::consts::TAU, height))
                .collect(),
            [0.055, 0.19, 0.29],
            0.11,
            BrushKind::Chisel,
        );
    }
    for i in 0..12 {
        let theta = i as f32 / 12.0 * std::f32::consts::TAU;
        let corners = [
            (theta - 0.13, 0.1),
            (theta, 0.22),
            (theta + 0.13, 0.1),
            (theta, -0.02),
            (theta - 0.13, 0.1),
        ];
        let mut coordinates = Vec::new();
        for pair in corners.windows(2) {
            for j in 0..=8 {
                let t = j as f32 / 8.0;
                coordinates.push((
                    pair[0].0 + (pair[1].0 - pair[0].0) * t,
                    pair[0].1 + (pair[1].1 - pair[0].1) * t,
                ));
            }
        }
        add(coordinates, [0.94, 0.80, 0.52], 0.035, BrushKind::Round);
    }
    for height in [guide.min_height() + 0.03, guide.max_height() - 0.03] {
        add(
            (0..=128)
                .map(|i| (i as f32 / 128.0 * std::f32::consts::TAU, height))
                .collect(),
            [0.84, 0.56, 0.25],
            0.025,
            BrushKind::Round,
        );
    }
    doc
}
/// Two pressure-shaped marks share a flat group; the third remains independent.
pub fn grouped_fixture() -> Document {
    let mut doc = pressure_fixture();
    doc.strokes[0].group_id = Some(999);
    doc.strokes[1].group_id = Some(999);
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
        ("dynamics", dynamics_fixture(), None),
        ("textures", textured_fixture(), None),
        ("layers", layered_fixture(), None),
        ("vase", vase_fixture(), None),
        ("groups", grouped_fixture(), None),
        ("fills", crate::polygon::study(), None),
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
            !s.points.is_empty()
                && doc.visible(s)
                && selected.as_ref().is_none_or(|ids| ids.contains(&s.id))
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
            let uv_min = geometry.iter().fold([f32::INFINITY; 2], |a, v| {
                [a[0].min(v.uv[0]), a[1].min(v.uv[1])]
            });
            let uv_max = geometry.iter().fold([f32::NEG_INFINITY; 2], |a, v| {
                [a[0].max(v.uv[0]), a[1].max(v.uv[1])]
            });
            let masked = stroke.brush.texture.kind != TextureKind::Solid;
            let opaque_pixels = if masked {
                texture::rgba(stroke.brush.texture)
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[3] == 255)
                    .count()
            } else {
                0
            };
            strokes.push(json!({"name":format!("Stroke_{}",stroke.id),"id":stroke.id.to_string(),"groupId":stroke.group_id.map(|id|id.to_string()),"layerId":stroke.layer_id.to_string(),"layerName":doc.layers.iter().find(|l|l.id==stroke.layer_id).unwrap().name,"color":stroke.color,"doubleSided":stroke.fill.is_some() || stroke.brush.kind == BrushKind::Ribbon,"masked":masked,"opaquePixels":opaque_pixels,"uvMin":uv_min,"uvMax":uv_max,"min":min.to_array(),"max":max.to_array()}));
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
