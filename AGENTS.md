# StarBreaker - AI Agent Instructions

This file covers the overall StarBreaker project (Rust crates, CLI, MCP
server, export pipeline). Blender-addon-specific guidance lives in
[blender_addon/AGENTS.md](blender_addon/AGENTS.md).

## Repository Layout

- `Cargo.toml` — workspace root. Crates live under `crates/`.
- `crates/starbreaker-3d/` — scene graph, `.soc` / light / interior
  parsing, decomposed-export JSON writer
  (`src/decomposed.rs`, `src/socpak.rs`, `src/types.rs`).
- `crates/starbreaker-p4k/`, `-chunks/`, `-cryxml/`, `-datacore/`,
  `-dds/`, `-wem/`, `-wwise/`, `-chf/`, `-common/` — format decoders.
- `cli/` — package name is `starbreaker` (binary `starbreaker`), NOT
  `starbreaker-cli`.
- `mcp/` — MCP server (see §MCP).
- `blender_addon/` — Python addon + tests. See its own AGENTS.md.
- `app/` — Tauri + React app (see `tasks.json` for build tasks).
- `docs/` — in-repo reference: export contract, material authoring,
  shader family inventory.

## Building

Use `cargo build` (debug) for iteration, NOT `cargo build --release`.
Debug profile is `[optimized + debuginfo]` in this workspace — fast
enough for testing. Release builds take much longer and are only
needed for deployment (MCP server, final binaries, CLI re-exports).

## Coding Practices

Shared across every language in the repo:

- **Keep files small.** A source file that grows past ~500 lines is a
  strong signal it wants to be split. Monolithic modules make diffs
  noisier, searches slower, and tests harder to target.
- **Split by responsibility, not by arbitrary line count.** Prefer
  cohesive modules (one type of concern per file) over sprawling
  "grab-bag" modules. Examples already in the tree:
  - Rust: each CryEngine format lives in its own crate under
    `crates/starbreaker-*`.
  - Python: `runtime/importer/` is split into mixins
    (`palette.py`, `decals.py`, `layers.py`, `materials.py`,
    `builders.py`, `groups.py`, `orchestration.py`) rather than one
    giant `importer.py`.
- **Fix the root cause, not the symptom.** Do not add `.max(small
  number)` style floors, fallback defaults, or try/except-pass around
  broken data. If the exporter is wrong, fix the exporter.
- **Never hard-code workarounds for specific assets.** Do not gate
  logic on a particular material name, ship name, texture path,
  socpak, or item ID. If one asset misbehaves, find the generic
  property of its category (shader family, blend mode flag, alpha
  usage, etc.) and fix the rule for the whole category. Named-asset
  branches rot the moment upstream renames or adds siblings.
- **Match existing conventions.** Read the neighbours before
  inventing a new pattern. Dataclass style in `manifest.py`, naming
  in `blender-material-contract-naming-rules.md`, error taxonomy in
  `starbreaker-common` — all of these exist so new code doesn't have
  to make them up again.
- **Don't over-engineer.** Only make changes that are directly
  requested or clearly necessary. Avoid speculative abstractions,
  "just in case" helpers, and refactors bundled into feature work.
- **Tests track behaviour, not lines.** Add or update tests when a
  behaviour changes; don't add tests just to bump coverage on
  trivial getters.

## Python

Always use `uv run python` instead of `python`, `python3`, or `py`
when running Python scripts or one-liners. This project uses `uv` for
Python tooling.

Exception: the Blender addon test suite runs with the system
`python3` (see `blender_addon/AGENTS.md`), because it stubs `bpy`.

## CLI re-export

After changing the Rust exporter, re-export a ship and reimport it in
Blender to verify behaviour. The binary is `target/release/starbreaker`
(package name `starbreaker`, not `starbreaker-cli`). Invoke it with:

```bash
SC_DATA_P4K=<path to Data.p4k> \
  ./target/release/starbreaker entity export <entity_name> <export_root> \
  --kind decomposed
```

