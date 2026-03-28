use starbreaker_common::SpanReader;

use crate::error::WemError;

/// Wwise codec IDs found in the fmt chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WemCodec {
    /// Standard PCM (0x0001)
    Pcm,
    /// Wwise IMA ADPCM (0x0002)
    Adpcm,
    /// Wwise Vorbis (0xFFFF) — most common on PC
    Vorbis,
    /// Wwise Opus (0x3041)
    Opus,
    /// Platinum Games ADPCM (0x8311)
    PtAdpcm,
    /// Unknown codec
    Unknown(u16),
}

impl WemCodec {
    pub fn from_id(id: u16) -> Self {
        match id {
            0x0001 | 0xFFFE => Self::Pcm,
            0x0002 => Self::Adpcm,
            0x0069 => Self::Adpcm, // older IMA variant
            0x3041 => Self::Opus,
            0x8311 => Self::PtAdpcm,
            0xFFFF => Self::Vorbis,
            other => Self::Unknown(other),
        }
    }

    /// Human-readable name for display.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pcm => "PCM",
            Self::Adpcm => "IMA ADPCM",
            Self::Vorbis => "Vorbis",
            Self::Opus => "Opus",
            Self::PtAdpcm => "PTADPCM",
            Self::Unknown(_) => "Unknown",
        }
    }
}

impl std::fmt::Display for WemCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "Unknown({id:#06x})"),
            other => f.write_str(other.name()),
        }
    }
}

/// Parsed fmt chunk data.
#[derive(Debug, Clone)]
pub struct WemFormat {
    pub codec: WemCodec,
    pub channels: u16,
    pub sample_rate: u32,
    pub avg_bytes_per_sec: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    /// Extra format bytes (codec-specific).
    pub extra_size: u16,
}

impl WemFormat {
    /// Parse from the raw bytes of a `fmt ` chunk.
    pub fn parse(data: &[u8]) -> Result<Self, WemError> {
        let mut reader = SpanReader::new(data);

        let codec_id = reader.read_u16()?;
        let channels = reader.read_u16()?;
        let sample_rate = reader.read_u32()?;
        let avg_bytes_per_sec = reader.read_u32()?;
        let block_align = reader.read_u16()?;
        let bits_per_sample = reader.read_u16()?;

        let extra_size = if reader.remaining() >= 2 {
            reader.read_u16()?
        } else {
            0
        };

        Ok(WemFormat {
            codec: WemCodec::from_id(codec_id),
            channels,
            sample_rate,
            avg_bytes_per_sec,
            block_align,
            bits_per_sample,
            extra_size,
        })
    }
}
