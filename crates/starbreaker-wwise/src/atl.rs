//! ATL (Audio Translation Layer) XML parser for Star Citizen's GameAudio files.
//!
//! Parses `Data\Libs\GameAudio\*.xml` from the P4k archive to build a lookup
//! index from trigger names to their associated Wwise events and bank files.
//!
//! The documents are read by [`crate::game_audio`] (CryXmlB or text XML);
//! [`AtlIndex::add_document`] indexes one, [`AtlIndex::from_p4k`] all of them
//! ([`crate::game_audio::load`] builds this and the external-source index in
//! one scan).

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::Serialize;
use starbreaker_cryxml::{CryXml, CryXmlNode};
use starbreaker_p4k::MappedP4k;

use crate::error::BnkError;
use crate::game_audio::{GameAudioDoc, cryxml_attr, for_each_document, xml_attr as get_attr};

/// A single ATL trigger entry, linking a trigger name to its Wwise event and bank file.
#[derive(Debug, Clone, Serialize)]
pub struct AtlTrigger {
    /// The ATL trigger name (e.g., `"Play_SSTP_AEGS_Gladius_Start_Run"`).
    pub trigger_name: String,
    /// The Wwise event name (usually matches trigger_name).
    pub wwise_event_name: String,
    /// The bank file name (e.g., `"SSTP_AEGS_Gladius_01.bnk"`).
    pub bank_name: String,
    /// Duration type: `"OneShot"` or `"Infinite"`.
    pub duration_type: String,
    /// Maximum attenuation radius, if specified.
    pub radius_max: Option<f32>,
}

/// Index of ATL triggers built from all GameAudio XML files in the P4k.
#[derive(Default)]
pub struct AtlIndex {
    /// trigger_name -> AtlTrigger
    triggers: HashMap<String, AtlTrigger>,
    /// bank_name -> list of trigger_names
    bank_triggers: HashMap<String, Vec<String>>,
}

impl AtlIndex {
    /// Build the ATL index by scanning all `Data\Libs\GameAudio\*.xml` files in the P4k.
    pub fn from_p4k(p4k: &MappedP4k) -> Result<Self, BnkError> {
        let mut index = Self::default();
        for_each_document(p4k, |name, doc| {
            if let Err(e) = index.add_document(doc) {
                eprintln!("warning: failed to parse ATL XML {name}: {e}");
            }
        });
        Ok(index)
    }

    /// Index the triggers of one GameAudio document.
    pub fn add_document(&mut self, doc: &GameAudioDoc<'_>) -> Result<(), BnkError> {
        let file_triggers = match doc {
            GameAudioDoc::Binary(xml) => parse_atl_cryxml(xml),
            GameAudioDoc::Text(xml) => parse_atl_xml(xml)?,
        };
        for trigger in file_triggers {
            self.bank_triggers
                .entry(trigger.bank_name.clone())
                .or_default()
                .push(trigger.trigger_name.clone());
            self.triggers.insert(trigger.trigger_name.clone(), trigger);
        }
        Ok(())
    }

    /// Look up a trigger by exact name.
    pub fn get_trigger(&self, name: &str) -> Option<&AtlTrigger> {
        self.triggers.get(name)
    }

    /// Search for triggers whose names contain the query (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<&AtlTrigger> {
        let query_lower = query.to_ascii_lowercase();
        self.triggers
            .values()
            .filter(|t| t.trigger_name.to_ascii_lowercase().contains(&query_lower))
            .collect()
    }

    /// Get all trigger names associated with a given bank file.
    pub fn triggers_for_bank(&self, bank_name: &str) -> Vec<&str> {
        self.bank_triggers
            .get(bank_name)
            .map(|names| names.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Total number of triggers in the index.
    pub fn len(&self) -> usize {
        self.triggers.len()
    }

    /// Whether the index contains no triggers.
    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }

    /// Iterate over all triggers in the index.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &AtlTrigger)> {
        self.triggers.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Get all unique bank names referenced by triggers.
    pub fn bank_names(&self) -> Vec<&str> {
        self.bank_triggers.keys().map(|s| s.as_str()).collect()
    }
}

// ── CryXmlB parser ──────────────────────────────────────────────────────────

