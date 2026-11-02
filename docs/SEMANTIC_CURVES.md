# Editable semantic curves

Smooth curves retain their editable anchors, pressure values, tangent offsets,
captured drawing frame, and tessellated stroke. Reopening a curve uses those
stored controls; it never reconstructs handles from the sampled mesh. A legacy
sampled stroke without curve metadata remains ordinary ink and does not gain
invented edit handles.

The captured frame is a snapshot of the plane or image bounds used to create
the curve. There is no live link to a guide or image plane. Moving, hiding,
locking, or deleting that source leaves the curve in world space. Selecting
another guide affects only future input.

Selection transforms update the anchors, tangent vectors, frame, and sampled
points together. Liquify, mirroring, and partial erasing produce ordinary
sampled ink and clear semantic curve metadata because those operations change
the original parameterization. Native save/reopen, merge, recovery, undo, and
redo preserve valid controls. Each curve is limited to 256 anchors and 4,096
tessellated points.

Run `cargo run --locked -- --verify-image-planes` to exercise commit, save and
reopen, exact anchor/tangent restoration, selected-curve editing, Apply,
Escape, focus-loss cancellation, undo/redo, native merge, recovery, and
transform preservation. The same verifier covers image-plane curve input.
