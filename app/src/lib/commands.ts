import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Wrap `invoke` with a `performance.now()` delta. Logs a `[perf]` line to
 * the console (captured by tauri-plugin-log -> app.log) whenever a command
 * takes longer than 50ms. Sub-50ms calls are cheap enough to skip logging.
 */
async function timedInvoke<T>(name: string, args?: unknown): Promise<T> {
  const t0 = performance.now();
  try {
    return await invoke<T>(name, args as Record<string, unknown>);
  } finally {
    const dt = performance.now() - t0;
    if (dt > 50) {
      console.info(`[perf] invoke:${name} ${dt.toFixed(0)}ms`);
    }
  }
}

export interface DiscoverResult {
  path: string;
  source: string;
}

export interface FileDirEntry {
  kind: "file";
  name: string;
  compressed_size: number;
  uncompressed_size: number;
}

export interface DirectoryDirEntry {
  kind: "directory";
  name: string;
}

export type DirEntry = FileDirEntry | DirectoryDirEntry;

export interface LoadProgress {
  fraction: number;
  message: string;
}

export interface SystemPalette {
  scheme: string;
  background: string;
  foreground: string;
  accent: string;
  success: string;
  warning: string;
  danger: string;
}

/** Get the OS system theme (dark/light, accent, palette). */
export async function getSystemTheme(): Promise<SystemPalette> {
  return invoke<SystemPalette>("get_system_theme");
}

/** Listen for OS theme changes. */
export function onSystemThemeChanged(
  callback: (palette: SystemPalette) => void,
): Promise<UnlistenFn> {
  return listen<SystemPalette>("system-theme-changed", (event) => {
    callback(event.payload);
  });
}

/** Discover all Data.p4k installations across channels. */
export async function discoverP4k(): Promise<DiscoverResult[]> {
  return invoke<DiscoverResult[]>("discover_p4k");
}

export interface P4kInfo {
  entry_count: number;
  total_bytes: number;
}

/** Open a P4k file and load it into the backend. */
export async function openP4k(path: string): Promise<P4kInfo> {
  return invoke<P4kInfo>("open_p4k", { path });
}

/** List directory contents from the loaded P4k. */
export async function listDir(path: string): Promise<DirEntry[]> {
  return invoke<DirEntry[]>("list_dir", { path });
}

/** List only subdirectory names under a path (fast). */
export async function listSubdirs(path: string): Promise<string[]> {
  return invoke<string[]>("list_subdirs", { path });
}

/** Open a file picker for Data.p4k. Returns the selected path or null. */
export async function browseP4k(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({
    title: "Select Data.p4k",
    filters: [{ name: "P4K Archive", extensions: ["p4k"] }],
    multiple: false,
    directory: false,
  });
  return result ?? null;
}

/** Listen for progress events during P4k loading. */
export function onLoadProgress(
  callback: (progress: LoadProgress) => void,
): Promise<UnlistenFn> {
  return listen<LoadProgress>("load-progress", (event) => {
    callback(event.payload);
  });
}

// ── Export types ──

export interface EntityDto {
  name: string;
  id: string;
  display_name: string | null;
  is_npc_or_internal: boolean;
}

export interface CategoryDto {
  name: string;
  entities: EntityDto[];
}

export interface ExportRequest {
  record_ids: string[];
  names: string[];
  output_dir: string;
  lod: number;
  mip: number;
  export_kind: string;
  material_mode: string;
  format: string;
  include_attachments: boolean;
  include_interior: boolean;
  include_lights: boolean;
  threads: number;
  overwrite_existing_assets: boolean;
  include_nodraw: boolean;
}

export interface ExportProgress {
  current: number;
  total: number;
  fraction: number;
  entity_name: string;
  entity_id: string;
  stage: string;
  error: string | null;
}

export interface ExportDone {
  success: number;
  errors: number;
  succeeded_ids: string[];
}

// ── Export commands ──

/** Scan DataCore for entity categories. Requires P4k to be loaded. */
export async function scanCategories(): Promise<CategoryDto[]> {
  return invoke<CategoryDto[]>("scan_categories");
}

