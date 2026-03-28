//! List all EntityClassDefinition records that have SGeometryResourceParams,
//! showing the geometry file path. Output to stdout (redirect to file).

use starbreaker_datacore::database::Database;

fn main() {
    eprintln!("Loading DCB...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb from P4k");
    let db = Database::from_bytes(&dcb_data).expect("failed to parse DCB");

    let geom_path = db.compile_rooted::<String>(
        "EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    ).expect("failed to compile geometry path");

    let mut count = 0;
    for record in db.records_of_type(geom_path.root_struct_id()) {
        let paths: Vec<String> = match db.query::<String>(&geom_path, record) {
            Ok(p) => p,
            Err(_) => continue,
        };

        if paths.is_empty() {
            continue;
        }

        let record_name = db.resolve_string2(record.name_offset);
        for path in &paths {
            let ext = path.rsplit('.').next().unwrap_or("");
            println!("{ext}\t{record_name}\t{path}");
            count += 1;
        }
    }

    eprintln!("{count} entities with geometry found");
}
