use starbreaker_common::{ParseError, SpanReader};

/// Read a variable-length quantity (VLQ) integer.
/// 7 bits per byte, high bit = continue.
pub fn read_vlq(reader: &mut SpanReader) -> Result<u32, ParseError> {
    let mut value: u32 = 0;
    loop {
        let byte = reader.read_u8()?;
        value = (value << 7) | (byte & 0x7F) as u32;
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_byte() {
        let data = [0x05u8];
        let mut reader = SpanReader::new(&data);
        assert_eq!(read_vlq(&mut reader).unwrap(), 5);
    }

    #[test]
    fn test_two_bytes() {
        let data = [0x81, 0x00];
        let mut reader = SpanReader::new(&data);
        assert_eq!(read_vlq(&mut reader).unwrap(), 128);
    }

    #[test]
    fn test_zero() {
        let data = [0x00u8];
        let mut reader = SpanReader::new(&data);
        assert_eq!(read_vlq(&mut reader).unwrap(), 0);
    }

    #[test]
    fn test_max_single_byte() {
        let data = [0x7F];
        let mut reader = SpanReader::new(&data);
        assert_eq!(read_vlq(&mut reader).unwrap(), 127);
    }
}
