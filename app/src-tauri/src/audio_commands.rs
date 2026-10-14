use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use starbreaker_p4k::MappedP4k;
use starbreaker_wwise::{AtlIndex, BnkFile, ExternalSourceEntry, ExternalSourceIndex, Hierarchy, SoundSource};

use crate::error::AppError;
use crate::state::AppState;

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AudioInitResult {
    pub trigger_count: usize,
    pub bank_count: usize,
    pub external_source_count: usize,
}

#[derive(Serialize)]
pub struct BankResult {
    pub name: String,
    pub trigger_count: usize,
}

#[derive(Serialize)]
pub struct EntityResult {
    pub name: String,
    pub record_path: String,
    pub trigger_count: usize,
}

#[derive(Serialize)]
pub struct TriggerResult {
    pub trigger_name: String,
    pub bank_name: String,
    pub duration_type: String,
    pub radius_max: Option<f32>,
}

#[derive(Serialize)]
pub struct TriggerDetail {
    pub trigger_name: String,
    pub bank_name: String,
    pub duration_type: String,
    pub sound_count: usize,
}

#[derive(Serialize)]
pub struct SoundResult {
    pub media_id: u32,
    pub label: String,
    pub source_type: String,
    pub bank_name: String,
    pub path_description: String,
    pub media_path: Option<String>,
    pub playable: bool,
}

#[derive(Serialize)]
pub struct AudioExportInfo {
    pub extension: String,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn ensure_atl(state: &AppState) -> Result<(), AppError> {
    // Fast path: already built
    if state.atl_index.lock().is_some() {
        return Ok(());
    }

    // Build outside any lock (expensive I/O)
    let p4k = get_p4k(state)?;
    let atl = AtlIndex::from_p4k(&p4k)?;

    // Store (another thread may have beaten us — that's fine, just overwrite)
    *state.atl_index.lock() = Some(atl);
    Ok(())
}

fn ensure_external_sources(state: &AppState) -> Result<(), AppError> {
    if state.external_source_index.lock().is_some() {
        return Ok(());
    }

    let p4k = get_p4k(state)?;
    let external_sources = ExternalSourceIndex::from_p4k(&p4k)?;
    *state.external_source_index.lock() = Some(external_sources);
    Ok(())
}

fn get_p4k(state: &AppState) -> Result<Arc<MappedP4k>, AppError> {
    state
        .p4k
        .lock()
        .clone()
        .ok_or_else(|| AppError::Internal("P4k not loaded".into()))
}

fn load_hierarchy(
    p4k: &MappedP4k,
    bank_name: &str,
    cache: &mut HashMap<String, Option<Arc<Hierarchy>>>,
    wwise_paths: &HashMap<String, String>,
) -> Option<Arc<Hierarchy>> {
    if let Some(cached) = cache.get(bank_name) {
        return cached.clone();
    }
    let result = (|| {
        // Use the bank path index to find the full P4k path, falling back to the root wwise dir
        let path = wwise_paths
            .get(bank_name)
            .cloned()
            .unwrap_or_else(|| format!("Data\\Sounds\\wwise\\{bank_name}"));
        let data = match p4k.read_file(&path) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("failed to read {path}: {e}");
                return None;
            }
        };
        let bnk = match BnkFile::parse(&data) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("failed to parse {path}: {e}");
                return None;
            }
        };
        let hirc = bnk.hirc.as_ref()?;
        Some(Arc::new(Hierarchy::from_section(hirc)))
    })();
    cache.insert(bank_name.to_string(), result.clone());
    result
}

/// Build a map of bank filename -> full P4k path by scanning the archive.
/// Build a map of filename -> full P4k path for all .bnk and .wem files under wwise/.
fn build_wwise_path_index(p4k: &MappedP4k) -> HashMap<String, String> {
    let mut index = HashMap::new();
    for entry in p4k.entries() {
        let path: &str = &entry.name;
        if path.starts_with("Data\\Sounds\\wwise\\")
            && (path.ends_with(".bnk") || path.ends_with(".wem"))
        {
            if let Some(filename) = path.rsplit('\\').next() {
                index.insert(filename.to_string(), path.to_string());
            }
        }
    }
    index
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MediaSourceInfo {
    source_type: String,
    playable: bool,
    description: String,
}

fn hirc_source_name(source: SoundSource) -> &'static str {
    match source {
        SoundSource::Embedded => "Embedded",
        SoundSource::PrefetchStream => "PrefetchStream",
        SoundSource::Stream => "Stream",
    }
}

