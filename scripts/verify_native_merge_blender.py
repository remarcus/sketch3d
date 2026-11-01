"""Import a native-merge GLB fixture into Blender and check inserted resources.

Run: blender --background --factory-startup --python scripts/verify_native_merge_blender.py -- verification/native-merge
Generate the fixture first with:
  cargo run -- --write-native-merge-fixture verification/native-merge
"""
import bpy
import json
import sys
from pathlib import Path

root = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
manifest = json.loads((root / "inserted-manifest.json").read_text(encoding="utf-8"))
if not manifest.get("passed") or not manifest.get("source_bundle_removed_before_reload_and_export"):
    raise AssertionError("native merge fixture did not complete source relocation and export")

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
bpy.ops.import_scene.gltf(filepath=str(root / manifest["glb"]))
meshes = {obj.name: obj for obj in bpy.context.scene.objects if obj.type == "MESH"}
if len(meshes) != manifest["export"]["strokes"] + manifest["export"]["image_planes"] + manifest["export"]["model_references"]:
    raise AssertionError(f"unexpected imported mesh count: {sorted(meshes)}")

for stroke_id in (manifest["destination_stroke_id"], manifest["inserted_stroke_id"]):
    obj = meshes.get(f"Stroke_{stroke_id}")
    if obj is None or str(obj.get("spatialSketchStrokeId")) != stroke_id:
        raise AssertionError(f"Blender lost stroke {stroke_id}")
    if len(obj.data.polygons) == 0:
        raise AssertionError(f"stroke {stroke_id} has no imported triangles")
    if stroke_id == manifest["inserted_stroke_id"] and str(obj.get("spatialSketchLayerId")) != manifest["inserted_layer_id"]:
        raise AssertionError("Blender lost the inserted layer ID")

plane = meshes.get(manifest["inserted_image_name"])
if plane is None or str(plane.get("spatialSketchImagePlaneId")) != manifest["inserted_image_plane_id"]:
    raise AssertionError("Blender lost the inserted image plane or its remapped ID")
if len(plane.data.vertices) != 4 or len(plane.data.polygons) != 2:
    raise AssertionError("inserted image plane did not import as a two-triangle quad")
uv = plane.data.uv_layers.active
if uv is None or len(uv.data) != 6:
    raise AssertionError("inserted image plane UVs did not survive GLB export")
plane_materials = [material for material in plane.data.materials if material]
images = [
    node.image
    for material in plane_materials
    if material.node_tree
    for node in material.node_tree.nodes
    if node.type == "TEX_IMAGE" and node.image
]
if len(images) != 1 or tuple(images[0].size) != tuple(manifest["inserted_image_size"]):
    raise AssertionError("inserted image plane texture did not survive GLB export")
if images[0].packed_file is None:
    raise AssertionError("inserted image plane is not self-contained in the GLB")

model = meshes.get(manifest["inserted_model_name"])
if model is None or str(model.get("spatialSketchObjReferenceId")) != manifest["inserted_model_id"]:
    raise AssertionError("Blender lost the inserted OBJ reference or its remapped ID")
if len(model.data.polygons) != manifest["inserted_model_triangle_count"]:
    raise AssertionError("inserted OBJ triangle count changed in Blender")
if model.get("spatialSketchAsset") != f"models/{manifest['inserted_model_id']}":
    raise AssertionError("inserted OBJ bundle metadata did not survive GLB export")
material = next((slot for slot in model.data.materials if slot), None)
shader = next((node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None) if material and material.node_tree else None
if shader is None:
    raise AssertionError("inserted OBJ material did not import into Blender")
base_color = tuple(shader.inputs["Base Color"].default_value)
if any(abs(actual - expected) > 0.02 for actual, expected in zip(base_color[:3], (0.1, 0.8, 0.2))):
    raise AssertionError(f"inserted OBJ diffuse material changed: {base_color}")
if abs(float(shader.inputs["Alpha"].default_value) - 0.75) > 0.02:
    raise AssertionError("inserted OBJ opacity changed")

report = {
    "passed": True,
    "blender": bpy.app.version_string,
    "glb": manifest["glb"],
    "checks": [
        "destination and inserted strokes imported with remapped IDs",
        "inserted layer ID retained",
        "inserted image plane quad, UVs, and packed PNG retained",
        "inserted OBJ mesh, remapped ID, material color, and opacity retained",
        "source bundle removed before native reload and GLB export",
    ],
    "mesh_objects": sorted(meshes),
    "image_size": list(images[0].size),
    "obj_triangles": len(model.data.polygons),
}
(root / "blender-insert-report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
print(f"Native merge GLB verification passed in Blender {bpy.app.version_string}: {len(meshes)} meshes")
