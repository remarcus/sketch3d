"""Bounded handoff tests and background imports of the checked-in export fixtures.

Run: blender --background --factory-startup --python scripts/test_sketch3d_blender_handoff.py -- verification/exports
"""
import json
import math
import shutil
import sys
import tempfile
from pathlib import Path

import bpy
from mathutils import Vector
sys.path.insert(0, str(Path(__file__).resolve().parent))
import sketch3d_blender_handoff as handoff


def expect_invalid(callback, label):
    try:
        callback()
    except (ValueError, OSError):
        return
    raise AssertionError("malformed handoff was accepted: " + label)


def malformed_tests(root):
    with tempfile.TemporaryDirectory(prefix="sketch3d-handoff-", dir=str(root)) as temp:
        temp = Path(temp)
        bad = temp / "bad.glb"
        bad.write_bytes(b"glTF" + b"\x02\0\0\0" + b"\xff" * 8)
        expect_invalid(lambda: handoff._read_glb(bad), "invalid header size")
        bad.write_bytes(b"glTF" + (2).to_bytes(4, "little") + (20).to_bytes(4, "little") + b"\0" * 8)
        expect_invalid(lambda: handoff._read_glb(bad), "missing JSON chunk")
        huge_json = b" " * (handoff.MAX_JSON_BYTES + 1)
        bad.write_bytes(b"glTF" + (2).to_bytes(4, "little") + (20 + len(huge_json) + 8).to_bytes(4, "little") + len(huge_json).to_bytes(4, "little") + (0x4E4F534A).to_bytes(4, "little") + huge_json)
        expect_invalid(lambda: handoff._read_glb(bad), "JSON budget")
        manifest = temp / "bad.json"
        manifest.write_text('{"format":"spatial-sketch-blender-handoff","version":999}', encoding="utf-8")
        expect_invalid(lambda: handoff._read_manifest(manifest), "unsupported version")
        manifest.write_text(json.dumps({"format": handoff.FORMAT, "version": 1, "source": {"file": "missing.glb", "sha256": "0" * 64}, "strokes": []}), encoding="utf-8")
        expect_invalid(lambda: handoff.import_manifest(manifest), "missing asset")
        return 5


