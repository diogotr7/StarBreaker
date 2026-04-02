# P4K Geometry File Preview

## Goal

When a user selects a geometry file in the P4K browser, show an interactive 3D preview in the right panel. Geometry, normals, and tangents are displayed at a modest LOD with no textures. The model can be orbited, panned, and zoomed.

## Supported File Types

| Extension | Triggers preview | Notes |
|-----------|-----------------|-------|
| `.skin`   | Yes | Primary mesh format |
| `.skinm`  | Yes | Companion; resolved back to `.skin` for loading |
| `.cgf`    | Yes | Static/rigid geometry |
| `.cgfm`   | Yes | Companion; resolved back to `.cgf` |
| `.cga`    | Yes | Animated geometry |
| `.chr`    | Yes | Skeleton (bones only) |

Files not in this set show file metadata (name, size, extension) in the right panel as before.

## Backend

### Tauri Command

A single new command in `app/src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn preview_geometry(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<u8>, AppError>
```

**Inputs**: P4K file path (e.g. `Objects/Spaceships/Ships/AEGS/Gladius/aegs_gladius.skin`).

**Outputs**: Raw GLB bytes.

**Behavior**:

1. Resolve companion files: if `path` ends in `.skinm`/`.cgfm`, strip the trailing `m` to get the primary. Conversely, the pipeline internally appends `m` to find the companion data.
2. Read the geometry file from the P4K in `AppState`.
3. Call `starbreaker_gltf::parse_skin` + `write_glb` (or `skin_to_glb`) with preview-oriented options:
   - `include_textures: false`
   - `include_materials: false`
   - `include_normals: true`
   - `include_tangents: true`
   - `lod_level: 2`
4. Return the GLB bytes.

**Error handling**: Return `AppError` on missing file, parse failure, or unsupported format. The frontend displays the error inline.

## Frontend

### GeometryPreview Component

New file: `app/src/components/geometry-preview.tsx`

**Props**: `path: string` (the selected P4K file path).

**Lifecycle**:

1. When `path` changes, call `invoke('preview_geometry', { path })`.
2. Show a loading spinner centered in the component while the backend generates the GLB.
3. On success, load the GLB into the Three.js scene.
4. On error, display the error message inline (replacing the viewport).
5. On unmount or path change, dispose the previous Three.js scene, renderer, and geometry to prevent GPU memory leaks.

**Stale response handling**: Track a request generation counter. If `path` changes before the previous request completes, ignore the stale response. This avoids race conditions when the user clicks through files quickly.

**Three.js setup**:

- `WebGLRenderer` with `antialias: true`, background matching the app's dark theme.
- `PerspectiveCamera` with auto-fit: after loading the GLB, compute the bounding box and position the camera so the entire model is visible with some padding.
- `OrbitControls` for rotate, pan, zoom.
- `HemisphereLight` (sky: `#b1bfd8`, ground: `#3a3a3a`, intensity ~1.5) for soft ambient fill.
- `DirectionalLight` (intensity ~1.0) positioned at the camera's initial location for definition.
- `MeshStandardMaterial` default (lit grey) — the GLB from the pipeline will use this automatically since no textures/materials are included.

**Resize handling**: Listen for container resize (ResizeObserver) and update renderer size + camera aspect ratio.

### P4K Browser Integration

In `app/src/views/p4k-browser.tsx`, replace the "Preview coming soon" right panel:

```
if (isGeometryFile(selectedPath)):
    <GeometryPreview path={selectedPath} />
else:
    <FileInfo path={selectedPath} size={selectedSize} />
```

`isGeometryFile` checks if the extension is one of: `.skin`, `.skinm`, `.cgf`, `.cgfm`, `.cga`, `.chr`.

## Files Changed

| File | Change |
|------|--------|
| `app/src-tauri/src/commands.rs` | Add `preview_geometry` command |
| `app/src-tauri/src/main.rs` | Register the new command |
| `app/src/components/geometry-preview.tsx` | New Three.js viewer component |
| `app/src/views/p4k-browser.tsx` | Wire preview into right panel |

## Out of Scope

- Texture/material preview (future enhancement)
- `.cdf` character definition assembly (requires DataCore + multi-part loading)
- Rendering mode toggle (wireframe, matcap, etc.)
- Animation playback
- Interior geometry from socpaks
