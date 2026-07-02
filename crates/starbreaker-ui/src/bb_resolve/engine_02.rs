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

// Consolidated engine chunk 02 (formerly: part_06.part, param_targets.part, part_07.part, part_08.part, part_09.part, part_10.part).
//   part_06.part: Synthesize `_SynthLocalizedParam_` operations from a parent `WidgetCanvas`
//   param_targets.part: Parent→child component-parameter injection target selection (split from

/// Synthesize `_SynthLocalizedParam_` operations from a parent `WidgetCanvas`
/// node's `paramInputValues` array into the child scene's operations.
///
/// When a `WidgetCanvas` node declares `paramInputValues` entries of type
/// `BuildingBlocks_ComponentParameterInputLocalization`, those entries override
/// the `defaultValue` of the matching `BuildingBlocks_BindingsLocalizedComponentParameter`
/// operations inside the child canvas.  This function injects a synthetic
/// `_SynthLocalizedParam_` operation for each such override so that
/// `BindingResolver` can map widget pointers to localization keys without
/// requiring ActionScript execution.
///
/// Synthetic ops have the same `_Pointer_` value as the matching
/// `BuildingBlocks_BindingsLocalizedComponentParameter` op; they are remapped
/// by `merge_child_scene` together with the rest of the child's operations.
pub(crate) fn inject_param_overrides(
    param_inputs: &[serde_json::Value],
    child_scene: &mut crate::bb_scene::BbScene,
) {
    if param_inputs.is_empty() {
        return;
    }

    // Build param_name → override maps from parent paramInputValues.
    let mut param_to_loc: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut param_to_string: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut param_to_bool: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut param_to_int: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for entry in param_inputs {
        let ty = entry
            .get("_Type_")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let Some(param) = entry.get("parameter").and_then(|v| v.as_str()) else {
            continue;
        };
        if param.is_empty() {
            continue;
        }
        let key = param.to_ascii_lowercase();
        if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputLocalization") {
            let Some(value) = entry.get("value").and_then(|v| v.as_str()) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            param_to_loc.insert(key, value.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputString") {
            let Some(value) = entry.get("value").and_then(|v| v.as_str()) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            param_to_string.insert(key, value.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputBoolean") {
            if let Some(v) = entry.get("value").and_then(|v| v.as_bool()) {
                param_to_bool.insert(param.to_ascii_lowercase(), v);
            }
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputInteger") {
            if let Some(v) = entry.get("value").and_then(|v| v.as_i64()) {
                param_to_int.insert(param.to_ascii_lowercase(), v);
            }
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_ComponentParameterInputNumber") {
            if let Some(v) = entry
                .get("value")
                .and_then(|v| v.as_f64())
                .map(|v| v.round() as i64)
            {
                param_to_int.insert(param.to_ascii_lowercase(), v);
            }
        }
    }
    if param_to_loc.is_empty()
        && param_to_string.is_empty()
        && param_to_bool.is_empty()
        && param_to_int.is_empty()
    {
        return;
    }

    // Scan existing ops for BuildingBlocks_BindingsLocalizedComponentParameter
    // entries and inject a synthetic _SynthLocalizedParam_ op for each match.
    let mut synthetics: Vec<serde_json::Value> = Vec::new();
    let mut param_ptr_to_loc: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut param_ptr_to_string: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for op in &child_scene.operations {
        let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
        let param_name_lc = op
            .get("parameter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let Some(ptr_str) = op.get("_Pointer_").and_then(|v| v.as_str()) else {
            continue;
        };
        if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedComponentParameter") {
            let Some(loc_key) = param_to_loc.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthLocalizedParam_",
                "_Pointer_": ptr_str,
                "resolvedLocKey": loc_key,
            }));
            param_ptr_to_loc.insert(ptr_str.to_owned(), loc_key.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringComponentParameter") {
            let Some(value) = param_to_string.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthStringParam_",
                "_Pointer_": ptr_str,
                "resolvedString": value,
            }));
            param_ptr_to_string.insert(ptr_str.to_owned(), value.to_owned());
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsBooleanComponentParameter") {
            let Some(value) = param_to_bool.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthBooleanParam_",
                "_Pointer_": ptr_str,
                "resolvedBool": value,
            }));
        } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsIntegerComponentParameter") {
            let Some(value) = param_to_int.get(&param_name_lc) else {
                continue;
            };
            synthetics.push(serde_json::json!({
                "_Type_": "_SynthIntegerParam_",
                "_Pointer_": ptr_str,
                "resolvedInt": value,
            }));
        }
    }

    // Also synthesize direct widget→loc mappings for LocalizedField ops that
    // consume those component-parameter pointers. This avoids losing the
    // mapping when intermediate pointer graphs are ambiguous after deep merges.
    if !param_ptr_to_loc.is_empty() || !param_ptr_to_string.is_empty() {
        for op in &child_scene.operations {
            let ty = op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if !ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedField")
                && !ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringField")
            {
                continue;
            }
            let Some(widget_ptr) = ptr_ref_str(op.get("widget")) else {
                continue;
            };
            let Some(input_ptr_raw) = ptr_ref_str(op.get("input")) else {
                continue;
            };
            if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedField") {
                let Some(loc_key) = param_ptr_to_loc.get(input_ptr_raw) else {
                    continue;
                };
                synthetics.push(serde_json::json!({
                    "_Type_": "_SynthLocalizedWidget_",
                    "widget": widget_ptr,
                    "resolvedLocKey": loc_key,
                }));
            } else if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringField") {
                let Some(value) = param_ptr_to_string.get(input_ptr_raw) else {
                    continue;
                };
                synthetics.push(serde_json::json!({
                    "_Type_": "_SynthStringWidget_",
                    "widget": widget_ptr,
                    "resolvedString": value,
                }));
            }
        }
    }

    if !synthetics.is_empty() {
        child_scene.operations.extend(synthetics);
    }
}

pub(crate) fn inject_dynamic_param_field_bindings(
    host_widget_id: BbNodeId,
    parent_operations: &[serde_json::Value],
    child_scene: &mut crate::bb_scene::BbScene,
    namespaced_hop: bool,
) {
    let parent_ptr_to_op: std::collections::HashMap<BbNodeId, &serde_json::Value> =
        parent_operations
            .iter()
            .filter_map(|op| Some((ptr_ref_to_id(op.get("_Pointer_"))?, op)))
            .collect();
    if parent_ptr_to_op.is_empty() {
        return;
    }

    let child_param_targets = child_component_parameter_targets(&child_scene.operations);
    if child_param_targets.is_empty() {
        return;
    }
    let relay_ptrs: std::collections::HashSet<BbNodeId> = child_scene
        .operations
        .iter()
        .filter(|op| op.get("_ParamRelay_").and_then(|v| v.as_bool()) == Some(true))
        .filter_map(|op| ptr_ref_to_id(op.get("_Pointer_")))
        .collect();

    let mut occupied_ptrs: std::collections::BTreeSet<BbNodeId> = child_scene
        .operations
        .iter()
        .filter_map(|op| ptr_ref_to_id(op.get("_Pointer_")))
        .collect();
    let mut injected_ops = Vec::new();

    for parent_op in parent_operations {
        let Some(widget_id) = ptr_ref_to_id(parent_op.get("widget")) else {
            continue;
        };
        if widget_id != host_widget_id {
            continue;
        }

        let Some(binding_kind) = component_parameter_kind_for_field_op(parent_op) else {
            continue;
        };
        let Some(field_name) = parent_op.get("field").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(target_ptrs) = child_param_targets
            .get(&(field_name.to_ascii_lowercase(), binding_kind))
        else {
            continue;
        };
        let Some(input_ptr) = ptr_ref_to_id(parent_op.get("input")) else {
            continue;
        };

        for &target_ptr in target_ptrs {
            // Every ComponentParameter replacement clone is a RELAY (eligible
            // for the NEXT hop's wiring): the OUTPUT card's number params run
            // master → power → outputinfo through a PLAIN (un-namespaced)
            // canvas hop, exactly like the pip chain's materialised hops.
            // Primaries stay scoped to the child's OWN defs (merged non-relay
            // defs are excluded above), so relays only ADD receivers keyed by
            // their cloned (slot, kind).
            let tag_relay = true;
            let _ = (namespaced_hop, &relay_ptrs);
            let mut ptr_remap = std::collections::HashMap::new();
            let _ = clone_binding_subgraph_into_child(
                input_ptr,
                target_ptr,
                &parent_ptr_to_op,
                &mut occupied_ptrs,
                &mut ptr_remap,
                &mut injected_ops,
                tag_relay,
            );
        }
    }

    if !injected_ops.is_empty() {
        child_scene.operations.extend(injected_ops);
    }
}