def assert_imported_color(obj, expected):
    material = next((item for item in obj.data.materials if item), None)
    if material is None or material.node_tree is None:
        raise AssertionError("stroke material has no shader graph: " + obj.name)
    shader = next((node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
    if shader is None:
        raise AssertionError("stroke material has no Principled shader: " + obj.name)
    socket = shader.inputs["Base Color"]
    if not socket.is_linked:
        if any(abs(actual - target) > 0.0001 for actual, target in zip(socket.default_value, expected)):
            raise AssertionError("imported base color differs from manifest: " + obj.name)
        return
    mix = socket.links[0].from_node
    if mix.type != "MIX" or mix.blend_type != "MULTIPLY":
        raise AssertionError("imported vertex color is not multiplied with base color: " + obj.name)
    factors = [item for item in mix.inputs if item.name == "B" and item.type == "RGBA" and not item.is_linked]
    if len(factors) != 1 or any(abs(actual - target) > 0.0001 for actual, target in zip(factors[0].default_value, expected)):
        raise AssertionError("imported material color factor differs from manifest: " + obj.name)
    colors = [link for item in mix.inputs if item.name == "A" and item.type == "RGBA" for link in item.links]
    if not colors or colors[0].from_node.type != "VERTEX_COLOR":
        raise AssertionError("imported vertex color attribute is missing: " + obj.name)
    layer_name = colors[0].from_node.layer_name
    if not layer_name and obj.data.color_attributes.active_color:
        layer_name = obj.data.color_attributes.active_color.name
    attr = obj.data.attributes.get(layer_name) if layer_name else None
    if attr is None or len(attr.data) == 0 or any(any(abs(channel - 1.0) > 0.0001 for channel in datum.color) for datum in attr.data):
        raise AssertionError("imported vertex color data changed base material color: " + obj.name)


def render_current_scene(output_path):
    scene = bpy.context.scene
    engines = {item.identifier for item in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items}
    scene.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in engines else "BLENDER_EEVEE"
    scene.render.resolution_x = 320
    scene.render.resolution_y = 240
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(output_path)
    world = bpy.data.worlds.new("Handoff comparison world")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.055, 0.075, 0.10, 1)
    world.node_tree.nodes["Background"].inputs[1].default_value = 0.6
    scene.world = world
    camera_data = bpy.data.cameras.new("Handoff comparison camera")
    camera = bpy.data.objects.new("Handoff comparison camera", camera_data)
    scene.collection.objects.link(camera)
    camera.location = (10, -13, 9)
    camera.rotation_euler = (Vector((0, -0.5, 1.7)) - camera.location).to_track_quat("-Z", "Y").to_euler()
    camera_data.type = "ORTHO"
    camera_data.ortho_scale = 13.5
    scene.camera = camera
    for name, location, energy, size in [
        ("Key", (6, -8, 11), 1700, 8),
        ("Fill", (-6, -3, 7), 1100, 7),
        ("Rim", (1, 7, 10), 1500, 6),
    ]:
        data = bpy.data.lights.new(name, "AREA")
        data.energy = energy
        data.shape = "DISK"
        data.size = size
        light = bpy.data.objects.new(name, data)
        scene.collection.objects.link(light)
        light.location = location
        light.rotation_euler = (Vector((0, 0, 1.6)) - light.location).to_track_quat("-Z", "Y").to_euler()
    bpy.ops.mesh.primitive_plane_add(size=200, location=(0, 0, -0.23))
    ground = bpy.context.object
    ground.name = "Handoff comparison floor"
    material = bpy.data.materials.new("Handoff comparison floor")
    material.diffuse_color = (0.055, 0.075, 0.10, 1)
    ground.data.materials.append(material)
    bpy.ops.render.render(write_still=True)
    image = bpy.data.images.load(str(output_path), check_existing=False)
    return tuple(image.size), list(image.pixels)


def compare_direct_and_handoff(root, manifest_path):
    house = Path(root) / "house.glb"
    if not house.is_file():
        return {"performed": False, "reason": "house.glb fixture is unavailable"}
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(house))
    direct_path = Path(root) / "handoff-render-direct.png"
    organized_path = Path(root) / "handoff-render-organized.png"
    direct_size, direct = render_current_scene(direct_path)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    handoff.import_manifest(manifest_path)
    organized_size, organized = render_current_scene(organized_path)
    if direct_size != organized_size or len(direct) != len(organized):
        raise AssertionError("direct and organized render sizes differ")
    delta = [abs(a - b) for a, b in zip(direct, organized)]
    mean = sum(delta) / max(1, len(delta))
    maximum = max(delta, default=0.0)
    if mean > 0.00001 or maximum > 0.01:
        raise AssertionError("collection organization changed house render (mean=%g max=%g)" % (mean, maximum))
    return {"performed": True, "resolution": list(direct_size), "directRender": direct_path.name, "organizedRender": organized_path.name, "meanAbsoluteChannelDifference": mean, "maxAbsoluteChannelDifference": maximum, "tolerance": {"mean": 0.00001, "max": 0.01}, "passed": True}


