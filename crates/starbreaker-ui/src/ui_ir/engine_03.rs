#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use sha2::{Digest, Sha256};
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
use crate::bb_bindings::BindingResolver;
#[allow(unused_imports)]
use crate::bb_layout;
#[allow(unused_imports)]
use crate::bb_layout::{LayoutResult, Rect};
#[allow(unused_imports)]
use crate::bb_scene::{BbNode, BbNodeId, BbNodeType, BbScene, BbValue};
#[allow(unused_imports)]
use crate::defaults::DefaultValueRegistry;
#[allow(unused_imports)]
use crate::pipeline::CanvasFetcher;

// Consolidated engine chunk 03 (formerly: part_11.part, part_12.part, part_13.part).

#[cfg(test)]
mod tests {
    #![allow(unused_imports, dead_code)]

    use super::*;

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }

    #[test]
    fn resolve_style_tag_record_uses_matched_tag_instead_of_full_database() {
        let tag_db_path = "libs/foundry/records/tagdatabase/tagdatabase.tagdatabase.json";
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(tag_db_path.to_string(), serde_json::json!({
                "_RecordId_": "66ee5bfc-d90b-41bd-ad2e-e0a2b3efe359",
                "_RecordName_": "TagDatabase.TagDatabase",
                "_RecordValue_": {
                    "_Type_": "TagDatabase",
                    "tags": [
                        {
                            "_RecordId_": "parent-tag",
                            "tagName": "Parent",
                            "children": [
                                {
                                    "_RecordId_": "target-tag",
                                    "tagName": "Target",
                                    "children": [
                                        {
                                            "_RecordId_": "descendant-tag",
                                            "tagName": "Descendant"
                                        }
                                    ]
                                }
                            ]
                        }
                    ]
                }
            }))]),
        };

        let tag_reference = serde_json::json!({
            "_RecordId_": "target-tag",
            "_RecordName_": "Tag.target-tag",
            "_RecordPath_": tag_db_path,
        });

        let resolved = resolve_style_tag_record(
            Some(&fetcher),
            &tag_reference,
            tag_db_path,
            "Tag.target-tag",
            "target-tag",
        )
        .expect("tag should resolve");

        assert_eq!(resolved.get("_Type_").and_then(|v| v.as_str()), Some("Tag"));
        assert_eq!(resolved.get("_RecordId_").and_then(|v| v.as_str()), Some("target-tag"));

        let record_value = resolved.get("_RecordValue_").expect("record value");
        assert_eq!(record_value.get("_RecordId_").and_then(|v| v.as_str()), Some("target-tag"));
        assert_eq!(record_value.get("tagName").and_then(|v| v.as_str()), Some("Target"));
        assert!(record_value.get("tags").is_none(), "full tag database leaked into record");
        assert!(record_value.get("children").is_none(), "descendant tags leaked into record");
    }

    #[test]
    fn compile_ir_style_tags_are_compact() {
        let tag_db_path = "libs/foundry/records/tagdatabase/tagdatabase.tagdatabase.json";
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(tag_db_path.to_string(), serde_json::json!({
                "_RecordId_": "66ee5bfc-d90b-41bd-ad2e-e0a2b3efe359",
                "_RecordName_": "TagDatabase.TagDatabase",
                "_RecordValue_": {
                    "_Type_": "TagDatabase",
                    "tags": [
                        {
                            "_RecordId_": "target-tag",
                            "tagName": "Target"
                        }
                    ]
                }
            }))]),
        };

        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestTags",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "root",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        },
                        "styleTags": [
                            {
                                "_RecordId_": "target-tag",
                                "_RecordName_": "Tag.target-tag",
                                "_RecordPath_": tag_db_path
                            }
                        ]
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let target = scene.nodes.values_mut().next().expect("target node");
        target.raw["FontSize"] = serde_json::Value::from(40.0);
        target.raw["__AppliedStyleEntries"] = serde_json::json!([
            {
                "name": "ParentTitleSize",
                "conditionsList": [
                    {
                        "conditions": [
                            {
                                "_Type_": "BuildingBlocks_StyleSelectorConditionParent",
                                "conditions": [
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionTag"}
                                ]
                            }
                        ]
                    }
                ],
                "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 40.0}
                ]
            }
        ]);
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-tags",
            Some("BuildingBlocks_Canvas.TestTags"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let actual = serde_json::to_value(ir).expect("serialize ir");
        let style_tag = actual
            .get("nodes")
            .and_then(|v| v.as_array())
            .and_then(|nodes| nodes.first())
            .and_then(|node| node.get("resolved_style_tags"))
            .and_then(|v| v.as_array())
            .and_then(|tags| tags.first())
            .and_then(|v| v.as_object())
            .expect("resolved style tag object");

        assert_eq!(style_tag.get("uuid").and_then(|v| v.as_str()), Some("target-tag"));
        assert_eq!(style_tag.get("tag_name").and_then(|v| v.as_str()), Some("Target"));
        assert_eq!(style_tag.len(), 2, "resolved style tags should serialize only essential fields");
    }

    #[test]
    fn collect_node_asset_refs_includes_brand_applied_paths() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAssets",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "asset_node",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.values_mut().next().expect("node");
        node.raw
            .as_object_mut()
            .expect("raw object")
            .insert(
                "ImagePath".to_string(),
                serde_json::Value::String(
                    "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_bottom-bar.tif".to_string(),
                ),
            );
        node.raw
            .as_object_mut()
            .expect("raw object")
            .insert(
                "SvgPath".to_string(),
                serde_json::Value::String(
                    "UI/Textures/Vector/General/BrandLogos/logo_bioticorp_a.svg".to_string(),
                ),
            );

        let asset_refs = collect_node_asset_refs(node);
        assert_eq!(
            asset_refs,
            vec![
                "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_bottom-bar.tif".to_string(),
                "UI/Textures/Vector/General/BrandLogos/logo_bioticorp_a.svg".to_string(),
            ]
        );
    }

    #[test]
    fn compile_ir_entry_less_overlay_custom_shape_defaults_to_missionobjectives() {
        // The power screen's system/card icons (shape_SystemIcon,
        // shape_OutputIcon…) are WidgetCustomShapes with
        // svgFill.enableColorOverlay and a null colour, and the DRAK brand
        // authors NO at-rest colour entry for them (other brands do: misc/orig
        // author "System Icon Color"). In-game they render the brand's
        // MissionObjectives slot — the design system's generic icon colour
        // (HUD records author `FillColor=MissionObjectives` for Icon Styles).
        // Entry-coloured shapes (the target chevrons' embedded Base, the
        // medical fingerprint's Accent1) resolve from raw first and are
        // unaffected.
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestIconShapeDefault",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape_SystemIcon",
                        "isActive": true,
                        "svgFill": {
                            "svgPath": "UI/Textures/Vector/General/CommonIcons/icon_common_weapon_gun.svg",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "color": null
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 99.0},
                            "height": {"behavior": "Fixed", "value": 99.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-icon-shape-default",
            Some("BuildingBlocks_Canvas.TestIconShapeDefault"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|n| n.name == "shape_SystemIcon")
            .expect("icon shape node");
        assert_eq!(
            node.icon_tint_colour_token.as_deref(),
            Some("MissionObjectives"),
            "an entry-less colour-overlay custom shape takes the generic icon colour"
        );
    }

    #[test]
    fn compile_ir_hud_overlay_custom_shape_keeps_native_svg_colour() {
        // On a cockpit HUD canvas (`HC_HUD_*`) an entry-less colour-overlay custom
        // shape does NOT take the MFD `MissionObjectives` generic-icon default — it
        // keeps its SVG's native colours. The DRAK master-mode weapon icon
        // (`guns.svg`, authored all-white) renders WHITE in the in-game
        // `master_mode_display_master` reference, not the MFD MissionObjectives
        // yellow the power-screen system icons take.
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.HC_HUD_TestIconShapeDefault",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape_Icon",
                        "isActive": true,
                        "svgFill": {
                            "svgPath": "UI/Textures/Vector/General/CommonIcons/guns.svg",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "color": null
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 99.0},
                            "height": {"behavior": "Fixed", "value": 99.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-hud-icon-shape-default",
            Some("BuildingBlocks_Canvas.HC_HUD_TestIconShapeDefault"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|n| n.name == "shape_Icon")
            .expect("icon shape node");
        assert_eq!(
            node.icon_tint_colour_token.as_deref(),
            None,
            "a HUD entry-less colour-overlay custom shape keeps its native SVG colour (white)"
        );
    }

    #[test]
    fn compile_ir_style_tagged_textfield_inherits_brand_style_fill_colour() {
        // The medical menu option headers (OptionNameText, Heading2) carry
        // style tags with no colour semantics (UI_Generic_Flag_03); their
        // white comes from the bioc brand text-style table's H2 entry
        // (FillColor=Bright in TextFieldWidgetStandard). A style tag that
        // resolves no colour must not block the brand-style fallback.
        let standard_path =
            "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/widgets/textfieldwidgetstandard.json";
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TextFieldWidgetStandard",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "defaultStyles": {"entries": []},
                "brandStyles": [
                    {"brandIdentifier": "file://./styles/s_bioc.json",
                     "entries": [
                        {"_Type_": "BuildingBlocks_StyleEntry",
                         "name": "H2",
                         "conditionsList": [],
                         "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierNumber",
                             "field": "FontSize", "value": 30.0},
                            {"_Type_": "BuildingBlocks_FieldModifierColor",
                             "field": "FillColor",
                             "color": {"_Type_": "BuildingBlocks_ColorStyle",
                                       "color": "Bright", "alpha": 1.0}}
                         ],
                         "transitions": []}
                     ]}
                ],
                "scene": [],
                "operations": []
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(standard_path.to_string(), standard)]),
        };

        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestBrandStyleColourFallback",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "OptionNameText",
                        "isActive": true,
                        "text": "MEDICAL CARE",
                        "labelProperties": {"style": "Heading2"},
                        "styleTags": [
                            {"_RecordId_": "44444444-4444-4444-4444-444444444444",
                             "_RecordName_": "Tag.44444444-4444-4444-4444-444444444444"}
                        ],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-brand-style-colour",
            Some("BuildingBlocks_Canvas.TestBrandStyleColourFallback"),
            (100, 100),
            &defaults(),
            Some("canvas:s_bioc".to_string()),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|n| n.name == "OptionNameText")
            .expect("text node");
        assert_eq!(
            node.text_style.as_ref().and_then(|s| s.colour_token.as_deref()),
            Some("Bright"),
            "a colour-less style tag must not block the brand text-style FillColor fallback"
        );
    }

    #[test]
    fn compile_ir_hud_ship_component_resolves_hud_brand_typography() {
        // A cockpit HUD ship-component (HC_HUD_*, here the compass) authors the
        // manufacturer's HUD-typography brand (s_drak_hud) like the MFD masters,
        // but classify_canvas_family only routes MC_*/M_* to "hud". The standard
        // text-style brand selection must recognise the HUD family so an
        // uncoloured Heading1 label resolves s_drak_hud H1 (audimatmono-regular,
        // Accent2) — NOT s_drak_env H1 (audimatmono-bold, Bright). The compass
        // heading labels rendered bold-and-white under the env brand.
        let standard_path =
            "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/widgets/textfieldwidgetstandard.json";
        let h1 = |font: &str, fill: &str| {
            serde_json::json!({
                "_Type_": "BuildingBlocks_StyleEntry",
                "name": "H1",
                "conditionsList": [],
                "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierRecordRef",
                     "field": {"_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                               "value": format!("file://./../../../../../../../libs/foundry/records/ui/buildingblocks/fontstyles/{font}.json")}},
                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 45.0},
                    {"_Type_": "BuildingBlocks_FieldModifierColor", "field": "FillColor",
                     "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": fill, "alpha": 1.0}}
                ],
                "transitions": []
            })
        };
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TextFieldWidgetStandard",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "defaultStyles": {"entries": []},
                "brandStyles": [
                    {"brandIdentifier": "file://./styles/s_drak_env.json",
                     "entries": [h1("audimatmono-bold", "Bright")]},
                    {"brandIdentifier": "file://./styles/s_drak_hud.json",
                     "entries": [h1("audimatmono-regular", "Accent2")]}
                ],
                "scene": [],
                "operations": []
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(standard_path.to_string(), standard)]),
        };
        let label_canvas = |name: &str| {
            serde_json::json!({
                "_RecordName_": format!("BuildingBlocks_Canvas.{name}"),
                "_RecordValue_": {
                    "size": {"x": 100, "y": 100},
                    "scene": [{
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "text_Label",
                        "isActive": true,
                        "text": "320",
                        "labelProperties": {"style": "Heading1"},
                        "size": {"width": {"behavior": "Fixed", "value": 80.0},
                                 "height": {"behavior": "Fixed", "value": 20.0}}
                    }],
                    "operations": []
                }
            })
        };
        let compile_label = |name: &str| {
            let canvas = label_canvas(name);
            let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
            let ir = compile_ui_ir_from_scene(
                &scene, Some(&fetcher), "guid-hud-brand",
                Some(&format!("BuildingBlocks_Canvas.{name}")), (100, 100), &defaults(),
                Some("manufacturer:drak".to_string()), None, &[], Vec::new(), Vec::new(), 100,
            );
            let node = ir.nodes.iter().find(|n| n.name == "text_Label").expect("label").clone();
            let style = node.text_style.expect("text style");
            (style.colour_token.clone(), style.font_record.clone())
        };

        // HUD ship-component → s_drak_hud H1 (regular, Accent2).
        let (hud_colour, hud_font) = compile_label("HC_HUD_Ship_Compass_Master");
        assert_eq!(hud_colour.as_deref(), Some("Accent2"),
            "a HUD ship-component Heading1 label takes s_drak_hud Accent2, not env Bright");
        assert!(hud_font.as_deref().is_some_and(|f| f.contains("audimatmono-regular")),
            "a HUD ship-component Heading1 label takes the s_drak_hud regular font, got {hud_font:?}");

        // Control: a non-HUD (environment) canvas keeps s_drak_env H1 (bold, Bright).
        let (env_colour, env_font) = compile_label("SomeEnvironmentCanvas");
        assert_eq!(env_colour.as_deref(), Some("Bright"),
            "a non-HUD canvas keeps the s_drak_env Bright H1");
        assert!(env_font.as_deref().is_some_and(|f| f.contains("audimatmono-bold")),
            "a non-HUD canvas keeps the s_drak_env bold font, got {env_font:?}");
    }

    #[test]
    fn compile_ir_keeps_state_tagged_inactive_image_inactive() {
        // State visibility is entry-driven: the annunciator's "Show Glow in
        // online state" NotTag-gates the glow, so a Moderate chiclet's glow
        // stays authored-inactive. A tag-name-keyed activation arm flipped it
        // back on (WPN showed a gradient it must not have).
        let tag_db_path = "libs/foundry/records/tagdatabase/tagdatabase.tagdatabase.json";
        let state_tag = "33333333-3333-3333-3333-333333333333";
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(tag_db_path.to_string(), serde_json::json!({
                "_RecordId_": "66ee5bfc-d90b-41bd-ad2e-e0a2b3efe359",
                "_RecordName_": "TagDatabase.TagDatabase",
                "_RecordValue_": {
                    "_Type_": "TagDatabase",
                    "tags": [
                        {"_RecordId_": state_tag, "tagName": "StateModerate"}
                    ]
                }
            }))]),
        };

        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestStateInactiveImage",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "Image_Gradient",
                        "isActive": false,
                        "imagePath": "UI/Textures/H_HUDscreens/Ships/General/Annunciator_On.tif",
                        "styleTags": [
                            {"_RecordId_": state_tag,
                             "_RecordName_": format!("Tag.{state_tag}"),
                             "_RecordPath_": tag_db_path}
                        ],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-state-inactive",
            Some("BuildingBlocks_Canvas.TestStateInactiveImage"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|n| n.name == "Image_Gradient")
            .expect("image node");
        assert!(
            !node.is_active,
            "an authored-inactive image stays inactive unless a style entry activates it \
             — state tags alone are not a visibility signal"
        );
    }

    #[test]
    fn compile_ir_records_image_colour_overlay_flag() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestOverlayFlag",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "Image_Gradient",
                        "isActive": true,
                        "imagePath": "UI/Textures/H_HUDscreens/Ships/General/Annunciator_On.tif",
                        "svgFill": {"enableColorOverlay": true, "color": null},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-overlay",
            Some("BuildingBlocks_Canvas.TestOverlayFlag"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|n| n.name == "Image_Gradient")
            .expect("image node");
        assert!(
            node.colour_overlay_enabled,
            "the IR carries the svgFill colour-overlay flag for image widgets"
        );
    }

    #[test]
    fn collect_node_asset_refs_prefers_styled_image_path_over_authored() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestStyledAsset",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "image_BG",
                        "isActive": true,
                        "imagePath": "UI/Textures/I_InteractiveScreens/MFD/PU Generic/PU_MFD_generic_4x3_background_1.tif",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.values_mut().next().expect("node");
        node.raw.as_object_mut().expect("raw object").insert(
            "ImagePath".to_string(),
            serde_json::Value::String(
                "UI/Textures/H_HUDscreens/Ships/DRAK/DRAK_Background_anunciators.tif".to_string(),
            ),
        );

        let asset_refs = collect_node_asset_refs(node);
        assert_eq!(
            asset_refs.first().map(String::as_str),
            Some("UI/Textures/H_HUDscreens/Ships/DRAK/DRAK_Background_anunciators.tif"),
            "a brand-applied ImagePath must override the authored texture"
        );
    }

    #[test]
    fn collect_node_asset_refs_styled_empty_image_path_clears_authored_image() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestClearedAsset",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "image_BG",
                        "isActive": true,
                        "imagePath": "UI/Textures/I_InteractiveScreens/MFD/PU Generic/PU_MFD_generic_4x3_background_1.tif",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.values_mut().next().expect("node");
        node.raw.as_object_mut().expect("raw object").insert(
            "ImagePath".to_string(),
            serde_json::Value::String(String::new()),
        );

        let asset_refs = collect_node_asset_refs(node);
        assert!(
            asset_refs.is_empty(),
            "a styled empty ImagePath clears the authored texture (flat-fill brand variants), got {asset_refs:?}"
        );
    }

    #[test]
    fn compile_ir_emits_semantic_colour_tokens() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestColours",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "isActive": true,
                        "text": "HELLO",
                        "textColor": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Bright",
                            "alpha": 1.0
                        },
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Accent2",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetIcon",
                        "name": "icon",
                        "isActive": true,
                        "iconProperties": {
                            "customIcon": "UI/Textures/icon.svg",
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Accent3",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape",
                        "isActive": true,
                        "renderShape": true,
                        "svgPath": "UI/Textures/Vector/General/FingerPrint.svg",
                        "FillColor": {"r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-colours",
            Some("BuildingBlocks_Canvas.TestColours"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let label = ir.nodes.iter().find(|node| node.name == "label").expect("label node");
        assert_eq!(label.background_fill_colour, None);
        assert_eq!(label.background_fill_colour_token.as_deref(), Some("Accent2"));
        assert_eq!(
            label.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            Some("Bright")
        );

        let icon = ir.nodes.iter().find(|node| node.name == "icon").expect("icon node");
        assert_eq!(icon.icon_tint_colour, None);
        assert_eq!(icon.icon_tint_colour_token.as_deref(), Some("Accent3"));

        let shape = ir.nodes.iter().find(|node| node.name == "shape").expect("shape node");
        assert_eq!(shape.icon_tint_colour, Some([0.25, 0.5, 0.75, 1.0]));
    }

    #[test]
    fn compile_ir_button_icon_placeholder_black_inherits_sibling_text_colour() {
        // The modular-kit button sheet styles the text field and the caret icon
        // as separate elements. sk_uilo_a is the lone `uilo` button kit that
        // authors the icon `FillColor` as a ColorSolid pure black (an editor
        // placeholder) while every sibling kit authors ColorStyle(Background) —
        // the same button-content role the text field uses. A ColorSolid literal
        // resolves to a token-less RGBA (the cascade writes `{r,g,b,a}` black and
        // drops `FillColorToken`), so the icon would otherwise fall through to the
        // SVG's native (unfilled → black) colour. The engine treats the
        // placeholder as unset: a button icon carrying ONLY placeholder black
        // inherits its sibling text field's colour token.
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestButtonIconColour",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "button_root",
                        "isActive": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 30.0}
                        },
                        "children": ["ptr:2", "ptr:3"]
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetIcon",
                        "name": "caret",
                        "isActive": true,
                        "parent": "_PointsTo_:ptr:1",
                        "iconProperties": {
                            "customIcon": "UI/Textures/Vector/arrow_carat_double_up.svg"
                        },
                        // Post-cascade shape of a ColorSolid-black FillColor:
                        // token-less float RGBA (see write_color_to_raw), no
                        // FillColorToken.
                        "FillColor": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "isActive": true,
                        "parent": "_PointsTo_:ptr:1",
                        "text": "CALL ELEVATOR",
                        // Post-cascade shape of a resolved ColorStyle(Background):
                        // RGBA plus the surviving role token.
                        "FillColor": {"r": 0.1, "g": 0.15, "b": 0.1, "a": 1.0},
                        "FillColorToken": "Background",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 60.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-button-icon-colour",
            Some("BuildingBlocks_Canvas.TestButtonIconColour"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let label = ir
            .nodes
            .iter()
            .find(|node| node.name == "label")
            .expect("label node");
        assert_eq!(
            label.icon_tint_colour_token.as_deref(),
            Some("Background"),
            "the sibling text field carries the button-content colour token"
        );

        let caret = ir
            .nodes
            .iter()
            .find(|node| node.name == "caret")
            .expect("caret node");
        assert_eq!(
            caret.icon_tint_colour, None,
            "placeholder black is not a real authored tint"
        );
        assert_eq!(
            caret.icon_tint_colour_token.as_deref(),
            Some("Background"),
            "placeholder-black button icon inherits the sibling text field's colour token"
        );
    }

    #[test]
    fn compile_ir_primary_state_tag_is_not_a_colour_directive() {
        let tag_db_path = "file://tagdatabase.tagdatabase.json";
        let primary_tag_id = "primary-tag-id";
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestPrimaryTextColourTag",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "primary_title",
                        "isActive": true,
                        "text": "DIGITAL MEDICAL ASSISTANT",
                        "labelProperties": {
                            "style": "Title3"
                        },
                        "styleTags": [
                            {
                                "_RecordPath_": tag_db_path,
                                "_RecordName_": "Tag.primary-tag-id",
                                "_RecordId_": primary_tag_id
                            }
                        ],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "primary_heading",
                        "isActive": true,
                        "text": "VIEW CURRENT STATUS",
                        "labelProperties": {
                            "style": "Heading6"
                        },
                        "styleTags": [
                            {
                                "_RecordPath_": tag_db_path,
                                "_RecordName_": "Tag.primary-tag-id",
                                "_RecordId_": primary_tag_id
                            }
                        ],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });
        let tag_db = serde_json::json!({
            "_RecordName_": "TagDatabase",
            "_RecordValue_": {
                "tags": [
                    {
                        "_RecordId_": primary_tag_id,
                        "_RecordName_": "Tag.primary-tag-id",
                        "_Type_": "Tag",
                        "tagName": "Primary",
                        "children": []
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(tag_db_path.to_string(), tag_db)].into_iter().collect(),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-primary-text-colour-tag",
            Some("BuildingBlocks_Canvas.TestPrimaryTextColourTag"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        // The colourless `Primary` state tag is NOT a colour directive: the
        // former Primary→Base arm was deleted 2026-06-12 (with the
        // text-format style route landed, the in-game Medical2 title's Base
        // light-blue is entry/brand-driven; disabling the arm drifts no
        // frozen pin). A bare Title-style node with only the Primary tag has
        // NO derived colour token — it falls to the brand text style at
        // render.
        let node = ir.nodes.iter().find(|node| node.name == "primary_title").expect("primary title node");
        assert_eq!(
            node.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            None
        );
        assert_eq!(node.resolved_style_tags.first().and_then(|tag| tag.tag_name.as_deref()), Some("Primary"));

        let heading = ir.nodes.iter().find(|node| node.name == "primary_heading").expect("primary heading node");
        assert_eq!(
            heading.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            None
        );
        assert_eq!(heading.resolved_style_tags.first().and_then(|tag| tag.tag_name.as_deref()), Some("Primary"));
    }

    #[test]
    fn compile_ir_untagged_heading1_text_has_no_derived_colour_token() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestHeadingBase",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "untagged_heading",
                        "isActive": true,
                        "text": "VIEW CURRENT STATUS",
                        "labelProperties": {
                            "style": "Heading1"
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-heading-base",
            Some("BuildingBlocks_Canvas.TestHeadingBase"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let heading = ir.nodes.iter().find(|node| node.name == "untagged_heading").expect("heading node");
        // The untagged-Heading1→Base semantic guess was DELETED 2026-06-12
        // (Phase 3 audit: entry-driven colours cover every frozen pin; an
        // untagged heading falls to the brand text style at render).
        assert_eq!(
            heading.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            None
        );
    }

    #[test]
    fn compile_ir_maps_ui_generic_flag_03_to_foreground_token() {
        let tag_db_path = "file://tagdatabase.tagdatabase.json";
        let emphasis_tag_id = "emphasis-tag-id";
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestEmphasisTextColourTag",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "emphasis_heading",
                        "isActive": true,
                        "text": "MODE",
                        "labelProperties": {
                            "style": "Heading6"
                        },
                        "styleTags": [
                            {
                                "_RecordPath_": tag_db_path,
                                "_RecordName_": "Tag.emphasis-tag-id",
                                "_RecordId_": emphasis_tag_id
                            }
                        ],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });
        let tag_db = serde_json::json!({
            "_RecordName_": "TagDatabase",
            "_RecordValue_": {
                "tags": [
                    {
                        "_RecordId_": emphasis_tag_id,
                        "_RecordName_": "Tag.emphasis-tag-id",
                        "_Type_": "Tag",
                        "tagName": "UI_Generic_Flag_03",
                        "children": []
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(tag_db_path.to_string(), tag_db)].into_iter().collect(),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-emphasis-text-colour-tag",
            Some("BuildingBlocks_Canvas.TestEmphasisTextColourTag"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "emphasis_heading").expect("emphasis node");
        // `UI_Generic_Flag_03` is NOT a colour signal: the power OUTPUT card's
        // "2" carries it and renders cream/Base in-game (no DRAK entry maps
        // the flag to a colour; the only flag-conditioned entry anywhere near
        // is a GRIN geometric one). Text colour comes from the brand text
        // style / authored entries, never from this tag.
        assert_ne!(
            node.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            Some("Foreground"),
            "the flag tag alone must not force a Foreground directive"
        );
    }

    #[test]
    fn compile_ir_maps_custom_shape_modify_tag_to_accent5_icon_tint_token() {
        let tag_db_path = "file://tagdatabase.tagdatabase.json";
        let modify_tag_id = "modify-tag-id";
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestCustomShapeModifyTag",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "modify_shape",
                        "isActive": true,
                        "svgFill": {
                            "svgPath": "UI/Textures/Vector/General/FingerPrint.svg",
                            "renderShape": true,
                            "enableColorOverlay": true
                        },
                        "styleTags": [
                            {
                                "_RecordPath_": tag_db_path,
                                "_RecordName_": "Tag.modify-tag-id",
                                "_RecordId_": modify_tag_id
                            }
                        ],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 40.0}
                        }
                    }
                ],
                "operations": []
            }
        });
        let tag_db = serde_json::json!({
            "_RecordName_": "TagDatabase",
            "_RecordValue_": {
                "tags": [
                    {
                        "_RecordId_": modify_tag_id,
                        "_RecordName_": "Tag.modify-tag-id",
                        "_Type_": "Tag",
                        "tagName": "Modify",
                        "children": []
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(tag_db_path.to_string(), tag_db)].into_iter().collect(),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-custom-shape-modify-tag",
            Some("BuildingBlocks_Canvas.TestCustomShapeModifyTag"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "modify_shape").expect("modify shape node");
        assert_eq!(node.icon_tint_colour_token.as_deref(), Some("Accent5"));
        assert_eq!(node.colour_blend_mode, Some(crate::ui_ir::UiIrColourBlendMode::Additive));
    }

    #[test]
    fn compile_ir_maps_footer_logo_and_bar_style_tags_to_accent1_icon_tint_token() {
        let tag_db_path = "file://tagdatabase.tagdatabase.json";
        let logo_tag_id = "logo-tag-id";
        let animate_tag_id = "animate8-tag-id";
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestFooterTintTags",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetManufacturerLogo",
                        "name": "Logo",
                        "styleTags": [{
                            "_RecordPath_": tag_db_path,
                            "_RecordName_": "Tag.logo-tag-id",
                            "_RecordId_": logo_tag_id
                        }],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "Image_bottom_bar",
                        "styleTags": [{
                            "_RecordPath_": tag_db_path,
                            "_RecordName_": "Tag.animate8-tag-id",
                            "_RecordId_": animate_tag_id
                        }],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });
        let tag_db = serde_json::json!({
            "_RecordName_": "TagDatabase",
            "_RecordValue_": {
                "tags": [
                    {
                        "_RecordId_": logo_tag_id,
                        "_RecordName_": "Tag.logo-tag-id",
                        "_Type_": "Tag",
                        "tagName": "UI_Generic_Flag_01",
                        "children": []
                    },
                    {
                        "_RecordId_": animate_tag_id,
                        "_RecordName_": "Tag.animate8-tag-id",
                        "_Type_": "Tag",
                        "tagName": "Animate_8",
                        "children": []
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(tag_db_path.to_string(), tag_db)].into_iter().collect(),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-footer-tint-tags",
            Some("BuildingBlocks_Canvas.TestFooterTintTags"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let logo = ir.nodes.iter().find(|node| node.name == "Logo").expect("logo node");
        assert_eq!(logo.icon_tint_colour_token.as_deref(), Some("Accent1"));

        let footer = ir
            .nodes
            .iter()
            .find(|node| node.name == "Image_bottom_bar")
            .expect("bottom bar node");
        assert_eq!(footer.icon_tint_colour_token.as_deref(), Some("Accent1"));
    }

    #[test]
    fn enable_background_false_suppresses_background_fill_even_with_background_color_present() {
        // Regression: DRAK brand style "Page Greebles Notarget Right" sets both
        // EnableBackground: false AND BackgroundColor: rgba(255,255,255,50) on
        // an image node. The explicit disable must win — background_fill_colour
        // should be None in the compiled IR.
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestEnableBackground",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "image_WithDisabledBackground",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        },
                        "EnableBackground": false,
                        "BackgroundColor": {
                            "r": 1.0, "g": 1.0, "b": 1.0, "a": 0.196
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-enable-background-test",
            Some("BuildingBlocks_Canvas.TestEnableBackground"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|n| n.name == "image_WithDisabledBackground")
            .expect("image node");
        assert!(
            node.background_fill_colour.is_none(),
            "EnableBackground: false must suppress background_fill_colour (was {:?})",
            node.background_fill_colour
        );
    }

    #[test]
    fn brand_text_style_fillcolor_is_authoritative_caption_role() {
        // A standard text-style entry's `FillColor` modifier is the game's own
        // colour role for that named style (e.g. `Heading6`/`H6` -> `Bright`).
        // `brand_text_style_colour_token` exposes it so a caption value resolves
        // to its real role instead of defaulting to white.
        let entry = serde_json::json!({
            "name": "H6",
            "modifiers": [
                { "field": "FontSize", "value": 21.0 },
                {
                    "field": "FillColor",
                    "color": {
                        "_Type_": "BuildingBlocks_ColorStyle",
                        "color": "Bright",
                        "alpha": 1.0
                    }
                }
            ]
        });
        let style = standard_text_style_from_entry(&entry);
        assert_eq!(style.fill_colour_token.as_deref(), Some("Bright"));

        let mut styles = std::collections::HashMap::new();
        styles.insert("H6".to_string(), style);
        assert_eq!(
            brand_text_style_colour_token(Some("Heading6"), &styles).as_deref(),
            Some("Bright")
        );
        // A style with no entry (no authored FillColor) yields no token.
        assert_eq!(brand_text_style_colour_token(Some("Heading3"), &styles), None);
    }

    #[test]
    fn title_node_with_explicit_bright_tag_keeps_bright_even_when_primary_tag_also_present() {
        let tag_db_path = "file://tagdatabase.tagdatabase.json";
        let bright_tag_id = "bright-tag-id";
        let primary_tag_id = "primary-tag-id";
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TierBrightPrimaryConflict",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "tier_level",
                        "isActive": true,
                        "text": "T3",
                        "labelProperties": {"style": "Title3"},
                        "styleTags": [
                            {"_RecordPath_": tag_db_path, "_RecordName_": "Tag.bright-tag-id", "_RecordId_": bright_tag_id},
                            {"_RecordPath_": tag_db_path, "_RecordName_": "Tag.primary-tag-id", "_RecordId_": primary_tag_id}
                        ],
                        "size": {"width": {"behavior": "Fixed", "value": 40.0}, "height": {"behavior": "Fixed", "value": 40.0}}
                    }
                ],
                "operations": []
            }
        });
        let tag_db = serde_json::json!({
            "_RecordName_": "TagDatabase",
            "_RecordValue_": {
                "tags": [
                    {"_RecordId_": bright_tag_id, "_RecordName_": "Tag.bright-tag-id", "_Type_": "Tag", "tagName": "Bright", "children": []},
                    {"_RecordId_": primary_tag_id, "_RecordName_": "Tag.primary-tag-id", "_Type_": "Tag", "tagName": "Primary", "children": []}
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(tag_db_path.to_string(), tag_db)].into_iter().collect(),
        };
        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-tier-bright-primary",
            Some("BuildingBlocks_Canvas.TierBrightPrimaryConflict"),
            (100, 100),
            &defaults(),
            None, None, &[],
            Vec::new(), Vec::new(),
            100,
        );
        // An explicit `Bright` colour tag on a Title-style node must win over the state
        // tag `Primary` → `Base` mapping. TierLevel nodes carry Bright to force white
        // rendering; the Primary tag is a state marker, not a colour override.
        let node = ir.nodes.iter().find(|n| n.name == "tier_level").expect("tier_level node");
        assert_eq!(
            node.text_style.as_ref().and_then(|s| s.colour_token.as_deref()),
            Some("Bright"),
            "explicit Bright tag must override Primary→Base for Title-style nodes"
        );
    }


}

#[cfg(test)]
mod tests_b {
    #![allow(unused_imports, dead_code)]

    use super::*;

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }


    #[test]
    fn compile_ir_resolves_standard_textfield_line_spacing_for_label_style() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestStandardLineSpacing",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "title",
                        "isActive": true,
                        "text": "DIGITAL MEDICAL ASSISTANT",
                        "FontSize": 100.0,
                        "labelProperties": {
                            "style": "Title3",
                            "anchorToParentX": 0.5,
                            "anchorToParentY": 0.5
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 40.0}
                        }
                    }
                ],
                "operations": []
            }
        });
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TextFieldWidgetStandard",
            "_RecordValue_": {
                "defaultStyles": {
                    "entries": [
                        {
                            "name": "T3",
                            "modifiers": [
                                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 115.0},
                                {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "LineSpacing", "value": -55.0}
                            ]
                        }
                    ]
                },
                "brandStyles": [
                    {
                        "brandIdentifier": "file://./styles/s_bioc.json",
                        "entries": [
                            {
                                "name": "T3",
                                "modifiers": [
                                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 115.0},
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierRecordRef",
                                        "field": {
                                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                                            "value": "file://./fontstyles/blenderpro-thin.json"
                                        }
                                    },
                                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "LineSpacing", "value": -35.0}
                                ]
                            }
                        ]
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(standard_text_field_widget_path().to_string(), standard)]
                .into_iter()
                .collect(),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-standard-line-spacing",
            Some("BuildingBlocks_Canvas.TestStandardLineSpacing"),
            (100, 100),
            &defaults(),
            Some("canvas:s_bioc".to_string()),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "title").expect("title node");
        assert_eq!(
            node.text_style.as_ref().and_then(|style| style.line_spacing),
            Some(-35.0)
        );
        assert_eq!(
            node.text_style.as_ref().and_then(|style| style.font_record.as_deref()),
            Some("file://./fontstyles/blenderpro-thin.json")
        );
    }

    #[test]
    fn compile_ir_uses_standard_heading_font_size_for_unsized_textfield() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestStandardHeadingSize",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "location",
                        "isActive": true,
                        "text": "Drake Clipper",
                        "labelProperties": {
                            "style": "Heading3",
                            "caseModifier": "None",
                            "anchorToParentX": 0.5,
                            "anchorToParentY": 0.5
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 40.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "machine_type",
                        "isActive": true,
                        "text": "MEDICAL ASSISTANT",
                        "labelProperties": {
                            "style": "Heading1",
                            "caseModifier": "Upper",
                            "anchorToParentX": 0.5,
                            "anchorToParentY": 0.5
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 40.0}
                        }
                    }
                ],
                "operations": []
            }
        });
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TextFieldWidgetStandard",
            "_RecordValue_": {
                "defaultStyles": {"entries": []},
                "brandStyles": [
                    {
                        "brandIdentifier": "file://./styles/s_bioc.json",
                        "entries": [
                            {
                                "name": "H3",
                                "modifiers": [
                                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 30.0},
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierRecordRef",
                                        "field": {
                                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                                            "value": "file://./fontstyles/blenderpro-thin.json"
                                        }
                                    },
                                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "LineSpacing", "value": 0.0}
                                ]
                            },
                            {
                                "name": "H1",
                                "modifiers": [
                                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 57.0},
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierRecordRef",
                                        "field": {
                                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                                            "value": "file://./fontstyles/blenderpro-thin.json"
                                        }
                                    },
                                    {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "LineSpacing", "value": 0.0}
                                ]
                            }
                        ]
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(standard_text_field_widget_path().to_string(), standard)]
                .into_iter()
                .collect(),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-standard-heading-size",
            Some("BuildingBlocks_Canvas.TestStandardHeadingSize"),
            (100, 100),
            &defaults(),
            Some("canvas:s_bioc".to_string()),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        // Data-backed model: a styled textfield renders at its named style's authored
        // brand FontSize verbatim (no nominal-scale constant). `caseModifier: None`, so the
        // H3 brand FontSize = 30 is used exactly (no caps reduction).
        let node = ir.nodes.iter().find(|node| node.name == "location").expect("location node");
        assert_eq!(
            node.text_style.as_ref().map(|style| &style.font_size),
            Some(&UiIrValue::Fixed { value: 30.0 })
        );

        // H1 brand FontSize = 57, used verbatim. `caseModifier: Upper` no longer applies
        // an all-caps size reduction (the ~0.98 fudge was removed), so the size is 57.
        let node = ir.nodes.iter().find(|node| node.name == "machine_type").expect("machine_type node");
        let style = node.text_style.as_ref().expect("machine_type text style");
        assert!(
            matches!(style.font_size, UiIrValue::Fixed { value } if (value - 57.0).abs() < 0.02),
            "caseModifier Upper uses full H1 57 (no caps reduction): {:?}",
            style.font_size,
        );
    }

    #[test]
    fn compile_ir_sizes_relative_height_textfield_to_its_field_not_the_named_default() {
        // A text field that authors a RELATIVE (Percent) HEIGHT with no FontSize
        // sizes its glyph to fill that field, NOT the named-style table default.
        // The compass tick labels author `height: 0.8` of the tick (no FontSize):
        // they fill the tick (cap ≈ field height) rather than the Heading1 60.
        // A sibling with the SAME style but a FIXED height keeps the named size.
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestRelativeHeightFontSize",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "tick_label",
                        "isActive": true,
                        "text": "120",
                        "labelProperties": {"style": "Heading1", "caseModifier": "None"},
                        "sizing": {
                            "width": {"behavior": "PercentOfY", "value": 50.0},
                            "height": {"behavior": "Percent", "value": 0.8}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "fixed_label",
                        "isActive": true,
                        "text": "120",
                        "labelProperties": {"style": "Heading1", "caseModifier": "None"},
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Percent", "value": 0.8}
                        }
                    }
                ],
                "operations": []
            }
        });
        let font_path =
            "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/fontstyles/test-imagepercent.json";
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TextFieldWidgetStandard",
            "_RecordValue_": {
                "defaultStyles": {"entries": []},
                "brandStyles": [{
                    "brandIdentifier": "file://./styles/s_bioc.json",
                    "entries": [{
                        "name": "H1",
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 57.0},
                            {"_Type_": "BuildingBlocks_FieldModifierRecordRef",
                             "field": {"_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                                       "value": font_path}}
                        ]
                    }]
                }]
            }
        });
        // The font carries imageSizePercent 0.75 (like AudimatMono). A PLAIN authored
        // size is divided by it (the slug image is 0.75 of the nominal); the
        // height-driven size is the field EM (verbatim), so it must NOT be divided —
        // boosting it ÷0.75 over-sized the compass labels (80 → 107) and clipped them.
        let font_record = serde_json::json!({
            "_RecordName_": "BuildingBlocks_FontStyle.TestImagePercent",
            "_RecordValue_": {"_Type_": "BuildingBlocks_FontStyle", "font": "$Low-Reg",
                              "imageSizePercent": 0.75}
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [
                (standard_text_field_widget_path().to_string(), standard),
                (font_path.to_string(), font_record),
            ]
            .into_iter()
            .collect(),
        };
        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-relative-height-font",
            Some("BuildingBlocks_Canvas.TestRelativeHeightFontSize"),
            (100, 100),
            &defaults(),
            Some("canvas:s_bioc".to_string()),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        // PercentOfY width + Percent height 0.8 of the 100px canvas → field 80px →
        // height-driven label fills it (font ≈ 80), not the named H1 60/57.
        let tick = ir.nodes.iter().find(|node| node.name == "tick_label").expect("tick_label");
        let tick_size = tick.text_style.as_ref().expect("tick text style").font_size.clone();
        assert!(
            matches!(tick_size, UiIrValue::Fixed { value } if (value - 80.0).abs() < 1.0),
            "PercentOfY-width relative-height label sizes its font to the 80px field, got {tick_size:?}"
        );

        // The sibling has the SAME Percent height but a FIXED width (the medical-bed
        // case, not height-driven) → it keeps the named H1 size 57, unaffected.
        let fixed = ir.nodes.iter().find(|node| node.name == "fixed_label").expect("fixed_label");
        let fixed_size = fixed.text_style.as_ref().expect("fixed text style").font_size.clone();
        assert!(
            matches!(fixed_size, UiIrValue::Fixed { value } if (value - 57.0).abs() < 0.02),
            "fixed-height label keeps the named-style 57, got {fixed_size:?}"
        );
    }

    #[test]
    fn compile_ir_uses_standard_title4_font_size_before_scene_style_borrow() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestStandardTitleSize",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "small_title_reference",
                        "isActive": true,
                        "text": "SMALL",
                        "FontSize": 40.0,
                        "labelProperties": {"style": "Title4"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 40.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "standard_title",
                        "isActive": true,
                        "text": "T3",
                        "labelProperties": {"style": "Title4"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 40.0}
                        }
                    }
                ],
                "operations": []
            }
        });
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TextFieldWidgetStandard",
            "_RecordValue_": {
                "defaultStyles": {"entries": [
                    {
                        "name": "Title4",
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 40.0},
                            {
                                "_Type_": "BuildingBlocks_FieldModifierRecordRef",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                                    "value": "file://./fontstyles/blenderpro-thin.json"
                                }
                            }
                        ]
                    },
                    {
                        "name": "T4",
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": 75.0},
                            {
                                "_Type_": "BuildingBlocks_FieldModifierRecordRef",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord",
                                    "value": "file://./fontstyles/blenderpro-medium.json"
                                }
                            }
                        ]
                    }
                ]},
                "brandStyles": []
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: [(standard_text_field_widget_path().to_string(), standard)]
                .into_iter()
                .collect(),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-standard-title-size",
            Some("BuildingBlocks_Canvas.TestStandardTitleSize"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        // Data-backed model: the named style's brand FontSize is used verbatim. The
        // `Title4` style maps to the `T4` brand entry (FontSize 75).
        let node = ir
            .nodes
            .iter()
            .find(|node| node.name == "standard_title")
            .expect("standard title node");
        assert_eq!(
            node.text_style.as_ref().map(|style| &style.font_size),
            Some(&UiIrValue::Fixed { value: 75.0 })
        );
    }

    #[test]
    fn short_heading1_uses_header_size_while_long_prompt_stays_shallow() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, Vec2, Vec3};

        let textfield = |style: &str| BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetTextField,
            name: "text".to_string(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing::default(),
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({"labelProperties": {"style": style}}),
        };

        // The per-style rect ladder was deleted (Phase 2/3 audit) — fixture
        // kept only to pin that the helper builder still parses.
        let _ = textfield("Heading1");
    }

    #[test]
    fn compile_ir_limits_style_palette_text_default_to_label_caption() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestStylePaletteTextColour",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "palette_text",
                        "isActive": true,
                        "text": "PATIENT NAME",
                        "__BrandIdentifier": "s_bioc",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_ComponentLabelCaptionPair",
                        "name": "palette_pair",
                        "isActive": true,
                        "__BrandIdentifier": "s_bioc",
                        "labelProperties": {
                            "label": "PATIENT NAME",
                            "style": "Heading3"
                        },
                        "alignment": "Left",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-style-palette-text-colour",
            Some("BuildingBlocks_Canvas.TestStylePaletteTextColour"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "palette_text").expect("palette text node");
        assert_eq!(
            node.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            None
        );

        let pair = ir.nodes.iter().find(|node| node.name == "palette_pair").expect("palette pair node");
        assert_eq!(
            pair.text_style.as_ref().and_then(|style| style.colour_token.as_deref()),
            Some("Base")
        );
    }
}

