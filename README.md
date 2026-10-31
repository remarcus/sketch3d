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
- Whole-stroke and partial-stroke erasers, undo/redo, versioned JSON documents, atomic saves, and unsaved-change prompts.
- A local reference image board for bounded PNG/JPEG imports, panning, zooming, color sampling, and image ordering.
- Transformable PNG/JPEG image planes with opacity, visibility, locking, active-plane brush drawing, native sidecar assets, undo/redo, and recovery.
- Selected-stroke appearance previews for color, radius, opacity, and brush type, with explicit Apply/Cancel and one-step undo.
- Drag brush selection and deselection with adjustable radius and visible-only or through-selection modes.
- Planar circle and ellipse tools with center/radius, bounding-box, and typed-dimension workflows.
- A planar smooth-curve tool with editable anchors/tangents, pressure-aware preview, and one-step commit.
- Named, saved plane/cylinder/revolved/sphere/cone guides with one active brush target and surface-aware stabilization.
- Selected-stroke Liquify previews with push/pull falloff controls and atomic undo.
- Saved camera bookmarks and timed view sequences in separate local workspace settings.
- Animated GIF export for camera turntables and saved sequences.
- Live plane-guide mirroring for brush and straight-line strokes, committed as one undoable pair.
- Background GLB export for the whole drawing or selected strokes, preserving original colors and pressure-shaped geometry.
- Round, ribbon, and flat/chisel brushes with pressure-shaped width, frozen orientation, and an editable brush sampler.
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
| Partial-stroke eraser | Erase inspector: enable Partial stroke eraser, then sweep over the canvas |
| Add/remove stroke from selection | Shift-click in Select tool |
| Brush select/deselect strokes | Select inspector: enable Drag brush selection, choose mode, and sweep the canvas |
| Edit selected stroke appearance | Select inspector: Edit appearance… |
| Draw circles / ellipses | Choose Circle or Ellipse; drag on a plane or enable typed dimensions |
| Add a scene image plane | Images inspector → Import image plane…; enable Draw here to ink on it |
| Save or activate a guide | Drawing guide → Saved guides; use the guide toggle to set the active brush surface |
| Draw a smooth curve | Choose Curve; add anchors and adjust tangent handles; Enter finishes, Escape cancels |
| Deform selected strokes | Select strokes → Liquify selected strokes…; adjust the brush center/displacement, then Apply or Cancel |
| Save/play camera views | View → Camera bookmarks and sequences… |
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

Documents use a versioned `.sketch.json` format with positions in world space. Version 10 adds saved guide resources; version 9 added image-plane records whose source images are stored in a sibling `<drawing filename>.assets` folder. Version 8 adds per-brush opacity, pressure-to-opacity mapping, and soft-edge settings; version 7 added editable planar fills with a physical paint offset, alongside flat groups and named layers. Versions 1–9 migrate while preserving their existing ink; versions 1–4 receive one visible, unlocked layer. Older application versions cannot open version-10 files. Open validates version, IDs, positions, pressure, color, image metadata, guide geometry, and size limits. Saves write to a temporary file in the destination directory and replace the destination after syncing. Errors appear in the bottom status bar. Recovery snapshots protect completed edits and include bounded image assets; use Save for your deliberate native drawing file. PNG renders include scene image planes. GLB export contains stroke geometry only. Camera bookmarks/sequences live in a separate local workspace file and do not travel with the drawing or recovery snapshot. Saved guides and image planes currently target brush input; switch to the temporary guide for line, fill, shape, and curve tools.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

`Cargo.lock` is checked in for reproducible application builds. The initial egui/wgpu integration uses the 0.31/24 API family; framework upgrades should be separate, tested changes.

## Boundaries and next steps

This is an early prototype, not a production art application. Native tablet backends for macOS/Linux, Wintab, additional curved guides, partial erasing and surface snapping remain future work. Active-stroke tessellation rebuilds that stroke on change; long strokes should move to incremental ring updates. GPU meshes currently use one draw call per stroke; batching and spatial indexing will matter for large documents. File dialogs and save/open currently run on the UI thread.

The immediate hardware test is pressure and tilt on the Wacom, followed by the Kamvas with mixed-DPI monitors. Low-latency behavior must be measured on those devices; no latency or frame-rate guarantee is claimed.

