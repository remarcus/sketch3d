"""Versioned Spatial Sketch GLB manifest and Blender organizer.

Run inside Blender:
  blender --background --python scripts/sketch3d_blender_handoff.py -- --manifest out.json drawing.glb
  blender --background --python scripts/sketch3d_blender_handoff.py -- --import out.json
"""
import hashlib
import json
import os
import struct
import sys
from pathlib import Path

import bpy

FORMAT = "spatial-sketch-blender-handoff"
VERSION = 1
MAX_GLB_BYTES = 512 * 1024 * 1024
MAX_JSON_BYTES = 64 * 1024 * 1024
MAX_NODES = 100_000
MAX_STROKES = 50_000
MAX_NAME = 1024


def _fail(message):
    raise ValueError("Sketch3D handoff: " + message)


def _read_glb(path):
    path = Path(path).resolve(strict=True)
    size = path.stat().st_size
    if size < 20 or size > MAX_GLB_BYTES:
        _fail("GLB size is outside the supported 20-byte to 512-MiB range")
    with path.open("rb") as stream:
        header = stream.read(12)
        magic, version, declared_size = struct.unpack("<4sII", header)
        if magic != b"glTF" or version != 2 or declared_size != size:
            _fail("invalid GLB 2 header or declared length")
        chunk_header = stream.read(8)
        if len(chunk_header) != 8:
            _fail("missing JSON chunk")
        length, kind = struct.unpack("<II", chunk_header)
        if kind != 0x4E4F534A or length == 0 or length > MAX_JSON_BYTES or length > size - 20:
            _fail("invalid or over-budget JSON chunk")
        raw = stream.read(length)
        try:
            document = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
            _fail("invalid GLB JSON: " + str(error))
    if not isinstance(document, dict):
        _fail("GLB document root must be an object")
    for section in ("nodes", "meshes", "materials", "images", "buffers"):
        values = document.get(section, [])
        if not isinstance(values, list):
            _fail("GLB " + section + " must be an array")
        if len(values) > MAX_NODES:
            _fail("GLB " + section + " exceeds the supported item budget")
    if len(document.get("nodes", [])) > MAX_NODES:
        _fail("GLB node count exceeds budget")
    for buffer in document.get("buffers", []):
        if not isinstance(buffer, dict) or "uri" in buffer:
            _fail("external GLTF buffers are unsupported; provide a self-contained GLB")
    for image in document.get("images", []):
        if not isinstance(image, dict) or "uri" in image:
            _fail("external images are unsupported; embed textures in the GLB")
    strokes = []
    for node in document.get("nodes", []):
        if not isinstance(node, dict):
            _fail("GLB node must be an object")
        extras = node.get("extras", {})
        if extras is None:
            extras = {}
        if not isinstance(extras, dict):
            _fail("node extras must be an object")
        if "spatialSketchStrokeId" in extras:
            if len(strokes) >= MAX_STROKES:
                _fail("stroke count exceeds 50,000")
            strokes.append((node, extras))
        name = node.get("name", "")
        if not isinstance(name, str) or len(name) > MAX_NAME:
            _fail("node name is invalid or too long")
    return path, document, strokes


def _clean_string(value, label, limit=MAX_NAME):
    if value is None:
        return ""
    if not isinstance(value, (str, int, float, bool)):
        _fail(label + " has an unsupported value")
    result = str(value)
    if len(result) > limit:
        _fail(label + " exceeds length limit")
    return result


