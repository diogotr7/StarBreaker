use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
    UiIrDocument, UiKnownOutlier, UiKnownOutlierRegistry, UiRegressionManifest, UiScreenSnapshot,
    UiSnapshotElement, compare_manifest_targets_with_loader,
    compare_manifest_targets_with_loader_and_outliers, compile_ir_for_binding, snapshot_from_ui_ir,
};

/// Reference-anchored known-outlier overrides (committed numbers, no image).
fn known_outliers() -> Vec<UiKnownOutlier> {
    let registry: UiKnownOutlierRegistry =
        serde_json::from_str(include_str!("fixtures/ui_ir/ui_known_outliers.json"))
            .expect("known-outlier registry fixture should parse");
    registry.outliers
}

#[derive(Debug, Deserialize)]
struct SnapshotFreezeFile {
    targets: Vec<SnapshotFreezeTarget>,
}

#[derive(Debug, Deserialize)]
struct SnapshotFreezeTarget {
    id: String,
    source_generated_png: String,
    canvas_guid: String,
    baseline_snapshot: UiScreenSnapshot,
}

struct LiveTargetCase {
    id: String,
    current_ir: UiIrDocument,
    baseline_snapshot: UiScreenSnapshot,
}

struct FsCanvasFetcher {
    guid_to_path: HashMap<String, PathBuf>,
    by_name: HashMap<String, String>,
}

impl CanvasFetcher for FsCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        let path = self
            .guid_to_path
            .get(guid)
            .ok_or_else(|| UiError::RenderError(format!("missing canvas guid: {guid}")))?;
        load_canvas_json_from_path(path)
            .map_err(|err| UiError::RenderError(format!("failed loading canvas {guid}: {err}")))
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        let guid = self
            .by_name
            .get(record_name)
            .or_else(|| self.by_name.get(&record_name.to_ascii_lowercase()))
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))?;
        self.fetch_canvas_json(guid)
    }
}

struct DummySwfFetcher;

impl SwfFetcher for DummySwfFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("SWF fetch not required".to_string()))
    }
}

struct DummyStyleFetcher {
    manufacturer_id: String,
}

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Ok(starbreaker_ui::StyleLoader::for_manufacturer(&self.manufacturer_id).neutral_fallback())
    }
}

struct DummyAssetFetcher;

impl AssetFetcher for DummyAssetFetcher {
    fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
        None
    }
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

fn load_canvas_json_from_path(path: &Path) -> Result<serde_json::Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn load_canvas_index(root: &Path) -> Result<FsCanvasFetcher, String> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files);

    let mut guid_to_path = HashMap::new();
    let mut by_name = HashMap::new();

    for path in files {
        let json = load_canvas_json_from_path(&path)?;
        let Some(record_name) = json
            .get("_RecordName_")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(record_id) = json
            .get("_RecordId_")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
        else {
            continue;
        };

        let bare_name = record_name
            .strip_prefix("BuildingBlocks_Canvas.")
            .unwrap_or(&record_name)
            .to_string();
        let path_stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_string();
        let path_rel = path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(|relative| relative.replace('\\', "/"));

        guid_to_path.insert(record_id.clone(), path.clone());
        by_name.insert(record_name.clone(), record_id.clone());
        by_name.insert(record_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(bare_name.clone(), record_id.clone());
        by_name.insert(bare_name.to_ascii_lowercase(), record_id.clone());
        by_name.insert(record_id.clone(), record_id.clone());
        if !path_stem.is_empty() {
            by_name.insert(path_stem.clone(), record_id.clone());
            by_name.insert(path_stem.to_ascii_lowercase(), record_id.clone());
        }
        if let Some(rel) = path_rel {
            by_name.insert(rel.clone(), record_id.clone());
            by_name.insert(rel.to_ascii_lowercase(), record_id.clone());
        }
    }

    Ok(FsCanvasFetcher { guid_to_path, by_name })
}

fn compile_target_ir(
    fetcher: &FsCanvasFetcher,
    localization_map: Option<HashMap<String, String>>,
    manufacturer_id: &str,
    canvas_guid: &str,
    target_size: (u32, u32),
) -> UiIrDocument {
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher {
        manufacturer_id: manufacturer_id.to_string(),
    };
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some(canvas_guid),
        content_canvas_guid: Some(canvas_guid),
        binding_kind: Some("mfd"),
        manufacturer_id: Some(manufacturer_id),
        helper_name: Some("freeze-ui-snapshot-ir"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: fetcher,
        swf_fetcher: &swf_fetcher,
        style_fetcher: &style_fetcher,
        asset_fetcher: &asset_fetcher,
        target_size,
        apply_postprocess: false,
        animation_sample_percent: None,
        localization_map,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };

    compile_ir_for_binding(&inputs).expect("target IR compile should succeed")
}

fn live_snapshot_manifest() -> UiRegressionManifest {
    serde_json::from_str(include_str!("fixtures/ui_ir/ui_snapshot_manifest.json"))
        .expect("snapshot manifest fixture should parse")
}

fn snapshot_freeze() -> SnapshotFreezeFile {
    serde_json::from_str(include_str!("fixtures/ui_ir/ui_snapshot_freeze.json"))
        .expect("snapshot freeze fixture should parse")
}

fn manufacturer_from_source_path(source_generated_png: &str) -> String {
    let parts: Vec<&str> = source_generated_png.split('/').collect();
    if let Some(index) = parts.iter().position(|part| *part == "ship") {
        if let Some(manufacturer) = parts.get(index + 1) {
            return (*manufacturer).to_string();
        }
    }
    "drak".to_string()
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

fn load_live_target_cases() -> Option<Vec<LiveTargetCase>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root should resolve from CARGO_MANIFEST_DIR");
    let canvas_root = workspace_root.join("ships/dcb_canvas/libs/foundry/records");
    if !canvas_root.is_dir() {
        eprintln!(
            "skipping live manifest IR guard (missing records root: {})",
            canvas_root.display()
        );
        return None;
    }
    let fetcher = load_canvas_index(&canvas_root).expect("canvas index should load");

    let localization_path = workspace_root.join("target/Data/Localization/english/global.ini");
    let localization_map = fs::read(&localization_path)
        .ok()
        .map(|bytes| starbreaker_ui::bb_loc_p4k::parse_ini_bytes(&bytes));

    let cases = snapshot_freeze()
        .targets
        .into_iter()
        .map(|target| {
            let manufacturer_id = manufacturer_from_source_path(&target.source_generated_png);
            let target_size = (
                target.baseline_snapshot.target_width,
                target.baseline_snapshot.target_height,
            );
            let current_ir = compile_target_ir(
                &fetcher,
                localization_map.clone(),
                &manufacturer_id,
                &target.canvas_guid,
                target_size,
            );

            LiveTargetCase {
                id: target.id,
                current_ir,
                baseline_snapshot: target.baseline_snapshot,
            }
        })
        .collect();

    Some(cases)
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
