# Character skin material (HumanSkin_V2) — Blender path

Goal: a CHF character imports into Blender through the normal decomposed
package path with an engine-faithful HumanSkin_V2 head material (tone
rebase, gloss/specular rules, micro detail, translucency approximation,
blemishes, wrinkle blending), driven by the CHF's own customization data.
glTF-native materials are out of scope for now (user decision 2026-10-13).

## Ground truth

- Shader: `HumanSkin_V2.cfxb` + `HumanSkinWrinkleBlend_V2.cfib` decoded from
  `Engine/ShaderCache_D3D11.pak` (FXB1). `GetMaterialAttribs` order:
  normal (TexSlot2) + wrinkle normal deltas → smoothness =
  `MatSpecColor.w · gloss(TexSlot2Attached) · 0.8` → albedo (TexSlot1) +
  wrinkle diffuse deltas `· sqrt(sat(w / max(1, Σw)))` → TONE_ADJUSTMENT
  rebase (TexSlot7 mask r/g/b) → blemishes → spec = TexSlot4 · MatSpecColor →
  tattoo/makeup → micro detail (TexSlot6 at `uv·65·(1,0.25)·MicroDetailTiling`,
  RNM, scaled by mask.b) → transmittance = `TexSlot12.r · TranslucencyMultiplier · 0.25`.
- CHF material GUIDs → files: DataCore record
  `SCharacterGenerationParams.DefaultCharacterGenerationParams`
  (`libs/foundry/records/gamemode/defaultcharactergenerationparams`),
  `materialLookupTable.{materials,textures,decals}[{guid, filePath}]`
  (94 materials incl. every per-character head `.mtl`, 40 textures).
- CHF overrides per material: `Head Material` (blemish textures + params,
  makeup), `BodyMaterial` (`BodyColor` → `FinalSkinTone`), `EyeMaterial`
  (`EyeColor`), `Hair/Beard/EyebrowDyeMaterial` (melanin + dye).

## Phases (sequential; each ends green + committed)

1. ✅ (PR #39) **Customization resolve (Rust, `character::customization`)** — CHF
   materials → resolved `.mtl` / texture paths via the lookup table, plus
   named params/colours. Tests: default CHFs resolve the head `.mtl`
   (`male17_t1_head_material.mtl` for masculinedefault) and blemish textures.
2. ✅ (PR #39) **Character package export (Rust, `character::package`)** — build a
   `DecomposedInput` from the DNA-shaped head (+ eyes/teeth/eyedetails, CHF
   items) with their `.mtl`s, write through `write_decomposed_export_blend`;
   add `character` metadata (resolved CHF overrides, wrinkle weights) to the
   root scene instance. Tests: package files exist, sidecar for the head is
   HumanSkin_V2 with TexSlot1/2/3/4/6/7/8/11/12 exported.
3. ✅ (PR #39) **Addon HumanSkin_V2 builder** (`runtime/importer/skin.py` mixin) —
   dispatched for `shader_family == "HumanSkin_V2"`; implements the ground
   truth above with CHF overrides from the instance metadata. Tests: bpy-stub
   dispatch test + socket probe; a live-Blender render check.
4. **Wrinkle animation keying** — partial. Expression clips carry the 24
   weights as a `PI_WrinklesMask` material-parameter track
   (`clip::MaterialParamTrack`), written to the GLB animation's extras
   (`starbreaker_material_params`, `gltf/animation.rs`). The `.blend`
   package does not key them yet: an allowlisted parity gap
   (`2026-10-14-glb-blend-parity.md`).

## Status / findings (2026-10-13)

- CLI: `starbreaker chf to-package <chf> <export-root> [--texture-mip N]`
  → `Packages/<chf stem>_LOD0_TEX<N>/{scene.json,scene.blend}` + shared `Data/`.
  CHF override textures (blemish mask/ID, makeup) ship as PNGs, mapped per
  customized submaterial in `texture_exports`.
- Blender: open `scene.blend`; the addon's material refresh builds the skin.
  Validated headless in Blender 5.1 / Cycles: female16 head, 329-node graph,
  24 wrinkle slots; a dark `BodyColor` rebases tone with detail preserved and
  freckles land per the blemish mask/ID.
- `BodyColor` (sRGB) linearised = `FinalSkinTone`: the default CHF colour
  linearises to ≈ the head's `SourceAverageColor` (identity rebase).
- `MicroDetailMap` is the engine `skin_micro_detail` (same data as the head's
  TexSlot6); its normal XY is in **A, G** (CryEngine detail layout).
- Not yet modelled: makeup / tattoo (`HumanSkinMakeup.cfib`, TexSlot13–15 +
  `Makeup*` params/colours, `CUSTOMISATION_ADVANCED` metal blend), `%DECAL_MAP`.
- `HairPBR %HAIR_CAP` (scalp / brow-base caps) fixed: the generic builder fed
  the cap's TexSlot1 Color into Base Color *and* Alpha → an opaque white
  shell whose straight-alpha colour showed the strand pattern. Per
  `HairPBR.cfxb` TexSlot1 is a ColorMap (rgb · BaseTintColor, coverage in
  alpha, fade `1 − (1 − N·V)^16`) → `importer/hair.py`.
- `HairPBR` fibres (cards / coat) built per `HairPBR.cfxb`: `GetHairColor`
  melanin absorption with per-strand ID variation, dye layer with hair-length
  fade (raw vertex colour x; y/z = AO), reflectance/roughness rules, dithered
  opacity as alpha (cards `opac^(2/3)`, coat `opac^(0.8/γ)`). Cap is
  diffuse-only (an earlier read of `#if ! %HAIR_CAP` as `#if %HAIR_CAP` was
  wrong — the old pretty-printer dropped `!` / numbers > 11).
- CHF material slot = `crc32(lowercase(port) + "_skinitemport")`; the CHF
  hair-dye colours are `DyeColor` (1) and `BaseTintColor` (2), sRGB.
- Scalp caps are tag-selected: the hairstyle publishes `$scalpVHair_NN+`, the
  universal scalp's `materialVariants` pick `f_hair_NN.mtl` (gender →
  scalpVHair). Validated: cap dense bands sit under the braids (overlay).
- Fibre shading = diffuse (wrap energy) + R / TT / TRT as Cycles Hair BSDFs
  along the card V axis (energies/shifts from `hair_lobes()`); Principled's
  grazing sheen made black hair read grey. Not modelled: per-card glint
  noise, `globalScattering`, engine noise for cards without an ID map,
  `DyeColorMap` (unbound → white).
- Still generic: eyes (`Eye`/cornea: render white), teeth.
- Fixed since: the `.blend` meshes now carry authored normals as the
  `custom_normal` corner attribute (`pipeline/blend_assembly/custom_normals.rs`),
  so the DNA-transferred head normals (and baked card frames) reach Blender.
