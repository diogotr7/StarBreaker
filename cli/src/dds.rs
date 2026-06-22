use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use starbreaker_dds::{AlphaMipFormat, AlphaMipLayout, DdsFile};
use starbreaker_dds::sibling::FsSiblingReader;

use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum DdsCommand {
    /// Show DDS file metadata (format, dimensions, mip levels)
    Info {
        /// Input .dds file (filesystem path or P4k path like "Data/...")
        input: String,
        /// Path to Data.p4k (for P4k paths)
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Decode a DDS from P4k to PNG
    Decode {
        /// DDS path in P4k
        input: String,
        /// Output PNG path [default: <input>.png]
        output: Option<PathBuf>,
        /// Mip level to decode
        #[arg(long, default_value = "0")]
        mip: usize,
        /// Decode alpha/smoothness channel instead of RGB normals
        #[arg(long)]
        alpha: bool,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Merge split DDS mipmaps into a single file
    Merge {
        /// Input base .dds file
        input: PathBuf,
        /// Output .dds file [default: <input>.merged.dds]
        output: Option<PathBuf>,
    },
    /// Batch merge split DDS files
    MergeAll {
        /// Input directory
        #[arg(short, long)]
        input: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Decode DDS to PNG
    ToPng {
        /// Input .dds file
        input: PathBuf,
        /// Output .png file [default: <input>.png]
        output: Option<PathBuf>,
    },
    /// Batch decode DDS to PNG
    ToPngAll {
        /// Input directory
        #[arg(short, long)]
        input: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
        /// Glob filter for filenames
        #[arg(long, default_value = "*.dds")]
        filter: String,
    },
    /// Scan DDNA DDS textures in a P4k and report alpha/smoothness coverage
    ScanDdna {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Stop after this many DDNA base DDS files
        #[arg(long)]
        limit: Option<usize>,
        /// Decode alpha mips while scanning, not just parse layout metadata
        #[arg(long)]
        decode_alpha: bool,
        /// Number of sample paths printed for each bucket
        #[arg(long, default_value = "5")]
        sample_limit: usize,
    },
}

impl DdsCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Info { input, p4k } => info(&input, p4k.as_deref()),
            Self::Decode { input, output, mip, alpha, p4k } => {
                let output = output.unwrap_or_else(|| {
                    let stem = input.rsplit(['/', '\\']).next().unwrap_or("output");
                    PathBuf::from(format!("{stem}.png"))
                });
                decode_p4k(&input, &output, mip, alpha, p4k.as_deref())
            }
            Self::Merge { input, output } => merge(input, output),
            Self::MergeAll { input, output } => merge_all(input, output),
            Self::ToPng { input, output } => to_png(input, output),
            Self::ToPngAll {
                input,
                output,
                filter,
            } => to_png_all(input, output, filter),
            Self::ScanDdna {
                p4k,
                limit,
                decode_alpha,
                sample_limit,
            } => scan_ddna(p4k.as_deref(), limit, decode_alpha, sample_limit),
        }
    }
}

