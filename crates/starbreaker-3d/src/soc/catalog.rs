//! Data-driven scene catalog enumeration.
//!
//! A "scene root" socpak is one that no other socpak references as a
//! `<Child>` in its main XML. Sub-modules (interior containers, base
//! geometry, kit pieces) are referenced by parent socpaks and therefore
//! show up as graph nodes with a non-zero in-degree. The roots are
//! exactly the in-degree-zero nodes -- they are the entries a user can
//! sensibly load with `compose_from_root` and get a complete level.
//!
//! The algorithm is:
//!
//! 1. Walk every `*.socpak` under the requested search roots.
//! 2. For each, parse its main XML and extract its `<Child>` socpak
//!    references (reusing [`super::scene::read_child_refs_from_socpak`]).
//! 3. Build a `path -> Vec<child_path>` graph, plus a set of every path
//!    that appeared as somebody's child.
//! 4. Roots = `(set of all socpak paths) - (set of every referenced
//!    path)`.
//! 5. For each root: derive a display name (XML `name=` attribute on the
//!    top-level node, falling back to filename stem), and compute the
//!    transitive child count via BFS.
//!
//! The walk is fail-soft: a single broken socpak (unreadable, malformed
//! zip, missing main XML) is logged at warn level and skipped. The rest
//! of the enumeration continues. This keeps a single corrupted entry
//! from blanking the catalog.
//!
//! # Initial filtering
//!
//! The catalog applies two light filters out of the gate:
//!
//! - **Minimum size.** Socpaks below 100 KB compressed are typically
//!   empty containers (placeholders, test stubs). They have no brushes
//!   and rendering them produces an empty scene, which is worse than
//!   not listing them.
//! - **Test-pattern names.** Paths or filenames that case-insensitively
//!   match `test`, `tmp`, or `backup` are skipped. These are
//!   developer-only assets the engine never ships into a production
//!   level.
//!
//! Anything else is kept. The user explicitly chose a liberal initial
//! pass over a curated catalog -- we expect to over-list and rely on
//! follow-up filtering iterations to add precision.

use std::collections::{HashMap, HashSet, VecDeque};

use starbreaker_p4k::MappedP4k;

use super::scene::{SceneError, read_child_refs_from_socpak};

// ── Lazy directory-tree listing ─────────────────────────────────────────────
//
// The catalog UI used to call `enumerate_scene_roots`, which walks every
// `.socpak` under `Data\ObjectContainers\` and parses each one's child
// refs to build a graph. On the live HOTFIX archive that is over a
// thousand zip-within-zip parses on every cold call -- enough to lock
// the Tauri webview while it runs.
//
// The lazy alternative is a directory-tree model: list one prefix at a
// time, return immediate children only, expand on demand. The composer
// downstream of `loadSceneToGltf` already walks down from whatever
// socpak root it is handed, so the "in-degree zero" classification was
// only ever needed to seed the UI -- it is not load-bearing for the
// load path. Dropping it makes the catalog feel instant.

/// One entry returned by [`list_socpak_dir`]. Either an immediate
/// subdirectory under the listed prefix (`Directory` -- no I/O has been
/// done against its contents yet) or a `.socpak` file directly under the
/// prefix (`SocpakFile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocpakDirEntry {
    /// Full p4k path. For directories this is the prefix the caller
    /// should pass back into [`list_socpak_dir`] to expand
    /// (backslash-terminated). For socpak files this is the full file
    /// path -- the same string [`super::compose_from_root`] accepts.
    pub path: String,
    /// Last path segment, suitable for rendering directly in a UI.
    pub display_name: String,
    pub kind: SocpakDirEntryKind,
    /// For directories: count of immediate children (subdirs + socpaks).
    /// For socpak files: file size in bytes (compressed).
    pub size_or_count: u64,
}

/// What kind of node [`SocpakDirEntry`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocpakDirEntryKind {
    Directory,
    SocpakFile,
}

/// List the immediate children of a p4k directory prefix.
///
/// `prefix` is a p4k-internal directory path. Slashes vs. backslashes
/// do not matter, and a trailing separator is optional -- the function
/// normalises both (`"Data/ObjectContainers/"`,
/// `"Data\\ObjectContainers\\"`, and `"Data/ObjectContainers"` are
/// equivalent). Returns subdirectories (one per unique immediate child
/// path segment that has further descendants) and any `.socpak` files
/// that live directly under the prefix. Files that are not socpaks are
/// ignored -- this is the catalog's view of the tree, not a generic
/// p4k browser.
///
/// The returned list is sorted: directories first (alphabetical), then
/// socpak files (alphabetical). Both sort case-insensitively.
///
/// Performance: O(N) over the entries stored under `prefix` thanks to
/// [`MappedP4k::list_dir`] using a binary-search-narrowed sorted index.
/// Listing the ~10 entries under `Data\\ObjectContainers\\` on the live
/// HOTFIX archive returns in well under a millisecond.
pub fn list_socpak_dir(p4k: &MappedP4k, prefix: &str) -> Vec<SocpakDirEntry> {
    // Lift the closure-based core out of the production p4k so the
    // build / sort logic is testable against synthetic backings.
    list_socpak_dir_impl(prefix, |path| {
        p4k.list_dir(path)
            .into_iter()
            .map(|e| match e {
                starbreaker_p4k::DirEntry::Directory(name) => DirChild::Directory(name),
                starbreaker_p4k::DirEntry::File(entry) => DirChild::File {
                    name: entry.name.clone(),
                    compressed_size: entry.compressed_size,
                },
            })
            .collect::<Vec<_>>()
    })
}

