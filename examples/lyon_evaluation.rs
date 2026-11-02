//! Bounded, isolated Lyon evaluation harness. This is not used by production geometry.
use glam::{Vec2, Vec3};
use image::{Rgb, RgbImage};
use lyon_tessellation::{
    geometry_builder::{simple_builder, VertexBuffers},
    math::point,
    path::Path,
    FillOptions, FillRule, FillTessellator, LineJoin, StrokeOptions, StrokeTessellator,
};
use serde::Serialize;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs,
    path::{Path as FsPath, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

const MAX_POINTS: usize = 256;
const MAX_VERTICES: usize = 65_535;
const MAX_INDICES: usize = 196_605;
const TOLERANCE: f32 = 0.1;

// The example runs cases serially. Counters are enabled only around one tessellation call.
struct CountingAllocator;
static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicUsize = AtomicUsize::new(0);
static REQUESTED_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() && TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() && TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        new_ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[derive(Clone, Debug)]
struct Mesh {
    positions: Vec<Vec2>,
    indices: Vec<u32>,
}
#[derive(Serialize)]
struct CaseReport {
    name: String,
    vertices: usize,
    triangles: usize,
    median_us: f64,
    allocation_calls_per_invocation: usize,
    requested_bytes_per_invocation: usize,
    baseline_triangles: Option<usize>,
    baseline_median_us: Option<f64>,
    baseline_allocation_calls: Option<usize>,
    baseline_requested_bytes: Option<usize>,
    error_pixels: Vec<usize>,
    alternate_fill_rule_error_pixels: Vec<usize>,
}
#[derive(Serialize)]
struct Report {
    crate_version: &'static str,
    build_profile: &'static str,
    timing_repetitions: usize,
    allocation_repetitions: usize,
    allocation_method: &'static str,
    limits: Limits,
    fill_cases: Vec<CaseReport>,
    stroke_cases: Vec<CaseReport>,
    plane_max_error: f32,
    import_obj: String,
    scale_pixels: [u32; 3],
}
#[derive(Serialize)]
struct Limits {
    max_input_points: usize,
    max_output_vertices: usize,
    max_output_indices: usize,
    tolerance: f32,
}

fn bounded(points: &[Vec2]) -> Result<(), String> {
    if points.len() < 3 || points.len() > MAX_POINTS {
        return Err(format!(
            "point count {} outside 3..={MAX_POINTS}",
            points.len()
        ));
    }
    if points.iter().any(|p| !p.is_finite()) {
        return Err("non-finite point".into());
    }
    Ok(())
}
fn path(points: &[Vec2], close: bool) -> Result<Path, String> {
    bounded(points)?;
    let mut b = Path::builder();
    b.begin(point(points[0].x, points[0].y));
    for p in &points[1..] {
        b.line_to(point(p.x, p.y));
    }
    b.end(close);
    Ok(b.build())
}
fn lyon_fill(points: &[Vec2], rule: FillRule) -> Result<Mesh, String> {
    let p = path(points, true)?;
    let mut buffers: VertexBuffers<lyon_tessellation::math::Point, u16> = VertexBuffers::new();
    let opts = FillOptions::default()
        .with_tolerance(TOLERANCE)
        .with_fill_rule(rule);
    FillTessellator::new()
        .tessellate_path(&p, &opts, &mut simple_builder(&mut buffers))
        .map_err(|e| format!("fill tessellation: {e:?}"))?;
    if buffers.vertices.len() > MAX_VERTICES || buffers.indices.len() > MAX_INDICES {
        return Err("tessellation output cap exceeded".into());
    }
    Ok(Mesh {
        positions: buffers
            .vertices
            .iter()
            .map(|p| Vec2::new(p.x, p.y))
            .collect(),
        indices: buffers.indices.into_iter().map(u32::from).collect(),
    })
}
fn lyon_stroke(points: &[Vec2], widths: Option<&[f32]>, join: LineJoin) -> Result<Mesh, String> {
    bounded(points)?;
    if let Some(w) = widths {
        if w.len() != points.len() || w.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("invalid width factors".into());
        }
    }
    let mut b = Path::builder_with_attributes(if widths.is_some() { 1 } else { 0 });
    let attrs = |i: usize| widths.map_or(&[][..], |w| std::slice::from_ref(&w[i]));
    b.begin(point(points[0].x, points[0].y), attrs(0));
    for (i, p) in points.iter().enumerate().skip(1) {
        b.line_to(point(p.x, p.y), attrs(i));
    }
    b.end(false);
    let p = b.build();
    let mut buffers: VertexBuffers<lyon_tessellation::math::Point, u16> = VertexBuffers::new();
    let mut opts = StrokeOptions::default()
        .with_tolerance(TOLERANCE)
        .with_line_join(join)
        .with_line_width(1.0);
    if widths.is_some() {
        opts = opts.with_variable_line_width(0);
    }
    StrokeTessellator::new()
        .tessellate_path(&p, &opts, &mut simple_builder(&mut buffers))
        .map_err(|e| format!("stroke tessellation: {e:?}"))?;
    if buffers.vertices.len() > MAX_VERTICES || buffers.indices.len() > MAX_INDICES {
        return Err("tessellation output cap exceeded".into());
    }
    Ok(Mesh {
        positions: buffers
            .vertices
            .iter()
            .map(|p| Vec2::new(p.x, p.y))
            .collect(),
        indices: buffers.indices.into_iter().map(u32::from).collect(),
    })
}

