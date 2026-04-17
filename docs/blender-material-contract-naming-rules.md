# Blender Material Contract Naming Rules

## Goal

These rules convert the verified slot evidence in `StarBreaker/docs/blender-material-slot-evidence.md` into stable contract input names for the generated seed contract.

The intent is to make the naming predictable for both the Blender node-group authoring work and the future runtime wiring.

## Rules

- Texture inputs use `TexSlotN_<Semantic>`.
  The original slot number always stays in the input name.
- Semantic suffixes use PascalCase in the contract and snake_case in the `semantic` field.
- If a slot is overloaded inside one shader family, choose a family-local umbrella name instead of splitting one slot into multiple inputs.
  Example: `DisplayScreen` `TexSlot9_ScreenSource` covers both RTT and static screen-plate variants.
- Public parameters use `Param_<OriginalName>`.
  The original parameter name is preserved rather than translated.
- Palette channels use `Palette_<ChannelName>`.
- Standalone virtual inputs use `Virtual_<NameWithoutDollar>`.
- When a slot is still too broad to name precisely, prefer a conservative auxiliary name like `DetailAux` or `TintMask` over an over-specific guess.

## Current Texture Input Map

## DisplayScreen

- `TexSlot2_NormalGloss`
- `TexSlot3_CrackNormalGloss`
- `TexSlot6_ScreenSurfaceMask`
- `TexSlot9_ScreenSource`
- `TexSlot10_CrackMask`
- `TexSlot11_Dirt`
- `TexSlot15_CondensationNormal`
- `TexSlot16_DisplayMask`
- `TexSlot17_PixelLayout`

## GlassPBR

- `TexSlot2_NormalGloss`
- `TexSlot4_TintColor`
- `TexSlot6_WearGloss`
- `TexSlot11_Dirt`
- `TexSlot15_CondensationNormal`
- `TexSlot16_PatternMask`

## HardSurface

- `TexSlot1_BaseColor`
- `TexSlot3_NormalGloss`
- `TexSlot6_Displacement`
- `TexSlot10_IridescenceColor`
- `TexSlot14_Emissive`

## Illum

- `TexSlot1_BaseColor`
- `TexSlot2_NormalGlossPrimary`
- `TexSlot3_NormalGlossSecondary`
- `TexSlot4_Specular`
- `TexSlot6_DetailAux`
- `TexSlot8_Height`
- `TexSlot9_BaseColorSecondary`
- `TexSlot10_SpecularSecondary`
- `TexSlot11_HeightSecondary`
- `TexSlot12_BlendMask`
- `TexSlot13_DetailSecondary`
- `TexSlot17_SubsurfaceMask`

## Layer

- `TexSlot1_BaseColor`
- `TexSlot2_NormalGloss`

## LayerBlend_V2

- `TexSlot3_NormalGloss`
- `TexSlot11_WearMask`
- `TexSlot12_BlendMask`
- `TexSlot13_HalControl`

## Eye

- `TexSlot1_BaseColor`
- `TexSlot2_IrisNormal`
- `TexSlot3_CorneaNormal`
- `TexSlot8_Height`

## Hair

- `TexSlot1_StrandColor`

## HairPBR

- `TexSlot1_OpacityMask`
- `TexSlot4_IdMap`

## HumanSkin_V2

- `TexSlot1_BaseColor`
- `TexSlot2_NormalGloss`
- `TexSlot3_WrinkleColor`
- `TexSlot4_Specular`
- `TexSlot6_SkinMicroDetail`
- `TexSlot8_WrinkleNormal`
- `TexSlot11_WrinkleMask`
- `TexSlot12_Transmission`

## MeshDecal

- `TexSlot1_DecalSource`
- `TexSlot2_Specular`
- `TexSlot3_NormalGloss`
- `TexSlot4_Height`
- `TexSlot5_BreakupMask`
- `TexSlot6_TintMask`
- `TexSlot7_StencilSource`
- `TexSlot8_GrimeBreakup`

Stencil note:

- when `TexSlot7` resolves to `$TintPaletteDecal`, the stencil also needs to route into the tint node-group path, not only the decal surface path

## UIPlane

- `TexSlot9_ScreenSource`
- `TexSlot17_PixelLayout`

## Monitor

- `TexSlot1_BaseColor`

Compatibility note:

- the exporter now preserves `Monitor` as its own shader family in sidecars; the current bundled Blender library still reuses the `Unknown` contract group for runtime wiring until a dedicated `Monitor` node group exists

## Non-Texture Inputs

Examples:

- `Param_FarGlowStartDistance`
- `Param_PomDisplacement`
- `Palette_Primary`
- `Palette_Glass`
- `Virtual_RenderToTexture`
- `Virtual_TintPaletteDecal`

## Current Status

These rules are now implemented in `starbreaker_addon.contract_naming` and consumed by the generated seed contract.

They are still authoring rules, not final runtime guarantees. The `.blend` library implementation may still split large families such as `Illum` into more than one top-level group while preserving these input names where the slots remain applicable.