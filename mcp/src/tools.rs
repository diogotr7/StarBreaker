use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_router,
};
use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::StructId;
use starbreaker_p4k::{MappedP4k, P4kArchive};
use starbreaker_ui::CanvasFetcher;

/// Lazily-loaded game data. Initialized on first tool call.
pub(crate) struct GameData {
    p4k_path: PathBuf,
    p4k: Arc<MappedP4k>,
    dcb_bytes: &'static [u8],
    db: Arc<Database<'static>>,
}

const MAX_DATACORE_QUERY_RESULTS: usize = 32;
const MAX_DATACORE_QUERY_JSON_NODES: usize = 50_000;
const MAX_DATACORE_QUERY_JSON_DEPTH: usize = 16;
const MAX_DATACORE_QUERY_ARRAY_ITEMS: usize = 256;
const MAX_DATACORE_QUERY_OBJECT_FIELDS: usize = 256;
const MAX_DATACORE_QUERY_STRING_CHARS: usize = 2048;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kSetDataPathRequest {
    #[schemars(description = "Absolute path to the Data.p4k file to use for subsequent StarBreaker MCP queries.")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchEntitiesRequest {
    #[schemars(description = "Case-insensitive name substring to search for")]
    pub query: String,
    #[schemars(description = "Maximum number of results (default 20)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityLoadoutRequest {
    #[schemars(description = "Entity name (substring match, uses shortest match)")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DatacoreRecordRequest {
    #[schemars(description = "Record GUID or name substring")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DatacoreQueryRequest {
    #[schemars(description = "Record GUID or name substring")]
    pub id: String,
    #[schemars(description = "DataCore property path (e.g. 'Components[VehicleComponentParams].vehicleDefinition')")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kReadRequest {
    #[schemars(description = "File path within P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kListRequest {
    #[schemars(description = "Directory path within P4k (e.g. 'Data\\Objects\\Spaceships'). Empty string for root.")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchRecordsRequest {
    #[schemars(description = "Case-insensitive name substring to search for")]
    pub query: String,
    #[schemars(description = "Optional struct type filter (e.g. 'EntityClassDefinition', 'TintPalette')")]
    pub struct_type: Option<String>,
    #[schemars(description = "Maximum number of results (default 20)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiRegressionRegistryRequest {
    #[schemars(description = "Optional manifest path. Defaults to crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json")]
    pub manifest_path: Option<String>,
    #[schemars(description = "Optional freeze lock path. Defaults to crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json")]
    pub freeze_path: Option<String>,
    #[schemars(description = "Optional case-insensitive substring filter applied to target ids")]
    pub target_filter: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiRegressionValidateRequest {
    #[schemars(description = "Validation mode: quick or full (default quick)")]
    pub mode: Option<String>,
    #[schemars(description = "Optional manifest path. Defaults to crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json")]
    pub manifest_path: Option<String>,
    #[schemars(description = "Optional freeze lock path. Defaults to crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json")]
    pub freeze_path: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiCanvasStyleInventoryRequest {
    #[schemars(description = "Canvas identifier to inspect: absolute JSON path, record GUID, full record name, or bare record name.")]
    pub canvas: String,
    #[schemars(description = "Optional local root containing decompiled BuildingBlocks JSON records. Defaults to ../ships/dcb_canvas/libs/foundry/records from the workspace root.")]
    #[allow(dead_code)]
    pub canvas_root: Option<String>,
    #[schemars(description = "Optional case-insensitive substring filter applied to style entry names.")]
    pub entry_filter: Option<String>,
    #[schemars(description = "If true, include compact condition summaries for each returned style entry. Default true.")]
    pub include_conditions: Option<bool>,
    #[schemars(description = "If true, include compact modifier summaries for each returned style entry. Default true.")]
    pub include_modifiers: Option<bool>,
    #[schemars(description = "Maximum number of style entries to return. Default 200.")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiSceneStyleProbeRequest {
    #[schemars(description = "Canvas identifier to resolve: absolute JSON path, record GUID, full record name, or bare record name.")]
    pub canvas: String,
    #[schemars(description = "Case-insensitive substring matched against resolved node name, type, or text fields.")]
    pub query: String,
    #[schemars(description = "Optional local root containing decompiled BuildingBlocks JSON records. Defaults to ../ships/dcb_canvas/libs/foundry/records from the workspace root.")]
    #[allow(dead_code)]
    pub canvas_root: Option<String>,
    #[schemars(description = "Optional manufacturer id used for brand style selection, e.g. drak, rsi, aegis.")]
    pub manufacturer: Option<String>,
    #[schemars(description = "Maximum number of matching nodes to return. Default 80.")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiIrQueryRequest {
    #[schemars(description = "Canvas GUID/name for the outer binding canvas.")]
    pub canvas_guid: String,
    #[schemars(description = "Optional content canvas GUID/name. Defaults to canvas_guid.")]
    pub content_guid: Option<String>,
    #[schemars(description = "Case-insensitive substring matched against IR node name or type.")]
    pub query: String,
    #[schemars(description = "Optional local root containing decompiled BuildingBlocks JSON records. Defaults to ../ships/dcb_canvas/libs/foundry/records from the workspace root.")]
    #[allow(dead_code)]
    pub canvas_root: Option<String>,
    #[schemars(description = "Optional local style record root. Defaults to <canvas_root>/ui/buildingblocks/styles.")]
    #[allow(dead_code)]
    pub style_root: Option<String>,
    #[schemars(description = "Manufacturer id used for style selection. Default drak.")]
    pub manufacturer: Option<String>,
    #[schemars(description = "Binding helper name included in the UI pipeline input. Default mcp-ui-query.")]
    pub helper: Option<String>,
    #[schemars(description = "Target render width. Default 1920.")]
    pub width: Option<u32>,
    #[schemars(description = "Target render height. Default 1080.")]
    pub height: Option<u32>,
    #[schemars(description = "Maximum number of matching IR nodes to return. Default 80.")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiTagLookupRequest {
    #[schemars(description = "Tag UUID (full or prefix) or case-insensitive tag-name substring to resolve against the live tag database (e.g. 'icon-element' or 'f530a994').")]
    pub query: String,
    #[schemars(description = "Maximum number of matches to return. Default 40.")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiKitSheetEntriesRequest {
    #[schemars(description = "BuildingBlocks_Style record to inspect, by name substring (e.g. sk_uilo_a_buttonprimarystyles, s_uilo_a) or GUID — including modular-kit component sheets.")]
    pub record: String,
    #[schemars(description = "Optional case-insensitive substring filter on entry names (e.g. Filled).")]
    pub entry_filter: Option<String>,
    #[schemars(description = "Maximum entries to return. Default 100.")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ImagePreviewRequest {
    #[schemars(description = "File path within P4k (DDS, PNG, JPG, etc.). For DDS, .tif extension is auto-converted to .dds")]
    pub path: String,
    #[schemars(description = "Mip level for DDS textures (0=full res, default 0)")]
    pub mip: Option<u32>,
    #[schemars(description = "Cubemap face index (0-5) for cubemap DDS. Omit for 2D textures. (Not yet implemented)")]
    #[allow(dead_code)]
    pub face: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChunkListRequest {
    #[schemars(description = "File path within P4k (.cga, .cgf, .cgam, .cgfm, .skin, .skinm, .chr, .soc)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChunkReadRequest {
    #[schemars(description = "File path within P4k")]
    pub path: String,
    #[schemars(description = "Chunk index (from chunk_list output). If omitted, returns all chunks.")]
    pub index: Option<u32>,
    #[schemars(description = "Maximum bytes to show per chunk in hex dump (default 256)")]
    pub max_bytes: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kSearchRequest {
    #[schemars(description = "Case-insensitive substring to search for in P4k file paths")]
    pub query: String,
    #[schemars(description = "Maximum number of results (default 50)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SocpakInspectRequest {
    #[schemars(description = "Path to a .socpak file in P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SocpakReadEntryRequest {
    #[schemars(description = "Path to a .socpak file in P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
    #[schemars(description = "Inner entry path within the .socpak ZIP (case-insensitive). Use the names returned by socpak_inspect.")]
    pub entry: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MtlSummaryRequest {
    #[schemars(description = "Path to .mtl file in P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DbaDumpRequest {
    #[schemars(description = "Path to a .dba or .caf file in P4k (case-insensitive, Data\\ prefix optional). May also be an absolute filesystem path.")]
    pub path: String,
    #[schemars(description = "Optional path to a rig source used to resolve bone hashes to names. Accepts either a .chr / .skin / .skinm skeleton (CompiledBones chunk) OR a .cga / .cgam scene-graph file (NMC chunk — used by ships like the Scorpius whose main body has no CHR). Same path resolution as 'path'. Required for bone_filter to work.")]
    pub skeleton: Option<String>,
    #[schemars(description = "Filter CLIPS by case-insensitive substring match on the clip metadata name (e.g. 'wings_deploy'). Independent of bone_filter.")]
    pub filter: Option<String>,
    #[schemars(description = "Filter CHANNELS by case-insensitive substring match on the resolved bone name (e.g. 'Wing_Mechanism'). Requires `skeleton` to be set; channels with unresolved hashes are excluded when this is active.")]
    pub bone_filter: Option<String>,
    #[schemars(description = "If true, include every keyframe per channel; otherwise only first/last samples are included. Default false.")]
    pub all_keyframes: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MannequinDumpRequest {
    #[schemars(description = "Entity name (substring match, uses shortest match) used to resolve the entity's SAnimationControllerParams.AnimationDatabase and AnimationController paths.")]
    pub entity: String,
    #[schemars(description = "Optional case-insensitive substring filter against fragment group name, GUID, or animation name.")]
    pub filter: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnimationBindingReportRequest {
    #[schemars(description = "Local filesystem folder of exported animation JSON sidecars (e.g. '<package>/animations'), scanned recursively for *.json.")]
    pub animation_folder: Option<String>,
    #[schemars(description = "Individual local animation JSON sidecar files to include, in addition to (or instead of) animation_folder.")]
    pub animation_json: Option<Vec<String>>,
    #[schemars(description = "Source .cga rig to resolve track hashes to node names. P4k path (Data\\ prefix optional) or absolute filesystem path. This is the highest-value source for resolving door/component tracks.")]
    pub cga: Option<String>,
    #[schemars(description = "Source .cgam rig file. P4k or filesystem path.")]
    pub cgam: Option<String>,
    #[schemars(description = "Source .meshsetup file (Joint names). P4k or filesystem path; CryXMLB is auto-decoded.")]
    pub meshsetup: Option<String>,
    #[schemars(description = "Source .chrparams file (animation references). P4k or filesystem path; CryXMLB is auto-decoded.")]
    pub chrparams: Option<String>,
    #[schemars(description = "Converted .glb file (exported node names). Filesystem path or P4k path.")]
    pub glb: Option<String>,
    #[schemars(description = "If true, return the human-readable Markdown summary instead of the full JSON report. Default false.")]
    pub markdown: Option<bool>,
}

/// Decode CryXMLB bytes to XML text; fall back to lossy UTF-8 for plain-text
/// sources (some `.meshsetup`/`.chrparams` are already textual).
fn decode_text_or_cryxml(data: &[u8]) -> String {
    if let Ok(xml) = starbreaker_cryxml::from_bytes(data) {
        return format!("{xml}");
    }
    String::from_utf8_lossy(data).into_owned()
}

/// Recursively collect `*.json` files under `dir` into `files`.
fn collect_json_files_recursive(
    dir: &std::path::Path,
    files: &mut Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_json_files_recursive(&path, files)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendSdnaRequest {
    #[schemars(description = "Absolute filesystem path to a .blend file (can be zstd-compressed Blender 5.x or uncompressed).")]
    pub path: String,
    #[schemars(description = "Optional struct name to look up (e.g. 'Object', 'Mesh', 'Light'). If omitted returns all struct names.")]
    pub struct_name: Option<String>,
    #[schemars(description = "Maximum recursion depth for nested struct fields (default 1, 0 = top-level only).")]
    pub max_depth: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendBlockInspectRequest {
    #[schemars(description = "Absolute filesystem path to a .blend file.")]
    pub path: String,
    #[schemars(description = "SDNA struct type name to filter blocks by (e.g. 'Object', 'Lamp'). If omitted, returns an overview of all block types.")]
    pub sdna_type: Option<String>,
    #[schemars(description = "Maximum bytes of raw hex to show per block (default 256, 0 = no hex dump).")]
    pub max_bytes: Option<u32>,
    #[schemars(description = "Maximum number of blocks to show (default 10).")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendPythonDiffRequest {
    #[schemars(description = "Absolute filesystem path to the .blend file to modify.")]
    pub blend_path: String,
    #[schemars(description = "Python script to run BEFORE saving (sets up a baseline). Pass empty string to skip baseline creation.")]
    pub before_script: String,
    #[schemars(description = "Python script to run AFTER loading the baseline (applies the change under test). Required.")]
    pub after_script: String,
    #[schemars(description = "Optional SDNA struct name to restrict the diff to blocks of that type (e.g. 'NodeTexImage'). If omitted, all changed blocks are shown.")]
    pub sdna_filter: Option<String>,
    #[schemars(description = "Override path to the Blender binary. Falls back to BLENDER_BIN env var, then PATH.")]
    pub blender_bin: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DatacoreStructDefRequest {
    #[schemars(description = "DataCore struct type name (e.g. 'BuildingBlocks_Canvas', 'EntityClassDefinition'). Case-sensitive exact match.")]
    pub name: String,
    #[schemars(description = "If true, include attributes inherited from parent structs. Default true.")]
    pub include_inherited: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DatacoreEnumDefRequest {
    #[schemars(description = "DataCore enum type name. Case-sensitive exact match.")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DatacoreTypeSearchRequest {
    #[schemars(description = "Case-insensitive substring matched against struct AND enum type names. Empty string matches all types.")]
    pub query: String,
    #[schemars(description = "Restrict results: 'struct', 'enum', or 'both' (default 'both').")]
    pub kind: Option<String>,
    #[schemars(description = "Maximum number of results (default 50).")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlendRunScriptRequest {
    #[schemars(description = "Absolute filesystem path to a .blend file to open with Blender.")]
    pub blend_path: String,
    #[schemars(description = "Python script body to execute in headless Blender after opening the file. \
        Wrap your output in print() calls. Output between __BLEND_SCRIPT_OUTPUT_START__ and \
        __BLEND_SCRIPT_OUTPUT_END__ sentinels is returned; if sentinels are absent the full \
        stdout+stderr is returned.")]
    pub script: String,
    #[schemars(description = "Override path to the Blender binary. Falls back to BLENDER_BIN env var, then PATH.")]
    pub blender_bin: Option<String>,
}


pub struct StarBreakerMcp {
    p4k_path: RwLock<Option<PathBuf>>,
    data: RwLock<Option<Arc<GameData>>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl StarBreakerMcp {
    pub fn new(p4k_path: Option<std::path::PathBuf>) -> Self {
        Self {
            p4k_path: RwLock::new(p4k_path),
            data: RwLock::new(None),
            tool_router: Self::tool_router(),
        }
    }

    fn load_game_data(path_override: Option<PathBuf>) -> anyhow::Result<GameData> {
        let start = std::time::Instant::now();
        let (p4k_path, p4k) = match path_override {
            Some(path) => {
                let p4k = starbreaker_p4k::MappedP4k::open(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to open P4k at {}: {e}", path.display()))?;
                (path, p4k)
            }
            None => {
                let (path, source) = starbreaker_p4k::find_p4k()
                    .map_err(|e| anyhow::anyhow!("Failed to auto-discover P4k: {e}"))?;
                let p4k = starbreaker_p4k::MappedP4k::open(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to open auto-discovered P4k at {}: {e}", path.display()))?;
                log::info!("P4K: {} ({source})", path.display());
                (path, p4k)
            }
        };
        let p4k = Arc::new(p4k);
        log::info!("P4k loaded from {} in {:.1}s", p4k_path.display(), start.elapsed().as_secs_f32());

        let dcb_bytes = p4k
            .read_file("Data\\Game2.dcb")
            .or_else(|_| p4k.read_file("Data\\Game.dcb"))
            .map_err(|e| anyhow::anyhow!("Failed to read DataCore from {}: {e}", p4k_path.display()))?;
        let dcb_bytes: &'static [u8] = Box::leak(dcb_bytes.into_boxed_slice());
        let db = Database::from_bytes(dcb_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse DataCore from {}: {e}", p4k_path.display()))?;
        log::info!(
            "DataCore: {} bytes, loaded in {:.1}s",
            dcb_bytes.len(),
            start.elapsed().as_secs_f32()
        );

        Ok(GameData {
            p4k_path,
            p4k,
            dcb_bytes,
            db: Arc::new(db),
        })
    }

    /// Lazily load P4k and DataCore on first access.
    fn data(&self) -> Arc<GameData> {
        if let Some(data) = self.data.read().expect("data lock poisoned").as_ref().cloned() {
            return data;
        }

        let mut data_guard = self.data.write().expect("data lock poisoned");
        if let Some(data) = data_guard.as_ref().cloned() {
            return data;
        }

        let p4k_path = self.p4k_path.read().expect("p4k path lock poisoned").clone();
        let data = Arc::new(
            Self::load_game_data(p4k_path)
                .unwrap_or_else(|e| panic!("{e}")),
        );
        *data_guard = Some(data.clone());
        data
    }

    fn switch_p4k_path(&self, path: PathBuf) -> anyhow::Result<Arc<GameData>> {
        let data = Arc::new(Self::load_game_data(Some(path.clone()))?);
        *self.p4k_path.write().expect("p4k path lock poisoned") = Some(path);
        *self.data.write().expect("data lock poisoned") = Some(data.clone());
        Ok(data)
    }

    /// Find an entity record by name substring (shortest match).
    fn find_entity<'a>(
        &self,
        db: &'a starbreaker_datacore::database::Database<'a>,
        search: &str,
    ) -> Option<&'a starbreaker_datacore::types::Record> {
        let search = search.to_lowercase();
        let entity_si = db.struct_id("EntityClassDefinition")?;
        let mut candidates: Vec<_> = db
            .records_of_type(entity_si)
            .filter(|r| {
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        candidates.first().copied()
    }

    /// Normalize a path for P4k lookup (ensure Data\ prefix, backslashes).
    fn normalize_p4k_path(path: &str) -> String {
        let p = if path.to_lowercase().starts_with("data\\") || path.to_lowercase().starts_with("data/") {
            path.replace('/', "\\")
        } else {
            format!("Data\\{}", path.replace('/', "\\"))
        };
        // Auto-convert .tif to .dds for texture lookups
        if p.to_lowercase().ends_with(".tif") {
            format!("{}.dds", &p[..p.len() - 4])
        } else {
            p
        }
    }

    fn split_socpak_entry_path(path: &str) -> Option<(&str, &str)> {
        let (outer, inner) = path.split_once("::")?;
        let outer = outer.trim();
        let inner = inner.trim();
        if outer.is_empty() || inner.is_empty() {
            return None;
        }
        Some((outer, inner))
    }

    fn read_p4k_file_direct(&self, path: &str) -> Result<Vec<u8>, String> {
        let p4k_path = Self::normalize_p4k_path(path);
        let data = self.data();
        data.p4k.read_file(&p4k_path)
            .or_else(|_| {
                data.p4k.entry_case_insensitive(&p4k_path)
                    .ok_or_else(|| format!("File not found in P4k: {p4k_path}"))
                    .and_then(|entry| data.p4k.read(entry).map_err(|e| format!("Error reading: {e}")))
            })
            .map_err(|e| format!("{e}"))
    }

    /// Read a file from P4k with case-insensitive fallback.
    fn read_p4k_file(&self, path: &str) -> Result<Vec<u8>, String> {
        if let Some((socpak_path, entry_path)) = Self::split_socpak_entry_path(path) {
            let socpak_bytes = self.read_p4k_file_direct(socpak_path)?;
            let archive = P4kArchive::from_bytes(&socpak_bytes)
                .map_err(|e| format!("Failed to parse socpak ZIP '{}': {e}", socpak_path))?;
            let requested = entry_path.replace('/', "\\").to_ascii_lowercase();
            let entry = archive
                .entries()
                .iter()
                .find(|entry| entry.name.replace('/', "\\").to_ascii_lowercase() == requested)
                .ok_or_else(|| {
                    format!(
                        "Entry not found in socpak '{}': {}",
                        Self::normalize_p4k_path(socpak_path),
                        entry_path
                    )
                })?;
            return archive
                .read(entry)
                .map_err(|e| format!("Failed to read '{}' from '{}': {e}", entry_path, socpak_path));
        }
        self.read_p4k_file_direct(path)
    }

    /// Read a file either from disk (if the path exists on disk) or
    /// from P4k. Used by debug tools that may receive either an
    /// extracted scratch file or a P4k-internal path.
    fn read_p4k_or_disk(&self, path: &str) -> Result<Vec<u8>, String> {
        let direct = std::path::Path::new(path);
        if direct.is_file() {
            return std::fs::read(direct).map_err(|e| format!("disk read failed: {e}"));
        }
        self.read_p4k_file(path)
    }

    fn decode_archive_entry_bytes(path: &str, data: &[u8]) -> String {
        let lower = path.to_lowercase();
        let is_cryxml_ext = lower.ends_with(".xml")
            || lower.ends_with(".mtl")
            || lower.ends_with(".chrparams")
            || lower.ends_with(".cdf")
            || lower.ends_with(".adb")
            || lower.ends_with(".comb")
            || lower.ends_with(".entxml");

        if is_cryxml_ext {
            if let Ok(xml) = starbreaker_cryxml::from_bytes(data) {
                return format!("{xml}");
            }
            if let Ok(text) = std::str::from_utf8(data) {
                return text.to_string();
            }
        }

        if let Ok(text) = std::str::from_utf8(data) {
            return text.to_string();
        }

        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        format!("[base64, {} bytes]\n{encoded}", data.len())
    }

    /// Find any record by GUID or name substring.
    fn find_record<'a>(
        &self,
        db: &'a starbreaker_datacore::database::Database<'a>,
        id: &str,
    ) -> Option<&'a starbreaker_datacore::types::Record> {
        if let Ok(guid) = id.parse::<starbreaker_common::CigGuid>() {
            return db.record_by_id(&guid);
        }
        let search = id.to_lowercase();
        let mut candidates: Vec<_> = db
            .records()
            .iter()
            .filter(|r| {
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        candidates.first().copied()
    }
}

/// P4K-backed asset fetcher for MCP UI tools.

pub(crate) struct P4kAssetFetcher {
    p4k: Arc<MappedP4k>,
}

impl P4kAssetFetcher {
    /// Create a new asset fetcher from the game data.
    pub(crate) fn new(data: &Arc<GameData>) -> Self {
        Self {
            p4k: Arc::clone(&data.p4k),
        }
    }

    /// Normalize a path for P4K lookup: ensure `Data\` prefix,
    /// convert forward slashes to backslashes, auto-convert `.tif` to `.dds`.
    fn normalize_path(path: &str) -> String {
        let p = if path.to_lowercase().starts_with("data\\") || path.to_lowercase().starts_with("data/") {
            path.replace('/', "\\")
        } else {
            format!("Data\\{}", path.replace('/', "\\"))
        };
        // Auto-convert .tif to .dds for texture lookups
        if p.to_lowercase().ends_with(".tif") {
            format!("{}.dds", &p[..p.len() - 4])
        } else {
            p
        }
    }
}

impl starbreaker_ui::pipeline::AssetFetcher for P4kAssetFetcher {
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        let normalized = Self::normalize_path(p4k_path);
        self.p4k
            .read_file(&normalized)
            .ok()
            .or_else(|| {
                // Case-insensitive fallback
                self.p4k
                    .entry_case_insensitive(&normalized)
                    .and_then(|entry| self.p4k.read(entry).ok())
            })
    }
}

/// P4K-backed SWF fetcher for MCP UI tools.
///
/// Implements [`starbreaker_ui::SwfFetcher`] by reading SWF files
/// from the P4K archive loaded from `Data.p4k`.
/// Replaces `McpNullSwfFetcher` which always returned an error.
pub(crate) struct P4kSwfFetcher {
    p4k: Arc<MappedP4k>,
}

impl P4kSwfFetcher {
    /// Create a new SWF fetcher from the game data.
    pub(crate) fn new(data: &Arc<GameData>) -> Self {
        Self {
            p4k: Arc::clone(&data.p4k),
        }
    }

    /// Normalize a path for P4K lookup: ensure `Data\` prefix,
    /// convert forward slashes to backslashes, auto-convert `.tif` to `.dds`.
    fn normalize_path(path: &str) -> String {
        let p = if path.to_lowercase().starts_with("data\\") || path.to_lowercase().starts_with("data/") {
            path.replace('/', "\\")
        } else {
            format!("Data\\{}", path.replace('/', "\\"))
        };
        // Auto-convert .tif to .dds for texture lookups
        if p.to_lowercase().ends_with(".tif") {
            format!("{}.dds", &p[..p.len() - 4])
        } else {
            p
        }
    }
}

impl starbreaker_ui::SwfFetcher for P4kSwfFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, starbreaker_ui::UiError> {
        let normalized = Self::normalize_path(p4k_path);
        self.p4k
            .read_file(&normalized)
            .or_else(|_| {
                // Case-insensitive fallback
                self.p4k
                    .entry_case_insensitive(&normalized)
                    .ok_or_else(|| format!("SWF file not found in P4K: {normalized}"))
                    .and_then(|entry| {
                        self.p4k.read(entry).map_err(|e| format!("Error reading SWF from P4K: {e}"))
                    })
            })
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: p4k_path.to_string(),
                source: e.into(),
            })
    }
}

/// DataCore-backed canvas fetcher for MCP UI tools.
///
/// Implements [`starbreaker_ui::CanvasFetcher`] by querying the
/// `BuildingBlocks_Canvas` DataCore records loaded from P4K.
/// Replaces `LocalUiRecordIndex` which scanned local JSON files.
pub(crate) struct P4kCanvasFetcher {
    db: Arc<Database<'static>>,
    /// Record families resolved by GUID/name. `BuildingBlocks_Canvas` plus the
    /// UI-support families the canvas pipeline fetches by name — currently
    /// `BuildingBlocks_AspectRatioLibrary` so the MFD aspect-tag content-scaling
    /// (`AspectRatioToTag_MFD`) resolves and `ui_ir_query` exercises the same
    /// path as the export (ledger 43; mirrors `datacore_ui_lookup_type_names`).
    lookup_struct_ids: Vec<StructId>,
}

impl P4kCanvasFetcher {
    /// Create a new canvas fetcher from the game data.
    ///
    /// Resolves the searched struct IDs at construction time so runtime lookups
    /// stay fast. `BuildingBlocks_Canvas` is required; the supplementary families
    /// are best-effort (skipped if absent).
    pub(crate) fn new(data: &Arc<GameData>) -> Result<Self, String> {
        let canvas_struct_id = data
            .db
            .struct_id("BuildingBlocks_Canvas")
            .ok_or("DataCore has no BuildingBlocks_Canvas struct")?;
        let mut lookup_struct_ids = vec![canvas_struct_id];
        if let Some(aspect) = data.db.struct_id("BuildingBlocks_AspectRatioLibrary") {
            lookup_struct_ids.push(aspect);
        }
        Ok(Self {
            db: Arc::clone(&data.db),
            lookup_struct_ids,
        })
    }

    /// Search the indexed record families for a matching GUID.
    fn find_by_guid(&self, guid: &str) -> Option<&starbreaker_datacore::types::Record> {
        let parsed = guid.parse::<starbreaker_common::CigGuid>().ok();
        self.lookup_struct_ids.iter().find_map(|sid| {
            self.db.records_of_type(*sid).find(|r| {
                parsed.is_some_and(|p| r.id == p) || format!("{}", r.id) == guid
            })
        })
    }

    /// Search the indexed record families for a matching name substring (shortest match).
    fn find_by_name(&self, name: &str) -> Option<&starbreaker_datacore::types::Record> {
        let search = name.to_lowercase();
        let mut candidates: Vec<_> = self
            .lookup_struct_ids
            .iter()
            .flat_map(|sid| self.db.records_of_type(*sid))
            .filter(|r| {
                self.db
                    .resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        candidates.sort_by_key(|r| self.db.resolve_string2(r.name_offset).len());
        candidates.first().copied()
    }
}

/// DataCore-backed style fetcher for MCP UI tools.
///
/// Implements [`starbreaker_ui::StyleFetcher`] by querying the
/// `BuildingBlocks_Style` DataCore records loaded from P4K.
/// Replaces `LocalUiStyleFetcher` which scanned local JSON files.
pub(crate) struct P4kStyleFetcher {
    db: Arc<Database<'static>>,
    style_struct_id: StructId,
}

impl P4kStyleFetcher {
    /// Create a new style fetcher from the game data.
    ///
    /// Resolves the struct ID for `BuildingBlocks_Style` at
    /// construction time so runtime lookups stay fast.
    pub(crate) fn new(data: &Arc<GameData>) -> Result<Self, String> {
        let style_struct_id = data
            .db
            .struct_id("BuildingBlocks_Style")
            .ok_or("DataCore has no BuildingBlocks_Style struct")?;
        Ok(Self {
            db: Arc::clone(&data.db),
            style_struct_id,
        })
    }

    /// Parse a `ManufacturerStyle` from a DataCore style record.
    fn parse_style(&self, record: &starbreaker_datacore::types::Record) -> Result<starbreaker_ui::ManufacturerStyle, starbreaker_ui::UiError> {
        let bytes = starbreaker_datacore::export::to_json(&self.db, record)
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: "style".to_string(),
                source: format!("DataCore JSON export failed: {e}").into(),
            })?;

        let json_str = String::from_utf8(bytes)
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: "style".to_string(),
                source: format!("DataCore JSON is not valid UTF-8: {e}").into(),
            })?;

        let record_json: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: "style".to_string(),
                source: format!("Failed to parse record JSON: {e}").into(),
            })?;

        // Extract the manufacturer ID from the record name (e.g. "BuildingBlocks_Style.DRAK" → "DRAK")
        let record_name = self.db.resolve_string2(record.name_offset);
        let manufacturer_id = record_name
            .rsplit('.')
            .next()
            .unwrap_or(&record_name)
            .to_ascii_lowercase();

        starbreaker_ui::StyleLoader::for_manufacturer(&manufacturer_id)
            .parse_buildingblocks_style_record(&record_json)
    }
}

impl starbreaker_ui::StyleFetcher for P4kStyleFetcher {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<starbreaker_ui::ManufacturerStyle, starbreaker_ui::UiError> {
        let id = manufacturer_id.to_ascii_lowercase();
        // Try exact manufacturer name first, then search by substring
        let candidates: Vec<_> = self
            .db
            .records_of_type(self.style_struct_id)
            .filter(|r| {
                let name = self.db.resolve_string2(r.name_offset);
                let bare = name.rsplit('.').next().unwrap_or(&name).to_ascii_lowercase();
                bare == id || bare.contains(&id)
            })
            .collect();

        if candidates.is_empty() {
            return Err(starbreaker_ui::UiError::RenderError(format!(
                "missing BuildingBlocks style record for manufacturer '{}'",
                manufacturer_id
            )));
        }

        // Sort by name length (shortest match wins)
        let mut sorted = candidates;
        sorted.sort_by_key(|r| self.db.resolve_string2(r.name_offset).len());

        self.parse_style(sorted.first().unwrap())
    }
}

impl starbreaker_ui::CanvasFetcher for P4kCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, starbreaker_ui::UiError> {
        let record = self
            .find_by_guid(guid)
            .ok_or_else(|| starbreaker_ui::UiError::FetchFailed {
                guid: guid.to_string(),
                source: format!("No BuildingBlocks_Canvas record with GUID {guid}").into(),
            })?;

        let bytes = starbreaker_datacore::export::to_json(&self.db, record)
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: guid.to_string(),
                source: format!("DataCore JSON export failed: {e}").into(),
            })?;

        let json_str = String::from_utf8(bytes)
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: guid.to_string(),
                source: format!("DataCore JSON is not valid UTF-8: {e}").into(),
            })?;

        serde_json::from_str(&json_str).map_err(|e| starbreaker_ui::UiError::FetchFailed {
            guid: guid.to_string(),
            source: format!("Failed to parse record JSON: {e}").into(),
        })
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, starbreaker_ui::UiError> {
        let record = self
            .find_by_name(record_name)
            .ok_or_else(|| starbreaker_ui::UiError::FetchFailed {
                guid: record_name.to_string(),
                source: format!("No BuildingBlocks_Canvas record matching name '{record_name}'").into(),
            })?;

        let bytes = starbreaker_datacore::export::to_json(&self.db, record)
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: record_name.to_string(),
                source: format!("DataCore JSON export failed: {e}").into(),
            })?;

        let json_str = String::from_utf8(bytes)
            .map_err(|e| starbreaker_ui::UiError::FetchFailed {
                guid: record_name.to_string(),
                source: format!("DataCore JSON is not valid UTF-8: {e}").into(),
            })?;

        serde_json::from_str(&json_str).map_err(|e| starbreaker_ui::UiError::FetchFailed {
            guid: record_name.to_string(),
            source: format!("Failed to parse record JSON: {e}").into(),
        })
    }
}


fn mcp_error_json(code: &str, message: String) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": 1,
        "error_code": code,
        "message": message,
    }))
    .unwrap_or_else(|_| format!("{{\"error_code\":\"{code}\"}}"))
}

