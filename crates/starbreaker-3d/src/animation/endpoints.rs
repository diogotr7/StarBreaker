//! Sparse first/final value decoding for catalogue inspection and clip matching.
//! Time tables remain shared with the full decoder; intermediate values are never decoded.
use super::codec;
use crate::error::Error;

pub(super) fn ends<T: Clone>(values: Vec<T>) -> Vec<T> {
    match values.len() {
        0..=2 => values,
        n => vec![values[0].clone(), values[n - 1].clone()],
    }
}

pub(super) fn rotation_ends(
    data: &[u8],
    offset: usize,
    count: usize,
    flags: u16,
) -> Result<Vec<[f32; 4]>, Error> {
    if count <= 2 {
        return codec::read_rotation_keys(data, offset, count, flags);
    }
    let stride = if flags >> 8 == 0x80 { 16 } else { 6 };
    let mut values = codec::read_rotation_keys(data, offset, 1, flags)?;
    values.extend(codec::read_rotation_keys(
        data,
        offset + (count - 1) * stride,
        1,
        flags,
    )?);
    Ok(values)
}

pub(super) fn position_ends(
    data: &[u8],
    offset: usize,
    count: usize,
    flags: u16,
) -> Result<Vec<[f32; 3]>, Error> {
    if count <= 2 {
        return codec::read_position_keys(data, offset, count, flags);
    }
    let format = flags >> 8;
    if format == 0xC0 {
        let mut values = codec::read_position_keys(data, offset, 1, flags)?;
        values.extend(codec::read_position_keys(
            data,
            offset + (count - 1) * 12,
            1,
            flags,
        )?);
        return Ok(values);
    }
    if format != 0xC1 && format != 0xC2 {
        return Err(Error::Other(format!(
            "Unsupported catalogue position encoding: {format:#x}"
        )));
    }
    let header = data
        .get(offset..offset + 24)
        .ok_or_else(|| Error::Other("Truncated position header".into()))?;
    let mut sparse = header.to_vec();
    let copy = |out: &mut Vec<u8>, start, size| -> Result<(), Error> {
        out.extend_from_slice(
            data.get(start..start + size)
                .ok_or_else(|| Error::Other("Truncated position endpoint".into()))?,
        );
        Ok(())
    };
    if format == 0xC1 {
        copy(&mut sparse, offset + 24, 6)?;
        copy(&mut sparse, offset + 24 + (count - 1) * 6, 6)?;
    } else {
        let scale = codec::read_vec3(header, 0);
        let mut axis_start = offset + 24;
        for scale in scale {
            // Same sentinel and planar layout as the full SNORM decoder.
            if scale.abs() < 3.0e38 {
                copy(&mut sparse, axis_start, 2)?;
                copy(&mut sparse, axis_start + (count - 1) * 2, 2)?;
                axis_start += count * 2;
            }
        }
    }
    codec::read_position_keys(&sparse, 0, 2, flags)
}
