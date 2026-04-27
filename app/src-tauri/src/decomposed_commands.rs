// Tauri commands for browsing and loading "decomposed" entity export packages.
//
// A decomposed export root has this layout:
//
//   <root>/
//     Packages/
//       <package_name>/         e.g. "Polaris_LOD0_TEX2"
//         scene.json
//         palettes.json
//         liveries.json
//     Data/
//       Objects/.../*.glb
//       Objects/.../*.materials.json
//       Objects/.../*.png
//       Objects/.../*.dds        (projector / gobo textures)
//
// The JSON contract is dynamic (built from `serde_json::json!` in
// `starbreaker_3d::decomposed`), so these commands return raw
// `serde_json::Value` instead of typed structs. Re-deriving Rust types
// here would duplicate the contract and rot the moment a field is added.
// The frontend reads fields it needs and ignores the rest.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use starbreaker_common::Progress;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::{EntityIndex, resolve_loadout_indexed};
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::error::AppError;
use crate::state::AppState;

/// Maximum file size we are willing to read into the frontend in one shot.
/// 256 MiB. Decomposed mesh GLBs are typically a few MiB; texture PNGs are
/// at most a few MiB at mip 2; DDS gobos are tiny. This cap exists to fail
/// fast if the frontend is pointed at the wrong file.
const MAX_DECOMPOSED_READ_BYTES: u64 = 256 * 1024 * 1024;

/// Resolve a frontend-supplied path to an absolute, normalized PathBuf.
///
/// We normalize separators so the frontend can pass either forward or
/// backslashes. We do NOT enforce a sandbox on the path here — the
/// frontend explicitly opens a directory the user picked, and reads files
/// underneath it. Tauri capabilities still gate which IPC commands the
/// frontend can call at all.
fn normalize_path(path: &str) -> PathBuf {
    PathBuf::from(path.replace('\\', "/"))
}

/// Read a file under the decomposed export root and return its bytes.
/// Used for GLB meshes, PNG textures, and DDS gobo textures so the frontend
/// does not need a tauri-plugin-fs scope.
#[tauri::command]
pub fn read_decomposed_file(path: String) -> Result<Vec<u8>, AppError> {
    let abs = normalize_path(&path);
    let metadata = std::fs::metadata(&abs)?;
    if !metadata.is_file() {
        return Err(AppError::Internal(format!(
            "decomposed read target is not a file: {}",
            abs.display()
        )));
    }
    if metadata.len() > MAX_DECOMPOSED_READ_BYTES {
        return Err(AppError::Internal(format!(
            "decomposed file '{}' is {} bytes (over {}-byte cap)",
            abs.display(),
            metadata.len(),
            MAX_DECOMPOSED_READ_BYTES,
        )));
    }
    Ok(std::fs::read(&abs)?)
}

/// Read a JSON file (scene.json / palettes.json / liveries.json /
/// *.materials.json) and return it as a parsed value. The frontend
/// inspects fields it knows about and ignores the rest.
#[tauri::command]
pub fn load_decomposed_json(path: String) -> Result<serde_json::Value, AppError> {
    let abs = normalize_path(&path);
    let bytes = std::fs::read(&abs)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Internal(format!("invalid JSON in '{}': {e}", abs.display())))?;
    Ok(value)
}

/// Convenience wrapper: load `scene.json` from a package directory.
/// `package_path` may be either the package directory itself
/// (`.../Packages/Polaris_LOD0_TEX2`) or the path to `scene.json`.
#[tauri::command]
pub fn load_decomposed_scene(package_path: String) -> Result<serde_json::Value, AppError> {
    let abs = normalize_path(&package_path);
    let scene_path = if abs
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("scene.json"))
        .unwrap_or(false)
    {
        abs
    } else {
        abs.join("scene.json")
    };
    load_decomposed_json(scene_path.to_string_lossy().into_owned())
}

/// Convenience wrapper: load `palettes.json` from a package directory.
/// Returns `Value::Null` if the file is absent (palette-less exports).
#[tauri::command]
pub fn load_decomposed_palettes(package_path: String) -> Result<serde_json::Value, AppError> {
    let abs = normalize_path(&package_path);
    let palettes_path = abs.join("palettes.json");
    if !palettes_path.exists() {
        return Ok(serde_json::Value::Null);
    }
    load_decomposed_json(palettes_path.to_string_lossy().into_owned())
}

/// Convenience wrapper: load `liveries.json` from a package directory.
/// Returns `Value::Null` if the file is absent.
#[tauri::command]
pub fn load_decomposed_liveries(package_path: String) -> Result<serde_json::Value, AppError> {
    let abs = normalize_path(&package_path);
    let liveries_path = abs.join("liveries.json");
    if !liveries_path.exists() {
        return Ok(serde_json::Value::Null);
    }
    load_decomposed_json(liveries_path.to_string_lossy().into_owned())
}

