use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::{EntityIndex, LoadoutNode, resolve_loadout_indexed};
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::common::{ExportOpts, load_dcb_bytes};
use crate::error::{CliError, Result};

fn bundled_extension(format: starbreaker_3d::ExportFormat) -> &'static str {
    match format {
        starbreaker_3d::ExportFormat::Glb => "glb",
        starbreaker_3d::ExportFormat::Stl => "stl",
        starbreaker_3d::ExportFormat::Blend => "blend",
    }
}

fn export_entity_name(name: &str) -> String {
    let trimmed = name.trim_matches('"');
    trimmed
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

pub(crate) fn sanitize_export_name(name: &str) -> String {
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

pub(crate) fn prepare_decomposed_output_root(output_root: &PathBuf, package_name: &str) -> Result<()> {
    if output_root.exists() {
        if output_root.is_file() {
            return Err(CliError::InvalidInput(format!(
                "decomposed output root '{}' already exists as a file",
                output_root.display(),
            )));
        }
    }

    let packages_root = output_root.join("Packages");
    let package_root = packages_root.join(package_name);
    std::fs::create_dir_all(&package_root)
        .map_err(|e| CliError::IoPath { source: e, path: package_root.display().to_string() })?;
    Ok(())
}

fn should_skip_existing_decomposed_asset(
    file: &starbreaker_3d::ExportedFile,
    skip_existing_assets: bool,
) -> bool {
    skip_existing_assets && file.kind.is_reusable_asset()
}

pub(crate) fn write_decomposed_file(
    file: &starbreaker_3d::ExportedFile,
    output_path: &PathBuf,
    skip_existing_assets: bool,
    p4k: &MappedP4k,
) -> Result<()> {
    if output_path.exists() {
        if !output_path.is_file() {
            return Err(CliError::InvalidInput(format!(
                "decomposed output path '{}' already exists as a directory",
                output_path.display(),
            )));
        }
        if should_skip_existing_decomposed_asset(file, skip_existing_assets)
            && existing_asset_matches_source_mtime(output_path, p4k, &file.relative_path)?
        {
            return Ok(());
        }
    }

    std::fs::write(output_path, &file.bytes)
        .map_err(|e| CliError::IoPath { source: e, path: output_path.display().to_string() })?;
    apply_source_mtime(output_path, p4k, &file.relative_path)?;
    Ok(())
}

fn should_write_ui_only_decomposed_file(relative_path: &str) -> bool {
    let normalized = relative_path.replace('\\', "/").to_ascii_lowercase();
    normalized.starts_with("data/ui/") || normalized.ends_with("/scene.json")
}

fn collect_existing_decomposed_assets(output_root: &Path, p4k: &MappedP4k) -> Result<HashSet<String>> {
    let data_root = output_root.join("Data");
    let mut existing = HashSet::new();
    if !data_root.exists() {
        return Ok(existing);
    }

    let mut pending = vec![data_root];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| CliError::IoPath { source: e, path: dir.display().to_string() })?
        {
            let entry = entry
                .map_err(|e| CliError::IoPath { source: e, path: dir.display().to_string() })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?;
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            let filename = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            if !matches!(extension, "blend" | "png" | "exr") && !filename.ends_with(".materials.json") {
                continue;
            }

            let relative = path
                .strip_prefix(output_root)
                .map_err(|_| {
                    CliError::InvalidInput(format!(
                        "failed to compute relative decomposed asset path for '{}'",
                        path.display(),
                    ))
                })?
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if !existing_asset_matches_source_mtime(&path, p4k, &relative)? {
                continue;
            }
            existing.insert(relative);
        }
    }

    Ok(existing)
}

fn existing_asset_matches_source_mtime(path: &Path, p4k: &MappedP4k, relative_path: &str) -> Result<bool> {
    let Some(source_seconds) = source_modified_unix_seconds(p4k, relative_path) else {
        return Ok(false);
    };
    let modified = path
        .metadata()
        .and_then(|metadata| metadata.modified())
        .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?;
    let Ok(asset_seconds) = modified.duration_since(UNIX_EPOCH) else {
        return Ok(false);
    };
    Ok(asset_seconds.as_secs() == source_seconds)
}

fn apply_source_mtime(path: &Path, p4k: &MappedP4k, relative_path: &str) -> Result<()> {
    let Some(seconds) = source_modified_unix_seconds(p4k, relative_path) else {
        return Ok(());
    };
    let file_time = filetime::FileTime::from_unix_time(seconds as i64, 0);
    filetime::set_file_mtime(path, file_time)
        .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })
}

fn source_modified_unix_seconds(p4k: &MappedP4k, relative_path: &str) -> Option<u64> {
    source_asset_candidates(relative_path)
        .into_iter()
        .find_map(|candidate| p4k.entry_case_insensitive(&candidate).map(|entry| dos_datetime_to_unix_seconds(entry.last_modified)))
}

fn source_asset_candidates(relative_path: &str) -> Vec<String> {
    let path = relative_path.replace('/', "\\");
    if let Some(stem) = path.strip_suffix(".materials.json") {
        return vec![format!("{}.mtl", strip_numeric_suffix(stem, "_tex"))];
    }
    if let Some(stem) = path.strip_suffix(".blend") {
        let source_stem = strip_numeric_suffix(stem, "_lod");
        return [".cgfm", ".cgam", ".skinm", ".cgf", ".cga", ".skin", ".chr"]
            .into_iter()
            .map(|extension| format!("{source_stem}{extension}"))
            .collect();
    }
    if let Some(stem) = path.strip_suffix(".png") {
        let source_stem = strip_numeric_suffix(stem, "_tex");
        return [".dds", ".tif", ".png"]
            .into_iter()
            .map(|extension| format!("{source_stem}{extension}"))
            .collect();
    }
    if let Some(stem) = path.strip_suffix(".exr") {
        return [".dds", ".tif", ".exr"]
            .into_iter()
            .map(|extension| format!("{stem}{extension}"))
            .collect();
    }
    Vec::new()
}

