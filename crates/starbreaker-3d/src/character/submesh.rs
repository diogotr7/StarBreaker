//! Skin head sub-meshes (eyes, teeth) onto the per-character posed rig.
//!
//! The CDF sub-meshes use the `ProtosMisc` deformer — i.e. they are SKINNED to
//! the rig skeleton (jaw, skull, eye bones), not surface-wrapped. Each carries
//! its own per-vertex bone weights (4-influence `IVOBONEMAP`, recovered by
//! [`crate::character::head_skin::parse_skin_mesh`]) and its own bind skeleton.
//!
//! We skin each sub-mesh with the head's MODIFIED skeleton, matching the
//! sub-mesh's bones to the head's by name CRC. Because the modified skeleton
//! carries both the per-character identity (slot 15) and any expression joints
//! (slot 31), the teeth/eyes follow identity (mouths that sit higher/forward
//! move the teeth with them) AND expression (the jaw opening drops the lower
//! teeth). This matches what the engine's Protos deformer does.

use std::collections::HashMap;

use crate::character::head_skin::{BindPoseSkeleton, Bone, HeadMesh};

use super::apply::ModifiedSkeleton;

/// Make wrap-deformed sub-meshes (`WD_*`: eye-details, hair, brows, lashes,
/// beard, scalp) follow the head's per-vertex NEUTRAL identity deformation
/// (slot 18), so they sit on the per-character surface instead of floating at
/// the base head's. Each vertex moves by the smooth head-delta field
/// ([`super::wrap::HeadDeltaField`]) sampled at its position.
pub fn wrap_submeshes_to_head_deltas(
    submeshes: &mut [(HeadMesh, BindPoseSkeleton)],
    base_head_positions: &[[f32; 3]],
    head_deltas: &[[f32; 3]],
) {
    let field = super::wrap::HeadDeltaField::new(base_head_positions, head_deltas);
    for (mesh, _) in submeshes.iter_mut() {
        field.apply(&mut mesh.positions);
    }
}

fn qrot(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let t = [
        2.0 * (y * v[2] - z * v[1]),
        2.0 * (z * v[0] - x * v[2]),
        2.0 * (x * v[1] - y * v[0]),
    ];
    [
        v[0] + w * t[0] + (y * t[2] - z * t[1]),
        v[1] + w * t[1] + (z * t[0] - x * t[2]),
        v[2] + w * t[2] + (x * t[1] - y * t[0]),
    ]
}
fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}
fn qconj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// World transforms (rotation, translation) per bone, composed parent-first.
/// Used for the head's modified skeleton (parent-first).
fn world_transforms(bones: &[Bone]) -> Vec<([f32; 4], [f32; 3])> {
    let mut out = Vec::with_capacity(bones.len());
    for b in bones {
        match b.parent {
            None => out.push((b.bind_rotation, b.bind_translation)),
            Some(p) => {
                let (pr, pt) = out[p as usize];
                let wr = qmul(pr, b.bind_rotation);
                let rt = qrot(pr, b.bind_translation);
                out.push((wr, [pt[0] + rt[0], pt[1] + rt[1], pt[2] + rt[2]]));
            }
        }
    }
    out
}

/// Re-point a sub-mesh's bind pose at the head's (re-bound) REST skeleton:
/// every sub-mesh bone whose CRC exists in `rest` takes `rest`'s world
/// transform. A `ProtosMisc` part whose per-character shape comes from its own
/// `.dna` (eyes, teeth) is then skinned RELATIVE to the character's rest pose:
/// unmoved at rest, deformed only by motion on top of it (expressions).
pub fn rebind_submesh_to(skeleton: &mut BindPoseSkeleton, rest: &BindPoseSkeleton) {
    let by_crc: HashMap<u32, &Bone> = rest.bones.iter().map(|b| (b.name_crc, b)).collect();
    for b in &mut skeleton.bones {
        if let Some(r) = by_crc.get(&b.name_crc) {
            b.world_rotation = r.world_rotation;
            b.world_translation = r.world_translation;
        }
    }
}

