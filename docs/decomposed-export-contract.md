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
- child attachment relationships via `parent_entity_name`, `parent_node_name`, `offset_position`, `offset_rotation`, `no_rotation`, and `port_flags`
- interior container transforms, optional child-attachment parents
  (`parent_entity_name`, `parent_node_name`), placement records, and exported
  light data
- material sidecar and palette references for every scene instance
- optional decal-host hints for scene instances that carry projected decal
  geometry:
  - `decal_host_channel` is a palette channel hint such as `primary`,
    `secondary`, `tertiary`, or `glass`
  - `decal_host_rgb` is a fixed linear RGB host-colour fallback when no
    palette channel is available
  - these fields are advisory. Non-FPS importers may use them to rebind
    MeshDecal `Host Tint` directly. FPS weapon importers may additionally fall
    back to local spatial host-material matching for control-only POM overlays
    when these hints are absent.
- optional additive `controls.engine_glow` metadata for engine-emission tuning in importers:
  - absolute emission-strength range metadata (`min_strength`, `max_strength`, `default_strength`)
  - `targets[]` keyed by `mesh_asset` + `material_sidecar` + `source_material_index`
  - `geometry_path` is retained as source evidence for the target entity
  - target records are emitted only from DataCore `Category = Thruster` child entities
  - informative target labels (`submaterial_name`, `blender_material_name`)

`port_flags` is the raw source `SItemPortDef.Flags` string for the item port that attached a child. Importers can use this to preserve source visibility semantics; for example, Blender hides attachments by default when the source port includes `invisible` while keeping the objects present for inspection.

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
- screen-family `screen_effects` metadata; `apply_crt = true` is emitted when
  authored DisplayScreen/UIPlane data exposes a `screen_pixel_layout` slot, with
  the source slot/path and authored `PixelGridTilingX/Y` values when present
- DisplayScreen/UIPlane render-to-texture slots may carry additive `generated_ui`
  metadata when the exporter can provide default visible content:
  - `export_kind = "generated_ui_still"` when the slot is bound to a
    deterministic source-derived PNG for runtime-only RTT content
  - `generated_ui.source_path`, `generated_ui.identity`,
    `generated_ui.frame_selection`, and `generated_ui.provenance` explain how
    the exported `export_path` was selected
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

Each exported scene entity carries an optional `animations` array containing all discoverable animations for that entity and its components. Animations are discovered by:

1. Locating the skeleton's `.chrparams` CryXmlB file (derived by swapping extension from `.chr`)
2. Parsing the animation map and `$TracksDatabase` reference (usually a `.dba` file)
3. Loading the tracks database and exporting **all** animation clips

**Animation Clip Structure:**

Each animation clip object contains:

- `name` — animation identifier (e.g., `lg_deploy_l`, `lg_retract`, derived from .chrparams event name)
- `fps` — playback frame rate (typically 30)
- `frame_count` — total keyframe count
- `bones` — map of bone identifiers (by name or CRC32 hash if available) to channel objects
- `fragments` — optional Mannequin ADB fragment metadata for clips reached from the entity's `SAnimationControllerParams` (`fragment`, `tags`, `frag_tags`, `scopes`, `animations`, blend timings, speeds, flags, and procedural params)

**Bone Channel Structure:**

Each bone channel contains:

- `rotation` — array of `[w, x, y, z]` quaternions (Blender wxyz convention after axis conversion) per keyframe
- `rotation_time` — array of source frame times parallel to `rotation`; if omitted, consumers should fall back to array indices
- `position` — array of `[x, y, z]` position vectors (Blender Z-up convention) per keyframe
- `position_time` — array of source frame times parallel to `position`; if omitted, consumers should fall back to array indices
- `has_rotation` / `has_position` — boolean flags indicating which channels are actually animated

**Bone Identification:**

Bone references use the following priority:

1. If the source skeleton provides CRC32 hashes (preferred), use `"bone_hash"` as the key (u32 hex string, e.g., `"0xC1571A1A"`)
2. Otherwise use bone name string (e.g., `"BONE_Back_Right_Foot_Main"`)

**Serialization Format:**

Animations are stored per-entity. The inline `animations` array in
`scene.json` carries lightweight **index records** only (`name`, `fps`,
`frame_count`, `fragments`, and a `sidecar` field giving the relative
path to the heavy per-clip JSON file). The full clip body (`bones`,
`rotation`, `rotation_time`, `position`, `position_time`) lives in a
companion file at `Packages/<entity>/animations/<sanitized-clip-name>.json`.

