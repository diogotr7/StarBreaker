/// Errors produced when parsing CryXmlB binary data.
#[derive(Debug, thiserror::Error)]
pub enum CryXmlError {
    /// The data did not start with the expected `CryXmlB\0` magic bytes.
    #[error("invalid magic: expected CryXmlB")]
    InvalidMagic,

    /// A lower-level binary parse error occurred.
    #[error(transparent)]
    Parse(#[from] starbreaker_common::ParseError),
}
