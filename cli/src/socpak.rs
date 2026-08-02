use std::path::PathBuf;

use clap::Subcommand;
use starbreaker_datacore::database::Database;

use crate::common::{load_dcb_bytes, ExportOpts};
use crate::entity::{prepare_decomposed_output_root, sanitize_export_name, write_decomposed_file};
use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum SocpakCommand {
    /// Export socpak interior containers to GLB or decomposed Blender package
    Export {
        /// P4k path substring for socpak files (case-insensitive)
        pattern: String,
        /// Output .glb path for bundled export, or output directory for decomposed Blender export
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
    let export_opts = starbreaker_3d::ExportOptions::from(&opts);
    match export_opts.kind {
        starbreaker_3d::ExportKind::Bundled => {
            let glb = starbreaker_3d::socpaks_to_glb(&db, &p4k, &socpak_paths, &export_opts)?;
            let output = output.unwrap_or_else(|| PathBuf::from(format!("{pattern}.glb")));
            std::fs::write(&output, &glb)
                .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
            eprintln!("Written {} bytes to {}", glb.len(), output.display());
        }
        starbreaker_3d::ExportKind::Decomposed => {
            let export_name = sanitize_export_name(&pattern);
            let decomposed = starbreaker_3d::socpaks_to_decomposed_blend(
                &db,
                &p4k,
                &socpak_paths,
                &export_name,
                &export_opts,
            )?;
            let output = output.unwrap_or_else(|| PathBuf::from(format!("{export_name}_socpak_export")));
            let package_name = format!(
                "{export_name}_LOD{}_TEX{}",
                export_opts.lod_level, export_opts.texture_mip
            );
            prepare_decomposed_output_root(&output, &package_name)?;
            for file in &decomposed.files {
                let output_path = output.join(&file.relative_path);
                if let Some(parent) = output_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| CliError::IoPath {
                        source: e,
                        path: parent.display().to_string(),
                    })?;
                }
                write_decomposed_file(
                    file,
                    &output_path,
                    opts.skip_existing_assets,
                    &p4k,
                    decomposed.streamed_paths.contains(&file.relative_path),
                )?;
            }
            eprintln!(
                "Written decomposed socpak Blender package with {} files to {}",
                decomposed.files.len(),
                output.display()
            );
        }
    }
    Ok(())
}
