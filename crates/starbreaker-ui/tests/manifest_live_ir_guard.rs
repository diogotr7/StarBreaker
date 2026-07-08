use std::collections::HashMap;

use starbreaker_ui::{
    UiIrDocument, UiKnownOutlier, UiKnownOutlierRegistry, UiRegressionManifest, UiScreenSnapshot,
    UiSnapshotElement, compare_manifest_targets_with_loader,
    compare_manifest_targets_with_loader_and_outliers, snapshot_from_ui_ir,
};

mod live_ir_harness;
use live_ir_harness::load_live_target_cases;

/// Reference-anchored known-outlier overrides (committed numbers, no image).
fn known_outliers() -> Vec<UiKnownOutlier> {
    let registry: UiKnownOutlierRegistry =
        serde_json::from_str(include_str!("fixtures/ui_ir/ui_known_outliers.json"))
            .expect("known-outlier registry fixture should parse");
    registry.outliers
}

fn live_snapshot_manifest() -> UiRegressionManifest {
    serde_json::from_str(include_str!("fixtures/ui_ir/ui_snapshot_manifest.json"))
        .expect("snapshot manifest fixture should parse")
}

fn focused_movement_snapshot(snapshot: &UiScreenSnapshot) -> UiScreenSnapshot {
    let mut elements: Vec<UiSnapshotElement> = snapshot
        .elements
        .iter()
        .cloned()
        .map(|mut element| {
            // Preserve element sets/categories while normalizing style noise.
            element.alpha = 1.0;
            element.blend_mode = None;
            element.asset_identity = None;
            element.alignment = None;
            element.vertical_alignment = None;
            element.overflow_mode = None;
            element.background_rgba = None;
            element.stroke_rgba = None;
            element.text_rgba = None;
            element.icon_tint_rgba = None;
            element.stroke_extent = None;
            element.text_font_identity = None;
            element.line_spacing = None;
            element
        })
        .collect();
    elements.sort_by(|a, b| a.identity.cmp(&b.identity));

    UiScreenSnapshot {
        schema_version: snapshot.schema_version,
        canvas_guid: snapshot.canvas_guid.clone(),
        canvas_name: snapshot.canvas_name.clone(),
        target_width: snapshot.target_width,
        target_height: snapshot.target_height,
        elements,
    }
}

fn focused_tint_snapshot(snapshot: &UiScreenSnapshot) -> UiScreenSnapshot {
    let mut elements: Vec<UiSnapshotElement> = snapshot
        .elements
        .iter()
        .cloned()
        .map(|mut element| {
            // Keep tint semantics and asset identity while removing unrelated
            // geometry/typography drift noise.
            element.x = 0.0;
            element.y = 0.0;
            element.w = 1.0;
            element.h = 1.0;
            element.alpha = 1.0;
            element.draw_order_index = 0;
            element.alignment = None;
            element.vertical_alignment = None;
            element.overflow_mode = None;
            element.stroke_extent = None;
            element.text_payload = None;
            element.text_font_identity = None;
            element.text_font_size = None;
            element.line_spacing = None;
            element
        })
        .collect();
    elements.sort_by(|a, b| a.identity.cmp(&b.identity));

    UiScreenSnapshot {
        schema_version: snapshot.schema_version,
        canvas_guid: snapshot.canvas_guid.clone(),
        canvas_name: snapshot.canvas_name.clone(),
        target_width: snapshot.target_width,
        target_height: snapshot.target_height,
        elements,
    }
}

fn visible_placeholder_nodes(document: &UiIrDocument) -> Vec<String> {
    document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .filter_map(|node| {
            let payload = node.text_payload.as_ref()?;
            match payload {
                starbreaker_ui::UiIrTextPayload::Resolved { text }
                    if text.contains("PLACEHOLDER") =>
                {
                    Some(format!("id={} name={} resolved={text}", node.id, node.name))
                }
                starbreaker_ui::UiIrTextPayload::UnresolvedKey { key }
                    if key.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER") =>
                {
                    Some(format!("id={} name={} unresolved={key}", node.id, node.name))
                }
                _ => None,
            }
        })
        .collect()
}

