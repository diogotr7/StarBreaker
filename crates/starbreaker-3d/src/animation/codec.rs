//! Low-level keyframe codec helpers for `.dba` / `.caf` animation data.
//!
//! All functions are `pub(super)` — they are internal to the `animation`
//! module and called by `parse_single_block` in `mod.rs`.

use crate::error::Error;

/// Encoded value sizes and inactive-axis sentinel shared by full/sparse decoders.
pub(super) const UNCOMPRESSED_QUAT_STRIDE: usize = 16;
pub(super) const SMALL_TREE_QUAT_STRIDE: usize = 6;
pub(super) const FLOAT_POSITION_STRIDE: usize = 12;
pub(super) const SNORM_POSITION_STRIDE: usize = 6;
pub(super) const SNORM_HEADER_SIZE: usize = 24;
pub(super) const FLT_MAX_SENTINEL: f32 = 3.0e38;

// ── Time key reading ────────────────────────────────────────────────────────

pub(crate) fn read_time_keys(
    data: &[u8],
    offset: usize,
    count: usize,
    format_flags: u16,
) -> Result<Vec<f32>, Error> {
    let time_format = format_flags & 0x0F;
    match time_format {
        // 1 byte per key, used directly as frame number
        0x00 => {
            if offset + count > data.len() {
                return Err(Error::Other(format!("Time keys overflow at 0x{offset:x}")));
            }
            Ok((0..count).map(|i| data[offset + i] as f32).collect())
        }
        // 2 bytes per key (u16 frame numbers)
        0x01 => {
            let size = count * 2;
            if offset + size > data.len() {
                return Err(Error::Other(format!("Time keys overflow at 0x{offset:x}")));
            }
            Ok((0..count)
                .map(|i| {
                    let o = offset + i * 2;
                    u16::from_le_bytes([data[o], data[o + 1]]) as f32
                })
                .collect())
        }
        // Per-frame keyframe bitmap. Header: start u16, end u16, then a
        // bitmap of (end - start + 1) bits stored LSB-first per byte. Each
        // set bit at index `b` indicates a keyframe at frame `start + b`.
        // The bitmap's first 4 bytes were historically misread as an opaque
        // u32 "marker" and the keys were instead stretched uniformly across
        // [start..end], which produced spurious asymmetric stagger between
        // channels with different keyframe counts (Phase 47).
        //
        // The total number of set bits must equal `count`. We trust the
        // bitmap; if the count disagrees we fall back to uniform stretch.
        0x02 | 0x42 => {
            if offset + 4 > data.len() {
                return Err(Error::Other(format!(
                    "Time header overflow at 0x{offset:x}"
                )));
            }
            let start = u16::from_le_bytes([data[offset], data[offset + 1]]) as u32;
            let end = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as u32;
            if end < start {
                return Err(Error::Other(format!(
                    "Time bitmap end {end} < start {start} at 0x{offset:x}"
                )));
            }
            if count == 0 {
                return Ok(Vec::new());
            }
            let bit_count = (end - start + 1) as usize;
            let byte_count = bit_count.div_ceil(8);
            let bitmap_start = offset + 4;
            if bitmap_start + byte_count > data.len() {
                return Err(Error::Other(format!(
                    "Time bitmap overflow at 0x{offset:x} (need {byte_count} bytes)"
                )));
            }
            let mut times = Vec::with_capacity(count);
            let total_set: u32 = (0..byte_count)
                .map(|i| data[bitmap_start + i].count_ones())
                .sum();
            if total_set as usize != count {
                log::warn!(
                    "Time bitmap at 0x{offset:x} has {total_set} set bits but count={count}; \
                     falling back to uniform stretch over [{start}..{end}]"
                );
                if count == 1 {
                    return Ok(vec![start as f32]);
                }
                let s = start as f32;
                let e = end as f32;
                return Ok((0..count)
                    .map(|i| s + (e - s) * i as f32 / (count - 1) as f32)
                    .collect());
            }
            'outer: for byte_idx in 0..byte_count {
                let b = data[bitmap_start + byte_idx];
                for bit_idx in 0..8 {
                    let frame = byte_idx * 8 + bit_idx;
                    if frame >= bit_count {
                        break 'outer;
                    }
                    if (b >> bit_idx) & 1 == 1 {
                        times.push((start as usize + frame) as f32);
                    }
                }
            }
            debug_assert_eq!(times.len(), count);
            Ok(times)
        }
        _ => {
            log::warn!(
                "Unknown time format 0x{time_format:02x} at offset 0x{offset:x}, using linear 0..N"
            );
            Ok((0..count).map(|i| i as f32).collect())
        }
    }
}

// ── Rotation key reading ────────────────────────────────────────────────────