fn component_parameter_kind_for_field_op(op: &serde_json::Value) -> Option<&'static str> {
    match op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("") {
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsBooleanField") => Some("bool"),
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsIntegerField") => Some("int"),
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsNumberField") => Some("num"),
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedField") => Some("loc"),
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringField") => Some("string"),
        _ => None,
    }
}

fn component_parameter_kind_for_component_op(op: &serde_json::Value) -> Option<&'static str> {
    match op.get("_Type_").and_then(|v| v.as_str()).unwrap_or("") {
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsBooleanComponentParameter") => {
            Some("bool")
        }
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsIntegerComponentParameter") => {
            Some("int")
        }
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsNumberComponentParameter") => {
            Some("num")
        }
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsLocalizedComponentParameter") => {
            Some("loc")
        }
        ty if ty.eq_ignore_ascii_case("BuildingBlocks_BindingsStringComponentParameter") => {
            Some("string")
        }
        _ => None,
    }
}

fn clone_binding_subgraph_into_child(
    source_ptr: BbNodeId,
    target_ptr: BbNodeId,
    parent_ptr_to_op: &std::collections::HashMap<BbNodeId, &serde_json::Value>,
    occupied_ptrs: &mut std::collections::BTreeSet<BbNodeId>,
    ptr_remap: &mut std::collections::HashMap<BbNodeId, BbNodeId>,
    cloned_ops: &mut Vec<serde_json::Value>,
    tag_relay: bool,
) -> Option<BbNodeId> {
    if let Some(&mapped) = ptr_remap.get(&source_ptr) {
        return Some(mapped);
    }

    let source_op = *parent_ptr_to_op.get(&source_ptr)?;
    let new_ptr = if ptr_remap.is_empty() {
        target_ptr
    } else {
        next_free_operation_ptr(occupied_ptrs)
    };
    ptr_remap.insert(source_ptr, new_ptr);

    for dependency_ptr in binding_input_ptrs(source_op) {
        let _ = clone_binding_subgraph_into_child(
            dependency_ptr,
            target_ptr,
            parent_ptr_to_op,
            occupied_ptrs,
            ptr_remap,
            cloned_ops,
            tag_relay,
        );
    }

    let mut cloned = source_op.clone();
    cloned["_Pointer_"] = serde_json::Value::String(format!("ptr:{new_ptr}"));
    remap_binding_input_fields(&mut cloned, ptr_remap);
    // A cloned parameter op is a RELAY awaiting the next hop's wiring: when
    // this child later merges into ITS parent, the next injection must rewire
    // these clones too (the pip sizing's 3-level `pipsLengthMax` chain).
    if tag_relay && component_parameter_kind_for_component_op(&cloned).is_some() {
        cloned["_ParamRelay_"] = serde_json::Value::Bool(true);
    }
    cloned_ops.push(cloned);
    Some(new_ptr)
}

fn binding_input_ptrs(op: &serde_json::Value) -> Vec<BbNodeId> {
    let mut refs = Vec::new();
    for key in [
        "input",
        "inputA",
        "inputB",
        "inputL",
        "inputR",
        "inputTrue",
        "inputFalse",
        "nZeros",
    ] {
        if let Some(ptr) = ptr_ref_to_id(op.get(key)) {
            refs.push(ptr);
        }
    }
    if let Some(inputs) = op.get("inputs").and_then(|v| v.as_array()) {
        refs.extend(inputs.iter().filter_map(|value| ptr_ref_to_id(Some(value))));
    }
    refs
}

fn remap_binding_input_fields(
    op: &mut serde_json::Value,
    ptr_remap: &std::collections::HashMap<BbNodeId, BbNodeId>,
) {
    for key in [
        "input",
        "inputA",
        "inputB",
        "inputL",
        "inputR",
        "inputTrue",
        "inputFalse",
        "nZeros",
    ] {
        remap_binding_ptr_value(op.get_mut(key), ptr_remap);
    }
    if let Some(inputs) = op.get_mut("inputs").and_then(|v| v.as_array_mut()) {
        for value in inputs {
            remap_binding_ptr_value(Some(value), ptr_remap);
        }
    }
}

fn remap_binding_ptr_value(
    value: Option<&mut serde_json::Value>,
    ptr_remap: &std::collections::HashMap<BbNodeId, BbNodeId>,
) {
    let Some(value) = value else {
        return;
    };
    let Some(old_ptr) = ptr_ref_to_id(Some(value)) else {
        return;
    };
    let Some(&new_ptr) = ptr_remap.get(&old_ptr) else {
        return;
    };
    *value = serde_json::Value::String(format!("_PointsTo_:ptr:{new_ptr}"));
}

fn next_free_operation_ptr(occupied_ptrs: &mut std::collections::BTreeSet<BbNodeId>) -> BbNodeId {
    let mut next = occupied_ptrs.iter().next_back().copied().unwrap_or(0).wrapping_add(1);
    while occupied_ptrs.contains(&next) {
        next = next.wrapping_add(1);
    }
    occupied_ptrs.insert(next);
    next
}

fn ptr_ref_to_id(value: Option<&serde_json::Value>) -> Option<BbNodeId> {
    let raw = ptr_ref_str(value)?;
    raw.strip_prefix("ptr:")?.parse::<BbNodeId>().ok()
}

fn ptr_ref_str(value: Option<&serde_json::Value>) -> Option<&str> {
    match value {
        Some(serde_json::Value::String(s)) => s.strip_prefix("_PointsTo_:").or(Some(s.as_str())),
        Some(serde_json::Value::Object(obj)) => obj.get("_Pointer_").and_then(|v| v.as_str()),
        _ => None,
    }
}

// Parent→child component-parameter injection target selection (split from
// part_06 for the line cap): which ComponentParameter defs in a resolved
// child scene receive a parent canvas's slot wiring. Primary = last-wins
// live def among the child's OWN authored ops; `_ParamRelay_` clones are
// additional receivers; `_MergedOp_` non-relay defs (already wired at
// their own depth) and shadowed stale defs are excluded; expansion-band
// (`_ExpansionOp_`) template defs stay candidates.