fn info(input: &str, p4k_path: Option<&Path>) -> Result<()> {
    let dds = if Path::new(input).exists() {
        // Filesystem path
        let data = std::fs::read(input)
            .map_err(|e| CliError::IoPath { source: e, path: input.to_string() })?;
        let reader = FsSiblingReader::new(input);
        DdsFile::from_split(&data, &reader)
            .or_else(|_| DdsFile::headers_only(&data))?
    } else {
        // Try P4k path
        let p4k = crate::common::load_p4k(p4k_path)?;
        let with_prefix = if input.starts_with("Data/") || input.starts_with("Data\\") {
            input.to_string()
        } else {
            format!("Data/{input}")
        };
        let normalized = with_prefix.replace('/', "\\");
        let entry = p4k
            .entry_case_insensitive(&normalized)
            .ok_or_else(|| CliError::NotFound(format!("not found in P4k: {normalized}")))?;
        let data = p4k.read(entry)?;
        let p4k_reader = P4kDdsSiblingReader {
            p4k: &p4k,
            base_path: normalized,
        };
        // Try split merge first, fall back to header-only parse for unsupported formats
        DdsFile::from_split(&data, &p4k_reader)
            .or_else(|_| DdsFile::headers_only(&data))?
    };

    // Format
    let format = starbreaker_dds::resolve_format(
        &dds.header.pixel_format,
        dds.dxt10_header.as_ref(),
    );
    let format_str = match &format {
        Ok(f) => format!("{f:?}"),
        Err(_) => {
            if let Some(ref dx10) = dds.dxt10_header {
                let dxgi_fmt = { dx10.dxgi_format };
                format!("DXGI {dxgi_fmt}")
            } else {
                let cc = dds.header.pixel_format.four_cc;
                let cc_str = String::from_utf8_lossy(&cc);
                format!("FourCC '{cc_str}'")
            }
        }
    };

    let (w, h) = (dds.header.width, dds.header.height);
    let mip_count_header = std::cmp::max(1, dds.header.mipmap_count) as usize;
    let mip_count_actual = dds.mip_data.len();
    let cubemap = dds.is_cubemap();

    println!("Format:     {format_str}");
    println!("Dimensions: {w} x {h}");
    println!("Cubemap:    {cubemap}");
    println!("Mip levels: {mip_count_actual} present (header declares {mip_count_header})");
    if !dds.alpha_mip_data.is_empty() {
        println!("Alpha mips: {}", dds.alpha_mip_data.len());
    }
    println!();
    println!("{:<6} {:>10} {:>10} {:>12}", "Mip", "Width", "Height", "Size");
    println!("{}", "-".repeat(42));
    for i in 0..mip_count_actual {
        let (mw, mh) = dds.dimensions(i);
        let size = dds.mip_data[i].len();
        let size_str = if size >= 1024 * 1024 {
            format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
        } else if size >= 1024 {
            format!("{:.1} KiB", size as f64 / 1024.0)
        } else {
            format!("{size} B")
        };
        println!("{:<6} {:>10} {:>10} {:>12}", i, mw, mh, size_str);
    }

    if !dds.alpha_mip_data.is_empty() {
        println!();
        println!(
            "{:<6} {:>10} {:>10} {:>12} {:>14} {:>18} {:>8} {:>8} {:>8}",
            "Alpha", "Width", "Height", "Size", "Format", "Layout", "Min", "Max", "Mean"
        );
        println!("{}", "-".repeat(105));
        for i in 0..dds.alpha_mip_data.len() {
            let (mw, mh) = dds.dimensions(i);
            let size = dds.alpha_mip_data[i].len();
            let size_str = if size >= 1024 * 1024 {
                format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
            } else if size >= 1024 {
                format!("{:.1} KiB", size as f64 / 1024.0)
            } else {
                format!("{size} B")
            };
            let format = dds
                .alpha_mip_format_for_mip(i)
                .map(alpha_mip_format_name)
                .unwrap_or("unknown");
            let layout = dds
                .alpha_mip_layout_for_mip(i)
                .map(alpha_mip_layout_name)
                .unwrap_or("unknown");
            let stats = dds
                .decode_alpha_mip(i)
                .ok()
                .and_then(|smoothness| byte_statistics(&smoothness));
            let (min, max, mean) = stats
                .map(|(min, max, mean)| {
                    (min.to_string(), max.to_string(), mean.to_string())
                })
                .unwrap_or_else(|| {
                    (
                        "decode_error".to_string(),
                        "decode_error".to_string(),
                        "decode_error".to_string(),
                    )
                });
            println!(
                "{:<6} {:>10} {:>10} {:>12} {:>14} {:>18} {:>8} {:>8} {:>8}",
                i, mw, mh, size_str, format, layout, min, max, mean
            );
        }
    }

    Ok(())
}

