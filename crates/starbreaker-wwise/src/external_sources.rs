//! ATL external-source parser for Wwise media assets.
//!
//! This module indexes `AudioExternalSources` entries from GameAudio XML files
//! (read by [`crate::game_audio`]) and resolves each ATL external-source name,
//! per language, to its WEM path. [`ExternalSourceIndex::add_document`] indexes
//! one document; [`crate::game_audio::load`] builds this and the ATL trigger
//! index in one scan.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::Serialize;
use starbreaker_cryxml::CryXml;
use starbreaker_p4k::MappedP4k;

use crate::error::BnkError;
use crate::game_audio::{GameAudioDoc, cryxml_attr, for_each_document, xml_attr as get_attr};

/// A Wwise file referenced by an ATL external source.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExternalSourceEntry {
    pub name: String,
    pub source_path: String,
    pub localized: bool,
    pub language: String,
    pub wwise_filename: String,
    pub p4k_path: String,
    pub duration_type: String,
    pub duration_min: Option<f32>,
    pub duration_max: Option<f32>,
}

/// Index of ATL external sources built from GameAudio XML files in the P4k.
#[derive(Default)]
pub struct ExternalSourceIndex {
    sources: HashMap<String, Vec<ExternalSourceEntry>>,
}

impl ExternalSourceIndex {
    /// Build the external-source index by scanning `Data\Libs\GameAudio\*.xml`.
    pub fn from_p4k(p4k: &MappedP4k) -> Result<Self, BnkError> {
        let mut index = Self::default();
        for_each_document(p4k, |name, doc| {
            if let Err(e) = index.add_document(doc) {
                eprintln!("warning: failed to parse external-source XML {name}: {e}");
            }
        });
        Ok(index)
    }

    /// Index the external sources of one GameAudio document.
    pub fn add_document(&mut self, doc: &GameAudioDoc<'_>) -> Result<(), BnkError> {
        let entries = match doc {
            GameAudioDoc::Binary(xml) => parse_external_sources_cryxml(xml),
            GameAudioDoc::Text(xml) => parse_external_sources_xml(xml)?,
        };
        for source in entries {
            self.sources.entry(source.name.clone()).or_default().push(source);
        }
        Ok(())
    }

    /// Total number of external-source entries.
    pub fn len(&self) -> usize {
        self.sources.values().map(Vec::len).sum()
    }

    /// Whether the index contains no external-source entries.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Search external-source names, Wwise filenames, languages, and source paths.
    pub fn search(&self, query: &str) -> Vec<&ExternalSourceEntry> {
        let query_lower = query.to_ascii_lowercase();
        let mut results: Vec<&ExternalSourceEntry> = self
            .sources
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|entry| {
                query_lower.is_empty()
                    || entry.name.to_ascii_lowercase().contains(&query_lower)
                    || entry.wwise_filename.to_ascii_lowercase().contains(&query_lower)
                    || entry.source_path.to_ascii_lowercase().contains(&query_lower)
                    || entry.language.to_ascii_lowercase().contains(&query_lower)
            })
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.language.cmp(&b.language)));
        results
    }

    /// The entry for source `name` in `language` (as the ATL entry names it,
    /// e.g. `English(US)`).
    pub fn get(&self, name: &str, language: &str) -> Option<&ExternalSourceEntry> {
        self.sources.get(name)?.iter().find(|entry| entry.language == language)
    }
}

fn p4k_path_from_wwise_filename(filename: &str) -> String {
    let path = filename.replace('/', "\\");
    if path.to_ascii_lowercase().starts_with("data\\") {
        path
    } else {
        format!("Data\\{path}")
    }
}

fn parse_bool(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v.eq_ignore_ascii_case("true") || v == "1")
}

fn parse_f32(value: Option<&str>) -> Option<f32> {
    value.and_then(|v| v.parse::<f32>().ok())
}

