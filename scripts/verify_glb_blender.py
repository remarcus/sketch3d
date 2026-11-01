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

def assert_imported_color(obj, shader, expected, label):
    base_color = shader.inputs["Base Color"]
    if not base_color.is_linked:
        assert_close(base_color.default_value, expected, label)
        return
    mix = base_color.links[0].from_node
    if mix.type != "MIX" or mix.blend_type != "MULTIPLY":
        raise AssertionError(f"{label}: imported color is not a base-color multiply")
    factors = [socket for socket in mix.inputs if socket.name == "B" and socket.type == "RGBA" and not socket.is_linked]
    if len(factors) != 1:
        raise AssertionError(f"{label}: could not identify imported color factor")
    assert_close(factors[0].default_value, expected, label + " factor")
    color_links = [link for socket in mix.inputs if socket.name == "A" and socket.type == "RGBA" for link in socket.links]
    if not color_links or color_links[0].from_node.type != "VERTEX_COLOR":
        raise AssertionError(f"{label}: Blender did not use the imported vertex-color attribute")
    layer_name = color_links[0].from_node.layer_name
    if not layer_name and obj.data.color_attributes.active_color:
        layer_name = obj.data.color_attributes.active_color.name
    attribute = obj.data.attributes.get(layer_name)
    if attribute is None or len(attribute.data) == 0:
        raise AssertionError(f"{label}: imported vertex-color data is missing")
    if any(any(abs(channel - 1.0) > 0.0001 for channel in datum.color) for datum in attribute.data):
        raise AssertionError(f"{label}: vertex colors change the exported material color")

