import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import {
  MAX_GLB_BYTES,
  readBoundedResponse,
  resolveShareModelUrl,
  validateGlb,
} from "../web/share-viewer/viewer.js";

function makeGlb(document = { asset: { version: "2.0" }, scenes: [{}], scene: 0 }) {
  const json = Buffer.from(JSON.stringify(document));
  const paddedLength = Math.ceil(json.length / 4) * 4;
  const totalLength = 12 + 8 + paddedLength;
  const bytes = Buffer.alloc(totalLength, 0x20);
  bytes.writeUInt32LE(0x46546c67, 0);
  bytes.writeUInt32LE(2, 4);
  bytes.writeUInt32LE(totalLength, 8);
  bytes.writeUInt32LE(paddedLength, 12);
  bytes.writeUInt32LE(0x4e4f534a, 16);
  json.copy(bytes, 20);
  return bytes;
}

test("validates a minimal binary glTF 2.0 GLB", () => {
  const result = validateGlb(makeGlb());
  assert.equal(result.document.asset.version, "2.0");
  assert.equal(result.bytes.byteLength, makeGlb().byteLength);
});

test("accepts a real exported Spatial Sketch GLB", async () => {
  const bytes = await readFile(new URL("../verification/exports/house.glb", import.meta.url));
  assert.equal(validateGlb(bytes).document.asset.version, "2.0");
});

test("rejects bad magic, other GLB versions, and mismatched file lengths", () => {
  const badMagic = makeGlb();
  badMagic.writeUInt32LE(0, 0);
  assert.throws(() => validateGlb(badMagic), /not a binary glTF/i);

  const badVersion = makeGlb();
  badVersion.writeUInt32LE(1, 4);
  assert.throws(() => validateGlb(badVersion), /only glTF 2.0/i);

  const badLength = makeGlb();
  badLength.writeUInt32LE(badLength.byteLength + 4, 8);
  assert.throws(() => validateGlb(badLength), /length header/i);
});

test("rejects malformed JSON chunks and truncated chunks", () => {
  const badJson = makeGlb();
  badJson.fill(0xff, 20);
  assert.throws(() => validateGlb(badJson), /JSON chunk is invalid/i);

  const truncated = makeGlb();
  truncated.writeUInt32LE(truncated.byteLength, 12);
  assert.throws(() => validateGlb(truncated), /JSON header chunk/i);
});

test("blocks external buffer and image URLs while allowing embedded data URIs", () => {
  for (const document of [
    { asset: { version: "2.0" }, buffers: [{ byteLength: 4, uri: "https://example.invalid/model.bin" }] },
    { asset: { version: "2.0" }, images: [{ uri: "//cdn.example.invalid/texture.png" }] },
    { asset: { version: "2.0" }, images: [{ uri: "textures/albedo.png" }] },
  ]) {
    assert.throws(() => validateGlb(makeGlb(document)), /external buffer and image URLs are blocked/i);
  }

  assert.equal(
    validateGlb(makeGlb({
      asset: { version: "2.0" },
      images: [{ uri: "data:image/png;base64,AA==" }],
    })).document.asset.version,
    "2.0",
  );
});

test("enforces the 10 MiB bound before parsing", () => {
  assert.equal(MAX_GLB_BYTES, 10 * 1024 * 1024);
  assert.throws(() => validateGlb(new Uint8Array(MAX_GLB_BYTES + 1)), /10 MiB limit/i);
});

test("allows same-site HTTPS links and localhost development only", () => {
  const production = { href: "https://viewer.example/viewer/?model=%2Fartwork%2Fa.glb", origin: "https://viewer.example" };
  assert.equal(resolveShareModelUrl("/artwork/a.glb", production).origin, production.origin);

  const local = { href: "http://localhost:8080/viewer/", origin: "http://localhost:8080" };
  assert.equal(resolveShareModelUrl("/artwork/a.glb", local).protocol, "http:");
  assert.throws(() => resolveShareModelUrl("https://other.example/a.glb", production), /same site/i);
  assert.throws(() => resolveShareModelUrl("https://user:pass@viewer.example/a.glb", production), /credentials/i);
  assert.throws(() => resolveShareModelUrl("http://viewer.example/a.glb", { href: "http://viewer.example/", origin: "http://viewer.example" }), /HTTPS/i);
  assert.throws(() => resolveShareModelUrl("/artwork/a.gltf", production), /\.glb/i);
});

test("checks response status and both declared and streamed response limits", async () => {
  await assert.rejects(
    readBoundedResponse({ ok: false, status: 404, headers: new Headers() }),
    /HTTP 404/i,
  );
  await assert.rejects(
    readBoundedResponse({
      ok: true,
      status: 200,
      headers: new Headers({ "content-length": String(MAX_GLB_BYTES + 1) }),
    }),
    /10 MiB limit/i,
  );
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array(6));
      controller.enqueue(new Uint8Array(6));
      controller.close();
    },
  });
  await assert.rejects(
    readBoundedResponse({ ok: true, status: 200, headers: new Headers(), body: stream }, 10),
    /limit/i,
  );
});

class FakeElement {
  constructor() {
    this.listeners = new Map();
    this.attributes = new Map();
    this.textContent = "";
    this.disabled = false;
    this.hidden = false;
    this.files = [];
  }

