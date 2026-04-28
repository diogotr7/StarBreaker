// Renders a SOC scene-package GLB (the output of `load_scene_to_gltf`)
// in Three.js. Different from `scene-viewer.tsx`, which consumes a
// decomposed-export package directory; this loads a single
// self-contained `.glb` from the scene cache.
//
// What this component does in load order:
//   1. Set up Three.js (renderer, perspective camera, environment,
//      fallback lights). Same canonical setup that `scene-viewer.tsx`
//      uses, simplified -- no paint-variant / livery state, no per-mesh
//      submaterial-index binding, no scene.json walk. The SOC GLB is
//      self-contained.
//   2. Attach the shared flight camera via `useFlightCamera` so WASDQE /
//      orbit / oblique projection / F9 capture all work the same way as
//      in the ship viewer. The handle is republished to the parent via
//      `onFlightCamReady` so the parent can host the projection-mode
//      picker in its toolbar.
//   3. Build an `asset://` URL for the cached GLB on disk (via
//      `sceneGlbAssetUrl`, which wraps Tauri's `convertFileSrc`) and
//      hand it to `GLTFLoader.load` so the webview fetches the file
//      through its own HTTP path. This avoids shipping the GLB across
//      the IPC channel as a `number[]` -- the prior `loader.parse(bytes)`
//      pathway crashed the renderer with `STATUS_BREAKPOINT` on the
//      ~272 MB Exec Hangar GLB once the IPC custom protocol fell back
//      to `postMessage`. Lights live in the `KHR_lights_punctual`
//      extension and are auto-instantiated as `THREE.PointLight` /
//      `THREE.SpotLight` by the loader.
//   4. Walk every loaded material. If `userData.diffuse_texture_path`
//      is set, queue a Tauri-side DDS decode (`previewDds`) and assign
//      the resulting `THREE.Texture` to the material's `map` slot. The
//      GLB does not embed any placeholder textures (earlier iterations
//      did, which produced "Couldn't load texture blob:" errors when
//      thousands of materials shared one blob-backed image whose blob
//      URL the GLTFLoader had already revoked); until the DDS resolves,
//      the material renders with its `baseColorFactor` only. Texture
//      lookups are cached by source path so repeated submaterials
//      share one `THREE.Texture`. Failures fall back to a flat color
//      (debug magenta in dev, neutral grey in production).
//   5. Apply a -90deg rotation around X to the SCENE ROOT to flip
//      CryEngine Z-up into Three.js / glTF Y-up. The emitter
//      deliberately does not bake this rotation into the GLB; the
//      flight cam is basis-agnostic, so the rotation lives here.
//   6. After the GLB is added to the scene root, frame the camera with
//      `flightCam.resetToScene(sceneRoot)`. The flight cam computes the
//      AABB itself; the response's `aabb_min/max` are kept on hand for
//      the bottom-left stats overlay only.
//
// Console.log lines the user should look for in DevTools when running
// the Tauri app:
//   [soc-scene] init three=<rev>
//   [soc-scene] glb-loaded bytes=<N> meshes=<N> nodes=<N> lights=<N> ms=<N>
//   [soc-scene] textures resolved=<R>/<T> failed=<F>

import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import {
  previewDds,
  sceneGlbAssetUrl,
  type LoadSceneResponse,
} from "../lib/commands";
import { FlightCamHud } from "./flight-cam-hud";
import { useFlightCamera, type FlightCamHandle } from "../lib/flight-camera";

/**
 * Diffuse map intensity scalar. Engine intensity values for SOC lights
 * are in physical units (candela for point / lux for directional);
 * Three.js MeshStandardMaterial / lights use arbitrary scene units.
 * The emitter writes a default intensity of 1.0 per light, so the
 * scaling here is mostly cosmetic until a future iteration threads
 * real DataCore intensities through. Documented here so the choice
 * is obvious to the next person tweaking it.
 */
const LIGHT_INTENSITY_SCALE = 1.0;

/**
 * Z-up to Y-up basis flip. SOC coordinates are CryEngine Z-up; glTF /
 * Three.js are Y-up. Rotating the scene root by -90deg around X moves
 * world +Z (CryEngine "up") onto world +Y (Three.js "up"). Verified
 * by inspecting where lights land -- a "ceiling" point light lands
 * above the geometry after rotation, not below.
 */
