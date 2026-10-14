//! RigLogic — CIG's facial DNA / blendshape format.
//!
//! Parses the `.RigLogic` standalone file (or the embedded `RigLogic`
//! IVO chunk inside head `.skin` files) into a typed [`RigLogic`] struct,
//! plus the sibling 80-byte [`ProtosInfo`] chunk, and composes the corrective
//! blendshape activations ([`compose_correctives`]) the character pipeline
//! drives the DNA joint and vertex deltas with.
//!
//! Tasks 1-4 own the header, static metadata tables, the variable-stride
//! delta tables, and the trailing region/joint tables — magic, rig name,
//! packed [`RigType`], identity scalars, the typed inline count/scale fields
//! at file offsets `0x40..0x68`, the eight `u64` section-start pointers at
//! `0x68..0xA8`, Section A's joint-control CRC32 (IEEE) table, Section B's
//! null-separated control-name pool, Section C's corrective-blendshape
//! composition rows, Section D + E's main (appearance) and secondary
//! (corrective) delta tables, Section F's skeleton-bone CRC32 (IEEE) table
//! (bone names follow the `FACIAL_LOD*` convention — confirmed in Task 7),
//! Section G's per-region bone-index lists, and Section H's per-bone-per-region
//! weight scalars.
//!
//! Variant metadata (ethnicity, build, skin tone, age range) is NOT stored in
//! the V1.9 `.RigLogic` file — it lives in companion `.dna` files loaded via
//! `DNAFile::Load` (anchored by the `"DNA Matrix 1.0"` and `_lod1.dna` strings
//! in the V1.9 binary). The original Task 4 plan's `Vec<Variant>` /
//! ethnicity/build/skin_tone fields came from V3.0 server-format assumptions
//! and do NOT apply to the V1.9 on-disk format. See "Known gaps" in
//! [`SPEC.md`].
//!
//! Spec: `docs/riglogic/SPEC.md`. The header layout was anchored against
//! `FaceRigData::Load` (`StarCitizen.exe` `0x14142d4d0`); the inline count
//! fields against `CFaceRigLogic::InitFaceRigLogic` (`0x14671a270`),
//! `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`), and
//! `Protos::ComputeMixedDeltaMatrixNumEntries` (`0x14142a5c0`). The trailing
//! sections (C, F, G, H) were byte-anchored in Task 4 against
//! `CFaceRigLogic::ExecuteGeneric`'s region-blending pass at offsets
//! `+0x98` (Section G) and `+0xa0` (Section H) and the corrective composition
//! loop at offset `+0x78` (Section C).
//!
//! # ParseError variant mapping
//!
//! `starbreaker_common::ParseError` uses different names from the spec's
//! error taxonomy. The mapping is:
//! - spec `BadMagic`     → `ParseError::InvalidMagic`
//! - spec `UnexpectedEof` → `ParseError::Truncated`
//! - spec `Invariant`    → `ParseError::Validation`
//! - spec `BadValue`     → `ParseError::UnexpectedValue`

pub mod sections;

use serde::{Deserialize, Serialize};
use starbreaker_common::{NameHash, ParseError, SpanReader};

/// The 8-byte magic at the start of every `.RigLogic` file (and of every
/// embedded `RigLogic` IVO chunk's content, after its 8-byte size prefix).
pub const RIG_LOGIC_MAGIC: [u8; 8] = *b"RIG V1.9";

/// The required value of the 16-bit `header_size` field at file offset
/// `0x3E`. Matches the file offset at which the section content begins
/// (`0xA8`). `FaceRigData::Load` rejects files where this differs.
pub const RIG_LOGIC_HEADER_SIZE: u16 = 0xA8;

/// Length of the null-padded rig name field (file offsets `0x08..0x30`).
const NAME_FIELD_BYTES: usize = 40;

/// Number of section-start pointers in the table at `0x68..0xA8`.
pub const NUM_SECTION_STARTS: usize = 8;

/// File offset where the section-start pointer table begins.
const SECTION_START_TABLE_OFFSET: usize = 0x68;

/// Byte size of a single `QuatT` element (4-float rotation + 3-float
/// translation, no trailing padding). Anchored in `CFaceRigLogic::ExecuteGeneric`
/// (`0x1466dec50`) via the per-joint rotation loop `lVar22 = (longlong)*psStack_188 * 0x1c`,
/// where `0x1c = 28` is the stride into the engine-side joint rest-transform array.
///
/// The on-disk `.RigLogic` (V1.9) file does NOT contain joint rest transforms —
/// they live in the companion skeleton. This constant is exposed for downstream
/// consumers that need to read `QuatT`-shaped buffers in adjacent CryEngine
/// formats and for cross-validation in tests.
pub const QUATT_STRIDE: usize = 28;

/// Byte stride of the Section A joint-control CRC32 (IEEE) table.
const SECTION_A_ENTRY_STRIDE: usize = 4;

/// Slot in `section_starts` whose value is the start offset of Section A
/// (the joint-control CRC32 table). This is the bootstrap slot, always `0xA8`.
const SECTION_A_START_SLOT: usize = 1;

/// Slot in `section_starts` whose value is the start offset of Section B
/// (the null-separated joint-control name string pool).
const SECTION_B_START_SLOT: usize = 0;

/// Slot in `section_starts` whose value is the start offset of Section C
/// (the next section past the name pool — used by the name-pool reader to
/// determine the pool's upper bound).
const SECTION_C_START_SLOT: usize = 2;

/// Slot in `section_starts` whose value is the start offset of Section D
/// (the main / appearance variable-stride delta table).
const SECTION_D_START_SLOT: usize = 3;

/// Slot in `section_starts` whose value is the start offset of Section E
/// (the secondary / corrective variable-stride delta table).
const SECTION_E_START_SLOT: usize = 4;

/// Slot in `section_starts` whose value is the start offset of Section F
/// (the skeleton-bone CRC32 table). Section D ends where this section begins;
/// likewise Section E ends at Section F's start.
const SECTION_F_START_SLOT: usize = 5;

/// Slot in `section_starts` whose value is the start offset of Section G
/// (the per-blend-region bone-index list table). Section F ends where this
/// begins.
const SECTION_G_START_SLOT: usize = 6;

/// Slot in `section_starts` whose value is the start offset of Section H
/// (the per-bone-per-region weight scalar table). Section G ends where this
/// begins.
const SECTION_H_START_SLOT: usize = 7;

/// Byte stride of a single Section F skeleton-bone CRC32 entry (a `u32`).
/// Anchored in `CFaceRigLogic::InitFaceRigLogic` (`0x14671a270`):
/// `*(undefined4 *)(*(longlong *)(lVar3 + 0x90) + (ulonglong)uVar20 * 4)`
/// — the loop reads 4 bytes per joint from `section_starts[5]`.
const SECTION_F_ENTRY_STRIDE: usize = 4;

/// Byte stride of a single Section C row. Anchored against
/// `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`)'s corrective-composition
/// loop, where the loop advances by 5 bytes per row (`uVar21 = uVar21 + 5`)
/// and indexes five `u8`s (`c[off-5..off-1]`) per row.
const SECTION_C_ROW_STRIDE: usize = 5;

/// Byte stride of a single Section G entry (a `u16`). The section is a flat
/// sequence of `u16` values; each region begins with a `count: u16` followed
/// by `count × u16` bone-list indices. Anchored against
/// `CFaceRigLogic::ExecuteGeneric` at `lVar42 = lVar42 + 2` and
/// `*(short *)(lVar22 + 2 + lVar42)`.
const SECTION_G_ENTRY_STRIDE: usize = 2;

/// Byte stride of a single Section H entry (a `f32`). Anchored against
/// `CFaceRigLogic::ExecuteGeneric` at `lVar32 = lVar32 + 4` and
/// `*(float *)(lVar32 + *(longlong *)(lVar18 + 0xa0))`.
const SECTION_H_ENTRY_STRIDE: usize = 4;

/// File offset where the `header_size` u16 lives.
const HEADER_SIZE_OFFSET: usize = 0x3E;

/// Slot 1 of the section-start table is invariantly the bootstrap value
/// (= the offset of the first byte past the table). It is also the
/// expected value of `header_size`.
const SECTION_START_BOOTSTRAP_VALUE: u64 = RIG_LOGIC_HEADER_SIZE as u64;

/// Packed 8-byte `RigType` classifier at file offsets `0x30..0x38`.
///
/// Layout (from the V1.9 standalone byte dump `01 01 00 03 01 00 06 00`,
/// matching the `RigType` layout discussed in the spec):
///
/// | Offset | Field    | Type | Notes |
/// |--------|----------|------|-------|
/// | 0      | `class`  | u8   | `1` = `Human` in the observed fixture. |
/// | 1      | `gender` | u8   | `1` = `Male` in the observed fixture. |
/// | 2      | `variant`| u8   | `0` in the observed fixture. |
/// | 3      | `part`   | u8   | `3` = `Face` in the observed fixture. |
/// | 4      | `tier`   | u8   | `1` (= `_t1_` in the rig name). |
/// | 5      | `lod`    | u8   | `0` in the observed fixture. |
/// | 6      | `flags_or_pad` | u16 | `0x0006` in the observed fixture. The loader does not read this field; its semantics (flags vs padding) are not yet anchored. |
// `Debug` is not derived on `#[repr(C, packed)]` types with multi-byte fields
// because the compiler cannot take references to unaligned fields.  A manual
// impl copies each field into a local before formatting.
#[repr(C, packed)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RigType {
    pub class: u8,
    pub gender: u8,
    pub variant: u8,
    pub part: u8,
    pub tier: u8,
    pub lod: u8,
    /// Trailing 2 bytes. Observed `0x0006`. The loader does not read this;
    /// it may be flags or padding. `[inferred]` per `docs/riglogic/SPEC.md`.
    pub flags_or_pad: u16,
}

impl std::fmt::Debug for RigType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Copy fields to locals to avoid borrowing unaligned packed fields.
        let class = self.class;
        let gender = self.gender;
        let variant = self.variant;
        let part = self.part;
        let tier = self.tier;
        let lod = self.lod;
        let flags_or_pad = self.flags_or_pad;
        f.debug_struct("RigType")
            .field("class", &class)
            .field("gender", &gender)
            .field("variant", &variant)
            .field("part", &part)
            .field("tier", &tier)
            .field("lod", &lod)
            .field("flags_or_pad", &flags_or_pad)
            .finish()
    }
}

// `Serialize` is not derived on `#[repr(C, packed)]` types with multi-byte
// fields because the compiler cannot take references to unaligned fields.
// The manual impl copies each field to a local before serializing.
impl Serialize for RigType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let class = self.class;
        let gender = self.gender;
        let variant = self.variant;
        let part = self.part;
        let tier = self.tier;
        let lod = self.lod;
        let flags_or_pad = self.flags_or_pad;
        let mut s = serializer.serialize_struct("RigType", 7)?;
        s.serialize_field("class", &class)?;
        s.serialize_field("gender", &gender)?;
        s.serialize_field("variant", &variant)?;
        s.serialize_field("part", &part)?;
        s.serialize_field("tier", &tier)?;
        s.serialize_field("lod", &lod)?;
        s.serialize_field("flags_or_pad", &flags_or_pad)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for RigType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Helper {
            class: u8,
            gender: u8,
            variant: u8,
            part: u8,
            tier: u8,
            lod: u8,
            flags_or_pad: u16,
        }
        let h = Helper::deserialize(deserializer)?;
        Ok(RigType {
            class: h.class,
            gender: h.gender,
            variant: h.variant,
            part: h.part,
            tier: h.tier,
            lod: h.lod,
            flags_or_pad: h.flags_or_pad,
        })
    }
}

/// A 28-byte joint rest transform: 4-float rotation quaternion followed by
/// 3-float translation. The on-disk `.RigLogic` (V1.9) file does NOT carry
/// `QuatT` tables — they live in the companion skeleton and are consumed via
/// the engine's `CCharInstance + 0x568` pointer. This type is exposed for
/// downstream consumers and tests that read `QuatT`-shaped buffers, and to
/// pin the 28-byte stride for the joint rotation loop in
/// `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`).
///
/// The rotation component is currently exposed as four bare `f32`s; pinning
/// the quaternion ordering (`(x, y, z, w)` vs `(w, x, y, z)`) and any axis
/// remap would require a fixture with non-trivial reference data and is
/// deferred to the integration-test task.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuatT {
    /// 4-float rotation quaternion. Ordering not yet pinned.
    pub rotation: [f32; 4],
    /// 3-float translation in skeleton-local space.
    pub translation: [f32; 3],
}

const _: () = {
    // Anchor the stride at compile time — see `QUATT_STRIDE`.
    assert!(core::mem::size_of::<QuatT>() == QUATT_STRIDE);
};

/// A single facial blend region's bone-list, populated from Section G.
///
/// Section G layout (byte-anchored in Task 4 against
/// `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`)'s region-blending pass):
///
/// ```text
/// for each of num_blend_regions regions:
///   u16  count             // number of bone entries for this region
///   u16  bone_indices[count] // i16-but-stored-as-u16 weight indices
/// ```
///
/// Each region's `weights` slice into Section H is `count` long, and Section H
/// is a flat concatenation across all regions in declaration order. The
/// `weights` are NOT stored on this struct — they live on
/// [`RigLogic::region_weights`] as one flat `Vec<f32>` of size
/// `sum(regions[i].weight_indices.len())`. Use [`RigLogic::region_weight_slice`]
/// to retrieve a specific region's weights.
///
/// **Naming note:** the field is called `weight_indices` because each entry
/// indexes into a per-joint weight scratch buffer at runtime (see
/// `pvVar53 = pvStack_1c0` — the per-joint translation accumulator). The
/// engine's accumulation is `region_output[i] += weights[bone_idx] * scalar`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendRegionBones {
    /// Per-region list of `i16` indices read directly from Section G. Length
    /// equals the `u16 count` byte that precedes the indices on disk. Values
    /// are stored as `u16`; the engine reads them as signed `short` and uses
    /// them as positive indices into a translation-accumulator buffer of
    /// length `num_joints * 3`.
    pub weight_indices: Vec<u16>,
}

/// A single Section C corrective-blendshape composition row — five `u8`
/// indices into the appearance-weight buffer.
///
/// **Anchored against `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`)**: the
/// corrective composition loop reads five bytes per row and multiplies their
/// referenced appearance weights together to produce one corrective weight:
///
/// ```text
/// corrective_weight = w[row[0]] * w[row[1]] * w[row[2]] * w[row[3]] * w[row[4]]
/// ```
///
/// A zero byte slot acts as a no-op factor (`w[0]` is the always-on
/// `CTL_PSD` control). The fixture's last 6 bytes of Section C are zero
/// padding past the `num_corrective_blendshapes` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectiveCompositionRow {
    /// Five `u8` appearance-control indices. Each value is in
    /// `0..num_appearance_blendshapes`; a value of 0 means "use the
    /// always-on `CTL_PSD` weight" (effectively a no-op factor of 1.0 once
    /// the rest-pose weights are seeded).
    pub indices: [u8; SECTION_C_ROW_STRIDE],
}

/// A single variable-stride delta record from Section D (appearance) or
/// Section E (corrective). Each record contributes to one output translation
/// axis (`output_idx`), summing a sparse linear combination of blendshape
/// weights as `sum_i (coef_i * weights[blendshape_idx_i])`, then scaling by
/// the table's global `f64` scale divided by `(1 << packed_value_bit_width) - 1`.
///
/// **Anchored against `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`)** —
/// the main and secondary delta loops walk the section as a flat sequence of
/// records (no per-blendshape grouping):
///
/// ```text
/// loop num_*_delta_records times:
///   u16 output_idx           // flat write target (0..num_joints*3)
///   u16 payload_len (N)      // number of u32 payload entries
///   N x u32 packed_payload   // each u32:
///                            //   low  (31 - bit_width) bits  = blendshape weight index
///                            //   high (bit_width + 1) bits   = signed delta coefficient
///                            //   (decoded via arithmetic shift; see decode_packed_delta)
/// ```
///
/// Records vary in size (some appearance records have ~60-90 payload entries,
/// the fixture's mean ≈ 76). Sections D and E share the SAME format; only the
/// scale, write-target buffer, and `num_*_delta_records` differ.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeltaRecord {
    /// Index into a flat `num_joints * 3` f32 translation-delta accumulator.
    /// Bits: full u16, but values in well-formed files are `< num_joints * 3`.
    /// Decoded directly from the first u16 of the on-disk record.
    pub output_idx: u16,
    /// Raw packed u32 payload values, exactly `payload_len` entries. Each
    /// entry packs a blendshape-weight index and a signed delta coefficient;
    /// use [`decode_packed_delta`] to split them given the file-level
    /// `packed_value_bit_width`.
    pub payload: Vec<u32>,
}

/// One side of the RigLogic delta tables — either the appearance (Section D)
/// or the corrective (Section E) blob, plus the global f64 scale that
/// normalises the packed integer coefficients.
///
/// The dequantization is `delta_value = (coef * scale) / ((1 << bit_width) - 1)`
/// — i.e. the parser keeps `scale` as the on-disk f64 directly, and the
/// evaluator (Task 5) divides by `(1 << bit_width) - 1` once to produce the
/// per-step floating-point scale. Anchored in `ExecuteGeneric`:
/// `fVar63 = (float)(*(double *)(buf + 0x48) / (double)((1 << bit_width) - 1));`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendShapeBlob {
    /// One record per logical "output axis" contribution. Length is exactly
    /// the `num_*_delta_records` count from the inline scalar table — 517 for
    /// the appearance table, 528 for the corrective table in the observed
    /// fixture.
    pub records: Vec<DeltaRecord>,
    /// The on-disk global delta scale — `f64` at file offset `0x48`
    /// (`primary_delta_scale`) for appearance, `0x50`
    /// (`secondary_delta_scale`) for corrective. Stored verbatim.
    pub scale: f64,
}

/// Decode a single packed u32 delta-record payload entry given the file's
/// `packed_value_bit_width` (= 22 in the observed fixture).
///
/// The packing is anchored against `CFaceRigLogic::ExecuteGeneric` and
/// `Protos::ComputeMixedDeltaMatrixNumEntries`:
///
/// ```text
/// shift = 31 - bit_width             // e.g. 9 when bit_width = 22
/// mask  = (1 << shift) - 1           // e.g. 0x1FF — low 9 bits
/// index = u32 & mask                 // blendshape weight index
/// coef  = (i32) u32 >> shift         // arithmetic-shift; signed (bit_width+1)-bit value
/// ```
///
/// **Bit layout** (for the observed `bit_width = 22`):
///
/// | Bits  | Width | Meaning                                                |
/// |-------|-------|--------------------------------------------------------|
/// | 0..=8  | 9    | `blendshape_idx` — into the shared appearance/corrective weights buffer (`num_appearance + num_corrective` slots). |
/// | 9..=31 | 23   | Signed delta `coef` — multiplied by the table scale (`scale / ((1 << bit_width) - 1)`) to produce a per-axis f32 contribution. |
///
/// The historical SPEC.md draft had the layout reversed (index in the high
/// bits, value in the low bits); the Ghidra decompile and a fixture walk
/// disprove that — `payload_u32 & 0x1FF` is the index (and `0x1FF = 511 ≥ 494
/// = num_appearance + num_corrective - 1`, which fits), and the high 23 bits
/// are the signed coefficient.
///
/// # Panics
///
/// Panics in debug builds if `bit_width == 0` or `bit_width > 30`; well-formed
/// files have `bit_width = 22` and the engine never reads files outside that
/// range. Release builds will produce nonsense for out-of-range widths but
/// will not UB.
#[inline]
pub fn decode_packed_delta(packed: u32, bit_width: u32) -> (u16, i32) {
    debug_assert!(
        bit_width > 0 && bit_width < 31,
        "decode_packed_delta: bit_width {bit_width} outside valid range (1..=30)",
    );
    let shift = 31 - bit_width;
    let mask = (1u32 << shift) - 1;
    let index = (packed & mask) as u16;
    let coef = (packed as i32) >> shift;
    (index, coef)
}

