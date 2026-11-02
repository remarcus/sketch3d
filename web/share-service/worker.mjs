const MODEL_LIMIT = 10 * 1024 * 1024;
const DEFAULT_MAX_OBJECTS = 900;
const DEFAULT_MAX_BYTES = 9 * 1024 * 1024 * 1024;
const RETENTION_MS = 30 * 24 * 60 * 60 * 1000;
const ID_BYTES = 24;
const CAPABILITY_BYTES = 32;
const ID_PATTERN = /^[0-9a-f]{48}$/u;
const KEY_PREFIX = "shares/";

function json(body, status = 200, extraHeaders = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store", ...extraHeaders },
  });
}

function error(message, status, extraHeaders = {}) {
  return json({ error: message }, status, extraHeaders);
}

function randomHex(size) {
  const bytes = crypto.getRandomValues(new Uint8Array(size));
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function digestToken(token) {
  const bytes = new TextEncoder().encode(token);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function constantTimeEqual(left, right) {
  if (typeof left !== "string" || typeof right !== "string" || left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) {
    difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  }
  return difference === 0;
}

async function readBounded(request, maxBytes) {
  const declaredLength = Number(request.headers.get("content-length"));
  if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
    throw new RangeError("GLB exceeds the 10 MiB limit.");
  }
  if (!request.body) throw new TypeError("A GLB request body is required.");
  const reader = request.body.getReader();
  const chunks = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel();
        throw new RangeError("GLB exceeds the 10 MiB limit.");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

export function validateGlb(bytes, maxBytes = MODEL_LIMIT) {
  if (!(bytes instanceof Uint8Array)) bytes = new Uint8Array(bytes);
  if (bytes.byteLength > maxBytes) throw new RangeError("GLB exceeds the 10 MiB limit.");
  if (bytes.byteLength < 20) throw new TypeError("Not a glTF 2.0 GLB.");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0, true) !== 0x46546c67 || view.getUint32(4, true) !== 2 || view.getUint32(8, true) !== bytes.byteLength) {
    throw new TypeError("Not a glTF 2.0 GLB.");
  }
  const jsonLength = view.getUint32(12, true);
  if (view.getUint32(16, true) !== 0x4e4f534a || jsonLength === 0 || jsonLength % 4 !== 0 || 20 + jsonLength > bytes.byteLength) {
    throw new TypeError("The GLB JSON chunk is invalid.");
  }
  let document;
  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(20, 20 + jsonLength)).replace(/[\u0000\u0020]+$/u, "");
    document = JSON.parse(text);
  } catch {
    throw new TypeError("The GLB JSON chunk is invalid.");
  }
  if (document?.asset?.version !== "2.0") throw new TypeError("The model does not declare glTF 2.0.");
  for (const collectionName of ["buffers", "images"]) {
    const resources = document[collectionName];
    if (!Array.isArray(resources)) continue;
    for (const resource of resources) {
      if (resource && typeof resource === "object" && "uri" in resource &&
          (typeof resource.uri !== "string" || !/^data:/iu.test(resource.uri))) {
        throw new TypeError("External buffer and image URLs are blocked.");
      }
    }
  }
  let offset = 20 + jsonLength;
  while (offset < bytes.byteLength) {
    if (offset + 8 > bytes.byteLength) throw new TypeError("The GLB contains a truncated chunk.");
    const chunkLength = view.getUint32(offset, true);
    offset += 8;
    if (chunkLength % 4 !== 0 || offset + chunkLength > bytes.byteLength) throw new TypeError("The GLB contains an invalid chunk length.");
    offset += chunkLength;
  }
  return bytes;
}

function capabilityFrom(request) {
  const match = /^Bearer ([0-9a-f]{64})$/u.exec(request.headers.get("authorization") ?? "");
  return match?.[1] ?? null;
}

function modelIdFromPath(pathname) {
  const match = /^\/api\/v1\/models\/([0-9a-f]{48})$/u.exec(pathname);
  return match?.[1] ?? null;
}

function publicModelIdFromPath(pathname) {
  const match = /^\/models\/([0-9a-f]{48})\.glb$/u.exec(pathname);
  return match?.[1] ?? null;
}

function shareKey(id) {
  return `${KEY_PREFIX}${id}.glb`;
}

function readLimit(env, name, fallback) {
  const value = Number(env[name]);
  return Number.isSafeInteger(value) && value > 0 ? value : fallback;
}

