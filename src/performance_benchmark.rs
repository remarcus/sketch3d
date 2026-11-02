//! Repeatable CPU workload benchmarks. These do not estimate GPU time or pen latency.
use crate::{
    dab_placement,
    document::{Brush, Document, Point, Stroke},
    dynamics::DabSettings,
    editing, geometry, gizmo,
};
use eframe::egui::{pos2, Rect};
use glam::{Quat, Vec3};
use serde_json::{json, Value};
use std::{collections::HashSet, hint::black_box, time::Instant};

const WARMUPS: usize = 1;
const SAMPLES: usize = 7;

pub fn run() -> Result<(), String> {
    let mut cases = vec![
        benchmark_long_stroke(),
        benchmark_long_dab_stroke(),
        benchmark_many_short_strokes(),
        benchmark_pick(),
        benchmark_large_selection()?,
        benchmark_linked_groups()?,
    ];
    cases.extend(benchmark_dab_presets());

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
        point_thickness: None,
        modifiers: None,
        fill: None,
        group_id: None,
        layer_id: 1,
        color: [0.2, 0.65, 0.85],
        gradient: None,
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

fn benchmark_long_dab_stroke() -> Value {
    const POINTS: usize = 4_096;
    const LENGTH: f32 = 57.3;
    let mut stroke = make_stroke(71, POINTS, 0.0);
    stroke.radius = 0.02;
    stroke.points = (0..POINTS)
        .map(|index| {
            let t = index as f32 / (POINTS - 1) as f32;
            Point {
                position: Vec3::new(LENGTH * t, 0.0, 0.0),
                pressure: 0.25 + 0.75 * t,
                tilt: None,
                time_ms: 0,
            }
        })
        .collect();
    stroke.brush.dab_seed = dab_placement::seed_for_stroke(stroke.id);
    stroke.brush.dabs = DabSettings {
        enabled: true,
        spacing: 0.35,
        ..Default::default()
    };

    let expected_dabs = dab_placement::point_count(&stroke)
        .expect("benchmark dab fixture stays within the placement budget");
    let mut sampled_dabs = 0usize;
    let sampler_timing = measure(|| {
        let dabs = dab_placement::sample(black_box(&stroke))
            .expect("benchmark dab fixture has valid settings");
        sampled_dabs = dabs.len();
        black_box(dabs);
    });
    let preview_vertices = geometry::stroke_mesh(&stroke).len();
    let preview_timing = measure(|| {
        let mesh = geometry::stroke_mesh(black_box(&stroke));
        black_box(mesh);
    });
    json!({
        "name": "long_dab_active_preview",
        "fixture": "4,096 evenly spaced world-space points over a 57.3-unit path; radius 0.02; spacing 0.35 radii",
        "points": stroke.points.len(),
        "world_length": LENGTH,
        "resolved_dabs": sampled_dabs,
        "expected_resolved_dabs": expected_dabs,
        "mesh_vertices": preview_vertices,
        "sampler": {
            "operation": "dab_placement::sample",
            "timing": stats(&sampler_timing)
        },
        "active_preview_geometry_rebuild": {
            "operation": "geometry::stroke_mesh for the current active stroke",
            "timing": stats(&preview_timing)
        }
    })
}

fn benchmark_dab_presets() -> Vec<Value> {
    const POINTS: usize = 4_096;
    let library = crate::presets::Library::default();
    [
        ("Stipple", 57.3, None),
        ("Spray", 57.3, None),
        ("Spray_dense_limit", 19.0, Some(0.1)),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, length, spacing_override))| {
        let preset_name = if name == "Spray_dense_limit" {
            "Spray"
        } else {
            name
        };
        let preset = library
            .presets
            .iter()
            .find(|preset| preset.name == preset_name)
            .expect("built-in dab benchmark preset exists");
        let mut stroke = make_stroke(730 + index as u64, POINTS, 0.0);
        stroke.radius = preset.radius;
        stroke.brush = preset.brush;
        stroke.brush.dab_seed = dab_placement::seed_for_stroke(stroke.id);
        if let Some(spacing) = spacing_override {
            stroke.brush.dabs.spacing = spacing;
        }
        stroke.points = (0..POINTS)
            .map(|point| {
                let t = point as f32 / (POINTS - 1) as f32;
                Point {
                    position: Vec3::new(length * t, 0.0, 0.0),
                    pressure: 0.15 + 0.85 * t,
                    tilt: None,
                    time_ms: (1000.0 * t).round() as u32,
                }
            })
            .collect();
        let resolved = dab_placement::sample(&stroke)
            .expect("built-in dab benchmark settings resolve within the budget");
        let expected_dabs = dab_placement::point_count(&stroke)
            .expect("built-in dab benchmark point count stays within the budget");
        assert_eq!(resolved.len(), expected_dabs);
        let mut sampled_count = 0usize;
        let sampler_timing = measure(|| {
            let dabs = dab_placement::sample(black_box(&stroke))
                .expect("built-in dab benchmark settings remain valid");
            sampled_count = dabs.len();
            black_box(dabs);
        });
        let mesh = geometry::stroke_mesh(&stroke);
        let mesh_timing = measure(|| {
            black_box(geometry::stroke_mesh(black_box(&stroke)));
        });
        let document = Document {
            linked_stroke_groups: Default::default(),
            strokes: vec![stroke.clone()],
            ..Document::default()
        };
        let glb = crate::export::encode(&document, None)
            .expect("built-in dab benchmark GLB remains within export budgets");
        let export_timing = measure(|| {
            black_box(
                crate::export::encode(black_box(&document), None)
                    .expect("built-in dab benchmark GLB remains within export budgets"),
            );
        });
        json!({
            "name": format!("{}_preset_placement_mesh_export", name.to_lowercase()),
            "fixture": "4,096 evenly spaced points over a planar path with interpolated pressure",
            "points": POINTS,
            "world_length": length,
            "radius": stroke.radius,
            "spacing_in_radii": stroke.brush.dabs.spacing,
            "scatter_in_radii": stroke.brush.dabs.scatter,
            "resolved_dabs": sampled_count,
            "mesh_vertices": mesh.len(),
            "glb_bytes": glb.bytes.len(),
            "sampler": stats(&sampler_timing),
            "mesh_rebuild": stats(&mesh_timing),
            "glb_encode": stats(&export_timing),
        })
    })
    .collect()
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
        linked_stroke_groups: Default::default(),
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
        linked_stroke_groups: Default::default(),
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

