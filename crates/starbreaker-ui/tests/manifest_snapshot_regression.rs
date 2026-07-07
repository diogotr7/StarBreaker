use starbreaker_ui::{
    UI_REGRESSION_MANIFEST_SCHEMA_VERSION, UiIrDocument, UiRegressionCategory,
    UiRegressionManifest, UiRegressionTarget, UiRegressionTier, UiScreenSnapshot,
    compare_manifest_targets_with_loader, snapshot_from_ui_ir,
};
use std::collections::HashMap;

fn snapshot_freeze_json() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/ui_ir/ui_snapshot_freeze.json"))
        .expect("ui snapshot freeze fixture should parse")
}

fn target_a_ir() -> UiIrDocument {
    serde_json::from_str(include_str!("fixtures/ui_ir/target_a-screen_16x9_a-ir.json"))
        .expect("ui_target_a IR fixture should parse")
}

fn target_b_ir() -> UiIrDocument {
    serde_json::from_str(include_str!(
        "fixtures/ui_ir/target_b-mesh_end_screen_plane-ir.json"
    ))
    .expect("ui_target_b IR fixture should parse")
}

fn snapshot_manifest() -> UiRegressionManifest {
    serde_json::from_str(include_str!("fixtures/ui_ir/ui_snapshot_manifest.json"))
        .expect("generic snapshot manifest fixture should parse")
}

fn manifest_snapshot_lookup() -> HashMap<String, UiScreenSnapshot> {
    HashMap::from([
        ("ui_target_a.baseline".to_string(), snapshot_from_ui_ir(&target_a_ir())),
        ("ui_target_a.current".to_string(), snapshot_from_ui_ir(&target_a_ir())),
        ("ui_target_b.baseline".to_string(), snapshot_from_ui_ir(&target_b_ir())),
        ("ui_target_b.current".to_string(), snapshot_from_ui_ir(&target_b_ir())),
        (
            "clipper_small_door.baseline".to_string(),
            snapshot_from_ui_ir(&target_b_ir()),
        ),
        (
            "clipper_small_door.current".to_string(),
            snapshot_from_ui_ir(&target_b_ir()),
        ),
    ])
}

fn run_generic_comparison(
    target_id: &str,
    category: UiRegressionCategory,
    baseline: UiScreenSnapshot,
    current: UiScreenSnapshot,
) -> starbreaker_ui::UiSnapshotComparison {
    let manifest = UiRegressionManifest {
        schema_version: UI_REGRESSION_MANIFEST_SCHEMA_VERSION,
        targets: vec![UiRegressionTarget {
            id: target_id.to_string(),
            category,
            baseline_path: "baseline".to_string(),
            current_path: "current".to_string(),
            tier: UiRegressionTier::Platinum,
            roi: None,
        }],
    };
    let snapshots = HashMap::from([
        ("baseline".to_string(), baseline),
        ("current".to_string(), current),
    ]);
    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot fixture for {path}"))
    })
    .expect("generic manifest runner should execute comparison");
    results.into_iter().next().expect("one target expected").comparison
}

#[test]
fn manifest_snapshots_are_deterministic_for_phase1_fixtures() {
    for document in [target_a_ir(), target_b_ir()] {
        let first = snapshot_from_ui_ir(&document);
        let second = snapshot_from_ui_ir(&document);
        assert_eq!(first, second, "snapshot extraction must be deterministic");
    }
}

#[test]
fn manifest_targets_pass_for_phase1_fixtures() {
    let mut manifest = snapshot_manifest();
    let snapshots = manifest_snapshot_lookup();
    manifest.targets.retain(|target| {
        snapshots.contains_key(&target.baseline_path) && snapshots.contains_key(&target.current_path)
    });

    let results = compare_manifest_targets_with_loader(&manifest, |path| {
        snapshots
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing snapshot fixture for {path}"))
    })
    .expect("manifest runner should load all manifest fixtures");

    assert_eq!(results.len(), manifest.targets.len(), "expected all fixture-backed manifest targets");
    // Non-empty-target precondition (alignment plan T5 review): retain-to-zero
    // would make both the len() check and the loop below pass vacuously.
    assert!(
        !results.is_empty(),
        "phase1 fixture guard retained zero fixture-backed targets — vacuous"
    );
    for result in results {
        assert!(
            result.comparison.passed,
            "manifest target {} should pass baseline comparison: {:?}",
            result.id,
            result.comparison.failures
        );
    }
}

