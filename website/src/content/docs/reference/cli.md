---
title: CLI Reference
description: starbreaker command-line interface.
---

Run `starbreaker --help` or `starbreaker <command> --help` for the
authoritative usage. Global options: `-v` / `-vv` for debug / trace logging,
`--mem-cap <MB>` to abort if memory use exceeds a cap.

## Top-level commands

- `p4k` — list and extract files from `Data.p4k`, optionally converting them
- `dcb` — query and extract DataCore records (JSON / XML)
- `entity` — export an entity (ship, vehicle, prop, armour piece) to GLB or a
  decomposed package; print its loadout; validate a decomposed export
- `skin` — export or inspect individual meshes
- `socpak` — export object containers (interiors) to GLB or a decomposed package
- `cryxml` — convert CryEngine binary XML to text XML
- `dds` — inspect, decode and merge DDS textures
- `glb` — inspect a GLB file
- `chf` — convert character head files to/from JSON; export a character to GLB
  or a decomposed package
- `armour` — export armour and clothing worn by a character
- `wwise` — inspect soundbanks, trace events, extract (and decode) audio
- `nmc` — dump the node-mesh-combo chunk of a `.cga` / `.cgf`
- `ui` — re-render ship-screen UI images from a decomposed `scene.json`
- `animation` — report why exported animation clips fail to bind
- `riglogic` — dump RigLogic facial-rig data as JSON