fn has_loose_wem(media_id: u32, wwise_paths: &HashMap<String, String>) -> bool {
    wwise_paths.contains_key(&format!("{media_id}.wem"))
}

fn classify_media_source(
    media_id: u32,
    hirc_source: Option<SoundSource>,
    embedded_ids: &HashSet<u32>,
    wwise_paths: &HashMap<String, String>,
) -> MediaSourceInfo {
    if embedded_ids.contains(&media_id) {
        return MediaSourceInfo {
            source_type: "Embedded".to_string(),
            playable: true,
            description: "embedded WEM data in bank".to_string(),
        };
    }

    if has_loose_wem(media_id, wwise_paths) {
        return MediaSourceInfo {
            source_type: "MediaFile".to_string(),
            playable: true,
            description: format!("Data\\Sounds\\wwise\\Media\\{media_id}.wem"),
        };
    }

    let source = hirc_source.map(hirc_source_name).unwrap_or("Embedded");
    let description = match source {
        "Embedded" => {
            "referenced by HIRC as Embedded, but this bank has no embedded DIDX/DATA entry and no loose Media WEM was found"
        }
        "PrefetchStream" | "Stream" => {
            "referenced by HIRC as streamed media, but no loose Media WEM was found"
        }
        _ => "referenced by HIRC, but no backing WEM data was found",
    };

    MediaSourceInfo {
        source_type: "Unavailable".to_string(),
        playable: false,
        description: description.to_string(),
    }
}

fn embedded_media_ids(bnk: &BnkFile<'_>) -> HashSet<u32> {
    bnk.data_index.iter().map(|entry| entry.id).collect()
}

fn make_sound_result(
    media_id: u32,
    hirc_source: Option<SoundSource>,
    bank_name: &str,
    path_description: String,
    embedded_ids: &HashSet<u32>,
    wwise_paths: &HashMap<String, String>,
) -> SoundResult {
    let info = classify_media_source(media_id, hirc_source, embedded_ids, wwise_paths);
    let path_description = if path_description.is_empty() {
        info.description
    } else {
        format!("{path_description}; {}", info.description)
    };

    SoundResult {
        media_id,
        label: media_id.to_string(),
        source_type: info.source_type,
        bank_name: bank_name.to_string(),
        path_description,
        media_path: None,
        playable: info.playable,
    }
}

fn make_external_source_result(source: &ExternalSourceEntry, p4k: &MappedP4k) -> SoundResult {
    let playable = p4k.entry_case_insensitive(&source.p4k_path).is_some();
    let duration = source
        .duration_max
        .map(|duration| format!(" ({duration:.2}s)"))
        .unwrap_or_default();
    let description = format!(
        "{} -> {}{}",
        source.name, source.p4k_path, duration
    );

    SoundResult {
        media_id: 0,
        label: source.name.clone(),
        source_type: "ExternalSource".to_string(),
        bank_name: source.language.clone(),
        path_description: description,
        media_path: Some(source.p4k_path.clone()),
        playable,
    }
}

fn read_media_path(p4k: &MappedP4k, path: &str) -> Result<Vec<u8>, AppError> {
    Ok(p4k.read_file(path)?)
}

fn read_streamed_wem(
    p4k: &MappedP4k,
    media_id: u32,
    wwise_paths: &HashMap<String, String>,
) -> Result<Vec<u8>, AppError> {
    let filename = format!("{media_id}.wem");
    let path = match wwise_paths.get(&filename) {
        Some(path) => path.clone(),
        None => {
            let fallback = format!("Data\\Sounds\\wwise\\Media\\{media_id}.wem");
            if p4k.entry_case_insensitive(&fallback).is_some() {
                fallback
            } else {
                return Err(AppError::Internal(format!(
                    "media {media_id} has no loose WEM file under Data\\Sounds\\wwise\\Media"
                )));
            }
        }
    };
    Ok(p4k.read_file(&path)?)
}

fn read_embedded_wem(
    p4k: &MappedP4k,
    media_id: u32,
    bank_name: &str,
    wwise_paths: &HashMap<String, String>,
) -> Result<Vec<u8>, AppError> {
    let bank_path = wwise_paths
        .get(bank_name)
        .cloned()
        .unwrap_or_else(|| format!("Data\\Sounds\\wwise\\{}", bank_name));
    let bank_data = p4k.read_file(&bank_path)?;
    let bnk = BnkFile::parse(&bank_data)?;
    let entry_data = bnk.wem_data_by_id(media_id)?;
    Ok(entry_data.to_vec())
}

