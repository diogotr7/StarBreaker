// High-resolution screenshot capture for the scene viewer.
//
// The base WebGLRenderer is sized to the viewport; this module spins up a
// throwaway renderer at the target megapixel count, renders one frame,
// dumps it as a PNG, then disposes its GPU resources. Reusing the base
// renderer is avoided so the visible viewport never changes size during
// capture (changing the canvas size would re-flow Three.js's depth /
// stencil buffers and could leave artifacts on the next render).

import * as THREE from "three";
import {
  applyObliqueShear,
  obliqueCabinetShear,
  type ProjectionMode,
} from "./flight-camera";

/** Compute a (width, height) pair whose aspect matches the viewport and
 *  whose pixel count is approximately `targetMegapixels` million. The
 *  derivation is height-first so the integer rounding never produces a
 *  zero dimension. Pure helper for tests. */
export function computeScreenshotDimensions(
  viewportAspect: number,
  targetMegapixels: number,
): { width: number; height: number } {
  const aspect = viewportAspect > 0 ? viewportAspect : 1;
  const totalPixels = targetMegapixels * 1_000_000;
  const height = Math.round(Math.sqrt(totalPixels / aspect));
  const width = Math.round(height * aspect);
  return { width, height };
}

/** Build a deterministic filename for a screenshot. The slug defaults to
 *  "viewer" when the package name is missing or empty so the output is
 *  always usable. The timestamp is local-time so the filename matches the
 *  user's wall clock. */
export function formatScreenshotFilename(
  packageSlug: string | null | undefined,
  date: Date,
): string {
  const slug =
    packageSlug && packageSlug.length > 0 ? packageSlug : "viewer";
  const pad = (n: number): string => n.toString().padStart(2, "0");
  const yyyy = date.getFullYear();
  const mm = pad(date.getMonth() + 1);
  const dd = pad(date.getDate());
  const hh = pad(date.getHours());
  const mi = pad(date.getMinutes());
  const ss = pad(date.getSeconds());
  return `screenshot_${slug}_${yyyy}${mm}${dd}_${hh}${mi}${ss}.png`;
}

export interface ScreenshotOpts {
  scene: THREE.Scene;
  camera: THREE.Camera;
  baseRenderer: THREE.WebGLRenderer;
  filename: string;
  /** Target megapixel count; defaults to 50. The actual width/height are
   *  derived to match the viewport aspect, so the final pixel count is
   *  within rounding distance of the target. */
  targetMegapixels?: number;
  /** Optional projection mode hint. When the active camera is an
   *  OrthographicCamera, the cloned screenshot camera's frustum is
   *  recomputed for the high-res aspect. When the mode is "oblique",
   *  the cabinet shear is reapplied so the screenshot matches what
   *  the viewport renders. */
  projectionMode?: ProjectionMode;
  /** Progress hook: phases run in order render -> encode -> download ->
   *  done. Failure throws and never emits "done". */
  onProgress?: (
    phase: "render" | "encode" | "download" | "done",
    info?: string,
  ) => void;
}

/** Capture a high-resolution screenshot of the current scene through the
 *  current camera. Returns the actual dimensions and the PNG byte size.
 *  Errors propagate; the caller is responsible for surfacing them. */
