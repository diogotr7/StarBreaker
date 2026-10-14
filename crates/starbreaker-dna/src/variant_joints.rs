//! Slot-15 per-variant per-joint neutral translation offset reader.
//!
//! Slot 15 holds, for each of `num_variants` identity variants, a Vec3
//! translation OFFSET per RigLogic joint (`num_joints`, 179 for the master
//! — NOT the skin skeleton's 198 bones). Layout on disk: row-major
//! `[variant][joint][xyz]` little-endian f32.
//!
//! Master fixture: 61 × 179 × 3 = 32,757 f32 = 131,028 bytes (exact slot
//! span). See `docs/riglogic/PHASE_4B_INVESTIGATION.md` §1 (slot map, slot
//! 15 row) and §6.16.6 (per-variant neutral-joint identity hypothesis).
//!
//! These offsets are added (after a per-(joint,variant) DNA-matrix fold) to
//! the bind-pose joint translations to produce the per-character NEUTRAL
//! skeleton — the rest-pose facial identity. Values are cm-scale (max
//! ~0.16); the variant-to-variant spread (~0.018) is the identity signal.

use crate::error::DnaError;
use crate::parser::DnaFile;

/// Pointer-table slot index for the per-variant neutral joint offsets.
pub const VARIANT_NEUTRAL_JOINTS_SLOT: usize = 15;

impl<'a> DnaFile<'a> {
    /// Read slot 15 as a flat `Vec<f32>` of length
    /// `num_variants * num_joints * 3`, indexed `[(v*num_joints + j)*3 + axis]`.
    ///
    /// `num_joints` is the RigLogic joint count (the `.dna` header doesn't
    /// carry it); pass `rig.num_joints`.
    ///
    /// # Errors
    /// - [`DnaError::VariantNeutralJointsUnset`] if slot 15 is null (LOD files).
    /// - [`DnaError::VariantNeutralJointsLengthMismatch`] if the span is too small.
    pub fn variant_neutral_joints(&self, num_joints: usize) -> Result<Vec<f32>, DnaError> {
        let num_variants = self.header.num_variants as usize;
        let needed_floats = num_variants * num_joints * 3;
        let needed_bytes = needed_floats * 4;

        let segment = self
            .segment_for_slot(VARIANT_NEUTRAL_JOINTS_SLOT)
            .ok_or(DnaError::VariantNeutralJointsUnset)?;
        let span = self.segment_bytes(segment);
        if span.len() < needed_bytes {
            return Err(DnaError::VariantNeutralJointsLengthMismatch {
                num_variants,
                num_joints,
                needed: needed_bytes,
                got: span.len(),
            });
        }

        let mut out = Vec::with_capacity(needed_floats);
        for i in 0..needed_floats {
            let off = i * 4;
            out.push(f32::from_le_bytes([
                span[off],
                span[off + 1],
                span[off + 2],
                span[off + 3],
            ]));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::{DnaFile, MAGIC, MIN_FILE_SIZE, POINTER_TABLE_OFFSET};

    use super::VARIANT_NEUTRAL_JOINTS_SLOT;

    /// Build a synthetic .dna with `num_variants` and slot 15 populated.
    fn build_dna_with_slot15(num_variants: u16, vals: &[f32]) -> Vec<u8> {
        let body_off = MIN_FILE_SIZE;
        let total = body_off + vals.len() * 4;
        let mut buf = vec![0u8; total];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&num_variants.to_le_bytes());
        let slot_off = POINTER_TABLE_OFFSET + VARIANT_NEUTRAL_JOINTS_SLOT * 8;
        buf[slot_off..slot_off + 8].copy_from_slice(&(body_off as u64).to_le_bytes());
        for (i, &v) in vals.iter().enumerate() {
            buf[body_off + i * 4..body_off + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        buf
    }

    #[test]
    fn reads_slot15_offsets() {
        // 2 variants × 2 joints × 3 = 12 floats.
        let vals: Vec<f32> = (0..12).map(|i| i as f32 * 0.01).collect();
        let buf = build_dna_with_slot15(2, &vals);
        let dna = DnaFile::read(&buf).expect("parse");
        let got = dna.variant_neutral_joints(2).expect("read slot 15");
        assert_eq!(got.len(), 12);
        assert!((got[3] - 0.03).abs() < 1e-6, "v0 joint1 x = {}", got[3]);
        assert!((got[6] - 0.06).abs() < 1e-6, "v1 joint0 x = {}", got[6]);
    }

    #[test]
    fn errors_when_span_too_small() {
        let buf = build_dna_with_slot15(2, &[0.0; 6]); // only 6 floats
        let dna = DnaFile::read(&buf).expect("parse");
        // ask for 2 joints (needs 12) → too small.
        assert!(dna.variant_neutral_joints(2).is_err());
    }
}
