use starbreaker_common::ColorRgba;
use std::str::FromStr;

#[test]
fn display() {
    let c = ColorRgba::new(255, 128, 0, 200);
    assert_eq!(c.to_string(), "#ff8000c8");
}

#[test]
fn parse() {
    let c = ColorRgba::from_str("#ff8000c8").unwrap();
    assert_eq!(c, ColorRgba::new(255, 128, 0, 200));
}

#[test]
fn round_trip() {
    let original = ColorRgba::new(0x12, 0x34, 0x56, 0x78);
    let s = original.to_string();
    let parsed = ColorRgba::from_str(&s).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn reject_no_prefix() {
    assert!(ColorRgba::from_str("ff8000c8").is_err());
}

#[test]
fn reject_wrong_length() {
    assert!(ColorRgba::from_str("#ff80").is_err());
    assert!(ColorRgba::from_str("#ff8000c8ff").is_err());
}

#[test]
fn reject_invalid_hex() {
    assert!(ColorRgba::from_str("#ZZZZZZZZ").is_err());
}

#[test]
fn serde_round_trip() {
    let original = ColorRgba::new(0xAB, 0xCD, 0xEF, 0x01);
    let json = serde_json::to_string(&original).unwrap();
    assert_eq!(json, "\"#abcdef01\"");
    let parsed: ColorRgba = serde_json::from_str(&json).unwrap();
    assert_eq!(original, parsed);
}