fn strip_numeric_suffix<'a>(value: &'a str, marker: &str) -> &'a str {
    let lower = value.to_ascii_lowercase();
    let Some(pos) = lower.rfind(marker) else {
        return value;
    };
    let suffix = &value[pos + marker.len()..];
    if !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()) {
        &value[..pos]
    } else {
        value
    }
}

fn dos_datetime_to_unix_seconds(value: u32) -> u64 {
    let time = value & 0xffff;
    let date = value >> 16;
    let second = (time & 0x1f) * 2;
    let minute = (time >> 5) & 0x3f;
    let hour = (time >> 11) & 0x1f;
    let day = (date & 0x1f).max(1);
    let month = ((date >> 5) & 0x0f).clamp(1, 12);
    let year = 1980 + ((date >> 9) & 0x7f) as i32;
    (days_from_civil(year, month, day) as u64) * 86_400
        + (hour as u64) * 3_600
        + (minute as u64) * 60
        + second as u64
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) as i64
}

#[cfg(test)]
mod tests {
    use super::{
        prepare_decomposed_output_root, should_write_ui_only_decomposed_file, source_asset_candidates,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn source_asset_candidates_strip_lod_and_tex_suffixes_case_insensitively() {
        assert_eq!(
            source_asset_candidates("data/objects/foo_LOD0.blend")[0],
            "data\\objects\\foo.cgfm"
        );
        assert_eq!(
            source_asset_candidates("data/textures/foo_tex0.png")[0],
            "data\\textures\\foo.dds"
        );
        assert_eq!(
            source_asset_candidates("data/materials/foo_TEX0.materials.json")[0],
            "data\\materials\\foo.mtl"
        );
    }

    #[test]
    fn prepare_decomposed_output_root_preserves_existing_packages() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("starbreaker_prepare_package_test_{unique}"));
        let target_package = root.join("Packages").join("ship").join("DRAK Buccaneer_LOD0_TEX0");
        let sibling_package = root.join("Packages").join("ship").join("DRAK Ironclad_LOD0_TEX0");
        std::fs::create_dir_all(&target_package).unwrap();
        std::fs::create_dir_all(&sibling_package).unwrap();
        std::fs::write(target_package.join("scene.json"), b"old").unwrap();
        std::fs::write(sibling_package.join("scene.json"), b"sibling").unwrap();

        prepare_decomposed_output_root(
            &root,
            "ship/DRAK Buccaneer_LOD0_TEX0",
        )
        .unwrap();

        assert_eq!(std::fs::read(target_package.join("scene.json")).unwrap(), b"old");
        assert_eq!(std::fs::read(sibling_package.join("scene.json")).unwrap(), b"sibling");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ui_only_filter_includes_ui_assets_and_scene_sidecars() {
        assert!(should_write_ui_only_decomposed_file(
            "Data/UI/Generated/ship/drak/Clipper/buildingblocks_canvas_i_door_small_drak.png"
        ));
        assert!(should_write_ui_only_decomposed_file(
            "data/ui/generated/ship/drak/clipper/buildingblocks_canvas_i_door_small_drak.png"
        ));
        assert!(should_write_ui_only_decomposed_file(
            "Packages/DRAK Clipper_LOD0_TEX0/scene.json"
        ));
    }

    #[test]
    fn ui_only_filter_excludes_non_ui_assets() {
        assert!(!should_write_ui_only_decomposed_file(
            "Data/Objects/ships/drak/clipper/clipper_lod0.blend"
        ));
        assert!(!should_write_ui_only_decomposed_file(
            "Data/Textures/ships/drak/clipper/albedo.dds"
        ));
    }
}

fn collect_existing_interior_assets(output_root: &Path) -> Result<HashMap<String, (String, Option<String>)>> {
    let packages_root = output_root.join("Packages");
    let mut existing = HashMap::new();
    if !packages_root.exists() {
        return Ok(existing);
    }

    let mut pending = vec![packages_root];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| CliError::IoPath { source: e, path: dir.display().to_string() })?
        {
            let entry = entry
                .map_err(|e| CliError::IoPath { source: e, path: dir.display().to_string() })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?;
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() || path.file_name().and_then(|name| name.to_str()) != Some("scene.json") {
                continue;
            }
            let bytes = std::fs::read(&path)
                .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?;
            collect_scene_interior_assets(&mut existing, &bytes);
        }
    }

    Ok(existing)
}

fn collect_scene_interior_assets(
    existing: &mut HashMap<String, (String, Option<String>)>,
    scene_bytes: &[u8],
) {
    let Ok(scene) = serde_json::from_slice::<serde_json::Value>(scene_bytes) else {
        return;
    };
    let Some(interiors) = scene.get("interiors").and_then(|value| value.as_array()) else {
        return;
    };
    for placement in interiors
        .iter()
        .filter_map(|interior| interior.get("placements").and_then(|value| value.as_array()))
        .flatten()
    {
        let Some(cgf_path) = placement.get("cgf_path").and_then(|value| value.as_str()) else {
            continue;
        };
        let material_path = placement.get("material_path").and_then(|value| value.as_str());
        let Some(mesh_asset) = placement.get("mesh_asset").and_then(|value| value.as_str()) else {
            continue;
        };
        let material_sidecar = placement
            .get("material_sidecar")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned);
        let key = interior_asset_lookup_key(cgf_path, material_path);
        existing.entry(key).or_insert_with(|| (mesh_asset.to_string(), material_sidecar));
    }
}

