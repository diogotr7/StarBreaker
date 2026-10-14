//! The combined unp4k document: classic unforge's `Game2.xml`.
//!
//! Besides one file per record path, classic unforge (before its 2025
//! rewrite) wrote the whole DataCore next to the `.dcb` as one XML
//! document: a `<DataForge>` root holding every record — main and non-main
//! alike — in record-table order, each as its unp4k element (record name,
//! `__type`, `__ref`, `__path`). Tools built on unp4k read records from it
//! that the per-file output cannot hold (several records share one path).
//!
//! [`write_unp4k_combined`]: the document for the given records, each
//! written as [`super::unp4k_xml::to_unp4k_xml`] writes it. Records are
//! rendered in parallel chunks and written in order, so the document is
//! streamed, never held whole in memory.

use std::io::Write;

use rayon::prelude::*;

use crate::database::Database;
use crate::error::ExportError;
use crate::types::Record;

/// Records rendered per parallel batch.
const CHUNK: usize = 1024;

/// Write the combined unp4k document for `records` to `w` (see the module docs).
pub fn write_unp4k_combined(db: &Database, records: &[&Record], mut w: impl Write) -> Result<(), ExportError> {
    w.write_all(b"<DataForge>\n")?;
    for chunk in records.chunks(CHUNK) {
        let rendered: Vec<Vec<u8>> = chunk
            .par_iter()
            .map(|record| super::unp4k_xml::to_unp4k_xml(db, record))
            .collect::<Result<_, _>>()?;
        for xml in rendered {
            w.write_all(&xml)?;
            w.write_all(b"\n")?;
        }
    }
    w.write_all(b"</DataForge>\n")?;
    Ok(())
}