def create_manifest(glb_path, output_path):
    path, doc, stroke_nodes = _read_glb(glb_path)
    layers = {}
    groups = {}
    materials = {}
    strokes = []
    assets = []
    for node, extras in stroke_nodes:
        layer_id = _clean_string(extras.get("spatialSketchLayerId"), "layer ID")
        layer_name = _clean_string(extras.get("spatialSketchLayerName") or "Unlayered", "layer name") or "Unlayered"
        if layer_id:
            layers.setdefault(layer_id, {"id": layer_id, "name": layer_name})
        group_id = _clean_string(extras.get("spatialSketchGroupId"), "group ID")
        if group_id:
            groups.setdefault(group_id, {"id": group_id, "name": "Group " + group_id})
        material_ids = []
        material_names = []
        mesh_index = node.get("mesh")
        if mesh_index is not None:
            if not isinstance(mesh_index, int) or isinstance(mesh_index, bool) or mesh_index < 0 or mesh_index >= len(doc.get("meshes", [])):
                _fail("stroke node has an invalid mesh index")
            primitives = doc["meshes"][mesh_index].get("primitives", [])
            if not isinstance(primitives, list) or len(primitives) > 100_000:
                _fail("mesh primitive list is invalid or over budget")
            for primitive in primitives:
                if not isinstance(primitive, dict):
                    _fail("mesh primitive must be an object")
                material_index = primitive.get("material")
                if material_index is None:
                    continue
                if not isinstance(material_index, int) or isinstance(material_index, bool) or material_index < 0 or material_index >= len(doc.get("materials", [])):
                    _fail("stroke primitive has an invalid material index")
                material_ids.append(material_index)
                source_material = doc["materials"][material_index]
                material_name = _clean_string(source_material.get("name") or "Material_%04d" % material_index, "material name")
                material_names.append(material_name)
                materials.setdefault(material_name, {
                    "id": material_index,
                    "name": material_name,
                    "baseColorFactor": (source_material.get("pbrMetallicRoughness") or {}).get("baseColorFactor", [1, 1, 1, 1]),
                    "alphaMode": source_material.get("alphaMode", "OPAQUE"),
                    "doubleSided": bool(source_material.get("doubleSided", False)),
                })
        material_name = _clean_string(extras.get("spatialSketchMaterialName"), "material name")
        stroke = {
            "id": _clean_string(extras.get("spatialSketchStrokeId"), "stroke ID", 128),
            "name": _clean_string(node.get("name") or "Stroke", "stroke name"),
            "layerId": layer_id or None,
            "layerName": layer_name,
            "groupId": group_id or None,
            "materialName": material_name or None,
            "materialIds": sorted(set(material_ids)),
            "materialNames": sorted(set(material_names)),
            "extras": extras,
        }
        strokes.append(stroke)
        asset = extras.get("spatialSketchAsset")
        if asset is not None:
            asset = _clean_string(asset, "asset reference", 4096)
            if asset not in assets:
                assets.append(asset)
    for node in doc.get("nodes", []):
        extras = node.get("extras", {}) or {}
        for key in ("spatialSketchAsset",):
            if extras.get(key) is not None:
                asset = _clean_string(extras[key], "asset reference", 4096)
                if asset not in assets:
                    assets.append(asset)
    manifest = {
        "format": FORMAT,
        "version": VERSION,
        "source": {"file": os.path.relpath(path, Path(output_path).resolve().parent).replace(os.sep, "/"), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()},
        "units": {"length": "meter", "unitInMeters": float((doc.get("extras") or {}).get("unitInMeters", 1.0))},
        "coordinates": {"source": "+X right, +Y up, +Z front", "blender": "+X right, +Y depth, +Z up", "glbToBlender": "(x, y, z) -> (x, -z, y); Blender glTF importer applies this transform"},
        "collections": {"layers": list(layers.values()), "groups": list(groups.values())},
        "materials": list(materials.values()),
        "strokes": strokes,
        "assets": assets,
        "limits": {"maxGlbBytes": MAX_GLB_BYTES, "maxJsonBytes": MAX_JSON_BYTES, "maxNodes": MAX_NODES, "maxStrokes": MAX_STROKES},
    }
    target = Path(output_path).resolve()
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return manifest


def _read_manifest(path):
    path = Path(path).resolve(strict=True)
    if path.stat().st_size > MAX_JSON_BYTES:
        _fail("manifest exceeds 64-MiB limit")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        _fail("invalid manifest JSON: " + str(error))
    if not isinstance(data, dict) or data.get("format") != FORMAT or data.get("version") != VERSION:
        _fail("unsupported manifest format or version")
    source = data.get("source")
    if not isinstance(source, dict) or not isinstance(source.get("file"), str) or len(source["file"]) > 4096:
        _fail("manifest source file is missing or invalid")
    if not isinstance(source.get("sha256"), str) or len(source["sha256"]) != 64:
        _fail("manifest source hash is invalid")
    strokes = data.get("strokes")
    if not isinstance(strokes, list) or len(strokes) > MAX_STROKES:
        _fail("manifest stroke list is invalid or over budget")
    for item in strokes:
        if not isinstance(item, dict) or not isinstance(item.get("id"), str) or len(item["id"]) > 128:
            _fail("manifest contains an invalid stroke entry")
        if not isinstance(item.get("extras", {}), dict):
            _fail("stroke extras must be an object")
    for section in ("materials", "assets"):
        values = data.get(section, [])
        if not isinstance(values, list) or len(values) > MAX_NODES:
            _fail("manifest " + section + " list is invalid or over budget")
    collections = data.get("collections", {})
    if not isinstance(collections, dict):
        _fail("manifest collections must be an object")
    for section in ("layers", "groups"):
        values = collections.get(section, [])
        if not isinstance(values, list) or len(values) > MAX_NODES:
            _fail("manifest " + section + " list is invalid or over budget")
    return path, data


