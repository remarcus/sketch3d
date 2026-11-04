//! Bounded import of editable, solid-color SVG artwork onto a planar guide.

use crate::curve_geometry::{CubicBezier, PressureSegment};
use crate::document::{Brush, CurveAnchor, CurveData, PlanarFill, Point, Stroke};
use glam::{Vec2, Vec3};
use quick_xml::{events::Event, Reader};

pub const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SVG_ELEMENTS: usize = 512;
pub const MAX_SVG_SEGMENTS: usize = 4096;
pub const MAX_IMPORTED_POINTS: usize = 262_144;

#[derive(Clone, Debug)]
pub struct SvgPath {
    pub segments: Vec<CubicBezier>,
    pub closed: bool,
    pub fill: Option<([f32; 3], f32)>,
    pub stroke: Option<([f32; 3], f32, f32)>,
}

#[derive(Clone, Debug)]
pub struct SvgArtwork {
    pub view_box: [f32; 4],
    pub paths: Vec<SvgPath>,
    pub skipped: usize,
}

#[derive(Clone, Copy, Debug)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };
    fn apply(self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }
    fn then(self, o: Self) -> Self {
        Self {
            a: o.a * self.a + o.c * self.b,
            b: o.b * self.a + o.d * self.b,
            c: o.a * self.c + o.c * self.d,
            d: o.b * self.c + o.d * self.d,
            e: o.a * self.e + o.c * self.f + o.e,
            f: o.b * self.e + o.d * self.f + o.f,
        }
    }
}

pub fn parse(bytes: &[u8]) -> Result<SvgArtwork, String> {
    if bytes.is_empty() || bytes.len() > MAX_SVG_BYTES {
        return Err("SVG file is empty or exceeds the 4 MiB import limit".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "SVG must be UTF-8")?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut view_box = None;
    let mut paths = Vec::new();
    let mut skipped = 0;
    let mut elements = 0;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("Invalid SVG XML: {e}"))?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                elements += 1;
                if elements > MAX_SVG_ELEMENTS {
                    return Err("SVG exceeds the 512 element import limit".into());
                }
                let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                let mut attrs = std::collections::HashMap::new();
                for attr in e.attributes().with_checks(false) {
                    let attr = attr.map_err(|e| format!("Invalid SVG attribute: {e}"))?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_ascii_lowercase();
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|e| format!("Invalid SVG attribute value: {e}"))?
                        .into_owned();
                    attrs.insert(key, value);
                }
                if name == "svg" {
                    view_box = attrs
                        .get("viewbox")
                        .and_then(|v| numbers(v))
                        .filter(|n| n.len() == 4 && n[2] > 0.0 && n[3] > 0.0)
                        .map(|n| [n[0], n[1], n[2], n[3]]);
                } else if name == "path" || name == "polygon" || name == "polyline" {
                    let transform = attrs
                        .get("transform")
                        .map(|v| parse_transform(v))
                        .unwrap_or(Some(Affine::IDENTITY));
                    let geometry = if let Some(transform) = transform {
                        if name == "path" {
                            attrs.get("d").and_then(|d| parse_path(d).ok())
                        } else {
                            attrs
                                .get("points")
                                .and_then(|p| parse_poly(p, name == "polygon").ok())
                        }
                        .map(|(segments, closed)| {
                            (
                                segments
                                    .into_iter()
                                    .map(|s| CubicBezier {
                                        p0: transform.apply(s.p0.truncate()).extend(0.0),
                                        p1: transform.apply(s.p1.truncate()).extend(0.0),
                                        p2: transform.apply(s.p2.truncate()).extend(0.0),
                                        p3: transform.apply(s.p3.truncate()).extend(0.0),
                                    })
                                    .collect(),
                                closed,
                            )
                        })
                    } else {
                        None
                    };
                    if let Some((segments, closed)) =
                        geometry.filter(|(s, _)| !s.is_empty() && s.len() <= MAX_SVG_SEGMENTS)
                    {
                        let fill =
                            parse_color(attrs.get("fill").map(String::as_str).unwrap_or("black"))
                                .map(|color| (color, opacity(&attrs, "fill-opacity")));
                        let stroke =
                            parse_color(attrs.get("stroke").map(String::as_str).unwrap_or("none"))
                                .map(|color| {
                                    (
                                        color,
                                        opacity(&attrs, "stroke-opacity"),
                                        attrs
                                            .get("stroke-width")
                                            .and_then(|x| x.parse().ok())
                                            .filter(|x: f32| x.is_finite() && *x > 0.0)
                                            .unwrap_or(1.0),
                                    )
                                });
                        if fill.is_none() && stroke.is_none() {
                            skipped += 1;
                        } else {
                            paths.push(SvgPath {
                                segments,
                                closed,
                                fill,
                                stroke,
                            });
                        }
                    } else {
                        skipped += 1;
                    }
                } else if !matches!(name.as_str(), "svg" | "g" | "title" | "desc" | "metadata") {
                    skipped += 1;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if paths.is_empty() {
        return Err("No supported solid-color SVG paths or polygons were found".into());
    }
    let bounds = view_box.unwrap_or_else(|| {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for path in &paths {
            for s in &path.segments {
                for p in [s.p0, s.p1, s.p2, s.p3] {
                    min = min.min(p.truncate());
                    max = max.max(p.truncate());
                }
            }
        }
        [
            min.x,
            min.y,
            (max.x - min.x).max(0.001),
            (max.y - min.y).max(0.001),
        ]
    });
    if !bounds.iter().all(|v| v.is_finite())
        || bounds[2] <= 0.0
        || bounds[3] <= 0.0
        || bounds.iter().any(|v| v.abs() > 1_000_000.0)
    {
        return Err("SVG viewBox must have finite, positive dimensions".into());
    }
    Ok(SvgArtwork {
        view_box: bounds,
        paths,
        skipped,
    })
}

