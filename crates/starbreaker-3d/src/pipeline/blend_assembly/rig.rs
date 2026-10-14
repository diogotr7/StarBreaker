//! The shared rig (`skeleton::Bone`, `Mesh.morph_targets`) as Blender data for
//! the `.blend` writers.
//!
//! - [`bone_rests`]: the skeleton's BIND pose as armature-space bone rests
//!   (the space the mesh vertices are written in), so an Armature modifier
//!   deforms exactly like the glTF skin (`inverse(bind) · pose`). Bones with
//!   no name are skipped (their children hang under the nearest named
//!   ancestor), as the glTF writer skips them.
//! - [`RigObject`]: an armature `Object` + `bArmature` + bones, ready to write.
//! - [`shape_key_targets`]: morph targets as absolute shape-key positions.
//! - [`bone_curves`] / [`shape_key_curves`]: animation clips as pose-bone and
//!   shape-key F-Curves; [`RigObject`] and [`animate_shape_keys`] attach them
//!   as actions.
//!
//! Bone display length is presentation only (Blender needs a non-zero bone):
//! the distance to the nearest child joint, else half the parent's length,
//! else 2 % of the skeleton's extent.

use starbreaker_blend::action::{ActionData, ID_ADT_OFFSET, ID_KE, ID_OB, SDNA_IDX_ANIM_DATA};
use starbreaker_blend::armature::{ArmatureData, BoneRest, build_armature_object};
use starbreaker_blend::shape_keys::ShapeKeyData;
use starbreaker_blend::{PtrAlloc, SDNA_IDX_OBJECT, write_block, write_ptr};

use crate::skeleton::Bone;
use crate::types::Mesh;

/// Name of the Armature modifier the writers add to skinned meshes.
pub(super) const ARMATURE_MODIFIER_NAME: &str = "StarBreaker Armature";

/// Bind-pose bone rests for the named bones of `bones` (see the module docs).
pub(super) fn bone_rests(bones: &[Bone]) -> Vec<BoneRest> {
    // Source index → output index, for named bones.
    let mut out_index: Vec<Option<usize>> = vec![None; bones.len()];
    let mut rests: Vec<BoneRest> = Vec::new();
    let mut source_of: Vec<usize> = Vec::new();
    for (i, bone) in bones.iter().enumerate() {
        if bone.name.is_empty() {
            continue;
        }
        // Nearest named ancestor already emitted (parents precede children).
        let mut parent = bone.parent_index.map(usize::from);
        let mut guard = 0;
        while let Some(p) = parent {
            if p >= i || guard > bones.len() {
                parent = None;
                break;
            }
            if out_index[p].is_some() {
                break;
            }
            parent = bones[p].parent_index.map(usize::from);
            guard += 1;
        }
        let [w, x, y, z] = bone.bind_world_rotation;
        let m = glam::Mat3::from_quat(glam::Quat::from_xyzw(x, y, z, w).normalize());
        out_index[i] = Some(rests.len());
        source_of.push(i);
        rests.push(BoneRest {
            name: bone.name.clone(),
            parent: parent.and_then(|p| out_index[p]),
            head: bone.bind_world_position,
            rotation: [m.x_axis.to_array(), m.y_axis.to_array(), m.z_axis.to_array()],
            length: 0.0,
        });
    }

    let heads: Vec<glam::Vec3> = rests.iter().map(|r| glam::Vec3::from(r.head)).collect();
    let (lo, hi) = heads.iter().fold((glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)), |(lo, hi), h| (lo.min(*h), hi.max(*h)));
    let extent = if heads.is_empty() { 0.0 } else { (hi - lo).length() };
    let fallback = if extent > 0.0 { 0.02 * extent } else { 0.05 };
    let min_length = fallback * 0.05;
    for i in 0..rests.len() {
        let nearest_child = (0..rests.len())
            .filter(|&c| rests[c].parent == Some(i))
            .map(|c| heads[c].distance(heads[i]))
            .filter(|&d| d > min_length)
            .fold(f32::INFINITY, f32::min);
        rests[i].length = if nearest_child.is_finite() {
            nearest_child
        } else {
            rests[i].parent.map_or(fallback, |p| (rests[p].length * 0.5).max(min_length))
        };
    }
    rests
}

