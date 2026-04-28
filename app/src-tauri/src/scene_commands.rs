// Tauri commands for SOC-based "scene" exports (top-level socpaks like
// the Executive Hangar). The scene exporter walks a socpak graph,
// resolves every brush mesh to a CGF on disk, and emits a single `.glb`
// containing one mesh per unique CGF plus one node per placement plus
// `KHR_lights_punctual` entries for every light entity.
//
// The output is consumed by the Maps tab — this iteration stops at
// "we can write a glTF, and the existing GLB loader can ingest it"; the
// frontend Three.js wiring lands in the next iteration.
//
// Cache layout (mirrors the decomposed-export cache, separate slot):
//
//   <app_local_data_dir>/
//     scene_cache/
//       <socpak_hash>__<contract>/
//         scene.glb
//         manifest.json
//
// The cache key is a hash of the socpak path + the contract version. A
// contract bump shifts every cache slot.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use starbreaker_3d::soc;
use starbreaker_3d::soc::{SceneCatalogEntry, SocpakDirEntry, SocpakDirEntryKind};

use crate::error::AppError;
use crate::state::AppState;

/// Bump when the SOC scene contract changes (mesh / placement / light
/// schema, glTF emitter behaviour). Old cache slots carrying a
/// different version stay on disk but are unreachable until pruned.
///
/// v2 (B2): cache key now folds in the loaded p4k's mtime + size so a
/// channel switch (HOTFIX <-> TECH-PREVIEW) or a post-update p4k bump
/// invalidates stale entries automatically; manifest now carries
/// `materials_resolved` / `materials_default` counters and emits
/// `scene-load-progress` events while building.
///
/// v3 (C2): emitter caps the light count at the renderer-friendly
/// default and drops the embedded placeholder PNG for textured
/// materials. Any v2 cache slot is unsafe to reuse — it carries the
/// thousands-of-lights GLB that crashes the WebGL shader compiler.
const SCENE_GLB_CONTRACT_VERSION: u32 = 3;

/// Bump when the scene-catalog enumeration algorithm changes (filter
/// constants, name-derivation rules, source kinds, etc.). Old
/// catalog cache JSONs stamped with a different version are
/// re-enumerated; they stay on disk until pruned. v1 (C3): graph
/// in-degree-zero roots under `Data/ObjectContainers/` with the
/// 100 KB minimum-size and `test|tmp|backup` name filters.
const SCENE_CATALOG_CONTRACT_VERSION: u32 = 1;

/// Search roots the catalog walks by default. Socpaks live almost
/// exclusively under `Data\ObjectContainers\` -- broadening the
/// search to the whole archive picks up nothing of interest and
/// wastes seconds on every cold enumeration.
const DEFAULT_CATALOG_SEARCH_ROOTS: &[&str] = &["Data/ObjectContainers/"];

/// Maximum recursion depth for the socpak walk. Three is enough for the
/// observed hangar / dungeon / module hierarchies; deeper levels can
/// override it through the `max_depth` argument.
const DEFAULT_MAX_DEPTH: u32 = 4;

/// Response payload for [`load_scene_to_gltf`]. Plain types so the
/// frontend can deserialise without a shared type contract.
#[derive(Debug, Clone, Serialize)]
pub struct LoadSceneResponse {
    /// Absolute path of the emitted `.glb` on disk.
    pub glb_path: String,
    /// Absolute path of the manifest JSON written next to the GLB.
    pub manifest_path: String,
    /// True when the slot was already present and the command did not
    /// re-emit. Useful for the frontend to choose between "loading..."
    /// and "ready" UI states.
    pub cache_hit: bool,
    /// Mesh and placement counts in the emitted GLB.
    pub mesh_count: u32,
    pub placement_count: u32,
    /// Lights kept in the emitted GLB. Capped to a renderer-friendly
    /// budget so the WebGL fragment shader does not exceed
    /// `MAX_FRAGMENT_UNIFORM_VECTORS`.
    pub light_count: u32,
    /// Lights present in the source scene but skipped because the cap
    /// was reached. `lights_dropped + light_count` equals the total
    /// number of lights the SOC parser found.
    pub lights_dropped: u32,
    /// World-space AABB of the brush placements, when at least one
    /// brush resolved. CryEngine Z-up.
    pub aabb_min: Option<[f32; 3]>,
    pub aabb_max: Option<[f32; 3]>,
    /// Bytes of the emitted GLB on disk. Useful for "show file size in
    /// the loading dialog" affordances.
    pub glb_bytes: u64,
    /// How many brush placements were dropped because their mesh failed
    /// to resolve. Surfacing this on the UI side helps spot a sudden
    /// regression in mesh-loading coverage.
    pub dropped_placements: u32,
    /// How many unique mesh paths failed to resolve in the underlying
    /// p4k.
    pub failed_mesh_paths: u32,
    /// Unique meshes whose MTL file resolved successfully.
    pub materials_resolved: u32,
    /// Unique meshes that fell back to the neutral default material
    /// because no MTL was found.
    pub materials_default: u32,
}

