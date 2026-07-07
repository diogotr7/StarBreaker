//! Guard against orphaned `#[test]` attributes (retro Part F, 2026-06-13).
//!
//! Inserting a new test ABOVE an existing one can detach that existing
//! `#[test]` from its function when a doc comment sits between them:
//!
//! ```text
//!     #[test]            // <- orphaned: now attaches to nothing useful
//!     /// doc for the new test
//!     #[test]
//!     fn new_test() { ... }
//!
//!     fn old_test() { ... }   // <- silently lost its #[test], stops running
//! ```
//!
//! This actually happened (the `bb_layout` OUTPUT-card spec test
//! `auto_text_children_flow_at_measured_widths` stopped running for two
//! commits). `cargo` only emits a non-fatal "duplicated attribute" /
//! "never used" warning, and the battery does not fail on warnings, so the
//! dead test was invisible. This guard is deterministic and cache-independent:
//! a `#[test]` line whose next non-blank line is a `///` doc comment is
//! orphaned (a correctly-attached `#[test]` is followed by `fn` / `pub fn` /
//! `async fn` / another attribute).

use std::fs;
use std::path::{Path, PathBuf};

fn collect_source_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("rs") | Some("part") | Some("inc")
        ) {
            out.push(path);
        }
    }
}

fn orphaned_test_attributes(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut orphans = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim() != "#[test]" {
            continue;
        }
        // Next non-blank line: a doc comment means the #[test] is detached
        // from its function by intervening docs (the orphan signature).
        let next = lines[idx + 1..]
            .iter()
            .find(|candidate| !candidate.trim().is_empty());
        if let Some(next) = next
            && next.trim_start().starts_with("///")
        {
            orphans.push(idx + 1);
        }
    }
    orphans
}

#[test]
fn no_orphaned_test_attributes_in_ui_sources() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_source_files(&manifest.join("src"), &mut files);
    collect_source_files(&manifest.join("tests"), &mut files);

    // Non-empty-target precondition (alignment plan T5, ledger 3/61/105): a
    // renamed `src/`/`tests/` or a broken walker would leave `files` empty and
    // the guard would approve nothing — a vacuous pass.
    assert!(
        !files.is_empty(),
        "orphaned-#[test] guard scanned zero source files under src/ and tests/ — vacuous"
    );

    let mut violations = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path).expect("read source");
        for line_no in orphaned_test_attributes(&source) {
            violations.push(format!("{}:{}", path.display(), line_no));
        }
    }
    assert!(
        violations.is_empty(),
        "orphaned `#[test]` attribute(s) — a `#[test]` followed by a doc \
         comment instead of its fn means the test below it lost its attribute \
         and is silently not running. Move the `#[test]` directly above its \
         own fn:\n{}",
        violations.join("\n")
    );
}

#[test]
fn detector_flags_the_orphan_pattern() {
    // The exact shape that bit us (synthetic).
    let orphaned = "    #[test]\n    /// doc for the next one\n    #[test]\n    fn real() {}\n";
    assert_eq!(orphaned_test_attributes(orphaned), vec![1]);

    let healthy = "    /// doc\n    #[test]\n    fn real() {}\n";
    assert!(orphaned_test_attributes(healthy).is_empty());
}
