//! Dump canonical UI IR for the two Clipper target canvases.
//!
//! Loads BuildingBlocks canvas records from the decomposed DataCore export under
//! `ships/dcb_canvas/libs/foundry/records/ui/buildingblocks`, resolves the
//! target canvases by GUID, and writes IR JSON outputs to phase-1 artifacts.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView,
    UiError, compile_ir_for_binding,
};

struct FsCanvasFetcher {
    guid_to_path: HashMap<String, PathBuf>,
    by_name: HashMap<String, String>,
}

impl CanvasFetcher for FsCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        self.load_json_for_guid(guid)
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        let guid = self
            .by_name
            .get(record_name)
            .or_else(|| self.by_name.get(&record_name.to_ascii_lowercase()))
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))?;
        self.load_json_for_guid(guid)
    }
}

impl FsCanvasFetcher {
    fn load_json_for_guid(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        let path = self
            .guid_to_path
            .get(guid)
            .ok_or_else(|| UiError::RenderError(format!("missing canvas guid: {guid}")))?;
        load_canvas_json_from_path(path).map_err(|e| {
            UiError::RenderError(format!(
                "failed to load canvas {} from {}: {e}",
                guid,
                path.display()
            ))
        })
    }
}

struct DummySwfFetcher;

impl SwfFetcher for DummySwfFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("SWF fetch not needed for IR dump".to_string()))
    }
}

struct DummyStyleFetcher;

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Ok(starbreaker_ui::StyleLoader::for_manufacturer("drak").neutral_fallback())
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
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

fn load_canvas_index(root: &Path) -> Result<FsCanvasFetcher, String> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files);

    let mut guid_to_path = HashMap::new();
    let mut by_name = HashMap::new();

    for path in files {
        let json = load_canvas_json_from_path(&path)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;

        let Some(record_name) = json
            .get("_RecordName_")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(record_id) = json
            .get("_RecordId_")
            .and_then(|v| v.as_str())
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
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let path_rel = path
            .strip_prefix(root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"));

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

        by_name.insert(starbreaker_ui::pipeline::extract_record_name(&record_name), record_id.clone());
        by_name.insert(
            starbreaker_ui::pipeline::extract_record_name(&record_name).to_ascii_lowercase(),
            record_id,
        );
    }

    Ok(FsCanvasFetcher {
        guid_to_path,
        by_name,
    })
}

fn load_canvas_json_from_path(path: &Path) -> Result<serde_json::Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("failed to parse {}: {e}", path.display()))
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

fn dump_one(
    fetcher: &FsCanvasFetcher,
    localization_map: Option<&HashMap<String, String>>,
    canvas_guid: &str,
    output_path: &Path,
) -> Result<(), String> {
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some(canvas_guid),
        content_canvas_guid: Some(canvas_guid),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("ui-target-ir-dump"),
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
        target_size: (1920, 1080),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: localization_map.cloned(),
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs)
        .map_err(|e| format!("failed to compile IR for {canvas_guid}: {e}"))?;

    let serialized = serde_json::to_string_pretty(&ir)
        .map_err(|e| format!("failed to serialize IR for {canvas_guid}: {e}"))?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    fs::write(output_path, serialized)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    Ok(())
}

fn main() -> Result<(), String> {
    let workspace = PathBuf::from(format!("{}/projects/scorg_tools", std::env::var("HOME").unwrap_or_default()));
    let canvas_root = workspace.join("ships/dcb_canvas/libs/foundry/records");
    let localization_map = load_localization_map(&workspace);

    let fetcher = load_canvas_index(&canvas_root)?;

    // Target canvases from docs/ui-plan2.md mapping.
    let ui_target_a_guid = "534bab84-299b-479a-a4af-4469df112ea7"; // I_Med_MedicalBed_A
    let ui_target_b_guid = "e9ad809d-ebcf-43a3-bb20-120f64556aef"; // I_Med_MedicalEndOfBed_A

    let out_root = workspace.join("docs/StarBreaker/ui-rework-artifacts/phase-1/ir-dumps");
    let out_target_a = out_root.join("ui_target_a-screen_16x9_a-ir.json");
    let out_target_b = out_root.join("ui_target_b-mesh_end_screen_plane-ir.json");

    dump_one(&fetcher, localization_map.as_ref(), ui_target_a_guid, &out_target_a)?;
    dump_one(&fetcher, localization_map.as_ref(), ui_target_b_guid, &out_target_b)?;

    println!("Wrote {}", out_target_a.display());
    println!("Wrote {}", out_target_b.display());

    Ok(())
}
