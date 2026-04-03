use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::{Subcommand, ValueEnum};
use starbreaker_p4k::{MappedP4k, P4kArchive, P4kEntry};

use crate::common::{load_p4k, matches_filter};
use crate::error::Result;

const FILTER_HELP: &str = "\
Glob pattern to filter entries by path.

Paths inside the P4k look like Data/Objects/ships/aurora/texture.dds.

  *   matches within a single directory (no slashes)
  **  matches across any number of directories

Examples:
  **/*.xml                        all .xml files anywhere
  **/*.dds                        all .dds files anywhere
  Data/Objects/ships/aurora/*.dds  .dds files directly in that folder
  Data/Objects/ships/aurora/**     everything under aurora/ (all depths)
  **/ships/**                     everything under any ships/ directory
  **/*aurora*                     any file with 'aurora' in the name";

const CONVERT_HELP: &str = "\
Convert files during extraction (repeatable).

Available converters:
  cryxml     CryXmlB binary → readable XML (e.g. .mtl, .chrparams, .xml)
  dds-png    DDS textures → PNG (merges split mips, drops .dds.N siblings)
  dds-merge  DDS split mips → single merged DDS (keeps DDS format)
  all        enable all converters

If both dds-png and dds-merge are given, dds-png wins.

Examples:
  --convert cryxml                  only decode binary XML
  --convert dds-png                 only convert textures to PNG
  --convert cryxml --convert dds-png  decode XML + convert textures
  --convert all                     everything

Typical workflows:
  # Extract all Gladius files, converting XML and textures:
  starbreaker p4k extract -o out --filter '**/AEGS/Gladius/**' --convert all

  # Extract only .mtl material files as readable XML:
  starbreaker p4k extract -o out --filter '**/*.mtl' --convert cryxml

  # Get merged DDS textures (for modding tools that need DDS):
  starbreaker p4k extract -o out --filter '**/*.dds' --convert dds-merge";

#[derive(Clone, ValueEnum)]
pub(crate) enum Converter {
    /// CryXmlB → readable XML
    Cryxml,
    /// DDS → PNG (merges split mips)
    DdsPng,
    /// DDS split mips → single merged DDS
    DdsMerge,
    /// Enable all converters
    All,
}

/// Resolved set of active converters.
#[derive(Default)]
struct ConvertFlags {
    cryxml: bool,
    dds_png: bool,
    dds_merge: bool,
}

impl ConvertFlags {
    fn from_list(converters: &[Converter]) -> Self {
        let mut flags = Self::default();
        for c in converters {
            match c {
                Converter::Cryxml => flags.cryxml = true,
                Converter::DdsPng => flags.dds_png = true,
                Converter::DdsMerge => flags.dds_merge = true,
                Converter::All => {
                    flags.cryxml = true;
                    flags.dds_png = true;
                    flags.dds_merge = true;
                }
            }
        }
        // dds-png wins over dds-merge if both specified
        if flags.dds_png {
            flags.dds_merge = false;
        }
        flags
    }

    fn any(&self) -> bool {
        self.cryxml || self.dds_png || self.dds_merge
    }