/// A single discovered decomposed package.
#[derive(Serialize)]
pub struct DecomposedPackageInfo {
    /// Absolute path to the package directory under `Packages/`.
    pub package_dir: String,
    /// Absolute path to the export root containing `Packages/` and `Data/`.
    pub export_root: String,
    /// The package name (e.g. `Polaris_LOD0_TEX2`).
    pub package_name: String,
    /// Whether `scene.json` exists inside the package directory.
    pub has_scene_manifest: bool,
}

/// List decomposed packages found under a root.
///
/// Accepts:
///   - an export root (directory containing `Packages/`)
///   - a `Packages/` directory directly
///   - a single package directory (returns just that package)
#[tauri::command]
pub fn list_decomposed_packages(root: String) -> Result<Vec<DecomposedPackageInfo>, AppError> {
    let abs = normalize_path(&root);
    if !abs.exists() {
        return Err(AppError::Internal(format!(
            "decomposed root '{}' does not exist",
            abs.display()
        )));
    }

    // Single-package case: caller passed the package directory itself.
    if abs.join("scene.json").exists() {
        let package_name = abs
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("(unknown)")
            .to_string();
        let export_root = derive_export_root(&abs);
        return Ok(vec![DecomposedPackageInfo {
            package_dir: abs.to_string_lossy().into_owned(),
            export_root,
            package_name,
            has_scene_manifest: true,
        }]);
    }

    // Directory case: find the `Packages/` directory.
    let packages_dir = if abs
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("Packages"))
        .unwrap_or(false)
    {
        abs.clone()
    } else {
        abs.join("Packages")
    };

    if !packages_dir.is_dir() {
        return Ok(Vec::new());
    }
    let export_root = packages_dir
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_else(|| abs.to_string_lossy().into_owned());

    let mut out = Vec::new();
    for entry in std::fs::read_dir(&packages_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let package_name = entry
            .file_name()
            .to_str()
            .map(|name| name.to_string())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let has_scene_manifest = path.join("scene.json").is_file();
        out.push(DecomposedPackageInfo {
            package_dir: path.to_string_lossy().into_owned(),
            export_root: export_root.clone(),
            package_name,
            has_scene_manifest,
        });
    }
    out.sort_by(|a, b| a.package_name.cmp(&b.package_name));
    Ok(out)
}

/// Walk up from a package directory to find the export root (the parent
/// of `Packages/`). Falls back to the package directory itself if the
/// expected layout is absent.
fn derive_export_root(package_dir: &Path) -> String {
    if let Some(parent) = package_dir.parent()
        && parent
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("Packages"))
            .unwrap_or(false)
        && let Some(grandparent) = parent.parent()
    {
        return grandparent.to_string_lossy().into_owned();
    }
    package_dir.to_string_lossy().into_owned()
}

// ── Scene Viewer self-service exporter ─────────────────────────────────
//
// The Scene Viewer browses ships/vehicles by name, hits a per-user cache
// keyed on (entity_name, export options that affect output), and exports
// missing entries via the same `assemble_glb_with_loadout_with_progress`
// API the CLI and 3D Export tab use. The cache lives under the Tauri
// `app_local_data_dir` so it survives app restarts.
//
// Layout:
//   <app_local_data_dir>/
//     decomposed_cache/
//       <entity_name>__lod<N>_mip<M>_<materials>/
//         Packages/<package_name>/{scene,palettes,liveries}.json
//         Data/Objects/.../
//
// Cache validity: a directory entry is "cache hit" if its `Packages/`
// subdir contains a package directory with a `scene.json` inside. The
// option-derived suffix on the parent dir is the cache key — change any
// option that affects output (LOD, mip, materials, attachments,
// interior, lights, nodraw) and you'll get a different cache slot.

/// Frontend-facing export options for the Scene Viewer. Mirrors the
/// fields of `starbreaker_3d::ExportOptions` that affect output, plus
/// the texture format we always render (GLB) and material kind we
/// always emit (Decomposed). Other CLI knobs (threads, format, kind)
/// are fixed to viewer-appropriate defaults inside the command handler.
#[derive(Clone, Debug, Deserialize)]
pub struct SceneExportOpts {
    pub lod: u32,
    pub mip: u32,
    /// "none" | "colors" | "textures" | "all"
    pub material_mode: String,
    pub include_attachments: bool,
    pub include_interior: bool,
    pub include_lights: bool,
    pub include_nodraw: bool,
}

impl Default for SceneExportOpts {
    fn default() -> Self {
        // Match the CLI defaults so most users never need to tweak these.
        Self {
            lod: 1,
            mip: 2,
            material_mode: "textures".to_string(),
            include_attachments: true,
            include_interior: true,
            include_lights: true,
            include_nodraw: false,
        }
    }
}

