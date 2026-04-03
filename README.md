# StarBreaker

[Download latest release](https://github.com/diogotr7/StarBreaker/releases/latest)

Toolkit for reading and extracting Star Citizen game files. Handles the P4k archive, DataCore database, CryEngine assets, Wwise audio, and character files.

There is also a legacy C# version on the [`master`](../../tree/master) branch.

## Crates

| Crate                  | Description                                                                                                   |
| ---------------------- | ------------------------------------------------------------------------------------------------------------- |
| `starbreaker-p4k`      | Read and extract files from `Data.p4k` archives                                                               |
| `starbreaker-datacore` | Parse the DataCore binary database (`.dcb`), query records, export to JSON/XML                                |
| `starbreaker-cryxml`   | Decode CryEngine binary XML                                                                                   |
| `starbreaker-chunks`   | Parse CryEngine chunk files (`.cgf`, `.cga`, `.skin`, `.chr`)                                                 |
| `starbreaker-gltf`     | Export CryEngine meshes to glTF/GLB, including materials and textures                                         |
| `starbreaker-dds`      | Read DDS textures and decode block-compressed formats                                                         |
| `starbreaker-chf`      | Read/write character head files (`.chf`) — the format behind [starchar](https://github.com/diogotr7/starchar) |
| `starbreaker-wwise`    | Parse Wwise soundbank (`.bnk`) files and resolve audio event chains                                           |
| `starbreaker-wem`      | Decode Wwise audio (`.wem`) to Ogg Vorbis                                                                     |
| `starbreaker-common`   | Shared types, binary reader, game install discovery                                                           |

## CLI

```
starbreaker <command> [options]
```

Commands: `p4k`, `dcb`, `entity`, `skin`, `socpak`, `cryxml`, `dds`, `glb`, `chf`, `wwise`.

Run `starbreaker <command> --help` for details.

<details>
<summary><b>P4k extraction</b> — extract and convert files from the game archive</summary>

### P4k extraction

```sh
# List files matching a pattern
starbreaker p4k list --filter '**/*.mtl'

# Extract everything under a ship's directory
starbreaker p4k extract -o ./out --filter '**/AEGS/Gladius/**'
```

#### Filtering

Paths inside the P4k look like `Data/Objects/Spaceships/Ships/AEGS/Gladius/texture.dds`.

| Pattern                 | Matches                                                |
| ----------------------- | ------------------------------------------------------ |
| `**/*.xml`              | All `.xml` files anywhere                              |
| `**/AEGS/Gladius/**`    | Everything under any `Gladius/` directory (all depths) |
| `**/AEGS/Gladius/*.mtl` | Only `.mtl` files directly in `Gladius/`               |
| `**/ships/**`           | Everything under any `ships/` directory                |
| `**/*aurora*`           | Any file with `aurora` in the name                     |

`*` matches within a single directory. `**` matches across any number of directories.

#### Converting files during extraction

The `--convert` flag transforms files as they're extracted. It's repeatable — combine multiple converters in one pass:

```sh
# Decode binary XML files (.mtl, .chrparams, .xml, etc.) to readable XML
starbreaker p4k extract -o ./out --filter '**/*.mtl' --convert cryxml

# Convert DDS textures to PNG (merges split mip siblings automatically)
starbreaker p4k extract -o ./out --filter '**/*.dds' --convert dds-png

# Get merged DDS files (for modding tools that need DDS format)
starbreaker p4k extract -o ./out --filter '**/*.dds' --convert dds-merge

# Everything at once — binary XML decoded, textures as both PNG and merged DDS
starbreaker p4k extract -o ./out --filter '**/AEGS/Gladius/**' --convert all
```

| Converter   | What it does                                                       |
| ----------- | ------------------------------------------------------------------ |
| `cryxml`    | CryXmlB binary to readable XML                                     |
| `dds-png`   | DDS textures to PNG (merges split `.dds.1`/`.dds.2` mips from P4k) |
| `dds-merge` | DDS split mips to single merged DDS file                           |
| `all`       | All of the above                                                   |

Example output:

```
Extracting 329 files...
[CONVERT] CryXML→XML, DDS→PNG, DDS→merged DDS
Pre-creating directories... 31 directories created.
[START] all cores threads
[DONE] Extracted 329/329 files in 0.6s
[DONE] Total: 91.3 MB | Avg throughput: 151.3 MB/s
[DONE] 196 files converted
```

</details>

<details>
<summary><b>Entity export</b> — export ships and vehicles to GLB</summary>

### Entity export

Export a ship or vehicle as a GLB file with full loadout assembly — geometry, skeleton, materials, and textures:

```sh
# Export a Gladius with textures at 1/4 resolution
starbreaker entity export Gladius gladius.glb

# Lightweight export — no textures, lower LOD
starbreaker entity export Gladius gladius.glb --no-textures --no-normals --lod 2

# Just inspect the loadout tree
starbreaker entity export Gladius --dump-hierarchy
```

```
Found 67 candidates, using shortest match: EntityClassDefinition.AEGS_Gladius

Loadout tree for EntityClassDefinition.AEGS_Gladius:
  G hardpoint_seat_access -> AEGS_Gladius_SCItem_SeatAccess
  G hardpoint_seat_pilot -> AEGS_Gladius_SCItem_Seat_Pilot
  G hardpoint_power_plant -> POWR_AEGS_S01_Regulus_SCItem
  G Hardpoint_cooler_left -> COOL_AEGS_S01_Bracer_SCItem
  G hardpoint_shield_generator_left -> SHLD_GODI_S01_AllStop_SCItem
  ...

GLB size: 37498672 bytes
Written to gladius.glb
```

</details>

<details>
<summary><b>DataCore queries</b> — query and export the game database</summary>

### DataCore queries

The DataCore is Star Citizen's binary database containing all game definitions — ships, items, weapons, missions, etc.

#### Query specific properties

Use `dcb query` to extract values from records using dot-separated property paths:

```sh
# Get geometry paths for all Gladius entities
starbreaker dcb query \
  'EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path' \
  --filter '*Gladius'
```

```
13 record(s) matched.
EntityClassDefinition.AEGS_Gladius    Objects\Spaceships\Ships\AEGS\Gladius\AEGS_Gladius.cga
EntityClassDefinition.Weapon_Rack_AEGS_Gladius    Objects/Spaceships/Ships/AEGS/Gladius/AEGS_Gladius_Gunrack.cga
EntityClassDefinition.Flair_Model_Aegis_Gladius   Objects/brush/planet/uee/flair_objects/model_spaceships/aegs_gladius/flair_aegs_gladius.cgf
...
```

```sh
# Get item types for Gladius components
starbreaker dcb query \
  'EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Type' \
  --filter '*AEGS_Gladius'
```

```
EntityClassDefinition.AEGS_Gladius    NOITEM_Vehicle
EntityClassDefinition.INTK_AEGS_Gladius   FuelIntake
EntityClassDefinition.HTNK_AEGS_Gladius   FuelTank
EntityClassDefinition.ARMR_AEGS_Gladius   Armor
...
```

```sh
# Dump full records as JSON (omit property path)
starbreaker dcb query EntityClassDefinition --filter '*AEGS_Gladius'
```

#### Bulk export

```sh
# Export all 57k+ records to JSON
starbreaker dcb extract --format json -o ./dcb_out

# Export only Gladius-related records
starbreaker dcb extract --format json -o ./dcb_out --filter '**/*gladius*'

# Use a .dcb file directly instead of extracting from P4k
starbreaker dcb extract --dcb Game2.dcb --format json -o ./dcb_out
```

</details>

<details>
<summary><b>DDS textures</b> — inspect, convert, and merge textures</summary>

### DDS textures

`dds info` and `dds decode` can read directly from the P4k archive. The other commands work on extracted files.

```sh
# Show texture metadata (from P4k — no extraction needed)
starbreaker dds info Objects/Spaceships/Ships/AEGS/Gladius/textures/cockpit_details_diff.dds
```

```
Format:     BC1Unorm
Dimensions: 2048 x 2048
Cubemap:    false
Mip levels: 10 present (header declares 10)

Mip         Width     Height         Size
------------------------------------------
0            2048       2048      2.0 MiB
1            1024       1024    512.0 KiB
2             512        512    128.0 KiB
...
```

```sh
# Decode a DDS from P4k directly to PNG
starbreaker dds decode Objects/path/to/texture.dds -o texture.png

# Convert an extracted DDS to PNG
starbreaker dds to-png texture.dds

# Batch convert a directory of extracted DDS files
starbreaker dds to-png-all -i ./extracted/textures -o ./png_out

# Merge split DDS mips (.dds + .dds.1, .dds.2, ...) into a single file
starbreaker dds merge texture.dds
```

</details>

<details>
<summary><b>CryXML conversion</b> — decode binary XML to readable text</summary>

### CryXML conversion

Convert CryEngine binary XML files to readable XML:

```sh
# Single file
starbreaker cryxml convert material.mtl

# Batch convert a directory
starbreaker cryxml convert-all -i ./extracted -o ./xml_out --filter '*.mtl'
```

</details>

<details>
<summary><b>Wwise audio</b> — extract and decode game audio</summary>

### Wwise audio

Extract and decode audio from Wwise soundbanks:

```sh
# List events in a soundbank
starbreaker wwise events ship_sounds.bnk

# Decode a .wem file to .ogg
starbreaker wwise decode-wem audio.wem -o audio.ogg
```

</details>

## App

A Tauri desktop app lives in `app/`. React + TypeScript frontend, Rust backend. Provides a GUI for browsing P4k contents, DataCore records, exporting 3D models, and playing audio.

## Building

Requires Rust (edition 2024). For the Tauri app, you also need Node.js.

```sh
# CLI only
cargo build --release -p starbreaker

# Tauri app
cd app && npm install && npm run tauri build
```

<details>
<summary><b>Game file discovery</b></summary>

The CLI and library auto-detect your Star Citizen install under `C:\Program Files\Roberts Space Industries\StarCitizen\`, scanning LIVE/PTU/EPTU channels by modification time.

To override, copy `.cargo/config.toml.example` to `.cargo/config.toml` and set your paths:

```toml
[env]
SC_DATA_P4K = "D:\\Games\\StarCitizen\\LIVE\\Data.p4k"
```

Or set the `SC_DATA_P4K` / `SC_EXE` environment variables directly.

</details>

## Acknowledgements

Projects without which StarBreaker would not have been possible

- **[unp4k](https://github.com/dolkensp/unp4k)** — P4k file support, DataCore decoding, CryXML
- **[scdatatools](https://gitlab.com/scmodding/frameworks/scdatatools)** — Socpak decoding and how to assemble the various parts of an entity together
- **[Blender-Tools](https://github.com/scorg-tools/Blender-Tools)** — Material handling, more assembling of entity parts together
- **[CryEngine-Converter](https://github.com/Markemp/Cryengine-Converter)** — Geometry formats, CryChunk, #ivo support
