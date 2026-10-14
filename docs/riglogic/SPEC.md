# RigLogic / Protos File Format

CIG's facial DNA / blendshape file format used by Star Citizen's character
customizer. This document is the reverse-engineered structural spec, anchored
to the Windows client binary (`StarCitizen.exe`, build May 2026, decompiled
via Ghidra) and direct byte inspection of
`protos_human_male_face_t1_pu.RigLogic`.

Tagging convention used below:
- **[verified]** — Confirmed against the Ghidra decompile AND on-disk bytes.
- **[code]** — Confirmed in the decompile but offsets/sizes need byte verification.
- **[inferred]** — Best guess from naming / context. Verify during implementation.
- **[TBD]** — Unknown. Write the parser to log this and inspect the result.

## Scope

This spec covers the file produced by CIG's facial-rig DNA pipeline. The file:

- Defines a packed `RigType` classifier (class/gender/variant/part/tier/lod)
- Provides a content-fingerprint name string + identity CRC32s
- Carries 8 section-start byte offsets that delimit the major data regions
- Stores joint-control name pools and per-control CRCs
- Stores sparse, quantized per-joint delta records for the appearance
  blendshapes
- Stores secondary (corrective) delta records for pose-space deformations
- Provides the data the engine needs to evaluate a CHF DNA spec into final
  joint poses

The file is **not** related to 3Lateral/MetaHuman's DNA format despite the
`RigLogic` naming overlap. It is CIG-bespoke.

## File-level layout

### Standalone

Path pattern: `Data\Objects\Characters\RigLogic\<rig_name>.RigLogic`

Example: `protos_human_male_face_t1_pu.RigLogic` — **observed size 328,056 bytes**.

**There is NO size+pad wrapper on the standalone file.** Byte 0 of the
standalone file is the first byte of the magic `R` (`0x52`). An earlier
draft of this spec claimed an 8-byte `data_size + _pad` wrapper preceded the
magic — that draft was wrong. The wrapper only exists on the IVO-embedded
form (see below).

### Embedded in `.skin`

Character head `.skin` files are IVO containers. The `.skin` for a face rig
contains a `RigLogic` chunk holding the same data as the standalone file,
prefixed with an 8-byte `u64` `data_size`. The IVO chunk header for chunk type
`RigLogic` (`0x0A2485B6`) records the total chunk-payload size as
`data_size + 8`.

For `protos_human_male_face_t1_pu_head.skin`:

| Idx | Type                 | ChunkType   | Version | Size      |
|-----|----------------------|-------------|---------|-----------|
| 0   | `ExportFlags`        | `0xBE5E493E`| 2304    | 16        |
| 1   | `CompiledBonesIvo320`| `0xC2011111`| 2304    | 18,512    |
| 2   | `MtlNameIvo320`      | `0x83353333`| 2304    | 176       |
| 3   | `MeshIvo320`         | `0x92914444`| 2304    | 104       |
| 4   | **`RigLogic`**       | `0x0A2485B6`| 2304    | 328,000   |
| 5   | **`ProtosInfo`**     | `0xF7086666`| 2304    | 80        |

`328,000 = 8 (data_size header) + 327,992 (RigLogic content)`.

### The 64-byte difference — RESOLVED in Task 7

The standalone file is **328,056 bytes**. The `data_size` reported by the
embedded chunk is **327,992** — 64 bytes less. The embedded chunk's total
IVO payload is `8 + 327,992 = 328,000 bytes` (8-byte `data_size` prefix +
RigLogic content).

**Task 7 resolution:** the 64-byte difference is a **layout difference in
Sections E through H** — NOT a Section H truncation. Both the standalone
and the embedded form carry a complete Section H (152 bytes = 38 f32
weights). The embedded form's `section_starts[5..8]` are each exactly 64
bytes lower than the standalone's:

| Slot | Standalone | Embedded | Delta |
|------|-----------|---------|-------|
| 5    | 327,064   | 327,000 | −64   |
| 6    | 327,780   | 327,716 | −64   |
| 7    | 327,904   | 327,840 | −64   |

The 64-byte shift originates in Section E (or possibly between E and F). The
most likely explanation is that the embedded form's Section E is 64 bytes
shorter than the standalone's (e.g. the final corrective delta record has a
different payload length), but the difference has not been byte-walked to
find the exact location.

Both forms parse successfully via `RigLogic::read`. The embedded form should
be passed to the parser **without** any zero-padding — the 152-byte Section
H is already present at the correct file offset within the 327,992-byte
content.

**Section H byte layout** (byte-anchored in Task 4 against
`CFaceRigLogic::ExecuteGeneric`'s region-blending pass): Section H is a
flat sequence of `sum(blend_region_bones[i].weight_indices.len())` f32
values. The on-disk byte count is `(sum_of_per_region_counts) × 4`. For the
observed fixture: 38 × 4 = 152 bytes (both standalone and embedded).

## Header layout (anchored to `FaceRigData::Load`)

All offsets below are **file offsets in the standalone file** (= chunk-content
offsets — the embedded form shifts everything by 8 due to its `data_size`
prefix).

The Ghidra decompile of `FaceRigData::Load` at `0x14142d4d0` was the anchor.
The loader's validation path is:

1. Cast the input pointer (`param_2`) as `char*`; check 8 bytes against
   `R`, `I`, `G`, ` `, `V`, `1`, `.`, `9`. **[verified]**
2. Check that `*(short *)(param_2 + 0x3e) == 0xa8` (= 168). **[verified]**
3. Check that `(uintptr_t)param_2 & 7 == 0` (8-byte alignment). **[verified]**
4. `memcpy` the buffer.
5. For each `i in {0x68, 0x70, 0x78, 0x80, 0x88, 0x90, 0x98, 0xa0}` (eight
   offsets), if `*(longlong *)(_Dst + i) != 0`, add `_Dst` to it. This
   converts eight relative offsets into absolute pointers. **[verified]**

| Offset | Size | Field                  | Type | Notes                                       |
|--------|------|------------------------|------|---------------------------------------------|
| 0x00   | 8    | `magic`                | `[u8; 8]` | `b"RIG V1.9"`. **[verified]** (`FaceRigData::Load` `0x14142d4d0`, byte-by-byte char compare on `param_2[0..7]`.) |
| 0x08   | 40   | `name`                 | `[u8; 40]` | Rig name, null-padded ASCII. **[verified]** Doubles as content fingerprint — engine consumers read 8-byte and 4-byte slices of it as quick equality checks. |
| 0x30   | 8    | `rig_type`             | packed `RigType` | `class:u8, gender:u8, variant:u8, part:u8, tier:u8, lod:u8, _pad:u16`. **[verified]** Confirmed bytes `01 01 00 03 01 00 06 00`. The trailing `06 00` is `[inferred]` to be `_pad`/`flags_lo`; it's not used in the loader. |
| 0x38   | 4    | `identity_crc_lo`      | `NameHash` (`u32`) | `0xDD6C67F6` for this file. **[verified]** as `CRC32C(rig_name)` via Task 6's ProtosInfo cross-link: the `rig_name_crc` field of the sibling `ProtosInfo` chunk holds the same value, and it equals `NameHash::from_string("protos_human_male_face_t1_pu")`. Not read by the loader. |
| 0x3C   | 2    | `identity_crc_hi_or_x` | `u16` | `0x4562` for this file. **[inferred]** Could be the low 16 bits of a second identity CRC. |
| 0x3E   | 2    | `header_size`          | `u16` | **Must equal `0x00A8 = 168`.** **[verified]** Loader rejects the file with `"Wrong header size in RigLogic data"` otherwise. This is the byte size of everything up through the end of the section-start pointer table. |
| 0x40   | 40   | `header_scalars`       | typed inline scalars | Count fields plus two `f64` scales. **[verified]** in Task 2 against `CFaceRigLogic::InitFaceRigLogic` (`0x14671a270`), `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`), and `Protos::ComputeMixedDeltaMatrixNumEntries` (`0x14142a5c0`). See "Header scalars" below for the per-offset semantics. |
| 0x68   | 64   | `section_starts`       | `[u64; 8]` | 8 section-start byte-offsets (relative to file start) delimiting the major data regions. **[verified]** Each non-zero slot is fixed up by the loader into an absolute pointer. Each section ends at the next-larger value in the sorted table. |
| 0xA8   | …    | section data           | —    | The eight sections, addressed via the pointer table. See "Section table" below. |

### Header scalars (`0x40..0x67`)

The 40 bytes between the header-size short and the section-start pointer table
hold inline scalar metadata: four `u16` count fields, two `f64` scale factors,
and four `u32` count / parameter fields. Every offset below is **[verified]**
against a consumer function — see the "Ghidra anchoring evidence" subsection
below.

| Offset | Size | Field                          | Type | Observed | Notes |
|--------|------|--------------------------------|------|----------|-------|
| 0x40   | 2    | `num_appearance_blendshapes`   | u16  | 205      | **[verified]** Length of Section A (joint-control CRC table) and Section B (control name pool). |
| 0x42   | 2    | `num_corrective_blendshapes`   | u16  | 290      | **[verified]** Total = `num_appearance + num_corrective` is allocated as the per-blendshape weight slot count in `InitFaceRigLogic`. |
| 0x44   | 2    | `num_joints`                   | u16  | 179      | **[verified]** Length of Section F (joint-name CRC table); each joint has a 28-byte rest-pose transform applied during `ExecuteGeneric`. |
| 0x46   | 2    | `_reserved0`                   | u16  | 0        | Always zero in the fixture; no consumer-side access traced. Treated as padding. |
| 0x48   | 8    | `primary_delta_scale`          | f64  | ~0.0677  | **[verified]** Read as `*(double *)(buf + 0x48)` in `ExecuteGeneric` and divided by `(1 << packed_value_bits) - 1` to produce the per-step scale for primary (appearance) delta dequantization. |
| 0x50   | 8    | `secondary_delta_scale`        | f64  | ~1.617   | **[verified]** Read as `*(double *)(buf + 0x50)` in `ExecuteGeneric` and divided by the same denominator to produce the per-step scale for secondary (corrective) delta dequantization. |
| 0x58   | 4    | `num_main_delta_records`       | u32  | 517      | **[verified]** Loop count in `ExecuteGeneric` for the main (Section D) delta table. |
| 0x5C   | 4    | `num_secondary_delta_records`  | u32  | 528      | **[verified]** Loop count in `ExecuteGeneric` for the secondary (Section E) delta table. |
| 0x60   | 4    | `num_blend_regions`            | u32  | 24       | **[verified]** Loop count in `ExecuteGeneric`'s final region-blending pass; reads paired tables at section_starts slots 6 (offsets 0x98 / Section G) and 7 (0xA0 / Section H). |
| 0x64   | 4    | `packed_value_bit_width`       | u32  | 22       | **[verified]** Per-record dequantization parameter. Used in TWO distinct roles, both anchored in `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`): (1) it sets the **scale denominator** `((1 << bit_width) - 1) = 0x3FFFFF` that normalises the on-disk f64 scale (`scale_step = primary_delta_scale / 0x3FFFFF`); (2) it sets the **bit-split** in each packed u32 payload entry — `shift = (31 - bit_width) = 9`, `mask = (1 << shift) - 1 = 0x1FF` (low 9 bits = blendshape index, high 23 bits = signed delta coefficient, decoded via arithmetic shift `(i32) packed >> 9`). The original spec draft had the bit split reversed; the corrected layout is anchored by both the Ghidra decompile and a fixture walk that confirms `max(packed & 0x1FF) = 490 < num_appearance + num_corrective (495)`. |

