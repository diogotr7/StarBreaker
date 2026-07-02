//! TDD tests for Phase 5: SWF-wins / BB-fallback precedence wiring.
//!
//! Tests:
//! 1. `flash_widget_node_gets_is_flash_renderer` — IR compilation marks Flash nodes.
//! 2. `compute_sample_data_export_ids_identifies_correct_sprites` — state detection.
//! 3. `hybrid_render_composites_swf_for_flash_node` — SWF content appears in output.
//! 4. `hybrid_render_suppresses_bb_subtree_of_flash_node` — BB child not drawn.
//! 5. `non_flash_node_still_renders_bb` — non-Flash BB nodes render normally.

mod swf_helpers;

use starbreaker_ui::bb_atlas::AtlasLibrary;
use starbreaker_ui::canvas::RgbaColor;
use starbreaker_ui::compose::ComposeContext;
use starbreaker_ui::defaults::DefaultValueRegistry;
use starbreaker_ui::hybrid_compose::render_ui_ir_with_swf_overlay;
use starbreaker_ui::style::{CrtParams, ManufacturerStyle};
use starbreaker_ui::swf_assets::SwfAssetLibrary;
use starbreaker_ui::ui_ir::{
    UiIrDocument, UiIrNode, UiIrRect, UiIrValue, UiRendererHint, UI_IR_SCHEMA_VERSION,
};

// ── Shared helpers ─────────────────────────────────────────────────────────────

