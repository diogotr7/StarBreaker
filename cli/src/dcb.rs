use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use starbreaker_datacore::database::Database;

use crate::common::{load_dcb_bytes, matches_filter};
use crate::error::Result;

#[derive(Clone, ValueEnum)]
pub enum DcbFormat {
    Json,
    Xml,
    Unp4k,
}

#[derive(Subcommand)]
pub enum DcbCommand {
    /// Extract DataCore records to individual files
    Extract {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Path to Game2.dcb (alternative to --p4k)
        #[arg(long)]
        dcb: Option<PathBuf>,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
        /// Output format
        #[arg(long, value_enum, default_value = "xml")]
        format: DcbFormat,
        /// Filter record names by glob
        #[arg(long)]
        filter: Option<String>,
    },
}

impl DcbCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Extract {
                p4k,
                dcb,
                output,
                format,
                filter,
            } => extract(p4k, dcb, output, format, filter),
        }
    }
}

fn extract(
    p4k_path: Option<PathBuf>,
    dcb_path: Option<PathBuf>,
    output: PathBuf,
    format: DcbFormat,
    filter: Option<String>,
) -> Result<()> {
    let (_p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), dcb_path.as_deref())?;
    let db = Database::from_bytes(&dcb_bytes)?;

    eprintln!("DataCore loaded.");

    let ext = match format {
        DcbFormat::Json => "json",
        DcbFormat::Xml | DcbFormat::Unp4k => "xml",
    };

    // Only export main records (matching C#'s behavior), using the file path
    // from the DataCore as the output directory structure.
    let records: Vec<_> = db
        .records()
        .iter()
        .filter(|r| {
            if !db.is_main_record(r) {
                return false;
            }
            let file_name = db.resolve_string(r.file_name_offset);
            matches_filter(file_name, filter.as_deref(), None)
        })
        .collect();

    eprintln!("Exporting {} records as {ext}...", records.len());

    let pb = ProgressBar::new(records.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len} ({elapsed}, ETA {eta})")?,
    );

    std::fs::create_dir_all(&output)?;

    records.par_iter().for_each(|record| {
        let file_name = db.resolve_string(record.file_name_offset);
        // Change extension to match output format (C# uses Path.ChangeExtension)
        let out_name = match file_name.rfind('.') {
            Some(dot) => format!("{}.{ext}", &file_name[..dot]),
            None => format!("{file_name}.{ext}"),
        };
        let out_path = output.join(&out_name);

        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("[ERR] create dir {}: {e}", parent.display());
            }
        }

        let result = match format {
            DcbFormat::Json => starbreaker_datacore::export::to_json(&db, record),
            DcbFormat::Unp4k => starbreaker_datacore::export::to_unp4k_xml(&db, record),
            DcbFormat::Xml => starbreaker_datacore::export::to_xml(&db, record),
        };

        match result {
            Ok(data) => {
                if let Err(e) = std::fs::write(&out_path, &data) {
                    eprintln!("Error writing {out_name}: {e}");
                }
            }
            Err(e) => eprintln!("Error exporting {file_name}: {e}"),
        }
        pb.inc(1);
    });

    pb.finish_and_clear();
    eprintln!("Done.");
    Ok(())
}
