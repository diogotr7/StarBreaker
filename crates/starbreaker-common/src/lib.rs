pub mod color;
pub mod discover;
pub mod error;
pub mod guid;
pub mod name_hash;
pub mod progress;
pub mod reader;
pub mod writer;

// Re-export primary types at crate root for convenience.
pub use color::ColorRgba;
pub use error::ParseError;
pub use guid::{CigGuid, GuidParseError};
pub use name_hash::NameHash;
pub use progress::Progress;
pub use reader::SpanReader;
pub use writer::SpanWriter;
