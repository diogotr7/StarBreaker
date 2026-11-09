//! Content-based stale-export validation for schema-v2 stamps; no timestamp tolerance.
#[path = "../../../starbreaker-3d/build_support/identity.rs"]
mod identity;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn check_values(
    stamp: &Value,
    expected_source: &str,
    files: &[(String, String)],
) -> Result<(), String> {
    if stamp["schema_version"] != 2 || stamp["archive_build_id"].as_str().is_none_or(str::is_empty)
    {
        return Err("invalid deterministic export stamp".into());
    }
    if stamp["exporter"]["source_sha256"] != expected_source {
        return Err("exporter source identity changed; re-export".into());
    }
    for (path, digest) in files {
        if stamp["generated_files"][path] != *digest {
            return Err(format!("{path}: generated file differs from export stamp"));
        }
    }
    Ok(())
}

pub fn check(stamp: &Value, export_root: &Path, sources: &[PathBuf]) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let expected = identity::source_digest(&root).map_err(|e| e.to_string())?;
    let files = sources
        .iter()
        .map(|path| {
            let relative = path
                .strip_prefix(export_root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("non-UTF-8 generated path")?
                .replace('\\', "/");
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let digest = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            Ok((relative, digest))
        })
        .collect::<Result<Vec<_>, String>>()?;
    check_values(stamp, &expected, &files)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp() -> Value {
        serde_json::json!({"schema_version": 2, "archive_build_id": "synthetic-build",
            "exporter": {"source_sha256": "synthetic-source"},
            "generated_files": {"synthetic.png": "synthetic-digest"}})
    }
    #[test]
    fn deterministic_stamp_accepts_matching_source_and_bytes_without_clocks() {
        assert!(
            check_values(
                &stamp(),
                "synthetic-source",
                &[("synthetic.png".into(), "synthetic-digest".into())]
            )
            .is_ok()
        );
    }
    #[test]
    fn deterministic_stamp_rejects_changed_exporter() {
        assert!(
            check_values(&stamp(), "other-source", &[])
                .unwrap_err()
                .contains("source identity")
        );
    }
    #[test]
    fn deterministic_stamp_rejects_replaced_or_missing_generated_files() {
        for name in ["synthetic.png", "missing.png"] {
            assert!(
                check_values(
                    &stamp(),
                    "synthetic-source",
                    &[(name.into(), "replaced".into())]
                )
                .is_err()
            );
        }
    }
}
