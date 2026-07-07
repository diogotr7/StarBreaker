//! Guardrail: the consolidated UI docs (`crates/starbreaker-ui/docs/ui-workflow.md`,
//! `crates/starbreaker-ui/docs/ui-reference.md`, the agent prompt) must not reference scripts,
//! docs, or examples that no longer exist (crates/starbreaker-ui/docs/ui-process-improvements.md
//! item 12 — verify-on-write). Deliberately forgiving: it only flags
//! vanished FILES, not prose accuracy.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn extract<'a>(text: &'a str, pattern: &str) -> Vec<String> {
    // Tiny hand-rolled matcher: find `pattern`-prefixed path tokens.
    let mut out = Vec::new();
    for (idx, _) in text.match_indices(pattern) {
        // Reject mid-path hits (e.g. `crates/.../docs/x.md` matched at `docs/`).
        if idx > 0 {
            let prev = text.as_bytes()[idx - 1] as char;
            if prev == '/' || prev.is_alphanumeric() {
                continue;
            }
        }
        let tail = &text[idx..];
        let token: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || "/_-.".contains(*c))
            .collect();
        let token = token.trim_end_matches(['.', '-']).to_string();
        if token.len() > pattern.len() {
            out.push(token);
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn ui_docs_reference_existing_files() {
    let root = repo_root();
    let docs = [
        "crates/starbreaker-ui/docs/ui-workflow.md",
        "crates/starbreaker-ui/docs/ui-reference.md",
        "crates/starbreaker-ui/docs/ui-matching-agent-prompt.md",
        "crates/starbreaker-ui/docs/ui-process-retro-prompt.md",
    ];
    let mut missing = Vec::new();
    let mut checked = 0usize;
    for doc in docs {
        let path = root.join(doc);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("missing consolidated doc {doc}: {e}"));
        // Ignore the historical "Supersedes the former ..." sentence.
        let text = match text.split_once("Supersedes the former") {
            Some((head, tail)) => {
                let rest = tail.split_once("\n\n").map(|(_, r)| r).unwrap_or("");
                format!("{head}{rest}")
            }
            None => text,
        };
        for prefix in ["scripts/", "docs/", "crates/starbreaker-ui/docs/"] {
            for token in extract(&text, prefix) {
                let has_ext = token.ends_with(".sh")
                    || token.ends_with(".py")
                    || token.ends_with(".md")
                    || token.ends_with(".json");
                if !has_ext {
                    continue;
                }
                checked += 1;
                if !root.join(&token).exists() {
                    missing.push(format!("{doc} -> {token}"));
                }
            }
        }
    }
    // Non-empty-target precondition (alignment plan T5): a zero-match run is a
    // harness FAILURE, not a clean pass (ledger item 3). If the extractor or the
    // docs' path-reference format drifts so that no file tokens are found, this
    // guard silently verifies nothing.
    assert!(
        checked > 0,
        "docs-reference guard extracted zero file tokens from the consolidated UI \
         docs — vacuous (extractor or doc path-reference format drifted?)"
    );
    assert!(
        missing.is_empty(),
        "consolidated UI docs reference files that do not exist (fix the doc \
         or restore the file in the same commit):\n{}",
        missing.join("\n")
    );
}