fn interior_asset_lookup_key(cgf_path: &str, material_path: Option<&str>) -> String {
    format!(
        "{}|{}",
        cgf_path.to_ascii_lowercase(),
        material_path.unwrap_or("").to_ascii_lowercase()
    )
}

fn read_reusable_decomposed_asset(
    output_root: &Path,
    reusable_asset_paths: &HashSet<String>,
    relative_path: &str,
) -> Option<Vec<u8>> {
    let key = relative_path.replace('\\', "/").to_ascii_lowercase();
    if !reusable_asset_paths.contains(&key) {
        return None;
    }
    std::fs::read(output_root.join(relative_path)).ok()
}

#[derive(Subcommand)]
pub enum EntityCommand {
    /// Export entity to a bundled file
    Export {
        /// Entity name (substring, case-insensitive)
        name: String,
        /// Output bundled file path
        output: Option<PathBuf>,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Write hierarchy JSON instead of GLB
        #[arg(long)]
        dump_hierarchy: bool,
        #[command(flatten)]
        opts: ExportOpts,
    },
    /// Print entity loadout tree
    Loadout {
        /// Entity name (substring, case-insensitive)
        name: String,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Validate an existing decomposed entity export directory
    Validate {
        /// Decomposed export root directory
        output: PathBuf,
    },
}

impl EntityCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Export {
                name,
                output,
                p4k,
                dump_hierarchy,
                opts,
            } => export(name, output, p4k, dump_hierarchy, opts),
            Self::Loadout { name, p4k } => loadout(name, p4k),
            Self::Validate { output } => validate(output),
        }
    }
}

fn validate(output: PathBuf) -> Result<()> {
    let report = starbreaker_3d::validate_decomposed_export(&output)?;
    println!("Valid:              {}", report.is_valid);
    println!("Meshes:             {}", report.mesh_count);
    println!("Lights:             {}", report.light_count);
    println!("Empties:            {}", report.empty_count);
    println!("Materials:          {}", report.material_count);
    println!("Vertex groups:      {}", report.vertex_group_count);
    println!("Warnings:           {}", report.warnings.len());
    println!("Errors:             {}", report.errors.len());

    if !report.metadata.is_empty() {
        println!();
        println!("Metadata:");
        let mut metadata: Vec<_> = report.metadata.iter().collect();
        metadata.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        for (key, value) in metadata {
            println!("  {key}: {value}");
        }
    }

    if !report.warnings.is_empty() {
        println!();
        println!("Warnings:");
        for warning in &report.warnings {
            println!("  {warning}");
        }
    }

    if !report.errors.is_empty() {
        println!();
        println!("Errors:");
        for error in &report.errors {
            println!("  {error}");
        }
        return Err(CliError::InvalidInput(format!(
            "decomposed export validation failed: {} error(s)",
            report.errors.len()
        )));
    }

    Ok(())
}

fn find_candidates<'a>(db: &'a Database, search: &str) -> Result<Vec<&'a Record>> {
    let search = search.to_lowercase();
    let entity_si = db
        .struct_id("EntityClassDefinition")
        .ok_or_else(|| CliError::NotFound("EntityClassDefinition struct not found in DCB".into()))?;
    let mut candidates: Vec<_> = db
        .records_of_type(entity_si)
        .filter(|r| {
            db.resolve_string2(r.name_offset)
                .to_lowercase()
                .contains(&search)
        })
        .collect();
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
    Ok(candidates)
}

