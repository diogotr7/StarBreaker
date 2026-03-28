use starbreaker_datacore::Database;
use std::{env, fs, time::Instant};

fn main() {
    let path = env::args().nth(1).expect("Usage: bench_export <path.dcb>");
    let data = fs::read(&path).expect("failed to read");

    let parse_start = Instant::now();
    let db = Database::from_bytes(&data).expect("failed to parse");
    let parse_time = parse_start.elapsed();

    let mut total_bytes = 0usize;
    let mut count = 0u32;

    let export_start = Instant::now();
    for record in db.records() {
        if !db.is_main_record(record) {
            continue;
        }
        if let Ok(json) = starbreaker_datacore::export::to_json(&db, record) {
            total_bytes += json.len();
            count += 1;
        }
    }
    let export_time = export_start.elapsed();

    println!("Parse time:  {:?}", parse_time);
    println!("Export time:  {:?} ({count} records)", export_time);
    println!(
        "Total bytes: {} ({:.1} MB)",
        total_bytes,
        total_bytes as f64 / 1_048_576.0
    );
    println!(
        "Records/sec: {:.0}",
        count as f64 / export_time.as_secs_f64()
    );
    println!(
        "MB/sec:      {:.1}",
        total_bytes as f64 / 1_048_576.0 / export_time.as_secs_f64()
    );
}
