use starbreaker_datacore::enums::{ConversionType, DataType};
use starbreaker_datacore::error::{ExportError, ParseError};
use starbreaker_datacore::starbreaker_common::ParseError as CommonParseError;
use starbreaker_datacore::types::{
    CigGuid, DataMapping, EnumDefinition, Pointer, PropertyDefinition, Record, Reference, StringId,
    StringId2, StructDefinition,
};

// ─── Error display tests ─────────────────────────────────────────────────────

#[test]
fn parse_error_truncated_display() {
    let e: ParseError = CommonParseError::Truncated {
        offset: 0,
        need: 16,
        have: 4,
    }
    .into();
    assert!(e.to_string().contains("need 16 bytes"));
    assert!(e.to_string().contains("have 4"));
}

#[test]
fn parse_error_unsupported_version_display() {
    let e = ParseError::UnsupportedVersion(5);
    assert_eq!(e.to_string(), "unsupported version: 5 (only v6 supported)");
}

#[test]
fn export_error_unknown_data_type_display() {
    let e = ExportError::UnknownDataType(0x1234);
    assert_eq!(e.to_string(), "unknown data type: 0x1234");
}

// ─── DataType discriminant values ────────────────────────────────────────────

#[test]
fn data_type_discriminants() {
    assert_eq!(DataType::Boolean as u16, 0x0001);
    assert_eq!(DataType::SByte as u16, 0x0002);
    assert_eq!(DataType::Int16 as u16, 0x0003);
    assert_eq!(DataType::Int32 as u16, 0x0004);
    assert_eq!(DataType::Int64 as u16, 0x0005);
    assert_eq!(DataType::Byte as u16, 0x0006);
    assert_eq!(DataType::UInt16 as u16, 0x0007);
    assert_eq!(DataType::UInt32 as u16, 0x0008);
    assert_eq!(DataType::UInt64 as u16, 0x0009);
    assert_eq!(DataType::String as u16, 0x000A);
    assert_eq!(DataType::Single as u16, 0x000B);
    assert_eq!(DataType::Double as u16, 0x000C);
    assert_eq!(DataType::Locale as u16, 0x000D);
    assert_eq!(DataType::Guid as u16, 0x000E);
    assert_eq!(DataType::EnumChoice as u16, 0x000F);
    assert_eq!(DataType::Class as u16, 0x0010);
    assert_eq!(DataType::StrongPointer as u16, 0x0110);
    assert_eq!(DataType::WeakPointer as u16, 0x0210);
    assert_eq!(DataType::Reference as u16, 0x0310);
}

#[test]
fn data_type_try_from_valid() {
    assert_eq!(DataType::try_from(0x0001u16).unwrap(), DataType::Boolean);
    assert_eq!(DataType::try_from(0x0310u16).unwrap(), DataType::Reference);
    assert_eq!(DataType::try_from(0x0010u16).unwrap(), DataType::Class);
}

#[test]
fn data_type_try_from_invalid() {
    let result = DataType::try_from(0xFFFFu16);
    assert!(result.is_err());
    if let Err(ExportError::UnknownDataType(v)) = result {
        assert_eq!(v, 0xFFFF);
    } else {
        panic!("expected UnknownDataType error");
    }
}

// ─── ConversionType discriminant values ──────────────────────────────────────

#[test]
fn conversion_type_discriminants() {
    assert_eq!(ConversionType::Attribute as u16, 0x00);
    assert_eq!(ConversionType::ComplexArray as u16, 0x01);
    assert_eq!(ConversionType::SimpleArray as u16, 0x02);
    assert_eq!(ConversionType::ClassArray as u16, 0x03);
}

#[test]
fn conversion_type_try_from_valid() {
    assert_eq!(
        ConversionType::try_from(0u16).unwrap(),
        ConversionType::Attribute
    );
    assert_eq!(
        ConversionType::try_from(3u16).unwrap(),
        ConversionType::ClassArray
    );
}

