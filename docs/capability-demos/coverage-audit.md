# SKETCH-066 coverage and freshness audit

Audited 2026-11-02 against app commit `ecb4229`. Sources: the five top-level local draft manifests and their output files, two committed tutorials and fixtures, capture implementation in `src/app.rs`, current UI code, task acceptance/status, and changes since capture. This is a source/metadata audit with inspection of the mirror-shapes and edit-erase posters, not a new full-playback or current-build visual review.

The drafts demonstrate a subset of the original capabilities. All five predate SKETCH-071–086 implementation and need current-build comparison before publication. Their age alone does not establish that the shown behavior is broken. Several clips need additional recorded actions to teach the workflow their names suggest.

## Existing clips

All are six seconds, 10 fps, Dark theme, 1280×720 MP4/poster, and 640×360 single-play GIF. All 20 primary media/caption files exist and match manifest byte sizes. All five manifests have `reviewed`, intended README surface, boundary, and privacy checks false. Earlier task notes describe some frame/privacy inspection, but the manifest sign-offs remain incomplete.

| Clip | Recorded app commit | What is actually captured | Gap and refresh action |
| --- | --- | --- | --- |
| `draw-orbit` | `d134602` | Drawing through the canvas handler, then orbit | Closest match to its planned workflow. Recheck drawing appearance and brush UI after the dab/brush changes; caption timing remains unsigned. Does not demonstrate newer brush families. |
| `curved-guides` | `c44f980` | Orbit of a cylinder and mirrored strokes prepared before capture | Does not show guide activation, drawing, periodic-seam crossing, or a ring guide despite cylinder/ring shot-list coverage. Capture the guide interaction and seam gesture; add a ring shot or narrow the advertised coverage to cylinder results. |
| `mirror-shapes` | `c44f980` | Orbit of a prepared mirrored line pair and ellipse | Does not show enabling mirror, drawing the line, or placing the ellipse. Mirror UI has since gained radial symmetry controls. Refresh the UI and record creation; keep radial symmetry separate until SKETCH-085 acceptance passes. |
| `edit-erase` | `e60b7ef` | Partial erase, undo, and orbit; selection/recolor happen before capture | Captured erase/undo is useful, but selection, appearance editing, and transforms are not taught. Add those actions; recheck partial erasing with the changed stroke metadata. Caption timing remains unsigned. |
| `reference-export` | `b8ea60b` | Orbit of staged image/OBJ references and an inserted lantern; GLB export happens before capture | A result preview, not an import/export walkthrough. Record the controls, insertion/placement and export result. Do not claim native file-picker interaction has been verified by this replay. Mixed-resource rotation/scale is newer and still awaiting acceptance. |

Curved-guides, mirror-shapes, and reference-export manifests mark caption timing checked. The source currently sets that flag automatically by clip type; a fresh recording must receive a fresh timing review rather than inherit this historical sign-off.

## What remains to finish the original 066 acceptance

- Record the 15–25 second overview. No overview capture mode or output was found; the CLI supports only the five focus names.
- Complete the five focused clips' action coverage above, then review full MP4/GIF playback, final frame/boundary behavior, captions, cursor cues, privacy, and readability on the intended surfaces.
- Record both 45–90 second tutorials. The house and lantern walkthroughs, narration and 60-second VTT drafts exist; tutorial footage does not. Synchronize captions to the eventual recordings and show save/reopen where promised.
- Add the planned Light/Dark comparison. Existing footage is Dark-only. SKETCH-056 has substantial software theme evidence, with OS/manual checks still open; identify exactly which software-verified state a comparison demonstrates.
- Resolve the 640px draft versus initial 960px README budget using actual embed review. Upscaling a draft does not add UI detail. Use a suitable fresh capture/preview or a poster-linked MP4 if the GIF is unreadable or over budget.
- Complete reproducibility and review metadata, then publish bounded curated assets or stable project-controlled links. The README currently offers two static tutorial rows, not a published five-clip gallery. Keep the written alternatives, posters/alt text and user-controlled video playback.

## Current functionality absent from the gallery plan

The original 066 does not require a clip for every feature. These are coverage additions to prioritize when presenting the current app, not extra blockers silently added to the original acceptance.

