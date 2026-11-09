//! Synthetic catalogue tests: source union, sparse decoding, timing, endpoints and namespace keys.
use super::*;
use std::collections::{HashMap, HashSet};
const DBA: &[u8] = include_bytes!("../../tests/fixtures/clip_catalogue/synthetic.dba");
const CAF: &[u8] = include_bytes!("../../tests/fixtures/clip_catalogue/synthetic.caf");
const PARAMS: &[u8] = include_bytes!("../../tests/fixtures/clip_catalogue/synthetic.chrparams");

#[test]
fn synthetic_catalogue_union_preserves_export_names_and_endpoints() {
    let params = crate::chrparams::ChrParams::from_bytes(PARAMS).unwrap();
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
