"""Independent Blender OBJ import check for the isolated Lyon evaluation fixture.

Run: blender --background --factory-startup --python scripts/verify_lyon_evaluation_blender.py -- verification/lyon-evaluation
"""
import json
import math
import sys
from pathlib import Path

import bpy

args = sys.argv[sys.argv.index("--") + 1:]
root = Path(args[0]) if args else Path("verification/lyon-evaluation")
obj = root / "rotated_concave.obj"
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
before = set(bpy.data.objects)
bpy.ops.wm.obj_import(filepath=str(obj))
created = [o for o in bpy.data.objects if o not in before and o.type == "MESH"]
if len(created) != 1:
    raise RuntimeError(f"expected one imported mesh, got {len(created)}")
mesh_obj = created[0]
mesh = mesh_obj.data
coords = [v.co[:] for v in mesh.vertices]
if not coords or any(not math.isfinite(c) for point in coords for c in point):
    raise RuntimeError("imported vertices empty or non-finite")
faces = [list(p.vertices) for p in mesh.polygons]
if not faces or any(len(face) != 3 for face in faces):
    raise RuntimeError("expected triangle faces")
if max(p[2] for p in coords) - min(p[2] for p in coords) < 0.1:
    raise RuntimeError("expected an inclined plane with measurable Z extent")
report = {
    "passed": True,
    "blender": bpy.app.version_string,
    "imported_object": mesh_obj.name,
    "vertices": len(coords),
    "triangles": len(faces),
    "bounds": [[min(p[i] for p in coords) for i in range(3)], [max(p[i] for p in coords) for i in range(3)]],
    "finite_vertices": True,
    "all_triangles": True,
    "inclined_plane_z_extent": max(p[2] for p in coords) - min(p[2] for p in coords),
    "source_obj": obj.name,
}
(root / "blender-import-report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
print(json.dumps(report, indent=2))
