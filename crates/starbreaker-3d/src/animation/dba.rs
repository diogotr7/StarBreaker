//! DBA (Animation Database) parsing.
//!
//! DBA files pack multiple animation clips; block-level parsing is shared
//! with CAF via [`super::caf::parse_animation_blocks`].

use super::caf::parse_animation_blocks;
use super::chunk_types;
use super::{AnimationClip, AnimationDatabase, BoneChannel};
use crate::error::Error;
use starbreaker_chunks::ChunkFile;

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
    let data_offset = db_data_chunk.offset as usize;
    let data_bytes = ivo
        .file_data()
        .get(data_offset..)
        .ok_or_else(|| Error::Other(format!("DBA data chunk offset out of range: {data_offset}")))?;
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
                start_rotation: None,
                start_position: None,
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
        let (name, fps, start_rotation, start_position) = match meta_entries.get(i) {
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
                // DBA only stores XY of the start position (the 48-byte
                // entry has no room for Z); expose the full XYZ here
                // with Z = 0.0 so consumers don't have to know the
                // on-disk quirk. See `DbaMetaEntry::start_position_xy`
                // and the layout comment in `parse_dba_metadata`.
                let start_pos = [meta.start_position_xy[0], meta.start_position_xy[1], 0.0_f32];
                (clip_name, fps, Some(meta.start_rotation), Some(start_pos))
            }
            None => (format!("anim_{i}"), 30.0, None, None),
        };
        clips.push(AnimationClip {
            name,
            fps,
            channels,
            start_rotation,
            start_position,
        });
    }
    clips
}

// ── DBA metadata parsing ────────────────────────────────────────────────────

fn parse_dba_metadata(data: &[u8]) -> Vec<(String, DbaMetaEntry)> {
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let entry_size = 48; // 0x30
    let entries_end = match count
        .checked_mul(entry_size)
        .and_then(|bytes| bytes.checked_add(4))
    {
        Some(end) => end,
        None => {
            log::warn!(
                "DBA metadata entry span overflow: count={} entry_size={}",
                count,
                entry_size
            );
            return Vec::new();
        }
    };
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

    // The names region starts at the next 8-byte boundary (entries end 4
    // bytes past one: 4 + 48·count). The padding is not always NUL (Scorpius
    // has 4 NULs, character_select_screen.dba `53 0C CC BC`), so align rather
    // than skip NULs; then skip any further NULs.
    let mut pos = entries_end.next_multiple_of(8);
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

#[cfg(test)]
mod tests {
    use super::parse_dba_metadata;

    #[test]
    fn parse_dba_metadata_returns_empty_when_entry_span_overflows() {
        let data = u32::MAX.to_le_bytes().to_vec();
        let entries = parse_dba_metadata(&data);
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_dba_metadata_returns_empty_when_entries_exceed_chunk_size() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes());
        data.resize(8, 0);

        let entries = parse_dba_metadata(&data);
        assert!(entries.is_empty());
    }
}
