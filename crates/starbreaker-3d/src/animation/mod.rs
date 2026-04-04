//! CryEngine animation parsing: .chrparams, .dba, .caf
//!
//! Ship animations are stored as:
//! - `.chrparams` — XML config mapping animation names to file paths
//! - `.dba` — IVO animation database containing packed animation clips
//! - `.caf` — IVO single animation clip (rare for ships, most are packed into .dba)

pub mod chrparams;
pub mod dba;
