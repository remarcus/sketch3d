export const MAX_GLB_BYTES = 10 * 1024 * 1024;
const GLB_MAGIC = 0x46546c67;
const JSON_CHUNK_TYPE = 0x4e4f534a;
const RESET_ORBIT = "0deg 75deg 105%";

function asBytes(buffer) {
  if (buffer instanceof Uint8Array) return buffer;
  if (buffer instanceof ArrayBuffer) return new Uint8Array(buffer);
  throw new TypeError("GLB input must be bytes.");
}

export function validateGlb(buffer, maxBytes = MAX_GLB_BYTES) {
  const bytes = asBytes(buffer);
  if (bytes.byteLength > maxBytes) {
    throw new Error(`This GLB is larger than the ${Math.floor(maxBytes / (1024 * 1024))} MiB limit.`);
  }
  if (bytes.byteLength < 20) throw new Error("This file is too short to be a GLB 2.0 model.");

  const header = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (header.getUint32(0, true) !== GLB_MAGIC) throw new Error("This file is not a binary glTF (.glb) model.");
  if (header.getUint32(4, true) !== 2) throw new Error("Only glTF 2.0 GLB files are supported.");
  if (header.getUint32(8, true) !== bytes.byteLength) throw new Error("The GLB length header does not match the file size.");

  let offset = 12;
  const jsonLength = header.getUint32(offset, true);
  const jsonType = header.getUint32(offset + 4, true);
  offset += 8;
  if (jsonType !== JSON_CHUNK_TYPE || jsonLength === 0 || jsonLength % 4 !== 0 || offset + jsonLength > bytes.byteLength) {
    throw new Error("The GLB does not contain a valid JSON header chunk.");
  }

  let document;
  try {
    const jsonText = new TextDecoder("utf-8", { fatal: true })
      .decode(bytes.subarray(offset, offset + jsonLength))
      .replace(/[\u0000\u0020]+$/u, "");
    document = JSON.parse(jsonText);
  } catch {
    throw new Error("The GLB JSON chunk is invalid.");
  }
  if (document?.asset?.version !== "2.0") throw new Error("The model does not declare glTF 2.0.");
  for (const collectionName of ["buffers", "images"]) {
    const resources = document[collectionName];
    if (!Array.isArray(resources)) continue;
    for (const resource of resources) {
      if (resource && typeof resource === "object" && "uri" in resource) {
        if (typeof resource.uri !== "string" || !/^data:/iu.test(resource.uri)) {
          throw new Error("External buffer and image URLs are blocked; GLB resources must be embedded or use data: URIs.");
        }
      }
    }
  }
  offset += jsonLength;

  while (offset < bytes.byteLength) {
    if (offset + 8 > bytes.byteLength) throw new Error("The GLB contains a truncated data chunk.");
    const chunkLength = header.getUint32(offset, true);
    offset += 8;
    if (chunkLength % 4 !== 0 || offset + chunkLength > bytes.byteLength) {
      throw new Error("The GLB contains an invalid data chunk length.");
    }
    offset += chunkLength;
  }

  return { bytes, document };
}

export function resolveShareModelUrl(value, locationLike = globalThis.location) {
  const url = new URL(value, locationLike.href);
  if (url.username || url.password) throw new Error("Shared model links must not contain credentials.");
  if (url.origin !== locationLike.origin) throw new Error("Shared models must be served from the same site as this viewer.");
  const isLocalDevelopment = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && isLocalDevelopment)) {
    throw new Error("Shared model links must use HTTPS.");
  }
  if (!url.pathname.toLowerCase().endsWith(".glb")) throw new Error("The shared link must point to a .glb file.");
  return url;
}

export async function readBoundedResponse(response, maxBytes = MAX_GLB_BYTES) {
  if (!response.ok) throw new Error(`The shared model could not be downloaded (HTTP ${response.status}).`);
  const contentLength = Number(response.headers.get("content-length"));
  if (Number.isFinite(contentLength) && contentLength > maxBytes) {
    throw new Error(`This GLB is larger than the ${Math.floor(maxBytes / (1024 * 1024))} MiB limit.`);
  }

  if (!response.body?.getReader) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.byteLength > maxBytes) throw new Error(`This GLB is larger than the ${Math.floor(maxBytes / (1024 * 1024))} MiB limit.`);
    return bytes;
  }

  const reader = response.body.getReader();
  const chunks = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel();
        throw new Error(`This GLB is larger than the ${Math.floor(maxBytes / (1024 * 1024))} MiB limit.`);
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

