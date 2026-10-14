//! Phase 4b.2a: top-K prune of the CHF weight buffer + per-pool re-normalise.
//!
//! Mirrors the inner step of `UpdateDNAMatrix` (Ghidra `0x14411f9c0`)
//! between the f64 → f32 cast (`vcvtpd2ps`) and the sparse-pair-list
//! construction. See `docs/riglogic/PHASE_4B_INVESTIGATION.md` §3 / §6.
//!
//! ## Algorithm
//!
//! 1. Element-wise cast f64 → f32.
//! 2. For each FACE-PART column (the cells `v*blends_per_pool + fp` across
//!    all variants `v`): while the count of nonzero cells exceeds `K = 4`,
//!    find the smallest nonzero cell and zero it. Ties: lowest variant
//!    index wins (see "Tie-breaking" below).
//! 3. Per-FACE-PART (column) sum-to-1 re-normalise.
//!
//! Pass 19h correction: prune + normalise are per face-part COLUMN, not per
//! variant row — the head blend is convex per face part (its head values
//! sum to ~65535).
//!
//! ## K = 4
//!
//! Engine-anchored constant. The `topK[]` buffer in
//! `ComputeControlPointFromWeights` (`auStack_128` in the decompile) holds
//! 4 `(int, double)` pairs, and `UpdateDNAMatrix` pre-prunes each pool to
//! ≤ 4 active weights before the sparse-pair-list construction.
//!
//! ## Tie-breaking
//!
//! When two nonzero cells in the same pool have the exact same f32 value,
//! "the smallest" is ambiguous. The engine's prune loop scans bytes / SIMD
//! lanes in some order but doesn't anchor a byte-for-byte rule we can
//! mirror. We pick "lowest index wins on equality": natural with
//! `iter().enumerate().min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(Equal))`,
//! and deterministic. Realistic CHF data (`value / 65535.0 / pool_sum`)
//! is extremely unlikely to produce exact f32 collisions. If 4b.3's
//! integration testing surfaces a divergence against the engine's
//! per-blendshape activations, we revisit.

use thiserror::Error;

/// Maximum nonzero cells per pool after pruning. Engine anchor: the
/// `topK[]` buffer in `ComputeControlPointFromWeights` and the explicit
/// pre-prune in `UpdateDNAMatrix`. See `PHASE_4B_INVESTIGATION.md` §3.
const K: usize = 4;

/// Errors from [`prune_top_k`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TopKError {
    /// Input buffer length didn't match `num_variants * blends_per_pool`.
    /// Strict-everywhere: a length mismatch usually indicates that the
    /// caller passed a buffer from a different pipeline stage with
    /// stale dimensions.
    #[error(
        "weights length mismatch: expected num_variants*blends_per_pool = {expected}, got {got}"
    )]
    LengthMismatch { expected: usize, got: usize },
}

