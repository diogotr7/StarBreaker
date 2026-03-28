use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::props::{AkPropBundle, RangedModifiers};
use super::rtpc::{InitialRtpc, StateChunk};

#[derive(Debug, Clone, Serialize)]
pub struct FxEntry {
    pub fx_index: u8,
    pub fx_id: u32,
    pub bits: u8,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NodeInitialFxParams {
    pub is_override_parent_fx: bool,
    pub fx_list: Vec<FxEntry>,
    pub bypass_all: bool,
}

impl NodeInitialFxParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let is_override = reader.read_u8()? != 0;
        let num_fx = reader.read_u8()? as usize;
        let mut bypass_all = false;
        let mut fx_list = Vec::with_capacity(num_fx);
        if num_fx > 0 {
            bypass_all = reader.read_u8()? != 0;
            for _ in 0..num_fx {
                fx_list.push(FxEntry {
                    fx_index: reader.read_u8()?,
                    fx_id: reader.read_u32()?,
                    bits: reader.read_u8()?,
                });
            }
        }
        Ok(NodeInitialFxParams { is_override_parent_fx: is_override, fx_list, bypass_all })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MetadataEntry {
    pub fx_index: u8,
    pub fx_id: u32,
    pub is_share_set: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NodeInitialMetadataParams {
    pub is_override_parent: bool,
    pub entries: Vec<MetadataEntry>,
}

impl NodeInitialMetadataParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let is_override = reader.read_u8()? != 0;
        let num = reader.read_u8()? as usize;
        let mut entries = Vec::with_capacity(num);
        for _ in 0..num {
            entries.push(MetadataEntry {
                fx_index: reader.read_u8()?,
                fx_id: reader.read_u32()?,
                is_share_set: reader.read_u8()? != 0,
            });
        }
        Ok(NodeInitialMetadataParams { is_override_parent: is_override, entries })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PathVertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub duration: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathPlaylistItem {
    pub vertices_offset: u32,
    pub num_vertices: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRange {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PositioningParams {
    pub bits_positioning: u8,
    pub bits_3d: Option<u8>,
    pub path_mode: Option<u8>,
    pub transition_time: Option<i32>,
    pub vertices: Vec<PathVertex>,
    pub playlist: Vec<PathPlaylistItem>,
    pub ranges: Vec<PathRange>,
}

impl PositioningParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let bits = reader.read_u8()?;
        let has_positioning = (bits & 0x01) != 0;
        let has_3d = (bits & 0x02) != 0;
        let mut result = PositioningParams { bits_positioning: bits, ..Default::default() };

        if has_positioning && has_3d {
            let bits_3d = reader.read_u8()?;
            result.bits_3d = Some(bits_3d);
            let e3d_pos_type = (bits >> 5) & 3;
            if e3d_pos_type != 0 {
                result.path_mode = Some(reader.read_u8()?);
                result.transition_time = Some(reader.read_i32()?);
                let num_vertices = reader.read_u32()? as usize;
                for _ in 0..num_vertices {
                    result.vertices.push(PathVertex {
                        x: reader.read_f32()?, y: reader.read_f32()?,
                        z: reader.read_f32()?, duration: reader.read_i32()?,
                    });
                }
                let num_playlist = reader.read_u32()? as usize;
                for _ in 0..num_playlist {
                    result.playlist.push(PathPlaylistItem {
                        vertices_offset: reader.read_u32()?,
                        num_vertices: reader.read_u32()?,
                    });
                }
                for _ in 0..num_playlist {
                    result.ranges.push(PathRange {
                        x: reader.read_f32()?, y: reader.read_f32()?, z: reader.read_f32()?,
                    });
                }
            }
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AuxParams {
    pub flags: u8,
    pub aux_bus_ids: [u32; 4],
    pub reflections_aux_bus: u32,
}

impl AuxParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let flags = reader.read_u8()?;
        let has_aux = (flags & 0x08) != 0;
        let mut aux_bus_ids = [0u32; 4];
        if has_aux {
            for id in &mut aux_bus_ids {
                *id = reader.read_u32()?;
            }
        }
        let reflections_aux_bus = reader.read_u32()?;
        Ok(AuxParams { flags, aux_bus_ids, reflections_aux_bus })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AdvSettingsParams {
    pub flags1: u8,
    pub virtual_queue_behavior: u8,
    pub max_num_instance: u16,
    pub below_threshold_behavior: u8,
    pub flags2: u8,
}

impl AdvSettingsParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        Ok(AdvSettingsParams {
            flags1: reader.read_u8()?,
            virtual_queue_behavior: reader.read_u8()?,
            max_num_instance: reader.read_u16()?,
            below_threshold_behavior: reader.read_u8()?,
            flags2: reader.read_u8()?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeBaseParams {
    pub fx_params: NodeInitialFxParams,
    pub metadata_params: NodeInitialMetadataParams,
    pub override_bus_id: u32,
    pub direct_parent_id: u32,
    pub priority_flags: u8,
    pub properties: AkPropBundle,
    pub ranged_modifiers: RangedModifiers,
    pub positioning: PositioningParams,
    pub aux_params: AuxParams,
    pub adv_settings: AdvSettingsParams,
    pub state_chunk: StateChunk,
    pub rtpc: InitialRtpc,
}

impl NodeBaseParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let fx_params = NodeInitialFxParams::parse(reader)?;
        let metadata_params = NodeInitialMetadataParams::parse(reader)?;
        let override_bus_id = reader.read_u32()?;
        let direct_parent_id = reader.read_u32()?;
        let priority_flags = reader.read_u8()?;
        let properties = AkPropBundle::parse(reader)?;
        let ranged_modifiers = RangedModifiers::parse(reader)?;
        let positioning = PositioningParams::parse(reader)?;
        let aux_params = AuxParams::parse(reader)?;
        let adv_settings = AdvSettingsParams::parse(reader)?;
        let state_chunk = StateChunk::parse(reader)?;
        let rtpc = InitialRtpc::parse(reader)?;
        Ok(NodeBaseParams {
            fx_params, metadata_params, override_bus_id, direct_parent_id,
            priority_flags, properties, ranged_modifiers, positioning,
            aux_params, adv_settings, state_chunk, rtpc,
        })
    }
}

/// Parse Children list (shared by containers).
pub fn parse_children(reader: &mut SpanReader) -> Result<Vec<u32>, ParseError> {
    let count = reader.read_u32()? as usize;
    let mut children = Vec::with_capacity(count);
    for _ in 0..count {
        children.push(reader.read_u32()?);
    }
    Ok(children)
}