    fn any_dds(&self) -> bool {
        self.dds_png || self.dds_merge
    }

    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.cryxml { parts.push("CryXML→XML"); }
        if self.dds_png { parts.push("DDS→PNG"); }
        if self.dds_merge { parts.push("DDS→merged DDS"); }
        parts.join(", ")
    }
}

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
        /// Glob pattern filter (see examples with --help)
        #[arg(long, group = "filter_mode", long_help = FILTER_HELP)]
        filter: Option<String>,
        /// Regex pattern filter
        #[arg(long, group = "filter_mode")]
        regex: Option<String>,
        /// Max threads (1 = sequential, default = all cores)
        #[arg(long)]
        max_threads: Option<usize>,
        /// Convert files during extraction (repeatable, see --help)
        #[arg(long, value_enum, long_help = CONVERT_HELP)]
        convert: Vec<Converter>,
    },
    /// List files in a P4k archive
    List {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Glob pattern filter (see examples with --help)
        #[arg(long, group = "filter_mode", long_help = FILTER_HELP)]
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
                convert,
            } => {
                let flags = ConvertFlags::from_list(&convert);
                extract(p4k, output, filter, regex, max_threads, flags)
            }
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
    conv: ConvertFlags,
) -> Result<()> {
    let p4k = load_p4k(p4k_path.as_deref())?;
    let p4k_file_path = p4k.path().to_path_buf();

    let re = regex_pattern
        .as_deref()
        .map(regex::Regex::new)
        .transpose()?;

    let has_filter = filter.is_some() || re.is_some();

    let mut entries: Vec<P4kEntry> = p4k
        .entries()
        .iter()
        .filter(|e| matches_filter(&e.name, filter.as_deref(), re.as_ref()))
        .filter(|e| e.uncompressed_size > 0)
        .cloned()
        .collect();

    if entries.is_empty() && has_filter {
        let pat = filter.as_deref().unwrap_or("<regex>");
        eprintln!("No files matched filter: {pat}");
        suggest_filter(filter.as_deref(), p4k.entries());
        return Ok(());
    }

    // When converting DDS, skip split sibling files (.dds.1, .dds.2, .dds.a, etc.)
    // — they'll be merged into the base .dds during conversion.
    if conv.any_dds() {
        entries.retain(|e| !is_dds_sibling(&e.name));
    }

    // Keep p4k alive when DDS conversion needs sibling reads.
    let p4k_ref: Option<MappedP4k> = if conv.any_dds() { Some(p4k) } else { drop(p4k); None };

    eprintln!("Extracting {} files...", entries.len());
    if conv.any() {
        eprintln!("[CONVERT] {}", conv.summary());
    }

    eprint!("Pre-creating directories... ");
    let dirs = P4kArchive::unique_directories(&entries);
    for dir in &dirs {
        let dir_path = output.join(dir.replace('\\', "/"));
        if let Err(e) = std::fs::create_dir_all(&dir_path) {
            eprintln!("[ERR] create dir {}: {e}", dir_path.display());
        }
    }
    eprintln!("{} directories created.", dirs.len());

    // Sort by offset like C# does.
    entries.sort_by_key(|e| e.offset);

    let total_bytes = AtomicU64::new(0);
    let files_done = AtomicU64::new(0);
    let error_count = AtomicU64::new(0);
    let converted_count = AtomicU64::new(0);
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
        let result = if conv.any_dds() && is_base_dds(&entry.name) {
            // DDS: read via MappedP4k so we can access siblings for split merging.
            p4k_ref.as_ref().unwrap().read(entry)
        } else {
            P4K_FILE.with(|cell| {
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
            })
        };

        match result {
            Ok(data) => {
                total_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);

                if conv.any() {
                    if !convert_and_write(
                        entry, &data, &output, &conv, p4k_ref.as_ref(),
                        &error_count, &converted_count,
                    ) {
                        // Not convertible — write raw.
                        let out_path = output.join(entry.name.replace('\\', "/"));
                        if let Err(e) = write_file(&out_path, &data) {
                            error_count.fetch_add(1, Ordering::Relaxed);
                            eprintln!("\n[ERR] Write {}: {e}", entry.name);
                        }
                    }
                } else {
                    let out_path = output.join(entry.name.replace('\\', "/"));
                    if let Err(e) = write_file(&out_path, &data) {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        eprintln!("\n[ERR] Write {}: {e}", entry.name);
                    }
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
    let converted = converted_count.load(Ordering::Relaxed);

    eprintln!("[DONE] Extracted {done}/{total_files} files in {:.1}s", secs);
    eprintln!(
        "[DONE] Total: {:.1} MB | Avg throughput: {:.1} MB/s",
        total_mb,
        total_mb / secs
    );
    if converted > 0 {
        eprintln!("[DONE] {converted} files converted");
    }
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

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Returns true if this P4k entry name is a DDS split sibling (not the base file).
/// Sibling suffixes: `.dds.1` through `.dds.8`, `.dds.a`, `.dds.1a` through `.dds.8a`.
fn is_dds_sibling(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // Match `.dds.` followed by a digit, optionally followed by `a`
    // or `.dds.a` (alpha-only sibling).
    if let Some(pos) = lower.rfind(".dds.") {
        let suffix = &lower[pos + 5..]; // after ".dds."
        matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8"
            | "1a" | "2a" | "3a" | "4a" | "5a" | "6a" | "7a" | "8a" | "a")
    } else {
        false
    }
}

/// Returns true if the entry is a base `.dds` file (not a sibling).
fn is_base_dds(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".dds")
}

/// P4k-backed sibling reader for DDS split file merging.
struct P4kSiblingReader<'a> {
    p4k: &'a MappedP4k,
    base_path: String,
}

impl starbreaker_dds::ReadSibling for P4kSiblingReader<'_> {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let path = format!("{}{suffix}", self.base_path);
        self.p4k
            .entry_case_insensitive(&path)
            .and_then(|entry| self.p4k.read(entry).ok())
    }
}