/** Start batch export. Progress reported via events. */
export async function startExport(request: ExportRequest): Promise<void> {
  return invoke<void>("start_export", { request });
}

/** Cancel an in-progress export. */
export async function cancelExport(): Promise<void> {
  return invoke<void>("cancel_export");
}

/** Listen for export progress events. */
export function onExportProgress(
  callback: (progress: ExportProgress) => void,
): Promise<UnlistenFn> {
  return listen<ExportProgress>("export-progress", (event) => {
    callback(event.payload);
  });
}

/** Listen for export completion. */
export function onExportDone(
  callback: (result: ExportDone) => void,
): Promise<UnlistenFn> {
  return listen<ExportDone>("export-done", (event) => {
    callback(event.payload);
  });
}

/** Open a folder picker for the export output directory. */
export async function browseOutputDir(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({
    title: "Select output directory",
    directory: true,
    multiple: false,
  });
  return result ?? null;
}

// ── DataCore types ──

export interface SearchResultDto {
  name: string;
  struct_type: string;
  path: string;
  id: string;
}

export interface TreeFolderEntry {
  kind: "folder";
  name: string;
}

export interface TreeRecordEntry {
  kind: "record";
  name: string;
  struct_type: string;
  id: string;
}

export type TreeEntryDto = TreeFolderEntry | TreeRecordEntry;

export interface RecordDto {
  name: string;
  struct_type: string;
  path: string;
  id: string;
  json: string;
}

// ── DataCore commands ──

/** Search records by name substring. Returns up to 500 results. */
export async function dcSearch(query: string): Promise<SearchResultDto[]> {
  return invoke<SearchResultDto[]>("dc_search", { query });
}

/** List tree entries (folders + records) at a given path. */
export async function dcListTree(path: string): Promise<TreeEntryDto[]> {
  return invoke<TreeEntryDto[]>("dc_list_tree", { path });
}

/** Get a record's full data for the property inspector. */
export async function dcGetRecord(recordId: string): Promise<RecordDto> {
  return invoke<RecordDto>("dc_get_record", { recordId });
}

/** Export a record as JSON, saving to the given path. */
export async function dcExportJson(recordId: string, outputPath: string): Promise<void> {
  return invoke<void>("dc_export_json", { recordId, outputPath });
}

/** Export a record as XML, saving to the given path. */
export async function dcExportXml(recordId: string, outputPath: string): Promise<void> {
  return invoke<void>("dc_export_xml", { recordId, outputPath });
}

/** Export all records under a folder path. Returns count of exported records. */
export async function dcExportFolder(
  pathPrefix: string,
  format: "json" | "xml",
  outputDir: string,
): Promise<number> {
  return invoke<number>("dc_export_folder", { pathPrefix, format, outputDir });
}

export interface BacklinkDto {
  name: string;
  id: string;
}

/** Get records that reference the given record. */
export async function dcGetBacklinks(recordId: string): Promise<BacklinkDto[]> {
  return invoke<BacklinkDto[]>("dc_get_backlinks", { recordId });
}

// ── Audio types ──

export interface AudioInitResult {
  trigger_count: number;
  bank_count: number;
}

export interface AudioBankResult {
  name: string;
  trigger_count: number;
}

export interface AudioEntityResult {
  name: string;
  record_path: string;
  trigger_count: number;
}

export interface AudioTriggerResult {
  trigger_name: string;
  bank_name: string;
  duration_type: string;
  radius_max: number | null;
}

export interface AudioTriggerDetail {
  trigger_name: string;
  bank_name: string;
  duration_type: string;
  sound_count: number;
}

export interface AudioSoundResult {
  media_id: number;
  source_type: string;
  bank_name: string;
  path_description: string;
}

// ── Audio commands ──

/** Build ATL index from P4k. Called once, cached. */
export async function audioInit(): Promise<AudioInitResult> {
  return invoke<AudioInitResult>("audio_init");
}

/** Search DataCore for entities with audio triggers matching query. */
export async function audioSearchEntities(query: string): Promise<AudioEntityResult[]> {
  return invoke<AudioEntityResult[]>("audio_search_entities", { query });
}