impl SceneExportOpts {
    /// Sanitized material mode token used both for `MaterialMode` and the
    /// cache key. Unknown values fall back to "textures" (mirrors CLI).
    fn material_mode_token(&self) -> &str {
        match self.material_mode.to_lowercase().as_str() {
            "none" => "none",
            "colors" => "colors",
            "all" => "all",
            _ => "textures",
        }
    }

    fn to_export_options(&self) -> starbreaker_3d::ExportOptions {
        let material_mode = match self.material_mode_token() {
            "none" => starbreaker_3d::MaterialMode::None,
            "colors" => starbreaker_3d::MaterialMode::Colors,
            "all" => starbreaker_3d::MaterialMode::All,
            _ => starbreaker_3d::MaterialMode::Textures,
        };
        starbreaker_3d::ExportOptions {
            kind: starbreaker_3d::ExportKind::Decomposed,
            format: starbreaker_3d::ExportFormat::Glb,
            material_mode,
            include_attachments: self.include_attachments,
            include_interior: self.include_interior,
            include_lights: self.include_lights,
            include_nodraw: self.include_nodraw,
            include_shields: false,
            texture_mip: self.mip,
            lod_level: self.lod,
        }
    }

    /// Suffix appended to the entity name in the cache directory.
    /// Encodes every option that affects the output bytes, plus the
    /// `_v<N>` contract-version segment. Bumping
    /// `starbreaker_3d::DECOMPOSED_CONTRACT_VERSION` shifts every cache
    /// slot to a new directory name, so old slots are orphaned (still on
    /// disk, just unreachable) until cleared by the user or pruned by
    /// `prune_stale_cache` on app start.
    fn cache_suffix(&self) -> String {
        cache_suffix_for(self, starbreaker_3d::DECOMPOSED_CONTRACT_VERSION)
    }
}

/// Standalone form of `SceneExportOpts::cache_suffix` so tests can pin
/// against a specific contract version without depending on the
/// currently-shipping value of `DECOMPOSED_CONTRACT_VERSION`.
fn cache_suffix_for(opts: &SceneExportOpts, version: u32) -> String {
    let attach = if opts.include_attachments { "1" } else { "0" };
    let interior = if opts.include_interior { "1" } else { "0" };
    let lights = if opts.include_lights { "1" } else { "0" };
    let nodraw = if opts.include_nodraw { "1" } else { "0" };
    format!(
        "lod{}_mip{}_{}_a{}_i{}_l{}_n{}_v{}",
        opts.lod,
        opts.mip,
        opts.material_mode_token(),
        attach,
        interior,
        lights,
        nodraw,
        version,
    )
}

#[derive(Clone, Serialize)]
pub struct SceneEntityDto {
    /// DataCore entity name (the export argument).
    pub entity_name: String,
    /// Localized display name when available, otherwise None.
    pub display_name: Option<String>,
    /// Category bucket, currently "Ships" or "Ground Vehicles".
    pub category: String,
    /// True if a previous export with the same options is on disk.
    pub cached: bool,
}

#[derive(Clone, Serialize)]
pub struct SceneExportProgress {
    pub fraction: f32,
    pub stage: String,
    pub entity_name: String,
}

#[derive(Clone, Serialize)]
pub struct SceneExportDone {
    pub entity_name: String,
    pub package_dir: Option<String>,
    pub error: Option<String>,
}

fn scene_cache_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let local = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Internal(format!("app_local_data_dir unavailable: {e}")))?;
    Ok(local.join("decomposed_cache"))
}

fn scene_cache_dir(app: &AppHandle, entity_name: &str, opts: &SceneExportOpts) -> Result<PathBuf, AppError> {
    let safe_name = sanitize_cache_segment(entity_name);
    let suffix = opts.cache_suffix();
    Ok(scene_cache_root(app)?.join(format!("{safe_name}__{suffix}")))
}

/// Find the package directory inside an export root, or None if absent.
fn find_package_with_scene(export_root: &Path) -> Option<PathBuf> {
    let packages = export_root.join("Packages");
    if !packages.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(&packages).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("scene.json").is_file() {
            return Some(path);
        }
    }
    None
}

/// Replace characters that are awkward in filesystem paths. Keeps the
/// entity name human-readable (no hashing) so cache dirs are scrubbable
/// by hand.
fn sanitize_cache_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.' => out.push(ch),
            ' ' => out.push('_'),
            _ => out.push('_'),
        }
    }
    if out.is_empty() {
        "Entity".to_string()
    } else {
        out
    }
}

/// Decomposed exporter package-name format: `<EXPORT_NAME>_LOD<n>_TEX<n>`.
/// Used to know which package directory we'll write under `Packages/`.
fn predicted_package_name(entity_name: &str, opts: &SceneExportOpts) -> String {
    let display = sanitize_export_name(&strip_record_prefix(entity_name));
    format!("{display}_LOD{}_TEX{}", opts.lod, opts.mip)
}

