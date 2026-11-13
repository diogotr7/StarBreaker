//! CAF (Animation Clip) and shared full/sparse block-parsing helpers.
//!
//! Both `.dba` and `.caf` use the same block format internally;
//! [`parse_animation_blocks_mode`] and [`parse_single_block`] are called from
//! [`super::dba`] as well. A block that fails to decode stays in the block list
//! as `None`, so a block's index is always its position in the file: DBA
//! metadata entry `i` names block `i`.

use super::chunk_types;
use super::codec;
use super::{AnimationClip, AnimationDatabase, BoneChannel, Keyframe};
use crate::error::Error;
use starbreaker_chunks::ChunkFile;

/// Raw controller entry from the animation block (24 bytes).
#[derive(Debug, Clone, Copy)]
struct ControllerEntry {
    num_rot_keys: u16,
    rot_format_flags: u16,
    rot_time_offset: u32,
    rot_data_offset: u32,
    num_pos_keys: u16,
    pos_format_flags: u16,
    pos_time_offset: u32,
    pos_data_offset: u32,
}

/// Minimal info from the IvoAnimInfo chunk.
pub(super) struct AnimInfo {
    pub fps: u16,
}

pub(super) fn parse_anim_info(data: &[u8]) -> AnimInfo {
    AnimInfo {
        fps: if data.len() >= 6 {
            u16::from_le_bytes([data[4], data[5]])
        } else {
            30
        },
    }
}

/// Parse a `.caf` file from raw bytes.
pub fn parse_caf(data: &[u8]) -> Result<AnimationDatabase, Error> {
    parse_caf_mode(data, false)
}

pub(super) fn parse_caf_mode(data: &[u8], endpoints: bool) -> Result<AnimationDatabase, Error> {
    let chunk_file = ChunkFile::from_bytes(data)?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return Err(Error::UnsupportedFormat),
    };

    let anim_info = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == chunk_types::ANIM_INFO)
        .map(|c| parse_anim_info(ivo.chunk_data(c)));
    let fps = anim_info.filter(|i| i.fps != 0).map(|i| i.fps as f32).unwrap_or(30.0);

    let caf_chunk = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == chunk_types::CAF_DATA)
        .ok_or_else(|| Error::Other("No CAF data chunk found".into()))?;

    let data_bytes = ivo.chunk_data(caf_chunk);
    let blocks = parse_animation_blocks_mode(data_bytes, endpoints)?;

    let clips = blocks
        .into_iter()
        .enumerate()
        .filter_map(|(i, channels)| Some((i, channels?)))
        .map(|(i, channels)| AnimationClip {
            name: format!("clip_{i}"),
            fps,
            channels,
            // `.caf` files have no DBA metadata block, so the data-backed
            // start anchor is unavailable here. The addon falls back to
            // first-sample anchoring for these clips (see
            // `_apply_best_channel_transform` in `package_ops.py`).
            start_rotation: None,
            start_position: None,
        })
        .collect();

    Ok(AnimationDatabase { clips })
}

// ── Block parsing ───────────────────────────────────────────────────────────

/// One entry per block header in file order; `None` marks a block whose
/// controllers could not be decoded (logged), keeping later blocks at their index.
pub(super) fn parse_animation_blocks_mode(
    data: &[u8], endpoints: bool,
) -> Result<Vec<Option<Vec<BoneChannel>>>, Error> {
    let mut blocks = Vec::new();
    let mut offset = 0usize;

    // DBA: first 4 bytes is total data size.
    if data.len() >= 4 {
        let total_size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if total_size > 0 && total_size <= data.len() {
            offset = 4; // skip total size field
        }
    }

    while offset + 12 <= data.len() {
        let sig = &data[offset..offset + 4];
        if sig != b"#caf" && sig != b"#dba" {
            break;
        }

        let bone_count = u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
        let _magic = u16::from_le_bytes([data[offset + 6], data[offset + 7]]);
        let _data_size = u32::from_le_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
        ]) as usize;

        let block_start = offset + 12;
        let headers_end = block_start + bone_count * 4 + bone_count * 24;

        match parse_single_block(data, block_start, bone_count, endpoints) {
            Ok(channels) => blocks.push(Some(channels)),
            Err(e) => {
                log::warn!("Failed to parse animation block at 0x{offset:x}: {e}");
                blocks.push(None);
            }
        }

        offset = headers_end;
    }

    Ok(blocks)
}