fn json_rect(rect: impl Into<(f32, f32, f32, f32)>) -> serde_json::Value {
    let (x, y, w, h) = rect.into();
    serde_json::json!({ "x": x, "y": y, "w": w, "h": h })
}

fn bb_rect_json(rect: starbreaker_ui::bb_layout::Rect) -> serde_json::Value {
    json_rect((rect.x, rect.y, rect.w, rect.h))
}

fn ui_rect_json(rect: starbreaker_ui::UiIrRect) -> serde_json::Value {
    json_rect((rect.x, rect.y, rect.w, rect.h))
}

fn compact_colour_fields(raw: &serde_json::Value) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    for key in [
        "FillColor",
        "FillColorToken",
        "BackgroundColor",
        "BackgroundColorToken",
        "BorderColorLeft",
        "BorderColorLeftToken",
        "BorderColorRight",
        "BorderColorRightToken",
        "BorderColorTop",
        "BorderColorTopToken",
        "BorderColorBottom",
        "BorderColorBottomToken",
        "EnableBackground",
        "IsActive",
    ] {
        if let Some(value) = raw.get(key) {
            fields.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(fields)
}

fn style_entry_name(entry: &serde_json::Value) -> Option<&str> {
    entry.get("name").and_then(|v| v.as_str())
}

fn summarize_style_entry(
    source: &str,
    index: usize,
    entry: &serde_json::Value,
    include_conditions: bool,
    include_modifiers: bool,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("source".to_string(), serde_json::Value::String(source.to_string()));
    obj.insert("index".to_string(), serde_json::json!(index));
    obj.insert(
        "name".to_string(),
        style_entry_name(entry)
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null),
    );
    obj.insert(
        "condition_list_count".to_string(),
        serde_json::json!(entry.get("conditionsList").and_then(|v| v.as_array()).map(Vec::len).unwrap_or(0)),
    );
    obj.insert(
        "modifier_count".to_string(),
        serde_json::json!(entry.get("modifiers").and_then(|v| v.as_array()).map(Vec::len).unwrap_or(0)),
    );
    obj.insert(
        "transition_count".to_string(),
        serde_json::json!(entry.get("transitions").and_then(|v| v.as_array()).map(Vec::len).unwrap_or(0)),
    );
    if include_conditions {
        obj.insert("conditions".to_string(), summarize_conditions(entry));
    }
    if include_modifiers {
        obj.insert("modifiers".to_string(), summarize_modifiers(entry));
    }
    serde_json::Value::Object(obj)
}

fn summarize_conditions(entry: &serde_json::Value) -> serde_json::Value {
    let mut out = Vec::new();
    if let Some(blocks) = entry.get("conditionsList").and_then(|v| v.as_array()) {
        for block in blocks {
            if let Some(conditions) = block.get("conditions").and_then(|v| v.as_array()) {
                for condition in conditions {
                    collect_condition_summary(condition, &mut out);
                }
            }
        }
    }
    serde_json::Value::Array(out)
}

fn collect_condition_summary(condition: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
    let condition_type = condition
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim_start_matches("BuildingBlocks_StyleSelector");
    let mut obj = serde_json::Map::new();
    obj.insert("type".to_string(), serde_json::Value::String(condition_type.to_string()));
    if let Some(widget_type) = condition.get("type").and_then(|v| v.as_str()) {
        obj.insert("widget_type".to_string(), serde_json::Value::String(widget_type.to_string()));
    }
    if let Some(tag) = condition.get("tag") {
        obj.insert("tag".to_string(), summarize_tag_ref(tag));
    }
    if let Some(tags) = condition.get("tags").and_then(|v| v.as_array()) {
        obj.insert(
            "tags".to_string(),
            serde_json::Value::Array(tags.iter().map(summarize_tag_ref).collect()),
        );
    }
    out.push(serde_json::Value::Object(obj));

    if let Some(children) = condition.get("conditions").and_then(|v| v.as_array()) {
        for child in children {
            collect_condition_summary(child, out);
        }
    }
}

fn summarize_tag_ref(tag: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": tag.get("_RecordId_").and_then(|v| v.as_str()).or_else(|| tag.as_str()),
        "name": tag.get("_RecordName_").and_then(|v| v.as_str()),
    })
}

fn summarize_modifiers(entry: &serde_json::Value) -> serde_json::Value {
    let Some(modifiers) = entry.get("modifiers").and_then(|v| v.as_array()) else {
        return serde_json::Value::Array(Vec::new());
    };
    serde_json::Value::Array(
        modifiers
            .iter()
            .map(|modifier| {
                serde_json::json!({
                    "type": modifier.get("_Type_").and_then(|v| v.as_str()),
                    "field": modifier_field_name(modifier),
                    "value": modifier.get("value"),
                    "color_token": modifier.get("color").and_then(|c| c.get("color")).and_then(|v| v.as_str()),
                    "alpha": modifier.get("color").and_then(|c| c.get("alpha")),
                    "record_value": modifier.get("field").and_then(|f| f.get("value")).and_then(|v| v.as_str()),
                })
            })
            .collect(),
    )
}

fn modifier_field_name(modifier: &serde_json::Value) -> Option<String> {
    modifier
        .get("field")
        .and_then(|field| field.as_str().map(str::to_string).or_else(|| {
            field
                .get("_Type_")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        }))
}

/// Walk the live tag database record for `"_Type_": "Tag"` nodes, returning
/// `(uuid, tagName)` pairs. The database is one `TagDatabase` record with the
/// full tag tree nested inside (the identity layer behind node `styleTags`
/// and kit-sheet entry conditions).
fn load_tag_names(db: &Database<'static>) -> Result<Vec<(String, String)>, String> {
    let struct_id = db
        .struct_id("TagDatabase")
        .ok_or_else(|| "DataCore has no TagDatabase struct".to_string())?;
    let record = db
        .records_of_type(struct_id)
        .next()
        .ok_or_else(|| "no TagDatabase record".to_string())?;
    let bytes = starbreaker_datacore::export::to_json(db, record)
        .map_err(|e| format!("TagDatabase JSON export failed: {e}"))?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("TagDatabase JSON parse failed: {e}"))?;
    let mut tags = Vec::new();
    collect_tag_nodes(&json, &mut tags);
    Ok(tags)
}

fn collect_tag_nodes(value: &serde_json::Value, out: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("_Type_").and_then(|v| v.as_str()) == Some("Tag") {
                if let (Some(id), Some(name)) = (
                    map.get("_RecordId_").and_then(|v| v.as_str()),
                    map.get("tagName").and_then(|v| v.as_str()),
                ) {
                    out.push((id.to_string(), name.to_string()));
                }
            }
            for v in map.values() {
                collect_tag_nodes(v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                collect_tag_nodes(v, out);
            }
        }
        _ => {}
    }
}

/// Compact one style-entry condition with tag UUIDs resolved to names
/// (`Tag(text-element-instance)`, `Ancestor breakConditions[...] ...`).
fn summarize_tagged_condition(
    cond: &serde_json::Value,
    tags: &std::collections::HashMap<String, String>,
) -> String {
    let ty = cond
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .replace("BuildingBlocks_StyleSelectorCondition", "");
    let tag_name = |id: &str| tags.get(id).cloned().unwrap_or_else(|| format!("{}?", &id[..id.len().min(8)]));
    let mut parts = Vec::new();
    let tag_id = cond
        .get("tag")
        .and_then(|t| t.get("_RecordId_"))
        .and_then(|v| v.as_str());
    match tag_id {
        Some(id) => parts.push(format!("{ty}({})", tag_name(id))),
        None => parts.push(ty),
    }
    for key in ["breakConditions", "conditions"] {
        if let Some(items) = cond.get(key).and_then(|v| v.as_array()) {
            if !items.is_empty() {
                let inner: Vec<String> = items
                    .iter()
                    .map(|c| summarize_tagged_condition(c, tags))
                    .collect();
                parts.push(format!("{key}[{}]", inner.join(", ")));
            }
        }
    }
    if let Some(anyof) = cond.get("tags").and_then(|v| v.as_array()) {
        if !anyof.is_empty() {
            let names: Vec<String> = anyof
                .iter()
                .filter_map(|t| t.get("_RecordId_").and_then(|v| v.as_str()))
                .map(tag_name)
                .collect();
            parts.push(format!("anyOf[{}]", names.join(", ")));
        }
    }
    parts.join(" ")
}

/// Compact one style-entry modifier: `Field = ColorStyle:<role> a=<alpha>`,
/// `Field = ColorSolid rgba(...)`, or `Field = <value>`.
fn summarize_typed_modifier(modifier: &serde_json::Value) -> String {
    let field = modifier.get("field").and_then(|v| v.as_str()).unwrap_or("?");
    if let Some(colour) = modifier.get("color") {
        if colour.get("_Type_").and_then(|v| v.as_str()) == Some("BuildingBlocks_ColorStyle") {
            return format!(
                "{field} = ColorStyle:{} a={}",
                colour.get("color").and_then(|v| v.as_str()).unwrap_or("?"),
                colour.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0)
            );
        }
        if let Some(inner) = colour.get("color") {
            if inner.get("_Type_").and_then(|v| v.as_str()) == Some("SRGBA8") {
                return format!(
                    "{field} = ColorSolid rgba({},{},{},{})",
                    inner.get("r").and_then(|v| v.as_u64()).unwrap_or(0),
                    inner.get("g").and_then(|v| v.as_u64()).unwrap_or(0),
                    inner.get("b").and_then(|v| v.as_u64()).unwrap_or(0),
                    inner.get("a").and_then(|v| v.as_u64()).unwrap_or(0)
                );
            }
        }
        return format!("{field} = {}", serde_json::to_string(colour).unwrap_or_default());
    }
    format!(
        "{field} = {}",
        serde_json::to_string(modifier.get("value").unwrap_or(&serde_json::Value::Null))
            .unwrap_or_default()
    )
}

