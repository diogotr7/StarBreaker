/// BNK section tags as little-endian u32 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionTag {
    /// Bank Header
    Bkhd,
    /// Data Index
    Didx,
    /// Raw audio data
    Data,
    /// Hierarchy (events, sounds, containers)
    Hirc,
    /// String ID table
    Stid,
    /// Initialization data
    Init,
    /// Streaming Manager / global settings
    Stmg,
    /// Environment settings
    Envs,
    /// Platform data
    Plat,
    /// Unknown section
    Unknown(u32),
}

impl SectionTag {
    pub fn from_u32(val: u32) -> Self {
        match val {
            0x44484B42 => Self::Bkhd,
            0x58444944 => Self::Didx,
            0x41544144 => Self::Data,
            0x43524948 => Self::Hirc,
            0x44495453 => Self::Stid,
            0x54494E49 => Self::Init,
            0x474D5453 => Self::Stmg,
            0x53564E45 => Self::Envs,
            0x54414C50 => Self::Plat,
            other => Self::Unknown(other),
        }
    }

    /// 4-char ASCII label for display.
    pub fn label(&self) -> String {
        let val = match self {
            Self::Bkhd => 0x44484B42u32,
            Self::Didx => 0x58444944,
            Self::Data => 0x41544144,
            Self::Hirc => 0x43524948,
            Self::Stid => 0x44495453,
            Self::Init => 0x54494E49,
            Self::Stmg => 0x474D5453,
            Self::Envs => 0x53564E45,
            Self::Plat => 0x54414C50,
            Self::Unknown(v) => *v,
        };
        let bytes = val.to_le_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl std::fmt::Display for SectionTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// A raw section found during BNK scanning.
#[derive(Debug, Clone, Copy)]
pub struct RawSection {
    pub tag: SectionTag,
    /// Offset of the section data (after the 8-byte tag+size header).
    pub data_offset: usize,
    /// Size of the section data in bytes.
    pub data_size: usize,
}
