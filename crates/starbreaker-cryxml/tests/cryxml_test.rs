// Tests that require extracted game data on disk are ignored by default.
// Run with: cargo test -- --ignored

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_material_file() {
    let data = std::fs::read(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/heads/male/npc/male17/male17_t1_head_material.mtl",
    )
    .unwrap();
    let xml = starbreaker_cryxml::from_bytes(&data).unwrap();
    let text = xml.to_string();
    assert!(
        text.contains("shader_head"),
        "expected shader_head in output"
    );
    assert!(text.contains("MtlFlags"), "expected MtlFlags in output");
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_character_customizer_xml() {
    let data =
        std::fs::read("D:/StarCitizen/P4k-470/Data/Libs/CharacterCustomizer/MasculineDefault.xml")
            .unwrap();
    let xml = starbreaker_cryxml::from_bytes(&data).unwrap();
    let text = xml.to_string();
    assert!(
        text.contains("CharacterCustomization"),
        "expected CharacterCustomization tag"
    );
    assert!(text.contains("dnaString"), "expected dnaString attribute");
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn is_cryxmlb_check() {
    let data = std::fs::read(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/heads/male/npc/male17/male17_t1_head_material.mtl",
    )
    .unwrap();
    assert!(starbreaker_cryxml::is_cryxmlb(&data));
    assert!(!starbreaker_cryxml::is_cryxmlb(b"not cryxml"));
}

#[test]
fn rejects_invalid_data() {
    assert!(starbreaker_cryxml::from_bytes(b"not cryxml data").is_err());
    assert!(starbreaker_cryxml::from_bytes(&[]).is_err());
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn matches_csharp_output() {
    // Parse the binary CryXmlB file.
    let bin =
        std::fs::read("D:/StarCitizen/P4k-470/Data/Libs/CharacterCustomizer/MasculineDefault.xml")
            .unwrap();
    let xml = starbreaker_cryxml::from_bytes(&bin).unwrap();
    let our_output = xml.to_string();

    // Read the C#-converted reference XML.
    let csharp_output = std::fs::read_to_string(
        "D:/StarCitizen/P4k-470/Data/Libs/CharacterCustomizer/MasculineDefault.xml.xml",
    )
    .unwrap();

    // Both outputs should contain the same key content.
    assert!(
        our_output.contains("modelTag=\"Male\""),
        "expected modelTag=\"Male\" in our output"
    );
    assert!(
        csharp_output.contains("modelTag=\"Male\""),
        "expected modelTag=\"Male\" in C# output"
    );
}
