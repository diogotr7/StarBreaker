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
    let stride = if flags >> 8 == 0x80 {
        codec::UNCOMPRESSED_QUAT_STRIDE
    } else {
        codec::SMALL_TREE_QUAT_STRIDE
    };
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
            offset + (count - 1) * codec::FLOAT_POSITION_STRIDE,
            1,
            flags,
        )?);
        return Ok(values);
    }
    if format != 0xC1 && format != 0xC2 {
        log::warn!("Unknown position format 0x{format:02x}, count={count}");
        return Ok(vec![[0.0; 3]; 2]);
    }
    let header = data
        .get(offset..offset + codec::SNORM_HEADER_SIZE)
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
        copy(
            &mut sparse,
            offset + codec::SNORM_HEADER_SIZE,
            codec::SNORM_POSITION_STRIDE,
        )?;
        copy(
            &mut sparse,
            offset + codec::SNORM_HEADER_SIZE + (count - 1) * codec::SNORM_POSITION_STRIDE,
            codec::SNORM_POSITION_STRIDE,
        )?;
    } else {
        let scale = codec::read_vec3(header, 0);
        let mut axis_start = offset + codec::SNORM_HEADER_SIZE;
        for scale in scale {
            // Same sentinel and planar layout as the full SNORM decoder.
            if scale.abs() < codec::FLT_MAX_SENTINEL {
                copy(&mut sparse, axis_start, 2)?;
                copy(&mut sparse, axis_start + (count - 1) * 2, 2)?;
                axis_start += count * 2;
            }
        }
    }
    codec::read_position_keys(&sparse, 0, 2, flags)
}
