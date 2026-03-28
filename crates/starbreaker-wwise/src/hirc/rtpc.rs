use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::props::AkPropBundleF32;
use super::vlq::read_vlq;

#[derive(Debug, Clone, Serialize)]
pub struct RtpcGraphPoint {
    pub from: f32,
    pub to: f32,
    pub interp: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RtpcCurve {
    pub rtpc_id: u32,
    pub rtpc_type: u8,
    pub rtpc_accum: u8,
    pub param_id: u32,
    pub curve_id: u32,
    pub scaling: u8,
    pub points: Vec<RtpcGraphPoint>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InitialRtpc {
    pub curves: Vec<RtpcCurve>,
}

impl InitialRtpc {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let num_curves = reader.read_u16()? as usize;
        let mut curves = Vec::with_capacity(num_curves);
        for _ in 0..num_curves {
            let rtpc_id = reader.read_u32()?;
            let rtpc_type = reader.read_u8()?;
            let rtpc_accum = reader.read_u8()?;
            let param_id = read_vlq(reader)?;
            let curve_id = reader.read_u32()?;
            let scaling = reader.read_u8()?;
            let num_points = reader.read_u16()? as usize;
            let mut points = Vec::with_capacity(num_points);
            for _ in 0..num_points {
                points.push(RtpcGraphPoint {
                    from: reader.read_f32()?,
                    to: reader.read_f32()?,
                    interp: reader.read_u32()?,
                });
            }
            curves.push(RtpcCurve {
                rtpc_id, rtpc_type, rtpc_accum, param_id, curve_id, scaling, points,
            });
        }
        Ok(InitialRtpc { curves })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StateProp {
    pub property_id: u32,
    pub accum_type: u8,
    pub in_db: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateEntry {
    pub state_id: u32,
    pub properties: AkPropBundleF32,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateGroup {
    pub group_id: u32,
    pub sync_type: u8,
    pub states: Vec<StateEntry>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StateChunk {
    pub state_props: Vec<StateProp>,
    pub state_groups: Vec<StateGroup>,
}

impl StateChunk {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let num_state_props = read_vlq(reader)? as usize;
        let mut state_props = Vec::with_capacity(num_state_props);
        for _ in 0..num_state_props {
            let property_id = read_vlq(reader)?;
            let accum_type = reader.read_u8()?;
            let in_db = reader.read_u8()?;
            state_props.push(StateProp { property_id, accum_type, in_db });
        }

        let num_state_groups = read_vlq(reader)? as usize;
        let mut state_groups = Vec::with_capacity(num_state_groups);
        for _ in 0..num_state_groups {
            let group_id = reader.read_u32()?;
            let sync_type = reader.read_u8()?;
            let num_states = read_vlq(reader)? as usize;
            let mut states = Vec::with_capacity(num_states);
            for _ in 0..num_states {
                let state_id = reader.read_u32()?;
                let properties = AkPropBundleF32::parse(reader)?;
                states.push(StateEntry { state_id, properties });
            }
            state_groups.push(StateGroup { group_id, sync_type, states });
        }

        Ok(StateChunk { state_props, state_groups })
    }
}
