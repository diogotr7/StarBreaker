# SOCPAK Structure for Visual Export

## Scope

This note documents `.socpak` contents for StarBreaker export work. It focuses
on data that can affect a Blender-visible result: meshes, mesh instances,
materials, textures, decals, POM, tint palettes, vertex colors, lights, signs,
screens, and nested object-container placement.

It intentionally does not try to preserve game-only behavior such as transit
logic, shopping logic, AI, mission links, audio triggers, room navigation, or
spawn logic unless that data also drives a visible object.

Evidence used here:

- LIVE `starbreaker-mcp` P4K lookups against exact archive paths.
- Fresh extraction under `%TEMP%\starbreaker-socpak-analysis`.
- Locally extracted socpak examples (outside the repo).
- Current StarBreaker parser code in `crates/starbreaker-3d/src/socpak.rs`,
  `included_objects.rs`, and `pipeline/interiors.rs`.

Older temp extraction artifacts were not used. Code references below name
functions rather than line numbers; the "Current StarBreaker Gaps" section
records what the code does today.

## Sample Set

Primary examples:

| SOCPAK | Why it was sampled | Files after ZIP extraction |
| --- | --- | ---: |
| `Data\ObjectContainers\PU\loc\mod\common\hangar\hightech_a\hangar_xltop_001_newbab.socpak` | Location-specific modifier over a base hangar | 9 |
| `Data\ObjectContainers\PU\loc\mod\common\hangar\hightech_a\hangar_xltop_001.socpak` | Large composed hangar with repeated children | 116 |
| `Data\ObjectContainers\PU\loc\mod\common\hangar\hightech_a\hangar_component_module_cargo_elevator_xl.socpak` | Child module referenced twice by the hangar | 7 |
| `Data\ObjectContainers\PU\loc\flagship\stanton\orison\orison_ind\orison_ind_lz_int.socpak` | Interior landing zone with many decals, displays, NPCs, and shop modifiers | 105 |
| `Data\ObjectContainers\PU\loc\mod\stanton\station\ser\reststop_ext\rs_ext_mic-leo1.socpak` | Exterior station with many repeated child OCs and one `.ale` | 102 |

Second-tier containers were extracted by exact full path after they appeared in
root manifests:

| Referenced SOCPAK | Parent evidence | Files |
| --- | --- | ---: |
| `Data\ObjectContainers\Setup\elevator_setup\elev_hightech\elev_ht_cargo_hangar_xlrg_a.socpak` | Child of `hangar_component_module_cargo_elevator_xl` | 6 |
| `Data\ObjectContainers\Setup\elevator_setup\playerhangar\playerhangar_elev_hangar_ht_gateway.socpak` | Repeated child of `hangar_xltop_001` | 8 |
| `Data\ObjectContainers\PU\Shops\admin\admin_small_orison_a.socpak` | `ObjectContainerModifier` child of Orison | 13 |
| `Data\ObjectContainers\PU\loc\mod\common\weapon_platform\wpn_pltfrm_space\wpn_pltfrm_spce_a.socpak` | Repeated child of `rs_ext_mic-leo1` | 5 |
| `Data\ObjectContainers\PU\loc\mod\stanton\station\ser\reststop_comm\rs_comm_mic-leo1.socpak` | Rest-stop communication ring child of `rs_ext_mic-leo1`; contains many station screens | 110 |
| `Data\ObjectContainers\PU\loc\mod\stanton\station\ser\reststop_entry\rs_entry_mic-leo1.socpak` | Rest-stop entry child of `rs_ext_mic-leo1`; contains more station screens and RTT objects | 183 |
| `Data\ObjectContainers\PU\loc\mod\common\ext_cargo\station_ext_cargo_elevator_001.socpak` | Exterior cargo elevator child; contains local cubemap DDS and a base child reference | 8 |
| `Data\ObjectContainers\Setup\elevator_setup\reststop\reststop_elev_atc.socpak` | Rest-stop ATC elevator child; mostly transit/elevator metadata | 6 |
| `Data\ObjectContainers\PU\loc\mod\stanton\station\ser\reststop_cargo\reststop_cargo_occu_0001.socpak` | Rest-stop cargo child with cargo-service signs, screens, decals, and lights | 73 |

Supporting non-SOCPAK samples:

- `Data\Localization\english\global.ini`, extracted from P4K to verify
  `Port Tressler` and Rest & Relax localization keys.
- P4K texture previews for
  `Data\UI\Textures\Props\Rest_Stop\RestandRelax\restandrelax_welcome.dds`
  and `restandrelax_elevator_a.dds`, which contain the Rest & Relax logo and
  readable screen text.

`p4k_read` accepts slash or backslash paths, but exact `p4k_search` matched the
archive form with backslashes, for example:

```text
Data\ObjectContainers\PU\loc\mod\common\hangar\hightech_a\hangar_component_module_cargo_elevator_xl.socpak
```

## High-Level Model

A `.socpak` is a ZIP/P4K-like archive stored as one file inside `Data.p4k`.
StarBreaker already proves this by reading the outer P4K entry and then parsing
the bytes with `P4kArchive::from_bytes`.

Most sampled archives use this shape:

```text
<name>.socpak
  <name>.xml
  <name>_editor.xml
  <name>.soc
  <name>.rmxml
  <name>.brmp
  <name>.altg
  <name>/
    entdata/*.entxml
    metadata/*.xml
    brush/*.cgf
    brush/*.cgfm
    cubemaps/**/*.dds       # not universal, observed in admin_small_orison_a
```

The archive can also contain `.ale`; in the sampled station exterior this is a
CryXMLB-backed monitored-zone file, not a renderable object. Keep it parseable
for diagnostics, but do not feed it into the visual exporter by default.

### `.soc` chunk types

The `.soc` file is a CrCh (chunked) container. The known chunk type IDs are:

| Type ID | Name | Content |
| --- | --- | --- |
| `0x0001` | RenderMesh (IVO) | Static mesh geometry. Not seen in socpak `.soc` files (these use `IncludedObjects` instead). |
| `0x0004` | CryXMLB | Entity data — lights, decals, screens, canvas text, props, and game-logic entities. Parsed as XML with `<Entities>` or `<SCOC_Entities>` root containers. |
| `0x0010` | IncludedObjects | Pre-baked static mesh placements: CGF paths, material paths, tint palette paths, and per-object 3×4 f64 transform matrices. |

The existing parser (`socpak.rs` `parse_soc`) iterates all chunks and handles
`IncludedObjects` and `CryXMLB`. Any other chunk type is silently skipped.

### Multiple `.soc` files

A socpak can contain more than one `.soc` file. The existing code
(`socpak.rs` `load_interior_from_socpak`) iterates **all** `.soc` entries, which is intentional:
the main `.soc` carries `IncludedObjects` geometry, while child `.soc` files
carry lights, VFX, and entity data. All are parsed and merged into a single
`InteriorPayload`.

## File Roles

| Entry | Observed header/root | Export relevance |
| --- | --- | --- |
| `<name>.xml` | Text XML, root `<ObjectContainer>` | Primary manifest. Contains object-container identity, bounds, component metadata, streaming OC info, and `ChildObjectContainers` with child paths and transforms. |
| `<name>_editor.xml` | Text XML | Editor/layer index. Useful for counts and object classes such as `Brush`, `GeomEntity`, `Decal`, `Light`, and `SCItemDisplayScreen`, but sampled objects often only store name/type/GUID here. |
| `<name>.soc` | `CrChF` | Main runtime visual payload. Contains `IncludedObjects` chunks with static mesh paths, material paths, tint palette paths, and per-object transforms, plus `CryXMLB` entity chunks. |
| `<name>.rmxml` | `CryXmlB` | Room mapping. The current converter emits readable but not well-formed XML with `RoomMapping` and GUID/distance records. Likely room/navigation/culling metadata, not direct visual geometry. |
| `<name>.brmp` | `BMRP` | Binary room/brush mapping metadata. Not currently understood. Treat as non-visual until a render-relevant field is identified. |
| `<name>.altg` | `ALTG` | Binary tag/layer/group metadata. Not currently understood. Useful for future grouping, not required for first visual export. |
| `<name>.ale` | `CrChF` with `CryXmlB` | Area logic. In `rs_ext_mic-leo1`, the file defines `MonitoredZone-001` with position, rotation, and `radiusKm=150`; skip for visual export unless debug overlays are requested. |
| `entdata/*.entxml` | `CryXmlB` | Per-entity records. Repeats child OC references and contains entity components, GUIDs, and sometimes renderable entity classes. Good cross-check for `.soc` CryXML and root manifest. |
| `metadata/*.xml` | Text XML | Component metadata such as transit, shop, landing area, insurance, goto point, etc. Usually game logic, but can help classify why an OC exists. |
| `brush/*.cgf`, `brush/*.cgfm` | `#ivo` or `CrChF` | Local packaged geometry. Many sampled designer brushes are proxy/collision materials, but they should be parsed and filtered by material semantics rather than ignored by path. |
| `cubemaps/**/*.dds` | DDS | Embedded cubemap/light-probe textures. Observed in `admin_small_orison_a`; texture resolution should check the inner archive before falling back to outer P4K. |

