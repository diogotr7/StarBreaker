//! Phase 4b.2b: fold the top-K-pruned weight buffer into the 2-D
//! DNA matrix. Mirrors `CFaceRigLogic::SetDNAMatrix`'s
//! sparse-pair-list → `+0xc8` step (Ghidra `0x146735b70`, anchored in
//! `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.6.5).
//!
//! ## Pass 4 correction
//!
//! The original Phase 4b.2b accumulator walked variant slot-31 payloads
//! and produced a `[num_blendshapes × blends_per_pool]` matrix. Pass 4
//! (PHASE_4B_INVESTIGATION.md §6.6) anchored that the engine's matrix
//! is `[num_variants × blends_per_pool]` and is just a fold of the
//! top-K f32 pair list; per-blendshape unpacking happens later in
//! `ExecuteHuman` (Phase 4b.3). This module now performs only the
//! fold; the variant-payload walking + per-blendshape integration moves
//! to Phase 4b.3.

use thiserror::Error;

use super::matrix::DnaMatrix2D;

/// Errors from [`build_dna_matrix`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DnaMatrixError {
    /// `pruned_weights.len() != num_variants * blends_per_pool`.
    #[error(
        "weights length mismatch: expected num_variants*blends_per_pool = {expected}, got {got}"
    )]
    WeightsLengthMismatch { expected: usize, got: usize },
}

/// Fold the top-K-pruned f32 weight buffer into the 2-D DNA matrix.
///
/// Input: `pruned_weights` (the output of [`super::top_k::prune_top_k`]),
/// shape `[num_variants × blends_per_pool]` row-major.
///
/// Output: [`DnaMatrix2D`] carrying the same data and its shape.
///
/// Errors:
/// - [`DnaMatrixError::WeightsLengthMismatch`] if
///   `pruned_weights.len() != num_variants * blends_per_pool`.
pub fn build_dna_matrix(
    pruned_weights: &[f32],
    num_variants: usize,
    blends_per_pool: usize,
) -> Result<DnaMatrix2D, DnaMatrixError> {
    let expected = num_variants * blends_per_pool;
    if pruned_weights.len() != expected {
        return Err(DnaMatrixError::WeightsLengthMismatch {
            expected,
            got: pruned_weights.len(),
        });
    }
    Ok(DnaMatrix2D::from_pruned(
        pruned_weights,
        num_variants,
        blends_per_pool,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_mismatch_errors() {
        // 5-element buffer but num_variants * blends_per_pool = 8.
        let pruned = vec![0.0_f32; 5];
        let result = build_dna_matrix(&pruned, 2, 4);
        assert_eq!(
            result,
            Err(DnaMatrixError::WeightsLengthMismatch {
                expected: 8,
                got: 5,
            })
        );
    }

    #[test]
    fn produces_matrix_with_correct_shape() {
        let pruned = vec![0.0_f32; 6];
        let matrix = build_dna_matrix(&pruned, 2, 3).expect("valid length");
        assert_eq!(matrix.num_variants(), 2);
        assert_eq!(matrix.blends_per_pool(), 3);
        assert_eq!(matrix.as_slice().len(), 6);
    }

    #[test]
    fn produces_matrix_preserves_data() {
        // All values exactly representable in f32 (dyadic rationals).
        // Layout: row-major [num_variants=2, blends_per_pool=3].
        // variant 0 row: [0.5,    0.25,    0.125  ]
        // variant 1 row: [0.0625, 0.03125, 0.015625]
        let pruned = vec![
            0.5_f32, 0.25, 0.125,
            0.0625, 0.03125, 0.015625,
        ];
        let matrix = build_dna_matrix(&pruned, 2, 3).expect("valid length");

        // Exact f32 equality: all values are dyadic rationals.
        assert_eq!(matrix.cell(0, 0), 0.5_f32);
        assert_eq!(matrix.cell(0, 1), 0.25_f32);
        assert_eq!(matrix.cell(0, 2), 0.125_f32);
        assert_eq!(matrix.cell(1, 0), 0.0625_f32);
        assert_eq!(matrix.cell(1, 1), 0.03125_f32);
        assert_eq!(matrix.cell(1, 2), 0.015625_f32);
    }

    #[test]
    fn zero_weights_produce_zero_matrix() {
        let pruned = vec![0.0_f32; 8];
        let matrix = build_dna_matrix(&pruned, 4, 2).expect("valid length");
        assert_eq!(matrix.num_variants(), 4);
        assert_eq!(matrix.blends_per_pool(), 2);
        assert!(
            matrix.as_slice().iter().all(|&v| v == 0.0),
            "all cells must be zero"
        );
    }
}
