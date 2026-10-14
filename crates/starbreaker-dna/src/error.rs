//! Error type for `.dna` parsing.
//!
//! Wraps [`starbreaker_common::ParseError`] with `.dna`-specific
//! validation variants (bad magic, mis-sized buffer, out-of-range
//! pointer-table entry).

use starbreaker_common::ParseError;
use thiserror::Error;

/// Which side of a variant's slot-31 delta arrays an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantDeltaSide {
    Main,
    Secondary,
}

impl std::fmt::Display for VariantDeltaSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariantDeltaSide::Main => f.write_str("main"),
            VariantDeltaSide::Secondary => f.write_str("secondary"),
        }
    }
}

/// Errors produced by [`crate::DnaFile::read`].
#[derive(Debug, PartialEq, Error)]
pub enum DnaError {
    /// Input buffer too short for the fixed 0x140-byte header + pointer
    /// table prefix.
    #[error(
        "DNA buffer too short: need at least {need} bytes for header + pointer table, got {have}"
    )]
    HeaderTruncated { need: usize, have: usize },

    /// The 16-byte magic at offset 0 did not match `"DNA V1.6\0\0\0\0\0\0\0\0"`.
    #[error("invalid DNA magic at offset 0: expected {expected:?}, got {got:?}")]
    InvalidMagic {
        expected: [u8; 16],
        got: [u8; 16],
    },

    /// `num_variants == 0`. The runtime emits the corresponding
    /// "Zero variants found" diagnostic at `0x147cf3640`.
    #[error("DNA file declares zero variants (the runtime rejects this)")]
    ZeroVariants,

    /// A pointer-table slot pointed outside the file buffer.
    #[error(
        "DNA pointer table slot {slot:#x} = {offset:#x} is outside the file (file size {file_size})"
    )]
    PointerOutOfRange {
        slot: usize,
        offset: u64,
        file_size: usize,
    },

    /// A pointer-table slot pointed *into* the header / pointer-table
    /// region itself (offsets `< 0x140`). The runtime always points to
    /// payload bytes.
    #[error(
        "DNA pointer table slot {slot:#x} = {offset:#x} is inside the header/pointer-table region (< 0x140)"
    )]
    PointerInsideHeader { slot: usize, offset: u64 },

    /// Slot 31's per-variant header table would extend past the end of
    /// the file. Either `num_variants` is larger than the file's actual
    /// variant array, or the slot 31 offset is corrupt. Mirrors the
    /// pointer-table fixup loop in `FUN_14142a990` which assumes the
    /// array runs for `num_variants * 0x28` bytes from the slot offset.
    #[error(
        "DNA slot 31 variant array overflows file: base {base:#x} + {need} bytes > file size {file_size}"
    )]
    VariantArrayOverflow {
        base: u64,
        need: usize,
        file_size: usize,
    },

    /// `variant_idx` was outside `0..num_variants`.
    #[error(
        "variant index {variant_idx} >= num_variants {num_variants}"
    )]
    VariantOutOfRange { variant_idx: usize, num_variants: usize },

    /// Master slot 31 is null — LOD `.dna` files do not carry per-variant
    /// delta records; only the master does. Calling `variant_main_deltas`
    /// or `variant_secondary_deltas` on a non-master file returns this.
    #[error("DNA slot 31 (per-variant deltas) is unset; only master .dna files carry it")]
    VariantArrayUnset,

    /// During iteration: the declared `num_*_delta_records` count could not
    /// be reached before running out of bytes.
    #[error(
        "variant {variant_idx} {side} delta records truncated: expected {expected_records}, got {got_records} (bytes remaining: {bytes_remaining})"
    )]
    VariantDeltaTruncated {
        variant_idx: usize,
        side: VariantDeltaSide,
        expected_records: u32,
        got_records: u32,
        bytes_remaining: usize,
    },

    /// Slot 21 (per-(joint × face-part) joint-weights matrix) is null.
    /// LOD files don't carry this slot; only the master does.
    #[error("DNA slot 21 (joint-weights matrix) is unset; only master .dna files carry it")]
    JointWeightsUnset,

    /// Slot 21's actual span is smaller than the declared
    /// `num_joints * mesh_blends_per_pool * 4` bytes.
    #[error(
        "DNA slot 21 joint-weights matrix too small: need {needed} bytes for {num_joints}×{mesh_bpp} f32, span = {got}"
    )]
    JointWeightsLengthMismatch {
        num_joints: usize,
        mesh_bpp: usize,
        needed: usize,
        got: usize,
    },

    /// Slot 15 (per-variant per-joint neutral translation offsets) is null.
    /// LOD files don't carry this slot; only the master does.
    #[error("DNA slot 15 (variant neutral joints) is unset; only master .dna files carry it")]
    VariantNeutralJointsUnset,

    /// Slot 15's actual span is smaller than `num_variants * num_joints * 3 * 4`.
    #[error(
        "DNA slot 15 variant-neutral-joints too small: need {needed} bytes for {num_variants}×{num_joints}×3 f32, span = {got}"
    )]
    VariantNeutralJointsLengthMismatch {
        num_variants: usize,
        num_joints: usize,
        needed: usize,
        got: usize,
    },

    /// Slot 22 (per-vertex DNA-region blend weights) is null. LOD files do
    /// carry slot 22, but a `.dna` without it returns this.
    #[error("DNA slot 22 (region weights) is unset")]
    RegionWeightsUnset,

    /// Slot 22's span is smaller than `num_vertices * mesh_blends_per_pool * 4 + 8`.
    #[error(
        "DNA slot 22 region-weights too small: need {needed} bytes for {regions}×{num_vertices} f32 + 8-byte trailer, span = {got}"
    )]
    RegionWeightsLengthMismatch {
        num_vertices: usize,
        regions: usize,
        needed: usize,
        got: usize,
    },

    /// Slot 12 / header `0x29` describe an impossible per-mesh region list.
    #[error("DNA mesh region list malformed: {0}")]
    MeshRegionsMalformed(String),

    /// Slot 2 (per-variant archetype name pool) is null.
    #[error("DNA slot 2 (variant names) is unset")]
    VariantNamesUnset,

    /// Slot 2 is too short for `num_variants` names or holds a non-ASCII name.
    #[error("DNA slot 2 (variant names) malformed: {0}")]
    VariantNamesMalformed(String),

    /// Slot 19 (per-(variant × blendshape) per-vertex expression deltas) is
    /// null. Only the master `.dna` carries it.
    #[error("DNA slot 19 (expression morphs) is unset; only master .dna files carry it")]
    ExpressionMorphsUnset,

    /// Slot 19's Region-A record count is not a clean multiple of
    /// `num_variants` (the variant-major grid `num_variants × num_blendshapes`
    /// must divide evenly).
    #[error(
        "DNA slot 19 Region-A record count {records} is not a multiple of num_variants {num_variants}"
    )]
    ExpressionMorphsMalformed { records: usize, num_variants: usize },

    /// Slot 18 (per-vertex neutral identity morphs) is null. Only the master
    /// `.dna` carries it.
    #[error("DNA slot 18 (neutral vertex morphs) is unset; only master .dna files carry it")]
    NeutralMorphsUnset,

    /// Slot 18's `[bitmask][cursor][records][codes]` layout does not fit the
    /// segment, or the cursor-vs-bitmask-popcount invariant fails (wrong
    /// `num_vertices` / `num_variants` / `mesh_blends_per_pool`).
    #[error(
        "DNA slot 18 malformed: {context} (num_vertices={num_vertices}, num_controls={num_controls})"
    )]
    NeutralMorphsMalformed {
        context: &'static str,
        num_vertices: usize,
        num_controls: usize,
    },

    /// Lower-level shared parse error (used for downstream tooling that
    /// wants `ParseError` as the common currency).
    #[error(transparent)]
    Parse(#[from] ParseError),
}