/// Payload emitted on the `scene-load-progress` event. Phases progress
/// monotonically through `compose -> resolve -> emit -> cache_write`.
/// The frontend reads `(current, total)` for the active phase and
/// renders a determinate bar.
#[derive(Debug, Clone, Serialize)]
pub struct SceneLoadProgress {
    pub phase: &'static str,
    pub current: u32,
    pub total: u32,
    /// Free-form status line for the loading dialog. Empty string when
    /// the phase progress alone is informative enough.
    pub message: String,
}

/// Read a GLB file from disk and return the bytes. The frontend uses
/// this to fetch the cached scene GLB into a `Uint8Array` it can hand
/// directly to GLTFLoader.parse, avoiding any need for an asset:
/// protocol capability. The path must be an absolute path to a file
/// inside our app-local-data scene cache root; we reject paths that
/// escape the cache to keep this from becoming a generic file-read
/// primitive.
#[tauri::command]
pub async fn read_scene_glb(
    app: AppHandle,
    glb_path: String,
) -> Result<Vec<u8>, AppError> {
    let cache_root = scene_glb_cache_root(&app)?;
    let cache_root_canon = std::fs::canonicalize(&cache_root).unwrap_or(cache_root);
    let candidate = std::path::PathBuf::from(&glb_path);
    let candidate_canon = std::fs::canonicalize(&candidate)
        .map_err(|e| AppError::Internal(format!("scene glb canonicalize: {e}")))?;
    if !candidate_canon.starts_with(&cache_root_canon) {
        return Err(AppError::Internal(format!(
            "refusing to read glb outside cache root: {}",
            candidate_canon.display()
        )));
    }
    tokio::task::spawn_blocking(move || std::fs::read(&candidate_canon).map_err(AppError::Io))
        .await
        .map_err(|e| AppError::Internal(format!("read_scene_glb join: {e}")))?
}

