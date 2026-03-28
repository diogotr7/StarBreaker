use crate::vorbis;
use crate::WemError;

/// Decode a Wwise Vorbis WEM to standard Ogg Vorbis.
pub fn vorbis_to_ogg(wem_bytes: &[u8]) -> Result<Vec<u8>, WemError> {
    vorbis::wem_to_ogg(wem_bytes)
}