fn missing_font_metadata_nodes(document: &UiIrDocument) -> Vec<String> {
    document
        .nodes
        .iter()
        .filter(|node| node.is_active)
        .filter_map(|node| {
            let text = match node.text_payload.as_ref() {
                Some(starbreaker_ui::UiIrTextPayload::Resolved { text }) if !text.trim().is_empty() => text,
                _ => return None,
            };
            let style = node.text_style.as_ref()?;
            let font_record = style.font_record.as_deref().unwrap_or("");
            if font_record.is_empty() {
                return None;
            }

            let Some(resolved) = style.resolved_font_record.as_ref() else {
                return None;
            };
            let resolved_value = resolved.get("_RecordValue_").unwrap_or(resolved);
            let font_symbol = resolved_value
                .get("font")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let paint_file = resolved_value
                .get("paintFile")
                .and_then(|value| value.as_str())
                .unwrap_or("");

            if font_symbol.is_empty() || paint_file.is_empty() {
                Some(format!(
                    "id={} name={} text='{}' font_record='{}' font='{}' paintFile='{}'",
                    node.id,
                    node.name,
                    text,
                    font_record,
                    font_symbol,
                    paint_file,
                ))
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn live_manifest_targets_have_no_visible_placeholder_text() {
    let Some(cases) = load_live_target_cases() else {
        return;
    };

    for case in cases {
        let placeholders = visible_placeholder_nodes(&case.current_ir);
        assert!(
            placeholders.is_empty(),
            "{} gold-standard output contains visible placeholder text. This indicates broken UI generation, not baseline drift. Do not update baselines; investigate placeholder/default/localization handling first.\n{}",
            case.id,
            placeholders.join("\n")
        );
    }
}

#[test]
fn live_manifest_targets_match_gold_standard_snapshot_geometry() {
    let Some(cases) = load_live_target_cases() else {
        return;
    };

    if cases
        .iter()
        .any(|case| !visible_placeholder_nodes(&case.current_ir).is_empty())
    {
        eprintln!(
            "skipping gold-standard geometry comparison because visible placeholder text already indicates broken target output"
        );
        return;
    }

    let mut manifest = live_snapshot_manifest();
    let mut snapshots = HashMap::new();
    for case in &cases {
        snapshots.insert(
            format!("{}.baseline", case.id),
            focused_movement_snapshot(&case.baseline_snapshot),
        );
        snapshots.insert(
            format!("{}.current", case.id),
            focused_movement_snapshot(&snapshot_from_ui_ir(&case.current_ir)),
        );
    }
    manifest.targets.retain(|target| {
        snapshots.contains_key(&target.baseline_path) && snapshots.contains_key(&target.current_path)
    });
    let outliers = known_outliers();
    let results = compare_manifest_targets_with_loader_and_outliers(
        &manifest,
        |path| {
            snapshots
                .get(path)
                .cloned()
                .ok_or_else(|| format!("missing snapshot fixture for {path}"))
        },
        &outliers,
    )
    .expect("manifest runner should compare live snapshots against baselines");

    // Non-empty-target precondition (alignment plan T5 review): the retain
    // above layers on the asserted `cases`; retained-to-zero would pass
    // vacuously.
    assert!(
        !results.is_empty(),
        "live geometry guard retained zero manifest targets — vacuous"
    );

    // Surface positive reinforcement: a known-outlier field moved closer to its
    // in-game reference than the frozen baseline. Never fails — signals a genuine
    // improvement worth re-freezing (do not revert it as a regression).
    for result in &results {
        for note in &result.comparison.improvements {
            eprintln!("[{}] {note}", result.id);
        }
    }

    let failures: Vec<String> = results
        .into_iter()
        .filter(|result| !result.comparison.passed)
        .map(|result| {
            format!(
                "{} gold-standard live IR drift. Do not update baselines unless the drift is intentional, source-backed, and explicitly approved.\n{}",
                result.id,
                result.comparison.failures.join("\n")
            )
        })
        .collect();

    assert!(
        failures.is_empty(),
        "{}",
        failures.join("\n\n")
    );
}

#[test]
fn live_manifest_targets_match_gold_standard_tint_semantics() {
    let Some(cases) = load_live_target_cases() else {
        return;
    };

    let mut manifest = live_snapshot_manifest();
    let mut snapshots = HashMap::new();
    for case in &cases {
        snapshots.insert(
            format!("{}.baseline", case.id),
            focused_tint_snapshot(&case.baseline_snapshot),
        );
        snapshots.insert(
            format!("{}.current", case.id),
            focused_tint_snapshot(&snapshot_from_ui_ir(&case.current_ir)),
        );
    }
    manifest.targets.retain(|target| {
        snapshots.contains_key(&target.baseline_path) && snapshots.contains_key(&target.current_path)
    });
    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot fixture for {path}"))
    })
    .expect("manifest runner should compare live tint semantics against baselines");

    // Non-empty-target precondition (alignment plan T5 review): same as the
    // geometry guard — the retain must leave at least one target.
    assert!(
        !results.is_empty(),
        "live tint-semantics guard retained zero manifest targets — vacuous"
    );

    let failures: Vec<String> = results
        .into_iter()
        .filter(|result| !result.comparison.passed)
        .map(|result| {
            format!(
                "{} gold-standard tint/brand drift. Do not update baselines unless the drift is intentional, source-backed, and explicitly approved.\n{}",
                result.id,
                result.comparison.failures.join("\n")
            )
        })
        .collect();

    assert!(
        failures.is_empty(),
        "{}",
        failures.join("\n\n")
    );
}

#[test]
fn live_manifest_targets_have_resolved_font_symbol_metadata() {
    let Some(cases) = load_live_target_cases() else {
        return;
    };

    for case in cases {
        let missing = missing_font_metadata_nodes(&case.current_ir);
        assert!(
            missing.is_empty(),
            "{} has active text nodes missing structural font metadata (font symbol/paintFile). This is a wrong-font risk and should be fixed in production data flow before baseline changes.\n{}",
            case.id,
            missing.join("\n")
        );
    }
}
