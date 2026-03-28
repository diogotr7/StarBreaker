//! Dump the loadout hierarchy of an entity from DataCore.
//! Shows how ship parts are assembled from sub-entities.
use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;

fn main() {
    let search = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("Usage: dump_loadout <entity_name>");
            std::process::exit(1);
        })
        .to_lowercase();

    eprintln!("Loading DCB...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb from P4k");
    let db = Database::from_bytes(&dcb_data).unwrap();

    // Find the entity record (prefer exact match, then shortest containing match)
    let mut candidates: Vec<_> = db
        .records()
        .iter()
        .filter(|r| {
            let struct_name = db.struct_name(r.struct_id());
            if struct_name != "EntityClassDefinition" {
                return false;
            }
            let name = db.resolve_string2(r.name_offset);
            name.to_lowercase().contains(&search)
        })
        .collect();
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
    let record = candidates.first().unwrap_or_else(|| {
        eprintln!("No EntityClassDefinition matching '{search}'");
        std::process::exit(1);
    });

    let record_name = db.resolve_string2(record.name_offset);
    let struct_name = db.struct_name(record.struct_id());
    println!("Record: {record_name} (type: {struct_name})");

    // Try to get the loadout component
    println!("\n--- SEntityComponentDefaultLoadoutParams ---");
    if let Ok(path) = db.compile_path::<Value>(
        record.struct_id(),
        "Components[SEntityComponentDefaultLoadoutParams]",
    ) {
        match db.query::<Value>(&path, record) {
            Ok(loadouts) if !loadouts.is_empty() => {
                for loadout in &loadouts {
                    print_value(loadout, 2, 5);
                }
            }
            Ok(_) => println!("  (empty)"),
            Err(e) => println!("  Query error: {e}"),
        }
    } else {
        println!("  Not found in schema");
    }

    // Also check geometry
    println!("\n--- SGeometryResourceParams ---");
    if let Ok(path) =
        db.compile_path::<Value>(record.struct_id(), "Components[SGeometryResourceParams]")
    {
        match db.query::<Value>(&path, record) {
            Ok(geoms) if !geoms.is_empty() => {
                for geom in &geoms {
                    print_value(geom, 2, 4);
                }
            }
            Ok(_) => println!("  (empty)"),
            Err(e) => println!("  Query error: {e}"),
        }
    } else {
        println!("  Not found in schema");
    }

    // Also look for SEntityComponentDefaultLoadoutParams (loadout)
    println!("\n--- Looking for loadout components ---");
    if let Ok(path) = db.compile_path::<Value>(
        record.struct_id(),
        "Components[SEntityComponentDefaultLoadoutParams]",
    ) {
        match db.query::<Value>(&path, record) {
            Ok(loadouts) => {
                for (i, loadout) in loadouts.iter().enumerate() {
                    println!("Loadout[{i}]:");
                    print_value(loadout, 2, 4); // max depth 4
                }
            }
            Err(e) => println!("  Query error: {e}"),
        }
    } else {
        println!("  No SEntityComponentDefaultLoadoutParams found");
    }
}

fn print_value(val: &Value, indent: usize, max_depth: usize) {
    if max_depth == 0 {
        println!("{}...", " ".repeat(indent));
        return;
    }
    match val {
        Value::Object { type_name, fields, .. } => {
            println!("{}{{{type_name}}}", " ".repeat(indent));
            for (name, v) in fields {
                print!("{}  {name}: ", " ".repeat(indent));
                match v {
                    Value::String(s) => println!("\"{}\"", s.chars().take(80).collect::<String>()),
                    Value::Guid(g) => println!("{g}"),
                    Value::Null => println!("null"),
                    Value::Bool(b) => println!("{b}"),
                    Value::Int32(n) => println!("{n}"),
                    Value::Float(f) => println!("{f}"),
                    Value::Array(arr) => {
                        println!("[{} items]", arr.len());
                        for (i, item) in arr.iter().take(5).enumerate() {
                            print!("{}    [{i}]: ", " ".repeat(indent));
                            print_value(item, indent + 6, max_depth - 1);
                        }
                        if arr.len() > 5 {
                            println!("{}    ... and {} more", " ".repeat(indent), arr.len() - 5);
                        }
                    }
                    Value::Object { .. } => {
                        println!();
                        print_value(v, indent + 4, max_depth - 1);
                    }
                    _ => println!("{v:?}"),
                }
            }
        }
        _ => println!("{}{val:?}", " ".repeat(indent)),
    }
}
