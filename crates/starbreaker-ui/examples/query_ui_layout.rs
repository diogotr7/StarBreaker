use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbaImage;
use starbreaker_p4k::MappedP4k;
use starbreaker_ui::bb_atlas::AtlasLibrary;
use starbreaker_ui::pipeline::{AssetFetcher, CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView};
use starbreaker_ui::ir_compose::{
    debug_linear_progress_meter_rect, debug_node_draw_rect, debug_text_drawn_bounds,
    debug_text_rects,
};
use starbreaker_ui::{ManufacturerStyle, StyleLoader, UiError, compile_ir_for_binding};

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
            .map_err(|e| UiError::RenderError(format!("failed to load {}: {e}", path.display())))
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

struct FsStyleFetcher {
    styles_root: PathBuf,
}

impl StyleFetcher for FsStyleFetcher {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError> {
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

struct NullSwfFetcher;

impl SwfFetcher for NullSwfFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError(format!(
            "SWF fetch not supported in query_ui_layout example: {p4k_path}"
        )))
    }
}

struct NullAssetFetcher;

impl AssetFetcher for NullAssetFetcher {
    fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
        None
    }
}

struct P4kFileFetcher {
    p4k: Arc<MappedP4k>,
}

impl AssetFetcher for P4kFileFetcher {
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        read_p4k_path(&self.p4k, p4k_path).ok()
    }
}

