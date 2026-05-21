use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use rayon::prelude::*;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::enums::{ConversionType, DataType};
use starbreaker_p4k::{MappedP4k, P4kArchive, P4kEntry};

use crate::common::load_p4k;
use crate::error::{CliError, Result};

#[derive(Clone, Copy, ValueEnum)]
pub enum DiffFormat {
    Json,
    Xml,
}

impl DiffFormat {
    fn extension(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Xml => "xml",
        }
    }
}

/// Generate diffable reports from a Star Citizen install
#[derive(Args)]
pub struct DiffCommand {
    /// Path to Data.p4k
    #[arg(long, env = "SC_DATA_P4K")]
    p4k: Option<PathBuf>,
    /// Output directory for the report tree
    #[arg(short, long, env = "OUTPUT_FOLDER")]
    output: PathBuf,
    /// Keep existing generated files in the output directory
    #[arg(short, long, env = "KEEP_OLD")]
    keep: bool,
    /// Output format for text reports
    #[arg(short, long, value_enum, default_value = "xml", env = "TEXT_FORMAT")]
    format: DiffFormat,
    /// Include compressed DataCore.dcb and StarCitizen.exe snapshots
    #[arg(short = 'b', long, env = "INCLUDE_BINARIES")]
    include_binaries: bool,
}

pub fn report_root(p4k_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = p4k_path {
        return Ok(path);
    }
    Ok(load_p4k(None)?.path().to_path_buf())
}

impl DiffCommand {
    pub fn run(self) -> Result<()> {
        let started = std::time::Instant::now();
        let p4k_path = report_root(self.p4k)?;
        let game_dir = p4k_path.parent().map(Path::to_path_buf);
        let binary_sources = if self.include_binaries {
            Some(resolve_binary_sources(game_dir.as_deref())?)
        } else {
            None
        };

        if !self.keep {
            clean_output(
                &self.output,
                matches!(self.format, DiffFormat::Json),
                self.include_binaries,
            )?;
        }

        let p4k = MappedP4k::open(&p4k_path)?;
        eprintln!("P4k loaded: {}", p4k.path().display());

        let mut sw = std::time::Instant::now();
        dump_p4k_listing(&p4k, &self.output.join("P4k"), &self.format)?;
        eprintln!("P4k listing dumped in {:?}", sw.elapsed());

        sw = std::time::Instant::now();
        extract_report_contents(&p4k, &self.output.join("P4kContents"))?;
        eprintln!("P4k contents extracted in {:?}", sw.elapsed());

        sw = std::time::Instant::now();
        let dcb_bytes = p4k
            .read_file("Data\\Game2.dcb")
            .or_else(|_| p4k.read_file("Data\\Game.dcb"))?;
        let db = Database::from_bytes(&dcb_bytes)?;
        export_datacore_records(&db, &self.output.join("DataCore"), &self.format)?;
        export_datacore_types(&db, &self.output.join("DataCoreTypes"), &self.format)?;
        export_datacore_enums(&db, &self.output.join("DataCoreEnums"), &self.format)?;
        eprintln!("DataCore reports exported in {:?}", sw.elapsed());

        if let Some(binary_sources) = binary_sources {
            sw = std::time::Instant::now();
            write_binary_snapshots(&self.output, &dcb_bytes, &binary_sources.exe_path)?;
            eprintln!("Binary snapshots exported in {:?}", sw.elapsed());
        }

        if let Some(game_dir) = game_dir {
            copy_build_manifest(&game_dir, &self.output)?;
        }

        eprintln!("Done in {:?}", started.elapsed());
        Ok(())
    }
}

