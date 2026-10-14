//! `.dna` file structural parser.
//!
//! Decodes the 0x40-byte header, the 0x100-byte pointer table at offset
//! `+0x40`, and exposes the payload byte spans pointed to by each
//! non-null pointer-table slot. Slot 31 (`+0x138`) is identified as the
//! per-variant delta-records array (anchored against
//! `Protos::ComputeMixedDeltaMatrixNumEntries` at `FUN_14142a5c0`).
//!
//! Semantic interpretation of the runtime
//! `ScatteredDataInterpolation` arrays — variant feature-space control
//! points (`double[3]`) and variant-space tetrahedra (`byte[4]`) — is
//! NOT in this layer. The Ghidra anchoring pass on the master `.dna`
//! (Phase 4a extension) confirmed those arrays are NOT stored as flat
//! pointer-table slots in the file; they are built at character-load
//! time by one of the tetrahedralization algorithms registered at
//! `FUN_140087990` ("Default", "CGAL", "CDT", "fTet"). See
//! `docs/riglogic/SPEC.md` ("`.dna` file format — anchored slot 31"
//! and "HALT: tetrahedra + control points") for the full evidence.
//!
//! Public types:
//! - [`DnaHeader`]: fixed inline header fields (0x00..0x40).
//! - [`DnaSegment`]: one nonzero pointer-table entry resolved to a byte
//!   span.
//! - [`DnaFile`]: full parsed view with header + list of segments.

use crate::error::DnaError;

/// 16-byte file magic prefix. Stored as ASCII `"DNA V1.6"` followed by
/// eight NUL bytes for the master male-head fixture; the female-head
/// fixture and other variants carry non-zero bytes at `0x0C..0x10`
/// (observed values like `0x000001B3`), so only the first
/// [`MAGIC_PREFIX_LEN`] bytes are validated against the canonical magic.
///
/// The Load function checks this as two integer compares:
/// `*(u64*)0 == 0x362e315620414e44` ("DNA V1.6" little-endian) and
/// `*(u32*)8 == 0`. Bytes `0x0C..0x10` are NOT validated by the engine.
pub const MAGIC: [u8; 16] = *b"DNA V1.6\0\0\0\0\0\0\0\0";

/// Number of leading bytes of [`MAGIC`] the engine validates. The
/// trailing 4 bytes (`0x0C..0x10`) are a free-form variant/build field
/// (see [`MAGIC`] for evidence).
pub const MAGIC_PREFIX_LEN: usize = 12;

/// File offset at which the pointer table starts.
pub const POINTER_TABLE_OFFSET: usize = 0x40;

/// Number of `u64` slots in the pointer table.
///
/// `FUN_14142a990` performs fixups from offset `+0x40` up to and
/// including `+0x138` inclusive, i.e. 32 slots × 8 bytes = 256 bytes.
/// (`+0x140` immediately after the table is reserved for the path-
/// string back-reference the loader writes at runtime.)
pub const POINTER_TABLE_SLOTS: usize = 32;

/// Minimum buffer size required to parse the header + pointer table.
pub const MIN_FILE_SIZE: usize = POINTER_TABLE_OFFSET + POINTER_TABLE_SLOTS * 8;

/// Pointer-table slot index (0-based) holding the per-variant
/// delta-records array.
///
/// Anchored to `Protos::ComputeMixedDeltaMatrixNumEntries`
/// (`FUN_14142a5c0`) which reads `*(longlong *)(param_3 + 0x138)` —
/// equivalently slot `(0x138 - 0x40) / 8 = 31`. The loader's fixup pass
/// (`FUN_14142a990`) treats this slot specially: in addition to
/// rebasing the slot itself, it walks the array and rebases two
/// internal pointer fields per variant record (offsets `+0x18` and
/// `+0x20`).
///
/// This slot is **null in LOD `.dna` files** (LOD1 in the fixture has
/// 0 here) and **populated in the master**.
pub const VARIANT_ARRAY_SLOT: usize = 31;

/// Byte stride of one variant-record header inside the slot-31 array.
///
/// Anchored to `FUN_14142a990`'s rebase loop, which steps the variant
/// pointer by 5 longlongs = 40 bytes per iteration. Per-variant header
/// layout (verified by reading the master `.dna`):
/// - `+0x00 u32`: per-variant `num_main_delta_records`
/// - `+0x04 u32`: per-variant `num_secondary_delta_records`
/// - `+0x08 f64`: per-variant `primary_delta_scale`
/// - `+0x10 f64`: per-variant `secondary_delta_scale`
/// - `+0x18 u64`: file offset of this variant's packed main deltas
/// - `+0x20 u64`: file offset of this variant's packed secondary deltas
pub const VARIANT_RECORD_STRIDE: usize = 0x28;