fn parse_single_block(
    data: &[u8],
    start: usize,
    bone_count: usize,
    endpoints: bool,
) -> Result<Vec<BoneChannel>, Error> {
    let mut pos = start;

    // Bone hash array: bone_count × u32.
    let hash_size = bone_count * 4;
    if pos + hash_size > data.len() {
        return Err(Error::Other("Bone hash array extends past block".into()));
    }
    let bone_hashes: Vec<u32> = (0..bone_count)
        .map(|i| {
            let o = pos + i * 4;
            u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
        })
        .collect();
    pos += hash_size;

    // Controller entries: bone_count × 24 bytes.
    let ctrl_size = bone_count * 24;
    if pos + ctrl_size > data.len() {
        return Err(Error::Other("Controller entries extend past block".into()));
    }
    let mut controllers: Vec<(usize, ControllerEntry)> = Vec::with_capacity(bone_count);
    for i in 0..bone_count {
        let o = pos + i * 24;
        controllers.push((
            o,
            ControllerEntry {
                num_rot_keys: u16::from_le_bytes([data[o], data[o + 1]]),
                rot_format_flags: u16::from_le_bytes([data[o + 2], data[o + 3]]),
                rot_time_offset: u32::from_le_bytes([
                    data[o + 4],
                    data[o + 5],
                    data[o + 6],
                    data[o + 7],
                ]),
                rot_data_offset: u32::from_le_bytes([
                    data[o + 8],
                    data[o + 9],
                    data[o + 10],
                    data[o + 11],
                ]),
                num_pos_keys: u16::from_le_bytes([data[o + 12], data[o + 13]]),
                pos_format_flags: u16::from_le_bytes([data[o + 14], data[o + 15]]),
                pos_time_offset: u32::from_le_bytes([
                    data[o + 16],
                    data[o + 17],
                    data[o + 18],
                    data[o + 19],
                ]),
                pos_data_offset: u32::from_le_bytes([
                    data[o + 20],
                    data[o + 21],
                    data[o + 22],
                    data[o + 23],
                ]),
            },
        ));
    }

    let mut channels = Vec::with_capacity(bone_count);
    for (i, (ctrl_offset, ctrl)) in controllers.iter().enumerate() {
        let base = *ctrl_offset;

        let rotations = if ctrl.num_rot_keys > 0 {
            let times = if ctrl.rot_time_offset > 0 {
                codec::read_time_keys(
                    data,
                    base + ctrl.rot_time_offset as usize,
                    ctrl.num_rot_keys as usize,
                    ctrl.rot_format_flags,
                )?
            } else {
                (0..ctrl.num_rot_keys as usize).map(|t| t as f32).collect()
            };
            let values = if endpoints {
                super::endpoints::rotation_ends(data, base + ctrl.rot_data_offset as usize,
                    ctrl.num_rot_keys as usize, ctrl.rot_format_flags)?
            } else {
                codec::read_rotation_keys(data, base + ctrl.rot_data_offset as usize,
                    ctrl.num_rot_keys as usize, ctrl.rot_format_flags)?
            };
            let times = if endpoints { super::endpoints::ends(times) } else { times };
            times
                .into_iter()
                .zip(values)
                .map(|(t, v)| Keyframe { time: t, value: v })
                .collect()
        } else {
            Vec::new()
        };

        let positions = if ctrl.num_pos_keys > 0 {
            let times = if ctrl.pos_time_offset > 0 {
                codec::read_time_keys(
                    data,
                    base + ctrl.pos_time_offset as usize,
                    ctrl.num_pos_keys as usize,
                    ctrl.pos_format_flags,
                )?
            } else {
                (0..ctrl.num_pos_keys as usize).map(|t| t as f32).collect()
            };
            let values = if endpoints {
                super::endpoints::position_ends(data, base + ctrl.pos_data_offset as usize,
                    ctrl.num_pos_keys as usize, ctrl.pos_format_flags)?
            } else {
                codec::read_position_keys(data, base + ctrl.pos_data_offset as usize,
                    ctrl.num_pos_keys as usize, ctrl.pos_format_flags)?
            };
            let times = if endpoints { super::endpoints::ends(times) } else { times };
            times
                .into_iter()
                .zip(values)
                .map(|(t, v)| Keyframe { time: t, value: v })
                .collect()
        } else {
            Vec::new()
        };

        channels.push(BoneChannel {
            bone_hash: bone_hashes[i],
            rotations,
            positions,
            rot_format_flags: ctrl.rot_format_flags,
            pos_format_flags: ctrl.pos_format_flags,
        });
    }

    Ok(channels)
}
