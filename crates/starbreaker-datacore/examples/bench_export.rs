use starbreaker_datacore::{Database, types::Record};
use std::{env, fs, time::Instant};

type ExportFn = fn(&Database, &Record) -> Result<Vec<u8>, starbreaker_datacore::ExportError>;

const EXPORTERS: &[(&str, ExportFn)] = &[
    ("json", starbreaker_datacore::export::to_json),
    ("xml", starbreaker_datacore::export::to_xml),
    ("classic_xml", starbreaker_datacore::export::to_classic_xml),
    ("unp4k_xml", starbreaker_datacore::export::to_unp4k_xml),
];

fn main() {
    let path = env::args().nth(1).expect("Usage: bench_export <path.dcb> [format]");
    let format = env::args().nth(2);
    let data = fs::read(&path).expect("failed to read");

    let parse_start = Instant::now();
    let db = Database::from_bytes(&data).expect("failed to parse");
    let parse_time = parse_start.elapsed();

    println!("Parse time:  {:?}", parse_time);
    println!();

    let mut ran = false;
    for (name, export) in EXPORTERS {
        if format.as_deref().is_some_and(|requested| requested != *name) {
            continue;
        }
        ran = true;
        bench_exporter(&db, name, *export);
    }

    if !ran {
        eprintln!(
            "unknown format '{}'; expected one of: {}",
            format.unwrap(),
            EXPORTERS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::process::exit(2);
    }
}

fn bench_exporter(db: &Database, name: &str, export: ExportFn) {
    let mut total_bytes = 0usize;
    let mut count = 0u32;
    let mut errors = 0u32;

    let export_start = Instant::now();
    for record in db.records() {
        if !db.is_main_record(record) {
            continue;
        }
        match export(db, record) {
            Ok(bytes) => {
                total_bytes += bytes.len();
                count += 1;
            }
            Err(_) => errors += 1,
        }
    }
    let export_time = export_start.elapsed();

    println!("{name}:");
    println!("  Export time: {:?} ({count} records, {errors} errors)", export_time);
    println!(
        "  Total bytes: {} ({:.1} MB)",
        total_bytes,
        total_bytes as f64 / 1_048_576.0
    );
    println!(
        "  Records/sec: {:.0}",
        count as f64 / export_time.as_secs_f64()
    );
    println!(
        "  MB/sec:      {:.1}",
        total_bytes as f64 / 1_048_576.0 / export_time.as_secs_f64()
    );
    println!();
}
