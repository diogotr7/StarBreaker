use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::node_base::{
    AuxParams, NodeInitialFxParams, NodeInitialMetadataParams, PositioningParams,
};
use super::props::AkPropBundle;
use super::rtpc::{InitialRtpc, StateChunk};
use super::vlq::read_vlq;

// ---------------------------------------------------------------------------
// AudioBus (types 8, 18)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DuckEntry {
    pub bus_id: u32,
    pub duck_volume: f32,
    pub fade_out_time: i32,
    pub fade_in_time: i32,
    pub fade_curve: u8,
    pub target_prop: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioBus {
    pub id: u32,
    pub override_bus_id: u32,
    pub device_shareset_id: Option<u32>,
    pub properties: AkPropBundle,
    pub positioning: PositioningParams,
    pub aux_params: AuxParams,
    pub recovery_time: i32,
    pub max_duck_volume: f32,
    pub ducks: Vec<DuckEntry>,
    pub fx_params: NodeInitialFxParams,
    pub metadata_params: NodeInitialMetadataParams,
    pub rtpc: InitialRtpc,
    pub state_chunk: StateChunk,
}

impl AudioBus {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let override_bus_id = reader.read_u32()?;
        let device_shareset_id = if override_bus_id == 0 {
            Some(reader.read_u32()?)
        } else {
            None
        };

        // BusInitialParams
        let properties = AkPropBundle::parse(reader)?;
        let positioning = PositioningParams::parse(reader)?;
        let aux_params = AuxParams::parse(reader)?;

        let recovery_time = reader.read_i32()?;
        let max_duck_volume = reader.read_f32()?;

        let num_ducks = reader.read_u32()? as usize;
        let mut ducks = Vec::with_capacity(num_ducks);
        for _ in 0..num_ducks {
            ducks.push(DuckEntry {
                bus_id: reader.read_u32()?,
                duck_volume: reader.read_f32()?,
                fade_out_time: reader.read_i32()?,
                fade_in_time: reader.read_i32()?,
                fade_curve: reader.read_u8()?,
                target_prop: reader.read_u8()?,
            });
        }

        // BusInitialFxParams (same structure as NodeInitialFxParams but without
        // the override parent byte — we parse it manually)
        let num_fx = reader.read_u8()? as usize;
        let mut bypass_all = false;
        let mut fx_list = Vec::with_capacity(num_fx);
        if num_fx > 0 {
            bypass_all = reader.read_u8()? != 0;
            for _ in 0..num_fx {
                fx_list.push(super::node_base::FxEntry {
                    fx_index: reader.read_u8()?,
                    fx_id: reader.read_u32()?,
                    bits: reader.read_u8()?,
                });
            }
        }
        let fx_params = NodeInitialFxParams {
            is_override_parent_fx: false,
            fx_list,
            bypass_all,
        };

        let metadata_params = NodeInitialMetadataParams::parse(reader)?;

        let rtpc = InitialRtpc::parse(reader)?;
        let state_chunk = StateChunk::parse(reader)?;

        Ok(AudioBus {
            id,
            override_bus_id,
            device_shareset_id,
            properties,
            positioning,
            aux_params,
            recovery_time,
            max_duck_volume,
            ducks,
            fx_params,
            metadata_params,
            rtpc,
            state_chunk,
        })
    }
}

// ---------------------------------------------------------------------------
// FxBase (types 16 FxShareSet, 17 FxCustom)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct FxBankData {
    pub index: u8,
    pub source_id: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct FxStatePropertyValue {
    pub property_id: u32,
    pub rtpc_accum: u8,
    pub value: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct FxBase {
    pub id: u32,
    pub fx_id: u32,
    pub plugin_params: Vec<u8>,
    pub bank_data: Vec<FxBankData>,
    pub rtpc: InitialRtpc,
    pub state_chunk: StateChunk,
    pub state_property_values: Vec<FxStatePropertyValue>,
}

impl FxBase {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let fx_id = reader.read_u32()?;
        let u_size = reader.read_u32()? as usize;
        let plugin_params = reader.read_bytes(u_size)?.to_vec();

        let num_bank_data = reader.read_u8()? as usize;
        let mut bank_data = Vec::with_capacity(num_bank_data);
        for _ in 0..num_bank_data {
            bank_data.push(FxBankData {
                index: reader.read_u8()?,
                source_id: reader.read_u32()?,
            });
        }

        let rtpc = InitialRtpc::parse(reader)?;
        let state_chunk = StateChunk::parse(reader)?;

        // v127+ state property overrides
        let num_values = reader.read_u16()? as usize;
        let mut state_property_values = Vec::with_capacity(num_values);
        for _ in 0..num_values {
            state_property_values.push(FxStatePropertyValue {
                property_id: read_vlq(reader)?,
                rtpc_accum: reader.read_u8()?,
                value: reader.read_f32()?,
            });
        }

        Ok(FxBase {
            id,
            fx_id,
            plugin_params,
            bank_data,
            rtpc,
            state_chunk,
            state_property_values,
        })
    }
}
