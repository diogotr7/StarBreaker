//! Alignment-safe helper functions for reading and validating binary values.
//!
//! SpanReader's `expect()` and `read_type::<T>()` use zerocopy's `ref_from_bytes`,
//! which requires proper alignment. Since CHF data comes from byte buffers that
//! may not be aligned, we use the primitive read methods (read_u32, read_u16, etc.)
//! which use `from_le_bytes` and don't require alignment.

use starbreaker_common::{CigGuid, NameHash, ParseError, SpanReader};

/// Read a u32 and assert it equals `expected`.
pub fn expect_u32(reader: &mut SpanReader, expected: u32) -> Result<u32, ParseError> {
    let offset = reader.position();
    let actual = reader.read_u32()?;
    if actual != expected {
        return Err(ParseError::UnexpectedValue {
            offset,
            expected: format!("{expected}"),
            actual: format!("{actual}"),
        });
    }
    Ok(actual)
}

/// Read a NameHash (4 bytes, alignment-safe since NameHash is repr(C, packed)).
pub fn read_name_hash(reader: &mut SpanReader) -> Result<NameHash, ParseError> {
    let val = reader.read_u32()?;
    Ok(NameHash(val))
}

/// Read a NameHash and assert it equals `expected`.
pub fn expect_name_hash(
    reader: &mut SpanReader,
    expected: NameHash,
) -> Result<NameHash, ParseError> {
    let offset = reader.position();
    let actual = read_name_hash(reader)?;
    if actual != expected {
        return Err(ParseError::UnexpectedValue {
            offset,
            expected: format!("{expected:?}"),
            actual: format!("{actual:?}"),
        });
    }
    Ok(actual)
}

/// Read a CigGuid (16 bytes, alignment-safe since CigGuid is repr(C, packed)).
pub fn read_guid(reader: &mut SpanReader) -> Result<CigGuid, ParseError> {
    let bytes = reader.read_bytes(16)?;
    let mut arr = [0u8; 16];
    arr.copy_from_slice(bytes);
    Ok(CigGuid::from_bytes(arr))
}

/// Read a CigGuid and assert it equals CigGuid::EMPTY.
pub fn expect_empty_guid(reader: &mut SpanReader) -> Result<(), ParseError> {
    let offset = reader.position();
    let guid = read_guid(reader)?;
    if guid != CigGuid::EMPTY {
        return Err(ParseError::UnexpectedValue {
            offset,
            expected: "empty GUID".to_string(),
            actual: format!("{guid}"),
        });
    }
    Ok(())
}
