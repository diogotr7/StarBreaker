//! GameAudio documents (`Data\Libs\GameAudio\*.xml`) and the indexes built
//! from them.
//!
//! The files are CryXmlB (binary XML) in the P4k, a few plain text XML.
//! [`parse_document`] reads either into a [`GameAudioDoc`];
//! [`for_each_document`] scans the P4k once, and [`load`] fills both the
//! ATL trigger index ([`AtlIndex`]) and the external-source index
//! ([`ExternalSourceIndex`]) from that one scan ([`index_documents`] for
//! documents in hand). Attribute helpers shared by both parsers:
//! [`cryxml_attr`], [`xml_attr`].

use quick_xml::events::BytesStart;
use starbreaker_cryxml::{CryXml, CryXmlNode, from_bytes, is_cryxmlb};
use starbreaker_p4k::MappedP4k;

use crate::atl::AtlIndex;
use crate::error::BnkError;
use crate::external_sources::ExternalSourceIndex;

/// The P4k directory holding the GameAudio documents.
const GAME_AUDIO_DIR: &str = r"Data\Libs\GameAudio\";

/// One GameAudio document, binary or text.
pub enum GameAudioDoc<'a> {
    Binary(CryXml<'a>),
    Text(&'a str),
}

/// Read a GameAudio file: CryXmlB, or text XML (a UTF-8 BOM allowed).
/// `None` for anything else.
pub fn parse_document(data: &[u8]) -> Result<Option<GameAudioDoc<'_>>, BnkError> {
    if is_cryxmlb(data) {
        return from_bytes(data)
            .map(|xml| Some(GameAudioDoc::Binary(xml)))
            .map_err(|e| BnkError::Xml(format!("CryXmlB: {e}")));
    }
    let text = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data);
    if !text.trim_ascii_start().starts_with(b"<") {
        return Ok(None);
    }
    std::str::from_utf8(text)
        .map(|xml| Some(GameAudioDoc::Text(xml)))
        .map_err(|e| BnkError::Xml(format!("non-UTF-8 XML: {e}")))
}

/// Call `f` with every GameAudio document in the P4k (with its P4k path).
/// Files that cannot be read or parsed are reported and skipped.
pub fn for_each_document(p4k: &MappedP4k, mut f: impl FnMut(&str, &GameAudioDoc<'_>)) {
    for entry in p4k.entries() {
        if !entry.name.starts_with(GAME_AUDIO_DIR) || !entry.name.to_ascii_lowercase().ends_with(".xml") {
            continue;
        }
        let data = match p4k.read(entry) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("warning: failed to read {}: {e}", entry.name);
                continue;
            }
        };
        match parse_document(&data) {
            Ok(Some(doc)) => f(&entry.name, &doc),
            Ok(None) => {}
            Err(e) => eprintln!("warning: {}: {e}", entry.name),
        }
    }
}

/// Both GameAudio indexes from one scan of the P4k.
pub fn load(p4k: &MappedP4k) -> Result<(AtlIndex, ExternalSourceIndex), BnkError> {
    let mut atl = AtlIndex::default();
    let mut external = ExternalSourceIndex::default();
    for_each_document(p4k, |name, doc| {
        if let Err(e) = atl.add_document(doc).and_then(|()| external.add_document(doc)) {
            eprintln!("warning: {name}: {e}");
        }
    });
    Ok((atl, external))
}

/// Both GameAudio indexes from `docs`.
pub fn index_documents<'a>(
    docs: impl IntoIterator<Item = GameAudioDoc<'a>>,
) -> Result<(AtlIndex, ExternalSourceIndex), BnkError> {
    let mut atl = AtlIndex::default();
    let mut external = ExternalSourceIndex::default();
    for doc in docs {
        atl.add_document(&doc)?;
        external.add_document(&doc)?;
    }
    Ok((atl, external))
}

/// A CryXmlB node's attribute.
pub(crate) fn cryxml_attr<'a>(xml: &'a CryXml<'_>, node: &CryXmlNode, name: &str) -> Option<&'a str> {
    xml.node_attributes(node).find(|(k, _)| *k == name).map(|(_, v)| v)
}

/// A text XML element's attribute (UTF-8).
pub(crate) fn xml_attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|attr| attr.key.as_ref() == name)
        .map(|attr| attr.value.into_owned())
}

#[cfg(test)]
mod tests;