fn clean_output(output: &Path, json: bool, include_binaries: bool) -> Result<()> {
    for dir in [
        "DataCore",
        "DataCoreTypes",
        "DataCoreEnums",
        "P4k",
        "P4kContents",
        "Protobuf",
    ] {
        let path = output.join(dir);
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
    }

    let manifest = output.join("build_manifest.json");
    if manifest.exists() {
        std::fs::remove_file(manifest)?;
    }

    if include_binaries {
        for file in ["DataCore.dcb.zst", "StarCitizen.exe.zst"] {
            let path = output.join(file);
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
    }

    let root_listing = output.join("P4k").join(format!("Data.{}", if json { "json" } else { "xml" }));
    if root_listing.exists() {
        std::fs::remove_file(root_listing)?;
    }
    Ok(())
}

struct BinarySources {
    exe_path: PathBuf,
}

fn resolve_binary_sources(game_dir: Option<&Path>) -> Result<BinarySources> {
    if let Ok(path) = std::env::var(starbreaker_common::discover::ENV_EXE) {
        let exe_path = PathBuf::from(&path);
        if exe_path.is_file() {
            return Ok(BinarySources { exe_path });
        }
        return Err(CliError::MissingRequirement(format!(
            "--include-binaries requires StarCitizen.exe, but SC_EXE points to missing file '{}'",
            exe_path.display()
        )));
    }

    if let Some(game_dir) = game_dir {
        let exe_path = game_dir.join("Bin64").join("StarCitizen.exe");
        if exe_path.is_file() {
            return Ok(BinarySources { exe_path });
        }
    }

    let exe_path = starbreaker_common::discover::find_exe()
        .map_err(|e| {
            let p4k_hint = game_dir
                .map(|dir| {
                    format!(
                        "; also checked '{}'",
                        dir.join("Bin64").join("StarCitizen.exe").display()
                    )
                })
                .unwrap_or_default();
            CliError::MissingRequirement(format!(
                "--include-binaries requires StarCitizen.exe discovery: {e}{p4k_hint}; set SC_EXE if needed"
            ))
        })?
        .path;
    Ok(BinarySources { exe_path })
}

fn write_binary_snapshots(output: &Path, dcb_bytes: &[u8], exe_path: &Path) -> Result<()> {
    std::fs::create_dir_all(output)?;
    write_zstd_reader(
        Cursor::new(dcb_bytes),
        &output.join("DataCore.dcb.zst"),
    )?;
    write_zstd_reader(File::open(exe_path)?, &output.join("StarCitizen.exe.zst"))?;
    Ok(())
}

fn write_zstd_reader(mut input: impl Read, output_path: &Path) -> Result<()> {
    let output = File::create(output_path)?;
    let mut encoder = zstd::stream::write::Encoder::new(output, 0)?;
    std::io::copy(&mut input, &mut encoder)?;
    encoder.finish()?;
    Ok(())
}

fn dump_p4k_listing(p4k: &MappedP4k, output: &Path, format: &DiffFormat) -> Result<()> {
    let mut dirs = p4k
        .entries()
        .par_iter()
        .try_fold(HashMap::new, |mut dirs, entry| {
            insert_p4k_report_entry(p4k, &mut dirs, entry, "")?;
            Ok::<_, CliError>(dirs)
        })
        .try_reduce(HashMap::new, |mut left, right| {
            merge_p4k_dirs(&mut left, right);
            Ok::<_, CliError>(left)
        })?;

    dirs.par_iter_mut().for_each(|(_, files)| {
        sort_p4k_files(files);
    });

    dirs.par_iter()
        .try_for_each(|(dir, files)| write_p4k_dir_report(output, dir, files, format))?;
    Ok(())
}

fn insert_p4k_report_entry(
    p4k: &MappedP4k,
    dirs: &mut HashMap<String, Vec<P4kEntry>>,
    entry: &P4kEntry,
    prefix: &str,
) -> Result<()> {
    let report_name = report_entry_name(prefix, &entry.name);

    if should_expand_socpak(&report_name) {
        let data = p4k.read(entry)?;
        let archive = P4kArchive::from_bytes(&data)?;
        for inner_entry in archive.entries() {
            insert_nested_p4k_report_entry(&archive, dirs, inner_entry, &report_name)?;
        }
        return Ok(());
    }

    let mut report_entry = entry.clone();
    report_entry.name = report_name;
    let dir = split_entry_dir(&report_entry.name).to_string();
    dirs.entry(dir).or_default().push(report_entry);
    Ok(())
}

fn insert_nested_p4k_report_entry(
    archive: &P4kArchive<'_>,
    dirs: &mut HashMap<String, Vec<P4kEntry>>,
    entry: &P4kEntry,
    prefix: &str,
) -> Result<()> {
    let report_name = report_entry_name(prefix, &entry.name);

    if should_expand_socpak(&report_name) {
        let data = archive.read(entry)?;
        let nested = P4kArchive::from_bytes(&data)?;
        for inner_entry in nested.entries() {
            insert_nested_p4k_report_entry(&nested, dirs, inner_entry, &report_name)?;
        }
        return Ok(());
    }

    let mut report_entry = entry.clone();
    report_entry.name = report_name;
    let dir = split_entry_dir(&report_entry.name).to_string();
    dirs.entry(dir).or_default().push(report_entry);
    Ok(())
}

fn report_entry_name(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}\\{name}")
    }
}

