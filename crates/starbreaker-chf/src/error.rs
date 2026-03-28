#[derive(Debug, thiserror::Error)]
pub enum ChfError {
    #[error(transparent)]
    Parse(#[from] starbreaker_common::ParseError),
    #[error("CRC32C mismatch: expected {expected:#x}, got {actual:#x}")]
    CrcMismatch { expected: u32, actual: u32 },
    #[error("invalid CHF container: {0}")]
    InvalidContainer(String),
    #[error("compression error: {0}")]
    Compression(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
