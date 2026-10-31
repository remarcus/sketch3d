# Architecture

## Data flow

Native tablet messages -> chronological normalized samples -> ray/plane intersection -> editable stroke points -> brush dynamics and tessellation -> cached vertex buffers -> depth-tested wgpu canvas -> egui texture.

## Modules

- `document.rs`: versioned file schema and validation. Does not depend on the UI or platform APIs.
- `geometry.rs`: camera, ray/plane intersection, grid, and brush meshes with pressure/tilt/taper. Unit tested separately from graphics hardware.
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

## Stroke stabilization

`stabilizer.rs` filters positions with a spatial trailing radius, while preserving pressure. The app converts the configured screen-space radius to world units at each contact sample. The filter stores its final raw target so finishing a stroke can append a short catch-up tail before the stroke enters history. Reset/cancel discards that target. Tests cover jitter reduction, exact endpoints, zero-strength passthrough, pressure, and cross-stroke isolation.

## Editable startup artwork

`scenes.rs` builds the Japanese-style garden house deterministically using the current document version, with sequential stroke IDs and the same point/pressure/color/radius data as user-created strokes. The artwork is authored in code and needs no downloaded assets. `starter.rs` installs the sample, its camera, and its drawing guide; New and House share the existing unsaved-change flow. Neither operation replaces an edited document without that flow. The untouched example is not dirty; later edits use normal history and save behavior. `--blank` and the two verification modes isolate startup artwork from drawing workflow tests.

## Arbitrary work planes

`DrawingPlane` holds an anchor, Euler rotation in degrees, and a normal offset. Its orthonormal basis drives both ray intersections and grid vertices. The app finishes active ink before changing a work plane; document points never depend on the current guide. Grid meshes are cached by the complete plane value and do not write depth, so they cannot occlude strokes.

The camera now has roll as well as yaw/pitch. Its explicit basis avoids an up-vector singularity for vertical views. Align-to-plane reconstructs yaw/pitch/roll from the guide normal and vertical axis, centers on the guide origin, and preserves distance. Plane-from-view converts the camera basis back into guide rotation around its focus. Tests cover rotated/translated intersections, grid coplanarity, rolled view conversion, and vertical alignment. `planes.rs` owns the interface controls and the drawing/persistence smoke checks. Guides remain workspace state; no document schema change is required.

## Cylindrical side guides

`surface.rs` defines an open finite cylinder, preview geometry, and a captured `SurfaceStabilizer`. Ray intersections are solved in the cylinder's rotated local frame using a double-precision quadratic, then clipped to its finite height and forward ray range. Preview visibility is separate from the drawing surface.

Plane stabilization retains world coordinates. Cylinder stabilization maps raw side hits into unwrapped `(angle * radius, height)` coordinates, preserving local world-unit distances and pressure. Both filtered points and the release tail are decoded back to the captured surface. Angular subdivisions are at most approximately 0.035 radians between emitted samples (within the subdivision limit), so the stored polyline approximates the arc with short chords. No cylinder metadata is added to stroke documents, and later transforms remain unconstrained world-space edits.

A missed guide hit finishes the current segment and allows a new segment on re-entry during that contact. The filter detects raw angular jumps above 90 degrees to split discontinuous side changes. Normal pointer-up, cancellation, navigation, and guide changes clear the resume state. Guide changes finish against the captured surface, so a radius or orientation edit cannot relocate an active stroke's tail.

`cylinders.rs` owns guide controls, alignment, the original editable lantern example, and the cylinder smoke workflow. Plane, cylinder, and hidden previews share the renderer's cached guide mesh. `--verify-cylinder` supplements the unchanged drawing/plane and house verification modes.

## GLB export

`export.rs` generates a self-contained glTF 2.0 binary asset directly from document strokes. It reuses the shared brush mesh dispatcher, drops degenerate triangles, and deduplicates position/normal pairs into indexed meshes. Materials are shared by original RGB values and sidedness; renderer highlight state is never consulted. Nodes preserve stroke IDs as string extras. Positions and normals use aligned interleaved buffer views; position accessors include bounds. The GLB writer uses padded JSON/BIN chunks, validates inputs and size budgets, and atomically replaces the destination only after full encoding and sync.

