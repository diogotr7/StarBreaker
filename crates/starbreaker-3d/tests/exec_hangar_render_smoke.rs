//! End-to-end render smoke test: walk Executive Hangar's socpak graph,
//! resolve every brush mesh against the installed `Data.p4k`, emit a
//! `.glb`, and re-open it through a glTF parser to assert the contract.
//!
//! Marked `#[ignore]` because it depends on a Star Citizen install at a
//! fixed path. Run manually with:
//!
//! ```text
//! cargo test --manifest-path <repo>/Cargo.toml -p starbreaker-3d \
//!   --test exec_hangar_render_smoke -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use starbreaker_3d::soc::{self, SceneError};
use starbreaker_p4k::MappedP4k;

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
const MIN_UNIQUE_MESHES: u32 = 100;
const MIN_NODES: u32 = 1_000;
const MIN_AABB_AXIS_EXTENT: f32 = 5.0;

#[test]
#[ignore = "depends on local SC install; run manually with --ignored"]
fn smoke_executive_hangar_render() {
    let p4k_path = locate_p4k().expect("no Star Citizen install with Data.p4k found");
    eprintln!("opening p4k: {}", p4k_path.display());
    let p4k = MappedP4k::open(&p4k_path).expect("MappedP4k::open");

    let mut composed = match soc::compose_from_root(&p4k, EXEC_HANGAR_ROOT_SOCPAK, 4) {
        Ok(scene) if scene.brush_count() >= MIN_EXPECTED_BRUSHES => scene,
        Ok(_) => compose_flat_list_or_skip(&p4k),
        Err(SceneError::SocpakNotFound(_)) => compose_flat_list_or_skip(&p4k),
        Err(other) => panic!("compose_from_root failed: {other}"),
    };
    if composed.brush_count() < MIN_EXPECTED_BRUSHES {
        let flat = compose_flat_list_or_skip(&p4k);
        for zone in flat.zones {
            composed.zones.push(zone);
        }
    }
    eprintln!(
        "composed: zones={} brushes={} entities={} lights={}",
        composed.zones.len(),
        composed.brush_count(),
        composed.entity_count(),
        composed.light_count(),
    );

    eprintln!("resolving meshes...");
    let renderable = soc::resolve_scene(&p4k, &composed);
    eprintln!(
        "resolve summary: meshes={} placements={} lights={} \
         dropped_placements={} failed_mesh_paths={}",
        renderable.meshes.len(),
        renderable.placements.len(),
        renderable.lights.len(),
        renderable.dropped_placements,
        renderable.failed_mesh_paths,
    );

    eprintln!("emitting GLB...");
    let (glb_bytes, summary) = soc::emit_glb(&renderable).expect("emit ok");
    eprintln!(
        "emit summary: mesh_count={} placement_count={} light_count={} \
         lights_dropped={} material_count={} texture_count={} glb_bytes={}",
        summary.mesh_count,
        summary.placement_count,
        summary.light_count,
        summary.lights_dropped,
        summary.material_count,
        summary.texture_count,
        glb_bytes.len(),
    );

    // Write GLB next to the JSON summary so a human can spot-check it
    // through any glTF viewer.
    let target_dir = locate_target_dir();
    std::fs::create_dir_all(&target_dir).ok();
    let glb_path = target_dir.join("exec-hangar-render-smoke.glb");
    std::fs::write(&glb_path, &glb_bytes).expect("write glb");
    eprintln!("glb written to {} ({} bytes)", glb_path.display(), glb_bytes.len());

    // Inspect the emitted glTF JSON header by parsing the bytes back.
    // We do not depend on the `gltf` crate here — we just decode the
    // 12-byte header + JSON chunk, then deserialise the JSON with
    // `serde_json` to check counts.
    let parsed = parse_glb_json(&glb_bytes).expect("re-open glb");
    let mesh_array = parsed
        .get("meshes")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    let node_array = parsed
        .get("nodes")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    let mat_array = parsed
        .get("materials")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    let tex_array = parsed
        .get("textures")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    let extensions_used = parsed
        .get("extensionsUsed")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let lights_in_extension = parsed
        .get("extensions")
        .and_then(|v| v.get("KHR_lights_punctual"))
        .and_then(|v| v.get("lights"))
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);

    let aabb = renderable
        .aabb
        .expect("at least one placement should produce an AABB");
    let extent = [
        aabb.1[0] - aabb.0[0],
        aabb.1[1] - aabb.0[1],
        aabb.1[2] - aabb.0[2],
    ];
    let axes_with_extent = extent
        .iter()
        .filter(|e| **e > MIN_AABB_AXIS_EXTENT)
        .count();

    // Verify at least one material carries the `diffuse_texture_path`
    // extra. The emitter does NOT embed a placeholder PNG / bind a
    // `baseColorTexture` (that produced thousands of shared blob URLs
    // and triggered "Couldn't load texture blob:" errors at render
    // time); instead it leaves `material.map = null` in the GLB and
    // surfaces the texture path through `extras` so the JS-side
    // resolver can substitute the real DDS bytes.
    let materials_with_diffuse_path = parsed
        .get("materials")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|m| {
                    m.get("extras")
                        .and_then(|e| e.get("diffuse_texture_path"))
                        .is_some()
                })
                .count() as u32
        })
        .unwrap_or(0);

    eprintln!(
        "json summary: meshes={mesh_array} nodes={node_array} \
         materials={mat_array} textures={tex_array} lights_ext={lights_in_extension} \
         materials_with_diffuse_path={materials_with_diffuse_path} \
         extensionsUsed={extensions_used:?}"
    );
    eprintln!(
        "AABB: min=({:.2}, {:.2}, {:.2}) max=({:.2}, {:.2}, {:.2}) extent=({:.2}, {:.2}, {:.2})",
        aabb.0[0], aabb.0[1], aabb.0[2], aabb.1[0], aabb.1[1], aabb.1[2],
        extent[0], extent[1], extent[2],
    );

    assert!(
        mesh_array >= MIN_UNIQUE_MESHES,
        "expected at least {MIN_UNIQUE_MESHES} unique meshes; got {mesh_array}"
    );
    assert!(
        node_array >= MIN_NODES,
        "expected at least {MIN_NODES} nodes; got {node_array}"
    );
    // The emitted GLB caps lights to a renderer-friendly budget. We
    // still expect *some* lights to make it through for the hangar.
    assert!(
        lights_in_extension >= 1,
        "expected at least 1 light in the emitted GLB; got {lights_in_extension}"
    );
    assert!(
        lights_in_extension <= starbreaker_3d::soc::DEFAULT_MAX_EMITTED_LIGHTS as u32,
        "emitted GLB exceeded the default light cap: \
         got {lights_in_extension}, cap is {}",
        starbreaker_3d::soc::DEFAULT_MAX_EMITTED_LIGHTS,
    );
    // Source SOC scene contains far more lights than the cap; verify
    // the cap is doing real work rather than running on a tiny scene.
    if (renderable.lights.len() as u32) > starbreaker_3d::soc::DEFAULT_MAX_EMITTED_LIGHTS as u32 {
        assert!(
            summary.lights_dropped > 0,
            "scene has {} input lights but lights_dropped is 0",
            renderable.lights.len(),
        );
    }
    assert!(
        axes_with_extent >= 2,
        "AABB should have non-zero extent in at least two axes; got {extent:?}"
    );
    assert!(
        materials_with_diffuse_path >= 1,
        "expected at least one material carrying a diffuse_texture_path extra; \
         got {materials_with_diffuse_path}"
    );

    let summary_json = serde_json::json!({
        "p4k": p4k_path.display().to_string(),
        "input": {
            "zones": composed.zones.len(),
            "brushes": composed.brush_count(),
            "entities": composed.entity_count(),
            "lights": composed.light_count(),
        },
        "resolve": {
            "unique_meshes": renderable.meshes.len(),
            "placements": renderable.placements.len(),
            "lights": renderable.lights.len(),
            "dropped_placements": renderable.dropped_placements,
            "failed_mesh_paths": renderable.failed_mesh_paths,
        },
        "emit": {
            "mesh_count": summary.mesh_count,
            "placement_count": summary.placement_count,
            "light_count": summary.light_count,
            "lights_dropped": summary.lights_dropped,
            "material_count": summary.material_count,
            "texture_count": summary.texture_count,
            "glb_bytes": glb_bytes.len(),
        },
        "gltf_json": {
            "meshes": mesh_array,
            "nodes": node_array,
            "materials": mat_array,
            "textures": tex_array,
            "lights_in_extension": lights_in_extension,
            "materials_with_diffuse_path": materials_with_diffuse_path,
            "extensions_used": extensions_used,
        },
        "aabb": {
            "min": [aabb.0[0], aabb.0[1], aabb.0[2]],
            "max": [aabb.1[0], aabb.1[1], aabb.1[2]],
            "extent": [extent[0], extent[1], extent[2]],
        },
        "glb_path": glb_path.display().to_string(),
    });
    let summary_path = target_dir.join("exec-hangar-render-smoke.json");
    std::fs::write(
        &summary_path,
        serde_json::to_string_pretty(&summary_json).expect("json"),
    )
    .expect("write summary");
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

/// Decode the JSON chunk out of a glTF binary container. Skips
/// validation — we trust our own emitter and only need to inspect a
/// handful of array lengths for the smoke assertions.
fn parse_glb_json(bytes: &[u8]) -> Option<serde_json::Value> {
    if bytes.len() < 12 + 8 || &bytes[0..4] != b"glTF" {
        return None;
    }
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    let json_type = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    if json_type != 0x4E4F534A {
        return None;
    }
    let json_start: usize = 20;
    let json_end = json_start.checked_add(json_len)?;
    if json_end > bytes.len() {
        return None;
    }
    serde_json::from_slice(&bytes[json_start..json_end]).ok()
}
