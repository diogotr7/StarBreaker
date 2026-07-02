use super::tests_support::make_test_scene;
use super::*;
use crate::bb_scene::BbBackground;
use serde_json::json;

    #[test]
    fn named_fill_color_preserves_token_in_raw() {
        let palette = json!({
            "colorStyles": [
                {"color": {"r": 1.0, "g": 0.5, "b": 0.25, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.25, "b": 0.75, "a": 1.0}}
            ]
        });

        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierColor",
            "field": "FillColor",
            "color": {
                "_Type_": "BuildingBlocks_ColorStyle",
                "color": "Accent1",
                "alpha": 1.0
            }
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &PaletteSources::uniform(&palette), None);

        assert_eq!(
            node.raw.get("FillColorToken").and_then(|value| value.as_str()),
            Some("Accent1")
        );
        assert!(node.raw.get("FillColor").is_some(), "resolved rgba should still be present");
    }

    #[test]
    fn literal_color_application_drops_stale_token_for_border_fields() {
        // A literal (`ColorSolid`) modifier carries no palette token; a stale
        // lower-tier token left in raw shadows the literal at draw time (the
        // Filled-button icon bug, review F5 / ledger 103). The drop-on-None
        // semantics live in `write_color_token_to_raw` so EVERY colour field
        // arm gets them, not just FillColor.
        let palette = json!({
            "colorStyles": [
                {"color": {"r": 1.0, "g": 0.5, "b": 0.25, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}},
                {"color": {"r": 0.0, "g": 0.25, "b": 0.75, "a": 1.0}}
            ]
        });
        let tokened = json!({
            "_Type_": "BuildingBlocks_FieldModifierColor",
            "field": "BorderColorTop",
            "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Accent1", "alpha": 1.0}
        });
        let literal = json!({
            "_Type_": "BuildingBlocks_FieldModifierColor",
            "field": "BorderColorTop",
            "color": {
                "_Type_": "BuildingBlocks_ColorSolid",
                "color": {"_Type_": "SRGBA8", "r": 0, "g": 0, "b": 0, "a": 255}
            }
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&tokened, node, &PaletteSources::uniform(&palette), None);
        assert_eq!(
            node.raw.get("BorderColorTopToken").and_then(|v| v.as_str()),
            Some("Accent1"),
            "precondition: the tokened application writes the token"
        );

        apply_modifier(&literal, node, &PaletteSources::uniform(&palette), None);
        assert!(
            node.raw.get("BorderColorTopToken").is_none(),
            "a literal application must drop the stale palette token"
        );
        let border = node.border.as_ref().expect("border should be ensured");
        assert_eq!(
            border.top.colour,
            Some([0.0, 0.0, 0.0, 1.0]),
            "the literal black must be applied"
        );
    }

    #[test]
    fn custom_shape_inline_overlay_accent1_resolves_to_surface_slot() {
        // s_bioc-like palette: slot 0 = light blue (foreground), slot 4 = dark blue
        // (surface). A custom-shape fill overlay (the medical "fingerprint") must
        // resolve `Accent1` to the darker surface slot 4, not the light slot 0.
        let palette = json!({
            "colorStyles": [
                {"color": {"r": 115, "g": 198, "b": 254, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 0, "b": 0, "a": 255}},
                {"color": {"r": 0, "g": 113, "b": 188, "a": 255}}
            ]
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        node.ty = crate::bb_scene::BbNodeType::WidgetCustomShape;
        node.raw = json!({
            "svgFill": {
                "renderShape": true,
                "enableColorOverlay": true,
                "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Accent1", "alpha": 1.0}
            }
        });

        apply_inline_color_overlay(node, &palette);

        let fill = node
            .raw
            .get("FillColor")
            .and_then(|v| v.as_object())
            .expect("inline overlay should apply a resolved FillColor");
        let chan = |k: &str| {
            let v = fill.get(k).and_then(|v| v.as_f64()).unwrap() as f32;
            if v > 1.0 { v / 255.0 } else { v }
        };
        // Surface slot 4 = (0,113,188): red ≈ 0. Foreground slot 0 = (115,198,254):
        // red ≈ 0.45. A low red proves we resolved the surface (dark) slot.
        assert!(chan("r") < 0.05, "expected surface slot4 (dark blue, low red), got r={}", chan("r"));
        assert!(chan("b") > 0.5, "expected a blue fill, got b={}", chan("b"));
        assert_eq!(node.raw.get("FillColorToken").and_then(|v| v.as_str()), Some("Accent1"));
    }

    /// A drak-HUD-shaped palette: slot 0 Base = orange, slot 6 Bright = cream,
    /// slot 8 Disabled = near-black. Mirrors `s_drak_hud.json`.
    fn drak_like_palette() -> serde_json::Value {
        let dummy = json!({"color": {"r": 0, "g": 0, "b": 0, "a": 255}});
        let mut slots = vec![dummy.clone(); 17];
        slots[0] = json!({"color": {"r": 255, "g": 158, "b": 57, "a": 255}}); // Base
        slots[6] = json!({"color": {"r": 255, "g": 255, "b": 224, "a": 255}}); // Bright
        slots[7] = json!({"color": {"r": 255, "g": 119, "b": 0, "a": 255}}); // Selected
        slots[8] = json!({"color": {"r": 20, "g": 13, "b": 5, "a": 255}}); // Disabled
        json!({ "colorStyles": slots })
    }

    fn raw_color_channel(node: &crate::bb_scene::BbNode, field: &str, chan: &str) -> f32 {
        let v = node
            .raw
            .get(field)
            .and_then(|c| c.get(chan))
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("missing {field}.{chan}")) as f32;
        if v > 1.0 { v / 255.0 } else { v }
    }

    #[test]
    fn background_color_disabled_resolves_to_dark_slot_8() {
        // The MFD footer's segment-box background is `BackgroundColor = Disabled`,
        // which in-game is the near-black slot 8 (20,13,5) — a dark, recessed bar.
        // Mapping it to the light slot 6 paints a bright bar (the opposite).
        let palette = drak_like_palette();
        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierColor",
            "field": "BackgroundColor",
            "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Disabled", "alpha": 1.0}
        });
        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &PaletteSources::uniform(&palette), None);
        let node = scene.nodes.get(&1).unwrap();
        assert!(
            raw_color_channel(node, "BackgroundColor", "r") < 0.15,
            "Disabled must resolve to dark slot 8, got r={}",
            raw_color_channel(node, "BackgroundColor", "r")
        );
    }

    #[test]
    fn fill_color_bright_surface_resolves_to_brand_slot_0() {
        // In the brand-apply (surface) resolver, `Bright` maps to the brand's
        // primary slot 0 — NOT the enum's index-6. Verified in-game: the MFD
        // footer's `Bright` selected-name renders the drak slot-0 orange (the
        // reference's "TARGET STATUS" is the same orange as "NO TARGET"), and
        // medical `Bright` custom-shapes render the s_bioc slot-0 light-blue.
        let palette = drak_like_palette();
        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierColor",
            "field": "FillColor",
            "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Bright", "alpha": 1.0}
        });
        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &PaletteSources::uniform(&palette), None);
        let node = scene.nodes.get(&1).unwrap();
        // Orange slot 0: red ≈ 1.0, blue ≈ 0.22. Cream slot 6: blue ≈ 0.88.
        assert!(
            raw_color_channel(node, "FillColor", "r") > 0.9
                && raw_color_channel(node, "FillColor", "b") < 0.4,
            "Bright (surface) must resolve to brand slot 0 orange, got r={} b={}",
            raw_color_channel(node, "FillColor", "r"),
            raw_color_channel(node, "FillColor", "b")
        );
    }

    #[test]
    fn overlay_icon_without_authored_colour_defaults_to_brand_base() {
        // The MFD footer's nav carats are overlay-enabled WidgetIcons with a null
        // `svgFill.color`; in-game they tint to the brand's primary foreground
        // (`Base` — drak slot 0 orange), not the SVG's own (dark) colour.
        let palette = drak_like_palette();
        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        node.ty = crate::bb_scene::BbNodeType::WidgetIcon;
        node.raw = json!({
            "svgFill": {"_Type_": "BuildingBlocks_SvgFill", "enableColorOverlay": true, "color": null}
        });
        apply_inline_color_overlay(node, &palette);
        let node = scene.nodes.get(&1).unwrap();
        assert!(
            raw_color_channel(node, "FillColor", "r") > 0.9
                && raw_color_channel(node, "FillColor", "b") < 0.4,
            "overlay icon with null colour must default to Base slot 0 orange, got r={} b={}",
            raw_color_channel(node, "FillColor", "r"),
            raw_color_channel(node, "FillColor", "b")
        );
        assert_eq!(
            node.raw.get("FillColorToken").and_then(|v| v.as_str()),
            Some("Base")
        );
    }

    #[test]
    fn record_ref_font_style_object_field_maps_to_font_style_record() {
        let palette = json!({});
        let modifier = json!({
            "_Type_": "BuildingBlocks_FieldModifierRecordRef",
            "field": {
                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                "value": "file://./../../fontstyles/blenderpro-bold.json"
            }
        });

        let mut scene = make_test_scene();
        let node = scene.nodes.get_mut(&1).expect("test node");
        apply_modifier(&modifier, node, &PaletteSources::uniform(&palette), None);

        assert_eq!(
            node.raw
                .get("FontStyleRecord")
                .and_then(|value| value.as_str()),
            Some("file://./../../fontstyles/blenderpro-bold.json")
        );
    }

    #[test]
    fn test_mixed_type_and_tag_condition_tag_mismatch() {
        // Mixed AllOf: type matches but tag doesn't → should NOT apply.
        let mut scene = make_test_scene(); // WidgetImage, tag = "tag-uuid-1"
        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAllOfCondition",
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionType",
                                "type": "Image"
                            },
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": { "_RecordId_": "wrong-tag" }
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/should_not_apply.tif"
                        }
                    }
                ]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).unwrap();
        assert!(
            node.raw.get("ImagePath").is_none(),
            "Mixed type+tag condition must NOT match when tag is wrong"
        );
    }

    #[test]
    fn test_condition_ancestor_matches_grandparent_tag() {
        let mut scene = make_test_scene();
        // Re-parent node 1 under parent 2 under grandparent 3.
        let mut parent = scene.nodes.get(&1).cloned().unwrap();
        parent.id = 2;
        parent.name = "parent".to_string();
        parent.parent = Some(3);
        parent.children = vec![1];
        parent.style_tag_uuids = vec!["parent-tag".to_string()];
        parent.raw = json!({});
        scene.nodes.insert(2, parent);

        let mut grandparent = scene.nodes.get(&1).cloned().unwrap();
        grandparent.id = 3;
        grandparent.name = "grandparent".to_string();
        grandparent.parent = None;
        grandparent.children = vec![2];
        grandparent.style_tag_uuids = vec!["ancestor-tag".to_string()];
        grandparent.raw = json!({});
        scene.nodes.insert(3, grandparent);

        let child = scene.nodes.get_mut(&1).unwrap();
        child.parent = Some(2);
        child.children.clear();
        child.background = Some(BbBackground::default());
        scene.roots = vec![3];

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAncestor",
                        "conditions": [{
                            "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                            "tag": { "_RecordId_": "ancestor-tag" }
                        }]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierColor",
                    "field": "FillColor",
                    "color": { "r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0 }
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);
        let fill = scene
            .nodes
            .get(&1)
            .unwrap()
            .background
            .as_ref()
            .unwrap()
            .fill_colour
            .unwrap();
        assert!((fill[0] - 0.25).abs() < 0.001);
        assert!((fill[1] - 0.5).abs() < 0.001);
        assert!((fill[2] - 0.75).abs() < 0.001);
        assert!((fill[3] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_condition_any_of_tag_matches_when_any_tag_matches() {
        let mut scene = make_test_scene();

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAnyOfTag",
                        "tags": [
                            { "_RecordId_": "wrong-tag" },
                            { "_RecordId_": "tag-uuid-1" }
                        ]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierString",
                    "field": "ImagePath",
                    "value": "UI/Textures/any_of_tag_hit.tif"
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).expect("test node");
        assert_eq!(
            node.raw.get("ImagePath").and_then(|value| value.as_str()),
            Some("UI/Textures/any_of_tag_hit.tif")
        );
    }

    #[test]
    fn test_condition_any_of_tag_no_match_when_no_tags_match() {
        let mut scene = make_test_scene();

        let brand = BrandStyle {
            identifier: "test_brand".to_string(),
            entries: &[json!({
                "conditionsList": [{
                    "conditions": [{
                        "_Type_": "BuildingBlocks_StyleSelectorConditionAnyOfTag",
                        "tags": [
                            { "_RecordId_": "wrong-tag-a" },
                            { "_RecordId_": "wrong-tag-b" }
                        ]
                    }]
                }],
                "modifiers": [{
                    "_Type_": "BuildingBlocks_FieldModifierString",
                    "field": "ImagePath",
                    "value": "UI/Textures/any_of_tag_should_not_apply.tif"
                }]
            })],
            raw: &json!({}),
        };

        apply_brand_modifiers(&mut scene, &brand, None);

        let node = scene.nodes.get(&1).expect("test node");
        assert!(
            node.raw.get("ImagePath").is_none(),
            "ConditionAnyOfTag should not match when node has none of the tags"
        );
    }