fn load_media_bytes(
    p4k: &MappedP4k,
    media_id: u32,
    source_type: &str,
    bank_name: &str,
    wwise_paths: &HashMap<String, String>,
    media_path: Option<&str>,
) -> Result<Vec<u8>, AppError> {
    if let Some(path) = media_path {
        return read_media_path(p4k, path);
    }

    match source_type {
        "ExternalSource" => Err(AppError::Internal(format!(
            "external source {bank_name} did not include a WEM path"
        ))),
        "MediaFile" | "Stream" | "PrefetchStream" => read_streamed_wem(p4k, media_id, wwise_paths),
        "Embedded" => match read_embedded_wem(p4k, media_id, bank_name, wwise_paths) {
            Ok(bytes) => Ok(bytes),
            Err(_) => read_streamed_wem(p4k, media_id, wwise_paths),
        },
        "Unavailable" => Err(AppError::Internal(format!(
            "media {media_id} is referenced by {bank_name}, but no playable WEM data is available"
        ))),
        other => Err(AppError::Internal(format!("unknown source type: {other}"))),
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn audio_init(state: State<'_, AppState>) -> Result<AudioInitResult, AppError> {
    ensure_atl(&state)?;
    ensure_external_sources(&state)?;

    // Build bank path index if not already done
    {
        let mut wp = state.wwise_paths.lock();
        if wp.is_empty() {
            let p4k = get_p4k(&state)?;
            *wp = build_wwise_path_index(&p4k);
        }
    }

    let atl_guard = state.atl_index.lock();
    let atl = atl_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio not initialized".into()))?;
    let external_guard = state.external_source_index.lock();
    let external_sources = external_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio external sources not initialized".into()))?;
    Ok(AudioInitResult {
        trigger_count: atl.len(),
        bank_count: atl.bank_names().len(),
        external_source_count: external_sources.len(),
    })
}

#[tauri::command]
pub fn audio_search_entities(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<EntityResult>, AppError> {
    let dcb_bytes = state
        .dcb_bytes
        .lock()
        .clone()
        .ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?;

    let db = starbreaker_datacore::database::Database::from_bytes(&dcb_bytes)?;

    let entities = starbreaker_wwise::datacore_audio::search_entities_with_audio(&db, &query);

    Ok(entities
        .into_iter()
        .take(500)
        .map(|e| EntityResult {
            name: e.entity_name,
            record_path: e.record_path,
            trigger_count: e.triggers.len(),
        })
        .collect())
}

#[tauri::command]
pub fn audio_search_triggers(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<TriggerResult>, AppError> {
    ensure_atl(&state)?;
    let atl_guard = state.atl_index.lock();
    let atl = atl_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

    Ok(atl
        .search(&query)
        .into_iter()
        .take(1000)
        .map(|t| TriggerResult {
            trigger_name: t.trigger_name.clone(),
            bank_name: t.bank_name.clone(),
            duration_type: t.duration_type.clone(),
            radius_max: t.radius_max,
        })
        .collect())
}

#[tauri::command]
pub fn audio_search_external_sources(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<TriggerDetail>, AppError> {
    ensure_external_sources(&state)?;
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let external_guard = state.external_source_index.lock();
    let external_sources = external_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio external sources not initialized".into()))?;

    Ok(external_sources
        .search(&query)
        .into_iter()
        .take(1000)
        .map(|source| TriggerDetail {
            trigger_name: source.name.clone(),
            bank_name: source.language.clone(),
            duration_type: source.duration_type.clone(),
            sound_count: 1,
        })
        .collect())
}

#[tauri::command]
pub fn audio_list_banks(state: State<'_, AppState>) -> Result<Vec<BankResult>, AppError> {
    ensure_atl(&state)?;
    let atl_guard = state.atl_index.lock();
    let atl = atl_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

    let mut banks: Vec<BankResult> = atl
        .bank_names()
        .into_iter()
        .map(|name| BankResult {
            trigger_count: atl.triggers_for_bank(name).len(),
            name: name.to_string(),
        })
        .collect();
    banks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(banks)
}

#[tauri::command]
pub fn audio_bank_triggers(
    state: State<'_, AppState>,
    bank_name: String,
) -> Result<Vec<TriggerDetail>, AppError> {
    ensure_atl(&state)?;
    let atl_guard = state.atl_index.lock();
    let atl = atl_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

    let trigger_names = atl.triggers_for_bank(&bank_name);
    let p4k = get_p4k(&state)?;
    let mut cache_guard = state.bank_cache.lock();
    let wp = state.wwise_paths.lock().clone();

    let mut results = Vec::new();
    for name in trigger_names {
        let (duration_type, sound_count) = match atl.get_trigger(name) {
            Some(trigger) => {
                let count = load_hierarchy(&p4k, &trigger.bank_name, &mut cache_guard, &wp)
                    .map(|h| h.resolve_event_by_name(&trigger.wwise_event_name).len())
                    .unwrap_or(0);
                (trigger.duration_type.clone(), count)
            }
            None => (String::new(), 0),
        };
        results.push(TriggerDetail {
            trigger_name: name.to_string(),
            bank_name: bank_name.clone(),
            duration_type,
            sound_count,
        });
    }
    Ok(results)
}

/// List all media in a bank. Tries three sources:
/// 1. HIRC Sound + MusicTrack objects (event-independent)
/// 2. DIDX embedded WEM entries (for banks with no HIRC sounds, e.g. music data banks)
/// 3. Falls back to empty if the bank can't be loaded at all
#[tauri::command]
pub fn audio_bank_media(
    state: State<'_, AppState>,
    bank_name: String,
) -> Result<Vec<SoundResult>, AppError> {
    let p4k = get_p4k(&state)?;
    let wp = state.wwise_paths.lock().clone();

    let bank_path = wp
        .get(&bank_name)
        .cloned()
        .unwrap_or_else(|| format!("Data\\Sounds\\wwise\\{}", bank_name));
    let bank_data = p4k.read_file(&bank_path)?;
    let bnk = BnkFile::parse(&bank_data)?;
    let embedded_ids = embedded_media_ids(&bnk);

    let mut seen = HashSet::new();
    let mut results = Vec::new();

    // 1. Scan HIRC for Sound + MusicTrack objects
    if let Some(hirc) = &bnk.hirc {
        let hierarchy = Hierarchy::from_section(hirc);
        for s in hierarchy.all_media() {
            if seen.insert(s.media_id) {
                results.push(make_sound_result(
                    s.media_id,
                    Some(s.source),
                    &bank_name,
                    String::new(),
                    &embedded_ids,
                    &wp,
                ));
            }
        }
    }

    // 2. Scan DIDX for embedded WEM entries not already found via HIRC
    for entry in &bnk.data_index {
        let id = entry.id;
        if seen.insert(id) {
            results.push(make_sound_result(
                id,
                None,
                &bank_name,
                String::new(),
                &embedded_ids,
                &wp,
            ));
        }
    }

    // If no media found and bank name ends with _Events, try companion data bank
    if results.is_empty() {
        let companion = if bank_name.ends_with("_Events.bnk") {
            Some(bank_name.replace("_Events.bnk", ".bnk"))
        } else {
            None
        };
        if let Some(companion_name) = companion {
            let companion_path = wp
                .get(&companion_name)
                .cloned()
                .unwrap_or_else(|| format!("Data\\Sounds\\wwise\\{}", companion_name));
            let data = match p4k.read_file(&companion_path) {
                Ok(d) => d,
                Err(e) => {
                    log::warn!("failed to read companion bank {companion_path}: {e}");
                    return Ok(results);
                }
            };
            match BnkFile::parse(&data) {
                Ok(cbnk) => {
                    let companion_embedded_ids = embedded_media_ids(&cbnk);
                    if let Some(hirc) = &cbnk.hirc {
                        let hierarchy = Hierarchy::from_section(hirc);
                        for s in hierarchy.all_media() {
                            if seen.insert(s.media_id) {
                                results.push(make_sound_result(
                                    s.media_id,
                                    Some(s.source),
                                    &companion_name,
                                    String::new(),
                                    &companion_embedded_ids,
                                    &wp,
                                ));
                            }
                        }
                    }
                    for entry in &cbnk.data_index {
                        if seen.insert(entry.id) {
                            results.push(make_sound_result(
                                entry.id,
                                None,
                                &companion_name,
                                String::new(),
                                &companion_embedded_ids,
                                &wp,
                            ));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("failed to parse companion bank {companion_path}: {e}");
                }
            }
        }
    }

    results.sort_by_key(|s| s.media_id);
    Ok(results)
}

#[tauri::command]
pub fn audio_entity_triggers(
    state: State<'_, AppState>,
    entity_name: String,
) -> Result<Vec<TriggerDetail>, AppError> {
    let dcb_bytes = state
        .dcb_bytes
        .lock()
        .clone()
        .ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?;
    let db = starbreaker_datacore::database::Database::from_bytes(&dcb_bytes)?;

    let entities = starbreaker_wwise::datacore_audio::search_entities_with_audio(&db, &entity_name);

    let entity = entities
        .iter()
        .find(|e| e.entity_name == entity_name)
        .ok_or_else(|| AppError::Internal(format!("entity '{}' not found", entity_name)))?;

    ensure_atl(&state)?;
    let atl_guard = state.atl_index.lock();
    let atl = atl_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

    let p4k = get_p4k(&state)?;
    let mut cache_guard = state.bank_cache.lock();
    let wp = state.wwise_paths.lock().clone();

    let mut results = Vec::new();
    for tref in &entity.triggers {
        let (bank_name, sound_count) = match atl.get_trigger(&tref.trigger_name) {
            Some(trigger) => {
                let count = load_hierarchy(&p4k, &trigger.bank_name, &mut cache_guard, &wp)
                    .map(|h| h.resolve_event_by_name(&trigger.wwise_event_name).len())
                    .unwrap_or(0);
                (trigger.bank_name.clone(), count)
            }
            None => ("?".to_string(), 0),
        };

        let duration_type = atl
            .get_trigger(&tref.trigger_name)
            .map(|t| t.duration_type.clone())
            .unwrap_or_default();

        results.push(TriggerDetail {
            trigger_name: tref.trigger_name.clone(),
            bank_name,
            duration_type,
            sound_count,
        });
    }

    Ok(results)
}

#[tauri::command]
pub fn audio_resolve_trigger(
    state: State<'_, AppState>,
    trigger_name: String,
) -> Result<Vec<SoundResult>, AppError> {
    ensure_atl(&state)?;
    let atl_guard = state.atl_index.lock();
    let atl = atl_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

    let trigger = atl
        .get_trigger(&trigger_name)
        .ok_or_else(|| AppError::Internal(format!("trigger '{}' not found", trigger_name)))?;

    let bank_name = trigger.bank_name.clone();
    let event_name = trigger.wwise_event_name.clone();
    drop(atl_guard);

    let p4k = get_p4k(&state)?;
    let mut cache_guard = state.bank_cache.lock();
    let wp = state.wwise_paths.lock().clone();
    let bank_path = wp
        .get(&bank_name)
        .cloned()
        .unwrap_or_else(|| format!("Data\\Sounds\\wwise\\{}", bank_name));
    let bank_data = p4k.read_file(&bank_path)?;
    let bnk = BnkFile::parse(&bank_data)?;
    let embedded_ids = embedded_media_ids(&bnk);

    let hierarchy = load_hierarchy(&p4k, &bank_name, &mut cache_guard, &wp)
        .ok_or_else(|| AppError::Internal(format!("failed to load bank '{}'", bank_name)))?;

    let sounds = hierarchy.resolve_event_by_name(&event_name);

    let mut seen = HashSet::new();
    Ok(sounds
        .iter()
        .filter(|s| seen.insert(s.media_id))
        .map(|s| {
            let path_desc = s
                .path
                .iter()
                .map(|id| format!("{id:#010x}"))
                .collect::<Vec<_>>()
                .join(" -> ");
            make_sound_result(
                s.media_id,
                Some(s.source),
                &bank_name,
                path_desc,
                &embedded_ids,
                &wp,
            )
        })
        .collect())
}

#[tauri::command]
pub fn audio_resolve_external_source(
    state: State<'_, AppState>,
    source_name: String,
) -> Result<Vec<SoundResult>, AppError> {
    ensure_external_sources(&state)?;
    let p4k = get_p4k(&state)?;

    let external_guard = state.external_source_index.lock();
    let external_sources = external_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("audio external sources not initialized".into()))?;

    let source = external_sources
        .get_preferred(&source_name)
        .ok_or_else(|| AppError::Internal(format!("external source '{}' not found", source_name)))?;

    Ok(vec![make_external_source_result(source, &p4k)])
}

#[tauri::command]
pub fn audio_decode_wem(
    state: State<'_, AppState>,
    media_id: u32,
    source_type: String,
    bank_name: String,
    media_path: Option<String>,
) -> Result<Vec<u8>, AppError> {
    let p4k = get_p4k(&state)?;
    let wp = state.wwise_paths.lock().clone();
    let wem_bytes = load_media_bytes(
        &p4k,
        media_id,
        &source_type,
        &bank_name,
        &wp,
        media_path.as_deref(),
    )?;

    let wem = starbreaker_wem::WemFile::parse(&wem_bytes)?;

    match wem.codec_type() {
        starbreaker_wem::WemCodec::Vorbis => {
            Ok(starbreaker_wem::decode::vorbis_to_ogg(&wem_bytes)?)
        }
        other => Err(AppError::Internal(format!(
            "codec {other} not supported for playback"
        ))),
    }
}

#[tauri::command]
pub fn audio_export_info(
    state: State<'_, AppState>,
    media_id: u32,
    source_type: String,
    bank_name: String,
    media_path: Option<String>,
) -> Result<AudioExportInfo, AppError> {
    let p4k = get_p4k(&state)?;
    let wp = state.wwise_paths.lock().clone();
    let wem_bytes = load_media_bytes(
        &p4k,
        media_id,
        &source_type,
        &bank_name,
        &wp,
        media_path.as_deref(),
    )?;
    let wem = starbreaker_wem::WemFile::parse(&wem_bytes)?;

    let extension = match wem.codec_type() {
        starbreaker_wem::WemCodec::Vorbis => "ogg",
        _ => "wem",
    };

    Ok(AudioExportInfo {
        extension: extension.to_string(),
    })
}

#[tauri::command]
pub fn audio_export_media(
    state: State<'_, AppState>,
    media_id: u32,
    source_type: String,
    bank_name: String,
    media_path: Option<String>,
    output_path: String,
) -> Result<(), AppError> {
    let p4k = get_p4k(&state)?;
    let wp = state.wwise_paths.lock().clone();
    let wem_bytes = load_media_bytes(
        &p4k,
        media_id,
        &source_type,
        &bank_name,
        &wp,
        media_path.as_deref(),
    )?;

    let wem = starbreaker_wem::WemFile::parse(&wem_bytes)?;
    let bytes = match wem.codec_type() {
        starbreaker_wem::WemCodec::Vorbis => starbreaker_wem::decode::vorbis_to_ogg(&wem_bytes)?,
        _ => wem_bytes,
    };

    let out = std::path::Path::new(&output_path);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, &bytes)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wwise_paths_with(ids: &[u32]) -> HashMap<String, String> {
        ids.iter()
            .map(|id| {
                (
                    format!("{id}.wem"),
                    format!("Data\\Sounds\\wwise\\Media\\{id}.wem"),
                )
            })
            .collect()
    }

    #[test]
    fn hirc_embedded_media_without_backing_wem_is_unavailable() {
        let embedded_ids = HashSet::new();
        let wwise_paths = HashMap::new();

        let info = classify_media_source(
            1_631_820_901,
            Some(starbreaker_wwise::SoundSource::Embedded),
            &embedded_ids,
            &wwise_paths,
        );

        assert_eq!(info.source_type, "Unavailable");
        assert!(!info.playable);
        assert!(info.description.contains("no embedded"));
        assert!(info.description.contains("no loose"));
    }

    #[test]
    fn hirc_embedded_media_with_didx_entry_is_playable_embedded() {
        let embedded_ids = HashSet::from([1_631_820_901]);
        let wwise_paths = HashMap::new();

        let info = classify_media_source(
            1_631_820_901,
            Some(starbreaker_wwise::SoundSource::Embedded),
            &embedded_ids,
            &wwise_paths,
        );

        assert_eq!(info.source_type, "Embedded");
        assert!(info.playable);
    }

    #[test]
    fn hirc_media_with_loose_wem_is_playable_media_file() {
        let embedded_ids = HashSet::new();
        let wwise_paths = wwise_paths_with(&[1_631_820_901]);

        let info = classify_media_source(
            1_631_820_901,
            Some(starbreaker_wwise::SoundSource::Embedded),
            &embedded_ids,
            &wwise_paths,
        );

        assert_eq!(info.source_type, "MediaFile");
        assert!(info.playable);
    }
}
