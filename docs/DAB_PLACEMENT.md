# Deterministic dab placement

SKETCH-071 version 1 uses a stable world-space sampler alongside the existing
continuous stroke representation. Existing brushes keep the continuous path;
dab strokes render from the sampler and use its resolved mesh for canvas
display and GLB/OBJ export. Picking uses conservative stroke extents as a broad
phase, then ray-tests the visible dab mesh so gaps between marks can reveal ink
behind them. Selection bounds use a conservative extent and may include empty
space between dabs.

## Placement rules

- Spacing is `stroke radius × spacing`, in world units. It does not depend on
  pointer frequency, elapsed time, camera zoom, or the number of input samples.
- The sampler places dab indices at global distances `0, step, 2 × step, …`.
  A tap is one dab. A short stroke with no later distance landmark stays one
  dab. Pen-up does not snap an extra dab to the endpoint; stabilized release
  samples extend the centerline and therefore may reach another normal spacing
  landmark.
- Dab centers interpolate the saved world-space points by arc length. At a
  sharp corner, tangent orientation uses the normalized incoming/outgoing
  bisector. At an exact reversal, it keeps the incoming tangent.
- The dab frame projects the stroke's captured guide normal against the local
  path tangent, then transports the brush angle in that guide-relative frame.
- Re-sampling the same piecewise-linear path preserves centers, pressure
  response, and indexed random values within `1e-5` world units for centers and
  `1e-5` for scalar values. At corners the sampler uses the incoming/outgoing
  bisector; at an exact reversal it keeps the incoming tangent. Consecutive
  duplicate samples do not add placements. A tap gets one dab, short strokes do
  not force an endpoint dab, and stabilized release points add only ordinary
  spacing landmarks that they cross.
- Zero-scatter curved-guide and model-surface paths interpolate the captured
  world-space centerline. This is chordal interpolation between saved samples,
  not geodesic travel; denser stabilized samples reduce chord error. Surface
  misses and large discontinuities create separate strokes, so dabs do not
  bridge an absent surface region.
- On analytic curved guides, scatter angle and radius are sampled with the
  existing seeded uniform-area rule, then mapped through the captured surface's
  local metric. Dab orientation follows the local surface normal and projected
  path tangent. Centers are reprojected from chordal interpolation onto the
  captured surface. A placement is omitted if it leaves a finite guide domain,
  reaches a singular frame, or turns the surface normal by more than 60 degrees.
- PNG image stamps on analytic guides are surface-projected tangent decals:
  the four quad corners are mapped onto the guide and the usual two triangles
  connect those corners. This has zero depth offset and does not subdivide the
  interior of the quad into a conforming curved mesh. A stamp is omitted when
  any corner leaves the finite domain, has a singular frame, or exceeds the
  60-degree normal-turn limit. Periodic parameterization carries placements
  across cylinder, sphere-longitude, and torus seams without a world-space
  seam jump.
- Dab-based brushes on analytic guides use an eight-sector tangent decal. Its
  center and perimeter are projected to the guide and its triangles follow the
  local surface normals. It has no depth offset; the existing 24-vertex-per-dab
  budget is unchanged. Marks are omitted if any perimeter point is outside the
  finite guide domain, has a singular frame, or exceeds the normal-turn limit.
- Saved cylinder, sphere, torus, cone, revolved, freeform, and loft guides are
  captured as validated surface snapshots in dab settings. Spacing remains
  measured along the saved world-space centerline; scatter and projection use
  the surface-local metric. Translation, rotation, and uniform selection scale
  transform both the stroke and its captured guide. Non-uniform/reflected
  selection transforms are rejected for these marks, and mirror-copy is
  unavailable, so the stored surface cannot silently detach from the stroke.
  Planar output is unchanged. Mesh-target image tips
  and scatter remain disabled until strokes can retain per-contact triangle
  data and use it consistently for rendering, picking, edits, and export.
- Size and angle variation, pressure response, and taper are resolved from the
  saved stroke. The random stream is SplitMix64 keyed by the stroke seed, dab
  index, and value channel, so editing a preset cannot change old artwork.
- Partial erase stores each retained fragment's original arc-distance offset
  and full length. The global dab phase, random values, and taper survive a
  split. Rigid transforms preserve the phase; uniform scaling scales its
  distance values with the drawing. Partial erasing projects the resolved dab
  marks and tests their visible footprints, then removes the spacing cell for
  each hit dab. Empty gaps are not erased merely because the conservative
  scatter bound overlaps the gesture.
- Stipple and spray presets use this same saved geometry path. Stipple uses
  regular spacing with no scatter; spray uses a bounded uniform-area offset in
  the captured plane or analytic surface. Model-target scatter remains disabled.
  Taps remain one dab, and
  dabs are ordinary selectable, erasable, transformable, and exportable mesh
  geometry.

## Versioning and budgets

