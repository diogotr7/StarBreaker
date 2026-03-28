use starbreaker_common::SpanReader;

use crate::error::BnkError;

// Star Citizen bank version as of 2026-03-27: 154
// This determines which HIRC field layouts to use in Phase 3.

/// Parsed Bank Header (BKHD section).
#[derive(Debug, Clone)]
pub struct BankHeader {
    /// Wwise SDK version that generated this bank.
    pub version: u32,
    /// Unique bank ID.
    pub bank_id: u32,
    /// Total size of the BKHD section (including version and bank_id).
    pub section_size: u32,
}

impl BankHeader {
    pub fn parse(data: &[u8], section_size: u32) -> Result<Self, BnkError> {
        let mut reader = SpanReader::new(data);
        let version = reader.read_u32()?;
        let bank_id = reader.read_u32()?;
        // Remaining BKHD bytes are version-dependent; skip for now.
        Ok(BankHeader {
            version,
            bank_id,
            section_size,
        })
    }
}
