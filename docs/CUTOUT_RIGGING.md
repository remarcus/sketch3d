# Two-bone cutout prototype

The timeline's **Create two-bone cutout prototype** action binds the active layer's current keyed artwork. **Root °** rotates the whole layer around the generated lower pivot; **Child °** bends the weighted distal portion of each stroke. **Key pose** stores the angles at the displayed frame. Pose angles interpolate linearly between keys, and seeking resolves the rig from the saved source strokes. The rigged layer is protected from direct edits; remove the prototype to restore the source pose.

This is a deliberately small prototype for cutout-style strokes. It creates one vertical two-bone rig for one layer. Stroke points receive a smooth child-bone weight based on normalized distance along each stroke, with the transition centered near 42–58% of the stroke. A root Z rotation affects the entire layer; the child rotation affects the weighted distal samples. Pressure, timing, color, brush settings, thickness, and other stroke attributes remain in their source records.

## Blender handoff

The editable Grease Pencil script exports the unposed source centerlines, per-sample vertex-group weights, two armature bones, and an action containing the rig pose keys. This is still a Blender script handoff and requires Blender 4.3 or newer. Other source semantics retain the existing handoff report.

## Native storage cost and limits

Rig data is optional in the versioned animation object, so existing drawings load without migration. The rig adds one JSON weight number per bound sample, two 3D pivot positions, one stroke ID per binding, and one frame number plus two angles per pose key. It does not duplicate the source strokes. The current prototype caps weights at 262,144 points, bindings at 100,000 strokes, and pose keys at 120. It supports one rigged layer per drawing and requires bound strokes to retain their point topology.

The prototype does not provide arbitrary bone chains, editable weight painting, or rig-preserving layer transforms. Blender playback and deformation review remain separate acceptance work.