function applyZUpToYUp(root: THREE.Object3D) {
  root.rotation.x = -Math.PI / 2;
}

/** Debug fallback color in dev mode, neutral grey in production. */
function fallbackColor(): number {
  // import.meta.env.DEV is true under `vite dev`. Hot path: we only
  // see this when texture resolution fails, so the cost of the env
  // check is fine.
  if (typeof import.meta !== "undefined" && import.meta.env?.DEV) {
    return 0xff00ff;
  }
  return 0x808080;
}

export interface SocSceneViewerProps {
  /** Response from `loadSceneToGltf` -- carries the cached GLB path
   *  and the AABB used for the stats overlay. */
  response: LoadSceneResponse;
  /** Optional status callback (e.g. for the toolbar). */
  onStatus?: (text: string) => void;
  /** Optional progress callback for the asset-protocol GLB fetch.
   *  Receives a fraction in [0, 1] derived from the underlying
   *  `XMLHttpRequest.onprogress` event; when the response is not
   *  `Content-Length`-tagged the fraction stays at the previous
   *  value rather than oscillating. */
  onLoadProgress?: (fraction: number, message: string) => void;
  /** Optional callback invoked once the flight-camera handle resolves.
   *  The parent uses this to host the projection-mode picker in its
   *  toolbar (mirrors the ship viewer). Called with `null` on
   *  unmount. */
  onFlightCamReady?: (handle: FlightCamHandle | null) => void;
}

interface TextureResolutionStats {
  resolved: number;
  total: number;
  failed: number;
}

