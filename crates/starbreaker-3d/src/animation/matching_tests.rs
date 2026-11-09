//! Synthetic regression tests for CAF event matching and scoring.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::caf_anchored_remap;
use super::tokenize_for_match;
use crate::animation::{AnimationClip, AnimationDatabase};
use crate::chrparams::ChrParams;

#[test]
fn tokenize_for_match_keeps_ship_specific_tokens() {
    let tokens = tokenize_for_match("RSI Scorpius wings deploy");
    assert!(tokens.iter().any(|token| token == "rsi"));
    assert!(tokens.iter().any(|token| token == "scorpius"));
}

#[test]
fn tokenize_for_match_still_removes_generic_stopwords() {
    let tokens = tokenize_for_match("Animations ships component wings");
    assert!(!tokens.iter().any(|token| token == "animations"));
    assert!(!tokens.iter().any(|token| token == "ships"));
    assert!(!tokens.iter().any(|token| token == "component"));
    assert!(tokens.iter().any(|token| token == "wings"));
}

#[test]
fn caf_anchored_remap_uses_unmatched_duplicate_name_slots() {
    let db = AnimationDatabase {
        clips: vec![
            AnimationClip {
                name: "animations/shared_clip.caf".into(),
                fps: 30.0,
                channels: vec![],
                start_rotation: None,
                start_position: None,
            },
            AnimationClip {
                name: "animations/shared_clip.caf".into(),
                fps: 30.0,
                channels: vec![],
                start_rotation: None,
                start_position: None,
            },
        ],
    };

    let chrparams = ChrParams {
        animations: BTreeMap::from([
            (
                "event_a".to_string(),
                "animations/shared_clip.caf".to_string(),
            ),
            (
                "event_b".to_string(),
                "animations/shared_clip.caf".to_string(),
            ),
        ]),
        animation_paths: BTreeMap::new(),
        ..Default::default()
    };

    let remapped = caf_anchored_remap(
        &db,
        &chrparams,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        false,
        false,
    );

    assert_eq!(remapped.len(), 2);
    assert!(remapped.iter().any(|clip| clip.name == "event_a"));
    assert!(remapped.iter().any(|clip| clip.name == "event_b"));
}

#[test]
fn caf_anchored_remap_matches_filename_when_metadata_omits_prefix() {
    let db = AnimationDatabase {
        clips: vec![AnimationClip {
            name: "shared_clip.caf".into(),
            fps: 30.0,
            channels: vec![],
            start_rotation: None,
            start_position: None,
        }],
    };

    let chrparams = ChrParams {
        animations: BTreeMap::from([(
            "event_prefixed".to_string(),
            "Animations/shared_clip.caf".to_string(),
        )]),
        animation_paths: BTreeMap::new(),
        ..Default::default()
    };

    let remapped = caf_anchored_remap(
        &db,
        &chrparams,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        false,
        false,
    );

    assert_eq!(remapped.len(), 1);
    assert_eq!(remapped[0].name, "event_prefixed");
}

/// When two DBA blocks have identical semantic scores (no path hint resolves
/// either), the tiebreaker picks the block with the *higher* motion score
/// (more actual joint movement).  This test pins that behaviour so a
/// future change to the tiebreak direction is caught immediately.
#[test]
fn caf_anchored_remap_equal_semantic_score_tiebreaks_on_higher_motion() {
    use crate::animation::{BoneChannel, Keyframe};

    let bone_hash = 0xC1571A1A_u32; // BONE_Back_Right_Foot_Main
    let skeleton_hashes: HashSet<u32> = [bone_hash].into_iter().collect();
    let mut skeleton_names: HashMap<u32, String> = HashMap::new();
    skeleton_names.insert(bone_hash, "bone_back_right_foot_main".to_string());

    // Block 0: minimal rotation delta (near-zero motion score).
    let still_clip = AnimationClip {
        name: "animations/gear.caf".into(),
        fps: 30.0,
        channels: vec![BoneChannel {
            bone_hash,
            rotations: vec![
                Keyframe {
                    time: 0.0,
                    value: [1.0, 0.0, 0.0, 0.0],
                },
                Keyframe {
                    time: 1.0,
                    value: [1.0, 0.0, 0.0, 0.0],
                },
            ],
            positions: vec![],
            rot_format_flags: 0,
            pos_format_flags: 0,
        }],
        start_rotation: None,
        start_position: None,
    };

    // Block 1: 180° rotation delta (high motion score).
    let moving_clip = AnimationClip {
        name: "animations/gear.caf".into(),
        fps: 30.0,
        channels: vec![BoneChannel {
            bone_hash,
            rotations: vec![
                Keyframe {
                    time: 0.0,
                    value: [1.0, 0.0, 0.0, 0.0],
                },
                Keyframe {
                    time: 1.0,
                    value: [0.0, 1.0, 0.0, 0.0],
                },
            ],
            positions: vec![],
            rot_format_flags: 0,
            pos_format_flags: 0,
        }],
        start_rotation: None,
        start_position: None,
    };

    let db = AnimationDatabase {
        clips: vec![still_clip, moving_clip.clone()],
    };

    // CAF path does NOT match either block by name — path lookup fails,
    // leaving only semantic scoring.  Both blocks have equal token overlap
    // (same bone, caf stem "unknown_anim" has no name tokens in common),
    // so motion score is the sole tiebreaker.
    let chrparams = ChrParams {
        animations: BTreeMap::from([(
            "gear_retract".to_string(),
            "Animations/unknown_anim.caf".to_string(),
        )]),
        animation_paths: BTreeMap::new(),
        ..Default::default()
    };

    let remapped = caf_anchored_remap(
        &db,
        &chrparams,
        &skeleton_hashes,
        &skeleton_names,
        &HashMap::new(),
        false,
        false,
    );

    assert_eq!(remapped.len(), 1, "expected exactly one clip matched");
    // Block 1 (moving_clip) has higher motion — must be chosen.
    assert_eq!(
        remapped[0].channels[0].rotations[1].value,
        [0.0_f32, 1.0, 0.0, 0.0],
        "tiebreaker should select the block with higher motion score"
    );
}
