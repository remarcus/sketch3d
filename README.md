# Spatial Sketch

An open-source, native 3D sketching prototype inspired by the idea of drawing in space. Written in Rust, rendered with wgpu. Independent of Feather and Sketchsoft.

## Current prototype

- Editable Japanese-style house and garden shown on startup.
- Pressure-sensitive tube strokes stored as editable 3D control points.
- Adjustable stroke stabilization with pressure preservation and endpoint catch-up.
- Freely oriented drawing planes with X/Y/Z rotation, anchor positioning, normal offset, and world-plane presets.
- Cylindrical side guides with adjustable placement, rotation, radius, height, and visibility, plus an editable lantern study.
- Orbit, pan, zoom, plane alignment, and camera reset.
- Windows Ink pressure, tilt, barrel-button and eraser diagnostics, including coalesced pen history.
- Mouse drawing with constant pressure, clearly distinguished from tablet input.
- Click/Shift-click selection, depth-aware picking, gold highlighting, and selection bounds.
- Move, rotate, and uniform-scale handles, numeric transforms, grouped duplication and deletion.
- Whole-stroke eraser, undo/redo, versioned JSON documents, atomic saves, and unsaved-change prompts.
- Background GLB export for the whole drawing or selected strokes, preserving original colors and pressure-shaped geometry.
- GPU depth testing and cached completed-stroke meshes. Only changed strokes are tessellated again.

**Windows is the first supported tablet target.** The application structure is portable; native macOS and Linux pen input is not implemented yet. Those platforms currently have mouse input only and have not been verified on physical hardware.

## Run

Install stable Rust using https://rustup.rs. On Windows, the MSVC Rust toolchain also needs Visual Studio Build Tools with the Desktop development with C++ workload and a Windows SDK.

```sh
cargo run --release
```

The app opens with an original Japanese-style garden house drawn entirely from editable strokes. **New** starts a blank drawing; **House** reloads the sample, with an unsaved-change prompt when needed. For a blank startup, use `cargo run --release -- --blank`.

For the original three-ring geometry example:

```sh
cargo run --release -- --demo
```

The Windows executable is `target/release/spatial-sketch.exe`. The first build downloads dependencies and can take several minutes.

On Debian/Ubuntu, install development packages needed by winit, Wayland/X11, and the file-dialog backend, such as `build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev libvulkan-dev libdbus-1-dev`. On macOS, install Xcode Command Line Tools. Platform setup remains to be validated by CI and real-device testing.

## Tablet setup

Use the manufacturer's tablet driver. Enable **Windows Ink** in the Wacom or Huion application profile. Map your tablet to the intended monitor, retaining the correct aspect ratio. The app receives driver-mapped coordinates; it does not implement its own hardware driver or tablet calibration.

Draw in the central canvas. The right panel reports actual pen samples, pressure, and tilt. If you can draw but the panel still says WAITING FOR PEN, the device is currently arriving as mouse input: pressure is not being received. Unsupported tilt is shown as not reported.

The first hardware targets are a screenless Wacom tablet and a a pen display. **Neither has been physically validated by this repository yet.** Wintab fallback remains future work.

## Controls

| Action | Input |
| --- | --- |
| Brush / whole-stroke eraser / select | B / E / V |
| Add/remove stroke from selection | Shift-click in Select tool |
| Select all / clear selection | Ctrl/Cmd+A / Escape |
| Move / rotate / uniform scale | G / R / S in Select tool |
| Duplicate selection | Ctrl/Cmd+D |
| Delete selection | Delete / Backspace |
| Cancel transform gesture | Escape |
| Orbit | Space + pen/mouse drag, or right mouse drag |
| Pan | Space + Shift + drag, or middle mouse drag |
| Zoom | Space + Alt + drag, or wheel |
| Align camera to current guide | F |
| Create drawing plane from view | Shift+F |
| Undo / redo | Ctrl/Cmd+Z / Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y |
| Save / save as | Ctrl/Cmd+S / Ctrl/Cmd+Shift+S |
| Export whole drawing / selection as GLB | Export menu |
| Open / new | Ctrl/Cmd+O / Ctrl/Cmd+N |
| Reload house starter / lantern study | House / Lantern buttons |