#### Ghidra anchoring evidence

| Field                          | Anchored against                                                            | Access pattern (Ghidra-decompiled) |
|--------------------------------|-----------------------------------------------------------------------------|-------------------------------------|
| `num_appearance_blendshapes`   | `InitFaceRigLogic` `0x14671a270`, `ExecuteGeneric` `0x1466dec50`            | `*(ushort *)(buf + 0x40)` — passed as `param_2=count` to the joint-CRC hashmap builder `FUN_14668f6b0(target, count, *(longlong *)(buf + 0x70), ...)`, which iterates `count` × `u32` from Section A. Cross-verified by fixture parse: `CRC32("CTL_PSD") = 0xFD66F569 == Section_A[0]`, anchoring both the count (205 entries) and the **CRC32 (IEEE, NOT CRC32C)** algorithm choice. |
| `num_corrective_blendshapes`   | `InitFaceRigLogic` `0x14671a270`, `Protos::ComputeMixedDeltaMatrixNumEntries` `0x14142a5c0` | `*(ushort *)(buf + 0x42)` — summed with 0x40 (`uVar20 = (uint)*(ushort *)(buf + 0x42) + (uint)*(ushort *)(buf + 0x40)`) to allocate a 495-slot weight buffer. |
| `num_joints`                   | `InitFaceRigLogic` `0x14671a270`, `ExecuteGeneric` `0x1466dec50`            | `*(ushort *)(buf + 0x44)` — iteration count for the bone-CRC lookup loop reading 179 × `u32` from Section F (`*(longlong *)(buf + 0x90)`). Cross-validated: Section F size = 716 bytes = 179 × 4. Also drives the rotation loop with 28-byte stride (`(longlong)*psStack_188 * 0x1c`) confirming the joint rest-transform stride. |
| `primary_delta_scale`          | `ExecuteGeneric` `0x1466dec50`                                              | `fVar63 = (float)(*(double *)(buf + 0x48) / (double)uVar54)` where `uVar54 = (1 << bits_in_low) - 1`. Read as 8-byte `double`, not as paired `(u32, f32)`. |
| `secondary_delta_scale`        | `ExecuteGeneric` `0x1466dec50`                                              | `fVar65 = (float)(*(double *)(buf + 0x50) / (double)uVar54)`. Same pattern as the primary scale. |
| `num_main_delta_records`       | `ExecuteGeneric` `0x1466dec50`                                              | `while (uVar54 < *(uint *)(buf + 0x58))` — outer loop count for processing Section D delta records. |
| `num_secondary_delta_records`  | `ExecuteGeneric` `0x1466dec50`                                              | `while (uStackX_10 < *(uint *)(buf + 0x5c))` — outer loop count for processing Section E delta records. |
| `num_blend_regions`            | `ExecuteGeneric` `0x1466dec50`                                              | `while (uVar54 < *(uint *)(buf + 0x60))` — outer loop count for the per-region post-processing pass; reads region structure at `*(longlong *)(buf + 0x98)` (Section G) and weights at `*(longlong *)(buf + 0xa0)` (Section H). |
| `packed_value_bit_width`       | `ExecuteGeneric` `0x1466dec50`, `Protos::ComputeMixedDeltaMatrixNumEntries` `0x14142a5c0` | `bVar27 = (byte)*(undefined4 *)(buf + 0x64); uVar54 = (1 << bVar27) - 1; uVar47 = (1 << (0x1F - bVar27)) - 1;` — splits each delta-record `u32` into a `bVar27`-bit quantized value field (low bits) and a `(32 - bVar27)`-bit index field (high bits). |

#### Joint-naming distinction

Note the two distinct counts:

- `num_appearance_blendshapes = 205` — number of joint *control* names (Section A holds their CRCs, Section B their strings). These are blendshape identifiers, not skeleton bones.
- `num_joints = 179` — number of skeleton bones the rig writes to. Section F holds 179 × `u32` CRC32 (IEEE) values that are resolved against the companion `.skin`'s `CompiledBonesIvo320` chunk via `FUN_1465ef130(skeleton, crc32)` to produce per-bone indices. Bone names follow the `FACIAL_LOD{N}_{side}_{part}` convention (see "Section F CRC algorithm — confirmed" above).

`num_joints (179) ≠ num_appearance_blendshapes (205)` and neither equals the companion skin's full bone count (198) — see the "Joint-control names vs skeleton bones" section below.

#### `QuatT` element size

The joint rest transform is consumed in `ExecuteGeneric` at `lVar22 = (longlong)*psStack_188 * 0x1c` — a 28-byte stride per joint, accessed as `[f32; 7]` (four-float rotation + three-float translation). **`QuatT` is 28 bytes, no trailing padding.**

The rest transforms themselves are NOT stored in the `.RigLogic` file — they are read from the companion skeleton (`CCharInstance + 0x568` / `+ 0x548`). The RigLogic file contributes the bone CRCs (Section F); the engine resolves those into bone indices and applies deltas to the engine's own per-bone rest poses.

This means **Task 2's Step 6 (joint base poses + rest transforms) is not applicable to the V1.9 format.** The plan-stage assumption that these tables live in the file came from the V3.0 server format, which does carry them. V1.9 splits the responsibility: skeleton-side data on the skin, delta deltas on the RigLogic.

### Section table (`0x68..0xA7`)

The eight `u64` values are byte-offsets into the file (relative to the
standalone file's byte 0 / the embedded chunk-content's byte 0).

**Semantics (anchored in Task 2):** the loader's fixup at
`FaceRigData::Load` adds the buffer base to each non-zero slot, turning the
on-disk value into an absolute pointer. Consumer code then uses those
pointers as **starts of section data**. Each section ends at the
*next-larger* value in the sorted table. For example,
`InitFaceRigLogic`'s `FUN_14668f6b0(..., count=205, ptr=*(longlong *)(buf + 0x70), ...)` reads 205 × `u32` *from* the address stored at slot 1
(= 168 = `0xA8`) — that's the bootstrap value, which is the smallest in the
table, and is the start of Section A. The parser field is named
`section_starts` accordingly.

For `protos_human_male_face_t1_pu.RigLogic`, in sorted-by-value order:

| Slot | Value    | Section | Length      | Content                                               |
|------|----------|---------|-------------|-------------------------------------------------------|
| 1    | 168      | A       | 820 B       | 205 × `u32` joint-control CRC32 table (IEEE poly `0xEDB88320`, **NOT** CRC32C). **[verified]** |
| 0    | 988      | B       | 5,848 B     | Null-separated joint-control name string pool. **[verified]** |
| 2    | 6,836    | C       | 1,456 B     | Corrective-blendshape composition table: **290 × 5-byte rows + 6 bytes zero padding**. Each row's five `u8` slots index into the appearance-weight buffer; the engine multiplies the referenced weights to produce one corrective weight per row. **[verified]** in Task 4. |
| 3    | 8,292    | D       | 157,676 B   | Main (appearance) delta table — 517 records of variable size. **[verified]** |
| 4    | 165,968  | E       | 161,096 B   | Secondary (corrective) delta table — 528 records of variable size. **[verified]** |
| 5    | 327,064  | F       | 716 B       | 179 × `u32` skeleton-joint CRC32 table. **CRC32 (IEEE, confirmed)** — Task 7 cross-verified by extracting `FACIAL_LOD*` bone names from the companion `.skin` and confirming all 179 entries match CRC32-IEEE hashes. **[verified]** |
| 6    | 327,780  | G       | 124 B       | Per-blend-region bone-index list table: 24 regions, each `(u16 count, u16 weight_indices[count])`. Total = `2 * (num_blend_regions + sum_counts) = 2 * (24 + 38) = 124` bytes. **[verified]** in Task 4. |
| 7    | 327,904  | H       | 152 B       | **38 × `f32` region weights** = `sum(blend_region_bones[i].weight_indices.len()) × 4`. Both standalone and embedded forms carry the full 152 bytes. The embedded form's Section H starts at offset 327,840 (standalone: 327,904) due to a 64-byte layout difference in Section E — see "The 64-byte difference" above. **[verified]** Task 7. |

### Section F CRC algorithm — confirmed CRC32 (IEEE) in Task 7

Section A's CRC algorithm was directly byte-verified: hashing
`"CTL_PSD"` with CRC32 (IEEE, polynomial `0xEDB88320`) yields `0xFD66F569`,
matching `joint_control_name_crcs[0]`. Section F's algorithm is now also
**confirmed CRC32 (IEEE)**: Task 7 extracted bone name strings from the
companion `.skin` file (`protos_human_male_face_t1_pu_head.skin`) and
verified that all 179 Section F entries match CRC32-IEEE hashes of `FACIAL_LOD*`
joint names.

Confirmed examples (fixture `protos_human_male_face_t1_pu.RigLogic`):

| Bone name                    | CRC32-IEEE   | Section F slot |
|------------------------------|--------------|----------------|
| `FACIAL_LOD0_L_Neck2`        | `0x40D44915` | 0              |
| `FACIAL_LOD0_R_Neck2`        | `0x7735AE86` | 1              |
| `FACIAL_LOD1_C_AdamsApple`   | `0xB010F8BF` | 2              |
| `FACIAL_LOD0_R_Neck1`        | `0xEE3CFF3C` | 4              |
| `FACIAL_LOD0_C_Skull`        | `0x150A280A` | 9              |

Note: bone names use the `FACIAL_LOD{N}_{side}_{part}` convention — NOT
simple anatomical names like `"head"` or `"jaw"`. The companion `.skin`'s
`CompiledBonesIvo320` chunk carries the full list.

The engine treats Section F entries as opaque hash keys, looking them up
against a pre-computed skeleton bone-name → bone-index hash table (built in
`CCharInstance` initialisation, anchored at `+0x548` / `+0x568` in
`FUN_1465ef130` at `0x1465ef130`).

### Section C corrective-composition loop discrepancy

`CFaceRigLogic::ExecuteGeneric`'s corrective-composition loop reads 5 bytes
of Section C per iteration and runs while `cursor < num_corrective_blendshapes`
— in the fixture, that's 290 / 5 = **58 iterations**, processing only the
first 290 bytes of Section C. However, Section C's on-disk size is 1,456
bytes = 290 × 5 + 6 padding bytes, suggesting **all 290 rows are valid
data** (not just 58). The discrepancy between the loop-trip count and the
on-disk row count is unresolved; possibilities include:

- The loop only fills 58 of 290 corrective weight slots, leaving slots
  `[num_appearance+58 .. num_appearance+num_corrective-1]` zero-initialised.
- Other consumers (not yet identified) read additional rows from Section C.
- The loop's iteration count is miscounted by Ghidra's decompile.

The parser stores all 290 rows verbatim. Downstream evaluators should be
aware that the engine may read only the first `floor(num_corrective / 5)`
rows in this code path.

`section_starts` (the parser field) is **not** monotonic by slot index — the
slots are slot-indexed (the meaning of slot 1 is "Section A start"; slot 0
is "Section B start"; etc.), and the **values** are not in ascending order.

The Task 1 parser stores the raw `[u64; 8]` and validates that:
- Slot 1 equals `0xA8` (the engine's invariant — every well-formed file).
- Every non-zero slot points within the file (`<= file_size`).

Per-section length checks (e.g. Section A length = `num_appearance_blendshapes × 4`)
are enforced in Task 2's section-aware readers.

## Sections D & E — variable-stride delta tables

**Anchored against `CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`)** in
Task 3. Sections D (main / appearance) and E (secondary / corrective) share
the **same on-disk format** — they differ only in which f64 scale applies
(`primary_delta_scale` at file offset `0x48` for D, `secondary_delta_scale`
at `0x50` for E), which `num_*_delta_records` count terminates the walk
(`num_main_delta_records` at `0x58` for D, `num_secondary_delta_records` at
`0x5C` for E), and which engine-side translation accumulator buffer is
written to (the main delta buffer for D, the corrective delta buffer for E).

The on-disk format is a **flat sequence of variable-length records** — there
is **NO per-blendshape "runs" / "starts" header table** indirecting the
records, despite an earlier plan-stage hypothesis. The engine walks each
section linearly:

```text
loop num_*_delta_records times:
  u16  output_idx       // 0..num_joints*3 — flat index into the per-joint
                        // (x, y, z) translation accumulator
  u16  payload_len (N)  // number of u32 packed payload entries
  u32  payload[N]       // each u32 packs:
                        //   low 9 bits  = blendshape weight index
                        //                  (0..num_appearance+num_corrective-1)
                        //   high 23 bits = signed delta coefficient
                        //                  (decoded via arithmetic shift)
```

The walk advances by exactly `4 + 4*N` bytes per record. Both sections in
the observed fixture consume their declared span exactly — no leftover
bytes, no truncated record (anchored against the fixture: 517 records ⇒ D
spans 8292..165968 = 157,676 bytes; 528 records ⇒ E spans 165968..327064 =
161,096 bytes).

### Per-payload u32 packing (`packed_value_bit_width = 22`)

```text
shift = 31 - packed_value_bit_width             // = 9
mask  = (1 << shift) - 1                        // = 0x1FF
blendshape_idx = (packed_u32 & mask)            // low 9 bits
coef           = ((i32) packed_u32) >> shift    // arithmetic shift; signed
                                                // (32 - shift) = 23-bit value
```

The engine's full per-record contribution to the output buffer is:

```text
let step = scale / ((1 << packed_value_bit_width) - 1);   // = scale / 0x3FFFFF
let mut acc: f32 = 0.0;
for entry in payload:
    let (idx, coef) = decode_packed(entry);
    acc += (coef as f32) * weights[idx];
output[output_idx] += acc * step;
```

`weights` is the shared `(num_appearance + num_corrective)`-slot blendshape
weight buffer — anchored against
`uStack_190 = (uint)*(ushort *)(buf + 0x42) + (uint)*(ushort *)(buf + 0x40)`
in `ExecuteGeneric`'s prologue.

### Validation invariants

The Task 3 reader (`read_delta_table` in `crates/starbreaker-3d/src/riglogic.rs`)
enforces:

1. The section span equals `next_section_start - section_start` and is
   in-bounds.
2. Walking exactly `num_*_delta_records` records consumes the full section
   span — no trailing slack bytes, no truncated final record.
3. Every record's `output_idx < num_joints * 3` (the per-joint translation
   accumulator length, anchored against
   `memset(pvVar24, 0, *(ushort *)(buf + 0x44) * 0xc)`).