fn strip_record_prefix(name: &str) -> String {
    let trimmed = name.trim_matches('"');
    trimmed
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

fn sanitize_export_name(name: &str) -> String {
    let mut cleaned = String::new();
    let mut last_was_space = false;
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            cleaned.push(ch);
            last_was_space = false;
        } else if ch.is_whitespace() || matches!(ch, '_' | '-') {
            if !cleaned.is_empty() && !last_was_space {
                cleaned.push(' ');
                last_was_space = true;
            }
        }
    }
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Export".to_string()
    } else {
        cleaned.to_string()
    }
}

/// List ships and vehicles available for scene viewing. Marks any
/// entity that already has a cached export with the given default opts
/// so the UI can render a "cached" badge without a second round-trip.
///
/// `opts` is optional. When None, cache_status is computed against
/// `SceneExportOpts::default()`.
#[tauri::command]
pub async fn list_scene_entities(
    app: AppHandle,
    state: State<'_, AppState>,
    opts: Option<SceneExportOpts>,
) -> Result<Vec<SceneEntityDto>, AppError> {
    let opts = opts.unwrap_or_default();
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?
            .clone()
    };
    let loc = {
        let guard = state.localization.lock();
        guard.clone()
    };
    let cache_root = scene_cache_root(&app)?;

    tokio::task::spawn_blocking(move || {
        let db = Database::from_bytes(&dcb_bytes)?;

        use starbreaker_datacore::QueryResultExt;
        use starbreaker_datacore::query::value::Value;

        let loc_compiled = db.compile_rooted::<Value>(
            "EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Localization.Name",
        ).optional()?;
        let inclusion_compiled = db.compile_rooted::<Value>(
            "EntityClassDefinition.StaticEntityClassData[EAEntityDataParams].inclusionMode",
        ).optional()?;

        let mut out: Vec<SceneEntityDto> = Vec::new();
        for record in db.records_by_type_name("EntityClassDefinition") {
            if !db.is_main_record(record) {
                continue;
            }
            let file_path = db.resolve_string(record.file_name_offset).to_lowercase();
            let category = if file_path.contains("entities/spaceships") {
                "Ships"
            } else if file_path.contains("entities/groundvehicles") {
                "Ground Vehicles"
            } else {
                continue;
            };

            // Skip non-player variants — same rule as the 3D Export tab.
            let is_npc_or_internal = inclusion_compiled.as_ref()
                .and_then(|c| db.query_single::<Value>(c, record).ok().flatten())
                .is_some_and(|v| matches!(v, Value::Enum(s) if s != "ReadyToInclude"));
            if is_npc_or_internal {
                continue;
            }

            let entity_name = db.resolve_string2(record.name_offset).to_string();
            let display_name = loc_compiled.as_ref()
                .and_then(|c| db.query_single::<Value>(c, record).ok().flatten())
                .and_then(|v| match v {
                    Value::String(s) | Value::Locale(s) => Some(s.to_string()),
                    _ => None,
                })
                .filter(|s| !s.is_empty() && s != "@LOC_UNINITIALIZED" && s != "@LOC_EMPTY")
                .and_then(|key| {
                    let stripped = key.strip_prefix('@').unwrap_or(&key);
                    loc.get(&stripped.to_lowercase()).cloned()
                });

            let safe = sanitize_cache_segment(&entity_name);
            let cache_dir = cache_root.join(format!("{safe}__{}", opts.cache_suffix()));
            let cached = find_package_with_scene(&cache_dir).is_some();

            out.push(SceneEntityDto {
                entity_name,
                display_name,
                category: category.to_string(),
                cached,
            });
        }

        let sort_key = |e: &SceneEntityDto| {
            e.display_name.clone().unwrap_or_else(|| e.entity_name.clone())
        };
        out.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        Ok::<_, AppError>(out)
    })
    .await
    .map_err(|e| AppError::Internal(format!("task join error: {e}")))?
}

/// Return the cache directory that `start_scene_export` would write to
/// for a given (entity, opts) pair. Frontend uses this both to render
/// the cache status and to mount the viewer when a cache hit exists.
#[tauri::command]
pub fn get_scene_cache_path(
    app: AppHandle,
    entity_name: String,
    opts: SceneExportOpts,
) -> Result<SceneCachePath, AppError> {
    let export_root = scene_cache_dir(&app, &entity_name, &opts)?;
    let package_dir = find_package_with_scene(&export_root);
    Ok(SceneCachePath {
        export_root: export_root.to_string_lossy().into_owned(),
        package_dir: package_dir.map(|p| p.to_string_lossy().into_owned()),
        cached: package_dir_predicate(&export_root, entity_name.as_str(), &opts),
    })
}

