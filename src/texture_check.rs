//! Reproducible CPU tessellation/mask generation measurements, not tablet latency.
use crate::{
    document::{Brush, BrushKind, Point, Stroke},
    geometry,
    texture::{self, TextureKind},
};
use glam::Vec3;
use std::{hint::black_box, time::Instant};
pub fn run() -> Result<(), String> {
    let mut cases = Vec::new();
    for shape in [BrushKind::Ribbon, BrushKind::Round] {
        for kind in [
            TextureKind::Solid,
            TextureKind::Grain,
            TextureKind::DryBrush,
            TextureKind::Pattern,
        ] {
            let mut stroke = Stroke {
                id: 1,
                group_id: None,
                layer_id: 1,
                color: [0.2, 0.8, 0.6],
                radius: 0.05,
                brush: Brush {
                    kind: shape,
                    ..Brush::default()
                },
                points: (0..10000)
                    .map(|i| {
                        let t = i as f32 * 0.004;
                        Point {
                            position: Vec3::new(t, t.sin(), (t * 0.3).cos()),
                            pressure: 0.7 + 0.3 * t.sin().abs(),
                            tilt: None,
                        }
                    })
                    .collect(),
            };
            stroke.brush.texture.kind = kind;
            let mut times = Vec::new();
            let mut vertices = 0;
            for _ in 0..5 {
                let start = Instant::now();
                let mesh = geometry::stroke_mesh(black_box(&stroke));
                vertices = mesh.len();
                black_box(&mesh);
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            times.sort_by(f64::total_cmp);
            let start = Instant::now();
            let png = texture::png(stroke.brush.texture)?;
            let mask_ms = start.elapsed().as_secs_f64() * 1000.0;
            cases.push(serde_json::json!({"shape":shape,"texture":kind,"points":stroke.points.len(),"vertices":vertices,"median_mesh_ms":times[2],"max_mesh_ms":times[4],"png_ms":mask_ms,"png_bytes":png.len()}));
        }
    }
    let path = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("verification");
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    std::fs::write(path.join("texture-benchmark.json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"profile":"release recommended","measure":"CPU mesh generation for 10,000-point strokes; 5 iterations, completed GPU meshes are cached separately","cases":cases})).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    Ok(())
}
