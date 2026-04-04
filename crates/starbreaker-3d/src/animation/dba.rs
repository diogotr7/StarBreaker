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

/// DBA metadata entry (48 = 0x30 bytes per animation, v0x902).
///
/// Layout (verified by hex dump of Gladius DBA):
/// ```text
/// +0x00: flags0 (u32)
/// +0x04: flags1 (u32)
/// +0x08: fps (u16)
/// +0x0A: num_controllers (u16)
/// +0x0C: unknown_a (u16)
/// +0x0E: unknown_b (u16)
/// +0x10: unknown_c (u32)
/// +0x14: end_frame (u32)
/// +0x18: start_rotation (f32 × 4, quaternion)
/// +0x28: padding/unknown (8 bytes)
/// ```
#[derive(Debug)]
struct DbaMetaEntry {
    fps: u16,
    _num_controllers: u16,
    _end_frame: u32,
    _start_rotation: [f32; 4],
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

    // Use file data from chunk offset (not bounded chunk_data) because DBA controller
    // offsets can reference keyframe data that extends past the IVO chunk boundary.
    let data_bytes = &ivo.file_data()[db_data.offset as usize..];
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
        // All controller offsets are relative to the start of the controller entry itself.
        // (confirmed via cgf-converter: controllerStart + ctrl.RotDataOffset)
        let base = *ctrl_offset;

        let rotations = if ctrl.num_rot_keys > 0 {
            let times = if ctrl.rot_time_offset > 0 {
                read_time_keys(data, base + ctrl.rot_time_offset as usize, ctrl.num_rot_keys as usize, ctrl.rot_format_flags)?
            } else {
                (0..ctrl.num_rot_keys as usize).map(|t| t as f32).collect()
            };
            let values = read_rotation_keys(data, base + ctrl.rot_data_offset as usize, ctrl.num_rot_keys as usize, ctrl.rot_format_flags)?;
            times.into_iter().zip(values).map(|(t, v)| Keyframe { time: t, value: v }).collect()
        } else {
            Vec::new()
        };