#[cfg(test)]
mod tests_c {
    #![allow(unused_imports, dead_code)]

    use super::*;

    #[test]
    fn portrait_font_screen_scale_scales_only_portrait_fill_screens() {
        // SQUARE gauge mesh (g-force/velocity ball, countermeasures): a 1920×1080
        // canvas filled to a square 1.0 target -> NOT portrait -> 1.0 (unchanged).
        assert!(
            (portrait_font_screen_scale((1920.0, 1080.0), (1920, 1920), true, true) - 1.0).abs()
                < 1e-4,
            "square-mesh gauge keeps font scale 1.0"
        );
        // PORTRAIT LR-indicator mesh (0.64): 1920×1080 canvas filled to a 1920×3000
        // target -> scale by the vertical fill axis sy = 3000/1080.
        assert!(
            (portrait_font_screen_scale((1920.0, 1080.0), (1920, 3000), true, true)
                - (3000.0 / 1080.0))
                .abs()
                < 1e-3,
            "portrait-mesh screen scales font by the vertical fill axis"
        );
        // NON-fill screen (e.g. an MFD / non-cockpit) stays 1.0 even on a portrait
        // target — the scale is scoped to the cockpit mesh-aspect fill branch.
        assert!(
            (portrait_font_screen_scale((1920.0, 1080.0), (1080, 1920), false, false) - 1.0).abs()
                < 1e-4,
            "non-fill screen keeps font scale 1.0"
        );
    }

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }


    #[test]
    fn compile_ir_keeps_asset_fill_colour_as_tint_not_background() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAssetFillTint",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "bottom_bar",
                        "isActive": true,
                        "ImagePath": "UI/Textures/I_InteractiveScreens/Med/i_med_bioc_bottom-bar.tif",
                        "FillColor": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Base",
                            "alpha": 1.0
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-asset-fill-tint",
            Some("BuildingBlocks_Canvas.TestAssetFillTint"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "bottom_bar").expect("bottom bar node");
        assert_eq!(node.background_fill_colour, None);
        assert_eq!(node.background_fill_colour_token.as_deref(), None);
        assert_eq!(node.icon_tint_colour_token.as_deref(), Some("Base"));
    }

    #[test]
    fn compile_ir_does_not_draw_fill_colour_on_disabled_background_containers() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestDisabledContainerFillTint",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "card_root",
                        "isActive": true,
                        "background": {
                            "enable": false,
                            "color": null
                        },
                        "FillColor": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Bright",
                            "alpha": 1.0
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "disabled_border_strip",
                        "isActive": true,
                        "background": {
                            "enable": false,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Base",
                                "alpha": 0.2
                            }
                        },
                        "svgFill": {
                            "svgPath": "UI/Textures/Vector/Drake/Hazardlines_header1.svg",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Base",
                                "alpha": 0.2
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "enabled_panel",
                        "isActive": true,
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Bright",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-disabled-container-fill-tint",
            Some("BuildingBlocks_Canvas.TestDisabledContainerFillTint"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let card_root = ir.nodes.iter().find(|node| node.name == "card_root").expect("card root node");
        assert_eq!(card_root.background_fill_colour, None);
        assert_eq!(card_root.background_fill_colour_token, None);

        let disabled_border_strip = ir
            .nodes
            .iter()
            .find(|node| node.name == "disabled_border_strip")
            .expect("disabled border strip node");
        assert_eq!(disabled_border_strip.background_fill_colour, None);
        assert_eq!(disabled_border_strip.background_fill_colour_token, None);
        assert_eq!(disabled_border_strip.background_fill_alpha, None);

        let enabled_panel = ir.nodes.iter().find(|node| node.name == "enabled_panel").expect("enabled panel node");
        assert_eq!(enabled_panel.background_fill_colour, None);
        assert_eq!(enabled_panel.background_fill_colour_token.as_deref(), Some("Bright"));
    }

    #[test]
    fn compile_ir_does_not_draw_background_on_renderertype_none_container() {
        // A DisplayWidget with `rendererType: "None"` is a non-rendering proxy /
        // group node (e.g. the compass `CanvasProxyRoot`). Even when it authors an
        // ENABLED white background, the engine never paints it — only nodes with a
        // primitive renderer (`rendererType: "Flash"`, like the compass centre line
        // and tick shapes) draw their background. Without this gate the compass
        // screen rendered an opaque white sheet over its dark vignette.
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestRendererTypeNoneBackground",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "proxy_root",
                        "isActive": true,
                        "rendererType": "None",
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorSolid",
                                "color": {"_Type_": "SRGBA8", "r": 255, "g": 255, "b": 255, "a": 255}
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "flash_line",
                        "isActive": true,
                        "rendererType": "Flash",
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorSolid",
                                "color": {"_Type_": "SRGBA8", "r": 255, "g": 255, "b": 255, "a": 255}
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 10.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-renderertype-none-background",
            Some("BuildingBlocks_Canvas.TestRendererTypeNoneBackground"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let proxy = ir.nodes.iter().find(|node| node.name == "proxy_root").expect("proxy node");
        assert_eq!(
            proxy.background_fill_colour, None,
            "rendererType:None container must not paint its background (was {:?})",
            proxy.background_fill_colour
        );
        assert_eq!(proxy.background_fill_colour_token, None);

        let flash = ir.nodes.iter().find(|node| node.name == "flash_line").expect("flash node");
        assert_eq!(
            flash.background_fill_colour,
            Some([1.0, 1.0, 1.0, 1.0]),
            "rendererType:Flash node keeps its authored white background"
        );
    }

    #[test]
    fn compile_ir_does_not_draw_background_on_renderertype_primitive_card() {
        // A WidgetCard with `rendererType: "Primitive"` is rendered by a 3D
        // primitive material (its empty `primitiveMaterialPath` draws nothing),
        // NOT the Flash UI background — so the engine paints no background even
        // when one is authored ENABLED. The DRAK master-mode display authors
        // `card_Icon` / `card_CurrentModeText` exactly this way (white a60), and
        // the in-game `master_mode_display_master` reference shows NO grey box
        // behind the bullets / SCM. A `WidgetRuntimeImage` (the SELF-STATUS
        // hologram) is ALSO `Primitive` but the hologram compositor reuses its
        // `background_fill_colour` as the holo tint, so it keeps its background.
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestRendererTypePrimitiveBackground",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCard",
                        "name": "primitive_card",
                        "isActive": true,
                        "rendererType": "Primitive",
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorSolid",
                                "color": {"_Type_": "SRGBA8", "r": 254, "g": 255, "b": 255, "a": 60}
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetRuntimeImage",
                        "name": "holo_image",
                        "isActive": true,
                        "rendererType": "Primitive",
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorSolid",
                                "color": {"_Type_": "SRGBA8", "r": 100, "g": 150, "b": 200, "a": 255}
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "primitive_bar_fill",
                        "isActive": true,
                        "rendererType": "Primitive",
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Base",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-renderertype-primitive-background",
            Some("BuildingBlocks_Canvas.TestRendererTypePrimitiveBackground"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let card = ir.nodes.iter().find(|node| node.name == "primitive_card").expect("card node");
        assert_eq!(
            card.background_fill_colour, None,
            "rendererType:Primitive WidgetCard must not paint its background (was {:?})",
            card.background_fill_colour
        );

        let holo = ir.nodes.iter().find(|node| node.name == "holo_image").expect("holo node");
        assert!(
            holo.background_fill_colour.is_some(),
            "rendererType:Primitive WidgetRuntimeImage keeps its background (holo tint)"
        );

        let bar = ir.nodes.iter().find(|node| node.name == "primitive_bar_fill").expect("bar node");
        assert_eq!(
            bar.background_fill_colour_token.as_deref(),
            Some("Base"),
            "rendererType:Primitive node with a ColorStyle (palette-role) fill renders (power-bar PipBox_Fill)"
        );
    }

    #[test]
    fn compile_ir_draws_explicit_background_color_on_disabled_base_background() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestExplicitBackgroundColor",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "style_background_panel",
                        "isActive": true,
                        "background": {
                            "enable": false,
                            "color": null
                        },
                        "BackgroundColor": {"r": 0.015, "g": 0.031, "b": 0.09, "a": 0.5},
                        "BackgroundColorToken": "Background",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-explicit-background-color",
            Some("BuildingBlocks_Canvas.TestExplicitBackgroundColor"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let panel = ir.nodes.iter().find(|node| node.name == "style_background_panel").expect("panel node");
        assert_eq!(panel.background_fill_colour, Some([0.015, 0.031, 0.09, 0.5]));
        assert_eq!(panel.background_fill_colour_token.as_deref(), Some("Background"));
    }

    #[test]
    fn compile_ir_marks_authored_accent_backgrounds_as_additive() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAccentBackgroundBlend",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "accent_panel",
                        "isActive": true,
                        "background": {
                            "enable": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Accent1",
                                "alpha": 1.0
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-accent-background-blend",
            Some("BuildingBlocks_Canvas.TestAccentBackgroundBlend"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let panel = ir.nodes.iter().find(|node| node.name == "accent_panel").expect("panel node");
        assert_eq!(panel.background_fill_colour_token.as_deref(), Some("Accent1"));
        assert_eq!(panel.colour_blend_mode, Some(UiIrColourBlendMode::Additive));
    }

    #[test]
    fn compile_ir_uses_representative_animation_alpha_for_static_render() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAnimationStart",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCircle",
                        "name": "pulse",
                        "isActive": true,
                        "alpha": 1.0,
                        "strokeExtent": 3.0,
                        "animation": {
                            "animationTimeline": {
                                "keyframes": [
                                    {
                                        "percent": 0.0,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            },
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "SizeX",
                                                    "value": 0.44
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.4,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 1.0
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.8,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            },
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "SizeX",
                                                    "value": 1.33
                                                }
                                            }
                                        ]
                                    }
                                ]
                            },
                            "loopIndefinitely": true
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "isActive": true,
                        "alpha": 1.0,
                        "text": "TOUCH TO START",
                        "animation": {
                            "animationTimeline": {
                                "keyframes": [
                                    {
                                        "percent": 0.0,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.33,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.66
                                                }
                                            }
                                        ]
                                    },
                                    {
                                        "percent": 0.99,
                                        "modifiers": [
                                            {
                                                "modifier": {
                                                    "_Type_": "BuildingBlocks_FieldModifierNumber",
                                                    "field": "Alpha",
                                                    "value": 0.0
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-animation-start",
            Some("BuildingBlocks_Canvas.TestAnimationStart"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let pulse = ir.nodes.iter().find(|node| node.name == "pulse").expect("pulse node");
        assert_eq!(pulse.alpha, 0.0);
        assert_eq!(pulse.stroke_extent, Some(3.0));

        let label = ir.nodes.iter().find(|node| node.name == "label").expect("label node");
        assert_eq!(label.alpha, 0.66);
    }

    /// A Clip/ClipFade ancestor hard-clips only the axes whose `fade<Axis>`
    /// flag is FALSE: a faded axis is owned by the engine's edge-fade
    /// machinery, not the scissor (the power Scrollview authors Clip with
    /// fadeXAxis=true / fadeYAxis=false and the in-game capture renders the
    /// third column's temp gauge complete past the viewport's right edge,
    /// unfaded — widthFadeThreshold 0 disables the fade itself — while the
    /// list stays bounded vertically).
    #[test]
    fn compile_ir_clip_rect_skips_fade_axes() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestPerAxisClip",
            "_RecordValue_": {
                "size": {"x": 200, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "scroller",
                        "isActive": true,
                        "overflow": {
                            "_Type_": "BuildingBlocks_Overflow",
                            "overflow": "Clip",
                            "fadeXAxis": true,
                            "fadeYAxis": false,
                            "fadeZAxis": false,
                            "widthFadeThreshold": 0.0,
                            "heightFadeThreshold": 0.0,
                            "depthFadeThreshold": 0.0
                        },
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 50.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "wide_child",
                        "parent": "_PointsTo_:ptr:1",
                        "isActive": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 160.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-per-axis-clip",
            Some("BuildingBlocks_Canvas.TestPerAxisClip"),
            (200, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let child = ir.nodes.iter().find(|node| node.name == "wide_child").expect("wide child");
        let clip = child.clip_rect.as_ref().expect("clip rect present (Y axis clips)");
        assert!((clip.y - 0.0).abs() < 0.01, "y clipped to scroller top, got {}", clip.y);
        assert!((clip.h - 50.0).abs() < 0.01, "h clipped to scroller height, got {}", clip.h);
        assert!(clip.x <= 0.01, "faded X axis must not clip (x), got {}", clip.x);
        assert!(clip.w >= 200.0 - 0.01, "faded X axis must not clip (w), got {}", clip.w);
    }
}
