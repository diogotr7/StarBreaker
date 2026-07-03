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

// Consolidated engine chunk 04 (formerly: array_list_tests.part, param_relay_tests.part, param_leak_tests.part, widget_standard_expansion.part, scrollbar_expansion.part, widget_standard_expansion_tests.part).
//   array_list_tests.part: Tests for arrayVariable-driven WidgetList materialisation (array_list.part):
//   param_relay_tests.part: Tests for multi-hop component-parameter relays (part_06's dynamic param
//   param_leak_tests.part: Tests for parent param wiring target selection vs merged descendant defs
//   widget_standard_expansion.part: Widget-standard template expansion.
//   scrollbar_expansion.part: ComponentScrollBar standard-template expansion helpers.
//   widget_standard_expansion_tests.part: Tests for widget-standard template expansion (see

// Tests for arrayVariable-driven WidgetList materialisation (array_list.part):
// the power screen's pip stack — `list_PowerBars` declares
// `arrayVariable: "pipList"` and the engine instantiates its single template
// child once per array entry, each entry's inheriting bindings namespaced
// `<listpath>/[000j]/…`. Counts come from the defaults registry and differ per
// outer list instance (weapons 4 / engines 6 / shields 4).
#[cfg(test)]
mod tests_array_list {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::defaults::DefaultValueRegistry;

    /// Item canvas wired like `gen_mc_s_powerlistitem`: a WidgetList with
    /// `arrayVariable: "pipList"` holding ONE pip template whose SizeX is
    /// bound to the per-pip `pipAmount` variable.
    fn item_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_ArrayItem",
            "_RecordId_": "00000000-0000-0000-0000-0000000a0001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 600.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "item_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetList",
                     "name": "pip_list", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "arrayVariable": "pipList",
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer", "direction": "ColumnReverse"}},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "pip_template", "parent": "_PointsTo_:ptr:2", "isActive": true,
                     "sizing": {"_Type_": "BuildingBlocks_Size",
                       "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 0.5, "behavior": "Percent"},
                       "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 0.0667, "behavior": "Percent"}}}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:3", "field": "SizeX", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                     "path": [], "binding": "pipAmount", "inheritsNamespace": true}
                ]
            }
        })
    }

    /// Parent canvas with the count-binding list-slot signature (`items`) whose
    /// item canvas contains the arrayVariable pip list.
    fn list_parent_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_ArrayParent",
            "_RecordId_": "00000000-0000-0000-0000-0000000a0002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 400.0, "y": 600.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "list_container", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_LayoutPolicyFlexContainer", "direction": "Row"}},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_listItem", "parent": "_PointsTo_:ptr:2", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_arrayitem.json"}
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
        if path.to_ascii_lowercase().contains("test_arrayitem") {
            Ok(item_canvas())
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    /// Each outer instance materialises its OWN pip count from the registry
    /// (`items/[000i]/pipList`), the template deactivates, and per-pip
    /// bindings are namespaced `items/[000i]/pipList/[000j]/…`.
    #[test]
    fn array_list_materialises_entries_per_instance_count() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("items", crate::canvas::Value::Int(2));
        // Mixed-case keys: engine binding paths are case-insensitive (the
        // power master reads `resourcenetworkUi`/`resourcenetworkui` for the
        // same variable in one canvas).
        defaults.insert_path("items/[0000]/piplist", crate::canvas::Value::Int(3));
        defaults.insert_path("items/[0001]/pipList", crate::canvas::Value::Int(2));
        let scene = resolve_canvas_graph_with_defaults(
            &list_parent_canvas(),
            Some("drak"),
            &fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let active_pips = scene
            .nodes
            .values()
            .filter(|n| n.name == "pip_template" && n.is_active)
            .count();
        assert_eq!(active_pips, 5, "3 + 2 pip entries must materialise");
        let inactive_templates = scene
            .nodes
            .values()
            .filter(|n| n.name == "pip_template" && !n.is_active)
            .count();
        assert_eq!(inactive_templates, 2, "each instance's authored template deactivates");

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
        for expected in [
            "items/[0000]/pipList/[0000]/pipAmount",
            "items/[0000]/pipList/[0002]/pipAmount",
            "items/[0001]/pipList/[0001]/pipAmount",
        ] {
            assert!(
                bindings.iter().any(|b| b == expected),
                "expected per-pip binding {expected}; got {bindings:?}"
            );
        }
        assert!(
            !bindings.iter().any(|b| b == "items/[0001]/pipList/[0002]/pipAmount"),
            "instance 1 has only 2 pips; got {bindings:?}"
        );
    }

    /// Registry-resolvable `SizeX` bindings on materialised entries override
    /// the authored editor-placeholder sizing (the pip template's authored
    /// 0.5-width is the >15-pip fallback; bound values replace it).
    #[test]
    fn bound_size_fields_override_authored_sizing_per_entry() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("items", crate::canvas::Value::Int(1));
        defaults.insert_path("items/[0000]/piplist", crate::canvas::Value::Int(2));
        defaults.insert_path(
            "items/[0000]/piplist/[0000]/pipamount",
            crate::canvas::Value::Float(0.25),
        );
        defaults.insert_path(
            "items/[0000]/piplist/[0001]/pipamount",
            crate::canvas::Value::Float(0.75),
        );
        let scene = resolve_canvas_graph_with_defaults(
            &list_parent_canvas(),
            Some("drak"),
            &fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let mut widths: Vec<f32> = scene
            .nodes
            .values()
            .filter(|n| n.name == "pip_template" && n.is_active)
            .map(|n| match n.sizing.width {
                crate::bb_scene::BbValue::Percent(v) => v,
                ref other => panic!("expected Percent width, got {other:?}"),
            })
            .collect();
        widths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(widths, vec![0.25, 0.75], "per-entry bound SizeX values apply");
    }

    /// Without a registry count for the array path the mechanism stays
    /// disengaged: the authored template keeps rendering (other list models —
    /// the count-binding slot signature — own such lists).
    #[test]
    fn array_list_absent_count_leaves_template_untouched() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("items", crate::canvas::Value::Int(1));
        let scene = resolve_canvas_graph_with_defaults(
            &list_parent_canvas(),
            Some("drak"),
            &fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");
        let templates: Vec<bool> = scene
            .nodes
            .values()
            .filter(|n| n.name == "pip_template")
            .map(|n| n.is_active)
            .collect();
        assert_eq!(templates, vec![true], "no registry count: template stays as authored");
    }

    /// A DIRECT BooleanVariable IsActive gate with a registry value applies
    /// it (the power columns' `canvas_OffPanel.IsActive ← ispoweredoff` with
    /// registry `false` hides the panel). Unresolved gates stay untouched —
    /// the medical capture states rely on at-rest heuristics, not this rule.
    #[test]
    fn direct_boolean_variable_gate_applies_registry_value() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_DirectGate",
            "_RecordId_": "00000000-0000-0000-0000-0000000a0009",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "off_panel", "parent": "_PointsTo_:ptr:1", "isActive": true},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "off_panel_child", "parent": "_PointsTo_:ptr:2", "isActive": true},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "ungated", "parent": "_PointsTo_:ptr:1", "isActive": true}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsBooleanField",
                     "widget": "_PointsTo_:ptr:2", "field": "IsActive", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                     "path": [], "binding": "ispoweredoff", "inheritsNamespace": true},
                    {"_Type_": "BuildingBlocks_BindingsBooleanField",
                     "widget": "_PointsTo_:ptr:4", "field": "IsActive", "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                     "path": [], "binding": "someunboundthing", "inheritsNamespace": true}
                ]
            }
        });
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("ispoweredoff", crate::canvas::Value::Bool(false));
        let scene = resolve_canvas_graph_with_defaults(
            &canvas,
            Some("drak"),
            &|p| Err(format!("no record for '{p}'")),
            None,
            None,
            &defaults,
        )
        .expect("resolve");
        let by_name = |name: &str| {
            scene
                .nodes
                .values()
                .find(|n| n.name == name)
                .map(|n| n.is_active)
                .expect(name)
        };
        assert!(!by_name("off_panel"), "registry false must hide the gated panel");
        assert!(!by_name("off_panel_child"), "the gated subtree hides with it");
        assert!(by_name("ungated"), "an unresolved gate stays as authored");
    }

    /// A zero registry count means an empty array at rest: the template hides.
    #[test]
    fn array_list_zero_count_hides_template() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("items", crate::canvas::Value::Int(1));
        defaults.insert_path("items/[0000]/piplist", crate::canvas::Value::Int(0));
        let scene = resolve_canvas_graph_with_defaults(
            &list_parent_canvas(),
            Some("drak"),
            &fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");
        let templates: Vec<bool> = scene
            .nodes
            .values()
            .filter(|n| n.name == "pip_template")
            .map(|n| n.is_active)
            .collect();
        assert_eq!(templates, vec![false], "zero count: template deactivates, no clones");
    }

    /// A TOP-LEVEL list (empty namespace) whose `arrayVariable` is a multi-segment
    /// engine-state path (e.g. the compass `FlightController/Compass/Ticks`)
    /// resolves its count directly from that path — the authored value is an
    /// absolute engine reference, not a namespace-relative name. At static rest the
    /// flight controller pushes no ticks, so the registry pins count 0 and the lone
    /// authored template (a per-entry clone template, not a standalone item)
    /// deactivates: the faithful empty compass. A BARE single-segment name
    /// ("itemList") at empty namespace stays UI-local and is left untouched.
    fn top_level_engine_list_canvas(array_var: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_TopLevelEngineList",
            "_RecordId_": "00000000-0000-0000-0000-0000000a000c",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetList",
                     "name": "list_engine", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "arrayVariable": array_var},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "tick_template", "parent": "_PointsTo_:ptr:2", "isActive": true}
                ],
                "operations": []
            }
        })
    }

    #[test]
    fn top_level_engine_state_array_list_resolves_count_from_path() {
        let resolve = |array_var: &str, defaults: &DefaultValueRegistry| {
            let scene = resolve_canvas_graph_with_defaults(
                &top_level_engine_list_canvas(array_var),
                Some("drak"),
                &|p| Err(format!("no record for '{p}'")),
                None,
                None,
                defaults,
            )
            .expect("resolve");
            scene
                .nodes
                .values()
                .find(|n| n.name == "tick_template")
                .map(|n| n.is_active)
                .expect("template node")
        };

        // Multi-segment engine-state path pinned to 0 → template deactivates.
        let mut zero = DefaultValueRegistry::default();
        zero.insert_path("FlightController/Compass/Ticks", crate::canvas::Value::Int(0));
        assert!(
            !resolve("FlightController/Compass/Ticks", &zero),
            "engine-state list pinned 0 at static rest: template deactivates"
        );

        // Same path with no pin → still skipped (template stays as authored).
        let empty = DefaultValueRegistry::default();
        assert!(
            resolve("FlightController/Compass/Ticks", &empty),
            "no registry count: engine-state list template stays as authored"
        );

        // A BARE single-segment name is UI-local: even a same-named pin must NOT
        // resolve it at empty namespace (guards the power outer `pipList`).
        let mut bare = DefaultValueRegistry::default();
        bare.insert_path("itemlist", crate::canvas::Value::Int(0));
        assert!(
            resolve("itemList", &bare),
            "bare single-segment name needs a namespace: untouched at top level"
        );
    }
}