/// Cast `weights_f64` to f32, prune each pool to at most `K = 4` nonzero
/// cells, then per-pool sum-to-1 re-normalise.
///
/// Input layout: row-major `[num_variants × blends_per_pool]` f64 (the
/// output of [`super::chf_weights::compute_chf_weights`]).
/// Output: same shape, f32. Length = `num_variants * blends_per_pool`.
///
/// See the module header for the algorithm and tie-breaking rule.
///
/// Returns `LengthMismatch` if
/// `weights_f64.len() != num_variants * blends_per_pool`.
pub fn prune_top_k(
    weights_f64: &[f64],
    num_variants: usize,
    blends_per_pool: usize,
) -> Result<Vec<f32>, TopKError> {
    let expected = num_variants * blends_per_pool;
    if weights_f64.len() != expected {
        return Err(TopKError::LengthMismatch {
            expected,
            got: weights_f64.len(),
        });
    }

    // Step 1: element-wise cast f64 → f32 (mirrors vcvtpd2ps).
    let mut out: Vec<f32> = weights_f64.iter().map(|&w| w as f32).collect();

    // Step 2 + 3: per-FACE-PART (column) prune + re-normalise.
    //
    // CORRECTION (Pass 19h): the convex blend is PER FACE PART over the
    // contributing heads (a face part's head values sum to ~65535), so the
    // top-K prune AND the sum-to-1 normalise are per face-part COLUMN (across
    // variants), NOT per variant row. The previous per-variant-row version
    // re-imposed the wrong axis and undid `compute_chf_weights`'s per-column
    // normalisation. Cell (variant v, face-part fp) = `v*blends_per_pool+fp`.
    for fp in 0..blends_per_pool {
        let cell = |v: usize| v * blends_per_pool + fp;

        // Prune to ≤ K active heads in this face-part column.
        loop {
            let active = (0..num_variants).filter(|&v| out[cell(v)] > 0.0).count();
            if active <= K {
                break;
            }
            // Smallest nonzero in the column; lowest variant index wins on tie.
            let min_v = (0..num_variants)
                .filter(|&v| out[cell(v)] > 0.0)
                .min_by(|&a, &b| {
                    out[cell(a)]
                        .partial_cmp(&out[cell(b)])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("active > K > 0 ⇒ at least one nonzero exists");
            out[cell(min_v)] = 0.0;
        }

        // Re-normalise the column to sum-1.
        let sum: f32 = (0..num_variants).map(|v| out[cell(v)]).sum();
        if sum > 0.0 {
            for v in 0..num_variants {
                out[cell(v)] /= sum;
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_column_no_prune_normalises() {
        // 4 variants × 1 face-part. Column = [0.5,0.25,0.125,0.125] (one per
        // variant). 4 active = K, no prune. Column sum = 1.0 → no-op.
        let weights_f64 = vec![0.5_f64, 0.25, 0.125, 0.125];
        let out = prune_top_k(&weights_f64, 4, 1).expect("valid length");
        assert_eq!(out.len(), 4);
        assert_eq!(out[0], 0.5_f32);
        assert_eq!(out[1], 0.25_f32);
        assert_eq!(out[2], 0.125_f32);
        assert_eq!(out[3], 0.125_f32);
    }

    #[test]
    fn column_prune_smallest_to_top_four() {
        // 5 variants × 1 face-part. Column [0.1,0.4,0.2,0.2,0.1]. Smallest
        // nonzero is variant 0 (0.1, ties variant 4; lowest variant wins) →
        // zeroed. Remaining [0,0.4,0.2,0.2,0.1] sum 0.9 → [0,4/9,2/9,2/9,1/9].
        let weights_f64 = vec![0.1_f64, 0.4, 0.2, 0.2, 0.1];
        let out = prune_top_k(&weights_f64, 5, 1).expect("valid length");
        assert_eq!(out.len(), 5);
        assert_eq!(out[0], 0.0_f32, "smallest at lowest variant should be zeroed");
        assert!((out[1] - 4.0_f32 / 9.0).abs() < 1e-6, "out[1]={}", out[1]);
        assert!((out[2] - 2.0_f32 / 9.0).abs() < 1e-6, "out[2]={}", out[2]);
        assert!((out[3] - 2.0_f32 / 9.0).abs() < 1e-6, "out[3]={}", out[3]);
        assert!((out[4] - 1.0_f32 / 9.0).abs() < 1e-6, "out[4]={}", out[4]);
    }

    #[test]
    fn columns_pruned_and_normalised_independently() {
        // 5 variants × 2 face-parts. Row-major [v*2 + fp].
        //   col 0 (fp 0): variants [0.3,0.3,0.2,0.1,0.1] → prune smallest
        //     (variant 3 ties variant 4 at 0.1; lowest variant 3 zeroed) →
        //     [0.3,0.3,0.2,0,0.1] sum 0.9 → [3/9,3/9,2/9,0,1/9].
        //   col 1 (fp 1): variants [0.5,0.3,0.2,0,0] → 3 active, no prune,
        //     sum 1.0.
        let weights_f64 = vec![
            0.3_f64, 0.5, // v0
            0.3, 0.3, // v1
            0.2, 0.2, // v2
            0.1, 0.0, // v3
            0.1, 0.0, // v4
        ];
        let out = prune_top_k(&weights_f64, 5, 2).expect("valid length");
        assert_eq!(out.len(), 10);
        // col 0 (indices 0,2,4,6,8):
        assert!((out[0] - 3.0_f32 / 9.0).abs() < 1e-6, "col0 v0={}", out[0]);
        assert!((out[2] - 3.0_f32 / 9.0).abs() < 1e-6, "col0 v1={}", out[2]);
        assert!((out[4] - 2.0_f32 / 9.0).abs() < 1e-6, "col0 v2={}", out[4]);
        assert_eq!(out[6], 0.0_f32, "col0 v3 pruned");
        assert!((out[8] - 1.0_f32 / 9.0).abs() < 1e-6, "col0 v4={}", out[8]);
        // col 1 (indices 1,3,5,7,9): no prune, ~identity normalise.
        assert!((out[1] - 0.5_f32).abs() < 1e-6, "col1 v0={}", out[1]);
        assert!((out[3] - 0.3_f32).abs() < 1e-6, "col1 v1={}", out[3]);
        assert!((out[5] - 0.2_f32).abs() < 1e-6, "col1 v2={}", out[5]);
        assert_eq!(out[7], 0.0_f32);
        assert_eq!(out[9], 0.0_f32);
    }

    #[test]
    fn length_mismatch_errors() {
        // 5-element buffer, but num_variants * blends_per_pool = 8.
        let weights_f64 = vec![0.0_f64; 5];
        let result = prune_top_k(&weights_f64, 2, 4);
        assert_eq!(
            result,
            Err(TopKError::LengthMismatch {
                expected: 8,
                got: 5
            })
        );
    }
}
