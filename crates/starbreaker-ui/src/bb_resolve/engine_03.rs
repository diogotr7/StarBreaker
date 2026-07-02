#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
use crate::bb_loc::LocFetcher;
#[allow(unused_imports)]
use crate::bb_scene::{BbNodeId, BbNodeType, BbScene, BbValue, parse_bb_canvas};
#[allow(unused_imports)]
use crate::bb_brand_style;
#[allow(unused_imports)]
use crate::record_name::extract_record_name;

// Consolidated engine chunk 03 (formerly: part_11.part, part_12.part, part_13.part, list_binding.part, list_binding_tests.part, array_list.part, registry_gates.part).
//   list_binding.part: List-binding slot materialisation.
//   list_binding_tests.part: Tests for list-binding slot materialisation (see list_binding.part): the
//   array_list.part: arrayVariable-driven WidgetList materialisation.
//   registry_gates.part: Registry-backed visibility gates: direct boolean variables and one-hop

#[cfg(test)]
mod tests_f {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::record_name::extract_record_name;

    /// `exportNode: false` marks editor-only nodes (mock layouts, DEL/Old
    /// leftovers, ambient-greeble groups like the medical footer's
    /// `base_animatedelements`): the engine build drops them. The parse stage
    /// already deactivates the node itself; the resolver must deactivate the
    /// SUBTREE — children author `exportNode: true` and would otherwise
    /// render. Structural rule, no name matching (plan P5.1).
    #[test]
    fn export_node_false_subtree_is_deactivated() {
        let canvas = serde_json::json!({
            "_RecordName_": "test_export_node",
            "_RecordId_": "00000000-0000-0000-0000-00000000000e",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "Root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "ambient_group", "parent": "_PointsTo_:ptr:1", "exportNode": false},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "greeble_child", "parent": "_PointsTo_:ptr:2", "exportNode": true},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "visible_sibling", "parent": "_PointsTo_:ptr:1", "exportNode": true}
                ]
            }
        });
        let scene = resolve_canvas_graph(&canvas, None, &|url| {
            Err(format!("no follows in fixture: {url}"))
        })
        .expect("resolve failed");

        let by_name = |name: &str| {
            scene
                .nodes
                .values()
                .find(|n| n.name == name)
                .unwrap_or_else(|| panic!("node {name} missing"))
        };
        assert!(!by_name("ambient_group").is_active, "container must be inactive");
        assert!(
            !by_name("greeble_child").is_active,
            "exportNode=false subtree must be deactivated even though the child authors exportNode=true"
        );
        assert!(by_name("visible_sibling").is_active, "sibling untouched");
    }


    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/fixtures/canvas/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {name}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("failed to parse {name}: {e}"))
    }


    fn one_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child",
            "_RecordId_": "00000000-0000-0000-0000-000000000001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"}
                ]
            }
        })
    }


    fn three_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child3",
            "_RecordId_": "00000000-0000-0000-0000-000000000002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        })
    }


    fn body_background_standard_fixture(texture_tag: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.BodyBackgroundWidgetStandard",
            "_RecordId_": "0b262f33-a075-42cb-907f-5e2fa3aa9df5",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_":"Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "_Type_": "BuildingBlocks_StyleEntry",
                        "name": "BackgroundTexture",
                        "conditionsList": [{
                            "_Type_": "BuildingBlocks_StyleConditionList",
                            "conditions": [{
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": {"_RecordId_": texture_tag}
                            }]
                        }],
                        "modifiers": [{
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/ModularKitStyles/_Default/SK_Default_BG.tif"
                        }]
                    }]
                },
                "brandStyles": [
                    {
                        "_Type_": "BuildingBlocks_BrandStyles",
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_drak.json",
                        "entries": [{
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "Background Texture Style",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": texture_tag}
                                }]
                            }],
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_FieldModifierString",
                                "field": "ImagePath",
                                "value": "UI/Textures/ModularKitStyles/DRAK/Drake_DoorPanel_Background-DarkTheme.tif"
                            }]
                        }]
                    },
                    {
                        "_Type_": "BuildingBlocks_BrandStyles",
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_bioc.json",
                        "entries": [{
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "Background Texture Style",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": texture_tag}
                                }]
                            }],
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_FieldModifierString",
                                "field": "ImagePath",
                                "value": "UI/Textures/ModularKitStyles/BIOC/BIOC_bg.tif"
                            }]
                        }]
                    }
                ],
                "scene": [],
                "operations": [{
                    "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                    "values": [
                        {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 0, "second": null},
                        {
                            "_Type_": "BuildingBlocks_IntegerTagPair",
                            "first": 1,
                            "second": {"_RecordId_": texture_tag}
                        }
                    ]
                }]
            }
        })
    }


    /// A host canvas (like M_MFD_Screen) with empty defaultStyles but a
    /// WidgetCanvas node that carries a `canvas` URL to a content canvas.
    fn host_canvas_with_widget_canvas_url(content_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_host",
            "_RecordId_": "00000000-0000-0000-0000-000000000010",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": []
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_content",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": content_url
                    }
                ]
            }
        })
    }


    /// Build a minimal canvas JSON that carries a single Pass 1 canvas-reference
    /// modifier pointing to `child_url`.
    fn canvas_with_style_ref(name: &str, child_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": name,
            "_RecordId_": "00000000-0000-0000-0000-0000000000aa",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "matchTo": "",
                        "modifiers": [{
                            "field": {
                                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                "value": child_url
                            }
                        }]
                    }]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": format!("{name}_root")}
                ]
            }
        })
    }


    /// Build a canvas with multiple conditional style entries, each pointing to
    /// a different child canvas URL.
    fn canvas_with_multi_conditional_refs(urls: &[(&str, &str)]) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = urls
            .iter()
            .map(|(name, url)| {
                serde_json::json!({
                    "name": name,
                    "matchTo": "",
                    "conditionsList": [{"_Type_": "BuildingBlocks_StyleConditionList", "conditions": [{"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"}]}],
                    "modifiers": [{
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                            "value": url
                        }
                    }]
                })
            })
            .collect();
        serde_json::json!({
            "_RecordName_": "multi_cond_root",
            "_RecordId_": "00000000-0000-0000-0000-000000000030",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": entries
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"}
                ]
            }
        })
    }


    #[test]
    fn pass2_does_not_follow_widget_canvas_urls_from_pass1_merged_children() {
        // Regression test for B3.3: Pass 2 must only follow WidgetCanvas.canvas
        // URLs that exist in the ROOT canvas's own scene BEFORE Pass 1 runs.
        //
        // Scenario (mirrors the real MC_S_Power_Master / MC_S_Self_Master bug):
        //
        //   master_canvas  (1 root node; NO WidgetCanvas nodes of its own)
        //     └── Pass 1 style ref → child_canvas.json
        //           child_canvas  (root + WidgetCanvas with canvas = "side_canvas.json")
        //
        // Because master has no WidgetCanvas nodes of its own, Pass 2 must
        // follow zero additional URLs at the master level.
        //
        // child_canvas's WidgetCanvas URL IS correctly followed by child's own
        // Pass 2 (since it appears in child's scene before child's Pass 1).
        // The visited set then prevents master's old-code Pass 2 from fetching
        // it a second time.  With this fix, master's Pass 2 does not even
        // attempt to collect the URL (collected list is empty before Pass 1).
        //
        // Verification: the fetcher is called exactly once for side_canvas
        // (by child's Pass 2, not master's), and the resolved scene has the
        // correct content — no spurious additional nodes from a phantom fetch.
        let side_canvas = serde_json::json!({
            "_RecordName_": "side_canvas",
            "_RecordId_": "00000000-0000-0000-0000-0000000000bb",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "side_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetIcon", "name": "side_icon", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        // child_canvas has a WidgetCanvas node pointing to side_canvas.
        let child_canvas = serde_json::json!({
            "_RecordName_": "child_canvas",
            "_RecordId_": "00000000-0000-0000-0000-0000000000cc",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "child_widget_canvas",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "file://./side_canvas.json"
                    }
                ]
            }
        });

        // master_canvas has no WidgetCanvas nodes in its own scene.
        let master_canvas = canvas_with_style_ref("master_canvas", "file://./child_canvas.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&master_canvas, None, &|url| {
            match url {
                "file://./side_canvas.json" => {
                    fetch_count.set(fetch_count.get() + 1);
                    Ok(side_canvas.clone())
                }
                "file://./child_canvas.json" => {
                    fetch_count.set(fetch_count.get() + 1);
                    Ok(child_canvas.clone())
                }
                // Framework records (tag database / widget standards) probed by
                // the expansion pass are not canvas follows.
                _ => Err(format!("framework record unavailable in fixture: {url}")),
            }
        })
        .expect("resolve failed");

        // side_canvas should be fetched exactly once (by child's own Pass 2).
        // master's Pass 2 has an empty URL list (master had no WidgetCanvas nodes
        // before Pass 1) so it never even attempts to follow side_canvas.
        // The visited set is a backstop but the fetch count must be 1 either way.
        assert_eq!(
            fetch_count.get(),
            2,
            "expected exactly 2 fetches (child_canvas + side_canvas), got {}",
            fetch_count.get()
        );

        // master(1) + child(2) + side(2) = 5 nodes — child correctly carries side content.
        assert_eq!(
            scene.nodes.len(),
            5,
            "expected 5 nodes (master root + child root + child WidgetCanvas + side root + side icon), got {}",
            scene.nodes.len()
        );
    }


    #[test]
    fn pass2_null_canvas_url_strings_are_not_followed() {
        // WidgetCanvas nodes may have canvas = "null" (literal string) when the
        // slot is unassigned (e.g. canvas_Interchangeable on an MFD host canvas).
        // These must not be passed to the fetcher.
        let host = serde_json::json!({
            "_RecordName_": "host",
            "_RecordId_": "00000000-0000-0000-0000-000000000020",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_interchangeable",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "null"
                    }
                ]
            }
        });

        let fetch_called = std::cell::Cell::new(false);
        let scene = resolve_canvas_graph(&host, None, &|url| {
            fetch_called.set(true);
            Err(format!("fetcher must not be called, got url: {url}"))
        })
        .expect("resolve must succeed even with null canvas URL");

        assert!(
            !fetch_called.get(),
            "fetcher must not be called for canvas=\"null\" WidgetCanvas nodes"
        );
        assert_eq!(scene.nodes.len(), 2, "expected 2 original nodes, got {}", scene.nodes.len());
    }


    #[test]
    fn pass1_follows_only_first_conditional_as_default() {
        // B7.2b: Pass 1 must follow only the FIRST (default-state) entry when
        // all entries are conditional.  Previously all 3 were followed, which
        // caused mode-mixing (GunsMode + NavMode + TurretMode merged together).
        //
        // Scenario: root has 3 conditional entries → child_a, child_b, child_c.
        // Each child has 2 nodes.  Only child_a (first entry) must be merged.
        let child_a = serde_json::json!({
            "_RecordName_": "child_a",
            "_RecordId_": "00000000-0000-0000-0000-000000000031",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "a_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_A", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child_b = serde_json::json!({
            "_RecordName_": "child_b",
            "_RecordId_": "00000000-0000-0000-0000-000000000032",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "b_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_B", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child_c = serde_json::json!({
            "_RecordName_": "child_c",
            "_RecordId_": "00000000-0000-0000-0000-000000000033",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "c_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_C", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        let root = canvas_with_multi_conditional_refs(&[
            ("Entry A", "file://./child_a.json"),
            ("Entry B", "file://./child_b.json"),
            ("Entry C", "file://./child_c.json"),
        ]);

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(match url {
                "file://./child_a.json" => child_a.clone(),
                "file://./child_b.json" => child_b.clone(),
                "file://./child_c.json" => child_c.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        // Only the first conditional entry (child_a) must be fetched.
        assert_eq!(
            fetch_count.get(),
            1,
            "expected 1 fetch (first-entry fallback only), got {}",
            fetch_count.get()
        );

        // root(1) + child_a(2) = 3 nodes total.
        assert_eq!(
            scene.nodes.len(),
            3,
            "expected 3 merged nodes (root + child_a(2)), got {}",
            scene.nodes.len()
        );

        let names: std::collections::HashSet<&str> = scene
            .nodes
            .values()
            .filter_map(|n| if n.name.is_empty() { None } else { Some(n.name.as_str()) })
            .collect();
        assert!(names.contains("a_root"), "a_root not found in merged scene");
        assert!(!names.contains("b_root"), "b_root must NOT appear (conditional entry skipped)");
        assert!(!names.contains("c_root"), "c_root must NOT appear (conditional entry skipped)");
    }
}

#[cfg(test)]
mod tests_g {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::record_name::extract_record_name;


    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/fixtures/canvas/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {name}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("failed to parse {name}: {e}"))
    }


    fn one_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child",
            "_RecordId_": "00000000-0000-0000-0000-000000000001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"}
                ]
            }
        })
    }


    fn three_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child3",
            "_RecordId_": "00000000-0000-0000-0000-000000000002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        })
    }


    fn body_background_standard_fixture(texture_tag: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.BodyBackgroundWidgetStandard",
            "_RecordId_": "0b262f33-a075-42cb-907f-5e2fa3aa9df5",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_":"Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "_Type_": "BuildingBlocks_StyleEntry",
                        "name": "BackgroundTexture",
                        "conditionsList": [{
                            "_Type_": "BuildingBlocks_StyleConditionList",
                            "conditions": [{
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": {"_RecordId_": texture_tag}
                            }]
                        }],
                        "modifiers": [{
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/ModularKitStyles/_Default/SK_Default_BG.tif"
                        }]
                    }]
                },
                "brandStyles": [
                    {
                        "_Type_": "BuildingBlocks_BrandStyles",
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_drak.json",
                        "entries": [{
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "Background Texture Style",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": texture_tag}
                                }]
                            }],
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_FieldModifierString",
                                "field": "ImagePath",
                                "value": "UI/Textures/ModularKitStyles/DRAK/Drake_DoorPanel_Background-DarkTheme.tif"
                            }]
                        }]
                    },
                    {
                        "_Type_": "BuildingBlocks_BrandStyles",
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_bioc.json",
                        "entries": [{
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "Background Texture Style",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": texture_tag}
                                }]
                            }],
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_FieldModifierString",
                                "field": "ImagePath",
                                "value": "UI/Textures/ModularKitStyles/BIOC/BIOC_bg.tif"
                            }]
                        }]
                    }
                ],
                "scene": [],
                "operations": [{
                    "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                    "values": [
                        {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 0, "second": null},
                        {
                            "_Type_": "BuildingBlocks_IntegerTagPair",
                            "first": 1,
                            "second": {"_RecordId_": texture_tag}
                        }
                    ]
                }]
            }
        })
    }


    /// A host canvas (like M_MFD_Screen) with empty defaultStyles but a
    /// WidgetCanvas node that carries a `canvas` URL to a content canvas.
    fn host_canvas_with_widget_canvas_url(content_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_host",
            "_RecordId_": "00000000-0000-0000-0000-000000000010",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": []
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_content",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": content_url
                    }
                ]
            }
        })
    }


    /// Build a minimal canvas JSON that carries a single Pass 1 canvas-reference
    /// modifier pointing to `child_url`.
    fn canvas_with_style_ref(name: &str, child_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": name,
            "_RecordId_": "00000000-0000-0000-0000-0000000000aa",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "matchTo": "",
                        "modifiers": [{
                            "field": {
                                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                "value": child_url
                            }
                        }]
                    }]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": format!("{name}_root")}
                ]
            }
        })
    }


    /// Build a canvas with multiple conditional style entries, each pointing to
    /// a different child canvas URL.
    fn canvas_with_multi_conditional_refs(urls: &[(&str, &str)]) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = urls
            .iter()
            .map(|(name, url)| {
                serde_json::json!({
                    "name": name,
                    "matchTo": "",
                    "conditionsList": [{"_Type_": "BuildingBlocks_StyleConditionList", "conditions": [{"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"}]}],
                    "modifiers": [{
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                            "value": url
                        }
                    }]
                })
            })
            .collect();
        serde_json::json!({
            "_RecordName_": "multi_cond_root",
            "_RecordId_": "00000000-0000-0000-0000-000000000030",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": entries
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"}
                ]
            }
        })
    }


    #[test]
    fn pass1_prefers_unconditional_over_first_conditional() {
        // B7.2b: when entries have a mix of conditional and unconditional,
        // Pass 1 must prefer the first UNCONDITIONAL entry over the first
        // overall.
        //
        // Scenario: root has [conditional→child_a, unconditional→child_b].
        // Only child_b must be merged.
        let child_a = serde_json::json!({
            "_RecordName_": "child_a",
            "_RecordId_": "00000000-0000-0000-0000-000000000041",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "a_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_A", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child_b = serde_json::json!({
            "_RecordName_": "child_b",
            "_RecordId_": "00000000-0000-0000-0000-000000000042",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "b_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text_B", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        // Build a root canvas with two entries: [0] conditional, [1] unconditional.
        let root = serde_json::json!({
            "_RecordName_": "mixed_root",
            "_RecordId_": "00000000-0000-0000-0000-000000000040",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [
                        // Conditional entry (has conditionsList with one condition).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "ConditionalEntry",
                            "conditionsList": [{"_Type_": "BuildingBlocks_StyleConditionList", "name": "cond", "conditions": [{"key": "mode", "value": "guns"}]}],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./child_a.json"
                                }
                            }]
                        },
                        // Unconditional entry (empty conditionsList).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "UnconditionalEntry",
                            "conditionsList": [],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./child_b.json"
                                }
                            }]
                        }
                    ]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"}
                ]
            }
        });

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(match url {
                "file://./child_a.json" => child_a.clone(),
                "file://./child_b.json" => child_b.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        // Only child_b (unconditional) must be fetched.
        assert_eq!(
            fetch_count.get(),
            1,
            "expected 1 fetch (unconditional entry preferred), got {}",
            fetch_count.get()
        );

        let names: std::collections::HashSet<&str> = scene
            .nodes
            .values()
            .filter_map(|n| if n.name.is_empty() { None } else { Some(n.name.as_str()) })
            .collect();
        assert!(names.contains("b_root"), "b_root (unconditional) must be in merged scene");
        assert!(!names.contains("a_root"), "a_root (conditional) must NOT appear");
    }


    /// When every entry in `defaultStyles.entries` is conditional, the
    /// most-tagged-node heuristic must select the entry whose condition tag
    /// appears on the scene node that carries the highest number of style tags.
    ///
    /// This mirrors the `MC_S_Self_Master` case where `canvas_GunsMode` has 2
    /// style tags while all other canvas nodes have 1, so the GunsMode entry
    /// (→ `gen_mc_s_weaponinfo`) must win over the first entry (→ `gen_mc_s_ammolists`).
    #[test]
    fn pick_default_entry_most_tagged_node_wins() {
        // Child canvases with a single content node each.
        let child_ammo = serde_json::json!({
            "_RecordName_": "ammo",
            "_RecordId_": "00000000-0000-0000-0000-000000000010",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "ammo_root"}
                ]
            }
        });
        let child_guns = serde_json::json!({
            "_RecordName_": "guns",
            "_RecordId_": "00000000-0000-0000-0000-000000000020",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "guns_root"}
                ]
            }
        });

        // Root: two conditional entries.
        // Scene has a DisplayWidget root (ptr:1) with two WidgetCanvas children
        // (ptr:2 = AmmoNumbers with 1 tag, ptr:3 = GunsMode with 2 tags).
        let root = serde_json::json!({
            "_RecordName_": "self_master",
            "_RecordId_": "00000000-0000-0000-0000-000000000001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [
                        // Entry 0 — condition tag matches canvas_AmmoNumbers (1 style tag).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "AmmoNumbers Canvas",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": "tag_ammo"}
                                }]
                            }],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./ammo.json"
                                }
                            }]
                        },
                        // Entry 1 — condition tag matches canvas_GunsMode (2 style tags).
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "GunsMode Canvas",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": "tag_guns_mode"}
                                }]
                            }],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./guns.json"
                                }
                            }]
                        }
                    ]
                },
                "brandStyles": [],
                "scene": [
                    // Root DisplayWidget (ptr:1).
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "self_master_root"},
                    // canvas_AmmoNumbers: 1 style tag.
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_AmmoNumbers",
                        "parent": "_PointsTo_:ptr:1",
                        "styleTags": [{"_RecordId_": "tag_ammo"}]
                    },
                    // canvas_GunsMode: 2 style tags — the "most-tagged" node.
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_GunsMode",
                        "parent": "_PointsTo_:ptr:1",
                        "styleTags": [
                            {"_RecordId_": "tag_guns_mode"},
                            {"_RecordId_": "tag_extra"}
                        ]
                    }
                ]
            }
        });

        let chosen = std::cell::Cell::new(None::<&'static str>);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            let label: &'static str = if url.contains("guns") { "guns" } else { "ammo" };
            chosen.set(Some(label));
            Ok(match url {
                "file://./ammo.json" => child_ammo.clone(),
                "file://./guns.json" => child_guns.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        assert_eq!(
            chosen.get(),
            Some("guns"),
            "most-tagged-node heuristic must pick GunsMode entry"
        );
        let names: std::collections::HashSet<&str> = scene
            .nodes
            .values()
            .filter_map(|n| if n.name.is_empty() { None } else { Some(n.name.as_str()) })
            .collect();
        assert!(names.contains("guns_root"), "guns_root must be in merged scene");
        assert!(!names.contains("ammo_root"), "ammo_root must NOT appear");
    }
}

#[cfg(test)]
mod tests_h {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::record_name::extract_record_name;


    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/fixtures/canvas/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {name}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("failed to parse {name}: {e}"))
    }


    fn one_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child",
            "_RecordId_": "00000000-0000-0000-0000-000000000001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"}
                ]
            }
        })
    }


    fn three_node_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_child3",
            "_RecordId_": "00000000-0000-0000-0000-000000000002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "child_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "c2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        })
    }


    fn body_background_standard_fixture(texture_tag: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.BodyBackgroundWidgetStandard",
            "_RecordId_": "0b262f33-a075-42cb-907f-5e2fa3aa9df5",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_":"Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "_Type_": "BuildingBlocks_StyleEntry",
                        "name": "BackgroundTexture",
                        "conditionsList": [{
                            "_Type_": "BuildingBlocks_StyleConditionList",
                            "conditions": [{
                                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                "tag": {"_RecordId_": texture_tag}
                            }]
                        }],
                        "modifiers": [{
                            "_Type_": "BuildingBlocks_FieldModifierString",
                            "field": "ImagePath",
                            "value": "UI/Textures/ModularKitStyles/_Default/SK_Default_BG.tif"
                        }]
                    }]
                },
                "brandStyles": [
                    {
                        "_Type_": "BuildingBlocks_BrandStyles",
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_drak.json",
                        "entries": [{
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "Background Texture Style",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": texture_tag}
                                }]
                            }],
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_FieldModifierString",
                                "field": "ImagePath",
                                "value": "UI/Textures/ModularKitStyles/DRAK/Drake_DoorPanel_Background-DarkTheme.tif"
                            }]
                        }]
                    },
                    {
                        "_Type_": "BuildingBlocks_BrandStyles",
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_bioc.json",
                        "entries": [{
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "Background Texture Style",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": texture_tag}
                                }]
                            }],
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_FieldModifierString",
                                "field": "ImagePath",
                                "value": "UI/Textures/ModularKitStyles/BIOC/BIOC_bg.tif"
                            }]
                        }]
                    }
                ],
                "scene": [],
                "operations": [{
                    "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                    "values": [
                        {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 0, "second": null},
                        {
                            "_Type_": "BuildingBlocks_IntegerTagPair",
                            "first": 1,
                            "second": {"_RecordId_": texture_tag}
                        }
                    ]
                }]
            }
        })
    }


    /// A host canvas (like M_MFD_Screen) with empty defaultStyles but a
    /// WidgetCanvas node that carries a `canvas` URL to a content canvas.
    fn host_canvas_with_widget_canvas_url(content_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "test_host",
            "_RecordId_": "00000000-0000-0000-0000-000000000010",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": []
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "base_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_content",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": content_url
                    }
                ]
            }
        })
    }


    /// Build a minimal canvas JSON that carries a single Pass 1 canvas-reference
    /// modifier pointing to `child_url`.
    fn canvas_with_style_ref(name: &str, child_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": name,
            "_RecordId_": "00000000-0000-0000-0000-0000000000aa",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "matchTo": "",
                        "modifiers": [{
                            "field": {
                                "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                "value": child_url
                            }
                        }]
                    }]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": format!("{name}_root")}
                ]
            }
        })
    }


    /// Build a canvas with multiple conditional style entries, each pointing to
    /// a different child canvas URL.
    fn canvas_with_multi_conditional_refs(urls: &[(&str, &str)]) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = urls
            .iter()
            .map(|(name, url)| {
                serde_json::json!({
                    "name": name,
                    "matchTo": "",
                    "conditionsList": [{"_Type_": "BuildingBlocks_StyleConditionList", "conditions": [{"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"}]}],
                    "modifiers": [{
                        "field": {
                            "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                            "value": url
                        }
                    }]
                })
            })
            .collect();
        serde_json::json!({
            "_RecordName_": "multi_cond_root",
            "_RecordId_": "00000000-0000-0000-0000-000000000030",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": entries
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"}
                ]
            }
        })
    }


    #[test]
    fn pick_default_entry_breaks_equal_tag_score_with_specificity() {
        let child_simple = serde_json::json!({
            "_RecordName_": "simple",
            "_RecordId_": "00000000-0000-0000-0000-000000000110",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "simple_root"}
                ]
            }
        });
        let child_specific = serde_json::json!({
            "_RecordName_": "specific",
            "_RecordId_": "00000000-0000-0000-0000-000000000120",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "specific_root"}
                ]
            }
        });

        let root = serde_json::json!({
            "_RecordName_": "specificity_master",
            "_RecordId_": "00000000-0000-0000-0000-000000000101",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "SimpleTagEntry",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [{
                                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                    "tag": {"_RecordId_": "tag_shared"}
                                }]
                            }],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./simple.json"
                                }
                            }]
                        },
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "SpecificTagEntry",
                            "conditionsList": [{
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [
                                    {
                                        "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                        "tag": {"_RecordId_": "tag_shared"}
                                    },
                                    {
                                        "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                        "tag": {"_RecordId_": "tag_extra"}
                                    }
                                ]
                            }],
                            "matchTo": "",
                            "modifiers": [{
                                "_Type_": "BuildingBlocks_StyleModifier",
                                "field": {
                                    "_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                    "value": "file://./specific.json"
                                }
                            }]
                        }
                    ]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "specificity_root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "canvas_Target",
                        "parent": "_PointsTo_:ptr:1",
                        "styleTags": [
                            {"_RecordId_": "tag_shared"},
                            {"_RecordId_": "tag_extra"}
                        ]
                    }
                ]
            }
        });

        let chosen = std::cell::Cell::new(None::<&'static str>);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            let label: &'static str = if url.contains("specific") { "specific" } else { "simple" };
            chosen.set(Some(label));
            Ok(match url {
                "file://./simple.json" => child_simple.clone(),
                "file://./specific.json" => child_specific.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        assert_eq!(chosen.get(), Some("specific"));
        let names: std::collections::HashSet<&str> = scene
            .nodes
            .values()
            .filter_map(|n| if n.name.is_empty() { None } else { Some(n.name.as_str()) })
            .collect();
        assert!(names.contains("specific_root"));
        assert!(!names.contains("simple_root"));
    }
}

// List-binding slot materialisation.
//
// The engine instantiates a list-driven `WidgetCanvas` slot once per entry of
// a runtime list (e.g. the power screen's `piplist`: a Scrollview hosting one
// `canvas_assignmentListItem` per power system, each instance's variables
// namespaced `piplist/[000N]/…`). The data signature, all authored:
//   - an integer count variable `<B>` plus sibling variable bindings of the
//     form `<B>/[0000]/…` in the same canvas,
//   - a widget gated `IsActive`/`Instantiated` on a boolean chain over `<B>`
//     (authored `<B> > 0`),
//   - a `WidgetCanvas` slot at/under the gated widget.
//
// A static render materialises from the defaults registry: `<B> = N` clones
// the slot into N flex siblings, each instance's inheriting variable bindings
// rewritten under `<B>/[000i]/`; an absent/zero count hides the gated
// container — the engine's at-rest state (count variables default 0).

/// Per-slot-instance binding namespace (`<count>/[000i]`), keyed by slot node.
type ListNamespaces = std::collections::HashMap<BbNodeId, String>;

struct ListSlotBinding {
    count_binding: String,
    gated_widgets: Vec<BbNodeId>,
    slots: Vec<BbNodeId>,
}

/// Detect and materialise list-bound slots. Returns the per-instance binding
/// namespaces for the Pass-2 merge loop to apply to each instance's resolved
/// child scene.
pub(crate) fn apply_list_slot_bindings(
    scene: &mut BbScene,
    canvas_urls: &mut Vec<(BbNodeId, String, Vec<serde_json::Value>)>,
    instantiated_false: &mut std::collections::HashSet<BbNodeId>,
    defaults: &crate::defaults::DefaultValueRegistry,
) -> ListNamespaces {
    let mut namespaces = ListNamespaces::new();
    for list in detect_list_slot_bindings(scene) {
        let count = defaults
            .lookup_path(&list.count_binding)
            .and_then(|value| match value {
                crate::canvas::Value::Int(i) => Some(*i),
                crate::canvas::Value::Float(f) => Some(*f as i64),
                _ => None,
            })
            .unwrap_or(0);
        if count <= 0 {
            // At rest a list-count variable holds 0, so the authored
            // `<count> > 0` gate hides the container.
            instantiated_false.extend(list.gated_widgets.iter().copied());
            continue;
        }
        for widget in &list.gated_widgets {
            instantiated_false.remove(widget);
        }
        for &slot in &list.slots {
            let Some((slot_url, slot_params)) = canvas_urls
                .iter()
                .find(|(id, _, _)| *id == slot)
                .map(|(_, url, params)| (url.clone(), params.clone()))
            else {
                continue;
            };
            namespaces.insert(slot, format!("{}/[{:04}]", list.count_binding, 0));
            // The engine clones the slot's ITEM TEMPLATE — its nearest gated
            // ancestor (the list widget's flex item) — once per entry.
            let template = nearest_gated_ancestor(scene, slot, &list.gated_widgets).unwrap_or(slot);
            for index in 1..count {
                let Some((cloned_root, id_map)) = clone_subtree_as_sibling_with_map(scene, template)
                else {
                    break;
                };
                if let Some(root) = scene.nodes.get_mut(&cloned_root)
                    && let Some(map) = root.raw.as_object_mut()
                {
                    map.insert("_MaterialisedEntry_".to_string(), serde_json::Value::Bool(true));
                }
                // Register every cloned slot canvas for Pass-2 follow with the
                // instance namespace.
                let mut stack = vec![cloned_root];
                while let Some(id) = stack.pop() {
                    let Some(node) = scene.nodes.get(&id) else {
                        continue;
                    };
                    stack.extend(node.children.iter().copied());
                    let has_url = node
                        .raw
                        .get("canvas")
                        .and_then(|v| v.as_str())
                        .is_some_and(|url| !url.is_empty() && url != "null");
                    if node.ty == BbNodeType::WidgetCanvas && has_url {
                        canvas_urls.push((id, slot_url.clone(), slot_params.clone()));
                        namespaces.insert(id, format!("{}/[{:04}]", list.count_binding, index));
                    }
                }
                duplicate_slot_field_ops_for_clone(scene, &id_map);
            }
        }
    }
    namespaces
}

/// Rewrite an instance's inheriting variable bindings (and synthetic static
/// variable names) under the slot's list namespace, mirroring the engine's
/// per-instance namespace assignment.
pub(crate) fn namespace_child_scene_bindings(child_scene: &mut BbScene, namespace: &str) {
    for op in &mut child_scene.operations {
        let Some(ty) = op.get("_Type_").and_then(|v| v.as_str()).map(str::to_owned) else {
            continue;
        };
        if ty == "_SynthStaticVariable_" {
            if let Some(name) = op.get("name").and_then(|v| v.as_str()).map(str::to_owned)
                && let Some(map) = op.as_object_mut()
            {
                map.insert(
                    "name".to_string(),
                    serde_json::Value::String(format!("{namespace}/{name}")),
                );
            }
            continue;
        }
        if !(ty.starts_with("BuildingBlocks_Bindings") && ty.ends_with("Variable")) {
            continue;
        }
        // `inheritsNamespace: false` bindings address absolute engine paths.
        if op.get("inheritsNamespace").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        if let Some(binding) = op
            .get("binding")
            .and_then(|v| v.as_str())
            .filter(|b| !b.is_empty())
            .map(str::to_owned)
            && let Some(map) = op.as_object_mut()
        {
            map.insert(
                "binding".to_string(),
                serde_json::Value::String(format!("{namespace}/{binding}")),
            );
        }
    }
}

/// Duplicate the parent scene's field operations that target an original
/// slot's `WidgetCanvas` nodes, retargeting each cloned instance. The engine
/// applies the same slot wiring (e.g. the power lists' `ParamInput2 ←
/// 'Max Piplist'`) to every instance; input chains are shared — only the
/// `widget` ref changes, and any `_Pointer_` is dropped so the copy does not
/// shadow the original's definition.
fn duplicate_slot_field_ops_for_clone(
    scene: &mut BbScene,
    id_map: &std::collections::HashMap<BbNodeId, BbNodeId>,
) {
    let mut copies: Vec<serde_json::Value> = Vec::new();
    for op in &scene.operations {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if !ty.contains("Field") {
            continue;
        }
        let Some(widget) = op
            .get("widget")
            .and_then(|v| v.as_str())
            .and_then(parse_points_to_or_ptr)
        else {
            continue;
        };
        let Some(&clone_id) = id_map.get(&widget) else {
            continue;
        };
        let is_canvas_slot = scene
            .nodes
            .get(&widget)
            .is_some_and(|node| node.ty == BbNodeType::WidgetCanvas);
        if !is_canvas_slot {
            continue;
        }
        let mut copy = op.clone();
        if let Some(map) = copy.as_object_mut() {
            map.remove("_Pointer_");
            map.insert(
                "widget".to_string(),
                serde_json::Value::String(format!("_PointsTo_:ptr:{clone_id}")),
            );
        }
        copies.push(copy);
    }
    scene.operations.extend(copies);
}

/// Find list-slot signatures in the scene's operations.
fn detect_list_slot_bindings(scene: &BbScene) -> Vec<ListSlotBinding> {
    let mut ptr_to_op: std::collections::HashMap<String, &serde_json::Value> =
        std::collections::HashMap::new();
    let mut variable_bindings: Vec<(String, String)> = Vec::new(); // (ptr, binding)
    for op in &scene.operations {
        if let Some(ptr) = op.get("_Pointer_").and_then(|v| v.as_str()) {
            ptr_to_op.insert(ptr.to_string(), op);
        }
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if ty.starts_with("BuildingBlocks_Bindings")
            && ty.ends_with("Variable")
            && let (Some(ptr), Some(binding)) = (
                op.get("_Pointer_").and_then(|v| v.as_str()),
                op.get("binding").and_then(|v| v.as_str()),
            )
        {
            variable_bindings.push((ptr.to_string(), binding.to_string()));
        }
    }

    // Count candidates: binding B with a sibling `B/[0000]/…` reference.
    let mut count_bindings: Vec<String> = Vec::new();
    for (_, binding) in &variable_bindings {
        if binding.contains("/[") {
            continue;
        }
        let indexed_prefix = format!("{binding}/[");
        if variable_bindings
            .iter()
            .any(|(_, other)| other.starts_with(&indexed_prefix))
            && !count_bindings.contains(binding)
        {
            count_bindings.push(binding.clone());
        }
    }

    let mut out = Vec::new();
    for count_binding in count_bindings {
        // Pointers of variable ops reading the count binding.
        let count_ptrs: std::collections::HashSet<&str> = variable_bindings
            .iter()
            .filter(|(_, b)| *b == count_binding)
            .map(|(p, _)| p.as_str())
            .collect();

        // Gated widgets: IsActive/Instantiated boolean fields whose input
        // chain references a count-variable pointer.
        let mut gated_widgets: Vec<BbNodeId> = Vec::new();
        for op in &scene.operations {
            if op.get("_Type_").and_then(|v| v.as_str())
                != Some("BuildingBlocks_BindingsBooleanField")
            {
                continue;
            }
            let field = op.get("field").and_then(|v| v.as_str()).unwrap_or("");
            if !matches!(field, "IsActive" | "Instantiated") {
                continue;
            }
            let Some(widget) = op
                .get("widget")
                .and_then(|v| v.as_str())
                .and_then(parse_points_to_or_ptr)
            else {
                continue;
            };
            let mut visited = std::collections::HashSet::new();
            if op
                .get("input")
                .is_some_and(|input| chain_references_ptr(input, &ptr_to_op, &count_ptrs, &mut visited))
                && !gated_widgets.contains(&widget)
            {
                gated_widgets.push(widget);
            }
        }
        if gated_widgets.is_empty() {
            continue;
        }

        // Slots: WidgetCanvas nodes at/under a gated widget with a canvas url.
        let mut slots: Vec<BbNodeId> = Vec::new();
        for (&id, node) in &scene.nodes {
            if node.ty != BbNodeType::WidgetCanvas {
                continue;
            }
            let has_url = node
                .raw
                .get("canvas")
                .and_then(|v| v.as_str())
                .is_some_and(|url| !url.is_empty() && url != "null");
            if !has_url {
                continue;
            }
            let mut cursor = Some(id);
            while let Some(current) = cursor {
                if gated_widgets.contains(&current) {
                    slots.push(id);
                    break;
                }
                cursor = scene.nodes.get(&current).and_then(|n| n.parent);
            }
        }
        if slots.is_empty() {
            continue;
        }
        out.push(ListSlotBinding {
            count_binding,
            gated_widgets,
            slots,
        });
    }
    out
}

/// Walk a binding-op input chain and report whether it references any of the
/// given variable pointers.
fn chain_references_ptr(
    input: &serde_json::Value,
    ptr_to_op: &std::collections::HashMap<String, &serde_json::Value>,
    targets: &std::collections::HashSet<&str>,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    match input {
        serde_json::Value::String(s) => {
            let ptr = s.strip_prefix("_PointsTo_:").unwrap_or(s);
            if !ptr.starts_with("ptr:") || !visited.insert(ptr.to_string()) {
                return false;
            }
            if targets.contains(ptr) {
                return true;
            }
            ptr_to_op
                .get(ptr)
                .is_some_and(|op| chain_references_ptr(op, ptr_to_op, targets, visited))
        }
        serde_json::Value::Object(obj) => {
            if let Some(ptr) = obj.get("_Pointer_").and_then(|v| v.as_str())
                && targets.contains(ptr)
            {
                return true;
            }
            for key in ["input", "inputL", "inputR", "inputTrue", "inputFalse"] {
                if obj
                    .get(key)
                    .is_some_and(|inner| chain_references_ptr(inner, ptr_to_op, targets, visited))
                {
                    return true;
                }
            }
            obj.get("inputs").and_then(|v| v.as_array()).is_some_and(|inputs| {
                inputs
                    .iter()
                    .any(|inner| chain_references_ptr(inner, ptr_to_op, targets, visited))
            })
        }
        _ => false,
    }
}

/// Parse `"_PointsTo_:ptr:N"` or `"ptr:N"` → node id.
fn parse_points_to_or_ptr(s: &str) -> Option<BbNodeId> {
    s.strip_prefix("_PointsTo_:")
        .unwrap_or(s)
        .strip_prefix("ptr:")
        .and_then(|n| n.parse().ok())
}

/// Next unused node id below the widget-standard expansion band.
fn next_free_low_node_id(scene: &BbScene) -> BbNodeId {
    let mut next = scene
        .nodes
        .keys()
        .rev()
        .find(|&&k| k < EXPANSION_ID_BASE)
        .copied()
        .unwrap_or(0)
        .wrapping_add(1);
    while scene.nodes.contains_key(&next) {
        next = next.wrapping_add(1);
    }
    next
}

/// The slot's nearest ancestor (inclusive) that appears in `gated`.
fn nearest_gated_ancestor(
    scene: &BbScene,
    slot: BbNodeId,
    gated: &[BbNodeId],
) -> Option<BbNodeId> {
    let mut cursor = Some(slot);
    while let Some(current) = cursor {
        if gated.contains(&current) {
            return Some(current);
        }
        cursor = scene.nodes.get(&current).and_then(|n| n.parent);
    }
    None
}

/// Next unused low-band node id that is also not pending in `reserved`.
fn next_free_low_node_id_excluding(
    scene: &BbScene,
    reserved: &std::collections::HashMap<BbNodeId, BbNodeId>,
) -> BbNodeId {
    let mut next = next_free_low_node_id(scene);
    while reserved.values().any(|&v| v == next) {
        next = next.wrapping_add(1);
    }
    next
}

// Tests for list-binding slot materialisation (see list_binding.part): the
// power screen's `piplist` pattern — count-gated container, slot cloned per
// entry, per-instance namespaced bindings, hidden at rest.
#[cfg(test)]
mod tests_list_binding {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::defaults::DefaultValueRegistry;

    fn item_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_ListItem",
            "_RecordId_": "00000000-0000-0000-0000-0000000l0001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "item_marker", "isActive": true}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:1", "field": "SizeX", "input": "_PointsTo_:ptr:2"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                     "path": [], "binding": "value", "inheritsNamespace": true}
                ]
            }
        })
    }

    /// Parent canvas wired like `gen_mc_s_powerlists`: a count variable
    /// (`items`), an indexed sibling reference (`items/[0000]/flag`), a
    /// container gated `Instantiated ← items > 0`, and a WidgetCanvas slot
    /// under it referencing the item canvas.
    fn list_parent_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_ListParent",
            "_RecordId_": "00000000-0000-0000-0000-0000000l0002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 400.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "list_container", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_LayoutPolicyFlexContainer", "direction": "Row"}},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_listItem", "parent": "_PointsTo_:ptr:2", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_listitem.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsBooleanField",
                     "widget": "_PointsTo_:ptr:2", "field": "Instantiated", "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsBooleanFromInteger",
                     "type": "Greater", "inputL": "_PointsTo_:ptr:12", "inputR": null, "value": 0},
                    {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "items", "inheritsNamespace": true},
                    {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "items/[0000]/flag", "inheritsNamespace": true}
                ]
            }
        })
    }

    fn fetcher(path: &str) -> Result<serde_json::Value, String> {
        if path.to_ascii_lowercase().contains("test_listitem") {
            Ok(item_canvas())
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    /// At rest (no list data) the count variable is 0 and the gated container
    /// must resolve inactive — the engine's `items > 0` gate.
    #[test]
    fn list_container_hides_when_count_is_unbound() {
        let scene = resolve_canvas_graph_with_defaults(
            &list_parent_canvas(),
            Some("drak"),
            &fetcher,
            None,
            None,
            &DefaultValueRegistry::default(),
        )
        .expect("resolve");
        let container = scene
            .nodes
            .values()
            .find(|n| n.name == "list_container")
            .expect("container");
        assert!(!container.is_active, "unbound count must hide the list container");
    }

    /// With `items = 2` bound, the slot materialises two instances and each
    /// instance's inheriting variable bindings are namespaced per index.
    #[test]
    fn list_slot_materialises_per_count_with_namespaced_bindings() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("items", crate::canvas::Value::Int(2));
        let scene = resolve_canvas_graph_with_defaults(
            &list_parent_canvas(),
            Some("drak"),
            &fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let container = scene
            .nodes
            .values()
            .find(|n| n.name == "list_container")
            .expect("container");
        assert!(container.is_active, "bound count must keep the container active");

        let markers = scene
            .nodes
            .values()
            .filter(|n| n.name == "item_marker")
            .count();
        assert_eq!(markers, 2, "two item instances must merge");

        let bindings: Vec<String> = scene
            .operations
            .iter()
            .filter_map(|op| {
                let ty = op.get("_Type_").and_then(|v| v.as_str())?;
                if !(ty.starts_with("BuildingBlocks_Bindings") && ty.ends_with("Variable")) {
                    return None;
                }
                op.get("binding").and_then(|v| v.as_str()).map(str::to_owned)
            })
            .collect();
        assert!(
            bindings.iter().any(|b| b == "items/[0000]/value"),
            "instance 0 binding must be namespaced; got {bindings:?}"
        );
        assert!(
            bindings.iter().any(|b| b == "items/[0001]/value"),
            "instance 1 binding must be namespaced; got {bindings:?}"
        );
    }
}

// arrayVariable-driven WidgetList materialisation.
//
// The engine's list widgets can bind an ARRAY variable directly
// (`WidgetList.arrayVariable`, e.g. the power screen pip stack's
// `"pipList"`), instantiating the list's single authored template child once
// per array entry; each entry's inheriting bindings resolve under the entry
// namespace `<listpath>/[000j]/…`. A static render materialises from the
// defaults registry: an integer count at the resolved array path clones the
// template (nodes AND its field-binding operations) per entry; count 0 hides
// the template (empty array at rest); an ABSENT count leaves the list
// untouched so other list models (the count-binding slot signature in
// list_binding.part) keep owning it.
//
// Relative array paths resolve against the instance namespace assigned by the
// outer list materialisation (so the per-system pip counts differ:
// `piplist/[0000]/pipList = 4`, `piplist/[0001]/pipList = 6`, …). A relative
// path with no namespace in scope cannot be resolved and is skipped.

/// The engine-wide 'selected' tag (TagDatabase
/// `797684dc-56d9-452f-9496-e0b7a2ae8dac`), applied by list selection
/// machinery to the selected entry; style entries (the pip selector arrow's
/// `PipBox_Selector_Arrow_Visibility`) condition on it.
const LIST_SELECTED_TAG_UUID: &str = "797684dc-56d9-452f-9496-e0b7a2ae8dac";

/// Materialise arrayVariable-bound WidgetLists in `scene` from the defaults
/// registry. `namespace` is the instance namespace assigned to this scene by
/// the outer list materialisation ("" when none).
pub(crate) fn apply_array_variable_lists(
    scene: &mut BbScene,
    namespace: &str,
    defaults: &crate::defaults::DefaultValueRegistry,
) {
    let lists: Vec<(BbNodeId, String)> = scene
        .nodes
        .iter()
        .filter_map(|(&id, node)| {
            let array_var = node
                .raw
                .get("arrayVariable")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())?;
            let path = if let Some(absolute) = array_var.strip_prefix('/') {
                absolute.to_owned()
            } else if namespace.is_empty() {
                // A namespace-less arrayVariable that is a MULTI-SEGMENT path
                // (e.g. the compass `FlightController/Compass/Ticks`) is an
                // absolute engine-state reference — the authored leading slash is
                // inconsistent across the data (cf. `/resourcenetworkUi/…`). Resolve
                // it directly; the registry pins the at-rest count (0 for the
                // compass, whose ticks the flight controller pushes live). A BARE
                // single-segment name (the power outer `pipList`, `itemList`) is
                // UI-local and genuinely needs a list namespace, so it stays skipped.
                if array_var.contains('/') {
                    array_var.to_owned()
                } else {
                    return None;
                }
            } else {
                format!("{namespace}/{array_var}")
            };
            Some((id, path))
        })
        .collect();

    for (list_id, path) in lists {
        let Some(count) = defaults.lookup_path(&path).and_then(|value| match value {
            crate::canvas::Value::Int(i) => Some(*i),
            crate::canvas::Value::Float(f) => Some(*f as i64),
            _ => None,
        }) else {
            continue;
        };
        let active_children: Vec<BbNodeId> = scene
            .nodes
            .get(&list_id)
            .map(|list| {
                list.children
                    .iter()
                    .copied()
                    .filter(|c| scene.nodes.get(c).is_some_and(|n| n.is_active))
                    .collect()
            })
            .unwrap_or_default();
        // Only a single-template list is an entry template; multi-child lists
        // hold authored items.
        let [template] = active_children.as_slice() else {
            continue;
        };
        let template = *template;

        // Engine list machinery rests its selection on one entry (the power
        // pip stacks park the cursor on the assignment-boundary pip, which
        // shows the selector arrow via the 'selected'-tag style conditions).
        let selected_index = defaults
            .lookup_path(&format!("{path}/selectedindex"))
            .and_then(|value| match value {
                crate::canvas::Value::Int(i) => Some(*i),
                crate::canvas::Value::Float(f) => Some(*f as i64),
                _ => None,
            });
        for index in 0..count.max(0) {
            let Some((clone_root, node_map)) = clone_subtree_as_sibling_with_map(scene, template)
            else {
                break;
            };
            // Mark materialised entries: scoped style semantics (ancestor
            // break conditions) apply within these subtrees only.
            if let Some(root) = scene.nodes.get_mut(&clone_root)
                && let Some(map) = root.raw.as_object_mut()
            {
                map.insert("_MaterialisedEntry_".to_string(), serde_json::Value::Bool(true));
            }
            if selected_index == Some(index)
                && let Some(root) = scene.nodes.get_mut(&clone_root)
                && !root
                    .style_tag_uuids
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(LIST_SELECTED_TAG_UUID))
            {
                root.style_tag_uuids.push(LIST_SELECTED_TAG_UUID.to_string());
            }
            let entry_namespace = format!("{path}/[{index:04}]");
            clone_subtree_field_ops(scene, &node_map, namespace, &entry_namespace);
        }
        deactivate_subtrees(scene, &std::collections::HashSet::from([template]));
    }
}

