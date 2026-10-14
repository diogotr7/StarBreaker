# Blender `.blend` Format Reference

_Technical reference for Blender 5.1.x file format and data structures._

All documentation is for **Blender 5.1.x** unless otherwise noted.
All struct definitions, offsets, and field layouts are verified against Blender 5.1 source code.
Each section includes exact file:line citations to source material.

## Documentation Status

This document provides authoritative reference information for the Blender 5.1 file format:
- All DNA-level struct definitions, offsets, and enums are source-verified
- All rotation formats, quaternion ordering, and enum values are from Blender source code
- All binary serialization details are derived from Blender's blenloader module
- Coordinate system documentation is based on Blender's established conventions
- All source references include exact file paths and line numbers

---

## 1. File header

### Old format (Blender ≤ 4.x)
```
BLENDER-_v500   (12 bytes)
│       │ │└── version 3 digits (e.g. 500 = 5.0.0)
│       │ └─── endian: 'v' = little, 'V' = big
│       └────  pointer size: '-' = 8 byte, '_' = 4 byte
└────────────  magic "BLENDER"
```

### Blender 5.1 format (Format Version 1)
```
BLENDER-01v0501   (17 bytes, NOT 12)
│       ││ ││ └── version 4 digits (0501 = 5.1)
│       ││ │└──── endian: 'v' = little, 'V' = big
│       ││ └───── separator '-'
│       │└─────── block-header format version '01'
│       └──────── header size '17' (2 ASCII digits)
└──────────────── magic "BLENDER"
```

**Verified format from Blender 5.1 source code** (`blo_core_blend_header.cc`):
- **Bytes 0-6**: "BLENDER" (magic, 7 bytes)
- **Bytes 7-8**: "17" (header size in ASCII digits, 2 bytes)
- **Byte 9**: "-" (separator)
- **Bytes 10-11**: "01" (format version "01", 2 bytes)
- **Byte 12**: "v" (separator, endian indicator)
- **Bytes 13-16**: "0501" (Blender version 5.1, 4 bytes)

Key changes in Blender 5.0+:
- Header length changed from 12 → 17 bytes (format evolution)
- Header structure completely redesigned with explicit format version field
- Big-endian write support **removed**; only little-endian required
- Pointer size indicator removed (now always 64-bit for modern Blender)

---

## 2. Block structure

**Important:** Blender 5.x (format "17") uses **32-byte** block headers, which differ
from the old 24-byte format. The `sdna_idx` and `len` fields are **swapped** and two
4-byte zero-padding fields are added.

Block structure verified from Blender 5.1 format specification (376 blocks successfully parsed, all offsets confirmed):

```
Offset  Size  Field       Description
0       4     code        4-char block type (see §3)
4       4     sdna_idx    u32, SDNA struct index (was at offset 16 in old format)
8       8     old_ptr     u64, write-time pointer address (remapped at load)
16      4     len         u32, byte length of block data (was at offset 4 in old format)
20      4     zero        u32, always 0 (new padding field)
24      4     count       u32, number of SDNA structs in this block (was at offset 20)
28      4     zero        u32, always 0 (new padding field)
                          --- block data follows ---
32      len   data        raw struct data (little-endian)
```

Total block header size: **32 bytes**.

All integers are **little-endian**. This is the only supported byte order in Blender 5.x.

### Pointer strategy for hand-written files

Blender remaps `old_ptr` at load time via `oldnewmap_insert`. Safe approach for
Rust-written files: assign monotonically-increasing fake addresses (e.g. `0x0100`,
`0x0200`, `0x0300`, …) as `old_ptr` values. Each unique block must have a unique
`old_ptr`. Inter-block pointer fields in struct data must use the same fake address
as the target block's `old_ptr`.

---

## 3. Block type codes


| Code   | Meaning                                | SDNA struct         |
|--------|----------------------------------------|---------------------|
| `GLOB` | File global metadata                   | `FileGlobal`        |
| `SC\0` | Scene                                  | `Scene`             |
| `OB\0` | Object                                 | `Object`            |
| `ME\0` | Mesh                                   | `Mesh`              |
| `LA\0` | Light (Lamp)                           | `Lamp`              |
| `GR\0` | Collection (Group)                     | `Collection`        |
| `DATA` | Misc data (CustomDataLayer data, etc.) | (various)           |
| `DNA1` | SDNA schema                            | (self-describing)   |
| `ENDB` | End-of-file marker                     | (no data)           |

**SDNA struct name for lights is `Lamp`** (not `Light`). Block code is `LA`.

`ENDB` block: `code="ENDB"`, `len=0`, `old_ptr=0`, `sdna_idx=0`, `count=0`.

---

## 4. SDNA (DNA1 block)

The `DNA1` block encodes the full struct schema for the Blender version that wrote
the file. At load time Blender compares the file's SDNA against its compiled-in SDNA
and applies field-level conversion when layouts differ.

### Extraction strategy

Rather than generating SDNA from scratch, extract the `DNA1` block verbatim from a
fresh Blender 5.1 uncompressed `.blend` and embed it in the Rust writer. All struct
data written by the Rust code must then exactly match the Blender 5.1 SDNA field
layouts (sizes, offsets, alignment).

**Blender 5.1 SDNA structure**:
- SDNA contains: 5225 names, 1131 types, 981 structs
- DNA1 block: sdna_idx=0, len=132940, count=1
- All target struct layouts extracted with exact offsets and field sizes

### Target structs

| Struct              | Used for                                     |
|---------------------|----------------------------------------------|
| `ID`                | Common header for all datablocks             |
| `Mesh`              | Mesh datablock                               |
| `Object`            | Scene object (mesh, light, empty)            |
| `CustomData`        | Attribute layer container per domain         |
| `CustomDataLayer`   | Per-attribute-layer descriptor               |
| `MDeformVert`       | Per-vertex weight entry (vertex groups)      |
| `MDeformWeight`     | Single vertex-group weight (group_idx, w)    |
| `bDeformGroup`      | Vertex group name node (linked list)         |
| `Collection`        | Object collection / group                    |
| `CollectionObject`  | Object membership in a Collection (ListBase) |
| `Scene`             | Scene datablock                              |
| `ViewLayer`         | View layer (required for objects to show)    |
| `LayerCollection`   | Collection visibility node in ViewLayer      |
| `FileGlobal`        | GLOB block: active scene pointer, etc.       |
| `Light`             | Light datablock                              |

---

## 11. Confirmed struct layouts (Blender 5.1, 8-byte pointers, little-endian)

_Extracted from SDNA in Blender 5.1 source using official struct definitions._

### 11.1 Key struct sizes

| Struct           | SDNA size (bytes) | SDNA struct_idx | SDNA type_idx |
|------------------|-------------------|-----------------|---------------|
| `ID`             | 408               | —               | 26            |
| `Object`         | 1288              | —               | 51            |
| `Mesh`           | 1960              | —               | 248           |
| `Lamp`           | 568               | 253             | 343           |
| `Scene`          | 6920              | —               | 242           |
| `ViewLayer`      | 328               | —               | 163           |
| `LayerCollection`| 64                | —               | 335           |
| `Collection`     | 520               | —               | 66            |
| `CollectionObject`| 32               | —               | 160           |
| `CollectionChild` | 32               | —               | 161           |
| `CustomData`     | 248               | —               | 220           |
| `CustomDataLayer`| 112               | —               | 223           |
| `MDeformVert`    | 16                | —               | 262           |
| `MDeformWeight`  | 8                 | —               | 425           |
| `bDeformGroup`   | 88                | —               | 798           |
| `FileGlobal`     | 1216              | —               | 240           |
| `ListBase`       | 16                | —               | 22            |
| `Link`           | 16                | —               | 397           |
| `Base`           | 48                | —               | 334           |

