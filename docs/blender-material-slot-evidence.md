# Blender Material Slot Evidence

## Purpose

This note records the current slot evidence for every shader family found in `StarBreaker/docs/blender-shader-family-inventory.json`.

The goal is to keep future node-group socket naming tied to verified `.mtl` usage plus exported sidecar role labels, instead of inferring semantics from the current fallback graph alone.

## Method

Two evidence sources were combined:

- exported `*.materials.json` sidecars across `ships/Data`, aggregated by shader family, slot number, and declared texture role
- representative `.mtl` files inspected with the StarBreaker MCP material summary tools

Representative `.mtl` audit set:

- `Data/Objects/Spaceships/Ships/ARGO/MOLE/argo_mole_exterior.mtl`
- `Data/Objects/Spaceships/Ships/ARGO/MOLE/argo_mole_interior.mtl`
- `Data/Objects/Spaceships/Ships/ARGO/MOLE/Argo_mole_bag.mtl`
- `Data/Objects/Spaceships/Ships/ARGO/SRV/Interior/argo_srv_interior.mtl`
- `Data/Materials/vehicles/components/component_master_01.mtl`
- `Data/Materials/UI/ui_general_solid_engineering.mtl`
- `Data/Objects/Spaceships/Ships/AEGS/Idris_Frigate/interior/ui/ui_screen_16x9-Large-Generic01.mtl`

The slot meanings below are treated as verified where sidecar role labels, `.mtl` filenames, and cross-ship examples line up. The remaining open questions are product-shape decisions, not raw slot discovery gaps.

## Family Evidence

## DisplayScreen

Verified examples:

- MOLE exterior and interior RTT screens
- SRV `RTT_Screen` and `Screen_Flash`
- RSI interior `RTT_Screen`
- building-set `screen_rtt`
- Idris `Flash`

Observed slot map:

- `TexSlot2`: `normal_gloss`
  Verified by `*_ddna` textures such as `glass_scratched_a_ddna.tif` and `ui_screen_16x9_ships_ddna.tif`.
- `TexSlot3`: `normal_gloss`
  Present in wider screen variants such as RAFT `Screen_flash`, where it carries crack/laminate `ddna` data.
- `TexSlot6`: `screen_surface_mask`
  Seen in RSI and building-set screen sidecars.
  The audited files use wear-like glass textures such as `glass_canopy_wear.tif`, so this is a screen-surface mask or wear input rather than a pure binary mask.
- `TexSlot9`: `screen_source`
  Verified by `$RenderToTexture` in MOLE, SRV, UI, and Idris screen materials.
  One RSI `Decal_Glow_Linked` variant instead uses a static diffuse/glow texture here, so the slot is overloaded between live RTT input and a fallback static screen plate.
- `TexSlot10`: `pattern_mask`
  Seen in wider screen variants such as `Screen_flash` and `screen`.
- `TexSlot11`: `dirt`
  Verified by dirt-like filenames such as `glass_screen_1_a_dirt_diff.tif`.
- `TexSlot15`: `condensation_normal`
  Cross-ship variants use `glass_screen_1_a_condensation_ddna.tif`, confirming it as a condensation normal/detail input.
- `TexSlot16`: `pattern_mask`
  Verified by filenames such as `displayscreen_mask_1_a_mask.tif`.
- `TexSlot17`: `screen_pixel_layout`
  Verified by RSI interior `RTT_Screen` and `RTT_Hud` sidecars.

Contract note:

- `DisplayScreen` needs a required RTT path plus optional mask, dirt, normal, and pixel-layout inputs.

## GlassPBR

Verified examples:

- MOLE exterior `glass_refractive` and `int_glass_cockpit`
- MOLE interior canopy and frosted glass
- SRV cockpit and frosted variants
- MOLE bag glass variants
- component master `Glass`

Observed slot map:

- `TexSlot2`: `normal_gloss`
  Verified by `*_ddna` filenames in frosted and patterned glass variants.
- `TexSlot4`: `tint_color`
  Rare outlier that resolves to glass tint/base-color data in the audited quantum-drive glass variant `qdrv_just_smal_glass_b_tint_diff.tif`.
- `TexSlot6`: `wear_gloss`
  Verified by gloss and canopy-wear style filenames such as `glass_canopy_gloss.tif` and `glass_canopy_wear.tif`.
- `TexSlot11`: `dirt`
  Verified by filenames such as `glass_canopy_dirt.tif`, `glass_int_dirt_diff.tif`, and similar dirt overlays.
- `TexSlot15`: `condensation_normal`
  Verified by 890 Jump `glass_condensation`, which uses `glass_screen_1_b_condensation_ddna.tif`.