pub(super) fn read_rotation_keys(
    data: &[u8],
    offset: usize,
    count: usize,
    format_flags: u16,
) -> Result<Vec<[f32; 4]>, Error> {
    let rot_format = format_flags >> 8;
    match rot_format {
        0x80 => read_uncompressed_quats(data, offset, count),
        0x82 => read_small_tree_48bit_quats(data, offset, count),
        _ => {
            log::warn!(
                "Unknown rotation format 0x{rot_format:02x}, falling back to SmallTree48Bit"
            );
            read_small_tree_48bit_quats(data, offset, count)
        }
    }
}

fn read_uncompressed_quats(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 4]>, Error> {
    let size = count * UNCOMPRESSED_QUAT_STRIDE;
    if offset + size > data.len() {
        return Err(Error::Other(format!(
            "Uncompressed quats overflow at 0x{offset:x}"
        )));
    }
    Ok((0..count)
        .map(|i| {
            let o = offset + i * UNCOMPRESSED_QUAT_STRIDE;
            [
                f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
                f32::from_le_bytes([data[o + 4], data[o + 5], data[o + 6], data[o + 7]]),
                f32::from_le_bytes([data[o + 8], data[o + 9], data[o + 10], data[o + 11]]),
                f32::from_le_bytes([data[o + 12], data[o + 13], data[o + 14], data[o + 15]]),
            ]
        })
        .collect())
}

/// SmallTree48BitQuat: 6 bytes (3 × u16) per quaternion.
fn read_small_tree_48bit_quats(
    data: &[u8],
    offset: usize,
    count: usize,
) -> Result<Vec<[f32; 4]>, Error> {
    let size = count * SMALL_TREE_QUAT_STRIDE;
    if offset + size > data.len() {
        return Err(Error::Other(format!(
            "SmallTree48BitQuat overflow at 0x{offset:x}"
        )));
    }
    Ok((0..count)
        .map(|i| {
            let o = offset + i * SMALL_TREE_QUAT_STRIDE;
            let s0 = u16::from_le_bytes([data[o], data[o + 1]]);
            let s1 = u16::from_le_bytes([data[o + 2], data[o + 3]]);
            let s2 = u16::from_le_bytes([data[o + 4], data[o + 5]]);
            decode_small_tree_quat_48(s0, s1, s2)
        })
        .collect())
}

/// Decode SmallTree48BitQuat from 3 × u16. Bit layout confirmed via Ghidra
/// (`FUN_14659d660`): cross-word boundaries with sign-bit borrow.
///
/// Returns `[x, y, z, w]`.
pub(super) fn decode_small_tree_quat_48(s0: u16, s1: u16, s2: u16) -> [f32; 4] {
    const INV_SCALE: f32 = 1.0 / 23170.0;
    const RANGE: f32 = std::f32::consts::FRAC_1_SQRT_2;

    let idx = (s2 >> 14) as usize;

    let raw0 = (s0 & 0x7FFF) as f32 * INV_SCALE - RANGE;
    let raw1 = ((s1 as u32).wrapping_mul(2).wrapping_sub((s0 as i16 >> 15) as u32) & 0x7FFF) as f32
        * INV_SCALE
        - RANGE;
    let raw2_bits = ((s1 >> 14) as u32).wrapping_add((s2 as i16 as i32 as u32).wrapping_mul(4));
    let raw2 = (raw2_bits & 0x7FFF) as f32 * INV_SCALE - RANGE;

    let w_sq = 1.0 - raw0 * raw0 - raw1 * raw1 - raw2 * raw2;
    let largest = if w_sq > 0.0 { w_sq.sqrt() } else { 0.0 };

    const TABLE: [[u8; 3]; 4] = [[1, 2, 3], [0, 2, 3], [0, 1, 3], [0, 1, 2]];
    let slots = TABLE[idx];
    let mut q = [0.0f32; 4];
    q[slots[0] as usize] = raw0;
    q[slots[1] as usize] = raw1;
    q[slots[2] as usize] = raw2;
    q[idx] = largest;
    q
}

// ── Position key reading ────────────────────────────────────────────────────

pub(super) fn read_position_keys(
    data: &[u8],
    offset: usize,
    count: usize,
    format_flags: u16,
) -> Result<Vec<[f32; 3]>, Error> {
    let pos_format = format_flags >> 8;
    match pos_format {
        // Uncompressed float Vec3 (12 bytes per key)
        0xC0 => {
            let size = count * FLOAT_POSITION_STRIDE;
            if offset + size > data.len() {
                return Err(Error::Other(format!(
                    "Float positions overflow at 0x{offset:x}"
                )));
            }
            Ok((0..count)
                .map(|i| {
                    let o = offset + i * FLOAT_POSITION_STRIDE;
                    [
                        f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
                        f32::from_le_bytes([data[o + 4], data[o + 5], data[o + 6], data[o + 7]]),
                        f32::from_le_bytes([
                            data[o + 8],
                            data[o + 9],
                            data[o + 10],
                            data[o + 11],
                        ]),
                    ]
                })
                .collect())
        }
        0xC1 => read_snorm_full_positions(data, offset, count),
        0xC2 => read_snorm_packed_positions(data, offset, count),
        _ => {
            log::warn!("Unknown position format 0x{pos_format:02x}, count={count}");
            Ok(vec![[0.0, 0.0, 0.0]; count])
        }
    }
}

