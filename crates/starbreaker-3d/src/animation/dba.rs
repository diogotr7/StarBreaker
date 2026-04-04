//! Parser for `.dba` (Animation Database) and `.caf` (Animation Clip) IVO files.
//!
//! Both formats use IVO container with animation blocks. A `.dba` packs multiple
//! clips, while `.caf` has a single clip.
//!
//! ## Block structure
//! ```text
//! Header (12 bytes): signature("#caf"/"#dba") + bone_count(u16) + magic(u16) + data_size(u32)
//! Bone hashes: [u32; bone_count]  — CRC32 of lowercase bone names
//! Controllers: [ControllerEntry; bone_count]  — 24 bytes each (rot track + pos track)
//! Keyframe data at offsets referenced by controllers
//! ```

use starbreaker_chunks::ChunkFile;
use crate::error::Error;

/// A parsed animation database containing one or more animation clips.
#[derive(Debug)]
pub struct AnimationDatabase {
    pub clips: Vec<AnimationClip>,
}

/// A single animation clip with per-bone channels.
#[derive(Debug)]
pub struct AnimationClip {
    /// Animation name (from DBA metadata, or filename for CAF).
    pub name: String,
    /// Frames per second (from metadata, default 30).
    pub fps: f32,
    /// Per-bone animation channels.
    pub channels: Vec<BoneChannel>,
}

/// Animation data for a single bone.
#[derive(Debug)]
pub struct BoneChannel {
    /// CRC32 hash of the bone name (lowercase ASCII).
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

/// Raw controller entry from the animation block (24 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
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

/// DBA metadata entry (44 bytes per animation).
#[derive(Debug)]
struct DbaMetaEntry {
    flags: u32,
    fps: u16,
    num_controllers: u16,
    _unknown1: u32,
    _unknown2: u32,
    start_rotation: [f32; 4],
    start_position: [f32; 3],
}

/// IVO chunk type IDs for animation data.
mod chunk_types {
    pub const DBA_DATA: u32 = 0x194FBC50;   // IvoDBAData, version 0x0900
    pub const DBA_META: u32 = 0xF7351608;   // IvoDBAMetadata, version 0x0900
    pub const CAF_DATA: u32 = 0xA9496CB5;   // IvoCAFData, version 0x0900
    pub const ANIM_INFO: u32 = 0x4733C6ED;  // IvoAnimInfo, version 0x0901
}

/// Parse a `.dba` file from raw bytes.
pub fn parse_dba(data: &[u8]) -> Result<AnimationDatabase, Error> {
    let chunk_file = ChunkFile::from_bytes(data)?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return Err(Error::UnsupportedFormat),
    };

    // Find DBA data and metadata chunks.
    let db_data_chunk = ivo.chunks().iter()
        .find(|c| c.chunk_type == chunk_types::DBA_DATA);
    let db_meta_chunk = ivo.chunks().iter()
        .find(|c| c.chunk_type == chunk_types::DBA_META);

    let Some(db_data) = db_data_chunk else {
        return Err(Error::Other("No DBA data chunk found".into()));
    };

    let data_bytes = ivo.chunk_data(db_data);
    let meta_bytes = db_meta_chunk.map(|c| ivo.chunk_data(c));

    // Parse metadata entries (animation names + fps).
    let meta_entries = meta_bytes
        .map(|b| parse_dba_metadata(b))
        .unwrap_or_default();

    // Parse animation blocks from DbData.
    let blocks = parse_animation_blocks(data_bytes)?;

    // Combine blocks with metadata.
    let clips: Vec<AnimationClip> = blocks.into_iter().enumerate().map(|(i, block)| {
        let (name, fps) = meta_entries.get(i)
            .map(|(name, meta)| (name.clone(), meta.fps as f32))
            .unwrap_or_else(|| (format!("anim_{i}"), 30.0));
        AnimationClip {
            name,
            fps,
            channels: block,
        }
    }).collect();

    Ok(AnimationDatabase { clips })
}