fn export(
    name: String,
    output: Option<PathBuf>,
    p4k_path: Option<PathBuf>,
    dump_hierarchy: bool,
    opts: ExportOpts,
) -> Result<()> {
    // Decomposed exports generate scene.blend with linked mesh instances.
    if opts.kind.to_lowercase() == "decomposed" {
        return export_blend(name, output, p4k_path, opts);
    }

    crate::log_mem_stats("start");
    let (p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    crate::log_mem_stats("after p4k+dcb load");
    let p4k = p4k.ok_or_else(|| CliError::MissingRequirement("P4k required for entity export".into()))?;
    let db = Database::from_bytes(&dcb_bytes)?;
    crate::log_mem_stats("after db parse");

    let candidates = find_candidates(&db, &name)?;
    if candidates.is_empty() {
        return Err(CliError::NotFound(format!("no EntityClassDefinition records matching '{name}'")));
    }

    let record = candidates[0];
    let rname = db.resolve_string2(record.name_offset);
    let export_name = sanitize_export_name(&export_entity_name(rname));
    if candidates.len() > 1 {
        eprintln!("Found {} candidates, using shortest match: {rname}", candidates.len());
    }

    let idx = EntityIndex::new(&db);
    let export_opts = starbreaker_3d::ExportOptions::from(&opts);
    let output = output.unwrap_or_else(|| {
        match export_opts.kind {
            starbreaker_3d::ExportKind::Bundled => {
                PathBuf::from(format!("{export_name}.{}", bundled_extension(export_opts.format)))
            }
            starbreaker_3d::ExportKind::Decomposed => PathBuf::from(name.clone()),
        }
    });
    let existing_asset_paths = if export_opts.kind == starbreaker_3d::ExportKind::Decomposed
        && opts.skip_existing_assets
    {
        Some(collect_existing_decomposed_assets(&output, &p4k)?)
    } else {
        None
    };
    let existing_interior_assets = if export_opts.kind == starbreaker_3d::ExportKind::Decomposed
        && opts.skip_existing_assets
    {
        Some(collect_existing_interior_assets(&output)?)
    } else {
        None
    };

    crate::log_mem_stats("before loadout resolve");
    let tree = resolve_loadout_indexed(&idx, record);
    crate::log_mem_stats("after loadout resolve");

    eprintln!("\nLoadout tree for {}:", tree.root.entity_name);
    for child in &tree.root.children {
        let g = if child.geometry_path.is_some() { "G" } else { "." };
        eprintln!("  {g} {} -> {}", child.item_port_name, child.entity_name);
    }

    if dump_hierarchy {
        let json = starbreaker_3d::dump_hierarchy(&db, &p4k, record, &tree);
        let json_path = output.with_extension("json");
        std::fs::write(&json_path, &json)
            .map_err(|e| CliError::IoPath { source: e, path: json_path.display().to_string() })?;
        eprintln!("Hierarchy written to {}", json_path.display());
        return Ok(());
    }

    crate::log_mem_stats("before export");
    let existing_asset_loader =
        |relative_path: &str| existing_asset_paths.as_ref().and_then(|paths| {
            read_reusable_decomposed_asset(&output, paths, relative_path)
        });
    let result = starbreaker_3d::assemble_glb_with_loadout_with_progress(
        &db,
        &p4k,
        record,
        &tree,
        &export_opts,
        None,
        existing_asset_paths.as_ref(),
        existing_interior_assets.as_ref(),
        Some(&existing_asset_loader),
    )?;
    crate::log_mem_stats("after export");
    eprintln!("Geometry: {}", result.geometry_path);
    eprintln!("Material: {}", result.material_path);
    match result.kind {
        starbreaker_3d::ExportKind::Bundled => {
            let bundled_bytes = result.bundled_bytes().ok_or_else(|| {
                CliError::InvalidInput(format!(
                    "entity export returned non-bundled output for {:?}",
                    result.kind,
                ))
            })?;
            eprintln!("Bundled export size: {} bytes", bundled_bytes.len());
            std::fs::write(&output, bundled_bytes)
                .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
        }
        starbreaker_3d::ExportKind::Decomposed => {
            let decomposed = result.decomposed.as_ref().ok_or_else(|| {
                CliError::InvalidInput("entity export returned no decomposed files".into())
            })?;
            eprintln!("Decomposed export file count: {}", decomposed.files.len());
            // The decomposed exporter names its package folder with a
            // `_LOD<n>_TEX<n>` suffix. Use that exact name here so we clean
            // the right directory and don't leave an empty sibling folder.
            let package_name = format!(
                "{export_name}_LOD{}_TEX{}",
                export_opts.lod_level, export_opts.texture_mip
            );
            prepare_decomposed_output_root(&output, &package_name)?;
            for file in &decomposed.files {
                if opts.ui_only_files && !should_write_ui_only_decomposed_file(&file.relative_path) {
                    continue;
                }
                let output_path = output.join(&file.relative_path);
                if let Some(parent) = output_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| CliError::IoPath { source: e, path: parent.display().to_string() })?;
                }
                write_decomposed_file(file, &output_path, opts.skip_existing_assets, &p4k)?;
            }
        }
    }
    crate::log_mem_stats("after write");
    eprintln!("Written to {}", output.display());
    Ok(())
}

