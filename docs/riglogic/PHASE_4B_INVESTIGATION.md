# Phase 4b: DNA-driven facial morphing — anchored findings

**Status:** reference. Every conclusion below is implemented in
`crates/starbreaker-dna` (slot readers) and
`crates/starbreaker-3d/src/character/` (DNA matrix, morphing, rig
assembly); `PIPELINE.md` describes the current export flow. This file keeps
the engine evidence the code relies on. Section numbers are stable because
code comments cite them; gaps in the numbering are sections that only held
superseded narrative.

**Fixture:** `Data/Objects/Characters/Human/heads/male/pu/protos_human_male_face_t1_pu/protos_human_male_face_t1_pu_head.dna`
in `Data.p4k` (75,716,224 bytes): `num_variants = 61`, `num_vertices = 5,584`,
`num_indices = 32,178`, 179 RigLogic joints, 495 blendshapes
(205 appearance + 290 corrective), `mesh_blends_per_pool = 13`.

**Binaries:** the first passes were anchored in a May 2026 `StarCitizen.exe`;
Passes 17–19 in `StarCitizen_4.8_live.exe`. Addresses drift between builds;
the RTTI / function-name strings are the durable anchors (see the address
table in §6).

---

## 1. Master `.dna` slot map

The `.dna` header (`0x00..0x40`, see `SPEC.md`) is followed by a 32-slot
pointer table. **Slot N's file offset is stored at `0x40 + N*8`**; at load
`FUN_14142a990` rebases every non-zero entry in place, so at runtime the
same slot is the pointer at `ProtosMesh + 0x40 + N*8` (slot 15 → `+0xb8`,
17 → `+0xc8`, 18 → `+0xd0`, 19 → `+0xd8`, 21 → `+0xe8`, 22 → `+0xf0`,
31 → `+0x138`). A 4.8 process dump confirmed every slot is consumed in place
from the file mapping; there is no decoded heap copy (Pass 19M).