/// Parse a `.caf` file from raw bytes.
pub fn parse_caf(data: &[u8]) -> Result<AnimationDatabase, Error> {
    let chunk_file = ChunkFile::from_bytes(data)?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return Err(Error::UnsupportedFormat),
    };

    // Find AnimInfo for FPS.
    let anim_info = ivo.chunks().iter()
        .find(|c| c.chunk_type == chunk_types::ANIM_INFO)
        .map(|c| parse_anim_info(ivo.chunk_data(c)));

    let fps = anim_info.as_ref().map(|i| i.fps as f32).unwrap_or(30.0);

    // Find CAF data chunk.
    let caf_chunk = ivo.chunks().iter()
        .find(|c| c.chunk_type == chunk_types::CAF_DATA)
        .ok_or_else(|| Error::Other("No CAF data chunk found".into()))?;

    let data_bytes = ivo.chunk_data(caf_chunk);
    let blocks = parse_animation_blocks(data_bytes)?;

    let clips = blocks.into_iter().enumerate().map(|(i, channels)| {
        AnimationClip {
            name: format!("clip_{i}"),
            fps,
            channels,
        }
    }).collect();

    Ok(AnimationDatabase { clips })
}

// ─── Animation block parsing ────────────────────────────────────────────────

/// Parse one or more animation blocks from raw data.
/// DBA files contain multiple sequential blocks; CAF files contain one.
fn parse_animation_blocks(data: &[u8]) -> Result<Vec<Vec<BoneChannel>>, Error> {
    let mut blocks = Vec::new();
    let mut offset = 0;

    // DBA: first 4 bytes is total data size
    if data.len() >= 4 {
        let total_size = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        if total_size > 0 && total_size <= data.len() {
            offset = 4; // skip total size field
        }
    }

    while offset + 12 <= data.len() {
        // Check for #caf or #dba signature
        let sig = &data[offset..offset + 4];
        if sig != b"#caf" && sig != b"#dba" {
            break;
        }

        let bone_count = u16::from_le_bytes(data[offset + 4..offset + 6].try_into().unwrap()) as usize;
        let _magic = u16::from_le_bytes(data[offset + 6..offset + 8].try_into().unwrap());
        let _data_size = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()) as usize;

        let block_start = offset + 12; // after 12-byte header
        // Headers are packed sequentially: header(12) + bone_hashes(bone_count*4) + controllers(bone_count*24)
        // The NEXT block starts right after the controller entries.
        // Keyframe data lives at the end, accessed via absolute offsets from controllers.
        let headers_end = block_start + bone_count * 4 + bone_count * 24;

        match parse_single_block(data, block_start, bone_count) {
            Ok(channels) => blocks.push(channels),
            Err(e) => log::warn!("Failed to parse animation block at 0x{offset:x}: {e}"),
        }

        offset = headers_end;
    }

    Ok(blocks)
}