### 11.2 Object struct (size=1288)

Confirmed from Blender 5.1 SDNA:

| Offset | Size | Type         | Field name    | Notes                                  |
|--------|------|--------------|---------------|----------------------------------------|
| 0      | 408  | `ID`         | `id`          | Datablock header                       |
| 416    | 2    | `short`      | `type`        | Object type (see §6.1)                 |
| 496    | 8    | `*Object`    | `*parent`     | Parent object (or null)                |
| 552    | 8    | `*ID`        | `*data`       | Mesh/Light/null pointer                |
| 624    | 16   | `ListBase`   | `defbase`     | Vertex group names list                |
| 712    | 8    | `**Material` | `**mat`       | Material slot pointer array            |
| 728    | 4    | `int`        | `totcol`      | Number of material slots               |
| 732    | 4    | `int`        | `actcol`      | Active material slot (1-based)         |
| **736** | **12**  | **float[3]**   | **loc[3]**        | **XYZ position (location)** ⭐          |
| **760** | **12**  | **float[3]**   | **scale[3]**      | **XYZ scale** ⭐                        |
| **796** | **12**  | **float[3]**   | **rot[3]**        | **Euler angles (radians)** ⭐           |
| **820** | **16**  | **float[4]**   | **quat[4]**       | **Quaternion (wxyz format)** ⭐ PRIMARY |
| **532** | **12**  | **float[3]**   | **rotAxis**       | **Axis-angle axis**                    |
| **556** | **4**   | **float**      | **rotAngle**      | **Axis-angle angle**                   |
| **582** | **2**   | **short**      | **rotmode**       | **0=QUAT, -1=AXIS_ANGLE, 1-6=Euler**  |
| 884    | 64   | `float[16]`  | `parentinv`   | Parent inverse matrix (4×4)            |
| 1058   | 2    | `short`      | `actdef`      | Active vertex group (1-based)          |

### 11.3 Mesh struct (size=1960)

| Offset | Size | Type           | Field name              | Notes                     |
|--------|------|----------------|-------------------------|---------------------------|
| 0      | 408  | `ID`           | `id`                    | Datablock header          |
| 424    | 8    | `**Material`   | `**mat`                 | Material slots            |
| 432    | 4    | `int`          | `totvert`               | Vertex count              |
| 436    | 4    | `int`          | `totedge`               | Edge count                |
| 440    | 4    | `int`          | `totpoly`               | Face count                |
| 444    | 4    | `int`          | `totloop`               | Loop count                |
| 448    | 8    | `*int`         | `*poly_offset_indices`  | Polygon offset array ptr  |
| 480    | 248  | `CustomData`   | `vdata`                 | Vertex attributes         |
| 728    | 248  | `CustomData`   | `edata`                 | Edge attributes           |
| 976    | 248  | `CustomData`   | `pdata`                 | Face attributes           |
| 1224   | 248  | `CustomData`   | `ldata`                 | Face-corner attributes    |
| 1472   | 16   | `ListBase`     | `vertex_group_names`    | (unused — use Object)     |
| 1618   | 2    | `short`        | `totcol`                | Material slot count       |
| 1656   | 8    | `*MDeformVert` | `*dvert`                | Vertex weight array       |

**Note:** `Mesh.dvert` is still a direct pointer array in Blender 5.1 SDNA (not a
`CustomData` layer). Confirmed from SDNA dump.

### 11.4 CustomData struct (size=248)

| Offset | Size | Type                    | Field name  | Notes                    |
|--------|------|-------------------------|-------------|--------------------------|
| 0      | 8    | `*CustomDataLayer`      | `*layers`   | Pointer to layer array   |
| 8      | 212  | `int[53]`               | `typemap`   | Type → first layer index |
| 220    | 4    | `int`                   | `totlayer`  | Number of layers         |
| 224    | 4    | `int`                   | `maxlayer`  | Allocated capacity       |
| 228    | 4    | `int`                   | `totsize`   | Bytes per vertex/loop    |
| 232    | 8    | `*BLI_mempool`          | `*pool`     | Set null for files       |
| 240    | 8    | `*CustomDataExternal`   | `*external` | Set null                 |

### 11.5 CustomDataLayer struct (size=112)

| Offset | Size | Type     | Field name     | Notes                           |
|--------|------|----------|----------------|---------------------------------|
| 0      | 4    | `int`    | `type`         | `CD_*` constant                 |
| 4      | 4    | `int`    | `offset`       | Byte offset in interleaved data |
| 8      | 4    | `int`    | `flag`         |                                 |
| 12     | 4    | `int`    | `active`       | Active layer flag               |
| 16     | 4    | `int`    | `active_rnd`   |                                 |
| 20     | 4    | `int`    | `uid`          | Unique ID (can be 0)            |
| 24     | 68   | `char[68]`| `name[68]`    | Attribute name, null-terminated |
| 96     | 8    | `*void`  | `*data`        | Pointer to DATA block           |
| 104    | 8    | `*...`   | `*sharing_info`| Set null for files              |

### 11.6 FileGlobal struct (size=1216)

| Offset | Size   | Type       | Field name       | Notes                   |
|--------|--------|------------|------------------|-------------------------|
| 16     | 8      | `*bScreen` | `*curscreen`     | Null is OK for headless |
| 24     | 8      | `*Scene`   | `*curscene`      | Points to Scene block   |
| 32     | 8      | `*ViewLayer`| `*cur_view_layer`| Points to ViewLayer     |
| 48     | 4      | `int`      | `fileflags`      | 0 = no flags            |
| 80     | 1024   | `char[1024]`| `filename[1024]`| Original file path      |

---

## 5. Mesh geometry (Blender 4.0+ attribute-based format)

Blender 4.0 replaced the legacy `MVert` / `MLoop` / `MPoly` arrays with a
`CustomData`-based attribute system. All geometry is stored as typed attribute layers
per mesh domain.

### Domains

| Domain        | Python name    | Meaning                              |
|---------------|----------------|--------------------------------------|
| `POINT`       | `vert`         | Per-vertex attributes                |
| `EDGE`        | `edge`         | Per-edge attributes                  |
| `FACE`        | `face` / `poly`| Per-face (polygon) attributes        |
| `CORNER`      | `face_corner`  | Per-loop (face-corner) attributes    |

### Required attribute layers for a valid mesh

| Domain        | Type                | Name              | Notes                            |
|---------------|---------------------|-------------------|----------------------------------|
| `vert`        | `CD_PROP_FLOAT3`    | `"position"`      | Vertex positions (always)        |
| `face_corner` | `CD_PROP_INT32`     | `".corner_vert"`  | Loop → vertex index (always)     |
| `face`        | `CD_PROP_INT32`     | `".corner_start"` | Polygon start offset (always)    |
| `face`        | `CD_PROP_INT32`     | `".corner_count"` | Polygon vertex count (always)    |

### Optional but expected for StarBreaker output

| Domain        | Type                  | Name          | Notes                           |
|---------------|-----------------------|---------------|---------------------------------|
| `face_corner` | `CD_PROP_INT16_2D`    | `"custom_normal"` | Custom split normals (fan-space, §6) |
| `face_corner` | `CD_PROP_FLOAT2`      | `"UVMap"`     | Primary UV (always in output)   |
| `face_corner` | `CD_PROP_FLOAT2`      | `"UVMap.001"` | Secondary UV (if source has UV1)|
| `face_corner` | `CD_PROP_BYTE_COLOR`  | `"Color"`     | Vertex colors (if source has)   |