/// Build a renderable scene for the given socpak and emit it as a GLB
/// on disk. Returns the cache path of the GLB plus a small summary.
/// Called by the frontend's Maps tab when the user opens a scene.
#[tauri::command]
pub async fn load_scene_to_gltf(
    app: AppHandle,
    state: State<'_, AppState>,
    socpak_path: String,
    max_depth: Option<u32>,
) -> Result<LoadSceneResponse, AppError> {
    let (p4k, p4k_path) = {
        let guard = state.p4k.lock();
        let arc = guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?
            .clone();
        let path = arc.path().to_string_lossy().into_owned();
        (arc, path)
    };
    let cache_root = scene_glb_cache_root(&app)?;
    std::fs::create_dir_all(&cache_root)?;

    let depth = max_depth.unwrap_or(DEFAULT_MAX_DEPTH);
    // Fold the loaded p4k's mtime + size into the cache key so channel
    // swaps (HOTFIX <-> TECH-PREVIEW) or post-update file replacement
    // invalidate stale slots without forcing a manual purge. Defaults to
    // 0/0 when the metadata read fails, which still produces a stable
    // key that just happens to ignore identity (unavoidable: better to
    // serve a slot than refuse to render).
    let p4k_id = p4k_identity(&p4k_path).unwrap_or_default();
    let cache_dir = scene_glb_cache_dir(&cache_root, &socpak_path, depth, &p4k_id);
    let glb_path = cache_dir.join("scene.glb");
    let manifest_path = cache_dir.join("manifest.json");

    if glb_path.is_file() && manifest_path.is_file() {
        if let Ok(text) = std::fs::read_to_string(&manifest_path)
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        {
            return Ok(response_from_cached_manifest(
                &glb_path,
                &manifest_path,
                &value,
            ));
        }
    }

    let socpak_path_for_blocking = socpak_path.clone();
    let cache_dir_for_blocking = cache_dir.clone();
    let glb_path_for_blocking = glb_path.clone();
    let manifest_path_for_blocking = manifest_path.clone();
    let app_for_blocking = app.clone();

    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&cache_dir_for_blocking)?;

        // Compose phase. Indeterminate (zone count is not known up
        // front), so we report 0/0 once before kicking off and let the
        // resolve phase replace it with a proper bar.
        emit_progress(&app_for_blocking, "compose", 0, 0, "Composing zones...");
        let scene = soc::compose_from_root(&p4k, &socpak_path_for_blocking, depth)
            .map_err(|e| AppError::Internal(format!("compose_from_root: {e}")))?;
        emit_progress(
            &app_for_blocking,
            "compose",
            scene.zones.len() as u32,
            scene.zones.len() as u32,
            &format!(
                "Composed {} zones ({} brushes)",
                scene.zones.len(),
                scene.zones.iter().map(|z| z.brushes.len()).sum::<usize>()
            ),
        );

        // Resolve phase. ~10 Hz throttle: track the last-emitted
        // timestamp and suppress events that arrive sooner than 100ms
        // after the previous one. Final tick is forced through.
        let last_emit_ms = Arc::new(AtomicU64::new(0));
        let resolve_start = Instant::now();
        let app_resolve = app_for_blocking.clone();
        let renderable = {
            let last_emit_ms = last_emit_ms.clone();
            let mut on_progress = |current: usize, total: usize| {
                let now_ms = resolve_start.elapsed().as_millis() as u64;
                let prev = last_emit_ms.load(Ordering::Relaxed);
                let is_final = current == total && total > 0;
                if !is_final && now_ms.saturating_sub(prev) < 100 {
                    return;
                }
                last_emit_ms.store(now_ms, Ordering::Relaxed);
                emit_progress(
                    &app_resolve,
                    "resolve",
                    current as u32,
                    total as u32,
                    &format!("Resolving meshes {current}/{total}"),
                );
            };
            soc::resolve_scene_with_progress(&p4k, &scene, &mut on_progress)
        };

        // Emit phase. The glTF emitter is a single linear pass; we
        // report start + end so the bar advances without trying to
        // instrument the bytecode walker.
        emit_progress(
            &app_for_blocking,
            "emit",
            0,
            renderable.meshes.len() as u32,
            "Emitting glTF...",
        );
        let (glb_bytes, summary) = soc::emit_glb(&renderable)
            .map_err(|e| AppError::Internal(format!("emit_glb: {e}")))?;
        emit_progress(
            &app_for_blocking,
            "emit",
            renderable.meshes.len() as u32,
            renderable.meshes.len() as u32,
            &format!(
                "Emitted {} bytes ({} meshes)",
                glb_bytes.len(),
                renderable.meshes.len()
            ),
        );

        // Cache write.
        emit_progress(
            &app_for_blocking,
            "cache_write",
            0,
            1,
            "Writing cache...",
        );
        std::fs::write(&glb_path_for_blocking, &glb_bytes)?;

        let manifest = build_manifest(
            &socpak_path_for_blocking,
            depth,
            &summary,
            renderable.dropped_placements,
            renderable.failed_mesh_paths,
            renderable.materials_resolved,
            renderable.materials_default,
            glb_bytes.len() as u64,
            &p4k_id,
        );
        std::fs::write(
            &manifest_path_for_blocking,
            serde_json::to_string_pretty(&manifest)
                .map_err(|e| AppError::Internal(format!("manifest serialize: {e}")))?,
        )?;
        emit_progress(
            &app_for_blocking,
            "cache_write",
            1,
            1,
            "Cache written",
        );

        let response = LoadSceneResponse {
            glb_path: glb_path_for_blocking.to_string_lossy().into_owned(),
            manifest_path: manifest_path_for_blocking.to_string_lossy().into_owned(),
            cache_hit: false,
            mesh_count: summary.mesh_count,
            placement_count: summary.placement_count,
            light_count: summary.light_count,
            lights_dropped: summary.lights_dropped,
            aabb_min: summary.aabb_min,
            aabb_max: summary.aabb_max,
            glb_bytes: glb_bytes.len() as u64,
            dropped_placements: renderable.dropped_placements,
            failed_mesh_paths: renderable.failed_mesh_paths,
            materials_resolved: renderable.materials_resolved,
            materials_default: renderable.materials_default,
        };
        Ok::<LoadSceneResponse, AppError>(response)
    })
    .await
    .map_err(|e| AppError::Internal(format!("scene task join: {e}")))?
}

/// Emit a `scene-load-progress` event. Failures are swallowed: if the
/// frontend's listener has unmounted, we still want the build to
/// continue.
fn emit_progress(
    app: &AppHandle,
    phase: &'static str,
    current: u32,
    total: u32,
    message: &str,
) {
    let _ = app.emit(
        "scene-load-progress",
        SceneLoadProgress {
            phase,
            current,
            total,
            message: message.to_string(),
        },
    );
}

/// Stable-ish identity for a p4k file: `(mtime_unix_secs, size_bytes)`.
/// Folded into the cache key so channel switches and post-update file
/// replacement invalidate stale slots automatically.
#[derive(Debug, Default, Clone, Copy)]
struct P4kIdentity {
    mtime_secs: i64,
    size_bytes: u64,
}

fn p4k_identity(path: &str) -> Option<P4kIdentity> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(P4kIdentity {
        mtime_secs: mtime,
        size_bytes: meta.len(),
    })
}

