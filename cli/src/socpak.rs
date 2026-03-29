use std::path::PathBuf;

use clap::Subcommand;
use starbreaker_datacore::database::Database;

use crate::common::{load_dcb_bytes, ExportOpts};
use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum SocpakCommand {
    /// Export socpak interior containers to GLB
    Export {
        /// P4k path substring for socpak files (case-insensitive)
        pattern: String,
        /// Output .glb path
        output: Option<PathBuf>,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        #[command(flatten)]
        opts: ExportOpts,
    },
}

impl SocpakCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Export {
                pattern,
                output,
                p4k,
                opts,
            } => export(pattern, output, p4k, opts),
        }
    }
}

fn export(
    pattern: String,
    output: Option<PathBuf>,
    p4k_path: Option<PathBuf>,
    opts: ExportOpts,
) -> Result<()> {
    let (p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    let p4k = p4k.ok_or_else(|| CliError::MissingRequirement("P4k required for socpak export".into()))?;
    let db = Database::from_bytes(&dcb_bytes)?;

    let search_lower = pattern.to_lowercase();
    let socpak_paths: Vec<String> = p4k
        .entries()
        .iter()
        .filter(|e| {
            let name = e.name.to_lowercase();
            name.contains(&search_lower) && name.ends_with(".socpak")
        })
        .map(|e| e.name.clone())
        .collect();

    if socpak_paths.is_empty() {
        return Err(CliError::NotFound(format!("no .socpak files matching '{pattern}'")));
    }

    eprintln!("Found {} socpak files", socpak_paths.len());
    let export_opts = starbreaker_gltf::ExportOptions::from(&opts);
    let glb = starbreaker_gltf::socpaks_to_glb(&db, &p4k, &socpak_paths, &export_opts)?;

    let output = output.unwrap_or_else(|| PathBuf::from(format!("{pattern}.glb")));
    std::fs::write(&output, &glb)?;
    eprintln!("Written {} bytes to {}", glb.len(), output.display());
    Ok(())
}
