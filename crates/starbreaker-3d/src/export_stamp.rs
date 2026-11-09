//! Deterministic decomposed-export provenance: archive, exporter, entity, options and UI bytes.
use crate::{error::Error, pipeline::ExportOptions};
use serde::Serializer;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use starbreaker_p4k::MappedP4k;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::Path,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) const SOURCE_SHA256: &str = env!("STARBREAKER_SOURCE_SHA256");

pub(crate) fn serialize_socpak_filter<S: Serializer>(
    filter: &Option<HashSet<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::Serialize;
    filter
        .as_ref()
        .map(|set| set.iter().collect::<BTreeSet<_>>())
        .serialize(serializer)
}

fn manifest_build_id(path: &Path) -> Result<Option<String>, Error> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Other(format!("reading archive build manifest: {e}"))),
    };
    let value: Value = serde_json::from_slice(&bytes)?;
    value["Data"]["BuildId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| Some(s.to_owned()))
        .ok_or_else(|| Error::Other("build_manifest.id missing Data.BuildId".into()))
}

pub(crate) fn archive_build_id(p4k: &MappedP4k) -> Result<String, Error> {
    if let Some(id) = manifest_build_id(&p4k.path().with_file_name("build_manifest.id"))? {
        return Ok(id);
    }
    // Portable archive copies may omit the install manifest. Derive an identity
    // from the archive directory, never the host path or filesystem timestamps.
    let mut entries: Vec<_> = p4k.entries().iter().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let mut hash = Sha256::new();
    for e in entries {
        hash.update((e.name.len() as u64).to_le_bytes());
        hash.update(e.name.as_bytes());
        hash.update(e.crc32.to_le_bytes());
        hash.update(e.uncompressed_size.to_le_bytes());
        hash.update(e.compressed_size.to_le_bytes());
        hash.update(e.compression_method.to_le_bytes());
        hash.update(e.offset.to_le_bytes());
        hash.update([u8::from(e.is_encrypted)]);
    }
    Ok(format!("directory-sha256:{}", hex(&hash.finalize())))
}

pub(crate) fn stamp<'a>(
    build_id: &str,
    entity: &str,
    options: &ExportOptions,
    files: impl Iterator<Item = (&'a str, &'a [u8])>,
) -> Result<Value, Error> {
    let generated_files: BTreeMap<_, _> = files
        .map(|(name, bytes)| (name.to_owned(), hex(&Sha256::digest(bytes))))
        .collect();
    Ok(json!({
        "schema_version": 2,
        "archive_build_id": build_id,
        "exporter": { "name": "starbreaker-3d", "version": env!("CARGO_PKG_VERSION"), "source_sha256": SOURCE_SHA256 },
        "entity": entity,
        "options": serde_json::to_value(options)?,
        "generated_files": generated_files,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stamp_is_stable_and_tracks_inputs_including_unordered_filters() {
        let mut options = ExportOptions::default();
        options.socpak_path_filter = Some(["synthetic/b", "synthetic/a"].map(str::to_owned).into());
        let make = |build, entity, opts: &ExportOptions| {
            stamp(
                build,
                entity,
                opts,
                [("Data/UI/Generated/synthetic.png", &[1, 2][..])].into_iter(),
            )
            .unwrap()
        };
        let first = make("build-a", "entity-a", &options);
        options.socpak_path_filter = Some(["synthetic/a", "synthetic/b"].map(str::to_owned).into());
        assert_eq!(first, make("build-a", "entity-a", &options));
        assert_ne!(first, make("build-b", "entity-a", &options));
        assert_ne!(first, make("build-a", "entity-b", &options));
        options.include_animations = !options.include_animations;
        assert_ne!(first, make("build-a", "entity-a", &options));
        assert!(first.get("written_at_epoch_s").is_none());
        assert_eq!(first["exporter"]["source_sha256"], SOURCE_SHA256);
    }
}
