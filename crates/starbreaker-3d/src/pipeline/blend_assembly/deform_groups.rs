//! Blender vertex groups (`bDeformGroup` + `MDeformVert` / `MDeformWeight`)
//! for one mesh object: decal masks plus one group per skin joint.
//!
//! - [`deform_groups`]: combine membership-only [`VertexGroup`]s (weight 1)
//!   with `Mesh::skin` (one group per joint name, the skin's weights) into
//!   per-vertex weight lists, in one pass.
//! - [`DeformGroupPtrs::allocate`] / [`write_deform_groups`]: the SDNA blocks,
//!   shared by the flat and NMC-hierarchy mesh writers.

use starbreaker_blend::{
    build_bdeformgroup, build_custom_data_layer_mdeformvert, build_mdeformvert_array,
    build_mdeformweight_array, write_block, PtrAlloc, SDNA_IDX_ATTRIBUTE_ARRAY, SDNA_IDX_BDEFORMGROUP,
    SDNA_IDX_MDEFORMVERT,
};

use super::VertexGroup;
use crate::mesh_rig::Skin;

/// Group names and, per vertex, its `(group index, weight)` pairs.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DeformGroups {
    pub names: Vec<String>,
    pub per_vertex: Vec<Vec<(u32, f32)>>,
}

/// Decal masks (weight 1) followed by one group per skin joint; `None` when
/// the mesh has neither.
pub(super) fn deform_groups(
    groups: Option<&Vec<VertexGroup>>,
    skin: Option<&Skin>,
    totvert: usize,
) -> Option<DeformGroups> {
    let groups = groups.map(Vec::as_slice).unwrap_or_default();
    if groups.is_empty() && skin.is_none() {
        return None;
    }
    let mut names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
    let mut per_vertex: Vec<Vec<(u32, f32)>> = vec![Vec::new(); totvert];
    for (index, group) in groups.iter().enumerate() {
        for &v in &group.vertex_indices {
            if let Some(slot) = per_vertex.get_mut(v) {
                slot.push((index as u32, 1.0));
            }
        }
    }
    if let Some(skin) = skin {
        let base = names.len() as u32;
        names.extend(skin.joint_names.iter().cloned());
        for (v, (joints, weights)) in skin.joints.iter().zip(&skin.weights).enumerate().take(totvert) {
            for (&joint, &weight) in joints.iter().zip(weights) {
                if weight > 0.0 {
                    per_vertex[v].push((base + joint as u32, weight));
                }
            }
        }
    }
    Some(DeformGroups { names, per_vertex })
}

/// Pointers for one mesh's vertex-group blocks.
#[derive(Debug, Clone, Default)]
pub(super) struct DeformGroupPtrs {
    pub group_ptrs: Vec<u64>,
    /// (`MDeformVert` attribute array, `MDeformVert` data)
    pub mdeformvert: (u64, u64),
    /// One `MDeformWeight[]` block per vertex.
    pub weight_ptrs: Vec<u64>,
    pub custom_data_layer: u64,
}

impl DeformGroupPtrs {
    pub fn allocate(ptrs: &mut PtrAlloc, groups: &DeformGroups) -> DeformGroupPtrs {
        DeformGroupPtrs {
            group_ptrs: groups.names.iter().map(|_| ptrs.alloc()).collect(),
            mdeformvert: (ptrs.alloc(), ptrs.alloc()),
            weight_ptrs: groups.per_vertex.iter().map(|_| ptrs.alloc()).collect(),
            custom_data_layer: ptrs.alloc(),
        }
    }

    pub fn first(&self) -> u64 {
        self.group_ptrs.first().copied().unwrap_or(0)
    }

    pub fn last(&self) -> u64 {
        self.group_ptrs.last().copied().unwrap_or(0)
    }
}