fn child_component_parameter_targets(
    child_operations: &[serde_json::Value],
) -> std::collections::HashMap<(String, &'static str), Vec<BbNodeId>> {
    // A parameter SLOT broadcasts: EVERY live authored def reading the slot
    // receives the wiring (gen_mc_s_poweroutputinfo declares 'availablepower'
    // twice — the value text and the icon/title gate). Relay clones
    // (`_ParamRelay_`, created on injection/materialisation hops) are
    // additional receivers: every materialised instance of a slot consumes
    // the same wiring (the power pip chain's 'Max Piplist' across three
    // canvas levels).
    // Injection replaces a pointer's definition by appending a shadowing op
    // (evaluation is last-wins), so only each pointer's LIVE (last) definition
    // is a candidate: a stale shadowed def still carries its previous slot
    // name and would mis-route same-kind wiring (the power master's
    // `batterytotal` landing on already-rewired pip params). For scenes
    // without relays every stale def shares its pointer's slot name, so this
    // is exactly the long-verified last-wins behaviour.
    // "Live" is judged among ComponentParameter-kind defs only: synthetic
    // `_Synth*Param_` shadowers (authored paramInputValues) do not displace a
    // parameter op's candidacy — the verified medical baselines pin dynamic
    // slot wiring overriding those statics.
    let mut live_index: std::collections::HashMap<BbNodeId, usize> =
        std::collections::HashMap::new();
    for (index, op) in child_operations.iter().enumerate() {
        if component_parameter_kind_for_component_op(op).is_none() {
            continue;
        }
        if let Some(ptr) = ptr_ref_to_id(op.get("_Pointer_")) {
            live_index.insert(ptr, index);
        }
    }
    let mut primaries: std::collections::HashMap<(String, &'static str), Vec<BbNodeId>> =
        std::collections::HashMap::new();
    let mut relays: std::collections::HashMap<(String, &'static str), Vec<BbNodeId>> =
        std::collections::HashMap::new();
    for (index, op) in child_operations.iter().enumerate() {
        let Some(kind) = component_parameter_kind_for_component_op(op) else {
            continue;
        };
        let Some(parameter) = op.get("parameter").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(ptr) = ptr_ref_to_id(op.get("_Pointer_")) else {
            continue;
        };
        if live_index.get(&ptr) != Some(&index) {
            continue;
        }
        let key = (parameter.to_ascii_lowercase(), kind);
        if op.get("_ParamRelay_").and_then(|v| v.as_bool()) == Some(true) {
            relays.entry(key.clone()).or_default().push(ptr);
            continue;
        }
        // A `_MergedOp_` def belongs to one of the child's own descendants
        // and was already wired during the child's resolve; only relays may
        // re-receive a parent's wiring. Without this, the power master's
        // integer ParamInput0 (`pipsLengthMax`) lands on the battery card's
        // merged `batteryremaining` def (same slot + kind) as a stray "6".
        // Expansion-band defs (`_ExpansionOp_`, widget-standard templates)
        // stay candidates: templates merge un-wired and the host's parent
        // wiring legitimately targets them (the medical close button's
        // icon-path ParamInput0).
        if op.get("_MergedOp_").and_then(|v| v.as_bool()) == Some(true)
            && op.get("_ExpansionOp_").and_then(|v| v.as_bool()) != Some(true)
        {
            continue;
        }
        primaries.entry(key).or_default().push(ptr);
    }
    let mut targets: std::collections::HashMap<(String, &'static str), Vec<BbNodeId>> =
        std::collections::HashMap::new();
    for (key, ptrs) in primaries {
        targets.entry(key).or_default().extend(ptrs);
    }
    for (key, ptrs) in relays {
        let slot = targets.entry(key).or_default();
        for ptr in ptrs {
            if !slot.contains(&ptr) {
                slot.push(ptr);
            }
        }
    }
    targets
}

#[cfg(test)]
mod tests {
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
    fn extract_record_name_long_file_url_uses_basename_without_json() {
        assert_eq!(
            extract_record_name("file://./a/b/c/gen_mc_s_target.json"),
            "gen_mc_s_target"
        );
    }


    #[test]
    fn extract_record_name_short_file_url_uses_local_basename() {
        assert_eq!(extract_record_name("file://./local.json"), "local");
    }


    #[test]
    fn extract_record_name_bare_json_name_strips_extension() {
        assert_eq!(extract_record_name("my_canvas.json"), "my_canvas");
    }


    #[test]
    fn extract_record_name_bare_name_is_unchanged() {
        assert_eq!(extract_record_name("my_canvas"), "my_canvas");
    }


    #[test]
    fn extract_record_name_fixture_path_returns_target_name() {
        assert_eq!(
            extract_record_name("file://./../../../../../../../../../../../libs/foundry/records/ui/buildingblocks/ships/displays/mfdscreens/mc_mfdcomponents/screens/target/types/gen_mc_s_target.json"),
            "gen_mc_s_target"
        );
    }


    #[test]
    fn extract_record_name_mixed_case_json_extension_strips_extension() {
        assert_eq!(extract_record_name("file://./local.Json"), "local");
    }


    #[test]
    fn resolve_inlines_referenced_animation_timeline_keyframes() {
        let timeline_path = "file://./animation/as_ping_slidein_a.json";
        let root = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.AnimationHost",
            "_RecordValue_": {
                "size": {"x": 200.0, "y": 100.0},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "animated_image",
                        "isActive": true,
                        "position": {"x": 0.0, "y": 0.0, "z": 0.0},
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 50.0}
                        },
                        "animation": {
                            "animationTimeline": {
                                "timelineRecord": timeline_path
                            }
                        }
                    }
                ],
                "operations": []
            }
        });
        let timeline = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Timeline.AS_Ping_SlideIn_A",
            "_RecordValue_": {
                "timeline": {
                    "_Type_": "BuildingBlocks_TimelineTypeEmbedded",
                    "keyframes": [
                        {
                            "percent": 0.0,
                            "modifiers": [
                                {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 50.0}}
                            ]
                        },
                        {
                            "percent": 1.0,
                            "modifiers": [
                                {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 0.0}}
                            ]
                        }
                    ]
                }
            }
        });

        let scene = resolve_canvas_graph(&root, None, &|path| {
            if path == timeline_path {
                Ok(timeline.clone())
            } else {
                Err(format!("unexpected path {path}"))
            }
        })
        .expect("resolved scene");

        let node = scene.nodes.get(&1).expect("node");
        assert!(
            node.raw
                .get("animation")
                .and_then(|animation| animation.get("animationTimeline"))
                .and_then(|timeline| timeline.get("keyframes"))
                .is_some(),
            "expected timeline keyframes to be inlined"
        );
        let layout = crate::bb_layout::layout_with_animation_sample(&scene, 200, 100, Some(50.0), false, false);
        assert!((layout.rects[&1].x - 25.0).abs() < 0.5);
    }


    #[test]
    fn resolve_with_rsi_manufacturer_fetches_brand_child() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let child = one_node_canvas();
        let scene = resolve_canvas_graph(&json, Some("rsi"), &|_p| Ok(child.clone()))
            .expect("resolve failed");
        assert!(
            scene.nodes.len() > 2,
            "expected >2 nodes after merge, got {}",
            scene.nodes.len()
        );
    }


    #[test]
    fn resolve_with_no_manufacturer_uses_default_styles() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let child = three_node_canvas();
        let scene = resolve_canvas_graph(&json, None, &|_p| Ok(child.clone()))
            .expect("resolve failed");
        assert!(
            scene.nodes.len() >= 5,
            "expected >=5 nodes after merge, got {}",
            scene.nodes.len()
        );
    }


    #[test]
    fn resolve_with_error_fetcher_does_not_panic() {
        let json = load_fixture("MC_S_Target_Master_b8d2d65c.json");
        let scene = resolve_canvas_graph(&json, None, &|_p| Err("stub error".to_string()))
            .expect("resolve must not fail even when fetcher errors");
        assert!(
            scene.nodes.len() >= 2,
            "expected at least the 2 original nodes, got {}",
            scene.nodes.len()
        );
    }


    #[test]
    fn texture_body_background_uses_standard_bioc_texture_style() {
        let body_background_texture_tag = "cc37cf84-f93a-4a60-82a6-efea090069b1";
        let root = serde_json::json!({
            "_RecordName_": "I_Med_Test",
            "_RecordId_": "00000000-0000-0000-0000-000000000200",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_":"Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "style": "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_bioc.json",
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [{
                    "_Pointer_": "ptr:24",
                    "_Type_": "BuildingBlocks_WidgetBodyBackground",
                    "name": "Background2D",
                    "styleTags": [],
                    "parent": null,
                    "isActive": true,
                    "exportNode": true,
                    "sizing": {
                        "_Type_": "BuildingBlocks_Size",
                        "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 64.0, "behavior": "Fixed"},
                        "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 64.0, "behavior": "Fixed"}
                    },
                    "backgroundType": "Texture"
                }]
            }
        });
        let scene = resolve_canvas_graph(&root, Some("drak"), &|path| {
            if path.contains("bodybackgroundwidgetstandard") {
                Ok(body_background_standard_fixture(body_background_texture_tag))
            } else {
                Err(format!("no fixture for {path}"))
            }
        })
        .expect("resolve body background");

        let node = scene
            .nodes
            .values()
            .find(|node| node.name == "Background2D")
            .expect("background node");
        assert!(
            node.style_tag_uuids
                .iter()
                .any(|tag| tag == body_background_texture_tag),
            "expected source-derived body-background texture tag"
        );
        assert_eq!(
            node.raw.get("ImagePath").and_then(|value| value.as_str()),
            Some("UI/Textures/ModularKitStyles/BIOC/BIOC_bg.tif")
        );
    }
}

