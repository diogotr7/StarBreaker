use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use starbreaker_p4k::MappedP4k;
use starbreaker_wwise::{AtlIndex, BnkFile, Hierarchy};

use crate::error::AppError;
use crate::state::AppState;

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AudioInitResult {
    pub trigger_count: usize,
    pub bank_count: usize,
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
    pub source_type: String,
    pub bank_name: String,
    pub path_description: String,
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
        let data = p4k.read_file(&path).ok()?;
        let bnk = BnkFile::parse(&data).ok()?;
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

fn read_streamed_wem(
    p4k: &MappedP4k,
    media_id: u32,
    wwise_paths: &HashMap<String, String>,
) -> Result<Vec<u8>, AppError> {
    let filename = format!("{media_id}.wem");
    let path = wwise_paths
        .get(&filename)
        .cloned()
        .unwrap_or_else(|| format!("Data\\Sounds\\wwise\\Media\\{media_id}.wem"));
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

// ── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn audio_init(state: State<'_, AppState>) -> Result<AudioInitResult, AppError> {
    ensure_atl(&state)?;

    // Build bank path index if not already done
    {
        let mut wp = state.wwise_paths.lock();
        if wp.is_empty() {
            let p4k = get_p4k(&state)?;
            *wp = build_wwise_path_index(&p4k);
        }
    }

    let atl_guard = state.atl_index.lock();
    let atl = atl_guard.as_ref().ok_or_else(|| AppError::Internal("audio not initialized".into()))?;
    Ok(AudioInitResult {
        trigger_count: atl.len(),
        bank_count: atl.bank_names().len(),
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

    let entities =
        starbreaker_wwise::datacore_audio::search_entities_with_audio(&db, &query);

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
    let atl = atl_guard.as_ref().ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

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
pub fn audio_list_banks(state: State<'_, AppState>) -> Result<Vec<BankResult>, AppError> {
    ensure_atl(&state)?;
    let atl_guard = state.atl_index.lock();
    let atl = atl_guard.as_ref().ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

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
    let atl = atl_guard.as_ref().ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

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

    let mut seen = HashSet::new();
    let mut results = Vec::new();

    // 1. Scan HIRC for Sound + MusicTrack objects
    if let Some(hirc) = &bnk.hirc {
        let hierarchy = Hierarchy::from_section(hirc);
        for s in hierarchy.all_media() {
            if seen.insert(s.media_id) {
                let source_type = match s.source {
                    starbreaker_wwise::SoundSource::Embedded => "Embedded",
                    starbreaker_wwise::SoundSource::PrefetchStream => "PrefetchStream",
                    starbreaker_wwise::SoundSource::Stream => "Stream",
                };
                results.push(SoundResult {
                    media_id: s.media_id,
                    source_type: source_type.to_string(),
                    bank_name: bank_name.clone(),
                    path_description: String::new(),
                });
            }
        }
    }

    // 2. Scan DIDX for embedded WEM entries not already found via HIRC
    for entry in &bnk.data_index {
        let id = entry.id;
        if seen.insert(id) {
            results.push(SoundResult {
                media_id: id,
                source_type: "Embedded".to_string(),
                bank_name: bank_name.clone(),
                path_description: String::new(),
            });
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
            if let Ok(data) = p4k.read_file(&companion_path) {
                if let Ok(cbnk) = BnkFile::parse(&data) {
                    if let Some(hirc) = &cbnk.hirc {
                        let hierarchy = Hierarchy::from_section(hirc);
                        for s in hierarchy.all_media() {
                            if seen.insert(s.media_id) {
                                let source_type = match s.source {
                                    starbreaker_wwise::SoundSource::Embedded => "Embedded",
                                    starbreaker_wwise::SoundSource::PrefetchStream => "PrefetchStream",
                                    starbreaker_wwise::SoundSource::Stream => "Stream",
                                };
                                results.push(SoundResult {
                                    media_id: s.media_id,
                                    source_type: source_type.to_string(),
                                    bank_name: companion_name.clone(),
                                    path_description: String::new(),
                                });
                            }
                        }
                    }
                    for entry in &cbnk.data_index {
                        if seen.insert(entry.id) {
                            results.push(SoundResult {
                                media_id: entry.id,
                                source_type: "Embedded".to_string(),
                                bank_name: companion_name.clone(),
                                path_description: String::new(),
                            });
                        }
                    }
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

    let entities =
        starbreaker_wwise::datacore_audio::search_entities_with_audio(&db, &entity_name);

    let entity = entities
        .iter()
        .find(|e| e.entity_name == entity_name)
        .ok_or_else(|| AppError::Internal(format!("entity '{}' not found", entity_name)))?;

    ensure_atl(&state)?;
    let atl_guard = state.atl_index.lock();
    let atl = atl_guard.as_ref().ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

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
    let atl = atl_guard.as_ref().ok_or_else(|| AppError::Internal("audio not initialized".into()))?;

    let trigger = atl
        .get_trigger(&trigger_name)
        .ok_or_else(|| AppError::Internal(format!("trigger '{}' not found", trigger_name)))?;

    let bank_name = trigger.bank_name.clone();
    let event_name = trigger.wwise_event_name.clone();
    drop(atl_guard);

    let p4k = get_p4k(&state)?;
    let mut cache_guard = state.bank_cache.lock();
    let wp = state.wwise_paths.lock().clone();

    let hierarchy = load_hierarchy(&p4k, &bank_name, &mut cache_guard, &wp)
        .ok_or_else(|| AppError::Internal(format!("failed to load bank '{}'", bank_name)))?;

    let sounds = hierarchy.resolve_event_by_name(&event_name);

    let mut seen = HashSet::new();
    Ok(sounds
        .iter()
        .filter(|s| seen.insert(s.media_id))
        .map(|s| {
            let source_type = match s.source {
                starbreaker_wwise::SoundSource::Embedded => "Embedded",
                starbreaker_wwise::SoundSource::PrefetchStream => "PrefetchStream",
                starbreaker_wwise::SoundSource::Stream => "Stream",
            };
            let path_desc = s
                .path
                .iter()
                .map(|id| format!("{id:#010x}"))
                .collect::<Vec<_>>()
                .join(" -> ");
            SoundResult {
                media_id: s.media_id,
                source_type: source_type.to_string(),
                bank_name: bank_name.clone(),
                path_description: path_desc,
            }
        })
        .collect())
}

#[tauri::command]
pub fn audio_decode_wem(
    state: State<'_, AppState>,
    media_id: u32,
    source_type: String,
    bank_name: String,
) -> Result<Vec<u8>, AppError> {
    let p4k = get_p4k(&state)?;
    let wp = state.wwise_paths.lock().clone();

    let wem_bytes = match source_type.as_str() {
        "Stream" | "PrefetchStream" => read_streamed_wem(&p4k, media_id, &wp)?,
        "Embedded" => {
            // Try extracting from bank DIDX first; fall back to streamed file
            // (some banks have HIRC-only with media stored externally)
            match read_embedded_wem(&p4k, media_id, &bank_name, &wp) {
                Ok(bytes) => bytes,
                Err(_) => read_streamed_wem(&p4k, media_id, &wp)?,
            }
        }
        other => return Err(AppError::Internal(format!("unknown source type: {other}"))),
    };

    let wem = starbreaker_wem::WemFile::parse(&wem_bytes)?;

    match wem.codec_type() {
        starbreaker_wem::WemCodec::Vorbis => {
            Ok(starbreaker_wem::decode::vorbis_to_ogg(&wem_bytes)?)
        }
        other => Err(AppError::Internal(format!("codec {other} not supported for playback"))),
    }
}
