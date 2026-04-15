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
- child attachment relationships via `parent_entity_name`, `parent_node_name`, `offset_position`, `offset_rotation`, and `no_rotation`
- interior container transforms, placement records, and exported light data
- material sidecar and palette references for every scene instance

## Material Sidecars

Each `*.materials.json` sidecar preserves:

- source material path and geometry path
- per-submaterial name, shader, shader family, and activation state
- decoded feature flags from `StringGenMask`
- direct texture-slot inventory with semantic roles, virtual-input flags, source paths, and exported texture paths
- public params as structured JSON values where simple coercion is safe
- layer manifests including source material paths, tint data, palette routing, UV tiling, and exported layer texture references
- material-set identity and palette-routing metadata
- variant-membership hints for palette-routed and layered materials

## Palette And Livery Rules

- Shared palettes are emitted once in `Packages/<package name>/palettes.json` and referenced everywhere else by `palette_id`.
- Material sidecars describe palette routing, but scene instances choose the concrete shared palette.
- `Packages/<package name>/liveries.json` groups entity and material usage by shared palette identity so Blender-side tooling can switch palettes centrally.

## Path Rules

- Source game paths are normalized to forward slashes and kept beneath canonical `Data/...` paths rooted at the export directory.
- Case is canonicalized from the actual P4k entry when possible so `Objects` and `objects` do not create duplicate export trees.
- Canonical textures preserve the original game-relative location whenever a direct source texture exists.
- Generated mesh and sidecar paths remain stable for the same source geometry or material path.