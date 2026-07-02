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

// Consolidated engine chunk 02 (formerly: part_09.part, part_10.part, part_11.part, part_12.part, part_13.part, part_14.part, part_15.part).
//   part_12.part: Tests: a parent's explicit padding defines its content box and fixed-size
//   part_13.part: Scroll-bar thumb geometry: the at-rest scroll model.
//   part_14.part: Intrinsic text measurement for Auto-sized flex children.
//   part_15.part: Flex shrink policy tests (split from part_14 for the 500-line cap).

#[cfg(test)]
mod tests_b {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/canvas/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"))
    }

    /// A `WidgetImage` whose size was overridden to a tiny fixed `0.7×0.4`
    /// placeholder (an authoring artefact — the DRAK target greebles) must expand
    /// to fill its parent, not collapse to ~0.58px, even under an
    /// `aspectOverridesWidth` canvas (csx≠csy). Reproduces obs 3's collapsed
    /// `image_PageGreeblesNoTargetRight`.
    #[test]
    fn image_tiny_fixed_placeholder_fills_parent_under_aspect_override() {
        use crate::bb_scene::{
            BbCoordinateMethod, BbNode, BbNodeType, BbScene, BbSizing, BbTrbl, BbValue, Vec2, Vec3,
        };
        use std::collections::BTreeMap;
        let mk = |id: BbNodeId, ty: BbNodeType, w: BbValue, h: BbValue, children: Vec<BbNodeId>, parent: Option<BbNodeId>| BbNode {
            id, parent, children, ty, name: format!("n{id}"), style_tag_uuids: vec![],
            is_active: true, layer: 0, alpha: 1.0, position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing { width: w, height: h }, padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(), background: None, border: None, radial: None,
            text: None, icon: None, raw: serde_json::Value::Null,
        };
        let root = mk(1, BbNodeType::DisplayWidget, BbValue::Percent(1.0), BbValue::Percent(1.0), vec![2], None);
        let img = mk(2, BbNodeType::WidgetImage, BbValue::Fixed(0.7), BbValue::Fixed(0.4), vec![], Some(1));
        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, img);
        let scene = BbScene { coordinate_method: BbCoordinateMethod::AspectOverridesWidth, canvas_size: (1920.0, 1080.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 1600, 1200);
        let w = result.rects[&2].w;
        assert!(w > 100.0, "tiny 0.7-fixed placeholder image must fill parent, got w={w}");
    }

    /// R5.I-A: `PercentOfX` height (and `PercentOfY` width) must be evaluated
    /// against THIS NODE's OWN other-axis dimension, not the parent's other
    /// dimension.

    #[test]
    fn non_surface_child_canvas_root_preserves_authored_pivot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let host = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "menu_host_canvas".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(0.77) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child_root = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "menu_root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.0, y: 0.03 },
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, host);
        nodes.insert(2, child_root);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let host_rect = result.rects[&1];
        let root_rect = result.rects[&2];
        let expected_root_y = host_rect.y - root_rect.h * 0.03;
        assert!((host_rect.h - 77.0).abs() < 0.5, "expected host height 77, got {}", host_rect.h);
        assert!((root_rect.y - expected_root_y).abs() < 0.5, "expected child root y {}, got {}", expected_root_y, root_rect.y);
    }

    /// Mirror of above: `PercentOfY` for width must use own height.
    #[test]
    fn percent_of_y_uses_own_height_not_parent() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1, parent: None, children: vec![2],
            ty: BbNodeType::DisplayWidget, name: "parent".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(400.0), height: BbValue::Fixed(200.0) },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 2, parent: Some(1), children: vec![],
            ty: BbNodeType::WidgetIcon, name: "icon".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 1.0, behavior: "PercentOfY".into() },
                height: BbValue::Percent(0.6),
            },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (400.0, 200.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 400, 200);
        let r = result.rects[&2];
        // height = 0.6 × parent_h(200) = 120
        // width  = 1.0 × own_h(120)    = 120  (NOT 1.0 × parent_h(200) or × parent_w(400))
        assert!((r.h - 120.0).abs() < 0.5, "expected height ≈ 120, got {}", r.h);
        assert!((r.w - 120.0).abs() < 0.5, "expected width ≈ 120 (square), got {}", r.w);
    }

    #[test]
    fn flex_row_wrap_moves_full_width_child_to_next_line() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(200.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "Wrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "columnSpacing": 0,
                    "rowSpacing": 0
                }
            }),
        };
        let child1 = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "c1".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child2 = BbNode {
            id: 3,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "c2".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child1);
        nodes.insert(3, child2);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (300.0, 200.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 300, 200);
        let r1 = result.rects[&2];
        let r2 = result.rects[&3];
        assert!((r1.x - 0.0).abs() < 0.5 && (r1.y - 0.0).abs() < 0.5);
        assert!(
            (r2.x - 0.0).abs() < 0.5 && (r2.y - 50.0).abs() < 0.5,
            "second wrapped child expected at y=50, got ({:.1},{:.1})",
            r2.x,
            r2.y
        );
    }

    #[test]
    fn flex_row_wrap_start_applies_child_main_axis_anchor() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(100.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "Wrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "columnSpacing": 0,
                    "rowSpacing": 0
                }
            }),
        };
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: "anchored_prompt".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(0.3), height: BbValue::Fixed(40.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: 0.01, y: 0.0 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let mut metric = child.clone();
        metric.id = 3;
        metric.name = "right_metric".into();
        metric.sizing = BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(40.0) };
        metric.anchor = Vec2 { x: 1.0, y: 0.0 };
        metric.pivot = Vec2 { x: 1.0, y: 0.0 };
        assert_eq!(row_flex_start_anchor_offset(&metric, 300.0, 50.0, 1.0), None);

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (300.0, 100.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 300, 100);
        let rect = result.rects[&2];
        assert!((rect.x - 3.0).abs() < 0.5, "expected x≈3 from 1% row anchor, got {}", rect.x);
    }

    #[test]
    fn horizontal_filled_separator_uses_centerline_anchor() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let separator = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::Other("BuildingBlocks_WidgetSeparator".into()),
            name: "separator".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(0.5), height: BbValue::Fixed(16.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: 0.0, y: 0.18 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "direction": "Horizontal",
                "svgFill": {
                    "renderShape": true,
                    "svgPath": ""
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, separator);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (100.0, 100.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 100, 100);
        let rect = result.rects[&1];
        assert!((rect.y - 10.0).abs() < 0.5, "expected centerline y=18 minus half height 8, got {}", rect.y);
    }

    #[test]
    fn flex_row_auto_label_caption_pairs_share_available_width() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "text-layout".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(604.8), height: BbValue::Fixed(108.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.0, y: 0.5 },
            anchor: Vec2 { x: 0.65, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "NoWrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 30.0,
                    "rowSpacing": 0.0
                }
            }),
        };
        let child = |id: u32, name: &str| BbNode {
            id,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::Other("BuildingBlocks_ComponentLabelCaptionPair".into()),
            name: name.into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 64.0, behavior: "Auto".into() },
                height: BbValue::Other { value: 64.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child(2, "operator"));
        nodes.insert(3, child(3, "patient"));
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (604.8, 108.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 605, 108);
        let r1 = result.rects[&2];
        let r2 = result.rects[&3];
        assert!((r1.w - 287.4).abs() < 1.0, "expected first pair width ≈ 287.4, got {}", r1.w);
        assert!((r2.w - 287.4).abs() < 1.0, "expected second pair width ≈ 287.4, got {}", r2.w);
        assert!((r2.x - (r1.x + r1.w + 30.0)).abs() < 1.0, "expected second pair after 30px spacing, got x={}", r2.x);
    }
}

