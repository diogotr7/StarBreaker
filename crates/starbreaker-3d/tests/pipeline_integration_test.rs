use std::path::{Path, PathBuf};

use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

const DEFAULT_DCB_PATH: &str = "../../Game2.dcb";
const DEFAULT_P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\PTU\Data.p4k";

fn integration_dcb_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("STARBREAKER_TEST_DCB") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
        eprintln!("STARBREAKER_TEST_DCB not found at {}, skipping", path.display());
        return None;
    }

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_DCB_PATH);
    if path.exists() {
        Some(path)
    } else {
        eprintln!("Game2.dcb not found at {}, skipping", path.display());
        None
    }
}

fn integration_p4k_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("STARBREAKER_TEST_P4K") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
        eprintln!("STARBREAKER_TEST_P4K not found at {}, skipping", path.display());
        return None;
    }

    let path = PathBuf::from(DEFAULT_P4K_PATH);
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "Data.p4k not found at {}. Set STARBREAKER_TEST_P4K to run ignored integration tests.",
            path.display()
        );
        None
    }
}

fn with_integration_context<F>(test_body: F)
where
    F: FnOnce(&Database<'_>, &MappedP4k),
{
    let Some(p4k_path) = integration_p4k_path() else {
        return;
    };

    let p4k = MappedP4k::open(&p4k_path).expect("failed to open Data.p4k");
    let dcb_data = if let Some(dcb_path) = integration_dcb_path() {
        std::fs::read(&dcb_path).expect("failed to read Game2.dcb")
    } else {
        p4k.read_file("Data\\Game2.dcb")
            .or_else(|_| p4k.read_file("Data\\Game.dcb"))
            .expect("failed to read Game2.dcb from Data.p4k")
    };
    let db = Database::from_bytes(&dcb_data).expect("failed to parse Game2.dcb");

    test_body(&db, &p4k);
}

fn glb_json(glb: &[u8]) -> serde_json::Value {
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json_str = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
    serde_json::from_str(json_str.trim()).unwrap()
}

fn find_entity_by_substring<'a>(db: &'a Database<'a>, needle: &str) -> Option<&'a Record> {
    let needle = needle.to_lowercase();
    let entity_struct = db.struct_id("EntityClassDefinition")?;
    db.records_of_type(entity_struct).find(|record| {
        db.resolve_string2(record.name_offset)
            .to_lowercase()
            .contains(&needle)
    })
}

fn export_entity_json(
    db: &Database<'_>,
    p4k: &MappedP4k,
    record: &Record,
    opts: &starbreaker_3d::ExportOptions,
) -> serde_json::Value {
    let tree = starbreaker_datacore::loadout::resolve_loadout(db, record);
    let result = starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, opts)
        .expect("representative bundled export failed");
    glb_json(&result.glb)
}

fn node_material_names(glb: &serde_json::Value, node_name: &str) -> Vec<String> {
    let materials = glb["materials"].as_array().expect("glTF materials should be an array");
    let meshes = glb["meshes"].as_array().expect("glTF meshes should be an array");
    let nodes = glb["nodes"].as_array().expect("glTF nodes should be an array");

    let Some(node) = nodes.iter().find(|node| node["name"].as_str() == Some(node_name)) else {
        return Vec::new();
    };
    let Some(mesh_index) = node["mesh"].as_u64() else {
        return Vec::new();
    };
    let primitives = meshes[mesh_index as usize]["primitives"]
        .as_array()
        .expect("glTF mesh primitives should be an array");

    primitives
        .iter()
        .filter_map(|primitive| primitive["material"].as_u64())
        .filter_map(|material_index| materials.get(material_index as usize))
        .filter_map(|material| material["name"].as_str())
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires Game2.dcb and Data.p4k; set STARBREAKER_TEST_DCB/STARBREAKER_TEST_P4K when needed"]
fn representative_decomposed_export_emits_complete_package() {
    with_integration_context(|db, p4k| {
        let record = find_entity_by_substring(db, "gladius")
            .or_else(|| find_entity_by_substring(db, "aurora"))
            .expect("expected at least one representative ship entity in Datacore");
        let tree = starbreaker_datacore::loadout::resolve_loadout(db, record);
        let opts = starbreaker_3d::ExportOptions {
            kind: starbreaker_3d::ExportKind::Decomposed,
            material_mode: starbreaker_3d::MaterialMode::All,
            include_interior: true,
            include_attachments: true,
            include_lights: true,
            lod_level: 0,
            texture_mip: 0,
            ..Default::default()
        };

        let result = starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, &opts)
            .expect("representative decomposed export failed");
        let decomposed = result
            .decomposed
            .as_ref()
            .expect("decomposed export should return a file package");

        let paths: Vec<&str> = decomposed.files.iter().map(|file| file.relative_path.as_str()).collect();
        assert!(paths.contains(&"scene.json"), "scene manifest missing: {paths:#?}");
        assert!(paths.contains(&"palettes/palettes.json"), "palette manifest missing: {paths:#?}");
        assert!(paths.contains(&"liveries/liveries.json"), "livery manifest missing: {paths:#?}");
        assert!(paths.iter().any(|path| path.starts_with("meshes/")), "mesh assets missing: {paths:#?}");
        assert!(paths.iter().any(|path| path.starts_with("materials/")), "material sidecars missing: {paths:#?}");
        assert!(paths.iter().any(|path| path.starts_with("textures/")), "canonical textures missing: {paths:#?}");
    });
}

