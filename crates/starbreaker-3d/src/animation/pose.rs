//! Bone pose utilities: native Y-up final-frame pose extraction and application,
//! skeleton block selection, and the separate Blender sidecar axis conversion.

use std::collections::{HashMap, HashSet};

use crate::error::Error;

use super::{AnimationClip, AnimationDatabase, parse_dba};

/// Final-frame local TRS pose for a single bone.
#[derive(Debug, Clone, Copy)]
pub struct BonePose {
    /// Local rotation in native CryEngine Y-up `wxyz` order, if keyed.
    pub rotation: Option<[f32; 4]>,
    /// Local position in native CryEngine Y-up, if keyed.
    pub position: Option<[f32; 3]>,
}

/// Convert a quaternion produced by the SmallTree48BitQuat decoder (CryEngine
/// Y-up `xyzw` convention) into the Blender Z-up `wxyz` form used by our
/// animation sidecars. Skeleton pose baking must keep the native Y-up basis.
///
/// Must match the position axis swap used by `clip_to_json` and the static
/// import's `_scene_position_to_blender`: `(cx, cy, cz) → (cx, -cz, cy)`.
/// Applying the same basis change to a quaternion's vector component gives
/// `(qx, qy, qz, qw) → (qw, qx, -qz, qy)` in Blender WXYZ form.
pub fn cry_xyzw_to_blender_wxyz(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    [w, x, -z, y]
}

/// Read a single named animation from a `.dba` and return final-frame local
/// TRS keyed by bone CRC32 hash.
///
/// `animation_name` is matched against the metadata strings stored in the
/// DBA chunk (case-insensitive substring). Prefer [`find_block_for_skeleton`]
/// for production use; this helper is kept for debugging.
pub fn read_dba_final_pose(
    dba_bytes: &[u8],
    animation_name: &str,
) -> Result<HashMap<u32, BonePose>, Error> {
    let db = parse_dba(dba_bytes)?;
    let needle = animation_name.to_ascii_lowercase();

    let clip = db
        .clips
        .iter()
        .find(|c| c.name.to_ascii_lowercase().contains(&needle))
        .ok_or_else(|| Error::Other(format!("Animation '{animation_name}' not found in DBA")))?;

    Ok(clip_final_pose(clip))
}

/// Build a final-frame `BonePose` map from a single animation clip.
///
/// Keep the skeleton's native CryEngine Y-up basis: reorder quaternions from
/// `xyzw` to `wxyz`, leaving positions unchanged. Missing channels stay `None`
/// so applying the pose preserves their bind components. The Blender axis
/// conversion belongs only to animation sidecar serialization.
pub fn clip_final_pose(clip: &AnimationClip) -> HashMap<u32, BonePose> {
    let mut poses = HashMap::with_capacity(clip.channels.len());
    for ch in &clip.channels {
        let rotation = ch.rotations.last().map(|kf| {
            let [x, y, z, w] = kf.value;
            [w, x, y, z]
        });
        let position = ch.positions.last().map(|kf| kf.value);
        poses.insert(ch.bone_hash, BonePose { rotation, position });
    }
    poses
}

/// Pick the best matching animation clip in `db` for a given skeleton, by
/// bone-hash signature.
///
/// A clip is a *candidate* iff every one of its channel bone hashes is present
/// in the skeleton (clip bones ⊆ skeleton bones). The first candidate is
/// returned, with the option to break ties by selecting the clip with the
/// largest angular delta between first and last keyframe.
pub fn find_block_for_skeleton<'a>(
    db: &'a AnimationDatabase,
    skeleton_bone_hashes: &HashSet<u32>,
    prefer_longest_arc: bool,
) -> Option<&'a AnimationClip> {
    let candidates: Vec<&AnimationClip> = db
        .clips
        .iter()
        .filter(|c| {
            !c.channels.is_empty()
                && c.channels
                    .iter()
                    .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }
    if !prefer_longest_arc || candidates.len() == 1 {
        return Some(candidates[0]);
    }

    candidates
        .into_iter()
        .map(|c| (clip_arc_score(c), c))
        .fold(None, |acc, (s, c)| match acc {
            None => Some((s, c)),
            Some((bs, _)) if s > bs => Some((s, c)),
            other => other,
        })
        .map(|(_, c)| c)
}