#[cfg(test)]
mod tests_c {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/canvas/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"))
    }

    /// R5.I-A: `PercentOfX` height (and `PercentOfY` width) must be evaluated
    /// against THIS NODE's OWN other-axis dimension, not the parent's other
    /// dimension.

    #[test]
    fn flex_row_right_anchored_label_caption_pairs_keep_intrinsic_width() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "text-layout".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(604.8), height: BbValue::Fixed(108.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 1.0, y: 0.5 },
            anchor: Vec2 { x: 1.0, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "NoWrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 30.0,
                    "rowSpacing": 0.0
                }
            }),
        };

        let child = |id: u32, name: &str| BbNode {
            id,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::Other("BuildingBlocks_ComponentLabelCaptionPair".into()),
            name: name.into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 64.0, behavior: "Auto".into() },
                height: BbValue::Other { value: 64.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 1.0, y: 0.0 },
            anchor: Vec2 { x: 1.0, y: 0.0 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child(2, "operator"));
        nodes.insert(3, child(3, "patient"));
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (604.8, 108.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 605, 108);
        let r1 = result.rects[&2];
        let r2 = result.rects[&3];
        assert!((r1.w - 64.0).abs() < 0.5, "expected first right-anchored pair intrinsic width ≈ 64, got {}", r1.w);
        assert!((r2.w - 64.0).abs() < 0.5, "expected second right-anchored pair intrinsic width ≈ 64, got {}", r2.w);
        assert!((r2.x - (r1.x + r1.w + 30.0)).abs() < 1.0, "expected second pair after 30px spacing, got x={}", r2.x);
    }

    #[test]
    fn flex_row_auto_intrinsic_components_stay_in_flow_for_end_alignment() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "text-layout".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(80.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "NoWrap",
                    "axisJustification": "End",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 8.0,
                    "rowSpacing": 0.0
                }
            }),
        };

        let left_button = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::ComponentGeneralButton,
            name: "Back".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 64.0, behavior: "Auto".into() },
                height: BbValue::Other { value: 64.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 1.0, y: 0.5 },
            anchor: Vec2 { x: 1.0, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let exit_bed = BbNode {
            id: 3,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::ComponentGeneralButtonSecondary,
            name: "ExitBed".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 64.0, behavior: "Auto".into() },
                height: BbValue::Other { value: 64.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 1.0, y: 0.5 },
            anchor: Vec2 { x: 1.0, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, left_button);
        nodes.insert(3, exit_bed);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 80.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 80);
        let left = result.rects[&2];
        let right = result.rects[&3];

        assert!((right.x + right.w - 200.0).abs() < 0.5, "expected right item to align to container end, got x={} w={}", right.x, right.w);
        assert!(left.x + left.w + 7.5 <= right.x, "expected distinct flowed items with spacing, got left={:?} right={:?}", left, right);
        assert!(left.x >= 0.0, "expected left item to stay within container, got x={}", left.x);
    }

    #[test]
    fn flex_row_space_between_spreads_children_to_container_edges() {
        // Authored `axisJustification: "SpaceBetween"` must distribute the free
        // space EQUALLY between items: first item flush to the start edge, last
        // item flush to the end edge, equal gaps in between (the power-screen
        // emissions header `base_EmissionsContainer` relies on this so its outer
        // separators reach the header edges). Counter-example: `Start` packs them
        // against the start edge, which is what an unhandled value falls back to.
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        fn item(id: BbNodeId, w: f32) -> BbNode {
            BbNode {
                id,
                parent: Some(1),
                children: vec![],
                ty: BbNodeType::DisplayWidget,
                name: format!("item{id}"),
                style_tag_uuids: vec![],
                is_active: true,
                layer: 0,
                alpha: 1.0,
                position: Vec3::default(),
                position_offset: Vec3::default(),
                sizing: BbSizing { width: BbValue::Fixed(w), height: BbValue::Fixed(80.0) },
                padding: BbTrbl::default(),
                margin: BbTrbl::default(),
                pivot: Vec2::default(),
                anchor: Vec2::default(),
                background: None,
                border: None,
                radial: None,
                text: None,
                icon: None,
                raw: serde_json::Value::Null,
            }
        }

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3, 4],
            ty: BbNodeType::DisplayWidget,
            name: "space-between-row".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(80.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "NoWrap",
                    "axisJustification": "SpaceBetween",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 0.0,
                    "rowSpacing": 0.0
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, item(2, 20.0));
        nodes.insert(3, item(3, 20.0));
        nodes.insert(4, item(4, 20.0));
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (300.0, 80.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 300, 80);
        let a = result.rects[&2];
        let b = result.rects[&3];
        let c = result.rects[&4];

        // free space = 300 - 60 = 240, split across 2 gaps = 120 each.
        assert!((a.x - 0.0).abs() < 0.5, "first item should be flush to start edge, got x={}", a.x);
        assert!((c.x + c.w - 300.0).abs() < 0.5, "last item should be flush to end edge, got x={} w={}", c.x, c.w);
        assert!((b.x - 140.0).abs() < 0.5, "middle item should sit at the equal-gap midpoint, got x={}", b.x);
    }

    #[test]
    fn flex_column_auto_textfields_keep_intrinsic_height_slots() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3, 4],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(1000.0), height: BbValue::Fixed(1000.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Column",
                    "wrap": "NoWrapInfinite",
                    "axisJustification": "Center",
                    "crossAxisJustification": "Center",
                    "itemAlignment": "Center",
                    "columnSpacing": 0.0,
                    "rowSpacing": 30.0
                }
            }),
        };

        let textfield = |id: u32, name: &str, style: &str, tags: Vec<&str>, anchor_x: f32| BbNode {
            id,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: name.into(),
            style_tag_uuids: tags.into_iter().map(String::from).collect(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Percent(0.7),
                height: BbValue::Other { value: 1.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: anchor_x, y: 0.0 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "labelProperties": {
                    "style": style
                }
            }),
        };

        let touch = BbNode {
            id: 4,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetCanvas,
            name: "TouchHere".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(400.0), height: BbValue::Fixed(400.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, textfield(2, "TitleText", "Title3", vec!["e6003a83-9795-4478-a61c-349f14016e5b"], 0.043));
        nodes.insert(3, textfield(3, "TouchPromptText", "Heading2", vec!["5e5c7c8f-847b-46c5-ad80-a57c941391ab"], 0.0));
        nodes.insert(4, touch);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (1000.0, 1000.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1000, 1000);
        let title = result.rects[&2];
        let prompt = result.rects[&3];
        let touch = result.rects[&4];

        assert!((title.y - 105.0).abs() < 0.5, "expected centered title flow y=105, got {}", title.y);
        assert!((title.x - 193.0).abs() < 0.5, "expected authored cross-axis anchor to offset title x, got {}", title.x);
        assert!((title.h - 270.0).abs() < 0.5, "expected Title3 intrinsic height 270, got {}", title.h);
        assert!((prompt.y - 390.0).abs() < 0.5, "expected prompt after title plus spacing, got {}", prompt.y);
        assert!((prompt.y - (title.y + title.h) - 15.4).abs() < 0.5, "expected derived title-to-prompt gap, got {}", prompt.y - (title.y + title.h));
        assert!((prompt.x - 150.0).abs() < 0.5, "expected prompt without cross-axis anchor offset at centered x, got {}", prompt.x);
        assert!((prompt.h - 60.0).abs() < 0.5, "expected prompt intrinsic height 60, got {}", prompt.h);
        assert!((touch.y - 466.0).abs() < 0.5, "expected touch canvas after intrinsic text slots, got {}", touch.y);
    }

    #[test]
    fn flex_column_nowrap_infinite_start_cross_axis_applies_child_anchor() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(1000.0), height: BbValue::Fixed(120.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Column",
                    "wrap": "NoWrapInfinite",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start",
                    "itemAlignment": "Start",
                    "columnSpacing": 0.0,
                    "rowSpacing": 0.0
                }
            }),
        };

        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: "WelcomeText".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Fixed(40.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2 { x: 0.01, y: 0.0 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (1000.0, 120.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1000, 120);
        let child = result.rects[&2];
        assert!((child.x - 10.0).abs() < 0.5, "expected child anchor.x to offset start cross-axis x, got {}", child.x);
    }

    #[test]
    fn inactive_bright_title3_auto_width_uses_authored_scale() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(1344.0), height: BbValue::Fixed(270.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let tier = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: "TierLevel".into(),
            style_tag_uuids: vec!["174b3e40-1b7b-4f01-a7dc-6420b7367d6b".into()],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 0.9, behavior: "Auto".into() },
                height: BbValue::Other { value: 1.0, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.01, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "affectsLayout": false,
                "labelProperties": {"style": "Title3"},
                "textAlignment": "Left"
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, tier);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (1344.0, 270.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1344, 270);
        let tier = result.rects[&2];

        assert!((tier.w - 145.8).abs() < 0.5, "expected authored Auto width scale to affect Title3 intrinsic width, got {}", tier.w);
        assert!((tier.x + tier.w * 0.5 - 13.44).abs() < 0.5, "expected anchor/pivot to stay attached to authored 1% anchor, got x={} w={}", tier.x, tier.w);
    }
}