def render_house():
    scene = bpy.context.scene
    engines = {item.identifier for item in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items}
    scene.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in engines else "BLENDER_EEVEE"
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
    raw = (root / case["file"]).read_bytes()
    json_length = int.from_bytes(raw[12:16], "little")
    source = json.loads(raw[20:20+json_length])
    source_triangles = sum(
        source["accessors"][primitive["indices"]]["count"] // 3
        for node in source["nodes"] if "mesh" in node
        for primitive in source["meshes"][node["mesh"]]["primitives"]
    )
    if any("uri" in image for image in source.get("images", [])) or any("uri" in buffer for buffer in source["buffers"]):
        raise AssertionError("Export depends on external image or buffer sidecars")
    source_materials = {node["name"]: source["materials"][source["meshes"][node["mesh"]]["primitives"][0]["material"]] for node in source["nodes"]}
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
        if "layerId" in expected:
            if str(obj.get("spatialSketchLayerId")) != expected["layerId"] or obj.get("spatialSketchLayerName") != expected["layerName"]:
                raise AssertionError("Layer metadata was lost")
        expected_group = expected.get("groupId")
        actual_group = obj.get("spatialSketchGroupId")
        if (None if actual_group is None else str(actual_group)) != expected_group:
            raise AssertionError("Group metadata was lost or leaked onto independent ink")
        points = [obj.matrix_world @ vertex.co for vertex in obj.data.vertices]
        low = [min(p[i] for p in points) for i in range(3)]
        high = [max(p[i] for p in points) for i in range(3)]
        # glTF +Y-up to Blender +Z-up: (x,y,z) becomes (x,-z,y).
        minimum, maximum = expected["min"], expected["max"]
        assert_close(low, (minimum[0], -maximum[2], minimum[1]), expected["name"]+" minimum", tolerance=0.00025)
        assert_close(high, (maximum[0], -minimum[2], maximum[1]), expected["name"]+" maximum", tolerance=0.00025)
        mat = obj.data.materials[0]
        shader = next(node for node in mat.node_tree.nodes if node.type == 'BSDF_PRINCIPLED')
        original_material = source_materials[expected["name"]]
        assert_close(original_material["pbrMetallicRoughness"]["baseColorFactor"], expected["color"]+[1], expected["name"]+" source color")
        if expected.get("masked", False):
            if original_material.get("alphaMode") != "MASK" or original_material.get("alphaCutoff") != 0.5:
                raise AssertionError("Texture coverage is not a masked material")
            if not shader.inputs["Base Color"].is_linked:
                raise AssertionError("Imported texture does not drive base color")
            mix = shader.inputs["Base Color"].links[0].from_node
            if mix.type != 'MIX' or mix.blend_type != 'MULTIPLY':
                raise AssertionError("Imported texture is not multiplied by its original color")
            factors = [socket for socket in mix.inputs if socket.name == 'B' and socket.type == 'RGBA' and not socket.is_linked]
            if len(factors) != 1:
                raise AssertionError("Could not identify imported base color factor")
            assert_close(factors[0].default_value, expected["color"]+[1], expected["name"]+" imported factor")
            image_nodes = [node for node in mat.node_tree.nodes if node.type == 'TEX_IMAGE' and node.image]
            if len(image_nodes) != 1 or image_nodes[0].image.packed_file is None:
                raise AssertionError("Masked material did not import its embedded image")
            image = image_nodes[0].image
            if tuple(image.size) != (128, 128):
                raise AssertionError("Wrong embedded image dimensions")
            alphas = list(image.pixels)[3::4]
            if any(min(abs(alpha), abs(alpha-1)) > 0.0001 for alpha in alphas):
                raise AssertionError("Mask contains unexpected translucent coverage")
            if sum(alpha > 0.5 for alpha in alphas) != expected["opaquePixels"]:
                raise AssertionError("Imported coverage differs from original mask")
            if not shader.inputs["Alpha"].is_linked:
                raise AssertionError("Imported mask does not drive material alpha")
        else:
            assert_imported_color(obj, shader, expected["color"]+[1], expected["name"]+" color")
        if "uvMin" in expected:
            uv_layer = obj.data.uv_layers.active
            if uv_layer is None:
                raise AssertionError("UV coordinates were lost on import")
            uvs = [loop.uv for loop in uv_layer.data]
            low_uv = [min(uv[i] for uv in uvs) for i in range(2)]
            high_uv = [max(uv[i] for uv in uvs) for i in range(2)]
            assert_close(low_uv, [expected["uvMin"][0], 1-expected["uvMax"][1]], "UV minimum")
            assert_close(high_uv, [expected["uvMax"][0], 1-expected["uvMin"][1]], "UV maximum")
        assert_close((shader.inputs["Metallic"].default_value,), (0,), "metalness")
        assert_close((shader.inputs["Roughness"].default_value,), (0.8,), "roughness")
        if mat.use_backface_culling == expected.get("doubleSided", False):
            raise AssertionError(f"Wrong sidedness for {expected['name']}")
        obj.data.calc_loop_triangles()
        triangles += len(obj.data.loop_triangles)
        vertices += len(obj.data.vertices)
        if any(not math.isfinite(component) for point in points for component in point):
            raise AssertionError("Non-finite imported geometry")
    if source_triangles != case["summary"]["triangles"]:
        raise AssertionError(f"GLB source triangle count mismatch for {case['name']}: {source_triangles} != {case['summary']['triangles']}")
    if triangles > source_triangles or source_triangles - triangles > max(2, int(source_triangles * 0.005)):
        raise AssertionError(f"Imported topology changed beyond tolerance for {case['name']}: {triangles} != {source_triangles}")
    reports.append({"name":case["name"],"passed":True,"meshes":len(objects),"vertices":vertices,"sourceTriangles":source_triangles,"importedTriangles":triangles})
    if case["name"] == "house":
        render_house()