impl SwfFetcher for P4kFileFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError> {
        read_p4k_path(&self.p4k, p4k_path)
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

fn sanitize_for_filename(value: &str) -> String {
    value.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn save_asset_preview(
    preview_dir: &Path,
    node: &starbreaker_ui::ui_ir::UiIrNode,
    draw_rect: starbreaker_ui::bb_layout::Rect,
) -> Result<Option<PathBuf>, String> {
    let Some(asset_ref) = node.asset_ref.as_deref() else {
        return Ok(None);
    };

    fs::create_dir_all(preview_dir)
        .map_err(|e| format!("failed to create {}: {e}", preview_dir.display()))?;
    let p4k = open_p4k()?;
    let fetcher = P4kFileFetcher { p4k };
    let atlas = AtlasLibrary::new(&fetcher, None);
    let image = atlas
        .resolve(
            asset_ref,
            draw_rect.w.round().max(1.0) as u32,
            draw_rect.h.round().max(1.0) as u32,
        )
        .ok_or_else(|| format!("failed to resolve asset preview for {}", asset_ref))?;

    let file_name = format!(
        "{}-{}-preview.png",
        sanitize_for_filename(&node.name),
        node.id
    );
    let output_path = preview_dir.join(file_name);
    save_png(&output_path, &image)?;
    Ok(Some(output_path))
}

fn save_png(path: &Path, image: &RgbaImage) -> Result<(), String> {
    image
        .save(path)
        .map_err(|e| format!("failed to save {}: {e}", path.display()))
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
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

        guid_to_path.insert(record_id.clone(), path.clone());
        by_name.insert(record_name.clone(), record_id.clone());
        by_name.insert(record_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(bare_name.clone(), record_id.clone());
        by_name.insert(bare_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(record_id.clone(), record_id);
    }

    Ok(FsCanvasFetcher { guid_to_path, by_name })
}

fn load_canvas_json_from_path(path: &Path) -> Result<serde_json::Value, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

fn print_usage() {
    eprintln!(
        "Usage: cargo run -p starbreaker-ui --example query_ui_layout -- \\\n  --canvas-guid <guid> --query <pattern> [--content-guid <guid>] [--manufacturer <id>] [--helper <name>] [--width <w>] [--height <h>]"
    );
}

fn main() -> Result<(), String> {
    let mut canvas_guid: Option<String> = None;
    let mut content_guid: Option<String> = None;
    let mut query: Option<String> = None;
    let mut manufacturer = String::from("drak");
    let mut helper = String::from("layout-query");
    let mut width: u32 = 1920;
    let mut height: u32 = 1080;
    let mut asset_preview_dir: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--canvas-guid" => canvas_guid = args.next(),
            "--content-guid" => content_guid = args.next(),
            "--query" => query = args.next(),
            "--manufacturer" => manufacturer = args.next().unwrap_or_else(|| "drak".to_string()),
            "--helper" => helper = args.next().unwrap_or_else(|| "layout-query".to_string()),
            "--width" => {
                width = args
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .ok_or_else(|| "invalid --width".to_string())?;
            }
            "--height" => {
                height = args
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .ok_or_else(|| "invalid --height".to_string())?;
            }
            "--asset-preview-dir" => {
                asset_preview_dir = args.next().map(PathBuf::from);
            }
            _ => {
                print_usage();
                return Err(format!("unknown arg: {arg}"));
            }
        }
    }

    let Some(canvas_guid) = canvas_guid else {
        print_usage();
        return Err("missing --canvas-guid".to_string());
    };
    let Some(query) = query else {
        print_usage();
        return Err("missing --query".to_string());
    };

    let workspace = PathBuf::from(format!("{}/projects/scorg_tools", std::env::var("HOME").unwrap_or_default()));
    let canvas_root = workspace.join("ships/dcb_canvas/libs/foundry/records");
    let fetcher = load_canvas_index(&canvas_root)?;
    let style_fetcher = FsStyleFetcher {
        styles_root: workspace.join("ships/dcb_canvas/libs/foundry/records/ui/buildingblocks/styles"),
    };
    let swf_fetcher = NullSwfFetcher;
    let asset_fetcher = NullAssetFetcher;

    let canvas_guid_s = canvas_guid;
    let content_guid_s = content_guid.unwrap_or_else(|| canvas_guid_s.clone());
    let binding = UiBindingView {
        canvas_guid: Some(canvas_guid_s.as_str()),
        content_canvas_guid: Some(content_guid_s.as_str()),
        binding_kind: Some("mfd"),
        manufacturer_id: Some(manufacturer.as_str()),
        helper_name: Some(helper.as_str()),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: &fetcher,
        swf_fetcher: &swf_fetcher,
        style_fetcher: &style_fetcher,
        asset_fetcher: &asset_fetcher,
        target_size: (width, height),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs)
        .map_err(|e| format!("failed to compile IR for query: {e}"))?;

    let query_lc = query.to_ascii_lowercase();
    let mut hits = 0usize;
    println!(
        "Layout query '{}' for canvas_guid={} content_guid={} target={}x{}",
        query, binding.canvas_guid.unwrap_or(""), binding.content_canvas_guid.unwrap_or(""), width, height
    );
    for node in &ir.nodes {
        let name_lc = node.name.to_ascii_lowercase();
        let ty_lc = node.node_type.to_ascii_lowercase();
        if name_lc.contains(&query_lc) || ty_lc.contains(&query_lc) {
            hits += 1;
            println!(
                "id={} parent={:?} active={} alpha={:.3} name='{}' type='{}' x={:.1} y={:.1} w={:.1} h={:.1}",
                node.id,
                node.parent_id,
                node.is_active,
                node.alpha,
                node.name,
                node.node_type,
                node.computed_rect.x,
                node.computed_rect.y,
                node.computed_rect.w,
                node.computed_rect.h
            );
            let draw_rect = debug_node_draw_rect(node, &ir);
            println!(
                "  draw_rect x={:.1} y={:.1} w={:.1} h={:.1}",
                draw_rect.x,
                draw_rect.y,
                draw_rect.w,
                draw_rect.h
            );
            println!(
                "  fill bg={:?} bg_token={:?} icon_tint={:?} icon_tint_token={:?} stroke={:?} stroke_token={:?}",
                node.background_fill_colour,
                node.background_fill_colour_token,
                node.icon_tint_colour,
                node.icon_tint_colour_token,
                node.stroke_colour,
                node.stroke_colour_token,
            );
            if let Some(asset_ref) = node.asset_ref.as_deref() {
                println!("  asset_ref {}", asset_ref);
                if let Some(preview_dir) = asset_preview_dir.as_deref() {
                    match save_asset_preview(preview_dir, node, draw_rect) {
                        Ok(Some(path)) => println!("  asset_preview {}", path.display()),
                        Ok(None) => {}
                        Err(err) => println!("  asset_preview_error {}", err),
                    }
                }
            }
            if let Some(custom_shape) = node.custom_shape.as_ref() {
                println!(
                    "  custom_shape type={:?} shape={:?} svg_path={:?} render_shape={:?}",
                    custom_shape.shape_type,
                    custom_shape.shape,
                    custom_shape.svg_path,
                    custom_shape.render_shape
                );
            }
            if let Some(text_rects) = debug_text_rects(node) {
                println!(
                    "  primary_text_rect x={:.1} y={:.1} w={:.1} h={:.1}",
                    text_rects.primary.x,
                    text_rects.primary.y,
                    text_rects.primary.w,
                    text_rects.primary.h
                );
                println!(
                    "  primary_text_origin x={:.1} y={:.1}",
                    text_rects.primary_text_origin.0,
                    text_rects.primary_text_origin.1
                );
                if let Some(secondary) = text_rects.secondary {
                    println!(
                        "  secondary_text_rect x={:.1} y={:.1} w={:.1} h={:.1}",
                        secondary.x,
                        secondary.y,
                        secondary.w,
                        secondary.h
                    );
                    if let Some(origin) = text_rects.secondary_text_origin {
                        println!(
                            "  secondary_text_origin x={:.1} y={:.1}",
                            origin.0,
                            origin.1
                        );
                    }
                }
            }
            if let Some(text_bounds) = debug_text_drawn_bounds(node) {
                println!(
                    "  primary_text_drawn_bounds x={:.1} y={:.1} w={:.1} h={:.1}",
                    text_bounds.primary.x,
                    text_bounds.primary.y,
                    text_bounds.primary.w,
                    text_bounds.primary.h
                );
                if let Some(secondary) = text_bounds.secondary {
                    println!(
                        "  secondary_text_drawn_bounds x={:.1} y={:.1} w={:.1} h={:.1}",
                        secondary.x,
                        secondary.y,
                        secondary.w,
                        secondary.h
                    );
                }
            }
            if let Some(meter_rect) = debug_linear_progress_meter_rect(node, &ir) {
                println!(
                    "  meter_draw_rect x={:.1} y={:.1} w={:.1} h={:.1}",
                    meter_rect.x,
                    meter_rect.y,
                    meter_rect.w,
                    meter_rect.h
                );
            }
        }
    }

    if hits == 0 {
        println!("(no matches)");
    }

    Ok(())
}
