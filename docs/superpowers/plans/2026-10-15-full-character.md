# Full character: body + head + items on one skeleton, with body animation

Goal: a CHF character exports as a whole person — body, DNA head, eyes/teeth,
hair/brows/lashes/beard/scalp — on one skeleton, through the shared model, so
the GLB and the `.blend` package carry the same rig and the same clips
(face and body). Parity is enforced by `tests/parity.rs` (plan
`2026-10-14-glb-blend-parity.md`, phase F): the character fixture grows with
every step.

## Findings (2026-10-15)

- **The CHF root port is the body.** `masculinedefault`'s item-port tree:
  `body_itemport → body_01_noMagicPocket` with `head_itemport → PU_Protos_Head`
  under it, and the head items (`eyes`, `hair`, `eyebrow`, `eyelashes`,
  `beard`, `universal_scalp`) under the head. No armour: that comes from
  loadouts.
- **Body geometry is tag-selected**, like the head items: the body entity's
  `SGeometryResourceParams` sub-geometry branches by gender tag (`Male` →
  `objects/characters/human/male_v7/body/m_body.skin`, `Female` → `f_body.skin`)
  and then by skin-tone tag (`body_cau`, `body_blk`, `body_brwn`, …) choosing
  the `.mtl`. `items::resolve_head_items` already resolves the same kind of
  tags (`published_tags`, `material_node(..).resolve(context)`); the CHF's
  resolved customization carries the body material (`BodyMaterial`).
- **Skeleton:** `m_body.skin` is skinned to `bhm_skeleton_v7.chr` (220 bones,
  root `World`); the body `.skin`'s own bone table has 84. The head rig
  (`…_head.skin`, 198 bones, root `World`) shares 15 bones with the body
  skeleton, **with identical bind poses** (0.0 m), and adds 183 facial bones.
  The head geometry is already in character model space (head at 1.54–1.85 m).
  ⇒ the merged skeleton is body skeleton ∪ head rig ∪ part-only bones, by name
  (`shared::append_missing_bones`, already used for the hair braids).
- **Clips:** `animation::AnimationClip` (from `.dba`/`.caf`, see
  `animation/`) keys channels by `bone_name_hash` = CRC32 of the exact bone
  name; values are bone-local rotation (CryEngine `xyzw`) and position, times
  in frames at the clip's `fps`. They map directly onto `clip::Clip` bone
  tracks (`[w, x, y, z]`, seconds). Ship clips need the addon's anchoring
  logic (parity plan E2); character clips are bone-local samples.

## Phases (sequential; each ends green + committed + checked in Blender)