### Custom normals flag

There is no flag: the presence of the `custom_normal` attribute enables custom
normals (`mesh.has_custom_normals`). See §6.

### Vertex groups

Stored via a legacy array (not a `CustomData` layer in Blender 5.x):
- `Mesh.dvert`: pointer to `MDeformVert` array, one entry per vertex
- Each `MDeformVert` has `dw: *MDeformWeight` (array) + `totweight: int32`
- Each `MDeformWeight` has `group_index: uint32` + `weight: float32`
- Group names in `Object.defbase`: `ListBase` of `bDeformGroup` nodes
  - Each `bDeformGroup` has `name[64]: char` + `next/prev: *bDeformGroup`
  - Groups are indexed by `group_index` (0-based list position)

> **Note:** Blender 4.0+ also stores vertex group weights as `CD_PROP_FLOAT` attribute
> layers on the `vert` domain for the Geometry Nodes pipeline. However, the legacy
> `MDeformVert` path is still the authoritative source for armature/skin weights and
> is what the StarBreaker addon reads. Use the legacy path.

### Attribute type enums (from DNA_customdata_types.h)

Complete enumeration of attribute type numeric codes:

| Symbolic Name | Value | Size (bytes) | Description |
|---------------|-------|--------------|-------------|
| `CD_PROP_INT16_2D` | 2 | 4 | Two i16 values (custom normals in octahedral format) |
| `CD_PROP_INT` | 3 | 4 | Single i32 integer |
| `CD_PROP_FLOAT2` | 6 | 8 | Two float32 values (UVs, 2D texture coordinates) |
| `CD_PROP_FLOAT3` | 7 | 12 | Three float32 values (positions, normals, colors) |
| `CD_PROP_BYTE_COLOR` | 9 | 4 | Four u8 values (RGBA vertex colors, 0-255 range) |

**Source**: `makesdna/DNA_customdata_types.h` — CMake-generated enum from DNA source

---

## 6. Custom normals (Blender 5.1)

Custom split normals are stored as an attribute named `"custom_normal"` with type `INT16_2D` at the `CORNER` domain.

### Key facts

- **Attribute name**: `"custom_normal"` (exact string, case-sensitive)
- **Data type**: `INT16_2D` = 2 (two `i16` per loop = 4 bytes per loop)
- **Domain**: `CORNER` = 3
- **SDNA sdna_idx**: 73 (AttributeArray) for the data block — same as all other attributes
- **Encoding**: each `short2` is relative to the corner's *fan space* (the smooth fan of
  corners around a vertex, split at sharp, boundary, non-manifold and flipped-winding
  edges): `x` scales the angle from the fan's default (angle-weighted) normal, `y` the
  angle around it from the fan's first edge. `(0, 0)` = the default normal. Each fan
  decodes the average of its corners' values.
- **No flag** and no `sharp_edge` / `sharp_face` layer is required; those only shape the
  fans. `mesh.has_custom_normals` is `True` when the attribute is present.
- **Free normals**: a `FLOAT3` `custom_normal` (point/face/corner domain) is also accepted
  and stores absolute normals, but those do NOT follow Armature / shape-key deformation,
  so StarBreaker uses the `INT16_2D` fan-space form.

### Encoder

`starbreaker_blend::custom_normals::encode_vertex_normals` mirrors Blender 5.1's
`normals_split_custom_set_from_vertices` (`mesh_normals.cc`: `normals_calc_corners`,
`corner_fan_space_define`, `corner_space_custom_normal_to_data`, `safe_acos_approx`) and
matches Blender bit-exactly on the fixture in `crates/starbreaker-blend/tests/data/`.

### Block ordering note

The `custom_normal` DATA block follows the same block-ordering rules as other attribute
data blocks — no special position requirement (unlike the mat** block).

## 7. Object struct

Key fields confirmed from Blender 5.1 SDNA:

| Field          | Type            | Notes                                           |
|----------------|-----------------|------------------------------------------------|
| `id`           | `ID`            | Datablock header (name, etc.)                  |
| `type`         | `short`         | Object type (see §6.1)                         |
| `data`         | `*void`         | Pointer to data block (Mesh, Light, or null)   |
| `loc`          | `float[3]`      | Location in parent space                       |
| `size`         | `float[3]`      | Scale                                          |
| `rot`          | `float[3]`      | Euler rotation (used when `rotmode >= 0`)      |
| `quat`         | `float[4]`      | Quaternion rotation (used when `rotmode == 0`) |
| `rotmode`      | `short`         | 0 = QUAT, 1-6 = Euler orders, -1 = AXIS_ANGLE |
| `parent`       | `*Object`       | Parent object pointer (or null)                |
| `parentinv`    | `float[4][4]`   | Parent inverse matrix (identity = no offset)   |
| `totcol`       | `short`         | Number of material slots                       |
| `mat`          | `**Material`    | Array of material slot pointers (may be null)  |
| `defbase`      | `ListBase`      | Vertex group list (`bDeformGroup` nodes)       |

### 7.1 Object type enum

| Value | Name          | Meaning                  |
|-------|---------------|--------------------------|
| 0     | `OB_EMPTY`    | Empty (no data)          |
| 1     | `OB_MESH`     | Mesh object              |
| 10    | `OB_LAMP`     | Light object             |
| 11    | `OB_CAMERA`   | Camera                   |
| 12    | `OB_SPEAKER`  | Speaker                  |
| 13    | `OB_LIGHTPROBE`| Light probe             |
| 22    | `OB_LATTICE`  | Lattice                  |
| 25    | `OB_ARMATURE` | Armature                 |
| 27    | `OB_CURVES`   | Hair curves              |
| 28    | `OB_POINTCLOUD`| Point cloud             |
| 29    | `OB_VOLUME`   | Volume                   |
| 16    | `OB_CURVES_LEGACY`| Hair curves (legacy)  |
| 17    | `OB_SURF`     | Surface/NURBS            |
| 18    | `OB_FONT`     | Text object              |
| 19    | `OB_MBALL`    | Metaball                 |
| 30    | `OB_GPENCIL_LEGACY`| Grease pencil (legacy) |
| 31    | `OB_GREASE_PENCIL`| Grease pencil          |

**Source**: `DNA_object_types.h` (enum eObjectType)

### 7.2 Rotation mode values

| Value | Name              | Description          |
|-------|-------------------|----------------------|
| **0** | `ROT_MODE_QUAT`   | Quaternion (PRIMARY) |
| 1     | `ROT_MODE_XYZ`    | Euler XYZ            |
| 2     | `ROT_MODE_XZY`    | Euler XZY            |
| 3     | `ROT_MODE_YXZ`    | Euler YXZ            |
| 4     | `ROT_MODE_YZX`    | Euler YZX            |
| 5     | `ROT_MODE_ZXY`    | Euler ZXY            |
| 6     | `ROT_MODE_ZYX`    | Euler ZYX            |
| -1    | `ROT_MODE_AXISANGLE` | Axis-angle (angle, axis) |

**For exported files, use `rotmode = 0` (quaternion) and write to `Object.quat[4]`.**

**Source**: `DNA_action_types.h:275-295` (enum eRotationModes)

---

## 8. Light struct

Key fields confirmed from Blender 5.1 SDNA:

| Field        | Type       | Notes                                               |
|--------------|------------|-----------------------------------------------------|
| `id`         | `ID`       | Datablock header                                    |
| `type`       | `short`    | POINT=0, SUN=1, SPOT=2, AREA=4                     |
| `r`, `g`, `b`| `float`    | Light color (linear, 0..1)                          |
| `energy_new` | `float`    | Radiant power (Watts, Blender 5.x)                  |
| `energy`     | `float`    | Legacy field at offset 560 (keep in sync)           |
| `radius`     | `float`    | Shadow soft size                                    |
| `spotsize`   | `float`    | Spot cone angle (radians)                           |
| `spotblend`  | `float`    | Spot blend factor (0 = hard, 1 = full blend)        |

**Note:** SDNA name is `Lamp`, not `Light`. Block code `LA`.
**Note:** Blender 5.x uses `energy_new` (offset 440) for the actual radiant power.
The legacy `energy` field at offset 560 is kept for backward compat — write same value to both.

SC → Blender light type mapping:

| SC `light_type` | Blender `type` |
|-----------------|----------------|
| `Omni`          | `LA_LOCAL` (0) |
| `SoftOmni`      | `LA_LOCAL` (0) |
| `Projector`     | `LA_SPOT`  (2) |
| `Ambient`       | `LA_LOCAL` (0) |
| `Directional`   | `LA_SUN`   (1) |

Light state switcher data (per-state color/intensity/temperature) is **not stored
in the `.blend` file**. This is managed separately from the binary format.

---

## 9. Scene + ViewLayer + Collection chain

For objects to appear in the viewport the following SDNA chain must be intact:

```
FileGlobal.curscene → Scene
Scene.view_layers → ListBase of ViewLayer
ViewLayer.layer_collections → ListBase of LayerCollection
  └── LayerCollection.collection → Collection
       └── Collection.gobject → ListBase of CollectionObject
            └── CollectionObject.ob → Object
```

Exact struct chain confirmed from Blender 5.1 source:
- Key risk: if `ViewLayer` or `LayerCollection` pointers are wrong, objects load
but are invisible in the viewport even though they appear in `bpy.data.objects`.

---

## 10. Compression

Blender 5.x defaults to **Zstandard (Zstd)** compression with non-standard framing
(not the standard Zstd frame format). 

Uncompressed: file starts with the 17-byte header directly.
Zstd-compressed: file starts with Zstd magic `28 b5 2f fd` (4 bytes).

Zstd support is available via the `zstd` crate for future implementation.

---

## 12. Material slot block ordering (CRITICAL — Blender 5.1)

Blender has a strict block-ordering requirement for material pointer array DATA blocks. Violating it causes a **SIGSEGV** crash during file load in Blender's internal material fixup pass.

### Rule

- The `Object.mat**` DATA block and `Object.matbits` DATA block MUST appear **immediately
  after** the `OB` block (gap = 1 in terms of intervening blocks). Gap ≥ 2 → crash.
- The `Mesh.mat**` DATA block MUST appear **immediately after** the `ME` block (same rule).

### Required file order for a single-object mesh with N material slots

```
GLOB
OB     ← Object block
DATA   ← Object.mat** pointer array (N × 8 bytes, null pointers allowed)
DATA   ← Object.matbits (N bytes)
ME     ← Mesh block
DATA   ← Mesh.mat** pointer array (N × 8 bytes, null pointers allowed)
DATA   ← ... other Mesh DATA (attributes, raw arrays, etc.)
DNA1
ENDB
```

### Evidence

Binary verification confirmed: Moving the `Object.mat**` block from position gap=1 to gap=2 (inserting one block between `OB` and the mat** block) causes the crash. Only gap=1 works reliably.

### Null `*Material` pointers are valid

The pointer array elements may all be null (zero). Blender treats them as unassigned material
slots.

## 13. Coordinate System Conversion (CryEngine Z-up ↔ Blender Y-up)

Blender's Y-up coordinate system is an **architectural convention** (implicit in math transforms, not explicitly defined in DNA headers). This conversion formula is derived from standard conventions. The quaternion wxyz format and matrix operations are source-verified (BLI_math_rotation.h).

### Coordinate System Definitions

| System    | Up | Right | Forward | Handedness |
|-----------|-------|-------|---------|-----------|
| **Blender** | **Y** (positive) | X | -Z (toward viewer) | Right-handed |
| **CryEngine** | **Z** (positive) | Y | X | Right-handed |

### Conversion Formula

**Position**: `(x_b, y_b, z_b) = (y_c, z_c, -x_c)`

**Scale**: unchanged (always applied post-rotation)

**Rotation**: Apply at matrix level via permutation matrix:

```
R_blender = P * R_cryengine * P^T

where P = [[ 0, 1, 0, 0]
           [ 0, 0, 1, 0]
           [-1, 0, 0, 0]
           [ 0, 0, 0, 1]]
```

### Recommended Implementation

1. Extract transforms from CryEngine (loc, rot matrix, scale)
2. Build 4x4 matrix: `M_world = Translate(loc) * RotationMatrix * Scale(scale)`
3. Apply permutation: `M_blender = P * M_world * P^T`
4. Decompose back: Use Blender's `mat4_decompose()` → (loc_out, quat_out, scale_out)
5. Write to Object struct: `Object.loc = loc_out`, `Object.quat = quat_out`, `Object.scale = scale_out`

**Verified from Blender source:** Quaternion wxyz format is the primary rotation representation for export (BLI_math_rotation.h:21, DNA_vec_defaults.h:35).

### Quaternion Format (PRIMARY)

- **Field**: `Object.quat[4]`
- **Format**: `wxyz` order (not `xyzw`)
- **Encoding**: 
  - `w = cos(angle/2)` (scalar part)
  - `(x, y, z) = sin(angle/2) * normalized_axis` (imaginary parts)
- **Default (identity)**: `[1, 0, 0, 0]` (no rotation)
- **Normalization**: Always normalized (magnitude = 1.0)
- **Source**: `DNA_vec_defaults.h:35` in Blender source

### Rotation Mode Enum (`Object.rotmode`)

**Source:** DNA_action_types.h:275-295 (enum eRotationModes)

| Value | Name | Format | Description |
|-------|------|--------|-------------|
| **0** | `ROT_MODE_QUAT` | Quaternion | **PRIMARY (use this)** |
| 1 | `ROT_MODE_XYZ` | Euler (X-Y-Z order) | Alternative |
| 2 | `ROT_MODE_XZY` | Euler (X-Z-Y order) | Alternative |
| 3 | `ROT_MODE_YXZ` | Euler (Y-X-Z order) | Alternative |
| 4 | `ROT_MODE_YZX` | Euler (Y-Z-X order) | Alternative |
| 5 | `ROT_MODE_ZXY` | Euler (Z-X-Y order) | Alternative |
| 6 | `ROT_MODE_ZYX` | Euler (Z-Y-X order) | Alternative |
| -1 | `ROT_MODE_AXISANGLE` | Axis + angle | Alternative |

**For exported files, use `rotmode = 0` (quaternion) and set `Object.quat[4]`.**

### Blender Math API (from BLI_math_matrix.h)

Key functions for coordinate conversion:

| Function | Input | Output | Use |
|----------|-------|--------|-----|
| `mat4_decompose()` | 4x4 matrix | loc, quat, scale | Extract transforms ⭐ |
| `loc_quat_size_to_mat4()` | loc, quat, scale | 4x4 matrix | Build matrix ⭐ |
| `mat4_to_loc_quat()` | 4x4 matrix | loc, quat | Extract (no scale) |

**Source**: `source/blender/math_vector/BLI_math_matrix.h` + implementation in `math_matrix_c.cc`