struct P4kDdsSiblingReader<'a> {
    p4k: &'a starbreaker_p4k::MappedP4k,
    base_path: String,
}

impl starbreaker_dds::ReadSibling for P4kDdsSiblingReader<'_> {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let path = format!("{}{suffix}", self.base_path);
        self.p4k
            .entry_case_insensitive(&path)
            .and_then(|entry| self.p4k.read(entry).ok())
    }
}

fn decode_p4k(input: &str, output: &Path, mip: usize, alpha: bool, p4k_path: Option<&Path>) -> Result<()> {
    let p4k = crate::common::load_p4k(p4k_path)?;
    let with_prefix = if input.starts_with("Data/") || input.starts_with("Data\\") {
        input.to_string()
    } else {
        format!("Data/{input}")
    };
    let normalized = with_prefix.replace('/', "\\");
    let entry = p4k
        .entry_case_insensitive(&normalized)
        .ok_or_else(|| CliError::NotFound(format!("not found in P4k: {normalized}")))?;
    let data = p4k.read(entry)?;
    let reader = P4kDdsSiblingReader {
        p4k: &p4k,
        base_path: normalized,
    };
    let dds = DdsFile::from_split(&data, &reader)?;

    if alpha {
        // Decode alpha/smoothness from sibling mips
        if !dds.has_alpha_mips() {
            return Err(CliError::NotFound("no alpha mips found (no .Xa sibling files)".into()));
        }
        let mip = mip.min(dds.alpha_mip_data.len().saturating_sub(1));
        let (w, h) = dds.dimensions(mip);
        eprintln!("Decoding alpha mip {mip}: {w}x{h} ({} alpha mips available)", dds.alpha_mip_data.len());
        let smoothness = dds.decode_alpha_mip(mip)?;
        // Write as grayscale PNG
        let img = image::GrayImage::from_raw(w, h, smoothness)
            .ok_or_else(|| CliError::InvalidInput("failed to create grayscale image".into()))?;
        img.save(output)?;
    } else {
        let mip = mip.min(dds.mip_count().saturating_sub(1));
        let (w, h) = dds.dimensions(mip);
        eprintln!("Decoding mip {mip}: {w}x{h}");
        dds.save_png(output, mip)?;
    }

    eprintln!("Written to {}", output.display());
    Ok(())
}

fn merge(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let data = std::fs::read(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let reader = FsSiblingReader::new(&input);
    let dds = DdsFile::from_split(&data, &reader)?;
    let merged = dds.to_dds();
    let output = output.unwrap_or_else(|| input.with_extension("merged.dds"));
    std::fs::write(&output, &merged)
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn merge_all(input: PathBuf, output: PathBuf) -> Result<()> {
    let files = collect_base_dds_files(&input)?;
    if files.is_empty() {
        return Err(CliError::NotFound("no base .dds files found".into()));
    }
    eprintln!("Merging {} files...", files.len());
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len} ({elapsed}, ETA {eta})")?,
    );

    files.par_iter().for_each(|file| {
        let rel = file.strip_prefix(&input).unwrap_or(file);
        let out_path = output.join(rel);
        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("[ERR] create dir {}: {e}", parent.display());
            }
        }
        let result = (|| -> Result<()> {
            let data = std::fs::read(file)?;
            let reader = FsSiblingReader::new(file);
            let dds = DdsFile::from_split(&data, &reader)?;
            std::fs::write(&out_path, dds.to_dds())?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Error merging {}: {e}", file.display());
        }
        pb.inc(1);
    });
    pb.finish_and_clear();
    eprintln!("Done.");
    Ok(())
}

