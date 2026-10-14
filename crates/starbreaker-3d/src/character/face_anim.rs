//! `.caf` face-animation decoder + appearance-weight extractor.
//!
//! Reads a CryAnimation `.caf` (Crytek Animation File) face-anim payload and
//! produces an appearance-control activation vector usable by
//! [`crate::character::controls::ControlInputs`] and the DNA-driven glb
//! pipeline.
//!
//! ## On-disk layout (reverse-engineered from the engine: `CFaceRigLogic::
//! ## ExecuteHuman` = `FUN_146e19c40`, loader `FUN_146caee20`, SC 4.8)
//!
//! The `.caf` is an IVO chunk container; the data chunk starts with `#caf`:
//!
//! ```text
//! +0x00  4   magic "#caf"
//! +0x04  u16 num_tracks       (controller nodes; subset of the rig's controls)
//! +0x06  u16 flags            (observed 0xFFFF)
//! +0x08  u32 size             (= chunk_size - 6)
//! +0x0C  u32[num_tracks]      sorted CRC32-IEEE hashes of control names
//! +HASH  Node[num_tracks]     24-byte controller nodes (zero-copy; the loader
//!                             returns this array directly)
//! +NODES ... key-time + key-value data referenced by node offsets ...
//! ```
//!
//! Each 24-byte node is a pair of 12-byte sub-controllers; the **active value
//! controller is the second half** (the first half, value-format `0x82`, is a
//! separate/unused channel that decodes to zero for face value tracks):
//!
//! ```text
//! +0x0c u16  key_count
//! +0x0e u8   time_format   '@'(0x40)=u8 frames, 'A'(0x41)=u16 frames,
//!                          'B'(0x42)=bitstream (set bit per keyed frame)
//! +0x0f u8   value_format  0xC0=f32x3 direct, 0xC1=u16x3*scale+bias,
//!                          0xC2=u16*scale+bias
//! +0x10 u32  time_offset   (relative to the node's own start)
//! +0x14 u32  value_offset  (relative to the node's own start)
//! ```
//!
//! The value block (at `node + value_offset`) is a 0x18-byte header followed by
//! the packed values: `scale = f32@[+0]`, `bias = f32@[+0xc]`, packed u16/f32
//! values at `+0x18`. Decoded value: `value[k] = u16[k] * scale + bias`
//! (`0xC2`), `u16[k*3] * scale + bias` (`0xC1`), or `f32[k*3]` (`0xC0`).
//!
//! The 'B' time block is `[u16 start][u16 end][u16 bitstream...]`: each set bit
//! at position `p` marks a keyframe at frame `start + p`; the k-th set bit is
//! key k's frame.
//!
//! The engine samples each controller by binary-searching the key times for the
//! requested frame, linearly interpolating the bracketing values, and
//! accumulating across clips scaled by the per-clip weight
//! (`FaceAnimDef + 12*clip + 8`): `_Dst[ctrl] += lerp(v[k], v[k+1], frac) *
//! clip_weight`. [`CafClip`] decodes one clip; the clip weight is applied
//! downstream (the builder's `intensity`).
//!
//! Key times are stored as absolute frames; [`CafClip`] normalizes them to a
//! `[0, 1]` loop using the clip's longest track as the length. `CTL_PSD`
//! (control 0) is forced to 1.0 (Pass 9 baseline); controls absent from the CAF
//! stay 0.0. CAF tracks not present in the rig's Section A are skipped.

use std::collections::HashMap;

use thiserror::Error;

use crate::riglogic::RigLogic;

const MAGIC: &[u8; 4] = b"#caf";
const HEADER_LEN: usize = 12;
/// Controller-node stride (two 12-byte sub-controllers).
const NODE_STRIDE: usize = 0x18;