/// True when the cache slot already holds a usable scene.json. This is
/// the same predicate as `find_package_with_scene().is_some()` but
/// expressed as a helper so the meaning is named at the call site.
fn package_dir_predicate(export_root: &Path, _entity_name: &str, _opts: &SceneExportOpts) -> bool {
    find_package_with_scene(export_root).is_some()
}

#[derive(Serialize)]
pub struct SceneCachePath {
    pub export_root: String,
    /// None when no scene.json has been written yet.
    pub package_dir: Option<String>,
    pub cached: bool,
}

/// Delete the on-disk cache for a single (entity, opts) pair. The
/// `_LOD0_TEX0` style suffix on the package dir means changing options
/// gives a different cache slot, so this only nukes the slot the
/// frontend was about to re-export into.
#[tauri::command]
pub fn clear_scene_cache(
    app: AppHandle,
    entity_name: String,
    opts: SceneExportOpts,
) -> Result<(), AppError> {
    let dir = scene_cache_dir(&app, &entity_name, &opts)?;
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| {
            AppError::Internal(format!("failed to clear cache '{}': {e}", dir.display()))
        })?;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ClearAllResult {
    /// Number of cache entries (top-level directories under
    /// `decomposed_cache/`) that were removed.
    pub entries_removed: u32,
    /// Total bytes freed across all removed entries (best-effort: sums
    /// the sizes seen during the pre-delete walk).
    pub bytes_freed: u64,
}

/// Wipe every entry under the per-user `decomposed_cache/` directory.
///
/// Defensive contract:
/// - Only deletes immediate children of the cache root; never follows
///   symlinks out, never accepts arbitrary paths from the caller.
/// - The cache root itself is recreated empty after the wipe so the
///   next export does not have to materialize it.
/// - Per-entry failures do not abort the whole pass; they are counted
///   but the loop continues so a single locked file does not strand
///   the rest of the cache.
#[tauri::command]
pub fn clear_all_scene_cache(app: AppHandle) -> Result<ClearAllResult, AppError> {
    let root = scene_cache_root(&app)?;
    if !root.is_dir() {
        return Ok(ClearAllResult {
            entries_removed: 0,
            bytes_freed: 0,
        });
    }

    let mut entries_removed: u32 = 0;
    let mut bytes_freed: u64 = 0;

    for entry in std::fs::read_dir(&root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        // Symlink defence: never traverse out of the cache root via a
        // symlinked entry. `symlink_metadata` reports the link itself,
        // not the target.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            // Remove the link itself but never follow it.
            if std::fs::remove_file(&path).is_ok() {
                entries_removed += 1;
            }
            continue;
        }
        if meta.is_dir() {
            bytes_freed = bytes_freed.saturating_add(directory_size(&path));
            if std::fs::remove_dir_all(&path).is_ok() {
                entries_removed += 1;
            }
        } else if meta.is_file() {
            bytes_freed = bytes_freed.saturating_add(meta.len());
            if std::fs::remove_file(&path).is_ok() {
                entries_removed += 1;
            }
        }
    }

    // Recreate the cache root empty so subsequent exports don't have to.
    let _ = std::fs::create_dir_all(&root);

    Ok(ClearAllResult {
        entries_removed,
        bytes_freed,
    })
}

#[derive(Serialize)]
pub struct CacheStats {
    /// Number of top-level entries (one per cached entity+opts slot).
    pub entry_count: u32,
    /// Total bytes used on disk across every cached slot.
    pub total_bytes: u64,
    /// Absolute path to the cache root, useful for "show in
    /// explorer" affordances.
    pub cache_root: String,
}

/// Walk `decomposed_cache/` and return entry count and total size.
/// Cheap enough to call on every UI refresh: a typical cache has under
/// 50 top-level entries; the walk is one stat per file.
#[tauri::command]
pub fn cache_stats(app: AppHandle) -> Result<CacheStats, AppError> {
    let root = scene_cache_root(&app)?;
    if !root.is_dir() {
        return Ok(CacheStats {
            entry_count: 0,
            total_bytes: 0,
            cache_root: root.to_string_lossy().into_owned(),
        });
    }

    let mut entry_count: u32 = 0;
    let mut total_bytes: u64 = 0;

    for entry in std::fs::read_dir(&root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            // Count the link, but don't traverse it.
            entry_count += 1;
            continue;
        }
        if meta.is_dir() {
            entry_count += 1;
            total_bytes = total_bytes.saturating_add(directory_size(&entry.path()));
        } else if meta.is_file() {
            entry_count += 1;
            total_bytes = total_bytes.saturating_add(meta.len());
        }
    }

    Ok(CacheStats {
        entry_count,
        total_bytes,
        cache_root: root.to_string_lossy().into_owned(),
    })
}

#[derive(Serialize)]
pub struct PruneStaleResult {
    /// Number of cache slots removed because their `_v<N>` segment did
    /// not match `current_version`, plus any entries whose name lacked
    /// a recognizable `_v<N>` segment entirely (legacy / unversioned).
    pub entries_removed: u32,
    /// Bytes freed by the prune.
    pub bytes_freed: u64,
}