### `.soc` CryXMLB entity structure

CryXMLB chunks inside `.soc` decode to XML with this hierarchy:

```xml
<SCOC_Entities>              <!-- or <Entities> as root tag -->
  <Entity EntityClass="Light" Name="Light-001"
          Pos="x,y,z" Rotate="qw,qx,qy,qz" Scale="sx,sy,sz"
          Material="materials/path" EntityClassGUID="{...}"
          Radius="5.0">
    <PropertiesDataCore>
      <EntityComponentLight lightType="Omni" useTemperature="0">
        <defaultState intensity="1.0" temperature="6500.0">
          <color r="1.0" g="0.95" b="0.9" />
        </defaultState>
        <sizeParams lightRadius="10.0" />
        <projectorParams texture="" FOV="0.0" />
      </EntityComponentLight>
      <!-- OR for geometry entities: -->
      <EntityGeometryResource>
        <Geometry>
          <Geometry>
            <Geometry path="objects/path/to/mesh.cgf" />
          </Geometry>
        </Geometry>
      </EntityGeometryResource>
    </PropertiesDataCore>
  </Entity>
  <!-- ... more Entity children ... -->
</SCOC_Entities>
```

Common entity-level attributes:

| Attribute | Type | Description |
| --- | --- | --- |
| `EntityClass` | string | Entity class name (`Light`, `LightBox`, `LightGroup`, `GeomEntity`, `Decal`, `Brush`, `CanvasDecal_Standalone`, etc.) |
| `Name` | string | Instance name within this container |
| `Pos` | CSV f64 | Local position as `x,y,z` |
| `Rotate` | CSV f64 | Local quaternion rotation as `qw,qx,qy,qz` |
| `Scale` | CSV f64 | Local scale as `sx,sy,sz` (default `"1,1,1"`) |
| `Material` | string | Optional material path override |
| `EntityClassGUID` | GUID string | When present, entity geometry/material must be resolved through DataCore `EntityClassDefinition` |
| `Radius` | f32 | Some entity types use this (e.g., lights as fallback radius) |

The code looks for `<Entities>` or `<SCOC_Entities>` containers
(`socpak.rs` `extract_cryxml_entities`), then iterates child `<Entity>` elements.

### `entdata/*.entxml` structure

Each `.entxml` file is a CryXMLB document with the same `<Entity>` schema
described above. The attributes are identical to `.soc` CryXMLB entities:
`EntityClass`, `Name`, `Pos`, `Rotate`, `Scale`, `Material`,
`EntityClassGUID`, and nested `PropertiesDataCore` children.

Entdata repeats child OC references (as `EntityClass="ObjectContainer"` or
`"ObjectContainerModifier"`) and serves as a cross-check for `.soc` CryXMLB
data. However, entdata `Pos` values use a different coordinate frame from root
manifest `pos` — prefer root manifest `ChildObjectContainers` for transform
composition and use entdata as a validation/component source.

### Unknown file formats

| Format | Header magic | Status |
| --- | --- | --- |
| `.rmxml` | `CryXmlB` | Decoded to readable but not well-formed XML with `RoomMapping` and GUID/distance records. Room/navigation/culling metadata — treat as non-visual until proven otherwise. |
| `.brmp` | `BMRP` | Binary room/brush mapping. Not decoded. Flagged for future reverse-engineering if culling/grouping data is needed for export. |
| `.altg` | `ALTG` | Binary tag/layer/group metadata. Not decoded. Useful for future grouping, not required for first visual export. |

### `.ale` area files

The `rs_ext_mic-leo1.ale` sample is only 736 bytes. Its strings identify a
single entity:

```text
Name=MonitoredZone-001
EntityClass=MonitoredZone
EntityComponentMonitoredZone
Pos=-5.4210108624275222e-20,0.00048799999999999999,-125
Rotate=0.70710677,0,0,0.70710677
radiusKm=150
```

That lines up with a crime/security monitoring area rather than geometry,
decal, light, screen, or placement data. Export should skip `.ale` for Blender
for now. If future tooling wants area debug visualization, parse it through the
same CryXMLB path used for `.soc` entity chunks and emit an optional sphere or
volume overlay outside the normal visual scene.

## Object-Container Composition

Nested socpaks are declared in root `<name>.xml` under
`<ChildObjectContainers>`.

### `<Child>` attribute schema

Each `<Child>` element in `<ChildObjectContainers>` carries these attributes:

| Attribute | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | yes | Target `.socpak` path (case-insensitive, forward slashes) |
| `class` | string | yes | `ObjectContainer` or `ObjectContainerModifier` |
| `entityName` | string | yes | Placed instance name within the parent |
| `pos` | CSV f64 | yes | Local translation as `x,y,z` |
| `rot` | CSV f64 | yes | Local quaternion rotation as `qw,qx,qy,qz` (note: root manifest uses quaternions, NOT Euler) |
| `external` | `"1"` | yes | `1` means the child lives in another `.socpak` file |

Scale, layer, and flags attributes have **not** been observed in the sampled
socpaks. The sampled set is large enough (hangars, rest stops, admin shops,
elevator modules, weapon platforms) to be confident these six are the complete
attribute set for visual export purposes.

### Transform composition

The root manifest stores child transforms as quaternion rotation + position.
The existing code's `build_container_transform` uses Euler Ang3 (degrees, ZYX
order) from DataCore `ObjectContainerRef` — that path is for ship-level
container offsets, **not** socpak-to-socpak composition.

For socpak child composition, the formula is:

```
child_world_transform = parent_world_transform × child_local_transform
```

Where `child_local_transform` is built from the `<Child>` attributes:

```
child_local = compose(pos_xyz, rot_qwqxqyqz)
            = translation_matrix(pos) × quaternion_rotation_matrix(rot)
```

Scale is uniformly `1.0` in all sampled child placements — no scale composition
is needed.

**Important**: Root manifest `pos`/`rot` are CSV quaternions (`qw,qx,qy,qz`),
while DataCore `ObjectContainerRef.Offset.Rotation` uses Euler Ang3 (degrees).
These are two different coordinate representations for two different transform
paths. Do not mix them. The socpak pipeline must use quaternion composition
throughout.

This is implemented: `socpak.rs` `read_child_socpak_refs` parses the root
`<name>.xml` `<ChildObjectContainers>` (children with `external="1"`, any
`class`, scale fixed at 1) and `collect_interior_tree_from_socpak` recurses
into each child with `parent × local` composition.

### Cycle detection

A cycle occurs when the same socpak path appears in the ancestor stack.
`collect_interior_tree_from_socpak` (and the inspection walk
`inspect_interior_tree_from_socpak_inner`) implement the check:

1. They keep a set of ancestor socpak paths during recursive descent, removing
   each path again after its subtree is done (so repeated siblings still load).
2. Before loading a socpak, they check whether its path is already an ancestor.
3. If so, they log `skipping cyclic socpak child reference` and skip that child
   (no error, no infinite loop).
4. The comparison is by normalized socpak path (case-insensitive, slash-normalized).

There is no evidence of intentional cycles in the sampled set. If one is found,
it is likely a data error.

### Instance count guidance

The sampled data shows:
- `hangar_xltop_001`: same child placed 2× (freight elevator module)
- `rs_ext_mic-leo1`: same child placed 8× (cargo elevator) and 4× (ATC elevator)

