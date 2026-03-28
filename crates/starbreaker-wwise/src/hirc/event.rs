use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::vlq::read_vlq;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: u32,
    pub action_ids: Vec<u32>,
}

impl Event {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let count = read_vlq(reader)? as usize;
        let mut action_ids = Vec::with_capacity(count);
        for _ in 0..count {
            action_ids.push(reader.read_u32()?);
        }
        Ok(Event { id, action_ids })
    }
}
