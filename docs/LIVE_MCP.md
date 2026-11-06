# Live session MCP

Start Sketch3D with `--mcp-live-app-server` from an MCP client. This opt-in mode
opens the desktop app and exposes a local stdio MCP server in the same process.
Keep stdin and stdout connected to the MCP client; diagnostics go to stderr.

The server exposes read-only inspection and explicitly previewed changes:

- `live_document_snapshot` reports the active file's base name, whether it is
  saved, whether it has unsaved changes, bounded document counts, and selected
  stroke IDs. It does not return artwork data or absolute file paths.
- `live_scene_outline` returns bounded pages of stable layer and saved-guide
  IDs and names, effective layer visibility and lock state (including folder
  state), guide type and state, and the active layer and guide IDs. The first
  request omits `cursor`; follow `next_cursor` to read further pages. Each
  cursor is bound to the document revision and is rejected after a scene
  change. Names are bounded, and the outline omits geometry and file paths.
- `live_viewport_capture` asks the UI thread for a screenshot and returns a PNG.
  Captures are limited to 4096 × 4096 pixels and 1,000,000 encoded bytes.
- `live_stroke_index` lists up to 100 stroke summaries per page for a layer
  ID from the scene outline. It reports stroke IDs, kinds, world bounds, and
  color and move eligibility. Follow `next_cursor` with the same layer.
- `live_preview_selection` takes 1 to 100 unique stroke IDs and the revision
  from inspection. It highlights the proposed replacement selection. Apply
  returns the revision for subsequent color or move previews. Cancel keeps
  the prior selection. Selection changes leave artwork history intact.
- `live_preview_selected_color` previews a solid RGB color on the selected
  ordinary strokes, using the current document revision from the snapshot.
  Protected or missing strokes, active gestures, and stale revisions are
  rejected. While the preview is open, a modal locks drawing and editing.
- `live_preview_selected_move` previews a world-space XYZ translation for
  selected independent editable strokes.
- `live_apply_edit` and `live_cancel_edit` act on the matching preview ID.
  Apply also requires the source revision. Artwork changes create one undo
  step. The preview can also be applied or cancelled in the app window.
  The existing color-specific Apply and Cancel tool names remain available.

Requests are limited to 64 KiB, responses to 2 MiB, and UI requests time out
after 15 seconds. The bridge reads application state, captures screenshots,
and applies edits on the UI thread. The revision is an opaque process-local
change token, not a security signature. Unsaved state is reported explicitly;
the filename is limited to its base name.

Example MCP server command:

```text
sketch3d --mcp-live-app-server
```
