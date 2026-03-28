use std::fmt;

use crate::error::ChunkFileError;
use crate::types::{CrChChunkTableEntry, IvoChunkTableEntry};
use starbreaker_common::SpanReader;

// ── Magic constants ──────────────────────────────────────────────────────────

const IVO_MAGIC: u32 = 0x6F766923; // "#ivo" as little-endian u32
const CRCH_MAGIC: u32 = 0x68437243; // "CrCh" as little-endian u32

const IVO_VERSION: u32 = 0x900;
const CRCH_VERSION: u32 = 0x746;

// ── Public entry types ───────────────────────────────────────────────────────

/// A parsed chunk entry from an IVO file, with a computed size.
#[derive(Debug, Clone)]
pub struct IvoChunkEntry {
    /// Hash-based chunk type identifier.
    pub chunk_type: u32,
    /// Chunk version number.
    pub version: u32,
    /// Byte offset of the chunk data within the file.
    pub offset: u64,
    /// Computed size in bytes (gap to next chunk or end-of-file).
    pub size: usize,
}

/// A parsed chunk entry from a CrCh file.
#[derive(Debug, Clone)]
pub struct CrChChunkEntry {
    /// Numeric chunk type identifier.
    pub chunk_type: u16,
    /// Chunk version (bit 15 = big-endian flag already stripped).
    pub version: u16,
    /// Whether this chunk is stored in big-endian byte order.
    pub big_endian: bool,
    /// Chunk ID.
    pub id: i32,
    /// Size in bytes.
    pub size: u32,
    /// Byte offset of the chunk data within the file.
    pub offset: u32,
}

// ── Parsed file containers ───────────────────────────────────────────────────

/// A parsed IVO chunk file.
pub struct IvoChunkFile<'a> {
    data: &'a [u8],
    chunks: Vec<IvoChunkEntry>,
}

impl fmt::Debug for IvoChunkFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IvoChunkFile")
            .field("data_len", &self.data.len())
            .field("chunks", &self.chunks)
            .finish()
    }
}

impl<'a> IvoChunkFile<'a> {
    /// The chunk entries in this file.
    pub fn chunks(&self) -> &[IvoChunkEntry] {
        &self.chunks
    }

    /// Returns the raw bytes of the given chunk entry.
    pub fn chunk_data(&self, entry: &IvoChunkEntry) -> &'a [u8] {
        let start = entry.offset as usize;
        let end = start + entry.size;
        &self.data[start..end]
    }
}

/// A parsed CrCh chunk file.
pub struct CrChChunkFile<'a> {
    data: &'a [u8],
    chunks: Vec<CrChChunkEntry>,
}

impl fmt::Debug for CrChChunkFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CrChChunkFile")
            .field("data_len", &self.data.len())
            .field("chunks", &self.chunks)
            .finish()
    }
}

impl<'a> CrChChunkFile<'a> {
    /// The chunk entries in this file.
    pub fn chunks(&self) -> &[CrChChunkEntry] {
        &self.chunks
    }

    /// Returns the raw bytes of the given chunk entry.
    pub fn chunk_data(&self, entry: &CrChChunkEntry) -> &'a [u8] {
        let start = entry.offset as usize;
        let end = start + entry.size as usize;
        &self.data[start..end]
    }
}

// ── Top-level enum ───────────────────────────────────────────────────────────

/// A CryEngine chunk file, either IVO or CrCh format.
#[derive(Debug)]
pub enum ChunkFile<'a> {
    Ivo(IvoChunkFile<'a>),
    CrCh(CrChChunkFile<'a>),
}

impl<'a> ChunkFile<'a> {
    /// Parse a chunk file from raw bytes, auto-detecting the format from the
    /// magic number.
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, ChunkFileError> {
        let mut reader = SpanReader::new(data);
        let magic = *reader.read_type::<u32>()?;

        match magic {
            IVO_MAGIC => parse_ivo(data, &mut reader),
            CRCH_MAGIC => parse_crch(data, &mut reader),
            _ => Err(ChunkFileError::UnrecognizedMagic(magic)),
        }
    }
}

// ── IVO parsing ──────────────────────────────────────────────────────────────

fn parse_ivo<'a>(
    data: &'a [u8],
    reader: &mut SpanReader<'a>,
) -> Result<ChunkFile<'a>, ChunkFileError> {
    // reader has already consumed magic (4 bytes)
    let version = *reader.read_type::<u32>()?;
    if version != IVO_VERSION {
        return Err(ChunkFileError::UnsupportedVersion(version));
    }

    let chunk_count = *reader.read_type::<u32>()?;
    let _chunk_table_offset = *reader.read_type::<u32>()?;

    // Read the raw chunk table entries
    let raw_entries = reader.read_slice::<IvoChunkTableEntry>(chunk_count as usize)?;

    // Build (index, offset) pairs sorted by offset so we can compute sizes
    let mut by_offset: Vec<(usize, u64)> = raw_entries
        .iter()
        .enumerate()
        .map(|(i, e)| (i, e.offset))
        .collect();
    by_offset.sort_by_key(|&(_, off)| off);

    // Compute sizes from gaps between consecutive offsets
    let file_len = data.len() as u64;
    let mut sizes = vec![0usize; raw_entries.len()];
    for (pos, &(idx, offset)) in by_offset.iter().enumerate() {
        let next_offset = if pos + 1 < by_offset.len() {
            by_offset[pos + 1].1
        } else {
            file_len
        };
        sizes[idx] = (next_offset - offset) as usize;
    }

    // Build the final entries in their original table order
    let chunks: Vec<IvoChunkEntry> = raw_entries
        .iter()
        .enumerate()
        .map(|(i, e)| IvoChunkEntry {
            chunk_type: e.chunk_type,
            version: e.version,
            offset: e.offset,
            size: sizes[i],
        })
        .collect();

    Ok(ChunkFile::Ivo(IvoChunkFile { data, chunks }))
}

// ── CrCh parsing ─────────────────────────────────────────────────────────────

fn parse_crch<'a>(
    data: &'a [u8],
    reader: &mut SpanReader<'a>,
) -> Result<ChunkFile<'a>, ChunkFileError> {
    // reader has already consumed magic (4 bytes)
    let version = *reader.read_type::<u32>()?;
    if version != CRCH_VERSION {
        return Err(ChunkFileError::UnsupportedVersion(version));
    }

    let chunk_count = *reader.read_type::<u32>()?;
    let _chunk_table_offset = *reader.read_type::<u32>()?;

    let raw_entries = reader.read_slice::<CrChChunkTableEntry>(chunk_count as usize)?;

    let chunks: Vec<CrChChunkEntry> = raw_entries
        .iter()
        .map(|e| CrChChunkEntry {
            chunk_type: e.chunk_type,
            version: e.version(),
            big_endian: e.is_big_endian(),
            id: e.id,
            size: e.size,
            offset: e.offset,
        })
        .collect();

    Ok(ChunkFile::CrCh(CrChChunkFile { data, chunks }))
}
