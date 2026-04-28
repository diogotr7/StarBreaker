---
title: Quick Start
description: Export your first ship to glTF.
---

Point StarBreaker at your Star Citizen `Data.p4k` and export an entity:

```bash
SC_DATA_P4K="C:/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k" \
  starbreaker entity export AEGS_Avenger ./out --kind decomposed
```

This produces a `scene.json` plus a `Packages/` directory with reusable
geometry and material sidecars. See [Exporting ships](/wiki/exporting-ships/)
for the full workflow.