fn should_expand_socpak(path: &str) -> bool {
    let file_name = path.rsplit('\\').next().unwrap_or(path);
    ascii_ends_with_ignore_case(file_name, ".socpak")
        && !ascii_starts_with_ignore_case(file_name, "shadercache_")
}

fn ascii_starts_with_ignore_case(value: &str, prefix: &str) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn ascii_ends_with_ignore_case(value: &str, suffix: &str) -> bool {
    value
        .as_bytes()
        .get(value.len().saturating_sub(suffix.len())..)
        .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix.as_bytes()))
}

fn sort_p4k_files(files: &mut [P4kEntry]) {
    files.sort_by_cached_key(|entry| {
        let file_name = split_entry_file_name(&entry.name);
        (legacy_p4k_sort_key(file_name), file_name.to_string())
    });
}

fn legacy_p4k_sort_key(value: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(value.len());
    for byte in value.bytes().map(|byte| byte.to_ascii_lowercase()) {
        match byte {
            b' ' => key.extend_from_slice(&[0, byte]),
            b'_' => key.extend_from_slice(&[1, byte]),
            b'-' => key.extend_from_slice(&[2, byte]),
            b'.' => key.extend_from_slice(&[3, byte]),
            b'0'..=b'9' => key.extend_from_slice(&[5, byte]),
            b'a'..=b'z' => key.extend_from_slice(&[6, byte]),
            b'\'' | b'(' | b')' | b'&' => key.extend_from_slice(&[4, byte]),
            _ => key.extend_from_slice(&[5, byte]),
        }
    }
    key
}

fn write_p4k_dir_report(
    output: &Path,
    dir: &str,
    files: &[P4kEntry],
    format: &DiffFormat,
) -> Result<()> {
    let name = dir.rsplit('\\').next().unwrap_or("Data");
    let out_dir = join_p4k_dir(output, dir);
    std::fs::create_dir_all(&out_dir)?;
    let out_path = out_dir.join(format!("{name}.{}", format.extension()));

    match format {
        DiffFormat::Json => write_p4k_dir_json(&out_path, name, files),
        DiffFormat::Xml => write_p4k_dir_xml(&out_path, name, files),
    }
}

fn write_p4k_dir_json(path: &Path, name: &str, files: &[P4kEntry]) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    let files = files.iter().map(|entry| {
        let file_name = split_entry_file_name(&entry.name);
        serde_json::json!({
            "Name": file_name,
            "CRC32": format!("0x{:08X}", entry.crc32),
            "Size": entry.uncompressed_size.to_string(),
            "CompressionType": entry.compression_method.to_string(),
            "Encrypted": legacy_bool(entry.is_encrypted),
        })
    });
    let report = serde_json::json!({
        "Name": name,
        "Files": files.collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut writer, &report)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn write_p4k_dir_xml(path: &Path, name: &str, files: &[P4kEntry]) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n")?;
    write!(writer, "<Directory Name=\"")?;
    write_xml_escaped(&mut writer, name)?;
    write!(writer, "\">\r\n")?;
    for entry in files {
        let file_name = split_entry_file_name(&entry.name);
        write!(
            writer,
            "  <File Name=\""
        )?;
        write_xml_escaped(&mut writer, file_name)?;
        write!(
            writer,
            "\" CRC32=\"0x{:08X}\" Size=\"{}\" CompressionType=\"{}\" Encrypted=\"{}\" />\r\n",
            entry.crc32, entry.uncompressed_size, entry.compression_method, legacy_bool(entry.is_encrypted)
        )?;
    }
    write!(writer, "</Directory>")?;
    Ok(())
}

fn extract_report_contents(p4k: &MappedP4k, output: &Path) -> Result<()> {
    let patterns = ["english\\global.ini", "tagdatabase.tagdatabase.xml"];
    let entries: Vec<&P4kEntry> = p4k
        .entries()
        .iter()
        .filter(|entry| {
            patterns
                .iter()
                .any(|suffix| ascii_ends_with_ignore_case(&entry.name, suffix))
        })
        .collect();

    entries.par_iter().try_for_each(|entry| {
        let data = p4k.read(entry)?;
        let out_path = output.join(entry.name.replace('\\', "/"));
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out_path, data)?;
        Ok::<_, CliError>(())
    })?;
    Ok(())
}