SKETCH-071 introduced native schema version 14. Current drawings use schema
version 20, which adds bounded tube profiles. Version 19 added timestamped
airbrush flow settings. Version 18 added bounded bristle/rake strand settings.
Version 17 added
the resolved image-tip aspect ratio used by stamp geometry; version 16 added
bounded embedded brush-tip PNG assets; version 15 added captured tilt magnitude
for tilt-driven shading. Missing
dab settings in earlier versions default to disabled; their continuous geometry
and appearance remain unchanged.
Each dab settings record uses algorithm version 1. Unknown algorithm versions
are rejected by document validation instead of being regenerated differently.

The current limits are 8,192 dabs per stroke and 50,000 dabs per document. A
round dab contributes 24 triangle vertices, so the per-stroke maximum is under
the existing two-million-vertex export limit. Scatter, spacing, size variation,
angle variation, and pressure-size controls are bounded and validated before
serialization or rendering.

The reproducible release benchmark uses 4,096 evenly spaced path points over
57.3 world units, radius `0.02`, and spacing `0.35` radii. On the recorded
Windows x86_64 machine with a local benchmark CPU, the median sampler time was 0.476
ms for 8,186 dabs; rebuilding the active stroke mesh (196,464 vertices) took
3.361 ms. The 7 measured iterations follow one warmup. This is CPU geometry
time only: it excludes GPU upload, drawing, repaint scheduling, native pen
events, and end-to-end input latency. Synthetic input does not establish
physical tablet behavior.

## Bristle and rake geometry

Parallel strands are part of one saved stroke and are regenerated from the
captured world-space brush frames. Each lane is a capped six-sided ribbon-like
tube with pressure-controlled width/opacity and optional tilt-driven fan
expansion. Missing tilt leaves the saved spread unchanged. Partial erase splits
the one stroke into ordinary editable fragments; each fragment retains the same
strand settings and derives its frame from its remaining points. The maximum is
16 strands and 400,000 generated vertices per stroke. The upstream review used
Open Brush at commit `c762836af3bc08a6370a27c372b1343f91ada7be`: [GeometryBrush.cs](https://github.com/icosa-foundation/open-brush/blob/c762836af3bc08a6370a27c372b1343f91ada7be/Assets/Scripts/Brushes/GeometryBrush.cs), [QuadStripBrush.cs](https://github.com/icosa-foundation/open-brush/blob/c762836af3bc08a6370a27c372b1343f91ada7be/Assets/Scripts/Brushes/QuadStripBrush.cs), and [BrushDescriptor.cs](https://github.com/icosa-foundation/open-brush/blob/c762836af3bc08a6370a27c372b1343f91ada7be/Assets/Scripts/Brushes/BrushDescriptor.cs). No upstream code or assets were copied, so no third-party notice was needed; Open Brush's Unity-oriented geometry was not a direct fit for this stroke representation.

The screen-space eraser treats the fan's full envelope as stroke coverage: a
partial erase splits the logical stroke when it intersects that envelope, and
removes all strands over the cut interval. Individual lanes and gaps are not
edited independently.

## Airbrush flow

SKETCH-077 adds flow to planar, curved-guide, and model-surface strokes. The
stroke stores point timestamps in milliseconds and a resolved flow rate and
algorithm version, enough to regenerate the same mesh on reload.
The sampler interpolates the captured path into 32 ms time cells and subdivides
each cell spatially at half the stroke radius. Each deposit contributes
`1 - exp(-rate × pressure × sample duration)` alpha; overlapping deposits
therefore accumulate independently of renderer repaint count. A stationary
contact is recorded on a timer until contact ends. Mouse input uses the existing
fixed pressure fallback.

The per-stroke limits are 65,536 timestamped points, 8,192 deposits, 589,824
vertices, and five minutes of contact. Picking and GLB use interpolated vertex
alpha as coverage. OBJ/MTL has no per-vertex alpha support, so it keeps deposit
geometry and base material opacity while losing soft accumulation detail.
Partial erasing keeps interpolated timestamps on retained points; it does not
rerun a preset or replace captured pressures.

## Tube profiles

SKETCH-078 adds square, triangular, and five-point star cross-sections to the
round tube brush. Profile geometry uses the captured guide frame and manual
angle; optional tilt orientation uses reported pen lean and falls back to that
manual frame when tilt is missing or upright. Pressure width, opacity, and
distance taper share the existing brush response settings. Flat profile faces
have face normals and the open ends use triangulated caps. A dot is a short
extruded profile with two caps. Coincident samples collapse before tessellation;
the maximum generated profile mesh is one million vertices per stroke.

The standard round profile keeps its original eight-sided geometry. Profiled
stroke selection, screen-space erasing bounds, surface picking, and GLB/OBJ
exports all consume the shared profile mesh. Non-uniform scaling stores separate
width, depth, and dot-axis adjustments so the generated profile tracks the
transformed cross-section.