/// Linear-blend-skin each `(sub-mesh, its bind skeleton)` onto the head's
/// modified skeleton (bones matched by CRC). Returns posed [`HeadMesh`]es
/// (positions updated; other fields preserved).
///
/// A sub-mesh bone whose CRC isn't found in the head skeleton, or a sub-mesh
/// without per-vertex weights, falls back to its bind position (no deform).
pub fn skin_submeshes_to_head(
    submeshes: &[(HeadMesh, BindPoseSkeleton)],
    head_modified: &ModifiedSkeleton,
) -> Vec<HeadMesh> {
    let head_world = world_transforms(&head_modified.bones);
    let crc_to_target: HashMap<u32, ([f32; 4], [f32; 3])> = head_modified
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name_crc, head_world[i]))
        .collect();

    submeshes
        .iter()
        .map(|(mesh, skel)| {
            let has_weights = mesh.joints.len() == mesh.positions.len()
                && mesh.weights.len() == mesh.positions.len()
                && !skel.bones.is_empty();
            if !has_weights {
                return mesh.clone();
            }
            // Sub-mesh bind world transforms (stored — its parent indices may
            // reference the full body skeleton, so don't recompute hierarchy).
            let bind: Vec<([f32; 4], [f32; 3])> = skel
                .bones
                .iter()
                .map(|b| (b.world_rotation, b.world_translation))
                .collect();
            let positions = (0..mesh.positions.len())
                .map(|vi| {
                    let pos = mesh.positions[vi];
                    let mut influences: Vec<(u16, f32)> =
                        mesh.joints[vi].into_iter().zip(mesh.weights[vi]).collect();
                    if let (Some(jh), Some(wh)) = (mesh.joints_hi.get(vi), mesh.weights_hi.get(vi)) {
                        influences.extend(jh.iter().copied().zip(wh.iter().copied()));
                    }
                    let mut acc = [0.0f32; 3];
                    for (j, w) in influences {
                        if w == 0.0 {
                            continue;
                        }
                        let b = j as usize;
                        if b >= bind.len() {
                            continue;
                        }
                        let (br, bt) = bind[b];
                        let (mr, mt) = crc_to_target
                            .get(&skel.bones[b].name_crc)
                            .copied()
                            .unwrap_or((br, bt));
                        let local = [pos[0] - bt[0], pos[1] - bt[1], pos[2] - bt[2]];
                        let pl = qrot(qconj(br), local);
                        let pw = qrot(mr, pl);
                        acc[0] += w * (pw[0] + mt[0]);
                        acc[1] += w * (pw[1] + mt[1]);
                        acc[2] += w * (pw[2] + mt[2]);
                    }
                    acc
                })
                .collect();
            HeadMesh { positions, ..mesh.clone() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::head_skin::Bone;

    fn bone(name: &str, crc: u32, t: [f32; 3]) -> Bone {
        Bone {
            name: name.into(),
            name_crc: crc,
            name_crc_lower: crc,
            parent: None,
            bind_translation: t,
            bind_rotation: [0.0, 0.0, 0.0, 1.0],
            world_translation: t,
            world_rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }
    fn mesh(positions: Vec<[f32; 3]>, joints: Vec<[u16; 4]>, weights: Vec<[f32; 4]>) -> HeadMesh {
        let n = positions.len();
        HeadMesh {
            num_vertices: n as u32,
            num_indices: 0,
            num_submeshes: 1,
            bbox_min: [0.0; 3],
            bbox_max: [0.0; 3],
            positions,
            normals: vec![[0.0, 0.0, 1.0]; n],
            uvs: vec![[0.0, 0.0]; n],
            indices: vec![],
            joints,
            weights,
            joints_hi: vec![],
            weights_hi: vec![],
        }
    }

    #[test]
    fn submesh_follows_matched_bone_translation() {
        // Sub-mesh weighted to bone CRC=7; head's matching bone moved +0.1 X.
        let sub = mesh(vec![[0.0, 0.0, 0.0]], vec![[0, 0, 0, 0]], vec![[1.0, 0.0, 0.0, 0.0]]);
        let sub_skel = BindPoseSkeleton { bones: vec![bone("jaw", 7, [0.0, 0.0, 0.0])] };
        let head_modified = ModifiedSkeleton {
            bones: vec![bone("jaw", 7, [0.1, 0.0, 0.0])],
        };
        let out = skin_submeshes_to_head(&[(sub, sub_skel)], &head_modified);
        let p = out[0].positions[0];
        assert!((p[0] - 0.1).abs() < 1e-5, "x = {}", p[0]);
    }

    #[test]
    fn no_weights_keeps_bind_pose() {
        let sub = mesh(vec![[1.0, 2.0, 3.0]], vec![], vec![]);
        let sub_skel = BindPoseSkeleton { bones: vec![bone("jaw", 7, [0.0; 3])] };
        let head_modified = ModifiedSkeleton { bones: vec![bone("jaw", 7, [0.5; 3])] };
        let out = skin_submeshes_to_head(&[(sub, sub_skel)], &head_modified);
        assert_eq!(out[0].positions[0], [1.0, 2.0, 3.0]);
    }

    /// After re-pointing the sub-mesh bind at the head's re-bound rest
    /// skeleton, skinning by that same rest is the identity (the DNA-shaped
    /// eyes/teeth stay put), and only motion relative to it deforms.
    #[test]
    fn rebound_bind_makes_rest_skinning_identity() {
        let sub = mesh(vec![[0.0, 0.0, 0.0]], vec![[0, 0, 0, 0]], vec![[1.0, 0.0, 0.0, 0.0]]);
        let mut sub_skel = BindPoseSkeleton { bones: vec![bone("jaw", 7, [0.0; 3])] };
        let rest = BindPoseSkeleton { bones: vec![bone("jaw", 7, [0.1, 0.0, 0.0])] };
        rebind_submesh_to(&mut sub_skel, &rest);
        let at_rest = ModifiedSkeleton { bones: rest.bones.clone() };
        let out = skin_submeshes_to_head(&[(sub.clone(), sub_skel.clone())], &at_rest);
        assert!(out[0].positions[0].iter().all(|c| c.abs() < 1e-6), "{:?}", out[0].positions[0]);
        let posed = ModifiedSkeleton { bones: vec![bone("jaw", 7, [0.1, 0.02, 0.0])] };
        let out = skin_submeshes_to_head(&[(sub, sub_skel)], &posed);
        assert!((out[0].positions[0][1] - 0.02).abs() < 1e-6, "{:?}", out[0].positions[0]);
    }

    #[test]
    fn rebind_leaves_unmatched_bones() {
        let mut sub_skel = BindPoseSkeleton { bones: vec![bone("other", 9, [0.3; 3])] };
        let rest = BindPoseSkeleton { bones: vec![bone("jaw", 7, [0.1, 0.0, 0.0])] };
        rebind_submesh_to(&mut sub_skel, &rest);
        assert_eq!(sub_skel.bones[0].world_translation, [0.3; 3]);
    }
}