#[inline]
fn rd_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[inline]
fn rd_f32(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Errors from [`CafClip::decode`] / [`load_caf_chunk_from_ivo`].
#[derive(Debug, Error)]
pub enum FaceAnimError {
    #[error("CAF too small: {got} bytes, need at least {min}")]
    Truncated { got: usize, min: usize },
    #[error("invalid #caf magic: got {got:?}, expected {expected:?}")]
    BadMagic { got: [u8; 4], expected: [u8; 4] },
    #[error("hash table out of bounds: num_tracks={num_tracks}, payload_len={payload_len}")]
    HashTableOob { num_tracks: usize, payload_len: usize },
    #[error("node array out of bounds: num_tracks={num_tracks}, payload_len={payload_len}")]
    IndexOob { num_tracks: usize, payload_len: usize },
}

/// One decoded appearance-control track: keyframe values plus their normalized
/// `[0, 1]` times (same length). Empty = control absent from the clip.
#[derive(Default)]
struct ControlTrack {
    times: Vec<f32>,
    values: Vec<f32>,
}

/// One decoded face-animation clip: per appearance-control keyframe arrays plus
/// a timeline sampler. Built by [`CafClip::decode`]; sampled with
/// [`CafClip::sample`].
pub struct CafClip {
    /// Indexed by appearance-control index (len = num_appearance_blendshapes).
    tracks: Vec<ControlTrack>,
    /// Largest keyframe count across resolved tracks (informational).
    pub num_keys: usize,
    /// Clip length in frames (the longest track's last key time). Lets callers
    /// play the loop at its authored speed (CryEngine face anims are 30 fps).
    pub frame_len: u32,
    pub tracks_resolved: usize,
    pub tracks_total: usize,
}

/// Decode one controller node's `(absolute_times, values)`. Returns `None` if
/// the node's offsets/format are out of bounds or produce non-finite values
/// (the engine only processes well-formed tracks).
fn decode_controller(
    chunk: &[u8],
    node_off: usize,
    count: usize,
    tfmt: u8,
    vfmt: u8,
    toff: usize,
    voff: usize,
) -> Option<(Vec<u32>, Vec<f32>)> {
    let valptr = node_off.checked_add(voff)?;
    let timeptr = node_off.checked_add(toff)?;

    // Values: 0xC0 = f32 triples (no scale/bias header); 0xC1/0xC2 = packed u16
    // (stride 3 / 1) after a 0x18-byte scale/bias header.
    let (scale, bias) = match vfmt {
        0xC0 => (1.0f32, 0.0f32),
        0xC1 | 0xC2 => {
            if valptr + 0x18 > chunk.len() {
                return None;
            }
            let s = rd_f32(chunk, valptr);
            let b = rd_f32(chunk, valptr + 12);
            if !s.is_finite() || !b.is_finite() {
                return None;
            }
            (s, b)
        }
        _ => return None,
    };
    let mut values = Vec::with_capacity(count);
    for k in 0..count {
        let v = match vfmt {
            0xC0 => {
                let o = valptr + k * 12;
                if o + 4 > chunk.len() {
                    return None;
                }
                rd_f32(chunk, o)
            }
            0xC1 => {
                let o = valptr + 0x18 + k * 6;
                if o + 2 > chunk.len() {
                    return None;
                }
                rd_u16(chunk, o) as f32 * scale + bias
            }
            0xC2 => {
                let o = valptr + 0x18 + k * 2;
                if o + 2 > chunk.len() {
                    return None;
                }
                rd_u16(chunk, o) as f32 * scale + bias
            }
            _ => return None,
        };
        if !v.is_finite() {
            return None;
        }
        values.push(v);
    }

    // Key times (absolute frames).
    let mut times = Vec::with_capacity(count);
    match tfmt {
        0x40 => {
            if timeptr + count > chunk.len() {
                return None;
            }
            for k in 0..count {
                times.push(chunk[timeptr + k] as u32);
            }
        }
        0x41 => {
            if timeptr + count * 2 > chunk.len() {
                return None;
            }
            for k in 0..count {
                times.push(rd_u16(chunk, timeptr + k * 2) as u32);
            }
        }
        0x42 => {
            if timeptr + 4 > chunk.len() {
                return None;
            }
            let start = rd_u16(chunk, timeptr) as u32;
            let bits = timeptr + 4;
            let mut p = 0usize;
            while times.len() < count {
                let wo = bits + (p / 16) * 2;
                if wo + 2 > chunk.len() {
                    return None;
                }
                if (rd_u16(chunk, wo) >> (p % 16)) & 1 == 1 {
                    times.push(start + p as u32);
                }
                p += 1;
            }
        }
        _ => return None,
    }
    Some((times, values))
}

/// Linear-interpolate a track at normalized time `t` (assumes `times` is
/// monotonic and `times.len() == values.len() >= 1`). Clamps outside the range.
fn sample_track(times: &[f32], values: &[f32], t: f32) -> f32 {
    if values.len() == 1 {
        return values[0];
    }
    if t <= times[0] {
        return values[0];
    }
    if t >= *times.last().unwrap() {
        return *values.last().unwrap();
    }
    let (mut lo, mut hi) = (0usize, times.len() - 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if times[mid] <= t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let span = times[hi] - times[lo];
    let frac = if span > 0.0 { (t - times[lo]) / span } else { 0.0 };
    values[lo] + (values[hi] - values[lo]) * frac
}

impl CafClip {
    /// Decode the `#caf` chunk payload against `rig`'s appearance controls.
    pub fn decode(chunk: &[u8], rig: &RigLogic) -> Result<Self, FaceAnimError> {
        if chunk.len() < HEADER_LEN {
            return Err(FaceAnimError::Truncated { got: chunk.len(), min: HEADER_LEN });
        }
        if &chunk[..4] != MAGIC {
            let mut got = [0u8; 4];
            got.copy_from_slice(&chunk[..4]);
            return Err(FaceAnimError::BadMagic { got, expected: *MAGIC });
        }
        let num_tracks = rd_u16(chunk, 4) as usize;
        let hash_end = HEADER_LEN + num_tracks * 4;
        if hash_end > chunk.len() {
            return Err(FaceAnimError::HashTableOob { num_tracks, payload_len: chunk.len() });
        }
        let node_base = hash_end;
        if node_base + num_tracks * NODE_STRIDE > chunk.len() {
            return Err(FaceAnimError::IndexOob { num_tracks, payload_len: chunk.len() });
        }

        let mut hash_to_idx: HashMap<u32, usize> =
            HashMap::with_capacity(rig.joint_control_names.len());
        for (i, name) in rig.joint_control_names.iter().enumerate() {
            hash_to_idx.insert(crc32fast::hash(name.as_bytes()), i);
        }
        let num_appearance = rig.num_appearance_blendshapes as usize;
        let mut tracks: Vec<ControlTrack> =
            (0..num_appearance).map(|_| ControlTrack::default()).collect();
        let mut tracks_resolved = 0usize;
        let mut num_keys = 0usize;

        // Decode each resolved track to (control_idx, abs_times, values), and
        // track the clip's frame length (longest last-key time).
        let mut pending: Vec<(usize, Vec<u32>, Vec<f32>)> = Vec::new();
        let mut clip_len: u32 = 0;
        for n in 0..num_tracks {
            let hash = rd_u32(chunk, HEADER_LEN + n * 4);
            let Some(&idx) = hash_to_idx.get(&hash) else { continue };
            tracks_resolved += 1;
            if idx == 0 {
                continue; // CTL_PSD is forced to 1.0 in `sample`.
            }
            let no = node_base + n * NODE_STRIDE;
            let count = rd_u16(chunk, no + 0x0c) as usize;
            let tfmt = chunk[no + 0x0e];
            let vfmt = chunk[no + 0x0f];
            if count == 0
                || !matches!(tfmt, 0x40 | 0x41 | 0x42)
                || !matches!(vfmt, 0xC0 | 0xC1 | 0xC2)
            {
                continue;
            }
            let toff = rd_u32(chunk, no + 0x10) as usize;
            let voff = rd_u32(chunk, no + 0x14) as usize;
            let Some((times, values)) = decode_controller(chunk, no, count, tfmt, vfmt, toff, voff)
            else {
                continue;
            };
            if let Some(&last) = times.last() {
                clip_len = clip_len.max(last);
            }
            num_keys = num_keys.max(values.len());
            pending.push((idx, times, values));
        }

        let denom = if clip_len == 0 { 1.0 } else { clip_len as f32 };
        for (idx, abs_times, values) in pending {
            let times = abs_times.iter().map(|&t| t as f32 / denom).collect();
            tracks[idx] = ControlTrack { times, values };
        }

        Ok(CafClip { tracks, num_keys, frame_len: clip_len, tracks_resolved, tracks_total: num_tracks })
    }

    /// Appearance-control weight vector at normalized time `t` (clamped to
    /// `[0, 1]`). Index 0 (CTL_PSD) is always 1.0; absent controls 0.0.
    pub fn sample(&self, t: f32) -> Vec<f32> {
        let t = t.clamp(0.0, 1.0);
        let mut out = vec![0.0f32; self.tracks.len()];
        if !out.is_empty() {
            out[0] = 1.0;
        }
        for (c, tr) in self.tracks.iter().enumerate() {
            if c == 0 || tr.values.is_empty() {
                continue;
            }
            out[c] = sample_track(&tr.times, &tr.values, t);
        }
        out
    }

    #[cfg(test)]
    fn track_values(&self, idx: usize) -> &[f32] {
        &self.tracks[idx].values
    }
}

/// Extract the `#caf` chunk data from an IVO container (`.caf` file bytes).
/// Returns the raw chunk data slice (starts with `#caf` magic).
pub fn load_caf_chunk_from_ivo(ivo_bytes: &[u8]) -> Result<&[u8], FaceAnimError> {
    use starbreaker_chunks::ChunkFile;
    let cf = ChunkFile::from_bytes(ivo_bytes)
        .map_err(|_| FaceAnimError::Truncated { got: ivo_bytes.len(), min: 32 })?;
    let ivo = match cf {
        ChunkFile::Ivo(i) => i,
        _ => return Err(FaceAnimError::BadMagic { got: [0; 4], expected: *MAGIC }),
    };
    for entry in ivo.chunks() {
        let data = ivo.chunk_data(entry);
        if data.len() >= 4 && &data[..4] == MAGIC {
            let start = entry.offset as usize;
            let len = entry.size as usize;
            if start + len > ivo_bytes.len() {
                return Err(FaceAnimError::Truncated { got: ivo_bytes.len(), min: start + len });
            }
            return Ok(&ivo_bytes[start..start + len]);
        }
    }
    Err(FaceAnimError::BadMagic { got: [0; 4], expected: *MAGIC })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riglogic::{BlendShapeBlob, RigLogic, RigType};
    use starbreaker_common::NameHash;

    fn synthetic_rig(names: Vec<&str>) -> RigLogic {
        let num_appearance = names.len() as u16;
        RigLogic {
            file_size: 0,
            name: "synthetic".into(),
            rig_type: RigType { class: 0, gender: 0, variant: 0, part: 0, tier: 0, lod: 0, flags_or_pad: 0 },
            identity_crc_lo: NameHash(0),
            identity_crc_hi_or_x: 0,
            header_size: 0xA8,
            num_appearance_blendshapes: num_appearance,
            num_corrective_blendshapes: 0,
            num_joints: 0,
            _reserved_0x46: 0,
            primary_delta_scale: 0.0,
            secondary_delta_scale: 0.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: 9,
            section_starts: [0; 8],
            joint_control_name_crcs: names.iter().map(|n| crc32fast::hash(n.as_bytes())).collect(),
            joint_control_names: names.into_iter().map(String::from).collect(),
            appearance: BlendShapeBlob { scale: 0.0, records: Vec::new() },
            corrective: BlendShapeBlob { scale: 0.0, records: Vec::new() },
            corrective_composition: Vec::new(),
            skeleton_bone_name_crcs: Vec::new(),
            blend_region_bones: Vec::new(),
            region_weights: Vec::new(),
        }
    }

    /// One synthetic track: control name, `'A'`-format u16 times, raw u16 values,
    /// and the `value = u16 * scale + bias` parameters.
    struct Track<'a> {
        name: &'a str,
        times: &'a [u16],
        raw: &'a [u16],
        scale: f32,
        bias: f32,
    }

    /// Build a valid `#caf` chunk in the real on-disk format: each node's active
    /// (second-half) controller uses time format `'A'` (u16) and value format
    /// `0xC2` (u16*scale+bias). The value block is a 0x18-byte header
    /// (scale@+0, bias@+0xc) followed by the u16 values at +0x18.
    fn synth_caf(tracks: &[Track]) -> Vec<u8> {
        let n = tracks.len();
        // Sort by hash (the header CRC table is sorted).
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| crc32fast::hash(tracks[i].name.as_bytes()));

        let node_base = HEADER_LEN + n * 4;
        let nodes_len = n * NODE_STRIDE;
        let data_off = node_base + nodes_len;

        // Lay out per-track time + value blocks in the data region.
        let mut data: Vec<u8> = Vec::new();
        // (data-relative time offset, data-relative value offset) per slot.
        let mut blocks: Vec<(usize, usize)> = Vec::with_capacity(n);
        for &i in &order {
            let t = &tracks[i];
            let time_pos = data.len();
            for &tm in t.times {
                data.extend_from_slice(&tm.to_le_bytes());
            }
            let val_pos = data.len();
            // 0x18 header: scale@0, bias@0xc, rest zero.
            let mut header = [0u8; 0x18];
            header[0..4].copy_from_slice(&t.scale.to_le_bytes());
            header[12..16].copy_from_slice(&t.bias.to_le_bytes());
            data.extend_from_slice(&header);
            for &v in t.raw {
                data.extend_from_slice(&v.to_le_bytes());
            }
            blocks.push((time_pos, val_pos));
        }

        let total = data_off + data.len();
        let mut buf = vec![0u8; total];
        buf[..4].copy_from_slice(MAGIC);
        buf[4..6].copy_from_slice(&(n as u16).to_le_bytes());
        buf[6..8].copy_from_slice(&0xFFFFu16.to_le_bytes());
        buf[8..12].copy_from_slice(&((total - 6) as u32).to_le_bytes());
        buf[data_off..].copy_from_slice(&data);

        for (slot, &i) in order.iter().enumerate() {
            let h = crc32fast::hash(tracks[i].name.as_bytes());
            buf[HEADER_LEN + slot * 4..HEADER_LEN + slot * 4 + 4].copy_from_slice(&h.to_le_bytes());
        }
        for (slot, (&i, &(time_pos, val_pos))) in order.iter().zip(&blocks).enumerate() {
            let no = node_base + slot * NODE_STRIDE;
            let count = tracks[i].raw.len() as u16;
            buf[no + 0x0c..no + 0x0e].copy_from_slice(&count.to_le_bytes());
            buf[no + 0x0e] = 0x41; // 'A' u16 times
            buf[no + 0x0f] = 0xC2; // u16*scale+bias
            // Offsets are relative to the node's own start.
            let toff = (data_off + time_pos - no) as u32;
            let voff = (data_off + val_pos - no) as u32;
            buf[no + 0x10..no + 0x14].copy_from_slice(&toff.to_le_bytes());
            buf[no + 0x14..no + 0x18].copy_from_slice(&voff.to_le_bytes());
        }
        buf
    }

    /// value = raw/65535 with scale=1/65535, bias=0 (matches the common ROM/idle
    /// encoding) — keeps test expectations in `[0, 1]`.
    fn norm_track<'a>(name: &'a str, times: &'a [u16], raw: &'a [u16]) -> Track<'a> {
        Track { name, times, raw, scale: 1.0 / 65535.0, bias: 0.0 }
    }

    #[test]
    fn decode_reads_values_via_scale_bias() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a", "CTL_b"]);
        let caf = synth_caf(&[
            norm_track("CTL_a", &[0, 1, 2, 3, 4], &[0, 16384, 32768, 49151, 65535]),
            norm_track("CTL_b", &[0, 1, 2, 3, 4], &[65535, 49151, 32768, 16384, 0]),
        ]);
        let clip = CafClip::decode(&caf, &rig).expect("decode");
        assert_eq!(clip.tracks_total, 2);
        assert_eq!(clip.tracks_resolved, 2);
        let a = clip.track_values(1);
        assert_eq!(a.len(), 5);
        assert!((a[0] - 0.0).abs() < 1e-4);
        assert!((a[2] - 0.5).abs() < 1e-3);
        assert!((a[4] - 1.0).abs() < 1e-4);
        let b = clip.track_values(2);
        assert!((b[0] - 1.0).abs() < 1e-4);
        assert!((b[4] - 0.0).abs() < 1e-4);
        assert!(clip.track_values(0).is_empty());
    }

    #[test]
    fn scale_bias_offsets_values() {
        // value = raw * 0.001 + 0.2: raw 0 -> 0.2, raw 1000 -> 1.2.
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a"]);
        let caf = synth_caf(&[Track {
            name: "CTL_a",
            times: &[0, 10],
            raw: &[0, 1000],
            scale: 0.001,
            bias: 0.2,
        }]);
        let clip = CafClip::decode(&caf, &rig).unwrap();
        let v = clip.track_values(1);
        assert!((v[0] - 0.2).abs() < 1e-5, "got {}", v[0]);
        assert!((v[1] - 1.2).abs() < 1e-5, "got {}", v[1]);
    }

    #[test]
    fn sample_interpolates_over_normalized_times() {
        // Non-uniform times: keys at frames 0, 1, 9, 10 (clip length 10).
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a"]);
        let caf = synth_caf(&[norm_track("CTL_a", &[0, 1, 9, 10], &[0, 0, 65535, 65535])]);
        let clip = CafClip::decode(&caf, &rig).unwrap();
        // CTL_PSD always 1.0.
        assert!((clip.sample(0.0)[0] - 1.0).abs() < 1e-6);
        // t=0 -> first key 0.0; t=1 -> last key 1.0.
        assert!((clip.sample(0.0)[1] - 0.0).abs() < 1e-4);
        assert!((clip.sample(1.0)[1] - 1.0).abs() < 1e-4);
        // Frame 5 is normalized t=0.5, halfway between frame-1 (0.0) and
        // frame-9 (1.0) -> 0.5.
        assert!((clip.sample(0.5)[1] - 0.5).abs() < 1e-2, "got {}", clip.sample(0.5)[1]);
        // Clamp outside [0,1].
        assert_eq!(clip.sample(-1.0)[1], clip.sample(0.0)[1]);
        assert_eq!(clip.sample(2.0)[1], clip.sample(1.0)[1]);
    }

    #[test]
    fn decode_first_frame_and_psd() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a"]);
        let caf = synth_caf(&[norm_track("CTL_a", &[0, 1], &[16384, 65535])]);
        let r = CafClip::decode(&caf, &rig).expect("ok");
        assert_eq!(r.tracks_total, 1);
        assert_eq!(r.tracks_resolved, 1);
        assert!((r.sample(0.0)[0] - 1.0).abs() < 1e-6); // CTL_PSD
        assert!((r.sample(0.0)[1] - (16384.0 / 65535.0)).abs() < 1e-4); // t=0 value
    }

    #[test]
    fn bad_magic_errors() {
        let rig = synthetic_rig(vec!["CTL_PSD"]);
        let mut bytes = synth_caf(&[]);
        if bytes.len() < 4 {
            bytes.resize(12, 0);
        }
        bytes[0] = b'X';
        let Err(err) = CafClip::decode(&bytes, &rig) else { panic!("bad magic must error") };
        assert!(matches!(err, FaceAnimError::BadMagic { .. }));
    }

    #[test]
    fn unmatched_tracks_are_skipped() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_known"]);
        let caf = synth_caf(&[norm_track("CTL_unknown", &[0, 1], &[65535, 65535])]);
        let r = CafClip::decode(&caf, &rig).expect("ok");
        assert_eq!(r.tracks_total, 1);
        assert_eq!(r.tracks_resolved, 0); // hash unmatched -> not resolved
        assert_eq!(r.sample(0.0), vec![1.0, 0.0]);
    }

    #[test]
    fn ctl_psd_never_overridden() {
        let rig = synthetic_rig(vec!["CTL_PSD"]);
        let caf = synth_caf(&[norm_track("CTL_PSD", &[0, 1], &[0, 0])]);
        let r = CafClip::decode(&caf, &rig).expect("ok");
        assert_eq!(r.sample(0.0), vec![1.0]);
    }
}
