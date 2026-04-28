// Renders a decomposed export package using Three.js.
//
// Loads `scene.json` from the package directory, then for every scene
// instance and interior placement fetches the referenced GLB mesh,
// material sidecar, and textures via the `decomposed_*` Tauri commands
// implemented in `src-tauri/src/decomposed_commands.rs`. Conversion logic
// (matrices, lights, materials) lives in `lib/decomposed-loader.ts` so
// this component stays presentational.
//
// Loading strategy:
//   - The Three.js render loop is started once at mount and never blocks
//     on loading. OrbitControls keeps responding to pan/zoom regardless
//     of how many meshes are in flight.
//   - Mesh / sidecar / texture fetches run in parallel through a small
//     semaphore (`runWithLimit`) so we never have more than `MAX_CONCURRENT`
//     IPC calls in flight. Each child resolves independently and is added
//     to the scene as soon as its mesh is parsed — sidecars/textures stream
//     in afterwards and self-update via `material.needsUpdate`.
//   - Between heavy batches we yield to the event loop with a
//     `requestAnimationFrame`-based microyield so input events have a
//     chance to drain.
//
// Limitations of the POC:
//   - Generic PBR fallback for all shader families. The Blender importer
//     has shader-specific node graphs we do not replicate here.
//   - No paint/livery palette substitution. Materials render with their
//     authored textures only.
//   - Lights use Three.js heuristics — exact Watt conversion is the
//     Blender importer's job; we use a conservative scalar.