#[cfg(test)]
mod tests_d {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/canvas/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"))
    }

    /// R5.I-A: `PercentOfX` height (and `PercentOfY` width) must be evaluated
    /// against THIS NODE's OWN other-axis dimension, not the parent's other
    /// dimension.

    // ── A1.6 — fixture-based layout tests ────────────────────────────────────

    /// MC_S_Target_Master: root DisplayWidget fills the canvas at Percent(1) sizing.
    #[test]
    fn mc_s_target_master_root_fills_canvas() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1600, 900);

        // Root rect should cover ≥99 % of target width (Percent(1.0) → 100 %).
        let root_id = *scene.roots.first().expect("at least one root");
        let root_rect = result.rects[&root_id];
        let ratio_w = root_rect.w / 1600.0;
        let ratio_h = root_rect.h / 900.0;
        assert!(
            (ratio_w - 1.0).abs() < 0.02,
            "root width ratio {ratio_w} not within 2 % of 1.0"
        );
        assert!(
            (ratio_h - 1.0).abs() < 0.02,
            "root height ratio {ratio_h} not within 2 % of 1.0"
        );
    }

    /// MC_S_Self_Master: 6 WidgetCanvas siblings all share the same parent
    /// inner rect (i.e. their x/y origins all lie inside the root's inner
    /// rect, modulo anchor offsets).
    #[test]
    fn mc_s_self_master_siblings_share_parent_inner() {
        let json = load_fixture("MC_S_Self_Master_680a71df.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1600, 900);

        // Find the root node.
        let root_id = *scene.roots.first().expect("root");
        let root_rect = result.rects[&root_id];

        // All WidgetCanvas siblings should have positive area.
        let widget_canvas_nodes: Vec<BbNodeId> = scene
            .nodes
            .values()
            .filter(|n| matches!(n.ty, BbNodeType::WidgetCanvas))
            .map(|n| n.id)
            .collect();

        assert!(
            widget_canvas_nodes.len() >= 6,
            "expected ≥6 WidgetCanvas nodes, got {}",
            widget_canvas_nodes.len()
        );

        for id in &widget_canvas_nodes {
            let rect = result.rects[id];
            assert!(rect.w > 0.0, "WidgetCanvas {id} has zero width");
            assert!(rect.h > 0.0, "WidgetCanvas {id} has zero height");
        }

        // The root rect should have positive area too.
        assert!(root_rect.w > 0.0 && root_rect.h > 0.0, "root has zero area");
    }

    /// BB_ScreenRadar: every WidgetCard's centre lies inside the canvas rect.
    #[test]
    fn bb_screen_radar_widget_cards_inside_canvas() {
        let json = load_fixture("BB_ScreenRadar_C_App_Starmap_68ff6d17.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1024, 1024);

        let canvas = result.canvas;
        // Expand the canvas slightly for floating-point tolerance.
        let expanded = Rect {
            x: canvas.x - 1.0,
            y: canvas.y - 1.0,
            w: canvas.w + 2.0,
            h: canvas.h + 2.0,
        };

        for (id, node) in &scene.nodes {
            if !matches!(node.ty, BbNodeType::WidgetCard) {
                continue;
            }
            let rect = result.rects[id];
            let (cx, cy) = rect.centre();
            assert!(
                expanded.contains_point(cx, cy),
                "WidgetCard {:?} centre ({cx:.1},{cy:.1}) lies outside canvas bounds",
                node.name,
            );
        }
    }

    /// EC_PowerManagement: 1 root node, rects has 1 entry, draw_order has
    /// length 1.
    #[test]
    fn ec_power_management_single_node() {
        let json = load_fixture("EC_PowerManagement_3228e5cc.json");
        let scene = parse_bb_canvas(&json).expect("parse");
        let result = layout(&scene, 1600, 900);

        assert_eq!(scene.roots.len(), 1, "expected 1 root");
        assert_eq!(result.rects.len(), 1, "expected 1 rect");
        assert_eq!(result.draw_order.len(), 1, "expected 1 draw_order entry");
    }

    /// Edge case: a `BbValue::Other` with an unknown behavior does not panic
    /// and produces a positive dimension.
    #[test]
    fn unknown_behavior_does_not_panic() {
        use crate::bb_scene::{BbNode, BbSizing, BbTrbl, BbValue, Vec2, Vec3};
        use crate::bb_scene::BbNodeType;

        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetCanvas,
            name: "test".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Other { value: 0.5, behavior: "Auto".into() },
                height: BbValue::Other { value: 0.5, behavior: "Auto".into() },
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (1920.0, 1080.0), roots: vec![1], nodes, operations: vec![] };

        // Must not panic.
        let result = layout(&scene, 1600, 900);
        let rect = result.rects[&1];
        // Fallback is fill → width = parent_inner.w
        assert!(rect.w > 0.0, "fallback width must be positive");
        assert!(rect.h > 0.0, "fallback height must be positive");
    }

    #[test]
    fn authored_scale_expands_rect_around_pivot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbScene, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "parent".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(100.0), height: BbValue::Fixed(100.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetCustomShape,
            name: "scaled_shape".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "scale": { "x": 1.2, "y": 1.4 }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (100.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 100, 100);
        let rect = result.rects[&2];

        assert!((rect.w - 60.0).abs() < 0.01, "expected scaled width 60, got {}", rect.w);
        assert!((rect.h - 70.0).abs() < 0.01, "expected scaled height 70, got {}", rect.h);
        assert!((rect.x - 20.0).abs() < 0.01, "expected pivot-centred x 20, got {}", rect.x);
        assert!((rect.y - 15.0).abs() < 0.01, "expected pivot-centred y 15, got {}", rect.y);
    }

    #[test]
    fn text_field_child_canvas_scale_does_not_expand_layout_slot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbScene, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(100.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let text = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![3],
            ty: BbNodeType::WidgetTextField,
            name: "prompt".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(60.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let backing_canvas = BbNode {
            id: 3,
            parent: Some(2),
            children: vec![],
            ty: BbNodeType::WidgetCanvas,
            name: "text_backing_canvas".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "scale": { "x": 1.0, "y": 1.5 },
                "sizingMethod": "Size"
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, text);
        nodes.insert(3, backing_canvas);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let rect = result.rects[&3];

        assert!((rect.w - 200.0).abs() < 0.01, "expected backing width to fill text slot, got {}", rect.w);
        assert!((rect.h - 60.0).abs() < 0.01, "expected backing height to stay in text slot, got {}", rect.h);
        assert!((rect.y - 0.0).abs() < 0.01, "expected backing to remain aligned to text slot, got {}", rect.y);
    }

    /// A3-PIVOT.3: when the canvas declares a 4:3 aspect (e.g. 800×600) and the
    /// target is 16:9 (1600×900), a uniform-scale letterbox is applied.  A root
    /// node with `Fixed(800)` sizing must NOT produce a rect of width 1600
    /// (the old non-uniform-stretch result); instead it should be scaled to 1200
    /// and horizontally centred with 200 px letterbox on each side.
    #[test]
    fn uniform_scale_letterboxes_mismatched_aspect() {
        use crate::bb_scene::{BbNode, BbSizing, BbTrbl, BbValue, Vec2, Vec3};
        use crate::bb_scene::BbNodeType;

        // Canvas 800×600, Fixed(800)×Fixed(600) root.
        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetCard,
            name: "root_card".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Fixed(800.0),
                height: BbValue::Fixed(600.0),
            },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (800.0, 600.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 1600, 900);
        let rect = result.rects[&1];

        // Uniform scale = min(1600/800, 900/600) = min(2.0, 1.5) = 1.5.
        // Fixed(800) → 800 × 1.5 = 1200.  NOT 1600 (old non-uniform stretch).
        assert_ne!(
            (rect.w, rect.h),
            (1600.0, 900.0),
            "Fixed(800×600) node must not be stretched to 1600×900"
        );
        assert!(
            (rect.w - 1200.0).abs() < 1.0,
            "expected width ≈ 1200 (uniform scale 1.5), got {:.1}",
            rect.w
        );
        assert!(
            (rect.h - 900.0).abs() < 1.0,
            "expected height ≈ 900 (uniform scale 1.5), got {:.1}",
            rect.h
        );
        // Letterbox: x offset = (1600 − 1200) / 2 = 200.
        assert!(
            (rect.x - 200.0).abs() < 1.0,
            "expected x ≈ 200 (letterbox), got {:.1}",
            rect.x
        );
    }

    #[test]
    fn layout_source_does_not_reintroduce_forbidden_hardcoded_or_heuristic_markers() {
        // Scan the REAL engine sources. (The pre-F1 version read `engine.inc`
        // — 3 include! lines — so the guard was silently vacuous.)
        let sources = [
            include_str!("engine_01.rs"),
            include_str!("engine_02.rs"),
            include_str!("engine_03.rs"),
        ];
        // Hard rule: keep layout generic across assets and screens. If this
        // trips, remove marker-based workarounds and fix structural causes.
        let forbidden = [
            ["med", "ical2"].concat(),
            ["med", "gel"].concat(),
            ["hard", "coded", "_offset"].concat(),
            ["magic", "_multiplier"].concat(),
            ["heu", "ristic", "_shift"].concat(),
            ["blend", "_factor"].concat(),
        ];

        for marker in forbidden {
            assert!(
                !sources.iter().any(|source| source.contains(marker.as_str())),
                "bb_layout hardcoding/heuristic marker reintroduced: {marker}"
            );
        }
    }

    /// A `FlexContainer` flows children by `layoutItemCommon.order` (CSS flex
    /// `order`), not scene/(layer,id) order. The MFD footer relies on this:
    /// children authored `[Prev(0), Next(4), Name(2)]` must lay out `Prev, Name,
    /// Next` so the nav carats sit at the bar's far ends.
    #[test]
    fn flex_row_orders_children_by_layout_item_order() {
        use crate::bb_scene::{
            BbCoordinateMethod, BbNode, BbNodeType, BbScene, BbSizing, BbTrbl, BbValue, Vec2, Vec3,
        };
        use std::collections::BTreeMap;
        let mk = |id: BbNodeId, order: i64| BbNode {
            id, parent: Some(1), children: vec![], ty: BbNodeType::WidgetCard,
            name: format!("c{order}"), style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(100.0), height: BbValue::Fixed(20.0) },
            padding: BbTrbl::default(), margin: BbTrbl::default(), pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::json!({ "layoutItemCommon": { "order": order } }),
        };
        let root = BbNode {
            id: 1, parent: None, children: vec![2, 3, 4], ty: BbNodeType::DisplayWidget,
            name: "flex".into(), style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(300.0), height: BbValue::Fixed(20.0) },
            padding: BbTrbl::default(), margin: BbTrbl::default(), pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::json!({ "layoutPolicy": {
                "_Type_": "BuildingBlocks_FlexContainer", "direction": "Row", "wrap": "NoWrap",
                "axisJustification": "Start", "crossAxisJustification": "Start",
                "itemAlignment": "Start", "columnSpacing": 0.0, "rowSpacing": 0.0 } }),
        };
        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, mk(2, 0)); // A, order 0
        nodes.insert(3, mk(3, 4)); // B, order 4
        nodes.insert(4, mk(4, 2)); // C, order 2
        let scene = BbScene { coordinate_method: BbCoordinateMethod::UseRaw, canvas_size: (300.0, 20.0), roots: vec![1], nodes, operations: vec![] };
        let result = layout(&scene, 300, 20);
        let (a, b, c) = (result.rects[&2], result.rects[&3], result.rects[&4]);
        assert!(a.x < c.x && c.x < b.x, "flow must be A(0), C(2), B(4); got A.x={} C.x={} B.x={}", a.x, c.x, b.x);
        assert!((c.x - 100.0).abs() < 1.0, "C (order 2) must sit in the middle ≈100, got {}", c.x);
    }
}