// Tests for multi-hop component-parameter relays (part_06's dynamic param
// field-binding injection): the power screen's pip sizing chain passes
// `pipsLengthMax` through THREE canvas levels by slot wiring
// (master `IntegerField ParamInput0 ← variable` → power 'pips length max' →
// powerlists 'Max Piplist' → item 'Max pipList'). Each hop's injection clones
// the parent's input subgraph onto the child's parameter op; clones of
// parameter ops are relay markers that the NEXT hop up must rewire again.
#[cfg(test)]
mod tests_param_relay {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::defaults::DefaultValueRegistry;

    /// Leaf canvas: a widget whose SizeY is bound to its own integer
    /// component parameter (ParamInput1).
    fn leaf_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_RelayLeaf",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0003",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "leaf_widget", "isActive": true,
                     "sizing": {"_Type_": "BuildingBlocks_Size",
                       "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 1.0, "behavior": "Percent"},
                       "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 0.05, "behavior": "Percent"}}}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:1", "field": "SizeY", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberFromInteger",
                     "asSeconds": false, "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                     "name": "Leaf Max", "parameter": "ParamInput1", "defaultValue": 0}
                ]
            }
        })
    }

    /// Middle canvas: declares its own integer parameter (ParamInput0) and
    /// relays it to a list-MATERIALISED leaf slot's ParamInput1 (the power
    /// screen's system columns all receive the same 'Max Piplist'; relays
    /// only arise on materialised-instance hops).
    fn middle_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_RelayMiddle",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "middle_root", "isActive": true},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "list_container", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_LayoutPolicyFlexContainer", "direction": "Row"}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_leaf", "parent": "_PointsTo_:ptr:4", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_relayleaf.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsBooleanField",
                     "widget": "_PointsTo_:ptr:4", "field": "Instantiated", "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsBooleanFromInteger",
                     "type": "Greater", "inputL": "_PointsTo_:ptr:12", "inputR": null, "value": 0},
                    {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "items", "inheritsNamespace": true},
                    {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "items/[0000]/flag", "inheritsNamespace": true},
                    {"_Type_": "BuildingBlocks_BindingsIntegerField",
                     "widget": "_PointsTo_:ptr:2", "field": "ParamInput1", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                     "name": "Middle Max", "parameter": "ParamInput0", "defaultValue": 0}
                ]
            }
        })
    }

    /// Root canvas: wires the middle instance's ParamInput0 from an engine
    /// variable (registry-backed).
    fn root_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_RelayRoot",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_middle", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_relaymiddle.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsIntegerField",
                     "widget": "_PointsTo_:ptr:2", "field": "ParamInput0", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "maxpips", "inheritsNamespace": true}
                ]
            }
        })
    }

    /// Frame canvas above the wiring root: in the real MFD the master canvas
    /// (which holds the variable wiring) is itself MERGED into the frame, so
    /// its field ops carry `_MergedOp_` and the global by-name parameter
    /// fallback cannot see them — the relay chain must work structurally.
    fn frame_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_RelayFrame",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0000",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "frame_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_content", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_relayroot.json"}
                ],
                "operations": []
            }
        })
    }

    fn fetcher(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_relayleaf") {
            Ok(leaf_canvas())
        } else if p.contains("test_relaymiddle") {
            Ok(middle_canvas())
        } else if p.contains("test_relayroot") {
            Ok(root_canvas())
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    /// The registry-backed variable must reach EVERY leaf instance's
    /// parameter through two relay hops, and each leaf's bound SizeY must
    /// apply to its sizing.
    #[test]
    fn parameter_relays_across_two_canvas_hops_to_all_instances() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("maxpips", crate::canvas::Value::Int(6));
        defaults.insert_path("items", crate::canvas::Value::Int(2));
        let scene = resolve_canvas_graph_with_defaults(
            &frame_canvas(),
            Some("drak"),
            &fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let heights: Vec<f32> = scene
            .nodes
            .values()
            .filter(|n| n.name == "leaf_widget")
            .map(|n| match n.sizing.height {
                crate::bb_scene::BbValue::Percent(v) => v,
                ref other => panic!("expected Percent height, got {other:?}"),
            })
            .collect();
        assert_eq!(heights.len(), 2, "both leaf instances must merge");
        for v in heights {
            assert!((v - 6.0).abs() < 1e-6, "leaf SizeY = relayed maxpips, got {v}");
        }
    }

    /// Root variant of the real power master: the middle canvas is selected by
    /// a Pass-1 `Set Canvas` style entry whose conditions match the host
    /// WidgetCanvas (type `Canvas`) — there is NO `matchTo` name. The host's
    /// parameter wiring must still inject into the merged child.
    fn root_canvas_pass1() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_RelayRootPass1",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0004",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_CanvasStyles",
                    "entries": [
                        {"_Type_": "BuildingBlocks_StyleEntry", "name": "Set Canvas",
                         "conditionsList": [
                            {"_Type_": "BuildingBlocks_StyleConditionList", "name": "cl",
                             "conditions": [
                                {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"}
                             ]}
                         ],
                         "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierRecordRef",
                             "field": {"_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                       "value": "file://./test_relaymiddle.json"}}
                         ],
                         "transitions": []}
                    ]
                },
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_interchangeable", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": ""}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsIntegerField",
                     "widget": "_PointsTo_:ptr:2", "field": "ParamInput0", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "maxpips", "inheritsNamespace": true}
                ]
            }
        })
    }

    fn fetcher_pass1(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_relayroot") {
            Ok(root_canvas_pass1())
        } else {
            fetcher(path)
        }
    }

    /// Same relay chain, but the wiring root enters via a Pass-1 conditional
    /// canvas reference (the power master's `canvas_Interchangeable`).
    #[test]
    fn parameter_relays_through_pass1_conditional_canvas_reference() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("maxpips", crate::canvas::Value::Int(6));
        defaults.insert_path("items", crate::canvas::Value::Int(2));
        let scene = resolve_canvas_graph_with_defaults(
            &frame_canvas(),
            Some("drak"),
            &fetcher_pass1,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let heights: Vec<f32> = scene
            .nodes
            .values()
            .filter(|n| n.name == "leaf_widget")
            .map(|n| match n.sizing.height {
                crate::bb_scene::BbValue::Percent(v) => v,
                ref other => panic!("expected Percent height, got {other:?}"),
            })
            .collect();
        assert_eq!(heights.len(), 2, "both leaf instances must merge");
        for v in heights {
            assert!((v - 6.0).abs() < 1e-6, "leaf SizeY = relayed maxpips, got {v}");
        }
    }

    /// Middle canvas whose leaf slot is MATERIALISED from a list count (the
    /// power lists' per-system columns): every cloned instance must receive
    /// the slot's parameter wiring, not just the authored original.
    fn middle_canvas_list() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_RelayMiddleList",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0005",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "middle_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "list_container", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_LayoutPolicyFlexContainer", "direction": "Row"}},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_listItem", "parent": "_PointsTo_:ptr:2", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_relayleaf.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsBooleanField",
                     "widget": "_PointsTo_:ptr:2", "field": "Instantiated", "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsBooleanFromInteger",
                     "type": "Greater", "inputL": "_PointsTo_:ptr:12", "inputR": null, "value": 0},
                    {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "items", "inheritsNamespace": true},
                    {"_Pointer_": "ptr:13", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "items/[0000]/flag", "inheritsNamespace": true},
                    {"_Type_": "BuildingBlocks_BindingsIntegerField",
                     "widget": "_PointsTo_:ptr:5", "field": "ParamInput1", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                     "name": "Middle Max", "parameter": "ParamInput0", "defaultValue": 0}
                ]
            }
        })
    }

    fn fetcher_list(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_relayleaf") {
            Ok(leaf_canvas())
        } else if p.contains("test_relaymiddle") {
            Ok(middle_canvas_list())
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    /// Child canvas whose content merges via a Pass-1 `Set Canvas` entry.
    fn pass1_content_child() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_Pass1Child",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0006",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_CanvasStyles",
                    "entries": [
                        {"_Type_": "BuildingBlocks_StyleEntry", "name": "Set Canvas",
                         "conditionsList": [
                            {"_Type_": "BuildingBlocks_StyleConditionList", "name": "cl",
                             "conditions": [
                                {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"}
                             ]}
                         ],
                         "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierRecordRef",
                             "field": {"_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                                       "value": "file://./test_pass1deep.json"}}
                         ],
                         "transitions": []}
                    ]
                },
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "child_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_inner", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true, "canvas": ""}
                ],
                "operations": []
            }
        })
    }

    fn pass1_deep_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_Pass1Deep",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0007",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "deep_marker", "isActive": true}
                ],
                "operations": []
            }
        })
    }

    /// Parent whose Pass-1-content child is MATERIALISED twice from a list
    /// count (the power columns): per-instance namespaces re-merge the
    /// per-instance content; un-namespaced canvases stay once-only.
    fn pass1_parent_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_Pass1Parent",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0008",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 200.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "list_container", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_LayoutPolicyFlexContainer", "direction": "Row"}},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_listItem", "parent": "_PointsTo_:ptr:2", "isActive": true,
                     "instantiated": true, "canvas": "file://./test_pass1child.json"}
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

    fn fetcher_pass1_content(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_pass1child") {
            Ok(pass1_content_child())
        } else if p.contains("test_pass1deep") {
            Ok(pass1_deep_canvas())
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    /// EVERY materialised instance of a child canvas merges its Pass-1
    /// content (the power columns' OffPanel/heat-bar canvases): the
    /// once-only set is scoped by the instance namespace.
    #[test]
    fn repeated_child_instances_each_merge_pass1_content() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("items", crate::canvas::Value::Int(2));
        let scene = resolve_canvas_graph_with_defaults(
            &pass1_parent_canvas(),
            Some("drak"),
            &fetcher_pass1_content,
            None,
            None,
            &defaults,
        )
        .expect("resolve");
        let markers = scene
            .nodes
            .values()
            .filter(|n| n.name == "deep_marker")
            .count();
        assert_eq!(markers, 2, "both slot instances must carry the Pass-1 content");
    }

    /// Every list-materialised slot instance receives the relayed parameter
    /// (the power screen's three system columns all get 'Max Piplist').
    #[test]
    fn parameter_relays_reach_all_materialised_list_instances() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("maxpips", crate::canvas::Value::Int(6));
        defaults.insert_path("items", crate::canvas::Value::Int(2));
        let scene = resolve_canvas_graph_with_defaults(
            &root_canvas(),
            Some("drak"),
            &fetcher_list,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let heights: Vec<f32> = scene
            .nodes
            .values()
            .filter(|n| n.name == "leaf_widget" && n.is_active)
            .map(|n| match n.sizing.height {
                crate::bb_scene::BbValue::Percent(v) => v,
                ref other => panic!("expected Percent height, got {other:?}"),
            })
            .collect();
        assert_eq!(heights.len(), 2, "two materialised instances must merge");
        for v in heights {
            assert!((v - 6.0).abs() < 1e-6, "instance SizeY = relayed maxpips, got {v}");
        }
    }

    /// Mirror of the master-mode display (`HC_HUD_Ship_Master_Mode_Display_Master`):
    /// the interchangeable host `WidgetCanvas` carries an AUTHORED `canvas`
    /// default pointing at ONE manufacturer's variant (the editor placeholder —
    /// here AEGS) while a brand-style `Set Canvas` modifier swaps it to the
    /// active ship's variant (DRAK). Only the active variant must render. The
    /// authored placeholder matches a NON-active brand entry's canvas-reference,
    /// so Pass 2 must skip it (the mode-switch guard's documented intent is "any
    /// conditional entry, regardless of which Pass 1 selected"). Without the
    /// skip both overlap — the AEGS ship-in-oval rendered on top of the DRAK
    /// layout. Counterexample: the velocity-num sibling authors `canvas: ""`, so
    /// only the modifier variant ever instantiated and the bug was latent.
    fn variant_marker_canvas(record: &str, marker: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": format!("BuildingBlocks_Canvas.{record}"),
            "_RecordId_": "00000000-0000-0000-0000-0000000r0010",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": marker, "isActive": true}
                ],
                "operations": []
            }
        })
    }

    fn brand_set_canvas_entry(brand_id: &str, variant_url: &str) -> serde_json::Value {
        serde_json::json!({
            "_Type_": "BuildingBlocks_BrandStyles",
            "brandIdentifier": brand_id,
            "entries": [
                {"_Type_": "BuildingBlocks_StyleEntry", "name": "Set Canvas",
                 "conditionsList": [
                    {"_Type_": "BuildingBlocks_StyleConditionList", "name": "cl",
                     "conditions": [
                        {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"}
                     ]}
                 ],
                 "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierRecordRef",
                     "field": {"_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                               "value": variant_url}}
                 ],
                 "transitions": []}
            ]
        })
    }

    fn interchangeable_brand_swap_root() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_InterchangeableBrandSwap",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0009",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "brandStyles": [
                    brand_set_canvas_entry(
                        "file://./../styles/s_aegs_hud.json", "file://./test_aegs_variant.json"),
                    brand_set_canvas_entry(
                        "file://./../styles/s_drak_hud.json", "file://./test_drak_variant.json")
                ],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_interchangeable", "parent": "_PointsTo_:ptr:1",
                     "isActive": true, "instantiated": true,
                     "canvas": "file://./test_aegs_variant.json"}
                ],
                "operations": []
            }
        })
    }

    fn fetcher_brand_swap(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_aegs_variant") {
            Ok(variant_marker_canvas("Test_AegsVariant", "aegs_marker"))
        } else if p.contains("test_drak_variant") {
            Ok(variant_marker_canvas("Test_DrakVariant", "drak_marker"))
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    #[test]
    fn interchangeable_authored_default_does_not_overlay_active_brand_variant() {
        let defaults = DefaultValueRegistry::default();
        let scene = resolve_canvas_graph_with_defaults(
            &interchangeable_brand_swap_root(),
            Some("drak"),
            &fetcher_brand_swap,
            None,
            None,
            &defaults,
        )
        .expect("resolve");
        let drak = scene.nodes.values().filter(|n| n.name == "drak_marker").count();
        let aegs = scene.nodes.values().filter(|n| n.name == "aegs_marker").count();
        assert_eq!(drak, 1, "active DRAK brand variant must render");
        assert_eq!(
            aegs, 0,
            "non-active AEGS authored-default placeholder must NOT overlay the active variant"
        );
    }

    /// A child canvas with a single uniquely-named node, so a test can assert
    /// whether that child's content merged into the parent.
    fn named_leaf_child(record: &str, node_name: &str) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": format!("BuildingBlocks_Canvas.{record}"),
            "_RecordId_": format!("00000000-0000-0000-0000-{:0>12}", node_name.len()),
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": node_name, "isActive": true}
                ]
            }
        })
    }

    /// The LR-indicator master pattern: an UN-NAMESPACED master with TWO
    /// interchangeable WidgetCanvas slots, each selected by its own `Set Canvas
    /// Left/Right` style entry via a distinct tag. `slot_width` controls whether
    /// the slots TILE (sub-full, e.g. `0.5` half-width laid out side-by-side —
    /// both visible) or OVERLAY (full-size `1.0` — mutually exclusive modes,
    /// like MC_S_Self_Master's five view modes).
    fn interchangeable_two_slot_master(slot_width: f64) -> serde_json::Value {
        let left_tag = "aaaaaaaa-0000-0000-0000-0000000000l1";
        let right_tag = "bbbbbbbb-0000-0000-0000-0000000000r1";
        let entry = |name: &str, tag: &str, child: &str| {
            serde_json::json!({
                "_Type_": "BuildingBlocks_StyleEntry", "name": name,
                "conditionsList": [{"_Type_": "BuildingBlocks_StyleConditionList", "name": "cl",
                    "conditions": [
                        {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Canvas"},
                        {"_Type_": "BuildingBlocks_StyleSelectorConditionTag", "tag": {"_RecordId_": tag}}
                    ]}],
                "modifiers": [{"_Type_": "BuildingBlocks_FieldModifierRecordRef",
                    "field": {"_Type_": "BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord",
                              "value": child}}],
                "transitions": []
            })
        };
        let slot = |ptr: &str, name: &str, anchor_x: f64, tag: &str| {
            serde_json::json!({
                "_Pointer_": ptr, "_Type_": "BuildingBlocks_WidgetCanvas",
                "name": name, "parent": "_PointsTo_:ptr:1", "isActive": true,
                "instantiated": true, "canvas": "",
                "anchor": {"_Type_": "Vec3", "x": anchor_x, "y": 0.0, "z": 0.0},
                "sizing": {"_Type_": "BuildingBlocks_Size",
                    "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": slot_width, "behavior": "Percent"},
                    "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 1.0, "behavior": "Percent"}},
                "styleTags": [{"_RecordId_": tag}]
            })
        };
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_TwoSlotMaster",
            "_RecordId_": "00000000-0000-0000-0000-00000000aa01",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_CanvasStyles",
                    "entries": [
                        entry("Set Canvas Left", left_tag, "file://./test_left_child.json"),
                        entry("Set Canvas Right", right_tag, "file://./test_right_child.json")
                    ]
                },
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root", "isActive": true},
                    slot("ptr:2", "canvas_left", 0.0, left_tag),
                    slot("ptr:3", "canvas_right", 1.0, right_tag)
                ]
            }
        })
    }

    fn fetcher_two_slot(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_left_child") {
            Ok(named_leaf_child("Test_LeftChild", "leaf_left"))
        } else if p.contains("test_right_child") {
            Ok(named_leaf_child("Test_RightChild", "leaf_right"))
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    /// Two HALF-WIDTH tiling slots (the LR-indicator master): BOTH columns'
    /// content must merge — they are laid out side-by-side and both visible.
    #[test]
    fn tiling_canvas_slots_both_merge_for_unnamespaced_master() {
        let scene =
            resolve_canvas_graph(&interchangeable_two_slot_master(0.5), Some("drak"), &fetcher_two_slot)
                .expect("resolve");
        let names: std::collections::HashSet<&str> =
            scene.nodes.values().map(|n| n.name.as_str()).collect();
        assert!(names.contains("leaf_left"), "left tiling slot content must merge; nodes={names:?}");
        assert!(names.contains("leaf_right"), "right tiling slot content must merge; nodes={names:?}");
    }

    /// Two FULL-SIZE overlay slots (MC_S_Self_Master's view-mode pattern): only
    /// the single at-rest pick merges — they stay mutually exclusive, guarding
    /// the sub-full discriminator from over-merging overlay modes.
    #[test]
    fn full_size_overlay_slots_stay_mutually_exclusive() {
        let scene =
            resolve_canvas_graph(&interchangeable_two_slot_master(1.0), Some("drak"), &fetcher_two_slot)
                .expect("resolve");
        let names: std::collections::HashSet<&str> =
            scene.nodes.values().map(|n| n.name.as_str()).collect();
        let merged = [names.contains("leaf_left"), names.contains("leaf_right")]
            .iter()
            .filter(|&&b| b)
            .count();
        assert_eq!(merged, 1, "full-size overlay modes stay mutually exclusive; nodes={names:?}");
    }
}

