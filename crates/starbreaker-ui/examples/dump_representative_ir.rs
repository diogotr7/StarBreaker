//! Dump canonical UI IR for the representative Phase 1 fixture canvases.
//!
//! Loads the same five fixture canvases used by `ui_ir_representative.rs` and
//! writes human-readable IR JSON files under the Phase 1 artifact directory.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView,
    UiError, compile_ir_for_binding,
};

struct FixtureCanvasFetcher {
    by_guid: HashMap<String, serde_json::Value>,
}

impl CanvasFetcher for FixtureCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        self.by_guid
            .get(guid)
            .cloned()
            .ok_or_else(|| UiError::RenderError(format!("missing canvas guid: {guid}")))
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        self.by_guid
            .values()
            .find(|value| {
                value
                    .get("_RecordName_")
                    .and_then(|name| name.as_str())
                    .is_some_and(|name| name == record_name)
            })
            .cloned()
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))
    }
}

struct DummySwfFetcher;

impl SwfFetcher for DummySwfFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError(
            "swf not required for representative IR dump".to_string(),
        ))
    }
}

struct DummyStyleFetcher;

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Err(UiError::RenderError(
            "style not required for representative IR dump".to_string(),
        ))
    }
}

struct DummyAssetFetcher;

impl AssetFetcher for DummyAssetFetcher {
    fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
        None
    }
}

fn load_fixture_map() -> HashMap<String, serde_json::Value> {
    let fixtures: [(&str, &str); 5] = [
        (
            "fixture-radar",
            include_str!("../tests/fixtures/canvas/BB_ScreenRadar_C_App_Starmap_68ff6d17.json"),
        ),
        (
            "fixture-power",
            include_str!("../tests/fixtures/canvas/EC_PowerManagement_3228e5cc.json"),
        ),
        (
            "fixture-target-gen",
            include_str!("../tests/fixtures/canvas/GEN_MC_S_Target_dd9ed6dc.json"),
        ),
        (
            "fixture-self-master",
            include_str!("../tests/fixtures/canvas/MC_S_Self_Master_680a71df.json"),
        ),
        (
            "fixture-target-master",
            include_str!("../tests/fixtures/canvas/MC_S_Target_Master_b8d2d65c.json"),
        ),
    ];

    fixtures
        .iter()
        .map(|(guid, raw)| {
            let parsed: serde_json::Value =
                serde_json::from_str(raw).expect("fixture JSON should parse");
            ((*guid).to_string(), parsed)
        })
        .collect()
}

fn dump_one(
    fetcher: &FixtureCanvasFetcher,
    fixture_guid: &str,
    output_name: &str,
) -> Result<PathBuf, String> {
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some(fixture_guid),
        content_canvas_guid: Some(fixture_guid),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("representative-ir-dump"),
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
        target_size: (2048, 1024),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs)
        .map_err(|error| format!("failed to compile IR for {fixture_guid}: {error}"))?;

    let output_path = PathBuf::from(format!("{}/projects/scorg_tools/docs/StarBreaker/ui-rework-artifacts/phase-1/ir-dumps", std::env::var("HOME").unwrap_or_default()))
        .join(output_name);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }

    let serialized = serde_json::to_string_pretty(&ir)
        .map_err(|error| format!("failed to serialize IR for {fixture_guid}: {error}"))?;
    fs::write(&output_path, serialized)
        .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;

    Ok(output_path)
}

fn main() -> Result<(), String> {
    let fetcher = FixtureCanvasFetcher {
        by_guid: load_fixture_map(),
    };

    let dumps = [
        ("fixture-radar", "fixture-radar-ir.json"),
        ("fixture-power", "fixture-power-ir.json"),
        ("fixture-target-gen", "fixture-target-gen-ir.json"),
        ("fixture-self-master", "fixture-self-master-ir.json"),
        ("fixture-target-master", "fixture-target-master-ir.json"),
    ];

    for (guid, output_name) in dumps {
        let output_path = dump_one(&fetcher, guid, output_name)?;
        println!("Wrote {}", output_path.display());
    }

    Ok(())
}