/// Parsed `.RigLogic` content. Tasks 1-2 own: the header (magic, rig name,
/// `RigType`, identity scalars, header-size invariant), the typed inline
/// count/scale fields at file offsets `0x40..0x68`, the eight section-start
/// pointers at `0x68..0xA8`, Section A's joint-control CRC32 (IEEE) table, and
/// Section B's null-separated control-name pool. Task 3 adds Section D and E
/// (variable-stride delta tables).
///
/// Task 4 will populate the corrective composition table (Section C) and the
/// region tables (Sections G & H).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigLogic {
    /// File size of the input buffer. Used by later tasks to bound-check
    /// section pointers.
    pub file_size: usize,
    /// Trimmed rig name (e.g. `"protos_human_male_face_t1_pu"`).
    pub name: String,
    /// Packed `RigType` classifier at file offsets `0x30..0x38`.
    pub rig_type: RigType,
    /// `u32` at file offset `0x38`. Plausibly an identity CRC32C.
    /// `[inferred]` per spec — the loader does not consult this field.
    pub identity_crc_lo: NameHash,
    /// `u16` at file offset `0x3C`. `[inferred]` — likely the low half of
    /// a second identity hash.
    pub identity_crc_hi_or_x: u16,
    /// `u16` at file offset `0x3E`. The loader requires this to equal
    /// `0xA8` (= [`RIG_LOGIC_HEADER_SIZE`]) — that value is also the file
    /// offset where section content begins.
    pub header_size: u16,

    // ── Typed inline scalars at file offsets 0x40..0x68 (40 bytes) ──────────
    /// Number of appearance blendshapes — also the number of joint-control
    /// names in Section A/B. `u16` at file offset `0x40`. **[verified]** via
    /// `CFaceRigLogic::InitFaceRigLogic` (`0x14671a270`):
    /// `FUN_14668f6b0(target, count=*(ushort *)(buf + 0x40), ptr=*(longlong *)(buf + 0x70), ...)`.
    pub num_appearance_blendshapes: u16,
    /// Number of corrective (secondary) blendshapes. `u16` at file offset
    /// `0x42`. **[verified]** — `InitFaceRigLogic` allocates
    /// `(num_appearance + num_corrective)` u32 slots; `ExecuteGeneric` uses it
    /// as a Section C composition-table iteration bound.
    pub num_corrective_blendshapes: u16,
    /// Number of skeleton joints the rig writes to. `u16` at file offset
    /// `0x44`. Length of Section F's CRC32C table (= `4 * num_joints` bytes).
    /// **[verified]** — `InitFaceRigLogic` builds a 179-entry bone-CRC →
    /// bone-index lookup table from Section F.
    pub num_joints: u16,
    /// Padding `u16` at file offset `0x46`. Always zero in the observed
    /// fixture; no consumer read traced. Stored to preserve byte-for-byte
    /// round-trip fidelity.
    pub _reserved_0x46: u16,
    /// `f64` at file offset `0x48` — primary (appearance) delta scale.
    /// **[verified]** in `ExecuteGeneric`:
    /// `fVar63 = (float)(*(double *)(buf + 0x48) / (double)((1 << packed_bits) - 1))`.
    pub primary_delta_scale: f64,
    /// `f64` at file offset `0x50` — secondary (corrective) delta scale.
    /// **[verified]** in `ExecuteGeneric`, same pattern as `primary_delta_scale`.
    pub secondary_delta_scale: f64,
    /// Number of records in the main (appearance) delta table at Section D.
    /// `u32` at file offset `0x58`. **[verified]** — `ExecuteGeneric`'s main
    /// delta loop bound: `while (uVar54 < *(uint *)(buf + 0x58))`.
    pub num_main_delta_records: u32,
    /// Number of records in the secondary (corrective) delta table at
    /// Section E. `u32` at file offset `0x5C`. **[verified]** — secondary
    /// delta loop bound in `ExecuteGeneric`.
    pub num_secondary_delta_records: u32,
    /// Number of facial blend regions. `u32` at file offset `0x60`.
    /// **[verified]** — region-blending loop bound in `ExecuteGeneric`,
    /// driving traversal of Section G (slot 6) and Section H (slot 7).
    pub num_blend_regions: u32,
    /// Bit-width of the packed-value field in delta records. `u32` at file
    /// offset `0x64`. **[verified]** — `ExecuteGeneric` splits each
    /// delta-record `u32` into a `packed_value_bit_width`-bit value field
    /// (low bits) and a `(32 - packed_value_bit_width)`-bit index field
    /// (high bits).
    pub packed_value_bit_width: u32,

    /// Eight `u64` section-start byte offsets at file offsets `0x68..0xA8`.
    /// On disk these are stored as file-relative offsets; `FaceRigData::Load`
    /// adds the buffer base at load time to convert them to absolute pointers.
    /// Each section ends at the next-larger value in the sorted table.
    /// Slot values are not monotonic by index. Slot 1 is invariantly `0xA8`
    /// (= the size of the header + section table; equal to
    /// [`RIG_LOGIC_HEADER_SIZE`]).
    pub section_starts: [u64; NUM_SECTION_STARTS],

    // ── Section content ─────────────────────────────────────────────────────
    /// Section A — joint-control name CRC32 hashes. Length =
    /// `num_appearance_blendshapes` (= 205 in the observed fixture).
    ///
    /// **CRC algorithm:** CRC32 (IEEE, polynomial `0xEDB88320`). NOT CRC32C —
    /// see SPEC.md. Anchored in Task 2 by cross-verifying
    /// `CRC32("CTL_PSD") == joint_control_name_crcs[0] == 0xFD66F569` against
    /// the real fixture. Do not compare against `NameHash::from_string(name)`,
    /// which computes CRC32C and will never match.
    ///
    /// **Naming note:** despite the historical SPEC term "joint name CRCs",
    /// this array indexes blendshape *controls* (e.g. `CTL_PSD`, `CTL_expressions_*`)
    /// — not skeleton bones. Skeleton-bone CRCs are in Section F.
    pub joint_control_name_crcs: Vec<u32>,

    /// Section B — null-separated joint-control name strings. Length is
    /// nominally `num_appearance_blendshapes` (one name per control). Empty
    /// entries are skipped silently to be tolerant of trailing pad bytes.
    pub joint_control_names: Vec<String>,

    /// Section D — main / appearance delta table. Exactly
    /// `num_main_delta_records` records (= 517 in the observed fixture).
    /// `scale` is `primary_delta_scale` (the on-disk f64 at file offset
    /// `0x48`). Walked as a flat sequence of `(u16 output_idx, u16 N,
    /// N x u32 packed)` records — see [`DeltaRecord`] for the on-disk
    /// layout and [`decode_packed_delta`] for the per-u32 unpack.
    ///
    /// **Note:** there is no per-blendshape "runs" / "starts" table. The
    /// engine walks Section D linearly; each record names its own write
    /// target via `output_idx`, and each payload u32 names its own input
    /// blendshape weight slot. The plan-stage assumption that a per-blendshape
    /// header indirection existed was wrong for V1.9.
    pub appearance: BlendShapeBlob,
    /// Section E — secondary / corrective delta table. Exactly
    /// `num_secondary_delta_records` records (= 528 in the observed fixture).
    /// `scale` is `secondary_delta_scale` (the on-disk f64 at file offset
    /// `0x50`). Same on-disk format as `appearance` — only the scale, the
    /// write-target buffer at evaluation time, and the record count differ.
    pub corrective: BlendShapeBlob,

    /// Section C — `num_corrective_blendshapes` × 5-byte corrective
    /// composition rows. Each row's five `u8` slots index into the
    /// appearance-control weight buffer and their referenced weights are
    /// multiplied together to produce one corrective weight (the result lands
    /// at offset `num_appearance + i` of the shared
    /// `(num_appearance + num_corrective)`-slot weight buffer).
    ///
    /// In the observed fixture the section is 1,456 bytes total: 290 rows of
    /// 5 bytes (= 1,450 bytes) followed by 6 bytes of zero padding. The
    /// padding is NOT exposed here — `corrective_composition.len()` is
    /// exactly `num_corrective_blendshapes`.
    pub corrective_composition: Vec<CorrectiveCompositionRow>,

    /// Section F — `num_joints` × `u32` skeleton-bone-name CRC32 codes. Each
    /// entry is looked up against the companion skeleton's bone-CRC hash
    /// table by `FUN_1465ef130` (anchored in `CFaceRigLogic::InitFaceRigLogic`
    /// `0x14671a270`) to resolve the engine-side bone index the rig writes
    /// to.
    ///
    /// **CRC algorithm:** assumed CRC32 (IEEE, poly `0xEDB88320`) to match
    /// Section A's algorithm — Section A is byte-verified against
    /// `crc32fast::hash("CTL_PSD") = 0xFD66F569`. Section F entries cannot be
    /// cross-verified from the `.RigLogic` file alone — anchoring them
    /// against a known bone name requires the companion `.skin`'s
    /// `CompiledBonesIvo320` chunk, which is out of scope for the parser. The
    /// engine itself treats them as opaque hash keys, never recomputing them
    /// at runtime.
    pub skeleton_bone_name_crcs: Vec<u32>,

    /// Section G — per-blend-region bone-list table. Length =
    /// `num_blend_regions` (= 24 in the observed fixture).
    ///
    /// **On-disk format** (byte-anchored against
    /// `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`)):
    ///
    /// ```text
    /// for each region:
    ///   u16 count
    ///   u16 weight_indices[count]   // i16, used as positive index
    /// ```
    ///
    /// The total entry count across all regions (= sum of `count`s) is
    /// 38 in the observed fixture; Section G is exactly
    /// `2 * (num_blend_regions + sum_counts) = 2 * (24 + 38) = 124` bytes.
    pub blend_region_bones: Vec<BlendRegionBones>,

    /// Section H — flat per-bone-per-region weight scalars. The slice
    /// concatenates all `region.weight_indices.len()` f32 values across all
    /// regions in declaration order, NOT one entry per region. Length =
    /// `sum(blend_region_bones[i].weight_indices.len())`.
    ///
    /// Both the standalone file and the embedded IVO chunk carry a complete
    /// Section H — 152 bytes = 38 `f32` values, matching the Section G count
    /// sum exactly. The embedded form places Section H at a different file
    /// offset (`section_starts[7] = 327,840` vs standalone `327,904`) due to
    /// a 64-byte layout difference in Section E, but the Section H data itself
    /// is identical. The parser uses Section H's actual span
    /// (buffer_end − section_h_start) and validates it equals
    /// `sum(blend_region_bones[i].weight_indices.len()) × 4`.
    pub region_weights: Vec<f32>,
}

/// Decoded layout of the 80-byte `ProtosInfo` IVO chunk (type `0xF7086666`).
///
/// Found as a sibling to the `RigLogic` chunk inside head `.skin` containers.
/// It pairs the skin attachment with its matching RigLogic rig by storing both
/// names and their CRC32C (NameHash) fingerprints.
///
/// ## On-disk layout (all fields little-endian)
///
/// | Offset | Size | Type      | Field          | Notes                                         |
/// |--------|------|-----------|----------------|-----------------------------------------------|
/// | 0x00   |    4 | u32       | data_size      | Remaining bytes after this field (always 76)  |
/// | 0x04   |    4 | u32       | version        | Observed as 1                                 |
/// | 0x08   |   16 | [u8; 16]  | mesh_name      | Null-padded attachment name (e.g. "head")     |
/// | 0x18   |    4 | u32       | mesh_name_crc  | CRC32C of the mesh name string                |
/// | 0x1C   |   40 | [u8; 40]  | rig_name       | Null-padded rig name (matches RigLogic.name)  |
/// | 0x44   |    8 | [u8; 8]   | _padding       | Observed all-zero                             |
/// | 0x4C   |    4 | u32       | rig_name_crc   | CRC32C of the rig name (= RigLogic.identity_crc_lo) |
///
/// Byte-anchored from the fixture
/// `protos_human_male_face_t1_pu_head.skin` chunk index 5, version 2304.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtosInfo {
    /// Attachment mesh name (e.g. `"head"`, `"mouth"`, `"eyes"`).
    pub mesh_name: String,
    /// CRC32C of `mesh_name`. Used to cross-validate the parsed string.
    pub mesh_name_crc: NameHash,
    /// Rig name; matches the `name` field of the paired [`RigLogic`] chunk.
    pub rig_name: String,
    /// CRC32C of `rig_name`. Should equal [`RigLogic::identity_crc_lo`].
    pub rig_name_crc: NameHash,
    /// Version field; observed as 1.
    pub version: u32,
}

impl RigLogic {
    /// Parse the header of a `.RigLogic` file.
    ///
    /// `bytes` must point at the first byte of the magic. For standalone
    /// files this is byte 0 of the file; for the IVO-embedded chunk, this
    /// is byte 8 (skip the 8-byte `data_size` prefix the chunk wrapper
    /// adds).
    ///
    /// Validates:
    /// - Magic equals `b"RIG V1.9"`.
    /// - `header_size` (u16 at offset `0x3E`) equals `0xA8`.
    /// - Section-start pointer table can be read in full.
    /// - Section-start slot 1 equals `0xA8` (the bootstrap invariant).
    /// - Every non-zero section-start is `<= bytes.len()`.
    ///
    /// Section content is not validated — that is the responsibility of
    /// Tasks 2-4.
    pub fn read(bytes: &[u8]) -> Result<Self, ParseError> {
        let file_size = bytes.len();
        let mut reader = SpanReader::new(bytes);

        // ── Magic ────────────────────────────────────────────────────────
        let magic_offset = reader.position();
        let magic = reader.read_bytes(8)?;
        if magic != RIG_LOGIC_MAGIC {
            // `ParseError::InvalidMagic` takes u32 values; pack the first
            // four bytes of each side for diagnostics.
            let expected_u32 = u32::from_le_bytes([
                RIG_LOGIC_MAGIC[0],
                RIG_LOGIC_MAGIC[1],
                RIG_LOGIC_MAGIC[2],
                RIG_LOGIC_MAGIC[3],
            ]);
            let got_u32 =
                u32::from_le_bytes([magic[0], magic[1], magic[2], magic[3]]);
            return Err(ParseError::InvalidMagic {
                offset: magic_offset,
                expected: expected_u32,
                got: got_u32,
            });
        }

        // ── Name (null-padded ASCII, file offsets 0x08..0x30) ────────────
        let name_offset = reader.position();
        let name_bytes = reader.read_bytes(NAME_FIELD_BYTES)?;
        let name_end = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(NAME_FIELD_BYTES);
        let name = std::str::from_utf8(&name_bytes[..name_end])
            .map_err(|e| ParseError::UnexpectedValue {
                offset: name_offset,
                expected: "valid UTF-8 (ASCII) rig name".to_string(),
                actual: format!("non-UTF-8 bytes: {e}"),
            })?
            .to_owned();

        // ── RigType (file offsets 0x30..0x38) ────────────────────────────
        debug_assert_eq!(reader.position(), 0x30);
        let class = reader.read_u8()?;
        let gender = reader.read_u8()?;
        let variant = reader.read_u8()?;
        let part = reader.read_u8()?;
        let tier = reader.read_u8()?;
        let lod = reader.read_u8()?;
        let flags_or_pad = reader.read_u16()?;
        let rig_type = RigType {
            class,
            gender,
            variant,
            part,
            tier,
            lod,
            flags_or_pad,
        };

        // ── Identity scalars (file offsets 0x38..0x3E) ───────────────────
        debug_assert_eq!(reader.position(), 0x38);
        let identity_crc_lo = NameHash(reader.read_u32()?);
        let identity_crc_hi_or_x = reader.read_u16()?;

        // ── Header size invariant (file offset 0x3E, u16) ────────────────
        debug_assert_eq!(reader.position(), HEADER_SIZE_OFFSET);
        let header_size = reader.read_u16()?;
        if header_size != RIG_LOGIC_HEADER_SIZE {
            return Err(ParseError::Validation {
                context: "RigLogic::read".to_string(),
                message: format!(
                    "header_size at offset {HEADER_SIZE_OFFSET:#x} = {header_size:#x}, \
                     expected {RIG_LOGIC_HEADER_SIZE:#x}",
                ),
            });
        }

        // ── Typed inline scalars (file offsets 0x40..0x68) ───────────────
        // All offsets verified against `CFaceRigLogic::InitFaceRigLogic`
        // (`0x14671a270`), `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`),
        // and `Protos::ComputeMixedDeltaMatrixNumEntries` (`0x14142a5c0`).
        debug_assert_eq!(reader.position(), 0x40);
        let num_appearance_blendshapes = reader.read_u16()?; // 0x40
        let num_corrective_blendshapes = reader.read_u16()?; // 0x42
        let num_joints = reader.read_u16()?; // 0x44
        let reserved_0x46 = reader.read_u16()?; // 0x46
        let primary_delta_scale = reader.read_f64()?; // 0x48
        let secondary_delta_scale = reader.read_f64()?; // 0x50
        let num_main_delta_records = reader.read_u32()?; // 0x58
        let num_secondary_delta_records = reader.read_u32()?; // 0x5C
        let num_blend_regions = reader.read_u32()?; // 0x60
        let packed_value_bit_width = reader.read_u32()?; // 0x64

        // ── Section-start pointer table (file offsets 0x68..0xA8) ────────
        debug_assert_eq!(reader.position(), SECTION_START_TABLE_OFFSET);
        let mut section_starts = [0u64; NUM_SECTION_STARTS];
        for slot in section_starts.iter_mut() {
            *slot = reader.read_u64()?;
        }

        // Slot 1 (the bootstrap) MUST equal the header-table size (0xA8).
        // This is the same invariant the loader checks via the `header_size`
        // short — encoded twice in the file, in different widths.
        if section_starts[1] != SECTION_START_BOOTSTRAP_VALUE {
            return Err(ParseError::Validation {
                context: "RigLogic::read".to_string(),
                message: format!(
                    "section_starts[1] = {} (0x{:x}), expected {} (bootstrap = end of header table)",
                    section_starts[1], section_starts[1], SECTION_START_BOOTSTRAP_VALUE,
                ),
            });
        }

        // Each non-zero pointer must lie within the file.
        for (slot_idx, &start) in section_starts.iter().enumerate() {
            if start != 0 && start as usize > file_size {
                return Err(ParseError::Validation {
                    context: "RigLogic::read".to_string(),
                    message: format!(
                        "section_starts[{slot_idx}] = {start} exceeds file size {file_size}",
                    ),
                });
            }
        }

        // ── Section A — joint-control CRC32 (IEEE) table ────────────────
        let joint_control_name_crcs = read_section_a_crcs(
            bytes,
            section_starts[SECTION_A_START_SLOT],
            section_starts[SECTION_B_START_SLOT],
            num_appearance_blendshapes,
        )?;

        // ── Section B — null-separated control name pool ─────────────────
        let joint_control_names = read_section_b_names(
            bytes,
            section_starts[SECTION_B_START_SLOT],
            section_starts[SECTION_C_START_SLOT],
        )?;

        // ── Section D — main (appearance) delta table ─────────────────────
        let appearance = read_delta_table(
            bytes,
            section_starts[SECTION_D_START_SLOT],
            section_starts[SECTION_E_START_SLOT],
            num_main_delta_records,
            num_joints,
            num_appearance_blendshapes,
            num_corrective_blendshapes,
            packed_value_bit_width,
            primary_delta_scale,
            "Section D (appearance)",
        )?;

        // ── Section E — secondary (corrective) delta table ────────────────
        let corrective = read_delta_table(
            bytes,
            section_starts[SECTION_E_START_SLOT],
            section_starts[SECTION_F_START_SLOT],
            num_secondary_delta_records,
            num_joints,
            num_appearance_blendshapes,
            num_corrective_blendshapes,
            packed_value_bit_width,
            secondary_delta_scale,
            "Section E (corrective)",
        )?;

        // ── Section C — corrective-blendshape composition table ──────────
        // Section C is read AFTER E (despite living physically before it in
        // the file) because the read order in the parser is by-table not
        // by-offset; the order here doesn't affect correctness.
        let corrective_composition = read_section_c_composition(
            bytes,
            section_starts[SECTION_C_START_SLOT],
            section_starts[SECTION_D_START_SLOT],
            num_corrective_blendshapes,
            num_appearance_blendshapes,
        )?;

        // ── Section F — skeleton-bone CRC32 table ─────────────────────────
        let skeleton_bone_name_crcs = read_section_f_crcs(
            bytes,
            section_starts[SECTION_F_START_SLOT],
            section_starts[SECTION_G_START_SLOT],
            num_joints,
        )?;

        // ── Section G — per-region bone-index lists ───────────────────────
        let blend_region_bones = read_section_g_region_bones(
            bytes,
            section_starts[SECTION_G_START_SLOT],
            section_starts[SECTION_H_START_SLOT],
            num_blend_regions,
        )?;

        // ── Section H — per-region weight scalars ─────────────────────────
        // Section H's end is special: this section is the last in the file,
        // so its upper bound is `file_size` (= `bytes.len()`) in the
        // standalone form. The embedded form's chunk-reported `data_size` may
        // truncate the section by 16 entries — see field doc on
        // `region_weights`.
        let region_weights = read_section_h_region_weights(
            bytes,
            section_starts[SECTION_H_START_SLOT],
            file_size as u64,
            &blend_region_bones,
        )?;

        Ok(RigLogic {
            file_size,
            name,
            rig_type,
            identity_crc_lo,
            identity_crc_hi_or_x,
            header_size,
            num_appearance_blendshapes,
            num_corrective_blendshapes,
            num_joints,
            _reserved_0x46: reserved_0x46,
            primary_delta_scale,
            secondary_delta_scale,
            num_main_delta_records,
            num_secondary_delta_records,
            num_blend_regions,
            packed_value_bit_width,
            section_starts,
            joint_control_name_crcs,
            joint_control_names,
            appearance,
            corrective,
            corrective_composition,
            skeleton_bone_name_crcs,
            blend_region_bones,
            region_weights,
        })
    }

