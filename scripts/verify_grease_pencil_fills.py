"""Verify editable solid-fill attributes in a Sketch3D Grease Pencil handoff.

Run inside Blender 4.3+:
  blender --background --factory-startup --python scripts/verify_grease_pencil_fills.py -- generated.py report.json

Generate ``generated.py`` by running the solid-fill handoff unit test with
``SKETCH3D_HANDOFF_SCRIPT_OUT`` set to the output path.
"""

import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def close(actual, expected, tolerance=1.0e-5):
    return abs(actual - expected) <= tolerance


def render_fill_probe(obj, data, render_path):
    probe = next(
        (
            (layer, frame, stroke)
            for layer in data["layers"]
            for frame in layer["frames"]
            for stroke in frame["strokes"]
            if stroke.get("fill")
            and all(
                close(stroke["fill"]["color"][i], expected)
                for i, expected in enumerate((0.72, 0.21, 0.13))
            )
        ),
        None,
    )
    require(probe is not None, "render probe's distinctive solid fill is missing")
    layer_data, frame_data, stroke_data = probe
    positions = [Vector(point["position"]) for point in stroke_data["points"]]
    center = sum(positions, Vector()) / len(positions)
    normal = (positions[1] - positions[0]).cross(positions[2] - positions[0]).normalized()
    require(normal.length > 0.9, "render probe fill contour is degenerate")
    camera_data = bpy.data.cameras.new("Fill appearance probe camera")
    camera = bpy.data.objects.new(camera_data.name, camera_data)
    bpy.context.collection.objects.link(camera)
    camera.location = center + normal * 20.0
    camera.rotation_euler = (-normal).to_track_quat("-Z", "Y").to_euler()
    camera_data.type = "ORTHO"
    camera_data.lens = 50
    projected = [camera.rotation_euler.to_matrix().inverted() @ (point - center) for point in positions]
    camera_data.ortho_scale = max(
        max(point.x for point in projected) - min(point.x for point in projected),
        max(point.y for point in projected) - min(point.y for point in projected),
    ) * 1.6
    bpy.context.scene.camera = camera

    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = 256
    scene.render.resolution_y = 256
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.film_transparent = True
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.render.filepath = str(render_path)
    scene.frame_set(frame_data["frame"])
    bpy.ops.render.render(write_still=True)
    image = bpy.data.images.load(str(render_path), check_existing=False)
    require(image.size[0] == 256, "Blender fill probe render failed")

    local = camera.rotation_euler.to_matrix().inverted() @ (center - camera.location)
    pixel_x = int((local.x / camera_data.ortho_scale + 0.5) * image.size[0])
    pixel_y = int((local.y / camera_data.ortho_scale + 0.5) * image.size[1])
    pixel_index = (pixel_y * image.size[0] + pixel_x) * 4
    pixel = list(image.pixels[pixel_index : pixel_index + 4])
    max_alpha = max(image.pixels[index] for index in range(3, len(image.pixels), 4))
    require(
        pixel[0] > pixel[1] * 1.25,
        f"solid fill did not render red at its interior probe: {pixel}; max alpha {max_alpha}",
    )
    require(pixel[3] > 0.2, f"solid fill rendered transparent at its interior probe: {pixel}; max alpha {max_alpha}")
    return {
        "layer": layer_data["name"],
        "frame": frame_data["frame"],
        "probePixel": pixel,
        "resolution": list(image.size),
    }


