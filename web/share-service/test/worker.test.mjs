import assert from "node:assert/strict";
import test from "node:test";
import worker, { ShareManager, validateGlb } from "../worker.mjs";

function makeGlb(document = { asset: { version: "2.0" }, scenes: [{}], scene: 0 }, binaryLength = 0) {
  const json = Buffer.from(JSON.stringify(document));
  const paddedLength = Math.ceil(json.length / 4) * 4;
  const paddedBinaryLength = Math.ceil(binaryLength / 4) * 4;
  const totalLength = 12 + 8 + paddedLength + (paddedBinaryLength > 0 ? 8 + paddedBinaryLength : 0);
  const bytes = Buffer.alloc(totalLength, 0x20);
  bytes.writeUInt32LE(0x46546c67, 0);
  bytes.writeUInt32LE(2, 4);
  bytes.writeUInt32LE(totalLength, 8);
  bytes.writeUInt32LE(paddedLength, 12);
  bytes.writeUInt32LE(0x4e4f534a, 16);
  json.copy(bytes, 20);
  if (paddedBinaryLength > 0) {
    bytes.writeUInt32LE(paddedBinaryLength, 20 + paddedLength);
    bytes.writeUInt32LE(0x004e4942, 24 + paddedLength);
  }
  return bytes;
}

class MemoryBucket {
  constructor() { this.objects = new Map(); this.failNextPut = false; }
  async put(key, value, options = {}) {
    if (this.failNextPut) {
      this.failNextPut = false;
      throw new Error("simulated R2 write failure");
    }
    const bytes = value instanceof Uint8Array ? new Uint8Array(value) : new Uint8Array(await new Response(value).arrayBuffer());
    const customMetadata = { ...options.customMetadata };
    this.objects.set(key, {
      size: bytes.byteLength,
      customMetadata,
      bytes,
      body: new ReadableStream({ start(controller) { controller.enqueue(bytes); controller.close(); } }),
    });
  }
  async get(key) { return this.objects.get(key) ?? null; }
  async delete(key) { this.objects.delete(key); }
  async list({ prefix = "" } = {}) {
    return {
      objects: [...this.objects]
        .filter(([key]) => key.startsWith(prefix))
        .map(([key, value]) => ({ key, size: value.size, customMetadata: { ...value.customMetadata } })),
      truncated: false,
    };
  }
}

function fakeRateLimiter(success = true, fail = false) {
  const keys = [];
  const state = { success, fail };
  return {
    keys,
    set success(value) { state.success = value; },
    set fail(value) { state.fail = value; },
    async limit({ key }) {
      keys.push(key);
      if (state.fail) throw new Error("simulated limiter failure");
      return { success: state.success };
    },
  };
}

function harness({ maxObjects = 900, maxBytes = 9 * 1024 * 1024 * 1024 } = {}) {
  const bucket = new MemoryBucket();
  const manager = new ShareManager({}, {
    SHARES: bucket,
    MAX_ACTIVE_SHARES: String(maxObjects),
    MAX_TOTAL_BYTES: String(maxBytes),
  });
  const namespace = {
    idFromName(name) { assert.equal(name, "global-share-quota-v1"); return name; },
    get() { return manager; },
  };
  const apiRateLimiter = fakeRateLimiter();
  const downloadRateLimiter = fakeRateLimiter();
  const env = {
    SHARES: bucket,
    SHARE_MANAGER: namespace,
    API_RATE_LIMITER: apiRateLimiter,
    DOWNLOAD_RATE_LIMITER: downloadRateLimiter,
    ASSETS: { async fetch() { return new Response("viewer asset", { status: 200 }); } },
  };
  return { bucket, manager, env, apiRateLimiter, downloadRateLimiter };
}

async function createShare(env, bytes = makeGlb()) {
  return await worker.fetch(new Request("https://share.example/api/v1/models", {
    method: "POST",
    headers: { "content-type": "model/gltf-binary", "cf-connecting-ip": "198.51.100.9" },
    body: bytes,
  }), env);
}