/// Owned mirror of [`starbreaker_p4k::DirEntry`] used by
/// [`list_socpak_dir_impl`]. The production p4k borrows from its entry
/// slice; tests want owned data without round-tripping through
/// `&P4kEntry`. Kept module-private -- this is a test seam, not a
/// public API.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DirChild {
    Directory(String),
    File { name: String, compressed_size: u64 },
}

/// Closure-driven body of [`list_socpak_dir`]. Splitting it out lets the
/// tests stub `list_dir` with synthetic data. The closure is called once
/// for the listed prefix and once per immediate subdirectory (to compute
/// child counts for the directory badge).
fn list_socpak_dir_impl<F>(prefix: &str, mut list_dir: F) -> Vec<SocpakDirEntry>
where
    F: FnMut(&str) -> Vec<DirChild>,
{
    // Normalise the caller's prefix:
    //   - swap forward slashes for the backslashes the p4k entries use
    //   - trim a single trailing separator so `list_dir` can append its
    //     own (it requires a non-terminated input)
    let mut normalised = prefix.replace('/', "\\");
    if normalised.ends_with('\\') {
        normalised.pop();
    }

    let mut directories: Vec<SocpakDirEntry> = Vec::new();
    let mut socpaks: Vec<SocpakDirEntry> = Vec::new();

    // For each immediate child of the prefix:
    //   - Directory -> count its own immediate children for the badge.
    //     This is one extra `list_dir` call per directory; cheap because
    //     each is a binary-search-narrowed scan.
    //   - File -> only keep `.socpak` leaves; record compressed size.
    for child in list_dir(&normalised) {
        match child {
            DirChild::Directory(name) => {
                let full_path = if normalised.is_empty() {
                    format!("{name}\\")
                } else {
                    format!("{normalised}\\{name}\\")
                };
                let dir_for_count = full_path.trim_end_matches('\\');
                // Count only socpak-relevant children: subdirs and
                // `.socpak` leaves. A directory full of unrelated files
                // (e.g. embedded XML / DDS) reports zero, which lets the
                // UI hide a useless expand chevron.
                let count = list_dir(dir_for_count)
                    .into_iter()
                    .filter(|e| match e {
                        DirChild::Directory(_) => true,
                        DirChild::File { name, .. } => {
                            name.to_ascii_lowercase().ends_with(".socpak")
                        }
                    })
                    .count() as u64;
                directories.push(SocpakDirEntry {
                    path: full_path,
                    display_name: name,
                    kind: SocpakDirEntryKind::Directory,
                    size_or_count: count,
                });
            }
            DirChild::File {
                name,
                compressed_size,
            } => {
                let name_lc = name.to_ascii_lowercase();
                if !name_lc.ends_with(".socpak") {
                    continue;
                }
                let leaf = name
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&name)
                    .to_string();
                socpaks.push(SocpakDirEntry {
                    path: name,
                    display_name: leaf,
                    kind: SocpakDirEntryKind::SocpakFile,
                    size_or_count: compressed_size,
                });
            }
        }
    }

    directories.sort_by(|a, b| {
        a.display_name
            .to_ascii_lowercase()
            .cmp(&b.display_name.to_ascii_lowercase())
    });
    socpaks.sort_by(|a, b| {
        a.display_name
            .to_ascii_lowercase()
            .cmp(&b.display_name.to_ascii_lowercase())
    });

    let mut out = Vec::with_capacity(directories.len() + socpaks.len());
    out.extend(directories);
    out.extend(socpaks);
    out
}

// ── Global path index ───────────────────────────────────────────────────────
//
// `list_all_socpaks` is a flat-list cousin of `list_socpak_dir`: it walks
// every entry in the loaded p4k once and returns the path of every
// `.socpak` file under any of `search_roots`. No chunk parsing, no graph
// traversal -- just a linear scan that costs whatever a `to_ascii_lowercase`
// per entry costs. Intended for the Maps tab's "search everywhere" mode,
// where the tree's branch-by-branch filter cannot find a path the user
// has not yet expanded.
//
// Returned paths are sorted alphabetically (case-insensitive) so callers
// can do straight substring matching without extra sort work.

