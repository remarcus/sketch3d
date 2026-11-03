# Curved guide construction

SKETCH-105 supports semantic circles, ellipses, and editable smooth curves on
finite cylinder guides. The cylinder is captured with each stroke, so edits
continue to use the original guide after selection changes or a document
reload. Other curved and model surfaces are outside this task's cylinder
scope.

## Cylinder chart

Controls use an unwrapped chart. Horizontal values are circumferential arc
lengths and vertical values are axial distances. A dragged circle measures a
Euclidean radius in that chart; a dragged ellipse uses opposite chart-space
corners. Typed circle radius and ellipse width/height use the same units.
Curve anchors and incoming/outgoing handles are chart positions and offsets.
Curve pressure stays attached to its anchor.

Input unwraps around the previous anchor or shape center, keeping seam
crossings continuous. Shape horizontal radius is limited to half the cylinder
circumference. Shapes and curve controls must stay within the finite axial
ends. Bezier handles outside the finite domain are rejected. Preview changes
remain separate from the drawing until Apply/Finish; Escape cancels them.

Shape strokes retain their kind, chart center, and chart radii alongside the
sampled outline, allowing the Circle/Ellipse tool to reopen the dimensions.
Curve strokes retain chart anchors, handles, tension, and the captured target.
Their samples use adaptive world-space tessellation at the current camera
tolerance, with bounded geometry. Samples lie on the captured cylinder and
use the standard stroke mesh for display, picking, erasing, and export.

Rigid and uniform positive transforms update both the sampled stroke and its
captured cylinder. Transforms that would change the analytic cylinder surface
are rejected with an explanation; users can convert/detach the stroke through
Reproject's explicit **Convert semantic curves to sampled strokes** option,
which retains the evaluated samples and drops the chart controls before the
transform is applied.
