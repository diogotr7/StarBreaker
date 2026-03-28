use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::bus::FxBankData;
use super::node_base::{FxEntry, NodeInitialFxParams};
use super::props::{AkPropBundle, AkPropBundleF32, RangedModifiers};

// ---------------------------------------------------------------------------
// State (type 1)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct State {
    pub id: u32,
    pub properties: AkPropBundleF32,
}

impl State {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let properties = AkPropBundleF32::parse(reader)?;
        Ok(State { id, properties })
    }
}

// ---------------------------------------------------------------------------
// DialogueEvent (type 15)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DialogueEvent {
    pub id: u32,
    pub probability: u8,
    pub tree_depth: u32,
    pub group_ids: Vec<u32>,
    pub group_types: Vec<u8>,
    pub tree_data_size: u32,
    pub tree_mode: u8,
    pub decision_tree: Vec<u8>,
    pub properties: AkPropBundle,
    pub ranged_modifiers: RangedModifiers,
}

impl DialogueEvent {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let probability = reader.read_u8()?;

        let tree_depth = reader.read_u32()?;
        let mut group_ids = Vec::with_capacity(tree_depth as usize);
        for _ in 0..tree_depth {
            group_ids.push(reader.read_u32()?);
        }
        let mut group_types = Vec::with_capacity(tree_depth as usize);
        for _ in 0..tree_depth {
            group_types.push(reader.read_u8()?);
        }

        let tree_data_size = reader.read_u32()?;
        let tree_mode = reader.read_u8()?;
        let decision_tree = reader.read_bytes(tree_data_size as usize)?.to_vec();

        let properties = AkPropBundle::parse(reader)?;
        let ranged_modifiers = RangedModifiers::parse(reader)?;

        Ok(DialogueEvent {
            id,
            probability,
            tree_depth,
            group_ids,
            group_types,
            tree_data_size,
            tree_mode,
            decision_tree,
            properties,
            ranged_modifiers,
        })
    }
}

// ---------------------------------------------------------------------------
// AudioDevice (type 21)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AudioDevice {
    pub id: u32,
    pub fx_id: u32,
    pub plugin_params: Vec<u8>,
    pub bank_data: Vec<FxBankData>,
    pub fx_params: NodeInitialFxParams,
}

impl AudioDevice {
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

        // Effect slots
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
        let fx_params = NodeInitialFxParams {
            is_override_parent_fx: false,
            fx_list,
            bypass_all,
        };

        Ok(AudioDevice {
            id,
            fx_id,
            plugin_params,
            bank_data,
            fx_params,
        })
    }
}