test("validates GLB 2.0 and rejects external resources", () => {
  assert.equal(validateGlb(makeGlb()).byteLength > 20, true);
  assert.throws(() => validateGlb(makeGlb({
    asset: { version: "2.0" },
    buffers: [{ byteLength: 4, uri: "https://outside.example/model.bin" }],
  })), /external buffer and image URLs are blocked/i);
  const invalidVersion = makeGlb();
  invalidVersion.writeUInt32LE(1, 4);
  assert.throws(() => validateGlb(invalidVersion), /not a glTF 2.0 GLB/i);
});

test("applies mutation and public-download rate limits before storage work", async () => {
  const mutation = harness();
  mutation.apiRateLimiter.success = false;
  const deniedCreate = await createShare(mutation.env);
  assert.equal(deniedCreate.status, 429);
  assert.equal(deniedCreate.headers.get("retry-after"), "60");
  assert.deepEqual(mutation.apiRateLimiter.keys, ["198.51.100.9"]);
  assert.equal(mutation.bucket.objects.size, 0);

  const download = harness();
  const created = await (await createShare(download.env)).json();
  download.downloadRateLimiter.success = false;
  const deniedRead = await worker.fetch(new Request(
    `https://share.example/models/${created.id}.glb`,
  ), download.env);
  assert.equal(deniedRead.status, 429);
  assert.deepEqual(download.downloadRateLimiter.keys, [created.id]);
});

test("fails closed if the configured rate limiter is unavailable", async () => {
  const { env, apiRateLimiter, bucket } = harness();
  apiRateLimiter.fail = true;
  const response = await createShare(env);
  assert.equal(response.status, 503);
  assert.equal(bucket.objects.size, 0);
});

test("exposes only a validated abuse-report contact without collecting report data", async () => {
  const { env } = harness();
  env.ABUSE_CONTACT = "abuse@example.org";
  const response = await worker.fetch(new Request("https://share.example/api/v1/config"), env);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.deepEqual(await response.json(), { abuseContact: "abuse@example.org" });

  env.ABUSE_CONTACT = "abuse@example.org?subject=unsafe";
  const invalid = await worker.fetch(new Request("https://share.example/api/v1/config"), env);
  assert.deepEqual(await invalid.json(), { abuseContact: null });
});

test("creates public URLs and keeps only a capability hash in R2 metadata", async () => {
  const { bucket, env } = harness();
  const bytes = makeGlb();
  const response = await createShare(env, bytes);
  assert.equal(response.status, 201);
  const result = await response.json();
  assert.match(result.id, /^[0-9a-f]{48}$/u);
  assert.match(result.capability, /^[0-9a-f]{64}$/u);
  const viewerUrl = new URL(result.url);
  assert.equal(viewerUrl.pathname, "/share-viewer/");
  assert.equal(viewerUrl.searchParams.get("model"), `/models/${result.id}.glb`);
  assert.equal(result.bytes, bytes.byteLength);

  const stored = await bucket.get(`shares/${result.id}.glb`);
  assert.deepEqual(Buffer.from(stored.bytes), bytes);
  assert.equal(stored.customMetadata.expiresAt, String(result.expiresAt));
  assert.equal(JSON.stringify(stored.customMetadata).includes(result.capability), false);
  assert.match(stored.customMetadata.capabilityHash, /^[0-9a-f]{64}$/u);
  assert.notEqual(stored.customMetadata.capabilityHash, result.capability);
  assert.equal(stored.size, bytes.byteLength);
});

test("serves public GLB without caching and immediately hides expired shares", async () => {
  const { env } = harness();
  const created = await (await createShare(env)).json();
  const url = `https://share.example/models/${created.id}.glb`;
  const response = await worker.fetch(new Request(url), env);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("cache-control"), "no-store, max-age=0");
  assert.equal(response.headers.get("content-type"), "model/gltf-binary");
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), makeGlb());

  const stored = await env.SHARES.get(`shares/${created.id}.glb`);
  stored.customMetadata.expiresAt = String(Date.now() - 1);
  const expired = await worker.fetch(new Request(url), env);
  assert.equal(expired.status, 404);
});

