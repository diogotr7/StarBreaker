---
title: Exporting Ships
description: Full workflow for exporting a Star Citizen ship to glTF.
---

StarBreaker's primary export format is the **decomposed** layout — a
`scene.json` describing the ship plus a reusable `Packages/` directory of
geometry and material sidecars. From there you can either render directly
or import into Blender.

## Pre-requisites

- A Star Citizen install (you supply your own `Data.p4k`)
- The `SC_DATA_P4K` environment variable pointing at it

## Command

```bash
starbreaker entity export AEGS_Avenger ./out --kind decomposed
```

The first argument is the entity's class name as it appears in DataCore
(e.g. `AEGS_Avenger`, `RSI_Constellation_Andromeda`).

## Output layout

```
out/
├── scene.json              # the ship: nodes, transforms, material refs
└── Packages/
    └── AEGS_Avenger/
        ├── geometry/       # .gltf / .bin chunks per mesh
        └── materials/      # one .json sidecar per material
```

## Importing into Blender

Install the Blender addon from the
[releases page](https://github.com/diogotr7/StarBreaker/releases/latest)
and use **File → Import → StarBreaker scene** to pick `scene.json`.

## Troubleshooting

If meshes are missing or materials look wrong, see the
[material system](/wiki/material-system/) page — the most common cause is
a shader family that hasn't been mapped yet.
