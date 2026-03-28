pub mod decode;
pub mod error;
pub mod fmt;
pub mod riff;
pub mod vorbis;

pub use error::WemError;
pub use fmt::{WemCodec, WemFormat};

use riff::RiffFile;

/// A parsed WEM audio file.
#[derive(Debug)]
pub struct WemFile<'a> {
    riff: RiffFile<'a>,
    pub format: WemFormat,
    /// Raw bytes of the entire WEM file (needed for codec decode).
    raw: &'a [u8],
}

impl<'a> WemFile<'a> {
    /// Parse a WEM file from raw bytes.
    pub fn parse(data: &'a [u8]) -> Result<Self, WemError> {
        let riff = RiffFile::parse(data)?;

        let fmt_chunk = riff
            .find_chunk(b"fmt ")
            .ok_or_else(|| WemError::MissingChunk {
                tag: "fmt ".into(),
            })?;
        let format = WemFormat::parse(riff.chunk_data(fmt_chunk))?;

        Ok(WemFile {
            riff,
            format,
            raw: data,
        })
    }

    /// Quick codec detection without full parse.
    pub fn codec(data: &[u8]) -> Result<WemCodec, WemError> {
        let riff = RiffFile::parse(data)?;
        let fmt_chunk = riff
            .find_chunk(b"fmt ")
            .ok_or_else(|| WemError::MissingChunk {
                tag: "fmt ".into(),
            })?;
        let fmt_data = riff.chunk_data(fmt_chunk);
        if fmt_data.len() < 2 {
            return Err(WemError::MissingChunk {
                tag: "fmt (too short)".into(),
            });
        }
        Ok(WemCodec::from_id(u16::from_le_bytes([
            fmt_data[0],
            fmt_data[1],
        ])))
    }

    pub fn codec_type(&self) -> WemCodec {
        self.format.codec
    }

    pub fn sample_rate(&self) -> u32 {
        self.format.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.format.channels
    }

    /// Get the raw data chunk bytes.
    pub fn audio_data(&self) -> Option<&'a [u8]> {
        self.riff
            .find_chunk(b"data")
            .map(|c| self.riff.chunk_data(c))
    }

    /// Get the full raw WEM bytes (needed by ww2ogg which re-parses internally).
    pub fn raw_bytes(&self) -> &'a [u8] {
        self.raw
    }

    /// Estimated duration in seconds (from avg_bytes_per_sec).
    pub fn estimated_duration_secs(&self) -> Option<f64> {
        let data_size = self.audio_data().map(|d| d.len())?;
        if self.format.avg_bytes_per_sec == 0 {
            return None;
        }
        Some(data_size as f64 / self.format.avg_bytes_per_sec as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid RIFF WAVE with just a fmt chunk (Vorbis codec).
    fn minimal_vorbis_wem() -> Vec<u8> {
        let mut buf = Vec::new();
        // RIFF header
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&0u32.to_le_bytes()); // file size placeholder
        buf.extend_from_slice(b"WAVE");
        // fmt chunk
        buf.extend_from_slice(b"fmt ");
        let fmt_size: u32 = 18;
        buf.extend_from_slice(&fmt_size.to_le_bytes());
        buf.extend_from_slice(&0xFFFFu16.to_le_bytes()); // codec = Vorbis
        buf.extend_from_slice(&2u16.to_le_bytes()); // channels
        buf.extend_from_slice(&48000u32.to_le_bytes()); // sample rate
        buf.extend_from_slice(&24000u32.to_le_bytes()); // avg bytes/sec
        buf.extend_from_slice(&2048u16.to_le_bytes()); // block align
        buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        buf.extend_from_slice(&0u16.to_le_bytes()); // extra size
        // data chunk
        buf.extend_from_slice(b"data");
        let data_size: u32 = 8;
        buf.extend_from_slice(&data_size.to_le_bytes());
        buf.extend_from_slice(&[0u8; 8]); // dummy audio data
        // Patch file size
        let file_size = (buf.len() - 8) as u32;
        buf[4..8].copy_from_slice(&file_size.to_le_bytes());
        buf
    }

    #[test]
    fn test_parse_minimal_vorbis() {
        let data = minimal_vorbis_wem();
        let wem = WemFile::parse(&data).unwrap();
        assert_eq!(wem.codec_type(), WemCodec::Vorbis);
        assert_eq!(wem.channels(), 2);
        assert_eq!(wem.sample_rate(), 48000);
        assert_eq!(wem.audio_data().unwrap().len(), 8);
    }

    #[test]
    fn test_codec_detection() {
        let data = minimal_vorbis_wem();
        let codec = WemFile::codec(&data).unwrap();
        assert_eq!(codec, WemCodec::Vorbis);
    }

    #[test]
    fn test_codec_from_id() {
        assert_eq!(WemCodec::from_id(0xFFFF), WemCodec::Vorbis);
        assert_eq!(WemCodec::from_id(0x3041), WemCodec::Opus);
        assert_eq!(WemCodec::from_id(0x0001), WemCodec::Pcm);
        assert_eq!(WemCodec::from_id(0x8311), WemCodec::PtAdpcm);
        assert_eq!(WemCodec::from_id(0x1234), WemCodec::Unknown(0x1234));
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"NOT_RIFF_DATA_HERE";
        let err = WemFile::parse(data).unwrap_err();
        assert!(matches!(err, WemError::InvalidRiffMagic { .. }));
    }

    #[test]
    fn test_estimated_duration() {
        let data = minimal_vorbis_wem();
        let wem = WemFile::parse(&data).unwrap();
        let dur = wem.estimated_duration_secs().unwrap();
        // 8 bytes / 24000 bytes_per_sec ≈ 0.000333 seconds
        assert!(dur > 0.0 && dur < 0.001);
    }
}
