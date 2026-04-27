//! NMC (Node Mesh Combo) inspection CLI.
//!
//! Provides the `dump` subcommand that reads a `.cga` / `.cgf` file's
//! NMC_Full chunk and prints each node's parent, scale, geometry type,
//! and `bone_to_world` 3×4 matrix. Used for diagnosing per-node basis
//! issues such as the wing-rotator top/bottom mirror documented in
//! `docs/StarBreaker/todo.md` (Phase 23 Sub-phase A).

use std::path::PathBuf;

use clap::Subcommand;

use starbreaker_3d::nmc::parse_nmc_full;

use crate::common::load_p4k;
use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum NmcCommand {
    /// Dump NMC nodes (parent, scale, geometry type, bone_to_world matrix)
    /// from a `.cga` / `.cgf` file.
    Dump {
        /// Path to a `.cga` / `.cgf` file. If the path begins with `Data/`
        /// or matches a P4k-internal path, the file is read from the P4k
        /// (requires `--p4k` or `SC_DATA_P4K`).
        path: String,
        /// Path to Data.p4k (only required when reading P4k-internal paths).
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Filter nodes by case-insensitive substring match on the node name.
        #[arg(long)]
        filter: Option<String>,
    },
}

impl NmcCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Dump { path, p4k, filter } => dump(path, p4k, filter),
        }
    }
}

fn read_input(path: &str, p4k_path: Option<&PathBuf>) -> Result<Vec<u8>> {
    let direct = std::path::Path::new(path);
    if direct.is_file() {
        return std::fs::read(direct).map_err(|e| CliError::IoPath {
            source: e,
            path: direct.display().to_string(),
        });
    }
    let p4k = load_p4k(p4k_path.map(|p| p.as_path()))
        .map_err(|e| CliError::InvalidInput(format!("p4k open failed: {e}")))?;
    let entry = p4k.entry_case_insensitive(path).ok_or_else(|| {
        CliError::NotFound(format!("'{path}' not found in P4k"))
    })?;
    p4k.read(entry)
        .map_err(|e| CliError::InvalidInput(format!("p4k read failed: {e}")))
}

fn dump(path: String, p4k_path: Option<PathBuf>, filter: Option<String>) -> Result<()> {
    let data = read_input(&path, p4k_path.as_ref())?;
    let (nodes, _mat_indices) = parse_nmc_full(&data).ok_or_else(|| {
        CliError::InvalidInput(format!(
            "no NMC_Full chunk found (or parse failed) in '{path}'"
        ))
    })?;

    let filter_lower = filter.as_deref().map(|s| s.to_lowercase());

    println!("# NMC dump: {} ({} nodes)", path, nodes.len());
    println!("# Each node prints: index name parent=<i|root> geom=<type> scale=[x,y,z]");
    println!("# Then bone_to_world (3 rows × 4 cols, row-major):");
    println!("#   [m00 m01 m02 tx]   <- row 0 (basis X axis + translation X)");
    println!("#   [m10 m11 m12 ty]   <- row 1 (basis Y axis + translation Y)");
    println!("#   [m20 m21 m22 tz]   <- row 2 (basis Z axis + translation Z)");
    println!();

    let mut shown = 0usize;
    for (idx, n) in nodes.iter().enumerate() {
        if let Some(f) = filter_lower.as_deref() {
            if !n.name.to_lowercase().contains(f) {
                continue;
            }
        }
        let parent = match n.parent_index {
            Some(p) => format!("{p}"),
            None => "root".to_string(),
        };
        println!(
            "[{idx:3}] {name} parent={parent} geom={geom} scale=[{sx:.4},{sy:.4},{sz:.4}]",
            name = n.name,
            geom = n.geometry_type,
            sx = n.scale[0],
            sy = n.scale[1],
            sz = n.scale[2],
        );
        for row in 0..3 {
            println!(
                "       b2w[{row}] = [{:>10.5}, {:>10.5}, {:>10.5}, {:>10.5}]",
                n.bone_to_world[row][0],
                n.bone_to_world[row][1],
                n.bone_to_world[row][2],
                n.bone_to_world[row][3],
            );
        }
        if !n.properties.is_empty() {
            let mut keys: Vec<&String> = n.properties.keys().collect();
            keys.sort();
            for k in keys {
                println!("       prop {k} = {}", n.properties[k]);
            }
        }
        println!();
        shown += 1;
    }

    if shown == 0 {
        if let Some(f) = filter.as_deref() {
            return Err(CliError::NotFound(format!(
                "no nodes matched filter '{f}'"
            )));
        }
    }

    Ok(())
}
