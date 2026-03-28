use starbreaker_common::NameHash;

#[test]
fn from_string_lookup() {
    let hash = NameHash::from_string("BaseMelanin");
    assert_eq!(hash.name(), Some("BaseMelanin"));
    assert_eq!(hash.to_string(), "BaseMelanin");
}

#[test]
fn known_hash_by_value() {
    // 0xa98beb34 is manually mapped to "Head Material"
    let hash = NameHash(0xa98beb34);
    assert_eq!(hash.name(), Some("Head Material"));
    assert_eq!(hash.to_string(), "Head Material");
}

#[test]
fn unknown_displays_hex() {
    let hash = NameHash(0xDEADBEEF);
    assert_eq!(hash.name(), None);
    assert_eq!(hash.to_string(), "0xDEADBEEF");
}

#[test]
fn round_trip() {
    // Compute hash from a known name, verify it resolves back
    let hash = NameHash::from_string("shader_Head");
    let display = hash.to_string();
    assert_eq!(display, "shader_Head");

    // The raw u32 should also resolve
    let hash2 = NameHash(hash.0);
    assert_eq!(hash2.name(), Some("shader_Head"));
}

#[test]
fn unknown_name_hashes_consistently() {
    let h1 = NameHash::from_string("some_unknown_name");
    let h2 = NameHash::from_string("some_unknown_name");
    assert_eq!(h1, h2);
    assert_eq!(h1.value(), h2.value());
}

#[test]
fn serde_round_trip_known_name() {
    let original = NameHash::from_string("DyeAmount");
    let json = serde_json::to_string(&original).unwrap();
    // Should serialize as the name string
    assert_eq!(json, "\"DyeAmount\"");
    let parsed: NameHash = serde_json::from_str(&json).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn serde_deserialize_hex() {
    let json = "\"0xDEADBEEF\"";
    let parsed: NameHash = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.value(), 0xDEADBEEF);
}