fn export_datacore_records(db: &Database<'_>, output: &Path, format: &DiffFormat) -> Result<()> {
    let ext = format.extension();
    let records: Vec<_> = db
        .records()
        .iter()
        .filter(|record| db.is_main_record(record))
        .collect();

    records.par_iter().try_for_each_init(
        || (Vec::new(), Vec::new()),
        |(body, legacy), record| {
        let file_name = db.resolve_string(record.file_name_offset);
        let out_path = output.join(change_extension(file_name, ext));
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if body.capacity() > 8 * 1024 * 1024 {
            *body = Vec::new();
        } else {
            body.clear();
        }
        match format {
            DiffFormat::Json => {
                starbreaker_datacore::export::write_json(db, record, &mut *body)?;
                std::fs::write(out_path, &*body)?;
            }
            DiffFormat::Xml => {
                starbreaker_datacore::export::write_classic_xml(db, record, &mut *body)?;
                legacy_xml_bytes_into(body, legacy);
                std::fs::write(out_path, &*legacy)?;
            }
        }
        Ok::<_, CliError>(())
    })?;
    Ok(())
}

fn join_p4k_dir(output: &Path, dir: &str) -> PathBuf {
    let mut path = output.to_path_buf();
    for component in dir.split('\\') {
        path.push(component);
    }
    path
}

fn merge_p4k_dirs(left: &mut HashMap<String, Vec<P4kEntry>>, right: HashMap<String, Vec<P4kEntry>>) {
    for (dir, mut files) in right {
        left.entry(dir).or_default().append(&mut files);
    }
}

fn export_datacore_types(db: &Database<'_>, output: &Path, format: &DiffFormat) -> Result<()> {
    let children = struct_children(db);
    let roots: Vec<usize> = db
        .struct_defs()
        .iter()
        .enumerate()
        .filter_map(|(index, def)| (def.parent_type_index == -1).then_some(index))
        .collect();

    for root in roots {
        export_type_node(db, output, &children, root, format)?;
    }
    Ok(())
}

fn export_type_node(
    db: &Database<'_>,
    current_dir: &Path,
    children: &[Vec<usize>],
    index: usize,
    format: &DiffFormat,
) -> Result<()> {
    let def = &db.struct_defs()[index];
    let name = db.resolve_string2(def.name_offset);
    let node_dir = if children[index].is_empty() {
        current_dir.to_path_buf()
    } else {
        current_dir.join(name)
    };
    std::fs::create_dir_all(&node_dir)?;
    let out_path = node_dir.join(format!("{name}.{}", format.extension()));
    write_type_report(db, index, &out_path, format)?;

    for &child in &children[index] {
        export_type_node(db, &node_dir, children, child, format)?;
    }
    Ok(())
}

fn write_type_report(db: &Database<'_>, index: usize, path: &Path, format: &DiffFormat) -> Result<()> {
    match format {
        DiffFormat::Json => {
            let def = &db.struct_defs()[index];
            let mut report = serde_json::Map::new();
            report.insert(
                "Name".to_string(),
                serde_json::Value::String(db.resolve_string2(def.name_offset).to_string()),
            );
            if def.parent_type_index != -1 {
                let parent = &db.struct_defs()[def.parent_type_index as usize];
                report.insert(
                    "Parent".to_string(),
                    serde_json::Value::String(db.resolve_string2(parent.name_offset).to_string()),
                );
            }
            let properties = properties_for_type(db, index)
                .into_iter()
                .map(|(name, type_name)| serde_json::json!({ "Name": name, "Type": type_name }))
                .collect::<Vec<_>>();
            report.insert("Properties".to_string(), serde_json::Value::Array(properties));
            let file = File::create(path)?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, &serde_json::Value::Object(report))?;
            writer.write_all(b"\n")?;
        }
        DiffFormat::Xml => {
            let def = &db.struct_defs()[index];
            let mut writer = Vec::new();
            write!(writer, "<Struct Name=\"")?;
            write_xml_escaped(&mut writer, db.resolve_string2(def.name_offset))?;
            write!(writer, "\"")?;
            if def.parent_type_index != -1 {
                let parent = &db.struct_defs()[def.parent_type_index as usize];
                write!(writer, " Parent=\"")?;
                write_xml_escaped(&mut writer, db.resolve_string2(parent.name_offset))?;
                write!(writer, "\"")?;
            }
            let properties = properties_for_type(db, index);
            if properties.is_empty() {
                write!(writer, " />")?;
            } else {
                writeln!(writer, ">")?;
                for (name, type_name) in properties {
                    write!(writer, "  <Property Name=\"")?;
                    write_xml_escaped(&mut writer, &name)?;
                    write!(writer, "\" Type=\"")?;
                    write_xml_escaped(&mut writer, &type_name)?;
                    writeln!(writer, "\" />")?;
                }
                write!(writer, "</Struct>")?;
            }
            std::fs::write(path, legacy_xml_bytes(writer))?;
        }
    }
    Ok(())
}