| Slot | Role (master fixture) | Reader | § |
|-----:|-----------------------|--------|---|
| 0, 1 | protos name `protos_human_male_face_t1_pu`, mesh name `head` | header | — |
| 2 | 61 × 40-byte archetype names; entry 0 = the protos base; CHF `head_id` indexes it directly (1..60 are the selectable heads) | `variant_names` | §6.16.24 |
| 4, 6, 8, 12, 13 | face-part / region name pools and index tables (slot 12 maps a mesh's local regions to global face parts) | — | — |
| 15 | `61 × 179 × 3` f32: per-variant **parent-local** neutral joint translations; variant 0 = the head `.skin` bone locals | `variant_neutral_joints` | §6.16.6 |
| 17 | `5,584 × 496` u8: rig-global per-(vertex, blendshape) attenuation mask for `BShapesGpu` | `segment_for_slot(17)` | §6.14 |
| 18 | per-(variant × region) **neutral vertex deltas** (the per-character head shape) | `neutral_morphs` | §6.16.24 |
| 19 | per-(variant × blendshape) archetype blendshapes (expression; zero at rest) | `expression_morphs` | §6.16.19 |
| 21 | `[179 × 13]` f32 per-(joint × region) mixing weights, rows sum to 1 | `joint_weights` | §6.7 |
| 22 | `[13][5,584]` f32 region masks (region-major) + 8 trailing bytes; per-vertex partition of unity | `region_weights` | — |
| 31 | 61 × 40-byte variant headers + packed joint-delta payloads (expression) | `variant_records`, `variant_{main,secondary}_deltas` | §5, §6.6 |

Slots 3, 5, 7, 9, 10, 14, 20 and 23–30 (hash tables, ethnicity / build name
pools, per-variant scalars) are not used by the exporter. Slot 16 is null.

## 2. `ScatteredDataInterpolation::ComputeControlPointFromWeights`

`FUN_143ff6710` (May build) / `FUN_1444e46d0` (4.8). Given a per-vertex blob
with `double[3]` control points (one per variant, `+0x98/+0xa0`) and
`byte[4]` tetrahedra (`+0x30/+0x38`), it keeps the top-4 weights, finds the
tetrahedron whose four vertices are those variants (fatal error
`"No tetrahedron exists that contains all variants in dna matrix for this
vertex"` otherwise) and solves for the interpolated point by Cramer's rule
four times, averaging the solutions. The tetrahedralisation is computed at
runtime per vertex (§4), not stored.

In 4.8 the caller is the per-vertex character-build driver `FUN_144589d40`
(hosted by `SCActorDNAComponent`, run once per character build), its
weights read from the DNA matrix and its control points fetched per vertex
from slot 18 (`GetVariantVertices`, §6.16.24). An older xref walk on the May
build had reached it only from the face-sculpting UI. The exporter does not
port this solver: it applies the DNA-matrix-weighted sum of the slot-18
per-variant deltas directly, which matches the in-game head shape
(§6.16.24).

## 3. CHF `DnaBlend` → DNA weight buffer

Engine: `CCharacterCustomizerComponent::HandleDNABlending` (`0x144053610`
May / `0x1445415a0` 4.8) accumulates into a
`[num_variants × mesh_bpp]` f64 buffer (`customizer + 0x2da0`, `+0x2dd0` in
4.8), clamping each cell to `[0, 1]`; `UpdateDNAMatrix` (`0x14411f9c0` /
`0x14462d5d0`) casts to f32 (`vcvtpd2ps`), prunes to the top **K = 4**
weights and re-normalises before building the sparse pair list.

As implemented (`dna_matrix::chf_weights`, `dna_matrix::top_k`):

```text
cell(head_id, face_part) += value / 65535       (clamp to [0,1]; value 0 skipped)
column face_part normalised to sum 1            (convex blend of heads per part)
top-4 prune per face-part column, then re-normalise the column
```

- Column index = the `FacePart` enum value (0..=12: EyebrowLeft … Neck),
  i.e. the 13 DNA regions (`ECharacterCustomizerDNARegion`). There is no
  3 → 13 expansion (§6.8).
- Row index = `DnaBlend.head_id`, the `.dna` variant index directly
  (variant 0 is the protos base).
- Prune and normalisation run per face-part **column**: the four `value`s of
  one face part sum to ~65535. An earlier per-variant-row reading scrambled
  the head ratios (corrected in Pass 19g/19h).
- The engine's per-customizer animated scale and ±1 sign (`+0x2b54`,
  `+0x2b5c`) are taken as 1.0 / +1 for a static export.

## 4. TetrahedronGenerator

CVar registered at `FUN_140087990`: `None / Default / CGAL / CDT / fTet`;
`Default` (`FUN_14400cc90`) is a Quickhull-style hull plus star
tetrahedralisation, run per vertex at character build. Not ported (see §2).

## 5. Slot 31 payload format

Slot 31 holds `num_variants` 40-byte headers (layout in §6.6.3 below / `SPEC.md`),
each pointing at a MAIN and a SECONDARY payload. A payload is
`num_*_records` variable-stride records:

```text
u16 output_idx      // output channel = joint * 3 + axis (§6.9)
u16 N
u32 packed[N]       // low 9 bits = blendshape index, rest = signed coef
```

Same format as RigLogic Section D: `index = packed & 0x1FF`,
`coef = (i32)packed >> 9`, dequantised by `max_packed = 2^22 − 1`
(`packed_value_bit_width = 22`). Variant 0's header matches the RigLogic
Section A scales (`primary_delta_scale = 0.067676`,
`secondary_delta_scale = 1.616942`); every variant carries its own scales.

## 6. End-to-end engine path

```text
CHF DnaBlend ──HandleDNABlending──▶ weights[var × 13] (f64, clamp + normalise)
            ──UpdateDNAMatrix────▶ f32, top-4 prune, sparse (variant, weight) pairs
            ──SetDNAMatrix────────▶ dense matrix [num_variants × 13], active-variant set
            ──ExecuteHuman────────▶ per-joint translation + Euler rotation deltas
                                     (slot 31 × slot 21 × matrix × _Dst activations)
```

`CFaceRigLogic::ExecuteFaceRigLogic` writes a `SkeletonPoseInt` (joint
transforms only — no vertex output). Vertex-level effects reach the mesh
through skinning, or on the GPU (`Deformer_BShapes_DQSkinning`: `BShapesGpu`
× slot 17 × `_Dst`). The per-character neutral shape (slot 18) is applied
at character build (§6.16.24).

