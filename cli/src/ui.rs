//! UI render replay commands (UI Plan 2 Phase 9).
//!
//! `starbreaker ui render` re-renders UI binding PNGs from an existing
//! decomposed `scene.json` using the canvas composer pipeline wired in
//! Phase 9. Each binding with a `canvas_guid` will be rendered using
//! `starbreaker-ui` and written to the output directory.
//!
//! See `/docs/ui-plan2.md`.

use std::path::PathBuf;

use clap::Subcommand;
use serde_json::Value;
use starbreaker_3d::types::UiBinding;
use starbreaker_datacore::database::Database;

use crate::common::load_dcb_bytes;
use crate::error::{CliError, Result};

#[derive(Subcommand, Debug)]
pub enum UiCommand {
    /// Render UI PNGs from an existing decomposed scene.json.
    ///
    /// Walks the `ui_bindings` in every scene instance and renders a PNG for
    /// each binding that has a `canvas_guid`. Writes output files to `--out-dir`
    /// (default: the directory that contains `scene.json`).
    Render {
        /// Path to the decomposed `scene.json` to re-render.
        #[arg(long)]
        scene: PathBuf,

        /// Output directory for rendered PNGs (default: same directory as scene.json).
        #[arg(long)]
        out_dir: Option<PathBuf>,

        /// Texture mip level for output PNG size (0=full, 2=1/4, 4=1/16).
        #[arg(long, default_value = "0")]
        mip: u32,

        /// Filter: only render the named helper (e.g. "Screen_Left_Lower_RTT").
        #[arg(long)]
        helper: Option<String>,

        /// Optional directory for canonical IR JSON dumps for each rendered binding.
        #[arg(long)]
        dump_ir_dir: Option<PathBuf>,

        /// Path to Data.p4k (default: SC_DATA_P4K env var or auto-discover).
        #[arg(long)]
        p4k: Option<PathBuf>,
    },
    /// Render a single MFD SWF file.
    ///
    /// Disabled in UI Plan 2. MFD rendering is being replaced by canvas
    /// composition; standalone SWF rendering is no longer a supported path.
    Mfd,
}

impl UiCommand {
    pub fn run(self) -> Result<()> {
        match self {
            UiCommand::Render { scene, out_dir, mip, helper, dump_ir_dir, p4k } => {
                run_render(&scene, out_dir.as_deref(), dump_ir_dir.as_deref(), mip, helper.as_deref(), p4k.as_deref().map(|p| p.as_ref()))
            }
            UiCommand::Mfd => Err(CliError::MissingRequirement(
                "starbreaker ui mfd: removed in UI Plan 2. MFD content is now \
                 produced by canvas composition, not standalone SWF rendering. \
                 See /docs/ui-plan2.md".to_string(),
            )),
        }
    }
}