fn opacity(attrs: &std::collections::HashMap<String, String>, key: &str) -> f32 {
    attrs
        .get(key)
        .and_then(|x| x.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}
fn parse_color(value: &str) -> Option<[f32; 3]> {
    let v = value.trim().to_ascii_lowercase();
    if v == "none" || v == "transparent" {
        return None;
    }
    let (r, g, b) = if let Some(hex) = v.strip_prefix('#') {
        if hex.len() == 3 {
            (
                u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?,
            )
        } else if hex.len() == 6 {
            (
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            )
        } else {
            return None;
        }
    } else {
        match v.as_str() {
            "black" => (0, 0, 0),
            "white" => (255, 255, 255),
            "red" => (255, 0, 0),
            "green" => (0, 128, 0),
            "blue" => (0, 0, 255),
            "yellow" => (255, 255, 0),
            "cyan" | "aqua" => (0, 255, 255),
            "magenta" | "fuchsia" => (255, 0, 255),
            "gray" | "grey" => (128, 128, 128),
            "orange" => (255, 165, 0),
            "purple" => (128, 0, 128),
            "navy" => (0, 0, 128),
            "teal" => (0, 128, 128),
            "lime" => (0, 255, 0),
            "maroon" => (128, 0, 0),
            _ => return None,
        }
    };
    Some([r, g, b].map(|c| {
        let s = c as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }))
}

fn numbers(s: &str) -> Option<Vec<f32>> {
    let mut out = Vec::new();
    let mut start = None;
    let b = s.as_bytes();
    for i in 0..=b.len() {
        let valid = i < b.len()
            && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'-' | b'+' | b'e' | b'E'));
        if valid {
            start.get_or_insert(i);
        } else if let Some(a) = start.take() {
            let n = s[a..i].parse::<f32>().ok()?;
            if !n.is_finite() {
                return None;
            }
            out.push(n);
        }
    }
    Some(out)
}

