//! Generic element-presence guard derived from the frozen IR snapshots.
//!
//! # Why this exists
//!
//! The whole-image colour regression guard
//! (`manifest_targets_whole_image_colour_regression_guard`) fails only when more
//! than a tier-dependent *fraction* of pixels drift (platinum 0.5%, gold 1%). An
//! element that occupies fewer pixels than that budget can vanish entirely and
//! stay under the threshold — the guard is structurally blind to it. This is the
//! ledger-77 class: nav arrows silently disappeared under a GREEN whole-image
//! guard and the owner caught it by eye (also 106/107).
//!
//! This guard closes that class *generically*: the expectation is derived from
//! each frozen target's OWN IR snapshot fixture — there is no hand-authored
//! per-screen element list. Adding a screen to the freeze extends coverage
//! automatically, exactly like the whole-image guard.
//!
//! # Definitions (locked against `ir-freeze-schema.md` + the snapshot schema)
//!
//! - **Stable node identifier**: `UiSnapshotElement.identity`, formatted by
//!   `snapshot_from_ui_ir` as `"{node.id}:{node.node_type}"`. This is the key the
//!   guard matches frozen elements to live elements on.
//! - **Non-degenerate rect**: `w > 0.0 && h > 0.0` on the element's
//!   `computed_rect`-derived `w`/`h`. A zero-width or zero-height rect draws
//!   nothing, so a degenerate element is not a "present, visible" element.
//! - **Drawable payload**: intrinsic to being in the snapshot at all. The
//!   snapshot capture (`snapshot_from_ui_ir`) only emits nodes that
//!   `classify_node` accepts — a node with a text payload (`Text`), an image
//!   asset reference (`Image`), or a custom shape (`Shape`). A frozen
//!   `UiSnapshotElement` therefore always carries a drawable payload; the guard
//!   needs no separate payload filter beyond presence in the frozen element
//!   list. This keeps the definition structural, not screen-specific.
//!
//! # The invariant
//!
//! For every frozen target, every element in the frozen IR snapshot that has a
//! non-degenerate rect (and thus, per above, a drawable payload) must still
//! exist in the freshly compiled live IR with a non-degenerate rect. A missing
//! identity or a live rect that collapsed to degenerate is a failure naming the
//! target and the node path.
//!
//! Frozen-target enumeration, the live-IR input source, the tier, and the
//! skip-when-no-game-data behaviour are shared verbatim with
//! `manifest_live_ir_guard` via [`live_ir_harness`] — this guard mirrors that one
//! exactly, it does not re-implement the loader.

use std::collections::HashMap;

use starbreaker_ui::{
    UiScreenSnapshot, UiSnapshotElement, UiSnapshotElementCategory, snapshot_from_ui_ir,
};

mod live_ir_harness;
use live_ir_harness::load_live_target_cases;

/// A rect draws nothing unless both dimensions are strictly positive.
fn is_non_degenerate(element: &UiSnapshotElement) -> bool {
    element.w > 0.0 && element.h > 0.0
}

/// For one frozen target: the drawable, non-degenerately-sized elements present
/// in the frozen snapshot that are missing (identity absent) or collapsed
/// (present but degenerate rect) in the freshly compiled live snapshot.
///
/// An empty result means the target preserves every element it froze. Degenerate
/// frozen elements are skipped — nothing was drawn there, so nothing is required.
fn missing_or_collapsed_elements(
    baseline: &UiScreenSnapshot,
    live: &UiScreenSnapshot,
) -> Vec<String> {
    let live_by_identity: HashMap<&str, &UiSnapshotElement> = live
        .elements
        .iter()
        .map(|element| (element.identity.as_str(), element))
        .collect();

    let mut failures = Vec::new();
    for base in &baseline.elements {
        // Frozen-side filter: only nodes that were actually drawn when frozen.
        // A drawable payload is intrinsic (snapshot_from_ui_ir only emits
        // classified Text/Image/Shape nodes), so presence in this list already
        // means "drawable" — the rect check is the only additional gate.
        if !is_non_degenerate(base) {
            continue;
        }
        match live_by_identity.get(base.identity.as_str()) {
            None => failures.push(format!(
                "{} ({:?}, frozen rect {:.2}x{:.2}) was drawn in the frozen IR snapshot but is ABSENT from live IR",
                base.identity, base.category, base.w, base.h
            )),
            Some(live_element) if !is_non_degenerate(live_element) => failures.push(format!(
                "{} ({:?}) is present in live IR but COLLAPSED to a degenerate rect {:.2}x{:.2} (frozen {:.2}x{:.2})",
                base.identity, base.category, live_element.w, live_element.h, base.w, base.h
            )),
            Some(_) => {}
        }
    }
    failures
}

/// A visibly-synthetic snapshot element for the failure-path unit test. Only
/// `identity` and the rect (`w`/`h`) are meaningful to the guard; every
/// game-data-bearing field (colours, fonts, tokens) is `None`/neutral so this
/// carries no copied game values.
fn syn_element(identity: &str, w: f32, h: f32) -> UiSnapshotElement {
    UiSnapshotElement {
        identity: identity.to_string(),
        node_id: 0,
        category: UiSnapshotElementCategory::Shape,
        draw_order_index: 0,
        node_type: "synthetic".to_string(),
        visible: true,
        x: 0.0,
        y: 0.0,
        w,
        h,
        alpha: 1.0,
        blend_mode: None,
        asset_identity: None,
        alignment: None,
        vertical_alignment: None,
        overflow_mode: None,
        background_rgba: None,
        background_tint_token: None,
        stroke_rgba: None,
        stroke_tint_token: None,
        text_rgba: None,
        text_tint_token: None,
        icon_tint_rgba: None,
        icon_tint_token: None,
        stroke_extent: None,
        text_payload: None,
        text_font_identity: None,
        text_font_size: None,
        line_spacing: None,
        primary_text_top: None,
        primary_text_left: None,
        secondary_text_top: None,
        secondary_text_left: None,
    }
}