/// Enumerate every `.socpak` path under the requested search roots.
///
/// `search_roots` are p4k-internal directory prefixes (e.g.
/// `"Data/ObjectContainers/"`). Slash flavour and trailing-separator are
/// normalised internally. Pass an empty slice to scan the entire archive
/// (rarely useful: socpaks live almost exclusively under
/// `Data\ObjectContainers\`).
///
/// The returned list is sorted by path, case-insensitively. The function
/// does not parse any socpak content -- if the caller needs sub-zone
/// counts or display names, [`enumerate_scene_roots`] is the right tool.
///
/// Performance: O(N) over `p4k.entries()` plus an alphabetical sort over
/// the matched subset. On the live HOTFIX archive (~1700 socpaks under
/// the standard root) this completes in a few hundred milliseconds at
/// most -- the dominant cost is the `to_ascii_lowercase` per entry.
pub fn list_all_socpaks(p4k: &MappedP4k, search_roots: &[&str]) -> Vec<String> {
    list_all_socpaks_impl(search_roots, p4k.entries().iter().map(|e| e.name.as_str()))
}

/// Iterator-driven body of [`list_all_socpaks`]. Splitting it out lets
/// the tests pass synthetic entry names without round-tripping through
/// a `MappedP4k`.
fn list_all_socpaks_impl<'a, I>(search_roots: &[&str], names: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let normalised_roots: Vec<String> = search_roots.iter().map(|r| normalise_root(r)).collect();

    let mut out: Vec<String> = Vec::new();
    for name in names {
        let name_lc = name.to_ascii_lowercase();
        if !name_lc.ends_with(".socpak") {
            continue;
        }
        if !normalised_roots.is_empty() {
            // Compare against the canonical form (no leading `data\`)
            // so `"Data/ObjectContainers/"` and the raw entry path with
            // the `Data\` prefix line up the same way the graph
            // enumeration does.
            let canon = canonicalise_path(name);
            if !normalised_roots.iter().any(|r| canon.starts_with(r)) {
                continue;
            }
        }
        out.push(name.to_string());
    }

    out.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
    out
}

// ── Filter constants ────────────────────────────────────────────────────────

/// Skip socpaks whose compressed size on disk is below this threshold.
/// Real scenes carry at least one populated SOC payload; anything
/// smaller is a placeholder. 100 KB is conservative -- the smallest
/// actual scene socpak we have observed sits around 250 KB.
pub const MIN_SOCPAK_SIZE_BYTES: u64 = 100 * 1024;

/// Lower-cased substring patterns that mark a socpak as test / scratch /
/// backup data. A path or filename containing any of these is skipped.
const TEST_PATH_PATTERNS: &[&str] = &["test", "tmp", "backup"];

// ── Public types ────────────────────────────────────────────────────────────

/// Source of the catalog entry. Today every entry is `GraphRoot`; the
/// `Other` variant is a placeholder for future heuristic-augmented
/// sources (XML metadata tags, DataCore-driven catalogs, etc.) so we
/// can introduce new sources without a contract bump on the
/// frontend-facing JSON.
///
/// The `as_snake_case` accessor returns the same string the Tauri
/// command serialises (kept manual so the consumer crate does not
/// have to take a serde dependency through the catalog API).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneSourceKind {
    /// Found by graph traversal, in-degree 0 in the reference graph.
    GraphRoot,
    /// Reserved for future heuristic-augmented sources.
    Other,
}

impl SceneSourceKind {
    /// Stable snake-case identifier for serialisation. Mirrors what a
    /// `#[serde(rename_all = "snake_case")]` derive would produce, so
    /// future migration is a one-line swap.
    pub fn as_snake_case(self) -> &'static str {
        match self {
            SceneSourceKind::GraphRoot => "graph_root",
            SceneSourceKind::Other => "other",
        }
    }
}

