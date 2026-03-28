use std::path::PathBuf;

/// Trait for reading sibling split files relative to a base DDS path.
pub trait ReadSibling {
    /// Read a sibling file by appending `suffix` to the base path.
    /// Returns `None` if the file does not exist.
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>>;
}

/// Filesystem-based sibling reader.
pub struct FsSiblingReader {
    base_path: PathBuf,
}

impl FsSiblingReader {
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }
}

impl ReadSibling for FsSiblingReader {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let mut path = self.base_path.clone().into_os_string();
        path.push(suffix);
        std::fs::read(path).ok()
    }
}
