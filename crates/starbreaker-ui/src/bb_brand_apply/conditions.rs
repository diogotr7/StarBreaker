//! Style-entry condition matching: which scene nodes a brand/canvas
//! style entry applies to. Owns `entry_matches_scene` (entry-level
//! gate used by bb_resolve host selection), `condition_matches_node`
//! (tag / type / ancestor / interaction-state selectors, including the
//! break-bounded ancestor-walk `breakConditions` semantics), and the tag
//! and node-type reference helpers.

use super::*;

/// Test whether a brand-style entry matches a node within a scene.
///
/// An entry matches when:
/// - Its `conditionsList` is absent or empty (unconditional), OR
/// - There exists at least one `conditionsList[i]` such that **all**
///   `conditions[j]` items pass. Conditions may be nested (`AllOf`, `AnyOf`,
///   `Parent`), and parent conditions are evaluated against the node's direct
///   parent in the parsed BB scene hierarchy.
pub fn entry_matches_scene(
    entry: &serde_json::Value,
    node_id: BbNodeId,
    node: &BbNode,
    scene: &BbScene,
) -> bool {
    let conditions_list = match entry.get("conditionsList").and_then(|v| v.as_array()) {
        Some(cl) => cl,
        None => return true,
    };

    if conditions_list.is_empty() {
        return true;
    }

    conditions_list.iter().any(|conditions_block| {
        let Some(conditions) = conditions_block.get("conditions").and_then(|v| v.as_array()) else {
            return false;
        };
        conditions
            .iter()
            .all(|condition| condition_matches_node(condition, node_id, node, scene))
    })
}

