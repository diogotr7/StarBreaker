# P4k Extract MCP Tool

## Summary

Add a `p4k_extract` tool to the StarBreaker MCP server that extracts files from the P4k archive to disk, mirroring the archive's directory structure. This replaces the planned "MCP Python eval" idea — the actual bottleneck is getting bytes out of P4k onto the filesystem, not running Python. Once files are on disk, any tool (Python scripts, hex editors, Blender, etc.) can work with them directly.

## Tool Specification

### Name

`p4k_extract`

### Parameters

| Parameter    | Type            | Required | Default      | Description                                      |
|-------------|-----------------|----------|--------------|--------------------------------------------------|
| `paths`     | array of string | yes      | —            | P4k paths or glob patterns to extract            |
| `output_dir`| string          | no       | `tmp/p4k/`   | Staging directory (relative to project root)     |

### Path Handling

- Paths are normalized using the existing `normalize_p4k_path()` logic: auto-prepend `Data\`, convert `/` to `\`, `.tif` to `.dds`
- Glob patterns expand against the P4k file index (e.g. `*.mtl`, `**/*.dds`, `Data/Objects/Spaceships/AEGS/AEGS_Gladius/*`)
- Glob matching is case-insensitive to match P4k lookup behavior

### Extraction Behavior

- Files are written to `{output_dir}/{p4k_path}`, preserving the full P4k directory structure
  - Example: `tmp/p4k/Data/Objects/Spaceships/AEGS/AEGS_Gladius/hull.cgf`
- CryXML binary files are auto-decoded to readable XML text (same behavior as the existing `p4k_read` tool)
- Existing files at the target path are overwritten silently
- Parent directories are created as needed

### Return Value

Text listing of extracted files with sizes and a summary:

```
Extracted 12 files to tmp/p4k/

  Data/Objects/Spaceships/AEGS/AEGS_Gladius/hull.cgf          124.5 KB
  Data/Objects/Spaceships/AEGS/AEGS_Gladius/hull.mtl            3.2 KB  (CryXML -> XML)
  Data/Objects/Spaceships/AEGS/AEGS_Gladius/hull.dds          512.0 KB
  ...

Total: 2.1 MB
```

- CryXML-decoded files are annotated with `(CryXML -> XML)`
- Sizes reflect the output size on disk (post-decode for CryXML)

### Error Handling

- If a path matches no files, include it in the output as a warning (e.g. `WARN: no matches for "Data/Objects/foo/*"`)
- If a single file fails to extract, log the error for that file and continue with the rest
- If `paths` is empty, return an error message

## Implementation Notes

### Glob Matching

Use the `glob` or `globset` crate to match patterns against P4k entry paths. The P4k index is already available via `self.p4k().entries()`. Convert both the pattern and entry paths to a consistent case (lowercase) for matching.

### CryXML Detection

Reuse the existing CryXML detection logic from `p4k_read` — check the file magic bytes before writing. If CryXML, decode and write the XML text instead of raw bytes.

### Staging Directory

- Default staging directory is `tmp/p4k/` relative to the project root
- Add `tmp/` to `.gitignore` if not already present
- The MCP server's working directory is the project root (set by the MCP client launch config) — use `std::env::current_dir()` to resolve relative output paths

### Code Location

Add the tool to `mcp/src/tools.rs` following the existing pattern:
1. Define `P4kExtractRequest` struct with `paths` and `output_dir` fields
2. Add `#[tool]` handler method on `StarBreakerMcp`
3. The `#[tool_router]` macro auto-registers it

### Dependencies

- Add `globset` to `mcp/Cargo.toml` for glob pattern matching
- No other new dependencies needed — file I/O is stdlib, CryXML decoding is already available

## What This Replaces

This tool replaces the previously planned "MCP Python eval" (`py_eval`) tool. The original idea was to embed a Python interpreter (via PyO3) in the MCP server with pre-bound P4k/DataCore globals. Through brainstorming, we determined:

1. The real pain point is extracting files from P4k, not running Python
2. Embedded Python (PyO3) would lose access to pip packages like numpy and PIL
3. Shelling out to Python just recreates existing `uv run python` workflow with extra steps
4. A simple extract tool solves the problem and works with any downstream tool
