//! Generate IR-only UI snapshot freeze data from the regression manifest.
//!
//! This example resolves each manifest target to a local BuildingBlocks canvas
//! record, compiles canonical UI IR from local decomposed records, converts it
//! to a `UiScreenSnapshot`, and writes an IR-only freeze file suitable for git.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
    UiScreenSnapshot, compile_ir_for_binding, snapshot_from_ui_ir,
};

#[derive(Debug, Deserialize)]
struct ManifestFile {
    schema_version: u32,
    targets: Vec<ManifestTarget>,
}

#[derive(Debug, Deserialize)]
struct ManifestTarget {
    id: String,
    category: String,
    tier: String,
    source_generated_png: String,
}

#[derive(Debug, Serialize)]
struct SnapshotFreezeFile {
    schema_version: u32,
    frozen_at: String,
    approver: String,
    reason: String,
    signature: Option<String>,
    manifest_path: String,
    /// Self-audit: per-identity changes vs the previous freeze
    /// (crates/starbreaker-ui/docs/ui-workflow.md §7). Empty on first freeze of a new file.
    delta: Vec<starbreaker_ui::freeze_audit::FreezeDelta>,
    targets: Vec<SnapshotFreezeTarget>,
}

#[derive(Debug, Serialize)]
struct SnapshotFreezeTarget {
    id: String,
    tier: String,
    category: String,
    source_generated_png: String,
    canvas_record_path: String,
    canvas_guid: String,
    baseline_snapshot: UiScreenSnapshot,
}

struct FsCanvasFetcher {
    guid_to_path: HashMap<String, PathBuf>,
    by_name: HashMap<String, String>,
}

impl CanvasFetcher for FsCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        let path = self
            .guid_to_path
            .get(guid)
            .ok_or_else(|| UiError::RenderError(format!("missing canvas guid: {guid}")))?;
        load_canvas_json_from_path(path)
            .map_err(|err| UiError::RenderError(format!("failed loading canvas {guid}: {err}")))
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        let guid = self
            .by_name
            .get(record_name)
            .or_else(|| self.by_name.get(&record_name.to_ascii_lowercase()))
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))?;
        self.fetch_canvas_json(guid)
    }
}

struct DummySwfFetcher;

impl SwfFetcher for DummySwfFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("SWF fetch not required".to_string()))
    }
}

struct DummyStyleFetcher {
    manufacturer_id: String,
}

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Ok(starbreaker_ui::StyleLoader::for_manufacturer(&self.manufacturer_id)
            .neutral_fallback())
    }
}

struct DummyAssetFetcher;

impl AssetFetcher for DummyAssetFetcher {
    fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
        None
    }
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

fn load_canvas_json_from_path(path: &Path) -> Result<serde_json::Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn load_canvas_index(root: &Path) -> Result<FsCanvasFetcher, String> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files);

    let mut guid_to_path = HashMap::new();
    let mut by_name = HashMap::new();

    for path in files {
        let json = load_canvas_json_from_path(&path)?;
        let Some(record_name) = json
            .get("_RecordName_")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(record_id) = json
            .get("_RecordId_")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
        else {
            continue;
        };

        let bare_name = record_name
            .strip_prefix("BuildingBlocks_Canvas.")
            .unwrap_or(&record_name)
            .to_string();
        let path_stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_string();

        guid_to_path.insert(record_id.clone(), path.clone());
        by_name.insert(record_name.clone(), record_id.clone());
        by_name.insert(record_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(bare_name.clone(), record_id.clone());
        by_name.insert(bare_name.to_ascii_lowercase(), record_id.clone());
        if !path_stem.is_empty() {
            by_name.insert(path_stem.clone(), record_id.clone());
            by_name.insert(path_stem.to_ascii_lowercase(), record_id.clone());
        }
    }

    Ok(FsCanvasFetcher { guid_to_path, by_name })
}