Addresses (string-anchored):

| Function | May 2026 | 4.8_live |
|---|---|---|
| `CCharacterCustomizerComponent::HandleDNABlending` | `0x144053610` | `0x1445415a0` |
| `CCharacterCustomizerComponent::UpdateDNAMatrix` | `0x14411f9c0` | `0x14462d5d0` |
| `CFaceRigLogic::SetDNAMatrix` | `0x146735b70` | `0x146e71f80` |
| `CFaceRigLogic::ExecuteFaceRigLogic` | `0x1466deab0` | `0x146e795a0` |
| `CFaceRigLogic::ExecuteHuman` | `0x1466e0aa0` | `0x146e19c40` |
| `CFaceRigLogic::InitFaceRigLogic` | — | `0x146e54af0` |
| `CFaceRigLogic::SetRuntimeRigInputParams` | — | `0x146e736e0` |
| `DNAFile::Load` / `LoadAllLODs` | `0x14142d280` / `0x14142d720` | `0x14173fdc0` / `0x141740260` |
| `Protos::GetMeshById` | `0x14142b9a0` | `0x14173e4d0` |
| `.dna` pointer fixup | `0x14142a990` | — |
| `CAttachmentManager::GetVariantVertices` worker (slot 18) | — | `0x146c67950` |

## 6.5. DNA-matrix construction

### 6.5.4. `DNAMatrix.vtable[+0x10]`

The step between `UpdateDNAMatrix` and `SetDNAMatrix` only sorts the
`(variant_idx, weight)` pairs and merges duplicates; it does not expand
payloads into blendshapes (Pass 4 correction of Pass 3). The per-blendshape
unpack happens in `ExecuteHuman`. The record's `output_idx` (open question
O12 when this was written) is the output channel `joint * 3 + axis`
(§6.9); the master payloads cover joints 0–178 × xyz.

## 6.6. Slot 31 is `mesh + 0x138` (Pass 4)

`ExecuteHuman` reads `*(mesh + 0x138)` as a table of `num_variants` (`mesh +
0x2a`) entries of stride `0x28`; `FUN_14142a990` special-cases the same
slot, rebasing the `+0x18`/`+0x20` payload offsets of each of the
`num_variants` entries. So the "per-blendshape table" Pass 3 described is the
slot-31 **per-variant** header table, and the dense DNA matrix is
`[num_variants × mesh_bpp]`, not `[num_blendshapes × …]`.

### 6.6.3. Slot-31 header layout

```text
+0x00 u32 num_main_records       (variant 0: 517)
+0x04 u32 num_secondary_records  (variant 0: 528)
+0x08 f64 primary_delta_scale    (translations)
+0x10 f64 secondary_delta_scale  (rotations)
+0x18 u64 main payload offset    (file-relative; pointer at runtime)
+0x20 u64 secondary payload offset
```

Header bytes used by the morph (Pass 6, `.dna` header = `ProtosMesh`):
`+0x26` u8 `num_face_parts` (3: head/eyes/teeth labels, not a matrix
dimension), `+0x27` u8 K = 4, `+0x28` u8 `mesh_blends_per_pool` = 13,
`+0x29` u8 local region count (13 head; 2 for the eyes/teeth `.dna`),
`+0x2a` u16 `num_variants`, `+0x2e` u16 `num_joints` (179).

### 6.6.5. Pipeline summary

`SetDNAMatrix` folds the pair list into the dense matrix
`matrix[variant × mesh_bpp + region]` and stores the sorted set of active
variants. `dna_matrix::accumulator::build_dna_matrix` produces the same
`DnaMatrix2D` from the pruned weight buffer. `ExecuteHuman` then folds it
with slot 21 per joint (§6.7) and unpacks the slot-31 payloads (§6.9.4).
A `+0xd0` per-row-sum buffer built by `SetDNAMatrix` is not read by
`ExecuteHuman`.

## 6.7. Slot 21: per-(joint × region) weights (Pass 5)

