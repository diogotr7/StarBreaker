//! Export an animated GLB for visual testing.
//!
//! Usage: test_animated_glb [entity_search] [dba_search] [output.glb]
//! Defaults: Zeus CL entity, Zeus ship DBA, zeus_animated.glb

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let entity_search = args.get(1).map(|s| s.as_str()).unwrap_or("Zeus CL");
    let dba_search = args.get(2).map(|s| s.as_str()).unwrap_or("Ships/RSI/Zeus.dba");
    let output_path = args.get(3).map(|s| s.as_str()).unwrap_or("zeus_animated.glb");

    // Open P4k and DataCore
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_entry = p4k
        .entries()
        .iter()
        .find(|e| e.name.ends_with(".dcb"))
        .expect("no .dcb in P4k");
    let dcb_data = p4k.read(dcb_entry).unwrap();
    let db = starbreaker_datacore::Database::from_bytes(&dcb_data).unwrap();

    // Find entity
    let entity_search_lower = entity_search.to_lowercase();
    // Two-pass: exact match first, then substring
    let record = db
        .records_by_type_name("EntityClassDefinition")
        .find(|r| {
            let name = db.resolve_string2(r.name_offset);
            let short = name.rsplit('.').next().unwrap_or(name);
            short.eq_ignore_ascii_case(entity_search)
        })
        .or_else(|| db.records_by_type_name("EntityClassDefinition").find(|r| {
            db.resolve_string2(r.name_offset).to_lowercase().contains(&entity_search_lower)
        }))
        .unwrap_or_else(|| panic!("No entity matching '{entity_search}'"));

    let entity_name = db.resolve_string2(record.name_offset);
    eprintln!("Entity: {entity_name}");

    // Export geometry via pipeline (no materials for speed)
    let opts = starbreaker_3d::ExportOptions {
        material_mode: starbreaker_3d::MaterialMode::None,
        lod_level: 0,
        texture_mip: 0,
        include_attachments: true,
        include_interior: false,
        ..Default::default()
    };

    let (mesh, materials, _textures, nmc, palette, geometry_path, material_path, skeleton_bones) =
        starbreaker_3d::export_entity_payload(&db, &p4k, record, &opts)
            .expect("failed to export entity");

    eprintln!(
        "Geometry: {} verts, {} nodes, {} skeleton bones",
        mesh.positions.len(),
        nmc.as_ref().map(|n| n.nodes.len()).unwrap_or(0),
        skeleton_bones.len()
    );

    // Find and parse DBA (normalize slashes — P4k uses backslashes)
    let dba_search_lower = dba_search.to_lowercase().replace('/', "\\");
    let dba_entry = p4k
        .entries()
        .iter()
        .find(|e| {
            let name = e.name.to_lowercase();
            name.contains(&dba_search_lower) && name.ends_with(".dba")
        })
        .unwrap_or_else(|| panic!("No .dba matching '{dba_search}'"));

    eprintln!(
        "DBA: {} ({} bytes)",
        dba_entry.name, dba_entry.uncompressed_size
    );
    let dba_data = p4k.read(dba_entry).unwrap();
    let anim_db =
        starbreaker_3d::animation::dba::parse_dba(&dba_data).expect("failed to parse DBA");
    eprintln!("Parsed {} animation clips", anim_db.clips.len());

    // Build GLB with animations
    use starbreaker_3d::gltf::*;
    let glb = write_glb(
        GlbInput {
            root_mesh: Some(mesh),
            root_materials: materials,
            root_textures: None,
            root_nmc: nmc,
            root_palette: palette,
            skeleton_bones,
            children: Vec::new(),
            interiors: starbreaker_3d::pipeline::LoadedInteriors::default(),
            animations: anim_db.clips,
        },
        &mut GlbLoaders {
            load_textures: &mut |_| None,
            load_interior_mesh: &mut |_| None,
        },
        &GlbOptions {
            material_mode: starbreaker_3d::MaterialMode::None,
            metadata: GlbMetadata {
                entity_name: Some(entity_name.to_string()),
                geometry_path: Some(geometry_path),
                material_path: Some(material_path),
                export_options: ExportOptionsMetadata {
                    material_mode: "None".into(),
                    format: "Glb".into(),
                    lod_level: 0,
                    texture_mip: 0,
                    include_attachments: true,
                    include_interior: false,
                },
            },
            fallback_palette: None,
        },
    )
    .expect("failed to build GLB");

    std::fs::write(output_path, &glb).unwrap();
    eprintln!(
        "Wrote {} ({:.1} MB)",
        output_path,
        glb.len() as f64 / 1_048_576.0
    );
}
