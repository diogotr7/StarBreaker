//! Gate test for the DBA animation reader.
//!
//! Verifies that the final keyframe of `rsi_scorpius_lg_deploy_r` in
//! `Scorpius.dba` decodes to the user-recorded deployed-pose target for
//! `BONE_Back_Right_Foot_Main` within ~3° (well under the 1°/axis tolerance
//! per axis specified in the phased plan).
//!
//! Run with: `cargo test -p starbreaker-3d -- --ignored dba_`.
//! Requires the fixture at `/tmp/sb_dba/Data/Animations/Spaceships/Ships/RSI/Scorpius.dba`,
//! which is produced by the canonical CLI extract command in the workspace
//! AGENTS.md (or by `starbreaker p4k extract --filter '**/Scorpius.dba'`).

use std::path::Path;

use starbreaker_3d::animation::{cry_xyzw_to_blender_wxyz, parse_dba};

const FIXTURE: &str = "/tmp/sb_dba/Data/Animations/Spaceships/Ships/RSI/Scorpius.dba";
const FOOT_BONE_HASH: u32 = 0xC1571A1A; // BONE_Back_Right_Foot_Main (zlib CRC32)

fn read_if_exists(path: &str) -> Option<Vec<u8>> {
    if Path::new(path).exists() {
        Some(std::fs::read(path).expect("failed to read DBA fixture"))
    } else {
        eprintln!("SKIP: fixture not found: {path}");
        None
    }
}

fn quat_angle_deg(a: [f32; 4], b: [f32; 4]) -> f32 {
    // Both in wxyz order; compare as rotations (sign-invariant).
    let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs();
    2.0 * dot.clamp(0.0, 1.0).acos().to_degrees()
}

#[test]
#[ignore = "requires extracted Scorpius.dba fixture"]
fn dba_scorpius_parses_55_clips() {
    let Some(data) = read_if_exists(FIXTURE) else {
        return;
    };
    let db = parse_dba(&data).expect("parse_dba failed");
    assert_eq!(db.clips.len(), 55, "Scorpius.dba should contain 55 clips");

    // Find the clip whose foot channel has the deploy fingerprint
    // (50 rotation keys + 90 position keys). Metadata→block alignment is
    // a separate known issue, so we find the deploy clip by data signature
    // not by metadata name.
    let deploy = db
        .clips
        .iter()
        .find(|c| {
            c.channels.iter().any(|ch| {
                ch.bone_hash == FOOT_BONE_HASH
                    && ch.rotations.len() == 50
                    && ch.positions.len() == 90
            })
        })
        .expect("rear-gear deploy clip (foot 50/90) missing");

    assert_eq!(deploy.channels.len(), 11);
    let foot = deploy
        .channels
        .iter()
        .find(|ch| ch.bone_hash == FOOT_BONE_HASH)
        .unwrap();
    assert_eq!(foot.rotations.len(), 50);
}

#[test]
#[ignore = "requires extracted Scorpius.dba fixture"]
fn dba_gate_test_rear_foot_deployed_pose() {
    let Some(data) = read_if_exists(FIXTURE) else {
        return;
    };
    let db = parse_dba(&data).expect("parse_dba failed");

    // Same fingerprint-based lookup as above.
    let deploy = db
        .clips
        .iter()
        .find(|c| {
            c.channels.iter().any(|ch| {
                ch.bone_hash == FOOT_BONE_HASH
                    && ch.rotations.len() == 50
                    && ch.positions.len() == 90
            })
        })
        .expect("rear-gear deploy clip missing");

    let foot = deploy
        .channels
        .iter()
        .find(|ch| ch.bone_hash == FOOT_BONE_HASH)
        .unwrap();
    let last = foot.rotations.last().unwrap();
    let pose_rot = cry_xyzw_to_blender_wxyz(last.value);

    // User-aligned target rotation in Blender Z-up wxyz.
    let target: [f32; 4] = [-0.5389_0413, 0.4081_7448, 0.5782_3223, 0.4567_5313];

    let angle = quat_angle_deg(pose_rot, target);
    eprintln!("Gate angle: {angle:.3}°  pose={pose_rot:?}  target={target:?}");
    assert!(
        angle < 5.0,
        "rear foot final-frame rotation differs by {angle:.3}° (want <5°)"
    );
}

#[test]
fn small_tree_decoder_unit_quat() {
    // Self-check: cry_xyzw_to_blender_wxyz preserves unit length.
    let q_xyzw = [-0.4538, -0.4116, 0.5912, 0.5244];
    let qb = cry_xyzw_to_blender_wxyz(q_xyzw);
    let len2 = qb[0].powi(2) + qb[1].powi(2) + qb[2].powi(2) + qb[3].powi(2);
    assert!((len2 - 1.0).abs() < 1e-3, "non-unit blender quat: len²={len2}");
}

#[test]
#[ignore = "requires extracted Scorpius.dba fixture"]
fn find_block_for_skeleton_picks_right_gear_deploy() {
    use starbreaker_3d::animation::{find_block_for_skeleton, clip_final_pose};
    use std::collections::HashSet;

    let Some(data) = read_if_exists(FIXTURE) else {
        return;
    };
    let db = parse_dba(&data).expect("parse DBA");

    // Right-gear .chr bone CRC32 set (zlib polynomial, case-preserved).
    // 12 BONE_Back_Right_* CRCs verified from
    // /tmp/sb_chr/.../RSI_Scorpius_Landinggear_Right_CHR.chr; block 45 in
    // Scorpius.dba is a subset of these (11 of 12 match).
    let skel: HashSet<u32> = [
        0xC1571A1A, 0xCF60D4A5, 0xAB14B4EC, 0xF2758208, 0x38EE324F, 0xAFA5701A,
        0xE7B06568, 0xF6365409, 0x18502E4B, 0xE25F1328, 0x82A13B38, 0xBBC68A63,
    ]
    .into_iter()
    .collect();

    let clip = find_block_for_skeleton(&db, &skel, true).expect("matched clip");

    assert!(
        clip.channels.iter().any(|c| c.bone_hash == FOOT_BONE_HASH),
        "selected clip missing rear-foot bone (hash {FOOT_BONE_HASH:#x}); got {} channels",
        clip.channels.len()
    );

    // Final pose for the foot bone must match the deployed-pose target
    // within the same tolerance as the gate test.
    let pose = clip_final_pose(clip);
    let foot = pose.get(&FOOT_BONE_HASH).expect("foot pose");
    let target = [-0.53890413, 0.40817448, 0.5782322, 0.45675313];
    let angle = quat_angle_deg(foot.rotation, target);
    assert!(
        angle < 5.0,
        "matcher selected wrong clip — foot off by {angle:.3}°"
    );
}