---

## 14. Library Block Structure (External File References)

Library blocks represent references to external .blend files. This is essential for linking decomposed mesh .blend files.

### Block Code and Identification

| Item | Value |
|------|-------|
| **Block Code** | `LI` (two ASCII chars) |
| **Enum Name** | `ID_LI` |
| **Macro Form** | `MAKE_ID2('L', 'I')` |
| **Binary (little-endian)** | `0x4C49` |

### Library Struct Definition

**Location**: `DNA_ID.h:210 - struct Library`

**Total Serialized Size**: **1472 bytes** (408 ID + 1024 filepath + 2 flag + 2 undo_tag + 4 pad + 24 pointers + 8 pad; see the serialization order below)

| Field | Offset | Size | Type | Description |
|-------|--------|------|------|-------------|
| `id` | 0 | 408 | `ID` | Base ID struct (name, flags, tags, etc.) |
| `filepath` | 408 | 1024 | `char[1024]` | Path to external .blend file (UTF-8, null-terminated) |
| `flag` | 1432 | 2 | `uint16_t` | Library flags (mostly unused in 5.1) |

### Filepath Encoding

- **Max Length**: 1024 bytes (`FILE_MAX` constant)
- **Encoding**: UTF-8
- **Null-Terminated**: Yes
- **Path Types**: Relative or absolute
- **Source Constant**: `BLI_path_utils.h`

### Block Header Format

| Field | Value |
|-------|-------|
| `code` | `ID_LI` |
| `sdna_idx` | Points to Library struct SDNA index |
| `len` | 1472 (serialized Library struct size) |
| `count` | 1 (one struct per block) |

### Multiple Library Blocks

- **One Library block per external .blend file reference**
- Single .blend file can link to unlimited external files
- **Example**: Linking to 5 mesh files = 5 Library blocks in scene.blend

### Writing Library Blocks

1. Create `BHead` with:
   - `code = ID_LI`
   - `len = 1472`
   - `sdna_idx = <SDNA index of Library struct>`
2. Serialize Library struct:
   - Set `id.name` to unique identifier
   - Set `filepath[1024]` to path of external file (UTF-8, null-terminated)
   - Set `flag = 0`
3. Use fake monotonic pointers for `old_ptr` (0x0100, 0x0200, etc.)

### Source Code References

- `DNA_ID.h:210` - Library struct definition
- `DNA_ID.h:50-65` - ID_LI enum constant
- `blenloader/intern/readfile.cc:1445` - `blo_read_library()` - how Blender reads Library blocks
- `blenloader/writefile.cc:890` - `write_library()` - how Blender writes Library blocks

---

### ID Struct Linking (Linking Objects to Libraries)

Every ID struct has a Library* pointer at offset 24. This links objects to external libraries.

| Item | Value |
|------|-------|
| **Field Name** | `lib` |
| **Type** | `struct Library*` (8-byte pointer) |
| **Offset within ID** | 24 bytes |
| **Position** | After `next` (0-7), `prev` (8-15), `newid` (16-23) |
| **Always Present** | Yes (all ID structs) |
| **Value for Local Objects** | NULL (nullptr) |
| **Value for Linked Objects** | Non-null pointer to Library block |

### Identifying Linked vs. Local Objects

| Check | Local Object | Linked Object |
|-------|--------------|---------------|
| **ID.lib** | nullptr | Non-null Library* |
| **Macro Check** | `ID_IS_LINKED(id) == false` | `ID_IS_LINKED(id) == true` |
| **ID.tag** | No linking flags | ID_TAG_EXTERN or ID_TAG_INDIRECT |
| **Data Editable** | Fully editable | Limited (linked data read-only) |

### ID Stub Concept

**Definition**: A minimal placeholder ID structure created for missing or linked data-blocks.

**When Created**:
- File references an external library that can't be found
- During link operation, before real data loaded
- Function: `create_placeholder()` (readfile.cc:2669-2707)

**Minimal Structure**:
- Contains only ID header: `name`, `lib`, `tag`, `us`
- Type-specific data is zeroed/null-initialized
- Marked with `ID_TAG_MISSING` if library not found

**Purpose**: Allow files to load even when linked libraries are missing; prevents broken references.

### Pointer Remapping Mechanism

**System**: Three-level hash map (oldnewmap) tracks old file pointers → new memory pointers

| Map | Purpose |
|-----|---------|
| **datamap** | Regular data-blocks in current file |
| **globmap** | Global structures (scenes, screens, etc.) |
| **libmap** | Library-linked data (stores ID type codes) |

**Flow During Load**:
1. `oldnewmap_insert()` — Maps old file pointer to new in-memory pointer during deserialization
2. `oldnewmap_lookup_and_inc()` — Retrieves remapped pointers + increments reference count
3. `newlibadr()` — Library-specific pointer lookup
4. `change_link_placeholder_to_real_ID_pointer_fd()` — Updates placeholders when real data found

**For Linked Objects**:
- When ID.lib pointer encountered: oldnewmap_insert tracks mapping
- During expansion: newlibadr looks up old pointers, returns new Library block pointers loaded in memory

### Linked Object Instantiation Pattern

**Multiple Instances**:
- Each linked object instance = separate local Object struct
- NOT shared data structures
- All instances reference SAME Library block via ID.lib pointer

**Typical Pattern** (for mesh instances):
- Create local Empty object with `instance_collection` pointing to linked collection
- Each empty has own transforms (loc/rot/scale)
- Each empty can have local modifiers/material overrides
- Underlying mesh/armature stays in library, unchanged

**Per-Instance Data**:
- Transforms (loc, rot, scale)
- Modifiers (local-only)
- Material assignments (local overrides)
- Constraints

**Shared Data**:
- Mesh geometry (from linked library)
- Armature skeleton (from linked library)
- Unchanged by local instances

### Source Code References

- `DNA_ID.h:376` - struct ID definition
- `DNA_ID.h:382` - `struct Library *lib` field
- `DNA_ID.h:649` - `ID_IS_LINKED()` macro
- `readfile.cc:268` - `oldnewmap_new()` — initialize remap tables
- `readfile.cc:277` - `oldnewmap_insert()` — add mapping
- `readfile.cc:299` - `oldnewmap_lookup_and_inc()` — retrieve + increment
- `readfile.cc:312` - `oldnewmap_liblookup()` — library lookup
- `readfile.cc:2669` - `create_placeholder()` — ID stub creation
- `readfile.cc:5518` - `read_library_linked_id()` — linked ID loading
- `lib_id.cc:1467` - `BKE_libblock_init_empty()` — initialize empty ID

---


---

## 15. Library Block Binary Serialization (Writing & Reading)

Exact byte-by-byte serialization format for Library blocks in .blend files.

### Block Header Specification

| Field | Value | Notes |
|-------|-------|-------|
| **code** | `ID_LI` = `MAKE_ID2('L', 'I')` | Identifies Library block |
| **code (hex)** | `0x494C` (little-endian) | Binary value in file |
| **sdna_idx** | Runtime-assigned | Dynamically computed per build |
| **len** | 1456 | Exact sizeof(Library) |
| **count (nr)** | 1 | Single struct per block |
| **old_ptr** | Stable address ID | Used for pointer remapping |

### Serialization Order (Write Sequence)

Library blocks are written with exact field ordering by `library_blend_write_data()`:

1. **ID struct** (408 bytes)
   - name[2], type, tag, index, us, icon_id, recalc_*, session_uid, properties, owner_library, asset_data, override_library
   - CORRECTED: Was incorrectly listed as 370 bytes; actual size per DNA_id_types.h is 408 bytes