// Tests: a parent's explicit padding defines its content box and fixed-size
// children are fitted into it (the modular-kit ghost button's 64px icon
// instance inside the Root entry's 15px-padded 64px chrome box renders 34px,
// measured on the medical reference capture).
#[cfg(test)]
mod tests_padding_fit {
    #![allow(unused_imports, dead_code)]

    use super::*;

    fn node(
        id: BbNodeId,
        parent: Option<BbNodeId>,
        children: Vec<BbNodeId>,
        ty: crate::bb_scene::BbNodeType,
        w: crate::bb_scene::BbValue,
        h: crate::bb_scene::BbValue,
        padding: f32,
    ) -> crate::bb_scene::BbNode {
        use crate::bb_scene::{BbNode, BbSizing, BbTrbl, Vec2, Vec3};
        BbNode {
            id,
            parent,
            children,
            ty,
            name: format!("n{id}"),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: w, height: h },
            padding: BbTrbl { top: padding, right: padding, bottom: padding, left: padding },
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        }
    }

    /// A fixed-size child larger than its padded parent's content box is
    /// fitted into the content box (64px icon in a 64px box padded 15 → 34px).
    #[test]
    fn fixed_child_fits_into_padded_parent_content_box() {
        use crate::bb_scene::{BbNodeType, BbValue};

        let parent = node(
            1, None, vec![2],
            BbNodeType::DisplayWidget,
            BbValue::Fixed(64.0), BbValue::Fixed(64.0),
            15.0,
        );
        let child = node(
            2, Some(1), vec![],
            BbNodeType::WidgetIcon,
            BbValue::Fixed(64.0), BbValue::Fixed(64.0),
            0.0,
        );

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene {
            coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw,
            canvas_size: (64.0, 64.0),
            roots: vec![1],
            nodes,
            operations: vec![],
        };

        let result = layout(&scene, 64, 64);
        let r = result.rects[&2];
        assert!((r.x - 15.0).abs() < 0.5, "expected x ≈ 15, got {}", r.x);
        assert!((r.y - 15.0).abs() < 0.5, "expected y ≈ 15, got {}", r.y);
        assert!((r.w - 34.0).abs() < 0.5, "expected width ≈ 34, got {}", r.w);
        assert!((r.h - 34.0).abs() < 0.5, "expected height ≈ 34, got {}", r.h);
    }

    /// The same fit applies on the flex no-grow path: the button template's
    /// ComponentRoot is a flex container, and its fixed-size icon instance
    /// must fit the padded content box.
    #[test]
    fn fixed_flex_item_fits_into_padded_container_content_box() {
        use crate::bb_scene::{BbNodeType, BbValue};

        let mut parent = node(
            1, None, vec![2],
            BbNodeType::DisplayWidget,
            BbValue::Fixed(64.0), BbValue::Fixed(64.0),
            15.0,
        );
        parent.raw = serde_json::json!({
            "layoutPolicy": {"_Type_": "BuildingBlocks_LayoutPolicyFlexContainer", "direction": "Row"}
        });
        let child = node(
            2, Some(1), vec![],
            BbNodeType::WidgetIcon,
            BbValue::Fixed(64.0), BbValue::Fixed(64.0),
            0.0,
        );

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene {
            coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw,
            canvas_size: (64.0, 64.0),
            roots: vec![1],
            nodes,
            operations: vec![],
        };

        let result = layout(&scene, 64, 64);
        let r = result.rects[&2];
        assert!((r.w - 34.0).abs() < 0.5, "expected flex item width ≈ 34, got {}", r.w);
        assert!((r.h - 34.0).abs() < 0.5, "expected flex item height ≈ 34, got {}", r.h);
    }

    /// An unpadded parent keeps legacy behaviour: fixed children may overflow.
    #[test]
    fn fixed_child_overflows_unpadded_parent() {
        use crate::bb_scene::{BbNodeType, BbValue};

        let parent = node(
            1, None, vec![2],
            BbNodeType::DisplayWidget,
            BbValue::Fixed(40.0), BbValue::Fixed(40.0),
            0.0,
        );
        let child = node(
            2, Some(1), vec![],
            BbNodeType::WidgetIcon,
            BbValue::Fixed(64.0), BbValue::Fixed(64.0),
            0.0,
        );

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene {
            coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw,
            canvas_size: (40.0, 40.0),
            roots: vec![1],
            nodes,
            operations: vec![],
        };

        let result = layout(&scene, 40, 40);
        let r = result.rects[&2];
        assert!((r.w - 64.0).abs() < 0.5, "expected width ≈ 64, got {}", r.w);
        assert!((r.h - 64.0).abs() < 0.5, "expected height ≈ 64, got {}", r.h);
    }

    /// `ColumnReverse` flex stacks items from the container's BOTTOM upward
    /// (the power screen's `list_PowerBars` pip stack).
    #[test]
    fn column_reverse_flex_stacks_items_from_the_bottom() {
        use crate::bb_scene::{BbNodeType, BbValue};

        let mut parent = node(
            1, None, vec![2, 3],
            BbNodeType::DisplayWidget,
            BbValue::Fixed(100.0), BbValue::Fixed(300.0),
            0.0,
        );
        parent.raw = serde_json::json!({
            "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer", "direction": "ColumnReverse"}
        });
        let first = node(
            2, Some(1), vec![],
            BbNodeType::DisplayWidget,
            BbValue::Fixed(100.0), BbValue::Fixed(40.0),
            0.0,
        );
        let second = node(
            3, Some(1), vec![],
            BbNodeType::DisplayWidget,
            BbValue::Fixed(100.0), BbValue::Fixed(40.0),
            0.0,
        );

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, first);
        nodes.insert(3, second);
        let scene = BbScene {
            coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw,
            canvas_size: (100.0, 300.0),
            roots: vec![1],
            nodes,
            operations: vec![],
        };

        let result = layout(&scene, 100, 300);
        let r2 = result.rects[&2];
        let r3 = result.rects[&3];
        // First item at the bottom (300 - 40 = 260), second stacked above it.
        assert!((r2.y - 260.0).abs() < 0.5, "first item bottom-anchored, got y={}", r2.y);
        assert!((r3.y - 220.0).abs() < 0.5, "second item above the first, got y={}", r3.y);
    }

}

// Scroll-bar thumb geometry: the at-rest scroll model.
//
// `bb_resolve` pairs an expanded scrollbar standard's thumb widget with its
// component's `target` scroll-view node (`_ScrollThumbPair_` /
// `_ScrollViewPair_` raw markers, see scrollbar_expansion.part). After the
// main layout pass this post-pass reproduces what the engine's scrollbar
// component computes each frame from the scroll model:
//
//   ratio  = viewport extent / content extent   (along the thumb's axis)
//   thumb  = track rect scaled by ratio, anchored at scroll offset 0 (at rest)
//
// Content extent is measured from the laid-out rects of the view's ACTIVE
// descendants (a clipped flex flow lays items past the viewport edge). When
// nothing overflows (ratio >= 1) the engine hides the whole bar (`_Show`
// false); statically that is reproduced by zeroing the rects of the expanded
// `ComponentRoot` subtree (the thumb's parent), which the renderer skips.
//
// Formula provenance (plan P2.2b, 2026-06-12): the proportional model is
// corroborated by the engine's own root SWF — `gfx.controls.ScrollIndicator
// .updateThumb` in `BuildingBlocks_root.swf` AVM1 computes
// `thumb = max(10, pageSize / max(1, (maxPos - minPos) + pageSize) * track)`,
// which for pixel-valued positions (maxPos - minPos = content - viewport,
// pageSize = viewport) reduces to exactly viewport/content x track
// (`examples/swf_avm1_dump.rs --ops controls.ScrollIndicator`). The BB
// scrollbar standard itself binds the bar's SizeX to the engine-pushed
// `_SizeRatio` component parameter (scrollbarhorizontalcomponentstandard
// operations), so the live value is computed C++-side: the power-screen
// residual (ours 393px vs reference 431px on a 978.8px track, i.e. ratio
// 0.402 vs 0.440) is an engine-input difference, not a formula error —
// parked with P7 in crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md.

pub(crate) fn apply_scroll_thumb_rects(scene: &BbScene, rects: &mut BTreeMap<BbNodeId, Rect>) {
    let thumbs: Vec<(BbNodeId, String, bool)> = scene
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            let pair = node.raw.get("_ScrollThumbPair_")?.as_str()?.to_owned();
            let horizontal = node
                .raw
                .get("_ScrollThumbAxis_")
                .and_then(|v| v.as_str())
                .is_none_or(|axis| axis.eq_ignore_ascii_case("x"));
            Some((*id, pair, horizontal))
        })
        .collect();

    for (thumb_id, pair, horizontal) in thumbs {
        let Some((view_id, _)) = scene.nodes.iter().find(|(_, node)| {
            node.raw
                .get("_ScrollViewPair_")
                .and_then(|v| v.as_str())
                == Some(pair.as_str())
        }) else {
            continue;
        };
        let Some(view_rect) = rects.get(view_id).copied() else {
            continue;
        };
        let viewport = if horizontal { view_rect.w } else { view_rect.h };
        if viewport <= 0.0 {
            continue;
        }
        let content_end = active_descendant_max_extent(scene, rects, *view_id, horizontal);
        let content = if horizontal {
            content_end - view_rect.x
        } else {
            content_end - view_rect.y
        };

        let ratio = if content > viewport { viewport / content } else { 1.0 };
        let Some(thumb) = scene.nodes.get(&thumb_id) else { continue };
        if ratio >= 1.0 {
            // Nothing overflows: the engine drops `_Show` and the bar
            // disappears. Zero the expanded component subtree's rects.
            let bar_root = thumb.parent.unwrap_or(thumb_id);
            zero_subtree_rects(scene, rects, bar_root);
            continue;
        }
        let Some(track) = thumb.parent.and_then(|pid| rects.get(&pid)).copied() else {
            continue;
        };
        if let Some(rect) = rects.get_mut(&thumb_id) {
            if horizontal {
                rect.x = track.x;
                rect.w = track.w * ratio;
            } else {
                rect.y = track.y;
                rect.h = track.h * ratio;
            }
        }
    }
}