When Windows Ink reports a barrel button directly, barrel + contact drag also orbits. Driver button mappings may instead generate a mouse button or shortcut. Essential controls do not require touch. A stroke ends when it leaves the canvas. An edge-on drawing plane cannot accept strokes: press F to align the view or orbit away from the edge.

Brush radius is in world units, so its visible size changes with camera distance. The pressure curve maps input pressure before tessellation. Existing strokes preserve their original color, width, and pressure samples.

## Documents

Documents use a versioned `.sketch.json` format with positions in world space. Open validates version, IDs, positions, pressure, color, and size limits. Saves write to a temporary file in the destination directory and replace the destination after syncing. Errors appear in the bottom status bar. There is no autosave or crash recovery yet; save regularly.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

`Cargo.lock` is checked in for reproducible application builds. The initial egui/wgpu integration uses the 0.31/24 API family; framework upgrades should be separate, tested changes.

## Boundaries and next steps

This is an early prototype, not a production art application. Native tablet backends for macOS/Linux, Wintab, additional curved guides, partial erasing, surface snapping, and autosave remain future work. Active-stroke tessellation rebuilds that stroke on change; long strokes should move to incremental ring updates. GPU meshes currently use one draw call per stroke; batching and spatial indexing will matter for large documents. File dialogs and save/open currently run on the UI thread.

The immediate hardware test is pressure and tilt on the Wacom, followed by the Kamvas with mixed-DPI monitors. Low-latency behavior must be measured on those devices; no latency or frame-rate guarantee is claimed.

