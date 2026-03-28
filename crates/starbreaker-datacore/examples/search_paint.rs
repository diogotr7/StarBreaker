use starbreaker_datacore::Database;
/// Search DataCore for paint/livery/palette related struct types and records.
/// Usage: cargo run --example search_paint [path.dcb]
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

    println!("=== Database stats ===");
    println!("Structs: {}", db.struct_defs().len());
    println!("Records: {}", db.records().len());
    println!();

    // --- Step 1: Find struct types matching paint/livery/palette/skin/tint/color ---
    let keywords = [
        "paint", "livery", "palette", "skin", "tint", "color", "colour", "decal", "dye",
    ];

    println!("=== Struct types matching keywords ===");
    let mut matching_struct_indices: Vec<(usize, &str)> = Vec::new();
    for (i, sd) in db.struct_defs().iter().enumerate() {
        let name = db.resolve_string2(sd.name_offset);
        let lower = name.to_lowercase();
        for kw in &keywords {
            if lower.contains(kw) {
                println!("  [{:>4}] {}", i, name);
                matching_struct_indices.push((i, name));
                break;
            }
        }
    }
    println!(
        "Total matching struct types: {}",
        matching_struct_indices.len()
    );
    println!();

    // --- Step 2: Find records whose struct type matches ---
    println!("=== Records with matching struct types ===");
    let matching_set: std::collections::HashSet<usize> =
        matching_struct_indices.iter().map(|&(i, _)| i).collect();

    let mut paint_records: Vec<(usize, &str, &str, &str)> = Vec::new();
    for (ri, record) in db.records().iter().enumerate() {
        let si = record.struct_index as usize;
        if matching_set.contains(&si) {
            let record_name = db.resolve_string2(record.name_offset);
            let file_name = db.resolve_string(record.file_name_offset);
            let struct_name = db.resolve_string2(db.struct_defs()[si].name_offset);
            paint_records.push((ri, record_name, file_name, struct_name));
        }
    }

    // Sort by file_name for readability
    paint_records.sort_by_key(|&(_, _, f, _)| f);

    for &(ri, rname, fname, sname) in &paint_records {
        println!("  [{}] {} (struct: {}) file: {}", ri, rname, sname, fname);
    }
    println!("Total matching records: {}", paint_records.len());
    println!();

    // --- Step 3: Find Gladius-related records ---
    println!("=== Gladius-related paint/livery records ===");
    let gladius_records: Vec<_> = paint_records
        .iter()
        .filter(|(_, rname, fname, _)| {
            let rn = rname.to_lowercase();
            let fn_ = fname.to_lowercase();
            rn.contains("gladius") || fn_.contains("gladius")
        })
        .collect();

    if gladius_records.is_empty() {
        println!("  (none found by name - searching all records for gladius references...)");
    }
    for &&(ri, rname, fname, sname) in &gladius_records {
        println!("  [{}] {} (struct: {}) file: {}", ri, rname, sname, fname);
    }
    println!();

    // --- Step 4: Also search for ANY record with "gladius" in name/file ---
    println!("=== ALL records with 'gladius' in name or file ===");
    let mut gladius_all: Vec<(&str, &str, &str)> = Vec::new();
    for record in db.records() {
        let record_name = db.resolve_string2(record.name_offset);
        let file_name = db.resolve_string(record.file_name_offset);
        let rn = record_name.to_lowercase();
        let fn_ = file_name.to_lowercase();
        if rn.contains("gladius") || fn_.contains("gladius") {
            let struct_name =
                db.resolve_string2(db.struct_defs()[record.struct_index as usize].name_offset);
            gladius_all.push((record_name, file_name, struct_name));
        }
    }
    gladius_all.sort_by_key(|&(_, f, _)| f);
    gladius_all.dedup();
    for &(rname, fname, sname) in &gladius_all {
        println!("  {} (struct: {}) file: {}", rname, sname, fname);
    }
    println!("Total gladius records: {}", gladius_all.len());
    println!();

    // --- Step 5: Dump JSON for first few interesting paint records ---
    println!("=== Dumping paint-related records as JSON ===");
    // First dump any gladius paint records
    let records_to_dump: Vec<usize> = gladius_records.iter().map(|&&(ri, _, _, _)| ri).collect();

    // If no gladius-specific paint records, dump the first few paint records as samples
    let records_to_dump = if records_to_dump.is_empty() {
        println!("  (No gladius-specific paint records, dumping first samples)");
        paint_records
            .iter()
            .take(10)
            .map(|&(ri, _, _, _)| ri)
            .collect()
    } else {
        records_to_dump
    };

    for ri in &records_to_dump {
        let record = &db.records()[*ri];
        let record_name = db.resolve_string2(record.name_offset);
        let file_name = db.resolve_string(record.file_name_offset);
        println!("\n--- Record: {} (file: {}) ---", record_name, file_name);
        if db.is_main_record(record) {
            match starbreaker_datacore::export::to_json(&db, record) {
                Ok(json) => {
                    let s = String::from_utf8_lossy(&json);
                    // Truncate very long output
                    if s.len() > 5000 {
                        println!("{}", &s[..5000]);
                        println!("... (truncated, {} total bytes)", s.len());
                    } else {
                        println!("{}", s);
                    }
                }
                Err(e) => println!("  Error: {}", e),
            }
        } else {
            println!("  (sub-record, skipping JSON export)");
        }
    }

    // --- Step 6: Search for "VehicleCustomization" or "PaintComponent" struct types ---
    println!("\n=== Struct types containing 'custom', 'visual', 'material', 'component' ===");
    let extra_keywords = [
        "customiz",
        "visual",
        "appearance",
        "materialassign",
        "vehiclecustom",
    ];
    for (i, sd) in db.struct_defs().iter().enumerate() {
        let name = db.resolve_string2(sd.name_offset);
        let lower = name.to_lowercase();
        for kw in &extra_keywords {
            if lower.contains(kw) {
                println!("  [{:>4}] {}", i, name);
                break;
            }
        }
    }
    println!();

    // --- Step 7: Search for enum types related to paint ---
    println!("=== Enum types matching paint/color keywords ===");
    for (i, ed) in db.enum_defs().iter().enumerate() {
        let name = db.resolve_string2(ed.name_offset);
        let lower = name.to_lowercase();
        for kw in &keywords {
            if lower.contains(kw) {
                let vc = ed.value_count;
                println!("  [{:>4}] {} ({} options)", i, name, vc);
                // Print enum options
                let options = db.enum_options(i as i32);
                for opt in options {
                    let opt_name = db.resolve_string2(*opt);
                    println!("         - {}", opt_name);
                }
                break;
            }
        }
    }
}
