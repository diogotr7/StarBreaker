//! Slot-21 per-(joint × face-part) mixing matrix reader.
//!
//! Anchored to `FUN_14142a990`'s pointer-fixup pass and `ExecuteHuman`'s
//! `mesh + 0xe8` deref at Ghidra `0x1466e0aa0`. See
//! `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.7 (Pass 5) for the byte
//! walk and row-stochastic property anchor.
//!
//! Layout on disk: `num_joints × mesh_blends_per_pool` f32 values, row-major.
//! Master fixture: 179 × 13 = 2,327 f32 = 9,308 bytes, plus 4 bytes pad
//! (slot span 9,312).

use crate::error::DnaError;
use crate::parser::DnaFile;

/// Pointer-table slot index for the per-(joint × face-part) mixing matrix.
pub const JOINT_WEIGHTS_SLOT: usize = 21;

impl<'a> DnaFile<'a> {
    /// Read the per-(joint × face-part) f32 mixing matrix from slot 21.
    ///
    /// Length of the returned `Vec<f32>` is `num_joints * mesh_blends_per_pool`.
    /// Caller supplies `num_joints` because the `.dna` header doesn't carry
    /// it (it's a RigLogic-level value).
    ///
    /// Returns an OWNED `Vec<f32>` (not a borrowed slice) to avoid pointer-
    /// alignment concerns when reinterpreting the byte slice. One allocation
    /// per call; the caller can cache it.
    ///
    /// # Errors
    ///
    /// - [`DnaError::JointWeightsUnset`] if slot 21 is null. LOD files
    ///   don't carry this slot; only the master `.dna` does.
    /// - [`DnaError::JointWeightsLengthMismatch`] if the slot's span is
    ///   smaller than `num_joints * mesh_blends_per_pool * 4` bytes.
    pub fn joint_weights(&self, num_joints: usize) -> Result<Vec<f32>, DnaError> {
        let mesh_bpp = self.header.mesh_blends_per_pool as usize;
        let needed_floats = num_joints * mesh_bpp;
        let needed_bytes = needed_floats * 4;

        // Find slot 21's segment.
        let segment = self
            .segment_for_slot(JOINT_WEIGHTS_SLOT)
            .ok_or(DnaError::JointWeightsUnset)?;

        let span = self.segment_bytes(segment);
        if span.len() < needed_bytes {
            return Err(DnaError::JointWeightsLengthMismatch {
                num_joints,
                mesh_bpp,
                needed: needed_bytes,
                got: span.len(),
            });
        }

        // Decode `needed_floats` little-endian f32s.
        let mut out = Vec::with_capacity(needed_floats);
        for i in 0..needed_floats {
            let off = i * 4;
            let bytes = [span[off], span[off + 1], span[off + 2], span[off + 3]];
            out.push(f32::from_le_bytes(bytes));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{
        DnaFile, MAGIC, MIN_FILE_SIZE, POINTER_TABLE_OFFSET,
    };

    /// Build a synthetic .dna with slot 21 populated by the given f32 values.
    /// `mesh_blends_per_pool` is written as a single byte at offset 0x28.
    fn build_dna_with_slot21(joint_weights_f32: &[f32], mesh_bpp: u8) -> Vec<u8> {
        let body_off = MIN_FILE_SIZE;
        let payload_bytes = joint_weights_f32.len() * 4;
        let total = body_off + payload_bytes;

        let mut buf = vec![0u8; total];
        buf[..16].copy_from_slice(&MAGIC);
        // num_variants = 1 (must be non-zero to pass parser validation).
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes());
        // mesh_blends_per_pool at 0x28 (u8 field).
        buf[0x28] = mesh_bpp;

        // Slot 21 points at body_off.
        let slot_off = POINTER_TABLE_OFFSET + JOINT_WEIGHTS_SLOT * 8;
        buf[slot_off..slot_off + 8].copy_from_slice(&(body_off as u64).to_le_bytes());

        // Write f32 values into the body.
        for (i, &f) in joint_weights_f32.iter().enumerate() {
            let off = body_off + i * 4;
            buf[off..off + 4].copy_from_slice(&f.to_le_bytes());
        }
        buf
    }

    #[test]
    fn parses_synthetic_joint_weights() {
        // 2 joints × 3 face-parts = 6 floats.
        let values = vec![1.0_f32, 0.5, 0.0, 0.25, 0.75, 0.0];
        let buf = build_dna_with_slot21(&values, 3);
        let dna = DnaFile::read(&buf).expect("parses");

        let weights = dna.joint_weights(2).expect("joint weights ok");
        assert_eq!(weights.len(), 6);
        assert_eq!(weights, values);
    }

    #[test]
    fn joint_weights_unset_errors() {
        // No slot 21 populated.
        let mut buf = vec![0u8; MIN_FILE_SIZE];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes());
        buf[0x28] = 3; // mesh_blends_per_pool (u8 field)

        let dna = DnaFile::read(&buf).expect("parses");
        let err = dna.joint_weights(2).unwrap_err();
        assert!(matches!(err, DnaError::JointWeightsUnset), "got {err:?}");
    }

    #[test]
    fn joint_weights_length_mismatch_errors() {
        // Populate slot 21 with only 2 floats but ask for 6.
        let values = vec![1.0_f32, 0.5];
        let buf = build_dna_with_slot21(&values, 3);
        let dna = DnaFile::read(&buf).expect("parses");

        let err = dna.joint_weights(2).unwrap_err();
        match err {
            DnaError::JointWeightsLengthMismatch {
                num_joints,
                mesh_bpp,
                needed,
                got,
            } => {
                assert_eq!(num_joints, 2);
                assert_eq!(mesh_bpp, 3);
                assert_eq!(needed, 24); // 6 f32 = 24 bytes
                assert_eq!(got, 8); // 2 f32 = 8 bytes
            }
            other => panic!("expected JointWeightsLengthMismatch, got {other:?}"),
        }
    }

    /// Master `.dna` fixture: verify slot 21 byte-walk gives a length matching
    /// `num_joints (179) × mesh_blends_per_pool (13) = 2327` and row-stochastic
    /// property holds for the first row.
    #[test]
    fn master_dna_joint_weights_row_stochastic() {
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_DNA) else { return };
        let dna = DnaFile::read(&bytes).expect("master .dna parses");

        let num_joints = 179usize;
        let mesh_bpp = dna.header.mesh_blends_per_pool as usize;
        assert_eq!(mesh_bpp, 13, "master mesh_bpp must equal 13");

        let weights = dna.joint_weights(num_joints).expect("master has slot 21");
        assert_eq!(weights.len(), num_joints * mesh_bpp);

        // Row-stochastic property (anchored in §6.7): every non-zero row must
        // sum to ~1.0. Row 0 is all zeros for the master (the root joint has no
        // blend-region participation). Row 1 is the first non-zero row.
        // Verify: every row with a non-zero sum actually sums to ~1.0.
        let mut found_nonzero_row = false;
        for row in 0..num_joints {
            let row_sum: f32 = weights[row * mesh_bpp..(row + 1) * mesh_bpp].iter().sum();
            if row_sum.abs() > 1e-6 {
                found_nonzero_row = true;
                assert!(
                    (row_sum - 1.0).abs() < 1e-5,
                    "row {row} sum = {row_sum}, expected ~1.0 (row-stochastic)"
                );
            }
        }
        assert!(found_nonzero_row, "no non-zero rows found — matrix appears empty");
    }
}
