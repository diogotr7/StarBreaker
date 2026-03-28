use starbreaker_datacore::Database;
use std::{env, fs};

fn main() {
    let data = if let Some(dcb_path) = env::args().nth(1) {
        fs::read(&dcb_path).expect("failed to read DCB file")
    } else {
        let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
        p4k.read_file("Data\\Game2.dcb")
            .expect("failed to read Game2.dcb")
    };
    let db = Database::from_bytes(&data).expect("failed to parse");

    println!("Struct definitions: {}", db.struct_defs().len());
    println!("Property definitions: {}", db.property_defs().len());
    println!("Enum definitions: {}", db.enum_defs().len());
    println!("Data mappings: {}", db.data_mappings().len());
    println!("Records: {}", db.records().len());
}