test("updates preserve capability and failed uploads leave the prior model intact", async () => {
  const { env, bucket } = harness();
  const firstBytes = makeGlb();
  const secondBytes = makeGlb({ asset: { version: "2.0", generator: "updated" } });
  const created = await (await createShare(env, firstBytes)).json();
  const url = `https://share.example/api/v1/models/${created.id}`;
  const unauthorized = await worker.fetch(new Request(url, { method: "PUT", body: secondBytes }), env);
  assert.equal(unauthorized.status, 401);

  const updated = await worker.fetch(new Request(url, {
    method: "PUT",
    headers: { authorization: `Bearer ${created.capability}` },
    body: secondBytes,
  }), env);
  assert.equal(updated.status, 200);
  assert.deepEqual(Buffer.from((await bucket.get(`shares/${created.id}.glb`)).bytes), secondBytes);

  bucket.failNextPut = true;
  const failed = await worker.fetch(new Request(url, {
    method: "PUT",
    headers: { authorization: `Bearer ${created.capability}` },
    body: firstBytes,
  }), env);
  assert.equal(failed.status, 503);
  assert.deepEqual(Buffer.from((await bucket.get(`shares/${created.id}.glb`)).bytes), secondBytes);
});

test("unpublishes immediately and rejects invalid management capabilities", async () => {
  const { env, bucket } = harness();
  const created = await (await createShare(env)).json();
  const apiUrl = `https://share.example/api/v1/models/${created.id}`;
  const wrong = await worker.fetch(new Request(apiUrl, {
    method: "DELETE",
    headers: { authorization: `Bearer ${"0".repeat(64)}` },
  }), env);
  assert.equal(wrong.status, 403);

  const removed = await worker.fetch(new Request(apiUrl, {
    method: "DELETE",
    headers: { authorization: `Bearer ${created.capability}` },
  }), env);
  assert.equal(removed.status, 204);
  assert.equal(await bucket.get(`shares/${created.id}.glb`), null);
  const publicFetch = await worker.fetch(new Request(`https://share.example/models/${created.id}.glb`), env);
  assert.equal(publicFetch.status, 404);
});

test("serializes creates and enforces aggregate object and byte limits", async () => {
  const { env, bucket } = harness({ maxObjects: 1, maxBytes: 1000 });
  const first = makeGlb();
  const results = await Promise.all([createShare(env, first), createShare(env, first)]);
  assert.deepEqual(results.map((response) => response.status).sort(), [201, 507]);
  assert.equal(bucket.objects.size, 1);

  const created = await results.find((response) => response.status === 201).json();
  const oversizedReplacement = makeGlb({ asset: { version: "2.0", generator: "larger" } }, 1024);
  // Force the replacement to exceed the configured aggregate quota while
  // remaining a structurally valid GLB.
  assert.ok(oversizedReplacement.byteLength > 1000);
  const update = await worker.fetch(new Request(`https://share.example/api/v1/models/${created.id}`, {
    method: "PUT",
    headers: { authorization: `Bearer ${created.capability}` },
    body: oversizedReplacement,
  }), env);
  assert.equal(update.status, 507);
});

test("expires shares from public reads and reclaims their quota during the next write", async () => {
  const { env, bucket } = harness({ maxObjects: 1 });
  const first = await (await createShare(env)).json();
  const oldObject = await bucket.get(`shares/${first.id}.glb`);
  oldObject.customMetadata.expiresAt = String(Date.now() - 1);

  const response = await createShare(env);
  assert.equal(response.status, 201);
  assert.equal(await bucket.get(`shares/${first.id}.glb`), null);
  assert.equal(bucket.objects.size, 1);
});

test("enforces the request body bound even if Content-Length is omitted", async () => {
  const { env } = harness();
  const bytes = new Uint8Array(10 * 1024 * 1024 + 1);
  const response = await createShare(env, bytes);
  assert.equal(response.status, 400);
  assert.match((await response.json()).error, /10 MiB limit/i);
});

test("routes assets through the static binding and rejects unsupported methods", async () => {
  const { env } = harness();
  assert.equal((await worker.fetch(new Request("https://share.example/share-viewer/"), env)).status, 200);
  const response = await worker.fetch(new Request("https://share.example/api/v1/models", { method: "GET" }), env);
  assert.equal(response.status, 405);
  assert.equal(response.headers.get("allow"), "POST, PUT, DELETE");
});
