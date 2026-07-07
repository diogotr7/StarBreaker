use std::fs;
use std::path::Path;

fn load_engine_module_source(module_dir: &str) -> String {
    // The stage engines are real submodules since review F1 (formerly
    // `engine.inc` + `engine_parts/*.part` include-splices): merge every
    // `.rs` file in the module directory.
    let module_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(module_dir);
    let mut paths: Vec<_> = fs::read_dir(&module_root)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", module_root.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no .rs sources found under {}",
        module_root.display()
    );
    let mut merged = String::new();
    for path in paths {
        let chunk = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        merged.push('\n');
        merged.push_str(&chunk);
    }
    merged
}

#[test]
fn hardcoding_guard_tests_exist_in_core_renderer_files() {
    let guarded_files: Vec<(&str, String, &str)> = vec![
        (
            "compose/tests.rs",
            include_str!("../src/compose/tests.rs").to_string(),
            "fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "ir_compose.rs",
            load_engine_module_source("ir_compose"),
            "fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "ui_ir.rs",
            load_engine_module_source("ui_ir"),
            "fn ui_ir_source_does_not_reintroduce_forbidden_hardcoded_markers()",
        ),
        (
            "bb_layout.rs",
            load_engine_module_source("bb_layout"),
            "fn layout_source_does_not_reintroduce_forbidden_hardcoded_or_heuristic_markers()",
        ),
    ];

    for (path, source, guard_fn_sig) in guarded_files {
        assert!(
            source.contains(guard_fn_sig),
            "required hardcoding guard test missing in {path}: expected `{guard_fn_sig}`",
        );
    }
}

/// Crate-wide ban on hard-coded `RgbaColor { .. }` colour literals.
///
/// A colour value copied into source (production OR test fixtures — e.g. the
/// s_bioc Base `r: 115, g: 198, b: 254` once embedded in ir_compose tests, or
/// the invented "Drake amber" fallback palette) is hard-coded game data: it
/// silently diverges from DataCore and normalises extending the pattern.
/// Colours must be parsed from game data at run time, or — for offline test
/// fixtures — loaded from a provenance-noted extracted fixture
/// (`tests/fixtures/ui_ir/brand_palettes_v1.json`).
///
/// Allowed without annotation:
/// - constructions whose fields are all expressions/variables (parsers);
/// - NEUTRAL literals: every numeric component 0 or 255 (pure
///   black/white/transparent — absence-of-light constants, not palette data).
/// Anything else requires a `hardcoding-guard: synthetic` annotation within
/// a few lines above the literal, reserved for genuinely arbitrary test
/// colours that are NOT copies of real game values.
///
/// Scans `examples/` as well as `src/`: diagnostic examples carried copied
/// drak palette values undetected until the examples joined the check
/// battery (plan P0.3).
#[test]
fn rgba_colour_literals_are_not_hardcoded() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations: Vec<String> = Vec::new();
    let mut files_scanned = 0usize;
    let mut stack = vec![manifest_dir.join("src"), manifest_dir.join("examples")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "rs" | "part" | "inc") {
                continue;
            }
            let source = fs::read_to_string(&path).expect("read source file");
            files_scanned += 1;
            scan_rgba_literals(&path, &source, &mut violations);
        }
    }
    // Non-empty-target precondition (alignment plan T5, ledger 3/61/105): if the
    // walk finds zero sources (src/ renamed, extension filter drift) the guard
    // approves nothing — a vacuous pass worse than no guard.
    assert!(
        files_scanned > 0,
        "rgba-literal guard scanned zero .rs/.part/.inc files under src/ and examples/ — vacuous"
    );
    assert!(
        violations.is_empty(),
        "hard-coded RgbaColor literals found (parse from game data or load the \
         provenance fixture; see crates/starbreaker-ui/AGENTS.md Core rules):\n{}",
        violations.join("\n")
    );
}