  addEventListener(type, callback) {
    this.listeners.set(type, callback);
  }

  dispatch(type, event = {}) {
    return this.listeners.get(type)?.(event);
  }

  setAttribute(name, value) {
    this.attributes.set(name, value);
    if (name === "src") this.src = value;
  }

  removeAttribute(name) {
    this.attributes.delete(name);
    if (name === "src") this.src = "";
  }

  updateFraming() { this.framingUpdated = true; }
  jumpCameraToGoal() { this.cameraJumped = true; }
}

function makeViewerDocument() {
  const elements = new Map([
    ["#artwork-viewer", new FakeElement()],
    ["#glb-file", new FakeElement()],
    ["#viewer-status", new FakeElement()],
    ["#viewer-error", new FakeElement()],
    ["#reset-view", new FakeElement()],
    ["#model-viewer-library", new FakeElement()],
  ]);
  return {
    elements,
    querySelector(selector) { return elements.get(selector); },
  };
}

test("local file workflow loads a checked GLB and restores camera controls", async () => {
  const document = makeViewerDocument();
  const fileInput = document.querySelector("#glb-file");
  const bytes = makeGlb();
  fileInput.files = [{
    name: "sample.glb",
    size: bytes.byteLength,
    async arrayBuffer() { return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength); },
  }];
  const { initializeViewer } = await import("../web/share-viewer/viewer.js");
  initializeViewer(document, { href: "http://localhost/viewer/", origin: "http://localhost" }, async () => {
    throw new Error("local file loading must not make a network request");
  });
  await fileInput.dispatch("change");

  const viewer = document.querySelector("#artwork-viewer");
  const reset = document.querySelector("#reset-view");
  const status = document.querySelector("#viewer-status");
  assert.match(viewer.src, /^blob:/);
  assert.match(status.textContent, /Loading sample\.glb/);
  viewer.dispatch("load");
  assert.equal(reset.disabled, false);
  assert.match(status.textContent, /sample\.glb loaded/);

  reset.dispatch("click");
  assert.equal(viewer.cameraOrbit, "0deg 75deg 105%");
  assert.equal(viewer.cameraTarget, "auto auto auto");
  assert.equal(viewer.fieldOfView, "auto");
  assert.equal(viewer.framingUpdated, true);
  assert.equal(viewer.cameraJumped, true);
});

test("unsupported local files produce an accessible error without loading", async () => {
  const document = makeViewerDocument();
  const fileInput = document.querySelector("#glb-file");
  fileInput.files = [{ name: "drawing.gltf", size: 100, arrayBuffer: async () => new ArrayBuffer(100) }];
  const { initializeViewer } = await import("../web/share-viewer/viewer.js");
  initializeViewer(document, { href: "http://localhost/viewer/", origin: "http://localhost" }, async () => {
    throw new Error("unsupported files must not make a network request");
  });
  await fileInput.dispatch("change");
  assert.equal(document.querySelector("#viewer-error").hidden, false);
  assert.match(document.querySelector("#viewer-error").textContent, /\.glb file/i);
  assert.equal(document.querySelector("#reset-view").disabled, true);
});

test("same-site share links fetch without cookies and surface network failures", async () => {
  const document = makeViewerDocument();
  const bytes = makeGlb();
  const location = {
    href: "https://viewer.example/viewer/?model=%2Fartwork%2Fsample.glb",
    origin: "https://viewer.example",
  };
  const { initializeViewer } = await import("../web/share-viewer/viewer.js");
  const requests = [];
  initializeViewer(document, location, async (url, options) => {
    requests.push({ url: url.href, options });
    return new Response(bytes, { status: 200, headers: { "content-length": String(bytes.byteLength) } });
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, "https://viewer.example/artwork/sample.glb");
  assert.equal(requests[0].options.credentials, "omit");
  assert.match(document.querySelector("#artwork-viewer").src, /^blob:/);
  assert.match(document.querySelector("#viewer-status").textContent, /Loading sample\.glb/);

  const failedDocument = makeViewerDocument();
  const failedLocation = {
    href: "https://viewer.example/viewer/?model=%2Fartwork%2Fmissing.glb",
    origin: "https://viewer.example",
  };
  initializeViewer(failedDocument, failedLocation, async () => { throw new TypeError("Failed to fetch"); });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(failedDocument.querySelector("#viewer-error").hidden, false);
  assert.match(failedDocument.querySelector("#viewer-error").textContent, /Failed to fetch/);
});

test("viewer reports a failed rendering-library load", async () => {
  const document = makeViewerDocument();
  document.defaultView = {
    customElements: {
      get: () => undefined,
      whenDefined: () => new Promise(() => {}),
    },
  };
  const { initializeViewer } = await import("../web/share-viewer/viewer.js");
  initializeViewer(document, { href: "http://localhost/viewer/", origin: "http://localhost" }, async () => {
    throw new Error("the viewer library test must not fetch a model");
  });
  document.querySelector("#model-viewer-library").dispatch("error");
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(document.querySelector("#glb-file").disabled, true);
  assert.equal(document.querySelector("#viewer-error").hidden, false);
  assert.match(document.querySelector("#viewer-error").textContent, /3D viewer library/i);
});