fn run_render(
    scene_path: &std::path::Path,
    out_dir: Option<&std::path::Path>,
    dump_ir_dir: Option<&std::path::Path>,
    texture_mip: u32,
    helper_filter: Option<&str>,
    p4k_path: Option<&std::path::Path>,
) -> Result<()> {
    // Load P4K and DataCore.
    let (p4k_opt, dcb_bytes) = load_dcb_bytes(p4k_path, None)?;
    let p4k = p4k_opt.ok_or_else(|| CliError::MissingRequirement(
        "P4K is required for ui render; set SC_DATA_P4K or pass --p4k".to_string(),
    ))?;
    let db = Database::from_bytes(&dcb_bytes)?;

    // Read and parse scene.json.
    let scene_bytes = std::fs::read(scene_path)?;
    let scene: Value = serde_json::from_slice(&scene_bytes)?;

    // Resolve output directory.
    let default_out = scene_path.parent().unwrap_or(std::path::Path::new("."));
    let out_dir = out_dir.unwrap_or(default_out);
    std::fs::create_dir_all(out_dir)?;
    if let Some(dump_ir_dir) = dump_ir_dir {
        std::fs::create_dir_all(dump_ir_dir)?;
    }

    // Collect bindings from root and every child instance.
    let mut bindings: Vec<UiBinding> = Vec::new();
    collect_bindings_from_value(&scene, &mut bindings);

    if bindings.is_empty() {
        eprintln!("No ui_bindings found in {}", scene_path.display());
        return Ok(());
    }

    // Derive manufacturer_id from the scene's root_entity.entity_name, e.g.
    // "EntityClassDefinition.RSI_Aurora_Mk2" → "RSI_Aurora_Mk2" → prefix "RSI" → "rsi".
    let manufacturer_id: Option<String> = scene
        .get("root_entity")
        .and_then(|re| re.get("entity_name"))
        .and_then(|v| v.as_str())
        .and_then(|s| {
            // Strip optional "EntityClassDefinition." prefix.
            let stem = s.rsplit_once('.').map(|(_, r)| r).unwrap_or(s);
            // First underscore-separated token is the manufacturer code.
            let prefix = stem
                .split(|c: char| c == '_' || c == '-' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if prefix.is_empty() { None } else { Some(prefix) }
        });

    let loc_data = starbreaker_3d::ui_pipeline::UiLocData::load(&p4k);
    // Replay derives the same per-ship UI values the export pipeline feeds
    // (power pools → pip stacks, temps) from the scene's root entity, so a
    // re-render matches the exported PNGs.
    let root_entity_name: Option<String> = scene
        .get("root_entity")
        .and_then(|re| re.get("entity_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.rsplit_once('.').map(|(_, r)| r).unwrap_or(s).to_string());
    let ship_data = match root_entity_name.as_deref() {
        Some(name) => starbreaker_3d::ui_pipeline::UiShipData::derive(&db, name),
        None => starbreaker_3d::ui_pipeline::UiShipData::none(),
    };
    let mut rendered = 0usize;
    let mut failed = 0usize;

    for binding in &bindings {
        if let Some(filter) = helper_filter {
            if binding.helper_name.as_deref() != Some(filter) {
                continue;
            }
        }
        // Only bindings with a canvas_guid can be rendered.
        if binding.canvas_guid.is_none() && binding.content_canvas_guid.is_none() {
            continue;
        }

        match starbreaker_3d::ui_pipeline::render_ui_binding_png(&binding, &db, &p4k, texture_mip, manufacturer_id.as_deref(), &loc_data, &ship_data, None, root_entity_name.as_deref()) {
            Ok(png_bytes) => {
                let file_name = png_name_for_binding(binding, texture_mip);
                let dest = out_dir.join(&file_name);
                std::fs::write(&dest, &png_bytes)?;
                eprintln!("  wrote  {}", dest.display());
                if let Some(dump_ir_dir) = dump_ir_dir {
                    let ir_json = starbreaker_3d::ui_pipeline::compile_ui_binding_ir_json(
                        binding,
                        &db,
                        &p4k,
                        texture_mip,
                        manufacturer_id.as_deref(),
                        &loc_data,
                        &ship_data,
                    )
                    .map_err(CliError::MissingRequirement)?;
                    let ir_dest = dump_ir_dir.join(format!("{}.ir.json", file_name.trim_end_matches(".png")));
                    std::fs::write(&ir_dest, ir_json)?;
                    eprintln!("  wrote  {}", ir_dest.display());
                }
                rendered += 1;
            }
            Err(e) => {
                eprintln!("  failed {:?}: {}", binding.helper_name, e);
                failed += 1;
            }
        }
    }

    eprintln!("ui render: {rendered} rendered, {failed} failed");
    Ok(())
}

/// Walk the scene JSON tree and collect every `UiBinding` object found inside
/// `ui_bindings` arrays.
fn collect_bindings_from_value(v: &Value, out: &mut Vec<UiBinding>) {
    match v {
        Value::Object(map) => {
            if let Some(Value::Array(arr)) = map.get("ui_bindings") {
                for item in arr {
                    if let Some(b) = parse_ui_binding(item) {
                        out.push(b);
                    }
                }
            }
            for child in map.values() {
                collect_bindings_from_value(child, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_bindings_from_value(item, out);
            }
        }
        _ => {}
    }
}

/// Parse a `serde_json::Value` object into a `UiBinding`.
fn parse_ui_binding(v: &Value) -> Option<UiBinding> {
    let obj = v.as_object()?;
    let str_field = |key: &str| -> Option<String> {
        obj.get(key)?.as_str().map(|s| s.to_owned())
    };
    let opt_str = |key: &str| -> Option<String> {
        obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_owned())
    };
    let opt_u16 = |key: &str| -> Option<u16> {
        obj.get(key)?.as_u64().map(|n| n as u16)
    };
    let opt_u32 = |key: &str| -> Option<u32> {
        obj.get(key)?.as_u64().map(|n| n as u32)
    };
    let opt_u8 = |key: &str| -> Option<u8> {
        obj.get(key)?.as_u64().map(|n| n as u8)
    };
    let default_light_color: Option<[u8; 4]> = obj
        .get("default_light_color")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            if arr.len() == 4 {
                Some([
                    arr[0].as_u64()? as u8,
                    arr[1].as_u64()? as u8,
                    arr[2].as_u64()? as u8,
                    arr[3].as_u64()? as u8,
                ])
            } else {
                None
            }
        });

    Some(UiBinding {
        binding_kind: str_field("binding_kind")?,
        source_entity_name: str_field("source_entity_name")?,
        helper_name: opt_str("helper_name"),
        default_view: opt_str("default_view"),
        default_state_is_off: obj
            .get("default_state_is_off")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        default_state_name: opt_str("default_state_name"),
        default_light_color,
        default_light_intensity_milli: opt_u16("default_light_intensity_milli"),
        canvas_guid: opt_str("canvas_guid"),
        canvas_record_name: opt_str("canvas_record_name"),
        canvas_record_path: opt_str("canvas_record_path"),
        canvas_widget_canvas_path: opt_str("canvas_widget_canvas_path"),
        canvas_widget_url_postfix: opt_str("canvas_widget_url_postfix"),
        canvas_widget_url_optional: opt_str("canvas_widget_url_optional"),
        canvas_variable_binding: opt_str("canvas_variable_binding"),
        content_canvas_guid: opt_str("content_canvas_guid"),
        content_canvas_record_name: opt_str("content_canvas_record_name"),
        screen_name_loc_key: opt_str("screen_name_loc_key"),
        transit_location_loc_key: opt_str("transit_location_loc_key"),
        dashboard_view_index: opt_u32("dashboard_view_index"),
        dashboard_screen_slot: opt_u32("dashboard_screen_slot"),
        owner_source_file: opt_str("owner_source_file"),
        runtime_image_source: opt_str("runtime_image_source"),
        generated_image_path: opt_str("generated_image_path"),
        generated_context_manifest_path: opt_str("generated_context_manifest_path"),
        generated_resolved_source_path: opt_str("generated_resolved_source_path"),
        generated_backend: opt_str("generated_backend"),
        generated_provenance: opt_str("generated_provenance"),
        generated_confidence: opt_u8("generated_confidence"),
        ui_screen_aspect_w_over_h: obj
            .get("ui_screen_aspect_w_over_h")
            .and_then(|v| v.as_f64())
            .map(|n| n as f32),
    })
}

/// Build a PNG filename for a binding. Helper name first; bindings without
/// one (e.g. the medical bed) fall back to the human-readable content/canvas
/// record name before the GUID, so replay outputs are findable without a
/// content-match hunt (ledger item 31).
fn png_name_for_binding(binding: &UiBinding, mip: u32) -> String {
    let base = binding
        .helper_name
        .as_deref()
        .or(binding.content_canvas_record_name.as_deref())
        .or(binding.canvas_record_name.as_deref())
        .or(binding.canvas_guid.as_deref())
        .unwrap_or(&binding.binding_kind);
    // Sanitise to filename-safe characters.
    let safe: String = base
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    format!("{safe}_TEX{mip}.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding_with(helper: Option<&str>, content: Option<&str>, guid: Option<&str>) -> UiBinding {
        let mut binding = UiBinding::default();
        binding.binding_kind = "physical".to_string();
        binding.helper_name = helper.map(str::to_owned);
        binding.content_canvas_record_name = content.map(str::to_owned);
        binding.canvas_guid = guid.map(str::to_owned);
        binding
    }

    #[test]
    fn png_name_prefers_helper_then_content_record_name_then_guid() {
        let helper = binding_with(Some("Screen_Annunciator_L"), Some("BuildingBlocks_Canvas.X"), Some("guid-1"));
        assert_eq!(png_name_for_binding(&helper, 0), "Screen_Annunciator_L_TEX0.png");

        let content = binding_with(None, Some("BuildingBlocks_Canvas.I_Med_MedicalBed_A"), Some("guid-1"));
        assert_eq!(
            png_name_for_binding(&content, 0),
            "BuildingBlocks_Canvas.I_Med_MedicalBed_A_TEX0.png"
        );

        let guid_only = binding_with(None, None, Some("534bab84-299b"));
        assert_eq!(png_name_for_binding(&guid_only, 0), "534bab84-299b_TEX0.png");
    }
}