/// Parse a single animation block.
/// `data` is the full chunk data; `start` is the offset after the 12-byte block header.
/// Controller offsets are absolute (relative to each controller's position in the data).
fn parse_single_block(data: &[u8], start: usize, bone_count: usize) -> Result<Vec<BoneChannel>, Error> {
    let mut pos = start;

    // Bone hash array: bone_count × u32
    let hash_size = bone_count * 4;
    if pos + hash_size > data.len() {
        return Err(Error::Other("Bone hash array extends past block".into()));
    }
    let bone_hashes: Vec<u32> = (0..bone_count)
        .map(|i| u32::from_le_bytes(data[pos + i * 4..pos + i * 4 + 4].try_into().unwrap()))
        .collect();
    pos += hash_size;

    // Controller entries: bone_count × 24 bytes.
    // Offsets in each controller are relative to the start of THAT controller entry.
    let ctrl_size = bone_count * 24;
    if pos + ctrl_size > data.len() {
        return Err(Error::Other("Controller entries extend past block".into()));
    }
    let mut controllers: Vec<(usize, ControllerEntry)> = Vec::with_capacity(bone_count);
    for i in 0..bone_count {
        let o = pos + i * 24;
        controllers.push((o, ControllerEntry {
            num_rot_keys: u16::from_le_bytes(data[o..o + 2].try_into().unwrap()),
            rot_format_flags: u16::from_le_bytes(data[o + 2..o + 4].try_into().unwrap()),
            rot_time_offset: u32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()),
            rot_data_offset: u32::from_le_bytes(data[o + 8..o + 12].try_into().unwrap()),
            num_pos_keys: u16::from_le_bytes(data[o + 12..o + 14].try_into().unwrap()),
            pos_format_flags: u16::from_le_bytes(data[o + 14..o + 16].try_into().unwrap()),
            pos_time_offset: u32::from_le_bytes(data[o + 16..o + 20].try_into().unwrap()),
            pos_data_offset: u32::from_le_bytes(data[o + 20..o + 24].try_into().unwrap()),
        }));
    }

    let mut channels = Vec::with_capacity(bone_count);
    for (i, (ctrl_offset, ctrl)) in controllers.iter().enumerate() {
        if i == 0 && ctrl.num_rot_keys > 0 {
            log::debug!("ctrl[0] at offset 0x{:x}: rot_keys={} rot_fmt=0x{:04x} rot_time_off=0x{:x} rot_data_off=0x{:x} -> abs data at 0x{:x}",
                ctrl_offset, ctrl.num_rot_keys, ctrl.rot_format_flags,
                ctrl.rot_time_offset, ctrl.rot_data_offset,
                ctrl_offset + ctrl.rot_data_offset as usize);
        }
        // Offsets in the controller entry are relative to the start of the chunk data
        // (not relative to the controller entry itself as in some other CryEngine formats).
        let rotations = if ctrl.num_rot_keys > 0 {
            let times = if ctrl.rot_time_offset > 0 {
                read_time_keys(data, ctrl.rot_time_offset as usize, ctrl.num_rot_keys as usize, ctrl.rot_format_flags)?
            } else {
                (0..ctrl.num_rot_keys as usize).map(|t| t as f32).collect()
            };
            let values = read_rotation_keys(data, ctrl.rot_data_offset as usize, ctrl.num_rot_keys as usize, ctrl.rot_format_flags)?;
            times.into_iter().zip(values).map(|(t, v)| Keyframe { time: t, value: v }).collect()
        } else {
            Vec::new()
        };

        let positions = if ctrl.num_pos_keys > 0 {
            let times = if ctrl.pos_time_offset > 0 {
                read_time_keys(data, ctrl.pos_time_offset as usize, ctrl.num_pos_keys as usize, ctrl.pos_format_flags)?
            } else {
                (0..ctrl.num_pos_keys as usize).map(|t| t as f32).collect()
            };
            let values = read_position_keys(data, ctrl.pos_data_offset as usize, ctrl.num_pos_keys as usize, ctrl.pos_format_flags)?;
            times.into_iter().zip(values).map(|(t, v)| Keyframe { time: t, value: v }).collect()
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

// ─── DBA metadata parsing ───────────────────────────────────────────────────

/// Parse DBA metadata chunk: animation entries + name string table.
fn parse_dba_metadata(data: &[u8]) -> Vec<(String, DbaMetaEntry)> {
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let entry_size = 44;
    let entries_end = 4 + count * entry_size;
    if entries_end > data.len() {
        return Vec::new();
    }

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let o = 4 + i * entry_size;
        let entry = DbaMetaEntry {
            flags: u32::from_le_bytes(data[o..o + 4].try_into().unwrap()),
            fps: u16::from_le_bytes(data[o + 4..o + 6].try_into().unwrap()),
            num_controllers: u16::from_le_bytes(data[o + 6..o + 8].try_into().unwrap()),
            _unknown1: u32::from_le_bytes(data[o + 8..o + 12].try_into().unwrap()),
            _unknown2: u32::from_le_bytes(data[o + 12..o + 16].try_into().unwrap()),
            start_rotation: [
                f32::from_le_bytes(data[o + 16..o + 20].try_into().unwrap()),
                f32::from_le_bytes(data[o + 20..o + 24].try_into().unwrap()),
                f32::from_le_bytes(data[o + 24..o + 28].try_into().unwrap()),
                f32::from_le_bytes(data[o + 28..o + 32].try_into().unwrap()),
            ],
            start_position: [
                f32::from_le_bytes(data[o + 32..o + 36].try_into().unwrap()),
                f32::from_le_bytes(data[o + 36..o + 40].try_into().unwrap()),
                f32::from_le_bytes(data[o + 40..o + 44].try_into().unwrap()),
            ],
        };
        entries.push(entry);
    }

    // Parse null-terminated name strings after the entries.
    let mut names = Vec::with_capacity(count);
    let mut pos = entries_end;
    for _ in 0..count {
        let end = data[pos..].iter().position(|&b| b == 0).unwrap_or(data.len() - pos);
        let name = std::str::from_utf8(&data[pos..pos + end]).unwrap_or("").to_string();
        names.push(name);
        pos += end + 1; // skip null terminator
    }

    names.into_iter().zip(entries).collect()
}

/// Parse AnimInfo chunk (48 bytes).
struct AnimInfo {
    flags: u32,
    fps: u16,
    num_bones: u16,
    end_frame: u32,
}

fn parse_anim_info(data: &[u8]) -> AnimInfo {
    AnimInfo {
        flags: u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4])),
        fps: u16::from_le_bytes(data[4..6].try_into().unwrap_or([0; 2])),
        num_bones: u16::from_le_bytes(data[6..8].try_into().unwrap_or([0; 2])),
        end_frame: u32::from_le_bytes(data[12..16].try_into().unwrap_or([0; 4])),
    }
}