/// Inline header fields read directly from bytes `0x00..0x40` of the file.
///
/// Field semantics anchored against Ghidra:
/// - `magic` — `*plVar1` / `*(u32*)8` magic check in `FUN_14142d280`.
/// - `timestamp` — cross-LOD identity check in
///   `DNAFile::LoadAllLODs` (`*(longlong *)(lVar8 + 0x10)`).
/// - `protos_name_id` — `*(int *)(lVar8 + 0x18)` compared to
///   `param_4[0x12]` in `LoadAllLODs`.
/// - `mesh_name_id` — `*(int *)(lVar8 + 0x1c)` compared to
///   `param_4[5]` in `LoadAllLODs`.
/// - `num_variants` — `*(short *)(lVar8 + 0x2a)` zero-check in
///   `LoadAllLODs` (`"Zero variants found"`); also the count used to
///   walk the slot-31 variant array in
///   `Protos::ComputeMixedDeltaMatrixNumEntries`
///   (`*(ushort *)(param_3 + 0x2a)`).
/// - `num_vertices` — `u32` at `+0x30`. Anchored empirically by
///   comparing the master `.dna` value (5584) against the head `.skin`
///   IVO `MeshIvo320` chunk's `num_vertices` field (5584); the LOD1
///   `.dna` carries the decimated LOD1 vertex count (3158). Each LOD
///   `.dna` therefore declares the vertex count of its own LOD-N mesh.
/// - `num_indices` — `u32` at `+0x34`. Same anchoring: master value
///   32178 equals the head `.skin`'s `num_indices` (= 10726 tris × 3).
///   LOD1 carries 17724 indices, the decimated count.
///
/// The remaining `header_u32_*` / `header_u16_*` fields still have no
/// firm Ghidra anchor. They are exposed verbatim so consumers can record
/// them and re-anchor against further LOD files later.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DnaHeader {
    /// Bytes `0x00..0x10`. Verified to equal [`MAGIC`].
    pub magic: [u8; 16],
    /// `u64` at `0x10` — content/timestamp identity used to bind LODs
    /// of the same protos together.
    pub timestamp: u64,
    /// `u32` at `0x18` — CRC32 of the protos asset name.
    pub protos_name_id: u32,
    /// `u32` at `0x1c` — CRC32 of the mesh (sub-asset) name.
    pub mesh_name_id: u32,
    /// `u32` at `0x20` (semantics not anchored).
    pub header_u32_20: u32,
    /// `u16` at `0x24` (semantics not anchored).
    pub header_u16_24: u16,
    /// `u8` at `0x26` — `num_face_parts` (count of face-part-type labels
    /// in slot 4, anchored as 3 for the master fixture). NOT the same as
    /// the customizer's `blends_per_pool` (`mesh_blends_per_pool`).
    /// Anchored in `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.6 (Pass 6).
    pub num_face_parts: u8,
    /// `u8` at `0x27` (top-K sparse slot count, anchored as 4 for the
    /// master fixture). Not load-bearing for the morphing pipeline.
    pub header_u8_27: u8,
    /// `u8` at `0x28` — `mesh_blends_per_pool` (= `mesh[+0x28]` at
    /// runtime). 13 for the master human-head fixture; the inner-loop
    /// count for the per-(joint × face-part) fold in `ExecuteHuman`.
    /// Anchored in `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.7 / §6.8.
    pub mesh_blends_per_pool: u8,
    /// `u8` at `0x29` — duplicate or cross-check of `mesh_blends_per_pool`
    /// (also 13 in the master fixture). Semantics not anchored.
    pub header_u8_29: u8,
    /// `u16` at `0x2a` — number of variants. Must be > 0 for the
    /// runtime to accept the file.
    pub num_variants: u16,
    /// `u16` at `0x2c` (semantics not anchored).
    pub header_u16_2c: u16,
    /// `u16` at `0x2e` (semantics not anchored).
    pub header_u16_2e: u16,
    /// `u32` at `0x30` — number of vertices in this LOD's mesh.
    /// Anchored by cross-validation against the head `.skin`'s
    /// `MeshIvo320.num_vertices`.
    pub num_vertices: u32,
    /// `u32` at `0x34` — number of indices in this LOD's mesh.
    /// Anchored by cross-validation against the head `.skin`'s
    /// `MeshIvo320.num_indices`.
    pub num_indices: u32,
    /// `u32` at `0x38` (semantics not anchored — likely a count).
    pub header_u32_38: u32,
    /// `u32` at `0x3c` (semantics not anchored).
    pub header_u32_3c: u32,
}

impl DnaHeader {
    /// Parse the 0x40-byte inline header from `bytes`. Caller must
    /// already have verified `bytes.len() >= 0x40`.
    fn parse(bytes: &[u8]) -> Self {
        // Helper closures (no panic on the asserted-prefix range).
        let u16 = |off: usize| u16::from_le_bytes([bytes[off], bytes[off + 1]]);
        let u32 = |off: usize| {
            u32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ])
        };
        let u64 = |off: usize| {
            u64::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
                bytes[off + 4],
                bytes[off + 5],
                bytes[off + 6],
                bytes[off + 7],
            ])
        };

        let mut magic = [0u8; 16];
        magic.copy_from_slice(&bytes[..16]);

        Self {
            magic,
            timestamp: u64(0x10),
            protos_name_id: u32(0x18),
            mesh_name_id: u32(0x1c),
            header_u32_20: u32(0x20),
            header_u16_24: u16(0x24),
            num_face_parts: bytes[0x26],
            header_u8_27: bytes[0x27],
            mesh_blends_per_pool: bytes[0x28],
            header_u8_29: bytes[0x29],
            num_variants: u16(0x2a),
            header_u16_2c: u16(0x2c),
            header_u16_2e: u16(0x2e),
            num_vertices: u32(0x30),
            num_indices: u32(0x34),
            header_u32_38: u32(0x38),
            header_u32_3c: u32(0x3c),
        }
    }
}

