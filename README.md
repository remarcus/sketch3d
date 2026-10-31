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

Documents use a versioned `.sketch.json` format with positions in world space. Version 6 adds flat stroke groups alongside named layers, visibility, and locking. Versions 1–5 migrate without changing resolved brush appearance; versions 1–4 receive one visible, unlocked layer. Older application versions cannot open version-6 files. Open validates version, IDs, positions, pressure, color, and size limits. Saves write to a temporary file in the destination directory and replace the destination after syncing. Errors appear in the bottom status bar. Recovery snapshots protect completed edits automatically; use Save for your deliberate native drawing file.

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

Next planned work is the optional startup tutorial (SKETCH-015). Soft/transparent brushes follow separately; brush-library importing remains deferred.

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

**Dynamics** opens an editable four-row study: pressure width, independent pressure thickness, tilt ribbon, and distance taper. Start with `cargo run --release -- --dynamics`. Run `cargo run --release -- --verify-dynamics` for synthetic pen contact/release/cancellation, missing-tilt fallback, persistence, and GPU verification; outputs are `verification/dynamics-report.json` and `verification/dynamics-preview.png`. These software tests do not replace physical tablet testing.

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