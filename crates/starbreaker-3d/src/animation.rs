//! Parser for `.dba` (Animation Database) and `.caf` (Animation Clip) IVO
//! files.
//!
//! Both formats use IVO container with animation blocks. A `.dba` packs
//! multiple clips, while `.caf` has a single clip.
//!
//! ## Block structure
//!
//! ```text
//! Header (12 bytes): signature("#caf"/"#dba") + bone_count(u16) + magic(u16) + data_size(u32)
//! Bone hashes:    [u32; bone_count]  — CRC32 of bone names
//! Controllers:    [ControllerEntry; bone_count]  — 24 bytes each (rot track + pos track)
//! Keyframe data at offsets referenced by controllers (relative to each controller's own offset)
//! ```
//!
//! Cross-validated against the reference implementation on
//! `diogotr7/StarBreaker` (commit
//! [`d01ae21`](https://github.com/diogotr7/StarBreaker/commit/d01ae217fb74bebf1fede7cd45a82b758f44cbb6)
//! on branch `feature/animation`) and the gate test for the Scorpius rear
//! gear (`docs/StarBreaker/animation-research.md`). The `SmallTree48BitQuat`
//! decoder follows the Ghidra-confirmed bit layout (sign-bit borrow across
//! u16 boundaries).

use std::collections::{HashMap, HashSet};

use starbreaker_chunks::ChunkFile;

use crate::error::Error;

// ── Public types ────────────────────────────────────────────────────────────

/// A parsed animation database containing one or more animation clips.
#[derive(Debug, Clone)]
pub struct AnimationDatabase {
    pub clips: Vec<AnimationClip>,
}

/// A single animation clip with per-bone channels.
#[derive(Debug, Clone)]
pub struct AnimationClip {
    /// Animation name (from DBA metadata, or filename for CAF).
    pub name: String,
    /// Frames per second (from metadata, default 30).
    pub fps: f32,
    /// Per-bone animation channels.
    pub channels: Vec<BoneChannel>,
}

/// Animation data for a single bone.
#[derive(Debug, Clone)]
pub struct BoneChannel {
    /// CRC32 hash of the bone name.
    pub bone_hash: u32,
    /// Rotation keyframes (time in frames, quaternion XYZW).
    pub rotations: Vec<Keyframe<[f32; 4]>>,
    /// Position keyframes (time in frames, XYZ).
    pub positions: Vec<Keyframe<[f32; 3]>>,
}

/// A single keyframe with time and value.
#[derive(Debug, Clone)]
pub struct Keyframe<T> {
    pub time: f32,
    pub value: T,
}

// ── Internal types ──────────────────────────────────────────────────────────

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

/// DBA metadata entry (48 = 0x30 bytes per animation, v0x902).
#[derive(Debug)]
#[allow(dead_code)]
struct DbaMetaEntry {
    fps: u16,
    /// Expected number of bone controllers in the matching block.
    num_controllers: u16,
    /// End frame from metadata entry.
    end_frame: u32,
    /// Start-frame reference rotation (xyzw quaternion in CryEngine space).
    /// Retained for future cross-validation; the current matcher uses
    /// 1:1 index alignment (see Phase 27 in animation-research.md).
    start_rotation: [f32; 4],
}

/// IVO chunk type IDs for animation data.
mod chunk_types {
    pub const DBA_DATA: u32 = 0x194FBC50; // IvoDBAData
    pub const DBA_META: u32 = 0xF7351608; // IvoDBAMetadata
    pub const CAF_DATA: u32 = 0xA9496CB5; // IvoCAFData
    pub const ANIM_INFO: u32 = 0x4733C6ED; // IvoAnimInfo
}

// ── Parsing entry points ────────────────────────────────────────────────────

/// Parse a `.dba` file from raw bytes.
pub fn parse_dba(data: &[u8]) -> Result<AnimationDatabase, Error> {
    let chunk_file = ChunkFile::from_bytes(data)?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return Err(Error::UnsupportedFormat),
    };

    let db_data_chunk = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == chunk_types::DBA_DATA)
        .ok_or_else(|| Error::Other("No DBA data chunk found".into()))?;
    let db_meta_chunk = ivo.chunks().iter().find(|c| c.chunk_type == chunk_types::DBA_META);

    // Use file data from chunk offset (not bounded chunk_data) because DBA
    // controller offsets can reference keyframe data that extends past the
    // IVO chunk boundary.
    let data_bytes = &ivo.file_data()[db_data_chunk.offset as usize..];
    let meta_entries = db_meta_chunk
        .map(|c| parse_dba_metadata(ivo.chunk_data(c)))
        .unwrap_or_default();

    let mut blocks = parse_animation_blocks(data_bytes)?;
    if !meta_entries.is_empty() && blocks.len() > meta_entries.len() {
        log::warn!(
            "DBA parse produced {} blocks but metadata lists {}; truncating to metadata count",
            blocks.len(),
            meta_entries.len()
        );
        blocks.truncate(meta_entries.len());
    }

    let clips = match_dba_metadata_to_blocks(blocks, &meta_entries);

    Ok(AnimationDatabase { clips })
}

fn match_dba_metadata_to_blocks(
    blocks: Vec<Vec<BoneChannel>>,
    meta_entries: &[(String, DbaMetaEntry)],
) -> Vec<AnimationClip> {
    if blocks.is_empty() {
        return Vec::new();
    }
    if meta_entries.is_empty() {
        return blocks
            .into_iter()
            .enumerate()
            .map(|(i, channels)| AnimationClip {
                name: format!("anim_{i}"),
                fps: 30.0,
                channels,
            })
            .collect();
    }

    // Authoritative mapping: DBA metadata entries are 1:1 index-aligned with
    // animation blocks. Verified empirically on Scorpius.dba (2026-04-27): all
    // 55 metadata entries match their corresponding block by num_controllers,
    // including the wings_deploy and rsi_scorpius_lg_deploy_r blocks that
    // earlier heuristic matchers misassigned. See
    // docs/StarBreaker/animation-research.md "Phase 27 — DBA metadata layout
    // corrected" for the byte-level decoding evidence.
    //
    // Mismatches in num_controllers between metadata and block at the same
    // index indicate either a parser bug or a corrupt DBA. Log a warning and
    // fall back to a positional name so the clip is still exported.
    let mut clips: Vec<AnimationClip> = Vec::with_capacity(blocks.len());
    for (i, channels) in blocks.into_iter().enumerate() {
        let (name, fps) = match meta_entries.get(i) {
            Some((name, meta)) => {
                if (meta.num_controllers as usize) != channels.len() {
                    log::warn!(
                        "[anim] DBA metadata[{i}] '{name}' nctrl={} disagrees with block channels={}; \
                         keeping index-aligned name but parser may have decoded entry size incorrectly",
                        meta.num_controllers,
                        channels.len()
                    );
                }
                let clip_name = if name.trim().is_empty() {
                    format!("anim_{i}")
                } else {
                    name.clone()
                };
                let fps = if meta.fps == 0 { 30.0 } else { meta.fps as f32 };
                (clip_name, fps)
            }
            None => (format!("anim_{i}"), 30.0),
        };
        clips.push(AnimationClip { name, fps, channels });
    }
    clips
}