fn to_png(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let data = std::fs::read(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let reader = FsSiblingReader::new(&input);
    let dds = DdsFile::from_split(&data, &reader)?;
    let output = output.unwrap_or_else(|| input.with_extension("png"));
    dds.save_png(&output, 0)?;
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn to_png_all(input: PathBuf, output: PathBuf, filter: String) -> Result<()> {
    let files: Vec<_> = collect_base_dds_files(&input)?
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| glob_match::glob_match(&filter, n))
                .unwrap_or(false)
        })
        .collect();

    if files.is_empty() {
        return Err(CliError::NotFound("no matching .dds files found".into()));
    }
    eprintln!("Converting {} files to PNG...", files.len());
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len} ({elapsed}, ETA {eta})")?,
    );

    files.par_iter().for_each(|file| {
        let rel = file.strip_prefix(&input).unwrap_or(file);
        let out_path = output.join(rel).with_extension("png");
        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("[ERR] create dir {}: {e}", parent.display());
            }
        }
        let result = (|| -> Result<()> {
            let data = std::fs::read(file)?;
            let reader = FsSiblingReader::new(file);
            let dds = DdsFile::from_split(&data, &reader)?;
            dds.save_png(&out_path, 0)?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Error converting {}: {e}", file.display());
        }
        pb.inc(1);
    });
    pb.finish_and_clear();
    eprintln!("Done.");
    Ok(())
}

fn scan_ddna(
    p4k_path: Option<&Path>,
    limit: Option<usize>,
    decode_alpha: bool,
    sample_limit: usize,
) -> Result<()> {
    let p4k = crate::common::load_p4k(p4k_path)?;
    let mut candidates: Vec<_> = p4k
        .entries()
        .iter()
        .filter(|entry| is_ddna_base_dds_path(&entry.name))
        .collect();
    candidates.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    if let Some(limit) = limit {
        candidates.truncate(limit);
    }

    eprintln!(
        "Scanning {} DDNA base DDS files{}...",
        candidates.len(),
        if decode_alpha { " with alpha decode" } else { "" }
    );

    let pb = ProgressBar::new(candidates.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len} ({elapsed}, ETA {eta})")?,
    );
    let results: Vec<_> = candidates
        .par_iter()
        .map(|entry| {
            let result = scan_ddna_entry(&p4k, entry, decode_alpha);
            pb.inc(1);
            result
        })
        .collect();
    pb.finish_and_clear();

    let mut stats = DdnaScanStats::default();
    for result in &results {
        stats.record(result, sample_limit);
    }

    println!("DDNA base DDS files: {}", stats.assets_total);
    println!("Parsed DDS assets:   {}", stats.assets_parsed);
    println!("With alpha mips:     {}", stats.assets_with_alpha);
    println!("Missing alpha mips:  {}", stats.assets_missing_alpha);
    println!("Alpha mips parsed:   {}", stats.alpha_mips_total);
    if decode_alpha {
        println!("Alpha mips decoded:  {}", stats.alpha_mips_decoded);
        println!("Decode failures:     {}", stats.assets_decode_failed);
    }

    print_count_table("Parse failures", &stats.parse_failures);
    print_count_table("Alpha format/layout mips", &stats.alpha_format_layouts);
    print_count_table("Alpha format/layout assets", &stats.asset_format_layouts);

    if !stats.samples.is_empty() {
        println!();
        println!("Samples:");
        for (bucket, paths) in &stats.samples {
            println!("  {bucket}:");
            for path in paths {
                println!("    {path}");
            }
        }
    }

    Ok(())
}

#[derive(Default)]
struct DdnaScanStats {
    assets_total: usize,
    assets_parsed: usize,
    assets_with_alpha: usize,
    assets_missing_alpha: usize,
    assets_decode_failed: usize,
    alpha_mips_total: usize,
    alpha_mips_decoded: usize,
    parse_failures: BTreeMap<String, usize>,
    alpha_format_layouts: BTreeMap<String, usize>,
    asset_format_layouts: BTreeMap<String, usize>,
    samples: BTreeMap<String, Vec<String>>,
}

