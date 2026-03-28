use starbreaker_datacore::reader::SpanReader;

// ─── read_u32 little-endian ──────────────────────────────────────────────────

#[test]
fn read_u32_little_endian() {
    let data = [0x01u8, 0x00, 0x00, 0x00];
    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_u32().unwrap(), 1);
}

#[test]
fn read_u32_little_endian_multi_byte() {
    // 0x0102_0304 in LE is [0x04, 0x03, 0x02, 0x01]
    let data = [0x04u8, 0x03, 0x02, 0x01];
    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_u32().unwrap(), 0x0102_0304);
}

// ─── read_i32 ────────────────────────────────────────────────────────────────

#[test]
fn read_i32_negative() {
    // -1 in LE is [0xff, 0xff, 0xff, 0xff]
    let data = [0xffu8, 0xff, 0xff, 0xff];
    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_i32().unwrap(), -1);
}

#[test]
fn read_i32_positive() {
    let data = [0x01u8, 0x00, 0x00, 0x00];
    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_i32().unwrap(), 1);
}

// ─── sequential reads ────────────────────────────────────────────────────────

#[test]
fn sequential_reads() {
    // u32(0x01020304) followed by u16(0x0506) followed by u8(0x07)
    let data = [0x04u8, 0x03, 0x02, 0x01, 0x06, 0x05, 0x07];
    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_u32().unwrap(), 0x0102_0304);
    assert_eq!(r.read_u16().unwrap(), 0x0506);
    assert_eq!(r.read_u8().unwrap(), 0x07);
}

// ─── advance and position tracking ───────────────────────────────────────────

#[test]
fn advance_and_position() {
    let data = [0u8; 16];
    let mut r = SpanReader::new(&data);
    assert_eq!(r.position(), 0);
    assert_eq!(r.remaining(), 16);

    r.advance(4).unwrap();
    assert_eq!(r.position(), 4);
    assert_eq!(r.remaining(), 12);

    r.advance(8).unwrap();
    assert_eq!(r.position(), 12);
    assert_eq!(r.remaining(), 4);
}

#[test]
fn advance_past_end_returns_error() {
    let data = [0u8; 4];
    let mut r = SpanReader::new(&data);
    let result = r.advance(8);
    assert!(result.is_err());
}

#[test]
fn new_at_sets_position() {
    let data = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let mut r = SpanReader::new_at(&data, 4);
    assert_eq!(r.position(), 4);
    assert_eq!(r.remaining(), 4);
    // reading from position 4: [0x05, 0x06, 0x07, 0x08] => u32 = 0x0807_0605
    assert_eq!(r.read_u32().unwrap(), 0x0807_0605);
}

// ─── read_bool ────────────────────────────────────────────────────────────────

#[test]
fn read_bool_zero_is_false() {
    let data = [0x00u8];
    let mut r = SpanReader::new(&data);
    assert!(!r.read_bool().unwrap());
}

#[test]
fn read_bool_nonzero_is_true() {
    for byte in [0x01u8, 0x02, 0x7f, 0xff] {
        let data = [byte];
        let mut r = SpanReader::new(&data);
        assert!(r.read_bool().unwrap(), "byte {byte:#04x} should be true");
    }
}

// ─── read past end returns Truncated error ────────────────────────────────────

#[test]
fn read_truncated_error() {
    let data = [0x01u8, 0x02];
    let mut r = SpanReader::new(&data);
    let result = r.read_u32();
    assert!(result.is_err());
}