/// Duplicate the field-binding operations of a cloned subtree: every op whose
/// `widget` targets an original subtree node is deep-copied together with its
/// transitive input-operation closure; copied `_Pointer_` definitions get
/// fresh ids, `widget` refs retarget the cloned nodes, and inheriting
/// variable bindings are rewritten under the entry namespace (stripping the
/// instance namespace prefix the bindings inherited from the outer list).
fn clone_subtree_field_ops(
    scene: &mut BbScene,
    node_map: &std::collections::HashMap<BbNodeId, BbNodeId>,
    parent_namespace: &str,
    entry_namespace: &str,
) {
    // Index ops by pointer for closure walks.
    let ptr_index: std::collections::HashMap<String, usize> = scene
        .operations
        .iter()
        .enumerate()
        .filter_map(|(i, op)| {
            op.get("_Pointer_")
                .and_then(|v| v.as_str())
                .map(|p| (p.to_owned(), i))
        })
        .collect();

    // Field ops targeting original subtree nodes.
    let field_op_indices: Vec<usize> = scene
        .operations
        .iter()
        .enumerate()
        .filter_map(|(i, op)| {
            let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if !ty.contains("Field") {
                return None;
            }
            let widget = op
                .get("widget")
                .and_then(|v| v.as_str())
                .and_then(parse_points_to_or_ptr)?;
            node_map.contains_key(&widget).then_some(i)
        })
        .collect();
    if field_op_indices.is_empty() {
        return;
    }

    // Transitive closure of referenced operation indices.
    let mut closure: Vec<usize> = Vec::new();
    let mut queue: Vec<usize> = field_op_indices.clone();
    let mut seen: std::collections::HashSet<usize> = queue.iter().copied().collect();
    while let Some(index) = queue.pop() {
        closure.push(index);
        let mut refs: Vec<String> = Vec::new();
        collect_points_to_refs(&scene.operations[index], &mut refs);
        for ptr in refs {
            if let Some(&target) = ptr_index.get(&ptr)
                && seen.insert(target)
            {
                queue.push(target);
            }
        }
    }
    closure.sort_unstable();

    // Fresh pointer ids for every `_Pointer_` definition in the closure. Node
    // ids and op pointers share one numbering space in authored canvases, so
    // allocate beyond the maximum of both.
    let mut next_ptr: BbNodeId = {
        let max_node = scene.nodes.keys().max().copied().unwrap_or(0);
        let max_op = ptr_index
            .keys()
            .filter_map(|p| p.strip_prefix("ptr:").and_then(|n| n.parse::<BbNodeId>().ok()))
            .max()
            .unwrap_or(0);
        max_node.max(max_op).wrapping_add(1)
    };
    let mut op_ptr_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for &index in &closure {
        let mut def_ptrs: Vec<String> = Vec::new();
        collect_pointer_defs(&scene.operations[index], &mut def_ptrs);
        for ptr in def_ptrs {
            op_ptr_map.entry(ptr).or_insert_with(|| {
                let fresh = format!("ptr:{next_ptr}");
                next_ptr = next_ptr.wrapping_add(1);
                fresh
            });
        }
    }

    let strip_prefix = if parent_namespace.is_empty() {
        String::new()
    } else {
        format!("{parent_namespace}/")
    };
    let mut cloned: Vec<serde_json::Value> = Vec::with_capacity(closure.len());
    for &index in &closure {
        let mut copy = scene.operations[index].clone();
        rewrite_cloned_op(
            &mut copy,
            &op_ptr_map,
            node_map,
            &strip_prefix,
            entry_namespace,
        );
        // Copied component-parameter ops still represent THIS canvas's own
        // parameter interface (one copy per pip of the item's 'Max pipList'),
        // so the parent hops' slot wiring must rewire them like the original.
        if copy
            .get("_Type_")
            .and_then(|v| v.as_str())
            .is_some_and(|ty| ty.ends_with("ComponentParameter"))
        {
            copy["_ParamRelay_"] = serde_json::Value::Bool(true);
        }
        cloned.push(copy);
    }
    scene.operations.extend(cloned);
}

