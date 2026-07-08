//! Shared live-IR harness for the frozen-target regression guards.
//!
//! Extracted verbatim from `manifest_live_ir_guard.rs` so that every guard which
//! needs "the current IR each frozen target compiles to" uses the *same*
//! frozen-target enumeration, the *same* live-IR input source, and the *same*
//! skip-when-no-game-data behaviour (reuse, do not duplicate — see
//! `ir-freeze-schema.md` and the swf_helpers precedent). Two guards include it:
//! `manifest_live_ir_guard` (typography/tint/placeholder semantics) and
//! `element_presence_guard` (sub-threshold element loss).
//!
//! Key items:
//! - [`load_live_target_cases`] — enumerates the frozen targets from
//!   `fixtures/ui_ir/ui_snapshot_freeze.json`, compiles each target's current IR
//!   from the canvas records export dir, and returns `None` (skip) when the
//!   records root is absent. Carries the T5 non-vacuous-target precondition.
//! - [`LiveTargetCase`] — one frozen target's id, freshly compiled current IR,
//!   and frozen baseline snapshot.
//! - [`FsCanvasFetcher`] / [`load_canvas_index`] — the canvas-record loader.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError, UiIrDocument,
    UiScreenSnapshot, compile_ir_for_binding,
};

#[derive(Debug, Deserialize)]
pub struct SnapshotFreezeFile {
    pub targets: Vec<SnapshotFreezeTarget>,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotFreezeTarget {
    pub id: String,
    pub source_generated_png: String,
    pub canvas_guid: String,
    pub baseline_snapshot: UiScreenSnapshot,
}

pub struct LiveTargetCase {
    pub id: String,
    pub current_ir: UiIrDocument,
    pub baseline_snapshot: UiScreenSnapshot,
}

pub struct FsCanvasFetcher {
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
        Ok(starbreaker_ui::StyleLoader::for_manufacturer(&self.manufacturer_id).neutral_fallback())
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
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

pub fn load_canvas_index(root: &Path) -> Result<FsCanvasFetcher, String> {
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
        let path_rel = path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(|relative| relative.replace('\\', "/"));

        guid_to_path.insert(record_id.clone(), path.clone());
        by_name.insert(record_name.clone(), record_id.clone());
        by_name.insert(record_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(bare_name.clone(), record_id.clone());
        by_name.insert(bare_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(record_id.clone(), record_id.clone());
        if !path_stem.is_empty() {
            by_name.insert(path_stem.clone(), record_id.clone());
            by_name.insert(path_stem.to_ascii_lowercase(), record_id.clone());
        }
        if let Some(rel) = path_rel {
            by_name.insert(rel.clone(), record_id.clone());
            by_name.insert(rel.to_ascii_lowercase(), record_id.clone());
        }
    }

    Ok(FsCanvasFetcher { guid_to_path, by_name })
}

fn compile_target_ir(
    fetcher: &FsCanvasFetcher,
    localization_map: Option<HashMap<String, String>>,
    manufacturer_id: &str,
    canvas_guid: &str,
    target_size: (u32, u32),
) -> UiIrDocument {
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
        localization_map,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    compile_ir_for_binding(&inputs).expect("target IR compile should succeed")
}

fn snapshot_freeze() -> SnapshotFreezeFile {
    serde_json::from_str(include_str!("../fixtures/ui_ir/ui_snapshot_freeze.json"))
        .expect("snapshot freeze fixture should parse")
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

/// Enumerate the frozen targets and compile each one's current IR.
///
/// Returns `None` (skip) when the canvas records root is absent — identical
/// skip behaviour to the live-IR guards it backs. When it returns `Some`, the
/// case list is guaranteed non-empty (the T5 non-vacuous precondition below).
pub fn load_live_target_cases() -> Option<Vec<LiveTargetCase>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root should resolve from CARGO_MANIFEST_DIR");
    let canvas_root = workspace_root.join("ships/dcb_canvas/libs/foundry/records");
    if !canvas_root.is_dir() {
        eprintln!(
            "skipping live manifest IR guard (missing records root: {})",
            canvas_root.display()
        );
        return None;
    }
    let fetcher = load_canvas_index(&canvas_root).expect("canvas index should load");

    let localization_path = workspace_root.join("target/Data/Localization/english/global.ini");
    let localization_map = fs::read(&localization_path)
        .ok()
        .map(|bytes| starbreaker_ui::bb_loc_p4k::parse_ini_bytes(&bytes));

    let cases: Vec<LiveTargetCase> = snapshot_freeze()
        .targets
        .into_iter()
        .map(|target| {
            let manufacturer_id = manufacturer_from_source_path(&target.source_generated_png);
            let target_size = (
                target.baseline_snapshot.target_width,
                target.baseline_snapshot.target_height,
            );
            let current_ir = compile_target_ir(
                &fetcher,
                localization_map.clone(),
                &manufacturer_id,
                &target.canvas_guid,
                target_size,
            );

            LiveTargetCase {
                id: target.id,
                current_ir,
                baseline_snapshot: target.baseline_snapshot,
            }
        })
        .collect();

    // Non-empty-target precondition (alignment plan T5, ledger 3/61/105): past
    // the skip-if-missing gate above, an emptied snapshot-freeze fixture would
    // leave every guard iterating zero cases and passing vacuously. One assert
    // here covers all callers.
    assert!(
        !cases.is_empty(),
        "live manifest guard built zero target cases — vacuous (snapshot freeze fixture emptied?)"
    );

    Some(cases)
}