impl DdnaScanStats {
    fn record(&mut self, result: &DdnaScanResult, sample_limit: usize) {
        self.assets_total += 1;
        if let Some(reason) = &result.parse_failure {
            *self.parse_failures.entry(reason.clone()).or_default() += 1;
            self.add_sample(&format!("parse_failure:{reason}"), &result.path, sample_limit);
            if reason == "missing_alpha_mips" {
                self.assets_parsed += 1;
                self.assets_missing_alpha += 1;
            }
            return;
        }

        self.assets_parsed += 1;
        self.assets_with_alpha += 1;
        self.alpha_mips_total += result.alpha_mip_buckets.len();
        self.alpha_mips_decoded += result.alpha_mips_decoded;

        if let Some(reason) = &result.decode_failure {
            self.assets_decode_failed += 1;
            self.add_sample(&format!("decode_failure:{reason}"), &result.path, sample_limit);
        }

        let mut asset_buckets = result.alpha_mip_buckets.clone();
        asset_buckets.sort();
        asset_buckets.dedup();
        let asset_bucket = asset_buckets.join(",");
        *self.asset_format_layouts.entry(asset_bucket.clone()).or_default() += 1;
        self.add_sample(
            &format!("asset_format_layout:{asset_bucket}"),
            &result.path,
            sample_limit,
        );

        for bucket in &result.alpha_mip_buckets {
            *self.alpha_format_layouts.entry(bucket.clone()).or_default() += 1;
        }
    }

    fn add_sample(&mut self, bucket: &str, path: &str, limit: usize) {
        if limit == 0 {
            return;
        }
        let samples = self.samples.entry(bucket.to_string()).or_default();
        if samples.len() < limit {
            samples.push(path.to_string());
        }
    }
}

struct DdnaScanResult {
    path: String,
    parse_failure: Option<String>,
    decode_failure: Option<String>,
    alpha_mip_buckets: Vec<String>,
    alpha_mips_decoded: usize,
}

fn scan_ddna_entry(
    p4k: &starbreaker_p4k::MappedP4k,
    entry: &starbreaker_p4k::P4kEntry,
    decode_alpha: bool,
) -> DdnaScanResult {
    let path = entry.name.clone();
    let base_bytes = match p4k.read(entry) {
        Ok(bytes) => bytes,
        Err(err) => {
            return DdnaScanResult {
                path,
                parse_failure: Some(format!("read_base:{err}")),
                decode_failure: None,
                alpha_mip_buckets: Vec::new(),
                alpha_mips_decoded: 0,
            };
        }
    };
    let reader = P4kDdsSiblingReader {
        p4k,
        base_path: path.clone(),
    };
    let dds = match DdsFile::from_split(&base_bytes, &reader) {
        Ok(dds) => dds,
        Err(err) => {
            return DdnaScanResult {
                path,
                parse_failure: Some(format!("invalid_dds:{err}")),
                decode_failure: None,
                alpha_mip_buckets: Vec::new(),
                alpha_mips_decoded: 0,
            };
        }
    };
    if !dds.has_alpha_mips() {
        return DdnaScanResult {
            path,
            parse_failure: Some("missing_alpha_mips".to_string()),
            decode_failure: None,
            alpha_mip_buckets: Vec::new(),
            alpha_mips_decoded: 0,
        };
    }

    let mut alpha_mip_buckets = Vec::with_capacity(dds.alpha_mip_data.len());
    let mut alpha_mips_decoded = 0;
    let mut decode_failure = None;
    for mip in 0..dds.alpha_mip_data.len() {
        let format = dds
            .alpha_mip_format_for_mip(mip)
            .map(alpha_mip_format_name)
            .unwrap_or("unknown");
        let layout = dds
            .alpha_mip_layout_for_mip(mip)
            .map(alpha_mip_layout_name)
            .unwrap_or("unknown");
        alpha_mip_buckets.push(format!("{format}/{layout}"));
        if decode_alpha {
            match dds.decode_alpha_mip(mip) {
                Ok(_) => alpha_mips_decoded += 1,
                Err(err) if decode_failure.is_none() => {
                    decode_failure = Some(format!("mip{mip}:{err}"));
                }
                Err(_) => {}
            }
        }
    }

    DdnaScanResult {
        path,
        parse_failure: None,
        decode_failure,
        alpha_mip_buckets,
        alpha_mips_decoded,
    }
}

