use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;

use crate::common::{load_dcb_bytes, matches_filter};
use crate::error::{CliError, Result};

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
    /// Query DataCore records by property path
    Query {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// Path to Game2.dcb (alternative to --p4k)
        #[arg(long)]
        dcb: Option<PathBuf>,
        /// Property path (e.g. EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path)
        #[arg(long_help = QUERY_HELP)]
        path: String,
        /// Filter record names by glob
        #[arg(long)]
        filter: Option<String>,
    },
}

const QUERY_HELP: &str = "\
Dot-separated property path starting with the record type name.

If only a type name is given, matching records are dumped as JSON.
If properties are included, only those values are extracted.

Use [TypeFilter] for polymorphic arrays (e.g. Components[SGeometryResourceParams]).

Examples:
  EntityClassDefinition
  EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path
  AmmoParams.damage[DamageInfo].DamagePhysical";

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
            Self::Query {
                p4k,
                dcb,
                path,
                filter,
            } => query(p4k, dcb, path, filter),
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

fn query(
    p4k_path: Option<PathBuf>,
    dcb_path: Option<PathBuf>,
    path: String,
    filter: Option<String>,
) -> Result<()> {
    let (_p4k, dcb_bytes) = load_dcb_bytes(p4k_path.as_deref(), dcb_path.as_deref())?;
    let db = Database::from_bytes(&dcb_bytes)?;

    // Split off the type name (first segment before any dot).
    let type_name = path.split('.').next().unwrap();
    let has_properties = path.len() > type_name.len();

    // Resolve the struct type.
    if db.struct_index_by_name(type_name).is_none() {
        return Err(CliError::NotFound(format!("unknown record type: {type_name}")));
    }

    // Collect matching records.
    let records: Vec<_> = db
        .records_by_type_name(type_name)
        .filter(|r| {
            if !db.is_main_record(r) {
                return false;
            }
            let name = db.resolve_string2(r.name_offset);
            matches_filter(name, filter.as_deref(), None)
        })
        .collect();

    if records.is_empty() {
        let msg = if let Some(f) = &filter {
            format!("no {type_name} records matching filter: {f}")
        } else {
            format!("no main records of type {type_name}")
        };
        return Err(CliError::NotFound(msg));
    }

    eprintln!("{} record(s) matched.", records.len());

    if !has_properties {
        // No property path — dump full records as JSON.
        let stdout = std::io::stdout();
        for record in &records {
            let name = db.resolve_string2(record.name_offset);
            eprintln!("--- {name}");
            let mut lock = stdout.lock();
            starbreaker_datacore::export::write_json(&db, record, &mut lock)
                .map_err(|e| CliError::InvalidInput(format!("export error: {e}")))?;
            use std::io::Write;
            writeln!(lock)?;
        }
    } else {
        // Compile the property path and query each record.
        let compiled = db
            .compile_rooted::<Value>(&path)
            .map_err(|e| CliError::InvalidInput(format!("path error: {e}")))?;

        for record in &records {
            let name = db.resolve_string2(record.name_offset);
            match db.query::<Value>(&compiled, record) {
                Ok(values) if values.is_empty() => {}
                Ok(values) => {
                    for value in &values {
                        let json = value_to_json(value);
                        match json {
                            serde_json::Value::String(s) => println!("{name}\t{s}"),
                            serde_json::Value::Null => {}
                            other => println!("{name}\t{other}"),
                        }
                    }
                }
                Err(_) => {} // Path doesn't apply to this record (type filter mismatch, etc.)
            }
        }
    }

    Ok(())
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int8(n) => serde_json::json!(n),
        Value::Int16(n) => serde_json::json!(n),
        Value::Int32(n) => serde_json::json!(n),
        Value::Int64(n) => serde_json::json!(n),
        Value::UInt8(n) => serde_json::json!(n),
        Value::UInt16(n) => serde_json::json!(n),
        Value::UInt32(n) => serde_json::json!(n),
        Value::UInt64(n) => serde_json::json!(n),
        Value::Float(n) => serde_json::json!(n),
        Value::Double(n) => serde_json::json!(n),
        Value::String(s) => serde_json::Value::String(s.to_string()),
        Value::Guid(g) => serde_json::Value::String(format!("{g:?}")),
        Value::Enum(s) => serde_json::Value::String(s.to_string()),
        Value::Locale(s) => serde_json::Value::String(s.to_string()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
        Value::Object { type_name, fields, .. } => {
            let mut map = serde_json::Map::new();
            map.insert("__type".to_string(), serde_json::Value::String(type_name.to_string()));
            for (k, v) in fields {
                map.insert(k.to_string(), value_to_json(v));
            }
            serde_json::Value::Object(map)
        }
    }
}
