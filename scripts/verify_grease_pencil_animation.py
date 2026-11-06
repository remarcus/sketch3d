"""Import and edit an animated Sketch3D Grease Pencil handoff in Blender.

Run inside Blender 4.3+:
  blender --background --factory-startup --python scripts/verify_grease_pencil_animation.py -- generated.py report.json

Generate ``generated.py`` from the Rust fixture test by setting
``SKETCH3D_HANDOFF_SCRIPT_OUT`` to the desired output path.
"""

import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def inspect_handoff(generated_script):
    namespace = {"__name__": "__main__"}
    exec(compile(generated_script.read_text(encoding="utf-8"), str(generated_script), "exec"), namespace)
    data = namespace["DATA"]
    obj = bpy.data.objects.get(data["name"])
    require(obj is not None, "generated Grease Pencil object is missing")
    require(obj.type == "GREASEPENCIL", "handoff did not create a Grease Pencil object")
    actual_layers = {layer.name: layer for layer in obj.data.layers}
    require(set(actual_layers) == {layer["name"] for layer in data["layers"]}, "layer names differ from payload")

    layer_reports = []
    for expected_layer in data["layers"]:
        layer = actual_layers[expected_layer["name"]]
        expected_frames = expected_layer["frames"]
        actual_frames = {frame.frame_number: frame for frame in layer.frames}
        expected_numbers = [frame["frame"] for frame in expected_frames]
        require(sorted(actual_frames) == expected_numbers, f"frame numbers differ on {layer.name}")
        require(layer.hide == expected_layer["hidden"], f"visibility differs on {layer.name}")
        for expected in expected_frames:
            drawing = actual_frames[expected["frame"]].drawing
            require(
                len(drawing.strokes) == len(expected["strokes"]),
                f"drawing count differs on {layer.name} at frame {expected['frame']}",
            )
            expected_points = sum(len(stroke["points"]) for stroke in expected["strokes"])
            actual_points = len(drawing.attributes["position"].data)
            require(actual_points == expected_points, f"point count differs on {layer.name} at frame {expected['frame']}")
        for frame_number in range(data["timing"]["start"], data["timing"]["end"] + 1):
            expected_hold = next(
                (frame for frame in reversed(expected_frames) if frame["frame"] <= frame_number),
                None,
            )
            resolved = layer.get_frame_at(frame_number)
            if expected_hold is None:
                require(resolved is None, f"unexpected held drawing on {layer.name} before its first key")
            else:
                require(resolved is not None, f"held drawing missing on {layer.name} at frame {frame_number}")
                require(resolved.frame_number == expected_hold["frame"], f"wrong held key on {layer.name} at frame {frame_number}")
                require(
                    len(resolved.drawing.strokes) == len(expected_hold["strokes"]),
                    f"wrong held drawing coverage on {layer.name} at frame {frame_number}",
                )
        layer_reports.append(
            {
                "name": layer.name,
                "frames": [
                    {"frame": frame["frame"], "strokes": len(frame["strokes"])}
                    for frame in expected_frames
                ],
            }
        )

    timing = data.get("timing")
    if timing:
        scene = bpy.context.scene
        require(scene.render.fps == timing["fps"], "FPS differs from handoff")
        require(scene.frame_start == timing["start"], "start frame differs from handoff")
        require(scene.frame_end == timing["end"], "end frame differs from handoff")
        require(scene.frame_current == timing["current"], "current frame differs from handoff")

    animated = next(
        (layer for layer in obj.data.layers if sum(bool(len(frame.drawing.strokes)) for frame in layer.frames) >= 2),
        None,
    )
    require(animated is not None, "fixture has no layer with two editable drawings")
    keyed_frames = sorted(frame.frame_number for frame in animated.frames)
    source_number = next(
        number
        for number in reversed(keyed_frames)
        if len(next(frame for frame in animated.frames if frame.frame_number == number).drawing.strokes) > 0
    )
    source_frame = next(frame for frame in animated.frames if frame.frame_number == source_number)
    drawing = source_frame.drawing
    position = drawing.attributes["position"].data[0]
    before = Vector(position.vector)
    after = before + Vector((0.125, 0.0, 0.0))
    position.vector = after
    require((Vector(position.vector) - after).length < 1.0e-6, "Grease Pencil point edit did not persist")

    target_number = next(
        number for number in range(source_number + 1, source_number + 100) if number not in keyed_frames
    )
    animated.frames.move(source_number, target_number)
    moved_numbers = sorted(frame.frame_number for frame in animated.frames)
    require(source_number not in moved_numbers and target_number in moved_numbers, "Grease Pencil key retime failed")
    moved_frame = next(frame for frame in animated.frames if frame.frame_number == target_number)
    require((Vector(moved_frame.drawing.attributes["position"].data[0].vector) - after).length < 1.0e-6, "edited stroke changed during retime")

    return {
        "blender": bpy.app.version_string,
        "object": obj.name,
        "fps": bpy.context.scene.render.fps,
        "start": bpy.context.scene.frame_start,
        "end": bpy.context.scene.frame_end,
        "current": bpy.context.scene.frame_current,
        "layers": layer_reports,
        "editedLayer": animated.name,
        "editedPointBefore": list(before),
        "editedPointAfter": list(after),
        "retimedFrom": source_number,
        "retimedTo": target_number,
        "unsupportedSemantics": data["unsupported"],
    }


def main():
    args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(args) != 2:
        raise SystemExit("usage: generated.py report.json")
    generated_script, report_path = map(Path, args)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report = inspect_handoff(generated_script)
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(
        "Sketch3D animated Grease Pencil handoff passed in Blender %s: %d layers, holds, edit, retime"
        % (report["blender"], len(report["layers"]))
    )


if __name__ == "__main__":
    main()
