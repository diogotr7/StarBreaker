//! Stage 8: apply per-joint translation and rotation deltas to the bind-pose skeleton.
//!
//! Two modes: [`apply_deltas`] POSES the skeleton (locals change, `world_*`
//! keep the bind pose, so skinning deforms the mesh — expressions), while
//! [`rebind_skeleton`] makes the deltas the NEW BIND POSE (locals change and
//! [`refresh_world_from_locals`] recomputes `world_*` by FK, so the GLB inverse
//! binds follow and the rest mesh is not deformed — the per-character slot-15
//! neutral skeleton).
//!
//! Inputs:
//!   - `rig.skeleton_bone_name_crcs`: CRC32 IEEE hashes (`num_joints` entries —
//!     179 in the observed fixture) — the RigLogic joint order. (From
//!     `crate::riglogic::RigLogic`.)
//!   - `deltas`: `&JointDeltas` with `translation` and `rotation_eu` of length
//!     `rig.num_joints`, from `compute_joint_deltas` or all-zero if DNA
//!     application is skipped.
//!   - `rest_pose`: `BindPoseSkeleton` with 198 bones from the head `.skin`'s
//!     `CompiledBonesIvo320` chunk.
//!
//! Per-joint amplification:
//!   The RigLogic region-weight modulation needs the V1.9 sparse layout
//!   (`rig.blend_region_bones` + `rig.region_weights`) — exact formula is
//!   not yet anchored. For v1 (skip-DNA), we apply the delta unmodified
//!   (amp = 1.0). When deltas are all-zero this is a no-op; when Phase 4
//!   lands and produces real deltas, the amplification math will need a
//!   Ghidra-anchored update.
//!
//! Output: `ModifiedSkeleton` with each bone's `bind_translation` and
//! `bind_rotation` updated by the DNA delta if the bone's CRC32-IEEE-of-name
//! appears in the rig's joint list. Translation is applied additively;
//! rotation is composed as `bind_quat * euler_xyz_to_quat(delta_eu)`.
//! Bones outside the rig (e.g. neck base, body root) stay at rest pose.

use std::collections::HashMap;

use starbreaker_common::NameHash;

use crate::character::head_skin::{BindPoseSkeleton, Bone};
use crate::riglogic::RigLogic;

use super::morphing::JointDeltas;
use super::CharacterError;

/// The bind-pose skeleton after DNA deltas have been applied.
///
/// Bone order and parentage are identical to the input `BindPoseSkeleton`.
/// Only `bind_translation` and `bind_rotation` may differ (for bones the
/// RigLogic joint list references); all other fields are copied verbatim.
#[derive(Debug, Clone)]
pub struct ModifiedSkeleton {
    pub bones: Vec<Bone>,
}

