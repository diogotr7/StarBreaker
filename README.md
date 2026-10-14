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
| `starbreaker-3d`       | Assemble entities, characters and interiors; export to GLB or decomposed `.blend` packages                    |
| `starbreaker-blend`    | Write Blender `.blend` library files (meshes, empties, lights, hierarchies)                                   |
| `starbreaker-dds`      | Read DDS textures and decode block-compressed formats                                                         |
| `starbreaker-chf`      | Read/write character head files (`.chf`) — the format behind [starchar](https://github.com/diogotr7/starchar) |
| `starbreaker-dna`      | Parse facial-rig DNA (`.dna`) files                                                                           |
| `starbreaker-gfx`      | Inspect GFx/SWF UI assets                                                                                     |
| `starbreaker-ui`       | Extract UI atoms from SWF and compose ship-screen canvases to images                                          |
| `starbreaker-wwise`    | Parse Wwise soundbank (`.bnk`) files and resolve audio event chains                                           |
| `starbreaker-wem`      | Decode Wwise audio (`.wem`) to Ogg Vorbis                                                                     |
| `starbreaker-common`   | Shared types, binary reader, game install discovery                                                           |

## CLI

```
starbreaker <command> [options]
```

Commands: `p4k`, `dcb`, `entity`, `skin`, `socpak`, `cryxml`, `dds`, `glb`, `chf`, `armour`, `wwise`, `nmc`, `ui`, `animation`, `riglogic`.

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

Export a ship or vehicle as a GLB file with full loadout assembly — geometry, skeleton, materials, and textures.
The name is a case-insensitive substring of the entity class name; the shortest match wins.
Defaults are `--lod 1`, `--mip 2` (textures at 1/4 resolution) and `--materials textures`:

```sh
# Export a Gladius
starbreaker entity export Gladius gladius.glb

# Lightweight export — flat material colours, no textures, lower LOD
starbreaker entity export Gladius gladius.glb --materials colors --lod 2

# Decomposed package (Packages/<name>/scene.json + shared Data/) for the Blender addon
starbreaker entity export Gladius ./export --kind decomposed

# Just print the loadout tree
starbreaker entity loadout Gladius

# Export an armour piece as worn (its female mesh, skinned to its bones)
starbreaker entity export cds_armor_heavy_arms_01_01_01 arms.glb --geometry-tag Female
```

Output of the first command:

```
Found 67 candidates, using shortest match: EntityClassDefinition.AEGS_Gladius

Loadout tree for EntityClassDefinition.AEGS_Gladius:
  G hardpoint_seat_access -> AEGS_Gladius_SCItem_SeatAccess
  G hardpoint_seat_pilot -> AEGS_Gladius_SCItem_Seat_Pilot
  G hardpoint_power_plant -> POWR_AEGS_S01_Regulus_SCItem
  G Hardpoint_cooler_left -> COOL_AEGS_S01_Bracer_SCItem
  G hardpoint_shield_generator_left -> SHLD_GODI_S01_AllStop_SCItem
  ...

Bundled export size: 37498672 bytes
Written to gladius.glb
```

</details>

<details>
<summary><b>Character export</b> — a <code>.chf</code> character as a rigged GLB or <code>.blend</code> package</summary>

### Character export

`chf to-glb` exports the whole character: the DNA-shaped head with its
eyes, teeth and CHF head items (hair, brows, lashes, beard), and the body,
all on the player actor skeleton. `chf to-package` writes the same rig as
a decomposed `.blend` package with the CHF's skin material customization.
Both can dress the character in a loadout and bake a face and/or body clip.

```sh
starbreaker chf to-glb masculinedefault.chf

# Dressed in a game loadout, playing a face idle
starbreaker chf to-glb masculinedefault.chf marine.glb \
  --loadout Data/Scripts/Loadouts/Character/Human/Armor/NPC/UEE/marine_Armor_Heavy.xml \
  --face-anim Data/Animations/Characters/Human/heads/male/ai/idlesets/happy/face/ai_civ_happy_idle_02_face.caf

# Decomposed .blend package for the Blender addon
starbreaker chf to-package masculinedefault.chf ./export
```

</details>

<details>
<summary><b>Armour export</b> — armour and clothing, single pieces or worn sets</summary>

### Armour export

A single piece exports as worn (its gendered mesh, skinned to its bones):

```sh
starbreaker entity export cds_armor_heavy_arms_01_01_01 arms.glb --geometry-tag Female
```

A set is worn by a character on one skeleton — the game's default
character of `--gender`, or any `.chf` with `--chf` — with covered body
parts left out (delete the head and body in Blender for the armour alone).
Pieces are exact entity class names; each goes on the port that accepts it,
as in game:

```sh
starbreaker armour export marine.glb --gender male \
  --wear cds_undersuit_01_01_01 --wear cds_armor_heavy_core_01_01_01 \
  --wear cds_armor_heavy_arms_01_01_01 --wear cds_armor_heavy_legs_01_01_01

# Decomposed .blend package from a game loadout, playing a body idle
starbreaker armour export marine_pkg --kind package \
  --loadout Data/Scripts/Loadouts/Character/Human/Armor/NPC/UEE/marine_Armor_Heavy.xml \
  --body-anim Data/Animations/Characters/Human/male_v7/character_select_screen.dba \
  --body-anim-clip char_customizer_stand_idle_01
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

# unp4k/unforge-compatible output, plus unforge's combined Game2.xml
# (every record, ~3 GB) for tools that read it, e.g. ScDataDumper
starbreaker dcb extract --format unp4k --combined -o ./import/Data
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
starbreaker dds decode Objects/path/to/texture.dds texture.png

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

# Extract a soundbank's WEM files, decoded to Ogg
starbreaker wwise extract ship_sounds.bnk -o ./audio --decode
```

</details>

## App

A Tauri desktop app lives in `app/`. React + TypeScript frontend, Rust backend. Provides a GUI for browsing P4k contents and DataCore records, exporting ships and socpak interiors, building and exporting characters, and playing audio.

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

The CLI and library auto-detect your Star Citizen install by scanning the known channel roots and picking the newest `Data.p4k`.

Supported default roots:

- Windows: `C:\Program Files\Roberts Space Industries\StarCitizen\`
- Linux LUG Wine or Proton installs: `~/Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/`

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