/// Remove cache slots whose contract-version segment does not match
/// `current_version`. Optional housekeeping the frontend can call from
/// `useEffect` on mount so old slots don't hang around indefinitely
/// after a contract bump.
///
/// Pruning rule: any top-level entry whose name does not end in
/// `_v<current_version>` is removed. Entries from prior tool versions
/// that pre-date the version stamp won't have a `_v<N>` segment and
/// will also be pruned, which is the desired effect — they were
/// rendered unreachable by the introduction of the version segment.
#[tauri::command]
pub fn prune_stale_cache(
    app: AppHandle,
    current_version: u32,
) -> Result<PruneStaleResult, AppError> {
    let root = scene_cache_root(&app)?;
    if !root.is_dir() {
        return Ok(PruneStaleResult {
            entries_removed: 0,
            bytes_freed: 0,
        });
    }

    let expected_suffix = format!("_v{current_version}");
    let mut entries_removed: u32 = 0;
    let mut bytes_freed: u64 = 0;

    for entry in std::fs::read_dir(&root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name.ends_with(&expected_suffix) {
            continue;
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            if std::fs::remove_file(&path).is_ok() {
                entries_removed += 1;
            }
            continue;
        }
        if meta.is_dir() {
            bytes_freed = bytes_freed.saturating_add(directory_size(&path));
            if std::fs::remove_dir_all(&path).is_ok() {
                entries_removed += 1;
            }
        } else if meta.is_file() {
            bytes_freed = bytes_freed.saturating_add(meta.len());
            if std::fs::remove_file(&path).is_ok() {
                entries_removed += 1;
            }
        }
    }

    Ok(PruneStaleResult {
        entries_removed,
        bytes_freed,
    })
}

/// Sum file sizes under a directory tree. Symlinks encountered during
/// the walk are counted by their link size, not their target — so a
/// link out of the cache cannot inflate the reported total via the
/// target's size, and cannot cause us to traverse outside the cache
/// root. Errors during the walk are swallowed because the result is
/// used for display, not correctness.
fn directory_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let meta = match std::fs::symlink_metadata(&p) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                // Skip — never traverse links during the walk.
                continue;
            }
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// Find the EntityClassDefinition record for an exact entity name, or
/// fall back to the shortest case-insensitive substring match (mirrors
/// CLI's `find_candidates`).
fn resolve_entity_record<'a>(db: &'a Database, name: &str) -> Result<&'a Record, AppError> {
    if let Some(rec) = db.records_by_type_name("EntityClassDefinition")
        .filter(|r| db.is_main_record(r))
        .find(|r| db.resolve_string2(r.name_offset) == name)
    {
        return Ok(rec);
    }
    let lower = name.to_lowercase();
    let mut candidates: Vec<&Record> = db.records_by_type_name("EntityClassDefinition")
        .filter(|r| db.is_main_record(r))
        .filter(|r| db.resolve_string2(r.name_offset).to_lowercase().contains(&lower))
        .collect();
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal(format!("entity '{name}' not found in DataCore")))
}

/// Write the decomposed export to `<export_root>/Packages/<pkg>` and
/// `<export_root>/Data/...`. This is the cache write path; subsequent
/// calls with the same (entity, opts) skip the export entirely.
fn write_scene_export(
    export_root: &Path,
    files: &[starbreaker_3d::ExportedFile],
    package_name: &str,
) -> Result<(), AppError> {
    // Wipe just the package directory so material sidecars from a previous
    // run can't bleed into a new export. Data/* assets are reused (cheap).
    let package_root = export_root.join("Packages").join(package_name);
    if package_root.exists() {
        std::fs::remove_dir_all(&package_root).map_err(|e| {
            AppError::Internal(format!(
                "failed to clear stale package dir '{}': {e}",
                package_root.display(),
            ))
        })?;
    }
    std::fs::create_dir_all(&package_root)?;

    for file in files {
        let path = export_root.join(&file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.bytes)?;
    }
    Ok(())
}

