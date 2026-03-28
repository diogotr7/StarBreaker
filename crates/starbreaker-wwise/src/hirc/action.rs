use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::props::{AkPropBundle, RangedModifiers};
use super::vlq::read_vlq;

#[derive(Debug, Clone, Serialize)]
pub enum ActionSpecificParams {
    Play { fade_curve: u8, bank_id: u32, bank_type: u32 },
    Stop { fade_curve: u8, flags: u8, except_ids: Vec<(u32, bool)> },
    Pause { fade_curve: u8, flags: u8, except_ids: Vec<(u32, bool)> },
    Resume { fade_curve: u8, flags: u8, except_ids: Vec<(u32, bool)> },
    SetState { state_group_id: u32, target_state_id: u32 },
    SetSwitch { switch_group_id: u32, switch_state_id: u32 },
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub id: u32,
    pub action_type: u16,
    pub target_id: u32,
    pub is_bus: bool,
    pub properties: AkPropBundle,
    pub ranged_modifiers: RangedModifiers,
    pub specific_params: ActionSpecificParams,
}

impl Action {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let action_type = reader.read_u16()?;
        let target_id = reader.read_u32()?;
        let is_bus_byte = reader.read_u8()?;
        let is_bus = (is_bus_byte & 0x01) != 0;
        let properties = AkPropBundle::parse(reader)?;
        let ranged_modifiers = RangedModifiers::parse(reader)?;

        let action_category = action_type >> 8;
        let specific_params = match action_category {
            0x04 => {
                let fade_curve = reader.read_u8()?;
                let bank_id = reader.read_u32()?;
                let bank_type = reader.read_u32()?;
                ActionSpecificParams::Play { fade_curve, bank_id, bank_type }
            }
            0x01 => {
                let fade_curve = reader.read_u8()?;
                let flags = reader.read_u8()?;
                let except_ids = parse_except_list(reader)?;
                ActionSpecificParams::Stop { fade_curve, flags, except_ids }
            }
            0x02 => {
                let fade_curve = reader.read_u8()?;
                let flags = reader.read_u8()?;
                let except_ids = parse_except_list(reader)?;
                ActionSpecificParams::Pause { fade_curve, flags, except_ids }
            }
            0x03 => {
                let fade_curve = reader.read_u8()?;
                let flags = reader.read_u8()?;
                let except_ids = parse_except_list(reader)?;
                ActionSpecificParams::Resume { fade_curve, flags, except_ids }
            }
            0x12 => ActionSpecificParams::SetState {
                state_group_id: reader.read_u32()?,
                target_state_id: reader.read_u32()?,
            },
            0x19 => ActionSpecificParams::SetSwitch {
                switch_group_id: reader.read_u32()?,
                switch_state_id: reader.read_u32()?,
            },
            _ => ActionSpecificParams::Other,
        };

        Ok(Action { id, action_type, target_id, is_bus, properties, ranged_modifiers, specific_params })
    }

    pub fn is_play(&self) -> bool {
        self.action_type == 0x0403
    }
}

fn parse_except_list(reader: &mut SpanReader) -> Result<Vec<(u32, bool)>, ParseError> {
    let count = read_vlq(reader)? as usize;
    let mut list = Vec::with_capacity(count);
    for _ in 0..count {
        let id = reader.read_u32()?;
        let is_bus = reader.read_u8()? != 0;
        list.push((id, is_bus));
    }
    Ok(list)
}