`export_ui.rs` captures a consistent document/selection snapshot, owns the native save dialog and worker result channel, and reports success/failure on the UI thread. Geometry generation and disk I/O happen on a worker. The UI prevents aliasing the active native source path and refuses to close while output is still being finalized. Export does not enter edit history or change native dirty/file state. Snapshot cloning still has an allocation cost on the UI thread for very large drawings.

The `gltf` crate is a dev-only independent parser used by unit tests. `export_check.rs` creates reproducible house, lantern, pressure, and selection fixtures. `scripts/verify_glb.cjs` runs the official Khronos validator; `scripts/verify_glb_blender.py` checks actual Blender imports and renders a review image. The exporter supports core opaque PBR materials without texture or extension dependencies. Native semantic brush data remains in the versioned JSON format, not the exported mesh asset.

## Brush geometry and version 2

`Stroke.brush` stores a round/ribbon/chisel kind, dimensionless thickness ratio, manual angle, and a frozen orthonormal normal/tangent pair. The pair is captured from the current guide at contact, so a dot has a complete frame even without a path tangent. Uniform scale changes radius, rotation changes both frame vectors, and edit snapshots preserve all brush metadata. Version-1 JSON gains the default round brush; unknown legacy brush extensions are ignored to preserve old appearance. Both old formats upgrade to the current version as described below.

`geometry::stroke_mesh` is shared by the renderer and exporter. Round uses the legacy tube positions and pressure profile with corrected cap normals and triangle winding. Ribbon/chisel deduplicate coincident samples, derive path tangents, and transport the width axis by projection. A captured tangent resolves dot and normal-parallel cases. Chisel faces use flat normals; ribbons remain geometrically thin and use two-sided rendering/materials. `stroke_extent` conservatively covers cross-sections and dot footprints for selection bounds, picking, and erasing. Existing point-count mesh caching remains valid because shape settings are captured once; transform paths explicitly invalidate changed stroke IDs.

`brushes.rs` owns shape controls, sampler loading, and the software brush workflow. The sampler also serves as an independent GLB export fixture. Opacity and import remain separately tracked tasks.

## Dynamics and version 3

`dynamics.rs` defines persisted independent pressure mappings, taper envelopes, arc distances, screen-tilt conversion, and optional hemisphere-aligned tilt interpolation. `Brush.dynamics` is frozen at stroke creation; `Point.pressure` stores raw pressure for new strokes and `Point.tilt` stores optional normalized world lean. Version-1/2 files upgrade to v3 with default mappings and no tilt; their old baked pressure values retain the original geometry. Validation rejects nonfinite/out-of-range responses, taper lengths, and tilt axes.

The pen contact path converts screen tilt using the current camera right/up basis, while pen-up and cancellation never add a pressure/tilt sample. Stabilization retains current contact dynamics, interpolates them for cylinder subdivision, and finishes with the last contact state. Missing tilt remains explicit rather than fabricated. Tilted cross-sections project lean perpendicular to the path, apply the captured manual angle, and choose a continuous axis hemisphere; a separate transported manual frame handles missing/singular lean.

Geometry maps physical width and chisel thickness independently and applies the minimum of distance-from-start/end taper ramps. It inserts up to three private distance landmarks before tessellation, so a sparse two-point stroke reaches the same taper extrema as a denser straight path. Native samples remain unchanged. A 1% taper floor retains nonzero tips; zero pressure minima may intentionally collapse a dimension. Radius, taper distances, and tilt/frame vectors transform together. Renderer and GLB export share the mesh dispatcher, with export budgets allowing for landmark vertices.

`brush_dynamics.rs` owns controls, the editable dynamics study, and a synthetic pen lifecycle smoke test. Hardware orientation/calibration and latency still need real-device validation.

## Masked textures and version 4

`texture.rs` generates original deterministic white/binary-alpha tiles from kind, strength, and seed. Solid and zero-strength settings share one canonical opaque cache key. Scale is a world-space U period used by geometry, not a separate image. Version-4 documents store settings per stroke; older documents upgrade with solid ink. Transform validation scales the period with the mark.

`Vertex` adds UVs (44-byte viewport stride). Tube UVs use arc length and circumference with duplicated wrap seams; ribbons/chisels use arc length and cross-section coordinates, with planar cap/dot mapping. Textured round marks use captured frames so ring roll remains attached under transforms; solid round positions retain the legacy basis. `stroke_mesh` remains the common viewport/export geometry path.

