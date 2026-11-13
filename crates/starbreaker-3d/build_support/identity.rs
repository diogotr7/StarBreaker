//! Reproducible exporter source identity shared by the build and UI stale-export guard.
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

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
    for entry in fs::read_dir(root.join("crates"))? {
        let dir = entry?.path();
        if !dir.is_dir() {
            continue;
        }
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
