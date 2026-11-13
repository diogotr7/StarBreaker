//! Synthetic regression tests for chrparams event → animation block pairing.

use std::collections::BTreeMap;

use super::{ClipOrigin, caf_anchored_remap, clip_for_caf_path, clips_for_caf_path};
use crate::animation::{AnimationClip, AnimationDatabase, BoneChannel, Keyframe, bone_name_hash};
use crate::chrparams::ChrParams;

/// A one-channel block whose single rotation key marks which block it is.
fn block(name: &str, bone: &str, marker: f32) -> AnimationClip {
    AnimationClip {
        name: name.into(),
        fps: 30.0,
        channels: vec![BoneChannel {
            bone_hash: bone_name_hash(bone),
            rotations: vec![
                Keyframe { time: 0.0, value: [0.0, 0.0, 0.0, 1.0] },
                Keyframe { time: marker, value: [0.0, 1.0, 0.0, 0.0] },
            ],
            positions: vec![],
            rot_format_flags: 0,
            pos_format_flags: 0,
        }],
        start_rotation: None,
        start_position: None,
    }
}

fn marker(clip: &AnimationClip) -> f32 {
    clip.channels[0].rotations[1].time
}

fn events(pairs: &[(&str, &str)]) -> ChrParams {
    ChrParams {
        animations: pairs
            .iter()
            .map(|(event, path)| (event.to_string(), path.to_string()))
            .collect::<BTreeMap<_, _>>(),
        ..Default::default()
    }
}

/// Pair with no loose CAF files available. `_rig_bones` documents what the rig owns: the pairing
/// must not depend on it.
fn remap(
    db: &AnimationDatabase,
    chrparams: &ChrParams,
    _rig_bones: &[&str],
    include_unmatched: bool,
) -> Vec<AnimationClip> {
    caf_anchored_remap(db, chrparams, include_unmatched, |_| Vec::new())
        .into_iter()
        .map(|paired| paired.clip)
        .collect()
}

#[test]
fn event_takes_the_block_its_caf_path_names_whatever_the_bones_are_called() {
    // The second block animates a bone whose name repeats every word of the event; the event's
    // CAF path still names the first.
    let db = AnimationDatabase {
        clips: vec![
            block("animations/synthetic/rig/clip_a.caf", "flap_left", 10.0),
            block("animations/synthetic/rig/clip_b.caf", "landing_gear_deploy_piston", 20.0),
        ],
    };
    let chrparams = events(&[("landing_gear_deploy", "Animations/Synthetic/Rig/clip_a.caf")]);
    let clips = remap(&db, &chrparams, &["flap_left", "landing_gear_deploy_piston"], false);
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].name, "landing_gear_deploy");
    assert_eq!(marker(&clips[0]), 10.0);
}

#[test]
fn path_named_block_is_used_even_when_it_animates_bones_the_rig_lacks() {
    let mut clip = block("animations/synthetic/rig/open.caf", "door_left", 10.0);
    clip.channels.push(block("", "bone_of_another_rig", 10.0).channels.remove(0));
    let db = AnimationDatabase { clips: vec![clip] };
    let chrparams = events(&[("open", "animations/synthetic/rig/open.caf")]);
    let clips = remap(&db, &chrparams, &["door_left"], false);
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].channels.len(), 2);
}

#[test]
fn event_whose_caf_is_absent_gets_no_clip() {
    // The only block animates this rig's bones and shares the event's words, but it is another CAF.
    let db = AnimationDatabase {
        clips: vec![block("animations/synthetic/rig/gear_retract.caf", "gear_retract_arm", 10.0)],
    };
    let chrparams = events(&[("gear_retract", "animations/synthetic/rig/missing.caf")]);
    assert!(remap(&db, &chrparams, &["gear_retract_arm"], false).is_empty());
}