/// Largest active-descendant end edge (`x+w` or `y+h`) under `root`,
/// starting from the root's own rect end so an empty view yields ratio 1.
fn active_descendant_max_extent(
    scene: &BbScene,
    rects: &BTreeMap<BbNodeId, Rect>,
    root: BbNodeId,
    horizontal: bool,
) -> f32 {
    let end = |rect: &Rect| if horizontal { rect.x + rect.w } else { rect.y + rect.h };
    let mut max_end = rects.get(&root).map(|r| end(r)).unwrap_or(0.0);
    let mut stack: Vec<BbNodeId> = scene
        .nodes
        .get(&root)
        .map(|n| n.children.clone())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(node) = scene.nodes.get(&id) else { continue };
        if !node.is_active {
            continue;
        }
        if let Some(rect) = rects.get(&id) {
            max_end = max_end.max(end(rect));
        }
        stack.extend(node.children.iter().copied());
    }
    max_end
}

fn zero_subtree_rects(scene: &BbScene, rects: &mut BTreeMap<BbNodeId, Rect>, root: BbNodeId) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(rect) = rects.get_mut(&id) {
            rect.w = 0.0;
            rect.h = 0.0;
        }
        if let Some(node) = scene.nodes.get(&id) {
            stack.extend(node.children.iter().copied());
        }
    }
}

#[cfg(test)]
mod scroll_thumb_tests {
    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    /// root > view (Clip, 100px) > list > 7 items of 47px; sibling track >
    /// thumb. The thumb must shrink to viewport/content of the track and
    /// stay at the track's start.
    fn scroll_scene(item_count: usize) -> BbScene {
        let mut scene_nodes = vec![
            serde_json::json!({"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                "name": "root",
                "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                           "height": {"value": 1.0, "behavior": "Percent"}}}),
            serde_json::json!({"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                "name": "view", "parent": "_PointsTo_:ptr:1",
                "overflow": {"overflow": "Clip"},
                "_ScrollViewPair_": "sp-test",
                "sizing": {"width": {"value": 100.0, "behavior": "Fixed"},
                           "height": {"value": 50.0, "behavior": "Fixed"}}}),
            serde_json::json!({"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetList",
                "name": "list", "parent": "_PointsTo_:ptr:2",
                "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer", "direction": "Row",
                                  "axisJustification": "Start"},
                "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                           "height": {"value": 1.0, "behavior": "Percent"}}}),
            serde_json::json!({"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                "name": "track", "parent": "_PointsTo_:ptr:1",
                "sizing": {"width": {"value": 100.0, "behavior": "Fixed"},
                           "height": {"value": 4.0, "behavior": "Fixed"}}}),
            serde_json::json!({"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_DisplayWidget",
                "name": "thumb", "parent": "_PointsTo_:ptr:4",
                "_ScrollThumbPair_": "sp-test", "_ScrollThumbAxis_": "x",
                "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                           "height": {"value": 1.0, "behavior": "Percent"}}}),
        ];
        for i in 0..item_count {
            scene_nodes.push(serde_json::json!({
                "_Pointer_": format!("ptr:{}", 10 + i),
                "_Type_": "BuildingBlocks_DisplayWidget",
                "name": format!("item{i}"), "parent": "_PointsTo_:ptr:3",
                "sizing": {"width": {"value": 47.0, "behavior": "Fixed"},
                           "height": {"value": 1.0, "behavior": "Percent"}}}));
        }
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 200.0, "y": 50.0},
                "coordinateMethod": "useRaw",
                "scene": scene_nodes,
                "operations": []
            }
        });
        parse_bb_canvas(&canvas).expect("scroll fixture should parse")
    }

    #[test]
    fn overflowing_content_shrinks_thumb_to_viewport_ratio() {
        let scene = scroll_scene(7);
        let result = layout(&scene, 200, 50);
        let track = result.rects[&4];
        let thumb = result.rects[&5];
        let expected = track.w * (100.0 / (7.0 * 47.0));
        assert!(
            (thumb.w - expected).abs() < 0.5,
            "thumb width {} should be viewport/content of the track ({expected})",
            thumb.w
        );
        assert!((thumb.x - track.x).abs() < 0.01, "at rest the thumb sits at the track start");
    }

    #[test]
    fn non_overflowing_content_hides_the_bar() {
        let scene = scroll_scene(2);
        let result = layout(&scene, 200, 50);
        let track = result.rects[&4];
        let thumb = result.rects[&5];
        assert_eq!((track.w, track.h), (0.0, 0.0), "track must collapse when nothing overflows");
        assert_eq!((thumb.w, thumb.h), (0.0, 0.0), "thumb must collapse when nothing overflows");
    }
}

// Intrinsic text measurement for Auto-sized flex children.
//
// The no-grow flex flow treats Auto main-axis children as zero-sized (no
// content measurement), which collapses text-backed value groups like the
// OUTPUT card's "2" + "/ 16" pair (Auto containers in a Center-justified
// row) onto one spot. `auto_text_intrinsic_main` measures the subtree's
// RESOLVED text (bb_resolve writes `raw["_ResolvedText_"]` for active text
// fields) with the shared TTF metrics, so such children flow at their
// natural width/height like the engine lays them out.

/// Best-effort intrinsic main-axis size of an Auto-sized flex child whose
/// subtree carries resolved text. `None` when no measurable text exists
/// (the caller keeps the zero-size rule).
pub(crate) fn auto_text_intrinsic_main(
    node_id: BbNodeId,
    scene: &BbScene,
    canvas_scale: f32,
    is_row: bool,
) -> Option<f32> {
    let renderer = crate::text::TextRenderer::new();
    let mut best: Option<f32> = None;
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        let Some(node) = scene.nodes.get(&id) else { continue };
        if !node.is_active {
            continue;
        }
        stack.extend(node.children.iter().copied());
        let Some((w, h)) = node_resolved_text_size(node, canvas_scale, &renderer) else {
            continue;
        };
        let main = if is_row { w } else { h };
        best = Some(best.map_or(main, |b: f32| b.max(main)));
    }
    best
}

/// Measure ONE node's resolved text (`raw["_ResolvedText_"]`) at the size the
/// renderer will draw it with: the EFFECTIVE (styled) size annotated by ui_ir
/// before layout, else the authored font size — both at the draw calibration.
/// `None` when the node carries no measurable text.
pub(crate) fn node_resolved_text_size(
    node: &crate::bb_scene::BbNode,
    canvas_scale: f32,
    renderer: &crate::text::TextRenderer,
) -> Option<(f32, f32)> {
    let text_value = node.raw.get("_ResolvedText_").and_then(|v| v.as_str())?;
    let text = (!text_value.trim().is_empty()).then_some(text_value)?;
    // Draw-metric annotations win: ui_ir's pre-layout pass measures the text
    // through the SAME glyph machinery the renderer will draw with (SWF font
    // advances at the IR font size — no TTF estimate, no calibration), so the
    // intrinsic box hugs the painted glyphs like the engine's does.
    let draw_w = node
        .raw
        .get("_DrawTextWidthPx_")
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0);
    let draw_h = node
        .raw
        .get("_DrawTextHeightPx_")
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0);
    if let (Some(w), Some(h)) = (draw_w, draw_h) {
        return Some((w as f32, h as f32));
    }
    let size_px = if let Some(effective) = node
        .raw
        .get("_EffectiveFontPx_")
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0)
    {
        // Measure == draw: the TTF estimate follows the same data-backed em
        // model as the renderer (IR font size = design-em px; plan P3.2
        // retired the tuned 1.5 calibration pair on both sides together).
        effective as f32
    } else {
        match node.text.as_ref().map(|t| &t.font_size) {
            Some(BbValue::Fixed(size)) if *size > 0.0 => {
                *size * canvas_scale
            }
            _ => return None,
        }
    };
    Some(renderer.measure(text, crate::text::FontKind::Mono, size_px))
}