#[tool_router]
impl StarBreakerMcp {
    #[tool(description = "Return the currently active Data.p4k path used by StarBreaker MCP tools.")]
    fn p4k_data_status(&self) -> String {
        let data = self.data();
        serde_json::to_string_pretty(&serde_json::json!({
            "p4k_path": data.p4k_path,
            "entries": data.p4k.entries().len(),
            "datacore_bytes": data.dcb_bytes.len(),
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Inspect UI regression targets by reconciling manifest and freeze metadata. Returns deterministic JSON with target ids, tiers, source paths, freeze hash presence, and status (`matched`, `in_manifest_only`, `in_freeze_only`).")]
    fn ui_regression_registry(&self, Parameters(req): Parameters<UiRegressionRegistryRequest>) -> String {
        fn error_json(code: &str, message: String) -> String {
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "error_code": code,
                "message": message,
            }))
            .unwrap_or_else(|_| format!("{{\"error_code\":\"{code}\"}}"))
        }

        fn as_array<'a>(root: &'a serde_json::Value, key: &str) -> Result<&'a Vec<serde_json::Value>, String> {
            root.get(key)
                .and_then(|v| v.as_array())
                .ok_or_else(|| format!("missing or invalid '{key}' array"))
        }

        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));

        let manifest_rel = req
            .manifest_path
            .unwrap_or_else(|| "crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json".to_string());
        let freeze_rel = req
            .freeze_path
            .unwrap_or_else(|| "crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json".to_string());

        let resolve = |path: &str| -> PathBuf {
            let p = Path::new(path);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                repo_root.join(p)
            }
        };

        let manifest_path = resolve(&manifest_rel);
        let freeze_path = resolve(&freeze_rel);

        let manifest_raw = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(e) => {
                return error_json(
                    "manifest_read_failed",
                    format!("failed to read manifest '{}': {e}", manifest_path.display()),
                )
            }
        };
        let freeze_raw = match std::fs::read_to_string(&freeze_path) {
            Ok(s) => s,
            Err(e) => {
                return error_json(
                    "freeze_read_failed",
                    format!("failed to read freeze '{}': {e}", freeze_path.display()),
                )
            }
        };

        let manifest_json: serde_json::Value = match serde_json::from_str(&manifest_raw) {
            Ok(v) => v,
            Err(e) => {
                return error_json(
                    "manifest_parse_failed",
                    format!("failed to parse manifest '{}': {e}", manifest_path.display()),
                )
            }
        };
        let freeze_json: serde_json::Value = match serde_json::from_str(&freeze_raw) {
            Ok(v) => v,
            Err(e) => {
                return error_json(
                    "freeze_parse_failed",
                    format!("failed to parse freeze '{}': {e}", freeze_path.display()),
                )
            }
        };

        let manifest_schema = manifest_json.get("schema_version").and_then(|v| v.as_u64());
        let freeze_schema = freeze_json.get("schema_version").and_then(|v| v.as_u64());
        if manifest_schema != Some(1) || freeze_schema != Some(1) || manifest_schema != freeze_schema {
            return error_json(
                "schema_mismatch",
                format!(
                    "manifest/freeze schema mismatch: manifest={manifest_schema:?} freeze={freeze_schema:?}; expected both to be 1"
                ),
            );
        }

        let targets = match as_array(&manifest_json, "targets") {
            Ok(v) => v,
            Err(msg) => return error_json("manifest_invalid", msg),
        };
        let artifacts = match as_array(&freeze_json, "artifacts") {
            Ok(v) => v,
            Err(msg) => return error_json("freeze_invalid", msg),
        };

        let mut manifest_map: std::collections::BTreeMap<String, serde_json::Value> =
            std::collections::BTreeMap::new();
        for target in targets {
            let Some(id) = target.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            manifest_map.insert(id.to_string(), target.clone());
        }

        let mut freeze_map: std::collections::BTreeMap<String, serde_json::Value> =
            std::collections::BTreeMap::new();
        for artifact in artifacts {
            let Some(id) = artifact.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            freeze_map.insert(id.to_string(), artifact.clone());
        }

        let filter = req.target_filter.map(|s| s.to_ascii_lowercase());
        let mut all_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        all_ids.extend(manifest_map.keys().cloned());
        all_ids.extend(freeze_map.keys().cloned());

        let mut rows = Vec::new();
        let mut matched = 0usize;
        let mut in_manifest_only = 0usize;
        let mut in_freeze_only = 0usize;

        for id in all_ids {
            if let Some(ref needle) = filter {
                if !id.to_ascii_lowercase().contains(needle) {
                    continue;
                }
            }

            let manifest_target = manifest_map.get(&id);
            let freeze_target = freeze_map.get(&id);

            let status = match (manifest_target, freeze_target) {
                (Some(_), Some(_)) => {
                    matched += 1;
                    "matched"
                }
                (Some(_), None) => {
                    in_manifest_only += 1;
                    "in_manifest_only"
                }
                (None, Some(_)) => {
                    in_freeze_only += 1;
                    "in_freeze_only"
                }
                (None, None) => continue,
            };

            let manifest_tier = manifest_target
                .and_then(|v| v.get("tier"))
                .and_then(|v| v.as_str());
            let freeze_tier = freeze_target
                .and_then(|v| v.get("tier"))
                .and_then(|v| v.as_str());
            let source_generated_png = manifest_target
                .and_then(|v| v.get("source_generated_png"))
                .and_then(|v| v.as_str())
                .or_else(|| freeze_target.and_then(|v| v.get("source_generated_png")).and_then(|v| v.as_str()));
            let freeze_hash = freeze_target
                .and_then(|v| v.get("sha256"))
                .and_then(|v| v.as_str());

            rows.push(serde_json::json!({
                "id": id,
                "status": status,
                "manifest_tier": manifest_tier,
                "freeze_tier": freeze_tier,
                "tier_mismatch": manifest_tier.is_some() && freeze_tier.is_some() && manifest_tier != freeze_tier,
                "source_generated_png": source_generated_png,
                "freeze_hash": freeze_hash,
                "has_freeze_hash": freeze_hash.is_some(),
            }));
        }

        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "manifest_path": manifest_path,
            "freeze_path": freeze_path,
            "summary": {
                "total_targets": rows.len(),
                "matched": matched,
                "in_manifest_only": in_manifest_only,
                "in_freeze_only": in_freeze_only,
            },
            "targets": rows,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Run UI regression baseline integrity validation (quick/full) and return structured JSON output including pass/fail status, exit code, and captured stdout/stderr.")]
    fn ui_regression_validate(&self, Parameters(req): Parameters<UiRegressionValidateRequest>) -> String {
        fn error_json(code: &str, message: String) -> String {
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "error_code": code,
                "message": message,
            }))
            .unwrap_or_else(|_| format!("{{\"error_code\":\"{code}\"}}"))
        }

        let mode = req
            .mode
            .unwrap_or_else(|| "quick".to_string())
            .to_ascii_lowercase();
        let mode_flag = match mode.as_str() {
            "quick" => "--quick",
            "full" => "--full",
            _ => {
                return error_json(
                    "invalid_mode",
                    format!("invalid mode '{mode}' (expected 'quick' or 'full')"),
                )
            }
        };

        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
        let script_path = repo_root.join("scripts/validate_ui_regression_artifacts.sh");
        if !script_path.is_file() {
            return error_json(
                "validator_missing",
                format!("validator script not found: {}", script_path.display()),
            );
        }

        let default_manifest = "crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json".to_string();
        let default_freeze = "crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json".to_string();

        let resolve = |path: &str| -> PathBuf {
            let p = Path::new(path);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                repo_root.join(p)
            }
        };

        let manifest_path = resolve(req.manifest_path.as_deref().unwrap_or(&default_manifest));
        let freeze_path = resolve(req.freeze_path.as_deref().unwrap_or(&default_freeze));

        let output = std::process::Command::new("bash")
            .arg(&script_path)
            .arg(mode_flag)
            .env("UI_REGRESSION_MANIFEST_PATH", &manifest_path)
            .env("UI_REGRESSION_FREEZE_PATH", &freeze_path)
            .current_dir(&repo_root)
            .output();

        let output = match output {
            Ok(out) => out,
            Err(e) => return error_json("validator_exec_failed", format!("failed to execute validator: {e}")),
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "mode": mode,
            "manifest_path": manifest_path,
            "freeze_path": freeze_path,
            "passed": output.status.success(),
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr,
            "error_code": if output.status.success() { serde_json::Value::Null } else { serde_json::Value::String("validation_failed".to_string()) },
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Inspect authored BuildingBlocks style containers for a local decompiled UI canvas. Returns embedded/default/brand/inline style-entry summaries, including conditions and modifiers, so UI matching can find the actual source style bundle before editing code.")]
    fn ui_canvas_style_inventory(&self, Parameters(req): Parameters<UiCanvasStyleInventoryRequest>) -> String {
        let canvas_fetcher = match P4kCanvasFetcher::new(&self.data()) {
            Ok(f) => f,
            Err(e) => return mcp_error_json("canvas_fetcher_init_failed", e),
        };
        let canvas = match canvas_fetcher.fetch_canvas_json(&req.canvas) {
            Ok(c) => c,
            Err(e) => return mcp_error_json("canvas_not_found", format!("{e}")),
        };
        let record_value = canvas.get("_RecordValue_").unwrap_or(&canvas);
        let include_conditions = req.include_conditions.unwrap_or(true);
        let include_modifiers = req.include_modifiers.unwrap_or(true);
        let limit = req.limit.unwrap_or(200).max(1) as usize;
        let filter = req.entry_filter.map(|s| s.to_ascii_lowercase());

        let mut containers = Vec::new();
        let mut entries = Vec::new();
        let mut push_entries = |source: String, maybe_entries: Option<&Vec<serde_json::Value>>| {
            let count = maybe_entries.map(Vec::len).unwrap_or(0);
            containers.push(serde_json::json!({
                "source": source,
                "entry_count": count,
            }));
            let Some(style_entries) = maybe_entries else {
                return;
            };
            for (idx, entry) in style_entries.iter().enumerate() {
                if entries.len() >= limit {
                    return;
                }
                if let Some(ref needle) = filter {
                    let name = style_entry_name(entry).unwrap_or("").to_ascii_lowercase();
                    if !name.contains(needle) {
                        continue;
                    }
                }
                entries.push(summarize_style_entry(
                    containers.last().and_then(|c| c.get("source")).and_then(|v| v.as_str()).unwrap_or("?"),
                    idx,
                    entry,
                    include_conditions,
                    include_modifiers,
                ));
            }
        };

        push_entries(
            "embeddedStyles".to_string(),
            record_value.get("embeddedStyles").and_then(|v| v.as_array()),
        );
        push_entries(
            "defaultStyles.entries".to_string(),
            record_value
                .get("defaultStyles")
                .and_then(|v| v.get("entries"))
                .and_then(|v| v.as_array()),
        );
        if let Some(brand_styles) = record_value.get("brandStyles").and_then(|v| v.as_array()) {
            for (idx, brand) in brand_styles.iter().enumerate() {
                let source = brand_style_label(brand, idx);
                push_entries(source, brand.get("entries").and_then(|v| v.as_array()));
            }
        }

        let mut inline_style_nodes = Vec::new();
        if let Some(scene_nodes) = record_value.get("scene").and_then(|v| v.as_array()) {
            for (scene_idx, node) in scene_nodes.iter().enumerate() {
                let Some(inline_entries) = node.get("inlineStyles").and_then(|v| v.as_array()) else {
                    continue;
                };
                if inline_entries.is_empty() {
                    continue;
                }
                let source = format!(
                    "scene[{scene_idx}].inlineStyles {}",
                    node.get("name").and_then(|v| v.as_str()).unwrap_or("<unnamed>")
                );
                inline_style_nodes.push(serde_json::json!({
                    "scene_index": scene_idx,
                    "name": node.get("name").and_then(|v| v.as_str()),
                    "pointer": node.get("_Pointer_").and_then(|v| v.as_str()),
                    "parent": node.get("parent").and_then(|v| v.as_str()),
                    "entry_count": inline_entries.len(),
                }));
                push_entries(source, Some(inline_entries));
            }
        }

        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "canvas": {
                "record_name": canvas.get("_RecordName_").and_then(|v| v.as_str()),
                "record_id": canvas.get("_RecordId_").and_then(|v| v.as_str()),
                "style_field": record_value.get("style"),
            },
            "containers": containers,
            "inline_style_nodes": inline_style_nodes,
            "returned_entry_count": entries.len(),
            "entry_limit": limit,
            "entries": entries,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Resolve a local decompiled BuildingBlocks canvas and list matching scene nodes with style tags, raw color/tint fields, and applied style-entry names. Use this before changing UI style code to prove which authored styles actually matched.")]
    fn ui_scene_style_probe(&self, Parameters(req): Parameters<UiSceneStyleProbeRequest>) -> String {
        let canvas_fetcher = match P4kCanvasFetcher::new(&self.data()) {
            Ok(f) => f,
            Err(e) => return mcp_error_json("canvas_fetcher_init_failed", e),
        };
        let canvas = match canvas_fetcher.fetch_canvas_json(&req.canvas) {
            Ok(c) => c,
            Err(e) => return mcp_error_json("canvas_not_found", format!("{e}")),
        };
        let manufacturer = req.manufacturer.as_deref();
        let fetch = |path: &str| {
            canvas_fetcher
                .fetch_canvas_by_name(path)
                .map_err(|e| e.to_string())
        };
        let scene = match starbreaker_ui::bb_resolve::resolve_canvas_graph_with_loc(
            &canvas,
            manufacturer,
            &fetch,
            None,
        ) {
            Ok(scene) => scene,
            Err(e) => return mcp_error_json("scene_resolve_failed", e),
        };

        let query = req.query.to_ascii_lowercase();
        let limit = req.limit.unwrap_or(80).max(1) as usize;
        let mut nodes = Vec::new();
        for node in scene.nodes.values() {
            let node_type = format!("{:?}", node.ty);
            let text = node.text.as_ref().map(|text| text.string.clone()).unwrap_or_default();
            let haystack = format!("{} {} {}", node.name, node_type, text).to_ascii_lowercase();
            if !haystack.contains(&query) {
                continue;
            }
            if nodes.len() >= limit {
                break;
            }
            let applied_entries: Vec<_> = node
                .raw
                .get("__AppliedStyleEntries")
                .and_then(|v| v.as_array())
                .map(|entries| {
                    entries
                        .iter()
                        .map(|entry| {
                            serde_json::json!({
                                "name": style_entry_name(entry),
                                "modifier_count": entry.get("modifiers").and_then(|v| v.as_array()).map(Vec::len).unwrap_or(0),
                                "modifiers": summarize_modifiers(entry),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            nodes.push(serde_json::json!({
                "id": node.id,
                "parent": node.parent,
                "children": node.children,
                "name": node.name,
                "type": node_type,
                "is_active": node.is_active,
                "alpha": node.alpha,
                "style_tag_uuids": node.style_tag_uuids,
                "text": text,
                "primary_state_tag": node.raw.get("PrimaryStateTag"),
                "colour_fields": compact_colour_fields(&node.raw),
                "applied_style_entries": applied_entries,
            }));
        }

        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "canvas": {
                "record_name": canvas.get("_RecordName_").and_then(|v| v.as_str()),
                "record_id": canvas.get("_RecordId_").and_then(|v| v.as_str()),
            },
            "manufacturer": manufacturer,
            "query": req.query,
            "matched_node_count": nodes.len(),
            "node_limit": limit,
            "nodes": nodes,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Resolve UI style-tag identity against the live tag database: a UUID (or prefix) returns its tag name; a name substring returns matching tags. Tags are the identity layer behind node styleTags and kit-sheet entry conditions.")]
    fn ui_tag_lookup(&self, Parameters(req): Parameters<UiTagLookupRequest>) -> String {
        let tags = match load_tag_names(&self.data().db) {
            Ok(t) => t,
            Err(e) => return mcp_error_json("tag_database_unavailable", e),
        };
        let query = req.query.to_ascii_lowercase();
        let limit = req.limit.unwrap_or(40).max(1) as usize;
        let mut matches: Vec<(String, String)> = tags
            .into_iter()
            .filter(|(id, name)| {
                id.to_ascii_lowercase().starts_with(&query)
                    || name.to_ascii_lowercase().contains(&query)
            })
            .collect();
        matches.sort_by(|a, b| a.1.cmp(&b.1));
        let total = matches.len();
        matches.truncate(limit);
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "query": req.query,
            "matched": total,
            "returned": matches.len(),
            "tags": matches
                .into_iter()
                .map(|(uuid, tag_name)| serde_json::json!({"uuid": uuid, "tag_name": tag_name}))
                .collect::<Vec<_>>(),
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Dump a BuildingBlocks_Style record's entries (modular-kit component sheets like sk_uilo_a_buttonprimarystyles included) with tag-NAME-resolved conditions and compact modifier values (ColorStyle roles / ColorSolid RGBA). MCP-native form of scripts/ui_canvas_query.py entries.")]
    fn ui_kit_sheet_entries(&self, Parameters(req): Parameters<UiKitSheetEntriesRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let Some(style_struct_id) = db.struct_id("BuildingBlocks_Style") else {
            return mcp_error_json(
                "style_struct_missing",
                "DataCore has no BuildingBlocks_Style struct".to_string(),
            );
        };
        let search = req.record.to_lowercase();
        let mut candidates: Vec<_> = db
            .records_of_type(style_struct_id)
            .filter(|r| {
                db.resolve_string2(r.name_offset).to_lowercase().contains(&search)
                    || format!("{}", r.id) == req.record
            })
            .collect();
        candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        let Some(record) = candidates.first() else {
            return mcp_error_json(
                "style_record_not_found",
                format!("no BuildingBlocks_Style record matching '{}'", req.record),
            );
        };
        let record_name = db.resolve_string2(record.name_offset).to_string();
        let json = match starbreaker_datacore::export::to_json(db, record)
            .map_err(|e| format!("record JSON export failed: {e}"))
            .and_then(|bytes| {
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .map_err(|e| format!("record JSON parse failed: {e}"))
            }) {
            Ok(v) => v,
            Err(e) => return mcp_error_json("record_export_failed", e),
        };
        let record_value = json.get("_RecordValue_").unwrap_or(&json);
        let tag_names: std::collections::HashMap<String, String> =
            load_tag_names(db).unwrap_or_default().into_iter().collect();
        let filter = req.entry_filter.map(|s| s.to_ascii_lowercase());
        let limit = req.limit.unwrap_or(100).max(1) as usize;
        let all_entries = record_value
            .get("entries")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut entries = Vec::new();
        for entry in &all_entries {
            if entries.len() >= limit {
                break;
            }
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            if let Some(ref needle) = filter {
                if !name.to_ascii_lowercase().contains(needle) {
                    continue;
                }
            }
            let mut conditions = Vec::new();
            for list in entry
                .get("conditionsList")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                for cond in list.get("conditions").and_then(|v| v.as_array()).into_iter().flatten() {
                    conditions.push(summarize_tagged_condition(cond, &tag_names));
                }
            }
            let modifiers: Vec<String> = entry
                .get("modifiers")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .map(summarize_typed_modifier)
                .collect();
            entries.push(serde_json::json!({
                "name": name,
                "when": conditions.join(" AND "),
                "modifiers": modifiers,
            }));
        }
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "record_name": record_name,
            "record_id": format!("{}", record.id),
            "entry_count": all_entries.len(),
            "returned": entries.len(),
            "entries": entries,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Compile a local decompiled UI canvas to canonical IR and return matching nodes with layout rects, draw rects, text bounds, style tags, and resolved color/tint tokens. This is the MCP-native form of query_ui_layout for UI matching.")]
    fn ui_ir_query(&self, Parameters(req): Parameters<UiIrQueryRequest>) -> String {
        let canvas_fetcher = match P4kCanvasFetcher::new(&self.data()) {
            Ok(f) => f,
            Err(e) => return mcp_error_json("canvas_fetcher_init_failed", e),
        };
        // Note: the pipeline re-fetches the canvas via canvas_fetcher internally,
        // so we don't need to keep the local `canvas` variable here.
        let _canvas = match canvas_fetcher.fetch_canvas_json(&req.canvas_guid) {
            Ok(c) => c,
            Err(e) => return mcp_error_json("canvas_not_found", format!("{e}")),
        };
        let style_fetcher = match P4kStyleFetcher::new(&self.data()) {
            Ok(f) => f,
            Err(e) => return mcp_error_json("style_fetcher_init_failed", e),
        };
        let swf_fetcher = P4kSwfFetcher::new(&self.data());
        let asset_fetcher = P4kAssetFetcher::new(&self.data());
        let manufacturer = req.manufacturer.unwrap_or_else(|| "drak".to_string());
        let helper = req.helper.unwrap_or_else(|| "mcp-ui-query".to_string());
        let width = req.width.unwrap_or(1920);
        let height = req.height.unwrap_or(1080);
        let content_guid = req.content_guid.unwrap_or_else(|| req.canvas_guid.clone());
        let binding = starbreaker_ui::UiBindingView {
            canvas_guid: Some(req.canvas_guid.as_str()),
            content_canvas_guid: Some(content_guid.as_str()),
            binding_kind: Some("mfd"),
            manufacturer_id: Some(manufacturer.as_str()),
            helper_name: Some(helper.as_str()),
            default_view_index: None,
            default_screen_slot: None,
            screen_name_loc_key: None,
            transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
        };
        let inputs = starbreaker_ui::PipelineInputs {
            binding: &binding,
            canvas_fetcher: &canvas_fetcher,
            swf_fetcher: &swf_fetcher,
            style_fetcher: &style_fetcher,
            asset_fetcher: &asset_fetcher,
            target_size: (width, height),
            apply_postprocess: false,
            animation_sample_percent: None,
            localization_map: None,
            loc_fetcher: None,
            derived_values: None,
            hologram_fetcher: None,
        };
        let ir = match starbreaker_ui::compile_ir_for_binding(&inputs) {
            Ok(ir) => ir,
            Err(e) => return mcp_error_json("ir_compile_failed", format!("{e}")),
        };

        let query = req.query.to_ascii_lowercase();
        let limit = req.limit.unwrap_or(80).max(1) as usize;
        let mut nodes = Vec::new();
        for node in &ir.nodes {
            let haystack = format!("{} {}", node.name, node.node_type).to_ascii_lowercase();
            if !haystack.contains(&query) {
                continue;
            }
            if nodes.len() >= limit {
                break;
            }
            let draw_rect = starbreaker_ui::ir_compose::debug_node_draw_rect(node, &ir);
            let text_rects = starbreaker_ui::ir_compose::debug_text_rects(node).map(|rects| {
                serde_json::json!({
                    "primary": bb_rect_json(rects.primary),
                    "primary_text_origin": rects.primary_text_origin,
                    "secondary": rects.secondary.map(bb_rect_json),
                    "secondary_text_origin": rects.secondary_text_origin,
                })
            });
            let text_bounds = starbreaker_ui::ir_compose::debug_text_drawn_bounds(node).map(|bounds| {
                serde_json::json!({
                    "primary": bb_rect_json(bounds.primary),
                    "secondary": bounds.secondary.map(bb_rect_json),
                })
            });
            nodes.push(serde_json::json!({
                "id": node.id,
                "parent": node.parent_id,
                "children": node.children,
                "name": node.name,
                "type": node.node_type,
                "is_active": node.is_active,
                "alpha": node.alpha,
                "computed_rect": ui_rect_json(node.computed_rect),
                "draw_rect": bb_rect_json(draw_rect),
                "background_fill_colour": node.background_fill_colour,
                "background_fill_colour_token": node.background_fill_colour_token,
                "stroke_colour": node.stroke_colour,
                "stroke_colour_token": node.stroke_colour_token,
                "icon_tint_colour": node.icon_tint_colour,
                "icon_tint_colour_token": node.icon_tint_colour_token,
                "text_payload": node.text_payload,
                "text_colour": node.text_style.as_ref().and_then(|style| style.colour),
                "text_colour_token": node.text_style.as_ref().and_then(|style| style.colour_token.clone()),
                "text_style": node.text_style,
                "text_rects": text_rects,
                "text_drawn_bounds": text_bounds,
                "asset_ref": node.asset_ref,
                "custom_shape": node.custom_shape,
                "style_tag_uuids": node.style_tag_uuids,
                "resolved_style_tags": node.resolved_style_tags,
            }));
        }

        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "canvas_guid": req.canvas_guid,
            "content_guid": content_guid,
            "manufacturer": manufacturer,
            "target_size": { "width": width, "height": height },
            "selected_style_source": ir.selected_style_source,
            "renderer_hint": ir.renderer_hint,
            "query": req.query,
            "matched_node_count": nodes.len(),
            "node_limit": limit,
            "nodes": nodes,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Switch StarBreaker MCP to a different Data.p4k for subsequent tools. The new archive and DataCore are loaded immediately; use p4k_data_status to confirm.")]
    fn p4k_set_data_path(&self, Parameters(req): Parameters<P4kSetDataPathRequest>) -> String {
        let path = Path::new(&req.path);
        if !path.is_file() {
            return format!("Data.p4k not found or not a file: {}", path.display());
        }
        match self.switch_p4k_path(path.to_path_buf()) {
            Ok(data) => serde_json::to_string_pretty(&serde_json::json!({
                "p4k_path": data.p4k_path,
                "entries": data.p4k.entries().len(),
                "datacore_bytes": data.dcb_bytes.len(),
            }))
            .unwrap_or_else(|e| format!("JSON error: {e}")),
            Err(e) => format!("Failed to switch Data.p4k: {e}"),
        }
    }

    #[tool(description = "Search DataCore for entity records by name substring. Returns JSON array of matches sorted by name length (best match first).")]
    fn search_entities(&self, Parameters(req): Parameters<SearchEntitiesRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let limit = req.limit.unwrap_or(20) as usize;
        let search = req.query.to_lowercase();

        let entity_si = match db.struct_id("EntityClassDefinition") {
            Some(si) => si,
            None => return "[]".to_string(),
        };

        let mut results: Vec<_> = db
            .records_of_type(entity_si)
            .filter(|r| {
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        results.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        results.truncate(limit);

        let json: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let name = db.resolve_string2(r.name_offset);
                let struct_type = db.resolve_string2(db.struct_def(r.struct_index).name_offset);
                let path = db.resolve_string(r.file_name_offset);
                serde_json::json!({
                    "name": format!("{struct_type}.{name}"),
                    "id": format!("{}", r.id),
                    "struct_type": struct_type,
                    "path": path,
                })
            })
            .collect();

        serde_json::to_string_pretty(&json).unwrap_or_else(|_| "[]".to_string())
    }

    #[tool(description = "Dump the resolved loadout tree for an entity. This is PROCESSED output from StarBreaker's loadout resolver — it resolves entityClassName references and queries geometry paths. For raw DataCore data, use datacore_query with path 'Components[SEntityComponentDefaultLoadoutParams]' instead.")]
    fn entity_loadout(&self, Parameters(req): Parameters<EntityLoadoutRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let record = match self.find_entity(&db, &req.name) {
            Some(r) => r,
            None => return format!("No entity found matching '{}'", req.name),
        };

        let idx = starbreaker_datacore::loadout::EntityIndex::new(&db);
        let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);

        let mut out = String::new();
        format_loadout_node(&tree.root, 0, &mut out);
        out
    }

    #[tool(description = "Dump a full DataCore record as pretty-printed JSON. Accepts a GUID or a name substring (uses shortest match).")]
    fn datacore_record(&self, Parameters(req): Parameters<DatacoreRecordRequest>) -> String {
        let data = self.data();
        let db = &data.db;

        let record = match self.find_record(&db, &req.id) {
            Some(r) => r,
            None => return format!("No record found for '{}'", req.id),
        };

        match starbreaker_datacore::export::to_json(&db, record) {
            Ok(bytes) => String::from_utf8(bytes)
                .unwrap_or_else(|_| "Error: invalid UTF-8 in JSON output".to_string()),
            Err(e) => format!("Error materializing record: {e}"),
        }
    }

    #[tool(description = "Query a specific property path on a DataCore record. Returns JSON at that path using bounded no-reference materialization, so broad object/array queries fail fast instead of expanding the whole graph. Example paths: 'Components[VehicleComponentParams].vehicleDefinition', 'Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path'")]
    fn datacore_query(&self, Parameters(req): Parameters<DatacoreQueryRequest>) -> String {
        let data = self.data();
        let db = &data.db;

        let record = match self.find_record(&db, &req.id) {
            Some(r) => r,
            None => return format!("No record found for '{}'", req.id),
        };

        let compiled = match db.compile_path::<starbreaker_datacore::query::value::Value>(
            record.struct_id(),
            &req.path,
        ) {
            Ok(c) => c,
            Err(e) => return format!("Invalid path '{}': {e}", req.path),
        };

        match db.query_no_references(&compiled, record) {
            Ok(results) => {
                if results.len() > MAX_DATACORE_QUERY_RESULTS {
                    return format!(
                        "Query result too large: {} top-level values matched (limit {}). Narrow the path before retrying.",
                        results.len(),
                        MAX_DATACORE_QUERY_RESULTS
                    );
                }
                let mut budget = JsonBudget::new();
                let json_values: Result<Vec<serde_json::Value>, String> = results
                    .iter()
                    .map(|value| value_to_json_limited(value, 0, &mut budget))
                    .collect();
                let json_values = match json_values {
                    Ok(values) => values,
                    Err(err) => return err,
                };
                if json_values.len() == 1 {
                    serde_json::to_string_pretty(&json_values[0])
                        .unwrap_or_else(|e| format!("JSON error: {e}"))
                } else {
                    serde_json::to_string_pretty(&json_values)
                        .unwrap_or_else(|e| format!("JSON error: {e}"))
                }
            }
            Err(e) => format!("Query error: {e}"),
        }
    }

    #[tool(description = "Read a file from the P4k archive. CryXML files (.xml, .mtl, .chrparams, .cdf) are auto-decoded to XML. Text files returned as-is. Binary files as base64. Also supports reading files inside a .socpak archive via 'outer/path.socpak::inner/path.entxml'.")]
    fn p4k_read(&self, Parameters(req): Parameters<P4kReadRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };
        Self::decode_archive_entry_bytes(&req.path, &data)
    }

    #[tool(description = "List files and directories under a P4k path. Shows name, compressed/uncompressed size, compression method, and encryption state for each file.")]
    fn p4k_list(&self, Parameters(req): Parameters<P4kListRequest>) -> String {
        let path = if req.path.is_empty() {
            String::new()
        } else {
            Self::normalize_p4k_path(&req.path).trim_end_matches('\\').to_string()
        };

        let data = self.data();
        let entries = data.p4k.list_dir(&path);
        if entries.is_empty() {
            return format!("No entries found under '{path}'");
        }

        let mut out = String::new();
        use std::fmt::Write;
        for entry in &entries {
            match entry {
                starbreaker_p4k::DirEntry::Directory(name) => {
                    let _ = writeln!(out, "  {name}/");
                }
                starbreaker_p4k::DirEntry::File(e) => {
                    let method = match e.compression_method {
                        0 => "store",
                        8 => "deflate",
                        100 => "zstd",
                        _ => "unknown",
                    };
                    let enc = if e.is_encrypted { " [encrypted]" } else { "" };
                    let ratio = if e.uncompressed_size > 0 {
                        format!("{:.0}%", e.compressed_size as f64 / e.uncompressed_size as f64 * 100.0)
                    } else {
                        "-".to_string()
                    };
                    let name = e.name.rsplit('\\').next().unwrap_or(&e.name);
                    let _ = writeln!(
                        out,
                        "  {name}  {}/{} ({ratio}, {method}){enc}",
                        format_size(e.compressed_size),
                        format_size(e.uncompressed_size),
                    );
                }
            }
        }
        let _ = writeln!(out, "\n{} entries", entries.len());
        out
    }

    #[tool(description = "Search all DataCore records by name substring. Unlike search_entities which only searches EntityClassDefinition records, this searches ALL record types. Optionally filter by struct type.")]
    fn search_records(&self, Parameters(req): Parameters<SearchRecordsRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let limit = req.limit.unwrap_or(20) as usize;
        let search = req.query.to_lowercase();

        let type_filter = req.struct_type.as_deref().map(|s| s.to_lowercase());

        let mut results: Vec<_> = db
            .records()
            .iter()
            .filter(|r| {
                if let Some(ref tf) = type_filter {
                    let st = db.resolve_string2(db.struct_def(r.struct_index).name_offset).to_lowercase();
                    if !st.contains(tf.as_str()) {
                        return false;
                    }
                }
                db.resolve_string2(r.name_offset)
                    .to_lowercase()
                    .contains(&search)
            })
            .collect();
        results.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
        results.truncate(limit);

        let json: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let name = db.resolve_string2(r.name_offset);
                let struct_type = db.resolve_string2(db.struct_def(r.struct_index).name_offset);
                let path = db.resolve_string(r.file_name_offset);
                serde_json::json!({
                    "name": format!("{struct_type}.{name}"),
                    "id": format!("{}", r.id),
                    "struct_type": struct_type,
                    "path": path,
                })
            })
            .collect();

        serde_json::to_string_pretty(&json).unwrap_or_else(|_| "[]".to_string())
    }

    #[tool(description = "Preview an image from the P4k archive. Supports DDS (with mip selection), PNG, JPG, and other formats. Returns the image for visual inspection. For DDS files, .tif extension is auto-converted to .dds.")]
    fn image_preview(&self, Parameters(req): Parameters<ImagePreviewRequest>) -> rmcp::model::Content {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return content_text(e),
        };

        let lower = req.path.to_lowercase();
        let is_dds = lower.ends_with(".dds") || lower.ends_with(".tif");

        let png_buf = if is_dds {
            let p4k_path = Self::normalize_p4k_path(&req.path);
            let p4k_clone = self.data().p4k.clone();
            let sibling = P4kSiblingReader { p4k: p4k_clone, base_path: p4k_path };
            let dds = match starbreaker_dds::DdsFile::from_split(&data, &sibling) {
                Ok(d) => d,
                Err(e) => return content_text(format!("DDS decode error: {e}")),
            };

            if dds.mip_count() == 0 {
                return content_text("DDS has no mip data");
            }

            let mip = req.mip.unwrap_or(0).min(dds.mip_count() as u32 - 1) as usize;
            let (w, h) = dds.dimensions(mip);

            let rgba = match dds.decode_rgba(mip) {
                Ok(r) => r,
                Err(e) => return content_text(format!("DDS decode error: {e}")),
            };

            let mut png_buf = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
            if let Err(e) = image::ImageEncoder::write_image(encoder, &rgba, w, h, image::ExtendedColorType::Rgba8) {
                return content_text(format!("PNG encode error: {e}"));
            }

            log::info!("DDS: {}x{}, mip {}/{}, cubemap={}", w, h, mip, dds.mip_count(), dds.is_cubemap());
            png_buf
        } else {
            match image::load_from_memory(&data) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    let mut png_buf = Vec::new();
                    let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
                    if let Err(e) = image::ImageEncoder::write_image(encoder, &rgba, w, h, image::ExtendedColorType::Rgba8) {
                        return content_text(format!("PNG encode error: {e}"));
                    }
                    png_buf
                }
                Err(e) => return content_text(format!("Image decode error: {e}")),
            }
        };

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
        // Return as image content — Claude can see this directly
        content_image(b64, "image/png")
    }

    #[tool(description = "List all chunks in a CryEngine chunk file (IVO or CrCh format). Shows chunk type, name, version, offset, and size. For IVO files with a NodeMeshCombos chunk, also shows NMC node summary (names + parent indices).")]
    fn chunk_list(&self, Parameters(req): Parameters<ChunkListRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let chunk_file = match starbreaker_chunks::ChunkFile::from_bytes(&data) {
            Ok(cf) => cf,
            Err(e) => return format!("Chunk file parse error: {e}"),
        };

        let mut out = String::new();
        use std::fmt::Write;

        match &chunk_file {
            starbreaker_chunks::ChunkFile::Ivo(ivo) => {
                let _ = writeln!(out, "Format: IVO (#ivo), {} chunks\n", ivo.chunks().len());
                let _ = writeln!(out, "{:<4} {:<12} {:>8} {:>10} {:>10}", "Idx", "Type", "Version", "Offset", "Size");
                let _ = writeln!(out, "{}", "-".repeat(50));
                for (i, chunk) in ivo.chunks().iter().enumerate() {
                    let name = starbreaker_chunks::known_types::ivo::name(chunk.chunk_type)
                        .unwrap_or("Unknown");
                    let _ = writeln!(out, "{:<4} {:<12} {:>8} {:>#10x} {:>10}",
                        i, name, chunk.version, chunk.offset, chunk.size);
                }

                // NMC summary if present
                if let Some(nmc_chunk) = ivo.chunks().iter().find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::NODE_MESH_COMBOS) {
                    let nmc_data = ivo.chunk_data(nmc_chunk);
                    // Try parsing NMC — use the full file data since parse_nmc_full expects the whole file
                    if let Some((nodes, _mat_indices)) = starbreaker_3d::nmc::parse_nmc_full(&data) {
                        let _ = writeln!(out, "\nNMC Nodes ({}):", nodes.len());
                        for (i, node) in nodes.iter().enumerate() {
                            let parent = node.parent_index.map(|p| format!("{p}")).unwrap_or_else(|| "root".to_string());
                            let _ = writeln!(out, "  [{i}] {:<30} parent={:<5} type={}", node.name, parent, node.geometry_type);
                        }
                    } else {
                        let _ = writeln!(out, "\nNMC chunk present ({} bytes) but could not parse", nmc_data.len());
                    }
                }
            }
            starbreaker_chunks::ChunkFile::CrCh(crch) => {
                let _ = writeln!(out, "Format: CrCh, {} chunks\n", crch.chunks().len());
                let _ = writeln!(out, "{:<4} {:<18} {:>4} {:>8} {:>10} {:>10} {}", "Idx", "Type", "ID", "Version", "Offset", "Size", "Endian");
                let _ = writeln!(out, "{}", "-".repeat(70));
                for (i, chunk) in crch.chunks().iter().enumerate() {
                    let name = starbreaker_chunks::known_types::crch::name(chunk.chunk_type)
                        .unwrap_or("Unknown");
                    let endian = if chunk.big_endian { "BE" } else { "LE" };
                    let _ = writeln!(out, "{:<4} {:<18} {:>4} {:>8} {:>#10x} {:>10} {}",
                        i, name, chunk.id, chunk.version, chunk.offset, chunk.size, endian);
                }
            }
        }

        out
    }

    #[tool(description = "Read raw bytes from specific chunk(s) in a CryEngine chunk file. Returns hex dump. Use chunk_list first to find chunk indices.")]
    fn chunk_read(&self, Parameters(req): Parameters<ChunkReadRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let chunk_file = match starbreaker_chunks::ChunkFile::from_bytes(&data) {
            Ok(cf) => cf,
            Err(e) => return format!("Chunk file parse error: {e}"),
        };

        let max_bytes = req.max_bytes.unwrap_or(256) as usize;
        let mut out = String::new();
        use std::fmt::Write;

        match &chunk_file {
            starbreaker_chunks::ChunkFile::Ivo(ivo) => {
                let chunks: Vec<usize> = if let Some(idx) = req.index {
                    vec![idx as usize]
                } else {
                    (0..ivo.chunks().len()).collect()
                };
                for idx in chunks {
                    let Some(chunk) = ivo.chunks().get(idx) else {
                        let _ = writeln!(out, "Chunk index {idx} out of range (max {})", ivo.chunks().len() - 1);
                        continue;
                    };
                    let name = starbreaker_chunks::known_types::ivo::name(chunk.chunk_type).unwrap_or("Unknown");
                    let chunk_data = ivo.chunk_data(chunk);
                    let show = chunk_data.len().min(max_bytes);
                    let _ = writeln!(out, "--- Chunk [{idx}] {name} ({} bytes) ---", chunk_data.len());
                    format_hex(&chunk_data[..show], &mut out);
                    if show < chunk_data.len() {
                        let _ = writeln!(out, "  ... ({} more bytes)", chunk_data.len() - show);
                    }
                    let _ = writeln!(out);
                }
            }
            starbreaker_chunks::ChunkFile::CrCh(crch) => {
                let chunks: Vec<usize> = if let Some(idx) = req.index {
                    vec![idx as usize]
                } else {
                    (0..crch.chunks().len()).collect()
                };
                for idx in chunks {
                    let Some(chunk) = crch.chunks().get(idx) else {
                        let _ = writeln!(out, "Chunk index {idx} out of range (max {})", crch.chunks().len() - 1);
                        continue;
                    };
                    let name = starbreaker_chunks::known_types::crch::name(chunk.chunk_type).unwrap_or("Unknown");
                    let chunk_data = crch.chunk_data(chunk);
                    let show = chunk_data.len().min(max_bytes);
                    let _ = writeln!(out, "--- Chunk [{idx}] {name} id={} ({} bytes) ---", chunk.id, chunk_data.len());
                    format_hex(&chunk_data[..show], &mut out);
                    if show < chunk_data.len() {
                        let _ = writeln!(out, "  ... ({} more bytes)", chunk_data.len() - show);
                    }
                    let _ = writeln!(out);
                }
            }
        }

        out
    }

    #[tool(description = "Search P4k archive file paths by substring. Returns matching paths with file sizes. Useful for finding files when you don't know the exact directory.")]
    fn p4k_search(&self, Parameters(req): Parameters<P4kSearchRequest>) -> String {
        let limit = req.limit.unwrap_or(50) as usize;
        let query = req.query.to_lowercase();

        let data = self.data();
        let mut results: Vec<_> = data
            .p4k
            .entries()
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&query))
            .collect();

        results.sort_by_key(|e| e.name.len());
        results.truncate(limit);

        if results.is_empty() {
            return format!("No P4k files matching '{}'", req.query);
        }

        let mut out = String::new();
        use std::fmt::Write;
        for e in &results {
            let _ = writeln!(out, "{}  ({})", e.name, format_size(e.uncompressed_size));
        }
        let _ = writeln!(out, "\n{} results", results.len());
        out
    }

    #[tool(description = "Inspect a .socpak file from P4k. Returns nested entry names and per-.soc chunk summaries, including raw IncludedObjects object-type counts so authored object variants inside the container can be identified.")]
    fn socpak_inspect(&self, Parameters(req): Parameters<SocpakInspectRequest>) -> String {
        let bytes = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let archive = match P4kArchive::from_bytes(&bytes) {
            Ok(a) => a,
            Err(e) => return format!("Failed to parse socpak ZIP '{}': {e}", req.path),
        };

        let entries: Vec<serde_json::Value> = archive
            .entries()
            .iter()
            .map(|entry| serde_json::json!({ "name": entry.name }))
            .collect();

        let soc_files: Vec<serde_json::Value> = archive
            .entries()
            .iter()
            .filter(|entry| entry.name.to_ascii_lowercase().ends_with(".soc"))
            .map(|entry| {
                let name = entry.name.clone();
                let soc_bytes = match archive.read(entry) {
                    Ok(data) => data,
                    Err(e) => {
                        return serde_json::json!({
                            "name": name,
                            "error": format!("Failed to read inner .soc: {e}"),
                        });
                    }
                };

                let chunk_file = match starbreaker_chunks::ChunkFile::from_bytes(&soc_bytes) {
                    Ok(cf) => cf,
                    Err(e) => {
                        return serde_json::json!({
                            "name": name,
                            "error": format!("Chunk parse failed: {e}"),
                        });
                    }
                };

                match &chunk_file {
                    starbreaker_chunks::ChunkFile::CrCh(crch) => {
                        let chunks: Vec<serde_json::Value> = crch
                            .chunks()
                            .iter()
                            .map(|chunk| {
                                serde_json::json!({
                                    "type": format!("0x{:04x}", chunk.chunk_type),
                                    "name": starbreaker_chunks::known_types::crch::name(chunk.chunk_type).unwrap_or("Unknown"),
                                    "id": chunk.id,
                                    "version": chunk.version,
                                    "offset": chunk.offset,
                                    "size": chunk.size,
                                })
                            })
                            .collect();

                        let included_objects: Vec<serde_json::Value> = crch
                            .chunks()
                            .iter()
                            .filter(|chunk| chunk.chunk_type == starbreaker_chunks::known_types::crch::INCLUDED_OBJECTS)
                            .map(|chunk| inspect_included_objects_chunk(crch.chunk_data(chunk)))
                            .collect();

                        serde_json::json!({
                            "name": name,
                            "format": "CrCh",
                            "chunk_count": crch.chunks().len(),
                            "chunks": chunks,
                            "included_objects": included_objects,
                        })
                    }
                    starbreaker_chunks::ChunkFile::Ivo(ivo) => {
                        let chunks: Vec<serde_json::Value> = ivo
                            .chunks()
                            .iter()
                            .map(|chunk| {
                                serde_json::json!({
                                    "type": format!("0x{:08x}", chunk.chunk_type),
                                    "name": starbreaker_chunks::known_types::ivo::name(chunk.chunk_type).unwrap_or("Unknown"),
                                    "version": chunk.version,
                                    "offset": chunk.offset,
                                    "size": chunk.size,
                                })
                            })
                            .collect();

                        serde_json::json!({
                            "name": name,
                            "format": "IVO",
                            "chunk_count": ivo.chunks().len(),
                            "chunks": chunks,
                            "included_objects": [],
                        })
                    }
                }
            })
            .collect();

        serde_json::to_string_pretty(&serde_json::json!({
            "path": Self::normalize_p4k_path(&req.path),
            "entry_count": archive.entries().len(),
            "entries": entries,
            "soc_files": soc_files,
        }))
        .unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Read a file from inside a .socpak archive. CryXML sidecars (.xml, .entxml, .mtl, .cdf, etc.) are auto-decoded like p4k_read; text entries are returned as text and binary entries as base64.")]
    fn socpak_read_entry(&self, Parameters(req): Parameters<SocpakReadEntryRequest>) -> String {
        let bytes = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let archive = match P4kArchive::from_bytes(&bytes) {
            Ok(a) => a,
            Err(e) => return format!("Failed to parse socpak ZIP '{}': {e}", req.path),
        };

        let requested = req.entry.replace('/', "\\").to_ascii_lowercase();
        let Some(entry) = archive
            .entries()
            .iter()
            .find(|entry| entry.name.replace('/', "\\").to_ascii_lowercase() == requested)
        else {
            return format!(
                "Entry not found in socpak '{}': {}",
                req.path,
                req.entry
            );
        };

        let entry_bytes = match archive.read(entry) {
            Ok(data) => data,
            Err(e) => return format!("Failed to read '{}' from '{}': {e}", req.entry, req.path),
        };

        Self::decode_archive_entry_bytes(&entry.name, &entry_bytes)
    }

    #[tool(description = "Summarize a .mtl material file from P4k. Shows each sub-material's index, name, shader, key flags (DECAL, STENCIL, POM, opacity, alpha_test), and texture slots. Much more compact than reading the raw MTL XML.")]
    fn mtl_summary(&self, Parameters(req): Parameters<MtlSummaryRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let xml = match starbreaker_cryxml::from_bytes(&data) {
            Ok(x) => x,
            Err(e) => return format!("Failed to parse MTL as CryXML: {e}"),
        };

        let root = xml.root();

        // Collect sub-material nodes. If there's a <SubMaterials> container, iterate its
        // children; otherwise treat the root as a single material.
        let mat_nodes: Vec<_> = if let Some(sub_node) = xml.node_children(root)
            .find(|c| xml.node_tag(c) == "SubMaterials")
        {
            xml.node_children(sub_node)
                .filter(|c| xml.node_tag(c) == "Material")
                .collect()
        } else {
            vec![root]
        };

        let mut out = String::new();
        use std::fmt::Write;
        let _ = writeln!(out, "{} sub-materials in {}\n", mat_nodes.len(), req.path);
        let _ = writeln!(out, "{:>3}  {:<40} {:<15} {}", "Idx", "Name", "Shader", "Flags / Textures");
        let _ = writeln!(out, "{}", "-".repeat(100));

        for (i, mat_node) in mat_nodes.iter().enumerate() {
            let attrs: std::collections::HashMap<&str, &str> =
                xml.node_attributes(mat_node).collect();

            let name = attrs.get("Name").copied().unwrap_or("--");
            let shader = attrs.get("Shader").copied().unwrap_or("--");
            let mask = attrs.get("StringGenMask").copied().unwrap_or("");
            let opacity: f32 = attrs.get("Opacity").and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let alpha_test: f32 = attrs.get("AlphaTest").and_then(|v| v.parse().ok()).unwrap_or(0.0);

            // Collect flags
            let mut flags = Vec::new();
            if mask.contains("%DECAL") { flags.push("DECAL".to_string()); }
            if mask.contains("STENCIL_MAP") { flags.push("STENCIL".to_string()); }
            if mask.contains("%PARALLAX_OCCLUSION_MAPPING") { flags.push("POM".to_string()); }
            if mask.contains("%VERTCOLORS") { flags.push("VCOL".to_string()); }
            if mask.contains("%WEAR_LAYER") { flags.push("WEAR".to_string()); }
            if mask.contains("%BLENDLAYER") { flags.push("BLEND".to_string()); }
            if opacity < 1.0 { flags.push(format!("opacity={opacity}")); }
            if alpha_test > 0.0 { flags.push(format!("alpha_test={alpha_test}")); }

            // Collect texture slots
            let mut tex_slots = Vec::new();
            if let Some(tex_node) = xml.node_children(mat_node)
                .find(|c| xml.node_tag(c) == "Textures")
            {
                for tex in xml.node_children(tex_node) {
                    if xml.node_tag(tex) != "Texture" { continue; }
                    let tex_attrs: std::collections::HashMap<&str, &str> =
                        xml.node_attributes(tex).collect();
                    let slot = tex_attrs.get("Map").copied().unwrap_or("?");
                    let file = tex_attrs.get("File").copied().unwrap_or("?");
                    // Show just the filename, not full path
                    let short = file.rsplit(['/', '\\']).next().unwrap_or(file);
                    tex_slots.push(format!("{slot}={short}"));
                }
            }

            // Count layers
            let layer_count = xml.node_children(mat_node)
                .find(|c| xml.node_tag(c) == "MatLayers")
                .map(|l| xml.node_children(l).filter(|c| xml.node_tag(c) == "Layer").count())
                .unwrap_or(0);
            if layer_count > 0 {
                flags.push(format!("{layer_count} layers"));
            }

            // Check palette tint on first layer
            if let Some(layers_node) = xml.node_children(mat_node)
                .find(|c| xml.node_tag(c) == "MatLayers")
            {
                if let Some(first_layer) = xml.node_children(layers_node)
                    .find(|c| xml.node_tag(c) == "Layer")
                {
                    let layer_attrs: std::collections::HashMap<&str, &str> =
                        xml.node_attributes(first_layer).collect();
                    if let Some(pt) = layer_attrs.get("PaletteTint") {
                        let pt_val: u8 = pt.parse().unwrap_or(0);
                        if pt_val > 0 {
                            let ch = match pt_val { 1 => "A", 2 => "B", 3 => "C", _ => "?" };
                            flags.push(format!("palette={ch}"));
                        }
                    }
                }
            }

            let flag_str = if flags.is_empty() { "-".to_string() } else { flags.join(", ") };

            let _ = writeln!(out, "{i:3}  {name:<40} {shader:<15} {flag_str}");
            for tex in &tex_slots {
                let _ = writeln!(out, "       {tex}");
            }
        }

        out
    }

    #[tool(description = "Inspect a CryEngine animation database (.dba) or compressed animation file (.caf). Returns structured JSON: clip metadata (name, fps, frame_count, channel_count) and per-channel bone hashes plus first/last keyframe samples (or full keyframe arrays with all_keyframes=true). Provide `skeleton` to resolve bone hashes to names — accepts either a CHR skeleton (CompiledBones) or a CGA/CGAM scene graph (NMC nodes) for ships whose main body has no CHR; combine with `bone_filter` to drill down to a small set of channels (e.g. wings, landing gear). Use `filter` for clip-name filtering. Replaces the legacy `starbreaker dba dump` CLI.")]
    fn dba_dump(&self, Parameters(req): Parameters<DbaDumpRequest>) -> String {
        let bytes = match self.read_p4k_or_disk(&req.path) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let db = if req.path.to_ascii_lowercase().ends_with(".caf") {
            match starbreaker_3d::animation::parse_caf(&bytes) {
                Ok(db) => db,
                Err(e) => return format!("parse_caf failed for {}: {e}", req.path),
            }
        } else {
            match starbreaker_3d::animation::parse_dba(&bytes) {
                Ok(db) => db,
                Err(e) => return format!("parse_dba failed for {}: {e}", req.path),
            }
        };
        let mut hash_to_name: std::collections::HashMap<u32, String> =
            std::collections::HashMap::new();
        if let Some(skel_path) = req.skeleton.as_ref() {
            match self.read_p4k_or_disk(skel_path) {
                Ok(sb) => {
                    if let Some(names) = starbreaker_3d::skeleton::parse_rig_node_names(&sb) {
                        for name in &names {
                            hash_to_name.insert(
                                starbreaker_3d::animation::bone_name_hash(name),
                                name.clone(),
                            );
                        }
                    } else {
                        return format!(
                            "Skeleton '{skel_path}' has no CompiledBones (CHR) or NMC (CGA/CGAM) chunk; cannot resolve bone names"
                        );
                    }
                }
                Err(e) => return format!("Failed to read skeleton '{skel_path}': {e}"),
            }
        }
        let value = starbreaker_3d::animation::dump_database_to_json(
            &db,
            &hash_to_name,
            req.filter.as_deref(),
            req.bone_filter.as_deref(),
            req.all_keyframes.unwrap_or(false),
        );
        match serde_json::to_string_pretty(&value) {
            Ok(s) => s,
            Err(e) => format!("serialize failed: {e}"),
        }
    }

    #[tool(description = "Report-only animation binding diagnostic for debugging why exported animation clips fail to bind in Blender. Scans a local folder of exported animation JSON sidecars (animation_folder, e.g. '<package>/animations') and cross-references every CAF/DBA track hash against node names recovered from source rigs: .cga/.cgam NMC nodes, .meshsetup joints, .chrparams references, and an optional .glb. Source files accept P4k paths (Data\\ prefix optional) OR absolute filesystem paths; CryXMLB .meshsetup/.chrparams are auto-decoded. Returns JSON: per-clip source_node_name coverage, the list of unresolved track hashes, and per-hash candidate name matches so you can see which rig owns each hash. Never mutates sidecars or guesses bindings. Set markdown=true for a readable summary. Typical use: set cga to a component .cga (e.g. a door) to discover which rig owns an unresolved track, or to the exterior hull .cga to confirm which tracks the hull does NOT own.")]
    fn animation_binding_report(
        &self,
        Parameters(req): Parameters<AnimationBindingReportRequest>,
    ) -> String {
        use starbreaker_3d::animation::binding_report as report;

        let mut names = report::NameSources::default();
        let mut source_reports = Vec::<serde_json::Value>::new();

        // .cga / .cgam: raw bytes parsed directly for NMC/rig node names.
        for (path, label) in [(req.cga.as_deref(), "cga"), (req.cgam.as_deref(), "cgam")] {
            if let Some(path) = path {
                match self.read_p4k_or_disk(path) {
                    Ok(data) => {
                        source_reports.push(report::rig_source_report(&data, label, path, &mut names))
                    }
                    Err(e) => return format!("Failed to read {label} '{path}': {e}"),
                }
            }
        }
        // .meshsetup / .chrparams: CryXMLB-decoded to text where needed.
        if let Some(path) = req.meshsetup.as_deref() {
            match self.read_p4k_or_disk(path) {
                Ok(data) => source_reports.push(report::meshsetup_source_report(
                    &decode_text_or_cryxml(&data),
                    path,
                    &mut names,
                )),
                Err(e) => return format!("Failed to read meshsetup '{path}': {e}"),
            }
        }
        if let Some(path) = req.chrparams.as_deref() {
            match self.read_p4k_or_disk(path) {
                Ok(data) => source_reports.push(report::chrparams_source_report(
                    &decode_text_or_cryxml(&data),
                    path,
                    &mut names,
                )),
                Err(e) => return format!("Failed to read chrparams '{path}': {e}"),
            }
        }
        if let Some(path) = req.glb.as_deref() {
            match self.read_p4k_or_disk(path) {
                Ok(data) => match report::glb_source_report(&data, path, &mut names) {
                    Some(r) => source_reports.push(r),
                    None => return format!("Failed to parse GLB JSON chunk: {path}"),
                },
                Err(e) => return format!("Failed to read glb '{path}': {e}"),
            }
        }

        // Animation sidecars live on disk (exported package).
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        if let Some(explicit) = &req.animation_json {
            for p in explicit {
                let path = std::path::PathBuf::from(p);
                if !path.is_file() {
                    return format!("animation JSON file not found: {p}");
                }
                files.push(path);
            }
        }
        if let Some(folder) = req.animation_folder.as_deref() {
            let dir = std::path::Path::new(folder);
            if !dir.is_dir() {
                return format!("animation folder not found: {folder}");
            }
            if let Err(e) = collect_json_files_recursive(dir, &mut files) {
                return format!("Failed to scan '{folder}': {e}");
            }
        }
        files.sort();
        files.dedup();
        if files.is_empty() {
            return "no animation JSON files were supplied/found (set animation_folder and/or animation_json)".to_string();
        }

        let mut clip_stats = Vec::new();
        for file in &files {
            let data = match std::fs::read(file) {
                Ok(d) => d,
                Err(e) => return format!("Failed to read {}: {e}", file.display()),
            };
            let value: serde_json::Value = match serde_json::from_slice(&data) {
                Ok(v) => v,
                Err(e) => return format!("Failed to parse JSON {}: {e}", file.display()),
            };
            clip_stats.extend(report::clips_from_value(&file.display().to_string(), &value));
        }

        let report_value = report::build_report(&names, source_reports, files.len(), &clip_stats);
        if req.markdown.unwrap_or(false) {
            return report::build_markdown_report(&report_value);
        }
        match serde_json::to_string_pretty(&report_value) {
            Ok(s) => s,
            Err(e) => format!("serialize failed: {e}"),
        }
    }

    #[tool(description = "Dump the Mannequin Animation Database (ADB) plus its companion ControllerDef for an entity. Returns structured JSON listing every Mannequin Fragment with group name, GUID, tags, FragTags, BlendOutDuration, OptionWeight, animations, scopes (resolved from the ControllerDef), and procedurals. Use to inspect fragment-scope metadata for animation troubleshooting (Phase 37). Note: the ADB has no per-bone blend-mode flag; use dba_dump's `rot_format_flags`/`pos_format_flags` for per-bone CAF metadata.")]
    fn mannequin_dump(&self, Parameters(req): Parameters<MannequinDumpRequest>) -> String {
        let data = self.data();
        let db = &data.db;
        let record = match self.find_entity(&db, &req.entity) {
            Some(r) => r,
            None => return format!("No entity matching '{}'", req.entity),
        };
        let source = match starbreaker_3d::query_animation_controller_source(&db, record) {
            Some(s) => s,
            None => return format!("Entity '{}' has no SAnimationControllerParams", req.entity),
        };
        let value = match starbreaker_3d::animation::dump_mannequin_adb_to_json(
            data.p4k.as_ref(),
            &source,
            req.filter.as_deref(),
        ) {
            Ok(v) => v,
            Err(e) => return format!("dump_mannequin_adb_to_json failed: {e}"),
        };
        match serde_json::to_string_pretty(&value) {
            Ok(s) => s,
            Err(e) => format!("serialize failed: {e}"),
        }
    }

    #[tool(description = "Parse the DNA1/SDNA block from a .blend file and show struct field layouts with absolute byte offsets. Accepts an absolute filesystem path to a .blend file (zstd-compressed Blender 5.x or uncompressed). Optionally filter to a single struct name; otherwise returns a list of all struct names and sizes.")]
    fn blend_sdna(&self, Parameters(req): Parameters<BlendSdnaRequest>) -> String {
        use crate::blend_debug::{decompress_blend, parse_blend_blocks, parse_sdna, format_struct_layout};

        let raw = match std::fs::read(&req.path) {
            Ok(b) => b,
            Err(e) => return format!("failed to read {}: {e}", req.path),
        };
        let data = match decompress_blend(&raw) {
            Ok(d) => d,
            Err(e) => return format!("decompress failed: {e}"),
        };
        let blocks = match parse_blend_blocks(&data) {
            Ok(r) => r,
            Err(e) => return format!("block parse failed: {e}"),
        };
        let dna1 = match blocks.iter().find(|b| &b.code == b"DNA1") {
            Some(b) => b,
            None => return "No DNA1 block found in file".to_string(),
        };
        let dna_data = &data[dna1.data_offset..dna1.data_offset + dna1.data_len];
        let sdna = match parse_sdna(dna_data, 8) {
            Ok(s) => s,
            Err(e) => return format!("SDNA parse failed: {e}"),
        };
        let max_depth = req.max_depth.unwrap_or(1) as usize;

        if let Some(name) = &req.struct_name {
            match sdna.find_struct(name) {
                Some((idx, s)) => {
                    let mut out = format!(
                        "Struct #{idx}: {} ({} bytes, {} fields)\n",
                        s.name, s.size, s.fields.len()
                    );
                    format_struct_layout(&sdna, s, 0, 0, max_depth, &mut out);
                    out
                }
                None => {
                    let available: Vec<&str> = sdna.structs.iter()
                        .filter(|s| s.name.to_lowercase().contains(&name.to_lowercase()))
                        .map(|s| s.name.as_str())
                        .collect();
                    if available.is_empty() {
                        format!("Struct '{name}' not found. There are {} structs total.", sdna.structs.len())
                    } else {
                        format!("Struct '{name}' not found exactly. Similar names: {}", available.join(", "))
                    }
                }
            }
        } else {
            let mut out = format!(
                "{} structs, ptr_size=8\n\n",
                sdna.structs.len()
            );
            for (i, s) in sdna.structs.iter().enumerate() {
                out.push_str(&format!("  [{i:4}] {} ({} bytes, {} fields)\n", s.name, s.size, s.fields.len()));
            }
            out
        }
    }

    #[tool(description = "Hex dump specific blocks from a .blend file filtered by SDNA struct type. Accepts an absolute filesystem path. If sdna_type is omitted, returns a summary table of all block types with counts and sizes.")]
    fn blend_block_inspect(&self, Parameters(req): Parameters<BlendBlockInspectRequest>) -> String {
        use crate::blend_debug::{decompress_blend, parse_blend_blocks, parse_sdna, hex_dump};

        let raw = match std::fs::read(&req.path) {
            Ok(b) => b,
            Err(e) => return format!("failed to read {}: {e}", req.path),
        };
        let data = match decompress_blend(&raw) {
            Ok(d) => d,
            Err(e) => return format!("decompress failed: {e}"),
        };
        let blocks = match parse_blend_blocks(&data) {
            Ok(r) => r,
            Err(e) => return format!("block parse failed: {e}"),
        };
        let dna1 = blocks.iter().find(|b| &b.code == b"DNA1");
        let sdna = dna1.and_then(|b| {
            let dna_data = &data[b.data_offset..b.data_offset + b.data_len];
            parse_sdna(dna_data, 8).ok()
        });

        let max_bytes = req.max_bytes.unwrap_or(256) as usize;
        let limit = req.limit.unwrap_or(10) as usize;

        if let Some(filter) = &req.sdna_type {
            // Find the SDNA struct index for this type
            let target_sdna_idx = sdna.as_ref().and_then(|s| s.find_struct(filter).map(|(i, _)| i));

            let matching: Vec<_> = blocks.iter()
                .filter(|b| {
                    if let Some(idx) = target_sdna_idx {
                        b.sdna_index as usize == idx
                    } else {
                        // Fallback: no SDNA, can't filter
                        false
                    }
                })
                .collect();

            if matching.is_empty() {
                let all_types: std::collections::BTreeMap<String, usize> = {
                    let mut map = std::collections::BTreeMap::new();
                    for b in &blocks {
                        if let Some(ref s) = sdna {
                            if (b.sdna_index as usize) < s.structs.len() {
                                *map.entry(s.structs[b.sdna_index as usize].name.clone()).or_default() += 1;
                            }
                        }
                    }
                    map
                };
                return format!(
                    "No blocks with SDNA type '{}' found.\nAvailable types: {}\n",
                    filter,
                    all_types.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            }

            let mut out = format!(
                "{} block(s) with SDNA type '{}' (showing up to {limit}):\n\n",
                matching.len(), filter
            );
            for b in matching.iter().take(limit) {
                out.push_str(&format!(
                    "Block code={} old_ptr={:#018x} sdna={} count={} data_len={}\n",
                    b.code_str(), b.old_ptr, b.sdna_index, b.count, b.data_len
                ));
                if max_bytes > 0 && b.data_len > 0 {
                    let end = (b.data_offset + max_bytes.min(b.data_len)).min(data.len());
                    out.push_str(&hex_dump(&data[b.data_offset..end], b.data_offset));
                }
                out.push('\n');
            }
            out
        } else {
            // Summary of all block types
            let mut type_counts: std::collections::BTreeMap<String, (usize, usize)> =
                std::collections::BTreeMap::new();
            for b in &blocks {
                let type_name = if let Some(ref s) = sdna {
                    if (b.sdna_index as usize) < s.structs.len() {
                        s.structs[b.sdna_index as usize].name.clone()
                    } else {
                        b.code_str()
                    }
                } else {
                    b.code_str()
                };
                let e = type_counts.entry(type_name).or_default();
                e.0 += 1;
                e.1 += b.data_len;
            }
            let mut out = format!(
                "{} total blocks in {}:\n\n",
                blocks.len(), req.path
            );
            out.push_str(&format!("{:<40} {:>8} {:>14}\n", "Type", "Count", "Total bytes"));
            out.push_str(&format!("{}\n", "-".repeat(65)));
            for (name, (count, total_bytes)) in &type_counts {
                out.push_str(&format!("{:<40} {:>8} {:>14}\n", name, count, total_bytes));
            }
            out
        }
    }

    #[tool(description = "Run headless Blender with a before/after Python script pair, save .blend files, decompress them, and show a binary diff of changed bytes with offsets and SDNA struct names. Useful for reverse-engineering Blender binary format changes.")]
    fn blend_python_diff(&self, Parameters(req): Parameters<BlendPythonDiffRequest>) -> String {
        use crate::blend_debug::{decompress_blend, parse_blend_blocks, parse_sdna, hex_dump, find_blender_bin};
        use std::process::Command;

        let blender = match find_blender_bin(req.blender_bin.as_deref()) {
            Ok(b) => b,
            Err(e) => return format!("Blender not found: {e}"),
        };

        let tmp = std::env::temp_dir().join(format!(
            "blend_python_diff_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        if let Err(e) = std::fs::create_dir_all(&tmp) {
            return format!("failed to create temp dir: {e}");
        }

        let before_blend = tmp.join("before.blend");
        let after_blend = tmp.join("after.blend");
        let before_py = tmp.join("before.py");
        let after_py = tmp.join("after.py");

        let before_data: Vec<u8>;
        let after_data: Vec<u8>;

        // Run "before" script
        if !req.before_script.is_empty() {
            let script = format!(
                "import bpy\nbpy.ops.wm.open_mainfile(filepath=r'{}')\n{}\nbpy.ops.wm.save_as_mainfile(filepath=r'{}')\n",
                req.blend_path,
                req.before_script,
                before_blend.display()
            );
            if let Err(e) = std::fs::write(&before_py, &script) {
                return format!("failed to write before script: {e}");
            }
            let status = Command::new(&blender)
                .args(["--background", "--python"])
                .arg(&before_py)
                .output();
            match status {
                Ok(out) if !out.status.success() => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!(
                        "Before Blender run failed:\nstdout: {}\nstderr: {}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("Failed to run Blender: {e}");
                }
                _ => {}
            }
            before_data = match std::fs::read(&before_blend) {
                Ok(b) => b,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("before.blend not written: {e}");
                }
            };
        } else {
            before_data = match std::fs::read(&req.blend_path) {
                Ok(b) => b,
                Err(e) => return format!("failed to read input blend: {e}"),
            };
        }

        // Run "after" script
        {
            let input_path = if !req.before_script.is_empty() {
                before_blend.to_string_lossy().to_string()
            } else {
                req.blend_path.clone()
            };
            let script = format!(
                "import bpy\nbpy.ops.wm.open_mainfile(filepath=r'{input_path}')\n{}\nbpy.ops.wm.save_as_mainfile(filepath=r'{}')\n",
                req.after_script,
                after_blend.display()
            );
            if let Err(e) = std::fs::write(&after_py, &script) {
                return format!("failed to write after script: {e}");
            }
            let status = Command::new(&blender)
                .args(["--background", "--python"])
                .arg(&after_py)
                .output();
            match status {
                Ok(out) if !out.status.success() => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!(
                        "After Blender run failed:\nstdout: {}\nstderr: {}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("Failed to run Blender: {e}");
                }
                _ => {}
            }
            after_data = match std::fs::read(&after_blend) {
                Ok(b) => b,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return format!("after.blend not written: {e}");
                }
            };
        }

        // Decompress both
        let before_decomp = match decompress_blend(&before_data) {
            Ok(d) => d,
            Err(e) => { let _ = std::fs::remove_dir_all(&tmp); return format!("before decompress: {e}"); }
        };
        let after_decomp = match decompress_blend(&after_data) {
            Ok(d) => d,
            Err(e) => { let _ = std::fs::remove_dir_all(&tmp); return format!("after decompress: {e}"); }
        };

        // Parse both block layouts + SDNA for name resolution
        let after_blocks = parse_blend_blocks(&after_decomp).unwrap_or_default();
        let sdna = after_blocks.iter().find(|b| &b.code == b"DNA1").and_then(|b| {
            let dna_data = &after_decomp[b.data_offset..b.data_offset + b.data_len];
            parse_sdna(dna_data, 8).ok()
        });

        // Diff changed bytes across blocks
        let before_blocks = parse_blend_blocks(&before_decomp).unwrap_or_default();

        let mut out = String::new();
        out.push_str(&format!(
            "blend_python_diff: before={} bytes, after={} bytes\n\n",
            before_decomp.len(), after_decomp.len()
        ));

        let mut diff_count = 0;
        for (idx, b_block) in after_blocks.iter().enumerate() {
            if &b_block.code == b"DNA1" || &b_block.code == b"ENDB" { continue; }

            // Resolve SDNA name
            let type_name = sdna.as_ref().and_then(|s| {
                s.structs.get(b_block.sdna_index as usize).map(|st| st.name.as_str())
            }).unwrap_or("?");

            // Apply filter
            if let Some(ref filter) = req.sdna_filter {
                if !type_name.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }
            }

            // Find matching block in before by old_ptr or position
            let matching_before = before_blocks.iter().find(|bb| bb.old_ptr == b_block.old_ptr)
                .or_else(|| before_blocks.get(idx));

            let b_data = if b_block.data_offset + b_block.data_len <= after_decomp.len() {
                &after_decomp[b_block.data_offset..b_block.data_offset + b_block.data_len]
            } else {
                continue;
            };

            if let Some(a_block) = matching_before {
                if a_block.data_offset + a_block.data_len > before_decomp.len() { continue; }
                let a_data = &before_decomp[a_block.data_offset..a_block.data_offset + a_block.data_len];
                if a_data == b_data { continue; }

                diff_count += 1;
                out.push_str(&format!(
                    "CHANGED block #{idx}: code={} type={} ptr={:#018x} sdna={} count={}\n",
                    b_block.code_str(), type_name, b_block.old_ptr, b_block.sdna_index, b_block.count
                ));
                // Show changed byte ranges
                let min_len = a_data.len().min(b_data.len());
                let mut range_start = None;
                let mut changed_ranges: Vec<(usize, usize)> = Vec::new();
                for byte_idx in 0..min_len {
                    if a_data[byte_idx] != b_data[byte_idx] {
                        match range_start {
                            None => range_start = Some(byte_idx),
                            Some(_) => {}
                        }
                    } else if let Some(start) = range_start.take() {
                        changed_ranges.push((start, byte_idx));
                    }
                }
                if let Some(start) = range_start.take() {
                    changed_ranges.push((start, min_len));
                }

                for (start, end) in &changed_ranges {
                    let ctx_start = start.saturating_sub(8);
                    let ctx_end = (end + 8).min(min_len);
                    out.push_str(&format!("  Bytes [{start}..{end}] changed (showing context [{ctx_start}..{ctx_end}]):\n"));
                    out.push_str("  BEFORE:\n");
                    out.push_str(&hex_dump(&a_data[ctx_start..ctx_end], ctx_start));
                    out.push_str("  AFTER:\n");
                    out.push_str(&hex_dump(&b_data[ctx_start..ctx_end], ctx_start));
                }
            } else {
                diff_count += 1;
                out.push_str(&format!(
                    "NEW block #{idx}: code={} type={} ptr={:#018x}\n",
                    b_block.code_str(), type_name, b_block.old_ptr
                ));
                out.push_str(&hex_dump(&b_data[..b_data.len().min(256)], 0));
            }
        }

        if diff_count == 0 {
            out.push_str("No changed blocks detected.\n");
        } else {
            out.push_str(&format!("\nTotal changed/new blocks: {diff_count}\n"));
        }

        let _ = std::fs::remove_dir_all(&tmp);
        out
    }

    /// Run an arbitrary Python script against a `.blend` file in headless Blender.
    ///
    /// Opens `blend_path` with `blender --background --python <script>` and returns the
    /// script's printed output.  If the script wraps output with `__BLEND_SCRIPT_OUTPUT_START__`
    /// and `__BLEND_SCRIPT_OUTPUT_END__` sentinels, only the content between them is returned
    /// (filtering Blender startup noise). If sentinels are absent, full stdout+stderr is returned.
    /// `import bpy` is always available in Blender scripts — no need to import it explicitly.
    #[tool(description = "Run an arbitrary Python script against a .blend file in headless Blender. \
        Opens blend_path with `blender --background --python script` and returns printed output. \
        Wrap important output with print('__BLEND_SCRIPT_OUTPUT_START__') / print('__BLEND_SCRIPT_OUTPUT_END__') \
        sentinels to filter out Blender startup noise. If sentinels are absent, full stdout+stderr is returned. \
        `import bpy` is always available — no need to import it explicitly in your script.")]
    fn blend_run_script(&self, Parameters(req): Parameters<BlendRunScriptRequest>) -> String {
        use crate::blend_debug::find_blender_bin;
        use std::io::Write as _;

        let blender_bin = match find_blender_bin(req.blender_bin.as_deref()) {
            Ok(b) => b,
            Err(e) => return format!("blender binary not found: {e}"),
        };

        let blend_path = std::path::Path::new(&req.blend_path);
        if !blend_path.exists() {
            return format!("blend file not found: {}", req.blend_path);
        }

        // Temp dir for the script file
        let tmp = std::path::PathBuf::from(format!(
            "/tmp/blend_run_script_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        if let Err(e) = std::fs::create_dir_all(&tmp) {
            return format!("failed to create temp dir: {e}");
        }

        let script_path = tmp.join("script.py");
        if let Err(e) = std::fs::File::create(&script_path)
            .and_then(|mut f| f.write_all(req.script.as_bytes()))
        {
            let _ = std::fs::remove_dir_all(&tmp);
            return format!("failed to write script: {e}");
        }

        let output = std::process::Command::new(&blender_bin)
            .args([
                "--background",
                &req.blend_path,
                "--python",
                script_path.to_str().unwrap_or(""),
            ])
            .output();

        let _ = std::fs::remove_dir_all(&tmp);

        let output = match output {
            Ok(o) => o,
            Err(e) => return format!("failed to run blender: {e}"),
        };

        let full = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        // Extract between sentinels if present
        const START: &str = "__BLEND_SCRIPT_OUTPUT_START__";
        const END: &str = "__BLEND_SCRIPT_OUTPUT_END__";
        if let (Some(start_pos), Some(end_pos)) = (full.find(START), full.rfind(END)) {
            let content_start = start_pos + START.len();
            if content_start < end_pos {
                return full[content_start..end_pos].trim().to_string();
            }
        }

        // Fallback: return everything (script likely errored)
        if !output.status.success() {
            format!("blender exited with status {}\n\n{full}", output.status)
        } else {
            full
        }
    }

    #[tool(description = "Return the DataCore struct definition for a type name. Lists each attribute with its data_type (e.g. 'Class', 'EnumChoice', 'Single'), conversion_type (Attribute/SimpleArray/ComplexArray/ClassArray), and referenced type name (resolved for Class/Pointer/Reference/Enum properties). Also returns parent struct name, instance size, and whether attributes are inherited. Use this to enumerate the schema of any struct family (e.g. BuildingBlocks_*Widget*, BuildingBlocks_*RendererPolicy) without scraping record samples.")]
    fn datacore_struct_def(&self, Parameters(req): Parameters<DatacoreStructDefRequest>) -> String {
        let data = self.data();
        let db = starbreaker_datacore::database::Database::from_bytes(&data.dcb_bytes)
            .expect("DataCore bytes validated at load");

        let Some(idx) = db.struct_index_by_name(&req.name) else {
            // Suggest near matches
            let q = req.name.to_lowercase();
            let near: Vec<&str> = db
                .struct_defs()
                .iter()
                .map(|s| db.resolve_string2(s.name_offset))
                .filter(|n| n.to_lowercase().contains(&q))
                .take(20)
                .collect();
            return if near.is_empty() {
                format!("No struct named '{}'.", req.name)
            } else {
                format!(
                    "No struct named '{}'. Did you mean: {}",
                    req.name,
                    near.join(", ")
                )
            };
        };

        let include_inherited = req.include_inherited.unwrap_or(true);
        let sd = *db.struct_def(idx);
        let parent_type_index = sd.parent_type_index;
        let struct_size = sd.struct_size;
        let attribute_count = sd.attribute_count;
        let first_attribute_index = sd.first_attribute_index;
        let parent_name = if parent_type_index >= 0 {
            Some(db.resolve_string2(db.struct_def(parent_type_index).name_offset).to_string())
        } else {
            None
        };

        let format_prop = |prop: &starbreaker_datacore::types::PropertyDefinition| -> serde_json::Value {
            use starbreaker_datacore::enums::{ConversionType, DataType};
            let data_type_raw = prop.data_type;
            let conv_type_raw = prop.conversion_type;
            let referenced_index = prop.struct_index;
            let name_offset = prop.name_offset;
            let pname = db.resolve_string2(name_offset).to_string();
            let dt_name = match DataType::try_from(data_type_raw) {
                Ok(d) => format!("{d:?}"),
                Err(_) => format!("Unknown(0x{data_type_raw:04x})"),
            };
            let ct_name = match ConversionType::try_from(conv_type_raw) {
                Ok(c) => format!("{c:?}"),
                Err(_) => format!("Unknown(0x{conv_type_raw:04x})"),
            };
            // Resolve referenced type for Class / pointer / reference / enum
            let ref_name = match DataType::try_from(data_type_raw) {
                Ok(DataType::Class)
                | Ok(DataType::StrongPointer)
                | Ok(DataType::WeakPointer)
                | Ok(DataType::Reference) => {
                    db.struct_defs()
                        .get(referenced_index as usize)
                        .map(|s| db.resolve_string2(s.name_offset).to_string())
                }
                Ok(DataType::EnumChoice) => {
                    db.enum_defs()
                        .get(referenced_index as usize)
                        .map(|e| db.resolve_string2(e.name_offset).to_string())
                }
                _ => None,
            };
            serde_json::json!({
                "name": pname,
                "data_type": dt_name,
                "conversion_type": ct_name,
                "referenced_type": ref_name,
                "referenced_index": referenced_index,
            })
        };

        let property_defs = db.property_defs();
        let own_first = first_attribute_index as usize;
        let own_count = attribute_count as usize;
        let own: Vec<serde_json::Value> = property_defs[own_first..own_first + own_count]
            .iter()
            .map(format_prop)
            .collect();

        let inherited: Vec<serde_json::Value> = if include_inherited && parent_type_index >= 0 {
            let all = db.all_properties(idx);
            let total = all.len();
            all.iter()
                .take(total - own_count)
                .map(|p| format_prop(*p))
                .collect()
        } else {
            Vec::new()
        };

        let out = serde_json::json!({
            "name": db.resolve_string2(sd.name_offset),
            "struct_index": idx,
            "parent": parent_name,
            "struct_size": struct_size,
            "attribute_count": attribute_count,
            "first_attribute_index": first_attribute_index,
            "inherited_attributes": inherited,
            "own_attributes": own,
        });
        serde_json::to_string_pretty(&out).unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Return the DataCore enum definition for a type name: the enum name and its list of value strings in declaration order. Use this to resolve the legal values of any EnumChoice property (e.g. the renderer-type enum referenced by BuildingBlocks widget records).")]
    fn datacore_enum_def(&self, Parameters(req): Parameters<DatacoreEnumDefRequest>) -> String {
        let data = self.data();
        let db = starbreaker_datacore::database::Database::from_bytes(&data.dcb_bytes)
            .expect("DataCore bytes validated at load");

        let q = req.name.to_lowercase();
        let mut match_idx: Option<i32> = None;
        for (i, e) in db.enum_defs().iter().enumerate() {
            if db.resolve_string2(e.name_offset) == req.name {
                match_idx = Some(i as i32);
                break;
            }
        }

        let Some(idx) = match_idx else {
            let near: Vec<&str> = db
                .enum_defs()
                .iter()
                .map(|e| db.resolve_string2(e.name_offset))
                .filter(|n| n.to_lowercase().contains(&q))
                .take(20)
                .collect();
            return if near.is_empty() {
                format!("No enum named '{}'.", req.name)
            } else {
                format!(
                    "No enum named '{}'. Did you mean: {}",
                    req.name,
                    near.join(", ")
                )
            };
        };

        let ed = *db.enum_def(idx);
        let value_count = ed.value_count;
        let values: Vec<&str> = db
            .enum_options(idx)
            .iter()
            .map(|sid| db.resolve_string2(*sid))
            .collect();

        let out = serde_json::json!({
            "name": db.resolve_string2(ed.name_offset),
            "enum_index": idx,
            "value_count": value_count,
            "values": values,
        });
        serde_json::to_string_pretty(&out).unwrap_or_else(|e| format!("JSON error: {e}"))
    }

    #[tool(description = "Substring-search DataCore struct AND enum type names. Returns kind ('struct' or 'enum'), name, and size/value_count, sorted shortest-name-first. Use kind='struct' or kind='enum' to restrict. Empty query returns the first `limit` types (useful to skim the registry).")]
    fn datacore_type_search(&self, Parameters(req): Parameters<DatacoreTypeSearchRequest>) -> String {
        let data = self.data();
        let db = starbreaker_datacore::database::Database::from_bytes(&data.dcb_bytes)
            .expect("DataCore bytes validated at load");
        let limit = req.limit.unwrap_or(50) as usize;
        let kind = req.kind.as_deref().unwrap_or("both").to_ascii_lowercase();
        let q = req.query.to_lowercase();

        let mut hits: Vec<serde_json::Value> = Vec::new();

        if kind == "struct" || kind == "both" {
            for (i, sd) in db.struct_defs().iter().enumerate() {
                let sd = *sd;
                let n = db.resolve_string2(sd.name_offset);
                if q.is_empty() || n.to_lowercase().contains(&q) {
                    let parent = if sd.parent_type_index >= 0 {
                        Some(db.resolve_string2(db.struct_def(sd.parent_type_index).name_offset))
                    } else {
                        None
                    };
                    let size = sd.struct_size;
                    let attribute_count = sd.attribute_count;
                    hits.push(serde_json::json!({
                        "kind": "struct",
                        "name": n,
                        "index": i,
                        "size": size,
                        "attribute_count": attribute_count,
                        "parent": parent,
                    }));
                }
            }
        }

        if kind == "enum" || kind == "both" {
            for (i, ed) in db.enum_defs().iter().enumerate() {
                let ed = *ed;
                let n = db.resolve_string2(ed.name_offset);
                if q.is_empty() || n.to_lowercase().contains(&q) {
                    let value_count = ed.value_count;
                    hits.push(serde_json::json!({
                        "kind": "enum",
                        "name": n,
                        "index": i,
                        "value_count": value_count,
                    }));
                }
            }
        }

        hits.sort_by_key(|h| h.get("name").and_then(|v| v.as_str()).map(|s| s.len()).unwrap_or(usize::MAX));
        hits.truncate(limit);

        serde_json::to_string_pretty(&hits).unwrap_or_else(|e| format!("JSON error: {e}"))
    }

}
#[rmcp::tool_handler]
impl ServerHandler for StarBreakerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Star Citizen game data server. Query DataCore records, entity loadouts, and P4k archive files.",
        )
    }
}

