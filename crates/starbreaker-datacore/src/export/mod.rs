pub mod json;
pub mod unp4k_xml;
pub mod xml;

use crate::database::Database;
use crate::error::ExportError;
use crate::types::Record;

/// Export a record to JSON bytes (pretty-printed).
pub fn to_json(db: &Database, record: &Record) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    write_json(db, record, &mut buf)?;
    Ok(buf)
}

/// Export a record to compact (non-indented) JSON bytes.
pub fn to_json_compact(db: &Database, record: &Record) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    write_json_compact(db, record, &mut buf)?;
    Ok(buf)
}

/// Export a record as JSON to an arbitrary writer (pretty-printed).
pub fn write_json(
    db: &Database,
    record: &Record,
    w: impl std::io::Write,
) -> Result<(), ExportError> {
    let mut sink = json::JsonSink::new(w, true);
    crate::walker::walk_record(db, record, &mut sink)?;
    Ok(())
}

/// Export a record as compact JSON to an arbitrary writer.
pub fn write_json_compact(
    db: &Database,
    record: &Record,
    w: impl std::io::Write,
) -> Result<(), ExportError> {
    let mut sink = json::JsonSink::new(w, false);
    crate::walker::walk_record(db, record, &mut sink)?;
    Ok(())
}

/// Export a record to unp4k-compatible XML bytes.
pub fn to_unp4k_xml(db: &Database, record: &Record) -> Result<Vec<u8>, ExportError> {
    unp4k_xml::to_unp4k_xml(db, record)
}

/// Export a record to XML bytes (pretty-printed).
pub fn to_xml(db: &Database, record: &Record) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    write_xml(db, record, &mut buf)?;
    Ok(buf)
}

/// Export a record as XML to an arbitrary writer (pretty-printed).
pub fn write_xml(
    db: &Database,
    record: &Record,
    w: impl std::io::Write,
) -> Result<(), ExportError> {
    let mut sink = xml::XmlSink::new(w, true);
    crate::walker::walk_record(db, record, &mut sink)?;
    Ok(())
}