pub(crate) fn condition_matches_node(
    condition: &serde_json::Value,
    node_id: BbNodeId,
    node: &BbNode,
    scene: &BbScene,
) -> bool {
    let cond_type = condition
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if cond_type.ends_with("ConditionAllOfCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions
                    .iter()
                    .all(|child| condition_matches_node(child, node_id, node, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionAnyOfCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions
                    .iter()
                    .any(|child| condition_matches_node(child, node_id, node, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionNotCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                !conditions
                    .iter()
                    .any(|child| condition_matches_node(child, node_id, node, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionParent") {
        let Some(parent_id) = node.parent else {
            return false;
        };
        let Some(parent) = scene.nodes.get(&parent_id) else {
            return false;
        };
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions
                    .iter()
                    .all(|child| condition_matches_node(child, parent_id, parent, scene))
            })
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionAncestor") {
        let Some(conditions) = condition.get("conditions").and_then(|v| v.as_array()) else {
            return false;
        };
        // `breakConditions` bound the walk EVERYWHERE: the boundary node is
        // tested match-before-break (a materialised pip carries both the
        // `general-list-item` break tag and its own state tag; the
        // annunciator chiclet's Item boundary carries the queried online
        // state for "Show Glow"), and positive conditions beyond the
        // boundary never match (an Unpowered pip's fill must not match the
        // column root's Powered tag; the glow must not match a same-tagged
        // ancestor outside its chiclet). An EMPTY conditions list with
        // breaks is a CONTAINMENT test: true only when an ancestor matches
        // the breaks (the pip selector arrow's entry requires living inside
        // the Secondary-tagged `PipBox_Selecter` slot; the touch-here
        // fingerprint's "ChangeSizeForASOP" requires an ASOP-host-tagged
        // ancestor and must NOT resize the medical at-rest fingerprint).
        let break_conditions = condition
            .get("breakConditions")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        let scoped_breaks = !break_conditions.is_empty();
        if conditions.is_empty() && !scoped_breaks {
            // Legacy semantics: an empty conditions list (and no breaks)
            // matches at the first resolvable ancestor (a parentless node
            // has none and fails).
            return node
                .parent
                .is_some_and(|parent| scene.nodes.contains_key(&parent));
        }
        let mut current = node.parent;
        while let Some(ancestor_id) = current {
            let Some(ancestor) = scene.nodes.get(&ancestor_id) else {
                break;
            };
            if !conditions.is_empty()
                && conditions
                    .iter()
                    .all(|child| condition_matches_node(child, ancestor_id, ancestor, scene))
            {
                return true;
            }
            if scoped_breaks
                && break_conditions
                    .iter()
                    .all(|child| condition_matches_node(child, ancestor_id, ancestor, scene))
            {
                // The walk stops at the boundary: positive conditions beyond
                // it never match; an empty-conditions containment test is
                // satisfied by reaching it.
                return conditions.is_empty();
            }
            current = ancestor.parent;
        }
        return false;
    }

    if cond_type.ends_with("ConditionAnyOfTag") {
        let Some(tags) = condition.get("tags").and_then(|v| v.as_array()) else {
            return false;
        };
        return tags.iter().filter_map(tag_ref_id).any(|tag_id| {
            node.style_tag_uuids
                .iter()
                .any(|node_tag| node_tag == tag_id)
        });
    }

    // ALL listed tags must be present (e.g. `BG_Warning` requires every warning
    // state tag). An empty/unresolvable `tags` list does NOT match — a condition
    // that requires "all of nothing" must not match every node (that over-matches
    // and reveals state-gated elements like the annunciator's ON gradient).
    if cond_type.ends_with("ConditionAllOfTag") {
        let Some(tags) = condition.get("tags").and_then(|v| v.as_array()) else {
            return false;
        };
        let ids: Vec<&str> = tags.iter().filter_map(tag_ref_id).collect();
        return !ids.is_empty()
            && ids
                .iter()
                .all(|tag_id| node.style_tag_uuids.iter().any(|node_tag| node_tag == *tag_id));
    }

    // The tag must be ABSENT (e.g. `BG_Neutral` hides the warning chrome with
    // `NotTag(warning-active)` when no warning state is active). This MUST be
    // checked before the `condition.get("tag").is_some()` catch below, because a
    // `NotTag` also carries a `tag` field and would otherwise be mis-evaluated as
    // a (presence) `ConditionTag` — the inverse of its meaning. An unresolvable
    // tag ref does NOT match (conservative — avoid over-matching).
    if cond_type.ends_with("ConditionNotTag") {
        return condition_tag_id(condition)
            .map(|tag_id| !node.style_tag_uuids.iter().any(|tag| tag == tag_id))
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionTag") || condition.get("tag").is_some() {
        return condition_tag_id(condition)
            .map(|tag_id| node.style_tag_uuids.iter().any(|tag| tag == tag_id))
            .unwrap_or(false);
    }

    if cond_type.ends_with("ConditionType") {
        return condition
            .get("type")
            .and_then(|v| v.as_str())
            .map(|type_str| node_type_matches(type_str, &node.ty))
            .unwrap_or(true);
    }

    false
}

/// Test whether an entry styles a textfield's IMPLICIT TEXT-FORMAT CHILD.
///
/// The engine renders a `WidgetTextField`'s text as a child element of the
/// widget, so an entry whose conditions are `Parent(...)`-wrapped selects that
/// child through the tagged field itself: `Parent(Tag(Size_3))` sizes the text
/// of `Size_3`-tagged textfields (the MFD content host's per-brand
/// `FontSizeSmall` table and the power card's `Battery Powered/Depleted Text`
/// entries, verified against the in-game power reference). Evaluation frame:
/// `Parent(c)` tests `c` on the field itself; tag conditions see the field's
/// (inherited) tags; a `Type(Text)` condition matches the implicit text-format
/// child (a `Text` node) UNLESS the conditions block is `Ancestor(...)`-wrapped
/// without a `Parent` anchor, which targets a real WidgetText widget instead
/// (the MFD footer's `Type(Text) + Ancestor[Tag]` screen-name entries do not
/// restyle the WidgetTextField in the in-game reference). Both a `Parent`-anchored
/// `Type(Text)` (DRAK velocity-num's `Type(Text) + Parent[(Not)Tag]` readouts,
/// FontSize 500/420) and a BARE `Type(Text)` (DRAK master-mode's `defaultStyles`
/// "New Style", FontSize 350 + white SCM/GUN) route to the field's text format.
/// Callers apply only TEXT-FORMAT modifiers for a match made via this route.
pub fn entry_matches_text_format(
    entry: &serde_json::Value,
    node_id: BbNodeId,
    node: &BbNode,
    scene: &BbScene,
) -> bool {
    if !matches!(node.ty, BbNodeType::WidgetTextField) {
        return false;
    }
    let Some(conditions_list) = entry.get("conditionsList").and_then(|v| v.as_array()) else {
        return false;
    };
    conditions_list.iter().any(|conditions_block| {
        let Some(conditions) = conditions_block.get("conditions").and_then(|v| v.as_array()) else {
            return false;
        };
        // A `Type(Text)` selector routes to the text-format child unless the
        // block is `Ancestor(...)`-wrapped without a `Parent` anchor — that is
        // the MFD-footer screen-name case, which targets a real WidgetText
        // widget instead. A `Parent(...)`-anchored block (velocity-num) and a
        // BARE `Type(Text)` (master-mode SCM/GUN) both route (see
        // `condition_matches_text_format`).
        let entry_has_parent = conditions.iter().any(condition_tree_contains_parent);
        let entry_has_ancestor = conditions.iter().any(condition_tree_contains_ancestor);
        let route_text_format = entry_has_parent || !entry_has_ancestor;
        !conditions.is_empty()
            && conditions.iter().all(|condition| {
                condition_matches_text_format(condition, node_id, node, scene, route_text_format)
            })
    })
}

/// Whether a condition tree contains a `Parent(...)` wrapper — the anchor that
/// routes a `Type(Text)` selector to a WidgetTextField's text-format child (vs
/// an `Ancestor`-wrapped or bare `Type(Text)`, which targets a real WidgetText
/// widget). Descends only the boolean combinators (AllOf/AnyOf/Not); a `Parent`
/// nested inside an `Ancestor`/`Parent` condition list is scoped to THAT node,
/// not the field, so it does not count.
fn condition_tree_contains_parent(condition: &serde_json::Value) -> bool {
    let cond_type = condition
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if cond_type.ends_with("ConditionParent") {
        return true;
    }
    if cond_type.ends_with("ConditionAllOfCondition")
        || cond_type.ends_with("ConditionAnyOfCondition")
        || cond_type.ends_with("ConditionNotCondition")
    {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .is_some_and(|conds| conds.iter().any(condition_tree_contains_parent));
    }
    false
}

/// Whether a condition tree contains an `Ancestor(...)` wrapper. An
/// `Ancestor`-wrapped `Type(Text)` (without a `Parent` anchor) is the MFD-footer
/// screen-name case: it targets a real WidgetText widget, NOT a WidgetTextField's
/// text-format child, so it is excluded from the text-format route. Descends only
/// the boolean combinators (AllOf/AnyOf/Not), like `condition_tree_contains_parent`.
fn condition_tree_contains_ancestor(condition: &serde_json::Value) -> bool {
    let cond_type = condition
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if cond_type.ends_with("ConditionAncestor") {
        return true;
    }
    if cond_type.ends_with("ConditionAllOfCondition")
        || cond_type.ends_with("ConditionAnyOfCondition")
        || cond_type.ends_with("ConditionNotCondition")
    {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .is_some_and(|conds| conds.iter().any(condition_tree_contains_ancestor));
    }
    false
}

/// True iff every leaf of this condition tree is `Type(Text)` and the tree
/// contains at least one such leaf — only `Type(Text)`, optionally nested in
/// `AllOf`/`AnyOf`, with no other selector kind. Any `Tag`/`NotTag`/`Parent`/
/// `Ancestor`/`Not`/interaction condition or a non-`Text` `Type(...)` makes it
/// non-bare.
fn condition_is_only_text(condition: &serde_json::Value) -> bool {
    let cond_type = condition
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if cond_type.ends_with("ConditionType") {
        return condition.get("type").and_then(|v| v.as_str()) == Some("Text");
    }
    if cond_type.ends_with("ConditionAllOfCondition") || cond_type.ends_with("ConditionAnyOfCondition")
    {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .is_some_and(|conds| !conds.is_empty() && conds.iter().all(condition_is_only_text));
    }
    false
}

/// True when an entry's selector is an UNCONDITIONAL bare `Type(Text)`
/// declaration — every condition (across every conditions block) is
/// `Type(Text)`, optionally wrapped in `AllOf`/`AnyOf`, with NO `Tag`/`NotTag`/
/// `Parent`/`Ancestor`/`Not`/interaction qualifier. This is the engine's
/// canvas-wide "all text is size N / colour C" style declaration — the DRAK
/// LR-indicator's `embeddedStyles` "Font Size" (-> FontSize 100, verified
/// against `lrind_master`). It is the gate for the text-format route OUTSIDE
/// the brand tier: a CONDITIONAL embedded entry (the target screen's
/// `Bright Elements` `Parent[Tag]` Bright override, the medical bed's
/// `Textfield_BrightColor_Override`) is a state/selection override the at-rest
/// capture does not show, so it must keep to the brand tier only (where the
/// full route already runs).
pub(crate) fn entry_is_unconditional_bare_text_selector(entry: &serde_json::Value) -> bool {
    let Some(list) = entry.get("conditionsList").and_then(|v| v.as_array()) else {
        return false;
    };
    !list.is_empty()
        && list.iter().all(|block| {
            block
                .get("conditions")
                .and_then(|v| v.as_array())
                .is_some_and(|conds| !conds.is_empty() && conds.iter().all(condition_is_only_text))
        })
}

fn condition_matches_text_format(
    condition: &serde_json::Value,
    node_id: BbNodeId,
    node: &BbNode,
    scene: &BbScene,
    route_text_format: bool,
) -> bool {
    let cond_type = condition
        .get("_Type_")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if cond_type.ends_with("ConditionParent") {
        // The text-format child's parent IS the textfield widget.
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions
                    .iter()
                    .all(|child| condition_matches_node(child, node_id, node, scene))
            })
            .unwrap_or(false);
    }
    if cond_type.ends_with("ConditionAllOfCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions.iter().all(|child| {
                    condition_matches_text_format(child, node_id, node, scene, route_text_format)
                })
            })
            .unwrap_or(false);
    }
    if cond_type.ends_with("ConditionAnyOfCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                conditions.iter().any(|child| {
                    condition_matches_text_format(child, node_id, node, scene, route_text_format)
                })
            })
            .unwrap_or(false);
    }
    if cond_type.ends_with("ConditionNotCondition") {
        return condition
            .get("conditions")
            .and_then(|v| v.as_array())
            .map(|conditions| {
                !conditions.iter().any(|child| {
                    condition_matches_text_format(child, node_id, node, scene, route_text_format)
                })
            })
            .unwrap_or(false);
    }
    if cond_type.ends_with("ConditionType") {
        // A WidgetTextField renders its text via an implicit text-format CHILD
        // that is itself a `Text` node, so a `Type(Text)` selector matches that
        // child whenever `route_text_format` holds — i.e. a `Parent(...)`-anchored
        // entry (DRAK velocity-num readouts, `Type(Text) + Parent[(Not)Tag]` →
        // FontSize 500/420) OR a BARE `Type(Text)` (DRAK master-mode SCM/GUN,
        // `defaultStyles` "New Style" → FontSize 350 + white). Only an
        // `Ancestor(...)`-wrapped `Type(Text)` (NOT also `Parent`-anchored)
        // targets a real WidgetText widget instead (the MFD footer's
        // `Type(Text) + Ancestor[Tag]` screen-name entries must NOT restyle the
        // WidgetTextField). Other widget types are not the text run, so they
        // never match here.
        return route_text_format
            && condition.get("type").and_then(|v| v.as_str()) == Some("Text");
    }
    // Tag / NotTag / Ancestor / interaction conditions share the widget's
    // context (the text child inherits its field's tags and ancestry).
    condition_matches_node(condition, node_id, node, scene)
}

