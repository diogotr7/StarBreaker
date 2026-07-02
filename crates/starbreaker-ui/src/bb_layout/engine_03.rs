#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use image::{Rgba, RgbaImage};
#[allow(unused_imports)]
use log::warn;
#[allow(unused_imports)]
use crate::bb_scene::{BbCoordinateMethod, BbNode, BbNodeId, BbNodeType, BbScene, BbValue};

// Consolidated engine chunk 03: additional bb_layout tests that would push
// engine_02.part past the 3000-line cap (crates/starbreaker-ui/docs/ui-workflow.md rule 5).

#[cfg(test)]
mod materialised_entry_tests {
    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    /// Two materialised LIST ENTRIES (Auto height, flagged `_MaterialisedEntry_`)
    /// in a SpaceEvenly column content-fit their text and DISTRIBUTE down the
    /// column instead of overlaying at one anchor (Auto's fill fallback) — the
    /// countermeasure list shows one launcher panel per entry. Without the
    /// materialised-entry content-fit both clones fill the column and pile onto
    /// one spot.
    #[test]
    fn materialised_list_entries_distribute_not_overlap() {
        let pct = serde_json::json!({"value": 1.0, "behavior": "Percent"});
        let pair = |e: u32, t: u32, txt: &str| {
            [
                serde_json::json!({"_Pointer_": format!("ptr:{e}"), "_Type_": "BuildingBlocks_DisplayWidget",
                    "name": "base_CountermeasureEntry", "parent": "_PointsTo_:ptr:1", "isActive": true,
                    "_MaterialisedEntry_": true, "sizing": {"width": pct, "height": {"value": 1.0, "behavior": "Auto"}}}),
                serde_json::json!({"_Pointer_": format!("ptr:{t}"), "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "text", "parent": format!("_PointsTo_:ptr:{e}"), "isActive": true,
                    "_ResolvedText_": txt, "text": txt, "fontSize": {"value": 40.0, "behavior": "Fixed"},
                    "sizing": {"width": pct, "height": {"value": 0.0, "behavior": "Auto"}}}),
            ]
        };
        let mut scene = vec![serde_json::json!({"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetList",
            "name": "List_Countermeasures", "isActive": true, "sizing": {"width": pct, "height": pct},
            "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer", "direction": "Column", "axisJustification": "SpaceEvenly"}})];
        scene.extend(pair(2, 3, "48"));
        scene.extend(pair(4, 5, "5"));
        let canvas = serde_json::json!({"_RecordValue_": {"_Type_": "BuildingBlocks_Canvas",
            "size": {"x": 600.0, "y": 600.0}, "coordinateMethod": "useRaw", "operations": [], "scene": scene}});
        let result = layout(&parse_bb_canvas(&canvas).expect("fixture parses"), 600, 600);
        let (e0, e1) = (result.rects[&2], result.rects[&4]);
        assert!(e0.h < 300.0 && e1.h < 300.0, "entries content-fit, got {} / {}", e0.h, e1.h);
        let overlap = (e0.y.max(e1.y)) < (e0.y + e0.h).min(e1.y + e1.h);
        assert!(!overlap, "materialised entries must not overlap: {e0:?} vs {e1:?}");
    }

    /// A 16:9 `useRaw` canvas on a SQUARE target with `cover_fit` FILLS the target
    /// non-uniformly (`sx`/`sy`): a full-width/height `Percent` panel spans the
    /// whole square — NOT cover-overflowed (w>target, the old uniform-max path) nor
    /// contain-letterboxed (w<target). This is the cockpit countermeasure/ball
    /// screen fit (the 16:9 UI is stretched onto the square physical screen).
    #[test]
    fn useraw_cover_fit_fills_square_target_non_uniformly() {
        let pct = serde_json::json!({"value": 1.0, "behavior": "Percent"});
        let canvas = serde_json::json!({"_RecordValue_": {"_Type_": "BuildingBlocks_Canvas",
            "size": {"x": 1920.0, "y": 1080.0}, "coordinateMethod": "useRaw", "operations": [],
            "scene": [serde_json::json!({"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                "name": "panel", "isActive": true, "sizing": {"width": pct, "height": pct}})]}});
        let parsed = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout_with_animation_sample(&parsed, 1920, 1920, None, true, true);
        let panel = result.rects[&1];
        assert!(
            (panel.w - 1920.0).abs() < 1.0 && (panel.h - 1920.0).abs() < 1.0,
            "fill: full-bleed panel must span the whole square target (no overflow / no letterbox), got {panel:?}"
        );
    }
}
