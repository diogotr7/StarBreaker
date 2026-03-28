use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::rtpc::{InitialRtpc, RtpcGraphPoint};

// ---------------------------------------------------------------------------
// Attenuation (type 14)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ConeParams {
    pub inside_degrees: f32,
    pub outside_degrees: f32,
    pub outside_volume: f32,
    pub lo_pass: f32,
    pub hi_pass: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttenuationCurve {
    pub scaling: u8,
    pub points: Vec<RtpcGraphPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attenuation {
    pub id: u32,
    pub cone: Option<ConeParams>,
    pub curve_to_use: [i8; 19],
    pub curves: Vec<AttenuationCurve>,
    pub rtpc: InitialRtpc,
}

impl Attenuation {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let is_cone_enabled = reader.read_u8()? != 0;
        let cone = if is_cone_enabled {
            Some(ConeParams {
                inside_degrees: reader.read_f32()?,
                outside_degrees: reader.read_f32()?,
                outside_volume: reader.read_f32()?,
                lo_pass: reader.read_f32()?,
                hi_pass: reader.read_f32()?,
            })
        } else {
            None
        };

        // 19 signed bytes for v142-154
        let mut curve_to_use = [0i8; 19];
        for slot in &mut curve_to_use {
            *slot = reader.read_i8()?;
        }

        let num_curves = reader.read_u8()? as usize;
        let mut curves = Vec::with_capacity(num_curves);
        for _ in 0..num_curves {
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
            curves.push(AttenuationCurve { scaling, points });
        }

        let rtpc = InitialRtpc::parse(reader)?;

        Ok(Attenuation {
            id,
            cone,
            curve_to_use,
            curves,
            rtpc,
        })
    }
}