#[cfg(test)]
mod tests_c {
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
    fn merge_scales_child_canvas_units_to_host_slot_size() {
        // Parent canvas authored at 1920x1080 with a 400x400 WidgetCanvas slot.
        let host = serde_json::json!({
            "_RecordName_": "host_touch_slot",
            "_RecordId_": "00000000-0000-0000-0000-000000000111",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_":"Vec3","x":1920.0,"y":1080.0,"z":0.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [{
                        "_Type_":"BuildingBlocks_StyleEntry",
                        "name":"Default",
                        "matchTo":"touch_slot",
                        "conditionsList":[],
                        "modifiers":[
                            {"_Type_":"BuildingBlocks_FieldModifier",
                             "field":{"_Type_":"BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord","value":"file://./child_touch.json"}}
                        ]
                    }]
                },
                "brandStyles": [],
                "scene": [
                    {"_Pointer_":"ptr:1","_Type_":"BuildingBlocks_DisplayWidget","name":"root"},
                    {"_Pointer_":"ptr:2","_Type_":"BuildingBlocks_WidgetCanvas","name":"touch_slot","parent":"_PointsTo_:ptr:1",
                     "sizing":{"_Type_":"BuildingBlocks_Size",
                        "width":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":400.0,"behavior":"Fixed"},
                        "height":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":400.0,"behavior":"Fixed"}}}
                ]
            }
        });
        // Child canvas authored at 1024x1024 with a 512px image.
        let child = serde_json::json!({
            "_RecordName_": "child_touch",
            "_RecordId_": "00000000-0000-0000-0000-000000000112",
            "_RecordValue_": {
                "_Type_":"BuildingBlocks_Canvas",
                "size":{"_Type_":"Vec3","x":1024.0,"y":1024.0,"z":0.0},
                "scene":[
                    {"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_DisplayWidget","name":"child_root",
                     "sizing":{"_Type_":"BuildingBlocks_Size",
                        "width":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":1024.0,"behavior":"Fixed"},
                        "height":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":1024.0,"behavior":"Fixed"}}},
                    {"_Pointer_":"ptr:11","_Type_":"BuildingBlocks_WidgetImage","name":"child_image","parent":"_PointsTo_:ptr:10",
                     "sizing":{"_Type_":"BuildingBlocks_Size",
                        "width":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":512.0,"behavior":"Fixed"},
                        "height":{"_Type_":"BuildingBlocks_FixedOrRelativeValue","value":256.0,"behavior":"Fixed"}}}
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|path| {
            if path.to_ascii_lowercase().contains("child_touch.json") {
                Ok(child.clone())
            } else {
                Err(format!("unexpected fetch path: {path}"))
            }
        })
        .expect("resolve failed");

        let image_node = scene
            .nodes
            .values()
            .find(|n| n.name == "child_image")
            .expect("child image node missing after merge");
        let w = match image_node.sizing.width {
            BbValue::Fixed(v) => v,
            _ => panic!("child image width must stay fixed"),
        };
        let h = match image_node.sizing.height {
            BbValue::Fixed(v) => v,
            _ => panic!("child image height must stay fixed"),
        };
        assert!(
            (w - 200.0).abs() < 0.01 && (h - 100.0).abs() < 0.01,
            "expected 512x256 in 1024-child scaled into 400x400 slot => 200x100, got {w}x{h}"
        );
    }


    #[test]
    fn resolve_follows_widget_canvas_canvas_field() {
        let content_url = "file://./content_canvas.json";
        let host = host_canvas_with_widget_canvas_url(content_url);

        // Content canvas has 5 nodes
        let content = serde_json::json!({
            "_RecordName_": "content_canvas",
            "_RecordId_": "00000000-0000-0000-0000-000000000011",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "content_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetIcon", "name": "icon1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetIcon", "name": "icon2", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetTextField", "name": "text2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|url| {
            // Framework records (tag database / widget standards) may be probed
            // by the widget-standard expansion; only the content canvas counts.
            if url != content_url {
                return Err(format!("framework record unavailable in fixture: {url}"));
            }
            Ok(content.clone())
        })
        .expect("resolve failed");

        // host (2) + content (5) = 7 nodes
        assert_eq!(
            scene.nodes.len(),
            7,
            "expected 7 merged nodes (2 host + 5 content), got {}",
            scene.nodes.len()
        );
    }


    #[test]
    fn instantiated_false_widget_is_deactivated_and_not_merged() {
        let host = serde_json::json!({
            "_RecordName_": "test_host_instantiated_false",
            "_RecordId_": "00000000-0000-0000-0000-000000000120",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "attract_canvas",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "file://./content_canvas.json"
                    }
                ],
                "operations": [
                    {"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_BindingsBooleanVariable","value":"Bed/state.BaseScreens.Attract"},
                    {"_Pointer_":"ptr:11","_Type_":"BuildingBlocks_BindingsBooleanVariable","value":"Bed/state.BaseScreens.MainMenu"},
                    {"_Pointer_":"ptr:12","_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:2","field":"Instantiated","input":"_PointsTo_:ptr:10"}
                ]
            }
        });
        let content = serde_json::json!({
            "_RecordName_": "content_canvas",
            "_RecordId_": "00000000-0000-0000-0000-000000000121",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "content_root"}
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|url| {
            assert_eq!(url, "file://./content_canvas.json");
            Ok(content.clone())
        })
        .expect("resolve failed");

        let widget = scene
            .nodes
            .get(&2)
            .expect("host widget canvas ptr:2 must exist");
        assert!(!widget.is_active, "Instantiated=false widget must be deactivated");
        assert_eq!(
            scene.nodes.len(),
            2,
            "inactive widget should not merge child canvas content"
        );
    }


    #[test]
    fn child_canvas_is_active_gate_uses_parent_selected_state() {
        let host = serde_json::json!({
            "_RecordName_": "test_host_selected_state",
            "_RecordId_": "00000000-0000-0000-0000-000000000130",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                "staticVariables": [
                    {"name": "Standing/state.BaseScreens.Attract", "value": true}
                ],
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"},
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "footer_slot",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "file://./footer_canvas.json"
                    }
                ],
                "operations": [
                    {"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"Standing/state.BaseScreens.Attract"}
                ]
            }
        });
        let footer = serde_json::json!({
            "_RecordName_": "footer_canvas",
            "_RecordId_": "00000000-0000-0000-0000-000000000131",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 800.0, "y": 80.0, "z": 0.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "footer_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField", "name": "patient_name", "parent": "_PointsTo_:ptr:1"}
                ],
                "operations": [
                    {"_Pointer_":"ptr:10","_Type_":"BuildingBlocks_BindingsBooleanVariable","binding":"Standing/state.BaseScreens.Attract"},
                    {"_Pointer_":"ptr:11","_Type_":"BuildingBlocks_BindingsBooleanInvert","input":"_PointsTo_:ptr:10"},
                    {"_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:2","field":"IsActive","input":"_PointsTo_:ptr:11"}
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|url| {
            assert_eq!(url, "file://./footer_canvas.json");
            Ok(footer.clone())
        })
        .expect("resolve failed");

        let patient = scene
            .nodes
            .values()
            .find(|node| node.name == "patient_name")
            .expect("merged footer patient node must exist");
        assert!(
            !patient.is_active,
            "child IsActive gate must inherit the parent's selected Attract state"
        );
    }
}

