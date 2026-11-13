//! The exporter's source identity (`build_support/identity.rs`) covers only the
//! crates the exporter is built from: `starbreaker-3d` and its workspace
//! dependencies, transitively, each crate's `Cargo.toml`, `build.rs`, `src/`
//! and `build_support/`, plus the workspace `Cargo.toml` / `Cargo.lock`. An
//! unrelated crate, or a crate's tests and fixtures, does not change it (and so
//! does not rebuild `starbreaker-3d`).

#[path = "../build_support/identity.rs"]
mod identity;

use std::fs;
use std::path::{Path, PathBuf};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A synthetic workspace: the exporter depends on `starbreaker-a`, which
/// depends on `starbreaker-b`; `starbreaker-z` is unrelated.
fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("sb_identity_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    write(&root.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n");
    write(&root.join("Cargo.lock"), "# lock\n");
    write(
        &root.join("crates/starbreaker-3d/Cargo.toml"),
        "[package]\nname = \"starbreaker-3d\"\n\n[dependencies]\nstarbreaker-a = { workspace = true }\nserde = \"1\"\n\n[dev-dependencies]\nstarbreaker-z = { workspace = true }\n",
    );
    write(&root.join("crates/starbreaker-3d/build.rs"), "fn main() {}\n");
    write(&root.join("crates/starbreaker-3d/src/lib.rs"), "//! 3d\n");
    write(&root.join("crates/starbreaker-3d/build_support/identity.rs"), "//! id\n");
    write(&root.join("crates/starbreaker-3d/tests/t.rs"), "//! test\n");
    write(&root.join("crates/starbreaker-a/Cargo.toml"), "[package]\nname = \"starbreaker-a\"\n\n[dependencies]\nstarbreaker-b = { path = \"../starbreaker-b\" }\n");
    write(&root.join("crates/starbreaker-a/src/lib.rs"), "//! a\n");
    write(&root.join("crates/starbreaker-a/tests/fixtures/big.json"), "{}\n");
    write(&root.join("crates/starbreaker-b/Cargo.toml"), "[package]\nname = \"starbreaker-b\"\n");
    write(&root.join("crates/starbreaker-b/src/lib.rs"), "//! b\n");
    write(&root.join("crates/starbreaker-z/Cargo.toml"), "[package]\nname = \"starbreaker-z\"\n");
    write(&root.join("crates/starbreaker-z/src/lib.rs"), "//! z\n");
    root
}

fn relative(root: &Path, files: &[PathBuf]) -> Vec<String> {
    files.iter().map(|p| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")).collect()
}

#[test]
fn identity_covers_the_exporter_and_its_dependencies_only() {
    let root = workspace("covers");
    let files = relative(&root, &identity::source_files(&root).unwrap());
    assert_eq!(
        files,
        [
            "Cargo.lock",
            "Cargo.toml",
            "crates/starbreaker-3d/Cargo.toml",
            "crates/starbreaker-3d/build.rs",
            "crates/starbreaker-3d/build_support/identity.rs",
            "crates/starbreaker-3d/src/lib.rs",
            "crates/starbreaker-a/Cargo.toml",
            "crates/starbreaker-a/src/lib.rs",
            "crates/starbreaker-b/Cargo.toml",
            "crates/starbreaker-b/src/lib.rs",
        ]
    );
}

#[test]
fn unrelated_crates_and_tests_leave_the_digest_unchanged() {
    let root = workspace("digest");
    let before = identity::source_digest(&root).unwrap();
    write(&root.join("crates/starbreaker-z/src/lib.rs"), "//! z changed\n");
    write(&root.join("crates/starbreaker-a/tests/fixtures/big.json"), "{\"changed\": true}\n");
    assert_eq!(identity::source_digest(&root).unwrap(), before);
    write(&root.join("crates/starbreaker-b/src/lib.rs"), "//! b changed\n");
    assert_ne!(identity::source_digest(&root).unwrap(), before, "a dependency's source is an exporter input");
}

#[test]
fn build_script_watches_the_same_inputs() {
    let root = workspace("watched");
    let watched = relative(&root, &identity::watched_paths(&root).unwrap());
    assert_eq!(
        watched,
        [
            "Cargo.toml",
            "Cargo.lock",
            "crates/starbreaker-3d/Cargo.toml",
            "crates/starbreaker-3d/build.rs",
            "crates/starbreaker-3d/src",
            "crates/starbreaker-3d/build_support",
            "crates/starbreaker-a/Cargo.toml",
            "crates/starbreaker-a/src",
            "crates/starbreaker-b/Cargo.toml",
            "crates/starbreaker-b/src",
        ]
    );
}