/// Recursively search for a WwiseFile node under a given parent.
fn find_wwise_file_recursive(xml: &CryXml<'_>, node: &CryXmlNode) -> Option<String> {
    for child in xml.node_children(node) {
        if xml.node_tag(child) == "WwiseFile" {
            return cryxml_attr(xml, child, "wwise_name").map(|s| s.to_owned());
        }
        // Recurse into children (e.g., ATLConfigGroup)
        if let Some(found) = find_wwise_file_recursive(xml, child) {
            return Some(found);
        }
    }
    None
}

/// Intermediate trigger data collected during CryXmlB parsing.
struct RawTrigger {
    trigger_name: String,
    wwise_event_name: String,
    duration_type: String,
    radius_max: Option<f32>,
    preload_name: String,
}

/// Parse a CryXmlB ATL document and return all triggers found.
fn parse_atl_cryxml(xml: &CryXml<'_>) -> Vec<AtlTrigger> {
    let root = xml.root();
    let mut raw_triggers: Vec<RawTrigger> = Vec::new();
    let mut preload_banks: HashMap<String, String> = HashMap::new();

    for section in xml.node_children(root) {
        match xml.node_tag(section) {
            "AudioTriggers" => {
                for trigger_node in xml.node_children(section) {
                    if xml.node_tag(trigger_node) != "ATLTrigger" {
                        continue;
                    }
                    let trigger_name = match cryxml_attr(xml, trigger_node, "atl_name") {
                        Some(n) => n.to_owned(),
                        None => continue,
                    };

                    let mut raw = RawTrigger {
                        trigger_name,
                        wwise_event_name: String::new(),
                        duration_type: String::new(),
                        radius_max: None,
                        preload_name: String::new(),
                    };

                    for child in xml.node_children(trigger_node) {
                        match xml.node_tag(child) {
                            "WwiseEvent" => {
                                if let Some(v) = cryxml_attr(xml, child, "wwise_name") {
                                    raw.wwise_event_name = v.to_owned();
                                }
                                if let Some(v) = cryxml_attr(xml, child, "wwise_duration_type") {
                                    raw.duration_type = v.to_owned();
                                }
                                if let Some(v) = cryxml_attr(xml, child, "wwise_radius_max") {
                                    raw.radius_max = v.parse::<f32>().ok();
                                }
                            }
                            "ATLPreload" => {
                                if let Some(v) = cryxml_attr(xml, child, "atl_name") {
                                    raw.preload_name = v.to_owned();
                                }
                            }
                            _ => {}
                        }
                    }

                    raw_triggers.push(raw);
                }
            }
            "AudioPreloads" | "ATLPreloads" => {
                for preload_node in xml.node_children(section) {
                    if xml.node_tag(preload_node) != "ATLPreloadRequest" {
                        continue;
                    }
                    let preload_name = match cryxml_attr(xml, preload_node, "atl_name") {
                        Some(n) => n.to_owned(),
                        None => continue,
                    };

                    // WwiseFile can be nested under ATLConfigGroup or directly
                    if let Some(bank) = find_wwise_file_recursive(xml, preload_node) {
                        preload_banks.insert(preload_name.clone(), bank);
                    }
                }
            }
            _ => {}
        }
    }

    // Resolve preload names to bank file names.
    raw_triggers
        .into_iter()
        .map(|raw| {
            let bank_name = preload_banks
                .get(&raw.preload_name)
                .cloned()
                .unwrap_or_default();
            AtlTrigger {
                trigger_name: raw.trigger_name,
                wwise_event_name: raw.wwise_event_name,
                bank_name,
                duration_type: raw.duration_type,
                radius_max: raw.radius_max,
            }
        })
        .collect()
}

// ── Plain XML fallback (quick-xml) ──────────────────────────────────────────

