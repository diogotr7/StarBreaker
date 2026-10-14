# Character pipeline: `.chf` + `Data.p4k` → GLB / `.blend` package

How a Star Citizen character file (`.chf`) becomes a rigged, optionally
animated character. Everything lives in `crates/starbreaker-3d/src/character/`
(module docs on every file) on top of `crates/starbreaker-dna` (`.dna`
readers) and `crates/starbreaker-chf` (CHF parser).

Companion documents (this directory):
- `SPEC.md` — RigLogic, head `.skin`/`.skinm` and `.dna` byte layouts.
- `PHASE_4B_INVESTIGATION.md` — the engine evidence for the DNA math (cited
  below as §N).
- `SCDR_ANALYSIS.md` — ground-truth checks, head items, wrinkle maps.

Supported rigs: the human male and female `t1_pu` heads.

## Flow

```text
.chf ─▶ starbreaker_chf::ChfFile::parse ─▶ ChfData { dna, materials, itemport }
                                               │
  HeadAssets::load(gender_hash) ◀──────────────┘   (loaded once per rig)
     CDF, head .skin/.skinm, master .dna, RigLogic,
     eyes/teeth/eye-details sub-meshes, PartSources
                                               │
  DNA matrix ─▶ HeadAssets::rig_state / expression_rig_state ─▶ CharacterRig
                                               │       (+ HeadAssets::character_rig:
                                               │        outfit, body clip)
                         ┌─────────────────────┴─────────────────────┐
              CharacterRig::to_glb                  package::export_character_package
              (shared glTF writer)                  (decomposed package, .blend via
                                                     pipeline::blend_assembly)
```

## 1. Parse and resolve the rig

- **CHF.** `starbreaker_chf` gives the DNA (`gender_hash`, `variant_hash`,
  `face_parts: FacePart → [DnaBlend { head_id, value }; 4]`), the material
  overrides and the item-port tree.
- **Rig identity** (`identity`): `gender_hash` alone selects the rig
  (`crc32c("protos_human_<gender>_face_t1_pu")`); `variant_hash` is
  per-customization and not used. The `.dna` is rig-global (§6.16.1).
- **`HeadAssets::load`** (`identity_context`) reads everything per rig once:
  the CDF (`cdf`), the head `.skin` + `.skinm` (`head_skin`: mesh, 198-bone
  bind skeleton, up to 8 skin influences, embedded RigLogic), the master
  `.dna`, the RigLogic (`crate::riglogic`), the CDF sub-meshes co-located with
  the head (eyes and teeth with their own `.dna`, `ProtosMisc` deformer; eye
  details, `WD_Elastic`), and each part as the entity pipeline loads it
  (`part_source::PartSource`: all vertex streams, submesh material ids,
  `.mtl`).

## 2. DNA matrix (§3)

`dna_matrix::{chf_weights, top_k, accumulator}`: each CHF blend adds
`value / 65535` to cell `(head_id, FacePart)` of a `[num_variants × 13]`
buffer; each face-part column is normalised, pruned to its 4 strongest heads
and re-normalised; the result is a `DnaMatrix2D`. Every per-character step
below is driven by this matrix.

## 3. Neutral identity (`HeadAssets::rig_state`)

- **Head shape** — `.dna` slot 18 (§6.16.24):
  `morphing::compute_neutral_vertex_deltas` folds the matrix into the
  per-(variant × region) neutral vertex deltas, added to the base head mesh
  as authored (no left/right symmetrising). Normals are carried onto the new
  shape (`normals::transfer_normals`).
- **Skeleton** — `.dna` slot 15 (§6.16.6):
  `morphing::compute_neutral_joint_offsets` (variant-0-relative, folded by
  slot 21) is applied as a re-bind (`apply::rebind_skeleton`): the joints
  move, the rest mesh does not.
- **Sub-meshes** (`pose_submeshes`): eyes and teeth add their own `.dna`'s
  slot-18 deltas and re-bind to the re-bound skeleton (`submesh`); eye details
  and the CHF head items are surface-wrapped to the slot-18 head shape
  (`wrap::HeadDeltaField`, inverse-square interpolation of the head deltas).
- **Head items** (`items`): hair, brows, lashes, beard and scalp from the
  CHF item ports, picked structurally (gendered sub-geometry with a `WD_*`
  wrap deformer); their materials follow the tag context (`materialVariants`).

## 4. Pose, blendshapes and the face clip

- **Controls → activations**: `controls::ControlInputs` (rest pose =
  `CTL_PSD` only, §6.11.2), Section C correctives
  (`riglogic::compose_correctives`).
- **Joint deltas** (slot 31 × slot 21 × matrix × activations, §6.9.4,
  §6.10): `morphing::compute_joint_deltas`, applied by `apply::apply_deltas`
  (translation added, Euler XYZ rotation multiplied in, §6.9.5). Zero at rest.
