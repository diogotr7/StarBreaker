use std::collections::HashMap;

use image::Rgba;
use starbreaker_ui::{
    CanvasFetcher, ManufacturerStyle, PipelineInputs, RgbaColor, StyleFetcher, SwfFetcher, UiBindingView,
    UiError, UiRendererHint, compile_ir_for_binding, render_for_binding, render_for_binding_ir,
};
use starbreaker_ui::pipeline::AssetFetcher;

struct MockCanvasFetcher {
    by_guid: HashMap<String, serde_json::Value>,
}

impl CanvasFetcher for MockCanvasFetcher {
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
        Err(UiError::RenderError("swf not required for IR compile test".to_string()))
    }
}

struct DummyStyleFetcher;

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Err(UiError::RenderError("style not required for IR compile test".to_string()))
    }
}

struct MockStyleFetcher {
    style: ManufacturerStyle,
}

impl StyleFetcher for MockStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<ManufacturerStyle, UiError> {
        Ok(self.style.clone())
    }
}

struct DummyAssetFetcher;

impl AssetFetcher for DummyAssetFetcher {
    fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn compile_ir_for_binding_uses_content_canvas_guid() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.TestRoot",
        "_RecordValue_": {
            "size": {"x": 100.0, "y": 100.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "status_text",
                    "text": "READY",
                    "isActive": true,
                    "position": {"x": 10.0, "y": 20.0},
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 60.0},
                        "height": {"behavior": "Fixed", "value": 20.0}
                    }
                }
            ],
            "operations": []
        }
    });

    let mut by_guid = HashMap::new();
    by_guid.insert("content-guid".to_string(), root);

    let canvas_fetcher = MockCanvasFetcher { by_guid };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("container-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("physical"),
        manufacturer_id: Some("drak"),
        helper_name: Some("Screen_Left_Upper_RTT"),
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
        target_size: (200, 100),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs).expect("IR should compile");
    assert_eq!(ir.canvas_guid, "content-guid");
    assert_eq!(ir.target_width, 200);
    assert_eq!(ir.target_height, 100);
    assert_eq!(ir.nodes.len(), 1);
    assert_eq!(ir.nodes[0].name, "status_text");
    assert_eq!(ir.renderer_hint, UiRendererHint::Bb);
}

#[test]
fn compile_ir_for_binding_matches_golden_fixture() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.TestRoot",
        "_RecordValue_": {
            "size": {"x": 100.0, "y": 100.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "status_text",
                    "text": "READY",
                    "isActive": true,
                    "position": {"x": 10.0, "y": 20.0},
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 60.0},
                        "height": {"behavior": "Fixed", "value": 20.0}
                    }
                }
            ],
            "operations": []
        }
    });

    let mut by_guid = HashMap::new();
    by_guid.insert("content-guid".to_string(), root);

    let canvas_fetcher = MockCanvasFetcher { by_guid };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("container-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("physical"),
        manufacturer_id: Some("drak"),
        helper_name: Some("Screen_Left_Upper_RTT"),
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
        target_size: (200, 100),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs).expect("IR should compile");
    let actual = serde_json::to_value(ir).expect("serialize actual");
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/ui_ir/expected_testroot_ir.json"
    ))
    .expect("parse expected fixture");

    assert_eq!(actual, expected);
}