- **G1. Body in the rig.** Resolve the body item from the CHF root port
  (gender tag + skin-tone/customization material), load it through the same
  part loader as the head parts (pipeline mesh + `.mtl`, skin by joint name),
  and make the body skeleton the rig's base: `CharacterRig::bones()` = body
  skeleton (rest = bind) + head rig (with its DNA re-bind and control pose on
  the facial joints) + part-only bones. Both writers then carry the body
  for free (child part bound by name; scene.blend armature from
  `root_bones`). Tests: merge keeps shared binds and parents facial bones
  under `Head`; p4k: the character GLB and package include the body, skinned
  to `Hips`…`Head`; the neck seam stays closed (head neck-ring vertices vs
  body neck vertices); parity fixture green. **Depends on** the
  "character GLB parts from the pipeline loader" change (it reshapes
  `CharacterRig`'s part representation).
  G1 ✅: `character::body` — `resolve_body` (root port's item, gender-only
  sub-geometry node, the `.mtl` of the CHF slot customizing that port =
  `BodyMaterial`), `actor_skeleton_path` (the `Player` actor's
  `SGeometryResourceParams` branches by the same gender tag — the CHF's
  `modelTag` — to `bhm_skeleton_v7.chr` / `bhf_skeleton_v2.chr`), `load_body`
  / `load_chf_body`. `CharacterRig.actor` + `attach_body`: `bones()` =
  actor skeleton overlaid by the head rig (`overlay_bones`: head-only bones
  under their parent by name; a shared bone takes the head rig's pose only
  where both hang it under the same parent — the head rig roots `Spine3` at
  `World`) + part-only bones. The DNA rest leaves the 15 shared bones on the
  actor bind (0.0 m / identical rotations). Package, `chf_to_glb_with_dna_
  identity_and_items` and the `build_*_with_items` GLB builders carry the
  body; the package binds `BodyMaterial` to it. Verified: neck seam (head
  open boundary vs body) ≤ 0.45 mm; parity fixture green with 10 meshes;
  in Blender one 458-bone armature, `LeftForeArm` moves only the body, `Neck`
  moves head + every head item together.
- **G2. Body clips.** `AnimationClip` → `clip::Clip` (by skeleton bone names;
  frames → seconds), a body clip option next to the face clip (CLI
  `--body-anim`, package `PackageAnimation`), and one combined clip when both
  are given (body bones + facial bones + morphs). Verify: GLB and package
  play the same body clip in Blender (parity fixture with a body clip).
  G2 ✅: `character::clips` — `CharacterAnimation { face, body }`
  (`FaceAnimation` = `.caf` + intensity; body = an `AnimationClip`),
  `load_body_clip` / `select_clip` (`.caf`, or a `.dba` clip by stem or
  unique substring), and `HeadAssets::character_rig` — the one rig both
  writers build from (rest or face-expression rig + body + body clip).
  `CharacterRig::add_body_clip` converts the clip by the rig's bone names
  (`animation::shared_clip`) and `Clip::merge`s the face clip into it: one
  clip named after the body clip (the face's 176 facial bones, morphs and
  wrinkle track + the body's bones; measured: no bone is in both). Each
  track keeps its own key times (face 8.1 s, body 10 s here). API:
  `export_character_package(.., &CharacterAnimation)` (replaces
  `PackageAnimation`), `HeadIdentityContext::build_character_glb` (replaces
  `build_expression_glb_with_items`), `chf_to_character_glb`; CLI
  `--face-anim`/`--body-anim`/`--body-anim-clip` on `chf to-package` and
  `chf to-glb` (which always exports the whole character, the package's
  rig). Verified in Blender (masculinedefault, happy idle face +
  `char_customizer_stand_idle_01`): GLB vs package scene.blend, absolute
  vertex positions at frames 0–230 — body ≤ 0.021 mm, head/eyes/teeth/hair
  ≤ 0.003 mm, while they move up to 52 mm; the parity fixture now plays
  both clips. Open: `.dba` clips whose `start_position` is non-zero
  (`e1c02s00_*`) are played as authored samples (no anchoring) — check
  whether the engine offsets the root by it before relying on them.
- **G3. Armour / clothing loadouts.** Attach armour items (CA_SKIN parts on
  the same skeleton, e.g. the KAP set) from a loadout record; they bind by
  name to the merged skeleton.
  Findings (2026-10-16):
  - **Loadouts** are the engine's item-port XML (CryXmlB),
    `Scripts/Loadouts/Character/Human/Armor/NPC/…`: `Body_ItemPort`
    (`itemName=""` = keep the character's body) → `Armor_Undersuit` →
    `Armor_Arms` / `Armor_Legs` / `Armor_Torso` / `Armor_Helmet`, plus
    non-skin items (`mobiglas_attach`). The archetype `baseLoadout` uses the
    same format. Garments branch their geometry by gender tag like the
    body; their `Male` node often has no `.mtl` (the material node has it).
  - **Clothing layers** (`SCItemClothingParams`): each item claims body
    zones (`Chunks[].MeshChunk`, enum `EMeshChunks`, 64 zones + UNDEFINED)
    at a `Layer` (body 0, undersuit 3, armour 4), with per-chunk
    `VisibleLayers` exceptions and `VisibilityConditions`; `HiddenParts`
    hides whole ports (undersuit → the `Clothing_*` ports; a full helmet →
    `Head_ItemPort`, `hair_flair_ItemPort`, …).
  - **Zones in the mesh**: every `.skinm` submesh descriptor's
    `node_parent_index` indexes the mesh's extra-word table (after the
    descriptors), each a zone-name hash: **FNV-1a-32 of the UPPERCASE
    `EMeshChunks` name including its NUL** (`L_HAND_ZONE\0` → 3335329176;
    all 30 `m_body` zones resolve; geometry agrees — node 26 at x = −0.645
    is `l_hand_zone`). A name outside the enum (the undersuit's main body)
    is in no zone, so nothing hides it.
  - Rule: a part's submesh in zone Z is hidden when another worn item claims
    Z at a higher layer that is not in the part's `VisibleLayers` for Z. So
    an undersuit hides the whole body, and armour arms (which claim the hand
    zones) hide the undersuit's gloves.
  Steps: (a) `clothing`: zone hash, `SCItemClothingParams` reader, hidden
  zones per item; (b) `PartSource` per-submesh zone table; (c) `loadout`:
  XML → entries → garments (gender geometry + material), `HiddenParts`;
  (d) `Outfit` (body + garments + hidden zones/ports) into
  `character_rig` → both writers drop hidden submeshes (`Mesh::subset`);
  (e) CLI `--loadout`; parity fixture with an armour loadout; Blender check.
  G3 ✅: `character::clothing` (`zone_hash`, `clothing_params`,
  `hidden_zones`), `PartSource::zones` (descriptor matched by index range →
  extra word) + `visible_submeshes`, `character::loadout` (`parse_loadout`,
  `resolve_garments`: the CHF root port keeps the CHF body; gender `.skin`
  node; material from `materialVariants`, else the node's, else empty = the
  `.skin`'s own `.mtl`; items without a gender `.skin` skipped; items under a
  `HiddenParts` port dropped), `character::outfit::Outfit` (body + loaded
  garments + covered zones + hidden CHF ports) → `CharacterRig::dress`
  (replaces `attach_body`); `part_payloads` writes `Mesh::subset` of the
  visible submeshes and skips fully covered parts. `character_rig`,
  `build_glb_with_items`, `build_character_glb` take `&Outfit`;
  `export_character_package` / `chf_to_character_glb` take the loadout; CLI
  `--loadout` (`to-package`, `to-glb`). Found and fixed on
  the way: `SubMaterial::is_decal` / `alpha_config` / `is_double_sided`
  matched `%DECAL` as a substring, so LayerBlend_V2's `%DECALS` (decal-sheet
  feature) made whole garments (and any such ship material) decals —
  dropped from textureless GLBs, alpha-blended, double-sided; now the whole
  token. Verified: masculinedefault in `marine_Armor_Heavy` — the undersuit
  covers all 32 body zones (body not written), the armour arms cover the
  undersuit's gloves (40,314 of 44,488 vertices written), mobiGlas worn;
  `Pirate_Heavy_01`'s helmet hides hair/brows/lashes/beard/scalp; parity
  fixture `armoured` (marine + body clip) green; Blender renders both
  outfits animated. Open: (1) ✅, (2) ✅, (3) ✅ (below).
  (1) ✅ Full helmets hide the DNA head. Rule from data: the DNA head is the
  CHF item on `Head_ItemPort` (`PU_Protos_Head`, the head rig's own `.skin`;
  eyes on `eyes_itemport` under it), and a full helmet's
  `SCItemClothingParams.HiddenParts` lists that port. Zones are not the
  mechanism: the head `.skin` has no zone table, and the helmet claims only
  `theta_head_zone`. `Outfit::hides_geometry` (the worn `.skin` of every CHF
  item on a hidden port) → `CharacterRig::hide_head`: no head geometry, no
  CDF sub-meshes (eyes, teeth, eye details), no blendshapes / morph tracks;
  the skeleton and facial bone tracks stay. Writers: a root without
  geometry is a mesh-less GLB node, and in the package `mesh_asset: null`
  with the armature at the entity root. Verified: `full_helmets_hide_the_dna_head`;
  parity fixture `helmeted` (Pirate_Heavy_01, face + body clip).
  (3) ✅ Material slots past the `.mtl`. The Pirate helmet `.skin`'s
  `MtlName` chunk records 7 slots of `m_slaver_heavy_armor_helmet_01_01_01`
  but the shipped `.mtl` has 6 (the mesh was compiled against an older
  `.mtl`); the GLB kept slot 6, the package dropped it. Engine rule
  (CryEngine `CMatInfo::GetSafeSubMtl`): a `.mtl` without `<SubMaterials>`
  serves every slot; a slot past a multi-material draws with the engine
  default `Materials/material_default.mtl` — drawn, not dropped.
  `material_slots::resolve_material_slots` applies it where a mesh meets its
  `.mtl` (`load_single_mesh`, and again after a paint override swaps the
  root `.mtl`); the sidecar carries the default at that slot (a hashed
  variant of the shared sidecar path). Parity allowlist entry removed;
  `helmeted` asserts the sidecar's `material_default`; Blender builds the
  slot as Illum.
  (2) ✅ Clothing visibility, RE'd from the client (Ghidra,
  `ItemPortContainer::RefreshMeshChunkVisibility` + the per-chunk predicate
  `CSCItemClothing::EnableOverlay::$_0`, cross-checked in the cig-scgs
  decompile). Per zone, layers top-down: the top layer's chunks are
  evaluated; a lower layer is drawn only if a visible chunk above lists it
  in `VisibleLayers` — which names LOWER layers (608/608 entries in data;
  the earlier rule read it upside down, so short sleeves hid the body's
  forearms). A chunk is visible when it has no `VisibilityConditions`, or
  one condition whose `PortVisibility` entries all match `ActiveTags` and
  not all `SkipTags`; all matching `DisableTags` hides it; an empty
  condition hides. An entry tests the items on its `Ports` (an empty
  existing port without `SkipTags` matches an empty `ActiveTags`; a missing
  port matches nothing) or, with no ports, every worn item's tags. A tag
  list matches when all its Tag records (by GUID) are among the item's
  entity `tags` + `OverlayTags`. `clothing::hidden_zones(&[Worn])`. Found on
  the way: `starbreaker-datacore` returned `Null` for query paths ending in
  a reference array (an entity's `tags`); now the record ids. Verified:
  `a_short_sleeved_shirt_keeps_the_body_forearms`,
  `an_armour_core_shows_its_backpack_mount_without_a_backpack`; Blender
  render of Shipjacker_Medium_01.
- **Armour export** (spec `docs/superpowers/specs/2026-10-13-armour-export-design.md`):
  a single wearable piece exports as worn with
  `entity export <item> --geometry-tag Male|Female` (the record's
  sub-geometry variant with that tag); sets via `starbreaker armour export --wear …`
  (`character::wear`: only ports that skin their item are worn on — hands
  and pockets carry; default wearer). Armour-only (no head/body) deferred
  with helmet head-hiding.

## Not in scope here

- Ship clips into GLBs (parity plan E2).
- `.blend` shader-parameter tracks (wrinkles) — parity allowlist.
