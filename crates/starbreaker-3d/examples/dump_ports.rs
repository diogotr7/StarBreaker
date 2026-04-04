use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;

fn main() {
    let search = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("Usage: dump_ports <entity_name>");
            std::process::exit(1);
        })
        .to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb from P4k");
    let db = Database::from_bytes(&dcb_data).unwrap();

    // Find exact match first, then contains, prefer shortest
    let mut candidates: Vec<_> = db
        .records()
        .iter()
        .filter(|r| {
            let sn = db.struct_name(r.struct_id());
            if sn != "EntityClassDefinition" {
                return false;
            }
            let name = db.resolve_string2(r.name_offset);
            name.rsplit('.').next().unwrap_or(name).to_lowercase() == search
                || name
                    .rsplit('.')
                    .next()
                    .unwrap_or(name)
                    .to_lowercase()
                    .contains(&search)
        })
        .collect();
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
    let record = candidates.first().unwrap();

    let name = db.resolve_string2(record.name_offset);
    eprintln!("Record: {name}");

    match db.compile_path::<Value>(
        record.struct_id(),
        "Components[SItemPortContainerComponentParams]",
    ) {
        Ok(compiled) => match db.query::<Value>(&compiled, record) {
            Ok(results) => {
                eprintln!("Found {} SItemPortContainerComponentParams", results.len());
                for comp in &results {
                    print_value(comp, 0, 8);
                }
            }
            Err(e) => eprintln!("Query error: {e}"),
        },
        Err(e) => eprintln!("Compile error: {e}"),
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
                    Value::String(s) => println!("\"{s}\""),
                    Value::Float(f) => println!("{f}"),
                    Value::Null => println!("null"),
                    Value::Bool(b) => println!("{b}"),
                    Value::Array(arr) => {
                        println!("[{} items]", arr.len());
                        for (i, item) in arr.iter().take(3).enumerate() {
                            print!("{}    [{i}]: ", " ".repeat(indent));
                            print_value(item, indent + 6, max_depth - 1);
                        }
                        if arr.len() > 3 {
                            println!("{}    ... and {} more", " ".repeat(indent), arr.len() - 3);
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
