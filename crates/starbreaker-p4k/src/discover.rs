//! Auto-discovery of Star Citizen Data.p4k files.
//!
//! Delegates to [`starbreaker_common::discover`] for install path scanning.

use std::path::PathBuf;

pub use starbreaker_common::discover::{ENV_P4K as ENV_VAR, DiscoverError as FindError};

/// Find the Data.p4k file, checking the env var first, then default locations.
pub fn find_p4k() -> Result<(PathBuf, String), FindError> {
    let d = starbreaker_common::discover::find_p4k()?;
    Ok((d.path, d.source))
}

/// Open the auto-discovered P4K as a `MappedP4k`.
///
/// Prints discovery info to stderr for visibility.
pub fn open_p4k() -> Result<crate::MappedP4k, OpenError> {
    let (path, source) = find_p4k().map_err(OpenError::Find)?;
    eprintln!("P4K: {} ({})", path.display(), source);
    crate::MappedP4k::open(&path).map_err(|e| OpenError::Open { path, source: e })
}

#[derive(Debug)]
pub enum OpenError {
    Find(FindError),
    Open { path: PathBuf, source: crate::P4kError },
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Find(e) => write!(f, "{e}"),
            OpenError::Open { path, source } => {
                write!(f, "Failed to open '{}': {source}", path.display())
            }
        }
    }
}

impl std::error::Error for OpenError {}
