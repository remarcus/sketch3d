"""Import the generated OBJ/MTL material fixture into Blender and inspect it.

Run: blender --background --factory-startup --python scripts/verify_obj_materials_blender.py -- verification/obj-materials
Generate the fixture first with:
  cargo run -- --write-obj-material-fixture verification/obj-materials
"""
import bpy
import json
import sys
from pathlib import Path

root = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
obj_path = root / "materials.obj"
if not obj_path.is_file():
    raise FileNotFoundError(obj_path)

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
bpy.ops.wm.obj_import(filepath=str(obj_path))
objects = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
if not objects:
    raise AssertionError("Blender imported no fixture meshes")
expected = {
    "material_00000": {"mode": "shaded", "alpha": 1.0, "emission": False},
    "material_00001": {"mode": "unlit", "alpha": 1.0, "emission": False},
    "material_00002": {"mode": "emissive", "alpha": 1.0, "emission": True},
    "material_00003": {"mode": "cutout", "alpha": 0.0, "emission": False},
}
report = {}
for name, requirement in expected.items():
    material = bpy.data.materials.get(name)
    if material is None:
        raise AssertionError(f"Blender did not import {name}")
    if material.node_tree is None:
        raise AssertionError(f"{name} has no imported shader node tree")
    shader = next((node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
    if shader is None:
        raise AssertionError(f"{name} has no Principled BSDF")
    base_color = tuple(shader.inputs["Base Color"].default_value)
    alpha = float(shader.inputs["Alpha"].default_value)
    emission_socket = shader.inputs.get("Emission Color") or shader.inputs.get("Emission")
    emission = tuple(emission_socket.default_value) if emission_socket else (0.0, 0.0, 0.0, 1.0)
    emission_strength = float(shader.inputs["Emission Strength"].default_value)
    if any(abs(actual - expected_value) > 0.02 for actual, expected_value in zip(base_color[:3], (0.25, 0.5, 0.75))):
        raise AssertionError(f"{name} lost Kd diffuse color: {base_color}")
    if abs(alpha - requirement["alpha"]) > 0.02:
        raise AssertionError(f"{name} imported alpha {alpha}, expected {requirement['alpha']}")
    has_emission = emission_strength > 0.01 and any(channel > 0.01 for channel in emission[:3])
    if has_emission != requirement["emission"]:
        raise AssertionError(f"{name} imported emission strength {emission_strength}, expected enabled={requirement['emission']}")
    report[name] = {
        "mode": requirement["mode"],
        "diffuse": list(base_color),
        "alpha": alpha,
        "emission": list(emission),
        "emission_strength": emission_strength,
        "emission_imported": has_emission,
    }

assigned = set()
for obj in objects:
    materials = list(obj.data.materials)
    for polygon in obj.data.polygons:
        if polygon.material_index >= len(materials) or materials[polygon.material_index] is None:
            raise AssertionError(f"{obj.name} has a polygon without an imported material")
        assigned.add(materials[polygon.material_index].name)
if not set(expected).issubset(assigned):
    raise AssertionError(f"used mesh material assignments differ: {assigned}")
polygon_count = sum(len(obj.data.polygons) for obj in objects)
if polygon_count == 0:
    raise AssertionError("Blender imported no stroke geometry")
mtl_name = next((line[6:].strip() for line in obj_path.read_text(encoding="utf-8").splitlines() if line.startswith("mtllib ")), None)
if not mtl_name:
    raise AssertionError("OBJ does not reference an MTL file")
mtl = (root / mtl_name).read_text(encoding="utf-8")
if "destination erasure is not represented" not in mtl:
    raise AssertionError("cutout limitation is missing from the exported MTL")
if "Ke 0.250000 0.500000 0.750000" not in mtl:
    raise AssertionError("emissive MTL does not include the stored emission color")

(root / "blender-obj-material-report.json").write_text(
    json.dumps({
        "passed": True,
        "blender": bpy.app.version_string,
        "meshes": [obj.name for obj in objects],
        "polygon_count": polygon_count,
        "materials": report,
        "additional_imported_materials": sorted(assigned - set(expected)),
        "checks": ["OBJ geometry imported", "four MTL materials assigned", "Kd colors preserved", "d opacity imported", "Ke emission declaration preserved", "cutout limitation disclosed"],
        "limitations": ["Blender 5.2 warns that illum 0 is unsupported by Principled BSDF, so its import does not retain the unlit-versus-shaded distinction."],
    }, indent=2),
    encoding="utf-8",
)
print(f"OBJ material verification passed in Blender {bpy.app.version_string}: {polygon_count} polygons across {len(objects)} meshes, {len(report)} materials")
