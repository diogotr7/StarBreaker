use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use starbreaker_dds::DdsFile;
use starbreaker_dds::sibling::FsSiblingReader;

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
    /// Decode a DDS from P4k to PNG (for debugging)
    Decode {
        /// DDS path in P4k
        input: String,
        /// Output PNG path
        output: PathBuf,
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
}

impl DdsCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Info { input, p4k } => info(&input, p4k.as_deref()),
            Self::Decode { input, output, mip, alpha, p4k } => decode_p4k(&input, &output, mip, alpha, p4k.as_deref()),
            Self::Merge { input, output } => merge(input, output),
            Self::MergeAll { input, output } => merge_all(input, output),
            Self::ToPng { input, output } => to_png(input, output),
            Self::ToPngAll {
                input,
                output,
                filter,
            } => to_png_all(input, output, filter),
        }
    }
}

fn info(input: &str, p4k_path: Option<&Path>) -> Result<()> {
    let dds = if Path::new(input).exists() {
        // Filesystem path
        let data = std::fs::read(input).context("failed to read DDS file")?;
        let reader = FsSiblingReader::new(input);
        DdsFile::from_split(&data, &reader)
            .or_else(|_| DdsFile::headers_only(&data))
            .context("failed to parse DDS")?
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
            .ok_or_else(|| anyhow::anyhow!("not found in P4k: {normalized}"))?;
        let data = p4k.read(entry).context("failed to read from P4k")?;
        let p4k_reader = P4kDdsSiblingReader {
            p4k: &p4k,
            base_path: normalized,
        };
        // Try split merge first, fall back to header-only parse for unsupported formats
        DdsFile::from_split(&data, &p4k_reader)
            .or_else(|_| DdsFile::headers_only(&data))
            .context("failed to parse DDS")?
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
        .ok_or_else(|| anyhow::anyhow!("not found in P4k: {normalized}"))?;
    let data = p4k.read(entry).context("failed to read from P4k")?;
    let reader = P4kDdsSiblingReader {
        p4k: &p4k,
        base_path: normalized,
    };
    let dds = DdsFile::from_split(&data, &reader).context("failed to parse DDS")?;

    if alpha {
        // Decode alpha/smoothness from sibling mips
        anyhow::ensure!(dds.has_alpha_mips(), "no alpha mips found (no .Xa sibling files)");
        let mip = mip.min(dds.alpha_mip_data.len().saturating_sub(1));
        let (w, h) = dds.dimensions(mip);
        eprintln!("Decoding alpha mip {mip}: {w}x{h} ({} alpha mips available)", dds.alpha_mip_data.len());
        let smoothness = dds.decode_alpha_mip(mip).context("failed to decode alpha mip")?;
        // Write as grayscale PNG
        let img = image::GrayImage::from_raw(w, h, smoothness)
            .ok_or_else(|| anyhow::anyhow!("failed to create grayscale image"))?;
        img.save(output).context("failed to save PNG")?;
    } else {
        let mip = mip.min(dds.mip_count().saturating_sub(1));
        let (w, h) = dds.dimensions(mip);
        eprintln!("Decoding mip {mip}: {w}x{h}");
        dds.save_png(output, mip).context("failed to decode/save PNG")?;
    }

    eprintln!("Written to {}", output.display());
    Ok(())
}

fn merge(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let data = std::fs::read(&input).context("failed to read DDS file")?;
    let reader = FsSiblingReader::new(&input);
    let dds = DdsFile::from_split(&data, &reader).context("failed to parse/merge DDS")?;
    let merged = dds.to_dds();
    let output = output.unwrap_or_else(|| input.with_extension("merged.dds"));
    std::fs::write(&output, &merged)?;
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn merge_all(input: PathBuf, output: PathBuf) -> Result<()> {
    let files = collect_base_dds_files(&input)?;
    if files.is_empty() {
        anyhow::bail!("no base .dds files found");
    }
    eprintln!("Merging {} files...", files.len());
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len}")
            .unwrap(),
    );

    files.par_iter().for_each(|file| {
        let rel = file.strip_prefix(&input).unwrap_or(file);
        let out_path = output.join(rel);
        if let Some(parent) = out_path.parent() {
            let _ = std::fs::create_dir_all(parent);
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
    let data = std::fs::read(&input).context("failed to read DDS file")?;
    let reader = FsSiblingReader::new(&input);
    let dds = DdsFile::from_split(&data, &reader).context("failed to parse DDS")?;
    let output = output.unwrap_or_else(|| input.with_extension("png"));
    dds.save_png(&output, 0)
        .context("failed to decode/save PNG")?;
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
        anyhow::bail!("no matching .dds files found");
    }
    eprintln!("Converting {} files to PNG...", files.len());
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len}")
            .unwrap(),
    );

    files.par_iter().for_each(|file| {
        let rel = file.strip_prefix(&input).unwrap_or(file);
        let out_path = output.join(rel).with_extension("png");
        if let Some(parent) = out_path.parent() {
            let _ = std::fs::create_dir_all(parent);
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