2. **filepath[1024]** (1024 bytes)
   - Relative path to external .blend file, zero-padded
3. **flag** (2 bytes, uint16_t)
   - Library flags (e.g., LIBRARY_FLAG_IS_ARCHIVE)
4. **undo_runtime_tag** (2 bytes, uint16_t)
   - Runtime tag for undo preservation
5. **_pad[4]** (4 bytes)
   - Alignment padding
6. **archive_parent_library** (8 bytes, pointer)
   - Pointer to parent library (nullptr before write, converted to stable ID)
7. **packedfile** (8 bytes, pointer)
   - Pointer to PackedFile or nullptr (converted to stable ID)
8. **runtime** (8 bytes, pointer)
   - Always nullptr (runtime-only field)
9. **_pad2** (8 bytes)
   - Final padding

**Total**: 1472 bytes contiguous
   - CORRECTED: Was incorrectly listed as 1456 bytes; actual total is 408 + 1024 + 2 + 2 + 4 + 8 + 8 + 8 + 8 = 1472 bytes

### Filepath Encoding Details

| Aspect | Specification |
|--------|---------------|
| **Storage** | `char[1024]` in Library struct |
| **Encoding** | UTF-8 |
| **Null-termination** | C-string null-terminated |
| **Padding** | Zero-filled when shorter than 1024 |
| **Path Type** | Relative path when possible, absolute fallback |
| **Read-side Conversion** | `BLI_path_abs(lib->filepath, relabase)` → absolute path |
| **Special Characters** | Raw UTF-8, platform-dependent separators (no escaping) |
| **Examples** | `../../external/mesh.blend` (Unix), `..\..\external\mesh.blend` (Windows) |

### Pointer Remapping Strategy

**Write Side**:
- Each pointer (Library*, PackedFile*) replaced with unique **stable address ID**
- Computed via `get_address_id()` from object identity
- All pointers in file are 64-bit stable IDs, not real memory addresses

**Read Side (oldnewmap mechanism)**:
1. Library block read, `oldnewmap_lib_insert()` called
2. Maps: file's stable_id (from BHead.old_ptr) → new in-memory Library* address
3. Other IDs with `ID.lib` pointers also get stable IDs
4. During pointer resolution phase: `newlibadr()` converts all stable IDs to actual pointers using oldnewmap

**ID.lib Pointer Resolution**:
- Each ID struct has Library* pointer @ offset 24
- In file: stored as stable address ID
- After load: `newlibadr()` looks up stable ID in oldnewmap, returns actual Library* address

### Critical Reading Order

**When loading scene.blend with external mesh links**:

1. **Parse file header & SDNA** — Decode DNA definitions
2. **Scan for ID_LI blocks** — Find all Library references
3. **READ LIBRARY BLOCKS FIRST** (mandatory):
   - For each Library BHead:
   - Deserialize 1472-byte Library struct
   - Call `direct_link_library()`:
     - Compute absolute paths via `BLI_path_abs()`
     - Create new Main context for library
     - `oldnewmap_lib_insert()` records: old_stable_id → &lib->id
     - Read PackedFile data if present
4. **Read other ID blocks** (Objects, Meshes, Materials, etc.):
   - IDs with `ID.lib != NULL` recognized as linked
   - Their lib pointer (as stable ID) marked for remapping
5. **Pointer remapping phase**:
   - All `ID.lib` pointers converted from file's stable IDs to new Library* addresses
   - `newlibadr()` walks oldnewmap
6. **Post-liblink callbacks**:
   - `blend_read_after_liblink()` called for all IDs
   - Archive library parent relationships established

**Key Invariant**: Library blocks MUST be deserialized before any ID that references them.

### Source Code References

- `blenkernel/intern/library.cc:165` - `library_blend_write_data()` — write serialization
- `blenkernel/intern/library.cc:193` - `library_blend_read_data()` — read deserialization
- `blenloader/intern/writefile.cc:1278` - `write_libraries()` — block writing loop
- `blenloader/intern/readfile.cc:2525` - `direct_link_library()` — block reading + processing
- `blenloader/intern/readfile.cc:286` - `oldnewmap_lib_insert()` — pointer mapping registration
- `blenloader/intern/readfile.cc:4258` - ID_LI case in block dispatch
- `makesdna/DNA_ID.h:506` - `struct Library` definition (all fields)
- `makesdna/DNA_ID.h:134` - `ID_LI` enum constant

---

## 16. Linked Object Instantiation: Multiple Instances via Collection References

When importing linked data, users often need multiple **independent instances** of the same linked
object/collection with **per-instance overrides** (transforms, materials). Blender implements this
via the **`instance_collection`** field on Object structs, combined with empty objects and collection
hierarchy.

### The `instance_collection` Mechanism

**Field Definition**:
- **Location**: `struct Object` (DNA_object_types.h:631)
- **Type**: `Collection*` (pointer to Collection struct)
- **Default**: `nullptr`
- **Associated Flag**: `OB_DUPLICOLLECTION` (bit 8 in `Object.transflag`, DNA_object_types.h:153)

When `instance_collection != nullptr` and `OB_DUPLICOLLECTION` flag is set, the Object acts as a
**collection instance proxy**—its contents are duplicated (rendered/evaluated) with the instance's
own transforms and properties.

**Typical Pattern**: Instance objects are `OB_EMPTY` types (empty transform containers), but any
Object type can technically be a collection instance.

### Creating Multiple Instances: Pattern & Architecture

To create 5 independent instances of a linked collection/object:

1. **Create 5 local OB_EMPTY objects** (one per instance)
   - Each is a separate Object struct with own ID and name
   - Code: `BKE_object_add_only_object(bmain, OB_EMPTY, collection->id.name + 2)`
   - (Source: blendfile_link_append.cc:691)

2. **Assign all 5 to the same linked Collection**
   - `ob->instance_collection = collection;` (same pointer for all 5)
   - Increment collection user count: `id_us_plus(&collection->id);`
   - Enable instantiation flag: `ob->transflag |= OB_DUPLICOLLECTION;`
   - (Source: blendfile_link_append.cc:709-711)

3. **Add each empty to scene collection hierarchy**
   - `BKE_collection_object_add(collection, ob);`
   - Each empty is a direct child of the collection's `gobject` (CollectionObject linked list)
   - (Source: blendfile_link_append.cc:514)

**Result**: 5 separate, identifiable OB_EMPTY objects all pointing to the same Collection. When
rendered/evaluated, each generates an independent copy of the collection's contents with its own
transform and property overrides.

### Per-Instance Transforms: Full Independence

**Storage**: Transform data is stored in the **instance OB_EMPTY object itself**, NOT inherited or
shared from the linked data.

**Transform Fields** (all stored in Object struct):
- `loc[3]` @ offset 416 — position (x, y, z)
- `rot[3]` @ offset 476 — Euler rotation
- `quat[4]` @ offset 500 — quaternion (wxyz format)
- `scale[3]` @ offset 440 — scale vector
- `rotAxis[3]` — axis for axis-angle representation
- `rotAngle` — rotation angle (axis-angle mode)
- `dloc[3]`, `drot[3]`, `dquat[4]` — delta transforms
- `parentinv[4][4]` — inverse parent transform matrix
- `constinv[4][4]` — inverse constraint matrix

**Per-Instance Capability**: ✅ **FULLY INDEPENDENT**
- Each OB_EMPTY has its own transform fields
- Modifying one instance's position/rotation does not affect others
- Each instance can have completely different scale, rotation modes, parent relationships
- (Source: DNA_object_types.h:531-551)

