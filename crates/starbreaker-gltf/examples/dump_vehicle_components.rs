//! Probe VehicleComponentParams to find objectContainers (ship interiors).
use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;

fn main() {
    let search = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("Usage: dump_vehicle_components <entity_name>");
            std::process::exit(1);
        })
        .to_lowercase();

    eprintln!("Loading DCB...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb from P4k");
    let db = Database::from_bytes(&dcb_data).unwrap();

    // Find the entity record
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
    println!("Record: {record_name}");

    // Try VehicleComponentParams
    println!("\n--- VehicleComponentParams ---");
    if let Ok(path) =
        db.compile_path::<Value>(record.struct_id(), "Components[VehicleComponentParams]")
    {
        match db.query::<Value>(&path, record) {
            Ok(vals) if !vals.is_empty() => {
                for v in &vals {
                    print_value(v, 2, 6);
                }
            }
            Ok(_) => println!("  (empty — component type exists but no data for this entity)"),
            Err(e) => println!("  Query error: {e}"),
        }
    } else {
        println!("  VehicleComponentParams not found in schema");
    }

    // Also list ALL component type names on this entity
    println!("\n--- All Components (type names only) ---");
    if let Ok(path) = db.compile_path::<Value>(record.struct_id(), "Components") {
        match db.query::<Value>(&path, record) {
            Ok(components) => {
                for (i, comp) in components.iter().enumerate() {
                    if let Value::Object { type_name, .. } = comp {
                        println!("  [{i}] {type_name}");
                    } else {
                        println!("  [{i}] {comp:?}");
                    }
                }
            }
            Err(e) => println!("  Query error: {e}"),
        }
    }

    // Try to find objectContainers via VehicleComponentParams
    println!("\n--- VehicleComponentParams.objectContainers ---");
    if let Ok(path) = db.compile_path::<Value>(
        record.struct_id(),
        "Components[VehicleComponentParams].objectContainers",
    ) {
        match db.query::<Value>(&path, record) {
            Ok(vals) if !vals.is_empty() => {
                for v in &vals {
                    print_value(v, 2, 6);
                }
            }
            Ok(_) => println!("  (empty)"),
            Err(e) => println!("  Query error: {e}"),
        }
    } else {
        println!("  Path not compilable");
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
                    Value::String(s) => println!("\"{}\"", s.chars().take(120).collect::<String>()),
                    Value::Guid(g) => println!("{g}"),
                    Value::Null => println!("null"),
                    Value::Bool(b) => println!("{b}"),
                    Value::Int32(n) => println!("{n}"),
                    Value::Float(f) => println!("{f}"),
                    Value::Array(arr) => {
                        println!("[{} items]", arr.len());
                        for (i, item) in arr.iter().take(10).enumerate() {
                            print!("{}    [{i}]: ", " ".repeat(indent));
                            print_value(item, indent + 6, max_depth - 1);
                        }
                        if arr.len() > 10 {
                            println!("{}    ... and {} more", " ".repeat(indent), arr.len() - 10);
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