/// Parse a single plain-text ATL XML file and return all triggers found.
fn parse_atl_xml(xml: &str) -> Result<Vec<AtlTrigger>, BnkError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut raw_triggers: Vec<RawTrigger> = Vec::new();
    let mut preload_banks: HashMap<String, String> = HashMap::new();

    let mut inside_triggers = false;
    let mut current_trigger: Option<RawTrigger> = None;
    let mut current_preload_name: Option<String> = None;
    let mut inside_preloads = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                handle_open_tag(
                    e,
                    false,
                    &mut inside_triggers,
                    &mut inside_preloads,
                    &mut current_trigger,
                    &mut current_preload_name,
                    &mut preload_banks,
                    &mut raw_triggers,
                );
            }
            Ok(Event::Empty(ref e)) => {
                handle_open_tag(
                    e,
                    true,
                    &mut inside_triggers,
                    &mut inside_preloads,
                    &mut current_trigger,
                    &mut current_preload_name,
                    &mut preload_banks,
                    &mut raw_triggers,
                );
            }
            Ok(Event::End(ref e)) => {
                let tag = e.local_name();
                match tag.as_ref() {
                    "AudioTriggers" => inside_triggers = false,
                    "ATLTrigger" => {
                        if let Some(trigger) = current_trigger.take() {
                            if !trigger.trigger_name.is_empty() {
                                raw_triggers.push(trigger);
                            }
                        }
                    }
                    "AudioPreloads" | "ATLPreloads" => inside_preloads = false,
                    "ATLPreloadRequest" => {
                        current_preload_name = None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(BnkError::Xml(format!("{e}"))),
            _ => {}
        }
    }

    // Resolve preload names to bank file names.
    let results = raw_triggers
        .into_iter()
        .map(|raw| {
            let bank_name = preload_banks
                .get(&raw.preload_name)
                .cloned()
                .unwrap_or_default();
            AtlTrigger {
                trigger_name: raw.trigger_name,
                wwise_event_name: raw.wwise_event_name,
                bank_name,
                duration_type: raw.duration_type,
                radius_max: raw.radius_max,
            }
        })
        .collect();

    Ok(results)
}

/// Process an opening or self-closing XML tag.
#[allow(clippy::too_many_arguments)]
fn handle_open_tag(
    e: &quick_xml::events::BytesStart<'_>,
    is_empty: bool,
    inside_triggers: &mut bool,
    inside_preloads: &mut bool,
    current_trigger: &mut Option<RawTrigger>,
    current_preload_name: &mut Option<String>,
    preload_banks: &mut HashMap<String, String>,
    raw_triggers: &mut Vec<RawTrigger>,
) {
    let tag = e.local_name();
    match tag.as_ref() {
        "AudioTriggers" => {
            *inside_triggers = true;
        }
        "ATLTrigger" => {
            if *inside_triggers {
                if let Some(name) = get_attr(e, "atl_name") {
                    let trigger = RawTrigger {
                        trigger_name: name,
                        wwise_event_name: String::new(),
                        duration_type: String::new(),
                        radius_max: None,
                        preload_name: String::new(),
                    };
                    // If self-closing, push immediately; otherwise set as current.
                    if is_empty {
                        raw_triggers.push(trigger);
                    } else {
                        *current_trigger = Some(trigger);
                    }
                }
            }
        }
        "WwiseEvent" => {
            if let Some(trigger) = current_trigger {
                if let Some(name) = get_attr(e, "wwise_name") {
                    trigger.wwise_event_name = name;
                }
                if let Some(dur) = get_attr(e, "wwise_duration_type") {
                    trigger.duration_type = dur;
                }
                if let Some(radius) = get_attr(e, "wwise_radius_max") {
                    trigger.radius_max = radius.parse::<f32>().ok();
                }
            }
        }
        "ATLPreload" => {
            if let Some(trigger) = current_trigger {
                if let Some(preload_name) = get_attr(e, "atl_name") {
                    trigger.preload_name = preload_name;
                }
            }
        }
        "AudioPreloads" | "ATLPreloads" => {
            *inside_preloads = true;
        }
        "ATLPreloadRequest" => {
            if *inside_preloads {
                *current_preload_name = get_attr(e, "atl_name");
            }
        }
        "WwiseFile" => {
            if let Some(preload_name) = current_preload_name {
                if let Some(wwise_name) = get_attr(e, "wwise_name") {
                    preload_banks.insert(preload_name.clone(), wwise_name);
                }
            }
        }
        _ => {}
    }
}