See [TASKS.md](TASKS.md) for the local feature backlog, [ARCHITECTURE.md](ARCHITECTURE.md) for implementation notes, and [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidance.

Initial implementations are in place for the reference board, selected-stroke appearance, drag selection, partial erasing, circle/ellipse tools, and plane-guide mirroring (SKETCH-033 and SKETCH-035–039). See TASKS.md for remaining interaction, persistence, and edge-case checks and the next features through SKETCH-055. Manual display-scaling/Narrator checks (SKETCH-032) and physical tablet validation (SKETCH-006) also remain open. Thin-line rendering, soft brushes, brush-package importing, and the benchmark pass are complete.

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

This creates `verification/exports/house.glb`, `lantern.glb`, `pressure.glb`, `brushes.glb`, `dynamics.glb`, `textures.glb`, `selection.glb`, and a source-geometry manifest. The normal `--verify` workflow also exercises the asynchronous UI path and verifies that source geometry, file identity, dirty state, selection, and undo/redo depths remain unchanged.

To validate fixtures using the official Khronos validator, from the project root:

```sh
npm install --prefix verification/gltf-validator --no-save --package-lock=false --ignore-scripts gltf-validator@2.0.0-dev.3.10
node scripts/verify_glb.cjs verification/exports
```

To check imports in an installed Blender (put `blender` on PATH or use its executable path):

```sh
blender --background --factory-startup --python-exit-code 1 --python scripts/verify_glb_blender.py -- verification/exports
```

The Blender check verifies stroke IDs, mesh counts, converted axes, per-stroke bounds, original color factors, double-sided ribbon materials, and triangle counts. It also renders the imported house to `verification/exports/house-blender.png` and saves a review scene. Verification cameras, lighting, and floor are added by that script after geometry checks; they are not part of the GLB export. Node and Blender are development verification tools, not application runtime requirements. The verification directory is ignored by Git.

Format reference: [Khronos glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html).

## Brush shapes — SKETCH-009

Choose **Round**, **Ribbon**, or **Flat / chisel** in the brush panel. Round preserves the original tube profile. Ribbon makes a thin strip visible from either side. Flat/chisel makes a solid rectangular strip; its **Thickness** setting is a percentage of the full width. **Radius / Half width** controls world-space size. All three support configurable pressure width; chisel thickness has an independent pressure response. Built-in masked textures are described below.

**Angle** rotates ribbon/chisel cross-sections around the stroke direction. The starting frame comes from the drawing plane or the cylinder surface at first contact. It is transported smoothly along the path; it does not face the camera, and changing the guide or active brush leaves existing strokes fixed. Transport along a complex 3D curve can twist relative to the guide surface. Taps produce a finite patch or solid stamp. Ribbons have no physical thickness; they appear edge-on when viewed along their surface.

**Brushes** opens an editable sampler: round at the top, ribbon in the middle, and chisel at the bottom, with pressure ramps and dots. Unsaved-change handling matches House and Lantern. Start with `cargo run --release -- --brushes`, or run `cargo run --release -- --verify-brushes` for drawing, selection, erasing, duplication, transforms, cylinder stabilization, save/reload, and GPU checks. Outputs are `verification/brush-report.json` and `verification/brush-preview.png`.

Native files preserve brush type, thickness ratio, angle, and orientation. Duplication and undo retain these values; rotation and uniform scale transform the whole brush mark. GLB uses the same meshes as the viewport, with double-sided ribbon materials. Physical pen pressure still requires tablet validation; optional tilt orientation is available as described below.

## Pressure, tilt, and taper — SKETCH-010

Expand **Pressure, tilt & taper** in the brush panel. Enable **Pressure: width** to map pressure to width; chisel also offers **Pressure: thickness** independently. Each mapping has a minimum fraction and response exponent: 1 is linear, larger values need more pressure, smaller values reach full size sooner. Disable a mapping for a constant dimension. Defaults preserve the original 12% minimum and linear response. A zero minimum can produce invisible geometry at zero contact pressure.

**Tilt orientation** uses reported pen lean to rotate ribbon/chisel cross-sections, with **Angle** as an additional rotation. Missing tilt, an upright pen, or lean parallel to the path uses the manual frame. Orientation is captured in world space, including camera roll, and transforms with the stroke. This is orientation control; tilt does not yet drive opacity or width. Physical Wacom/Huion tilt behavior remains unverified.

**Start taper / End taper** set world-space distances independently. Both dimensions follow the taper; overlapping tapers reduce the middle of short strokes, while dots retain their footprint. The profile uses path distance rather than event count, and the mesh inserts taper landmarks for sparse strokes. Tips retain a 1% taper floor to avoid collapsing otherwise nonzero profiles. End taper previews while drawing and settles after the stabilized release tail. Scaling a stroke scales its taper lengths too.

New strokes retain raw pressure, optional normalized pen lean, and a copy of all dynamics settings. Changing controls affects future strokes. Pen-up pressure/tilt is excluded, cancellation discards the unfinished stroke, and each completed stroke remains one undo step. Existing v1/v2 files retain their baked response through migration. GLB bakes the same dimensions, orientation, and taper as the viewport.

**Dynamics** opens an editable four-row study: pressure width, independent pressure thickness, tilt ribbon, and distance taper. Start with `cargo run --release -- --dynamics`. Run `cargo run --release -- --verify-dynamics` for synthetic pen contact/release/cancellation, missing-tilt fallback, persistence, and GPU verification; outputs are `verification/dynamics-report.json`, `verification/dynamics-preview.png`, `verification/opacity-report.json`, and `verification/opacity-overlap.png`. The opacity verifier renders eight overlapping marks into a 512×512 target, reverses document order, orbits the camera, and checks opaque occlusion. Its five synchronous GPU readbacks include CPU submission and readback wait, not just GPU execution. These software tests do not replace physical tablet testing.

## Opacity and soft edges — SKETCH-013

Expand **Pressure, tilt & taper**. **Opacity** sets the brush's base alpha; **Pressure: opacity** optionally varies alpha from the captured pressure samples using the same minimum and response curve controls as width. **Soft edges** feathers round-stroke silhouettes and ribbon edges. Each stroke retains its settings; changing the brush controls affects future marks. Older drawings and presets default to fully opaque.

The viewport draws opaque marks first, then blends translucent marks in camera-depth order with depth testing and no depth writes. This keeps document order from controlling ordinary overlap and lets opaque surfaces cover translucent marks. Intersecting strokes can still have approximate layer ordering where their geometry crosses; transparent strokes do not cast solid shadows. GLB stores final alpha per vertex using standard `COLOR_0` and `BLEND` materials, so pressure variation is retained without custom extensions. Soft coverage is baked against the captured brush frame and can look slightly different when viewed from a substantially different angle or in a viewer with a different transparent-surface sorter.

## Built-in textures — SKETCH-011

Expand **Surface texture** to choose **Solid**, **Grain / chalk**, **Dry brush**, or **Diamond pattern**. **Spacing** is the repeat distance along the stroke in world units; across the width/circumference the tile follows the brush section. **Strength** removes more coverage, and **Vary pattern** changes the captured seed. Strength zero is solid. These are opaque masks with holes; soft transparency belongs to SKETCH-013.

Patterns follow distance along ribbon/chisel marks and around round strokes, including dots. They stay attached when orbiting or switching guides. Uniform scale scales the repeat distance with the ink. Rotation, duplication, undo, and save/reload preserve the resolved settings. Picking and erasing use the whole stroke footprint, including visible holes. Selection turns covered ink gold while preserving holes and depth behavior.

**Textures** opens an editable comparison with flat marks, dots, and round companions. Start with `cargo run --release -- --textures`. Run `cargo run --release -- --verify-textures` to verify rendering, selection, transforms, drawing/erasing, and native persistence. Outputs are `verification/texture-report.json` and `verification/texture-preview.png`.

GLB embeds the same original 128×128 PNG masks, UVs, repeat/nearest samplers, original colors, and glTF MASK materials. No image sidecars are needed. The masks are generated by this project under its GPL-3.0-or-later license; no third-party artwork is used. `--verify-export` includes `textures.glb`, and the Blender verifier checks packed image pixels, UV bounds, color factors, and masking.

For reproducible long-stroke measurements, run `cargo run --release -- --benchmark-textures`. It writes `verification/texture-benchmark.json`: five CPU tessellations per 10,000-point ribbon/round mark and PNG-generation time for each style. One local run measured roughly 1.5–1.7 ms per ribbon mesh and 11–12 ms per textured round mesh. These measurements describe this machine and CPU path, not tablet latency or a frame-rate guarantee. Completed meshes and mask bindings are cached; many unique masks consume additional GPU memory.

## Local brush library — SKETCH-012

The **Brush library** panel includes Round Ink, Ribbon, Flat Marker, Chalk, Dry Brush, and Diamond Pattern. Browse the list to inspect a live mesh preview and settings, then choose **Use preset** to apply it. Preview pressure and tilt are synthetic; the preview does not depend on your hardware. Applying a preset finishes active ink and configures future strokes, including color, size, geometry, dynamics, texture, and stabilization.

Set the brush controls and choose **Save as new** to create a personal configuration. The name field supports **Rename** and **Duplicate**; **Update** replaces the selected preset from the current controls. **Delete** removes a library entry. Built-ins can be edited or deleted; **Restore built-in defaults** restores the six originals and keeps personal presets. Names must be unique (ignoring case), and a conflicting personal name must be renamed before restoring defaults. The library supports up to 250 personal presets.

Changes save atomically to `%LOCALAPPDATA%/SpatialSketch/brushes.json` on Windows. Other platforms use `$XDG_CONFIG_HOME/SpatialSketch/brushes.json` or `~/.config/SpatialSketch/brushes.json`; those platforms still need native tablet validation. This versioned library is separate from drawing files. **Save library** retries failed writes; an error notice identifies edits retained only in memory.

Corrupt or unsupported library data falls back to temporary built-ins without overwriting the original file. **Repair library (keep backup)** explicitly preserves the original bytes in a timestamped sibling backup before saving valid defaults. Drawing files are unaffected. Editing, renaming, or deleting presets never changes previously drawn strokes or their exports, because strokes store resolved settings. Importing/sharing libraries remains deferred under SKETCH-014.

Run `cargo run --release -- --verify-presets` for library operations, restart persistence, explicit repair/backup, resolved stroke settings, export preservation, and GPU previews. It uses an isolated library under `verification/preset-library`, writes `verification/preset-report.json` and `verification/preset-preview.png`, and leaves the normal personal library untouched.

## Autosave and interrupted-session recovery — SKETCH-003

Completed drawing/edit commands queue recovery snapshots after two seconds without another edit, or at the next idle point after fifteen seconds of pending edits. Active strokes and transform gestures finish before a snapshot is taken. Serialization and disk writes run on an ordered background worker; the UI clones a consistent document only when a snapshot is due. Large drawings can still make that clone noticeable. An unfinished stroke, or edits newer than the last confirmed snapshot, can be lost if the process stops.

The right panel reports pending, saving, saved, unavailable, and failed recovery states. A failed write never reports newer edits as protected and retries after fifteen seconds. Untouched starters and blank sessions produce no drawing snapshots. Recovery is separate from native Save / Save as and never silently updates the original file.

On startup, **Recover interrupted drawings** shows the original file identity (or Untitled), snapshot age, and stroke count. **Recover copy** creates a dirty, untitled document; choose Save as to give it a native destination. The old snapshot is retained until a new-session recovery copy is durable. **Discard snapshot** deletes that recovery entry. **Continue without recovery** keeps entries on disk, available from the bottom **Recovery** button. Recovering while another drawing is edited uses the normal unsaved-change prompt.

Successful native saves and approved New/Open/sample replacements clear this session's old recovery copy; canceling or failing Open leaves it intact. Normal close waits for ordered cleanup. If deletion fails, the retained-copy error stays visible and a second close explicitly exits with the copy retained. Other running instances are excluded by session locks. Snapshots older than thirty days, future-dated records, and originals saved more recently are flagged and remain available as copies. Incomplete, corrupt, unsupported, or oversized files are listed with an explanation and left untouched unless dismissed.

Storage is `%LOCALAPPDATA%/SpatialSketch/recovery` on Windows, beside the local preset library. Each process has its own file; OS locks release after unexpected termination. Native saves cannot target this reserved folder. Discovery bounds file count and aggregate bytes to limit background work. Recovery preserves native geometry, pressure, tilt, dynamics, and texture settings; it does not restore undo history, camera state, guide controls, or the local preset library.

Run the forced-crash regression on Windows from the project root:

```powershell
./scripts/verify_recovery.ps1
```

It starts an isolated edited session, waits for a confirmed snapshot, force-stops only that test process, and restarts the app. The restarted check restores exact data into an unsaved copy, validates cleanup/dismissal, and confirms the original native file's SHA256 is unchanged. Outputs include `verification/recovery-report.json`, `recovery-offer.png`, and `recovery-preview.png`. The test uses `verification/recovery`, leaving normal application recovery data untouched. Build the release executable first; the script has bounded waits and stops only processes it created.

## Drawing layers — SKETCH-004

The **Drawing layers** panel selects the active layer for new ink. **Add** creates and activates a named layer; **Rename** updates the active layer. Names must be unique. **Show** controls visibility; **Lock** protects its strokes. Drawing on a hidden/locked active layer is blocked with a status message.

Hidden ink is excluded from the viewport, picking, selection, and GLB exports. Locked visible ink stays visible and exports, but picking/erasing/selection skip it (an editable stroke behind it can still be picked). Selected strokes can move to a visible, unlocked target layer. Duplication retains each source layer. All layer metadata and stroke reassignment operations support undo/redo and recovery snapshots; temporary selection is cleared when its strokes become protected.

**Delete empty** refuses nonempty layers and the last remaining layer. Move or deliberately delete their editable strokes first; deleting a layer never implicitly deletes artwork. Native version-5 drawings preserve all layer state. Layers organize ink without changing its 3D positions or using a painter-style stacking order; scene depth still determines visibility.

Run `cargo run --release -- --verify-layers` for drawing restrictions, visibility/picking, reassignment/undo, deletion guards, save/reload, and GPU rendering. Outputs are `verification/layer-report.json` and `verification/layer-preview.png`. `--verify-export` also creates `layers.glb`, demonstrating visible locked ink and exclusion of hidden strokes, with layer IDs/names retained as node metadata.

## Revolved drawing surfaces — SKETCH-016

Choose **Revolved** in the Drawing guide panel to shape a vase, bowl, or other rotational side surface. Scroll the left tools panel to Drawing guide (or collapse Layers/Brush library). Drag the cyan profile points or edit their **R** radius and **Y** height. Insert/remove points to use two through eight ordered profile knots. Vase, Bowl, and Cylinder buttons provide starting profiles. Heights must increase and radii stay positive; invalid edits leave the last valid guide in place.

Position and rotate the guide, use **Profile from view**, or press **F** to align the camera. Pen/mouse rays hit the nearest visible side; orbit to paint its back. Pressure, tilt orientation, stabilization, layers, textures, and normal stroke editing remain available. Filtering and release tails follow the captured curved surface, including profile corners and the angular seam. Missed regions split strokes as with the cylinder. The profile is piecewise linear and revolves about local Y; open ends/caps are not drawing surfaces.

Guide changes finish active ink first. Existing strokes remain fixed when changing/hiding the guide, and support selection, duplication, undo, saving, recovery, and GLB export. The custom guide/profile itself is temporary workspace state, like existing plane/cylinder controls; native files retain the resulting world-space ink rather than the profile editor configuration. This feature does not deform existing artwork or create a solid-model baking tool.

**Vase** opens an original painted study made from 73 editable strokes: brown body marks, blue bands, cream diamond inlays, and gold rims. Start with `cargo run --release -- --vase`. Run `cargo run --release -- --verify-vase` for rotated-guide drawing, captured tails, pressure/tilt, duplication/undo, guide independence, native persistence, and GPU rendering. Outputs are `verification/vase-report.json` and `verification/vase-preview.png`. `--verify-export` includes `vase.glb`; it contains the ink meshes, without the guide wireframe or a hidden solid vase.

### Stroke groups

In selection mode (V), select strokes on one editable layer, then choose **Group** (Ctrl/Cmd+G). Clicking a member selects its whole group; Shift-click toggles the group. **Select individual strokes** allows member editing; turning it off expands the selection to complete groups. **Ungroup** (Ctrl/Cmd+Shift+G) removes membership from the selected groups.

Groups share the selection transform pivot. Duplicating a complete group creates an independent group; copying only some members creates ungrouped strokes. Groups stay on one layer, so moving them between layers requires selecting every member. Layer visibility and locking protect all members consistently. Grouping, duplication, transforms, and ungrouping support undo, native saving, and recovery. GLB exports preserve separate meshes with group metadata.

Delete removes the selected strokes: normally a whole group, or individual members in individual mode. The whole-stroke eraser still removes one stroke at a time. A group may retain a single member; empty groups disappear automatically. Nested groups and brush import remain future work.
## Optional startup tutorial

On first launch, a welcome panel offers **Start practice** or **Skip**. Nine optional lessons cover marks/undo/erasing, actual pressure feedback, mouse and pen navigation, brush shapes/textures/presets, stabilization/taper, plane/curved guides, selection/transforms, layers/groups, and native Save versus GLB Export/recovery. Relevant control sections receive a turquoise tutorial marker. Use Back, Next, Finish, or Exit practice at any time; no exercise requires touch, tilt, or a particular tablet brand.

The offer normally appears once after starting or skipping. **Offer tutorial on every startup** remembers a recurring offer. **Help → Start / replay tutorial** opens it again, and Help also provides the startup preference and shortcuts. Startup recovery decisions appear before the tutorial.

Practice uses a separate blank drawing and edit history. Your original document, native path, dirty state, selection, history, camera, guides, lighting, and brush settings stay in memory and return on exit. The normal unsaved-change prompt appears before entering, leaving unsaved practice, or closing. Closing from practice first handles practice, then any unsaved original drawing. New/Open/sample and recovery switches require leaving practice first.

Autosave continues protecting the suspended original. **Practice itself has no automatic recovery**; use Save as to keep it deliberately, or GLB Export for practice meshes. Saving practice does not clear the original's recovery protection or change its file. Personal brush-library edits remain real library edits, as stated in the lesson.

Preferences are stored separately from native drawings in `%LOCALAPPDATA%/SpatialSketch/onboarding.json` on Windows (beside the brush library on other platforms). Missing preferences offer first-launch onboarding; invalid, oversized, or newer-version preferences are left intact on load and reported in Help. An explicit preference change/start/skip can replace them with current settings. Preference save errors stay visible.

Developer check: `cargo run --release -- --verify-tutorial` uses isolated workspace preferences and recovery storage. It verifies entry/exit cancellation, optional navigation, practice saving, exact original/history restoration, original recovery snapshots, close guards, and the lesson UI. Output: `verification/tutorial-preview.png` and `verification/tutorial-report.json`. Physical tablet validation remains deferred.
## Scene lighting and ground shadows

Open **Scene** in the top toolbar to adjust directional light azimuth/elevation, intensity, and ambient contribution. **Cast shadows** enables shadows on visible artwork and the optional ground. **Show ground** provides a finite XZ receiver with height, center, size, and color controls. **Fit ground & enable shadows** places it just below visible artwork, sizes it for the current light, and enables shadows. Fit again after changing the drawing or using a much lower light angle. **Reset scene lighting** restores the original lighting and disables ground/shadows.

The ground is a presentation surface, not ink or a drawing guide: it cannot be picked, painted, grouped, or exported. Use the normal plane/curved guides for drawing. Hidden strokes neither receive nor cast visible shadows; locked visible strokes still do. Selection gold affects display color only, and textured brush holes also let light through.

Lighting and ground are **temporary workspace settings** retained while switching drawings in the same session and reset on restart. They do not mark the document dirty, have no edit-history commands, and are excluded from native Save, recovery snapshots, and GLB export. Native document version is now 7; lighting remains workspace-only. Tutorial practice starts with default lighting and restores the suspended original's lighting on exit. Exported meshes keep their original colors/materials; downstream applications supply their own lights and floor.

The renderer uses one fixed 2048×2048 directional depth map with filtered comparisons. Its world-space fit is independent of the camera. **Bias** is in world units: raise it slightly for speckles on surfaces, lower it for detached shadows. Very thin marks in large drawings can exceed the map's resolution; changing artwork bounds or light direction can also change sampling. There are no cascaded maps, physical area-light penumbras, or guaranteed subpixel shadow detail in this pass. Shadows are off by default, preserving the existing presentation until enabled. Standalone GPU benchmarks remain deferred to SKETCH-007.

Developer check: `cargo run --release -- --verify-lighting` verifies actual GPU pixels for ground darkening, masked shadow holes, selection invariance, hidden/locked rules, orbit/angle handling, and unchanged native/GLB ink. It writes `verification/lighting-report.json`, a UI preview, and house/planar/building images. Readback synchronization is used only by verification, never by normal frames.
## Filled planar shapes and outlines

Choose **Planar fill [P]**, select a plane guide, and click/tap corners. Click the first corner again or use **Close & fill / Enter** to commit. Concave simple polygons are supported, with 3–256 corners and no holes. **Undo corner / Backspace** removes the last draft corner; **Escape / Cancel contour** clears the draft. The fill color and paint offset are taken when closing. Pressure, taper, stabilization, and brush textures do not affect a solid fill.

The first corner captures the plane and layer. Orbiting and guide adjustments leave the draft on that captured plane. Cylinder/revolved guides cannot create fills: choose Plane first. Self-intersections, duplicates, degenerate area, invalid offsets, and nonplanar data report an error and leave the draft available to correct. Drafts do not alter existing artwork, enter edit history, or autosave. Finish/cancel before native Save, Export, switching drawings, leaving practice, or closing the app. Switching to Brush/Erase/Select explicitly cancels the unfinished contour.

**Paint behind plane** defaults to 0.003 world units opposite the captured plane normal. Draw outlines using B on the same guide plane; the fill sits slightly behind them. This is actual saved mesh separation, so GLB viewers see the same layering without special depth settings. Other objects still occlude both paint and outlines normally. Zero offset can cause z-fighting. From behind, the physical fill can hide outlines on its front side. Very large distances or tiny details may require an appropriate offset; there is no global always-on-top ink mode.

Use V to select a filled interior; concave cutouts stay unselectable. Fills support object transforms, duplication, mixed fill/stroke groups, layer visibility/locking, whole-mark erasing/deletion, undo/redo, native saving, and recovery. Scaling also scales the paint offset. Existing brush-radius/point/offset limits apply to transforms. This first version edits completed fills as objects; per-corner editing, holes, automatic freehand closure recognition, textured fills, extrusion, and a bucket fill are outside this pass.

**Open painted-house study** in the Fill tool opens an original editable wall/roof/window/porch example, including a concave porch and separate outlines. The original Japanese starter remains available through House. GLB exports fills as double-sided triangle meshes, bakes the offset, and adds optional planar-fill/offset metadata. It contains no UI draft, guides, lights, or presentation ground.

Developer check: `cargo run --release -- --verify-fills` writes `verification/fills-report.json`, `fills-preview.png`, and `filled-house.png`. The export fixture set includes `fills.glb` for Khronos validation and Blender import. Forced-crash recovery verification now includes a planar fill alongside pressure/tilt/textured ink. Document validation bounds contour complexity and rejects malformed filled geometry; legacy files treat any unknown fill extension as ordinary ink, matching their original rendering.
## Place a drawing plane on artwork

Press **K**, use **Pick plane [K]** in the toolbar, or **Place plane on artwork** in Drawing guide. Click/tap a visible artwork surface to propose a plane. The grid and pink normal marker preview it; your current guide and artwork remain unchanged until you choose **Apply / Enter** or **Apply & face plane**. Click another surface to repick. **Cancel / Escape** keeps the previous guide, including cylinder/revolved mode and visibility. Space-drag and mouse navigation remain available while picking.

**Flip normal** reverses the proposed front side. **Normal offset** moves the plane along that normal; its default 0.005 world units gives a small clearance for new ink. Zero places the plane at the actual surface point. Applying switches to a visible plane guide; F faces it later if desired. Accepted planes are independent workspace guides: moving or deleting the original artwork does not move the plane. These settings are temporary, with no native/recovery/GLB schema changes (document version stays 8).

Picking uses the nearest visible triangle surface, including two-sided filled faces, ribbon/chisel faces, round surfaces, and caps. It respects the same repeated nearest-sampled mask holes as the renderer, so a hole can expose a reference behind it. Hidden layers, guide wireframes, and the presentation ground are excluded. Locked visible artwork is allowed as a read-only reference; it stays locked, and drawing still requires an editable active layer.

Round strokes produce a local tangent plane using interpolated surface normals; flat faces use their face normal. The initial normal faces the viewing ray, with a stable projected tangent where possible. A cap or sharp edge can have an ambiguous orientation—pick a broad interior area or repick/flip. Very thin marks need an actual surface hit, with no selection halo; zoom in when necessary. This places a plane, rather than fitting an entire building face or continuously wrapping new ink onto arbitrary artwork.

Placement mode pauses ordinary drawing, selection, and edit shortcuts; the preview panel remains interactive. Resolve an unfinished fill contour before starting. Leaving for another document or practice/close action ends placement without applying the preview. Native Save and GLB Export never include a pending preview. Existing manual planes, plane-from-view, presets, and numeric controls remain available after applying/cancelling.

Developer check: `cargo run --release -- --verify-placement` writes `verification/placement-preview.png` and `placement-report.json`. Unit checks cover rotated/locked/hidden references, nearest hits, back faces, round/chisel surfaces and caps, misses without a halo, and masked holes. The graphical workflow verifies cancellation of an existing curved guide, flip/offset, unchanged document/history/GLB bytes, alignment, and new ink constrained to the accepted plane. Physical tablet and performance campaigns remain deferred.
## Background and artwork-only view

Open **Scene → Background** to choose a canvas color, the **Dark / Blue / Light** presets, or **Reset background**. Canvas background is independent of stroke, fill, and presentation-ground colors. **Remember background** saves the current color as your startup default; other color changes are session-only. Lighting and ground still reset on restart. Background preferences live in `%LOCALAPPDATA%/SpatialSketch/presentation.json` on Windows, beside the brush library on other platforms, separately from drawings and onboarding.

Use **Artwork [Tab]**, the Scene panel button, or **Tab** to enter artwork-only view. It hides side panels, the main toolbar/status bar, grids, guide badges, selection gold/handles, and sample labels. The **Workspace [Tab]** button stays visible alongside **Save** and **Recovery**. Tab or Escape returns to editing. Shortcuts apply when a text field is not being edited; the mouse/pen Workspace button is always available.

Artwork-only view is read-only for marks: ordinary contact and eraser contact do not edit artwork. Orbit/pan/zoom still work through Space/pen or mouse controls, and Ctrl/Cmd+S (Shift for Save as) can save. Selection, tool/guide settings, open Scene-panel state, and edit history return unchanged; deliberate camera navigation is retained. Finish/cancel a fill draft, plane preview, tutorial, or active dialog before entering. Entry finishes an active stroke/transform normally.

Recovery and unsaved-change prompts return to the workspace. Close requests also leave artwork view, including an export that temporarily blocks closing. Native save errors return to editing so the error can be seen. The compact toolbar indicates recovery issues; Recovery opens the normal recovery flow. Artwork-only mode is never remembered across restarts, so each launch starts with the workspace visible.

Background and view mode do not change native document version 11, recovery content, GLB materials, or ink. Invalid, oversized, or newer-version preference files are retained on load; Scene shows the issue and uses defaults until you explicitly remember a valid preference. Reset changes the current session; use Remember afterward to reset the startup default as well.

## Saved freeform guide

In **Drawing → Saved guides**, choose **Add freeform** to create an editable bounded sheet. Adjust the center and rotation, sweep depth, bend, and four local XY directrix control points. The directrix control X coordinates must remain increasing, which keeps the curve monotone and excludes loops. Activate the guide with **Use** and draw with the brush; new ink is stabilized on the guide while existing strokes remain fixed. The preview is a wire grid. Ray hits are approximated against a deterministic 64×8 triangle mesh, so the guide is suitable for sketching and reference construction rather than precision CAD. Line, fill, shape, and curve draft tools remain unavailable while a saved guide is active.

## Loft guides

Select 2–8 open cross-section strokes drawn in the same endpoint direction, then choose **Preview loft from selected strokes** in **Drawing → Saved guides**. The preview lists ordered sections and confirms that each stroke keeps its endpoint direction. Apply saves a snapshot; Cancel or Escape leaves the drawing unchanged. The tool samples each stroke by arc length, orders sections along their shared progression axis, and rejects degenerate, folded, or direction-mismatched sections. Later edits to source strokes do not alter the loft. Tension and transform are editable.

## OBJ reference models

Use **Drawing → OBJ references → Import OBJ reference…** to load a bounded Wavefront OBJ with optional relative MTL files. The importer supports positions, UVs, normals, objects/groups, polygon triangulation, and MTL diffuse color (`Kd`) plus opacity (`d`/`Tr`). Texture maps and other MTL properties are ignored; the inspector reports the count of unsupported directives. Paths cannot escape the OBJ directory. Imported files are normalized into the drawing's `.assets/models` folder, so reopening a saved drawing does not require the original source files. Models are separate read-only reference geometry: they can be shown/hidden, locked, moved, rotated, scaled, and undone, and they are excluded from GLB and artwork PNG exports. Recovery snapshots include the model and declared MTL files under their managed paths.

In Select mode, click a visible model to select it in the viewport. A teal box marks its transformed bounds; use the model's inspector controls for move, rotation, and scale. Shift-click toggles the model selection. Stroke selection and model selection remain separate.

Enable **Draw here** on one visible, unlocked model to place freehand strokes on its surface. Picking selects the nearest positive triangle hit in that model and accepts both face directions. A miss ends the current stroke. Sampling continues only within one triangle or across a shared manifold edge, with a 24-pixel maximum gap; larger jumps, non-manifold edges, and separately indexed seams split the stroke so it cannot bridge disconnected mesh parts. Surface points are stored directly in world space without planar stabilization. Each stroke keeps the surface normal and a camera-facing tangent captured at its first hit. Moving or scaling the OBJ later leaves existing strokes where they were drawn; the resulting strokes remain ordinary editable ink and are included in normal drawing exports. This surface workflow has compile validation; hands-on interaction and save/recovery round trips remain to be checked.

## Animated GIF export

Choose **Export → Animated GIF (.gif)…** for a fixed-focus full turntable or a saved camera sequence. Sequence export uses the saved timing and camera sampler; turntables use the current view as a fixed-pivot start pose. The exporter samples deterministically and renders clean artwork frames without selection handles or editing overlays. Output is bounded to 480 frames, 24 FPS, and 921,600 pixels per frame, with progress and cancellation. The current camera and drawing are not changed. GIF encoding uses the bundled Rust `gif` and `color_quant` crates (MIT/Apache-2.0 and MIT); no external encoder is required. Existing destinations require overwrite confirmation, and incomplete/cancelled output is discarded from a temporary file.

The **Reconstructed reveal** mode is a synthetic preview from the current visible strokes in saved document order. It spreads those strokes evenly across the chosen duration; it does not claim to reproduce when they were drawn. Deleted strokes are absent, while imports and mirrored copies appear in their current list order. Native document saves already preserve this order, so no recording log or additional storage is needed.

Developer check: `cargo run --release -- --verify-presentation` uses isolated preferences and writes `verification/presentation-report.json` and `presentation-preview.png`. GPU readback checks blue/light/dark sRGB values. Lifecycle checks cover explicit preference restart, mode entry guards, read-only contact, workspace restoration, save-failure visibility, recovery/unsaved prompts, and unchanged JSON/GLB ink. Dedicated benchmarks remain deferred.
## Orthographic camera views

Open **View** to switch Perspective/Orthographic and choose Front, Side, Top, or Face active guide. **5** toggles projection; **1 / 3 / 7** face front/side/top when no text field is active. Navigation, artwork view, guide picking, drawing, and transform handles use the chosen projection. Switching preserves framing at the camera focus plane; off-plane objects change apparent size as perspective is removed. Zoom uses the same focus-plane scale in both modes.

World views retain the camera focus and do not change the drawing guide; use F to face the active guide or Shift+F to make a guide from the view. Camera settings are temporary, excluded from native Save/recovery/GLB, and most sample/New/reset actions restore perspective defaults. Orthographic depth covers 500 world units either side of the focus plane and does not change while zooming. Very large/deep scenes may need a different focus.

`--verify-views` checks projection framing/rays, pixel scale, orthographic picking/drawing/transforms, and unchanged native/GLB artwork; it writes `verification/views-preview.png` and `views-report.json`. Straight-line assistance is described below.
## Straight-line drawing assistance

Select **Straight line [L]**, press on a plane guide, drag to preview, and release to create one editable ink mark. Choose **Free angle**, plane-local **Horizontal / Vertical**, or **Angle snap** with a 1–90° step. Constraints follow the plane captured at contact, including rotated planes, rather than screen axes. Freehand B remains the default; cylinder/revolved guides use freehand surface drawing and cannot create a straight line through their interior.

The line freezes the current color, radius, brush shape, texture, dynamics, plane, layer, constraint, and angle step at contact. Start/end contact pressure and optional world tilt are retained and interpolated through ordinary brush geometry; pen-up updates position without replacing pressure with a hover/zero value. Mouse input uses fixed pressure. Taper/textures work normally; stabilization is bypassed for exact endpoints.

Escape, cancelled contact, leaving the canvas, releasing over UI, or navigation/guide/view changes cancel an unfinished preview. Zero-length lines create no mark. Resolve a line preview before Save/Export, document switches, plane placement, or artwork-only mode. Completed lines support normal selection, groups, duplication, transforms, deletion, native save/recovery, and GLB export; creation is one undo step. Lines use the existing stroke fields and do not add line-specific document state.

`--verify-lines` tests actual mouse event ordering, exact release endpoints, rotated orthographic constraints, pressure/tilt/taper capture, cancellation, protected layers, native reload, and GLB encoding, producing `verification/lines-preview.png` and `lines-report.json`. Export fixtures include `lines.glb`; forced-crash recovery includes a completed constrained line alongside fills and textured pen ink. Physical hardware testing and dedicated benchmarks remain deferred.

## Thin-line rendering

The canvas uses 4x multisampling when the adapter supports it, with a 1x fallback. It resolves the multisampled color into the same single-sample canvas texture used by the UI and readback. Textured brush masks use alpha-to-coverage at 4x; the fallback retains the prior binary mask cutoff. This is a display-only change: stroke meshes remain world-space geometry, and selection, picking, shadows, and GLB dimensions continue to use the original meshes.

The before/after capture is the same house line study from `--verify-lines`: `verification/thin-lines-before.png` (1x) and `verification/lines-preview.png` (4x). On the test GPU / Vulkan adapter, the canvas was 955×867 pixels (827,985 pixels). The 4x target evaluates 3,311,940 coverage sample positions per frame, a 4× raster sample count; it adds a multisampled color target and four-sample depth attachment. The UI's combined UI/canvas CPU counter read 0.33 ms before and 0.37 ms after in these captures; this is CPU submission time, not GPU time. No dedicated GPU timestamp query is enabled, so the sample count and attachment cost are the recorded GPU-work estimate. `verification/lines-report.json` records adapter, dimensions, and sample count when `--verify-lines` is run.

## Repeating grouped details

In **Select**, select a grouped detail and choose **Repeat…**. Set the number of copies and spacing on world axes or the active guide's rotated local axes. The preview shows cyan copies while the source stays selected. Apply adds all copies as one undoable edit; Cancel or Escape removes the preview without changing artwork. Each full group copy gets fresh stroke and group IDs, so copies can be selected and edited separately. Fill contours, outlines, layer assignments, colors, and brush settings are retained. The default spacing follows the selected artwork's projected width plus a small gap.

Repeat is limited to 100 copies per operation and still enforces the document limits of 100,000 strokes and 2,000,000 control points. Invalid counts, offsets, layer states, ID exhaustion, and document bounds are rejected before copies are committed. Run `cargo run --release -- --verify-repeat` to exercise guide-local spacing, preview/cancel, independent IDs, locked-layer protection, one-step undo/redo, native reload, and GLB export. It writes `verification/repeat-preview.png` and `verification/repeat-report.json`.

## Scene palette and color sampling

Expand **Scene palette** in the workspace tools to add the current RGB color as a named swatch, choose it for future brush marks, or rename/remove a swatch. Palette edits save atomically in a versioned `palette.json` beside the local brush library, independently of brush presets and drawing documents. It supports up to 32 named colors; names are limited to 32 characters and must be unique.

Choose **Sample artwork [I]** or press I, then click or tap visible ink or a planar fill. Sampling picks the nearest visible artwork and reads its stored base color, so lighting and selection highlighting do not affect the result. Hidden layers are skipped; visible locked layers can be sampled. A sample changes the current color only. Contact is consumed until release, so a press-drag cannot leave a mark. Escape or **Cancel sampling** returns to the brush without changing artwork.

Run `cargo run --release -- --verify-palette` for fill/outline sampling, visibility/locking, contact cancellation, palette CRUD, and restart persistence checks. It writes `verification/palette-preview.png`, `verification/palette-report.json`, and an isolated palette file under `verification/palette/`.

## Artwork image export

Choose **Export → Artwork image (.png)…** to save the current view as a PNG. Set dimensions up to 8192 pixels per side and 8 megapixels total. The export uses the current camera target and vertical framing; changing the output aspect changes the horizontal framing. It keeps the current background and lighting and includes visible layers, while omitting guides, selection tint, handles, and interface overlays. The export renders from a snapshot and does not change the drawing, camera, selection, history, or workspace presentation.

An existing destination requires a separate overwrite confirmation. Run `cargo run --release -- --verify-png` to render and inspect the storefront fixture at two dimensions; it writes `verification/artwork-1024x768.png`, `verification/artwork-640x480.png`, and `verification/png-report.json`.
