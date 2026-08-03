//! P4K-backed [`SwfFetcher`] and [`AssetFetcher`] implementations.
//!
//! [`P4kSwfFetcher`] serves SWF bytes and enumerates SWF subdirectories from
//! the live P4K archive. [`P4kAssetFetcher`] serves DDS/PNG/SVG bytes, trying
//! a set of path candidates and the `Data\` prefix variant for each.

use starbreaker_p4k::MappedP4k;
use starbreaker_ui::{UiError, pipeline::SwfFetcher};

pub(super) struct P4kSwfFetcher<'a> {
    pub(super) p4k: &'a MappedP4k,
}

impl<'a> SwfFetcher for P4kSwfFetcher<'a> {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError> {
        // Binary search per candidate (same shape as `read_p4k_asset` /
        // `asset_exists`) instead of the old O(all archive entries) linear
        // scan per fetch. Candidate order is [native, Data\native], so the
        // `Data\`-prefixed real location still wins whenever only it exists.
        let candidates = p4k_swf_candidates(p4k_path);
        let entry = candidates
            .iter()
            .find_map(|candidate| self.p4k.entry_case_insensitive(candidate))
            .ok_or_else(|| UiError::FetchFailed {
                guid: p4k_path.to_string(),
                source: format!("SWF not found in P4K: {p4k_path}").into(),
            })?;
        self.p4k.read(entry).map_err(|e| UiError::FetchFailed {
            guid: p4k_path.to_string(),
            source: Box::new(e),
        })
    }

    /// Enumerate immediate child directory names under `prefix` from the live
    /// P4K entry list.  This is what makes the Phase-1 deterministic SWF
    /// resolver work in production: without it the default empty implementation
    /// would yield no ship-subdir candidates and every ship-subdir SWF (target
    /// MFD, annunciators) would be unfindable.
    fn list_swf_dirs(&self, prefix: &str) -> Vec<String> {
        swf_immediate_subdirs(self.p4k.entries().iter().map(|entry| entry.name.as_str()), prefix)
    }
}

/// Immediate child directory names directly under `prefix`, matched
/// case-insensitively against native (`\`-separated) P4K entry names.
///
/// Returned names preserve their original casing and are deduped + sorted.
/// `prefix` is expected to end with a `\` (the directory whose children are
/// listed, e.g. `Data\UI\ShipInterface\assets\SWF\DRA\`).  Entries that are
/// direct files of `prefix` (no further separator) are not directories and are
/// skipped.  Matching is case-insensitive because P4K entry casing is not
/// guaranteed to match the resolver's constructed prefix (the same reason
/// `fetch_swf_bytes` compares with `eq_ignore_ascii_case`).
pub(super) fn swf_immediate_subdirs<'a>(names: impl Iterator<Item = &'a str>, prefix: &str) -> Vec<String> {
    // An empty prefix would match every entry and enumerate the whole archive's
    // top-level directories; callers always pass a concrete directory path.
    if prefix.is_empty() {
        return Vec::new();
    }
    let plen = prefix.len();
    let mut seen = std::collections::BTreeSet::new();
    for name in names {
        if name.len() <= plen {
            continue;
        }
        if !name.as_bytes()[..plen].eq_ignore_ascii_case(prefix.as_bytes()) {
            continue;
        }
        let rest = &name[plen..];
        // A subdirectory child has at least one further separator after its name;
        // an entry with no further `\` is a direct file, not a directory.
        if let Some(sep) = rest.find('\\') {
            let subdir = &rest[..sep];
            if !subdir.is_empty() {
                seen.insert(subdir.to_string());
            }
        }
    }
    seen.into_iter().collect()
}

pub(super) fn p4k_swf_candidates(path: &str) -> Vec<String> {
    let native = path.replace('/', "\\");
    let mut candidates = vec![native.clone()];
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with("data/") && !lower.starts_with("data\\") {
        candidates.push(format!("Data\\{native}"));
    }
    candidates
}

pub(super) struct P4kAssetFetcher<'a> {
    pub(super) p4k: &'a MappedP4k,
}

impl<'a> starbreaker_ui::bb_atlas::AssetFetcher for P4kAssetFetcher<'a> {
    fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
        read_p4k_asset(self.p4k, p4k_path)
    }

    fn asset_exists(&self, p4k_path: &str) -> bool {
        p4k_asset_candidates(p4k_path)
            .iter()
            .any(|candidate| self.p4k.entry_case_insensitive(candidate).is_some())
    }
}

pub(super) fn read_p4k_asset(p4k: &MappedP4k, p4k_path: &str) -> Option<Vec<u8>> {
    for candidate in p4k_asset_candidates(p4k_path) {
        if let Ok(bytes) = p4k.read_file(&candidate) {
            return Some(bytes);
        }
    }
    None
}

pub(super) fn p4k_asset_candidates(path: &str) -> Vec<String> {
    fn push_with_data_prefix(candidates: &mut Vec<String>, candidate: String) {
        if !candidates.iter().any(|existing| existing.eq_ignore_ascii_case(&candidate)) {
            candidates.push(candidate.clone());
        }
        let lower = candidate.to_ascii_lowercase();
        if !lower.starts_with("data\\") {
            let prefixed = format!("Data\\{candidate}");
            if !candidates
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&prefixed))
            {
                candidates.push(prefixed);
            }
        }
    }

    let native = path.replace('/', "\\");
    let normalised = starbreaker_ui::bb_assets::UiAssetResolver::normalise_path(path)
        .replace('/', "\\");
    let mut candidates = Vec::new();
    for seed in [native, normalised] {
        push_with_data_prefix(&mut candidates, seed.clone());
        if seed.to_ascii_lowercase().ends_with(".tif") {
            if let Some(stem) = seed.strip_suffix(".tif") {
                push_with_data_prefix(&mut candidates, format!("{stem}.dds"));
            } else if let Some(stem) = seed.strip_suffix(".TIF") {
                push_with_data_prefix(&mut candidates, format!("{stem}.dds"));
            }
        }
    }
    candidates
}
