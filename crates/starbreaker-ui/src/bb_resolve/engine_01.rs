#[allow(unused_imports)]
use super::*;
// Consolidated engine chunk 01 (formerly: part_01.part, part_02.part, part_03.part, pass1.part, part_04.part, part_05.part).
//   part_01.part: BuildingBlocks canvas graph resolver.
//   part_03.part: Inner recursive resolver.  `visited` accumulates normalised record names
//   pass1.part: Pass 1 of the recursive resolver: follow default-state canvas-reference
//   part_05.part: Node IDs at or above this value belong to the widget-standard expansion

// BuildingBlocks canvas graph resolver.
//
// Two-pass drill-down from a root `BuildingBlocks_Canvas` JSON record:
//
// **Pass 1** — `defaultStyles` / `brandStyles` canvas-reference modifiers.
// Selects the *default-state* entry from `defaultStyles.entries[]` (or a
// matching `brandStyles[]` entry) and fetches its
// `CanvasReferenceRecord`-typed modifier value.  The default-state entry is:
// 1. The first entry whose `conditionsList` is absent or empty
//    (unconditional — always active regardless of game state).
// 2. Failing that, the entry whose condition tag matches the scene node with
//    the highest style-tag count ("most-tagged node" heuristic — the primary
//    content slot carries more tags than sub-component slots).
// 3. Last resort: the first entry overall (used when no scene-node heuristic
//    applies, e.g. `MC_S_Target_Master` which has exactly one conditional
//    entry that is effectively the default).
//
// Selecting a single entry prevents runtime mode-switching canvases (e.g.
// `GunsMode`, `NavMode`, `TurretMode` on `MC_S_Self_Master`) from all being
// merged into the static render at once.  Each fetched child canvas is
// itself resolved recursively (up to `MAX_CANVAS_DEPTH` total levels), so
// deep hierarchies like
// `MC_S_Power_Master → GEN_MC_S_Power → gen_mc_s_powerlists → …` are fully
// expanded in one call.
//
// **Pass 2** — `WidgetCanvas.canvas` field.
// Some host canvases (e.g. `M_MFD_Screen`) have an empty `defaultStyles` and
// carry all their content via a `BuildingBlocks_WidgetCanvas` node whose
// `canvas` field is a `file://` URL pointing to the real content canvas.
// Pass 2 follows those references recursively so the merged scene includes
// the full content hierarchy.  Both passes share the same depth counter and
// a global `visited` path set to guard against cycles.
//
// **Mode-switch guard** — `conditional_canvas_norms`.
// All canvas URLs referenced by *any* conditional entry (regardless of which
// entry Pass 1 selected) are collected into `conditional_canvas_norms`.
// Pass 2 skips any URL in this set that was not visited by Pass 1, preventing
// WidgetCanvas nodes that serve as mode-switchable slots (e.g. `canvas_NavMode`
// pointing to `gen_mc_s_nav.json`) from being followed when a different mode
// was selected in Pass 1.

use std::collections::{HashMap, HashSet};

use crate::bb_loc::LocFetcher;
use crate::bb_scene::{BbNodeId, BbNodeType, BbScene, BbValue, parse_bb_canvas};
use crate::bb_brand_style;
use crate::record_name::extract_record_name;

fn is_linear_progress_meter(node: &crate::bb_scene::BbNode) -> bool {
    matches!(
        &node.ty,
        BbNodeType::Other(kind)
            if kind.eq_ignore_ascii_case("BuildingBlocks_WidgetLinearProgressMeter")
    )
}

/// Apply the modular-kit component sheets (`sk_<kit>_*styles`) for
/// `style_identifier` at [`Tier::StandardModule`]. Runs inside the resolve and
/// AGAIN from the pipeline after the root style projection: the projection
/// applies the weakest tiers (root `defaultStyles` + brand) sequentially last,
/// and the engine is last-writer-wins, so without the re-application a canvas
/// `defaultStyles` entry clobbers the kit chrome it should rank below (the
/// transit button's Filled-state corner geometry — ledger 106).
pub fn apply_modular_kit_sheets(
    scene: &mut BbScene,
    style_id: &str,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
) {
    // Named colour roles in modular-kit entries resolve against the brand
    // Style record's palette (the module records carry no `colorStyles`).
    let module_chrome_palette = fetch_by_path(style_id).ok();
    let module_paths = [
        modular_linearprogress_style_path(style_id),
        modular_buttonprimary_style_path(style_id),
        modular_buttonsecondary_style_path(style_id),
    ];

    for module_path in module_paths.into_iter().flatten() {
        match fetch_by_path(&module_path) {
            Ok(module_style_json) => {
                let module_style_value = module_style_json
                    .get("_RecordValue_")
                    .unwrap_or(&module_style_json);
                if let Some(entries) = module_style_value.get("entries").and_then(|v| v.as_array()) {
                    if module_path.contains("_linearprogressmeterstyles") {
                        seed_implicit_linearprogress_style_tags(
                            scene,
                            entries,
                            fetch_by_path,
                        );
                    }
                    if module_path.contains("_buttonsecondarystyles") {
                        apply_buttonsecondary_modular_styles(scene, entries);
                    }

                    let palette = module_chrome_palette
                        .as_ref()
                        .unwrap_or(module_style_value);
                    crate::bb_style_engine::apply(
                        scene,
                        &[crate::bb_style_engine::StyleSheet {
                            tier: crate::bb_style_engine::Tier::StandardModule,
                            identifier: extract_record_name(&module_path),
                            fills: module_style_value,
                            chrome: palette,
                            entries: entries.as_slice(),
                            scope: crate::bb_style_engine::SheetScope::Scene,
                        }],
                        loc_fetcher,
                    );
                }
            }
            Err(e) => {
                log::debug!(
                    "bb_resolve: no modular style '{}' for '{}': {}",
                    module_path,
                    style_id,
                    e
                );
            }
        }
    }

    // The scrollbar sheet applies scoped to expanded scrollbar standards
    // only — see `apply_scrollbar_modular_styles`.
    apply_scrollbar_modular_styles(
        scene,
        style_id,
        module_chrome_palette.as_ref(),
        fetch_by_path,
    );
}

/// The modular-kit style identifier for a canvas: its own `style` link, else
/// the resolved brand identifier (the same derivation the resolve uses).
pub fn modular_style_identifier(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
) -> Option<String> {
    let record_value = root_json.get("_RecordValue_").unwrap_or(root_json);
    record_value
        .get("style")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(crate::record_name::extract_record_name)
        .or_else(|| {
            // No style link: resolve the manufacturer brand by IDENTITY + canvas
            // family (hud/env) rather than the legacy prefix scan (B1).
            let canvas_name = root_json
                .get("_RecordName_")
                .and_then(|v| v.as_str())
                .or_else(|| record_value.get("_RecordName_").and_then(|v| v.as_str()));
            let class = bb_brand_style::brand_class_for_canvas(canvas_name);
            bb_brand_style::resolve_brand_identity(
                root_json,
                None,
                manufacturer_id,
                class,
                bb_brand_style::BrandPolicy::Default,
            )
            .map(|brand| brand.identifier)
        })
}

/// Framework record path of a modular-kit component style sheet: the style
/// link `S_<kit>` (or a direct `sk_<kit>`) maps to
/// `modularkitstyles/sk_<kit>/sk_<kit>_<component>styles.json`.
fn modular_kit_style_path(style_identifier: &str, component: &str) -> Option<String> {
    let normalized = style_identifier.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let module_id = if let Some(rest) = normalized.strip_prefix("s_") {
        format!("sk_{}", rest)
    } else if normalized.starts_with("sk_") {
        normalized
    } else {
        return None;
    };

    Some(format!(
        "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/styles/modularkitstyles/{0}/{0}_{component}styles.json",
        module_id
    ))
}

fn modular_linearprogress_style_path(style_identifier: &str) -> Option<String> {
    modular_kit_style_path(style_identifier, "linearprogressmeter")
}

fn modular_buttonsecondary_style_path(style_identifier: &str) -> Option<String> {
    modular_kit_style_path(style_identifier, "buttonsecondary")
}

fn modular_buttonprimary_style_path(style_identifier: &str) -> Option<String> {
    modular_kit_style_path(style_identifier, "buttonprimary")
}

pub(crate) fn modular_scrollbar_style_path(style_identifier: &str) -> Option<String> {
    modular_kit_style_path(style_identifier, "scrollbar")
}

pub(crate) fn standard_body_background_widget_path() -> String {
    "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/widgets/bodybackgroundwidgetstandard.json".to_string()
}

fn body_background_uses_texture(node: &crate::bb_scene::BbNode) -> bool {
    if !matches!(node.ty, BbNodeType::WidgetBodyBackground) {
        return false;
    }

    background_type_is_texture(node.raw.get("backgroundType"))
}

fn background_type_is_texture(background_type: Option<&serde_json::Value>) -> bool {
    match background_type {
        Some(serde_json::Value::String(value)) => value.eq_ignore_ascii_case("Texture"),
        Some(serde_json::Value::Number(value)) => value.as_i64() == Some(1),
        _ => false,
    }
}

/// Whether THIS record's authored `scene[]` contains a texture-mode body
/// background widget. The standard's styles bind to the widget at its defining
/// canvas with that canvas's brand context (`M_Eng_MFDContent` selects
/// `s_drak_hud`); outer levels see the node only via merge and must not
/// re-apply the standard with their own (often absent) brand context — the
/// manufacturer-prefix fallback would hit `s_drak_env` first and overwrite.
fn record_authors_texture_body_background(record_value: &serde_json::Value) -> bool {
    record_value
        .get("scene")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|node| {
            node.get("_Type_").and_then(|v| v.as_str())
                == Some("BuildingBlocks_WidgetBodyBackground")
                && background_type_is_texture(node.get("backgroundType"))
        })
}

