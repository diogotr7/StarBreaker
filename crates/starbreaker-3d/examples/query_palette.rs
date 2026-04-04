//! Query the TintPalette colors for an entity from DataCore.
use starbreaker_datacore::database::Database;

fn main() {
    let search = std::env::args().nth(1).unwrap_or("AEGS_Gladius".into());
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb from P4k");
    let db = Database::from_bytes(&dcb_data).expect("parse DCB");

    // Find shortest-name match (most likely the main entity)
    let mut candidates: Vec<_> = db
        .records()
        .iter()
        .filter(|r| {
            let sname = db.struct_name(r.struct_id());
            sname == "EntityClassDefinition" && db.resolve_string2(r.name_offset).contains(&search)
        })
        .collect();
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
    let record = candidates.first().expect("entity not found");

    println!("Entity: {}", db.resolve_string2(record.name_offset));

    // Try to query palette color values directly through the Reference chain
    let base = "Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord";

    // Try entryA tint color
    for entry in ["entryA", "entryB", "entryC"] {
        for channel in ["r", "g", "b"] {
            let path = format!("{base}.root.{entry}.tintColor.{channel}");
            match db.compile_path::<u8>(record.struct_id(), &path) {
                Ok(compiled) => match db.query::<u8>(&compiled, record) {
                    Ok(vals) if !vals.is_empty() => {
                        print!("{entry}.tintColor.{channel} = {}", vals[0]);
                        if vals.len() > 1 {
                            print!(" ({} results)", vals.len());
                        }
                        println!();
                    }
                    Ok(_) => println!("{entry}.{channel}: empty"),
                    Err(e) => println!("{entry}.{channel}: query err: {e}"),
                },
                Err(e) => {
                    println!("compile err for {path}: {e}");
                    // Try without TintPaletteRef type filter
                    let alt = format!(
                        "Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord.root.{entry}.tintColor.{channel}"
                    );
                    match db.compile_path::<u8>(record.struct_id(), &alt) {
                        Ok(compiled) => match db.query::<u8>(&compiled, record) {
                            Ok(vals) if !vals.is_empty() => {
                                println!("  ALT: {entry}.{channel} = {}", vals[0])
                            }
                            _ => println!("  ALT also failed"),
                        },
                        Err(e2) => println!("  ALT compile err: {e2}"),
                    }
                    break; // Don't repeat for each channel
                }
            }
        }
    }

    // Also try glass color
    for channel in ["r", "g", "b"] {
        let path = format!("{base}.root.glassColor.{channel}");
        if let Ok(compiled) = db.compile_path::<u8>(record.struct_id(), &path)
            && let Ok(vals) = db.query::<u8>(&compiled, record)
            && !vals.is_empty()
        {
            println!("glassColor.{channel} = {}", vals[0]);
        }
    }
}
