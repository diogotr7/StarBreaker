# StarBreaker Blender Add-on

This add-on imports Phase 3 decomposed StarBreaker exports into Blender and rebuilds reusable, template-driven materials from the preserved Mode 2 contract.

## What It Imports

- `Packages/<package>/scene.json`: scene graph, attachment relationships, interior placements, and lights
- `Packages/<package>/palettes.json`: shared palette records referenced by scene instances
- `Packages/<package>/liveries.json`: palette and livery usage grouped by entity and material identity
- `Data/.../*.materials.json`: material sidecars paired with exported mesh assets

The material sidecars preserve shader family, decoded feature flags, semantic texture slots, layer manifests, palette routing, and public params. In the Blender add-on, materials are shared by semantic identity derived from that preserved contract plus the selected palette, not by sidecar filename alone.

## Implemented Features

- decomposed package import from `scene.json`
- scene and attachment reconstruction with shared mesh data for repeated assets
- compact template library for physical surfaces, layered wear, decals or stencil, POM-style fallback, screens or HUDs, biology, hair, and effects
- palette and livery switching driven by the preserved metadata
- Cycles-first material defaults with Eevee-safe fallbacks
- metadata inspection and raw JSON dumping for imported objects and materials

## Blender Usage

Enable the add-on module from `starbreaker_addon`, then use the `StarBreaker` panel in the 3D View sidebar.

Typical flow:

1. Import a decomposed package by selecting `scene.json`.
2. Select any imported object under the package root.
3. Use `Apply Palette` or `Apply Livery` to switch preserved variants.
4. Use `Dump Metadata` to inspect the preserved scene-instance or material contract in Blender text datablocks.

## Validation

Pure-Python validation lives under `tests/` and runs against the checked-in ship fixtures.

Manual Blender validation is documented in `../../docs/StarBreaker/blender-addon-manual-validation.md`.
