use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use starbreaker_p4k::MappedP4k;
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, ManufacturerStyle, PipelineInputs, StyleFetcher, StyleLoader, SwfFetcher,
    UiBindingView, UiError, compile_ir_for_binding,
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

struct P4kFileFetcher {
    p4k: Arc<MappedP4k>,
}

impl SwfFetcher for P4kFileFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError> {
        read_p4k_path(&self.p4k, p4k_path)
    }
}

impl AssetFetcher for P4kFileFetcher {
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        read_p4k_path(&self.p4k, p4k_path).ok()
    }
}

struct FsStyleFetcher {
    styles_root: PathBuf,
}

impl FsStyleFetcher {
    fn load_style_for_manufacturer(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError> {
        let id = manufacturer_id.to_ascii_lowercase();
        let candidates = [
            self.styles_root.join(format!("{id}.json")),
            self.styles_root.join(format!("s_{id}_hud.json")),
            self.styles_root.join(format!("s_{id}_env.json")),
        ];

        for path in candidates {
            if !path.is_file() {
                continue;
            }
            let record_json = load_canvas_json_from_path(&path)
                .map_err(|e| UiError::RenderError(format!("failed to parse {}: {e}", path.display())))?;
            return StyleLoader::for_manufacturer(&id).parse_buildingblocks_style_record(&record_json);
        }

        Err(UiError::RenderError(format!(
            "missing BuildingBlocks style record for manufacturer '{manufacturer_id}' under {}",
            self.styles_root.display()
        )))
    }
}

impl StyleFetcher for FsStyleFetcher {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError> {
        self.load_style_for_manufacturer(manufacturer_id)
    }
}

fn normalize_p4k_path(path: &str) -> String {
    let with_prefix = if path.to_ascii_lowercase().starts_with("data\\")
        || path.to_ascii_lowercase().starts_with("data/")
    {
        path.to_string()
    } else {
        format!("Data\\{}", path)
    };
    with_prefix.replace('/', "\\")
}

fn read_p4k_path(p4k: &MappedP4k, path: &str) -> Result<Vec<u8>, UiError> {
    let normalized = normalize_p4k_path(path);
    p4k.read_file(&normalized)
        .map_err(|e| UiError::RenderError(format!("failed to read '{}' from P4K: {e}", normalized)))
}

fn open_p4k() -> Result<Arc<MappedP4k>, String> {
    starbreaker_p4k::open_p4k()
        .map(Arc::new)
        .map_err(|e| format!("failed to open Data.p4k: {e}"))
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

        let Some(record_name) = json.get("_RecordName_").and_then(|v| v.as_str()).map(str::to_owned) else {
            continue;
        };
        let Some(record_id) = json.get("_RecordId_").and_then(|v| v.as_str()).map(str::to_owned) else {
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

        let extracted_name = starbreaker_ui::pipeline::extract_record_name(&record_name);
        by_name.insert(extracted_name.clone(), record_id.clone());
        by_name.insert(extracted_name.to_ascii_lowercase(), record_id.clone());
    }

    Ok(FsCanvasFetcher { guid_to_path, by_name })
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

fn trace_for_canvas(
    guid: &'static str,
    helper_name: &'static str,
    fetcher: &FsCanvasFetcher,
    style_fetcher: &FsStyleFetcher,
    file_fetcher: &P4kFileFetcher,
    localization_map: Option<HashMap<String, String>>,
) -> Result<String, String> {
    let binding = UiBindingView {
        canvas_guid: Some(guid),
        content_canvas_guid: Some(guid),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some(helper_name),
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
        swf_fetcher: file_fetcher,
        style_fetcher,
        asset_fetcher: file_fetcher,
        target_size: (1920, 1080),
        apply_postprocess: false,
        animation_sample_percent: Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT),
        localization_map,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs)
        .map_err(|e| format!("failed to compile IR for {helper_name}: {e}"))?;

    let mut out = String::new();
    out.push_str(&format!("## {helper_name}\n"));
    out.push_str(&format!("- Canvas GUID: `{guid}`\n"));
    out.push_str(&format!("- Canvas name: `{}`\n", ir.canvas_name.as_deref().unwrap_or("<unknown>")));
    out.push_str(&format!("- Renderer hint: `{:?}`\n", ir.renderer_hint));
    out.push_str(&format!("- Selected style source: `{}`\n", ir.selected_style_source.as_deref().unwrap_or("<none>")));
    out.push_str(&format!("- Selected SWF source: `{}`\n", ir.selected_swf_source.as_deref().unwrap_or("<none>")));
    out.push_str(&format!("- Confidence: `{}`\n", ir.confidence));
    out.push_str(&format!("- Node count: `{}`\n", ir.nodes.len()));
    if ir.warnings.is_empty() {
        out.push_str("- Warnings: none\n");
    } else {
        out.push_str("- Warnings:\n");
        for warning in &ir.warnings {
            out.push_str(&format!("  - {}\n", warning));
        }
    }
    if ir.unresolved_references.is_empty() {
        out.push_str("- Unresolved references: none\n");
    } else {
        out.push_str("- Unresolved references:\n");
        for value in &ir.unresolved_references {
            out.push_str(&format!("  - {}\n", value));
        }
    }
    out.push('\n');
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = PathBuf::from(format!("{}/projects/scorg_tools", std::env::var("HOME").unwrap_or_default()));
    let canvas_root = workspace.join("ships/dcb_canvas/libs/foundry/records");
    let fetcher = load_canvas_index(&canvas_root)?;
    let style_fetcher = FsStyleFetcher {
        styles_root: workspace.join("ships/dcb_canvas/libs/foundry/records/ui/buildingblocks/styles"),
    };
    let p4k = open_p4k()?;
    let file_fetcher = P4kFileFetcher { p4k };
    let localization_map = load_localization_map(&workspace);

    let mut report = String::new();
    report.push_str("# Phase 3 Decision Traces\n\n");
    report.push_str("Date: 2026-05-27\n\n");
    report.push_str("Verification images refreshed via `./scripts/generate_ui_regression_artifacts.sh`:\n");
    report.push_str("- `StarBreaker/test-artifacts/ui/ui_target_a.png`\n");
    report.push_str("- `StarBreaker/test-artifacts/ui/ui_target_b.png`\n\n");
    report.push_str(&trace_for_canvas(
        "534bab84-299b-479a-a4af-4469df112ea7",
        "ui_target_a-phase3-trace",
        &fetcher,
        &style_fetcher,
        &file_fetcher,
        localization_map.clone(),
    )?);
    report.push_str(&trace_for_canvas(
        "e9ad809d-ebcf-43a3-bb20-120f64556aef",
        "ui_target_b-phase3-trace",
        &fetcher,
        &style_fetcher,
        &file_fetcher,
        localization_map,
    )?);

    let output = workspace.join("docs/StarBreaker/ui-rework-artifacts/phase-3/decision-traces.md");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, report)?;
    println!("Wrote {}", output.display());
    Ok(())
}