#[derive(Debug, thiserror::Error)]
pub enum DdsError {
    #[error("invalid DDS magic")]
    InvalidMagic,
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("mip level {level} out of range (max {max})")]
    MipOutOfRange { level: usize, max: usize },
    #[error("decode error: {0}")]
    Decode(String),
    #[error(transparent)]
    Parse(#[from] starbreaker_common::ParseError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("image error: {0}")]
    Image(String),
}