fn response_from_cached_manifest(
    glb_path: &std::path::Path,
    manifest_path: &std::path::Path,
    value: &serde_json::Value,
) -> LoadSceneResponse {
    let glb_bytes = std::fs::metadata(glb_path).map(|m| m.len()).unwrap_or(0);
    let mesh_count = value
        .get("mesh_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let placement_count = value
        .get("placement_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let light_count = value
        .get("light_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let lights_dropped = value
        .get("lights_dropped")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let dropped_placements = value
        .get("dropped_placements")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let failed_mesh_paths = value
        .get("failed_mesh_paths")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let materials_resolved = value
        .get("materials_resolved")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let materials_default = value
        .get("materials_default")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let aabb_min = value.get("aabb_min").and_then(parse_vec3);
    let aabb_max = value.get("aabb_max").and_then(parse_vec3);
    LoadSceneResponse {
        glb_path: glb_path.to_string_lossy().into_owned(),
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        cache_hit: true,
        mesh_count,
        placement_count,
        light_count,
        lights_dropped,
        aabb_min,
        aabb_max,
        glb_bytes,
        dropped_placements,
        failed_mesh_paths,
        materials_resolved,
        materials_default,
    }
}

fn parse_vec3(value: &serde_json::Value) -> Option<[f32; 3]> {
    let arr = value.as_array()?;
    if arr.len() != 3 {
        return None;
    }
    Some([
        arr[0].as_f64()? as f32,
        arr[1].as_f64()? as f32,
        arr[2].as_f64()? as f32,
    ])
}

#[allow(clippy::too_many_arguments)]
fn build_manifest(
    socpak_path: &str,
    max_depth: u32,
    summary: &soc::GlbEmitSummary,
    dropped_placements: u32,
    failed_mesh_paths: u32,
    materials_resolved: u32,
    materials_default: u32,
    glb_bytes: u64,
    p4k_id: &P4kIdentity,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "contract_version".into(),
        serde_json::json!(SCENE_GLB_CONTRACT_VERSION),
    );
    map.insert("socpak_path".into(), serde_json::json!(socpak_path));
    map.insert("max_depth".into(), serde_json::json!(max_depth));
    map.insert("mesh_count".into(), serde_json::json!(summary.mesh_count));
    map.insert(
        "placement_count".into(),
        serde_json::json!(summary.placement_count),
    );
    map.insert("light_count".into(), serde_json::json!(summary.light_count));
    map.insert(
        "lights_dropped".into(),
        serde_json::json!(summary.lights_dropped),
    );
    map.insert(
        "material_count".into(),
        serde_json::json!(summary.material_count),
    );
    map.insert(
        "texture_count".into(),
        serde_json::json!(summary.texture_count),
    );
    if let Some(mn) = summary.aabb_min {
        map.insert("aabb_min".into(), serde_json::json!(mn));
    }
    if let Some(mx) = summary.aabb_max {
        map.insert("aabb_max".into(), serde_json::json!(mx));
    }
    map.insert(
        "dropped_placements".into(),
        serde_json::json!(dropped_placements),
    );
    map.insert(
        "failed_mesh_paths".into(),
        serde_json::json!(failed_mesh_paths),
    );
    map.insert(
        "materials_resolved".into(),
        serde_json::json!(materials_resolved),
    );
    map.insert(
        "materials_default".into(),
        serde_json::json!(materials_default),
    );
    map.insert("glb_bytes".into(), serde_json::json!(glb_bytes));
    map.insert(
        "p4k_mtime_secs".into(),
        serde_json::json!(p4k_id.mtime_secs),
    );
    map.insert(
        "p4k_size_bytes".into(),
        serde_json::json!(p4k_id.size_bytes),
    );
    serde_json::Value::Object(map)
}

fn scene_glb_cache_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let local = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Internal(format!("app_local_data_dir unavailable: {e}")))?;
    Ok(local.join("scene_cache"))
}

fn scene_glb_cache_dir(
    cache_root: &std::path::Path,
    socpak_path: &str,
    max_depth: u32,
    p4k_id: &P4kIdentity,
) -> PathBuf {
    let hash = stable_hash_socpak(socpak_path, max_depth, p4k_id);
    let safe_tail = sanitize_socpak_tail(socpak_path);
    cache_root.join(format!(
        "{safe_tail}__d{max_depth}_{hash:016x}_v{SCENE_GLB_CONTRACT_VERSION}"
    ))
}

/// Stable hash over the socpak path + max_depth + p4k identity so the
/// cache key is deterministic across app restarts and invalidates when
/// the underlying p4k file changes (channel switch, post-update). Uses
/// the same `DefaultHasher` algorithm the rest of the codebase relies
/// on for option fingerprints.
fn stable_hash_socpak(socpak_path: &str, max_depth: u32, p4k_id: &P4kIdentity) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    socpak_path.to_ascii_lowercase().hash(&mut hasher);
    max_depth.hash(&mut hasher);
    p4k_id.mtime_secs.hash(&mut hasher);
    p4k_id.size_bytes.hash(&mut hasher);
    hasher.finish()
}

