use std::path::PathBuf;

use clap::Subcommand;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::{EntityIndex, LoadoutNode, resolve_loadout_indexed};
use starbreaker_datacore::types::Record;

use crate::common::{ExportOpts, load_dcb_bytes};
use crate::error::{CliError, Result};

fn bundled_extension(format: starbreaker_3d::ExportFormat) -> &'static str {
    match format {
        starbreaker_3d::ExportFormat::Glb => "glb",
        starbreaker_3d::ExportFormat::Stl => "stl",
    }
}

fn prepare_decomposed_output_root(output: &PathBuf, explicit_output: bool) -> Result<()> {
    if output.exists() {
        if output.is_file() {
            return Err(CliError::InvalidInput(format!(
                "decomposed output root '{}' already exists as a file",
                output.display(),
            )));
        }

        if explicit_output {
            let mut entries = std::fs::read_dir(output)
                .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
            if entries
                .next()
                .transpose()
                .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?
                .is_some()
            {
                return Err(CliError::InvalidInput(format!(
                    "decomposed output directory '{}' must be empty or absent",
                    output.display(),
                )));
            }
        } else {
            std::fs::remove_dir_all(output)
                .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
        }
    }

    std::fs::create_dir_all(output)
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    Ok(())
}

#[derive(Subcommand)]
pub enum EntityCommand {
    /// Export entity to a bundled file
    Export {
        /// Entity name (substring, case-insensitive)
        name: String,
        /// Output bundled file path
        output: Option<PathBuf>,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Write hierarchy JSON instead of GLB
        #[arg(long)]
        dump_hierarchy: bool,
        #[command(flatten)]
        opts: ExportOpts,
    },
    /// Print entity loadout tree
    Loadout {
        /// Entity name (substring, case-insensitive)
        name: String,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
}

impl EntityCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Export {
                name,
                output,
                p4k,
                dump_hierarchy,
                opts,
            } => export(name, output, p4k, dump_hierarchy, opts),
            Self::Loadout { name, p4k } => loadout(name, p4k),
        }
    }
}

fn find_candidates<'a>(db: &'a Database, search: &str) -> Result<Vec<&'a Record>> {
    let search = search.to_lowercase();
    let entity_si = db
        .struct_id("EntityClassDefinition")
        .ok_or_else(|| CliError::NotFound("EntityClassDefinition struct not found in DCB".into()))?;
    let mut candidates: Vec<_> = db
        .records_of_type(entity_si)
        .filter(|r| {
            db.resolve_string2(r.name_offset)
                .to_lowercase()
                .contains(&search)
        })
        .collect();
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());
    Ok(candidates)
}

fn export(
    name: String,
    output: Option<PathBuf>,
    p4k_path: Option<PathBuf>,
    dump_hierarchy: bool,
    opts: ExportOpts,
) -> Result<()> {
    crate::log_mem_stats("start");
    let (p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    crate::log_mem_stats("after p4k+dcb load");
    let p4k = p4k.ok_or_else(|| CliError::MissingRequirement("P4k required for entity export".into()))?;
    let db = Database::from_bytes(&dcb_bytes)?;
    crate::log_mem_stats("after db parse");

    let candidates = find_candidates(&db, &name)?;
    if candidates.is_empty() {
        return Err(CliError::NotFound(format!("no EntityClassDefinition records matching '{name}'")));
    }

    let record = candidates[0];
    let rname = db.resolve_string2(record.name_offset);
    if candidates.len() > 1 {
        eprintln!("Found {} candidates, using shortest match: {rname}", candidates.len());
    }

    let idx = EntityIndex::new(&db);
    let export_opts = starbreaker_3d::ExportOptions::from(&opts);
    let explicit_output = output.is_some();
    let output = output.unwrap_or_else(|| {
        match export_opts.kind {
            starbreaker_3d::ExportKind::Bundled => {
                PathBuf::from(format!("{name}.{}", bundled_extension(export_opts.format)))
            }
            starbreaker_3d::ExportKind::Decomposed => PathBuf::from(name.clone()),
        }
    });

    crate::log_mem_stats("before loadout resolve");
    let tree = resolve_loadout_indexed(&idx, record);
    crate::log_mem_stats("after loadout resolve");

    eprintln!("\nLoadout tree for {}:", tree.root.entity_name);
    for child in &tree.root.children {
        let g = if child.geometry_path.is_some() { "G" } else { "." };
        eprintln!("  {g} {} -> {}", child.item_port_name, child.entity_name);
    }

    if dump_hierarchy {
        let json = starbreaker_3d::dump_hierarchy(&db, &p4k, record, &tree);
        let json_path = output.with_extension("json");
        std::fs::write(&json_path, &json)
            .map_err(|e| CliError::IoPath { source: e, path: json_path.display().to_string() })?;
        eprintln!("Hierarchy written to {}", json_path.display());
        return Ok(());
    }

    crate::log_mem_stats("before export");
    let result = starbreaker_3d::assemble_glb_with_loadout(&db, &p4k, record, &tree, &export_opts)?;
    crate::log_mem_stats("after export");
    eprintln!("Geometry: {}", result.geometry_path);
    eprintln!("Material: {}", result.material_path);
    match result.kind {
        starbreaker_3d::ExportKind::Bundled => {
            let bundled_bytes = result.bundled_bytes().ok_or_else(|| {
                CliError::InvalidInput(format!(
                    "entity export returned non-bundled output for {:?}",
                    result.kind,
                ))
            })?;
            eprintln!("Bundled export size: {} bytes", bundled_bytes.len());
            std::fs::write(&output, bundled_bytes)
                .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
        }
        starbreaker_3d::ExportKind::Decomposed => {
            let decomposed = result.decomposed.as_ref().ok_or_else(|| {
                CliError::InvalidInput("entity export returned no decomposed files".into())
            })?;
            eprintln!("Decomposed export file count: {}", decomposed.files.len());
            prepare_decomposed_output_root(&output, explicit_output)?;
            for file in &decomposed.files {
                let output_path = output.join(&file.relative_path);
                if let Some(parent) = output_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| CliError::IoPath { source: e, path: parent.display().to_string() })?;
                }
                std::fs::write(&output_path, &file.bytes)
                    .map_err(|e| CliError::IoPath { source: e, path: output_path.display().to_string() })?;
            }
        }
    }
    crate::log_mem_stats("after write");
    eprintln!("Written to {}", output.display());
    Ok(())
}

fn loadout(name: String, p4k_path: Option<PathBuf>) -> Result<()> {
    let (_, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), None)?;
    let db = Database::from_bytes(&dcb_bytes)?;

    let candidates = find_candidates(&db, &name)?;
    if candidates.is_empty() {
        return Err(CliError::NotFound(format!("no EntityClassDefinition records matching '{name}'")));
    }

    let idx = EntityIndex::new(&db);
    for record in &candidates {
        let tree = resolve_loadout_indexed(&idx, record);
        print_loadout_node(&tree.root, 0);
    }
    Ok(())
}

fn print_loadout_node(node: &LoadoutNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let geom = node.geometry_path.as_deref().unwrap_or("-");
    println!(
        "{indent}{} [{}] geom={geom}",
        node.entity_name, node.item_port_name
    );
    for child in &node.children {
        print_loadout_node(child, depth + 1);
    }
}
