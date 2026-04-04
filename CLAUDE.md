# StarBreaker - Claude Code Instructions

## Building

Use `cargo build` (debug) for iteration, NOT `cargo build --release`. Debug profile is `[optimized + debuginfo]` in this workspace — fast enough for testing. Release builds take much longer and are only needed for deployment. Only use `--release` for MCP server deployment or final binaries.

## Python

Always use `uv run python` instead of `python`, `python3`, or `py` when running Python scripts or one-liners. This project uses `uv` for Python tooling.

## MCP Server

The StarBreaker MCP server provides DataCore, P4k, and chunk inspection tools for Claude Code. To deploy after making changes:

```bash
taskkill //F //IM starbreaker-mcp.exe 2>/dev/null; cargo build --release -p starbreaker-mcp && cp target/release/starbreaker-mcp.exe mcp/starbreaker-mcp.exe
```

You must kill the running MCP process before copying, or the file will be locked. Then restart Claude Code to pick up the new binary. The `.mcp.json` points to `mcp/starbreaker-mcp.exe` (the deployed copy), not the build artifact, so the running server isn't locked by workspace builds.

### When to Add MCP Tools

If you find yourself doing a task that MCP would be a good fit for (e.g., repeatedly querying game data, inspecting files, or doing lookups that shell commands are awkward for), add a new tool to the MCP server or note it as a task for later.

### Available MCP Tools

Use these tools (via ToolSearch for `starbreaker`) to research game data without shelling out to the CLI:

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

- **MCP tools** return raw/lightly-processed data for research. Use them to investigate DataCore records, browse files, inspect textures, and understand game data structure.
- **CLI** (`cargo run --bin starbreaker`) is for export operations and testing the full export pipeline. Use it when you need to actually export a GLB or test changes to the export code.
- For raw DataCore loadout data, use `datacore_query` with path `Components[SEntityComponentDefaultLoadoutParams]`. The `entity_loadout` tool returns StarBreaker's processed/resolved tree instead.