/// Take the file stem of a socpak path and sanitise it for filesystem
/// use. Result is bounded at 64 ASCII chars so cache directory names
/// stay manageable.
fn sanitize_socpak_tail(socpak_path: &str) -> String {
    let tail = socpak_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(socpak_path);
    let stem = tail.strip_suffix(".socpak").unwrap_or(tail);
    let mut out = String::with_capacity(stem.len().min(64));
    for ch in stem.chars().take(64) {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.' => out.push(ch),
            ' ' => out.push('_'),
            _ => out.push('_'),
        }
    }
    if out.is_empty() {
        "scene".into()
    } else {
        out
    }
}

// ── Scene catalog (Maps tab list) ───────────────────────────────────────────
//
// The scene catalog enumerates every "scene root" socpak in the loaded
// p4k. A root is a socpak that no other socpak references as a child.
// Enumeration walks every socpak under
// `DEFAULT_CATALOG_SEARCH_ROOTS`, builds a child-reference graph,
// and returns the in-degree-zero nodes. Result is cached to disk so
// the next call against the same p4k is a JSON read instead of a
// fresh ~5-15s enumeration.

/// JSON-shape mirror of `soc::SceneCatalogEntry`. We carry our own
/// type rather than serialising the lib type directly so the
/// `source_kind` field can ship as a snake-case string without
/// `starbreaker-3d` taking a serde dependency.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct SceneCatalogEntryDto {
    pub path: String,
    pub display_name: String,
    pub sub_zone_count: usize,
    /// `"graph_root"` for the C3 implementation. Future variants
    /// land alongside without a contract version bump.
    pub source_kind: String,
}

impl From<SceneCatalogEntry> for SceneCatalogEntryDto {
    fn from(value: SceneCatalogEntry) -> Self {
        Self {
            path: value.path,
            display_name: value.display_name,
            sub_zone_count: value.sub_zone_count,
            source_kind: value.source_kind.as_snake_case().to_string(),
        }
    }
}

/// Cached on-disk representation. Bundles entries with the contract
/// version so a stale slot from an older algorithm is detected on
/// load and rebuilt.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct SceneCatalogCachePayload {
    contract_version: u32,
    p4k_path: String,
    p4k_mtime_secs: i64,
    p4k_size_bytes: u64,
    entries: Vec<SceneCatalogEntryDto>,
}

/// Walk the loaded p4k and return the in-degree-zero scene roots.
///
/// `channel` is reserved for future use (HOTFIX / TECH-PREVIEW
/// switching without re-loading). For now it is ignored and the
/// command always operates against the currently-loaded p4k --
/// surfacing a `Not loaded` error when no archive has been opened.
#[tauri::command]
pub async fn enumerate_scenes(
    app: AppHandle,
    state: State<'_, AppState>,
    #[allow(unused_variables)]
    channel: Option<String>,
) -> Result<Vec<SceneCatalogEntryDto>, AppError> {
    let (p4k, p4k_path) = {
        let guard = state.p4k.lock();
        let arc = guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?
            .clone();
        let path = arc.path().to_string_lossy().into_owned();
        (arc, path)
    };

    let p4k_id = p4k_identity(&p4k_path).unwrap_or_default();
    let cache_root = scene_catalog_cache_root(&app)?;
    std::fs::create_dir_all(&cache_root)?;
    let cache_path = scene_catalog_cache_file(&cache_root, &p4k_path, &p4k_id);

    if cache_path.is_file()
        && let Ok(text) = std::fs::read_to_string(&cache_path)
        && let Ok(payload) = serde_json::from_str::<SceneCatalogCachePayload>(&text)
        && payload.contract_version == SCENE_CATALOG_CONTRACT_VERSION
        && payload.p4k_mtime_secs == p4k_id.mtime_secs
        && payload.p4k_size_bytes == p4k_id.size_bytes
    {
        return Ok(payload.entries);
    }

    let p4k_path_for_blocking = p4k_path.clone();
    let cache_path_for_blocking = cache_path.clone();
    let entries = tokio::task::spawn_blocking(move || -> Result<Vec<SceneCatalogEntryDto>, AppError> {
        let raw = soc::enumerate_scene_roots(&p4k, DEFAULT_CATALOG_SEARCH_ROOTS)
            .map_err(|e| AppError::Internal(format!("enumerate_scene_roots: {e}")))?;
        let dtos: Vec<SceneCatalogEntryDto> =
            raw.into_iter().map(SceneCatalogEntryDto::from).collect();

        // Best-effort cache write. A serialise / write failure is
        // logged but does not fail the command -- the user still
        // gets the catalog, the next call just rebuilds it.
        let payload = SceneCatalogCachePayload {
            contract_version: SCENE_CATALOG_CONTRACT_VERSION,
            p4k_path: p4k_path_for_blocking,
            p4k_mtime_secs: p4k_id.mtime_secs,
            p4k_size_bytes: p4k_id.size_bytes,
            entries: dtos.clone(),
        };
        match serde_json::to_string(&payload) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&cache_path_for_blocking, text) {
                    log::warn!(
                        "scene-catalog: failed to write cache to {}: {err}",
                        cache_path_for_blocking.display()
                    );
                }
            }
            Err(err) => log::warn!("scene-catalog: failed to serialise cache: {err}"),
        }
        Ok(dtos)
    })
    .await
    .map_err(|e| AppError::Internal(format!("scene catalog task join: {e}")))?;

    entries
}