fn print_count_table(title: &str, counts: &BTreeMap<String, usize>) {
    if counts.is_empty() {
        return;
    }
    println!();
    println!("{title}:");
    for (key, value) in counts {
        println!("  {key}: {value}");
    }
}

fn byte_statistics(values: &[u8]) -> Option<(u8, u8, u8)> {
    let (&first, rest) = values.split_first()?;
    let mut min = first;
    let mut max = first;
    let mut sum = u64::from(first);
    for value in rest {
        min = min.min(*value);
        max = max.max(*value);
        sum += u64::from(*value);
    }
    let mean = ((sum as f64) / (values.len() as f64)).round() as u8;
    Some((min, max, mean))
}

fn alpha_mip_format_name(format: AlphaMipFormat) -> &'static str {
    match format {
        AlphaMipFormat::Bc4Unorm => "bc4_unorm",
        AlphaMipFormat::Bc4Snorm => "bc4_snorm",
        AlphaMipFormat::R8Unorm => "r8_unorm",
    }
}

fn alpha_mip_layout_name(layout: AlphaMipLayout) -> &'static str {
    match layout {
        AlphaMipLayout::NumberedSibling => "numbered_sibling",
        AlphaMipLayout::HeaderedTail => "headered_tail",
        AlphaMipLayout::RawTailSplit => "raw_tail_split",
        AlphaMipLayout::RawSinglePayload => "raw_single_payload",
    }
}

fn is_ddna_base_dds_path(path: &str) -> bool {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file_name.to_ascii_lowercase().ends_with(".dds")
        && !file_name.to_ascii_lowercase().contains(".dds.")
        && path_has_file_stem_token(path, &["ddna"])
}

fn path_has_file_stem_token(path: &str, tokens: &[&str]) -> bool {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name)
        .to_ascii_lowercase();
    stem.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| tokens.iter().any(|expected| token == *expected))
}

/// Collect `.dds` files, skipping split siblings (.dds.1, .dds.2, etc.)
fn collect_base_dds_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_recursive(dir, &mut files)?;
    Ok(files)
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, out)?;
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".dds") && !name.contains(".dds.") {
                out.push(path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_ddna_base_dds_path, path_has_file_stem_token};

    #[test]
    fn ddna_scan_candidate_uses_file_stem_token() {
        assert!(is_ddna_base_dds_path(
            r"Data\Objects\FPS_Weapons\Test\panel_ddna.dds"
        ));
        assert!(is_ddna_base_dds_path(
            "Data/Objects/FPS_Weapons/Test/panel-ddna.dds"
        ));
        assert!(!is_ddna_base_dds_path(
            r"Data\Objects\Test_ddna_cache\panel_diff.dds"
        ));
        assert!(!is_ddna_base_dds_path(
            r"Data\Objects\FPS_Weapons\Test\panel_ddna.dds.7a"
        ));
        assert!(!is_ddna_base_dds_path(
            r"Data\Objects\FPS_Weapons\Test\panel_diff.dds"
        ));
    }

    #[test]
    fn file_stem_token_detection_does_not_match_substrings() {
        assert!(path_has_file_stem_token("panel.ddna.dds", &["ddna"]));
        assert!(path_has_file_stem_token("panel_ddna.dds", &["ddna"]));
        assert!(!path_has_file_stem_token("panel_ddnaold.dds", &["ddna"]));
        assert!(!path_has_file_stem_token(
            "Data/Objects/ddna_cache/panel_diff.dds",
            &["ddna"]
        ));
    }
}
