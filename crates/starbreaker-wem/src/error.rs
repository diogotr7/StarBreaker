use std::io;

#[derive(Debug, thiserror::Error)]
pub enum WemError {
    #[error(transparent)]
    Parse(#[from] starbreaker_common::ParseError),

    #[error("invalid RIFF magic: expected RIFF or RIFX, got {got:#010x}")]
    InvalidRiffMagic { got: u32 },

    #[error("invalid WAVE magic: expected WAVE, got {got:#010x}")]
    InvalidWaveMagic { got: u32 },

    #[error("missing required chunk: {tag}")]
    MissingChunk { tag: String },

    #[error("unsupported codec: {0:#06x}")]
    UnsupportedCodec(u16),

    #[error("decode error: {0}")]
    Decode(String),

    #[error(transparent)]
    Io(#[from] io::Error),
}