/** Search ATL index by trigger name substring. */
export async function audioSearchTriggers(query: string): Promise<AudioTriggerResult[]> {
  return invoke<AudioTriggerResult[]>("audio_search_triggers", { query });
}

/** List all soundbanks with trigger counts. */
export async function audioListBanks(): Promise<AudioBankResult[]> {
  return invoke<AudioBankResult[]>("audio_list_banks");
}

/** Get all triggers for a specific bank. */
export async function audioBankTriggers(bankName: string): Promise<AudioTriggerDetail[]> {
  return invoke<AudioTriggerDetail[]>("audio_bank_triggers", { bankName });
}

/** List all media in a bank by scanning HIRC directly (bypasses event resolution). */
export async function audioBankMedia(bankName: string): Promise<AudioSoundResult[]> {
  return invoke<AudioSoundResult[]>("audio_bank_media", { bankName });
}

/** Get all triggers for a specific entity, with resolved sound counts. */
export async function audioEntityTriggers(entityName: string): Promise<AudioTriggerDetail[]> {
  return invoke<AudioTriggerDetail[]>("audio_entity_triggers", { entityName });
}

/** Resolve a trigger to its leaf sounds via ATL -> bank -> HIRC. */
export async function audioResolveTrigger(triggerName: string): Promise<AudioSoundResult[]> {
  return invoke<AudioSoundResult[]>("audio_resolve_trigger", { triggerName });
}

/** Decode a WEM to Ogg bytes for browser playback. */
export async function audioDecodeWem(
  mediaId: number,
  sourceType: string,
  bankName: string,
): Promise<number[]> {
  return invoke<number[]>("audio_decode_wem", { mediaId, sourceType, bankName });
}

export interface FolderExtractProgress {
  current: number;
  total: number;
  name: string;
}

/** Listen for folder extract progress events. */
export function onFolderExtractProgress(
  callback: (progress: FolderExtractProgress) => void,
): Promise<UnlistenFn> {
  return listen<FolderExtractProgress>("folder-extract-progress", (event) => {
    callback(event.payload);
  });
}

/** Extract files under a P4k folder path to disk. Optional filter by extension (e.g. "mtl,xml"). */
export async function extractP4kFolder(
  pathPrefix: string,
  outputDir: string,
  filter?: string,
): Promise<number> {
  return invoke<number>("extract_p4k_folder", { pathPrefix, outputDir, filter: filter ?? null });
}

// ── Raw file access ──

/** Read a raw file from the P4K. */
export async function readP4kFile(path: string): Promise<ArrayBuffer> {
  const bytes = await invoke<number[]>("read_p4k_file", { path });
  return new Uint8Array(bytes).buffer;
}

// ── Geometry preview ──

/** Generate a GLB preview for a geometry file. Returns raw GLB bytes. */
export async function previewGeometry(path: string): Promise<ArrayBuffer> {
  const bytes = await invoke<number[]>("preview_geometry", { path });
  return new Uint8Array(bytes).buffer;
}

// ── XML preview ──

/** Decode a CryXMLB file and return formatted XML text. */
export async function previewXml(path: string): Promise<string> {
  return invoke<string>("preview_xml", { path });
}

// ── DDS preview ──

export interface DdsPreviewResult {
  png: number[];
  width: number;
  height: number;
  mip_level: number;
  mip_count: number;
}

/** Decode a DDS texture and return PNG bytes + metadata. */
export async function previewDds(
  path: string,
  mip?: number,
): Promise<DdsPreviewResult> {
  return invoke<DdsPreviewResult>("preview_dds", { path, mip: mip ?? null });
}

/** Save a DDS texture from P4K as a PNG file to disk. */
export async function exportDdsPng(
  path: string,
  outputPath: string,
  mip?: number,
): Promise<void> {
  return invoke<void>("export_dds_png", { path, outputPath, mip: mip ?? null });
}

/** Extract a single file from P4K to disk. */
export async function extractP4kFile(
  path: string,
  outputPath: string,
): Promise<void> {
  return invoke<void>("extract_p4k_file", { path, outputPath });
}

