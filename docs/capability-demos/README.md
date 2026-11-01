# Capability demo recording workflow

This folder defines the capture recipe for SKETCH-066. **No capability clips are published yet.** Record and review each clip only after the workflow it demonstrates has passed its feature checks.

The editable sample documents are generated from the built-in original scenes with `cargo run -- --write-capability-fixtures docs/capability-demos/fixtures`. The command refuses to overwrite existing files and removes files it created if fixture generation stops partway through. Static tutorial alternatives and draft captions are in `tutorials/`.

## Capture a clip

1. Build the application and stage the exact fixture and UI state listed in [shot-list.md](shot-list.md).
2. Close dialogs, notifications, and unrelated windows. Keep private file paths and account information out of the application window.
3. Run `scripts/record_capability_demo.ps1` from PowerShell 7 with a short unique name, duration, fixture, theme, DPI, and one `-ShotStep` argument per action. The script accepts only the exact `Sketch3D` title and captures that window only; it refuses to proceed if multiple windows have that title and never captures the desktop.
4. Operate the app manually during the capture. The capture utility does not inject input, synthesize pressure, or measure latency. If a future clip uses deterministic replay, identify that fact in both the clip manifest and its caption.
5. Review the MP4, GIF loop, poster, and JSON manifest. Correct `review` fields and record the reviewer/notes only after checking the clip at its intended README/tutorial size.
6. Keep raw masters and unreviewed material under ignored `verification/capability-demos/`. Copy only reviewed, bounded assets into a curated release location. Do not add raw recordings to Git.

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

The capture manifest includes byte sizes and SHA-256 hashes for the MP4, GIF (when created), and poster. If publishing the completed output set fails partway through, the script removes files already moved by that capture so a retry does not inherit a partial set.

## Review checklist

- Confirm the recorded window is Sketch3D and no unrelated screen content appears.
- Confirm every shown action is supported by the selected task and that the app is in a verified build.
- Check captions, cursor cues, control labels, line readability, loop boundary, and playback at the actual embed dimensions.
- Do not claim tablet pressure, device compatibility, latency, or platform behavior unless those were independently verified.
- Add asset provenance/license, app commit, fixture version, viewport/DPI/theme, steps, duration, and encoding recipe to the manifest.
- Provide equivalent written steps and a static image/poster with alt text; animation must not be the only instruction.

## Manifest

Use [manifest-template.json](manifest-template.json). The capture script writes a populated per-clip manifest next to each local output. A clip is not ready for publication while its `review.reviewed` field is false.