/// Apply per-joint translation and rotation deltas to the rest pose.
///
/// Returns `Ok(ModifiedSkeleton)` on success.
///
/// # Errors
///
/// - `CharacterError::Rbf` — `deltas.translation.len()` or
///   `deltas.rotation_eu.len()` != `rig.num_joints as usize`.
/// - `CharacterError::JointMappingFailed` — a RigLogic joint CRC has no
///   matching bone in `rest_pose`. This is always an error because silently
///   dropping a joint would produce a wrong deformation.
pub fn apply_deltas(
    rig: &RigLogic,
    deltas: &JointDeltas,
    rest_pose: &BindPoseSkeleton,
) -> Result<ModifiedSkeleton, CharacterError> {
    let n = rig.num_joints as usize;
    if deltas.translation.len() != n {
        return Err(CharacterError::Rbf(format!(
            "apply_deltas: deltas.translation.len()={} but rig.num_joints={}",
            deltas.translation.len(),
            n
        )));
    }
    if deltas.rotation_eu.len() != n {
        return Err(CharacterError::Rbf(format!(
            "apply_deltas: deltas.rotation_eu.len()={} but rig.num_joints={}",
            deltas.rotation_eu.len(),
            n
        )));
    }

    // Build CRC → rest-pose bone index lookup.
    // Phase 3 finding: bone-name CRCs in the .skin's CompiledBonesIvo320 use
    // CRC32 IEEE (same algorithm as RigLogic Section F), verified by
    // reconstructing every name in the fixture from the trailing string table.
    let mut crc_to_bone: HashMap<u32, usize> =
        HashMap::with_capacity(rest_pose.bones.len());
    for (i, bone) in rest_pose.bones.iter().enumerate() {
        crc_to_bone.insert(bone.name_crc, i);
    }

    // Start with the rest pose.
    let mut bones = rest_pose.bones.clone();

    for rig_joint_idx in 0..n {
        let crc = rig.skeleton_bone_name_crcs[rig_joint_idx];
        let bone_idx = match crc_to_bone.get(&crc) {
            Some(&i) => i,
            None => {
                return Err(CharacterError::JointMappingFailed {
                    rig_joint_idx: rig_joint_idx as u32,
                    crc: NameHash(crc),
                });
            }
        };

        // Translation: additive (per Pass 7 §6.9.5).
        let t_delta = deltas.translation[rig_joint_idx];
        for axis in 0..3 {
            bones[bone_idx].bind_translation[axis] += t_delta[axis];
        }

        // Rotation: bind_quat * euler_to_quat(delta_eu) (per Pass 7 §6.9.5).
        let r_delta = deltas.rotation_eu[rig_joint_idx];
        if r_delta != [0.0_f32; 3] {
            let delta_quat = euler_xyz_to_quat(r_delta);
            bones[bone_idx].bind_rotation =
                quat_multiply(bones[bone_idx].bind_rotation, delta_quat);
        }
    }

    Ok(ModifiedSkeleton { bones })
}

/// Apply per-joint deltas as a NEW BIND POSE (re-bind), not as a pose.
///
/// Runs [`apply_deltas`] (deltas are parent-LOCAL: `.dna` slot 15 stores each
/// joint's parent-local bind translation per variant, byte-identical to the
/// `.skin` locals for variant 0), then recomputes every bone's world
/// transform by forward kinematics from the new locals. Because the GLB
/// inverse bind matrices are built from `world_*`, the result moves the
/// joints WITHOUT deforming the rest mesh: the per-character skeleton becomes
/// the rest pose that expression animation pivots around, while the
/// per-vertex identity shape comes from slot 18. Children of a moved joint
/// move with it (their locals are unchanged), as in the engine's FK.
///
/// # Errors
/// Same as [`apply_deltas`].
pub fn rebind_skeleton(
    rig: &RigLogic,
    deltas: &JointDeltas,
    rest: &BindPoseSkeleton,
) -> Result<BindPoseSkeleton, CharacterError> {
    let mut bones = apply_deltas(rig, deltas, rest)?.bones;
    refresh_world_from_locals(&mut bones);
    Ok(BindPoseSkeleton { bones })
}

/// Recompute `world_translation` / `world_rotation` from the parent-local
/// `bind_translation` / `bind_rotation` chain. Parents must precede children
/// (guaranteed by `CompiledBonesIvo320`).
pub fn refresh_world_from_locals(bones: &mut [Bone]) {
    for i in 0..bones.len() {
        match bones[i].parent.map(|p| p as usize) {
            Some(p) => {
                let (pt, pr) = (bones[p].world_translation, bones[p].world_rotation);
                let r = quat_rotate(pr, bones[i].bind_translation);
                bones[i].world_translation = [pt[0] + r[0], pt[1] + r[1], pt[2] + r[2]];
                bones[i].world_rotation = quat_multiply(pr, bones[i].bind_rotation);
            }
            None => {
                bones[i].world_translation = bones[i].bind_translation;
                bones[i].world_rotation = bones[i].bind_rotation;
            }
        }
    }
}

