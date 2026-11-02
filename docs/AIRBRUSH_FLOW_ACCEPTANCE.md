# Airbrush flow acceptance

The hidden `--verify-dynamics` workflow now runs an Airbrush acceptance pass
through the production canvas input, timer, geometry, picking, native document,
GLB, OBJ/MTL, and GPU renderer paths. Its deterministic report and generated
fixtures are written beneath `verification/airbrush-flow/run-*`.

The acceptance compares the same 256 ms pressure-ramped gesture captured at
15 Hz, 30 Hz, and 125 Hz. It also compares a 1,024 ms stationary hold recorded
at 30 Hz and 8 Hz. Both comparisons require equal deposit counts and nearly
identical generated vertices. A duration sweep verifies that center coverage
increases through the stationary hold and stays within the opacity range.
The app-driven checks send synthetic pen samples through `canvas_input`, test
the 32 ms stationary timer, and stop or cancel contact through hover, pen-up,
barrel navigation, cancellation, and focus loss. Missing pen pressure is
verified through the canvas path and uses the documented mouse fallback of
1.0.

The fixture exercises opacity-aware picking, a near-transparent picking miss,
native version 24 save/reload, byte-identical GLB re-encoding, GLB blended
alpha, and OBJ mesh size. The OBJ MTL file carries the exporter's note that
OBJ/MTL cannot preserve per-vertex soft opacity. Front and orbit GPU renders of
two overlapping flow marks are saved with the report. The verifier also checks
the limits of 65,536 stored points, 8,192 deposits, and 589,824 generated
vertices per stroke, and records mesh build time for a large-radius fixture.

Run it with:

```powershell
$env:TEMP = (Resolve-Path verification\tmp).Path
$env:TMP = $env:TEMP
cargo run --release --locked -- --verify-dynamics
```

This scripted path does not replace testing with a physical pressure-sensitive
pen. It verifies timestamped sample invariance and recorded pressure behavior;
the user-facing pressure source remains dependent on the active input device.
