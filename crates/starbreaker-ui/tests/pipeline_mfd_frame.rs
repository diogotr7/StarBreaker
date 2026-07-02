//! TDD tests for Phase 6: MFD frame composition and footer injection.
//!
//! Tests:
//! 1. `mfd_frame_canvas_used_when_distinct_from_content` — mfd binding compiles from frame canvas
//! 2. `mfd_base_root_alpha_patched_to_one` — base_Root alpha=0.0 is patched to 1.0
//! 3. `mfd_screen_name_injected_into_text_ScreenName` — screen_name_loc_key resolves into node
//! 4. `mfd_non_mfd_binding_still_uses_content_canvas` — other bindings unaffected
//! 5. `mfd_frame_render_uses_content_when_same_guid` — no frame substitution when canvas_guid==content_guid
//! 6. `mfd_text_scaled_by_host_stage_cover_scale` — font sizes scale by the host
//!    Flash stage→target cover scale on the frame path
//! 7. `mfd_text_unscaled_without_host_stage` — no host movie → font sizes verbatim

use std::collections::HashMap;

use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
    compile_ir_for_binding,
};
use starbreaker_ui::bb_loc::LocFetcher;
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::ui_ir::UiIrTextPayload;

// ── Minimal fakes ──────────────────────────────────────────────────────────────

struct MapCanvasFetcher(HashMap<String, serde_json::Value>);

impl CanvasFetcher for MapCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        self.0
            .get(guid)
            .cloned()
            .ok_or_else(|| UiError::RenderError(format!("missing: {guid}")))
    }
}

struct NoSwf;
impl SwfFetcher for NoSwf {
    fn fetch_swf_bytes(&self, _: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("no swf".into()))
    }
}

struct NoStyle;
impl StyleFetcher for NoStyle {
    fn fetch_manufacturer_style(&self, _: &str) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Err(UiError::RenderError("no style".into()))
    }
}

struct NoAsset;
impl AssetFetcher for NoAsset {
    fn fetch_image_bytes(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

struct MapLocFetcher(HashMap<String, String>);
impl LocFetcher for MapLocFetcher {
    fn fetch_loc(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

// ── Canvas fixtures ────────────────────────────────────────────────────────────

fn frame_canvas() -> serde_json::Value {
    serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.FrameCanvas",
        "_RecordValue_": {
            "size": {"x": 1920.0, "y": 1080.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetCanvas",
                    "name": "base_Root",
                    "alpha": 0.0,
                    "isActive": true,
                    "inheritsAlpha": true,
                    // Page-in start-state: authored alpha 0.0 with a page-in
                    // animation block (matches m_eng_mfdcontent.base_Root). The
                    // structural rule settles this to 1.0 for static renders.
                    "animation": {"animationTimeline": null, "duration": 1.0, "additive": true},
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 1920.0},
                        "height": {"behavior": "Fixed", "value": 1080.0}
                    }
                },
                {
                    "_Pointer_": "ptr:2",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "text_ScreenName",
                    "parent": "_PointsTo_:ptr:1",
                    "isActive": true,
                    "text": "@ui_leaderboards_Loadout",
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 400.0},
                        "height": {"behavior": "Fixed", "value": 50.0}
                    }
                },
                {
                    "_Pointer_": "ptr:3",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "frame_chrome_label",
                    "parent": "_PointsTo_:ptr:1",
                    "isActive": true,
                    "text": "FRAME",
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 400.0},
                        "height": {"behavior": "Fixed", "value": 50.0}
                    }
                }
            ],
            "operations": []
        }
    })
}

fn content_canvas() -> serde_json::Value {
    serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.ContentCanvas",
        "_RecordValue_": {
            "size": {"x": 800.0, "y": 600.0},
            "scene": [
                {
                    "_Pointer_": "ptr:10",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "content_only_label",
                    "isActive": true,
                    "text": "CONTENT",
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 200.0},
                        "height": {"behavior": "Fixed", "value": 50.0}
                    }
                }
            ],
            "operations": []
        }
    })
}

fn mfd_fetcher() -> MapCanvasFetcher {
    let mut map = HashMap::new();
    map.insert("frame-guid".into(), frame_canvas());
    map.insert("content-guid".into(), content_canvas());
    MapCanvasFetcher(map)
}

fn mfd_binding<'a>(screen_name_key: Option<&'a str>) -> UiBindingView<'a> {
    UiBindingView {
        canvas_guid: Some("frame-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("TestHelper"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: screen_name_key,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    }
}

fn inputs_with_binding<'a>(
    binding: &'a UiBindingView<'a>,
    canvas_fetcher: &'a MapCanvasFetcher,
    loc_fetcher: Option<&'a dyn LocFetcher>,
) -> PipelineInputs<'a> {
    PipelineInputs {
        binding,
        canvas_fetcher,
        swf_fetcher: &NoSwf,
        style_fetcher: &NoStyle,
        asset_fetcher: &NoAsset,
        target_size: (200, 150),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher,
        derived_values: None,
        hologram_fetcher: None,
    }
}