/// Collect `_PointsTo_:ptr:N` references anywhere in an op value.
fn collect_points_to_refs(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(ptr) = s.strip_prefix("_PointsTo_:") {
                out.push(ptr.to_owned());
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_points_to_refs(v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                collect_points_to_refs(v, out);
            }
        }
        _ => {}
    }
}

/// Collect `_Pointer_` definitions anywhere in an op value (an op element can
/// nest further pointer-bearing operation objects inline).
fn collect_pointer_defs(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(ptr) = map.get("_Pointer_").and_then(|v| v.as_str()) {
                out.push(ptr.to_owned());
            }
            for v in map.values() {
                collect_pointer_defs(v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                collect_pointer_defs(v, out);
            }
        }
        _ => {}
    }
}

/// Rewrite a cloned operation in place: fresh `_Pointer_` ids, remapped
/// `_PointsTo_` references, `widget` refs retargeted to cloned nodes, and
/// inheriting variable bindings namespaced under the entry.
fn rewrite_cloned_op(
    value: &mut serde_json::Value,
    op_ptr_map: &std::collections::HashMap<String, String>,
    node_map: &std::collections::HashMap<BbNodeId, BbNodeId>,
    strip_prefix: &str,
    entry_namespace: &str,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(ptr)) = map.get_mut("_Pointer_")
                && let Some(fresh) = op_ptr_map.get(ptr.as_str())
            {
                *ptr = fresh.clone();
            }
            let ty = map
                .get("_Type_")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let inherits = map
                .get("inheritsNamespace")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if ty.starts_with("BuildingBlocks_Bindings")
                && ty.ends_with("Variable")
                && inherits
                && let Some(serde_json::Value::String(binding)) = map.get_mut("binding")
                && !binding.is_empty()
                && !binding.starts_with('/')
            {
                let leaf = if !strip_prefix.is_empty() {
                    binding.strip_prefix(strip_prefix).unwrap_or(binding)
                } else {
                    binding.as_str()
                };
                *binding = format!("{entry_namespace}/{leaf}");
            }
            for (key, v) in map.iter_mut() {
                if key == "_Pointer_" || key == "binding" {
                    continue;
                }
                rewrite_cloned_op(v, op_ptr_map, node_map, strip_prefix, entry_namespace);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                rewrite_cloned_op(v, op_ptr_map, node_map, strip_prefix, entry_namespace);
            }
        }
        serde_json::Value::String(s) => {
            if let Some(ptr) = s.strip_prefix("_PointsTo_:") {
                if let Some(fresh) = op_ptr_map.get(ptr) {
                    *s = format!("_PointsTo_:{fresh}");
                } else if let Some(node) = parse_points_to_or_ptr(s)
                    && let Some(&clone) = node_map.get(&node)
                {
                    *s = format!("_PointsTo_:ptr:{clone}");
                }
            }
        }
        _ => {}
    }
}

