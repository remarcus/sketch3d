# Capability demo shot list

Record only after the associated task's checks pass. Suggested timings are edit lengths, not required capture speed. Keep cursor/input cues visible, avoid showing private paths, and pause briefly on each important control.

The hidden current-build replays below use production handlers for featured actions and synthetic pressure-free input. Review full MP4 playback and captions before publication; manifests are deliberately unsigned until that review.

## Overview

**Name:** `sketch3d-overview` · **Target:** 18–22 seconds · **Fixture:** Japanese house or another licensed original fixture.

1. Show the editable garden-house scene (1 second).
2. Draw a short detail in the open canvas area (7 seconds).
3. Select and move the detail, then undo and redo (2 seconds).
4. Orbit to show the detail and scene depth (6 seconds).
5. Show the production GLB export options (2 seconds).

## Focus clips (6–15 seconds each)

| Clip | Feature/task | Steps | Static alternative |
| --- | --- | --- | --- |
| `draw-orbit` | Plane drawing and camera orbit (SKETCH-001/021) | Draw; orbit; pause on the 3D result. | Written Controls section and a reviewed still. |
| `curved-guides` | Cylinder guide and mirrored drawing (SKETCH-001/063) | Create a saved cylinder; draw across it through the production pointer handler; inspect source/reflected strokes; orbit. Ring-guide drawing remains outside this clip. | Guide documentation and native sample. |
| `mirror-shapes` | Live mirror and editable ellipse (SKETCH-038/039/062) | Enable mirror; draw a line; place an ellipse through production handlers; orbit. Circle and image-plane drawing are separate capabilities. | Tool-specific README steps and fixture. |
| `edit-erase` | Selection, appearance, and partial erasing (SKETCH-035–037) | Select and recolor during capture; erase a segment; inspect two fragments; undo and orbit. | Keyboard/control table and before/after stills. |
| `reference-export` | Reference/model import and export (SKETCH-033/044/059/061/068) | During capture, run the production OBJ Import button with an injected generated-file result, insert the saved lantern through production native merge, and export the combined scene through the whole-scene GLB worker. The native picker and insert preview are not shown. | Import/export docs and sample files. |

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
