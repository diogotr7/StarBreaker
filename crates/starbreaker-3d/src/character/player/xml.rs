//! Small owned XML view shared by the player format readers (CryXmlB or XML).
use super::super::CharacterError;
use std::collections::BTreeMap;
pub(super) type Result<T> = std::result::Result<T, CharacterError>;
pub(super) fn error(message: impl Into<String>) -> CharacterError {
    CharacterError::Loadout(message.into())
}
#[derive(Clone, Debug, Default)]
pub(super) struct Node {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Node>,
}
impl Node {
    pub fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map(String::as_str).unwrap_or("")
    }
    pub fn child(&self, tag: &str) -> Option<&Node> {
        self.children.iter().find(|n| n.tag == tag)
    }
    pub fn number(&self, key: &str, default: f64) -> Result<f64> {
        match self.attrs.get(key) {
            None => Ok(default),
            Some(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| error(format!("invalid {} {key}: {s}", self.tag))),
        }
    }
}
pub(super) fn parse(bytes: &[u8]) -> Result<Node> {
    let converted;
    let bytes = if starbreaker_cryxml::is_cryxmlb(bytes) {
        converted = starbreaker_cryxml::from_bytes(bytes)
            .map_err(|e| error(e.to_string()))?
            .to_string();
        converted.as_bytes()
    } else {
        bytes
    };
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut stack = vec![Node::default()];
    loop {
        use quick_xml::events::Event;
        match reader.read_event().map_err(|e| error(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) => {
                let empty = bytes.get(reader.buffer_position() as usize - 2) == Some(&b'/');
                let mut node = Node {
                    tag: e.name().as_ref().to_owned(),
                    ..Default::default()
                };
                for a in e.attributes() {
                    let a = a.map_err(|e| error(e.to_string()))?;
                    node.attrs.insert(
                        a.key.as_ref().to_owned(),
                        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|e| error(e.to_string()))?
                            .into_owned(),
                    );
                }
                if empty {
                    stack.last_mut().unwrap().children.push(node);
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                if stack.len() < 2 {
                    return Err(error("unbalanced XML"));
                }
                let n = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(n);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 || stack[0].children.len() != 1 {
        return Err(error("expected one XML root"));
    }
    Ok(stack.pop().unwrap().children.remove(0))
}
