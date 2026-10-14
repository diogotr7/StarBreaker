//! `starbreaker riglogic` subcommand — inspect and dump `.RigLogic` files.
//!
//! Accepts a standalone `.RigLogic` file or a `.skin` IVO container. Auto-
//! detects the input format from the first 4 bytes: `#ivo` → IVO container;
//! `RIG ` → standalone. Dumps the parsed struct as pretty JSON to stdout.
//!
//! The `--section` flag selects a named subset of the output:
//! - `header`      — counts, name, rig_type, section_starts
//! - `joints`      — joint_control_name_crcs + joint_control_names
//! - `regions`     — blend_region_bones + region_weights
//! - `appearance`  — appearance blob (record count + scale)
//! - `corrective`  — corrective blob + corrective_composition
//! - `protos-info` — ProtosInfo chunk (`.skin` input only)

use std::path::PathBuf;

use clap::Subcommand;
use starbreaker_3d::riglogic::sections::section_json;
use starbreaker_chunks::{ChunkFile, known_types};

use crate::error::{CliError, Result};

/// Magic bytes for auto-detection.
const IVO_MAGIC: &[u8; 4] = b"#ivo";
const RIG_MAGIC: &[u8; 4] = b"RIG ";

/// Size of the 8-byte data_size prefix that the IVO chunk wrapper prepends to
/// embedded RigLogic chunk content.
const IVO_CHUNK_PREFIX_BYTES: usize = 8;

#[derive(Subcommand)]
pub enum RigLogicCommand {
    /// Dump a .RigLogic file or a .skin IVO container as JSON
    Dump {
        /// Path to a standalone `.RigLogic` file or a `.skin` IVO container
        path: PathBuf,
        /// Dump only this section: header | joints | regions | appearance |
        /// corrective | protos-info
        #[arg(long, value_name = "SECTION")]
        section: Option<String>,
    },
}

impl RigLogicCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Dump { path, section } => dump(path, section),
        }
    }
}

fn dump(path: PathBuf, section: Option<String>) -> Result<()> {
    let data = std::fs::read(&path)
        .map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?;

    // Auto-detect: IVO container vs standalone RigLogic.
    let is_ivo = data.len() >= 4 && data[..4] == *IVO_MAGIC;
    let is_standalone = data.len() >= 4 && data[..4] == *RIG_MAGIC;

    if !is_ivo && !is_standalone {
        return Err(CliError::InvalidInput(format!(
            "unrecognised file format: expected '#ivo' or 'RIG ' magic at start of {}",
            path.display()
        )));
    }

    if is_ivo {
        dump_from_ivo(&data, section.as_deref(), &path)
    } else {
        dump_standalone(&data, section.as_deref())
    }
}

/// Dump from a standalone `.RigLogic` file.
fn dump_standalone(data: &[u8], section: Option<&str>) -> Result<()> {
    let rig = starbreaker_3d::riglogic::RigLogic::read(data)?;
    let json = section_json(&rig, None, section).map_err(CliError::InvalidInput)?;
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

/// Dump from a `.skin` IVO container.
fn dump_from_ivo(data: &[u8], section: Option<&str>, path: &PathBuf) -> Result<()> {
    let chunk_file = ChunkFile::from_bytes(data).map_err(|e| {
        CliError::InvalidInput(format!(
            "failed to parse IVO container {}: {e}",
            path.display()
        ))
    })?;
    let ChunkFile::Ivo(ivo) = &chunk_file else {
        return Err(CliError::InvalidInput(format!(
            "{} is not an IVO chunk file",
            path.display()
        )));
    };

    // Find the RigLogic chunk.
    let rl_entry = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == known_types::ivo::RIG_LOGIC)
        .ok_or_else(|| {
            CliError::NotFound(format!(
                "no RigLogic chunk (0x{:08X}) in {}",
                known_types::ivo::RIG_LOGIC,
                path.display()
            ))
        })?;

    // The IVO chunk wrapper prepends an 8-byte data_size field. Strip it.
    let raw = ivo.chunk_data(rl_entry);
    if raw.len() < IVO_CHUNK_PREFIX_BYTES {
        return Err(CliError::InvalidInput(format!(
            "RigLogic chunk in {} is too short ({} bytes)",
            path.display(),
            raw.len()
        )));
    }
    let rig_bytes = &raw[IVO_CHUNK_PREFIX_BYTES..];
    let rig = starbreaker_3d::riglogic::RigLogic::read(rig_bytes)?;

    // Optionally also parse ProtosInfo.
    let protos_info = if section.is_none() || section == Some("protos-info") {
        ivo.chunks()
            .iter()
            .find(|c| c.chunk_type == known_types::ivo::PROTOS_INFO)
            .map(|entry| {
                starbreaker_3d::riglogic::ProtosInfo::read(ivo.chunk_data(entry))
            })
            .transpose()?
    } else {
        None
    };

    let json = section_json(&rig, protos_info.as_ref(), section).map_err(CliError::InvalidInput)?;
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}
