use starbreaker_common::CigGuid;
use std::str::FromStr;

#[test]
fn round_trip_display_parse() {
    let original = CigGuid::from_bytes([
        0x78, 0x56, 0x34, 0x12, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0x9A, 0xBC, 0xDE,
        0xF0,
    ]);
    let s = original.to_string();
    let parsed = CigGuid::from_str(&s).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn empty_guid() {
    let empty = CigGuid::EMPTY;
    assert!(empty.is_empty());

    let s = empty.to_string();
    assert_eq!(s, "00000000-0000-0000-0000-000000000000");

    let parsed = CigGuid::from_str(&s).unwrap();
    assert_eq!(empty, parsed);
    assert!(parsed.is_empty());
}

#[test]
fn invalid_format_rejected() {
    assert!(CigGuid::from_str("not-a-guid").is_err());
    assert!(CigGuid::from_str("").is_err());
    assert!(CigGuid::from_str("0123456789abcdef0123456789abcdefxx").is_err());
    // Too short
    assert!(CigGuid::from_str("00000000-0000-0000-0000-0000000000").is_err());
    // Non-hex character
    assert!(CigGuid::from_str("ZZZZZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZZZZZZZZZ").is_err());
}

#[test]
fn serde_round_trip() {
    let original = CigGuid::from_bytes([
        0x78, 0x56, 0x34, 0x12, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0x9A, 0xBC, 0xDE,
        0xF0,
    ]);
    let json = serde_json::to_string(&original).unwrap();
    let parsed: CigGuid = serde_json::from_str(&json).unwrap();
    assert_eq!(original, parsed);
}