def main():
    args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(args) != 2:
        raise SystemExit("usage: generated.py report.json")
    generated_script, report_path = (Path(arg).resolve() for arg in args)
    namespace = {"__name__": "__main__"}
    exec(
        compile(generated_script.read_text(encoding="utf-8"), str(generated_script), "exec"),
        namespace,
    )
    data = namespace["DATA"]
    obj = bpy.data.objects.get(data["name"])
    require(obj is not None and obj.type == "GREASEPENCIL", "Grease Pencil object is missing")

    fill_records = []
    animated_target = None
    for layer_data in data["layers"]:
        layer = obj.data.layers.get(layer_data["name"])
        require(layer is not None, f"missing layer {layer_data['name']}")
        frames = {frame.frame_number: frame for frame in layer.frames}
        for expected_frame in layer_data["frames"]:
            frame = frames[expected_frame["frame"]]
            expected_fills = [
                stroke for stroke in expected_frame["strokes"] if stroke.get("fill")
            ]
            actual_strokes = frame.drawing.strokes
            require(
                len(actual_strokes) == len(expected_frame["strokes"]),
                f"stroke count differs on {layer.name} at frame {frame.frame_number}",
            )
            attributes = frame.drawing.attributes
            require(
                not expected_fills
                or (
                    "fill_color" in attributes
                    and "fill_opacity" in attributes
                    and "cyclic" in attributes
                ),
                f"fill attributes missing on {layer.name} at frame {frame.frame_number}: {list(attributes.keys())}",
            )
            fill_colors = attributes["fill_color"].data if expected_fills else []
            fill_opacities = attributes["fill_opacity"].data if expected_fills else []
            cyclic = attributes["cyclic"].data if expected_fills else []
            material_indices = attributes["material_index"].data if expected_fills else []
            for index, expected_stroke in enumerate(expected_frame["strokes"]):
                expected_fill = expected_stroke.get("fill")
                if not expected_fill:
                    continue
                color = fill_colors[index].color
                opacity = fill_opacities[index].value
                require(
                    all(close(color[i], expected_fill["color"][i]) for i in range(3)),
                    f"fill color differs on {layer.name} at frame {frame.frame_number}",
                )
                require(
                    close(opacity, expected_fill["opacity"]),
                    f"fill opacity differs on {layer.name} at frame {frame.frame_number}",
                )
                require(cyclic[index].value, "solid fill contour is not closed")
                material = obj.data.materials[material_indices[index].value]
                require(
                    material.grease_pencil.show_fill and material.grease_pencil.show_stroke,
                    f"Grease Pencil material hides the fill or contour for {layer.name}",
                )
                fill_records.append(
                    {
                        "layer": layer.name,
                        "frame": frame.frame_number,
                        "color": list(color)[:3],
                        "opacity": opacity,
                        "points": len(expected_stroke["points"]),
                    }
                )
            if expected_fills and len(layer.frames) > 1:
                animated_target = (layer, frame, expected_fills[0])

    require(fill_records, "fixture contains no editable solid fills")
    require(animated_target is not None, "fixture has no animated fill drawing to edit")
    render_probe = render_fill_probe(obj, data, report_path.with_suffix(".render.png"))

    layer, frame, expected_stroke = animated_target
    drawing = frame.drawing
    position = drawing.attributes["position"].data[0]
    before = Vector(position.vector)
    after = before + Vector((0.01, 0.0, 0.0))
    position.vector = after
    require((Vector(position.vector) - after).length < 1.0e-6, "fill contour point edit failed")

    source_number = frame.frame_number
    target_number = source_number + 1
    require(all(item.frame_number != target_number for item in layer.frames), "retime target is occupied")
    layer.frames.move(source_number, target_number)
    moved = next(item for item in layer.frames if item.frame_number == target_number)
    require(
        (Vector(moved.drawing.attributes["position"].data[0].vector) - after).length < 1.0e-6,
        "edited fill contour changed during retime",
    )
    fill_opacities = moved.drawing.attributes["fill_opacity"].data
    require(close(fill_opacities[0].value, expected_stroke["fill"]["opacity"]), "fill opacity changed during retime")

    report = {
        "blender": bpy.app.version_string,
        "object": obj.name,
        "editableSolidFills": fill_records,
        "renderProbe": render_probe,
        "editedContourPointBefore": list(before),
        "editedContourPointAfter": list(after),
        "retimedFrom": source_number,
        "retimedTo": target_number,
        "unsupportedSemantics": data["unsupported"],
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(
        "Sketch3D solid-fill handoff passed in Blender %s: %d editable fill frames, contour edit, retime"
        % (report["blender"], len(fill_records))
    )


if __name__ == "__main__":
    main()
