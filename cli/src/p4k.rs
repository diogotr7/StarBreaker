use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Subcommand;
use starbreaker_p4k::{P4kArchive, P4kEntry};

use crate::common::{load_p4k, matches_filter};
use crate::error::Result;

#[derive(Subcommand)]
pub enum P4kCommand {
    /// Extract files from a P4k archive
    Extract {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
        /// Glob pattern filter
        #[arg(long, group = "filter_mode")]
        filter: Option<String>,
        /// Regex pattern filter
        #[arg(long, group = "filter_mode")]
        regex: Option<String>,
        /// Max threads (1 = sequential, default = all cores)
        #[arg(long)]
        max_threads: Option<usize>,
    },
    /// List files in a P4k archive
    List {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Glob pattern filter
        #[arg(long, group = "filter_mode")]
        filter: Option<String>,
        /// Regex pattern filter
        #[arg(long, group = "filter_mode")]
        regex: Option<String>,
    },
}

impl P4kCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Extract {
                p4k,
                output,
                filter,
                regex,
                max_threads,
            } => extract(p4k, output, filter, regex, max_threads),
            Self::List { p4k, filter, regex } => list(p4k, filter, regex),
        }
    }
}

fn extract(
    p4k_path: Option<PathBuf>,
    output: PathBuf,
    filter: Option<String>,
    regex_pattern: Option<String>,
    max_threads: Option<usize>,
) -> Result<()> {
    let p4k = load_p4k(p4k_path.as_deref())?;
    let p4k_file_path = p4k.path().to_path_buf();

    let re = regex_pattern
        .as_deref()
        .map(regex::Regex::new)
        .transpose()?;

    let mut entries: Vec<P4kEntry> = p4k
        .entries()
        .iter()
        .filter(|e| matches_filter(&e.name, filter.as_deref(), re.as_ref()))
        .filter(|e| e.uncompressed_size > 0)
        .cloned()
        .collect();

    drop(p4k);

    eprintln!("Extracting {} files...", entries.len());

    eprint!("Pre-creating directories... ");
    let dirs = P4kArchive::unique_directories(&entries);
    for dir in &dirs {
        let dir_path = output.join(dir.replace('\\', "/"));
        let _ = std::fs::create_dir_all(&dir_path);
    }
    eprintln!("{} directories created.", dirs.len());

    // Sort by offset like C# does.
    entries.sort_by_key(|e| e.offset);

    let total_bytes = AtomicU64::new(0);
    let files_done = AtomicU64::new(0);
    let error_count = AtomicU64::new(0);
    let total_files = entries.len() as u64;
    let start = std::time::Instant::now();
    let report_interval = 10_000u64;

    let num_threads = max_threads.unwrap_or(0); // 0 = rayon default (all cores)
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()?;

    eprintln!(
        "[START] {} threads",
        if num_threads == 0 { "all cores".to_string() } else { num_threads.to_string() }
    );

    // Thread-local P4k file handles — one per rayon worker.
    thread_local! {
        static P4K_FILE: std::cell::RefCell<Option<File>> = const { std::cell::RefCell::new(None) };
    }

    use rayon::prelude::*;

    pool.install(|| entries.par_iter().for_each(|entry| {
        let result = P4K_FILE.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                match File::open(&p4k_file_path) {
                    Ok(f) => *slot = Some(f),
                    Err(e) => return Err(starbreaker_p4k::P4kError::Io(e)),
                }
            }
            let file = slot.as_mut().ok_or_else(|| {
                starbreaker_p4k::P4kError::Io(std::io::Error::other("P4k file handle missing"))
            })?;
            P4kArchive::read_from_file(file, entry)
        });

        match result {
            Ok(data) => {
                total_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                let out_path = output.join(entry.name.replace('\\', "/"));
                if let Err(e) = write_file(&out_path, &data) {
                    error_count.fetch_add(1, Ordering::Relaxed);
                    eprintln!("\n[ERR] Write {}: {e}", entry.name);
                }
            }
            Err(e) => {
                error_count.fetch_add(1, Ordering::Relaxed);
                eprintln!("\n[ERR] Read {}: {e}", entry.name);
            }
        }

        let done = files_done.fetch_add(1, Ordering::Relaxed) + 1;
        if done % report_interval == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let mb = total_bytes.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);
            let errors = error_count.load(Ordering::Relaxed);
            eprintln!(
                "[PROGRESS] {done}/{total_files} files | {:.1}s | {:.0} MB | {:.0} MB/s | {errors} errors",
                elapsed, mb, mb / elapsed
            );
        }
    }));

    let elapsed = start.elapsed();
    let total_mb = total_bytes.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);
    let secs = elapsed.as_secs_f64();
    let done = files_done.load(Ordering::Relaxed);
    let errors = error_count.load(Ordering::Relaxed);

    eprintln!("[DONE] Extracted {done}/{total_files} files in {:.1}s", secs);
    eprintln!(
        "[DONE] Total: {:.1} MB | Avg throughput: {:.1} MB/s",
        total_mb,
        total_mb / secs
    );
    if errors > 0 {
        eprintln!("[DONE] {errors} errors encountered");
    }

    Ok(())
}

fn write_file(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let file = File::create(path)?;
    if data.len() > 65536 {
        let mut writer = BufWriter::with_capacity(data.len().min(1 << 20), file);
        writer.write_all(data)?;
        writer.flush()?;
    } else {
        let mut file = file;
        file.write_all(data)?;
    }
    Ok(())
}

fn list(
    p4k_path: Option<PathBuf>,
    filter: Option<String>,
    regex_pattern: Option<String>,
) -> Result<()> {
    let p4k = load_p4k(p4k_path.as_deref())?;
    let re = regex_pattern
        .as_deref()
        .map(regex::Regex::new)
        .transpose()?;

    for entry in p4k.entries() {
        if matches_filter(&entry.name, filter.as_deref(), re.as_ref()) {
            println!("{}\t{}", entry.name, entry.uncompressed_size);
        }
    }
    Ok(())
}
