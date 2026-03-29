use std::io::Read;

use crate::chf_data::ChfData;
use crate::error::ChfError;
use starbreaker_common::{ParseError, SpanReader};

const CHF_SIZE: usize = 4096;
const MAGIC: u16 = 0x4242;
const MODDED_MARKER: &[u8; 8] = b"diogotr7";

/// A CHF container file. Always exactly 4096 bytes on disk.
///
/// The 16-byte header layout is:
///   `[u16 magic=0x4242] [u16 flags] [u32 crc32c] [u32 compressed_size] [u32 decompressed_size]`
/// followed by zstd-compressed data (level 1).
pub struct ChfFile {
    /// Decompressed CHF binary data.
    pub data: Vec<u8>,
    /// Bytes 2-3 of the header (flags/version field). Purpose unknown but preserved for round-trip.
    /// Written as part of the header word alongside the magic: `CONCAT22(flags, 0x4242)`.
    pub unknown: [u8; 2],
    /// True only if the last 8 bytes of the container are the modded marker.
    pub is_modded: bool,
}

impl ChfFile {
    /// Parse a 4096-byte CHF container.
    pub fn from_chf(bytes: &[u8]) -> Result<Self, ChfError> {
        if bytes.len() != CHF_SIZE {
            return Err(ChfError::InvalidContainer(format!(
                "expected {} bytes, got {}",
                CHF_SIZE,
                bytes.len()
            )));
        }

        // Header: u16 magic, 2 unknown bytes, u32 crc, u32 compressed_size, u32 decompressed_size
        let magic = u16::from_le_bytes([bytes[0], bytes[1]]);
        if magic != MAGIC {
            return Err(ChfError::InvalidContainer(format!(
                "bad magic: expected {MAGIC:#06x}, got {magic:#06x}"
            )));
        }

        let unknown = [bytes[2], bytes[3]];
        let mut reader = SpanReader::new_at(bytes, 4);
        let expected_crc = reader.read_u32()?;
        let compressed_size = reader.read_u32()? as usize;
        let _decompressed_size = reader.read_u32()? as usize;

        // Validate CRC32C over bytes[16..4096]
        let actual_crc = crc32c::crc32c(&bytes[16..CHF_SIZE]);
        if actual_crc != expected_crc {
            return Err(ChfError::CrcMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        // Decompress zstd data
        let compressed = &bytes[16..16 + compressed_size];
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(std::io::Cursor::new(compressed))
            .map_err(|e| ChfError::Compression(e.to_string()))?;
        let mut data = Vec::new();
        decoder
            .read_to_end(&mut data)
            .map_err(|e| ChfError::Compression(e.to_string()))?;

        // Check modded marker at last 8 bytes
        let tail = &bytes[CHF_SIZE - 8..CHF_SIZE];
        let is_modded = tail == MODDED_MARKER || tail == [0u8; 8];

        Ok(ChfFile {
            data,
            unknown,
            is_modded,
        })
    }

    /// Re-pack into a 4096-byte CHF container.
    pub fn to_chf(&self) -> Result<Vec<u8>, ChfError> {
        let compressed = ruzstd::encoding::compress_to_vec(
            std::io::Cursor::new(&self.data),
            ruzstd::encoding::CompressionLevel::Fastest,
        );

        if 16 + compressed.len() > CHF_SIZE {
            return Err(ChfError::Compression(format!(
                "compressed data ({} bytes) too large for {} byte container (max payload: {})",
                compressed.len(),
                CHF_SIZE,
                CHF_SIZE - 16
            )));
        }

        let mut output = vec![0u8; CHF_SIZE];

        // Write header
        output[0..2].copy_from_slice(&MAGIC.to_le_bytes());
        output[2..4].copy_from_slice(&self.unknown);
        // CRC placeholder at [4..8] -- filled below
        output[8..12].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
        output[12..16].copy_from_slice(&(self.data.len() as u32).to_le_bytes());

        // Write compressed data
        output[16..16 + compressed.len()].copy_from_slice(&compressed);

        // Write modded marker if applicable
        if self.is_modded {
            let marker_start = CHF_SIZE - MODDED_MARKER.len();
            output[marker_start..CHF_SIZE].copy_from_slice(MODDED_MARKER);
        }

        // Compute and write CRC32C over bytes[16..4096]
        let crc = crc32c::crc32c(&output[16..CHF_SIZE]);
        output[4..8].copy_from_slice(&crc.to_le_bytes());

        Ok(output)
    }

    /// Wrap raw decompressed bytes directly (e.g. from a .bin file).
    pub fn from_bin(data: Vec<u8>) -> Self {
        ChfFile {
            data,
            unknown: [0; 2],
            is_modded: true,
        }
    }

    /// Parse the decompressed data into structured ChfData.
    pub fn parse(&self) -> Result<ChfData, ParseError> {
        ChfData::read(&self.data)
    }
}