- `TexSlot16`: `pattern_mask`
  Verified by SRV cockpit/frosted variants that introduce an extra mask texture.

Contract note:

- `GlassPBR` cannot use one fixed slot set. Some materials have no textures at all, while others need optional normal, wear/gloss, dirt, and mask inputs.

## HardSurface

Verified examples:

- MOLE exterior paint and stripe materials
- MOLE interior paints
- SRV interior trims, tile panels, and paint families
- component master trims

Observed slot map:

- `TexSlot1`: `base_color`
  Verified by repeated `paint_largescale_diff.tif` and similar diffuse/base-color patterns.
- `TexSlot3`: `normal_gloss`
  Verified by filenames such as `paint_largescale_ddn.tif`, `trims_2_piping_ddna.tif`, and leather normal maps.
- `TexSlot6`: `displacement`
  The audited SRV `Trim_piping` case uses `TexSlot6=trims_2_piping_displ.tif`, so this resolves as a displacement-style auxiliary map for POM hard-surface variants.
- `TexSlot10`: `iridescence_color`
  Verified by Talon `metal_iridescent` and `metal_iridescent_dark`, which use `espr_irridescence_green_purple_diff.tif`.
- `TexSlot14`: `emissive`
  Wider RSI master variants resolve this slot to emissive/glow content such as `rsi_grav_plates_glow.tif`, `greeble_machine_b_emis.tif`, and `core_plating_emissive_b.tif`.

Shape notes:

- MOLE-style two-layer paints often expose `TexSlot1` and `TexSlot3` only.
- Some HardSurface families are one-layer or POM-enabled with no explicit textures at all.
- Palette usage is real in this family and appears on primary, secondary, and tertiary routed materials.

Contract note:

- HardSurface needs family-specific optional inputs and likely sub-variants for simple one-layer trims versus palette-driven paint stacks.

## Illum

Verified examples:

- MOLE exterior and interior trims, decals, glows, rubber, and structure materials
- SRV interior trims, padding, rubber, damage, glow, and button materials
- MOLE bag industrial, decal, glass-adjacent, hose, and trim materials
- component master glow/decal families

Observed slot map:

- `TexSlot1`: `base_color`
  Verified throughout the family by diffuse and color textures.
- `TexSlot2`: `normal_gloss`
  Verified by `*_ddna` filenames.
- `TexSlot3`: `normal_gloss`
  Common in blend-heavy or layered variants where a second normal-style texture is present.
- `TexSlot4`: `specular_support`
  Verified by repeated `*_spec.tif` usage.
- `TexSlot6`: `detail_aux`
  Appears in detail-heavy, leather, BlackGlass, and pipe cases.
  Cross-ship filenames such as `metal_b_detail.tif`, `fabric_detail.tif`, `paintedmetal01_udm.tif`, and `rsi_int_glass_patterned_detail_square.tif` resolve this as a detail or pattern-support channel.
- `TexSlot8`: `height`
  Verified by repeated displacement files in POM materials.
- `TexSlot9`: `alternate_base_color` or `decal_sheet`
  Verified in blend-heavy materials where a second diffuse-like sheet is used.
- `TexSlot10`: `specular_secondary`
  Often paired with `TexSlot9` and `TexSlot12` in blend materials.
  Cross-ship files such as `tars_trim_1_a_spec.tif` and `metal_panels_spec.tif` resolve this as a secondary specular-support slot for blend variants.
- `TexSlot11`: `height_secondary`
  Appears in rubber, trim, and hose variants.
  Cross-ship files such as `tars_trim_1_a_displ.tif`, `drill_hose_disp.tif`, and `aegs_int_rubber_industrial_grip_displ.tif` resolve this as a secondary displacement or height slot in blend-heavy Illum variants.
- `TexSlot12`: `blend_mask`
  Verified as a blend or wear-mask slot in several decal and layered materials.
  Cross-ship files include `tars_trim_1_a_blend.tif`, `metal_panels_blnd.tif`, and `universal_shared_1_b_wear.tif`.
- `TexSlot13`: `detail_secondary`
  Appears with detail-repeat style files in leather, hose, and painted metal variants.
  Cross-ship files such as `metal_scratches-01_detail.tif`, `metal_b_detail.tif`, `fabric_detail.tif`, and `leather_base_delt_detail.dds` resolve this as a detail or UDM-style auxiliary channel.
- `TexSlot17`: `subsurface_mask`
  Verified by creature-organic `marok_meat_sss.tif` in the marok organic sample.

Shape notes:

- `TexSlot1/2/4/8` is the canonical POM/decal structure pattern.
- `TexSlot1` alone is common for simple glow and decal-diffuse cases.
- `TexSlot1/2/3/9/10/12` and `TexSlot1/2/3/6/9/12/13` show up in blend-heavy upholstery, rubber, hose, and industrial surface cases.

