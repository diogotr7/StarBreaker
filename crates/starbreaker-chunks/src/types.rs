use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

// ── IVO format (Star Citizen) ────────────────────────────────────────────────

/// Raw on-disk header for an IVO chunk file (16 bytes).
///
/// Layout: magic(u32) + version(u32) + chunk_count(u32) + chunk_table_offset(u32)
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct IvoHeader {
    pub magic: u32,
    pub version: u32,
    pub chunk_count: u32,
    pub chunk_table_offset: u32,
}

/// Raw on-disk chunk table entry for an IVO file (16 bytes).
///
/// Layout: chunk_type(u32) + version(u32) + offset(u64)
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct IvoChunkTableEntry {
    pub chunk_type: u32,
    pub version: u32,
    pub offset: u64,
}

// ── CrCh format (Legacy CryEngine) ──────────────────────────────────────────

/// Raw on-disk header for a CrCh chunk file (16 bytes).
///
/// Layout: magic(u32) + version(u32) + chunk_count(u32) + chunk_table_offset(u32)
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct CrChHeader {
    pub magic: u32,
    pub version: u32,
    pub chunk_count: u32,
    pub chunk_table_offset: u32,
}

/// Raw on-disk chunk table entry for a CrCh file (16 bytes).
///
/// Layout: chunk_type(u16) + version_raw(u16) + id(i32) + size(u32) + offset(u32)
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct CrChChunkTableEntry {
    pub chunk_type: u16,
    pub version_raw: u16,
    pub id: i32,
    pub size: u32,
    pub offset: u32,
}

impl CrChChunkTableEntry {
    /// Whether the chunk data is stored in big-endian byte order (bit 15 of version_raw).
    pub fn is_big_endian(&self) -> bool {
        (self.version_raw & 0x8000) != 0
    }

    /// The chunk version with the big-endian flag stripped.
    pub fn version(&self) -> u16 {
        self.version_raw & 0x7FFF
    }
}
