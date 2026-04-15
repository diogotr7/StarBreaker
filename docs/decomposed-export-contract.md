# Decomposed Export Contract

Phase 3 Mode 2 export writes a reusable package rooted at a caller-selected directory.

Within that package root:

- `scene.json` describes the root entity, child attachments, interior placements, light definitions, and relative asset references.
- `meshes/` contains reusable mesh-only `.glb` assets. Source geometry paths are normalized under this directory, for example `meshes/Data/Objects/.../ship.glb`.
- `textures/` contains canonical exported `.png` files on normalized game-relative paths. Derived textures use explicit suffixes such as `.normal.png` and `.roughness.png`.
- `materials/` contains structured material sidecars on normalized game-relative paths, for example `materials/Data/Objects/.../ship.materials.json`.
- `palettes/palettes.json` contains shared palette identities that scene instances reference by `palette_id`.
- `liveries/liveries.json` groups scene and material usage by shared palette identity.

## Scene Manifest

`scene.json` includes:

- the root package rule: all paths are relative to the selected package root
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

- Shared palettes are emitted once in `palettes/palettes.json` and referenced everywhere else by `palette_id`.
- Material sidecars describe palette routing, but scene instances choose the concrete shared palette.
- `liveries/liveries.json` groups entity and material usage by shared palette identity so Blender-side tooling can switch palettes centrally.

## Path Rules

- Source game paths are normalized to forward slashes and kept beneath their package category root.
- Canonical textures preserve the original game-relative location whenever a direct source texture exists.
- Generated mesh and sidecar paths remain stable for the same source geometry or material path.