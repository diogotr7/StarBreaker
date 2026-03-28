use std::collections::HashMap;

use starbreaker_common::SpanReader;

use crate::error::BnkError;

/// Parse the STID (String ID) section into a map of bank_id -> name.
pub fn parse_stid(data: &[u8]) -> Result<HashMap<u32, String>, BnkError> {
    let mut reader = SpanReader::new(data);
    let _string_type = reader.read_u32()?; // 1 = UTF-8
    let count = reader.read_u32()?;

    let mut map = HashMap::with_capacity(count as usize);
    for _ in 0..count {
        let id = reader.read_u32()?;
        let name_len = reader.read_u8()? as usize;
        let name_bytes = reader.read_bytes(name_len)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        map.insert(id, name);
    }

    Ok(map)
}
