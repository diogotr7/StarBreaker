//! Named-control input for the facial rig.
//!
//! Translates user-friendly control names (e.g. `"CTL_jaw_open"`,
//! `"CTL_smile_left"`) into the per-appearance-blendshape weight vector
//! that `ExecuteHuman`'s external-controls branch (Pass 17 §6.15.3)
//! receives. The vector length is `rig.num_appearance_blendshapes` —
//! it's the LOW half of the full `_Dst[]` activation buffer; the high
//! half (the corrective slots `num_appearance..num_appearance+num_corrective`)
//! is computed downstream by Section C composition (see
//! [`crate::riglogic::compose_correctives`]).
//!
//! ## Baseline
//!
//! [`ControlInputs::rest_pose`] seeds `CTL_PSD` (Section A index 0) to
//! `1.0` and every other control to `0.0`. Pass 9 anchored that the
//! engine unconditionally sets `_Dst[0] = 1.0` after both branches of
//! ExecuteHuman finish populating activations — `CTL_PSD` is the
//! always-on baseline that the Section C composition table treats as
//! a "no-op factor of 1.0" when a row references it.

use thiserror::Error;

use crate::riglogic::RigLogic;

/// Errors from [`ControlInputs::set`] / [`ControlInputs::set_idx`].
#[derive(Debug, Error, PartialEq)]
pub enum ControlError {
    #[error("unknown control name '{name}' (rig has {available} appearance controls)")]
    UnknownName { name: String, available: usize },

    #[error(
        "control index {idx} out of range (num_appearance_blendshapes = {num_appearance})"
    )]
    IndexOutOfRange { idx: usize, num_appearance: usize },
}

/// Per-appearance-control weight buffer, sized at construction to
/// `rig.num_appearance_blendshapes`.
///
/// Index 0 (`CTL_PSD`) is seeded to `1.0` by [`Self::rest_pose`]. All
/// other slots start at `0.0`. Use [`Self::set`] or [`Self::set_idx`]
/// to drive specific controls.
#[derive(Debug)]
pub struct ControlInputs<'rig> {
    rig: &'rig RigLogic,
    weights: Vec<f32>,
}

impl<'rig> ControlInputs<'rig> {
    /// New control buffer with `CTL_PSD` (index 0) = `1.0` and every
    /// other appearance control = `0.0`. Matches the engine's rest-pose
    /// `_Dst[]` shape (Pass 9 §6.11.2).
    pub fn rest_pose(rig: &'rig RigLogic) -> Self {
        let mut weights = vec![0.0_f32; rig.num_appearance_blendshapes as usize];
        if !weights.is_empty() {
            weights[0] = 1.0;
        }
        Self { rig, weights }
    }

