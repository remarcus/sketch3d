# Deterministic dab placement

SKETCH-071 version 1 uses a stable world-space sampler alongside the existing
continuous stroke representation. Existing brushes keep the continuous path;
dab strokes render from the sampler and use the same resulting mesh for canvas
display, picking, selection bounds, GLB, and OBJ export.

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
- Collinear re-sampling of the same path produces identical centers and random
  values. For paths that differ geometrically, center error is bounded by the
  maximum centerline deviation plus `1e-5` world units.
- Scatter is uniform in area in the captured guide plane, bounded by
  `stroke radius × scatter`. Input rejects scatter on curved guides and model
  surfaces, where an offset would move the center off the hit surface. With
  scatter set to zero, curved-guide seams and segmented misses retain their
  existing surface sampling behavior; missed runs are separate strokes and do
  not bridge. Model-surface dabs remain on their sampled mesh path.
- Size and angle variation, pressure response, and taper are resolved from the
  saved stroke. The random stream is SplitMix64 keyed by the stroke seed, dab
  index, and value channel, so editing a preset cannot change old artwork.
- Partial erase stores each retained fragment's original arc-distance offset
  and full length. The global dab phase, random values, and taper survive a
  split. Rigid transforms preserve the phase; uniform scaling scales its
  distance values with the drawing.
- Stipple and spray presets use this same saved geometry path. Stipple uses
  regular spacing with no scatter; spray uses a bounded uniform-area offset in
  the captured plane. Scatter is rejected for curved guides and model targets,
  so spray does not detach marks from those surfaces. Taps remain one dab, and
  dabs are ordinary selectable, erasable, transformable, and exportable mesh
  geometry.

## Versioning and budgets

SKETCH-071 introduced native schema version 14. Current drawings use schema
version 17, which adds the resolved image-tip aspect ratio used by stamp
geometry. Version 16 added bounded embedded brush-tip PNG assets; version 15
added captured tilt magnitude for tilt-driven shading. Missing
dab settings in earlier versions default to disabled; their continuous geometry
and appearance remain unchanged.
Each dab settings record uses algorithm version 1. Unknown algorithm versions
are rejected by document validation instead of being regenerated differently.

The current limits are 8,192 dabs per stroke and 50,000 dabs per document. A
round dab contributes 24 triangle vertices, so the per-stroke maximum is under
the existing two-million-vertex export limit. Scatter, spacing, size variation,
angle variation, and pressure-size controls are bounded and validated before
serialization or rendering.

Performance acceptance remains open until a reproducible long-stroke sampler
and active-preview benchmark is recorded. Synthetic input does not establish
physical tablet behavior.