#[test]
fn snapshot_freeze_ids_match_manifest_ids() {
    let manifest = snapshot_manifest();
    let freeze = snapshot_freeze_json();

    assert_eq!(
        freeze.get("schema_version").and_then(|value| value.as_u64()),
        Some(1),
        "snapshot freeze schema version should remain 1"
    );

    let mut manifest_ids: Vec<String> = manifest.targets.into_iter().map(|target| target.id).collect();
    manifest_ids.sort();

    let mut freeze_ids: Vec<String> = freeze
        .get("targets")
        .and_then(|value| value.as_array())
        .expect("snapshot freeze should contain targets array")
        .iter()
        .map(|target| {
            target
                .get("id")
                .and_then(|value| value.as_str())
                .expect("frozen target should have id")
                .to_string()
        })
        .collect();
    freeze_ids.sort();

    assert_eq!(freeze_ids, manifest_ids, "snapshot freeze ids should match manifest ids exactly");
}

#[test]
fn snapshot_freeze_remains_ir_only() {
    fn contains_key(value: &serde_json::Value, key: &str) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                map.contains_key(key) || map.values().any(|child| contains_key(child, key))
            }
            serde_json::Value::Array(items) => items.iter().any(|child| contains_key(child, key)),
            _ => false,
        }
    }

    let freeze = snapshot_freeze_json();
    let targets = freeze
        .get("targets")
        .and_then(|value| value.as_array())
        .expect("snapshot freeze should contain targets array");

    assert!(
        !targets.is_empty(),
        "snapshot freeze should contain at least one target"
    );
    assert!(
        targets.iter().all(|target| target.get("baseline_snapshot").is_some()),
        "every frozen target should include a baseline snapshot"
    );
    assert!(
        !contains_key(&freeze, "artifact_path"),
        "snapshot freeze must not contain artifact paths"
    );
    assert!(
        !contains_key(&freeze, "sha256"),
        "snapshot freeze must not contain image hashes"
    );
}

#[test]
fn manifest_snapshot_comparator_flags_text_case_drift() {
    let baseline_doc = target_a_ir();
    let mut current_doc = baseline_doc.clone();

    let text_node = current_doc
        .nodes
        .iter_mut()
        .find(|node| {
            node.is_active
                && matches!(
                    node.text_payload,
                    Some(starbreaker_ui::UiIrTextPayload::Resolved { .. })
                )
        })
        .expect("fixture should include active resolved text");
    if let Some(starbreaker_ui::UiIrTextPayload::Resolved { text }) = text_node.text_payload.as_mut() {
        let lowered = text.to_ascii_lowercase();
        *text = if lowered == *text {
            format!("{text}x")
        } else {
            lowered
        };
    }

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = run_generic_comparison(
        "target_a_text_case",
        UiRegressionCategory::Text,
        baseline,
        current,
    );

    assert!(!comparison.passed, "case drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("text payload/case drift")),
        "failure output should mention text payload/case drift"
    );
}