This split (introduced in Phase 35) keeps `scene.json` small and lets
the Blender addon load animation keyframe data lazily — only when the
user actually applies a clip. The sidecar filename is derived from the
clip name with characters outside `[A-Za-z0-9_.-]` replaced by `_`;
collisions are disambiguated with a numeric suffix.

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

`Packages/<entity>/animations/lg_deploy_l.json` (sidecar body):

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

Ship screen textures are generated at export time and written into the shared
`Data/UI/Generated/` tree. Each `child` object in `scene.json` may contain a
`ui_bindings` array that records the provenance and output location of every
resolved screen for that child entity.

### `UiBinding` shape

Defined in `crates/starbreaker-3d/src/types.rs`. Key fields:

| Field | Type | Description |
|---|---|---|
| `binding_kind` | `String` | Screen category: `"mfd"`, `"radar"`, `"physical"` (annunciator/door/etc.) |
| `helper_name` | `Option<String>` | Named child on the seat/dashboard that places this screen (e.g. `Screen_Annunciator_L`). Same name appears as the Blender mesh name. |
| `canvas_guid` | `Option<String>` | DataCore GUID of the BuildingBlocks canvas root record. |
| `content_canvas_guid` | `Option<String>` | DataCore GUID of the per-helper content canvas resolved from the parent vehicle's `SCItemUIView_DashboardCanvasDef`. Takes precedence over `canvas_guid` during rendering. |
| `screen_name_loc_key` | `Option<String>` | Localization key for the MFD screen name from the resolved `SMFDView.name` (e.g. `@ui_MFD_View_TargetStatus`). Injected into the frame canvas's `text_ScreenName` footer node during rendering. `null` for non-MFD bindings. |
| `generated_image_path` | `Option<String>` | Relative path from the export root to the generated PNG (e.g. `Data/UI/Generated/02ea771f3d96bf1e_TEX0.png`). `null` when rendering failed or was skipped (e.g. null canvas GUID). |
| `generated_backend` | `Option<String>` | Rendering backend that produced the PNG (always `"starbreaker-ui/compose"`). |
| `generated_provenance` | `Option<String>` | Freeform note explaining how the image was produced or why it was skipped. |

### Content-addressed PNG layout

Generated PNGs are stored at:

```
Data/UI/Generated/{hash16}_TEX{mip}.png
```

where `hash16` is the first 16 hex digits of the SHA-256 of the rendered RGBA
buffer and `mip` is the texture mip level (`0` for full resolution). This path
scheme is implemented in `crates/starbreaker-3d/src/ui_pipeline.rs`
(`generate_image_path`).

Content addressing means that two screen bindings that resolve to identical
rendered output (e.g. the same canvas at the same mip) share one PNG on disk.

### Composer pipeline

For each `UiBinding` with a non-null canvas GUID:

1. **Canvas record** — fetched from DataCore (`DatacoreCanvasFetcher`).
   `content_canvas_guid` is preferred over `canvas_guid`.
2. **Widget tree resolution** — `CanvasWidgetTreeResolver` recursively
   resolves sub-canvas references to produce a flat `ResolvedCanvas`.
3. **SWF static atoms** — any SWF paths referenced in the widget tree are
   loaded from the P4K (`P4kSwfFetcher`). A minimal empty library is used
   when no SWF path is present.
4. **Manufacturer style** — `ManufacturerStyleFetcher` maps the binding's
   manufacturer identifier to a `ManufacturerStyle` (tint colour, backlight,
   CRT parameters). Falls back to Drake amber with a log warning for
   unrecognised manufacturers.
5. **Rasterisation** — `render_canvas_with_postprocess` from
   `starbreaker_ui::compose` draws the widget tree with `tiny-skia`, then
   applies the manufacturer CRT pass (tint, scanlines, vignette).
6. **PNG encode + write** — the RGBA buffer is SHA-256 hashed to derive the
   output path. The file is written only if not already present (deduplication).
   The `generated_image_path` field is set on the binding.

Canvas record → widget tree → static atoms → tint/CRT post-process → PNG is
the complete rendering path. No AVM1/ActionScript is executed at any stage.

### Consumer

The Blender addon reads `scene.json` and, for each child entity with a
`ui_bindings` array, maps each binding's `helper_name` to the corresponding
Blender mesh object. When `generated_image_path` is non-null, the addon loads
the PNG as a screen emission texture applied to the mesh's UI material slot.

Bindings with a null `generated_image_path` are imported without a texture;
the mesh retains its base material colour (typically near-black for a dark
powered-off screen).