fn extract_body_background_texture_tag_id(standard_record: &serde_json::Value) -> Option<String> {
    let record_value = standard_record
        .get("_RecordValue_")
        .unwrap_or(standard_record);
    record_value
        .get("operations")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .find_map(|operation| {
            let ty = operation.get("_Type_").and_then(|v| v.as_str())?;
            if !ty.eq_ignore_ascii_case("BuildingBlocks_BindingsTagFromIntegerSwitch") {
                return None;
            }
            operation
                .get("values")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .find_map(|pair| {
                    if pair.get("first").and_then(|v| v.as_i64()) != Some(1) {
                        return None;
                    }
                    pair.get("second")
                        .and_then(|tag| tag.get("_RecordId_"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
        })
}

fn seed_body_background_texture_tags(scene: &mut BbScene, tag_id: &str) {
    for node in scene.nodes.values_mut() {
        if body_background_uses_texture(node)
            && !node.style_tag_uuids.iter().any(|id| id == tag_id)
        {
            node.style_tag_uuids.push(tag_id.to_string());
        }
    }
}

fn apply_body_background_standard_styles(
    scene: &mut BbScene,
    standard_record: &serde_json::Value,
    manufacturer_id: Option<&str>,
    preferred_brand: Option<&str>,
    brand_class: bb_brand_style::BrandClass,
    loc_fetcher: Option<&dyn LocFetcher>,
) {
    let Some(texture_tag_id) = extract_body_background_texture_tag_id(standard_record) else {
        return;
    };
    seed_body_background_texture_tags(scene, &texture_tag_id);
    if !scene
        .nodes
        .values()
        .any(|node| body_background_uses_texture(node))
    {
        return;
    }

    let record_value = standard_record
        .get("_RecordValue_")
        .unwrap_or(standard_record);
    if let Some(brand_style) = bb_brand_style::resolve_brand_identity(
        standard_record,
        preferred_brand,
        manufacturer_id,
        brand_class,
        bb_brand_style::BrandPolicy::Default,
    ) {
        crate::bb_style_engine::apply(
            scene,
            &[crate::bb_style_engine::StyleSheet::uniform(
                crate::bb_style_engine::Tier::Brand,
                brand_style.identifier.clone(),
                brand_style.raw,
                brand_style.entries,
            )],
            loc_fetcher,
        );
    } else if let Some(entries) = record_value
        .get("defaultStyles")
        .and_then(|styles| styles.get("entries"))
        .and_then(|entries| entries.as_array())
        .filter(|entries| !entries.is_empty())
    {
        crate::bb_style_engine::apply(
            scene,
            &[crate::bb_style_engine::StyleSheet::uniform(
                crate::bb_style_engine::Tier::StandardModule,
                "BodyBackgroundWidgetStandard.defaultStyles",
                record_value,
                entries.as_slice(),
            )],
            loc_fetcher,
        );
    }
}

// P4.4 audit (2026-06-13, plan): this name-pluck (find the entry literally
// named "RootGhost", copy its uniform corner radius onto ghost
// `ComponentGeneralButtonSecondary` nodes) SURVIVES the selector-engine
// unification. disable->adjudicate (env P44_DISABLE_ROOTGHOST, fresh export):
// NO frozen pin depends on it — drak's RootGhost has no radius modifiers
// (only EnableBackground:false) and the bioc medical targets contain no ghost
// buttonsecondary nodes. The modifier kernel now DOES handle Border*Radius
// (modifiers_number.rs `set_raw_corner_radius`), so the buttonsecondary sheet
// the engine already runs right after this call would apply RootGhost's radii
// generically IFF the entry's conditions (Tag 21788313 + Parent(AnyOfTag))
// match the expanded ghost-button node. That match is UNVERIFIED: no frozen
// target exercises a ghost button, and the brands whose RootGhost carries
// radii (aegs/bioc/crus/orig/crlf) are not in the frozen set. Per
// crates/starbreaker-ui/docs/ui-workflow.md §5 (a clean disable->adjudicate proves "no frozen pin",
// not "correct everywhere"), the pluck is KEPT. DELETION CRITERION: a
// reference for any ghost-button screen confirming the engine's
// condition-matched application reproduces the radius — then this and
// `apply_buttonsecondary_modular_styles` become redundant.
fn extract_rootghost_button_secondary_corner_radius(style_entries: &[serde_json::Value]) -> Option<f32> {
    let root_ghost_entry = style_entries.iter().find(|entry| {
        entry
            .get("name")
            .and_then(|v| v.as_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("RootGhost"))
    })?;

    let modifiers = root_ghost_entry.get("modifiers").and_then(|v| v.as_array())?;
    let radius_fields = [
        "BorderTopLeftRadius",
        "BorderTopRightRadius",
        "BorderBottomLeftRadius",
        "BorderBottomRightRadius",
    ];

    let mut radii = Vec::with_capacity(radius_fields.len());
    for field_name in radius_fields {
        let value = modifiers
            .iter()
            .find(|modifier| {
                modifier
                    .get("field")
                    .and_then(|field| field.as_str())
                    .is_some_and(|field| field.eq_ignore_ascii_case(field_name))
            })
            .and_then(|modifier| modifier.get("value"))
            .and_then(|value| value.as_f64())? as f32;
        radii.push(value);
    }

    let first = *radii.first()?;
    if first <= 0.0 || radii.iter().any(|radius| (*radius - first).abs() > f32::EPSILON) {
        return None;
    }

    Some(first)
}

fn set_uniform_border_radius(node: &mut crate::bb_scene::BbNode, radius: f32) {
    let Some(raw_obj) = node.raw.as_object_mut() else {
        return;
    };
    let border = raw_obj
        .entry("border".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(border_obj) = border.as_object_mut() else {
        return;
    };

    for corner in ["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"] {
        let corner_value = border_obj
            .entry(corner.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let Some(corner_obj) = corner_value.as_object_mut() else {
            continue;
        };

        let radius_value = corner_obj
            .entry("radius".to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let Some(radius_obj) = radius_value.as_object_mut() else {
            continue;
        };

        radius_obj.insert(
            "value".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(radius as f64).unwrap()),
        );
        radius_obj.insert(
            "behavior".to_string(),
            serde_json::Value::String("Fixed".to_string()),
        );
    }
}

fn apply_buttonsecondary_modular_styles(
    scene: &mut BbScene,
    style_entries: &[serde_json::Value],
) {
    let Some(radius) = extract_rootghost_button_secondary_corner_radius(style_entries) else {
        return;
    };

    for node in scene.nodes.values_mut() {
        if !matches!(node.ty, BbNodeType::ComponentGeneralButtonSecondary) {
            continue;
        }
        let is_ghost = node
            .raw
            .get("fillStyle")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("Ghost"));
        if !is_ghost {
            continue;
        }
        set_uniform_border_radius(node, radius);
    }
}

fn collect_style_condition_tags(
    condition: &serde_json::Value,
    out: &mut Vec<(String, Option<String>, Option<String>)>,
) {
    if let Some(tag) = condition.get("tag") {
        let tag_id = tag
            .get("_RecordId_")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| {
                tag.as_str().and_then(|s| {
                    s.strip_prefix("Tag.")
                        .map(str::to_owned)
                        .or_else(|| Some(s.to_owned()))
                })
            });

        if let Some(tag_id) = tag_id {
            let tag_record_name = tag
                .get("_RecordName_")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .or_else(|| Some(format!("Tag.{tag_id}")));
            let tag_record_path = tag
                .get("_RecordPath_")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            out.push((tag_id, tag_record_name, tag_record_path));
        }
    }

    if let Some(conditions) = condition.get("conditions").and_then(|v| v.as_array()) {
        for nested in conditions {
            collect_style_condition_tags(nested, out);
        }
    }
    if let Some(break_conditions) = condition.get("breakConditions").and_then(|v| v.as_array()) {
        for nested in break_conditions {
            collect_style_condition_tags(nested, out);
        }
    }
}

fn find_tag_name_in_tree(tags: &[serde_json::Value], tag_id: &str) -> Option<String> {
    for tag in tags {
        if tag
            .get("_RecordId_")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id.eq_ignore_ascii_case(tag_id))
        {
            return tag
                .get("tagName")
                .and_then(|v| v.as_str())
                .map(|name| name.to_ascii_lowercase());
        }

        if let Some(children) = tag.get("children").and_then(|v| v.as_array()) {
            if let Some(name) = find_tag_name_in_tree(children, tag_id) {
                return Some(name);
            }
        }
    }
    None
}

fn resolve_tag_name(
    tag_id: &str,
    tag_record_name: Option<&str>,
    tag_record_path: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Option<String> {
    if let Some(record_name) = tag_record_name {
        if let Ok(tag_record) = fetch_by_path(record_name) {
            if let Some(tag_name) = tag_record
                .get("_RecordValue_")
                .and_then(|v| v.get("tagName"))
                .and_then(|v| v.as_str())
            {
                return Some(tag_name.to_ascii_lowercase());
            }
        }
    }

    if let Some(record_path) = tag_record_path {
        if let Ok(tag_database_record) = fetch_by_path(record_path) {
            if let Some(tags) = tag_database_record
                .get("_RecordValue_")
                .and_then(|v| v.get("tags"))
                .and_then(|v| v.as_array())
            {
                return find_tag_name_in_tree(tags, tag_id);
            }
        }
    }

    None
}

fn seed_implicit_linearprogress_style_tags(
    scene: &mut BbScene,
    style_entries: &[serde_json::Value],
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) {
    let mut implicit_tags: Vec<(String, String)> = Vec::new();

    for entry in style_entries {
        let Some(condition_lists) = entry.get("conditionsList").and_then(|v| v.as_array()) else {
            continue;
        };
        for list in condition_lists {
            let Some(conditions) = list.get("conditions").and_then(|v| v.as_array()) else {
                continue;
            };
            for condition in conditions {
                let mut found = Vec::new();
                collect_style_condition_tags(condition, &mut found);
                for (tag_id, tag_record_name, tag_record_path) in found {
                    let tag_name = resolve_tag_name(
                        &tag_id,
                        tag_record_name.as_deref(),
                        tag_record_path.as_deref(),
                        fetch_by_path,
                    );
                    if let Some(tag_name) = tag_name {
                        implicit_tags.push((tag_id.clone(), tag_name));
                    }
                }
            }
        }
    }

    for (tag_id, tag_name) in implicit_tags {
        match tag_name.as_str() {
            "meter-element-instance" => {
                for node in scene.nodes.values_mut() {
                    if is_linear_progress_meter(node)
                        && !node.style_tag_uuids.iter().any(|id| id == &tag_id)
                    {
                        node.style_tag_uuids.push(tag_id.clone());
                    }
                }
            }
            "progress-meter-state-active" => {
                for node in scene.nodes.values_mut() {
                    let is_active_progress = is_linear_progress_meter(node)
                        && node
                            .raw
                            .get("progress")
                            .and_then(|v| v.as_f64())
                            .map(|v| v > 0.0)
                            .unwrap_or(false);
                    if is_active_progress && !node.style_tag_uuids.iter().any(|id| id == &tag_id)
                    {
                        node.style_tag_uuids.push(tag_id.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

/// Maximum total nesting depth across both passes.
///
/// The real MFD hierarchy has at least four levels:
/// `M_MFD_Screen → MC_S_Power_Master → GEN_MC_S_Power → gen_mc_s_powerlists`.
/// A cap of 8 provides ample headroom while still preventing runaway recursion.
pub(crate) const MAX_CANVAS_DEPTH: u8 = 8;

/// Parse `root_json`, recursively resolve all child canvases, and return a
/// fully-merged [`BbScene`].
///
/// `manufacturer_id` selects a matching `brandStyles[]` entry (e.g. `"drak"`);
/// when no brand matches, `defaultStyles.entries[]` are used at every level of
/// the hierarchy.  Individual child fetch or parse failures are logged and
/// skipped so a partial scene is still returned.
///
/// This is a backwards-compatible wrapper around [`resolve_canvas_graph_with_loc`]
/// that passes `None` for the localization fetcher.
pub fn resolve_canvas_graph(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Result<BbScene, String> {
    resolve_canvas_graph_with_loc(root_json, manufacturer_id, fetch_by_path, None)
}

/// Like [`resolve_canvas_graph`] but accepts an optional localization fetcher.
///
/// When `loc_fetcher` is `Some`, brand-applied string modifier values that start
/// with `@` are resolved through the fetcher before being written to nodes.
pub fn resolve_canvas_graph_with_loc(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
) -> Result<BbScene, String> {
    resolve_canvas_graph_with_loc_and_bound_view(
        root_json,
        manufacturer_id,
        fetch_by_path,
        loc_fetcher,
        None,
    )
}

/// Like [`resolve_canvas_graph_with_loc`] but also accepts the binding's bound
/// content-canvas `_RecordName_` (e.g. `"BuildingBlocks_Canvas.MC_S_Target_Master"`).
///
/// When set, an MFD frame's mutually-exclusive content-view slots are
/// instantiated by the bound view rather than by the frame's arbitrary
/// static-default boolean state (which otherwise drops the bound view's content
/// during Pass 2). See [`crate::mfd_view`].
pub fn resolve_canvas_graph_with_loc_and_bound_view(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
    bound_view_canvas: Option<&str>,
) -> Result<BbScene, String> {
    resolve_canvas_graph_with_defaults(
        root_json,
        manufacturer_id,
        fetch_by_path,
        loc_fetcher,
        bound_view_canvas,
        &crate::defaults::DefaultValueRegistry::default(),
    )
}

/// Like [`resolve_canvas_graph_with_loc_and_bound_view`] but also receives the
/// binding's default-value registry, used for data-driven resolution such as
/// list-slot materialisation (e.g. the power screen's `piplist` instances).
pub fn resolve_canvas_graph_with_defaults(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
    bound_view_canvas: Option<&str>,
    defaults: &crate::defaults::DefaultValueRegistry,
) -> Result<BbScene, String> {
    resolve_canvas_graph_with_defaults_and_host_stage(
        root_json,
        manufacturer_id,
        fetch_by_path,
        loc_fetcher,
        bound_view_canvas,
        defaults,
        None,
    )
}

/// Like [`resolve_canvas_graph_with_defaults`] but also receives the binding's
/// host SWF stage size (from the movie header), which sizes the MFD frame's
/// bound content-view slot (see [`crate::mfd_view`], plan P5.4).
pub fn resolve_canvas_graph_with_defaults_and_host_stage(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
    bound_view_canvas: Option<&str>,
    defaults: &crate::defaults::DefaultValueRegistry,
    host_stage_size: Option<(f32, f32)>,
) -> Result<BbScene, String> {
    let mut visited = HashSet::new();
    // Seed visited with the root's own record name so cycles back to the root
    // are caught without needing a separate check.
    if let Some(name) = root_json.get("_RecordName_").and_then(|v| v.as_str()) {
        visited.insert(name.to_ascii_lowercase());
    }
    let mut scene = resolve_canvas_graph_inner(
        root_json,
        manufacturer_id,
        fetch_by_path,
        loc_fetcher,
        0,
        &mut visited,
        None,
        None,
        None,
        &HashMap::new(),
        bound_view_canvas,
        defaults,
        None,
        host_stage_size,
    )?;
    // Bound geometry resolves against the full merged operation set (the pip
    // sizing's component-parameter chain crosses canvas levels), so apply it
    // once over the final scene.
    crate::bb_bindings::resolve_geometry_fields_into_scene(&mut scene, defaults);
    // Registry-backed direct visibility gates (the power columns' off panels).
    apply_registry_direct_gates(&mut scene, defaults);
    // Bound text values, resolved once for layout-time intrinsic sizing
    // (Auto-sized flex text like the OUTPUT card's "2" / "/ 16" pair).
    resolve_text_values_into_scene(&mut scene, defaults);
    Ok(scene)
}

/// Resolve each active text field's bound value into `raw["_ResolvedText_"]`
/// so the layout engine can measure Auto-sized flex children (it has no
/// binding access of its own).
fn resolve_text_values_into_scene(
    scene: &mut BbScene,
    defaults: &crate::defaults::DefaultValueRegistry,
) {
    let resolver = crate::bb_bindings::BindingResolver::from_operations(&scene.operations);
    let ids: Vec<BbNodeId> = scene
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.is_active && matches!(node.ty, crate::bb_scene::BbNodeType::WidgetTextField)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        let Some(node) = scene.nodes.get(&id) else { continue };
        let text = resolver.resolve_text_detailed(id, &node.raw, defaults).text;
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.starts_with('@') {
            continue;
        }
        let text = text.clone();
        if let Some(node) = scene.nodes.get_mut(&id)
            && let Some(map) = node.raw.as_object_mut()
        {
            map.insert("_ResolvedText_".to_string(), serde_json::json!(text));
        }
    }
}

/// Apply the external *shared* styles referenced by `defaultStyles.sharedStyles`
/// (e.g. the footer's `mfd_g_header.json`), if any. These carry chrome the
/// canvas's own styles don't — the screen-name segment-box background + its
/// top-border line, separator strokes, and selected/unselected name colours.
/// Only MFD frame/content canvases reference `sharedStyles`; standalone screens
/// leave it null, so this does not touch other gold-standard targets.
fn apply_shared_styles(
    scene: &mut BbScene,
    record_value: &serde_json::Value,
    fills_palette: &serde_json::Value,
    chrome_palette: &serde_json::Value,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
) {
    let Some(shared_url) = record_value
        .get("defaultStyles")
        .and_then(|ds| ds.get("sharedStyles"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "null")
    else {
        return;
    };
    match fetch_by_path(shared_url) {
        Ok(shared_json) => {
            let shared_value = shared_json.get("_RecordValue_").unwrap_or(&shared_json);
            if let Some(entries) = shared_value.get("entries").and_then(|v| v.as_array()) {
                // The shared style's own `colorStyles` are null placeholders; its
                // `ColorStyle` roles resolve against the canvas's effective brand
                // palette. Chrome (Background*/Border*) roles may resolve via the
                // fetched brand Style record; fill roles keep the container-only
                // behaviour (see `PaletteSources`).
                crate::bb_style_engine::apply(
                    scene,
                    &[crate::bb_style_engine::StyleSheet {
                        tier: crate::bb_style_engine::Tier::Shared,
                        identifier: extract_record_name(shared_url),
                        fills: fills_palette,
                        chrome: chrome_palette,
                        entries: entries.as_slice(),
                        scope: crate::bb_style_engine::SheetScope::Scene,
                    }],
                    loc_fetcher,
                );
            }
        }
        Err(e) => log::warn!("bb_resolve: failed to load sharedStyles {shared_url}: {e}"),
    }
}

/// Collect the child canvas's embedded style entries that reference any of
/// the child's PENDING state tags (see [`crate::bb_bindings::pending_state_tags`]) —
/// the entries the child's own cascade SKIPPED because their gating state
/// was unresolvable there (the annunciator chiclets' severity →
/// "Moderate - Text"/"Off - Text"/"Show Glow in online state"). They apply
/// once, at the parent level, where the parent-injected params resolve the
/// state. Entries whose tags resolved at child time (the power pips' state
/// entries) applied there and must NOT re-run — a blanket re-application
/// regressed the power footer and icons. `embeddedStyles` is authored as
/// either a bare entry array or an `{ entries: [...] }` container.
fn collect_late_state_style_entries(
    canvas_json: &serde_json::Value,
    child_pending: &std::collections::HashSet<String>,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Vec<(
    Vec<serde_json::Value>,
    serde_json::Value,
    serde_json::Value,
    String,
    crate::bb_style_engine::Tier,
)> {
    if child_pending.is_empty() {
        return Vec::new();
    }
    let record_value = canvas_json.get("_RecordValue_").unwrap_or(canvas_json);
    let filter = |container: &[serde_json::Value]| -> Vec<serde_json::Value> {
        container
            .iter()
            .filter(|entry| entry_references_any_tag(entry, child_pending))
            .cloned()
            .collect()
    };
    // The SAME runtime containers (and palettes) the child's cascade
    // pending-filtered: the SELECTED brand container (the power list item's
    // drak `PipBox_Fill_*` state entries — chrome fields resolve against the
    // fetched brand Style record, exactly like apply_brand_modifiers_with_-
    // palette) and the canvas's embeddedStyles (the annunciator chiclet
    // state entries; uniform record-value palette). defaultStyles are
    // editor-time defaults — see apply_canvas_style_cascade.
    // Each pass carries the identifier of its ORIGIN container (the brand
    // container's `s_*` identifier vs the literal "embeddedStyles"), so the
    // deferred re-application keeps container-class semantics (the textfield
    // text-format route runs only for brand containers — the power card's
    // state-deferred `Battery Powered/Depleted Text` FontSizes).
    let mut passes: Vec<(
        Vec<serde_json::Value>,
        serde_json::Value,
        serde_json::Value,
        String,
        crate::bb_style_engine::Tier,
    )> = Vec::new();
    // Resolve the selected brand container by IDENTITY + canvas family (hud/env),
    // not the legacy manufacturer-prefix scan (B1). No style link at this site.
    let canvas_name = canvas_json
        .get("_RecordName_")
        .and_then(|v| v.as_str())
        .or_else(|| record_value.get("_RecordName_").and_then(|v| v.as_str()));
    let class = bb_brand_style::brand_class_for_canvas(canvas_name);
    if let Some(brand) = bb_brand_style::resolve_brand_identity(
        canvas_json,
        None,
        manufacturer_id,
        class,
        bb_brand_style::BrandPolicy::Default,
    ) {
        let brand_entries = filter(brand.entries);
        if !brand_entries.is_empty() {
            let chrome = brand_palette_record(Some(&brand), fetch_by_path)
                .unwrap_or_else(|| brand.raw.clone());
            passes.push((
                brand_entries,
                brand.raw.clone(),
                chrome,
                brand.identifier.clone(),
                crate::bb_style_engine::Tier::Brand,
            ));
        }
    }
    if let Some(embedded) = record_value.get("embeddedStyles") {
        let embedded_entries = embedded
            .as_array()
            .or_else(|| embedded.get("entries").and_then(|v| v.as_array()));
        if let Some(embedded_entries) = embedded_entries {
            let filtered = filter(embedded_entries);
            if !filtered.is_empty() {
                passes.push((
                    filtered,
                    record_value.clone(),
                    record_value.clone(),
                    "embeddedStyles".to_string(),
                    crate::bb_style_engine::Tier::Embedded,
                ));
            }
        }
    }
    passes
}

/// Recursively collect tag `_RecordId_`s referenced by an entry's conditions
/// (Tag / NotTag / AnyOfTag / AllOfTag and anything nested under Parent /
/// Ancestor / AllOf / AnyOf / Not, including Ancestor `breakConditions`).
/// A `NotTag` is as much a reference as a positive tag: an entry gated on
/// `NotTag(pending-state)` cannot be evaluated before the state resolves.
fn collect_condition_tag_ids(value: Option<&serde_json::Value>, out: &mut Vec<String>) {
    let Some(value) = value else { return };
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_condition_tag_ids(Some(item), out);
            }
        }
        serde_json::Value::Object(map) => {
            let cond_type = map.get("_Type_").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(uuid) = map
                .get("tag")
                .and_then(|tag| tag.get("_RecordId_"))
                .and_then(|v| v.as_str())
            {
                out.push(uuid.to_ascii_lowercase());
            }
            // `AnyOfTag`/`AllOfTag` list items are BARE tag refs (no `tag`
            // wrapper): `{ _RecordPath_, _RecordName_, _RecordId_ }`.
            if cond_type.is_empty()
                && let Some(uuid) = map.get("_RecordId_").and_then(|v| v.as_str())
            {
                out.push(uuid.to_ascii_lowercase());
            }
            for key in ["conditions", "conditionsList", "tags", "breakConditions"] {
                collect_condition_tag_ids(map.get(key), out);
            }
        }
        _ => {}
    }
}

/// Whether a style entry's conditions reference ANY tag in `tags`
/// (positively or via `NotTag`).
fn entry_references_any_tag(
    entry: &serde_json::Value,
    tags: &std::collections::HashSet<String>,
) -> bool {
    if tags.is_empty() {
        return false;
    }
    let mut refs: Vec<String> = Vec::new();
    collect_condition_tag_ids(entry.get("conditionsList"), &mut refs);
    refs.iter().any(|uuid| tags.contains(uuid))
}

/// The `BuildingBlocks_Style` record carrying the brand's colour palette.
///
/// A canvas's `brandStyles[]` container carries only `entries`; named colour
/// roles resolve against the Style record its `brandIdentifier` names (e.g.
/// `s_drak_hud` → `colorStyles`). Fetch that record so colour modifiers are not
/// dropped for palette-less containers. `None` when the container already
/// carries a palette or the record is unavailable.
fn brand_palette_record(
    brand_style: Option<&bb_brand_style::BrandStyle<'_>>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> Option<serde_json::Value> {
    let brand = brand_style?;
    if brand.raw.get("colorStyles").is_some() {
        return None;
    }
    let identifier_url = brand.raw.get("brandIdentifier").and_then(|v| v.as_str())?;
    match fetch_by_path(identifier_url) {
        Ok(record) => Some(record),
        Err(e) => {
            log::debug!(
                "bb_resolve: brand palette record '{}' not fetchable: {e}",
                identifier_url
            );
            None
        }
    }
}

/// Apply the canvas's style cascade to the resolved scene.
///
/// Priority (lowest first): the canvas-level `style` record link, then
/// sharedStyles, then the selected brand's `brandStyles[]` entries (or, when no
/// brand matches the ship manufacturer, the canvas's `defaultStyles.entries` as
/// the brand-tier fallback), then the canvas's own embeddedStyles. Verified
/// against the medical (s_bioc style link under shared), MFD footer (s_drak_hud
/// brand over the generic shared `ScreenNameBackground`) and DRAK velocity-num
/// SCREEN (defaultStyles fallback — no drak brand declared) references.
#[allow(clippy::too_many_arguments)]
fn apply_canvas_style_cascade(
    scene: &mut BbScene,
    root_json: &serde_json::Value,
    record_value: &serde_json::Value,
    manufacturer_id: Option<&str>,
    preferred_brand: Option<&str>,
    local_style_value: Option<&serde_json::Value>,
    palette_source: Option<&serde_json::Value>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
    pending_state_tags: &std::collections::HashSet<String>,
) {
    // Entries gated on a PENDING state tag (unresolvable producing chain —
    // a parent-injected param) are skipped here: their truth is unknown and
    // evaluating them with the tag assumed absent mis-styles state-driven
    // elements (the offline chiclet's glow). They re-apply via the parent's
    // deferred late-state pass once the state resolves.
    let filter_pending = |entries: &[serde_json::Value]| -> Vec<serde_json::Value> {
        entries
            .iter()
            .filter(|entry| !entry_references_any_tag(entry, pending_state_tags))
            .cloned()
            .collect()
    };
    let brand_style = bb_brand_style::resolve_brand_style(root_json, manufacturer_id, preferred_brand);
    let brand_palette_record = brand_palette_record(brand_style.as_ref(), fetch_by_path);
    if brand_style.is_none()
        && let Some(style_value) = local_style_value
        && let Some(entries) = style_value.get("entries").and_then(|v| v.as_array())
    {
        let identifier = record_value
            .get("style")
            .and_then(|v| v.as_str())
            .map(crate::record_name::extract_record_name)
            .unwrap_or_else(|| "linked_style".to_string());
        let filtered = filter_pending(entries);
        crate::bb_style_engine::apply(
            scene,
            &[crate::bb_style_engine::StyleSheet::uniform(
                crate::bb_style_engine::Tier::StyleLink,
                identifier,
                style_value,
                filtered.as_slice(),
            )],
            loc_fetcher,
        );
    }
    // Shared ColorStyle roles (Bright/Disabled) resolve against the full brand palette, not the canvas's narrower one.
    let shared_fills = brand_style
        .as_ref()
        .map(|b| b.raw)
        .unwrap_or(palette_source.unwrap_or(record_value));
    let shared_chrome = brand_palette_record.as_ref().unwrap_or(shared_fills);
    // `defaultStyles.entries` are EDITOR-TIME defaults whenever a brand
    // RESOLVES — the brand supersedes them. The evidence: the annunciator's
    // `CornerRadius` (radius 30) lives ONLY there and the in-game chiclet
    // frames are square; the power `System Icon Color` exists in defaultStyles
    // + misc/orig brand containers but NOT drak's, and the in-game drak system
    // icons render the SVGs' own white. Both of those canvases DO resolve a
    // matching brand, so defaultStyles stay editor-time for them. ONLY when no
    // brand matches the ship manufacturer do `defaultStyles.entries` become the
    // runtime brand-tier look (the no-match fallback applied below — the drak
    // velocity-num SCREEN readouts). Runtime styling = sharedStyles < brand
    // (or, no-brand-match, defaultStyles) < embedded < inline.
    apply_shared_styles(scene, record_value, shared_fills, shared_chrome, fetch_by_path, loc_fetcher);
    if let Some(brand_style) = brand_style.as_ref() {
        let palette = brand_palette_record.as_ref().unwrap_or(brand_style.raw);
        let filtered = filter_pending(brand_style.entries);
        crate::bb_style_engine::apply(
            scene,
            &[crate::bb_style_engine::StyleSheet {
                tier: crate::bb_style_engine::Tier::Brand,
                identifier: brand_style.identifier.clone(),
                fills: brand_style.raw,
                chrome: palette,
                entries: filtered.as_slice(),
                scope: crate::bb_style_engine::SheetScope::Scene,
            }],
            loc_fetcher,
        );
    } else if let Some(entries) = record_value
        .get("defaultStyles")
        .and_then(|d| d.get("entries"))
        .and_then(|v| v.as_array())
        .filter(|e| !e.is_empty())
    {
        // No `brandStyles[]` entry matches the ship manufacturer → fall back to
        // `defaultStyles.entries`, the canvas's brand-agnostic look (the Pass-1
        // canvas-reference rule above: "when no brand matches, defaultStyles are
        // used at every level"). The DRAK velocity-num SCREEN variant declares
        // only an `s_grey_hud` brand (grey-HUD ships); a drak ship matches none,
        // so its readout sizing (FontSize 420/500 + white StrokeColor) comes
        // from defaultStyles — applied at the BRAND tier so the textfield
        // text-format route reaches the `Type(Text)+Parent[…]` readout entries.
        // This does NOT resurrect the editor-time-default counterexamples (the
        // annunciator's `CornerRadius` and power's `System Icon Color`): those
        // canvases DO resolve a matching brand, so this fallback never fires for
        // them. The brand-agnostic palette is the canvas's own.
        let palette = palette_source.unwrap_or(record_value);
        let filtered = filter_pending(entries);
        crate::bb_style_engine::apply(
            scene,
            &[crate::bb_style_engine::StyleSheet::uniform(
                crate::bb_style_engine::Tier::Brand,
                "defaultStyles",
                palette,
                filtered.as_slice(),
            )],
            loc_fetcher,
        );
    }
    if let Some(entries) = record_value.get("embeddedStyles").and_then(|v| v.as_array()) {
        let palette_source = palette_source.unwrap_or(record_value);
        let filtered = filter_pending(entries);
        crate::bb_style_engine::apply(
            scene,
            &[crate::bb_style_engine::StyleSheet::uniform(
                crate::bb_style_engine::Tier::Embedded,
                "embeddedStyles",
                palette_source,
                filtered.as_slice(),
            )],
            loc_fetcher,
        );
    }
    // Node-authored `inlineStyles` are the final cascade stage. Every entry
    // pass above already re-applies them last, but a canvas with no brand /
    // shared / embedded entries would otherwise never run a pass at all —
    // an empty-entry pass guarantees they apply.
    crate::bb_style_engine::apply(
        scene,
        &[crate::bb_style_engine::StyleSheet::uniform(
            crate::bb_style_engine::Tier::Inline,
            "inline",
            shared_fills,
            &[],
        )],
        loc_fetcher,
    );
}

/// Inner recursive resolver.  `visited` accumulates normalised record names
/// (lower-cased basenames extracted from `file://` URLs or bare names) seen so
/// far in the call chain; any path already in `visited` is skipped to break
/// cycles.
///
/// `instance_namespace` is the binding namespace this canvas instance lives
/// under (a list-slot instance's `piplist/[000i]`, inherited by its own
/// children). Namespacing at resolve START — before state tags and the style
/// cascade — lets instance-specific data (pip states) drive styling.
fn resolve_canvas_graph_inner(
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
    depth: u8,
    visited: &mut HashSet<String>,
    inherited_style: Option<&serde_json::Value>,
    inherited_style_identifier: Option<&str>,
    inherited_param_inputs: Option<&[serde_json::Value]>,
    inherited_boolean_bindings: &HashMap<String, bool>,
    bound_view_canvas: Option<&str>,
    defaults: &crate::defaults::DefaultValueRegistry,
    instance_namespace: Option<&str>,
    host_stage_size: Option<(f32, f32)>,
) -> Result<BbScene, String> {
    let mut scene = parse_bb_canvas(root_json)?;
    inline_animation_timeline_references(&mut scene, fetch_by_path);
    if let Some(namespace) = instance_namespace {
        namespace_child_scene_bindings(&mut scene, namespace);
    }
    let record_value = root_json.get("_RecordValue_").ok_or("missing _RecordValue_")?;
    let local_boolean_bindings = {
        let mut bindings = inherited_boolean_bindings.clone();
        bindings.extend(
            crate::bb_state_filter::resolved_boolean_variable_bindings_with_param_inputs_and_inherited(
                record_value,
                inherited_param_inputs.unwrap_or(&[]),
                inherited_boolean_bindings,
            ),
        );
        bindings
    };

    if depth >= MAX_CANVAS_DEPTH {
        return Ok(scene);
    }

    // Collect Pass 2 URLs from the ROOT canvas's own scene nodes BEFORE Pass 1
    // merges child canvas nodes.  Without this guard, Pass 2 would follow
    // WidgetCanvas.canvas references that originated in merged child scenes
    // (e.g. sub-view tabs inside a master canvas), pulling in unrelated sibling
    // canvases and producing a mixed-content scene.
    //
    // Pass 1 below may still add new WidgetCanvas nodes via child canvas merges,
    // but those children are responsible for following their own WidgetCanvas
    // URLs in their own recursive resolve calls — not in ours.
    // Also capture paramInputValues so child scenes can inherit parent-provided
    // component-parameter overrides such as annunciator labels. Dynamic parent
    // field bindings are injected separately after child resolution so they can
    // preserve binding graphs instead of collapsing to concrete values.
    let root_canvas_urls: Vec<(BbNodeId, String, Vec<serde_json::Value>)> = scene
        .nodes
        .values()
        .filter(|n| n.ty == BbNodeType::WidgetCanvas)
        .filter_map(|n| {
            let url = n.raw.get("canvas").and_then(|v| v.as_str())?;
            if url.is_empty() || url == "null" {
                return None;
            }
            let param_inputs: Vec<serde_json::Value> = n
                .raw
                .get("paramInputValues")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            Some((n.id, url.to_owned(), param_inputs))
        })
        .collect();

    let debug_trace = std::env::var("STARBREAKER_UI_DEBUG").as_deref() == Ok("1");

    let mut local_style_identifier = inherited_style_identifier.map(ToOwned::to_owned);
    let local_style_value = record_value.get("style").and_then(|style| {
        if let Some(style_url) = style.as_str().filter(|s| !s.is_empty()) {
            local_style_identifier = Some(extract_record_name(style_url));
            match fetch_by_path(style_url) {
                Ok(style_json) => Some(style_json.get("_RecordValue_").cloned().unwrap_or(style_json)),
                Err(e) => {
                    log::warn!(
                        "bb_resolve: failed to fetch canvas-level style record '{}': {}",
                        style_url,
                        e
                    );
                    None
                }
            }
        } else if style.is_object() && !style.is_null() {
            Some(style.clone())
        } else {
            None
        }
    });
    let palette_source = local_style_value.as_ref().or(inherited_style);

    // Pass 1: follow the default-state canvas-reference modifier (extracted
    // to pass1.part for the line cap). Returns the canvas norms merged at
    // THIS level, consumed by the Pass-2 skip logic below.
    let pass1_merged_norms = run_pass1_canvas_references(
        &mut scene,
        record_value,
        root_json,
        manufacturer_id,
        fetch_by_path,
        loc_fetcher,
        depth,
        visited,
        palette_source,
        local_style_identifier.as_deref(),
        &local_boolean_bindings,
        bound_view_canvas,
        defaults,
        instance_namespace,
        host_stage_size,
        debug_trace,
    );

    // Pass 2: follow WidgetCanvas.canvas field references.
    //
    // Host canvases such as M_MFD_Screen store their content canvas URL in the
    // `canvas` field of a `BuildingBlocks_WidgetCanvas` scene node rather than
    // in `defaultStyles.entries`.  We fetch and resolve each such URL so the
    // merged scene captures the full content hierarchy.
    //
    // Only the URLs collected from the root canvas's own nodes (before Pass 1)
    // are followed here — see the comment above.
    //
    // Some WidgetCanvas nodes serve as mode-switchable content slots whose
    // DEFAULT canvas matches a canvas already followed by Pass 1.  To prevent
    // Pass 2 from fetching those canvases again, we collect every canvas norm
    // referenced by a conditional Pass-1 entry.
    //
    // **Snapshot semantics**: we capture `visited` immediately after Pass 1 so
    // we can distinguish "already merged by Pass 1" from "first time seen in
    // Pass 2".  The outer Pass 2 loop does NOT insert into `visited`, which
    // allows the same template URL to appear multiple times in `root_canvas_urls`
    // with different `paramInputValues` (e.g. the 5 chiclet slots in the
    // annunciator screen all share one `h_eng_annunciator` template canvas).
    // Cycle protection is provided by the MAX_CANVAS_DEPTH depth limit on
    // recursive `resolve_canvas_graph_inner` calls — no cycle can run forever.
    let conditional_canvas_norms: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        for entry in all_canvas_guard_entries(record_value) {
            let has_conditions = entry
                .get("conditionsList")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            if !has_conditions {
                continue;
            }
            let Some(mods) = entry.get("modifiers").and_then(|v| v.as_array()) else {
                continue;
            };
            for modifier in mods {
                let Some(field) = modifier.get("field") else {
                    continue;
                };
                let is_canvas_ref = field
                    .get("_Type_")
                    .and_then(|v| v.as_str())
                    == Some("BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord");
                if !is_canvas_ref {
                    continue;
                }
                if let Some(path) = field.get("value").and_then(|v| v.as_str()) {
                    let norm = extract_record_name(path).to_ascii_lowercase();
                    set.insert(norm);
                }
            }
        }
        set
    };

    // Child canvases' style entries, re-applied (subtree-scoped) after THIS
    // level's state-tag resolution. A child's own cascade ran before its
    // dynamic params were injected, so entries conditioned on param-derived
    // state tags (the annunciator chiclets' "Critical - Text"/"Off - Text")
    // can only match here.
    #[allow(clippy::type_complexity)]
    let mut deferred_child_style_passes: Vec<(
        BbNodeId,
        Vec<serde_json::Value>,
        serde_json::Value,
        serde_json::Value,
        String,
        crate::bb_style_engine::Tier,
    )> = Vec::new();

    {
        let mut canvas_urls = root_canvas_urls;

        // Compute the set of WidgetCanvas node pointers whose `Instantiated`
        // field binding evaluates to `false` under static defaults.  These
        // canvases are inactive at startup and must not be followed in Pass 2.
        // For canvases without any `Instantiated` bindings (e.g. MFD screens)
        // the set is empty and all WidgetCanvas URLs are followed normally.
        let mut instantiated_false = crate::bb_state_filter::instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
            record_value,
            inherited_param_inputs.unwrap_or(&[]),
            inherited_boolean_bindings,
            Some(defaults),
        );

        // MFD frame content-view selection: the frame embeds a landscape
        // (full-width) and portrait (narrow) content slot gated by `useportraitview`
        // and filled from `landscapecanvasguid` / `portraitcanvasguid`. The bound
        // content is the view's landscape canvas (physical screens are landscape),
        // so inject it onto the landscape slot at full width and skip the portrait
        // slot — not whichever slot's authored placeholder matches the content.
        if let Some(bound) = bound_view_canvas {
            crate::mfd_view::apply_bound_mfd_view_with_host_stage(
                &mut scene,
                bound,
                &mut canvas_urls,
                &mut instantiated_false,
                host_stage_size,
            );
        }

        let list_namespaces = apply_list_slot_bindings(
            &mut scene,
            &mut canvas_urls,
            &mut instantiated_false,
            defaults,
        );

        deactivate_subtrees(&mut scene, &instantiated_false);

        // Force-activate authored-inactive nodes whose `IsActive` binding resolves
        // a GENUINE true at rest (e.g. the cockpit-radar background
        // `image_Background`, `IsActive ← NOT(IsVolumetric)`, at the flat radar).
        // The deactivation-only filter above can't flip an authored-false node
        // active, but the engine treats the `IsActive` binding as the runtime
        // truth. Applied AFTER deactivation so a genuine active wins; scoped to a
        // genuine `Some(true)` (not the unset→override), so medical's at-rest-
        // unset `IsActive` nodes are untouched (workflow §10).
        let force_active = crate::bb_state_filter::forced_active_widgets_with_defaults(
            record_value,
            inherited_param_inputs.unwrap_or(&[]),
            inherited_boolean_bindings,
            Some(defaults),
        );
        for id in &force_active {
            if let Some(node) = scene.nodes.get_mut(id) {
                node.is_active = true;
            }
        }

        // Snapshot the visited set as it stands right after Pass 1.  Used
        // below to test "was this canvas already merged by Pass 1?" without
        // modifying the set (so the same URL may appear multiple times in the
        // loop with different paramInputValues and each instance is processed).
        // Keys are instance-namespace-scoped like Pass 1's (see pass1.part).
        let _ = &pass1_merged_norms;
        let post_pass1_visited: std::collections::HashSet<String> = visited.clone();

        for (node_id, url, param_inputs) in canvas_urls {
            // Skip WidgetCanvas nodes whose Instantiated binding is false.
            // This prevents inactive state sub-canvases (e.g. Attract, LogIn,
            // MainMenu on a medical kiosk) from being merged into the static render.
            if instantiated_false.contains(&node_id) {
                if debug_trace {
                    log::info!(
                        "bb_resolve[depth={}]: Pass2 skipping ptr:{} {} (Instantiated=false)",
                        depth, node_id, url,
                    );
                }
                continue;
            }
            let norm = match instance_namespace {
                Some(ns) => format!("{ns}|{}", extract_record_name(&url).to_ascii_lowercase()),
                None => extract_record_name(&url).to_ascii_lowercase(),
            };
            // If this canvas URL appears in ANY conditional entry but was NOT
            // selected in Pass 1 (not present in the post-Pass-1 snapshot),
            // skip it.  It is the default canvas for a mode-switchable slot,
            // and the selected mode's canvas has already replaced it.
            if conditional_canvas_norms.contains(&norm) && !post_pass1_visited.contains(&norm) {
                if debug_trace {
                    log::info!(
                        "bb_resolve[depth={}]: Pass2 skipping {} (not selected by Pass1 conditional)",
                        depth, norm,
                    );
                }
                continue;
            }
            // Skip canvases that were already merged by Pass 1.  We do NOT
            // insert into `visited` here so that multiple WidgetCanvas nodes
            // referencing the same template URL (e.g. chiclet slots all sharing
            // `h_eng_annunciator`) are each resolved independently.
            if post_pass1_visited.contains(&norm) {
                log::debug!(
                    "bb_resolve: skipping already-pass1-merged WidgetCanvas url '{}'",
                    url
                );
                continue;
            }
            if debug_trace {
                log::info!(
                    "bb_resolve[depth={}]: Pass2 following WidgetCanvas.canvas -> {}",
                    depth, norm,
                );
            }
            let child_json = match fetch_by_path(&url) {
                Ok(json) => json,
                Err(e) => {
                    log::warn!(
                        "bb_resolve: failed to fetch WidgetCanvas canvas '{}': {}",
                        url,
                        e
                    );
                    continue;
                }
            };
            // A list-slot instance introduces its own namespace (already
            // fully qualified — the count binding was namespaced at this
            // level's resolve start); other children inherit this canvas's.
            // An ABSOLUTE `urlPostfix` (leading slash — the emissions
            // instance's `/Vehicle/SignatureSystem`) addresses the engine
            // data root and becomes the child instance's namespace. Relative
            // postfixes are NOT composed: the medical canvases author their
            // bindings (partially) pre-qualified and the platinum-frozen
            // registry keys capture that resolution.
            let postfix_namespace: Option<String> = scene
                .nodes
                .get(&node_id)
                .and_then(|n| n.raw.get("urlPostfix"))
                .and_then(|v| v.as_str())
                .filter(|s| s.starts_with('/') && !s.trim_matches('/').is_empty())
                .map(|s| s.trim_matches('/').to_owned());
            let child_namespace = list_namespaces
                .get(&node_id)
                .map(String::as_str)
                .or(postfix_namespace.as_deref())
                .or(instance_namespace);
            let mut child_scene = match resolve_canvas_graph_inner(
                &child_json,
                manufacturer_id,
                fetch_by_path,
                loc_fetcher,
                depth + 1,
                visited,
                palette_source,
                local_style_identifier.as_deref(),
                Some(param_inputs.as_slice()),
                &local_boolean_bindings,
                bound_view_canvas,
                defaults,
                child_namespace,
                host_stage_size,
            ) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!(
                        "bb_resolve: failed to resolve WidgetCanvas content '{}': {}",
                        url,
                        e
                    );
                    continue;
                }
            };
            // The child's pending state tags reflect ITS cascade-time
            // knowledge — compute BEFORE the parent injections wire the
            // params (afterwards the chains resolve and nothing is pending).
            let child_pending = crate::bb_bindings::pending_state_tags(&child_scene, defaults);
            inject_param_overrides(&param_inputs, &mut child_scene);
            inject_dynamic_param_field_bindings(
                node_id,
                &scene.operations,
                &mut child_scene,
                child_namespace.is_some(),
            );
            for (entries, fills, chrome, origin_identifier, origin_tier) in
                collect_late_state_style_entries(&child_json, &child_pending, manufacturer_id, fetch_by_path)
            {
                deferred_child_style_passes.push((
                    node_id,
                    entries,
                    fills,
                    chrome,
                    origin_identifier,
                    origin_tier,
                ));
            }
            merge_child_scene(&mut scene, child_scene, "", Some(node_id), false);
        }
    }

    // Materialise arrayVariable-bound lists BEFORE state tags and the style
    // cascade so per-entry data (pip states) participates in styling. Entry
    // counts resolve against this instance's namespace (the power pip stacks
    // differ per system: 4/6/4).
    apply_array_variable_lists(&mut scene, instance_namespace.unwrap_or(""), defaults);

    // Apply style modifiers after scene resolution is complete (the R2 phase).
    // Cascade order mirrors the engine: generic sharedStyles base first, then
    // the selected brand's `brandStyles[]` entries (per-brand override for MC_*,
    // single-entry for IC_*) or the canvas-level `style` record (e.g. `s_bioc`
    // on medical canvases) override it, then canvas embeddedStyles.
    let preferred_brand = local_style_identifier.as_deref();
    if record_authors_texture_body_background(record_value)
        && scene.nodes.values().any(body_background_uses_texture)
    {
        let module_path = standard_body_background_widget_path();
        match fetch_by_path(&module_path) {
            Ok(standard_record) => {
                // The standard's brand container is matched by brand-record
                // identity: resolve the brand THIS canvas selected (the MFD content
                // view's `s_drak_hud`) by IDENTITY + canvas family — NOT the
                // manufacturer-prefix scan, which cannot distinguish the hud/env
                // container pair the shared standard carries (B1).
                let canvas_name = root_json
                    .get("_RecordName_")
                    .and_then(|v| v.as_str())
                    .or_else(|| record_value.get("_RecordName_").and_then(|v| v.as_str()));
                let brand_class = bb_brand_style::brand_class_for_canvas(canvas_name);
                let canvas_brand = bb_brand_style::resolve_brand_identity(
                    root_json,
                    preferred_brand,
                    manufacturer_id,
                    brand_class,
                    bb_brand_style::BrandPolicy::Default,
                )
                .map(|brand| brand.identifier);
                apply_body_background_standard_styles(
                    &mut scene,
                    &standard_record,
                    manufacturer_id,
                    canvas_brand.as_deref().or(preferred_brand),
                    brand_class,
                    loc_fetcher,
                );
            }
            Err(e) => {
                log::debug!("bb_resolve: no standard body-background widget '{module_path}': {e}");
            }
        }
    }

    // The modular-kit style id comes from the canvas's style link (medical
    // `s_bioc` → `sk_bioc`) or, for brandStyles-selected canvases (the MFD
    // header's `s_drak_hud`), from the resolved brand identifier.
    let modular_style_id = local_style_identifier.clone().or_else(|| {
        bb_brand_style::resolve_brand_style(root_json, manufacturer_id, preferred_brand)
            .map(|brand| brand.identifier)
    });
    let standard_embedded_entries = expand_widget_standards(&mut scene, fetch_by_path);
    let tag_defaults_override;
    let tag_defaults: &crate::defaults::DefaultValueRegistry =
        if std::env::var("SB_TAGS_NO_DEFAULTS").as_deref() == Ok("1") {
            tag_defaults_override = Default::default();
            &tag_defaults_override
        } else {
            defaults
        };
    crate::bb_bindings::resolve_state_tags_into_scene(&mut scene, tag_defaults);
    // State tags whose producing chains are still unresolvable at THIS level
    // (parent-injected params) gate the cascade below: entries referencing
    // them are skipped and re-applied by the parent's deferred pass.
    let pending_state_tags = crate::bb_bindings::pending_state_tags(&scene, tag_defaults);
    // Deferred child-canvas entries re-run against the freshly resolved state
    // tags, scoped to each child's subtree (see the collection site above).
    // The empty palette keeps colours token-only; an unchanged token is a
    // no-op against the child cascade's earlier resolution.
    for (host_id, entries, fills, chrome, identifier, origin_tier) in &deferred_child_style_passes {
        crate::bb_style_engine::apply(
            &mut scene,
            &[crate::bb_style_engine::StyleSheet {
                tier: *origin_tier,
                identifier: identifier.clone(),
                fills,
                chrome,
                entries,
                scope: crate::bb_style_engine::SheetScope::Subtree(*host_id),
            }],
            loc_fetcher,
        );
    }
    // The expanded standards' own embeddedStyles run against the freshly
    // resolved state tags (the scrollbar's `_Show` → `scrollbar-show` tag
    // activates `ComponentRoot` through its `RootShow` entry).
    if !standard_embedded_entries.is_empty() {
        let empty_raw = serde_json::json!({});
        crate::bb_style_engine::apply(
            &mut scene,
            &[crate::bb_style_engine::StyleSheet::uniform(
                crate::bb_style_engine::Tier::StandardModule,
                "widget-standard-embedded",
                &empty_raw,
                standard_embedded_entries.as_slice(),
            )],
            loc_fetcher,
        );
    }
    apply_canvas_style_cascade(
        &mut scene,
        root_json,
        record_value,
        manufacturer_id,
        preferred_brand,
        local_style_value.as_ref(),
        palette_source,
        fetch_by_path,
        loc_fetcher,
        &pending_state_tags,
    );
    if let Some(style_id) = modular_style_id.as_deref() {
        apply_modular_kit_sheets(&mut scene, style_id, fetch_by_path, loc_fetcher);
    }
    finalize_widget_standard_fields(&mut scene, &Default::default());

    // `exportNode: false` marks editor-only authored nodes (mock layout
    // images, "...DEL"/"(Old)"/"TEST" leftovers, ambient-greeble groups like
    // the medical footer's `base_animatedelements`): the engine build drops
    // them. Parse already deactivates the node itself; descendants author
    // `exportNode: true`, so the subtree is deactivated here. Structural
    // replacement for the former `base_animatedelements` name match
    // (plan P5.1, 2026-06-12) — the duplicate `card_OutputTitleTextContainer`
    // under the power output card's `base_MinBatteryAssignment` (one false,
    // one true sibling) is the discriminating counterexample showing the flag,
    // not the name, carries the semantics.
    let export_disabled_roots: std::collections::HashSet<BbNodeId> = scene
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.raw.get("exportNode").and_then(|v| v.as_bool()) == Some(false)
        })
        .map(|(id, _)| *id)
        .collect();
    if !export_disabled_roots.is_empty() {
        deactivate_subtrees(&mut scene, &export_disabled_roots);
    }

    Ok(scene)
}

/// Pass 1 of the recursive resolver: follow default-state canvas-reference
/// modifiers (see `pick_default_entry_decision`), recursively resolving and
/// merging the referenced canvases. ONE entry is followed per HOST widget:
/// mode-switching canvases (GunsMode / NavMode / TurretMode share a slot)
/// stay mutually exclusive, while INDEPENDENT slot-filling entries (the
/// power item's OffPanel + heat-bar slots, selected by entry conditions)
/// each merge into their own host. Returns the canvas norms merged at this
/// level; once-only keys are scoped by the instance namespace so
/// materialised list instances re-merge their own per-instance content.
#[allow(clippy::too_many_arguments)]
fn run_pass1_canvas_references(
    scene: &mut BbScene,
    record_value: &serde_json::Value,
    root_json: &serde_json::Value,
    manufacturer_id: Option<&str>,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
    loc_fetcher: Option<&dyn LocFetcher>,
    depth: u8,
    visited: &mut HashSet<String>,
    palette_source: Option<&serde_json::Value>,
    local_style_identifier: Option<&str>,
    local_boolean_bindings: &HashMap<String, bool>,
    bound_view_canvas: Option<&str>,
    defaults: &crate::defaults::DefaultValueRegistry,
    instance_namespace: Option<&str>,
    host_stage_size: Option<(f32, f32)>,
    debug_trace: bool,
) -> std::collections::HashSet<String> {
    let mut pass1_merged_norms: std::collections::HashSet<String> = std::collections::HashSet::new();
    // The picked entry leads; further canvas-ref entries from the same active
    // scope follow only when their resolved HOST widget is distinct (one
    // entry per slot).
    let mut follow_entries: Vec<&serde_json::Value> = Vec::new();
    if let Some(selection) = pick_default_entry_decision(record_value, root_json, manufacturer_id) {
        if debug_trace {
            log::info!(
                "bb_resolve[depth={}]: default entry decision={} entry={:?} score={}",
                depth,
                selection.reason.as_str(),
                selection.entry.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                selection.score.unwrap_or(0),
            );
        }
        let mut seen_hosts: std::collections::HashSet<BbNodeId> = std::collections::HashSet::new();
        let selected_match_to = selection
            .entry
            .get("matchTo")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let leading_host = host_node_for_match_to(scene, selected_match_to)
            .or_else(|| host_canvas_node_for_entry(scene, selection.entry));
        if let Some(host) = leading_host {
            seen_hosts.insert(host);
        }
        follow_entries.push(selection.entry);
        // Additional canvas-ref hosts are followed in two cases:
        //   * MATERIALISED instances — the power item's OffPanel + heat-bar
        //     slots are independent fills (always followed); and
        //   * un-namespaced masters whose slots TILE — each a sub-full panel
        //     laid out alongside its sibling (the LR-indicator master's
        //     left/right half-width columns, both visible at once).
        // MC_S_Self_Master's five mode slots are FULL-size centred overlays
        // (`Percent >= 1.0`), so they are NOT sub-full tiling slots and stay
        // mutually exclusive under the at-rest single pick.
        let leading_is_tiling =
            leading_host.is_some_and(|host| host_is_subfull_tiling_slot(scene, host));
        for entry in pick_active_entries(record_value, root_json, manufacturer_id) {
            if std::ptr::eq(entry, selection.entry) {
                continue;
            }
            let has_canvas_ref = entry
                .get("modifiers")
                .and_then(|v| v.as_array())
                .is_some_and(|mods| {
                    mods.iter().any(|m| {
                        m.get("field")
                            .and_then(|f| f.get("_Type_"))
                            .and_then(|v| v.as_str())
                            == Some("BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord")
                    })
                });
            if !has_canvas_ref {
                continue;
            }
            let entry_match_to = entry.get("matchTo").and_then(|v| v.as_str()).unwrap_or("");
            let Some(host) = host_node_for_match_to(scene, entry_match_to)
                .or_else(|| host_canvas_node_for_entry(scene, entry))
            else {
                continue;
            };
            // Materialised instances follow every distinct host; un-namespaced
            // masters follow only when BOTH the leading and this host are
            // sub-full tiling panels — excludes the self-master overlay modes.
            let follow_this = instance_namespace.is_some()
                || (leading_is_tiling && host_is_subfull_tiling_slot(scene, host));
            if !follow_this {
                continue;
            }
            if seen_hosts.insert(host) {
                follow_entries.push(entry);
            }
        }
    }
    for entry in follow_entries {
        let entry_name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let match_to = entry.get("matchTo").and_then(|v| v.as_str()).unwrap_or("");

        if let Some(modifiers) = entry.get("modifiers").and_then(|v| v.as_array()) {
            for modifier in modifiers {
                let Some(field) = modifier.get("field") else {
                    continue;
                };
                let is_canvas_ref = field
                    .get("_Type_")
                    .and_then(|v| v.as_str())
                    == Some("BuildingBlocks_FieldModifierRecordRefTypeCanvasReferenceRecord");
                if !is_canvas_ref {
                    continue;
                }
                let Some(path) = field.get("value").and_then(|v| v.as_str()) else {
                    continue;
                };

                // Normalise path to a record name for once-only/cycle
                // detection. The key is scoped by the instance namespace:
                // materialised list instances (the power columns) each
                // re-merge their own copy of per-instance content (OffPanel,
                // heat bars), while un-namespaced canvases keep the global
                // once-only semantics the verified baselines pin.
                let norm = match instance_namespace {
                    Some(ns) => format!("{ns}|{}", extract_record_name(path).to_ascii_lowercase()),
                    None => extract_record_name(path).to_ascii_lowercase(),
                };
                if !visited.insert(norm.clone()) {
                    log::debug!("bb_resolve: skipping already-visited child canvas '{}'", path);
                    continue;
                }

                if debug_trace {
                    log::info!(
                        "bb_resolve[depth={}]: Pass1 following entry {:?} -> {}",
                        depth,
                        entry_name,
                        norm,
                    );
                }

                let child_json = match fetch_by_path(path) {
                    Ok(json) => json,
                    Err(e) => {
                        log::warn!("bb_resolve: failed to fetch child canvas '{}': {}", path, e);
                        continue;
                    }
                };
                let match_to_param_inputs = if match_to.is_empty() {
                    None
                } else {
                    param_inputs_for_match_to(scene, match_to)
                };
                // Recurse: resolve the child's own style-references and WidgetCanvas URLs.
                let child_result = resolve_canvas_graph_inner(
                    &child_json,
                    manufacturer_id,
                    fetch_by_path,
                    loc_fetcher,
                    depth + 1,
                    visited,
                    palette_source,
                    local_style_identifier,
                    match_to_param_inputs.as_deref(),
                    local_boolean_bindings,
                    bound_view_canvas,
                    defaults,
                    instance_namespace,
                    host_stage_size,
                );
                let mut child_scene = match child_result {
                    Ok(scene) => scene,
                    Err(e) => {
                        log::warn!(
                            "bb_resolve: failed to resolve child canvas '{}': {}",
                            path,
                            e
                        );
                        continue;
                    }
                };
                pass1_merged_norms.insert(norm.clone());
                // Pass1 canvas-reference merges can also carry localized
                // paramInputValues on the matched host node (same mechanism as
                // Pass2 WidgetCanvas.canvas references). Inject those overrides
                // so localized component-parameter defaults resolve correctly.
                if let Some(param_inputs) = match_to_param_inputs.as_ref() {
                    inject_param_overrides(param_inputs, &mut child_scene);
                }
                // Without a matchTo name the entry's style conditions select
                // the host canvas slot (the power master's `Set Canvas`, the
                // power item's OffPanel slot): wire its parameters AND merge
                // the content under it, so slot-level gates (`IsActive ←
                // ispoweredoff`) govern the merged subtree.
                let host_node_id = host_node_for_match_to(scene, match_to)
                    .or_else(|| host_canvas_node_for_entry(scene, entry));
                if let Some(host_node_id) = host_node_id {
                    inject_dynamic_param_field_bindings(
                        host_node_id,
                        &scene.operations,
                        &mut child_scene,
                        instance_namespace.is_some(),
                    );
                }
                let host_override: Option<BbNodeId> = if std::env::var("SB_NO_HOSTOVR").as_deref() == Ok("1") { None } else if match_to.is_empty() { host_node_id } else { None };
                merge_child_scene(scene, child_scene, match_to, host_override, false);
            }
        }
    }

    pass1_merged_norms
}

fn inline_animation_timeline_references(
    scene: &mut BbScene,
    fetch_by_path: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) {
    for node in scene.nodes.values_mut() {
        let Some(timeline_slot) = node
            .raw
            .get_mut("animation")
            .and_then(|animation| animation.get_mut("animationTimeline"))
        else {
            continue;
        };
        let Some(timeline_path) = timeline_slot
            .get("timelineRecord")
            .and_then(|value| value.as_str())
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        let Ok(timeline_record) = fetch_by_path(timeline_path) else {
            continue;
        };
        let record_value = timeline_record
            .get("_RecordValue_")
            .unwrap_or(&timeline_record);
        let Some(timeline) = record_value
            .get("timeline")
            .or_else(|| record_value.get("animationTimeline"))
            .or_else(|| record_value.get("keyframes").map(|_| record_value))
        else {
            continue;
        };
        if timeline.get("keyframes").is_some() {
            *timeline_slot = timeline.clone();
        }
    }
}

pub(crate) fn deactivate_subtrees(scene: &mut BbScene, roots: &std::collections::HashSet<BbNodeId>) {
    let mut stack: Vec<BbNodeId> = roots.iter().copied().collect();
    let mut seen: std::collections::HashSet<BbNodeId> = std::collections::HashSet::new();
    while let Some(node_id) = stack.pop() {
        if !seen.insert(node_id) {
            continue;
        }
        let Some(node) = scene.nodes.get_mut(&node_id) else {
            continue;
        };
        node.is_active = false;
        stack.extend(node.children.iter().copied());
    }
}

fn pick_active_entries<'a>(
    record_value: &'a serde_json::Value,
    record_root: &'a serde_json::Value,
    manufacturer_id: Option<&str>,
) -> Vec<&'a serde_json::Value> {
    let preferred_brand = record_value
        .get("style")
        .and_then(|v| v.as_str())
        .map(extract_record_name);
    // Use new brand-style resolver (R1 phase) which handles IC_* per-canvas override + generic fallback
    if let Some(brand_style) = bb_brand_style::resolve_brand_style(
        record_root,
        manufacturer_id,
        preferred_brand.as_deref(),
    ) {
        return brand_style.entries.iter().collect();
    }

    // Fall back to defaultStyles when no brand match
    record_value
        .get("defaultStyles")
        .map(entries_from)
        .unwrap_or_default()
}

/// Return the single default-state entry to follow in Pass 1.
///
/// Prefers the first entry whose `conditionsList` is absent or empty
/// (unconditional — always active).  Falls back to the entry that targets
/// the scene node with the highest style-tag count when all entries are
/// conditional (the "most-tagged" node heuristic selects the primary content
/// slot over sub-component slots — e.g. `canvas_GunsMode` with 2 tags wins
/// over `canvas_AmmoNumbers` with 1 tag on `MC_S_Self_Master`).  Falls back
/// to the first entry overall as a last resort.
struct DefaultEntryDecision<'a> {
    entry: &'a serde_json::Value,
    reason: DefaultEntryReason,
    score: Option<usize>,
}

#[derive(Clone, Copy)]
enum DefaultEntryReason {
    FirstUnconditional,
    HighestTagScore,
    HighestTagScoreThenSpecificity,
    FirstEntryFallback,
}

impl DefaultEntryReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::FirstUnconditional => "first_unconditional",
            Self::HighestTagScore => "highest_tag_score",
            Self::HighestTagScoreThenSpecificity => "highest_tag_score_then_specificity",
            Self::FirstEntryFallback => "first_entry_fallback",
        }
    }
}