fn benchmark_linked_groups() -> Result<Value, String> {
    const GROUP_STROKES: usize = 12;
    const POINTS_PER_STROKE: usize = 80;
    const COPIES: usize = 20;
    let source_strokes: Vec<_> = (0..GROUP_STROKES)
        .map(|index| {
            let mut stroke = make_stroke(index as u64 + 1, POINTS_PER_STROKE, index as f32 * 0.08);
            stroke.group_id = Some(500);
            stroke
        })
        .collect();
    let source_ids: HashSet<_> = source_strokes.iter().map(|stroke| stroke.id).collect();
    let mut linked = Document {
        strokes: source_strokes.clone(),
        ..Document::default()
    };
    let first =
        editing::create_linked_group(&linked, &source_ids, 10_000, 1, Vec3::new(1.0, 0.5, 0.0))?;
    first.apply(&mut linked, true);
    let definition_id = linked.linked_stroke_groups.definitions[0].id;
    let mut next_stroke_id = 10_000 + GROUP_STROKES as u64;
    for index in 1..COPIES {
        let edit = editing::add_linked_instance(
            &linked,
            definition_id,
            next_stroke_id,
            1,
            Vec3::new(1.0, (index + 1) as f32 * 0.5, 0.0),
        )?;
        edit.apply(&mut linked, true);
        next_stroke_id += GROUP_STROKES as u64;
    }

    let mut repeated = Document {
        strokes: source_strokes.clone(),
        ..Document::default()
    };
    let source_snapshot = editing::snapshot(&repeated, &source_ids);
    let mut repeated_copy_ids = HashSet::new();
    let mut copy_id = 10_000_u64;
    for index in 0..COPIES {
        let mut copy = editing::transformed(
            &source_snapshot,
            Vec3::ZERO,
            Vec3::new(1.0, (index + 1) as f32 * 0.5, 0.0),
            Quat::IDENTITY,
            1.0,
        )?;
        for (_, stroke) in &mut copy {
            stroke.id = copy_id;
            copy_id += 1;
            stroke.group_id = None;
            repeated_copy_ids.insert(stroke.id);
        }
        repeated
            .strokes
            .extend(copy.into_iter().map(|(_, stroke)| stroke));
    }

    let linked_bytes = serde_json::to_vec(&linked)
        .map_err(|error| error.to_string())?
        .len();
    let repeated_bytes = serde_json::to_vec(&repeated)
        .map_err(|error| error.to_string())?
        .len();
    let repeated_selected: HashSet<_> = repeated_copy_ids
        .iter()
        .copied()
        .chain(source_ids.iter().copied())
        .collect();
    let repeated_all_snapshot = editing::snapshot(&repeated, &repeated_selected);
    let linked_after = editing::transformed(
        &editing::snapshot(&linked, &source_ids),
        Vec3::ZERO,
        Vec3::new(0.03, -0.02, 0.0),
        Quat::IDENTITY,
        1.0,
    )?;
    let linked_edit = editing::Edit::Change {
        before: editing::snapshot(&linked, &source_ids),
        after: linked_after,
    };
    let repeated_after = editing::transformed(
        &repeated_all_snapshot,
        Vec3::ZERO,
        Vec3::new(0.03, -0.02, 0.0),
        Quat::IDENTITY,
        1.0,
    )?;
    let repeated_edit = editing::Edit::Change {
        before: repeated_all_snapshot,
        after: repeated_after,
    };
    let linked_edit_timing = measure(|| {
        let mut sample = linked.clone();
        linked_edit.apply(&mut sample, true);
        linked_edit.apply(&mut sample, false);
        black_box(sample);
    });
    let repeated_edit_timing = measure(|| {
        let mut sample = repeated.clone();
        repeated_edit.apply(&mut sample, true);
        repeated_edit.apply(&mut sample, false);
        black_box(sample);
    });
    let linked_render_timing = measure(|| {
        let vertices: usize = linked
            .strokes
            .iter()
            .map(|stroke| geometry::stroke_mesh(black_box(stroke)).len())
            .sum();
        black_box(vertices);
    });
    let repeated_render_timing = measure(|| {
        let vertices: usize = repeated
            .strokes
            .iter()
            .map(|stroke| geometry::stroke_mesh(black_box(stroke)).len())
            .sum();
        black_box(vertices);
    });
    let estimate_heap = |document: &Document| {
        std::mem::size_of::<Document>()
            + document.strokes.capacity() * std::mem::size_of::<Stroke>()
            + document
                .strokes
                .iter()
                .map(|stroke| {
                    stroke.points.capacity() * std::mem::size_of::<Point>()
                        + stroke.curve.as_ref().map_or(0, |curve| {
                            curve.anchors.capacity()
                                * std::mem::size_of::<crate::document::CurveAnchor>()
                        })
                })
                .sum::<usize>()
            + document.linked_stroke_groups.definitions.capacity()
                * std::mem::size_of::<crate::document::LinkedStrokeGroupDefinition>()
            + document
                .linked_stroke_groups
                .definitions
                .iter()
                .map(|definition| {
                    definition.source_stroke_ids.capacity() * std::mem::size_of::<u64>()
                })
                .sum::<usize>()
            + document.linked_stroke_groups.instances.capacity()
                * std::mem::size_of::<crate::document::LinkedStrokeGroupInstance>()
            + document
                .linked_stroke_groups
                .instances
                .iter()
                .map(|instance| instance.stroke_ids.capacity() * std::mem::size_of::<u64>())
                .sum::<usize>()
    };
    Ok(json!({
        "name": "linked_group_vs_independent_repeated_copies",
        "fixture": {
            "source_strokes": GROUP_STROKES,
            "points_per_stroke": POINTS_PER_STROKE,
            "instances": COPIES,
            "total_expanded_strokes": linked.strokes.len(),
            "total_control_points": linked.strokes.iter().map(|stroke| stroke.points.len()).sum::<usize>()
        },
        "method": "Same deterministic planar architectural-detail strokes and baked geometry in both documents; synchronous CPU source edit plus undo pair; CPU stroke mesh generation; serialized native document byte count as a portable storage proxy.",
        "linked": {
            "native_json_bytes": linked_bytes,
            "estimated_heap_bytes": estimate_heap(&linked),
            "edit_and_undo": stats(&linked_edit_timing),
            "cpu_mesh_generation": stats(&linked_render_timing)
        },
        "independent_repeated_copies": {
            "native_json_bytes": repeated_bytes,
            "estimated_heap_bytes": estimate_heap(&repeated),
            "edit_and_undo": stats(&repeated_edit_timing),
            "cpu_mesh_generation": stats(&repeated_render_timing)
        },
        "limitations": [
            "Serialized byte count is a storage proxy, not allocator-measured resident heap size.",
            "Mesh timings exclude GPU upload, draw calls, and frame time.",
            "The linked implementation intentionally stores baked strokes for selection, recovery, and portable export, so it does not reduce expanded geometry memory."
        ]
    }))
}