#[test]
fn conversion_type_try_from_invalid() {
    assert!(ConversionType::try_from(0x0010u16).is_err());
}

// ─── DataType::inline_size ───────────────────────────────────────────────────

#[test]
fn data_type_inline_size() {
    assert_eq!(DataType::Boolean.inline_size(), 1);
    assert_eq!(DataType::SByte.inline_size(), 1);
    assert_eq!(DataType::Byte.inline_size(), 1);
    assert_eq!(DataType::Int16.inline_size(), 2);
    assert_eq!(DataType::UInt16.inline_size(), 2);
    assert_eq!(DataType::Int32.inline_size(), 4);
    assert_eq!(DataType::UInt32.inline_size(), 4);
    assert_eq!(DataType::EnumChoice.inline_size(), 4);
    assert_eq!(DataType::Int64.inline_size(), 8);
    assert_eq!(DataType::UInt64.inline_size(), 8);
    assert_eq!(DataType::Single.inline_size(), 4);
    assert_eq!(DataType::Double.inline_size(), 8);
    assert_eq!(DataType::String.inline_size(), 4);
    assert_eq!(DataType::Locale.inline_size(), 4);
    assert_eq!(DataType::Guid.inline_size(), 16);
    assert_eq!(DataType::StrongPointer.inline_size(), 8);
    assert_eq!(DataType::WeakPointer.inline_size(), 8);
    assert_eq!(DataType::Reference.inline_size(), 20);
    // Class is variable/recursive
    assert_eq!(DataType::Class.inline_size(), 0);
}

// ─── CigGuid ─────────────────────────────────────────────────────────────────

#[test]
fn cig_guid_size() {
    assert_eq!(size_of::<CigGuid>(), 16);
}

#[test]
fn cig_guid_empty() {
    assert!(CigGuid::EMPTY.is_empty());
    let g = CigGuid::from_bytes([0u8; 16]);
    assert!(g.is_empty());
}

#[test]
fn cig_guid_not_empty() {
    let mut bytes = [0u8; 16];
    bytes[0] = 1;
    let g = CigGuid::from_bytes(bytes);
    assert!(!g.is_empty());
}

#[test]
fn cig_guid_equality() {
    let a = CigGuid::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    let b = CigGuid::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    let c = CigGuid::from_bytes([0u8; 16]);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

/// Test the non-standard CigGuid display format:
/// bytes: [b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11, b12, b13, b14, b15]
/// display: b7 b6 b5 b4 - b3 b2 - b1 b0 - b15 b14 - b13 b12 b11 b10 b9 b8
#[test]
fn cig_guid_display_format() {
    // bytes:   [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    //           0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]
    // index:     0     1     2     3     4     5     6     7
    //            8     9    10    11    12    13    14    15
    //
    // Expected: 77665544-3322-1100-ffee-ddccbbaa9988
    let bytes = [
        0x00u8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    let g = CigGuid::from_bytes(bytes);
    assert_eq!(g.to_string(), "77665544-3322-1100-ffee-ddccbbaa9988");
}

// ─── Struct sizes ─────────────────────────────────────────────────────────────

#[test]
fn struct_definition_size() {
    assert_eq!(size_of::<StructDefinition>(), 16);
}

#[test]
fn property_definition_size() {
    assert_eq!(size_of::<PropertyDefinition>(), 12);
}

#[test]
fn enum_definition_size() {
    assert_eq!(size_of::<EnumDefinition>(), 8);
}

#[test]
fn data_mapping_size() {
    assert_eq!(size_of::<DataMapping>(), 8);
}

#[test]
fn record_size() {
    assert_eq!(size_of::<Record>(), 32);
}

#[test]
fn pointer_size() {
    assert_eq!(size_of::<Pointer>(), 8);
}

#[test]
fn reference_size() {
    assert_eq!(size_of::<Reference>(), 20);
}

#[test]
fn string_id_size() {
    assert_eq!(size_of::<StringId>(), 4);
}

#[test]
fn string_id2_size() {
    assert_eq!(size_of::<StringId2>(), 4);
}
