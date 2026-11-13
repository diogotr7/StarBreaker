//! Reproducible exporter source identity shared by the build and UI stale-export guard.
//!
//! The identity covers only what the exporter is built from: `starbreaker-3d`
//! and the workspace crates it depends on, transitively ([`exporter_crates`],
//! read from each `Cargo.toml`'s normal and build dependencies), each crate's
//! `Cargo.toml`, `build.rs`, `src/` and `build_support/`, plus the workspace
//! `Cargo.toml` and `Cargo.lock` ([`source_files`]). `build.rs` watches exactly
//! these ([`watched_paths`]), so editing an unrelated crate, or any crate's
//! tests and fixtures, neither changes the identity nor rebuilds the exporter.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

/// The crate the identity starts from.
const EXPORTER: &str = "starbreaker-3d";

/// Workspace crates named in the normal, build or target dependency tables
/// of `manifest` (dev-dependencies are not build inputs).
fn workspace_dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_deps = section == "dependencies" || section == "build-dependencies" || section.ends_with(".dependencies") || section.ends_with(".build-dependencies");
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim();
            if key.starts_with("starbreaker-") {
                out.push(key.to_owned());
            }
        }
    }
    out
}

/// Directories of the exporter and every workspace crate it depends on,
/// transitively, sorted.
pub fn exporter_crates(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![EXPORTER.to_owned()];
    while let Some(name) = queue.pop() {
        let dir = root.join("crates").join(&name);
        if !dir.join("Cargo.toml").is_file() || !seen.insert(dir.clone()) {
            continue;
        }
        queue.extend(workspace_dependencies(&fs::read_to_string(dir.join("Cargo.toml"))?));
    }
    Ok(seen.into_iter().collect())
}

/// The files and directories `build.rs` reruns on: the workspace manifest and
/// lock file, and each exporter crate's `Cargo.toml`, `build.rs`, `src/` and
/// `build_support/` (directories also catch added and removed files).
/// Used by `build.rs`; the other includers of this file only hash.
#[allow(dead_code)]
pub fn watched_paths(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = vec![root.join("Cargo.toml"), root.join("Cargo.lock")];
    for dir in exporter_crates(root)? {
        out.extend(
            ["Cargo.toml", "build.rs", "src", "build_support"].iter().map(|n| dir.join(n)).filter(|p| p.exists()),
        );
    }
    Ok(out)
}

pub fn source_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                collect(&entry.path(), files)?;
            } else if entry.file_type()?.is_file() {
                files.push(entry.path());
            }
        }
        Ok(())
    }
    let mut files = vec![root.join("Cargo.toml"), root.join("Cargo.lock")];
    for dir in exporter_crates(root)? {
        for name in ["Cargo.toml", "build.rs"] {
            let path = dir.join(name);
            if path.is_file() {
                files.push(path);
            }
        }
        for name in ["src", "build_support"] {
            let path = dir.join(name);
            if path.is_dir() {
                collect(&path, &mut files)?;
            }
        }
    }
    files.sort();
    Ok(files)
}

pub fn source_digest(root: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    for path in source_files(root)? {
        let relative = path
            .strip_prefix(root)
            .expect("source under workspace")
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF-8 source path"))?
            .replace('\\', "/");
        let bytes = fs::read(&path)?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
