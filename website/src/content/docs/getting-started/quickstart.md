---
title: Quick Start
description: Export your first ship.
---

Export an entity as a decomposed package (StarBreaker finds your `Data.p4k`
automatically):

```bash
starbreaker entity export AEGS_Gladius ./out --kind decomposed
```

This writes `out/Packages/<ship>_LOD1_TEX2/` (`scene.json`, `scene.blend`,
palettes and liveries) plus a shared `out/Data/` tree of meshes, material
sidecars and textures. Drop `--kind decomposed` and give a `.glb` path to get a
single glTF binary instead. See [Exporting ships](/wiki/exporting-ships/) for
the full workflow.
