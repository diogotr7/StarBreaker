use std::path::PathBuf;

use clap::Subcommand;

use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum ChfCommand {
    /// Convert a .chf file to JSON
    ToJson {
        /// Input .chf file
        input: PathBuf,
        /// Output .json file [default: <input>.json]
        output: Option<PathBuf>,
    },
    /// Convert a JSON file back to .chf
    FromJson {
        /// Input .json file
        input: PathBuf,
        /// Output .chf file [default: <input>.chf]
        output: Option<PathBuf>,
    },
}

impl ChfCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::ToJson { input, output } => to_json(input, output),
            Self::FromJson { input, output } => from_json(input, output),
        }
    }
}

fn to_json(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let data = std::fs::read(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let json = starbreaker_chf::chf_to_json(&data)?;
    let output = output.unwrap_or_else(|| input.with_extension("json"));
    std::fs::write(&output, json.as_bytes())
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn from_json(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let json = std::fs::read_to_string(&input)
        .map_err(|e| CliError::IoPath { source: e, path: input.display().to_string() })?;
    let chf_bytes = starbreaker_chf::json_to_chf(&json)?;
    let output = output.unwrap_or_else(|| input.with_extension("chf"));
    std::fs::write(&output, &chf_bytes)
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written to {}", output.display());
    Ok(())
}
