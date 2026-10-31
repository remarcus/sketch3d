# Tessellation optimization comparison

Recorded on the same Windows x86_64 machine, with the release profile and the same deterministic benchmark workloads. Each row is the median of seven timed iterations after one warmup. Values are CPU wall-clock milliseconds; they are not GPU timings or pen latency.

| Workload | Before | After | Change |
| --- | ---: | ---: | ---: |
| One round stroke, 10,000 points | 12.4455 | 6.7870 | 45.4% faster |
| 1,000 round strokes, 16 points each | 5.3360 | 4.5049 | 15.6% faster |

The optimization stores each eight-vertex tube ring in a fixed-size array instead of allocating a separate `Vec` for every input point. It also reserves the known triangle output size once. The output remains 480,000 vertices for the long stroke and 768,000 vertices for the many-short-strokes workload, matching the baseline counts. This run does not measure GPU buffer uploads, draw calls, GPU frame pacing, or native pen latency.

The complete post-change samples and selection/picking workload timings are emitted to `verification/performance-benchmark.json`. The benchmark command and its measurement boundaries are documented in `PERFORMANCE_BENCHMARKS.md`.