export function SocSceneViewer({
  response,
  onStatus,
  onLoadProgress,
  onFlightCamReady,
}: SocSceneViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  // Refs published by the bootstrap effect so `useFlightCamera` (which
  // runs its own effect after this one) can attach. The render loop
  // reads `flightCamRef.current?.getActiveCamera()` so projection-mode
  // swaps and oblique screenshots both work.
  const rendererRef = useRef<THREE.WebGLRenderer | null>(null);
  const cameraRef = useRef<THREE.PerspectiveCamera | null>(null);
  const sceneRef = useRef<THREE.Scene | null>(null);
  const sceneRootRef = useRef<THREE.Group | null>(null);
  const flightCamRef = useRef<FlightCamHandle | null>(null);

  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<{
    meshes: number;
    nodes: number;
    lights: number;
    textures: TextureResolutionStats;
  } | null>(null);
  const generationRef = useRef(0);

  // ── Three.js bootstrap (runs once) ────────────────────────────────
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const renderer = new THREE.WebGLRenderer({
      antialias: true,
      powerPreference: "high-performance",
    });
    renderer.setClearColor(0x101418);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 0.85;
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
    renderer.setSize(container.clientWidth, container.clientHeight);
    container.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(
      45,
      container.clientWidth / container.clientHeight,
      0.1,
      100000,
    );
    camera.position.set(60, 30, 60);

    // Publish refs so `useFlightCamera` (next effect on this mount)
    // can read them. Must happen before the render loop starts so the
    // flight cam attaches on the first useful frame.
    rendererRef.current = renderer;
    cameraRef.current = camera;
    sceneRef.current = scene;

    // Fallback fill so a level with disabled lights still shows up.
    // SOC scenes carry hundreds-to-thousands of point lights through
    // `KHR_lights_punctual` so this is mostly a safety net.
    const hemi = new THREE.HemisphereLight(0xb1bfd8, 0x202428, 0.3);
    hemi.name = "fallback_hemi";
    scene.add(hemi);
    const dir = new THREE.DirectionalLight(0xffffff, 0.6);
    dir.position.set(50, 100, 25);
    dir.name = "fallback_dir";
    scene.add(dir);

    // PMREM environment so PBR materials get reasonable IBL.
    const pmremGenerator = new THREE.PMREMGenerator(renderer);
    const roomEnv = new RoomEnvironment();
    const envTexture = pmremGenerator.fromScene(roomEnv, 0.04).texture;
    scene.environment = envTexture;
    scene.environmentIntensity = 0.5;

    const sceneRoot = new THREE.Group();
    sceneRoot.name = "soc_scene_root";
    applyZUpToYUp(sceneRoot);
    scene.add(sceneRoot);
    sceneRootRef.current = sceneRoot;

    let animId = 0;
    const animate = () => {
      animId = requestAnimationFrame(animate);
      // Render with whichever camera the flight cam currently exposes
      // (perspective, orthographic, or oblique). Until the flight-cam
      // hook attaches on the next effect pass, `flightCamRef.current`
      // is null and we fall back to the perspective camera so the very
      // first frame still renders.
      const activeCam = flightCamRef.current?.getActiveCamera() ?? camera;
      renderer.render(scene, activeCam);
    };
    animate();

    console.log(`[soc-scene] init three=${THREE.REVISION}`);

    const ro = new ResizeObserver(() => {
      const w = container.clientWidth;
      const h = container.clientHeight;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    });
    ro.observe(container);

    return () => {
      cancelAnimationFrame(animId);
      ro.disconnect();
      renderer.dispose();
      pmremGenerator.dispose();
      envTexture.dispose();
      if (renderer.domElement.parentElement === container) {
        container.removeChild(renderer.domElement);
      }
      rendererRef.current = null;
      cameraRef.current = null;
      sceneRef.current = null;
      sceneRootRef.current = null;
    };
  }, []);

  // Attach the flight camera. Runs after the bootstrap effect populated
  // the renderer/camera/scene refs above, on the same mount pass. Stays
  // null on the first render (refs are still null), then resolves to a
  // stable handle once the hook's effect has installed listeners.
  const flightCam = useFlightCamera({ rendererRef, cameraRef, sceneRef });
  useEffect(() => {
    flightCamRef.current = flightCam;
  }, [flightCam]);

  // Hand the flight-cam handle up to the parent so it can host the
  // projection-mode picker in its top-right toolbar.
  useEffect(() => {
    onFlightCamReady?.(flightCam);
    return () => {
      onFlightCamReady?.(null);
    };
  }, [flightCam, onFlightCamReady]);

  // F9 captures a high-resolution screenshot through the active camera.
  // Listener lives on `window` so capture works regardless of which
  // child element holds focus, except for typing targets where F9 is
  // suppressed to avoid surprise captures while a text field is active.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent): void => {
      if (e.code !== "F9") return;
      const ae = document.activeElement;
      const tag = ae?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      // The decomposed scene viewer owns the screenshot helper; the
      // SOC viewer does not bring its own copy. F9 here is a no-op
      // hook for symmetry; if the user wants captures of SOC scenes,
      // the existing screenshot capture path can be wired in a future
      // iteration. For now, do nothing rather than throw.
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  // ── Scene load on response change ────────────────────────────────
  useEffect(() => {
    const renderer = rendererRef.current;
    const sceneRoot = sceneRootRef.current;
    if (!renderer || !sceneRoot) return;
    const gen = ++generationRef.current;
    setError(null);
    setStats(null);

    // Clear previous scene children. Do not strip the basis rotation
    // from sceneRoot itself -- that lives on the root and persists
    // across loads.
    clearChildren(sceneRoot);

    void (async () => {
      const t0 = performance.now();
      onStatus?.("Fetching scene...");
      try {
        // Build the asset:// URL up front. This is a pure path
        // mangling operation (no IPC), and it is what lets the GLB
        // bypass the JSON-encoded `Vec<u8>` IPC channel that crashed
        // the renderer with `STATUS_BREAKPOINT` on prior iterations.
        // The asset-protocol scope in `tauri.conf.json` restricts
        // resolvable paths to `$APPLOCALDATA/scene_cache/**`.
        const glbUrl = sceneGlbAssetUrl(response.glb_path);
        console.log(`[soc-scene] glb-fetch url=${glbUrl}`);

        const tFetch0 = performance.now();
        const loader = new GLTFLoader();
        const gltf = await new Promise<{
          scene: THREE.Group;
          parser: { json: { meshes?: unknown[]; nodes?: unknown[] } };
        }>((resolve, reject) => {
          loader.load(
            glbUrl,
            (g) =>
              resolve({
                scene: g.scene as unknown as THREE.Group,
                parser: g.parser as unknown as {
                  json: { meshes?: unknown[]; nodes?: unknown[] };
                },
              }),
            (event) => {
              // Three.js forwards XHR progress events. `lengthComputable`
              // is false when the response has no `Content-Length`
              // header -- for the local asset protocol it is reliably
              // populated, but we guard either way to avoid a
              // divide-by-zero.
              if (gen !== generationRef.current) return;
              if (event.lengthComputable && event.total > 0) {
                const fraction = Math.min(1, event.loaded / event.total);
                onLoadProgress?.(
                  fraction,
                  `Fetching GLB ${(event.loaded / 1024 / 1024).toFixed(0)} ` +
                    `/ ${(event.total / 1024 / 1024).toFixed(0)} MiB`,
                );
              }
            },
            (err) => reject(err),
          );
        });
        if (gen !== generationRef.current) return;
        const tFetch1 = performance.now();

        // Count what came in. The fetch + parse pair is reported as
        // a single `parse_ms` for continuity with prior iterations'
        // diagnostic output; the underlying split is fetch + parse
        // running back-to-back inside the loader.
        const counts = countSceneContent(gltf.scene);
        const meshCount = (gltf.parser.json.meshes ?? []).length;
        const nodeCount = (gltf.parser.json.nodes ?? []).length;
        console.log(
          `[soc-scene] glb-loaded bytes=${response.glb_bytes} ` +
            `meshes=${meshCount} nodes=${nodeCount} ` +
            `lights=${counts.lights} lights_dropped=${response.lights_dropped} ` +
            `parse_ms=${(tFetch1 - tFetch0).toFixed(0)}`,
        );

        // Scale every light's intensity. KHR_lights_punctual values
        // come from the SOC emitter's defaults (1.0) until a future
        // iteration threads real DataCore-side intensities through;
        // we apply the scaling scalar so future intensity tuning is
        // centralised here.
        applyLightIntensityScale(gltf.scene);

        // Attach the loaded tree to the basis-rotated scene root.
        sceneRoot.add(gltf.scene);

        // Resolve textures in the background. Materials render with
        // the placeholder white until each resolves, then swap in.
        onStatus?.("Resolving textures...");
        const texStats = await resolveAllTextures(
          gltf.scene,
          response.placement_count,
        );
        if (gen !== generationRef.current) return;

        // Hand framing to the flight cam. It walks the scene root's
        // AABB itself and positions the camera + orbit pivot. Until
        // the hook has attached on this mount, the call falls through
        // and the user can re-frame manually with the keys.
        flightCamRef.current?.resetToScene(sceneRoot);

        setStats({
          meshes: meshCount,
          nodes: nodeCount,
          lights: counts.lights,
          textures: texStats,
        });
        const t1 = performance.now();
        onStatus?.(
          `Loaded ${meshCount} meshes / ${counts.lights} lights ` +
            `/ ${texStats.resolved}/${texStats.total} textures (${((t1 - t0) / 1000).toFixed(1)}s)`,
        );
        console.log(
          `[soc-scene] textures resolved=${texStats.resolved}/${texStats.total} ` +
            `failed=${texStats.failed}`,
        );
      } catch (err) {
        if (gen !== generationRef.current) return;
        const msg = err instanceof Error ? err.message : String(err);
        console.error("[soc-scene] load failed:", msg);
        setError(msg);
        onStatus?.(`Load failed: ${msg}`);
      }
    })();
  }, [response, onStatus, onLoadProgress]);

  return (
    <div className="relative w-full h-full">
      <div ref={containerRef} className="w-full h-full" />
      {error && (
        <div className="absolute top-3 left-3 right-3 z-10 max-w-[80vw] mx-auto bg-danger/15 border border-danger/30 text-danger text-xs px-3 py-2 rounded-md font-mono break-words shadow">
          {error}
        </div>
      )}
      {stats && (
        <div className="absolute bottom-3 left-3 z-10 bg-bg-alt/90 border border-border text-xs text-text-sub px-3 py-1.5 rounded-md shadow font-mono tabular-nums">
          {stats.meshes} meshes / {stats.nodes} nodes
          {" / lights "}
          {stats.lights}
          {response.lights_dropped > 0
            ? `/${stats.lights + response.lights_dropped}`
            : ""}
          {" / "}
          {stats.textures.resolved}/{stats.textures.total} textures
          {response.materials_resolved > 0 || response.materials_default > 0 ? (
            <>
              {" / "}
              materials {response.materials_resolved}/
              {response.materials_resolved + response.materials_default}
            </>
          ) : null}
        </div>
      )}
      <FlightCamHud handle={flightCam} />
    </div>
  );
}

