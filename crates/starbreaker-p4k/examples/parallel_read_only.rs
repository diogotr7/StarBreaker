//! Parallel read+decompress benchmark — discards data, no disk writes.
//!
//! Isolates the P4k read+decompress path from filesystem write costs (NTFS
//! metadata, SSD SLC cache filling) so we can compare just the read-side
//! change between branches.
//!
//! Usage: `cargo run --release --example parallel_read_only -- <substring> [iterations] [mode]`
//!
//! Modes:
//!   shared    — MappedP4k::read (this branch's MappedP4k internals)
//!   perthread — thread_local! File + P4kArchive::read_from_file_at
//!
//! Default: shared, 5 iterations.
//! Picks all entries whose path contains <substring> (case-insensitive),
//! reads+decompresses them in parallel via rayon, discards the data,
//! reports throughput.

use rayon::prelude::*;
use starbreaker_p4k::{MappedP4k, P4kArchive};
use std::cell::RefCell;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

thread_local! {
    static TL_FILE: RefCell<Option<File>> = const { RefCell::new(None) };
}

fn main() {
    let mut args = std::env::args().skip(1);
    let needle = args.next().unwrap_or_else(|| ".xml".to_string());
    let iterations: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let mode = args.next().unwrap_or_else(|| "shared".to_string());

    let p4k_path: PathBuf = starbreaker_common::discover::find_p4k()
        .expect("could not find Data.p4k (set SC_DATA_P4K)")
        .path;
    eprintln!("opening {}", p4k_path.display());
    let p4k = MappedP4k::open(&p4k_path).expect("open p4k");

    let needle_lower = needle.to_ascii_lowercase();
    let entries: Vec<_> = p4k
        .entries()
        .iter()
        .filter(|e| e.name.to_ascii_lowercase().contains(&needle_lower))
        .filter(|e| e.uncompressed_size > 0)
        .cloned()
        .collect();
    eprintln!("matched {} entries", entries.len());
    if entries.is_empty() {
        return;
    }

    let total_uncompressed: u64 = entries.iter().map(|e| e.uncompressed_size).sum();
    eprintln!("total uncompressed: {:.1} MB", total_uncompressed as f64 / 1_048_576.0);
    eprintln!("threads: {}", rayon::current_num_threads());
    eprintln!("mode: {}", mode);

    // Warm-up: stabilize OS page cache for the P4k file.
    eprintln!("warmup pass...");
    let _: u64 = entries
        .par_iter()
        .map(|e| p4k.read(e).map(|d| d.len() as u64).unwrap_or(0))
        .sum();

    eprintln!("\n--- timed runs ---");
    for i in 1..=iterations {
        let bytes = AtomicU64::new(0);
        let errors = AtomicU64::new(0);
        let start = Instant::now();

        match mode.as_str() {
            "shared" => {
                entries.par_iter().for_each(|entry| {
                    match p4k.read(entry) {
                        Ok(data) => {
                            bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                        }
                        Err(_) => {
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
            "perthread" => {
                let path = &p4k_path;
                entries.par_iter().for_each(|entry| {
                    TL_FILE.with(|cell| {
                        let mut slot = cell.borrow_mut();
                        if slot.is_none() {
                            match File::open(path) {
                                Ok(f) => *slot = Some(f),
                                Err(_) => {
                                    errors.fetch_add(1, Ordering::Relaxed);
                                    return;
                                }
                            }
                        }
                        let file = slot.as_ref().expect("just initialized");
                        match P4kArchive::read_from_file_at(file, entry) {
                            Ok(data) => {
                                bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                            }
                            Err(_) => {
                                errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    });
                });
            }
            other => {
                panic!("unknown mode {other:?}, expected shared|perthread");
            }
        }

        let dur = start.elapsed();
        let mb = bytes.load(Ordering::Relaxed) as f64 / 1_048_576.0;
        let mbs = mb / dur.as_secs_f64();
        eprintln!(
            "run {}: {:.3}s | {:.1} MB | {:.1} MB/s | {} errors",
            i,
            dur.as_secs_f64(),
            mb,
            mbs,
            errors.load(Ordering::Relaxed),
        );
    }
}