Contract note:

- `Illum` is too broad for a single flat contract without variants or grouped slot shapes.
  It should be split by actual slot pattern when the node-group interfaces are finalized.

## LayerBlend_V2

Verified examples:

- component master layered paint, plastic, metal, and rubber entries
- wider sidecar inventory across components, relays, weapons, and props

Observed slot map:

- `TexSlot3`: `normal_gloss`
  Verified by repeated `*_ddna` textures.
- `TexSlot11`: `wear_mask`
  Verified by wear-mask files such as `universal_shared_1_a_wear.dds` and `cooler_just_wear.tif`.
- `TexSlot12`: `blend_mask`
  Verified by blend-mask files such as `universal_shared_1_b_blend.tif` and `generic_blendmap_diff.tif`.
- `TexSlot13`: `hal_control`
  Verified by HAL-control files such as `universal_shared_1_a_hal.dds`, `cooler_just_hal.tif`, and `qdrvl_just_smal_hal.tif`.

Shape notes:

- Palette-routed primary, secondary, and tertiary variants are real in this family.
- Layer counts range far beyond the simpler HardSurface paint cases.

Contract note:

- `LayerBlend_V2` needs its own family contract and should not inherit the HardSurface assumptions.

## MeshDecal

Verified examples:

- MOLE exterior `emblems`, `RTT_Text_To_Decal`, and stencil decals
- SRV `RTT_Text_To_Decal`, POM decals, emblem tint, and glow-unlinked variants
- component master decal families

Observed slot map:

- `TexSlot1`: `base_color` or `render_to_texture`
  Verified by both diffuse decal textures and `$RenderToTexture` in RTT decal cases.
- `TexSlot2`: `specular`
  Present in POM decal variants.
  Cross-ship files such as `Components_master_pom_spec.tif`, `rsi_decals_base_a_spec.tif`, and `argo_pom_spec.tif` resolve this as a specular-support slot.
- `TexSlot3`: `normal_gloss`
  Verified by `*_ddna` decal textures.
- `TexSlot4`: `height`
  Verified by displacement files in POM decals.
- `TexSlot5`: `breakup_mask`
  Used in diffuse decal variants such as `Decal_DIFF`.
  Cross-ship files such as `component_decal_wear.tif` and grime masks like `ship_mf_genericGrime_a.dds` resolve this as breakup or wear support.
- `TexSlot6`: `tint_mask`
  Seen in POM decal variants such as `Decal_POM` and `Decal_Stitching`.
  Cross-ship files such as `Components_master_pom_tint.tif` and `argo_atls_pattern_blendmode_mask.dds` resolve this as an auxiliary tint or blend-mode mask slot.
- `TexSlot7`: `stencil` or `tint_palette_decal`
  Verified directly by `$TintPaletteDecal` and stencil decal cases.
  Vulture `Ext_livery_01` confirms that exterior livery stencils also use `TexSlot7=$TintPaletteDecal`, so stencil decals need to feed the tint node group as well as the decal node group.
- `TexSlot8`: `breakup`
  Verified by MOLE `emblems`, where grime/breakup support is paired with stencil tinting.

Contract note:

- `MeshDecal` needs explicit support for RTT, stencil tint, POM height, and breakup maps.

## UIMesh

Verified examples:

- SRV interior `Panel_Tile_Metal_A`

Raw XML follow-up:

- Odyssey skull-helmet `UIMesh` materials such as `emissiveglass_m`, `skull1_m`, and `skull3_m` add `TexSlot1` inputs with animated `TexMod` state and, in one case, a `VertexDeform` block.

Observed slot map:

- current exported inventory: no texture slots were observed
- raw XML follow-up: `TexSlot1` can be present on effect-style `UIMesh` materials and may carry animated `TexMod` data rather than a static surface texture

Contract note:

- Treat `UIMesh` as a parameter-heavy family, but not a purely parameter-only one. The contract should leave room for optional `TexSlot1`, animated `TexMod`, and `VertexDeform` preservation even if the first Blender node group remains effect-focused.

## UIPlane

Verified examples:

- MOLE and SRV HUD RTT planes
- `ui_general_solid_engineering.mtl`
- RSI interior `RTT_Hud`

Observed slot map:

- `TexSlot9`: `render_to_texture`
  Verified directly by `$RenderToTexture`.
- `TexSlot17`: `screen_pixel_layout`
  Verified by RSI interior RTT HUD sidecars.

Contract note:

- `UIPlane` is simpler than `DisplayScreen`: it needs RTT support and an optional pixel-layout input.