/// Export the root geometry of an entity directly to `.blend` format.
///
/// Resolves the entity's geometry path from DataCore, loads the `.skinm` from
/// the P4K, parses it via `starbreaker_3d::parse_skin`, and writes the result
/// as a Blender 5.x `.blend` library file containing a single OB_MESH object.
///
/// Phase 68 will extend this to handle the full node hierarchy (children,
/// lights, empties) and per-face material assignments.
fn export_blend(
    name: String,
    output: Option<PathBuf>,
    p4k_path: Option<PathBuf>,
    opts: ExportOpts,
) -> Result<()> {
    let (p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    let p4k = p4k.ok_or_else(|| CliError::MissingRequirement("P4k required for entity blend export".into()))?;
    let db = Database::from_bytes(&dcb_bytes)?;

    let candidates = find_candidates(&db, &name)?;
    if candidates.is_empty() {
        return Err(CliError::NotFound(format!("no EntityClassDefinition records matching '{name}'")));
    }
    let record = candidates[0];
    let rname = db.resolve_string2(record.name_offset);
    let export_name = sanitize_export_name(&export_entity_name(rname));
    if candidates.len() > 1 {
        eprintln!("Found {} candidates, using shortest match: {rname}", candidates.len());
    }

    let is_decomposed = opts.kind.to_lowercase() == "decomposed";

    if is_decomposed {
        // Blend decomposed export: keep the Rust-owned decomposed graph from
        // `starbreaker_3d`, then assemble `scene.blend` via a Rust-launched
        // background Blender process from the emitted `scene.json`.
        let idx = EntityIndex::new(&db);
        let tree = resolve_loadout_indexed(&idx, record);
        let mut export_opts = starbreaker_3d::ExportOptions::from(&opts);
        export_opts.kind = starbreaker_3d::ExportKind::Decomposed;

        let output_dir = output.unwrap_or_else(|| PathBuf::from(&name));
        let package_name = format!(
            "{export_name}_LOD{}_TEX{}",
            export_opts.lod_level, export_opts.texture_mip
        );

        let existing_asset_paths = if opts.skip_existing_assets {
            Some(collect_existing_decomposed_assets(&output_dir, &p4k)?)
        } else {
            None
        };
        let existing_interior_assets = if opts.skip_existing_assets {
            Some(collect_existing_interior_assets(&output_dir)?)
        } else {
            None
        };
        prepare_decomposed_output_root(&output_dir, &package_name)?;

        let existing_asset_loader =
            |relative_path: &str| existing_asset_paths.as_ref().and_then(|paths| {
                read_reusable_decomposed_asset(&output_dir, paths, relative_path)
            });
        let result = starbreaker_3d::assemble_glb_with_loadout_with_progress(
            &db,
            &p4k,
            record,
            &tree,
            &export_opts,
            None,
            existing_asset_paths.as_ref(),
            existing_interior_assets.as_ref(),
            Some(&existing_asset_loader),
        )?;

        let decomposed = result
            .decomposed
            .as_ref()
            .ok_or_else(|| CliError::InvalidInput("entity export returned no decomposed files".into()))?;

        for file in &decomposed.files {
            if opts.ui_only_files && !should_write_ui_only_decomposed_file(&file.relative_path) {
                continue;
            }
            let output_path = output_dir.join(&file.relative_path);
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| CliError::IoPath {
                    source: e,
                    path: parent.display().to_string(),
                })?;
            }
            write_decomposed_file(file, &output_path, opts.skip_existing_assets, &p4k)?;
        }

        let scene_json_path = output_dir.join("Packages").join(&package_name).join("scene.json");
        if !scene_json_path.is_file() {
            return Err(CliError::NotFound(format!(
                "decomposed scene manifest missing after export: {}",
                scene_json_path.display()
            )));
        }

        eprintln!("Written to {}", output_dir.display());
        return Ok(());
    }

    // Resolve geometry path from DataCore
    let geom_compiled = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
        )
        .map_err(|_| CliError::NotFound(format!("entity '{name}' has no geometry component")))?;
    let geometry_path = db
        .query_single::<String>(&geom_compiled, record)
        .map_err(|e| CliError::InvalidInput(format!("DataCore query failed: {e}")))?
        .ok_or_else(|| CliError::NotFound(format!("entity '{name}' has no geometry path")))?;

    // Derive the P4K path for the mesh data file (.skinm / .cgfm)
    // DataCore paths look like "objects/ships/aegs/gladius.skin"; strip Data/ prefix
    // and convert slashes, then append 'm' for the mashed geometry streams file.
    let clean_path = geometry_path
        .strip_prefix("Data/").or_else(|| geometry_path.strip_prefix("Data\\"))
        .or_else(|| geometry_path.strip_prefix("data/")).or_else(|| geometry_path.strip_prefix("data\\"))
        .unwrap_or(&geometry_path);
    let p4k_base = format!("Data\\{}", clean_path.replace('/', "\\"));
    let lod = opts.lod;
    let companion_path = if lod > 0 {
        let lod_path = if let Some(dot) = p4k_base.rfind('.') {
            format!("{}_lod{}{}", &p4k_base[..dot], lod, &p4k_base[dot..])
        } else {
            format!("{}_lod{}", p4k_base, lod)
        };
        let lod_companion = format!("{lod_path}m");
        if p4k.entry_case_insensitive(&lod_companion).is_some() {
            lod_companion
        } else {
            format!("{p4k_base}m")
        }
    } else {
        format!("{p4k_base}m")
    };

    let entry = p4k
        .entry_case_insensitive(&companion_path)
        .ok_or_else(|| CliError::NotFound(format!("mesh data not found in P4K: {companion_path}")))?;

    eprintln!("Loading mesh from: {}", entry.name);
    let mesh_bytes = p4k.read(entry)
        .map_err(|e| CliError::InvalidInput(format!("Failed to read mesh data: {e}")))?;

    let mesh = starbreaker_3d::parse_skin(&mesh_bytes)
        .map_err(|e| CliError::InvalidInput(format!("Failed to parse mesh: {e}")))?;

    eprintln!(
        "Mesh: {} verts, {} triangles, {} submeshes, uvs={}, colors={}",
        mesh.positions.len(),
        mesh.indices.len() / 3,
        mesh.submeshes.len(),
        mesh.uvs.is_some(),
        mesh.colors.is_some(),
    );

    // Bundled blend export (single monolithic mesh).
    let blend_bytes = crate::blend::mesh_to_blend(&export_name, &mesh);
    let output = output.unwrap_or_else(|| PathBuf::from(format!("{export_name}.blend")));
    std::fs::write(&output, &blend_bytes)
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written {} bytes to {}", blend_bytes.len(), output.display());
    Ok(())
}