def _resolve_source(manifest_path, source):
    candidate = (manifest_path.parent / source["file"]).resolve(strict=True)
    if not candidate.is_file() or candidate.stat().st_size > MAX_GLB_BYTES:
        _fail("source GLB is missing or over budget")
    digest = hashlib.sha256(candidate.read_bytes()).hexdigest()
    if digest != source["sha256"]:
        _fail("source GLB hash does not match manifest")
    return candidate


def _set_custom_properties(owner, prefix, extras):
    for key, value in extras.items():
        if isinstance(value, (str, int, float, bool)) or value is None:
            try:
                owner[prefix + key.removeprefix("spatialSketch")] = value
            except (TypeError, ValueError):
                pass
        elif isinstance(value, (list, dict)):
            encoded = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
            if len(encoded) <= 16_384:
                owner[prefix + key.removeprefix("spatialSketch")] = encoded


def import_manifest(manifest_path, collection_name="Sketch3D Handoff"):
    manifest_path, data = _read_manifest(manifest_path)
    glb = _resolve_source(manifest_path, data["source"])
    # The bounded parser rejects remote/sidecar dependencies before Blender's importer runs.
    _read_glb(glb)
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(glb))
    imported = list(set(bpy.data.objects) - before)
    by_id = {}
    stroke_data = {item["id"]: item for item in data["strokes"]}
    for obj in imported:
        sid = str(obj.get("spatialSketchStrokeId", ""))
        if sid in stroke_data:
            by_id[sid] = obj
    root = bpy.data.collections.new(collection_name)
    bpy.context.scene.collection.children.link(root)
    layer_collections = {}
    for layer in data.get("collections", {}).get("layers", []):
        coll = bpy.data.collections.new("Layer " + (layer.get("name") or layer["id"]))
        coll["sketch3dLayerId"] = layer["id"]
        coll["sketch3dLayerName"] = layer.get("name", "")
        root.children.link(coll)
        layer_collections[layer["id"]] = coll
    group_collections = {}
    for group in data.get("collections", {}).get("groups", []):
        coll = bpy.data.collections.new("Group " + group["id"])
        coll["sketch3dGroupId"] = group["id"]
        root.children.link(coll)
        group_collections[group["id"]] = coll
    for sid, obj in by_id.items():
        item = stroke_data[sid]
        extras = item.get("extras", {})
        _set_custom_properties(obj, "sketch3d_", extras)
        obj["sketch3d_manifest_version"] = VERSION
        obj["sketch3d_stroke_id"] = sid
        layer_id = item.get("layerId")
        group_id = item.get("groupId")
        destinations = []
        if layer_id in layer_collections:
            destinations.append(layer_collections[layer_id])
        if group_id in group_collections:
            destinations.append(group_collections[group_id])
        if not destinations:
            destinations.append(root)
        for collection in list(obj.users_collection):
            collection.objects.unlink(obj)
        for collection in destinations:
            collection.objects.link(obj)
    # Non-stroke imported geometry (image planes and referenced models) stays in root
    # and retains source extras from the GLB importer.
    for obj in imported:
        if obj.name in bpy.data.objects and obj not in by_id.values():
            if not obj.users_collection:
                root.objects.link(obj)
            _set_custom_properties(obj, "sketch3d_", {key: obj[key] for key in obj.keys() if key.startswith("spatialSketch")})
    for material in bpy.data.materials:
        for obj in imported:
            if obj.type == "MESH" and material.name in {item.name for item in obj.data.materials if item}:
                material["sketch3d_handoff"] = FORMAT
                material["sketch3d_mapping"] = "GLB Principled material; see source extras for original brush/gradient semantics"
    return {"importedObjects": len(imported), "strokes": len(by_id), "layers": len(layer_collections), "groups": len(group_collections), "source": str(glb)}


def main(args):
    if len(args) < 2:
        _fail("usage: --manifest OUTPUT.json INPUT.glb | --import MANIFEST.json")
    if args[0] == "--manifest" and len(args) == 3:
        manifest = create_manifest(args[2], args[1])
        print("Created v%d manifest for %d strokes: %s" % (VERSION, len(manifest["strokes"]), args[1]))
    elif args[0] == "--import" and len(args) == 2:
        print(json.dumps(import_manifest(args[1]), indent=2))
    else:
        _fail("usage: --manifest OUTPUT.json INPUT.glb | --import MANIFEST.json")


if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    main(argv)