fn parse_transform(s: &str) -> Option<Affine> {
    let mut result = Affine::IDENTITY;
    let mut rest = s.trim();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let close = rest[open + 1..].find(')')? + open + 1;
        let name = rest[..open].trim();
        let ns = numbers(&rest[open + 1..close])?;
        let t = match name {
            "translate" if (1..=2).contains(&ns.len()) => Affine {
                e: ns[0],
                f: *ns.get(1).unwrap_or(&0.0),
                ..Affine::IDENTITY
            },
            "scale" if (1..=2).contains(&ns.len()) => Affine {
                a: ns[0],
                d: *ns.get(1).unwrap_or(&ns[0]),
                ..Affine::IDENTITY
            },
            "matrix" if ns.len() == 6 => Affine {
                a: ns[0],
                b: ns[1],
                c: ns[2],
                d: ns[3],
                e: ns[4],
                f: ns[5],
            },
            "rotate" if (1..=3).contains(&ns.len()) => {
                let a = ns[0].to_radians();
                let (sin, cos) = a.sin_cos();
                let rot = Affine {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    e: 0.0,
                    f: 0.0,
                };
                if ns.len() == 3 {
                    Affine::IDENTITY
                        .then(Affine {
                            e: ns[1],
                            f: ns[2],
                            ..Affine::IDENTITY
                        })
                        .then(rot)
                        .then(Affine {
                            e: -ns[1],
                            f: -ns[2],
                            ..Affine::IDENTITY
                        })
                } else {
                    rot
                }
            }
            _ => return None,
        };
        if [t.a, t.b, t.c, t.d, t.e, t.f]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
        {
            return None;
        }
        result = result.then(t);
        rest = rest[close + 1..].trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }
    Some(result)
}

