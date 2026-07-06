//! `ui_variant_styles` MCP tool — the authored-vs-applied style drill.
//!
//! For a canvas + node query, lists each matched node's AUTHORED style entries
//! per tier (`defaultStyles`, the manufacturer-selected brand, canvas
//! `embeddedStyles` — including bare `Type(Text)` selectors) that MATCH the
//! node, marking each `applied` by its membership in the resolved
//! `__AppliedStyleEntries`. This surfaces an authored-but-UNAPPLIED font/colour
//! (the repeated parity blocker: a `defaultStyles` FontSize a brand supersedes
//! shows `applied:false`) in one call.
//!
//! Registration is a thin `#[tool]` shim in `tools.rs`; the logic lives here.
//! It reuses the cascade's own selector matchers
//! (`bb_brand_apply::{entry_matches_scene, entry_matches_text_format}`) so it
//! never re-implements Parent/Ancestor/text-format selector semantics.
//!
//! `applied` is decided by `__AppliedStyleEntries` membership, NOT by comparing
//! the authored value against the compiled-IR effective value: the MFD
//! host-stage scale multiplies design font sizes (~1.667x), so a numeric
//! FontSize compare would false-negative the exact case this drill exists to
//! diagnose. Membership is the cascade's own record of "did this entry's
//! modifiers run on this node".

use serde_json::{json, Value};
use starbreaker_ui::bb_brand_apply::{entry_matches_scene, entry_matches_text_format};
use starbreaker_ui::bb_brand_style::resolve_brand_style;

use crate::tools::{
    compact_colour_fields, summarize_conditions, summarize_modifiers, StarBreakerMcp,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UiVariantStylesRequest {
    #[schemars(description = "Canvas identifier to resolve: absolute JSON path, record GUID, full record name, or bare record name.")]
    pub canvas: String,
    #[schemars(description = "Case-insensitive substring matched against resolved node name, type, or text.")]
    pub query: String,
    #[schemars(description = "Optional manufacturer id used for brand style selection, e.g. drak, rsi, aegis.")]
    pub manufacturer: Option<String>,
    #[schemars(description = "Maximum number of matching nodes to return. Default 80.")]
    pub limit: Option<u32>,
}

/// Two entries are the same authored entry (for applied-membership) when they
/// share `name` + `modifiers` — the identity + effect, which the cascade does
/// NOT transform when it records the applied clone (condition tag names may be
/// resolved on the applied side, so `conditionsList` is deliberately excluded).
fn entries_equivalent(a: &Value, b: &Value) -> bool {
    a.get("name") == b.get("name") && a.get("modifiers") == b.get("modifiers")
}

pub(crate) fn ui_variant_styles_impl(server: &StarBreakerMcp, req: UiVariantStylesRequest) -> String {
    let manufacturer = req.manufacturer.as_deref();
    let (canvas, scene) = match server.resolve_scene_for_canvas(&req.canvas, manufacturer) {
        Ok(v) => v,
        Err(json) => return json,
    };
    let record_value = canvas.get("_RecordValue_").unwrap_or(&canvas);

    // Authored entries per tier, read off the fetched canvas exactly as the
    // production cascade (apply_canvas_style_cascade) and ui_canvas_style_inventory.
    let mut tiers: Vec<(String, Vec<Value>)> = Vec::new();
    if let Some(entries) = record_value
        .get("defaultStyles")
        .and_then(|v| v.get("entries"))
        .and_then(|v| v.as_array())
    {
        tiers.push(("defaultStyles".to_string(), entries.clone()));
    }
    if let Some(brand) = resolve_brand_style(&canvas, manufacturer, None) {
        tiers.push((format!("brand:{}", brand.identifier), brand.entries.to_vec()));
    }
    if let Some(entries) = record_value.get("embeddedStyles").and_then(|v| v.as_array()) {
        tiers.push(("embeddedStyles".to_string(), entries.clone()));
    }

    let query = req.query.to_ascii_lowercase();
    let limit = req.limit.unwrap_or(80).max(1) as usize;

    let mut nodes = Vec::new();
    for node in scene.nodes.values() {
        if nodes.len() >= limit {
            break;
        }
        let node_type = format!("{:?}", node.ty);
        let text = node.text.as_ref().map(|t| t.string.clone()).unwrap_or_default();
        let haystack = format!("{} {} {}", node.name, node_type, text).to_ascii_lowercase();
        if !haystack.contains(&query) {
            continue;
        }

        let applied_set = node
            .raw
            .get("__AppliedStyleEntries")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut authored_entries = Vec::new();
        for (tier, entries) in &tiers {
            for entry in entries {
                let node_id = node.id;
                let matched = entry_matches_scene(entry, node_id, node, &scene)
                    || entry_matches_text_format(entry, node_id, node, &scene);
                if !matched {
                    continue;
                }
                let applied = applied_set.iter().any(|a| entries_equivalent(a, entry));
                authored_entries.push(json!({
                    "tier": tier,
                    "name": entry.get("name"),
                    "selector": summarize_conditions(entry),
                    "fields": summarize_modifiers(entry),
                    "applied": applied,
                }));
            }
        }

        nodes.push(json!({
            "id": node.id,
            "name": node.name,
            "type": node_type,
            "is_active": node.is_active,
            "style_tag_uuids": node.style_tag_uuids,
            "text": text,
            "authored_entries": authored_entries,
            "effective": {
                "font_size": node.raw.get("FontSize"),
                "colour_fields": compact_colour_fields(&node.raw),
            },
        }));
    }

    serde_json::to_string_pretty(&json!({
        "schema_version": 1,
        "canvas": {
            "record_name": canvas.get("_RecordName_").and_then(|v| v.as_str()),
            "record_id": canvas.get("_RecordId_").and_then(|v| v.as_str()),
        },
        "manufacturer": manufacturer,
        "query": req.query,
        "matched_node_count": nodes.len(),
        "node_limit": limit,
        "nodes": nodes,
    }))
    .unwrap_or_else(|e| format!("JSON error: {e}"))
}
