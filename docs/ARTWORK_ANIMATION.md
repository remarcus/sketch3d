# Artwork animation

Artwork animation is a separate, bounded keyframe track stored in the native
document. Version 1 stores an ordered stroke snapshot per integer frame (up to
120 keyframes, frame 100,000, 100,000 strokes per keyframe, and 2,000,000
points across the track). Playback is deterministic at the saved FPS and holds
the preceding keyframe by default. Optional interpolation blends position and
pressure only when stroke identity, point topology, timing, appearance, and
brush settings match. Semantic curves are held because their anchor metadata
must move with their endpoints. Incompatible keyframes are held rather than
partially blended.

The track FPS is 1–60. The animation toolbar's Set, Duplicate, Delete, and setting changes each make
one undoable animation edit. Editing the current pose is an ordinary frame-local
edit; Set keyframe captures that pose. Seeking and playback update the displayed
pose and frame without creating edits, and clear frame-local undo/redo so an
undo cannot cross into a different pose. Save/export requires the displayed
pose to match its current frame's resolved keyframe pose; set/update the
keyframe first. FPS, range, interpolation, and onion-skin changes are separate
undoable edits. Layer moves update every stored pose, and a layer used by any
keyframe cannot be deleted until its strokes are moved or removed.

Onion skins are translucent viewport overlays. They do not enter document
strokes, hit testing, selection, undo, still exports, or animated exports.
Linked-group definitions and instances are rejected when setting or duplicating
animation keyframes; make instances unique first. This keeps source/generated
stroke ownership unambiguous in every stored pose.

Animated GIF/MP4 export renders the saved inclusive integer frame range at the
track FPS from a detached document copy. MP4 supports the full 1–60 FPS range;
GIF supports up to 50 FPS and rounds each frame delay to its 10 ms timing grid,
with the effective rate shown in the export dialog. It excludes onion skins and other
editor overlays, and does not change the open editor document. Native merge
imports only the source document's displayed/current stroke pose as static
artwork; its animation track is not merged. Open the source document separately
to continue editing its animation.