        let positions = if ctrl.num_pos_keys > 0 {
            let times = if ctrl.pos_time_offset > 0 {
                read_time_keys(data, base + ctrl.pos_time_offset as usize, ctrl.num_pos_keys as usize, ctrl.pos_format_flags)?
            } else {
                (0..ctrl.num_pos_keys as usize).map(|t| t as f32).collect()
            };
            let values = read_position_keys(data, base + ctrl.pos_data_offset as usize, ctrl.num_pos_keys as usize, ctrl.pos_format_flags)?;
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
/// Entry layout (0x30 = 48 bytes each):
/// ```text
/// +0x00: flags (u32)
/// +0x04: unknown (u32)
/// +0x08: unknown (u32)
/// +0x0C: fps (u16)
/// +0x0E: num_controllers (u16)
/// +0x10: unknown (u32)
/// +0x14: unknown (u32)
/// +0x18: end_frame (u32)
/// +0x1C: start_rotation (quat, 16 bytes)
/// +0x2C: start_position (vec3, 12 bytes)
/// ```
fn parse_dba_metadata(data: &[u8]) -> Vec<(String, DbaMetaEntry)> {
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let entry_size = 48; // 0x30
    let entries_end = 4 + count * entry_size;
    if entries_end > data.len() {
        log::warn!("DBA metadata: {} entries × {} bytes = {} exceeds chunk size {}",
            count, entry_size, entries_end, data.len());
        return Vec::new();
    }

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let o = 4 + i * entry_size;
        let entry = DbaMetaEntry {
            fps: u16::from_le_bytes(data[o + 8..o + 10].try_into().unwrap()),
            _num_controllers: u16::from_le_bytes(data[o + 10..o + 12].try_into().unwrap()),
            _end_frame: u32::from_le_bytes(data[o + 20..o + 24].try_into().unwrap()),
            _start_rotation: [
                f32::from_le_bytes(data[o + 24..o + 28].try_into().unwrap()),
                f32::from_le_bytes(data[o + 28..o + 32].try_into().unwrap()),
                f32::from_le_bytes(data[o + 32..o + 36].try_into().unwrap()),
                f32::from_le_bytes(data[o + 36..o + 40].try_into().unwrap()),
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
    fps: u16,
}

fn parse_anim_info(data: &[u8]) -> AnimInfo {
    AnimInfo {
        fps: u16::from_le_bytes(data[4..6].try_into().unwrap_or([0; 2])),
    }
}

// ─── Time key reading ───────────────────────────────────────────────────────

fn read_time_keys(data: &[u8], offset: usize, count: usize, format_flags: u16) -> Result<Vec<f32>, Error> {
    // cgf-converter: GetTimeFormat extracts low nibble (& 0x0F), not low byte
    let time_format = format_flags & 0x0F;
    match time_format {
        // Format 0x00: byte array — each key is 1 byte, used directly as frame number
        // (covers format flags 0x8040 → 0x40 & 0x0F = 0x00)
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

fn read_rotation_keys(data: &[u8], offset: usize, count: usize, format_flags: u16) -> Result<Vec<[f32; 4]>, Error> {
    // High byte of format_flags determines rotation compression:
    //   0x80 = uncompressed quaternion (16 bytes per key)
    //   0x82 = SmallTree48BitQuat (6 bytes per key)
    // Low nibble determines time format (handled separately).
    let rot_format = format_flags >> 8;
    match rot_format {
        0x80 => read_uncompressed_quats(data, offset, count),
        0x82 => read_small_tree_48bit_quats(data, offset, count),
        _ => {
            log::warn!("Unknown rotation format 0x{rot_format:02x}, falling back to SmallTree48Bit");
            read_small_tree_48bit_quats(data, offset, count)
        }
    }
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

/// SmallTree48BitQuat: 6 bytes (3 × u16) per quaternion.
fn read_small_tree_48bit_quats(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 4]>, Error> {
    let size = count * 6;
    if offset + size > data.len() {
        return Err(Error::Other(format!("SmallTree48BitQuat overflow at 0x{offset:x}")));
    }

    Ok((0..count).map(|i| {
        let o = offset + i * 6;
        let s0 = u16::from_le_bytes(data[o..o + 2].try_into().unwrap());
        let s1 = u16::from_le_bytes(data[o + 2..o + 4].try_into().unwrap());
        let s2 = u16::from_le_bytes(data[o + 4..o + 6].try_into().unwrap());
        decode_small_tree_quat_48(s0, s1, s2)
    }).collect())
}

/// Decode SmallTree48BitQuat from 3 × u16.
/// Bit layout (confirmed via Ghidra FUN_14659d660):
///   u16[0] bits 0-14: v0 (15 bits), bit 15: sign/carry for v1
///   u16[1] bits 0-13: v1 (combined with bit 15 of u16[0]), bits 14-15: partial v2
///   u16[2] bits 0-13: v2 (combined with bits 14-15 of u16[1]), bits 14-15: largest index
/// Scale: DAT_1487b1534 = 1/23170.0, offset: DAT_147a60420 = 1/sqrt(2)
/// Lookup table at DAT_1487a9900: [1,2,3, 0,2,3, 0,1,3, 0,1,2]
/// Output: [x, y, z, w] with reconstructed component at index position.
fn decode_small_tree_quat_48(s0: u16, s1: u16, s2: u16) -> [f32; 4] {
    const INV_SCALE: f32 = 1.0 / 23170.0;
    const RANGE: f32 = std::f32::consts::FRAC_1_SQRT_2;

    // 2-bit index from top of 3rd short
    let idx = (s2 >> 14) as usize;

    // Extract 3 × 15-bit values with cross-word boundaries (matching Ghidra exactly):
    // v0 = u16[0] & 0x7FFF
    let raw0 = (s0 & 0x7FFF) as f32 * INV_SCALE - RANGE;
    // v1 = (u16[1] * 2 - (i16(u16[0]) >> 15)) & 0x7FFF  (borrows sign bit from u16[0])
    let raw1 = ((s1 as u32).wrapping_mul(2).wrapping_sub((s0 as i16 >> 15) as u32) & 0x7FFF) as f32 * INV_SCALE - RANGE;
    // v2 = ((u16[1] >> 14) + i16(u16[2]) * 4) & 0x7FFF
    let raw2 = ((s1 >> 14) as u32).wrapping_add((s2 as i16 as i32 as u32).wrapping_mul(4));
    let raw2 = (raw2 & 0x7FFF) as f32 * INV_SCALE - RANGE;

    // Reconstruct the largest component
    let w_sq = 1.0 - raw0 * raw0 - raw1 * raw1 - raw2 * raw2;
    let largest = if w_sq > 0.0 { w_sq.sqrt() } else { 0.0 };

    // Placement via lookup table: idx determines where reconstructed goes
    // Table: [1,2,3, 0,2,3, 0,1,3, 0,1,2]
    const TABLE: [[u8; 3]; 4] = [[1,2,3], [0,2,3], [0,1,3], [0,1,2]];
    let slots = TABLE[idx];
    let mut q = [0.0f32; 4];
    q[slots[0] as usize] = raw0;
    q[slots[1] as usize] = raw1;
    q[slots[2] as usize] = raw2;
    q[idx] = largest;
    q
}

// ─── Position key reading ───────────────────────────────────────────────────

fn read_position_keys(data: &[u8], offset: usize, count: usize, format_flags: u16) -> Result<Vec<[f32; 3]>, Error> {
    let pos_format = format_flags >> 8;
    log::trace!("pos_format=0x{pos_format:02X} flags=0x{format_flags:04X} count={count}");
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

/// SNORM full positions: 24-byte header (scale Vec3 + offset Vec3), then 6 bytes per key (u16 × 3).
/// Confirmed via Ghidra: `value = (float)(u16) * scale + offset`
fn read_snorm_full_positions(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 3]>, Error> {
    if offset + 24 + count * 6 > data.len() {
        return Err(Error::Other(format!("SNORM full positions overflow at 0x{offset:x}")));
    }
    let scale = read_vec3(data, offset);
    let pos_offset = read_vec3(data, offset + 12);
    log::trace!("SNORM_full: scale=[{:.6},{:.6},{:.6}] offset=[{:.4},{:.4},{:.4}] count={count}",
        scale[0], scale[1], scale[2], pos_offset[0], pos_offset[1], pos_offset[2]);

    Ok((0..count).map(|i| {
        let o = offset + 24 + i * 6;
        let ux = u16::from_le_bytes(data[o..o + 2].try_into().unwrap());
        let uy = u16::from_le_bytes(data[o + 2..o + 4].try_into().unwrap());
        let uz = u16::from_le_bytes(data[o + 4..o + 6].try_into().unwrap());
        [
            ux as f32 * scale[0] + pos_offset[0],
            uy as f32 * scale[1] + pos_offset[1],
            uz as f32 * scale[2] + pos_offset[2],
        ]
    }).collect())
}

/// SNORM packed positions: 24-byte header (scale Vec3 + offset Vec3) + variable u16 per key.
/// Inactive channels have scale == FLT_MAX; their value is offset directly.
/// Confirmed via Ghidra: `value = (float)(u16) * scale + offset`
fn read_snorm_packed_positions(data: &[u8], offset: usize, count: usize) -> Result<Vec<[f32; 3]>, Error> {
    if offset + 24 > data.len() {
        return Err(Error::Other(format!("SNORM packed header overflow at 0x{offset:x}")));
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
        return Err(Error::Other(format!("SNORM packed positions overflow at 0x{offset:x}")));
    }

    Ok((0..count).map(|i| {
        let o = data_start + i * bytes_per_key;
        let mut pos = pos_offset; // inactive channels get offset directly
        let mut byte_offset = 0;
        for ch in 0..3 {
            if active[ch] {
                let uv = u16::from_le_bytes(data[o + byte_offset..o + byte_offset + 2].try_into().unwrap());
                pos[ch] = uv as f32 * scale[ch] + pos_offset[ch];
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
