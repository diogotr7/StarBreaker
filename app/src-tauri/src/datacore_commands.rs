use std::collections::BTreeSet;

use serde::Serialize;
use tauri::State;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::enums::{ConversionType, DataType};
use starbreaker_datacore::reader::SpanReader;
use starbreaker_datacore::types::{CigGuid, Pointer, Reference};

use crate::error::AppError;
use crate::state::AppState;

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct SearchResultDto {
    pub name: String,
    pub struct_type: String,
    pub path: String,
    pub id: String,
}

#[derive(Clone, Serialize)]
pub struct BacklinkDto {
    pub name: String,
    pub id: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind")]
pub enum TreeEntryDto {
    #[serde(rename = "folder")]
    Folder { name: String },
    #[serde(rename = "record")]
    Record {
        name: String,
        struct_type: String,
        id: String,
    },
}

#[derive(Clone, Serialize)]
pub struct RecordDto {
    pub name: String,
    pub struct_type: String,
    pub path: String,
    pub id: String,
    /// Full record data as JSON string (parsed by frontend into collapsible tree).
    pub json: String,
}

// ── Record index (built during loading) ──────────────────────────────────────

pub struct RecordEntry {
    pub name: String,
    pub name_lower: String,
    pub struct_type: String,
    pub path: String,
    pub id: String,
}

/// Build a lightweight index of all main records from the DCB bytes.
pub fn build_record_index(dcb_bytes: &[u8]) -> Vec<RecordEntry> {
    let db = match starbreaker_datacore::database::Database::from_bytes(dcb_bytes) {
        Ok(db) => db,
        Err(_) => return Vec::new(),
    };

    db.records()
        .iter()
        .filter(|r| db.is_main_record(r))
        .map(|record| {
            let record_name = db.resolve_string2(record.name_offset);
            let struct_type = db
                .resolve_string2(db.struct_def(record.struct_index).name_offset)
                .to_string();
            let path = db.resolve_string(record.file_name_offset).to_string();
            let full_name = format!("{struct_type}.{record_name}");
            let name_lower = full_name.to_lowercase();
            let id = format!("{}", record.id);

            RecordEntry {
                name: full_name,
                name_lower,
                struct_type,
                path,
                id,
            }
        })
        .collect()
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Search records by name substring. Returns up to 500 results.
#[tauri::command]
pub fn dc_search(state: State<'_, AppState>, query: String) -> Vec<SearchResultDto> {
    let guard = state.record_index.lock();
    let index = match guard.as_ref() {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let query_lower = query.to_lowercase();

    index
        .iter()
        .filter(|entry| {
            if query_lower.is_empty() {
                true
            } else {
                entry.name_lower.contains(&query_lower)
            }
        })
        .take(500)
        .map(|entry| SearchResultDto {
            name: entry.name.clone(),
            struct_type: entry.struct_type.clone(),
            path: entry.path.clone(),
            id: entry.id.clone(),
        })
        .collect()
}

/// List tree entries (folders + records) at a given path.
/// Empty path returns the root level.
#[tauri::command]
pub fn dc_list_tree(state: State<'_, AppState>, path: String) -> Vec<TreeEntryDto> {
    let guard = state.record_index.lock();
    let index = match guard.as_ref() {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let normalized = if path.is_empty() {
        String::new()
    } else if path.ends_with('/') {
        path.clone()
    } else {
        format!("{path}/")
    };

    let mut folders = BTreeSet::new();
    let mut records = Vec::new();

    for entry in index.iter() {
        if normalized.is_empty() {
            // Root level: first segment of each path
            if let Some(first_slash) = entry.path.find('/') {
                folders.insert(entry.path[..first_slash].to_string());
            } else if !entry.path.is_empty() {
                // Record at root level (path is just the filename)
                records.push(TreeEntryDto::Record {
                    name: entry.name.clone(),
                    struct_type: entry.struct_type.clone(),
                    id: entry.id.clone(),
                });
            }
        } else if let Some(rest) = entry.path.strip_prefix(&normalized) {
            if let Some(next_slash) = rest.find('/') {
                folders.insert(rest[..next_slash].to_string());
            } else {
                // Record at this exact level
                records.push(TreeEntryDto::Record {
                    name: entry.name.clone(),
                    struct_type: entry.struct_type.clone(),
                    id: entry.id.clone(),
                });
            }
        }
    }

    let mut result: Vec<TreeEntryDto> = folders
        .into_iter()
        .map(|name| TreeEntryDto::Folder { name })
        .collect();

    records.sort_by(|a, b| {
        let name_a = match a {
            TreeEntryDto::Record { name, .. } => name,
            TreeEntryDto::Folder { name } => name,
        };
        let name_b = match b {
            TreeEntryDto::Record { name, .. } => name,
            TreeEntryDto::Folder { name } => name,
        };
        name_a.cmp(name_b)
    });

    result.extend(records);
    result
}

/// Get a record's full data as JSON for the property inspector.
#[tauri::command]
pub async fn dc_get_record(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<RecordDto, AppError> {
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard.as_ref().ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?.clone()
    };

    let record_id_clone = record_id.clone();

    tokio::task::spawn_blocking(move || {
        let db = starbreaker_datacore::database::Database::from_bytes(&dcb_bytes)?;

        let guid: starbreaker_datacore::types::CigGuid = record_id_clone.parse()?;

        let record = db
            .record_by_id(&guid)
            .ok_or_else(|| AppError::Internal("record not found".into()))?;

        let name = db.resolve_string2(record.name_offset).to_string();
        let struct_type = db
            .resolve_string2(db.struct_def(record.struct_index).name_offset)
            .to_string();
        let path = db.resolve_string(record.file_name_offset).to_string();

        let mut buf = Vec::new();
        let mut sink = crate::ui_sink::UiJsonSink::new(&mut buf);
        starbreaker_datacore::walker::walk_record(&db, record, &mut sink)?;
        let json = String::from_utf8(buf)?;

        Ok::<_, AppError>(RecordDto {
            name: format!("{struct_type}.{name}"),
            struct_type,
            path,
            id: format!("{}", record.id),
            json,
        })
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

/// Export a record as JSON, saving to the given path.
#[tauri::command]
pub async fn dc_export_json(
    state: State<'_, AppState>,
    record_id: String,
    output_path: String,
) -> Result<(), AppError> {
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard.as_ref().ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let db = starbreaker_datacore::database::Database::from_bytes(&dcb_bytes)?;
        let guid: starbreaker_datacore::types::CigGuid = record_id.parse()?;
        let record = db
            .record_by_id(&guid)
            .ok_or_else(|| AppError::Internal("record not found".into()))?;
        let json_bytes = starbreaker_datacore::export::to_json(&db, record)?;
        std::fs::write(&output_path, &json_bytes)?;
        Ok::<_, AppError>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

/// Get records that reference the given record (backlinks).
#[tauri::command]
pub async fn dc_get_backlinks(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<Vec<BacklinkDto>, AppError> {
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard.as_ref().ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let db = Database::from_bytes(&dcb_bytes)?;

        let target_guid: CigGuid = record_id.parse()?;

        let mut backlinks = Vec::new();

        for record in db.records() {
            if !db.is_main_record(record) {
                continue;
            }
            if record.id == target_guid {
                continue;
            }

            let mut refs = Vec::new();
            collect_references(&db, record.struct_index, record.instance_index as i32, &mut refs);

            if refs.iter().any(|id| *id == target_guid) {
                let name = db.resolve_string2(record.name_offset);
                let struct_type =
                    db.resolve_string2(db.struct_def(record.struct_index).name_offset);
                backlinks.push(BacklinkDto {
                    name: format!("{struct_type}.{name}"),
                    id: format!("{}", record.id),
                });
            }
        }

        backlinks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok::<_, AppError>(backlinks)
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

/// Export a record as XML, saving to the given path.
#[tauri::command]
pub async fn dc_export_xml(
    state: State<'_, AppState>,
    record_id: String,
    output_path: String,
) -> Result<(), AppError> {
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard.as_ref().ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let db = Database::from_bytes(&dcb_bytes)?;
        let guid: CigGuid = record_id.parse()?;
        let record = db
            .record_by_id(&guid)
            .ok_or_else(|| AppError::Internal("record not found".into()))?;
        let xml_bytes = starbreaker_datacore::export::to_xml(&db, record)?;
        std::fs::write(&output_path, &xml_bytes)?;
        Ok::<_, AppError>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

// ── Backlink reference collection ────────────────────────────────────────────

fn collect_references(db: &Database, struct_index: i32, instance_index: i32, out: &mut Vec<CigGuid>) {
    let instance_bytes = db.get_instance(struct_index, instance_index);
    let mut reader = SpanReader::new(instance_bytes);
    collect_struct_refs(db, struct_index, &mut reader, out);
}

fn collect_struct_refs(db: &Database, struct_index: i32, reader: &mut SpanReader, out: &mut Vec<CigGuid>) {
    let prop_indices = db.all_property_indices(struct_index);
    let props = db.property_defs();
    for &pi in prop_indices {
        let prop = &props[pi as usize];
        let Ok(dt) = DataType::try_from(prop.data_type) else { continue };
        let Ok(ct) = ConversionType::try_from(prop.conversion_type) else { continue };
        if ct == ConversionType::Attribute {
            collect_attr_refs(db, dt, prop.struct_index as i32, reader, out);
        } else {
            collect_array_refs(db, dt, prop.struct_index as i32, reader, out);
        }
    }
}

fn collect_attr_refs(db: &Database, dt: DataType, prop_struct_index: i32, reader: &mut SpanReader, out: &mut Vec<CigGuid>) {
    match dt {
        DataType::Reference => {
            if let Ok(reference) = reader.read_type::<Reference>() {
                if !reference.is_null() {
                    out.push(reference.record_id);
                }
            }
        }
        DataType::Class => {
            collect_struct_refs(db, prop_struct_index, reader, out);
        }
        DataType::StrongPointer => {
            if let Ok(ptr) = reader.read_type::<Pointer>() {
                if !ptr.is_null() {
                    collect_references(db, ptr.struct_index, ptr.instance_index, out);
                }
            }
        }
        other => {
            let _ = reader.advance(other.inline_size());
        }
    }
}

fn collect_array_refs(db: &Database, dt: DataType, prop_struct_index: i32, reader: &mut SpanReader, out: &mut Vec<CigGuid>) {
    let count = reader.read_i32().unwrap_or(0);
    let first = reader.read_i32().unwrap_or(0);
    match dt {
        DataType::Reference => {
            for i in first..first + count {
                let reference = &db.reference_values[i as usize];
                if !reference.is_null() {
                    out.push(reference.record_id);
                }
            }
        }
        DataType::Class => {
            for i in first..first + count {
                collect_references(db, prop_struct_index, i, out);
            }
        }
        DataType::StrongPointer => {
            for i in first..first + count {
                let ptr = &db.strong_values[i as usize];
                if !ptr.is_null() {
                    collect_references(db, ptr.struct_index, ptr.instance_index, out);
                }
            }
        }
        _ => {}
    }
}