| Coverage to add | Tasks | Evidence/status and suggested shot |
| --- | --- | --- |
| Selection transforms and mixed resources | 057, 069; newer 081 | Verified non-uniform stroke transforms and mixed-resource translation are absent from the clips. Show select/move/undo on a stroke plus image/OBJ/guide selection. Rotation and uniform scale across mixed resources are implemented but still need runtime/persistence/export review. |
| Materials and image-plane drawing | 058, 060 | Software-verified, absent. Show material appearance and a bounded drawing tool on an image plane; do not imply the existing reference orbit demonstrates drawing on it. |
| Ring guide and curved mirror operation | 062, 063 | Software-verified. Show actual guide activation/drawing and the resulting reflected geometry, rather than only a prepared cylinder orbit. |
| Construction history and session replay | 064, 070 | Complete per task evidence, absent. Show record/pause/resume, trim/playback, and replay export; distinguish app-operation tutorials from artwork replay exports. |
| Drawing library and recovery | 065, 053 | Verified workflows, absent. Show thumbnails, trash/restore and reopening an editable drawing. |
| Expanded brushes and portable image tips | 071–078 | Implemented with acceptance still open. After visual/runtime/export checks, add a brush sampler: graphite/charcoal, stipple/spray, image stamps/foliage, bristle/rake, stationary airbrush buildup, and tube profiles. Include portable brush import separately. Pressure/tilt claims require physical evidence. |
| Reopened curves and snapping | 079, 080 | Implemented; interactive/save-reopen or large-scene acceptance remains. After checks, show editing a committed curve and grid/feature snap cues. |
| Layer opacity/blends and stroke gradients | 082–084 | Implemented; pixel/runtime/export acceptance remains. After checks, show per-stroke layer behavior and an arc-length color gradient. Do not describe layer blending as isolated group compositing. |
| Radial symmetry and linked groups | 085, 086 | Implemented; interactive/recovery/export acceptance remains. After checks, show planar brush/line radial copies, and a source-group edit propagating to linked instances. |
| Liquify | 067 | Synthetic software workflow verified; physical pressure acceptance remains. A disclosed software-only preview is possible; it cannot claim validated pen-pressure behavior. |

SKETCH-087/088 are feasibility work, not finished painting/tessellation features to advertise. SKETCH-089 corrects tablet documentation; future captions must preserve its unverified physical-device and Linux/macOS runtime qualifications.

## Tooling, fixtures, and documentation drift

- The two committed fixtures use native format **13**; the current writer uses **24**, and the loader accepts **1–24**. They are older baseline samples, not proof of incompatibility. Reopen/save/reload them in the selected recording build before tutorials. They do not exercise the newer brush, editable-curve, layer, gradient, or linked-group metadata. Keep format version distinct from the manifest's fixture revision (`fixture_version: 1`).
- Hidden-replay manifests omit SHA-256 hashes for every output, although the manual recorder/template includes them. Add hashes to future replay output sets for provenance. Existing byte-size agreement does not establish content identity.
- Replay `source_tree_dirty` checks only a named subset of source files. Newer geometry, brush, document, shader, and other dependencies can change without that flag noticing. Use full relevant-tree state and record the build identity when refreshing; `git rev-parse HEAD` at recording time alone cannot prove an already-built executable matches that commit.
- Three replay caption-review flags are hard-coded true by clip type. Reset review state on capture and record explicit review of each output set.
- The TASKS progress summary still said actual recordings were pending even though five focus drafts exist. That wording is corrected with this audit. The native-format summary there is historical and does not describe the current writer.

## Refresh rule and order

Before publication, capture the original five workflows and overview/tutorials on one selected build after their featured behavior passes its checks. Review them on the intended surfaces and sign off that exact output set. Preserve existing drafts as historical evidence rather than treating them as current footage.

Refresh a clip when its visible controls/shortcuts, input/geometry behavior, material rendering, fixture format/content, persistence/export result, theme/layout, or narration changes. Review shared UI changes across all clips; assess feature changes against the task links above. A new commit alone triggers comparison, not automatic invalidation. Add newer-feature clips after their acceptance checks pass, without delaying original 066 for unrelated unfinished features.
