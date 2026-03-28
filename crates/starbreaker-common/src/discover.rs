//! Auto-discovery of Star Citizen install paths.
//!
//! Resolution order:
//! 1. Explicit environment variable override (per-resource)
//! 2. Auto-scan default install locations, pick the newest by modification time
//!
//! # Environment variables
//!
//! - `SC_DATA_P4K` — path to `Data.p4k`
//! - `SC_EXE` — path to `StarCitizen.exe`
//!
//! # Setting up `.cargo/config.toml`
//!
//! ```toml
//! [env]
//! SC_DATA_P4K = "C:\\Program Files\\Roberts Space Industries\\StarCitizen\\LIVE\\Data.p4k"
//! SC_EXE = "C:\\Program Files\\Roberts Space Industries\\StarCitizen\\LIVE\\Bin64\\StarCitizen.exe"
//! ```

use std::path::{Path, PathBuf};

/// Known Star Citizen channel names, in preference order.
pub const CHANNELS: &[&str] = &["LIVE", "PTU", "EPTU", "TECH-PREVIEW"];

/// Default install root on Windows.
pub const DEFAULT_ROOT: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen";

/// Environment variable for overriding the P4K path.
pub const ENV_P4K: &str = "SC_DATA_P4K";

/// Environment variable for overriding the exe path.
pub const ENV_EXE: &str = "SC_EXE";

/// A discovered file with its source info.
#[derive(Debug)]
pub struct Discovered {
    pub path: PathBuf,
    /// "env", or channel name like "LIVE", "PTU", etc.
    pub source: String,
}

/// Find a file under the SC install root by checking an env var first, then
/// scanning channels for a relative path within each channel directory.
///
/// `env_var` — environment variable name to check first
/// `relative_path` — path relative to the channel directory (e.g. "Data.p4k" or "Bin64/StarCitizen.exe")
pub fn find_file(env_var: &str, relative_path: &str) -> Result<Discovered, DiscoverError> {
    // 1. Check environment variable
    if let Ok(val) = std::env::var(env_var) {
        let path = PathBuf::from(&val);
        if path.is_file() {
            return Ok(Discovered {
                path,
                source: "env".to_string(),
            });
        }
        return Err(DiscoverError::EnvVarNotFound {
            var: env_var.to_string(),
            path,
        });
    }

    // 2. Auto-scan default locations
    let root = Path::new(DEFAULT_ROOT);
    if !root.is_dir() {
        return Err(DiscoverError::NotInstalled);
    }

    let mut candidates: Vec<(PathBuf, String, std::time::SystemTime)> = Vec::new();
    for &channel in CHANNELS {
        let file = root.join(channel).join(relative_path);
        if file.is_file() {
            let mtime = file
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            candidates.push((file, channel.to_string(), mtime));
        }
    }

    if candidates.is_empty() {
        return Err(DiscoverError::NotFound {
            relative_path: relative_path.to_string(),
        });
    }

    // Pick the newest by modification time
    candidates.sort_by(|a, b| b.2.cmp(&a.2));
    let (path, source, _) = candidates.into_iter().next().unwrap();
    Ok(Discovered { path, source })
}

/// Find Data.p4k (returns the newest).
pub fn find_p4k() -> Result<Discovered, DiscoverError> {
    find_file(ENV_P4K, "Data.p4k")
}

/// Find all Data.p4k files across all channels.
pub fn find_all_p4k() -> Vec<Discovered> {
    // Check env var first
    if let Ok(val) = std::env::var(ENV_P4K) {
        let path = PathBuf::from(&val);
        if path.is_file() {
            return vec![Discovered {
                path,
                source: "env".to_string(),
            }];
        }
    }

    let root = Path::new(DEFAULT_ROOT);
    if !root.is_dir() {
        return Vec::new();
    }

    CHANNELS
        .iter()
        .filter_map(|&channel| {
            let file = root.join(channel).join("Data.p4k");
            if file.is_file() {
                Some(Discovered {
                    path: file,
                    source: channel.to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Find StarCitizen.exe.
pub fn find_exe() -> Result<Discovered, DiscoverError> {
    find_file(ENV_EXE, "Bin64/StarCitizen.exe")
}

#[derive(Debug)]
pub enum DiscoverError {
    /// Env var was set but the file doesn't exist.
    EnvVarNotFound { var: String, path: PathBuf },
    /// Default install directory doesn't exist.
    NotInstalled,
    /// Install directory exists but the file wasn't found in any channel.
    NotFound { relative_path: String },
}

impl std::fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoverError::EnvVarNotFound { var, path } => {
                write!(f, "{var} is set to '{}' but the file does not exist", path.display())
            }
            DiscoverError::NotInstalled => write!(
                f,
                "Star Citizen not found at '{DEFAULT_ROOT}'"
            ),
            DiscoverError::NotFound { relative_path } => write!(
                f,
                "No '{relative_path}' found in any channel ({}) under '{DEFAULT_ROOT}'",
                CHANNELS.join(", ")
            ),
        }
    }
}

impl std::error::Error for DiscoverError {}