/// One catalog entry the frontend will offer to the user.
#[derive(Debug, Clone)]
pub struct SceneCatalogEntry {
    /// Canonical p4k path of the root socpak (backslash-separated, with
    /// the leading `Data\` segment).
    pub path: String,
    /// Best-effort human-readable display name.
    pub display_name: String,
    /// Count of transitively-reachable child socpaks. Useful as a
    /// rough "scene complexity" hint: leaf modules end up at 0,
    /// assembly trees with one tier of children land in single
    /// digits, multi-zone hangars and dungeons climb into the dozens.
    pub sub_zone_count: usize,
    /// Provenance of this entry. See [`SceneSourceKind`].
    pub source_kind: SceneSourceKind,
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Walk the loaded p4k under each of `search_roots`, find every
/// `*.socpak`, and return the in-degree-zero nodes of the reference
/// graph as catalog entries.
///
/// `search_roots` are p4k-internal directory prefixes (e.g.
/// `"Data/ObjectContainers/"`). Slashes vs. backslashes do not matter --
/// the function normalises both. Pass an empty slice to scan the entire
/// archive; that is rarely what you want because socpaks live almost
/// exclusively under `Data\ObjectContainers\`.
///
/// The returned list is sorted by `display_name` (case-insensitive)
/// before being handed back, so downstream UIs do not have to re-sort.
pub fn enumerate_scene_roots(
    p4k: &MappedP4k,
    search_roots: &[&str],
) -> Result<Vec<SceneCatalogEntry>, SceneError> {
    // Phase 1: collect every candidate socpak that passes the cheap
    // filename / size filter. Map keys are canonical (lowercased,
    // backslash-normalised) paths so the graph dedupes case variants;
    // the value carries the original-case path so we can return it
    // verbatim to the frontend (some SOC paths preserve case for
    // display purposes).
    let normalised_roots: Vec<String> = search_roots.iter().map(|r| normalise_root(r)).collect();

    let mut original_path: HashMap<String, String> = HashMap::new();
    for entry in p4k.entries() {
        let name_lc = entry.name.to_ascii_lowercase();
        if !name_lc.ends_with(".socpak") {
            continue;
        }
        // Compare against the canonical form (no `data\` prefix) so
        // root specs like `"Data/ObjectContainers/"` line up with
        // p4k entry names like `"Data\ObjectContainers\..."`.
        let canon_name = canonicalise_path(&entry.name);
        if !normalised_roots.is_empty()
            && !normalised_roots.iter().any(|r| canon_name.starts_with(r))
        {
            continue;
        }
        if entry.compressed_size < MIN_SOCPAK_SIZE_BYTES {
            continue;
        }
        if matches_test_pattern(&name_lc) {
            continue;
        }
        original_path
            .entry(canon_name)
            .or_insert_with(|| entry.name.clone());
    }

    // Phase 2: parse each candidate's child refs. Soft-fail on parse
    // errors so one broken socpak cannot abort the whole scan. The
    // resulting graph keys / values are canonicalised so cross-socpak
    // references with different casing line up.
    let mut graph: HashMap<String, Vec<String>> = HashMap::with_capacity(original_path.len());
    let mut referenced: HashSet<String> = HashSet::new();
    for (key, original) in &original_path {
        let children = match read_child_refs_from_socpak(p4k, original) {
            Ok(refs) => refs,
            Err(err) => {
                log::warn!(
                    "scene-catalog: failed to read child refs from {original}: {err}"
                );
                graph.insert(key.clone(), Vec::new());
                continue;
            }
        };
        let mut child_keys: Vec<String> = Vec::with_capacity(children.len());
        for child in children {
            let child_key = canonicalise_path(&child.name);
            referenced.insert(child_key.clone());
            child_keys.push(child_key);
        }
        graph.insert(key.clone(), child_keys);
    }

    // Phase 3: roots are nodes whose canonical key is not in the
    // referenced set.
    let mut entries: Vec<SceneCatalogEntry> = Vec::new();
    for (key, original) in &original_path {
        if referenced.contains(key) {
            continue;
        }
        let display_name = derive_display_name(p4k, original);
        let sub_zone_count = transitive_child_count(&graph, key);
        entries.push(SceneCatalogEntry {
            path: original.clone(),
            display_name,
            sub_zone_count,
            source_kind: SceneSourceKind::GraphRoot,
        });
    }

    entries.sort_by(|a, b| {
        a.display_name
            .to_ascii_lowercase()
            .cmp(&b.display_name.to_ascii_lowercase())
    });

    Ok(entries)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Lowercase a path, replace forward slashes with backslashes, and
/// strip any leading `data\` so two casings of the same path hash
/// identically. Returned values are stable canonical keys for the
/// graph; the original-case string is kept separately for display.
fn canonicalise_path(path: &str) -> String {
    let lowered = path.to_ascii_lowercase().replace('/', "\\");
    lowered
        .strip_prefix("data\\")
        .map(|s| s.to_string())
        .unwrap_or(lowered)
}

/// Normalise a search root the same way [`canonicalise_path`] does
/// internal nodes, then ensure it ends with a backslash so prefix
/// matches stay anchored at directory boundaries (otherwise
/// `"objectcontainers"` would match a path `"objectcontainers_old\..."`).
fn normalise_root(root: &str) -> String {
    if root.is_empty() {
        return String::new();
    }
    let mut canon = canonicalise_path(root);
    if !canon.ends_with('\\') {
        canon.push('\\');
    }
    canon
}

/// Return true when the (lower-cased) path contains any test-pattern
/// substring.
fn matches_test_pattern(path_lc: &str) -> bool {
    TEST_PATH_PATTERNS.iter().any(|pat| path_lc.contains(pat))
}

/// Compute the number of socpaks reachable through the child graph
/// from `start`, exclusive of the start itself. Uses a visited-set BFS
/// so cycles or shared submodules count exactly once. Missing graph
/// edges (children that reference socpaks that did not pass the size
/// or test-pattern filter, and therefore have no entry in `graph`)
/// stop the walk on that branch -- they would not be loadable by the
/// composer anyway.
fn transitive_child_count(graph: &HashMap<String, Vec<String>>, start: &str) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    if let Some(children) = graph.get(start) {
        for c in children {
            if seen.insert(c.clone()) {
                queue.push_back(c.clone());
            }
        }
    }

    while let Some(node) = queue.pop_front() {
        if let Some(children) = graph.get(&node) {
            for c in children {
                if seen.insert(c.clone()) {
                    queue.push_back(c.clone());
                }
            }
        }
    }

    seen.len()
}

/// Derive a display name for a root socpak. Tries:
///
/// 1. The `name=` attribute on the top-level XML node, when present.
/// 2. The socpak filename stem.
/// 3. The full path (only if the stem is empty, which should never
///    happen for a real p4k entry).
fn derive_display_name(p4k: &MappedP4k, socpak_path: &str) -> String {
    if let Some(name) = read_root_name_attr(p4k, socpak_path) {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let stem = filename_stem(socpak_path);
    if !stem.is_empty() {
        return stem;
    }
    socpak_path.to_string()
}

/// Return the filename stem (last path segment with any `.socpak`
/// suffix removed). Tolerant of either separator.
fn filename_stem(path: &str) -> String {
    let tail = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = tail
        .strip_suffix(".socpak")
        .or_else(|| tail.strip_suffix(".SOCPAK"))
        .unwrap_or(tail);
    stem.to_string()
}

/// Try to read the `name=` attribute on the socpak's top-level XML
/// node. Returns `None` for any failure -- callers fall back to the
/// filename stem.
fn read_root_name_attr(p4k: &MappedP4k, socpak_path: &str) -> Option<String> {
    let entry = p4k.entry_case_insensitive(socpak_path)?;
    let socpak_bytes = p4k.read(entry).ok()?;
    let inner = starbreaker_p4k::P4kArchive::from_bytes(&socpak_bytes).ok()?;

    // Find the main XML using the same heuristic the composer uses
    // (first non-editor / non-metadata `.xml` at the socpak root).
    let mut xml_entry = None;
    for e in inner.entries() {
        let name_lc = e.name.to_ascii_lowercase();
        if !name_lc.ends_with(".xml") {
            continue;
        }
        if name_lc.contains("editor")
            || name_lc.contains("metadata")
            || name_lc.contains("entdata")
            || name_lc.contains("entxml")
        {
            continue;
        }
        xml_entry = Some(e);
        break;
    }
    let xml_bytes = inner.read(xml_entry?).ok()?;

    extract_root_name(&xml_bytes)
}

/// Extract the `name=` attribute on whatever the top-level / root XML
/// node turns out to be. Handles both CryXmlB and plain-text XML.
fn extract_root_name(xml_bytes: &[u8]) -> Option<String> {
    if starbreaker_cryxml::is_cryxmlb(xml_bytes) {
        let xml = starbreaker_cryxml::from_bytes(xml_bytes).ok()?;
        let root = xml.root();
        for (k, v) in xml.node_attributes(root) {
            if k.eq_ignore_ascii_case("name") {
                return Some(v.to_string());
            }
        }
        return None;
    }

    let text = std::str::from_utf8(xml_bytes).ok()?;
    extract_root_name_text(text)
}

/// Plain-text-XML version of the root-name extraction. Looks for the
/// first non-comment, non-PI tag and pulls its `name=` attribute. We
/// avoid pulling in a full XML parser here because the SOC main-XML
/// shape is well-defined and the engine writes its attribute strings
/// verbatim.
fn extract_root_name_text(xml: &str) -> Option<String> {
    let mut cursor = 0;
    let bytes = xml.as_bytes();
    while cursor < bytes.len() {
        // Find next '<'
        let lt = xml[cursor..].find('<')? + cursor;
        if lt + 1 >= bytes.len() {
            return None;
        }
        let next = bytes[lt + 1];
        // Skip processing instructions and comments
        if next == b'?' || next == b'!' {
            // Find matching '>' and continue
            let close = xml[lt..].find('>')? + lt;
            cursor = close + 1;
            continue;
        }
        // Found a real tag; grab its head up to the next '>' or '/'
        let close_rel = xml[lt + 1..].find('>')?;
        let block = &xml[lt + 1..lt + 1 + close_rel];
        // Pull `name="..."` out of the block.
        return extract_attr_value(block, "name");
    }
    None
}

/// Pull `key="value"` from a tag-head block. Mirrors the helper in
/// `scene.rs`. Tolerates surrounding attributes; matches on the literal
/// space-prefixed needle so a key like `pos` does not match `position`.
fn extract_attr_value(block: &str, key: &str) -> Option<String> {
    let needle_space = format!(" {key}=\"");
    let start = if let Some(idx) = block.find(&needle_space) {
        idx + needle_space.len()
    } else {
        // Fall back to the first attribute on the tag (no leading
        // space): `<Tag name="...">`.
        let needle = format!("{key}=\"");
        let idx = block.find(&needle)?;
        // Make sure this is not a substring of a longer attribute name
        // (e.g. `displayname="..."`).
        if idx > 0 {
            let prev = block.as_bytes()[idx - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                return None;
            }
        }
        idx + needle.len()
    };
    let rest = &block[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalise_path_strips_data_prefix_and_normalises_separators() {
        assert_eq!(
            canonicalise_path("Data\\ObjectContainers\\Foo.socpak"),
            "objectcontainers\\foo.socpak"
        );
        assert_eq!(
            canonicalise_path("data/objectcontainers/foo.socpak"),
            "objectcontainers\\foo.socpak"
        );
        // Already-canonical paths round-trip.
        assert_eq!(
            canonicalise_path("objectcontainers\\foo.socpak"),
            "objectcontainers\\foo.socpak"
        );
    }

    #[test]
    fn normalise_root_appends_separator_and_lowercases() {
        assert_eq!(normalise_root("Data/ObjectContainers"), "objectcontainers\\");
        assert_eq!(
            normalise_root("Data\\ObjectContainers\\"),
            "objectcontainers\\"
        );
        assert_eq!(normalise_root(""), "");
    }

    #[test]
    fn matches_test_pattern_catches_common_developer_paths() {
        assert!(matches_test_pattern("data\\objectcontainers\\test\\foo.socpak"));
        assert!(matches_test_pattern("data\\foo\\bar_tmp.socpak"));
        assert!(matches_test_pattern("data\\foo\\backup\\bar.socpak"));
        assert!(!matches_test_pattern(
            "data\\objectcontainers\\pyro\\hangar.socpak"
        ));
    }

    #[test]
    fn filename_stem_strips_socpak_suffix() {
        assert_eq!(filename_stem("Data\\foo\\bar.socpak"), "bar");
        assert_eq!(filename_stem("Data/foo/baz.socpak"), "baz");
        assert_eq!(filename_stem("naked.socpak"), "naked");
        // No suffix: pass through.
        assert_eq!(filename_stem("nothing"), "nothing");
    }

    #[test]
    fn extract_root_name_text_pulls_attribute_from_first_tag() {
        let xml = r#"<?xml version="1.0"?><Object name="Exec_Hangar" pos="0,0,0"/>"#;
        assert_eq!(
            extract_root_name_text(xml).as_deref(),
            Some("Exec_Hangar")
        );
    }

    #[test]
    fn extract_root_name_text_ignores_comments_and_processing_instructions() {
        let xml = r#"<?xml version="1.0"?>
            <!-- comment -->
            <Object name="Foo"/>
        "#;
        assert_eq!(extract_root_name_text(xml).as_deref(), Some("Foo"));
    }

    #[test]
    fn extract_root_name_text_returns_none_when_attribute_absent() {
        let xml = r#"<Object pos="0,0,0"/>"#;
        assert_eq!(extract_root_name_text(xml), None);
    }

    #[test]
    fn extract_attr_value_does_not_match_attribute_prefixes() {
        // `displayname` should not match the `name` lookup.
        let block = r#"Object displayname="Wrong" pos="0,0,0""#;
        assert_eq!(extract_attr_value(block, "name"), None);

        let block_ok = r#"Object name="Right" pos="0,0,0""#;
        assert_eq!(
            extract_attr_value(block_ok, "name"),
            Some("Right".to_string())
        );
    }

    #[test]
    fn transitive_child_count_walks_tree_uniquely() {
        // Build a small graph:
        //   root -> a, b
        //   a    -> c
        //   b    -> c        (c shared between a and b)
        //   c    -> (none)
        let mut g: HashMap<String, Vec<String>> = HashMap::new();
        g.insert("root".into(), vec!["a".into(), "b".into()]);
        g.insert("a".into(), vec!["c".into()]);
        g.insert("b".into(), vec!["c".into()]);
        g.insert("c".into(), vec![]);

        // From root: { a, b, c } = 3 unique.
        assert_eq!(transitive_child_count(&g, "root"), 3);
        // From a: just { c }.
        assert_eq!(transitive_child_count(&g, "a"), 1);
        // From a leaf: 0.
        assert_eq!(transitive_child_count(&g, "c"), 0);
    }

    #[test]
    fn transitive_child_count_handles_cycles() {
        let mut g: HashMap<String, Vec<String>> = HashMap::new();
        // a -> b -> a (cycle). Visited set guards against the loop.
        g.insert("a".into(), vec!["b".into()]);
        g.insert("b".into(), vec!["a".into()]);
        assert_eq!(transitive_child_count(&g, "a"), 2); // a, b once each
    }

    #[test]
    fn transitive_child_count_stops_at_missing_edges() {
        let mut g: HashMap<String, Vec<String>> = HashMap::new();
        // root references a child we filtered out (no entry in graph).
        g.insert("root".into(), vec!["filtered_out".into()]);
        // Walk visits the missing child once -- it counts toward
        // sub_zone_count even though we cannot recurse through it. The
        // alternative (skip-not-in-graph) would bias counts downward
        // for any root that points at a small / test-named submodule.
        assert_eq!(transitive_child_count(&g, "root"), 1);
    }

    #[test]
    fn min_socpak_size_threshold_picks_realistic_floor() {
        // Sanity: a 50 KB socpak is under the threshold; 200 KB is over.
        assert!(50 * 1024 < MIN_SOCPAK_SIZE_BYTES);
        assert!(200 * 1024 >= MIN_SOCPAK_SIZE_BYTES);
    }

    // ── Lazy directory listing (`list_socpak_dir_impl`) ────────────────────
    //
    // The production entry point (`list_socpak_dir`) takes a `MappedP4k`
    // which can only be built from a real on-disk archive, so these
    // tests target the closure-driven inner function. A small
    // `HashMap<String, Vec<DirChild>>` stands in for the p4k's directory
    // index; the closure looks up the requested directory and returns
    // an owned clone (production hands back borrowed entries -- the
    // test seam does not need that optimisation).

    fn make_dir(name: &str) -> DirChild {
        DirChild::Directory(name.to_string())
    }

    fn make_file(name: &str, size: u64) -> DirChild {
        DirChild::File {
            name: name.to_string(),
            compressed_size: size,
        }
    }

    #[test]
    fn list_socpak_dir_groups_directories_before_files() {
        // Layout under `Data\ObjectContainers`:
        //   Data\ObjectContainers\PU\          (subdir, has child)
        //   Data\ObjectContainers\Stations\    (subdir, has child)
        //   Data\ObjectContainers\loose.socpak (file)
        let mut backing: std::collections::HashMap<String, Vec<DirChild>> =
            std::collections::HashMap::new();
        backing.insert(
            "Data\\ObjectContainers".into(),
            vec![
                make_dir("PU"),
                make_dir("Stations"),
                make_file("Data\\ObjectContainers\\loose.socpak", 4096),
            ],
        );
        backing.insert(
            "Data\\ObjectContainers\\PU".into(),
            vec![make_file("Data\\ObjectContainers\\PU\\zone.socpak", 8192)],
        );
        backing.insert(
            "Data\\ObjectContainers\\Stations".into(),
            vec![make_dir("alpha")],
        );

        let result = list_socpak_dir_impl("Data/ObjectContainers/", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });

        // Directories first, alphabetically, then socpak files.
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].display_name, "PU");
        assert_eq!(result[0].kind, SocpakDirEntryKind::Directory);
        assert_eq!(result[1].display_name, "Stations");
        assert_eq!(result[1].kind, SocpakDirEntryKind::Directory);
        assert_eq!(result[2].display_name, "loose.socpak");
        assert_eq!(result[2].kind, SocpakDirEntryKind::SocpakFile);
        assert_eq!(result[2].size_or_count, 4096);
    }