#[allow(dead_code)]
fn assemble_master_blend_from_scene_json(
    export_root: &Path,
    scene_json_path: &Path,
    output_blend_path: &Path,
    package_name: &str,
) -> Result<()> {
    const SCRIPT: &str = r#"
import json
import math
import sys
from pathlib import Path

import bpy
import mathutils


def sc_matrix_to_blender(sc_matrix):
    if sc_matrix is None:
        return mathutils.Matrix.Identity(4)
    m = mathutils.Matrix(sc_matrix).transposed()
    conv = mathutils.Matrix(((1.0, 0.0, 0.0, 0.0), (0.0, 0.0, -1.0, 0.0), (0.0, 1.0, 0.0, 0.0), (0.0, 0.0, 0.0, 1.0)))
    return conv @ m @ conv.inverted()


def canonical_name(name):
    if len(name) > 4 and name[-4] == "." and name[-3:].isdigit():
        return name[:-4]
    return name


def clear_scene():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for datablocks in (bpy.data.meshes, bpy.data.materials, bpy.data.cameras, bpy.data.lights):
        for block in list(datablocks):
            if block.users == 0:
                datablocks.remove(block)


def ensure_collection(name):
    collection = bpy.data.collections.get(name)
    if collection is None:
        collection = bpy.data.collections.new(name)
        bpy.context.scene.collection.children.link(collection)
    return collection


def cleanup_orphans():
    for datablocks in (
        bpy.data.meshes,
        bpy.data.materials,
        bpy.data.images,
        bpy.data.cameras,
        bpy.data.lights,
    ):
        for block in list(datablocks):
            if block.users == 0:
                datablocks.remove(block)


def unique_library_name(prefix, raw_name, used_names):
    base = f"{prefix}__{canonical_name(raw_name)}"
    candidate = base
    suffix = 1
    while candidate in used_names:
        suffix += 1
        candidate = f"{base}_{suffix}"
    used_names.add(candidate)
    return candidate


def build_material_vertex_groups(obj):
    if obj.type != "MESH" or obj.data is None:
        return
    if len(obj.vertex_groups) > 0:
        for group in list(obj.vertex_groups):
            obj.vertex_groups.remove(group)

    vertices_by_group = {}
    for poly in obj.data.polygons:
        slot_index = poly.material_index
        if slot_index < 0 or slot_index >= len(obj.material_slots):
            continue
        slot = obj.material_slots[slot_index]
        material_name = ""
        if slot.material is not None:
            material_name = canonical_name(slot.material.name)
        if not material_name:
            material_name = canonical_name(slot.name) or f"material_{slot_index}"
        bucket = vertices_by_group.setdefault(material_name, set())
        for vertex_index in poly.vertices:
            bucket.add(int(vertex_index))

    for group_name, vertex_indices in vertices_by_group.items():
        if not vertex_indices:
            continue
        group = obj.vertex_groups.new(name=group_name)
        group.add(sorted(vertex_indices), 1.0, "REPLACE")


def import_gltf_template(full_path):
    before = {obj.as_pointer() for obj in bpy.data.objects}
    result = bpy.ops.import_scene.gltf(
        filepath=str(full_path),
        import_pack_images=False,
        merge_vertices=False,
        import_select_created_objects=False,
    )
    if "FINISHED" not in result:
        return []
    return [obj for obj in bpy.data.objects if obj.as_pointer() not in before]


def write_mesh_libraries(full_path, imported):
    full_path.parent.mkdir(parents=True, exist_ok=True)

    used_object_names = set()
    used_mesh_names = set()
    prefix = full_path.stem
    mesh_libraries = {}
    for obj in imported:
        obj.name = unique_library_name(prefix, obj.name, used_object_names)
        if obj.type != "MESH" or obj.data is None:
            continue
        mesh = obj.data
        if mesh.as_pointer() not in mesh_libraries:
            mesh.name = unique_library_name(prefix, mesh.name, used_mesh_names)
            mesh_library_path = full_path.with_name(f"{mesh.name}.blend")
            bpy.data.libraries.write(str(mesh_library_path), {mesh}, path_remap="RELATIVE_ALL")
            mesh_libraries[mesh.as_pointer()] = {
                "mesh_name": mesh.name,
                "mesh_library_path": str(mesh_library_path),
            }
    return mesh_libraries


def template_sidecar_path(full_path):
    return full_path.with_suffix(".blend.template.json")


def build_and_write_asset_template(full_path):
    imported = import_gltf_template(full_path)
    if not imported:
        return None

    imported_ptrs = {obj.as_pointer() for obj in imported}
    source_names = {obj.as_pointer(): canonical_name(obj.name) for obj in imported}
    mesh_libraries = write_mesh_libraries(full_path, imported)

    nodes = []
    root_names = []
    for obj in imported:
        source_name = source_names[obj.as_pointer()]
        parent_name = None
        if obj.parent is not None and obj.parent.as_pointer() in imported_ptrs:
            parent_name = source_names[obj.parent.as_pointer()]
        if parent_name is None:
            root_names.append(source_name)
        mesh_info = mesh_libraries.get(obj.data.as_pointer()) if obj.type == "MESH" and obj.data is not None else None
        nodes.append(
            {
                "source_name": source_name,
                "parent_source_name": parent_name,
                "matrix_local": [list(row) for row in obj.matrix_basis.copy()],
                "mesh_name": mesh_info["mesh_name"] if mesh_info is not None else None,
                "mesh_library_path": mesh_info["mesh_library_path"] if mesh_info is not None else None,
                "object_type": obj.type,
            }
        )

    template = {
        "nodes": nodes,
        "root_names": root_names,
    }
    sidecar_path = template_sidecar_path(full_path)
    with sidecar_path.open("w", encoding="utf-8") as handle:
        json.dump(template, handle)

    for obj in reversed(imported):
        bpy.data.objects.remove(obj, do_unlink=True)
    cleanup_orphans()
    return template


def load_asset_template(full_path, asset_cache):
    cached = asset_cache.get(str(full_path))
    if cached is not None:
        return cached

    sidecar_path = template_sidecar_path(full_path)
    if not sidecar_path.is_file():
        return None
    with sidecar_path.open("r", encoding="utf-8") as handle:
        template = json.load(handle)
    asset_cache[str(full_path)] = template
    return template


def ensure_linked_mesh(mesh_library_path, mesh_name, linked_mesh_cache):
    cache_key = (mesh_library_path, mesh_name)
    cached = linked_mesh_cache.get(cache_key)
    if cached is not None:
        return cached

    with bpy.data.libraries.load(mesh_library_path, link=True) as (data_from, data_to):
        data_to.meshes = [mesh_name] if mesh_name in data_from.meshes else []

    mesh = data_to.meshes[0] if data_to.meshes else None
    if mesh is not None:
        linked_mesh_cache[cache_key] = mesh
    return mesh


def instantiate_template(template, collection, instance_name, linked_mesh_cache):
    instances = {}
    created = []

    for node in template["nodes"]:
        object_name = node["source_name"]
        if node["parent_source_name"] is None:
            object_name = instance_name
        linked_data = None
        if node.get("mesh_name") and node.get("mesh_library_path"):
            linked_data = ensure_linked_mesh(node["mesh_library_path"], node["mesh_name"], linked_mesh_cache)
        obj = bpy.data.objects.new(object_name, linked_data)
        collection.objects.link(obj)
        instances[node["source_name"]] = obj
        created.append((obj, node))

    for obj, node in created:
        parent_name = node["parent_source_name"]
        obj.parent = None
        if parent_name is not None:
            obj.parent = instances[parent_name]
            obj.matrix_parent_inverse = mathutils.Matrix.Identity(4)
        obj.matrix_basis = mathutils.Matrix(node["matrix_local"])
        build_material_vertex_groups(obj)

    roots = [instances[name] for name in template["root_names"] if name in instances]
    if len(roots) == 1:
        anchor = roots[0]
    else:
        anchor = bpy.data.objects.new(instance_name, None)
        collection.objects.link(anchor)
        for root in roots:
            world = root.matrix_world.copy()
            root.parent = anchor
            root.matrix_world = world

    node_index = {}
    for source_name, obj in instances.items():
        node_index[source_name] = obj
        node_index[source_name.strip().lower()] = obj
    return anchor, node_index


def import_mesh_asset(export_root, rel_path, collection, asset_cache, linked_mesh_cache, instance_name):
    if not rel_path:
        return None, {}
    full_path = (export_root / rel_path).resolve()
    if not full_path.is_file():
        return None, {}

    template = load_asset_template(full_path, asset_cache)
    if template is None:
        return None, {}
    return instantiate_template(template, collection, instance_name, linked_mesh_cache)


def iter_scene_mesh_assets(scene):
    seen = set()

    def add_asset(rel_path):
        if rel_path and rel_path not in seen:
            seen.add(rel_path)
            return rel_path
        return None

    root_asset = add_asset((scene.get("root_entity") or {}).get("mesh_asset"))
    if root_asset:
        yield root_asset
    for child in scene.get("children", []):
        rel_path = add_asset(child.get("mesh_asset"))
        if rel_path:
            yield rel_path
    for interior in scene.get("interiors", []):
        for placement in interior.get("placements", []):
            rel_path = add_asset(placement.get("mesh_asset"))
            if rel_path:
                yield rel_path


def build_libraries(scene, export_root):
    clear_scene()
    for rel_path in iter_scene_mesh_assets(scene):
        full_path = (export_root / rel_path).resolve()
        if full_path.is_file():
            build_and_write_asset_template(full_path)
    clear_scene()


def apply_local_transform(anchor, parent, local_transform_sc):
    local = sc_matrix_to_blender(local_transform_sc)
    if parent is not None:
        anchor.parent = parent
        anchor.matrix_parent_inverse = mathutils.Matrix.Identity(4)
        anchor.matrix_basis = local
    else:
        anchor.matrix_world = local


def find_parent_node(index_by_entity, parent_entity_name, parent_node_name):
    if not parent_entity_name or not parent_node_name:
        return None
    candidates = index_by_entity.get(parent_entity_name, [])
    for node_index in reversed(candidates):
        node = node_index.get(parent_node_name)
        if node is not None:
            return node
        node = node_index.get(parent_node_name.strip().lower())
        if node is not None:
            return node
    return None


def instantiate_record(record, export_root, collection, parent_default, parent_node, index_by_entity):
    entity_name = record.get("entity_name") or "SceneInstance"
    anchor, nodes = import_mesh_asset(
        export_root,
        record.get("mesh_asset"),
        collection,
        ASSET_CACHE,
        LINKED_MESH_CACHE,
        entity_name,
    )
    if anchor is None:
        anchor = bpy.data.objects.new(entity_name, None)
        collection.objects.link(anchor)
        nodes = {}

    anchor.name = entity_name
    apply_local_transform(anchor, parent_node if parent_node is not None else parent_default, record.get("local_transform_sc"))
    index_by_entity.setdefault(entity_name, []).append(nodes)
    return anchor, nodes


def sc_pos_to_blender(pos):
    """Convert a CryEngine world-space position vector to Blender space."""
    x, y, z = pos
    return mathutils.Vector((x, -z, y))


BLENDER_LIGHT_TYPE = {
    "point": "POINT",
    "spot": "SPOT",
    "sun": "SUN",
    "area": "AREA",
}


def add_interior_lights(interior, interior_anchor, package_collection):
    for light_data in interior.get("lights", []):
        name = light_data.get("name", "light")
        semantic_kind = (light_data.get("semantic_light_kind") or "point").lower()
        blender_type = BLENDER_LIGHT_TYPE.get(semantic_kind, "POINT")
        color = light_data.get("color") or [1.0, 1.0, 1.0]
        intensity = float(light_data.get("intensity") or 1.0)
        radius_m = float(light_data.get("radius_m") or 1.0)
        position = light_data.get("position") or [0.0, 0.0, 0.0]
        outer_angle = light_data.get("outer_angle")
        inner_angle = light_data.get("inner_angle")

        light = bpy.data.lights.new(name=name, type=blender_type)
        light.color = color[:3]
        light.energy = intensity
        if blender_type == "POINT":
            light.shadow_soft_size = radius_m
        elif blender_type == "SPOT":
            if outer_angle is not None:
                light.spot_size = math.radians(float(outer_angle))
            if inner_angle is not None and outer_angle and outer_angle > 0:
                light.spot_blend = 1.0 - (float(inner_angle) / float(outer_angle))

        obj = bpy.data.objects.new(name, light)
        package_collection.objects.link(obj)
        # Position lights in world space (SC positions are absolute, not relative to container).
        # Leave unparented so the -90 X correction on package_root does not double-rotate them.
        obj.location = sc_pos_to_blender(position)


def assemble_scene(scene, export_root, output_blend, package_name):
    clear_scene()
    package_collection = ensure_collection(f"StarBreaker {package_name}")
    package_root = bpy.data.objects.new(f"StarBreaker {package_name}", None)
    package_collection.objects.link(package_root)
    # CryEngine is Z-up; correct to Blender Y-up by rotating the root empty -90° on X.
    package_root.rotation_euler = (math.radians(-90), 0.0, 0.0)

    index_by_entity = {}
    global ASSET_CACHE, LINKED_MESH_CACHE
    ASSET_CACHE = {}
    LINKED_MESH_CACHE = {}

    root_record = scene.get("root_entity") or {}
    root_anchor, root_nodes = instantiate_record(
        root_record,
        export_root,
        package_collection,
        package_root,
        None,
        index_by_entity,
    )

    for child in scene.get("children", []):
        parent_node = find_parent_node(
            index_by_entity,
            child.get("parent_entity_name"),
            child.get("parent_node_name"),
        )
        instantiate_record(
            child,
            export_root,
            package_collection,
            root_anchor,
            parent_node,
            index_by_entity,
        )

    for interior in scene.get("interiors", []):
        interior_anchor = bpy.data.objects.new("InteriorContainer", None)
        package_collection.objects.link(interior_anchor)
        apply_local_transform(interior_anchor, root_anchor, interior.get("container_transform"))
        add_interior_lights(interior, interior_anchor, package_collection)
        for placement in interior.get("placements", []):
            mesh_asset = placement.get("mesh_asset")
            instance_name = Path(mesh_asset).stem if mesh_asset else "InteriorInstance"
            anchor, _nodes = import_mesh_asset(
                export_root,
                mesh_asset,
                package_collection,
                ASSET_CACHE,
                LINKED_MESH_CACHE,
                instance_name,
            )
            if anchor is None:
                continue
            anchor.parent = interior_anchor
            placement_transform = placement.get("transform")
            if placement_transform is not None:
                anchor.matrix_parent_inverse = mathutils.Matrix.Identity(4)
                anchor.matrix_basis = sc_matrix_to_blender(placement_transform)

    bpy.ops.wm.save_as_mainfile(filepath=str(output_blend), copy=True)


def main():
    argv = sys.argv
    if "--" not in argv:
        raise RuntimeError("missing -- separator")
    args = argv[argv.index("--") + 1 :]
    if len(args) != 5:
        raise RuntimeError(f"expected 5 args, got {len(args)}")

    mode = args[0]
    scene_json_path = Path(args[1])
    export_root = Path(args[2])
    output_blend = Path(args[3])
    package_name = args[4]

    with scene_json_path.open("r", encoding="utf-8") as handle:
        scene = json.load(handle)

    if mode == "libraries":
        build_libraries(scene, export_root)
        return
    if mode == "scene":
        assemble_scene(scene, export_root, output_blend, package_name)
        return
    raise RuntimeError(f"unknown mode: {mode}")


if __name__ == "__main__":
    main()
"#;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| CliError::InvalidInput(format!("system time error: {e}")))?
        .as_millis();
    let script_path = std::env::temp_dir().join(format!("starbreaker_blend_assembly_{ts}.py"));
    std::fs::write(&script_path, SCRIPT)
        .map_err(|e| CliError::IoPath { source: e, path: script_path.display().to_string() })?;

    if let Some(parent) = output_blend_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CliError::IoPath { source: e, path: parent.display().to_string() })?;
    }

    for mode in ["libraries", "scene"] {
        let output = Command::new("blender")
            .arg("--background")
            .arg("--factory-startup")
            .arg("--python")
            .arg(&script_path)
            .arg("--")
            .arg(mode)
            .arg(scene_json_path)
            .arg(export_root)
            .arg(output_blend_path)
            .arg(package_name)
            .output()
            .map_err(|e| CliError::InvalidInput(format!("failed to run blender for scene assembly ({mode}): {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let message = format!(
                "blender scene assembly failed in {mode} mode (status {:?})\nstdout:\n{}\nstderr:\n{}",
                output.status.code(),
                stdout,
                stderr
            );
            let _ = std::fs::remove_file(&script_path);
            return Err(CliError::InvalidInput(message));
        }
    }

    let _ = std::fs::remove_file(&script_path);

    Ok(())
}

fn loadout(name: String, p4k_path: Option<PathBuf>) -> Result<()> {
    let (_, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    let db = Database::from_bytes(&dcb_bytes)?;

    let candidates = find_candidates(&db, &name)?;
    if candidates.is_empty() {
        return Err(CliError::NotFound(format!("no EntityClassDefinition records matching '{name}'")));
    }

    let idx = EntityIndex::new(&db);
    for record in &candidates {
        let tree = resolve_loadout_indexed(&idx, record);
        print_loadout_node(&tree.root, 0);
    }
    Ok(())
}

fn print_loadout_node(node: &LoadoutNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let geom = node.geometry_path.as_deref().unwrap_or("-");
    println!(
        "{indent}{} [{}] geom={geom}",
        node.entity_name, node.item_port_name
    );
    for child in &node.children {
        print_loadout_node(child, depth + 1);
    }
}
