# Decomposed Export Contract

Phase 3 Mode 2 export now writes a reusable shared-root package at a caller-selected export directory.

Within that export root:

- `Packages/<package name>/scene.json` describes the root entity, child attachments, interior placements, light definitions, and shared asset references.
- `Packages/<package name>/palettes.json` contains shared palette identities that scene instances reference by `palette_id`.
- `Packages/<package name>/liveries.json` groups scene and material usage by shared palette identity.
- `Data/...` contains reusable mesh `.glb` assets, material sidecars, and exported textures using canonical P4k-style paths rooted at `Data/`.
- exporting another ship to the same root reuses matching `Data/...` assets instead of duplicating category-specific copies.

## Scene Manifest

`scene.json` includes:

- the root package rule: all asset paths are relative to the selected export root
- the package directory path under `Packages/<package name>`
- root entity metadata and asset references
- child attachment relationships via `parent_entity_name`, `parent_node_name`, `offset_position`, `offset_rotation`, and `no_rotation`
- interior container transforms, placement records, and exported light data
- material sidecar and palette references for every scene instance

## Light Records

Each entry in a scene's `lights` list carries:

- `name`, `light_type` (`Omni`, `SoftOmni`, `Projector`, `Ambient`),
  `position`, `rotation` (CryEngine-space; the Blender addon applies
  the axis conversion and the spot-axis basis correction)
- `color` (linear RGB), `intensity` (candela), `radius`,
  `inner_angle` / `outer_angle` for projectors
- `temperature` (Kelvin) + `use_temperature` flag so Cycles can match
  the in-engine blackbody colour
- `projector_texture` (package-root-relative DDS path) for light
  cookies / gobos
- `active_state` and a `states` map capturing every authored
  CryEngine state (`offState`, `defaultState`, `auxiliaryState`,
  `emergencyState`, `cinematicState`). The flat `color` / `intensity`
  / `temperature` fields are copied from the first non-zero state in
  priority order `default → auxiliary → emergency → cinematic`; the
  full map lets the Blender addon switch between states at runtime
  without re-exporting. See `docs/StarBreaker/lights-research.md` for
  the full schema.

### Planned Additive Cross-DCC Light Semantics

The current light contract is enough for the Blender addon, but it still
requires importer-side Star Citizen interpretation. The planned Rust-sidecar
migration adds semantic fields rather than replacing the raw ones above.

Recommended additive light fields:

- `semantic_light_kind` — normalized reusable kind such as `point`, `spot`,
  `area`, `sun`, or `ambient_proxy`, while preserving the raw source
  `light_type` string.
- `transform_basis` — explicit basis label for exported transform fields,
  for example `cryengine_z_up`.
- `direction_sc` — normalized SC-space forward direction for projector /
  spotlight records so consumers do not have to reverse-engineer a spotlight
  axis from the raw source quaternion.
- `outer_angle_full_deg` / `inner_angle_full_deg` — explicit full-angle names
  to remove half-angle ambiguity.
- `intensity_raw` on the flattened active record and every exported state
  snapshot, plus `intensity_unit` to state that the value is still the
  authored CryEngine intensity scalar.
- `intensity_candela_proxy` — explicit label for the exporter's current
  candela-style scaled value, emitted on the flattened active record and on
  each exported light state alongside the legacy `intensity_cd` alias.
- `radius_m` — explicit metric label for attenuation distance.
- `projector_texture_format_hint` — document that projector textures may be
  DDS / block-compressed / HDR assets and should not be assumed to have a PNG
  fallback.

These semantic fields are intentionally DCC-agnostic. They must not encode
Blender-only decisions such as final Watt conversion, Blender spotlight-axis
corrections, or Blender gobo node-graph details.

### Planned Additive Cross-DCC Transform Semantics

The current contract preserves the raw transform inputs, but downstream
importers still have to repeat Star Citizen-specific attachment resolution.
The planned additive transform fields are:

- `local_transform_sc` — fully resolved SC-space local matrix relative to the
  exported parent.
- `world_transform_sc` — optional fully resolved SC-space world matrix for
  consumers that prefer direct placement over parent-relative reconstruction.
- `source_transform_basis` — explicit basis label for those matrices.
- `resolved_no_rotation` — note that `no_rotation` helper semantics have
  already been baked into the emitted transform.

Current incremental rollout note:

- child scene instances now emit `source_transform_basis = cryengine_z_up`
  plus `local_transform_sc` for the common child path and for `no_rotation`
  records whose parent-relative local matrix has been resolved by the exporter.
- `resolved_no_rotation = true` means the exporter has already baked the
  legacy helper-suppression rule into `local_transform_sc`; importers should
  consume that matrix directly rather than re-implementing the rule.
- raw `offset_position`, `offset_rotation`, and `no_rotation` remain in the
  sidecar for debugging and compatibility with older importers.

The exporter should own Star Citizen-specific transform composition, helper
resolution, and duplicate-offset suppression. Importers should own only the
final mapping from SC basis into their host DCC's coordinate system.

### Compatibility Rules

The Rust migration is additive and must be backward-compatible while Blender
and any external consumers migrate.

- Older sidecars that lack the new semantic fields remain valid; importers
  fall back to the existing raw `light_type`, `position`, `rotation`,
  `intensity`, `radius`, and per-state payloads.
- Newer sidecars should continue emitting the raw fields alongside the new
  semantic fields until every known consumer has migrated.
- Blender-specific behavior such as final Watt conversion, gobo UV wiring,
  per-image gobo compensation, and shadow soft-size heuristics stays in the
  importer even after the semantic fields exist.

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

Each `*.materials.json` sidecar preserves:

- source material path and geometry path
- per-submaterial name, raw shader string, shader family classification if known, and activation state
- decoded feature flags from `StringGenMask`
- direct texture-slot inventory with semantic roles, virtual-input flags, source paths, and exported texture paths
- DDNA identity markers on exported normal-gloss source PNGs plus `alpha_semantic` markers such as `smoothness` when the source texture alpha carries shader-relevant data
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

The current sidecar contract is now substantially closer to the raw `.mtl` XML surface, but it is still intentionally split into two layers:

- curated semantic fields meant for Blender reconstruction and stable downstream use
- authored XML-derived fields kept for inspection, debugging, and future reconstruction upgrades

### Texture Export Rules

- Decomposed exports now write source textures as `.png` using the original `Data/...` filename with only the extension changed.
- Rust no longer emits derived `.roughness.png` exports for DDNA textures in the decomposed material contract.
- DDNA normal-gloss exports preserve smoothness in the PNG alpha channel so Blender shader groups can derive roughness with node logic instead of relying on Rust-side image reinterpretation.
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