fn syn_screen(elements: Vec<UiSnapshotElement>) -> UiScreenSnapshot {
    UiScreenSnapshot {
        schema_version: 2,
        canvas_guid: "synthetic".to_string(),
        canvas_name: None,
        target_width: 1920,
        target_height: 1080,
        elements,
    }
}

/// Failure-path proof (runs without game data). This is the ledger-77 class made
/// mechanical: a small drawable element frozen present that vanishes or collapses
/// in live IR must be reported.
#[test]
fn synthetic_missing_or_collapsed_node_fails_the_guard() {
    let baseline = syn_screen(vec![syn_element("999:widget_nav_arrow", 10.0, 12.0)]);

    // (a) identity absent from live -> reported.
    let absent = missing_or_collapsed_elements(&baseline, &syn_screen(vec![]));
    assert_eq!(absent.len(), 1, "expected one absent-node failure, got {absent:?}");
    assert!(
        absent[0].contains("999:widget_nav_arrow") && absent[0].contains("ABSENT"),
        "{absent:?}"
    );

    // (b) present but collapsed to zero width -> reported.
    let collapsed = missing_or_collapsed_elements(
        &baseline,
        &syn_screen(vec![syn_element("999:widget_nav_arrow", 0.0, 12.0)]),
    );
    assert_eq!(collapsed.len(), 1, "expected one collapsed-node failure, got {collapsed:?}");
    assert!(collapsed[0].contains("COLLAPSED"), "{collapsed:?}");

    // (c) present + non-degenerate -> no false positive.
    let ok = missing_or_collapsed_elements(
        &baseline,
        &syn_screen(vec![syn_element("999:widget_nav_arrow", 11.0, 9.0)]),
    );
    assert!(ok.is_empty(), "unexpected failure for a preserved element: {ok:?}");

    // (d) a degenerate FROZEN element is not required (nothing was drawn there).
    let degenerate_baseline = syn_screen(vec![syn_element("42:widget_zero", 0.0, 0.0)]);
    assert!(
        missing_or_collapsed_elements(&degenerate_baseline, &syn_screen(vec![])).is_empty(),
        "a degenerate frozen element must not be required in live IR"
    );
}

/// Element-granularity vacuity proof (runs without game data). An emptied /
/// all-degenerate re-freeze contributes zero non-degenerate frozen elements —
/// exactly the condition the `non_degenerate_frozen > 0` assert in
/// `element_presence_all_frozen_targets` fires on. This exercises the same
/// count expression (the `is_non_degenerate` filter) that guards it, so a
/// regression that made the filter accept degenerate rects is caught here.
#[test]
fn all_degenerate_frozen_set_counts_as_zero() {
    let degenerate = vec![
        syn_element("1:a", 0.0, 0.0),
        syn_element("2:b", 0.0, 5.0),
        syn_element("3:c", 5.0, 0.0),
    ];
    let non_degenerate = degenerate.iter().filter(|e| is_non_degenerate(e)).count();
    assert_eq!(
        non_degenerate, 0,
        "an all-degenerate frozen set must yield zero drawable elements — the vacuity assert must fire on it"
    );

    // Sanity: one non-degenerate element flips the count positive (assert passes).
    let mixed = vec![syn_element("4:d", 0.0, 0.0), syn_element("5:e", 3.0, 4.0)];
    assert_eq!(mixed.iter().filter(|e| is_non_degenerate(e)).count(), 1);
}

/// For every frozen target, every drawable + non-degenerate element in the frozen
/// IR snapshot must still be present and non-degenerate in the live IR.
#[test]
fn element_presence_all_frozen_targets() {
    let Some(cases) = load_live_target_cases() else {
        // Identical skip-when-no-game-data behaviour as manifest_live_ir_guard.
        return;
    };

    // T5 non-vacuous precondition (alignment plan): a fresh checkout with an
    // emptied freeze fixture must not let this guard pass vacuously. The shared
    // loader also asserts this; this documents the invariant at the guard site.
    assert!(
        !cases.is_empty(),
        "element-presence guard scanned zero frozen targets — vacuous"
    );

    // Element-granularity vacuity guard: !cases.is_empty() only proves targets
    // were scanned, not that any of them froze a drawable element. A bad
    // re-freeze that emptied the element lists (or left them all degenerate)
    // would iterate nothing and pass the loop below vacuously. Require at least
    // one non-degenerate frozen element across all targets, using the SAME
    // drawable/non-degenerate predicate the guard checks live IR against.
    let non_degenerate_frozen = cases
        .iter()
        .flat_map(|case| &case.baseline_snapshot.elements)
        .filter(|element| is_non_degenerate(element))
        .count();
    assert!(
        non_degenerate_frozen > 0,
        "element-presence guard: zero non-degenerate frozen elements across all targets — vacuous (bad re-freeze?)"
    );

    let mut failures = Vec::new();
    for case in &cases {
        let live = snapshot_from_ui_ir(&case.current_ir);
        for detail in missing_or_collapsed_elements(&case.baseline_snapshot, &live) {
            failures.push(format!("[{}] {detail}", case.id));
        }
    }

    assert!(
        failures.is_empty(),
        "frozen elements missing or collapsed in live IR — sub-threshold element loss the whole-image pixel budget cannot see (ledger 77/106/107). Do NOT re-freeze to hide this; fix the render/IR path first.\n{}",
        failures.join("\n")
    );
}