/// Label for a `brandStyles[idx]` container in the style inventory. Appends the
/// brand identifier's basename (`brandStyles[1] s_drak_hud`) so the inventory
/// shows WHICH brand each index is — `brandIdentifier` is a record path
/// (`file://.../styles/s_drak_hud.json`); a null/missing one falls back to the
/// bare index.
fn brand_style_label(brand: &serde_json::Value, idx: usize) -> String {
    match brand
        .get("brandIdentifier")
        .and_then(|value| value.as_str())
        .map(|path| {
            path.rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".json")
        }) {
        Some(name) if !name.is_empty() => format!("brandStyles[{idx}] {name}"),
        _ => format!("brandStyles[{idx}]"),
    }
}

#[cfg(test)]
mod ui_regression_registry_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn brand_style_label_uses_brand_identifier_basename() {
        let brand = serde_json::json!({
            "brandIdentifier":
                "file://./../../libs/foundry/records/ui/buildingblocks/styles/s_drak_hud.json"
        });
        assert_eq!(brand_style_label(&brand, 1), "brandStyles[1] s_drak_hud");
        // null / missing brandIdentifier falls back to the bare index.
        assert_eq!(brand_style_label(&serde_json::json!({}), 3), "brandStyles[3]");
    }

    #[test]
    fn ui_regression_registry_defaults_report_expected_targets() {
        let server = StarBreakerMcp::new(None);
        let response = server.ui_regression_registry(Parameters(UiRegressionRegistryRequest {
            manifest_path: None,
            freeze_path: None,
            target_filter: None,
        }));

        let json: serde_json::Value =
            serde_json::from_str(&response).expect("registry response should be valid JSON");

        assert_eq!(json.get("schema_version").and_then(|v| v.as_u64()), Some(1));
        // The default manifest + freeze fixtures carry the full set of onboarded
        // UI parity targets (15 as of the compass/countermeasures/self/g-force/
        // velocity/master-mode/annunciator/... arcs), and every one is matched
        // (present in both manifest and freeze). Bump this guard when a target is
        // onboarded or retired.
        assert_eq!(
            json.get("summary")
                .and_then(|v| v.get("total_targets"))
                .and_then(|v| v.as_u64()),
            Some(15)
        );
        assert_eq!(
            json.get("summary")
                .and_then(|v| v.get("matched"))
                .and_then(|v| v.as_u64()),
            Some(15)
        );
    }

    #[test]
    fn ui_regression_registry_filter_limits_results() {
        let server = StarBreakerMcp::new(None);
        let response = server.ui_regression_registry(Parameters(UiRegressionRegistryRequest {
            manifest_path: None,
            freeze_path: None,
            target_filter: Some("ui_target_a".to_string()),
        }));

        let json: serde_json::Value =
            serde_json::from_str(&response).expect("registry response should be valid JSON");
        let targets = json
            .get("targets")
            .and_then(|v| v.as_array())
            .expect("targets array should exist");

        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].get("id").and_then(|v| v.as_str()),
            Some("ui_target_a")
        );
    }

    #[test]
    fn ui_regression_registry_surfaces_manifest_read_errors() {
        let server = StarBreakerMcp::new(None);
        let response = server.ui_regression_registry(Parameters(UiRegressionRegistryRequest {
            manifest_path: Some("does/not/exist.json".to_string()),
            freeze_path: None,
            target_filter: None,
        }));

        let json: serde_json::Value =
            serde_json::from_str(&response).expect("error response should be valid JSON");
        assert_eq!(
            json.get("error_code").and_then(|v| v.as_str()),
            Some("manifest_read_failed")
        );
    }

    #[test]
    fn ui_regression_registry_surfaces_schema_mismatch() {
        let server = StarBreakerMcp::new(None);

        let tmp_root = std::env::temp_dir().join(format!(
            "starbreaker_mcp_registry_schema_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp_root).expect("temp dir should be created");

        let manifest_path = tmp_root.join("manifest.json");
        let freeze_path = tmp_root.join("freeze.json");

        let mut mf = std::fs::File::create(&manifest_path).expect("manifest file create");
        writeln!(mf, "{{\"schema_version\":1,\"targets\":[]}}")
            .expect("manifest write");

        let mut ff = std::fs::File::create(&freeze_path).expect("freeze file create");
        writeln!(ff, "{{\"schema_version\":2,\"artifacts\":[]}}")
            .expect("freeze write");

        let response = server.ui_regression_registry(Parameters(UiRegressionRegistryRequest {
            manifest_path: Some(manifest_path.to_string_lossy().to_string()),
            freeze_path: Some(freeze_path.to_string_lossy().to_string()),
            target_filter: None,
        }));

        let _ = std::fs::remove_dir_all(&tmp_root);

        let json: serde_json::Value =
            serde_json::from_str(&response).expect("error response should be valid JSON");
        assert_eq!(
            json.get("error_code").and_then(|v| v.as_str()),
            Some("schema_mismatch")
        );
    }

    #[test]
    fn ui_regression_validate_rejects_invalid_mode() {
        let server = StarBreakerMcp::new(None);
        let response = server.ui_regression_validate(Parameters(UiRegressionValidateRequest {
            mode: Some("fast".to_string()),
            manifest_path: None,
            freeze_path: None,
        }));

        let json: serde_json::Value =
            serde_json::from_str(&response).expect("validator response should be valid JSON");
        assert_eq!(
            json.get("error_code").and_then(|v| v.as_str()),
            Some("invalid_mode")
        );
    }

    // The validator is a POSIX shell script (bash + jq); on Windows the
    // bash bridge mangles the Windows paths passed via env vars, so these
    // two tests only run where the script actually executes.
    #[cfg(unix)]
    #[test]
    fn ui_regression_validate_quick_mode_passes_with_default_fixtures() {
        let server = StarBreakerMcp::new(None);
        let response = server.ui_regression_validate(Parameters(UiRegressionValidateRequest {
            mode: Some("quick".to_string()),
            manifest_path: None,
            freeze_path: None,
        }));

        let json: serde_json::Value =
            serde_json::from_str(&response).expect("validator response should be valid JSON");
        assert_eq!(json.get("mode").and_then(|v| v.as_str()), Some("quick"));
        assert_eq!(json.get("passed").and_then(|v| v.as_bool()), Some(true));
    }

    // POSIX-script-backed — see the note on the quick-mode test above.
    #[cfg(unix)]
    #[test]
    fn ui_regression_validate_reports_missing_freeze_file() {
        let server = StarBreakerMcp::new(None);
        let response = server.ui_regression_validate(Parameters(UiRegressionValidateRequest {
            mode: Some("full".to_string()),
            manifest_path: None,
            freeze_path: Some("does/not/exist-freeze.json".to_string()),
        }));

        let json: serde_json::Value =
            serde_json::from_str(&response).expect("validator response should be valid JSON");
        assert_eq!(json.get("passed").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            json.get("error_code").and_then(|v| v.as_str()),
            Some("validation_failed")
        );
        assert!(
            json.get("stderr")
                .and_then(|v| v.as_str())
                .map(|s| s.contains("freeze file not found"))
                .unwrap_or(false),
            "expected stderr to mention missing freeze file"
        );
    }

    /// Helper: run a test with a live P4K/DataCore connection. Passes the server
    /// to the closure so the test can query real DataCore records.
    /// Uses auto-discovery (same path as production code).
    ///
    /// SKIPS (early-returns) when no Data.p4k can be discovered — CI runners
    /// have no Star Citizen install, and `server.data()` would otherwise
    /// panic. Locally, set `SC_DATA_P4K` to point at non-default data.
    fn with_p4k_test<F>(test: F)
    where
        F: FnOnce(StarBreakerMcp),
    {
        if let Err(e) = starbreaker_p4k::find_p4k() {
            eprintln!("SKIP: no Data.p4k available ({e}); set SC_DATA_P4K to run P4K-backed MCP tests");
            return;
        }
        let server = StarBreakerMcp::new(None);
        // Verify P4K loaded successfully via auto-discovery
        if server.data().p4k.entries().is_empty() {
            panic!(
                "P4K auto-discovery found a path but loaded no entries. \
                Set SC_DATA_P4K env var to override."
            );
        }
        test(server);
    }

    // -----------------------------------------------------------------------
    // Phase 1.5: Unit tests for P4kCanvasFetcher (mock-based, no P4K required)
    // -----------------------------------------------------------------------

    /// A mock canvas fetcher for unit testing.
    struct MockCanvasFetcher {
        canvases: std::collections::HashMap<String, serde_json::Value>,
    }

    impl MockCanvasFetcher {
        fn new() -> Self {
            Self {
                canvases: std::collections::HashMap::new(),
            }
        }

        fn insert(&mut self, guid: impl Into<String>, canvas: serde_json::Value) {
            self.canvases.insert(guid.into(), canvas);
        }

        fn insert_with_name(&mut self, guid: impl Into<String>, name: impl Into<String>, canvas: serde_json::Value) {
            let guid_str: String = guid.into();
            let name_str: String = name.into();
            let mut c = canvas;
            c["_RecordName_"] = serde_json::Value::String(name_str);
            c["_RecordId_"] = serde_json::Value::String(guid_str.clone());
            self.canvases.insert(guid_str, c);
        }
    }

    impl starbreaker_ui::CanvasFetcher for MockCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, starbreaker_ui::UiError> {
            self.canvases
                .get(guid)
                .cloned()
                .ok_or_else(|| starbreaker_ui::UiError::FetchFailed {
                    guid: guid.to_string(),
                    source: format!("No canvas with GUID {guid}").into(),
                })
        }

        fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, starbreaker_ui::UiError> {
            let search = record_name.to_lowercase();
            let mut candidates: Vec<(String, serde_json::Value)> = self
                .canvases
                .iter()
                .filter(|(_, c)| {
                    c.get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .map(|n| n.to_lowercase().contains(&search))
                        .unwrap_or(false)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            candidates.sort_by_key(|(_, c)| {
                c.get("_RecordName_")
                    .and_then(|v| v.as_str())
                    .map(|s| s.len())
                    .unwrap_or(usize::MAX)
            });
            match candidates.into_iter().next() {
                Some((_, c)) => Ok(c),
                None => Err(starbreaker_ui::UiError::FetchFailed {
                    guid: record_name.to_string(),
                    source: format!("No canvas matching name '{record_name}'").into(),
                }),
            }
        }
    }

    #[test]
    fn fetch_canvas_json_with_known_guid() {
        let mut mock = MockCanvasFetcher::new();
        let canvas = serde_json::json!({
            "_RecordId_": "test-guid-123",
            "_RecordName_": "TestCanvas",
            "scene": []
        });
        mock.insert("test-guid-123", canvas.clone());

        let result = mock.fetch_canvas_json("test-guid-123").expect("should find canvas");
        assert_eq!(
            result.get("_RecordId_").and_then(|v| v.as_str()),
            Some("test-guid-123")
        );
    }

    #[test]
    fn fetch_canvas_by_name_with_known_substring() {
        let mut mock = MockCanvasFetcher::new();
        mock.insert_with_name("guid-1", "BuildingBlocks_Canvas.GEN_MC_S_Target", serde_json::json!({"scene": []}));
        mock.insert_with_name("guid-2", "BuildingBlocks_Canvas.GEN_MC_S_TargetDiagram", serde_json::json!({"scene": []}));

        // Shortest match should win
        let result = mock.fetch_canvas_by_name("Target").expect("should find canvas");
        assert_eq!(
            result.get("_RecordId_").and_then(|v| v.as_str()),
            Some("guid-1")
        );
    }

    #[test]
    fn fetch_canvas_json_with_non_existent_guid() {
        let mock = MockCanvasFetcher::new();
        let result = mock.fetch_canvas_json("non-existent-guid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("non-existent-guid"));
    }

    #[test]
    fn fetch_canvas_by_name_with_empty_string() {
        let mut mock = MockCanvasFetcher::new();
        mock.insert_with_name("guid-1", "SomeCanvas", serde_json::json!({"scene": []}));

        // Empty string matches everything — should return shortest match
        let result = mock.fetch_canvas_by_name("");
        assert!(result.is_ok());
    }

    #[test]
    fn fetch_canvas_by_name_with_empty_string_no_canvases() {
        let mock = MockCanvasFetcher::new();
        let result = mock.fetch_canvas_by_name("");
        // No canvases → no match → error
        assert!(result.is_err());
    }

    #[test]
    fn extract_record_name_strips_file_scheme() {
        assert_eq!(
            starbreaker_ui::pipeline::extract_record_name("file://./path/to/canvas.json"),
            "canvas"
        );
    }

    #[test]
    fn extract_record_name_strips_file_scheme_no_extension() {
        assert_eq!(
            starbreaker_ui::pipeline::extract_record_name("file://./path/to/canvas"),
            "canvas"
        );
    }

    #[test]
    fn extract_record_name_handles_malformed_url() {
        // No scheme, no path — just a bare name
        assert_eq!(
            starbreaker_ui::pipeline::extract_record_name("bare_name"),
            "bare_name"
        );
    }

    #[test]
    fn extract_record_name_handles_deeply_nested_relative_path() {
        assert_eq!(
            starbreaker_ui::pipeline::extract_record_name("file://../../../../../../libs/foundry/records/gen_mc_s_target.json"),
            "gen_mc_s_target"
        );
    }

    #[test]
    fn extract_record_name_handles_case_insensitive_extension() {
        assert_eq!(
            starbreaker_ui::pipeline::extract_record_name("file://./path/to/Canvas.JSON"),
            "Canvas"
        );
    }

    #[test]
    fn fetch_canvas_by_name_case_insensitive() {
        let mut mock = MockCanvasFetcher::new();
        mock.insert_with_name("guid-1", "BuildingBlocks_Canvas.GEN_MC_S_Target", serde_json::json!({"scene": []}));

        let result = mock.fetch_canvas_by_name("target").expect("should find case-insensitively");
        assert_eq!(
            result.get("_RecordId_").and_then(|v| v.as_str()),
            Some("guid-1")
        );
    }

    #[test]
    fn fetch_canvas_by_name_no_match() {
        let mut mock = MockCanvasFetcher::new();
        mock.insert_with_name("guid-1", "BuildingBlocks_Canvas.GEN_MC_S_Target", serde_json::json!({"scene": []}));

        let result = mock.fetch_canvas_by_name("nonexistent_xyz");
        assert!(result.is_err());
    }

    #[test]
    fn ui_canvas_style_inventory_reports_style_sources() {
        with_p4k_test(|server| {
            let response = server.ui_canvas_style_inventory(Parameters(UiCanvasStyleInventoryRequest {
                canvas: "dd9ed6dc-7fe4-4884-9d11-c143290c9498".to_string(),
                canvas_root: None,
                entry_filter: None,
                include_conditions: Some(true),
                include_modifiers: Some(true),
                limit: None,
            }));

            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            assert_eq!(json.get("schema_version").and_then(|v| v.as_u64()), Some(1));
            assert!(
                json.get("entries")
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| !arr.is_empty())
            );
        });
    }

    #[test]
    fn ui_scene_style_probe_reports_applied_entries() {
        with_p4k_test(|server| {
            let response = server.ui_scene_style_probe(Parameters(UiSceneStyleProbeRequest {
                canvas: "dd9ed6dc-7fe4-4884-9d11-c143290c9498".to_string(),
                query: "Target".to_string(),
                canvas_root: None,
                manufacturer: None,
                limit: None,
            }));

            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            let nodes = json.get("nodes").and_then(|v| v.as_array()).expect("nodes array");
            assert!(nodes.len() >= 1);
        });
    }

    #[test]
    fn ui_ir_query_reports_matching_ir_nodes() {
        with_p4k_test(|server| {
            let response = server.ui_ir_query(Parameters(UiIrQueryRequest {
                canvas_guid: "dd9ed6dc-7fe4-4884-9d11-c143290c9498".to_string(),
                content_guid: None,
                query: "Target".to_string(),
                canvas_root: None,
                style_root: None,
                manufacturer: Some("drak".to_string()),
                helper: None,
                width: Some(1920),
                height: Some(1080),
                limit: None,
            }));

            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            assert_eq!(json.get("error_code"), None, "{json:#}");
            let nodes = json.get("nodes").and_then(|v| v.as_array()).expect("nodes array");
            assert!(nodes.len() >= 1);
        });
    }

    #[test]
    fn ui_tag_lookup_resolves_element_instance_tags() {
        with_p4k_test(|server| {
            let response = server.ui_tag_lookup(Parameters(UiTagLookupRequest {
                query: "icon-element".to_string(),
                limit: None,
            }));
            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            assert_eq!(json.get("error_code"), None, "{json:#}");
            let tags = json.get("tags").and_then(|v| v.as_array()).expect("tags array");
            assert!(
                tags.iter().any(|t| t.get("tag_name").and_then(|v| v.as_str())
                    == Some("icon-element-instance")),
                "{json:#}"
            );
        });
    }

    #[test]
    fn ui_kit_sheet_entries_resolves_condition_tag_names() {
        with_p4k_test(|server| {
            let response = server.ui_kit_sheet_entries(Parameters(UiKitSheetEntriesRequest {
                record: "sk_uilo_a_buttonprimarystyles".to_string(),
                entry_filter: Some("FilledText".to_string()),
                limit: None,
            }));
            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            assert_eq!(json.get("error_code"), None, "{json:#}");
            let entries = json.get("entries").and_then(|v| v.as_array()).expect("entries");
            assert!(!entries.is_empty(), "{json:#}");
            let when = entries[0].get("when").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                when.contains("text-element-instance"),
                "conditions should carry resolved tag names, got: {when}"
            );
        });
    }

    // -----------------------------------------------------------------------
    // Phase 2.5: Unit tests for P4kStyleFetcher (mock-based, no P4K required)
    // -----------------------------------------------------------------------

    use starbreaker_ui::StyleFetcher;

    /// A mock style fetcher for unit testing.
    struct MockStyleFetcher {
        styles: std::collections::HashMap<String, starbreaker_ui::ManufacturerStyle>,
    }

    impl MockStyleFetcher {
        fn new() -> Self {
            Self {
                styles: std::collections::HashMap::new(),
            }
        }

        fn insert(&mut self, manufacturer_id: &str, style: starbreaker_ui::ManufacturerStyle) {
            self.styles.insert(manufacturer_id.to_string(), style);
        }
    }

    impl starbreaker_ui::StyleFetcher for MockStyleFetcher {
        fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<starbreaker_ui::ManufacturerStyle, starbreaker_ui::UiError> {
            self.styles
                .get(manufacturer_id)
                .cloned()
                .ok_or_else(|| starbreaker_ui::UiError::RenderError(format!(
                    "missing BuildingBlocks style record for manufacturer '{}'",
                    manufacturer_id
                )))
        }
    }

    #[test]
    fn style_fetcher_returns_drak_style() {
        let mut mock = MockStyleFetcher::new();
        let style = starbreaker_ui::ManufacturerStyle {
            name: "drak".to_string(),
            primary_tint: starbreaker_ui::canvas::RgbaColor { r: 240, g: 168, b: 104, a: 255 },
            secondary_tint: None,
            colour_slots: vec![starbreaker_ui::canvas::RgbaColor { r: 240, g: 168, b: 104, a: 255 }],
            background: starbreaker_ui::canvas::RgbaColor { r: 48, g: 32, b: 16, a: 255 },
            backlight: starbreaker_ui::canvas::RgbaColor { r: 102, g: 214, b: 255, a: 255 },
            font_family_hints: vec!["Rajdhani".into(), "Orbitron".into()],
            crt: starbreaker_ui::style::CrtParams::default(),
        };
        mock.insert("drak", style.clone());

        let result = mock.fetch_manufacturer_style("drak").expect("should find drak style");
        assert_eq!(result.name, "drak");
        assert_eq!(result.primary_tint.r, 240);
        assert_eq!(result.primary_tint.g, 168);
    }

    #[test]
    fn style_fetcher_returns_banu_style() {
        let mut mock = MockStyleFetcher::new();
        let style = starbreaker_ui::ManufacturerStyle {
            name: "banu".to_string(),
            primary_tint: starbreaker_ui::canvas::RgbaColor { r: 200, g: 100, b: 50, a: 255 },
            secondary_tint: None,
            colour_slots: vec![],
            background: starbreaker_ui::canvas::RgbaColor { r: 30, g: 20, b: 10, a: 255 },
            backlight: starbreaker_ui::canvas::RgbaColor { r: 255, g: 200, b: 100, a: 255 },
            font_family_hints: vec![],
            crt: starbreaker_ui::style::CrtParams::default(),
        };
        mock.insert("banu", style.clone());

        let result = mock.fetch_manufacturer_style("banu").expect("should find banu style");
        assert_eq!(result.name, "banu");
        assert_eq!(result.primary_tint.r, 200);
    }

    #[test]
    fn style_fetcher_unknown_manufacturer_returns_error() {
        let mock = MockStyleFetcher::new();
        let result = mock.fetch_manufacturer_style("unknown_xyz");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unknown_xyz"));
    }

    #[test]
    fn style_fetcher_empty_string_returns_error() {
        let mock = MockStyleFetcher::new();
        let result = mock.fetch_manufacturer_style("");
        assert!(result.is_err());
    }

    #[test]
    fn ui_scene_style_probe_with_drak_manufacturer() {
        with_p4k_test(|server| {
            let response = server.ui_scene_style_probe(Parameters(UiSceneStyleProbeRequest {
                canvas: "dd9ed6dc-7fe4-4884-9d11-c143290c9498".to_string(),
                query: "Target".to_string(),
                canvas_root: None,
                manufacturer: Some("drak".to_string()),
                limit: None,
            }));

            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            assert_eq!(json.get("schema_version").and_then(|v| v.as_u64()), Some(1));
            let nodes = json.get("nodes").and_then(|v| v.as_array()).expect("nodes array");
            assert!(nodes.len() >= 1);
        });
    }

    #[test]
    fn ui_scene_style_probe_with_banu_manufacturer() {
        with_p4k_test(|server| {
            let response = server.ui_scene_style_probe(Parameters(UiSceneStyleProbeRequest {
                canvas: "dd9ed6dc-7fe4-4884-9d11-c143290c9498".to_string(),
                query: "Target".to_string(),
                canvas_root: None,
                manufacturer: Some("banu".to_string()),
                limit: None,
            }));

            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            assert_eq!(json.get("schema_version").and_then(|v| v.as_u64()), Some(1));
            let nodes = json.get("nodes").and_then(|v| v.as_array()).expect("nodes array");
            assert!(nodes.len() >= 1);
        });
    }

    #[test]
    fn ui_canvas_style_inventory_with_drak_manufacturer() {
        with_p4k_test(|server| {
            let response = server.ui_canvas_style_inventory(Parameters(UiCanvasStyleInventoryRequest {
                canvas: "dd9ed6dc-7fe4-4884-9d11-c143290c9498".to_string(),
                canvas_root: None,
                entry_filter: None,
                include_conditions: Some(true),
                include_modifiers: Some(true),
                limit: None,
            }));

            let json: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
            assert_eq!(json.get("schema_version").and_then(|v| v.as_u64()), Some(1));
            assert!(
                json.get("entries")
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| !arr.is_empty())
            );
        });
    }

    #[test]
    fn style_fetcher_no_records_for_manufacturer_returns_error() {
        let mock = MockStyleFetcher::new();
        // No records inserted
        let result = mock.fetch_manufacturer_style("nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    // -----------------------------------------------------------------------
    // Phase 3.5: Unit tests for P4kSwfFetcher path normalization
    // -----------------------------------------------------------------------

    #[test]
    fn p4k_swf_fetcher_normalize_path_adds_data_prefix() {
        let path = P4kSwfFetcher::normalize_path("UI/ShipInterface/assets/SWF/DRAK/test.swf");
        assert_eq!(path, "Data\\UI\\ShipInterface\\assets\\SWF\\DRAK\\test.swf");
    }

    #[test]
    fn p4k_swf_fetcher_normalize_path_converts_slashes() {
        let path = P4kSwfFetcher::normalize_path("Data/UI/ShipInterface/test.swf");
        assert_eq!(path, "Data\\UI\\ShipInterface\\test.swf");
    }

    #[test]
    fn p4k_swf_fetcher_normalize_path_preserves_backslashes() {
        let path = P4kSwfFetcher::normalize_path(r"Data\UI\ShipInterface\test.swf");
        assert_eq!(path, r"Data\UI\ShipInterface\test.swf");
    }

    #[test]
    fn p4k_swf_fetcher_normalize_path_auto_convert_tif_to_dds() {
        let path = P4kSwfFetcher::normalize_path("UI/Textures/test.tif");
        assert_eq!(path, "Data\\UI\\Textures\\test.dds");
    }

    #[test]
    fn p4k_swf_fetcher_normalize_path_case_insensitive_data_prefix() {
        // normalize_path preserves original case of prefix, only converts slashes
        let path = P4kSwfFetcher::normalize_path("data/UI/test.swf");
        assert_eq!(path, "data\\UI\\test.swf");
    }

    #[test]
    fn p4k_swf_fetcher_normalize_path_preserves_existing_data_prefix() {
        let path = P4kSwfFetcher::normalize_path(r"Data\UI\ShipInterface\SWF\DRAK\test.swf");
        assert_eq!(path, r"Data\UI\ShipInterface\SWF\DRAK\test.swf");
    }

    #[test]
    fn p4k_swf_fetcher_normalize_path_no_extension_unchanged() {
        let path = P4kSwfFetcher::normalize_path("UI/ShipInterface/test.swf");
        assert_eq!(path, "Data\\UI\\ShipInterface\\test.swf");
    }

    // -----------------------------------------------------------------------
    // Phase 4.5: Unit tests for P4kAssetFetcher path normalization
    // -----------------------------------------------------------------------

    #[test]
    fn p4k_asset_fetcher_normalize_path_adds_data_prefix() {
        let path = P4kAssetFetcher::normalize_path("UI/Textures/test.dds");
        assert_eq!(path, "Data\\UI\\Textures\\test.dds");
    }

    #[test]
    fn p4k_asset_fetcher_normalize_path_converts_slashes() {
        let path = P4kAssetFetcher::normalize_path("Data/UI/Textures/test.svg");
        assert_eq!(path, "Data\\UI\\Textures\\test.svg");
    }

    #[test]
    fn p4k_asset_fetcher_normalize_path_preserves_backslashes() {
        let path = P4kAssetFetcher::normalize_path(r"Data\UI\Textures\test.png");
        assert_eq!(path, r"Data\UI\Textures\test.png");
    }

    #[test]
    fn p4k_asset_fetcher_normalize_path_auto_convert_tif_to_dds() {
        let path = P4kAssetFetcher::normalize_path("UI/Textures/test.tif");
        assert_eq!(path, "Data\\UI\\Textures\\test.dds");
    }

    #[test]
    fn p4k_asset_fetcher_normalize_path_preserves_existing_data_prefix() {
        let path = P4kAssetFetcher::normalize_path(r"Data\UI\Textures\Vector\test.svg");
        assert_eq!(path, r"Data\UI\Textures\Vector\test.svg");
    }

    #[test]
    fn p4k_asset_fetcher_normalize_path_no_extension_unchanged() {
        let path = P4kAssetFetcher::normalize_path("UI/Textures/test.dds");
        assert_eq!(path, "Data\\UI\\Textures\\test.dds");
    }
}

fn format_loadout_node(
    node: &starbreaker_datacore::loadout::LoadoutNode,
    depth: usize,
    out: &mut String,
) {
    use std::fmt::Write;
    let indent = "  ".repeat(depth);
    let geom = node.geometry_path.as_deref().unwrap_or("-");
    let _ = writeln!(
        out,
        "{indent}{} [{}] geom={geom}",
        node.entity_name, node.item_port_name
    );
    for child in &node.children {
        format_loadout_node(child, depth + 1, out);
    }
}

struct JsonBudget {
    remaining_nodes: usize,
}

impl JsonBudget {
    fn new() -> Self {
        Self {
            remaining_nodes: MAX_DATACORE_QUERY_JSON_NODES,
        }
    }

    fn consume(&mut self, detail: impl Into<String>) -> Result<(), String> {
        if self.remaining_nodes == 0 {
            return Err(format!(
                "Query result too large: exceeded JSON node limit {} while serializing {}. Narrow the path before retrying.",
                MAX_DATACORE_QUERY_JSON_NODES,
                detail.into()
            ));
        }
        self.remaining_nodes -= 1;
        Ok(())
    }
}

/// Convert a DataCore `Value` to a bounded `serde_json::Value`.
fn value_to_json_limited(
    v: &starbreaker_datacore::query::value::Value,
    depth: usize,
    budget: &mut JsonBudget,
) -> Result<serde_json::Value, String> {
    use starbreaker_datacore::query::value::Value;
    if depth > MAX_DATACORE_QUERY_JSON_DEPTH {
        return Err(format!(
            "Query result too deep: exceeded nesting limit {}. Narrow the path before retrying.",
            MAX_DATACORE_QUERY_JSON_DEPTH
        ));
    }
    budget.consume(format!("depth {}", depth))?;
    match v {
        Value::Null => Ok(serde_json::Value::Null),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Int8(n) => Ok(serde_json::json!(*n)),
        Value::Int16(n) => Ok(serde_json::json!(*n)),
        Value::Int32(n) => Ok(serde_json::json!(*n)),
        Value::Int64(n) => Ok(serde_json::json!(*n)),
        Value::UInt8(n) => Ok(serde_json::json!(*n)),
        Value::UInt16(n) => Ok(serde_json::json!(*n)),
        Value::UInt32(n) => Ok(serde_json::json!(*n)),
        Value::UInt64(n) => Ok(serde_json::json!(*n)),
        Value::Float(n) => Ok(serde_json::json!(*n)),
        Value::Double(n) => Ok(serde_json::json!(*n)),
        Value::String(s) => Ok(serde_json::Value::String(truncate_string(s))),
        Value::Guid(g) => Ok(serde_json::Value::String(format!("{g}"))),
        Value::Enum(s) => Ok(serde_json::Value::String(truncate_string(s))),
        Value::Locale(s) => Ok(serde_json::Value::String(truncate_string(s))),
        Value::Array(items) => {
            if items.len() > MAX_DATACORE_QUERY_ARRAY_ITEMS {
                return Err(format!(
                    "Query result too large: array has {} items (limit {}). Narrow the path before retrying.",
                    items.len(),
                    MAX_DATACORE_QUERY_ARRAY_ITEMS
                ));
            }
            let values = items
                .iter()
                .map(|item| value_to_json_limited(item, depth + 1, budget))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::Value::Array(values))
        }
        Value::Object {
            type_name, fields, record_id,
        } => {
            if fields.len() > MAX_DATACORE_QUERY_OBJECT_FIELDS {
                return Err(format!(
                    "Query result too large: object '{}' has {} fields (limit {}). Narrow the path before retrying.",
                    type_name,
                    fields.len(),
                    MAX_DATACORE_QUERY_OBJECT_FIELDS
                ));
            }
            let mut map = serde_json::Map::new();
            map.insert("__type".to_string(), serde_json::Value::String(type_name.to_string()));
            if let Some(rid) = record_id {
                map.insert("__id".to_string(), serde_json::Value::String(format!("{rid}")));
            }
            for (key, val) in fields {
                map.insert(
                    key.to_string(),
                    value_to_json_limited(val, depth + 1, budget)?,
                );
            }
            Ok(serde_json::Value::Object(map))
        }
    }
}

fn truncate_string(value: &str) -> String {
    if value.chars().count() <= MAX_DATACORE_QUERY_STRING_CHARS {
        return value.to_string();
    }
    value
        .chars()
        .take(MAX_DATACORE_QUERY_STRING_CHARS)
        .collect::<String>()
        + "…"
}

/// Create a Content::Text item.
fn content_text(text: impl Into<String>) -> rmcp::model::Content {
    rmcp::model::Content::new(rmcp::model::RawContent::text(text), None)
}

/// Create a Content::Image item.
fn content_image(data: impl Into<String>, mime_type: impl Into<String>) -> rmcp::model::Content {
    rmcp::model::Content::new(rmcp::model::RawContent::image(data, mime_type), None)
}

/// P4k-backed sibling reader for split DDS mip files.
struct P4kSiblingReader {
    p4k: Arc<MappedP4k>,
    base_path: String,
}

impl starbreaker_dds::ReadSibling for P4kSiblingReader {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let path = format!("{}{suffix}", self.base_path);
        self.p4k.read_file(&path).ok()
    }
}