    #[test]
    fn list_socpak_dir_normalises_prefix_with_or_without_trailing_separator() {
        let mut backing: std::collections::HashMap<String, Vec<DirChild>> =
            std::collections::HashMap::new();
        backing.insert(
            "Data\\ObjectContainers".into(),
            vec![make_file("Data\\ObjectContainers\\foo.socpak", 1)],
        );

        // Forward slashes, trailing separator.
        let a = list_socpak_dir_impl("Data/ObjectContainers/", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });
        // Backslashes, no trailing separator.
        let b = list_socpak_dir_impl("Data\\ObjectContainers", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });
        // Backslashes, trailing separator.
        let c = list_socpak_dir_impl("Data\\ObjectContainers\\", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });

        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].display_name, "foo.socpak");
    }

    #[test]
    fn list_socpak_dir_skips_non_socpak_files() {
        let mut backing: std::collections::HashMap<String, Vec<DirChild>> =
            std::collections::HashMap::new();
        backing.insert(
            "Data\\ObjectContainers".into(),
            vec![
                make_file("Data\\ObjectContainers\\readme.txt", 100),
                make_file("Data\\ObjectContainers\\index.xml", 200),
                make_file("Data\\ObjectContainers\\real.socpak", 4096),
            ],
        );

        let result = list_socpak_dir_impl("Data/ObjectContainers/", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].display_name, "real.socpak");
    }

    #[test]
    fn list_socpak_dir_directory_entries_carry_child_count() {
        let mut backing: std::collections::HashMap<String, Vec<DirChild>> =
            std::collections::HashMap::new();
        backing.insert("root".into(), vec![make_dir("PU")]);
        // PU directly contains 2 subdirs and 1 socpak that count, plus
        // 1 unrelated file that should NOT count.
        backing.insert(
            "root\\PU".into(),
            vec![
                make_dir("loc"),
                make_dir("zones"),
                make_file("root\\PU\\hangar.socpak", 1),
                make_file("root\\PU\\manifest.xml", 1),
            ],
        );

        let result = list_socpak_dir_impl("root", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, SocpakDirEntryKind::Directory);
        assert_eq!(
            result[0].size_or_count, 3,
            "PU has 2 subdirs + 1 socpak that count toward the badge"
        );
    }

    #[test]
    fn list_socpak_dir_socpak_filter_is_case_insensitive() {
        let mut backing: std::collections::HashMap<String, Vec<DirChild>> =
            std::collections::HashMap::new();
        backing.insert(
            "Data\\OC".into(),
            vec![
                make_file("Data\\OC\\Foo.SOCPAK", 1),
                make_file("Data\\OC\\BAR.socpak", 2),
                make_file("Data\\OC\\baz.SocPak", 3),
            ],
        );

        let result = list_socpak_dir_impl("Data\\OC", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });

        assert_eq!(result.len(), 3);
        // Sorted case-insensitively by display_name.
        assert_eq!(result[0].display_name, "BAR.socpak");
        assert_eq!(result[1].display_name, "baz.SocPak");
        assert_eq!(result[2].display_name, "Foo.SOCPAK");
    }

    #[test]
    fn list_socpak_dir_directory_path_terminates_with_separator() {
        // Caller round-trips `path` back into another `list_socpak_dir`
        // call to expand a branch -- so directory paths MUST end with
        // a trailing separator the way the function expects on input.
        let mut backing: std::collections::HashMap<String, Vec<DirChild>> =
            std::collections::HashMap::new();
        backing.insert(
            "Data\\ObjectContainers".into(),
            vec![make_dir("PU")],
        );
        backing.insert("Data\\ObjectContainers\\PU".into(), vec![]);

        let result = list_socpak_dir_impl("Data/ObjectContainers/", |p| {
            backing.get(p).cloned().unwrap_or_default()
        });

        assert_eq!(result.len(), 1);
        assert!(
            result[0].path.ends_with('\\'),
            "directory path must end with a separator: {}",
            result[0].path
        );
        assert_eq!(result[0].path, "Data\\ObjectContainers\\PU\\");
    }

    // ── Global path index (`list_all_socpaks_impl`) ────────────────────────
    //
    // The production entry point (`list_all_socpaks`) needs a real
    // `MappedP4k`, so these tests target the iterator-driven inner
    // function and pass synthetic entry names directly.

    #[test]
    fn list_all_socpaks_filters_non_socpak_entries() {
        let names = [
            "Data\\ObjectContainers\\PU\\hangar.socpak",
            "Data\\ObjectContainers\\PU\\readme.txt",
            "Data\\ObjectContainers\\PU\\manifest.xml",
            "Data\\ObjectContainers\\PU\\dungeon.socpak",
        ];
        let out = list_all_socpaks_impl(&["Data/ObjectContainers/"], names.iter().copied());
        assert_eq!(
            out,
            vec![
                "Data\\ObjectContainers\\PU\\dungeon.socpak".to_string(),
                "Data\\ObjectContainers\\PU\\hangar.socpak".to_string(),
            ]
        );
    }

    #[test]
    fn list_all_socpaks_returns_sorted_alphabetically_case_insensitive() {
        let names = [
            "Data\\ObjectContainers\\PU\\Zulu.socpak",
            "Data\\ObjectContainers\\PU\\alpha.socpak",
            "Data\\ObjectContainers\\PU\\Beta.SOCPAK",
            "Data\\ObjectContainers\\PU\\charlie.socpak",
        ];
        let out = list_all_socpaks_impl(&["Data/ObjectContainers/"], names.iter().copied());
        assert_eq!(
            out,
            vec![
                "Data\\ObjectContainers\\PU\\alpha.socpak".to_string(),
                "Data\\ObjectContainers\\PU\\Beta.SOCPAK".to_string(),
                "Data\\ObjectContainers\\PU\\charlie.socpak".to_string(),
                "Data\\ObjectContainers\\PU\\Zulu.socpak".to_string(),
            ]
        );
    }

    #[test]
    fn list_all_socpaks_respects_search_roots() {
        let names = [
            "Data\\ObjectContainers\\PU\\inside.socpak",
            "Data\\Other\\elsewhere.socpak",
            "Data\\ObjectContainers\\Stations\\alpha.socpak",
        ];
        // Only entries under `Data\\ObjectContainers\\` should pass.
        let out = list_all_socpaks_impl(&["Data/ObjectContainers/"], names.iter().copied());
        assert_eq!(
            out,
            vec![
                "Data\\ObjectContainers\\PU\\inside.socpak".to_string(),
                "Data\\ObjectContainers\\Stations\\alpha.socpak".to_string(),
            ]
        );
    }

    #[test]
    fn list_all_socpaks_empty_roots_means_scan_everything() {
        let names = [
            "Data\\ObjectContainers\\inside.socpak",
            "Data\\Other\\elsewhere.socpak",
            "totally\\unrelated.socpak",
        ];
        let out = list_all_socpaks_impl(&[], names.iter().copied());
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn list_all_socpaks_normalises_search_root_separators() {
        let names = ["Data\\ObjectContainers\\PU\\inside.socpak"];
        // All three forms of the search root should match.
        let a = list_all_socpaks_impl(&["Data/ObjectContainers/"], names.iter().copied());
        let b = list_all_socpaks_impl(&["Data\\ObjectContainers"], names.iter().copied());
        let c = list_all_socpaks_impl(&["Data\\ObjectContainers\\"], names.iter().copied());
        assert_eq!(a.len(), 1);
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    /// Synthetic graph test: a single root with two children, neither
    /// of which is referenced by anyone else, yields exactly the root.
    #[test]
    fn root_detection_on_synthetic_graph() {
        // Set of all paths.
        let all: Vec<&str> = vec!["root.socpak", "child_a.socpak", "child_b.socpak"];
        // Graph: root references both children; children reference
        // nothing.
        let mut g: HashMap<String, Vec<String>> = HashMap::new();
        g.insert("root.socpak".into(), vec!["child_a.socpak".into(), "child_b.socpak".into()]);
        g.insert("child_a.socpak".into(), vec![]);
        g.insert("child_b.socpak".into(), vec![]);

        let referenced: HashSet<String> = g
            .values()
            .flat_map(|v| v.iter().cloned())
            .collect();

        let roots: Vec<&str> = all
            .iter()
            .copied()
            .filter(|p| !referenced.contains(*p))
            .collect();

        assert_eq!(roots, vec!["root.socpak"]);
    }
}