plane_case = manifest.get("imagePlaneFixture")
if plane_case:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    raw = (root / plane_case["file"]).read_bytes()
    json_length = int.from_bytes(raw[12:16], "little")
    source = json.loads(raw[20:20+json_length])
    if any("uri" in image for image in source.get("images", [])):
        raise AssertionError("Image-plane export depends on an external image")
    if source["extras"].get("scope") != "all visible strokes, image planes, and OBJ models":
        raise AssertionError("Mixed-resource fixture is not a whole-scene export")
    source_node = next(node for node in source["nodes"] if node["name"] == plane_case["name"])
    if str(source_node["extras"].get("spatialSketchImagePlaneId")) != plane_case["id"]:
        raise AssertionError("Image-plane ID metadata was lost in source GLB")
    if source_node["extras"].get("spatialSketchAsset") != plane_case["asset"]:
        raise AssertionError("Image-plane asset metadata was lost in source GLB")
    source_model_node = next(node for node in source["nodes"] if node["name"] == plane_case["modelName"])
    if str(source_model_node["extras"].get("spatialSketchObjReferenceId")) != plane_case["modelId"]:
        raise AssertionError("OBJ reference ID metadata was lost in source GLB")
    if source_model_node["extras"].get("spatialSketchAsset") != plane_case["modelAsset"]:
        raise AssertionError("OBJ managed asset metadata was lost in source GLB")
    ignored = source_model_node["extras"].get("spatialSketchIgnoredObjMaterialProperties", [])
    if not any("Ns" in warning for warning in ignored) or not any("map_Kd" in warning for warning in ignored):
        raise AssertionError("Unsupported OBJ material properties were not disclosed in GLB metadata")
    model_primitive = source["meshes"][source_model_node["mesh"]]["primitives"][0]
    model_material = source["materials"][model_primitive["material"]]
    assert_close(model_material["pbrMetallicRoughness"]["baseColorFactor"], plane_case["modelDiffuse"]+[plane_case["modelOpacity"]], "OBJ Kd and opacity")
    if model_material.get("alphaMode") != "BLEND" or model_material.get("doubleSided") is not True:
        raise AssertionError("OBJ material opacity or two-sided setting was not exported")
    source_material = source["materials"][source["meshes"][source_node["mesh"]]["primitives"][0]["material"]]
    if source_material.get("doubleSided") is not True:
        raise AssertionError("Image-plane material is not double-sided")
    if source_material.get("alphaMode") != "BLEND":
        raise AssertionError("Image-plane alpha is not represented as blended material")
    if abs(source_material["pbrMetallicRoughness"]["baseColorFactor"][3] - plane_case["opacity"]) > 0.0001:
        raise AssertionError("Image-plane opacity factor changed")
    embedded = source["images"][0]
    if "bufferView" not in embedded or embedded.get("mimeType") != "image/png":
        raise AssertionError("Image-plane texture is not embedded as PNG")
    bpy.ops.import_scene.gltf(filepath=str(root / plane_case["file"]))
    obj = bpy.data.objects.get(plane_case["name"])
    if obj is None or obj.type != 'MESH':
        raise AssertionError("Blender did not import the image-plane mesh")
    if str(obj.get("spatialSketchImagePlaneId")) != plane_case["id"]:
        raise AssertionError("Blender lost image-plane ID metadata")
    points = [obj.matrix_world @ vertex.co for vertex in obj.data.vertices]
    low = [min(point[i] for point in points) for i in range(3)]
    high = [max(point[i] for point in points) for i in range(3)]
    minimum, maximum = plane_case["min"], plane_case["max"]
    assert_close(low, (minimum[0], -maximum[2], minimum[1]), "image plane minimum", tolerance=0.00025)
    assert_close(high, (maximum[0], -minimum[2], maximum[1]), "image plane maximum", tolerance=0.00025)
    if len(obj.data.vertices) != 4 or len(obj.data.polygons) != 2:
        raise AssertionError("Image plane did not import as a two-triangle quad")
    uv = obj.data.uv_layers.active
    if uv is None:
        raise AssertionError("Image-plane UV coordinates were lost")
    assert_close([min(loop.uv[i] for loop in uv.data) for i in range(2)], [0.0, 0.0], "image-plane UV minimum")
    assert_close([max(loop.uv[i] for loop in uv.data) for i in range(2)], [1.0, 1.0], "image-plane UV maximum")
    mat = obj.data.materials[0]
    images = [node.image for node in mat.node_tree.nodes if node.type == 'TEX_IMAGE' and node.image]
    if len(images) != 1 or tuple(images[0].size) != tuple(plane_case["pixelSize"]):
        raise AssertionError("Blender image-plane texture is missing or has the wrong size")
    if images[0].packed_file is None:
        raise AssertionError("Blender image-plane texture was not packed with the GLB")
    alphas = list(images[0].pixels)[3::4]
    if not any(0.0 < alpha < 1.0 for alpha in alphas):
        raise AssertionError("Blender did not retain the image-plane texture's translucent pixels")
    model = bpy.data.objects.get(plane_case["modelName"])
    if model is None or model.type != 'MESH':
        raise AssertionError("Blender did not import the OBJ reference mesh")
    if str(model.get("spatialSketchObjReferenceId")) != plane_case["modelId"]:
        raise AssertionError("Blender lost OBJ reference ID metadata")
    if model.get("spatialSketchAsset") != plane_case["modelAsset"]:
        raise AssertionError("Blender lost OBJ bundle metadata")
    points = [model.matrix_world @ vertex.co for vertex in model.data.vertices]
    low = [min(point[i] for point in points) for i in range(3)]
    high = [max(point[i] for point in points) for i in range(3)]
    minimum, maximum = plane_case["modelMin"], plane_case["modelMax"]
    assert_close(low, (minimum[0], -maximum[2], minimum[1]), "OBJ minimum", tolerance=0.00025)
    assert_close(high, (maximum[0], -minimum[2], maximum[1]), "OBJ maximum", tolerance=0.00025)
    if len(model.data.vertices) != 3 or len(model.data.polygons) != 1:
        raise AssertionError("OBJ triangle topology changed during GLB import")
    assert_close(tuple(model.data.polygons[0].normal), (0.0, -1.0, 0.0), "OBJ transformed normal", tolerance=0.00025)
    uv_layer = model.data.uv_layers.active
    if uv_layer is None:
        raise AssertionError("OBJ texture coordinates were lost")
    assert_close([min(loop.uv[i] for loop in uv_layer.data) for i in range(2)], [0.0, 0.0], "OBJ UV minimum")
    assert_close([max(loop.uv[i] for loop in uv_layer.data) for i in range(2)], [1.0, 1.0], "OBJ UV maximum")
    if not model.data.materials:
        raise AssertionError("OBJ diffuse material is missing in Blender")
    shader = next((node for node in model.data.materials[0].node_tree.nodes if node.type == 'BSDF_PRINCIPLED'), None)
    if shader is None:
        raise AssertionError("OBJ material did not import as Principled BSDF")
    assert_close(tuple(shader.inputs["Base Color"].default_value)[:3], plane_case["modelDiffuse"], "OBJ imported diffuse color")
    assert_close([shader.inputs["Alpha"].default_value], [plane_case["modelOpacity"]], "OBJ imported material opacity")
    reports.append({"name":"mixed-scene","passed":True,"objects":3,"imageVertices":4,"modelVertices":3,"sourceTriangles":plane_case["summary"]["triangles"],"embeddedImage":list(images[0].size)})

(root / "blender-report.json").write_text(json.dumps({"passed":True,"blender":bpy.app.version_string,"checks":["mesh names and stroke IDs","Y-up to Z-up orientation","per-stroke dimensions and pressure-shaped bounds","original PBR color factors","ribbon double-sided material","embedded mask images and coverage","UV bounds","GLB source triangle counts","imported topology within 0.5%","finite geometry","image-plane ID and bounds","embedded image-plane PNG, UVs, and opacity","OBJ reference transforms and bounds","OBJ normals and triangle topology","MTL Kd diffuse color and opacity","ignored MTL property metadata"],"cases":reports},indent=2),encoding="utf-8")
print("Sketch3D GLB import verification passed", flush=True)