`mesh + 0xe8` is slot 21: `[num_joints × mesh_bpp]` f32 (179 × 13 = 9,308
bytes + 4 pad = the 9,312-byte slot). 178 of 179 rows sum to exactly 1.0;
row 0 (facial root) is all zero. `ExecuteHuman` reads
`slot21[mesh_bpp * joint + j] * matrix[mesh_bpp * variant + j]` in its
innermost loop:

```text
fold[joint, variant] = Σ_j slot21[joint][j] · matrix[variant][j]
```

## 6.8. Customizer and mesh share the region dimension (Pass 6)

`customizer + 0x28f1` is written at character init from `mesh[+0x28]`
(`FUN_144092b40`, both writers), and the DNAMatrix layout factory
(`FUN_14142a1d0`) copies the same byte into the layout `+0x10` that
`UpdateDNAMatrix` uses as its stride. So the customizer weight buffer and
the dense matrix both have **13** columns; there is no expansion from the
CHF's 3 face-part labels. A CHF `(FacePart, slot)` blend writes the cell
`(head_id, FacePart)` directly (§3).

## 6.9. `ExecuteHuman` output (Pass 7)

`ExecuteHuman` outputs per-joint **translation** deltas (primary payloads,
added to the bind translation) and per-joint **Euler XYZ rotation** deltas
(secondary payloads, converted to a quaternion and multiplied into the bind
rotation), written into `SkeletonPoseInt`. The `+0x150`/`+0x158` packed
lists are a cache written and read inside the same call.

### 6.9.4. Algorithm

Per output channel `c = joint * 3 + axis`, over the active variants `v`:

```text
tx[joint][axis] = Σ_v Σ_{(bs, coef) in main record c of v}
                    coef · primary_delta_scale[v] · fold[joint, v] · _Dst[bs] / max_packed
rx[joint][axis] = same with secondary records and secondary_delta_scale[v]
```

(`max_packed = 2^22 − 1`; the scale is the raw per-variant value, §6.10.)
Implemented by `morphing::compute_joint_deltas` → `JointDeltas {
translation, rotation_eu }`.

### 6.9.5. Applying the deltas

`final.translation = bind.translation + tx`;
`final.rotation = bind.rotation * euler_xyz_to_quat(rx)` (standard XYZ
half-angle product, matching the sin/cos chain at `0x1466e4dbf`).
Implemented by `apply::apply_deltas`, which maps the 179 RigLogic joints to
the head `.skin` bones by CRC32 of the joint name.

## 6.10. Output scaling (Pass 8)

Each packed coefficient is multiplied by the **raw** per-variant
`primary_delta_scale` / `secondary_delta_scale` divided by `max_packed`
(`fVar83 = slot31[v].primary_delta_scale / dVar90` with
`dVar90 = 2^22 − 1`). The `max_pri_scale` / `max_sec_scale` values the
engine stores at `+0x140` only quantise and dequantise the `+0x150/+0x158`
cache and cancel out; normalising by them inflated the deltas ~60×.

## 6.11. The `_Dst` activation buffer (Pass 9)

### 6.11.1. Allocation and population

`_Dst` is a zeroed `[f32; num_blendshapes]` stack buffer in `ExecuteHuman`.
It is filled by one of two branches (§6.15.3): FaceAnim playback, or a
copy of `num_appearance` external control values. Then `_Dst[0] = 1.0`
(`CTL_PSD`) unconditionally, and Section C writes each corrective
`_Dst[num_appearance + i]` as the product of five appearance entries
(`riglogic::compose_correctives`). An eye look-at block can also write a
few eye-control slots; it is not part of the identity path.

### 6.11.2. Rest pose

With no animation and no external controls, `_Dst = [1.0, 0, …, 0]`: every
Section C row references a zero entry, so all correctives are 0, and only
the 231 of 2,365,201 slot-31 packed entries with `bs == 0` could fire (none
land on a joint with non-zero fold). **Slot 31 contributes nothing at rest;
it is an expression modulator, not the per-character neutral.**
`controls::ControlInputs::rest_pose` reproduces this baseline.

## 6.13. `BShapesGpu` (Pass 14)

