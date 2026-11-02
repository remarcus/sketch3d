# Spatial Sketch static GLB viewer

This viewer loads one binary glTF 2.0 `.glb` at a time. It supports local file selection and same-site HTTPS links in the `model` query parameter. The desktop app provides explicit publish, update, and unpublish actions; the candidate Cloudflare Worker and its deployment gates are documented in [`web/share-service/README.md`](../share-service/README.md) and [`docs/interactive-sharing-design.md`](../../docs/interactive-sharing-design.md).

The viewer rejects files over 10 MiB, malformed GLB headers/chunks, unsupported extensions, cross-site URLs, and non-HTTPS shared URLs (except localhost development). It does not upload local files or access app documents, recovery data, camera, or microphone. Orbit, pan, and zoom use the pinned Google `<model-viewer>` 4.4.0 web component; hosting the page requires network access to its CDN script. See the [official documentation](https://modelviewer.dev/docs/) for its camera controls and API.

Serve this directory over HTTPS (or localhost for development). Select a `.glb` file to preview it locally. A configured service can provide a same-origin URL such as `?model=/models/<id>.glb`. When its operator configures `ABUSE_CONTACT`, shared-model pages show an email link that opens the visitor's email app with that model URL.

Run the host-independent parser, URL, and UI-flow checks with:

```powershell
node --test scripts/test_share_viewer.mjs
```
