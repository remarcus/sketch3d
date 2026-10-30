# Architecture

## Data flow

Native tablet messages -> chronological normalized samples -> ray/plane intersection -> editable stroke points -> tube tessellation -> cached vertex buffers -> depth-tested wgpu canvas -> egui texture.

## Modules

- `document.rs`: versioned file schema and validation. Does not depend on the UI or platform APIs.
- `geometry.rs`: camera, ray/plane intersection, grid, and pressure-aware tube meshes. Unit tested separately from graphics hardware.
- `input.rs`: normalized sample type and Windows Ink listener. Contains the platform-specific unsafe boundary.
- `renderer.rs`: custom WGSL pipeline, offscreen color/depth targets, camera uniform, grid and per-stroke GPU cache. Reallocates buffers geometrically; completed stroke buffers are reused.
- `editing.rs`: grouped snapshots, reversible edits, selection bounds, and validated transforms. Tests cover exact stroke order and attribute preservation.
- `gizmo.rs`: projection, depth-aware picking, axis/ray constraints, and screen-sized world-space handles.
- `selection.rs`: selection interface, gesture lifecycle, numeric transforms, grouped duplicate/delete, and software gesture verification.
- `app.rs`: tool state, canvas interaction, edit history, atomic saves, file dialogs, diagnostics, and interface.

## Tablet boundary

The Windows listener hooks the app's own window procedure on its UI thread and forwards all messages to eframe. It reads pointer history while handling each pointer message, since the history belongs to that message. History is reversed to chronological order before queueing. Screen coordinates are converted to client pixels, then to egui points using pixels_per_point. Focus/capture loss cancels an in-progress stroke. The previous window procedure is restored on drop if our hook is still installed.

Pressure is optional in the normalized sample. A pressure value of zero is valid, not a marker for missing input. A missing pressure value produces constant-width drawing while diagnostics continue to say not reported. Mouse-promoted pen events are temporarily suppressed on the canvas when native samples are arriving; egui still receives those events for tool widgets.

Future backends should preserve sample order, pressure availability, tilt availability, contact lifecycle, and cancellation. Avoid coupling the document format to Windows structures. Before broad tablet support, add pointer-ID/device routing and verify cancellation/capture behavior with actual devices. The prototype assumes one active pen.

## Rendering and performance

Positions remain in world space. The camera projection uses a depth range compatible with wgpu. A texture with a dedicated depth buffer is rendered before egui composes the interface. Stroke tubes have eight sides and pressure per ring. Camera movement changes only the uniform, not stored stroke geometry. Loading a document explicitly invalidates the mesh cache.

Current costs to address after profiling: complete active-stroke tessellation, scanning strokes for whole-stroke erasure, one draw call per stroke, synchronous document operations, and rendering on every requested UI frame. The diagnostics CPU duration measures update work; it is not end-to-end pen latency, GPU time, or display frame time.

## Persistence and history

The document schema contains only semantic drawing data, not GPU vertices. Edit history records added or removed strokes rather than copying the whole document. Undo history is session-local. The dirty indicator is conservative: undoing back to the saved state can still show unsaved changes.

## Scope

The application uses independent code and its own UI and document format. It does not read Feather's proprietary files. Native cross-platform pen support and package distribution require additional implementation and validation.

## Selection transactions

A gesture captures only selected strokes before the first drag. Every preview is recomputed from that original snapshot, preventing cumulative drift. Releasing the handle records one before/after edit; Escape or input focus loss restores the original snapshot without altering redo history. Group changes merge strokes in document order rather than repeatedly shifting the document vector. Undo and redo operate on the same edit representation as grouped delete and duplicate.

Selection highlighting is render-only. The renderer invalidates changed strokes when positions/radii change even if their point count is unchanged, and when selection membership changes. Cached buffer capacity is retained for these updates. The current picking method uses projected centerline segments plus brush width and depth; it is not exact triangle-level picking. Gizmos and bounds are interface overlays and remain visible through scene geometry.