/// Scene frame rate of the `.blend` writers (`Scene.r.frs_sec`); clip time
/// `t` (seconds) lands on frame `1 + t * BLEND_FPS`.
pub(super) const BLEND_FPS: f32 = 24.0;

fn frame(t: f32) -> f32 {
    1.0 + t * BLEND_FPS
}

/// The clip's bone tracks as pose-bone F-Curves (`rotation_quaternion`
/// w, x, y, z then `location`, grouped by bone). A pose channel is relative to
/// the bone's rest local `R = parent_rest⁻¹ · rest` (the bind pose, as
/// [`bone_rests`] lays it out), so the animated local `L` becomes
/// `R⁻¹ · L`: rotation `R_rot⁻¹ · L_rot`, location `R_rot⁻¹ · (L_t − R_t)`.
/// Tracks for bones the armature lacks are skipped (logged).
pub(super) fn bone_curves(clip: &crate::clip::Clip, bones: &[Bone]) -> Vec<starbreaker_blend::action::Curve> {
    use starbreaker_blend::action::Curve;
    // Parity guard (docs/superpowers/plans/2026-10-14-glb-blend-parity.md): no `..` — a new Clip field must be written
    // by the .blend writers or named as a gap.
    let crate::clip::Clip {
        name: _,
        bones: _,            // here: pose-bone curves
        morphs: _,           // shape_key_curves
        material_params: _, // PARITY GAP (plan E): shader parameter tracks (wrinkles) are not written to .blend
    } = clip;
    let rests = bone_rests(bones);
    let world = |r: &BoneRest| {
        let m = glam::Mat3::from_cols(r.rotation[0].into(), r.rotation[1].into(), r.rotation[2].into());
        (glam::Quat::from_mat3(&m).normalize(), glam::Vec3::from(r.head))
    };
    let mut curves = Vec::new();
    for track in &clip.bones {
        let Some(rest) = rests.iter().find(|r| r.name == track.bone) else {
            log::warn!("animation '{}': no armature bone '{}'", clip.name, track.bone);
            continue;
        };
        let (rot, head) = world(rest);
        let (rest_rot, rest_t) = match rest.parent.map(|p| world(&rests[p])) {
            Some((parent_rot, parent_head)) => (parent_rot.inverse() * rot, parent_rot.inverse() * (head - parent_head)),
            None => (rot, head),
        };
        let inverse = rest_rot.inverse();
        let path = |what: &str| format!("pose.bones[\"{}\"].{what}", track.bone);
        if let Some(keys) = &track.rotation {
            let mut previous = glam::Quat::IDENTITY;
            let pose: Vec<glam::Quat> = keys
                .values
                .iter()
                .map(|&[w, x, y, z]| {
                    let q = inverse * glam::Quat::from_xyzw(x, y, z, w).normalize();
                    let q = if q.dot(previous) < 0.0 { -q } else { q };
                    previous = q;
                    q
                })
                .collect();
            for (index, component) in [3usize, 0, 1, 2].into_iter().enumerate() {
                curves.push(Curve {
                    rna_path: path("rotation_quaternion"),
                    array_index: index as i32,
                    group: Some(track.bone.clone()),
                    keys: keys.times.iter().zip(&pose).map(|(&t, q)| [frame(t), q.to_array()[component]]).collect(),
                });
            }
        }
        if let Some(keys) = &track.translation {
            let pose: Vec<glam::Vec3> = keys.values.iter().map(|&v| inverse * (glam::Vec3::from(v) - rest_t)).collect();
            for index in 0..3 {
                curves.push(Curve {
                    rna_path: path("location"),
                    array_index: index as i32,
                    group: Some(track.bone.clone()),
                    keys: keys.times.iter().zip(&pose).map(|(&t, v)| [frame(t), v[index]]).collect(),
                });
            }
        }
    }
    curves
}

