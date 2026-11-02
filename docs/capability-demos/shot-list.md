# Capability demo shot list

Record only after the associated task's checks pass. Suggested timings are edit lengths, not required capture speed. Keep cursor/input cues visible, avoid showing private paths, and pause briefly on each important control.

The [coverage audit](coverage-audit.md) distinguishes these intended shots from actions actually captured in the existing drafts and lists newer features awaiting acceptance before additional clips.

## Overview

**Name:** `sketch3d-overview` · **Target:** 18–22 seconds · **Fixture:** Japanese house or another licensed original fixture.

1. Show the clean workspace and editable house (2 seconds).
2. Draw a short brush stroke on the plane (4 seconds).
3. Orbit to show the stroke's 3D depth (4 seconds).
4. Select and move the stroke, then undo (5 seconds).
5. Return to the complete scene and show the export menu (3 seconds).

## Focus clips (6–15 seconds each)

| Clip | Feature/task | Steps | Static alternative |
| --- | --- | --- | --- |
| `draw-orbit` | Plane drawing and camera orbit (SKETCH-001/021) | Draw; orbit; pause on the 3D result. | Written Controls section and a reviewed still. |
| `curved-guides` | Cylinder/ring guides (SKETCH-001/063) | Activate guide; draw across a periodic seam; orbit. | Guide documentation and a static wireframe image. |
| `mirror-shapes` | Live mirror and editable ellipse (SKETCH-038/039/062) | Enable mirror; draw a line; place an ellipse on the front guide. The deterministic replay prepares the reflected line and ellipse through production handlers before capture, then records the orbit. Circle and image-plane drawing are separate verified capabilities, but are not shown in this clip. | Tool-specific README steps and fixture. |
| `edit-erase` | Selection, appearance, and partial erasing (SKETCH-035–037) | Select; change appearance; erase a segment; undo. The deterministic replay prepares the editable mark before capture, then performs the partial erase and undo through production handlers. | Keyboard/control table and before/after stills. |
| `reference-export` | Reference/model import and export (SKETCH-033/044/059/061/068) | Before capture, generate and import a reference image and OBJ through production asset/render paths, insert the saved lantern through native merge, and export the combined scene through the production GLB exporter; then record a synthetic orbit of the staged result. | Import/export docs and sample files. |

If a listed feature has not passed its interaction/persistence checks, omit it or use the static alternative. Never substitute synthetic pressure samples for tablet evidence.

## Tutorial captures (45–90 seconds each)

### Explore and edit a curved drawing

Prerequisites: verified curved-guide interaction, selection, undo/redo, and save flow. Use the [cylindrical lantern fixture](fixtures/cylindrical-lantern.sketch.json).

1. Open the fixture and briefly explain that its strokes are editable.
2. Orbit around the lantern and pause on its curved strokes.
3. Select one stroke, move it, then undo and redo.
4. Orbit to inspect the result and save a separate copy.

### Select, transform, and save a scene

Prerequisites: verified selection, transform, undo/redo, drawing, and save/reload workflows. Use the [garden-house fixture](fixtures/garden-house.sketch.json).

1. Open the fixture and explain that its strokes are editable.
2. Select and move a roof or wall stroke; undo and redo the change.
3. Orbit to inspect the scene from another angle.
4. Draw a short detail on the current plane; align the view if needed.
5. Save a separate copy, reopen it, and confirm that its strokes remain editable.

Static text alternatives and draft narration are in [tutorials/curved-drawing.md](tutorials/curved-drawing.md) and [tutorials/house-editing.md](tutorials/house-editing.md). Captions must name each shortcut as it appears. Include a reviewed poster, descriptive alt text, prerequisites, sample-document link, feature/task links, app commit, fixture version, theme, viewport/DPI, and the manual/replay capture method in each manifest. The `tutorials` section stays unlisted until both captures are reviewed.
