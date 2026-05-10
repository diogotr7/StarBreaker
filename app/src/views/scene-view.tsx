// Self-service Scene Viewer.
//
// The user picks a ship/vehicle from a searchable list. If a cached
// export exists for the current options, it loads instantly. Otherwise
// the in-process exporter runs, emits progress, and the resulting
// package is mounted in the viewer.
//
// Rendering itself is unchanged — `SceneViewer` consumes a
// `DecomposedPackageInfo` (package_dir + export_root + package_name).
// We synthesize one from the cache hit / export-done payload.
//
// The "Open Decomposed Package..." button is kept as an escape hatch
// for power users with externally-produced packages.

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Box,
  FolderOpen,
  RotateCw,
  CheckCircle2,
  Search,
  Trash2,
  X,
} from "lucide-react";
import {
  browseDecomposedRoot,
  cacheStats,
  cancelSceneExport,
  clearAllSceneCache,
  clearSceneCache,
  DECOMPOSED_CONTRACT_VERSION,
  DEFAULT_SCENE_EXPORT_OPTS,
  getSceneCachePath,
  listAllSocpaks,
  listDecomposedPackages,
  listSceneEntities,
  loadSceneToGltf,
  onSceneExportDone,
  onSceneExportProgress,
  onSceneLoadProgress,
  pruneStaleCache,
  startSceneExport,
  type CacheStats,
  type DecomposedPackageInfo,
  type LoadSceneResponse,
  type SceneEntityDto,
  type SceneExportOpts,
  type SocpakDirEntry,
} from "../lib/commands";
import { SocpakTree } from "../components/socpak-tree";
import { SceneViewer } from "../components/scene-viewer";
import { SocSceneViewer } from "../components/soc-scene-viewer";
import { ProgressOverlay } from "../components/progress-overlay";
import {
  DEFAULT_VIEWER_SETTINGS,
  type ViewerSettings,
} from "../components/settings-panel";
import { ViewerToolbar } from "../components/viewer-toolbar";
import type { FlightCamHandle } from "../lib/flight-camera";
import { type PaintVariant } from "../lib/decomposed-loader";
import { type RenderStyle } from "../lib/render-styles";
import { useProgressReporter } from "../lib/progress-reporter";

