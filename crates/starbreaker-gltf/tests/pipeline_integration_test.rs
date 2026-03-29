use std::path::Path;

use starbreaker_datacore::database::Database;
use starbreaker_p4k::MappedP4k;

const DCB_PATH: &str = "../../Game2.dcb";
const P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\PTU\Data.p4k";

#[test]
#[ignore]
fn assemble_glb_produces_valid_glb() {
    let dcb_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DCB_PATH);
    if !dcb_path.exists() {
        eprintln!("Game2.dcb not found at {}, skipping", dcb_path.display());
        return;
    }
    let p4k_path = Path::new(P4K_PATH);
    if !p4k_path.exists() {
        eprintln!("Data.p4k not found at {}, skipping", p4k_path.display());
        return;
    }

    let dcb_data = std::fs::read(&dcb_path).unwrap();
    let db = Database::from_bytes(&dcb_data).unwrap();
    let p4k = MappedP4k::open(p4k_path).unwrap();

    // Find an EntityClassDefinition with geometry
    let opts = starbreaker_gltf::ExportOptions::default();
    let mut found = None;
    for record in db.records() {
        let struct_name = db.struct_name(record.struct_id());
        if struct_name == "EntityClassDefinition" {
            let tree = starbreaker_datacore::loadout::resolve_loadout(&db, record);
            match starbreaker_gltf::assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts) {
                Ok(result) => {
                    found = Some((db.resolve_string2(record.name_offset).to_string(), result));
                    break;
                }
                Err(_) => continue,
            }
        }
    }

    let (entity_name, result) = found.expect("should find at least one entity with geometry");

    println!("Entity: {entity_name}");
    println!("Geometry: {}", result.geometry_path);
    println!("Material: {}", result.material_path);
    println!("GLB size: {} bytes", result.glb.len());

    // Verify GLB magic bytes
    assert!(result.glb.len() > 12, "GLB too small");
    assert_eq!(&result.glb[0..4], b"glTF", "missing GLB magic bytes");
}

/// Ensure the depth limit in the Value materializer doesn't silently drop
/// loadout entries. The Gladius is the reference ship: it must export guns
/// (entityClassReference entries at depth ~7-8) and missiles (nested loadout
/// entries at depth ~4-5).
#[test]
#[ignore]
fn depth_limit_preserves_gladius_loadout() {
    let dcb_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DCB_PATH);
    if !dcb_path.exists() {
        return;
    }
    let p4k_path = Path::new(P4K_PATH);
    if !p4k_path.exists() {
        return;
    }

    let dcb_data = std::fs::read(&dcb_path).unwrap();
    let db = Database::from_bytes(&dcb_data).unwrap();
    let p4k = MappedP4k::open(p4k_path).unwrap();

    let idx = starbreaker_datacore::loadout::EntityIndex::new(&db);
    let record = idx
        .find_record("AEGS_Gladius")
        .expect("AEGS_Gladius not found");
    let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);

    // Flatten all loadout entries
    fn collect_names(nodes: &[starbreaker_datacore::loadout::LoadoutNode], out: &mut Vec<String>) {
        for n in nodes {
            out.push(format!("{} -> {}", n.item_port_name, n.entity_name));
            collect_names(&n.children, out);
        }
    }
    let mut names = Vec::new();
    collect_names(&tree.root.children, &mut names);

    // Guns must be present (entityClassReference at depth ~7-8)
    let has_gun_mount = names.iter().any(|n| n.contains("hardpoint_gun_nose"));
    assert!(
        has_gun_mount,
        "missing gun mount — depth limit too low?\nentries: {names:#?}"
    );

    // Missiles must be present (nested loadout entries)
    let has_missiles = names.iter().any(|n| n.contains("missile_01_attach"));
    assert!(
        has_missiles,
        "missing missiles — depth limit too low?\nentries: {names:#?}"
    );

    // Export must succeed with geometry
    let opts = starbreaker_gltf::ExportOptions {
        include_textures: false,
        texture_mip: 0,
        lod_level: 0,
        include_interior: false,
        include_normals: true,
        include_lights: false,
        include_tangents: true,
        include_materials: true,
        experimental_textures: false,
    };
    let result = starbreaker_gltf::assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts)
        .expect("Gladius export failed");

    // Must have at least 45 child meshes (hull + thrusters + guns + missiles + components)
    // Parse JSON to count nodes
    let json_len = u32::from_le_bytes(result.glb[12..16].try_into().unwrap()) as usize;
    let json_str = std::str::from_utf8(&result.glb[20..20 + json_len]).unwrap();
    let root: serde_json::Value = serde_json::from_str(json_str.trim()).unwrap();
    let mesh_count = root["meshes"].as_array().map(|a| a.len()).unwrap_or(0);

    // 48 child meshes + root NMC meshes ≈ lots of meshes
    assert!(
        mesh_count >= 50,
        "too few meshes ({mesh_count}) — loadout entries likely truncated by depth limit"
    );
}

/// Stress test: the Idris P Collector Military is the heaviest ship (295 geometry
/// entries). Ensures the depth limit and Value materializer handle the most
/// complex loadout without crashing, OOMing, or silently dropping entries.
#[test]
#[ignore]
fn depth_limit_preserves_idris_p_collector_military() {
    let dcb_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DCB_PATH);
    if !dcb_path.exists() {
        return;
    }
    let p4k_path = Path::new(P4K_PATH);
    if !p4k_path.exists() {
        return;
    }

    let dcb_data = std::fs::read(&dcb_path).unwrap();
    let db = Database::from_bytes(&dcb_data).unwrap();
    let p4k = MappedP4k::open(p4k_path).unwrap();

    let idx = starbreaker_datacore::loadout::EntityIndex::new(&db);
    let record = idx
        .find_record("AEGS_Idris_P_Collector_Military")
        .expect("AEGS_Idris_P_Collector_Military not found");
    let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);

    fn count_geom(nodes: &[starbreaker_datacore::loadout::LoadoutNode]) -> usize {
        let mut g = 0;
        for n in nodes {
            if n.geometry_path.is_some() {
                g += 1;
            }
            g += count_geom(&n.children);
        }
        g
    }
    let geom_count = count_geom(&tree.root.children);

    // Must resolve at least 250 geometry entries (full count is ~295)
    assert!(
        geom_count >= 250,
        "Idris P CM: only {geom_count} geometry entries (expected 250+) — depth limit too low?"
    );

    // Export must succeed
    let opts = starbreaker_gltf::ExportOptions {
        include_textures: false,
        texture_mip: 0,
        lod_level: 2,
        include_interior: false,
        include_normals: true,
        include_lights: false,
        include_tangents: true,
        include_materials: true,
        experimental_textures: false,
    };
    let result = starbreaker_gltf::assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts)
        .expect("Idris P Collector Military export failed");

    assert!(
        result.glb.len() > 10_000_000,
        "GLB too small ({} bytes) — geometry likely missing",
        result.glb.len()
    );
}