fn pick_default_entry_decision<'a>(
    record_value: &'a serde_json::Value,
    record_root: &'a serde_json::Value,
    manufacturer_id: Option<&str>,
) -> Option<DefaultEntryDecision<'a>> {
    let entries = pick_active_entries(record_value, record_root, manufacturer_id);
    // Prefer first unconditional (empty/absent conditionsList).
    if let Some(entry) = entries.iter().copied().find(|e| {
        e.get("conditionsList")
            .and_then(|v| v.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true)
    }) {
        return Some(DefaultEntryDecision {
            entry,
            reason: DefaultEntryReason::FirstUnconditional,
            score: None,
        });
    }

    // When all entries are conditional, use style-tag count on the matched
    // scene node as a tiebreaker.  Entries that target scene nodes with MORE
    // tags are more specifically annotated (e.g. the primary weapon-info slot
    // carries both a system tag and a content-type tag), so they are preferred.
    if entries.len() > 1 {
        let scene = record_value
            .get("scene")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        // Build a map from tag RecordId → number of style tags on the scene
        // node that carries that tag.
        let mut tag_to_node_tag_count: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for node in scene {
            let style_tags = node
                .get("styleTags")
                .and_then(|v| v.as_array())
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            let count = style_tags.len();
            for tag in style_tags {
                if let Some(rid) = tag.get("_RecordId_").and_then(|v| v.as_str()) {
                    // Prefer the higher count if a tag appears on multiple nodes.
                    let e = tag_to_node_tag_count.entry(rid).or_insert(0);
                    if count > *e {
                        *e = count;
                    }
                }
            }
        }

        // Score each entry by the max style-tag count of its condition's tag.
        let mut best_entry = entries[0];
        let mut best_score = 0usize;
        let mut best_specificity = 0usize;
        let mut used_specificity_tiebreak = false;
        for entry in &entries {
            let score = condition_tag_score(entry, &tag_to_node_tag_count);
            let specificity = condition_tag_specificity(entry, tag_to_node_tag_count.keys().copied());
            if score > best_score {
                best_score = score;
                best_specificity = specificity;
                best_entry = entry;
                used_specificity_tiebreak = false;
            } else if score == best_score && score > 0 && specificity > best_specificity {
                best_specificity = specificity;
                best_entry = entry;
                used_specificity_tiebreak = true;
            }
        }
        if best_score > 0 {
            return Some(DefaultEntryDecision {
                entry: best_entry,
                reason: if used_specificity_tiebreak {
                    DefaultEntryReason::HighestTagScoreThenSpecificity
                } else {
                    DefaultEntryReason::HighestTagScore
                },
                score: Some(best_score),
            });
        }
    }

    // Last resort: first entry (structural default when all entries are
    // conditional and no scene-node heuristic applies, e.g. MC_S_Target_Master
    // which has exactly one entry used at runtime despite being tagged
    // conditional in the data).
    entries.into_iter().next().map(|entry| DefaultEntryDecision {
        entry,
        reason: DefaultEntryReason::FirstEntryFallback,
        score: None,
    })
}

