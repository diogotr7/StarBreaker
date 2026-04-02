# Geometry Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show an interactive 3D preview of geometry files in the P4K browser's right panel.

**Architecture:** A Tauri command reads a geometry file from P4K, converts it to GLB via the existing `starbreaker_gltf::skin_to_glb` pipeline (no textures, modest LOD), and returns the bytes. A React component renders the GLB using Three.js with OrbitControls.

**Tech Stack:** Rust/Tauri backend, React 19 + TypeScript frontend, Three.js (already in dependencies), `starbreaker_gltf` crate.

**Spec:** `docs/superpowers/specs/2026-04-03-geometry-preview-design.md`

---

### Task 1: Backend — Add `preview_geometry` Tauri command

**Files:**
- Modify: `app/src-tauri/src/commands.rs` (add command at end of file)
- Modify: `app/src-tauri/src/main.rs` (register command in handler list)

- [ ] **Step 1: Add the `preview_geometry` command to `commands.rs`**

Append this function at the end of `app/src-tauri/src/commands.rs`, before the closing of the file:

```rust
/// Generate a GLB preview for a geometry file in the P4K.
/// Accepts .skin, .skinm, .cgf, .cgfm, .cga, .chr paths.
/// Companion files (.skinm/.cgfm) are resolved to their primary (.skin/.cgf).
#[tauri::command]
pub fn preview_geometry(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<u8>, AppError> {
    let p4k = state
        .p4k
        .lock()
        .as_ref()
        .ok_or_else(|| AppError::Internal("P4K not loaded".into()))?
        .clone();

    // Resolve companion: .skinm -> .skin, .cgfm -> .cgf
    let primary = if path.ends_with('m')
        && (path.ends_with(".skinm") || path.ends_with(".cgfm"))
    {
        path[..path.len() - 1].to_string()
    } else {
        path.clone()
    };

    // Try reading the companion file first (has vertex data), fall back to primary
    let companion = format!("{primary}m");
    let data = p4k
        .read_file(&companion)
        .or_else(|_| p4k.read_file(&primary))?;

    let glb = starbreaker_gltf::skin_to_glb(&data)?;
    Ok(glb)
}
```

- [ ] **Step 2: Register the command in `main.rs`**

In `app/src-tauri/src/main.rs`, add `commands::preview_geometry` to the `generate_handler![]` macro list. Add it after the last `commands::` entry (look for the existing `commands::cancel_export` or similar and add on the next line):

```rust
commands::preview_geometry,
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p starbreaker-app`
Expected: compiles with no errors.

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/commands.rs app/src-tauri/src/main.rs
git commit -m "feat: add preview_geometry Tauri command"
```

---

### Task 2: Frontend — Add `previewGeometry` command binding

**Files:**
- Modify: `app/src/lib/commands.ts` (add command wrapper)

- [ ] **Step 1: Add the TypeScript command wrapper**

Add this at the end of `app/src/lib/commands.ts`, before the audio section or at the very end:

```typescript
// ── Geometry preview ──

/** Generate a GLB preview for a geometry file. Returns raw GLB bytes. */
export async function previewGeometry(path: string): Promise<ArrayBuffer> {
  const bytes = await invoke<number[]>("preview_geometry", { path });
  return new Uint8Array(bytes).buffer;
}
```

- [ ] **Step 2: Commit**

```bash
git add app/src/lib/commands.ts
git commit -m "feat: add previewGeometry command binding"
```

---

### Task 3: Frontend — Create `GeometryPreview` component

**Files:**
- Create: `app/src/components/geometry-preview.tsx`

- [ ] **Step 1: Create the component file**

Create `app/src/components/geometry-preview.tsx` with this content:

```tsx
import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { previewGeometry } from "../lib/commands";

interface Props {
  path: string;
}

