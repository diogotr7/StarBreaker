use std::fmt::{self, Write};

use starbreaker_common::SpanReader;

use crate::error::CryXmlError;
use crate::types::{CryXmlAttribute, CryXmlHeader, CryXmlNode};

const MAGIC: &[u8; 8] = b"CryXmlB\0";

/// A parsed CryXmlB document that borrows directly from the input bytes.
pub struct CryXml<'a> {
    nodes: &'a [CryXmlNode],
    child_indices: &'a [i32],
    attributes: &'a [CryXmlAttribute],
    string_data: &'a [u8],
}

/// Returns `true` if `data` begins with the `CryXmlB\0` magic.
pub fn is_cryxmlb(data: &[u8]) -> bool {
    data.len() > MAGIC.len() && data[..MAGIC.len()] == *MAGIC
}

/// Parse a CryXmlB binary blob into a zero-copy [`CryXml`] handle.
pub fn from_bytes(data: &[u8]) -> Result<CryXml<'_>, CryXmlError> {
    if data.len() < MAGIC.len() || data[..MAGIC.len()] != *MAGIC {
        return Err(CryXmlError::InvalidMagic);
    }

    let mut reader = SpanReader::new(data);
    // Skip past the magic we already validated.
    reader.advance(MAGIC.len())?;

    let header: &CryXmlHeader = reader.read_type()?;

    let node_count = header.node_count as usize;
    let attribute_count = header.attribute_count as usize;
    let child_count = header.child_count as usize;
    let string_data_size = header.string_data_size as usize;

    let nodes = reader.read_slice::<CryXmlNode>(node_count)?;
    let child_indices = reader.read_slice::<i32>(child_count)?;
    let attributes = reader.read_slice::<CryXmlAttribute>(attribute_count)?;
    let string_data = reader.read_bytes(string_data_size)?;

    Ok(CryXml {
        nodes,
        child_indices,
        attributes,
        string_data,
    })
}

impl<'a> CryXml<'a> {
    /// The root node of the document (always index 0).
    pub fn root(&self) -> &CryXmlNode {
        &self.nodes[0]
    }

    /// Resolve the tag name for a node.
    pub fn node_tag(&self, node: &CryXmlNode) -> &'a str {
        self.get_string(node.tag_string_offset)
    }

    /// Iterate over `(key, value)` attribute pairs for a node.
    pub fn node_attributes(
        &self,
        node: &CryXmlNode,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + '_ {
        let start = node.first_attribute_index as usize;
        let count = node.attribute_count as usize;
        self.attributes[start..start + count].iter().map(|attr| {
            let key = self.get_string(attr.key_string_offset);
            let val = self.get_string(attr.value_string_offset);
            (key, val)
        })
    }

    /// Iterate over the direct child nodes.
    pub fn node_children(&self, node: &CryXmlNode) -> impl Iterator<Item = &'a CryXmlNode> + '_ {
        let start = node.first_child_index as usize;
        let count = node.child_count as usize;
        self.child_indices[start..start + count]
            .iter()
            .map(|&idx| &self.nodes[idx as usize])
    }

    /// Resolve a null-terminated string from the string data section.
    fn get_string(&self, offset: u32) -> &'a str {
        let data = &self.string_data[offset as usize..];
        let len = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        if len == 0 {
            return "";
        }
        // The data is ASCII so this should always succeed.
        std::str::from_utf8(&data[..len]).unwrap_or("")
    }

    // ── XML text output ──────────────────────────────────────────────────

    fn write_node(&self, buf: &mut String, node_index: usize, depth: usize) {
        let node = &self.nodes[node_index];
        let tag = self.get_string(node.tag_string_offset);
        let indent = "  ".repeat(depth);

        let _ = write!(buf, "{indent}<{tag}");

        // Write attributes.
        let attr_start = node.first_attribute_index as usize;
        let attr_count = node.attribute_count as usize;
        for attr in &self.attributes[attr_start..attr_start + attr_count] {
            let key = self.get_string(attr.key_string_offset);
            let val = self.get_string(attr.value_string_offset);
            let _ = write!(buf, " {key}=\"{}\"", escape_xml_attr(val));
        }

        let child_start = node.first_child_index as usize;
        let child_count = node.child_count as usize;

        if child_count == 0 {
            buf.push_str(" />\n");
        } else {
            buf.push_str(">\n");
            for &child_idx in &self.child_indices[child_start..child_start + child_count] {
                self.write_node(buf, child_idx as usize, depth + 1);
            }
            let _ = writeln!(buf, "{indent}</{tag}>");
        }
    }
}

impl fmt::Display for CryXml<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = String::new();
        self.write_node(&mut buf, 0, 0);
        f.write_str(&buf)
    }
}

/// Escape the five XML-special characters inside attribute values.
fn escape_xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}