`--kind decomposed` emits the reusable `scene.json` +
`Packages/<name>/` layout documented in
`docs/decomposed-export-contract.md`. Workspace-specific ship paths
and the `SC_DATA_P4K` location are in the workspace-root AGENTS.md.

## Git

The StarBreaker repo is self-contained (root = `StarBreaker/`); the
parent workspace is not a git repo. Commit with whatever author identity is already configured in your
environment. If git complains that `user.name` / `user.email` are
unset, configure them via `git config` rather than inlining `-c
user.name=... -c user.email=...` on every commit — doing that leaks
whatever placeholder you happen to use into the repo history.

Current work branch: `starbreaker-exporter`. Commits are local only —
do not push unless explicitly asked.

## MCP Server

The StarBreaker MCP server provides DataCore, P4k, and chunk
inspection tools for Claude Code. To deploy after making changes:

```bash
# Windows
taskkill //F //IM starbreaker-mcp.exe 2>/dev/null; cargo build --release -p starbreaker-mcp && cp target/release/starbreaker-mcp.exe mcp/starbreaker-mcp.exe

# Linux
pkill -f starbreaker-mcp || true
cargo build --release -p starbreaker-mcp && cp target/release/starbreaker-mcp mcp/starbreaker-mcp
```

You must kill the running MCP process before copying, or the file
will be locked / busy. Then restart the client to pick up the new
binary. The `.mcp.json` points to the deployed copy, not the build
artifact, so the running server isn't locked by workspace builds.

### When to Add MCP Tools

If you find yourself doing a task that MCP would be a good fit for
(e.g., repeatedly querying game data, inspecting files, or doing
lookups that shell commands are awkward for), add a new tool to the
MCP server or note it as a task for later.

### Available MCP Tools

Use these tools (via ToolSearch for `starbreaker`) to research game
data without shelling out to the CLI:

- **`search_entities`** — find EntityClassDefinition records by name substring
- **`search_records`** — search ALL DataCore record types (tint palettes, ammo, attachables, etc.)
- **`entity_loadout`** — dump resolved loadout tree (processed — resolves entity references and geometry paths)
- **`datacore_record`** — dump full record as JSON (by GUID or name substring)
- **`datacore_query`** — query a specific property path (e.g. `Components[VehicleComponentParams].vehicleDefinition`)
- **`p4k_list`** — browse P4k directories (shows size, compression, encryption)
- **`p4k_read`** — read P4k files (auto-decodes CryXML to XML text)
- **`image_preview`** — decode and view DDS/PNG/JPG textures from P4k (multimodal — you can see the image)
- **`chunk_list`** — list IVO/CrCh chunks in geometry files (type, version, size, NMC node summary)
- **`chunk_read`** — hex dump of specific chunks

### When to Use MCP vs CLI

- **MCP tools** return raw/lightly-processed data for research. Use
  them to investigate DataCore records, browse files, inspect
  textures, and understand game data structure.
- **CLI** (`cargo run --bin starbreaker` or the release binary
  above) is for export operations and testing the full export
  pipeline. Use it when you need to actually export a GLB or test
  changes to the export code.
- For raw DataCore loadout data, use `datacore_query` with path
  `Components[SEntityComponentDefaultLoadoutParams]`. The
  `entity_loadout` tool returns StarBreaker's processed/resolved tree
  instead.

## Reference Docs

- `docs/decomposed-export-contract.md` — scene.json / palettes.json /
  liveries.json / material-sidecar contract. Update when adding new
  fields to the exporter.
- `docs/blender-material-contract-naming-rules.md` — how shader
  families and slots are named and reconstructed.
- `docs/blender-material-slot-evidence.md` — evidence dumps used to
  derive the naming rules.
- `docs/blender-material-template-authoring.md` — how to author
  reusable Blender material node templates.
- `docs/blender-shader-family-inventory.json` — the canonical list of
  CryEngine shader families we know about.

