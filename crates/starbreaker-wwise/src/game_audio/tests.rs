//! `game_audio` on synthetic GameAudio documents: one pass fills every index.

use super::*;

/// A text GameAudio document with a trigger (and its bank preload) and an
/// external source localized into two languages.
const DOC: &str = r#"<ATLConfig atl_name="Test">
  <AudioTriggers>
    <ATLTrigger atl_name="Play_Test">
      <WwiseEvent wwise_name="Play_Test_Event" wwise_duration_type="OneShot" />
      <ATLPreload atl_name="Test_Preload" />
    </ATLTrigger>
  </AudioTriggers>
  <AudioPreloads>
    <ATLPreloadRequest atl_name="Test_Preload">
      <WwiseFile wwise_name="Test_Bank.bnk" />
    </ATLPreloadRequest>
  </AudioPreloads>
  <AudioExternalSources>
    <ATLExternalSource atl_name="Ship_Voice" path="ExternalSources/Voices" is_localised="true">
      <ATLExternalSourceEntry language="English(US)">
        <WwiseExternalSource wwise_duration_type="OneShot" wwise_filename="Sounds/wwise/media/English(US)/ext1.wem" />
      </ATLExternalSourceEntry>
      <ATLExternalSourceEntry language="German(DE)">
        <WwiseExternalSource wwise_duration_type="OneShot" wwise_filename="Sounds/wwise/media/German(DE)/ext2.wem" />
      </ATLExternalSourceEntry>
    </ATLExternalSource>
  </AudioExternalSources>
</ATLConfig>"#;

#[test]
fn one_pass_fills_triggers_and_external_sources() {
    let doc = parse_document(DOC.as_bytes()).unwrap().expect("text XML is a document");
    let (atl, external) = index_documents([doc]).unwrap();

    let trigger = atl.get_trigger("Play_Test").expect("trigger indexed");
    assert_eq!(trigger.bank_name, "Test_Bank.bnk");
    assert_eq!(external.len(), 2, "one entry per language");
}

/// Each language of an external source resolves to its own WEM.
#[test]
fn external_sources_resolve_by_name_and_language() {
    let doc = parse_document(DOC.as_bytes()).unwrap().unwrap();
    let (_, external) = index_documents([doc]).unwrap();

    let german = external.get("Ship_Voice", "German(DE)").expect("German entry");
    assert_eq!(german.p4k_path, "Data\\Sounds\\wwise\\media\\German(DE)\\ext2.wem");
    let english = external.get("Ship_Voice", "English(US)").expect("English entry");
    assert_eq!(english.p4k_path, "Data\\Sounds\\wwise\\media\\English(US)\\ext1.wem");
    assert!(external.get("Ship_Voice", "French(FR)").is_none());
    assert!(external.get("Other", "English(US)").is_none());
}

/// Text XML (with or without a BOM) is a document; anything else is not.
#[test]
fn documents_are_cryxmlb_or_text_xml() {
    let with_bom = [b"\xEF\xBB\xBF".as_slice(), DOC.as_bytes()].concat();
    assert!(matches!(parse_document(&with_bom), Ok(Some(GameAudioDoc::Text(_)))));
    assert!(matches!(parse_document(b"\x00\x01binary"), Ok(None)));
}

/// An empty search lists every external source (the app shows them before
/// anything is typed), sorted by name then language.
#[test]
fn an_empty_external_source_search_lists_everything() {
    let doc = parse_document(DOC.as_bytes()).unwrap().unwrap();
    let (_, external) = index_documents([doc]).unwrap();
    let all = external.search("");
    let languages: Vec<&str> = all.iter().map(|e| e.language.as_str()).collect();
    assert_eq!(languages, ["English(US)", "German(DE)"]);
}