#[test]
fn manifest_snapshot_comparator_flags_new_visible_shape() {
    let baseline_doc = target_b_ir();
    let mut current_doc = baseline_doc.clone();

    let mut new_node = current_doc
        .nodes
        .iter()
        .find(|node| node.is_active)
        .expect("fixture should include active nodes")
        .clone();
    new_node.id = 999_999;
    new_node.node_type = "widget_custom_shape".to_string();
    new_node.text_payload = None;
    new_node.text_style = None;
    new_node.asset_ref = None;
    new_node.custom_shape = Some(starbreaker_ui::ui_ir::UiIrCustomShape {
        shape_type: Some("svg".to_string()),
        shape: None,
        svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
        render_shape: Some(true),
        enable_nine_slice_rect: None,
        nine_slice_rect: None,
        nine_slice_scale: None,
    });
    current_doc.nodes.push(new_node);

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = run_generic_comparison(
        "target_b_new_shape",
        UiRegressionCategory::Shape,
        baseline,
        current,
    );

    assert!(
        !comparison.passed,
        "new visible shape should fail comparator"
    );
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("unexpected new visible element")),
        "failure output should mention unexpected new visible element"
    );
}

#[test]
fn manifest_snapshot_comparator_flags_geometry_drift() {
    let baseline_doc = target_a_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| {
            node.is_active
                && (node.text_payload.is_some() || node.asset_ref.is_some() || node.custom_shape.is_some())
        })
        .expect("fixture should include active tracked nodes");
    node.computed_rect.x += 60.0;

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = run_generic_comparison(
        "target_a_geometry",
        UiRegressionCategory::Text,
        baseline,
        current,
    );

    assert!(!comparison.passed, "geometry drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains(" x drift ") || line.contains(": x drift")),
        "failure output should include x geometry drift"
    );
}

#[test]
fn manifest_snapshot_comparator_flags_colour_drift() {
    let baseline_doc = target_b_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| node.is_active && node.text_style.is_some())
        .expect("fixture should include active styled text");
    let style = node.text_style.as_mut().expect("text style expected");
    style.colour = Some([1.0, 0.0, 0.0, 1.0]);

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = run_generic_comparison(
        "target_b_colour",
        UiRegressionCategory::Text,
        baseline,
        current,
    );

    assert!(!comparison.passed, "colour drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("text_rgba") || line.contains("icon_tint_rgba") || line.contains("background_rgba")),
        "failure output should include colour-channel drift"
    );
}

#[test]
fn manifest_snapshot_comparator_flags_background_asset_identity_drift() {
    let baseline_doc = target_a_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| node.is_active && node.asset_ref.is_some())
        .expect("fixture should include active asset-backed image nodes");
    node.asset_ref = Some("UI/Textures/Debug/replaced_background.tif".to_string());

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = run_generic_comparison(
        "target_a_background_asset",
        UiRegressionCategory::Image,
        baseline,
        current,
    );

    assert!(!comparison.passed, "background asset drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("asset identity drift")),
        "failure output should include asset identity drift"
    );
}

#[test]
fn manifest_snapshot_comparator_flags_font_identity_drift() {
    let baseline_doc = target_a_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| node.is_active && node.text_style.is_some())
        .expect("fixture should include active styled text");
    let style = node.text_style.as_mut().expect("text style expected");
    style.font_record = Some("$Text1Bold".to_string());

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = run_generic_comparison(
        "target_a_font",
        UiRegressionCategory::Font,
        baseline,
        current,
    );

    assert!(!comparison.passed, "font identity/font_weight drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("font identity/font_weight drift")),
        "failure output should include font identity/font_weight drift"
    );
}

#[test]
fn manifest_snapshot_comparator_flags_draw_order_drift() {
    let baseline_doc = target_b_ir();
    let mut current_doc = baseline_doc.clone();

    let node = current_doc
        .nodes
        .iter_mut()
        .find(|node| node.is_active)
        .expect("fixture should include active nodes");
    node.layer += 100;

    let baseline = snapshot_from_ui_ir(&baseline_doc);
    let current = snapshot_from_ui_ir(&current_doc);
    let comparison = run_generic_comparison(
        "target_b_draw_order",
        UiRegressionCategory::Text,
        baseline,
        current,
    );

    assert!(!comparison.passed, "draw-order drift should fail comparator");
    assert!(
        comparison
            .failures
            .iter()
            .any(|line| line.contains("draw-order drift")),
        "failure output should include draw-order drift"
    );
}