/// [`clone_subtree_as_sibling`] variant that also returns the original→clone
/// node id map (needed to retarget cloned field operations).
fn clone_subtree_as_sibling_with_map(
    scene: &mut BbScene,
    root: BbNodeId,
) -> Option<(BbNodeId, std::collections::HashMap<BbNodeId, BbNodeId>)> {
    let parent_id = scene.nodes.get(&root)?.parent;
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = scene.nodes.get(&id) else {
            continue;
        };
        order.push(id);
        stack.extend(node.children.iter().copied());
    }
    let mut id_map: std::collections::HashMap<BbNodeId, BbNodeId> =
        std::collections::HashMap::with_capacity(order.len());
    for &id in &order {
        let clone_id = next_free_low_node_id_excluding(scene, &id_map);
        id_map.insert(id, clone_id);
    }
    for &id in &order {
        let Some(mut clone) = scene.nodes.get(&id).cloned() else {
            continue;
        };
        let clone_id = id_map[&id];
        clone.id = clone_id;
        clone.parent = if id == root {
            parent_id
        } else {
            clone.parent.and_then(|p| id_map.get(&p).copied())
        };
        clone.children = clone
            .children
            .iter()
            .filter_map(|c| id_map.get(c).copied())
            .collect();
        scene.nodes.insert(clone_id, clone);
    }
    let cloned_root = id_map[&root];
    if let Some(parent) = parent_id.and_then(|id| scene.nodes.get_mut(&id)) {
        parent.children.push(cloned_root);
    }
    Some((cloned_root, id_map))
}

