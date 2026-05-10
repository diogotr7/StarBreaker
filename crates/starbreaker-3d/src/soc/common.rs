//! Shared CrCh container parsing helpers used by every SOC submodule.
//!
//! A `.soc` file is a CrCh-format binary blob with a 16-byte header and a
//! chunk table. Each submodule (brushes, entities, visarea, scene) needs to
//! locate its chunks and read primitive types out of the same byte slice;
//! keeping the constants and byte-readers in one place ensures the modules
//! stay numerically consistent.

// ── CrCh container constants ────────────────────────────────────────────────

pub(super) const CRCH_MAGIC: [u8; 4] = *b"CrCh";
pub(super) const CRCH_VERSION: u32 = 0x0746;

pub(super) const CHUNK_TABLE_ENTRY_SIZE: usize = 16;

pub(super) const CHUNK_TYPE_CRYXMLB: u16 = 0x0004;
pub(super) const CHUNK_TYPE_VISAREA: u16 = 0x000E;
pub(super) const CHUNK_TYPE_STATOBJ: u16 = 0x0010;

#[derive(Debug, Clone, Copy)]
pub(super) struct CrChHeader {
    pub chunk_count: u32,
    pub chunk_table_offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ChunkEntry {
    pub chunk_type: u16,
    /// Chunk-level version field. Currently `1` in shipping builds even
    /// though brush records inside the chunk use the v15 layout, so it is
    /// treated as opaque and not used to gate parsing.
    #[allow(dead_code)]
    pub version: u16,
    pub offset: u32,
    pub size: u32,
}

// ── Byte readers ────────────────────────────────────────────────────────────

#[inline]
pub(super) fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

#[inline]
pub(super) fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ])
}

#[inline]
pub(super) fn read_i32(data: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ])
}

#[inline]
pub(super) fn read_f32(data: &[u8], off: usize) -> f32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[off..off + 4]);
    f32::from_le_bytes(buf)
}

#[inline]
pub(super) fn read_f64(data: &[u8], off: usize) -> f64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[off..off + 8]);
    f64::from_le_bytes(buf)
}

// ── Container header + chunk table ──────────────────────────────────────────

/// Result of inspecting a `.soc` file's CrCh container header. Every
/// submodule produces its own `Result` type, so this helper stays
/// error-agnostic: callers map the `Option` / range checks to their own
/// error variants.
pub(super) fn parse_crch_header(data: &[u8]) -> Option<CrChHeader> {
    if data.len() < 16 {
        return None;
    }
    if data[0..4] != CRCH_MAGIC {
        return None;
    }
    let version = read_u32(data, 4);
    if version != CRCH_VERSION {
        return None;
    }
    Some(CrChHeader {
        chunk_count: read_u32(data, 8),
        chunk_table_offset: read_u32(data, 12),
    })
}

/// Read every entry in the chunk table. Returns `None` when the table
/// extends past the end of the buffer.
pub(super) fn parse_chunk_table(data: &[u8], header: &CrChHeader) -> Option<Vec<ChunkEntry>> {
    let count = header.chunk_count as usize;
    let start = header.chunk_table_offset as usize;
    let bytes_needed = count.saturating_mul(CHUNK_TABLE_ENTRY_SIZE);
    let end = start.saturating_add(bytes_needed);
    if end > data.len() {
        return None;
    }

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = start + i * CHUNK_TABLE_ENTRY_SIZE;
        let chunk_type = read_u16(data, off);
        let version_raw = read_u16(data, off + 2);
        let version = version_raw & 0x7FFF;
        let _id = read_u32(data, off + 4);
        let size = read_u32(data, off + 8);
        let offset = read_u32(data, off + 12);
        out.push(ChunkEntry {
            chunk_type,
            version,
            offset,
            size,
        });
    }
    Some(out)
}

/// Borrow a chunk's body bytes from the file blob, clipped to file end.
pub(super) fn chunk_slice<'a>(data: &'a [u8], chunk: &ChunkEntry) -> &'a [u8] {
    let start = (chunk.offset as usize).min(data.len());
    let end = start
        .saturating_add(chunk.size as usize)
        .min(data.len());
    &data[start..end]
}
