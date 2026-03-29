use std::path::{Path, PathBuf};

use starbreaker_chf::{ChfFile, parse_chf, write_chf};

fn fixture_base() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .join("..")
        .join("..")
        .join("research")
}

fn collect_files_recursive(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !dir.exists() {
        return files;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_files_recursive(&path, extension));
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            files.push(path);
        }
    }
    files
}

fn get_all_files(extension: &str) -> Vec<PathBuf> {
    let base = fixture_base();
    let local = base.join("localCharacters");
    let website = base.join("websiteCharacters");

    let mut files = Vec::new();

    if local.exists() {
        files.extend(collect_files_recursive(&local, extension));
    }
    if website.exists() {
        files.extend(collect_files_recursive(&website, extension));
    }

    files
}

#[test]
fn parse_all_chf_files() {
    let files = get_all_files("chf");
    if files.is_empty() {
        eprintln!("SKIP: no .chf files found in research directories");
        return;
    }

    let total = files.len();
    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    for path in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                failures.push((path.clone(), format!("read error: {e}")));
                continue;
            }
        };
        let chf_file = match ChfFile::from_chf(&bytes) {
            Ok(f) => f,
            Err(e) => {
                failures.push((path.clone(), format!("container error: {e}")));
                continue;
            }
        };
        if let Err(e) = chf_file.parse() {
            failures.push((path.clone(), format!("parse error: {e}")));
        }
    }

    assert!(total > 0, "expected to find .chf files");

    if !failures.is_empty() {
        let mut msg = format!("{} of {} .chf files failed:\n", failures.len(), total);
        for (path, err) in &failures {
            msg.push_str(&format!("  {}: {}\n", path.display(), err));
        }
        panic!("{msg}");
    }

    eprintln!("OK: parsed {total} .chf files successfully");
}

#[test]
fn round_trip_all_bin_files() {
    let files = get_all_files("bin");
    if files.is_empty() {
        eprintln!("SKIP: no .bin files found in research directories");
        return;
    }

    let total = files.len();
    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    for path in &files {
        let bin_data = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                failures.push((path.clone(), format!("read error: {e}")));
                continue;
            }
        };
        let chf_data = match parse_chf(&bin_data) {
            Ok(d) => d,
            Err(e) => {
                failures.push((path.clone(), format!("parse error: {e}")));
                continue;
            }
        };
        let written = write_chf(&chf_data);
        if bin_data != written {
            let first_diff = bin_data
                .iter()
                .zip(written.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(bin_data.len().min(written.len()));
            failures.push((
                path.clone(),
                format!(
                    "round-trip mismatch: original {} bytes, written {} bytes, first diff at offset {first_diff}",
                    bin_data.len(),
                    written.len()
                ),
            ));
        }
    }

    assert!(total > 0, "expected to find .bin files");

    if !failures.is_empty() {
        let mut msg = format!(
            "{} of {} .bin files failed round-trip:\n",
            failures.len(),
            total
        );
        for (path, err) in &failures {
            msg.push_str(&format!("  {}: {}\n", path.display(), err));
        }
        panic!("{msg}");
    }

    eprintln!("OK: round-tripped {total} .bin files successfully");
}