## Additional Families Outside The Current Exported Fixture Set

These families were not present in the current `ships/Data` sidecar inventory, but direct `.mtl` inspection already provides enough evidence to seed their future contracts.

## Layer

Verified example:

- `Data/Materials/Layers/organic/skin_01.mtl`

Observed slot map:

- `TexSlot1`: `base_color`
- `TexSlot2`: `normal_gloss`

## Eye

Verified example:

- `Data/Objects/Characters/Human/heads/male/pu/silas/silas_t0_material.mtl`

Observed slot map:

- `TexSlot1`: `base_color`
- `TexSlot2`: `iris_normal`
- `TexSlot3`: `cornea_normal`
- `TexSlot8`: `height`

## Hair

Verified examples:

- `Data/Objects/Characters/Human/heads/shared/hair/f_hat_hair_01.mtl`
- `Data/Objects/Characters/Human/heads/male/pu/silas/silas_t0_material.mtl`

Observed slot map:

- `TexSlot1`: `strand_color`

## HairPBR

Verified example:

- `Data/Objects/Characters/Creatures/marok/marok_bird_m_hair_01_02.mtl`

Observed slot map:

- `TexSlot1`: `opacity_mask`
- `TexSlot4`: `id_map`

## HumanSkin_V2

Verified example:

- `Data/Objects/Characters/Human/heads/male/pu/silas/silas_t0_material.mtl`

Observed slot map:

- `TexSlot1`: `base_color`
- `TexSlot2`: `normal_gloss`
- `TexSlot3`: `wrinkle_color`
- `TexSlot4`: `specular`
- `TexSlot6`: `skin_micro_detail`
- `TexSlot8`: `wrinkle_normal`
- `TexSlot11`: `wrinkle_mask`
- `TexSlot12`: `transmission`

## Organic

Verified examples:

- `Data/Objects/Architecture/planetary/asteroids/common/ast_01.mtl`
- `Data/Objects/mining/crystals/large_crystal_01.mtl`
- `Data/Objects/mining/crystals/small_crystal_cluster.mtl`

Observed slot map:

- `TexSlot1`: `blend_mask`
  Verified by the asteroid and crystal height-blend cases where the visible surface response is assembled from referenced layers rather than a direct diffuse sheet.
- `TexSlot2`: `normal_gloss`
  Verified by recurring `*_ddna` inputs on both asteroid and crystal variants.
- `TexSlot3`: `base_color`
  Present on the asteroid branch, where the visible diffuse input is separate from the blend mask in `TexSlot1`.
- `TexSlot8`: `height`
  Present on displacement or tessellation-heavy variants when an explicit height-style source is authored.
- `TexSlot17`: `subsurface` or opacity-like support
  Rare slot that remains family-specific and should stay explicit rather than being forced into a generic slot-number rule.

Contract note:

- `Organic` is another direct counterexample to any global `TexSlot1 = base color` assumption.
- The current exporter-side semantic mapping already preserves the important split between `TexSlot1` blend-mask behavior, `TexSlot2` ddna data, and `TexSlot3` visible diffuse data when present.
- Reconstruction risk for this family is now mostly about broader sampling of its layer-driven and tessellation-heavy variants, not a missing first-pass slot audit.

## Monitor

Verified examples:

- MOLE bag `GUI Screens`
- component master `RTT_Screen`
- 890 Jump `screen_glow`, `screens_glow`, and `glow_holographic_decals`

Observed slot map:

- `TexSlot1`: `base_color`
  Verified in `GUI Screens` with `temp_displays_diff.tif`.
- component master also contains a `Monitor` shader case with no explicit texture slots.

Contract note:

- The exporter now preserves this family as `Monitor` in the shipped sidecar fixtures.
- The bundled Blender library still routes `Monitor` through the current `SB_Unknown_v1` contract group until a dedicated `Monitor` node group exists.

## Contract Consequences

- Do not assign one universal semantic meaning to a `TexSlotN` across families.
- The safe contract boundary is the shader family, not the slot number.
- `DisplayScreen`, `UIPlane`, and `Monitor` all need RTT-aware handling, but they do not share the same optional inputs.
- `Illum` and `HardSurface` both need variant-aware contracts because their slot shapes differ widely inside a single family.
- `MeshDecal` and `LayerBlend_V2` have enough verified structure to justify dedicated top-level groups immediately.
- The contract should keep the original `TexSlotN` prefix even when the semantic suffix is now evidence-backed.

## Current Status

The current inventory-wide shader-family audit is complete.

Open follow-up items:

- decide whether `Monitor` remains separate from `DisplayScreen`
- split `Illum` and likely parts of `HardSurface` into explicit contract variants instead of a single monolithic group interface