// ── Test 1: Frame canvas used for mfd bindings with distinct GUIDs ─────────────

#[test]
fn mfd_frame_canvas_used_when_distinct_from_content() {
    let fetcher = mfd_fetcher();
    let binding = mfd_binding(None);
    let inputs = inputs_with_binding(&binding, &fetcher, None);

    let ir = compile_ir_for_binding(&inputs).expect("compile should succeed");

    // Frame canvas contains "text_ScreenName" and "frame_chrome_label"; content contains "content_only_label".
    // With frame render, IR should include frame nodes.
    let has_screen_name = ir.nodes.iter().any(|n| n.name == "text_ScreenName");
    let has_chrome = ir.nodes.iter().any(|n| n.name == "frame_chrome_label");
    assert!(
        has_screen_name,
        "IR should contain text_ScreenName from the frame canvas; nodes: {:?}",
        ir.nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
    );
    assert!(
        has_chrome,
        "IR should contain frame_chrome_label from the frame canvas"
    );
}

// ── Test 2: base_Root alpha=0.0 patched to 1.0 ────────────────────────────────

#[test]
fn mfd_base_root_alpha_patched_to_one() {
    let fetcher = mfd_fetcher();
    let binding = mfd_binding(None);
    let inputs = inputs_with_binding(&binding, &fetcher, None);

    let ir = compile_ir_for_binding(&inputs).expect("compile should succeed");

    let root_node = ir.nodes.iter().find(|n| n.name == "base_Root");
    assert!(root_node.is_some(), "IR should have base_Root node from frame canvas");
    let alpha = root_node.unwrap().alpha;
    assert_eq!(
        alpha, 1.0,
        "base_Root with alpha=0.0 must be patched to 1.0 in frame render; got {alpha}"
    );
}

// ── Test 3: text_ScreenName gets injected screen name ─────────────────────────

#[test]
fn mfd_screen_name_injected_into_text_screen_name() {
    let fetcher = mfd_fetcher();
    let binding = mfd_binding(Some("@ui_MFD_View_TargetStatus"));
    let mut loc_map = HashMap::new();
    loc_map.insert("ui_MFD_View_TargetStatus".to_string(), "TARGET STATUS".to_string());
    let loc = MapLocFetcher(loc_map);
    let inputs = inputs_with_binding(&binding, &fetcher, Some(&loc));

    let ir = compile_ir_for_binding(&inputs).expect("compile should succeed");

    let node = ir.nodes.iter().find(|n| n.name == "text_ScreenName")
        .expect("text_ScreenName node must be in IR");

    match &node.text_payload {
        Some(UiIrTextPayload::Resolved { text }) => {
            assert_eq!(
                text, "TARGET STATUS",
                "text_ScreenName must be resolved to the screen name"
            );
        }
        other => panic!(
            "expected Resolved{{text: TARGET STATUS}}, got {:?}", other
        ),
    }
}

// ── Test 4: Non-mfd bindings still use content canvas ─────────────────────────

