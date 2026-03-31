use std::path::{Path, PathBuf};
use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum CryxmlCommand {
    /// Convert a CryXmlB file to XML
    Convert {
        /// Input CryXmlB file
        input: PathBuf,
        /// Output XML file [default: <input>.xml]
        output: Option<PathBuf>,
    },
    /// Batch convert CryXmlB files to XML
    ConvertAll {
        /// Input directory
        #[arg(short, long)]
        input: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
        /// Glob filter for filenames (e.g. "*.xml")
        #[arg(long, default_value = "*")]
        filter: String,
    },
}

impl CryxmlCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Convert { input, output } => convert(input, output),
            Self::ConvertAll { input, output, filter } => convert_all(input, output, filter),
        }
    }
}

fn convert(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let data = std::fs::read(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    if !starbreaker_cryxml::is_cryxmlb(&data) {
        return Err(CliError::InvalidInput(format!("{} is not a CryXmlB file", input.display())));
    }
    let cryxml = starbreaker_cryxml::from_bytes(&data)?;
    let xml = format!("{cryxml}");
    let output = output.unwrap_or_else(|| input.with_extension("xml"));
    std::fs::write(&output, xml.as_bytes())
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn convert_all(input: PathBuf, output: PathBuf, filter: String) -> Result<()> {
    let files = walkdir(&input, &filter)?;
    if files.is_empty() {
        return Err(CliError::NotFound(format!("no files found in {}", input.display())));
    }

    eprintln!("Converting {} files...", files.len());
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(ProgressStyle::default_bar().template("[{bar:40}] {pos}/{len} ({elapsed}, ETA {eta})")?);

    let errors = std::sync::atomic::AtomicUsize::new(0);

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
            if !starbreaker_cryxml::is_cryxmlb(&data) {
                return Ok(());
            }
            let cryxml = starbreaker_cryxml::from_bytes(&data)?;
            std::fs::write(&out_path, format!("{cryxml}").as_bytes())?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Error converting {}: {e}", file.display());
            errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        pb.inc(1);
    });

    pb.finish_and_clear();
    let err_count = errors.load(std::sync::atomic::Ordering::Relaxed);
    eprintln!("Done. {} errors.", err_count);
    Ok(())
}

fn walkdir(dir: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    walk_recursive(dir, pattern, &mut files)?;
    Ok(files)
}

fn walk_recursive(dir: &Path, pattern: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_recursive(&path, pattern, out)?;
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if glob_match::glob_match(pattern, name) {
                out.push(path);
            }
        }
    }
    Ok(())
}
