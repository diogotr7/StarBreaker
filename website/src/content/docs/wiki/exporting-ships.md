---
title: Exporting Ships
description: Full workflow for exporting a Star Citizen ship.
---

StarBreaker exports a ship either as a single **GLB** or as a **decomposed**
package — a `scene.json` (and a ready-made `scene.blend`) describing the ship,
backed by a shared `Data/` tree of meshes, material sidecars and textures that
later exports to the same folder reuse. The decomposed package is what the
Blender addon works with.

## Pre-requisites

- A Star Citizen install (you supply your own `Data.p4k`). StarBreaker finds it
  in the standard install locations; set `SC_DATA_P4K` only to use a different
  archive.

## Command

```bash
starbreaker entity export AEGS_Gladius ./out --kind decomposed
```

The first argument is matched case-insensitively as a substring of the
entity's DataCore class name; when several match, the shortest name wins (the
CLI prints how many candidates it found). Run
`starbreaker entity loadout <name>` to check what a name resolves to.

Useful options: `--lod` (default `1`; `0` is the highest detail), `--mip`
(texture mip, default `2` = 1/4 resolution; `0` is full size) and `--materials`
(`none`, `colors`, `textures` (default) or `all`).

## Output layout

```
out/
├── Packages/
│   └── <ship>_LOD1_TEX2/
│       ├── scene.json      # entities, attachments, interiors, lights, animations
│       ├── scene.blend     # the assembled scene for Blender
│       ├── palettes.json   # palettes the scene instances reference
│       └── liveries.json   # palette / livery usage
└── Data/                   # shared .blend meshes, *.materials.json sidecars,
                            # and textures, at P4k-style paths
```

Pass the same output folder for every ship: it is the shared export root, not
a package folder.

## Importing into Blender

The Blender addon lives in the repository at `blender_addon/starbreaker_addon`
(it is not on the releases page). Install that folder as a Blender add-on,
enable it, then open `Packages/<ship>_LOD1_TEX2/scene.blend`. The addon
rebuilds the materials after the file loads; its controls (paint, palette,
livery, light states, animations) are in the **StarBreaker** tab of the 3D
View sidebar (N-panel).

## Troubleshooting

If meshes are missing or materials look wrong, see the
[material system](/wiki/material-system/) page — the most common cause is
a shader family that hasn't been mapped yet.