fn parse_path(s: &str) -> Result<(Vec<CubicBezier>, bool), String> {
    let mut tokens = Vec::<String>::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() || b[i] == b',' {
            i += 1;
            continue;
        }
        if b[i].is_ascii_alphabetic() {
            tokens.push((b[i] as char).to_string());
            i += 1;
            continue;
        }
        let start = i;
        if matches!(b[i], b'+' | b'-') {
            i += 1;
        }
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
            i += 1;
        }
        if i < b.len() && matches!(b[i], b'e' | b'E') {
            i += 1;
            if i < b.len() && matches!(b[i], b'+' | b'-') {
                i += 1;
            }
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i == start {
            return Err("Unsupported SVG path syntax".into());
        }
        tokens.push(s[start..i].into());
    }
    let mut seg = Vec::new();
    let mut cursor = Vec2::ZERO;
    let mut start = Vec2::ZERO;
    let mut cmd = ' ';
    let mut index = 0;
    let mut closed = false;
    while index < tokens.len() {
        if tokens[index].len() == 1 && tokens[index].as_bytes()[0].is_ascii_alphabetic() {
            cmd = tokens[index].chars().next().unwrap();
            index += 1;
            if cmd == 'Z' || cmd == 'z' {
                seg.push(line(cursor, start));
                cursor = start;
                closed = true;
                cmd = ' ';
                continue;
            }
        }
        if cmd == ' ' {
            return Err("SVG path needs an explicit command".into());
        }
        let rel = cmd.is_ascii_lowercase();
        let upper = cmd.to_ascii_uppercase();
        let point = |x: f32, y: f32, from: Vec2, rel: bool| {
            if rel {
                from + Vec2::new(x, y)
            } else {
                Vec2::new(x, y)
            }
        };
        let read = |idx: &mut usize| -> Result<f32, String> {
            let v = tokens
                .get(*idx)
                .ok_or("Incomplete SVG path command")?
                .parse::<f32>()
                .map_err(|_| "Invalid SVG path number")?;
            *idx += 1;
            if !v.is_finite() || v.abs() > 1_000_000.0 {
                return Err("SVG path coordinate exceeds the import limit".into());
            }
            Ok(v)
        };
        match upper {
            'M' | 'L' => {
                let p = point(read(&mut index)?, read(&mut index)?, cursor, rel);
                if upper == 'M' {
                    if !seg.is_empty() {
                        return Err(
                            "Multiple SVG subpaths in one path are not supported yet".into()
                        );
                    }
                    start = p;
                    cmd = if rel { 'l' } else { 'L' };
                } else {
                    seg.push(line(cursor, p));
                }
                cursor = p;
            }
            'H' => {
                let x = read(&mut index)?;
                let p = Vec2::new(if rel { cursor.x + x } else { x }, cursor.y);
                seg.push(line(cursor, p));
                cursor = p;
            }
            'V' => {
                let y = read(&mut index)?;
                let p = Vec2::new(cursor.x, if rel { cursor.y + y } else { y });
                seg.push(line(cursor, p));
                cursor = p;
            }
            'C' => {
                let p1 = point(read(&mut index)?, read(&mut index)?, cursor, rel);
                let p2 = point(read(&mut index)?, read(&mut index)?, cursor, rel);
                let p3 = point(read(&mut index)?, read(&mut index)?, cursor, rel);
                seg.push(CubicBezier {
                    p0: cursor.extend(0.0),
                    p1: p1.extend(0.0),
                    p2: p2.extend(0.0),
                    p3: p3.extend(0.0),
                });
                cursor = p3;
            }
            'Q' => {
                let q = point(read(&mut index)?, read(&mut index)?, cursor, rel);
                let p = point(read(&mut index)?, read(&mut index)?, cursor, rel);
                let p1 = cursor.lerp(q, 2.0 / 3.0);
                let p2 = p.lerp(q, 2.0 / 3.0);
                seg.push(CubicBezier {
                    p0: cursor.extend(0.0),
                    p1: p1.extend(0.0),
                    p2: p2.extend(0.0),
                    p3: p.extend(0.0),
                });
                cursor = p;
            }
            _ => return Err(format!("Unsupported SVG path command {cmd}")),
        }
        if seg.len() > MAX_SVG_SEGMENTS {
            return Err("SVG path exceeds the segment limit".into());
        }
    }
    if seg.is_empty() {
        return Err("SVG path is empty".into());
    }
    Ok((seg, closed))
}
fn line(a: Vec2, b: Vec2) -> CubicBezier {
    CubicBezier {
        p0: a.extend(0.0),
        p1: a.lerp(b, 1.0 / 3.0).extend(0.0),
        p2: a.lerp(b, 2.0 / 3.0).extend(0.0),
        p3: b.extend(0.0),
    }
}
fn parse_poly(s: &str, closed: bool) -> Result<(Vec<CubicBezier>, bool), String> {
    let n = numbers(s).ok_or("Invalid SVG points")?;
    if n.len() < 4 || n.len() % 2 != 0 || n.len() / 2 > MAX_SVG_SEGMENTS {
        return Err("SVG polygon has invalid or excessive points".into());
    }
    let pts = n
        .chunks_exact(2)
        .map(|p| Vec2::new(p[0], p[1]))
        .collect::<Vec<_>>();
    let mut seg = pts.windows(2).map(|p| line(p[0], p[1])).collect::<Vec<_>>();
    if closed {
        seg.push(line(*pts.last().unwrap(), pts[0]));
    }
    Ok((seg, closed))
}

