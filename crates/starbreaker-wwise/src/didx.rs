use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use starbreaker_common::SpanReader;

use crate::error::BnkError;

/// A single entry in the Data Index (DIDX) section.
/// Points to a WEM blob inside the DATA section.
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct DataIndexEntry {
    /// WEM file ID.
    pub id: u32,
    /// Byte offset within the DATA section.
    pub offset: u32,
    /// Size of the WEM data in bytes.
    pub size: u32,
}

/// Parse the DIDX section into a vec of entries.
pub fn parse_didx(data: &[u8]) -> Result<Vec<DataIndexEntry>, BnkError> {
    let mut reader = SpanReader::new(data);
    let entry_count = data.len() / std::mem::size_of::<DataIndexEntry>();
    let entries = reader.read_slice::<DataIndexEntry>(entry_count)?;
    Ok(entries.to_vec())
}
