# Spatial Sketch static GLB viewer

This host-independent viewer loads one binary glTF 2.0 `.glb` at a time. It supports local file selection and a future same-site HTTPS link in the `model` query parameter. Publishing, updating, and unpublishing are not implemented until the host, ownership model, costs, and retention policy in [`docs/interactive-sharing-design.md`](../../docs/interactive-sharing-design.md) are decided.

The viewer rejects files over 10 MiB, malformed GLB headers/chunks, unsupported extensions, cross-site URLs, and non-HTTPS shared URLs (except localhost development). It does not upload local files or access app documents, recovery data, camera, or microphone. Orbit, pan, and zoom use the pinned Google `<model-viewer>` 4.4.0 web component; hosting the page requires network access to its CDN script. See the [official documentation](https://modelviewer.dev/docs/) for its camera controls and API.

Serve this directory over HTTPS (or localhost for development). Select a `.glb` file to preview it locally. A publishing service can later provide a same-origin URL such as `?model=/artwork/<id>.glb` after its service and privacy requirements are approved.

Run the host-independent parser, URL, and UI-flow checks with:

```powershell
node --test scripts/test_share_viewer.mjs
```