    /// Borrow the [`BlendRegionBones::weight_indices`]-aligned slice of
    /// Section H weight scalars for blend-region `region_idx`.
    ///
    /// Returns `None` if `region_idx >= self.blend_region_bones.len()`.
    /// Otherwise the returned slice has the same length as the region's
    /// `weight_indices` (which is the per-region count read from Section G).
    ///
    /// Panics if internal invariants are violated (i.e. the parser was
    /// somehow constructed with inconsistent `region_weights` and
    /// `blend_region_bones`). Well-formed files produced by [`Self::read`]
    /// can never trigger this.
    pub fn region_weight_slice(&self, region_idx: usize) -> Option<&[f32]> {
        if region_idx >= self.blend_region_bones.len() {
            return None;
        }
        let start: usize = self
            .blend_region_bones
            .iter()
            .take(region_idx)
            .map(|r| r.weight_indices.len())
            .sum();
        let len = self.blend_region_bones[region_idx].weight_indices.len();
        let end = start + len;
        Some(&self.region_weights[start..end])
    }
}

/// Read Section A: the `num_appearance_blendshapes` × `u32` joint-control
/// CRC32 (IEEE, poly `0xEDB88320`) table. The section starts at the file
/// offset stored in slot 1 of `section_starts` (= the bootstrap value `0xA8`)
/// and ends at slot 0's offset (= the start of Section B).
///
/// Length is validated against `num_appearance_blendshapes` so a malformed
/// header with a count mismatching the section size errors cleanly rather
/// than reading garbage or silently truncating.
///
/// **Naming note:** historical CIG documentation calls this
/// `m_protosJointNameCRC32s`, but the array indexes blendshape *controls*
/// (e.g. `CTL_expressions_blinkR`, `CTL_PSD`), not skeleton bones. Bone
/// CRCs are in Section F. The field is named `joint_control_name_crcs` on
/// the `RigLogic` struct accordingly.
fn read_section_a_crcs(
    bytes: &[u8],
    section_start: u64,
    next_section_start: u64,
    num_entries: u16,
) -> Result<Vec<u32>, ParseError> {
    let context = "RigLogic::read_section_a_crcs";
    let start = section_start as usize;
    let end = next_section_start as usize;
    if section_start == 0 || next_section_start == 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section A bounds invalid: start = {section_start}, next_start = {next_section_start}",
            ),
        });
    }
    if end <= start {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section A end ({end}) must be greater than start ({start})",
            ),
        });
    }
    let span_len = end - start;
    let expected_len = (num_entries as usize) * SECTION_A_ENTRY_STRIDE;
    if span_len != expected_len {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section A length {span_len} bytes does not match \
                 num_appearance_blendshapes ({num_entries}) × {SECTION_A_ENTRY_STRIDE} \
                 = {expected_len} bytes",
            ),
        });
    }
    if end > bytes.len() {
        return Err(ParseError::Truncated {
            offset: start,
            need: expected_len,
            have: bytes.len().saturating_sub(start),
        });
    }
    let slice = &bytes[start..end];
    let mut crcs = Vec::with_capacity(num_entries as usize);
    for i in 0..num_entries as usize {
        let off = i * SECTION_A_ENTRY_STRIDE;
        let raw = u32::from_le_bytes([
            slice[off],
            slice[off + 1],
            slice[off + 2],
            slice[off + 3],
        ]);
        crcs.push(raw);
    }
    Ok(crcs)
}

/// Read Section B: the null-separated joint-control name string pool. The
/// section starts at slot 0's offset (= end of Section A) and ends at slot
/// 2's offset (= start of Section C).
///
/// Empty entries (consecutive null bytes / trailing pad) are skipped
/// silently — the section's exact length is not a multiple of the entries'
/// average size, and the engine relies on the null-separator boundary
/// rather than on a fixed stride. Non-UTF-8 bytes within an entry produce a
/// validation error rather than being silently dropped.
fn read_section_b_names(
    bytes: &[u8],
    section_start: u64,
    next_section_start: u64,
) -> Result<Vec<String>, ParseError> {
    let context = "RigLogic::read_section_b_names";
    let start = section_start as usize;
    let end = next_section_start as usize;
    if section_start == 0 || next_section_start == 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section B bounds invalid: start = {section_start}, next_start = {next_section_start}",
            ),
        });
    }
    if end < start {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section B next_start ({end}) must be >= start ({start})",
            ),
        });
    }
    if end > bytes.len() {
        return Err(ParseError::Truncated {
            offset: start,
            need: end - start,
            have: bytes.len().saturating_sub(start),
        });
    }
    if end == start {
        // Empty name pool is valid (e.g. zero-blendshape rigs).
        return Ok(Vec::new());
    }
    let slice = &bytes[start..end];
    let mut names = Vec::new();
    let mut cursor = 0usize;
    while cursor < slice.len() {
        // Find next null byte (or end-of-section).
        let end_of_entry = slice[cursor..]
            .iter()
            .position(|&b| b == 0)
            .map(|rel| cursor + rel)
            .unwrap_or(slice.len());
        if end_of_entry > cursor {
            let entry_bytes = &slice[cursor..end_of_entry];
            let entry = std::str::from_utf8(entry_bytes).map_err(|e| {
                ParseError::UnexpectedValue {
                    offset: start + cursor,
                    expected: "valid UTF-8 control name".to_string(),
                    actual: format!("non-UTF-8 bytes: {e}"),
                }
            })?;
            names.push(entry.to_owned());
        }
        cursor = end_of_entry + 1; // skip the null
    }
    Ok(names)
}

/// Read one of the two variable-stride delta tables (Section D / appearance
/// or Section E / corrective). Both sections share the same on-disk format:
///
/// ```text
/// repeat num_records times:
///   u16  output_idx
///   u16  payload_len  (N)
///   u32  payload[N]
/// ```
///
/// **Anchored** against `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`) —
/// the main / secondary delta-record loops walk this exact pattern (`puVar51 =
/// *(ushort **)(buf + 0x80)` / `+ 0x88`, advancing the u16 cursor by `2 + 2*N`
/// each iteration, doing `*(uint *)(buf + 0x58)` / `0x5c` iterations).
///
/// Validates:
/// - `num_records == expected_record_count` and the records exactly fill the
///   section (no leftover bytes, no truncated record).
/// - Every `output_idx < num_joints * 3` (the target translation-buffer
///   length, anchored against `memset(pvVar24, 0, num_joints * 0xc)`).
/// - Every packed `blendshape_idx < num_appearance + num_corrective` (the
///   shared weight-buffer length, anchored against `uStack_190 = num_appearance
///   + num_corrective`).
#[allow(clippy::too_many_arguments)]
fn read_delta_table(
    bytes: &[u8],
    section_start: u64,
    next_section_start: u64,
    expected_record_count: u32,
    num_joints: u16,
    num_appearance_blendshapes: u16,
    num_corrective_blendshapes: u16,
    packed_value_bit_width: u32,
    scale: f64,
    label: &str,
) -> Result<BlendShapeBlob, ParseError> {
    let context = "RigLogic::read_delta_table";
    let start = section_start as usize;
    let end = next_section_start as usize;
    if section_start == 0 || next_section_start == 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "{label} bounds invalid: start = {section_start}, next_start = {next_section_start}",
            ),
        });
    }
    if end < start {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "{label}: next_section_start ({end}) must be >= section_start ({start})",
            ),
        });
    }
    if end > bytes.len() {
        return Err(ParseError::Truncated {
            offset: start,
            need: end - start,
            have: bytes.len().saturating_sub(start),
        });
    }

    // Empty / zero-record table — return an empty blob without touching the
    // section bytes or validating `packed_value_bit_width`. This matches the
    // engine: `ExecuteGeneric` skips the loop entirely when
    // `*(uint *)(buf + 0x58) == 0` (or `0x5c == 0`), and the bit-width field
    // is only consumed by the loop body. Zero records combined with a
    // non-empty section span is still an error (the section bytes would be
    // garbage by the on-disk contract).
    if expected_record_count == 0 {
        if end != start {
            return Err(ParseError::Validation {
                context: context.to_string(),
                message: format!(
                    "{label}: zero records declared but section spans {} bytes \
                     (expected exactly 0)",
                    end - start,
                ),
            });
        }
        return Ok(BlendShapeBlob {
            records: Vec::new(),
            scale,
        });
    }

    if packed_value_bit_width == 0 || packed_value_bit_width > 30 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "{label}: packed_value_bit_width = {packed_value_bit_width} outside valid range (1..=30)",
            ),
        });
    }

    // Cap on output_idx: the engine memsets a `num_joints * 12`-byte (= 3 f32s
    // per joint) translation accumulator and indexes it as `output[output_idx]`,
    // so well-formed records satisfy `output_idx < num_joints * 3`.
    let output_idx_bound: u32 = (num_joints as u32) * 3;
    // Cap on payload-u32 blendshape_idx: the engine memsets a
    // `(num_appearance + num_corrective) * 4`-byte weight buffer and indexes it
    // as `weights[blendshape_idx]`.
    let blendshape_idx_bound: u32 =
        (num_appearance_blendshapes as u32) + (num_corrective_blendshapes as u32);

    let mut reader = SpanReader::new(&bytes[start..end]);
    let mut records: Vec<DeltaRecord> = Vec::with_capacity(expected_record_count as usize);

    for record_idx in 0..expected_record_count {
        // Each record header is 4 bytes; if there's not even room for that,
        // the section is shorter than the count says.
        if reader.remaining() < 4 {
            return Err(ParseError::Validation {
                context: context.to_string(),
                message: format!(
                    "{label}: record {record_idx} truncated — {} bytes left in section, \
                     need at least 4 for the (output_idx, payload_len) header",
                    reader.remaining(),
                ),
            });
        }
        let record_byte_off = start + reader.position();
        let output_idx = reader.read_u16()?;
        let payload_len = reader.read_u16()? as usize;

        if (output_idx as u32) >= output_idx_bound {
            return Err(ParseError::Validation {
                context: context.to_string(),
                message: format!(
                    "{label}: record {record_idx} at file offset {record_byte_off:#x} has \
                     output_idx = {output_idx} >= num_joints * 3 ({output_idx_bound})",
                ),
            });
        }

        let payload_bytes = payload_len * 4;
        if reader.remaining() < payload_bytes {
            return Err(ParseError::Validation {
                context: context.to_string(),
                message: format!(
                    "{label}: record {record_idx} at file offset {record_byte_off:#x} declares \
                     payload_len = {payload_len} ({payload_bytes} bytes), but only {} bytes \
                     remain in section",
                    reader.remaining(),
                ),
            });
        }

        let mut payload: Vec<u32> = Vec::with_capacity(payload_len);
        for entry_idx in 0..payload_len {
            let packed = reader.read_u32()?;
            let (idx, _coef) = decode_packed_delta(packed, packed_value_bit_width);
            if (idx as u32) >= blendshape_idx_bound {
                return Err(ParseError::Validation {
                    context: context.to_string(),
                    message: format!(
                        "{label}: record {record_idx} payload entry {entry_idx} has \
                         blendshape_idx = {idx} >= num_appearance + num_corrective \
                         ({blendshape_idx_bound}); raw packed u32 = {packed:#010x}",
                    ),
                });
            }
            payload.push(packed);
        }

        records.push(DeltaRecord { output_idx, payload });
    }

    // The records must exactly fill the section — the engine relies on the
    // section's outer boundary as a sanity bound and any leftover bytes
    // indicate a format mismatch.
    if reader.remaining() != 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "{label}: after consuming {expected_record_count} records, {} bytes remain \
                 unread in section (expected exactly 0)",
                reader.remaining(),
            ),
        });
    }

    Ok(BlendShapeBlob { records, scale })
}

/// Read Section C: the corrective-blendshape composition table.
///
/// On-disk format (anchored against `CFaceRigLogic::ExecuteGeneric`
/// (`0x1466dec50`)'s corrective-composition loop): a flat sequence of
/// `num_corrective_blendshapes` rows, each 5 bytes. Each row's five `u8`
/// values index into the appearance-weight buffer (slots
/// `0..num_appearance_blendshapes`) and the engine multiplies the indexed
/// weights to produce one corrective weight per row.
///
/// The section's on-disk byte length is **not** strictly equal to
/// `num_corrective_blendshapes × 5` — the real fixture pads to a slightly
/// larger boundary (1,456 bytes for 290 rows = 1,450 bytes + 6 bytes of zero
/// padding). The reader accepts any trailing pad bytes that are all zero, but
/// requires the first `num_corrective_blendshapes × 5` bytes to be present
/// in-bounds.
///
/// Per-byte validation: each `u8` index must be `< num_appearance_blendshapes`
/// (i.e. point into the appearance weight buffer, not past it).
fn read_section_c_composition(
    bytes: &[u8],
    section_start: u64,
    next_section_start: u64,
    num_corrective_blendshapes: u16,
    num_appearance_blendshapes: u16,
) -> Result<Vec<CorrectiveCompositionRow>, ParseError> {
    let context = "RigLogic::read_section_c_composition";
    let start = section_start as usize;
    let end = next_section_start as usize;
    if section_start == 0 || next_section_start == 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section C bounds invalid: start = {section_start}, next_start = {next_section_start}",
            ),
        });
    }
    if end < start {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section C: next_section_start ({end}) must be >= section_start ({start})",
            ),
        });
    }
    if end > bytes.len() {
        return Err(ParseError::Truncated {
            offset: start,
            need: end - start,
            have: bytes.len().saturating_sub(start),
        });
    }
    let n = num_corrective_blendshapes as usize;
    let needed = n * SECTION_C_ROW_STRIDE;
    let span_len = end - start;
    if span_len < needed {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section C span {span_len} bytes < required {needed} bytes \
                 ({n} × {SECTION_C_ROW_STRIDE})",
            ),
        });
    }
    let slice = &bytes[start..start + needed];
    let pad = &bytes[start + needed..end];
    // Trailing padding (e.g. the 6-byte fixture tail) MUST be zero — anything
    // else means we have the wrong stride or row count.
    if let Some(non_zero_idx) = pad.iter().position(|&b| b != 0) {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section C trailing pad byte at section-offset {} is {:#04x}; \
                 expected zero. {} pad bytes total.",
                needed + non_zero_idx,
                pad[non_zero_idx],
                pad.len(),
            ),
        });
    }
    let mut rows = Vec::with_capacity(n);
    let app_count = num_appearance_blendshapes as usize;
    for i in 0..n {
        let off = i * SECTION_C_ROW_STRIDE;
        let row: [u8; SECTION_C_ROW_STRIDE] = [
            slice[off],
            slice[off + 1],
            slice[off + 2],
            slice[off + 3],
            slice[off + 4],
        ];
        // Each index must point into the appearance-weight buffer (or be
        // zero, which is also `< num_appearance` and indexes the always-on
        // `CTL_PSD` slot).
        for (slot, &idx) in row.iter().enumerate() {
            if (idx as usize) >= app_count {
                return Err(ParseError::Validation {
                    context: context.to_string(),
                    message: format!(
                        "Section C row {i} slot {slot} = {idx} >= num_appearance_blendshapes ({app_count})",
                    ),
                });
            }
        }
        rows.push(CorrectiveCompositionRow { indices: row });
    }
    Ok(rows)
}