/// Try to convert `data` in place. Returns `true` if a conversion was performed
/// (even if it failed — errors are counted but the file is skipped).
fn convert_and_write(
    entry: &P4kEntry,
    data: &[u8],
    output: &std::path::Path,
    conv: &ConvertFlags,
    p4k: Option<&MappedP4k>,
    error_count: &AtomicU64,
    converted_count: &AtomicU64,
) -> bool {
    // CryXML → XML
    if conv.cryxml && starbreaker_cryxml::is_cryxmlb(data) {
        match starbreaker_cryxml::from_bytes(data) {
            Ok(cryxml) => {
                let xml = format!("{cryxml}");
                let out_path = output.join(entry.name.replace('\\', "/"));
                if let Err(e) = write_file(&out_path, xml.as_bytes()) {
                    error_count.fetch_add(1, Ordering::Relaxed);
                    eprintln!("\n[ERR] Write (cryxml→xml) {}: {e}", entry.name);
                } else {
                    converted_count.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(e) => {
                error_count.fetch_add(1, Ordering::Relaxed);
                eprintln!("\n[ERR] Convert (cryxml→xml) {}: {e}", entry.name);
            }
        }
        return true;
    }

    // DDS → PNG
    if conv.dds_png && is_base_dds(&entry.name) {
        let dds = parse_dds_with_siblings(data, &entry.name, p4k);
        match dds {
            Ok(dds) => {
                let rel = entry.name.replace('\\', "/");
                let png_rel = format!("{}.png", &rel[..rel.len() - 4]);
                let out_path = output.join(&png_rel);
                match dds.save_png(&out_path, 0) {
                    Ok(()) => {
                        converted_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        eprintln!("\n[ERR] Convert (dds→png) {}: {e}", entry.name);
                    }
                }
            }
            Err(e) => {
                error_count.fetch_add(1, Ordering::Relaxed);
                eprintln!("\n[ERR] Parse DDS {}: {e}", entry.name);
            }
        }
        return true;
    }

    // DDS → merged DDS
    if conv.dds_merge && is_base_dds(&entry.name) {
        let dds = parse_dds_with_siblings(data, &entry.name, p4k);
        match dds {
            Ok(dds) => {
                let out_path = output.join(entry.name.replace('\\', "/"));
                if let Err(e) = write_file(&out_path, &dds.to_dds()) {
                    error_count.fetch_add(1, Ordering::Relaxed);
                    eprintln!("\n[ERR] Write (dds merge) {}: {e}", entry.name);
                } else {
                    converted_count.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(e) => {
                error_count.fetch_add(1, Ordering::Relaxed);
                eprintln!("\n[ERR] Parse DDS {}: {e}", entry.name);
            }
        }
        return true;
    }

    false
}

fn parse_dds_with_siblings(
    data: &[u8],
    name: &str,
    p4k: Option<&MappedP4k>,
) -> std::result::Result<starbreaker_dds::DdsFile, starbreaker_dds::DdsError> {
    if let Some(p4k) = p4k {
        let reader = P4kSiblingReader {
            p4k,
            base_path: name.to_string(),
        };
        starbreaker_dds::DdsFile::from_split(data, &reader)
    } else {
        starbreaker_dds::DdsFile::from_bytes(data)
    }
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

    let has_filter = filter.is_some() || re.is_some();
    let mut count = 0u64;

    for entry in p4k.entries() {
        if matches_filter(&entry.name, filter.as_deref(), re.as_ref()) {
            println!("{}\t{}", entry.name, entry.uncompressed_size);
            count += 1;
        }
    }

    if count == 0 && has_filter {
        let pat = filter.as_deref().unwrap_or("<regex>");
        eprintln!("No files matched filter: {pat}");
        suggest_filter(filter.as_deref(), p4k.entries());
    }

    Ok(())
}

/// Print helpful suggestions when a glob filter matched nothing.
fn suggest_filter(filter: Option<&str>, entries: &[P4kEntry]) {
    let Some(pattern) = filter else { return };

    // If the pattern looks like a bare extension glob (e.g. "*.xml"),
    // suggest the ** version.
    if pattern.starts_with("*.") && !pattern.contains('/') && !pattern.contains('\\') {
        eprintln!();
        eprintln!("Hint: * only matches within a single directory (no slashes).");
        eprintln!("      To match across directories, use **/{pattern}");
        return;
    }

    // If the pattern ends with dir/*.ext, they may have wanted dir/**/*.ext.
    let norm = pattern.replace('\\', "/");
    if let Some(pos) = norm.rfind('/') {
        let tail = &norm[pos + 1..];
        if tail.starts_with("*.") && !norm.contains("**") {
            let dir = &norm[..pos];
            eprintln!();
            eprintln!("Hint: {dir}/*.ext only matches files directly in {dir}/.");
            eprintln!("      To include subdirectories, use {dir}/**/{tail}");
            eprintln!("      To match everything under it,  use {dir}/**");
            return;
        }
    }

    // Generic fallback — show sample paths and examples.
    eprintln!();
    eprintln!("Hint: paths inside the P4k look like:");
    let mut shown = 0;
    for entry in entries.iter().take(20_000) {
        if shown >= 3 {
            break;
        }
        let depth = entry.name.matches('\\').count();
        if depth >= 2 {
            eprintln!("  {}", entry.name.replace('\\', "/"));
            shown += 1;
        }
    }
    eprintln!();
    eprintln!("Quick reference:");
    eprintln!("  *   matches within one directory    *.dds = files in the root only");
    eprintln!("  **  matches across directories      **/*.dds = .dds files anywhere");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  --filter '**/*.xml'                       all .xml files");
    eprintln!("  --filter 'Data/Objects/ships/aurora/*.dds' .dds directly in aurora/");
    eprintln!("  --filter 'Data/Objects/ships/aurora/**'    everything under aurora/");
}
