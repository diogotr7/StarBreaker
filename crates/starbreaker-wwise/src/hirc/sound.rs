use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::node_base::NodeBaseParams;
use super::SoundSource;

#[derive(Debug, Clone, Serialize)]
pub struct Sound {
    pub id: u32,
    pub plugin_id: u32,
    pub stream_type: SoundSource,
    pub media_id: u32,
    pub cache_id: u32,
    pub in_memory_size: u32,
    pub source_bits: u8,
    pub node_base: NodeBaseParams,
}

impl Sound {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let plugin_id = reader.read_u32()?;
        let plugin_type = plugin_id & 0x0F;
        let stream_type = SoundSource::from_u8(reader.read_u8()?);
        let media_id = reader.read_u32()?;
        let cache_id = reader.read_u32()?;
        let in_memory_size = reader.read_u32()?;
        let source_bits = reader.read_u8()?;

        if plugin_type == 2 {
            let param_size = reader.read_u32()? as usize;
            reader.advance(param_size)?;
        }

        let node_base = NodeBaseParams::parse(reader)?;

        Ok(Sound { id, plugin_id, stream_type, media_id, cache_id, in_memory_size, source_bits, node_base })
    }

    pub fn is_codec(&self) -> bool {
        (self.plugin_id & 0x0F) == 1
    }

    pub fn is_language_specific(&self) -> bool {
        (self.source_bits & 0x01) != 0
    }
}