function abuseContactFrom(env) {
  const contact = typeof env.ABUSE_CONTACT === "string" ? env.ABUSE_CONTACT.trim() : "";
  return /^[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,63}$/iu.test(contact) ? contact : null;
}

async function enforceRateLimit(binding, key) {
  try {
    const result = await binding.limit({ key });
    if (result?.success) return null;
    return error("Too many requests. Please try again later.", 429, { "retry-after": "60" });
  } catch {
    return error("The share service is temporarily unavailable. Please retry.", 503);
  }
}

export class ShareManager {
  constructor(state, env) {
    this.state = state;
    this.env = env;
    this.queue = Promise.resolve();
  }

  fetch(request) {
    const operation = this.queue.then(() => this.handle(request));
    this.queue = operation.catch(() => {});
    return operation;
  }

  async listShares() {
    const objects = [];
    let cursor;
    do {
      const page = await this.env.SHARES.list({
        prefix: KEY_PREFIX,
        limit: 1000,
        include: ["customMetadata"],
        ...(cursor ? { cursor } : {}),
      });
      objects.push(...page.objects);
      cursor = page.truncated ? page.cursor : undefined;
    } while (cursor);
    return objects;
  }

  async purgeExpired(now) {
    const objects = await this.listShares();
    const retained = [];
    for (const object of objects) {
      const expiresAt = Number(object.customMetadata?.expiresAt);
      if (Number.isFinite(expiresAt) && expiresAt > now) {
        retained.push(object);
        continue;
      }
      try {
        await this.env.SHARES.delete(object.key);
      } catch {
        // Keep objects whose expiry could not be purged counted against quotas.
        retained.push(object);
      }
    }
    return retained;
  }

  async handle(request) {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/api/v1/models") return await this.create(request, url);
    const id = modelIdFromPath(url.pathname);
    if (!id) return error("Not found.", 404);
    if (request.method === "PUT") return await this.update(request, id);
    if (request.method === "DELETE") return await this.remove(request, id);
    return error("Method not allowed.", 405, { allow: "PUT, DELETE" });
  }

  async create(request, url) {
    let bytes;
    try {
      bytes = validateGlb(await readBounded(request, MODEL_LIMIT));
    } catch (cause) {
      return error(cause instanceof RangeError ? cause.message : cause instanceof TypeError ? cause.message : "Invalid GLB.", 400);
    }
    const now = Date.now();
    let records;
    try {
      records = await this.purgeExpired(now);
    } catch {
      return error("The share service is temporarily unavailable. Please retry.", 503);
    }
    const maxObjects = readLimit(this.env, "MAX_ACTIVE_SHARES", DEFAULT_MAX_OBJECTS);
    const maxBytes = readLimit(this.env, "MAX_TOTAL_BYTES", DEFAULT_MAX_BYTES);
    const currentBytes = records.reduce((sum, record) => sum + record.size, 0);
    if (records.length >= maxObjects || currentBytes + bytes.byteLength > maxBytes) {
      return error("The share service storage quota is full.", 507);
    }
    let id;
    try {
      for (let attempt = 0; attempt < 4; attempt += 1) {
        const candidate = randomHex(ID_BYTES);
        if (!(await this.env.SHARES.get(shareKey(candidate)))) {
          id = candidate;
          break;
        }
      }
    } catch {
      return error("The share service is temporarily unavailable. Please retry.", 503);
    }
    if (!id) return error("A public share ID could not be allocated. Please retry.", 503);
    const capability = randomHex(CAPABILITY_BYTES);
    const capabilityHash = await digestToken(capability);
    const expiresAt = now + RETENTION_MS;
    try {
      await this.env.SHARES.put(shareKey(id), bytes, {
        httpMetadata: { contentType: "model/gltf-binary" },
        customMetadata: { expiresAt: String(expiresAt), capabilityHash },
      });
    } catch {
      return error("The model could not be stored. Please retry.", 503);
    }
    const viewerUrl = new URL("/share-viewer/", url.origin);
    viewerUrl.searchParams.set("model", `/models/${id}.glb`);
    return json({ id, url: viewerUrl.href, capability, bytes: bytes.byteLength, expiresAt }, 201);
  }

