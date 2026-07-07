//! Guardrail test: fail when any Rust source file (`.rs`) or engine part file
//! (`.part`) in `src/` exceeds 3000 lines. Files should still split by
//! RESPONSIBILITY well before the cap (target chunks ~2500 lines or less);
//! the cap is the hard stop, not the goal.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_LINES: usize = 3000;

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("rs") | Some("part")) {
            out.push(path);
        }
    }
}

#[test]
fn rust_source_files_stay_under_line_cap() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");

    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);
    files.sort();

    // Non-empty-target precondition (alignment plan T5, ledger 3/61/105): a
    // guard that scans zero targets passes vacuously. If `src/` were renamed or
    // the walker broke, this loop would silently approve nothing.
    assert!(
        !files.is_empty(),
        "line-count guard scanned zero source files under {} — vacuous",
        src_dir.display()
    );

    let mut violations = Vec::new();
    for file in files {
        let contents = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
        let line_count = contents.lines().count();
        if line_count > MAX_LINES {
            let rel = file
                .strip_prefix(&manifest_dir)
                .unwrap_or(&file)
                .display()
                .to_string();
            violations.push(format!("{rel} has {line_count} lines"));
        }
    }

    assert!(
        violations.is_empty(),
        "line-count guard failed (>{MAX_LINES} lines):\n{}",
        violations.join("\n")
    );
}
