use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::node_base::{parse_children, NodeBaseParams};
use super::rtpc::{InitialRtpc, RtpcGraphPoint};

// Type 5
#[derive(Debug, Clone, Serialize)]
pub struct PlaylistItem {
    pub play_id: u32,
    pub weight: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RanSeqContainer {
    pub id: u32,
    pub node_base: NodeBaseParams,
    pub loop_count: u16,
    pub loop_mod_min: u16,
    pub loop_mod_max: u16,
    pub transition_time: f32,
    pub transition_time_mod_min: f32,
    pub transition_time_mod_max: f32,
    pub avoid_repeat_count: u16,
    pub transition_mode: u8,
    pub random_mode: u8,
    pub mode: u8,
    pub flags: u8,
    pub children: Vec<u32>,
    pub playlist: Vec<PlaylistItem>,
}

impl RanSeqContainer {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let node_base = NodeBaseParams::parse(reader)?;
        let loop_count = reader.read_u16()?;
        let loop_mod_min = reader.read_u16()?;
        let loop_mod_max = reader.read_u16()?;
        let transition_time = reader.read_f32()?;
        let transition_time_mod_min = reader.read_f32()?;
        let transition_time_mod_max = reader.read_f32()?;
        let avoid_repeat_count = reader.read_u16()?;
        let transition_mode = reader.read_u8()?;
        let random_mode = reader.read_u8()?;
        let mode = reader.read_u8()?;
        let flags = reader.read_u8()?;
        let children = parse_children(reader)?;
        let playlist_count = reader.read_u16()? as usize;
        let mut playlist = Vec::with_capacity(playlist_count);
        for _ in 0..playlist_count {
            playlist.push(PlaylistItem {
                play_id: reader.read_u32()?,
                weight: reader.read_i32()?,
            });
        }
        Ok(RanSeqContainer {
            id,
            node_base,
            loop_count,
            loop_mod_min,
            loop_mod_max,
            transition_time,
            transition_time_mod_min,
            transition_time_mod_max,
            avoid_repeat_count,
            transition_mode,
            random_mode,
            mode,
            flags,
            children,
            playlist,
        })
    }
}

// Type 6
#[derive(Debug, Clone, Serialize)]
pub struct SwitchGroup {
    pub switch_id: u32,
    pub node_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchNodeParam {
    pub node_id: u32,
    pub flags: u8,
    pub fade_out_time: i32,
    pub fade_in_time: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchContainer {
    pub id: u32,
    pub node_base: NodeBaseParams,
    pub group_type: u8,
    pub group_id: u32,
    pub default_switch: u32,
    pub is_continuous_validation: bool,
    pub children: Vec<u32>,
    pub switch_groups: Vec<SwitchGroup>,
    pub switch_params: Vec<SwitchNodeParam>,
}

impl SwitchContainer {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let node_base = NodeBaseParams::parse(reader)?;
        let group_type = reader.read_u8()?;
        let group_id = reader.read_u32()?;
        let default_switch = reader.read_u32()?;
        let is_continuous_validation = reader.read_u8()? != 0;
        let children = parse_children(reader)?;

        let num_sg = reader.read_u32()? as usize;
        let mut switch_groups = Vec::with_capacity(num_sg);
        for _ in 0..num_sg {
            let switch_id = reader.read_u32()?;
            let num_items = reader.read_u32()? as usize;
            let mut node_ids = Vec::with_capacity(num_items);
            for _ in 0..num_items {
                node_ids.push(reader.read_u32()?);
            }
            switch_groups.push(SwitchGroup { switch_id, node_ids });
        }

        let num_sp = reader.read_u32()? as usize;
        let mut switch_params = Vec::with_capacity(num_sp);
        for _ in 0..num_sp {
            switch_params.push(SwitchNodeParam {
                node_id: reader.read_u32()?,
                flags: reader.read_u8()?,
                fade_out_time: reader.read_i32()?,
                fade_in_time: reader.read_i32()?,
            });
        }

        Ok(SwitchContainer {
            id,
            node_base,
            group_type,
            group_id,
            default_switch,
            is_continuous_validation,
            children,
            switch_groups,
            switch_params,
        })
    }
}

// Type 7
#[derive(Debug, Clone, Serialize)]
pub struct ActorMixer {
    pub id: u32,
    pub node_base: NodeBaseParams,
    pub children: Vec<u32>,
}

impl ActorMixer {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let node_base = NodeBaseParams::parse(reader)?;
        let children = parse_children(reader)?;
        Ok(ActorMixer { id, node_base, children })
    }
}

// Type 9
#[derive(Debug, Clone, Serialize)]
pub struct LayerAssociation {
    pub child_id: u32,
    pub curve: Vec<RtpcGraphPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Layer {
    pub layer_id: u32,
    pub rtpc: InitialRtpc,
    pub rtpc_id: u32,
    pub rtpc_type: u8,
    pub associations: Vec<LayerAssociation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlendContainer {
    pub id: u32,
    pub node_base: NodeBaseParams,
    pub children: Vec<u32>,
    pub layers: Vec<Layer>,
    pub is_continuous_validation: bool,
}

impl BlendContainer {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let node_base = NodeBaseParams::parse(reader)?;
        let children = parse_children(reader)?;

        let num_layers = reader.read_u32()? as usize;
        let mut layers = Vec::with_capacity(num_layers);
        for _ in 0..num_layers {
            let layer_id = reader.read_u32()?;
            let rtpc = InitialRtpc::parse(reader)?;
            let rtpc_id = reader.read_u32()?;
            let rtpc_type = reader.read_u8()?;
            let num_assoc = reader.read_u32()? as usize;
            let mut associations = Vec::with_capacity(num_assoc);
            for _ in 0..num_assoc {
                let child_id = reader.read_u32()?;
                let curve_size = reader.read_u32()? as usize;
                let mut curve = Vec::with_capacity(curve_size);
                for _ in 0..curve_size {
                    curve.push(RtpcGraphPoint {
                        from: reader.read_f32()?,
                        to: reader.read_f32()?,
                        interp: reader.read_u32()?,
                    });
                }
                associations.push(LayerAssociation { child_id, curve });
            }
            layers.push(Layer { layer_id, rtpc, rtpc_id, rtpc_type, associations });
        }

        let is_continuous_validation = reader.read_u8()? != 0;
        Ok(BlendContainer { id, node_base, children, layers, is_continuous_validation })
    }
}
