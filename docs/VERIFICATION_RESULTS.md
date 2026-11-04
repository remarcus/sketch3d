# Normalized verifier results

`scripts/run_verification.ps1` is the shared PowerShell 7 entry point for local runs and CI. It wraps the existing startup smoke, animation save/reopen, and still-export verifiers without replacing their original reports.

Build the release executable, then run the suite from the repository root:

```powershell
cargo build --locked --release
pwsh -File scripts/run_verification.ps1
```

Use `-Check startup`, `-Check save-reopen`, or `-Check export` to select checks. Supply `-Executable` when the release executable is elsewhere, `-OutputDirectory` to change the output root, or `-TimeoutSeconds` to change the per-check time limit. GitHub Actions calls this same script after its release build and uploads the result directory.

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

## Read results through local MCP

Start the opt-in read-only server with the normalized output root:

```powershell
target/release/sketch3d --mcp-verification-server verification/normalized
```

Configure an MCP host to launch that command as a stdio server. It supports the standard initialize/tools lifecycle for MCP protocol versions `2025-11-25` and `2025-06-18`, and exposes:

- `verification_list_checks`: lists the three supported check IDs and up to 100 completed run IDs.
- `verification_read_result`: returns one version 1 `result.json`.
- `verification_list_artifacts`: lists the artifact paths and sizes recorded for one check.
- `verification_read_artifact`: returns a recorded `.log` or `.txt` artifact as text, or a `.png` preview as an MCP image.
- `verification_run_check`: runs one allowlisted check using the shared PowerShell runner and returns its normalized result and artifact paths. `timeout_seconds` is optional and bounded from 5 to 600 seconds.

The server canonicalizes the configured root and each run/artifact path, accepts only direct-child run IDs made of ASCII letters, digits, hyphens and underscores, and rejects symlinked artifacts or paths outside their run directory. Artifact reads require a path recorded for the selected check. Text is limited to 256 KiB and must be UTF-8; PNG files are limited to 1,000,000 bytes and bounded image dimensions. Requests are capped at 64 KiB, results at 512 KiB, artifact records at 256, scanned run entries at 1,000, and protocol responses at 2 MiB. A run-check call is restricted to `startup`, `save-reopen`, and `export`, uses the runner's unique run directory, caps captured process output, and bounds check timeouts to 5–600 seconds. Protocol messages use stdout; startup and fatal errors use stderr.

## Inspect saved drawings through local MCP

Start a drawing-only server with an explicit native-file input directory and an existing GLB output directory:

```powershell
target/release/sketch3d --mcp-drawing-server drawings exports
```

The server exposes `drawing_inspect_native` and `drawing_export_glb`. Both accept a relative `.json` path beneath the configured input directory; absolute paths, traversal, and symlinks are rejected. Inspection returns bounded layer and guide summaries, stroke point bounds, and managed image/model asset requirements. Export reads and validates the same native file, resolves its managed `.assets` bundle, and writes a GLB named after the source into the configured output directory. Existing export names are left untouched. Native files are capped at 64 MiB; summaries are capped at 512 KiB and exports at 128 MiB. Configure this drawing-only mode separately from the verification report server.
