//! The shader keyword grammar: token id -> keyword text for `FXB1` shader bins.
//!
//! Keyword ids are positions in the engine's parser token list, so any keyword
//! CIG inserts shifts every id after it. A stale grammar therefore decodes a
//! stream into plausible but wrong text, and structural checks cannot tell
//! (most shifted ids still resolve). The only reliable selector observed is the
//! bin's **cache version word** (`FXB1` header): across every build examined it
//! changes exactly when the grammar does.
//!
//! So exactly one grammar ships — the latest known, in
//! `grammar/cache-<version>.txt` — bound to [`SUPPORTED_CACHE_VERSION`].
//! [`for_cache_version`] hands it out for that version and refuses every other
//! one; there is no fallback and no guessing.
//!
//! Grammar text format: one token per line, `id<TAB>text`, the id decimal or
//! `0x`-prefixed; blank lines and `#` comments are ignored.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::ShaderCacheError;

/// A keyword grammar: token id -> keyword text.
#[derive(Debug, Clone, Default)]
pub struct Grammar {
    map: HashMap<u32, String>,
}

impl Grammar {
    /// Parse a grammar file.
    pub fn parse(text: &str) -> Result<Self, ShaderCacheError> {
        let mut map = HashMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((id, token)) = line.split_once('\t') else {
                return Err(ShaderCacheError::MalformedGrammar(n + 1));
            };
            let id = id.trim();
            let id = match id.strip_prefix("0x").or_else(|| id.strip_prefix("0X")) {
                Some(hex) => u32::from_str_radix(hex, 16),
                None => id.parse::<u32>(),
            }
            .map_err(|_| ShaderCacheError::MalformedGrammar(n + 1))?;
            map.insert(id, token.to_owned());
        }
        Ok(Self { map })
    }

    /// Look up a syntax token.
    pub fn get(&self, id: u32) -> Option<&str> {
        self.map.get(&id).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Every id in this grammar, ascending.
    pub fn ids(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.map.keys().copied().collect();
        v.sort_unstable();
        v
    }
}

/// Cache version word of the shader bins the shipped grammar belongs to.
pub const SUPPORTED_CACHE_VERSION: u32 = 0x00B2_22BE;

const SHIPPED_TEXT: &str = include_str!("../grammar/cache-00b222be.txt");

/// The shipped grammar (for [`SUPPORTED_CACHE_VERSION`]).
pub fn shipped() -> &'static Grammar {
    static SHIPPED: OnceLock<Grammar> = OnceLock::new();
    SHIPPED.get_or_init(|| {
        Grammar::parse(SHIPPED_TEXT).expect("the shipped grammar file is well formed")
    })
}

/// The grammar for a shader bin's cache version, or an error naming both
/// versions when this build does not support it.
pub fn for_cache_version(version: u32) -> Result<&'static Grammar, ShaderCacheError> {
    if version == SUPPORTED_CACHE_VERSION {
        Ok(shipped())
    } else {
        Err(ShaderCacheError::UnsupportedCacheVersion {
            found: version,
            supported: SUPPORTED_CACHE_VERSION,
        })
    }
}
