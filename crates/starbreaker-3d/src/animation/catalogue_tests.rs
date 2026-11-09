//! Synthetic catalogue tests: source union, sparse decoding, timing, endpoints and namespace keys.
use super::*;
use std::collections::{HashMap, HashSet};
const DBA: &[u8] = include_bytes!("../../tests/fixtures/clip_catalogue/synthetic.dba");
const CAF: &[u8] = include_bytes!("../../tests/fixtures/clip_catalogue/synthetic.caf");
const PARAMS: &[u8] = include_bytes!("../../tests/fixtures/clip_catalogue/synthetic.chrparams");

#[test]
fn synthetic_catalogue_union_preserves_export_names_and_endpoints() {
    let params = crate::chrparams::ChrParams::from_bytes(&PARAMS.to_vec()).unwrap();
    assert_eq!(
        params.resolved_caf_path("cycle.caf"),
        "animations/synthetic/cycle.caf"
    );
    // ChunkFile's zero-copy u32 reader needs aligned storage (as P4k reads provide).
    let dba_bytes = DBA.to_vec();
    let sparse = dba::parse_dba_mode(&dba_bytes, true).unwrap();
    let full = parse_dba(&dba_bytes).unwrap();
    let remap = |db: &AnimationDatabase| {
        matching::caf_anchored_remap(
            db,
            &params,
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            true,
            false,
        )
    };
    let sparse = remap(&sparse);
    let full = remap(&full);
    assert_eq!(sparse.len(), 2);
    assert_eq!(sparse[0].name, "latch_cycle");
    assert_eq!(sparse[1].name, "animations/synthetic/spare.caf");
    for (i, (s, f)) in sparse.iter().zip(full.iter()).enumerate() {
        assert_eq!(s.name, f.name);
        assert_eq!(s.channels[0].positions.len(), 2);
        assert_eq!(f.channels[0].positions.len(), 3);
        assert_eq!(
            s.channels[0].positions.last().unwrap().value,
            f.channels[0].positions.last().unwrap().value
        );
        let mut json = clip_to_json(s);
        assert_eq!(json["frame_count"], clip_to_json(f)["frame_count"]);
        json["catalogue_source"] = serde_json::json!(if i == 0 { "Chrparams" } else { "DbaOnly" });
        let summary = catalogue::summarize(&json, "synthetic_owner").unwrap();
        assert_eq!(summary.fps, 24);
        assert_eq!(summary.frame_count, 2);
        assert_eq!(summary.duration_s, 2.0 / 24.0);
        assert_eq!(summary.boomerang, i == 0);
        assert!(summary.has_rotation && summary.has_position);
        assert_eq!(summary.bone_count, 1);
    }
    let caf_bytes = CAF.to_vec();
    let caf = caf::parse_caf_mode(&caf_bytes, true).unwrap();
    assert_eq!(caf.clips.len(), 2);
    assert_eq!(caf.clips[0].fps, 24.0);
    assert_eq!(caf.clips[0].channels[0].positions.len(), 2);
}

#[test]
fn sparse_snorm_and_compressed_rotations_match_full_decoding() {
    // Invented scale/offset and three-key arrays; exercise both planar and interleaved encodings.
    for format in [0xC100, 0xC200] {
        let mut bytes = Vec::new();
        for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
            bytes.extend(v.to_le_bytes());
        }
        for v in 1u16..=9 {
            bytes.extend(v.to_le_bytes());
        }
        assert_eq!(
            endpoints::position_ends(&bytes, 0, 3, format).unwrap(),
            endpoints::ends(codec::read_position_keys(&bytes, 0, 3, format).unwrap())
        );
        assert!(endpoints::position_ends(&bytes[..25], 0, 3, format).is_err());
    }
    let bytes = [0u8; 18];
    assert_eq!(
        endpoints::rotation_ends(&bytes, 0, 3, 0x8200).unwrap(),
        endpoints::ends(codec::read_rotation_keys(&bytes, 0, 3, 0x8200).unwrap())
    );
}

#[test]
fn opposite_quaternion_signs_are_the_same_endpoint_pose() {
    let clip = serde_json::json!({"name":"cycle", "fps":24, "frame_count":2, "catalogue_source":"Caf", "bones": {
        "0x1": {"has_rotation":true,"has_position":false,"rotation":[[1,0,0,0],[-1,0,0,0]]}
    }});
    assert!(catalogue::summarize(&clip, "owner").unwrap().boomerang);
    assert_ne!(
        crate::decomposed::interior_clip_key("mechanism_a", "open"),
        crate::decomposed::interior_clip_key("mechanism_b", "open")
    );
    assert_eq!(
        crate::decomposed::humanize_animation_label("mechanism_a", "open"),
        "Mechanism A — Open"
    );
}