// Tests for parent param wiring target selection vs merged descendant defs
// (part_06's `child_component_parameter_targets`): a same-slot/same-kind
// ComponentParameter merged from the child's own descendants was already
// wired during the child's resolve and must not steal the value (the power
// master's pipsLengthMax ParamInput0 leaking into the battery card's
// batteryremaining as a stray "6"). The frame canvas above the wiring root
// mirrors the real MFD: the master's field ops are merged (`_MergedOp_`) so
// the global by-field-name parameter fallback cannot see them.
#[cfg(test)]
mod tests_param_leak {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::defaults::DefaultValueRegistry;

    /// A canvas's parameter wiring targets the child's OWN authored defs:
    /// a same-slot/same-kind parameter merged from the child's own
    /// descendant (already wired during the child's resolve) must not steal
    /// the value (the power master's `pipsLengthMax` ParamInput0 leaking
    /// into the battery card's `batteryremaining` ParamInput0 as a "6").
    fn leak_grandchild_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_LeakGrandchild",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0011",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "grandchild_widget", "isActive": true,
                     "sizing": {"_Type_": "BuildingBlocks_Size",
                       "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 1.0, "behavior": "Percent"},
                       "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 0.05, "behavior": "Percent"}}}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:1", "field": "SizeY", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberFromInteger",
                     "asSeconds": false, "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                     "name": "batteryremaining", "parameter": "ParamInput0", "defaultValue": 0}
                ]
            }
        })
    }

    fn leak_middle_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_LeakMiddle",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0012",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "middle_root", "isActive": true},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "middle_widget", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "sizing": {"_Type_": "BuildingBlocks_Size",
                       "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 1.0, "behavior": "Percent"},
                       "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 0.05, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_grandchild", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_leakgrandchild.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:3", "field": "SizeY", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberFromInteger",
                     "asSeconds": false, "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                     "name": "pips length max", "parameter": "ParamInput0", "defaultValue": 0}
                ]
            }
        })
    }

    fn leak_root_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_LeakRoot",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0013",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_middle", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_leakmiddle.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsIntegerField",
                     "widget": "_PointsTo_:ptr:2", "field": "ParamInput0", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "path": [], "binding": "maxpips", "inheritsNamespace": true}
                ]
            }
        })
    }

    /// Frame above the wiring root, mirroring the real MFD: the master's
    /// field ops are merged (`_MergedOp_`) so the global by-field-name
    /// parameter fallback cannot see them — the leak under test is the
    /// injection path's target selection.
    fn leak_frame_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_LeakFrame",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0014",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "frame_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_content", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_leakroot.json"}
                ],
                "operations": []
            }
        })
    }

    fn leak_fetcher(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_leakgrandchild") {
            Ok(leak_grandchild_canvas())
        } else if p.contains("test_leakmiddle") {
            Ok(leak_middle_canvas())
        } else if p.contains("test_leakroot") {
            Ok(leak_root_canvas())
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    #[test]
    fn parameter_wiring_skips_merged_descendant_defs() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("maxpips", crate::canvas::Value::Int(6));
        let scene = resolve_canvas_graph_with_defaults(
            &leak_frame_canvas(),
            Some("drak"),
            &leak_fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let height = |name: &str| {
            scene
                .nodes
                .values()
                .find(|n| n.name == name)
                .map(|n| match n.sizing.height {
                    crate::bb_scene::BbValue::Percent(v) => v,
                    ref other => panic!("expected Percent height, got {other:?}"),
                })
                .unwrap_or_else(|| panic!("node '{name}' missing"))
        };
        assert!(
            (height("middle_widget") - 6.0).abs() < 1e-6,
            "the child's OWN param must receive the wired value, got {}",
            height("middle_widget")
        );
        assert!(
            (height("grandchild_widget") - 0.05).abs() < 1e-6,
            "a merged descendant's same-slot param must NOT receive the value, got {}",
            height("grandchild_widget")
        );
    }


    /// The OUTPUT card's value chain: master wires a NUMBER var into the
    /// power canvas's ParamInput2; power's own 'total' def relays it into the
    /// output-info slot's ParamInput0; the info canvas renders it through
    /// `LocalizedFromInteger(IntegerFromNumber(param))`. A PLAIN
    /// (un-namespaced) canvas hop must relay like the pip chain's
    /// materialised hops.
    fn output_info_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_OutputInfo",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0021",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "text_total", "isActive": true,
                     "text": "", "labelProperties": {"label": ""}}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsLocalizedField",
                     "widget": "_PointsTo_:ptr:1", "field": "ParamInput0", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsLocalizedFromInteger",
                     "defaultNZeros": 0, "nZeros": null, "withSeparators": true,
                     "input": "_PointsTo_:ptr:11"},
                    {"_Pointer_": "ptr:11", "_Type_": "BuildingBlocks_BindingsIntegerFromNumber",
                     "input": "_PointsTo_:ptr:12"},
                    {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsNumberComponentParameter",
                     "name": "totalpossiblepower", "parameter": "ParamInput0", "defaultValue": 0.0}
                ]
            }
        })
    }

    fn output_power_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_OutputPower",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0022",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "power_root", "isActive": true},
                    {"_Pointer_": "ptr:16", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_outputinfo", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_outputinfo.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:16", "field": "ParamInput0", "input": "_PointsTo_:ptr:18"},
                    {"_Pointer_": "ptr:18", "_Type_": "BuildingBlocks_BindingsNumberComponentParameter",
                     "name": "totalpossiblepower", "parameter": "ParamInput2", "defaultValue": 0.0}
                ]
            }
        })
    }

    fn output_master_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_OutputMaster",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0023",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "master_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_power", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_outputpower.json"}
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:2", "field": "ParamInput2", "input": "_PointsTo_:ptr:10"},
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsNumberVariable",
                     "path": [], "binding": "totalpower", "inheritsNamespace": true}
                ]
            }
        })
    }

    fn output_frame_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test_OutputFrame",
            "_RecordId_": "00000000-0000-0000-0000-0000000r0024",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "frame_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "canvas_content", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "instantiated": true,
                     "canvas": "file://./test_outputmaster.json"}
                ],
                "operations": []
            }
        })
    }

    fn output_fetcher(path: &str) -> Result<serde_json::Value, String> {
        let p = path.to_ascii_lowercase();
        if p.contains("test_outputinfo") {
            Ok(output_info_canvas())
        } else if p.contains("test_outputpower") {
            Ok(output_power_canvas())
        } else if p.contains("test_outputmaster") {
            Ok(output_master_canvas())
        } else {
            Err(format!("no record for '{path}'"))
        }
    }

    #[test]
    fn number_param_relays_across_plain_canvas_hops_into_text() {
        let mut defaults = DefaultValueRegistry::default();
        defaults.insert_path("totalpower", crate::canvas::Value::Float(16.0));
        let scene = resolve_canvas_graph_with_defaults(
            &output_frame_canvas(),
            Some("drak"),
            &output_fetcher,
            None,
            None,
            &defaults,
        )
        .expect("resolve");

        let (text_id, _) = scene
            .nodes
            .iter()
            .find(|(_, n)| n.name == "text_total")
            .expect("text node must merge");
        let resolver = crate::bb_bindings::BindingResolver::from_operations(&scene.operations);
        assert_eq!(
            resolver.resolve_field_text(*text_id, "ParamInput0", &defaults).as_deref(),
            Some("16"),
            "the master's number variable must relay through the plain canvas hops"
        );
    }
}