fn scene_catalog_cache_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let local = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Internal(format!("app_local_data_dir unavailable: {e}")))?;
    Ok(local.join("scene_catalog"))
}

/// One JSON file per (p4k path, mtime, size) tuple. Channel switches
/// land in a different filename automatically; post-update file
/// replacement lands in a different filename automatically. We do
/// not delete stale slots -- they are cheap on disk and a future
/// pruning pass can sweep them.
fn scene_catalog_cache_file(
    cache_root: &std::path::Path,
    p4k_path: &str,
    p4k_id: &P4kIdentity,
) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    p4k_path.to_ascii_lowercase().hash(&mut hasher);
    p4k_id.mtime_secs.hash(&mut hasher);
    p4k_id.size_bytes.hash(&mut hasher);
    let hash = hasher.finish();
    cache_root.join(format!(
        "catalog_{hash:016x}_v{SCENE_CATALOG_CONTRACT_VERSION}.json"
    ))
}

// ── Lazy directory-tree listing (Maps tab) ─────────────────────────────────
//
// Replaces the eager `enumerate_scenes` traversal as the catalog
// driver. The frontend asks for one prefix at a time; this command
// returns its immediate children (subdirs + `.socpak` files) and the
// frontend caches expanded branches client-side.

/// JSON-shape mirror of `soc::SocpakDirEntry`. Carrying our own type
/// keeps the underlying lib type free of a serde dependency and lets
/// the `kind` field ship as a stable snake-case string the frontend can
/// pattern-match without an enum bump.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct SocpakDirEntryDto {
    pub path: String,
    pub display_name: String,
    /// `"directory"` or `"socpak_file"`.
    pub kind: String,
    pub size_or_count: u64,
}

impl From<SocpakDirEntry> for SocpakDirEntryDto {
    fn from(value: SocpakDirEntry) -> Self {
        Self {
            path: value.path,
            display_name: value.display_name,
            kind: match value.kind {
                SocpakDirEntryKind::Directory => "directory".into(),
                SocpakDirEntryKind::SocpakFile => "socpak_file".into(),
            },
            size_or_count: value.size_or_count,
        }
    }
}

/// List the immediate children of a p4k directory prefix as
/// directories + `.socpak` files, sorted (dirs first, both
/// alphabetical). Used by the Maps tab tree to expand a branch on
/// click without paying the cost of a full graph traversal.
///
/// `prefix` is a p4k-internal directory path. Trailing-separator and
/// slash-flavour are normalised internally.
#[tauri::command]
pub async fn list_socpak_dir_cmd(
    state: State<'_, AppState>,
    prefix: String,
) -> Result<Vec<SocpakDirEntryDto>, AppError> {
    let p4k = {
        let guard = state.p4k.lock();
        guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?
            .clone()
    };

    // The listing is fast enough (sub-millisecond on `Data\ObjectContainers\`,
    // a few ms on the largest interior subtrees) that an uncached
    // synchronous call inside the async runtime is fine. If a future
    // prefix turns out to be expensive, we can lift this into
    // `spawn_blocking`.
    let entries = soc::list_socpak_dir(&p4k, &prefix);
    Ok(entries.into_iter().map(SocpakDirEntryDto::from).collect())
}

// ── Global socpak path index (Maps tab "search everywhere") ────────────────
//
// `list_all_socpaks_cmd` is the cousin of `list_socpak_dir_cmd`: instead
// of returning one branch's children, it returns every `.socpak` path in
// the loaded p4k. Intended to seed the Maps tab's search box -- the lazy
// tree's branch-by-branch filter cannot find a path under an unexpanded
// directory, but this list can.
//
// Cache key: the loaded p4k's mtime + size, the same identity tuple
// `load_scene_to_gltf` uses. A channel switch or post-update file
// replacement automatically lands in a different cache slot. Cold call
// against the live HOTFIX archive completes in a few hundred ms; a
// cached call is a JSON read.

