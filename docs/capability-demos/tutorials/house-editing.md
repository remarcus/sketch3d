# Select, transform, and save a scene

**Status:** Static steps pair with the current-build 60-second [captioned capture](../media/house-tutorial.mp4) and [VTT captions](../media/house-tutorial.en.vtt). Review timing, playback, and readability before publication. The clip uses synthetic pressure-free input.

**Sample:** [Garden house](../fixtures/garden-house.sketch.json), generated from the built-in original fixture in `src/scenes.rs`.

**Draft timed captions:** [WebVTT sidecar](house-editing.en.vtt), planned for a 60-second tutorial. Check every cue against the final recording before publishing.

**Prerequisites:** Sketch3D; mouse or tablet. Mouse input uses constant pressure.

## Steps

1. Open the garden-house fixture and inspect the editable strokes in the canvas.
2. Press **B** and draw a short detail in open canvas above or beside the model.
3. Press **V** and select the new stroke. Use **G** and drag to move it; press **Ctrl/Cmd+Z** to undo and **Ctrl/Cmd+Shift+Z** to redo.
4. Orbit with right-mouse drag to inspect the moved stroke from another angle.
5. Save a copy with **Ctrl/Cmd+Shift+S**. Reopen the copy to confirm the strokes are still editable.

## Draft narration / caption transcript

This narration transcript follows the visible action order in the recording. Confirm cue timing and control visibility before publication.

- “Start from the editable garden-house scene.”
- “Draw a detail on the front plane.”
- “Select the new stroke and move it. Undo and redo to compare the geometry.”
- “Orbit to inspect the change in three dimensions.”
- “Save a separate copy, then reopen it to confirm the drawing remains editable.”

**Recorded native sample:** [house-tutorial-saved.sketch.json](../media/house-tutorial-saved.sketch.json), saved and reopened through the production handlers during capture. Manifest includes commit, viewport/DPI/theme, captions, output sizes, and review status.