The renderer caches mask bind groups alongside cached stroke meshes and prunes unused images. Its fragment shader samples the mask at explicit LOD 0 and discards holes before depth writes; display-only selection color retains coverage. GLB adds interleaved UVs, embedded PNG buffer views, deduplicated images/materials, repeat/nearest sampling, and MASK alpha cutoff 0.5. The Blender check follows the imported color multiplication nodes, verifies packed image alpha pixels and UV bounds, and rejects sidecars.

`brush_textures.rs` owns controls and graphical smoke checks. `texture_check.rs` provides reproducible release CPU measurements for long strokes; it is not a GPU or hardware latency benchmark. Active strokes still rebuild on input, while completed strokes reuse buffers. Partial erasing and alpha-aware picking are outside this feature.

## Local preset library

`presets.rs` stores fully resolved Brush/radius/color/stabilization values in an independent version-1 library. Fixed built-in IDs 1–6 are recoverable; personal IDs start at 100. Transactional CRUD validates names, IDs, counts, and settings before updating memory. Atomic saves use a synced temporary file in the destination directory. Library validation does not read or mutate drawings.

`presets_ui.rs` separates browsing from application: only Use preset returns a cloned configuration to `SketchApp`. A separate cached SceneRenderer displays a real synthetic-pressure/tilt stroke before selection. Library changes invalidate its preview without modifying the main canvas. Failed saves keep in-memory edits and an explicit retry action. Failed loads keep temporary defaults and block replacement until explicit repair creates and syncs a uniquely named byte-preserving backup. Restore defaults retains personal entries and reports name conflicts transactionally.

`preset_checks.rs` exercises application, lifecycle, restart, fallback/repair, native stroke independence, unchanged GLB bytes, and preview rendering. Verification modes use workspace libraries under verification rather than normal application data. Library versions are independent of document versions; drawings retain resolved settings without requiring a preset ID or local library.

## Recovery snapshots and session lifecycle

`recovery_store.rs` stores a version-1 envelope containing session identity, snapshot time, original native path, and a validated/migrated native document. Writes serialize, sync a temporary file, and atomically replace only the session's recovery file. Alias checks protect native documents; deletion is restricted to direct recovery entries and refuses native document JSON. Discovery preserves invalid/old records as visible issues and bounds files/bytes. Exclusive OS file locks distinguish a live session from an interrupted one, including multiple app instances.

`recovery.rs` owns an ordered worker channel. Committed edits increment revisions; document boundaries increment epochs and enqueue Clear after earlier Save commands. Late responses cannot protect a new drawing, and a late write cannot resurrect a discarded session. Quiet/max intervals coalesce edits, pending snapshots block duplicate writes, failures retain unprotected revisions and bounded retry timing, and failed dismissals remain visible independently of subsequent Save/Clear success. Normal shutdown releases its lock; a forced process stop leaves a stale lock whose OS ownership has ended.

`recovery_ui.rs` integrates edit tracking, startup discovery, recover/dismiss/continue actions, reserved native-save locations, and close cleanup. A recovered drawing is untitled and dirty. Its original recovery entry is retired only after a new-session write succeeds (or a deliberate Save/discard makes retirement safe). New/Open/sample boundaries clear only the current session after the existing unsaved-change flow. Native file identity is retained only as recovery metadata until Save as. Closing waits for the ordered queue; failed cleanup requires an explicit second close to retain copies and exit.

Software verification uses an isolated workspace recovery directory. `scripts/verify_recovery.ps1` kills its owned fixture process only after a durable snapshot, restarts the app, checks exact input/brush data and source-file hash preservation, and verifies startup offer/cleanup. Unit tests cover atomic storage, corruption/versions/staleness, live sessions, coalescing, retries, epoch ordering, dismissal path spelling, and persistent cleanup errors. No document schema change is required.

## Flat layers and version 5

Document layers have stable IDs, names, visibility, and locking; each stroke stores a layer ID and the document stores its active layer. Native versions 1–4 migrate to one visible/unlocked layer, preserving brush data. Version-5 validation rejects duplicate/invalid layers, missing active layers, and orphan memberships.

