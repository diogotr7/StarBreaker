use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

#[derive(Debug, Clone, Serialize)]
pub struct AkProp {
    pub id: u8,
    pub value: u32,
}

impl AkProp {
    pub fn as_f32(&self) -> f32 {
        f32::from_bits(self.value)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AkPropBundle {
    pub props: Vec<AkProp>,
}

impl AkPropBundle {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let count = reader.read_u8()? as usize;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            ids.push(reader.read_u8()?);
        }
        let mut props = Vec::with_capacity(count);
        for i in 0..count {
            props.push(AkProp {
                id: ids[i],
                value: reader.read_u32()?,
            });
        }
        Ok(AkPropBundle { props })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RangedModifier {
    pub id: u8,
    pub min: u32,
    pub max: u32,
}

impl RangedModifier {
    pub fn min_f32(&self) -> f32 { f32::from_bits(self.min) }
    pub fn max_f32(&self) -> f32 { f32::from_bits(self.max) }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RangedModifiers {
    pub modifiers: Vec<RangedModifier>,
}

impl RangedModifiers {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let count = reader.read_u8()? as usize;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            ids.push(reader.read_u8()?);
        }
        let mut modifiers = Vec::with_capacity(count);
        for i in 0..count {
            let min = reader.read_u32()?;
            let max = reader.read_u32()?;
            modifiers.push(RangedModifier { id: ids[i], min, max });
        }
        Ok(RangedModifiers { modifiers })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AkPropBundleF32 {
    pub props: Vec<(u16, f32)>,
}

impl AkPropBundleF32 {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let count = reader.read_u16()? as usize;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            ids.push(reader.read_u16()?);
        }
        let mut props = Vec::with_capacity(count);
        for i in 0..count {
            props.push((ids[i], reader.read_f32()?));
        }
        Ok(AkPropBundleF32 { props })
    }
}