4. Every payload u32's decoded `blendshape_idx < num_appearance +
   num_corrective` (the shared weight-buffer length).
5. When `expected_record_count == 0`, the section span must be 0 bytes —
   non-empty span with zero records is a contract violation.

The bit-split's correctness was double-anchored by (a) the Ghidra
decompile of `ExecuteGeneric`'s `*(float *)((longlong)pvStack_1c0 +
(ulonglong)(*puVar31 & uVar47) * 4)` access pattern — where `uVar47 =
(1 << 9) - 1` is the mask — and (b) a Python walk of the real fixture
that confirmed `max(packed & 0x1FF) = 490 < num_appearance + num_corrective
= 495`. A swap of the two halves would put `max_index ≈ 4.2M` against a
495-slot buffer, immediately UB at runtime.

### Why there's no per-blendshape header

`Protos::ComputeMixedDeltaMatrixNumEntries` (`0x14142a5c0`) DOES contain a
table of `int*` offsets that map `(joint, axis) → u16-position-in-D`,
indexed by a `lVar5 + (uVar15 * stride + iVar17) * 4` lookup. That table
lives elsewhere in memory (resolved from a `m_*` field by the engine — NOT
in this `.RigLogic` file's eight on-disk sections). The same function still
walks each record by reading `*(ushort *)(record_base + 2 + iVar8 * 2)` to
get N and then `record_base + 4 + iVar8 * 2` for the payload — confirming
the same record layout this spec describes.

The `ExecuteGeneric` consumer path does NOT use the external lookup table;
it walks D and E linearly, record by record. The Task 3 parser matches this
linear walk and does not synthesise the lookup table.

## Section content (forward references)

The eight sections, in sorted-by-start order. Sizes are anchored against
Task 2's count fields where applicable:

Standalone (`protos_human_male_face_t1_pu.RigLogic`, 328,056 bytes):

```
Section A  0x0A8  – 0x3DC      820 B    Joint-control CRC32 table (205 × u32)            [verified]
Section B  0x3DC  – 0x1AB4   5,848 B    Joint-control name string pool (null-separated)  [verified]
Section C  0x1AB4 – 0x2064   1,456 B    Corrective composition (290 × 5-byte rows + 6 pad) [verified]
Section D  0x2064 – 0x28850 157,676 B   Main delta table (517 appearance records)        [verified]
Section E  0x28850 – 0x4FD98 161,096 B  Secondary delta table (528 corrective records)   [verified]
Section F  0x4FD98 – 0x50064    716 B   Skeleton-bone CRC32 (IEEE) table (179 × u32)     [verified]
Section G  0x50064 – 0x500E0    124 B   Per-region bone-index lists (24 regions, flat u16) [verified]
Section H  0x500E0 – 0x50178    152 B   Per-region weight scalars (38 × f32)             [verified]
```

Embedded form (IVO chunk content, 327,992 bytes): Sections A–E are
identical except that Section E ends 64 bytes earlier. Sections F, G, H
carry the same data but start 64 bytes earlier:

```
Section F  0x4FF58 – 0x50024    716 B   (same data, offset 64 bytes lower)
Section G  0x50024 – 0x500A0    124 B
Section H  0x500A0 – 0x50138    152 B   (same 38 × f32 weights, complete)
```

Both forms parse successfully via `RigLogic::read` without modification.
See "The 64-byte difference — RESOLVED in Task 7" above.

Sections A, B, C, D, E, F, G, and H all have **[verified]** byte-level
layouts as of Task 4. Section C / F / G / H were byte-anchored against
`CFaceRigLogic::ExecuteGeneric` (`0x1466dec50`) and
`CFaceRigLogic::InitFaceRigLogic` (`0x14671a270`) in Task 4. Section F's CRC
**algorithm** (IEEE) is assumed-by-analogy with Section A; see "Section F
CRC algorithm" note above.

## Pointer-fixup walk vs section-start table

An older draft of this spec described a 31-pointer fixup table where each
pointer is one of the named `Protos::m_*` fields (`m_protosName`,
`m_variantNames`, `m_appearanceBlendShapes`, ...). That table comes from the
**Linux server build's `cig-scgs` decompile** (which targets the
`"DNA V3.0"` magic) and does NOT match the V1.9 on-disk format. The V1.9
binary's `FaceRigData::Load` only fixes up **eight** offsets — the section-start
table at `0x68..0xA7`. The 31 logical fields the engine sees are computed by
higher-level `Protos::*` accessors from those eight section boundaries plus
the section content. **The on-disk format is the eight-section model
described above; the 31-pointer view is a runtime abstraction.**

## Joint-control names vs skeleton bones — the 205 vs 198 distinction

Verified from the companion `.skin`'s `CompiledBonesIvo320` chunk:
- Skeleton has **198 bones** (u32 count at chunk offset 0).
- RigLogic claims **205 controls** (the `0xCD` count at file offset 0x40 of
  the standalone file).

The 7-control discrepancy: not all entries in RigLogic's control list are
actual bones. **Meta-controls** (`CTL_expressions_*`, `CTL_PSD`) are
high-level rig *inputs* that synthesize bone movements — e.g.,
`CTL_expressions_blinkR`, `CTL_expressions_eyeLookDownR`, `CTL_PSD`
(pose-space deformation trigger). They don't correspond to individual skeleton
bones.

**Note on the V1.9 delta-record model (Task 3-anchored):** the on-disk
records in Sections D and E do NOT carry a per-record `joint_idx` field, and
the records are NOT grouped by blendshape. Each record has an
`output_idx: u16` (= `joint_idx * 3 + axis_idx`, indexing a flat
`num_joints * 3` translation accumulator) and a payload of packed u32s; each
payload u32 carries a 9-bit `blendshape_idx` into the shared
`num_appearance + num_corrective`-slot weight buffer. The dequantized output
is `output[output_idx] += scale_step * sum_i (coef_i * weights[blendshape_idx_i])`.
See "Sections D & E — variable-stride delta tables" above for the full
on-disk layout and Ghidra anchoring.

So in the runtime model (NOT the on-disk model):
- **205 = number of blendshape control names** (one per Section B entry,
  one per Section A CRC32, indexed by the engine via name → control-slot
  resolution). 
- **179 = number of skeleton joints addressed by the rig** (drives Section
  F's CRC32 table and the per-joint 28-byte rest transforms on the
  companion skeleton).
- **198 ≠ 179**: the companion `.skin`'s `CompiledBonesIvo320` chunk
  carries 198 bones; the rig only writes to a 179-bone subset.
- **Meta-controls** (`CTL_expressions_*`, `CTL_PSD`) are high-level rig
  *inputs* that synthesize bone movements — e.g., `CTL_expressions_blinkR`,
  `CTL_PSD` (pose-space deformation trigger). They don't correspond to
  individual skeleton bones and live in the same 205-slot control list as
  the per-bone controls.

## Worked example: first 0xB0 bytes

Bytes of `protos_human_male_face_t1_pu.RigLogic` (standalone), offset 0:

```
0x00: 52 49 47 20 56 31 2e 39 | 70 72 6f 74 6f 73 5f 68
0x10: 75 6d 61 6e 5f 6d 61 6c | 65 5f 66 61 63 65 5f 74
0x20: 31 5f 70 75 00 00 00 00 | 00 00 00 00 00 00 00 00
0x30: 01 01 00 03 01 00 06 00 | f6 67 6c dd 62 45 a8 00
0x40: cd 00 22 01 b3 00 00 00 | 00 00 00 c0 3b 53 b1 3f
0x50: 00 00 00 60 fe de f9 3f | 05 02 00 00 10 02 00 00
0x60: 18 00 00 00 16 00 00 00 | dc 03 00 00 00 00 00 00
0x70: a8 00 00 00 00 00 00 00 | b4 1a 00 00 00 00 00 00
0x80: 64 20 00 00 00 00 00 00 | 50 88 02 00 00 00 00 00
0x90: 98 fd 04 00 00 00 00 00 | 64 00 05 00 00 00 00 00
0xA0: e0 00 05 00 00 00 00 00 | ...
```

Interpretation:

- `0x00-0x07`: magic = `"RIG V1.9"`. **[verified]**
- `0x08-0x2F`: name = `"protos_human_male_face_t1_pu"` null-padded to 40 bytes. **[verified]**
- `0x30-0x37`: `RigType`. `class=1, gender=1, variant=0, part=3, tier=1, lod=0`. **[verified]** RigType byte pattern; the trailing `06 00` is `_pad`/`flags_lo` (`[inferred]`).
- `0x38-0x3B`: u32 = `0xDD6C67F6`. **[inferred]** Identity CRC32.
- `0x3C-0x3D`: u16 = `0x4562`. **[inferred]** Second identity field.
- `0x3E-0x3F`: u16 = `0x00A8 = 168`. **[verified]** Header size validation check.
- `0x40-0x67`: 40 bytes of inline scalars — four `u16` counts (205, 290, 179, 0), two `f64` delta scales, four `u32` count/parameter fields (517, 528, 24, 22). **[verified]** in Task 2 — see "Header scalars" table.
- `0x68-0xA7`: 8 × u64 section-start byte offsets (`section_starts` field). **[verified]**
- `0xA8+`: Section content (see "Section content" forward reference).

## Variant metadata, blendshape evaluation, etc.

**Sections A through H** are all byte-anchored as of Task 4 (see the section
table above). The remaining forward-referenced topics are:

- **Blendshape evaluator** (Task 5): combines Section D + E + C +
  `(num_appearance + num_corrective)`-slot weight buffer into per-joint
  translation deltas. The evaluator is anchored against
  `CFaceRigLogic::ExecuteGeneric`'s main and secondary delta loops, plus the
  region-blending pass that reads Section G + H. Not yet implemented.
- **`ProtosInfo` chunk** (Task 6 ✅): the sibling IVO chunk type `0xF7086666`
  (80 bytes). Fully decoded — see layout table below.
- **Variant metadata**: see "Known gaps" — out-of-file, in `.dna` files.

## ProtosInfo chunk layout

IVO chunk type `0xF7086666`, version 2304 (`0x900`). Payload is exactly 80
bytes. All integers little-endian.

Byte-anchored from fixture
`protos_human_male_face_t1_pu_head.skin` (chunk index 5). Both CRC fields use
CRC32C (Castagnoli), consistent with the `NameHash` convention used throughout
StarBreaker.

| Offset | Size | Type       | Field           | Value (fixture)                              |
|--------|------|------------|-----------------|----------------------------------------------|
| 0x00   |    4 | `u32`      | `data_size`     | 76 (= total − 4; invariant)                  |
| 0x04   |    4 | `u32`      | `version`       | 1                                            |
| 0x08   |   16 | `[u8; 16]` | `mesh_name`     | `"head\0..."` (null-padded ASCII)            |
| 0x18   |    4 | `u32`      | `mesh_name_crc` | `0xEA7C97E8` = CRC32C("head")                |
| 0x1C   |   40 | `[u8; 40]` | `rig_name`      | `"protos_human_male_face_t1_pu\0..."` (null-padded) |
| 0x44   |    8 | `[u8; 8]`  | _(padding)_     | all-zero                                     |
| 0x4C   |    4 | `u32`      | `rig_name_crc`  | `0xDD6C67F6` = CRC32C(rig_name) = `RigLogic.identity_crc_lo` |

**Cross-validation:** `rig_name_crc` equals the `identity_crc_lo` field of the
paired `RigLogic` chunk (verified: both are `0xDD6C67F6` for this fixture).
`mesh_name` matches the skin attachment point name (e.g. `"head"`, `"mouth"`,
`"eyes"`).

**Evidence:** byte-inspection only (Ghidra MCP offline during Task 6). The CRC
type (CRC32C, not CRC32-IEEE) was confirmed by computing `NameHash::from_string`
for both name strings and matching the stored values exactly.

## Head `.skin` chunk dispatcher

The full head `.skin` IVO chunk dispatcher is `FUN_1405c9600` in
`StarCitizen.exe` (= `CSkin::ParseChunks` / equivalent). It walks the file's
chunk table and routes each entry into a per-slot pointer on the receiving
`CSkin`-like struct. Every chunk type whose pointer fields it knows about,
plus the RTTI type-name pool that anchors each branch:

| Chunk type   | Slot       | RTTI type name                       | Role                              |
|--------------|------------|--------------------------------------|-----------------------------------|
| `0xBE5E493E` | `[0]`      | `CHUNKHEADER_0x900R_RC_VERSION`      | Export flags / version metadata   |
| `0xC2011111` | `[1]`      | `CHUNKHEADER_0x900R_SKELETON`        | `CompiledBonesIvo320` — bones     |
| `0x90C62222` | `[2]`      | `CHUNKHEADER_0x900R_PHYSICS`         | Physics proxy                     |
| `0x83353333` | `[3]`      | `CHUNKHEADER_0x900R_MATERIALINFO`    | `MtlNameIvo320` — material name   |
| `0xA7169999` | `[4]`      | `CHUNKHEADER_0x900R_RBF`             | Per-rig RBF blob (out-of-scope)   |
| `0x92914444` | `[5]`      | `CHUNKHEADER_0x900R_MESHINFO`        | `MeshIvo320` — mesh metadata only |
| `0x0A2485B6` | `[6]`      | `CHUNKHEADER_0x900R_RIGLOGIC`        | `RigLogic` chunk                  |
| `0x7E035555` | `[7]`      | `CHUNKHEADER_0x900R_RIGINFO`         | Per-rig info                      |
| `0xF7086666` | `[8]`      | `CHUNKHEADER_0x900R_PROTOSINFO`      | `ProtosInfo` chunk                |
| `0x2B8A2C29` | `[9]`      | `CHUNKHEADER_0x900R_SIMTOPOLOGY`     | Cloth/sim topology                |

**The head `.skin` carries no vertex / index / bone-weight buffers.** The
dispatcher recognises 10 chunk types; none of them are a vertex stream. The
`MeshIvo320` chunk is 104 bytes — only counts + bounding box (see below).
For Phase 3 this means the head mesh geometry must be sourced from outside
the `.skin` file (a companion mesh file, or via the larger CharCustomizer
pipeline). The `.skin` itself is rig-and-binding-metadata only.

## CompiledBonesIvo320 layout

IVO chunk type `0xC2011111`, version 2304 (`0x900`). For the fixture
`protos_human_male_face_t1_pu_head.skin` the chunk is 18,512 bytes carrying
198 bones plus their string-pool of names.

```text
+0x0000  u32                bone_count            (= 198)
+0x0004  [BoneRecord; 198]  bone_records          (68 × 198 = 13,464 bytes)
+0x349C  [u8; ...]          string_table          (null-terminated bone names + trailing material name)
```

Byte boundary anchored: bone records end at offset `4 + 198 * 68 = 0x349C`,
and the very first byte at `+0x349C` is `'W'` — the start of `"World\0"`.

### BoneRecord (68 bytes, little-endian)

| Offset | Size | Type        | Field               | Notes                                                  |
|--------|------|-------------|---------------------|--------------------------------------------------------|
| 0x00   |   4  | `u32`       | `name_crc`          | **CRC32 IEEE** of `name` (matches `crc32fast::hash`)   |
| 0x04   |   4  | `u32`       | `name_crc_lower`    | **CRC32 IEEE** of `name.to_ascii_lowercase()`          |
| 0x08   |   4  | `i32`       | `parent_index`      | `-1` for root; otherwise an index `< i` (parent-first) |
| 0x0C   |  16  | `[f32; 4]`  | `local_rotation`    | Quaternion `(x, y, z, w)`, parent-relative             |
| 0x1C   |  12  | `[f32; 3]`  | `local_translation` | Parent-relative                                        |
| 0x28   |  16  | `[f32; 4]`  | `world_rotation`    | Quaternion `(x, y, z, w)`, world bind pose             |
| 0x38   |  12  | `[f32; 3]`  | `world_translation` | World bind pose (CryEngine Z-up, metres)               |

**CRC algorithm — verified.** Bone CRCs use CRC32-IEEE (NOT CRC32C). All
198 bone CRCs in the fixture were reconstructed by computing
`crc32fast::hash(name.as_bytes())` against the decoded string-table entries
and matching exactly. This is the same algorithm RigLogic Section F uses
for joint-name lookup.

**Bone CRC fields — the two roles confirmed by the fixture:**

- `name_crc` matches the bone's name as written (`World` → `0xFBB63E47`,
  `Spine3` → `0x61379B62`, `LeftShoulder` → `0x2D4660A8`, …).
- `name_crc_lower` matches the lowercase form (`world` → `0x3A771143`,
  `spine3` → `0x669B9E54`, `leftshoulder` → `0xCC01D906`, `head` →
  `0xA7F3F69C`). All 198 of these are checked in
  `decode_bones_chunk`'s test guard.

**Parent ordering — verified.** All 198 bones satisfy `parent < i` (or
`parent == -1`). There is exactly one root: bone 0, `"World"`. Implementers
may rely on parent-first traversal without a topological sort.

**Local vs. world transforms.** Bone records carry both transforms
redundantly. For non-trivial bones (e.g. `Head`), the local quaternion is a
small rotation relative to the parent (`Head` local = `(0, 0, -0.017, 0.9998)`,
i.e. ~2° about Z), and the world quaternion is the cumulative pose
(`Head` world = `(0, -0.707, 0, 0.707)`, the −90°-about-Y standard head
pose). For the root and for "zeroed" intermediate bones, both transforms
are identical. The redundancy is the engine's, not the format's — keeping
both avoids recomputing the world pose at draw time.

### String table

A flat sequence of `bone_count + 1` null-terminated ASCII names. Names
appear in the **same order as the bone records** (verified — `names[i]`'s
CRC32-IEEE matches `bone_records[i].name_crc` for all 198 bones). One
trailing name follows the per-bone names: the material name (e.g.
`protos_human_male_face_t1_pu_head_material`) that ties this chunk back to
the sibling `MtlNameIvo320` chunk. The parser ignores everything past the
last bone name.

**Evidence:** byte-anchored against the fixture. Ghidra confirms the
SKELETON chunk role for type `0xC2011111` via the RTTI type-name string at
`0x1498eafc8`. The per-record stride was derived empirically from the
chunk size (`18,508 / 198 != integer` ruled out simple flat layouts; the
string table at the end resolved the residual).

## `.skinm` companion — vertex geometry (Phase 3b)

The head `.skin` carries NO vertex geometry (see `MeshIvo320` below). The
companion `.skinm` file (`protos_human_male_face_t1_pu_head.skinm`, 1,363,368
bytes) contains the actual mesh data and is required to populate
[`HeadMesh`]'s geometry buffers.

### Chunk layout of the head `.skinm`

| Idx | Chunk type   | Name         | Version | Size        | Role                          |
|-----|--------------|--------------|---------|-------------|-------------------------------|
| 0   | `0xB8757777` | `IVO_SKIN2`  | 2304    | 399,664 B   | Vertex geometry + bone weights |
| 1   | `0x57A38888` | `BSHAPES_GPU`| 2304    | 963,656 B   | GPU blend-shape data (not used here) |

The `IVO_SKIN2` chunk is parsed by the existing
`crates/starbreaker-3d/src/ivo/skin.rs::SkinMesh::read`. No new chunk
decoder was needed — the vertex-stream decoder was already correct for this
format.

### `IVO_SKIN2` stream layout (5,584 vertices, 32,178 indices)

Streams appear in this order within the `IVO_SKIN2` chunk payload:

| Stream tag   | Name          | Elem size | Count  | Role                              |
|--------------|---------------|-----------|--------|-----------------------------------|
| `0xEECDC168` | `IVOINDICES`  | 2         | 32,178 | u16 triangle indices              |
| `0x91329AE9` | `IVOVERTSUVS` | 20        | 5,584  | f32×3 positions + BGRA + UV half×2 |
| `0xB95E9A1B` | `IVOTANGENTS` | 8         | 5,584  | i16 SNorm quaternion (normal + tangent) |
| `0x9CF3F615` | `IVONORMALS`  | 4         | 5,584  | Packed 15-15-1-1 unit vector (backup normals) |
| `0xD9EED421` | `IVOCOLORS2`  | 4         | 5,584  | Vertex colors (BGRA u8)           |
| `0x6ECA3708` | `IVOBONEMAP32`| 24        | 5,584  | 8×u16 joint indices + 8×u8 weights |

**Key observations:**

- Positions are **f32 float** (elem_size=20), not quantized i16. No
  dequantization is needed; the `min_bound`/`max_bound` bbox from the
  `IVO_SKIN2` `MeshInfo` header matches the `.skin`'s `MeshIvo320` bbox.
- The `IVOBONEMAP32` stream (elem_size=24) uses an **extended 8-influence
  format** — unlike ship meshes which use `IVOBONEMAP` (elem_size=12,
  4 influences). This is decoded in Phase 3b by the new `BoneMap32` type
  and `RawBoneMap24` raw layout in `crates/starbreaker-3d/src/ivo/skin.rs`.
  **The head SUB-meshes (eyes/teeth/eye-details) instead use the 4-influence
  `IVOBONEMAP` (elem_size=12)**; `populate_mesh_geometry` reads whichever is
  present (Pass 19k — sub-mesh rig skinning).
- The top-4 influences from `IVOBONEMAP32` are kept in `HeadMesh::joints` /
  `HeadMesh::weights` (sufficient for glTF `JOINTS_0` / `WEIGHTS_0`).
  Weights are renormalized to sum to 1.0 after truncation.
- No axis conversion is applied in Phase 3b. All data is in CryEngine Z-up
  space; Phase 6 glTF emit handles the Y-up swap.

### `IVOBONEMAP32` on-disk layout (24 bytes / vertex)

```text
+0x00  u16×8  joint_indices   // 8 bone indices (0-based into CompiledBonesIvo320)
+0x10  u8×8   weights         // 8 weights (0..=255 → 0.0..=1.0); sum ≈ 255
```

Confirmed from the fixture: the first 3 vertices' weight sums are all
255 ± 1 (uint) / 1.0 ± 0.01 (f32), passing the renormalization check.
The joint indices are in range `[0, 197]` (within the 198-bone skeleton).

---

## MeshIvo320 layout

IVO chunk type `0x92914444`, version 2304 (`0x900`). For the head/teeth/eyes
the chunk is 104 bytes — counts + bounding box only. The **eye-details**
sub-mesh carries a **120-byte variant** with the same counts+bbox field
offsets (0x04/0x08/0x0C, 0x18, 0x24) plus 16 trailing bytes (an extra
scale/range block we don't read), so `decode_mesh_chunk` requires `len >= 104`
rather than an exact size (Pass 19k).

| Offset | Size | Type        | Field           | Value (fixture)                              |
|--------|------|-------------|-----------------|----------------------------------------------|
| 0x00   |   4  | `u32`       | `flags2`        | `0`                                          |
| 0x04   |   4  | `u32`       | `num_vertices`  | `5584`                                       |
| 0x08   |   4  | `u32`       | `num_indices`   | `32178` (= 10,726 triangles)                 |
| 0x0C   |   4  | `u32`       | `num_submeshes` | `1`                                          |
| 0x10   |   8  | `[u8; 8]`   | _(reserved)_    | all-zero                                     |
| 0x18   |  12  | `[f32; 3]`  | `bbox_min`      | `(-0.157, -0.097, 1.539)` (head-height bbox) |
| 0x24   |  12  | `[f32; 3]`  | `bbox_max`      | `(+0.157, +0.148, 1.848)`                    |
| 0x30   |  12  | `[u8; 12]`  | _(zero)_        | all-zero                                     |
| 0x3C   |   4  | `u32`       | _(unknown)_     | `2`                                          |
| 0x40   |   4  | `u32`       | _(unknown)_     | `1`                                          |
| 0x44   |  28  | `[u8; 28]`  | _(zero / pad)_  | mostly zero, last 8 bytes are noise          |

The two non-zero fields at `+0x3C` and `+0x40` are documented as `_unknown`;
they do not gate parsing.

**The chunk contains NO geometry buffers.** Unlike the ship `.skin`
`MeshIvo320` chunk (which embeds counts + submesh descriptors + the
quantized vertex stream in one larger block), the head variant stops at
the metadata header. The vertex stream, index stream, and bone-weight
stream that one would expect from a CryEngine `.skin` are simply absent
from the file. This was verified two ways:

1. The chunk-table sizes account for every byte of the 347,000-byte
   fixture (`16 + 18,512 + 176 + 104 + 328,000 + 80 = 346,888` plus the
   112-byte IVO header). No unaccounted region exists.
2. The SKIN dispatcher (`FUN_1405c9600`) recognises ten distinct chunk
   types, none of which are a vertex-stream chunk. The only chunks it
   stores pointers for in this file are the six listed in the chunk table
   above.

**Geometry is in the `.skinm` companion.** Phase 3b (`parse_head_skin` in
`crates/starbreaker-3d/src/character/head_skin.rs`) reads the companion
`.skinm` to populate `HeadMesh::positions`, `normals`, `uvs`, `indices`,
`joints`, and `weights`. See the "`.skinm` companion" section above for
the full stream layout and bone-weight format.

**Evidence:** byte-anchored. The chunk role (`MESHINFO`) is confirmed by
the RTTI type name `CHUNKHEADER_0x900R_MESHINFO` at `0x1498eaf90`.

## RBF / DNA-to-weights algorithm (engine `ScatteredDataInterpolation`)

This section documents the engine's **CIG-bespoke scattered-data
interpolation**, anchored to the V1.9 Windows client: the character
customizer's per-vertex solver that turns the CHF DNA blend selections
(the `face_parts: BTreeMap<FacePart, [DnaBlend; 4]>` field on `Dna`)
into the per-character neutral head shape. StarBreaker does not port
it: it applies the DNA-matrix-weighted sum of the `.dna` slot-18
per-variant neutral deltas directly (see "`.dna` file format" below and
`PHASE_4B_INVESTIGATION.md` §6.16.24), which reproduces the in-game
head shape.

### Engine algorithm (Ghidra-anchored)

**Class:** `ScatteredDataInterpolation` (private to the CIG character
customizer subsystem; only one symbol leaks to strings).

**Entry point:** `ScatteredDataInterpolation::ComputeControlPointFromWeights`
at function `FUN_143ff6710` (Ghidra address `0x143ff6710` in
`StarCitizen.exe` build May 2026). The string
`"%s No tetrahedron exists that contains all variants in dna matrix for this vertex"`
at `0x1483481a0` is the panic anchor; the leading `%s` is filled with
the class-qualified function name string at `0x148348160`.

**Caller:** `FUN_1440849d0`, part of `CCharacterCustomizerComponent`
(near `HandleDNABlending` `0x148344100` and `UpdateDNAMatrix`
`0x148344388`). The caller iterates one
`ComputeControlPointFromWeights` invocation per **vertex** (or per
per-vertex DNA-blend control point), passing:

- `param_1`: pointer to a per-vertex `ScatteredDataInterpolation` data
  block (lives in the loaded `.dna` file mmap), structured roughly as:
  - `+0x30 .. +0x38`: pointer span over `byte[4]` **tetrahedra** —
    each tetrahedron is 4 variant indices.
  - `+0x98 .. +0xa0`: pointer span over `double[3]` **control points**
    in feature space — one 3D position per variant.
- `param_2`: output `float[3]` — the interpolated 3D control point for
  the current vertex.
- `param_3`: pointer to a `std::vector<double>` of **per-variant
  weights**, length = number of variants = number of control points.

The function:

1. Validates that the weights vector length matches the control-point
   count; on mismatch returns `(0,0,0)`.
2. Walks every (variant, weight) pair. If a weight equals
   `DAT_147a211d8` (a sentinel) and is the highest weight seen, the
   function short-circuits and returns that variant's control point
   directly. This is the "exact-variant" fast path used when the user
   selects a single variant at 100%.
3. Otherwise it builds a sorted "top-K" list of `(variant_index, weight)`
   pairs (the buffer at `auStack_128`, max length 4 — i.e. it keeps
   the 4 strongest variants), inserting only when weight exceeds the
   current minimum in the buffer.
4. Iterates the **tetrahedra** array (`(*+0x38 - *+0x30) / 4` rows,
   each row a `byte[4]` of variant indices) and finds the first
   tetrahedron whose 4 vertices are all present in the top-K list.
5. If no such tetrahedron exists, calls
   `FUN_140292e60("%s No tetrahedron exists that contains all variants in dna matrix for this vertex", "ScatteredDataInterpolation::ComputeControlPointFromWeights")`
   — this is `FatalError` / `__noreturn`. **Strict-fail on
   degenerate input.**
6. With the containing tetrahedron found, takes the 4 variants' control
   points `V0..V3` (each `double[3]` → converted to `float[3]`) and
   computes:
   - The 4 face-normal vectors `n_i` (each normalised via Newton-Raphson
     refined `rsqrtps`).
   - 4 plane offsets `d_i = dot(V_apex_i, n_i)`.
   - The 4 **per-face plane-intersection points** by projecting an
     interpolated weight-space point onto each face's plane, gated by
     a `|denominator| >= threshold` test (`DAT_147e0f10c`) to skip
     near-parallel faces.
   - Linear blends between the projected points and the tetrahedron
     vertices using the four sentinel-minus-weight scalars
     `1.0 - w_i` (where `DAT_147a211d8 = 1.0` is the sentinel).
   - Solves a 3-equation Cramer's-rule system (4 times — once per
     dropped face) to find barycentric-equivalent coordinates of the
     interpolated point inside the tetrahedron.
   - Cross-checks the 4 independent solutions for self-consistency;
     if they disagree by more than `DAT_147a2a04c` it fatals with
     `"calculated plane intersects are not equivalent!"`.
   - Returns the averaged 4-way solution scaled by `DAT_147a33dd0`.
7. Output is a single `float[3]` — **a 3D position in feature space,
   one per vertex / per per-vertex DNA control point**.

### Why this is NOT a per-blendshape-weight evaluator

The output of `ComputeControlPointFromWeights` is a **3D position per
vertex** (or per per-vertex DNA control point), not a `[f32; 495]`
appearance/corrective-blendshape weight vector. The function is part
of a **per-vertex deformation** pipeline where each driven vertex
carries its own tetrahedral mesh in variant feature-space, and the
output 3D position drives that vertex's morph offset directly.

The pipeline appears to be:

1. Engine builds the per-vertex `ScatteredDataInterpolation` blob
   (`param_1`) at character-construction time from the `.dna` files.
2. User edits one of the `[DnaBlend; 4]` slots (the `param_1 + 0x2da0`
   `double[blend_pool_count * blends_per_pool]` array touched by
   `HandleDNABlending`).
3. `UpdateDNAMatrix` recomputes the per-variant weights from the
   user-facing blend selections.
4. For every driven vertex, `ComputeControlPointFromWeights` is called
   with the per-vertex tetrahedra blob + the variant-weight vector,
   producing the vertex's deformed feature-space position.
5. That feature-space position is consumed (deeper in the customizer)
   to produce the actual vertex delta, which is then fed into the
   skinning / blendshape pipeline.

In other words: the engine does NOT compute `[f32; 495]` blendshape
weights from CHF DNA at all. The appearance / corrective blendshape
activations are **animation-driven** at runtime by `CFaceRigLogic`
(facial-expression keyframes or external controls), while CHF DNA
blends drive a parallel **per-vertex neutral-shape** path.

### External inputs

The data behind `ComputeControlPointFromWeights` (other than the runtime
weight vector) comes from **separate `.dna` files**, loaded by
`DNAFile::LoadAllLODs` (`FUN_14142d720`) via path patterns:

- `<rig_name>.dna`
- `<rig_name>_lod1.dna` … `<rig_name>_lod5.dna`

Loader: `DNAFile::Load` (`FUN_14142d280`, anchored by string
`"bool __cdecl DNAFile::Load(const char *)"` at `0x147cf3358`).

The variant count is asserted to be non-zero with error
`"[%s] Failed to initialize protos mesh %s in DNA file %s. Zero variants found."`
at `0x147cf3640` (anchor for the variant-table presence).

The control points and tetrahedra themselves are not stored on disk:
they are runtime `std::vector`s built per vertex when a character is
built (the per-vertex control points are decoded from slot 18). The
`.RigLogic` file carries none of this — see "Variant metadata
(out-of-file)" in [Known gaps](#known-gaps). The parsed format is in
[`.dna` file format](#dna-file-format) below (crate `starbreaker-dna`).

## Known gaps

**Per-region metadata layout (still open):** Task 4 anchored Sections G & H
as **per-region bone-index lists + per-bone-per-region weight scalars** —
NOT a per-region "name / CRC / symmetry / flags / pivot" table. The V3.0
server format may carry such metadata; the V1.9 client format does not.
There is currently no evidence in the V1.9 binary of a region name pool, a
region symmetry table, or per-region pivots. The typed per-region data
lives on `RigLogic::blend_region_bones: Vec<BlendRegionBones>` and
`RigLogic::region_weights: Vec<f32>`; the character pipeline uses them as
the wrinkle-map drivers (`character::wrinkles`).

**Variant metadata (out-of-file):** the V1.9 `.RigLogic` file does NOT
contain ethnicity / build / skin-tone / age / variant arrays. Those
concepts come from the V3.0 server format and were carried forward into the
plan-stage Task 4 design doc by mistake. In the V1.9 client, that data
lives in **separate `.dna` files** loaded via `DNAFile::Load`. Anchoring
strings in the V1.9 binary:

- `"DNA Matrix 1.0"` at `0x147cf3048`
- `"DNAFile::Load"` at `0x147cf3320`
- `"_lod1.dna"` at `0x147cf3420`
- `"DNAFaceCustomizer"` at `0x147a2aa40`

The `.dna` format anchoring is a separate task, NOT a follow-up to the
`.RigLogic` parser. The `RigLogic` struct deliberately has **no**
`Vec<Variant>` or per-ethnicity field; the original Task 4 plan's
forward-looking signatures (drawn from V3.0 server-format assumptions) are
not applicable to V1.9.

**Section C corrective-composition loop trip count (unresolved):** Section
C has 290 valid 5-byte rows on disk (= `num_corrective_blendshapes`), but
the `CFaceRigLogic::ExecuteGeneric` decompile reads only `floor(290 / 5) =
58` rows on each invocation. Either the loop is read incorrectly, or only
58 of 290 corrective weights are actually computed at runtime. See
"Section C corrective-composition loop discrepancy" above. The parser
stores all 290 rows verbatim regardless.

## `.dna` file format

The DNA per-vertex morphing pipeline is driven by separate `.dna` files
loaded by `DNAFile::Load` (Ghidra anchor `FUN_14142d280`,
`"DNAFile::Load"` string at `0x147cf3320`). The format is V1.6 and lives
alongside the `.RigLogic` content but is structurally independent.

Phase 4a (`crates/starbreaker-dna`) implements the parser at the
structural level — header, pointer table, segment spans. The Phase 4a
extension pass anchored the per-variant **delta-records** array
(pointer-table slot 31, `+0x138`) by cross-decompiling
`Protos::ComputeMixedDeltaMatrixNumEntries` against the master `.dna`
fixture. Two payloads originally hypothesised to be flat slots in the
file — the per-variant feature-space **control points** (`double[3]`)
and the variant-space **tetrahedra** (`byte[4]`) — turned out to be
**runtime-constructed**, not stored on disk; see "Phase 4a extension: HALT on control points +
tetrahedra" below. Typed slot readers on `DnaFile`: `variant_names` (2),
`variant_neutral_joints` (15), `neutral_morphs` (18), `expression_morphs`
(19), `joint_weights` (21), `region_weights` (22),
`variant_main_deltas` / `variant_secondary_deltas` (31).

### File-level layout

Path pattern (one file per LOD; master file holds all LODs collapsed):

```text
Data/Objects/Characters/.../<protos_name>/<protos_name>_head.dna          (master)
Data/Objects/Characters/.../<protos_name>/<protos_name>_head_lod1.dna     (LOD 1)
Data/Objects/Characters/.../<protos_name>/<protos_name>_head_lod2.dna
...
```

Verified against the LOD1 fixture
`protos_human_male_face_t1_pu_head_lod1.dna` (1,552,792 bytes):

| Offset    | Size | Field             | Tag           | Notes |
|-----------|------|-------------------|---------------|-------|
| `0x00`    | 16   | `magic`           | **[verified]**| ASCII `"DNA V1.6"` then eight NUL bytes. The Load function compares as `u64 == 0x362e315620414e44` and the trailing `u32 == 0`. |
| `0x10`    | 8    | `timestamp`       | **[verified]**| `u64`. `LoadAllLODs` cross-validates that every LOD in a set has the same value (`*(longlong*)(lVar8 + 0x10)`). |
| `0x18`    | 4    | `protos_name_id`  | **[verified]**| `u32` (CRC32 of the protos asset name). LoadAllLODs compares against `param_4[0x12]`. In the LOD1 fixture: `3,714,869,238` (= CRC32 of `"protos_human_male_face_t1_pu"`). |
| `0x1c`    | 4    | `mesh_name_id`    | **[verified]**| `u32` (CRC32 of the mesh / sub-asset name). LoadAllLODs compares against `param_4[5]`. In the LOD1 fixture: `3,934,033,896` (= CRC32 of `"head"`). |
| `0x20`    | 4    | `header_u32_20`   | **[inferred]**| Unanchored; likely a flag/format word. Fixture value: `0xa8c9c5ca`. |
| `0x24`    | 4    | `header_u32_24`   | **[inferred]**| Unanchored. Fixture value: `0x04030100` — looks like four packed bytes. |
| `0x28`    | 2    | `header_u16_28`   | **[inferred]**| Unanchored. Fixture value: `3341`. |
| `0x2a`    | 2    | `num_variants`    | **[verified]**| `u16`. The `"Zero variants found"` runtime diagnostic at `0x147cf3640` is emitted iff this is zero (`*(short *)(lVar8 + 0x2a) == 0`). Fixture value: `61`. |
| `0x2c`    | 2    | `header_u16_2c`   | **[inferred]**| Unanchored. Fixture value: `198`. |
| `0x2e`    | 2    | `header_u16_2e`   | **[inferred]**| Unanchored. Fixture value: `179`. |
| `0x30`    | 4    | `num_vertices`    | **[verified]**| `u32` — this LOD's mesh vertex count. Anchored via cross-validation against the head `.skin` `MeshIvo320.num_vertices`: master `.dna` declares `5,584` (matching head `.skin`), LOD1 `.dna` declares `3,158` (the decimated LOD1 count). |
| `0x34`    | 4    | `num_indices`     | **[verified]**| `u32` — this LOD's mesh index count. Anchored the same way: master declares `32,178` (matching head `.skin`), LOD1 declares `17,724`. |
| `0x38`    | 4    | `header_u32_38`   | **[inferred]**| Unanchored. Low u16 = `780` in both LOD1 and master; high u16 = `495` in master (= `num_appearance + num_corrective` from RigLogic) but `0` in LOD1. Suggests the high half is the total blendshape count, populated only on the master. |
| `0x3c`    | 4    | `header_u32_3c`   | **[inferred]**| Unanchored. LOD1: `0x01040508`, master: `0x00040508`. The differing high byte (0x01 vs 0x00) looks like a LOD-index byte but only two LOD samples are available, so this is **[inferred]** until confirmed against LOD2..LOD5. |
| `0x40`..`0x140` | 256 | `pointer_table[32]` | **[verified]** | 32 `u64` file-relative offsets, indexed by the **runtime** struct's slot offset (slot `i` lives at file offset `0x40 + i*8`). Zero = absent. `FUN_14142a990` adds the file base to every nonzero slot to produce a pointer in-memory. |
| `0x140`..end of payload | varies | payload bytes for non-null slots | **[verified]** | Each slot's payload starts at the offset stored in the table. No explicit per-slot length is recorded — segment extent is inferred from the next-larger non-null slot offset (or end of file). |
| EOF − strlen(path) − 1 | varies | trailing path copy | **[verified]** | The Load function appends a `memcpy` of the file path bytes after the payload (`*(size_t *)(*param_1 + 0x140) = param_1[1] + _Count; memcpy(...)`). |

### Pointer-table slot map (partial)

From the LOD1 fixture (`*` = the only non-null slots in this file):

| Slot index | Runtime byte offset | Slot value (file offset) | Span size  | Status / role |
|-----------:|---------------------|--------------------------|-----------:|---------------|
| 0  *       | `+0x40`             | `0x148`                  | 29 bytes   | **[verified]** Null-terminated protos name string (`"protos_human_male_face_t1_pu"`). Cross-anchored by `*(undefined8 *)(lVar8 + 0x40)` use in `LoadAllLODs`. |
| 1  *       | `+0x48`             | `0x165`                  | 11 bytes   | **[verified]** Null-terminated mesh name string (`"head"`). Cross-anchored by `*(undefined8 *)(lVar8 + 0x48)`. |
| 2..11      | `+0x50`..`+0x98`    | `0`                      | —          | Null in LOD1. Roles unanchored — the slots exist (fixup loop covers them) but no payload is referenced in this LOD. |
| 12 *       | `+0xa0`             | `0x170`                  | 56 bytes   | **[verified-as-int-array]** 12 × `i32` containing `0,1,2,3,...,11`. Cross-anchored by `plVar27[0x14]` (= `+0xa0` as a qword offset) in `FUN_1465546f0`'s wrap-deformation path: `*(uint *)(plVar27[0x14] + uVar38 * 4)`. **[inferred]** semantically: per-variant or per-LOD remap index list. |
| 13..17     | `+0xa8`..`+0xc8`    | `0`                      | —          | Null in LOD1. |
| 18 *       | `+0xd0`             | `0x28328`                | 1,388,144 bytes | **[inferred]** This LOD's per-(variant × region) neutral vertex deltas, presumably in the master's slot-18 layout (header `0x38` low u16 = 780 controls in both; `PHASE_4B_INVESTIGATION.md` §6.16.24). Not validated on LOD1 (the exporter uses LOD0). |
| 19..21     | `+0xd8`..`+0xe8`    | `0`                      | —          | Null in LOD1. |
| 22 *       | `+0xf0`             | `0x1a8`                  | 164,224 bytes | **[inferred]** This LOD's per-vertex DNA-region masks in the master's slot-22 layout: `13 × 3,158 × f32 + 8` = 164,224 bytes exactly. |
| 23..30     | `+0xf8`..`+0x130`   | `0`                      | —          | Null in LOD1. |
| 31 (master only) | `+0x138`     | `0x00024004` (master)    | 21,586,292 bytes | **[verified]** Per-variant delta-records array. Slot 31 has special handling in `FUN_14142a990`: after the base fixup it walks an inner table of size `num_variants` and fixes up two qwords per row at `+0x18` / `+0x20` relative to that table. **NULL in LOD1**, populated in the master. Layout cross-anchored against `Protos::ComputeMixedDeltaMatrixNumEntries` AND the master file's bytes (variant 0's `primary_delta_scale = 0.0676762` matches the RigLogic Section A scale, confirming the field assignments). See "Slot 31: per-variant delta records" below for the per-record byte layout. |

### Master `.dna` pointer-table slot map (Phase 4b investigation)

The LOD1 file is sparse — only slots 0, 1, 12, 18, 22 are populated.
The **master** `protos_human_male_face_t1_pu_head.dna` (75,716,224 bytes,
`num_variants = 61`, `num_vertices = 5584`, `num_joints = 179` from the
companion `.RigLogic`) populates 31 of 32 slots (slot 16 is null). The
Phase 4b investigation extracted the master's slot values and computed
the span of each slot (next-larger non-null slot offset, or EOF). Slot
roles are mostly inferred from byte content; see
[`PHASE_4B_INVESTIGATION.md`](PHASE_4B_INVESTIGATION.md) for full
details and the chain of evidence.

| Slot | File offset | Span (bytes) | Tag           | Decoded role |
|-----:|------------:|-------------:|---------------|--------------|
|  0   |         328 |          29  | **[verified]**  | Protos name `"protos_human_male_face_t1_pu"`. |
|  1   |         357 |           5  | **[verified]**  | Mesh name `"head"`. |
| 13   |         362 |          70  | **[anchored]**  | Face-part root strings (e.g. `FACIAL_C_FacialRoot`). |
|  3   |         432 |         248  | **[inferred]**  | 62 × `u32` hash-like values (probably per-variant identity hash + zero terminator). |
|  2   |         680 |       2,448  | **[verified]**  | **Per-variant archetype name pool**: 61 × 40-byte NUL-padded names, entry 0 = `protos_human_<g>_face_t1_pu`; CHF `head_id` indexes it; each name is the stem of `<name>_head.skinm`. **Reader:** `DnaFile::variant_names()`. |
|  5   |       3,128 |          16  | **[inferred]**  | 3 × `u32` NameHash + zero (likely CRCs of slot 4's face-part labels). |
|  4   |       3,144 |          56  | **[anchored]**  | Face-part labels: `"teeth"`, `"eyes"`, `"head"` (3 × 16-byte slots + 8 padding). |
| 24   |       3,200 |          24  | **[inferred]**  | 24-byte preamble before slot 23 (looks like 2 × NameHash u64 + 8 zero). |
| 23   |       3,224 |          88  | **[anchored]**  | Ethnicity pool: `"AFRICAN"`, `"ASIAN"`, `"CAUCASIAN"`, … (16-byte slots). |
| 25   |       3,312 |       1,224  | **[inferred]**  | Float lookup table (306 × `f32`); first bytes show `0/1` patterns. Possibly per-(ethnicity, face-part, axis) bias. |
| 27   |       4,536 |          24  | **[inferred]**  | Preamble before slot 26 (like slot 24). |
| 26   |       4,560 |          72  | **[anchored]**  | Build pool: `"SKINNY"`, `"AVERAGE"`, `"HEAVY"`, … (16-byte slots). |
| 28   |       4,632 |         984  | **[inferred]**  | 61 × 16 bytes + 8 pad. Four `f32` per variant — likely a 4-component classifier or orientation. |
| 29   |       5,616 |         248  | **[inferred]**  | 62 × `f32` in `[0,1]`. Per-variant scalar (build/ethnicity weight?). |
| 30   |       5,864 |         248  | **[inferred]**  | 62 × `i32` small ints (e.g. 40, 25, 60, 35). Per-variant integer parameter. |
|  9   |       6,112 |         800  | **[inferred]**  | Mostly `0x01` bytes — per-variant per-face-part flag table candidate. |
| 10   |       6,912 |       9,520  | **[inferred]**  | 2380 × `f32`. Span does not factor cleanly by `num_variants` or `num_vertices`. |
| 15   |      16,432 |     131,028  | **[verified]** | **`61 × 179 × 3 × f32`.** Per-variant per-RigLogic-joint **parent-LOCAL bind translation**; variant 0 equals the head `.skin` bone locals exactly. The variant-0-relative fold is applied as a RE-BIND (`apply::rebind_skeleton`). |
| 31   |     147,460 |  21,586,292  | **[verified]**  | Per-variant delta-records array (61 × 40-byte headers + packed payloads). See "Slot 31" subsection below. |
| 14   |  21,733,752 |         720  | **[inferred]**  | 720 bytes; doesn't match 12 × 60 or 12 × 61 cleanly. Unclear. |
| 21   |  21,734,472 |       9,312  | **[verified]**  | Per-(joint × region) weights `[179][13]` f32 + 4 pad; each row sums to 1 (joint 0 all zero). `ExecuteHuman` folds the DNA matrix with it. **Reader:** `DnaFile::joint_weights()` (`PHASE_4B_INVESTIGATION.md` §6.7). |
|  7   |  21,743,784 |          56  | **[inferred]**  | 14 × `u32` hashes (matches slot 8's 14 entries — possibly per-region NameHash). |
|  6   |  21,743,840 |         216  | **[anchored]**  | Per-region name pool: 9 × 24-byte slots `"BROW_L"`, `"BROW_R"`, `"EYE_L"`, … |
|  8   |  21,744,056 |          56  | **[anchored]**  | 14 × `u32` symmetric index pairs `[1,3,2,4,6,5,…]` — left/right region swap. |
| 11   |  21,744,112 |          48  | **[anchored]**  | Path string `"Objects/Characters/Human/heads/highlighting/"`. |
| 12   |  21,744,160 |          56  | **[verified]**  | 14 × `u32` `[0,1,…,13]` (master) / 12 × `u32` `[0,1,…,11]` (LOD1). Per-LOD index remap. |
| 22   |  21,744,216 |     290,376  | **[verified]** | **Per-vertex DNA-region masks**, REGION-major `[13][5584] f32` from the slot start + 8 trailing bytes (an earlier "8-byte header, vertex-major" reading was wrong). Per-vertex partition of unity. **Reader:** `DnaFile::region_weights()` (returns the vertex-major view). |
| 18   |  22,034,592 |   2,416,576  | **[verified]** | **Per-(variant × region) neutral vertex deltas** (packed codes). One variant decoded with all regions on equals `<archetype>_head.skinm − protos base` within 0.2 mm. **Reader:** `DnaFile::neutral_morphs()`. |
| 20   |  24,451,168 |      23,768  | **[anchored]**  | Blendshape-control name pool. First entry `"#BS_CTL_PSD"`. |
| 19   |  24,474,936 |  48,471,624  | **[verified]** | **Per-(variant × 495-blendshape) archetype blendshape library** — cell `(v,b)` = archetype `v`'s `BShapesGpu[b]`. See "Slot 19" subsection below. |
| 17   |  72,946,560 |   2,769,664  | **[inferred]**  | **`2,769,664 / 5584 ≈ 495.99`** and `num_appearance + num_corrective = 495`. **Per-vertex × per-blendshape `u8` mask**, stride 496: the rig-global attenuation `1 − byte/255` applied to `BShapesGpu` deltas (empirical; `PHASE_4B_INVESTIGATION.md` §6.14). |

**The slot 31 variant-array region (147,460 → 21,733,752)** is internally
sub-divided into the 61 × 40-byte header table at the start, followed by
the 61 variants' packed delta payloads (variant 0's main payload starts
at `0x64934 = 411,956`, well inside this region). The conservative
"slot 31 span" stated above (`21,586,292` bytes) covers the whole
header-plus-payloads region.

### Slot 31: per-variant delta records

Anchored against `Protos::ComputeMixedDeltaMatrixNumEntries`
(`FUN_14142a5c0`) and re-verified by decoding the master `.dna`'s
slot-31 bytes directly. Slot 31 holds an array of `num_variants`
records, each `0x28` (40) bytes:

| Offset | Size | Field                          | Type | Notes |
|--------|------|--------------------------------|------|-------|
| `+0x00`| 4    | `num_main_delta_records`       | u32  | Per-variant override count. Outer loop in `ComputeMixedDeltaMatrixNumEntries` reads this directly off the variant header (`lVar5 + uVar15 * (int)pcStackX_20 + iVar17) * 4`-style indexed access). Master variant 0: `517` (matches the RigLogic Section A `num_main_delta_records`). |
| `+0x04`| 4    | `num_secondary_delta_records`  | u32  | Master variant 0: `528` (matches RigLogic `num_secondary_delta_records`). |
| `+0x08`| 8    | `primary_delta_scale`          | f64  | Per-variant override of RigLogic's `primary_delta_scale`. Master variant 0: `0.067676` (= RigLogic's value to 6 d.p.). Variant 1 has `0.044233` — i.e. each variant carries its own scale. |
| `+0x10`| 8    | `secondary_delta_scale`        | f64  | Same pattern. Master variant 0: `1.616942` (= RigLogic's `secondary_delta_scale`). |
| `+0x18`| 8    | `main_delta_entries_offset`    | u64  | Raw file-relative offset to this variant's packed main-delta entries. `FUN_14142a990` adds the file base at load time to convert to a pointer. Each entry is a `u32` packed `(blendshape_idx, signed_delta)` — same format as RigLogic Section D. |
| `+0x20`| 8    | `secondary_delta_entries_offset` | u64 | Same as `+0x18` but for secondary (corrective) deltas. |

Total array span: `num_variants * 0x28` bytes for the headers, followed
by the per-variant packed delta payloads pointed at by `+0x18` / `+0x20`.
The conservative segment length recorded by the parser captures all of
this (the next-larger non-null slot bounds the region).

### Slot 19: per-(variant × blendshape) archetype blendshape library

> **Corrected 2026-10-13.** The earlier reading below this heading
> (vertex walk from 0, bytes 1..4 as a u24 weight scaling the `+0x14` vec3)
> was wrong and produced misplaced, mis-shaped deltas. The layout here is
> verified against ground truth: cell `(v, b)` reproduces `BShapesGpu[b]` of
> archetype `v`'s own `<name>_head.skinm` (variant 0 = the protos base head)
> to within one 8-bit quantisation step for ≥ 99.86 % of entries on all 60
> male archetypes (test `dna_ground_truth::slot19_cells_equal_archetype_blendshapes`).

**Reader:** `starbreaker_dna::DnaFile::expression_morphs()` → `ExpressionMorphs`
(`.record(variant, blendshape)` → `ExpressionRecord { start_vertex, scale,
bias, entries() }`, entries yield `(vertex, Vec3 delta)`). Consumed by
`starbreaker_3d::character::expression`.

**Region A** `[0, dword[0] = 0xEBE60)` = `61 × 495` records × 32 bytes,
variant-major (`r → variant = r / 495, blendshape = r % 495`):

| Offset | Size | Field | Notes |
|--------|------|-------|-------|
| `+0x00`| 4  | `block_offset` (u32) | Byte offset (from the slot start) of the entry list. |
| `+0x04`| 4  | `hi16 \| lo16` (u32) | `lo16` = entry count; **`hi16` = start vertex** of the walk. Zero ⇒ empty cell. |
| `+0x08`| 12 | `scale` (vec3 f32) | Per-axis dequantisation step (= range / 255). |
| `+0x14`| 12 | `bias` (vec3 f32)  | Per-axis minimum (delta of code 0). |

**Region B** entry (4 bytes): `[u8 vertex stride][u8 qx][u8 qy][u8 qz]`. The
vertex index starts at `hi16` and advances by each stride; the entry's delta is
`bias + q · scale` per axis (codes truncated, so ≤ 1 step error).

**Semantics.** Slot 19 is each archetype's own blendshape set — the
expression counterpart of slot 18 (neutral shape). A character's blendshape
`b` is the same region-masked convex archetype blend:
`blend_b[vtx] = Σ_v fold[vtx][v] · T_v[b][vtx]`, `fold[vtx][v] = Σ_r
slot22[r][vtx] · matrix[v][r]` (slot 22 is a partition of unity, the matrix is
per-region normalised). Added to the base head's `BShapesGpu` target `b`, the
per-character correction is `blend_b − T_0[b]`. Blendshape 0 (CTL_PSD) is empty
for every variant, so slot 19 is zero at the neutral rest pose.

### Phase 4a extension: HALT on control points + tetrahedra

Not stored in the `.dna`: the `ScatteredDataInterpolation` control points and tetrahedra are runtime vectors built per vertex at character build (see "External inputs" above), so the parser exposes neither; the exporter uses slot 18 instead (`PHASE_4B_INVESTIGATION.md` §6.16.24).

### Ghidra anchors

- `DNAFile::Load` — `FUN_14142d280`. String `"DNAFile::Load"` at
  `0x147cf3320`. Verifies magic, allocates a single contiguous buffer,
  reads the file, runs the pointer-fixup pass, and stores the path
  copy at the end.
- `DNAFile::LoadAllLODs` — `FUN_14142d720`. String `"DNAFile::LoadAllLODs"`
  at `0x147cf3498`. Iterates `_lod1.dna`..`_lod5.dna`, validates
  matching `protos_name_id` / `mesh_name_id`, matching `timestamp`
  across LODs, and the non-zero `num_variants` requirement.
- Pointer-fixup helper — `FUN_14142a990`. Adds the file base to every
  nonzero `u64` at offsets `+0x40, +0x48, …, +0x138`. For slot 31
  (`+0x138`) additionally walks the inner table and fixes up two
  qwords per row.
- `Protos::ComputeMixedDeltaMatrixNumEntries` — `FUN_14142a5c0`. Uses
  the DNA struct fields: `+0x2a` (num_variants), `+0x138` (variant
  array pointer), inner per-variant stride `0x28` bytes, sub-pointers
  at `+0x18` and `+0x20` per row. Per-variant header layout decoded
  from the master `.dna`:
  `(u32 num_main, u32 num_secondary, f64 primary_scale, f64 secondary_scale, u64 main_offset, u64 sec_offset)`.
- Wrap-deformation consumer — `FUN_1465546f0`. Reads `+0x29` (byte,
  per-variant driver count), `+0x2a` (num_variants), `+0xa0` (int32
  remap array), plus the variant array off `+0x138`.
- `ScatteredDataInterpolation::ComputeControlPointFromWeights` —
  `FUN_143ff6710`. String at `0x148348160`. Reads the **runtime**
  blob at `param_1+0x30/0x38` (tetrahedra) and `param_1+0x98/0xa0`
  (control points). Caller `FUN_1440849d0` passes `character + 0x2b98`
  as that `param_1` — i.e. the blob is a per-character heap allocation,
  not a `.dna` file slice. The blob is move-constructed by
  `FUN_143f6d760`, which treats those triples as
  `(begin, end, capacity)` `std::vector` fields.
- Tetrahedron-generator CVar — `FUN_140087990` registers
  `None / Default / CGAL / CDT / fTet`. Confirms the runtime computes
  the variant-space tetrahedralization on load rather than reading it
  from disk.

### Strings used during anchoring

| Address      | String |
|--------------|--------|
| `0x147cf3320`| `"DNAFile::Load"` |
| `0x147cf3498`| `"DNAFile::LoadAllLODs"` |
| `0x147cf3640`| `"[%s] Failed to initialize protos mesh %s in DNA file %s. Zero variants found."` |
| `0x147cf34b0`| `"[%s] Invalid protos name (%s vs. %s) in DNA file %s. ..."` |
| `0x147cf3530`| `"[%s] Invalid protos mesh name (%s vs. %s) in DNA file %s. ..."` |
| `0x147cf3140`| `"void __cdecl Protos::ComputeMixedDeltaMatrixNumEntries(const struct FaceRigHeader &,const class DNAHeader &)"` |

## References

### Ghidra (current authoritative source — V1.9 Windows client)

Build: `StarCitizen.exe` build May 2026, decompile via Ghidra MCP.

- `FaceRigData::Load` at `0x14142d4d0` — the loader. Verifies magic at
  byte 0..7, header-size short at byte `0x3e`, 8-byte alignment, then
  `memcpy`s and fixes up eight relative offsets at buffer-offsets
  `0x68, 0x70, 0x78, 0x80, 0x88, 0x90, 0x98, 0xA0`.
- `CSkin::LoadNewSKIN` at `0x1465fc580` — calls `FaceRigData::Load` with
  the IVO chunk content (skipping the 8-byte `data_size` prefix). Anchors
  the embedded-vs-standalone wrapper distinction.

Strings used during anchoring:

- `[%s] Wrong version number in RigLogic data: '%s'` (`0x147cf2f70`)
- `[%s] Wrong header size in RigLogic data: '%s'` (`0x147cf2fa8`)
- `[%s] Pointers not 8-byte aligned in RigLogic data: '%s'` (`0x147cf2fd8`)

### Linux server decompile (historic; format-mismatched)

A decompile of the Linux dedicated server's CryEngine sources (the
`cig-scgs` sibling project, outside this repo) contains the V3.0 format.
It uses a different format magic
(`"DNA V3.0"`) and the struct accesses don't anchor V1.9 bytes. Useful for
cross-reading naming conventions (`m_protosName`, `m_appearanceBlendShapes`,
etc.) but the absolute offsets disagree.

Companion CHF parser: `crates/starbreaker-chf`. CHF's `[DnaBlend; 4]` per face
part are the inputs that drive variant selection / blend weights into this
`RigLogic` data.
