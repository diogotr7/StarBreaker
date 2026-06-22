pub mod alpha;
pub mod dds_file;
pub mod decode;
pub mod error;
pub mod sibling;
pub mod types;

// Re-exports for convenience.
pub use alpha::{AlphaMipFormat, AlphaMipLayout};
pub use dds_file::{DdsFile, resolve_format};
pub use error::DdsError;
pub use sibling::{FsSiblingReader, ReadSibling};
pub use types::{DdsHeader, DdsHeaderDxt10, DdsPixelFormat, DxgiFormat};
