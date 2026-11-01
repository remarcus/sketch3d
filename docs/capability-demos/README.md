# Capability demo recording workflow

This folder defines the capture recipe for SKETCH-066. **No capability clips are published yet.** Record and review each clip only after the workflow it demonstrates has passed its feature checks.

The editable sample documents are generated from the built-in original scenes with `cargo run -- --write-capability-fixtures docs/capability-demos/fixtures`. The command refuses to overwrite existing files and removes files it created if fixture generation stops partway through. Static tutorial alternatives, caption transcripts, and draft 60-second WebVTT tracks are in `tutorials/`.

## Deterministic hidden preview

The first focused preview can be regenerated without opening a visible app window:

```powershell
cargo run --locked -j1 -- --record-capability-demo draw-orbit
```

This bounded 6-second replay draws a stroke and orbits the camera through `canvas_input`, captures one full app frame at a time, and writes an H.264 MP4 master, a 640×360 GIF preview, a full-size PNG poster, a WebVTT sidecar, and a JSON manifest under ignored `verification/capability-demos/drafts/`. The replay adds visible `DRAW` and `ORBIT` cursor cues and a synthetic-input disclosure. MP4 encoding uses FFmpeg when available and Blender's H.264 encoder otherwise. The command refuses to overwrite any of these outputs; remove or move the complete draft set before regenerating it. The manifest discloses synthetic `pressure=None` input, records the source commit and capture details, and leaves review flags false. This draft does not measure tablet pressure or latency and must pass the review checklist before it can be used in the README.

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