export async function captureScreenshot(
  opts: ScreenshotOpts,
): Promise<{ width: number; height: number; bytes: number }> {
  const targetMP = opts.targetMegapixels ?? 50;
  // Read the live viewport aspect so the screenshot framing matches what
  // the user sees on-screen at capture time. `getBoundingClientRect()`
  // is post-CSS-scaling; using `clientWidth` would be device pixels and
  // could subtly differ on high-DPI setups.
  const rect = opts.baseRenderer.domElement.getBoundingClientRect();
  const viewportAspect =
    rect.height > 0 && rect.width > 0 ? rect.width / rect.height : 1;
  const { width, height } = computeScreenshotDimensions(viewportAspect, targetMP);
  const aspect = width / height;

  // Throwaway renderer; preserveDrawingBuffer is required so toBlob can
  // read back the freshly rendered framebuffer.
  const tmpRenderer = new THREE.WebGLRenderer({
    antialias: true,
    preserveDrawingBuffer: true,
  });
  tmpRenderer.setPixelRatio(1);
  tmpRenderer.setSize(width, height, false);

  // Mirror colour-management settings from the base renderer so the
  // screenshot matches the on-screen render. Without this the captured
  // PNG could come out gamma-shifted relative to the viewport.
  tmpRenderer.outputColorSpace = opts.baseRenderer.outputColorSpace;
  tmpRenderer.toneMapping = opts.baseRenderer.toneMapping;
  tmpRenderer.toneMappingExposure = opts.baseRenderer.toneMappingExposure;

  let ssCam: THREE.Camera;
  if (opts.camera instanceof THREE.PerspectiveCamera) {
    const cloned = opts.camera.clone() as THREE.PerspectiveCamera;
    cloned.aspect = aspect;
    cloned.updateProjectionMatrix();
    ssCam = cloned;
  } else if (opts.camera instanceof THREE.OrthographicCamera) {
    const cloned = opts.camera.clone() as THREE.OrthographicCamera;
    // Recompute frustum bounds for the new aspect, anchoring on the
    // current vertical extent (top/bottom unchanged). This is the
    // inverse of `computeOrthoFrustum`'s aspect-derived horizontal sizing
    // and keeps the framing consistent with what the viewport shows.
    const halfH = (cloned.top - cloned.bottom) * 0.5;
    cloned.top = halfH;
    cloned.bottom = -halfH;
    cloned.right = halfH * aspect;
    cloned.left = -halfH * aspect;
    cloned.updateProjectionMatrix();
    if (opts.projectionMode === "oblique") {
      applyObliqueShear(cloned.projectionMatrix, obliqueCabinetShear());
      cloned.projectionMatrixInverse.copy(cloned.projectionMatrix).invert();
    }
    ssCam = cloned;
  } else {
    // Generic Camera fallback: clone, do not touch the projection matrix.
    ssCam = opts.camera.clone();
  }

  try {
    opts.onProgress?.("render", `${width}x${height}`);
    tmpRenderer.render(opts.scene, ssCam);

    opts.onProgress?.("encode");
    const blob = await new Promise<Blob>((resolve, reject) => {
      tmpRenderer.domElement.toBlob((b) => {
        if (b) resolve(b);
        else reject(new Error("toBlob returned null"));
      }, "image/png");
    });

    opts.onProgress?.("download", `${blob.size} bytes`);
    await saveBlob(blob, opts.filename);

    opts.onProgress?.("done");
    return { width, height, bytes: blob.size };
  } catch (err) {
    console.error("[screenshot] capture failed:", err);
    throw err;
  } finally {
    // Cleanup runs even on error so the temporary GL context is released.
    tmpRenderer.dispose();
    tmpRenderer.forceContextLoss();
  }
}

/** Persist the PNG bytes. Tries the Tauri save dialog first; falls back to
 *  a browser anchor download when Tauri APIs are not available. The
 *  anchor path covers vitest / browser-only runs and the case where
 *  `plugin-fs` is not bundled.
 *
 *  `plugin-fs` is not in this app's package.json today, so the dialog
 *  branch is dormant in the current build; if/when plugin-fs ships, the
 *  branch lights up automatically without code changes. The dynamic
 *  string-keyed import keeps TypeScript from demanding type definitions
 *  for a missing dependency. */
async function saveBlob(blob: Blob, filename: string): Promise<void> {
  const tauriDialog = await tryImport<{
    save: (opts: {
      defaultPath?: string;
      filters?: { name: string; extensions: string[] }[];
    }) => Promise<string | null>;
  }>("@tauri-apps/plugin-dialog");
  const tauriFs = await tryImport<{
    writeFile: (path: string, data: Uint8Array) => Promise<void>;
  }>("@tauri-apps/plugin-fs");

  if (tauriDialog && tauriFs) {
    const path = await tauriDialog.save({
      defaultPath: filename,
      filters: [{ name: "PNG", extensions: ["png"] }],
    });
    if (!path) {
      // User cancelled; treat as a no-op without throwing.
      return;
    }
    const bytes = new Uint8Array(await blob.arrayBuffer());
    await tauriFs.writeFile(path, bytes);
    return;
  }

  // Anchor download fallback. URL.createObjectURL + an off-DOM <a>
  // element is the standard browser pattern; works inside Tauri's
  // webview too because Tauri exposes a real DOM.
  const url = URL.createObjectURL(blob);
  try {
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.click();
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Dynamic-import wrapper that hides missing modules behind a null
 *  return. The string-keyed indirection keeps TypeScript from issuing
 *  TS2307 for plugins that are not installed in every build. */
async function tryImport<T>(moduleName: string): Promise<T | null> {
  try {
    // Vite resolves the literal at runtime; TS sees only `string` so it
    // does not type-check the missing module.
    const dynamicImport = (s: string): Promise<unknown> =>
      import(/* @vite-ignore */ s);
    const mod = (await dynamicImport(moduleName)) as T;
    return mod;
  } catch {
    return null;
  }
}