fn export_datacore_enums(db: &Database<'_>, output: &Path, format: &DiffFormat) -> Result<()> {
    std::fs::create_dir_all(output)?;
    (0..db.enum_defs().len()).into_par_iter().try_for_each(|index| {
        let enum_def = &db.enum_defs()[index];
        let name = db.resolve_string2(enum_def.name_offset);
        let values = db
            .enum_options(index as i32)
            .iter()
            .map(|value| db.resolve_string2(*value))
            .collect::<Vec<_>>();
        let path = output.join(format!("{name}.{}", format.extension()));
        match format {
            DiffFormat::Json => {
                let file = File::create(path)?;
                let mut writer = BufWriter::new(file);
                let report = serde_json::json!({ "Name": name, "Values": values });
                serde_json::to_writer_pretty(&mut writer, &report)?;
                writer.write_all(b"\n")?;
            }
            DiffFormat::Xml => {
                let mut writer = Vec::new();
                write!(writer, "<Enum Name=\"")?;
                write_xml_escaped(&mut writer, name)?;
                writeln!(writer, "\">")?;
                for value in values {
                    write!(writer, "  <Value>")?;
                    write_xml_escaped(&mut writer, value)?;
                    writeln!(writer, "</Value>")?;
                }
                write!(writer, "</Enum>")?;
                std::fs::write(path, legacy_xml_bytes(writer))?;
            }
        }
        Ok::<_, CliError>(())
    })?;
    Ok(())
}

fn struct_children(db: &Database<'_>) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); db.struct_defs().len()];
    for (index, def) in db.struct_defs().iter().enumerate() {
        if def.parent_type_index != -1 {
            children[def.parent_type_index as usize].push(index);
        }
    }
    for child_list in &mut children {
        child_list.sort_by_key(|&index| db.resolve_string2(db.struct_defs()[index].name_offset));
    }
    children
}

fn properties_for_type(db: &Database<'_>, index: usize) -> Vec<(String, String)> {
    db.all_properties(index as i32)
        .into_iter()
        .map(|prop| {
            let name = db.resolve_string2(prop.name_offset).to_string();
            let type_name = property_type_name(db, prop.data_type, prop.conversion_type, prop.struct_index);
            (name, type_name)
        })
        .collect()
}

fn property_type_name(
    db: &Database<'_>,
    data_type: u16,
    conversion_type: u16,
    struct_index: u16,
) -> String {
    let scalar = match DataType::try_from(data_type) {
        Ok(DataType::Boolean) => "bool".to_string(),
        Ok(DataType::Byte) => "byte".to_string(),
        Ok(DataType::SByte) => "sbyte".to_string(),
        Ok(DataType::Int16) => "short".to_string(),
        Ok(DataType::UInt16) => "ushort".to_string(),
        Ok(DataType::Int32) => "int".to_string(),
        Ok(DataType::UInt32) => "uint".to_string(),
        Ok(DataType::Int64) => "long".to_string(),
        Ok(DataType::UInt64) => "ulong".to_string(),
        Ok(DataType::Single) => "float".to_string(),
        Ok(DataType::Double) => "double".to_string(),
        Ok(DataType::Guid) => "CigGuid".to_string(),
        Ok(DataType::Locale | DataType::String) => "string".to_string(),
        Ok(DataType::EnumChoice) => db
            .enum_defs()
            .get(struct_index as usize)
            .map(|def| db.resolve_string2(def.name_offset).to_string())
            .unwrap_or_else(|| format!("Enum#{struct_index}")),
        Ok(DataType::Reference | DataType::StrongPointer | DataType::WeakPointer | DataType::Class) => db
            .struct_defs()
            .get(struct_index as usize)
            .map(|def| db.resolve_string2(def.name_offset).to_string())
            .unwrap_or_else(|| format!("Struct#{struct_index}")),
        Err(_) => format!("Unknown({data_type:#06x})"),
    };

    match ConversionType::try_from(conversion_type) {
        Ok(ConversionType::Attribute) => scalar,
        Ok(_) => format!("{scalar}[]"),
        Err(_) => format!("{scalar}[conversion:{conversion_type:#06x}]"),
    }
}

