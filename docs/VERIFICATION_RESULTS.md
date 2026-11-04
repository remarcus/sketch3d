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
