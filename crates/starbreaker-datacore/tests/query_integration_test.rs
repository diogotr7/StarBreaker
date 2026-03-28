use starbreaker_datacore::database::Database;

fn load_game_dcb() -> Option<Vec<u8>> {
    let p4k = starbreaker_p4k::open_p4k().ok()?;
    p4k.read_file("Data\\Game2.dcb").ok()
}

#[test]
#[ignore]
fn query_entity_geometry_path_from_real_dcb() {
    let data = match load_game_dcb() {
        Some(d) => d,
        None => {
            eprintln!("Game2.dcb not found, skipping integration test");
            return;
        }
    };
    let db = Database::from_bytes(&data).unwrap();

    // Find an EntityClassDefinition record with a SGeometryResourceParams component
    let mut found_entity = None;
    for record in db.records() {
        let struct_name = db.struct_name(record.struct_id());
        if struct_name == "EntityClassDefinition"
            && let Ok(path) = db.compile_path::<String>(
                record.struct_id(),
                "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
            )
            && let Ok(Some(val)) = db.query_single::<String>(&path, record)
        {
            found_entity = Some((db.resolve_string2(record.name_offset).to_string(), val));
            break;
        }
    }

    let (entity_name, geom_path) = found_entity
        .expect("should find at least one EntityClassDefinition with SGeometryResourceParams");
    println!("Entity: {entity_name}");
    println!("Geometry path: {geom_path}");
    assert!(
        geom_path.contains('.'),
        "geometry path should have file extension: {geom_path}"
    );
}