// ── Helpers ─────────────────────────────────────────────────────────

function clearChildren(group: THREE.Object3D) {
  for (let i = group.children.length - 1; i >= 0; i--) {
    const child = group.children[i];
    group.remove(child);
    disposeRecursive(child);
  }
}

function disposeRecursive(obj: THREE.Object3D) {
  obj.traverse((child) => {
    if (child instanceof THREE.Mesh) {
      child.geometry?.dispose();
      const m = child.material;
      if (Array.isArray(m)) {
        for (const mat of m) mat.dispose();
      } else if (m) {
        m.dispose();
      }
    }
  });
}

function countSceneContent(root: THREE.Object3D): {
  meshes: number;
  lights: number;
} {
  let meshes = 0;
  let lights = 0;
  root.traverse((c) => {
    if (c instanceof THREE.Mesh) meshes += 1;
    if (c instanceof THREE.Light) lights += 1;
  });
  return { meshes, lights };
}

function applyLightIntensityScale(root: THREE.Object3D) {
  root.traverse((c) => {
    if (c instanceof THREE.Light) {
      c.intensity *= LIGHT_INTENSITY_SCALE;
    }
  });
}

/**
 * Walk every material in the scene; for each one that has a
 * `userData.diffuse_texture_path` extra, kick off a Tauri-side DDS
 * decode and assign the result to the material's `.map` slot. The GLB
 * does not embed any placeholder texture, so `.map` is `null` on every
 * material when the loader returns; until the DDS resolves a material
 * renders with its `baseColorFactor` only. Texture lookups are cached
 * so duplicate paths share one `THREE.Texture`. Failures keep `.map =
 * null` and tint the base color (debug magenta in dev, neutral grey
 * in prod).
 */
