use starbreaker_common::ParseError;

/// Errors that can occur when working with P4k archives.
#[derive(Debug, thiserror::Error)]
pub enum P4kError {
    /// The End of Central Directory record was not found.
    #[error("end of central directory record not found")]
    EocdNotFound,

    /// A ZIP structure had an invalid signature.
    #[error("invalid signature: expected {expected:#010x}, got {got:#010x}")]
    InvalidSignature { expected: u32, got: u32 },

    /// The compression method is not supported.
    #[error("unsupported compression method: {0}")]
    UnsupportedCompression(u16),

    /// Decompression failed.
    #[error("decompression failed: {0}")]
    Decompression(String),

    /// Decryption failed.
    #[error("decryption failed: {0}")]
    Decryption(String),

    /// An encrypted entry uses a non-zstd compression method.
    #[error("encrypted entry uses non-zstd compression method {0}")]
    EncryptedNonZstd(u16),

    /// A binary parsing error from starbreaker-common.
    #[error(transparent)]
    Parse(#[from] ParseError),

    /// A file was not found in the archive.
    #[error("entry not found in P4k: {0}")]
    EntryNotFound(String),

    /// An I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