import { useCallback, useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import {
  DECOMPOSED_CONTRACT_VERSION,
  loadDecomposedScene,
  loadDecomposedJson,
  loadDecomposedPaints,
  readDecomposedFile,
  type DecomposedPackageInfo,
} from "../lib/commands";
import {
  applyDebugFallbackToMaterial,
  applySCBasisToObject,
  buildLight,
  buildMaterial,
  clearMobiGlasMaterials,
  findSubmaterialIndexForGlbName,
  getMaterialMetrics,
  hologramMaterials,
  materialCacheKey,
  matrixFromOffsetEuler,
  matrixFromRows,
  resetMaterialMetrics,
  resolveContractPath,
  setDebugFallbackMode,
  stripCryEngineZUp,
  updateMobiGlasTime,
  type LightRecord,
  type MaterialBuildMetrics,
  type MaterialSidecar,
  type PaintVariant,
  type PaintsManifest,
  type RenderStyle,
  type SceneInstance,
  type SceneManifest,
  type SubmaterialRecord,
} from "../lib/decomposed-loader";
import { MaterialInspector, type PickHit } from "./material-inspector";
import { FlightCamHud } from "./flight-cam-hud";
import {
  dispatchViewerHotkey,
  useFlightCamera,
  type FlightCamHandle,
} from "../lib/flight-camera";
import {
  captureScreenshot,
  formatScreenshotFilename,
} from "../lib/screenshot";

/**
 * Live diagnostic overrides applied to the scene after construction.
 * Values are applied via scene traversal on every change; the triages
 * in decomposed-loader.ts remain as the construction-time baseline.
 * undefined means "do not override" (use whatever the material has).
 */
export interface DiagnosticSettings {
  /** envMapIntensity override for all non-glass, non-hologram materials. */
  envMapIntensity: number;
  /** renderer.toneMappingExposure */
  toneMappingExposure: number;
  /** metalness override for HardSurface-family materials (not glass, not hologram). */
  metalness: number;
  /** roughness override, applied only when roughnessOverrideEnabled is true. */
  roughness: number;
  /** Whether the roughness slider is active. */
  roughnessOverrideEnabled: boolean;
  /** clearcoat override for MeshPhysicalMaterial (HardSurface path). */
  clearcoat: number;
  /** AmbientLight / HemisphereLight intensity. */
  ambientIntensity: number;
  /** DirectionalLight intensity (all directional lights in scene). */
  directionalIntensity: number;
  /** Camera-attached ("headlight") intensity, if one exists. */
  headlightIntensity: number;
  /** Multiplier on HSL saturation for every material.color (1.0 = unchanged). */
  colorSaturation: number;
}

export const DEFAULT_DIAGNOSTIC_SETTINGS: DiagnosticSettings = {
  envMapIntensity: 0,
  toneMappingExposure: 0.85,
  metalness: 0.4,
  roughness: 0.5,
  roughnessOverrideEnabled: false,
  clearcoat: 0,
  ambientIntensity: 0.3,
  directionalIntensity: 0.6,
  headlightIntensity: 1.0,
  colorSaturation: 1.0,
};

interface Props {
  packageInfo: DecomposedPackageInfo;
  /** Presentation mode for materials. Switching rebuilds materials in
   *  place without re-loading meshes or textures. */
  renderStyle?: RenderStyle;
  /** Show or hide the 1km neutral ground plane. Defaults to true. The
   *  plane lives on `scene` (not `sceneRoot`) so it persists across
   *  package switches; toggling visibility here just flips
   *  `mesh.visible` on the existing geometry — no rebuild, no GC. */
  showGroundPlane?: boolean;
  /** Show or hide the GridHelper overlay. Defaults to true. */
  showGrid?: boolean;
  /** RGB color of the ground plane as [r, g, b] in 0-255 integer range. */
  groundPlaneColor?: [number, number, number];
  /** Selected paint/livery variant's `palette_id`, or null for the
   *  default (as-baked) livery. Switching rebuilds exterior materials
   *  in place — the swap loads the variant sidecar's textures on
   *  demand but reuses cached meshes and per-package state. */
  livery?: string | null;
  /** Live diagnostic overrides swept by the Settings panel sliders.
   *  Applied post-construction via scene traversal; the loader triages
   *  remain as the baseline. Omit to use defaults. */
  diagnostics?: DiagnosticSettings;
  /** Fired once per scene load with the list of paint variants the
   *  exporter wrote to `paints.json`. The list is per-ship and may be
   *  empty; the parent renders a livery dropdown only when at least
   *  one variant exists. */
  onPaints?: (variants: PaintVariant[]) => void;
  /** Display under the toolbar, optional. */
  onStatus?: (text: string) => void;
  /** Fired when the flight camera handle is ready (and again with
   *  null on unmount). The parent uses this to render the projection-
   *  mode picker inside its top-right toolbar instead of letting it
   *  collide with the toolbar's own controls. */
  onFlightCamReady?: (handle: FlightCamHandle | null) => void;
}

/** Per-mesh binding so we can rebuild materials in place when the
 *  user changes the render-style or the livery without re-fetching
 *  meshes. Each mesh remembers both its currently-active submaterial
 *  (the one driving its current Three.js material) and the original
 *  on-disk submaterial it loaded as. The default record lets us
 *  restore "no livery" without re-running the scene-load pipeline. */
interface MaterialBinding {
  mesh: THREE.Mesh;
  /** Currently-active sidecar path. Equals `defaultSidecarKey` when
   *  no livery is overriding this binding; equals the variant's
   *  `exterior_material_sidecar` when one is. */
  sidecarKey: string;
  /** Currently-active submaterial record (drives style switching). */
  submaterial: SubmaterialRecord;
  /** The on-disk sidecar this mesh's GLB material was originally
   *  resolved against. Used by the livery effect to identify exterior
   *  bindings (those whose default key matches the entity's root
   *  exterior sidecar) and to restore "no livery". */
  defaultSidecarKey: string;
  /** Original submaterial record, restored when the livery selection
   *  returns to default. */
  defaultSubmaterial: SubmaterialRecord;
}

/**
 * Cap on simultaneous Tauri IPC reads. Tauri serialises Vec<u8> over a
 * single IPC channel; pushing 2000+ requests at once stalls the channel
 * and starves the WebGL renderer. 8 keeps the pipeline full without
 * monopolising the bridge.
 */
const MAX_CONCURRENT = 8;

/** Yield to the event loop so input events / OrbitControls can drain. */
function microyield(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

interface LoadProgress {
  loaded: number;
  total: number;
  current: string;
}

export function SceneViewer({
  packageInfo,
  renderStyle = "textured",
  showGroundPlane = true,
  showGrid = true,
  groundPlaneColor = [128, 128, 128],
  livery = null,
  diagnostics = DEFAULT_DIAGNOSTIC_SETTINGS,
  onPaints,
  onStatus,
  onFlightCamReady,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const stateRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    sceneRoot: THREE.Group;
    groundMesh: THREE.Mesh;
    gridHelper: THREE.GridHelper;
    animId: number;
  } | null>(null);

  // Flight-camera wiring. The hook reads these refs once its effect runs
  // (which is after the bootstrap effect below populates them, since
  // effects fire top-down inside one component). Until then the handle
  // is null; the HUD and the reset-on-load effect short-circuit cleanly
  // on null.
  const rendererRef = useRef<THREE.WebGLRenderer | null>(null);
  const cameraRef = useRef<THREE.PerspectiveCamera | null>(null);
  const sceneRef = useRef<THREE.Scene | null>(null);
  const flightCamRef = useRef<FlightCamHandle | null>(null);

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<LoadProgress | null>(null);
  // Timestamp (performance.now()) recorded when the scene load starts,
  // used by the first-frame effect to emit [perf] first-frame.
  const loadStartRef = useRef<number>(0);
  const [stats, setStats] = useState<{
    instances: number;
    interiors: number;
    lights: number;
    metrics: MaterialBuildMetrics;
  } | null>(null);
  const generationRef = useRef(0);
  // Picker state. `pickHits` is the ordered list of meshes intersected
  // by the camera ray on the most recent left-click; `selectedHitIndex`
  // is the row currently expanded into the details panel. Both are
  // reset to empty/null when the panel's close button is pressed or a
  // new scene is loaded.
  const [pickHits, setPickHits] = useState<PickHit[]>([]);
  const [selectedHitIndex, setSelectedHitIndex] = useState<number | null>(null);
  // Debug-fallback mode: when on, the loader recolours fallback /
  // stand-in material paths into distinct neons so the user can
  // visually identify which case is firing on each surface. Toggling
  // triggers a material rebuild via the effect below; submaterials
  // that resolved correctly stay normal-coloured either way.
  const [debugFallbacks, setDebugFallbacks] = useState(false);
  const activeDebugRef = useRef(false);

  // Combined bottom-right panel (flight-cam HUD + scene-load metrics)
  // visibility. Hidden state collapses both sections to a tiny
  // re-expand affordance so the canvas reclaims that screen real
  // estate. Toggled by the H key and the in-panel button below.
  const [hudVisible, setHudVisible] = useState(true);

  // Persistent caches scoped to the current scene load. The bindings
  // list lets us rebuild materials in place when the render style
  // changes; the texture cache lets a re-bind reuse already-decoded
  // textures so the rebuild is fast.
  const bindingsRef = useRef<MaterialBinding[]>([]);
  const materialCacheRef = useRef<Map<string, THREE.Material>>(new Map());
  const textureCacheRef = useRef<Map<string, Promise<THREE.Texture | null>>>(
    new Map(),
  );
  // The active render style at the time of the last scene load, used by
  // the rebuild effect to detect when a switch is needed.
  const activeStyleRef = useRef<RenderStyle>(renderStyle);

  // ── Livery state, scoped to the loaded package ──
  //
  // The livery rebuild needs to run AFTER the scene is fully loaded,
  // so the texture-loader, sidecar-loader, and per-package texture
  // cache must outlive a single scene-load `useEffect` invocation.
  // We hold them in refs here so the livery effect (declared later)
  // can pull them out without participating in the scene-load
  // closure. All three are reset at the start of every scene load
  // and fall back to no-op behaviour when null.
  const loadTextureRef = useRef<
    ((path: string) => Promise<THREE.Texture | null>) | null
  >(null);
  const loadSidecarRef = useRef<
    ((path: string) => Promise<MaterialSidecar | null>) | null
  >(null);
  const paintVariantsRef = useRef<PaintVariant[]>([]);
  /** Path of the entity's root-entity `material_sidecar`. A binding is
   *  "exterior" iff its `defaultSidecarKey` matches this string —
   *  paint-variant overrides only swap sidecars that already pointed at
   *  the root exterior. Landing gears and other shared exterior parts
   *  reference the same root sidecar by design, so they get re-painted
   *  along with the hull. Null when scene.json had no root sidecar. */
  const defaultExteriorSidecarRef = useRef<string | null>(null);
  /** The active livery at the time of the last scene load / rebuild,
   *  used by the livery effect to detect when a switch is needed and
   *  avoid double-applying the same selection. */
  const activeLiveryRef = useRef<string | null>(livery);
  /** Mirror of the `livery` prop, kept current via the effect below.
   *  Lets the scene-load closure read the latest value at the moment
   *  it kicks the initial-livery apply (after paints.json lands), so
   *  a user selection made between mount and paints-fetch still wins. */
  const liveryPropRef = useRef<string | null>(livery);
  useEffect(() => {
    liveryPropRef.current = livery;
  }, [livery]);

  // -- Three.js bootstrap (runs once; render loop never blocks on loads) --
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const renderer = new THREE.WebGLRenderer({
      antialias: true,
      powerPreference: "high-performance",
    });
    renderer.setClearColor(0x101418);
    // Three.js r183 defaults to LinearSRGBColorSpace output and NoToneMapping,
    // which emits linear floats into a framebuffer the OS treats as sRGB.
    // Result: dark, oversaturated, hue-shifted image, plus any HDR value above
    // 1.0 clamps hard. Use sRGB output and ACES filmic tonemapping so scenes
    // look like every other modern PBR viewer.
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    // Exposure tuned for ship exteriors lit by IBL + a single key light.
    // 1.0 (the comfortable default) reads as "outdoors at noon" against
    // the procedural RoomEnvironment, which blows out highlights on
    // metallic hull paint and produces the blue/white rim visual the
    // user reported. 0.85 brings overall luminance down ~15% so actual
    // material colours read instead of IBL-saturated highlights.
    renderer.toneMappingExposure = 0.85;
    // High-DPI displays (4K / retina) push 2-3x the pixel count, which kills
    // frame rate during pan with no visible benefit past ~1.5x. Clamp.
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
    // Publish the bootstrapped objects so `useFlightCamera` (which runs its
    // effect after this one) can attach. The hook owns the per-frame input
    // loop that previously belonged to OrbitControls; the render loop below
    // just calls `renderer.render(scene, camera)` and trusts the hook to
    // keep the camera transform current.
    rendererRef.current = renderer;
    cameraRef.current = camera;
    sceneRef.current = scene;

    // Fallback lighting so the model is visible even before scene lights
    // load. The IBL below provides most of the ambient/diffuse fill via
    // `scene.environment`, so the hemisphere intensity is low — keeping
    // it nonzero just hedges against an envmap that doesn't carry the
    // sky/ground gradient we want.
    const hemi = new THREE.HemisphereLight(0xb1bfd8, 0x202428, 0.3);
    hemi.name = "fallback_hemi";
    scene.add(hemi);
    // Sky-direction key light. Positioned high and to one side so PBR
    // surfaces show meaningful highlights and self-shadowing cues without
    // any actual shadow-map cost. Intensity 0.6 reads as a moderate
    // outdoor key — 1.5 (the prior value) overdrove the IBL and
    // produced the white/blue/pink rim visual the user reported on
    // Aurora hull paints.
    const dir = new THREE.DirectionalLight(0xffffff, 0.6);
    dir.position.set(50, 100, 25);
    dir.name = "fallback_dir";
    scene.add(dir);

    // Image-based lighting. PMREMGenerator pre-filters a procedural
    // RoomEnvironment scene into a mipmap chain that
    // MeshStandardMaterial / MeshPhysicalMaterial sample for ambient
    // reflection at every roughness. `scene.environment` opts every PBR
    // material in automatically; we deliberately leave
    // `scene.background` unset so the dark clearColor stays as the
    // backdrop.
    //
    // Swap-out: to use a real HDRI later, replace the `fromScene` call
    // with `pmremGenerator.fromEquirectangular(hdrTexture)` -- the
    // downstream wiring (`scene.environment = ...`) is identical, so
    // changing the source is a one-line edit. RGBELoader from
    // `three/addons/loaders/RGBELoader.js` reads `.hdr` files in the
    // format that produces the right input for `fromEquirectangular`.
    const pmremGenerator = new THREE.PMREMGenerator(renderer);
    const roomEnv = new RoomEnvironment();
    const envTexture = pmremGenerator.fromScene(roomEnv, 0.04).texture;
    scene.environment = envTexture;
    // RoomEnvironment is bright by default — its built-in lights are
    // sized for "Three.js demo room" lighting, not "outdoors." On
    // metallic hull paint with the ACES tone map, full intensity
    // dominates the actual material colour and reads as a mirror
    // reflecting the procedural sky. 0.5 keeps the ambient fill but
    // halves the specular reflection that was washing out paints.
    scene.environmentIntensity = 0.5;

    // Ground plane. Sits 100m below world origin, 1km square, neutral
    // matte. Receives shadows in case we wire shadow mapping later. Added
    // to `scene` (not `sceneRoot`) so it persists across package switches
    // — sceneRoot is cleared on every load — and is therefore excluded
    // from the camera-fit bounds calculation in `fitCamera`, which walks
    // sceneRoot only.
    const groundGeometry = new THREE.PlaneGeometry(1000, 1000);
    const groundMaterial = new THREE.MeshStandardMaterial({
      color: 0x808080,
      roughness: 0.9,
      metalness: 0.0,
    });
    const ground = new THREE.Mesh(groundGeometry, groundMaterial);
    ground.name = "ground_plane";
    ground.rotation.x = -Math.PI / 2;
    ground.position.y = -100;
    ground.receiveShadow = true;
    // Apply the initial visibility from props. The dedicated effect below
    // keeps it in sync as the prop changes; setting it here too avoids a
    // one-frame flash of the plane when the user mounts the viewer with
    // ground hidden.
    ground.visible = showGroundPlane;
    scene.add(ground);

    // Helpers
    const grid = new THREE.GridHelper(200, 40, 0x444444, 0x222222);
    grid.name = "grid_helper";
    grid.visible = showGrid;
    scene.add(grid);
    const axes = new THREE.AxesHelper(5);
    scene.add(axes);

    const sceneRoot = new THREE.Group();
    sceneRoot.name = "decomposed_root";
    // Apply the Z-up -> Y-up basis change once at the scene root.
    // Loaded GLBs are normalised to raw CryEngine Z-up basis at parse
    // time (see `stripCryEngineZUp` in decomposed-loader.ts) so all
    // inter-mesh transforms in scene.json compose consistently in
    // CryEngine basis before this final rotation flips the whole scene
    // into Three.js Y-up.
    applySCBasisToObject(sceneRoot);
    scene.add(sceneRoot);

    let animId = 0;
    // Perf instrumentation. Logged every 2s; grep `[perf]` in the console.
    // Strip when render-on-demand lands.
    const PERF_LOG_INTERVAL_MS = 2000;
    const perfWindow: { frameMs: number; renderMs: number }[] = [];
    let lastFrameTs = performance.now();
    let lastLogTs = lastFrameTs;
    // Renderer GPU info: use UNMASKED_VENDOR/RENDERER via WEBGL_debug_renderer_info
    // so we can verify the webview is actually using the dGPU and not iGPU.
    const gl = renderer.getContext();
    const dbgInfo = gl.getExtension("WEBGL_debug_renderer_info");
    const gpuVendor = dbgInfo ? gl.getParameter(dbgInfo.UNMASKED_VENDOR_WEBGL) : "unknown";
    const gpuRenderer = dbgInfo ? gl.getParameter(dbgInfo.UNMASKED_RENDERER_WEBGL) : "unknown";
    console.log(
      `[perf] init three=${THREE.REVISION} ship=${packageInfo.package_name} gpu_vendor=${gpuVendor} gpu_renderer=${gpuRenderer}`,
    );
    const animate = () => {
      animId = requestAnimationFrame(animate);
      const frameStart = performance.now();
      const frameMs = frameStart - lastFrameTs;
      lastFrameTs = frameStart;

      // Tick hologram materials' `time` uniform so the scanline /
      // shimmer animation runs. The `hologramMaterials` array is
      // populated by `buildHologramMaterial` per package load; we
      // assume it's append-only and walk it directly.
      const elapsedSec = frameStart * 0.001;
      for (let i = 0; i < hologramMaterials.length; i += 1) {
        const u = hologramMaterials[i].uniforms.time;
        if (u) u.value = elapsedSec;
      }
      // Same lifecycle for MobiGlas-style materials, which use a
      // separate Set registry rather than the array used by holograms.
      updateMobiGlasTime(elapsedSec);

      const renderStart = performance.now();
      // Render with whichever camera the flight cam currently exposes
      // (perspective, orthographic, or oblique). Until the flight-cam
      // hook attaches on the next effect pass, `flightCamRef.current`
      // is null and we fall back to the perspective camera so the very
      // first frame still renders.
      const activeCam = flightCamRef.current?.getActiveCamera() ?? camera;
      renderer.render(scene, activeCam);
      const renderEnd = performance.now();

      perfWindow.push({
        frameMs,
        renderMs: renderEnd - renderStart,
      });

      if (renderEnd - lastLogTs >= PERF_LOG_INTERVAL_MS) {
        const n = perfWindow.length;
        if (n > 0) {
          let sumFrame = 0, sumRender = 0, maxRender = 0;
          for (const s of perfWindow) {
            sumFrame += s.frameMs;
            sumRender += s.renderMs;
            if (s.renderMs > maxRender) maxRender = s.renderMs;
          }
          const avgFrame = sumFrame / n;
          const fps = avgFrame > 0 ? 1000 / avgFrame : 0;
          const info = renderer.info;
          const mem = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
          const heap = mem ? ` heap=${Math.round(mem.usedJSHeapSize / 1024 / 1024)}MB` : "";
          console.log(
            `[perf] fps=${fps.toFixed(1)} frame_avg=${avgFrame.toFixed(2)}ms ` +
              `render_avg=${(sumRender / n).toFixed(2)}ms render_max=${maxRender.toFixed(2)}ms ` +
              `draw_calls=${info.render.calls} tris=${info.render.triangles} ` +
              `textures=${info.memory.textures} geometries=${info.memory.geometries} ` +
              `programs=${info.programs?.length ?? 0}${heap} samples=${n}`,
          );
        }
        perfWindow.length = 0;
        lastLogTs = renderEnd;
      }
    };
    animate();

    stateRef.current = {
      renderer,
      scene,
      camera,
      sceneRoot,
      groundMesh: ground,
      gridHelper: grid,
      animId,
    };

    const ro = new ResizeObserver(() => {
      const w = container.clientWidth;
      const h = container.clientHeight;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    });
    ro.observe(container);

    // Picker: detect a no-drag left click on the canvas, raycast
    // through the cursor, and surface the ordered hit list to the
    // inspector panel. Listening at pointerdown/up rather than "click"
    // lets us keep the down-position and disambiguate pick from
    // orbit-drag without fighting OrbitControls.
    const raycaster = new THREE.Raycaster();
    const ndc = new THREE.Vector2();
    let downX = 0;
    let downY = 0;
    let downT = 0;
    const onPointerDown = (e: PointerEvent): void => {
      if (e.button !== 0) return;
      downX = e.clientX;
      downY = e.clientY;
      downT = performance.now();
    };
    const onPointerUp = (e: PointerEvent): void => {
      if (e.button !== 0) return;
      const dx = e.clientX - downX;
      const dy = e.clientY - downY;
      const dt = performance.now() - downT;
      // 5px / 350ms thresholds: large enough that a relaxed click
      // registers, small enough that an orbit drag never triggers
      // a pick. Tuned to feel like a "tap" rather than a click.
      if (dx * dx + dy * dy > 25) return;
      if (dt > 350) return;
      const rect = renderer.domElement.getBoundingClientRect();
      ndc.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      ndc.y = -(((e.clientY - rect.top) / rect.height) * 2 - 1);
      // Read the active camera per-pick so raycasting tracks projection
      // mode without coupling the flight cam to the picker. THREE's
      // Raycaster.setFromCamera handles PerspectiveCamera and the
      // built-in OrthographicCamera natively. The oblique mode
      // post-multiplies a custom shear onto the ortho's
      // projectionMatrix; setFromCamera builds rays from
      // projectionMatrixInverse, which the hook keeps in sync, so the
      // ray direction reflects the sheared view. Picker accuracy in
      // oblique mode has not been visually validated yet.
      const pickCam = flightCamRef.current?.getActiveCamera() ?? camera;
      raycaster.setFromCamera(ndc, pickCam);
      // Intersect against the scene root (the model). Skip the ground
      // mesh — its hits are noise for material inspection. recursive
      // = true walks the whole subtree.
      const hits = raycaster.intersectObject(sceneRoot, true);
      const bindings = bindingsRef.current;
      const bindingByMesh = new Map<THREE.Mesh, MaterialBinding>();
      for (const b of bindings) bindingByMesh.set(b.mesh, b);
      const pickList: PickHit[] = [];
      let idx = 0;
      for (const hit of hits) {
        // Only meshes carry materials; lines/points etc. would never
        // bind to a SubmaterialRecord, and the user wouldn't be
        // clicking those for material info.
        if (!(hit.object instanceof THREE.Mesh)) continue;
        const binding = bindingByMesh.get(hit.object);
        pickList.push({
          hitIndex: idx++,
          distance: hit.distance,
          mesh: hit.object,
          point: hit.point.clone(),
          submaterial: binding?.submaterial ?? null,
          sidecarKey: binding?.sidecarKey ?? null,
          defaultSidecarKey: binding?.defaultSidecarKey ?? null,
        });
      }
      setPickHits(pickList);
      setSelectedHitIndex(pickList.length > 0 ? 0 : null);
    };
    renderer.domElement.addEventListener("pointerdown", onPointerDown);
    renderer.domElement.addEventListener("pointerup", onPointerUp);

    return () => {
      ro.disconnect();
      renderer.domElement.removeEventListener("pointerdown", onPointerDown);
      renderer.domElement.removeEventListener("pointerup", onPointerUp);
      cancelAnimationFrame(animId);
      // Release the IBL render target and the ground mesh's geometry /
      // material; created once at init so they live as long as the
      // renderer does. The flight-cam hook owns its own dispose path
      // (its effect cleanup runs alongside this one).
      pmremGenerator.dispose();
      envTexture.dispose();
      groundGeometry.dispose();
      groundMaterial.dispose();
      renderer.dispose();
      if (renderer.domElement.parentElement === container) {
        container.removeChild(renderer.domElement);
      }
      rendererRef.current = null;
      cameraRef.current = null;
      sceneRef.current = null;
      stateRef.current = null;
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
  // projection-mode picker in its top-right toolbar (the in-canvas
  // standalone slot collided with the Livery + Style + Settings strip).
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
      e.preventDefault();
      const scene = sceneRef.current;
      const baseRenderer = rendererRef.current;
      const camera =
        flightCamRef.current?.getActiveCamera() ?? cameraRef.current;
      if (!scene || !baseRenderer || !camera) return;
      const projectionMode = flightCamRef.current?.getState().projectionMode;
      const filename = formatScreenshotFilename(
        packageInfo.package_name ?? null,
        new Date(),
      );
      captureScreenshot({
        scene,
        camera,
        baseRenderer,
        filename,
        projectionMode,
        onProgress: (phase, info) => {
          console.info(
            `[screenshot] phase=${phase}${info ? ` ${info}` : ""}`,
          );
        },
      }).catch((err) => {
        console.error("[screenshot] handler caught:", err);
      });
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [packageInfo.package_name]);

  // R reframes the camera to the current sceneRoot. Numpad 0-5 snap to
  // named view presets (overhead / perspective2 / side / fore / aft /
  // perspective). H toggles the combined HUD + stats panel. The
  // dispatch table lives in `dispatchViewerHotkey` so it stays unit-
  // testable without a DOM. Listener lives on `window` so it fires
  // regardless of which child element holds focus, except for typing
  // targets where the binding is suppressed.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent): void => {
      const ae = document.activeElement;
      const tag = ae?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      const sceneRoot = stateRef.current?.sceneRoot ?? null;
      const handled = dispatchViewerHotkey(
        { code: e.code, repeat: e.repeat },
        flightCamRef.current,
        sceneRoot,
        () => setHudVisible((v) => !v),
      );
      if (handled) e.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  // Rebuild exterior bindings against either the variant's substitute
  // sidecar (`palette_id !== null && variant.exterior_material_sidecar`)
  // or the original on-disk sidecar (default). Walks every recorded
  // binding once: bindings whose `defaultSidecarKey` doesn't match the
  // entity's root exterior sidecar are skipped (they're shared
  // components, decals, interiors — not paint-bearing). Submaterial
  // matching falls back through `submaterial_name` then `index` so the
  // variant sidecar can be authored against the same MTL roster as the
  // default with minor reordering.
  //
  // Stable identity (empty deps) — reads only refs and pure helpers.
  // The load effect captures this for its initial-livery apply path.
  const applyLiveryRebuild = useCallback(
    async (nextLivery: string | null): Promise<void> => {
      const bindings = bindingsRef.current;
      if (bindings.length === 0) {
        activeLiveryRef.current = nextLivery;
        return;
      }
      const exteriorKey = defaultExteriorSidecarRef.current;
      if (!exteriorKey) {
        // No root exterior sidecar means there's nothing this view can
        // paint differently. Surface the no-op cleanly.
        activeLiveryRef.current = nextLivery;
        return;
      }
      const loadSidecar = loadSidecarRef.current;
      const loadTexture = loadTextureRef.current;
      if (!loadSidecar || !loadTexture) {
        // Scene load hasn't published the loaders yet. The trailing
        // initial-livery apply at the tail of the load effect, or the
        // user's next selection, will retry.
        return;
      }

      // Resolve the variant. A null livery means restore default.
      const variant: PaintVariant | null = nextLivery
        ? paintVariantsRef.current.find((v) => v.palette_id === nextLivery) ?? null
        : null;
      const overridePath = variant?.exterior_material_sidecar ?? null;

      // Pre-load the override sidecar once. Sidecar fetches are
      // deduped per-package via `loadSidecarRef`, so subsequent
      // re-applications of the same livery are free.
      const overrideSidecar: MaterialSidecar | null = overridePath
        ? await loadSidecar(overridePath)
        : null;
      if (overridePath && !overrideSidecar) {
        console.warn(
          "[scene-viewer] livery override sidecar failed to load:",
          overridePath,
        );
      }

      // Per-rebuild material cache keyed by `materialCacheKey`. Two
      // exterior meshes that resolve to the same submaterial share a
      // freshly-built material so draw-call count and shader-program
      // count don't multiply with mesh count.
      const style = activeStyleRef.current;
      const newCache = new Map<string, THREE.Material>();
      const usedKeys = new Set<string>();
      let exteriorBindings = 0;
      let overridden = 0;

      for (const binding of bindings) {
        if (binding.defaultSidecarKey !== exteriorKey) {
          // Non-exterior binding (interior, hardpoint child with its
          // own sidecar, etc.). Leave it alone.
          const key = materialCacheKey(binding.submaterial, style);
          usedKeys.add(key);
          newCache.set(key, binding.mesh.material as THREE.Material);
          continue;
        }
        exteriorBindings += 1;

        // Pick the submaterial record this binding should now use.
        // Override sidecar takes precedence; falls back to default
        // when the override doesn't carry a matching entry (the
        // variant authored a different roster) or when the user
        // restored "Default".
        let target: SubmaterialRecord = binding.defaultSubmaterial;
        let activeSidecarKey = binding.defaultSidecarKey;
        if (overrideSidecar?.submaterials?.length) {
          const submats = overrideSidecar.submaterials;
          const defaultName = binding.defaultSubmaterial.submaterial_name;
          let idx = -1;
          if (defaultName) {
            idx = submats.findIndex((sm) => sm.submaterial_name === defaultName);
          }
          if (idx < 0 && typeof binding.defaultSubmaterial.index === "number") {
            // Index fallback for variants that renamed but preserved
            // positional alignment with the default sidecar's roster.
            const defaultIdx = binding.defaultSubmaterial.index;
            idx = submats.findIndex((sm) => sm.index === defaultIdx);
          }
          if (idx >= 0) {
            target = submats[idx];
            activeSidecarKey = overridePath ?? binding.defaultSidecarKey;
            overridden += 1;
          }
        }

        const cacheKey = materialCacheKey(target, style);
        let material = newCache.get(cacheKey);
        if (!material) {
          const built = buildMaterial(target, loadTexture, style);
          material = built.material;
          newCache.set(cacheKey, material);
        }
        binding.mesh.material = material;
        binding.submaterial = target;
        binding.sidecarKey = activeSidecarKey;
        usedKeys.add(cacheKey);
      }

      // Dispose materials from the prior cache that no longer have a
      // referrer. We can't blanket-dispose the whole previous cache:
      // non-exterior bindings still reference their entries verbatim
      // (we copied them into `newCache` above), so disposing those
      // would leave naked GPU handles on the live meshes.
      for (const [key, mat] of materialCacheRef.current) {
        if (!usedKeys.has(key)) mat.dispose();
      }
      materialCacheRef.current = newCache;
      activeLiveryRef.current = nextLivery;

      console.info(
        `[scene-viewer] livery rebuild: variant=${variant?.palette_id ?? "default"} ` +
          `exteriorBindings=${exteriorBindings} overridden=${overridden}`,
      );
    },
    [],
  );

  // -- Scene load on packageInfo change --
  useEffect(() => {
    const state = stateRef.current;
    if (!state) return;
    const gen = ++generationRef.current;
    loadStartRef.current = performance.now();
    setLoading(true);
    setError(null);
    setStats(null);
    setProgress(null);
    setPickHits([]);
    setSelectedHitIndex(null);
    resetMaterialMetrics();

    // Clear previous scene. The basis rotation lives on the sceneRoot
    // itself, not its children, so clearing children preserves it.
    clearObject(state.sceneRoot);
    // Drop bindings now so a stale render-style switch arriving before
    // the new scene loads doesn't try to re-bind disposed meshes.
    bindingsRef.current = [];
    // Dispose any materials cached from the previous scene before we
    // overwrite the reference; meshes that referenced them have been
    // disposed by clearObject above.
    for (const old of materialCacheRef.current.values()) {
      old.dispose();
    }
    materialCacheRef.current = new Map();
    // Reset livery state — the new package has its own paint variants
    // and exterior sidecar. The loadTexture / loadSidecar refs are
    // re-populated below once the per-package loaders are constructed.
    loadTextureRef.current = null;
    loadSidecarRef.current = null;
    paintVariantsRef.current = [];
    defaultExteriorSidecarRef.current = null;
    activeLiveryRef.current = livery;
    onPaints?.([]);

    void (async () => {
      // Phase-timing marks. Cleared at the start of each load so marks
      // from a prior run don't pollute the final summary.
      const perfBase = `load:${packageInfo.package_name}`;
      performance.clearMarks(perfBase + ":start");
      performance.mark(perfBase + ":start");
      const perfT0 = performance.now();

      try {
        onStatus?.("Loading scene.json...");
        const manifest = (await loadDecomposedScene(packageInfo.package_dir)) as SceneManifest;
        if (gen !== generationRef.current) return;
        const perfAfterScene = performance.now();

        // Refuse stale exports up front. The loader assumes contract v2
        // semantics (GLB material.extras.submat_index, MtlName-first
        // sidecar resolution). Pre-v2 exports lack the engine-truth
        // submaterial cross-reference and bind GLB materials to whatever
        // sidecar the writer happened to emit -- which on
        // wrong-sidecar-bug entities (gimbal mounts, KLWE laser repeaters,
        // shared component meshes) is a different MTL than the GLB's
        // material names came from. Without this gate, the symptom is a
        // flood of "[decomposed-loader] no sidecar submaterial matches GLB
        // material ..." warnings and a whitebox scene that no amount of
        // shader / material work in the loader can fix. A version mismatch
        // is data-staleness, not a bug to paper over -- surface it loudly
        // so the user re-exports.
        const manifestContract = manifest.contract_version ?? 0;
        if (manifestContract !== DECOMPOSED_CONTRACT_VERSION) {
          throw new Error(
            `Stale export: scene.json contract_version=${manifestContract}, ` +
              `loader expects v${DECOMPOSED_CONTRACT_VERSION}. ` +
              `Re-export this entity (Scene Viewer toolbar -> RotateCw on the row, ` +
              `or 'Clear all' to wipe and restart) so the GLB writer emits ` +
              `extras.submat_index and the sidecar resolves via the engine-truth ` +
              `MtlName chunk. Loading older packages would silently bind GLB ` +
              `materials to the wrong sidecar's submaterials and render a ` +
              `whitebox scene.`,
          );
        }

        const exportRoot = packageInfo.export_root;
        const gltfLoader = new GLTFLoader();
        const meshCache = new Map<string, Promise<THREE.Group>>();
        const sidecarCache = new Map<string, Promise<MaterialSidecar | null>>();
        // Reset persistent caches for the new scene load.
        bindingsRef.current = [];
        materialCacheRef.current = new Map<string, THREE.Material>();
        textureCacheRef.current = new Map<string, Promise<THREE.Texture | null>>();
        // Drop holograms from the previous package so the animate loop
        // doesn't tick orphan ShaderMaterials that point at disposed
        // resources. The new package will repopulate as its hologram
        // materials are built.
        hologramMaterials.length = 0;
        clearMobiGlasMaterials();
        const textureCache = textureCacheRef.current;
        activeStyleRef.current = renderStyle;

        // ---- Loaders (cached, deduped) ----

        const loadMesh = (contractPath: string): Promise<THREE.Group> => {
          const cached = meshCache.get(contractPath);
          if (cached) return cached.then((g) => g.clone());
          const promise = readDecomposedFile(
            resolveContractPath(exportRoot, contractPath),
          ).then(
            (buf) =>
              new Promise<THREE.Group>((resolve, reject) => {
                gltfLoader.parse(
                  buf,
                  "",
                  // Strip the GLB-level CryEngine_Z_up wrapper so the
                  // returned tree is in raw CryEngine basis. The
                  // sceneRoot applies the single basis change for the
                  // whole assembled scene.
                  (gltf) => {
                    stripCryEngineZUp(gltf.scene);
                    // CryEngine packs damage / wear / dirt masks into the
                    // mesh COLOR_0 attribute, NOT actual vertex colours.
                    // GLTFLoader auto-enables `vertexColors: true` on every
                    // material whose primitive carries COLOR_0, which then
                    // multiplies the material colour by those mask values
                    // per-fragment. The result is bright magenta / red
                    // splotches wherever a mask channel is non-zero. Strip
                    // the attribute and disable the flag on every material.
                    stripMaskVertexColors(gltf.scene);
                    resolve(gltf.scene);
                  },
                  (err) => reject(err),
                );
              }),
          );
          meshCache.set(contractPath, promise);
          return promise.then((g) => g.clone());
        };

        const loadSidecar = (contractPath: string): Promise<MaterialSidecar | null> => {
          const cached = sidecarCache.get(contractPath);
          if (cached) return cached;
          const promise = loadDecomposedJson(
            resolveContractPath(exportRoot, contractPath),
          ).then((v) => v as MaterialSidecar | null).catch(() => null);
          sidecarCache.set(contractPath, promise);
          return promise;
        };

        const loadTexture = (contractPath: string): Promise<THREE.Texture | null> => {
          const cached = textureCache.get(contractPath);
          if (cached) return cached;
          const promise = (async (): Promise<THREE.Texture | null> => {
            try {
              const buf = await readDecomposedFile(
                resolveContractPath(exportRoot, contractPath),
              );
              const mime = guessMime(contractPath);
              if (mime === "application/octet-stream") {
                // DDS / KTX / unknown — TextureLoader can't handle these
                // generically. Skip rather than spawn a doomed decode.
                return null;
              }
              const blob = new Blob([buf], { type: mime });
              // createImageBitmap offloads PNG/JPEG decode to a worker
              // thread, so it doesn't block the renderer.
              if (typeof createImageBitmap === "function") {
                const bitmap = await createImageBitmap(blob);
                const texture = new THREE.Texture(bitmap as unknown as HTMLImageElement);
                texture.needsUpdate = true;
                return texture;
              }
              // Fallback: synchronous Image() decode on main thread.
              const url = URL.createObjectURL(blob);
              try {
                return await new Promise<THREE.Texture>((resolve, reject) => {
                  new THREE.TextureLoader().load(
                    url,
                    (tex) => resolve(tex),
                    undefined,
                    (err) => reject(err),
                  );
                });
              } finally {
                URL.revokeObjectURL(url);
              }
            } catch (err) {
              console.warn("[scene-viewer] texture load failed:", contractPath, err);
              return null;
            }
          })();
          textureCache.set(contractPath, promise);
          return promise;
        };

        // Publish the per-package loaders so the livery effect (declared
        // outside this useEffect) can re-fetch the variant sidecar and
        // its textures using the same caches the initial load populated.
        loadTextureRef.current = loadTexture;
        loadSidecarRef.current = loadSidecar;
        // Record the entity's root exterior sidecar. The livery override
        // only swaps bindings whose default sidecar matches this — that
        // covers the hull plus shared exterior parts (landing gears on
        // the Mustang, etc.) that intentionally reference the same
        // sidecar in the as-baked export.
        defaultExteriorSidecarRef.current =
          manifest.root_entity?.material_sidecar ?? null;

        // ---- Material binding (fire-and-forget; textures self-update) ----
        //
        // Texture loads run as Promise.allSettled in the background and
        // re-bind via material.needsUpdate when they resolve. We do not
        // await them before adding the mesh to the scene; the mesh will
        // render with the fallback PBR material until its textures land.
        const applySidecarMaterials = async (
          group: THREE.Group,
          sidecarKey: string,
          sidecar: MaterialSidecar | null,
        ): Promise<Promise<void>[]> => {
          if (!sidecar?.submaterials || sidecar.submaterials.length === 0) {
            return [];
          }
          const style = activeStyleRef.current;
          const submats = sidecar.submaterials;
          const newTexturePromises: Promise<void>[] = [];

          // Build (or reuse from cache) one material per submaterial. Two
          // instances of the same submaterial share one material and one
          // shader program, which matters when a scene mounts hundreds of
          // hardpoint copies of the same component.
          const materials: THREE.Material[] = submats.map((sm) => {
            const cacheKey = materialCacheKey(sm, style);
            const cached = materialCacheRef.current.get(cacheKey);
            if (cached) return cached;
            const built = buildMaterial(sm, loadTexture, style);
            materialCacheRef.current.set(cacheKey, built.material);
            for (const p of built.texturePromises) newTexturePromises.push(p);
            return built.material;
          });

          // Bind materials by reading the engine-truth submaterial index
          // the Rust GLB writer emits as `material.extras.submat_index`.
          // GLTFLoader merges extras directly into `material.userData`
          // via `Object.assign(material.userData, extras)`, so the value
          // lands at `userData.submat_index` — a single property lookup,
          // no name parsing, no fallback ladder. Names diverge across
          // naming spaces (MTL-source vs Blender-semantic, `_mtl_` vs
          // `:` vs bare-name) and Three.js may clone-and-suffix names
          // for uniqueness — none of that matters because we go straight
          // to the index.
          //
          // Falls back to name-matching for older cached exports written
          // before the `submat_index` extra existed (pre contract v2);
          // those slots are auto-orphaned by `pruneStaleCache` on app
          // mount, so this fallback only kicks in during the brief
          // window where a stale GLB is processed before the new export
          // overwrites it.
          group.traverse((child) => {
            if (!(child instanceof THREE.Mesh)) return;
            const childMat = child.material as THREE.Material | undefined;
            if (!childMat) return;
            const submatIndex = childMat.userData?.submat_index;
            let idx =
              typeof submatIndex === "number" && submatIndex >= 0 && submatIndex < submats.length
                ? submatIndex
                : -1;
            if (idx < 0) {
              idx = findSubmaterialIndexForGlbName(childMat.name ?? "", submats);
            }
            if (idx >= 0) {
              child.material = materials[idx];
              // Skip meshes whose activation_state is inactive (e.g.
              // stencil-float decals authored as opt-in geometry).
              if (submats[idx].activation_state?.state === "inactive") {
                child.visible = false;
              }
              bindingsRef.current.push({
                mesh: child,
                sidecarKey,
                submaterial: submats[idx],
                // The initial load is "no livery active" -- current and
                // default coincide. The livery effect updates the active
                // pair and consults `defaultSidecarKey` to identify
                // which bindings are exterior candidates.
                defaultSidecarKey: sidecarKey,
                defaultSubmaterial: submats[idx],
              });
            }
          });
          return newTexturePromises;
        };

        // ---- Build a flat unit-of-work list, then run with a semaphore ----
        //
        // Children are attached to their parent's named bone (matched by
        // `parent_node_name` against any Object3D within the parent's
        // loaded mesh group). Because mesh loads run concurrently, a
        // child may be ready before its parent. We resolve this with a
        // per-entity Promise registry: each unit publishes its loaded
        // group via `entityResolver`, and child units await
        // `entityWaiters.get(parent_entity_name)` before attaching.
        // Multi-level (grandchild) attachments work because every loaded
        // mesh — root or child — registers itself.

        type MeshUnit = {
          label: string;
          meshAsset: string;
          sidecar?: string | null;
          /** Entity name used as the key when other units look up this one
           * as their parent. Empty for unkeyed units (e.g. interiors). */
          entityName: string;
          /**
           * How to attach the loaded group. `static` parents straight
           * into a known group with an optional transform. `bone` waits
           * for the parent entity to load, then matches a named bone
           * inside it; on failure it falls back to the scene root.
           */
          attach:
            | { kind: "static"; parent: THREE.Group; transform?: THREE.Matrix4 }
            | {
                kind: "bone";
                parentEntity: string;
                /** Human-readable parent label for diagnostics. The
                 *  `parentEntity` field is the registry key (e.g.
                 *  `id:23`) which is opaque on its own; `parentLabel`
                 *  carries the authored entity name so warnings remain
                 *  readable. */
                parentLabel: string;
                parentNodeName: string;
                /** Local matrix to apply once attached. */
                transform?: THREE.Matrix4;
                /**
                 * If true, the local_transform_sc was computed by the
                 * exporter as a *root-attached* transform (it includes
                 * the parent bone's world translation). On bone-resolved
                 * attach we must still place this at scene root, not
                 * inside the bone, or the translation gets composed
                 * with the bone's own world transform.
                 */
                rootAttachedTransform: boolean;
              };
        };

        const units: MeshUnit[] = [];

        // Per-entity-name registry. Producers `resolve` once their group
        // is loaded; consumers `await` the matching promise before
        // attaching themselves. We use one Deferred per entity.
        type Deferred<T> = { promise: Promise<T>; resolve: (v: T) => void };
        const makeDeferred = <T,>(): Deferred<T> => {
          let resolve!: (v: T) => void;
          const promise = new Promise<T>((r) => {
            resolve = r;
          });
          return { promise, resolve };
        };
        const entityResolvers = new Map<string, Deferred<THREE.Group>>();
        const ensureResolver = (key: string): Deferred<THREE.Group> => {
          let d = entityResolvers.get(key);
          if (!d) {
            d = makeDeferred<THREE.Group>();
            entityResolvers.set(key, d);
          }
          return d;
        };

        // Build registry keys. Producers and consumers must agree on
        // the same derivation. We prefer the unique per-export
        // `instance_id` (added with the contract's instance-id field)
        // over `entity_name`, because two scene instances can share the
        // same authored entity name (e.g. paired weapon mounts) and a
        // name-keyed registry would collapse them into one slot —
        // attaching every child to whichever sibling won the registry
        // race. When `instance_id` is absent (older exports), we fall
        // back to the entity name and warn so the operator knows
        // disambiguation is best-effort. The `id:` / `name:` prefix
        // keeps the two key spaces from colliding.
        let warnedLegacyKeyForExport = false;
        const instanceKey = (id: number | undefined, name: string | undefined): string => {
          if (typeof id === "number" && Number.isFinite(id)) {
            return `id:${id}`;
          }
          if (!warnedLegacyKeyForExport) {
            warnedLegacyKeyForExport = true;
            console.warn(
              "[scene-viewer] scene.json lacks `instance_id` on one or more " +
                "entries; falling back to entity_name-based attach matching. " +
                "Re-export to enable disambiguation of paired siblings.",
            );
          }
          return `name:${name ?? ""}`;
        };

        // Root entity
        const root = manifest.root_entity;
        if (root?.mesh_asset) {
          const rootName = root.entity_name ?? "root";
          // Root reserves instance_id 0 in the contract; pre-fill so
          // children that reference parent_instance_id = 0 resolve
          // even when older exports omit the explicit field on root.
          const rootKey = instanceKey(root.instance_id ?? 0, rootName);
          ensureResolver(rootKey); // pre-create so children can await
          units.push({
            label: rootName,
            meshAsset: root.mesh_asset,
            sidecar: root.material_sidecar,
            entityName: rootKey,
            attach: { kind: "static", parent: state.sceneRoot },
          });
        }

        // Children
        const children = manifest.children ?? [];
        for (const child of children) {
          if (!child.mesh_asset) continue;
          const childName = child.entity_name ?? "child";
          const childKey = instanceKey(child.instance_id, childName);
          const parentEntity = child.parent_entity_name ?? "";
          const parentNodeName = child.parent_node_name ?? "";
          // Resolve the parent registry key. When the export carries
          // `parent_instance_id` we use that directly; otherwise we
          // fall back to the parent's entity name (best-effort, with
          // the same caveat as above for collision-prone names).
          const parentKey = instanceKey(child.parent_instance_id, parentEntity);
          // Pre-create the child's resolver so grandchildren can await it.
          ensureResolver(childKey);
          // Only attempt bone-attach when we actually have both a parent
          // entity (or instance id) and a target bone name. Otherwise
          // fall back to root.
          const transform = instanceTransform(child);
          const hasParent =
            typeof child.parent_instance_id === "number" || parentEntity.length > 0;
          if (hasParent && parentNodeName) {
            ensureResolver(parentKey); // make sure the slot exists
            const noRotation = child.no_rotation === true;
            // Human-readable label for the parent. Prefer the authored
            // entity name; fall back to the bare key. Used only for
            // diagnostic warnings — the registry lookup uses parentKey.
            const parentLabel = parentEntity.length > 0 ? parentEntity : parentKey;
            units.push({
              label: childName,
              meshAsset: child.mesh_asset,
              sidecar: child.material_sidecar,
              entityName: childKey,
              attach: {
                kind: "bone",
                parentEntity: parentKey,
                parentLabel,
                parentNodeName,
                transform,
                rootAttachedTransform: noRotation,
              },
            });
          } else {
            units.push({
              label: childName,
              meshAsset: child.mesh_asset,
              sidecar: child.material_sidecar,
              entityName: childKey,
              attach: { kind: "static", parent: state.sceneRoot, transform },
            });
          }
        }

        // Interiors + their placements + lights. Interior containers do
        // not participate in bone-attach: they are CGF instances placed
        // by container/placement transforms, not by named-bone lookup.
        let totalLights = 0;
        const interiors = manifest.interiors ?? [];
        for (const interior of interiors) {
          const interiorGroup = new THREE.Group();
          interiorGroup.name = interior.name ?? "interior";
          if (interior.container_transform) {
            const m = matrixFromRows(interior.container_transform);
            interiorGroup.matrixAutoUpdate = false;
            interiorGroup.matrix.copy(m);
          }
          state.sceneRoot.add(interiorGroup);

          // Lights are cheap (no IO); add them up front so something is
          // visible while meshes stream in.
          for (const lightRecord of interior.lights ?? []) {
            const light = buildLight(lightRecord as LightRecord);
            if (!light) continue;
            interiorGroup.add(light);
            if (light instanceof THREE.SpotLight && light.target) {
              interiorGroup.add(light.target);
            }
            totalLights += 1;
          }

          for (const placement of interior.placements ?? []) {
            if (!placement.mesh_asset) continue;
            units.push({
              label: placement.cgf_path ?? interior.name ?? "placement",
              meshAsset: placement.mesh_asset,
              sidecar: placement.material_sidecar,
              entityName: "",
              attach: {
                kind: "static",
                parent: interiorGroup,
                transform: placement.transform
                  ? matrixFromRows(placement.transform)
                  : undefined,
              },
            });
          }
        }

        if (gen !== generationRef.current) return;

        // Resolve any registry slots that no unit will produce. This
        // happens when a child references a `parent_entity_name` that is
        // not present in the export (orphan reference). Without this,
        // the child would await its parent's promise forever.
        const producedKeys = new Set<string>();
        for (const u of units) {
          if (u.entityName) producedKeys.add(u.entityName);
        }
        const orphanKeys: string[] = [];
        for (const [key, def] of entityResolvers) {
          if (!producedKeys.has(key)) {
            // No producer; resolve with an empty placeholder so any
            // dependent child treats the bone lookup as "missing" and
            // falls back to scene root attachment.
            def.resolve(new THREE.Group());
            orphanKeys.push(key);
          }
        }

        // Registry summary diagnostic. Helps debug attach pairing without
        // having to instrument by hand. Logged once per scene load at
        // info level so it's visible but not spammy. When orphans exist,
        // upgrade to warn so the operator notices.
        const registrySummary = {
          units: units.length,
          producers: producedKeys.size,
          resolvers: entityResolvers.size,
          orphans: orphanKeys,
        };
        if (orphanKeys.length > 0) {
          console.warn("[scene-viewer] registry summary:", registrySummary);
        } else {
          console.info("[scene-viewer] registry summary:", registrySummary);
        }

        const perfAfterManifest = performance.now();

        const total = units.length;
        let loadedCount = 0;
        let instanceCount = 0;
        // Track all in-flight texture promises so we can wait for
        // visual completion before declaring the load done. Failures in
        // individual textures don't fail the load (allSettled).
        const textureWork: Promise<unknown>[] = [];
        // Frame the camera the first time we have something on screen.
        let framedOnce = false;

        setProgress({ loaded: 0, total, current: "" });

        const runUnit = async (unit: MeshUnit): Promise<void> => {
          if (gen !== generationRef.current) return;
          let publishedGroup: THREE.Group | null = null;
          try {
            const group = await loadMesh(unit.meshAsset);
            if (gen !== generationRef.current) return;
            group.name = unit.label;

            // Resolve the attachment target. For bone-attach units this
            // may need to wait for the parent mesh to load. We do the
            // wait BEFORE applying any transform / adding to the scene
            // because the local transform interpretation depends on
            // whether we land on the bone or the fallback root.
            let attachParent: THREE.Object3D = state.sceneRoot;
            let attachTransform: THREE.Matrix4 | undefined;
            if (unit.attach.kind === "static") {
              attachParent = unit.attach.parent;
              attachTransform = unit.attach.transform;
            } else {
              const parentDef = entityResolvers.get(unit.attach.parentEntity);
              if (!parentDef) {
                // Defensive: the unit-build pass calls
                // `ensureResolver(parentKey)` for every bone-attach
                // unit, so this should never fire. If it does, the
                // registry derivation has drifted between producer and
                // consumer — log loudly so the mismatch surfaces.
                console.warn(
                  "[scene-viewer] no parent resolver registered for key '%s' " +
                    "(parent label '%s'); attaching child '%s' to scene root",
                  unit.attach.parentEntity,
                  unit.attach.parentLabel,
                  unit.label,
                );
              }
              const parentGroup = parentDef ? await parentDef.promise : null;
              if (gen !== generationRef.current) return;
              const bone =
                parentGroup && unit.attach.parentNodeName
                  ? findNodeByName(parentGroup, unit.attach.parentNodeName)
                  : null;
              if (bone) {
                attachParent = bone;
                // local_transform_sc is bone-relative; apply it directly.
                attachTransform = unit.attach.transform;
              } else {
                // No matching bone — fall back to scene root. The
                // exporter's `local_transform_sc` already encodes the
                // appropriate world placement in this case (either
                // identity if it was a pure bone-attached child whose
                // bone could not be found, or the resolved root-attached
                // matrix when no_rotation=true). We still warn so the
                // operator can spot missing bones.
                if (parentGroup && unit.attach.parentNodeName) {
                  console.warn(
                    "[scene-viewer] no bone '%s' on parent '%s' (key %s) for child '%s'; attaching to scene root",
                    unit.attach.parentNodeName,
                    unit.attach.parentLabel,
                    unit.attach.parentEntity,
                    unit.label,
                  );
                }
                attachParent = state.sceneRoot;
                attachTransform = unit.attach.transform;
              }
              // When the exporter resolved this child's transform as a
              // root-attached matrix (no_rotation=true), the matrix
              // already includes the bone's world translation. In that
              // case, even if we found the bone, we should NOT compose
              // with the bone's transform — re-route to scene root.
              if (bone && unit.attach.rootAttachedTransform) {
                attachParent = state.sceneRoot;
              }
            }

            if (attachTransform) {
              group.matrixAutoUpdate = false;
              group.matrix.copy(attachTransform);
            }
            attachParent.add(group);
            instanceCount += 1;
            publishedGroup = group;

            // Frame once we have at least one mesh attached so the user
            // sees something instead of empty space.
            if (!framedOnce) {
              framedOnce = true;
              state.sceneRoot.updateMatrixWorld(true);
              flightCamRef.current?.resetToScene(state.sceneRoot);
            }

            // Sidecar + textures run in parallel with the next mesh.
            if (unit.sidecar) {
              const sidecarKey = unit.sidecar;
              const sidecarPromise = loadSidecar(sidecarKey).then(
                async (sidecar) => {
                  if (gen !== generationRef.current) return;
                  const texPromises = await applySidecarMaterials(group, sidecarKey, sidecar);
                  if (texPromises.length > 0) {
                    textureWork.push(
                      Promise.allSettled(texPromises),
                    );
                  }
                },
              );
              textureWork.push(sidecarPromise);
            }
          } catch (err) {
            console.warn(
              "[scene-viewer] mesh load failed:",
              unit.label,
              unit.meshAsset,
              err,
            );
          } finally {
            // Publish (or null-publish) so anyone awaiting this entity
            // unblocks regardless of whether the load succeeded.
            if (unit.entityName) {
              const def = entityResolvers.get(unit.entityName);
              if (def) {
                // Resolve with the group if we have one; otherwise an
                // empty placeholder so dependents don't await forever.
                def.resolve(publishedGroup ?? new THREE.Group());
              }
            }
            loadedCount += 1;
            setProgress({
              loaded: loadedCount,
              total,
              current: unit.label,
            });
          }
        };

        // Run the unit list with bounded concurrency. Yield every batch
        // so the event loop drains.
        await runWithLimit(units, MAX_CONCURRENT, runUnit, async () => {
          await microyield();
        });

        if (gen !== generationRef.current) return;
        const perfAfterGlb = performance.now();

        // Wait for any remaining sidecar / texture work to finish before
        // we mark the load complete. We don't add new geometry from these,
        // so the user can already pan around the assembled scene.
        await Promise.allSettled(textureWork);

        if (gen !== generationRef.current) return;
        const perfAfterTextures = performance.now();

        // Final camera frame in case streaming added geometry well outside
        // the initial bounds.
        state.sceneRoot.updateMatrixWorld(true);
        if (!framedOnce) {
          flightCamRef.current?.resetToScene(state.sceneRoot);
        }

        // Emit a single summary line with all phase deltas so the bottleneck
        // is visible in app.log without DevTools. Field meanings:
        //   scene_json  = load_decomposed_scene IPC round-trip (Rust JSON read + serialize)
        //   manifest    = TS-side unit-list build from parsed manifest (sync, should be <10ms)
        //   glb         = all GLB IPC reads + Three.js parse (the big parallel phase)
        //   textures    = remaining sidecar/texture IPC reads + createImageBitmap decode
        //   total       = wall time from scene-load useEffect trigger to texture settle
        const dScene    = (perfAfterScene    - perfT0).toFixed(0);
        const dManifest = (perfAfterManifest - perfAfterScene).toFixed(0);
        const dGlb      = (perfAfterGlb      - perfAfterManifest).toFixed(0);
        const dTextures = (perfAfterTextures  - perfAfterGlb).toFixed(0);
        const dTotal    = (perfAfterTextures  - perfT0).toFixed(0);
        console.info(
          `[perf] load ${packageInfo.package_name}: ` +
          `scene_json=${dScene}ms manifest=${dManifest}ms ` +
          `glb=${dGlb}ms textures=${dTextures}ms total=${dTotal}ms ` +
          `units=${units.length}`,
        );

        setLoading(false);
        setProgress(null);
        setStats({
          instances: instanceCount,
          interiors: interiors.length,
          lights: totalLights,
          metrics: getMaterialMetrics(),
        });
        onStatus?.(
          `${packageInfo.package_name}: ${instanceCount} parts, ${interiors.length} interiors, ${totalLights} lights`,
        );

        // Fetch paint variants and surface them to the parent. This is
        // fire-and-forget — paints.json is small (under a few KB) and
        // optional; missing files (older exports, entities with no
        // DataCore paint variants) yield an empty list and the parent
        // hides the dropdown. We fetch AFTER setStats so the viewer
        // is fully usable before the dropdown populates; the user can
        // pan/zoom while the variant list loads.
        try {
          const paints = (await loadDecomposedPaints(
            packageInfo.package_dir,
          )) as PaintsManifest | null;
          if (gen !== generationRef.current) return;
          const variants = paints?.paint_variants ?? [];
          paintVariantsRef.current = variants;
          onPaints?.(variants);
          // Honor a livery selection that was set BEFORE the scene
          // finished loading. The livery effect only runs on prop
          // change after the initial mount, so a parent that defaulted
          // `livery` to a non-null value would otherwise see no swap.
          // Read through the ref so a selection made between mount
          // and paints-fetch lands as well.
          const targetLivery = liveryPropRef.current;
          if (targetLivery && targetLivery !== activeLiveryRef.current) {
            await applyLiveryRebuild(targetLivery);
          }
        } catch (err) {
          // Paints are non-essential — log and move on.
          console.warn("[scene-viewer] paints.json load failed:", err);
        }
      } catch (err) {
        if (gen !== generationRef.current) return;
        const msg = err instanceof Error ? err.message : String(err);
        console.error("[scene-viewer] load failed:", err);
        setError(msg);
        setLoading(false);
        setProgress(null);
        onStatus?.(`Error: ${msg}`);
      }
    })();
    // `livery`, `onPaints`, and `applyLiveryRebuild` are intentionally
    // NOT in the dep array. The load runs ONCE per package; livery is
    // applied via the dedicated effect below (which honors prop changes
    // mid-load via `liveryPropRef`). Including `livery` here would
    // tear down and re-load the entire scene on every dropdown click.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [packageInfo.package_dir, packageInfo.export_root, packageInfo.package_name, onStatus]);

  // -- Ground plane visibility toggle. The mesh lives on `scene` (not
  //    `sceneRoot`), so it survives package switches; flipping
  //    `mesh.visible` is a near-zero-cost render-loop bypass — no
  //    material, geometry, or scene-graph mutation. --
  useEffect(() => {
    const state = stateRef.current;
    if (!state) return;
    state.groundMesh.visible = showGroundPlane;
  }, [showGroundPlane]);

  // -- Grid helper visibility toggle. --
  useEffect(() => {
    const state = stateRef.current;
    if (!state) return;
    state.gridHelper.visible = showGrid;
  }, [showGrid]);

  // -- Ground plane color. Applied live when the RGB sliders change. --
  useEffect(() => {
    const state = stateRef.current;
    if (!state) return;
    const mat = state.groundMesh.material as THREE.MeshStandardMaterial;
    const [r, g, b] = groundPlaneColor;
    mat.color.setRGB(r / 255, g / 255, b / 255);
    mat.needsUpdate = true;
  }, [groundPlaneColor]);

  // -- Diagnostic overrides: traverse scene and apply live slider values. --
  // Runs whenever any diagnostic field changes. Does NOT rebuild materials;
  // it mutates the already-built Three.js material objects in place so the
  // next render frame picks up the new values without any allocation.
  //
  // Material family detection uses the same criteria as the loader:
  //  - HardSurface = MeshStandardMaterial or MeshPhysicalMaterial whose
  //    name doesn't match the hologram/glass/decal sentinel patterns.
  //    We rely on the ground-plane name guard (skipping the ground mesh)
  //    rather than trying to enumerate exact shader families here.
  //  - Glass / hologram / decal materials are skipped for metalness and
  //    clearcoat overrides because their visually-correct values differ
  //    from hull paint defaults.
  //
  // Color saturation is applied after any color already set by the
  // loader (including the positive-control red from the triage-instrument
  // agent), so the two compose correctly.
  useEffect(() => {
    const state = stateRef.current;
    if (!state) return;
    const { renderer, scene } = state;

    // 1. Renderer-level: tone-mapping exposure.
    renderer.toneMappingExposure = diagnostics.toneMappingExposure;

    // 2. Scene lights.
    scene.traverse((obj) => {
      if (obj instanceof THREE.AmbientLight || obj instanceof THREE.HemisphereLight) {
        // Skip the ground-plane's own pseudo-ambient if any; they are
        // identified by name. The fallback_hemi we created at init is the
        // target.
        obj.intensity = diagnostics.ambientIntensity;
      } else if (obj instanceof THREE.DirectionalLight) {
        obj.intensity = diagnostics.directionalIntensity;
      }
    });

    // Camera-attached "headlight" — look for a PointLight or SpotLight
    // that is a direct child of the camera.
    state.camera.children.forEach((child) => {
      if (child instanceof THREE.PointLight || child instanceof THREE.SpotLight) {
        child.intensity = diagnostics.headlightIntensity;
      }
    });

    // 3. Material traversal.
    // Pre-build a scratch Color so we don't allocate one per material.
    const hsl = { h: 0, s: 0, l: 0 };
    scene.traverse((obj) => {
      if (!(obj instanceof THREE.Mesh)) return;
      // Skip the ground plane.
      if (obj.name === "ground_plane") return;

      const mats: THREE.Material[] = Array.isArray(obj.material)
        ? obj.material
        : [obj.material];

      for (const mat of mats) {
        // Determine if this is a glass/hologram/decal material. We use the
        // same name-sentinel the loader uses: glass materials have
        // transmission=1, holograms are ShaderMaterial, decals have name
        // containing "decal". Skip those for metalness/clearcoat/envmap.
        const isGlass =
          mat instanceof THREE.MeshPhysicalMaterial && mat.transmission > 0.5;
        const isHologram = mat instanceof THREE.ShaderMaterial;
        const isDecal =
          typeof mat.name === "string" && mat.name.toLowerCase().includes("decal");
        const isHardSurface = !isGlass && !isHologram && !isDecal;

        if (isHardSurface) {
          if ("envMapIntensity" in mat) {
            (mat as THREE.MeshStandardMaterial).envMapIntensity =
              diagnostics.envMapIntensity;
          }
          if ("metalness" in mat) {
            (mat as THREE.MeshStandardMaterial).metalness = diagnostics.metalness;
          }
          if (diagnostics.roughnessOverrideEnabled && "roughness" in mat) {
            (mat as THREE.MeshStandardMaterial).roughness = diagnostics.roughness;
          }
          if ("clearcoat" in mat && mat instanceof THREE.MeshPhysicalMaterial) {
            (mat as THREE.MeshPhysicalMaterial).clearcoat = diagnostics.clearcoat;
          }
        }

        // Color saturation: applies to every material that has a .color,
        // regardless of family. Composes with whatever color is already set
        // (including positive-control red from the triage-instrument agent).
        if ("color" in mat && (mat as THREE.MeshStandardMaterial).color instanceof THREE.Color) {
          const col = (mat as THREE.MeshStandardMaterial).color;
          // Read the color we last stored in userData.origColor if available;
          // if not, use current color as the baseline.
          let base: THREE.Color;
          if (mat.userData._diagOrigColor instanceof THREE.Color) {
            base = mat.userData._diagOrigColor;
          } else {
            base = col.clone();
            mat.userData._diagOrigColor = base;
          }
          base.getHSL(hsl);
          col.setHSL(hsl.h, Math.min(1, hsl.s * diagnostics.colorSaturation), hsl.l);
        }

        mat.needsUpdate = true;
      }
    });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    diagnostics.envMapIntensity,
    diagnostics.toneMappingExposure,
    diagnostics.metalness,
    diagnostics.roughness,
    diagnostics.roughnessOverrideEnabled,
    diagnostics.clearcoat,
    diagnostics.ambientIntensity,
    diagnostics.directionalIntensity,
    diagnostics.headlightIntensity,
    diagnostics.colorSaturation,
    // Re-apply after scene loads so newly-built materials pick up
    // current slider values without the user having to wiggle a slider.
    loading,
  ]);

  // -- First-frame marker. When the load transitions from true -> false,
  //    schedule one rAF tick and log the wall time from load-start to the
  //    first rendered frame. This tells us how much overhead React state
  //    propagation + the render-loop tick adds on top of the load phase. --
  useEffect(() => {
    if (loading) return; // still loading or initial mount
    const t0 = loadStartRef.current;
    if (t0 === 0) return; // no load has started yet
    const rafId = requestAnimationFrame(() => {
      const dt = (performance.now() - t0).toFixed(0);
      console.info(`[perf] first-frame ${dt}ms (from load-start to rAF)`);
    });
    return () => cancelAnimationFrame(rafId);
  }, [loading]);

  // -- Render-style switch: rebuild materials for every recorded
  // binding without re-loading meshes or textures. Cached materials are
  // shared across instances of the same submaterial so the rebuild
  // cost is one buildMaterial call per *unique* (submaterial, style)
  // pair rather than per mesh. --
  useEffect(() => {
    if (renderStyle === activeStyleRef.current) return;
    if (bindingsRef.current.length === 0) {
      activeStyleRef.current = renderStyle;
      return;
    }
    // Use the same texture cache the load did, so the textured rebuild
    // hits the cache instead of re-decoding every PNG.
    const loadCachedTexture = (path: string): Promise<THREE.Texture | null> => {
      const cached = textureCacheRef.current.get(path);
      // Re-read failures yield null without trying to fetch — the
      // viewer is offline of new IPC during a style switch.
      return cached ?? Promise.resolve(null);
    };
    const newCache = new Map<string, THREE.Material>();
    for (const binding of bindingsRef.current) {
      const cacheKey = materialCacheKey(binding.submaterial, renderStyle);
      let material = newCache.get(cacheKey);
      if (!material) {
        const built = buildMaterial(
          binding.submaterial,
          loadCachedTexture,
          renderStyle,
        );
        material = built.material;
        newCache.set(cacheKey, material);
      }
      binding.mesh.material = material;
    }
    // Dispose old materials that are no longer referenced. Safe because
    // the bindings list above just rewrote every reference.
    for (const old of materialCacheRef.current.values()) {
      old.dispose();
    }
    materialCacheRef.current = newCache;
    activeStyleRef.current = renderStyle;
  }, [renderStyle]);

  // -- Debug-fallback toggle: walk every cached material and swap its
  //    `.color` (or `uniforms.baseColor.value` for the Hologram
  //    ShaderMaterial) to the kind's neon — or restore the stashed
  //    `userData.originalColor`. Mutating in place avoids the
  //    rebuild + 1000× shader-recompile storm that froze the main
  //    thread on big ships. --
  useEffect(() => {
    if (debugFallbacks === activeDebugRef.current) return;
    setDebugFallbackMode(debugFallbacks);
    let touched = 0;
    for (const m of materialCacheRef.current.values()) {
      applyDebugFallbackToMaterial(m);
      touched += 1;
    }
    activeDebugRef.current = debugFallbacks;
    console.info(
      `[debug-fallbacks] mode=${debugFallbacks} swapped colours on ${touched} materials`,
    );
  }, [debugFallbacks]);

  // -- Livery switch: swap exterior bindings to the variant's
  //    substitute sidecar (or restore the default). Skips when the
  //    selection hasn't changed; the load effect handles the
  //    initial-livery apply once paints.json has been fetched. --
  useEffect(() => {
    if (livery === activeLiveryRef.current) return;
    void applyLiveryRebuild(livery);
  }, [livery, applyLiveryRebuild]);

  return (
    <div ref={containerRef} className="relative w-full h-full">
      {loading && (
        <div className="absolute top-2 left-2 px-3 py-1.5 rounded-md bg-bg-alt/90 border border-border z-10">
          <div className="flex items-center gap-2 text-sm text-text-sub">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24" fill="none">
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
              />
            </svg>
            <span>
              Loading {packageInfo.package_name}
              {progress && progress.total > 0
                ? ` - ${progress.loaded}/${progress.total} (${Math.floor(
                    (progress.loaded / progress.total) * 100,
                  )}%)`
                : "..."}
            </span>
          </div>
          {progress && progress.current && (
            <p className="text-[11px] text-text-dim font-mono mt-1 max-w-md truncate">
              {progress.current}
            </p>
          )}
        </div>
      )}
      {error && (
        <div className="absolute top-2 left-2 right-2 px-3 py-2 rounded-md bg-danger/10 border border-danger z-10">
          <p className="text-sm font-medium text-danger">Failed to load package</p>
          <p className="text-xs text-text-dim mt-1 font-mono break-all">{error}</p>
        </div>
      )}
      {/* Combined bottom-right panel: flight-cam HUD readout (top
          section) + scene-load metrics (below the divider). A single
          show/hide toggle (button + H key) collapses the whole panel
          to a tiny re-expand affordance so the canvas reclaims the
          screen real estate. The metrics block is only rendered while
          stats is non-null and there is no load error. */}
      {!error && (hudVisible ? (
        <div
          className="absolute bottom-2 right-2 z-10 max-w-md rounded-md bg-bg-alt/95 border border-border shadow"
        >
          <div className="flex items-start justify-between gap-2 px-3 py-1.5">
            <FlightCamHud handle={flightCam} embedded />
            <button
              type="button"
              onClick={() => setHudVisible(false)}
              title="Hide stats panel (H)"
              aria-label="Hide stats panel"
              className="text-[10px] px-1.5 py-0.5 rounded border border-border bg-bg/50 text-text-sub hover:bg-bg/80 hover:text-text font-mono shrink-0"
            >
              hide
            </button>
          </div>
          {stats && (
            <div className="border-t border-border px-3 py-1.5">
              <div className="flex items-center justify-between gap-2">
                <p className="text-xs text-text-sub font-mono">
                  {stats.instances} parts &middot; {stats.interiors} interiors &middot; {stats.lights} lights
                </p>
                <button
                  type="button"
                  className={`text-[10px] px-2 py-0.5 rounded border font-mono ${
                    debugFallbacks
                      ? "bg-accent/20 border-accent text-text"
                      : "bg-bg/50 border-border text-text-sub hover:bg-bg/80"
                  }`}
                  onClick={() => setDebugFallbacks((v) => !v)}
                  title="Recolour fallback / stand-in materials with diagnostic neons (cyan = heuristic primary, green = no palette, red = unknown family, yellow = hologram, magenta-violet = screen, pink = skin)"
                >
                  {debugFallbacks ? "DBG ON" : "DBG"}
                </button>
              </div>
              <p className="text-[11px] text-text-dim font-mono mt-1">
                mats {stats.metrics.totalBuilt}
                {(() => {
                  const fams = Object.entries(stats.metrics.byFamily)
                    .sort((a, b) => b[1] - a[1])
                    .slice(0, 4)
                    .map(([k, v]) => `${k}=${v}`)
                    .join(" ");
                  return fams ? ` (${fams})` : "";
                })()}
              </p>
              <p className="text-[11px] text-text-dim font-mono">
                tex {stats.metrics.diffuseTextureSuccess}/
                {stats.metrics.diffuseTextureSuccess + stats.metrics.diffuseTextureMiss}
                {" "}&middot; tint {stats.metrics.paletteTintApplied}
                {stats.metrics.paletteHeuristicPrimaryFallback > 0
                  ? ` (+${stats.metrics.paletteHeuristicPrimaryFallback} heur)`
                  : ""}
                {stats.metrics.paletteTintMissing > 0
                  ? ` -${stats.metrics.paletteTintMissing} miss`
                  : ""}
              </p>
              <p className="text-[11px] text-text-dim font-mono">
                cc {stats.metrics.clearCoatFired}
                {" "}&middot; sysB {stats.metrics.systemBFired}
                {" "}&middot; sysA {stats.metrics.systemAFired}
                {" "}&middot; ddna {stats.metrics.ddnaRoughnessHooked}
                {" "}&middot; pom {stats.metrics.pomHooked}
                {stats.metrics.dirtOverlayHooked > 0
                  ? ` &middot; dirt ${stats.metrics.dirtOverlayHooked}`
                  : ""}
              </p>
            </div>
          )}
        </div>
      ) : (
        <button
          type="button"
          onClick={() => setHudVisible(true)}
          title="Show stats panel (H)"
          aria-label="Show stats panel"
          className="absolute bottom-2 right-2 z-10 text-[10px] px-2 py-1 rounded border border-border bg-bg-alt/90 text-text-sub hover:bg-bg-alt hover:text-text font-mono shadow"
        >
          show stats
        </button>
      ))}
      <MaterialInspector
        hits={pickHits}
        selectedHitIndex={selectedHitIndex}
        onSelectHit={setSelectedHitIndex}
        onClose={() => {
          setPickHits([]);
          setSelectedHitIndex(null);
        }}
        packageSlug={packageInfo.package_name}
        liveryState={livery ?? "nolivery"}
      />
    </div>
  );
}

function instanceTransform(instance: SceneInstance): THREE.Matrix4 {
  return instance.local_transform_sc
    ? matrixFromRows(instance.local_transform_sc)
    : matrixFromOffsetEuler(instance.offset_position, instance.offset_rotation);
}

/**
 * Run `worker` over `items` with at most `limit` in flight at a time.
 * After each completed worker the optional `onBatch` callback runs, which
 * we use to yield to the event loop so input events can drain. Resolves
 * once every item has been attempted (worker errors are the worker's
 * problem; this helper does not propagate them).
 */
async function runWithLimit<T>(
  items: T[],
  limit: number,
  worker: (item: T) => Promise<void>,
  onBatch?: () => Promise<void>,
): Promise<void> {
  if (items.length === 0) return;
  let cursor = 0;
  let sinceYield = 0;
  const inflight = new Set<Promise<void>>();
  const launch = (): void => {
    if (cursor >= items.length) return;
    const item = items[cursor++];
    const p = worker(item).finally(() => {
      inflight.delete(p);
    });
    inflight.add(p);
  };

  // Prime up to `limit` workers.
  for (let i = 0; i < Math.min(limit, items.length); i++) launch();

  while (inflight.size > 0) {
    await Promise.race(inflight);
    sinceYield += 1;
    if (sinceYield >= limit && onBatch) {
      sinceYield = 0;
      await onBatch();
    }
    while (inflight.size < limit && cursor < items.length) launch();
  }
}

/**
 * Walk an Object3D subtree looking for a node whose name matches
 * `target` (case-insensitive). Returns the first match in pre-order
 * traversal; this matches the exporter's `node_name_to_idx` policy of
 * keeping the first occurrence on collision.
 *
 * Three.js's built-in `getObjectByName` is case-sensitive, but the
 * exporter writes `parent_node_name` from CryEngine NMC node names
 * which can mix case. Lower-casing both sides is the safe match.
 */
function findNodeByName(
  root: THREE.Object3D,
  target: string,
): THREE.Object3D | null {
  const lower = target.toLowerCase();
  let found: THREE.Object3D | null = null;
  let descendantCount = 0;
  const namedDescendants: string[] = [];
  root.traverse((node) => {
    descendantCount += 1;
    if (node.name) namedDescendants.push(node.name);
    if (found) return;
    if (node.name && node.name.toLowerCase() === lower) {
      found = node;
    }
  });
  if (!found) {
    // Diagnostic: pinpoint why the lookup failed. If the descendant
    // list is empty, the parentGroup never received the GLB hierarchy
    // (clone or strip ate it). If the list is populated but lacks
    // `target`, the scene.json reference and the GLB node names have
    // drifted. Close-match shows partial overlaps for naming-convention
    // bugs (e.g. case, prefix, trailing characters).
    const closeMatches = namedDescendants.filter(
      (n) => n.toLowerCase().includes(lower) || lower.includes(n.toLowerCase()),
    );
    console.warn(
      "[findNodeByName] MISS target='%s' rootName='%s' rootDirectChildren=%d descendantsVisited=%d named=%d closeMatches=%o",
      target,
      root.name || "<unnamed>",
      root.children.length,
      descendantCount,
      namedDescendants.length,
      closeMatches.slice(0, 8),
    );
  }
  return found;
}

/**
 * Disable any mesh-vertex-colour multiplication that GLTFLoader auto-enables
 * when a primitive carries a COLOR_0 attribute. CryEngine writes wear / damage
 * / dirt masks into that channel - they are NOT real colours, and rendering
 * them as such yields magenta and red blotches all over the model.
 *
 * Operates on every Mesh in the subtree:
 *   - `material.vertexColors = false` and `material.needsUpdate = true` so
 *     the cached shader recompiles without the mask multiplication.
 *   - delete `geometry.attributes.color` so the mask data isn't even uploaded
 *     to the GPU.
 *
 * Idempotent. Safe to call on every loaded GLB regardless of whether COLOR_0
 * is actually present.
 */
function stripMaskVertexColors(root: THREE.Object3D): void {
  root.traverse((node) => {
    if (!(node instanceof THREE.Mesh)) return;
    const geom = node.geometry as THREE.BufferGeometry | undefined;
    if (geom?.attributes.color) {
      geom.deleteAttribute("color");
    }
    const mats = Array.isArray(node.material) ? node.material : [node.material];
    for (const m of mats) {
      if (!m) continue;
      // `vertexColors` only exists on the materials we care about. Use a
      // type guard so we don't try to assign on (e.g.) a stray ShaderMaterial.
      if ("vertexColors" in m && (m as THREE.Material & { vertexColors: boolean }).vertexColors) {
        (m as THREE.Material & { vertexColors: boolean }).vertexColors = false;
        m.needsUpdate = true;
      }
    }
  });
}

function clearObject(obj: THREE.Object3D): void {
  // Dispose children's resources before removing them.
  while (obj.children.length > 0) {
    const child = obj.children[0];
    child.traverse((node) => {
      if (node instanceof THREE.Mesh) {
        node.geometry.dispose();
        if (Array.isArray(node.material)) {
          for (const m of node.material) m.dispose();
        } else if (node.material) {
          node.material.dispose();
        }
      }
    });
    obj.remove(child);
  }
}

function guessMime(path: string): string {
  const lower = path.toLowerCase();
  if (lower.endsWith(".png")) return "image/png";
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
  if (lower.endsWith(".webp")) return "image/webp";
  // DDS / KTX would need a special loader. Returning a generic mime keeps the
  // TextureLoader from rejecting it; in practice .dds entries only appear as
  // projector textures and the viewer skips them.
  return "application/octet-stream";
}
