# GLB ↔ Blender parity

Goal: the glTF and Blender outputs carry the same decoded data, and the build
or the tests fail when one of them drops something.

## Why they drift (2026-10-14)

1. **Two data models.** Entity exports build `DecomposedInput` / `Mesh`
   (`types.rs`) and serialize it twice: `GlbBuilder` (glTF) and
   `pipeline/blend_assembly.rs` + `starbreaker_blend` (hand-written `.blend`
   SDNA blocks). `Mesh` has no skin weights or morph targets, so the character
   rig lives in its own types (`HeadMesh`, `character::head_skin::Bone`,
   `BlendShapes`, `GltfAnim`) with its own glTF writer (`character/glb.rs`).
   Character skinning, blendshapes and the wrinkle animation reach glTF only.
2. **Silent drops.** Per-vertex data is copied or serialized in about 8 places
   (`Mesh::merge_from`, `subset_mesh_for_submeshes`, axis conversion,
   `pack_geometry`, `hash_mesh_for_reuse`, two `.blend` mesh paths). Adding or
   ignoring a field compiles fine. For example: the `.blend` path carries
   `normals` through splitting but never writes them, and `merge_from` leaves
   `normals`/`tangents` shorter than `positions` when only one side has them.
3. **Materials are rebuilt on the Blender side only** (addon builders over the
   sidecars); glTF materials are approximations.

## Direction (agreed 2026-10-14)

One export pipeline for every entity kind (ships, items, characters), with
**both** outputs (GLB and `.blend`) for all of them; each has its own
advantages. No kind-specific writer: characters become one more producer of
the shared model, and `character/glb.rs` stops being a separate writer.
(The 2026-10-13 "canonical character GLB" plan is not pursued.)

Why the shared model must grow anyway: the entity pipeline already parses
skinning (`IVOBONEMAP` 4 influences, `IVOBONEMAP32` 8) but only uses it to
split rigid submeshes onto bones (`split_rigid_weighted_submeshes`); a
soft-skinned mesh (`rigid_ratio < 0.9`) loses its weights in both outputs.
Armour, clothing and characters all need real skins.

## Rules

- Producers build the shared model and never branch on the output format;
  only writers know the format.
- Every function that copies or serializes `Mesh` / `SubMesh` destructures it
  **without `..`**. A new field breaks the build until each site handles it
  or names it as a gap (`field: _ // PARITY GAP: <why>`).
- Per-vertex operations live on `Mesh` (`types.rs`), not in the writers.
- The parity test's allowlist only shrinks.

## Phases (sequential; each ends green + committed)

- **A. Guards.** ✅ PR #39. Exhaustive destructuring at every `Mesh`/`SubMesh` copy and
  serialization site. Move per-vertex copying (`subset_mesh_for_submeshes`)
  onto `Mesh`. Fix the `merge_from` padding bug (TDD). Name existing gaps:
  `.blend` custom normals and tangents.
- **B. Rig in the shared model.** Steps: (B1 ✅, B2 ✅, B3 ✅) `Mesh.skin` + `Mesh.morph_targets`
  fields, carried by every guarded copy site, filled from the IVO bone maps for
  soft-skinned submeshes; writers name them as gaps. (B2) `GlbBuilder` emits
  them. (B3) `.blend` vertex groups from the skin. Details: `Mesh.skin` (joints/weights, more than 4
  influences), `Mesh.morph_targets` (named position/normal deltas), and one
  skeleton type with inverse binds (unify `skeleton::Bone` and
  `character::head_skin::Bone`). `GlbBuilder` emits skins, `JOINTS_n`/`WEIGHTS_n`
  and morph targets (ported from `character/glb.rs`). `.blend`: vertex groups
  from the skin (`MDeformVert` already exists); armature and shape keys are
  named gaps until phase D.
  B2 notes: the rigid splitter now uses "exactly one influence" (it used the
  dominant joint, snapping blended armour/clothing onto single bones); a mostly
  rigid submesh splits its rigid triangles and keeps a soft remainder. Live
  check: `m_kap_combat_heavy_arms_01` exports a 68-joint skin that Blender's
  glTF importer binds and deforms. (The flat-path skeleton was flat
  world-matrix bone nodes then; C1 made it a joint hierarchy.)
  B3 notes: `deform_groups` (decal masks + one group per skin joint) is the
  single vertex-group writer for the flat and hierarchy `.blend` paths; it
  replaced two copies, one quadratic and one that corrupted multi-group
  vertices. `cli/src/blend.rs` was a third, root-mesh-only `.blend` writer
  (bundled `entity export --format blend`); it is gone, replaced by
  `skin_to_blend` on the shared writer (and `skin export … out.blend`).
  Verified in Blender on `m_kap_combat_heavy_arms_01`: `.blend` and GLB have
  the same 20,510 vertices, the same joint groups and identical per-group
  weight mass. Armature + modifier remain a named gap (phase D).