/// Extract the maximum style-tag count of any scene node matched by an
/// entry's `conditionsList` tag conditions.
///
/// Returns 0 when the entry has no recognisable tag conditions or none of its
/// tags appear in `tag_to_count`.
fn condition_tag_score(
    entry: &serde_json::Value,
    tag_to_count: &std::collections::HashMap<&str, usize>,
) -> usize {
    let cond_lists = match entry.get("conditionsList").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return 0,
    };
    let mut max = 0usize;
    for cond_list in cond_lists {
        let conditions = cond_list
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        for cond in conditions {
            max = max.max(score_condition_node(cond, tag_to_count));
        }
    }
    max
}

fn condition_tag_specificity<'a>(
    entry: &serde_json::Value,
    known_tags: impl IntoIterator<Item = &'a str>,
) -> usize {
    let known_tags: std::collections::HashSet<&str> = known_tags.into_iter().collect();
    let cond_lists = match entry.get("conditionsList").and_then(|v| v.as_array()) {
        Some(v) => v,
        None => return 0,
    };
    cond_lists
        .iter()
        .filter_map(|cond_list| cond_list.get("conditions").and_then(|v| v.as_array()))
        .flatten()
        .filter_map(|condition| {
            condition
                .get("tag")
                .and_then(|tag| tag.get("_RecordId_"))
                .and_then(|value| value.as_str())
        })
        .filter(|tag| known_tags.contains(tag))
        .count()
}