#[test]
fn mfd_non_mfd_binding_still_uses_content_canvas() {
    let fetcher = mfd_fetcher();
    let binding = UiBindingView {
        canvas_guid: Some("frame-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("physical"),  // NOT "mfd"
        manufacturer_id: Some("drak"),
        helper_name: Some("TestHelper"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    };
    let inputs = inputs_with_binding(&binding, &fetcher, None);

    let ir = compile_ir_for_binding(&inputs).expect("compile should succeed");

    // Non-mfd bindings should use content_canvas_guid → shows content_only_label, not frame nodes
    let has_content = ir.nodes.iter().any(|n| n.name == "content_only_label");
    let has_frame = ir.nodes.iter().any(|n| n.name == "frame_chrome_label");
    assert!(
        has_content,
        "non-mfd binding must use content canvas (content_only_label expected)"
    );
    assert!(
        !has_frame,
        "non-mfd binding must NOT use frame canvas (frame_chrome_label should be absent)"
    );
}

// ── Test 5: Same canvas_guid and content_canvas_guid uses content directly ────

#[test]
fn mfd_frame_render_uses_content_when_same_guid() {
    let fetcher = mfd_fetcher();
    let binding = UiBindingView {
        canvas_guid: Some("content-guid"),       // same as content
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("TestHelper"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    };
    let inputs = inputs_with_binding(&binding, &fetcher, None);

    let ir = compile_ir_for_binding(&inputs).expect("compile should succeed");

    let has_content = ir.nodes.iter().any(|n| n.name == "content_only_label");
    let has_frame = ir.nodes.iter().any(|n| n.name == "frame_chrome_label");
    assert!(
        has_content,
        "same-GUID mfd binding must use content canvas (content_only_label expected)"
    );
    assert!(
        !has_frame,
        "same-GUID mfd binding must NOT render frame (frame_chrome_label should be absent)"
    );
}

// ── Tests 6/7: host Flash stage scales MFD text ───────────────────────────────

/// A minimal uncompressed SWF whose header RECT declares a 1280×720 px stage
/// (25600×14400 twips): `FWS` v6, RECT, 24 fps, 1 frame, End tag.
fn host_swf_1280x720() -> Vec<u8> {
    vec![
        b'F', b'W', b'S', 6, 23, 0, 0, 0, // header, file length 23
        0x80, 0x00, 0x03, 0x20, 0x00, 0x00, 0x01, 0xC2, 0x00, // RECT 0..25600, 0..14400
        0x00, 0x18, // frame rate 24.0
        0x01, 0x00, // frame count 1
        0x00, 0x00, // End tag
    ]
}

struct HostSwf;
impl SwfFetcher for HostSwf {
    fn fetch_swf_bytes(&self, path: &str) -> Result<Vec<u8>, UiError> {
        if path == "host.swf" {
            Ok(host_swf_1280x720())
        } else {
            Err(UiError::RenderError(format!("missing swf: {path}")))
        }
    }
}

/// A 4:3 frame canvas (like `M_MFD_Screen`'s 800×600) carrying a text node with
/// an authored design-unit `fontSize: 60`.
fn frame_canvas_4x3_with_text() -> serde_json::Value {
    serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.Frame4x3",
        "_RecordValue_": {
            "size": {"x": 800.0, "y": 600.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_DisplayWidget",
                    "name": "base_root",
                    "isActive": true,
                    "sizing": {
                        "width": {"behavior": "Percent", "value": 1.0},
                        "height": {"behavior": "Percent", "value": 1.0}
                    }
                },
                {
                    "_Pointer_": "ptr:2",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "text_StageScaled",
                    "parent": "_PointsTo_:ptr:1",
                    "isActive": true,
                    "text": "SCALED",
                    "fontSize": 60.0,
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 400.0},
                        "height": {"behavior": "Fixed", "value": 50.0}
                    }
                }
            ],
            "operations": []
        }
    })
}

fn stage_scale_fetcher() -> MapCanvasFetcher {
    let mut map = HashMap::new();
    map.insert("frame4x3-guid".into(), frame_canvas_4x3_with_text());
    map.insert("content-guid".into(), content_canvas());
    MapCanvasFetcher(map)
}

fn stage_scaled_font_size(host_swf_path: Option<&str>) -> f32 {
    let fetcher = stage_scale_fetcher();
    let binding = UiBindingView {
        canvas_guid: Some("frame4x3-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("TestHelper"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path,
        screen_aspect_w_over_h: None,
    };
    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: &fetcher,
        swf_fetcher: &HostSwf,
        style_fetcher: &NoStyle,
        asset_fetcher: &NoAsset,
        // Frame aspect (800×600 → 0.75) lifts the effective target to 1600×1200.
        target_size: (1600, 900),
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };
    let ir = compile_ir_for_binding(&inputs).expect("compile should succeed");
    let node = ir
        .nodes
        .iter()
        .find(|n| n.name == "text_StageScaled")
        .expect("text_StageScaled must be in IR");
    let style = node.text_style.as_ref().expect("text node must carry a text style");
    match &style.font_size {
        starbreaker_ui::ui_ir::UiIrValue::Fixed { value } => *value,
        other => panic!("expected fixed font size, got {other:?}"),
    }
}

/// The engine hosts MFD canvases in the binding's Flash movie (a 1280×720 GFx
/// stage) and renders that stage onto the screen RTT with NoBorder/cover
/// scaling. Textfield font sizes are stage-unit values, so the design size must
/// be multiplied by max(target_w/stage_w, target_h/stage_h):
/// 60 × max(1600/1280, 1200/720) = 60 × 5/3 = 100.
#[test]
fn mfd_text_scaled_by_host_stage_cover_scale() {
    // Fixture self-check: the minimal SWF must really declare a 1280×720 stage.
    let lib = starbreaker_ui::swf_assets::SwfAssetLibrary::new(host_swf_1280x720())
        .expect("host swf fixture must parse");
    assert_eq!(lib.stage_size(), (1280.0, 720.0), "host stage fixture");

    let size = stage_scaled_font_size(Some("host.swf"));
    assert!(
        (size - 100.0).abs() < 0.01,
        "frame-path text must scale by the host stage cover scale (60 × 5/3 = 100); got {size}"
    );
}

#[test]
fn mfd_text_unscaled_without_host_stage() {
    let size = stage_scaled_font_size(None);
    assert!(
        (size - 60.0).abs() < 0.01,
        "without a host movie the design font size renders verbatim; got {size}"
    );
}