// ── Decomposed export browsing ──
//
// The decomposed export contract is documented in
// `docs/decomposed-export-contract.md`. The Rust exporter emits dynamic
// JSON, not strongly-typed structs, so wrappers below return `unknown`
// for manifests and `ArrayBuffer` for binary payloads. The scene-viewer
// loader knows the shape and reads the fields it needs.

export interface DecomposedPackageInfo {
  package_dir: string;
  export_root: string;
  package_name: string;
  has_scene_manifest: boolean;
}

/**
 * List decomposed packages under a root. Accepts an export root, a
 * `Packages/` directory, or a single package directory (returns just that
 * one).
 */
export async function listDecomposedPackages(
  root: string,
): Promise<DecomposedPackageInfo[]> {
  return invoke<DecomposedPackageInfo[]>("list_decomposed_packages", { root });
}

/** Load `scene.json` from a decomposed package directory. */
export async function loadDecomposedScene(
  packagePath: string,
): Promise<unknown> {
  return timedInvoke<unknown>("load_decomposed_scene", { packagePath });
}

/**
 * Load `palettes.json` from a decomposed package directory. Returns
 * `null` when the file is absent (palette-less exports).
 */
export async function loadDecomposedPalettes(
  packagePath: string,
): Promise<unknown> {
  return invoke<unknown>("load_decomposed_palettes", { packagePath });
}

/** Load `liveries.json` from a decomposed package directory, or null. */
export async function loadDecomposedLiveries(
  packagePath: string,
): Promise<unknown> {
  return invoke<unknown>("load_decomposed_liveries", { packagePath });
}

/**
 * Load `paints.json` from a decomposed package directory. Returns null
 * when the file is absent (older exports, or entities with no paint
 * variants in DataCore).
 *
 * No dedicated Rust command exists for `paints.json`; we lean on the
 * generic `load_decomposed_json` and treat any read error as "absent".
 * This keeps the change UI-only — adding a Tauri command would force a
 * cargo rebuild for what is effectively a one-line file read.
 */
export async function loadDecomposedPaints(
  packagePath: string,
): Promise<unknown> {
  // Forward slashes work on every OS the Tauri command tolerates.
  const path = `${packagePath.replace(/[\\/]+$/, "")}/paints.json`;
  try {
    return await loadDecomposedJson(path);
  } catch {
    return null;
  }
}

/**
 * Load any JSON file under the export root. Used for material sidecars
 * (`*.materials.json`) referenced from `scene.json`.
 */
export async function loadDecomposedJson(path: string): Promise<unknown> {
  return timedInvoke<unknown>("load_decomposed_json", { path });
}

/**
 * Read a binary file under the export root (GLB / PNG / DDS). Returns
 * an `ArrayBuffer` so callers can hand it directly to GLTFLoader,
 * THREE.TextureLoader, or `URL.createObjectURL`.
 */
export async function readDecomposedFile(path: string): Promise<ArrayBuffer> {
  const bytes = await timedInvoke<number[]>("read_decomposed_file", { path });
  return new Uint8Array(bytes).buffer;
}

/** Open a directory picker for an export root. Returns null if cancelled. */
export async function browseDecomposedRoot(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({
    title: "Select decomposed export package",
    directory: true,
    multiple: false,
  });
  return result ?? null;
}

// ── Scene Viewer self-service exporter ──
//
// Mirror of `SceneExportOpts` in `decomposed_commands.rs`. Every field
// that affects on-disk output is part of the cache key, so changing any
// of these forces a re-export.

export interface SceneExportOpts {
  lod: number;
  mip: number;
  /** "none" | "colors" | "textures" | "all" */
  material_mode: string;
  include_attachments: boolean;
  include_interior: boolean;
  include_lights: boolean;
  include_nodraw: boolean;
}

/** Defaults used when the user hasn't tweaked anything. Match Rust side. */
export const DEFAULT_SCENE_EXPORT_OPTS: SceneExportOpts = {
  lod: 1,
  mip: 2,
  material_mode: "textures",
  include_attachments: true,
  include_interior: true,
  include_lights: true,
  include_nodraw: false,
};