/// Standard flex SHRINK: overflowing no-grow children scale down
/// proportionally to fit (CSS flex-shrink; the battery card's column authors
/// 0.9 + 0.5 Auto fractions overflowing a 227px container). Exemptions where
/// the engine lets content overflow instead:
/// - scrolling containers (the power pip columns),
/// - wrap-enabled rows,
/// - flows with any non-flex-managed child — sizing-method "None" resolves
///   via the fill fallback and overflows by design (the medical footer's
///   TextLayout row keeps both full-width label-caption pairs).
///
/// Returns the (possibly reduced) total main extent including spacing.
pub(crate) fn apply_flex_no_grow_shrink(
    sizes: &mut [(BbNodeId, f32, f32, bool)],
    scene: &BbScene,
    container: Rect,
    is_row: bool,
    item_spacing: f32,
    total_main: f32,
    container_scrollable: bool,
    wrap_enabled: bool,
) -> f32 {
    let shrink_avail = if is_row { container.w } else { container.h };
    if container_scrollable || wrap_enabled || shrink_avail <= 0.0 || total_main <= shrink_avail {
        return total_main;
    }
    let is_shrinkable = |child_id: &BbNodeId| {
        scene.nodes.get(child_id).is_some_and(|node| {
            !flex_shrink_disabled(node)
                && (main_axis_flex_managed(node, is_row)
                    || zero_auto_text_backed(*child_id, node, scene, is_row))
        })
    };
    let spacing_total = item_spacing * (sizes.len().saturating_sub(1) as f32);
    // Split the row into flex-managed (shrinkable) and intrinsic
    // (non-shrinkable, e.g. an Auto-width title) children. The intrinsic ones
    // keep their measured size; the shrinkable ones (a card's icon + separator)
    // compress to fit the space the title leaves — this squashes the battery
    // icon so the longer BATTERY title fits the fixed-width header, matching the
    // engine. When every child is shrinkable this reduces to the old uniform
    // scale (intrinsic_main = 0); when none is (the medical footer's None-
    // authored pairs arrive as Auto 64 LIVE) the row is left untouched.
    let mut shrinkable_main = 0.0f32;
    let mut intrinsic_main = 0.0f32;
    for (child_id, w, h, _) in sizes.iter() {
        let main = if is_row { *w } else { *h };
        if is_shrinkable(child_id) {
            shrinkable_main += main;
        } else {
            intrinsic_main += main;
        }
    }
    if shrinkable_main <= 0.0 {
        return total_main;
    }
    let target_shrinkable = (shrink_avail - spacing_total - intrinsic_main).max(0.0);
    if target_shrinkable >= shrinkable_main {
        return total_main;
    }
    let scale = target_shrinkable / shrinkable_main;
    if std::env::var("BB_SHRINK_PROBE").as_deref() == Ok("1") {
        for (child_id, w, h, _) in sizes.iter() {
            let node = scene.nodes.get(child_id);
            eprintln!(
                "BB_SHRINK_PROBE: scale={scale} is_row={is_row} child={child_id} name={:?} ty={:?} shrinkable={} main_sizing={:?} w={w} h={h}",
                node.map(|n| n.name.as_str()),
                node.map(|n| &n.ty),
                is_shrinkable(child_id),
                node.map(|n| if is_row { &n.sizing.width } else { &n.sizing.height }),
            );
        }
    }
    for (child_id, w, h, _) in sizes.iter_mut() {
        if is_shrinkable(child_id) {
            if is_row {
                *w *= scale;
            } else {
                *h *= scale;
            }
        }
    }
    sizes
        .iter()
        .map(|(_, w, h, _)| if is_row { *w } else { *h })
        .sum::<f32>()
        + spacing_total
}

/// Auto ZERO-hint (value 0.0) children whose main size came from the text
/// intrinsic are CONTENT-SIZED flex items and shrink with their flow (the
/// emissions Numbers Container's emitted/ambient pair otherwise overflows
/// its 141px band at nominal-font measure). Zero-hint children WITHOUT
/// measurable text keep the zero-size rule and stay outside the shrink set.
/// Existence of an intrinsic is scale-independent, so the probe scale is 1.
fn zero_auto_text_backed(
    node_id: BbNodeId,
    node: &crate::bb_scene::BbNode,
    scene: &BbScene,
    is_row: bool,
) -> bool {
    let main = if is_row { &node.sizing.width } else { &node.sizing.height };
    let zero_auto = matches!(
        main,
        BbValue::Other { value, behavior } if behavior == "Auto" && *value == 0.0
    );
    zero_auto && auto_text_intrinsic_main(node_id, scene, 1.0, is_row).is_some()
}

/// A flex item with an explicit `shrinkProportion` of 0 must NOT shrink
/// (CSS flex-shrink: 0): it keeps its base size and the row/column overflows.
/// The g-force / velocity ball authors its square `card_BallArea` (PercentOfY
/// width) and the overflowing `card_Readouts` both at shrinkProportion 0, so
/// the readouts crop while the ball stays square. Absent or non-zero
/// shrinkProportion keeps the existing (shrinkable) behaviour — this only
/// removes a node from the shrink set when the author explicitly froze it.
fn flex_shrink_disabled(node: &crate::bb_scene::BbNode) -> bool {
    node.raw
        .get("layoutPolicyItem")
        .and_then(|item| item.get("shrinkProportion"))
        .and_then(|value| value.as_f64())
        .is_some_and(|proportion| proportion == 0.0)
}

/// A child participates in flex main-axis sizing when its authored sizing is
/// an extent the layout engine actually distributes: Fixed, Percent,
/// normalized Auto fractions (0,1] and cross-axis percent (mirroring
/// `resolve_value`'s taxonomy). Auto content hints (value > 1) and unknown
/// behaviors ("None" in authored data) resolve via the fill fallback and are
/// not flex-managed.
fn main_axis_flex_managed(node: &crate::bb_scene::BbNode, is_row: bool) -> bool {
    let main = if is_row { &node.sizing.width } else { &node.sizing.height };
    match main {
        BbValue::Fixed(_) | BbValue::Percent(_) => true,
        BbValue::Other { value, behavior } => {
            (behavior == "Auto" && *value > 0.0 && *value <= 1.0)
                || (is_row && behavior == "PercentOfY")
                || (!is_row && behavior == "PercentOfX")
        }
    }
}

#[cfg(test)]
mod auto_text_flex_tests {
    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    /// The TTF text-measure fallback follows the SAME data-backed em model
    /// as the SWF path (plan P3.2): the IR font size IS the design-em pixel
    /// size (ascent+|descent| = size px; rusttype's Scale already normalises
    /// to that span), so a 30px Auto text child measures ~30px tall — with NO
    /// tuned multiplier. The deleted `LAYOUT_TEXT_MEASURE_CALIBRATION = 1.5`
    /// inflated this to 45px; it was calibrated when DejaVu stood in for game
    /// fonts on live screens, a case that no longer exists (the shared
    /// fontlib merges into every binding's assets, and live layout always
    /// uses SWF draw-metric annotations).
    #[test]
    fn auto_text_intrinsic_height_is_the_nominal_em_size() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 400.0, "y": 300.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "column", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Column", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "label", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "OUTPUT",
                     "text": "OUTPUT",
                     "fontSize": {"value": 30.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.0, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 400, 300);
        let label = result.rects[&2];
        assert!(
            (label.h - 30.0).abs() <= 1.0,
            "30px text measures ~30px tall under the em model (no 1.5 inflation), got {}",
            label.h
        );
    }

    /// Two Auto-width text containers in a Center-justified row flow at
    /// their measured widths side by side (the OUTPUT card's "2" "/ 16").
    #[test]
    fn auto_text_children_flow_at_measured_widths() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 400.0, "y": 100.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "row", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Center"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "current", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "2",
                     "text": "2",
                     "fontSize": {"value": 20.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Auto"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "total", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "/ 16",
                     "text": "/ 16",
                     "fontSize": {"value": 20.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Auto"},
                                "height": {"value": 1.0, "behavior": "Percent"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 400, 100);
        let current = result.rects[&2];
        let total = result.rects[&3];
        assert!(current.w > 5.0, "current text gets a measured width, got {}", current.w);
        assert!(total.w > current.w, "the longer text is wider, got {} vs {}", total.w, current.w);
        assert!(
            (total.x - (current.x + current.w)).abs() < 0.6,
            "the pair flows adjacently: total.x {} after current end {}",
            total.x,
            current.x + current.w
        );
        // Center justification: the group is centred in the 400px row.
        let group_left = current.x;
        let group_right = total.x + total.w;
        let centre_offset = ((group_left + group_right) / 2.0 - 200.0).abs();
        assert!(centre_offset < 1.0, "group centred, offset {centre_offset}");
    }

    /// An OVERLAY (non-flex) Auto-hint textfield with resolved text sizes to
    /// its measured text, honouring its authored anchor/pivot — the heat
    /// gauge's `CelsiusSymbol` (anchor y 1.02, pivot y 1.0, Auto 64) hangs
    /// just below the gauge bottom; the fill fallback used to stretch it over
    /// the whole gauge, painting "ºC" mid-bar.
    #[test]
    fn overlay_auto_text_field_sizes_to_text_and_anchors_below() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 58.0, "y": 296.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "gauge_root", "isActive": true,
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "celsius", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "anchor": {"x": 0.5, "y": 1.02, "z": 0.0},
                     "pivot": {"x": 0.5, "y": 1.0, "z": 0.0},
                     "_ResolvedText_": "ºC",
                     "text": "ºC",
                     "fontSize": {"value": 18.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 64.0, "behavior": "Auto"},
                                "height": {"value": 64.0, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 58, 296);
        let celsius = result.rects[&2];
        assert!(
            celsius.h < 100.0 && celsius.h > 4.0,
            "text-sized height, got {}",
            celsius.h
        );
        let bottom = celsius.y + celsius.h;
        assert!(
            (bottom - 1.02 * 296.0).abs() < 1.5,
            "bottom anchored at 1.02 of the gauge, got {bottom}"
        );
    }

    /// A text-backed Auto child in a MIXED row (the battery card's header:
    /// fixed icon + fixed separator + Auto title card) flows after its
    /// siblings at measured width instead of filling the container from the
    /// origin (which paints BATTERY over the icon).
    #[test]
    fn mixed_row_auto_text_child_flows_after_siblings() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 339.0, "y": 114.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "header", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Start",
                                      "columnSpacing": 5.0},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCard",
                     "name": "icon_card", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 80.0, "behavior": "Fixed"},
                                "height": {"value": 80.0, "behavior": "Fixed"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCard",
                     "name": "separator_card", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 34.0, "behavior": "Fixed"},
                                "height": {"value": 68.0, "behavior": "Fixed"}}},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_WidgetCard",
                     "name": "title_card", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 32.0, "behavior": "Auto"},
                                "height": {"value": 32.0, "behavior": "Auto"}}},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "title_text", "parent": "_PointsTo_:ptr:4", "isActive": true,
                     "_ResolvedText_": "BATTERY",
                     "text": "BATTERY",
                     "fontSize": {"value": 20.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 339, 114);
        let icon = result.rects[&2];
        let sep = result.rects[&3];
        let title = result.rects[&4];
        assert!(
            (sep.x - (icon.x + icon.w + 5.0)).abs() < 0.6,
            "separator flows after the icon"
        );
        assert!(
            (title.x - (sep.x + sep.w + 5.0)).abs() < 0.6,
            "title flows after the separator, got title.x {} vs sep end {}",
            title.x,
            sep.x + sep.w
        );
        assert!(
            title.w > 5.0 && title.w < 339.0,
            "title takes its measured width, got {}",
            title.w
        );
    }

}