- **Blendshapes**: the head `.skinm` `BShapesGpu` chunk
  (`blendshapes`, §6.13) attenuated by slot 17 (§6.14), written as morph
  targets `bs_NNN`.
- **Face clip** (`HeadAssets::expression_rig_state`, `animation_export`):
  a `.caf` decoded by `face_anim::CafClip` (§6.16.15) is sampled per frame
  through the same pose pipeline into bone, morph-weight and wrinkle tracks
  (`wrinkles`: RigLogic Section G/H → HumanSkin_V2 `PI_WrinklesMask`). Its
  blendshapes are the character's own slot-19 archetype blend
  (`expression`, §6.16.19), pruned to those the clip uses. The result is one
  `crate::clip::Clip`.

## 5. Body, outfit and body clip (`HeadAssets::character_rig`, `clips`)

- **Body** (`body`): the item on the CHF's root port (its gender sub-geometry
  `.skin`, material from the CHF `BodyMaterial` slot) and the player actor
  skeleton (`bhm_skeleton_v7.chr` / `bhf_skeleton_v2.chr`). The rig is built
  on the actor skeleton, with the head rig's facial bones overlaid by name.
- **Outfit** (`outfit`, `loadout`, `clothing`): a loadout's garments (each
  item's gendered `.skin`, `loadout::worn_geometry`). Clothing layers hide
  the body zones a higher layer covers (`clothing::hidden_zones`, by
  `SCItemClothingParams` chunks and the `.skinm` zone words), and
  `HiddenParts` ports drop what is worn under them (a full helmet hides the
  CHF's hair and facial hair). `CharacterRig::dress` adds the body and
  garments as parts.
- **Picked pieces** (`wear`): items given by name go on the free port that
  accepts them (their `AttachDef.Type` and subtype, and the port's
  `RequiredPortTags` on the item), searching the body's ports and those of
  items already worn, producing the same `LoadoutEntry` tree as a loadout.
  A piece the wearer's gender has no mesh for is reported, not worn.
- **Body clip**: `clips::load_body_clip` (a `.caf`, or one clip of a `.dba`
  picked by name), merged with a face clip into one clip
  (`CharacterRig::add_body_clip`).

## 6. Writers

`shared::CharacterRig` is one character in the shared export model: the head
mesh (its `PartSource` reshaped to the character), the rest and posed
skeletons, optional blendshapes, the parts (CDF sub-meshes, head items,
body, garments — all at rest, skinned by joint name; the pose and clips
reach them through their skins) and animation clips.

- **GLB**: `CharacterRig::to_glb` writes it with the shared glTF writer
  (`crate::gltf::write_glb`, `.mtl` materials as colours, no textures).
  Wrinkle tracks go into the animation extras
  (`starbreaker_material_params`).
- **Package**: `package::export_character_package` writes the same rig as a
  decomposed package (`Packages/<name>/scene.json` + `.blend` assets via
  `pipeline::write_decomposed_export_blend` / `blend_assembly`), with the
  CHF-selected head `.mtl`, the resolved CHF material customization
  (`customization`: skin tone, blemishes, makeup, eye and hair colours,
  override textures as PNGs) on the root instance, and clips as Blender
  actions. The Blender addon builds HumanSkin_V2 and HairPBR from it. The
  `.blend` does not carry the wrinkle tracks yet (parity gap,
  `docs/superpowers/plans/2026-10-14-glb-blend-parity.md`).

## Entry points

| Entry | Calls |
|---|---|
| `starbreaker chf to-glb <chf> [out.glb] [--loadout …] [--face-anim … --face-anim-intensity …] [--body-anim … --body-anim-clip …]` | `character::chf_to_character_glb` |
| `starbreaker chf to-package <chf> <export-root> [--texture-mip N]` + the same flags | `package::export_character_package` |
| `starbreaker armour export <out> --wear … [--loadout …] [--gender male\|female \| --chf …] --kind glb\|package` | `wear::dress_entries` on the given CHF or the game's default character (`default_wearer_chf`), then the two calls above |
| App character builder (`app/src-tauri/src/character_commands.rs`) | `HeadIdentityContext` (a per-gender cached `HeadAssets`): `build_glb` (live head preview without morph targets; GLB export with them), `build_expression_glb` |

Every CHF entry point exports the whole character (head, sub-meshes, head
items and body on the actor skeleton). Single armour pieces export outside
this pipeline, via `starbreaker entity export <item> --geometry-tag Male|Female`.

## Open

- A helmet hiding `Head_ItemPort` hides the CHF's head items but not the DNA
  head itself, which is the rig's root mesh in both writers.
- Clothing `VisibilityConditions` are not evaluated.
- Hair physics bones (`hh_*`) are not simulated.
- Rigs other than human `t1_pu` are not resolved.
