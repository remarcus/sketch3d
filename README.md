# Spatial Sketch

An open-source, native 3D sketching prototype inspired by the idea of drawing in space. Written in Rust, rendered with wgpu. Independent of Feather and Sketchsoft.

## Current prototype

- Pressure-sensitive tube strokes stored as editable 3D control points.
- Adjustable stroke stabilization with pressure preservation and endpoint catch-up.
- Front, side, and ground drawing planes, with adjustable depth.
- Orbit, pan, zoom, plane alignment, and camera reset.
- Windows Ink pressure, tilt, barrel-button and eraser diagnostics, including coalesced pen history.
- Mouse drawing with constant pressure, clearly distinguished from tablet input.
- Click/Shift-click selection, depth-aware picking, gold highlighting, and selection bounds.
- Move, rotate, and uniform-scale handles, numeric transforms, grouped duplication and deletion.
- Whole-stroke eraser, undo/redo, versioned JSON documents, atomic saves, and unsaved-change prompts.
- GPU depth testing and cached completed-stroke meshes. Only changed strokes are tessellated again.

**Windows is the first supported tablet target.** The application structure is portable; native macOS and Linux pen input is not implemented yet. Those platforms currently have mouse input only and have not been verified on physical hardware.

## Run

Install stable Rust using https://rustup.rs. On Windows, the MSVC Rust toolchain also needs Visual Studio Build Tools with the Desktop development with C++ workload and a Windows SDK.

```sh
cargo run --release
```

For a built-in 3D example:

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
| Align camera to current plane | F |
| Undo / redo | Ctrl/Cmd+Z / Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y |
| Save / save as | Ctrl/Cmd+S / Ctrl/Cmd+Shift+S |
| Open / new | Ctrl/Cmd+O / Ctrl/Cmd+N |

When Windows Ink reports a barrel button directly, barrel + contact drag also orbits. Driver button mappings may instead generate a mouse button or shortcut. Essential controls do not require touch. A stroke ends when it leaves the canvas. An edge-on drawing plane cannot accept strokes: align the view or orbit away from the edge.

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

This is an early prototype, not a production art application. Native tablet backends for macOS/Linux, Wintab, editable curved guides, partial erasing, surface snapping, glTF export and autosave remain future work. Active-stroke tessellation rebuilds that stroke on change; long strokes should move to incremental ring updates. GPU meshes currently use one draw call per stroke; batching and spatial indexing will matter for large documents. File dialogs and save/open currently run on the UI thread.

The immediate hardware test is pressure and tilt on the Wacom, followed by the Kamvas with mixed-DPI monitors. Low-latency behavior must be measured on those devices; no latency or frame-rate guarantee is claimed.

See [ARCHITECTURE.md](ARCHITECTURE.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

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