The head `.skinm` chunk `0x57A38888` holds 495 blendshapes as per-vertex
sparse deltas: a 32-byte header, a per-vertex CSR opcode table, per-blendshape
`(a, b)` Vec3 pairs, a vertex × blendshape participation bitfield and 3-byte
opcodes. `delta(V, B) = byte_unsigned · a[B] + b[B]` per axis. Slots with
`b ≈ (9, 9, 9)` and `a ≈ 4e-8` are reserved and emit zero. The engine
evaluates this on the GPU; the formula was validated visually. Decoder:
`character::blendshapes`.

## 6.14. Slot 17: blendshape attenuation (Pass 15)

```text
final_delta(V, B) = bshapes_delta(V, B) · (1 − slot17[V × stride + B] / 255)
```

0 = fully active, 255 = masked. The `.dna` is rig-global (§6.16.1), so this
is one mask for every character of the rig, not identity. The CPU uploads
slot 17 once; the GPU deformer applies it. Empirical (validated visually
against in-game expressions); `(1 − b/256)` vs `/255` is within noise.

### 6.14.5. Row stride and the trailing byte

The male master's stride is `num_blendshapes + 1 = 496`: one trailing byte
per vertex that the mask formula does not use (its meaning is unknown). The
female master's stride is 392, fewer than its blendshape count; blendshapes
past `stride − 1` pass through unmodulated. `blendshapes.rs` derives the
stride from the buffer length.

## 6.15. Where `_Dst` comes from (Pass 17)

### 6.15.3. The two input branches

`ExecuteHuman` branches on `CFaceRigLogic + 0x10`:

- `0`: **FaceAnim playback** — samples the `FaceAnimDef` clips at the
  current time into `_Dst` (decode in §6.16.15).
- non-zero: **external controls** — copies `num_appearance` floats from
  `*(this + 0x18)`.

Only `SetRuntimeRigInputParams` (vtable `+0x48`) and
`UseRuntimeRigInputParams` (`+0x50`) write those fields. The DNA-matrix path
(`SetDNAMatrix`) and the activation path are independent.
`character::controls` builds the external-controls vector from named
controls.

### 6.15.5. The customizer uses the FaceAnim branch

`InitFaceRigLogic` never enables external controls and the customizer's
slider flow never calls `SetRuntimeRigInputParams`, so in the customizer
`_Dst` comes from an idle / preview FaceAnim. That is why slider drags move
the face there, while a rest-pose export (`_Dst = [1, 0, …]`) shows no
slot-31 motion.

## 6.16. Per-character identity (Pass 19)

### 6.16.1. The `.dna` is rig-global

Assets resolve by `gender_hash` only (`character::identity`); every male
character loads the same master `.dna`. Per-character data is the CHF's DNA
matrix; the rig-global slots are blended by it.

### 6.16.6. Slot 15: the per-character neutral skeleton

Slot 15 holds each variant's parent-local joint translations. The
character's neutral skeleton is their DNA-matrix blend, relative to
variant 0 (the protos base = the `.skin` bind pose):

```text
translation[j] = Σ_v fold[j, v] · (slot15[v][j] − slot15[0][j])     (fold: §6.7)
```

(`morphing::compute_neutral_joint_offsets`.) The offsets are applied as a
**re-bind** (`apply::rebind_skeleton`: new rest pose, world transforms and
inverse binds recomputed by FK), so the joints move without deforming the
head mesh; expressions then pivot about the character's own joints, and the
eyes and teeth follow. Slot 15 alone is only joint-level (~0.65 cm for a
typical blend, up to ~1.5 cm for one archetype). The bulk head shape is
slot 18 (§6.16.24). The face-rig functions never read slot 15; the engine's
consumer was not located.

### 6.16.15. FaceAnim (`.caf`) decode (Pass 19f)

