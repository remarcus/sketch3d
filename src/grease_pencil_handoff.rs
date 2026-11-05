//! One-way Blender Grease Pencil handoff script generation.
use crate::document::{Document, Stroke};
use serde::Serialize;

#[derive(Serialize)]
struct Payload {
    name: String,
    layers: Vec<LayerPayload>,
    unsupported: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    two_bone_rig: Option<TwoBoneRigPayload>,
}

#[derive(Serialize)]
struct LayerPayload {
    name: String,
    strokes: Vec<StrokePayload>,
}

#[derive(Serialize)]
struct StrokePayload {
    id: u64,
    points: Vec<PointPayload>,
}

#[derive(Serialize)]
struct TwoBoneRigPayload {
    layer_id: u64,
    root: [f32; 3],
    joint: [f32; 3],
    bindings: Vec<BoneBindingPayload>,
    poses: Vec<BonePosePayload>,
}

#[derive(Serialize)]
struct BoneBindingPayload {
    stroke_id: u64,
    child_weights: Vec<f32>,
}

#[derive(Serialize)]
struct BonePosePayload {
    frame: u32,
    root_angle: f32,
    child_angle: f32,
}

#[derive(Serialize)]
struct PointPayload {
    position: [f32; 3],
    radius: f32,
    opacity: f32,
    color: [f32; 3],
}

pub fn build_script(document: &Document, name: &str) -> Result<(String, Vec<String>), String> {
    let source_strokes = if document.animation.two_bone_rig.is_some() {
        let mut source_animation = document.animation.clone();
        source_animation.two_bone_rig = None;
        source_animation
            .resolve(document.animation.current_frame)
            .unwrap_or_else(|| document.strokes.clone())
    } else {
        document.strokes.clone()
    };
    let mut layers: Vec<_> = document
        .layers
        .iter()
        .map(|layer| LayerPayload {
            name: layer.name.clone(),
            strokes: Vec::new(),
        })
        .collect();
    let mut unsupported = Vec::new();
    for stroke in &source_strokes {
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
            || layer.mask_source.is_some()
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
        layers[layer_index].strokes.push(StrokePayload {
            id: stroke.id,
            points,
        });
    }
    let two_bone_rig = document
        .animation
        .two_bone_rig
        .as_ref()
        .map(|rig| TwoBoneRigPayload {
            layer_id: rig.layer_id,
            root: rig.root.to_array(),
            joint: rig.joint.to_array(),
            bindings: rig
                .point_weights
                .iter()
                .map(|binding| BoneBindingPayload {
                    stroke_id: binding.stroke_id,
                    child_weights: binding.child_weights.clone(),
                })
                .collect(),
            poses: rig
                .poses
                .iter()
                .map(|pose| BonePosePayload {
                    frame: pose.frame,
                    root_angle: pose.root_angle,
                    child_angle: pose.child_angle,
                })
                .collect(),
        });
    let payload = Payload {
        name: name.chars().take(80).collect(),
        layers,
        unsupported: unsupported.clone(),
        two_bone_rig,
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

if DATA.get("two_bone_rig"):
    rig = DATA["two_bone_rig"]
    arm_data = bpy.data.armatures.new(f"{DATA['name']} · Two-bone prototype")
    arm_obj = bpy.data.objects.new(f"{DATA['name']} · Two-bone prototype", arm_data)
    bpy.context.collection.objects.link(arm_obj)
    bpy.context.view_layer.objects.active = arm_obj
    arm_obj.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    root_bone = arm_data.edit_bones.new("Root")
    root_bone.head = rig["root"]
    root_bone.tail = rig["joint"]
    child_bone = arm_data.edit_bones.new("Child")
    child_bone.head = rig["joint"]
    direction = [rig["joint"][i] - rig["root"][i] for i in range(3)]
    length = max(sum(value * value for value in direction) ** 0.5, 0.001)
    child_bone.tail = [rig["joint"][i] + direction[i] / length * length for i in range(3)]
    child_bone.parent = root_bone
    child_bone.use_connect = True
    bpy.ops.object.mode_set(mode="OBJECT")

    upper_group = obj.vertex_groups.new(name="Root")
    lower_group = obj.vertex_groups.new(name="Child")
    weights = {binding["stroke_id"]: binding["child_weights"] for binding in rig["bindings"]}
    global_point = 0
    for layer_data in DATA["layers"]:
        for stroke in layer_data["strokes"]:
            stroke_weights = weights.get(stroke["id"])
            for point_index in range(len(stroke["points"])):
                if stroke_weights is None or point_index >= len(stroke_weights):
                    global_point += 1
                    continue
                child_weight = stroke_weights[point_index]
                root_weight = 1.0 - child_weight
                if root_weight > 0.0:
                    upper_group.add([global_point], root_weight, "REPLACE")
                if child_weight > 0.0:
                    lower_group.add([global_point], child_weight, "REPLACE")
                global_point += 1

    modifier = obj.modifiers.new("Two-bone cutout rig", "ARMATURE")
    modifier.object = arm_obj
    arm_obj.animation_data_create()
    action = bpy.data.actions.new(f"{DATA['name']} · Two-bone poses")
    arm_obj.animation_data.action = action
    root_pose = arm_obj.pose.bones["Root"]
    child_pose = arm_obj.pose.bones["Child"]
    root_pose.rotation_mode = "XYZ"
    child_pose.rotation_mode = "XYZ"
    for pose in rig["poses"]:
        frame = pose["frame"] + 1
        root_pose.rotation_euler.z = pose["root_angle"]
        root_pose.keyframe_insert(data_path="rotation_euler", index=2, frame=frame)
        child_pose.rotation_euler.z = pose["child_angle"]
        child_pose.keyframe_insert(data_path="rotation_euler", index=2, frame=frame)

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
