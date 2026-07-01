# Optimisation ledger

Append-only record of pipeline optimisation passes: what was profiled, what the
dominant cost actually was, which changes paid off, and — critically — which
approaches are **proven dead-ends** so they are not re-attempted. Read this before
starting a pass (the `starbreaker-optimisation` skill's Required reads).

Format per item:

- **Observed** — the measurement that prompted it (stage, wall, CPU%, RSS).
- **Finding** — the root cause / lever / dead-end, with evidence.
- **Action** — what landed (commit) or was reverted.

Cumulative export numbers and the longer narrative live in the `idris-export-perf`
project memory; this ledger is the per-pass profiling record.

---

## Workload: `AEGS_Idris_P --kind decomposed --lod 0` (Ryzen 5800X, 16 threads)

The reference workload — a capital ship exercises root + child + interior +
texture + UI stages. `RUST_LOG=info` emits the `[timing][decomposed]` /
`[timing][blend]` breakdown; `/usr/bin/time -v` gives wall / CPU% / max RSS.

### Levers that paid off (history; all byte-identical)

1. **UI render de-duplication** (`476a9503e`). Observed: ~219 UI binding renders
   dominating child+interior stages. Finding: only **29 unique** renders (a
   binding's PNG depends only on its `UiBindingView` fields). Action: render each
   unique key once (`UiRenderKey` + `prerender_ui_bindings`), look up per binding.
   Interior+child UI render 44s→~12s shared prerender.
2. **Parallel texture pre-decode** (`b01a54bfa`). Finding: interior textures were
   decoded serially. Action: `prewarm_decomposed_textures` enumerates the unique
   `(path, flavor)` set and decodes it in a `par_iter` into a shared `PngCache`.
   Byte-identical (after the flavor-aware cache key fix `6b3de94cd`).
3. **O(depth) path canonicalisation** (`02648f27d`, Phase 1). Observed:
   `interior_asset_resolve` ~13–16s; `canonicalize_output_path_case` scanned every
   key in the output map for every segment of every inserted path = O(files²).
   Finding: the cost was the scan, NOT mesh/decode. Action: `OutputFiles` wrapper
   with a lowercase-prefix→canonical-segment `case_index`, canonicalising in
   O(path-depth). Same first-seen casing → byte-identical. Wall **46.7→43.7s**
   (3-run, this machine).
4. **DDNA→roughness derivation memo** (2026-07-02; workload `anvl_carrack --kind
   decomposed --lod 0 --mip 0 --materials all`). Observed: post-PR29 the Carrack
   export regressed from ~1.5min to HOURS — `child_assets` 280s and the interior
   placement loop <1000/6168 placements in 23min, all on ONE core, with ZERO
   `png_cache` misses. Finding: `export_ddna_roughness_asset_with_status` (new in
   PR #29, `694fb8485`) runs `load_roughness_texture_result` — a full mip-level
   DDS decode + smoothness-statistics scan + PNG encode — **before** consulting
   `texture_cache`, because the sidecar status needs the decoded statistics; so
   the cost was O(sidecar references) instead of O(unique DDNA sources), and it
   bypasses `cached_load_keyed`/prewarm entirely (which is why `[tex-miss]`
   probes stayed silent — instrument the RIGHT layer). Action: memoize the whole
   `(TextureExportRef, TextureDerivationStatus)` result per raw source path
   (`ddna_status_cache`), shared across root/paint/child/interior sidecars.
   Carrack hours→**1:19**; Clipper same-flags **10:47.8→0:50.6** (12.8×) with a
   **0-line `diff -rq`** (excl. export_stamp) unfixed-vs-fixed, and a 0-line
   two-run determinism diff. Diagnosis trail: prior-session "42s baseline" was a
   STALE BINARY (June-20 build predating PR #29) — old hashed binaries under
   `target/release/deps/starbreaker-<hash>` let you time-travel-bisect without
   rebuilding; gdb/perf are locked down (ptrace_scope, perf_event_paranoid=4) so
   thread run-state (`/proc/<pid>/task/*/stat`) + staged eprintln probes are the
   available localisation tools.

### Dead-ends — do NOT re-attempt

1. **jemalloc via `LD_PRELOAD`** — measured SLOWER (98.8s vs 88.3s at the time).
   The pipeline is not allocator-bound.
2. **Parallelising the interior sidecar build** (the `build_interior_sidecar`
   prebuild + deferred-texture merge; attempted + DROPPED 2026-06-21). Observed: a
   `par_iter` prebuild of the sidecar JSON for 1459 assets took 9.66s at only
   **~1× effective speedup** (CPU never above ~600% on 16 threads). Finding: the
   interior sidecar work is **MEMORY-bandwidth-bound, not CPU-bound** (mesh clones
   + texture handling), so threads don't help; and the machinery needed to make the
   parallel path OOM-safe + byte-identical (defer pre-warmed textures as references,
   per-reference tokens, a per-sidecar string-replace at merge, a serial closure to
   keep `mesh_data_map` deterministic → the mesh view built twice) added **+17s** to
   the serial merge. Net result was fully byte-identical, deterministic, OOM-safe —
   and **+26s SLOWER** (70s vs 43.7s, 3 runs each). Reverted in full. **LESSON
   (load-bearing): profile parallelism efficiency on a representative slice BEFORE
   building the parallel path** — a low CPU% / ~1× `par_iter` is the stop sign.
   Naively holding every prebuilt asset's files (with decoded PNG bytes) also OOMs
   at 12GB+; that's the symptom of memory-bound work, not a thing to engineer
   around.

### Open / next candidates

- `prerender_ui` ~12s (29 unique renders, ~0.45s `graph2` each) — diminishing, but
  the next-largest single stage. CPU-bound (rasterisation) → an algorithmic or
  caching win, not more threads.
- Remaining `interior_asset_resolve` is `write_material_sidecar` JSON build + file
  inserts (memory-bound per the dead-end above — a parallel rewrite is off the
  table; look for redundant work / a cheaper serialization instead).
- First-decode of each unique DDNA source in `export_ddna_roughness_asset_with_status`
  is still serial inside the child/interior loops (~20s of Carrack's interior
  phase at mip 0). A `par_iter` prewarm of the unique DDNA set into
  `ddna_status_cache` (mirroring `prewarm_decomposed_textures`) is the natural
  next lever — the per-source result is a pure function, so it stays
  byte-identical; profile before building (DDS decode is CPU-bound, unlike the
  sidecar JSON dead-end).