/// Parse a `.caf` file from raw bytes.
pub fn parse_caf(data: &[u8]) -> Result<AnimationDatabase, Error> {
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
    let fps = anim_info.map(|i| i.fps as f32).unwrap_or(30.0);

    let caf_chunk = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == chunk_types::CAF_DATA)
        .ok_or_else(|| Error::Other("No CAF data chunk found".into()))?;

    let data_bytes = ivo.chunk_data(caf_chunk);
    let blocks = parse_animation_blocks(data_bytes)?;

    let clips = blocks
        .into_iter()
        .enumerate()
        .map(|(i, channels)| AnimationClip {
            name: format!("clip_{i}"),
            fps,
            channels,
        })
        .collect();

    Ok(AnimationDatabase { clips })
}

// ── Block parsing ───────────────────────────────────────────────────────────

fn parse_animation_blocks(data: &[u8]) -> Result<Vec<Vec<BoneChannel>>, Error> {
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

        match parse_single_block(data, block_start, bone_count) {
            Ok(channels) => blocks.push(channels),
            Err(e) => log::warn!("Failed to parse animation block at 0x{offset:x}: {e}"),
        }

        offset = headers_end;
    }

    Ok(blocks)
}

fn parse_single_block(
    data: &[u8],
    start: usize,
    bone_count: usize,
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
                read_time_keys(
                    data,
                    base + ctrl.rot_time_offset as usize,
                    ctrl.num_rot_keys as usize,
                    ctrl.rot_format_flags,
                )?
            } else {
                (0..ctrl.num_rot_keys as usize).map(|t| t as f32).collect()
            };
            let values = read_rotation_keys(
                data,
                base + ctrl.rot_data_offset as usize,
                ctrl.num_rot_keys as usize,
                ctrl.rot_format_flags,
            )?;
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
                read_time_keys(
                    data,
                    base + ctrl.pos_time_offset as usize,
                    ctrl.num_pos_keys as usize,
                    ctrl.pos_format_flags,
                )?
            } else {
                (0..ctrl.num_pos_keys as usize).map(|t| t as f32).collect()
            };
            let values = read_position_keys(
                data,
                base + ctrl.pos_data_offset as usize,
                ctrl.num_pos_keys as usize,
                ctrl.pos_format_flags,
            )?;
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
        });
    }

    Ok(channels)
}

// ── DBA metadata parsing ────────────────────────────────────────────────────

fn parse_dba_metadata(data: &[u8]) -> Vec<(String, DbaMetaEntry)> {
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let entry_size = 48; // 0x30
    let entries_end = 4 + count * entry_size;
    if entries_end > data.len() {
        log::warn!(
            "DBA metadata: {} entries × {} bytes = {} exceeds chunk size {}",
            count,
            entry_size,
            entries_end,
            data.len()
        );
        return Vec::new();
    }

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let o = 4 + i * entry_size;
        // Layout (v0x902, 48 bytes per entry, empirically verified against
        // Scorpius.dba 2026-04-27 — see docs/StarBreaker/animation-research.md
        // "Phase 27 — DBA metadata layout corrected"):
        //   +0x00 (4) flags0 (often 0; sometimes float weight)
        //   +0x04 (4) flags1 (0 or small int)
        //   +0x08 (2) fps        (u16, e.g. 30 = 0x001E)
        //   +0x0A (2) num_controllers (u16, == bone_count of the matching block)
        //   +0x0C (4) version    (always 0x00000900 in v0x902)
        //   +0x10 (4) reserved
        //   +0x14 (4) end_frame  (u32, frame count of the clip)
        //   +0x18 (16) start_rotation (f32×4 quaternion XYZW)
        //   +0x28 (8)  start_position trailing — only 8 of 12 bytes fit; the
        //              last component is implicit / unused for matching.
        // Block ordering is identical to metadata ordering; matching is by
        // index (see match_dba_metadata_to_blocks below).
        let fps = u16::from_le_bytes([data[o + 8], data[o + 9]]);
        let num_controllers = u16::from_le_bytes([data[o + 10], data[o + 11]]);
        let end_frame = u32::from_le_bytes([data[o + 20], data[o + 21], data[o + 22], data[o + 23]]);
        let start_rotation = [
            f32::from_le_bytes(data[o + 24..o + 28].try_into().unwrap_or([0; 4])),
            f32::from_le_bytes(data[o + 28..o + 32].try_into().unwrap_or([0; 4])),
            f32::from_le_bytes(data[o + 32..o + 36].try_into().unwrap_or([0; 4])),
            f32::from_le_bytes(data[o + 36..o + 40].try_into().unwrap_or([0; 4])),
        ];
        entries.push(DbaMetaEntry {
            fps,
            num_controllers,
            end_frame,
            start_rotation,
        });
    }

    // Names region is preceded by alignment-padding NUL bytes (observed in
    // Scorpius.dba: 4 leading NULs to align the first name to an 8-byte
    // boundary). Skip leading NULs so we land on the first real name.
    let mut pos = entries_end;
    while pos < data.len() && data[pos] == 0 {
        pos += 1;
    }

    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        if pos >= data.len() {
            names.push(String::new());
            continue;
        }
        let end = data[pos..]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(data.len() - pos);
        let name = std::str::from_utf8(&data[pos..pos + end])
            .unwrap_or("")
            .to_string();
        names.push(name);
        pos += end + 1;
    }

    names.into_iter().zip(entries).collect()
}

struct AnimInfo {
    fps: u16,
}

fn parse_anim_info(data: &[u8]) -> AnimInfo {
    AnimInfo {
        fps: if data.len() >= 6 {
            u16::from_le_bytes([data[4], data[5]])
        } else {
            30
        },
    }
}

// ── Time key reading ────────────────────────────────────────────────────────