From `ExecuteHuman`'s animation branch (4.8, `FUN_146e19c40`): a
`FaceAnimDef` is a list of 12-byte clip records (`u32` anim CRC, `u32` clip
id, `f32` clip weight). Each `#caf` controller node (24 bytes, the value
controller is the second half) has a key count, a time format (`'@'` u8,
`'A'` u16, `'B'` bitstream of keyed frames) and a value format (`0xC0` f32,
`0xC1`/`0xC2` `u16 · scale + bias`). Sampling binary-searches the key times
and lerps; `_Dst[ctrl] += value · clip_weight`. Ported as
`face_anim::CafClip` (the clip weight is the export's intensity).

### 6.16.19. Slot 19: archetype blendshapes (expression only)

Slot 19 is `61 × 495` cells, variant-major; cell `(v, b)` is archetype `v`'s
own blendshape `b` (variant 0 = the base head's `BShapesGpu[b]`). Byte layout
(32-byte cell records + `[stride, qx, qy, qz]` entries, delta
`bias + q · scale`) is in `SPEC.md` ("Slot 19"); an earlier u24-weight
reading was wrong and was corrected against ground truth. Blendshape 0
(`CTL_PSD`) is empty for every variant, so **slot 19 is identically zero at
the rest pose**: it is per-character expression, not identity. A
character's blendshape `b` is the region-masked archetype blend
(`character::expression`):

```text
blend_b[vtx] = Σ_v fold[vtx][v] · T_v[b][vtx]
fold[vtx][v] = Σ_r slot22[r][vtx] · matrix[v][r]   (normalised to stay convex)
```

Expression clips use these blendshapes in place of the base head's.

### 6.16.24. Slot 18: the per-character neutral head shape (Pass 19N)

Decoded from the `GetVariantVertices` worker (`FUN_146c67950`, IDA
decompile) and validated byte-for-byte; an earlier "13-bit packed" reading
was the sparse set bits of the bitmask. With
`num_controls = local_regions × (num_variants − 1)` (780 for the head) and
`mask_words = ⌈num_controls / 32⌉`:

```text
[ num_vertices × mask_words u32 ]   per-vertex bitmask (bit c = control c active)
[ num_vertices u32 ]                per-vertex cursor into the code stream
[ num_controls × 0x20 ]             per control: f32 scale xyz @+0, bias xyz @+0x10
( 4-byte gap )
[ u8 codes ]                        3 bytes per active (vertex, control)
```

Invariant: `cursor[V+1] − cursor[V] == popcount(bitmask[V])`.

```text
delta[V] = Σ_{c set in bitmask[V]} coef[c] · (code · scale[c] + bias[c])   per axis
c        = (variant − 1) · local_regions + local_region
coef[c]  = matrix[variant][global_region(local_region)]
```

The weight is the DNA archetype blend with no activation gate, so this is
non-zero at rest: it is the per-character neutral. Implemented by
`starbreaker_dna::NeutralMorphs` and `morphing::compute_neutral_vertex_deltas`.
Validated in game (head_id → geometry, the four-head blend and
region → anatomy all match) and against the archetype meshes (one variant
with all regions on equals `<archetype>_head.skinm` − base within 0.2 mm).

Conclusions that shape the export:

- The head mesh is `base + slot-18 deltas`, skinned to the slot-15 re-bound
  skeleton (§6.16.6). Skinning slot 18's shape by a displaced but not
  re-bound skeleton double-deforms it.
- Deltas are applied **as authored**. The archetype codes carry a small
  left/right asymmetry (~7 %) and the engine does not mirror-average them;
  mirroring also smeared one-sided DNA edits onto both halves
  (test `asymmetric_dna_edit_stays_one_sided`).
- The eyes and teeth have their own `.dna` (two local regions each, mapped
  by slot 12 to EYE_L/EYE_R and NOSE_C/MOUTH_C); their slot 18 gives each
  archetype's own eyes and teeth. Wrap-deformed parts (eye details, CHF hair
  and facial-hair items) follow the slot-18 head surface
  (`character::wrap::HeadDeltaField`).
- Normals are carried onto the morphed shape (`normals::transfer_normals`).

## Open questions

- The engine's consumer of slot 15 (the skeleton blend is reproduced but
  its call site was never found).
- The trailing byte of each slot-17 row and the female slot-17 stride
  (§6.14.5).
- Per-vertex mask semantics in slot 17 are empirical; the GPU deformer
  (`Deformer_BShapes_DQSkinning`) was never decoded.
- Slots 3, 9, 10, 14, 25, 28–30 (per-variant scalars and tables) are
  unused and undecoded.
