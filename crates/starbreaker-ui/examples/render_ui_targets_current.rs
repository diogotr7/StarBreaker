use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use starbreaker_p4k::MappedP4k;
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, ManufacturerStyle, PipelineInputs, StyleFetcher, StyleLoader, SwfFetcher,
    UiBindingView, UiError, compile_ir_for_binding,
    render_for_binding,
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
    fn load_style_for_manufacturer(
        &self,
        manufacturer_id: &str,
    ) -> Result<ManufacturerStyle, UiError> {
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
    fn fetch_manufacturer_style(
        &self,
        manufacturer_id: &str,
    ) -> Result<ManufacturerStyle, UiError> {
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

        let extracted_name = starbreaker_ui::pipeline::extract_record_name(&record_name);
        by_name.insert(extracted_name.clone(), record_id.clone());
        by_name.insert(extracted_name.to_ascii_lowercase(), record_id.clone());
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

fn parse_animation_sample_percent() -> Result<Option<f32>, String> {
    let mut args = std::env::args().skip(1);
    let mut animation_sample_percent = Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT);
    while let Some(arg) = args.next() {
        let value = if let Some(value) = arg.strip_prefix("--animation-percent=") {
            value.to_string()
        } else if arg == "--animation-percent" {
            args.next()
                .ok_or_else(|| "--animation-percent requires a value from 0 to 100".to_string())?
        } else {
            return Err(format!(
                "unknown argument {arg}; supported option: --animation-percent <0..100>"
            ));
        };

        let percent = value
            .parse::<f32>()
            .map_err(|_| format!("invalid --animation-percent value {value:?}"))?;
        if !(0.0..=100.0).contains(&percent) {
            return Err(format!("--animation-percent must be from 0 to 100, got {percent}"));
        }
        animation_sample_percent = Some(percent);
    }
    Ok(animation_sample_percent)
}

fn render_target(
    output_path: &Path,
    guid: &'static str,
    helper_name: &'static str,
    fetcher: &FsCanvasFetcher,
    style_fetcher: &FsStyleFetcher,
    file_fetcher: &P4kFileFetcher,
    localization_map: Option<HashMap<String, String>>,
    loc_fetcher: Option<&dyn starbreaker_ui::bb_loc::LocFetcher>,
    animation_sample_percent: Option<f32>,
) -> Result<(), String> {
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
        animation_sample_percent,
        localization_map,
        loc_fetcher,
        derived_values: None,
        hologram_fetcher: None,
    };

    let png = render_for_binding(&inputs).map_err(|e| {
        format!(
            "failed to render {} ({}): {e}",
            output_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("target output"),
            guid
        )
    })?;

    fs::write(output_path, png)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    Ok(())
}

