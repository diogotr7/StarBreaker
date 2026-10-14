# Blender Import / Export Performance Analysis

**Status:** Phase 2 (direct-first path resolution) is done;
Phases 3-6 are open.

## Context

The current branch implements a native `.blend` socpak export path on top
of the existing decomposed exporter. Output quality is mostly acceptable,
but opening the generated `scene.blend` in Blender can take a long time
because the StarBreaker add-on starts rebuilding package materials after
the file loads.

The exporter cost is currently tolerable. The main concern is importer
latency, especially for large ships or packages with many linked assets,
material sidecars, layered materials, decals, and POM materials.

## Why the Game Loads Faster

Star Citizen is loading cooked runtime assets through Star Engine. Its
load path is optimized around:

- binary asset streaming;
- packed archive reads;
- precompiled/runtime-native material and shader systems;
- GPU-ready texture formats;
- asset managers that share loaded resources by identity;
- visibility, LOD, and streaming decisions that avoid fully materializing
  everything as editable content at once.

The StarBreaker Blender path does something fundamentally heavier. It is
not only loading the scene; it is also reconstructing editable DCC data:

- parse `scene.json`, `palettes.json`, `liveries.json`, `paints.json`;
- resolve sidecar and texture paths under the export root;
- load material sidecar JSON;
- create or validate Blender material datablocks;
- create editable node trees via Python/RNA;
- load image datablocks;
- append bundled template/POM node groups;
- assign material overrides per object slot;
- refresh Blender mesh/material/dependency evaluation.

That means the importer currently performs part of the "cooking" step at
file-open time. The engine avoids most of that work by consuming data in
the format it was built to run.

## Current Pipeline

### Exporter

The decomposed exporter entry point is:

- `crates/starbreaker-3d/src/decomposed.rs::write_decomposed_export`

The existing performance test notes that `write_decomposed_export` has
been the dominant export phase in previous measurements:

- `crates/starbreaker-3d/tests/phase_6d_performance_test.rs`

The native `.blend` exporter then builds actual mesh `.blend` files and a
linked `scene.blend`:

- `crates/starbreaker-3d/src/pipeline/blend_assembly.rs`

Important observations:

- Native mesh `.blend` asset generation is already parallelized through
  `build_native_blend_assets`.
- Individual mesh assets are compressed for Blender 5.x storage.
- `scene.blend` links external mesh `.blend` files rather than embedding
  every mesh directly.

This is a good direction: it moves geometry loading toward Blender-native
file loading instead of Python/glTF import.

### Blender Import / File Open

When a generated `scene.blend` is opened, the add-on registers a
load-post handler:

- `blender_addon/starbreaker_addon/ui.py::_starbreaker_load_post`

That handler schedules `_material_refresh_prompt_timer`, which scans for
package roots needing refresh and creates:

- `blender_addon/starbreaker_addon/runtime/package_ops.py::MaterialRefreshSession`

`MaterialRefreshSession` walks package mesh objects and calls:

- `PackageImporter.rebuild_object_materials`

Material creation/reuse flows through:

- `runtime/importer/materials.py::material_for_submaterial`
- `runtime/importer/builders.py::_build_managed_material`

The importer already contains several targeted optimizations:

- deferred `view_layer.update()` batching;
- batched orphan material removal with `bpy.data.batch_remove`;
- disabled Blender glTF post-import selection via
  `import_select_created_objects=False`;
- vectorized bitangent-sign baking with `foreach_get` / `foreach_set`;
- material identity and slot-layout caches.

These help, but the large cost remains the overall architecture: many
editable materials are still built or validated at open/import time.

## Likely Bottlenecks

### 1. Export Root Path Indexing

**Resolved (Phase 2).** `PackageBundle.resolve_path()` used to build the
recursive export-root path index (`export_root.rglob("*")`) before trying
direct candidate paths, so on a shared export root like `ships/` it could scan
every exported file before resolving the first sidecar or texture.

File:

- `blender_addon/starbreaker_addon/manifest.py`

It now resolves direct-first:

1. normalize candidates;
2. try `export_root / candidate` directly;
3. build the recursive fallback index (`_build_path_index`, cached on the
   bundle) only when a direct lookup fails.

Still open: making the path index global per export-root mtime or root path,
so multiple packages do not rebuild it.

### 2. Full Material Cooking on File Open

The generated `scene.blend` can open quickly from Blender's point of
view, but the add-on then starts material refresh. For large packages,
that makes "open file" feel like a full import.

The preference already exists:

- `auto_refresh_unloaded_materials_on_load`

But the current default behavior favors convenience over latency.

Recommended behavior for native `scene.blend` packages:

- open quickly with placeholder/proxy materials;
- make full material refresh explicit, or defer it incrementally;
- optionally auto-refresh only visible or selected content first.

Expected impact: very high perceived latency improvement.

Risk: medium, because it changes default UX and users may expect final
materials immediately after open.

### 3. Rebuilding Editable Material Graphs

`_build_managed_material` constructs material node graphs through Blender
Python/RNA. That is inherently slower than loading prebuilt datablocks
from a `.blend` library.

This is the largest architectural mismatch with the game engine. The game
loads runtime material definitions; the add-on cooks Blender material
graphs as it imports.

Recommended fix: add a cooked Blender material cache.