#[test]
#[ignore = "requires Data.p4k; set STARBREAKER_TEST_P4K when needed"]
fn mole_front_mining_cab_decomposed_export_keeps_exterior_decals() {
    with_integration_context(|db, p4k| {
        let idx = starbreaker_datacore::loadout::EntityIndex::new(db);
        let record = idx
            .find_record("ARGO_MOLE")
            .expect("ARGO_MOLE not found");
        let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);
        let opts = starbreaker_3d::ExportOptions {
            kind: starbreaker_3d::ExportKind::Decomposed,
            material_mode: starbreaker_3d::MaterialMode::All,
            include_interior: true,
            include_attachments: true,
            include_lights: true,
            lod_level: 0,
            texture_mip: 0,
            ..Default::default()
        };

        let result = starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, &opts)
            .expect("ARGO_MOLE decomposed export failed");
        let decomposed = result
            .decomposed
            .as_ref()
            .expect("decomposed export should return a file package");
        let front_cab = decomposed
            .files
            .iter()
            .find(|file| {
                file.relative_path
                    .ends_with("Data/Objects/Spaceships/Turrets/ARGO/Mole/ARGO_Mole_Front_MiningCab/ARGO_Mole_Front_MiningCab.glb")
            })
            .expect("front mining cab mesh asset missing from decomposed export");

        let glb = glb_json(&front_cab.bytes);
        let fmc_exterior_materials = node_material_names(&glb, "fmc_exterior");

        assert!(
            fmc_exterior_materials.iter().any(|name| name.contains("decals")),
            "expected fmc_exterior to keep a decal primitive, got {fmc_exterior_materials:#?}"
        );
    });
}

#[test]
#[ignore = "requires Game2.dcb and Data.p4k; set STARBREAKER_TEST_DCB/STARBREAKER_TEST_P4K when needed"]
fn assemble_glb_produces_valid_glb() {
    with_integration_context(|db, p4k| {
        // Find an EntityClassDefinition with geometry.
        let opts = starbreaker_3d::ExportOptions::default();
        let mut found = None;
        for record in db.records() {
            let struct_name = db.struct_name(record.struct_id());
            if struct_name == "EntityClassDefinition" {
                let tree = starbreaker_datacore::loadout::resolve_loadout(db, record);
                match starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, &opts) {
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

        // Verify GLB magic bytes.
        assert!(result.glb.len() > 12, "GLB too small");
        assert_eq!(&result.glb[0..4], b"glTF", "missing GLB magic bytes");
    });
}

/// Ensure the depth limit in the Value materializer doesn't silently drop
/// loadout entries. The Gladius is the reference ship: it must export guns
/// (entityClassReference entries at depth ~7-8) and missiles (nested loadout
/// entries at depth ~4-5).
#[test]
#[ignore = "requires Game2.dcb and Data.p4k; set STARBREAKER_TEST_DCB/STARBREAKER_TEST_P4K when needed"]
fn depth_limit_preserves_gladius_loadout() {
    with_integration_context(|db, p4k| {
        let idx = starbreaker_datacore::loadout::EntityIndex::new(db);
        let record = idx
            .find_record("AEGS_Gladius")
            .expect("AEGS_Gladius not found");
        let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);

        // Flatten all loadout entries.
        fn collect_names(nodes: &[starbreaker_datacore::loadout::LoadoutNode], out: &mut Vec<String>) {
            for n in nodes {
                out.push(format!("{} -> {}", n.item_port_name, n.entity_name));
                collect_names(&n.children, out);
            }
        }
        let mut names = Vec::new();
        collect_names(&tree.root.children, &mut names);

        // Guns must be present (entityClassReference at depth ~7-8).
        let has_gun_mount = names.iter().any(|n| n.contains("hardpoint_gun_nose"));
        assert!(
            has_gun_mount,
            "missing gun mount — depth limit too low?\nentries: {names:#?}"
        );

        // Missiles must be present (nested loadout entries).
        let has_missiles = names.iter().any(|n| n.contains("missile_01_attach"));
        assert!(
            has_missiles,
            "missing missiles — depth limit too low?\nentries: {names:#?}"
        );

        // Export must succeed with geometry.
        let opts = starbreaker_3d::ExportOptions {
            material_mode: starbreaker_3d::MaterialMode::Colors,
            include_interior: false,
            lod_level: 0,
            texture_mip: 0,
            ..Default::default()
        };
        let result = starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, &opts)
            .expect("Gladius export failed");

        // Must have at least 45 child meshes (hull + thrusters + guns + missiles + components).
        let root = glb_json(&result.glb);
        let mesh_count = root["meshes"].as_array().map(|a| a.len()).unwrap_or(0);

        // 48 child meshes + root NMC meshes ≈ lots of meshes.
        assert!(
            mesh_count >= 50,
            "too few meshes ({mesh_count}) — loadout entries likely truncated by depth limit"
        );
    });
}

