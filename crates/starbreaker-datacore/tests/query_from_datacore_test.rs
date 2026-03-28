mod test_helpers;

use starbreaker_common::SpanReader;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::enums::DataType;
use starbreaker_datacore::query::from_datacore::FromDataCore;
use starbreaker_datacore::types::CigGuid;
use zerocopy::IntoBytes;

#[test]
fn read_i32_from_reader() {
    let b = test_helpers::DcbBuilder::new();
    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();
    let bytes = 42i32.to_le_bytes();
    let mut reader = SpanReader::new(&bytes);
    let val = i32::read_from_reader(&db, &mut reader, DataType::Int32).unwrap();
    assert_eq!(val, 42);
}

#[test]
fn read_f32_from_reader() {
    let b = test_helpers::DcbBuilder::new();
    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();
    let bytes = 1.234f32.to_le_bytes();
    let mut reader = SpanReader::new(&bytes);
    let val = f32::read_from_reader(&db, &mut reader, DataType::Single).unwrap();
    assert!((val - 1.234).abs() < 0.001);
}

#[test]
fn read_bool_from_reader() {
    let b = test_helpers::DcbBuilder::new();
    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();
    let bytes = [1u8];
    let mut reader = SpanReader::new(&bytes);
    let val = bool::read_from_reader(&db, &mut reader, DataType::Boolean).unwrap();
    assert!(val);
}

#[test]
fn read_string_from_reader() {
    let mut b = test_helpers::DcbBuilder::new();
    let sid = b.add_string1("hello world");
    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();
    let bytes = sid.as_bytes().to_vec();
    let mut reader = SpanReader::new(&bytes);
    let val = String::read_from_reader(&db, &mut reader, DataType::String).unwrap();
    assert_eq!(val, "hello world");
}

#[test]
fn read_guid_from_reader() {
    let b = test_helpers::DcbBuilder::new();
    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();
    let guid = CigGuid::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    let bytes = guid.as_bytes().to_vec();
    let mut reader = SpanReader::new(&bytes);
    let val = CigGuid::read_from_reader(&db, &mut reader, DataType::Guid).unwrap();
    assert_eq!(val, guid);
}

#[test]
fn expected_types_string() {
    let types = String::expected_data_types();
    assert!(types.contains(&DataType::String));
    assert!(types.contains(&DataType::Locale));
    assert!(types.contains(&DataType::EnumChoice));
}

#[test]
fn expected_types_i32() {
    let types = i32::expected_data_types();
    assert_eq!(types, &[DataType::Int32]);
}
