/// Errors produced when parsing a CryEngine chunk file container.
#[derive(Debug, thiserror::Error)]
pub enum ChunkFileError {
    /// The first 4 bytes do not match any known magic number.
    #[error("unrecognized magic: {0:#010x}")]
    UnrecognizedMagic(u32),

    /// The file version is not one we know how to handle.
    #[error("unsupported version: {0:#x}")]
    UnsupportedVersion(u32),

    /// A lower-level parse error (truncation, bad layout, etc.).
    #[error(transparent)]
    Parse(#[from] starbreaker_common::ParseError),
}
