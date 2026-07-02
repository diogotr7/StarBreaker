//! Number/boolean modifier application tests for the geometry fields split
//! into `modifiers_number.rs`: padding, border corner radii, and svg flips.

use super::tests_support::make_test_scene;
use super::*;
use serde_json::json;

    /// `Padding*` modifiers must land on the typed `node.padding` the layout
    /// engine reads — the modular-kit button `Root` entry insets the icon
    /// instance inside the chrome box with `PaddingTop/Right/Bottom/Left 15`.
    #[test]
    fn test_padding_fields_apply_to_typed_padding() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "PaddingTop",
                        "value": 15.0
                    },
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "PaddingRight",
                        "value": 16.0
                    },
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "PaddingBottom",
                        "value": 17.0
                    },
                    {
                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                        "field": "PaddingLeft",
                        "value": 18.0
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(node.padding.top, 15.0);
        assert_eq!(node.padding.right, 16.0);
        assert_eq!(node.padding.bottom, 17.0);
        assert_eq!(node.padding.left, 18.0);
    }

    /// `SvgFlipHorizontal` writes the authored raw `svgFill.flipHorizontal`
    /// the IR's asset-layout reader consumes — the MFD header's "Button Icon
    /// Flip" entry mirrors the left nav arrow.
    #[test]
    fn test_svg_flip_horizontal_writes_raw_svg_fill() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierBoolean", "field": "SvgFlipHorizontal", "value": true}
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert_eq!(
            node.raw
                .get("svgFill")
                .and_then(|s| s.get("flipHorizontal"))
                .and_then(|v| v.as_bool()),
            Some(true),
            "flip must land on raw svgFill.flipHorizontal"
        );
    }

    /// `Border<Corner>Radius` modifiers write the authored raw border-radius
    /// structure the IR's `node_corner_radius` reads — the sk RootGhost entry
    /// rounds the ghost button chrome with all four corners at 6.0.
    #[test]
    fn test_border_corner_radius_fields_apply_to_raw_border() {
        let mut scene = make_test_scene();
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "BorderTopLeftRadius", "value": 6.0},
                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "BorderTopRightRadius", "value": 6.0},
                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "BorderBottomLeftRadius", "value": 6.0},
                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "BorderBottomRightRadius", "value": 6.0}
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        for corner in ["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"] {
            let radius = node
                .raw
                .get("border")
                .and_then(|b| b.get(corner))
                .and_then(|c| c.get("radius"))
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_f64());
            assert_eq!(radius, Some(6.0), "corner '{corner}' radius should be 6.0");
        }
    }


    /// A `SizeX`/`SizeY` number modifier changes the VALUE and preserves the
    /// node's authored sizing behaviour unless an explicit
    /// `WidthBehavior`/`HeightBehavior` override says otherwise — the
    /// emissions clones (Percent-sized) styled `SizeY 1.0` stay parent
    /// fractions; converting to Fixed collapsed the header to 1px.
    #[test]
    fn test_size_modifiers_preserve_authored_behaviour() {
        let mut scene = make_test_scene();
        {
            let node = scene.nodes.get_mut(&1).unwrap();
            node.sizing.width = crate::bb_scene::BbValue::Percent(0.3);
            node.sizing.height = crate::bb_scene::BbValue::Percent(0.5);
        }
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeY", "value": 1.0}
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        assert_eq!(
            scene.nodes[&1].sizing.height,
            crate::bb_scene::BbValue::Percent(1.0),
            "Percent stays Percent"
        );

        // An explicit HeightBehavior raw override still wins.
        scene.nodes.get_mut(&1).unwrap().raw = json!({"HeightBehavior": "Fixed"});
        apply_brand_modifiers(&mut scene, &brand, None);
        assert_eq!(scene.nodes[&1].sizing.height, crate::bb_scene::BbValue::Fixed(1.0));
    }

    /// Enumerated flex modifiers rewrite the node's authored layoutPolicy —
    /// the drak emissions "Numbers Container" entry turns the authored Row
    /// into a Column (emitted stacked above ambient).
    #[test]
    fn test_flex_enum_modifiers_rewrite_layout_policy() {
        let mut scene = make_test_scene();
        scene.nodes.get_mut(&1).unwrap().raw = json!({
            "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                             "direction": "Row", "axisJustification": "Start",
                             "crossAxisJustification": "Stretch"}
        });
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierEnumerated",
                     "field": {"_Type_": "BuildingBlocks_FieldModifierEnumeratedTypeFlexDirection",
                               "value": "Column"}},
                    {"_Type_": "BuildingBlocks_FieldModifierEnumerated",
                     "field": {"_Type_": "BuildingBlocks_FieldModifierEnumeratedTypeFlexCrossAxisJustification",
                               "value": "Center"}}
                ]
            })],
            raw: &json!({}),
        };
        apply_brand_modifiers(&mut scene, &brand, None);
        let lp = &scene.nodes[&1].raw["layoutPolicy"];
        assert_eq!(lp["direction"], "Column");
        assert_eq!(lp["crossAxisJustification"], "Center");
        assert_eq!(lp["axisJustification"], "Start", "untouched field survives");
    }

/// The button standards' Filled state authors PER-CORNER radii + chamfer
/// flags (`RootFilled`: BR radius 20, TL/BR chamfer true). The radii must
/// land in the raw border structure `node_corner_radius` reads, and the
/// chamfer booleans must be recorded for the draw (console button, ledger
/// 106 — the corners stayed at the template's uniform 10).
#[test]
fn per_corner_radius_and_chamfer_modifiers_apply() {
    let mut scene = super::tests_support::make_test_scene();
    let node = scene.nodes.get_mut(&1).expect("test node");
    let palette = serde_json::json!({"colorStyles": []});
    let mods = [
        serde_json::json!({"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "BorderTopLeftRadius", "value": 0.0}),
        serde_json::json!({"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "BorderBottomRightRadius", "value": 20.0}),
        serde_json::json!({"_Type_": "BuildingBlocks_FieldModifierBoolean", "field": "EnableTopLeftBorderChamfer", "value": true}),
        serde_json::json!({"_Type_": "BuildingBlocks_FieldModifierBoolean", "field": "EnableBottomRightBorderChamfer", "value": true}),
    ];
    for modifier in &mods {
        super::modifiers::apply_modifier(
            modifier,
            node,
            &super::colors::PaletteSources::uniform(&palette),
            None,
        );
    }
    let radius = |corner: &str| {
        node.raw
            .get("border")
            .and_then(|b| b.get(corner))
            .and_then(|c| c.get("radius"))
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_f64())
    };
    // Zero radius = keep authored (the reference keeps the canvas's base
    // rounding on the standards' zeroed corners — ledger 106).
    assert_eq!(radius("topLeftRadius"), None);
    assert_eq!(radius("bottomRightRadius"), Some(20.0));
    assert_eq!(
        node.raw.get("EnableTopLeftBorderChamfer").and_then(|v| v.as_bool()),
        Some(true),
        "chamfer flags must be recorded for the draw"
    );
    assert_eq!(
        node.raw.get("EnableBottomRightBorderChamfer").and_then(|v| v.as_bool()),
        Some(true)
    );
}
