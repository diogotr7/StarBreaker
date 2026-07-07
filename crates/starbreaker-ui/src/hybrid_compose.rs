//! Hybrid UI IR renderer that composes IR-backed BB content with SWF overlays.
//!
//! `render_ui_ir_with_swf_overlay` renders the IR document with Phase 5 Flash-wins
//! precedence, but ONLY for genuine full-stage SWF hosts: a `is_flash_renderer`
//! node whose entire BB subtree paints nothing (a pure placeholder — see
//! `is_full_stage_swf_host`). Such a node suppresses its (empty) BB subtree and
//! composites the SWF stage at its `computed_rect`. An ordinary
//! `rendererType:"Flash"` primitive node with real BB children keeps its BB
//! subtree and is never over-painted. Nodes without `is_flash_renderer` render
//! via the normal BB path unchanged.

use std::collections::{HashMap, HashSet};

use image::RgbaImage;
use tiny_skia::{Color, Rect as TskRect};

use crate::bb_atlas::AtlasLibrary;
use crate::compose::ComposeContext;
use crate::error::UiError;
use crate::ir_compose::render_ui_ir_document;
use crate::swf_render::draw_swf_stage_rgba_in_rect;
use crate::swf_render::state_select::compute_sample_data_export_ids;
use crate::ui_ir::UiIrDocument;

/// Render a UI IR document with SWF-wins / BB-fallback precedence (Phase 5).
///
/// For each genuine full-stage SWF host (see `is_full_stage_swf_host`): a
/// `is_flash_renderer` node whose entire BB subtree paints no BB content:
/// - The node and its entire BB subtree are removed from the BB render pass.
/// - The SWF stage (frame 0, with sample-data suppression) is composited at
///   the node's `computed_rect` over the BB result.
///
/// An ordinary `rendererType:"Flash"` primitive node with a drawable BB
/// descendant is NOT a host: its BB subtree is kept and no SWF is stamped.
///
/// `loc_fn` resolves `@key` loc strings in EditText fields encountered during
/// the SWF stage render.
pub fn render_ui_ir_with_swf_overlay(
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    loc_fn: &dyn Fn(&str) -> Option<String>,
) -> Result<RgbaImage, UiError> {
    let flash_ids: Vec<u32> = document
        .nodes
        .iter()
        .filter(|n| n.is_flash_renderer && is_full_stage_swf_host(document, n.id))
        .map(|n| n.id)
        .collect();

    if flash_ids.is_empty() {
        return render_ui_ir_document(document, ctx, atlas);
    }

    // Collect all node IDs in Flash subtrees (root + all recursive children).
    let suppressed_bb_ids = collect_subtree_ids(document, &flash_ids);

    // Render the BB document with Flash subtrees removed.
    let reduced = UiIrDocument {
        nodes: document
            .nodes
            .iter()
            .filter(|n| !suppressed_bb_ids.contains(&n.id))
            .cloned()
            .collect(),
        ..document.clone()
    };
    let mut image = render_ui_ir_document(&reduced, ctx, atlas)?;

    // Composite the SWF stage at each Flash node's computed_rect.
    let suppressed_swf = compute_sample_data_export_ids(ctx.assets);
    let tint = Color::WHITE;
    for &flash_id in &flash_ids {
        let Some(node) = document.nodes.iter().find(|n| n.id == flash_id) else {
            continue;
        };
        let r = &node.computed_rect;
        let Some(dest) = TskRect::from_xywh(r.x, r.y, r.w, r.h) else {
            continue;
        };
        draw_swf_stage_rgba_in_rect(&mut image, ctx.assets, dest, tint, 1.0, &suppressed_swf, loc_fn);
    }

    Ok(image)
}

/// Collect the IDs of all nodes reachable from `roots` via the children graph.
fn collect_subtree_ids(document: &UiIrDocument, roots: &[u32]) -> HashSet<u32> {
    let children_of: HashMap<u32, &[u32]> = document
        .nodes
        .iter()
        .map(|n| (n.id, n.children.as_slice()))
        .collect();

    let mut result = HashSet::new();
    let mut queue: std::collections::VecDeque<u32> = roots.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        if result.insert(id) {
            if let Some(children) = children_of.get(&id) {
                queue.extend(children.iter().copied());
            }
        }
    }
    result
}

/// A `rendererType:"Flash"` node is a genuine full-stage SWF host only when its
/// entire BB subtree (the node and all recursive children) paints nothing — a
/// pure placeholder whose visual is meant to come from the SWF stage. If any
/// node in the subtree paints real BB content, the Flash node is an ordinary
/// primitive renderer and must keep its BB subtree (the overlay must not fire on
/// it). Safe default for a populated subtree: NOT a host.
fn is_full_stage_swf_host(document: &UiIrDocument, root_id: u32) -> bool {
    let subtree = collect_subtree_ids(document, &[root_id]);
    !document
        .nodes
        .iter()
        .any(|n| subtree.contains(&n.id) && n.paints_bb_content())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::bb_atlas::AtlasLibrary;
    use crate::compose::ComposeContext;
    use crate::defaults::DefaultValueRegistry;
    use crate::style::ManufacturerStyle;
    use crate::swf_assets::SwfAssetLibrary;
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiIrDocument, UiRendererHint};

    struct EmptyFetcher;

    impl crate::bb_atlas::AssetFetcher for EmptyFetcher {
        fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
            None
        }
    }

    fn stub_style() -> ManufacturerStyle {
        // Real s_drak_hud palette via the provenance fixture (no hard-coded
        // colour values in test source — see test_palettes).
        crate::test_palettes::brand_style("s_drak_hud")
    }

    #[test]
    fn hybrid_rendering_does_not_require_separate_swf_assets_parameter() {
        // After the D5 fix, render_ui_ir_with_swf_overlay no longer calls
        // draw_swf_visual_exports_rgba. Per-node symbol drawing uses ctx.assets.
        // The function no longer requires a separate swf_assets argument.
        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "hybrid-guid".to_string(),
            canvas_name: Some("Hybrid".to_string()),
            target_width: 64,
            target_height: 64,
            selected_style_source: None,
            selected_swf_source: Some("test.swf".to_string()),
            renderer_hint: UiRendererHint::Hybrid,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: Vec::new(),
        };

        let fetcher = EmptyFetcher;
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = DefaultValueRegistry::with_well_known_path_defaults();
        let assets = SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, &|_| None)
            .expect("hybrid IR should render without requiring separate swf_assets");
    }
}