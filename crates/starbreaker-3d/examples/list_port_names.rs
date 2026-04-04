//! List all port names and helper names for an entity.
use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;

fn main() {
    let search = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: list_port_names <entity_name>");
        std::process::exit(1);
    }).to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_data = p4k.read_file("Data\\Game2.dcb").expect("failed to read Game2.dcb");
    let db = Database::from_bytes(&dcb_data).unwrap();

    let mut candidates: Vec<_> = db.records().iter()
        .filter(|r| {
            db.struct_name(r.struct_id()) == "EntityClassDefinition" &&
            db.resolve_string2(r.name_offset).to_lowercase().contains(&search)
        })
        .collect();
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
    let record = candidates.first().unwrap();
    let name = db.resolve_string2(record.name_offset);
    eprintln!("Record: {name}");

    let compiled = db.compile_path::<Value>(
        record.struct_id(),
        "Components[SItemPortContainerComponentParams]",
    ).unwrap();
    let components = db.query::<Value>(&compiled, record).unwrap();

    for comp in &components {
        let ports = get_array(comp, "Ports").unwrap_or(&EMPTY);
        println!("Ports ({} total):", ports.len());
        for port in ports {
            let port_name = get_str(port, "Name").unwrap_or("?");
            let helper_name = get_obj(port, "AttachmentImplementation")
                .and_then(|ai| get_obj(ai, "Helper"))
                .and_then(|hn| get_obj(hn, "Helper"));
            let bone = helper_name.and_then(|h| get_str(h, "Name")).unwrap_or("");
            let offset_pos = helper_name
                .and_then(|h| get_obj(h, "Offset"))
                .and_then(|o| get_obj(o, "Position"));
            let (px, py, pz) = if let Some(p) = offset_pos {
                (get_f32(p, "x"), get_f32(p, "y"), get_f32(p, "z"))
            } else {
                (0.0, 0.0, 0.0)
            };
            let offset_rot = helper_name
                .and_then(|h| get_obj(h, "Offset"))
                .and_then(|o| get_obj(o, "Rotation"));
            let (rx, ry, rz) = if let Some(r) = offset_rot {
                (get_f32(r, "x"), get_f32(r, "y"), get_f32(r, "z"))
            } else {
                (0.0, 0.0, 0.0)
            };
            let no_rot = get_obj(port, "AttachmentImplementation")
                .and_then(|ai| get_obj(ai, "constraintParams"))
                .and_then(|cp| get_bool(cp, "noRotation"))
                .unwrap_or(false);
            println!("  {port_name:40} helper={bone:40} pos=[{px:.2},{py:.2},{pz:.2}] rot=[{rx:.1},{ry:.1},{rz:.1}] noRot={no_rot}");
        }
    }
}

static EMPTY: Vec<Value<'static>> = Vec::new();

fn get_str<'a>(val: &'a Value, name: &str) -> Option<&'a str> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields { if *k == name { if let Value::String(s) = v { return Some(s); } } }
    }
    None
}
fn get_obj<'a>(val: &'a Value, name: &str) -> Option<&'a Value<'a>> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields { if *k == name { if let Value::Object { .. } = v { return Some(v); } } }
    }
    None
}
fn get_array<'a>(val: &'a Value, name: &str) -> Option<&'a Vec<Value<'a>>> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields { if *k == name { if let Value::Array(a) = v { return Some(a); } } }
    }
    None
}
fn get_f32(val: &Value, name: &str) -> f32 {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                match v {
                    Value::Float(f) => return *f,
                    Value::Double(f) => return *f as f32,
                    Value::Int32(i) => return *i as f32,
                    _ => {}
                }
            }
        }
    }
    0.0
}
fn get_bool(val: &Value, name: &str) -> Option<bool> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields { if *k == name { if let Value::Bool(b) = v { return Some(*b); } } }
    }
    None
}
