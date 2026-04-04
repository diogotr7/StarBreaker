//! Single-ship export with max settings for CPU profiling.
//!
//! Usage:
//!     cargo run --release --example profile_idris

use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::{EntityIndex, resolve_loadout_indexed};

fn main() {
    env_logger::init();

    let opts = starbreaker_3d::ExportOptions {
        material_mode: starbreaker_3d::MaterialMode::Textures,
        include_interior: true,
        lod_level: 0,
        texture_mip: 0,
        ..Default::default()
    };

    eprintln!("Opening P4k...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    eprintln!("Loading Game2.dcb...");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb");
    let db = Database::from_bytes(&dcb_data).expect("failed to parse DCB");

    let entity_si = db
        .struct_id("EntityClassDefinition")
        .expect("EntityClassDefinition not found");
    let record = db
        .records_of_type(entity_si)
        .find(|r| db.resolve_string2(r.name_offset) == "EntityClassDefinition.AEGS_Idris_M")
        .expect("AEGS_Idris_M not found");

    let name = db.resolve_string2(record.name_offset);
    eprintln!("Exporting {name} with max settings...");

    let idx = EntityIndex::new(&db);
    let tree = resolve_loadout_indexed(&idx, record);

    let t0 = std::time::Instant::now();
    let result = starbreaker_3d::assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts)
        .expect("export failed");
    let elapsed = t0.elapsed();

    let output = "idris_profile.glb";
    std::fs::write(output, &result.glb).expect("failed to write");

    eprintln!("GLB: {} bytes ({:.1} MB)", result.glb.len(), result.glb.len() as f64 / (1024.0 * 1024.0));
    eprintln!("Time: {:.2}s", elapsed.as_secs_f64());
}
