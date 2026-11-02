# Performance benchmarks

Run `cargo run --release -- --benchmark-performance` from the repository root. It writes `verification/performance-benchmark.json`.

The report includes the OS, architecture, available logical CPU count, workload sizes, and seven measured iterations after one warmup. Each timing is synchronous CPU wall-clock time on the local machine; use the median for comparisons. Run the same command on an otherwise idle machine and compare reports from the same release build and machine when evaluating a change.

Workloads measure production functions for tessellating one 10,000-point stroke, sampling and rebuilding a 4,096-point long dab stroke, tessellating 1,000 short strokes, artwork picking among 1,000 strokes, and large selection snapshot, bounds, transform, and apply/undo operations over 500 selected strokes in a 1,000-stroke document. Workload geometry is generated deterministically. The dab case records sampler time separately from rebuilding the active stroke mesh. These cases identify CPU paths that merit profiling; they do not reproduce every drawing session.

The SKETCH-071 reference run (Windows x86_64, a local benchmark CPU, release profile, 2026-11-02) measured a median 0.476 ms to resolve 8,186 dabs and 3.361 ms to build their 196,464-vertex mesh. Each median comes from seven measured iterations after one warmup. The fixture parameters are checked in; complete sample distributions are written to `verification/performance-benchmark.json` when the benchmark runs.

This benchmark does not issue GPU timestamp queries or measure GPU buffer upload, draw calls, GPU frame time, native pen-event handling, or end-to-end pen latency. The report states these limits. Do not treat its wall-clock times as GPU or device-latency measurements.

The existing `--benchmark-textures` command separately measures CPU tessellation and procedural brush-mask PNG generation for 10,000-point strokes. Its report is `verification/texture-benchmark.json`.
