# SC Dressing Room — analysis and what we adopted

[SC Dressing Room](https://scdressingroom.gamers-fix.com/) is a three.js
(WebGPU) character viewer that uses a JS port of `starbreaker-chf` to read
CHFs. Its offline Node pipeline (`pipeline/build-face-rig.mjs`, `build-skin8.mjs`,
`probe-wrinklemap.mjs`, …) bakes game data into flat arrays, and the browser
evaluates simple formulas. Analysed 2026-10-13 from its public bundles and
`/data/*.json|bin`. Everything below marked **verified** was re-derived from
`Data.p4k` and is pinned by `crates/starbreaker-3d/tests/dna_ground_truth.rs`.
Nothing here depends on the site.

## DNA head: verified facts (`.dna` master file)

| Slot | Content | Verified against |
|------|---------|------------------|
| 2  | 61 × 40 B archetype names; CHF `head_id` indexes it; entry 0 = protos base | names = `<name>_head.skinm` stems |
| 15 | per-variant **parent-local** joint translations (179 RigLogic joints) | variant 0 = head `.skin` bone locals |
| 18 | per-(variant × region) neutral vertex deltas | one variant, all regions = archetype mesh − base ≤ 0.2 mm |
| 19 | per-(variant × blendshape) **archetype blendshapes** | cell (v,b) = archetype v `BShapesGpu[b]`, ≥ 99.86 % within one 8-bit step |
| 21 | joint × region weights `[179][13]` | (byte-identical to the site's table) |
| 22 | vertex region masks, **region-major** `[13][NV]` f32 + 8 trailing bytes | per-vertex partition of unity |

Per character, with `k[v][r]` = CHF blend values normalised per face part
(top-4) and `mask` = slot 22:

```
shape[vtx]      = base[vtx] + Σ_r mask[r][vtx] Σ_v k[v][r] · Δ18_v[vtx]
joint_local[j]  = rest[j]  + Σ_r w21[j][r]   Σ_v k[v][r] · (s15_v[j] − s15_0[j])   (re-bind)
blendshape_b    = Σ_v fold_v · T19_v[b],  fold_v = Σ_r mask[r][vtx] · k[v][r]
```

### Bugs this fixed on `feature/dna`
1. Slot 22 was read vertex-major after an "8-byte header", which scrambled every mask.
2. Slot 19 was decoded as a u24 weight × direction starting at vertex 0. It is actually a start
   vertex (`+0x04` hi16), scale (`+0x08`) and bias (`+0x14`), with entries `[stride, qx, qy, qz]`.
3. Slot-15 joints were posed but not **re-bound**: the inverse binds came from the
   stale rest pose. `apply::rebind_skeleton` recomputes the world transforms by FK.
4. The head kept its base normals after the slot-18 morph (10.2° → 2.3° mean
   error after `normals::transfer_normals`).
5. Head skinning was truncated to 4 influences. 2,660 of 5,584 male head
   vertices use more, so the head now emits `JOINTS_1`/`WEIGHTS_1`.

The site makes one mistake here: it treats the slot-15 deltas as head-space
deltas, but they are parent-local.

## Eyes, teeth and head items (adopted)
- **Eyes and teeth have their own `.dna`** (`protos_*_{eyes,teeth}.dna`), covering only
  their face parts. Header byte `0x29` is the local region count, and slot 12 lists the
  global part of each local region: eyes = EYE_L/EYE_R, teeth = NOSE_C/MOUTH_C.
  Each part's slot 18 reproduces the archetype's own eyes/teeth mesh. The site
  instead "seats" rigid eyes, which is an approximation.
- **Head items** (hair, brows, lashes, beard, universal scalp) are CHF item ports.
  Each one's DataCore record has per-gender (`Male`/`Female`) sub-geometry with a
  `WD_*` wrap deformer (`WD_Elastic`, `WD_ElasticNUScaling`, `WD_ElasticDQSkinning`).
  So the engine fits them to the per-character head surface. Brows, lashes and beard
  are also skinned to `FACIAL_*` joints (90–93 % of their weight) for expressions;
  scalp hair is weighted to `Head` and the `hh_*` physics bones.
  `character::items` resolves them structurally (by deformer). `character::wrap`
  applies a smooth inverse-square (Shepard) field of the head's slot-18 deltas, using a 3 cm grid.
  On a pure `silas_t1` head the universal scalp keeps its authored clearance
  within 0.03 mm, versus 3.6 mm unwrapped.

Items are wrapped at the character's rest pose and then written as skinned
parts bound by joint name (`character::shared::CharacterRig`), so brows,
lashes and beard follow the facial joints in an expression clip.

## Still open
- Hair physics bones (`hh_*`, pendulum params in the item `.cdf`) are not simulated.

## Wrinkle maps (adopted)
- **Drivers:** RigLogic Section G/H, which is *not* bone-region blending. It holds 24 groups of
  (activation index, weight) rows (38 rows), byte-identical to the standalone
  `Objects/Characters/RigLogic/shared_wrinkle_mask_human_face_t1.RigLogic`.
  Group = weighted sum of activations, clamped to [0, 1].
- **Shader:** decoded from `HumanSkinWrinkleBlend_V2.cfib` (FXB1 token stream +
  CRC string table) in `Engine/ShaderCache_D3D11.pak`.
  - With `%WRINKLE_BLENDING_2`, `PI_WrinklesMask` is `float4[6]`. The mask has 2 panels, and each panel uses
    R 4 / G 3 / B 2 / A 3 bands of 64 levels.
  - Each band feeds `WM_0` (atlas tile 1, the "stretch" set: forehead lines, neck cords, lip pucker) or
    `WM_1` (tile 2, the "compress" set: frown lines, crow's feet, nose lines, chin dimpling).
  - The full table is `character::wrinkles::SLOT_LAYOUT`.
- **Group → slot:** this is not the identity and is not stored in any data file
  (checked: shader, material, `.dna`, both RigLogic files, an exe constant scan).
  It was derived from data:
  - Each group lands on its own side's panel.
  - A confidence-ordered one-to-one assignment of rig-evaluated expression footprints to mask
    regions reproduces the table on both genders. That assignment is pinned by
    `tests/wrinkles.rs`.
  - The atlas tile semantics agree with it: brow-down and chin are in tile 2; brow-raise-inner and lip pucker are in tile 1.
  - The site's measured table differs in 4 entries: it swaps the chin and lower-lip-pucker regions.
- **Export:** an expression clip carries the 24 weights as a `PI_WrinklesMask`
  material-parameter track (`clip::MaterialParamTrack`). The GLB writer puts it in
  the animation's extras (`starbreaker_material_params`, `gltf/animation.rs`):
  glTF cannot animate shader parameters.
- **Blender:** the addon's HumanSkin_V2 builder (`runtime/importer/skin.py`)
  blends the wrinkle atlas tiles by 24 `wrinkle_NN` weights.

## Still open (wrinkles)
- The `.blend` package does not key the wrinkle weights yet (an allowlisted
  GLB/`.blend` parity gap, `docs/superpowers/plans/2026-10-14-glb-blend-parity.md`).

## Rendering: the site ports CIG's shaders line by line
The site's comments cite CryFX sources (`HumanSkin_V2.cfxb`, `HairPBR.cfxb`, `layerLib.cfib`,
`ShadeLib.cfib`, `Eye.cfxb`). It decoded them from `Shaders/CFXIB/D3D11` in
`ShaderCache_D3D11.pak` with its own `tools/fxb-decode.py`, and checked them against DXBC disassembly.
We have since decoded `HumanSkin_V2` and `HairPBR` from the shader cache ourselves
(`docs/superpowers/plans/2026-10-13-character-skin-material.md`); the addon's
skin and hair builders follow that math.

The terms most worth adopting in the Blender addon, in priority order:

1. **Skin tone is a rebase, not a multiply** (`TONE_ADJUSTMENT`). With `src` =
   mtl `SourceAverageColor` (or the diffuse mean) and `tone` = the CHF colour,
   linearised:
   `d = max3|albedo−src|`, `c2 = lum(tone)−lum(src)`,
   `amt = mix(SourceDetail, min(1,SourceDetail), sat(4d)) · (c2<0 ? 1+sat(2|c2|) : 1/(1+sat(c2))) · mask.g`.
   Then `details = albedo/src`, and its `.gb` channels are pulled toward `dot(.gb,(0.7,0.3))` by `sat(4|src.g−tone.g|)`.
   The output is `mix(albedo, sat(pow(details, amt)·tone), mask.r)`. Head and body match
   because both are rebased onto the same tone.
   The diffuse is built in this order: wrinkle → tone → blemish → tattoo → makeup.
2. **Skin specular:**
   - Smoothness = `(Shininess/255)·gloss·0.8`, where gloss is the ddna alpha.
   - Roughness = `clamp(1−sm, 0.04, 1)`. CryEngine's GGX α is `(1−g)²`.
   - F0 = `MatSpecColor·spec map`, f90 = `sat(avg(F0)/0.02)`, Fresnel exponent `5·0.65`.
   - Micro detail: `skin_micro_detail` sampled at `uv·65·(1,0.25)`, faded by UV stretch, combined with reoriented normal mapping (RNM).
3. **Normal maps:** ddna xy = `sample.yx`, no extra green flip.
4. **LayerBlend_V2:**
   - Blend weights: `a1=R, a2=G(1−a1), a3=B(1−a1−a2), a4=rest`.
   - Palette tints are linearised as `(v/255)²` (gamma 2, not sRGB).
   - `_hal` map: R = hue shift via `layerblendHueLUT`, B = luminance `·(B·0.4+0.8)`, G = AO.
   - Wear is exactly 0 at instance wear 0.
5. **Hair colour** uses the Chiang 2016 melanin model:
   - `q = −ln(1−m²)`, `σ = q(1−red)σEu + q·red·σPheo`, colour = `exp(−√σ·5.888)`.
   - `σEu = (0.419,0.697,1.37)·1.2`, `σPheo = ((0.187,0.4,1.05)·2.4)^1.35`.
6. **Tonemap:** the game's own display curve, with toe/shoulder `ks=0.5` and
   hue-preserving desaturation.