// ─── Time key reading ───────────────────────────────────────────────────────

fn read_time_keys(data: &[u8], offset: usize, count: usize, format_flags: u16) -> Result<Vec<f32>, Error> {
    let time_format = format_flags & 0xFF;
    match time_format {
        // Format 0x00: byte array — each key is 1 byte, used directly as frame number
        0x00 => {
            if offset + count > data.len() {
                return Err(Error::Other(format!("Time keys overflow at 0x{offset:x}")));
            }
            Ok((0..count).map(|i| data[offset + i] as f32).collect())
        }
        // Format 0x02 / 0x42: uint16 header with interpolation
        // 8-byte header: start(u16) + end(u16) + marker(u32), then interpolate
        0x02 | 0x42 => {
            if offset + 8 > data.len() {
                return Err(Error::Other(format!("Time header overflow at 0x{offset:x}")));
            }
            let start = u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()) as f32;
            let end = u16::from_le_bytes(data[offset + 2..offset + 4].try_into().unwrap()) as f32;
            if count <= 1 {
                return Ok(vec![start]);
            }
            Ok((0..count).map(|i| {
                start + (end - start) * i as f32 / (count - 1) as f32
            }).collect())
        }
        _ => {
            log::warn!("Unknown time format 0x{time_format:02x} at offset 0x{offset:x}, using linear 0..N");
            Ok((0..count).map(|i| i as f32).collect())
        }
    }
}

// ─── Rotation key reading ───────────────────────────────────────────────────

fn read_rotation_keys(data: &[u8], offset: usize, count: usize, _format_flags: u16) -> Result<Vec<[f32; 4]>, Error> {
    // IVO DBA/CAF always uses uncompressed quaternions (16 bytes each: x,y,z,w).
    // The format flags control the TIME encoding, not the rotation compression.
    read_uncompressed_quats(data, offset, count)
}

fn read_uncompressed_quats(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 4]>, Error> {
    let size = count * 16;
    if offset + size > data.len() {
        return Err(Error::Other(format!("Uncompressed quats overflow at 0x{offset:x}")));
    }
    Ok((0..count).map(|i| {
        let o = offset + i * 16;
        [
            f32::from_le_bytes(data[o..o + 4].try_into().unwrap()),
            f32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()),
            f32::from_le_bytes(data[o + 8..o + 12].try_into().unwrap()),
            f32::from_le_bytes(data[o + 12..o + 16].try_into().unwrap()),
        ]
    }).collect())
}

/// SmallTree48BitQuat: 6 bytes per quaternion.
/// 3 components at 15 bits each + 3-bit index of largest component.
/// Scale: 23170.0, Range: ±0.707106781186 (±1/√2)
fn read_small_tree_48bit_quats(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 4]>, Error> {
    let size = count * 6;
    if offset + size > data.len() {
        return Err(Error::Other(format!("SmallTree48BitQuat overflow at 0x{offset:x}")));
    }

    Ok((0..count).map(|i| {
        let o = offset + i * 6;
        let m1 = u16::from_le_bytes(data[o..o + 2].try_into().unwrap()) as u64;
        let m2 = u16::from_le_bytes(data[o + 2..o + 4].try_into().unwrap()) as u64;
        let m3 = u16::from_le_bytes(data[o + 4..o + 6].try_into().unwrap()) as u64;

        let packed = m1 | (m2 << 16) | (m3 << 32);
        decode_small_tree_quat_48(packed)
    }).collect())
}

fn decode_small_tree_quat_48(packed: u64) -> [f32; 4] {
    const SCALE: f32 = 23170.0;
    const RANGE: f32 = std::f32::consts::FRAC_1_SQRT_2; // 0.707106781186

    // 3-bit index of largest component
    let largest_idx = (packed >> 45) & 0x7;
    // 3 × 15-bit values
    let v0 = ((packed >> 0) & 0x7FFF) as f32 / SCALE - RANGE;
    let v1 = ((packed >> 15) & 0x7FFF) as f32 / SCALE - RANGE;
    let v2 = ((packed >> 30) & 0x7FFF) as f32 / SCALE - RANGE;

    // Reconstruct the 4th component from unit quaternion constraint
    let w_sq = 1.0 - v0 * v0 - v1 * v1 - v2 * v2;
    let w = if w_sq > 0.0 { w_sq.sqrt() } else { 0.0 };

    match largest_idx {
        0 => [w, v0, v1, v2],
        1 => [v0, w, v1, v2],
        2 => [v0, v1, w, v2],
        _ => [v0, v1, v2, w],
    }
}