fn scan_rgba_literals(path: &Path, source: &str, violations: &mut Vec<String>) {
    let mut search_from = 0;
    while let Some(rel) = source[search_from..].find("RgbaColor") {
        let start = search_from + rel;
        search_from = start + "RgbaColor".len();
        let Some(brace_rel) = source[start..].find('{') else { continue };
        let brace = start + brace_rel;
        // Struct-definition / non-literal uses have code between the name and
        // the brace (e.g. `pub struct RgbaColor {`): only `RgbaColor {`
        // (whitespace only) is a literal construction.
        if !source[start + "RgbaColor".len()..brace].trim().is_empty() {
            continue;
        }
        let Some(end_rel) = source[brace..].find('}') else { continue };
        let span = &source[brace + 1..brace + end_rel];

        let mut numeric_values: Vec<u32> = Vec::new();
        let mut has_literal_field = false;
        for field in span.split(',') {
            let Some((_, value)) = field.split_once(':') else { continue };
            let value = value.trim();
            if value.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                has_literal_field = true;
                let digits: String =
                    value.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<u32>() {
                    numeric_values.push(n);
                }
            }
        }
        if !has_literal_field {
            continue;
        }
        if numeric_values.iter().all(|&n| n == 0 || n == 255) {
            continue;
        }
        let line_no = source[..start].lines().count();
        let annotated = source[..start]
            .lines()
            .rev()
            .take(4)
            .any(|line| line.contains("hardcoding-guard: synthetic"));
        if annotated {
            continue;
        }
        violations.push(format!("{}:{}", path.display(), line_no));
    }
}

/// Companion to the `RgbaColor` guard for the `[f32; 4]` / `[u8; 4]`
/// colour-array shape (`Some([0.0, 113.0 / 255.0, …])`-style pins were this
/// category). PRODUCTION code only: `#[cfg(test)]` regions are skipped —
/// arbitrary synthetic arrays are fine in fixtures. A 4-element numeric
/// array literal on a line that names a colour-ish field (color/colour/
/// tint/rgba/fill/stroke) must be NEUTRAL (every element 0, 1, 0.0, 1.0 or
/// 255) or carry the `hardcoding-guard: synthetic` annotation nearby.
#[test]
fn colour_array_literals_are_not_hardcoded_in_production() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations: Vec<String> = Vec::new();
    let mut files_scanned = 0usize;
    let mut stack = vec![src_root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "rs" | "part" | "inc") {
                continue;
            }
            // Whole-file test modules (src/*/tests*.rs, test support/fixtures)
            // are exempt — this guard is production-only.
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.starts_with("tests") || stem == "test_palettes" {
                continue;
            }
            let source = fs::read_to_string(&path).expect("read source file");
            files_scanned += 1;
            scan_colour_array_literals(&path, &strip_cfg_test_regions(&source), &mut violations);
        }
    }
    // Non-empty-target precondition (alignment plan T5, ledger 3/61/105): a
    // renamed src/ or a broken extension filter would scan zero production files
    // and approve nothing vacuously.
    assert!(
        files_scanned > 0,
        "colour-array guard scanned zero production .rs/.part/.inc files under src/ — vacuous"
    );
    assert!(
        violations.is_empty(),
        "hard-coded colour-array literals found in production code (derive from \
         the brand palette / parsed data instead):\n{}",
        violations.join("\n")
    );
}

