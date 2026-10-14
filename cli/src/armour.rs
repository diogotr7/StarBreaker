//! `starbreaker armour export`: armour and clothing worn by a character, the
//! whole set on one skeleton (spec
//! `docs/superpowers/specs/2026-10-13-armour-export-design.md`). Pieces are
//! picked by entity class name (`--wear`, placed on the ports that accept
//! them — `character::wear`) and/or come from a loadout XML (`--loadout`);
//! the wearer is a CHF (`--chf`) or the game's default character of
//! `--gender`. Output: a GLB, or a decomposed `.blend` package. Single
//! pieces export with `entity export <item> --geometry-tag Male|Female`.

use std::path::PathBuf;

use clap::Subcommand;
use starbreaker_3d::character::Gender;

use crate::chf::{CharacterArgs, character_animation, load_animation, load_loadout};
use crate::common::load_dcb_bytes;
use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum ArmourCommand {
    /// Export armour/clothing pieces worn by a character (GLB or package).
    Export {
        /// Output `.glb` file (`--kind glb`) or export root directory (`--kind package`)
        output: PathBuf,
        /// A piece to wear: exact entity class name (e.g. `cds_armor_heavy_arms_01_01_01`). Repeatable.
        #[arg(long)]
        wear: Vec<String>,
        /// Wearer gender: the game's default male or female character.
        #[arg(long, default_value = "male", value_parser = parse_gender, conflicts_with = "chf")]
        gender: Gender,
        /// Wear the pieces on this character (`.chf`) instead of the default one.
        #[arg(long)]
        chf: Option<PathBuf>,
        /// `glb` (one file) or `package` (decomposed `.blend` package).
        #[arg(long, default_value = "glb", value_parser = ["glb", "package"])]
        kind: String,
        /// Path to Data.p4k [default: auto-discover via SC_DATA_P4K]
        #[arg(long)]
        p4k: Option<PathBuf>,
        /// Texture mip for packages (0 = full resolution)
        #[arg(long, default_value = "0")]
        texture_mip: u32,
        #[command(flatten)]
        character: CharacterArgs,
    },
}

fn parse_gender(s: &str) -> std::result::Result<Gender, String> {
    s.parse()
}

impl ArmourCommand {
    pub fn run(self) -> Result<()> {
        let Self::Export { output, wear, gender, chf, kind, p4k, texture_mip, character } = self;
        let (p4k, dcb_bytes) = load_dcb_bytes(p4k.as_deref(), None)?;
        let p4k = p4k.ok_or_else(|| CliError::InvalidInput("Data.p4k is required".into()))?;
        let db = starbreaker_datacore::Database::from_bytes(&dcb_bytes)?;
        let chf_bytes = match &chf {
            Some(path) => std::fs::read(path).map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?,
            None => starbreaker_3d::character::default_wearer_chf(&p4k, gender).map_err(|e| CliError::InvalidInput(e.to_string()))?,
        };
        let parsed = starbreaker_chf::ChfFile::from_chf(&chf_bytes)
            .map_err(|e| CliError::InvalidInput(format!("chf: {e}")))?
            .parse()
            .map_err(|e| CliError::InvalidInput(format!("chf: {e}")))?;

        let mut entries = load_loadout(&p4k, &character)?;
        if !wear.is_empty() {
            // Picked pieces go on the character's and the loadout's items.
            let placed = starbreaker_3d::character::wear::dress_entries(&db, &parsed, &entries, &wear)
                .map_err(|e| CliError::InvalidInput(e.to_string()))?;
            for problem in &placed.unplaced {
                eprintln!("not worn: {problem}");
            }
            if placed.unplaced.len() == wear.len() {
                return Err(CliError::InvalidInput("none of the --wear pieces could be worn".into()));
            }
            entries = placed.entries;
        }
        if entries.is_empty() {
            return Err(CliError::InvalidInput("nothing to wear: give --wear and/or --loadout".into()));
        }

        let (caf, body) = load_animation(&p4k, &character)?;
        let clips = character_animation(&character, caf.as_deref(), body.as_ref());
        if kind == "glb" {
            let glb = starbreaker_3d::character::chf_to_character_glb(&chf_bytes, &p4k, &db, &clips, &entries)
                .map_err(|e| CliError::InvalidInput(format!("armour GLB failed: {e}")))?;
            std::fs::write(&output, &glb).map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
            eprintln!("Written {} bytes to {}", glb.len(), output.display());
            return Ok(());
        }
        let name = output.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "armour".into());
        let opts = starbreaker_3d::ExportOptions {
            kind: starbreaker_3d::ExportKind::Decomposed,
            format: starbreaker_3d::ExportFormat::Blend,
            texture_mip,
            ..starbreaker_3d::ExportOptions::default()
        };
        let export = starbreaker_3d::character::package::export_character_package(&p4k, &db, &chf_bytes, &name, &opts, &clips, &entries)
            .map_err(|e| CliError::InvalidInput(format!("armour package failed: {e}")))?;
        for file in &export.files {
            let path = output.join(&file.relative_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| CliError::IoPath { source: e, path: parent.display().to_string() })?;
            }
            std::fs::write(&path, &file.bytes).map_err(|e| CliError::IoPath { source: e, path: path.display().to_string() })?;
        }
        eprintln!("Wrote {} files under {}", export.files.len(), output.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: ArmourCommand,
    }

    #[test]
    fn export_parses_pieces_and_options() {
        let cli = Cli::try_parse_from(["t", "export", "out.glb", "--wear", "a", "--wear", "b", "--gender", "female"]).unwrap();
        let ArmourCommand::Export { wear, gender, kind, .. } = cli.command;
        assert_eq!(wear, ["a", "b"]);
        assert_eq!(gender, starbreaker_3d::character::Gender::Female);
        assert_eq!(kind, "glb");
    }

    #[test]
    fn the_chf_decides_the_gender() {
        assert!(Cli::try_parse_from(["t", "export", "o", "--wear", "a", "--chf", "x.chf", "--gender", "female"]).is_err());
        assert!(Cli::try_parse_from(["t", "export", "o", "--wear", "a", "--chf", "x.chf"]).is_ok());
    }

    #[test]
    fn kind_is_glb_or_package() {
        assert!(Cli::try_parse_from(["t", "export", "o", "--wear", "a", "--kind", "stl"]).is_err());
    }
}
