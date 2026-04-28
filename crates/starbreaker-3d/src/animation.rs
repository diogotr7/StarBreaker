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

/// DataCore-declared Mannequin animation-controller sources for an entity.
#[derive(Debug, Clone)]
pub struct AnimationControllerSource {
    pub animation_database: String,
    pub animation_controller: String,
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
    /// Raw 16-bit `rot_format_flags` from the CAF/DBA controller entry.
    /// Currently understood as the rotation-keyframe encoding format
    /// (e.g. quaternion compression). Captured verbatim so debug
    /// tooling can hunt for additive/override bits (Phase 37).
    pub rot_format_flags: u16,
    /// Raw 16-bit `pos_format_flags` from the CAF/DBA controller
    /// entry. Captured verbatim alongside `rot_format_flags`.
    pub pos_format_flags: u16,
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
    /// Start-frame reference position (XY only; only 8 bytes fit in the
    /// 48-byte entry). Empirically `(0, 0)` for most clips on Scorpius;
    /// non-zero for landing-gear and similar clips that translate the
    /// whole bone group. See Phase 29 in todo.md.
    start_position_xy: [f32; 2],
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
            rot_format_flags: ctrl.rot_format_flags,
            pos_format_flags: ctrl.pos_format_flags,
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
        //   +0x28 (8)  start_position trailing — only XY of a 3-component
        //              position fits (Z elided / always implicit 0).
        //              Empirically (0, 0) for most clips; non-zero for
        //              landing-gear and similar group-translating clips
        //              (Phase 29 empirical confirmation, 2026-04-28).
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
        let start_position_xy = [
            f32::from_le_bytes(data[o + 40..o + 44].try_into().unwrap_or([0; 4])),
            f32::from_le_bytes(data[o + 44..o + 48].try_into().unwrap_or([0; 4])),
        ];
        entries.push(DbaMetaEntry {
            fps,
            num_controllers,
            end_frame,
            start_rotation,
            start_position_xy,
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
    let n_active = active.iter().filter(|&&a| a).count();
    let total_bytes = count * n_active * 2;
    let data_start = offset + 24;
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
/// Must match the position axis swap used by `clip_to_json` and the static
/// import's `_scene_position_to_blender`: `(cx, cy, cz) → (cx, -cz, cy)`.
/// Applying the same basis change to a quaternion's vector component gives
/// `(qx, qy, qz, qw) → (qw, qx, -qz, qy)` in Blender WXYZ form. Keeping
/// rotation and translation in the same basis is required for animation
/// deltas to compose correctly with the bone's bind pose.
pub fn cry_xyzw_to_blender_wxyz(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    [w, x, -z, y]
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
            // Same basis change as `clip_to_json` and static import:
            // CryEngine (x, y, z) → Blender (x, -z, y).
            [x, -z, y]
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
        let mut rotation_time_array = vec![];
        for keyframe in &channel.rotations {
            let q = cry_xyzw_to_blender_wxyz(keyframe.value);
            rotation_array.push(serde_json::json!([q[0], q[1], q[2], q[3]]));
            rotation_time_array.push(serde_json::json!(keyframe.time));
        }

        let mut position_array = vec![];
        let mut position_time_array = vec![];
        for keyframe in &channel.positions {
            let p = keyframe.value;
            // CryEngine Y-up → Blender Z-up axis swap: (x, y, z) → (x, -z, y).
            // Must match the static-import convention used by the addon's
            // `_scene_position_to_blender` (runtime/importer/utils.py); both
            // sides need to put CryEngine X into Blender X so that animation
            // deltas land in the same frame as the bone's bind position.
            position_array.push(serde_json::json!([p[0], -p[2], p[1]]));
            position_time_array.push(serde_json::json!(keyframe.time));
        }

        let bone_key = format!("0x{:X}", channel.bone_hash);
        bones[bone_key] = serde_json::json!({
            "has_rotation": has_rotation,
            "has_position": has_position,
            "rotation": rotation_array,
            "rotation_time": rotation_time_array,
            "position": position_array,
            "position_time": position_time_array,
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

/// Per-bone animation blend mode, derived from the geometric
/// relationship between the bone's CHR-bind position and the AABB
/// of all CAF clip samples that touch the bone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoneBlendMode {
    /// Bind sits inside (or coincident with) the CAF sample AABB —
    /// the clip is interpreted as additive on top of bind. This is
    /// the default and matches the addon's anchor-relative
    /// composition path (`bind ⋅ (anchor⁻¹ ⋅ sample)`).
    Additive,
    /// Bind sits strictly outside the CAF sample AABB on at least one
    /// axis — the clip is interpreted as an override. The addon
    /// should use the sampled pose verbatim (`result = sample`)
    /// instead of composing it on top of bind.
    Override,
}

impl BoneBlendMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BoneBlendMode::Additive => "additive",
            BoneBlendMode::Override => "override",
        }
    }
}

/// Classify each bone's animation blend mode by testing whether the
/// CHR-bind local position sits inside the AABB of the bone's CAF
/// position samples across **all** clips. A bone with no position
/// samples or no bind entry is omitted from the result (the addon
/// defaults to additive).
///
/// Containment is strict on each axis: bind is "outside" if any
/// component is < min or > max with no epsilon. This is data-grounded
/// — the only inputs are the CHR-bind position and the CAF sample
/// stream — and uses no heuristics, name lookups, or absolute-unit
/// thresholds.
///
/// Both inputs are interpreted in raw CryEngine local-space (the
/// same convention as `BoneChannel.positions` and
/// `crate::skeleton::Bone::local_position`); no axis swap is
/// applied.
pub fn classify_bone_blend_modes(
    clips: &[AnimationClip],
    binds: &std::collections::HashMap<u32, [f32; 3]>,
) -> std::collections::HashMap<u32, BoneBlendMode> {
    // Build per-bone AABB from all CAF position samples.
    let mut bbox: std::collections::HashMap<u32, ([f32; 3], [f32; 3])> =
        std::collections::HashMap::new();
    for clip in clips {
        for ch in &clip.channels {
            if ch.positions.is_empty() {
                continue;
            }
            let entry = bbox.entry(ch.bone_hash).or_insert_with(|| {
                let p = ch.positions[0].value;
                (p, p)
            });
            for kf in &ch.positions {
                let p = kf.value;
                for axis in 0..3 {
                    if p[axis] < entry.0[axis] {
                        entry.0[axis] = p[axis];
                    }
                    if p[axis] > entry.1[axis] {
                        entry.1[axis] = p[axis];
                    }
                }
            }
        }
    }
    let mut out = std::collections::HashMap::new();
    for (hash, (min, max)) in &bbox {
        let Some(bind) = binds.get(hash) else {
            continue;
        };
        let outside = (0..3).any(|axis| bind[axis] < min[axis] || bind[axis] > max[axis]);
        out.insert(
            *hash,
            if outside { BoneBlendMode::Override } else { BoneBlendMode::Additive },
        );
    }
    out
}

/// Inject a `blend_mode` field into every clip's per-bone entry of
/// the JSON produced by [`database_to_animations_json`]. Bones
/// without an entry in `modes` are left untouched (the addon
/// defaults to additive).
pub fn annotate_animations_json_with_blend_modes(
    clips_json: &mut serde_json::Value,
    modes: &std::collections::HashMap<u32, BoneBlendMode>,
) {
    let Some(arr) = clips_json.as_array_mut() else {
        return;
    };
    for clip in arr.iter_mut() {
        let Some(bones) = clip.get_mut("bones").and_then(|v| v.as_object_mut()) else {
            continue;
        };
        for (key, value) in bones.iter_mut() {
            // bone_key is "0xHEX" — parse back to u32.
            let Some(stripped) = key.strip_prefix("0x").or(Some(key)) else { continue };
            let Ok(hash) = u32::from_str_radix(stripped, 16) else { continue };
            let Some(mode) = modes.get(&hash) else { continue };
            if let Some(obj) = value.as_object_mut() {
                obj.insert(
                    "blend_mode".to_string(),
                    serde_json::Value::String(mode.as_str().to_string()),
                );
            }
        }
    }
}

/// Structured dump of an animation database for diagnostic / debug
/// tooling. Returns a JSON value with one entry per clip listing
/// channel counts, frame counts, per-channel bone hashes (resolved to
/// names when `hash_to_name` provides them), and either first/last
/// keyframe samples or the full keyframe stream depending on
/// `all_keyframes`.
///
/// Used by the StarBreaker MCP `dba_dump` tool. Replaces the previous
/// `starbreaker dba dump` CLI subcommand (Phase 36).
pub fn dump_database_to_json(
    db: &AnimationDatabase,
    hash_to_name: &std::collections::HashMap<u32, String>,
    filter: Option<&str>,
    bone_filter: Option<&str>,
    all_keyframes: bool,
) -> serde_json::Value {
    let filter_lc = filter.map(|f| f.to_ascii_lowercase());
    let bone_filter_lc = bone_filter.map(|f| f.to_ascii_lowercase());
    let mut clips_out: Vec<serde_json::Value> = Vec::new();
    for (idx, clip) in db.clips.iter().enumerate() {
        if let Some(needle) = filter_lc.as_ref() {
            if !clip.name.to_ascii_lowercase().contains(needle) {
                continue;
            }
        }
        let frame_count = clip
            .channels
            .iter()
            .map(|ch| ch.rotations.len().max(ch.positions.len()))
            .max()
            .unwrap_or(0);
        let mut channels_out: Vec<serde_json::Value> = Vec::with_capacity(clip.channels.len());
        for ch in &clip.channels {
            let bone_name = hash_to_name.get(&ch.bone_hash).cloned();
            // Bone-name filter: when set, skip channels whose resolved name
            // doesn't contain the substring (case-insensitive). Channels with
            // unresolved hashes are skipped when a bone filter is active so the
            // output is unambiguous.
            if let Some(needle) = bone_filter_lc.as_ref() {
                let matches = bone_name
                    .as_ref()
                    .map(|n| n.to_ascii_lowercase().contains(needle))
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            let mut channel_value = serde_json::json!({
                "bone_hash": format!("0x{:08X}", ch.bone_hash),
                "bone_name": bone_name,
                "rotation_count": ch.rotations.len(),
                "position_count": ch.positions.len(),
                "rot_format_flags": format!("0x{:04X}", ch.rot_format_flags),
                "pos_format_flags": format!("0x{:04X}", ch.pos_format_flags),
            });
            if all_keyframes {
                channel_value["rotations"] = serde_json::Value::Array(
                    ch.rotations.iter().map(|kf| serde_json::json!({
                        "time": kf.time,
                        "value": kf.value,
                    })).collect(),
                );
                channel_value["positions"] = serde_json::Value::Array(
                    ch.positions.iter().map(|kf| serde_json::json!({
                        "time": kf.time,
                        "value": kf.value,
                    })).collect(),
                );
            } else {
                if let (Some(first), Some(last)) = (ch.rotations.first(), ch.rotations.last()) {
                    channel_value["rotation_first"] =
                        serde_json::json!({"time": first.time, "value": first.value});
                    channel_value["rotation_last"] =
                        serde_json::json!({"time": last.time, "value": last.value});
                }
                if let (Some(first), Some(last)) = (ch.positions.first(), ch.positions.last()) {
                    channel_value["position_first"] =
                        serde_json::json!({"time": first.time, "value": first.value});
                    channel_value["position_last"] =
                        serde_json::json!({"time": last.time, "value": last.value});
                }
            }
            channels_out.push(channel_value);
        }
        clips_out.push(serde_json::json!({
            "index": idx,
            "name": clip.name,
            "fps": clip.fps,
            "channel_count": clip.channels.len(),
            "frame_count": frame_count,
            "channels": channels_out,
        }));
    }
    serde_json::json!({
        "clip_count": db.clips.len(),
        "skeleton_bones_resolved": hash_to_name.len(),
        "clips": clips_out,
    })
}

/// Sanitize a clip name into a safe filename component.
///
/// Replaces characters outside `[A-Za-z0-9_.-]` with `_`. Used by the
/// decomposed exporter to derive per-clip animation sidecar filenames
/// under `Packages/<entity>/animations/<clip>.json`.
pub fn sanitize_clip_filename(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "clip".to_string();
    }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "clip".to_string()
    } else {
        out
    }
}

/// Split a fully-serialized animation clip into a lightweight index
/// record (preserves `name`, `fps`, `frame_count`, `fragments`, etc.,
/// and adds a `sidecar` reference) and a heavy sidecar body (the full
/// clip including the `bones` keyframe arrays).
///
/// The exporter writes the sidecar body to a separate JSON file under
/// `Packages/<entity>/animations/<clip>.json` so that the inline
/// `scene.json` only carries an index. The Blender importer then loads
/// the sidecar lazily when a clip is actually applied.
///
/// `sidecar_relative_path` is stored on the index record verbatim.
pub fn split_clip_for_sidecar(
    clip: &serde_json::Value,
    sidecar_relative_path: &str,
) -> (serde_json::Value, serde_json::Value) {
    let mut index = clip.clone();
    if let Some(map) = index.as_object_mut() {
        map.remove("bones");
        map.insert(
            "sidecar".to_string(),
            serde_json::Value::String(sidecar_relative_path.to_string()),
        );
    }
    (index, clip.clone())
}

/// Attach Mannequin ADB fragment metadata to already-serialized animation clips.
pub fn annotate_animation_fragments_json(
    p4k: &starbreaker_p4k::MappedP4k,
    clips: &mut [serde_json::Value],
    source: &AnimationControllerSource,
) -> Result<(), Error> {
    let scopes = read_controller_fragment_scopes(p4k, &source.animation_controller);
    let fragments_by_clip = read_mannequin_fragments_by_clip(p4k, &source.animation_database, &scopes)?;

    for clip in clips.iter_mut() {
        let Some(name) = clip.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        let keys = clip_name_lookup_keys(name);
        let mut fragments: Vec<serde_json::Value> = Vec::new();
        for key in keys {
            if let Some(values) = fragments_by_clip.get(&key) {
                for fragment in values {
                    if !fragments.iter().any(|existing| existing == fragment) {
                        fragments.push(fragment.clone());
                    }
                }
            }
        }
        if !fragments.is_empty() {
            clip["fragments"] = serde_json::Value::Array(fragments);
        }
    }

    Ok(())
}

fn read_mannequin_fragments_by_clip(
    p4k: &starbreaker_p4k::MappedP4k,
    animation_database: &str,
    scopes: &HashMap<String, Vec<String>>,
) -> Result<HashMap<String, Vec<serde_json::Value>>, Error> {
    let path = mannequin_adb_p4k_path(animation_database);
    let data = p4k
        .entry_case_insensitive(&path)
        .and_then(|entry| p4k.read(entry).ok())
        .ok_or_else(|| Error::Other(format!("Cannot load Mannequin ADB: {path}")))?
        .to_vec();
    let xml = starbreaker_cryxml::from_bytes(&data)
        .map_err(|error| Error::Other(format!("Mannequin ADB CryXml parse: {error:?}")))?;

    let mut fragments = Vec::new();
    collect_mannequin_fragments(&xml, xml.root(), None, false, scopes, &mut fragments);

    let mut by_clip: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for fragment in fragments {
        let animation_names = fragment
            .get("animations")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|animation| animation.get("name").and_then(|value| value.as_str()))
            .flat_map(clip_name_lookup_keys)
            .collect::<Vec<_>>();
        for key in animation_names {
            by_clip.entry(key).or_default().push(fragment.clone());
        }
    }

