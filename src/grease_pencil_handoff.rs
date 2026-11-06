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
    timing: Option<TimingPayload>,
}

#[derive(Serialize)]
struct LayerPayload {
    name: String,
    hidden: bool,
    frames: Vec<DrawingPayload>,
}

#[derive(Serialize)]
struct DrawingPayload {
    frame: u32,
    strokes: Vec<StrokePayload>,
}

#[derive(Serialize)]
struct TimingPayload {
    fps: u32,
    start: u32,
    end: u32,
    current: u32,
}

#[derive(Serialize)]
struct StrokePayload {
    id: u64,
    points: Vec<PointPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill: Option<FillPayload>,
}

#[derive(Serialize)]
struct FillPayload {
    color: [f32; 3],
    opacity: f32,
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
    let animation = &document.animation;
    let animated = !animation.is_empty();
    if animated && !animation.current_pose_matches(&document.strokes) {
        return Err("Set or update the current keyframe before exporting the animation.".into());
    }
    let mut unsupported = Vec::new();
    if animation.interpolate {
        unsupported.push("Whole-drawing interpolation is exported as held keys.".into());
    }
    if animated && animation.two_bone_rig.is_some() {
        unsupported.push("Animated drawing export omits the two-bone rig and its poses.".into());
    }
    let mut encode = |stroke: &Stroke, layer: &crate::document::Layer| {
        if stroke.points.is_empty() {
            unsupported.push(format!(
                "Stroke {}: no centerline points; skipped",
                stroke.id
            ));
            return None;
        }
        if let Some(reason) = unsupported_semantics(stroke) {
            unsupported.push(format!("Stroke {}: {reason}", stroke.id));
        }
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
        let fill = stroke.fill.as_ref().and_then(|fill| {
            let supported = fill.holes.is_empty()
                && fill.gradient.is_none()
                && fill.texture.is_none()
                && fill.cylinder.is_none()
                && stroke.gradient.is_none();
            supported.then_some(FillPayload {
                color: stroke.color,
                opacity: (stroke.brush.opacity * layer.opacity).clamp(0.0, 1.0),
            })
        });
        Some(StrokePayload {
            id: stroke.id,
            points,
            fill,
        })
    };
    let layers = document
        .layers
        .iter()
        .map(|layer| {
            let mut drawing = |frame, strokes: &[Stroke]| DrawingPayload {
                frame,
                strokes: strokes
                    .iter()
                    .filter(|stroke| stroke.layer_id == layer.id)
                    .filter_map(|stroke| encode(stroke, layer))
                    .collect(),
            };
            let mut frames = Vec::new();
            if let Some(track) = animation
                .layer_tracks
                .iter()
                .find(|track| track.layer_id == layer.id)
            {
                for (index, key) in track.keyframes.iter().enumerate() {
                    frames.push(drawing(key.frame + 1, &key.strokes));
                    let end = key.frame + key.exposure;
                    let next = track
                        .keyframes
                        .get(index + 1)
                        .map_or(animation.range_end + 1, |key| key.frame);
                    if end < next {
                        frames.push(drawing(end + 1, &[]));
                    }
                }
                if let Some(first) = track.keyframes.first() {
                    if first.frame > animation.range_start {
                        frames.insert(0, drawing(animation.range_start + 1, &first.strokes));
                    }
                }
            } else if let Some(first) = animation.keyframes.first() {
                if first.frame > animation.range_start {
                    frames.push(drawing(animation.range_start + 1, &first.strokes));
                }
                frames.extend(
                    animation
                        .keyframes
                        .iter()
                        .map(|key| drawing(key.frame + 1, &key.strokes)),
                );
            } else {
                let strokes = animation
                    .base_strokes
                    .as_deref()
                    .filter(|_| animated)
                    .unwrap_or(&source_strokes);
                frames.push(drawing(
                    if animated {
                        animation.range_start + 1
                    } else {
                        1
                    },
                    strokes,
                ));
            }
            LayerPayload {
                name: layer.name.clone(),
                hidden: !layer.visible || !document.layer_folder_visible(layer.id),
                frames,
            }
        })
        .collect();
    unsupported.sort();
    unsupported.dedup();
    let two_bone_rig = document
        .animation
        .two_bone_rig
        .as_ref()
        .filter(|_| !animated)
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
        timing: (animated || animation.two_bone_rig.is_some()).then_some(TimingPayload {
            fps: animation.fps.into(),
            start: animation.range_start + 1,
            end: animation.range_end + 1,
            current: animation.current_frame + 1,
        }),
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
grease_pencils = getattr(bpy.data, "grease_pencils_v3", None)
if grease_pencils is None:
    grease_pencils = bpy.data.grease_pencils
gp = grease_pencils.new(DATA["name"])
obj = bpy.data.objects.new(DATA["name"], gp)
bpy.context.collection.objects.link(obj)

materials = {}

def write_drawing(frame, strokes):
    if not strokes:
        return
    drawing = frame.drawing
    drawing.add_strokes([len(stroke["points"]) for stroke in strokes])
    positions = drawing.attributes["position"].data
    radii = (drawing.attributes.get("radius") or drawing.attributes.new("radius", "FLOAT", "POINT")).data
    opacities = (drawing.attributes.get("opacity") or drawing.attributes.new("opacity", "FLOAT", "POINT")).data
    material_indices = (drawing.attributes.get("material_index") or drawing.attributes.new("material_index", "INT", "CURVE")).data
    vertex_colors = drawing.attributes.get("vertex_color") or drawing.attributes.new("vertex_color", "FLOAT_COLOR", "POINT")
    fill_colors = (drawing.attributes.get("fill_color") or drawing.attributes.new("fill_color", "FLOAT_COLOR", "CURVE")).data
    fill_opacities = (drawing.attributes.get("fill_opacity") or drawing.attributes.new("fill_opacity", "FLOAT", "CURVE")).data
    fill_ids = (drawing.attributes.get("fill_id") or drawing.attributes.new("fill_id", "INT", "CURVE")).data
    cyclic = (drawing.attributes.get("cyclic") or drawing.attributes.new("cyclic", "BOOLEAN", "CURVE")).data
    point_index = 0
    for stroke_index, stroke in enumerate(strokes):
        for point in stroke["points"]:
            positions[point_index].vector = point["position"]
            radii[point_index].value = point["radius"]
            opacities[point_index].value = point["opacity"]
            color = point["color"]
            vertex_colors.data[point_index].color = (*color, 1.0)
            point_index += 1

