# StarBreaker

[Download latest release](https://github.com/diogotr7/StarBreaker/releases/latest)

Toolkit for reading and extracting Star Citizen game files. Handles the P4k archive, DataCore database, CryEngine assets, Wwise audio, and character files.

There is also a legacy C# version on the [`csharp`](../../tree/csharp) branch.

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

### Game file discovery

The CLI and library auto-detect your Star Citizen install under `C:\Program Files\Roberts Space Industries\StarCitizen\`, scanning LIVE/PTU/EPTU channels by modification time.

To override, copy `.cargo/config.toml.example` to `.cargo/config.toml` and set your paths:

```toml
[env]
SC_DATA_P4K = "D:\\Games\\StarCitizen\\LIVE\\Data.p4k"
```

Or set the `SC_DATA_P4K` / `SC_EXE` environment variables directly.


