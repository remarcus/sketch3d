"""Import export fixtures into Blender and check axes, bounds, materials, and geometry.
Run: blender --background --factory-startup --python scripts/verify_glb_blender.py -- verification/exports
"""
import bpy
import json
import math
import sys
from pathlib import Path
from mathutils import Vector

root = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
reports = []

def assert_close(actual, expected, label, tolerance=0.0001):
    if len(actual) != len(expected) or any(abs(a-b) > tolerance for a,b in zip(actual, expected)):
        raise AssertionError(f"{label}: {tuple(actual)} != {tuple(expected)}")

def render_house():
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE_NEXT"
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 960
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(root / "house-blender.png")
    scene.world = bpy.data.worlds.new("Verification world")
    scene.world.use_nodes = True
    scene.world.node_tree.nodes["Background"].inputs[0].default_value = (0.055, 0.075, 0.10, 1)
    scene.world.node_tree.nodes["Background"].inputs[1].default_value = 0.6
    camera_data = bpy.data.cameras.new("Verification camera")
    camera = bpy.data.objects.new("Verification camera", camera_data)
    scene.collection.objects.link(camera)
    camera.location = (10, -13, 9)
    camera.rotation_euler = (Vector((0, -0.5, 1.7)) - camera.location).to_track_quat('-Z', 'Y').to_euler()
    camera_data.type = 'ORTHO'
    camera_data.ortho_scale = 13.5
    scene.camera = camera
    for name, location, energy, size in [
        ("Key", (6, -8, 11), 1700, 8),
        ("Fill", (-6, -3, 7), 1100, 7),
        ("Rim", (1, 7, 10), 1500, 6),
    ]:
        data = bpy.data.lights.new(name, 'AREA')
        data.energy = energy
        data.shape = 'DISK'
        data.size = size
        light = bpy.data.objects.new(name, data)
        scene.collection.objects.link(light)
        light.location = location
        light.rotation_euler = (Vector((0, 0, 1.6))-light.location).to_track_quat('-Z', 'Y').to_euler()
    bpy.ops.mesh.primitive_plane_add(size=200, location=(0, 0, -0.23))
    ground = bpy.context.object
    ground.name = "Verification floor (not exported)"
    material = bpy.data.materials.new("Verification floor")
    material.diffuse_color = (0.055, 0.075, 0.10, 1)
    ground.data.materials.append(material)
    bpy.ops.wm.save_as_mainfile(filepath=str(root / "house-import.blend"))
    bpy.ops.render.render(write_still=True)

for case in manifest["cases"]:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(root / case["file"]))
    objects = {obj.name: obj for obj in bpy.context.scene.objects if obj.type == 'MESH'}
    if set(objects) != {s["name"] for s in case["strokes"]}:
        raise AssertionError(f"Wrong imported stroke objects for {case['name']}")
    triangles = 0
    vertices = 0
    for expected in case["strokes"]:
        obj = objects[expected["name"]]
        if str(obj.get("spatialSketchStrokeId")) != expected["id"]:
            raise AssertionError("Stroke ID metadata was lost")
        points = [obj.matrix_world @ vertex.co for vertex in obj.data.vertices]
        low = [min(p[i] for p in points) for i in range(3)]
        high = [max(p[i] for p in points) for i in range(3)]
        # glTF +Y-up to Blender +Z-up: (x,y,z) becomes (x,-z,y).
        minimum, maximum = expected["min"], expected["max"]
        assert_close(low, (minimum[0], -maximum[2], minimum[1]), expected["name"]+" minimum")
        assert_close(high, (maximum[0], -minimum[2], maximum[1]), expected["name"]+" maximum")
        mat = obj.data.materials[0]
        shader = next(node for node in mat.node_tree.nodes if node.type == 'BSDF_PRINCIPLED')
        assert_close(shader.inputs["Base Color"].default_value, expected["color"]+[1], expected["name"]+" color")
        assert_close((shader.inputs["Metallic"].default_value,), (0,), "metalness")
        assert_close((shader.inputs["Roughness"].default_value,), (0.8,), "roughness")
        if mat.use_backface_culling == expected.get("doubleSided", False):
            raise AssertionError(f"Wrong sidedness for {expected['name']}")
        obj.data.calc_loop_triangles()
        triangles += len(obj.data.loop_triangles)
        vertices += len(obj.data.vertices)
        if any(not math.isfinite(component) for point in points for component in point):
            raise AssertionError("Non-finite imported geometry")
    if triangles != case["summary"]["triangles"]:
        raise AssertionError(f"Triangle count mismatch for {case['name']}")
    reports.append({"name":case["name"],"passed":True,"meshes":len(objects),"vertices":vertices,"triangles":triangles})
    if case["name"] == "house":
        render_house()

(root / "blender-report.json").write_text(json.dumps({"passed":True,"blender":bpy.app.version_string,"checks":["mesh names and stroke IDs","Y-up to Z-up orientation","per-stroke dimensions and pressure-shaped bounds","original PBR color factors","ribbon double-sided material","triangle counts","finite geometry"],"cases":reports},indent=2),encoding="utf-8")
print("Spatial Sketch GLB import verification passed", flush=True)