    Ok(by_clip)
}

fn read_controller_fragment_scopes(
    p4k: &starbreaker_p4k::MappedP4k,
    animation_controller: &str,
) -> HashMap<String, Vec<String>> {
    let path = mannequin_adb_p4k_path(animation_controller);
    let Some(data) = p4k
        .entry_case_insensitive(&path)
        .and_then(|entry| p4k.read(entry).ok())
        .map(|bytes| bytes.to_vec())
    else {
        return HashMap::new();
    };
    let Ok(xml) = starbreaker_cryxml::from_bytes(&data) else {
        return HashMap::new();
    };

    let mut scopes = HashMap::new();
    collect_controller_fragment_scopes(&xml, xml.root(), &mut scopes);
    scopes
}

fn collect_controller_fragment_scopes(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    scopes: &mut HashMap<String, Vec<String>>,
) {
    let tag = xml.node_tag(node);
    if tag != "ControllerDef" && tag != "Tags" && tag != "Fragments" && tag != "FragmentDefs" {
        let attrs = xml.node_attributes(node).collect::<HashMap<_, _>>();
        if let Some(raw_scopes) = attrs.get("scopes") {
            scopes.insert(tag.to_string(), split_tag_list(raw_scopes));
        }
    }
    for child in xml.node_children(node) {
        collect_controller_fragment_scopes(xml, child, scopes);
    }
}

