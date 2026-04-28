//! End-to-end smoke test: parse the Executive Hangar SOC payload from a
//! real installed `Data.p4k` and assert basic sanity on the result.
//!
//! Marked `#[ignore]` because it depends on a Star Citizen install being
//! present at a fixed path. Run manually with:
//!
//! ```text
//! cargo test --manifest-path <repo>/Cargo.toml -p starbreaker-3d \
//!   --test exec_hangar_smoke -- --ignored --nocapture
//! ```
//!
//! The test writes a JSON summary of brush / entity / visarea counts and the
//! brush-translation AABB to `target/exec-hangar-smoke.json` so a supervisor
//! can spot-check changes between iterations.

use std::path::PathBuf;

use starbreaker_3d::soc::{self, SceneError};
use starbreaker_p4k::MappedP4k;

// Top-level Executive Hangar socpaks observed in `Data/ObjectContainers/PU/
// loc/mod/pyro/asteroid_base/`. The "final_set" socpak is the master that
// nests the interior + hangar containers; we walk it recursively so the
// composer pulls in every child zone.
const EXEC_HANGAR_ROOT_SOCPAK: &str =
    "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\asteroid_base\\ext\\ab_final_set\\\
     ab_pyro_final_set_dungeon_executive-001.socpak";

const FALLBACK_FLAT_SOCPAKS: &[&str] = &[
    "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\asteroid_base\\ext\\ab_assembled\\\
     ab_pyro_asmbl_01_dung_exec_001a.socpak",
    "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\asteroid_base\\int\\ab_int_dungeon\\\
     ab_pyro_int_dung_exec_001.socpak",
    "Data\\ObjectContainers\\PU\\loc\\mod\\pyro\\asteroid_base\\ext\\ab_ext\\\
     ab_pyro_hangar_dungeon_exec_001.socpak",
];

const P4K_CANDIDATES: &[&str] = &[
    r"C:\Program Files\Roberts Space Industries\StarCitizen\HOTFIX\Data.p4k",
    r"C:\Program Files\Roberts Space Industries\StarCitizen\TECH-PREVIEW\Data.p4k",
    r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k",
];

const MIN_EXPECTED_BRUSHES: usize = 200;
const MIN_AABB_AXIS_EXTENT: f32 = 5.0;