/// Format bytes as a hex dump with ASCII sidebar.
fn format_hex(data: &[u8], out: &mut String) {
    use std::fmt::Write;
    for (i, chunk) in data.chunks(16).enumerate() {
        let _ = write!(out, "  {:04x}: ", i * 16);
        for (j, byte) in chunk.iter().enumerate() {
            let _ = write!(out, "{:02x} ", byte);
            if j == 7 { let _ = write!(out, " "); }
        }
        // Pad if short line
        for _ in chunk.len()..16 {
            let _ = write!(out, "   ");
        }
        if chunk.len() <= 8 { let _ = write!(out, " "); }
        let _ = write!(out, " |");
        for byte in chunk {
            let c = if byte.is_ascii_graphic() || *byte == b' ' { *byte as char } else { '.' };
            let _ = write!(out, "{c}");
        }
        let _ = writeln!(out, "|");
    }
}

/// Format a byte size as human-readable.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 { return format!("{bytes} B"); }
    if bytes < 1024 * 1024 { return format!("{:.1} KB", bytes as f64 / 1024.0); }
    if bytes < 1024 * 1024 * 1024 { return format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)); }
    format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

fn inspect_included_objects_chunk(data: &[u8]) -> serde_json::Value {
    match starbreaker_3d::IncludedObjects::from_bytes(data) {
        Ok(io) => {
            let raw_counts = match raw_included_object_type_counts(data) {
                Ok(counts) => counts
                    .into_iter()
                    .map(|(obj_type, count)| {
                        serde_json::json!({
                            "type": format!("0x{obj_type:08x}"),
                            "count": count,
                        })
                    })
                    .collect::<Vec<_>>(),
                Err(e) => {
                    return serde_json::json!({
                        "error": format!("Raw IncludedObjects scan failed: {e}"),
                    });
                }
            };

            serde_json::json!({
                "cgf_count": io.cgf_paths.len(),
                "material_count": io.material_paths.len(),
                "palette_count": io.tint_palette_paths.len(),
                "parsed_type1_object_count": io.objects.len(),
                "raw_object_type_counts": raw_counts,
                "cgf_sample": io.cgf_paths.iter().take(20).cloned().collect::<Vec<_>>(),
                "palette_paths": io.tint_palette_paths,
            })
        }
        Err(e) => serde_json::json!({
            "error": format!("IncludedObjects parse failed: {e}"),
        }),
    }
}

