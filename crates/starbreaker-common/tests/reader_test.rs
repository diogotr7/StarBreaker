use starbreaker_common::{ParseError, SpanReader};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

#[test]
fn read_u32_little_endian() {
    let data = [0x01, 0x00, 0x00, 0x00];
    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_u32().unwrap(), 1);
}

#[test]
fn truncated_error_has_offset() {
    let data = [0x01, 0x02];
    let mut r = SpanReader::new(&data);
    // Advance 1 byte so offset is 1
    r.read_u8().unwrap();
    let err = r.read_u32().unwrap_err();
    match err {
        ParseError::Truncated { offset, need, have } => {
            assert_eq!(offset, 1);
            assert_eq!(need, 4);
            assert_eq!(have, 1);
        }
        other => panic!("expected Truncated, got {other:?}"),
    }
}

#[test]
fn expect_matching() {
    let data = 42u32.to_le_bytes();
    let mut r = SpanReader::new(&data);
    let val = r.expect(42u32).unwrap();
    assert_eq!(*val, 42u32);
}

#[test]
fn expect_mismatch() {
    let data = 99u32.to_le_bytes();
    let mut r = SpanReader::new(&data);
    let err = r.expect(42u32).unwrap_err();
    match err {
        ParseError::UnexpectedValue {
            offset,
            expected,
            actual,
        } => {
            assert_eq!(offset, 0);
            assert!(expected.contains("42"), "expected field: {expected}");
            assert!(actual.contains("99"), "actual field: {actual}");
        }
        other => panic!("expected UnexpectedValue, got {other:?}"),
    }
}

#[test]
fn expect_any() {
    let data = 2u32.to_le_bytes();
    let mut r = SpanReader::new(&data);
    let val = r.expect_any(&[1u32, 2, 3]).unwrap();
    assert_eq!(*val, 2u32);
}

#[test]
fn expect_any_mismatch() {
    let data = 99u32.to_le_bytes();
    let mut r = SpanReader::new(&data);
    let err = r.expect_any(&[1u32, 2, 3]).unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedValue { .. }));
}

#[test]
fn split_off() {
    let data = [1, 2, 3, 4, 5, 6];
    let mut r = SpanReader::new(&data);
    let mut sub = r.split_off(3).unwrap();
    assert_eq!(r.position(), 3);
    assert_eq!(r.remaining(), 3);
    assert_eq!(sub.read_u8().unwrap(), 1);
    assert_eq!(sub.read_u8().unwrap(), 2);
    assert_eq!(sub.read_u8().unwrap(), 3);
    assert!(sub.is_empty());
}

#[test]
fn peek_type() {
    let data = 42u32.to_le_bytes();
    let mut r = SpanReader::new(&data);
    let peeked = r.peek_type::<u32>().unwrap();
    assert_eq!(*peeked, 42u32);
    // Position should not have advanced
    assert_eq!(r.position(), 0);
    // Now actually read it
    let read = r.read_type::<u32>().unwrap();
    assert_eq!(*read, 42u32);
    assert_eq!(r.position(), 4);
}

#[test]
fn read_slice() {
    let data = [1u32.to_le_bytes(), 2u32.to_le_bytes(), 3u32.to_le_bytes()].concat();
    let mut r = SpanReader::new(&data);
    let slice = r.read_slice::<u32>(3).unwrap();
    assert_eq!(slice, &[1, 2, 3]);
    assert!(r.is_empty());
}

#[test]
fn is_empty() {
    let data = [0u8; 2];
    let mut r = SpanReader::new(&data);
    assert!(!r.is_empty());
    r.read_bytes(2).unwrap();
    assert!(r.is_empty());
}

#[test]
fn empty_reader_is_empty() {
    let r = SpanReader::new(&[]);
    assert!(r.is_empty());
}

#[test]
fn sequential_reads() {
    let mut data = Vec::new();
    data.extend_from_slice(&42u32.to_le_bytes());
    data.extend_from_slice(&[0xFF]);
    data.extend_from_slice(&100u16.to_le_bytes());

    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_u32().unwrap(), 42);
    assert_eq!(r.read_u8().unwrap(), 0xFF);
    assert_eq!(r.read_u16().unwrap(), 100);
    assert!(r.is_empty());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
struct TestStruct {
    a: u32,
    b: u16,
}

#[test]
fn read_type_struct() {
    let mut data = Vec::new();
    data.extend_from_slice(&7u32.to_le_bytes());
    data.extend_from_slice(&3u16.to_le_bytes());

    let mut r = SpanReader::new(&data);
    let val = r.read_type::<TestStruct>().unwrap();
    let a = val.a;
    let b = val.b;
    assert_eq!(a, 7);
    assert_eq!(b, 3);
}
