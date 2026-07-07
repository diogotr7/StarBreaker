//! Canvas style-entry projection for the pipeline: applies the root canvas's
//! own style entries onto the resolved scene after graph resolution (split
//! from `pipeline/mod.rs` for the line cap).
//!
//! NOTE (plan P4.1 inventory): this is the one place `defaultStyles.entries`
//! run at all (they are editor-time in the cascade) — scoped to the ROOT
//! (binding frame) canvas, followed by a re-application of the root brand
//! container. Flagged for the P4.4 re-audit.

use crate::bb_brand_style::{brand_class_for_canvas, resolve_brand_identity, BrandPolicy};
use crate::bb_style_engine::{apply, StyleSheet, Tier};

pub(super) fn project_canvas_style_entries(
    scene: &mut crate::bb_scene::BbScene,
    raw_root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    loc_fetcher: Option<&dyn crate::bb_loc::LocFetcher>,
) {
    let record_value = raw_root_json.get("_RecordValue_").unwrap_or(raw_root_json);
    // Resolve the root brand by IDENTITY + canvas family (hud/env), not the
    // legacy manufacturer-prefix scan (B1). No style link at this site.
    let canvas_name = raw_root_json
        .get("_RecordName_")
        .and_then(|v| v.as_str())
        .or_else(|| record_value.get("_RecordName_").and_then(|v| v.as_str()));
    let class = brand_class_for_canvas(canvas_name);
    let selected_brand =
        resolve_brand_identity(raw_root_json, None, manufacturer_id, class, BrandPolicy::Default);
    let palette_source = selected_brand.as_ref().map(|brand| brand.raw).unwrap_or(record_value);

    if let Some(default_entries) = record_value
        .get("defaultStyles")
        .and_then(|styles| styles.get("entries"))
        .and_then(|entries| entries.as_array())
    {
        apply(
            scene,
            &[StyleSheet::uniform(
                Tier::StyleLink,
                "root-defaultStyles",
                palette_source,
                default_entries.as_slice(),
            )],
            loc_fetcher,
        );
    }

    if let Some(brand) = selected_brand {
        apply(
            scene,
            &[StyleSheet::uniform(
                Tier::Brand,
                brand.identifier.clone(),
                brand.raw,
                brand.entries,
            )],
            loc_fetcher,
        );
    }
}
