# Capability demo recording workflow

This folder contains the current-build SKETCH-066 capture recipe, editable samples, action-focused replays, and static tutorial alternatives. Every replay uses synthetic pressure-free input; none measures tablet pressure or latency. Review every MP4, caption, poster, and manifest before publication. Review flags stay false until that review is complete.

See the [coverage audit](coverage-audit.md) for action coverage, provenance, remaining review checks, and newer functionality that is outside the original 066 acceptance.

The editable sample documents are generated from built-in original scenes with `cargo run -- --write-capability-fixtures docs/capability-demos/fixtures`. Tutorial runs also save a native sample through the production save path and reopen it through the production open path. Static alternatives and caption transcripts are in `tutorials/`.

## Deterministic hidden preview

Run each bounded replay from the repository root. Existing outputs are never overwritten; move the complete set aside before recapturing.

```powershell
cargo run --locked -j1 -- --record-capability-demo draw-orbit
```

Each short focus clip produces a six-second MP4 and a 640×360 GIF preview (≤5 MiB), full-size poster, VTT, and JSON manifest in ignored `verification/capability-demos/drafts/`. The GIF plays once. The app displays synthetic-input/no-pressure disclosure and action-specific pointer cues. MP4 uses FFmpeg/libx264 when installed and the application’s Blender H.264 fallback otherwise. Manifests record the source commit, capture dimensions, fixture provenance, output sizes, and review fields; caption timing and privacy flags remain false until reviewed.

The five focus replays are:

- `draw-orbit`: draw through `canvas_input`, then orbit.
- `curved-guides`: create mirrored strokes across a saved cylinder with the production pointer handler, then orbit.
- `mirror-shapes`: create the mirrored line and ellipse through production line/shape handlers, then orbit.
- `edit-erase`: select a stroke, change its appearance, partially erase it, undo, and orbit through production handlers.
- `reference-export`: use the production OBJ Import button with an injected generated file result, insert a saved lantern through native merge, and export GLB during capture. The native picker and insertion preview are deliberately not displayed.

Example:

```powershell
cargo run --locked -j1 -- --record-capability-demo curved-guides
```

The overview and two long-form tutorials use 60 deterministic app checkpoints. Overview checkpoints are held three times to create an 18-second 10-fps MP4; tutorial checkpoints are held ten times to create 60-second MP4s. Their GIFs are accelerated six-second previews only. Use the MP4 and VTT for normal-speed instruction.

```powershell
cargo run --locked -j1 -- --record-capability-demo overview
cargo run --locked -j1 -- --record-capability-demo house-tutorial
cargo run --locked -j1 -- --record-capability-demo curved-tutorial
```

The overview shows drawing, a selection transform with undo/redo, camera orbit, and production GLB export options. Both tutorials show drawing, selection, move, undo/redo, orbit, native save and native reopen; each manifest links a matching saved native sample. House and curved tutorial VTT files correspond to the capture checkpoints and include visible-action descriptions. The local [static tutorial alternatives](tutorials/house-editing.md) and [curved-drawing steps](tutorials/curved-drawing.md) remain available when video cannot be played.

## Capture a clip

1. Build the application and stage the exact fixture and UI state listed in [shot-list.md](shot-list.md).
2. Close dialogs, notifications, and unrelated windows. Keep private file paths and account information out of the application window.
3. Run `scripts/record_capability_demo.ps1` from PowerShell 7 with a short unique name, duration, fixture, theme, DPI, and one `-ShotStep` argument per action. The script accepts only the exact `Sketch3D` title and captures that window only; it refuses to proceed if multiple windows have that title and never captures the desktop.
4. Operate the app manually during the capture. The capture utility does not inject input, synthesize pressure, or measure latency. If a future clip uses deterministic replay, identify that fact in both the clip manifest and its caption.
5. For tutorials longer than 25 seconds, provide `-CaptionFile` with a WebVTT sidecar. Validate the draft without starting capture by adding `-ValidateOnly`; cues must be ordered, non-overlapping, and within the requested duration.
6. Review the MP4, GIF loop, poster, caption timing, and JSON manifest. The script copies captions as a sidecar and records their size/hash; it does not claim that draft timings align with the footage. Correct `review.captions_timing_checked` and other `review` fields only after checking the clip at its intended README/tutorial size.
7. Keep raw masters and unreviewed material under ignored `verification/capability-demos/`. Copy only reviewed, bounded assets into a curated release location. Do not add raw recordings to Git.

Example (run from the repository root):

```powershell
.\scripts\record_capability_demo.ps1 `
  -Name draw-orbit `
  -DurationSeconds 20 `
  -Fixture house `
  -FixtureVersion v1 `
  -AppCommit (git rev-parse HEAD) `
  -Viewport 1920x1080 `
  -DpiPercent 100 `
  -Theme Dark `
  -ShotStep 'B: draw a short stroke' `
  -ShotStep 'Space-drag: orbit to reveal depth' `
  -ShotStep 'V: select the new stroke'
```

FFmpeg must include the Windows `gdigrab` input and libx264. The script caps captures at 2,700 frames, 7,680×4,320 metadata dimensions, 512 MiB per MP4, and 5 MiB per short GIF. GIF previews are generated only for clips up to 25 seconds; longer tutorials use an MP4 and poster. Output names never overwrite existing files.

Timed WebVTT is a sidecar, not burned into the MP4. Long clips require one. Caption files are UTF-8, at most 1 MiB and 200 cues; timestamps accept WebVTT's `MM:SS.mmm` short form and `HH:MM:SS.mmm` form, cannot overlap, and must end within the declared clip duration. The hours field is optional when zero and required when nonzero, as defined by the [W3C WebVTT timestamp syntax](https://www.w3.org/TR/webvtt1/#webvtt-timestamp). The two tutorial VTT files currently contain 60-second draft timing; they still need synchronization against reviewed footage.

For either 60-second tutorial, add `-DurationSeconds 60 -CaptionFile docs/capability-demos/tutorials/house-editing.en.vtt` (or `curved-drawing.en.vtt`) to the capture command. Use `-ValidateOnly` to check the selected VTT without launching FFmpeg or starting a recording.

The capture manifest includes byte sizes and SHA-256 hashes for the MP4, GIF (when created), poster, and WebVTT sidecar (when supplied). If publishing the completed output set fails partway through, the script removes files already moved by that capture so a retry does not inherit a partial set.

## Review checklist

- Confirm the recorded window is Sketch3D and no unrelated screen content appears.
- Confirm every shown action is supported by the selected task and that the app is in a verified build.
- Check captions, cursor cues, control labels, line readability, loop boundary, and playback at the actual embed dimensions.
- Do not claim tablet pressure, device compatibility, latency, or platform behavior unless those were independently verified.
- Add asset provenance/license, app commit, fixture version, viewport/DPI/theme, steps, duration, and encoding recipe to the manifest.
- Provide equivalent written steps and a static image/poster with alt text; animation must not be the only instruction.

## Manifest

Use [manifest-template.json](manifest-template.json). The capture script writes a populated per-clip manifest next to each local output. A clip is not ready for publication while its `review.reviewed` field is false.