/**
 * Format a byte count as a short human string. Falls back to bytes for
 * sub-KB values; uses decimal (1000-based) units to match what most file
 * managers display alongside disk free space.
 */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let idx = 0;
  while (value >= 1000 && idx < units.length - 1) {
    value /= 1000;
    idx += 1;
  }
  const digits = idx === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[idx]}`;
}

/** Derive a short slug from a socpak path for screenshot filenames.
 *  Strips the directory prefix and the `.socpak` extension; falls back
 *  to "soc_scene" when the input is null. */
function socpakSlug(socpakPath: string | null): string {
  if (!socpakPath) return "soc_scene";
  const normalised = socpakPath.replace(/\\/g, "/");
  const tail = normalised.split("/").pop() ?? socpakPath;
  return tail.replace(/\.socpak$/i, "");
}

/** Synthesize a DecomposedPackageInfo from a package_dir path. */
function packageInfoFromDir(packageDir: string): DecomposedPackageInfo {
  // Pull out the trailing folder name as the package name and walk back
  // up two levels (`Packages/<name>` → root) for the export root. The
  // SceneViewer component already tolerates either separator.
  const norm = packageDir.replace(/\\/g, "/");
  const segments = norm.split("/").filter((s) => s.length > 0);
  const packageName = segments[segments.length - 1] ?? packageDir;
  // Drop the last two segments: `<package_name>` and `Packages`.
  const exportRoot = segments.slice(0, -2).join("/");
  return {
    package_dir: norm,
    export_root: exportRoot.length > 0 ? exportRoot : norm,
    package_name: packageName,
    has_scene_manifest: true,
  };
}

/**
 * Root prefix for the Maps tab tree. Socpaks live almost exclusively
 * under `Data\ObjectContainers\` -- starting elsewhere would surface
 * directories the composer cannot load. Backslash-terminated to
 * match the format the lazy listing returns.
 */
const MAPS_ROOT_PREFIX = "Data\\ObjectContainers\\";

/**
 * Hardcoded fallback used only when the lazy root listing errors
 * (P4k not loaded, command rejected, IPC failure). Keeps the Maps
 * tab clickable instead of leaving the user staring at a permanent
 * error state. The Executive Hangar path is the reference scene we
 * know loads end-to-end.
 */
const MAP_FALLBACK_PATH =
  "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\asteroid_base\\ext\\ab_final_set\\ab_pyro_final_set_dungeon_executive-001.socpak";

/**
 * Translate a SOC scene-load progress event into a [0,1] fraction
 * for the backend phase of the unified progress overlay. Phase weights
 * inside the backend step are tuned to observed Exec Hangar timings on
 * a cold cache: compose ~5%, resolve ~85%, emit ~8%, cache_write ~2%.
 * Within each sub-phase the fraction is `current/total` linearly.
 */
function computeSocBackendFraction(
  phase: string,
  current: number,
  total: number,
): number {
  const within = total > 0 ? Math.min(1, Math.max(0, current / total)) : 0;
  switch (phase) {
    case "compose":
      return 0.0 + within * 0.05;
    case "resolve":
      return 0.05 + within * 0.85;
    case "emit":
      return 0.9 + within * 0.08;
    case "cache_write":
      return 0.98 + within * 0.02;
    default:
      return within;
  }
}

export function SceneView() {
  // Source list: the union of "discovered DataCore entities" and any
  // externally-mounted packages from the escape-hatch picker.
  const [entities, setEntities] = useState<SceneEntityDto[]>([]);
  const [entitiesLoading, setEntitiesLoading] = useState(false);
  const [entitiesError, setEntitiesError] = useState<string | null>(null);

  const [search, setSearch] = useState("");
  const [activeCategory, setActiveCategory] = useState<string>("Ships");

  const [active, setActive] = useState<DecomposedPackageInfo | null>(null);
  const [activeEntityName, setActiveEntityName] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  // Unified progress reporter. Both the Ships export pipeline and the
  // SOC scene-load pipeline drive this same reporter; the
  // <ProgressOverlay> renders one bar over many phases. The reporter
  // owns the phase array; handlers update phases by id. See
  // `lib/progress-reporter.ts`.
  //
  // CRITICAL: do NOT depend on the whole `progress` object in
  // `useCallback` deps below -- destructure the four mutating
  // callbacks (which are stable across renders) and depend on those
  // individually. The reporter's identity changes whenever its phases
  // state updates, and listing it in deps cascades through every
  // callback that consumes it -- which then re-creates downstream
  // props on every progress tick. The previous shape of this code
  // re-fired SocSceneViewer's load effect on every tick, spawning
  // overlapping GLTF fetches and saturating Tauri's IPC channel until
  // the renderer locked up.
  const progress = useProgressReporter();
  const {
    begin: progressBegin,
    update: progressUpdate,
    advance: progressAdvance,
    reset: progressReset,
  } = progress;
  // Track which entity / socpak the active progress run belongs to so
  // late events from prior runs don't leak into the current overlay.
  const activeRunRef = useRef<{ kind: "ships" | "soc"; key: string } | null>(
    null,
  );
  const [status, setStatus] = useState<string>("");
  // Presentation mode for materials. Switching is in-place; the scene
  // is not re-loaded.
  const [renderStyle, setRenderStyle] = useState<RenderStyle>("textured");
  // Per-ship paint variants discovered from the loaded package's
  // `paints.json`. The dropdown hides itself when this list is empty
  // (entities with no DataCore paint variants, or older exports
  // pre-paints.json). Keyed by `palette_id` to align with the
  // exporter's stable identifier; null means "default (as-baked)".
  const [paintVariants, setPaintVariants] = useState<PaintVariant[]>([]);
  const [livery, setLivery] = useState<string | null>(null);

  // Flight-camera handle the SceneViewer publishes upward so we can
  // host the projection-mode picker in the top-right toolbar instead
  // of having it free-float inside the canvas (where it collided with
  // the Livery + Style + Settings strip).
  const [flightCamHandle, setFlightCamHandle] =
    useState<FlightCamHandle | null>(null);

  // SOC scene state (Maps tab). The Maps tab loads a top-level
  // socpak through `loadSceneToGltf`, which composes many zones,
  // resolves meshes/materials, and writes a single GLB to the scene
  // cache. The `socResponse` carries the cache path + AABB the
  // SocSceneViewer needs to render and frame. Load progress flows
  // through the unified `progress` reporter above (3 phases:
  // backend / fetch / decode).
  const [socResponse, setSocResponse] = useState<LoadSceneResponse | null>(null);
  const [socActivePath, setSocActivePath] = useState<string | null>(null);
  const [socError, setSocError] = useState<string | null>(null);
  // Flight-camera handle published by the SocSceneViewer. Hosted in
  // the Maps tab toolbar via `<ProjectionModePicker>`.
  const [socFlightCamHandle, setSocFlightCamHandle] =
    useState<FlightCamHandle | null>(null);

  // Maps tab now uses the lazy `SocpakTree` component, which manages
  // its own per-branch state. We only track tab-level mount lifecycle
  // here.
  const [mapsMounted, setMapsMounted] = useState(false);
  // Global socpak path index. Populated lazily on the first Maps tab
  // activation via `listAllSocpaks` and held for the rest of the
  // session. The lazy tree's branch-by-branch filter cannot find a
  // path the user has not yet expanded; this index drives the "search
  // everywhere" mode in `SocpakTree` so a typed query matches across
  // the entire archive. Null while loading; the tree falls back to
  // local-branch filtering until the index lands.
  const [socpakIndex, setSocpakIndex] = useState<string[] | null>(null);
  const [socpakIndexError, setSocpakIndexError] = useState<string | null>(null);

  // User-tunable viewer settings (collapsible panel in the bottom-right
  // of the viewer pane). New settings extend `ViewerSettings`; the panel
  // renders rows from the state shape so adding one is local.
  const [viewerSettings, setViewerSettings] = useState<ViewerSettings>(
    DEFAULT_VIEWER_SETTINGS,
  );
  const updateSettings = useCallback((patch: Partial<ViewerSettings>) => {
    setViewerSettings((prev) => ({ ...prev, ...patch }));
  }, []);

  // H key handler. Both viewer surfaces forward to this so the binding
  // stays in lockstep with the Settings panel's "Show pivot orb"
  // toggle.
  const togglePivotOrb = useCallback(() => {
    setViewerSettings((prev) => ({ ...prev, showPivotOrb: !prev.showPivotOrb }));
  }, []);

  // Fast Preview toggle. When enabled, the exporter skips interior
  // socpak containers — full Polaris drops from minutes to seconds at
  // the cost of an empty hull. The cache key already encodes
  // `include_interior` (`_i0` vs `_i1`), so fast and full live in
  // separate cache slots. Default OFF so first-time users get the full
  // ship; flipping the toggle re-runs `listSceneEntities` so the
  // "cached" badges reflect the chosen slot.
  const [fastPreview, setFastPreview] = useState(false);
  // Show WIP toggle. When enabled, listSceneEntities also returns
  // CIG-internal / in-development entities (Mauler, Idris, Javelin,
  // Starlifter A2, plus 400+ NPC variants and derelicts). The list
  // grows from ~80 ships to ~500. WIP entries get an [WIP] badge in
  // the row. Default OFF -- most users want the player-flyable
  // roster only. Toggling re-runs listSceneEntities; does not
  // affect the export cache key (these flags only filter the LIST,
  // not the export bytes).
  const [showWip, setShowWip] = useState(false);

  const opts: SceneExportOpts = useMemo(
    () => ({
      ...DEFAULT_SCENE_EXPORT_OPTS,
      include_interior: !fastPreview,
      include_wip: showWip,
    }),
    [fastPreview, showWip],
  );

  // Track in-flight export so progress events from older runs don't
  // overwrite newer state.
  const generationRef = useRef(0);

  // Cache status surfaced in the toolbar. `null` while the first stats
  // call is in flight or after a failure; the UI hides the chip in that
  // case rather than showing "0 entries", which would be misleading.
  const [cache, setCache] = useState<CacheStats | null>(null);
  // Two-click confirmation state for "Clear all". When non-null, the
  // button label switches to "Click again to confirm" until the timeout
  // fires or the user clicks again. Less obtrusive than window.confirm
  // and consistent with how the per-row re-export button behaves.
  const [confirmClearAll, setConfirmClearAll] = useState(false);
  const confirmClearTimer = useRef<number | null>(null);

  const refreshCacheStats = useCallback(async () => {
    try {
      const stats = await cacheStats();
      setCache(stats);
    } catch (err) {
      // Don't surface as a hard error — the cache chip is informational.
      console.warn("cache_stats failed:", err);
      setCache(null);
    }
  }, []);

  const refreshEntities = useCallback(async () => {
    setEntitiesLoading(true);
    setEntitiesError(null);
    try {
      const list = await listSceneEntities(opts);
      setEntities(list);
    } catch (err) {
      setEntitiesError(err instanceof Error ? err.message : String(err));
    } finally {
      setEntitiesLoading(false);
    }
  }, [opts]);

  // On first mount only: prune cache slots stamped with prior contract
  // versions, then read stats so the toolbar reflects what's left. The
  // prune is opt-in housekeeping (safe to skip; orphaned slots just
  // waste disk until the user clicks "Clear all"), but doing it once
  // per launch keeps things tidy after a contract bump.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await pruneStaleCache(DECOMPOSED_CONTRACT_VERSION);
      } catch (err) {
        // Non-fatal: a locked file or absent cache root just means
        // there was nothing to prune.
        console.warn("prune_stale_cache failed:", err);
      }
      if (!cancelled) {
        await refreshCacheStats();
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Fetch the entity list on mount.
  useEffect(() => {
    refreshEntities();
  }, [refreshEntities]);

  // Push the pivot-orb visibility setting onto whichever flight cam
  // is currently mounted. Two effects (one per surface) so the call
  // does not fire against a stale handle when the user switches tabs.
  // The orb is hidden by default at construction in the flight-cam
  // hook itself, so this is a noop when the setting is at its
  // default until the user opts in.
  useEffect(() => {
    flightCamHandle?.setPivotOrbVisible(viewerSettings.showPivotOrb);
  }, [flightCamHandle, viewerSettings.showPivotOrb]);
  useEffect(() => {
    socFlightCamHandle?.setPivotOrbVisible(viewerSettings.showPivotOrb);
  }, [socFlightCamHandle, viewerSettings.showPivotOrb]);

  // Reset livery and paint-variant list whenever the active package
  // changes. The SceneViewer publishes the new ship's paints via
  // `onPaints` once its scene loads; the dropdown stays empty in the
  // meantime so a stale variant from a previous ship can't be
  // accidentally re-applied.
  useEffect(() => {
    setLivery(null);
    setPaintVariants([]);
  }, [active?.package_dir]);

  // Subscribe to scene export events for the lifetime of the view.
  // Progress and completion drive the unified reporter so the export
  // shares the same overlay as SOC loads.
  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    onSceneExportProgress((p) => {
      if (cancelled) return;
      const run = activeRunRef.current;
      if (!run || run.kind !== "ships" || run.key !== p.entity_name) return;
      progressUpdate("export", { fraction: p.fraction, detail: p.stage });
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    });

    onSceneExportDone((r) => {
      if (cancelled) return;
      const run = activeRunRef.current;
      if (!run || run.kind !== "ships" || run.key !== r.entity_name) return;

      if (r.error) {
        progressReset();
        activeRunRef.current = null;
        setActionError(`Export failed: ${r.error}`);
        return;
      }
      if (r.package_dir) {
        progressAdvance("export");
        progressReset();
        activeRunRef.current = null;
        // Refresh the entity list so the cached badge updates, and
        // bump the toolbar stats since a new slot just landed.
        refreshEntities();
        refreshCacheStats();
        setActive(packageInfoFromDir(r.package_dir));
        setActiveEntityName(r.entity_name);
      }
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    });

    return () => {
      cancelled = true;
      for (const fn of unlisteners) fn();
    };
  }, [refreshEntities, refreshCacheStats, progressUpdate, progressAdvance, progressReset]);

  // Mount the SocpakTree only after the Maps tab is first opened. The
  // tree fires its own root listing on mount, so we want that single
  // backend call to fire once -- on entry to the tab -- not repeatedly
  // as React re-renders the parent. Once mounted, switching away and
  // back keeps the tree (and its loaded-branch cache) intact.
  useEffect(() => {
    if (activeCategory === "Maps" && !mapsMounted) {
      setMapsMounted(true);
    }
  }, [activeCategory, mapsMounted]);

  // Kick off the global socpak path index in parallel with the lazy
  // tree's first paint. `listAllSocpaks` is cached on disk by p4k
  // identity so the second-launch path is a JSON read; the cold path
  // is a few hundred ms of linear scan. We deliberately do NOT block
  // the tree on this -- the user can browse / click leaves while the
  // index loads. Once it lands, the tree's search input switches to
  // global mode automatically.
  useEffect(() => {
    if (activeCategory !== "Maps") return;
    if (socpakIndex !== null) return;
    if (socpakIndexError !== null) return;
    let cancelled = false;
    (async () => {
      try {
        const paths = await listAllSocpaks();
        if (cancelled) return;
        setSocpakIndex(paths);
      } catch (err) {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        setSocpakIndexError(msg);
        // Do not surface as a blocking error -- the lazy tree still
        // works without the index. Log so a missing index is visible
        // in the dev tools.
        console.warn("list_all_socpaks_cmd failed:", msg);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [activeCategory, socpakIndex, socpakIndexError]);

  // Subscribe to SOC scene-load progress events. Translates the
  // backend's `(phase, current, total, message)` into the unified
  // reporter's "backend" phase. The fetch + decode phases are owned
  // by the SocSceneViewer and update the reporter directly via the
  // callbacks the parent passes down.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    onSceneLoadProgress((p) => {
      if (cancelled) return;
      const run = activeRunRef.current;
      if (!run || run.kind !== "soc") return;
      const fraction = computeSocBackendFraction(p.phase, p.current, p.total);
      progressUpdate("backend", { fraction, detail: p.message || p.phase });
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [progressUpdate]);

  // Tear down the pending two-click confirmation timer when the view
  // unmounts so the timeout doesn't fire against a stale setState.
  useEffect(() => {
    return () => {
      if (confirmClearTimer.current !== null) {
        window.clearTimeout(confirmClearTimer.current);
        confirmClearTimer.current = null;
      }
    };
  }, []);

  const handleClearAll = useCallback(async () => {
    // First click arms the confirmation; second click within 2s does
    // the wipe. The button label flips between the two states. We
    // intentionally avoid `window.confirm` here — the chrome dialog is
    // jarring for an action this narrow.
    if (!confirmClearAll) {
      setConfirmClearAll(true);
      if (confirmClearTimer.current !== null) {
        window.clearTimeout(confirmClearTimer.current);
      }
      confirmClearTimer.current = window.setTimeout(() => {
        setConfirmClearAll(false);
        confirmClearTimer.current = null;
      }, 2000);
      return;
    }

    if (confirmClearTimer.current !== null) {
      window.clearTimeout(confirmClearTimer.current);
      confirmClearTimer.current = null;
    }
    setConfirmClearAll(false);

    try {
      const result = await clearAllSceneCache();
      // Drop any in-memory references to caches that no longer exist.
      // The active scene viewer keeps its already-loaded geometry —
      // clearing the cache only affects future loads — so we don't
      // close it. The entity list re-runs so badges flip off.
      await refreshEntities();
      await refreshCacheStats();
      setStatus(
        `Cleared ${result.entries_removed} cache entr` +
          (result.entries_removed === 1 ? "y" : "ies"),
      );
    } catch (err) {
      setActionError(
        `Clear cache failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }, [confirmClearAll, refreshEntities, refreshCacheStats]);

  const categories = useMemo(() => {
    const set = new Set<string>();
    for (const e of entities) set.add(e.category);
    // Stable order: Ships first, then alphabetical.
    return Array.from(set).sort((a, b) => {
      if (a === b) return 0;
      if (a === "Ships") return -1;
      if (b === "Ships") return 1;
      return a.localeCompare(b);
    });
  }, [entities]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    return entities.filter((e) => {
      if (e.category !== activeCategory) return false;
      if (q.length === 0) return true;
      return (
        e.entity_name.toLowerCase().includes(q) ||
        (e.display_name?.toLowerCase().includes(q) ?? false)
      );
    });
  }, [entities, activeCategory, search]);

  const launchEntity = useCallback(
    async (entity: SceneEntityDto, forceReexport = false) => {
      setActionError(null);
      const gen = ++generationRef.current;

      try {
        if (forceReexport) {
          await clearSceneCache(entity.entity_name, opts);
          // Per-row re-export removed a slot; reflect that in the
          // toolbar chip immediately. The new slot will be re-added by
          // the export-done handler.
          refreshCacheStats();
        }

        // Cache check first — instant load when present.
        if (!forceReexport) {
          const slot = await getSceneCachePath(entity.entity_name, opts);
          if (slot.cached && slot.package_dir) {
            if (gen !== generationRef.current) return;
            setActive(packageInfoFromDir(slot.package_dir));
            setActiveEntityName(entity.entity_name);
            return;
          }
        }

        // Cache miss → kick off the in-process export.
        if (gen !== generationRef.current) return;
        setActive(null);
        setActiveEntityName(entity.entity_name);
        activeRunRef.current = { kind: "ships", key: entity.entity_name };
        progressBegin([
          {
            id: "export",
            label: `Exporting ${entity.entity_name}`,
            weight: 1,
            fraction: 0,
            detail: "Starting export...",
          },
        ]);
        await startSceneExport(entity.entity_name, opts);
      } catch (err) {
        if (gen !== generationRef.current) return;
        progressReset();
        activeRunRef.current = null;
        setActionError(err instanceof Error ? err.message : String(err));
      }
    },
    [opts, refreshCacheStats, progressBegin, progressReset],
  );

  const handleCancel = useCallback(async () => {
    try {
      await cancelSceneExport();
    } catch (err) {
      console.error("Cancel failed:", err);
    }
    progressReset();
    activeRunRef.current = null;
    generationRef.current++;
  }, [progressReset]);

  const launchMap = useCallback(
    async (socpakPath: string) => {
      setSocError(null);
      setActionError(null);
      setSocActivePath(socpakPath);
      // CRITICAL: do NOT clear socResponse here. The previous version
      // unmounted SocSceneViewer between loads, which threw away the
      // decode work for the prior scene and forced a full re-walk on
      // every click. More importantly, dropping it via setState here
      // and then setting it again 100ms later (cache-hit path) caused
      // SocSceneViewer to mount, run its load effect, then unmount,
      // then re-mount -- the load effect's clearChildren() during the
      // brief mount window left the renderer in a half-built state
      // that downstream renders couldn't recover from on big maps.
      // Setting socResponse to the new value below replaces the prop
      // in-place, which triggers the load effect once, cleanly.
      activeRunRef.current = { kind: "soc", key: socpakPath };
      // Three phases for a SOC load. Weights are based on observed
      // wall-clock on Exec Hangar (cold cache): backend ~10s, fetch
      // ~1-2s on local disk, decode ~50s and dominates -- so the
      // aggregate bar moves at a steady visible pace through the
      // whole load instead of jumping to ~95% the moment the backend
      // returns and then sitting there for nearly a minute.
      progressBegin([
        {
          id: "backend",
          label: "Composing scene",
          weight: 30,
          fraction: 0,
          detail: "Reading socpak...",
        },
        {
          id: "fetch",
          label: "Fetching GLB",
          weight: 10,
          fraction: null,
        },
        {
          id: "decode",
          label: "Resolving textures",
          weight: 60,
          fraction: null,
        },
      ]);
      try {
        const response = await loadSceneToGltf(socpakPath);
        // If the user navigated away mid-load, drop the result.
        setSocActivePath((current) => {
          if (current !== socpakPath) return current;
          // Backend phase is done; SocSceneViewer will drive fetch +
          // decode through the reporter directly.
          progressAdvance("backend");
          setSocResponse(response);
          console.info(
            `[soc-load] cache_hit=${response.cache_hit} ` +
              `meshes=${response.mesh_count} placements=${response.placement_count} ` +
              `lights=${response.light_count} materials=${response.materials_resolved}/` +
              `${response.materials_resolved + response.materials_default} ` +
              `glb_bytes=${response.glb_bytes} ` +
              `dropped=${response.dropped_placements} failed_meshes=${response.failed_mesh_paths}`,
          );
          return current;
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setSocError(msg);
        progressReset();
        activeRunRef.current = null;
      }
    },
    [progressBegin, progressAdvance, progressReset],
  );

  /** Bridge `SocpakTree`'s leaf-click event to the existing load flow. */
  const handleTreeLeafClick = useCallback(
    (entry: SocpakDirEntry) => {
      void launchMap(entry.path);
    },
    [launchMap],
  );

  /** Click handler for the fallback Executive Hangar entry. */
  const handleFallbackClick = useCallback(() => {
    void launchMap(MAP_FALLBACK_PATH);
  }, [launchMap]);

  const handleSocCancel = useCallback(() => {
    // No backend cancel hook for `load_scene_to_gltf` yet -- the load
    // runs as a single `spawn_blocking` task that owns its work
    // through completion. Clearing local state stops the UI from
    // reacting to in-flight events; the backend keeps building so
    // the user does not have to repeat the wait on the next click.
    setSocActivePath(null);
    progressReset();
    activeRunRef.current = null;
  }, [progressReset]);

  /** Reporter callback handed to `<SocSceneViewer>` so it can update
   *  the fetch + decode phases of the unified progress overlay. We
   *  pass the reporter's `update` directly; the child is trusted to
   *  pass the right phase id ("fetch" or "decode"). */
  const updateSocPhase = useCallback(
    (
      id: "fetch" | "decode",
      patch: Partial<{ fraction: number | null; detail: string }>,
    ) => {
      // Guard: if the user has navigated away mid-load the reporter
      // has been reset; this update would otherwise re-introduce a
      // phantom phase. Keying on activeRunRef rather than reporter
      // state since reporter.phases isn't read here for stability.
      if (!activeRunRef.current || activeRunRef.current.kind !== "soc") return;
      progressUpdate(id, patch);
    },
    [progressUpdate],
  );

  /** Called by `<SocSceneViewer>` when it has finished loading the
   *  scene (decode complete, materials applied, framing reset). Closes
   *  the overlay. */
  const handleSocLoadComplete = useCallback(() => {
    if (!activeRunRef.current || activeRunRef.current.kind !== "soc") return;
    progressReset();
    activeRunRef.current = null;
  }, [progressReset]);

  // Escape hatch: pick an external decomposed root.
  const handleOpenExternal = async () => {
    setActionError(null);
    try {
      const dir = await browseDecomposedRoot();
      if (!dir) return;
      const found = await listDecomposedPackages(dir);
      if (found.length === 0) {
        setActionError(
          "No decomposed packages found. Pick a folder containing 'Packages/' or a single package directory.",
        );
        return;
      }
      const pkg = found.find((p) => p.has_scene_manifest) ?? found[0];
      setActive(pkg);
      setActiveEntityName(null);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="flex flex-col w-full h-full">
      {/* ── Toolbar ── */}
      <div
        className="flex items-center gap-3 px-4 border-b border-border shrink-0"
        style={{ height: "var(--toolbar-height)" }}
      >
        <Box size={16} strokeWidth={1.75} className="text-text-sub shrink-0" />
        <h2 className="text-sm font-medium text-text shrink-0">Scene Viewer</h2>
        {status && (
          <span className="text-xs text-text-dim font-mono truncate flex-1 min-w-0">
            {status}
          </span>
        )}
        <label
          className={`
            ${status ? "" : "ml-auto"} flex items-center gap-2 px-2.5 py-1.5 rounded-md
            text-xs cursor-pointer transition-colors select-none
            ${
              fastPreview
                ? "bg-primary/15 text-text"
                : "bg-surface hover:bg-surface-hi text-text-sub hover:text-text"
            }
          `}
          title="Skip interior socpak containers. Polaris drops from minutes to seconds. Cached separately from the full export."
        >
          <input
            type="checkbox"
            checked={fastPreview}
            onChange={(e) => setFastPreview(e.target.checked)}
            className="accent-accent cursor-pointer"
          />
          Fast preview (skip interiors)
        </label>
        <label
          className={`
            flex items-center gap-2 px-2.5 py-1.5 rounded-md
            text-xs cursor-pointer transition-colors select-none
            ${
              showWip
                ? "bg-primary/15 text-text"
                : "bg-surface hover:bg-surface-hi text-text-sub hover:text-text"
            }
          `}
          title="Also list CIG-internal / in-development entities (Mauler, Idris-M, Javelin, Starlifter A2, plus NPC variants and derelicts). The list grows from ~80 to ~500 entries; WIP entries get a [WIP] badge."
        >
          <input
            type="checkbox"
            checked={showWip}
            onChange={(e) => setShowWip(e.target.checked)}
            className="accent-accent cursor-pointer"
          />
          Show WIP
        </label>
        {cache && (
          <span
            className="flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-surface text-text-sub text-xs select-none"
            title={`Cache root: ${cache.cache_root}`}
          >
            <span className="text-text-faint">Cache</span>
            <span className="tabular-nums">
              {cache.entry_count} {cache.entry_count === 1 ? "entry" : "entries"}
            </span>
            <span className="text-text-faint">
              ({formatBytes(cache.total_bytes)})
            </span>
          </span>
        )}
        <button
          onClick={handleClearAll}
          disabled={cache !== null && cache.entry_count === 0}
          className={`
            flex items-center gap-2 px-3 py-1.5 rounded-md text-xs transition-colors
            disabled:opacity-40 disabled:cursor-not-allowed
            ${
              confirmClearAll
                ? "bg-danger/15 text-danger hover:bg-danger/25"
                : "bg-surface hover:bg-surface-hi text-text-sub hover:text-text"
            }
          `}
          title={
            confirmClearAll
              ? "Click again within 2s to wipe every cached scene"
              : "Wipe every cached scene under decomposed_cache/"
          }
        >
          <Trash2 size={14} strokeWidth={1.75} />
          {confirmClearAll ? "Click again to confirm" : "Clear all"}
        </button>
        <button
          onClick={handleOpenExternal}
          className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-surface hover:bg-surface-hi text-text-sub hover:text-text text-xs transition-colors"
          title="Open an externally-produced decomposed package"
        >
          <FolderOpen size={14} strokeWidth={1.75} />
          Open Package...
        </button>
      </div>

      <div className="flex-1 flex overflow-hidden min-h-0">
        {/* ── Left rail: scene picker ── */}
        <div className="w-[300px] shrink-0 border-r border-border bg-bg-alt flex flex-col min-h-0">
          {/* Search */}
          <div className="p-3 border-b border-border flex items-center gap-2">
            <Search size={14} className="text-text-faint shrink-0" />
            <input
              type="text"
              placeholder="Search ships..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="flex-1 bg-transparent outline-none text-sm text-text placeholder:text-text-faint"
            />
            {search && (
              <button
                onClick={() => setSearch("")}
                className="text-text-faint hover:text-text"
                title="Clear"
              >
                <X size={14} />
              </button>
            )}
          </div>

          {/* Category tabs */}
          <div className="flex gap-1 px-3 py-2 border-b border-border flex-wrap">
            {categories.map((cat) => {
              const count = entities.filter((e) => e.category === cat).length;
              return (
                <button
                  key={cat}
                  onClick={() => setActiveCategory(cat)}
                  className={`
                    px-2.5 py-1 rounded-md text-xs font-medium transition-colors cursor-pointer
                    ${
                      cat === activeCategory
                        ? "bg-primary/15 text-text"
                        : "text-text-dim hover:bg-surface/60 hover:text-text"
                    }
                  `}
                >
                  {cat}
                  <span className="ml-1.5 opacity-60">{count}</span>
                </button>
              );
            })}
            {/* Maps tab -- placeholder for future multi-zone scene rendering. */}
            <button
              onClick={() => setActiveCategory("Maps")}
              className={`
                px-2.5 py-1 rounded-md text-xs font-medium transition-colors cursor-pointer
                ${
                  activeCategory === "Maps"
                    ? "bg-primary/15 text-text"
                    : "text-text-dim hover:bg-surface/60 hover:text-text"
                }
              `}
            >
              Maps
            </button>
          </div>

          {/* Entity list (or Maps tree). The Maps tab uses a separate
              flex column so SocpakTree can own its own scroll region;
              the entity list keeps the legacy single-scroll layout. */}
          {activeCategory === "Maps" ? (
            <div className="flex-1 flex flex-col min-h-0">
              <div className="px-3 py-2 border-b border-border/60 shrink-0">
                <p className="text-[11px] text-text-dim leading-relaxed">
                  Browse the p4k object-container tree. Click a folder to
                  expand it; click a `.socpak` to load the scene. Cold
                  loads take ~10 seconds; cached loads are instant.
                </p>
              </div>
              {mapsMounted && (
                <SocpakTree
                  rootPrefix={MAPS_ROOT_PREFIX}
                  onLeafClick={handleTreeLeafClick}
                  activePath={socActivePath}
                  globalIndex={socpakIndex}
                />
              )}
              {/* Always-visible escape hatch to the reference Exec Hangar
                  scene, in case the tree errors or the user wants the
                  one-click path that worked in the prior catalog UI. */}
              <div className="px-3 py-2 border-t border-border/60 shrink-0">
                <button
                  onClick={handleFallbackClick}
                  className="w-full text-left text-[10px] text-text-faint hover:text-text-sub transition-colors"
                  title={MAP_FALLBACK_PATH}
                >
                  Open Executive Hangar (reference scene)
                </button>
              </div>
            </div>
          ) : (
          <div className="flex-1 overflow-y-auto">
            {entitiesLoading && (
              <div className="px-4 py-6 text-xs text-text-dim">
                Scanning DataCore...
              </div>
            )}
            {entitiesError && (
              <div className="px-4 py-3 text-xs text-danger break-words">
                {entitiesError}
              </div>
            )}
            {!entitiesLoading && !entitiesError && filtered.length === 0 && (
              <div className="px-4 py-6 text-xs text-text-dim">
                No matches.
              </div>
            )}
            {filtered.map((entity) => {
              const isActive = activeEntityName === entity.entity_name;
              const isBusy =
                activeRunRef.current?.kind === "ships" &&
                activeRunRef.current.key === entity.entity_name &&
                progress.active;
              return (
                <div
                  key={entity.entity_name}
                  className={`
                    group flex items-center gap-2 px-3 py-2 text-xs cursor-pointer
                    transition-colors border-l-2
                    ${
                      isActive
                        ? "bg-primary/10 text-text border-l-accent"
                        : "text-text-sub hover:bg-surface/40 border-l-transparent"
                    }
                  `}
                  onClick={() => {
                    if (!isBusy) launchEntity(entity);
                  }}
                  title={entity.entity_name}
                >
                  <div className="flex-1 min-w-0">
                    <div className="truncate flex items-center gap-1.5">
                      <span className="truncate">
                        {entity.display_name ?? entity.entity_name}
                      </span>
                      {entity.is_wip && (
                        <span
                          className="shrink-0 text-[9px] font-mono uppercase tracking-wider px-1 py-0.5 rounded bg-warning/20 text-warning border border-warning/30"
                          title="inclusionMode != ReadyToInclude in DataCore. WIP capital ship, NPC variant, derelict, or mission objective. Export may produce a partial mesh or placeholder textures."
                        >
                          WIP
                        </span>
                      )}
                    </div>
                    {entity.display_name && (
                      <div className="truncate text-[10px] text-text-faint">
                        {entity.entity_name}
                      </div>
                    )}
                  </div>
                  {entity.cached && (
                    <CheckCircle2
                      size={14}
                      className="text-success shrink-0"
                      strokeWidth={1.75}
                      aria-label="Cached"
                    />
                  )}
                  {entity.cached && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        launchEntity(entity, true);
                      }}
                      className="opacity-0 group-hover:opacity-100 text-text-faint hover:text-text transition-opacity shrink-0"
                      title="Re-export (clear cache)"
                    >
                      <RotateCw size={12} strokeWidth={1.75} />
                    </button>
                  )}
                </div>
              );
            })}
          </div>
          )}
        </div>

        {/* ── Right pane: viewer / progress ── */}
        <div className="flex-1 relative overflow-hidden min-w-0">
          {/* Unified progress overlay -- driven by the reporter, used by
              both Ships exports and SOC scene loads. Stays up across
              backend / fetch / decode phases so the user has continuous
              feedback during a multi-stage load. */}
          {progress.active && (
            <ProgressOverlay
              title={
                activeRunRef.current?.kind === "ships"
                  ? `Exporting ${activeRunRef.current.key}`
                  : "Loading scene"
              }
              phases={progress.phases}
              onCancel={
                activeRunRef.current?.kind === "ships"
                  ? handleCancel
                  : handleSocCancel
              }
              cancelTitle={
                activeRunRef.current?.kind === "ships"
                  ? undefined
                  : "Stop tracking the load. The backend keeps building so the next click reuses cached output."
              }
            />
          )}

          {/* SOC scene-package mode: dedicated viewer for the Maps tab.
              Renders a self-contained GLB emitted by `loadSceneToGltf`
              instead of a decomposed-export package. */}
          {activeCategory === "Maps" && (
            <>
              {socError && !progress.active && (
                <div className="absolute top-3 right-3 z-10 max-w-md bg-danger/15 border border-danger/30 text-danger text-xs px-3 py-2 rounded-md font-mono break-words shadow">
                  {socError}
                </div>
              )}
              {!socResponse && !progress.active && (
                <div className="absolute inset-0 flex items-center justify-center">
                  <div className="max-w-md text-center px-6">
                    <Box
                      size={48}
                      strokeWidth={1.5}
                      className="mx-auto text-text-dim mb-3 opacity-40"
                    />
                    <h3 className="text-base font-medium text-text mb-2">
                      Pick a scene
                    </h3>
                    <p className="text-sm text-text-sub">
                      Choose a map from the list. Cold loads take a few
                      seconds; cached loads are instant.
                    </p>
                  </div>
                </div>
              )}
              {socResponse && (
                <>
                  <ViewerToolbar
                    flightCamHandle={socFlightCamHandle}
                    renderStyle={renderStyle}
                    onRenderStyleChange={setRenderStyle}
                    settings={viewerSettings}
                    onSettingsChange={updateSettings}
                    kind="soc"
                  />
                  <SocSceneViewer
                    response={socResponse}
                    onStatus={setStatus}
                    onFlightCamReady={setSocFlightCamHandle}
                    renderStyle={renderStyle}
                    onPhaseProgress={updateSocPhase}
                    onLoadComplete={handleSocLoadComplete}
                    onTogglePivotOrb={togglePivotOrb}
                    screenshotSlug={socpakSlug(socActivePath)}
                  />
                </>
              )}
            </>
          )}

          {/* Decomposed-export mode: existing scene-viewer pipeline. */}
          {activeCategory !== "Maps" && actionError && !progress.active && (
            <div className="absolute top-3 right-3 z-10 max-w-md bg-danger/15 border border-danger/30 text-danger text-xs px-3 py-2 rounded-md font-mono break-words shadow">
              {actionError}
            </div>
          )}

          {activeCategory !== "Maps" && !active && !progress.active && (
            <div className="absolute inset-0 flex items-center justify-center">
              <div className="max-w-md text-center px-6">
                <Box
                  size={48}
                  strokeWidth={1.5}
                  className="mx-auto text-text-dim mb-3 opacity-40"
                />
                <h3 className="text-base font-medium text-text mb-2">
                  Pick a scene
                </h3>
                <p className="text-sm text-text-sub">
                  Choose a ship from the list. Cached scenes load
                  instantly; new ones export in the background.
                </p>
              </div>
            </div>
          )}

          {activeCategory !== "Maps" && active && (
            <>
              <ViewerToolbar
                flightCamHandle={flightCamHandle}
                renderStyle={renderStyle}
                onRenderStyleChange={setRenderStyle}
                settings={viewerSettings}
                onSettingsChange={updateSettings}
                kind="ships"
                leadingSlot={
                  paintVariants.length > 0 ? (
                    <label className="flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-bg-alt/90 border border-border text-xs text-text-sub shadow">
                      <span className="text-text-faint">Livery</span>
                      <select
                        value={livery ?? ""}
                        // Blur after selection so arrow keys go back
                        // to orbiting the camera instead of cycling
                        // the dropdown options.
                        onChange={(e) => {
                          setLivery(
                            e.target.value === "" ? null : e.target.value,
                          );
                          e.currentTarget.blur();
                        }}
                        className="bg-transparent outline-none text-text cursor-pointer"
                      >
                        <option value="" className="bg-bg-alt text-text">
                          Default
                        </option>
                        {paintVariants.map((v) => (
                          <option
                            key={v.palette_id}
                            value={v.palette_id}
                            className="bg-bg-alt text-text"
                          >
                            {v.display_name ??
                              v.subgeometry_tag ??
                              v.palette_id}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : null
                }
              />
              <SceneViewer
                packageInfo={active}
                renderStyle={renderStyle}
                showGroundPlane={viewerSettings.showGroundPlane}
                showGrid={viewerSettings.showGrid}
                groundPlaneColor={viewerSettings.groundPlaneColor}
                diagnostics={viewerSettings.diagnostics}
                livery={livery}
                onPaints={setPaintVariants}
                onStatus={setStatus}
                onFlightCamReady={setFlightCamHandle}
                onTogglePivotOrb={togglePivotOrb}
              />
            </>
          )}
        </div>
      </div>
    </div>
  );
}