/// One resolved pointer-table entry.
///
/// `slot_index` is the index in the 32-entry table (0..32).
/// `file_offset` is the byte offset into the file at which the segment
/// starts. `length` is the conservative segment length: it stops at
/// either the next-larger non-null slot or, if this is the largest
/// non-null slot, at the end of the buffer.
///
/// Note that the engine does not record an explicit length per slot —
/// it infers length from the in-header counts plus the slot's role.
/// The `length` here is therefore a SAFE upper bound on the segment's
/// extent within the file, not the semantic payload length.
#[derive(Debug, Clone, Copy)]
pub struct DnaSegment {
    /// 0..32 — index into the pointer table.
    pub slot_index: usize,
    /// Pointer-table byte offset (`POINTER_TABLE_OFFSET + slot_index*8`).
    pub slot_byte_offset: usize,
    /// File-relative offset the slot pointed to.
    pub file_offset: usize,
    /// Conservative span length (see type-level docs).
    pub length: usize,
}

/// One parsed entry of the per-variant delta-records array stored at
/// pointer-table slot 31 (`+0x138`).
///
/// See [`VARIANT_ARRAY_SLOT`] and [`VARIANT_RECORD_STRIDE`] for the
/// anchoring. Every field is read verbatim from the on-disk record;
/// the two `…_offset` fields are **post-fixup file offsets** (i.e.
/// `FUN_14142a990` has already added the file base) when read from
/// a freshly-loaded buffer — we expose the raw on-disk relative offsets
/// here, since we never apply the engine's destructive fixup.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VariantRecord {
    /// `u32 @ +0x00` — number of packed main (appearance) delta entries
    /// for this variant.
    pub num_main_delta_records: u32,
    /// `u32 @ +0x04` — number of packed secondary (corrective) delta
    /// entries for this variant.
    pub num_secondary_delta_records: u32,
    /// `f64 @ +0x08` — primary-delta scale factor (per-variant
    /// override of the RigLogic Section A `primary_delta_scale`).
    pub primary_delta_scale: f64,
    /// `f64 @ +0x10` — secondary-delta scale factor (per-variant
    /// override of the RigLogic Section A `secondary_delta_scale`).
    pub secondary_delta_scale: f64,
    /// `u64 @ +0x18` — raw on-disk file offset of this variant's
    /// packed main-delta entries.
    pub main_delta_entries_offset: u64,
    /// `u64 @ +0x20` — raw on-disk file offset of this variant's
    /// packed secondary-delta entries.
    pub secondary_delta_entries_offset: u64,
}

/// Full parsed view of a `.dna` file.
///
/// The body of the file is left as a reference back into the caller's
/// byte buffer — `DnaFile` borrows. Use [`DnaFile::segment_bytes`] to
/// look up the raw bytes for a given slot.
///
/// Memory characteristics: parsing is O(header + table + num_variants)
/// and allocates only the small `segments` and `variant_records` `Vec`s.
/// No payload bytes are copied.
#[derive(Debug)]
pub struct DnaFile<'a> {
    /// The underlying file buffer (borrowed).
    pub bytes: &'a [u8],
    /// Inline header fields (`0x00..0x40`).
    pub header: DnaHeader,
    /// Raw 32-entry pointer table (`0x40..0x140`). `pointer_table[i]`
    /// is the raw u64 offset stored on disk; 0 means "absent".
    pub pointer_table: [u64; POINTER_TABLE_SLOTS],
    /// One entry per *non-null* pointer-table slot, with the slot's
    /// conservative byte length filled in. Sorted by `slot_index`.
    pub segments: Vec<DnaSegment>,
    /// Per-variant delta-record headers parsed from slot 31
    /// ([`VARIANT_ARRAY_SLOT`]). Length equals `header.num_variants`
    /// when slot 31 is populated. **Empty** when slot 31 is null —
    /// this is the case for every LOD `.dna` we have observed; only
    /// the master `.dna` carries this slot.
    pub variant_records: Vec<VariantRecord>,
}