/// SNORM full positions: 24-byte header (scale Vec3 + offset Vec3),
/// then 6 bytes per key (u16 × 3). `value = (f32)u16 * scale + offset`.
fn read_snorm_full_positions(
    data: &[u8],
    offset: usize,
    count: usize,
) -> Result<Vec<[f32; 3]>, Error> {
    if offset + SNORM_HEADER_SIZE + count * SNORM_POSITION_STRIDE > data.len() {
        return Err(Error::Other(format!(
            "SNORM full positions overflow at 0x{offset:x}"
        )));
    }
    let scale = read_vec3(data, offset);
    let pos_offset = read_vec3(data, offset + 12);

    Ok((0..count)
        .map(|i| {
            let o = offset + SNORM_HEADER_SIZE + i * SNORM_POSITION_STRIDE;
            let ux = u16::from_le_bytes([data[o], data[o + 1]]);
            let uy = u16::from_le_bytes([data[o + 2], data[o + 3]]);
            let uz = u16::from_le_bytes([data[o + 4], data[o + 5]]);
            [
                ux as f32 * scale[0] + pos_offset[0],
                uy as f32 * scale[1] + pos_offset[1],
                uz as f32 * scale[2] + pos_offset[2],
            ]
        })
        .collect())
}

/// SNORM packed positions: 24-byte header (scale Vec3 + offset Vec3) followed
/// by **planar (axis-major)** u16 streams — one contiguous `count × u16` array
/// per active axis, in axis order (X, Y, Z, skipping inactive). Inactive
/// channels (`scale == FLT_MAX`) use `offset` directly.
///
/// Layout for `active = [false, true, true]`, `count = 44`:
///
/// ```text
/// [Y0..Y43 as 88 bytes][Z0..Z43 as 88 bytes]
/// ```
///
/// The earlier interleaved (key-major) decode happened to produce correct
/// results for single-active-axis channels (where planar ≡ interleaved), but
/// catastrophically misaligned multi-axis channels (Scorpius `wings_deploy` /
/// `Wing_Grabber_Main_Bottom_Right` was the canonical regression case).
pub(crate) fn read_snorm_packed_positions(
    data: &[u8],
    offset: usize,
    count: usize,
) -> Result<Vec<[f32; 3]>, Error> {
    if offset + SNORM_HEADER_SIZE > data.len() {
        return Err(Error::Other(format!(
            "SNORM packed header overflow at 0x{offset:x}"
        )));
    }
    let scale = read_vec3(data, offset);
    let pos_offset = read_vec3(data, offset + 12);

    let active: [bool; 3] = [
        scale[0].abs() < FLT_MAX_SENTINEL,
        scale[1].abs() < FLT_MAX_SENTINEL,
        scale[2].abs() < FLT_MAX_SENTINEL,
    ];
    let n_active = active.iter().filter(|&&a| a).count();
    let total_bytes = count * n_active * 2;
    let data_start = offset + SNORM_HEADER_SIZE;
    if total_bytes > 0 && data_start + total_bytes > data.len() {
        return Err(Error::Other(format!(
            "SNORM packed positions overflow at 0x{offset:x}"
        )));
    }

    // Per-axis planar offsets: axis `ch` starts at `data_start + axis_idx * count * 2`
    // where `axis_idx` is the active-axis ordinal (0..n_active).
    let mut axis_starts: [usize; 3] = [0; 3];
    {
        let mut next = data_start;
        for ch in 0..3 {
            if active[ch] {
                axis_starts[ch] = next;
                next += count * 2;
            }
        }
    }

    Ok((0..count)
        .map(|i| {
            let mut pos = pos_offset;
            for ch in 0..3 {
                if active[ch] {
                    let o = axis_starts[ch] + i * 2;
                    let uv = u16::from_le_bytes([data[o], data[o + 1]]);
                    pos[ch] = uv as f32 * scale[ch] + pos_offset[ch];
                }
            }
            pos
        })
        .collect())
}

pub(super) fn read_vec3(data: &[u8], offset: usize) -> [f32; 3] {
    [
        f32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]),
        f32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]),
        f32::from_le_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
        ]),
    ]
}
