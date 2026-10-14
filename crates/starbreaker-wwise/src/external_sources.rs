//! ATL external-source parser for Wwise media assets.
//!
//! This module indexes `AudioExternalSources` entries from GameAudio XML files
//! and resolves each ATL external-source name to its WEM path.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::Serialize;
use starbreaker_cryxml::{CryXml, CryXmlNode, from_bytes, is_cryxmlb};
use starbreaker_p4k::MappedP4k;

use crate::error::BnkError;

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
pub struct ExternalSourceIndex {
    sources: HashMap<String, Vec<ExternalSourceEntry>>,
}

impl ExternalSourceIndex {
    /// Build the external-source index by scanning `Data\Libs\GameAudio\*.xml`.
    pub fn from_p4k(p4k: &MappedP4k) -> Result<Self, BnkError> {
        let mut sources: HashMap<String, Vec<ExternalSourceEntry>> = HashMap::new();
        let prefix = r"Data\Libs\GameAudio\";

        for entry in p4k.entries() {
            if !entry.name.starts_with(prefix) {
                continue;
            }
            if !entry.name.to_ascii_lowercase().ends_with(".xml") {
                continue;
            }

            let data = match p4k.read(entry) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("warning: failed to read {}: {e}", entry.name);
                    continue;
                }
            };

            let entries = if is_cryxmlb(&data) {
                match from_bytes(&data) {
                    Ok(xml) => parse_external_sources_cryxml(&xml),
                    Err(e) => {
                        eprintln!("warning: failed to parse CryXmlB {}: {e}", entry.name);
                        continue;
                    }
                }
            } else if starts_with_xml(&data) {
                let xml_str = match std::str::from_utf8(strip_bom(&data)) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("warning: non-UTF8 XML in {}: {e}", entry.name);
                        continue;
                    }
                };
                match parse_external_sources_xml(xml_str) {
                    Ok(entries) => entries,
                    Err(e) => {
                        eprintln!("warning: failed to parse external-source XML {}: {e}", entry.name);
                        continue;
                    }
                }
            } else {
                continue;
            };

            for source in entries {
                sources.entry(source.name.clone()).or_default().push(source);
            }
        }

        Ok(Self { sources })
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

    /// Get a localized source by name, preferring English(US) when present.
    pub fn get_preferred(&self, name: &str) -> Option<&ExternalSourceEntry> {
        let entries = self.sources.get(name)?;
        entries
            .iter()
            .find(|entry| entry.language.eq_ignore_ascii_case("English(US)"))
            .or_else(|| entries.first())
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

fn strip_bom(data: &[u8]) -> &[u8] {
    if data.starts_with(b"\xEF\xBB\xBF") {
        &data[3..]
    } else {
        data
    }
}

fn starts_with_xml(data: &[u8]) -> bool {
    let data = strip_bom(data);
    let trimmed = data
        .iter()
        .position(|&b| !b.is_ascii_whitespace())
        .map(|i| &data[i..])
        .unwrap_or(data);
    trimmed.starts_with(b"<")
}

fn cryxml_attr<'a>(xml: &'a CryXml<'_>, node: &CryXmlNode, name: &str) -> Option<&'a str> {
    xml.node_attributes(node)
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
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
                b"AudioExternalSources" => inside_external_sources = true,
                b"ATLExternalSource" if inside_external_sources => {
                    if let Some(name) = get_attr(e, b"atl_name") {
                        current_source = Some(CurrentSource {
                            name,
                            source_path: get_attr(e, b"path").unwrap_or_default(),
                            localized: parse_bool(get_attr(e, b"is_localised").as_deref()),
                        });
                    }
                }
                b"ATLExternalSourceEntry" => {
                    current_language = get_attr(e, b"language");
                }
                _ => {}
            },
            Ok(Event::Empty(ref e)) => {
                if e.local_name().as_ref() == b"WwiseExternalSource" {
                    if let Some(source) = &current_source {
                        if let Some(wwise_filename) = get_attr(e, b"wwise_filename") {
                            entries.push(ExternalSourceEntry {
                                name: source.name.clone(),
                                source_path: source.source_path.clone(),
                                localized: source.localized,
                                language: current_language.clone().unwrap_or_default(),
                                p4k_path: p4k_path_from_wwise_filename(&wwise_filename),
                                wwise_filename,
                                duration_type: get_attr(e, b"wwise_duration_type").unwrap_or_default(),
                                duration_min: parse_f32(get_attr(e, b"wwise_duration_min").as_deref()),
                                duration_max: parse_f32(get_attr(e, b"wwise_duration_max").as_deref()),
                            });
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => match e.local_name().as_ref() {
                b"AudioExternalSources" => inside_external_sources = false,
                b"ATLExternalSource" => current_source = None,
                b"ATLExternalSourceEntry" => current_language = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(BnkError::Xml(format!("{e}"))),
            _ => {}
        }
    }

    Ok(entries)
}

fn get_attr(e: &quick_xml::events::BytesStart<'_>, attr_name: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == attr_name {
            return String::from_utf8(attr.value.to_vec()).ok();
        }
    }
    None
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
