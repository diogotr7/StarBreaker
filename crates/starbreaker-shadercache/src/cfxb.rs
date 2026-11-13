//! Parser for the `FXB1` shader-bin container (`.cfxb` / `.cfib`).
//!
//! Layout: a 28-byte header, the token id stream, then a string table of
//! `(u32 id, cstring text)` records. The token ids below the engine's
//! "user token" boundary refer to the compiler's keyword grammar, which is
//! versioned separately (see [`crate::grammar`]); ids above it index the
//! file's own string table.

use std::collections::HashMap;

use crate::error::ShaderCacheError;

pub const FXB_MAGIC: &[u8; 4] = b"FXB1";
pub const FXB_HEADER_LEN: usize = 28;

/// A decoded shader-bin file.
#[derive(Debug, Clone)]
pub struct ShaderBin {
    /// Cache/compiler version word; changes between game builds.
    pub version: u32,
    pub version_lo: u16,
    pub version_hi: u16,
    /// CRC of the source file the bin was compiled from.
    pub source_crc: u32,
    /// Token id stream.
    pub tokens: Vec<u32>,
    /// Identifier string table (id -> text).
    pub table: HashMap<u32, String>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Parse a `.cfxb` / `.cfib` payload.
pub fn parse(bytes: &[u8]) -> Result<ShaderBin, ShaderCacheError> {
    if bytes.len() < FXB_HEADER_LEN || &bytes[0..4] != FXB_MAGIC {
        return Err(ShaderCacheError::BadShaderBinMagic);
    }
    let version = u32_at(bytes, 4);
    let version_lo = u16_at(bytes, 8);
    let version_hi = u16_at(bytes, 10);
    let off_string_table = u32_at(bytes, 12) as usize;
    let off_params = u32_at(bytes, 16) as usize;
    let n_tokens = u32_at(bytes, 20) as usize;
    let source_crc = u32_at(bytes, 24);

    let token_end = FXB_HEADER_LEN + n_tokens * 4;
    if token_end > bytes.len() || off_string_table > bytes.len() {
        return Err(ShaderCacheError::MalformedShaderBin(
            "token stream or string table out of bounds",
        ));
    }
    let tokens = bytes[FXB_HEADER_LEN..token_end]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let table_end = off_params.min(bytes.len());
    let mut table = HashMap::new();
    let mut p = off_string_table;
    while p + 4 <= table_end {
        let id = u32_at(bytes, p);
        let Some(nul) = bytes[p + 4..].iter().position(|&c| c == 0) else {
            break;
        };
        let text = String::from_utf8_lossy(&bytes[p + 4..p + 4 + nul]).into_owned();
        table.insert(id, text);
        p += 4 + nul + 1;
    }

    Ok(ShaderBin {
        version,
        version_lo,
        version_hi,
        source_crc,
        tokens,
        table,
    })
}
