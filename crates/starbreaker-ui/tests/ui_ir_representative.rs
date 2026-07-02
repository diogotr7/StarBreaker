use std::collections::HashMap;

use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
    compile_ir_for_binding, stable_hash_ui_ir, validate_ui_ir_document,
};
use starbreaker_ui::pipeline::AssetFetcher;

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
            .find(|v| {
                v.get("_RecordName_")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n == record_name)
            })
            .cloned()
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))
    }
}

struct DummySwfFetcher;

impl SwfFetcher for DummySwfFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("swf not required for IR fixture tests".to_string()))
    }
}

struct DummyStyleFetcher;

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Err(UiError::RenderError("style not required for IR fixture tests".to_string()))
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
            include_str!("fixtures/canvas/BB_ScreenRadar_C_App_Starmap_68ff6d17.json"),
        ),
        (
            "fixture-power",
            include_str!("fixtures/canvas/EC_PowerManagement_3228e5cc.json"),
        ),
        (
            "fixture-target-gen",
            include_str!("fixtures/canvas/GEN_MC_S_Target_dd9ed6dc.json"),
        ),
        (
            "fixture-self-master",
            include_str!("fixtures/canvas/MC_S_Self_Master_680a71df.json"),
        ),
        (
            "fixture-target-master",
            include_str!("fixtures/canvas/MC_S_Target_Master_b8d2d65c.json"),
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

#[test]
fn compile_ir_for_representative_fixtures_validates() {
    let canvas_fetcher = FixtureCanvasFetcher {
        by_guid: load_fixture_map(),
    };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    for guid in [
        "fixture-radar",
        "fixture-power",
        "fixture-target-gen",
        "fixture-self-master",
        "fixture-target-master",
    ] {
        let binding = UiBindingView {
            canvas_guid: Some(guid),
            content_canvas_guid: Some(guid),
            binding_kind: Some("mfd"),
            manufacturer_id: Some("drak"),
            helper_name: Some("representative-fixture"),
            default_view_index: None,
            default_screen_slot: None,
            screen_name_loc_key: None,
            transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
        };

        let inputs = PipelineInputs {
            binding: &binding,
            canvas_fetcher: &canvas_fetcher,
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
            .unwrap_or_else(|e| panic!("IR should compile for fixture {guid}: {e}"));
        validate_ui_ir_document(&ir)
            .unwrap_or_else(|errs| panic!("IR validation should pass for fixture {guid}: {errs:?}"));
        assert!(!ir.nodes.is_empty(), "fixture {guid} should compile at least one node");
    }
}

#[test]
fn stable_hash_is_repeatable_for_representative_fixture() {
    let canvas_fetcher = FixtureCanvasFetcher {
        by_guid: load_fixture_map(),
    };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("fixture-target-master"),
        content_canvas_guid: Some("fixture-target-master"),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("stable-hash"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: &canvas_fetcher,
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

    let ir_a = compile_ir_for_binding(&inputs).expect("first compile");
    let ir_b = compile_ir_for_binding(&inputs).expect("second compile");
    let hash_a = stable_hash_ui_ir(&ir_a).expect("hash a");
    let hash_b = stable_hash_ui_ir(&ir_b).expect("hash b");

    assert_eq!(hash_a, hash_b);
    assert!(!hash_a.is_empty());
}