#[test]
fn render_for_binding_ir_produces_nonempty_png() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.TestRoot",
        "_RecordValue_": {
            "size": {"x": 100.0, "y": 100.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "status_text",
                    "text": "READY",
                    "isActive": true,
                    "position": {"x": 10.0, "y": 20.0},
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 60.0},
                        "height": {"behavior": "Fixed", "value": 20.0}
                    }
                }
            ],
            "operations": []
        }
    });

    let mut by_guid = HashMap::new();
    by_guid.insert("content-guid".to_string(), root);

    let canvas_fetcher = MockCanvasFetcher { by_guid };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("container-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("physical"),
        manufacturer_id: Some("drak"),
        helper_name: Some("Screen_Left_Upper_RTT"),
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
        target_size: (200, 100),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let png = render_for_binding_ir(&inputs).expect("IR render should succeed");
    let img = image::load_from_memory(&png)
        .expect("png should decode")
        .to_rgba8();

    assert_eq!(img.dimensions(), (200, 100));

    let mut distinct = std::collections::HashSet::new();
    for y in (0..img.height()).step_by(4) {
        for x in (0..img.width()).step_by(4) {
            distinct.insert(img.get_pixel(x, y).0);
        }
    }
    assert!(
        distinct.len() > 1,
        "expected non-uniform image from IR renderer"
    );
}

#[test]
fn render_for_binding_matches_ir_entrypoint_for_bb_canvas() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.TestRoot",
        "_RecordValue_": {
            "size": {"x": 100.0, "y": 100.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "status_text",
                    "text": "READY",
                    "isActive": true,
                    "position": {"x": 10.0, "y": 20.0},
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 60.0},
                        "height": {"behavior": "Fixed", "value": 20.0}
                    }
                }
            ],
            "operations": []
        }
    });

    let mut by_guid = HashMap::new();
    by_guid.insert("content-guid".to_string(), root);

    let canvas_fetcher = MockCanvasFetcher { by_guid };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("container-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("physical"),
        manufacturer_id: Some("drak"),
        helper_name: Some("Screen_Left_Upper_RTT"),
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
        target_size: (200, 100),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let production = render_for_binding(&inputs).expect("production render should succeed");
    let ir_only = render_for_binding_ir(&inputs).expect("IR entrypoint should succeed");

    assert_eq!(production, ir_only, "production renderer should stay pinned to the IR path");
}

#[test]
fn render_for_binding_ir_honours_canvas_style_override() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.TestRoot",
        "_RecordValue_": {
            "style": "file://libs/foundry/records/ui/styles/s_bioc.json",
            "size": {"x": 16.0, "y": 16.0},
            "scene": [],
            "operations": []
        }
    });
    let canvas_style = serde_json::json!({
        "_RecordName_": "s_bioc",
        "_RecordValue_": {
            // BB_ColorStyle slots: the screen background reads slot 9
            // (Background), NOT slot 8 (Disabled) — plan P5.3, reference-
            // verified against the dark-room captures. Slot 8 here is a
            // DECOY: a regression back to slot-8 would render (99,99,99)
            // and fail this assertion.
            "colorStyles": [
                {"color": {"r": 64, "g": 200, "b": 255, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 99, "g": 99, "b": 99, "a": 255}},
                {"color": {"r": 5, "g": 10, "b": 20, "a": 255}}
            ]
        }
    });

    let mut by_guid = HashMap::new();
    by_guid.insert("content-guid".to_string(), root);
    by_guid.insert("style-guid".to_string(), canvas_style);

    let canvas_fetcher = MockCanvasFetcher { by_guid };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = MockStyleFetcher {
        style: ManufacturerStyle {
            name: "drake".to_string(),
            primary_tint: RgbaColor { r: 240, g: 168, b: 104, a: 255 },
            secondary_tint: None,
            colour_slots: vec![RgbaColor { r: 240, g: 168, b: 104, a: 255 }],
            background: RgbaColor { r: 48, g: 32, b: 16, a: 255 },
            backlight: RgbaColor { r: 0, g: 0, b: 0, a: 255 },
            font_family_hints: Vec::new(),
            crt: Default::default(),
        },
    };
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("container-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("physical"),
        manufacturer_id: Some("drak"),
        helper_name: Some("MedicalStyleOverride"),
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
        target_size: (16, 16),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    let png = render_for_binding_ir(&inputs).expect("IR render should succeed");
    let img = image::load_from_memory(&png)
        .expect("png should decode")
        .to_rgba8();

    assert_eq!(img.get_pixel(8, 8), &Rgba([5, 10, 20, 255]));
}
