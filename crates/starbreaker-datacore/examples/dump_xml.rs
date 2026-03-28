use rayon::prelude::*;
use starbreaker_datacore::Database;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{env, fs, path::Path, time::Instant};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: dump_xml <path.dcb> <output_dir>");
        std::process::exit(1);
    }

    let start = Instant::now();
    let data = fs::read(&args[1]).expect("failed to read");
    println!("Read file: {:?}", start.elapsed());

    let parse_start = Instant::now();
    let db = Database::from_bytes(&data).expect("failed to parse");
    println!("Parse: {:?}", parse_start.elapsed());
    println!("Records: {}", db.records().len());

    let output_dir = Path::new(&args[2]);
    let export_start = Instant::now();

    let counter = AtomicU32::new(0);
    db.records().par_iter().for_each(|record| {
        if !db.is_main_record(record) {
            return;
        }
        let filename = db.resolve_string(record.file_name_offset);
        let path = output_dir.join(filename).with_extension("xml");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        match starbreaker_datacore::export::to_xml(&db, record) {
            Ok(xml) => {
                fs::write(&path, &xml).ok();
            }
            Err(e) => {
                eprintln!("Error exporting {filename}: {e}");
            }
        }
        counter.fetch_add(1, Ordering::Relaxed);
    });
    let count = counter.load(Ordering::Relaxed);

    println!("Exported {count} records in {:?}", export_start.elapsed());
    println!("Total: {:?}", start.elapsed());
}
