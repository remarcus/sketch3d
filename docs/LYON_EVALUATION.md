# Lyon planar tessellation evaluation

This study evaluates `lyon_tessellation` 1.0.23 in an opt-in example. It does
not change or call the production geometry path. Run it with:

```powershell
cargo run --locked --example lyon_evaluation --features lyon-evaluation -- verification/lyon-evaluation
python scripts/verify_lyon_evaluation_blender.py verification/lyon-evaluation/rotated_concave.obj verification/lyon-evaluation/blender-import-report.json
```

The example bounds inputs to 256 points and outputs to 65,535 vertices and
196,605 indices. It measures convex and concave fills, a bow-tie, a double-wound
contour under NonZero and EvenOdd rules, sharp miter and bevel joins, variable
width strokes, and a guide-local contour mapped to an inclined world plane. It
writes 128, 512, and 2,048 pixel comparison images, a JSON report, and an OBJ
fixture for independent Blender import. Repetitions, limits, CPU medians,
triangle counts, pixel differences, and allocator estimates are in
`verification/lyon-evaluation/report.json`.

On the recorded Windows debug run, convex and concave simple-fill masks matched
the local ear-clip reference with zero differing pixels at all three sizes.
Their medians were 6.2 and 7.7 microseconds for Lyon, compared with 0.8 and 1.7
microseconds for the reference. Lyon reported 26 / 8,107 and 31 / 8,989 median
allocation calls / cumulative requested bytes per invocation; the reference
reported 4 / 112 and 5 / 208. These allocation numbers are approximate,
adapter-inclusive, count requested allocation sizes, and do not represent
retained memory. Times are local microbenchmarks, not app-level performance
predictions.

For a twice-wound square, EvenOdd emits zero triangles while NonZero emits two;
the masks differ by 11,664, 184,900, and 2,958,400 pixels at the three tested
resolutions. The current ear-clip path rejects self-intersections, so this is a
semantic extension rather than a drop-in behavior match. The inclined-plane
mapping had a maximum measured coordinate error of about `1.34e-7`. Blender
5.2.3 independently imported the exported OBJ as six finite vertices and four
triangle faces with nonzero Z extent, confirming that the plane remained
inclined.

**Decision:** keep current production geometry and defer Lyon adoption. The
simple-contour result is visually equivalent on these fixtures, while the
adapter costs more CPU and allocations and fill-rule choices change behavior
for self-intersections. A production integration should be proposed separately
for a concrete feature that needs holes or configurable fill rules.
