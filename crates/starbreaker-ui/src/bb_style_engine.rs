//! The single selector engine for the BuildingBlocks style cascade
//! (plan P4.2; pass inventory: `crates/starbreaker-ui/docs/ui-cascade-passes.md`).
//!
//! A [`StyleSheet`] describes ONE entry container (its cascade [`Tier`],
//! identifier, palette sources, entries, and scope); [`apply`] runs a slice
//! of sheets in order through the SAME application kernel every legacy
//! entry point uses (`bb_brand_apply::apply_style_entries_filtered`), so
//! conditions, modifiers, probes, and the `__InlineFontSize` /
//! `__EntryFontSize` / `__AppliedStyleEntries` marker semantics are reused
//! verbatim. The TEXT-FORMAT route (entries styling a textfield's text
//! format) runs at [`Tier::Brand`] (Parent-wrapped or bare `Type(Text)`
//! entries — the tier carries the semantics the legacy path inferred from
//! the `s_*` identifier prefix) AND, since the LR-indicator arc (ledger
//! 96/97), at [`Tier::Embedded`] for an UNCONDITIONAL bare `Type(Text)`
//! selector (`TextFormatRoute::BareTextOnly`).

use crate::bb_loc::LocFetcher;
use crate::bb_scene::{BbNodeId, BbScene};

/// Cascade tier of a sheet's ORIGIN container, lowest first. Deferred
/// late-state re-application is a [`SheetScope::Subtree`], not a tier: a
/// deferred sheet keeps its origin tier (so e.g. a deferred brand sheet
/// keeps brand-tier semantics), per the P4.2 design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Canvas `style` record link (applied only when no brand resolves).
    StyleLink,
    /// `defaultStyles.sharedStyles` record.
    Shared,
    /// Selected `brandStyles[]` container — the only tier with the
    /// text-format route.
    Brand,
    /// The canvas's `embeddedStyles`.
    Embedded,
    /// Widget-standard module sheets (`sk_<brand>_*styles`) and the
    /// expanded standards' own embedded entries.
    StandardModule,
    /// The empty finishing pass that guarantees node `inlineStyles` apply
    /// on canvases with no other containers.
    Inline,
}

/// What part of the scene a sheet applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetScope {
    /// Every node.
    Scene,
    /// Only the subtree under this root (deferred late-state passes).
    Subtree(BbNodeId),
    /// Only nodes whose `raw` carries this marker key (widget-standard
    /// module sheets, e.g. `_ScrollbarStandard_`).
    Marker(String),
}

/// One entry container, ready to apply.
pub struct StyleSheet<'a> {
    pub tier: Tier,
    /// Probe/marker identifier (brand name, shared record name,
    /// "embeddedStyles", …). Brand-tier sheets also stamp
    /// `__BrandIdentifier` on nodes, as the legacy path did.
    pub identifier: String,
    /// Palette for fill-class colour roles.
    pub fills: &'a serde_json::Value,
    /// Palette for chrome-class colour roles (`PaletteSources` split).
    pub chrome: &'a serde_json::Value,
    pub entries: &'a [serde_json::Value],
    pub scope: SheetScope,
}

impl<'a> StyleSheet<'a> {
    /// A scene-scoped sheet with one palette for both colour classes.
    pub fn uniform(
        tier: Tier,
        identifier: impl Into<String>,
        palette: &'a serde_json::Value,
        entries: &'a [serde_json::Value],
    ) -> Self {
        Self {
            tier,
            identifier: identifier.into(),
            fills: palette,
            chrome: palette,
            entries,
            scope: SheetScope::Scene,
        }
    }
}