There is no enforced maximum. The pipeline should allow unlimited instances of
the same child path with different transforms, as the game engine does. The
current code correctly models this pattern.

Example from `hangar_xltop_001_newbab`:

```xml
<Child external="1"
       name="Data/objectcontainers/pu/loc/mod/common/hangar/hightech_a/hangar_xltop_001.socpak"
       entityName="ObjectContainerModifier-hangar_xltop_001_newbab"
       class="ObjectContainerModifier"
       pos="0,0,0"
       rot="-1,0,0,0" />
```

That confirms the New Babbage variant composes over the base hangar. In this
specific sample, the extra data is mostly transit metadata and a physics-grid
pivot, so it is not important for visual export beyond the few local sign/light
objects it also carries.

Example from `hangar_xltop_001`:

```xml
<Child external="1"
       name="Data/objectcontainers/pu/loc/mod/common/hangar/hightech_a/hangar_component_module_cargo_elevator_xl.socpak"
       entityName="ObjectContainer_FreightElevator_XLTop-000"
       class="ObjectContainer"
       pos="-119.99999989534263,152.00000019470463,-112.00000085495412"
       rot="-0.70710701,-2.8914494e-07,-7.7933578e-07,0.70710653" />

<Child external="1"
       name="Data/objectcontainers/pu/loc/mod/common/hangar/hightech_a/hangar_component_module_cargo_elevator_xl.socpak"
       entityName="ObjectContainer_FreightElevator_XLTop-001"
       class="ObjectContainer"
       pos="120.00000034662662,-152.00000028847717,-112.00000018358696"
       rot="0.70710629,-3.0428146e-07,4.8011583e-07,0.70710725" />
```

This is true instancing: one child socpak path is placed twice with different
transforms.

The same pattern appears in the rest-stop exterior. In
`rs_ext_mic-leo1.xml`, one `station_ext_cargo_elevator_001.socpak` child is
placed eight times around the station, and one `reststop_elev_atc.socpak` child
is placed four times for pads/elevators. The same root also references
`rs_comm_mic-leo1.socpak`, `rs_entry_mic-leo1.socpak`, and
`reststop_cargo_occu_0001.socpak`, which are visually important because they
contain many station screens, signs, decals, and lights. This is another reason
the exporter must model child socpak instances, not just inline one copy of a
referenced path.

### `ObjectContainer` vs `ObjectContainerModifier`

Do not globally skip `ObjectContainerModifier`.

- `hangar_xltop_001_newbab` uses an `ObjectContainerModifier` at identity to
  layer a location variant on top of the base hangar. Most of the extra data is
  transit-specific and can be filtered as non-visual.
- `orison_ind_lz_int` uses four `ObjectContainerModifier` children for shops:
  `cousincrows_orison`, `providence_surplus_orison_int`,
  `covalex_orison_int`, and `admin_small_orison_a`. The extracted
  `admin_small_orison_a` contains substantial visible geometry, lights, decals,
  and cubemap DDS files. Skipping modifiers would drop real visuals.

The practical rule is:

1. Recursively load both `ObjectContainer` and `ObjectContainerModifier`.
2. Filter non-visual entities/components inside the loaded container.
3. Treat an all-nonvisual modifier as a no-op for render output.

### Root Manifest vs Entdata Coordinates

The same child references also appear in decoded `entdata/*.entxml` as
`EntityClass="ObjectContainer"` or `EntityClass="ObjectContainerModifier"` with
an `EntityComponentObjectContainer objectContainer="..."`.

However, the sampled entxml `Pos` values do not match root-manifest `pos`
values directly. They appear to be in a different coordinate frame or editor
space. For socpak-to-socpak composition, prefer the root manifest
`ChildObjectContainers` transform and use entdata as a validation/component
source until the coordinate conversion is proven.

## Mesh Sources

There are three relevant mesh paths.

### 1. `.soc` IncludedObjects

This is the main static mesh instance list.

Current StarBreaker parsing shows the chunk contains:

- A count and list of CGF paths as fixed 256-byte strings.
- A count and list of material paths.
- A count and list of tint palette paths.
- An object byte section.
- Type 1 objects with a CGF index and a 3×4 f64 transform.
- Type 7 and Type 10 objects, currently skipped and still unknown.

### IncludedObjects binary layout

The existing `IncludedObjects::from_bytes()` parser (`included_objects.rs`)
decodes the chunk in this order:

```
[4 bytes padding]
[num_cgfs: u32] [cgf_paths: num_cgfs × 256-byte null-terminated strings]
[num_materials: u16] [num_palettes: u16]
[material_paths: num_materials × 256-byte strings]
[tint_palette_paths: num_palettes × 256-byte strings]
[28 unknown bytes]
[objects_byte_len: u32] [objects section: objects_byte_len bytes]
```

Each object starts with a `u32` type marker:

| Type | Size | Known fields |
| --- | --- | --- |
| `0x0001` (Type 1) | 168 or 184 bytes | `vector1[3]` (3×f64), `vector2[3]` (3×f64), `unknown1` (u64), `cgf_index` (u16), `unknown2` (u16), `transform` (3×4 column-major f64: three rotation columns, then the translation), `unknown3` (u64). If `unknown3==0`, 16 extra trailing bytes; the u16 at object offset +172 in that block is the per-object tint-palette index (`0xFFFF` = no override). |
| `0x0007` (Type 7) | 152 bytes | Currently skipped entirely. Not decoded. |
| `0x0010` (Type 10) | 136 bytes | Currently skipped entirely. Not decoded. |

### Per-object tint palette

`IncludedObject::tint_palette_index` is decoded from the u16 at object offset
+172 (present only in the `unknown3 == 0` trailing block; the u16 at +170 is a
separate flags field whose values exceed the palette count). It indexes
`tint_palette_paths`; `0xFFFF` means the object keeps its CGF materials.
`socpak.rs` `included_objects_to_meshes` stores the resolved path as the
mesh's `tint_palette_name`, which the decomposed export emits as the
placement's `palette_id` override.

### Material-per-object mapping

IncludedObjects placements carry **no** material override:
`included_objects_to_meshes` sets `material_path: None`, so each placement
uses its CGF/CGFM file's own material library (the same path entity geometry
uses). `IncludedObjects.material_paths` is parsed but not assigned per
object.

The `IncludedObject` struct's `unknown2` field (`u16` at offset +62 from
object start) was once a candidate material index into `material_paths`;
that has **not been confirmed**, and the code does not use it.

### Type 7 and Type 10 objects (open question)

Type 7 (152 bytes) and Type 10 (136 bytes) objects are silently skipped. They
may represent:
- Visual placement categories not yet decoded (decals, lights, triggers).
- Non-visual data (physics volumes, navigation links).

The pipeline should investigate by hex-dumping Type 7/10 objects from sampled
socpaks and looking for patterns — CGF indices, transform matrices, entity
class GUIDs, or other recognizable structures. Until decoded, the pipeline
will silently drop any visual data these objects may carry.

### Existing parser

The existing `IncludedObjects::from_bytes()` parser in `included_objects.rs`
already handles Type 1 objects and returns a structured `IncludedObjects` with
`cgf_paths`, `material_paths`, `tint_palette_paths`, and `objects` (Type 1
only). The socpak pipeline should reuse this parser directly rather than
re-implementing it.

Examples of mesh paths found directly in sampled `.soc` files:

```text
objects/buildingsets/human/brand/freight/assets/freight_decal_notice_a.cgf
objects/buildingsets/human/brand/decal/signs/primary_sign_elevator_a.cgf
objects/buildingsets/human/hightech/loc/stanton/orison/industrial_plate/int/ind_sign_transit_guide_a.cgf
objects/buildingsets/human/station/utilitarian/core_c/core_c_ring_lrg_a_45_name_display.cgf
```

