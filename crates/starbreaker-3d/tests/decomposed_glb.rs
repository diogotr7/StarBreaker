//! Live-data regression for decomposed GLB payloads and material slot identity.
use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
use starbreaker_datacore::{
    Database,
    loadout::{EntityIndex, resolve_loadout_indexed},
};

#[test]
fn decomposed_glb_meshes_are_real_glbs() {
    let Ok(found) = starbreaker_common::discover::find_p4k() else {
        eprintln!("skipping: no Data.p4k installed");
        return;
    };
    let p4k = starbreaker_p4k::MappedP4k::open(found.path).unwrap();
    let path = if p4k.entry_case_insensitive(r"Data\Game2.dcb").is_some() {
        r"Data\Game2.dcb"
    } else {
        r"Data\Game.dcb"
    };
    let bytes = p4k.read_file(path).unwrap();
    let db = Database::from_bytes(&bytes).unwrap();
    let idx = EntityIndex::new(&db);
    let record = idx
        .find_record("AEGS_Gladius")
        .expect("live regression fixture");
    let tree = resolve_loadout_indexed(&idx, record);
    let opts = ExportOptions {
        kind: ExportKind::Decomposed,
        format: ExportFormat::Glb,
        include_animations: true,
        apply_default_animation_pose: false,
        ..Default::default()
    };
    let result =
        starbreaker_3d::assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts).unwrap();
    let files = result.decomposed.unwrap().files;
    let meshes: Vec<_> = files
        .iter()
        .filter(|f| f.relative_path.ends_with(".glb"))
        .collect();
    assert!(!meshes.is_empty());
    let scene_file = files
        .iter()
        .find(|f| f.relative_path.ends_with("/scene.json"))
        .unwrap();
    let scene: serde_json::Value = serde_json::from_slice(&scene_file.bytes).unwrap();
    let root = &scene["root_entity"];
    let root_mesh = files
        .iter()
        .find(|f| f.relative_path == root["mesh_asset"].as_str().unwrap())
        .unwrap();
    let len = u32::from_le_bytes(root_mesh.bytes[12..16].try_into().unwrap()) as usize;
    let root_glb: serde_json::Value =
        serde_json::from_slice(&root_mesh.bytes[20..20 + len]).unwrap();
    let sidecar = files
        .iter()
        .find(|f| f.relative_path == root["material_sidecar"].as_str().unwrap())
        .unwrap();
    let sidecar: serde_json::Value = serde_json::from_slice(&sidecar.bytes).unwrap();
    assert_eq!(root_glb["nodes"].as_array().unwrap().len(), 274);
    assert_eq!(root_glb["meshes"].as_array().unwrap().len(), 89);
    assert_eq!(root_glb["materials"].as_array().unwrap().len(), 27);
    assert_eq!(sidecar["submaterials"].as_array().unwrap().len(), 28);
    for (i, material) in root_glb["materials"].as_array().unwrap().iter().enumerate() {
        let submaterial = sidecar["submaterials"][i]["submaterial_name"]
            .as_str()
            .unwrap();
        assert!(
            material["name"]
                .as_str()
                .unwrap()
                .ends_with(&format!("_{submaterial}_{i:02}")),
            "GLB slot {i} must match the sidecar array position: {}",
            material["name"]
        );
    }
    let highest = root_glb["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["primitives"].as_array().unwrap())
        .filter_map(|p| p["material"].as_u64())
        .max()
        .unwrap();
    assert_eq!(highest, 26);
    for file in meshes {
        assert!(
            file.bytes.starts_with(b"glTF"),
            "empty/invalid mesh: {} ({} bytes, prefix {:?})",
            file.relative_path,
            file.bytes.len(),
            file.bytes.get(..4)
        );
        let len = u32::from_le_bytes(file.bytes[12..16].try_into().unwrap()) as usize;
        let doc: serde_json::Value = serde_json::from_slice(&file.bytes[20..20 + len]).unwrap();
        for (i, material) in doc["materials"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let suffix = material["name"]
                .as_str()
                .unwrap()
                .rsplit('_')
                .next()
                .unwrap();
            assert!(
                material["name"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!("_{i:02}"))
            );
            assert_eq!(
                suffix.parse::<usize>().unwrap(),
                i,
                "material slot identity"
            );
        }
    }
}
