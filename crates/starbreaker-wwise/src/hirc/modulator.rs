use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::props::{AkPropBundle, RangedModifiers};
use super::rtpc::InitialRtpc;

// ---------------------------------------------------------------------------
// Modulator (types 19 LfoModulator, 20 EnvelopeModulator, 22 TimeModulator)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Modulator {
    pub id: u32,
    pub properties: AkPropBundle,
    pub ranged_modifiers: RangedModifiers,
    pub rtpc: InitialRtpc,
}

impl Modulator {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let properties = AkPropBundle::parse(reader)?;
        let ranged_modifiers = RangedModifiers::parse(reader)?;
        let rtpc = InitialRtpc::parse(reader)?;

        Ok(Modulator {
            id,
            properties,
            ranged_modifiers,
            rtpc,
        })
    }
}