fn raw_included_object_type_counts(
    data: &[u8],
) -> Result<std::collections::BTreeMap<u32, usize>, String> {
    let mut off = 4usize;
    skip_included_objects_header(data, &mut off)?;

    let len_objects_bytes = read_u32_le(data, &mut off)? as usize;
    let objects_end = off
        .checked_add(len_objects_bytes)
        .ok_or_else(|| "IncludedObjects object section overflow".to_string())?;
    if objects_end > data.len() {
        return Err(format!(
            "IncludedObjects object section truncated: end={objects_end}, len={}",
            data.len()
        ));
    }

    let mut counts = std::collections::BTreeMap::new();
    while off + 4 <= objects_end {
        let obj_type = read_u32_at_le(data, off)?;
        *counts.entry(obj_type).or_insert(0) += 1;

        match obj_type {
            0x0000_0001 => {
                let base_size = 168usize;
                if off + base_size > objects_end {
                    return Err(format!("Type1 object truncated at offset {off}"));
                }
                let unknown3 = read_u64_at_le(data, off + 160)?;
                let actual_size = if unknown3 == 0 { 184usize } else { 168usize };
                let mut end = off + actual_size;
                if end > objects_end {
                    return Err(format!(
                        "Type1 object overruns object section at offset {off} (size {actual_size})"
                    ));
                }
                while end + 4 <= objects_end {
                    let next_type = read_u32_at_le(data, end)?;
                    if next_type == 0 {
                        end += 4;
                    } else {
                        break;
                    }
                }
                off = end;
            }
            0x0000_0007 => off += 152,
            0x0000_0010 => off += 136,
            _ => off += 4,
        }

        if off > objects_end {
            return Err(format!(
                "Object scan advanced past end of section: off={off}, end={objects_end}"
            ));
        }
    }

    Ok(counts)
}