/// Sum of angular deltas between first and last rotation key across all
/// channels (radians, sign-invariant). Higher = more motion.
fn clip_arc_score(clip: &AnimationClip) -> f32 {
    let mut total = 0.0f32;
    for ch in &clip.channels {
        if let (Some(first), Some(last)) = (ch.rotations.first(), ch.rotations.last()) {
            let a = first.value;
            let b = last.value;
            let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs();
            total += 1.0 - dot.clamp(0.0, 1.0);
        }
    }
    total
}

/// Quaternion multiplication on `wxyz` quaternions (Blender convention).
pub(crate) fn quat_mul_wxyz(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [aw, ax, ay, az] = a;
    let [bw, bx, by, bz] = b;
    [
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    ]
}

/// Rotate a 3-vector by a `wxyz` unit quaternion.
pub(crate) fn quat_rotate_vec_wxyz(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [w, x, y, z] = q;
    let [vx, vy, vz] = v;
    let tx = 2.0 * (y * vz - z * vy);
    let ty = 2.0 * (z * vx - x * vz);
    let tz = 2.0 * (x * vy - y * vx);
    [
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    ]
}

/// Bone-like accessor: minimal interface needed to bake a pose without a
/// circular dependency on `crate::skeleton`.
pub trait BoneTransforms {
    fn name(&self) -> &str;
    fn parent_index(&self) -> Option<usize>;
    fn local_rotation_wxyz(&self) -> [f32; 4];
    fn local_position(&self) -> [f32; 3];
    fn set_local_rotation_wxyz(&mut self, q: [f32; 4]);
    fn set_local_position(&mut self, p: [f32; 3]);
    fn set_world_rotation_wxyz(&mut self, q: [f32; 4]);
    fn set_world_position(&mut self, p: [f32; 3]);
}

/// Compute the CRC32 hash that DBA uses for a bone name.
///
/// CryEngine uses standard CRC32 (zlib polynomial) on the **case-preserved**
/// UTF-8 byte sequence of the bone name (no terminator).
pub fn bone_name_hash(name: &str) -> u32 {
    crc32fast::hash(name.as_bytes())
}

