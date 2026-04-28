# Star Citizen Animation Formats — A Practical Reference

A code-grounded guide for extracting and reconstructing Star Citizen
character and ship animations in Blender, 3ds Max, Maya, or any DCC
that consumes glTF / FBX. Every claim in this document is backed by
the production parser in
[`crates/starbreaker-3d/`](../crates/starbreaker-3d/)
and validated against live Star Citizen game data.

> **Status.** This guide describes the on-disk formats, the data flow
> from `.p4k` archive to runtime, the coordinate-system conventions, and
> the practical steps to import animation into a third-party DCC. It
> does not cover the runtime CryEngine "Mannequin" fragment system,
> which composes higher-level state transitions on top of the clips
> documented here (see [§13 Limitations](#13-limitations)).

---

## Contents

1. [Introduction](#1-introduction)
2. [Asset-graph overview](#2-asset-graph-overview)
3. [The IVO container](#3-the-ivo-container)
4. [`.chr` — Skeleton](#4-chr--skeleton)
5. [`.skin` / `.cgf` — Mesh and skinning data](#5-skin--cgf--mesh-and-skinning-data)
6. [NMC — Node-Mesh-Combo table](#6-nmc--node-mesh-combo-table)
7. [`.cdf` — Entity composition](#7-cdf--entity-composition)
8. [`.chrparams` — Clip name → file mapping](#8-chrparams--clip-name--file-mapping)
9. [`.dba` and `.caf` — Animation database / single clip](#9-dba-and-caf--animation-database--single-clip)
10. [Time-key formats](#10-time-key-formats)
11. [Quaternion compression](#11-quaternion-compression)
12. [Position compression](#12-position-compression)
13. [Coordinate-system mapping](#13-coordinate-system-mapping)
14. [Reconstructing animations in a DCC](#14-reconstructing-animations-in-a-dcc)
15. [Limitations and out-of-scope content](#15-limitations-and-out-of-scope-content)
16. [Source provenance](#16-source-provenance)

---

## 1. Introduction

Star Citizen ships and characters are authored in CryEngine (Lumberyard
fork) and packaged into `Data.p4k`, a custom ZIP-like archive.
Animation data is split across several file types, none of which have
public specifications. The formats described here are the result of
empirical reverse engineering, cross-validated against:

- the production Rust parser at
  [`crates/starbreaker-3d/`](../crates/starbreaker-3d/),
- the Ghidra-confirmed bit layout of the 48-bit "smallest three"
  quaternion decoder,
- end-to-end validation by importing decoded animations into Blender
  and visually matching in-game footage.

The guide covers eight on-disk formats:

| Extension     | Purpose                                        |
| ------------- | ---------------------------------------------- |
| `.cdf`        | Entity composition (XML, references CHR + SKIN attachments) |
| `.chrparams`  | Animation event-name → CAF filename map (XML) |
| `.chr`        | Skeleton with bone hierarchy and bind transforms |
| `.skin`       | Skinned mesh with per-vertex bone weights      |
| `.cgf`        | Static (un-skinned) mesh                       |
| `.dba`        | Animation **D**ata**b**ase **a**rchive — many clips for one rig |
| `.caf`        | Single animation clip outside a DBA            |
| `.chr.mtl` / `.skin.mtl` | Material file (CryXmlB)             |

All binary formats are little-endian. Multi-byte numeric fields are
unaligned and may straddle struct boundaries — readers must treat the
on-disk layout as packed.

---

## 2. Asset-graph overview

Given an entity name (e.g. `rsi_scorpius`), the dependency graph is:

```text
DataCore record (entity)
        │
        ├── geometry: <something>.cdf   ← entry point
        │
        ▼
   .cdf  (XML)
        │
        ├── <Model File="...chr"/>      ← skeleton
        ├── <Attachment Type="CA_SKIN"
        │           Binding="...skin"/> ← skinned mesh chunks
        └── <Attachment Type="CA_BONE"
                    Binding="...cgf"/>  ← rigid attachments
        │
        ▼
   .chr  (IVO + CryCh)
        │
        ├── ChrParams sibling: <name>.chrparams  (XML)
        │       │
        │       └── <Animation name="open"
        │             path="canopy_open.caf"/>   ← clip catalogue
        │
        └── $TracksDatabase: <Ship>.dba  ← packs many .caf clips
        │
        ▼
   .dba / .caf  (IVO)
        │
        └── per-bone rotation + position channels
```

This guide uses the **RSI Scorpius** as a recurring worked example
because it exercises every animation feature: a 33-bone landing-gear
rig, multi-stage canopy, X-shape wing deploy with per-channel
asymmetric keyframing, and inline IK helpers.

---

## 3. The IVO container

Most binary files (CHR, SKIN, CGF, DBA, CAF) share a common outer
wrapper called **IVO** (the file's internal magic `#ivo`). The
container is a simple typed-chunk table:

| Offset | Size | Field                                              |
| ------ | ---- | -------------------------------------------------- |
| 0x00   | 4    | Magic `'#ivo'`                                     |
| 0x04   | 4    | Version (commonly `0x900` for animation)           |
| 0x08   | 4    | `chunk_count` (number of typed chunks)             |
| 0x0C   | 4    | `chunk_table_offset` (typically `0x10`)            |

The chunk table has one 16-byte entry per chunk:

| Offset | Size | Field                              |
| ------ | ---- | ---------------------------------- |
| 0x00   | 4    | `chunk_type` (FOURCC-style u32)    |
| 0x04   | 4    | `version`                          |
| 0x08   | 8    | `file_offset` (absolute, u64)      |

Chunks themselves are packed sequentially after the table and may
contain their own inner sub-magic and length prefixes.

### 3.1 Recognised chunk types

Verified in
[`crates/starbreaker-3d/src/animation.rs:117-122`](../crates/starbreaker-3d/src/animation.rs)
and [`crates/starbreaker-chunks/src/known_types.rs`](../crates/starbreaker-chunks/src/known_types.rs):

| Const                  | Value         | Used by                               |
| ---------------------- | ------------- | ------------------------------------- |
| `DB_DATA`              | `0x194FBC50`  | DBA — packed clip data                |
| `DBA` (DBA_META)       | `0xF7351608`  | DBA — clip-name table + metadata      |
| `CAF_DATA`             | `0xA9496CB5`  | CAF — single-clip data                |
| `ANIM_INFO`            | `0x4733C6ED`  | CAF / DBA — fps + flags               |
| `COMPILED_BONES`       | `0xC201973C`  | CHR (v901) — skeleton                 |
| `COMPILED_BONES_IVO320`| `0xC2011111`  | CHR (v900) — skeleton                 |
| `NODE_MESH_COMBOS`     | `0x70697FDA`  | CGF / SKIN — NMC node table           |
| `IVO_SKIN2`            | `0xB8757777`  | SKIN — vertex / index / bone-map data |
| `MESH_IVO320`          | `0x92914444`  | CGF — static mesh data                |
| `MTL_NAME_IVO320`      | `0x83353333`  | CGF / SKIN — material name slot       |

The chunk's `version` field controls inner-layout selection (e.g. CHR
v900 vs v901, see §4.2).

---

## 4. `.chr` — Skeleton

A `.chr` file declares the bone hierarchy and bind pose used by all
animation clips referencing it via `.chrparams`. It is an IVO file
whose key chunk is `COMPILED_BONES` (or `COMPILED_BONES_IVO320`).

### 4.1 The `Bone` domain type

After parsing, each bone is exposed as
([`crates/starbreaker-3d/src/skeleton.rs`](../crates/starbreaker-3d/src/skeleton.rs)):

```rust
pub struct Bone {
    pub name: String,
    pub parent_index: Option<u16>,
    pub object_node_index: Option<u16>,

    /// Parent-relative position [x, y, z]
    pub local_position: [f32; 3],
    /// Parent-relative rotation quaternion [w, x, y, z]
    pub local_rotation: [f32; 4],

    /// World-space position [x, y, z] in CryEngine convention
    pub world_position: [f32; 3],
    /// World-space rotation quaternion [w, x, y, z] in CryEngine convention
    pub world_rotation: [f32; 4],
}
```

Bone names are stored elsewhere in the IVO file (typically a
NUL-terminated string table) and bound to entries by index.

### 4.2 v900 entry layout (chunk `0xC2011111`)

68 bytes per bone, packed:

| Offset | Size | Field            | Notes                              |
| ------ | ---- | ---------------- | ---------------------------------- |
| 0x00   | 4    | `controller_id`  | Used by some legacy tools          |
| 0x04   | 4    | `limb_id`        | IK chain identifier                |
| 0x08   | 4    | `parent_index`   | i32; negative ⇒ root               |
| 0x0C   | 28   | `relative` TRS   | Local TRS                          |
| 0x28   | 28   | `world` TRS      | World TRS                          |

Each 28-byte `RawQuatTrans`:

| Offset | Size | Field | Value     |
| ------ | ---- | ----- | --------- |
| 0x00   | 16   | quat  | `qx, qy, qz, qw` (XYZW order, f32) |
| 0x10   | 12   | trans | `tx, ty, tz` (f32)                 |

> **Quaternion field order in CHR is `xyzw`**, but the parsed `Bone`
> exposes `wxyz` (with the `w` first) to match Blender's
> `rotation_quaternion` convention. The conversion is purely
> rearrangement — no axis remap happens at this stage.

### 4.3 v901 entry layout (chunk `0xC201973C`)

A more compact version where transforms are stored in separate parallel
blocks; the per-bone entry is 16 bytes (IDs + parent + transform-block
indices). The reader reassembles the same `Bone` struct.

### 4.4 Use as bind pose

The skeleton's `world_position` and `world_rotation` constitute the
**bind pose**. All animation clips assume the rig sits in this pose at
clip-frame 0 unless the clip explicitly authors a different starting
transform. Re-targeting tools should use the CHR bind pose as the rest
position when constructing a Blender armature, an FBX skeleton, or an
NLA neutral.

---

## 5. `.skin` / `.cgf` — Mesh and skinning data

A `.skin` file is the on-disk container for a skinned mesh chunk and a
companion bone-map. A `.cgf` file is the equivalent for static
(non-skinned) geometry. Both share the IVO outer wrapper and largely
the same internal mesh chunks; they differ mainly in whether a bone-map
chunk is present.

The parsed in-memory representation
([`crates/starbreaker-3d/src/types.rs`](../crates/starbreaker-3d/src/types.rs)):

```rust
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub secondary_uvs: Option<Vec<[f32; 2]>>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<SubMesh>,
    pub model_min: [f32; 3],
    pub model_max: [f32; 3],
    // ...
}

pub struct SubMesh {
    pub material_name: Option<String>,
    pub material_id: u32,
    pub first_index: u32,
    pub num_indices: u32,
    pub first_vertex: u32,
    pub num_vertices: u32,
    pub node_parent_index: u16,   // index into NMC / bone table
}
```

### 5.1 BoneMap12 — per-vertex weights

When the chunk `IVOBONEMAP` (`0x677C7B23`) is present, each vertex
carries a 12-byte `BoneMap12` record:

| Offset | Size | Field                          |
| ------ | ---- | ------------------------------ |
| 0x00   | 4×2  | `joint_indices: [u16; 4]`      |
| 0x08   | 4×1  | `weights: [u8; 4]` (sum to 255)|

The `dominant_joint()` helper returns the bone-index with the highest
weight.

### 5.2 Position quantisation

When positions are stored quantised they are 16-bit packed and
dequantised as **signed-normalised** values centred on the mesh-info's
**second** bounding box (`min_bound` / `max_bound`, not the model-space
`model_min` / `model_max` extent which is used for the NMC scene-graph
transforms).

The exact formula
([`crates/starbreaker-3d/src/dequant.rs::dequantize_position`](../crates/starbreaker-3d/src/dequant.rs)):

```text
for axis in 0..3:
    snorm        = (raw_u16 reinterpreted as i16) / 32767.0   // in [-1, +1]
    half_extent  = max(  (max_bound[axis] - min_bound[axis]) / 2  ,  1.0  )
    center       = (max_bound[axis] + min_bound[axis]) / 2
    position[axis] = snorm * half_extent + center
```

The `max(half_extent, 1.0)` clamp matches the legacy CryEngine-Converter
behaviour: for axes where the bbox extent is below 2 m, the half-extent
is pinned to 1 m so the SNorm value passes through unscaled. This is
rare in production data but must be honoured to match the in-game
geometry exactly.

Both bounding boxes are present in the mesh-info chunk and are always
populated. Mixing them up (using `model_min/max` for dequantisation,
or interpreting the raw values as unsigned u16 anchored at min) appears
to work for many meshes but produces silent positional drift where the
two bboxes diverge.

### 5.3 Rigid-vs-soft submesh resolution

CryEngine's `.skin` files conflate two different deformation models in
a single file:

- **Soft-skinned submeshes**, where a single triangle's three vertices
  span multiple bones with weighted blending (true GPU skinning).
- **Rigid submeshes**, where every vertex of a submesh ultimately
  belongs to one bone — the geometry is a rigid attachment (e.g. a
  landing-gear strut, a piston, a canopy half) that should follow that
  bone with no per-vertex blending.

Most exporters need to emit these two cases differently: soft-skinned
submeshes become a glTF `skin` accessor with `JOINTS_0` / `WEIGHTS_0`,
while rigid submeshes are most cleanly emitted as separate meshes
parented to the bone they belong to (no skinning required).

The classifier in
[`crates/starbreaker-3d/src/types.rs::split_rigid_weighted_submeshes`](../crates/starbreaker-3d/src/types.rs)
walks each submesh's triangles, looks up each vertex's dominant joint
via `BoneMap12`, and groups triangles by joint. The decision rule:

| Condition                                       | Outcome                                        |
| ----------------------------------------------- | ---------------------------------------------- |
| `bone_maps == None`                             | Treat as static; keep `node_parent_index = 0` |
| `rigid_ratio < 0.9`                             | Soft-skinned; keep original (emit as `skin`)  |
| `grouped_indices.len() ≥ 1` and `rigid_ratio ≥ 0.9` | Split per-joint; reassign `node_parent_index = joint` for each piece |

`rigid_ratio` is the fraction of triangles where all three vertices
share the same dominant joint. The 0.9 threshold tolerates a few
mixed-bone triangles at seams without misclassifying.

After classification, rigid pieces are vertex-rebased into
their owning bone's local frame (so glTF emission can attach the mesh
directly under the bone node and inherit its world transform). The
in-memory operation is `bone_inverse * world_position` per vertex, with
normals/tangents rotated by the inverse bone rotation.

---

## 6. NMC — Node-Mesh-Combo table

The `NODE_MESH_COMBOS` chunk (`0x70697FDA`) carries a per-node table
that any consumer must read to assemble the final scene:

```rust
pub struct NmcNode {
    pub name: String,
    pub parent_index: Option<u16>,    // None for root (0xFFFF on disk)
    pub world_to_bone: [[f32; 4]; 3], // 3x4 row-major
    pub bone_to_world: [[f32; 4]; 3], // 3x4 row-major
    pub scale: [f32; 3],
    pub geometry_type: u16,           // 0 = GEOM, 3 = HELP2, ...
    pub properties: HashMap<String, String>,
}
```

The 3×4 affine matrices `world_to_bone` and `bone_to_world` are stored
explicitly (not synthesised from a quat+translation). For a CGF
without skin weights, each submesh's `node_parent_index` indexes into
this table to determine which node owns the geometry.

When a `.skin` is loaded standalone (no companion CHR), an NMC table
can be **synthesised** from the inline `CompiledBones` chunk; this
lets static-skin assets export cleanly without an external skeleton.

---

## 7. `.cdf` — Entity composition

A `.cdf` is a CryXmlB-encoded XML file that lists the skeleton plus
all attached parts:

```xml
<CharacterDefinition>
  <Model File="objects/spaceships/ships/rsi/scorpius/exterior/rsi_scorpius.chr"
         Material="objects/.../rsi_scorpius_body.mtl"/>
  <AttachmentList>
    <Attachment Type="CA_SKIN"
                AName="LandingGear_Front"
                Binding="objects/.../rsi_scorpius_landinggear_front_skin.skin"
                ...
    />
    <Attachment Type="CA_BONE"
                AName="MissileBay_Left"
                Binding="objects/.../missilebay.cgf"
                BoneName="hardpoint_missilebay_left"
                Position="0 0 0"
                Rotation="1 0 0 0"
    />
    <!-- ... -->
  </AttachmentList>
</CharacterDefinition>
```

Two attachment types are common:

- **`CA_SKIN`** — a `.skin` whose vertices share the parent CHR's bones
  (skinned attachment, follows the skeleton automatically).
- **`CA_BONE`** — a `.cgf` (rigid) anchored to a named bone via
  `BoneName`, with an optional position + quaternion offset.

The exporter resolves all attachments, fetches each `.skin` / `.cgf`,
and emits a single combined glTF tree with the attachments parented
correctly.

---

## 8. `.chrparams` — Clip name → file mapping

A `.chrparams` is a CryXmlB XML sibling of the `.chr` that names every
animation event the rig supports and the `.caf` file (or `.dba` clip
name) that implements it:

```xml
<Params>
  <AnimationList>
    <Animation name="#filepath"
               path="Animations/Spaceships/Ships/RSI/Scorpius"/>
    <Animation name="$TracksDatabase"
               path="Animations/Spaceships/Ships/RSI/Scorpius.dba"/>
    <Animation name="canopy_open"     path="canopy_open.caf"/>
    <Animation name="canopy_close"    path="canopy_close.caf"/>
    <Animation name="wings_deploy"    path="rsi_Scorpius_wings_deploy.caf"/>
    <Animation name="landing_gear_extend"
               path="rsi_Scorpius_lg_deploy_r.caf"/>
    <!-- ... -->
  </AnimationList>
</Params>
```

The two pseudo-keys deserve attention:

- **`#filepath`** — base directory all relative `path` values resolve
  against.
- **`$TracksDatabase`** — the `.dba` whose internal clip table can
  satisfy any name not found as a standalone `.caf`. When a clip name
  resolves to a `.caf` filename, look first inside the DBA's clip
  catalogue (§9.2) before reading a separate file.
- **`$AnimEventDatabase`** *(optional)* — points to a separate
  `.animevents` CryXmlB document containing per-clip event hints
  (`bone="…"` and `parameter="…"` attributes attached to `<event>`
  elements inside `<animation animation="…caf"/>` blocks). These
  hints are advisory metadata, not required for reconstruction (see
  §15.2).

`.chrparams` does **not** contain blend curves, IK targets, drivers, or
constraints — clips are self-sufficient.

---

## 9. `.dba` and `.caf` — Animation database / single clip

A `.dba` packs many clips into one file; a `.caf` is a single clip.
Both are IVO files. A `.caf` contains a single `CAF_DATA` chunk; a
`.dba` contains one `DB_DATA` chunk (the packed clip blocks) plus one
`DBA_META` chunk (the catalogue).

### 9.1 Clip block

A `.caf`'s `CAF_DATA` chunk payload begins directly with one clip
block. A `.dba`'s `DB_DATA` chunk payload begins with a 4-byte u32
**total_size** prefix, after which clip blocks are packed
back-to-back. Each clip block uses this layout:

```text
+0x00  '#dba' or '#caf'  (4-byte signature)
+0x04  bone_count        (u16)
+0x06  0xAA55            (u16 magic — empirically observed in all DBA
                          / CAF data inspected; not validated by parsers
                          shipped to date)
+0x08  data_size         (u32 — block size including header)
+0x0C  bone_hashes       (bone_count × u32)  — see §9.3
+....  controllers       (bone_count × ControllerEntry, 24 bytes each)
+....  keyframe payload  (positions + rotations + time tables)
```

Inside a DBA, blocks are packed back-to-back; the loop terminates when
a `'#dba' / '#caf'` magic is no longer found. Blocks are read in
sequence; their order is identical to the metadata catalogue order
(§9.2), so `block[i]` belongs to `metadata_entry[i]` and clip name
`names[i]`.

### 9.2 DBA metadata catalogue (chunk `DBA_META`)

The catalogue maps clip names to their per-block metadata:

```text
+0x00  count (u32)
+0x04  count × 48-byte entry
+....  4-byte NUL-pad
+....  NUL-terminated UTF-8 string table (alphabetically sorted CAF names)
```

Each 48-byte entry:

| Offset | Size | Field             | Notes                                 |
| ------ | ---- | ----------------- | ------------------------------------- |
| 0x00   | 4    | `flags0`          | Often 0                               |
| 0x04   | 4    | `flags1`          | Often 0                               |
| 0x08   | 2    | `fps`             | Frames per second (e.g. 30)           |
| 0x0A   | 2    | `num_controllers` | Equals matching block's `bone_count`  |
| 0x0C   | 4    | `version`         | 0x900                                 |
| 0x10   | 4    | reserved (0)      |                                       |
| 0x14   | 4    | `end_frame`       | Total frame count                     |
| 0x18   | 16   | `start_rotation`  | f32×4 quaternion (XYZW)               |
| 0x28   | 8    | `start_position`  | f32×2 (XY only — Z elided)            |

Catalogue entries are 1:1 positional with clip blocks: `entry[i]`
describes `block[i]`. The string table provides clip names in
alphabetical order; the parser maps them back to entries by index.

### 9.3 Per-bone routing — CRC32 hash

`bone_hashes[i]` is the CRC32 (zlib polynomial) of the bone name in the
**case-preserved** byte representation:

```rust
pub fn bone_name_hash(name: &str) -> u32 {
    crc32fast::hash(name.as_bytes())
}
```

(Source:
[`crates/starbreaker-3d/src/animation.rs:1092-1097`](../crates/starbreaker-3d/src/animation.rs).)

Earlier public hypotheses about CryEngine bone hashing assume
lowercase normalisation, which does not match Star Citizen's data.
Hashes match only when the bone-name string is fed verbatim, e.g.
`"BONE_Front_Landing_Gear_Foot"` → `0x...`.

The 1:1 array alignment means controller `i` belongs to whichever
skeleton bone has the matching hash. Consumers should either:

1. precompute the hash for every bone in the CHR and build a
   `bone_hash → bone_index` map, or
2. iterate the clip's hashes in order and look each one up.

If a clip's hash does not match any skeleton bone, the controller is
silently dropped — this is normal because some clips animate auxiliary
helper bones not present in every CHR variant.

### 9.4 ControllerEntry

Each per-bone controller is exactly 24 bytes:

```rust
struct ControllerEntry {
    num_rot_keys:     u16,
    rot_format_flags: u16,
    rot_time_offset:  u32,
    rot_data_offset:  u32,
    num_pos_keys:     u16,
    pos_format_flags: u16,
    pos_time_offset:  u32,
    pos_data_offset:  u32,
}
```

(Source:
[`crates/starbreaker-3d/src/animation.rs:169-189`](../crates/starbreaker-3d/src/animation.rs).)

**Critical detail:** all four offsets are **relative to the start of
the controller entry itself**, not to the block, the chunk, or the
file. A controller for bone `b` always reads its data at `&data[base +
offset]` where `base` is the byte offset of that controller within the
clip data. A `*_time_offset` of 0 means "no separate time table — keys
are at frame `[0, 1, 2, … num_keys-1]`" (the uniform fallback).

`format_flags` (16 bits) encodes two unrelated things:

- low nibble (`flags & 0x0F`) → **time-key format** (§10)
- high byte (`flags >> 8`)   → **value compression format** (§11 / §12)

### 9.5 FPS

The clip's playback rate is taken from:

1. the matching DBA metadata entry (preferred), or
2. the `ANIM_INFO` chunk (CAF), or
3. **30 fps** (default fallback when neither is present).

Frame indices in time-key tables and in clip metadata's `end_frame` are
in this fps. Time in seconds is `frame / fps`.

---

## 10. Time-key formats

The low nibble of `*_format_flags` selects a time-key encoding:

| Code           | Encoding                                                                    |
| -------------- | --------------------------------------------------------------------------- |
| `0x00`         | `u8 × num_keys` — each byte is a frame number                               |
| `0x01`         | `u16 × num_keys` — each short is a frame number                             |
| `0x02` / `0x42`| **Per-frame keyframe bitmap** — see §10.1                                   |

If `*_time_offset == 0`, no time table is read; keys default to
`[0, 1, 2, …, num_keys-1]`.

### 10.1 The bitmap encoding (`0x02` / `0x42`)

This was historically the most-misunderstood part of the format. The
correct layout, validated against Scorpius `wings_deploy.caf` and
landing-gear clips, is:

```text
+0x00  start  (u16)  — first frame keyed
+0x02  end    (u16)  — last frame keyed (inclusive)
+0x04  bitmap (u8 × ceil((end - start + 1) / 8))
```

The bitmap is **LSB-first per byte**: bit `b` in byte `n` represents
frame `start + n*8 + b`. A set bit means "there is a keyframe at this
frame". The number of set bits across all bytes equals the channel's
`num_*_keys`. Keyframes occur on the exact frames marked by the
bitmap; they are **not** uniformly distributed across `[start..=end]`.

Worked example — Scorpius `wings_deploy.caf`, top-right wing
(controller hash `0x5F3AF303`):

```
start = 0, end = 75   ⇒ bit_count = 76
byte_count = ⌈76 / 8⌉ = 10
24 bits set across the 10 bytes  ⇒ num_rot_keys = 24
keyframes at frames marked by the set bits, not at uniform intervals
```

A reader that treated this as "uniformly stretch 24 keys across 76
frames" silently corrupts every clip with non-uniform cadence —
implementations must respect the bitmap. The StarBreaker parser
isolates this as its own case to make the encoding obvious to anyone
porting it to a new language.

When a count mismatch is detected (set bits ≠ `num_*_keys`), the
canonical fallback is uniform distribution, with a logged warning. In
practice all production data we have validated has matched bit-counts.

### 10.2 Verbatim parser

The reference implementation
([`crates/starbreaker-3d/src/animation.rs::read_time_keys`](../crates/starbreaker-3d/src/animation.rs)):

```rust
0x02 | 0x42 => {
    let start = u16::from_le_bytes([data[off], data[off + 1]]) as u32;
    let end   = u16::from_le_bytes([data[off + 2], data[off + 3]]) as u32;
    let bit_count  = (end - start + 1) as usize;
    let byte_count = bit_count.div_ceil(8);
    let bitmap     = &data[off + 4 .. off + 4 + byte_count];

    let mut times = Vec::with_capacity(count);
    for byte_idx in 0 .. byte_count {
        let b = bitmap[byte_idx];
        for bit_idx in 0 .. 8 {
            let frame = byte_idx * 8 + bit_idx;
            if frame >= bit_count { break; }
            if (b >> bit_idx) & 1 == 1 {
                times.push((start as usize + frame) as f32);
            }
        }
    }
    Ok(times)
}
```

---

## 11. Quaternion compression

The high byte of `rot_format_flags` selects the rotation encoding.
Two formats are observed in current Star Citizen data; three more from
the historical CryEngine catalogue are documented for completeness.

| `rot_format_flags >> 8` | Name                       | Bytes/key | Status      |
| ----------------------- | -------------------------- | --------- | ----------- |
| `0x80`                  | `eNoCompressQuat` (f32×4)  | 16        | Verified    |
| `0x82`                  | `eSmallTree48BitQuat`      | 6         | Verified    |
| (legacy)                | `eShortInt3Quat`           | 6         | Documented  |
| (legacy)                | `eSmallTreeDWORDQuat`      | 4         | Documented  |
| (legacy)                | `eSmallTree64BitQuat`      | 8         | Documented  |
| (legacy)                | `eSmallTree64BitExtQuat`   | 8         | Documented  |

### 11.1 `0x80` — Uncompressed quaternion

Four little-endian f32 values per key, in **CryEngine XYZW** order.
Convert to Blender's `WXYZ` form via the basis change in §13.

### 11.2 `0x82` — `SmallTree48BitQuat` (the dominant format)

This is "smallest three" 15-bit packed quaternion compression with two
header bits selecting which component was dropped. The exact decoder
required Ghidra-confirmation of CryEngine's binary because the bit
fields straddle u16 boundaries with sign-bit borrow. The verified
layout
([`crates/starbreaker-3d/src/animation.rs::decode_small_tree_quat_48`](../crates/starbreaker-3d/src/animation.rs)):

```text
Read three little-endian u16 words:  s0, s1, s2

idx       = (s2 >> 14) & 0x3        // 2-bit index of dropped component
INV_SCALE = 1.0 / 23170.0
RANGE     = 1 / √2  ≈ 0.7071067811865

// raw0: low 15 bits of s0
raw0_bits = s0 & 0x7FFF

// raw1: (s1 << 1) with bit 0 borrowing from s0's sign bit, masked to 15 bits
//   if s0's MSB is set, add 1 to (s1 * 2); else use (s1 * 2)
raw1_bits = (s1 * 2 + ((s0 >> 15) & 1)) & 0x7FFF

// raw2: (s1 >> 14) low bits, with high bits coming from sign-extended s2 * 4
//   (s2 sign-extended as i16, then *4, then add (s1 >> 14), then mask 15 bits)
raw2_bits = ((s1 >> 14) + sign_extend_i16(s2) * 4) & 0x7FFF

raw0 = (raw0_bits as f32) * INV_SCALE - RANGE
raw1 = (raw1_bits as f32) * INV_SCALE - RANGE
raw2 = (raw2_bits as f32) * INV_SCALE - RANGE

w_sq    = 1 - raw0² - raw1² - raw2²
largest = sqrt(max(w_sq, 0))

slot_table[idx] = which output components receive raw0, raw1, raw2,
                  with the dropped (largest) component reconstructed at
                  index `idx`:

    idx=0 → q[0]=largest, q[1]=raw0, q[2]=raw1, q[3]=raw2
            (output: [largest, raw0, raw1, raw2])
    idx=1 → q[1]=largest, q[0]=raw0, q[2]=raw1, q[3]=raw2
            (output: [raw0, largest, raw1, raw2])
    idx=2 → q[2]=largest, q[0]=raw0, q[1]=raw1, q[3]=raw2
            (output: [raw0, raw1, largest, raw2])
    idx=3 → q[3]=largest, q[0]=raw0, q[1]=raw1, q[2]=raw2
            (output: [raw0, raw1, raw2, largest])
```

Returns `[x, y, z, w]` in CryEngine's quaternion convention.

The Ghidra-traced "sign-bit borrow" between adjacent u16 words is
essential — naïve readers that treat `s0`, `s1`, `s2` as independent
15-bit values produce visually-similar but quantitatively wrong
rotations that fail visual snap-pose tests.

A regression test pinned to `0x82` decoding measures **2.29° angular
error** vs. the deployed-pose target on the Scorpius rear-foot, which
is well within the ±5° quantisation budget the encoding allows.

### 11.3 Legacy formats (reference)

- **`eShortInt3Quat` (6 bytes):** three i16 components for `x, y, z`;
  `w` reconstructed as `+sqrt(1 - x² - y² - z²)`.
- **`eSmallTreeDWORDQuat` (4 bytes):** packed 10-bit smallest-three with
  2-bit index — same scheme as 48-bit but with 10-bit quantisation per
  component.
- **`eSmallTree64BitQuat` (8 bytes):** 20-bit smallest-three.
- **`eSmallTree64BitExtQuat` (8 bytes):** mixed 21/20-bit packing.

These are present in CryEngine's reference codec but have not been
observed in production Star Citizen DBA / CAF files inspected to date.

---

## 12. Position compression

The high byte of `pos_format_flags` selects the position encoding:

| `pos_format_flags >> 8` | Encoding                                | Bytes/key |
| ----------------------- | --------------------------------------- | --------- |
| `0xC0`                  | Uncompressed `f32×3`                    | 12        |
| `0xC1`                  | SNORM with 24-byte header (interleaved) | 6         |
| `0xC2`                  | SNORM with 24-byte header (planar)      | 2 per active axis |

### 12.1 `0xC0` — uncompressed

Three little-endian f32 components per key, CryEngine XYZ order.

### 12.2 `0xC1` — SNORM full

Header (24 bytes), then `u16 × 3` per key:

```text
+0x00  scale[3]  (f32)
+0x0C  offset[3] (f32)
+0x18  data: u16 × 3 × num_keys (interleaved x,y,z per key)

key[i].axis[a] = data_u16(i, a) * scale[a] + offset[a]
```

### 12.3 `0xC2` — SNORM packed (planar)

Same 24-byte header, but per-axis data is **planar** (axis 0 first
across all keys, then axis 1, then axis 2) and an **inactive axis**
sentinel is encoded by `scale[a]` exceeding `3.0e38`:

```text
const FLT_SENTINEL: f32 = 3.0e38;
active[a] = scale[a].abs() < FLT_SENTINEL;

axis_starts[a] starts at the data block; if active[a]:
    bytes [axis_starts[a] .. axis_starts[a] + num_keys * 2] hold u16×num_keys
    next axis starts after this block

key[i].axis[a] =
    active[a] ? data_u16(axis_starts[a], i) * scale[a] + offset[a]
              : offset[a]                                   // constant
```

This is how CryEngine elides motion on axes with no animation —
a single offset value substitutes for `num_keys` u16 reads.

---

## 13. Coordinate-system mapping

Star Citizen's CryEngine animation data is right-handed and
**Y-up** on disk — the same convention glTF uses, but the opposite of
Blender, Maya, and 3ds Max (which are Z-up or whose Z-up scene mode
is the conventional choice). Additionally, the on-disk quaternion
order is `xyzw`, while many DCCs (and the parsed `Bone` struct in
this project) standardise on `wxyz`.

This pipeline therefore involves three coordinate mappings:

1. **CryEngine on-disk → glTF scene.** The exporter wraps the entire
   scene in a parent node named `CryEngine_Z_up` whose transform is
   the standard Y-up→Z-up rotation. (The name describes the *output*
   axis convention of the wrapper subtree, not the input.)

   ```text
   ⎡1  0  0  0⎤
   ⎢0  0 -1  0⎥
   ⎢0  1  0  0⎥
   ⎣0  0  0  1⎦
   ```

   Vertex positions, bone transforms, and animation samples are stored
   verbatim in their CryEngine Y-up values; the wrapper rotation
   re-orients the whole subtree at glTF-load time so a Z-up DCC sees
   geometry the right way up.

2. **Per-sample axis basis.** When animation channels are emitted into
   a sidecar JSON consumed directly by a Z-up DCC (bypassing glTF's
   own coordinate handling), the basis change is applied per-sample.
   Mapping CryEngine Y-up `(cx, cy, cz)` to Blender Z-up `(bx, by, bz)`:

   ```text
   position: (cx, cy, cz)            → (cx, -cz, cy)
   quaternion (xyzw) → (wxyz):
            (qx, qy, qz, qw)          → (qw, qx, -qz, qy)
   ```

   This sends CryEngine's `+Y` (up) onto Blender's `+Z` (up), and
   CryEngine's `+Z` onto Blender's `-Y` (so the engine's forward axis
   becomes Blender's `-Y`, which matches Blender's default camera
   orientation).

   The position swap and quaternion swap are mathematically the same
   operation expressed in component form. They must always be applied
   **in lockstep** — applying one without the other produces rotated
   geometry whose rotations animate around the wrong axis. A
   regression test
   (`cry_xyzw_to_blender_wxyz_axis_swap_matches_position_swap`) pins
   them together.

3. **glTF → DCC.** Blender's glTF 2.0 importer applies its own Y-up→Z-up
   correction when the scene units are Z-up. For 3ds Max or Maya
   (also Z-up by default), the DCC's glTF / FBX importer applies the
   equivalent correction. The net effect across the round trip is
   that the geometry ends up oriented the way the original CryEngine
   artist saw it in their authoring tool.

### 13.1 Component-order summary

| Stage                         | Quaternion order | Axis convention                                |
| ----------------------------- | ---------------- | ---------------------------------------------- |
| CryEngine on disk             | `xyzw`           | Y-up RH                                        |
| Parsed `Bone` struct          | `wxyz`           | Y-up RH (CryEngine native, just reordered)     |
| Sidecar JSON (per-sample)     | `wxyz`           | Z-up RH (Blender / Maya target)                |
| glTF emission                 | `xyzw` (per spec)| Y-up; `CryEngine_Z_up` wrapper rotates subtree |
| Blender `rotation_quaternion` | `wxyz`           | Z-up RH                                        |
| 3ds Max scene (FBX import)    | `wxyz`           | Z-up RH (3ds Max default)                      |

### 13.2 Worked example

A bone whose CryEngine-native rotation quaternion at frame 38 is
`q_cry = (qx, qy, qz, qw)` must be inserted into Blender as:

```python
sample_q = (qw, qx, -qz, qy)   # WXYZ for Blender
```

If the bone was previously keyed at `prev_q`, hemisphere-align before
inserting:

```python
if dot(prev_q, sample_q) < 0:
    sample_q = (-w, -x, -y, -z)   # negate every component
obj.rotation_quaternion = sample_q
obj.keyframe_insert(data_path="rotation_quaternion", frame=blender_frame)
```

(See §14.4 for why this alignment is mandatory when the consuming DCC
uses LINEAR interpolation between keyframes.)

---

## 14. Reconstructing animations in a DCC

Once you have a parser for the formats above, the reconstruction
recipe for any DCC is:

### 14.1 Build the rig

1. Parse the entity's `.cdf` to discover the CHR and SKIN attachments.
2. Parse the CHR to obtain the bone list (name, parent, local TRS,
   world TRS).
3. In the DCC, create either an armature (bone-pose-driven) or an
   `Object` per bone (simpler for ships, where bones are mostly
   rigid-mesh anchors and IK is shallow). The StarBreaker Blender
   addon uses the per-Object-per-bone model for ships because:
   - it preserves CryEngine's exact transform composition,
   - it lets each rigid-mesh attachment be a child of its bone
     `Object` with no skinning,
   - and it keeps the imported scene editable without armature
     pose-bone gymnastics.
4. For each CHR bone, set `obj.rotation_mode = 'QUATERNION'`,
   `obj.rotation_quaternion = bone.local_rotation`, and
   `obj.location = bone.local_position`. Parent each bone-object to
   its parent bone-object (or to the rig root if `parent_index` is
   `None`).

### 14.2 Build the meshes

1. For every `.skin` referenced as `CA_SKIN`, classify each submesh as
   rigid or soft-skinned via the §5.3 rule.
2. **Rigid submeshes**: rebase vertices into the owning bone's local
   frame (multiply by `bone_world.inverse()`), create a mesh per
   bone-group, parent it under the bone object. No skinning.
3. **Soft-skinned submeshes**: emit a glTF `skin` with `JOINTS_0` and
   `WEIGHTS_0` from the `BoneMap12` entries; the consuming DCC's glTF
   importer handles the rest.
4. For every `.cgf` referenced as `CA_BONE`, simply parent the static
   mesh under the named bone with the offset position and rotation
   from the `.cdf` attachment.

### 14.3 Index the available clips

1. Parse the `.chrparams` to get the clip-name → CAF-filename map.
2. Resolve the `$TracksDatabase` path; parse the DBA metadata catalogue
   to list every clip the DBA contains.
3. Some clips appear **only** in the DBA (not in `.chrparams`); the
   user-facing list should be the union of both sources.

### 14.4 Bake animation samples

For each user-selected clip:

1. Locate the matching block in the DBA (or the lone block of a CAF).
2. For each controller `i`:
   - hash → bone-name lookup in your CHR map; skip if no match.
   - decode `num_rot_keys` rotations and their time table (§10).
   - decode `num_pos_keys` positions and their time table.
   - convert each sample to the DCC's coordinate system (§13).
3. Insert keyframes:
   - **Use LINEAR interpolation** to match CryEngine's runtime
     playback semantics. CryEngine does not bezier-ease between
     authored keys.
   - **Hemisphere-align consecutive quaternion samples** before
     inserting. The compressed quaternion decoder is allowed to flip
     the sign of `q` (q and −q encode the same rotation), but a DCC
     using per-component LINEAR interpolation between two
     hemisphere-mismatched keys passes through the zero quaternion at
     the midpoint and produces a 180° snap-flip. Compute
     `dot(prev_q, q)`; if negative, replace `q` with `-q` before
     inserting. (This is why slerp is **not** an alternative — slerp
     would still take the long way around between mismatched
     hemispheres unless you canonicalise first.)
4. Frame indices in the time tables map directly to DCC frames at the
   clip's fps; if the DCC's scene fps differs, scale linearly.

### 14.5 Bind pose vs. clip start

Some clips (notably retract-style transitions) are authored as
**boomerang tracks**: their first sample is identical to their last,
so playing them forwards or backwards both ends at the bind pose. In
this case, "Snap to first frame" and "Snap to last frame" yield the
same result.

Other clips (deploy-style) are **not** boomeranged: the last sample
differs from the first. The first sample is the bind pose; the last
sample is the deployed pose. A useful import-mode set is therefore:

- **None** — no animation; bind pose only.
- **Snap first** — pose at clip frame 0.
- **Snap last** — pose at clip's final frame.
- **Action** — full animation curve.

### 14.6 Anchor-relative rotation composition

Some clips are authored with a **non-identity start rotation** in
their DBA metadata (§9.2 `start_rotation`). When applying these clips
to a rig whose bind pose differs from the clip's authored origin, the
correct composition is:

```text
result = bind ⋅ (start⁻¹ ⋅ sample)
```

The `start⁻¹ ⋅ sample` factor cancels the clip's authored origin,
leaving only the per-frame delta; multiplying by `bind` re-anchors
that delta in the rig's actual bind pose. Hemisphere-align `sample`
against `start` before composing to avoid the same snap-flip pitfall.

This composition is the import-time analogue of CryEngine's runtime
behaviour, where the animation system continually composes clip
transforms onto the rig's current pose rather than overwriting it.

### 14.7 FBX / 3ds Max path

For DCCs that consume FBX rather than glTF:

1. Build the rig and meshes in glTF as above.
2. Convert the glTF to FBX with a tool that preserves bone-mesh
   parenting (Blender's FBX exporter is reliable for this; AssImp's is
   adequate but loses some metadata).
3. The FBX coordinate system is configurable; pick the one matching
   the target DCC. 3ds Max defaults to Z-up, so Z-up FBX is the
   typical choice.

The animation samples themselves carry through unchanged because the
glTF representation already contains baked LINEAR keyframes in the
target DCC's coordinate convention.

---

## 15. Limitations and out-of-scope content

Some animation behaviour visible in-game is **not** authored in the
files documented here. Awareness of where the gaps lie is essential
for any importer:

### 15.1 Primary state transforms

`.dba` and `.caf` clips carry **secondary motion only**: the
articulation of joints, pistons, plates, doors, canopy halves, etc.
Per-state primary transforms — the full slide-and-tilt of a canopy
between Closed and Open, the leg extension of a landing gear, the wing
fold-out of an X-shape ship — are driven by either:

- **CryEngine Mannequin fragment system** (a state-machine layer not
  exposed in `.chr` / `.chrparams` / `.dba`), or
- **C++ vehicle controller code** that runs procedural translation /
  rotation alongside the secondary-motion clips.

For example, on the Scorpius:

- `landing_gear_deploy.caf` (registered in `chrparams`) animates the
  bay door — zero net translation on the gear leg.
- `landing_gear_extend.caf` (DBA-only, **not** in `chrparams`)
  animates 33 bones including the foot's full 2.4-metre extension.

A faithful importer should expose **the union of `.chrparams` clips
and DBA-only clips**, letting the user pick the one that visually
matches the runtime behaviour they want.

### 15.2 Animevents `bone` attribute

A `<event>` element in an `.animevents` file carries a `bone`
attribute. In the StarBreaker pipeline the value is consumed as a
**routing hint** — it is tokenised and used to disambiguate which DBA
clip block matches a given chrparams event when several blocks have
overlapping bone-hash sets. It does **not** declare an animation
target on its own; the actual per-bone routing is determined entirely
by the clip's bone-hash list (§9.3). The empirical match between
animevents `bone` strings and audio-source bones suggests the original
in-engine purpose is to anchor sound playback at a 3D point on the
rig, but reliance on that interpretation is not required to import
animations correctly.

### 15.3 IK and constraints

Neither `.chrparams` nor the clip files contain IK chains, drivers, or
constraint definitions. IK is solved at runtime by CryEngine's animation
system using the rig's `limb_id` annotations (set in `.chr` v900) plus
gameplay-driven targets. Reproducing in-game IK behaviour requires
re-implementing the runtime solver, which is outside the scope of the
file formats documented here.

### 15.4 Material and shader animation

Material parameter animation (e.g. emissive blink, paint stripe
swap) lives in a separate system and is not encoded in `.dba` /
`.caf`.

### 15.5 Facial / character animation

This guide focuses on ship and mechanical-rig animation. Character
facial animation uses a different controller scheme (Mannequin
fragments + blend trees + facial bone constraints) which is partially
expressible in `.dba` but typically requires the full Mannequin context
to play back faithfully.

---

## 16. Source provenance

Every byte-level claim in this document is grounded in one of:

- The production Rust parser at
  [`crates/starbreaker-3d/src/`](../crates/starbreaker-3d/src/)
  (animation.rs, types.rs, skeleton.rs, nmc.rs, gltf/mod.rs,
  pipeline.rs, chrparams.rs).
- The production Blender importer at
  [`blender_addon/starbreaker_addon/`](../blender_addon/starbreaker_addon/)
  (runtime/package_ops.py, runtime/constants.py).
- An out-of-tree empirical research log retaining DBA byte hexdumps,
  clip-by-clip bone-hash matches, decoder gate-test results, and
  visual validations performed during reverse engineering. (Not
  redistributed in this repository.)
- Cross-reference with the public StarBreaker upstream at
  [github.com/diogotr7/StarBreaker](https://github.com/diogotr7/StarBreaker)
  (commit `d01ae21` on branch `feature/animation` for the original
  block-iteration logic), which served as a starting point before
  Star-Citizen-specific corrections (case-preserved hashing, bitmap
  time-key encoding, hemisphere alignment, single-joint rigid-mesh
  reassignment) were derived empirically.

The Rust parser ships with 129 lib tests including:

- `time_format_0x42_decodes_per_frame_keyframe_bitmap` — pins the §10.1
  bitmap encoding.
- `time_format_0x42_count_mismatch_falls_back_to_uniform` — pins the
  fallback behaviour.
- `cry_xyzw_to_blender_wxyz_axis_swap_matches_position_swap` — pins the
  §13 coordinate-system contract.
- `synthetic_skin_rebases_rigid_submesh_vertices_to_bone_space` — pins
  the §5.3 rigid-mesh classifier.
- `build_mesh_reassigns_single_joint_root_submesh_to_owning_bone` —
  pins the single-joint reassignment rule.

End-to-end visual validation has been performed on the RSI Scorpius
(landing gear, wings, canopy), RSI Aurora Mk2, DRAK Vulture, and the
Mole, with imported rigs matching in-game footage frame-for-frame
within the quantisation budget of the 48-bit quaternion format
(≈±5° per joint).

Contributions, corrections, and additional empirical evidence are
welcomed at the StarBreaker fork. The formats are inferred, not
specified; any discrepancy between this document and the on-disk
behaviour observed in a future game patch should be treated as a bug
in this document, not in the data.