// Registry-backed visibility gates: direct boolean variables and one-hop
// comparisons over registry-backed variables (the power off panels and the
// weapons column's heat gauge).

/// Apply registry-backed DIRECT boolean-variable visibility gates: a node
/// whose `IsActive`/`Instantiated` field binds STRAIGHT to a
/// `Bindings*BooleanVariable` with a registry value takes that value when it
/// is `false` (the power columns' `canvas_OffPanel.IsActive ← ispoweredoff`).
/// Chains, parameters and unresolved variables are left to the at-rest
/// heuristics in `bb_state_filter` — the medical capture baselines depend on
/// those staying untouched (deactivation-only, direct-binding-only on
/// purpose).
pub(crate) fn apply_registry_direct_gates(
    scene: &mut BbScene,
    defaults: &crate::defaults::DefaultValueRegistry,
) {
    let ptr_to_op: std::collections::HashMap<String, &serde_json::Value> = scene
        .operations
        .iter()
        .filter_map(|op| {
            op.get("_Pointer_")
                .and_then(|v| v.as_str())
                .map(|p| (p.to_owned(), op))
        })
        .collect();
    let mut gated_off: std::collections::HashSet<BbNodeId> = std::collections::HashSet::new();
    for op in &scene.operations {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        if !ty.ends_with("BooleanField") {
            continue;
        }
        let field = op.get("field").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(field, "IsActive" | "Instantiated") {
            continue;
        }
        let Some(widget) = op
            .get("widget")
            .and_then(|v| v.as_str())
            .and_then(parse_points_to_or_ptr)
        else {
            continue;
        };
        let Some(input) = op
            .get("input")
            .and_then(|v| v.as_str())
            .map(|s| s.strip_prefix("_PointsTo_:").unwrap_or(s))
        else {
            continue;
        };
        let Some(input_op) = ptr_to_op.get(input) else {
            continue;
        };
        let input_ty = input_op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        // Direct boolean variable: apply its registry value.
        if input_ty.starts_with("BuildingBlocks_Bindings") && input_ty.ends_with("BooleanVariable") {
            let Some(binding) = input_op.get("binding").and_then(|v| v.as_str()) else {
                continue;
            };
            let resolved = defaults.lookup_path(binding).map(|value| match value {
                crate::canvas::Value::Bool(b) => *b,
                crate::canvas::Value::Int(i) => *i != 0,
                crate::canvas::Value::Float(f) => *f != 0.0,
                crate::canvas::Value::Str(s) | crate::canvas::Value::Guid(s) => {
                    matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
                }
            });
            if resolved == Some(false) {
                gated_off.insert(widget);
            }
            continue;
        }
        // One-hop comparison over a registry-backed variable (the heat bars'
        // `IsActive ← tempIndicator/maxTemp > 0` with the weapons pool's
        // registry 0): the comparison's variable operand must have a registry
        // value — unresolved variables keep the at-rest heuristics.
        if input_ty.ends_with("BooleanFromNumber") || input_ty.ends_with("BooleanFromInteger") {
            let operand = ["input", "inputL"].iter().find_map(|key| {
                let ptr = input_op
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .map(|s| s.strip_prefix("_PointsTo_:").unwrap_or(s))?;
                ptr_to_op.get(ptr)
            });
            let Some(operand_op) = operand else {
                continue;
            };
            // Allow one NumberFromInteger conversion between comparison and variable.
            let operand_op = if operand_op
                .get("_Type_")
                .and_then(|v| v.as_str())
                .is_some_and(|ty| ty.ends_with("NumberFromInteger"))
            {
                let Some(inner) = operand_op
                    .get("input")
                    .and_then(|v| v.as_str())
                    .map(|s| s.strip_prefix("_PointsTo_:").unwrap_or(s))
                    .and_then(|ptr| ptr_to_op.get(ptr))
                else {
                    continue;
                };
                inner
            } else {
                operand_op
            };
            let operand_ty = operand_op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if !(operand_ty.starts_with("BuildingBlocks_Bindings") && operand_ty.ends_with("Variable")) {
                continue;
            }
            let Some(binding) = operand_op.get("binding").and_then(|v| v.as_str()) else {
                continue;
            };
            if defaults.lookup_path(binding).is_none() {
                continue;
            }
            let resolver = crate::bb_bindings::BindingResolver::from_operations(&scene.operations);
            let gate = ["IsActive", "Instantiated"].iter().find_map(|field_name| {
                if *field_name == field {
                    resolver.resolve_field_bool(widget, field_name, defaults)
                } else {
                    None
                }
            });
            if gate == Some(false) {
                gated_off.insert(widget);
            }
        }
    }
    if !gated_off.is_empty() {
        deactivate_subtrees(scene, &gated_off);
    }
}