/// Bump when the global-index payload shape or filter rules change.
/// Today: single field (entries: Vec<String>) plus identity tuple.
const SOCPAK_INDEX_CONTRACT_VERSION: u32 = 1;

/// Default search roots for the global socpak index. Mirrors
/// [`DEFAULT_CATALOG_SEARCH_ROOTS`] -- broadening past `Data\ObjectContainers\`
/// returns nothing the Maps tab can sensibly load.
const DEFAULT_INDEX_SEARCH_ROOTS: &[&str] = &["Data/ObjectContainers/"];

/// On-disk cache payload for the global socpak path index. Carries the
/// contract version + p4k identity so a stale slot from a different
/// archive (or older algorithm) is detected and rebuilt.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct SocpakIndexCachePayload {
    contract_version: u32,
    p4k_path: String,
    p4k_mtime_secs: i64,
    p4k_size_bytes: u64,
    entries: Vec<String>,
}

/// Walk every entry in the loaded p4k and return the path of every
/// `.socpak` file under any of `search_roots` (defaults to
/// `["Data/ObjectContainers/"]`). Returned paths are sorted
/// alphabetically (case-insensitive). Cached on disk by p4k identity --
/// the cold call is a few hundred ms, the cached call is a JSON read.
#[tauri::command]
pub async fn list_all_socpaks_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    search_roots: Option<Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let (p4k, p4k_path) = {
        let guard = state.p4k.lock();
        let arc = guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?
            .clone();
        let path = arc.path().to_string_lossy().into_owned();
        (arc, path)
    };

    let p4k_id = p4k_identity(&p4k_path).unwrap_or_default();
    let cache_root = socpak_index_cache_root(&app)?;
    std::fs::create_dir_all(&cache_root)?;

    // The cache filename folds in the chosen search-root set so two
    // callers with different roots do not stomp each other. Default
    // callers always land in the same slot.
    let roots_for_key: Vec<String> = match &search_roots {
        Some(v) => v.clone(),
        None => DEFAULT_INDEX_SEARCH_ROOTS.iter().map(|s| (*s).to_string()).collect(),
    };
    let cache_path = socpak_index_cache_file(&cache_root, &p4k_path, &p4k_id, &roots_for_key);

    if cache_path.is_file()
        && let Ok(text) = std::fs::read_to_string(&cache_path)
        && let Ok(payload) = serde_json::from_str::<SocpakIndexCachePayload>(&text)
        && payload.contract_version == SOCPAK_INDEX_CONTRACT_VERSION
        && payload.p4k_mtime_secs == p4k_id.mtime_secs
        && payload.p4k_size_bytes == p4k_id.size_bytes
    {
        return Ok(payload.entries);
    }

    // Cold path. Run the linear scan on a blocking task; even at a
    // few hundred ms it would otherwise stall the Tauri runtime.
    let p4k_path_for_blocking = p4k_path.clone();
    let cache_path_for_blocking = cache_path.clone();
    let entries = tokio::task::spawn_blocking(move || -> Result<Vec<String>, AppError> {
        let roots: Vec<&str> = roots_for_key.iter().map(|s| s.as_str()).collect();
        let entries = soc::list_all_socpaks(&p4k, &roots);

        // Best-effort cache write. A serialise / write failure logs but
        // does not fail the command -- the user still gets the index;
        // the next call just rebuilds it.
        let payload = SocpakIndexCachePayload {
            contract_version: SOCPAK_INDEX_CONTRACT_VERSION,
            p4k_path: p4k_path_for_blocking,
            p4k_mtime_secs: p4k_id.mtime_secs,
            p4k_size_bytes: p4k_id.size_bytes,
            entries: entries.clone(),
        };
        match serde_json::to_string(&payload) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&cache_path_for_blocking, text) {
                    log::warn!(
                        "socpak-index: failed to write cache to {}: {err}",
                        cache_path_for_blocking.display()
                    );
                }
            }
            Err(err) => log::warn!("socpak-index: failed to serialise cache: {err}"),
        }
        Ok(entries)
    })
    .await
    .map_err(|e| AppError::Internal(format!("socpak index task join: {e}")))?;

    entries
}

fn socpak_index_cache_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let local = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Internal(format!("app_local_data_dir unavailable: {e}")))?;
    Ok(local.join("scene_index"))
}