struct EmptyFetcher;
impl starbreaker_ui::bb_atlas::AssetFetcher for EmptyFetcher {
    fn fetch_image_bytes(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

fn black_style() -> ManufacturerStyle {
    ManufacturerStyle {
        name: "test".to_string(),
        primary_tint: RgbaColor { r: 255, g: 255, b: 255, a: 255 },
        secondary_tint: None,
        colour_slots: vec![],
        background: RgbaColor { r: 0, g: 0, b: 0, a: 255 },
        backlight: RgbaColor { r: 0, g: 0, b: 0, a: 255 },
        font_family_hints: vec![],
        crt: CrtParams::default(),
    }
}

fn minimal_node(
    id: u32,
    parent_id: Option<u32>,
    children: Vec<u32>,
    is_flash: bool,
    rect: (f32, f32, f32, f32),
    fill: Option<[f32; 4]>,
) -> UiIrNode {
    UiIrNode {
        id,
        parent_id,
        children,
        node_type: "display_widget".to_string(),
        name: format!("node_{id}"),
        is_active: true,
        layer: 0,
        alpha: 1.0,
        anchor: [0.5, 0.5],
        pivot: [0.5, 0.5],
        rotation_deg: None,
        authored_position: [0.0, 0.0],
        authored_size: [
            UiIrValue::Fixed { value: rect.2 },
            UiIrValue::Fixed { value: rect.3 },
        ],
        padding: [0.0; 4],
        margin: [0.0; 4],
        overflow_mode: None,
        clip_rect: None,
        computed_rect: UiIrRect { x: rect.0, y: rect.1, w: rect.2, h: rect.3 },
        background_fill_colour: fill,
        corner_radius: None,
        corner_radii: None,
        corner_chamfers: None,
        background_fill_alpha: None,
        background_fill_colour_token: None,
        circle_fill_colour_token: None,
        style_provenance: None,
        colour_overlay_enabled: false,
        segmented_fill: None,
        polygon: None,
        border: None,
        stroke_colour: None,
        stroke_colour_token: None,
        stroke_extent: None,
        colour_blend_mode: None,
        icon_tint_colour: None,
        icon_tint_colour_token: None,
        icon_preset: None,
        text_payload: None,
        secondary_text_payload: None,
        secondary_text_style: None,
        meter_progress: None,
        text_style: None,
        asset_ref: None,
        primitive_material: None,
        primitive_uv_start: None,
        primitive_uv_size: None,
        asset_layout: None,
        custom_shape: None,
        style_tag_uuids: vec![],
        resolved_style_tags: vec![],
        is_flash_renderer: is_flash,
        auto_font_size: false,
    }
}

fn make_document(
    width: u32,
    height: u32,
    swf_source: Option<&str>,
    nodes: Vec<UiIrNode>,
) -> UiIrDocument {
    UiIrDocument {
        schema_version: UI_IR_SCHEMA_VERSION,
        canvas_guid: "test-p5-guid".to_string(),
        canvas_name: Some("Phase5Test".to_string()),
        target_width: width,
        target_height: height,
        selected_style_source: None,
        selected_swf_source: swf_source.map(str::to_string),
        renderer_hint: UiRendererHint::Hybrid,
        confidence: 100,
        warnings: vec![],
        unresolved_references: vec![],
        resolved_asset_refs: vec![],
        missing_asset_refs: vec![],
        nodes,
    }
}

// ── Test 1: IR compilation marks Flash nodes ───────────────────────────────────

#[test]
fn flash_widget_node_gets_is_flash_renderer() {
    use starbreaker_ui::bb_scene::parse_bb_canvas;
    use starbreaker_ui::ui_ir::compile_ui_ir_from_scene;

    let canvas = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.FlashTest",
        "_RecordValue_": {
            "size": {"x": 100, "y": 100},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetCanvas",
                    "name": "canvas_FlashWidget",
                    "rendererType": "Flash",
                    "size": {
                        "width": {"behavior": "Fixed", "value": 100.0},
                        "height": {"behavior": "Fixed", "value": 100.0}
                    }
                },
                {
                    "_Pointer_": "ptr:2",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "text_NoTarget",
                    "parent": "_PointsTo_:ptr:1",
                    "text": "NO TARGET",
                    "size": {
                        "width": {"behavior": "Fixed", "value": 80.0},
                        "height": {"behavior": "Fixed", "value": 20.0}
                    }
                },
                {
                    "_Pointer_": "ptr:3",
                    "_Type_": "BuildingBlocks_WidgetCanvas",
                    "name": "canvas_Normal",
                    "size": {
                        "width": {"behavior": "Fixed", "value": 50.0},
                        "height": {"behavior": "Fixed", "value": 50.0}
                    }
                }
            ],
            "operations": []
        }
    });

    let scene = parse_bb_canvas(&canvas).expect("parse canvas");
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let ir = compile_ui_ir_from_scene(
        &scene,
        None,
        "test-guid",
        Some("FlashTest"),
        (100, 100),
        &defaults,
        None,
        Some("Data\\test.swf".to_string()),
        &[],
        vec![],
        vec![],
        100,
    );

    let flash_node = ir
        .nodes
        .iter()
        .find(|n| n.name == "canvas_FlashWidget")
        .expect("canvas_FlashWidget not found in IR");
    assert!(
        flash_node.is_flash_renderer,
        "canvas_FlashWidget must have is_flash_renderer=true"
    );

    let text_node = ir
        .nodes
        .iter()
        .find(|n| n.name == "text_NoTarget")
        .expect("text_NoTarget not found in IR");
    assert!(
        !text_node.is_flash_renderer,
        "text_NoTarget child must have is_flash_renderer=false"
    );

    let normal_node = ir
        .nodes
        .iter()
        .find(|n| n.name == "canvas_Normal")
        .expect("canvas_Normal not found in IR");
    assert!(
        !normal_node.is_flash_renderer,
        "canvas_Normal must have is_flash_renderer=false"
    );
}

// ── Test 2: Sample data detection ─────────────────────────────────────────────

