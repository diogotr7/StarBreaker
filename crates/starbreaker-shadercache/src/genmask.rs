//! Shader generation-mask bits, read from the shipped `.ext` property files.
//!
//! Every shader template ships a `<Family>.ext` describing its material
//! properties and the bit each one occupies in that family's generation mask:
//!
//! ```text
//! Property
//! {
//!     Name = %NORMAL_MAP
//!     Mask = 0x1
//!     …
//! }
//! ```
//!
//! Bits are **per family**: the same flag name sits on different bits in
//! different templates (e.g. `%ANIM_GLOW` is one bit in HardSurface and another
//! in Illum), so tables are never merged. [`GenMasks`] holds one [`GenMask`] per
//! `.ext` file, looked up by family name.
//!
//! `.ext` files live in `Engine\ShaderCache_EXT.pak` inside `Data.p4k`, so the
//! flag→bit tables are data the user already owns; nothing is baked in.

use std::collections::HashMap;

/// One shader family's flag name → mask bit table.
#[derive(Debug, Clone, Default)]
pub struct GenMask {
    bits: HashMap<String, u64>,
}

fn parse_hex(text: &str) -> Option<u64> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

impl GenMask {
    /// Parse one `.ext` file.
    pub fn parse(text: &str) -> Self {
        let mut bits = HashMap::new();
        let mut name: Option<String> = None;
        let mut mask: Option<u64> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("Name = %") {
                name = Some(rest.trim().to_owned());
            } else if let Some(rest) = line.strip_prefix("Mask = ") {
                mask = parse_hex(rest);
            } else if line == "}"
                && let (Some(n), Some(m)) = (name.take(), mask.take()) {
                    bits.insert(n, m);
                }
        }
        Self { bits }
    }

    /// The bit for a flag name (accepts an optional leading `%`).
    pub fn bit(&self, flag: &str) -> Option<u64> {
        self.bits.get(flag.trim_start_matches('%')).copied()
    }

    /// OR the bits for a set of flags, returning the mask plus any flags this
    /// family does not define.
    pub fn mask_for(&self, flags: &[String]) -> (u64, Vec<String>) {
        let mut mask = 0u64;
        let mut unknown = Vec::new();
        for flag in flags {
            match self.bit(flag) {
                Some(bit) => mask |= bit,
                None => unknown.push(flag.clone()),
            }
        }
        (mask, unknown)
    }

    /// Number of flags known.
    pub fn len(&self) -> usize {
        self.bits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }

    /// Every known flag, ascending.
    pub fn flags(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.bits.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }
}

/// Per-family generation-mask tables, keyed by `.ext` file stem.
#[derive(Debug, Clone, Default)]
pub struct GenMasks {
    /// lowercase family -> (family as spelled in the file name, table)
    families: HashMap<String, (String, GenMask)>,
}

impl GenMasks {
    /// Build from `(entry path, text)` pairs; entries that are not `.ext`
    /// files are ignored. The family name is the file stem.
    pub fn from_ext_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut families = HashMap::new();
        for (path, text) in files {
            let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
            let Some(stem) = file
                .strip_suffix(".ext")
                .or_else(|| file.strip_suffix(".EXT"))
            else {
                continue;
            };
            families.insert(stem.to_ascii_lowercase(), (stem.to_owned(), GenMask::parse(text)));
        }
        Self { families }
    }

    /// The table of one shader family (case-insensitive), if it ships one.
    pub fn family(&self, name: &str) -> Option<&GenMask> {
        self.families.get(&name.to_ascii_lowercase()).map(|(_, m)| m)
    }

    /// Family names, ascending.
    pub fn families(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.families.values().map(|(n, _)| n.as_str()).collect();
        v.sort_unstable();
        v
    }
}
