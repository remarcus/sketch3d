# Curved guide construction

SKETCH-105 starts with circles and ellipses on finite cylinder guides. Their
dimensions use an unwrapped chart: horizontal values are circumferential arc
lengths and vertical values are axial distances. A dragged circle measures the
Euclidean radius in that chart; a dragged ellipse uses opposite chart-space
corners. Typed circle radius and ellipse width/height use the same units.

The chart unwraps input around the selected center, so a shape can cross the
angular seam without a world-space discontinuity. Shapes may not exceed half
the cylinder circumference in horizontal radius, or extend past either finite
axial end. The outline is tessellated adaptively from the current camera's
world-per-pixel tolerance and bounded to 24–512 segments. Generated samples lie
on the captured cylinder surface.

This first slice stores a sampled editable outline, like existing planar
shapes. Captured semantic dimensions, reopening shape controls, and editable
curves on cylinders remain in progress; those will be added before SKETCH-105
is complete. Sphere, torus, freeform, loft, and model targets remain outside
this task's initial cylinder scope.