#[cfg(test)]
mod tests_d {
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
    fn resolve_follows_multiple_widget_canvas_levels() {
        // Verify that the resolver follows WidgetCanvas.canvas URLs at depth 1,
        // 2, 3, and 4 (all within MAX_CANVAS_DEPTH = 8).
        //
        // Chain: host → level1 → level2 → level3 → level4 (leaf)
        // Each level has 1 root node + 1 WidgetCanvas child (level4 is a leaf).
        fn make_canvas(name: &str, id: &str, child_url: Option<&str>) -> serde_json::Value {
            let scene_nodes: Vec<serde_json::Value> = {
                let mut nodes = vec![serde_json::json!({
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_DisplayWidget",
                    "name": format!("{name}_root")
                })];
                if let Some(url) = child_url {
                    nodes.push(serde_json::json!({
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": format!("{name}_canvas"),
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": url
                    }));
                }
                nodes
            };
            serde_json::json!({
                "_RecordName_": name,
                "_RecordId_": id,
                "_RecordValue_": {
                    "_Type_": "BuildingBlocks_Canvas",
                    "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                    "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                    "brandStyles": [],
                    "scene": scene_nodes
                }
            })
        }

        // level4 is a leaf — no child URL, so no fetch attempt for level5.
        let level4 = make_canvas("level4", "00000000-0000-0000-0000-000000000004", None);
        let level3 = make_canvas("level3", "00000000-0000-0000-0000-000000000003", Some("file://./level4.json"));
        let level2 = make_canvas("level2", "00000000-0000-0000-0000-000000000002", Some("file://./level3.json"));
        let level1 = make_canvas("level1", "00000000-0000-0000-0000-000000000001", Some("file://./level2.json"));
        let host = host_canvas_with_widget_canvas_url("file://./level1.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&host, None, &|url| {
            fetch_count.set(fetch_count.get() + 1);
            Ok(match url {
                "file://./level1.json" => level1.clone(),
                "file://./level2.json" => level2.clone(),
                "file://./level3.json" => level3.clone(),
                "file://./level4.json" => level4.clone(),
                _ => return Err(format!("unexpected fetch: {url}")),
            })
        })
        .expect("resolve failed");

        // Depth chain: host(depth=0)→level1(1)→level2(2)→level3(3)→level4(4)
        // level4 is a leaf, so exactly 4 fetches occur.
        assert_eq!(fetch_count.get(), 4, "expected 4 fetches (levels 1–4), got {}", fetch_count.get());

        // host(2) + level1(2) + level2(2) + level3(2) + level4(1) = 9 nodes
        assert_eq!(scene.nodes.len(), 9, "expected 9 merged nodes, got {}", scene.nodes.len());
    }


    #[test]
    fn resolve_does_not_follow_widget_canvas_url_beyond_depth_cap() {
        // A chain that loops back to the same URL exercises cycle-detection.
        //
        // Since the Pass 2 outer loop no longer deduplicates by URL (to allow
        // intentional multi-instantiation of template canvases), cycles are
        // now bounded purely by MAX_CANVAS_DEPTH.  Each recursive level fetches
        // the canvas once, so the total fetch count equals MAX_CANVAS_DEPTH.
        fn make_level_canvas(child_url: &str) -> serde_json::Value {
            serde_json::json!({
                "_RecordName_": "level_n",
                "_RecordId_": "00000000-0000-0000-0000-000000000099",
                "_RecordValue_": {
                    "_Type_": "BuildingBlocks_Canvas",
                    "size": {"_Type_": "Vec3", "x": 800.0, "y": 600.0, "z": 0.0},
                    "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                    "brandStyles": [],
                    "scene": [
                        {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"},
                        {
                            "_Pointer_": "ptr:2",
                            "_Type_": "BuildingBlocks_WidgetCanvas",
                            "name": "next_level",
                            "parent": "_PointsTo_:ptr:1",
                            "canvas": child_url
                        }
                    ]
                }
            })
        }

        let host = host_canvas_with_widget_canvas_url("file://./level.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&host, None, &|_url| {
            fetch_count.set(fetch_count.get() + 1);
            // Every fetch returns a canvas that points back to "file://./level.json".
            // The depth cap terminates traversal at MAX_CANVAS_DEPTH levels.
            Ok(make_level_canvas("file://./level.json"))
        })
        .expect("resolve failed");

        // Cycle detection: the same normalised URL "level" is only fetched once.
        assert!(
            fetch_count.get() <= super::MAX_CANVAS_DEPTH as u32,
            "fetched {} times, should be ≤ MAX_CANVAS_DEPTH={}",
            fetch_count.get(), super::MAX_CANVAS_DEPTH,
        );

        // There must be merged nodes from the fetched levels.
        assert!(scene.nodes.len() >= 2, "must have at least 2 merged nodes, got {}", scene.nodes.len());
    }


    #[test]
    fn pass2_instantiates_same_template_url_multiple_times() {
        // Regression test for B11: when multiple WidgetCanvas nodes in the
        // root canvas all reference the same template URL with different
        // paramInputValues (e.g. the 5 chiclet slots in the annunciator screen
        // all use `h_eng_annunciator`), each slot must produce an independent
        // set of merged nodes.
        //
        // Before the fix, the Pass 2 outer loop deduped by URL — only the first
        // slot was resolved and the other 4 produced no nodes.
        let template = serde_json::json!({
            "_RecordName_": "chiclet_template",
            "_RecordId_": "00000000-0000-0000-0000-000000000090",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "chiclet_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetText", "name": "chiclet_label",
                     "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });

        // Host canvas has 3 WidgetCanvas nodes all pointing to the same template.
        let host = serde_json::json!({
            "_RecordName_": "annunciator_host",
            "_RecordId_": "00000000-0000-0000-0000-000000000091",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 1920.0, "y": 1080.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "host_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "slot_a",
                     "parent": "_PointsTo_:ptr:1", "canvas": "file://./chiclet_template.json"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "slot_b",
                     "parent": "_PointsTo_:ptr:1", "canvas": "file://./chiclet_template.json"},
                    {"_Pointer_": "ptr:4", "_Type_": "BuildingBlocks_WidgetCanvas", "name": "slot_c",
                     "parent": "_PointsTo_:ptr:1", "canvas": "file://./chiclet_template.json"}
                ]
            }
        });

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&host, None, &|url| {
            assert_eq!(url, "file://./chiclet_template.json", "unexpected url: {url}");
            fetch_count.set(fetch_count.get() + 1);
            Ok(template.clone())
        })
        .expect("resolve failed");

        // The template must be fetched once per slot (3 slots → 3 fetches).
        assert_eq!(
            fetch_count.get(),
            3,
            "expected 3 fetches (one per template slot), got {}",
            fetch_count.get()
        );

        // host(4) + 3 × template(2) = 10 nodes.
        assert_eq!(
            scene.nodes.len(),
            10,
            "expected 10 merged nodes (4 host + 3×2 template), got {}",
            scene.nodes.len()
        );
    }


    #[test]
    fn gen_mc_s_target_fixture_has_widget_text_fields() {
        // The GEN_MC_S_Target canvas contains WidgetTextField nodes inline.
        // Resolving it (with a failing fetcher for nested canvas URLs) must
        // return a scene that includes at least one WidgetTextField node.
        use crate::bb_scene::BbNodeType;
        let json = load_fixture("GEN_MC_S_Target_dd9ed6dc.json");
        let scene = resolve_canvas_graph(&json, None, &|_p| Err("no fetcher in test".to_string()))
            .expect("resolve failed");
        let text_count = scene.nodes.values().filter(|n| n.ty == BbNodeType::WidgetTextField).count();
        assert!(
            text_count >= 1,
            "expected at least 1 WidgetTextField in GEN_MC_S_Target, got {}",
            text_count,
        );
    }


    #[test]
    fn mc_s_self_master_differs_from_gen_mc_s_target() {
        // MC_S_Self_Master and GEN_MC_S_Target represent different MFD screens.
        // Their resolved scenes must have different node counts (proving that
        // different root canvases produce distinct merged results).
        let target_json = load_fixture("GEN_MC_S_Target_dd9ed6dc.json");
        let self_json = load_fixture("MC_S_Self_Master_680a71df.json");

        let target_scene =
            resolve_canvas_graph(&target_json, None, &|_p| Err("no fetcher".to_string()))
                .expect("target resolve failed");
        let self_scene =
            resolve_canvas_graph(&self_json, None, &|_p| Err("no fetcher".to_string()))
                .expect("self resolve failed");

        assert_ne!(
            target_scene.nodes.len(),
            self_scene.nodes.len(),
            "GEN_MC_S_Target ({} nodes) and MC_S_Self_Master ({} nodes) must differ",
            target_scene.nodes.len(),
            self_scene.nodes.len(),
        );
    }
}