See [TASKS.md](TASKS.md) for the local feature backlog, [ARCHITECTURE.md](ARCHITECTURE.md) for implementation notes, and [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidance.

## License

GPL-3.0-or-later. See LICENSE. Dependencies retain their own licenses.

## Software smoke verification

```sh
cargo run --release -- --verify
```

This opens the app, replays synthetic mouse events through the canvas handler, checks undo/redo, erasing, Shift multi-selection, move/rotate/scale handle gestures, cancellation, grouped duplication/deletion, and numeric transforms, saves and reloads a document (including replacing an existing save), renders a 3D example, and captures the app's own screenshot. It then exits. Outputs are `verification/report.json` and `verification/preview.png`. This verifies software paths and GPU initialization; it does not simulate or validate physical tablet hardware.

## Selection and transforms

Press **V**, then click a stroke. Shift-click adds or removes strokes. Picking considers screen distance, brush width, and depth; overlapping strokes prefer the nearer stroke rather than the last one drawn. Click empty canvas to clear selection. Selection is highlighted in gold without changing the saved ink color.

- **Move (G):** drag a red X, green Y, or blue Z arrow for an axis-constrained move. Drag the center square to move in the camera's view plane.
- **Rotate (R):** drag a colored ring around its world axis. Rings seen edge-on are hidden; orbit the camera to expose them.
- **Scale (S):** drag the gold square to scale uniformly around the group's center. Brush radii scale too; pressure samples remain unchanged.
- **Numeric values:** enter world offsets, rotation degrees, or a scale factor in the selection panel, then choose Apply values. Rotation applies X, then Y, then Z around the shared pivot.
- **Duplicate:** makes offset copies and selects those copies. Delete removes the whole selected group. Each command is one undo step.

Each handle gesture uses the selection's bounding-box center as its pivot and becomes one undo step on release. Escape or focus loss cancels the active gesture and restores its original geometry. Controls continue to work with pen displays and screenless tablets; touch is not required. Scale is limited by the document's supported brush radius range (0.001–1 world units), and transforms cannot move points outside the document bounds.

Selection is temporary interface state, so it is not saved in the drawing or restored by undo. Undo/redo restores geometry, widths, stroke order, and IDs. Transforming geometry does not change point counts; the renderer explicitly refreshes affected cached buffers.

## Stroke stabilization

The brush panel's **Stabilize** slider controls a spatial trailing filter. At 0%, positions are recorded directly. Higher values reduce hand jitter but let the line trail the pen; 35% is the initial setting. The radius is derived from screen pixels, so zooming does not radically change the feel. Pressure is passed through immediately rather than smoothed.

On release, the stroke catches up to the final contact position with a short interpolated tail. Pen-up pressure is not treated as a new ink sample. The tail belongs to the same undo step as the stroke. Cancellation discards filter state, and a new stroke starts exactly at its first contact point. This is a position stabilizer, not curve fitting or a post-process on existing strokes.

## Japanese garden house

The startup scene is original procedural artwork, constructed from ordinary document strokes: curved tiled roof runs, timber framing, shoji-style screens, a raised veranda, stepping stones, a flowering tree, garden shrubs, and lanterns. No external artwork or network request is needed. Every component supports the existing selection, transform, erase, undo, and save tools. Selection and editing still operate per stroke; this is not a separate imported mesh or a grouped CAD model.

The unmodified sample is treated as a fresh example, so simply closing it does not ask you to save. Editing makes it an unsaved drawing. New and House respect that unsaved state. New clears the sample and resets the camera and guide for drawing; House restores the sample and its presentation camera. `--blank` bypasses the sample on startup, while `--demo` opens the original ring example.

```sh
cargo run --release -- --verify-house
```

This checks sample validity, duplication/undo, New/House switching, the unsaved-change guard, save/reload, and GPU rendering. It writes `verification/house-report.json` and `verification/house-preview.png`, then exits. The original `--verify` still exercises the drawing, selection, transform, and stabilization workflows independently of the startup scene.

## Freely oriented drawing planes

The Drawing plane panel supports more than the Front, Side, and Ground presets:

- **Rotation X/Y/Z** sets the plane orientation in degrees, applied X then Y then Z.
- **Anchor X/Y/Z** positions its local origin in world units. **Offset** shifts the plane along its rotated normal relative to that anchor.
- **Plane from view (Shift+F)** creates a plane through the camera's focus point, with its axes aligned to the current camera view, including roll.
- **Center on selection** places the anchor at the selected strokes' bounding-box center while retaining the current plane orientation, and clears Offset.
- **Align view to plane (F)** centers the camera on the plane origin and aligns the camera axes to the plane. Camera distance is preserved.

World-plane preset buttons reset both orientation and placement to that world plane. Changing any guide setting finishes an active stroke first; existing artwork stays fixed in world space. The cyan **N** arrow marks the normal on custom planes. The grid is a background guide and does not cut through ink. Duplication offsets follow the current plane's local horizontal axis.

For a sloped detail, orbit to a useful angle, press Shift+F, position the plane with Anchor/Offset (or Center on selection), then press F and draw. Plane controls are temporary workspace state and are not saved with the drawing; the resulting 3D strokes retain their positions when saved and reopened. Additional curved guide types and automatic surface snapping remain future work.

## Cylindrical drawing guide — SKETCH-001

Choose **Cylinder** in the Drawing guide panel. Set its center, rotation, radius, and height. **F** aligns the camera to its local front and keeps the camera outside the guide; **Cylinder from view** places it at the camera focus with its axis vertical in that view. Center on selection uses the selected strokes' bounds. **Shift+F** still switches to a plane created from the view.

Draw on the cylinder's **side surface**. Orbit to reach its back. The guide is an open cylinder: caps are not drawing surfaces. Pen/mouse rays choose the nearest forward side hit within the cylinder's height. Missing the side ends that stroke segment; re-entering during the same contact starts a new segment. Angular jumps larger than 90 degrees between contact samples also split ink instead of bridging unrelated sides. Each segment has its own undo step.

Stabilization works in unwrapped angular arc-length and height coordinates. This keeps seam crossings continuous and maps smoothed samples and release tails back onto the curved surface. Sparse samples are subdivided along the arc rather than connected by a large straight chord. Stored ink remains ordinary editable world-space stroke points; the guide is only used while drawing.

**Show guide** controls preview visibility only; drawing still follows the selected surface while the wireframe is hidden. Changing or resizing a guide finishes active ink on its captured original surface. Existing strokes do not move when a guide is hidden, resized, rotated, or replaced. Plane and cylinder settings remain temporary workspace state, not part of the saved document.

The **Lantern** button opens an original editable cylindrical study, with the same unsaved-change protection as New and House. The house remains the default startup scene. To open the study directly:

```sh
cargo run --release -- --lantern
```

Cylinder verification runs separately from the existing drawing and house checks:

```sh
cargo run --release -- --verify-cylinder
```

It exercises surface-bound drawing and release tails, misses and large side jumps, the angular seam, rotated guides, captured-surface completion, guide visibility/switching, duplication/deletion with undo, native document save/reload, and GPU rendering. Outputs are `verification/cylinder-report.json` and `verification/cylinder-preview.png`. Physical tablet testing remains a separate hardware task.

## GLB export — SKETCH-002

Use **Export → Whole drawing (.glb)** or **Export → Selected strokes (.glb)**. Choose a destination ending in `.glb` in the native save dialog. Selection export is available when drawable strokes are selected. Finish an active stroke or transform before exporting.

Export takes a snapshot and builds the file in a background worker. You can continue drawing; edits made after that snapshot belong to a later export. The status bar reports completion or an error. Let an active export finish before closing the app.

The result is a self-contained **binary glTF 2.0** file with indexed triangle meshes, outward-facing triangles, normals, and opaque PBR materials. Each drawable stroke becomes a named mesh object. Colors come from the original stroke data; pressure is baked into mesh thickness. Selection highlights, drawing guides, handles, cameras, and interface geometry are excluded. No external textures or binary sidecar files are required.

One drawing world unit exports as **one meter**, with right-handed +X right, +Y up, and +Z front coordinates. Blender converts this to its +Z-up convention on import. Material factors preserve the drawing colors; lighting and tone mapping in other viewers can change their rendered appearance.

**Keep the native `.sketch.json` drawing too.** GLB contains meshes, not editable brush control points, pressure input, or undo history. Export does not mark the native drawing as saved or change its file identity. The UI rejects destinations resolving to the open native drawing. Output replaces an existing destination only after the completed file has been written and synced; failed export leaves existing files intact. Empty scopes are rejected. This prototype limits binary geometry to 128 MiB and rejects individual strokes whose working meshes would exceed its per-stroke export budget.

### Independent verification

```sh
cargo run --release -- --verify-export
```

This creates `verification/exports/house.glb`, `lantern.glb`, `pressure.glb`, `selection.glb`, and a source-geometry manifest. The normal `--verify` workflow also exercises the asynchronous UI path and verifies that source geometry, file identity, dirty state, selection, and undo/redo depths remain unchanged.

To validate fixtures using the official Khronos validator, from the project root:

```sh
npm install --prefix verification/gltf-validator --no-save --package-lock=false --ignore-scripts gltf-validator@2.0.0-dev.3.10
node scripts/verify_glb.cjs verification/exports
```

To check imports in an installed Blender (put `blender` on PATH or use its executable path):

```sh
blender --background --factory-startup --python-exit-code 1 --python scripts/verify_glb_blender.py -- verification/exports
```

The Blender check verifies stroke IDs, mesh counts, converted axes, per-stroke bounds, original color factors, and triangle counts. It also renders the imported house to `verification/exports/house-blender.png` and saves a review scene. Verification cameras, lighting, and floor are added by that script after geometry checks; they are not part of the GLB export. Node and Blender are development verification tools, not application runtime requirements. The verification directory is ignored by Git.

Format reference: [Khronos glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html).
