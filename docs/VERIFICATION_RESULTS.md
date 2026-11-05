# Normalized verifier results

`scripts/run_verification.ps1` is the shared PowerShell 7 entry point for local runs and CI. It wraps the existing startup smoke, animation save/reopen, and still-export verifiers without replacing their original reports.

Build the release executable, then run the suite from the repository root:

```powershell
cargo build --locked --release
pwsh -File scripts/run_verification.ps1
```

Use `-Check startup`, `-Check save-reopen`, `-Check export`, or `-Check glb-texture-roundtrip` to select checks. Supply `-Executable` when the release executable is elsewhere, `-OutputDirectory` to change the output root, or `-TimeoutSeconds` to change the per-check time limit. GitHub Actions calls this same script after its release build and uploads the result directory.

Each invocation creates a unique run directory. `result.json` uses schema version 1 and contains one record per check:

```json
{
  "schema_version": 1,
  "run_id": "...",
  "generated_utc": "...",
  "results": [
    {
      "check_id": "startup",
      "outcome": "passed",
      "failure_reason": null,
      "exit_code": 0,
      "artifacts": ["startup.stdout.log", "startup.stderr.log"]
    }
  ]
}
```

Outcomes are `passed`, `failed`, `unsupported`, or `timed_out`. Artifact paths are relative to the run directory. Original verifier reports and generated images are retained alongside captured stdout and stderr. Unsupported environments do not fail the suite; failures and timeouts return a nonzero process exit code.

New normalized reports may include top-level `build` and `source_revision` objects and per-check `fixture` and `rendering` objects. Build identity records the executable's product/file version and SHA-256 fingerprint; source revision is the Git commit when available. Each check has a stable fixture ID and, when a fixture source file exists, its SHA-256 fingerprint. Export rendering metadata contains the report's pixel resolution only. Missing metadata is represented by `null` values plus an explicit availability status. These fields are optional, so older schema-version-1 results remain readable. MCP-synthesized results use the MCP binary's package version and mark unavailable fingerprints/revision explicitly.

## Read results through local MCP

Start the report-only server with the normalized output root:

```powershell
target/release/sketch3d --mcp-verification-server verification/normalized
```

Configure an MCP host to launch that command as a stdio server. It supports the standard initialize/tools lifecycle for MCP protocol versions `2025-11-25` and `2025-06-18`, and exposes:

- `verification_list_checks`: lists the four supported check IDs and up to 100 completed run IDs.
- `verification_read_result`: returns one version 1 `result.json`.
- `verification_list_artifacts`: lists the artifact paths and sizes recorded for one check.
- `verification_read_artifact`: returns a recorded `.log` or `.txt` artifact as text, or a `.png` preview as an MCP image.

This mode exposes report-reading tools only. It cannot start verifier processes, including if a client calls `verification_run_check` by name.

To explicitly enable the allowlisted execution tool, launch a separate MCP configuration with:

```powershell
target/release/sketch3d --mcp-verification-execution-server verification/normalized
```

Execution mode adds `verification_run_check`, which accepts the supported check IDs, uses a unique run directory, caps captured process output, and bounds `timeout_seconds` from 5 to 600 seconds. `glb-texture-roundtrip` runs natively through the same release executable hosting MCP; other checks use the shared PowerShell runner. Both modes canonicalize the configured root and run/artifact paths, accept only direct-child run IDs made of ASCII letters, digits, hyphens and underscores, and reject symlinked artifacts or paths outside their run directory. Artifact reads require a path recorded for the selected check. Text is limited to 256 KiB and must be UTF-8; PNG files are limited to 1,000,000 bytes and bounded image dimensions. Requests are capped at 64 KiB, results at 512 KiB, artifact records at 256, scanned run entries at 1,000, and protocol responses at 2 MiB. Protocol messages use stdout; startup and fatal errors use stderr.

## GLB texture round-trip check

Select `glb-texture-roundtrip` through the CLI runner or MCP execution tool. Its deterministic `textured-glb-managed-native-roundtrip-v1` fixture exercises textured GLB import, managed asset bundling, native JSON save/reopen, and whole-scene GLB export. The verifier checks texture metadata and requires exact exported PNG pixels. It writes `glb-texture-roundtrip-report.json`, `texture-preview.png`, the native drawing, and exported GLB into the original check artifact folder. This is structural texture evidence; it does not replace viewport visual review, recovery coverage, WebP extension fixtures, or platform testing.

## Inspect saved drawings through local MCP

Start a drawing-only server with an explicit native-file input directory and an existing GLB output directory:

```powershell
target/release/sketch3d --mcp-drawing-server drawings exports
```

The server exposes `drawing_inspect_native` and `drawing_export_glb`. Both accept a relative `.json` path beneath the configured input directory; absolute paths, traversal, and symlinks are rejected. Inspection returns bounded layer and guide summaries, stroke point bounds, and managed image/model asset requirements. Export reads and validates the same native file, resolves its managed `.assets` bundle, and writes a GLB named after the source into the configured output directory. Existing export names are left untouched. Native files are capped at 64 MiB; summaries are capped at 512 KiB and exports at 128 MiB.

The `drawing_create_native_copy` tool accepts a typed batch of one planar guide, one layer, and supported line, polyline, or rectangle strokes. Stroke coordinates are two-dimensional points local to the guide; the guide origin is world-space and its rotation is an x/y/z/w quaternion. Stroke colors are linear RGB. Batches are capped at 64 commands, 60 strokes and 2,048 points. The tool validates the complete document before atomically creating a new `.json` file in the configured output directory; it refuses existing names and does not modify source drawings. Configure this drawing-only mode separately from the verification report server.

Example batch:

```json
{
  "output_name": "quick-study.json",
  "commands": [
    {"type":"create_plane","name":"Front","origin":[0,0,0],"rotation":[0,0,0,1]},
    {"type":"create_layer","name":"Ink"},
    {"type":"line","start":[-1,0],"end":[1,0],"color":[0.05,0.05,0.05],"radius":0.025},
    {"type":"rectangle","min":[-0.5,-0.5],"max":[0.5,0.5]}
  ]
}
```
