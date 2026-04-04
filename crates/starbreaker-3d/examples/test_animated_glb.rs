//! Export an animated GLB using the full loadout pipeline.
//! Usage: test_animated_glb [entity_search] [dba_search] [output.glb]

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let entity_search = args.get(1).map(|s| s.as_str()).unwrap_or("AEGS_Avenger_Titan");
    let dba_search = args.get(2).map(|s| s.as_str()).unwrap_or("Ships/AEGS/Avenger.dba");
    let output_path = args.get(3).map(|s| s.as_str()).unwrap_or("animated.glb");

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_entry = p4k.entries().iter().find(|e| e.name.ends_with(".dcb")).unwrap();
    let dcb_data = p4k.read(dcb_entry).unwrap();
    let db = starbreaker_datacore::Database::from_bytes(&dcb_data).unwrap();

    // Find entity (exact match on last name component, then substring fallback)
    let entity_search_lower = entity_search.to_lowercase();
    let record = db.records_by_type_name("EntityClassDefinition")
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

    // Build loadout tree
    let tree = starbreaker_datacore::loadout::resolve_loadout(&db, record);

    // Find and parse DBA
    let dba_search_lower = dba_search.to_lowercase().replace('/', "\\");
    let dba_entry = p4k.entries().iter()
        .find(|e| {
            let n = e.name.to_lowercase();
            n.contains(&dba_search_lower) && n.ends_with(".dba")
        })
        .unwrap_or_else(|| panic!("No .dba matching '{dba_search}'"));

    eprintln!("DBA: {} ({} bytes)", dba_entry.name, dba_entry.uncompressed_size);
    let dba_data = p4k.read(dba_entry).unwrap();
    let anim_db = starbreaker_3d::animation::dba::parse_dba(&dba_data).unwrap();
    eprintln!("Parsed {} animation clips", anim_db.clips.len());

    // Export with full loadout + animations
    let opts = starbreaker_3d::ExportOptions {
        material_mode: starbreaker_3d::MaterialMode::None,
        lod_level: 1,
        texture_mip: 0,
        include_attachments: true,
        include_interior: false,
        ..Default::default()
    };

    let result = starbreaker_3d::assemble_glb_with_loadout_and_animations(
        &db, &p4k, record, &tree, &opts, anim_db.clips,
    ).expect("failed to build GLB");

    std::fs::write(output_path, &result.glb).unwrap();
    eprintln!("Wrote {} ({:.1} MB)", output_path, result.glb.len() as f64 / 1_048_576.0);
}
