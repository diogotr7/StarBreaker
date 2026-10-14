//! 2-D DNA matrix: `[num_variants × blends_per_pool]` row-major f32.
//!
//! Engine anchor: `CFaceRigLogic + 0xc8` per
//! `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.6.5 (Pass 4 — corrected
//! from Pass 3's `num_blendshapes × blends_per_pool` mis-identification).
//! Pass 4 anchored that the `+0xc8` matrix's row dimension is
//! `num_variants`, not `num_blendshapes`. The per-blendshape unpack
//! happens later in `ExecuteHuman` (Phase 4b.3).
//!
//! Built by [`super::accumulator::build_dna_matrix`]; consumed by
//! Phase 4b.3 (currently in progress).
//!
//! Indexing convention: `data[variant_idx * blends_per_pool + face_part_idx]`.

/// 2-D DNA matrix carrying its shape with the data so consumers can't
/// mix up the dimensions.
#[derive(Debug, Clone, PartialEq)]
pub struct DnaMatrix2D {
    data: Vec<f32>,
    num_variants: usize,
    blends_per_pool: usize,
}

impl DnaMatrix2D {
    /// All-zeros matrix of the given shape. `data.len() ==
    /// num_variants * blends_per_pool`.
    #[cfg(test)]
    pub(crate) fn zeros(num_variants: usize, blends_per_pool: usize) -> Self {
        Self {
            data: vec![0.0_f32; num_variants * blends_per_pool],
            num_variants,
            blends_per_pool,
        }
    }

    /// Number of variant rows.
    pub fn num_variants(&self) -> usize {
        self.num_variants
    }

    /// Number of face-part columns.
    pub fn blends_per_pool(&self) -> usize {
        self.blends_per_pool
    }

    /// Read `[variant_idx, face_part_idx]`.
    ///
    /// # Panics
    ///
    /// Panics if either index is out of range. Strict-everywhere — no
    /// silent zero on bad input.
    pub fn cell(&self, variant_idx: usize, face_part_idx: usize) -> f32 {
        // Note: face_part_idx >= blends_per_pool can index into the next
        // row without panicking. Callers build indices by construction
        // so this never fires in practice. If callers ever index
        // dynamically, add an explicit bounds check.
        self.data[variant_idx * self.blends_per_pool + face_part_idx]
    }

    /// Mutable access for accumulation. Same panic contract as
    /// [`cell`]. `pub(crate)` so only the dna_matrix module-tree can
    /// write; downstream consumers get an immutable view.
    #[cfg(test)]
    pub(crate) fn cell_mut(&mut self, variant_idx: usize, face_part_idx: usize) -> &mut f32 {
        // Note: face_part_idx >= blends_per_pool can index into the next
        // row without panicking. Callers build indices by construction
        // so this never fires in practice.
        &mut self.data[variant_idx * self.blends_per_pool + face_part_idx]
    }

    /// Build a matrix by copying `data`. Caller must ensure
    /// `data.len() == num_variants * blends_per_pool`.
    ///
    /// `pub(crate)` so only the accumulator can call this; external
    /// consumers should go through `build_dna_matrix` which performs
    /// the length check.
    pub(crate) fn from_pruned(
        data: &[f32],
        num_variants: usize,
        blends_per_pool: usize,
    ) -> Self {
        debug_assert_eq!(
            data.len(),
            num_variants * blends_per_pool,
            "DnaMatrix2D::from_pruned: data.len() must equal num_variants * blends_per_pool",
        );
        Self {
            data: data.to_vec(),
            num_variants,
            blends_per_pool,
        }
    }

    /// Borrow the underlying row-major buffer. Length is
    /// `num_variants * blends_per_pool`.
    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeros_has_correct_shape() {
        let m = DnaMatrix2D::zeros(10, 3);
        assert_eq!(m.num_variants(), 10);
        assert_eq!(m.blends_per_pool(), 3);
        assert_eq!(m.as_slice().len(), 30);
        assert!(
            m.as_slice().iter().all(|&v| v == 0.0),
            "all cells must be zero"
        );
    }

    #[test]
    fn cell_accessor_round_trip() {
        let mut m = DnaMatrix2D::zeros(4, 2);
        *m.cell_mut(0, 0) = 1.5;
        *m.cell_mut(3, 1) = -2.25;
        *m.cell_mut(2, 0) = 7.0;

        assert_eq!(m.cell(0, 0), 1.5);
        assert_eq!(m.cell(3, 1), -2.25);
        assert_eq!(m.cell(2, 0), 7.0);
        // Cells we didn't touch stay zero.
        assert_eq!(m.cell(0, 1), 0.0);
        assert_eq!(m.cell(1, 0), 0.0);
    }

    /// Out-of-range read must panic. We use plain `Vec` indexing
    /// internally so this fires in both debug and release. If we ever
    /// switch to `get_unchecked`, this test will need updating.
    #[test]
    #[should_panic]
    fn cell_out_of_range_panics() {
        let m = DnaMatrix2D::zeros(4, 2);
        // variant_idx = 4 is one past the end.
        let _ = m.cell(4, 0);
    }
}