/// Recursively walk a condition node to find tag RecordIds and return the
/// maximum style-tag count found in `tag_to_count`.
fn score_condition_node(
    cond: &serde_json::Value,
    tag_to_count: &std::collections::HashMap<&str, usize>,
) -> usize {
    // Direct tag condition: {"_Type_": "…ConditionTag", "tag": {"_RecordId_": "…"}}
    if let Some(tag_id) = cond
        .get("tag")
        .and_then(|t| t.get("_RecordId_"))
        .and_then(|v| v.as_str())
    {
        return tag_to_count.get(tag_id).copied().unwrap_or(0);
    }
    // Compound (AllOf/AnyOf): recurse into "conditions" children.
    let mut max = 0usize;
    if let Some(children) = cond.get("conditions").and_then(|v| v.as_array()) {
        for child in children {
            max = max.max(score_condition_node(child, tag_to_count));
        }
    }
    max
}

fn entries_from(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    value
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|entries| entries.iter().collect())
        .unwrap_or_default()
}

/// Every style entry that could carry a `canvas`-reference modifier, across
/// `defaultStyles` AND every `brandStyles[]` manufacturer block — NOT just the
/// active brand (`pick_active_entries`).
///
/// The Pass-2 mode-switch guard (`conditional_canvas_norms`) needs the canvas
/// URL of *any* conditional entry "regardless of which entry Pass 1 selected"
/// (the resolver header's documented intent). A WidgetCanvas host can author a
/// `canvas` DEFAULT that is another manufacturer's variant placeholder — the
/// master-mode display authors the AEGS-Gladius variant as its editor
/// placeholder while a `Set Canvas` modifier swaps it to the active ship's
/// variant. On a DRAK ship `pick_active_entries` returns only the DRAK entries,
/// so the AEGS placeholder is absent from the guard and Pass 2 follows it,
/// overlaying the AEGS ship-in-oval on top of the DRAK layout. Collecting every
/// brand's entries closes that gap; the guard still only skips a URL that was
/// NOT visited by Pass 1, so genuine content canvases (which no brand entry
/// references) are untouched.
fn all_canvas_guard_entries(record_value: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut out: Vec<&serde_json::Value> = Vec::new();
    if let Some(default_styles) = record_value.get("defaultStyles") {
        out.extend(entries_from(default_styles));
    }
    if let Some(brands) = record_value.get("brandStyles").and_then(|v| v.as_array()) {
        for brand in brands {
            out.extend(entries_from(brand));
        }
    }
    out
}

