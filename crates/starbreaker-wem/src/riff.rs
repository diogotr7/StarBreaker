use starbreaker_common::SpanReader;

use crate::error::WemError;

/// RIFF = little-endian, RIFX = big-endian
const RIFF_MAGIC: u32 = 0x46464952; // "RIFF" as LE u32
const RIFX_MAGIC: u32 = 0x52494658; // "RIFX" as LE u32 (bytes R,I,F,X)
const WAVE_MAGIC: u32 = 0x45564157; // "WAVE" as LE u32

/// A parsed RIFF chunk header.
#[derive(Debug, Clone, Copy)]
pub struct RiffChunk {
    /// 4-byte ASCII tag as a little-endian u32.
    pub tag: u32,
    /// Offset of the chunk data (after the 8-byte header) in the original buffer.
    pub data_offset: usize,
    /// Size of the chunk data in bytes.
    pub data_size: usize,
}

impl RiffChunk {
    /// Return the 4-byte tag as an ASCII string (for display).
    pub fn tag_str(&self) -> [u8; 4] {
        self.tag.to_le_bytes()
    }
}

/// Parsed RIFF container with chunk index.
#[derive(Debug)]
pub struct RiffFile<'a> {
    data: &'a [u8],
    pub big_endian: bool,
    pub chunks: Vec<RiffChunk>,
}

impl<'a> RiffFile<'a> {
    /// Parse a RIFF/RIFX WAVE container, indexing all top-level chunks.
    pub fn parse(data: &'a [u8]) -> Result<Self, WemError> {
        let mut reader = SpanReader::new(data);

        let magic = reader.read_u32()?;
        let big_endian = match magic {
            RIFF_MAGIC => false,
            RIFX_MAGIC => true,
            other => return Err(WemError::InvalidRiffMagic { got: other }),
        };

        let _file_size = reader.read_u32()?;

        let wave_magic = reader.read_u32()?;
        if wave_magic != WAVE_MAGIC {
            return Err(WemError::InvalidWaveMagic { got: wave_magic });
        }

        let mut chunks = Vec::new();
        while reader.remaining() >= 8 {
            let tag = reader.read_u32()?;
            let size = reader.read_u32()? as usize;
            let data_offset = reader.position();

            chunks.push(RiffChunk {
                tag,
                data_offset,
                data_size: size,
            });

            // Advance past chunk data, with WORD alignment padding
            let padded = (size + 1) & !1;
            let skip = padded.min(reader.remaining());
            reader.advance(skip)?;
        }

        Ok(RiffFile {
            data,
            big_endian,
            chunks,
        })
    }

    /// Find a chunk by its 4-byte ASCII tag (e.g., b"fmt ").
    pub fn find_chunk(&self, tag: &[u8; 4]) -> Option<&RiffChunk> {
        let tag_u32 = u32::from_le_bytes(*tag);
        self.chunks.iter().find(|c| c.tag == tag_u32)
    }

    /// Get the raw data for a chunk.
    pub fn chunk_data(&self, chunk: &RiffChunk) -> &'a [u8] {
        &self.data[chunk.data_offset..chunk.data_offset + chunk.data_size]
    }
}
