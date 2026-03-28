use rayon::prelude::*;
use starbreaker_datacore::Database;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{env, fs, path::Path, time::Instant};

fn main() {
    let args: Vec<String> = env::args().collect();

    // Usage: dump_json [path.dcb] <output_dir>
    // If only output_dir is given, auto-discover DCB from P4K.
    let (data, output_dir_arg) = if args.len() >= 3 {
        (
            fs::read(&args[1]).expect("failed to read DCB file"),
            args[2].clone(),
        )
    } else if args.len() == 2 {
        let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
        (
            p4k.read_file("Data\\Game2.dcb")
                .expect("failed to read Game2.dcb"),
            args[1].clone(),
        )
    } else {
        eprintln!("Usage: dump_json [path.dcb] <output_dir>");
        std::process::exit(1);
    };

    let start = Instant::now();
    println!("Read file: {:?}", start.elapsed());

    let parse_start = Instant::now();
    let db = Database::from_bytes(&data).expect("failed to parse");
    println!("Parse: {:?}", parse_start.elapsed());
    println!("Records: {}", db.records().len());

    let output_dir = Path::new(&output_dir_arg);
    let export_start = Instant::now();

    let counter = AtomicU32::new(0);
    db.records().par_iter().for_each(|record| {
        if !db.is_main_record(record) {
            return;
        }
        let filename = db.resolve_string(record.file_name_offset);
        let path = output_dir.join(filename).with_extension("json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        match starbreaker_datacore::export::to_json(&db, record) {
            Ok(json) => {
                fs::write(&path, &json).ok();
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