fn print_layout_locations(inputs: &PipelineInputs<'_>, label: &str, query: &str) -> Result<(), String> {
    let ir = compile_ir_for_binding(inputs)
        .map_err(|e| format!("failed to compile IR for location query ({label}): {e}"))?;

    let query_lc = query.to_ascii_lowercase();
    println!("Location query '{}' on {}:", query, label);
    let mut hits = 0usize;
    for node in &ir.nodes {
        let name_lc = node.name.to_ascii_lowercase();
        let ty_lc = node.node_type.to_ascii_lowercase();
        if name_lc.contains(&query_lc) || ty_lc.contains(&query_lc) {
            hits += 1;
            println!(
                "  id={} name='{}' type='{}' x={:.1} y={:.1} w={:.1} h={:.1}",
                node.id,
                node.name,
                node.node_type,
                node.computed_rect.x,
                node.computed_rect.y,
                node.computed_rect.w,
                node.computed_rect.h
            );
        }
    }
    if hits == 0 {
        println!("  (no matches)");
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let animation_sample_percent = parse_animation_sample_percent()?;
    let workspace = PathBuf::from(format!("{}/projects/scorg_tools", std::env::var("HOME").unwrap_or_default()));
    let canvas_root = workspace.join("ships/dcb_canvas/libs/foundry/records");
    let localization_map = load_localization_map(&workspace);
    let fetcher = load_canvas_index(&canvas_root)?;

    let output_dir = workspace.join("StarBreaker/test-artifacts/ui/ui_target_a-current.png");
    if let Some(parent) = output_dir.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    let p4k = match open_p4k() {
        Ok(p4k) => Some(p4k),
        Err(e) => {
            eprintln!("warning: {e}");
            None
        }
    };

    let style_fetcher = FsStyleFetcher {
        styles_root: workspace.join("ships/dcb_canvas/libs/foundry/records/ui/buildingblocks/styles"),
    };
    let file_fetcher = p4k
        .as_ref()
        .map(|p4k| P4kFileFetcher { p4k: Arc::clone(p4k) })
        .ok_or_else(|| "P4K-backed fetcher unavailable".to_string())?;
    let ini_loc_fetcher = (std::env::var("SB_UI_USE_LOC_FETCHER").as_deref() == Ok("1"))
        .then(|| starbreaker_ui::bb_loc_p4k::load_global_ini(|path| read_p4k_path(&file_fetcher.p4k, path).ok()));
    let loc_fetcher = ini_loc_fetcher
        .as_ref()
        .map(|fetcher| fetcher as &dyn starbreaker_ui::bb_loc::LocFetcher);

    let comparison_dir = workspace.join("StarBreaker/test-artifacts/ui");
    let ui_target_a_output = comparison_dir.join("ui_target_a-current.png");
    let ui_target_b_output = comparison_dir.join("ui_target_b-current.png");
    let locate_query = std::env::var("SB_UI_LOCATE").ok().filter(|s| !s.trim().is_empty());

    if loc_fetcher.is_none() {
        let loc_a = localization_map.clone();
        let loc_b = localization_map.clone();
        std::thread::scope(|scope| {
            let render_a = scope.spawn(|| {
                render_target(
                    &ui_target_a_output,
                    "534bab84-299b-479a-a4af-4469df112ea7",
                    "ui_target_a-phase2-render",
                    &fetcher,
                    &style_fetcher,
                    &file_fetcher,
                    loc_a,
                    None,
                    animation_sample_percent,
                )
            });
            let render_b = scope.spawn(|| {
                render_target(
                    &ui_target_b_output,
                    "e9ad809d-ebcf-43a3-bb20-120f64556aef",
                    "ui_target_b-phase2-render",
                    &fetcher,
                    &style_fetcher,
                    &file_fetcher,
                    loc_b,
                    None,
                    animation_sample_percent,
                )
            });

            let result_a = render_a
                .join()
                .map_err(|_| "ui_target_a render thread panicked".to_string())?;
            let result_b = render_b
                .join()
                .map_err(|_| "ui_target_b render thread panicked".to_string())?;

            result_a?;
            result_b?;
            Ok::<(), String>(())
        })?;
    } else {
        render_target(
            &ui_target_a_output,
            "534bab84-299b-479a-a4af-4469df112ea7",
            "ui_target_a-phase2-render",
            &fetcher,
            &style_fetcher,
            &file_fetcher,
            localization_map.clone(),
            loc_fetcher,
            animation_sample_percent,
        )?;
        render_target(
            &ui_target_b_output,
            "e9ad809d-ebcf-43a3-bb20-120f64556aef",
            "ui_target_b-phase2-render",
            &fetcher,
            &style_fetcher,
            &file_fetcher,
            localization_map,
            loc_fetcher,
            animation_sample_percent,
        )?;
    }

    if let Some(query) = locate_query.as_deref() {
        let ui_target_a_binding = UiBindingView {
            canvas_guid: Some("534bab84-299b-479a-a4af-4469df112ea7"),
            content_canvas_guid: Some("534bab84-299b-479a-a4af-4469df112ea7"),
            binding_kind: Some("mfd"),
            manufacturer_id: Some("drak"),
            helper_name: Some("ui_target_a-phase2-render"),
            default_view_index: None,
            default_screen_slot: None,
            screen_name_loc_key: None,
            transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
        };
        let ui_target_a_inputs = PipelineInputs {
            binding: &ui_target_a_binding,
            canvas_fetcher: &fetcher,
            swf_fetcher: &file_fetcher,
            style_fetcher: &style_fetcher,
            asset_fetcher: &file_fetcher,
            target_size: (1920, 1080),
            apply_postprocess: false,
            animation_sample_percent,
            localization_map: load_localization_map(&workspace),
            loc_fetcher: None,
            derived_values: None,
            hologram_fetcher: None,
        };
        print_layout_locations(&ui_target_a_inputs, "ui_target_a", query)?;

        let ui_target_b_binding = UiBindingView {
            canvas_guid: Some("e9ad809d-ebcf-43a3-bb20-120f64556aef"),
            content_canvas_guid: Some("e9ad809d-ebcf-43a3-bb20-120f64556aef"),
            binding_kind: Some("mfd"),
            manufacturer_id: Some("drak"),
            helper_name: Some("ui_target_b-phase2-render"),
            default_view_index: None,
            default_screen_slot: None,
            screen_name_loc_key: None,
            transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
        };
        let ui_target_b_inputs = PipelineInputs {
            binding: &ui_target_b_binding,
            canvas_fetcher: &fetcher,
            swf_fetcher: &file_fetcher,
            style_fetcher: &style_fetcher,
            asset_fetcher: &file_fetcher,
            target_size: (1920, 1080),
            apply_postprocess: false,
            animation_sample_percent,
            localization_map: load_localization_map(&workspace),
            loc_fetcher: None,
            derived_values: None,
            hologram_fetcher: None,
        };
        print_layout_locations(&ui_target_b_inputs, "ui_target_b", query)?;
    }

    println!("Wrote {}", ui_target_a_output.display());
    println!("Wrote {}", ui_target_b_output.display());
    Ok(())
}