/// Stress test: the Idris P Collector Military is the heaviest ship (295 geometry
/// entries). Ensures the depth limit and Value materializer handle the most
/// complex loadout without crashing, OOMing, or silently dropping entries.
#[test]
#[ignore = "requires Game2.dcb and Data.p4k; set STARBREAKER_TEST_DCB/STARBREAKER_TEST_P4K when needed"]
fn depth_limit_preserves_idris_p_collector_military() {
    with_integration_context(|db, p4k| {
        let idx = starbreaker_datacore::loadout::EntityIndex::new(db);
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

        // Must resolve at least 250 geometry entries (full count is ~295).
        assert!(
            geom_count >= 250,
            "Idris P CM: only {geom_count} geometry entries (expected 250+) — depth limit too low?"
        );

        // Export must succeed.
        let opts = starbreaker_3d::ExportOptions {
            material_mode: starbreaker_3d::MaterialMode::Colors,
            include_interior: false,
            lod_level: 2,
            texture_mip: 0,
            ..Default::default()
        };
        let result = starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, &opts)
            .expect("Idris P Collector Military export failed");

        assert!(
            result.glb.len() > 10_000_000,
            "GLB too small ({} bytes) — geometry likely missing",
            result.glb.len()
        );
    });
}

#[test]
#[ignore = "requires Game2.dcb and Data.p4k; set STARBREAKER_TEST_DCB/STARBREAKER_TEST_P4K when needed"]
fn representative_bundled_exports_preserve_semantic_metadata() {
    with_integration_context(|db, p4k| {
        let opts = starbreaker_3d::ExportOptions {
            material_mode: starbreaker_3d::MaterialMode::Colors,
            include_interior: false,
            lod_level: 0,
            texture_mip: 0,
            ..Default::default()
        };

        let gladius = find_entity_by_substring(db, "gladius")
            .expect("expected at least one Gladius-like entity in Datacore");
        let gladius_root = export_entity_json(db, p4k, gladius, &opts);
        let gladius_materials = gladius_root["materials"]
            .as_array()
            .expect("Gladius export should contain materials");
        assert!(
            gladius_materials.iter().any(|material| material["extras"]["semantic"]["shader_family"].is_string()),
            "expected representative bundled export to preserve shader-family semantics"
        );
        assert!(
            gladius_materials.iter().any(|material| material["extras"]["semantic"]["activation_state"].is_object()),
            "expected representative bundled export to preserve activation-state semantics"
        );

        let variant = ["vulture", "starlancer", "mole", "talon"]
            .into_iter()
            .find_map(|needle| find_entity_by_substring(db, needle))
            .expect("expected at least one representative livery-sensitive entity in Datacore");
        let variant_root = export_entity_json(db, p4k, variant, &opts);
        let variant_materials = variant_root["materials"]
            .as_array()
            .expect("representative variant export should contain materials");
        assert!(
            variant_materials
                .iter()
                .any(|material| material["extras"]["semantic"]["material_set_identity"].is_object()),
            "expected representative bundled export to preserve material-set identity"
        );
        assert!(
            variant_materials.iter().any(|material| {
                material["extras"]["semantic"]["palette"].is_object()
                    || material["extras"]["semantic"]["layer_manifest"].is_array()
            }),
            "expected representative bundled export to preserve palette or layer semantics on a livery-sensitive entity"
        );
    });
}