// Flex shrink policy tests (split from part_14 for the 500-line cap).

#[cfg(test)]
mod flex_shrink_tests {
    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    /// Auto ZERO-hint (value 0.0) text-backed children in a COLUMN stack at
    /// their measured text heights — the emissions Numbers Container stacks
    /// emitted above ambient. (Non-zero Auto text children content-fit the same
    /// way — see `column_nonzero_auto_text_children_content_fit_not_fill`.)
    #[test]
    fn column_zero_auto_text_children_stack_at_measured_heights() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 350.0, "y": 141.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "numbers", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Column", "axisJustification": "Center"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "emitted", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "3.5K",
                     "text": "3.5K",
                     "fontSize": {"value": 20.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.0, "behavior": "Auto"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "ambient", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "294.1",
                     "text": "294.1",
                     "fontSize": {"value": 20.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.0, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 350, 141);
        let emitted = result.rects[&2];
        let ambient = result.rects[&3];
        assert!(
            emitted.h > 4.0 && emitted.h < 141.0,
            "emitted gets a measured height, got {}",
            emitted.h
        );
        assert!(
            ambient.y >= emitted.y + emitted.h - 0.6,
            "ambient stacks below emitted: ambient.y {} vs emitted end {}",
            ambient.y,
            emitted.y + emitted.h
        );
    }

    /// A COLUMN child authored NON-zero `Auto` height ALSO sizes to its text
    /// content (the value is only the no-content fallback fraction). The power
    /// battery card's `base_ValuesContainer` (0.9 Auto over the "0 / 0" line)
    /// used to FILL 0.9/(0.9+0.5) of the 220px column = 141px, leaving ~71px
    /// empty below the numbers that pushed the OFFLINE row ~35px below the
    /// reference (handoff P14). Medical's header is a ROW child (its fill is
    /// cross-axis), so the frozen baselines are untouched.
    #[test]
    fn column_nonzero_auto_text_children_content_fit_not_fill() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 340.0, "y": 220.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "battery", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Column", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "values", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "0 / 0",
                     "text": "0 / 0",
                     "fontSize": {"value": 30.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.9, "behavior": "Auto"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "offline", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "OFFLINE",
                     "text": "OFFLINE",
                     "fontSize": {"value": 30.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.5, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 340, 220);
        let values = result.rects[&2];
        let offline = result.rects[&3];
        // Content-fit, NOT the 0.9-fill share (~141px of the 220px column).
        assert!(
            values.h < 120.0,
            "non-zero Auto values box content-fits, not 0.9-fill: got {}",
            values.h
        );
        // OFFLINE stacks directly under the content-sized values box — no fill
        // gap pushes it down the column (the bug was offline.y ~141).
        assert!(
            offline.y < 120.0 && offline.y <= values.y + values.h + 0.6,
            "OFFLINE stacks under values: offline.y {} vs values end {}",
            offline.y,
            values.y + values.h
        );
    }