fn load_localization_map(workspace_root: &Path) -> Option<HashMap<String, String>> {
    let ini_path = workspace_root.join("target/Data/Localization/english/global.ini");
    let bytes = fs::read(&ini_path).ok()?;
    let map = starbreaker_ui::bb_loc_p4k::parse_ini_bytes(&bytes);
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

fn manufacturer_from_source_path(source_generated_png: &str) -> String {
    let parts: Vec<&str> = source_generated_png.split('/').collect();
    if let Some(index) = parts.iter().position(|part| *part == "ship") {
        if let Some(manufacturer) = parts.get(index + 1) {
            return (*manufacturer).to_string();
        }
    }
    "drak".to_string()
}

fn canvas_basename_from_source_path(source_generated_png: &str) -> Result<String, String> {
    let filename = Path::new(source_generated_png)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("source_generated_png missing filename: {source_generated_png}"))?;
    let basename = filename
        .strip_prefix("buildingblocks_canvas_")
        .unwrap_or(filename)
        .to_string();
    Ok(basename)
}

fn resolve_canvas_record(
    ui_records_root: &Path,
    basename: &str,
) -> Result<(String, String, (u32, u32)), String> {
    let mut matches = Vec::new();
    collect_json_files(ui_records_root, &mut matches);
    let mut record_matches: Vec<PathBuf> = matches
        .into_iter()
        .filter(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(basename))
        .collect();
    record_matches.sort();

    match record_matches.as_slice() {
        [] => Err(format!("no UI canvas record found for basename {basename}")),
        [path] => {
            let json = load_canvas_json_from_path(path)?;
            let guid = json
                .get("_RecordId_")
                .and_then(|value| value.as_str())
                .ok_or_else(|| format!("record missing _RecordId_: {}", path.display()))?
                .to_string();
            let size = json
                .get("_RecordValue_")
                .and_then(|value| value.get("size"))
                .ok_or_else(|| format!("record missing _RecordValue_.size: {}", path.display()))?;
            let width = size
                .get("x")
                .and_then(|value| value.as_f64())
                .ok_or_else(|| format!("record missing size.x: {}", path.display()))?;
            let height = size
                .get("y")
                .and_then(|value| value.as_f64())
                .ok_or_else(|| format!("record missing size.y: {}", path.display()))?;
            if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
                return Err(format!(
                    "record has invalid size {}x{}: {}",
                    width,
                    height,
                    path.display()
                ));
            }
            Ok((
                guid,
                path.display().to_string(),
                (width.round() as u32, height.round() as u32),
            ))
        }
        _ => Err(format!(
            "multiple UI canvas records found for basename {}: {}",
            basename,
            record_matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn compile_snapshot(
    fetcher: &FsCanvasFetcher,
    localization_map: Option<&HashMap<String, String>>,
    manufacturer_id: &str,
    canvas_guid: &str,
    target_size: (u32, u32),
) -> Result<UiScreenSnapshot, String> {
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher {
        manufacturer_id: manufacturer_id.to_string(),
    };
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some(canvas_guid),
        content_canvas_guid: Some(canvas_guid),
        binding_kind: Some("mfd"),
        manufacturer_id: Some(manufacturer_id),
        helper_name: Some("freeze-ui-snapshot-ir"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: fetcher,
        swf_fetcher: &swf_fetcher,
        style_fetcher: &style_fetcher,
        asset_fetcher: &asset_fetcher,
        target_size,
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: localization_map.cloned(),
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs)
        .map_err(|err| format!("failed to compile IR for {canvas_guid}: {err}"))?;
    Ok(snapshot_from_ui_ir(&ir))
}

fn main() -> Result<(), String> {
    let mut manifest_path: Option<PathBuf> = None;
    let mut output_path: Option<PathBuf> = None;
    let mut approver: Option<String> = None;
    let mut reason: Option<String> = None;
    let mut signature: Option<String> = None;
    let mut allow_empty = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => manifest_path = args.next().map(PathBuf::from),
            "--output" => output_path = args.next().map(PathBuf::from),
            "--approver" => approver = args.next(),
            "--reason" => reason = args.next(),
            "--signature" => signature = args.next(),
            "--allow-empty" => allow_empty = true,
            _ => return Err(format!("unknown arg: {arg}")),
        }
    }

    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_root.join("../..").canonicalize().map_err(|err| err.to_string())?;
    let workspace_root = repo_root.parent().ok_or_else(|| "repo root missing workspace parent".to_string())?;

    let manifest_path = manifest_path.unwrap_or_else(|| {
        repo_root.join("crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json")
    });
    let output_path = output_path.unwrap_or_else(|| {
        repo_root.join("crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json")
    });
    let approver = approver.ok_or_else(|| "missing --approver".to_string())?;
    let reason = reason.ok_or_else(|| "missing --reason".to_string())?;

    let manifest_raw = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read manifest {}: {err}", manifest_path.display()))?;
    let manifest: ManifestFile = serde_json::from_str(&manifest_raw)
        .map_err(|err| format!("failed to parse manifest {}: {err}", manifest_path.display()))?;
    if manifest.schema_version != 1 {
        return Err(format!("unsupported manifest schema version: {}", manifest.schema_version));
    }

    let records_root = workspace_root.join("ships/dcb_canvas/libs/foundry/records");
    let ui_records_root = records_root.join("ui/buildingblocks");
    let fetcher = load_canvas_index(&records_root)?;
    let localization_map = load_localization_map(workspace_root);

    let mut targets = Vec::with_capacity(manifest.targets.len());
    for target in manifest.targets {
        let basename = canvas_basename_from_source_path(&target.source_generated_png)?;
        let (canvas_guid, canvas_record_path, target_size) =
            resolve_canvas_record(&ui_records_root, &basename)?;
        // Workspace-relative provenance: no machine-specific home prefix in
        // the checked-in fixture (owner preference, 2026-06-13).
        let canvas_record_path = canvas_record_path
            .strip_prefix(&format!("{}/", workspace_root.display()))
            .map(str::to_owned)
            .unwrap_or(canvas_record_path);
        let manufacturer_id = manufacturer_from_source_path(&target.source_generated_png);
        let baseline_snapshot = compile_snapshot(
            &fetcher,
            localization_map.as_ref(),
            &manufacturer_id,
            &canvas_guid,
            target_size,
        )?;

        targets.push(SnapshotFreezeTarget {
            id: target.id,
            tier: target.tier,
            category: target.category,
            source_generated_png: target.source_generated_png,
            canvas_record_path,
            canvas_guid,
            baseline_snapshot,
        });
    }

    let frozen_at = String::from_utf8(
        Command::new("date")
            .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
            .output()
            .map_err(|err| format!("failed to invoke date: {err}"))?
            .stdout,
    )
    .map_err(|err| format!("date output was not utf8: {err}"))?
    .trim()
    .to_string();

    let mut freeze = SnapshotFreezeFile {
        schema_version: 1,
        frozen_at,
        approver,
        reason,
        signature,
        manifest_path: manifest_path.display().to_string(),
        delta: Vec::new(),
        targets,
    };

    // Self-auditing delta report (crates/starbreaker-ui/docs/ui-workflow.md §7): diff against the
    // existing freeze, print every changed identity, embed the delta in the
    // written file, and refuse a no-op re-freeze unless --allow-empty.
    // The comparison goes through the SERIALIZED form so float formatting is
    // identical on both sides (a Value round-trip widens f32 and produces
    // phantom precision deltas).
    if let Ok(existing_raw) = fs::read_to_string(&output_path) {
        let existing: serde_json::Value = serde_json::from_str(&existing_raw)
            .map_err(|err| format!("existing freeze is not valid JSON: {err}"))?;
        let new_serialized = serde_json::to_string(&freeze)
            .map_err(|err| format!("failed to serialize freeze file: {err}"))?;
        let new_value: serde_json::Value = serde_json::from_str(&new_serialized)
            .map_err(|err| format!("freeze round-trip failed: {err}"))?;
        let deltas = starbreaker_ui::freeze_audit::diff_freeze_documents(&existing, &new_value);
        if deltas.is_empty() && !allow_empty {
            return Err(
                "no baseline changes vs the existing freeze — refusing a no-op \
                 re-freeze (pass --allow-empty to update metadata only)"
                    .to_string(),
            );
        }
        println!("freeze delta vs existing baseline ({} change(s)):", deltas.len());
        for d in &deltas {
            println!("  {} {} {}: {} -> {}", d.target, d.identity, d.field, d.old, d.new);
        }
        println!("the --reason and the commit message must account for every line above.");
        freeze.delta = deltas;
    }

    let serialized = serde_json::to_string_pretty(&freeze)
        .map_err(|err| format!("failed to serialize freeze file: {err}"))?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&output_path, serialized)
        .map_err(|err| format!("failed to write {}: {err}", output_path.display()))?;

    println!("wrote {}", output_path.display());
    Ok(())
}
