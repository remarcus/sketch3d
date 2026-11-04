# SVG export

**File → Export → Active plane as SVG** projects visible artwork orthogonally onto the current drawing plane. The SVG uses world units and does not change with camera zoom or orbit. Visible layers are emitted in document order as named SVG groups, with layer opacity retained. Solid stroke and fill colors, fill opacity, and nested fill holes are preserved as vector paths.

SVG is a 2D format, so several Sketch3D features use documented approximations:

- Strokes are projected to the active plane. Their depth order is replaced by document layer order, so overlapping strokes may composite differently from the 3D canvas.
- Per-point width, pressure variation, brush-tip shape, textured coverage, dab scatter, and material lighting are represented by a round centerline stroke using the base width. Stroke gradients and locally painted colors are reduced to their mean color.
- Fill gradients and image textures are reduced to a mean fill color. Clipping masks and blend modes are not baked into SVG paths.
- Hidden layers are omitted. Visible locked layers are included. The export is capped at one million source points.

The output remains editable vector artwork; fallback behavior does not alter the native drawing.