Layer edits store before/after metadata and ID assignments rather than duplicate point arrays. The app finishes active strokes/transforms before applying layer changes and records them in normal edit/recovery history. Shared visible/editable helpers govern drawing, picking, selection snapshots, bounds, and export. Rendering skips hidden strokes; export retains locked visible ink and node layer metadata. Background export snapshots clone layer metadata alongside selected strokes. Empty-only deletion keeps membership valid and prevents implicit artwork loss.

Layer order does not override depth testing. Nested hierarchy remains future work. Software verification supplements migration, protection, undo, and exporter parser tests with a graphical layer workflow.

## Revolved-profile guides

`revolved.rs` models a fixed-capacity ordered radius/height profile and its world transform. Each segment is a conical frustum (including cylindrical/linear degeneracies); double-precision quadratic roots are clipped by segment height and the nearest positive side hit is selected. Captured normal/circumferential tangent frames support existing brush orientation. Guides are workspace values and do not change the native document schema.

Surface stabilization unwraps angle scaled by a fixed reference radius and maps height to meridian arc distance. It decodes filtered samples back onto the captured profile. Emission adds exact profile-knot crossings as well as angular subdivisions, so strokes and release tails cannot shortcut a bulge through a long chord. Existing plane/cylinder behavior is retained. `revolved_ui.rs` provides draggable/numeric editing, guide switching, the editable vase sample, and graphical checks; export reuses ordinary document strokes and the shared mesh dispatcher.

## Flat stroke groups and version 6

Each stroke stores an optional stable group ID; membership is implicit rather than a separate container. Native version 6 validates nonzero group IDs and requires all members to share a layer. Versions 1–5 migrate with ungrouped strokes. Singleton groups are valid and empty groups need no cleanup.

Group edits use ordinary before/after stroke snapshots, preserving undo and recovery. Selection expands a clicked member to its editable group unless individual mode is active. Leaving individual mode expands selected groups in linear passes. Regrouping and cross-layer moves reject partial group selections. Duplication remaps complete groups to fresh IDs and detaches partial copies; membership checks use linear scans and count maps. Transforms use existing selection bounds and pivots. Erasing remains per stroke.

GLB keeps separate stroke meshes and writes optional spatialSketchGroupId string extras. Grouping changes metadata without changing mesh geometry. Model/editing tests include migration, invalid membership, locking, history, independent copies, and a 10,000-stroke operation fixture; the graphical workflow checks the integrated selection/edit/export path.
## Tutorial and isolated practice workspace

`tutorial.rs` owns version-1 local onboarding preferences and nine lesson definitions. Preferences use bounded reads and atomic writes; unsupported/corrupt files are retained on load, suppress automatic offers, and surface an error in Help. First launch offers onboarding once; recurring startup offers are opt-in. Verification routes preferences into the workspace rather than user application data. No native document version change is needed.

`tutorial_ui.rs` waits for initial recovery discovery and recovery decisions before an automatic welcome offer. Lessons are nonmodal so canvas/tools remain usable; the welcome offer and existing confirmation/help/recovery windows block drawing. Canvas input checks the topmost egui layer before starting strokes or navigation, preventing input through floating lesson windows. Control-section markers identify the current lesson target.

Practice moves the original document/history/file/selection and workspace settings into a suspended state and creates a blank lesson document. Existing pending-action confirmation handles entry and exit. New/Open/sample/recovery boundaries are blocked until practice ends. Close first handles unsaved practice, restores the original, and then applies the original's close/save flow. Native saving/exporting practice is supported; practice saves do not call the original recovery session's mark-saved/retirement methods.

The existing recovery worker/session continues owning the original drawing and lock. During practice, edit tracking ignores practice edits and recovery ticks use the suspended original document/path/dirty state. Entry requests an original snapshot; a pending older write can finish before the ordinary tick submits a newer original revision. Practice geometry is never substituted into that recovery session. Practice is intentionally disposable unless saved, and both welcome and lesson UI explain this. Normal exit restores original history and workspace state without creating a document boundary or clearing recovery. Personal preset library changes remain global.

Preference tests cover restart, recurring-offer settings, corruption/future versions, bounded reads, and no load-time overwrite. The graphical tutorial check exercises original versus practice history, native source preservation, original recovery contents, unsaved/cancel/close flows, lesson navigation, and rendering. Standalone performance and physical hardware campaigns remain at the end of the backlog.
## Scene lighting and directional shadow maps