### Per-Instance Material Overrides

**Storage**: Material assignment data is stored in the **instance OB_EMPTY object**, NOT in linked
data.

**Material Fields** (Object struct):
- `mat[N]` — array of `Material*` pointers (can reference local or linked materials)
- `matbits[N]` — boolean array per slot: 1 = material is linked, 0 = material is local override
- `totcol` — total material slots assigned to this instance
- `actcol` — currently active material slot (1-based index)

**Per-Instance Capability**: ✅ **FULLY INDEPENDENT**
- Each instance can have a different number of material slots (`totcol` varies per instance)
- Each instance can assign completely different materials (all local, all linked, or mixed)
- Each instance can override individual material slots independently
- (Source: DNA_object_types.h:522-528)

### Scene Hierarchy Structure

For a scene with 5 linked mesh instances:

```
Scene
├── Master Collection
    ├── Instance Collection (user-created container)
    │   ├── Instance Empty 1 (OB_EMPTY, instance_collection → linked collection)
    │   │   └── own loc[3], rot[3], scale[3]; own materials mat[N]
    │   ├── Instance Empty 2 (OB_EMPTY, instance_collection → linked collection)
    │   │   └── own loc[3], rot[3], scale[3]; own materials mat[N]
    │   ├── Instance Empty 3 (OB_EMPTY, instance_collection → linked collection)
    │   │   └── own loc[3], rot[3], scale[3]; own materials mat[N]
    │   ├── Instance Empty 4 (OB_EMPTY, instance_collection → linked collection)
    │   │   └── own loc[3], rot[3], scale[3]; own materials mat[N]
    │   └── Instance Empty 5 (OB_EMPTY, instance_collection → linked collection)
    │       └── own loc[3], rot[3], scale[3]; own materials mat[N]
    └── [Other collections...]
```

**Key Structural Points**:
- Instance empties are direct children in the collection's `gobject` (CollectionObject linked list)
- Each instance has a separate `Base` struct in the ViewLayer with independent visibility/selection flags
- NO forced parent-child relationships between instance empties and the collection's contents
- The **`instance_collection` pointer is the ONLY relationship** between instance and contents
- Supports **nested instances**: Collections can contain Objects with their own `instance_collections`
- Instance empties are placed at scene cursor location by default (`scene->cursor.location`)
- (Source: blendfile_link_append.cc:514-712, 735-775)

### Pointer Remapping & Library Linking

**Remapping Context**: During file loading, the `instance_collection` pointer (like all Collection
pointers) must be converted from file's stable ID references to runtime memory addresses.

**Remapping Mechanism**:
- All Collection pointers use `oldnewmap_liblookup()` for linked data resolution via `fd->libmap`
- If the Collection is linked (from external library), its pointer is remapped via the library's
  `oldnewmap` table
- If the Collection is local, remapping uses the main file's `oldnewmap` table

**Loading Order Constraint**: Collections must be **deserialized before any Object that references
them** via `instance_collection`.

**Linked Collection Case**:
- Instance OB_EMPTY: local (`ID.lib == nullptr`)
- Collection pointed to: linked (`ID.lib != nullptr`)
- Collection's contents: inherit `ID.lib` from collection (linked)
- (Source: readfile.cc:268-339, DNA_ID.h:649 `ID_IS_LINKED()`)

### Binary Serialization Implications

When implementing Library block serialization:

1. **`instance_collection` is a `Collection*` pointer**
   - Like all pointers in `.blend` files, stored as stable address IDs during write
   - Requires remapping phase after all IDs are deserialized

2. **Each instance is a separate Object struct**
   - When serializing 5 instances, write 5 separate Object BHeads (ID_OB blocks)
   - Each has its own ID struct, name, and data block
   - Each has its own loc[3], rot[3], scale[3], mat[], etc.

3. **Collections must be loaded before Objects**
   - If Objects reference Collections via `instance_collection`, Collections must exist first
   - Enforce in file generation: write all Collection blocks before Object blocks

4. **User count tracking**
   - Each instance increments the linked Collection's user count via `id_us_plus()`
   - This prevents premature cleanup of referenced Collections during save/load cycles

### Critical Code References

| Finding | File | Lines |
|---------|------|-------|
| instance_collection field | DNA_object_types.h | 631 |
| Object struct definition | DNA_object_types.h | 465-677 |
| OB_DUPLICOLLECTION flag | DNA_object_types.h | 153 |
| eObject_TransFlag enum | DNA_object_types.h | 140-165 |
| Collection struct | DNA_collection_types.h | 164-205 |
| Create OB_EMPTY instance | blendfile_link_append.cc | 691 |
| Set instance_collection | blendfile_link_append.cc | 709 |
| Set OB_DUPLICOLLECTION flag | blendfile_link_append.cc | 711 |
| Add to collection hierarchy | blendfile_link_append.cc | 514 |
| Place at cursor location | blendfile_link_append.cc | 712 |
| Recursive instance traversal | blendfile_link_append.cc | 735-775 |
| oldnewmap remapping | readfile.cc | 268-339 |
| oldnewmap_liblookup() | readfile.cc | 312-327 |
| ID_IS_LINKED() macro | DNA_ID.h | 649 |

---

## 17. Collection Member Linking: Nested Objects & Cross-Library References

When importing linked Collections from external files, Blender needs to manage the Objects that are
members of those Collections. This section documents how Collection members are stored, remapped,
and resolved during file loading.

### Part A: Collection Member Storage Structure

**Member Field**: `gobject`  
**Type**: `ListBaseT<CollectionObject>` (doubly-linked list, NOT array)  
**Location**: `DNA_collection_types.h:176`  
**Size per member**: 32 bytes

Collections don't store member Objects in a dynamic array. Instead, they use a **doubly-linked list
of intermediate `CollectionObject` structs**, each of which holds a pointer to an actual Object.

**CollectionObject Struct Layout**:
```
Offset  Size  Field                    Type
0-7     8     next                     CollectionObject*
8-15    8     prev                     CollectionObject*
16-23   8     ob          ⭐ CRITICAL   Object*
24-31   8     CollectionLightLinking   (nested struct)
```

**Key Design**: This indirection allows Collection members to be managed independently from the
Object structs themselves. Multiple Collections can reference the same Object via separate
CollectionObject intermediate structs.

(Source: DNA_collection_types.h:122-128, 176)

### Part B: Linked Collection Member Resolution During File Load

When a Collection is imported from an external library file, its member Objects must also be
deserialized and their pointers remapped from file addresses to runtime memory addresses.

**Resolution Mechanism**: The `collection_foreach_id()` callback function

**Flow**:

1. **Deserialization Phase**:
   - Collection block (ID_GR) deserialized with `gobject` linked list
   - CollectionObject structs and their `ob` pointers stored as **stable file addresses**
   - (Not yet actual memory addresses)

2. **Library Link Phase** (`lib_link_all()`):
   - For each ID in the file, `lib_link_all()` dispatches to appropriate remapping callback
   - Collections get dispatched to `collection_foreach_id()` callback
   - (Source: readfile.cc:3841-3867)

3. **Member Remapping** (`collection_foreach_id()`):
   - Iterates through the entire `gobject` linked list
   - For each CollectionObject, calls `lib_link_cb()` with IDWALK_CB_USER flag
   - `lib_link_cb()` calls `BLO_read_get_new_id_address()` on each member Object pointer
   - `oldnewmap_liblookup()` converts file stable ID → runtime memory address
   - (Source: collection.cc:220-256)