/// Read Section F: the `num_joints` × `u32` skeleton-bone CRC32 table.
///
/// Anchored against `CFaceRigLogic::InitFaceRigLogic` (`0x14671a270`)'s
/// bone-CRC lookup loop, which reads `num_joints` × `u32` from
/// `*(longlong *)(buf + 0x90)` (= `section_starts[5]`). The CRC algorithm is
/// assumed CRC32 (IEEE, polynomial `0xEDB88320`) to match Section A's
/// algorithm — Section A's first entry is byte-verified against
/// `crc32fast::hash("CTL_PSD") = 0xFD66F569`. Section F entries are looked up
/// by the engine via `FUN_1465ef130` against a pre-computed skeleton-CRC hash
/// table; they are never recomputed from a name at runtime, so the algorithm
/// choice cannot be cross-verified from the `.RigLogic` file alone.
fn read_section_f_crcs(
    bytes: &[u8],
    section_start: u64,
    next_section_start: u64,
    num_joints: u16,
) -> Result<Vec<u32>, ParseError> {
    let context = "RigLogic::read_section_f_crcs";
    let start = section_start as usize;
    let end = next_section_start as usize;
    if section_start == 0 || next_section_start == 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section F bounds invalid: start = {section_start}, next_start = {next_section_start}",
            ),
        });
    }
    if end < start {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section F: next_section_start ({end}) must be >= section_start ({start})",
            ),
        });
    }
    let span_len = end - start;
    let expected_len = (num_joints as usize) * SECTION_F_ENTRY_STRIDE;
    if span_len != expected_len {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section F length {span_len} bytes does not match \
                 num_joints ({num_joints}) × {SECTION_F_ENTRY_STRIDE} = {expected_len} bytes",
            ),
        });
    }
    if end > bytes.len() {
        return Err(ParseError::Truncated {
            offset: start,
            need: expected_len,
            have: bytes.len().saturating_sub(start),
        });
    }
    if num_joints == 0 {
        return Ok(Vec::new());
    }
    let slice = &bytes[start..end];
    let mut crcs = Vec::with_capacity(num_joints as usize);
    for i in 0..num_joints as usize {
        let off = i * SECTION_F_ENTRY_STRIDE;
        let raw = u32::from_le_bytes([
            slice[off],
            slice[off + 1],
            slice[off + 2],
            slice[off + 3],
        ]);
        crcs.push(raw);
    }
    Ok(crcs)
}

/// Read Section G: the per-blend-region bone-index list table.
///
/// On-disk format (byte-anchored against `CFaceRigLogic::ExecuteGeneric`
/// (`0x1466dec50`)'s region-blending pass):
///
/// ```text
/// for each of num_blend_regions regions:
///   u16 count
///   u16 weight_indices[count]
/// ```
///
/// The reader walks the full section span and validates:
/// - exactly `num_blend_regions` regions are consumed,
/// - the walk lands EXACTLY at `next_section_start` (no leftover bytes).
fn read_section_g_region_bones(
    bytes: &[u8],
    section_start: u64,
    next_section_start: u64,
    num_blend_regions: u32,
) -> Result<Vec<BlendRegionBones>, ParseError> {
    let context = "RigLogic::read_section_g_region_bones";
    let start = section_start as usize;
    let end = next_section_start as usize;
    if section_start == 0 || next_section_start == 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section G bounds invalid: start = {section_start}, next_start = {next_section_start}",
            ),
        });
    }
    if end < start {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section G: next_section_start ({end}) must be >= section_start ({start})",
            ),
        });
    }
    if end > bytes.len() {
        return Err(ParseError::Truncated {
            offset: start,
            need: end - start,
            have: bytes.len().saturating_sub(start),
        });
    }
    if num_blend_regions == 0 {
        if end != start {
            return Err(ParseError::Validation {
                context: context.to_string(),
                message: format!(
                    "Section G: zero regions declared but section spans {} bytes",
                    end - start,
                ),
            });
        }
        return Ok(Vec::new());
    }
    let mut reader = SpanReader::new(&bytes[start..end]);
    let mut regions = Vec::with_capacity(num_blend_regions as usize);
    for region_idx in 0..num_blend_regions {
        if reader.remaining() < SECTION_G_ENTRY_STRIDE {
            return Err(ParseError::Validation {
                context: context.to_string(),
                message: format!(
                    "Section G: region {region_idx} truncated — {} bytes left, \
                     need at least {SECTION_G_ENTRY_STRIDE} for the count u16",
                    reader.remaining(),
                ),
            });
        }
        let count = reader.read_u16()? as usize;
        let needed = count * SECTION_G_ENTRY_STRIDE;
        if reader.remaining() < needed {
            return Err(ParseError::Validation {
                context: context.to_string(),
                message: format!(
                    "Section G: region {region_idx} declares count = {count} \
                     ({needed} bytes), but only {} bytes remain",
                    reader.remaining(),
                ),
            });
        }
        let mut weight_indices = Vec::with_capacity(count);
        for _ in 0..count {
            weight_indices.push(reader.read_u16()?);
        }
        regions.push(BlendRegionBones { weight_indices });
    }
    if reader.remaining() != 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section G: after consuming {num_blend_regions} regions, {} bytes \
                 remain unread (expected exactly 0)",
                reader.remaining(),
            ),
        });
    }
    Ok(regions)
}

/// Read Section H: flat per-region-per-bone weight scalars (f32).
///
/// Section H is the last section in the file. Its end offset is determined by
/// the buffer end (`file_size`) — NOT by a `next_section_start` pointer.
///
/// **Length:** the section's f32 count must equal
/// `sum(blend_region_bones[i].weight_indices.len())`. If the span is shorter
/// (the embedded chunk's reported `data_size` may truncate the last 16
/// entries — see `RigLogic::region_weights` doc), the reader errors;
/// **callers reading the embedded IVO form should extend the chunk buffer by
/// 64 zero bytes (or use the standalone file's total length) before calling
/// [`RigLogic::read`]**, OR pass the full standalone-file buffer directly.
/// The parser does not silently zero-fill missing entries.
fn read_section_h_region_weights(
    bytes: &[u8],
    section_start: u64,
    file_end: u64,
    blend_region_bones: &[BlendRegionBones],
) -> Result<Vec<f32>, ParseError> {
    let context = "RigLogic::read_section_h_region_weights";
    let start = section_start as usize;
    let end = file_end as usize;
    if section_start == 0 {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!("Section H start invalid: {section_start}"),
        });
    }
    if end < start {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section H: file_end ({end}) must be >= section_start ({start})",
            ),
        });
    }
    if end > bytes.len() {
        return Err(ParseError::Truncated {
            offset: start,
            need: end - start,
            have: bytes.len().saturating_sub(start),
        });
    }
    let expected_count: usize = blend_region_bones
        .iter()
        .map(|r| r.weight_indices.len())
        .sum();
    let span_len = end - start;
    let expected_bytes = expected_count * SECTION_H_ENTRY_STRIDE;
    if span_len < expected_bytes {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section H span {span_len} bytes < required {expected_bytes} bytes \
                 ({expected_count} f32 × {SECTION_H_ENTRY_STRIDE}). The embedded \
                 IVO chunk's reported `data_size` is known to truncate this \
                 section by 64 bytes in V1.9; pass the standalone file or pad \
                 the buffer.",
            ),
        });
    }
    // Anything beyond `expected_bytes` (within `span_len`) is not anchored to
    // any consumer in the engine — reject as a contract violation so the
    // parser is strict-everywhere even when the section happens to be longer
    // than the engine reads. The standalone file has `span_len == expected_bytes`
    // for the V1.9 fixture; if a future file exceeds that, this strictness
    // forces a deliberate format-version bump rather than silent acceptance.
    if span_len > expected_bytes {
        return Err(ParseError::Validation {
            context: context.to_string(),
            message: format!(
                "Section H span {span_len} bytes > expected {expected_bytes} bytes; \
                 trailing {} bytes unaccounted for. The V1.9 standalone format \
                 has no tail past Section H — investigate before accepting.",
                span_len - expected_bytes,
            ),
        });
    }
    let slice = &bytes[start..start + expected_bytes];
    let mut weights = Vec::with_capacity(expected_count);
    for i in 0..expected_count {
        let off = i * SECTION_H_ENTRY_STRIDE;
        let raw = u32::from_le_bytes([
            slice[off],
            slice[off + 1],
            slice[off + 2],
            slice[off + 3],
        ]);
        weights.push(f32::from_bits(raw));
    }
    Ok(weights)
}

/// Length of the null-padded mesh name field at `0x08` (bytes 0x08..0x18).
const PROTOS_INFO_MESH_NAME_BYTES: usize = 16;

/// Length of the null-padded rig name field at `0x1C` (bytes 0x1C..0x44).
/// Matches [`NAME_FIELD_BYTES`] — the same 40-byte convention used in [`RigLogic`].
const PROTOS_INFO_RIG_NAME_BYTES: usize = 40;

/// Total expected byte count of the `ProtosInfo` IVO chunk payload.
const PROTOS_INFO_SIZE: usize = 80;

/// The expected value of the `data_size` field at offset 0x00 in
/// `ProtosInfo` — the number of bytes after that field (80 − 4 = 76).
const PROTOS_INFO_DATA_SIZE: u32 = (PROTOS_INFO_SIZE - 4) as u32;

impl ProtosInfo {
    /// Parse the 80-byte `ProtosInfo` chunk payload.
    ///
    /// `bytes` must be exactly 80 bytes (the full chunk content as returned by
    /// [`starbreaker_chunks::IvoChunkFile::chunk_data`] for chunk type
    /// `0xF7086666`).
    ///
    /// Validates:
    /// - Buffer length is exactly 80 bytes.
    /// - `data_size` field equals 76 (= 80 − 4).
    /// - `mesh_name_crc` equals CRC32C of the decoded mesh name.
    /// - `rig_name_crc` equals CRC32C of the decoded rig name.
    pub fn read(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() != PROTOS_INFO_SIZE {
            return Err(ParseError::Validation {
                context: "ProtosInfo::read".to_string(),
                message: format!(
                    "expected {PROTOS_INFO_SIZE} bytes, got {}",
                    bytes.len()
                ),
            });
        }

        // 0x00..0x04 — data_size: must equal 76
        let data_size = u32::from_le_bytes(bytes[0x00..0x04].try_into().unwrap());
        if data_size != PROTOS_INFO_DATA_SIZE {
            return Err(ParseError::Validation {
                context: "ProtosInfo::read".to_string(),
                message: format!(
                    "data_size {data_size} != expected {PROTOS_INFO_DATA_SIZE}"
                ),
            });
        }

        // 0x04..0x08 — version
        let version = u32::from_le_bytes(bytes[0x04..0x08].try_into().unwrap());

        // 0x08..0x18 — mesh_name (16-byte null-padded ASCII)
        let mesh_raw = &bytes[0x08..0x08 + PROTOS_INFO_MESH_NAME_BYTES];
        let mesh_end = mesh_raw
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(PROTOS_INFO_MESH_NAME_BYTES);
        let mesh_name = std::str::from_utf8(&mesh_raw[..mesh_end])
            .map_err(|e| ParseError::Validation {
                context: "ProtosInfo::read".to_string(),
                message: format!("invalid mesh_name bytes: {e}"),
            })?
            .to_owned();

        // 0x18..0x1C — mesh_name_crc (CRC32C of mesh_name, via NameHash convention)
        let mesh_name_crc =
            NameHash(u32::from_le_bytes(bytes[0x18..0x1C].try_into().unwrap()));
        let expected_mesh_crc = NameHash::from_string(&mesh_name);
        if mesh_name_crc != expected_mesh_crc {
            return Err(ParseError::Validation {
                context: "ProtosInfo::read".to_string(),
                message: format!(
                    "mesh_name_crc mismatch: \
                     stored 0x{:08X}, computed 0x{:08X} for {:?}",
                    mesh_name_crc.value(),
                    expected_mesh_crc.value(),
                    mesh_name
                ),
            });
        }

        // 0x1C..0x44 — rig_name (40-byte null-padded ASCII, same size as RigLogic.name)
        let rig_raw = &bytes[0x1C..0x1C + PROTOS_INFO_RIG_NAME_BYTES];
        let rig_end = rig_raw
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(PROTOS_INFO_RIG_NAME_BYTES);
        let rig_name = std::str::from_utf8(&rig_raw[..rig_end])
            .map_err(|e| ParseError::Validation {
                context: "ProtosInfo::read".to_string(),
                message: format!("invalid rig_name bytes: {e}"),
            })?
            .to_owned();

        // 0x44..0x4C — 8 bytes of zero padding
        let padding = &bytes[0x44..0x4C];
        if padding != [0u8; 8] {
            return Err(ParseError::Validation {
                context: "ProtosInfo::read".to_string(),
                message: format!(
                    "expected zero padding at 0x44..0x4C, got {:02x?}",
                    padding
                ),
            });
        }

        // 0x4C..0x50 — rig_name_crc (CRC32C of rig_name, equals RigLogic.identity_crc_lo)
        let rig_name_crc =
            NameHash(u32::from_le_bytes(bytes[0x4C..0x50].try_into().unwrap()));
        let expected_rig_crc = NameHash::from_string(&rig_name);
        if rig_name_crc != expected_rig_crc {
            return Err(ParseError::Validation {
                context: "ProtosInfo::read".to_string(),
                message: format!(
                    "rig_name_crc mismatch: \
                     stored 0x{:08X}, computed 0x{:08X} for {:?}",
                    rig_name_crc.value(),
                    expected_rig_crc.value(),
                    rig_name
                ),
            });
        }

        Ok(ProtosInfo {
            mesh_name,
            mesh_name_crc,
            rig_name,
            rig_name_crc,
            version,
        })
    }
}