/// Apply keyed native Y-up components to bones, preserving unkeyed local
/// components and recomputing cached world TRS. Returns the number of bones
/// with at least one applied component; inverse-bind transforms stay unchanged.
pub fn apply_pose_to_skeleton<B: BoneTransforms>(
    bones: &mut [B],
    pose: &HashMap<u32, BonePose>,
) -> usize {
    let mut updated = 0usize;
    for bone in bones.iter_mut() {
        let h = bone_name_hash(bone.name());
        if let Some(p) = pose.get(&h) {
            if let Some(rot) = p.rotation {
                bone.set_local_rotation_wxyz(rot);
            }
            if let Some(pos) = p.position {
                bone.set_local_position(pos);
            }
            if p.rotation.is_some() || p.position.is_some() {
                updated += 1;
            }
        }
    }
    if updated == 0 {
        return 0;
    }

    let n = bones.len();
    let mut world_q: Vec<[f32; 4]> = vec![[1.0, 0.0, 0.0, 0.0]; n];
    let mut world_p: Vec<[f32; 3]> = vec![[0.0; 3]; n];
    let mut done: Vec<bool> = vec![false; n];

    let mut progress = true;
    while progress {
        progress = false;
        for i in 0..n {
            if done[i] {
                continue;
            }
            let parent = bones[i].parent_index();
            let (pq, pp) = match parent {
                None => ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                Some(pi) if pi == i => ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                Some(pi) if pi < n && done[pi] => (world_q[pi], world_p[pi]),
                Some(_) => continue,
            };
            let lq = bones[i].local_rotation_wxyz();
            let lp = bones[i].local_position();
            world_q[i] = quat_mul_wxyz(pq, lq);
            let rotated = quat_rotate_vec_wxyz(pq, lp);
            world_p[i] = [pp[0] + rotated[0], pp[1] + rotated[1], pp[2] + rotated[2]];
            done[i] = true;
            progress = true;
        }
    }

    for i in 0..n {
        if done[i] {
            bones[i].set_world_rotation_wxyz(world_q[i]);
            bones[i].set_world_position(world_p[i]);
        }
    }

    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{BoneChannel, Keyframe, clip_to_json};
    use crate::skeleton::Bone;

    fn synthetic_bone(name: &str) -> Bone {
        // Arbitrary synthetic bind transform, unrelated to any game asset.
        Bone {
            name: name.into(),
            parent_index: None,
            object_node_index: None,
            local_position: [7.0, 8.0, 9.0],
            local_rotation: [0.5, -0.5, 0.5, 0.5],
            world_position: [7.0, 8.0, 9.0],
            world_rotation: [0.5, -0.5, 0.5, 0.5],
            bind_world_position: [7.0, 8.0, 9.0],
            bind_world_rotation: [0.5, -0.5, 0.5, 0.5],
        }
    }

    fn synthetic_clip(channels: Vec<BoneChannel>) -> AnimationClip {
        AnimationClip {
            name: "synthetic_pose".into(),
            fps: 30.0,
            channels,
            start_rotation: None,
            start_position: None,
        }
    }

    #[test]
    fn final_pose_preserves_unkeyed_bind_components() {
        let mut bones = [
            synthetic_bone("position_only"),
            synthetic_bone("rotation_only"),
            synthetic_bone("empty"),
            synthetic_bone("absent"),
        ];
        let bind = bones.clone();
        let clip = synthetic_clip(vec![
            BoneChannel {
                bone_hash: bone_name_hash("position_only"),
                rotations: vec![],
                positions: vec![Keyframe {
                    time: 1.0,
                    value: [2.0, 0.0, 0.0],
                }],
                rot_format_flags: 0,
                pos_format_flags: 0,
            },
            BoneChannel {
                bone_hash: bone_name_hash("rotation_only"),
                rotations: vec![Keyframe {
                    time: 1.0,
                    value: [1.0, 0.0, 0.0, 0.0],
                }],
                positions: vec![],
                rot_format_flags: 0,
                pos_format_flags: 0,
            },
            BoneChannel {
                bone_hash: bone_name_hash("empty"),
                rotations: vec![],
                positions: vec![],
                rot_format_flags: 0,
                pos_format_flags: 0,
            },
        ]);
        apply_pose_to_skeleton(&mut bones, &clip_final_pose(&clip));
        assert_eq!(bones[0].local_rotation, bind[0].local_rotation);
        assert_eq!(bones[0].local_position, [2.0, 0.0, 0.0]);
        assert_eq!(bones[1].local_position, bind[1].local_position);
        assert_eq!(bones[1].local_rotation, [0.0, 1.0, 0.0, 0.0]);
        for i in [2, 3] {
            assert_eq!(bones[i].local_rotation, bind[i].local_rotation);
            assert_eq!(bones[i].local_position, bind[i].local_position);
        }
    }

    #[test]
    fn final_pose_is_native_y_up_while_sidecar_stays_blender_z_up() {
        let mut bones = [synthetic_bone("sampled")];
        let bind = bones[0].clone();
        // Synthetic unit quaternion with distinct Y/Z components and sign.
        let clip = synthetic_clip(vec![BoneChannel {
            bone_hash: bone_name_hash("sampled"),
            rotations: vec![
                Keyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0, 1.0],
                },
                Keyframe {
                    time: 4.0,
                    value: [0.0, 0.6, 0.0, 0.8],
                },
            ],
            positions: vec![
                Keyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0],
                },
                Keyframe {
                    time: 4.0,
                    value: [1.0, 2.0, -3.0],
                },
            ],
            rot_format_flags: 0,
            pos_format_flags: 0,
        }]);
        let before = serde_json::to_vec(&clip_to_json(&clip)).unwrap();
        assert_eq!(
            apply_pose_to_skeleton(&mut bones, &clip_final_pose(&clip)),
            1
        );
        assert_eq!(bones[0].local_position, [1.0, 2.0, -3.0]);
        assert_eq!(bones[0].local_rotation, [0.8, 0.0, 0.6, 0.0]);
        assert_eq!(bones[0].world_position, bones[0].local_position);
        assert_eq!(bones[0].world_rotation, bones[0].local_rotation);
        assert_eq!(bones[0].bind_world_position, bind.bind_world_position);
        assert_eq!(bones[0].bind_world_rotation, bind.bind_world_rotation);
        let sidecar = clip_to_json(&clip);
        assert_eq!(before, serde_json::to_vec(&sidecar).unwrap());
        let key = format!("0x{:X}", bone_name_hash("sampled"));
        assert_eq!(
            sidecar["bones"][&key]["position"][1],
            serde_json::json!([1.0, 3.0, 2.0])
        );
        assert_eq!(
            sidecar["bones"][&key]["rotation"][1],
            serde_json::json!([0.8_f32, 0.0, 0.0, 0.6_f32])
        );
    }
}
