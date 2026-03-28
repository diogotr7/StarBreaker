use std::io::Cursor;

use ww2ogg::{CodebookLibrary, WwiseRiffVorbis};

use crate::WemError;

/// Convert a Wwise Vorbis WEM (raw bytes) to standard Ogg Vorbis.
///
/// Tries default codebooks first with validation, falls back to aoTuV 6.03.
/// Validation is needed because decode can "succeed" with the wrong codebook
/// but produce garbled audio.
pub fn wem_to_ogg(wem_bytes: &[u8]) -> Result<Vec<u8>, WemError> {
    // Try default codebooks first, validate output
    if let Ok(ogg) = try_with_codebooks(wem_bytes, CodebookLibrary::default_codebooks) {
        if ww2ogg::validate(&ogg).is_ok() {
            return Ok(ogg);
        }
    }

    // Fall back to aoTuV codebooks
    let ogg = try_with_codebooks(wem_bytes, CodebookLibrary::aotuv_codebooks)?;
    // Validate aoTuV result too — if both fail, something is truly wrong
    if let Err(e) = ww2ogg::validate(&ogg) {
        return Err(WemError::Decode(format!("validation failed with both codebooks: {e}")));
    }
    Ok(ogg)
}

fn try_with_codebooks(
    wem_bytes: &[u8],
    codebook_fn: fn() -> ww2ogg::WemResult<CodebookLibrary>,
) -> Result<Vec<u8>, WemError> {
    let codebooks = codebook_fn().map_err(|e| WemError::Decode(e.to_string()))?;
    let input = Cursor::new(wem_bytes);
    let mut converter =
        WwiseRiffVorbis::new(input, codebooks).map_err(|e| WemError::Decode(e.to_string()))?;

    let mut ogg_data = Vec::new();
    converter
        .generate_ogg(&mut ogg_data)
        .map_err(|e| WemError::Decode(e.to_string()))?;

    Ok(ogg_data)
}