/// Blank out `#[cfg(test)]`-gated regions (the following item's braces),
/// preserving line numbers so violation reports stay accurate.
fn strip_cfg_test_regions(source: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut in_test = false;
    for line in source.lines() {
        if !in_test && line.trim_start().starts_with("#[cfg(test)]") {
            in_test = true;
            depth = 0;
            out.push(String::new());
            continue;
        }
        if in_test {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            out.push(String::new());
            if depth <= 0 && line.contains('}') {
                in_test = false;
            }
            continue;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

fn scan_colour_array_literals(path: &Path, source: &str, violations: &mut Vec<String>) {
    let colourish = ["color", "colour", "tint", "rgba", "fill", "stroke"];
    let all_lines: Vec<&str> = source.lines().collect();
    for (idx, line) in all_lines.iter().enumerate() {
        let lower = line.to_ascii_lowercase();
        if !colourish.iter().any(|kw| lower.contains(kw)) {
            continue;
        }
        let Some(open) = line.find('[') else { continue };
        let Some(close_rel) = line[open..].find(']') else { continue };
        let inner = &line[open + 1..open + close_rel];
        let elements: Vec<&str> = inner.split(',').map(str::trim).collect();
        if elements.len() != 4
            || !elements
                .iter()
                .all(|e| e.chars().next().is_some_and(|c| c.is_ascii_digit()))
        {
            continue;
        }
        let neutral = |e: &str| matches!(e, "0" | "1" | "0.0" | "1.0" | "255" | "0u8" | "255u8");
        if elements.iter().all(|e| neutral(e)) {
            continue;
        }
        let annotated = all_lines[..idx]
            .iter()
            .rev()
            .take(3)
            .any(|l| l.contains("hardcoding-guard: synthetic"));
        if annotated {
            continue;
        }
        violations.push(format!("{}:{}", path.display(), idx + 1));
    }
}

/// The extracted brand-palette fixture must stay in sync with the live
/// DataCore records: when the decompiled record mirror is present (same
/// skip-if-missing pattern as `manifest_live_ir_guard`), every fixture slot
/// is compared against the record's authored `colorStyles`. Refresh
/// `brand_palettes_v1.json` (+ its `.notes.md`) when an upstream patch
/// changes a palette.
#[test]
fn brand_palette_fixture_matches_live_records() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root should resolve");
    let styles_root =
        workspace_root.join("ships/dcb_canvas/libs/foundry/records/ui/buildingblocks/styles");
    if !styles_root.is_dir() {
        eprintln!(
            "skipping brand palette fixture validation (missing records root: {})",
            styles_root.display()
        );
        return;
    }
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ui_ir/brand_palettes_v1.json"))
            .expect("fixture parses");
    let brands = fixture["brands"].as_object().expect("brands object");
    // Non-empty-target precondition (alignment plan T5, ledger 3/61/105): an
    // emptied fixture would compare zero brands and pass vacuously.
    assert!(
        !brands.is_empty(),
        "brand-palette fixture guard found zero brands — vacuous (fixture emptied?)"
    );
    for (brand, entry) in brands {
        let record_path = styles_root.join(format!("{brand}.json"));
        let record: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&record_path)
                .unwrap_or_else(|e| panic!("read {}: {e}", record_path.display())),
        )
        .expect("record parses");
        let live = record["_RecordValue_"]["colorStyles"]
            .as_array()
            .expect("record colorStyles");
        let frozen = entry["colorStyles"].as_array().expect("fixture colorStyles");
        assert_eq!(
            frozen.len(),
            live.len(),
            "{brand}: fixture slot count diverged from the live record"
        );
        for (i, (frozen_slot, live_slot)) in frozen.iter().zip(live).enumerate() {
            let live_colour = live_slot.get("color").filter(|c| !c.is_null());
            match (frozen_slot.is_null(), live_colour) {
                (true, None) => {}
                (false, Some(colour)) => {
                    for k in ["r", "g", "b"] {
                        assert_eq!(
                            frozen_slot[k].as_u64(),
                            colour[k].as_u64(),
                            "{brand} slot {i} channel {k} diverged from the live record"
                        );
                    }
                }
                _ => panic!("{brand} slot {i}: null-ness diverged from the live record"),
            }
        }
    }
}

#[test]
fn bb_layout_source_has_no_forbidden_heuristic_markers() {
    let source = load_engine_module_source("bb_layout");
    let forbidden = [
        ["hard", "coded", "_offset"].concat(),
        ["magic", "_multiplier"].concat(),
        ["heu", "ristic", "_shift"].concat(),
        ["blend", "_factor"].concat(),
    ];

    for marker in forbidden {
        assert!(
            !source.contains(marker.as_str()),
            "bb_layout heuristic marker reintroduced: {marker}",
        );
    }
}