export function GeometryPreview({ path }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const stateRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    controls: OrbitControls;
    animId: number;
  } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);

  // Initialize Three.js scene once on mount
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setClearColor(0x1e1e2e); // dark theme background
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(container.clientWidth, container.clientHeight);
    container.appendChild(renderer.domElement);

    const scene = new THREE.Scene();

    const camera = new THREE.PerspectiveCamera(
      50,
      container.clientWidth / container.clientHeight,
      0.01,
      10000,
    );
    camera.position.set(0, 2, 5);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.1;

    // Lighting
    const hemi = new THREE.HemisphereLight(0xb1bfd8, 0x3a3a3a, 1.5);
    scene.add(hemi);
    const dir = new THREE.DirectionalLight(0xffffff, 1.0);
    dir.position.set(5, 10, 7);
    scene.add(dir);

    // Render loop
    let animId = 0;
    const animate = () => {
      animId = requestAnimationFrame(animate);
      controls.update();
      renderer.render(scene, camera);
    };
    animate();

    stateRef.current = { renderer, scene, camera, controls, animId };

    // Resize observer
    const ro = new ResizeObserver(() => {
      const w = container.clientWidth;
      const h = container.clientHeight;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    });
    ro.observe(container);

    return () => {
      ro.disconnect();
      cancelAnimationFrame(animId);
      controls.dispose();
      renderer.dispose();
      container.removeChild(renderer.domElement);
      stateRef.current = null;
    };
  }, []);

  // Load GLB when path changes
  useEffect(() => {
    const state = stateRef.current;
    if (!state) return;

    const gen = ++generationRef.current;
    setLoading(true);
    setError(null);

    // Clear previous model
    const model = state.scene.getObjectByName("model");
    if (model) {
      state.scene.remove(model);
      model.traverse((child) => {
        if (child instanceof THREE.Mesh) {
          child.geometry.dispose();
          if (Array.isArray(child.material)) {
            child.material.forEach((m) => m.dispose());
          } else {
            child.material.dispose();
          }
        }
      });
    }

    previewGeometry(path)
      .then((buffer) => {
        if (gen !== generationRef.current) return; // stale

        const loader = new GLTFLoader();
        loader.parse(buffer, "", (gltf) => {
          if (gen !== generationRef.current) return; // stale
          gltf.scene.name = "model";
          state.scene.add(gltf.scene);
          fitCamera(state.camera, state.controls, gltf.scene);
          setLoading(false);
        });
      })
      .catch((err) => {
        if (gen !== generationRef.current) return; // stale
        setError(String(err));
        setLoading(false);
      });
  }, [path]);

  return (
    <div ref={containerRef} className="relative w-full h-full">
      {loading && (
        <div className="absolute inset-0 flex items-center justify-center bg-base/80 z-10">
          <div className="flex flex-col items-center gap-2 text-text-dim">
            <svg
              className="animate-spin h-6 w-6"
              viewBox="0 0 24 24"
              fill="none"
            >
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
              />
            </svg>
            <span className="text-xs">Loading geometry...</span>
          </div>
        </div>
      )}
      {error && (
        <div className="absolute inset-0 flex items-center justify-center bg-base z-10">
          <div className="text-center px-8">
            <p className="text-red-400 text-sm font-medium">
              Failed to load geometry
            </p>
            <p className="text-text-dim text-xs mt-1 font-mono break-all">
              {error}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}

/** Position the camera so the entire model is visible. */
function fitCamera(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  object: THREE.Object3D,
) {
  const box = new THREE.Box3().setFromObject(object);
  const center = box.getCenter(new THREE.Vector3());
  const size = box.getSize(new THREE.Vector3()).length();

  controls.target.copy(center);

  const fov = camera.fov * (Math.PI / 180);
  const distance = size / (2 * Math.tan(fov / 2)) * 1.2; // 1.2x padding

  const direction = camera.position.clone().sub(center).normalize();
  camera.position.copy(center.clone().add(direction.multiplyScalar(distance)));
  camera.near = size / 100;
  camera.far = size * 100;
  camera.updateProjectionMatrix();
  controls.update();
}
```

- [ ] **Step 2: Verify TypeScript compiles**

Run from `app/`: `npx tsc --noEmit`
Expected: no type errors (or only pre-existing ones).

- [ ] **Step 3: Commit**

```bash
git add app/src/components/geometry-preview.tsx
git commit -m "feat: add GeometryPreview Three.js component"
```

---

### Task 4: Frontend — Wire preview into P4K browser

**Files:**
- Modify: `app/src/views/p4k-browser.tsx`

- [ ] **Step 1: Add import and helper function**

At the top of `app/src/views/p4k-browser.tsx`, add the import alongside the existing imports:

```typescript
import { GeometryPreview } from "../components/geometry-preview";
```

Then add this helper function after the `formatSize` function (around line 12):

```typescript
const GEOMETRY_EXTENSIONS = [".skin", ".skinm", ".cgf", ".cgfm", ".cga", ".chr"];

function isGeometryFile(path: string): boolean {
  const lower = path.toLowerCase();
  return GEOMETRY_EXTENSIONS.some((ext) => lower.endsWith(ext));
}
```

- [ ] **Step 2: Replace the preview placeholder**

Replace the right-panel placeholder div (the `{/* Preview panel (placeholder) */}` section, lines 262-272) with:

```tsx
      {/* Preview panel */}
      <div className="flex-1 flex items-center justify-center text-text-dim overflow-hidden">
        {selectedPath && isGeometryFile(selectedPath) ? (
          <GeometryPreview path={selectedPath} />
        ) : selectedPath ? (
          <div className="text-center">
            <p className="text-sm font-mono break-all px-8">{selectedPath}</p>
          </div>
        ) : (
          <p className="text-sm">Select a file to preview</p>
        )}
      </div>
```

- [ ] **Step 3: Verify it compiles and renders**

Run from `app/`: `npx tsc --noEmit`
Expected: no type errors.

Then run: `npm run dev` (or `cargo tauri dev`)
Expected: the app launches. Navigate to P4K browser, click a `.skin` file, and you should see a 3D model in the right panel.

- [ ] **Step 4: Commit**

```bash
git add app/src/views/p4k-browser.tsx
git commit -m "feat: wire geometry preview into P4K browser"
```

---

### Task 5: Manual testing and polish

**Files:** None (testing only)

- [ ] **Step 1: Test with various geometry types**

Launch the app with `cargo tauri dev`. Open a P4K and test these file types:

1. Click a `.skin` file (e.g. under `Objects/Spaceships/Ships/AEGS/Gladius/`) — should render the mesh
2. Click a `.skinm` file (same directory) — should also render (companion resolution)
3. Click a `.cgf` file (e.g. under `Objects/`) — should render static geometry
4. Click a `.cgfm` file — should also render
5. Click a `.cga` file — should render animated geometry mesh
6. Click a `.chr` file — may show bones or fail gracefully with an error message
7. Click a non-geometry file (`.dds`, `.xml`) — should show the path text, no 3D preview
8. Click through multiple geometry files quickly — should not crash, stale responses should be discarded
9. Resize the right panel — the 3D viewport should resize with it

- [ ] **Step 2: Fix any issues found**

Address any problems discovered during testing. Common issues:
- Three.js import paths may need adjustment (check `three/addons/` vs `three/examples/jsm/`)
- Large models may need the camera near/far planes adjusted
- The `ArrayBuffer` return type from Tauri may need conversion (Tauri returns `number[]` for `Vec<u8>` — if so, convert with `new Uint8Array(data).buffer`)

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "fix: geometry preview polish from manual testing"
```
