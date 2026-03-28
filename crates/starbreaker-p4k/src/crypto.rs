use aes::cipher::{BlockDecryptMut, KeyIvInit};
use cbc::Decryptor;

use crate::error::P4kError;

type Aes128CbcDec = Decryptor<aes::Aes128>;

/// AES-128-CBC key used by CIG for P4k encryption.
const AES_KEY: [u8; 16] = [
    0x5E, 0x7A, 0x20, 0x02, 0x30, 0x2E, 0xEB, 0x1A, 0x3B, 0xB6, 0x17, 0xC3, 0x0F, 0xDE, 0x1E, 0x47,
];

/// IV: 16 zero bytes.
const AES_IV: [u8; 16] = [0u8; 16];

/// Decrypt AES-128-CBC data with CIG's zero-byte padding.
///
/// Returns a new `Vec<u8>` with trailing zero bytes trimmed.
pub fn decrypt(data: &[u8]) -> Result<Vec<u8>, P4kError> {
    // AES-CBC requires input length to be a multiple of 16
    if data.is_empty() {
        return Ok(Vec::new());
    }

    // Clone the data so we can decrypt in-place
    let mut buf = data.to_vec();

    let decryptor = Aes128CbcDec::new(&AES_KEY.into(), &AES_IV.into());

    decryptor
        .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
        .map_err(|e| P4kError::Decryption(e.to_string()))?;

    // Trim trailing zero bytes (CIG uses zero-padding, not PKCS7)
    let last_non_zero = buf.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    buf.truncate(last_non_zero);

    Ok(buf)
}
