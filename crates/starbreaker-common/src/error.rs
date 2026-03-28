/// Errors produced when parsing binary data.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// The input was shorter than expected.
    #[error("data truncated at offset {offset}: need {need} bytes, have {have}")]
    Truncated {
        offset: usize,
        need: usize,
        have: usize,
    },

    /// A field contained an unexpected value.
    #[error("unexpected value at offset {offset}: expected {expected}, got {actual}")]
    UnexpectedValue {
        offset: usize,
        expected: String,
        actual: String,
    },

    /// A zerocopy layout conversion failed.
    #[error("invalid struct layout: {0}")]
    InvalidLayout(String),

    /// A magic number did not match.
    #[error("invalid magic at offset {offset}: expected {expected:#010x}, got {got:#010x}")]
    InvalidMagic {
        offset: usize,
        expected: u32,
        got: u32,
    },

    /// A higher-level validation check failed.
    #[error("validation error in {context}: {message}")]
    Validation { context: String, message: String },
}