async fn pump_scene_progress(
    app: AppHandle,
    entity_name: String,
    progress: Arc<Progress>,
    done: Arc<AtomicBool>,
) {
    // Hard cap displayed progress at 0.99 until the export task signals
    // completion. The internal Progress value is bounded at 1.0 by
    // Progress::report's clamp, but stale completion writes from one
    // phase have caused the UI to flash >100% in the past. Capping here
    // means the bar can never overshoot, and we deliberately render a
    // "Finishing..." stage when we hit the ceiling so the user knows
    // the run is still alive at the cap.
    loop {
        let is_done = done.load(Ordering::Relaxed);
        let (raw_fraction, raw_stage) = progress.get();
        let (fraction, stage) = if is_done {
            (1.0_f32, raw_stage)
        } else if raw_fraction >= 0.99 {
            (
                0.99_f32,
                if raw_stage.is_empty() {
                    "Finishing...".to_string()
                } else {
                    raw_stage
                },
            )
        } else {
            (raw_fraction.clamp(0.0, 0.99), raw_stage)
        };
        let _ = app.emit(
            "scene-export-progress",
            SceneExportProgress {
                fraction,
                stage,
                entity_name: entity_name.clone(),
            },
        );
        if is_done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Run the decomposed exporter for a single entity in-process. Writes
/// to the cache slot for the given options and emits progress on
/// `scene-export-progress`. Always emits a single `scene-export-done`
/// when the task finishes (success or failure).
#[tauri::command]
pub async fn start_scene_export(
    app: AppHandle,
    state: State<'_, AppState>,
    entity_name: String,
    opts: SceneExportOpts,
) -> Result<(), AppError> {
    state.scene_export_cancel.store(false, Ordering::SeqCst);

    let p4k = {
        let guard = state.p4k.lock();
        guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("P4k not loaded".into()))?
            .clone()
    };
    let dcb_bytes = {
        let guard = state.dcb_bytes.lock();
        guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("DataCore not loaded".into()))?
            .clone()
    };
    let cancel = state.scene_export_cancel.clone();
    let export_root = scene_cache_dir(&app, &entity_name, &opts)?;
    let predicted_package = predicted_package_name(&entity_name, &opts);

    let progress = Arc::new(Progress::new());
    let done_flag = Arc::new(AtomicBool::new(false));
    tauri::async_runtime::spawn(pump_scene_progress(
        app.clone(),
        entity_name.clone(),
        progress.clone(),
        done_flag.clone(),
    ));

    let app_for_done = app.clone();
    let entity_for_done = entity_name.clone();
    tokio::task::spawn_blocking(move || {
        let result = run_scene_export_blocking(
            &p4k,
            &dcb_bytes,
            &entity_name,
            &export_root,
            &predicted_package,
            &opts,
            &progress,
            &cancel,
        );
        progress.report(1.0, "Done");
        done_flag.store(true, Ordering::Relaxed);
        let _ = app_for_done.emit(
            "scene-export-done",
            match result {
                Ok(pkg) => SceneExportDone {
                    entity_name: entity_for_done,
                    package_dir: Some(pkg.to_string_lossy().into_owned()),
                    error: None,
                },
                Err(e) => SceneExportDone {
                    entity_name: entity_for_done,
                    package_dir: None,
                    error: Some(e.to_string()),
                },
            },
        );
    });

    Ok(())
}

/// Cancel the in-flight scene export, if any. Cancellation is checked
/// before each entity's resolve step, so the current export may still
/// finish writing files before stopping.
#[tauri::command]
pub fn cancel_scene_export(state: State<'_, AppState>) {
    state.scene_export_cancel.store(true, Ordering::SeqCst);
}

#[allow(clippy::too_many_arguments)]
fn run_scene_export_blocking(
    p4k: &MappedP4k,
    dcb_bytes: &[u8],
    entity_name: &str,
    export_root: &Path,
    predicted_package: &str,
    opts: &SceneExportOpts,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<PathBuf, AppError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::Internal("export cancelled".into()));
    }

    progress.report(0.02, "Loading DataCore");
    let db = Database::from_bytes(dcb_bytes)?;

    progress.report(0.05, "Resolving entity");
    let record = resolve_entity_record(&db, entity_name)?;

    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::Internal("export cancelled".into()));
    }

    progress.report(0.10, "Resolving loadout");
    let idx = EntityIndex::new(&db);
    let tree = resolve_loadout_indexed(&idx, record);

    let export_opts = opts.to_export_options();
    progress.report(0.20, "Assembling scene");

    let result = starbreaker_3d::assemble_glb_with_loadout_with_progress(
        &db,
        p4k,
        record,
        &tree,
        &export_opts,
        Some(progress),
        None,
    )?;

    let decomposed = result
        .decomposed
        .as_ref()
        .ok_or_else(|| AppError::Internal("scene export returned no decomposed files".into()))?;

    progress.report(0.92, "Writing package");
    write_scene_export(export_root, &decomposed.files, predicted_package)?;

    let package_dir = find_package_with_scene(export_root).ok_or_else(|| {
        AppError::Internal(format!(
            "scene export wrote files but no scene.json found under '{}'",
            export_root.display()
        ))
    })?;

    progress.report(0.99, "Finalizing");
    Ok(package_dir)
}

#[cfg(test)]
mod scene_tests {
    use super::*;