/// Node IDs at or above this value belong to the widget-standard expansion
/// band. Template nodes injected by `expand_widget_standards` are allocated
/// here so they never shift the sequential IDs of ordinary merged canvas
/// nodes (platinum snapshots key elements by node ID).
pub(crate) const EXPANSION_ID_BASE: BbNodeId = 0xF000_0000;

pub(crate) fn merge_child_scene(
    parent_scene: &mut BbScene,
    child_scene: BbScene,
    match_to: &str,
    host_parent_override: Option<BbNodeId>,
    reserve_id_band: bool,
) {
    let BbScene {
        coordinate_method: child_coordinate_method,
        canvas_size: child_canvas_size,
        roots: child_roots,
        nodes: child_nodes,
        operations: child_ops,
        ..
    } = child_scene;

    // Build a collision-free mapping from child original IDs to new IDs.
    // We use monotonic counters that wrap and skip any ID already present
    // in the parent, so the merge is always safe regardless of the depth to
    // which children have been recursively merged.
    //
    // Two ID bands keep ordinary canvas-node IDs independent of template
    // expansion: the low band continues after the largest non-expansion key,
    // while expansion-band child nodes (and whole expansion merges, flagged
    // by `reserve_id_band`) re-allocate within the band so injecting template
    // nodes at any depth never renumbers sibling canvases merged later.
    let mut next_low: BbNodeId = parent_scene
        .nodes
        .keys()
        .rev()
        .find(|&&k| k < EXPANSION_ID_BASE)
        .copied()
        .unwrap_or(0)
        .wrapping_add(1);
    let mut next_high: BbNodeId = parent_scene
        .nodes
        .keys()
        .next_back()
        .copied()
        .filter(|&k| k >= EXPANSION_ID_BASE)
        .map(|k| k.wrapping_add(1))
        .unwrap_or(EXPANSION_ID_BASE);
    let mut id_map: HashMap<BbNodeId, BbNodeId> = HashMap::with_capacity(child_nodes.len());
    for &orig_id in child_nodes.keys() {
        let counter = if reserve_id_band || orig_id >= EXPANSION_ID_BASE {
            &mut next_high
        } else {
            &mut next_low
        };
        // Advance past any ID already occupied in the parent or already
        // assigned to another child node in this batch.
        while parent_scene.nodes.contains_key(counter) || id_map.values().any(|&v| v == *counter) {
            *counter = counter.wrapping_add(1);
        }
        id_map.insert(orig_id, *counter);
        *counter = counter.wrapping_add(1);
    }

    let child_roots_reided: Vec<BbNodeId> = child_roots
        .iter()
        .filter_map(|id| id_map.get(id).copied())
        .collect();

    let host_parent_id = if let Some(id) = host_parent_override {
        if parent_scene.nodes.contains_key(&id) {
            Some(id)
        } else {
            None
        }
    } else {
        find_host_parent(parent_scene, match_to)
    };
    let Some(host_parent_id) = host_parent_id else {
        log::warn!("bb_resolve: no host parent available for child canvas merge");
        return;
    };
    if should_adopt_child_coordinate_method(parent_scene, host_parent_id, child_coordinate_method) {
        parent_scene.coordinate_method = child_coordinate_method;
    }
    let canvas_scale = child_canvas_scale_for_host(parent_scene, host_parent_id, child_canvas_size);

    let mut inserted_child_roots = Vec::new();
    for (orig_id, mut node) in child_nodes {
        let new_id = match id_map.get(&orig_id).copied() {
            Some(id) => id,
            None => {
                log::warn!("bb_resolve: no id mapping for child node {orig_id}; skipping");
                continue;
            }
        };
        node.id = new_id;
        node.parent = node.parent.and_then(|p| id_map.get(&p).copied());
        node.children = node
            .children
            .into_iter()
            .filter_map(|c| id_map.get(&c).copied())
            .collect();

        if child_roots.contains(&orig_id) {
            node.parent = Some(host_parent_id);
            inserted_child_roots.push(new_id);
        }
        if let Some((sx, sy)) = canvas_scale {
            scale_node_from_child_canvas(&mut node, sx, sy);
        }

        parent_scene.nodes.insert(new_id, node);
    }

    // Remap operation-pointer namespace to avoid collisions across merged child
    // canvases. Node IDs and operation IDs are distinct domains in source
    // records, but both use ptr:N string encoding and can collide numerically.
    let op_id_map = remap_child_operation_ids(parent_scene, &child_ops);

    // Remap ptr: / _PointsTo_:ptr: references in operations using both node-id
    // and operation-id maps.
    let mut remapped_ops = child_ops;
    for op in &mut remapped_ops {
        remap_ptrs_in_json_map(op, &id_map, &op_id_map);
        // Mark instance-local operations so the binding resolver's global
        // by-field-name override scan ignores them (see `bb_bindings::build`).
        // Expansion merges (widget-standard templates) are additionally
        // flagged: unlike canvas-reference merges they never received a
        // parent's parameter wiring, so injection may still target them.
        if let Some(map) = op.as_object_mut() {
            map.insert("_MergedOp_".to_string(), serde_json::Value::Bool(true));
            if reserve_id_band {
                map.insert("_ExpansionOp_".to_string(), serde_json::Value::Bool(true));
            }
        }
    }
    parent_scene.operations.extend(remapped_ops);

    if let Some(host) = parent_scene.nodes.get_mut(&host_parent_id) {
        let roots_to_add = if inserted_child_roots.is_empty() {
            child_roots_reided
        } else {
            inserted_child_roots
        };
        host.children.extend(roots_to_add);
        let mut seen = std::collections::BTreeSet::new();
        host.children.retain(|id| seen.insert(*id));
    }
}

