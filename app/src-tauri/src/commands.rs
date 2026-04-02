use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use starbreaker_common::Progress;
use starbreaker_datacore::types::CigGuid;
use starbreaker_p4k::MappedP4k;

use crate::error::AppError;
use crate::state::AppState;

/// Discovery result returned to the frontend.
#[derive(Serialize)]
pub struct DiscoverResult {
    pub path: String,
    pub source: String,
}

/// A directory entry returned to the frontend.
#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum DirEntryDto {
    #[serde(rename = "file")]
    File {
        name: String,
        compressed_size: u64,
        uncompressed_size: u64,
    },
    #[serde(rename = "directory")]
    Directory { name: String },
}

/// Progress event payload.
#[derive(Clone, Serialize)]
pub struct LoadProgress {
    pub fraction: f32,
    pub message: String,
}

/// Discover all Data.p4k installations across channels.
#[tauri::command]
pub fn discover_p4k() -> Vec<DiscoverResult> {
    starbreaker_common::discover::find_all_p4k()
        .into_iter()
        .map(|d| DiscoverResult {
            path: d.path.to_string_lossy().into_owned(),
            source: d.source,
        })
        .collect()
}

/// Open a P4k file and store it in managed state.
/// Also extracts Data\Game2.dcb and caches the bytes.
#[tauri::command]
pub async fn open_p4k(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<usize, AppError> {
    let path_clone = path.clone();
    let app_clone = app.clone();

    // Run the heavy open on a blocking thread with progress polling
    let (mapped, dcb_bytes, loc_map, record_index) = tokio::task::spawn_blocking(move || {
        let progress = std::sync::Arc::new(Progress::new());

        // Poll progress and emit events to the frontend
        let progress_poll = progress.clone();
        let poll_thread = std::thread::spawn(move || {
            loop {
                let (fraction, message) = progress_poll.get();
                let _ = app_clone.emit(
                    "load-progress",
                    LoadProgress { fraction, message },
                );
                if fraction >= 1.0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });

        let mapped = MappedP4k::open_with_progress(&path_clone, Some(&*progress));

        progress.report(1.0, "Done");
        let _ = poll_thread.join();

        // Extract DCB bytes and localization from the P4k
        let p4k = mapped?;
        let dcb_bytes = p4k.read_file("Data\\Game2.dcb")?;
        let loc_data = p4k
            .read_file("Data\\Localization\\english\\global.ini")
            .unwrap_or_default();
        let loc_map = crate::state::parse_localization(&loc_data);
        let record_index = crate::datacore_commands::build_record_index(&dcb_bytes);
        Ok::<_, AppError>((p4k, dcb_bytes, loc_map, record_index))
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
    ?;

    let count = mapped.len();
    let arc_p4k = Arc::new(mapped);

    *state.p4k.lock() = Some(arc_p4k);
    *state.dcb_bytes.lock() = Some(dcb_bytes);
    *state.localization.lock() = loc_map;
    *state.record_index.lock() = Some(record_index);

    Ok(count)
}

/// List only subdirectory names under a path (fast — no file data serialized).
#[tauri::command]
pub fn list_subdirs(state: State<'_, AppState>, path: String) -> Result<Vec<String>, AppError> {
    let guard = state.p4k.lock();
    let p4k = guard.as_ref().ok_or_else(|| AppError::Internal("P4k not loaded".into()))?;

    Ok(p4k.list_subdirs(&path))
}

/// List directory contents from the loaded P4k.
#[tauri::command]
pub fn list_dir(state: State<'_, AppState>, path: String) -> Result<Vec<DirEntryDto>, AppError> {
    let guard = state.p4k.lock();
    let p4k = guard.as_ref().ok_or_else(|| AppError::Internal("P4k not loaded".into()))?;

    let entries = p4k.list_dir(&path);
    let dtos = entries
        .into_iter()
        .map(|e| match e {
            starbreaker_p4k::DirEntry::File(f) => DirEntryDto::File {
                name: f.name.rsplit('\\').next().unwrap_or(&f.name).to_string(),
                compressed_size: f.compressed_size,
                uncompressed_size: f.uncompressed_size,
            },
            starbreaker_p4k::DirEntry::Directory(name) => DirEntryDto::Directory { name },
        })
        .collect();

    Ok(dtos)
}

// ── DataCore / Export DTOs ──────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct EntityDto {
    pub name: String,
    pub id: String,
    /// Localized display name (e.g., "S-38 Pistol"). None if no translation found.
    pub display_name: Option<String>,
    /// True if not a player-available variant (inclusionMode != "ReadyToInclude").
    /// Covers AI, template, unmanned, and other non-player variants.
    pub is_npc_or_internal: bool,
}

#[derive(Clone, Serialize)]
pub struct CategoryDto {
    pub name: String,
    pub entities: Vec<EntityDto>,
}

/// Scan EntityClassDefinition records from the cached DCB and return categorized entities.
#[tauri::command]
pub async fn scan_categories(state: State<'_, AppState>) -> Result<Vec<CategoryDto>, AppError> {
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard.as_ref().ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?.clone()
    };
    let loc = {
        let guard = state.localization.lock();
        guard.clone()
    };

    tokio::task::spawn_blocking(move || {
        let db = starbreaker_datacore::database::Database::from_bytes(&dcb_bytes)?;

        use starbreaker_datacore::QueryResultExt;
        use starbreaker_datacore::query::value::Value;

        // Pre-compile query paths using rooted syntax (StructName.path).
        // .optional() turns TypeFilterMismatch into None (component not in schema),
        // but propagates real errors (typo in path, wrong leaf type, etc.).
        let loc_compiled = db.compile_rooted::<Value>(
            "EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Localization.Name",
        ).optional()?;

        let inclusion_compiled = db.compile_rooted::<Value>(
            "EntityClassDefinition.StaticEntityClassData[EAEntityDataParams].inclusionMode",
        ).optional()?;

        let mut ships = Vec::new();
        let mut ground_vehicles = Vec::new();
        let mut weapons = Vec::new();
        let mut other = Vec::new();

        for record in db.records_by_type_name("EntityClassDefinition") {
            if !db.is_main_record(record) {
                continue;
            }

            let name = db.resolve_string2(record.name_offset).to_string();
            let file_path = db.resolve_string(record.file_name_offset);
            let file_path_lower = file_path.to_lowercase();

            // Look up localized display name from DataCore's localization key.
            // The record stores e.g. "@item_Namebehr_pistol_ballistic_01" — strip
            // the "@" prefix and look up in the INI map.
            let display_name = loc_compiled.as_ref()
                .and_then(|c| db.query_single::<Value>(c, record).ok().flatten())
                .and_then(|v| match v {
                    Value::String(s) | Value::Locale(s) => Some(s.to_string()),
                    other => {
                        eprintln!("WARNING: Localization.Name for {name}: expected String/Locale, got {other:?}");
                        None
                    }
                })
                .filter(|s| !s.is_empty() && s != "@LOC_UNINITIALIZED" && s != "@LOC_EMPTY")
                .and_then(|key| {
                    let stripped = key.strip_prefix('@').unwrap_or(&key);
                    loc.get(&stripped.to_lowercase()).cloned()
                });

            // Non-player variants have inclusionMode != "ReadyToInclude".
            // inclusionMode is a DataCore enum — query as Value.
            // Entities without the component return None (not NPC).
            let is_npc_or_internal = inclusion_compiled.as_ref()
                .and_then(|c| db.query_single::<Value>(c, record).ok().flatten())
                .is_some_and(|v| match v {
                    Value::Enum(s) => s != "ReadyToInclude",
                    _ => false,
                });

            let info = EntityDto {
                name,
                id: format!("{}", record.id),
                display_name,
                is_npc_or_internal,
            };

            if file_path_lower.contains("entities/spaceships") {
                ships.push(info);
            } else if file_path_lower.contains("entities/groundvehicles") {
                ground_vehicles.push(info);
            } else if file_path_lower.contains("weapon") {
                weapons.push(info);
            } else {
                other.push(info);
            }
        }

        // Sort by display name when available, fall back to DataCore name
        let sort_key = |e: &EntityDto| {
            e.display_name.clone().unwrap_or_else(|| e.name.clone())
        };
        ships.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        ground_vehicles.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        weapons.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        other.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));

        Ok::<_, AppError>(vec![
            CategoryDto {
                name: "Ships".to_string(),
                entities: ships,
            },
            CategoryDto {
                name: "Ground Vehicles".to_string(),
                entities: ground_vehicles,
            },
            CategoryDto {
                name: "Weapons".to_string(),
                entities: weapons,
            },
            CategoryDto {
                name: "Other".to_string(),
                entities: other,
            },
        ])
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

// ── Export commands ──────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ExportProgress {
    pub current: usize,
    pub total: usize,
    pub entity_name: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ExportDone {
    pub success: usize,
    pub errors: usize,
}

#[derive(Deserialize)]
pub struct ExportRequest {
    pub record_ids: Vec<String>,
    pub names: Vec<String>,
    pub output_dir: String,
    pub lod: u32,
    pub mip: u32,
    pub include_textures: bool,
    pub include_interior: bool,
    pub include_normals: bool,
    pub include_lights: bool,
    pub include_tangents: bool,
    pub include_materials: bool,
    pub experimental_textures: bool,
}

/// Start exporting selected entities to GLB files.
#[tauri::command]
pub async fn start_export(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ExportRequest,
) -> Result<(), AppError> {
    // Reset cancel flag
    state.export_cancel.store(false, Ordering::SeqCst);

    // Clone data out of state
    let p4k = {
        let guard = state.p4k.lock();
        guard.as_ref().ok_or_else(|| AppError::Internal("P4k not loaded".into()))?.clone()
    };
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard.as_ref().ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?.clone()
    };
    let cancel = state.export_cancel.clone();

    // Parse record IDs upfront
    let record_ids: Vec<CigGuid> = request
        .record_ids
        .iter()
        .map(|s| s.parse::<CigGuid>())
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let opts = starbreaker_gltf::ExportOptions {
        include_textures: request.include_textures,
        texture_mip: request.mip,
        lod_level: request.lod,
        include_interior: request.include_interior,
        include_normals: request.include_normals,
        include_lights: request.include_lights,
        include_tangents: request.include_tangents,
        include_materials: request.include_materials,
        experimental_textures: request.experimental_textures,
    };

    let names = request.names;
    let output_dir = request.output_dir;

    tokio::task::spawn_blocking(move || {
        let db = match starbreaker_datacore::database::Database::from_bytes(&dcb_bytes) {
            Ok(db) => db,
            Err(_) => {
                let _ = app.emit(
                    "export-done",
                    ExportDone {
                        success: 0,
                        errors: record_ids.len(),
                    },
                );
                return;
            }
        };

        let total = record_ids.len();
        let success = AtomicUsize::new(0);
        let errors = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);

        // Use a dedicated thread pool capped at half the CPU cores to avoid
        // melting the system — each export is memory-heavy (mesh + DDS + PNG).
        let num_threads = (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) / 2).max(2);
        let pool = match rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build()
        {
            Ok(pool) => pool,
            Err(e) => {
                let _ = app.emit(
                    "export-done",
                    ExportDone {
                        success: 0,
                        errors: total,
                    },
                );
                eprintln!("failed to build thread pool: {e}");
                return;
            }
        };

        pool.install(|| {
        use rayon::prelude::*;
        record_ids.par_iter().zip(names.par_iter()).for_each(|(record_id, name)| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }

            let i = completed.fetch_add(1, Ordering::Relaxed);
            let _ = app.emit(
                "export-progress",
                ExportProgress {
                    current: i + 1,
                    total,
                    entity_name: name.clone(),
                    error: None,
                },
            );

            let filename = format!("{}.glb", sanitize_filename(name));
            let output_path = std::path::PathBuf::from(&output_dir).join(&filename);

            match export_single(&db, &p4k, record_id, &output_path, &opts) {
                Ok(()) => { success.fetch_add(1, Ordering::Relaxed); }
                Err(e) => {
                    let _ = app.emit(
                        "export-progress",
                        ExportProgress {
                            current: i + 1,
                            total,
                            entity_name: name.clone(),
                            error: Some(format!("{name}: {e}")),
                        },
                    );
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        }); // pool.install

        let _ = app.emit("export-done", ExportDone {
            success: success.load(Ordering::Relaxed),
            errors: errors.load(Ordering::Relaxed),
        });
    });

    Ok(())
}

/// Cancel an in-progress export.
#[tauri::command]
pub fn cancel_export(state: State<'_, AppState>) {
    state.export_cancel.store(true, Ordering::SeqCst);
}

/// Export a single entity to a GLB file.
fn export_single(
    db: &starbreaker_datacore::database::Database,
    p4k: &MappedP4k,
    record_id: &CigGuid,
    output_path: &Path,
    opts: &starbreaker_gltf::ExportOptions,
) -> Result<(), AppError> {
    let record = db
        .record_by_id(record_id)
        .ok_or_else(|| AppError::Internal("record not found".into()))?;
    let idx = starbreaker_datacore::loadout::EntityIndex::new(db);
    let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);
    let result = starbreaker_gltf::assemble_glb_with_loadout(db, p4k, record, &tree, opts)?;
    std::fs::write(output_path, &result.glb)?;
    Ok(())
}

/// Sanitize a filename by replacing invalid characters with underscores.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

/// Generate a GLB preview for a geometry file in the P4K.
/// Accepts .skin, .skinm, .cgf, .cgfm, .cga, .chr paths.
/// Companion files (.skinm/.cgfm) are resolved to their primary (.skin/.cgf).
#[tauri::command]
pub fn preview_geometry(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<u8>, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    // Resolve companion: .skinm -> .skin, .cgfm -> .cgf
    let primary = if path.ends_with('m')
        && (path.ends_with(".skinm") || path.ends_with(".cgfm"))
    {
        path[..path.len() - 1].to_string()
    } else {
        path.clone()
    };

    // Try reading the companion file first (has vertex data), fall back to primary
    let companion = format!("{primary}m");
    let data = p4k
        .read_file(&companion)
        .or_else(|_| p4k.read_file(&primary))?;

    let glb = starbreaker_gltf::skin_to_glb(&data)?;
    Ok(glb)
}

/// Decode a CryXMLB file from the P4K and return it as formatted XML text.
#[tauri::command]
pub fn preview_xml(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    let data = p4k.read_file(&path)?;

    // Try CryXMLB decode first, fall back to raw UTF-8
    if starbreaker_cryxml::is_cryxmlb(&data) {
        let cryxml = starbreaker_cryxml::from_bytes(&data)?;
        Ok(format!("{cryxml}"))
    } else {
        Ok(String::from_utf8_lossy(&data).into_owned())
    }
}

/// Decode a DDS texture from the P4K and return it as PNG bytes.
/// Uses mip level 2 (1/4 resolution) for fast preview.
#[tauri::command]
pub fn preview_dds(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<u8>, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    let data = p4k.read_file(&path)?;
    let dds = starbreaker_dds::DdsFile::from_bytes(&data)?;

    // Use mip level 2 if available, otherwise highest available
    let mip = std::cmp::min(2, dds.mip_count().saturating_sub(1));
    let (width, height) = dds.dimensions(mip);
    let rgba = dds.decode_rgba(mip)?;

    // Encode as PNG
    let mut png_buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
    image::ImageEncoder::write_image(
        encoder,
        &rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(png_buf)
}