fn skip_included_objects_header(data: &[u8], off: &mut usize) -> Result<(), String> {
    let num_cgfs = read_u32_le(data, off)? as usize;
    advance(data, off, num_cgfs * 256, "CGF paths")?;

    let num_materials = read_u16_le(data, off)? as usize;
    let num_palettes = read_u16_le(data, off)? as usize;
    advance(data, off, num_materials * 256, "material paths")?;
    advance(data, off, num_palettes * 256, "palette paths")?;
    advance(data, off, 28, "unknown header bytes")?;
    Ok(())
}

fn advance(data: &[u8], off: &mut usize, len: usize, label: &str) -> Result<(), String> {
    let end = off
        .checked_add(len)
        .ok_or_else(|| format!("Offset overflow while skipping {label}"))?;
    if end > data.len() {
        return Err(format!(
            "IncludedObjects truncated while skipping {label}: need end={end}, len={}",
            data.len()
        ));
    }
    *off = end;
    Ok(())
}

fn read_u16_le(data: &[u8], off: &mut usize) -> Result<u16, String> {
    let value = read_u16_at_le(data, *off)?;
    *off += 2;
    Ok(value)
}

fn read_u32_le(data: &[u8], off: &mut usize) -> Result<u32, String> {
    let value = read_u32_at_le(data, *off)?;
    *off += 4;
    Ok(value)
}