  async authorize(request, id) {
    const capability = capabilityFrom(request);
    if (!capability) return { response: error("A management capability is required.", 401) };
    let object;
    try {
      object = await this.env.SHARES.get(shareKey(id));
    } catch {
      return { response: error("The share service is temporarily unavailable. Please retry.", 503) };
    }
    const expiresAt = Number(object?.customMetadata?.expiresAt);
    const capabilityHash = object?.customMetadata?.capabilityHash;
    if (!object || !Number.isFinite(expiresAt) || expiresAt <= Date.now()) return { response: error("Share not found or expired.", 404) };
    const suppliedHash = await digestToken(capability);
    if (!constantTimeEqual(suppliedHash, capabilityHash)) return { response: error("Invalid management capability.", 403) };
    return { object, capabilityHash, expiresAt };
  }

  async update(request, id) {
    const authorized = await this.authorize(request, id);
    if (authorized.response) return authorized.response;
    let bytes;
    try {
      bytes = validateGlb(await readBounded(request, MODEL_LIMIT));
    } catch (cause) {
      return error(cause instanceof RangeError ? cause.message : cause instanceof TypeError ? cause.message : "Invalid GLB.", 400);
    }
    let records;
    try {
      records = await this.purgeExpired(Date.now());
    } catch {
      return error("The share service is temporarily unavailable. Please retry.", 503);
    }
    const currentRecord = records.find((record) => record.key === shareKey(id));
    if (!currentRecord) return error("Share not found or expired.", 404);
    const currentBytes = records.reduce((sum, record) => sum + record.size, 0);
    const maxBytes = readLimit(this.env, "MAX_TOTAL_BYTES", DEFAULT_MAX_BYTES);
    const nextBytes = currentBytes - currentRecord.size + bytes.byteLength;
    if (nextBytes > maxBytes) return error("The share service storage quota is full.", 507);
    const expiresAt = Date.now() + RETENTION_MS;
    try {
      await this.env.SHARES.put(shareKey(id), bytes, {
        httpMetadata: { contentType: "model/gltf-binary" },
        customMetadata: { expiresAt: String(expiresAt), capabilityHash: authorized.capabilityHash },
      });
    } catch {
      return error("The model could not be updated. The previous revision remains available.", 503);
    }
    return json({ id, bytes: bytes.byteLength, expiresAt });
  }

  async remove(request, id) {
    const authorized = await this.authorize(request, id);
    if (authorized.response) return authorized.response;
    try {
      await this.env.SHARES.delete(shareKey(id));
    } catch {
      return error("The model could not be unpublished. Please retry.", 503);
    }
    return new Response(null, { status: 204, headers: { "cache-control": "no-store" } });
  }
}

async function publicModel(request, env, id) {
  if (request.method !== "GET" && request.method !== "HEAD") return error("Method not allowed.", 405, { allow: "GET, HEAD" });
  let object;
  try {
    object = await env.SHARES.get(shareKey(id));
  } catch {
    return error("The share service is temporarily unavailable. Please retry.", 503);
  }
  const expiresAt = Number(object?.customMetadata?.expiresAt);
  if (!object || !Number.isFinite(expiresAt) || expiresAt <= Date.now()) return error("Share not found or expired.", 404);
  const headers = new Headers({
    "content-type": "model/gltf-binary",
    "content-length": String(object.size),
    "cache-control": "no-store, max-age=0",
    "x-content-type-options": "nosniff",
    "content-disposition": "attachment; filename=\"artwork.glb\"",
  });
  if (request.method === "HEAD") return new Response(null, { status: 200, headers });
  return new Response(object.body, { status: 200, headers });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const modelId = publicModelIdFromPath(url.pathname);
    if (modelId) {
      const rateLimitResponse = await enforceRateLimit(env.DOWNLOAD_RATE_LIMITER, modelId);
      if (rateLimitResponse) return rateLimitResponse;
      return await publicModel(request, env, modelId);
    }

    if (url.pathname.startsWith("/api/v1/")) {
      if (url.pathname === "/api/v1/config" && request.method === "GET") {
        return json({ abuseContact: abuseContactFrom(env) });
      }
      if (request.method !== "POST" && request.method !== "PUT" && request.method !== "DELETE") {
        return error("Method not allowed.", 405, { allow: "POST, PUT, DELETE" });
      }
      const clientIp = request.headers.get("cf-connecting-ip") || "unknown";
      const rateLimitResponse = await enforceRateLimit(env.API_RATE_LIMITER, clientIp);
      if (rateLimitResponse) return rateLimitResponse;
      const id = env.SHARE_MANAGER.idFromName("global-share-quota-v1");
      const manager = env.SHARE_MANAGER.get(id);
      return await manager.fetch(request);
    }

    return await env.ASSETS.fetch(request);
  },
};