// Reference implementation mirroring the current simple-contour ear-clip behavior.
fn baseline_ear_clip(points: &[Vec2]) -> Result<Mesh, String> {
    bounded(points)?;
    let n = points.len();
    let mut area = 0.0;
    for i in 0..n {
        area += points[i].perp_dot(points[(i + 1) % n]);
    }
    if area.abs() < 1e-6 {
        return Err("degenerate area".into());
    }
    let orient = area.signum();
    let mut ring = (0..n).collect::<Vec<_>>();
    let mut ids = Vec::new();
    let mut guard = 0;
    while ring.len() > 3 && guard < n * n {
        guard += 1;
        let mut found = false;
        for j in 0..ring.len() {
            let a = ring[(j + ring.len() - 1) % ring.len()];
            let b = ring[j];
            let c = ring[(j + 1) % ring.len()];
            if orient * (points[b] - points[a]).perp_dot(points[c] - points[b]) <= 1e-6 {
                continue;
            }
            let tri = [points[a], points[b], points[c]];
            if ring
                .iter()
                .copied()
                .filter(|&k| k != a && k != b && k != c)
                .any(|k| in_triangle(points[k], tri))
            {
                continue;
            }
            ids.extend([a as u32, b as u32, c as u32]);
            ring.remove(j);
            found = true;
            break;
        }
        if !found {
            return Err("unsupported self-intersection or non-simple contour".into());
        }
    }
    if ring.len() == 3 {
        ids.extend(ring.iter().map(|&i| i as u32));
    }
    Ok(Mesh {
        positions: points.to_vec(),
        indices: ids,
    })
}
fn in_triangle(p: Vec2, t: [Vec2; 3]) -> bool {
    let d1 = (p - t[0]).perp_dot(t[1] - t[0]);
    let d2 = (p - t[1]).perp_dot(t[2] - t[1]);
    let d3 = (p - t[2]).perp_dot(t[0] - t[2]);
    (d1 >= -1e-6 && d2 >= -1e-6 && d3 >= -1e-6) || (d1 <= 1e-6 && d2 <= 1e-6 && d3 <= 1e-6)
}
fn measure<F: FnMut() -> Result<Mesh, String>>(
    mut f: F,
) -> Result<(Mesh, f64, usize, usize), String> {
    let mut ts = Vec::new();
    let mut last = None;
    for _ in 0..25 {
        let now = Instant::now();
        last = Some(f()?);
        ts.push(now.elapsed().as_secs_f64() * 1e6);
    }
    ts.sort_by(|a, b| a.total_cmp(b));
    let mut calls = Vec::new();
    let mut bytes = Vec::new();
    for _ in 0..25 {
        ALLOCATION_CALLS.store(0, Ordering::SeqCst);
        REQUESTED_BYTES.store(0, Ordering::SeqCst);
        TRACK_ALLOCATIONS.store(true, Ordering::SeqCst);
        let result = f();
        TRACK_ALLOCATIONS.store(false, Ordering::SeqCst);
        result?;
        calls.push(ALLOCATION_CALLS.load(Ordering::SeqCst));
        bytes.push(REQUESTED_BYTES.load(Ordering::SeqCst));
    }
    calls.sort_unstable();
    bytes.sort_unstable();
    Ok((last.unwrap(), ts[12], calls[12], bytes[12]))
}
fn raster(mesh: &Mesh, size: u32) -> RgbImage {
    let mut img = RgbImage::from_pixel(size, size, Rgb([255, 255, 255]));
    let (min, max) = bounds(&mesh.positions);
    let span = (max - min).max(Vec2::splat(1e-6));
    let pad = size as f32 * 0.08;
    let scale = (size as f32 - 2.0 * pad) / span.max_element();
    let map = |p: Vec2| {
        Vec2::new(
            pad + (p.x - min.x) * scale,
            size as f32 - pad - (p.y - min.y) * scale,
        )
    };
    for tri in mesh.indices.chunks_exact(3) {
        let a = map(mesh.positions[tri[0] as usize]);
        let b = map(mesh.positions[tri[1] as usize]);
        let c = map(mesh.positions[tri[2] as usize]);
        let lo = a.min(b).min(c).floor().max(Vec2::ZERO);
        let hi = a.max(b).max(c).ceil().min(Vec2::splat(size as f32 - 1.0));
        for y in lo.y as u32..=hi.y as u32 {
            for x in lo.x as u32..=hi.x as u32 {
                let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if in_triangle(q, [a, b, c]) {
                    img.put_pixel(x, y, Rgb([55, 105, 185]));
                }
            }
        }
    }
    img
}
fn bounds(p: &[Vec2]) -> (Vec2, Vec2) {
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for v in p {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    (lo, hi)
}
fn error_pixels(a: &RgbImage, b: &RgbImage) -> usize {
    a.pixels()
        .zip(b.pixels())
        .filter(|(x, y)| x.0 != y.0)
        .count()
}
fn write_obj(path: &FsPath, vertices: &[Vec3], indices: &[u32]) -> Result<(), String> {
    let mut s = String::new();
    for p in vertices {
        s.push_str(&format!("v {:.7} {:.7} {:.7}\n", p.x, p.y, p.z));
    }
    for t in indices.chunks_exact(3) {
        s.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
    }
    fs::write(path, s).map_err(|e| e.to_string())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("verification/lyon-evaluation"));
    fs::create_dir_all(&out)?;
    let fixtures: Vec<(&str, Vec<Vec2>)> = vec![
        (
            "convex",
            vec![
                Vec2::new(-1., -1.),
                Vec2::new(1., -1.),
                Vec2::new(1., 1.),
                Vec2::new(-1., 1.),
            ],
        ),
        (
            "concave",
            vec![
                Vec2::new(-1., -1.),
                Vec2::new(1., -1.),
                Vec2::new(1., 0.),
                Vec2::new(0., 0.),
                Vec2::new(0., 1.),
                Vec2::new(-1., 1.),
            ],
        ),
        (
            "bow_tie",
            vec![
                Vec2::new(-1., -1.),
                Vec2::new(1., 1.),
                Vec2::new(-1., 1.),
                Vec2::new(1., -1.),
            ],
        ),
        (
            "double_wound",
            vec![
                Vec2::new(-1., -1.),
                Vec2::new(1., -1.),
                Vec2::new(1., 1.),
                Vec2::new(-1., 1.),
                Vec2::new(-1., -1.),
                Vec2::new(1., -1.),
                Vec2::new(1., 1.),
                Vec2::new(-1., 1.),
            ],
        ),
    ];
    let scales = [128, 512, 2048];
    let mut fills = Vec::new();
    let mut import_mesh = None;
    for (name, pts) in &fixtures {
        for (rule_name, rule) in [
            ("nonzero", FillRule::NonZero),
            ("evenodd", FillRule::EvenOdd),
        ] {
            let (mesh, us, allocs, alloc_bytes) = measure(|| lyon_fill(pts, rule))?;
            let (other_rule_mesh, _, _, _) = measure(|| {
                lyon_fill(
                    pts,
                    if rule == FillRule::EvenOdd {
                        FillRule::NonZero
                    } else {
                        FillRule::EvenOdd
                    },
                )
            })?;
            let (base, baseline_us, baseline_allocs, baseline_bytes) =
                match measure(|| baseline_ear_clip(pts)) {
                    Ok((mesh, median, allocs, bytes)) => {
                        (Some(mesh), Some(median), Some(allocs), Some(bytes))
                    }
                    Err(_) => (None, None, None, None),
                };
            let mut diffs = Vec::new();
            let mut rule_diffs = Vec::new();
            for size in scales {
                let candidate = raster(&mesh, size);
                candidate.save(out.join(format!("{name}-{rule_name}-{size}.png")))?;
                rule_diffs.push(error_pixels(&candidate, &raster(&other_rule_mesh, size)));
                if let Some(ref b) = base {
                    diffs.push(error_pixels(&candidate, &raster(b, size)));
                }
            }
            if *name == "concave" && rule == FillRule::NonZero {
                import_mesh = Some(mesh.clone());
            }
            let bt = base.as_ref().map(|b| b.indices.len() / 3);
            fills.push(CaseReport {
                name: format!("{name}/{rule_name}"),
                vertices: mesh.positions.len(),
                triangles: mesh.indices.len() / 3,
                median_us: us,
                allocation_calls_per_invocation: allocs,
                requested_bytes_per_invocation: alloc_bytes,
                baseline_triangles: bt,
                baseline_median_us: baseline_us,
                baseline_allocation_calls: baseline_allocs,
                baseline_requested_bytes: baseline_bytes,
                error_pixels: diffs,
                alternate_fill_rule_error_pixels: rule_diffs,
            });
        }
    }
    let sharp = vec![
        Vec2::new(-1., -0.7),
        Vec2::new(0., 0.7),
        Vec2::new(0.12, -0.7),
    ];
    let variable = vec![Vec2::new(-1., 0.), Vec2::new(0., 0.55), Vec2::new(1., 0.)];
    let stroke_inputs = [
        ("sharp_miter", &sharp, None, LineJoin::Miter),
        ("sharp_bevel", &sharp, None, LineJoin::Bevel),
        (
            "variable_width",
            &variable,
            Some(&[0.25, 1.8, 0.55][..]),
            LineJoin::Round,
        ),
    ];
    let mut strokes = Vec::new();
    for (name, pts, widths, join) in stroke_inputs {
        let (m, us, allocs, alloc_bytes) = measure(|| lyon_stroke(pts, widths, join))?;
        for size in scales {
            raster(&m, size).save(out.join(format!("stroke-{name}-{size}.png")))?;
        }
        strokes.push(CaseReport {
            name: name.into(),
            vertices: m.positions.len(),
            triangles: m.indices.len() / 3,
            median_us: us,
            allocation_calls_per_invocation: allocs,
            requested_bytes_per_invocation: alloc_bytes,
            baseline_triangles: None,
            baseline_median_us: None,
            baseline_allocation_calls: None,
            baseline_requested_bytes: None,
            error_pixels: vec![],
            alternate_fill_rule_error_pixels: vec![],
        });
    }
    let normal = Vec3::new(0.31, -0.48, 0.82).normalize();
    let u = (Vec3::X - normal * Vec3::X.dot(normal)).normalize();
    let v = normal.cross(u).normalize();
    let origin = Vec3::new(2.4, -1.7, 3.1);
    let mesh = import_mesh.ok_or("missing import mesh")?;
    let mut plane_err = 0.0f32;
    let mut world_vertices = Vec::with_capacity(mesh.positions.len());
    for p in &mesh.positions {
        let world = origin + u * p.x + v * p.y;
        plane_err = plane_err.max((world - origin).dot(normal).abs());
        world_vertices.push(world);
    }
    write_obj(
        &out.join("rotated_concave.obj"),
        &world_vertices,
        &mesh.indices,
    )?;
    let report = Report {
        crate_version: "1.0.23",
        build_profile: "debug",
        timing_repetitions: 25,
        allocation_repetitions: 25,
        allocation_method: "median global alloc/realloc calls and cumulative requested sizes, scoped to a single-threaded invocation; includes adapter output conversion and excludes frees",
        limits: Limits {
            max_input_points: MAX_POINTS,
            max_output_vertices: MAX_VERTICES,
            max_output_indices: MAX_INDICES,
            tolerance: TOLERANCE,
        },
        fill_cases: fills,
        stroke_cases: strokes,
        plane_max_error: plane_err,
        import_obj: "rotated_concave.obj".into(),
        scale_pixels: scales,
    };
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "Wrote Lyon evaluation fixtures and report to {}",
        out.display()
    );
    Ok(())
}
