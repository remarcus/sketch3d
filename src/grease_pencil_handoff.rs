//! One-way Blender Grease Pencil handoff script generation.
use crate::document::{Document, Stroke};
use serde::Serialize;

#[derive(Serialize)]
struct Payload {
    name: String,
    layers: Vec<LayerPayload>,
    unsupported: Vec<String>,
}

#[derive(Serialize)]
struct LayerPayload {
    name: String,
    strokes: Vec<StrokePayload>,
}

#[derive(Serialize)]
struct StrokePayload {
    points: Vec<PointPayload>,
}

#[derive(Serialize)]
struct PointPayload {
    position: [f32; 3],
    radius: f32,
    opacity: f32,
    color: [f32; 3],
}

pub fn build_script(document: &Document, name: &str) -> Result<(String, Vec<String>), String> {
    let mut layers: Vec<_> = document
        .layers
        .iter()
        .map(|layer| LayerPayload {
            name: layer.name.clone(),
            strokes: Vec::new(),
        })
        .collect();
    let mut unsupported = Vec::new();
    for stroke in &document.strokes {
        let Some(layer_index) = document
            .layers
            .iter()
            .position(|layer| layer.id == stroke.layer_id)
        else {
            unsupported.push(format!("Stroke {}: missing layer; skipped", stroke.id));
            continue;
        };
        if stroke.points.is_empty() {
            unsupported.push(format!(
                "Stroke {}: no centerline points; skipped",
                stroke.id
            ));
            continue;
        }
        if let Some(reason) = unsupported_semantics(stroke) {
            unsupported.push(format!("Stroke {}: {reason}", stroke.id));
        }
        let layer = &document.layers[layer_index];
        if layer.opacity != 1.0
            || layer.clip_to_below
            || layer.blend_mode != crate::document::LayerBlendMode::Normal
        {
            unsupported.push(format!(
                "Stroke {}: layer opacity, clipping or blend mode is not preserved",
                stroke.id
            ));
        }
        let point_count = stroke.points.len().saturating_sub(1).max(1) as f32;
        let distances = crate::dynamics::arc_lengths(&stroke.points);
        let total_length = distances.last().copied().unwrap_or(0.0);
        let points = stroke
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let color = stroke
                    .point_colors
                    .as_ref()
                    .and_then(|colors| colors.get(index))
                    .and_then(|color| *color)
                    .or_else(|| {
                        stroke
                            .gradient
                            .map(|gradient| gradient.sample(index as f32 / point_count))
                    })
                    .unwrap_or(stroke.color);
                let thickness = stroke
                    .point_thickness
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .copied()
                    .unwrap_or(1.0);
                PointPayload {
                    position: point.position.to_array(),
                    radius: (stroke.radius
                        * stroke.brush.thickness
                        * crate::dynamics::pressure_scale(
                            stroke.brush.dynamics.thickness,
                            point.pressure,
                        )
                        * thickness
                        * crate::dynamics::taper_factor(
                            distances[index],
                            total_length,
                            stroke.brush.dynamics.taper_start,
                            stroke.brush.dynamics.taper_end,
                        ))
                    .max(0.0001),
                    opacity: (stroke.brush.opacity
                        * crate::dynamics::opacity_scale(
                            stroke.brush.dynamics.opacity,
                            point.pressure,
                        ))
                    .clamp(0.0, 1.0),
                    color,
                }
            })
            .collect();
        layers[layer_index].strokes.push(StrokePayload { points });
    }
    let payload = Payload {
        name: name.chars().take(80).collect(),
        layers,
        unsupported: unsupported.clone(),
    };
    let payload_json = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
    // serde_json's quoted string is a valid Python string literal; json.loads
    // then safely decodes arbitrary Unicode and user-supplied layer names.
    let payload_literal =
        serde_json::to_string(&payload_json).map_err(|error| error.to_string())?;
    let template = r#"import bpy
import json

# Run with Blender 4.3 or newer (Grease Pencil 3).
DATA = json.loads(__PAYLOAD__)
gp = bpy.data.grease_pencils_v3.new(DATA["name"])
obj = bpy.data.objects.new(DATA["name"], gp)
bpy.context.collection.objects.link(obj)

for layer_data in DATA["layers"]:
    layer = gp.layers.new(layer_data["name"], set_active=False)
    frame = layer.frames.new(1)
    strokes = layer_data["strokes"]
    if not strokes:
        continue
    drawing = frame.drawing
    drawing.add_strokes([len(stroke["points"]) for stroke in strokes])
    positions = drawing.attributes["position"].data
    radii = drawing.attributes["radius"].data
    opacities = drawing.attributes["opacity"].data
    material_indices = drawing.attributes["material_index"].data
    vertex_colors = drawing.attributes.new("vertex_color", "FLOAT_COLOR", "POINT")
    point_index = 0
    for stroke_index, stroke in enumerate(strokes):
        for point in stroke["points"]:
            positions[point_index].vector = point["position"]
            radii[point_index].value = point["radius"]
            opacities[point_index].value = point["opacity"]
            color = point["color"]
            vertex_colors.data[point_index].color = (*color, point["opacity"])
            point_index += 1

    for stroke_index, stroke in enumerate(strokes):
        material = bpy.data.materials.new(f"{DATA['name']} · {layer_data['name']} · {stroke_index + 1}")
        bpy.data.materials.create_gpencil_data(material)
        base = stroke["points"][0]["color"] if stroke["points"] else [0.0, 0.0, 0.0]
        material.grease_pencil.color = (*base, 1.0)
        obj.data.materials.append(material)
        material_indices[stroke_index].value = len(obj.data.materials) - 1

print("Sketch3D Grease Pencil handoff created:", obj.name)
if DATA["unsupported"]:
    print("Unsupported source semantics (centerlines were handed off):")
    for item in DATA["unsupported"]:
        print("  -", item)
"#;
    let script = template.replace("__PAYLOAD__", &payload_literal);
    Ok((script, unsupported))
}

fn unsupported_semantics(stroke: &Stroke) -> Option<String> {
    let mut unsupported = Vec::new();
    if stroke.fill.is_some() {
        unsupported.push("fill geometry");
    }
    if stroke.curve.is_some() {
        unsupported.push("editable curve controls (centerline is flattened)");
    }
    if stroke.gradient.is_some() {
        unsupported.push("gradient is approximated as per-point color");
    }
    if stroke.brush.kind != crate::document::BrushKind::Round {
        unsupported.push("non-round brush shape");
    }
    if stroke.brush.texture.kind != crate::texture::TextureKind::Solid
        || stroke.brush.texture.image.is_some()
    {
        unsupported.push("brush texture");
    }
    if stroke.brush.flow.enabled {
        unsupported.push("timed Airbrush behavior");
    }
    if stroke.brush.dynamics.width.enabled
        || stroke.brush.dynamics.tilt_orientation
        || stroke.brush.dynamics.tilt_shading
    {
        unsupported.push("width, tilt orientation or tilt shading response");
    }
    if stroke.brush.softness > 0.0 {
        unsupported.push("soft edge/compositing behavior");
    }
    if stroke.material != crate::document::StrokeMaterial::default() {
        unsupported.push("non-default stroke material/compositing mode");
    }
    if stroke.brush.dabs.enabled
        || stroke.brush.tip_asset_id != 0
        || stroke.brush.tip_variants.iter().any(Option::is_some)
    {
        unsupported.push("dab/image-tip behavior");
    }
    if stroke.modifiers.is_some() {
        unsupported.push("live stroke modifiers");
    }
    (!unsupported.is_empty()).then(|| unsupported.join(", "))
}