    for stroke_index, stroke in enumerate(strokes):
        fill = stroke.get("fill")
        if fill:
            fill_colors[stroke_index].color = (*fill["color"], 1.0)
            fill_opacities[stroke_index].value = fill["opacity"]
            fill_ids[stroke_index].value = stroke_index + 1
            cyclic[stroke_index].value = True
        base = tuple(stroke["points"][0]["color"])
        if base not in materials:
            material = bpy.data.materials.new(f"{DATA['name']} · Ink {len(materials) + 1}")
            bpy.data.materials.create_gpencil_data(material)
            material.grease_pencil.color = (*base, 1.0)
            material.grease_pencil.show_stroke = True
            material.grease_pencil.show_fill = True
            obj.data.materials.append(material)
            materials[base] = len(obj.data.materials) - 1
        material_indices[stroke_index].value = materials[base]

for layer_data in DATA["layers"]:
    layer = gp.layers.new(layer_data["name"], set_active=False)
    layer.hide = layer_data["hidden"]
    for drawing_data in layer_data["frames"]:
        write_drawing(layer.frames.new(drawing_data["frame"]), drawing_data["strokes"])

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
        for stroke in layer_data["frames"][0]["strokes"]:
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

if DATA.get("timing"):
    timing = DATA["timing"]
    scene = bpy.context.scene
    scene.render.fps = timing["fps"]
    scene.render.fps_base = 1.0
    scene.frame_start = timing["start"]
    scene.frame_end = timing["end"]
    scene.frame_set(timing["current"])

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
    if let Some(fill) = &stroke.fill {
        if !fill.holes.is_empty() {
            unsupported.push("fill holes (only the outer outline is handed off)");
        }
        if fill.gradient.is_some() || stroke.gradient.is_some() {
            unsupported.push("fill gradients (filled handoff supports solid color only)");
        }
        if fill.texture.is_some() {
            unsupported.push("fill texture");
        }
        if fill.cylinder.is_some() {
            unsupported.push("curved fill surface (only planar solid fills are handed off)");
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn payload(document: &Document) -> serde_json::Value {
        let (script, _) = build_script(document, "Fill fixture").unwrap();
        let literal = script
            .lines()
            .find_map(|line| line.strip_prefix("DATA = json.loads("))
            .unwrap();
        let encoded: String = serde_json::from_str(literal.strip_suffix(')').unwrap()).unwrap();
        serde_json::from_str(&encoded).unwrap()
    }

    #[test]
    fn export_preserves_layer_keys_and_inserts_a_blank_for_an_exposure_gap() {
        let mut document = crate::scenes::pencil_house();
        let layer_id = document.active_layer;
        let strokes: Vec<_> = document
            .strokes
            .iter()
            .filter(|stroke| stroke.layer_id == layer_id)
            .take(1)
            .cloned()
            .collect();
        let animation = document
            .animation
            .set_layer_keyframe(2, layer_id, &strokes, &document.strokes)
            .unwrap()
            .set_layer_exposure(layer_id, 2, 2)
            .unwrap()
            .set_layer_keyframe(7, layer_id, &[], &document.strokes)
            .unwrap();
        let other_layer_id = document
            .layers
            .iter()
            .find(|layer| layer.id != layer_id)
            .unwrap()
            .id;
        let other_strokes: Vec<_> = document
            .strokes
            .iter()
            .filter(|stroke| stroke.layer_id == other_layer_id)
            .take(1)
            .cloned()
            .collect();
        let mut changed_other_strokes = other_strokes.clone();
        for point in changed_other_strokes
            .iter_mut()
            .flat_map(|stroke| &mut stroke.points)
        {
            point.position.x += 0.25;
        }
        let mut animation = animation
            .set_layer_keyframe(4, other_layer_id, &other_strokes, &document.strokes)
            .unwrap()
            .set_layer_exposure(other_layer_id, 4, 3)
            .unwrap()
            .set_layer_keyframe(9, other_layer_id, &changed_other_strokes, &document.strokes)
            .unwrap();
        animation.current_frame = 7;
        document.strokes = animation.resolve(7).unwrap();
        document.animation = animation;
        let (script, _) = build_script(&document, "Animation fixture").unwrap();
        let literal = script
            .lines()
            .find_map(|line| line.strip_prefix("DATA = json.loads("))
            .unwrap();
        let payload: String = serde_json::from_str(literal.strip_suffix(')').unwrap()).unwrap();
        let data: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let track = &data["layers"][1]["frames"];
        let frames: Vec<_> = track
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| {
                (
                    frame["frame"].as_u64().unwrap(),
                    frame["strokes"].as_array().unwrap().len(),
                )
            })
            .collect();
        assert_eq!(frames, vec![(1, 1), (3, 1), (5, 0), (8, 0)]);
        let static_layer_name = document
            .layers
            .iter()
            .find(|layer| {
                !document
                    .animation
                    .layer_tracks
                    .iter()
                    .any(|track| track.layer_id == layer.id)
            })
            .unwrap()
            .name
            .clone();
        let static_layer = data["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|layer| layer["name"] == static_layer_name)
            .unwrap();
        assert_eq!(static_layer["frames"].as_array().unwrap().len(), 1);
        assert_eq!(data["timing"]["current"], 8);
        let second_track = data["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|layer| {
                layer["name"]
                    == document
                        .layers
                        .iter()
                        .find(|layer| layer.id == other_layer_id)
                        .unwrap()
                        .name
            })
            .unwrap()["frames"]
            .as_array()
            .unwrap();
        let second_frames: Vec<_> = second_track
            .iter()
            .map(|frame| {
                (
                    frame["frame"].as_u64().unwrap(),
                    frame["strokes"].as_array().unwrap().len(),
                )
            })
            .collect();
        assert_eq!(second_frames, vec![(1, 1), (5, 1), (8, 0), (10, 1)]);
        // Set by the external Blender smoke command to exercise this exact script.
        if let Some(path) = std::env::var_os("SKETCH3D_HANDOFF_SCRIPT_OUT") {
            std::fs::write(path, script).expect("write generated Blender smoke fixture");
        }
    }

    #[test]
    fn solid_fill_color_and_opacity_survive_static_and_animated_handoff_payloads() {
        let mut document = crate::scenes::pencil_house();
        let index = document
            .strokes
            .iter()
            .position(|stroke| stroke.fill.is_some())
            .unwrap();
        let mut fill = document.strokes[index].clone();
        fill.color = [0.72, 0.21, 0.13];
        fill.brush.opacity = 0.65;
        fill.brush = crate::document::Brush {
            opacity: 0.65,
            ..crate::document::Brush::default()
        };
        let region = fill.fill.as_mut().unwrap();
        region.offset = 0.01;
        region.gradient = None;
        region.texture = None;
        region.holes.clear();
        region.cylinder = None;

        document.strokes[index] = fill.clone();
        let static_payload = payload(&document);
        let layer = static_payload["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|layer| {
                layer["name"]
                    == document
                        .layers
                        .iter()
                        .find(|layer| layer.id == fill.layer_id)
                        .unwrap()
                        .name
            })
            .unwrap();
        let static_fill = &layer["frames"][0]["strokes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stroke| stroke["id"] == fill.id)
            .unwrap()["fill"];
        assert_eq!(static_fill["color"], serde_json::json!([0.72, 0.21, 0.13]));
        assert_eq!(static_fill["opacity"], serde_json::json!(0.65));
        assert!(!static_payload["unsupported"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item.as_str().unwrap().contains("fill holes")
                    || item.as_str().unwrap().contains("fill gradients")
            }));
        let mut unsupported_fill = fill.clone();
        unsupported_fill
            .fill
            .as_mut()
            .unwrap()
            .holes
            .push(vec![Vec3::ZERO, Vec3::X, Vec3::Y]);
        assert!(unsupported_semantics(&unsupported_fill)
            .unwrap()
            .contains("fill holes"));

        let animated = document
            .animation
            .set_layer_keyframe(0, fill.layer_id, &[fill.clone()], &document.strokes)
            .unwrap()
            .set_layer_keyframe(5, fill.layer_id, &[], &document.strokes)
            .unwrap();
        document.strokes = animated.resolve(animated.current_frame).unwrap();
        document.animation = animated;
        if let Some(path) = std::env::var_os("SKETCH3D_HANDOFF_SCRIPT_OUT") {
            let (script, _) = build_script(&document, "Sketch3D Solid Fill Fixture").unwrap();
            std::fs::write(path, script).expect("write generated Blender solid-fill fixture");
        }
        let animated_payload = payload(&document);
        let layer = animated_payload["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|layer| {
                layer["name"]
                    == document
                        .layers
                        .iter()
                        .find(|layer| layer.id == fill.layer_id)
                        .unwrap()
                        .name
            })
            .unwrap();
        let frames = layer["frames"].as_array().unwrap();
        assert_eq!(frames[0]["frame"], 1);
        assert_eq!(
            frames[0]["strokes"][0]["fill"]["color"],
            serde_json::json!([0.72, 0.21, 0.13])
        );
        assert_eq!(
            frames[0]["strokes"][0]["fill"]["opacity"],
            serde_json::json!(0.65)
        );
        assert_eq!(frames[1]["frame"], 6);
        assert!(frames[1]["strokes"].as_array().unwrap().is_empty());
    }
}
