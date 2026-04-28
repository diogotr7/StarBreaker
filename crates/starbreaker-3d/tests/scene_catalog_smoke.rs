//! End-to-end smoke test for the scene catalog enumerator.
//!
//! Loads the installed HOTFIX (or fallback channel) `Data.p4k`, runs
//! `enumerate_scene_roots` against `Data/ObjectContainers/`, and asserts
//! sanity on the result. Dumps the full catalog JSON to
//! `target/scene_catalog_smoke.json` so the supervisor can inspect what
//! came out.
//!
//! Marked `#[ignore]` because it depends on a Star Citizen install at a
//! fixed path. Run manually with:
//!
//! ```text
//! cargo test --manifest-path <repo>/Cargo.toml -p starbreaker-3d \
//!   --test scene_catalog_smoke -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use starbreaker_3d::soc::{self, SceneCatalogEntry};
use starbreaker_p4k::MappedP4k;

const P4K_CANDIDATES: &[&str] = &[
    r"C:\Program Files\Roberts Space Industries\StarCitizen\HOTFIX\Data.p4k",
    r"C:\Program Files\Roberts Space Industries\StarCitizen\TECH-PREVIEW\Data.p4k",
    r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k",
];

const SEARCH_ROOTS: &[&str] = &["Data/ObjectContainers/"];

/// Sanity floor: a real p4k always carries at least dozens of scene
/// roots (asteroid bases, hangars, modular outposts, ship interiors).
const MIN_EXPECTED_COUNT: usize = 10;

/// Sanity ceiling: if the enumerator returns more than this, the
/// graph is broken (every socpak became a "root") and the filter
/// needs revisiting before we ship the list to the user.
const MAX_EXPECTED_COUNT: usize = 5000;

/// Path tails the catalog must surface so the smoke test stays
/// anchored to the iteration-A reference scene. The Executive Hangar
/// socpak (`ab_pyro_final_set_dungeon_executive-001.socpak`) turns
/// out NOT to be a graph root in the live HOTFIX p4k -- a parent
/// `ab_final_set` aggregator references it, putting it at in-degree
/// 1. The user picks the parent, the composer recurses into the
/// executive variant.
///
/// We assert that at least one of these well-known asteroid-base
/// hangar / dungeon roots survives the filter. Different builds
/// rename the parent aggregator; a substring match is more durable
/// than pinning a specific filename.
const REQUIRED_PATH_SUBSTRINGS: &[&str] = &[
    "ab_pyro_final_set_dungeon",
    "ab_collector_hangar",
    "asteroid_base",
];

#[test]
#[ignore = "depends on local SC install; run manually with --ignored"]
fn smoke_scene_catalog_from_p4k() {
    let p4k_path = locate_p4k().expect("no Star Citizen install with Data.p4k found");
    eprintln!("opening p4k: {}", p4k_path.display());
    let p4k = MappedP4k::open(&p4k_path).expect("MappedP4k::open");

    let entries =
        soc::enumerate_scene_roots(&p4k, SEARCH_ROOTS).expect("enumerate_scene_roots");

    let total = entries.len();
    eprintln!("scene catalog: {total} root socpaks");
    if total > 0 {
        eprintln!("first: {} ({})", entries[0].display_name, entries[0].path);
        eprintln!(
            "last:  {} ({})",
            entries[total - 1].display_name,
            entries[total - 1].path
        );
    }

    // Dump JSON dump so the supervisor can scan the list. Done before
    // assertions so even an assertion failure leaves a debuggable
    // artefact behind.
    let target_dir = locate_target_dir();
    let summary_path = target_dir.join("scene_catalog_smoke.json");
    if let Some(parent) = summary_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::to_string_pretty(&entries_to_json(&entries)).expect("json");
    std::fs::write(&summary_path, body).expect("write summary");
    eprintln!("summary written to {}", summary_path.display());

    assert!(
        total >= MIN_EXPECTED_COUNT,
        "expected at least {MIN_EXPECTED_COUNT} scene roots; got {total}"
    );
    assert!(
        total <= MAX_EXPECTED_COUNT,
        "expected at most {MAX_EXPECTED_COUNT} scene roots; got {total} -- the graph or filter is probably broken"
    );

    for needle in REQUIRED_PATH_SUBSTRINGS {
        let present = entries
            .iter()
            .any(|e| e.path.to_ascii_lowercase().contains(needle));
        assert!(
            present,
            "expected at least one root whose path contains {needle:?}"
        );
    }

    for e in &entries {
        assert!(
            !e.display_name.trim().is_empty(),
            "every entry should have a non-empty display name; got {:?}",
            e
        );
    }
}

fn entries_to_json(entries: &[SceneCatalogEntry]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "path": e.path,
                "display_name": e.display_name,
                "sub_zone_count": e.sub_zone_count,
                "source_kind": e.source_kind.as_snake_case(),
            })
        })
        .collect();
    serde_json::json!({
        "count": arr.len(),
        "entries": arr,
    })
}

fn locate_p4k() -> Option<PathBuf> {
    for cand in P4K_CANDIDATES {
        let p = PathBuf::from(cand);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn locate_target_dir() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let workspace_root = manifest
        .ancestors()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.clone());
    workspace_root.join("target")
}