// Widget-standard template expansion.
//
// The engine instantiates standard template canvases for component-backed
// widgets: a `WidgetIcon` hosts `iconwidgetstandard` (its `IconInstance`
// custom-shape renders the glyph, driven by the IconPreset/CustomIcon/ShowIcon
// component parameters) and a `ComponentGeneralButtonSecondary` hosts
// `buttongeneralsecondarycomponentstandard` (ComponentRoot chrome + icon/label
// sub-instances, FillStyle → fill-style state tag). Hosts carry implicit
// framework tags (`icon`, `general-button-secondary`) resolved from the tag
// database by name, which brand style entries target (e.g. the Drake MFD
// footer's "Button Icons" custom arrow, the sk_<brand> button chrome).
//
// `expand_widget_standards` runs after canvas merging and before state-tag /
// style application (phase 1: instance nodes + static params + host tags);
// `finalize_widget_standard_fields` runs after style application (phase 2:
// styled CustomIcon overrides, SvgPath / RenderShape / IsActive field
// resolution into the expanded nodes).

/// Framework record path of the standard icon widget template.
fn icon_widget_standard_path() -> &'static str {
    "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/widgets/iconwidgetstandard.json"
}

/// Framework record path of the standard secondary-button component template.
fn button_secondary_component_standard_path() -> &'static str {
    "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/components/buttongeneralsecondarycomponentstandard.json"
}

/// Framework record path of the PRIMARY general-button component standard
/// (the `Filled` call-to-action variant used by e.g. transit call consoles).
fn button_general_component_standard_path() -> &'static str {
    "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/components/buttongeneralcomponentstandard.json"
}

/// Framework record path of the tag database.
fn tag_database_path() -> &'static str {
    "file://./../../../../../../../libs/foundry/records/tagdatabase/tagdatabase.tagdatabase.json"
}

/// The implicit framework tag name the engine attaches to a widget host of the
/// given type (style entries target these, e.g. `Tag(icon)` for custom icon
/// overrides). `None` for types without a standard-template host tag.
fn implicit_host_tag_name(ty: &BbNodeType) -> Option<&'static str> {
    match ty {
        BbNodeType::WidgetIcon => Some("icon"),
        BbNodeType::ComponentGeneralButton => Some("general-button-primary"),
        BbNodeType::ComponentGeneralButtonSecondary => Some("general-button-secondary"),
        _ => None,
    }
}

/// Resolve a tag's `_RecordId_` from the tag database by its `tagName`.
fn tag_id_by_name(tag_database: &serde_json::Value, tag_name: &str) -> Option<String> {
    fn walk(value: &serde_json::Value, tag_name: &str) -> Option<String> {
        match value {
            serde_json::Value::Object(map) => {
                if map.get("tagName").and_then(|v| v.as_str()) == Some(tag_name) {
                    return map
                        .get("_RecordId_")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);
                }
                map.values().find_map(|child| walk(child, tag_name))
            }
            serde_json::Value::Array(items) => items.iter().find_map(|child| walk(child, tag_name)),
            _ => None,
        }
    }
    walk(tag_database, tag_name)
}

/// A static component-parameter value fed from the host's authored properties.
enum ParamValue {
    Int(i64),
    Bool(bool),
    /// A plain number for a `BindingsNumberComponentParameter` op (e.g. the
    /// button standards' "Icon AnchorToParentX" — the host's authored
    /// `iconProperties.anchorToParentX`).
    Num(f64),
    Str(String),
    /// A localization KEY (e.g. `@ui_interactor_call_elevator`) for a
    /// `BindingsLocalizedComponentParameter` op — resolved through the
    /// localization table at bind time, unlike `Str` which is a literal.
    Loc(String),
}

/// Inject synthetic resolved-parameter ops into a standard template's child
/// scene, matched by the template's authored `ComponentParameter` op `name`
/// (the component data contract). Each synthetic op shares the parameter op's
/// pointer and is annotated with `_HostNodeId_` so post-style finalization can
/// re-target styled overrides per host instance.
fn inject_standard_params(
    child_scene: &mut crate::bb_scene::BbScene,
    host_id: BbNodeId,
    params: &[(&str, ParamValue)],
) {
    let mut synthetics = Vec::new();
    for op in &mut child_scene.operations {
        let Some(op_name) = op.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some((_, value)) = params.iter().find(|(name, _)| *name == op_name) else {
            continue;
        };
        let Some(ptr) = op.get("_Pointer_").and_then(|v| v.as_str()).map(str::to_owned) else {
            continue;
        };
        if let Some(map) = op.as_object_mut() {
            map.insert("_HostNodeId_".to_string(), serde_json::json!(host_id));
        }
        let synth = match value {
            ParamValue::Int(value) => serde_json::json!({
                "_Type_": "_SynthIntegerParam_",
                "_Pointer_": ptr,
                "_HostNodeId_": host_id,
                "resolvedInt": value,
            }),
            ParamValue::Bool(value) => serde_json::json!({
                "_Type_": "_SynthBooleanParam_",
                "_Pointer_": ptr,
                "_HostNodeId_": host_id,
                "resolvedBool": value,
            }),
            ParamValue::Num(value) => serde_json::json!({
                "_Type_": "_SynthNumberParam_",
                "_Pointer_": ptr,
                "_HostNodeId_": host_id,
                "resolvedNumber": value,
            }),
            ParamValue::Str(value) => serde_json::json!({
                "_Type_": "_SynthStringParam_",
                "_Pointer_": ptr,
                "_HostNodeId_": host_id,
                "resolvedString": value,
            }),
            ParamValue::Loc(key) => serde_json::json!({
                "_Type_": "_SynthLocalizedParam_",
                "_Pointer_": ptr,
                "_HostNodeId_": host_id,
                "resolvedLocKey": key,
            }),
        };
        synthetics.push(synth);
    }
    child_scene.operations.extend(synthetics);
}