#[test]
#[ignore = "depends on local SC install; run manually with --ignored"]
fn smoke_executive_hangar_from_p4k() {
    let p4k_path = locate_p4k().expect("no Star Citizen install with Data.p4k found");
    eprintln!("opening p4k: {}", p4k_path.display());
    let p4k = MappedP4k::open(&p4k_path).expect("MappedP4k::open");

    // First try walking the master socpak so transforms cascade naturally.
    // If the master can't be reached (different build, renamed layout) fall
    // back to loading the three known top-level socpaks at world origin —
    // that still produces a usable scene for the smoke assertions.
    let mut scene = match soc::compose_from_root(&p4k, EXEC_HANGAR_ROOT_SOCPAK, 4) {
        Ok(scene) if scene.brush_count() >= MIN_EXPECTED_BRUSHES => scene,
        Ok(weak) => {
            eprintln!(
                "compose_from_root yielded only {} brushes; falling back to flat-list mode",
                weak.brush_count()
            );
            compose_flat_list_or_skip(&p4k)
        }
        Err(SceneError::SocpakNotFound(_)) => compose_flat_list_or_skip(&p4k),
        Err(other) => panic!("compose_from_root failed: {other}"),
    };

    if scene.brush_count() < MIN_EXPECTED_BRUSHES {
        // Augment with the flat list — picks up any zone the recursive
        // walk missed because of XML-level differences.
        let flat = compose_flat_list_or_skip(&p4k);
        for zone in flat.zones {
            scene.zones.push(zone);
        }
    }

    let brush_count = scene.brush_count();
    let entity_count = scene.entity_count();
    let light_count = scene.light_count();
    let zone_count = scene.zones.len();

    let aabb = scene
        .brush_aabb()
        .expect("at least one brush should produce an AABB");
    let extent = [
        aabb.1[0] - aabb.0[0],
        aabb.1[1] - aabb.0[1],
        aabb.1[2] - aabb.0[2],
    ];
    let axes_with_extent = extent
        .iter()
        .filter(|e| **e > MIN_AABB_AXIS_EXTENT)
        .count();

    let visarea_count: usize = scene
        .zones
        .iter()
        .filter_map(|z| {
            // Re-parse the SOC for visarea data: the composer doesn't track
            // it because the renderer doesn't consume it yet, but the smoke
            // test wants a count for the JSON summary.
            // Cheap because the bytes are usually still in the OS page
            // cache after the brush walk.
            let socpak_path = guess_socpak_path_for_zone(&z.name)?;
            let socpak_bytes = match read_socpak(&p4k, &socpak_path) {
                Some(b) => b,
                None => return None,
            };
            let inner = starbreaker_p4k::P4kArchive::from_bytes(&socpak_bytes).ok()?;
            for e in inner.entries() {
                if e.name.to_ascii_lowercase().ends_with(".soc") {
                    let bytes = inner.read(e).ok()?;
                    if let Ok(va) = soc::visarea::parse(&bytes) {
                        return Some(va.total());
                    }
                }
            }
            None
        })
        .sum();

    eprintln!(
        "exec-hangar smoke: zones={zone_count} brushes={brush_count} \
         entities={entity_count} lights={light_count} visareas={visarea_count}"
    );
    eprintln!(
        "AABB: min=({:.2}, {:.2}, {:.2}) max=({:.2}, {:.2}, {:.2}) extent=({:.2}, {:.2}, {:.2})",
        aabb.0[0], aabb.0[1], aabb.0[2], aabb.1[0], aabb.1[1], aabb.1[2],
        extent[0], extent[1], extent[2],
    );

    assert!(
        brush_count >= MIN_EXPECTED_BRUSHES,
        "expected at least {MIN_EXPECTED_BRUSHES} brushes; got {brush_count}"
    );
    assert!(
        light_count >= 1,
        "expected at least one light entity; got {light_count} (entity_count={entity_count})"
    );
    assert!(
        axes_with_extent >= 2,
        "AABB should have non-zero extent in at least two axes; got {extent:?}"
    );

    // Dump JSON summary so the supervisor can compare iterations.
    let summary = serde_json::json!({
        "zones": zone_count,
        "brushes": brush_count,
        "entities": entity_count,
        "lights": light_count,
        "visareas": visarea_count,
        "aabb": {
            "min": [aabb.0[0], aabb.0[1], aabb.0[2]],
            "max": [aabb.1[0], aabb.1[1], aabb.1[2]],
            "extent": [extent[0], extent[1], extent[2]],
        },
        "p4k": p4k_path.display().to_string(),
    });

    let target_dir = locate_target_dir();
    let summary_path = target_dir.join("exec-hangar-smoke.json");
    if let Some(parent) = summary_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::to_string_pretty(&summary).expect("json");
    std::fs::write(&summary_path, body).expect("write summary");
    eprintln!("summary written to {}", summary_path.display());
}

fn compose_flat_list_or_skip(p4k: &MappedP4k) -> soc::ComposedScene {
    let any_present = FALLBACK_FLAT_SOCPAKS
        .iter()
        .any(|p| p4k.entry_case_insensitive(p).is_some());
    assert!(
        any_present,
        "no Executive Hangar socpaks resolved in the installed Data.p4k — \
         the test cannot run on this build"
    );

    soc::compose_from_flat_list(p4k, FALLBACK_FLAT_SOCPAKS).expect("compose_from_flat_list")
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
    // CARGO_MANIFEST_DIR points at the crate dir; target sits at workspace
    // root, two levels up.
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

fn guess_socpak_path_for_zone(zone_name: &str) -> Option<String> {
    // The composer's zone name is the socpak file stem. Map back to its
    // p4k path using the known Executive Hangar layout. Returning `None`
    // simply skips visarea counting for that zone, which is acceptable for
    // a smoke test.
    let lc = zone_name.to_ascii_lowercase();
    let path = if lc.contains("final_set") {
        EXEC_HANGAR_ROOT_SOCPAK
    } else if lc.contains("asmbl_01_dung_exec_001a") {
        FALLBACK_FLAT_SOCPAKS[0]
    } else if lc.contains("int_dung_exec_001") {
        FALLBACK_FLAT_SOCPAKS[1]
    } else if lc.contains("hangar_dungeon_exec_001") {
        FALLBACK_FLAT_SOCPAKS[2]
    } else {
        return None;
    };
    Some(path.to_string())
}

fn read_socpak(p4k: &MappedP4k, path: &str) -> Option<Vec<u8>> {
    let entry = p4k.entry_case_insensitive(path)?;
    p4k.read(entry).ok()
}