- **C. Character through the shared model.** ✅ PR #39. `character::package` and
  `chf to-glb` both build `DecomposedInput` (skeleton, skinned head/items,
  blendshapes); `character/glb.rs` stops being a separate writer.
  C notes: (C1) the shared writer emits the skeleton as a joint hierarchy
  (resolves the B2 open note) and skins from `skeleton::Bone.bind_world_*`, so a
  posed skeleton deforms its meshes. (C2/C3) `HeadAssets::rig_state` →
  `character::shared::CharacterRig` (head + `bs_NNN` morph targets, posed
  skeleton bound at the per-character rest, sub-meshes and CHF items at REST
  as child meshes skinned by joint name); every static character GLB
  (`chf to-glb`, the app preview, `HeadIdentityContext::build_glb*`) is written
  by `GlbBuilder`. Parts now follow expressions through their skins (live:
  `CTL_C_jaw ty = 1` moves head and teeth together, up to 34 mm; one armature
  of 270 bones drives all 9 meshes in Blender). Only the animated expression
  GLB still uses `character/glb.rs` (phase E deletes it). (C4) the package
  reads the same rest rig and carries its skeleton as `root_bones`; package
  files are byte-identical, the bones wait for the phase-D armature. Morph
  targets stay out of the package until `.blend` has shape keys (D).
  Side effect: a flat-root GLB names its root node and mesh after
  `metadata.entity_name` (not for decomposed GLBs) and children can attach to
  it by `parent_entity_name`.
- **D. Blender rig.** ✅ PR #39. Armature object and bones, shape keys, and custom split
  normals in `starbreaker_blend`, or built by the addon from package data.
  Decide per feature when we get here; the parity test treats both as "present
  in the Blender output".
  D notes: written by Rust, not the addon (decomposed packages are opened as
  `scene.blend`). `starbreaker_blend::armature` (bArmature/Bone/armature
  Object/ArmatureModifierData + Blender's `vec_roll_to_mat3` / roll
  extraction) and `::shape_keys` (relative Key + KeyBlocks), field values
  matched against a file Blender 5.1 wrote. The flat `.blend` writer turns
  the skeleton's BIND pose into an armature and puts an Armature modifier
  first on a skinned mesh; `scene.blend` writes one armature for the package
  skeleton (`root_bones`) in the root mesh object's space and binds every
  skinned mesh object to it (the counterpart of glTF skins binding by joint
  name); asset files carry shape keys, never armatures. Verified: KAP armour
  `.blend` deforms like its GLB (rest + posed within 0.001 mm, edit-mode round
  trip 3e-5); masculinedefault `scene.blend` = one 198-bone armature driving
  all 9 parts at rest within 1 µm, 496 shape keys, `L_Eyeball` turns only the
  left eye. The package takes each part's skin from the rig (the pipeline
  loader leaves fully rigid parts unbound). Open: (1) NMC hierarchy assets
  get no Armature modifier or shape keys (their skeletons are NMC-node
  empties); (2) a POSED skeleton (default animation pose, control-driven
  character pose) is not written as pose channels — Blender shows the bind
  pose (E: pose = frame 0); (3) custom split normals — DONE: `custom_normal` corner attribute, see below;
  (4) fully rigid `.skin` parts are unbound in BOTH flat writers outside the
  package (separate task). Note: single facial joints are not expressions —
  RigLogic drives LOD0 and LOD1 joints together (E).
