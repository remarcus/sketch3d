# OBJ model line art

Open **OBJ references** and select a visible model, then use **Preview line art**.
The preview finds open boundaries, creases at or above the selected face-angle
threshold, and view-dependent silhouettes. It checks the current camera and
visible OBJ references for occlusion.

Preview geometry stays separate from the drawing. **Update preview** recomputes
it from the source model, its current transform, and the current camera. **Cancel**
discards the preview. **Bake** adds the preview as ordinary independent strokes
in one undo step. After baking, removing the source OBJ does not remove those
strokes.

The extractor is deterministic and bounded to 250,000 input triangles,
750,000 candidate edges, 20,000 output strokes, and 40,000 output points. A
preview that reaches the stroke limit is marked incomplete. Native files,
recovery snapshots, and stroke exports store baked line art as ordinary strokes;
the detached preview is not persisted or exported.

Automated checks cover deterministic open, curved, and cube meshes; silhouette
changes across cameras; front/back and hidden-model occlusion; extraction limits
and a 2,048-triangle timing fixture. Baked strokes are also round-tripped through
native serialization and recovery snapshots, then checked in parsed GLB and OBJ
exports.