Cache key should include at least:

- material sidecar path;
- sidecar mtime or content hash;
- submaterial index;
- palette id or palette scope;
- material template contract/library version;
- add-on material schema version;
- POM/detail quality tier;
- host decal channel/RGB variant where applicable.

Possible storage options:

- package-local `materials.blend`;
- export-root material cache `.blend`;
- saved fully refreshed `scene.blend`;
- per-sidecar material library files.

Expected impact: very high on repeated opens and repeated imports.

Risk: medium-high, because cache invalidation must be correct.

### 4. Object-by-Object Material Refresh

`MaterialRefreshSession` currently iterates mesh objects and rebuilds or
assigns slots per object. `rebuild_object_materials()` has useful caches,
but the refresh unit is still an object.

A faster model is:

1. scan objects once;
2. group by material layout identity:
   - material sidecar;
   - effective palette;
   - slot mapping;
   - source sidecar;
   - host decal signature;
3. build the tuple of target materials once per group;
4. assign material slots in a tight second pass.

This reduces repeated sidecar lookup, submaterial remapping, material
compatibility checks, host decal variant lookup, and slot layout work.

Expected impact: medium-high for packages with many repeated interior
placements or repeated components.

Risk: medium.

### 5. POM and Template Group Appends

POM materials are expensive because the importer appends and patches
bundled POM node groups per height image:

- `runtime/importer/groups.py::_ensure_runtime_parallax_group`

This is correct for editability and visual fidelity, but expensive for
interactive loading. Blender shader groups cannot accept an image
datablock as a normal socket input, so per-height-image copies are
currently required for the authored POM chain.

Recommended change: add material quality tiers.

Suggested tiers:

- `Proxy`: simple Principled material, palette/base color only, no POM.
- `Preview`: main textures and major shader groups, no full POM.
- `Final`: current full material graph.

Default file-open should use `Proxy` or `Preview`; users can upgrade to
`Final` for inspection/rendering.

Expected impact: high on POM-heavy scenes.

Risk: medium, because visual results differ until upgraded.

### 6. glTF Import Path Still Exists

The native `.blend` open path links `.blend` mesh assets, but the Python
package importer still has `ensure_template()` using:

- `bpy.ops.import_scene.gltf`

This is relevant when using the add-on's explicit import operator rather
than opening the generated native `scene.blend`.

If `scene.json` references `.blend` mesh assets, `ensure_template()` should
detect that and use Blender library loading instead of the glTF operator.

Recommended behavior:

- `.blend` mesh asset: load/link objects or collections from the library;
- `.glb` mesh asset: keep current glTF operator path.

Expected impact: high for explicit imports of native `.blend` packages.

Risk: medium-high, because the object hierarchy/material slot semantics
must match the current glTF-template path.

## Exporter Opportunities

Exporter optimization is less urgent than importer optimization, but there
are clear opportunities:

- Deduplicate material sidecar and texture exports across children and
  interiors more aggressively.
- Persist texture conversion/cache data across exports, not only within
  one export process.
- Parallelize independent sidecar/texture extraction where data
  ownership allows it.
- Avoid generating placeholder mesh asset entries in decomposed export
  phases when native `.blend` assembly will immediately replace them.
- Keep using existing-asset detection to skip unchanged mesh/material
  writes.

The native `.blend` asset build path already uses Rayon. The remaining
export bottlenecks are likely texture extraction/recompression, sidecar
generation, and repeated material processing.

## Prioritized Plan

### Phase 1: Instrument Blender Import

Add timing around:

- package load;
- path resolution and path index construction;
- sidecar load;
- material reuse hit/miss;
- `_build_managed_material`;
- image load;
- POM group append/patch;
- host decal variant creation;
- cleanup/purge;
- final view-layer update.

Goal: get a per-import timing table for a small ship, medium ship, and a
large slow package.

### Phase 2: Direct-First Path Resolution (done)

`PackageBundle.resolve_path()` builds the recursive export-root index only
after direct candidate paths fail.

### Phase 3: Fast-Open Native Scene Mode

For generated native `scene.blend` packages:

- default to no full auto-refresh on file load, or use proxy refresh only;
- expose explicit "Build Full Materials";
- keep the existing preference for users who want current behavior.

Goal: make `scene.blend` open feel like a file open, not a full import.

### Phase 4: Cooked Material Cache

Create reusable Blender material libraries keyed by sidecar, submaterial,
palette, template version, and quality tier.

Target outcome:

- first import may still cook;
- repeated opens mostly load Blender datablocks;
- package refresh only rebuilds stale cache entries.

### Phase 5: Grouped Refresh

Replace pure object-by-object material rebuild with grouped layout refresh:

- scan package objects;
- group compatible objects;
- build material slot tuple once per group;
- assign slots in bulk.

### Phase 6: Quality Tiers

Add explicit material quality tiers:

- proxy;
- preview;
- final.

Use proxy/preview for open and navigation, final for rendering or material
inspection.

## Design Principle

The importer should stop treating file open as a full material authoring
session. The faster model is:

1. export/cook reusable Blender-native assets;
2. open/link those assets cheaply;
3. lazily upgrade only the materials or objects the user needs.

That mirrors the engine's advantage: do expensive translation once, cache
the result by stable identities, and make runtime loading mostly a
datablock/resource load.
