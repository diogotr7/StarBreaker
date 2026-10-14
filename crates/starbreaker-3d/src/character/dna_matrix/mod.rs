//! DNA-matrix pipeline: CHF facial customisation → per-blendshape activations.
//!
//! Mirrors the per-frame morphing path of `CCharacterCustomizerComponent`
//! anchored in `StarCitizen.exe` (May 2026 build). See
//! `docs/riglogic/PHASE_4B_INVESTIGATION.md` for the Ghidra reference.
//!
//! Sub-modules:
//! - [`chf_weights`] — Phase 4b.1: CHF DnaBlend → flat f64 weight buffer.
//! - [`top_k`] — Phase 4b.2a: cast f64 → f32 + prune to top-4 active per pool + re-normalise.
//! - [`matrix`] — Phase 4b.2b: 2-D `DnaMatrix2D` newtype carrying `[num_variants × blends_per_pool]` shape (Pass 4 corrected).
//! - [`accumulator`] — Phase 4b.2b: fold the top-K-pruned weight buffer into the 2-D DNA matrix.

pub mod accumulator;
pub mod chf_weights;
pub mod matrix;
pub mod top_k;

pub use accumulator::{DnaMatrixError, build_dna_matrix};
pub use chf_weights::{ChfWeightsError, compute_chf_weights};
pub use matrix::DnaMatrix2D;
pub use top_k::{TopKError, prune_top_k};
