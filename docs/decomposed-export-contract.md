# Decomposed Export Contract

The decomposed export (`--kind decomposed`) writes a reusable shared-root
package at a caller-selected export directory. The writer is
`crates/starbreaker-3d/src/decomposed.rs`; the Blender addon reads it through
`blender_addon/starbreaker_addon/manifest.py`.

Within that export root:

- `Packages/<package name>/scene.json` describes the root entity, child attachments, interior placements, light definitions, and shared asset references. The package name is the cleaned entity label plus `_LOD<n>_TEX<m>` (e.g. `RSI Aurora Mk2_LOD0_TEX0`).
- `Packages/<package name>/palettes.json` contains shared palette identities that scene instances reference by `palette_id`.
- `Packages/<package name>/liveries.json` groups scene and material usage by shared palette identity.
- `Packages/<package name>/paints.json` (only when the entity has paint variants) lists each variant's `subgeometry_tag`, `palette_id`, `display_name`, and `exterior_material_sidecar`. The addon treats it as optional.
- `Packages/<package name>/animations/<clip>.json` holds per-clip animation bodies (see [Animation Records](#animation-records)).
- `Packages/<package name>/weapon_assembly_diagnostics.json` is written only for FPS weapon assemblies that produced diagnostics.
- `Data/...` contains reusable mesh assets, material sidecars, and exported textures using canonical P4k-style paths rooted at `Data/`. Mesh assets are `.glb` with the Glb format and `.blend` with the Blend format (`mesh_asset_extension`); the Blend format also writes `Packages/<package name>/scene.blend`, which links the mesh `.blend` files.
- exporting another ship to the same root reuses matching `Data/...` assets instead of duplicating category-specific copies.

## Scene Manifest

### Glb format

Callers requesting `ExportKind::Decomposed` with `ExportFormat::Glb` now receive
a GLB package with `scene.json`, rather than passing through native Blend
assembly and receiving `scene.blend`. The `__child` mesh rewrite step belongs
to Blend assembly and is Blend-only. The decomposed mesh writer explicitly
sets `GlbOptions.preserve_material_slots`; the behavior does not depend on the
free-form export-kind metadata label. Bundled callers leave that option false.

### GLB mesh assets

The entity CLI accepts `--kind decomposed --format glb|blend`. Socpak packages (`socpak export --kind decomposed`) accept the same formats (GLB through the shared decomposed
exporter, as for entities); `--dds` writes the game's DDS blocks (`TextureContainer::Dds`) instead of PNG. Decomposed
exports default to `blend`; bundled exports continue to use GLB. GLB packages
contain actual per-mesh GLBs and a JSON scene manifest, without `scene.blend`.
The validator accepts both formats and parses GLB mesh payloads structurally.

Decomposed GLB materials are named `<mtl stem>_<submaterial name>_<NN>`, with
`NN` a minimum two-digit decimal index. A primitive's material index `i` is the
array position `submaterials[i]` in its material sidecar, not that entry's
`index` field (which retains the original source slot before hide filtering).
Each mesh retains all sidecar slots through its highest referenced slot;
unused slots within that range are material entries without primitives.
Trailing unused slots are omitted. Meshes sharing a sidecar therefore use the
same material index space. Bundled GLB material naming/deduplication is unchanged.

For the LIVE 4.10.2 Gladius fixture at LOD 1 / mip 2, the root has 274 nodes,
89 meshes and 27 GLB materials: its shared sidecar has 28 slots, and the highest
referenced slot is 26. The package has 45 non-empty mesh GLBs, 23 material
sidecars, 366 PNGs and 63 animation sidecars.

### Game physics (`include_physics`)

`ExportOptions.include_physics` (default `false`; upstream output unchanged) adds the source's CryPhysics
collision geometry (`.cga` / `.cgf` IVO chunk `0x58DE1772`, decoded by `ivo/physics.rs`) to decomposed GLB mesh
assets (`decomposed_physics.rs`):

- A glTF node of an NMC node carries `extras.sc_physics`: an array of records in that node's own frame (the frame
  of its render vertices). Every record has `part` (the source key's high half: a per-node index, **not** a
  collision filter) and `kind`: `box` (`center`, `half_size`, `axes` = 3 row vectors), `sphere` (`center`,
  `radius`), `cylinder` / `capsule` (`center`, `axis`, `radius`, `half_height`), `trimesh` (`mesh` = index of an
  unreferenced one-primitive glTF mesh, `vertices`, `triangles` counts), or `damage_mesh` / `sdf` (`bytes` only:
  Star Engine damage physics, not collision).
- `class` is present when the node name gives one (`physics_class.rs`, the engine's naming convention; the chunk
  itself carries no filter): `interaction` (`$IP_*` interaction volumes), `tires_only`
  (`TIRES_ONLY_COLLISION(S)`), `player_only` (`PLAYER_ONLY_COLLISIONS`), `no_terrain` (`IGNORE_TERRAIN_COLLIDE` /
  `_COLLISION`, `NO_TERRAIN_COLLISION`), `grid_volume` (`physicsgrid_` / `localgrid_allow_no_collide*`). Absent:
  an ordinary collider.
- Nodes the export drops (`filter_nmc_hierarchy`, e.g. `$proxy_*`) that carry colliders are recreated as
  transform-only nodes. Whole-object physics (`0xFFFF`) sits on the object's single geometry node, else on a
  `$physics_object` node whose frame is the object's.
- A character definition (`.cdf`, CryXmlB) carries no physics chunk: its mesh asset gets the physics of each
  attachment's `Binding` file (in 4.10.2 all 148 distinct `CA_BONE` `.cgf` / `.cga` parts of the 1 703 `.cdf`
  carry `StatObjPhysics`). A `CA_BONE` part's records sit under the glTF node of its `BoneName` (the frame its
  rigid render part moves with; per-node records on transform-only recreations of the part's NMC nodes,
  whole-object records on a matrix-less `$physics_object` child); a part on no bone, or on a bone the GLB lacks,
  sits in the object's frame as above. Each such record carries `source`, the attachment's `Binding` path.
- `root_entity.physics_grid` (vehicles; `physics_grid.rs`): the vehicle's physics-grid volume from DataCore
  `VehicleComponentParams.physicsGrid.physicsProxy` — `{ "source_path", "shapes": [...] }`, shapes as above but in
  the root entity's frame (CryEngine Z-up metres) with trimesh `vertices` / `triangles` inline: each node's NMC
  matrix composed down its parent hierarchy, as the GLB writer places render vertices (`nmc_world_matrices`; an
  identity-or-zero matrix is identity). Each shape is its own closed volume. Inside it an actor belongs to the
  vehicle's grid, where the root entity's own colliders (its exterior hull) do not block it.
