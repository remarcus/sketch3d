# Performance benchmarks

Run `cargo run --release -- --benchmark-performance` from the repository root. It writes `verification/performance-benchmark.json`.

The report includes the OS, architecture, available logical CPU count, workload sizes, and seven measured iterations after one warmup. Each timing is synchronous CPU wall-clock time on the local machine; use the median for comparisons. Run the same command on an otherwise idle machine and compare reports from the same release build and machine when evaluating a change.

Workloads measure production functions for tessellating one 10,000-point stroke, tessellating 1,000 short strokes, artwork picking among 1,000 strokes, and large selection snapshot, bounds, transform, and apply/undo operations over 500 selected strokes in a 1,000-stroke document. Workload geometry is generated deterministically. These cases are intended to identify CPU paths that merit profiling; they do not reproduce every drawing session.

This benchmark does not issue GPU timestamp queries or measure GPU buffer upload, draw calls, GPU frame time, native pen-event handling, or end-to-end pen latency. The report states these limits. Do not treat its wall-clock times as GPU or device-latency measurements.

The existing `--benchmark-textures` command separately measures CPU tessellation and procedural brush-mask PNG generation for 10,000-point strokes. Its report is `verification/texture-benchmark.json`.