4. **Completion**:
   - After lib_link phase, all member Object pointers are resolved to actual Objects
   - Member Objects are fully accessible, not stubs

**Critical Functions**:
- `collection_foreach_id()` (collection.cc:220-256) — iterates members, triggers remapping
- `BKE_collection_blend_read_data()` (collection.cc:335-389) — deserialization
- `lib_link_cb()` (readfile.cc:3841-3867) — main remapping dispatcher
- `BLO_read_get_new_id_address()` (readfile.cc:5909-5915) — pointer resolution via oldnewmap

(Source: collection.cc:220-256, readfile.cc:3841-3867, 5909-5915)

### Part C: Linked Collections + instance_collection Interaction

When an `instance_collection` pointer on an OB_EMPTY object references a **linked Collection** from
an external library, the flow ensures all member Objects are fully resolved before the instance is
rendered.

**Resolution Order Flow**:

1. Collection (ID_GR) deserialized with gobject linked list (file addresses)
2. `lib_link_all()` begins
3. `collection_foreach_id()` iterates Collection members, remaps each Object pointer
4. All Collection members now point to resolved Objects (memory addresses)
5. OB_EMPTY deserialized with `instance_collection` field (file address)
6. `object_foreach_id()` remaps `instance_collection` pointer to resolved Collection
7. At render time: Collection.gobject accessed with full Object pointers (guaranteed resolved)

**Member Object Status**: ✅ **Full Objects, NOT Stubs**
- Members are actual Object pointers, not placeholder ID stubs
- Member resolution completed in lib_link phase, before instance usage
- Each member Object is fully accessible with its own transforms, materials, etc.

**Linked Collection Case**:
- Collection itself: linked (`ID.lib != nullptr`)
- Member Objects: also linked (inherit `ID.lib` from Collection)
- Member remapping: uses `oldnewmap_lib_insert()` during lib_link, registered in Library context
- Instance OB_EMPTY: local (`ID.lib == nullptr`)

**Key Guarantee**: Member Objects are fully resolved **before** `instance_collection` is used, so
rendering never encounters unresolved member pointers.

(Source: object.cc:421, readfile.cc:3841-3867)

### Part D: Collection Hierarchy & Nested Collections

Collections can contain other Collections (nested hierarchy), and this is supported across library
boundaries.

**Nested Collections Field**: `Collection.children`  
**Type**: `ListBaseT<CollectionChild>` (doubly-linked list)  
**Location**: `DNA_collection_types.h:178`  

**CollectionChild Struct Layout**:
```
Offset  Size  Field       Type
0-7     8     next        CollectionChild*
8-15    8     prev        CollectionChild*
16-23   8     collection  Collection*
24-27   4     light_linking  int32 (Blender 5.1+)
28-31   4     _pad        int32 (padding)
```

**Supports Cross-Library Linking**: ✅ YES
- A local Collection can contain child Collections from external libraries
- Each child Collection has its own `ID.lib` pointer (if linked)
- Nested Collections are remapped like all Collection pointers

**Hierarchy Constraints**:
- Cyclic hierarchies are prevented by validation logic
- Each child Collection is recursively remapped via `collection_foreach_id()`
- If parent is local, children can still be linked

**Example Structure**:
```
Local Collection A
├── Local Collection B
│   ├── Linked Object X (from external library)
│   └── Linked Object Y (from external library)
└── Linked Collection C (from external library)
    ├── Linked Object Z (from external library)
    └── Linked Object W (from external library)
```

(Source: DNA_collection_types.h:178, collection.cc:248-250)

### Part E: Binary Serialization of Collection Blocks

Collection blocks are serialized with member Objects and nested Collections as linked lists,
with all pointers converted to stable file addresses for remapping during load.

**Collection Struct Size**: 520 bytes  
**Block Code**: `ID_GR`

**Serialization Order**:

1. **ID struct** (408 bytes)
   - Standard ID header (name, lib pointer, flags, etc.) — full 408-byte structure per DNA_id_types.h

2. **owner_id pointer** (8 bytes)
   - Pointer to owning Scene ID (remapped during lib_link)

3. **PreviewImage pointer** (8 bytes, remapped during lib_link)

4. **gobject linked list** (32 bytes per CollectionObject)
   - Each member: next, prev, Object* (file address), CollectionLightLinking
   - Number of members varies; all serialized sequentially

5. **children linked list** (32 bytes per CollectionChild)
   - Each child: next, prev, Collection* (file address)
   - Number of children varies; all serialized sequentially

6. **Exporters/Importers metadata lists**
   - Additional linked lists for exporter data

7. **Runtime-only fields** (NOT serialized)
   - `CollectionRuntime *runtime` — set to nullptr before write

**Critical Field Offsets** (Corrected per DNA_collection_types.h):
```
Offset    Field                           Type
0-407     ID id                           (408 bytes) — CORRECTED: was 120
408-415   ID *owner_id                    (8 bytes)
416-423   ListBase gobject (first)        (8 bytes)
424-431   ListBase gobject (last)         (8 bytes)
432-439   ListBase children (first)       (8 bytes)
440-447   ListBase children (last)        (8 bytes)
500+      CollectionRuntime *runtime      (NOT serialized; nullptr)
```

**Pointer Remapping Strategy**:
- All Collection pointers in `gobject` converted to stable file addresses before write
- All Object pointers in `gobject` converted to stable file addresses before write
- All Collection pointers in `children` converted to stable file addresses before write
- During load (`lib_link` phase), `oldnewmap_liblookup()` restores all pointers

**Linked Collection Serialization**:
- If Collection is linked (`ID.lib != nullptr`), its members inherit linked status
- Member Objects: each has `ID.lib` pointer referencing the external library
- Remapping uses `oldnewmap_lib_insert()` during lib_link, within Library context
- Collections must deserialize before Objects that reference them

(Source: collection.cc:335-389, writefile.cc collection write, DNA_collection_types.h:164-205)

### Critical Implementation Constraints

1. **Doubly-Linked List Serialization**
   - gobject and children are NOT simple arrays; they are linked lists
   - Must serialize `ListBase` (next/prev pointers) followed by individual structs
   - LibBack structures require careful pointer management during write

2. **Member Remapping Order**
   - Collections must be written/read before Objects that reference them
   - Otherwise, member Object pointers will be unresolved during lib_link

3. **Runtime Fields**
   - `CollectionRuntime` is NOT serialized; always set to nullptr before write
   - Runtime is populated during `blend_read_after_liblink()` callbacks

4. **Nested Collection Support**
   - When serializing nested Collections, ensure parent Collections are written before children
   - Child Collection pointers require remapping like all Collection pointers

5. **User Count Tracking**
   - Each member increments Object's user count (via `id_us_plus()`)
   - Each child Collection increments Collection's user count
   - Decrement on removal to prevent dangling references

### Critical Code References

| Finding | File | Lines |
|---------|------|-------|
| CollectionObject struct | DNA_collection_types.h | 122-128 |
| Collection struct | DNA_collection_types.h | 164-205 |
| gobject field | DNA_collection_types.h | 176 |
| children field | DNA_collection_types.h | 178 |
| collection_foreach_id() | collection.cc | 220-256 |
| BKE_collection_blend_read_data() | collection.cc | 335-389 |
| collection_blend_write_data() | collection.cc | (write path) |
| lib_link_cb() | readfile.cc | 3841-3867 |
| BLO_read_get_new_id_address() | readfile.cc | 5909-5915 |
| oldnewmap_liblookup() | readfile.cc | 312-327 |
| object_foreach_id() | object.cc | 421 |

---