export function initializeViewer(documentLike = globalThis.document, locationLike = globalThis.location, fetchImpl = globalThis.fetch) {
  if (!documentLike || !locationLike || !fetchImpl) return;
  const viewer = documentLike.querySelector("#artwork-viewer");
  const fileInput = documentLike.querySelector("#glb-file");
  const status = documentLike.querySelector("#viewer-status");
  const error = documentLike.querySelector("#viewer-error");
  const resetButton = documentLike.querySelector("#reset-view");
  if (!viewer || !fileInput || !status || !error || !resetButton) return;

  let activeObjectUrl = null;
  let currentName = "artwork";
  let generation = 0;
  const setStatus = (message) => { status.textContent = message; };
  const showError = (message) => {
    error.textContent = message;
    error.hidden = false;
    resetButton.disabled = true;
    viewer.removeAttribute("src");
    if (activeObjectUrl) URL.revokeObjectURL(activeObjectUrl);
    activeObjectUrl = null;
  };
  const clearError = () => { error.hidden = true; error.textContent = ""; };

  const registry = documentLike.defaultView?.customElements ?? globalThis.customElements;
  let rejectLibraryLoad = null;
  let libraryTimer = null;
  const libraryReady = registry?.whenDefined && !registry.get("model-viewer")
    ? new Promise((resolve, reject) => {
        rejectLibraryLoad = reject;
        fileInput.disabled = true;
        setStatus("Loading the 3D viewer…");
        libraryTimer = setTimeout(() => reject(new Error("The 3D viewer library could not be loaded. Check your network connection and reload this page.")), 15000);
        registry.whenDefined("model-viewer").then(() => {
          clearTimeout(libraryTimer);
          resolve();
        });
      })
    : Promise.resolve();
  const libraryScript = documentLike.querySelector("#model-viewer-library");
  libraryScript?.addEventListener("error", () => {
    if (rejectLibraryLoad) {
      clearTimeout(libraryTimer);
      rejectLibraryLoad(new Error("The 3D viewer library could not be loaded. Check your network connection and reload this page."));
    }
  });
  libraryReady.then(() => {
    fileInput.disabled = false;
    if (error.hidden) setStatus("Viewer ready. GLB files up to 10 MiB are supported.");
  }).catch((loadError) => {
    fileInput.disabled = true;
    showError(loadError instanceof Error ? loadError.message : "The 3D viewer library could not be loaded.");
  });

  viewer.addEventListener("load", () => {
    clearError();
    resetButton.disabled = false;
    setStatus(`${currentName} loaded. Drag to orbit, right-drag or use a modifier to pan, and scroll or pinch to zoom.`);
  });
  viewer.addEventListener("error", () => showError("The GLB passed its header checks but could not be rendered. The model may contain unsupported or damaged data."));
  viewer.addEventListener("progress", (event) => {
    const percent = Math.round((event.detail?.totalProgress ?? 0) * 100);
    if (percent < 100) setStatus(`Loading ${currentName}… ${percent}%`);
  });

  const setModel = (bytes, name) => {
    validateGlb(bytes);
    generation += 1;
    const blob = new Blob([bytes], { type: "model/gltf-binary" });
    const nextObjectUrl = URL.createObjectURL(blob);
    if (activeObjectUrl) URL.revokeObjectURL(activeObjectUrl);
    activeObjectUrl = nextObjectUrl;
    currentName = name;
    resetButton.disabled = true;
    clearError();
    setStatus(`Loading ${name}…`);
    viewer.src = nextObjectUrl;
    viewer.removeAttribute("camera-orbit");
    viewer.setAttribute("camera-target", "auto auto auto");
  };

  fileInput.addEventListener("change", async () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    const requestGeneration = ++generation;
    try {
      await libraryReady;
      if (!file.name.toLowerCase().endsWith(".glb")) throw new Error("Choose a binary glTF .glb file. .gltf and other formats are not supported.");
      if (file.size > MAX_GLB_BYTES) throw new Error("This GLB is larger than the 10 MiB limit.");
      setStatus(`Checking ${file.name}…`);
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (requestGeneration !== generation) return;
      setModel(bytes, file.name);
    } catch (loadError) {
      if (requestGeneration === generation) {
        showError(loadError instanceof Error ? loadError.message : "The GLB could not be opened.");
      }
    }
  });

  resetButton.addEventListener("click", () => {
    viewer.cameraTarget = "auto auto auto";
    viewer.cameraOrbit = RESET_ORBIT;
    viewer.fieldOfView = "auto";
    viewer.updateFraming();
    viewer.jumpCameraToGoal();
  });

  const sharedPath = new URL(locationLike.href).searchParams.get("model");
  if (sharedPath) {
    (async () => {
      const requestGeneration = ++generation;
      try {
        await libraryReady;
        const url = resolveShareModelUrl(sharedPath, locationLike);
        setStatus("Downloading shared artwork…");
        const response = await fetchImpl(url, {
          mode: "same-origin",
          credentials: "omit",
          redirect: "error",
          cache: "no-store",
        });
        const bytes = await readBoundedResponse(response);
        if (requestGeneration !== generation) return;
        setModel(bytes, decodeURIComponent(url.pathname.split("/").pop() || "shared artwork"));
      } catch (loadError) {
        if (requestGeneration === generation) {
          showError(loadError instanceof Error ? loadError.message : "The shared artwork could not be loaded.");
        }
      }
    })();
  }
}

if (typeof document !== "undefined") initializeViewer();
