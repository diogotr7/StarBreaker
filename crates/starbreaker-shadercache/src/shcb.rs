//! The `SHCB` shader-cache container.
//!
//! Layout (28-byte header, then the payload):
//! `magic "SHCB"`, `u32 version`, `u64 hash`, `u32 payload_size`,
//! `u32 stage_count`, `u32 padding`.
//!
//! For Vulkan caches the payload is a SPIR-V module.

use crate::error::ShaderCacheError;

/// SPIR-V magic number, little-endian on disk (`03 02 23 07`).
pub const SPIRV_MAGIC: u32 = 0x0723_0203;

pub const SHCB_MAGIC: &[u8; 4] = b"SHCB";
pub const SHCB_HEADER_LEN: usize = 28;

/// A parsed shader-cache block.
#[derive(Debug, Clone, Copy)]
pub struct Shcb<'a> {
    /// Container version word. Changes between game builds.
    pub version: u32,
    /// Module hash as stored by the engine.
    pub hash: u64,
    /// Raw payload (SPIR-V for Vulkan).
    pub payload: &'a [u8],
}

/// Split an `SHCB` block into its header fields and payload.
pub fn parse(bytes: &[u8]) -> Result<Shcb<'_>, ShaderCacheError> {
    if bytes.len() < SHCB_HEADER_LEN || &bytes[0..4] != SHCB_MAGIC {
        return Err(ShaderCacheError::BadShcbMagic);
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let hash = u64::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    ]);
    let payload = &bytes[SHCB_HEADER_LEN..];
    Ok(Shcb {
        version,
        hash,
        payload,
    })
}

/// Split an `SHCB` block and return the SPIR-V payload, validating its magic.
pub fn spirv_payload(bytes: &[u8]) -> Result<Shcb<'_>, ShaderCacheError> {
    let shcb = parse(bytes)?;
    if shcb.payload.len() < 4 {
        return Err(ShaderCacheError::NotSpirv(0));
    }
    let magic = u32::from_le_bytes([
        shcb.payload[0],
        shcb.payload[1],
        shcb.payload[2],
        shcb.payload[3],
    ]);
    if magic != SPIRV_MAGIC {
        return Err(ShaderCacheError::NotSpirv(magic));
    }
    Ok(shcb)
}