fn parse_external_sources_cryxml(xml: &CryXml<'_>) -> Vec<ExternalSourceEntry> {
    let root = xml.root();
    let mut entries = Vec::new();

    for section in xml.node_children(root) {
        if xml.node_tag(section) != "AudioExternalSources" {
            continue;
        }

        for source_node in xml.node_children(section) {
            if xml.node_tag(source_node) != "ATLExternalSource" {
                continue;
            }
            let name = match cryxml_attr(xml, source_node, "atl_name") {
                Some(name) => name.to_owned(),
                None => continue,
            };
            let source_path = cryxml_attr(xml, source_node, "path").unwrap_or_default().to_owned();
            let localized = parse_bool(cryxml_attr(xml, source_node, "is_localised"));

            for entry_node in xml.node_children(source_node) {
                if xml.node_tag(entry_node) != "ATLExternalSourceEntry" {
                    continue;
                }
                let language = cryxml_attr(xml, entry_node, "language")
                    .unwrap_or_default()
                    .to_owned();
                for wwise_node in xml.node_children(entry_node) {
                    if xml.node_tag(wwise_node) != "WwiseExternalSource" {
                        continue;
                    }
                    let wwise_filename = match cryxml_attr(xml, wwise_node, "wwise_filename") {
                        Some(filename) => filename.to_owned(),
                        None => continue,
                    };
                    entries.push(ExternalSourceEntry {
                        name: name.clone(),
                        source_path: source_path.clone(),
                        localized,
                        language: language.clone(),
                        p4k_path: p4k_path_from_wwise_filename(&wwise_filename),
                        wwise_filename,
                        duration_type: cryxml_attr(xml, wwise_node, "wwise_duration_type")
                            .unwrap_or_default()
                            .to_owned(),
                        duration_min: parse_f32(cryxml_attr(xml, wwise_node, "wwise_duration_min")),
                        duration_max: parse_f32(cryxml_attr(xml, wwise_node, "wwise_duration_max")),
                    });
                }
            }
        }
    }

    entries
}

struct CurrentSource {
    name: String,
    source_path: String,
    localized: bool,
}

fn parse_external_sources_xml(xml: &str) -> Result<Vec<ExternalSourceEntry>, BnkError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut entries = Vec::new();
    let mut inside_external_sources = false;
    let mut current_source: Option<CurrentSource> = None;
    let mut current_language: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => match e.local_name().as_ref() {
                "AudioExternalSources" => inside_external_sources = true,
                "ATLExternalSource" if inside_external_sources => {
                    if let Some(name) = get_attr(e, "atl_name") {
                        current_source = Some(CurrentSource {
                            name,
                            source_path: get_attr(e, "path").unwrap_or_default(),
                            localized: parse_bool(get_attr(e, "is_localised").as_deref()),
                        });
                    }
                }
                "ATLExternalSourceEntry" => {
                    current_language = get_attr(e, "language");
                }
                _ => {}
            },
            Ok(Event::Empty(ref e)) => {
                if e.local_name().as_ref() == "WwiseExternalSource" {
                    if let Some(source) = &current_source {
                        if let Some(wwise_filename) = get_attr(e, "wwise_filename") {
                            entries.push(ExternalSourceEntry {
                                name: source.name.clone(),
                                source_path: source.source_path.clone(),
                                localized: source.localized,
                                language: current_language.clone().unwrap_or_default(),
                                p4k_path: p4k_path_from_wwise_filename(&wwise_filename),
                                wwise_filename,
                                duration_type: get_attr(e, "wwise_duration_type").unwrap_or_default(),
                                duration_min: parse_f32(get_attr(e, "wwise_duration_min").as_deref()),
                                duration_max: parse_f32(get_attr(e, "wwise_duration_max").as_deref()),
                            });
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => match e.local_name().as_ref() {
                "AudioExternalSources" => inside_external_sources = false,
                "ATLExternalSource" => current_source = None,
                "ATLExternalSourceEntry" => current_language = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(BnkError::Xml(format!("{e}"))),
            _ => {}
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_xml_external_source() {
        let xml = r#"
<ATLConfig atl_name="ATL_Global_SC_ExternalSources">
  <AudioExternalSources>
    <ATLExternalSource atl_name="DXSM_SSCV_AEGS_OEM_UI_Systems_Bootup" path="ExternalSources/Voices/SC/ShipComputers/DXSM_SSCV_AEGS_OEM_UI" is_localised="true">
      <ATLExternalSourceEntry language="English(US)">
        <WwiseExternalSource wwise_duration_max="2.178" wwise_duration_min="2.178" wwise_duration_type="OneShot" wwise_filename="Sounds/wwise/media/English(US)/ext123.wem" />
      </ATLExternalSourceEntry>
    </ATLExternalSource>
  </AudioExternalSources>
</ATLConfig>"#;

        let entries = parse_external_sources_xml(xml).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "DXSM_SSCV_AEGS_OEM_UI_Systems_Bootup");
        assert_eq!(entries[0].language, "English(US)");
        assert_eq!(entries[0].duration_type, "OneShot");
        assert_eq!(entries[0].duration_max, Some(2.178));
        assert_eq!(
            entries[0].p4k_path,
            "Data\\Sounds\\wwise\\media\\English(US)\\ext123.wem"
        );
    }
}
