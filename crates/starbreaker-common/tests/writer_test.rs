use starbreaker_common::{SpanReader, SpanWriter};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

#[test]
fn write_u32_little_endian() {
    let mut w = SpanWriter::new();
    w.write_u32(1);
    assert_eq!(w.into_inner(), [0x01, 0x00, 0x00, 0x00]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
struct TestStruct {
    a: u32,
    b: u16,
}

#[test]
fn write_val_struct() {
    let val = TestStruct { a: 7, b: 3 };
    let mut w = SpanWriter::new();
    w.write_val(&val);
    let data = w.into_inner();
    assert_eq!(data.len(), 6);

    // Verify the bytes via a reader
    let mut r = SpanReader::new(&data);
    let read_back = r.read_type::<TestStruct>().unwrap();
    let a = read_back.a;
    let b = read_back.b;
    assert_eq!(a, 7);
    assert_eq!(b, 3);
}

#[test]
fn reader_writer_round_trip() {
    let mut w = SpanWriter::new();
    w.write_u32(42);
    w.write_u8(0xFF);
    w.write_u16(100);
    w.write_f32(1.5);
    w.write_i32(-7);
    w.write_u64(999_999_999_999);

    let data = w.into_inner();
    let mut r = SpanReader::new(&data);
    assert_eq!(r.read_u32().unwrap(), 42);
    assert_eq!(r.read_u8().unwrap(), 0xFF);
    assert_eq!(r.read_u16().unwrap(), 100);
    assert!((r.read_f32().unwrap() - 1.5).abs() < 1e-6);
    assert_eq!(r.read_i32().unwrap(), -7);
    assert_eq!(r.read_u64().unwrap(), 999_999_999_999);
    assert!(r.is_empty());
}

#[test]
fn default_writer() {
    let w = SpanWriter::default();
    assert!(w.is_empty());
    assert_eq!(w.len(), 0);
}

#[test]
fn with_capacity() {
    let mut w = SpanWriter::with_capacity(1024);
    assert!(w.is_empty());
    w.write_bytes(&[1, 2, 3]);
    assert_eq!(w.len(), 3);
    assert!(!w.is_empty());
}
