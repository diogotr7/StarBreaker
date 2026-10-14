use starbreaker_datacore::{Database, types::Record};
use std::{env, fs, time::Instant};

type ExportFn = fn(&Database, &Record) -> Result<Vec<u8>, starbreaker_datacore::ExportError>;
type WriteFn = fn(&Database, &Record, &mut Vec<u8>) -> Result<(), starbreaker_datacore::ExportError>;

const EXPORTERS: &[(&str, ExportFn, WriteFn)] = &[
    (
        "json",
        starbreaker_datacore::export::to_json,
        write_json,
    ),
    (
        "xml",
        starbreaker_datacore::export::to_xml,
        write_xml,
    ),
    (
        "classic_xml",
        starbreaker_datacore::export::to_classic_xml,
        write_classic_xml,
    ),
    (
        "unp4k_xml",
        starbreaker_datacore::export::to_unp4k_xml,
        write_unp4k_xml,
    ),
];

fn main() {
    let path = env::args()
        .nth(1)
        .expect("Usage: bench_parallel <path.dcb> [format]");
    let format = env::args().nth(2);
    let data = fs::read(&path).expect("failed to read");

    let db = Database::from_bytes(&data).expect("failed to parse");

    let mut ran = false;
    for (name, export, write) in EXPORTERS {
        if format.as_deref().is_some_and(|requested| requested != *name) {
            continue;
        }
        ran = true;
        bench_exporter(&db, name, *export, *write);
    }

    if !ran {
        eprintln!(
            "unknown format '{}'; expected one of: {}",
            format.unwrap(),
            EXPORTERS
                .iter()
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::process::exit(2);
    }
}

fn bench_exporter(db: &Database, name: &str, export: ExportFn, write: WriteFn) {
    println!("{name}:");

    // Single-threaded, new Vec per record
    let start = Instant::now();
    let mut total_bytes_st = 0usize;
    let mut count_st = 0u32;
    let mut errors_st = 0u32;
    for record in db.records() {
        if !db.is_main_record(record) {
            continue;
        }
        match export(db, record) {
            Ok(bytes) => {
                total_bytes_st += bytes.len();
                count_st += 1;
            }
            Err(_) => errors_st += 1,
        }
    }
    let st_time = start.elapsed();
    println!(
        "  Single-threaded (alloc): {:?} ({count_st} records, {errors_st} errors, {:.1} MB)",
        st_time,
        total_bytes_st as f64 / 1_048_576.0
    );

    // Single-threaded, reuse buffer
    let start = Instant::now();
    let mut total_bytes_reuse = 0usize;
    let mut count_reuse = 0u32;
    let mut errors_reuse = 0u32;
    let mut buf: Vec<u8> = Vec::with_capacity(256 * 1024);
    for record in db.records() {
        if !db.is_main_record(record) {
            continue;
        }
        buf.clear();
        match write(db, record, &mut buf) {
            Ok(()) => {
                total_bytes_reuse += buf.len();
                count_reuse += 1;
            }
            Err(_) => errors_reuse += 1,
        }
    }
    let reuse_time = start.elapsed();
    println!(
        "  Single-threaded (reuse): {:?} ({count_reuse} records, {errors_reuse} errors, {:.1} MB)",
        reuse_time,
        total_bytes_reuse as f64 / 1_048_576.0
    );

    // Parallel (CPU only, no I/O)
    {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let start = Instant::now();
        let total_bytes = AtomicUsize::new(0);
        let count = AtomicUsize::new(0);
        let errors = AtomicUsize::new(0);
        db.records().par_iter().for_each(|record| {
            if !db.is_main_record(record) {
                return;
            }
            match export(db, record) {
                Ok(bytes) => {
                    total_bytes.fetch_add(bytes.len(), Ordering::Relaxed);
                    count.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        let par_time = start.elapsed();
        let c = count.load(Ordering::Relaxed);
        let b = total_bytes.load(Ordering::Relaxed);
        let e = errors.load(Ordering::Relaxed);
        println!(
            "  Parallel (rayon):        {:?} ({c} records, {e} errors, {:.1} MB)",
            par_time,
            b as f64 / 1_048_576.0
        );
        println!(
            "  Speedup vs alloc: {:.1}x",
            st_time.as_secs_f64() / par_time.as_secs_f64()
        );
    }
    println!();
}

fn write_unp4k_xml(
    db: &Database,
    record: &Record,
    buf: &mut Vec<u8>,
) -> Result<(), starbreaker_datacore::ExportError> {
    starbreaker_datacore::export::unp4k_xml::write_unp4k_xml(db, record, buf)
}

fn write_json(
    db: &Database,
    record: &Record,
    buf: &mut Vec<u8>,
) -> Result<(), starbreaker_datacore::ExportError> {
    starbreaker_datacore::export::write_json(db, record, buf)
}

fn write_xml(
    db: &Database,
    record: &Record,
    buf: &mut Vec<u8>,
) -> Result<(), starbreaker_datacore::ExportError> {
    starbreaker_datacore::export::write_xml(db, record, buf)
}

fn write_classic_xml(
    db: &Database,
    record: &Record,
    buf: &mut Vec<u8>,
) -> Result<(), starbreaker_datacore::ExportError> {
    starbreaker_datacore::export::write_classic_xml(db, record, buf)
}