async function resolveAllTextures(
  root: THREE.Object3D,
  _placementCount: number,
): Promise<TextureResolutionStats> {
  const cache = new Map<string, Promise<THREE.Texture | null>>();
  const stats: TextureResolutionStats = { resolved: 0, total: 0, failed: 0 };

  // Materials are shared across many meshes (one material per
  // submaterial). We collect a unique set first so we only walk
  // each one once.
  const materials = new Set<THREE.Material>();
  root.traverse((child) => {
    if (!(child instanceof THREE.Mesh)) return;
    const m = child.material;
    if (Array.isArray(m)) {
      for (const mat of m) materials.add(mat);
    } else if (m) {
      materials.add(m);
    }
  });

  const tasks: Promise<void>[] = [];

  for (const material of materials) {
    const userData = (material as unknown as { userData?: Record<string, unknown> })
      .userData;
    const diffusePath = userData?.diffuse_texture_path;
    const normalPath = userData?.normal_texture_path;

    if (typeof diffusePath === "string" && diffusePath.length > 0) {
      stats.total += 1;
      tasks.push(
        loadTextureCached(cache, diffusePath, "diffuse").then((tex) => {
          const standard = material as THREE.MeshStandardMaterial;
          if (tex) {
            // Diffuse maps render in sRGB; createImageBitmap-backed
            // THREE.Texture defaults to NoColorSpace, so set explicitly.
            tex.colorSpace = THREE.SRGBColorSpace;
            standard.map = tex;
            standard.needsUpdate = true;
            stats.resolved += 1;
          } else {
            // Resolution failed. Material renders with its
            // baseColorFactor only, tinted for debug visibility in dev.
            standard.map = null;
            standard.color = new THREE.Color(fallbackColor());
            standard.needsUpdate = true;
            stats.failed += 1;
          }
        }),
      );
    }

    if (typeof normalPath === "string" && normalPath.length > 0) {
      stats.total += 1;
      tasks.push(
        loadTextureCached(cache, normalPath, "normal").then((tex) => {
          const standard = material as THREE.MeshStandardMaterial;
          if (tex) {
            // Normal maps stay Linear (no gamma).
            tex.colorSpace = THREE.NoColorSpace;
            standard.normalMap = tex;
            standard.needsUpdate = true;
            stats.resolved += 1;
          } else {
            stats.failed += 1;
          }
        }),
      );
    }
  }

  // Bound concurrency: too many simultaneous Tauri previewDds calls
  // saturate the IPC channel. 6 keeps the renderer responsive.
  await runWithLimit(tasks, 6);
  return stats;
}