fn collect_mannequin_fragments(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    current_fragment_group: Option<String>,
    in_fragment_list: bool,
    scopes: &HashMap<String, Vec<String>>,
    out: &mut Vec<serde_json::Value>,
) {
    let tag = xml.node_tag(node);
    let now_in_fragment_list = in_fragment_list || tag == "FragmentList";
    let group = if now_in_fragment_list && tag != "FragmentList" && tag != "Fragment" {
        Some(tag.to_string())
    } else {
        current_fragment_group
    };

    if tag == "Fragment" {
        if let Some(fragment) = mannequin_fragment_json(xml, node, group.as_deref(), scopes) {
            out.push(fragment);
        }
    }

    for child in xml.node_children(node) {
        collect_mannequin_fragments(xml, child, group.clone(), now_in_fragment_list, scopes, out);
    }
}

fn mannequin_fragment_json(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    group: Option<&str>,
    scopes: &HashMap<String, Vec<String>>,
) -> Option<serde_json::Value> {
    let group = group.unwrap_or("");
    let attrs = xml.node_attributes(node).collect::<HashMap<_, _>>();
    let animations = collect_fragment_animations(xml, node);
    if animations.is_empty() {
        return None;
    }
    let procedurals = collect_fragment_procedurals(xml, node);

    let mut fragment = serde_json::json!({
        "fragment": group,
        "guid": attrs.get("GUID").copied().unwrap_or_default(),
        "tags": split_tag_list(attrs.get("Tags").copied().unwrap_or_default()),
        "frag_tags": split_tag_list(attrs.get("FragTags").copied().unwrap_or_default()),
        "blend_out_duration": parse_f32_attr(attrs.get("BlendOutDuration").copied()),
        "option_weight": parse_f32_attr(attrs.get("OptionWeight").copied()),
        "animations": animations,
    });
    if let Some(scope_list) = scopes.get(group) {
        fragment["scopes"] = serde_json::json!(scope_list);
    }
    if !procedurals.is_empty() {
        fragment["procedurals"] = serde_json::Value::Array(procedurals);
    }
    Some(fragment)
}

