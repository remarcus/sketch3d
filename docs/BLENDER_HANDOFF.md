# Spatial Sketch Blender handoff

The handoff is an additive workflow around the app's self-contained GLB export. It does not change the native drawing format or promise an editable reconstruction of every brush.

## Install and use

The script targets Blender 4.3 or newer and has been run-verified with Blender 5.2.3 LTS. It uses Blender's bundled glTF 2.0 importer and Python standard library; no add-on installation is needed. From the repository root:

```powershell
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' --background --factory-startup --python scripts/sketch3d_blender_handoff.py -- --manifest drawing.handoff.json drawing.glb
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' --background --factory-startup --python scripts/sketch3d_blender_handoff.py -- --import drawing.handoff.json
```

Export the `.glb` from Sketch3D first. Keep the manifest beside the GLB when moving the handoff. The manifest stores a relative source filename and SHA-256; import rejects a missing, changed, or over-budget source. Textures and mesh buffers must be embedded in the GLB. External GLTF buffer or image URIs are rejected before Blender import. Asset identifiers retained as metadata (for example, an image-plane resource path) are provenance labels; the rendered payload is the embedded GLB image/mesh, so no machine-specific app asset directory is needed to display the handoff.

## Manifest v1

`spatial-sketch-blender-handoff` version `1` contains:

- `source.file` (manifest-relative GLB path) and `source.sha256`.
- `units.length = "meter"` and the source `unitInMeters` value.
- Source and Blender coordinate labels and the conversion `(x, y, z) -> (x, -z, y)`. Blender's glTF importer applies the glTF Y-up to Blender Z-up conversion; the organizer does not rotate the imported objects a second time.
- `collections.layers`, `collections.groups`, `materials`, and `strokes`. The material table records GLB material IDs/names, base color factor, alpha mode, and sidedness. Each stroke records source ID/name, layer/group IDs, material IDs/names, layer name, and the source GLB `extras` object for bounded JSON-sized values.
- `assets`, which records source asset identifiers from node extras. Displayable image/model payloads are embedded in the GLB.

Blender meshes keep the ordinary GLB materials and vertex color/texture payload. Each stroke object receives `sketch3d_stroke_id`, `sketch3d_manifest_version`, and bounded `sketch3d_*` custom properties copied from Sketch3D extras. Layer and group collections are keyed by source IDs. A stroke is linked to its layer collection and, when present, its group collection. A mesh can belong to both collections without changing its transform. Non-stroke image-plane and OBJ geometry remains in the handoff root collection.

## Representation and fallbacks

| Sketch3D feature | Blender handoff representation | Limit |
| --- | --- | --- |
| Stroke geometry | Imported GLB mesh, one source stroke per mesh node | Brush is not a live Blender brush/curve |
| Shaded, unlit, emissive, cutout material | GLB Principled material plus `spatialSketchMaterial` metadata | Unlit distinction depends on Blender's glTF importer; inspect material nodes |
| Brush dynamics and procedural texture | Retained only where already baked into GLB geometry, color, or embedded texture | Dynamics/texture authoring behavior is not reconstructed |
| Gradient | Baked GLB vertex color, plus gradient stop/interpolation extras on stroke object | Not a live gradient control; emissive gradients have uniform emission fallback |
| Cutout | Fully transparent geometry and compositing fallback metadata | Does not erase destination strokes in Blender |
| Layer opacity and blend | Layer name/ID collection and metadata | GLB uses normal blend fallback; collection opacity is not composited |
| Linked group | Group ID collection, with each baked instance mesh preserved | No live linked/instanced definition; geometry is independent |
| Image plane / referenced OBJ | Embedded GLB mesh and texture, with source resource extras | Original editable source asset is not reconstructed |

These limitations match the current GLB export contract. Do not interpret the manifest as declaring lossless parity.

## Bounds and validation

The importer accepts GLB 2 only, caps the source at 512 MiB, the JSON chunk and manifest at 64 MiB, nodes/meshes/materials/images/buffers at 100,000 each, strokes at 50,000, node names at 1,024 characters, asset identifiers at 4,096 characters, and copied nested extras at 16 KiB per property. Unsupported schema/version, malformed JSON/header, external URI dependencies, absent or changed source files, and invalid stroke records fail before organizing imported data. SHA-256 is checked before import. The source file is read and parsed before Blender's GLB importer is invoked.

Run the focused background verification:

```powershell
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' --background --factory-startup --python scripts/test_sketch3d_blender_handoff.py -- verification/exports
```

The report is written to `verification/exports/blender-handoff-report.json`. Existing `scripts/verify_glb_blender.py` independently checks source geometry/material/image details and renders the house GLB. The handoff harness imports all 12 original stroke fixtures plus the mixed image-plane/OBJ scene, and renders the house GLB directly and after collection organization using the same low-resolution Blender scene. The two PNG renders are saved beside the report and compared by channel values. This demonstrates that the organization step itself does not materially change the render. It does not establish pixel parity with Sketch3D or brush parity.