#[test]
fn unknown_position_format_keeps_dba_blocks_aligned() {
    let mut bytes = DBA.to_vec();
    let data_offset = match starbreaker_chunks::ChunkFile::from_bytes(&bytes).unwrap() {
        starbreaker_chunks::ChunkFile::Ivo(ivo) => ivo.chunks()[0].offset as usize,
        _ => unreachable!(),
    };
    // First block: total-size prefix + block header + hash + position-flags offset.
    let flags = data_offset + 4 + 12 + 4 + 14;
    bytes[flags..flags + 2].copy_from_slice(&0xFF00u16.to_le_bytes());
    let full = parse_dba(&bytes).unwrap();
    let sparse = dba::parse_dba_mode(&bytes, true).unwrap();
    assert_eq!(sparse.clips.len(), full.clips.len());
    for (s, f) in sparse.clips.iter().zip(&full.clips) {
        assert_eq!(s.name, f.name);
        assert_eq!(
            s.channels[0].positions.first().unwrap().value,
            f.channels[0].positions.first().unwrap().value
        );
        assert_eq!(
            s.channels[0].positions.last().unwrap().value,
            f.channels[0].positions.last().unwrap().value
        );
    }
}

#[test]
fn zero_caf_fps_uses_the_missing_info_default() {
    let mut bytes = CAF.to_vec();
    let info_offset = match starbreaker_chunks::ChunkFile::from_bytes(&bytes).unwrap() {
        starbreaker_chunks::ChunkFile::Ivo(ivo) => ivo.chunks()[1].offset as usize,
        _ => unreachable!(),
    };
    bytes[info_offset + 4..info_offset + 6].copy_from_slice(&0u16.to_le_bytes());
    for endpoints in [false, true] {
        let db = caf::parse_caf_mode(&bytes, endpoints).unwrap();
        assert_eq!(db.clips[0].fps, 30.0);
        let mut clip = clip_to_json(&db.clips[0]);
        clip["catalogue_source"] = serde_json::json!("Caf");
        assert!(catalogue::summarize(&clip, "synthetic").is_ok());
    }
}

#[test]
fn chrparams_wildcards_are_mapping_rules_not_clip_names() {
    for name in ["*", "door_*", "door_?"] {
        let mut bytes = PARAMS.to_vec();
        let start = bytes
            .windows(b"latch_cycle".len())
            .position(|w| w == b"latch_cycle")
            .unwrap();
        bytes[start..start + b"latch_cycle".len()].fill(0);
        bytes[start..start + name.len()].copy_from_slice(name.as_bytes());
        let params = crate::chrparams::ChrParams::from_bytes(&bytes).unwrap();
        assert!(params.animations.is_empty());
        assert!(params.animation_paths.is_empty());
    }
}

#[test]
fn sparse_planar_positions_skip_inactive_axes() {
    let mut bytes = Vec::new();
    for v in [f32::MAX, 2.0, 3.0, 7.0, 5.0, 6.0] {
        bytes.extend(v.to_le_bytes());
    }
    for v in [1u16, 4, 8, 2, 6, 9] {
        bytes.extend(v.to_le_bytes());
    }
    let sparse = endpoints::position_ends(&bytes, 0, 3, 0xC200).unwrap();
    assert_eq!(
        sparse,
        endpoints::ends(codec::read_position_keys(&bytes, 0, 3, 0xC200).unwrap())
    );
    assert_eq!(sparse[0][0], 7.0);
    assert_eq!(sparse[1][0], 7.0);
}

#[test]
fn sparse_block_uses_bitmap_frame_times() {
    // Three samples at frames 2, 5, 9. Header/controllers precede time/value pools.
    let mut block = b"#caf".to_vec();
    block.extend(1u16.to_le_bytes());
    block.extend(0xAA55u16.to_le_bytes());
    block.extend(40u32.to_le_bytes());
    block.extend(1u32.to_le_bytes());
    block.extend([0; 24]);
    let controller = 16;
    let times = block.len();
    block.extend(2u16.to_le_bytes());
    block.extend(9u16.to_le_bytes());
    block.push(0b10001001);
    let values = block.len();
    for x in [0.0f32, 5.0, 0.0] {
        for v in [x, 0.0, 0.0] {
            block.extend(v.to_le_bytes());
        }
    }
    block[controller + 12..controller + 14].copy_from_slice(&3u16.to_le_bytes());
    block[controller + 14..controller + 16].copy_from_slice(&0xC002u16.to_le_bytes());
    block[controller + 16..controller + 20]
        .copy_from_slice(&((times - controller) as u32).to_le_bytes());
    block[controller + 20..controller + 24]
        .copy_from_slice(&((values - controller) as u32).to_le_bytes());
    let sparse = caf::parse_animation_blocks_mode(&block, true).unwrap();
    assert_eq!(
        sparse[0][0]
            .positions
            .iter()
            .map(|k| k.time)
            .collect::<Vec<_>>(),
        vec![2.0, 9.0]
    );
    let full = caf::parse_animation_blocks_mode(&block, false).unwrap();
    assert_eq!(
        full[0][0]
            .positions
            .iter()
            .map(|k| k.time)
            .collect::<Vec<_>>(),
        vec![2.0, 5.0, 9.0]
    );
}
