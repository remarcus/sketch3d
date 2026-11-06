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


def main():
    args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(args) != 2:
        raise SystemExit("usage: generated.py report.json")
    generated_script, report_path = map(Path, args)
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
            require(not expected_fills or (
                "fill_color" in attributes and "fill_opacity" in attributes and "cyclic" in attributes,
            ), f"fill attributes missing on {layer.name} at frame {frame.frame_number}: {list(attributes.keys())}")
            fill_colors = attributes["fill_color"].data if expected_fills else []
            fill_opacities = attributes["fill_opacity"].data if expected_fills else []
            cyclic = attributes["cyclic"].data if expected_fills else []
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