fn should_adopt_child_coordinate_method(
    parent_scene: &BbScene,
    host_parent_id: BbNodeId,
    child_coordinate_method: crate::bb_scene::BbCoordinateMethod,
) -> bool {
    if matches!(child_coordinate_method, crate::bb_scene::BbCoordinateMethod::UseRaw) {
        return false;
    }

    let Some(host) = parent_scene.nodes.get(&host_parent_id) else {
        return false;
    };

    matches!(host.ty, crate::bb_scene::BbNodeType::WidgetCanvas)
        && host.parent.is_none()
        && matches!(host.sizing.width, BbValue::Percent(value) if (value - 1.0).abs() < f32::EPSILON)
        && matches!(host.sizing.height, BbValue::Percent(value) if (value - 1.0).abs() < f32::EPSILON)
        && host.position.x.abs() < f32::EPSILON
        && host.position.y.abs() < f32::EPSILON
}

fn child_canvas_scale_for_host(
    parent_scene: &BbScene,
    host_parent_id: BbNodeId,
    child_canvas_size: (f32, f32),
) -> Option<(f32, f32)> {
    let host = parent_scene.nodes.get(&host_parent_id)?;
    let child_w = child_canvas_size.0;
    let child_h = child_canvas_size.1;
    if child_w <= 0.0 || child_h <= 0.0 {
        return None;
    }
    let host_w = match host.sizing.width {
        BbValue::Fixed(v) if v > 0.0 => v,
        _ => return None,
    };
    let host_h = match host.sizing.height {
        BbValue::Fixed(v) if v > 0.0 => v,
        _ => return None,
    };
    let sx = host_w / child_w;
    let sy = host_h / child_h;
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return None;
    }
    if sx > 4.0 || sy > 4.0 || sx < 0.25 || sy < 0.25 {
        log::debug!(
            "bb_resolve: skipping child-canvas scaling for host ptr:{} (child {:.0}x{:.0} -> host {:.0}x{:.0}, scale {:.3}x{:.3})",
            host_parent_id,
            child_w,
            child_h,
            host_w,
            host_h,
            sx,
            sy,
        );
        return None;
    }
    if (sx - 1.0).abs() < 0.0001 && (sy - 1.0).abs() < 0.0001 {
        return None;
    }
    Some((sx, sy))
}