    #[test]
    fn cache_suffix_changes_with_each_relevant_option() {
        let base = SceneExportOpts::default();
        let suffix_default = base.cache_suffix();
        let mut alt = base.clone();
        alt.lod = 2;
        assert_ne!(suffix_default, alt.cache_suffix());
        let mut alt = base.clone();
        alt.mip = 4;
        assert_ne!(suffix_default, alt.cache_suffix());
        let mut alt = base.clone();
        alt.material_mode = "colors".into();
        assert_ne!(suffix_default, alt.cache_suffix());
        let mut alt = base.clone();
        alt.include_attachments = false;
        assert_ne!(suffix_default, alt.cache_suffix());
        let mut alt = base.clone();
        alt.include_interior = false;
        assert_ne!(suffix_default, alt.cache_suffix());
        let mut alt = base.clone();
        alt.include_lights = false;
        assert_ne!(suffix_default, alt.cache_suffix());
        let mut alt = base.clone();
        alt.include_nodraw = true;
        assert_ne!(suffix_default, alt.cache_suffix());
    }

    #[test]
    fn cache_suffix_is_stable_for_unknown_material_mode() {
        // Unknown material values fall back to "textures" both for the
        // export options and the cache key. Two unknown values must
        // produce the same cache slot.
        let mut a = SceneExportOpts::default();
        a.material_mode = "garbage".into();
        let mut b = SceneExportOpts::default();
        b.material_mode = "alsobad".into();
        assert_eq!(a.cache_suffix(), b.cache_suffix());
    }

    #[test]
    fn sanitize_cache_segment_strips_path_chars() {
        assert_eq!(sanitize_cache_segment("ARGO_MOLE"), "ARGO_MOLE");
        assert_eq!(sanitize_cache_segment("RSI Polaris"), "RSI_Polaris");
        assert_eq!(sanitize_cache_segment("name/with\\stuff"), "name_with_stuff");
    }

    #[test]
    fn predicted_package_name_matches_decomposed_layout() {
        let opts = SceneExportOpts::default();
        let name = predicted_package_name("RSI_Polaris", &opts);
        // Default lod=1, mip=2 → matches "..._LOD1_TEX2".
        assert_eq!(name, "RSI Polaris_LOD1_TEX2");
    }

    #[test]
    fn cache_suffix_distinguishes_fast_preview_from_full() {
        // Fast preview (no interiors) must not collide with the full
        // export's cache slot. The `_i0` vs `_i1` segment in the
        // suffix is what makes this true.
        let mut full = SceneExportOpts::default();
        full.include_interior = true;
        let mut fast = SceneExportOpts::default();
        fast.include_interior = false;
        assert!(full.cache_suffix().contains("_i1_"));
        assert!(fast.cache_suffix().contains("_i0_"));
        assert_ne!(full.cache_suffix(), fast.cache_suffix());
    }

    #[test]
    fn cache_suffix_carries_contract_version_segment() {
        // Bumping the contract version must shift every cache slot to a
        // new directory name so old slots become unreachable. The
        // `_v<N>` segment at the tail is the mechanism.
        let opts = SceneExportOpts::default();
        let v1 = cache_suffix_for(&opts, 1);
        let v2 = cache_suffix_for(&opts, 2);
        assert!(v1.ends_with("_v1"), "expected v1 suffix tail, got {v1}");
        assert!(v2.ends_with("_v2"), "expected v2 suffix tail, got {v2}");
        assert_ne!(v1, v2);
    }

    #[test]
    fn cache_suffix_matches_current_contract_version() {
        // The runtime suffix must end in the current contract version.
        // If this test fails after a CONTRACT_VERSION bump, that's
        // expected — both numbers move together.
        let opts = SceneExportOpts::default();
        let runtime = opts.cache_suffix();
        let expected_tail = format!("_v{}", starbreaker_3d::DECOMPOSED_CONTRACT_VERSION);
        assert!(
            runtime.ends_with(&expected_tail),
            "runtime suffix {runtime} does not end in {expected_tail}",
        );
    }

    #[test]
    fn cache_suffix_for_is_deterministic_for_same_inputs() {
        let opts = SceneExportOpts::default();
        assert_eq!(cache_suffix_for(&opts, 1), cache_suffix_for(&opts, 1));
        assert_eq!(cache_suffix_for(&opts, 7), cache_suffix_for(&opts, 7));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_handles_backslashes() {
        let result = normalize_path("C:\\foo\\bar\\baz.glb");
        assert_eq!(result, PathBuf::from("C:/foo/bar/baz.glb"));
    }

    #[test]
    fn derive_export_root_finds_grandparent_of_packages() {
        let pkg = PathBuf::from("/tmp/export/Packages/Polaris_LOD0_TEX2");
        assert_eq!(derive_export_root(&pkg), "/tmp/export");
    }

    #[test]
    fn derive_export_root_falls_back_when_no_packages_parent() {
        let pkg = PathBuf::from("/tmp/standalone");
        assert_eq!(derive_export_root(&pkg), "/tmp/standalone");
    }
}