fn read_u16_at_le(data: &[u8], idx: usize) -> Result<u16, String> {
    let bytes: [u8; 2] = data
        .get(idx..idx + 2)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Truncated u16 at offset {idx}"))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_at_le(data: &[u8], idx: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = data
        .get(idx..idx + 4)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Truncated u32 at offset {idx}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64_at_le(data: &[u8], idx: usize) -> Result<u64, String> {
    let bytes: [u8; 8] = data
        .get(idx..idx + 8)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Truncated u64 at offset {idx}"))?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::{
        inspect_included_objects_chunk, raw_included_object_type_counts, value_to_json_limited,
        JsonBudget, StarBreakerMcp,
    };
    use starbreaker_datacore::query::value::Value;

    fn build_included_objects_chunk(object_payloads: &[Vec<u8>]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0u8; 4]); // padding
        buf.extend_from_slice(&1u32.to_le_bytes()); // one cgf

        let mut path = b"test.cgf".to_vec();
        path.resize(256, 0);
        buf.extend_from_slice(&path);

        buf.extend_from_slice(&0u16.to_le_bytes()); // materials
        buf.extend_from_slice(&0u16.to_le_bytes()); // palettes
        buf.extend_from_slice(&[0u8; 28]); // unknown bytes

        let objects: Vec<u8> = object_payloads.iter().flat_map(|payload| payload.clone()).collect();
        buf.extend_from_slice(&(objects.len() as u32).to_le_bytes());
        buf.extend_from_slice(&objects);
        buf
    }

    fn type1_object(unknown3: u64) -> Vec<u8> {
        let mut obj = Vec::new();
        obj.extend_from_slice(&1u32.to_le_bytes());
        obj.extend_from_slice(&[0u8; 48]); // vector1 + vector2
        obj.extend_from_slice(&[0u8; 8]); // unknown1
        obj.extend_from_slice(&0u16.to_le_bytes()); // cgf id
        obj.extend_from_slice(&0u16.to_le_bytes()); // unknown2
        for &val in &[
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0f64,
        ] {
            obj.extend_from_slice(&val.to_le_bytes());
        }
        obj.extend_from_slice(&unknown3.to_le_bytes());
        if unknown3 == 0 {
            obj.extend_from_slice(&[0u8; 16]);
        }
        obj
    }

    fn fixed_type_object(obj_type: u32, total_size: usize) -> Vec<u8> {
        let mut obj = Vec::with_capacity(total_size);
        obj.extend_from_slice(&obj_type.to_le_bytes());
        obj.resize(total_size, 0);
        obj
    }

    #[test]
    fn raw_included_object_type_counts_reports_mixed_types() {
        let data = build_included_objects_chunk(&[
            type1_object(42),
            fixed_type_object(0x0000_0007, 152),
            fixed_type_object(0x0000_0010, 136),
        ]);

        let counts = raw_included_object_type_counts(&data).expect("raw scan should succeed");
        assert_eq!(counts.get(&0x0000_0001), Some(&1));
        assert_eq!(counts.get(&0x0000_0007), Some(&1));
        assert_eq!(counts.get(&0x0000_0010), Some(&1));
    }

    #[test]
    fn inspect_included_objects_chunk_exposes_skipped_types() {
        let data = build_included_objects_chunk(&[
            type1_object(0),
            fixed_type_object(0x0000_0007, 152),
            fixed_type_object(0x0000_0010, 136),
        ]);

        let value = inspect_included_objects_chunk(&data);
        let parsed_count = value
            .get("parsed_type1_object_count")
            .and_then(|v| v.as_u64());
        assert_eq!(parsed_count, Some(1));

        let raw_counts = value
            .get("raw_object_type_counts")
            .and_then(|v| v.as_array())
            .expect("raw counts array");
        assert!(raw_counts.iter().any(|entry| {
            entry.get("type").and_then(|v| v.as_str()) == Some("0x00000007")
                && entry.get("count").and_then(|v| v.as_u64()) == Some(1)
        }));
        assert!(raw_counts.iter().any(|entry| {
            entry.get("type").and_then(|v| v.as_str()) == Some("0x00000010")
                && entry.get("count").and_then(|v| v.as_u64()) == Some(1)
        }));
    }

    #[test]
    fn decode_archive_entry_bytes_returns_utf8_for_entxml_fallback() {
        let decoded = StarBreakerMcp::decode_archive_entry_bytes(
            "top_deck\\entdata\\123.entxml",
            b"<Entity Name=\"Door\" />",
        );
        assert_eq!(decoded, "<Entity Name=\"Door\" />");
    }

    #[test]
    fn split_socpak_entry_path_parses_outer_and_inner_paths() {
        let (outer, inner) = StarBreakerMcp::split_socpak_entry_path(
            "ObjectContainers/Ships/DRAK/ironclad/top_deck.socpak::top_deck/entdata/1425539375.entxml",
        )
        .expect("nested socpak path should parse");
        assert_eq!(outer, "ObjectContainers/Ships/DRAK/ironclad/top_deck.socpak");
        assert_eq!(inner, "top_deck/entdata/1425539375.entxml");
    }

    #[test]
    fn split_socpak_entry_path_rejects_empty_segments() {
        assert!(StarBreakerMcp::split_socpak_entry_path("foo.socpak::").is_none());
        assert!(StarBreakerMcp::split_socpak_entry_path("::inner.entxml").is_none());
        assert!(StarBreakerMcp::split_socpak_entry_path("plain/path.xml").is_none());
    }

    #[test]
    fn value_to_json_limited_rejects_large_arrays() {
        let value = Value::Array(
            (0..(super::MAX_DATACORE_QUERY_ARRAY_ITEMS + 1))
                .map(|_| Value::Int32(1))
                .collect(),
        );

        let err = value_to_json_limited(&value, 0, &mut JsonBudget::new())
            .expect_err("large array should be rejected");

        assert!(err.contains("array has"));
    }

    #[test]
    fn value_to_json_limited_truncates_long_strings() {
        let input = "a".repeat(super::MAX_DATACORE_QUERY_STRING_CHARS + 10);
        let leaked: &'static str = Box::leak(input.into_boxed_str());

        let json = value_to_json_limited(&Value::String(leaked), 0, &mut JsonBudget::new())
            .expect("string should serialize");

        let output = json.as_str().expect("serialized string");
        assert_eq!(
            output.chars().count(),
            super::MAX_DATACORE_QUERY_STRING_CHARS + 1
        );
        assert!(output.ends_with('…'));
    }
}