/// The host's effective custom-icon path: an authored `customIcon`, else the
/// `iconPreset` enum resolved to its standard vector asset.
fn host_icon_path(node: &crate::bb_scene::BbNode) -> Option<String> {
    let props = node.raw.get("iconProperties")?;
    let custom = props
        .get("customIcon")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(custom) = custom {
        return Some(custom.to_owned());
    }
    props
        .get("iconPreset")
        .and_then(|v| v.as_str())
        .and_then(crate::icon_preset::svg_path_for_preset)
}

fn icon_properties_show(node: &crate::bb_scene::BbNode) -> bool {
    node.raw
        .get("iconProperties")
        .and_then(|props| props.get("show"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn label_properties_show(node: &crate::bb_scene::BbNode) -> bool {
    node.raw
        .get("labelProperties")
        .and_then(|props| props.get("show"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// `BB_ButtonFillStyle` enum index (the template's `FillStyle` integer
/// parameter, switched onto `fill-style-filled` / `fill-style-ghost` tags by
/// the standard's own operations).
fn button_fill_style_index(node: &crate::bb_scene::BbNode) -> i64 {
    match node.raw.get("fillStyle").and_then(|v| v.as_str()) {
        Some(style) if style.eq_ignore_ascii_case("Ghost") => 1,
        _ => 0,
    }
}

/// Copy the host button's authored glyph identity (`iconPreset`/`customIcon`)
/// onto the merged template's `WidgetIcon` instances, mirroring the engine's
/// IconPreset/CustomIcon parameter routing. Visibility is not copied — the
/// instance's `IsActive` stays bound to the template's `ShowIcon` parameter.
/// Only the expansion-band subtree is walked; the host's authored children
/// (e.g. a sibling custom `WidgetIcon`) keep their own identity.
/// Add `tag_id` to every merged expansion instance of type `instance_ty` under
/// `host_id` (skipping nodes that already carry it). Used to attach the widget
/// standards' `*-element-instance` styling tags to a component standard's
/// instance widgets — the identity the modular-kit sheets' state entries
/// (Filled/Ghost foreground colours) select on.
fn tag_expanded_instances(
    scene: &mut BbScene,
    host_id: BbNodeId,
    instance_ty: &BbNodeType,
    tag_id: &str,
) {
    for_each_expanded_instance(scene, host_id, |node| {
        if node.ty != *instance_ty {
            return;
        }
        if !node
            .style_tag_uuids
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(tag_id))
        {
            node.style_tag_uuids.push(tag_id.to_string());
        }
    });
}

/// Visit every merged expansion-instance node under `host_id` (depth-first over
/// children in the expansion id band). The shared traversal behind the
/// post-merge forwarding/tagging passes — they differ only in the visit body.
fn for_each_expanded_instance(
    scene: &mut BbScene,
    host_id: BbNodeId,
    mut visit: impl FnMut(&mut crate::bb_scene::BbNode),
) {
    let Some(host) = scene.nodes.get(&host_id) else {
        return;
    };
    let mut stack: Vec<BbNodeId> = host
        .children
        .iter()
        .copied()
        .filter(|id| *id >= EXPANSION_ID_BASE)
        .collect();
    while let Some(id) = stack.pop() {
        let Some(node) = scene.nodes.get_mut(&id) else {
            continue;
        };
        stack.extend(node.children.iter().copied().filter(|c| *c >= EXPANSION_ID_BASE));
        visit(node);
    }
}

/// Copy the host button's authored `labelProperties.caseModifier` onto merged
/// text-field instances. The instance binds its CONTENT from the injected
/// `Label` parameter, but the case transform is read from the field's own raw
/// (`case_modifier_from_raw`) — the template authors `None`, so without this
/// the host's `Upper` (e.g. CALL ELEVATOR) renders mixed-case.
fn forward_host_label_case(scene: &mut BbScene, host_id: BbNodeId) {
    let Some(host) = scene.nodes.get(&host_id) else {
        return;
    };
    let Some(case) = host
        .raw
        .get("labelProperties")
        .and_then(|lp| lp.get("caseModifier"))
        .cloned()
    else {
        return;
    };
    for_each_expanded_instance(scene, host_id, |node| {
        if !matches!(node.ty, BbNodeType::WidgetTextField) {
            return;
        }
        if let Some(lp) = node
            .raw
            .as_object_mut()
            .map(|map| {
                map.entry("labelProperties")
                    .or_insert_with(|| serde_json::json!({}))
            })
            .and_then(|v| v.as_object_mut())
        {
            lp.insert("caseModifier".to_string(), case.clone());
        }
    });
}

fn forward_host_icon_identity(scene: &mut BbScene, host_id: BbNodeId) {
    let Some(host) = scene.nodes.get(&host_id) else {
        return;
    };
    let Some(props) = host.raw.get("iconProperties") else {
        return;
    };
    let preset = props.get("iconPreset").cloned();
    let custom = props.get("customIcon").cloned();
    if preset.is_none() && custom.is_none() {
        return;
    }
    for_each_expanded_instance(scene, host_id, |node| {
        if !matches!(node.ty, BbNodeType::WidgetIcon) {
            return;
        }
        let raw = node
            .raw
            .as_object_mut()
            .map(|map| {
                map.entry("iconProperties")
                    .or_insert_with(|| serde_json::json!({}))
            })
            .and_then(|v| v.as_object_mut());
        if let Some(obj) = raw {
            if let Some(preset) = preset.clone() {
                obj.insert("iconPreset".to_string(), preset);
            }
            if let Some(custom) = custom.clone() {
                obj.insert("customIcon".to_string(), custom);
            }
            // The parse-time `BbIcon` bakes the preset→SVG resolution; rebuild
            // it or the draw keeps the template's authored default glyph.
            node.icon = Some(crate::bb_scene::parse_icon(&node.raw, &node.ty));
        }
    });
}

/// True when `host` already has an expanded template instance (a child carrying
/// the `canvas-proxy-root` framework tag).
fn has_expanded_instance(
    scene: &crate::bb_scene::BbScene,
    host: &crate::bb_scene::BbNode,
    proxy_root_tag: &str,
) -> bool {
    host.children.iter().any(|child_id| {
        scene.nodes.get(child_id).is_some_and(|child| {
            child
                .style_tag_uuids
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case(proxy_root_tag))
        })
    })
}

/// Phase 1: expand standard templates under their host widgets. Returns the
/// expanded templates' own `embeddedStyles` entries that participate in the
/// scene's style application (currently the scrollbar's `RootShow` gate; the
/// icon/button standards keep their pre-existing entry-less behaviour).
pub(crate) fn expand_widget_standards(
    scene: &mut BbScene,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Vec<serde_json::Value> {
    let mut collected_embedded_entries: Vec<serde_json::Value> = Vec::new();
    let hosts: Vec<(BbNodeId, BbNodeType)> = scene
        .nodes
        .iter()
        .filter(|(_, node)| {
            (matches!(
                node.ty,
                BbNodeType::WidgetIcon
                    | BbNodeType::ComponentGeneralButton
                    | BbNodeType::ComponentGeneralButtonSecondary
            ) || is_scrollbar_component(&node.ty))
                && node.is_active
        })
        .map(|(id, node)| (*id, node.ty.clone()))
        .collect();
    if hosts.is_empty() {
        return collected_embedded_entries;
    }

    let Ok(tag_database) = fetch_by_path(tag_database_path()) else {
        log::debug!("widget-standard expansion: tag database unavailable; hosts keep authored tags only");
        return collected_embedded_entries;
    };
    let proxy_root_tag = tag_id_by_name(&tag_database, "canvas-proxy-root").unwrap_or_default();
    let mut template_cache: std::collections::HashMap<&'static str, serde_json::Value> =
        std::collections::HashMap::new();
    let mut tag_cache: std::collections::HashMap<&'static str, Option<String>> =
        std::collections::HashMap::new();

    for (host_id, host_ty) in hosts {
        let Some(host) = scene.nodes.get(&host_id) else {
            continue;
        };
        let template_path = match host_ty {
            BbNodeType::WidgetIcon => icon_widget_standard_path(),
            BbNodeType::ComponentGeneralButton => button_general_component_standard_path(),
            BbNodeType::ComponentGeneralButtonSecondary => {
                button_secondary_component_standard_path()
            }
            ref ty if is_scrollbar_component(ty) => scrollbar_component_standard_path(host),
            _ => continue,
        };
        if !proxy_root_tag.is_empty() && has_expanded_instance(scene, host, &proxy_root_tag) {
            continue;
        }
        // A hidden icon host draws nothing — skip the template entirely.
        if matches!(host_ty, BbNodeType::WidgetIcon) && !icon_properties_show(host) {
            continue;
        }

        let template_json = match template_cache.get(template_path) {
            Some(json) => json.clone(),
            None => match fetch_by_path(template_path) {
                Ok(json) => {
                    template_cache.insert(template_path, json.clone());
                    json
                }
                Err(e) => {
                    log::debug!("widget-standard expansion: template '{template_path}' unavailable: {e}");
                    continue;
                }
            },
        };
        let mut child_scene = match parse_bb_canvas(&template_json) {
            Ok(child_scene) => child_scene,
            Err(e) => {
                log::debug!("widget-standard expansion: template '{template_path}' parse failed: {e}");
                continue;
            }
        };

        let params: Vec<(&str, ParamValue)> = match host_ty {
            BbNodeType::WidgetIcon => {
                let mut params = vec![("ShowIcon", ParamValue::Bool(icon_properties_show(host)))];
                if let Some(path) = host_icon_path(host) {
                    params.push(("CustomIcon", ParamValue::Str(path)));
                }
                params
            }
            BbNodeType::ComponentGeneralButton
            | BbNodeType::ComponentGeneralButtonSecondary => {
                let mut params = vec![
                    ("FillStyle", ParamValue::Int(button_fill_style_index(host))),
                    ("ShowIcon", ParamValue::Bool(icon_properties_show(host))),
                    ("ShowLabel", ParamValue::Bool(label_properties_show(host))),
                ];
                // The instance's text field binds its content from the `Label`
                // component parameter (a localized key); without it the
                // template's authored default renders instead of the host's.
                if let Some(label) = host
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("label"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    params.push(("Label", ParamValue::Loc(label.to_string())));
                }
                // The templates derive the icon/label instance ANCHORS from the
                // host's authored anchorToParent through an op graph
                // (anchor = f(param) − 1 + position-switch); an unwired
                // parameter defaults 0.0 and the graph yields −1, throwing the
                // element OUT of the button (the medbed close ✕ rendered left
                // of its square). Feed the authored values, like the engine's
                // component-property → named-parameter routing.
                for (param, props, field) in [
                    ("Icon AnchorToParentX", "iconProperties", "anchorToParentX"),
                    ("Icon AnchorToParentY", "iconProperties", "anchorToParentY"),
                    ("Label AnchorToParentX", "labelProperties", "anchorToParentX"),
                    ("Label AnchorToParentY", "labelProperties", "anchorToParentY"),
                ] {
                    if let Some(value) = host
                        .raw
                        .get(props)
                        .and_then(|p| p.get(field))
                        .and_then(|v| v.as_f64())
                    {
                        params.push((param, ParamValue::Num(value)));
                    }
                }
                params
            }
            // `_Show` reflects the runtime "content overflows" check; the
            // at-rest assumption is corrected by the layout-time scroll
            // model, which hides the bar again when nothing overflows.
            ref ty if is_scrollbar_component(ty) => vec![("_Show", ParamValue::Bool(true))],
            _ => continue,
        };
        inject_standard_params(&mut child_scene, host_id, &params);
        if is_scrollbar_component(&host_ty) {
            annotate_scroll_thumb_and_view(scene, host_id, &mut child_scene);
            collected_embedded_entries.extend(template_embedded_style_entries(&template_json));
            mark_scrollbar_standard_nodes(&mut child_scene);
        }
        // A button standard's embedded styles carry its state-tag-driven
        // visuals (e.g. the `Filled` backplate fill selected by the FillStyle
        // tag switch); without them the merged custom-shape instances have no
        // fill and the button renders as bare text.
        if matches!(
            host_ty,
            BbNodeType::ComponentGeneralButton | BbNodeType::ComponentGeneralButtonSecondary
        ) {
            collected_embedded_entries.extend(template_embedded_style_entries(&template_json));
        }
        merge_child_scene(scene, child_scene, "", Some(host_id), true);

        // The engine routes a button's authored icon identity through the
        // template's IconPreset/CustomIcon parameters into its nested icon
        // instance; reproduce that flow onto the merged instance nodes.
        if matches!(
            host_ty,
            BbNodeType::ComponentGeneralButton | BbNodeType::ComponentGeneralButtonSecondary
        ) {
            forward_host_icon_identity(scene, host_id);
            forward_host_label_case(scene, host_id);
            // The engine nests each instance widget's OWN standard (icon /
            // text-field widget standards), whose inner element carries the
            // `*-element-instance` styling tag the modular-kit button sheets
            // target (e.g. the Filled state's dark `FillColor`). Expansion here
            // is single-level, so attach those element tags to the merged
            // instances directly — resolved from the tag database by NAME.
            // Only SHOWN elements get their styling identity: the kit's base
            // entries (`RootTextFieldElementInstance` → IsActive=true) would
            // otherwise reveal an element the host authored hidden.
            let host_shows = scene
                .nodes
                .get(&host_id)
                .map(|host| (icon_properties_show(host), label_properties_show(host)))
                .unwrap_or((false, false));
            for (instance_ty, tag_name, shown) in [
                (BbNodeType::WidgetIcon, "icon-element-instance", host_shows.0),
                (BbNodeType::WidgetTextField, "text-element-instance", host_shows.1),
            ] {
                if !shown {
                    continue;
                }
                let tag_id = tag_cache
                    .entry(tag_name)
                    .or_insert_with(|| tag_id_by_name(&tag_database, tag_name))
                    .clone();
                let Some(tag_id) = tag_id else { continue };
                tag_expanded_instances(scene, host_id, &instance_ty, &tag_id);
            }
        }

        if let Some(host) = scene.nodes.get_mut(&host_id) {
            // The instance renders the glyph; the host's parse-time preset
            // fallback would draw a duplicate.
            if matches!(
                host_ty,
                BbNodeType::WidgetIcon
                    | BbNodeType::ComponentGeneralButton
                    | BbNodeType::ComponentGeneralButtonSecondary
            ) && let Some(icon) = host.icon.as_mut()
            {
                icon.image_record = None;
            }
            // Likewise the instance's text field renders the label (bound via
            // the injected `Label` parameter); blank the host's own authored
            // label key or the text draws twice at slightly different rects.
            if matches!(
                host_ty,
                BbNodeType::ComponentGeneralButton | BbNodeType::ComponentGeneralButtonSecondary
            ) && let Some(lp) = host
                .raw
                .get_mut("labelProperties")
                .and_then(|v| v.as_object_mut())
            {
                lp.insert("label".to_string(), serde_json::json!(""));
            }
            if let Some(tag_name) = implicit_host_tag_name(&host_ty) {
                let tag_id = tag_cache
                    .entry(tag_name)
                    .or_insert_with(|| tag_id_by_name(&tag_database, tag_name))
                    .clone();
                if let Some(tag_id) = tag_id
                    && !host
                        .style_tag_uuids
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case(&tag_id))
                {
                    host.style_tag_uuids.push(tag_id);
                }
            }
        }
    }
    collected_embedded_entries
}

/// Phase 2 (post style application): re-target styled custom-icon overrides
/// and resolve the templates' field bindings onto the expanded nodes.
///
/// Runs once at the depth that performed the expansion; the `_HostNodeId_`
/// annotations are stripped afterwards because node ids are remapped when this
/// scene is merged into a parent (a stale annotation would walk bogus nodes).
pub(crate) fn finalize_widget_standard_fields(
    scene: &mut BbScene,
    defaults: &crate::defaults::DefaultValueRegistry,
) {
    // A style entry that supplies a custom icon writes the host's string
    // `ParamInput0` (the icon contract's CustomIcon parameter — e.g. the Drake
    // footer's "Button Icons" pixel arrow). Override the injected synthetic.
    let mut overrides: Vec<(BbNodeId, String)> = Vec::new();
    for op in &scene.operations {
        if op.get("_Type_").and_then(|v| v.as_str()) != Some("_SynthStringParam_") {
            continue;
        }
        let Some(host_id) = op.get("_HostNodeId_").and_then(|v| v.as_u64()) else {
            continue;
        };
        let host_id = host_id as BbNodeId;
        let Some(styled) = scene.nodes.get(&host_id).and_then(|host| {
            host.raw
                .get("ParamInput0")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        }) else {
            continue;
        };
        overrides.push((host_id, styled));
    }
    for op in &mut scene.operations {
        if op.get("_Type_").and_then(|v| v.as_str()) != Some("_SynthStringParam_") {
            continue;
        }
        let Some(host_id) = op.get("_HostNodeId_").and_then(|v| v.as_u64()) else {
            continue;
        };
        if let Some((_, styled)) = overrides
            .iter()
            .find(|(candidate, _)| *candidate == host_id as BbNodeId)
            && let Some(map) = op.as_object_mut()
        {
            map.insert("resolvedString".to_string(), serde_json::json!(styled));
        }
    }

    // Resolve the templates' bound fields into the expanded nodes only (the
    // hosts annotated during phase 1) — field bindings elsewhere keep their
    // existing handling. Only fields that resolve to a concrete value are
    // applied (unresolvable runtime bindings keep the authored state).
    let mut expanded: std::collections::HashSet<BbNodeId> = std::collections::HashSet::new();
    for op in &scene.operations {
        if let Some(host_id) = op.get("_HostNodeId_").and_then(|v| v.as_u64()) {
            expanded.insert(host_id as BbNodeId);
        }
    }
    if expanded.is_empty() {
        return;
    }
    let mut frontier: Vec<BbNodeId> = expanded.iter().copied().collect();
    while let Some(id) = frontier.pop() {
        let Some(node) = scene.nodes.get(&id) else { continue };
        for child in &node.children {
            if expanded.insert(*child) {
                frontier.push(*child);
            }
        }
    }

    let resolver = crate::bb_bindings::BindingResolver::from_operations(&scene.operations);
    let node_ids: Vec<BbNodeId> = expanded.into_iter().collect();
    for node_id in node_ids {
        if let Some(svg_path) = resolver.resolve_field_text(node_id, "SvgPath", defaults) {
            let trimmed = svg_path.trim();
            if !trimmed.is_empty()
                && let Some(node) = scene.nodes.get_mut(&node_id)
                && let Some(map) = node.raw.as_object_mut()
            {
                map.insert("SvgPath".to_string(), serde_json::json!(trimmed));
            }
        }
        if let Some(render_shape) = resolver.resolve_field_bool(node_id, "RenderShape", defaults)
            && let Some(node) = scene.nodes.get_mut(&node_id)
        {
            if let Some(svg_fill) = node
                .raw
                .get_mut("svgFill")
                .and_then(|fill| fill.as_object_mut())
            {
                svg_fill.insert("renderShape".to_string(), serde_json::json!(render_shape));
            }
            if !render_shape {
                node.is_active = false;
            }
        }
        if let Some(is_active) = resolver.resolve_field_bool(node_id, "IsActive", defaults)
            && let Some(node) = scene.nodes.get_mut(&node_id)
        {
            node.is_active = is_active;
        }
    }

    for op in &mut scene.operations {
        if let Some(map) = op.as_object_mut() {
            map.remove("_HostNodeId_");
        }
    }
}

// ComponentScrollBar standard-template expansion helpers.
//
// A `BuildingBlocks_ComponentScrollBar` carries no canvas of its own: the
// engine instantiates the directional scrollbar standard
// (`scrollbarhorizontal|verticalcomponentstandard`) under it and feeds the
// component's runtime scroll model into the template parameters: `_Show`
// (content overflows → SecondaryStateTag `scrollbar-show` → the template's
// embedded `RootShow` style activates `ComponentRoot`), `_SizeRatio`
// (thumb `SizeX|SizeY` = viewport/content fraction) and `_AnchorRatio`
// (thumb anchor = scroll offset). The static render reproduces the model in
// two stages: expansion pairs the template's thumb widget with the
// component's `target` scroll-view node via `_ScrollThumbPair_` /
// `_ScrollViewPair_` raw markers (strings — they survive node-id remapping
// on merge), and `bb_layout::apply_scroll_thumb_rects` computes the at-rest
// ratio from the laid-out view/content geometry.

/// Framework record path of the directional scrollbar component standard.
fn scrollbar_component_standard_path(host: &crate::bb_scene::BbNode) -> &'static str {
    let vertical = host
        .raw
        .get("direction")
        .and_then(|v| v.as_str())
        .is_some_and(|direction| direction.eq_ignore_ascii_case("Vertical"));
    if vertical {
        "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/components/scrollbarverticalcomponentstandard.json"
    } else {
        "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/components/scrollbarhorizontalcomponentstandard.json"
    }
}

fn is_scrollbar_component(ty: &BbNodeType) -> bool {
    matches!(
        ty,
        BbNodeType::Other(kind) if kind.eq_ignore_ascii_case("BuildingBlocks_ComponentScrollBar")
    )
}

/// Pair the template's thumb widget (the node whose `SizeX`/`SizeY` binds to
/// the `_SizeRatio` component parameter) with the host's `target` scroll-view
/// node. Markers are plain strings on `raw` so they survive the node-id
/// remapping every later merge applies.
fn annotate_scroll_thumb_and_view(
    scene: &mut BbScene,
    host_id: BbNodeId,
    child_scene: &mut crate::bb_scene::BbScene,
) {
    static SCROLL_PAIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let Some(size_ratio_ptr) = child_scene.operations.iter().find_map(|op| {
        let is_param = op.get("_Type_").and_then(|v| v.as_str())
            == Some("BuildingBlocks_BindingsNumberComponentParameter");
        let is_size_ratio = op.get("name").and_then(|v| v.as_str()) == Some("_SizeRatio");
        (is_param && is_size_ratio)
            .then(|| op.get("_Pointer_").and_then(|v| v.as_str()).map(str::to_owned))
            .flatten()
    }) else {
        return;
    };
    let input_ref = format!("_PointsTo_:{size_ratio_ptr}");
    let Some((thumb_id, axis)) = child_scene.operations.iter().find_map(|op| {
        if op.get("_Type_").and_then(|v| v.as_str())
            != Some("BuildingBlocks_BindingsNumberField")
            || op.get("input").and_then(|v| v.as_str()) != Some(input_ref.as_str())
        {
            return None;
        }
        let axis = match op.get("field").and_then(|v| v.as_str()) {
            Some("SizeX") => "x",
            Some("SizeY") => "y",
            _ => return None,
        };
        let widget_id = op
            .get("widget")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("_PointsTo_:ptr:"))
            .and_then(|n| n.parse::<BbNodeId>().ok())?;
        Some((widget_id, axis))
    }) else {
        return;
    };
    let Some(target_id) = scene.nodes.get(&host_id).and_then(|host| {
        host.raw
            .get("target")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("_PointsTo_:ptr:"))
            .and_then(|n| n.parse::<BbNodeId>().ok())
    }) else {
        return;
    };
    if !scene.nodes.contains_key(&target_id) {
        return;
    }

    let pair_key = format!(
        "scroll-pair-{}",
        SCROLL_PAIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    if let Some(thumb) = child_scene.nodes.get_mut(&thumb_id)
        && let Some(map) = thumb.raw.as_object_mut()
    {
        map.insert("_ScrollThumbPair_".to_string(), serde_json::json!(pair_key));
        map.insert("_ScrollThumbAxis_".to_string(), serde_json::json!(axis));
    }
    if let Some(view) = scene.nodes.get_mut(&target_id)
        && let Some(map) = view.raw.as_object_mut()
    {
        map.insert("_ScrollViewPair_".to_string(), serde_json::json!(pair_key));
    }
}

/// The template's own `embeddedStyles` entries (the scrollbar's `RootShow`
/// visibility gate). Applied by the caller after state-tag resolution so
/// entry conditions can match the `_Show` → `scrollbar-show` tag chain.
fn template_embedded_style_entries(template_json: &serde_json::Value) -> Vec<serde_json::Value> {
    template_json
        .get("_RecordValue_")
        .unwrap_or(template_json)
        .get("embeddedStyles")
        .and_then(|v| v.as_array())
        .map(|entries| entries.to_vec())
        .unwrap_or_default()
}

/// Raw marker carried by every node of an expanded scrollbar standard: the
/// brand's `sk_<brand>_scrollbarstyles` module sheet is applied scoped to
/// these nodes (see `apply_scrollbar_modular_styles`).
const SCROLLBAR_STANDARD_NODE_MARKER: &str = "_ScrollBarStandardNode_";

fn mark_scrollbar_standard_nodes(child_scene: &mut crate::bb_scene::BbScene) {
    for node in child_scene.nodes.values_mut() {
        if let Some(map) = node.raw.as_object_mut() {
            map.insert(
                SCROLLBAR_STANDARD_NODE_MARKER.to_string(),
                serde_json::Value::Bool(true),
            );
        }
    }
}

/// Apply the brand's scrollbar module sheet to expanded scrollbar standards.
/// The sheet belongs to the STANDARD's own style cascade: the engine applies
/// it to the instantiated template canvas, never the host scene — its `Root`
/// entry targets the generic `canvas-proxy-root` tag that every expanded
/// standard root (icon instances, button chrome) also carries, so the
/// application is scoped to nodes carrying the scrollbar standard marker.
/// Runs in the modular-sheets phase because the brand identifier resolves at
/// ancestor canvas depths (the power-lists canvas carries no `s_drak_hud`
/// brand entry of its own), after the standards have merged in.
pub(crate) fn apply_scrollbar_modular_styles(
    scene: &mut BbScene,
    style_id: &str,
    chrome_palette: Option<&serde_json::Value>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) {
    if !scene
        .nodes
        .values()
        .any(|node| node.raw.get(SCROLLBAR_STANDARD_NODE_MARKER).is_some())
    {
        return;
    }
    let Some(sheet_path) = modular_scrollbar_style_path(style_id) else { return };
    let sheet_json = match fetch_by_path(&sheet_path) {
        Ok(json) => json,
        Err(e) => {
            log::debug!("scrollbar styles: no module sheet '{sheet_path}': {e}");
            return;
        }
    };
    let sheet_value = sheet_json.get("_RecordValue_").unwrap_or(&sheet_json).clone();
    let Some(entries) = sheet_value.get("entries").and_then(|v| v.as_array()) else {
        return;
    };
    // Named colour roles (`Base`) resolve against the brand Style record's
    // palette — the module sheet itself carries no `colorStyles`.
    crate::bb_style_engine::apply(
        scene,
        &[crate::bb_style_engine::StyleSheet {
            tier: crate::bb_style_engine::Tier::StandardModule,
            identifier: extract_record_name(&sheet_path),
            fills: &sheet_value,
            chrome: chrome_palette.unwrap_or(&sheet_value),
            entries: entries.as_slice(),
            scope: crate::bb_style_engine::SheetScope::Marker(
                SCROLLBAR_STANDARD_NODE_MARKER.to_string(),
            ),
        }],
        None,
    );
}

// Tests for widget-standard template expansion (see
// widget_standard_expansion.part): host icon forwarding into the button
// template's icon instance and host parse-time fallback clearing.
#[cfg(test)]
mod tests_expansion {
    #![allow(unused_imports, dead_code)]

    use super::*;

    fn tag_database_fixture() -> serde_json::Value {
        serde_json::json!({
            "tags": [
                {"tagName": "canvas-proxy-root", "_RecordId_": "21788313-6aff-45b4-a3ad-fdc62b1cf849"},
                {"tagName": "general-button-secondary", "_RecordId_": "cf236683-1f17-41ee-a0d2-0e23c5f1c552"},
                {"tagName": "icon", "_RecordId_": "8b9ef179-0000-0000-0000-000000000000"}
            ]
        })
    }

    /// Minimal stand-in for `buttongeneralsecondarycomponentstandard`:
    /// ComponentRoot (proxy-root tagged) hosting a `WidgetIcon` instance whose
    /// authored preset is the template default (`ArrowHollowRight`).
    fn button_template_fixture() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.ButtonGeneralSecondaryComponentStandard",
            "_RecordId_": "00000000-0000-0000-0000-00000000b001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 64.0, "y": 64.0, "z": 0.0},
                "scene": [
                    {
                        "_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "ComponentRoot",
                        "styleTags": [{"_RecordId_": "21788313-6aff-45b4-a3ad-fdc62b1cf849"}]
                    },
                    {
                        "_Pointer_": "ptr:8", "_Type_": "BuildingBlocks_WidgetIcon",
                        "name": "IconWidgetInstance", "parent": "_PointsTo_:ptr:1",
                        "iconProperties": {
                            "_Type_": "BuildingBlocks_ComponentIconProperties",
                            "show": true, "iconPreset": "ArrowHollowRight", "customIcon": ""
                        }
                    }
                ],
                "operations": [
                    {
                        "_Pointer_": "ptr:30",
                        "_Type_": "BuildingBlocks_BindingsNumberComponentParameter",
                        "name": "Icon AnchorToParentX", "parameter": "ParamInput0",
                        "defaultValue": 0.0
                    },
                    {
                        "_Pointer_": "ptr:31",
                        "_Type_": "BuildingBlocks_BindingsNumberComponentParameter",
                        "name": "Icon AnchorToParentY", "parameter": "ParamInput1",
                        "defaultValue": 0.0
                    }
                ]
            }
        })
    }

    /// Host canvas: a `ComponentGeneralButtonSecondary` authored with the
    /// medical close button's icon contract (show=true, preset `GeneralX`).
    fn button_host_scene() -> BbScene {
        let canvas = serde_json::json!({
            "_RecordName_": "test_button_host",
            "_RecordId_": "00000000-0000-0000-0000-00000000c001",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_ComponentGeneralButtonSecondary",
                        "name": "ExitBed", "parent": "_PointsTo_:ptr:1",
                        "fillStyle": "Ghost",
                        "iconProperties": {
                            "_Type_": "BuildingBlocks_ComponentIconProperties",
                            "show": true, "iconPreset": "GeneralX", "customIcon": "",
                            "anchorToParentX": 0.5, "anchorToParentY": 0.5
                        },
                        "labelProperties": {
                            "_Type_": "BuildingBlocks_ComponentLabelProperties",
                            "show": false, "label": "@LOC_PLACEHOLDER"
                        }
                    }
                ],
                "operations": []
            }
        });
        parse_bb_canvas(&canvas).expect("host canvas should parse")
    }

    fn expansion_fetcher(path: &str) -> Result<serde_json::Value, String> {
        if path == tag_database_path() {
            Ok(tag_database_fixture())
        } else if path == button_secondary_component_standard_path() {
            Ok(button_template_fixture())
        } else {
            Err(format!("test fetcher: no record for '{path}'"))
        }
    }

    /// The engine routes a button's authored `iconProperties` through the
    /// template's IconPreset/CustomIcon parameters into its icon instance:
    /// the merged `IconWidgetInstance` must carry the HOST's glyph identity,
    /// not the template's authored default.
    #[test]
    fn button_expansion_forwards_host_icon_identity_to_icon_instance() {
        let mut scene = button_host_scene();
        expand_widget_standards(&mut scene, &expansion_fetcher);

        let icon_instance = scene
            .nodes
            .values()
            .find(|n| matches!(n.ty, BbNodeType::WidgetIcon) && n.id >= EXPANSION_ID_BASE)
            .expect("expansion should merge the template's WidgetIcon instance");
        assert_eq!(
            icon_instance
                .raw
                .get("iconProperties")
                .and_then(|p| p.get("iconPreset"))
                .and_then(|v| v.as_str()),
            Some("GeneralX"),
            "icon instance must inherit the host button's authored iconPreset"
        );
        assert_eq!(
            host_icon_path(icon_instance),
            crate::icon_preset::svg_path_for_preset("GeneralX"),
            "forwarded preset must resolve to the host glyph's vector asset"
        );
        // Assert the layer the RENDERER reads: forwarding only `raw` while the
        // parse-time `BbIcon` kept the template default passed the raw-level
        // assertion but drew the wrong glyph (ledger 103).
        let baked = icon_instance
            .icon
            .as_ref()
            .expect("merged icon instance must carry a parse-time BbIcon");
        assert_eq!(
            baked.image_record,
            crate::icon_preset::svg_path_for_preset("GeneralX"),
            "the baked BbIcon must resolve the forwarded preset, not the template default"
        );
    }

    /// The expanded instance renders the glyph; the host's parse-time icon
    /// fallback must be cleared or the glyph draws twice.
    #[test]
    fn button_expansion_clears_host_icon_fallback() {
        let mut scene = button_host_scene();
        expand_widget_standards(&mut scene, &expansion_fetcher);

        let host = scene
            .nodes
            .values()
            .find(|n| matches!(n.ty, BbNodeType::ComponentGeneralButtonSecondary))
            .expect("host should survive expansion");
        assert!(
            host.icon.as_ref().is_none_or(|icon| icon.image_record.is_none()),
            "button host parse-time icon fallback must be cleared after expansion"
        );
    }

    /// The templates derive the icon instance's ANCHOR from the host's
    /// authored `iconProperties.anchorToParentX/Y` through the "Icon
    /// AnchorToParent*" component parameters; an unwired parameter defaults
    /// 0.0 and the template op graph (f(param) − 1 + switch) yields −1,
    /// throwing the icon out of the button (the medbed close ✕ rendered left
    /// of its square). The expansion must inject the authored numbers.
    #[test]
    fn button_expansion_injects_host_anchor_to_parent_params() {
        let mut scene = button_host_scene();
        expand_widget_standards(&mut scene, &expansion_fetcher);

        let synth: Vec<_> = scene
            .operations
            .iter()
            .filter(|op| op.get("_Type_").and_then(|v| v.as_str()) == Some("_SynthNumberParam_"))
            .collect();
        assert_eq!(synth.len(), 2, "both icon anchor params injected: {synth:?}");
        for op in synth {
            assert_eq!(
                op.get("resolvedNumber").and_then(|v| v.as_f64()),
                Some(0.5),
                "the host's authored anchorToParent 0.5 must reach the parameter"
            );
        }
    }

    /// Host canvas: a PRIMARY `ComponentGeneralButton` authored with the
    /// transit call-console's contract (fillStyle `Filled`, double-caret icon
    /// above an `@ui_interactor_call_elevator` label).
    fn primary_button_host_scene() -> BbScene {
        let canvas = serde_json::json!({
            "_RecordName_": "test_primary_button_host",
            "_RecordId_": "00000000-0000-0000-0000-00000000c002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 512.0, "y": 740.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_ComponentGeneralButton",
                        "name": "Button_Call", "parent": "_PointsTo_:ptr:1",
                        "fillStyle": "Filled",
                        "iconPosition": "Top",
                        "iconProperties": {
                            "_Type_": "BuildingBlocks_ComponentIconProperties",
                            "show": true, "iconPreset": "ArrowCaratDoubleUp", "customIcon": ""
                        },
                        "labelProperties": {
                            "_Type_": "BuildingBlocks_ComponentLabelProperties",
                            "show": true, "label": "@ui_interactor_call_elevator"
                        }
                    }
                ],
                "operations": []
            }
        });
        parse_bb_canvas(&canvas).expect("host canvas should parse")
    }

    fn primary_expansion_fetcher(path: &str) -> Result<serde_json::Value, String> {
        if path == tag_database_path() {
            Ok(tag_database_fixture())
        } else if path == button_general_component_standard_path() {
            Ok(button_template_fixture())
        } else {
            Err(format!("test fetcher: no record for '{path}'"))
        }
    }

    /// A PRIMARY `ComponentGeneralButton` (e.g. a transit console's CALL
    /// ELEVATOR button) expands through its component standard exactly like
    /// the secondary variant: the merged icon instance carries the HOST's
    /// authored glyph identity.
    #[test]
    fn primary_button_expansion_forwards_host_icon_identity() {
        let mut scene = primary_button_host_scene();
        expand_widget_standards(&mut scene, &primary_expansion_fetcher);

        let icon_instance = scene
            .nodes
            .values()
            .find(|n| matches!(n.ty, BbNodeType::WidgetIcon) && n.id >= EXPANSION_ID_BASE)
            .expect("expansion should merge the primary standard's WidgetIcon instance");
        assert_eq!(
            icon_instance
                .raw
                .get("iconProperties")
                .and_then(|p| p.get("iconPreset"))
                .and_then(|v| v.as_str()),
            Some("ArrowCaratDoubleUp"),
            "icon instance must inherit the primary host's authored iconPreset"
        );
        // Assert the layer the RENDERER reads (ledger 103): the parse-time
        // `BbIcon` bakes the preset→SVG resolution, so a raw-only forward
        // passes the assertion above while still drawing the template glyph.
        let baked = icon_instance
            .icon
            .as_ref()
            .expect("merged icon instance must carry a parse-time BbIcon");
        assert_eq!(
            baked.image_record,
            crate::icon_preset::svg_path_for_preset("ArrowCaratDoubleUp"),
            "the baked BbIcon must resolve the forwarded preset, not the template default"
        );
    }

    /// Minimal stand-in for `scrollbarhorizontalcomponentstandard`: an
    /// inactive proxy-tagged `ComponentRoot` hosting the thumb, with the
    /// authored `_SizeRatio` → `SizeX` and `_Show` → `scrollbar-show` →
    /// `SecondaryStateTag` op chains and the `RootShow` embedded style.
    fn scrollbar_template_fixture() -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.ScrollBarHorizontalComponentStandard",
            "_RecordId_": "00000000-0000-0000-0000-00000000sb01",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "embeddedStyles": [
                    {
                        "_Type_": "BuildingBlocks_StyleEntry",
                        "name": "RootShow",
                        "conditionsList": [
                            {"conditions": [
                                {"_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                 "tag": {"_RecordId_": "e02a2290-9d6c-46f9-8235-4e828d804cee"}}
                            ]}
                        ],
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierBoolean",
                             "field": "IsActive", "value": true}
                        ]
                    }
                ],
                "scene": [
                    {
                        "_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "ComponentRoot", "isActive": false,
                        "styleTags": [{"_RecordId_": "21788313-6aff-45b4-a3ad-fdc62b1cf849"}]
                    },
                    {
                        "_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "BarElementInstance", "parent": "_PointsTo_:ptr:1",
                        "styleTags": [{"_RecordId_": "9ca447ec-f238-4751-9f22-440db0c25cb2"}]
                    }
                ],
                "operations": [
                    {"_Type_": "BuildingBlocks_BindingsNumberField",
                     "widget": "_PointsTo_:ptr:2", "field": "SizeX", "input": "_PointsTo_:ptr:3"},
                    {"_Pointer_": "ptr:3",
                     "_Type_": "BuildingBlocks_BindingsNumberComponentParameter",
                     "name": "_SizeRatio", "parameter": "ParamInput1", "defaultValue": 0.0},
                    {"_Pointer_": "ptr:5",
                     "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                     "name": "_Show", "parameter": "ParamInput1", "defaultValue": false},
                    {"_Pointer_": "ptr:6", "_Type_": "BuildingBlocks_BindingsTagFromBoolean",
                     "isTrue": {"_RecordId_": "e02a2290-9d6c-46f9-8235-4e828d804cee"},
                     "isFalse": null, "input": "_PointsTo_:ptr:5"},
                    {"_Type_": "BuildingBlocks_BindingsStringField",
                     "widget": "_PointsTo_:ptr:1", "field": "SecondaryStateTag",
                     "input": "_PointsTo_:ptr:6"}
                ]
            }
        })
    }

    /// Host canvas mirroring the power screen's wiring: a clipping
    /// `Scrollview` and a `ComponentScrollBar` targeting it.
    fn scrollbar_host_scene() -> BbScene {
        let canvas = serde_json::json!({
            "_RecordName_": "test_scrollbar_host",
            "_RecordId_": "00000000-0000-0000-0000-00000000c002",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "Scrollview", "parent": "_PointsTo_:ptr:1",
                     "overflow": {"_Type_": "BuildingBlocks_Overflow", "overflow": "Clip"}},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_ComponentScrollBar",
                     "name": "ListScroll", "parent": "_PointsTo_:ptr:1",
                     "direction": "Horizontal", "target": "_PointsTo_:ptr:2"}
                ],
                "operations": []
            }
        });
        parse_bb_canvas(&canvas).expect("host canvas should parse")
    }

    fn scrollbar_fetcher(path: &str) -> Result<serde_json::Value, String> {
        if path == tag_database_path() {
            Ok(tag_database_fixture())
        } else if path.contains("scrollbarhorizontalcomponentstandard") {
            Ok(scrollbar_template_fixture())
        } else {
            Err(format!("test fetcher: no record for '{path}'"))
        }
    }

    /// Expansion must pair the template's thumb with the host's `target`
    /// scroll view and surface the template's embedded `RootShow` entry.
    #[test]
    fn scrollbar_expansion_pairs_thumb_with_target_view() {
        let mut scene = scrollbar_host_scene();
        let entries = expand_widget_standards(&mut scene, &scrollbar_fetcher);

        let thumb = scene
            .nodes
            .values()
            .find(|n| n.raw.get("_ScrollThumbPair_").is_some())
            .expect("expansion should annotate the template thumb");
        assert_eq!(
            thumb.raw.get("_ScrollThumbAxis_").and_then(|v| v.as_str()),
            Some("x"),
            "a SizeX-bound thumb scrolls along x"
        );
        let pair = thumb.raw.get("_ScrollThumbPair_").cloned();
        let view = scene
            .nodes
            .values()
            .find(|n| n.name == "Scrollview")
            .expect("scroll view should survive expansion");
        assert_eq!(
            view.raw.get("_ScrollViewPair_").cloned(),
            pair,
            "the host's target view must carry the thumb's pair key"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.get("name").and_then(|v| v.as_str()) == Some("RootShow")),
            "the scrollbar template's embedded RootShow entry must be surfaced"
        );
    }

    /// The full at-rest visibility chain: `_Show=true` synth param →
    /// `scrollbar-show` SecondaryStateTag → embedded `RootShow` style →
    /// `ComponentRoot.IsActive`.
    #[test]
    fn scrollbar_show_chain_activates_component_root() {
        let mut scene = scrollbar_host_scene();
        let entries = expand_widget_standards(&mut scene, &scrollbar_fetcher);
        crate::bb_bindings::resolve_state_tags_into_scene(&mut scene, &Default::default());
        let empty_raw = serde_json::json!({});
        let brand = bb_brand_style::BrandStyle {
            identifier: "widget-standard-embedded".to_string(),
            entries: entries.as_slice(),
            raw: &empty_raw,
        };
        crate::bb_brand_apply::apply_brand_modifiers(&mut scene, &brand, None);

        let root = scene
            .nodes
            .values()
            .find(|n| n.name == "ComponentRoot")
            .expect("expansion should merge the template root");
        assert!(
            root.is_active,
            "_Show=true must activate the scrollbar root through RootShow"
        );
    }
}