These paths are not ZIP entries. They resolve against the outer P4K, usually
under `Data\Objects\...`, with case-insensitive lookup and optional `Data\`
prefix normalization.

### 2. Entity geometry

The `.soc` `CryXMLB` chunk and decoded `entdata/*.entxml` can contain renderable
entities with either:

- Inline geometry under
  `PropertiesDataCore -> EntityGeometryResource -> Geometry -> Geometry -> Geometry @path`.
- An `EntityClassGUID`, which must be resolved through DataCore
  `EntityClassDefinition` geometry/material records.

Current code already resolves some GUID-backed geometry and can expand loadout
attachments for interior entities. That path matters for small props where the
visible geometry lives on a child loadout item.

#### Geometry resolution paths

There are two known paths for resolving entity geometry:

1. **Inline CryXMLB geometry** — `PropertiesDataCore → EntityGeometryResource → Geometry → Geometry → Geometry @path`. The path attribute directly gives a CGF/CGA path. The existing `extract_entity_geometry()` in `socpak.rs` follows this chain.

2. **DataCore GUID resolution** — When an entity has an `EntityClassGUID` but no inline geometry, the GUID is looked up in DataCore `EntityClassDefinition`. The geometry is then found via `SGeometryResourceParams` components, which is the same path `resolve_guid_geometry` uses for ship/entity exports. This path provides both geometry and material defaults.

### 3. Local `brush/*.cgf` and `.cgfm`

The inner archive can contain local brush geometry. Sample counts:

- `hangar_xltop_001`: 45 `.cgf` and 45 `.cgfm` in `brush/`.
- `orison_ind_lz_int`: 7 `.cgf` and 7 `.cgfm`.
- `rs_ext_mic-leo1`: 24 `.cgf` and 24 `.cgfm`.

Many sampled brush files reference proxy or collision materials such as:

```text
materials/bhvr/special/proxy_nodraw
materials/special/collision_proxy_entitiesonly
engineassets/texturemsg/defaultsolids
```

Do not hard-code a brush-folder skip. Load local brush files when referenced or
when the editor/object data indicates they are visible, then let material
semantics hide `NoDraw`, proxy, and collision-only geometry.

## Vertex Colors

Vertex colors are mesh-stream data in CGF/CGFM/NMC content, not a socpak XML
field. StarBreaker already carries them as `Mesh.colors` and exports them to
glTF `COLOR_0` / Blender corner colors in other paths.

For socpak export, the important requirement is to preserve the mesh loader's
vertex color stream through the same decomposed exporter path used for entity
exports. Materials with `VERTCOLORS`/`VCOL` flags need those colors to reach the
Blender material graph.

## Materials and Textures

SOCPAKs mostly reference external materials and textures, but they can also
carry local texture payloads.

Material sources to resolve:

- Material paths in `.soc` IncludedObjects.
- Material paths embedded in CGF/CGFM material libraries.
- Entity `Material` attributes.
- DataCore material paths when geometry is resolved through `EntityClassGUID`.

Texture sources to resolve:

- External `.mtl` texture slots under `Data\Materials\...` or alongside
  `Data\Objects\...`.
- Local inner-archive DDS files, observed in
  `admin_small_orison_a/admin_small_orison_a/cubemaps/asoa_light_int/`.
- Virtual material inputs such as `$RenderToTexture` and `$TintPaletteDecal`.

Example embedded cubemaps from `admin_small_orison_a`:

```text
admin_small_orison_a/cubemaps/asoa_light_int/cm_shop_admin_orison_small_int_probe_001_cm.dds
admin_small_orison_a/cubemaps/asoa_light_int/cm_shop_admin_orison_small_int_probe_001_cm_diff.dds
admin_small_orison_a/cubemaps/asoa_light_int/cm_shop_admin_orison_small_int_probe_002_cm.dds
admin_small_orison_a/cubemaps/asoa_light_int/cm_shop_admin_orison_small_int_probe_002_cm_diff.dds
```

Texture lookup should therefore be:

1. Resolve material and texture path normally.
2. If a texture path is relative or container-local, check the inner socpak.
3. Otherwise resolve against the outer P4K.

### Inner-archive texture resolution mechanics

When a material references a texture whose path matches an entry inside the
socpak's own ZIP, the inner-archive copy takes priority over the outer P4K.

The resolution algorithm:

1. After resolving the `.mtl` file (from P4K), parse its texture slot paths.
2. For each texture path, check if it matches a path inside the socpak's inner
   ZIP archive. The match is by **exact relative path** — the material's texture
   slot contains a full path like `admin_small_orison_a/cubemaps/asoa_light_int/cm_shop_admin_orison_small_int_probe_001_cm.dds`,
   which can be directly looked up as an entry in the inner ZIP.
3. If the inner ZIP has a matching entry, read the DDS from there.
4. If not found in the inner ZIP, resolve the path against the outer P4K.

The inner-archive textures observed so far are all cubemap DDS files under a
container-named subdirectory (`admin_small_orison_a/cubemaps/...`). The `.mtl`
files that reference them use the same path prefix. This means the matching is
straightforward: if the texture path starts with the container's directory name
as it appears in the ZIP, try the inner archive first.

## Tint Palettes

Tint palette paths are explicit in the `.soc` IncludedObjects chunk.

Examples seen in sampled `.soc` files:

```text
Libs/Foundry/Records/TintPalettes/default
Libs/Foundry/Records/TintPalettes/Generic/gen_lightgrey_grey_darkblue
Libs/Foundry/Records/TintPalettes/Generic/gen_lightgrey_grey_darkgrey
```

Palettes are resolved per object, which gives each container (and each child
container) its own palette scope without a parent overwriting a child:

- Each palette path resolves to a DataCore tint-palette record
  (`pipeline/interiors.rs` `resolve_interior_palette`, cached per path).
- Preserve tint-palette decal inputs such as `$TintPaletteDecal` in material
  sidecars; those are decal/livery inputs, not just base paint inputs.

### Palette-to-mesh binding

In the `IncludedObjects` chunk, tint palette paths are a flat list, and each
Type 1 object selects one through its per-object `tint_palette_index` (see
[Per-object tint palette](#per-object-tint-palette); `0xFFFF` = none).
`pipeline/interiors.rs` resolves that path into the placement's `palette`,
emitted as the placement `palette_id` in `scene.json`.

Interior containers carry **no** container-wide palette (`palette: None`): the
earlier `tint_palette_names.first()` fallback applied the generic `default`
palette (a placeholder with a weapon decal and near-black tints) to whole
interiors and was removed.

## Decals

Decals appear in at least two forms.

### Mesh decals

Many decal-like visuals are ordinary mesh instances whose CGF path or material
path is decal themed:

```text
objects/buildingsets/human/brand/freight/assets/freight_decal_notice_a.cgf
objects/buildingsets/human/brand/decal/signs/primary_sign_elevator_a.cgf
materials/pu/decal/stains/surface/decal_surface_var_hg_b
materials/decals/weathered/weathered001d
materials/decals/scratches/scratches_small_a
```

MCP material summaries confirmed typical `Decal` shader slot shapes:

```text
Data\Materials\PU\decal\stains\surface\decal_surface_var_hg_b.mtl
  Shader=Decal
  TexSlot1=decal_surface_var_b_diff.tif
  TexSlot2=decal_surface_var_b_ddna.tif

Data\Materials\decals\scratches\scratches_small_a.mtl
  Shader=Decal opacity=0.8
  TexSlot1=scratchesSmall_decal_01.tif
  TexSlot2=gloss90_ddna.tif
  TexSlot4=metal_clear_spec.tif
```

These should go through the normal mesh/material export path and land in the
decomposed material sidecar with decal semantics.

### Decal entities/projectors

Editor XML also lists many `Object type="Decal"` records:

| SOCPAK | Editor decal count |
| --- | ---: |
| `hangar_component_module_cargo_elevator_xl` | 16 |
| `hangar_xltop_001` | 544 |
| `orison_ind_lz_int` | 1019 |
| `rs_ext_mic-leo1` | 468 |
| `admin_small_orison_a` | 5 |

In sampled editor XML, a decal object often only has `name`, `type`, and
`guid`. That is not enough to reconstruct a projector. The actual transform,
projection size, material, and flags are likely in the `.soc` CryXMLB entity
payload or another binary chunk.

### Decal entity CryXMLB structure (under investigation)

Decal entities in `.soc` CryXMLB data have **not been fully characterized yet**.
The editor XML shows entity class `"Decal"`, but the CryXMLB payload inside
`.soc` files needs to be inspected to determine:

- The exact entity class name(s) used in CryXMLB (candidate: `Decal`).
- The full attribute list on the `<Entity>` element.
- The `PropertiesDataCore` child structure, which likely contains projection
  parameters (size, aspect ratio, depth), material path, and opacity.
- Whether decal entities use the same `PropertiesDataCore → EntityGeometryResource`
  path as other geometry entities, or a dedicated component.

To investigate: extract a `.soc` CryXMLB chunk from a socpak with many decals
(e.g., `orison_ind_lz_int` with 1019 editor decals) and search for entities
with `EntityClass="Decal"` or similar. Log all attributes and children for
those entities. The pipeline should add `Decal` to the entity processing path
once the structure is known.

Exporter guidance:

- Do not claim decal support from editor XML counts alone.
- Parse `.soc` CryXMLB for `Decal` entities and inspect all attributes and
  component children.
- If a decal has projector/volume data, emit it as a decal projector or
  generated mesh plane in the decomposed scene.
- If it is mesh-backed, route it through the mesh/material path instead.

## POM

POM is a material feature, not a standalone socpak file type.

It should be detected from material data:

- `StringGenMask` tokens such as POM/parallax flags.
- Shader family and material name variants.
- Height/displacement slots.
- Existing decomposed sidecar roles.

MCP examples outside the sampled socpaks show the expected slot shape:

```text
Data\Materials\vehicles\manufacturer\CRUS\crus_pom.mtl
  Shader=Layer
  TexSlot1=crus_pom_diff.tif
  TexSlot2=crus_pom_ddna.tif
  TexSlot3=crus_pom_displ.tif
  TexSlot6=crus_pom_spec.tif
```

Existing docs also identify MeshDecal POM shapes such as
`TexSlot1/2/4/8` and `$TintPaletteDecal` combinations. Socpak export should
not implement a separate POM parser; it should make sure all socpak material
paths enter the same decomposed material analysis that ship/entity exports use.

## Text, Signs, and Screens

There are four visually different categories.

### Static sign meshes

Many "text" surfaces are just mesh paths with sign/display names. These should
export as normal geometry plus materials/textures. Examples from sampled `.soc`
files:

```text
objects/buildingsets/human/brand/decal/signs/primary_sign_elevator_a.cgf
objects/buildingsets/human/hightech/loc/stanton/orison/industrial_plate/int/ind_sign_transit_guide_a.cgf
objects/buildingsets/human/station/utilitarian/core_c/core_c_ring_lrg_a_45_name_display.cgf
```

The material path is often enough to explain the sign type. For example, the
`core_c_ring_lrg_a_45_name_display.cgf` chunk material is
`objects/buildingsets/human/station/utilitarian/station_util_a`, whose material
summary contains many atlas/decal/POM/vertex-color submaterials. This looks
like a mesh/material atlas sign, not a literal string embedded in the socpak.

Another sign mesh,
`objects/buildingsets/human/brand/decal/signs/secondary_sign_03_a.cgf`, points
at `objects/buildingsets/human/brand/lorville/lorville_brand_a`. That material
contains alphabet, number, iconography, screen-border, and stencil submaterials,
including `$TintPaletteDecal`. For exporter purposes this is still mesh +
material + texture atlas work, not localization work.

### Pre-rendered UI/sign textures

Some readable signage exists as texture assets outside the socpak. For Rest &
Relax, P4K contains:

```text
Data\UI\Textures\Props\Rest_Stop\RestandRelax\restandrelax_welcome.dds
Data\UI\Textures\Props\Rest_Stop\RestandRelax\restandrelax_elevator_a.dds
```

MCP image preview confirmed the first texture visibly contains the Rest & Relax
logo, "WELCOME", and the small caption "Everything your trip needs. Full stop."
The second texture contains the Rest & Relax logo plus "INNER STATION TRANSIT".

These are exportable as normal texture dependencies once a material/UI record
points to them. They should not be searched for inside the socpak ZIP itself
unless the texture is embedded under the container directory.

### Screen entities

Editor XML exposes placed screen-like entities, but usually only as
name/type/GUID/tags. It is useful for discovery, not enough for final render
data.

Counts from sampled editor XML:

- `rs_ext_mic-leo1`: 4 `SCItemDisplayScreen_Station_Info_A`.
- `rs_comm_mic-leo1`: 8 `GeomEntity_RttUIWithAudio`, 1
  `RenderToTextureView`, 1 `SCItemDisplayScreen`, 9
  `SCItemDisplayScreen_Station_Info_A`, and 1
  `SCItemDisplayScreen_Station_Info_B`.
- `rs_entry_mic-leo1`: 3 `GeomEntity_RttUIWithAudio`, 1
  `RenderToTextureView`, 7 `SCItemDisplayScreen_Station_Info_A`, and 1
  `SCItemDisplayScreen_Station_Info_B`.
- `orison_ind_lz_int`: 31 `SCItemDisplayScreen`.

This is important for `rs_ext_mic-leo1`: the exterior root does contain a few
station-info screens, but many more RTT/display objects live in the referenced
`rs_comm_mic-leo1` and `rs_entry_mic-leo1` child socpaks. A recursive socpak
walk is required before deciding text/screens are missing.

DataCore fills in the entity class defaults:

- `SCItemDisplayScreen_Station_Info_A` and `_B` use geometry
  `objects/buildingsets/human/universal/prop/screen/screen_universal_1_16x9_a.cgf`
  and material `Materials/UI/rtt_comms_opaque_hightech.mtl`.
- Both include `UIRenderToTextureEntityComponentParams` with
  `runtimeImageSource=UserInterface`.
- Both use `UIOwnerEntityComponentParams` pointing at the Building Blocks UI
  element `libs/foundry/records/ui/uielements/buildingblocks.json`.
- `Station_Info_A` uses the normal/off canvas
  `libs/foundry/records/ui/buildingblocks/digitalsignagecanvas.json` and
  carries a `DigitalSignageComponentParams.defaultCanvas` of
  `libs/foundry/records/ui/buildingblocks/f_fluffscreens/f_sgn/sgn_welcome_restandrelax_a.json`.
- `Station_Info_B` uses
  `libs/foundry/records/ui/buildingblocks/f_fluffscreens/f_state/f_state_reststop_elevator_a.json`
  for the normal view and `sgn_welcome_restandrelax_a.json` for the off view.
- `GeomEntity_RttUIWithAudio` uses geometry
  `objects/buildingsets/human/universal/prop/screen/screen_9x16_1_00154x00274_a.cgf`,
  material `materials/test/rendering/rtt_test_material.mtl`, UI element
  `libs/foundry/records/ui/uielements/lowtechpanelscreen.json`, and
  `runtimeImageSource=UserInterface`.

The Building Blocks canvas paths above are DataCore record paths. P4K path
search did not find them as standalone archive files, so the exporter should
obtain them through DataCore, not by trying to read those JSON paths from the
P4K file table.

### Canvas decals and text providers

The general text path is not "find literal words in the socpak". It is:

1. Find a placed renderable entity in recursive `.soc` data.
2. Resolve its DataCore entity class defaults.
3. Merge placed `.soc` overrides from `UserVariablesComponentParams`.
4. Resolve localization keys through `Data\Localization\<language>\global.ini`.
5. Preserve unresolved UI/runtime sources instead of guessing.

The `global.ini` localization file lives in P4K at:
`Data\Localization\english\global.ini` (for English). Other languages follow the
same pattern under `Data\Localization\<language>\`. The file is an INI-style
text file with lines like `Stanton4_Transfer=Port Tressler`. Look up the key
after stripping the `@` prefix from the `locKey` attribute.

### Building Blocks canvas resolution

The full path from a placed entity to resolved text involves:

1. **Entity component** — `UIBuildingBlocksEntityComponentParams` on the placed
   entity points to a UI element record (e.g.,
   `libs/foundry/records/ui/uielements/buildingblocks.json`).

2. **Canvas record** — The entity class's DataCore definition provides the
   canvas record path (e.g.,
   `libs/foundry/records/ui/buildingblocks/digitalsignagecanvas.json` or
   `libs/foundry/records/ui/buildingblocks/i_interactivescreens/sgn/i_sgn_infopanel_a.json`).
   These are DataCore records, not P4K file paths — resolve them through
   DataCore queries.

3. **Variable bindings** — The canvas record contains
   `BuildingBlocks_BindingsLocalizedVariable` and
   `BuildingBlocks_BindingsLocalizedField` entries that map widget names to
   entity variable names (e.g., `InfoPanel_Title1` → text widget "Title1").

4. **Entity variables** — The placed entity's `UserVariablesComponentParams`
   provides the actual values, either as `@locKey` references (resolved via
   `global.ini`) or literal strings.

The pipeline does not need to render the canvas layout — it only needs to
extract the text content and style variables for each text surface.

`CanvasDecal`, `CanvasDecal_Standalone`, and related provider entities are the
clearest static-text candidates. DataCore shows the reusable class shape:

- `CanvasDecal_Standalone` uses `UICanvasDecalDescriptorEntityComponentParams`
  with canvas `libs/foundry/records/ui/buildingblocks/dev/tests/cdt_singletext.json`.
- It has `UserVariablesComponentParams` for `fontsize`, `letterSpacing`,
  `Color_R/G/B/A`, alignment booleans, a `fontstyle` record reference, and
  `locStrings` with variable `title`.
- `CanvasDecal_Standalone_NoLoc` uses
  `LiteralStringProviderComponentParams` with literal string variable
  `LiteralString`. It still has normal font/color/alignment variables.
- `CanvasDecalProvider` has provider-style `locStrings` such as `Title` and
  `SubTitle`, plus generic number/integer/bool inputs. It may feed nearby
  consumer decals through UI binding components rather than carrying the final
  mesh itself.

### Component schemas for text entities

These components appear in `.soc` CryXMLB under `PropertiesDataCore` children
of text/canvas/screen entities. The schemas below reflect the DataCore class
definitions and the attributes observed in placed entity overrides.

#### `UserVariablesComponentParams`

Carries local variable overrides for the placed entity. In CryXMLB, this
appears as a child of `PropertiesDataCore`:

```xml
<PropertiesDataCore>
  <UserVariablesComponentParams>
    <locStrings>
      <SBindableLocalizedStringVariable variableName="title">
        <value @locKey="@Stanton4_Transfer" />
        <!-- OR literal text: -->
        <!-- <value @locKey="" @defaultString="Port Tressler" /> -->
      </SBindableLocalizedStringVariable>
    </locStrings>
    <SBindableFloatVariable variableName="fontsize" value="48.0" />
    <SBindableFloatVariable variableName="letterSpacing" value="0.0" />
    <SBindableFloatVariable variableName="Color_R" value="1.0" />
    <SBindableFloatVariable variableName="Color_G" value="1.0" />
    <SBindableFloatVariable variableName="Color_B" value="1.0" />
    <SBindableFloatVariable variableName="Color_A" value="1.0" />
    <SBindableBoolVariable variableName="AlignLeft" value="0" />
    <SBindableBoolVariable variableName="AlignCenter" value="1" />
    <SBindableBoolVariable variableName="AlignRight" value="0" />
    <RecordRefUserVariableTypeFontStyle variableName="fontstyle"
      path="Libs/Foundry/Records/UI/FontStyles/..." />
  </UserVariablesComponentParams>
</PropertiesDataCore>
```

#### `UICanvasDecalDescriptorEntityComponentParams`

References the Building Blocks canvas definition:

```xml
<UICanvasDecalDescriptorEntityComponentParams
  canvas="libs/foundry/records/ui/buildingblocks/dev/tests/cdt_singletext.json" />
```

#### `LiteralStringProviderComponentParams`

Used by `CanvasDecal_Standalone_NoLoc` for non-localized literal text:

```xml
<LiteralStringProviderComponentParams>
  <SBindableLiteralStringVariable variableName="LiteralString" value="Station 12" />
</LiteralStringProviderComponentParams>
```

Placed `.soc` data then supplies the actual transform, mesh, material, and
variable overrides. In the LEO rest-stop samples, the station-name signs are
ordinary canvas decal examples:

```text
rs_entry_mic-leo1.soc
  CanvasDecal_Standalone
  objects/buildingsets/human/station/utilitarian/brand_decals/station_canvas_decal_name_int_a.cgf
  Materials/PU/decal/canvas_dcl/cnvs_sign_glow_black_a
  UICanvasDecalDescriptorEntityComponentParams
  UserVariablesComponentParams
  RecordRefUserVariableTypeFontStyle
  locStrings
  @Stanton4_Transfer -> Port Tressler

rs_entry_arc-leo1.soc
  same class/component pattern
  @Stanton3_Transfer -> Baijini Point
```

Those two examples are direct SOCPAK text bindings. They are not proof that
visible station text comes from `MissionLocationTemplate`. Location templates
can still be useful context, but the visual exporter should prefer the placed
canvas/text entity and its own user variables.

The Orison industrial sample shows the same idea outside station names:

```text
orison_ind_lz_int.soc
  InfoScreen
  objects/buildingsets/human/hightech/prop/sign/sign_map_1_b.cgf
  UserVariablesComponentParams
  locStrings
  @Orison_DiscoverySpot_009_title
  @Orison_DiscoverySpot_009_desc

Data\Localization\english\global.ini
  Orison_DiscoverySpot_009_title=Who are the People Working for Crusader?
  Orison_DiscoverySpot_009_desc=It takes a special kind of person ...
```

The `InfoScreen` DataCore class uses Building Blocks canvas
`libs/foundry/records/ui/buildingblocks/i_interactivescreens/sgn/i_sgn_infopanel_a.json`
and declares localized variables `InfoPanel_Title1`,
`InfoPanel_Text1`, through `InfoPanel_Title4` and `InfoPanel_Text4`.
The `BuildingBlocks_Canvas.I_Sgn_InfoPanel_A` record binds those localized
variables to text widgets via `BuildingBlocks_BindingsLocalizedVariable` and
`BuildingBlocks_BindingsLocalizedField`. This is the generic RTT/text path:
the entity provides variables; the canvas defines where those variables render.

Do not treat every loc key in a `.soc` as visible Blender text. Room names,
transit destinations, interaction prompts, shop labels, and navigation labels
also use localization keys. Export them as visual text only when they are tied
to a renderable text/canvas/screen entity or to a material/mesh that presents
that text.

### RTT and dynamic screen sources

RTT screens and canvas text decals are related, but not identical. Canvas
decals are usually renderable text planes with explicit user variables. RTT
screens are material surfaces whose content may be a Building Blocks canvas,
video, Flash/GFx, static UI texture, provider-fed runtime UI, or an unresolved
runtime source.

MCP material summaries confirm virtual render-to-texture inputs:

```text
Data\Materials\UI\ui_general_solid_engineering.mtl
  Shader=UIPlane
  TexSlot9=$RenderToTexture

Data\Materials\UI\rtt_comms_opaque_hightech.mtl
  Shader=UIPlane
  TexSlot9=$RenderToTexture
  TexSlot17=pixel_layout_crt.tif

Data\Objects\Spaceships\Ships\AEGS\Idris_Frigate\interior\ui\ui_screen_16x9-Large-Generic01.mtl
  Submaterial Flash, Shader=DisplayScreen, TexSlot9=$RenderToTexture

Data\Materials\test\rendering\rtt_test_material.mtl
  Shader=UIPlane
  TexSlot9=$RenderToTexture
```

For Blender export, preserve `$RenderToTexture` as a virtual input in the
material sidecar and mark the surface as an RTT candidate. Then classify the UI
source separately:

- **Building Blocks localized canvas**: parse
  `UIBuildingBlocksEntityComponentParams`, canvas records, and
  `UserVariablesComponentParams.locStrings`. Example: `InfoScreen` on Orison
  maps `InfoPanel_*` bindings to `@Orison_DiscoverySpot_009_*`.
- **DigitalSignage default canvas**: parse `DigitalSignageComponentParams` and
  `defaultCanvas`. Example: Rest & Relax station info screens point at
  `sgn_welcome_restandrelax_a.json` and
  `f_state_reststop_elevator_a.json`.
- **Static UI texture**: export the texture as the screen preview when the UI
  source resolves to DDS art such as the Rest & Relax textures above.
- **Video or Flash/GFx**: preserve the media path as a runtime/preview source.
  The sampled rest-stop comm child places a display with
  `UI/Video/Orison/Orison_Tourism_video.bk2`; DataCore news-screen records can
  carry a `filename` user variable such as a `.gfx` path.
- **Runtime provider**: preserve provider/consumer links and emit an unresolved
  placeholder if the content depends on game runtime state.

For Blender, export each text or screen surface as a structured record, not as
a guessed string:

```text
text_surface:
  source_container: Data\ObjectContainers\...\example.socpak
  entity_class: CanvasDecal_Standalone | InfoScreen | SCItemDisplayScreen_*
  entity_guid: <CryGUID>
  category: canvas_localized | canvas_literal | rtt_buildingblocks |
            rtt_static_texture | rtt_video | rtt_runtime
  transform: <composed world/local transform>
  mesh: <CGF/CGA path>
  material: <MTL path>
  canvas: <BuildingBlocks/DataCore canvas record if any>
  variables:
    - name: title
      source: loc_key
      key: @Stanton4_Transfer
      value_en: Port Tressler
    - name: LiteralString
      source: literal
      value: <text>
  style:
    fontstyle: <RecordRefUserVariableTypeFontStyle>
    fontsize: <float/int variable>
    letter_spacing: <float variable>
    color_rgba: <Color_R/G/B/A variables>
    alignment: <AlignLeft/AlignCenter/AlignRight variables>
  unresolved_source: <provider/video/gfx/runtime path when no static text exists>
```

The Blender importer can use that record to create real Blender text for simple
canvas decals, bake a preview texture for resolvable Building Blocks screens,
or show an annotated placeholder plane for runtime-only RTT surfaces.

## Lights

Lights are visible export data.

Editor XML counts can be large:

- `hangar_xltop_001`: 1633 `Light`, 83 `LightBox`, 82 `EnvironmentLight`.
- `orison_ind_lz_int`: 2259 `Light`, 83 `LightBox`, 82 `EnvironmentLight`.
- `rs_ext_mic-leo1`: 1414 `Light`.

Current code parses some `.soc` CryXMLB entities:

- `Light`
- `LightBox`
- `LightGroup`
- `LightGroupPoweredItem`

That is the right source to continue from, but the exporter should avoid
treating editor counts as render-ready data. The `.soc` CryXMLB/entity payload
should provide the actual transform, color, radius, projector texture, and
state information.

### Light entity CryXMLB structure

A single `Light` entity in `.soc` CryXMLB has this structure:

```xml
<Entity EntityClass="Light" Name="HangarLight-001"
        Pos="10.5,2.0,-3.0" Rotate="0.707,0,0,0.707" Scale="1,1,1">
  <PropertiesDataCore>
    <EntityComponentLight lightType="Omni" useTemperature="0">
      <offState intensity="0.0" temperature="6500.0">
        <color r="1.0" g="0.95" b="0.9" />
      </offState>
      <defaultState intensity="1.0" temperature="6500.0">
        <color r="1.0" g="0.95" b="0.9" />
      </defaultState>
      <auxiliaryState intensity="0.5" temperature="6500.0">
        <color r="1.0" g="0.9" b="0.8" />
      </auxiliaryState>
      <emergencyState intensity="2.0" temperature="3500.0">
        <color r="1.0" g="0.4" b="0.2" />
      </emergencyState>
      <sizeParams lightRadius="15.0" />
      <projectorParams texture="" FOV="0.0" />
    </EntityComponentLight>
  </PropertiesDataCore>
</Entity>
```

A `LightGroup` entity wraps multiple baked-in lights:

```xml
<Entity EntityClass="LightGroup" Name="LightGroup-Ceiling-001"
        Pos="0,5,0" Rotate="1,0,0,0" Scale="1,1,1">
  <PropertiesDataCore>
    <EntityComponentLightGroup>
      <BakedInLights>
        <Light>
          <RelativeXForm translation="1,0,0" rotation="0.707,0,0,0.707" />
          <EntityComponentLight lightType="Omni" useTemperature="1">
            <defaultState intensity="0.8" temperature="4000.0">
              <color r="1.0" g="0.95" b="0.85" />
            </defaultState>
            <sizeParams lightRadius="8.0" />
          </EntityComponentLight>
        </Light>
        <Light>
          <RelativeXForm translation="-1,0,0" rotation="0.707,0,0,0.707" />
          <!-- ... another EntityComponentLight ... -->
        </Light>
      </BakedInLights>
    </EntityComponentLightGroup>
  </PropertiesDataCore>
</Entity>
```

The `EntityComponentLight` attributes and children:

| Element/Attribute | Type | Description |
| --- | --- | --- |
| `lightType` | string | `Omni`, `SoftOmni`, `Projector`, `Ambient`, `Directional` |
| `useTemperature` | bool | Whether to use Kelvin temperature instead of RGB color |
| `<defaultState>` | child | Primary light state with `intensity`, `temperature`, `<color>` |
| `<offState>` | child | Off state (intensity usually 0) |
| `<auxiliaryState>` | child | Dim/alternate state |
| `<emergencyState>` | child | Emergency/alarm state |
| `<cinematicState>` | child | Cinematic lighting state |
| `<sizeParams lightRadius>` | child | Attenuation radius in meters |
| `<projectorParams texture>` | child | Projector cookie texture path |
| `<projectorParams FOV>` | child | Projector field of view (degrees) |

## Non-Visual Data to Filter

These records are useful for understanding why a container exists, but should
not drive Blender geometry unless they also reference a visual entity:

- Transit metadata and transit gateways.
- Landing-area metadata.
- Shop metadata.
- Insurance providers.
- Go-to points and AI/navigation data.
- Audio triggers and ambience.
- Monitored zones / `.ale` area volumes.
- Room mapping (`.rmxml`) and room/brush maps (`.brmp`), unless later proven to
  carry culling or grouping needed for export.
- Spawn closet and mission-link components.

The New Babbage hangar variant is a good example: it references the base hangar
and adds transit/destination metadata. Most of that metadata can be ignored for
visual export, but the base hangar reference must still be followed.

## Recommended Decomposed Export Pipeline

### Overview

The pipeline transforms a socpak (ZIP inside P4K) into a decomposed scene
graph that fits into the existing `scene.json` / material sidecar / texture
export contract documented in `decomposed-export-contract.md`.

### Step dependencies and parallelism

The 17 steps fall into dependency groups:

```
Steps 1-4  (load + parse):     strictly sequential
Steps 5-6  (build nodes):      depends on step 4; can run in parallel
Steps 7-8  (secondary parse):  depends on step 3; can run in parallel with 5-6
Step  9    (child recursion):   depends on step 5 (child refs from manifest)
Step  10   (DataCore resolve):  depends on step 6 (entity list from .soc)
Step  11   (localization):      depends on step 10 (variable bindings)
Steps 12-13 (mesh resolution):  depends on steps 6, 8 (geometry paths)
Step  14   (materials):         depends on steps 6, 12 (mesh material refs)
Step  15   (material pipeline): depends on step 14
Step  16   (emit scene graph):  depends on all prior steps
Step  17   (filter non-visual): runs during step 16, not after
```

### Data flow between steps

Each step produces typed data that flows into downstream steps. The existing
codebase already provides these types:

| Step | Produces | Existing type |
| --- | --- | --- |
| 4 | Parsed root manifest | Custom XML parse (no struct yet — needs one) |
| 5 | Container node | `InteriorContainerData` (name, transform, placements, lights, palette) |
| 6 | Static meshes + entities | `IncludedObjects` → `InteriorMesh` list; CryXMLB → entity list |
| 6a | Tint palette names | `Vec<String>` from `IncludedObjects.tint_palette_paths` |
| 9 | Recursive child payloads | `InteriorPayload` per child (name, meshes, lights, transform, palettes) |
| 10 | Resolved entity geometry | `InteriorMesh` with filled `cgf_path` or `entity_class_guid` |
| 12-13 | Parsed mesh geometry | `Mesh` (positions, normals, UVs, colors, submeshes) |
| 14-15 | Material sidecars | Material JSON files following decomposed-export-contract |

### Integration with existing decomposed types

The socpak pipeline maps into the existing decomposed export system as follows:

- **Per socpak** → one `InteriorContainerData` with its meshes, lights, and palette.
- **Per child socpak instance** → another `InteriorContainerData` composed with the parent's transform.
- **Scene graph emission** — each container becomes a set of entries in `scene.json`'s interior containers section, using the same `InteriorPayload` / `InteriorContainerData` types that ship already use.
- **Material sidecars** — socpak materials go through the same decomposed material pipeline as ship materials.
- **Texture export** — socpak-local textures (inner-archive DDS) are resolved and exported alongside P4K textures using the same path rules.

The key mapping: **one socpak = one `InteriorPayload`** (with recursive children producing nested `InteriorPayload` instances). This reuses the existing types without introducing new ones.

### Steps

1. Accept either a local `.socpak` or an exact P4K path.
2. Normalize the path for P4K lookup:
   - case-insensitive
   - slash/backslash tolerant
   - add `Data\` if missing
3. Read the outer P4K entry and parse the socpak bytes as an inner ZIP/P4K
   archive.
4. Decode root `<name>.xml`.
5. Build a container node with:
   - source path
   - local bounds/radius
   - root manifest metadata needed for diagnostics
   - child container references from `ChildObjectContainers`
6. Parse `<name>.soc` as a CrCh chunk file:
   - `IncludedObjects` -> static mesh placements, material paths, tint palettes
   - `CryXMLB` -> entities, lights, possible decals/screens/text candidates
7. Parse `.ale` files if present as CrCh/CryXMLB for classification, then
   skip monitored-zone/area-only data from the normal Blender visual export.
8. Decode `entdata/*.entxml`:
   - cross-check child refs
   - resolve entity GUIDs
   - find geometry/material/light/screen data missing from `.soc` parsing
9. Resolve child socpaks recursively from root-manifest child refs:
   - compose parent transform with child `pos`/`rot`/scale (quaternion-based)
   - allow repeated instances of the same child path
   - detect cycles by normalized path in ancestor stack
10. Resolve DataCore-backed entity class definitions for placed entities:
    - geometry/material from `SGeometryResourceParams`
    - lights and interaction state only where it changes visuals
    - canvas/text decal components such as
      `EntityComponentUICanvasDecalDescriptor`,
      `UICanvasDecalDescriptorEntityComponentParams`,
      `UserVariablesComponentParams`, `LiteralStringProviderComponentParams`,
      `locStrings`, literal strings, font/style references, color, size, and
      alignment variables
    - `SCItemDisplayScreen*`, `GeomEntity_RttUIWithAudio`,
      `UIRenderToTextureEntityComponentParams`, `UIOwnerEntityComponentParams`,
      `UIBuildingBlocksEntityComponentParams`, and
      `DigitalSignageComponentParams`
11. Resolve localization and UI preview sources separately from socpak parsing:
    - `.soc` text/canvas entities that provide direct loc keys or literal
      string variables
    - `Data\Localization\<language>\global.ini` for `@key` strings
    - DataCore `BuildingBlocks_Canvas` records for localized variable bindings
      and text-widget layout
    - `DigitalSignageComponentParams.defaultCanvas`, user-variable media paths,
      and provider/consumer links for RTT screens
    - UI texture assets under `Data\UI\...` when a display/canvas resolves to
      static art
12. Resolve every mesh path:
    - if it is an inner-archive local file, read it from the socpak
    - otherwise read from the outer P4K
13. Parse CGF/CGFM normally:
    - preserve positions, normals, tangents, UVs, UV2, vertex colors, submesh
      material IDs, and NMC node hierarchy
14. Resolve materials:
    - IncludedObjects material list
    - CGF/CGFM material refs
    - entity `Material`
    - DataCore material path for GUID-backed entities
15. Route materials through the existing decomposed material sidecar pipeline:
    - decals
    - POM
    - RTT/screens/text candidates
    - tint palette and tint palette decal inputs
    - texture export
16. Emit the decomposed scene graph:
    - container nodes
    - child container instances
    - mesh instances
    - light instances
    - decal projectors or mesh decals
    - text surface records for localized/literal canvas decals and resolvable
      Building Blocks text
    - screen/RTT placeholders and resolved preview textures when available
    - material sidecars and texture assets
17. Skip non-visual game metadata by component/entity class after visual data
    has been extracted.

## Current StarBreaker Gaps

The current socpak implementation covers the core composition path; the
remaining gaps are mostly decals, text, and undecoded data.

### Implemented

- Root XML `ChildObjectContainers` are followed recursively
  (`read_child_socpak_refs` + `collect_interior_tree_from_socpak`), with the
  ancestor-set cycle check described above.
- Repeated child instances of the same socpak path each load with their own
  composed transform.
- Socpak container data maps into the `scene.json` `interiors[]` schema of
  `decomposed-export-contract.md` (socpak → `InteriorPayload` →
  `InteriorContainerData` → interior container, placements, and lights).
- IncludedObjects placements use the CGF/CGFM's own material library
  (`material_path: None`); there is no first-material assignment.
- Tint palettes are per object (`IncludedObject::tint_palette_index` →
  placement `palette_id`); interior containers carry no container-wide palette.
- Pak-mounted geometry baked inside a socpak is read from the inner archive
  (`read_socpak_internal_geometry`).
- Entity UI canvases authored inline on `.soc` entities are captured
  (`extract_entity_ui_canvas`) and, with class-default canvases, become
  placement `ui_bindings` rendered by the UI pipeline; transit-console floor
  names come from the socpak's `metadata/sctransitdestination.xml`.

### Open

- It does not reconstruct Decal projectors from `.soc` entity data — the decal
  entity CryXMLB structure has not been characterized yet.
- It skips IncludedObjects Type 7 and Type 10 objects, which may represent
  visual or placement categories not yet decoded. The Type 1 `unknown2` field
  is still undecoded.
- It does not parse text decal entities, `UserVariablesComponentParams`,
  `locStrings`, literal strings, color/size/alignment variables, or font/style
  references from `.soc` CryXMLB payloads.
- It does not resolve DigitalSignage media paths, provider links, or UI
  texture preview assets beyond what the canvas render covers.
- It follows `ObjectContainerModifier` children like any other external child;
  it does not classify a modifier as a visual or non-visual overlay.
- It does not understand `.brmp` or `.altg` — likely non-visual metadata, but
  flagged for future reverse-engineering if culling/grouping data is needed.
- It does not resolve embedded DDS textures inside a socpak as first-class
  texture sources.
- It does not classify `.ale` area files; monitored zones can be skipped for
  visual export, but should not be mistaken for missing mesh data.
- It does not read `entdata/*.entxml`; entdata child transforms remain
  cross-check data until the coordinate-space difference from root manifest
  placement is understood.

## Practical Rules

- Use full archive paths whenever a manifest gives them. Do not search only by
  filename suffix if an exact `Data\...socpak` path is available.
- Normalize slashes and case, but keep the original archive path in diagnostics.
- Root `<name>.xml` child references define socpak composition.
- `.soc` IncludedObjects define most static mesh placements.
- CGF/CGFM files define vertex colors, UVs, submaterials, and local material
  refs.
- Materials define decals, POM, tint inputs, and RTT/screen behavior.
- Do not assume readable text literals live inside the socpak. Many visible
  text surfaces store localization keys or user-variable bindings in `.soc`,
  while final language strings live in localization files.
- A loc key alone is not proof of visible text. Render it only when attached to
  a visual text/canvas/screen component or a material/mesh that presents it.
- Do not treat location records such as `MissionLocationTemplate` as direct
  visual text sources unless an explicit binding is found. Prefer the placed
  entity's own user variables and UI components.
- Rest & Relax screen art also exists as
  `Data\UI\Textures\Props\Rest_Stop\RestandRelax` DDS assets; other screens
  may instead point at Building Blocks canvases, `.bk2` video, `.gfx` Flash, or
  runtime providers.
- Export RTT placeholders/virtual inputs until the real DataCore/UI/localization
  source is identified, then upgrade them to resolved text, static preview
  textures, or media-backed placeholders.
- Do not skip `ObjectContainerModifier`; inspect and filter by actual visual
  contents.
- Do not skip brush geometry by folder name; skip by material or proven
  non-visual class.
- Skip `.ale` monitored-zone files for normal visual export.
- Treat transit/shop/AI/audio/navigation metadata as non-visual unless it
  points to a renderable mesh, light, decal, or screen.