/// Rotate `v` by unit quaternion `q` ([x, y, z, w]).
pub(crate) fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let p = quat_multiply(quat_multiply(q, [v[0], v[1], v[2], 0.0]), quat_conjugate(q));
    [p[0], p[1], p[2]]
}

/// Conjugate (= inverse for unit quaternions) of `q` ([x, y, z, w]).
pub(crate) fn quat_conjugate(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Convert an Euler XYZ rotation (radians) to a quaternion [x, y, z, w].
///
/// Standard formula: q = qx * qy * qz where each qi is the half-angle
/// quaternion about axis i. Matches the engine's sin/cos chain at Ghidra
/// `0x1466e4dbf` (Pass 7 §6.9.5).
pub(crate) fn euler_xyz_to_quat(eu: [f32; 3]) -> [f32; 4] {
    let (sx, cx) = ((eu[0] * 0.5).sin(), (eu[0] * 0.5).cos());
    let (sy, cy) = ((eu[1] * 0.5).sin(), (eu[1] * 0.5).cos());
    let (sz, cz) = ((eu[2] * 0.5).sin(), (eu[2] * 0.5).cos());
    [
        sx * cy * cz + cx * sy * sz, // x
        cx * sy * cz - sx * cy * sz, // y
        cx * cy * sz + sx * sy * cz, // z
        cx * cy * cz - sx * sy * sz, // w
    ]
}

/// Multiply two quaternions [x, y, z, w] in Hamilton convention: a * b.
pub(crate) fn quat_multiply(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by, // x
        aw * by - ax * bz + ay * bw + az * bx, // y
        aw * bz + ax * by - ay * bx + az * bw, // z
        aw * bw - ax * bx - ay * by - az * bz, // w
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::head_skin::Bone;
    use crate::riglogic::{
        BlendShapeBlob, RigLogic, RigType,
    };

    /// Build a synthetic Bone with the given name at zero translation.
    fn synthetic_bone(name: &str, parent: Option<u32>) -> Bone {
        let crc = crc32fast::hash(name.as_bytes());
        let crc_lower = crc32fast::hash(name.to_ascii_lowercase().as_bytes());
        Bone {
            name: name.into(),
            name_crc: crc,
            name_crc_lower: crc_lower,
            parent,
            bind_translation: [0.0, 0.0, 0.0],
            bind_rotation: [0.0, 0.0, 0.0, 1.0],
            world_translation: [0.0, 0.0, 0.0],
            world_rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    /// Build a minimal `RigLogic` whose only joint entry matches `bone_name`.
    ///
    /// All fields that `apply_deltas` does not consult are set to their
    /// simplest legal values. `RigLogic` doesn't derive `Default` and has many
    /// fields (mostly `Vec`s with natural empty defaults), so this helper
    /// hand-constructs the struct directly rather than adding a blanket
    /// `Default` derive — that would be a separate refactor touching multiple
    /// `#[repr(C, packed)]` types.
    fn synthetic_rig_one_joint(bone_name: &str) -> RigLogic {
        let crc = crc32fast::hash(bone_name.as_bytes());
        RigLogic {
            file_size: 0,
            name: "test_rig".into(),
            rig_type: RigType {
                class: 1,
                gender: 1,
                variant: 0,
                part: 3,
                tier: 1,
                lod: 0,
                flags_or_pad: 0,
            },
            identity_crc_lo: NameHash(0),
            identity_crc_hi_or_x: 0,
            header_size: 0xA8,
            num_appearance_blendshapes: 0,
            num_corrective_blendshapes: 0,
            num_joints: 1,
            _reserved_0x46: 0,
            primary_delta_scale: 1.0,
            secondary_delta_scale: 1.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: 22,
            section_starts: [0u64; 8],
            joint_control_name_crcs: vec![],
            joint_control_names: vec![],
            appearance: BlendShapeBlob {
                records: vec![],
                scale: 1.0,
            },
            corrective: BlendShapeBlob {
                records: vec![],
                scale: 1.0,
            },
            corrective_composition: vec![],
            skeleton_bone_name_crcs: vec![crc],
            blend_region_bones: vec![],
            region_weights: vec![],
        }
    }

    /// Build a JointDeltas with the given translation, zero rotation.
    fn translation_only(translation: Vec<[f32; 3]>) -> JointDeltas {
        let n = translation.len();
        JointDeltas {
            translation,
            rotation_eu: vec![[0.0_f32; 3]; n],
        }
    }

    #[test]
    fn zero_deltas_preserves_rest_pose() {
        let rest = BindPoseSkeleton {
            bones: vec![synthetic_bone("FACIAL_LOD0_C_AdamsApple", None)],
        };
        let rig = synthetic_rig_one_joint("FACIAL_LOD0_C_AdamsApple");
        let result = apply_deltas(&rig, &translation_only(vec![[0.0, 0.0, 0.0]]), &rest)
            .expect("zero deltas should apply without error");
        assert_eq!(
            result.bones[0].bind_translation,
            [0.0, 0.0, 0.0],
            "bind_translation must be unchanged for zero delta"
        );
    }

    #[test]
    fn nonzero_delta_modifies_bind_translation() {
        let rest = BindPoseSkeleton {
            bones: vec![synthetic_bone("FACIAL_LOD0_C_AdamsApple", None)],
        };
        let rig = synthetic_rig_one_joint("FACIAL_LOD0_C_AdamsApple");
        let result = apply_deltas(&rig, &translation_only(vec![[1.0, 2.0, 3.0]]), &rest)
            .expect("matching CRC should apply");
        assert_eq!(
            result.bones[0].bind_translation,
            [1.0, 2.0, 3.0],
            "bind_translation must reflect the applied delta"
        );
    }

    #[test]
    fn world_translation_is_not_modified() {
        // apply_deltas only updates bind_translation; world_translation stays
        // at rest. Phase 6 can recompute world transforms by chaining locals.
        let rest = BindPoseSkeleton {
            bones: vec![synthetic_bone("FACIAL_LOD0_C_AdamsApple", None)],
        };
        let rig = synthetic_rig_one_joint("FACIAL_LOD0_C_AdamsApple");
        let result = apply_deltas(&rig, &translation_only(vec![[1.0, 2.0, 3.0]]), &rest).unwrap();
        assert_eq!(
            result.bones[0].world_translation,
            [0.0, 0.0, 0.0],
            "world_translation must not be modified by apply_deltas"
        );
    }

    #[test]
    fn unmatched_crc_returns_joint_mapping_error() {
        let rest = BindPoseSkeleton {
            // "root" CRC won't match the rig's "FACIAL_LOD0_C_AdamsApple" CRC.
            bones: vec![synthetic_bone("root", None)],
        };
        let rig = synthetic_rig_one_joint("FACIAL_LOD0_C_AdamsApple");
        let result = apply_deltas(&rig, &translation_only(vec![[1.0, 0.0, 0.0]]), &rest);
        assert!(
            matches!(result, Err(CharacterError::JointMappingFailed { rig_joint_idx: 0, .. })),
            "expected JointMappingFailed at index 0, got: {result:?}"
        );
    }

    #[test]
    fn wrong_deltas_length_returns_rbf_error() {
        let rest = BindPoseSkeleton {
            bones: vec![synthetic_bone("root", None)],
        };
        let rig = synthetic_rig_one_joint("root"); // num_joints = 1
        // Pass empty deltas — length 0 != 1.
        let result = apply_deltas(&rig, &translation_only(vec![]), &rest);
        assert!(
            matches!(result, Err(CharacterError::Rbf(_))),
            "expected Rbf length-mismatch error, got: {result:?}"
        );
    }

    #[test]
    fn bones_outside_rig_stay_at_rest() {
        // Three bones in skeleton; only the second is in the rig.
        // The first and third must remain at rest pose.
        let rest = BindPoseSkeleton {
            bones: vec![
                synthetic_bone("World", None),
                synthetic_bone("FACIAL_LOD0_C_AdamsApple", Some(0)),
                synthetic_bone("Spine3", Some(0)),
            ],
        };
        let rig = synthetic_rig_one_joint("FACIAL_LOD0_C_AdamsApple");
        let result = apply_deltas(&rig, &translation_only(vec![[5.0, 5.0, 5.0]]), &rest)
            .expect("should succeed");
        // Only bone[1] is modified.
        assert_eq!(result.bones[0].bind_translation, [0.0, 0.0, 0.0], "World unchanged");
        assert_eq!(result.bones[1].bind_translation, [5.0, 5.0, 5.0], "FACIAL updated");
        assert_eq!(result.bones[2].bind_translation, [0.0, 0.0, 0.0], "Spine3 unchanged");
    }

    #[test]
    fn zero_rotation_preserves_bind_rotation() {
        // Identity bind rotation [0, 0, 0, 1]; zero Euler delta → unchanged.
        let rest = BindPoseSkeleton {
            bones: vec![synthetic_bone("root", None)],
        };
        let rig = synthetic_rig_one_joint("root");
        let deltas = JointDeltas {
            translation: vec![[0.0, 0.0, 0.0]],
            rotation_eu: vec![[0.0, 0.0, 0.0]],
        };
        let result = apply_deltas(&rig, &deltas, &rest).expect("ok");
        assert_eq!(result.bones[0].bind_rotation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn single_axis_rotation_composes_correctly() {
        // Identity bind, π/2 rotation about X.
        // Expected quat: (sin(π/4), 0, 0, cos(π/4)) ≈ (0.7071, 0, 0, 0.7071).
        let rest = BindPoseSkeleton {
            bones: vec![synthetic_bone("root", None)],
        };
        let rig = synthetic_rig_one_joint("root");
        let deltas = JointDeltas {
            translation: vec![[0.0, 0.0, 0.0]],
            rotation_eu: vec![[std::f32::consts::FRAC_PI_2, 0.0, 0.0]],
        };
        let result = apply_deltas(&rig, &deltas, &rest).expect("ok");
        let q = result.bones[0].bind_rotation;
        let half_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
        assert!((q[0] - half_sqrt2).abs() < 1e-5, "q[0] = {}", q[0]);
        assert!((q[1] - 0.0).abs() < 1e-5, "q[1] = {}", q[1]);
        assert!((q[2] - 0.0).abs() < 1e-5, "q[2] = {}", q[2]);
        assert!((q[3] - half_sqrt2).abs() < 1e-5, "q[3] = {}", q[3]);
    }

    #[test]
    fn rotation_composes_with_existing_bind() {
        // Non-identity bind rotation (90° about Y), apply 90° delta about X.
        // Expected: bind_q * delta_q.
        let mut bone = synthetic_bone("root", None);
        let half_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
        bone.bind_rotation = [0.0, half_sqrt2, 0.0, half_sqrt2]; // 90° about Y

        let rest = BindPoseSkeleton { bones: vec![bone] };
        let rig = synthetic_rig_one_joint("root");
        let deltas = JointDeltas {
            translation: vec![[0.0, 0.0, 0.0]],
            rotation_eu: vec![[std::f32::consts::FRAC_PI_2, 0.0, 0.0]],
        };
        let result = apply_deltas(&rig, &deltas, &rest).expect("ok");
        // Manually compute expected: q_bind = [0, s, 0, s] where s = 0.7071;
        // q_delta = [s, 0, 0, s].
        // q_bind * q_delta:
        //   x = s * s + 0 * s + s * 0 - 0 * 0 = s²       = 0.5
        //   y = s * 0 - 0 * 0 + s * s + 0 * s = s²       = 0.5
        //   z = s * 0 + 0 * 0 - s * s + 0 * s = -s²      = -0.5
        //   w = s * s - 0 * s - s * 0 - 0 * 0 = s²       = 0.5
        let q = result.bones[0].bind_rotation;
        assert!((q[0] - 0.5).abs() < 1e-5, "q[0] = {}", q[0]);
        assert!((q[1] - 0.5).abs() < 1e-5, "q[1] = {}", q[1]);
        assert!((q[2] - (-0.5)).abs() < 1e-5, "q[2] = {}", q[2]);
        assert!((q[3] - 0.5).abs() < 1e-5, "q[3] = {}", q[3]);
    }

    // ── rebind_skeleton / refresh_world_from_locals ──────────────────────

    /// root at origin rotated 90° about Z; `a` one unit along root-local +X
    /// (= world +Y); `c` one unit along a-local +X (= world (0,2,0)).
    fn rotated_chain() -> BindPoseSkeleton {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let rz90 = [0.0, 0.0, s, s];
        let mut root = synthetic_bone("root", None);
        root.bind_rotation = rz90;
        root.world_rotation = rz90;
        let mut a = synthetic_bone("a", Some(0));
        a.bind_translation = [1.0, 0.0, 0.0];
        a.world_translation = [0.0, 1.0, 0.0];
        a.world_rotation = rz90;
        let mut c = synthetic_bone("c", Some(1));
        c.bind_translation = [1.0, 0.0, 0.0];
        c.world_translation = [0.0, 2.0, 0.0];
        c.world_rotation = rz90;
        BindPoseSkeleton { bones: vec![root, a, c] }
    }

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5)
    }

    #[test]
    fn refresh_world_reproduces_consistent_rest() {
        let rest = rotated_chain();
        let mut bones = rest.bones.clone();
        refresh_world_from_locals(&mut bones);
        for (o, r) in bones.iter().zip(&rest.bones) {
            assert!(close3(o.world_translation, r.world_translation), "{}", r.name);
        }
    }

    #[test]
    fn rebind_moves_world_by_parent_rotated_local_delta() {
        let rest = rotated_chain();
        let rig = synthetic_rig_one_joint("a");
        let deltas = JointDeltas {
            translation: vec![[0.5, 0.0, 0.0]],
            rotation_eu: vec![[0.0; 3]],
        };
        let out = rebind_skeleton(&rig, &deltas, &rest).expect("ok");
        // Local +X under the 90°-Z root is world +Y.
        assert!(close3(out.bones[1].bind_translation, [1.5, 0.0, 0.0]));
        assert!(close3(out.bones[1].world_translation, [0.0, 1.5, 0.0]), "{:?}", out.bones[1].world_translation);
        // Child follows its parent (local unchanged, world shifted).
        assert!(close3(out.bones[2].bind_translation, [1.0, 0.0, 0.0]));
        assert!(close3(out.bones[2].world_translation, [0.0, 2.5, 0.0]));
        // Root untouched.
        assert!(close3(out.bones[0].world_translation, [0.0; 3]));
    }

    #[test]
    fn rebind_with_zero_deltas_preserves_world() {
        let rest = rotated_chain();
        let rig = synthetic_rig_one_joint("a");
        let deltas = JointDeltas {
            translation: vec![[0.0; 3]],
            rotation_eu: vec![[0.0; 3]],
        };
        let out = rebind_skeleton(&rig, &deltas, &rest).expect("ok");
        for (o, r) in out.bones.iter().zip(&rest.bones) {
            assert!(close3(o.world_translation, r.world_translation));
        }
    }

    #[test]
    fn rebind_propagates_mapping_error() {
        let rest = rotated_chain();
        let rig = synthetic_rig_one_joint("missing");
        let deltas = JointDeltas {
            translation: vec![[0.0; 3]],
            rotation_eu: vec![[0.0; 3]],
        };
        assert!(matches!(
            rebind_skeleton(&rig, &deltas, &rest),
            Err(CharacterError::JointMappingFailed { .. })
        ));
    }
}