fn scale_node_from_child_canvas(node: &mut crate::bb_scene::BbNode, sx: f32, sy: f32) {
    node.position.x *= sx;
    node.position.y *= sy;
    node.position_offset.x *= sx;
    node.position_offset.y *= sy;
    scale_bb_value(&mut node.sizing.width, sx);
    scale_bb_value(&mut node.sizing.height, sy);
    node.padding.left *= sx;
    node.padding.right *= sx;
    node.padding.top *= sy;
    node.padding.bottom *= sy;
    node.margin.left *= sx;
    node.margin.right *= sx;
    node.margin.top *= sy;
    node.margin.bottom *= sy;
    if let Some(text) = node.text.as_mut() {
        scale_bb_value(&mut text.font_size, sy);
    }
    if let Some(border) = node.border.as_mut() {
        let sw = sx.min(sy);
        border.top.width *= sw;
        border.right.width *= sw;
        border.bottom.width *= sw;
        border.left.width *= sw;
    }
}

fn scale_bb_value(value: &mut BbValue, scale: f32) {
    if let BbValue::Fixed(v) = value {
        *v *= scale;
    }
}

fn remap_child_operation_ids(
    parent_scene: &BbScene,
    child_ops: &[serde_json::Value],
) -> HashMap<BbNodeId, BbNodeId> {
    let mut occupied: std::collections::BTreeSet<BbNodeId> = std::collections::BTreeSet::new();
    for id in parent_scene.nodes.keys().copied() {
        occupied.insert(id);
    }
    for op in &parent_scene.operations {
        if let Some(id) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("ptr:"))
            .and_then(|n| n.parse::<BbNodeId>().ok())
        {
            occupied.insert(id);
        }
    }

    let mut next: BbNodeId = occupied.iter().next_back().copied().unwrap_or(0).wrapping_add(1);
    let mut out = HashMap::new();
    for op in child_ops {
        let Some(old_id) = op
            .get("_Pointer_")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("ptr:"))
            .and_then(|n| n.parse::<BbNodeId>().ok())
        else {
            continue;
        };
        if out.contains_key(&old_id) {
            continue;
        }
        while occupied.contains(&next) || out.values().any(|&v| v == next) {
            next = next.wrapping_add(1);
        }
        out.insert(old_id, next);
        next = next.wrapping_add(1);
    }
    out
}

fn remap_ptrs_in_json_map(
    v: &mut serde_json::Value,
    id_map: &HashMap<BbNodeId, BbNodeId>,
    op_id_map: &HashMap<BbNodeId, BbNodeId>,
) {
    remap_ptrs_in_json_map_keyed(v, None, id_map, op_id_map);
}

fn remap_ptrs_in_json_map_keyed(
    v: &mut serde_json::Value,
    key: Option<&str>,
    id_map: &HashMap<BbNodeId, BbNodeId>,
    op_id_map: &HashMap<BbNodeId, BbNodeId>,
) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(n) = s.strip_prefix("ptr:").and_then(|n| n.parse::<BbNodeId>().ok()) {
                let remapped = remap_ptr_with_key(n, key, id_map, op_id_map);
                if let Some(new_id) = remapped {
                    *s = format!("ptr:{new_id}");
                }
            } else if let Some(n) = s
                .strip_prefix("_PointsTo_:ptr:")
                .and_then(|n| n.parse::<BbNodeId>().ok())
            {
                let remapped = remap_ptr_with_key(n, key, id_map, op_id_map);
                if let Some(new_id) = remapped {
                    *s = format!("_PointsTo_:ptr:{new_id}");
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                remap_ptrs_in_json_map_keyed(item, key, id_map, op_id_map);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, value) in map {
                remap_ptrs_in_json_map_keyed(value, Some(k.as_str()), id_map, op_id_map);
            }
        }
        _ => {}
    }
}

fn remap_ptr_with_key(
    old: BbNodeId,
    key: Option<&str>,
    id_map: &HashMap<BbNodeId, BbNodeId>,
    op_id_map: &HashMap<BbNodeId, BbNodeId>,
) -> Option<BbNodeId> {
    let k = key.unwrap_or("");
    let prefer_node = matches!(k, "widget" | "parent" | "target");
    let prefer_op = k == "_Pointer_" || k.starts_with("input") || k == "nZeros";

    if prefer_node {
        if let Some(&n) = id_map.get(&old) {
            return Some(n);
        }
        if let Some(&n) = op_id_map.get(&old) {
            return Some(n);
        }
        return None;
    }
    if prefer_op {
        if let Some(&n) = op_id_map.get(&old) {
            return Some(n);
        }
        if let Some(&n) = id_map.get(&old) {
            return Some(n);
        }
        return None;
    }
    op_id_map.get(&old).copied().or_else(|| id_map.get(&old).copied())
}

fn find_host_parent(parent_scene: &BbScene, match_to: &str) -> Option<BbNodeId> {
    if !match_to.is_empty() {
        // Explicit matchTo: find by name among root children.
        for root_id in &parent_scene.roots {
            let Some(root) = parent_scene.nodes.get(root_id) else {
                continue;
            };
            for child_id in &root.children {
                let Some(child) = parent_scene.nodes.get(child_id) else {
                    continue;
                };
                if child.name.eq_ignore_ascii_case(match_to) {
                    return Some(*child_id);
                }
            }
        }
    }

    // When match_to is empty (no explicit host specified), look for a scene
    // node that carries exactly two style tags — this is the pattern for
    // canvas content slots like `canvas_TargetStatus` which carries both a
    // content-type tag (e.g. TargetStatus) and a UI-generic flag tag
    // (e.g. UI_Generic_Flag_01).  These two-tag nodes are the primary content
    // slots that should receive merged child canvas content.
    //
    // The node is a direct child of the root (parent points to a root ID),
    // not a root itself.  If multiple two-tag candidates exist, prefer the
    // one whose name starts with `canvas_` (the primary content canvas slot).
    let root_ids: std::collections::HashSet<BbNodeId> =
        parent_scene.roots.iter().copied().collect();
    let mut two_tag_candidates: Vec<BbNodeId> = Vec::new();
    for (id, node) in &parent_scene.nodes {
        if node.style_tag_uuids.len() == 2 {
            // Direct child of root: parent is Some and points to a root ID.
            let is_root_child = node.parent
                .map(|p| root_ids.contains(&p))
                .unwrap_or(false);
            if is_root_child {
                two_tag_candidates.push(*id);
            }
        }
    }
    if two_tag_candidates.len() == 1 {
        return two_tag_candidates.into_iter().next();
    }
    // Multiple two-tag candidates: prefer the one whose name starts with
    // `canvas_` — these are the primary content canvas slots.
    if two_tag_candidates.len() > 1 {
        for &id in &two_tag_candidates {
            if let Some(node) = parent_scene.nodes.get(&id) {
                if node.name.starts_with("canvas_") {
                    return Some(id);
                }
            }
        }
    }

    parent_scene.roots.first().copied()
}

fn param_inputs_for_match_to(
    scene: &BbScene,
    match_to: &str,
) -> Option<Vec<serde_json::Value>> {
    for root_id in &scene.roots {
        let Some(root) = scene.nodes.get(root_id) else {
            continue;
        };
        for child_id in &root.children {
            let Some(child) = scene.nodes.get(child_id) else {
                continue;
            };
            if !child.name.eq_ignore_ascii_case(match_to) {
                continue;
            }
            let inputs = child
                .raw
                .get("paramInputValues")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if !inputs.is_empty() {
                return Some(inputs);
            }
        }
    }
    None
}

fn host_node_for_match_to(scene: &BbScene, match_to: &str) -> Option<BbNodeId> {
    if match_to.is_empty() {
        return None;
    }

    for root_id in &scene.roots {
        let Some(root) = scene.nodes.get(root_id) else {
            continue;
        };
        for child_id in &root.children {
            let Some(child) = scene.nodes.get(child_id) else {
                continue;
            };
            if child.name.eq_ignore_ascii_case(match_to) {
                return Some(*child_id);
            }
        }
    }

    None
}

/// Host WidgetCanvas for a Pass-1 canvas-reference entry without a `matchTo`
/// name: the entry's style conditions select the host (the power master's
/// `Set Canvas` entry matches type `Canvas` + a brand tag on
/// `canvas_Interchangeable`). Returns the first matching WidgetCanvas node.
fn host_canvas_node_for_entry(scene: &BbScene, entry: &serde_json::Value) -> Option<BbNodeId> {
    let mut ids: Vec<BbNodeId> = scene
        .nodes
        .iter()
        .filter(|(_, node)| node.ty == BbNodeType::WidgetCanvas)
        .map(|(&id, _)| id)
        .collect();
    ids.sort_unstable();
    ids.into_iter().find(|&id| {
        scene
            .nodes
            .get(&id)
            .is_some_and(|node| crate::bb_brand_apply::entry_matches_scene(entry, id, node, scene))
    })
}

/// A WidgetCanvas slot is a sub-full TILING panel when it does NOT fill its
/// parent on at least one axis (`Percent < 1.0`). Such slots are laid out
/// alongside their siblings rather than overlaid: the LR-indicator master
/// (`HC_HUD_Ship_LRInd_Master`) tiles a left and a right `width = 0.5 Percent`
/// column, both visible at once, so every matching `Set Canvas` entry must fill
/// its own slot. A slot sized to fill or over-fill the canvas (`Percent >= 1.0`,
/// or a non-`Percent` behaviour such as `PercentOfY`) is instead a centred
/// OVERLAY mode — `MC_S_Self_Master`'s five mutually-exclusive view modes — which
/// stay mutually exclusive under the at-rest single pick (the counterexample
/// scoping this rule).
fn host_is_subfull_tiling_slot(scene: &BbScene, id: BbNodeId) -> bool {
    let Some(node) = scene.nodes.get(&id) else {
        return false;
    };
    let subfull = |v: &crate::bb_scene::BbValue| {
        matches!(v, crate::bb_scene::BbValue::Percent(p) if *p < 1.0 - 1e-3)
    };
    subfull(&node.sizing.width) || subfull(&node.sizing.height)
}
