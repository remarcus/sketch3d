# SKETCH-066 coverage and freshness audit

Audited 2026-11-02 against the current capture implementation and curated manifests from commit `65f46de`. All eight outputs decode end-to-end, VTT cues validate and match their action checkpoints, hashes/sizes/provenance match, and selected frames/posters have been visually reviewed. `captions_timing_checked`, `loop_boundary_checked`, and `private_content_checked` are recorded per manifest. Native-size motion readability and rendered hosted-README review remain open, so aggregate `reviewed` remains false.

The five focused clips, 18-second overview, and two 60-second tutorials now have bounded current-build replays. Each uses synthetic pressure-free input and discloses that no tablet pressure or latency was measured. The reference-export replay uses an injected generated file-picker result and does not show the native picker or insert-preview dialog.

## Existing clips

The five focused clips are six seconds at 10 fps with full-size MP4/posters and bounded 640×360 single-play GIFs. The overview is an 18-second 10-fps MP4; tutorials are 60-second 10-fps MP4s. Their six-second GIFs are accelerated previews, not normal-speed instruction. Every clip has a VTT and manifest; long tutorials include native sample files saved/reopened through production handlers. Caption timing, single-play behavior, privacy/path scans, and selected-frame/poster review are complete; native-size motion and hosted-surface review are not.

| Clip | Recorded app commit | What is actually captured | Gap and refresh action |
| --- | --- | --- | --- |
| `draw-orbit` | `65f46de` | Draw through `canvas_input`, then orbit | Current focused action clip. Synthetic input; no tablet measurements. |
| `curved-guides` | `65f46de` | Create and draw across a saved cylinder via production pointer handler; orbit | Cylinder guide/mirror action is shown. Ring guide remains outside this clip. |
| `mirror-shapes` | `65f46de` | Create a mirrored line and editable ellipse through production handlers; orbit | Radial symmetry remains outside this clip and must not be inferred. |
| `edit-erase` | `65f46de` | Select and recolor during capture; partially erase; inspect fragments; undo; orbit | Native selection/appearance/erase handlers are used. |
| `reference-export` | `65f46de` | Production OBJ Import control with injected path; native merge; whole-scene GLB export; orbit | The OS picker and insertion preview are not displayed. |
| `overview` | `65f46de` | Draw; select/move/undo/redo; orbit; show production GLB export options | 18-second overview, synthetic replay. |
| `house-tutorial` | `65f46de` | Draw; select/move/undo/redo; orbit; production native save/reopen | 60-second MP4, accelerated GIF preview, captioned and paired with saved sample. |
| `curved-tutorial` | `65f46de` | Draw/reflected stroke; select/move/undo/redo; orbit; native save/reopen | 60-second MP4, accelerated GIF preview, captioned and paired with saved cylinder sample. |

The eight VTT files pass ordering, overlap, and duration validation. Their cue ranges match the deterministic action checkpoints. Native-size playback readability and the exact hosted surface remain to be reviewed.

## What remains to finish the original 066 acceptance

- Review motion readability at native video resolution and inspect the final README rendering at its hosted surface; then update the aggregate `reviewed` and `intended_readme_surface_checked` flags for that exact output set.
- The native file picker and insertion preview are not shown in `reference-export`; keep this limitation in its caption/README description.
- Long-form GIFs are accelerated six-second previews. The README links the full MP4 and VTT for normal-speed tutorials and keeps static written alternatives available.
- Light/Dark comparison is optional and was removed from publication requirements by user direction (2026-11-02). Existing footage is Dark-only.
- The GIFs are 640px wide and under 5 MiB. Confirm that size on the hosted README; do not upscale if it makes UI text softer. Use poster-linked MP4 if the GIF is unreadable.

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
- `scripts/curate_capability_demos.ps1` copies bounded outputs into the local media folder, requires `source_tree_dirty=false`, and records SHA-256 hashes for clips, posters, captions, samples, and the scene GLB. Hashes identify exact files but are not a review sign-off.
- Replay `source_tree_dirty` checks the implementation source files that define the replays and production paths. `app_commit` records the build source revision; generate final assets from the committed build before curation.
- The TASKS progress summary still said actual recordings were pending even though five focus drafts exist. That wording is corrected with this audit. The native-format summary there is historical and does not describe the current writer.

## Refresh rule and order

Before publication, capture the original five workflows and overview/tutorials on one selected build after their featured behavior passes its checks. Review them on the intended surfaces and sign off that exact output set. Preserve existing drafts as historical evidence rather than treating them as current footage.

Refresh a clip when its visible controls/shortcuts, input/geometry behavior, material rendering, fixture format/content, persistence/export result, theme/layout, or narration changes. Review shared UI changes across all clips; assess feature changes against the task links above. A new commit alone triggers comparison, not automatic invalidation. Add newer-feature clips after their acceptance checks pass, without delaying original 066 for unrelated unfinished features.
