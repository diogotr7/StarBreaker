# StarBreaker Blender Add-on

This add-on works with decomposed StarBreaker exports in Blender (5.1): it rebuilds reusable, template-driven materials from the exported material contract and provides palette, livery, light-state and animation controls.

## What It Reads

- `Packages/<package>/scene.blend`: the assembled scene the exporter writes for Blender
- `Packages/<package>/scene.json`: scene graph, attachment relationships, interior placements, lights, animations, and character customization
- `Packages/<package>/palettes.json`: shared palette records referenced by scene instances
- `Packages/<package>/liveries.json`: palette and livery usage grouped by entity and material identity
- `Data/.../*.materials.json`: material sidecars paired with exported mesh assets

The material sidecars preserve shader family, decoded feature flags, semantic texture slots, layer manifests, palette routing, and public params. In the Blender add-on, materials are shared by semantic identity derived from that contract plus the selected palette, not by sidecar filename alone. The full contract is in `../docs/decomposed-export-contract.md`.

## Features

- material rebuild for ships, socpak interiors, characters and armour packages
- compact template library for physical surfaces, layered wear, decals or stencil, POM relief, screens or HUDs, skin, hair, and effects
- paint, palette and livery switching driven by the exported metadata
- light-state switching across the authored CryEngine states (default, auxiliary, emergency, cinematic)
- animation controls: snap a clip to its first or last frame, or insert it as an action
- Cycles-first material defaults with EEVEE-safe fallbacks
- metadata inspection and raw JSON dumping for imported objects and materials

## Blender Usage

Install the `starbreaker_addon` folder as a legacy add-on (copy it into Blender's user `scripts/addons/` folder, or zip it and use Install from Disk), enable it, then use the `StarBreaker` tab in the 3D View sidebar (N-panel).

Typical flow:

1. Export with `starbreaker entity export <name> <export root> --kind decomposed` (or `chf to-package` / `armour export --kind package`).
2. Open `Packages/<package>/scene.blend`. Materials rebuild automatically after the file loads (add-on preference "Refresh unloaded materials after file load"); **Refresh Materials** in the panel does it on demand.
3. Select any object under the package root.
4. Use **Apply Paint**, **Apply Palette** or **Apply Livery** to switch variants, the light-state buttons to switch lighting, and the animation list to pose or play clips.
5. Use **Dump Metadata** to inspect the scene-instance or material contract in Blender text datablocks.

## Material Node Layout

Generated material nodes are laid out from the `MaterialNodeLayout` defaults in `starbreaker_addon/runtime/importer/types.py` (exposed as `MATERIAL_NODE_LAYOUT`).

- `texture_x`, `texture_start_y`, `texture_vertical_step`, and `texture_width` control the imported image texture column.
- `primary_x` and `primary_y` control the main shader or group node position.
- `group_width` controls the width applied to shader-group nodes such as `SB_Illum_v1`.
- `output_x` and `output_y` control the Material Output node position.

Adjust those values if you want to retune the generated node spacing or alignment, then sync or reinstall the add-on before refreshing materials in Blender.

## Validation

The pure-Python suite under `tests/` stubs `bpy` and builds most of its inputs in the tests themselves (three manifest/template tests read `component_master_01_TEX0.materials.json` from a decomposed export at `../../ships/Data` and error without it); run it with `python3 -m unittest discover -s tests -q` from this folder. Agent and deploy guidance is in [AGENTS.md](AGENTS.md).