`lighting.rs` defines presentation-only settings, visible-artwork ground placement, and shadow projection fitting. Settings are renderer/workspace values, not document or preset fields; lighting adds no native fields and does not change recovery envelope v1 or GLB artwork. Defaults preserve the prior ambient/directional balance with ground and shadows off. `lighting_ui.rs` exposes a nonmodal Scene panel, fit, reset, and graphical checks. Tutorial workspace suspension includes lighting.

Cached stroke meshes retain their actual vertex bounds. A union of visible bounds drives a camera-independent light-space orthographic fit; projected caster corners extend depth coverage to the ground receiver. Padding, minimum extents, and XY texel snapping support small/large scenes without following camera motion. The light elevation is limited to 10–85 degrees and the view uses an alternate up vector near vertical. The depth map is fixed at 2048 square; large scene bounds can reduce thin-mark coverage.

The masked depth-only pass shares cached stroke geometry and procedural mask bindings, with no culling, depth bias, and the same alpha cutoff/LOD as the visible pass. Hidden strokes are skipped; locking does not hide casters. Selection changes only cached vertex colors, which the depth shader ignores. The ground is a cached six-vertex receiver, not a caster or document stroke. The main shader combines ambient plus directional illumination with nine comparison samples; world-unit receiver/normal bias supplements raster depth bias. Guides bypass lighting/shadows.

Shadow resources use a separate sampling bind group and a depth-pass pipeline layout that does not bind the active depth attachment for sampling. The main canvas receives the completed map after the shadow pass. No CPU waits occur during normal rendering. Verification adds a COPY_SRC canvas/readback buffer, checks receiver pixels rather than merely successful GPU submission, and isolates caches when rendering different fixture documents with reused stroke IDs. The shadow pass is skipped when disabled or there are no visible meshes; dedicated timing/optimization is deferred.
## Planar fills and native version 7

A semantic mark remains a Stroke for shared IDs, ordered history, grouping, layers, and export. An optional PlanarFill contains a world-unit offset; when present, its points are a closed simple contour rather than a sampled ink path. The captured brush normal/tangent supply the plane frame. Fill geometry is solid and double-sided, independent of brush dynamics and radius. Native v7 validates filled contours; versions 1–6 remove unknown fill extensions before deserialization and retain their ink behavior. Recovery envelope v1 embeds the upgraded document unchanged.

`polygon.rs` projects into a double-precision local plane, bounds contours to 256 corners, checks planarity, finite coordinates/offsets, degenerate edges/area, and nonadjacent intersections, removes redundant collinear corners, then ear-clips either winding into consistently oriented triangles. The shared geometry dispatcher returns actual offset triangle positions for GPU and export. Interior picking intersects the shifted plane and uses boundary-inclusive point containment, preserving concave notches. Bounds include shifted vertices; transforms rotate the captured frame and scale the physical offset before revalidation.

The fill offset separates paint from outlines along the captured normal without changing global depth comparison or overriding unrelated occluders. GLB uses the same mesh, double-sided materials, and optional spatialSketchGeometry/FillOffset extras. Existing layer/group metadata and selected export filters apply to both ink and fills. No exporter-specific redraw order is needed.

`fills_ui.rs` owns the P tool, captured-plane/layer drafts, corner undo, close/cancel, color/offset settings, and the painted-house study. Only a validated close allocates an ID, inserts an Add edit, marks dirty, and records recovery. Save/export and document/practice boundaries require resolving a draft first. Completed fills use ordinary editing/history; guide changes never deform existing fills. Tutorial practice preserves/restores the fill tool and offset. No completed-vertex editor or hole topology is introduced.

`fills_checks.rs` exercises input/concavity/interior picking, invalid-draft guards, layers, groups, duplication, transforms/history, native v7 reload, and actual GPU outline/occlusion pixels across views. Unit tests validate winding, triangle area, malformed contours, migration, and mixed-mark editing. The export fixture adds filled architecture with physical offsets; independent validators check mesh bounds, normals, sidedness, and imported geometry. Forced-crash verification includes both fills and pressure-sensitive ink. Standalone benchmarks remain deferred.