/// Apply `sheets` to the scene IN ORDER. Order is the cascade: callers pass
/// sheets lowest-tier first; within a pass the kernel applies matching
/// entries then the node's own `inlineStyles` last (unchanged semantics).
pub fn apply(scene: &mut BbScene, sheets: &[StyleSheet<'_>], loc_fetcher: Option<&dyn LocFetcher>) {
    for sheet in sheets {
        apply_sheet(scene, sheet, loc_fetcher);
    }
}

fn apply_sheet(scene: &mut BbScene, sheet: &StyleSheet<'_>, loc_fetcher: Option<&dyn LocFetcher>) {
    let allowed = match &sheet.scope {
        SheetScope::Subtree(root) => Some(collect_subtree(scene, *root)),
        _ => None,
    };
    let scope_marker = match &sheet.scope {
        SheetScope::Marker(marker) => Some(marker.as_str()),
        _ => None,
    };
    // The text-format route is tier-scoped: the BRAND tier runs the full route
    // (every Parent-wrapped / bare `Type(Text)` entry, conditional overrides
    // included); the STYLELINK, SHARED and EMBEDDED tiers run it ONLY for
    // unconditional bare `Type(Text)` declarations (a canvas-wide text size —
    // e.g. the DRAK LR-indicator's `embeddedStyles` FontSize 100, or a root
    // `defaultStyles` / `mfd_g_*` shared FontSize), so conditional
    // state/overrides stay brand-tier-only. StandardModule (element-instance
    // markers/tags) and Inline (empty finishing pass) carry no canvas-wide bare
    // `Type(Text)` — N/A. The `__BrandIdentifier` stamp is brand-tier-only.
    let text_format_route = match sheet.tier {
        Tier::Brand => crate::bb_brand_apply::TextFormatRoute::Full,
        Tier::StyleLink | Tier::Shared | Tier::Embedded => {
            crate::bb_brand_apply::TextFormatRoute::BareTextOnly
        }
        Tier::StandardModule | Tier::Inline => crate::bb_brand_apply::TextFormatRoute::Off,
    };
    crate::bb_brand_apply::apply_style_entries_for_engine(
        scene,
        sheet.entries,
        sheet.fills,
        sheet.chrome,
        Some(sheet.identifier.as_str()),
        loc_fetcher,
        scope_marker,
        allowed.as_ref(),
        text_format_route,
        sheet.tier == Tier::Brand,
        // A shared/generic sheet (mfd_g_*) is not the styling authority for a
        // custom shape's intrinsic authored fill — suppresses the emissions
        // separator recolour-to-Base; brand/embedded/inline still override.
        sheet.tier == Tier::Shared,
    );
}

fn collect_subtree(scene: &BbScene, root: BbNodeId) -> std::collections::HashSet<BbNodeId> {
    let mut allowed = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !allowed.insert(id) {
            continue;
        }
        if let Some(node) = scene.nodes.get(&id) {
            stack.extend(node.children.iter().copied());
        }
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    fn scene_with_tagged_nodes() -> BbScene {
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "root", "isActive": true,
                     "styleTags": [{"_RecordId_": "aaaa-tag"}]},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "child", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "styleTags": [{"_RecordId_": "aaaa-tag"}]}
                ]
            }
        });
        parse_bb_canvas(&canvas).expect("fixture parses")
    }

    fn alpha_entry(name: &str, value: f64) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "conditionsList": [{
                "conditions": [{
                    "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                    "tag": {"_RecordId_": "aaaa-tag"}
                }]
            }],
            "modifiers": [{
                "_Type_": "BuildingBlocks_FieldModifierNumber",
                "field": "Alpha",
                "value": value
            }]
        })
    }

    fn node_alpha(scene: &BbScene, name: &str) -> f32 {
        scene
            .nodes
            .values()
            .find(|n| n.name == name)
            .expect("node")
            .alpha
    }

    #[test]
    fn sheets_apply_in_order_later_wins() {
        let mut scene = scene_with_tagged_nodes();
        let palette = serde_json::json!({});
        let low = [alpha_entry("low", 0.25)];
        let high = [alpha_entry("high", 0.75)];
        let sheets = [
            StyleSheet::uniform(Tier::Shared, "shared", &palette, &low),
            StyleSheet::uniform(Tier::Embedded, "embeddedStyles", &palette, &high),
        ];
        apply(&mut scene, &sheets, None);
        assert_eq!(node_alpha(&scene, "root"), 0.75, "later sheet wins");
    }

    #[test]
    fn shared_tier_background_color_keeps_custom_shape_authored_colour() {
        // The emissions header separators are WidgetCustomShapes that author
        // `background.color = Accent1/Accent2` (the in-game bars are that red).
        // The shared `mfd_g_emissions` "New Style" entry (BackgroundColor=Base)
        // must NOT recolour them — a generic shared sheet is not the styling
        // authority for a shape's intrinsic fill. A BRAND-tier entry still can.
        let canvas = serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetCustomShape",
                     "name": "sep", "isActive": true,
                     "styleTags": [{"_RecordId_": "aaaa-tag"}],
                     "background": {"_Type_": "BuildingBlocks_Background", "enable": true,
                        "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Accent1", "alpha": 1.0}}}
                ]
            }
        });
        let bg_entry = |token: &str| serde_json::json!({
            "name": "sep colour",
            "conditionsList": [{"conditions": [{
                "_Type_": "BuildingBlocks_StyleSelectorConditionTag",
                "tag": {"_RecordId_": "aaaa-tag"}}]}],
            "modifiers": [{"_Type_": "BuildingBlocks_FieldModifierColor", "field": "BackgroundColor",
                "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": token, "alpha": 1.0}}]
        });
        let palette = serde_json::json!({});
        // Mirror the IR reader (`background_fill_colour_token_from_raw`): a
        // cascade-written `BackgroundColorToken` wins, else the authored
        // `background.color`.
        fn effective_bg(scene: &BbScene) -> Option<String> {
            let node = scene.nodes.values().find(|n| n.name == "sep")?;
            if let Some(token) = node.raw.get("BackgroundColorToken").and_then(|v| v.as_str()) {
                return Some(token.to_owned());
            }
            node.raw.get("background")?.get("color")?.get("color")?.as_str().map(str::to_owned)
        }

        let mut shared_scene = parse_bb_canvas(&canvas).expect("parses");
        let shared = [bg_entry("Base")];
        apply(&mut shared_scene, &[StyleSheet::uniform(Tier::Shared, "mfd_g_emissions", &palette, &shared)], None);
        assert_eq!(effective_bg(&shared_scene).as_deref(), Some("Accent1"),
            "shared-tier BackgroundColor must not override a custom shape's authored colour");

        let mut brand_scene = parse_bb_canvas(&canvas).expect("parses");
        let brand = [bg_entry("Base")];
        apply(&mut brand_scene, &[StyleSheet::uniform(Tier::Brand, "s_drak_hud", &palette, &brand)], None);
        assert_eq!(effective_bg(&brand_scene).as_deref(), Some("Base"),
            "brand-tier BackgroundColor still overrides (a brand CAN restyle the shape)");
    }

    #[test]
    fn subtree_scope_leaves_outside_nodes_untouched() {
        let mut scene = scene_with_tagged_nodes();
        let child_id = *scene
            .nodes
            .iter()
            .find(|(_, n)| n.name == "child")
            .map(|(id, _)| id)
            .expect("child id");
        let palette = serde_json::json!({});
        let entries = [alpha_entry("deferred", 0.5)];
        let sheets = [StyleSheet {
            tier: Tier::Embedded,
            identifier: "deferred-origin".to_string(),
            fills: &palette,
            chrome: &palette,
            entries: &entries,
            scope: SheetScope::Subtree(child_id),
        }];
        apply(&mut scene, &sheets, None);
        assert_eq!(node_alpha(&scene, "child"), 0.5, "subtree node styled");
        assert_eq!(node_alpha(&scene, "root"), 1.0, "outside node untouched");
    }

    #[test]
    fn brand_tier_stamps_brand_identifier_marker() {
        let mut scene = scene_with_tagged_nodes();
        let palette = serde_json::json!({});
        let entries = [alpha_entry("brand", 0.9)];
        let sheets = [StyleSheet::uniform(Tier::Brand, "s_test_brand", &palette, &entries)];
        apply(&mut scene, &sheets, None);
        let root = scene.nodes.values().find(|n| n.name == "root").unwrap();
        assert_eq!(
            root.raw.get("__BrandIdentifier").and_then(|v| v.as_str()),
            Some("s_test_brand"),
            "brand tier stamps the identifier"
        );
    }

    #[test]
    fn non_brand_tier_does_not_stamp_brand_identifier() {
        let mut scene = scene_with_tagged_nodes();
        let palette = serde_json::json!({});
        let entries = [alpha_entry("shared", 0.9)];
        let sheets = [StyleSheet::uniform(Tier::Shared, "mfd_g_content", &palette, &entries)];
        apply(&mut scene, &sheets, None);
        let root = scene.nodes.values().find(|n| n.name == "root").unwrap();
        assert!(root.raw.get("__BrandIdentifier").is_none());
    }

    // --- Embedded-tier text-format route (LR-indicator labels) ---------------
    //
    // The DRAK LR-indicator (`drak_hc_hud_cutlass_lind`/`_rind`) authors its
    // canvas-root `embeddedStyles` "Font Size" as an UNCONDITIONAL bare
    // `Type(Text)` -> FontSize 100; the in-game `lrind_master` reference shows
    // the labels far larger than the `Heading1` (60) fallback, so that
    // canvas-wide text declaration must reach the field's text format. The
    // Brand-tier route alone missed it: that FontSize lives at `Tier::Embedded`
    // (pass 7) and the LR-ind `defaultStyles` is empty, so there is no
    // no-brand-match defaultStyles fallback like velocity-num/master-mode had.
    // The embedded route is SCOPED to unconditional bare `Type(Text)` so the
    // documented embedded state/overrides stay excluded (the target screen's
    // `Bright Elements` = `Parent[Tag]` -> Bright FillColor and the medical
    // bed's `Textfield_BrightColor_Override` are at-rest-absent CONDITIONAL
    // overrides; the brand tier still routes them, the embedded tier must not).

    fn textfield_canvas() -> serde_json::Value {
        serde_json::json!({
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_Canvas",
                "size": {"x": 100.0, "y": 100.0},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "label", "isActive": true,
                     "styleTags": [{"_RecordId_": "tagT"}]}
                ]
            }
        })
    }

    fn bare_text_fontsize(value: f64) -> serde_json::Value {
        serde_json::json!({
            "name": "Font Size",
            "conditionsList": [{"conditions": [
                {"_Type_": "BuildingBlocks_StyleSelectorConditionAllOfCondition",
                 "conditions": [{"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Text"}]}
            ]}],
            "modifiers": [{"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": value}]
        })
    }

    // AllOf[Type(Text), Tag(tagT)] — a CONDITIONAL text declaration standing in
    // for the target/medbed Bright overrides. The field carries `tagT`, so the
    // brand tier routes it; the embedded tier must not (it is not unconditional).
    fn conditional_text_fontsize(value: f64) -> serde_json::Value {
        serde_json::json!({
            "name": "Conditional Text",
            "conditionsList": [{"conditions": [
                {"_Type_": "BuildingBlocks_StyleSelectorConditionAllOfCondition",
                 "conditions": [
                    {"_Type_": "BuildingBlocks_StyleSelectorConditionType", "type": "Text"},
                    {"_Type_": "BuildingBlocks_StyleSelectorConditionTag", "tag": {"_RecordId_": "tagT"}}
                 ]}
            ]}],
            "modifiers": [{"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "FontSize", "value": value}]
        })
    }

    fn label_fontsize(scene: &BbScene) -> Option<f64> {
        scene.nodes.values().find(|n| n.name == "label")?.raw.get("FontSize").and_then(|v| v.as_f64())
    }

    #[test]
    fn embedded_tier_routes_unconditional_bare_text_fontsize() {
        let palette = serde_json::json!({});
        let mut scene = parse_bb_canvas(&textfield_canvas()).expect("parses");
        let size = [bare_text_fontsize(100.0)];
        apply(&mut scene, &[StyleSheet::uniform(Tier::Embedded, "embeddedStyles", &palette, &size)], None);
        assert_eq!(label_fontsize(&scene), Some(100.0),
            "embedded bare Type(Text) FontSize must reach the field's text format (LR-indicator labels)");
    }

    #[test]
    fn embedded_tier_excludes_conditional_text_override_but_brand_keeps_it() {
        let palette = serde_json::json!({});
        let cond = [conditional_text_fontsize(99.0)];
        let mut emb = parse_bb_canvas(&textfield_canvas()).expect("parses");
        apply(&mut emb, &[StyleSheet::uniform(Tier::Embedded, "embeddedStyles", &palette, &cond)], None);
        assert_eq!(label_fontsize(&emb), None,
            "a CONDITIONAL embedded text entry must NOT take the embedded text-format route \
             (target `Bright Elements` / medbed override stay excluded)");
        let mut brand = parse_bb_canvas(&textfield_canvas()).expect("parses");
        apply(&mut brand, &[StyleSheet::uniform(Tier::Brand, "s_drak_hud", &palette, &cond)], None);
        assert_eq!(label_fontsize(&brand), Some(99.0),
            "the brand tier still routes a conditional Type(Text) entry (unchanged)");
    }

    // --- StyleLink / Shared text-format route (widened per plan B2-3) --------
    //
    // StyleLink (pass 4, canvas `style` link applied only when no brand) and
    // Shared (pass 5, `defaultStyles.sharedStyles` — `mfd_g_*`) join Embedded on
    // the bare-only route: an UNCONDITIONAL bare `Type(Text)` FontSize is a
    // canvas-wide text size and must reach the field, while CONDITIONAL
    // selectors stay brand-tier-only (same discriminator as Embedded).

    #[test]
    fn stylelink_tier_routes_unconditional_bare_text_fontsize() {
        let palette = serde_json::json!({});
        let mut scene = parse_bb_canvas(&textfield_canvas()).expect("parses");
        let size = [bare_text_fontsize(100.0)];
        apply(&mut scene, &[StyleSheet::uniform(Tier::StyleLink, "linked_style", &palette, &size)], None);
        assert_eq!(label_fontsize(&scene), Some(100.0),
            "style-link bare Type(Text) FontSize must reach the field's text format");
    }

    #[test]
    fn stylelink_tier_excludes_conditional_text_override_but_brand_keeps_it() {
        let palette = serde_json::json!({});
        let cond = [conditional_text_fontsize(99.0)];
        let mut sl = parse_bb_canvas(&textfield_canvas()).expect("parses");
        apply(&mut sl, &[StyleSheet::uniform(Tier::StyleLink, "linked_style", &palette, &cond)], None);
        assert_eq!(label_fontsize(&sl), None,
            "a CONDITIONAL style-link text entry must NOT take the route (stays brand-only)");
    }

    #[test]
    fn shared_tier_routes_unconditional_bare_text_fontsize() {
        let palette = serde_json::json!({});
        let mut scene = parse_bb_canvas(&textfield_canvas()).expect("parses");
        let size = [bare_text_fontsize(100.0)];
        apply(&mut scene, &[StyleSheet::uniform(Tier::Shared, "mfd_g_content", &palette, &size)], None);
        assert_eq!(label_fontsize(&scene), Some(100.0),
            "shared-style bare Type(Text) FontSize must reach the field's text format");
    }

    #[test]
    fn shared_tier_excludes_conditional_text_override_but_brand_keeps_it() {
        let palette = serde_json::json!({});
        let cond = [conditional_text_fontsize(99.0)];
        let mut sh = parse_bb_canvas(&textfield_canvas()).expect("parses");
        apply(&mut sh, &[StyleSheet::uniform(Tier::Shared, "mfd_g_content", &palette, &cond)], None);
        assert_eq!(label_fontsize(&sh), None,
            "a CONDITIONAL shared text entry must NOT take the route (stays brand-only)");
    }
}
