//! `.dna` file format parser for Star Citizen facial-rig DNA files.
//!
//! These files are loaded by `DNAFile::Load` (Ghidra anchor `FUN_14142d280`,
//! string `"DNAFile::Load"` at `0x147cf3320`) and consumed by the engine's
//! per-vertex morphing path (`Protos::ComputeMixedDeltaMatrixNumEntries`,
//! string at `0x147cf3140`).
//!
//! Path pattern (one file per LOD):
//!
//! ```text
//! Data/Objects/Characters/.../<protos_name>/<protos_name>_head.dna          (master, all LODs)
//! Data/Objects/Characters/.../<protos_name>/<protos_name>_head_lod1.dna     (LOD 1)
//! Data/Objects/Characters/.../<protos_name>/<protos_name>_head_lod2.dna
//! ...
//! ```
//!
//! File-level layout (verified against the LOD1 fixture
//! `protos_human_male_face_t1_pu_head_lod1.dna`, 1,552,792 bytes):
//!
//! - Bytes `0x00..0x10`: 16-byte magic block `b"DNA V1.6\0\0\0\0\0\0\0\0"`
//!   (checked by Load against `0x362e315620414e44` then a u32 zero).
//! - Bytes `0x10..0x18`: timestamp (`u64`) — `LoadAllLODs` cross-validates
//!   that every LOD file in a set carries the same value.
//! - Bytes `0x18..0x1c`: `protos_name_id` (`u32`, CRC32) — compared against
//!   the SKIN file's `param_4[0x12]`.
//! - Bytes `0x1c..0x20`: `mesh_name_id` (`u32`, CRC32) — compared against
//!   the SKIN file's `param_4[5]`.
//! - Bytes `0x20..0x24`: `u32` `header_u32_20` (semantics not anchored).
//! - Bytes `0x24..0x26`: `u16` `header_u16_24` (semantics not anchored).
//! - Byte `0x26`: `u8` `num_face_parts` (= 3 for master human head;
//!   anchored in PHASE_4B_INVESTIGATION §6.6 Pass 6).
//! - Byte `0x27`: `u8` `header_u8_27` (= 4 for master; top-K sparse slot count).
//! - Byte `0x28`: `u8` `mesh_blends_per_pool` (= 13 for master;
//!   `mesh[+0x28]` at runtime; anchored in PHASE_4B_INVESTIGATION §6.7/§6.8).
//! - Byte `0x29`: `u8` `header_u8_29` (= 13 for master; likely duplicate of
//!   `mesh_blends_per_pool`; semantics not anchored).
//! - Bytes `0x2a..0x2c`: `num_variants` (`u16`) — the canonical
//!   "Zero variants found" check reads this slot.
//! - Bytes `0x2c..0x30`: `u16` header words (semantics not anchored).
//! - Bytes `0x30..0x34`: `num_vertices` (`u32`) — this LOD's mesh
//!   vertex count, anchored against the head `.skin`'s `MeshIvo320`
//!   chunk by master-file cross-validation (5,584 on the master matches
//!   the head `.skin`; LOD1 carries the decimated 3,158).
//! - Bytes `0x34..0x38`: `num_indices` (`u32`) — mesh index count,
//!   anchored the same way.
//! - Bytes `0x38..0x40`: trailing `u32` words (semantics not anchored).
//! - Bytes `0x40..0x140`: **pointer table** — 32 slots × `u64`. Each
//!   nonzero value is an offset into the same file (rebased to a pointer
//!   by `FUN_14142a990`'s fixup pass).
//! - Bytes `0x140..` : payload region pointed to by the table.
//! - End of file: a copy of the original path that Load appends after the
//!   payload (the runtime resets `*(size_t *)(*param_1 + 0x140)` to this).
//!
//! Slot 31 (`+0x138`) holds the per-variant delta-records array:
//! `num_variants` records of 40 bytes each, layout
//! `(u32 num_main, u32 num_secondary, f64 primary_scale,
//!   f64 secondary_scale, u64 main_offset, u64 secondary_offset)`.
//! This slot is **NULL** in every LOD `.dna` we have observed — only the
//! master file carries it. See [`VariantRecord`] for the parsed view.
//!
//! See `docs/riglogic/SPEC.md` ("`.dna` file format") for the full byte
//! map, what each pointer slot is believed to carry, and which slots are
//! still unanchored.
//!
//! Public API: [`DnaFile::read`] returns a parsed view of the header,
//! pointer table, and (when present) the per-variant delta-record headers
//! from slot 31. The runtime `ScatteredDataInterpolation` arrays —
//! per-variant feature-space control points and variant-space tetrahedra
//! — are NOT stored as flat pointer-table slots in the file; they are
//! constructed at character-load time by one of the tetrahedralization
//! algorithms registered at `FUN_140087990`. See the
//! "Phase 4a extension: HALT on control points + tetrahedra" section in
//! `docs/riglogic/SPEC.md` for the evidence trail.

pub mod error;
pub mod expression_morphs;
pub mod joint_weights;
pub mod neutral_morphs;
pub mod parser;
pub mod region_weights;
pub mod variant_deltas;
pub mod variant_joints;
pub mod variant_names;
#[cfg(test)]
mod test_p4k;

pub use error::{DnaError, VariantDeltaSide};
pub use expression_morphs::{
    ExpressionEntries, ExpressionMorphs, ExpressionRecord, EXPRESSION_MORPHS_SLOT,
};
pub use joint_weights::JOINT_WEIGHTS_SLOT;
pub use neutral_morphs::{NeutralMorphs, NEUTRAL_VERTEX_MORPHS_SLOT};
pub use parser::{
    DnaFile, DnaHeader, DnaSegment, MAGIC, POINTER_TABLE_OFFSET, POINTER_TABLE_SLOTS,
    VARIANT_ARRAY_SLOT, VARIANT_RECORD_STRIDE, VariantRecord,
};
pub use variant_deltas::{VariantDeltaIter, VariantDeltaRecord};
pub use region_weights::{MESH_REGIONS_SLOT, REGION_WEIGHTS_SLOT};
pub use variant_joints::VARIANT_NEUTRAL_JOINTS_SLOT;
pub use variant_names::{VARIANT_NAME_STRIDE, VARIANT_NAMES_SLOT};