// ─── Position key reading ───────────────────────────────────────────────────

fn read_position_keys(data: &[u8], offset: usize, count: usize, format_flags: u16) -> Result<Vec<[f32; 3]>, Error> {
    let pos_format = format_flags >> 8;
    match pos_format {
        // 0xC0: uncompressed float Vector3 (12 bytes per key)
        0xC0 => {
            let size = count * 12;
            if offset + size > data.len() {
                return Err(Error::Other(format!("Float positions overflow at 0x{offset:x}")));
            }
            Ok((0..count).map(|i| {
                let o = offset + i * 12;
                [
                    f32::from_le_bytes(data[o..o + 4].try_into().unwrap()),
                    f32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()),
                    f32::from_le_bytes(data[o + 8..o + 12].try_into().unwrap()),
                ]
            }).collect())
        }
        // 0xC1: SNORM full — 24-byte header + 6 bytes per key
        0xC1 => read_snorm_full_positions(data, offset, count),
        // 0xC2: SNORM packed — 24-byte header + variable bytes per key
        0xC2 => read_snorm_packed_positions(data, offset, count),
        _ => {
            log::warn!("Unknown position format 0x{pos_format:02x}, count={count}");
            Ok(vec![[0.0, 0.0, 0.0]; count])
        }
    }
}

/// SNORM full positions: 24-byte header (min Vec3 + max Vec3), then 6 bytes per key (3 × i16 SNORM).
fn read_snorm_full_positions(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 3]>, Error> {
    if offset + 24 + count * 6 > data.len() {
        return Err(Error::Other(format!("SNORM full positions overflow at 0x{offset:x}")));
    }
    let min = read_vec3(data, offset);
    let max = read_vec3(data, offset + 12);
    let range = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];

    Ok((0..count).map(|i| {
        let o = offset + 24 + i * 6;
        let sx = i16::from_le_bytes(data[o..o + 2].try_into().unwrap());
        let sy = i16::from_le_bytes(data[o + 2..o + 4].try_into().unwrap());
        let sz = i16::from_le_bytes(data[o + 4..o + 6].try_into().unwrap());
        [
            min[0] + (sx as f32 / 32767.0 + 1.0) * 0.5 * range[0],
            min[1] + (sy as f32 / 32767.0 + 1.0) * 0.5 * range[1],
            min[2] + (sz as f32 / 32767.0 + 1.0) * 0.5 * range[2],
        ]
    }).collect())
}

/// SNORM packed positions: 24-byte header + variable bytes per key.
/// Only encodes channels that actually change (active channel mask in header).
fn read_snorm_packed_positions(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 3]>, Error> {
    if offset + 24 > data.len() {
        return Err(Error::Other(format!("SNORM packed header overflow at 0x{offset:x}")));
    }
    let min = read_vec3(data, offset);
    let max = read_vec3(data, offset + 12);
    let range = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];

    // Determine active channels: channels with non-zero range
    let active: [bool; 3] = [
        range[0].abs() > 1e-10,
        range[1].abs() > 1e-10,
        range[2].abs() > 1e-10,
    ];
    let bytes_per_key: usize = active.iter().filter(|&&a| a).count() * 2;

    let data_start = offset + 24;
    if bytes_per_key > 0 && data_start + count * bytes_per_key > data.len() {
        return Err(Error::Other(format!("SNORM packed positions overflow at 0x{offset:x}")));
    }

    Ok((0..count).map(|i| {
        let o = data_start + i * bytes_per_key;
        let mut pos = min;
        let mut byte_offset = 0;
        for ch in 0..3 {
            if active[ch] {
                let sv = i16::from_le_bytes(data[o + byte_offset..o + byte_offset + 2].try_into().unwrap());
                pos[ch] = min[ch] + (sv as f32 / 32767.0 + 1.0) * 0.5 * range[ch];
                byte_offset += 2;
            }
        }
        pos
    }).collect())
}

fn read_vec3(data: &[u8], offset: usize) -> [f32; 3] {
    [
        f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()),
        f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()),
        f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()),
    ]
}