#[test]
fn compute_sample_data_export_ids_identifies_correct_sprites() {
    use starbreaker_ui::swf_render::state_select::compute_sample_data_export_ids;

    let swf_bytes = swf_helpers::make_sample_data_swf();
    let lib = SwfAssetLibrary::new(swf_bytes).expect("SwfAssetLibrary::new");

    let suppressed = compute_sample_data_export_ids(&lib);

    let static_id = lib.lookup_export("StaticState").expect("StaticState");
    let sample_id = lib.lookup_export("SampleState").expect("SampleState");
    let always_id = lib.lookup_export("AlwaysVisible").expect("AlwaysVisible");

    assert!(
        !suppressed.contains(&static_id),
        "StaticState has @loc-key EditText — must NOT be suppressed"
    );
    assert!(
        suppressed.contains(&sample_id),
        "SampleState has sample-data EditText — must be suppressed"
    );
    assert!(
        !suppressed.contains(&always_id),
        "AlwaysVisible has no EditText — must NOT be suppressed"
    );
}

// ── Test 3: Hybrid render composites SWF content ──────────────────────────────

#[test]
fn hybrid_render_composites_swf_for_flash_node() {
    // SWF renders white "A" text on the stage → non-black pixels appear
    let swf_bytes = swf_helpers::make_edit_text_with_font_swf();
    let assets = SwfAssetLibrary::new(swf_bytes).expect("SwfAssetLibrary");
    let style = black_style();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets, hologram_fetcher: None };
    let atlas = AtlasLibrary::new(&EmptyFetcher, None);

    let document = make_document(
        100,
        100,
        Some("test.swf"),
        vec![minimal_node(1, None, vec![], true, (0.0, 0.0, 100.0, 100.0), None)],
    );

    let result = render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, &|_| None)
        .expect("render failed");

    assert_eq!(result.width(), 100);
    assert_eq!(result.height(), 100);
    let non_black = result
        .pixels()
        .filter(|p| p[0] > 10 || p[1] > 10 || p[2] > 10)
        .count();
    assert!(
        non_black > 0,
        "Hybrid render produced all-black image — SWF content not composited"
    );
}

// ── Test 4: BB subtree of Flash node is suppressed ────────────────────────────

#[test]
fn hybrid_render_suppresses_bb_subtree_of_flash_node() {
    // SWF has no drawable stage content; Flash node's BB child has red fill.
    // After hybrid render: no red pixels (BB child suppressed), only black background.
    let assets = SwfAssetLibrary::new(vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ])
    .expect("minimal swf");
    let style = black_style();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets, hologram_fetcher: None };
    let atlas = AtlasLibrary::new(&EmptyFetcher, None);

    // Flash parent (id=1), child with red fill (id=2)
    let red_fill = Some([1.0f32, 0.0, 0.0, 1.0]);
    let document = make_document(
        100,
        100,
        Some("test.swf"),
        vec![
            minimal_node(1, None, vec![2], true, (0.0, 0.0, 100.0, 100.0), None),
            minimal_node(2, Some(1), vec![], false, (0.0, 0.0, 100.0, 100.0), red_fill),
        ],
    );

    let result = render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, &|_| None)
        .expect("render failed");

    let red_pixels = result.pixels().filter(|p| p[0] > 200 && p[1] < 50 && p[2] < 50).count();
    assert_eq!(
        red_pixels,
        0,
        "BB child of Flash node must be suppressed — found {red_pixels} red pixels"
    );
}

// ── Test 5: Non-Flash nodes still render BB content ───────────────────────────

#[test]
fn non_flash_node_still_renders_bb() {
    // No Flash nodes; node with red fill renders normally via BB path.
    let assets = SwfAssetLibrary::new(vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ])
    .expect("minimal swf");
    let style = black_style();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets, hologram_fetcher: None };
    let atlas = AtlasLibrary::new(&EmptyFetcher, None);

    let red_fill = Some([1.0f32, 0.0, 0.0, 1.0]);
    let document = make_document(
        100,
        100,
        None,
        vec![minimal_node(1, None, vec![], false, (10.0, 10.0, 80.0, 80.0), red_fill)],
    );

    let result = render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, &|_| None)
        .expect("render failed");

    let red_pixels = result.pixels().filter(|p| p[0] > 200 && p[1] < 50 && p[2] < 50).count();
    assert!(
        red_pixels > 0,
        "Non-Flash BB node with red fill must render — found 0 red pixels"
    );
}
