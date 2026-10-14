//! Rig data carried by the shared [`crate::types::Mesh`]: skinning and morph
//! targets, plus the per-vertex operations every copy site needs.
//!
//! - [`Skin`]: up to [`MAX_INFLUENCES`] joints per vertex. Joints index the
//!   mesh's own `joint_names` table (each `.skin` numbers joints against its
//!   own bone table), so writers map them onto the entity skeleton by name.
//!   All-zero weights mean "no influence: follows the mesh node".
//! - [`MorphTarget`]: a named set of per-vertex position (and optionally
//!   normal) deltas in mesh space.
//! - [`merge_skins`] / [`merge_morph_targets`]: append another mesh's rig data,
//!   unioning joints and targets by name.
//! - [`Skin::gather`] / [`MorphTarget::gather`]: per-vertex subsetting.

/// Maximum joint influences per vertex (`IVOBONEMAP32` carries 8).
pub const MAX_INFLUENCES: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    /// Joint names; `joints` indexes this table.
    pub joint_names: Vec<String>,
    pub joints: Vec<[u16; MAX_INFLUENCES]>,
    /// Normalised weights (sum 1 for an influenced vertex, all 0 for none).
    pub weights: Vec<[f32; MAX_INFLUENCES]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MorphTarget {
    pub name: String,
    pub position_deltas: Vec<[f32; 3]>,
    pub normal_deltas: Option<Vec<[f32; 3]>>,
}

impl Skin {
    /// The entries for `order` (old vertex indices), in that order.
    pub fn gather(&self, order: &[u32]) -> Skin {
        Skin {
            joint_names: self.joint_names.clone(),
            joints: order.iter().map(|&o| self.joints[o as usize]).collect(),
            weights: order.iter().map(|&o| self.weights[o as usize]).collect(),
        }
    }

    /// `count` vertices fully bound to one joint (a rigid attachment).
    pub fn rigid(joint_name: &str, count: usize) -> Skin {
        let mut weights = [0.0; MAX_INFLUENCES];
        weights[0] = 1.0;
        Skin { joint_names: vec![joint_name.to_owned()], joints: vec![[0; MAX_INFLUENCES]; count], weights: vec![weights; count] }
    }

    /// `count` vertices without influence.
    fn unweighted(joint_names: Vec<String>, count: usize) -> Skin {
        Skin { joint_names, joints: vec![[0; MAX_INFLUENCES]; count], weights: vec![[0.0; MAX_INFLUENCES]; count] }
    }
}

impl MorphTarget {
    /// The deltas for `order` (old vertex indices), in that order.
    pub fn gather(&self, order: &[u32]) -> MorphTarget {
        MorphTarget {
            name: self.name.clone(),
            position_deltas: order.iter().map(|&o| self.position_deltas[o as usize]).collect(),
            normal_deltas: self
                .normal_deltas
                .as_ref()
                .map(|n| order.iter().map(|&o| n[o as usize]).collect()),
        }
    }
}

/// Append `theirs` (for `their_count` vertices) to `mine` (for `my_count`).
/// Joint tables are unioned by name and `theirs` is remapped; a side without
/// a skin gets vertices without influence.
pub fn merge_skins(
    mine: Option<Skin>,
    theirs: Option<Skin>,
    my_count: usize,
    their_count: usize,
) -> Option<Skin> {
    let (mut merged, theirs) = match (mine, theirs) {
        (None, None) => return None,
        (Some(mine), theirs) => (mine, theirs),
        (None, Some(theirs)) => (Skin::unweighted(Vec::new(), my_count), Some(theirs)),
    };
    let Some(theirs) = theirs else {
        let names = merged.joint_names.clone();
        let pad = Skin::unweighted(names, their_count);
        merged.joints.extend(pad.joints);
        merged.weights.extend(pad.weights);
        return Some(merged);
    };
    let remap: Vec<u16> = theirs
        .joint_names
        .iter()
        .map(|name| match merged.joint_names.iter().position(|n| n == name) {
            Some(index) => index as u16,
            None => {
                merged.joint_names.push(name.clone());
                (merged.joint_names.len() - 1) as u16
            }
        })
        .collect();
    for (joints, weights) in theirs.joints.into_iter().zip(theirs.weights) {
        let mut remapped = [0u16; MAX_INFLUENCES];
        for slot in 0..MAX_INFLUENCES {
            if weights[slot] > 0.0 {
                remapped[slot] = remap[joints[slot] as usize];
            }
        }
        merged.joints.push(remapped);
        merged.weights.push(weights);
    }
    Some(merged)
}

/// Append `theirs` to `mine` by target name; a side without a target gets
/// zero deltas for its vertices.
pub fn merge_morph_targets(
    mine: &mut Vec<MorphTarget>,
    theirs: Vec<MorphTarget>,
    my_count: usize,
    their_count: usize,
) {
    let zero = |n: usize| vec![[0.0f32; 3]; n];
    let mut theirs: Vec<Option<MorphTarget>> = theirs.into_iter().map(Some).collect();
    for target in mine.iter_mut() {
        let incoming = theirs
            .iter_mut()
            .find(|t| t.as_ref().is_some_and(|t| t.name == target.name))
            .and_then(Option::take);
        let (positions, normals) = match incoming {
            Some(t) => (t.position_deltas, t.normal_deltas),
            None => (zero(their_count), None),
        };
        target.position_deltas.extend(positions);
        target.normal_deltas = match (target.normal_deltas.take(), normals) {
            (None, None) => None,
            (mine_n, theirs_n) => {
                let mut n = mine_n.unwrap_or_else(|| zero(my_count));
                n.extend(theirs_n.unwrap_or_else(|| zero(their_count)));
                Some(n)
            }
        };
    }
    for target in theirs.into_iter().flatten() {
        let mut positions = zero(my_count);
        positions.extend(target.position_deltas);
        let normal_deltas = target.normal_deltas.map(|n| {
            let mut padded = zero(my_count);
            padded.extend(n);
            padded
        });
        mine.push(MorphTarget { name: target.name, position_deltas: positions, normal_deltas });
    }
}
