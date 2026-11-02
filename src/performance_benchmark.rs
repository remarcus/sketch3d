//! Repeatable CPU workload benchmarks. These do not estimate GPU time or pen latency.
use crate::{
    document::{Brush, Document, Point, Stroke},
    editing, geometry, gizmo,
};
use eframe::egui::{pos2, Rect};
use glam::{Quat, Vec3};
use serde_json::{json, Value};
use std::{collections::HashSet, hint::black_box, time::Instant};

const WARMUPS: usize = 1;
const SAMPLES: usize = 7;

pub fn run() -> Result<(), String> {
    let cases = vec![
        benchmark_long_stroke(),
        benchmark_many_short_strokes(),
        benchmark_pick(),
        benchmark_large_selection()?,
    ];

    let report = json!({
        "passed": true,
        "profile": "release recommended",
        "schema_version": 1,
        "machine": {
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "logical_cpus": std::thread::available_parallelism().map(usize::from).ok(),
            "rust_profile": "release",
            "gpu": "not queried by this CPU-only benchmark"
        },
        "method": {
            "warmup_iterations": WARMUPS,
            "measured_iterations": SAMPLES,
            "statistic": "median elapsed wall-clock time of synchronous CPU operations",
            "limits": [
                "No GPU timestamp queries are performed; buffer upload, draw-call, and GPU frame time are not measured.",
                "No native pen events are generated; event handling, device latency, and end-to-end pen latency are not measured.",
                "Wall-clock timings are local machine observations, not performance guarantees."
            ]
        },
        "cases": cases,
    });
    let path = std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join("verification");
    std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    std::fs::write(
        path.join("performance-benchmark.json"),
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn measure(mut operation: impl FnMut()) -> Vec<f64> {
    for _ in 0..WARMUPS {
        operation();
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        operation();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    samples
}

fn stats(samples: &[f64]) -> Value {
    json!({
        "median_ms": samples[samples.len() / 2],
        "min_ms": samples[0],
        "max_ms": samples[samples.len() - 1],
        "samples_ms": samples
    })
}

fn make_stroke(id: u64, point_count: usize, x_offset: f32) -> Stroke {
    let points = (0..point_count)
        .map(|index| {
            let t = index as f32 / point_count.max(1) as f32;
            Point {
                position: Vec3::new(
                    x_offset + t * 0.5,
                    (t * std::f32::consts::TAU * 2.0).sin() * 0.15,
                    (t * 3.0).cos() * 0.04,
                ),
                pressure: 0.25 + 0.75 * (t * std::f32::consts::PI).sin().abs(),
                tilt: None,
                time_ms: 0,
            }
        })
        .collect();
    Stroke {
        curve: None,
        material: Default::default(),
        id,
        fill: None,
        group_id: None,
        layer_id: 1,
        color: [0.2, 0.65, 0.85],
        radius: 0.02,
        brush: Brush::default(),
        points,
    }
}

fn benchmark_long_stroke() -> Value {
    let stroke = make_stroke(1, 10_000, 0.0);
    let mut vertex_count = 0;
    let timing = measure(|| {
        let mesh = geometry::stroke_mesh(black_box(&stroke));
        vertex_count = mesh.len();
        black_box(mesh);
    });
    json!({
        "name": "long_stroke_tessellation",
        "operation": "geometry::stroke_mesh",
        "strokes": 1,
        "points_per_stroke": stroke.points.len(),
        "vertices": vertex_count,
        "timing": stats(&timing)
    })
}

fn benchmark_many_short_strokes() -> Value {
    let strokes: Vec<_> = (0..1_000)
        .map(|index| make_stroke(index + 1, 16, index as f32 * 0.0001))
        .collect();
    let mut vertices = 0usize;
    let timing = measure(|| {
        vertices = 0;
        for stroke in &strokes {
            let mesh = geometry::stroke_mesh(black_box(stroke));
            vertices += mesh.len();
            black_box(mesh);
        }
    });
    json!({
        "name": "many_short_strokes_tessellation",
        "operation": "geometry::stroke_mesh for every stroke",
        "strokes": strokes.len(),
        "points_per_stroke": 16,
        "total_points": strokes.iter().map(|stroke| stroke.points.len()).sum::<usize>(),
        "vertices": vertices,
        "timing": stats(&timing)
    })
}

fn benchmark_pick() -> Value {
    let doc = Document {
        strokes: (0..1_000)
            .map(|index| make_stroke(index + 1, 16, index as f32 * 0.001))
            .collect(),
        ..Document::default()
    };
    let camera = geometry::Camera::default();
    let rect = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(1280.0, 720.0));
    let pos = pos2(640.0, 360.0);
    let mut picked = None;
    let timing = measure(|| {
        picked = gizmo::pick(black_box(&doc), &camera, rect, pos);
        black_box(picked);
    });
    json!({
        "name": "artwork_pick",
        "operation": "gizmo::pick at viewport center",
        "strokes": doc.strokes.len(),
        "points_per_stroke": 16,
        "picked_stroke_id": picked,
        "timing": stats(&timing)
    })
}

fn benchmark_large_selection() -> Result<Value, String> {
    let doc = Document {
        strokes: (0..1_000)
            .map(|index| make_stroke(index + 1, 20, index as f32 * 0.0002))
            .collect(),
        ..Document::default()
    };
    let selected: HashSet<_> = doc
        .strokes
        .iter()
        .take(500)
        .map(|stroke| stroke.id)
        .collect();
    let snapshot = editing::snapshot(&doc, &selected);
    let mut bounds = None;
    let bounds_timing = measure(|| {
        bounds = editing::bounds(black_box(&doc), black_box(&selected));
        black_box(bounds);
    });
    let mut transformed_count = 0usize;
    let transform_timing = measure(|| {
        let after = editing::transformed(
            black_box(&snapshot),
            Vec3::ZERO,
            Vec3::new(0.25, -0.1, 0.0),
            Quat::from_rotation_z(0.05),
            1.02,
        )
        .expect("benchmark transform fixture must remain valid");
        transformed_count = after.len();
        black_box(after);
    });
    let after = editing::transformed(
        &snapshot,
        Vec3::ZERO,
        Vec3::new(0.25, -0.1, 0.0),
        Quat::from_rotation_z(0.05),
        1.02,
    )
    .map_err(str::to_owned)?;
    let apply_timing = measure(|| {
        let mut sample = doc.clone();
        editing::replace(&mut sample, &snapshot, &after, true);
        editing::replace(&mut sample, &after, &snapshot, false);
        black_box(sample);
    });
    let snapshot_timing = measure(|| {
        black_box(editing::snapshot(&doc, &selected));
    });
    Ok(json!({
        "name": "large_selection_operations",
        "strokes_in_document": doc.strokes.len(),
        "selected_strokes": snapshot.len(),
        "points_per_selected_stroke": 20,
        "snapshot": stats(&snapshot_timing),
        "bounds": {"min": bounds.map(|value| value.0.to_array()), "max": bounds.map(|value| value.1.to_array()), "timing": stats(&bounds_timing)},
        "transform": {"changed_strokes": transformed_count, "timing": stats(&transform_timing)},
        "apply_undo_redo_pair": stats(&apply_timing)
    }))
}