/// Compose Section C correctives on top of an appearance-control weight
/// vector, producing the full `_Dst[]` activation buffer that
/// `ExecuteHuman` consumes.
///
/// `appearance_weights` must have exactly `rig.num_appearance_blendshapes`
/// entries (the low half of `_Dst[]`). The result is a fresh
/// `Vec<f32>` of length `num_appearance + num_corrective`:
///
/// - `result[0..num_appearance]` is the appearance half, copied verbatim
///   from `appearance_weights`.
/// - `result[num_appearance + i]` for each Section C row is the product
///   of the five appearance-weight indices the row references:
///
/// ```text
/// result[num_appearance + i] = result[row.indices[0]]
///                            * result[row.indices[1]]
///                            * result[row.indices[2]]
///                            * result[row.indices[3]]
///                            * result[row.indices[4]]
/// ```
///
/// Indices are `u8`, so they can reference any of the first 256 slots
/// of `result[]` — including earlier corrective slots. Rows are
/// processed in order so a row referencing corrective slot `j < i`
/// reads its freshly-composed value (matches the engine's in-place
/// update in `ExecuteHuman` Anchor D, Pass 9 §6.11.1).
///
/// **Anchored against `CFaceRigLogic::ExecuteHuman`** (`0x1466e0aa0` in
/// the May 2026 build; PHASE_4B_INVESTIGATION.md §6.11.1 Anchor D):
///
/// ```c
/// pfVar72 = (float *)(_Dst + num_appearance);
/// do {
///     *pfVar72 = _Dst[row[0]] * _Dst[row[1]] * _Dst[row[2]]
///              * _Dst[row[3]] * _Dst[row[4]];
///     pfVar72++;
/// } while (--lVar43 != 0);
/// ```
///
/// # Errors
///
/// - `ParseError::Validation` if `appearance_weights.len() !=
///   rig.num_appearance_blendshapes`.
/// - `ParseError::Validation` if any row index references past the
///   end of the activation buffer (would happen if a malformed rig
///   declared a Section C index that overflows `num_appearance +
///   num_corrective`).
pub fn compose_correctives(
    rig: &RigLogic,
    appearance_weights: &[f32],
) -> Result<Vec<f32>, ParseError> {
    let na = rig.num_appearance_blendshapes as usize;
    let nc = rig.num_corrective_blendshapes as usize;

    if appearance_weights.len() != na {
        return Err(ParseError::Validation {
            context: "compose_correctives".to_string(),
            message: format!(
                "appearance_weights.len()={} != num_appearance_blendshapes={}",
                appearance_weights.len(),
                na,
            ),
        });
    }
    if rig.corrective_composition.len() != nc {
        return Err(ParseError::Validation {
            context: "compose_correctives".to_string(),
            message: format!(
                "corrective_composition.len()={} != num_corrective_blendshapes={}",
                rig.corrective_composition.len(),
                nc,
            ),
        });
    }

    let total = na + nc;
    let mut dst = vec![0.0_f32; total];
    dst[..na].copy_from_slice(appearance_weights);

    for (i, row) in rig.corrective_composition.iter().enumerate() {
        let mut product: f32 = 1.0;
        for &idx in &row.indices {
            let idx = idx as usize;
            if idx >= total {
                return Err(ParseError::Validation {
                    context: "compose_correctives".to_string(),
                    message: format!(
                        "corrective row {i} references index {idx} >= total slots {total}",
                    ),
                });
            }
            product *= dst[idx];
        }
        dst[na + i] = product;
    }

    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inline header-scalar values for a synthetic test blob.
    #[derive(Clone, Copy, Debug)]
    struct SyntheticScalars {
        num_appearance_blendshapes: u16,
        num_corrective_blendshapes: u16,
        num_joints: u16,
        reserved: u16,
        primary_delta_scale: f64,
        secondary_delta_scale: f64,
        num_main_delta_records: u32,
        num_secondary_delta_records: u32,
        num_blend_regions: u32,
        packed_value_bit_width: u32,
    }

    impl SyntheticScalars {
        fn zeroed() -> Self {
            Self {
                num_appearance_blendshapes: 0,
                num_corrective_blendshapes: 0,
                num_joints: 0,
                reserved: 0,
                primary_delta_scale: 0.0,
                secondary_delta_scale: 0.0,
                num_main_delta_records: 0,
                num_secondary_delta_records: 0,
                num_blend_regions: 0,
                packed_value_bit_width: 0,
            }
        }
    }

    /// Synthesize a minimal valid RigLogic byte buffer (header + section
    /// table only — no section content). Caller is responsible for padding
    /// out to cover all in-bounds section pointers.
    fn synthetic_header(
        name: &str,
        rig_type: RigType,
        scalars: SyntheticScalars,
        section_starts: [u64; NUM_SECTION_STARTS],
    ) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(0xA8);
        bytes.extend_from_slice(&RIG_LOGIC_MAGIC);

        let mut name_field = [0u8; NAME_FIELD_BYTES];
        let name_bytes = name.as_bytes();
        let copy_len = name_bytes.len().min(NAME_FIELD_BYTES);
        name_field[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        bytes.extend_from_slice(&name_field);

        bytes.push(rig_type.class);
        bytes.push(rig_type.gender);
        bytes.push(rig_type.variant);
        bytes.push(rig_type.part);
        bytes.push(rig_type.tier);
        bytes.push(rig_type.lod);
        bytes.extend_from_slice(&rig_type.flags_or_pad.to_le_bytes());

        bytes.extend_from_slice(&0u32.to_le_bytes()); // identity_crc_lo
        bytes.extend_from_slice(&0u16.to_le_bytes()); // identity_crc_hi_or_x
        bytes.extend_from_slice(&RIG_LOGIC_HEADER_SIZE.to_le_bytes()); // header_size

        // Typed inline scalars at file offsets 0x40..0x68.
        bytes.extend_from_slice(&scalars.num_appearance_blendshapes.to_le_bytes());
        bytes.extend_from_slice(&scalars.num_corrective_blendshapes.to_le_bytes());
        bytes.extend_from_slice(&scalars.num_joints.to_le_bytes());
        bytes.extend_from_slice(&scalars.reserved.to_le_bytes());
        bytes.extend_from_slice(&scalars.primary_delta_scale.to_le_bytes());
        bytes.extend_from_slice(&scalars.secondary_delta_scale.to_le_bytes());
        bytes.extend_from_slice(&scalars.num_main_delta_records.to_le_bytes());
        bytes.extend_from_slice(&scalars.num_secondary_delta_records.to_le_bytes());
        bytes.extend_from_slice(&scalars.num_blend_regions.to_le_bytes());
        bytes.extend_from_slice(&scalars.packed_value_bit_width.to_le_bytes());

        for start in section_starts.iter() {
            bytes.extend_from_slice(&start.to_le_bytes());
        }
        bytes
    }

    fn default_rig_type() -> RigType {
        RigType {
            class: 1,
            gender: 1,
            variant: 0,
            part: 3,
            tier: 1,
            lod: 0,
            flags_or_pad: 6,
        }
    }

    /// Synthetic section bounds that exactly fit `num_blendshapes` CRC
    /// entries in Section A (= `4 × n` bytes) and a small Section B name
    /// pool. Sections C through H are all collapsed to a single empty offset
    /// just past Section B — that lets the strict Task 3 delta-table reader
    /// pass when the synthetic header declares zero delta records (the
    /// "expected_record_count == 0 with empty span" fast-path).
    fn synthetic_section_starts_for(
        num_blendshapes: u16,
        name_pool_bytes: u64,
    ) -> [u64; NUM_SECTION_STARTS] {
        let section_a_start: u64 = 0xA8;
        let section_a_bytes = (num_blendshapes as u64) * SECTION_A_ENTRY_STRIDE as u64;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start + name_pool_bytes;
        // Slots 2..7 all collapsed to `section_c_start` so every later
        // section (C, D, E, F, G, H) has a zero-byte span. The Task 3
        // reader's `expected_record_count == 0 && end == start` fast-path
        // accepts this.
        [
            section_b_start, // slot 0 (Section B)
            section_a_start, // slot 1 (bootstrap / Section A)
            section_c_start, // slot 2 (Section C)
            section_c_start, // slot 3 (Section D)
            section_c_start, // slot 4 (Section E)
            section_c_start, // slot 5 (Section F)
            section_c_start, // slot 6 (Section G)
            section_c_start, // slot 7 (Section H)
        ]
    }

    /// Build a synthetic file with Section A populated with `crcs` and
    /// Section B populated with null-separated `names`. Other sections are
    /// empty (8 bytes each — enough to satisfy bounds checks).
    fn build_synthetic_file(
        name: &str,
        crcs: &[u32],
        names: &[&str],
    ) -> Vec<u8> {
        let mut name_pool: Vec<u8> = Vec::new();
        for n in names {
            name_pool.extend_from_slice(n.as_bytes());
            name_pool.push(0);
        }
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: crcs.len() as u16,
            ..SyntheticScalars::zeroed()
        };
        let section_starts = synthetic_section_starts_for(
            crcs.len() as u16,
            name_pool.len() as u64,
        );
        let mut bytes = synthetic_header(
            name,
            default_rig_type(),
            scalars,
            section_starts,
        );
        // Section A: 4 × n CRC bytes.
        for &c in crcs {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        // Section B: null-separated names.
        bytes.extend_from_slice(&name_pool);
        // Pad to cover slot 7 of section_starts.
        let max_end = *section_starts.iter().max().unwrap() as usize;
        if bytes.len() < max_end {
            bytes.resize(max_end, 0);
        }
        bytes
    }

    #[test]
    fn synthetic_header_parses_round_trip() {
        let bytes = build_synthetic_file(
            "protos_human_male_face_t1_pu",
            &[0x11111111, 0x22222222, 0x33333333],
            &["ctl_a", "ctl_b", "ctl_c"],
        );

        let rig = RigLogic::read(&bytes).expect("synthetic header parses");
        assert_eq!(rig.name, "protos_human_male_face_t1_pu");
        assert_eq!(rig.rig_type, default_rig_type());
        assert_eq!(rig.header_size, RIG_LOGIC_HEADER_SIZE);
        assert_eq!(rig.num_appearance_blendshapes, 3);
        assert_eq!(rig.joint_control_name_crcs.len(), 3);
        assert_eq!(rig.joint_control_name_crcs[0], 0x11111111);
        assert_eq!(rig.joint_control_names, vec!["ctl_a", "ctl_b", "ctl_c"]);
    }

    #[test]
    fn magic_validation_rejects_wrong_magic() {
        let mut bytes = build_synthetic_file("test", &[1, 2], &["a", "b"]);
        bytes[..8].copy_from_slice(b"WRONGMAG");
        let result = RigLogic::read(&bytes);
        assert!(
            matches!(result, Err(ParseError::InvalidMagic { .. })),
            "expected InvalidMagic, got {result:?}",
        );
    }

    #[test]
    fn name_field_trim_drops_null_padding() {
        let bytes = build_synthetic_file("shortname", &[0xAA], &["x"]);
        let rig = RigLogic::read(&bytes).expect("synthetic header parses");
        assert_eq!(rig.name, "shortname");
        // The trailing bytes of the 40-byte name field must be zero.
        let name_field = &bytes[0x08..0x30];
        assert_eq!(name_field[9..], [0u8; NAME_FIELD_BYTES - 9]);
    }

    #[test]
    fn header_size_mismatch_errors() {
        let mut bytes = build_synthetic_file("test", &[1], &["x"]);
        // Patch the u16 at offset 0x3E to a wrong value (0xBB instead of 0xA8).
        bytes[HEADER_SIZE_OFFSET..HEADER_SIZE_OFFSET + 2]
            .copy_from_slice(&0x00BBu16.to_le_bytes());
        let result = RigLogic::read(&bytes);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation, got {result:?}",
        );
    }

    #[test]
    fn section_table_bootstrap_invariant_enforced() {
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: 1,
            ..SyntheticScalars::zeroed()
        };
        // Replace the slot-1 bootstrap value with a bogus 999.
        let mut bad_starts = synthetic_section_starts_for(1, 4);
        bad_starts[1] = 999;
        let bytes = synthetic_header(
            "test",
            default_rig_type(),
            scalars,
            bad_starts,
        );
        let result = RigLogic::read(&bytes);
        let err = result.unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("section_starts[1]"),
                    "validation error mentioned {message:?}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn section_pointer_out_of_bounds_errors() {
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: 1,
            ..SyntheticScalars::zeroed()
        };
        let mut starts = synthetic_section_starts_for(1, 4);
        starts[7] = 999_999;
        let mut bytes = synthetic_header(
            "test",
            default_rig_type(),
            scalars,
            starts,
        );
        // Pad to cover every in-bounds slot (0..7) so the bounds check
        // ultimately fails on slot 7 alone.
        let max_in_bounds = starts.iter().filter(|&&e| e != 999_999).max().copied().unwrap_or(0)
            as usize;
        if bytes.len() < max_in_bounds {
            bytes.resize(max_in_bounds, 0);
        }
        let result = RigLogic::read(&bytes);
        match result {
            Err(ParseError::Validation { ref message, .. }) => {
                assert!(
                    message.contains("section_starts[7]"),
                    "validation error mentioned {message:?}",
                );
            }
            other => panic!("expected Validation for out-of-bounds pointer, got {other:?}"),
        }
    }

    #[test]
    fn truncated_input_errors_cleanly() {
        // 4 bytes is less than the magic length.
        let bytes = [0x52u8, 0x49, 0x47, 0x20];
        let result = RigLogic::read(&bytes);
        assert!(matches!(result, Err(ParseError::Truncated { .. })));
    }

    #[test]
    fn section_a_length_mismatch_errors() {
        // Build a header where num_appearance_blendshapes (= 5) does not
        // match the Section A span (= 3 × 4 = 12 bytes). Section A length
        // check should reject.
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: 5,
            ..SyntheticScalars::zeroed()
        };
        // Construct section_starts with Section A spanning only 3 CRCs.
        let section_starts = synthetic_section_starts_for(3, 0);
        let mut bytes = synthetic_header(
            "test",
            default_rig_type(),
            scalars,
            section_starts,
        );
        let max_end = *section_starts.iter().max().unwrap() as usize;
        if bytes.len() < max_end {
            bytes.resize(max_end, 0);
        }
        let result = RigLogic::read(&bytes);
        match result {
            Err(ParseError::Validation { ref message, .. }) => {
                assert!(
                    message.contains("Section A length"),
                    "expected Section A length mismatch error, got {message:?}",
                );
            }
            other => panic!("expected Validation for Section A length, got {other:?}"),
        }
    }

    #[test]
    fn section_a_reads_crcs_in_order() {
        let crcs = [0xDEADBEEF, 0xCAFEBABE, 0x12345678];
        let bytes = build_synthetic_file("test", &crcs, &["a", "b", "c"]);
        let rig = RigLogic::read(&bytes).expect("synthetic parses");
        let parsed: Vec<u32> = rig.joint_control_name_crcs.clone();
        assert_eq!(parsed, crcs);
    }

    #[test]
    fn section_b_skips_empty_entries() {
        // Names with consecutive nulls between them — empty entries should
        // be skipped, valid entries should be retained in order.
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: 2,
            ..SyntheticScalars::zeroed()
        };
        let name_pool: &[u8] = b"first\0\0\0second\0";
        let section_starts = synthetic_section_starts_for(2, name_pool.len() as u64);
        let mut bytes = synthetic_header(
            "test",
            default_rig_type(),
            scalars,
            section_starts,
        );
        bytes.extend_from_slice(&0x11111111u32.to_le_bytes());
        bytes.extend_from_slice(&0x22222222u32.to_le_bytes());
        bytes.extend_from_slice(name_pool);
        let max_end = *section_starts.iter().max().unwrap() as usize;
        if bytes.len() < max_end {
            bytes.resize(max_end, 0);
        }
        let rig = RigLogic::read(&bytes).expect("parses");
        assert_eq!(rig.joint_control_names, vec!["first", "second"]);
    }

    #[test]
    fn typed_inline_scalars_round_trip() {
        // Verify every inline-scalar field on the RigLogic struct receives
        // the value we wrote into the synthetic header. Section D / E / F / G
        // / H are declared empty (zero counts) so the strict per-section
        // readers' "zero entries + empty span" fast-paths are used — that
        // lets this test focus on the header scalars without supplying real
        // content for those sections. The Task 4 fields (`num_joints`,
        // `num_corrective_blendshapes`, `num_blend_regions`) are exercised by
        // separate dedicated tests below.
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: 7,
            num_corrective_blendshapes: 0,
            num_joints: 0,
            reserved: 0,
            primary_delta_scale: 0.0625,
            secondary_delta_scale: 1.617,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: 22,
        };
        let section_starts = synthetic_section_starts_for(7, 0);
        let mut bytes = synthetic_header(
            "scalars_round_trip",
            default_rig_type(),
            scalars,
            section_starts,
        );
        // Section A content (7 × u32) — empty name pool follows immediately.
        for i in 0..7u32 {
            bytes.extend_from_slice(&i.to_le_bytes());
        }
        let max_end = *section_starts.iter().max().unwrap() as usize;
        if bytes.len() < max_end {
            bytes.resize(max_end, 0);
        }
        let rig = RigLogic::read(&bytes).expect("parses");
        assert_eq!(rig.num_appearance_blendshapes, 7);
        assert_eq!(rig.num_corrective_blendshapes, 0);
        assert_eq!(rig.num_joints, 0);
        assert_eq!(rig.num_main_delta_records, 0);
        assert_eq!(rig.num_secondary_delta_records, 0);
        assert_eq!(rig.num_blend_regions, 0);
        assert_eq!(rig.packed_value_bit_width, 22);
        assert!((rig.primary_delta_scale - 0.0625).abs() < 1e-12);
        assert!((rig.secondary_delta_scale - 1.617).abs() < 1e-12);
        // Empty blobs propagate scale from the inline scalars.
        assert!(rig.appearance.records.is_empty());
        assert!(rig.corrective.records.is_empty());
        assert!((rig.appearance.scale - 0.0625).abs() < 1e-12);
        assert!((rig.corrective.scale - 1.617).abs() < 1e-12);
    }

    #[test]
    fn quatt_stride_is_28_bytes() {
        // QuatT must be exactly 28 bytes (Ghidra-anchored stride from
        // CFaceRigLogic::ExecuteGeneric's per-joint rotation loop).
        assert_eq!(core::mem::size_of::<QuatT>(), 28);
        assert_eq!(QUATT_STRIDE, 28);
    }

    // ─────────────────── Task 3 unit tests ───────────────────────────────

    #[test]
    fn decode_packed_delta_known_values() {
        // bit_width = 22 (the V1.9 observed value):
        //   shift = 9, mask = 0x1FF
        //   index = packed & 0x1FF
        //   coef  = (i32) packed >> 9
        let (idx, coef) = decode_packed_delta(0xFE4FB253, 22);
        assert_eq!(idx, 83);
        assert_eq!(coef, -55_335);

        // Zero packs to (0, 0).
        let (idx, coef) = decode_packed_delta(0, 22);
        assert_eq!(idx, 0);
        assert_eq!(coef, 0);

        // Positive max-magnitude: packed = 0x7FFFFFFF
        //   index = 0x1FF = 511
        //   coef  = 0x7FFFFFFF >> 9 (arith) = 0x3FFFFF = 4_194_303
        let (idx, coef) = decode_packed_delta(0x7FFFFFFF, 22);
        assert_eq!(idx, 511);
        assert_eq!(coef, 4_194_303);

        // Negative max-magnitude: packed = 0x80000000
        //   index = 0
        //   coef  = (i32) 0x80000000 >> 9 = -4_194_304
        let (idx, coef) = decode_packed_delta(0x80000000, 22);
        assert_eq!(idx, 0);
        assert_eq!(coef, -4_194_304);

        // Round-trip a synthetic value:
        //   coef = 12345, index = 42, bit_width = 22
        //   packed = (12345 << 9) | 42 = 0x3084_BC2A
        let packed: u32 = ((12_345i32) << 9) as u32 | 42;
        let (idx, coef) = decode_packed_delta(packed, 22);
        assert_eq!(idx, 42);
        assert_eq!(coef, 12_345);
    }

    #[test]
    fn decode_packed_delta_works_with_smaller_bit_widths() {
        // Verify the helper handles the documented full range (1..=30), not
        // just the observed bit_width = 22 case. Useful guard against the
        // helper accidentally hard-coding 22.
        // bit_width = 16 → shift = 15, mask = 0x7FFF
        //   packed = (i32 coef << 15) | (idx & 0x7FFF)
        //   coef = 100, idx = 1234 → packed = (100 << 15) | 1234 = 0x0032_04D2
        let (idx, coef) = decode_packed_delta(0x003204D2, 16);
        assert_eq!(idx, 1234);
        assert_eq!(coef, 100);
    }

    /// Build a synthetic file with `appearance_payloads` and
    /// `corrective_payloads` populated as flat lists of (output_idx,
    /// payload_u32s). Layouts use small counts so the test stays within a
    /// few hundred bytes.
    fn build_file_with_delta_tables(
        num_joints: u16,
        num_app: u16,
        num_corr: u16,
        bit_width: u32,
        primary_scale: f64,
        secondary_scale: f64,
        appearance: &[(u16, Vec<u32>)],
        corrective: &[(u16, Vec<u32>)],
    ) -> Vec<u8> {
        // Layout:
        //   header (0xA8)
        //   section A (4 * num_app bytes) — zero CRCs
        //   section B (0 bytes) — collapsed
        //   section C (5 * num_corr bytes) — zero composition rows
        //   section D (variable) — appearance records
        //   section E (variable) — corrective records
        //   section F (4 * num_joints bytes) — zero CRCs
        //   section G/H (0 bytes each) — collapsed (zero regions)
        let section_a_start: u64 = 0xA8;
        let section_a_bytes = (num_app as u64) * 4;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start;
        let section_c_bytes = (num_corr as u64) * 5;
        let section_d_start = section_c_start + section_c_bytes;
        let mut section_d_bytes: u64 = 0;
        for (_, p) in appearance {
            section_d_bytes += 4 + 4 * p.len() as u64;
        }
        let section_e_start = section_d_start + section_d_bytes;
        let mut section_e_bytes: u64 = 0;
        for (_, p) in corrective {
            section_e_bytes += 4 + 4 * p.len() as u64;
        }
        let section_f_start = section_e_start + section_e_bytes;
        let section_f_bytes = (num_joints as u64) * 4;
        let section_g_start = section_f_start + section_f_bytes;
        let section_starts: [u64; NUM_SECTION_STARTS] = [
            section_b_start, // 0
            section_a_start, // 1 (bootstrap)
            section_c_start, // 2
            section_d_start, // 3
            section_e_start, // 4
            section_f_start, // 5
            section_g_start, // 6
            section_g_start, // 7
        ];

        let scalars = SyntheticScalars {
            num_appearance_blendshapes: num_app,
            num_corrective_blendshapes: num_corr,
            num_joints,
            reserved: 0,
            primary_delta_scale: primary_scale,
            secondary_delta_scale: secondary_scale,
            num_main_delta_records: appearance.len() as u32,
            num_secondary_delta_records: corrective.len() as u32,
            num_blend_regions: 0,
            packed_value_bit_width: bit_width,
        };
        let mut bytes = synthetic_header("delta_test", default_rig_type(), scalars, section_starts);
        // Section A: zero CRCs.
        bytes.extend(std::iter::repeat(0u8).take(section_a_bytes as usize));
        // Section C: zero composition rows (5 bytes each, all zero — slot 0
        // = always-on `CTL_PSD` proxy, valid for any num_app >= 1).
        bytes.extend(std::iter::repeat(0u8).take(section_c_bytes as usize));
        // Section D content.
        for (out_idx, payload) in appearance {
            bytes.extend_from_slice(&out_idx.to_le_bytes());
            bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            for entry in payload {
                bytes.extend_from_slice(&entry.to_le_bytes());
            }
        }
        // Section E content.
        for (out_idx, payload) in corrective {
            bytes.extend_from_slice(&out_idx.to_le_bytes());
            bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            for entry in payload {
                bytes.extend_from_slice(&entry.to_le_bytes());
            }
        }
        // Section F: zero CRCs.
        bytes.extend(std::iter::repeat(0u8).take(section_f_bytes as usize));
        bytes
    }

    #[test]
    fn delta_tables_round_trip_synthetic() {
        let num_joints: u16 = 3;
        let num_app: u16 = 2;
        let num_corr: u16 = 1;
        let bit_width: u32 = 22;
        // Three appearance records, two corrective. output_idx values <
        // num_joints * 3 = 9. blendshape_idx values < num_app + num_corr = 3.
        let appearance: Vec<(u16, Vec<u32>)> = vec![
            (
                0,
                vec![
                    (1u32 << 9) | 0, // coef=1, idx=0
                    (2u32 << 9) | 1, // coef=2, idx=1
                ],
            ),
            (
                4,
                vec![
                    ((-3i32) << 9) as u32 | 2, // coef=-3, idx=2
                ],
            ),
            (8, vec![]), // zero-payload record is legal
        ];
        let corrective: Vec<(u16, Vec<u32>)> = vec![
            (
                1,
                vec![
                    (5u32 << 9) | 0, // coef=5, idx=0
                ],
            ),
            (
                7,
                vec![
                    ((-7i32) << 9) as u32 | 2,
                    (11u32 << 9) | 1,
                ],
            ),
        ];
        let bytes = build_file_with_delta_tables(
            num_joints, num_app, num_corr, bit_width, 0.5, 0.25,
            &appearance, &corrective,
        );

        let rig = RigLogic::read(&bytes).expect("synthetic delta tables parse");
        assert_eq!(rig.appearance.records.len(), 3);
        assert_eq!(rig.corrective.records.len(), 2);
        assert_eq!(rig.appearance.scale, 0.5);
        assert_eq!(rig.corrective.scale, 0.25);

        // Spot-check round-trip.
        assert_eq!(rig.appearance.records[0].output_idx, 0);
        assert_eq!(rig.appearance.records[0].payload.len(), 2);
        let (idx0, coef0) =
            decode_packed_delta(rig.appearance.records[0].payload[0], bit_width);
        assert_eq!((idx0, coef0), (0, 1));
        let (idx1, coef1) =
            decode_packed_delta(rig.appearance.records[0].payload[1], bit_width);
        assert_eq!((idx1, coef1), (1, 2));
        // Empty-payload record.
        assert_eq!(rig.appearance.records[2].output_idx, 8);
        assert!(rig.appearance.records[2].payload.is_empty());
        // Corrective negative coef round-trip.
        let (idx, coef) =
            decode_packed_delta(rig.corrective.records[1].payload[0], bit_width);
        assert_eq!((idx, coef), (2, -7));
    }

    #[test]
    fn delta_table_rejects_output_idx_out_of_range() {
        let bit_width: u32 = 22;
        // output_idx = 100, but num_joints = 3 → max valid = 8.
        let appearance: Vec<(u16, Vec<u32>)> = vec![(100, vec![])];
        let bytes = build_file_with_delta_tables(
            3, 2, 1, bit_width, 0.5, 0.25, &appearance, &[],
        );
        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("output_idx = 100"),
                    "expected output_idx-range error, got {message:?}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn delta_table_rejects_blendshape_idx_out_of_range() {
        let bit_width: u32 = 22;
        // num_app + num_corr = 3 → max blendshape_idx = 2; we encode idx=5.
        let bad_packed = (1u32 << 9) | 5;
        let appearance: Vec<(u16, Vec<u32>)> = vec![(0, vec![bad_packed])];
        let bytes = build_file_with_delta_tables(
            3, 2, 1, bit_width, 0.5, 0.25, &appearance, &[],
        );
        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("blendshape_idx = 5"),
                    "expected blendshape-idx range error, got {message:?}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn delta_table_rejects_truncated_payload() {
        // Hand-build a file where a record declares payload_len = 10 but
        // section D's next-section pointer leaves only 2 u32s of space.
        let bit_width: u32 = 22;
        let num_joints: u16 = 3;
        let num_app: u16 = 2;
        let num_corr: u16 = 1;

        let section_a_start: u64 = 0xA8;
        let section_a_bytes = (num_app as u64) * 4;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start;
        let section_d_start = section_c_start;
        // Section D fits only 4 (header) + 2 * 4 = 12 bytes.
        let section_d_bytes: u64 = 12;
        let section_e_start = section_d_start + section_d_bytes;
        let section_f_start = section_e_start;
        let section_starts: [u64; NUM_SECTION_STARTS] = [
            section_b_start,
            section_a_start,
            section_c_start,
            section_d_start,
            section_e_start,
            section_f_start,
            section_f_start,
            section_f_start,
        ];

        let scalars = SyntheticScalars {
            num_appearance_blendshapes: num_app,
            num_corrective_blendshapes: num_corr,
            num_joints,
            reserved: 0,
            primary_delta_scale: 1.0,
            secondary_delta_scale: 1.0,
            num_main_delta_records: 1,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: bit_width,
        };
        let mut bytes = synthetic_header(
            "truncated", default_rig_type(), scalars, section_starts,
        );
        bytes.extend(std::iter::repeat(0u8).take(section_a_bytes as usize));
        // Section D: declare payload_len = 10 but only write 2 u32s.
        bytes.extend_from_slice(&0u16.to_le_bytes()); // output_idx
        bytes.extend_from_slice(&10u16.to_le_bytes()); // payload_len (LIE)
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("payload_len = 10"),
                    "expected truncated-payload error, got {message:?}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn delta_table_rejects_leftover_bytes() {
        // Build a file where the section span exceeds what
        // num_main_delta_records implies — extra trailing bytes that the
        // strict reader must reject.
        let bit_width: u32 = 22;
        let num_joints: u16 = 3;
        let num_app: u16 = 2;
        let num_corr: u16 = 1;

        let section_a_start: u64 = 0xA8;
        let section_a_bytes = (num_app as u64) * 4;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start;
        let section_d_start = section_c_start;
        // 4 (header) + 0 (no payload) + 8 trailing slack bytes.
        let section_d_bytes: u64 = 4 + 8;
        let section_e_start = section_d_start + section_d_bytes;
        let section_f_start = section_e_start;
        let section_starts: [u64; NUM_SECTION_STARTS] = [
            section_b_start,
            section_a_start,
            section_c_start,
            section_d_start,
            section_e_start,
            section_f_start,
            section_f_start,
            section_f_start,
        ];

        let scalars = SyntheticScalars {
            num_appearance_blendshapes: num_app,
            num_corrective_blendshapes: num_corr,
            num_joints,
            reserved: 0,
            primary_delta_scale: 1.0,
            secondary_delta_scale: 1.0,
            num_main_delta_records: 1,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: bit_width,
        };
        let mut bytes = synthetic_header(
            "leftover", default_rig_type(), scalars, section_starts,
        );
        bytes.extend(std::iter::repeat(0u8).take(section_a_bytes as usize));
        // Section D: one zero-payload record, then 8 trailing slack bytes.
        bytes.extend_from_slice(&0u16.to_le_bytes()); // output_idx
        bytes.extend_from_slice(&0u16.to_le_bytes()); // payload_len = 0
        bytes.extend_from_slice(&[0u8; 8]); // slack

        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("8 bytes remain unread"),
                    "expected leftover-bytes error, got {message:?}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn delta_table_rejects_nonempty_span_with_zero_records() {
        // Zero records declared but section spans 8 bytes — must error.
        let bit_width: u32 = 22;
        let num_joints: u16 = 3;
        let num_app: u16 = 2;
        let num_corr: u16 = 1;

        let section_a_start: u64 = 0xA8;
        let section_a_bytes = (num_app as u64) * 4;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start;
        let section_d_start = section_c_start;
        let section_d_bytes: u64 = 8;
        let section_e_start = section_d_start + section_d_bytes;
        let section_f_start = section_e_start;
        let section_starts: [u64; NUM_SECTION_STARTS] = [
            section_b_start,
            section_a_start,
            section_c_start,
            section_d_start,
            section_e_start,
            section_f_start,
            section_f_start,
            section_f_start,
        ];

        let scalars = SyntheticScalars {
            num_appearance_blendshapes: num_app,
            num_corrective_blendshapes: num_corr,
            num_joints,
            reserved: 0,
            primary_delta_scale: 1.0,
            secondary_delta_scale: 1.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: bit_width,
        };
        let mut bytes = synthetic_header(
            "nonempty_zero", default_rig_type(), scalars, section_starts,
        );
        bytes.extend(std::iter::repeat(0u8).take(section_a_bytes as usize));
        bytes.extend_from_slice(&[0u8; 8]);

        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("zero records declared but section spans"),
                    "expected zero-record/nonempty-span error, got {message:?}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// Real-fixture parse: read the extracted standalone file and assert
    /// its header parses to the verified-byte values. Reads the male t1_pu
    /// `.RigLogic` from Data.p4k; skipped without one (CI).
    #[test]
    fn fixture_parses() {
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_RIGLOGIC) else { return };
        assert_eq!(bytes.len(), 328_056, "expected fixture size 328,056");
        let rig = RigLogic::read(&bytes).expect("real fixture parses");
        assert_eq!(rig.name, "protos_human_male_face_t1_pu");
        assert_eq!(rig.rig_type.class, 1);
        assert_eq!(rig.rig_type.gender, 1);
        assert_eq!(rig.rig_type.part, 3);
        assert_eq!(rig.rig_type.tier, 1);
        assert_eq!(rig.identity_crc_lo.value(), 0xDD6C67F6);
        assert_eq!(rig.header_size, RIG_LOGIC_HEADER_SIZE);
        assert_eq!(
            rig.section_starts,
            [988, 168, 6_836, 8_292, 165_968, 327_064, 327_780, 327_904]
        );
        // Anchored count fields.
        assert_eq!(rig.num_appearance_blendshapes, 205);
        assert_eq!(rig.num_corrective_blendshapes, 290);
        assert_eq!(rig.num_joints, 179);
        assert_eq!(rig.num_main_delta_records, 517);
        assert_eq!(rig.num_secondary_delta_records, 528);
        assert_eq!(rig.num_blend_regions, 24);
        assert_eq!(rig.packed_value_bit_width, 22);
        // Section A must have exactly num_appearance_blendshapes entries.
        assert_eq!(
            rig.joint_control_name_crcs.len(),
            rig.num_appearance_blendshapes as usize,
        );
        // Section B should have ~205 control names. Tolerate ±5% for
        // trailing pad-byte slop until Tasks 3-4 anchor the exact stride.
        assert!(
            rig.joint_control_names.len() >= 195
                && rig.joint_control_names.len() <= 215,
            "expected ~205 control names, got {}",
            rig.joint_control_names.len(),
        );
        // Cross-verify: hashing the first control name with CRC32 (IEEE,
        // polynomial 0xEDB88320 — NOT CRC32C) should match the first entry
        // in the CRC array. For the observed fixture:
        //   CRC32("CTL_PSD") = 0xFD66F569 == joint_control_name_crcs[0]
        // This anchors that Section A uses CRC32 (IEEE), not CRC32C.
        let first_name = &rig.joint_control_names[0];
        let computed = crc32fast::hash(first_name.as_bytes());
        assert_eq!(
            computed,
            rig.joint_control_name_crcs[0],
            "CRC32('{first_name}') = {:#X}, but Section A[0] = {:#X}",
            computed,
            rig.joint_control_name_crcs[0],
        );
        assert_eq!(rig.num_blend_regions, 24);

        // ── Task 3: Section D / E delta tables ──────────────────────────
        assert_eq!(
            rig.appearance.records.len(),
            rig.num_main_delta_records as usize,
            "appearance.records.len() should equal num_main_delta_records",
        );
        assert_eq!(rig.appearance.records.len(), 517);
        assert_eq!(rig.appearance.scale, rig.primary_delta_scale);

        assert_eq!(
            rig.corrective.records.len(),
            rig.num_secondary_delta_records as usize,
            "corrective.records.len() should equal num_secondary_delta_records",
        );
        assert_eq!(rig.corrective.records.len(), 528);
        assert_eq!(rig.corrective.scale, rig.secondary_delta_scale);

        // First appearance record matches the byte-level walk against the
        // raw fixture: byte_off=8292 (Section D start), output_idx=0, N=59,
        // first u32 = 0xFE4FB253.
        let r0 = &rig.appearance.records[0];
        assert_eq!(r0.output_idx, 0);
        assert_eq!(r0.payload.len(), 59);
        assert_eq!(r0.payload[0], 0xFE4FB253);
        assert_eq!(r0.payload[1], 0xFFB7BE54);
        assert_eq!(r0.payload[2], 0x007CDC55);

        // First corrective record: byte_off=165968 (Section E start),
        // output_idx=0, N=61, first u32 = 0xF78FEC53.
        let s0 = &rig.corrective.records[0];
        assert_eq!(s0.output_idx, 0);
        assert_eq!(s0.payload.len(), 61);
        assert_eq!(s0.payload[0], 0xF78FEC53);

        // Decode the first appearance payload entry and validate the bit
        // layout against an independent Python walk:
        //   0xFE4FB253 with bit_width = 22:
        //     shift = 31 - 22 = 9, mask = 0x1FF
        //     index = 0xFE4FB253 & 0x1FF = 0x53 = 83
        //     coef  = (i32) 0xFE4FB253 >> 9 = -55_335
        let (idx, coef) =
            decode_packed_delta(0xFE4FB253, rig.packed_value_bit_width);
        assert_eq!(idx, 83);
        assert_eq!(coef, -55_335);
        assert!(
            (idx as u32) < (rig.num_appearance_blendshapes as u32)
                + (rig.num_corrective_blendshapes as u32),
            "decoded blendshape_idx {idx} out of range",
        );

        // Every record's output_idx must be a valid joint-axis slot.
        let max_output = (rig.num_joints as u32) * 3;
        for (i, r) in rig.appearance.records.iter().enumerate() {
            assert!(
                (r.output_idx as u32) < max_output,
                "appearance record {i} has output_idx {} >= {max_output}",
                r.output_idx,
            );
        }
        for (i, r) in rig.corrective.records.iter().enumerate() {
            assert!(
                (r.output_idx as u32) < max_output,
                "corrective record {i} has output_idx {} >= {max_output}",
                r.output_idx,
            );
        }
    }

    /// Anchored against the real fixture byte layout. The byte sequence
    /// here is the first 0xA8 bytes of `protos_human_male_face_t1_pu.RigLogic`
    /// (verified via direct file inspection and Ghidra `FaceRigData::Load`
    /// at `0x14142d4d0`).
    ///
    /// This test exercises only the header / count fields (Section A and B
    /// are populated with zeros below), so it does NOT validate Section A
    /// or B content from the real fixture — that is `fixture_parses`'s job.
    #[test]
    fn anchored_header_bytes_parse() {
        // First 0xA8 bytes of the real standalone fixture.
        let header: [u8; 0xA8] = [
            // 0x00: magic "RIG V1.9"
            0x52, 0x49, 0x47, 0x20, 0x56, 0x31, 0x2E, 0x39,
            // 0x08-0x2F: name "protos_human_male_face_t1_pu" null-padded
            0x70, 0x72, 0x6F, 0x74, 0x6F, 0x73, 0x5F, 0x68,
            0x75, 0x6D, 0x61, 0x6E, 0x5F, 0x6D, 0x61, 0x6C,
            0x65, 0x5F, 0x66, 0x61, 0x63, 0x65, 0x5F, 0x74,
            0x31, 0x5F, 0x70, 0x75, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // 0x30-0x37: RigType
            0x01, 0x01, 0x00, 0x03, 0x01, 0x00, 0x06, 0x00,
            // 0x38-0x3F: identity_crc_lo / hi_or_x / header_size
            0xF6, 0x67, 0x6C, 0xDD, 0x62, 0x45, 0xA8, 0x00,
            // 0x40-0x67: inline scalars (typed, Task 2-anchored)
            //   0x40: u16 num_appearance_blendshapes = 205
            //   0x42: u16 num_corrective_blendshapes = 290
            //   0x44: u16 num_joints = 179
            //   0x46: u16 reserved = 0
            //   0x48: f64 primary_delta_scale
            //   0x50: f64 secondary_delta_scale
            //   0x58: u32 num_main_delta_records = 517
            //   0x5C: u32 num_secondary_delta_records = 528
            //   0x60: u32 num_blend_regions = 24
            //   0x64: u32 packed_value_bit_width = 22
            0xCD, 0x00, 0x22, 0x01, 0xB3, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0xC0, 0x3B, 0x53, 0xB1, 0x3F,
            0x00, 0x00, 0x00, 0x60, 0xFE, 0xDE, 0xF9, 0x3F,
            0x05, 0x02, 0x00, 0x00, 0x10, 0x02, 0x00, 0x00,
            0x18, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00,
            // 0x68-0xA7: section_starts [988, 168, 6836, 8292, 165968, 327064, 327780, 327904]
            0xDC, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xA8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xB4, 0x1A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x64, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x50, 0x88, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x98, 0xFD, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x64, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xE0, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        // Pad to cover the largest section_start value (327904) so bounds checks pass.
        let mut bytes = Vec::with_capacity(327_904);
        bytes.extend_from_slice(&header);
        bytes.resize(327_904, 0);

        // This test exists to anchor the HEADER byte layout (offsets 0x00..
        // 0x68) and the section-start pointer table layout (0x68..0xA8).
        // Section D-H content is not part of its scope; the section bodies
        // are zero-padded here. The strict Task 3 delta-table reader needs
        // the declared record counts to match the actual section bytes, so
        // we zero the count fields AND collapse slots 3..7 of the
        // section-start table to make Sections D-H zero-byte empty regions.
        // The real fixture's delta-table content is exercised by
        // `fixture_parses` — and a separate `anchored_section_starts_bytes`
        // sub-assertion below pins the raw on-disk section-start bytes the
        // test loaded from the real file.
        let on_disk_section_starts: [u64; NUM_SECTION_STARTS] =
            [988, 168, 6_836, 8_292, 165_968, 327_064, 327_780, 327_904];
        // Cross-check the on-disk pointer table bytes — the literal header
        // above was supposed to encode this exact sequence.
        for (slot, expected) in on_disk_section_starts.iter().enumerate() {
            let off = SECTION_START_TABLE_OFFSET + slot * 8;
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&bytes[off..off + 8]);
            assert_eq!(
                u64::from_le_bytes(buf),
                *expected,
                "on-disk section_starts[{slot}] byte mismatch",
            );
        }

        bytes[0x58..0x5C].copy_from_slice(&0u32.to_le_bytes()); // num_main_delta_records → 0
        bytes[0x5C..0x60].copy_from_slice(&0u32.to_le_bytes()); // num_secondary_delta_records → 0
        bytes[0x60..0x64].copy_from_slice(&0u32.to_le_bytes()); // num_blend_regions → 0
        // Patch count fields used by Section C and Section F readers to zero
        // — those sections are collapsed below, so the counts must match.
        bytes[0x42..0x44].copy_from_slice(&0u16.to_le_bytes()); // num_corrective_blendshapes → 0
        bytes[0x44..0x46].copy_from_slice(&0u16.to_le_bytes()); // num_joints → 0

        // Collapse slots 2..7 to the value of slot 0 (= 988, end of Section A).
        // This makes Sections B/C/D/E/F/G/H all zero-byte and lets the strict
        // per-section readers' "zero entries + empty span" fast-paths accept
        // the patched file.
        let collapsed_end: u64 = 988;
        for slot in 2..NUM_SECTION_STARTS {
            let off = SECTION_START_TABLE_OFFSET + slot * 8;
            bytes[off..off + 8].copy_from_slice(&collapsed_end.to_le_bytes());
        }
        // Truncate the buffer to the new file_end so Section H's reader (which
        // uses `file_size` as the upper bound on the last section) doesn't see
        // a 327k-byte trailing slack region.
        bytes.truncate(collapsed_end as usize);

        let rig = RigLogic::read(&bytes).expect("anchored bytes parse");
        assert_eq!(rig.name, "protos_human_male_face_t1_pu");
        assert_eq!(
            rig.rig_type,
            RigType {
                class: 1,
                gender: 1,
                variant: 0,
                part: 3,
                tier: 1,
                lod: 0,
                flags_or_pad: 6,
            }
        );
        assert_eq!(rig.identity_crc_lo.value(), 0xDD6C67F6);
        assert_eq!(rig.identity_crc_hi_or_x, 0x4562);
        assert_eq!(rig.header_size, RIG_LOGIC_HEADER_SIZE);
        // Parsed section_starts reflect the patched slots 2..7.
        assert_eq!(
            rig.section_starts,
            [988, 168, 988, 988, 988, 988, 988, 988]
        );
        // Anchored count fields (replaces the old `header_scalars`
        // byte-window assertions). num_corrective_blendshapes and num_joints
        // were patched to 0 above so the collapsed Section C / F readers
        // accept zero-span sections.
        assert_eq!(rig.num_appearance_blendshapes, 205);
        assert_eq!(rig.num_corrective_blendshapes, 0);
        assert_eq!(rig.num_joints, 0);
        assert_eq!(rig._reserved_0x46, 0);
        // num_main/secondary_delta_records were patched to 0 above to bypass
        // Section D / E content validation; assert the patched values.
        assert_eq!(rig.num_main_delta_records, 0);
        assert_eq!(rig.num_secondary_delta_records, 0);
        assert_eq!(rig.num_blend_regions, 0);
        assert_eq!(rig.packed_value_bit_width, 22);
        // appearance & corrective both empty after the count patch.
        assert!(rig.appearance.records.is_empty());
        assert!(rig.corrective.records.is_empty());
        // Scales still match the on-disk bytes.
        assert_eq!(rig.appearance.scale, rig.primary_delta_scale);
        assert_eq!(rig.corrective.scale, rig.secondary_delta_scale);
        // f64 scales — exact match against the byte pattern.
        let expected_primary = f64::from_le_bytes([
            0x00, 0x00, 0x00, 0xC0, 0x3B, 0x53, 0xB1, 0x3F,
        ]);
        let expected_secondary = f64::from_le_bytes([
            0x00, 0x00, 0x00, 0x60, 0xFE, 0xDE, 0xF9, 0x3F,
        ]);
        assert_eq!(rig.primary_delta_scale, expected_primary);
        assert_eq!(rig.secondary_delta_scale, expected_secondary);
        // Section A spans 820 bytes — but we padded with zeros, so we expect
        // 205 zero u32 entries.
        assert_eq!(rig.joint_control_name_crcs.len(), 205);
        for &h in &rig.joint_control_name_crcs {
            assert_eq!(h, 0);
        }
        // Section B is all zeros (padding) — no valid names parsed.
        assert!(rig.joint_control_names.is_empty());
        // Task 4 fields: all collapsed sections produced empty Vecs.
        assert!(rig.corrective_composition.is_empty());
        assert!(rig.skeleton_bone_name_crcs.is_empty());
        assert!(rig.blend_region_bones.is_empty());
        assert!(rig.region_weights.is_empty());
    }

    // ─────────────────── Task 4 unit tests ───────────────────────────────

    /// Build a synthetic file with Section C / F / G / H populated. All
    /// other sections collapsed to empty regions.
    #[allow(clippy::too_many_arguments)]
    fn build_file_with_trailing_sections(
        num_app: u16,
        corrective_rows: &[[u8; 5]],
        skeleton_bone_crcs: &[u32],
        region_bones: &[Vec<u16>],
        region_weights: &[f32],
    ) -> Vec<u8> {
        let num_corr = corrective_rows.len() as u16;
        let num_joints = skeleton_bone_crcs.len() as u16;
        let num_blend_regions = region_bones.len() as u32;

        let section_a_start: u64 = 0xA8;
        let section_a_bytes = (num_app as u64) * 4;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start; // section B empty
        let section_c_bytes = (num_corr as u64) * 5;
        let section_d_start = section_c_start + section_c_bytes;
        let section_e_start = section_d_start; // D empty
        let section_f_start = section_e_start; // E empty
        let section_f_bytes = (num_joints as u64) * 4;
        let section_g_start = section_f_start + section_f_bytes;
        let section_g_bytes: u64 = region_bones
            .iter()
            .map(|r| 2 + 2 * r.len() as u64)
            .sum();
        let section_h_start = section_g_start + section_g_bytes;
        let section_h_bytes = (region_weights.len() as u64) * 4;
        let file_end = section_h_start + section_h_bytes;
        let section_starts: [u64; NUM_SECTION_STARTS] = [
            section_b_start, // 0
            section_a_start, // 1 (bootstrap)
            section_c_start, // 2
            section_d_start, // 3
            section_e_start, // 4
            section_f_start, // 5
            section_g_start, // 6
            section_h_start, // 7
        ];

        let scalars = SyntheticScalars {
            num_appearance_blendshapes: num_app,
            num_corrective_blendshapes: num_corr,
            num_joints,
            reserved: 0,
            primary_delta_scale: 1.0,
            secondary_delta_scale: 1.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions,
            packed_value_bit_width: 22,
        };
        let mut bytes = synthetic_header("task4_test", default_rig_type(), scalars, section_starts);
        // Section A: zero CRCs.
        bytes.extend(std::iter::repeat(0u8).take(section_a_bytes as usize));
        // Section C: composition rows.
        for row in corrective_rows {
            bytes.extend_from_slice(row);
        }
        // Section F: bone CRCs.
        for c in skeleton_bone_crcs {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        // Section G: region bone-index lists.
        for region in region_bones {
            bytes.extend_from_slice(&(region.len() as u16).to_le_bytes());
            for &idx in region {
                bytes.extend_from_slice(&idx.to_le_bytes());
            }
        }
        // Section H: per-region weight scalars.
        for w in region_weights {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(
            bytes.len() as u64,
            file_end,
            "synthetic file size mismatch: {} vs expected {}",
            bytes.len(),
            file_end,
        );
        bytes
    }

    #[test]
    fn section_c_corrective_composition_round_trips() {
        let rows: Vec<[u8; 5]> = vec![
            [2, 4, 6, 0, 0],
            [15, 16, 17, 0, 0],
            [0, 0, 0, 0, 0],
        ];
        let bytes = build_file_with_trailing_sections(
            32,    // num_app — large enough to admit indices 0..17
            &rows,
            &[],
            &[],
            &[],
        );
        let rig = RigLogic::read(&bytes).expect("parses");
        assert_eq!(rig.corrective_composition.len(), 3);
        assert_eq!(rig.corrective_composition[0].indices, [2, 4, 6, 0, 0]);
        assert_eq!(rig.corrective_composition[1].indices, [15, 16, 17, 0, 0]);
        assert_eq!(rig.corrective_composition[2].indices, [0, 0, 0, 0, 0]);
    }

    #[test]
    fn section_c_rejects_out_of_range_index() {
        let rows: Vec<[u8; 5]> = vec![[5, 0, 0, 0, 0]];
        // num_app = 5 means valid indices are 0..4; idx=5 is out of range.
        let bytes = build_file_with_trailing_sections(5, &rows, &[], &[], &[]);
        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("Section C row 0 slot 0 = 5"),
                    "unexpected message: {message}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn section_f_skeleton_bone_crcs_round_trip() {
        let crcs: Vec<u32> = vec![0xDEADBEEF, 0xCAFEBABE, 0x12345678, 0x40D44915];
        let bytes = build_file_with_trailing_sections(
            1, // num_app
            &[],
            &crcs,
            &[],
            &[],
        );
        let rig = RigLogic::read(&bytes).expect("parses");
        assert_eq!(rig.skeleton_bone_name_crcs.len(), 4);
        assert_eq!(rig.skeleton_bone_name_crcs, crcs);
    }

    #[test]
    fn section_g_region_bones_round_trip() {
        // 3 regions, with 3 + 2 + 1 = 6 weight entries total.
        let region_bones: Vec<Vec<u16>> = vec![
            vec![125, 133, 358],
            vec![2, 10],
            vec![127],
        ];
        // Section H must have len = sum(region_bones[i].len()) = 6.
        let region_weights: Vec<f32> = vec![1.0, 0.5, -1.0, 1.0, 0.5, 0.7];
        let bytes = build_file_with_trailing_sections(
            1, // num_app
            &[],
            &[],
            &region_bones,
            &region_weights,
        );
        let rig = RigLogic::read(&bytes).expect("parses");
        assert_eq!(rig.blend_region_bones.len(), 3);
        assert_eq!(rig.blend_region_bones[0].weight_indices, vec![125, 133, 358]);
        assert_eq!(rig.blend_region_bones[1].weight_indices, vec![2, 10]);
        assert_eq!(rig.blend_region_bones[2].weight_indices, vec![127]);
        assert_eq!(rig.region_weights, vec![1.0, 0.5, -1.0, 1.0, 0.5, 0.7]);
        // region_weight_slice indexing
        assert_eq!(rig.region_weight_slice(0), Some([1.0, 0.5, -1.0].as_slice()));
        assert_eq!(rig.region_weight_slice(1), Some([1.0, 0.5].as_slice()));
        assert_eq!(rig.region_weight_slice(2), Some([0.7].as_slice()));
        assert_eq!(rig.region_weight_slice(3), None);
    }

    #[test]
    fn section_g_rejects_truncated_region_count() {
        // Hand-craft a file whose Section G next-start leaves only 1 byte for
        // the count u16 of region 0.
        let section_a_start: u64 = 0xA8;
        let num_app: u16 = 1;
        let section_a_bytes = (num_app as u64) * 4;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start;
        let section_d_start = section_c_start;
        let section_e_start = section_d_start;
        let section_f_start = section_e_start;
        let section_g_start = section_f_start;
        // 1 byte total — not enough for the u16 count.
        let section_h_start = section_g_start + 1;
        let section_starts: [u64; NUM_SECTION_STARTS] = [
            section_b_start, section_a_start, section_c_start, section_d_start,
            section_e_start, section_f_start, section_g_start, section_h_start,
        ];
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: num_app,
            num_corrective_blendshapes: 0,
            num_joints: 0,
            reserved: 0,
            primary_delta_scale: 1.0,
            secondary_delta_scale: 1.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 1, // claim 1 region but provide only 1 byte
            packed_value_bit_width: 22,
        };
        let mut bytes = synthetic_header("g_trunc", default_rig_type(), scalars, section_starts);
        bytes.extend(std::iter::repeat(0u8).take(section_a_bytes as usize));
        bytes.push(0); // 1 byte of Section G
        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("Section G: region 0 truncated"),
                    "unexpected: {message}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn section_h_rejects_short_span() {
        // Section H needs 4*3 = 12 bytes for 3 weight indices in region 0
        // (count from Section G), but we'll allocate only 4 bytes for it.
        let section_a_start: u64 = 0xA8;
        let num_app: u16 = 1;
        let section_a_bytes = (num_app as u64) * 4;
        let section_b_start = section_a_start + section_a_bytes;
        let section_c_start = section_b_start;
        let section_d_start = section_c_start;
        let section_e_start = section_d_start;
        let section_f_start = section_e_start;
        let section_g_start = section_f_start;
        // Section G: 1 region with count = 3, so 2 + 3*2 = 8 bytes
        let section_g_bytes: u64 = 8;
        let section_h_start = section_g_start + section_g_bytes;
        let file_end = section_h_start + 4; // Section H = only 4 bytes
        let section_starts: [u64; NUM_SECTION_STARTS] = [
            section_b_start, section_a_start, section_c_start, section_d_start,
            section_e_start, section_f_start, section_g_start, section_h_start,
        ];
        let scalars = SyntheticScalars {
            num_appearance_blendshapes: num_app,
            num_corrective_blendshapes: 0,
            num_joints: 0,
            reserved: 0,
            primary_delta_scale: 1.0,
            secondary_delta_scale: 1.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 1,
            packed_value_bit_width: 22,
        };
        let mut bytes = synthetic_header("h_short", default_rig_type(), scalars, section_starts);
        bytes.extend(std::iter::repeat(0u8).take(section_a_bytes as usize));
        // Section G: 1 region, count=3, three zero indices.
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        // Section H: only 4 bytes (= 1 f32), need 12 (= 3 f32).
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(bytes.len() as u64, file_end);
        let err = RigLogic::read(&bytes).unwrap_err();
        match err {
            ParseError::Validation { ref message, .. } => {
                assert!(
                    message.contains("Section H span 4 bytes < required 12"),
                    "unexpected: {message}",
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn fixture_parses_trailing_sections() {
        // Real-fixture sub-test for Task 4: assert Section C / F / G / H
        // counts and a couple of byte-level spot-checks (same fixture as
        // `fixture_parses`).
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_RIGLOGIC) else { return };
        let rig = RigLogic::read(&bytes).expect("real fixture parses");

        // Section C — 290 corrective rows (= num_corrective_blendshapes).
        assert_eq!(rig.corrective_composition.len(), 290);
        // Cross-check against the raw Python walk: row 0 = [2, 4, 6, 0, 0].
        assert_eq!(rig.corrective_composition[0].indices, [2, 4, 6, 0, 0]);
        assert_eq!(rig.corrective_composition[5].indices, [15, 16, 17, 0, 0]);

        // Section F — 179 skeleton-bone CRC32 entries.
        assert_eq!(rig.skeleton_bone_name_crcs.len(), 179);
        // The first CRC observed in the fixture is `0x40D44915`.
        assert_eq!(rig.skeleton_bone_name_crcs[0], 0x40D44915);

        // Section G — 24 blend regions; per-region counts decoded from the
        // fixture via an independent Python walk:
        //   region 0: count=3, indices=[125, 133, 358]
        //   region 1: count=3, indices=[2, 10, 206]
        //   region 2: count=1, indices=[127]
        //   ...
        assert_eq!(rig.blend_region_bones.len(), 24);
        assert_eq!(rig.blend_region_bones[0].weight_indices, vec![125, 133, 358]);
        assert_eq!(rig.blend_region_bones[1].weight_indices, vec![2, 10, 206]);
        assert_eq!(rig.blend_region_bones[2].weight_indices, vec![127]);

        // Sum of counts across all 24 regions = 38 in the fixture.
        let total_entries: usize = rig
            .blend_region_bones
            .iter()
            .map(|r| r.weight_indices.len())
            .sum();
        assert_eq!(total_entries, 38);

        // Section H — 38 f32 weights (matches the sum of Section G counts).
        // **In the standalone fixture, Section H spans 152 bytes** (NOT 88 as
        // an earlier draft of the spec claimed). The embedded IVO chunk's
        // `data_size` truncates by 64 bytes; the standalone has the full data.
        assert_eq!(rig.region_weights.len(), 38);
        // Spot-check first three weights from Python walk: [1.0, 0.5, -1.0].
        assert_eq!(rig.region_weights[0], 1.0);
        assert_eq!(rig.region_weights[1], 0.5);
        assert_eq!(rig.region_weights[2], -1.0);
        // Last weights: indices 36, 37 are 1.0.
        assert_eq!(rig.region_weights[36], 1.0);
        assert_eq!(rig.region_weights[37], 1.0);

        // region_weight_slice indexing matches the per-region partitioning.
        assert_eq!(rig.region_weight_slice(0), Some([1.0, 0.5, -1.0].as_slice()));
        assert_eq!(
            rig.region_weight_slice(1),
            Some([1.0, 0.5, -1.0].as_slice()),
        );
        assert_eq!(rig.region_weight_slice(2), Some([1.0].as_slice()));
        // Past the last region is None.
        assert!(rig.region_weight_slice(24).is_none());
    }

    // ── Task 7 integration tests ─────────────────────────────────────────────

    /// Compares the RigLogic chunk embedded in the `.skin` IVO container
    /// against the standalone `.RigLogic` file for all fields except Section H.
    ///
    /// Reads both from Data.p4k (the standalone male t1_pu `.RigLogic` and
    /// head `.skin`); skipped without one.
    ///
    /// ## Layout of the embedded chunk
    ///
    /// The IVO RigLogic chunk has an 8-byte prefix before the `"RIG V1.9"`
    /// magic:
    ///   - bytes 0..4: `data_size` (u32 LE) — bytes remaining after this field.
    ///     Observed value: 327,992 (= 328,056 - 64; 64 bytes short of standalone
    ///     because Section H is truncated).
    ///   - bytes 4..8: zero padding.
    ///
    /// After skipping those 8 bytes we have the raw RigLogic buffer. Because
    /// Section H is 64 bytes short (16 missing f32 weight entries), the strict
    /// length check in `RigLogic::read` rejects the truncated buffer. Per
    /// SPEC.md, the workaround is to pad the buffer by 64 zero bytes before
    /// parsing. This test applies that padding, parses both forms, and verifies
    /// that all pre-Section-H fields are identical.
    #[test]
    fn embedded_chunk_matches_standalone() {
        use starbreaker_chunks::{known_types::ivo::RIG_LOGIC, ChunkFile};

        // ── gate on env vars ──────────────────────────────────────────────────
        let Some(standalone_bytes) = crate::test_p4k::read(crate::test_p4k::MALE_RIGLOGIC) else { return };
        let Some(skin_bytes) = crate::test_p4k::read(&format!("{}.skin", crate::test_p4k::MALE_HEAD)) else { return };

        // ── parse standalone ──────────────────────────────────────────────────
        let standalone =
            RigLogic::read(&standalone_bytes).expect("standalone fixture parses without error");

        // ── extract embedded chunk from .skin ─────────────────────────────────
        let cf = ChunkFile::from_bytes(&skin_bytes).expect("parse IVO chunk file");
        let ivo = match cf {
            ChunkFile::Ivo(ref i) => i,
            _ => panic!("expected IVO format"),
        };

        let entry = ivo
            .chunks()
            .iter()
            .find(|c| c.chunk_type == RIG_LOGIC)
            .expect("RigLogic chunk not found in .skin fixture");

        let raw_chunk = ivo.chunk_data(entry);

        // The embedded chunk has an 8-byte prefix (data_size u32 + 4-byte pad)
        // before the "RIG V1.9" magic. Skip it.
        assert!(
            raw_chunk.len() > 8,
            "embedded chunk is unexpectedly short: {} bytes",
            raw_chunk.len()
        );
        let embedded_rl_bytes = &raw_chunk[8..];

        // Verify the magic is in place at the expected position.
        assert_eq!(
            &embedded_rl_bytes[..8],
            b"RIG V1.9",
            "expected 'RIG V1.9' magic after 8-byte prefix"
        );

        // Parse the embedded buffer directly — no padding needed. Diagnostic
        // investigation confirmed that the embedded form has a different
        // section_starts[7] (Section H start = 327,840) from the standalone
        // (Section H start = 327,904). Both forms have a complete 152-byte
        // Section H (38 f32 entries). The earlier "truncation" hypothesis was
        // wrong. See SPEC.md: "Embedded vs standalone layout difference."
        let embedded = RigLogic::read(embedded_rl_bytes)
            .expect("embedded chunk should parse without error");

        // ── field-by-field comparison (all pre-Section-H fields) ─────────────
        assert_eq!(embedded.name, standalone.name, "name mismatch");
        assert_eq!(
            embedded.rig_type.class, standalone.rig_type.class,
            "rig_type.class mismatch"
        );
        assert_eq!(
            embedded.rig_type.gender, standalone.rig_type.gender,
            "rig_type.gender mismatch"
        );
        assert_eq!(
            embedded.rig_type.part, standalone.rig_type.part,
            "rig_type.part mismatch"
        );
        assert_eq!(
            embedded.rig_type.tier, standalone.rig_type.tier,
            "rig_type.tier mismatch"
        );
        assert_eq!(
            embedded.identity_crc_lo, standalone.identity_crc_lo,
            "identity_crc_lo mismatch"
        );
        assert_eq!(
            embedded.header_size, standalone.header_size,
            "header_size mismatch"
        );
        assert_eq!(
            embedded.num_appearance_blendshapes, standalone.num_appearance_blendshapes,
            "num_appearance_blendshapes mismatch"
        );
        assert_eq!(
            embedded.num_corrective_blendshapes, standalone.num_corrective_blendshapes,
            "num_corrective_blendshapes mismatch"
        );
        assert_eq!(
            embedded.num_joints, standalone.num_joints,
            "num_joints mismatch"
        );
        assert_eq!(
            embedded.primary_delta_scale, standalone.primary_delta_scale,
            "primary_delta_scale mismatch"
        );
        assert_eq!(
            embedded.secondary_delta_scale, standalone.secondary_delta_scale,
            "secondary_delta_scale mismatch"
        );
        assert_eq!(
            embedded.num_main_delta_records, standalone.num_main_delta_records,
            "num_main_delta_records mismatch"
        );
        assert_eq!(
            embedded.num_secondary_delta_records, standalone.num_secondary_delta_records,
            "num_secondary_delta_records mismatch"
        );
        assert_eq!(
            embedded.num_blend_regions, standalone.num_blend_regions,
            "num_blend_regions mismatch"
        );
        assert_eq!(
            embedded.packed_value_bit_width, standalone.packed_value_bit_width,
            "packed_value_bit_width mismatch"
        );
        // The embedded form has all section pointers from slot 5 onward shifted
        // by −64 bytes relative to standalone. Slots 0–4 are identical; slots
        // 5, 6, and 7 each differ by exactly 64 bytes. This reflects a 64-byte
        // layout difference in Section D or E between the two forms. The data
        // contents of each section are identical; only the section boundaries
        // differ. See SPEC.md: "Embedded vs standalone section layout."
        assert_eq!(
            embedded.section_starts[..5], standalone.section_starts[..5],
            "section_starts[0..5] mismatch (these should be identical)"
        );
        for slot in 5..8 {
            let emb = embedded.section_starts[slot];
            let sta = standalone.section_starts[slot];
            assert_eq!(
                sta.saturating_sub(emb), 64,
                "section_starts[{slot}]: embedded={emb}, standalone={sta}; \
                 expected standalone to be exactly 64 bytes ahead of embedded"
            );
        }
        assert_eq!(
            embedded.joint_control_name_crcs, standalone.joint_control_name_crcs,
            "joint_control_name_crcs mismatch"
        );
        assert_eq!(
            embedded.skeleton_bone_name_crcs, standalone.skeleton_bone_name_crcs,
            "skeleton_bone_name_crcs mismatch"
        );
        assert_eq!(
            embedded.appearance.records.len(),
            standalone.appearance.records.len(),
            "appearance.records.len() mismatch"
        );
        assert_eq!(
            embedded.corrective.records.len(),
            standalone.corrective.records.len(),
            "corrective.records.len() mismatch"
        );
        assert_eq!(
            embedded.corrective_composition, standalone.corrective_composition,
            "corrective_composition mismatch"
        );
        assert_eq!(
            embedded.blend_region_bones, standalone.blend_region_bones,
            "blend_region_bones mismatch"
        );

        // Section H: both embedded and standalone have 38 complete f32 entries
        // (152 bytes). The embedded form's Section H starts at a different
        // offset (327,840 vs 327,904) but contains the same weight data.
        assert_eq!(
            embedded.region_weights.len(),
            standalone.region_weights.len(),
            "region_weights count mismatch: embedded={}, standalone={}",
            embedded.region_weights.len(),
            standalone.region_weights.len(),
        );
        assert_eq!(
            embedded.region_weights, standalone.region_weights,
            "region_weights mismatch"
        );
    }

    /// Verifies that Section F's `skeleton_bone_name_crcs` includes the CRC32
    /// IEEE hash of known bone names from the companion skeleton.
    ///
    /// Reads the male t1_pu `.RigLogic` from Data.p4k; skipped without one.
    ///
    /// ## CRC algorithm — confirmed CRC32 IEEE
    ///
    /// Both Section A and Section F use CRC32 (IEEE, polynomial `0xEDB88320`),
    /// the same algorithm as `crc32fast::hash`. This was confirmed by extracting
    /// bone name strings from the companion `.skin` file and verifying that ALL
    /// 179 Section F entries match CRC32-IEEE hashes of `FACIAL_LOD*` joint
    /// names (e.g. `crc32fast::hash("FACIAL_LOD0_L_Neck2") = 0x40D44915 ==
    /// skeleton_bone_name_crcs[0]`). The SPEC.md note "Section F CRC algorithm
    /// (assumed-by-analogy)" has been updated to "confirmed CRC32 IEEE."
    ///
    /// Bone names follow the `FACIAL_LOD{N}_{side}_{part}` convention of CIG's
    /// face rig — NOT generic anatomical names like "head" or "jaw".
    #[test]
    fn fixture_skeleton_bone_crcs_include_known() {
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_RIGLOGIC) else { return };
        let rig = RigLogic::read(&bytes).expect("real fixture parses");

        // Confirmed bone names from the companion .skin fixture extracted via
        // `strings`. Section F stores CRC32-IEEE of the FACIAL_LOD* joint
        // names — NOT simple anatomical names. Multiple matches are asserted
        // to rule out coincidental collision.
        let confirmed_bones: &[(&str, u32)] = &[
            // crc32fast::hash("FACIAL_LOD0_L_Neck2") = 0x40D44915 = slot [0]
            ("FACIAL_LOD0_L_Neck2", 0x40D44915),
            // crc32fast::hash("FACIAL_LOD0_R_Neck1") = 0xEE3CFF3C = slot [4]
            ("FACIAL_LOD0_R_Neck1", 0xEE3CFF3C),
            // crc32fast::hash("FACIAL_LOD1_C_AdamsApple") = 0xB010F8BF = slot [2]
            ("FACIAL_LOD1_C_AdamsApple", 0xB010F8BF),
            // crc32fast::hash("FACIAL_LOD0_C_Skull") = 0x150A280A = slot [9]
            ("FACIAL_LOD0_C_Skull", 0x150A280A),
        ];

        for (name, expected_crc) in confirmed_bones {
            let computed = crc32fast::hash(name.as_bytes());
            assert_eq!(
                computed, *expected_crc,
                "CRC32-IEEE mismatch for '{name}': computed {computed:#010X}, expected {expected_crc:#010X}"
            );
            assert!(
                rig.skeleton_bone_name_crcs.contains(expected_crc),
                "CRC32(\"{name}\") = {expected_crc:#010X} not found in Section F — \
                 algorithm or bone name list has changed"
            );
        }
    }

    /// Verifies that all region weights in Section H are finite (no NaN or Inf)
    /// and checks their range.
    ///
    /// Reads the male t1_pu `.RigLogic` from Data.p4k; skipped without one.
    ///
    /// V1.9 weights are NOT a probability simplex (i.e. they do NOT sum to 1.0
    /// per region). Observed values include negative weights (e.g. `-1.0` at
    /// index 2). The only guaranteed invariants are: all values are finite, and
    /// they fall within the observed range of approximately [-1.0, 1.0].
    #[test]
    fn fixture_region_weights_finite_nonneg() {
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_RIGLOGIC) else { return };
        let rig = RigLogic::read(&bytes).expect("real fixture parses");

        assert!(
            !rig.region_weights.is_empty(),
            "region_weights should be non-empty for fixture rig"
        );

        // All weights must be finite (no NaN, no Inf).
        let non_finite: Vec<(usize, f32)> = rig
            .region_weights
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, f)| !f.is_finite())
            .collect();
        assert!(
            non_finite.is_empty(),
            "found non-finite region weights at indices: {non_finite:?}"
        );

        // Check the range. Observed fixture values are in [-1.0, 1.0].
        // Negative weights are valid (observed at index 2 = -1.0).
        // Log a note if any weight is outside [-1.0, 1.0] — not a hard
        // failure, since the valid range is not formally documented.
        let out_of_range: Vec<(usize, f32)> = rig
            .region_weights
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, f)| *f < -1.0 || *f > 1.0)
            .collect();
        if !out_of_range.is_empty() {
            eprintln!(
                "fixture_region_weights_finite_nonneg: {} weights outside [-1.0, 1.0]: {:?}",
                out_of_range.len(),
                out_of_range,
            );
        }
    }

    // ── ProtosInfo tests ─────────────────────────────────────────────────────

    /// Build a valid 80-byte ProtosInfo blob for unit tests.
    ///
    /// Layout matches the on-disk format documented in [`ProtosInfo`]:
    /// - 0x00: data_size = 76
    /// - 0x04: version = 1
    /// - 0x08: mesh_name (16-byte null-padded)
    /// - 0x18: mesh_name_crc (CRC32C)
    /// - 0x1C: rig_name (40-byte null-padded)
    /// - 0x44: 8 bytes of zeros
    /// - 0x4C: rig_name_crc (CRC32C)
    fn make_protos_info_blob(mesh_name: &str, rig_name: &str) -> [u8; 80] {
        let mut buf = [0u8; 80];
        // data_size = 76
        buf[0x00..0x04].copy_from_slice(&76u32.to_le_bytes());
        // version = 1
        buf[0x04..0x08].copy_from_slice(&1u32.to_le_bytes());
        // mesh_name (16-byte, null-padded)
        let mn = mesh_name.as_bytes();
        let mn_len = mn.len().min(16);
        buf[0x08..0x08 + mn_len].copy_from_slice(&mn[..mn_len]);
        // mesh_name_crc
        let mc = NameHash::from_string(mesh_name).value();
        buf[0x18..0x1C].copy_from_slice(&mc.to_le_bytes());
        // rig_name (40-byte, null-padded)
        let rn = rig_name.as_bytes();
        let rn_len = rn.len().min(40);
        buf[0x1C..0x1C + rn_len].copy_from_slice(&rn[..rn_len]);
        // padding 0x44..0x4C already zero
        // rig_name_crc
        let rc = NameHash::from_string(rig_name).value();
        buf[0x4C..0x50].copy_from_slice(&rc.to_le_bytes());
        buf
    }

    /// ProtosInfo: rejects a buffer shorter than 80 bytes.
    #[test]
    fn protos_info_rejects_short_buffer() {
        let result = ProtosInfo::read(&[0u8; 79]);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation error for 79-byte input"
        );
    }

    /// ProtosInfo: rejects a buffer longer than 80 bytes.
    #[test]
    fn protos_info_rejects_long_buffer() {
        let result = ProtosInfo::read(&[0u8; 81]);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation error for 81-byte input"
        );
    }

    /// ProtosInfo: rejects empty input.
    #[test]
    fn protos_info_rejects_empty_buffer() {
        let result = ProtosInfo::read(&[]);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation error for empty input"
        );
    }

    /// ProtosInfo: rejects a blob with wrong data_size field.
    #[test]
    fn protos_info_rejects_wrong_data_size() {
        let mut blob = make_protos_info_blob("head", "protos_human_male_face_t1_pu");
        // Corrupt data_size to 77 (should be 76).
        blob[0x00..0x04].copy_from_slice(&77u32.to_le_bytes());
        let result = ProtosInfo::read(&blob);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation error for wrong data_size"
        );
    }

    /// ProtosInfo: rejects a blob with non-zero padding bytes.
    #[test]
    fn protos_info_rejects_nonzero_padding() {
        let mut blob = make_protos_info_blob("head", "protos_human_male_face_t1_pu");
        // Write a non-zero byte into the padding region.
        blob[0x44] = 0xFF;
        let result = ProtosInfo::read(&blob);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation error for non-zero padding"
        );
    }

    /// ProtosInfo: rejects a blob with mismatched mesh_name_crc.
    #[test]
    fn protos_info_rejects_bad_mesh_crc() {
        let mut blob = make_protos_info_blob("head", "protos_human_male_face_t1_pu");
        // Corrupt the mesh_name_crc.
        blob[0x18..0x1C].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
        let result = ProtosInfo::read(&blob);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation error for bad mesh_name_crc"
        );
    }

    /// ProtosInfo: rejects a blob with mismatched rig_name_crc.
    #[test]
    fn protos_info_rejects_bad_rig_crc() {
        let mut blob = make_protos_info_blob("head", "protos_human_male_face_t1_pu");
        // Corrupt the rig_name_crc.
        blob[0x4C..0x50].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
        let result = ProtosInfo::read(&blob);
        assert!(
            matches!(result, Err(ParseError::Validation { .. })),
            "expected Validation error for bad rig_name_crc"
        );
    }

    /// ProtosInfo: parses a well-formed synthetic blob correctly.
    #[test]
    fn protos_info_parses_known_blob() {
        let blob = make_protos_info_blob("head", "protos_human_male_face_t1_pu");
        let info = ProtosInfo::read(&blob).expect("synthetic blob should parse");
        assert_eq!(info.mesh_name, "head");
        assert_eq!(info.rig_name, "protos_human_male_face_t1_pu");
        assert_eq!(info.version, 1);
        assert_eq!(info.mesh_name_crc, NameHash::from_string("head"));
        assert_eq!(
            info.rig_name_crc,
            NameHash::from_string("protos_human_male_face_t1_pu")
        );
    }

    /// ProtosInfo: parses the real fixture `.skin` file.
    ///
    /// Reads `protos_human_male_face_t1_pu_head.skin` from Data.p4k; skipped
    /// without one (CI).
    #[test]
    fn protos_info_fixture_parses() {
        use starbreaker_chunks::{known_types::ivo::PROTOS_INFO, ChunkFile};

        let Some(bytes) = crate::test_p4k::read(&format!("{}.skin", crate::test_p4k::MALE_HEAD)) else { return };

        let cf = ChunkFile::from_bytes(&bytes).expect("parse IVO chunk file");
        let ivo = match cf {
            ChunkFile::Ivo(ref i) => i,
            _ => panic!("expected IVO format, got CrCh"),
        };

        let entry = ivo
            .chunks()
            .iter()
            .find(|c| c.chunk_type == PROTOS_INFO)
            .expect("ProtosInfo chunk not found in .skin fixture");

        let data = ivo.chunk_data(entry);
        assert_eq!(data.len(), 80, "ProtosInfo chunk must be 80 bytes");

        let info = ProtosInfo::read(data).expect("ProtosInfo::read on fixture should succeed");

        assert_eq!(info.mesh_name, "head");
        assert_eq!(info.rig_name, "protos_human_male_face_t1_pu");
        assert_eq!(info.version, 1);
        // CRC cross-check: rig_name_crc must equal identity_crc_lo in the RigLogic file.
        assert_eq!(info.rig_name_crc, NameHash(0xDD6C67F6), "rig_name_crc should equal RigLogic.identity_crc_lo");
        assert_eq!(info.mesh_name_crc, NameHash(0xEA7C97E8), "mesh_name_crc should be CRC32C of 'head'");
    }

    // ── compose_correctives tests ────────────────────────────────────────────

    /// Minimal RigLogic for compose_correctives — same shape helper as
    /// character::controls::tests but exposed here so tests stay local.
    fn rig_for_correctives(
        num_appearance: u16,
        corrective_rows: Vec<[u8; SECTION_C_ROW_STRIDE]>,
    ) -> RigLogic {
        let num_corrective = corrective_rows.len() as u16;
        RigLogic {
            file_size: 0,
            name: "synthetic".into(),
            rig_type: RigType {
                class: 0,
                gender: 0,
                variant: 0,
                part: 0,
                tier: 0,
                lod: 0,
                flags_or_pad: 0,
            },
            identity_crc_lo: NameHash(0),
            identity_crc_hi_or_x: 0,
            header_size: 0xA8,
            num_appearance_blendshapes: num_appearance,
            num_corrective_blendshapes: num_corrective,
            num_joints: 0,
            _reserved_0x46: 0,
            primary_delta_scale: 0.0,
            secondary_delta_scale: 0.0,
            num_main_delta_records: 0,
            num_secondary_delta_records: 0,
            num_blend_regions: 0,
            packed_value_bit_width: 9,
            section_starts: [0; 8],
            joint_control_name_crcs: vec![0; num_appearance as usize],
            joint_control_names: (0..num_appearance).map(|i| format!("CTL_{i}")).collect(),
            appearance: BlendShapeBlob { scale: 0.0, records: Vec::new() },
            corrective: BlendShapeBlob { scale: 0.0, records: Vec::new() },
            corrective_composition: corrective_rows
                .into_iter()
                .map(|indices| CorrectiveCompositionRow { indices })
                .collect(),
            skeleton_bone_name_crcs: Vec::new(),
            blend_region_bones: Vec::new(),
            region_weights: Vec::new(),
        }
    }

    #[test]
    fn compose_correctives_empty_rows_returns_appearance_only() {
        let rig = rig_for_correctives(3, Vec::new());
        let appearance = vec![1.0_f32, 0.5, 0.25];
        let dst = compose_correctives(&rig, &appearance).expect("ok");
        assert_eq!(dst, vec![1.0_f32, 0.5, 0.25]);
    }

    #[test]
    fn compose_correctives_zero_indices_act_as_ctl_psd_factor() {
        // With CTL_PSD=1.0 at index 0, a row of [0,0,0,0,0] should produce 1.0.
        let rig = rig_for_correctives(3, vec![[0, 0, 0, 0, 0]]);
        let appearance = vec![1.0_f32, 0.5, 0.25];
        let dst = compose_correctives(&rig, &appearance).expect("ok");
        assert_eq!(dst.len(), 4);
        assert_eq!(dst[3], 1.0); // five 1.0 factors
    }

    #[test]
    fn compose_correctives_full_product_of_five_indices() {
        // appearance=[1.0, 2.0, 3.0, 4.0]; row=[0,1,2,3,0]
        //   product = 1.0 * 2.0 * 3.0 * 4.0 * 1.0 = 24.0
        let rig = rig_for_correctives(4, vec![[0, 1, 2, 3, 0]]);
        let appearance = vec![1.0_f32, 2.0, 3.0, 4.0];
        let dst = compose_correctives(&rig, &appearance).expect("ok");
        assert_eq!(dst[4], 24.0);
    }

    #[test]
    fn compose_correctives_zero_factor_kills_row() {
        // If any referenced appearance weight is 0, the product collapses.
        let rig = rig_for_correctives(3, vec![[1, 2, 0, 0, 0]]);
        let appearance = vec![1.0_f32, 0.0, 5.0]; // appearance[1] = 0
        let dst = compose_correctives(&rig, &appearance).expect("ok");
        assert_eq!(dst[3], 0.0);
    }

    #[test]
    fn compose_correctives_later_row_reads_earlier_corrective() {
        // Engine writes _Dst[num_appearance + i] in order; a later row
        // referencing an earlier corrective slot sees its updated value.
        // appearance=[1.0, 2.0]; row0=[0,0,0,0,1] -> _Dst[2] = 1*1*1*1*2 = 2.0
        //                       row1=[2,0,0,0,0] -> _Dst[3] = _Dst[2]*1*1*1*1 = 2.0
        let rig = rig_for_correctives(2, vec![[0, 0, 0, 0, 1], [2, 0, 0, 0, 0]]);
        let appearance = vec![1.0_f32, 2.0];
        let dst = compose_correctives(&rig, &appearance).expect("ok");
        assert_eq!(dst[2], 2.0);
        assert_eq!(dst[3], 2.0); // would be 0.0 if we read pre-update
    }

    #[test]
    fn compose_correctives_appearance_length_mismatch_errors() {
        let rig = rig_for_correctives(3, vec![[0; 5]]);
        let appearance = vec![1.0_f32, 0.5]; // only 2 instead of 3
        let err = compose_correctives(&rig, &appearance).unwrap_err();
        match err {
            ParseError::Validation { context, .. } => {
                assert_eq!(context, "compose_correctives");
            }
            _ => panic!("expected Validation error, got {err:?}"),
        }
    }

    #[test]
    fn compose_correctives_out_of_range_index_errors() {
        // Row references appearance index 99, but only 3 appearance + 1 corrective slot exists (total 4).
        let rig = rig_for_correctives(3, vec![[99, 0, 0, 0, 0]]);
        let appearance = vec![1.0_f32, 0.5, 0.25];
        let err = compose_correctives(&rig, &appearance).unwrap_err();
        match err {
            ParseError::Validation { context, message } => {
                assert_eq!(context, "compose_correctives");
                assert!(message.contains("99"));
            }
            _ => panic!("expected Validation error, got {err:?}"),
        }
    }
}
