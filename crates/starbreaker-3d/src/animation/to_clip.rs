//! Decoded `.dba` / `.caf` clips ([`AnimationClip`]) → the shared model's
//! [`crate::clip::Clip`], so both writers can play them.
//!
//! Channels are keyed by [`bone_name_hash`] (CRC32 of the exact bone name);
//! [`shared_clip`] names them by a skeleton's bones. Values are bone-local
//! (parent-relative) samples in CryEngine space — rotation `xyzw` becomes
//! `[w, x, y, z]`, positions stay — and key times (frames at the clip's
//! `fps`) become seconds. Channels for bones the skeleton lacks, and repeat
//! channels for a bone, are dropped and counted in the log.

use super::{AnimationClip, bone_name_hash};
use crate::clip::{BoneTrack, Clip, Keys};

/// `clip` as a shared-model clip over the bones named `bone_names`.
pub fn shared_clip(clip: &AnimationClip, bone_names: &[String]) -> Clip {
    let by_hash: std::collections::HashMap<u32, &String> = bone_names.iter().map(|n| (bone_name_hash(n), n)).collect();
    let fps = if clip.fps > 0.0 { clip.fps } else { 30.0 };
    let (mut unknown, mut repeated) = (0usize, 0usize);
    let mut bones: Vec<BoneTrack> = Vec::new();
    for channel in &clip.channels {
        let Some(name) = by_hash.get(&channel.bone_hash) else {
            unknown += 1;
            continue;
        };
        if bones.iter().any(|b| &b.bone == *name) {
            repeated += 1;
            continue;
        }
        let rotation = (!channel.rotations.is_empty()).then(|| Keys {
            times: channel.rotations.iter().map(|k| k.time / fps).collect(),
            values: channel.rotations.iter().map(|k| {
                let [x, y, z, w] = k.value;
                [w, x, y, z]
            }).collect(),
        });
        let translation = (!channel.positions.is_empty()).then(|| Keys {
            times: channel.positions.iter().map(|k| k.time / fps).collect(),
            values: channel.positions.iter().map(|k| k.value).collect(),
        });
        if rotation.is_some() || translation.is_some() {
            bones.push(BoneTrack { bone: (*name).clone(), rotation, translation });
        }
    }
    if unknown + repeated > 0 {
        log::info!(
            "clip '{}': {} channel(s) for bones outside the skeleton, {} repeat channel(s) dropped",
            clip.name,
            unknown,
            repeated
        );
    }
    Clip { name: clip.name.clone(), bones, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{BoneChannel, Keyframe};

    fn channel(name: &str, rot: Vec<(f32, [f32; 4])>, pos: Vec<(f32, [f32; 3])>) -> BoneChannel {
        BoneChannel {
            bone_hash: bone_name_hash(name),
            rotations: rot.into_iter().map(|(time, value)| Keyframe { time, value }).collect(),
            positions: pos.into_iter().map(|(time, value)| Keyframe { time, value }).collect(),
            rot_format_flags: 0,
            pos_format_flags: 0,
        }
    }

    fn clip(channels: Vec<BoneChannel>) -> AnimationClip {
        AnimationClip { name: "idle".into(), fps: 30.0, channels, start_rotation: None, start_position: None }
    }

    #[test]
    fn channels_take_the_skeletons_names_and_seconds() {
        let c = clip(vec![channel("Head", vec![(0.0, [0.0, 0.0, 0.0, 1.0]), (15.0, [0.0, 0.0, 1.0, 0.0])], vec![(30.0, [0.0, 0.0, 1.6])])]);
        let shared = shared_clip(&c, &["World".into(), "Head".into()]);
        assert_eq!(shared.name, "idle");
        let head = &shared.bones[0];
        assert_eq!(head.bone, "Head");
        let rot = head.rotation.as_ref().unwrap();
        assert_eq!(rot.times, vec![0.0, 0.5]);
        assert_eq!(rot.values[1], [0.0, 0.0, 0.0, 1.0], "xyzw (0,0,1,0) → wxyz (0,0,0,1)");
        assert_eq!(head.translation.as_ref().unwrap().times, vec![1.0]);
        assert!(shared.invalid_track().is_none());
    }

    #[test]
    fn hashes_are_case_sensitive_and_unknown_or_repeat_channels_drop() {
        let c = clip(vec![
            channel("head", vec![(0.0, [0.0, 0.0, 0.0, 1.0])], vec![]),
            channel("Head", vec![(0.0, [0.0, 0.0, 0.0, 1.0])], vec![]),
            channel("Head", vec![(0.0, [1.0, 0.0, 0.0, 0.0])], vec![]),
            channel("Ghost", vec![(0.0, [0.0, 0.0, 0.0, 1.0])], vec![]),
        ]);
        let shared = shared_clip(&c, &["Head".into()]);
        assert_eq!(shared.bones.len(), 1);
        assert_eq!(shared.bones[0].rotation.as_ref().unwrap().values[0], [1.0, 0.0, 0.0, 0.0], "first Head channel kept");
    }

    #[test]
    fn empty_channels_and_zero_fps() {
        let mut c = clip(vec![channel("Head", vec![], vec![]), channel("Neck", vec![(30.0, [0.0, 0.0, 0.0, 1.0])], vec![])]);
        c.fps = 0.0;
        let shared = shared_clip(&c, &["Head".into(), "Neck".into()]);
        assert_eq!(shared.bones.len(), 1, "a channel without keys is no track");
        assert_eq!(shared.bones[0].rotation.as_ref().unwrap().times, vec![1.0], "fps 0 falls back to the 30 fps default");
    }
}