/// The clip's morph tracks for `mesh`'s morph targets as shape-key `value`
/// F-Curves.
pub(super) fn shape_key_curves(clip: &crate::clip::Clip, mesh: &Mesh) -> Vec<starbreaker_blend::action::Curve> {
    clip.morphs
        .iter()
        .filter(|m| mesh.morph_targets.iter().any(|t| t.name == m.name))
        .map(|m| starbreaker_blend::action::Curve {
            rna_path: format!("key_blocks[\"{}\"].value", m.name),
            array_index: 0,
            group: None,
            keys: m.keys.times.iter().zip(&m.keys.values).map(|(&t, &v)| [frame(t), v]).collect(),
        })
        .collect()
}

/// Absolute shape-key positions for each morph target of `mesh`.
pub(super) fn shape_key_targets(mesh: &Mesh) -> Vec<(String, Vec<[f32; 3]>)> {
    mesh.morph_targets
        .iter()
        .map(|t| {
            let positions = mesh
                .positions
                .iter()
                .zip(&t.position_deltas)
                .map(|(p, d)| [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
                .collect();
            (t.name.clone(), positions)
        })
        .collect()
}

/// An armature object with its armature data and bone actions (the first
/// clip assigned, the others kept with a fake user), ready to write.
pub(super) struct RigObject {
    pub object_ptr: u64,
    object: Vec<u8>,
    data: ArmatureData,
    anim: Option<(u64, Vec<u8>)>,
    actions: Vec<ActionData>,
}

/// Actions for `clips` on one ID: the first assigned through a new
/// `AnimData` (pointer, block), the rest kept with a fake user.
fn id_actions(
    clips: &[crate::clip::Clip],
    curves: impl Fn(&crate::clip::Clip) -> Vec<starbreaker_blend::action::Curve>,
    action_name: impl Fn(&str) -> String,
    owner: (i16, &str),
    ptrs: &mut PtrAlloc,
) -> Result<(Option<(u64, Vec<u8>)>, Vec<ActionData>), crate::error::Error> {
    let mut actions = Vec::new();
    for clip in clips {
        let curves = curves(clip);
        if curves.is_empty() {
            continue;
        }
        let mut action = ActionData::new(&action_name(&clip.name), owner.0, owner.1, &curves, ptrs)
            .map_err(crate::error::Error::Other)?;
        if !actions.is_empty() {
            action.set_fake_user();
        }
        actions.push(action);
    }
    let anim = actions.first().map(|a| (ptrs.alloc(), a.anim_data()));
    Ok((anim, actions))
}

/// Animate the shape keys of `mesh` (its `Key` in `keys`) with the clips'
/// morph tracks; returns the actions to write after the key.
pub(super) fn animate_shape_keys(
    clips: &[crate::clip::Clip],
    mesh: &Mesh,
    keys: &mut ShapeKeyData,
    ptrs: &mut PtrAlloc,
) -> Result<Vec<ActionData>, crate::error::Error> {
    let (anim, actions) =
        id_actions(clips, |c| shape_key_curves(c, mesh), |n| format!("{n}_shapes"), (ID_KE, "Key"), ptrs)?;
    if let Some((ptr, data)) = anim {
        keys.set_anim_data(ptr, data);
    }
    Ok(actions)
}

/// The last frame any clip reaches (for the scene's end frame).
pub(super) fn clips_end_frame(clips: &[crate::clip::Clip]) -> Option<i32> {
    clips
        .iter()
        .flat_map(|c| {
            let bones = c.bones.iter().flat_map(|b| {
                b.rotation.iter().flat_map(|k| k.times.iter()).chain(b.translation.iter().flat_map(|k| k.times.iter()))
            });
            bones.chain(c.morphs.iter().flat_map(|m| m.keys.times.iter()))
        })
        .copied()
        .reduce(f32::max)
        .map(|t| frame(t).ceil() as i32)
}

impl RigObject {
    /// The armature object `name` for `bones` (bind pose) animated by the
    /// clips' bone tracks, parented to `parent_ptr` with a local transform
    /// (`quat` `[w, x, y, z]`). `None` when no bone is named.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        bones: &[Bone],
        clips: &[crate::clip::Clip],
        parent_ptr: u64,
        loc: [f32; 3],
        quat: [f32; 4],
        scale: [f32; 3],
        ptrs: &mut PtrAlloc,
    ) -> Result<Option<Self>, crate::error::Error> {
        let rests = bone_rests(bones);
        if rests.is_empty() {
            return Ok(None);
        }
        let object_ptr = ptrs.alloc();
        let data = ArmatureData::new(name, &rests, ptrs).map_err(crate::error::Error::Other)?;
        let mut object = build_armature_object(name, data.armature_ptr, loc, quat, scale, parent_ptr);
        let (anim, actions) = id_actions(clips, |c| bone_curves(c, bones), str::to_owned, (ID_OB, name), ptrs)?;
        if let Some((ptr, _)) = &anim {
            write_ptr(&mut object, ID_ADT_OFFSET, *ptr);
        }
        Ok(Some(Self { object_ptr, object, data, anim, actions }))
    }

    /// Write the `OB` block and its `AnimData`, the armature and its bones,
    /// then the actions.
    pub fn write(&self, out: &mut Vec<u8>) {
        write_block(out, b"OB\0\0", SDNA_IDX_OBJECT, self.object_ptr, 1, &self.object);
        if let Some((ptr, data)) = &self.anim {
            write_block(out, b"DATA", SDNA_IDX_ANIM_DATA, *ptr, 1, data);
        }
        self.data.write(out);
        for action in &self.actions {
            action.write(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bone(name: &str, parent: Option<u16>, at: [f32; 3]) -> Bone {
        Bone {
            name: name.into(),
            parent_index: parent,
            object_node_index: None,
            local_position: at,
            local_rotation: [1.0, 0.0, 0.0, 0.0],
            world_position: at,
            world_rotation: [1.0, 0.0, 0.0, 0.0],
            bind_world_position: at,
            bind_world_rotation: [1.0, 0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn rests_use_the_bind_pose_and_keep_the_hierarchy() {
        let mut posed = bone("jaw", Some(0), [0.0, 0.1, 0.0]);
        posed.world_position = [5.0, 5.0, 5.0]; // a pose must not leak into the rest
        let h = std::f32::consts::FRAC_1_SQRT_2;
        posed.bind_world_rotation = [h, 0.0, 0.0, h]; // 90° about Z
        let rests = bone_rests(&[bone("root", None, [0.0; 3]), posed]);
        assert_eq!(rests[1].head, [0.0, 0.1, 0.0]);
        assert_eq!(rests[1].parent, Some(0));
        let y = rests[1].rotation[1];
        assert!((y[0] + 1.0).abs() < 1e-6 && y[1].abs() < 1e-6, "Y axis rotated onto -X: {y:?}");
    }

    #[test]
    fn rests_skip_unnamed_bones_and_reparent_their_children() {
        let rests = bone_rests(&[bone("root", None, [0.0; 3]), bone("", Some(0), [0.0, 0.0, 1.0]), bone("tip", Some(1), [0.0, 0.0, 2.0])]);
        assert_eq!(rests.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["root", "tip"]);
        assert_eq!(rests[1].parent, Some(0));
    }

    #[test]
    fn display_lengths_are_positive() {
        let rests = bone_rests(&[bone("root", None, [0.0; 3]), bone("a", Some(0), [0.0, 0.0, 1.0]), bone("b", Some(0), [0.0, 0.0, 0.0])]);
        assert!((rests[0].length - 1.0).abs() < 1e-6, "nearest child at 1 (the coincident one is ignored)");
        assert!(rests.iter().all(|r| r.length > 0.0), "{rests:?}");
        let single = bone_rests(&[bone("only", None, [0.0; 3])]);
        assert!(single[0].length > 0.0);
    }

    fn keys<T>(times: &[f32], values: Vec<T>) -> crate::clip::Keys<T> {
        crate::clip::Keys { times: times.to_vec(), values }
    }

    /// Pose channels are the animated local relative to the rest local:
    /// a track equal to the rest is the identity pose; a turn of the child
    /// about its parent shows up as that turn.
    #[test]
    fn bone_curves_are_relative_to_the_rest() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let mut jaw = bone("jaw", Some(0), [0.0, 0.0, 1.0]);
        jaw.bind_world_rotation = [h, 0.0, 0.0, h]; // rest: 90° about Z
        jaw.local_rotation = [h, 0.0, 0.0, h];
        let bones = vec![bone("root", None, [0.0; 3]), jaw];
        let clip = crate::clip::Clip {
            name: "c".into(),
            bones: vec![crate::clip::BoneTrack {
                bone: "jaw".into(),
                // rest, then rest + 90° more about Z; translation moved up 0.5
                rotation: Some(keys(&[0.0, 1.0], vec![[h, 0.0, 0.0, h], [0.0, 0.0, 0.0, 1.0]])),
                translation: Some(keys(&[0.0, 1.0], vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.5]])),
            }],
            ..Default::default()
        };
        let curves = bone_curves(&clip, &bones);
        let path = |c: &starbreaker_blend::action::Curve| (c.rna_path.clone(), c.array_index);
        assert_eq!(curves.len(), 7);
        assert_eq!(path(&curves[0]), ("pose.bones[\"jaw\"].rotation_quaternion".to_string(), 0));
        assert_eq!(path(&curves[4]), ("pose.bones[\"jaw\"].location".to_string(), 0));
        assert!(curves.iter().all(|c| c.group.as_deref() == Some("jaw")));
        let at = |c: usize, k: usize| curves[c].keys[k];
        assert!(at(0, 0)[0] == 1.0 && (at(0, 0)[1] - 1.0).abs() < 1e-5, "frame 1: identity w");
        assert!((at(0, 1)[0] - (1.0 + BLEND_FPS)).abs() < 1e-4, "1 s later at the scene rate");
        assert!((at(0, 1)[1] - h).abs() < 1e-5 && (at(3, 1)[1] - h).abs() < 1e-5, "90° more about Z");
        // world +Z offset in a frame turned 90° about Z is still +Z
        assert!((at(6, 1)[1] - 0.5).abs() < 1e-5, "location z: {:?}", curves[6].keys);
    }

    #[test]
    fn bone_curves_skip_unknown_bones_and_keep_quaternions_continuous() {
        let bones = vec![bone("root", None, [0.0; 3])];
        let clip = crate::clip::Clip {
            name: "c".into(),
            bones: vec![
                crate::clip::BoneTrack { bone: "ghost".into(), rotation: Some(keys(&[0.0], vec![[1.0, 0.0, 0.0, 0.0]])), translation: None },
                crate::clip::BoneTrack {
                    bone: "root".into(),
                    rotation: Some(keys(&[0.0, 1.0], vec![[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]])),
                    translation: None,
                },
            ],
            ..Default::default()
        };
        let curves = bone_curves(&clip, &bones);
        assert_eq!(curves.len(), 4, "rotation only, ghost skipped");
        assert_eq!(curves[0].keys[1][1], 1.0, "-q flipped to stay next to q");
    }

    #[test]
    fn shape_key_curves_follow_the_meshs_targets() {
        let mut mesh = Mesh::empty();
        mesh.positions = vec![[0.0; 3]];
        mesh.morph_targets = vec![crate::mesh_rig::MorphTarget { name: "bs_001".into(), position_deltas: vec![[0.0; 3]], normal_deltas: None }];
        let clip = crate::clip::Clip {
            name: "c".into(),
            morphs: vec![
                crate::clip::ScalarTrack { name: "bs_000".into(), keys: keys(&[0.0], vec![1.0]) },
                crate::clip::ScalarTrack { name: "bs_001".into(), keys: keys(&[0.0, 0.5], vec![0.0, 1.0]) },
            ],
            ..Default::default()
        };
        let curves = shape_key_curves(&clip, &mesh);
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].rna_path, "key_blocks[\"bs_001\"].value");
        assert_eq!(curves[0].keys, vec![[1.0, 0.0], [1.0 + 0.5 * BLEND_FPS, 1.0]]);
    }

    #[test]
    fn shape_key_targets_are_absolute() {
        let mut mesh = Mesh::empty();
        mesh.positions = vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        mesh.morph_targets = vec![crate::mesh_rig::MorphTarget {
            name: "bs_000".into(),
            position_deltas: vec![[0.0, 0.0, 0.5], [0.0; 3]],
            normal_deltas: None,
        }];
        let targets = shape_key_targets(&mesh);
        assert_eq!(targets[0].0, "bs_000");
        assert_eq!(targets[0].1, vec![[1.0, 0.0, 0.5], [0.0, 1.0, 0.0]]);
    }
}