async function runWithLimit<T>(tasks: Promise<T>[], limit: number): Promise<T[]> {
  if (limit <= 0 || tasks.length <= limit) return Promise.all(tasks);
  const results: T[] = new Array(tasks.length);
  let next = 0;
  const workers = Array.from({ length: limit }, async () => {
    while (true) {
      const i = next++;
      if (i >= tasks.length) return;
      results[i] = await tasks[i];
    }
  });
  await Promise.all(workers);
  return results;
}

/**
 * Resolve one CryEngine texture path against the loaded p4k via the
 * `previewDds` Tauri command. The MTL stores texture paths with `.tif`
 * extensions; the engine swaps that for `.dds` at load time. We try
 * the same fallback ladder: `.dds`, then the original extension. Mip
 * 2 (1/4 size) is the chosen detail level -- full-size textures are
 * 4x larger and the SOC scene viewer needs to budget VRAM.
 */
function loadTextureCached(
  cache: Map<string, Promise<THREE.Texture | null>>,
  contractPath: string,
  _kind: "diffuse" | "normal",
): Promise<THREE.Texture | null> {
  const cached = cache.get(contractPath);
  if (cached) return cached;

  const promise = (async (): Promise<THREE.Texture | null> => {
    const candidates = buildTextureCandidates(contractPath);
    for (const candidate of candidates) {
      try {
        const result = await previewDds(candidate, 2);
        if (!result?.png || result.png.length === 0) continue;
        const blob = new Blob([new Uint8Array(result.png)], {
          type: "image/png",
        });
        if (typeof createImageBitmap === "function") {
          const bitmap = await createImageBitmap(blob);
          const texture = new THREE.Texture(
            bitmap as unknown as HTMLImageElement,
          );
          texture.needsUpdate = true;
          texture.wrapS = THREE.RepeatWrapping;
          texture.wrapT = THREE.RepeatWrapping;
          return texture;
        }
        // Fallback for environments without createImageBitmap.
        const url = URL.createObjectURL(blob);
        try {
          return await new Promise<THREE.Texture>((resolve, reject) => {
            new THREE.TextureLoader().load(
              url,
              (tex) => {
                tex.wrapS = THREE.RepeatWrapping;
                tex.wrapT = THREE.RepeatWrapping;
                resolve(tex);
              },
              undefined,
              (err) => reject(err),
            );
          });
        } finally {
          URL.revokeObjectURL(url);
        }
      } catch {
        // Try the next candidate.
        continue;
      }
    }
    console.warn("[soc-scene] texture resolve failed:", contractPath);
    return null;
  })();

  cache.set(contractPath, promise);
  return promise;
}

/**
 * Build the ordered list of p4k paths to try when resolving a texture
 * path from an MTL. MTL paths arrive in mixed forms:
 *   - "objects/foo/diffuse.tif"        (no Data prefix, .tif extension)
 *   - "objects/foo/diffuse.dds"        (already .dds)
 *   - "Data\\objects\\foo\\diffuse"    (no extension)
 * We normalise separators and try .dds first (the engine's runtime
 * extension), then the original.
 */
function buildTextureCandidates(rawPath: string): string[] {
  const normalised = rawPath.replace(/\//g, "\\");
  const withDataPrefix = normalised.toLowerCase().startsWith("data\\")
    ? normalised
    : `Data\\${normalised}`;
  const dot = withDataPrefix.lastIndexOf(".");
  const stem = dot >= 0 ? withDataPrefix.slice(0, dot) : withDataPrefix;
  const candidates = [
    `${stem}.dds`,
    withDataPrefix,
    `${stem}.tif`,
  ];
  // De-dupe while preserving order.
  return Array.from(new Set(candidates));
}