impl<'a> DnaFile<'a> {
    /// Parse a `.dna` file from its raw bytes.
    ///
    /// Strict checks performed:
    /// 1. Buffer is at least [`MIN_FILE_SIZE`] bytes (header + pointer
    ///    table).
    /// 2. First [`MAGIC_PREFIX_LEN`] bytes equal `MAGIC[..MAGIC_PREFIX_LEN]`.
    ///    Bytes `0x0C..0x10` are a per-variant field and not validated
    ///    (the engine itself only checks the first 12).
    /// 3. `num_variants > 0` (mirrors the "Zero variants found"
    ///    runtime check).
    /// 4. Every non-null pointer-table slot falls within the file
    ///    buffer and is at or above [`POINTER_TABLE_OFFSET`] +
    ///    pointer-table size (i.e. inside the payload region, not
    ///    inside the header or pointer table itself).
    pub fn read(bytes: &'a [u8]) -> Result<Self, DnaError> {
        if bytes.len() < MIN_FILE_SIZE {
            return Err(DnaError::HeaderTruncated {
                need: MIN_FILE_SIZE,
                have: bytes.len(),
            });
        }

        // 1. Magic — only the first MAGIC_PREFIX_LEN bytes are validated
        //    (engine anchor); bytes 12..16 vary by character variant.
        let mut got_magic = [0u8; 16];
        got_magic.copy_from_slice(&bytes[..16]);
        if got_magic[..MAGIC_PREFIX_LEN] != MAGIC[..MAGIC_PREFIX_LEN] {
            return Err(DnaError::InvalidMagic {
                expected: MAGIC,
                got: got_magic,
            });
        }

        // 2. Header fields (0x00..0x40).
        let header = DnaHeader::parse(&bytes[..0x40]);
        if header.num_variants == 0 {
            return Err(DnaError::ZeroVariants);
        }

        // 3. Pointer table (0x40..0x140).
        let mut pointer_table = [0u64; POINTER_TABLE_SLOTS];
        for (i, slot) in pointer_table.iter_mut().enumerate() {
            let off = POINTER_TABLE_OFFSET + i * 8;
            *slot = u64::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
                bytes[off + 4],
                bytes[off + 5],
                bytes[off + 6],
                bytes[off + 7],
            ]);
        }

        // 4. Validate each non-null slot, gather (slot_index, offset).
        let file_size = bytes.len();
        let payload_start = MIN_FILE_SIZE as u64;
        let mut sorted: Vec<(usize, usize)> = Vec::with_capacity(POINTER_TABLE_SLOTS);
        for (i, &slot) in pointer_table.iter().enumerate() {
            if slot == 0 {
                continue;
            }
            if slot < payload_start {
                return Err(DnaError::PointerInsideHeader {
                    slot: i,
                    offset: slot,
                });
            }
            if slot as usize > file_size {
                return Err(DnaError::PointerOutOfRange {
                    slot: i,
                    offset: slot,
                    file_size,
                });
            }
            sorted.push((i, slot as usize));
        }

        // 5. Compute conservative segment lengths by sorting offsets and
        //    using the next-larger offset (or end-of-file) as the upper
        //    bound. This is the same strategy used to identify segment
        //    sizes during reverse-engineering.
        let mut offsets_only: Vec<usize> = sorted.iter().map(|(_, off)| *off).collect();
        offsets_only.sort_unstable();
        offsets_only.push(file_size);

        let mut segments = Vec::with_capacity(sorted.len());
        for &(slot_index, file_offset) in &sorted {
            // Find next strictly greater offset.
            let next = offsets_only
                .iter()
                .find(|&&o| o > file_offset)
                .copied()
                .unwrap_or(file_size);
            segments.push(DnaSegment {
                slot_index,
                slot_byte_offset: POINTER_TABLE_OFFSET + slot_index * 8,
                file_offset,
                length: next - file_offset,
            });
        }
        // Sort by slot index for predictable iteration order.
        segments.sort_by_key(|s| s.slot_index);

        // 6. Parse the per-variant delta-records header table from slot 31,
        //    if present. Strict size check: the header table is exactly
        //    `num_variants * VARIANT_RECORD_STRIDE` bytes at the start of
        //    the slot. The slot's full conservative span will be larger
        //    than that (it also includes the per-variant packed delta
        //    payloads that the variant headers point at) — we only
        //    enforce that the buffer is big enough for the headers
        //    themselves.
        let mut variant_records = Vec::new();
        if let Some(slot) = pointer_table.get(VARIANT_ARRAY_SLOT).copied()
            && slot != 0
        {
            let base = slot as usize;
            let need = header.num_variants as usize * VARIANT_RECORD_STRIDE;
            let end = base
                .checked_add(need)
                .ok_or(DnaError::VariantArrayOverflow {
                    base: slot,
                    need,
                    file_size,
                })?;
            if end > file_size {
                return Err(DnaError::VariantArrayOverflow {
                    base: slot,
                    need,
                    file_size,
                });
            }

            variant_records.reserve_exact(header.num_variants as usize);
            for i in 0..header.num_variants as usize {
                let off = base + i * VARIANT_RECORD_STRIDE;
                variant_records.push(VariantRecord {
                    num_main_delta_records: u32::from_le_bytes([
                        bytes[off],
                        bytes[off + 1],
                        bytes[off + 2],
                        bytes[off + 3],
                    ]),
                    num_secondary_delta_records: u32::from_le_bytes([
                        bytes[off + 4],
                        bytes[off + 5],
                        bytes[off + 6],
                        bytes[off + 7],
                    ]),
                    primary_delta_scale: f64::from_le_bytes([
                        bytes[off + 8],
                        bytes[off + 9],
                        bytes[off + 10],
                        bytes[off + 11],
                        bytes[off + 12],
                        bytes[off + 13],
                        bytes[off + 14],
                        bytes[off + 15],
                    ]),
                    secondary_delta_scale: f64::from_le_bytes([
                        bytes[off + 16],
                        bytes[off + 17],
                        bytes[off + 18],
                        bytes[off + 19],
                        bytes[off + 20],
                        bytes[off + 21],
                        bytes[off + 22],
                        bytes[off + 23],
                    ]),
                    main_delta_entries_offset: u64::from_le_bytes([
                        bytes[off + 24],
                        bytes[off + 25],
                        bytes[off + 26],
                        bytes[off + 27],
                        bytes[off + 28],
                        bytes[off + 29],
                        bytes[off + 30],
                        bytes[off + 31],
                    ]),
                    secondary_delta_entries_offset: u64::from_le_bytes([
                        bytes[off + 32],
                        bytes[off + 33],
                        bytes[off + 34],
                        bytes[off + 35],
                        bytes[off + 36],
                        bytes[off + 37],
                        bytes[off + 38],
                        bytes[off + 39],
                    ]),
                });
            }
        }

        Ok(Self {
            bytes,
            header,
            pointer_table,
            segments,
            variant_records,
        })
    }

    /// Borrow the raw bytes for `segment`. Returns a sub-slice of the
    /// underlying file buffer.
    pub fn segment_bytes(&self, segment: &DnaSegment) -> &'a [u8] {
        &self.bytes[segment.file_offset..segment.file_offset + segment.length]
    }

    /// Find the segment for a specific pointer-table slot, if non-null.
    pub fn segment_for_slot(&self, slot_index: usize) -> Option<&DnaSegment> {
        self.segments
            .iter()
            .find(|s| s.slot_index == slot_index)
    }

    /// Convenience: borrow the bytes for slot `slot_index`, or `None`
    /// if that slot is null.
    pub fn slot_bytes(&self, slot_index: usize) -> Option<&'a [u8]> {
        self.segment_for_slot(slot_index)
            .map(|s| self.segment_bytes(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid `.dna` buffer with one non-null pointer
    /// slot pointing at a known payload.
    fn synth_minimal() -> Vec<u8> {
        // Layout: header (0x40) + table (0x100) + 32-byte payload.
        let mut buf = vec![0u8; MIN_FILE_SIZE + 32];
        buf[..16].copy_from_slice(&MAGIC);
        // timestamp
        buf[0x10..0x18].copy_from_slice(&0xdead_beef_u64.to_le_bytes());
        // protos_name_id = 0x11111111, mesh_name_id = 0x22222222
        buf[0x18..0x1c].copy_from_slice(&0x1111_1111_u32.to_le_bytes());
        buf[0x1c..0x20].copy_from_slice(&0x2222_2222_u32.to_le_bytes());
        // num_variants = 7 (just non-zero)
        buf[0x2a..0x2c].copy_from_slice(&7u16.to_le_bytes());
        // Pointer-table slot 0 -> offset 0x140 (start of payload).
        buf[POINTER_TABLE_OFFSET..POINTER_TABLE_OFFSET + 8]
            .copy_from_slice(&(MIN_FILE_SIZE as u64).to_le_bytes());
        // Payload contents
        for (i, b) in buf[MIN_FILE_SIZE..].iter_mut().enumerate() {
            *b = i as u8;
        }
        buf
    }

    #[test]
    fn parses_synthetic_minimal_blob() {
        let buf = synth_minimal();
        let dna = DnaFile::read(&buf).expect("synthetic blob should parse");

        assert_eq!(dna.header.magic, MAGIC);
        assert_eq!(dna.header.timestamp, 0xdead_beef);
        assert_eq!(dna.header.protos_name_id, 0x1111_1111);
        assert_eq!(dna.header.mesh_name_id, 0x2222_2222);
        assert_eq!(dna.header.num_variants, 7);

        // Pointer table: only slot 0 set.
        assert_eq!(dna.pointer_table[0], MIN_FILE_SIZE as u64);
        for s in &dna.pointer_table[1..] {
            assert_eq!(*s, 0);
        }

        // Exactly one segment.
        assert_eq!(dna.segments.len(), 1);
        let seg = dna.segments[0];
        assert_eq!(seg.slot_index, 0);
        assert_eq!(seg.slot_byte_offset, POINTER_TABLE_OFFSET);
        assert_eq!(seg.file_offset, MIN_FILE_SIZE);
        // Span runs to end of file (32 bytes).
        assert_eq!(seg.length, 32);

        let body = dna.segment_bytes(&seg);
        assert_eq!(body.len(), 32);
        assert_eq!(body[0], 0);
        assert_eq!(body[31], 31);
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut buf = synth_minimal();
        buf[0] = b'X'; // corrupt first magic byte
        let err = DnaFile::read(&buf).unwrap_err();
        match err {
            DnaError::InvalidMagic { expected, got } => {
                assert_eq!(expected, MAGIC);
                assert_ne!(got, MAGIC);
                assert_eq!(got[0], b'X');
            }
            other => panic!("expected InvalidMagic, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_variants() {
        let mut buf = synth_minimal();
        buf[0x2a..0x2c].copy_from_slice(&0u16.to_le_bytes());
        let err = DnaFile::read(&buf).unwrap_err();
        assert!(matches!(err, DnaError::ZeroVariants));
    }

    #[test]
    fn rejects_truncated_buffer() {
        let buf = vec![0u8; MIN_FILE_SIZE - 1];
        let err = DnaFile::read(&buf).unwrap_err();
        match err {
            DnaError::HeaderTruncated { need, have } => {
                assert_eq!(need, MIN_FILE_SIZE);
                assert_eq!(have, MIN_FILE_SIZE - 1);
            }
            other => panic!("expected HeaderTruncated, got {other:?}"),
        }
    }

    #[test]
    fn rejects_pointer_inside_header() {
        let mut buf = synth_minimal();
        // slot 1 -> 0x100 (inside the pointer table, < MIN_FILE_SIZE).
        buf[POINTER_TABLE_OFFSET + 8..POINTER_TABLE_OFFSET + 16]
            .copy_from_slice(&0x100_u64.to_le_bytes());
        let err = DnaFile::read(&buf).unwrap_err();
        match err {
            DnaError::PointerInsideHeader { slot, offset } => {
                assert_eq!(slot, 1);
                assert_eq!(offset, 0x100);
            }
            other => panic!("expected PointerInsideHeader, got {other:?}"),
        }
    }

    #[test]
    fn rejects_pointer_out_of_range() {
        let mut buf = synth_minimal();
        // slot 1 -> way past EOF.
        let bogus = (buf.len() as u64) + 1;
        buf[POINTER_TABLE_OFFSET + 8..POINTER_TABLE_OFFSET + 16]
            .copy_from_slice(&bogus.to_le_bytes());
        let err = DnaFile::read(&buf).unwrap_err();
        assert!(matches!(err, DnaError::PointerOutOfRange { slot: 1, .. }));
    }

    #[test]
    fn segment_lengths_sort_by_offset_not_slot_index() {
        // Set slot 5 -> offset_A, slot 1 -> offset_B with B > A.
        // Each segment's length must be (next_larger_offset - own_offset),
        // regardless of slot index.
        let mut buf = vec![0u8; MIN_FILE_SIZE + 64];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes());

        // slot 5 -> MIN_FILE_SIZE       (lower offset, lower span)
        // slot 1 -> MIN_FILE_SIZE + 16  (higher offset, runs to EOF)
        let lo = MIN_FILE_SIZE as u64;
        let hi = (MIN_FILE_SIZE + 16) as u64;
        let off_slot1 = POINTER_TABLE_OFFSET + 8;
        let off_slot5 = POINTER_TABLE_OFFSET + 5 * 8;
        buf[off_slot1..off_slot1 + 8].copy_from_slice(&hi.to_le_bytes());
        buf[off_slot5..off_slot5 + 8].copy_from_slice(&lo.to_le_bytes());

        let dna = DnaFile::read(&buf).expect("parse");
        assert_eq!(dna.segments.len(), 2);
        // segments are sorted by slot_index
        assert_eq!(dna.segments[0].slot_index, 1);
        assert_eq!(dna.segments[1].slot_index, 5);
        // slot 1's offset is higher, so its segment runs to EOF.
        assert_eq!(dna.segments[0].file_offset, hi as usize);
        assert_eq!(dna.segments[0].length, buf.len() - hi as usize);
        // slot 5's offset is lower; its segment ends at the start of slot 1.
        assert_eq!(dna.segments[1].file_offset, lo as usize);
        assert_eq!(dna.segments[1].length, (hi - lo) as usize);

        // segment_for_slot lookup + slot_bytes consistency.
        let seg5 = dna.segment_for_slot(5).expect("slot 5 present");
        assert_eq!(seg5.length, 16);
        let bytes5 = dna.slot_bytes(5).unwrap();
        assert_eq!(bytes5.len(), 16);
        assert!(dna.slot_bytes(0).is_none());
    }

    /// Synthetic test: when slot 31 is populated, the parser decodes
    /// `num_variants` variant-record headers at the right stride and
    /// preserves their fields verbatim.
    #[test]
    fn parses_synthetic_variant_array() {
        // Build a small file: header + table + 2 variant records (= 80 bytes).
        const NV: u16 = 2;
        let body_off = MIN_FILE_SIZE; // 0x140
        let body_len = NV as usize * VARIANT_RECORD_STRIDE;
        let mut buf = vec![0u8; body_off + body_len];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&NV.to_le_bytes());

        // Slot 31 → body_off.
        let slot31_off = POINTER_TABLE_OFFSET + VARIANT_ARRAY_SLOT * 8;
        buf[slot31_off..slot31_off + 8].copy_from_slice(&(body_off as u64).to_le_bytes());

        // Variant 0: (main=10, sec=20, pscale=0.5, sscale=2.0, ptr1=0xaa, ptr2=0xbb)
        let v0 = body_off;
        buf[v0..v0 + 4].copy_from_slice(&10u32.to_le_bytes());
        buf[v0 + 4..v0 + 8].copy_from_slice(&20u32.to_le_bytes());
        buf[v0 + 8..v0 + 16].copy_from_slice(&0.5f64.to_le_bytes());
        buf[v0 + 16..v0 + 24].copy_from_slice(&2.0f64.to_le_bytes());
        buf[v0 + 24..v0 + 32].copy_from_slice(&0xaau64.to_le_bytes());
        buf[v0 + 32..v0 + 40].copy_from_slice(&0xbbu64.to_le_bytes());

        // Variant 1: (main=7, sec=8, pscale=1.5, sscale=3.0, ptr1=0xcc, ptr2=0xdd)
        let v1 = body_off + VARIANT_RECORD_STRIDE;
        buf[v1..v1 + 4].copy_from_slice(&7u32.to_le_bytes());
        buf[v1 + 4..v1 + 8].copy_from_slice(&8u32.to_le_bytes());
        buf[v1 + 8..v1 + 16].copy_from_slice(&1.5f64.to_le_bytes());
        buf[v1 + 16..v1 + 24].copy_from_slice(&3.0f64.to_le_bytes());
        buf[v1 + 24..v1 + 32].copy_from_slice(&0xccu64.to_le_bytes());
        buf[v1 + 32..v1 + 40].copy_from_slice(&0xddu64.to_le_bytes());

        let dna = DnaFile::read(&buf).expect("synthetic variant-array blob should parse");
        assert_eq!(dna.variant_records.len(), 2);
        assert_eq!(dna.variant_records[0].num_main_delta_records, 10);
        assert_eq!(dna.variant_records[0].num_secondary_delta_records, 20);
        assert_eq!(dna.variant_records[0].primary_delta_scale, 0.5);
        assert_eq!(dna.variant_records[0].secondary_delta_scale, 2.0);
        assert_eq!(dna.variant_records[0].main_delta_entries_offset, 0xaa);
        assert_eq!(dna.variant_records[0].secondary_delta_entries_offset, 0xbb);
        assert_eq!(dna.variant_records[1].num_main_delta_records, 7);
        assert_eq!(dna.variant_records[1].num_secondary_delta_records, 8);
        assert_eq!(dna.variant_records[1].primary_delta_scale, 1.5);
        assert_eq!(dna.variant_records[1].secondary_delta_scale, 3.0);
        assert_eq!(dna.variant_records[1].main_delta_entries_offset, 0xcc);
        assert_eq!(dna.variant_records[1].secondary_delta_entries_offset, 0xdd);
    }

    /// Synthetic test: if slot 31's declared base + `num_variants * 0x28`
    /// extends past the buffer end, the parser surfaces
    /// [`DnaError::VariantArrayOverflow`].
    #[test]
    fn rejects_variant_array_overflow() {
        // Set num_variants very high so the strict size check fails
        // (file is only MIN_FILE_SIZE + 32 bytes, but we declare
        // 1000 variants × 40 bytes = 40,000 bytes needed).
        let mut buf = vec![0u8; MIN_FILE_SIZE + 32];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&1000u16.to_le_bytes());

        let slot31_off = POINTER_TABLE_OFFSET + VARIANT_ARRAY_SLOT * 8;
        buf[slot31_off..slot31_off + 8].copy_from_slice(&(MIN_FILE_SIZE as u64).to_le_bytes());

        let err = DnaFile::read(&buf).unwrap_err();
        match err {
            DnaError::VariantArrayOverflow { need, .. } => {
                assert_eq!(need, 1000 * VARIANT_RECORD_STRIDE);
            }
            other => panic!("expected VariantArrayOverflow, got {other:?}"),
        }
    }

    /// LOD1 fixture test (read from Data.p4k): the LOD1 fixture is `protos_human_male_face_t1_pu_head_lod1.dna`
    /// — the lowest-detail LOD, which has slot 31 (the per-variant
    /// delta-records array) **null**. The Phase 4a extension confirmed
    /// (via Ghidra + master `.dna` byte inspection) that LOD `.dna`
    /// files only carry a small subset of the master's pointer-table
    /// slots; this test pins that observation.
    ///
    /// Skipped without a Data.p4k.
    #[test]
    fn fixture_dna_parses() {
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_DNA_LOD1) else { return };
        let dna = DnaFile::read(&bytes).expect("LOD1 .dna should parse");

        // Header sanity.
        assert_eq!(dna.header.magic, MAGIC);
        assert!(
            dna.header.num_variants > 0,
            "num_variants must be > 0 to clear the runtime check"
        );

        // The LOD1 fixture is the
        // `protos_human_male_face_t1_pu_head_lod1.dna` file. Its known
        // counts (verified by byte inspection during the Phase 4a
        // anchoring pass):
        //   num_variants = 61
        //   protos_name_id = 3,714,869,238 (CRC32 of "protos_human_male_face_t1_pu")
        //   mesh_name_id   = 3,934,033,896 (CRC32 of "head")
        //   num_vertices   = 3,158       (decimated from master's 5,584)
        //   num_indices    = 17,724
        // We don't pin those exact values here (the test is intended to
        // pass on other .dna files too) but we sanity-check ranges.
        assert!(dna.header.num_variants < 1024, "implausibly many variants");
        assert!(dna.header.protos_name_id != 0);
        assert!(dna.header.mesh_name_id != 0);
        assert!(
            dna.header.num_vertices > 0,
            "num_vertices > 0 for any real LOD"
        );
        assert!(
            dna.header.num_indices > 0,
            "num_indices > 0 for any real LOD"
        );

        // Pointer table: every non-null slot must be inside the file
        // and inside the payload region (already enforced by `read`).
        for seg in &dna.segments {
            assert!(seg.file_offset >= MIN_FILE_SIZE);
            assert!(seg.file_offset + seg.length <= bytes.len());
        }

        // At least one non-null slot — the runtime always populates
        // *something* (protos name string, mesh name string, etc.).
        assert!(
            !dna.segments.is_empty(),
            "a real .dna file has at least one non-null pointer slot"
        );

        // The protos name and mesh name strings are stored as
        // C strings in two of the early pointer slots. Look for at
        // least one ASCII-text region in the parsed segments — a
        // weaker check than naming the exact slot, but valuable as
        // sanity.
        let mut found_ascii = false;
        for seg in &dna.segments {
            let body = dna.segment_bytes(seg);
            if body
                .iter()
                .take(64)
                .any(|&b| (b'A'..=b'z').contains(&b))
            {
                found_ascii = true;
                break;
            }
        }
        assert!(
            found_ascii,
            "expected at least one segment to contain printable ASCII (the protos/mesh name strings)"
        );

        // Slot 31 (per-variant delta-records array) is NULL in every
        // LOD `.dna` we have observed: only the master file carries it.
        // Pin that here so a regression surfaces immediately. The
        // structural parser therefore produces an empty
        // `variant_records` vector for LOD `.dna` files.
        assert_eq!(
            dna.pointer_table[VARIANT_ARRAY_SLOT], 0,
            "slot 31 is expected to be null in LOD .dna files (variant data lives in the master)"
        );
        assert!(
            dna.variant_records.is_empty(),
            "LOD .dna files do not populate the variant_records array"
        );
    }

    /// Master `.dna` fixture test (read from Data.p4k). The master file is the highest-detail LOD0 file
    /// (`protos_human_male_face_t1_pu_head.dna`, ~72 MB) and is the
    /// **only** LOD that carries slot 31 (the per-variant delta-records
    /// array). This test pins the variant-record layout decoded from
    /// it.
    ///
    /// Skipped without a Data.p4k.
    #[test]
    fn master_dna_parses() {
        let Some(bytes) = crate::test_p4k::read(crate::test_p4k::MALE_DNA) else { return };
        let dna = DnaFile::read(&bytes).expect("master .dna should parse");

        // Header sanity — should match the LOD1 file's identity fields
        // exactly (timestamp / protos / mesh CRCs are LOD-set invariant).
        assert_eq!(dna.header.magic, MAGIC);
        assert!(dna.header.num_variants > 0);
        assert!(
            dna.header.num_vertices > 0,
            "master must declare its LOD0 vertex count"
        );
        assert!(
            dna.header.num_indices > 0,
            "master must declare its LOD0 index count"
        );

        // The master file is the only LOD that populates slot 31
        // (the per-variant delta-records array). Cross-validate the
        // strict size check fires correctly: slot 31 is non-null AND
        // `variant_records.len() == num_variants`.
        assert_ne!(
            dna.pointer_table[VARIANT_ARRAY_SLOT], 0,
            "master .dna must populate slot 31 (per-variant delta records)"
        );
        assert_eq!(
            dna.variant_records.len(),
            dna.header.num_variants as usize,
            "variant_records length must match header.num_variants"
        );

        // Each variant record's delta-scale fields must be finite.
        // The `num_*_delta_records` fields can in principle be 0 for
        // an empty variant, so we only check finiteness on the scales
        // here.
        for (i, v) in dna.variant_records.iter().enumerate() {
            assert!(
                v.primary_delta_scale.is_finite(),
                "variant {i}: primary_delta_scale must be finite, got {}",
                v.primary_delta_scale
            );
            assert!(
                v.secondary_delta_scale.is_finite(),
                "variant {i}: secondary_delta_scale must be finite, got {}",
                v.secondary_delta_scale
            );

            // The two internal pointer fields are file-relative
            // offsets (the engine fixes them up on load). They must
            // each point inside the file when non-zero.
            let n = bytes.len() as u64;
            assert!(
                v.main_delta_entries_offset == 0 || v.main_delta_entries_offset < n,
                "variant {i}: main delta offset {:#x} out of file (size {n})",
                v.main_delta_entries_offset,
            );
            assert!(
                v.secondary_delta_entries_offset == 0 || v.secondary_delta_entries_offset < n,
                "variant {i}: secondary delta offset {:#x} out of file (size {n})",
                v.secondary_delta_entries_offset,
            );
        }
    }
}
