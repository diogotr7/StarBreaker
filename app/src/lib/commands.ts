import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

/** Discover all Data.p4k installations across channels. */
export async function discoverP4k(): Promise<DiscoverResult[]> {
  return invoke<DiscoverResult[]>("discover_p4k");
}

/** Open a P4k file and load it into the backend. Returns entry count. */
export async function openP4k(path: string): Promise<number> {
  return invoke<number>("open_p4k", { path });
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
  include_textures: boolean;
  include_interior: boolean;
  include_normals: boolean;
  include_lights: boolean;
  include_tangents: boolean;
  include_materials: boolean;
  experimental_textures: boolean;
}

export interface ExportProgress {
  current: number;
  total: number;
  entity_name: string;
  error: string | null;
}

export interface ExportDone {
  success: number;
  errors: number;
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