fn split_entry_dir(name: &str) -> &str {
    name.rsplit_once('\\')
        .map(|(dir, _)| dir)
        .unwrap_or("Data")
}

fn split_entry_file_name(name: &str) -> &str {
    name.rsplit_once('\\')
        .map(|(_, file)| file)
        .unwrap_or(name)
}

fn change_extension(path: &str, ext: &str) -> String {
    match path.rfind('.') {
        Some(dot) => format!("{}.{ext}", &path[..dot]),
        None => format!("{path}.{ext}"),
    }
}

fn copy_build_manifest(game_dir: &Path, output: &Path) -> Result<()> {
    let source = game_dir.join("build_manifest.id");
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(output)?;
    std::fs::copy(source, output.join("build_manifest.json"))?;
    Ok(())
}

fn write_xml_escaped(writer: &mut impl Write, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        let escaped = match *byte {
            b'&' => Some(b"&amp;".as_slice()),
            b'<' => Some(b"&lt;".as_slice()),
            b'>' => Some(b"&gt;".as_slice()),
            b'"' => Some(b"&quot;".as_slice()),
            _ => None,
        };
        if let Some(escaped) = escaped {
            if start < index {
                writer.write_all(&bytes[start..index])?;
            }
            writer.write_all(escaped)?;
            start = index + 1;
        }
    }
    if start < bytes.len() {
        writer.write_all(&bytes[start..])?;
    }
    Ok(())
}

fn legacy_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn legacy_xml_bytes(bytes: Vec<u8>) -> Vec<u8> {
    let mut output = Vec::new();
    legacy_xml_bytes_into(&bytes, &mut output);
    output
}

fn legacy_xml_bytes_into(bytes: &[u8], output: &mut Vec<u8>) {
    let text = legacy_xml_body(bytes);
    output.clear();
    output.reserve(3 + 38 + 2 + text.len());
    write_legacy_xml(output, text).expect("writing to Vec cannot fail");
}

fn legacy_xml_body(mut text: &[u8]) -> &[u8] {
    const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    const DECL: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>";

    if let Some(stripped) = text.strip_prefix(BOM) {
        text = stripped;
    }
    if let Some(stripped) = text.strip_prefix(DECL) {
        text = stripped;
        while matches!(text.first(), Some(b'\r' | b'\n')) {
            text = &text[1..];
        }
    }

    while matches!(text.last(), Some(b'\r' | b'\n')) {
        text = &text[..text.len() - 1];
    }

    text
}

