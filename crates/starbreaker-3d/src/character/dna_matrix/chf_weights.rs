//! Phase 4b.1: CHF DnaBlend → DNA-matrix weight buffer.
//!
//! Mirrors the per-blend accumulate-clamp-normalise path of
//! `HandleDNABlending` (Ghidra anchor `0x144053610`). See
//! `docs/riglogic/PHASE_4B_INVESTIGATION.md` §3.
//!
//! ## Indexing convention
//!
//! Output buffer is row-major `[num_variants × blends_per_pool]` f64:
//! `weights[head_id * blends_per_pool + face_part_idx]`.
//!
//! - `head_id` (= pool index) is read from each `DnaBlend.head_id`.
//! - `face_part_idx` is the **`FacePart` enum value** (`*face_part as u8
//!   as usize`). Each `FacePart` enum value (0..=12) is the column index
//!   directly; columns for face_parts not populated by the CHF stay zero.
//!
//! ## `blends_per_pool` — Pass 6 anchored
//!
//! `mesh_blends_per_pool` is anchored to the `.dna` header's `+0x28`
//! field (13 for human heads). Pass 6 (PHASE_4B_INVESTIGATION §6.8)
//! ruled out a 3 → 13 expansion — the customizer-level dimension IS
//! the mesh dimension. Each `FacePart` enum value (0..=12) is the
//! column index directly; columns beyond CHF's populated face_parts
//! stay zero. The previous BTreeMap-position interpretation is wrong.
//!
//! ## Open caveat — anim_scale / sign
//!
//! The engine multiplies each accumulated value by a per-customizer
//! animated scale (`+0x2b54` f32) and a sign (`+0x2b5c` ±1) before the
//! clamp. For a passive (non-animating) CHF → glb export we treat these
//! as `1.0` / `+1`. Reflected directly in the formula below.

use starbreaker_chf::Dna;
use thiserror::Error;

/// Errors from [`compute_chf_weights`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChfWeightsError {
    /// A `DnaBlend.head_id` was out of range for the supplied
    /// `num_variants`. Strict-everywhere: a stale CHF / .dna pairing is
    /// a real error worth surfacing, not silently skipping.
    #[error("CHF DnaBlend head_id={head_id} >= num_variants={num_variants}")]
    HeadIdOutOfRange { head_id: u16, num_variants: usize },

    /// A CHF DnaBlend's face_part enum value is >= mesh_blends_per_pool.
    /// Shouldn't happen on well-formed data (FacePart enum has 13 values,
    /// mesh_bpp is 13) but is a defensive guard.
    #[error(
        "CHF face_part enum value {face_part} >= mesh_blends_per_pool {mesh_bpp}"
    )]
    FacePartOutOfRange { face_part: u16, mesh_bpp: u16 },
}