fn collect_fragment_animations(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> Vec<serde_json::Value> {
    let mut values = Vec::new();
    for child in xml.node_children(node) {
        if xml.node_tag(child) == "AnimLayer" {
            let mut blend = serde_json::json!({});
            for layer_child in xml.node_children(child) {
                let child_tag = xml.node_tag(layer_child);
                let attrs = xml.node_attributes(layer_child).collect::<HashMap<_, _>>();
                if child_tag == "Blend" {
                    blend = serde_json::json!({
                        "exit_time": parse_f32_attr(attrs.get("ExitTime").copied()),
                        "start_time": parse_f32_attr(attrs.get("StartTime").copied()),
                        "duration": parse_f32_attr(attrs.get("Duration").copied()),
                    });
                } else if child_tag == "Animation" {
                    let mut animation = serde_json::json!({
                        "name": attrs.get("name").copied().unwrap_or_default(),
                        "blend": blend,
                    });
                    if let Some(flags) = attrs.get("flags") {
                        animation["flags"] = serde_json::json!(flags);
                    }
                    if let Some(speed) = parse_f32_attr(attrs.get("speed").copied()) {
                        animation["speed"] = serde_json::json!(speed);
                    }
                    values.push(animation);
                }
            }
        }
        values.extend(collect_fragment_animations(xml, child));
    }
    values
}

fn collect_fragment_procedurals(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> Vec<serde_json::Value> {
    let mut values = Vec::new();
    for child in xml.node_children(node) {
        if xml.node_tag(child) == "Procedural" {
            let attrs = xml.node_attributes(child).collect::<HashMap<_, _>>();
            let mut params = serde_json::json!({});
            for proc_child in xml.node_children(child) {
                if xml.node_tag(proc_child) != "ProceduralParams" {
                    continue;
                }
                for param in xml.node_children(proc_child) {
                    let param_attrs = xml.node_attributes(param).collect::<HashMap<_, _>>();
                    if let Some(value) = param_attrs.get("value") {
                        params[xml.node_tag(param)] = serde_json::json!(value);
                    }
                }
            }
            values.push(serde_json::json!({
                "type": attrs.get("type").copied().unwrap_or_default(),
                "params": params,
            }));
        }
        values.extend(collect_fragment_procedurals(xml, child));
    }
    values
}

fn mannequin_adb_p4k_path(path: &str) -> String {
    let normalized = path.trim_start_matches("Data/").trim_start_matches("Data\\");
    let with_prefix = if normalized.to_ascii_lowercase().starts_with("animations/")
        || normalized.to_ascii_lowercase().starts_with("animations\\")
    {
        normalized.to_string()
    } else {
        format!("Animations/Mannequin/ADB/{normalized}")
    };
    format!("Data/{}", with_prefix).replace('/', "\\")
}

/// Structured dump of a Mannequin ADB plus its companion ControllerDef
/// XML for diagnostic / debug tooling. Returns a JSON value with one
/// entry per Mannequin Fragment containing `fragment` (group name),
/// `guid`, `tags`, `frag_tags`, `blend_out_duration`, `option_weight`,
/// `animations`, `scopes` (resolved from the ControllerDef), and any
/// `procedurals`. Used by the StarBreaker MCP `mannequin_dump` tool.
///
/// Phase 37 conclusion: ADB fragment metadata is captured at
/// fragment scope only — there is no per-bone blend-mode flag.
/// CAF/DBA `Controller` chunks expose `rot_format_flags` and
/// `pos_format_flags` per bone (now visible via `dba_dump`), but
/// these encode keyframe compression format, not additive/override
/// blend mode. Both are surfaced via MCP so empirical inspection can
/// be done from agent sessions; the canonical fallback when neither
/// distinguishes a bone is the geometric convex-hull test (Phase 38).
pub fn dump_mannequin_adb_to_json(
    p4k: &starbreaker_p4k::MappedP4k,
    source: &AnimationControllerSource,
    filter: Option<&str>,
) -> Result<serde_json::Value, Error> {
    let scopes = read_controller_fragment_scopes(p4k, &source.animation_controller);
    let adb_path = mannequin_adb_p4k_path(&source.animation_database);
    let data = p4k
        .entry_case_insensitive(&adb_path)
        .and_then(|entry| p4k.read(entry).ok())
        .ok_or_else(|| Error::Other(format!("Cannot load Mannequin ADB: {adb_path}")))?
        .to_vec();
    let xml = starbreaker_cryxml::from_bytes(&data)
        .map_err(|error| Error::Other(format!("Mannequin ADB CryXml parse: {error:?}")))?;
    let mut fragments = Vec::new();
    collect_mannequin_fragments(&xml, xml.root(), None, false, &scopes, &mut fragments);

    let filter_lc = filter.map(|f| f.to_ascii_lowercase());
    let filtered: Vec<serde_json::Value> = fragments
        .into_iter()
        .filter(|f| {
            let Some(needle) = filter_lc.as_ref() else {
                return true;
            };
            // Match against fragment group name, GUID, or any animation name.
            if let Some(group) = f.get("fragment").and_then(|v| v.as_str()) {
                if group.to_ascii_lowercase().contains(needle) {
                    return true;
                }
            }
            if let Some(guid) = f.get("guid").and_then(|v| v.as_str()) {
                if guid.to_ascii_lowercase().contains(needle) {
                    return true;
                }
            }
            if let Some(anims) = f.get("animations").and_then(|v| v.as_array()) {
                for a in anims {
                    if let Some(n) = a.get("name").and_then(|v| v.as_str()) {
                        if n.to_ascii_lowercase().contains(needle) {
                            return true;
                        }
                    }
                }
            }
            false
        })
        .collect();

    Ok(serde_json::json!({
        "animation_database": source.animation_database,
        "animation_controller": source.animation_controller,
        "adb_path": adb_path,
        "fragment_count": filtered.len(),
        "fragments": filtered,
    }))
}

fn clip_name_lookup_keys(name: &str) -> Vec<String> {
    let lower = name.trim().replace('\\', "/").to_ascii_lowercase();
    let stem = lower
        .rsplit_once('/')
        .map(|(_, tail)| tail)
        .unwrap_or(lower.as_str())
        .trim_end_matches(".caf")
        .to_string();
    if stem == lower {
        vec![lower]
    } else {
        vec![lower, stem]
    }
}

fn split_tag_list(raw: &str) -> Vec<String> {
    raw.split(|ch: char| ch == '+' || ch == '|' || ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_f32_attr(raw: Option<&str>) -> Option<f32> {
    raw.and_then(|value| value.parse::<f32>().ok())
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

    // NOTE: clip direction is *not* corrected here.
    //
    // The previous Phase 24B implementation called `correct_clip_direction`,
    // which inferred "expected" temporal direction from substrings of the
    // clip name (`deploy`/`open`/`extend` vs. `retract`/`close`/`compress`)
    // and reversed keyframe time when the bind-distance heuristic disagreed.
    // That logic was both name-based (a forbidden hard-coding pattern in
    // this codebase) and based on a wrong assumption — that the bind pose
    // is always the "closed/retracted" state. For Scorpius wings the bind
    // pose is the *deployed* state, so the heuristic reversed `wings_deploy`
    // into a clip that ends in the retracted state, breaking snap-to-state.
    //
    // Direction is now resolved on the addon side using authoritative
    // Mannequin fragment metadata (`speed`, `frag_tags`) and per-channel
    // cyclic detection. The exporter emits the clip with its authored
    // keyframe order; whichever fragment references it provides the
    // semantic mapping (Deploy/Retract/Open/Close, forward or `speed=-1`).
    //
    // See `package_ops._fragment_endpoint_policy` and
    // `_apply_best_channel_transform` for the consuming logic.

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
        // Phase 38 (deferred): a per-bone CAF blend-mode classifier was
        // attempted here using AABB-of-CAF-samples vs CHR-bind containment.
        // Empirically the test inverts the additive/override split (over-
        // marks stationary tracks as override). Phase 37 confirmed neither
        // CAF Controller flags nor Mannequin ADB carry the bit. The
        // `BoneBlendMode` enum, `classify_bone_blend_modes` helper, and
        // `annotate_animations_json_with_blend_modes` helper remain as
        // latent infrastructure for a future data-grounded discriminator;
        // the addon's runtime override path consumes the field when set.
        let value = database_to_animations_json(&AnimationDatabase { clips });
        return Ok(Some(value));
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

    #[test]
    fn clip_to_json_position_axis_swap_matches_static_import() {
        // Pin the CryEngine Y-up → Blender Z-up axis swap for animation
        // position keyframes. This MUST match the static-import convention
        // used by the addon's `_scene_position_to_blender` in
        // `blender_addon/starbreaker_addon/runtime/importer/utils.py`,
        // which maps (cry_x, cry_y, cry_z) → (cry_x, -cry_z, cry_y). If
        // the two diverge, animation deltas land in a different basis than
        // the bone's bind pose and the result is the inverted X-shape
        // failure documented in `docs/StarBreaker/animation-research.md`
        // (Scorpius wing-deploy kinematics).
        let clip = AnimationClip {
            name: "test_clip".to_string(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash: 0xDEADBEEF,
                rotations: vec![],
                positions: vec![Keyframe {
                    time: 0.0,
                    value: [1.0, 2.0, 3.0],
                }],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
        };

        let json = clip_to_json(&clip);
        let bones = json["bones"].as_object().unwrap();
        let entry = bones.values().next().unwrap();
        let pos = entry["position"].as_array().unwrap();
        let kf = pos[0].as_array().unwrap();
        assert_eq!(kf[0].as_f64().unwrap(), 1.0, "Blender X must be cry_x");
        assert_eq!(kf[1].as_f64().unwrap(), -3.0, "Blender Y must be -cry_z");
        assert_eq!(kf[2].as_f64().unwrap(), 2.0, "Blender Z must be cry_y");
        let pos_times = entry["position_time"].as_array().unwrap();
        assert_eq!(pos_times[0].as_f64().unwrap(), 0.0, "Position key time must survive JSON export");
    }

    #[test]
    fn clip_to_json_preserves_rotation_times() {
        let clip = AnimationClip {
            name: "timed_clip".to_string(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash: 0xDEADBEEF,
                rotations: vec![Keyframe {
                    time: 12.5,
                    value: [0.0, 0.0, 0.0, 1.0],
                }],
                positions: vec![],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
        };

        let json = clip_to_json(&clip);
        let bones = json["bones"].as_object().unwrap();
        let entry = bones.values().next().unwrap();
        let rotation_times = entry["rotation_time"].as_array().unwrap();
        assert_eq!(rotation_times[0].as_f64().unwrap(), 12.5);
    }

    #[test]
    fn cry_xyzw_to_blender_wxyz_axis_swap_matches_position_swap() {
        // The quaternion's vector component must transform under the same
        // basis change as positions: CryEngine (cx, cy, cz) → Blender
        // (cx, -cz, cy). For an input quaternion (qx=1, qy=2, qz=3, qw=4)
        // the Blender WXYZ form must be (4, 1, -3, 2). If this drifts from
        // the position swap (e.g. picks up the legacy (cy, -cz, cx)
        // convention), animation rotations land in a basis 90° away from
        // their position deltas and the wing-deploy X-shape collapses.
        let q = [1.0_f32, 2.0, 3.0, 4.0]; // CryEngine xyzw
        let blender = cry_xyzw_to_blender_wxyz(q);
        assert_eq!(blender[0], 4.0, "Blender W = cry_w");
        assert_eq!(blender[1], 1.0, "Blender X axis = cry_x axis");
        assert_eq!(blender[2], -3.0, "Blender Y axis = -cry_z axis");
        assert_eq!(blender[3], 2.0, "Blender Z axis = cry_y axis");
    }

    #[test]
    fn sanitize_clip_filename_replaces_unsafe_chars() {
        assert_eq!(sanitize_clip_filename("landing_gear_extend"), "landing_gear_extend");
        assert_eq!(sanitize_clip_filename("Animations/canopy.caf"), "Animations_canopy.caf");
        assert_eq!(sanitize_clip_filename("foo bar/baz\\qux"), "foo_bar_baz_qux");
        assert_eq!(sanitize_clip_filename(""), "clip");
        assert_eq!(sanitize_clip_filename("   "), "clip");
        assert_eq!(sanitize_clip_filename("clip-1.0_v2"), "clip-1.0_v2");
    }

    #[test]
    fn split_clip_for_sidecar_extracts_bones_and_records_sidecar() {
        let clip = AnimationClip {
            name: "landing_gear_extend".to_string(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash: 0xCAFEBABE,
                rotations: vec![Keyframe { time: 0.0, value: [0.0, 0.0, 0.0, 1.0] }],
                positions: vec![Keyframe { time: 0.0, value: [1.0, 2.0, 3.0] }],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
        };
        let mut full = clip_to_json(&clip);
        // Mimic fragment annotation by adding a fragments key.
        full["fragments"] = serde_json::json!([{"tags": "Deploy"}]);

        let sidecar_rel = "animations/landing_gear_extend.json";
        let (index, body) = split_clip_for_sidecar(&full, sidecar_rel);

        // Index keeps lightweight metadata + sidecar reference, drops bones.
        assert_eq!(index["name"].as_str().unwrap(), "landing_gear_extend");
        assert_eq!(index["fps"].as_u64().unwrap(), 30);
        assert!(index["frame_count"].is_number());
        assert_eq!(index["sidecar"].as_str().unwrap(), sidecar_rel);
        assert_eq!(index["fragments"], serde_json::json!([{"tags": "Deploy"}]));
        assert!(index.get("bones").is_none(), "Index must not carry bones");

        // Body is the full clip, including bones.
        assert_eq!(body["name"].as_str().unwrap(), "landing_gear_extend");
        assert!(body.get("bones").is_some(), "Sidecar body must carry bones");
        let bones = body["bones"].as_object().unwrap();
        assert_eq!(bones.len(), 1);
    }

    #[test]
    fn classify_bone_blend_modes_marks_outlier_bones_override() {
        // additive bone: bind sits inside the AABB of CAF samples.
        let additive_hash = 0xAAAA_AAAA_u32;
        // override bone: bind is far outside the AABB on at least one axis.
        let override_hash = 0xBBBB_BBBB_u32;
        // bone with no position samples — must be omitted from result.
        let unsampled_hash = 0xCCCC_CCCC_u32;

        let clips = vec![AnimationClip {
            name: "deploy".to_string(),
            fps: 30.0,
            channels: vec![
                BoneChannel {
                    bone_hash: additive_hash,
                    rotations: vec![],
                    positions: vec![
                        Keyframe { time: 0.0, value: [0.0, 0.0, 0.0] },
                        Keyframe { time: 1.0, value: [1.0, 1.0, 1.0] },
                    ],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
                BoneChannel {
                    bone_hash: override_hash,
                    rotations: vec![],
                    positions: vec![
                        Keyframe { time: 0.0, value: [10.0, 0.0, 0.0] },
                        Keyframe { time: 1.0, value: [11.0, 1.0, 0.0] },
                    ],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
                BoneChannel {
                    bone_hash: unsampled_hash,
                    rotations: vec![Keyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    }],
                    positions: vec![],
                    rot_format_flags: 0,
                    pos_format_flags: 0,
                },
            ],
        }];

        let mut binds = std::collections::HashMap::new();
        // Additive bind sits inside the AABB.
        binds.insert(additive_hash, [0.5_f32, 0.5, 0.5]);
        // Override bind sits 8m off the AABB on X.
        binds.insert(override_hash, [2.0_f32, 0.0, 0.0]);
        // Unsampled bone has a bind but no samples — must be omitted.
        binds.insert(unsampled_hash, [0.0_f32, 0.0, 0.0]);

        let modes = classify_bone_blend_modes(&clips, &binds);
        assert_eq!(modes.get(&additive_hash), Some(&BoneBlendMode::Additive));
        assert_eq!(modes.get(&override_hash), Some(&BoneBlendMode::Override));
        assert!(
            !modes.contains_key(&unsampled_hash),
            "Bones without position samples must not be classified"
        );

        // Round-trip through the JSON annotator.
        let mut clips_json =
            database_to_animations_json(&AnimationDatabase { clips: clips.clone() });
        annotate_animations_json_with_blend_modes(&mut clips_json, &modes);
        let bones = clips_json[0]["bones"].as_object().unwrap();
        assert_eq!(
            bones[&format!("0x{additive_hash:X}")]["blend_mode"]
                .as_str()
                .unwrap(),
            "additive"
        );
        assert_eq!(
            bones[&format!("0x{override_hash:X}")]["blend_mode"]
                .as_str()
                .unwrap(),
            "override"
        );
    }

    /// Phase 45 regression: SNORM-packed (`0xC2`) position channels with two
    /// active axes use **planar (axis-major)** layout, not interleaved
    /// (key-major). The decoder previously produced correct results only for
    /// single-active-axis channels (where planar ≡ interleaved); multi-axis
    /// channels (e.g. `Wing_Grabber_Main_Bottom_Right` in Scorpius
    /// `wings_deploy`) were catastrophically misaligned, causing
    /// `BR[i] ≈ BL[2*i]` for the first 22 keys and a flatline thereafter.
    /// See [`docs/StarBreaker/todo.md`] Phase 45 for the byte-level evidence.
    #[test]
    fn snorm_packed_two_active_axes_uses_planar_layout() {
        // Synthesize a 4-key channel with X inactive (FLT_MAX), Y and Z
        // active. Planar layout: [Y0,Y1,Y2,Y3 as 8 bytes][Z0,Z1,Z2,Z3 as 8
        // bytes]. With Y u16s = [0, 1000, 2000, 3000] and Z u16s =
        // [10000, 20000, 30000, 40000], scale_y=1.0, scale_z=0.001, the
        // expected decoded last key is (offset_x, 3000.0+offset_y,
        // 40.0+offset_z). If the old interleaved decode were used, the last
        // key would consume bytes 24..28 (= Z stream bytes 0..4) and produce
        // a totally different value pair.
        let mut bytes = Vec::new();
        // 24-byte header: scale Vec3 + offset Vec3
        bytes.extend_from_slice(&f32::MAX.to_le_bytes()); // scale_x = FLT_MAX (inactive)
        bytes.extend_from_slice(&1.0f32.to_le_bytes());   // scale_y = 1.0
        bytes.extend_from_slice(&0.001f32.to_le_bytes()); // scale_z = 0.001
        bytes.extend_from_slice(&100.0f32.to_le_bytes()); // offset_x = 100
        bytes.extend_from_slice(&200.0f32.to_le_bytes()); // offset_y = 200
        bytes.extend_from_slice(&300.0f32.to_le_bytes()); // offset_z = 300
        // Planar Y stream (4 keys × u16):
        for v in [0u16, 1000, 2000, 3000] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        // Planar Z stream (4 keys × u16):
        for v in [10000u16, 20000, 30000, 40000] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        let positions = read_snorm_packed_positions(&bytes, 0, 4).expect("decode");
        assert_eq!(positions.len(), 4);
        // X is inactive — value is the offset directly.
        for p in &positions {
            assert_eq!(p[0], 100.0, "X must equal offset for inactive axis");
        }
        // Y values: u16 * 1.0 + 200
        let expected_y = [200.0, 1200.0, 2200.0, 3200.0];
        // Z values: u16 * 0.001 + 300
        let expected_z = [310.0, 320.0, 330.0, 340.0];
        for i in 0..4 {
            assert!(
                (positions[i][1] - expected_y[i]).abs() < 1e-3,
                "Y[{i}] = {} (want {})",
                positions[i][1],
                expected_y[i]
            );
            assert!(
                (positions[i][2] - expected_z[i]).abs() < 1e-3,
                "Z[{i}] = {} (want {})",
                positions[i][2],
                expected_z[i]
            );
        }
    }

    /// Single-active-axis `0xC2` channels must continue to decode identically
    /// to the pre-Phase-45 behaviour (planar ≡ interleaved when n_active=1).
    #[test]
    fn snorm_packed_single_active_axis_unchanged() {
        let mut bytes = Vec::new();
        // X and Z inactive, Y active.
        bytes.extend_from_slice(&f32::MAX.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes()); // scale_y = 2.0
        bytes.extend_from_slice(&f32::MAX.to_le_bytes());
        bytes.extend_from_slice(&(-5.0f32).to_le_bytes()); // offset_x = -5
        bytes.extend_from_slice(&10.0f32.to_le_bytes());   // offset_y = 10
        bytes.extend_from_slice(&7.0f32.to_le_bytes());    // offset_z = 7
        for v in [0u16, 100, 200, 300] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let positions = read_snorm_packed_positions(&bytes, 0, 4).expect("decode");
        let expected_y = [10.0, 210.0, 410.0, 610.0];
        for i in 0..4 {
            assert_eq!(positions[i][0], -5.0);
            assert_eq!(positions[i][2], 7.0);
            assert!((positions[i][1] - expected_y[i]).abs() < 1e-3);
        }
    }

    #[test]
    fn dump_database_bone_filter_excludes_unmatched_and_unresolved() {
        // Build a minimal in-memory database with three bones to validate
        // that `bone_filter` keeps only resolved channels whose name
        // contains the substring (case-insensitive).
        let wing_left_hash = bone_name_hash("Wing_Mechanism_Bottom_Left");
        let wing_right_hash = bone_name_hash("Wing_Mechanism_Bottom_Right");
        let other_hash = bone_name_hash("Some_Other_Bone");
        let unresolved_hash: u32 = 0xDEADBEEF;

        let make_ch = |hash: u32| BoneChannel {
            bone_hash: hash,
            rotations: vec![Keyframe { time: 0.0, value: [0.0, 0.0, 0.0, 1.0] }],
            positions: vec![],
            rot_format_flags: 0,
            pos_format_flags: 0,
        };

        let db = AnimationDatabase {
            clips: vec![AnimationClip {
                name: "wings_deploy".to_string(),
                fps: 30.0,
                channels: vec![
                    make_ch(wing_left_hash),
                    make_ch(wing_right_hash),
                    make_ch(other_hash),
                    make_ch(unresolved_hash),
                ],
            }],
        };
        let mut hash_to_name = std::collections::HashMap::new();
        hash_to_name.insert(wing_left_hash, "Wing_Mechanism_Bottom_Left".to_string());
        hash_to_name.insert(wing_right_hash, "Wing_Mechanism_Bottom_Right".to_string());
        hash_to_name.insert(other_hash, "Some_Other_Bone".to_string());

        // No bone_filter: all 4 channels pass through.
        let no_filter =
            dump_database_to_json(&db, &hash_to_name, None, None, false);
        assert_eq!(no_filter["clips"][0]["channels"].as_array().unwrap().len(), 4);

        // bone_filter="wing_mechanism" (case-insensitive): only the two wings.
        let wings =
            dump_database_to_json(&db, &hash_to_name, None, Some("wing_mechanism"), false);
        let chans = wings["clips"][0]["channels"].as_array().unwrap();
        assert_eq!(chans.len(), 2);
        for ch in chans {
            assert!(ch["bone_name"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
                .contains("wing_mechanism"));
        }

        // bone_filter without a skeleton (empty hash_to_name) excludes everything.
        let no_skel = dump_database_to_json(
            &db,
            &std::collections::HashMap::new(),
            None,
            Some("wing_mechanism"),
            false,
        );
        assert_eq!(
            no_skel["clips"][0]["channels"].as_array().unwrap().len(),
            0,
            "channels with unresolved hashes must be excluded when bone_filter is set"
        );
    }

    #[test]
    fn time_format_0x42_decodes_per_frame_keyframe_bitmap() {
        // Phase 47: time format 0x02/0x42 is a per-frame keyframe bitmap of
        // (end - start + 1) bits, LSB-first per byte. Each set bit at index
        // `b` indicates a keyframe at frame `start + b`. The first 4 bytes
        // of the bitmap occupy the slot historically misread as a u32
        // "marker"; the rest follows immediately after.
        //
        // Sample below is the Scorpius `wings_deploy.caf` Top-Right wing
        // mechanism (bone hash 0x5F3AF303). num_rot = 24, end = 75, so the
        // bitmap is 76 bits = 10 bytes. Byte sequence (incl. start/end u16
        // pair) verified empirically by `dump_dba_time_stream` against the
        // shipped Scorpius DBA.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u16.to_le_bytes()); // start
        bytes.extend_from_slice(&75u16.to_le_bytes()); // end
        // 10 bytes of bitmap, LSB-first per byte:
        bytes.extend_from_slice(&[
            0xa5, 0x92, 0x72, 0x8a, // first 4 bytes (was the "marker")
            0x25, 0x59, 0x0a, 0x00, 0x00, 0x08, // remaining 6 bytes
        ]);

        let times = read_time_keys(&bytes, 0, 24, 0x8242).expect("decode bitmap");
        assert_eq!(times.len(), 24, "expected 24 keys, got {}", times.len());
        // Verify a few: first set bit in 0xa5 (= 1010 0101 LSB-first) is
        // bit 0 → frame 0, then bit 2 → frame 2, bit 5 → frame 5, bit 7
        // → frame 7.
        assert_eq!(times[0], 0.0);
        assert_eq!(times[1], 2.0);
        assert_eq!(times[2], 5.0);
        assert_eq!(times[3], 7.0);
        // Last key must reach frame 75 (the end of the bitmap), since
        // 0x08 in byte 9 has bit 3 set → frame 9*8+3 = 75.
        assert_eq!(*times.last().unwrap(), 75.0);
    }

    #[test]
    fn time_format_0x42_count_mismatch_falls_back_to_uniform() {
        // If the encoded bitmap's set-bit count disagrees with the
        // controller's `num_rot_keys`, fall back to uniform stretch so the
        // export still yields something playable. We do NOT silently
        // truncate or pad.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&7u16.to_le_bytes());
        bytes.extend_from_slice(&[0xff]); // 8 bits set → 8 keys
        let times = read_time_keys(&bytes, 0, 5, 0x0042).expect("decode");
        assert_eq!(times.len(), 5);
        // Uniform fallback: 0, 1.75, 3.5, 5.25, 7.0
        assert!((times[0] - 0.0).abs() < 1e-5);
        assert!((times[4] - 7.0).abs() < 1e-5);
    }
}