def import_mixed_resources(root, manifest_dir, plane_case):
    glb = Path(root) / plane_case["file"]
    manifest_path = manifest_dir / "mixed-resources.sketch3d-handoff.json"
    generated = handoff.create_manifest(glb, manifest_path)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    result = handoff.import_manifest(manifest_path)
    plane = bpy.data.objects.get(plane_case["name"])
    model = bpy.data.objects.get(plane_case["modelName"])
    if plane is None or model is None or plane.type != "MESH" or model.type != "MESH":
        raise AssertionError("handoff did not import mixed image/model resources")
    for obj, source_id, prop in ((plane, plane_case["id"], "spatialSketchImagePlaneId"), (model, plane_case["modelId"], "spatialSketchObjReferenceId")):
        if str(obj.get(prop)) != source_id:
            raise AssertionError("resource identity metadata lost during handoff: " + obj.name)
        if not obj.get("spatialSketchAsset"):
            raise AssertionError("portable source asset identifier missing: " + obj.name)
        if any(not math.isfinite(float(value)) for row in obj.matrix_world for value in row):
            raise AssertionError("resource transform is not finite: " + obj.name)
    plane_points = [plane.matrix_world @ vertex.co for vertex in plane.data.vertices]
    low = [min(point[axis] for point in plane_points) for axis in range(3)]
    high = [max(point[axis] for point in plane_points) for axis in range(3)]
    expected_low = (plane_case["min"][0], -plane_case["max"][2], plane_case["min"][1])
    expected_high = (plane_case["max"][0], -plane_case["min"][2], plane_case["max"][1])
    if any(abs(a - b) > 0.00025 for a, b in zip(low, expected_low)) or any(abs(a - b) > 0.00025 for a, b in zip(high, expected_high)):
        raise AssertionError("image-plane bounds changed during handoff")
    images = [node.image for material in plane.data.materials if material and material.node_tree for node in material.node_tree.nodes if node.type == "TEX_IMAGE" and node.image]
    if len(images) != 1 or images[0].packed_file is None or tuple(images[0].size) != tuple(plane_case["pixelSize"]):
        raise AssertionError("image-plane texture dependency is not embedded and packed")
    if not any(0.0 < alpha < 1.0 for alpha in list(images[0].pixels)[3::4]):
        raise AssertionError("image-plane texture lost its translucent pixels")
    model_points = [model.matrix_world @ vertex.co for vertex in model.data.vertices]
    model_low = [min(point[axis] for point in model_points) for axis in range(3)]
    model_high = [max(point[axis] for point in model_points) for axis in range(3)]
    model_min = (plane_case["modelMin"][0], -plane_case["modelMax"][2], plane_case["modelMin"][1])
    model_max = (plane_case["modelMax"][0], -plane_case["modelMin"][2], plane_case["modelMax"][1])
    if any(abs(a - b) > 0.00025 for a, b in zip(model_low, model_min)) or any(abs(a - b) > 0.00025 for a, b in zip(model_high, model_max)):
        raise AssertionError("OBJ reference bounds changed during handoff")
    shader = next((node for material in model.data.materials if material and material.node_tree for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
    if shader is None or any(abs(a - b) > 0.02 for a, b in zip(shader.inputs["Base Color"].default_value[:3], plane_case["modelDiffuse"])) or abs(shader.inputs["Alpha"].default_value - plane_case["modelOpacity"]) > 0.02:
        raise AssertionError("OBJ material color or opacity changed during handoff")
    return {"fixture": "mixed-resources", "objects": result["importedObjects"], "strokes": len(generated["strokes"]), "embeddedImage": list(images[0].size), "imagePlaneId": plane_case["id"], "modelId": plane_case["modelId"], "passed": True}


def main(root):
    root = Path(root).resolve()
    source_manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    reports = []
    manifest_count = 0
    house_manifest = None
    manifest_dir = root / "handoff-test-manifests"
    manifest_dir.mkdir(exist_ok=True)
    mixed_resources = None
    if source_manifest.get("imagePlaneFixture"):
        mixed_resources = import_mixed_resources(root, manifest_dir, source_manifest["imagePlaneFixture"])
    for case in source_manifest.get("cases", []):
        glb = root / case["file"]
        if not glb.is_file():
            continue
        output_manifest = manifest_dir / (glb.stem + ".sketch3d-handoff.json")
        generated = handoff.create_manifest(glb, output_manifest)
        manifest_count += 1
        _, source_document, _ = handoff._read_glb(glb)
        for image in source_document.get("images", []):
            if "bufferView" not in image:
                raise AssertionError("fixture has an image dependency not embedded in its GLB: " + case["name"])
        if case["name"] == "house":
            house_manifest = output_manifest
        bpy.ops.wm.read_factory_settings(use_empty=True)
        result = handoff.import_manifest(output_manifest)
        stroke_objects = [obj for obj in bpy.context.scene.objects if obj.type == "MESH" and obj.get("sketch3d_stroke_id")]
        if len(stroke_objects) != len(generated["strokes"]):
            raise AssertionError("stroke count differs for " + case["name"])
        if result["strokes"] != len(stroke_objects):
            raise AssertionError("import report count differs for " + case["name"])
        expected_by_id = {str(item["id"]): item for item in case["strokes"]}
        for obj in stroke_objects:
            data = next(item for item in generated["strokes"] if item["id"] == obj["sketch3d_stroke_id"])
            if not obj.data.materials:
                raise AssertionError("stroke has no material: " + obj.name)
            imported_material_names = {material.name for material in obj.data.materials if material}
            if any(name not in imported_material_names for name in data.get("materialNames", [])):
                raise AssertionError("material organization differs from GLB manifest: " + obj.name)
            if any(not any(node.type == "BSDF_PRINCIPLED" for node in material.node_tree.nodes) for material in obj.data.materials if material and material.use_nodes and material.node_tree):
                raise AssertionError("imported material has no Principled shader: " + obj.name)
            if any(not math.isfinite(float(v)) for row in obj.matrix_world for v in row):
                raise AssertionError("stroke has a non-finite transform: " + obj.name)
            expected = expected_by_id.get(obj["sketch3d_stroke_id"])
            if expected:
                points = [obj.matrix_world @ vertex.co for vertex in obj.data.vertices]
                low = [min(point[axis] for point in points) for axis in range(3)]
                high = [max(point[axis] for point in points) for axis in range(3)]
                expected_low = (expected["min"][0], -expected["max"][2], expected["min"][1])
                expected_high = (expected["max"][0], -expected["min"][2], expected["max"][1])
                if any(abs(actual - target) > 0.00025 for actual, target in zip(low, expected_low)):
                    raise AssertionError("minimum transformed bound changed: " + obj.name)
                if any(abs(actual - target) > 0.00025 for actual, target in zip(high, expected_high)):
                    raise AssertionError("maximum transformed bound changed: " + obj.name)
                if not expected.get("masked", False) and "spatialSketchGradientStopsLinearRgb" not in data["extras"]:
                    assert_imported_color(obj, expected["color"] + [1.0])
            layer_id = data.get("layerId")
            group_id = data.get("groupId")
            collection_ids = {collection.get("sketch3dLayerId") for collection in obj.users_collection}
            group_ids = {collection.get("sketch3dGroupId") for collection in obj.users_collection}
            if layer_id and layer_id not in collection_ids:
                raise AssertionError("stroke missing its layer collection: " + obj.name)
            if group_id and group_id not in group_ids:
                raise AssertionError("stroke missing its group collection: " + obj.name)
            if "spatialSketchMaterial" in data["extras"] and obj.get("sketch3d_Material") != data["extras"]["spatialSketchMaterial"]:
                raise AssertionError("material semantics metadata lost: " + obj.name)
            for key, value in data["extras"].items():
                if isinstance(value, (str, int, float, bool)) or value is None:
                    if obj.get("sketch3d_" + key.removeprefix("spatialSketch")) != value:
                        raise AssertionError("source metadata was not retained: " + key)
        reports.append({"fixture": case["name"], "manifestStrokes": len(generated["strokes"]), "importedStrokes": len(stroke_objects), "objects": result["importedObjects"], "layers": result["layers"], "groups": result["groups"], "embeddedImages": len(source_document.get("images", [])), "passed": True})
    if not reports:
        raise AssertionError("no original GLB fixtures found")
    render_comparison = compare_direct_and_handoff(root, house_manifest) if house_manifest else {"performed": False, "reason": "house fixture has no handoff manifest"}
    malformed = malformed_tests(root)
    shutil.rmtree(manifest_dir)
    report = {
        "passed": True,
        "blender": bpy.app.version_string,
        "manifestVersion": handoff.VERSION,
        "fixtures": reports,
        "mixedResources": mixed_resources,
        "malformedCasesRejected": malformed,
        "checks": ["versioned manifest generation", "SHA-256 source validation", "layer/group collection organization", "stroke material metadata and Principled color graphs", "transformed bounds against fixture expectations", "finite Blender transforms", "embedded image/model dependencies and translucency", "bounded malformed input"],
        "renderComparison": render_comparison,
        "limitations": ["Brush dynamics and procedural texture behavior are not reconstructed.", "Gradient color is the GLB's baked vertex color; gradient metadata is retained.", "Cutout is transparent geometry and cannot erase destination pixels.", "Layer blend modes fall back to normal in GLB.", "Linked groups are baked geometry; group IDs are organized as Blender collections without live instancing.", "Assets must be embedded in the GLB; external GLTF URIs are rejected."],
    }
    report_path = root / "blender-handoff-report.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("Sketch3D Blender handoff passed in Blender %s: %d fixture imports, %d bounded malformed-input cases" % (bpy.app.version_string, len(reports), malformed))


if __name__ == "__main__":
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(args) != 1:
        raise SystemExit("usage: blender --background --python scripts/test_sketch3d_blender_handoff.py -- verification/exports")
    main(args[0])