/// Write the `bDeformGroup` list, the `MDeformVert` array (one weight block
/// per influenced vertex, holding all its weights) and its custom-data layer.
pub(super) fn write_deform_groups(out: &mut Vec<u8>, groups: &DeformGroups, ptrs: &DeformGroupPtrs) {
    for (index, name) in groups.names.iter().enumerate() {
        let next = ptrs.group_ptrs.get(index + 1).copied().unwrap_or(0);
        let prev = if index > 0 { ptrs.group_ptrs[index - 1] } else { 0 };
        write_block(out, b"DATA", SDNA_IDX_BDEFORMGROUP, ptrs.group_ptrs[index], 1, &build_bdeformgroup(name, next, prev));
    }
    let mut verts = Vec::with_capacity(groups.per_vertex.len());
    let mut payloads = Vec::new();
    for (v, weights) in groups.per_vertex.iter().enumerate() {
        if weights.is_empty() {
            verts.push((0, 0));
        } else {
            verts.push((ptrs.weight_ptrs[v], weights.len() as u32));
            payloads.push((ptrs.weight_ptrs[v], build_mdeformweight_array(weights)));
        }
    }
    let (array_ptr, data_ptr) = ptrs.mdeformvert;
    let array = build_mdeformvert_array(&verts);
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_ptr, 1, &array);
    for (ptr, data) in payloads {
        write_block(out, b"DATA", 0, ptr, 1, &data);
    }
    write_block(out, b"DATA", SDNA_IDX_MDEFORMVERT, data_ptr, groups.per_vertex.len() as u32, &array);
    write_block(out, b"DATA", 0, ptrs.custom_data_layer, 1, &build_custom_data_layer_mdeformvert(data_ptr));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_rig::MAX_INFLUENCES;

    fn skin(names: &[&str], per_vertex: &[&[(u16, f32)]]) -> Skin {
        let mut joints = Vec::new();
        let mut weights = Vec::new();
        for pairs in per_vertex {
            let mut j = [0u16; MAX_INFLUENCES];
            let mut w = [0f32; MAX_INFLUENCES];
            for (slot, &(joint, weight)) in pairs.iter().enumerate() {
                j[slot] = joint;
                w[slot] = weight;
            }
            joints.push(j);
            weights.push(w);
        }
        Skin { joint_names: names.iter().map(|s| s.to_string()).collect(), joints, weights }
    }

    #[test]
    fn no_groups_and_no_skin_means_no_deform_data() {
        assert_eq!(deform_groups(None, None, 3), None);
        assert_eq!(deform_groups(Some(&Vec::new()), None, 3), None);
    }

    #[test]
    fn decal_groups_are_weight_one_memberships() {
        let groups = vec![VertexGroup { name: "decal".into(), vertex_indices: vec![0, 2] }];
        let out = deform_groups(Some(&groups), None, 3).unwrap();
        assert_eq!(out.names, vec!["decal"]);
        assert_eq!(out.per_vertex, vec![vec![(0, 1.0)], vec![], vec![(0, 1.0)]]);
    }

    #[test]
    fn skin_joints_follow_decal_groups_with_their_weights() {
        let groups = vec![VertexGroup { name: "decal".into(), vertex_indices: vec![1] }];
        let skin = skin(&["root", "arm"], &[&[(0, 1.0)], &[(0, 0.25), (1, 0.75)], &[]]);
        let out = deform_groups(Some(&groups), Some(&skin), 3).unwrap();
        assert_eq!(out.names, vec!["decal", "root", "arm"]);
        assert_eq!(out.per_vertex[0], vec![(1, 1.0)]);
        assert_eq!(out.per_vertex[1], vec![(0, 1.0), (1, 0.25), (2, 0.75)], "a vertex in several groups keeps all weights");
        assert!(out.per_vertex[2].is_empty(), "no influence: no weights (Blender leaves it at rest)");
    }

    #[test]
    fn writer_emits_one_weight_block_per_influenced_vertex() {
        let skin = skin(&["root", "arm"], &[&[(0, 0.5), (1, 0.5)], &[], &[(1, 1.0)]]);
        let groups = deform_groups(None, Some(&skin), 3).unwrap();
        let mut ptrs = PtrAlloc::new(0x1000);
        let block_ptrs = DeformGroupPtrs::allocate(&mut ptrs, &groups);
        let mut out = Vec::new();
        write_deform_groups(&mut out, &groups, &block_ptrs);
        // 2 bDeformGroup + MDeformVert array + 2 weight blocks + MDeformVert data + layer
        let blocks = out.windows(4).filter(|w| *w == b"DATA").count();
        assert_eq!(blocks, 2 + 1 + 2 + 1 + 1);
    }
}