/// Convert parsed paths into strokes on the supplied planar frame.
pub fn to_strokes(
    art: &SvgArtwork,
    plane: crate::geometry::DrawingPlane,
    layer_id: u64,
    first_id: u64,
) -> Result<Vec<Stroke>, String> {
    let (u, v, n) = plane.basis();
    let size = art.view_box[2..].iter().copied().fold(0.0_f32, f32::max);
    let scale = 2.0 / size;
    let origin = plane.origin();
    let world = |p: Vec3| {
        let x = (p.x - art.view_box[0] - art.view_box[2] * 0.5) * scale;
        let y = -(p.y - art.view_box[1] - art.view_box[3] * 0.5) * scale;
        origin + u * x + v * y
    };
    let mut result = Vec::new();
    let mut id = first_id;
    let mut total_points = 0_usize;
    for path in &art.paths {
        let mut segments = path.segments.clone();
        if path.fill.is_some() && !path.closed {
            if let (Some(first), Some(last)) = (segments.first().copied(), segments.last().copied())
            {
                if first.p0.distance_squared(last.p3) > 1e-10 {
                    segments.push(CubicBezier {
                        p0: last.p3,
                        p1: last.p3.lerp(first.p0, 1.0 / 3.0),
                        p2: last.p3.lerp(first.p0, 2.0 / 3.0),
                        p3: first.p0,
                    });
                }
            }
        }
        for s in &mut segments {
            s.p0 = world(s.p0);
            s.p1 = world(s.p1);
            s.p2 = world(s.p2);
            s.p3 = world(s.p3);
        }
        let make = |color: [f32; 3],
                    opacity: f32,
                    radius: f32,
                    fill: bool,
                    id: u64|
         -> Result<Stroke, String> {
            let pressure_segments = segments
                .iter()
                .map(|s| PressureSegment {
                    curve: *s,
                    pressure_start: 1.0,
                    pressure_end: 1.0,
                })
                .collect::<Vec<_>>();
            let prototype = Stroke {
                model_surface: None,
                fill: None,
                material: crate::document::StrokeMaterial::Unlit,
                group_id: None,
                layer_id,
                id,
                color,
                gradient: None,
                radius: radius.clamp(0.001, 1.0),
                brush: Brush {
                    normal: n,
                    tangent: u,
                    opacity,
                    ..Brush::default()
                },
                points: Vec::new(),
                point_thickness: None,
                point_colors: None,
                modifiers: None,
                curve: None,
            };
            let mut stroke = crate::curve_geometry::tessellate_stroke(
                &pressure_segments,
                &prototype,
                0.002,
                crate::curve_geometry::MAX_CURVE_POINTS,
            )?;
            let mut anchors = segments
                .iter()
                .map(|s| CurveAnchor {
                    position: s.p0,
                    pressure: 1.0,
                    incoming: Vec3::ZERO,
                    outgoing: Vec3::ZERO,
                    manual: true,
                })
                .collect::<Vec<_>>();
            anchors.push(CurveAnchor {
                position: segments.last().unwrap().p3,
                pressure: 1.0,
                incoming: Vec3::ZERO,
                outgoing: Vec3::ZERO,
                manual: true,
            });
            for (i, s) in segments.iter().enumerate() {
                anchors[i].outgoing = s.p1 - s.p0;
                anchors[i + 1].incoming = s.p2 - s.p3;
            }
            let frame_origin = origin;
            let frame_normal = n;
            let frame_tangent = u;
            stroke.curve = Some(CurveData {
                anchors,
                frame_origin,
                frame_normal,
                frame_tangent,
                half_extent: None,
                tension: 0.0,
                surface_target: None,
                shape: None,
            });
            if fill {
                let mut contour = stroke.points.iter().map(|p| p.position).collect::<Vec<_>>();
                if contour.len() > 2
                    && contour[0].distance_squared(*contour.last().unwrap()) <= 1e-10
                {
                    contour.pop();
                }
                if contour.len() < 3 {
                    return Err("Filled SVG path has too few points".into());
                }
                stroke.fill = Some(PlanarFill::default());
                stroke.points = contour
                    .iter()
                    .copied()
                    .map(|position| Point {
                        position,
                        pressure: 1.0,
                        tilt: None,
                        time_ms: 0,
                    })
                    .collect();
                stroke.curve = None;
            }
            Ok(stroke)
        };
        if let Some((color, opacity)) = path.fill {
            let stroke = make(color, opacity, 0.001, true, id)?;
            total_points = total_points.saturating_add(stroke.points.len());
            if total_points > MAX_IMPORTED_POINTS {
                return Err("SVG artwork exceeds the 262,144 imported-point limit".into());
            }
            result.push(stroke);
            id = id.checked_add(1).ok_or("Stroke IDs exhausted")?;
        }
        if let Some((color, opacity, width)) = path.stroke {
            let radius = (width * scale * 0.5).clamp(0.001, 1.0);
            let stroke = make(color, opacity, radius, false, id)?;
            total_points = total_points.saturating_add(stroke.points.len());
            if total_points > MAX_IMPORTED_POINTS {
                return Err("SVG artwork exceeds the 262,144 imported-point limit".into());
            }
            result.push(stroke);
            id = id.checked_add(1).ok_or("Stroke IDs exhausted")?;
        }
    }
    Ok(result)
}