fn read_time_keys(
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
        // 8-byte header (start u16 + end u16 + marker u32), interpolate linearly
        0x02 | 0x42 => {
            if offset + 8 > data.len() {
                return Err(Error::Other(format!(
                    "Time header overflow at 0x{offset:x}"
                )));
            }
            let start = u16::from_le_bytes([data[offset], data[offset + 1]]) as f32;
            let end = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as f32;
            if count <= 1 {
                return Ok(vec![start]);
            }
            Ok((0..count)
                .map(|i| start + (end - start) * i as f32 / (count - 1) as f32)
                .collect())
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

fn read_rotation_keys(
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
    let size = count * 16;
    if offset + size > data.len() {
        return Err(Error::Other(format!(
            "Uncompressed quats overflow at 0x{offset:x}"
        )));
    }
    Ok((0..count)
        .map(|i| {
            let o = offset + i * 16;
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
    let size = count * 6;
    if offset + size > data.len() {
        return Err(Error::Other(format!(
            "SmallTree48BitQuat overflow at 0x{offset:x}"
        )));
    }
    Ok((0..count)
        .map(|i| {
            let o = offset + i * 6;
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
fn decode_small_tree_quat_48(s0: u16, s1: u16, s2: u16) -> [f32; 4] {
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

fn read_position_keys(
    data: &[u8],
    offset: usize,
    count: usize,
    format_flags: u16,
) -> Result<Vec<[f32; 3]>, Error> {
    let pos_format = format_flags >> 8;
    match pos_format {
        // Uncompressed float Vec3 (12 bytes per key)
        0xC0 => {
            let size = count * 12;
            if offset + size > data.len() {
                return Err(Error::Other(format!(
                    "Float positions overflow at 0x{offset:x}"
                )));
            }
            Ok((0..count)
                .map(|i| {
                    let o = offset + i * 12;
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
    if offset + 24 + count * 6 > data.len() {
        return Err(Error::Other(format!(
            "SNORM full positions overflow at 0x{offset:x}"
        )));
    }
    let scale = read_vec3(data, offset);
    let pos_offset = read_vec3(data, offset + 12);

    Ok((0..count)
        .map(|i| {
            let o = offset + 24 + i * 6;
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

/// SNORM packed positions: 24-byte header + variable u16 per active channel.
/// Inactive channels have `scale == FLT_MAX` and use `offset` directly.
fn read_snorm_packed_positions(
    data: &[u8],
    offset: usize,
    count: usize,
) -> Result<Vec<[f32; 3]>, Error> {
    if offset + 24 > data.len() {
        return Err(Error::Other(format!(
            "SNORM packed header overflow at 0x{offset:x}"
        )));
    }
    let scale = read_vec3(data, offset);
    let pos_offset = read_vec3(data, offset + 12);

    const FLT_MAX_SENTINEL: f32 = 3.0e38;
    let active: [bool; 3] = [
        scale[0].abs() < FLT_MAX_SENTINEL,
        scale[1].abs() < FLT_MAX_SENTINEL,
        scale[2].abs() < FLT_MAX_SENTINEL,
    ];
    let bytes_per_key: usize = active.iter().filter(|&&a| a).count() * 2;
    let data_start = offset + 24;
    if bytes_per_key > 0 && data_start + count * bytes_per_key > data.len() {
        return Err(Error::Other(format!(
            "SNORM packed positions overflow at 0x{offset:x}"
        )));
    }

    Ok((0..count)
        .map(|i| {
            let o = data_start + i * bytes_per_key;
            let mut pos = pos_offset;
            let mut byte_offset = 0;
            for ch in 0..3 {
                if active[ch] {
                    let uv = u16::from_le_bytes([
                        data[o + byte_offset],
                        data[o + byte_offset + 1],
                    ]);
                    pos[ch] = uv as f32 * scale[ch] + pos_offset[ch];
                    byte_offset += 2;
                }
            }
            pos
        })
        .collect())
}

fn read_vec3(data: &[u8], offset: usize) -> [f32; 3] {
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

// ── High-level helpers ──────────────────────────────────────────────────────

/// Final-frame local TRS pose for a single bone.
#[derive(Debug, Clone, Copy)]
pub struct BonePose {
    /// Local rotation as quaternion in Blender Z-up `wxyz` order.
    pub rotation: [f32; 4],
    /// Local position in Blender Z-up.
    pub position: Option<[f32; 3]>,
}

/// Convert a quaternion produced by [`decode_small_tree_quat_48`] (CryEngine
/// Y-up `xyzw` convention) into the Blender Z-up `wxyz` form used by our
/// pipeline.
///
/// Empirically derived from the Scorpius rear-gear gate test
/// (see `docs/StarBreaker/animation-research.md`):
/// `blender_wxyz = (w, y, -z, x)` matched the user-aligned deployed-pose
/// target at 2.54° on the foot bone and held smoothly across all 50
/// keyframes of `lg_deploy_r`.
pub fn cry_xyzw_to_blender_wxyz(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    [w, y, -z, x]
}

/// Read a single named animation from a `.dba` and return final-frame local
/// TRS keyed by bone CRC32 hash, ready to overwrite `Bone.local_*` in the
/// pipeline.
///
/// `animation_name` is matched against the metadata strings stored in the
/// DBA chunk (case-insensitive substring; e.g. pass
/// `"rsi_scorpius_lg_deploy_r"` to find the deploy track).
///
/// **Caveat:** in some DBAs the metadata names and the actual block contents
/// are not 1:1 aligned (see `docs/StarBreaker/animation-research.md`). Prefer
/// [`find_block_for_skeleton`] for production use; this helper is kept for
/// debugging.
pub fn read_dba_final_pose(
    dba_bytes: &[u8],
    animation_name: &str,
) -> Result<HashMap<u32, BonePose>, Error> {
    let db = parse_dba(dba_bytes)?;
    let needle = animation_name.to_ascii_lowercase();

    let clip = db
        .clips
        .iter()
        .find(|c| c.name.to_ascii_lowercase().contains(&needle))
        .ok_or_else(|| Error::Other(format!("Animation '{animation_name}' not found in DBA")))?;

    Ok(clip_final_pose(clip))
}

/// Build a final-frame `BonePose` map from a single animation clip.
///
/// Quaternions are converted to Blender Z-up `wxyz` via
/// [`cry_xyzw_to_blender_wxyz`] and positions get the same axis swap.
pub fn clip_final_pose(clip: &AnimationClip) -> HashMap<u32, BonePose> {
    let mut poses = HashMap::with_capacity(clip.channels.len());
    for ch in &clip.channels {
        let rotation = ch
            .rotations
            .last()
            .map(|kf| cry_xyzw_to_blender_wxyz(kf.value))
            .unwrap_or([1.0, 0.0, 0.0, 0.0]);
        let position = ch.positions.last().map(|kf| {
            let [x, y, z] = kf.value;
            [y, -z, x]
        });
        poses.insert(ch.bone_hash, BonePose { rotation, position });
    }
    poses
}

// ── Block selection by skeleton signature ───────────────────────────────────

/// Pick the best matching animation clip in `db` for a given skeleton, by
/// bone-hash signature.
///
/// `skeleton_bone_hashes` is the set of bone CRC32 hashes from the parsed
/// `.chr`. A clip is a *candidate* iff every one of its channel bone hashes is
/// present in the skeleton (i.e. clip bones ⊆ skeleton bones). The first such
/// clip is returned, with the option to break ties by selecting the clip with
/// the **largest angular delta** between its first and last keyframe — useful
/// to pick "deploy" over "compress" when multiple gear animations share the
/// same bone subset.
///
/// Returns `None` if no candidate exists.
///
/// This bypasses the (currently broken) metadata→block name alignment.
pub fn find_block_for_skeleton<'a>(
    db: &'a AnimationDatabase,
    skeleton_bone_hashes: &std::collections::HashSet<u32>,
    prefer_longest_arc: bool,
) -> Option<&'a AnimationClip> {
    let candidates: Vec<&AnimationClip> = db
        .clips
        .iter()
        .filter(|c| {
            !c.channels.is_empty()
                && c.channels
                    .iter()
                    .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }
    if !prefer_longest_arc || candidates.len() == 1 {
        return Some(candidates[0]);
    }

    // Score = sum across channels of (1 - |first·last|) on rotation.
    candidates
        .into_iter()
        .map(|c| (clip_arc_score(c), c))
        .fold(None, |acc, (s, c)| match acc {
            None => Some((s, c)),
            Some((bs, _)) if s > bs => Some((s, c)),
            other => other,
        })
        .map(|(_, c)| c)
}

/// Sum of angular deltas between first and last rotation key across all
/// channels (radians, sign-invariant). Higher = more motion.
fn clip_arc_score(clip: &AnimationClip) -> f32 {
    let mut total = 0.0f32;
    for ch in &clip.channels {
        if let (Some(first), Some(last)) = (ch.rotations.first(), ch.rotations.last()) {
            let a = first.value;
            let b = last.value;
            let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs();
            total += 1.0 - dot.clamp(0.0, 1.0);
        }
    }
    total
}

// ── Skeleton baking ─────────────────────────────────────────────────────────

/// Quaternion multiplication on `wxyz` quaternions (Blender convention).
fn quat_mul_wxyz(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [aw, ax, ay, az] = a;
    let [bw, bx, by, bz] = b;
    [
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    ]
}

/// Rotate a 3-vector by a `wxyz` unit quaternion.
fn quat_rotate_vec_wxyz(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    // qvq^-1 with q = (w, x, y, z); for unit quat conjugate has negated xyz.
    let [w, x, y, z] = q;
    let [vx, vy, vz] = v;
    // t = 2 * (xyz × v)
    let tx = 2.0 * (y * vz - z * vy);
    let ty = 2.0 * (z * vx - x * vz);
    let tz = 2.0 * (x * vy - y * vx);
    [
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    ]
}

/// Bone-like accessor: minimal interface needed to bake a pose. We use a
/// trait so `apply_pose_to_skeleton` can stay in this crate without a
/// circular dep on `crate::skeleton`.
pub trait BoneTransforms {
    fn name(&self) -> &str;
    fn parent_index(&self) -> Option<usize>;
    fn local_rotation_wxyz(&self) -> [f32; 4];
    fn local_position(&self) -> [f32; 3];
    fn set_local_rotation_wxyz(&mut self, q: [f32; 4]);
    fn set_local_position(&mut self, p: [f32; 3]);
    fn set_world_rotation_wxyz(&mut self, q: [f32; 4]);
    fn set_world_position(&mut self, p: [f32; 3]);
}

/// Compute the CRC32 hash that DBA uses for a bone name.
///
/// CryEngine uses standard CRC32 (zlib polynomial) on the **case-preserved**
/// UTF-8 byte sequence of the bone name (no terminator).
pub fn bone_name_hash(name: &str) -> u32 {
    crc32fast::hash(name.as_bytes())
}

/// Apply a final-frame `pose` to a slice of bones, overwriting both their
/// local TRS and their cached world TRS for any bone whose CRC32 hash is
/// present in `pose`.
///
/// Bones not in `pose` keep their original local transform but have their
/// world transform recomputed from the (possibly updated) parent chain so
/// the hierarchy stays consistent.
///
/// Returns the number of bones that were updated.
pub fn apply_pose_to_skeleton<B: BoneTransforms>(
    bones: &mut [B],
    pose: &HashMap<u32, BonePose>,
) -> usize {
    // Step 1: overwrite locals in-place.
    let mut updated = 0usize;
    for bone in bones.iter_mut() {
        let h = bone_name_hash(bone.name());
        if let Some(p) = pose.get(&h) {
            bone.set_local_rotation_wxyz(p.rotation);
            if let Some(pos) = p.position {
                bone.set_local_position(pos);
            }
            updated += 1;
        }
    }
    if updated == 0 {
        return 0;
    }

    // Step 2: recompute world transforms top-down.
    // We assume parents come before children in `bones`, but defensively
    // iterate fixed-point style: compute world for any bone whose parent
    // already has a recomputed world (or is None).
    let n = bones.len();
    let mut world_q: Vec<[f32; 4]> = vec![[1.0, 0.0, 0.0, 0.0]; n];
    let mut world_p: Vec<[f32; 3]> = vec![[0.0; 3]; n];
    let mut done: Vec<bool> = vec![false; n];

    let mut progress = true;
    while progress {
        progress = false;
        for i in 0..n {
            if done[i] {
                continue;
            }
            let parent = bones[i].parent_index();
            let (pq, pp) = match parent {
                None => ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                Some(pi) if pi == i => ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                Some(pi) if pi < n && done[pi] => (world_q[pi], world_p[pi]),
                Some(_) => continue,
            };
            let lq = bones[i].local_rotation_wxyz();
            let lp = bones[i].local_position();
            world_q[i] = quat_mul_wxyz(pq, lq);
            let rotated = quat_rotate_vec_wxyz(pq, lp);
            world_p[i] = [pp[0] + rotated[0], pp[1] + rotated[1], pp[2] + rotated[2]];
            done[i] = true;
            progress = true;
        }
    }

    for i in 0..n {
        if done[i] {
            bones[i].set_world_rotation_wxyz(world_q[i]);
            bones[i].set_world_position(world_p[i]);
        }
    }

    updated
}

// ── JSON serialization ──────────────────────────────────────────────────────

/// Convert an animation clip to sidecar JSON format.
///
/// Quaternions are converted to Blender Z-up `wxyz` convention.
/// Returns a serde_json Value ready for inclusion in the scene manifest.
pub fn clip_to_json(clip: &AnimationClip) -> serde_json::Value {
    let mut bones = serde_json::json!({});

    for channel in &clip.channels {
        let has_rotation = !channel.rotations.is_empty();
        let has_position = !channel.positions.is_empty();

        if !has_rotation && !has_position {
            continue; // Skip empty channels
        }

        let mut rotation_array = vec![];
        for keyframe in &channel.rotations {
            let q = cry_xyzw_to_blender_wxyz(keyframe.value);
            rotation_array.push(serde_json::json!([q[0], q[1], q[2], q[3]]));
        }

        let mut position_array = vec![];
        for keyframe in &channel.positions {
            let p = keyframe.value;
            // Apply CryEngine Y-up → Blender Z-up axis swap: (x, y, z) → (y, -z, x)
            position_array.push(serde_json::json!([p[1], -p[2], p[0]]));
        }

        let bone_key = format!("0x{:X}", channel.bone_hash);
        bones[bone_key] = serde_json::json!({
            "has_rotation": has_rotation,
            "has_position": has_position,
            "rotation": rotation_array,
            "position": position_array,
        });
    }

    // Calculate frame count from both rotation and position keyframes
    let mut max_frame = 0u32;
    for channel in &clip.channels {
        for keyframe in &channel.rotations {
            max_frame = max_frame.max(keyframe.time.ceil() as u32);
        }
        for keyframe in &channel.positions {
            max_frame = max_frame.max(keyframe.time.ceil() as u32);
        }
    }

    serde_json::json!({
        "name": clip.name,
        "fps": clip.fps as u32,
        "frame_count": max_frame,
        "bones": bones,
    })
}

/// Convert a full database to a JSON array of animations.
pub fn database_to_animations_json(db: &AnimationDatabase) -> serde_json::Value {
    serde_json::Value::Array(db.clips.iter().map(clip_to_json).collect())
}

fn tokenize_for_match(input: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "animations",
        "animation",
        "spaceships",
        "ships",
        "objects",
        "object",
        "rsi",
        "scorpius",
        "play",
        "ssmp",
        "component",
        "audio",
        "trigger",
        "event",
        "caf",
        // Directional tokens are too generic and cause false matches
        // (e.g. cooler_left_* selecting wing clips just because many wing
        // bones contain "left").
        "left",
        "right",
        "top",
        "bottom",
        "front",
        "rear",
        "main",
        // Common action verbs are non-discriminative across many clips.
        "open",
        "close",
        "deploy",
        "retract",
    ];

    input
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| t.len() >= 3)
        .filter(|t| !STOPWORDS.iter().any(|w| w == t))
        .collect()
}

fn clip_semantic_score(
    clip: &AnimationClip,
    event_tokens: &[String],
    skeleton_bone_name_by_hash: &HashMap<u32, String>,
) -> i32 {
    if event_tokens.is_empty() {
        return 0;
    }

    let mut score = 0i32;

    // DBA metadata names can be misaligned with block contents, so semantic
    // scoring is intentionally based on resolved channel bone names only.
    for ch in &clip.channels {
        let Some(name) = skeleton_bone_name_by_hash.get(&ch.bone_hash) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let bone_tokens = tokenize_for_match(&lower);

        for token in event_tokens {
            if bone_tokens.iter().any(|bt| bt == token) {
                score += 4;
            } else if lower.contains(token) {
                score += 2;
            }
        }
    }
    score
}

fn clip_motion_score_milli(clip: &AnimationClip) -> i64 {
    let mut score = 0.0f64;

    for ch in &clip.channels {
        if ch.rotations.len() >= 2 {
            let q0 = ch.rotations.first().map(|k| k.value).unwrap_or([0.0; 4]);
            let q1 = ch.rotations.last().map(|k| k.value).unwrap_or([0.0; 4]);
            let dot = (q0[0] * q1[0] + q0[1] * q1[1] + q0[2] * q1[2] + q0[3] * q1[3])
                .abs()
                .clamp(0.0, 1.0) as f64;
            // Quaternion angular distance in radians.
            let angle = 2.0f64 * dot.acos();
            score += angle;
        }

        if ch.positions.len() >= 2 {
            let p0 = ch.positions.first().map(|k| k.value).unwrap_or([0.0; 3]);
            let p1 = ch.positions.last().map(|k| k.value).unwrap_or([0.0; 3]);
            let dx = (p1[0] - p0[0]) as f64;
            let dy = (p1[1] - p0[1]) as f64;
            let dz = (p1[2] - p0[2]) as f64;
            score += (dx * dx + dy * dy + dz * dz).sqrt();
        }
    }

    (score * 1000.0).round() as i64
}

/// Match DBA blocks to `.chrparams` event names using a hybrid approach:
///
/// 1. **Path-based** (primary): resolve the chrparams CAF path to its full
///    engine path and match it case-insensitively against DBA metadata names.
///    This works for any DBA where metadata is correctly ordered.
///
/// 2. **Bone-subset fallback** (secondary): if the path-matched block contains
///    bones that are NOT in this skeleton, the DBA metadata for this section is
///    scrambled (as seen in `Scorpius.dba` for landing gear clips).  In that
///    case, fall back to finding the first unmatched DBA block whose entire bone
///    set is a subset of this skeleton's bones.
///
/// Unmatched DBA blocks retain their original DBA metadata names only when
/// `include_unmatched` is `true`.  Pass `false` for child skeleton sources that
/// share the root's DBA so that already-covered blocks are not duplicated.
fn caf_anchored_remap(
    db: &AnimationDatabase,
    chrparams: &crate::chrparams::ChrParams,
    skeleton_bone_hashes: &HashSet<u32>,
    skeleton_bone_name_by_hash: &HashMap<u32, String>,
    animevents_targets_by_caf: &HashMap<String, Vec<String>>,
    include_unmatched: bool,
    allow_bone_subset_fallback: bool,
) -> Vec<AnimationClip> {
    // Build a name→index map from the DBA metadata names (case-insensitive).
    let mut name_map: HashMap<String, usize> = HashMap::new();
    for (i, clip) in db.clips.iter().enumerate() {
        name_map.entry(clip.name.to_ascii_lowercase()).or_insert(i);
    }

    // When skeleton_bone_hashes is empty we skip validation (skeleton not found).
    let can_validate = !skeleton_bone_hashes.is_empty();

    let mut matched = vec![false; db.clips.len()];
    let mut named_clips: Vec<AnimationClip> = Vec::new();

    for (event_name, caf_path) in &chrparams.animations {
        let resolved_caf = chrparams.resolved_caf_path(caf_path);
        let resolved_lower = resolved_caf.to_ascii_lowercase();
        let caf_file = resolved_caf
            .rsplit_once('/')
            .map(|(_, tail)| tail)
            .unwrap_or(resolved_caf.as_str())
            .trim_end_matches(".caf");

        let mut event_tokens = tokenize_for_match(event_name);
        event_tokens.extend(tokenize_for_match(caf_file));
        if let Some(targets) = animevents_targets_by_caf.get(&resolved_lower) {
            for target in targets {
                event_tokens.extend(tokenize_for_match(target));
            }
        }

        let mut chosen_idx: Option<usize> = None;
        let mut path_idx_hint: Option<usize> = None;

        // Step 1: path-based lookup.
        if let Some(&path_idx) = name_map.get(&resolved_lower) {
            if !matched[path_idx] {
                let block_valid = !can_validate || db.clips[path_idx]
                    .channels
                    .iter()
                    .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash));
                if block_valid {
                    // Keep as a hint; we may override it if semantic scoring finds
                    // a better candidate among similarly-valid blocks.
                    path_idx_hint = Some(path_idx);
                    chosen_idx = Some(path_idx);
                } else {
                    log::debug!(
                        "[anim] path-matched block {path_idx} for '{event_name}' has bones outside skeleton — using semantic/bone-subset fallback"
                    );
                }
            }
        }

        // Step 1.5: semantic disambiguation. This is especially important for
        // root body CHRs where many clips share controller counts/start quats and
        // path-index alignment can be wrong when DBA metadata order differs from
        // block order.
        if can_validate && !skeleton_bone_name_by_hash.is_empty() {
            let best_semantic = (0..db.clips.len())
                .filter(|&i| !matched[i])
                .filter(|&i| {
                    !db.clips[i].channels.is_empty()
                        && db.clips[i]
                            .channels
                            .iter()
                            .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
                })
                .map(|i| {
                    let composite =
                        clip_semantic_score(&db.clips[i], &event_tokens, skeleton_bone_name_by_hash);
                    let motion = clip_motion_score_milli(&db.clips[i]);
                    (
                        i,
                        composite,
                        motion,
                    )
                })
                .max_by_key(|(_, score, motion)| (*score, *motion));

            if let Some((semantic_idx, semantic_score, _semantic_motion)) = best_semantic {
                let hinted_score = path_idx_hint
                    .map(|idx| {
                        clip_semantic_score(
                            &db.clips[idx],
                            &event_tokens,
                            skeleton_bone_name_by_hash,
                        )
                    })
                    .unwrap_or(i32::MIN);

                // Prefer semantic winner only when it has a strictly stronger
                // bone-name token match than the path hint, OR when no path
                // hint exists AND the semantic match has positive overlap.
                // Equal-score motion-tiebreak previously caused systematic
                // reassignment of correct path matches to nearby high-motion
                // blocks; require a strict lexical advantage instead.
                let strictly_better = semantic_score > hinted_score;
                let no_hint_with_overlap =
                    path_idx_hint.is_none() && semantic_score > 0;
                if strictly_better || no_hint_with_overlap {
                    chosen_idx = Some(semantic_idx);
                }
            }
        }

        // Step 2: bone-subset fallback if path lookup failed or was invalid.
        // Only used for child CHRs (small bone sets); the root body CHR
        // has a large superset of bones, so this fallback would misfire.
        if chosen_idx.is_none() && can_validate && allow_bone_subset_fallback {
            chosen_idx = (0..db.clips.len()).find(|&i| {
                !matched[i]
                    && !db.clips[i].channels.is_empty()
                    && db.clips[i]
                        .channels
                        .iter()
                        .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
            });
            if chosen_idx.is_some() {
                log::debug!(
                    "[anim] bone-subset fallback: assigned block {:?} to '{event_name}'",
                    chosen_idx
                );
            }
        }

        if let Some(idx) = chosen_idx {
            matched[idx] = true;
            named_clips.push(AnimationClip {
                name: event_name.clone(),
                fps: db.clips[idx].fps,
                channels: db.clips[idx].channels.clone(),
            });
        } else {
            log::debug!(
                "[anim] no DBA block found for event '{event_name}' ({resolved_caf})"
            );
        }
    }

    // Append unmatched DBA blocks with their original metadata names, but only
    // when the caller wants them (root skeleton context).
    if include_unmatched {
        for (i, clip) in db.clips.iter().enumerate() {
            if !matched[i] {
                named_clips.push(clip.clone());
            }
        }
    }

    // Step 3: clip-direction correction for named animations.
    //
    // Empirical finding (Phase 24B / 23A): paired animation clips in the DBA
    // are time-reversed relative to their event-name semantic. For example,
    // `wings_deploy` is observed to start at ~63° rotation and end at ~5°
    // (near bind), which is the *retract* direction. The companion
    // `wings_retract` is the inverse. The chrparams labelling and DBA
    // direction disagree.
    //
    // We reverse keyframe time within a clip when the name keyword and the
    // observed orientation disagree:
    //   - `deploy`/`open`/`extend`  → expected end_frame_far_from_bind
    //   - `retract`/`close`/`compress` → expected start_frame_far_from_bind
    //
    // Only applied when there is a clear bind-distance asymmetry between
    // first and last frames (>2x ratio AND at least one frame >5° rotation
    // or >0.05m position).
    for clip in named_clips.iter_mut() {
        correct_clip_direction(clip);
    }

    named_clips
}

/// Diagnostic record for a single chrparams event, describing how the
/// matching pipeline (path → semantic → bone-subset) selected a DBA block.
///
/// Returned by [`caf_anchored_remap_decisions`]. Used by the `dba match`
/// CLI subcommand to debug clip-mismatches such as the wings_deploy
/// X-shape issue.
#[derive(Debug, Clone)]
pub struct ClipMatchDecision {
    pub event_name: String,
    pub caf_path: String,
    /// Final block index chosen, or `None` if no block matched.
    pub chosen_block: Option<usize>,
    /// Which step picked the block: "path", "semantic-override",
    /// "semantic-no-hint", "bone-subset", or "unmatched".
    pub method: &'static str,
    /// The block matched purely by path lookup (Step 1), if any.
    pub path_block: Option<usize>,
    /// The block winning the semantic-overlap scoring (Step 1.5), if any.
    pub semantic_block: Option<usize>,
    /// Semantic score of the path-matched block (or i32::MIN if none).
    pub path_score: i32,
    /// Semantic score of the semantic-best block (or 0 if none).
    pub semantic_score: i32,
}

/// Run the per-event matching loop from [`caf_anchored_remap`] and return
/// per-event decision details without building the named clips. This is a
/// diagnostic helper used by the CLI `dba match` subcommand.
pub fn caf_anchored_remap_decisions(
    db: &AnimationDatabase,
    chrparams: &crate::chrparams::ChrParams,
    skeleton_bone_hashes: &HashSet<u32>,
    skeleton_bone_name_by_hash: &HashMap<u32, String>,
    animevents_targets_by_caf: &HashMap<String, Vec<String>>,
    allow_bone_subset_fallback: bool,
) -> Vec<ClipMatchDecision> {
    let mut name_map: HashMap<String, usize> = HashMap::new();
    for (i, clip) in db.clips.iter().enumerate() {
        name_map.entry(clip.name.to_ascii_lowercase()).or_insert(i);
    }

    let can_validate = !skeleton_bone_hashes.is_empty();
    let mut matched = vec![false; db.clips.len()];
    let mut decisions: Vec<ClipMatchDecision> = Vec::new();

    for (event_name, caf_path) in &chrparams.animations {
        let resolved_caf = chrparams.resolved_caf_path(caf_path);
        let resolved_lower = resolved_caf.to_ascii_lowercase();
        let caf_file = resolved_caf
            .rsplit_once('/')
            .map(|(_, tail)| tail)
            .unwrap_or(resolved_caf.as_str())
            .trim_end_matches(".caf");

        let mut event_tokens = tokenize_for_match(event_name);
        event_tokens.extend(tokenize_for_match(caf_file));
        if let Some(targets) = animevents_targets_by_caf.get(&resolved_lower) {
            for target in targets {
                event_tokens.extend(tokenize_for_match(target));
            }
        }

        let mut chosen_idx: Option<usize> = None;
        let mut method: &'static str = "unmatched";
        let mut path_block: Option<usize> = None;
        let mut semantic_block: Option<usize> = None;
        let mut path_score: i32 = i32::MIN;
        let mut semantic_score_val: i32 = 0;

        if let Some(&path_idx) = name_map.get(&resolved_lower) {
            if !matched[path_idx] {
                let block_valid = !can_validate
                    || db.clips[path_idx]
                        .channels
                        .iter()
                        .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash));
                if block_valid {
                    path_block = Some(path_idx);
                    chosen_idx = Some(path_idx);
                    method = "path";
                    path_score = clip_semantic_score(
                        &db.clips[path_idx],
                        &event_tokens,
                        skeleton_bone_name_by_hash,
                    );
                }
            }
        }

        if can_validate && !skeleton_bone_name_by_hash.is_empty() {
            let best_semantic = (0..db.clips.len())
                .filter(|&i| !matched[i])
                .filter(|&i| {
                    !db.clips[i].channels.is_empty()
                        && db.clips[i]
                            .channels
                            .iter()
                            .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
                })
                .map(|i| {
                    let composite = clip_semantic_score(
                        &db.clips[i],
                        &event_tokens,
                        skeleton_bone_name_by_hash,
                    );
                    let motion = clip_motion_score_milli(&db.clips[i]);
                    (i, composite, motion)
                })
                .max_by_key(|(_, score, motion)| (*score, *motion));

            if let Some((semantic_idx, semantic_score, _)) = best_semantic {
                semantic_block = Some(semantic_idx);
                semantic_score_val = semantic_score;
                let strictly_better = semantic_score > path_score;
                let no_hint_with_overlap = path_block.is_none() && semantic_score > 0;
                if strictly_better {
                    chosen_idx = Some(semantic_idx);
                    method = "semantic-override";
                } else if no_hint_with_overlap {
                    chosen_idx = Some(semantic_idx);
                    method = "semantic-no-hint";
                }
            }
        }

        if chosen_idx.is_none() && can_validate && allow_bone_subset_fallback {
            chosen_idx = (0..db.clips.len()).find(|&i| {
                !matched[i]
                    && !db.clips[i].channels.is_empty()
                    && db.clips[i]
                        .channels
                        .iter()
                        .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
            });
            if chosen_idx.is_some() {
                method = "bone-subset";
            }
        }

        if let Some(idx) = chosen_idx {
            matched[idx] = true;
        }

        decisions.push(ClipMatchDecision {
            event_name: event_name.clone(),
            caf_path: resolved_caf,
            chosen_block: chosen_idx,
            method,
            path_block,
            semantic_block,
            path_score,
            semantic_score: semantic_score_val,
        });
    }

    decisions
}
/// Returns the magnitude of "distance from bind" for a single keyframe entry.
/// Bind = identity rotation (w=1, xyz=0) and zero position. Returns the
/// rotation angle in radians plus position magnitude in meters.
fn keyframe_bind_distance(
    rot: Option<&[f32; 4]>,
    pos: Option<&[f32; 3]>,
) -> f64 {
    let mut d = 0.0f64;
    if let Some(q) = rot {
        // Quaternion stored as CryEngine XYZW in the channel keyframe;
        // the W component is at index 3.
        let w = q[3].clamp(-1.0, 1.0) as f64;
        d += 2.0 * w.abs().clamp(0.0, 1.0).acos();
    }
    if let Some(p) = pos {
        let dx = p[0] as f64;
        let dy = p[1] as f64;
        let dz = p[2] as f64;
        d += (dx * dx + dy * dy + dz * dz).sqrt();
    }
    d
}

fn correct_clip_direction(clip: &mut AnimationClip) {
    // Determine expected direction from clip name.
    let name_lower = clip.name.to_ascii_lowercase();
    // "extend" matches before "retract" via substring; we check explicitly.
    let expects_end_far = name_lower.contains("deploy")
        || name_lower.contains("open")
        || name_lower.contains("extend");
    let expects_start_far = name_lower.contains("retract")
        || name_lower.contains("close")
        || name_lower.contains("compress");

    if !(expects_end_far || expects_start_far) {
        return; // No directional keyword.
    }
    // Per-channel SIGNED contribution: (last_bind_dist - first_bind_dist).
    // Sum across channels gives the net direction. Position channels often
    // store static offsets, so rotation is preferred; we fall back to
    // position deltas when rotation has no signal.
    let mut signed_total = 0.0f64;
    let mut total_motion = 0.0f64;
    for ch in &clip.channels {
        let r0 = ch.rotations.first().map(|k| &k.value);
        let r_n = ch.rotations.last().map(|k| &k.value);
        if r0.is_none() || r_n.is_none() {
            continue;
        }
        let d0 = keyframe_bind_distance(r0, None);
        let d_n = keyframe_bind_distance(r_n, None);
        signed_total += d_n - d0;
        total_motion += (d_n - d0).abs();
    }

    if total_motion < 0.087 {
        for ch in &clip.channels {
            if ch.positions.len() < 2 {
                continue;
            }
            let p0 = ch.positions.first().unwrap().value;
            let p_n = ch.positions.last().unwrap().value;
            let m0 = (p0[0].powi(2) + p0[1].powi(2) + p0[2].powi(2)).sqrt() as f64;
            let m_n = (p_n[0].powi(2) + p_n[1].powi(2) + p_n[2].powi(2)).sqrt() as f64;
            let delta = ((p_n[0] - p0[0]).powi(2)
                + (p_n[1] - p0[1]).powi(2)
                + (p_n[2] - p0[2]).powi(2))
            .sqrt() as f64;
            if delta > 0.01 {
                signed_total += m_n - m0;
                total_motion += delta;
            }
        }
    }

    if total_motion < 0.087 {
        return; // Not enough motion to determine direction.
    }

    let directional_strength = signed_total.abs() / total_motion;
    log::info!(
        "[anim] direction-check '{}' signed={:.3} motion={:.3} strength={:.2}",
        clip.name, signed_total, total_motion, directional_strength
    );
    if directional_strength < 0.30 {
        return;
    }

    let actual_end_far = signed_total > 0.0;
    let needs_reverse = if expects_end_far {
        !actual_end_far
    } else {
        actual_end_far
    };

    if !needs_reverse {
        return;
    }

    // Reverse keyframe time. We mirror times around the clip's max_time so
    // the duration is preserved and frame 0 stays frame 0.
    let mut max_time = 0.0f32;
    for ch in &clip.channels {
        for k in &ch.rotations {
            max_time = max_time.max(k.time);
        }
        for k in &ch.positions {
            max_time = max_time.max(k.time);
        }
    }

    log::info!(
        "[anim] reversing direction of clip '{}' signed={:.3} expects_end_far={}",
        clip.name,
        signed_total,
        expects_end_far
    );

    for ch in clip.channels.iter_mut() {
        for k in ch.rotations.iter_mut() {
            k.time = max_time - k.time;
        }
        ch.rotations.reverse();
        for k in ch.positions.iter_mut() {
            k.time = max_time - k.time;
        }
        ch.positions.reverse();
    }
}

pub fn extract_animations_for_skeleton_json(
    p4k: &starbreaker_p4k::MappedP4k,
    skeleton_path: &str,
    include_unmatched_dba_blocks: bool,
    allow_bone_subset_fallback: bool,
) -> Result<Option<serde_json::Value>, Error> {
    let mut candidate_paths = Vec::new();
    if let Some(path) = swap_extension(skeleton_path, ".chrparams") {
        candidate_paths.push(path);
    }
    // SC assets often ship `*_SKIN.skin` + `*_CHR.chr/.chrparams` pairs.
    let skin_to_chr = skeleton_path
        .replace("_SKIN.skin", "_CHR.chrparams")
        .replace("_skin.skin", "_chr.chrparams")
        .replace("_skin.SKIN", "_chr.chrparams");
    if !candidate_paths.iter().any(|path| path.eq_ignore_ascii_case(&skin_to_chr)) {
        candidate_paths.push(skin_to_chr);
    }

    // Try candidate chrparams paths; skip if none found.
    let mut chrparams_data = None;
    for candidate in &candidate_paths {
        let candidate_p4k = crate::pipeline::datacore_path_to_p4k(candidate);
        if let Some(data) = p4k
            .entry_case_insensitive(&candidate_p4k)
            .and_then(|e| p4k.read(e).ok())
        {
            chrparams_data = Some(data.to_vec());
            break;
        }
    }
    let Some(chrparams_data) = chrparams_data else {
        return Ok(None); // Skeleton has no discoverable chrparams
    };

    // Parse chrparams to get tracks database path
    let chrparams = match crate::chrparams::ChrParams::from_bytes(&chrparams_data) {
        Ok(value) => value,
        Err(error) => {
            let text = error.to_string();
            // Some non-skeleton assets are probed via heuristic path swaps and
            // resolve to non-CryXml payloads. Treat those as "no animations" to
            // avoid noisy warnings during normal export.
            if text.contains("InvalidMagic") {
                return Ok(None);
            }
            return Err(Error::Other(format!("Failed to parse chrparams: {error}")));
        }
    };

    let animevents_targets_by_caf: HashMap<String, Vec<String>> = chrparams
        .anim_event_database
        .as_deref()
        .and_then(|path| {
            let resolved = chrparams.resolved_caf_path(path);
            let resolved_p4k = crate::pipeline::datacore_path_to_p4k(&resolved);
            p4k.entry_case_insensitive(&resolved_p4k)
                .and_then(|e| p4k.read(e).ok())
                .and_then(|bytes| crate::chrparams::parse_animevents_targets(&bytes).ok())
        })
        .unwrap_or_default();

    // Prefer tracks database (.dba) when present.
    if let Some(tracks_db_path) = chrparams.tracks_database.clone() {
        let resolved_path = chrparams.resolved_caf_path(&tracks_db_path);
        let resolved_p4k = crate::pipeline::datacore_path_to_p4k(&resolved_path);
        let dba_data = p4k
            .entry_case_insensitive(&resolved_p4k)
            .and_then(|e| p4k.read(e).ok())
            .ok_or_else(|| Error::Other(format!("Cannot load tracks database: {resolved_path}")))?
            .to_vec();
        let db = parse_dba(&dba_data)?;
        // Load the skeleton file and compute its bone hash set.  This is used
        // to identify which DBA blocks belong to this CHR (bone-subset scan).
        let skeleton_p4k_path = crate::pipeline::datacore_path_to_p4k(skeleton_path);
        let (skeleton_bone_hashes, skeleton_bone_name_by_hash): (
            HashSet<u32>,
            HashMap<u32, String>,
        ) = p4k
            .entry_case_insensitive(&skeleton_p4k_path)
            .and_then(|e| p4k.read(e).ok())
            .and_then(|data| crate::skeleton::parse_skeleton(&data))
            .map(|bones| {
                let hashes = bones
                    .iter()
                    .map(|b| bone_name_hash(&b.name))
                    .collect::<HashSet<_>>();
                let name_map = bones
                    .iter()
                    .map(|b| (bone_name_hash(&b.name), b.name.to_ascii_lowercase()))
                    .collect::<HashMap<_, _>>();
                (hashes, name_map)
            })
            .unwrap_or_default();
        log::debug!(
            "[anim] skeleton '{}' has {} bone hashes",
            skeleton_path,
            skeleton_bone_hashes.len()
        );
        let clips = caf_anchored_remap(
            &db,
            &chrparams,
            &skeleton_bone_hashes,
            &skeleton_bone_name_by_hash,
            &animevents_targets_by_caf,
            include_unmatched_dba_blocks,
            allow_bone_subset_fallback,
        );
        return Ok(Some(database_to_animations_json(&AnimationDatabase { clips })));
    }

    // Fallback for chrparams that reference per-clip CAF files directly.
    if chrparams.animations.is_empty() {
        return Ok(None);
    }
    let mut clips = Vec::new();
    for (event_name, caf_path) in &chrparams.animations {
        let resolved_path = chrparams.resolved_caf_path(caf_path);
        let resolved_p4k = crate::pipeline::datacore_path_to_p4k(&resolved_path);
        let Some(caf_data) = p4k
            .entry_case_insensitive(&resolved_p4k)
            .and_then(|e| p4k.read(e).ok())
        else {
            continue;
        };
        if let Ok(mut db) = parse_caf(&caf_data) {
            for mut clip in db.clips.drain(..) {
                clip.name = event_name.clone();
                clips.push(clip);
            }
        }
    }
    if clips.is_empty() {
        return Ok(None);
    }
    Ok(Some(database_to_animations_json(&AnimationDatabase { clips })))
}

/// Helper: swap file extension. E.g., "file.chr" → "file.chrparams"
fn swap_extension(path: &str, new_ext: &str) -> Option<String> {
    if let Some(dot_pos) = path.rfind('.') {
        let base = &path[..dot_pos];
        Some(format!("{}{}", base, new_ext))
    } else {
        None
    }
}



#[cfg(test)]
mod bake_tests {
    use super::*;

    #[test]
    fn bone_hash_matches_known_values() {
        // Verified externally via Python `zlib.crc32` (case preserved).
        assert_eq!(bone_name_hash("BONE_Back_Right_Foot_Main"), 0xC1571A1A);
    }

    #[test]
    fn quat_mul_identity() {
        let id = [1.0, 0.0, 0.0, 0.0];
        let q = [0.7071068, 0.7071068, 0.0, 0.0];
        let out = quat_mul_wxyz(id, q);
        for i in 0..4 {
            assert!((out[i] - q[i]).abs() < 1e-6, "{:?}", out);
        }
    }

    #[test]
    fn quat_rotate_basis() {
        // 90° about Z (wxyz): w=cos45, z=sin45
        let q = [0.7071068, 0.0, 0.0, 0.7071068];
        let v = [1.0, 0.0, 0.0];
        let r = quat_rotate_vec_wxyz(q, v);
        assert!((r[0] - 0.0).abs() < 1e-5, "{:?}", r);
        assert!((r[1] - 1.0).abs() < 1e-5, "{:?}", r);
        assert!(r[2].abs() < 1e-5, "{:?}", r);
    }
}