    /// A CENTER-justified column's Auto card with fallback value EXACTLY 1.0
    /// content-fits its text — closing the `(0,1.0)`-vs-`>1.0` gap. The
    /// master-mode display (`HC_HUD_Ship_Master_Mode_Display_Master`) stacks two
    /// `WidgetCard`s (each wrapping one readout: "SCM" / submode) authored `Auto`
    /// value 1.0 on both axes; the `(0,1.0)` path needs `< 1.0` and the
    /// velocity-num path `> 1.0`, so 1.0 fell to the fill `else`, leaving both
    /// cards full-height and overlapping (the big grey box). Center-scoped, so
    /// the medical column (not center-justified) keeps its platinum fill (§10).
    #[test]
    fn center_column_auto_value_one_card_content_fits_text() {
        let card = |ptr: &str, name: &str, txt_ptr: &str, name2: &str, txt: &str| {
            serde_json::json!([
                {"_Pointer_": ptr, "_Type_": "BuildingBlocks_WidgetCard", "name": name,
                 "parent": "_PointsTo_:ptr:1", "isActive": true,
                 "sizing": {"width": {"value": 1.0, "behavior": "Auto"}, "height": {"value": 1.0, "behavior": "Auto"}}},
                {"_Pointer_": txt_ptr, "_Type_": "BuildingBlocks_WidgetTextField", "name": name2,
                 "parent": format!("_PointsTo_:{ptr}"), "isActive": true,
                 "_ResolvedText_": txt, "_DrawTextWidthPx_": 90.0, "_DrawTextHeightPx_": 40.0, "text": txt,
                 "fontSize": {"value": 40.0, "behavior": "Fixed"},
                 "sizing": {"width": {"value": 1.0, "behavior": "Auto"}, "height": {"value": 1.0, "behavior": "Auto"}}}
            ])
        };
        let mut scene_nodes = vec![serde_json::json!(
            {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_TextColumn", "isActive": true,
             "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer", "direction": "Column",
                              "axisJustification": "Center", "crossAxisJustification": "Center"},
             "sizing": {"width": {"value": 1.0, "behavior": "Percent"}, "height": {"value": 1.0, "behavior": "Percent"}}})];
        for n in card("ptr:2", "card_CurrentModeText", "ptr:3", "text_CurrentMode", "SCM").as_array().unwrap() {
            scene_nodes.push(n.clone());
        }
        for n in card("ptr:4", "card_SubModeText", "ptr:5", "text_SubMode", "NAV").as_array().unwrap() {
            scene_nodes.push(n.clone());
        }
        let canvas = serde_json::json!({"_RecordValue_": {"_Type_": "BuildingBlocks_Canvas",
            "size": {"x": 400.0, "y": 600.0}, "coordinateMethod": "useRaw",
            "scene": scene_nodes, "operations": []}});
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 400, 600);
        let (current, sub) = (result.rects[&2], result.rects[&4]);
        // Each card content-fits its single text line (not the 600px column)…
        assert!(current.h < 200.0, "current-mode card content-fits, got {}", current.h);
        assert!(sub.h < 200.0, "submode card content-fits, got {}", sub.h);
        // …and the two cards STACK rather than overlapping at the same rect.
        assert!(
            sub.y >= current.y + current.h - 0.6,
            "submode stacks below current-mode: sub.y {} vs current end {}",
            sub.y, current.y + current.h
        );
    }

    /// The non-zero `Auto` value stays the fallback when the child has NO
    /// measurable text: it fills `value × container` as before (only
    /// text-backed children switch to content-fit).
    #[test]
    fn column_nonzero_auto_without_text_keeps_fill_fallback() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 340.0, "y": 200.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "col", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Column", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "band", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.5, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 340, 200);
        let band = result.rects[&2];
        // No text → fall back to fill: 0.5 × 200 = 100.
        assert!(
            (band.h - 100.0).abs() < 1.0,
            "text-less non-zero Auto band fills value×container (~100), got {}",
            band.h
        );
    }

    /// The intrinsic text measure must match what the draw will actually
    /// paint (catalog #3/#4, crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md): ui_ir's
    /// pre-layout annotation pass — which knows the resolved font record
    /// and the draw-side SWF glyph metrics — writes
    /// `_DrawTextWidthPx_`/`_DrawTextHeightPx_`; `node_resolved_text_size`
    /// prefers those annotations over its TTF estimate. On the SWF draw
    /// path the TTF estimate overshoots ~1.5× (the draw renders at the IR
    /// font size with NO `TEXT_RENDER_SIZE_CALIBRATION`), which parked the
    /// OUTPUT title 72px right (authored Right alignment in the oversized
    /// box) and OFFLINE at 543px in a 339px card.
    #[test]
    fn auto_text_child_prefers_draw_metrics_annotations() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 400.0, "y": 100.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "row", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "title", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "OUTPUT",
                     "_DrawTextWidthPx_": 160.4,
                     "_DrawTextHeightPx_": 30.0,
                     "text": "OUTPUT",
                     "fontSize": {"value": 20.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 32.0, "behavior": "Auto"},
                                "height": {"value": 32.0, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 400, 100);
        let title = result.rects[&2];
        assert!(
            (title.w - 160.4).abs() < 0.6,
            "the Auto title box hugs the DRAW width annotation, got {}",
            title.w
        );
    }

    /// Zero-auto text-backed children are CONTENT-SIZED flex items: when
    /// their measured intrinsics overflow the column they shrink
    /// proportionally to fit like every other flex-managed class (the
    /// emissions Numbers Container's emitted/ambient pair authors 0.0Auto
    /// texts whose nominal-font measure overflows the 141px band — the
    /// engine shows both lines adjacent inside the band).
    #[test]
    fn column_zero_auto_text_children_shrink_to_fit_container() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 350.0, "y": 141.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "numbers", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Column", "axisJustification": "Center"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "emitted", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "3.5K",
                     "text": "3.5K",
                     "fontSize": {"value": 80.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.0, "behavior": "Auto"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "ambient", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "294.1",
                     "text": "294.1",
                     "fontSize": {"value": 80.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.0, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 350, 141);
        let emitted = result.rects[&2];
        let ambient = result.rects[&3];
        assert!(
            emitted.h > 4.0,
            "emitted keeps a measured height, got {}",
            emitted.h
        );
        assert!(
            ambient.y >= emitted.y + emitted.h - 0.6,
            "ambient stacks below emitted: ambient.y {} vs emitted end {}",
            ambient.y,
            emitted.y + emitted.h
        );
        assert!(
            ambient.y + ambient.h <= 141.0 + 0.6,
            "the pair shrinks to fit the 141px column, got ambient end {}",
            ambient.y + ambient.h
        );
    }

    /// Flex flow lays children in `layoutItemCommon.order`, not scene order —
    /// the emissions clones author scene order EM, CS, IR with orders 3, 5, 1
    /// (separators interleaved at even orders) and the engine shows IR first.
    #[test]
    fn flex_children_flow_in_layout_item_order() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 300.0, "y": 50.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "row", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "em", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutItemCommon": {"_Type_": "BuildingBlocks_LayoutItemCommon", "order": 3},
                     "sizing": {"width": {"value": 50.0, "behavior": "Fixed"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "cs", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutItemCommon": {"_Type_": "BuildingBlocks_LayoutItemCommon", "order": 5},
                     "sizing": {"width": {"value": 50.0, "behavior": "Fixed"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "ir", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutItemCommon": {"_Type_": "BuildingBlocks_LayoutItemCommon", "order": 1},
                     "sizing": {"width": {"value": 50.0, "behavior": "Fixed"},
                                "height": {"value": 1.0, "behavior": "Percent"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 300, 50);
        let (ir, em, cs) = (result.rects[&4], result.rects[&2], result.rects[&3]);
        assert!(
            ir.x < em.x && em.x < cs.x,
            "order 1 < 3 < 5 lays IR, EM, CS left to right: ir.x={} em.x={} cs.x={}",
            ir.x, em.x, cs.x
        );
    }

    /// Overflowing no-grow flex children shrink proportionally to fit (CSS
    /// flex-shrink): the battery card's column authors 0.9 + 0.5 fractions
    /// (318px in a 227px container) and the in-game card keeps all three
    /// rows inside. Scrolling lists are exempt (the pip columns overflow by
    /// design).
    #[test]
    fn overflowing_no_grow_children_shrink_to_fit() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 300.0, "y": 227.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "column", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Column", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "values", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.9, "behavior": "Auto"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "offline_row", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 0.5, "behavior": "Auto"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 300, 227);
        let values = result.rects[&2];
        let offline = result.rects[&3];
        let shrink = 227.0 / (0.9 * 227.0 + 0.5 * 227.0);
        assert!(
            (values.h - 0.9 * 227.0 * shrink).abs() < 1.0,
            "values row shrinks proportionally, got {}",
            values.h
        );
        assert!(
            (offline.y - (values.y + values.h)).abs() < 0.6,
            "rows stack sequentially after shrink"
        );
        assert!(
            offline.y + offline.h <= 227.5,
            "rows fit the container, bottom {}",
            offline.y + offline.h
        );
    }

    /// A flex item with an explicit `shrinkProportion` of 0 must NOT shrink (CSS
    /// flex-shrink: 0): it keeps its base size and the row overflows. The g-force
    /// / velocity ball's `card_BallArea` is authored SQUARE (width = PercentOfY =
    /// its own height) with shrinkProportion 0, alongside a `card_Readouts` that
    /// also authors shrinkProportion 0 and is meant to OVERFLOW/crop (the gauge has
    /// no separate num screen). Honouring shrinkProportion keeps the ball SQUARE;
    /// without it the row (ball + readouts) overflowed the 16:9 canvas and the
    /// shrink narrowed the ball below square — the squashed g-force cross.
    #[test]
    fn flex_shrink_proportion_zero_keeps_ball_square() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 1280.0, "y": 720.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "base_Root", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Start",
                                      "crossAxisJustification": "Center"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "card_BallArea", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutPolicyItem": {"_Type_": "BuildingBlocks_FlexItem",
                                          "growProportion": 0.0, "shrinkProportion": 0.0},
                     "sizing": {"width": {"value": 1.0, "behavior": "PercentOfY"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "card_Readouts", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutPolicyItem": {"_Type_": "BuildingBlocks_FlexItem",
                                          "growProportion": 0.0, "shrinkProportion": 0.0},
                     "sizing": {"width": {"value": 1.5, "behavior": "PercentOfY"},
                                "height": {"value": 0.6, "behavior": "Percent"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 1280, 720);
        let ball = result.rects[&2];
        assert!(
            (ball.w - ball.h).abs() < 1.0,
            "card_BallArea (shrinkProportion 0) must stay square, got {}x{}",
            ball.w, ball.h
        );
        assert!(
            (ball.w - 720.0).abs() < 1.0,
            "card_BallArea keeps its base square width (720), got {}",
            ball.w
        );
    }

    /// Sizing-method "None" children are not flex-managed: they fall back to
    /// container fill and visibly overflow (the medical footer's TextLayout
    /// row holds two 604.8px-wide label-caption pairs in a 604.8px row with
    /// overflow Visible). Shrink must not redistribute them.
    #[test]
    fn fill_fallback_children_do_not_shrink() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 604.8, "y": 108.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "text_layout", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Start",
                                      "columnSpacing": 30.0},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "operator_name", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 64.0, "behavior": "None"},
                                "height": {"value": 64.0, "behavior": "None"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "patient_name", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 64.0, "behavior": "None"},
                                "height": {"value": 64.0, "behavior": "None"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 605, 108);
        assert!(
            (result.rects[&2].w - 604.8).abs() < 1.5 && (result.rects[&3].w - 604.8).abs() < 1.5,
            "fill-fallback children keep container width, got {} and {}",
            result.rects[&2].w,
            result.rects[&3].w
        );
    }

    /// The flex-managed taxonomy mirrors `resolve_value`: Fixed, Percent and
    /// normalized Auto fractions (0,1] are engine-distributed extents; Auto
    /// content hints (>1, e.g. the medical label-caption pairs' Auto 64) and
    /// method "None" resolve via the fill fallback and must not shrink.
    #[test]
    fn auto_content_hints_are_not_flex_managed() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 100.0, "y": 100.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "fixed", "isActive": true,
                     "sizing": {"width": {"value": 80.0, "behavior": "Fixed"},
                                "height": {"value": 80.0, "behavior": "Fixed"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "auto_fraction", "isActive": true,
                     "sizing": {"width": {"value": 0.9, "behavior": "Auto"},
                                "height": {"value": 0.9, "behavior": "Auto"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "auto_hint", "isActive": true,
                     "sizing": {"width": {"value": 64.0, "behavior": "Auto"},
                                "height": {"value": 64.0, "behavior": "Auto"}}},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "method_none", "isActive": true,
                     "sizing": {"width": {"value": 64.0, "behavior": "None"},
                                "height": {"value": 64.0, "behavior": "None"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let by_name = |name: &str| {
            scene
                .nodes
                .values()
                .find(|n| n.name == name)
                .expect("fixture node")
        };
        assert!(main_axis_flex_managed(by_name("fixed"), true));
        assert!(main_axis_flex_managed(by_name("auto_fraction"), true));
        assert!(!main_axis_flex_managed(by_name("auto_hint"), true));
        assert!(!main_axis_flex_managed(by_name("method_none"), true));
    }

    /// A scrolling list keeps its overflow (the power screen's pip columns
    /// scroll horizontally past the viewport).
    #[test]
    fn scrolling_list_children_do_not_shrink() {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 100.0, "y": 50.0},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetList",
                     "name": "list", "isActive": true,
                     "scrollPolicy": {"_Type_": "BuildingBlocks_UnidirectionalScroller",
                                      "scrollDirection": "Horizontal"},
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "item_a", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 80.0, "behavior": "Fixed"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "item_b", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"width": {"value": 80.0, "behavior": "Fixed"},
                                "height": {"value": 1.0, "behavior": "Percent"}}}
                ],
                "operations": []
            }
        });
        let scene = parse_bb_canvas(&canvas).expect("fixture parses");
        let result = layout(&scene, 100, 50);
        assert!(
            (result.rects[&2].w - 80.0).abs() < 0.5 && (result.rects[&3].w - 80.0).abs() < 0.5,
            "scroll-list items keep their authored size"
        );
    }
}