#[cfg(test)]
mod tests_e {
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


    /// `defaultStyles.entries` are EDITOR-TIME defaults that a RESOLVING brand
    /// supersedes: the annunciator's defaultStyles `CornerRadius` (radius 30)
    /// would round the chiclet frames the in-game reference shows square, and
    /// the power `System Icon Color` (absent from drak's brand container) would
    /// tint the system icons the reference shows white — but BOTH canvases
    /// resolve a matching brand (drak), which is what suppresses their
    /// defaultStyles. Model that faithfully: a matching brand present →
    /// defaultStyles NOT applied. (The no-brand-match FALLBACK — where
    /// defaultStyles DO become the runtime look — is the companion test below.)
    fn default_styles_base_canvas(brand_styles: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.DefaultStylesBase",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {
                    "_Type_": "BuildingBlocks_DefaultStyles",
                    "sharedStyles": null,
                    "entries": [
                        {
                            "_Type_": "BuildingBlocks_StyleEntry",
                            "name": "System Icon Color",
                            "conditionsList": [
                                {
                                    "_Type_": "BuildingBlocks_StyleConditionList",
                                    "conditions": [
                                        {
                                            "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                                            "tag": {"_RecordId_": "e5cd9d57-953e-4e53-8759-839ab5c0b8d9"}
                                        }
                                    ]
                                }
                            ],
                            "modifiers": [
                                {
                                    "_Type_": "BuildingBlocks_FieldModifierColor",
                                    "field": "FillColor",
                                    "color": {
                                        "_Type_": "BuildingBlocks_ColorStyle",
                                        "color": "Accent2",
                                        "alpha": 1.0
                                    }
                                }
                            ],
                            "transitions": []
                        }
                    ]
                },
                "brandStyles": brand_styles,
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCustomShape",
                     "name": "shape_Icon", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "styleTags": [
                        {"_RecordId_": "e5cd9d57-953e-4e53-8759-839ab5c0b8d9"}
                     ]},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetCustomShape",
                     "name": "shape_Plain", "parent": "_PointsTo_:ptr:1", "isActive": true}
                ],
                "operations": []
            }
        })
    }

    #[test]
    fn default_styles_suppressed_when_a_brand_resolves() {
        // A matching `s_drak` brand resolves for a `drak` ship — exactly the
        // power/annunciator situation. defaultStyles stay editor-time; the
        // System Icon Color must NOT tint the icon.
        let root = default_styles_base_canvas(serde_json::json!([
            {
                "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/brands/s_drak.json",
                "entries": []
            }
        ]));
        let scene = resolve_canvas_graph(&root, Some("drak"), &|p| Err(format!("no fetch: {p}")))
            .expect("resolve");
        let icon = scene
            .nodes
            .values()
            .find(|n| n.name == "shape_Icon")
            .expect("icon node");
        assert!(
            icon.raw.get("FillColorToken").is_none(),
            "a resolving brand suppresses defaultStyles — got {:?}",
            icon.raw.get("FillColorToken")
        );
    }

    /// The no-brand-match FALLBACK: when no `brandStyles[]` entry matches the
    /// ship manufacturer, `defaultStyles.entries` become the runtime brand-tier
    /// look (the DRAK velocity-num SCREEN readout sizing). Here the only
    /// declared brand is `s_grey_hud`, which a `drak` ship does not match, so
    /// the System Icon Color defaultStyle now applies.
    #[test]
    fn default_styles_apply_as_fallback_when_no_brand_matches() {
        let root = default_styles_base_canvas(serde_json::json!([
            {
                "brandIdentifier": "file://libs/foundry/records/ui/buildingblocks/styles/s_grey_hud.json",
                "entries": []
            }
        ]));
        let scene = resolve_canvas_graph(&root, Some("drak"), &|p| Err(format!("no fetch: {p}")))
            .expect("resolve");
        let icon = scene
            .nodes
            .values()
            .find(|n| n.name == "shape_Icon")
            .expect("icon node");
        assert_eq!(
            icon.raw.get("FillColorToken").and_then(|v| v.as_str()),
            Some("Accent2"),
            "no brand matched → defaultStyles apply as the runtime fallback"
        );
    }

    #[test]
    fn inject_param_overrides_synthesizes_boolean_component_parameter_values() {
        let mut child_scene = crate::bb_scene::BbScene {
            coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw,
            canvas_size: (1920.0, 1080.0),
            roots: Vec::new(),
            nodes: std::collections::BTreeMap::new(),
            operations: vec![serde_json::json!({
                "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                "_Pointer_": "ptr:42",
                "parameter": "ParamInput0",
                "defaultValue": false
            })],
        };
        let param_inputs = vec![serde_json::json!({
            "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
            "parameter": "ParamInput0",
            "value": true
        })];

        inject_param_overrides(&param_inputs, &mut child_scene);

        let synth = child_scene
            .operations
            .iter()
            .find(|op| {
                op.get("_Type_").and_then(|v| v.as_str()) == Some("_SynthBooleanParam_")
                    && op.get("_Pointer_").and_then(|v| v.as_str()) == Some("ptr:42")
            })
            .expect("expected _SynthBooleanParam_ for ptr:42");
        assert_eq!(synth.get("resolvedBool").and_then(|v| v.as_bool()), Some(true));
    }


    #[test]
    fn resolve_canvas_graph_inherits_dynamic_integer_param_bindings_into_child_canvas() {
        let host = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.DynamicParamHost",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "host_root"
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "slot",
                        "parent": "_PointsTo_:ptr:1",
                        "canvas": "file://./child_canvas.json"
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsIntegerField",
                        "widget": "_PointsTo_:ptr:2",
                        "field": "ParamInput0",
                        "input": "_PointsTo_:ptr:10"
                    },
                    {
                        "_Pointer_": "ptr:10",
                        "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                        "binding": "/AnnunciatorProvider/Issues/[0001]/Severity"
                    }
                ]
            }
        });
        let child = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.DynamicParamChild",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetImage",
                        "name": "item"
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsStringField",
                        "widget": "_PointsTo_:ptr:1",
                        "field": "PrimaryStateTag",
                        "input": "_PointsTo_:ptr:11"
                    },
                    {
                        "_Pointer_": "ptr:11",
                        "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                        "input": "_PointsTo_:ptr:12",
                        "defaultValue": "",
                        "values": [
                            {
                                "first": 1,
                                "second": {
                                    "_RecordName_": "Tag.active",
                                    "_RecordId_": "1477f18d-9b3e-4e5c-8047-dc60ba606ddb"
                                }
                            }
                        ]
                    },
                    {
                        "_Pointer_": "ptr:12",
                        "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                        "parameter": "ParamInput0",
                        "defaultValue": 0
                    }
                ]
            }
        });

        let scene = resolve_canvas_graph(&host, None, &|url| match url {
            "file://./child_canvas.json" => Ok(child.clone()),
            _ => Err(format!("unexpected fetch: {url}")),
        })
        .expect("resolve failed");

        let item_id = scene
            .nodes
            .values()
            .find(|node| node.name == "item")
            .map(|node| node.id)
            .expect("expected merged child node");
        let resolver = crate::bb_bindings::BindingResolver::from_operations(&scene.operations);
        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path(
            "/AnnunciatorProvider/Issues/[0001]/Severity",
            crate::canvas::Value::Int(1),
        );

        assert_eq!(
            resolver.resolve_field_text(item_id, "PrimaryStateTag", &defaults),
            Some("Tag.active".to_string())
        );
    }


    /// A child canvas's state-conditioned style entries must apply once the
    /// node state tags are resolvable — the annunciator chiclet severities
    /// arrive via PARENT-injected dynamic params, so at the child's own
    /// cascade time the state tags don't exist yet and entries like
    /// "Critical - Text" (Parent[AnyOfTag state]) can never match. The child's
    /// entries are deferred and re-applied (subtree-scoped) after
    /// `resolve_state_tags_into_scene` at the parent level.
    #[test]
    fn registry_boolean_default_keeps_instantiated_widget_active() {
        let host = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.BodyBackgroundGateHost",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetBodyBackground",
                     "name": "background_Main", "parent": "_PointsTo_:ptr:1",
                     "isActive": true, "backgroundType": "Texture"}
                ],
                "operations": [
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                     "binding": "backgroundenabled"},
                    {"_Type_": "BuildingBlocks_BindingsBooleanField",
                     "widget": "_PointsTo_:ptr:2", "field": "Instantiated",
                     "input": "_PointsTo_:ptr:10"}
                ]
            }
        });

        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("backgroundenabled", crate::canvas::Value::Bool(true));
        let scene = resolve_canvas_graph_with_defaults(
            &host,
            None,
            &|other| Err(format!("framework record unavailable in fixture: {other}")),
            None,
            None,
            &defaults,
        )
        .expect("resolve failed");

        let bg = scene
            .nodes
            .values()
            .find(|node| node.name == "background_Main")
            .expect("background node");
        assert!(
            bg.is_active,
            "a registry-pinned boolean binding (backgroundenabled=true) keeps the \
             Instantiated-gated body background active"
        );
    }

    #[test]
    fn registry_false_boolean_deactivates_is_active_gated_widget() {
        // The annunciator strip's image_BG gates IsActive on the host boolean
        // `EnableBackground`, which the registry pins FALSE (in-game the strip
        // background is near pure black). A registry-known false must win over
        // the conservative keep-active rescues for unresolved bindings.
        let host = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.StripBackgroundHost",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetImage",
                     "name": "image_BG", "parent": "_PointsTo_:ptr:1",
                     "isActive": true}
                ],
                "operations": [
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                     "binding": "EnableBackground"},
                    {"_Type_": "BuildingBlocks_BindingsBooleanField",
                     "widget": "_PointsTo_:ptr:2", "field": "IsActive",
                     "input": "_PointsTo_:ptr:10"}
                ]
            }
        });

        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("EnableBackground", crate::canvas::Value::Bool(false));
        let scene = resolve_canvas_graph_with_defaults(
            &host,
            None,
            &|other| Err(format!("framework record unavailable in fixture: {other}")),
            None,
            None,
            &defaults,
        )
        .expect("resolve failed");

        let bg = scene
            .nodes
            .values()
            .find(|node| node.name == "image_BG")
            .expect("background node");
        assert!(
            !bg.is_active,
            "a registry-pinned FALSE boolean binding deactivates the IsActive-gated widget"
        );
    }

    #[test]
    fn body_background_standard_uses_owning_canvas_brand_container() {
        let texture_tag = "cc37cf84-f93a-4a60-82a6-efea090069b1";
        let texture_entry = |image_path: &str| {
            serde_json::json!({
                "_Type_": "BuildingBlocks_StyleEntry",
                "name": "Background Texture Style",
                "conditionsList": [
                    {
                        "_Type_": "BuildingBlocks_StyleConditionList",
                        "conditions": [
                            {"_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                             "tag": {"_RecordId_": texture_tag}}
                        ]
                    }
                ],
                "modifiers": [
                    {"_Type_": "BuildingBlocks_FieldModifierString",
                     "field": "ImagePath", "value": image_path},
                    {"_Type_": "BuildingBlocks_FieldModifierNumber",
                     "field": "Alpha", "value": 0.2}
                ],
                "transitions": []
            })
        };
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.BodyBackgroundWidgetStandard",
            "_RecordValue_": {
                "size": {"x": 1920.0, "y": 1080.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [
                    {"brandIdentifier": "file://./styles/s_drak_env.json",
                     "entries": [texture_entry("UI/Textures/ModularKitStyles/DRAK/Drake_DoorPanel_Background-DarkTheme.tif")]},
                    {"brandIdentifier": "file://./styles/s_drak_hud.json",
                     "entries": [texture_entry("UI/Textures/I_InteractiveScreens/MFD/DRAK/DRAK_GroundVehicle_Dashboard_background_2.tif")]}
                ],
                "scene": [],
                "operations": [
                    {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                     "input": null, "defaultValue": null,
                     "values": [
                        {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 1,
                         "second": {"_RecordId_": texture_tag,
                                    "_RecordName_": format!("Tag.{texture_tag}")}}
                     ]}
                ]
            }
        });
        // The content canvas authors the body-background widget and selects the
        // HUD brand container; the host above it has no brand containers at all
        // (the power master) and must not re-apply the standard with the
        // manufacturer-prefix fallback (which hits `s_drak_env` first).
        let content = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.BodyBackgroundContent",
            "_RecordValue_": {
                "size": {"x": 800.0, "y": 600.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [
                    {"brandIdentifier": "file://./styles/s_drak_hud.json", "entries": []}
                ],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "content_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetBodyBackground",
                     "name": "background_Main", "parent": "_PointsTo_:ptr:1",
                     "isActive": true, "backgroundType": "Texture"}
                ],
                "operations": []
            }
        });
        let host = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.BodyBackgroundHostMaster",
            "_RecordValue_": {
                "size": {"x": 1920.0, "y": 1080.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "host_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "slot", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "canvas": "file://./body_background_content.json"}
                ],
                "operations": []
            }
        });

        let defaults = crate::defaults::DefaultValueRegistry::new();
        let standard_path = super::standard_body_background_widget_path();
        let scene = resolve_canvas_graph_with_defaults(
            &host,
            Some("drak"),
            &|url| {
                if url == "file://./body_background_content.json" {
                    Ok(content.clone())
                } else if url == standard_path {
                    Ok(standard.clone())
                } else {
                    Err(format!("framework record unavailable in fixture: {url}"))
                }
            },
            None,
            None,
            &defaults,
        )
        .expect("resolve failed");

        let bg = scene
            .nodes
            .values()
            .find(|node| node.name == "background_Main")
            .expect("merged background node");
        assert_eq!(
            bg.raw.get("ImagePath").and_then(|v| v.as_str()),
            Some("UI/Textures/I_InteractiveScreens/MFD/DRAK/DRAK_GroundVehicle_Dashboard_background_2.tif"),
            "the standard's brand container is matched by the owning canvas's selected \
             brand identifier (s_drak_hud), not the first manufacturer-prefix hit (s_drak_env)"
        );
    }

    #[test]
    fn child_state_conditioned_entries_apply_after_parent_param_injection() {
        let state_tag = "11111111-1111-1111-1111-111111111111";
        let child = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.DeferredStyleChild",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "embeddedStyles": [
                    {
                        "_Type_": "BuildingBlocks_StyleEntry",
                        "name": "Critical - Text",
                        "conditionsList": [
                            {
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Text"},
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionParent",
                                     "conditions": [
                                        {"_Type_": "BuildingBlocks_StyleSelectorConditionAnyOfTag",
                                         "tags": [{"_RecordId_": state_tag}]}
                                     ]}
                                ]
                            }
                        ],
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierColor",
                             "field": "FillColor",
                             "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Background", "alpha": 1.0}}
                        ],
                        "transitions": []
                    }
                ],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "item", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetText",
                     "name": "label", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "text": "WPN"}
                ],
                "operations": [
                    {"_Pointer_": "ptr:9", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                     "name": "Value item", "parameter": "ParamInput0", "defaultValue": 0},
                    {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                     "input": "_PointsTo_:ptr:9", "defaultValue": null,
                     "values": [
                        {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 4,
                         "second": {"_RecordId_": state_tag, "_RecordName_": format!("Tag.{state_tag}")}}
                     ]},
                    {"_Type_": "BuildingBlocks_BindingsStringField",
                     "widget": "_PointsTo_:ptr:1", "field": "PrimaryStateTag",
                     "input": "_PointsTo_:ptr:12"}
                ]
            }
        });
        let host = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.DeferredStyleHost",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "host_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "slot", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "canvas": "file://./deferred_child.json"}
                ],
                "operations": [
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "binding": "/Test/Severity"},
                    {"_Type_": "BuildingBlocks_BindingsIntegerField",
                     "widget": "_PointsTo_:ptr:2", "field": "ParamInput0",
                     "input": "_PointsTo_:ptr:10"}
                ]
            }
        });

        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("/Test/Severity", crate::canvas::Value::Int(4));
        let scene = resolve_canvas_graph_with_defaults(
            &host,
            None,
            &|url| match url {
                "file://./deferred_child.json" => Ok(child.clone()),
                other => Err(format!("framework record unavailable in fixture: {other}")),
            },
            None,
            None,
            &defaults,
        )
        .expect("resolve failed");

        let item = scene
            .nodes
            .values()
            .find(|node| node.name == "item")
            .expect("merged child item");
        assert!(
            item.style_tag_uuids.iter().any(|t| t == state_tag),
            "the parent-injected severity resolves the item's state tag, got {:?}",
            item.style_tag_uuids
        );
        let label = scene
            .nodes
            .values()
            .find(|node| node.name == "label")
            .expect("merged child label");
        assert_eq!(
            label.raw.get("FillColorToken").and_then(|v| v.as_str()),
            Some("Background"),
            "the state-conditioned child entry applies after param injection"
        );
    }

    /// An entry referencing a PENDING state tag (the producing chain is
    /// unresolvable at the child's own cascade time — the severity param is
    /// parent-injected) must NOT apply prematurely with the tag assumed
    /// absent: the annunciator's "Show Glow in online state" NotTag-gates on
    /// the state tags and lit the OFFLINE chiclet's gradient. It is deferred
    /// and evaluated at the parent level where the state is known — here
    /// severity 4 produces the tag, so the NotTag entry must never apply.
    #[test]
    fn pending_state_entries_wait_for_their_state() {
        let state_tag = "22222222-2222-2222-2222-222222222222";
        let child = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PendingStateChild",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "embeddedStyles": [
                    {
                        "_Type_": "BuildingBlocks_StyleEntry",
                        "name": "Glow when no state",
                        "conditionsList": [
                            {
                                "_Type_": "BuildingBlocks_StyleConditionList",
                                "conditions": [
                                    {"_Type_": "BuildingBlocks_StyleSelectorConditionNotTag",
                                     "tag": {"_RecordId_": state_tag}}
                                ]
                            }
                        ],
                        "modifiers": [
                            {"_Type_": "BuildingBlocks_FieldModifierNumber",
                             "field": "Alpha", "value": 0.5}
                        ],
                        "transitions": []
                    }
                ],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "item", "isActive": true}
                ],
                "operations": [
                    {"_Pointer_": "ptr:9", "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                     "name": "Value item", "parameter": "ParamInput0", "defaultValue": 0},
                    {"_Pointer_": "ptr:12", "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                     "input": "_PointsTo_:ptr:9", "defaultValue": null,
                     "values": [
                        {"_Type_": "BuildingBlocks_IntegerTagPair", "first": 4,
                         "second": {"_RecordId_": state_tag, "_RecordName_": format!("Tag.{state_tag}")}}
                     ]},
                    {"_Type_": "BuildingBlocks_BindingsStringField",
                     "widget": "_PointsTo_:ptr:1", "field": "PrimaryStateTag",
                     "input": "_PointsTo_:ptr:12"}
                ]
            }
        });
        let host = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PendingStateHost",
            "_RecordValue_": {
                "size": {"x": 100.0, "y": 100.0},
                "defaultStyles": {"entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "host_root", "isActive": true},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetCanvas",
                     "name": "slot", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "canvas": "file://./pending_child.json"}
                ],
                "operations": [
                    {"_Pointer_": "ptr:10", "_Type_": "BuildingBlocks_BindingsIntegerVariable",
                     "binding": "/Test/PendingSeverity"},
                    {"_Type_": "BuildingBlocks_BindingsIntegerField",
                     "widget": "_PointsTo_:ptr:2", "field": "ParamInput0",
                     "input": "_PointsTo_:ptr:10"}
                ]
            }
        });

        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("/Test/PendingSeverity", crate::canvas::Value::Int(4));
        let scene = resolve_canvas_graph_with_defaults(
            &host,
            None,
            &|url| match url {
                "file://./pending_child.json" => Ok(child.clone()),
                other => Err(format!("framework record unavailable in fixture: {other}")),
            },
            None,
            None,
            &defaults,
        )
        .expect("resolve failed");

        let item = scene
            .nodes
            .values()
            .find(|node| node.name == "item")
            .expect("merged child item");
        assert!(
            item.style_tag_uuids.iter().any(|t| t == state_tag),
            "severity 4 resolves the state tag at the parent level"
        );
        assert_eq!(
            item.alpha, 1.0,
            "the NotTag(pending-state) entry must not have applied prematurely"
        );
    }

    #[test]
    fn pass1_style_ref_child_is_resolved_recursively() {
        // Phase B1 regression: Pass 1 canvas-reference children must themselves
        // be recursively resolved, not just shallowly parsed.
        //
        // Hierarchy:
        //   root  (1 root node, Pass 1 ref → child_canvas.json)
        //     └── child  (1 node, Pass 1 ref → grandchild_canvas.json)
        //           └── grandchild  (3 nodes, no refs)
        //
        // With the old shallow parse, grandchild nodes were never merged.
        // With the recursive fix, the scene must contain root + child + grandchild.
        let grandchild = serde_json::json!({
            "_RecordName_": "grandchild",
            "_RecordId_": "00000000-0000-0000-0000-0000000000cc",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"_Type_": "Vec3", "x": 100.0, "y": 100.0, "z": 0.0},
                "defaultStyles": {"_Type_": "BuildingBlocks_DefaultStyles", "sharedStyles": null, "entries": []},
                "brandStyles": [],
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "gc_root"},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetIcon", "name": "gc_icon1", "parent": "_PointsTo_:ptr:1"},
                    {"_Pointer_": "ptr:3", "_Type_": "BuildingBlocks_WidgetIcon", "name": "gc_icon2", "parent": "_PointsTo_:ptr:1"}
                ]
            }
        });
        let child = canvas_with_style_ref("child", "file://./grandchild.json");
        let root = canvas_with_style_ref("root", "file://./child.json");

        let fetch_count = std::cell::Cell::new(0u32);
        let scene = resolve_canvas_graph(&root, None, &|url| {
            Ok(match url {
                "file://./child.json" => {
                    fetch_count.set(fetch_count.get() + 1);
                    child.clone()
                }
                "file://./grandchild.json" => {
                    fetch_count.set(fetch_count.get() + 1);
                    grandchild.clone()
                }
                // Framework records (tag database / widget standards) probed by
                // the expansion pass are not canvas follows.
                _ => return Err(format!("framework record unavailable in fixture: {url}")),
            })
        })
        .expect("resolve failed");

        // Both child and grandchild must have been fetched.
        assert_eq!(fetch_count.get(), 2, "expected 2 fetches (child + grandchild), got {}", fetch_count.get());
        // root(1) + child(1) + grandchild(3) = 5 nodes
        assert_eq!(scene.nodes.len(), 5, "expected 5 merged nodes (root+child+grandchild), got {}", scene.nodes.len());
    }
}