- **E. Animation tracks** in the shared model: bone TRS, morph weights and
  material scalars (the 24 wrinkle weights) → glTF animations and the package's
  existing clip JSON → addon actions.
  E1 ✅ PR #39: `crate::clip::Clip` (local bone TRS keys by bone name,
  morph-weight tracks by target name, material-parameter tracks) →
  `gltf::animation` (one glTF animation per clip; bone nodes switch to TRS;
  morph weights per mesh at the union of key times; material parameters in
  the animation's extras). The character expression GLB is a `CharacterRig`
  with an "expression" clip; `character/glb.rs` is deleted, and eyes/teeth
  now move with the clip through their skins.
  E3 ✅ PR #39 (decided: Rust writes the actions, as in D):
  `starbreaker_blend::action` (Blender 5.1 layered actions, matched against
  a Blender-written file); bone tracks → pose-bone curves `R⁻¹·L` on the
  armature object, morph tracks → shape-key curves on the mesh's Key (in the
  asset file, beside the mesh); first clip assigned, others fake-user; the
  scene end frame covers the clips. Packages can bake a face clip
  (`chf to-package --face-anim`). Reshaped character parts moved to
  `Characters/<name>/<file>` (they collided at shared source paths; the
  directories are dropped because Windows cannot open library paths over 260
  characters). Verified: the package's scene.blend plays the happy idle with
  head (bones + 224 shape keys), teeth and eyes within 0.0007 mm of the GLB.
  Gap: material-parameter tracks (wrinkles) are not written to .blend.
  E2 ⏳ open: entity `.dba`/`.caf` clips into entity GLBs. Today they reach
  Blender only as package clip JSON, which the addon applies with
  ship-specific logic (Additive/Override bone classification, `start⁻¹ ·
  sample` anchoring, position policies, endpoint/transition policies,
  duplicate-hash disambiguation; `runtime/package_ops.py`
  `_apply_best_channel_transform`). Converting them to `Clip`s means moving
  that composition into Rust first; then both writers get them for free.
- **F. Parity test.** Export the same input both ways, read both back, reduce
  each to one summary (nodes, transforms, vertex counts + position hash,
  normals, UV sets, colours, material slot → sidecar, bones, weights, morph
  names, animation channels) and compare. Fixtures: synthetic (no p4k) +
  one character + one ship (p4k-gated). Known gaps in
  `crates/starbreaker-3d/tests/parity_allowlist.txt`.
  ✅ PR #39. `starbreaker_blend::reader` + `starbreaker_3d::parity`
  (`from_glb`, `from_blend` following linked libraries, `diff`,
  `Allowlist`): per mesh counts, position-multiset hash, normals, UV sets,
  colours, material slots, joint weight mass, morph names; weighted joints'
  bind heads; clip channels, key times, morph values. Fixtures: synthetic
  (`parity::tests`) and, in `tests/parity.rs`: `armour` (the KAP arms
  `.skin`), `character` (masculinedefault: body, face + body clip since
  full-character G2), `armoured` (masculinedefault in the marine heavy armour
  loadout, body clip) and `piece` (one armour item,
  `cds_armor_heavy_arms_01_01_01`, through the entity pipeline). Unexpected
  differences AND stale allowlist patterns fail. Sabotage check: dropping the .blend shape-key curves fails with 223
  differences. Found and fixed: the character skeleton lacked the items'
  own bones (hair braids, beard) — now merged into the rig by name.
  Allowlisted gap (reason in the file): .blend material-parameter tracks.
  Closed since: (PR #39) the character GLB builds its parts with the
  package's loader (`character::part_source`: colours, both UV sets,
  per-submesh `.mtl` materials), and a submesh without a `.mtl` gets its own
  default material per material id in the GLB (it was merged), which also
  closed the armour material-slot gap; (PR #39) `.blend` custom split
  normals — `Mesh.normals` as Blender's fan-relative `custom_normal` corner
  attribute (`starbreaker_blend::custom_normals`, bit-exact vs Blender's
  encoder; follows the Armature modifier), and meshes with authored normals
  drop the Weld + Weighted Normal modifiers. KAP arms: corner normals within
  1e-3 of the GLB at rest and posed (the old writer was off by up to 0.89).
  Open: the ship fixture — the two writers decompose NMC meshes differently
  (GLB: one vertex buffer shared by node primitives; .blend: a subset mesh
  per node), so it needs a world-space geometry summary first.