/// Modifier fields that style a textfield's text format (the subset an entry
/// matched via [`entry_matches_text_format`] may apply).
pub(crate) fn is_text_format_modifier(modifier: &serde_json::Value) -> bool {
    let type_str = modifier
        .get("_Type_")
        .and_then(|v| v.as_str())
        .or_else(|| {
            modifier
                .get("field")
                .and_then(|f| f.get("_Type_"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    if type_str.ends_with("FontStyleRecord") {
        return true;
    }
    let field = modifier
        .get("field")
        .and_then(|f| f.as_str().or_else(|| f.get("field").and_then(|v| v.as_str())))
        .unwrap_or("");
    matches!(
        field,
        "FontSize" | "AutoFontSize" | "FillColor" | "StrokeColor" | "LetterSpacing" | "LineSpacing"
    )
}

pub(crate) fn condition_tag_id(condition: &serde_json::Value) -> Option<&str> {
    let tag = condition.get("tag")?;
    tag_ref_id(tag)
}

pub(crate) fn tag_ref_id(tag: &serde_json::Value) -> Option<&str> {
    tag.get("_RecordId_")
        .and_then(|v| v.as_str())
        .or_else(|| tag.as_str())
}

/// Return `true` when `type_str` from a `ConditionType` entry matches the node type.
///
/// Maps the game's short widget-family names (e.g. `"Image"`) to our `BbNodeType`
/// variants.  Unknown type strings return `false`.
pub(crate) fn node_type_matches(type_str: &str, ty: &BbNodeType) -> bool {
    match type_str {
        "Image" => matches!(ty, BbNodeType::WidgetImage),
        // The game's `Text` widget type is the plain `WidgetText`, NOT a text
        // FIELD: the MFD footer's `TextSpacing` (LetterSpacing 5) and
        // `SelectedName`/`UnSelectedName` entries are all `Type(Text)` and none
        // of their effects appear on the (WidgetTextField) screen-name text in
        // the in-game reference — its tracking and colour come from the brand
        // H1 table instead.
        "Text" => matches!(ty, BbNodeType::WidgetText),
        "TextField" => matches!(ty, BbNodeType::WidgetTextField),
        "Canvas" => matches!(ty, BbNodeType::WidgetCanvas),
        "Icon" => matches!(ty, BbNodeType::WidgetIcon),
        "Card" => matches!(ty, BbNodeType::WidgetCard),
        // The game's `Base` widget type is the base display widget — it matches
        // `DisplayWidget` nodes (e.g. the footer's `base_BG`, which the
        // `ScreenNameBackground` style gates on `AllOf(Type=Base, Tag …)`).
        "Base" | "DisplayWidget" => matches!(ty, BbNodeType::DisplayWidget),
        "CustomShape" => matches!(ty, BbNodeType::WidgetCustomShape),
        "Polygon" => matches!(ty, BbNodeType::Other(kind)
            if kind.eq_ignore_ascii_case("BuildingBlocks_WidgetPolygon")),
        _ => false,
    }
}