    /// Set a control by name. Returns an error if the name isn't in
    /// `rig.joint_control_names`. The match is case-sensitive and
    /// exact — there is no fuzzy lookup.
    pub fn set(&mut self, name: &str, value: f32) -> Result<&mut Self, ControlError> {
        let idx = self
            .rig
            .joint_control_names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| ControlError::UnknownName {
                name: name.to_string(),
                available: self.rig.joint_control_names.len(),
            })?;
        self.weights[idx] = value;
        Ok(self)
    }

    /// Set a control by raw appearance index. Returns an error if
    /// `idx >= num_appearance_blendshapes`.
    pub fn set_idx(&mut self, idx: usize, value: f32) -> Result<&mut Self, ControlError> {
        if idx >= self.weights.len() {
            return Err(ControlError::IndexOutOfRange {
                idx,
                num_appearance: self.weights.len(),
            });
        }
        self.weights[idx] = value;
        Ok(self)
    }

    /// Borrow the appearance-weight slice (length =
    /// `rig.num_appearance_blendshapes`).
    pub fn appearance_weights(&self) -> &[f32] {
        &self.weights
    }

    /// Consume and return the weights buffer. Hand the result to
    /// [`crate::riglogic::compose_correctives`] to obtain the full
    /// `_Dst[]` vector ready for `compute_joint_deltas`.
    pub fn into_appearance_weights(self) -> Vec<f32> {
        self.weights
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riglogic::RigLogic;

    /// Construct a minimal RigLogic by hand with a known joint-control
    /// name list. Avoids depending on the full RigLogic parser in unit
    /// tests — we only exercise the fields ControlInputs reads.
    fn synthetic_rig(names: Vec<&str>) -> RigLogic {
        let num_appearance = names.len() as u16;
        RigLogic {
            file_size: 0,
            name: "synthetic".into(),
            rig_type: crate::riglogic::RigType {
                class: 0,
                gender: 0,
                variant: 0,
                part: 0,
                tier: 0,
                lod: 0,
                flags_or_pad: 0,
            },
            identity_crc_lo: starbreaker_common::NameHash(0),
            identity_crc_hi_or_x: 0,
            header_size: 0xA8,
            num_appearance_blendshapes: num_appearance,
            num_corrective_blendshapes: 0,
            num_joints: 0,
            _reserved_0x46: 0,
            primary_delta_scale: 0.0,
            secondary_delta_scale: 0.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: 9,
            section_starts: [0; 8],
            joint_control_name_crcs: vec![0; names.len()],
            joint_control_names: names.into_iter().map(String::from).collect(),
            appearance: crate::riglogic::BlendShapeBlob { scale: 0.0, records: Vec::new() },
            corrective: crate::riglogic::BlendShapeBlob { scale: 0.0, records: Vec::new() },
            corrective_composition: Vec::new(),
            skeleton_bone_name_crcs: Vec::new(),
            blend_region_bones: Vec::new(),
            region_weights: Vec::new(),
        }
    }

    #[test]
    fn rest_pose_seeds_ctl_psd_at_one() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_jaw_open", "CTL_smile_left"]);
        let inputs = ControlInputs::rest_pose(&rig);
        assert_eq!(inputs.appearance_weights(), &[1.0, 0.0, 0.0]);
    }

    #[test]
    fn rest_pose_with_zero_controls_is_empty() {
        let rig = synthetic_rig(vec![]);
        let inputs = ControlInputs::rest_pose(&rig);
        assert!(inputs.appearance_weights().is_empty());
    }

    #[test]
    fn set_by_name_writes_correct_slot() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_jaw_open", "CTL_smile_left"]);
        let mut inputs = ControlInputs::rest_pose(&rig);
        inputs.set("CTL_jaw_open", 0.75).expect("known name");
        assert_eq!(inputs.appearance_weights(), &[1.0, 0.75, 0.0]);
    }

    #[test]
    fn set_unknown_name_errors() {
        let rig = synthetic_rig(vec!["CTL_PSD"]);
        let mut inputs = ControlInputs::rest_pose(&rig);
        let err = inputs.set("CTL_nonexistent", 0.5).unwrap_err();
        assert_eq!(
            err,
            ControlError::UnknownName {
                name: "CTL_nonexistent".into(),
                available: 1,
            }
        );
    }

    #[test]
    fn set_by_idx_writes_correct_slot() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a", "CTL_b"]);
        let mut inputs = ControlInputs::rest_pose(&rig);
        inputs.set_idx(2, -0.5).expect("in range");
        assert_eq!(inputs.appearance_weights(), &[1.0, 0.0, -0.5]);
    }

    #[test]
    fn set_idx_out_of_range_errors() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a"]);
        let mut inputs = ControlInputs::rest_pose(&rig);
        let err = inputs.set_idx(5, 1.0).unwrap_err();
        assert_eq!(
            err,
            ControlError::IndexOutOfRange { idx: 5, num_appearance: 2 }
        );
    }

    #[test]
    fn fluent_chain_via_explicit_calls() {
        // The mutable-self builder doesn't chain through `?` cleanly,
        // but consecutive set calls work as the documented usage pattern.
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_jaw_open", "CTL_smile_left"]);
        let mut inputs = ControlInputs::rest_pose(&rig);
        inputs.set("CTL_jaw_open", 1.0).expect("ok");
        inputs.set("CTL_smile_left", 0.3).expect("ok");
        assert_eq!(inputs.appearance_weights(), &[1.0, 1.0, 0.3]);
    }

    #[test]
    fn overwrite_replaces_previous_value() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a"]);
        let mut inputs = ControlInputs::rest_pose(&rig);
        inputs.set("CTL_a", 0.5).unwrap();
        inputs.set("CTL_a", 0.9).unwrap();
        assert_eq!(inputs.appearance_weights(), &[1.0, 0.9]);
    }

    #[test]
    fn into_appearance_weights_consumes() {
        let rig = synthetic_rig(vec!["CTL_PSD", "CTL_a"]);
        let mut inputs = ControlInputs::rest_pose(&rig);
        inputs.set("CTL_a", 0.25).unwrap();
        let weights = inputs.into_appearance_weights();
        assert_eq!(weights, vec![1.0_f32, 0.25]);
    }
}