- Asset reuse (`existing_asset_paths`, the CLI's `--skip-existing-assets`) matches paths only: an existing GLB is
  reused whether or not it was written with `include_physics`. Do not mix exports with and without physics in one
  output root (the CLI and app always export without; the asset path carries no physics flag).

### Manifest fields

`scene.json` (`build_scene_manifest_value`) carries these top-level keys:

- `version` (`2`) and `export_kind` (`"Decomposed"`)
- `package_rule`: `root`, `package_dir` (`Packages/<package name>`),
  `paths_are_relative_to_export_root` (always `true`), `shared_asset_root`
  (`Data`), `normalized_p4k_relative_paths`. All asset paths in the package are
  relative to the selected export root.
- `root_entity`: `entity_name`, `geometry_path`, `material_path`,
  `mesh_asset`, `material_sidecar`, `palette_id`, and optionally `animations`
  (only the root entity carries animations) and, in character packages,
  `character` (the resolved CHF material customization and HumanSkin_V2
  wrinkle-slot layout; written by `character/package.rs`), and with
  `include_physics` `physics_grid` (vehicles; see Game physics). `mesh_asset` is
  `null` when the root has no geometry (no vertices, no NMC nodes — e.g. a
  socpak scene root, or a character whose head a full helmet hides); the
  `.blend` then has no root mesh object and the package armature sits at the
  entity root.
- `export_options`: `kind`, `format`, `material_mode`, `lod_level`,
  `texture_mip`, `include_attachments`, `include_interior`, `include_lights`
- `children[]`: one scene instance per attached child entity (below)
- `interiors[]`: interior containers (below)
- `assembly_kind` and `weapon_assembly`, only for FPS weapon packages
  (`assembly_kind = "fps_weapon"`)
- `controls.engine_glow`, only when at least one engine-glow target exists
  (below)

### Child scene instances

Each `children[]` entry (`scene_instance_json`) carries:

- `entity_name`, `geometry_path`, `material_path`, `mesh_asset`,
  `material_sidecar`, `palette_id`
- `item_type`: authored `SAttachableComponentParams.AttachDef.Type`, or null when absent. Consumers of older manifests must also accept an absent field; do not infer categories from entity names.
- the attachment relationship: `parent_node_name`, `parent_entity_name`,
  `offset_position`, `offset_rotation`, `no_rotation`, `detach_direction`
  (the port's detach direction in port-local source axes), and `port_flags`
- the exporter-resolved transform: `source_transform_basis`,
  `local_transform_sc`, `resolved_no_rotation` (see
  [Transform Fields](#transform-fields))
- `ui_bindings[]` (see [UI Bindings](#ui-bindings))

`port_flags` is the raw source `SItemPortDef.Flags` string for the item port that attached a child. Importers can use this to preserve source visibility semantics; for example, Blender hides attachments by default when the source port includes `invisible` while keeping the objects present for inspection.

### Interior containers

Each `interiors[]` entry (`interior_container_json`) carries `name`, optional
child-attachment parents (`parent_entity_name`, `parent_node_name`),
`palette_id`, `container_transform`, `placements[]`, and `lights[]` (see
[Light Records](#light-records)). Each placement carries `cgf_path`,
`material_path`, `mesh_asset`, `material_sidecar`, `entity_class_guid`,
`transform`, `palette_id` (a per-placement palette that overrides the
container's: the socpak object's own per-object tint palette, or a
loadout-attached child's palette), and `ui_bindings[]`.

Placements also carry the container entity they are
(`pipeline/interior_entities.rs`, `socpak/entity_links.rs`):

- `entity_class_guid` — the entity class record the placement was spawned
  from: the `.soc` `EntityClassGUID` (also when the entity authors inline
  geometry, which overrides only the class geometry), or the record of a
  loadout-attached item; `null` for `IncludedObjects` geometry.
- `entity_id` — the `.soc` `EntityCryGUID`; `null` for `IncludedObjects`
  geometry and loadout-attached items. Ids are unique within one `interiors[]`
  entry only: a container instanced several times repeats its ids, so joins
  stay inside one entry.
- `links[]` — the ids the entity's `EntityLinks` target, in authored order
  (e.g. a door control panel links the door it operates). Targets that are not
  exported (light chipsets, cameras) remain in the list.
- `loadout_owner_entity_id`, `item_port_name` — for a loadout-attached item:
  the `entity_id` of the container entity whose loadout (directly or through
  nested items) attached it, and the port it occupies on its parent. An
  entity's inline `EntityComponentDefaultLoadout` replaces its class loadout
  (e.g. a door's control panels at `control_panel_a` / `control_panel_b`).

Animated placements (an `.cga` whose class has `SAnimationControllerParams`)
take their clips and Mannequin fragments from that class (see
[Animation Records](#animation-records)).

`proximity_sensors[]` lists the container's walk-up triggers: entities whose
class has `SCItemProximitySensorParams` (recognised by that component, not by
class name). Each carries `entity_id`, `entity_class`, `entity_class_guid`,
`transform` (4×4 column-major, container-local: the frame of `placements[]`),
`volume`, `exit_delay` (seconds, the class's `exitDelay`) and `links[]` (the
ids the sensor drives, e.g. a door). `volume` is `{"kind": "box", "extent",
"offset"}` from `SCItemProximitySensorBoxParams`, as authored, in the sensor
frame. The `.soc` `Radius` of these entities equals `|extent|`, consistent
with a box spanning `offset ± extent`; the half-size reading is inferred from
that, not documented by the data. Other shape types are not exported (logged).

A door's interactions therefore join as: its placement `entity_id` ←
a panel placement's `links[]`, ← a panel placement's
`loadout_owner_entity_id`, ← a sensor's `links[]`.

`room_volumes[]` lists the extents of the rooms the container registers in
`<name>/metadata/entitycomponentroom.xml`, one entry per `AreaComponent`
segment that links a registered room GUID: `room` (GUID), `room_name` and
`source` (labels only), `transform` (4×4 column-major, segment space →
container-local space, the frame of `lights[]`), and `kind`: `box` (`box_min`,
`box_max`), `prism` (`points` in the segment XY plane, extruded along +Z by
`height` from the lowest point) or `trimesh` (`vertices`, three per
triangle). A light flagged `affects_this_area_only` is confined by the engine
to the area it sits in; the volumes let a consumer make that decision. The
array is empty when the container registers no rooms. A point on a shared
edge or vertex of a `trimesh` must be counted once: `RoomVolume::contains`
(`socpak/room_containment.rs`) casts +Z and breaks edge ties symbolically.

### Engine-glow controls

`controls.engine_glow` is additive metadata for engine-emission tuning in
importers:

- `label` (`"Engine Glow"`) and `units` (`"material_property_glow"`): the value
  written to the material's **Glow** property, not lumens, normalized thrust,
  or the multiplier's input parameter
- range metadata (`min_strength`, `max_strength`, `default_strength`) derived
  from the authored modifier chains as described below
- `targets[]` keyed by `mesh_asset` + `material_sidecar` + `source_material_index`
  (deduplicated on those plus `geometry_path`)
- each target also carries `entity_name` and `geometry_path` (source evidence
  for the target entity) and the informative labels `submaterial_name` and
  `blender_material_name`
- target records come only from child entities whose DataCore
  `Category` is `Thruster` **and** whose `SAttachableComponentParams.AttachDef.Type`
  is `MainThruster` (`should_export_engine_glow_targets`), and only from
  an enabled DataCore `SEntityComponentEffects.entityEffects.materialEffects`
  link writing `materialProperty = Glow`. Static emission alone never qualifies.
  A named `subMeshName` joins exactly (case-insensitive) to the owning entity's
  NMC node, then to source material IDs. Empty and `"*"` selectors apply to the
  owning entity's entire mesh; they do not require an NMC node name.
  Both enabled `tagEffects` and `triggerEffects` are supported. A node-level
  write qualifies only glow-capable slots: forward-rendered Illum/HardSurface
  (`MtlFlags & 0x8000`), or a material with an `EMITTANCE_MAP` /
  `MACRO_EMISSIVE_MAP` feature token. Deferred materials without those sources
  do not become targets merely by sharing the node. Neither material names nor
  texture names are emission evidence. Dynamic bindings do not change static
  sidecar emission strength
- each target has `property_links[]`, preserving `effect_kind` (`tagEffects` or
  `triggerEffects`), `effect_index`, `property_link_index`, `submesh_name`,
  `parameter_record_id`, `tag_record_id`, and the ordered `modifiers[]` chain.
  A modifier is `{kind:"multiplier", multiplier:...}` or
  `{kind:"min_max", minimum:..., maximum:...}`. Unknown or malformed modifier
  types fail export instead of silently producing a fabricated range
- per-target `multiplier[]`, `minimum[]`, and `maximum[]` are parallel arrays
  aligned with `property_links[]`, so several tag effects retain separate values
  on one material target. The same summary fields occur on each link.
  `multiplier` is the product of its authored multipliers (null if none);
  `minimum` / `maximum` are output bounds after applying the ordered chain,
  with null for an unbounded/unauthored bound. The full chain remains authoritative
  when a clamp precedes a multiplier. Multipliers are dimensionless;
  output bounds and `static_glow` use Glow property units

Control bounds are the union of the individual link output ranges: the lowest
minimum and highest maximum, or null when any contributing bound is unknown.
The manual control's initial `default_strength` is the derived lower bound (also
null if unknown), not a prediction of the active game tag or idle thrust.
Every target has at least one authored property link; `static_glow` only records
the material's original value and does not supply control bounds.
There is no invented input-parameter domain or display cap; very
large authored clamp maxima are preserved verbatim. Importers must handle null
bounds and must not sum mutually exclusive effects just because both are listed.
Evaluating game tags and their input parameters remains runtime behaviour.

### Importer-only decal-host fields

`decal_host_channel` (a palette channel hint such as `primary`, `secondary`,
`tertiary`, or `glass`) and `decal_host_rgb` (a fixed linear RGB host colour)
are read from scene instances by `manifest.py` (`SceneInstanceRecord`), but the
exporter **never emits them**. They are reserved: the addon resolves MeshDecal
`Host Tint` routing itself (and, for FPS weapons, by local spatial
host-material matching) when they are absent, which is always today.

## Light Records

Schema v2 exports attached vehicle light groups in `root_entity.lights` and
`children[].lights`. Lights without a resolved group helper remain in
`interiors[].lights`; a source light occurs **once**, never in both arrays.
All entity arrays are emitted (empty when none). Standalone SOC packages retain
the container representation. Schema-v1 consumers must add entity light-array
support before consuming v2 scenes.

Vehicle discovery follows DataCore `VehicleComponentParams.objectContainers`
and loadout children. In current game builds, the MultiLight XML port can be
unequipped: actual groups are authored as SOC `SLightGroupComponentParams`
with baked `LightComponentParams`, not as a ship-specific equipped light item.
`root_entity.vehicle_light_ports` and `children[].vehicle_light_ports` preserve each owner's XML ports and their
`attachment_points` map (point name -> bone), including modification overrides.
Explicit SOC group helpers take precedence over XML attachment-point mappings.
Do not infer a missing item, light, helper or colour from an asset name.

Entity light records add `owner_entity`, `source_container`,
`source_container_index`, `source_light_index`, `source_parent_node_name`,
`parent_node_name` (actual resolved NMC node), `parent_local_position` and
`parent_local_rotation`. Their flat position/rotation are **owner-entity local**;
parent-local values are an equivalent transform for attaching the light to an
animated helper. Apply either the entity-local transform or the parent-local
transform under that helper, never both. Interior light records retain their
container-local transforms and add the same source indices. Those indices join
the metadata-only `entity_light_catalogue` with an export using the same options;
they are not persistent IDs across source builds or export-option changes.

`attachment` is null for single lights, or contains the source `group`, optional
`helper`, and optional `attachment_point` parsed from SOC `TileAttachmentParams`.
A group that names an item port (`TileAttachmentParams@ItemPortName`) of the
container descriptor (`<name>.xml`, `TileItemPortEntries/ItemPort`) is placed
with its origin on that port's `interactionOffset`, which is authored in the
frame of the entity that owns the container; the group's lights keep their
offsets from the group origin and their orientation (the descriptor ports
carry no usable rotation). The `.soc` position of such a group is in the same
frame for some containers and container-local for others, so it is not used
for placement. This applies to the containers listed directly on an entity's
`VehicleComponentParams.objectContainers`; groups in nested child containers
and in containers of loadout children keep their `.soc` placement.
A helper-parented container's `container_transform` is relative to that
helper (`parent_node_name` of `parent_entity_name`); the bundled GLB hangs
the container node under the helper node, or under the owner entity node
when the container names no helper, and falls back to the scene root with a
warning only when neither node exists.
A missing or ambiguous helper leaves the light in its original container and
emits a diagnostic; the exporter does not substitute the origin. Empty groups
produce no fallback lights. The catalogue keeps all lights and their resolved
entity positions, including lights retained in interior arrays by the exporter.
Catalogue projector references are source paths; exported references are images.

Each light record (built in `decomposed.rs` from the `LightInfo` parsed in
`socpak.rs`) carries:

- `name`, `light_type` (the raw source type: `Omni`, `SoftOmni`, `Projector`,
  `Ambient`, …), and `semantic_light_kind` (a normalized reusable kind:
  `point`, `spot`, `area`, `sun`, or `ambient_proxy`)
- `position`, `rotation` (quaternion `[w, x, y, z]`), and `direction_sc` (the
  normalized SC-space forward direction, so consumers need not derive a
  spotlight axis from the quaternion), all in the basis named by
  `transform_basis` (`"cryengine_z_up"`). The Blender addon applies the axis
  conversion and the spot-axis basis correction.
- `color` (linear RGB; derived from the active state's temperature when the
  component sets `useTemperature`)
- `intensity_raw` (the authored CryEngine intensity of the active state),
  `intensity_unit` (`"cryengine_authored_intensity"`), and
  `intensity_candela_proxy` (the exporter's candela-style scaled value);
  `intensity` is a legacy alias of `intensity_candela_proxy`.
  `intensity_candela_proxy_model = "raw_x1500"` labels the retained compatibility
  recipe, **not physical candelas**. Ambient proxies still apply the existing
  `glow_multiplier × 0.25` gain after ×1500; this change does not recalibrate it.
- `radius` and its explicit metric duplicate `radius_m` (attenuation radius in
  metres)
- `inner_angle` / `outer_angle` for projectors: **half-angles** in degrees
  (`outer = FOV / 2`, `inner = 0.8 × outer`); `null` for other types
- `projector_texture` for projector gobos: the export-root-relative path of
  the exported texture, `.exr` when the source is HDR (BC6H), otherwise `.png`
  (a white PNG fallback if both fail); `null` for non-projector lights
- `light_style`, the active state's authored style (also retained per state)
- `active_state` and a `states` map capturing every authored CryEngine state
  (`offState`, `defaultState`, `auxiliaryState`, `emergencyState`,
  `cinematicState`; missing states are absent). An authored `defaultState` is
  always active, including intensity **0**; its colour, style and intensity
  populate the flat fields. Only when defaultState is absent, select the first
  positive-intensity state in `auxiliary → emergency → cinematic` order.
  If none qualifies, `active_state` is empty and the flat intensity is zero. The full map lets the Blender addon switch between
  states at runtime without re-exporting.

Each `states` entry carries `intensity_raw`, `intensity_unit`, `intensity_cd`,
`intensity_candela_proxy`, `intensity_candela_proxy_model` (`"raw_x1500"`),
`temperature` (Kelvin), `use_temperature`, `color`,
`light_style` (the authored CryEngine light-style id, e.g. strobe patterns),
and `preset_tag` (an optional authored preset tag such as `fast` / `slow`).
`temperature` and `use_temperature` exist only per state, not on the flat
record. Background on the light data: `docs/star-citizen-lighting-research.md`.

The following component fields appear in all three scene light arrays,
the metadata-only light catalogue, and each
bundled GLB `KHR_lights_punctual.lights[].extras` record:

Sparse entity components are overrides of their `EntityClassDefinition` record.
At export time, the `LightComponentParams` record is read from DataCore and
merged attribute by attribute, including nested state, size, shadow, fade and
miscellaneous parameters. Explicit zeroes, false flags and empty strings override
class values. Missing state intensity inherits the matching class state; it is
never interpreted as an authored zero. Class states also participate in default
state selection. Low-level SOC parsing without DataCore retains the unresolved
source for the export pass.

The final column below applies only when both the component and class lack the
attribute.

| JSON field | Authored source | Missing attribute |
| --- | --- | --- |
| `bulb_radius_m` | `sizeParams.bulbRadius` | 0.05 m (CryEngine Light.lua default) |
| `plane_width_m`, `plane_height_m` | `sizeParams.planeWidth`, `planeHeight` | null |
| `shadow_casting` | `shadowParams.shadowCasting` | null |
| `max_shadow_cast_distance_m` | `shadowParams.maxShadowCastDist` | null |
| `affects_this_area_only`, `affect_gi` | component `affectsThisAreaOnly`, resolved `affectGI` | null |
| `attenuation_tweak` | `miscParams.attenuationTweak` | 1.0 |
| `plane_softness` | `miscParams.planeSoftness` | null |
| `fade_min_distance_m`, `fade_max_distance_m` | `fadeParams.minDistance`, `maxDistance` | null |
| `importance` | component `importance` (integer or DataCore enum option index) | null |
| `glow_multiplier` | `miscParams.glowMultiplier` | null |

LIVE authors `importance` as a named enum. `importance_name` preserves that name,
while `importance` is the zero-based option index read from DataCore's
`LightComponentParams.importance` property enum at run time; no ordinal table is
copied into the exporter. Numeric XML importance is kept directly. Low-level SOC
parsing without a database (or an unknown enum name) preserves the name and
leaves the index null; the shared interior-building path resolves it for scene,
catalogue and bundled exports.

`affect_gi_mode` preserves the original `affectGI` spelling. `affect_gi` resolves
`On` / `Off` or boolean spellings directly, and `InheritAffectsObjects` from the
merged component's `affectsObjects` flag. `affect_gi` is nullable: unknown modes
or a missing inherited objects flag remain null, preserving the raw mode.
Unresolved GI warns once per export. Missing importance enum metadata also
warns once per export; ordinals remain null and source names are preserved.

All dimensions/distances are source metres; explicit zeroes remain zero.
No bulb-size or attenuation hypothesis is baked into the proxy. Optional numeric
fields serialize as null rather than invented dimensions or distances.
GLB extras preserve the entire LightInfo, including all states and source fields;
the standard glTF intensity/range remain compatibility values.

The Blender addon enables `use_shadow` whenever `affects_this_area_only` is true
to substitute for unavailable vis-area containment; otherwise it honours
`shadow_casting`. It maps `bulb_radius_m` to
`shadow_soft_size`, and plane width/height to rectangular AREA `size`/`size_y`.
Missing/null dimensions preserve the older package's import behavior. Zeroes and
false flags are applied, not mistaken for missing data. The active state is used
as exported; an off default is not replaced with a lit auxiliary state.

Scene and material schema versions remain **2**. Consumers must accept null
for unresolved flags, radius, glow multiplier, light type, colour, style, state
intensity, temperature, temperature-use flag and preset tag.
The intensity aliases/proxies are null if their source inputs are unknown.
The documented engine defaults apply only to bulb radius (0.05 m) and attenuation
tweak (1.0), after class inheritance. Bundled glTF leaves unresolved intensity
dark because its standard light object requires a number; its extras retain
null, an unknown/nonpositive range is omitted, and an unknown colour uses neutral
white only in the renderer (with a diagnostic). Native Blender lamp assembly
skips lights whose intensity, radius, colour or type cannot be resolved, with a diagnostic;
scene metadata retains them. A missing temperature does not enable blackbody
colour conversion. The addon preserves null state values during manifest parsing
and state serialization; switching to a state without a temperature keeps authored
RGB and clears any stale temperature property. Package caches must include the exporter revision/format
identity so changes to inheritance or field semantics invalidate older exports.

### Cross-DCC Semantic Fields

The semantic fields above (`semantic_light_kind`, `transform_basis`,
`direction_sc`, `intensity_raw`, `intensity_unit`, `intensity_candela_proxy`,
`radius_m`) were added beside the raw fields rather than replacing them. They
are intentionally DCC-agnostic and must not encode Blender-only decisions such
as final Watt conversion, Blender spotlight-axis corrections, or Blender gobo
node-graph details.

Proposed but **not emitted**: `outer_angle_full_deg` / `inner_angle_full_deg`
(explicit full-angle names) and `projector_texture_format_hint`. Consumers
must treat `inner_angle` / `outer_angle` as half-angles.

### Transform Fields

Every child scene instance carries an exporter-resolved local transform
(`resolve_child_instance_transforms`):

- `source_transform_basis` — basis label for `local_transform_sc`. Child
  instances emit `"gltf_y_up"`: the matrix is the child's glTF node matrix from
  the exporter's scene-graph builder. The addon also accepts
  `"cryengine_z_up"` (and then applies its scene axis conversion); it applies a
  `"gltf_y_up"` matrix directly as the Blender basis.
- `local_transform_sc` — the 4×4 local matrix relative to the exported parent,
  as four column arrays (column-major, as glTF stores it).
- `resolved_no_rotation` — `true` when the child uses `no_rotation`, meaning
  the exporter has already baked the helper-suppression rule (and
  duplicate-offset suppression) into `local_transform_sc`; importers should
  consume that matrix directly rather than re-implementing the rule.

A world-space `world_transform_sc` was proposed but is **not emitted**.
Interior containers and placements carry plain `container_transform` /
`transform` matrices and no basis label.

The exporter owns Star Citizen-specific transform composition, helper
resolution, and duplicate-offset suppression. Importers own only the final
mapping into their host DCC's coordinate system.

### Compatibility Rules

- Older sidecars that lack the semantic fields remain valid; the addon falls
  back to the raw `light_type`, `position`, `rotation`, `intensity`, `radius`,
  and per-state payloads (`manifest.py` reads `intensity_candela_proxy`
  falling back to `intensity`, and `radius_m` falling back to `radius`).
- The raw fields continue to be emitted alongside the semantic fields.
- Blender-specific behavior such as final Watt conversion, gobo UV wiring,
  per-image gobo compensation, and shadow soft-size heuristics stays in the
  importer.

Current retained compatibility aliases and fallbacks:

- `intensity` remains on the flattened light record as the legacy exporter
  candela proxy field; importers should prefer `intensity_candela_proxy` when
  present.
- `intensity_cd` remains on state snapshots as a compatibility alias for older
  runtime JSON payloads; importers should prefer `intensity_candela_proxy` and
  fall back to `intensity_cd` only when needed.
- raw `offset_position`, `offset_rotation`, and `no_rotation` remain part of
  the public sidecar for debugging and older importer compatibility even though
  migrated consumers should prefer `local_transform_sc` and
  `resolved_no_rotation`.

## Material Sidecars

A sidecar is shared by every mesh of its source `.mtl`. Submesh slots
follow the engine (`crates/starbreaker-3d/src/material_slots.rs`, CryEngine
`GetSafeSubMtl`): a `.mtl` without `<SubMaterials>` serves every slot, and a
slot past a multi-material's submaterials (a mesh compiled against an older
`.mtl`) draws with the engine default `Materials/material_default.mtl`. Such
a mesh's sidecar appends that material at the slot's `index`, so its content
differs and it is written under a hashed variant of the shared path.

Each `*.materials.json` sidecar preserves:

- source material path and geometry path
- per-submaterial name, raw shader string, shader family classification if known, and activation state
- decoded feature flags from `StringGenMask`
- direct texture-slot inventory with semantic roles, virtual-input flags, source paths, and exported texture paths
- screen-family `screen_effects` metadata; `apply_crt = true` is emitted when
  authored DisplayScreen/UIPlane data exposes a `screen_pixel_layout` slot, with
  the source slot/path and authored `PixelGridTilingX/Y` values when present
- DisplayScreen/UIPlane render-to-texture slots carry no image of their own;
  generated screen images are attached per scene instance through
  `ui_bindings` (see [UI Bindings](#ui-bindings))
- DDNA identity markers on exported normal-gloss source PNGs plus `alpha_semantic` / `alpha_channel` markers such as `smoothness` in alpha when the source texture carries shader-relevant data
- derived DDNA roughness references that describe their source texture identity, source channel, value transform, packed output channel, and constant packed channels
- structured `texture_transform` objects derived from authored `TexMod` blocks when texture UV animation or tiling metadata is present
- public params as structured JSON values where simple coercion is safe
- layer manifests including source material paths, authored layer attrs, `Submtl`-selected resolved layer-material metadata, palette routing, UV tiling, resolved layer snapshots, per-layer semantic `texture_slots`, and exported layer texture references
- authored material-set metadata such as root attributes and root-level `PublicParams`
- authored submaterial attributes exactly as read from the `.mtl`
- authored per-texture metadata, including nested child blocks such as `TexMod`
- authored non-texture child blocks such as `VertexDeform`
- material-set identity and palette-routing metadata
- resolved paint-override selectors when equipped paints choose a palette or material through `SubGeometry` tag matching
- variant-membership hints for palette-routed and layered materials

### Curated emission and decal semantics (material schema v2)

Every submaterial has `emissive`. The validator checks explicit integer schema
versions on root/package `scene.json` and material sidecars: `1` and `2` are
accepted; missing, non-integer and unsupported versions are errors.

- `color`: tint-only linear RGB factor. Nonzero `Emissive` is the tint; an all-zero
  triplet means no tint and exports `[1,1,1]`.
- `strength`, in priority order: positive `Glow`; otherwise `1`
  for Illum/HardSurface with `MtlFlags & 0x8000` and non-black Emissive;
  otherwise `1` for an enabled `%EMITTANCE_MAP` or `%MACRO_EMISSIVE_MAP` token;
  otherwise `0`. Both tokens count as emittance maps.
  The forward flag is CryEngine `MTL_FLAG_REQUIRE_FORWARD_RENDERING`. This rule
  identifies forward glow cards without using material or texture names.
  LayerBlend_V2 and non-forward materials never emit from Emissive alone.
- `forward_rendering`: the authored `MtlFlags & 0x8000` decision, also present
  in `alpha_config`. The flag alone does not imply emission (runtime-driven
  thruster surfaces and glass can also have it). `MtlFlags` is parsed as signed
  64-bit and reinterpreted as bits, preserving negative authored values.
- `glow`: parsed raw scalar (`Glow`/`GlowAmount`); absent is zero.
- `uses_base_texture`: false for `%EMITTANCE_MAP` or `%MACRO_EMISSIVE_MAP`, true
  otherwise. These are exact decoded permutation tokens, not slot/asset names.
- `emittance_texture`: original slot reference or null. With the corresponding
  token enabled, explicit `Emittance`/`Emissive` bindings or HardSurface's
  documented `TexSlot14` locate the map. A token without a resolvable binding
  still enables strength but leaves this reference null; no texture is invented.
- `alpha_glow_cutoff`: authored scalar from PublicParams/attributes, or null.

Final emission is `surface × color × strength`, where surface is
`Diffuse × base-colour texture` when `uses_base_texture` is true, or the emittance
map otherwise. Black Emissive with positive Glow therefore emits its surface
colour. Neutral white Emissive with no Glow/map stays off unless it is an
authored forward Illum/HardSurface card. Colour remains tint only; importers must
not apply strength or Diffuse twice.

`SubMaterial::emissive_factor()` returns `color × strength`. This shared fix also
changes bundled GLB emissive factors and the emissive-texture/screen-back-colour
paths in `pipeline/textures.rs`; it is not confined to sidecars. Bundled texture
baking remains an approximation of the source shader.

Scene and material sidecars remain schema **v2** after these corrections. The
validator continues to accept both v1 and v2; raw authored fields are preserved.
The viewer cache format is independently bumped to v6 to invalidate earlier
v2 packages with the superseded emission semantics.

Every submaterial also carries `alpha_config`:

- `forward_rendering`: authored `MTL_FLAG_REQUIRE_FORWARD_RENDERING` (0x8000).
- `two_sided`: authored `MTL_FLAG_2SIDED` (0x2), independent of shader inference.
- `alpha_test` and `opacity`: parsed authored `AlphaTest` and `Opacity`, with the
  existing parser defaults 0 and 1 respectively when absent. Raw attribute
  presence remains available in `authored_attributes`.

Forward emissive cards need soft coverage: reconstruct them as additive or
translucent unlit surfaces, without imposing a host's default mask threshold.
The flags and authored thresholds are provided independently so a forward flag
on a non-emissive material is not mistaken for a glow-card blend decision.
This metadata does not change the legacy bundled glTF `alpha_config()` or
`is_double_sided()` approximation (the latter treats decals as double-sided).

`decal` is null outside MeshDecal. MeshDecal entries carry:

- `kind`: `relief_only`, `color_sheet`, `stencil_palette`, or `unknown`.
  Stencil-map plus the `$TintPaletteDecal` virtual binding selects the palette
  path. POM without structural colour/stencil/tint controls selects relief.
  Otherwise DIFFUSE_MAP with TexSlot1 selects the colour-sheet path; ambiguous
  combinations remain unknown. Classification follows the Blender control-only
  POM exclusions, including DECAL_OPACITY_MAP and slots 5–8.
- `slots`: only present inputs, referenced by their original `TexSlotN` names:
  `coverage`, `base_color` (colour sheets), `normal_gloss`, `height`, `breakup`,
  `tint_mask`, `stencil`, and `grime`. Resolve them through `texture_slots` to
  obtain paths, virtual-input metadata and UV transforms. Relief-only TexSlot1
  supplies coverage alpha, not owned base colour.
- `public_params`: authored `DecalAlphaMultiplier`, `DecalDiffuseOpacity`,
  `DecalFalloff`, `PomDisplacement`, `PomHeightBias` and every `Stencil*` parameter.
  Scalars/bools are JSON numbers/bools; RGB triplets are numeric arrays. Missing
  parameters remain absent; the exporter does not invent shader defaults.

The original `authored_attributes`, `authored_public_params`, `public_params`,
feature flags and texture inventory are preserved. Consumers of v1 sidecars can
continue their existing reconstruction; v2 consumers should prefer these curated
objects. Geometry/material slot indices and exported asset names are unchanged.

Blender importer note:

- Control-only `MeshDecal` POM materials are not treated as visible colour
  decals. When they have POM enabled but no structural decal/stencil/tint-mask
  signals, `TexSlot1` is used as a coverage alpha mask, `TexSlot3` as
  normal-gloss input, and `TexSlot4` as height.
- For FPS weapon packages (`assembly_kind = "fps_weapon"`), Blender represents
  the game-style GBuffer overlay by cloning the matched host material and
  injecting only the POM alpha/normal/height into that clone, so the host
  material remains the owner of base colour, roughness, metallic, and specular
  response.
- For non-FPS packages, the importer keeps the authored `SB_MeshDecal_v1`
  material and applies the existing `Host Tint` channel/RGB rebind. This
  preserves ship-style POM decals that intentionally render as coloured
  MeshDecal overlays rather than FPS weapon relief-only host clones.

The current sidecar contract is now substantially closer to the raw `.mtl` XML surface, but it is still intentionally split into two layers:

- curated semantic fields meant for Blender reconstruction and stable downstream use
- authored XML-derived fields kept for inspection, debugging, and future reconstruction upgrades

### Texture Export Rules

- Decomposed exports now write source textures as `.png` using the original `Data/...` filename with only the extension changed.
### Optional compressed texture container

`ExportOptions::texture_container` is `TextureContainer::Png` by default. This
preserves the existing PNG bytes and paths (and omits the default option from
serialized export options). `TextureContainer::Dds` applies to decomposed Generic
and Normal texture assets, palette decal textures, and DDNA alpha derivations.
Bundled exports, player-package `texture_png`, generated UI PNGs, and gobo PNG/EXR
exports retain their existing paths. `ui_only_files` projected paths remain PNG-only.

Supported sources are written as `Data/.../<stem>_TEX<mip>.dds`; sidecar
`export_path` always names the emitted container. Unsupported sources use the
existing PNG conversion. Block-compressed sources also fall back to PNG when the
selected top mip width or height is not a multiple of 4 (including 1×1, 2×2 and
odd sizes). Smaller tail mips are allowed. Empty chains and chains exceeding
`floor(log2(max(width, height))) + 1` mips cannot pass through. DDS normal maps
pass through BC5 colour mips behind the same normal/gloss filename gate as PNG; their smoothness siblings are separate.
Decomposed DDS assembly defers bundled texture loading for root entities and
children, including standalone FPS weapons. DDS files are reusable texture assets.
DDS normal source refs retain DDNA identity but omit PNG-only alpha-channel claims.
DDNA alpha uses `<stem>_smoothness_TEX<mip>.dds` and its derived ref and derivation
status describe `packed_texture_format = "smoothness"`,
`packed_channel_semantics = { "r": "smoothness" }`, `value_channel = "r"`,
`value_transform = "identity"`, with no constant channels. Provenance still names
the source DDNA alpha, and the semantic role remains `roughness`. PNG fallback
keeps `roughness_grayscale` and `sqrt_one_minus` below.

Package DDS consists of `DDS `, a 124-byte header, an always-present DX10 header,
and the selected mip through the last available mip, verbatim. Selection clamps
to the available colour or alpha chain independently. Width/height describe the
selected mip; mip count describes the written chain; header flags are
`DDSD_MIPMAPCOUNT|DDSD_LINEARSIZE|CAPS|HEIGHT|WIDTH|PIXELFORMAT`, and linear size is
the first written mip's byte count. The pixel FourCC is `DX10`; resource dimension
is 3 (2D), misc flags are zero, and array size is 1.

Pass-through formats are DXGI 61, 71/72, 74/75, 77/78, 80/81, 83/84, 98/99,
87/91, and 28/29. Legacy DXT1, DXT3, DXT5, ATI1/BC4U, BC4S, ATI2/BC5U, BC5S
map respectively to 71, **74 (BC2)**, 77, 80, 81, 83, 84. Legacy R8/L8 mask
0xff maps to 61; RGBA8 and BGRA8 masks map to 28 and 87. Source sRGB encoding
promotes 71→72, 74→75, 77→78, 98→99, 87→91, 28→29, so consumers need only DXGI.
BC6H, unsupported formats, cubemaps, arrays, volumes, sRGB sources without an
sRGB variant, and incorrectly sized mip payloads fall back to PNG. Alpha DDS
chains must have one uniform BC4 UNORM, BC4 SNORM, or R8 format across every
written mip, each with its exact expected byte size; smoothness is linear.

DDS export and both prewarm passes perform no pixel decode or PNG encode for
pass-through assets. DDS derivation diagnostics retain dimensions, mip selection,
and alpha format/layout; decoded smoothness and roughness statistics are omitted.
PNG paths retain their existing statistics. Existing-file reuse distinguishes
DDS and PNG, including PNG fallback when the preferred container is DDS.

The following alpha-in-PNG and grayscale-roughness rules describe PNG export:

- DDNA normal-gloss exports preserve smoothness in the source PNG alpha channel and mark that slot with `texture_identity = ddna_normal`, `alpha_semantic = smoothness`, and `alpha_channel = a` only when the same source has an exported `ddna_derivations[]` smoothness payload. DDNA sources with `status = missing` keep `texture_identity = ddna_normal` but must not claim `alpha_semantic = smoothness`. The DDNA identity is detected from the source filename stem token `ddna`, not from directory names or arbitrary path substrings.
- Rust also emits derived DDNA roughness PNGs when alpha mips are available. Any texture slot classified as `normal_gloss` with `texture_identity = ddna_normal` is eligible, even when the authored DDNA sampler is not the conventional TexSlot2 field. Derived roughness refs are written to `derived_textures[]` for direct material normal-gloss slots and to per-layer `roughness_texture` for layer-material normal-gloss slots.
- Newer sidecars also emit `ddna_derivations[]` beside direct submaterial textures and inside each layer manifest. These records are diagnostic coverage entries for every eligible DDNA normal-gloss source. Each record carries the DDNA transform contract (`derived_from_texture_identity = ddna_normal`, `derived_from_semantic = smoothness`, `derived_from_channel = a`, `value_transform = sqrt_one_minus`, `value_channel = r`, `packed_texture_format = roughness_grayscale`, packed channel semantics, and alpha channel constants) so even `status = missing` entries describe how the source would have been converted. `status = exported` also includes the derived `export_path` plus mip selection and parse metadata (`requested_mip`, `selected_mip`, `mip_selection`, `alpha_mip_format`, `alpha_mip_layout`, `width`, `height`, `alpha_mip_count`), decoded source smoothness statistics (`smoothness_min`, `smoothness_max`, `smoothness_mean`, byte range 0-255), and derived roughness statistics (`roughness_min`, `roughness_max`, `roughness_mean`, byte range 0-255). `mip_selection = requested` means the requested texture mip had a matching alpha mip; `mip_selection = clamped_to_available_alpha_mip` means the requested mip exceeded available DDNA alpha mips and the exporter used the last available alpha mip. `alpha_mip_format` is one of `bc4_unorm`, `bc4_snorm`, or `r8_unorm`. `alpha_mip_layout` is one of `numbered_sibling`, `headered_tail`, `raw_tail_split`, or `raw_single_payload` and describes how the selected alpha mip was sourced from split CryEngine DDS siblings. `status = missing` includes a structural `reason` such as `missing_alpha_mips` when the source DDNA has no alpha/smoothness payload.
- Split DDNA alpha/smoothness mips are decoded according to their alpha tail DDS header when present. Raw `.a` tail payloads without headers are split across the remaining mip levels when their total byte size exactly matches raw `R8_UNORM` or BC4 mip sizes; otherwise they keep the legacy single-payload fallback. Raw alpha sibling payloads are inferred from the current mip dimensions when the byte size unambiguously identifies raw `R8_UNORM`; otherwise BC4 remains the default. DX10 `R8_UNORM` alpha tails and inferred raw R8 siblings are treated as one-byte-per-pixel smoothness.
- Every texture ref with `role = normal_gloss` and `texture_identity = ddna_normal` must have a matching `ddna_derivations[]` record by normalized `source_path`, including identity-only refs whose smoothness payload is missing.
- Derived DDNA roughness references use `export_kind = derived_ddna_alpha`, `derived_from_texture_identity = ddna_normal`, `derived_from_semantic = smoothness`, `derived_from_channel = a`, `value_transform = sqrt_one_minus`, `value_channel = r`, and `packed_texture_format = roughness_grayscale`.
- A derived DDNA roughness reference must match an exported `ddna_derivations[]` record with the same normalized `source_path`; its `export_path` is the derivation `export_path`. Direct submaterial refs live in `derived_textures[]`; layer refs live in the layer's `roughness_texture` object.
- Derived DDNA roughness PNGs are decomposed-friendly grayscale roughness images: RGB all store perceptual roughness (`sqrt(1 - smoothness)`) and alpha is unused (`1.0`). The same data is described by `packed_channel_semantics` and `constant_channel_values` on the texture reference. GLB export may still use glTF metallic-roughness packing internally, but decomposed `*_roughness_TEX*.png` assets are grayscale parameter maps.
- Validation checks exported DDNA source alpha and derived roughness PNGs both by metadata statistics and by per-pixel transform: each roughness RGB value must match `round(sqrt(1 - source_alpha / 255) * 255)`.
- Contract groups may expose paired `*_alpha` inputs next to diffuse-style color sockets. The Blender importer resolves those inputs from the alpha channel of the same source-slot texture automatically.

### Remaining XML-first Expansion Priorities

The exporter-side contract gaps are now mostly closed. The remaining work is primarily broader sampling and evidence collection:

- any additional raw submaterial attrs not yet surfaced in the curated semantic contract, especially rare family-specific fields that matter to reconstruction
- broader sampling of non-texture child blocks beyond the currently preserved payload shapes, including any deeper waveform trees that appear in future fixtures
- broader sampling of referenced layer materials to confirm rarer `Submtl` selector patterns and any layer-only child blocks that do not appear in the current fixtures

## Palette And Livery Rules

- Shared palettes are emitted once in `Packages/<package name>/palettes.json` and referenced everywhere else by `palette_id`.
- Material sidecars describe palette routing, but scene instances choose the concrete shared palette.
- `Packages/<package name>/liveries.json` groups entity and material usage by shared palette identity so Blender-side tooling can switch palettes centrally.

## Path Rules

- Source game paths are normalized to forward slashes and kept beneath canonical `Data/...` paths rooted at the export directory.
- Case is canonicalized from the actual P4k entry when possible so `Objects` and `objects` do not create duplicate export trees.
- Canonical textures preserve the original game-relative location whenever a direct source texture exists.
- Generated mesh and sidecar paths remain stable for the same source geometry or material path.

## Animation Records

Default-pose GLB exports bake only the selected clip's keyed components at
their final sample. An unkeyed rotation or position keeps its bind value;
entirely unkeyed bones remain at bind locally. Cached world transforms are
recomputed through the hierarchy, while inverse-bind transforms remain intact.
The bake uses the skeleton's native CryEngine Y-up basis (quaternions reordered
from `xyzw` to `wxyz`, positions unchanged), before normal glTF emission and its
`CryEngine_Z_up` wrapper. The per-sample Blender Z-up conversion below belongs
only to animation sidecars and is unchanged by default-pose baking. Decomposed
viewer exports default to bind pose with sidecars rather than a baked pose.

Only the root entity carries animations: `root_entity.animations` is an
optional array holding every discoverable clip for the root skeleton and the
attached children's skeletons and controller-bearing interior rigs (`decomposed.rs`). Root/shared clips are merged by clip name; child controllers retain their own namespace.
Animations are discovered by:

1. Locating the skeleton's `.chrparams` CryXmlB file (derived by swapping extension from `.chr`)
2. Parsing the animation map and `$TracksDatabase` reference (usually a `.dba` file).
   `#filepath` is a running directive: each event resolves under the `#filepath`
   in effect where it is declared (a file may declare several), directive names
   are case-insensitive, and the prefix is always joined to the event's path by
   exactly one separator (an empty `#filepath` leaves the path as given).
3. Pairing every event with the CAF its resolved path names
   (`animation/matching.rs`): the tracks-database entry of that name — entry `i`
   of the database's metadata owns key block `i` — or else the loose `.caf` file
   at that path. Nothing else decides the pairing: not the bones a block
   animates, not the words in a name. An event whose CAF is stored in neither
   place gets no clip. For the root rig, database entries that no event names
   follow under their database path as the clip name. A key block that cannot be
   decoded yields no clip and leaves a gap: the blocks after it keep their index
   and so their own metadata entry. A loose CAF that is stored but cannot be read
   or decoded is logged as a warning and costs only its own event.

The default-pose bake resolves its event by the same rule (`clips_for_caf_path`),
loose CAFs included.

A clip merged by name across rigs carries every contributor's bones, each tagged
with its `source_skeleton_path`. In the game each rig plays its own animation of
that name, so the clip keeps every contributing animation apart
(`animation/rig_sources.rs`):

- `sources` — the timing and anchor of every contributing animation, in merge
  order: `{skeleton_path, fps, frame_count, start_rotation?, start_position?}`,
  the values of that animation's own clip (keys absent there are absent here).
  Records equal in every field are one record, so an animation reached through
  several entities is listed once. A record identifies a rig's rate, length and
  start pose, not a key block: two animations of one rig that agree on all of
  them share a record. A rig that contributes no channel has none, and every
  record is named by at least one channel.
  `skeleton_path` is the rig the animation was read for; a channel re-stamped to
  the geometry that owns its node keeps its `source`.
- Every channel (each variant of a variant array included) carries `source`, the
  index of its animation in `sources`. Its key times are frames at that
  animation's `fps`, within `0..=frame_count` of that record.
- A bone's entry is one channel or a flat array of variants. A contributor that
  brings an array joins it variant by variant; an array never holds an array.
- The clip-level `fps` / `start_rotation` / `start_position` / `fragments` stay
  the first contributor's. The clip-level `frame_count` is the longest
  contributor's duration in frames at the clip-level `fps` (rounded up). A
  contributor whose rate cannot be related to the clip's (one of the two is
  missing) is logged and does not extend the clip.

A consumer times and anchors a channel by its `sources` record: played forward
every animation starts with the clip and a shorter one holds its last key;
played backward (a fragment with `speed: -1`) every animation starts at its own
end. The Blender addon does not read `sources` yet and still uses the clip-level
values. Clips that were not built by this merge (the player's) have no
`sources`; their clip-level values apply to every
channel.

Known gaps of the pairing (flagged, not worked around):

- The engine's global CAF index (`Animations/Animations.img`) is not parsed, so
  an event whose CAF lives only in a tracks database its own chrparams does not
  declare gets no clip.
- Only the last `$TracksDatabase` of a chrparams is read; a file declaring
  several loses the events stored in the others.
- Wildcard animation rules (`name="*"`) are not expanded into events.
- A merged clip has one set of fragment references (the first contributor's);
  the other contributors' own fragments are dropped.
- Clips are merged by name alone, so unrelated items whose animations share a
  name (the `open` / `close` of a power plant, a jump module and a fuel port)
  land in one clip and play together.
- The Blender addon ignores `sources`: it times every channel by the clip-level
  `fps` / `frame_count` and anchors it on the clip-level start pose.
- A loose CAF holding several key blocks yields one clip per block, all under
  the event's name, so they merge into one clip. Which block the engine plays
  is not known.
- Event names are compared case-sensitively and the first declaration of a name
  wins. Seven chrparams in 4.10.2 declare names differing only in case (a
  consumer with case-insensitive names collapses them), and 45 declare the same
  name more than once (`animation_paths` keeps every path, the export uses the
  first).

When there is no root skeleton (socpak interior exports), child clips are
keyed per entity type so identically-named clips on different entities (every
door's `door_open`) stay separate, and each such clip gains a human-readable
`display_name`.

Child `SAnimationControllerParams` are resolved before merging. Their IDs use
`item_<first 16 SHA-256 bytes in hex>__<clip>` from lowercase owner plus normalized
rig path, preserving sidecar names within ustar limits. Fragment animation IDs use
the same namespace, and the sidecar carries `owner_entity`; `display_name` remains
human-readable. This prevents a child's `door_open` from selecting an unrelated
root animation. Interior controllers come from entity-class identity where
available (a `.soc` entity's `EntityClassGUID`, kept when the entity authors
inline geometry; the first class-spawned placement of a geometry supplies it
when the geometry also appears as raw `IncludedObjects`), otherwise an exact
DataCore geometry-path relationship; conflicting controller definitions are
left unresolved. Interior clips are keyed per geometry, so every placement of
that geometry (each a rig of its own) shares them. Both catalogue and export use this
discovery and naming rule. Decomposed GLB interiors preserve the NMC hierarchy
for the animated CGA category, leaving those vertices unbaked so transforms
apply once; static CGF interiors retain the flat-mesh path.

**Animation Clip Structure:**

Each animation clip object (`animation/serialise.rs`, `clip_to_json`) contains:

- `name` — animation identifier (e.g., `lg_deploy_l`, `lg_retract`, derived from .chrparams event name)
- `fps` — playback frame rate (integer, typically 30)
- `frame_count` — the last keyframe time across all channels, rounded up (not a count of samples)
- `bones` — map of bone keys to channel objects (see Bone Identification)
- `start_rotation` / `start_position` — optional DBA-metadata clip start pose
  (`[w, x, y, z]` and `[x, y, z]`, in the same Blender convention as the
  samples). The addon uses it as the anchor for
  `result = bind ⋅ (start⁻¹ ⋅ sample)`; CAF clips omit both and the addon
  anchors on the first sample.
- `fragments` — optional Mannequin ADB fragment metadata for clips reached from the owning entity's `SAnimationControllerParams` (`animation/mannequin.rs`): `fragment`, `guid`, `tags`, `frag_tags`, `blend_out_duration`, `option_weight`, `scopes`, `animations` (each with `name`, `blend` timings, and optional `flags` / `speed`), and optional `procedurals`

**Bone Channel Structure:**

Each bone channel contains:

- `rotation` — array of `[w, x, y, z]` quaternions (Blender wxyz convention after axis conversion) per keyframe
- `rotation_time` — array of source frame times parallel to `rotation`; if omitted, consumers should fall back to array indices
- `position` — array of `[x, y, z]` position vectors (Blender Z-up convention) per keyframe
- `position_time` — array of source frame times parallel to `position`; if omitted, consumers should fall back to array indices
- `has_rotation` / `has_position` — boolean flags indicating which channels are actually animated
- `source_skeleton_path` and, when the hash resolves to a skeleton node,
  `source_node_name` — the rig the channel was extracted from
- `blend_mode` — optional `"additive"` or `"override"`, from
  `classify_bone_blend_modes` (override when the bone's bind position lies
  outside the bounding box of its position samples). Bones without it default
  to additive.

**Bone Identification:**

Bone keys are always the bone's CRC32 hash as an uppercase hex string with a
`0x` prefix and no zero padding (`format!("0x{:X}")`, e.g. `"0xC1571A1A"`).
Names are never used as keys; `source_node_name` carries the resolved name.
When two channels in one clip share a hash, the key's value is an **array** of
channel objects instead of a single object; consumers must accept both shapes.

**Serialization Format:**

The inline `root_entity.animations` array in `scene.json` carries
lightweight **index records**: each clip with its `bones` map removed and a
`sidecar` field added (`split_clip_for_sidecar`). `sidecar` is a path
relative to the package directory (e.g. `animations/lg_deploy_l.json`). The
full clip, including `bones`, lives in that companion file at
`Packages/<package name>/animations/<sanitized-clip-name>.json`.

This split keeps `scene.json` small and lets the Blender addon load animation
keyframe data lazily — only when the user actually applies a clip. The sidecar
filename is derived from the clip name with characters outside
`[A-Za-z0-9_.-]` replaced by `_`; collisions are disambiguated with a numeric
suffix (`_2`, `_3`, …).

For shared skeletons or components, all clips are listed and the
Blender addon filters by context.

**Example:**

`scene.json` (index record only):

```json
{
  "name": "lg_deploy_l",
  "fps": 30,
  "frame_count": 120,
  "sidecar": "animations/lg_deploy_l.json",
  "fragments": [
    {
      "fragment": "Landing_Gear",
      "frag_tags": ["Deploy"],
      "tags": ["Landing_Gear"],
      "scopes": ["LandingGear", "LandingGearFront", "LandingGearLeft", "LandingGearRight"],
      "animations": [{"name": "landing_gear_extend", "flags": "ForceSkelUpdate"}]
    }
  ]
}
```

`Packages/<package name>/animations/lg_deploy_l.json` (sidecar body):

```json
{
  "name": "lg_deploy_l",
  "fps": 30,
  "frame_count": 120,
  "bones": {
    "0xC1571A1A": {
      "has_rotation": true,
      "has_position": true,
      "rotation": [
        [0.707, 0.0, 0.0, 0.707],
        [0.708, 0.0, 0.0, 0.705]
      ],
      "rotation_time": [0.0, 5.0],
      "position": [
        [0.0, 0.0, 0.0],
        [0.1, 0.0, -0.05]
      ],
      "position_time": [0.0, 5.0]
    }
  },
  "fragments": [
    {
      "fragment": "Landing_Gear",
      "frag_tags": ["Deploy"],
      "tags": ["Landing_Gear"],
      "scopes": ["LandingGear", "LandingGearFront", "LandingGearLeft", "LandingGearRight"],
      "animations": [{"name": "landing_gear_extend", "flags": "ForceSkelUpdate"}]
    }
  ]
}
```

**Import Modes (Blender Addon):**

The Blender addon provides four playback modes per animation:

- **None** — leave skeleton bones in bind pose, do not apply animation
- **Snap to First Frame** — apply rotation and position from keyframe 0 only
- **Snap to Last Frame** — apply rotation and position from the literal final keyframe, except source-tagged cyclic transition clips (for example Mannequin Open/Close clips whose first/final samples return to the same state) use the timed transition pose selected from the exported source samples
- **Insert as Action** — create a Blender Action with per-bone f-curve channels for full timeline playback

**Compatibility:**

- Sidecars generated without animations (older exports) simply omit the `animations` array; the Blender addon gracefully skips animation UI if absent.
- Future exports may extend this format with additional fields such as per-bone rotation/position masks, compression metadata, or event markers. Importers should safely ignore unknown fields.

## UI Bindings

Ship screen textures are rendered at export time by the `starbreaker-ui`
crate and written into the shared `Data/UI/Generated/` tree. Each
`children[]` entry and each interior placement (`interiors[].placements[]`)
carries a `ui_bindings` array that records the provenance and output location
of every resolved screen for that entity. The root entity carries none.

### `UiBinding` shape

The struct is `UiBinding` in `crates/starbreaker-3d/src/types.rs`; the JSON is
written by `ui_binding_json` in `decomposed.rs`. Every field below is always
present (optional values serialize as `null`). The struct's
`canvas_widget_canvas_path`, `canvas_widget_url_postfix`,
`canvas_widget_url_optional`, and `canvas_variable_binding` are internal and
not serialized.

| Field | Type | Description |
|---|---|---|
| `binding_kind` | string | Screen category: `"mfd"`, `"radar"`, or `"physical"` (annunciators, doors, gauges, …). |
| `source_entity_name` | string | Entity that contributed the binding. |
| `helper_name` | string or null | Named helper on the seat/dashboard that places this screen (e.g. `Screen_Annunciator_L`). The same name appears as the Blender mesh name. |
| `default_view` | string or null | The resolved default view name. |
| `default_state_is_off` | bool | `true` when the authored default view had no concrete content (null/zero or shell canvas). The exporter still renders an "on" image from the first concrete view; consumers can use the flag to start the screen off (the addon stores it on the object). |
| `default_state_name` | string or null | Authored default state name. |
| `default_light_color` | `[u8; 4]` or null | Authored default light colour (RGBA). |
| `default_light_intensity_milli` | u16 or null | Authored default light intensity × 1000. |
| `canvas_guid` | string or null | DataCore GUID of the BuildingBlocks canvas record (for MFDs, the frame canvas). |
| `canvas_record_name` / `canvas_record_path` | string or null | DataCore name / path of that canvas record. |
| `content_canvas_guid` | string or null | GUID of the per-helper content canvas resolved from the parent vehicle's `SCItemUIView_DashboardCanvasDef`. Rendering prefers it over `canvas_guid`, except for MFDs with a distinct frame canvas, which render the frame. |
| `content_canvas_record_name` | string or null | DataCore name of the content canvas record. |
| `dashboard_view_index` / `dashboard_screen_slot` | u32 or null | `View[]` index and `screens[]` slot in the dashboard canvas def that supplied `content_canvas_guid`. |
| `screen_name_loc_key` | string or null | Localization key for the MFD screen name from the resolved `SMFDView.name` (e.g. `@ui_MFD_View_TargetStatus`), injected into the frame canvas's `text_ScreenName` footer. `null` for non-MFD bindings. |
| `transit_location_loc_key` | string or null | Localization key of the transit location a lift-call console serves (nearest `SCTransitDestination`). It sets the console's floor-name heading and is part of the render identity. |
| `owner_source_file` | string or null | Host SWF path of the owning UI component. |
| `runtime_image_source` | string or null | Runtime image source, when authored. |
| `generated_image_path` | string or null | Export-root-relative path of the generated PNG (see below). `null` when rendering failed. |
| `generated_context_manifest_path` | string or null | Path of the `scene.json` that owns the binding. |
| `generated_resolved_source_path` | string or null | Source the render resolved: `runtime_image_source`, else the widget canvas path, else `canvas_record_path`, else `"unknown"`. |
| `generated_backend` | string or null | Rendering backend; always `"starbreaker-ui"` when an image was produced. |
| `generated_provenance` | string or null | A JSON object **encoded as a string**: `resolved_canvas_ids`, `resolved_canvas_names`, `selected_style_source`, `selected_swf_source`, `render_backend`, `fallback_counters`, `unresolved_references`, `confidence`. |
| `generated_confidence` | u8 or null | 0–100 score of how completely the binding resolved (content canvas, image source, record name, helper name). |
| `ui_screen_aspect_w_over_h` | f32 or null | Physical aspect (width / height) of the screen mesh's render-target faces (`ui_pipeline/screen_aspect.rs`). It drives the render proportions of physical and radar screens; `null` when the mesh can't be measured. |

The `generated_*` fields are filled only when the render succeeds; a failed
render logs a warning and leaves them `null`.

### Generated PNG layout

Generated PNGs are stored at (`generated_ui_binding_path`):

```
Data/UI/Generated/{ui_type}/{manufacturer}/{ship}/{asset}.png
```

- `ui_type` is `vehicle` when the root geometry path contains `/vehicles/`,
  otherwise `ship`.
- `manufacturer` is the root entity's manufacturer code (lowercase, e.g.
  `drak`), or `unknown`.
- `ship` is the cleaned root entity label with a leading manufacturer token
  removed (`DRAK_Clipper` → `Clipper`).
- `asset` is the first non-empty of `content_canvas_record_name`,
  `canvas_record_name`, `helper_name`, `default_view`, the widget URL postfix,
  `source_entity_name`, and `binding_kind`, lowercased with characters outside
  `[a-z0-9_-]` replaced by `_`. Transit screens append their floor
  (`transit_location_loc_key` without `@`) so per-floor renders don't collide.

Example: `Data/UI/Generated/ship/drak/Clipper/mesh_end_screen_plane.png`.

Paths are names, not content hashes. Each distinct render identity
(`UiRenderKey`: canvas GUIDs, kind, helper, dashboard view/slot, loc keys,
host SWF, screen aspect) is rendered once and reused across placements. When
two bindings map to the same path, the first file written wins.

Every export that produced generated UI files also writes
`Packages/<package>/.export_stamp.json` (see *Deterministic generated-UI
provenance* below), which the starbreaker-ui visual guard uses to reject stale
PNGs.

### Render pipeline

`crates/starbreaker-3d/src/ui_pipeline.rs` (`render_ui_binding_png`) bridges
the exporter to `starbreaker_ui::pipeline`:

1. **Inputs** — a `UiBindingView` built from the binding, DataCore fetchers
   for canvases and styles (`ui_pipeline/canvas_fetcher.rs`,
   `style_fetcher.rs`), P4K fetchers for SWFs, atlas assets and holograms, and
   per-export shared data (the localization map, plus a default-value registry
   built once per export from it and the ship-derived values in
   `ship_values.rs`).
2. **Target size** — `mfd` renders at 1600×900, `radar` at 1024×1024, and
   other kinds at the canvas's authored size (2048×1024 if none).
3. **Compile** — `compile_ir_for_binding_with` picks the canvas, resolves the
   BuildingBlocks widget tree (`CanvasWidgetTreeResolver`), and builds the
   canonical UI IR document (`ui_ir`): layout rects, resolved text and
   colours, asset references, and the selected style and SWF sources.
4. **Style** — `load_style_for_ir` resolves the manufacturer
   `BuildingBlocks_Style` record from DataCore (`ManufacturerStyleFetcher`);
   if none matches it falls back to the Drake amber defaults with a warning.
5. **Render** — `render_ui_ir_document` (`ir_compose`) rasterises the IR with
   `tiny-skia`; screens whose IR selects a SWF host render through
   `render_ui_ir_with_swf_overlay` (`hybrid_compose.rs`), which composites the
   SWF's static stage where a Flash host node has no BuildingBlocks content.
6. **Encode** — the image is PNG-encoded and written at the path above.

No CRT/tint post-process is applied to exported PNGs, and no AVM1/ActionScript
is executed at any stage; SWFs are read for static atoms only.

### Consumer

The Blender addon reads `scene.json` and, for each scene instance or interior
placement with a `ui_bindings` array, picks the binding whose `helper_name`
matches one of the object's source node names (falling back to the first
binding with an image). When `generated_image_path` is non-null, the addon
loads the PNG as the screen emission texture on the object's UI material
slot, and marks the object when `default_state_is_off` is set.

Bindings with a null `generated_image_path` are imported without a texture;
the mesh retains its base material colour (typically near-black for a dark
powered-off screen).

## Metadata-only animation catalogue

`entity_clip_catalogue(db, p4k, record, loadout_tree)` returns the serializable
`animation::catalogue::Catalogue` for the default entity export. It discovers
root, landing-gear and resolved loadout rigs without loading mesh/texture payloads.
It shares the exporter name matcher, merge helper and interior key/label helpers;
DBA/CAF inspection decodes first/final values only, retaining the motion score
used by matching. Time tables are read for the exported final-key frame count.

The catalogue's clip `id` joins `root_entity.animations[].name`. Named clips
shared by several rigs retain the first owner in export order; merged channel
counts and endpoint boomerang flags cover all contributing rigs. Optional
Mannequin fragment annotations use the same DataCore controller source. The
catalogue does not change the animation sidecar or scene index format. As above,
per-entity interior namespaces apply to rootless socpak packages, not ship
packages with a root rig (including Cutlass).

Catalogue review follow-up: the exporter and catalogue now consume the same
ordered rig-source list (including DataCore geometry fallback and exact gear
owner filenames) and clip merger. Rig-local failures warn and skip that source.
Chrparams event names containing `*` or `?` are mapping rules and are no longer
emitted as literal animation names; concrete DBA metadata entries remain in the
union. This can replace an old wildcard index record with a concrete source name.
The catalogue's additive `contributors: Vec<String>` lists contributing owners
and resolved rig node names. Existing `id`/`owner_entity` semantics remain intact;
this does not yet make merged ship door clips independently addressable.

### Blender entity-light consumption

The addon consumes `root_entity.lights`, every `children[].lights`, and
`interiors[].lights`. Older scenes with absent entity arrays default to empty
lists. Resolved entity lights attach to the exact helper in their own instance
using `parent_local_position`/`parent_local_rotation`; if the helper is missing,
they use the exported entity-local frame under that instance anchor. Interior
lights retain their container-local frame. The entity-local `direction_sc` is
not reused after switching to a helper-local rotation. Source IDs occur once
across the three locations, preventing duplicate light creation.

## Deterministic generated-UI provenance

When an export contains generated UI files, it writes
`Packages/<package>/.export_stamp.json` (schema version 2) in its own package:
ships exported to one root share `Data/`, so a stamp there would describe only
the last export. Exports written before this kept it at
`Data/UI/Generated/.export_stamp.json`; the visual guard still reads that as a
fallback. The stamp records `archive_build_id` from the adjacent install
`build_manifest.id` (`Data.BuildId`), or a SHA-256 of sorted archive-directory
entries when the manifest is absent; `exporter` (crate name/version and a
compile-time source SHA-256); `entity`; every serialized export `options` field;
and `generated_files` mapping generated paths to SHA-256 payload digests. Set-valued
options are sorted before serialization. Malformed build manifests are errors.

The source digest includes the workspace manifest and lockfile and the source
and build inputs (`Cargo.toml`, `build.rs`, `src/`, `build_support/`) of
`starbreaker-3d` and the workspace crates it depends on, transitively (normal
and build dependencies), with workspace-relative names. Other crates, and every
crate's tests and fixtures, are not exporter inputs: changing them changes
neither the digest nor triggers a rebuild of the exporter. It contains no checkout path, hostname,
Git working-directory state, wall clock or executable timestamp. Identical inputs
produce identical stamp bytes across independent exports and executable consumers.

The Blender `PackageBundle` reader loads scene/palette/livery/paint manifests and
does not consume this auxiliary file. UI visual regression accepts v2 by checking
exporter source identity and compared PNG content hashes; it retains the legacy
timestamp guard for v1 exports. No timestamp exceptions are needed in package
byte comparisons. This provenance file is not a signature for untrusted input.

## Player kits (WP-P2a)

`character::player::export_player` writes the GLB flavour of contract v2 with
`assembly_kind: "character"`. The default CHF of the requested gender supplies
identity and the body; the loadout XML supplies equipment. Visible skinned parts
carry the complete bind rig, including socket-only joints, and follow the root
rig by joint name. A visible skinned part becomes the root mesh when clothing
hides the head. No animation is baked into the bind pose. The CHF supplies head identity; the
loadout head subtree is excluded from the clothing pass to avoid duplicate heads.

`children[].port` (and the root part's `port`) describes its authored item port:
`{name, owner_entity, flags: string[], types: [{Type, SubTypes}], attachment:
"bone"|"skin", attach_bone, offset: {position: [x,y,z], rotation_degrees:
[x,y,z]}, item_offset_helper, select_tag}`. Offsets use native SC axes, metres and
Euler degrees. Skin attachments follow the leader pose, not the helper transform.
For bone attachments, align the item's named offset helper with the body joint
plus the port offset. Port flags, including `invis_p1` and `HeadWear`, are retained.
Logical items without geometry remain metadata-only children.

A personal weapon child nests its existing `fps_weapon` assembly: root asset
fields, `weapon_assembly`, `children`, and `animations`. Its `package_dir` resolves
its own animation sidecars; those are NOT body clips. Nested assets use `Data/PlayerItems/<child_id>/...` and nested manifests use
`Packages/PlayerItems/<child_id>/...`, all export-root-relative. This preserves
independent material/livery views of repeated items. References in their JSON
sidecars are rebased; original `geometry_path` and clip source paths stay intact.
Each nested provenance stamp lives under its own `package_dir`. `id` is instance-specific, even for duplicate items.

The additive top-level `player` object is:

```text
{
  gender, loadout, skeleton, camera_bone, root_motion_bone,
  hand_ports: [port],
  weapons: [{entity, child_id, stow_port: port, mannequin_tags: [string],
             grip_data: SGripContainer, size, subtype}],
  motion: {unavailable_tag_definitions: [string],
    states: [{weapon_child_id: string|null, stance, move_speed_tag: string|null, first_person, tags,
    fragments: [{fragment, guid, source, tags, frag_tags, query_frag_tags, scopes: [string],
      layers: [{clip OR blend_space, layer, additive: bool|null, missing?: true, loop, speed,
                blend_in, blend_out, exit_time, start_time, weight, joint_mask}]}]}]},
  blend_spaces: [{id, source, dimensions: [{name, min, max, authored_min,
    authored_max, cells}], examples: [{clip, alias, params, authored_params,
    playback_scale, direct_delta_motion, component?, missing?: true}], faces: [[example_index, ...]],
    components?: [id], selectors?: [{name, choose_blend_space, locked}]}],
  diagnostics: [{kind: "missing_clip", clip, fragment, tags, frag_tags, gender}]
}
```

The camera binding is the `HeadCam` literal in StarCitizen.exe; the Player entity's
DataCore audio-listener bones corroborate the joint but do not define a camera
binding. The exporter keeps this in one named constant and validates the joint.
Root motion uses the skeleton's parentless joint, not a copied bone name.

Body animation sidecars retain the existing `bones`, CRC keys, time arrays and
Blender-axis sample convention. New fields are `additive`, `cyclic`, `asset_flags`,
`duration` (seconds), `source_skeleton_path`, and `root_motion: {bone, space:
"sc_local", start, end}`, where each endpoint is `{position: [x,y,z], rotation:
[w,x,y,z]}` in native SC local axes/metres. Asset flags come from IvoAnimInfo (CAF)
or the embedded AnimInfo in DBA metadata: bit 0 additive, bit 1 cyclic. Controller
compression flags and clip filename suffixes are not additive signals.

The Mannequin resolver selects the first ordered fragment whose global and
fragment tags are subsets of the query, following ADB sub-database routing.
Fragment names come from imported FragmentDefs; locomotion/selection categories
come from their fragment-tag group schemas. Each resolved layer retains its source
layer index and sequencing times. Empty animation entries are omitted. A zero
speed or weight remains zero. `loop` is the Mannequin request; `cyclic` is the asset
flag, and the two are distinct.

Blend-space examples keep authored SetPara values; absent parameters come from
root displacement/duration and direction in the initial root frame, including
PlaybackScale for speed. Dimensions expose both authored grid bounds and bounds
expanded to include the actual samples. LIVE data has samples outside its grid
(e.g. ±pi samples with ±3.141 bounds); clamping them would corrupt interpolation.

Default selection is alphabetical among DefaultLoadouts that contain a personal
weapon on a bone stow port. In LIVE 4.10.2 this selects `Brightsky.xml`, not the P4-AR
PVE loadout named in the brief. Request the PVE loadout explicitly to verify that
weapon; no entity/loadout-name special case selects it.

All actor queries, unarmed and per weapon, include the untagged speed state and
every authored `moveSpeed` tag variant. Body locomotion
fragments must have their own motion schema and animate the actor root scope;
fragments requiring an external item's animation scope are excluded. Stow queries
use the port selection tag independently of the carry locomotion base. `scopes`
retains the controller's resolved scope binding, including first-person overrides.
Missing imports for unrelated tag definitions are reported in
`motion.unavailable_tag_definitions`; missing registered body clips are diagnosed as described below. Malformed assets
and decode errors remain fatal.
Combined `.comb` spaces retain component topology, component IDs and selector
dimensions; consumers must evaluate selectors before interpolating each component.

Missing clips do not alter fragment resolution. A selected layer retains its
authored `clip` and gets `missing: true`, `additive: null`; it has no sidecar or
root animation index entry. Each affected fragment contributes a deduplicated
`player.diagnostics` entry with its authored tags, fragment tags and gender.
For blend spaces, a missing example is marked `missing: true`; its unavailable
root-derived parameters remain null, and layers referencing that space are marked
missing. Authored sample parameters, faces and all fragment timing are retained.
Consumers must not attempt to play missing layers.

Source resolution follows recursive chrparams `$Include`, `#filepath` wildcard
chains and every registered `$TracksDatabase`, without filtering DBA clip names
by gender directory. This allows authored cross-rig source sharing. Inspection of
LIVE 4.10.2 female chrparams found no includes or male search roots; its registered
female DBAs contain no male_v7 entries. The shared selection ADB references
`pistol_alerted_stand_select_01_fp_add`, but only the male loose CAF exists and the
female remap list supplies no mapping. It is therefore reported as missing,
without borrowing a male clip or substituting another fragment. The verifier
prints this sidearm diagnostic; missing clips in required stand FP/3P unarmed or
stocked-rifle idle, movement, select or deselect states fail verification.

The motion base is shared actor state, inferred once across matching weapon-tagged
options in all participating fragment schemas with a `MotionBase` group. That
same base is included in every participating fragment query; a movement fragment
need not repeat a weapon tag to inherit the association authored by idle. Stow
selection remains independent. If the current stance has no weapon/base
association, the association lookup considers other authored stance tags while
retaining the weapon, speed, first-person and skeleton context. Actual fragment
resolution continues to use only the requested stance. This is needed because
LIVE rifle/base associations are authored for stand but reused by crouch.

No locomotion base name is hard-coded. The inspected weapon DataCore grip motion
bases describe carry poses, and its enslavement ADB has no motion-base mapping;
the shared player motion ADB supplies the locomotion association. The schema is
unchanged. Consumers should select `move_speed_tag` alongside stance and view mode.
The verifier requires every armed standing state to have looping movement queried
with the same base as its idle and validates all clip/blend-space references.
