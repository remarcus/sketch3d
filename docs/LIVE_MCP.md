# Live session MCP

Start Sketch3D with `--mcp-live-app-server` from an MCP client. This opt-in mode
opens the desktop app and exposes a local stdio MCP server in the same process.
Keep stdin and stdout connected to the MCP client; diagnostics go to stderr.

The server exposes two read-only tools:

- `live_document_snapshot` reports the active file's base name, whether it is
  saved, whether it has unsaved changes, bounded document counts, and selected
  stroke IDs. It does not return artwork data or absolute file paths.
- `live_viewport_capture` asks the UI thread for a screenshot and returns a PNG.
  Captures are limited to 4096 × 4096 pixels and 1,000,000 encoded bytes.

Requests are limited to 64 KiB, responses to 2 MiB, and UI requests time out
after 15 seconds. The bridge reads application state and captures screenshots
on the UI thread. It does not write or change the drawing. Unsaved state is
reported explicitly; the filename is limited to its base name.

Example MCP server command:

```text
sketch3d --mcp-live-app-server
```

Live mutation and preview/apply tools are not included in this read-only mode.