fn write_legacy_xml(mut writer: impl Write, text: &[u8]) -> Result<()> {
    const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    const DECL: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>";

    writer.write_all(BOM)?;
    writer.write_all(DECL)?;
    writer.write_all(b"\r\n")?;

    let mut index = 0;
    let mut chunk_start = 0;
    while index < text.len() {
        match text[index] {
            b'\r' => {
                if chunk_start < index {
                    writer.write_all(&text[chunk_start..index])?;
                }
                writer.write_all(b"\r\n")?;
                index += 1;
                if matches!(text.get(index), Some(b'\n')) {
                    index += 1;
                }
                chunk_start = index;
            }
            b'\n' => {
                if chunk_start < index {
                    writer.write_all(&text[chunk_start..index])?;
                }
                writer.write_all(b"\r\n")?;
                index += 1;
                chunk_start = index;
            }
            _ => index += 1,
        }
    }
    if chunk_start < text.len() {
        writer.write_all(&text[chunk_start..])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{legacy_bool, legacy_xml_bytes, sort_p4k_files, write_p4k_dir_xml};
    use starbreaker_p4k::P4kEntry;

    #[test]
    fn legacy_xml_bytes_adds_bom_crlf_declaration_and_no_terminal_newline() {
        let output = legacy_xml_bytes(b"<Root>\n  <Value />\n</Root>\n".to_vec());
        assert!(output.starts_with(b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n"));
        assert!(output.windows(2).any(|pair| pair == b"\r\n"));
        assert!(!output.ends_with(b"\r\n"));
        assert!(!output.windows(2).any(|pair| pair == b"\n\n"));
    }

    #[test]
    fn p4k_diff_files_are_sorted_by_file_name() {
        let entries = [
            entry("Animations\\pu_dialog_events_vanduul.xml"),
            entry("Animations\\DirectionalBlends.img"),
            entry("Animations\\pu_dialog_events_male.xml"),
            entry("Animations\\pu_dialog_events_female.xml"),
            entry("Animations\\Animations.img"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "Animations\\Animations.img",
                "Animations\\DirectionalBlends.img",
                "Animations\\pu_dialog_events_female.xml",
                "Animations\\pu_dialog_events_male.xml",
                "Animations\\pu_dialog_events_vanduul.xml",
            ]
        );
    }

    #[test]
    fn p4k_diff_files_use_legacy_culture_ordering() {
        let entries = [
            entry("Data\\bspace\\1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Run.comb"),
            entry("Data\\bspace\\1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Run_Left.bspace"),
            entry("Data\\bspace\\1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Run_Right.bspace"),
            entry("Data\\bspace\\1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Walk.comb"),
            entry("Data\\bspace\\1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Walk_Left.bspace"),
            entry("Data\\bspace\\1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Walk_Right.bspace"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.rsplit('\\').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Run_Left.bspace",
                "1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Run_Right.bspace",
                "1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Run.comb",
                "1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Walk_Left.bspace",
                "1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Walk_Right.bspace",
                "1D-BSpace_AI_Quazigrazer_Stand_Relaxed_Idle2Walk.comb",
            ]
        );
    }

    #[test]
    fn p4k_diff_files_sort_suffixes_like_old_csharp_dump() {
        let entries = [
            entry("Data\\cockpit\\cockpit_scythe_gloc_passout.caf"),
            entry("Data\\cockpit\\cockpit_scythe_gloc_passout_idle.caf"),
            entry("Data\\cockpit\\cockpit_scythe_gloc_passout_idle_downpitch_add.caf"),
            entry("Data\\cockpit\\cockpit_scythe_gloc_passout_idle_leftbank_add.caf"),
            entry("Data\\cockpit\\cockpit_scythe_gloc_passout_idle_rightbank_add.caf"),
            entry("Data\\cockpit\\cockpit_scythe_gloc_passout_idle_uppitch_add.caf"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.rsplit('\\').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "cockpit_scythe_gloc_passout_idle_downpitch_add.caf",
                "cockpit_scythe_gloc_passout_idle_leftbank_add.caf",
                "cockpit_scythe_gloc_passout_idle_rightbank_add.caf",
                "cockpit_scythe_gloc_passout_idle_uppitch_add.caf",
                "cockpit_scythe_gloc_passout_idle.caf",
                "cockpit_scythe_gloc_passout.caf",
            ]
        );
    }

    #[test]
    fn p4k_diff_files_sort_extension_before_numeric_suffix() {
        let entries = [
            entry("Data\\bspace\\1D-BSpace_AI_Vlk_Alerted_Stand_TurnOnSpot.comb"),
            entry("Data\\bspace\\1D-BSpace_AI_Vlk_Alerted_Stand_TurnOnSpot090.bspace"),
            entry("Data\\bspace\\1D-BSpace_AI_Vlk_Alerted_Stand_TurnOnSpot180.bspace"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.rsplit('\\').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "1D-BSpace_AI_Vlk_Alerted_Stand_TurnOnSpot.comb",
                "1D-BSpace_AI_Vlk_Alerted_Stand_TurnOnSpot090.bspace",
                "1D-BSpace_AI_Vlk_Alerted_Stand_TurnOnSpot180.bspace",
            ]
        );
    }

    #[test]
    fn p4k_diff_files_sort_case_insensitively_like_legacy_dump() {
        let entries = [
            entry("Data\\Materials\\CloudsSwirl_1.mtl"),
            entry("Data\\Materials\\Datapad_GreenGlow.mtl"),
            entry("Data\\Materials\\NoFriction.mtl"),
            entry("Data\\Materials\\TESTflat.mtl"),
            entry("Data\\Materials\\alpha_white.mtl"),
            entry("Data\\Materials\\alpha.mtl"),
            entry("Data\\Materials\\beam_test.mtl"),
            entry("Data\\Materials\\chrome_sphere.mtl"),
            entry("Data\\Materials\\chrome_sphere1.mtl"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.rsplit('\\').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "alpha_white.mtl",
                "alpha.mtl",
                "beam_test.mtl",
                "chrome_sphere.mtl",
                "chrome_sphere1.mtl",
                "CloudsSwirl_1.mtl",
                "Datapad_GreenGlow.mtl",
                "NoFriction.mtl",
                "TESTflat.mtl",
            ]
        );
    }

    #[test]
    fn p4k_diff_files_sort_uppercase_by_text_position_not_ascii_position() {
        let entries = [
            entry("Data\\Materials\\terra_atrium_canopy.mtl"),
            entry("Data\\Materials\\terra_atrium_ext.mtl"),
            entry("Data\\Materials\\Terra_Backdrop_ext.mtl"),
            entry("Data\\Materials\\terra_turings_facade_waterfall.mtl"),
            entry("Data\\Materials\\terra_turings_facade.mtl"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.rsplit('\\').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "terra_atrium_canopy.mtl",
                "terra_atrium_ext.mtl",
                "Terra_Backdrop_ext.mtl",
                "terra_turings_facade_waterfall.mtl",
                "terra_turings_facade.mtl",
            ]
        );
    }

    #[test]
    fn p4k_diff_files_sort_space_before_underscore_continuation() {
        let entries = [
            entry("Data\\Objects\\console_info_banu_2_a_lod1.cgf"),
            entry("Data\\Objects\\console_info_banu_2_a_lod1.cgfm"),
            entry("Data\\Objects\\console_info_banu_2_a.cgf"),
            entry("Data\\Objects\\console_info_banu_2_a.cgfm"),
            entry("Data\\Objects\\console_info_banu_2_a.meshsetup"),
            entry("Data\\Objects\\console_info_banu_2_a .mtl"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.rsplit('\\').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "console_info_banu_2_a .mtl",
                "console_info_banu_2_a_lod1.cgf",
                "console_info_banu_2_a_lod1.cgfm",
                "console_info_banu_2_a.cgf",
                "console_info_banu_2_a.cgfm",
                "console_info_banu_2_a.meshsetup",
            ]
        );
    }

    #[test]
    fn p4k_diff_files_sort_parenthesis_before_digits() {
        let entries = [
            entry("Data\\Objects\\area_18_sign_01_01.cgf"),
            entry("Data\\Objects\\area_18_sign_(_screen_01.cgf"),
        ];
        let mut files = entries.to_vec();

        sort_p4k_files(&mut files);

        let names = files
            .iter()
            .map(|entry| entry.name.rsplit('\\').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "area_18_sign_(_screen_01.cgf",
                "area_18_sign_01_01.cgf",
            ]
        );
    }

    #[test]
    fn p4k_diff_xml_uses_legacy_bool_casing() {
        assert_eq!(legacy_bool(true), "True");
        assert_eq!(legacy_bool(false), "False");

        let temp = std::env::temp_dir().join(format!(
            "starbreaker-p4k-bool-{}.xml",
            std::process::id()
        ));
        let entries = [entry("Data\\foo.bin")];

        write_p4k_dir_xml(&temp, "Data", &[entries[0].clone()]).unwrap();
        let text = std::fs::read_to_string(&temp).unwrap();
        let _ = std::fs::remove_file(temp);

        assert!(text.contains("Encrypted=\"False\""));
        assert!(!text.contains("Encrypted=\"false\""));
    }

    #[test]
    fn p4k_diff_xml_leaves_apostrophes_unescaped_like_legacy_dump() {
        let temp = std::env::temp_dir().join(format!(
            "starbreaker-p4k-apostrophe-{}.xml",
            std::process::id()
        ));
        let entries = [entry("Data\\levski_mural_'_01_diff.dds")];

        write_p4k_dir_xml(&temp, "Data", &[entries[0].clone()]).unwrap();
        let text = std::fs::read_to_string(&temp).unwrap();
        let _ = std::fs::remove_file(temp);

        assert!(text.contains("Name=\"levski_mural_'_01_diff.dds\""));
        assert!(!text.contains("&apos;"));
    }

    fn entry(name: &str) -> P4kEntry {
        P4kEntry {
            name: name.to_string(),
            compressed_size: 0,
            uncompressed_size: 1,
            compression_method: 0,
            is_encrypted: false,
            offset: 0,
            crc32: 0,
            last_modified: 0,
        }
    }
}