/// Compute the DNA-matrix weight buffer from a CHF DnaBlend table.
///
/// Mirrors `HandleDNABlending` (Ghidra `0x144053610`):
///
/// 1. For every `(face_part, DnaBlend)` pair: accumulate
///    `DnaBlend.value as f64 / 65535.0` into
///    `weights[head_id * blends_per_pool + face_part_idx]`, with a
///    per-update clamp to `[0, 1]`. `face_part_idx` = `*face_part as u8
///    as usize` (enum value directly, per Pass 6 anchor §6.8).
/// 2. Per-FACE-PART (column) sum-to-1 normalise: for each face-part column,
///    sum the head weights across variants; if `> 0`, divide each by the
///    sum. (The 4 blend values within one face part already sum to ~65535,
///    so this is a convex blend of the source heads PER face part. Pass 19g
///    correction — was previously per-head_id-row, which scrambled ratios.)
///
/// `mesh_blends_per_pool` is the `.dna` header `+0x28` value (13 for
/// human heads). Returns `HeadIdOutOfRange` if any `DnaBlend.head_id >=
/// num_variants`. Returns `FacePartOutOfRange` if a face_part enum value
/// >= `mesh_blends_per_pool` (defensive guard; shouldn't fire on
/// well-formed data).
pub fn compute_chf_weights(
    dna: &Dna,
    num_variants: usize,
    mesh_blends_per_pool: usize,
) -> Result<Vec<f64>, ChfWeightsError> {
    let blends_per_pool = mesh_blends_per_pool;
    let mut weights = vec![0.0_f64; num_variants * blends_per_pool];

    // Step 1: accumulate + per-update clamp (mirrors HandleDNABlending
    // Path B's in-loop clamp at the call site `weights[i] = clamp(..., 0, 1)`).
    //
    // Index by FacePart enum value (Pass 6 anchor §6.8): each FacePart
    // value (0..=12) maps directly to a column index.
    for (face_part, slots) in dna.face_parts.iter() {
        let face_part_idx = *face_part as u8 as usize;
        if face_part_idx >= blends_per_pool {
            return Err(ChfWeightsError::FacePartOutOfRange {
                face_part: face_part_idx as u16,
                mesh_bpp: blends_per_pool as u16,
            });
        }
        for blend in slots.iter() {
            // Skip zero-valued blends before consulting head_id. Padding slots
            // have `value = 0` and may carry junk in `head_id` — this matches
            // the engine's `HandleDNABlending`, which skips zero-value blends
            // before reading `head_id`.
            if blend.value == 0 {
                continue;
            }
            let head_id = blend.head_id;
            if (head_id as usize) >= num_variants {
                return Err(ChfWeightsError::HeadIdOutOfRange {
                    head_id,
                    num_variants,
                });
            }
            let cell = head_id as usize * blends_per_pool + face_part_idx;
            let increment = blend.value as f64 / 65535.0;
            weights[cell] = (weights[cell] + increment).clamp(0.0, 1.0);
        }
    }

    // Step 2: per-FACE-PART (column) sum-to-1 normalise.
    //
    // CORRECTION (Pass 19g): the convex blend is PER FACE PART over the (up
    // to 4) source heads — the CHF's 4 `value`s within one face part sum to
    // ~65535 (e.g. 19605+12944+32985 = 65534). So each face-part *column*
    // (the head weights for that part) must sum to 1, NOT each head_id row.
    // The previous per-head_id-row normalisation scrambled the 4-head ratios
    // and produced plausible-but-wrong faces.
    for fp in 0..blends_per_pool {
        let sum: f64 = (0..num_variants).map(|v| weights[v * blends_per_pool + fp]).sum();
        if sum > 0.0 {
            for v in 0..num_variants {
                weights[v * blends_per_pool + fp] /= sum;
            }
        }
    }

    Ok(weights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use starbreaker_chf::{Dna, DnaBlend, FacePart};
    use starbreaker_common::NameHash;
    use std::collections::BTreeMap;

    /// Helper: build a `Dna` with the given face_part entries and a
    /// matching `part_count`. Other Dna fields are filler.
    fn make_dna(face_parts: BTreeMap<FacePart, [DnaBlend; 4]>) -> Dna {
        let part_count = face_parts.len() as u16;
        let max_head_id = face_parts
            .values()
            .flatten()
            .map(|b| b.head_id)
            .max()
            .unwrap_or(0);
        Dna {
            raw_bytes: Vec::new(),
            gender_hash: NameHash(0),
            variant_hash: NameHash(0),
            part_count,
            blends_per_part: 4,
            header_unknown: 0,
            max_head_id,
            face_parts,
        }
    }

    /// Helper: build a `[DnaBlend; 4]` from individual `(head_id, value)`
    /// pairs, zero-padding the remaining slots.
    fn blends(slots: &[(u16, u16)]) -> [DnaBlend; 4] {
        let mut out = [DnaBlend { value: 0, head_id: 0 }; 4];
        for (i, &(head_id, value)) in slots.iter().enumerate().take(4) {
            out[i] = DnaBlend { value, head_id };
        }
        out
    }

    #[test]
    fn empty_dna_produces_zero_weights() {
        let dna = make_dna(BTreeMap::new());
        let weights = compute_chf_weights(&dna, 61, 13).expect("empty dna parses");
        assert_eq!(
            weights.len(),
            61 * 13,
            "expected num_variants * mesh_bpp = {} * {} = {}",
            61, 13, 61 * 13
        );
        assert!(
            weights.iter().all(|&w| w == 0.0),
            "empty dna must produce all-zero weights"
        );
    }

    #[test]
    fn single_blend_full_value_normalises_to_one() {
        let mut face_parts = BTreeMap::new();
        face_parts.insert(FacePart::Mouth, blends(&[(0, 65535)]));
        let dna = make_dna(face_parts);

        let weights = compute_chf_weights(&dna, 4, 13).expect("single blend parses");
        assert_eq!(weights.len(), 4 * 13);
        // FacePart::Mouth has enum value 9 → column 9.
        // head_id = 0 → row 0. After accumulate + normalise: weights[0*13 + 9] = 1.0.
        assert!((weights[9] - 1.0).abs() < 1e-12, "weights[9] = {}", weights[9]);
        // Other cells of row 0 stay zero (no other face_part populated).
        for fp in 0..13 {
            if fp == 9 { continue; }
            assert_eq!(weights[fp], 0.0, "row 0 cell {fp} should be 0");
        }
    }

    #[test]
    fn two_heads_blend_50_50_per_face_part() {
        // Mouth face part blends head 0 and head 1 at 32768 each (50/50).
        // Per-FACE-PART normalise: column 9 (Mouth) sums head0+head1 = 1.0 →
        // each head gets 0.5 (a 50/50 blend of the two heads for the mouth).
        let mut face_parts = BTreeMap::new();
        face_parts.insert(FacePart::Mouth, blends(&[(0, 32768), (1, 32768)]));
        let dna = make_dna(face_parts);

        let weights = compute_chf_weights(&dna, 2, 13).expect("two heads parse");
        assert_eq!(weights.len(), 2 * 13);
        assert!((weights[0 * 13 + 9] - 0.5).abs() < 1e-9, "head 0 mouth: {}", weights[0 * 13 + 9]);
        assert!((weights[1 * 13 + 9] - 0.5).abs() < 1e-9, "head 1 mouth: {}", weights[1 * 13 + 9]);
    }

    #[test]
    fn clamp_caps_at_one_before_normalise() {
        // EyebrowLeft (col 0): head 0 gets 3×65535 → clamps to 1.0; head 1
        // gets value 32768 → 0.5 (all in the SAME face-part column). Per-
        // FACE-PART (col 0) normalise: sum = 1.5 → head 0 = 1.0/1.5 ≈ 0.667,
        // head 1 = 0.5/1.5 ≈ 0.333. Without clamp head 0 would be 3.0 →
        // 3.0/3.5 ≈ 0.857 (measurably different).
        let mut face_parts = BTreeMap::new();
        face_parts.insert(
            FacePart::EyebrowLeft,
            blends(&[(0, 65535), (0, 65535), (0, 65535), (1, 32768)]),
        );
        let dna = make_dna(face_parts);

        let weights = compute_chf_weights(&dna, 2, 13).expect("clamp test parses");
        assert_eq!(weights.len(), 2 * 13);

        let head1_raw = 32768.0_f64 / 65535.0; // ≈ 0.5
        let col0_sum = 1.0 + head1_raw;
        let expected_h0 = 1.0 / col0_sum; // clamped numerator
        let expected_h1 = head1_raw / col0_sum;
        assert!(
            (weights[0 * 13 + 0] - expected_h0).abs() < 1e-9,
            "head0 col0 expected ~{expected_h0}, got {}",
            weights[0 * 13 + 0]
        );
        assert!(
            (weights[1 * 13 + 0] - expected_h1).abs() < 1e-9,
            "head1 col0 expected ~{expected_h1}, got {}",
            weights[1 * 13 + 0]
        );

        // Sanity: without the clamp, head0 col0 would be 3.0/3.5 ≈ 0.857.
        assert!(
            (weights[0 * 13 + 0] - 3.0 / 3.5).abs() > 0.1,
            "head0 col0 should be far from the unclamped result; clamp missing"
        );
    }

    #[test]
    fn head_id_out_of_range_errors() {
        let mut face_parts = BTreeMap::new();
        face_parts.insert(FacePart::Mouth, blends(&[(100, 1)]));
        let dna = make_dna(face_parts);

        let result = compute_chf_weights(&dna, 61, 13);
        assert_eq!(
            result,
            Err(ChfWeightsError::HeadIdOutOfRange {
                head_id: 100,
                num_variants: 61
            })
        );
    }

    #[test]
    fn zero_value_blend_with_out_of_range_head_id_is_accepted() {
        // CHF padding slots have `value = 0` and may carry junk `head_id`s
        // (the engine's HandleDNABlending skips zero-value blends before
        // consulting head_id). Verify we accept the same shape.
        let mut face_parts = BTreeMap::new();
        face_parts.insert(
            FacePart::Mouth,
            blends(&[(0, 65535), (9999, 0), (12345, 0), (61, 0)]),
        );
        let dna = make_dna(face_parts);

        let weights = compute_chf_weights(&dna, 61, 13).expect(
            "zero-value padding slots with junk head_ids must not error",
        );
        assert_eq!(weights.len(), 61 * 13);
        // Only the nonzero blend (head_id=0, value=65535) contributes.
        // FacePart::Mouth is enum value 9 → column 9 of row 0.
        assert!((weights[9] - 1.0).abs() < 1e-12, "weights[9] = {}", weights[9]);
        // All other cells of row 0 stay zero.
        for fp in 0..13 {
            if fp == 9 { continue; }
            assert_eq!(weights[fp], 0.0, "col {fp} should be 0");
        }
        // All other pool rows stay zero.
        for pool in 1..61 {
            for fp in 0..13 {
                assert_eq!(weights[pool * 13 + fp], 0.0);
            }
        }
    }

    #[test]
    fn face_part_out_of_range_errors() {
        // Synthetic CHF with FacePart::Neck (enum value 12).
        // mesh_bpp = 12 means valid column range is [0, 12), so Neck is OOR.
        let mut face_parts = BTreeMap::new();
        face_parts.insert(FacePart::Neck, blends(&[(0, 65535)]));
        let dna = make_dna(face_parts);

        let result = compute_chf_weights(&dna, 4, 12);
        assert_eq!(
            result,
            Err(ChfWeightsError::FacePartOutOfRange {
                face_part: 12,
                mesh_bpp: 12,
            })
        );
    }
}