#[test]
fn events_naming_the_same_caf_all_get_its_block() {
    let db = AnimationDatabase {
        clips: vec![block("animations/synthetic/rig/shared.caf", "door_left", 10.0)],
    };
    let chrparams = events(&[
        ("event_a", "animations/synthetic/rig/shared.caf"),
        ("event_b", "animations\\synthetic\\rig\\SHARED.caf"),
    ]);
    let clips = remap(&db, &chrparams, &["door_left"], true);
    assert_eq!(
        clips.iter().map(|clip| clip.name.as_str()).collect::<Vec<_>>(),
        ["event_a", "event_b"]
    );
    assert!(clips.iter().all(|clip| marker(clip) == 10.0));
}

#[test]
fn blocks_no_event_names_keep_their_database_name_only_on_request() {
    let db = AnimationDatabase {
        clips: vec![
            block("animations/synthetic/rig/used.caf", "door_left", 10.0),
            block("animations/synthetic/rig/spare.caf", "door_left", 20.0),
        ],
    };
    let chrparams = events(&[("open", "animations/synthetic/rig/used.caf")]);
    let with = remap(&db, &chrparams, &["door_left"], true);
    assert_eq!(
        with.iter().map(|clip| clip.name.as_str()).collect::<Vec<_>>(),
        ["open", "animations/synthetic/rig/spare.caf"]
    );
    assert_eq!(remap(&db, &chrparams, &["door_left"], false).len(), 1);
}

#[test]
fn event_missing_from_the_database_plays_its_loose_caf() {
    let db = AnimationDatabase {
        clips: vec![block("animations/synthetic/rig/in_database.caf", "door_left", 10.0)],
    };
    let chrparams = events(&[
        ("close", "Animations/Synthetic/Rig/loose.caf"),
        ("open", "animations/synthetic/rig/in_database.caf"),
    ]);
    let mut asked = Vec::new();
    let paired = caf_anchored_remap(&db, &chrparams, true, |path| {
        asked.push(path.to_string());
        vec![block("", "door_left", 30.0)]
    });
    assert_eq!(asked, ["Animations/Synthetic/Rig/loose.caf"]);
    assert_eq!(
        paired
            .iter()
            .map(|p| (p.clip.name.as_str(), marker(&p.clip), p.origin))
            .collect::<Vec<_>>(),
        [
            ("close", 30.0, ClipOrigin::LooseCafEvent),
            ("open", 10.0, ClipOrigin::DatabaseEvent),
        ]
    );
}

#[test]
fn caf_path_lookup_ignores_case_and_separator() {
    let db = AnimationDatabase {
        clips: vec![
            block("animations/synthetic/rig/a.caf", "door_left", 10.0),
            block("animations/synthetic/rig/b.caf", "door_left", 20.0),
        ],
    };
    let clip = clip_for_caf_path(&db, "Animations\\Synthetic\\Rig\\B.caf").unwrap();
    assert_eq!(marker(clip), 20.0);
    assert!(clip_for_caf_path(&db, "animations/synthetic/rig/c.caf").is_none());
}

#[test]
fn caf_path_in_the_database_never_consults_the_loose_file() {
    let db = AnimationDatabase {
        clips: vec![block("Animations/Synthetic/ShipA/hatch_open.caf", "hatch", 3.0)],
    };
    let clips = clips_for_caf_path(&db, "animations\\synthetic\\shipa\\HATCH_OPEN.caf", |_| {
        panic!("the database holds this CAF")
    });
    assert_eq!(clips.iter().map(marker).collect::<Vec<_>>(), [3.0]);
}

#[test]
fn caf_path_missing_from_the_database_is_the_loose_file() {
    let db = AnimationDatabase {
        clips: vec![block("Animations/Synthetic/ShipA/hatch_open.caf", "hatch", 3.0)],
    };
    let clips = clips_for_caf_path(&db, "Animations/Synthetic/ShipA/hatch_close.caf", |path| {
        assert_eq!(path, "Animations/Synthetic/ShipA/hatch_close.caf");
        vec![block("clip_0", "hatch", 7.0)]
    });
    assert_eq!(clips.iter().map(marker).collect::<Vec<_>>(), [7.0]);
}