/// One JSON file per (p4k path, mtime, size, search_roots) tuple.
/// Channel switches / post-update file replacement / different root
/// sets each land in a different filename. Slots are not auto-pruned;
/// they are cheap on disk.
fn socpak_index_cache_file(
    cache_root: &std::path::Path,
    p4k_path: &str,
    p4k_id: &P4kIdentity,
    search_roots: &[String],
) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    p4k_path.to_ascii_lowercase().hash(&mut hasher);
    p4k_id.mtime_secs.hash(&mut hasher);
    p4k_id.size_bytes.hash(&mut hasher);
    // Order-sensitive on purpose: changing the order of search roots
    // does not change the resulting set, but keeping the hash sensitive
    // means the worst case is an extra cold rebuild, never a wrong cache
    // hit.
    for r in search_roots {
        r.to_ascii_lowercase().hash(&mut hasher);
    }
    let hash = hasher.finish();
    cache_root.join(format!(
        "index_{hash:016x}_v{SOCPAK_INDEX_CONTRACT_VERSION}.json"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> P4kIdentity {
        P4kIdentity {
            mtime_secs: 1_700_000_000,
            size_bytes: 4096,
        }
    }

    #[test]
    fn cache_dir_distinguishes_socpaks() {
        let root = std::path::Path::new("/cache");
        let a = scene_glb_cache_dir(root, "Data\\foo\\a.socpak", 4, &id());
        let b = scene_glb_cache_dir(root, "Data\\foo\\b.socpak", 4, &id());
        assert_ne!(a, b);
    }

    #[test]
    fn cache_dir_distinguishes_max_depth() {
        let root = std::path::Path::new("/cache");
        let d3 = scene_glb_cache_dir(root, "Data\\foo\\a.socpak", 3, &id());
        let d4 = scene_glb_cache_dir(root, "Data\\foo\\a.socpak", 4, &id());
        assert_ne!(d3, d4);
    }

    #[test]
    fn cache_dir_distinguishes_p4k_identity() {
        let root = std::path::Path::new("/cache");
        let a = scene_glb_cache_dir(
            root,
            "Data\\foo\\a.socpak",
            4,
            &P4kIdentity {
                mtime_secs: 1_700_000_000,
                size_bytes: 4096,
            },
        );
        let b = scene_glb_cache_dir(
            root,
            "Data\\foo\\a.socpak",
            4,
            &P4kIdentity {
                mtime_secs: 1_700_000_001,
                size_bytes: 4096,
            },
        );
        assert_ne!(a, b, "different p4k mtime should yield different slot");
    }

    #[test]
    fn cache_dir_carries_contract_version() {
        let root = std::path::Path::new("/cache");
        let dir = scene_glb_cache_dir(root, "Data\\foo\\a.socpak", 4, &id());
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap();
        assert!(
            name.ends_with(&format!("_v{SCENE_GLB_CONTRACT_VERSION}")),
            "cache dir name should end with the contract version: {name}"
        );
    }

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(
            sanitize_socpak_tail("Data\\foo bar/baz.socpak"),
            "baz"
        );
    }

    #[test]
    fn socpak_index_cache_file_distinguishes_p4k_identity() {
        let root = std::path::Path::new("/cache");
        let roots: Vec<String> = vec!["Data/ObjectContainers/".into()];
        let a = socpak_index_cache_file(root, "/tmp/HOTFIX/Data.p4k", &id(), &roots);
        let b = socpak_index_cache_file(
            root,
            "/tmp/HOTFIX/Data.p4k",
            &P4kIdentity {
                mtime_secs: id().mtime_secs + 1,
                size_bytes: id().size_bytes,
            },
            &roots,
        );
        assert_ne!(a, b, "different p4k mtime should produce a different slot");
    }

    #[test]
    fn socpak_index_cache_file_distinguishes_p4k_path() {
        let root = std::path::Path::new("/cache");
        let roots: Vec<String> = vec!["Data/ObjectContainers/".into()];
        let a = socpak_index_cache_file(root, "/tmp/HOTFIX/Data.p4k", &id(), &roots);
        let b = socpak_index_cache_file(root, "/tmp/TECH-PREVIEW/Data.p4k", &id(), &roots);
        assert_ne!(a, b);
    }

    #[test]
    fn socpak_index_cache_file_distinguishes_search_roots() {
        let root = std::path::Path::new("/cache");
        let a = socpak_index_cache_file(
            root,
            "/tmp/HOTFIX/Data.p4k",
            &id(),
            &["Data/ObjectContainers/".to_string()],
        );
        let b = socpak_index_cache_file(
            root,
            "/tmp/HOTFIX/Data.p4k",
            &id(),
            &["Data/Other/".to_string()],
        );
        assert_ne!(a, b);
    }

    #[test]
    fn socpak_index_cache_file_carries_contract_version() {
        let root = std::path::Path::new("/cache");
        let path = socpak_index_cache_file(
            root,
            "/tmp/HOTFIX/Data.p4k",
            &id(),
            &["Data/ObjectContainers/".to_string()],
        );
        let name = path.file_name().and_then(|n| n.to_str()).unwrap();
        assert!(
            name.ends_with(&format!("_v{SOCPAK_INDEX_CONTRACT_VERSION}.json")),
            "cache file name should end with the contract version: {name}"
        );
    }
}
