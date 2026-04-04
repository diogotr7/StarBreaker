use std::sync::{Arc, OnceLock};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_router,
};
use starbreaker_p4k::MappedP4k;

/// Lazily-loaded game data. Initialized on first tool call.
struct GameData {
    p4k: Arc<MappedP4k>,
    dcb_bytes: Vec<u8>,
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
pub struct MtlSummaryRequest {
    #[schemars(description = "Path to .mtl file in P4k (case-insensitive, Data\\ prefix optional)")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct P4kExtractRequest {
    #[schemars(description = "P4k paths or glob patterns to extract (e.g. ['Data/Objects/Spaceships/AEGS/AEGS_Gladius/*', 'Data/Objects/foo.cgf'])")]
    pub paths: Vec<String>,
    #[schemars(description = "Output directory (default: tmp/p4k/ relative to working dir). Preserves P4k directory structure.")]
    pub output_dir: Option<String>,
}


pub struct StarBreakerMcp {
    p4k_path: Option<std::path::PathBuf>,
    data: OnceLock<GameData>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl StarBreakerMcp {
    pub fn new(p4k_path: Option<std::path::PathBuf>) -> Self {
        Self {
            p4k_path,
            data: OnceLock::new(),
            tool_router: Self::tool_router(),
        }
    }

    /// Lazily load P4k and DataCore on first access.
    fn data(&self) -> &GameData {
        self.data.get_or_init(|| {
            let start = std::time::Instant::now();
            let p4k = match &self.p4k_path {
                Some(path) => starbreaker_p4k::MappedP4k::open(path)
                    .unwrap_or_else(|e| panic!("Failed to open P4k at {}: {e}", path.display())),
                None => starbreaker_p4k::open_p4k()
                    .unwrap_or_else(|e| panic!("Failed to auto-discover P4k: {e}")),
            };
            let p4k = Arc::new(p4k);
            log::info!("P4k loaded in {:.1}s", start.elapsed().as_secs_f32());

            let dcb_bytes = p4k
                .read_file("Data\\Game2.dcb")
                .or_else(|_| p4k.read_file("Data\\Game.dcb"))
                .expect("Failed to read DataCore from P4k");
            log::info!("DataCore: {} bytes, loaded in {:.1}s", dcb_bytes.len(), start.elapsed().as_secs_f32());

            GameData { p4k, dcb_bytes }
        })
    }

    fn p4k(&self) -> &MappedP4k {
        &self.data().p4k
    }

    fn db(&self) -> starbreaker_datacore::database::Database<'_> {
        starbreaker_datacore::database::Database::from_bytes(&self.data().dcb_bytes)
            .expect("DataCore bytes validated at load")
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

    /// Read a file from P4k with case-insensitive fallback.
    fn read_p4k_file(&self, path: &str) -> Result<Vec<u8>, String> {
        let p4k_path = Self::normalize_p4k_path(path);
        self.p4k().read_file(&p4k_path)
            .or_else(|_| {
                self.p4k().entry_case_insensitive(&p4k_path)
                    .ok_or_else(|| format!("File not found in P4k: {p4k_path}"))
                    .and_then(|entry| self.p4k().read(entry).map_err(|e| format!("Error reading: {e}")))
            })
            .map_err(|e| format!("{e}"))
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

#[tool_router]
impl StarBreakerMcp {
    #[tool(description = "Search DataCore for entity records by name substring. Returns JSON array of matches sorted by name length (best match first).")]
    fn search_entities(&self, Parameters(req): Parameters<SearchEntitiesRequest>) -> String {
        let db = self.db();
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
        let db = self.db();
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
        let db = self.db();

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

    #[tool(description = "Query a specific property path on a DataCore record. Returns the JSON value at that path. Example paths: 'Components[VehicleComponentParams].vehicleDefinition', 'Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path'")]
    fn datacore_query(&self, Parameters(req): Parameters<DatacoreQueryRequest>) -> String {
        let db = self.db();

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

        match db.query::<starbreaker_datacore::query::value::Value>(&compiled, record) {
            Ok(results) => {
                let json_values: Vec<serde_json::Value> =
                    results.iter().map(value_to_json).collect();
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

    #[tool(description = "Read a file from the P4k archive. CryXML files (.xml, .mtl, .chrparams, .cdf) are auto-decoded to XML. Text files returned as-is. Binary files as base64.")]
    fn p4k_read(&self, Parameters(req): Parameters<P4kReadRequest>) -> String {
        let data = match self.read_p4k_file(&req.path) {
            Ok(d) => d,
            Err(e) => return e,
        };

        let lower = req.path.to_lowercase();
        let is_cryxml_ext = lower.ends_with(".xml")
            || lower.ends_with(".mtl")
            || lower.ends_with(".chrparams")
            || lower.ends_with(".cdf")
            || lower.ends_with(".adb")
            || lower.ends_with(".comb")
            || lower.ends_with(".entxml");

        if is_cryxml_ext {
            if let Ok(xml) = starbreaker_cryxml::from_bytes(&data) {
                return format!("{xml}");
            }
            if let Ok(text) = std::str::from_utf8(&data) {
                return text.to_string();
            }
        }

        if let Ok(text) = std::str::from_utf8(&data) {
            return text.to_string();
        }

        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        format!("[base64, {} bytes]\n{encoded}", data.len())
    }

    #[tool(description = "List files and directories under a P4k path. Shows name, compressed/uncompressed size, compression method, and encryption state for each file.")]
    fn p4k_list(&self, Parameters(req): Parameters<P4kListRequest>) -> String {
        let path = if req.path.is_empty() {
            String::new()
        } else {
            Self::normalize_p4k_path(&req.path).trim_end_matches('\\').to_string()
        };

        let entries = self.p4k().list_dir(&path);
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
        let db = self.db();
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
                let _ = writeln!(out, "{:<4} {:<12} {:>10} {:>8} {:>10} {:>10}", "Idx", "Type", "ChunkType", "Version", "Offset", "Size");
                let _ = writeln!(out, "{}", "-".repeat(62));
                for (i, chunk) in ivo.chunks().iter().enumerate() {
                    let name = starbreaker_chunks::known_types::ivo::name(chunk.chunk_type)
                        .unwrap_or("Unknown");
                    let _ = writeln!(out, "{:<4} {:<12} {:#010x} {:>8} {:>#10x} {:>10}",
                        i, name, chunk.chunk_type, chunk.version, chunk.offset, chunk.size);
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

        let mut results: Vec<_> = self
            .p4k()
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

    #[tool(description = "Extract files from the P4k archive to disk. Supports glob patterns (e.g. 'Data/Objects/Spaceships/AEGS/AEGS_Gladius/*'). CryXML files are auto-decoded to XML. Returns a listing of extracted files with sizes.")]
    fn p4k_extract(&self, Parameters(req): Parameters<P4kExtractRequest>) -> String {
        use std::fmt::Write;

        if req.paths.is_empty() {
            return "Error: paths array is empty".to_string();
        }

        let output_dir = std::path::PathBuf::from(
            req.output_dir.as_deref().unwrap_or("tmp/p4k"),
        );

        // Collect all P4k entry paths for glob matching
        let all_entries: Vec<&str> = self.p4k().entries().iter().map(|e| e.name.as_str()).collect();

        // Resolve each input path/glob to matching P4k entries
        let mut matched_paths: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        for pattern in &req.paths {
            let normalized = Self::normalize_p4k_path(pattern);

            // Check if it's a glob pattern
            if normalized.contains('*') || normalized.contains('?') || normalized.contains('[') {
                // Convert backslashes to forward slashes for glob matching, then back
                let glob_pattern = normalized.replace('\\', "/");
                let matcher = match globset::GlobBuilder::new(&glob_pattern)
                    .case_insensitive(true)
                    .literal_separator(false)
                    .build()
                {
                    Ok(g) => match g.compile_matcher() {
                        m => m,
                    },
                    Err(e) => {
                        warnings.push(format!("WARN: invalid glob '{}': {e}", pattern));
                        continue;
                    }
                };

                let mut count = 0;
                for entry_path in &all_entries {
                    let forward = entry_path.replace('\\', "/");
                    if matcher.is_match(&forward) {
                        matched_paths.push(entry_path.to_string());
                        count += 1;
                    }
                }
                if count == 0 {
                    warnings.push(format!("WARN: no matches for '{}'", pattern));
                }
            } else {
                // Exact path — try direct lookup with case-insensitive fallback
                if self.p4k().entry(&normalized).is_some() {
                    matched_paths.push(normalized);
                } else if let Some(entry) = self.p4k().entry_case_insensitive(&normalized) {
                    matched_paths.push(entry.name.clone());
                } else {
                    warnings.push(format!("WARN: not found '{}'", pattern));
                }
            }
        }

        if matched_paths.is_empty() && warnings.is_empty() {
            return "No files matched".to_string();
        }

        // Deduplicate
        matched_paths.sort();
        matched_paths.dedup();

        // CryXML-eligible extensions
        let cryxml_exts = [".xml", ".mtl", ".chrparams", ".cdf", ".adb", ".comb", ".entxml"];

        // Extract files
        let mut results: Vec<(String, u64, bool)> = Vec::new(); // (path, size_on_disk, was_cryxml)
        let mut errors: Vec<String> = Vec::new();
        let mut total_bytes: u64 = 0;

        for p4k_path in &matched_paths {
            let data = match self.p4k().read_file(p4k_path) {
                Ok(d) => d,
                Err(e) => {
                    errors.push(format!("ERROR: {p4k_path}: {e}"));
                    continue;
                }
            };

            // Try CryXML decode for eligible extensions
            let lower = p4k_path.to_lowercase();
            let is_cryxml_ext = cryxml_exts.iter().any(|ext| lower.ends_with(ext));
            let (output_data, was_cryxml) = if is_cryxml_ext {
                if let Ok(xml) = starbreaker_cryxml::from_bytes(&data) {
                    (format!("{xml}").into_bytes(), true)
                } else {
                    (data, false)
                }
            } else {
                (data, false)
            };

            let dest = output_dir.join(p4k_path.replace('\\', "/"));
            if let Some(parent) = dest.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(format!("ERROR: mkdir {}: {e}", parent.display()));
                    continue;
                }
            }

            let size = output_data.len() as u64;
            if let Err(e) = std::fs::write(&dest, &output_data) {
                errors.push(format!("ERROR: write {}: {e}", dest.display()));
                continue;
            }

            total_bytes += size;
            results.push((p4k_path.clone(), size, was_cryxml));
        }

        // Format output
        let mut out = String::new();
        let _ = writeln!(out, "Extracted {} files to {}\n", results.len(), output_dir.display());

        for warning in &warnings {
            let _ = writeln!(out, "  {warning}");
        }
        for error in &errors {
            let _ = writeln!(out, "  {error}");
        }
        if !warnings.is_empty() || !errors.is_empty() {
            let _ = writeln!(out);
        }

        for (path, size, was_cryxml) in &results {
            let annotation = if *was_cryxml { "  (CryXML -> XML)" } else { "" };
            let _ = writeln!(out, "  {:<70} {:>10}{}", path, format_size(*size), annotation);
        }

        let _ = writeln!(out, "\nTotal: {}", format_size(total_bytes));
        out
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

/// Convert a DataCore `Value` to a `serde_json::Value`.
fn value_to_json(v: &starbreaker_datacore::query::value::Value) -> serde_json::Value {
    use starbreaker_datacore::query::value::Value;
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int8(n) => serde_json::json!(*n),
        Value::Int16(n) => serde_json::json!(*n),
        Value::Int32(n) => serde_json::json!(*n),
        Value::Int64(n) => serde_json::json!(*n),
        Value::UInt8(n) => serde_json::json!(*n),
        Value::UInt16(n) => serde_json::json!(*n),
        Value::UInt32(n) => serde_json::json!(*n),
        Value::UInt64(n) => serde_json::json!(*n),
        Value::Float(n) => serde_json::json!(*n),
        Value::Double(n) => serde_json::json!(*n),
        Value::String(s) => serde_json::Value::String(s.to_string()),
        Value::Guid(g) => serde_json::Value::String(format!("{g}")),
        Value::Enum(s) => serde_json::Value::String(s.to_string()),
        Value::Locale(s) => serde_json::Value::String(s.to_string()),
        Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(value_to_json).collect())
        }
        Value::Object {
            type_name, fields, record_id,
        } => {
            let mut map = serde_json::Map::new();
            map.insert("__type".to_string(), serde_json::Value::String(type_name.to_string()));
            if let Some(rid) = record_id {
                map.insert("__id".to_string(), serde_json::Value::String(format!("{rid}")));
            }
            for (key, val) in fields {
                map.insert(key.to_string(), value_to_json(val));
            }
            serde_json::Value::Object(map)
        }
    }
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