export interface SceneEntityDto {
  entity_name: string;
  display_name: string | null;
  category: string;
  cached: boolean;
}

export interface SceneCachePath {
  export_root: string;
  /** Null when no scene.json has been written yet. */
  package_dir: string | null;
  cached: boolean;
}

export interface SceneExportProgress {
  fraction: number;
  stage: string;
  entity_name: string;
}

export interface SceneExportDone {
  entity_name: string;
  package_dir: string | null;
  error: string | null;
}

/** List ships/vehicles available for the Scene Viewer. */
export async function listSceneEntities(
  opts?: SceneExportOpts,
): Promise<SceneEntityDto[]> {
  return invoke<SceneEntityDto[]>("list_scene_entities", {
    opts: opts ?? null,
  });
}

/** Resolve the cache slot for an (entity, opts) pair. */
export async function getSceneCachePath(
  entityName: string,
  opts: SceneExportOpts,
): Promise<SceneCachePath> {
  return invoke<SceneCachePath>("get_scene_cache_path", {
    entityName,
    opts,
  });
}

/** Delete the on-disk cache slot for an (entity, opts) pair. */
export async function clearSceneCache(
  entityName: string,
  opts: SceneExportOpts,
): Promise<void> {
  return invoke<void>("clear_scene_cache", { entityName, opts });
}

/**
 * Decomposed-export contract version this frontend was built against.
 * Mirror of `starbreaker_3d::DECOMPOSED_CONTRACT_VERSION`. Bump in
 * lockstep with the Rust constant. Used as the argument to
 * `pruneStaleCache` so the prune pass keeps slots stamped with the
 * current version and discards every other slot.
 */
export const DECOMPOSED_CONTRACT_VERSION = 2;

export interface ClearAllResult {
  entries_removed: number;
  bytes_freed: number;
}

/**
 * Wipe every entry under the per-user `decomposed_cache/` directory.
 * Defensively only deletes immediate children of the cache root; never
 * follows symlinks out, never accepts arbitrary paths. Returns the
 * number of entries removed and bytes freed.
 */
export async function clearAllSceneCache(): Promise<ClearAllResult> {
  return invoke<ClearAllResult>("clear_all_scene_cache");
}

export interface CacheStats {
  entry_count: number;
  total_bytes: number;
  /** Absolute path to the cache root on disk. */
  cache_root: string;
}

/** Walk `decomposed_cache/` and return entry count + total size. */
export async function cacheStats(): Promise<CacheStats> {
  return invoke<CacheStats>("cache_stats");
}

export interface PruneStaleResult {
  entries_removed: number;
  bytes_freed: number;
}

/**
 * Remove cache slots whose contract-version segment does not match
 * `currentVersion`. Safe to call on app start from `useEffect`. Slots
 * stamped with the current version are preserved; everything else
 * (legacy unversioned slots, slots from older contracts) is deleted.
 */
export async function pruneStaleCache(
  currentVersion: number,
): Promise<PruneStaleResult> {
  return invoke<PruneStaleResult>("prune_stale_cache", { currentVersion });
}

/**
 * Run the decomposed exporter for a single entity in-process. Progress
 * is emitted on `scene-export-progress`; completion (success or error)
 * is emitted on `scene-export-done`.
 */
export async function startSceneExport(
  entityName: string,
  opts: SceneExportOpts,
): Promise<void> {
  return timedInvoke<void>("start_scene_export", { entityName, opts });
}

/** Cancel the in-flight scene export, if any. */
export async function cancelSceneExport(): Promise<void> {
  return timedInvoke<void>("cancel_scene_export");
}

/** Listen for scene export progress events. */
export function onSceneExportProgress(
  callback: (progress: SceneExportProgress) => void,
): Promise<UnlistenFn> {
  return listen<SceneExportProgress>("scene-export-progress", (event) => {
    callback(event.payload);
  });
}

/** Listen for scene export completion events. */
export function onSceneExportDone(
  callback: (result: SceneExportDone) => void,
): Promise<UnlistenFn> {
  return listen<SceneExportDone>("scene-export-done", (event) => {
    callback(event.payload);
  });
}

// ── Diagnostic file capture ──

