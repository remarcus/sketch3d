//! Bounded import of editable, solid-color SVG artwork onto a planar guide.

use crate::curve_geometry::{CubicBezier, PressureSegment};
use crate::document::{Brush, CurveAnchor, CurveData, PlanarFill, Point, Stroke};
use glam::{Vec2, Vec3};
use quick_xml::{events::Event, Reader, XmlVersion};

pub const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SVG_ELEMENTS: usize = 512;
pub const MAX_SVG_SEGMENTS: usize = 4096;
pub const MAX_IMPORTED_POINTS: usize = 262_144;
const MAX_STYLESHEET_BYTES: usize = 64 * 1024;
const MAX_CSS_RULES: usize = 256;
const MAX_SVG_CONTOURS: usize = 512;
const MAX_SVG_CONTOUR_TESTS: usize = 5_000_000;

#[derive(Clone, Debug)]
pub struct SvgPath {
    pub contours: Vec<SvgContour>,
    pub fill: Option<([f32; 3], f32)>,
    pub stroke: Option<([f32; 3], f32, f32)>,
    pub fill_rule: SvgFillRule,
}

#[derive(Clone, Debug)]
pub struct SvgContour {
    pub segments: Vec<CubicBezier>,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum SvgFillRule {
    #[default]
    NonZero,
    EvenOdd,
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

#[derive(Clone, Debug)]
struct SvgStyle {
    fill: String,
    stroke: String,
    opacity: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
    stroke_width: f32,
}

#[derive(Clone, Debug)]
struct CssRule {
    selector: CssSelector,
    declarations: std::collections::HashMap<String, String>,
    order: usize,
}

#[derive(Clone, Debug)]
enum CssSelector {
    Element(String),
    Class(String),
    Id(String),
    ElementClass(String, String),
}

impl CssSelector {
    fn specificity(&self) -> u8 {
        match self {
            Self::Element(_) => 1,
            Self::Class(_) | Self::ElementClass(_, _) => 10,
            Self::Id(_) => 100,
        }
    }

    fn matches(&self, element: &str, attrs: &std::collections::HashMap<String, String>) -> bool {
        let has_class = |class: &str| {
            attrs
                .get("class")
                .is_some_and(|classes| classes.split_ascii_whitespace().any(|item| item == class))
        };
        match self {
            Self::Element(name) => element == name,
            Self::Class(class) => has_class(class),
            Self::Id(id) => attrs.get("id").is_some_and(|value| value == id),
            Self::ElementClass(name, class) => element == name && has_class(class),
        }
    }
}

impl Default for SvgStyle {
    fn default() -> Self {
        Self {
            fill: "black".into(),
            stroke: "none".into(),
            opacity: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
        }
    }
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
    let css_rules = parse_stylesheets(text)?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut view_box = None;
    let mut paths = Vec::new();
    let mut skipped = 0;
    let mut elements = 0;
    let mut transforms = vec![Affine::IDENTITY];
    let mut styles = vec![SvgStyle::default()];
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("Invalid SVG XML: {e}"))?;
        let is_empty = matches!(&event, Event::Empty(_));
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
                        .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                        .map_err(|e| format!("Invalid SVG attribute value: {e}"))?
                        .into_owned();
                    attrs.insert(key, value);
                }
                let inline_style = attrs.remove("style");
                let mut matching_rules: Vec<_> = css_rules
                    .iter()
                    .filter(|rule| rule.selector.matches(&name, &attrs))
                    .collect();
                matching_rules.sort_by_key(|rule| (rule.selector.specificity(), rule.order));
                let mut styled_attrs = std::collections::HashMap::new();
                for rule in matching_rules {
                    styled_attrs.extend(rule.declarations.clone());
                }
                styled_attrs.extend(attrs);
                if let Some(inline) = inline_style {
                    for declaration in inline.split(';') {
                        if let Some((key, value)) = declaration.split_once(':') {
                            styled_attrs
                                .insert(key.trim().to_ascii_lowercase(), value.trim().into());
                        }
                    }
                }
                let mut attrs = styled_attrs;
                let parent_transform = *transforms.last().unwrap_or(&Affine::IDENTITY);
                let parent_style = styles.last().cloned().unwrap_or_default();
                if name == "svg" {
                    view_box = attrs
                        .get("viewbox")
                        .and_then(|v| numbers(v))
                        .filter(|n| n.len() == 4 && n[2] > 0.0 && n[3] > 0.0)
                        .map(|n| [n[0], n[1], n[2], n[3]]);
                    if !is_empty {
                        let own = attrs
                            .get("transform")
                            .map(|v| parse_transform(v))
                            .unwrap_or(Some(Affine::IDENTITY))
                            .ok_or("Unsupported SVG root transform")?;
                        transforms.push(own.then(parent_transform));
                        styles.push(derive_style(&parent_style, &attrs));
                    }
                } else if name == "g" {
                    if !is_empty {
                        let own = attrs
                            .get("transform")
                            .map(|v| parse_transform(v))
                            .unwrap_or(Some(Affine::IDENTITY))
                            .ok_or("Unsupported SVG group transform")?;
                        transforms.push(own.then(parent_transform));
                        styles.push(derive_style(&parent_style, &attrs));
                    }
                } else if matches!(
                    name.as_str(),
                    "path" | "polygon" | "polyline" | "rect" | "circle" | "ellipse" | "line"
                ) {
                    if name == "line" {
                        attrs.entry("fill".into()).or_insert_with(|| "none".into());
                    }
                    let own = attrs
                        .get("transform")
                        .map(|v| parse_transform(v))
                        .unwrap_or(Some(Affine::IDENTITY));
                    let transform = own.map(|local| local.then(parent_transform));
                    let geometry = if let Some(transform) = transform {
                        parse_element_geometry(&name, &attrs).ok().map(|contours| {
                            contours
                                .into_iter()
                                .map(|contour| SvgContour {
                                    segments: contour
                                        .segments
                                        .into_iter()
                                        .map(|s| CubicBezier {
                                            p0: transform.apply(s.p0.truncate()).extend(0.0),
                                            p1: transform.apply(s.p1.truncate()).extend(0.0),
                                            p2: transform.apply(s.p2.truncate()).extend(0.0),
                                            p3: transform.apply(s.p3.truncate()).extend(0.0),
                                        })
                                        .collect(),
                                    closed: contour.closed,
                                })
                                .collect::<Vec<_>>()
                        })
                    } else {
                        None
                    };
                    if let Some(contours) = geometry.filter(|contours| {
                        !contours.is_empty()
                            && contours
                                .iter()
                                .map(|contour| contour.segments.len())
                                .sum::<usize>()
                                <= MAX_SVG_SEGMENTS
                    }) {
                        let transform_scale = transform
                            .map(|matrix| (matrix.a * matrix.d - matrix.b * matrix.c).abs().sqrt())
                            .unwrap_or(1.0);
                        let mut style = derive_style(&parent_style, &attrs);
                        if name == "line" {
                            style.fill = "none".into();
                        }
                        let fill = parse_color(&style.fill).map(|color| {
                            (color, (style.opacity * style.fill_opacity).clamp(0.0, 1.0))
                        });
                        let stroke = parse_color(&style.stroke).map(|color| {
                            (
                                color,
                                (style.opacity * style.stroke_opacity).clamp(0.0, 1.0),
                                style.stroke_width * transform_scale,
                            )
                        });
                        if fill.is_none() && stroke.is_none() {
                            skipped += 1;
                        } else {
                            paths.push(SvgPath {
                                contours,
                                fill,
                                stroke,
                                fill_rule: match attrs
                                    .get("fill-rule")
                                    .map(|value| value.trim().to_ascii_lowercase())
                                    .as_deref()
                                {
                                    Some("evenodd") => SvgFillRule::EvenOdd,
                                    _ => SvgFillRule::NonZero,
                                },
                            });
                        }
                    } else {
                        skipped += 1;
                    }
                } else if !matches!(
                    name.as_str(),
                    "svg" | "g" | "title" | "desc" | "metadata" | "style"
                ) {
                    skipped += 1;
                }
            }
            Event::End(ref e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                if name == "svg" || name == "g" {
                    if transforms.len() > 1 {
                        transforms.pop();
                        styles.pop();
                    }
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
            for contour in &path.contours {
                for s in &contour.segments {
                    for p in [s.p0, s.p1, s.p2, s.p3] {
                        min = min.min(p.truncate());
                        max = max.max(p.truncate());
                    }
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

fn parse_stylesheets(svg: &str) -> Result<Vec<CssRule>, String> {
    let lower = svg.to_ascii_lowercase();
    let mut cursor = 0;
    let mut rules = Vec::new();
    let mut stylesheet_bytes = 0;
    while let Some(start) = lower[cursor..].find("<style") {
        let start = cursor + start;
        if lower
            .as_bytes()
            .get(start + "<style".len())
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
        {
            cursor = start + "<style".len();
            continue;
        }
        let Some(open_end) = lower[start..].find('>') else {
            break;
        };
        let content_start = start + open_end + 1;
        let Some(close_start) = lower[content_start..].find("</style") else {
            break;
        };
        let content_end = content_start + close_start;
        let stylesheet = &svg[content_start..content_end];
        stylesheet_bytes += stylesheet.len();
        if stylesheet_bytes > MAX_STYLESHEET_BYTES {
            return Err("SVG embedded styles exceed the 64 KiB import limit".into());
        }
        let clean = strip_css_comments(stylesheet);
        for rule in clean.split('}') {
            let Some((selectors, declarations)) = rule.split_once('{') else {
                continue;
            };
            let declarations = parse_declarations(declarations);
            if declarations.is_empty() {
                continue;
            }
            for selector in selectors.split(',').filter_map(parse_simple_selector) {
                if rules.len() >= MAX_CSS_RULES {
                    return Err("SVG exceeds the 256 embedded CSS rule import limit".into());
                }
                let order = rules.len();
                rules.push(CssRule {
                    selector,
                    declarations: declarations.clone(),
                    order,
                });
            }
        }
        cursor = content_end + "</style".len();
    }
    Ok(rules)
}

fn strip_css_comments(css: &str) -> String {
    let mut output = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("*/") else {
            return output;
        };
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    output
}

fn parse_simple_selector(selector: &str) -> Option<CssSelector> {
    let selector = selector.trim();
    if selector.is_empty()
        || selector
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '>' | '+' | '~' | ':' | '[' | ']'))
    {
        return None;
    }
    if let Some(id) = selector.strip_prefix('#') {
        return valid_css_identifier(id).then(|| CssSelector::Id(id.to_owned()));
    }
    if let Some(class) = selector.strip_prefix('.') {
        return valid_css_identifier(class).then(|| CssSelector::Class(class.to_owned()));
    }
    if let Some((element, class)) = selector.split_once('.') {
        if valid_css_identifier(element) && valid_css_identifier(class) {
            return Some(CssSelector::ElementClass(
                element.to_ascii_lowercase(),
                class.to_owned(),
            ));
        }
        return None;
    }
    valid_css_identifier(selector).then(|| CssSelector::Element(selector.to_ascii_lowercase()))
}

fn valid_css_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn parse_declarations(declarations: &str) -> std::collections::HashMap<String, String> {
    declarations
        .split(';')
        .filter_map(|declaration| {
            let (key, value) = declaration.split_once(':')?;
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().trim_end_matches("!important").trim();
            matches!(
                key.as_str(),
                "fill"
                    | "stroke"
                    | "fill-rule"
                    | "opacity"
                    | "fill-opacity"
                    | "stroke-opacity"
                    | "stroke-width"
            )
            .then(|| (key, value.to_owned()))
        })
        .collect()
}

fn derive_style(parent: &SvgStyle, attrs: &std::collections::HashMap<String, String>) -> SvgStyle {
    let number = |key: &str, fallback: f32| {
        attrs
            .get(key)
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(fallback)
    };
    SvgStyle {
        fill: attrs
            .get("fill")
            .cloned()
            .unwrap_or_else(|| parent.fill.clone()),
        stroke: attrs
            .get("stroke")
            .cloned()
            .unwrap_or_else(|| parent.stroke.clone()),
        opacity: parent.opacity * number("opacity", 1.0).clamp(0.0, 1.0),
        fill_opacity: parent.fill_opacity * number("fill-opacity", 1.0).clamp(0.0, 1.0),
        stroke_opacity: parent.stroke_opacity * number("stroke-opacity", 1.0).clamp(0.0, 1.0),
        stroke_width: number("stroke-width", parent.stroke_width).clamp(0.001, 10_000.0),
    }
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
                            e: -ns[1],
                            f: -ns[2],
                            ..Affine::IDENTITY
                        })
                        .then(rot)
                        .then(Affine {
                            e: ns[1],
                            f: ns[2],
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
        result = t.then(result);
        rest = rest[close + 1..].trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }
    Some(result)
}

fn parse_path(s: &str) -> Result<Vec<SvgContour>, String> {
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
    let mut contours = Vec::new();
    let mut cursor = Vec2::ZERO;
    let mut start = Vec2::ZERO;
    let mut cmd = ' ';
    let mut previous_command = ' ';
    let mut last_cubic_control: Option<Vec2> = None;
    let mut last_quadratic_control: Option<Vec2> = None;
    let mut index = 0;
    let mut closed = false;
    while index < tokens.len() {
        if tokens[index].len() == 1 && tokens[index].as_bytes()[0].is_ascii_alphabetic() {
            cmd = tokens[index].chars().next().unwrap();
            index += 1;
            if cmd == 'Z' || cmd == 'z' {
                if cursor.distance_squared(start) > 1e-12 {
                    seg.push(line(cursor, start));
                }
                cursor = start;
                closed = true;
                previous_command = 'Z';
                last_cubic_control = None;
                last_quadratic_control = None;
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
        match upper {
            'M' | 'L' => {
                let p = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                if upper == 'M' {
                    if !seg.is_empty() {
                        contours.push(SvgContour {
                            segments: std::mem::take(&mut seg),
                            closed,
                        });
                        if contours.len() > MAX_SVG_CONTOURS {
                            return Err("SVG path exceeds the 512 contour import limit".into());
                        }
                        closed = false;
                    }
                    start = p;
                    last_cubic_control = None;
                    last_quadratic_control = None;
                    cmd = if rel { 'l' } else { 'L' };
                } else {
                    seg.push(line(cursor, p));
                }
                cursor = p;
            }
            'H' => {
                let x = read_path_number(&tokens, &mut index)?;
                let p = Vec2::new(if rel { cursor.x + x } else { x }, cursor.y);
                seg.push(line(cursor, p));
                cursor = p;
            }
            'V' => {
                let y = read_path_number(&tokens, &mut index)?;
                let p = Vec2::new(cursor.x, if rel { cursor.y + y } else { y });
                seg.push(line(cursor, p));
                cursor = p;
            }
            'C' => {
                let p1 = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                let p2 = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                let p3 = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                seg.push(CubicBezier {
                    p0: cursor.extend(0.0),
                    p1: p1.extend(0.0),
                    p2: p2.extend(0.0),
                    p3: p3.extend(0.0),
                });
                cursor = p3;
                last_cubic_control = Some(p2);
                last_quadratic_control = None;
            }
            'S' => {
                let p1 = if matches!(previous_command, 'C' | 'S') {
                    last_cubic_control.map_or(cursor, |control| cursor * 2.0 - control)
                } else {
                    cursor
                };
                let p2 = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                let p3 = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                seg.push(CubicBezier {
                    p0: cursor.extend(0.0),
                    p1: p1.extend(0.0),
                    p2: p2.extend(0.0),
                    p3: p3.extend(0.0),
                });
                cursor = p3;
                last_cubic_control = Some(p2);
                last_quadratic_control = None;
            }
            'Q' => {
                let q = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                let p = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                let p1 = cursor.lerp(q, 2.0 / 3.0);
                let p2 = p.lerp(q, 2.0 / 3.0);
                seg.push(CubicBezier {
                    p0: cursor.extend(0.0),
                    p1: p1.extend(0.0),
                    p2: p2.extend(0.0),
                    p3: p.extend(0.0),
                });
                cursor = p;
                last_quadratic_control = Some(q);
                last_cubic_control = None;
            }
            'T' => {
                let q = if matches!(previous_command, 'Q' | 'T') {
                    last_quadratic_control.map_or(cursor, |control| cursor * 2.0 - control)
                } else {
                    cursor
                };
                let p = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                let p1 = cursor.lerp(q, 2.0 / 3.0);
                let p2 = p.lerp(q, 2.0 / 3.0);
                seg.push(CubicBezier {
                    p0: cursor.extend(0.0),
                    p1: p1.extend(0.0),
                    p2: p2.extend(0.0),
                    p3: p.extend(0.0),
                });
                cursor = p;
                last_quadratic_control = Some(q);
                last_cubic_control = None;
            }
            'A' => {
                let rx = read_path_number(&tokens, &mut index)?.abs();
                let ry = read_path_number(&tokens, &mut index)?.abs();
                let rotation_degrees = read_path_number(&tokens, &mut index)?;
                let large_arc = read_svg_flag(&mut tokens, &mut index)?;
                let sweep = read_svg_flag(&mut tokens, &mut index)?;
                let end = point(
                    read_path_number(&tokens, &mut index)?,
                    read_path_number(&tokens, &mut index)?,
                    cursor,
                    rel,
                );
                seg.extend(arc_segments(
                    cursor,
                    end,
                    rx,
                    ry,
                    rotation_degrees,
                    large_arc,
                    sweep,
                )?);
                cursor = end;
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            _ => return Err(format!("Unsupported SVG path command {cmd}")),
        }
        previous_command = upper;
        if seg.len() > MAX_SVG_SEGMENTS {
            return Err("SVG path exceeds the segment limit".into());
        }
    }
    if !seg.is_empty() {
        if contours.len() >= MAX_SVG_CONTOURS {
            return Err("SVG path exceeds the 512 contour import limit".into());
        }
        contours.push(SvgContour {
            segments: seg,
            closed,
        });
    }
    if contours.is_empty() {
        return Err("SVG path is empty".into());
    }
    Ok(contours)
}

fn read_svg_flag(tokens: &mut [String], index: &mut usize) -> Result<bool, String> {
    let token = tokens.get_mut(*index).ok_or("Incomplete SVG arc command")?;
    let first = token.chars().next().ok_or("Incomplete SVG arc flag")?;
    if first != '0' && first != '1' {
        return Err("SVG arc flags must be either zero or one".into());
    }
    if token.len() == 1 {
        *index += 1;
    } else {
        *token = token[1..].to_owned();
    }
    Ok(first == '1')
}

fn read_path_number(tokens: &[String], index: &mut usize) -> Result<f32, String> {
    let value = tokens
        .get(*index)
        .ok_or("Incomplete SVG path command")?
        .parse::<f32>()
        .map_err(|_| "Invalid SVG path number")?;
    *index += 1;
    if !value.is_finite() || value.abs() > 1_000_000.0 {
        return Err("SVG path coordinate exceeds the import limit".into());
    }
    Ok(value)
}

fn line(a: Vec2, b: Vec2) -> CubicBezier {
    CubicBezier {
        p0: a.extend(0.0),
        p1: a.lerp(b, 1.0 / 3.0).extend(0.0),
        p2: a.lerp(b, 2.0 / 3.0).extend(0.0),
        p3: b.extend(0.0),
    }
}

fn arc_segments(
    from: Vec2,
    to: Vec2,
    mut rx: f32,
    mut ry: f32,
    rotation_degrees: f32,
    large_arc: bool,
    sweep: bool,
) -> Result<Vec<CubicBezier>, String> {
    if !from.is_finite()
        || !to.is_finite()
        || !rx.is_finite()
        || !ry.is_finite()
        || !rotation_degrees.is_finite()
        || rx > 1_000_000.0
        || ry > 1_000_000.0
    {
        return Err("SVG arc parameters exceed the import limit".into());
    }
    if from.distance_squared(to) <= 1e-12 {
        return Ok(Vec::new());
    }
    if rx <= 1e-6 || ry <= 1e-6 {
        return Ok(vec![line(from, to)]);
    }
    rx = rx.max(1e-6);
    ry = ry.max(1e-6);
    let phi = rotation_degrees
        .to_radians()
        .rem_euclid(std::f32::consts::TAU);
    let (sin_phi, cos_phi) = phi.sin_cos();
    let midpoint = (from + to) * 0.5;
    let half_delta = (from - to) * 0.5;
    let x_prime = cos_phi * half_delta.x + sin_phi * half_delta.y;
    let y_prime = -sin_phi * half_delta.x + cos_phi * half_delta.y;
    let lambda = x_prime * x_prime / (rx * rx) + y_prime * y_prime / (ry * ry);
    if lambda > 1.0 {
        let correction = lambda.sqrt();
        rx *= correction;
        ry *= correction;
    }
    if !rx.is_finite() || !ry.is_finite() || rx > 1_000_000.0 || ry > 1_000_000.0 {
        return Err("Corrected SVG arc radii exceed the import limit".into());
    }
    let numerator =
        (rx * rx * ry * ry - rx * rx * y_prime * y_prime - ry * ry * x_prime * x_prime).max(0.0);
    let denominator = rx * rx * y_prime * y_prime + ry * ry * x_prime * x_prime;
    if denominator <= 1e-12 {
        return Ok(vec![line(from, to)]);
    }
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let coefficient = sign * (numerator / denominator).sqrt();
    let center_prime = Vec2::new(
        coefficient * rx * y_prime / ry,
        coefficient * -ry * x_prime / rx,
    );
    let center = Vec2::new(
        cos_phi * center_prime.x - sin_phi * center_prime.y + midpoint.x,
        sin_phi * center_prime.x + cos_phi * center_prime.y + midpoint.y,
    );
    let unit_start = Vec2::new(
        (x_prime - center_prime.x) / rx,
        (y_prime - center_prime.y) / ry,
    );
    let unit_end = Vec2::new(
        (-x_prime - center_prime.x) / rx,
        (-y_prime - center_prime.y) / ry,
    );
    let theta = unit_start.y.atan2(unit_start.x);
    let cross = unit_start.x * unit_end.y - unit_start.y * unit_end.x;
    let mut delta = cross.atan2(unit_start.dot(unit_end));
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let pieces = (delta.abs() / std::f32::consts::FRAC_PI_2)
        .ceil()
        .clamp(1.0, 4.0) as usize;
    let step = delta / pieces as f32;
    let ellipse_point = |angle: f32| {
        let local = Vec2::new(rx * angle.cos(), ry * angle.sin());
        center
            + Vec2::new(
                cos_phi * local.x - sin_phi * local.y,
                sin_phi * local.x + cos_phi * local.y,
            )
    };
    let ellipse_tangent = |angle: f32| {
        let local = Vec2::new(-rx * angle.sin(), ry * angle.cos());
        Vec2::new(
            cos_phi * local.x - sin_phi * local.y,
            sin_phi * local.x + cos_phi * local.y,
        )
    };
    let mut result = Vec::with_capacity(pieces);
    for piece in 0..pieces {
        let a0 = theta + step * piece as f32;
        let a1 = a0 + step;
        let alpha = 4.0 / 3.0 * (step * 0.25).tan();
        let p0 = if piece == 0 { from } else { ellipse_point(a0) };
        let p3 = if piece + 1 == pieces {
            to
        } else {
            ellipse_point(a1)
        };
        result.push(CubicBezier {
            p0: p0.extend(0.0),
            p1: (p0 + ellipse_tangent(a0) * alpha).extend(0.0),
            p2: (p3 - ellipse_tangent(a1) * alpha).extend(0.0),
            p3: p3.extend(0.0),
        });
    }
    Ok(result)
}

fn parse_element_geometry(
    name: &str,
    attrs: &std::collections::HashMap<String, String>,
) -> Result<Vec<SvgContour>, String> {
    let value = |key: &str, default: f32| -> Result<f32, String> {
        let result = attrs
            .get(key)
            .map(|text| text.parse::<f32>())
            .transpose()
            .map_err(|_| format!("Invalid SVG {key} value"))?
            .unwrap_or(default);
        if !result.is_finite() || result.abs() > 1_000_000.0 {
            return Err(format!("SVG {key} exceeds the import limit"));
        }
        Ok(result)
    };
    match name {
        "path" => attrs
            .get("d")
            .ok_or_else(|| "SVG path is missing its d attribute".to_owned())
            .and_then(|data| parse_path(data)),
        "polygon" | "polyline" => attrs
            .get("points")
            .ok_or_else(|| "SVG point shape is missing its points attribute".to_owned())
            .and_then(|points| parse_poly(points, name == "polygon"))
            .map(|(segments, closed)| vec![SvgContour { segments, closed }]),
        "rect" => {
            let x = value("x", 0.0)?;
            let y = value("y", 0.0)?;
            let width = value("width", 0.0)?;
            let height = value("height", 0.0)?;
            let mut rx = value("rx", 0.0)?;
            let mut ry = value("ry", 0.0)?;
            if width <= 0.0 || height <= 0.0 || rx < 0.0 || ry < 0.0 {
                return Err("SVG rectangle dimensions and corner radii must be nonnegative".into());
            }
            if !attrs.contains_key("rx") {
                rx = ry;
            }
            if !attrs.contains_key("ry") {
                ry = rx;
            }
            rx = rx.min(width * 0.5);
            ry = ry.min(height * 0.5);
            if rx <= 0.0 || ry <= 0.0 {
                Ok(vec![SvgContour {
                    segments: segments_for_points(
                        &[
                            Vec2::new(x, y),
                            Vec2::new(x + width, y),
                            Vec2::new(x + width, y + height),
                            Vec2::new(x, y + height),
                        ],
                        true,
                    ),
                    closed: true,
                }])
            } else {
                Ok(vec![SvgContour {
                    segments: rounded_rect_segments(x, y, width, height, rx, ry),
                    closed: true,
                }])
            }
        }
        "circle" | "ellipse" => {
            let cx = value("cx", 0.0)?;
            let cy = value("cy", 0.0)?;
            let (rx, ry) = if name == "circle" {
                let radius = value("r", 0.0)?;
                (radius, radius)
            } else {
                (value("rx", 0.0)?, value("ry", 0.0)?)
            };
            if rx <= 0.0 || ry <= 0.0 {
                return Err("SVG circle or ellipse radii must be positive".into());
            }
            Ok(vec![SvgContour {
                segments: ellipse_segments(cx, cy, rx, ry),
                closed: true,
            }])
        }
        "line" => {
            let start = Vec2::new(value("x1", 0.0)?, value("y1", 0.0)?);
            let end = Vec2::new(value("x2", 0.0)?, value("y2", 0.0)?);
            if start.distance_squared(end) <= 1e-10 {
                return Err("SVG line endpoints must be distinct".into());
            }
            Ok(vec![SvgContour {
                segments: vec![line(start, end)],
                closed: false,
            }])
        }
        _ => Err("Unsupported SVG element".into()),
    }
}

fn ellipse_segments(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<CubicBezier> {
    const KAPPA: f32 = 0.552_284_8;
    let center = Vec2::new(cx, cy);
    let right = center + Vec2::new(rx, 0.0);
    let bottom = center + Vec2::new(0.0, ry);
    let left = center - Vec2::new(rx, 0.0);
    let top = center - Vec2::new(0.0, ry);
    vec![
        CubicBezier {
            p0: right.extend(0.0),
            p1: (right + Vec2::new(0.0, ry * KAPPA)).extend(0.0),
            p2: (bottom + Vec2::new(rx * KAPPA, 0.0)).extend(0.0),
            p3: bottom.extend(0.0),
        },
        CubicBezier {
            p0: bottom.extend(0.0),
            p1: (bottom - Vec2::new(rx * KAPPA, 0.0)).extend(0.0),
            p2: (left + Vec2::new(0.0, ry * KAPPA)).extend(0.0),
            p3: left.extend(0.0),
        },
        CubicBezier {
            p0: left.extend(0.0),
            p1: (left - Vec2::new(0.0, ry * KAPPA)).extend(0.0),
            p2: (top - Vec2::new(rx * KAPPA, 0.0)).extend(0.0),
            p3: top.extend(0.0),
        },
        CubicBezier {
            p0: top.extend(0.0),
            p1: (top + Vec2::new(rx * KAPPA, 0.0)).extend(0.0),
            p2: (right - Vec2::new(0.0, ry * KAPPA)).extend(0.0),
            p3: right.extend(0.0),
        },
    ]
}

fn rounded_rect_segments(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rx: f32,
    ry: f32,
) -> Vec<CubicBezier> {
    let right = x + width;
    let bottom = y + height;
    let mut segments = vec![line(Vec2::new(x + rx, y), Vec2::new(right - rx, y))];
    segments.push(ellipse_arc_piece(
        right - rx,
        y + ry,
        rx,
        ry,
        -std::f32::consts::FRAC_PI_2,
        0.0,
    ));
    segments.push(line(
        Vec2::new(right, y + ry),
        Vec2::new(right, bottom - ry),
    ));
    segments.push(ellipse_arc_piece(
        right - rx,
        bottom - ry,
        rx,
        ry,
        0.0,
        std::f32::consts::FRAC_PI_2,
    ));
    segments.push(line(
        Vec2::new(right - rx, bottom),
        Vec2::new(x + rx, bottom),
    ));
    segments.push(ellipse_arc_piece(
        x + rx,
        bottom - ry,
        rx,
        ry,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
    ));
    segments.push(line(Vec2::new(x, bottom - ry), Vec2::new(x, y + ry)));
    segments.push(ellipse_arc_piece(
        x + rx,
        y + ry,
        rx,
        ry,
        std::f32::consts::PI,
        3.0 * std::f32::consts::FRAC_PI_2,
    ));
    segments
}

fn ellipse_arc_piece(cx: f32, cy: f32, rx: f32, ry: f32, start: f32, end: f32) -> CubicBezier {
    let center = Vec2::new(cx, cy);
    let point = |angle: f32| center + Vec2::new(rx * angle.cos(), ry * angle.sin());
    let tangent = |angle: f32| Vec2::new(-rx * angle.sin(), ry * angle.cos());
    let alpha = 4.0 / 3.0 * ((end - start) * 0.25).tan();
    let p0 = point(start);
    let p3 = point(end);
    CubicBezier {
        p0: p0.extend(0.0),
        p1: (p0 + tangent(start) * alpha).extend(0.0),
        p2: (p3 - tangent(end) * alpha).extend(0.0),
        p3: p3.extend(0.0),
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
    Ok((segments_for_points(&pts, closed), closed))
}

fn segments_for_points(points: &[Vec2], closed: bool) -> Vec<CubicBezier> {
    let mut segments = points
        .windows(2)
        .map(|pair| line(pair[0], pair[1]))
        .collect::<Vec<_>>();
    if closed && points.len() > 1 {
        segments.push(line(*points.last().unwrap(), points[0]));
    }
    segments
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
        let stroke_contours = path
            .contours
            .iter()
            .map(|contour| transform_contour(contour, &world, false))
            .collect::<Vec<_>>();
        let fill_contours = path
            .contours
            .iter()
            .map(|contour| transform_contour(contour, &world, path.fill.is_some()))
            .collect::<Vec<_>>();
        let make = |segments: &[CubicBezier],
                    color: [f32; 3],
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
            let local_fill_contours = path
                .contours
                .iter()
                .map(|contour| transform_contour(contour, &|point| point, true))
                .collect::<Vec<_>>();
            for (outer, holes) in compound_fill_groups(&local_fill_contours, path.fill_rule)? {
                let mut stroke = make(&fill_contours[outer], color, opacity, 0.001, true, id)?;
                total_points = total_points.saturating_add(stroke.points.len());
                if total_points > MAX_IMPORTED_POINTS {
                    return Err("SVG artwork exceeds the 262,144 imported-point limit".into());
                }
                let mut hole_points = Vec::with_capacity(holes.len());
                for hole in holes {
                    let contour = make(&fill_contours[hole], color, opacity, 0.001, false, id)?
                        .points
                        .into_iter()
                        .map(|point| point.position)
                        .collect::<Vec<_>>();
                    let contour = without_repeated_closure(contour);
                    if contour.len() < 3 {
                        return Err("Filled SVG hole has too few points".into());
                    }
                    total_points = total_points.saturating_add(contour.len());
                    if total_points > MAX_IMPORTED_POINTS {
                        return Err("SVG artwork exceeds the 262,144 imported-point limit".into());
                    }
                    hole_points.push(contour);
                }
                if let Some(fill) = stroke.fill.as_mut() {
                    fill.holes = hole_points;
                }
                result.push(stroke);
                id = id.checked_add(1).ok_or("Stroke IDs exhausted")?;
            }
        }
        if let Some((color, opacity, width)) = path.stroke {
            let radius = (width * scale * 0.5).clamp(0.001, 1.0);
            for contour in &stroke_contours {
                let stroke = make(contour, color, opacity, radius, false, id)?;
                total_points = total_points.saturating_add(stroke.points.len());
                if total_points > MAX_IMPORTED_POINTS {
                    return Err("SVG artwork exceeds the 262,144 imported-point limit".into());
                }
                result.push(stroke);
                id = id.checked_add(1).ok_or("Stroke IDs exhausted")?;
            }
        }
    }
    Ok(result)
}

fn transform_contour(
    contour: &SvgContour,
    world: &impl Fn(Vec3) -> Vec3,
    close_for_fill: bool,
) -> Vec<CubicBezier> {
    let mut segments = contour.segments.clone();
    if close_for_fill && !contour.closed {
        if let (Some(first), Some(last)) = (segments.first().copied(), segments.last().copied()) {
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
    for segment in &mut segments {
        segment.p0 = world(segment.p0);
        segment.p1 = world(segment.p1);
        segment.p2 = world(segment.p2);
        segment.p3 = world(segment.p3);
    }
    segments
}

fn without_repeated_closure(mut points: Vec<Vec3>) -> Vec<Vec3> {
    if points.len() > 2 && points[0].distance_squared(*points.last().unwrap()) <= 1e-10 {
        points.pop();
    }
    points
}

fn compound_fill_groups(
    contours: &[Vec<CubicBezier>],
    fill_rule: SvgFillRule,
) -> Result<Vec<(usize, Vec<usize>)>, String> {
    if contours.is_empty() {
        return Ok(Vec::new());
    }
    let mut polygons = Vec::with_capacity(contours.len());
    let mut point_count = 0_usize;
    for contour in contours {
        let polygon = flatten_contour(contour);
        point_count = point_count.saturating_add(polygon.len());
        if point_count > MAX_IMPORTED_POINTS {
            return Err("SVG compound path exceeds the contour point budget".into());
        }
        polygons.push(polygon);
    }
    if point_count.saturating_mul(polygons.len()) > MAX_SVG_CONTOUR_TESTS {
        return Err("SVG compound path exceeds the contour nesting work limit".into());
    }
    if polygons.iter().any(|polygon| polygon.len() < 3) {
        return Err("Filled SVG subpath has too few points".into());
    }
    let areas = polygons
        .iter()
        .map(|polygon| signed_area(polygon))
        .collect::<Vec<_>>();
    let mut parents = vec![None; polygons.len()];
    for child in 0..polygons.len() {
        let sample = polygons[child][0];
        parents[child] = (0..polygons.len())
            .filter(|&candidate| {
                candidate != child
                    && areas[candidate].abs() > areas[child].abs()
                    && point_in_polygon(sample, &polygons[candidate])
            })
            .min_by(|&a, &b| areas[a].abs().total_cmp(&areas[b].abs()));
    }
    let mut depths = vec![0_usize; polygons.len()];
    let mut filled = vec![false; polygons.len()];
    for index in 0..polygons.len() {
        let mut ancestors = Vec::new();
        let mut parent = parents[index];
        while let Some(ancestor) = parent {
            if ancestors.len() >= polygons.len() {
                return Err("SVG compound path contains cyclic contour nesting".into());
            }
            ancestors.push(ancestor);
            parent = parents[ancestor];
        }
        depths[index] = ancestors.len();
        filled[index] = match fill_rule {
            SvgFillRule::EvenOdd => depths[index] % 2 == 0,
            SvgFillRule::NonZero => {
                let winding = ancestors
                    .iter()
                    .map(|&ancestor| areas[ancestor].signum() as i32)
                    .sum::<i32>()
                    + areas[index].signum() as i32;
                winding != 0
            }
        };
    }
    let mut groups = Vec::new();
    for outer in 0..polygons.len() {
        if !filled[outer] || parents[outer].is_some_and(|parent| filled[parent]) {
            continue;
        }
        let holes = (0..polygons.len())
            .filter(|&index| parents[index] == Some(outer) && !filled[index])
            .collect();
        groups.push((outer, holes));
    }
    Ok(groups)
}

fn flatten_contour(segments: &[CubicBezier]) -> Vec<Vec2> {
    const STEPS_PER_SEGMENT: usize = 8;
    let mut points = Vec::with_capacity(segments.len() * STEPS_PER_SEGMENT);
    for segment in segments {
        for step in 0..STEPS_PER_SEGMENT {
            let t = step as f32 / STEPS_PER_SEGMENT as f32;
            let one_minus_t = 1.0 - t;
            let point = segment.p0.truncate() * one_minus_t.powi(3)
                + segment.p1.truncate() * (3.0 * one_minus_t.powi(2) * t)
                + segment.p2.truncate() * (3.0 * one_minus_t * t.powi(2))
                + segment.p3.truncate() * t.powi(3);
            points.push(point);
        }
    }
    points
}

fn signed_area(points: &[Vec2]) -> f32 {
    (0..points.len())
        .map(|index| {
            let next = (index + 1) % points.len();
            points[index].perp_dot(points[next])
        })
        .sum::<f32>()
        * 0.5
}

fn point_in_polygon(point: Vec2, polygon: &[Vec2]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}
