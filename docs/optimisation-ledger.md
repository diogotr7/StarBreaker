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
5. **flate2 → zlib-rs backend** (2026-07-18; workload `drak_clipper --kind
   decomposed --lod 0 --mip 0 --materials all`). Observed: bare `flate2 = "1"`
   resolves the default `miniz_oxide` (`rust_backend`) inflate on the P4K + SWF
   deflate hot paths. Finding: zlib-rs is the faster inflate backend and, being a
   format-exact DEFLATE decoder, produces bit-identical output — so it is a free
   swap under the byte oracle. Action: hoisted a workspace dependency
   `flate2 = { workspace = true, features = ["zlib-rs"] }` and pointed the four
   consumer crates (p4k, ui, gfx, blend) at it; `cargo tree -i flate2` shows one
   `flate2 v1.1.9` with the `zlib-rs` feature and `zlib-rs v0.6.6` linked. Byte
   oracle **0-line** `diff -rq` (excl. export_stamp) pre1-vs-post. Timing Δ:
   Clipper wall 48.0–49.0s (pre) → 45.6–48.3s (post, 3-run best 45.59s) — neutral
   within run-to-run noise, because inflate is a small slice of the ~40s
   asset/UI/interior-bound total (see the `[timing][blend] total: 39.59s`
   breakdown: interior_asset_resolve 12.1s, child_assets 7.8s, prerender_ui 7.4s
   dominate). Kept as a zero-risk backend upgrade that also speeds P4K reads
   elsewhere (MCP, UI) off the measured export path.

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

### Item-7 gate: SWF parse-once — GATED-IN (2026-07-18)

- **Observed** — post-zlib-rs re-profile of `drak_clipper --kind decomposed
  --lod 0 --mip 0 --materials all`, serial (`RAYON_NUM_THREADS=1`), 27 UI
  bindings, serial UI wall 26.957s. Per-stage totals: `render` 14.151s (52.5%),
  `ir_compile` 4.976s (18.5%), `graph2` 1.763s (6.5%), `swf_load` 1.654s (6.1%),
  `swf_load_measure` 1.648s (6.1%), `encode` 0.249s. Full table in
  `docs/StarBreaker/starbreaker-ui-perf-baseline.md` (Post-zlib-rs re-profile).
  The binding count fell 43 (B0, 2026-06-08) → 27 because the `UiRenderKey`
  render-dedup (commit `476a9503e`, 2026-06-21, after B0) renders one
  representative per distinct render key, collapsing duplicate screens.
- **Finding** — `swf_load` (1.654s) and `swf_load_measure` (1.648s) are two
  SEPARATE real SWF parses of the same asset (`pipeline/mod.rs:595` and `:750`,
  no cross-call cache), so their cost SUMS to 3.302s — rank #3 by stage cost,
  above `graph2` (1.763s). The prior GATED-OUT verdict wrongly treated the two
  timers as one load counted twice; they are two loads, each paid. The B2a/B2b
  memoisation collapsed `ir_compile` (303.9s → 4.98s), so with SWF now parsed
  twice per binding it is a genuine top-3 lever.
- **Action** — item-7 GATE = **GATED-IN** (reviewer-overturned, orchestrator-
  adjudicated). Task 3 (SWF parse-once: cache the parse across the `:595` and
  `:750` call sites) is IN SCOPE for this run. Docs-only here; no code change in
  this commit.

### Item-6: colour.rs LUT fast path for opaque premul blends — LANDED (2026-07-18)

- **Observed** — `blend_premul_linear` and `blend_premul_add_linear`
  (`crates/starbreaker-ui/src/colour.rs`) un-premultiply each of the 3 colour
  channels with `srgb_channel_to_linear(v/a)` — a `powf` — on BOTH the src and
  dst side, every pixel, even when the side is fully opaque (`a==255`). The
  compositor blends in linear light (B4), so these two fns are on the hot
  `render` stage (14.151s serial, 52.5% of UI wall).
- **Finding** — when `a==255` the un-premultiply divide is identity
  (`v/255 / 1.0`), the `.clamp(0,1)` is a no-op (`v/255 ≤ 1`), and
  `srgb_channel_to_linear(v/255.0) == u8_to_linear(v)` bit-exact for all `v`
  (already proven by `lut_matches_powf_helper_for_all_bytes`). For the additive
  fn the premultiply-back factor `* sa`/`* da` is `* 1.0` — exact in IEEE754.
  So on an opaque side the two per-channel `powf` decodes can be replaced by a
  256-entry LUT lookup with ZERO numeric change.
- **Action** — hoisted `src_opaque`/`dst_opaque` (`==255`) bools and, per side,
  substituted `u8_to_linear(_)` for the `powf` decode in both premul fns. No
  opaque copy short-circuit (encode∘decode is not provably identity; encode
  stays `powf`). Kept verbatim pre-change bodies as `#[cfg(test)]`
  `blend_premul_*_slow` reference fns and added
  `fast_path_matches_slow_over_all_bytes_and_boundaries` (exhaustive over all
  256 byte values × {255,200,128,1,0}² sa/da, covering both fast and slow
  branches) — passes, proving the refactor a no-op everywhere. Byte oracle vs
  `ships_perf_bench/clipper_pre1`: `diff -rq` empty (export_stamp filtered), all
  26 UI PNGs `cmp`-identical. `cargo test -p starbreaker-ui` green, freeze SHAs
  unchanged (`manifest_targets_whole_image_colour_regression_guard` fails on an
  unrelated pre-existing STALE-EXPORT timestamp guard only). Timing
  (informational, serial `RAYON_NUM_THREADS=1`, load ~2 at start): `render`
  stage 12.080s vs B1's 14.151s = −2.07s (−14.6%), consistent with dropping two
  `powf` decodes per opaque-pixel channel.

### Item-7: SWF parse-once + cached stage frames — LANDED (2026-07-18)

- **Observed** — `SwfAssetLibrary::new` decompressed+parsed each SWF ~8× (one
  full `decompress_swf`+`parse_swf` per extractor via the `with_parsed_swf!`
  macro), `stage_frame`/`stage_size` re-parsed `self.raw` on every call (per
  flash node), and `merge_swf_bytes` parsed 5 more times. Combined the SWF was
  decompressed+parsed ~20×/image. The two load timers `swf_load` (`:750`) and
  `swf_load_measure` (`:595`) summed to 3.302s (B1) — stage-cost rank #3.
- **Finding** — every extractor iterates the SAME tag slice; parsing once and
  running all extractors over `&parsed.tags` in one scope removes every
  re-parse, byte-identically. Stage frames are deterministic given the tag
  stream, so all main-timeline frame snapshots + the post-last-`ShowFrame` tail
  can be captured in one walk at construction and served from cache. `raw` was
  read only by the two stage accessors — droppable once stage data is cached.
- **Action** — added a single `parse_tags(&[u8]) -> swf::SwfBuf` choke point
  (with a `#[cfg(test)]` `SWF_PARSE_COUNT`) in `extract.rs`; deleted the
  `with_parsed_swf!` macro; split each extractor into a `*_from_tags(&[Tag])`
  core plus a thin `extract_*(bytes)` wrapper (the now-callerless
  `extract_main_timeline_labels` wrapper was dropped — its `*_from_tags` core is
  used directly and it was never re-exported). Two-pass structures preserved:
  `extract_fonts_from_tags` (DefineFont build then DefineFontInfo mutate),
  `extract_font_edit_text_metrics_from_tags` (ImportAssets then DefineEditText),
  and the `ShowFrame` counter in `extract_main_timeline_labels_from_tags`. Added
  `extract_all_stage_frames_from_tags -> (snapshots, tail)` in `stage.rs`
  (byte-exact reproduction of the old `extract_stage_frame` break-on-`==`/`>`
  loop, shared `apply_place_object` helper preserving the
  `previous`/`Modify`/`Replace` + `Matrix::IDENTITY` inheritance). `library.rs`:
  dropped `raw`, added cached `stage_size`/`stage_frames`/`stage_frames_tail`;
  `new` and `merge_swf_bytes` now parse once over `&parsed.tags`; stage
  accessors read the cache; `find_font_by_name` CharacterId-sorted determinism
  left untouched. New `library_construction_parses_once` test asserts exactly
  one decompress+parse per `new` and that cached `stage_size` matches the
  standalone extractor.
- **Lifetime deviation (amendment 5 / B3a)** — the literal
  `parse_tags(bytes) -> Vec<Tag>` in the design is not expressible: `Tag<'a>`
  borrows the decompressed `SwfBuf`, so tags cannot outlive it. `parse_tags`
  instead returns the owned `SwfBuf` and each caller does
  `let buf = parse_tags(b)?; let parsed = swf::parse_swf(&buf)?;` then runs all
  `*_from_tags(&parsed.tags)` in that scope. Same "one decompress+parse per
  construction" guarantee; no fully-owned Tag model (large, unnecessary).
- **Verification** — `cargo test -p starbreaker-ui` / `-p starbreaker-3d` green,
  freeze SHAs unchanged (only the unrelated pre-existing STALE-EXPORT timestamp
  guard `manifest_targets_whole_image_colour_regression_guard` fails). Byte
  oracle vs `ships_perf_bench/clipper_pre1`: `diff -rq` empty (export_stamp
  filtered), all 26 UI PNGs `cmp`-identical; two-run determinism `diff` empty.
  Timing (informational, serial `RAYON_NUM_THREADS=1`, machine load 1.6–5.5
  during capture — indicative): `swf_load` 1.654s → 0.666s (−60%),
  `swf_load_measure` 1.648s → 0.666s (−60%); combined stage-#3 cost
  3.302s → 1.332s.

### Item-8: DDNA→roughness parallel pre-decode — LANDED (2026-07-18)

- **Pre-check (borderline)** — Task-2 serial-vs-parallel probe measured a
  **4.09× parallel speedup over 50 DDNA sources** — above the 4× gate but only
  just; the keep-or-revert decision was deferred to the post-change stage-delta
  timing below.
- **Observed** — the first decode of each unique DDNA→roughness source happened
  serially inside the `child_assets` and `interior_assets` loops of
  `write_decomposed_export`. The existing `ddna_status_cache` memo only
  collapsed *repeat* touches of a source; the first full mip-decode +
  smoothness-statistics pass + PNG encode of each of the 212 unique Carrack
  sources was still paid serially on the writer thread.
- **Finding** — pre-decoding those 212 sources in parallel (`par_iter`) up front
  costs ~2.2s and removes ~9.7s of serial first-decode from the writer stages, a
  net ~8s wall win at byte-identity. Carrack stage deltas (baseline = 3 runs
  01:26–01:29 at low load; post = 2 runs 04:16–04:18 at 1-min load ~2.5, so
  **indicative** per the load<2 rule — but both post runs beat all three
  baselines despite higher load, so the gain is robust):
  - `prewarm_roughness` (new): 2.20s (212 sources)
  - `child_assets`: 6.12s → 4.78s (−1.34s)
  - `interior_assets`: 19.83s → 11.43s (−8.40s)
  - `[timing][decomposed] total`: 29.90s → 19.77s (−10.13s)
  - `[timing][blend] total`: 56.85s → 48.56s (−8.29s)
  - Max-RSS: 11.8G baseline → 12.72G (13,339,532 kB), **+7.8%** — the cost of
    holding 212 decoded `RoughnessTextureLoad`s (png + grayscale_png) resident
    during the write; under the 10% concern threshold, accepted.
- **Action** — added `prewarm_decomposed_roughness` (decomposed.rs) driven from
  the same `prewarm_assets` vec as `prewarm_decomposed_textures`, built in
  `blend_assembly.rs` beside `prewarmed_png_cache` and threaded as a
  **consult-only `&RoughnessCache`** (textures.rs alias:
  `HashMap<String, Result<RoughnessTextureLoad, RoughnessTextureLoadError>>`,
  keyed by RAW source path) down to `export_ddna_roughness_asset_with_status`,
  which consults it before `load_roughness_texture_result` on a miss.
  **CRITICAL trap honoured (D5):** `ddna_status_cache` is NEVER prewarmed — its
  cache-hit early-return precedes the `files`/`texture_cache` inserts, so seeding
  it would drop every roughness PNG. The prewarm holds ONLY the pure decode
  `Result`; the serial writer keeps every bookkeeping insert. New gated unit test
  `prewarmed_roughness_matches_cold_and_emits_png` proves the prewarmed path
  yields an identical `(ref, status)` AND emits the roughness PNG into `files`.
- **Amendment-11 note (read the delta correctly)** — layer submaterials resolved
  via `resolve_layer_submaterial` (decomposed.rs, the `.mtl`-layer branch in
  `extract_material_entry`) are NOT in the prewarm enumeration and still decode
  serially on first touch. This is byte-safe (a miss falls back to the identical
  serial decode) and means the parallel speedup is partial — the timing delta
  above already reflects that residual serial cost.
- **Verification** — `cargo build` + `cargo test -p starbreaker-3d --lib` green
  (473 pass incl. the new gated test with SC_DATA_P4K set; skips without).
  Byte oracle vs `ships_perf_bench/{carrack_pre1,clipper_pre1}`: `diff -rq`
  empty (export_stamp filtered) on BOTH ships. Two-run Carrack determinism:
  `diff` empty. RSS +7.8% (above).

### Item-12.1 + 12.2: per-CGF path precompute + prewarm mtl-cache reuse — LANDED (2026-07-18)

- **Observed (12.1)** — the interior placement loop in `write_decomposed_export`
  recomputed `normalize_source_path` for every placement's CGF path (and, when
  present, its material path) **twice** — once to build the `interior_asset_lookup_key`
  cache key (decomposed.rs, per-placement block) and again when constructing the
  `InteriorPlacementRecord`. Placements re-reference a much smaller
  `input.interiors.unique_cgfs` set by `mesh_index`, so on a capital ship this was
  ~24k redundant normalizations over a few hundred distinct CGFs.
- **Finding (12.1)** — normalizing each unique CGF once (a `Vec<PrecomputedCgf>`
  indexed by `mesh_index`, built before the container loop) and looking up
  `cache_key` / `normalized_cgf_path` / `normalized_material_path` per placement
  yields byte-identical output: the values are identical strings and the
  first-seen `case_index` only records what is written, which is unchanged.
- **Action (12.1)** — added a local `PrecomputedCgf { normalized_cgf_path,
  normalized_material_path, cache_key }`; the per-placement recomputation and the
  `InteriorPlacementRecord` recomputation both now clone from `precomp`.
- **Observed (12.2)** — `prewarm_decomposed_textures` built a local
  `mtl_cache: HashMap<String, Option<MtlFile>>` while resolving each asset's
  canonical source `.mtl` + sidecar slot paths, then **dropped it**. The serial
  writer's own `mtl_cache` then re-parsed those same `.mtl` files on first touch.
- **Finding (12.2)** — the mtl_cache is a pure path→parse memo consulted by key,
  so seeding the writer's cache with the prewarm's entries avoids re-parsing with
  zero output effect and introduces no new nondeterminism (retrieval is by key;
  iteration order is irrelevant to output).
- **Action (12.2)** — `prewarm_decomposed_textures` now returns
  `(PngCache, HashMap<String, Option<MtlFile>>)`; `blend_assembly.rs` destructures
  it (`prewarmed_mtl_cache`, logged in the `[timing][blend] prewarm_textures`
  line — 115 mtl entries on Carrack) and threads it into a new
  `write_decomposed_export` `mtl_cache` param (order `png_cache, roughness_cache,
  mtl_cache`, composing with the Item-8 signature). The single blend caller passes
  the prewarmed memo; a second caller would pass `HashMap::new()`.
- **Timing (informational, INDICATIVE — 1-min load ~5.8, above the load<2 rule;
  benchmark window pressure)** — Carrack stage deltas vs Item-8 post numbers:
  - `child_assets`: 4.78s → 4.44s (−0.34s)
  - `interior_assets`: 11.43s → 9.76s (−1.67s)
  - `[timing][blend] total`: 48.56s → 44.11s (−4.45s)
  These beat the Item-8 post numbers despite ~2× higher load, so the direction is
  robust even though the magnitudes are not clean. No new parallelism was added
  (12.2 seeds a memo; 12.1 removes redundant serial work), so no determinism
  double-run is required for this task.
- **Verification** — `cargo build` + `cargo test -p starbreaker-3d --lib` green
  (472 pass). Byte oracle vs `ships_perf_bench/{carrack_pre1,clipper_pre1}`:
  `diff -rq` empty (export_stamp filtered) on BOTH ships.

### Item-2: canonicalize mesh + material sidecar paths to P4K-entry casing — LANDED (2026-07-18)

- **Observed** — three divergent casing authorities in `decomposed.rs`:
  `texture_relative_path`→`normalize_source_path` used **P4K-entry casing**
  (correct); `mesh_asset_relative_path` discarded its `p4k` param (`let _ = p4k;`)
  and used `normalize_requested_source_path` (source-string casing);
  `material_sidecar_relative_path`→`normalize_material_source_for_manifest`
  **force-lowercased** after `Data/`. The accumulating export tree at
  `ships/Data` therefore carried dual-cased `objects`/`Objects` (1.6G/5.0G) and
  `materials`/`Materials` (26M/14M) directories, and per-run casing varied.
- **Finding** — mesh/material builders diverged from the texture (P4K-entry)
  authority. Because Blender ID names inside each `.blend` derive from the mesh
  asset stem, the lowercase builder also emitted **case-duplicated objects**
  (one lowercase, one P4K-cased Object instance for the same mesh) — the
  "case-dup export bug". Unifying on the P4K authority collapses the duplicate.
- **Action** — `mesh_asset_relative_path` now builds from
  `normalize_source_path(p4k, geometry_path)`;
  `normalize_material_source_for_manifest(p4k, path)` and
  `material_sidecar_relative_path(p4k, …)` gained a `p4k` param and delegate to
  `normalize_source_path`. `p4k` threaded to all callers (five `.map` closures
  at the paint/child/interior sites, `canonical_material_source_path`,
  `reusable_interior_asset_paths`, `projected_material_sidecar_path`, and the
  build-material-sidecar site). `OutputFiles.case_index` retained as
  belt-and-braces. The three lowercase-asserting unit tests
  (`_uses_source_mtl_not_geometry_path`, `_encodes_mip_level`,
  `_normalizes_case`) were removed and subsumed by a new data-gated
  (`SC_DATA_P4K`) integration test
  `path_builders_use_p4k_entry_casing_regardless_of_input_case` that asserts the
  builders adopt P4K-entry casing (and still cover source-vs-geometry identity +
  mip encoding). Note: `datacore_path_to_p4k` still strips a `Data/` prefix
  case-sensitively — an all-caps `DATA/` prefix is a separate, pre-existing
  concern (shared with the texture authority) not addressed here; the test
  varies only the path body.
- **Case-modulo oracle (2026-07-18, INDICATIVE — machine loaded)** — vs
  `ships_perf_bench/{clipper,carrack}_pre1` (confirmed byte-identical to HEAD
  `f4dd29c9b` before this change, so a valid baseline): case-folded relative
  path sets match **1:1** on both ships; every non-`.blend` file (JSON/PNG) is
  identical modulo ASCII path casing. `.blend` files differ beyond casing but
  **only** by (a) P4K casing re-cased into embedded Blender ID names and (b)
  removal of the case-duplicated Object — proven benign: SDNA block inspection
  of a diverging pair shows Mesh 6=6, Material 8=8, MDeformVert 6=6,
  AttributeArray 48=48 (264384 B both), raw_data bytes identical (1953618 B);
  only `Object 9→8`, `CollectionObject 9→8`, and two empty `raw_data` stubs
  removed. No geometry/material/vertex data lost. Two-run determinism
  (`clipper_item2_a` vs `_b`) diff empty (exact).
- **Cleanup — BLOCKED-on-cleanup (deferred to owner)** — the strict
  byte-subset/merge guards fail on stale historical content, so per the handoff
  guard the destructive deletions were NOT performed:
  - `ships/Data/objects` → `Objects`: **0** files unique to lowercase (Objects
    is a strict path-superset, 1036 unique to it); of 54 shared-path diffs, 39
    are case-only (stale pre-fix casing) and **15 genuine** (9 `.blend` via the
    benign case-dup mechanism, 6 `.materials.json` — stale-version content).
  - `ships/Data/materials` → `Materials`: **0** unique to canonical, 14
    entries (incl. whole `bhvr/` subdir) unique to lowercase, **3** genuine
    shared-path `.materials.json` diffs (stale versions).
  Both trees are historical accumulations exported at different code versions;
  the discrepancies are stale artifacts, not fix defects, but the guard mandates
  stop-and-report on any non-case difference. ~1.6G+ reclaim pends owner sign-off.
- **Fresh baselines** — `ships_perf_bench/clipper_post1` (42.6s wall, 9.6G RSS)
  and `carrack_post1` (46.6s wall, 13.3G RSS) captured with `/usr/bin/time -v` +
  `RUST_LOG=info` (one run each, machine loaded → INDICATIVE). These **replace**
  `*_pre1` as the byte oracles for later tasks (e.g. Item-7 zstd swap); the
  `*_pre1` dirs are retained for orchestrator-managed retirement.
- **Verification** — TDD: gated invariant test FAILS pre-fix (mesh path
  `Data/OBJECTS/…` vs `Data/objects/…` case-unstable), PASSES post-fix.
  `cargo test -p starbreaker-3d --lib` green (469 pass, 3 ignored).
  `cargo test -p starbreaker-ui` freeze SHAs green (9/9; the known
  `manifest_targets_whole_image_colour_regression_guard` stale-export mtime
  exception is unrelated).

### Item 10 — P4K zstd decode via the `zstd` crate (2026-07-18, `feature/ui`)

- **Observed** — the Task-3 decode-time probe (item-10 gate) measured ~85 CPU-s
  spent in `zstd_decompress` against ~59s wall on the decomposed capital-ship
  export, i.e. zstd decode is a >5% share of the run (gate PASSED).
- **Finding** — the C `libzstd` binding (`zstd` crate) decodes materially faster
  than the pure-Rust `ruzstd` streaming decoder, and its output is byte-identical
  by format (both emit the same decompressed payload; the P4K stores standard
  zstd frames). `zstd = "0.13"` is already linked workspace-wide via
  `starbreaker-blend` and `mcp`, so the swap adds no new toolchain.
- **Action** — swapped `starbreaker-p4k::archive::zstd_decompress` from
  `ruzstd::decoding::StreamingDecoder` to `zstd::stream::copy_decode` (size-hint
  preallocated `Vec`); dropped `ruzstd = "0.8"` from `starbreaker-p4k/Cargo.toml`,
  added `zstd = "0.13"`. Signature unchanged.
  **Partial-consolidation tradeoff (amendment 3 / planner F3):** `starbreaker-chf`
  is left wholly on `ruzstd` (both decode AND encode). chf encode (`to_chf`,
  container.rs:82) writes game-consumed 4096-byte save files and is off every
  byte oracle — no test proves the game accepts a `zstd`-crate frame — so it must
  keep `ruzstd`. Swapping only chf decode would ADD `zstd` while NOT removing
  `ruzstd` from chf (negative consolidation), and chf decode is off the export
  hot path. So `ruzstd` remains a workspace dependency (chf only); this item
  consolidates the P4K decode path solely.
- **Verification** — `cargo build` + `cargo test -p starbreaker-p4k` green
  (13 unit + 8 real-P4K integration incl. `open_real_p4k`, `read_socpak_as_zip`,
  `read_encrypted_entry` — all exercise zstd decode); `cargo test -p
  starbreaker-3d --lib` green (469 pass, 3 ignored). Byte oracles vs the NEW
  post-A6 baselines `ships_perf_bench/{clipper,carrack}_post1`: `diff -rq …
  | grep -v export_stamp` **empty** for BOTH ships.
- **Timing (INDICATIVE — machine loaded, load avg ~6–8 during a concurrent
  review)** — internal `[timing]` `write_decomposed_export`: clipper 30.67s,
  carrack 32.81s, both below the loaded post1 wall captures (clipper 42.6s /
  carrack 46.6s). Directional improvement, but not a clean comparison — re-bench
  in a quiet window for a defensible delta.

### Addon: resolve_path lazy index

- **Observed** — `PackageBundle.resolve_path` rglob'd the whole shared
  `ships/` root (~9,150 files, grows per ship) on every call via an
  unconditional `_build_path_index()`, though manifests store normalised
  paths that almost always hit the direct `export_root/candidate` check
  (bottleneck #1, `docs/blender-import-export-performance.md`, never applied).
- **Finding** — the walk is pure waste on the direct-hit path; the index is
  only needed as a case-insensitive fallback after all direct candidates miss.
- **Action** — one-loop lazy init: `path_index=None`, per candidate try
  `direct.exists()` then lazily build+consult the index on first miss;
  precedence unchanged. Regression `test_direct_hit_does_not_build_path_index`
  asserts `_path_index is None` after a direct hit. (commit `e9ef925fe`)

### Addon: POM bias reads one pixel not the atlas

- **Observed** — `_height_image_background_bias` copied the entire height
  image (`pixels[:]`, ~500 MB transient for 2048² RGBA, twice) to read the
  top-left pixel; `_luma` only touches `buf[0:3]`. The import has OOM'd at
  12 GB+ (dead-end 2).
- **Finding** — `pixels[:]` materialises the whole buffer; a bounded
  `pixels[0:4]` slice yields the same four floats.
- **Action** — `pixels[:]` → `pixels[0:4]` at both sites (temp-load :326 and
  fallback :333); `orchestration.py:1247` (a real full-image loop) untouched.
  Behaviour-identical; `PomBiasSliceTests` guards the slice bound.
  (commit `30f3f60ca`)

### Ops: disk retention (item 5, re-scoped)

- **Observed** — handoff premised a 46–66 G reclaim from `target/debug`
  (68 G). Verified 2026-07-18: no `target/debug` tree exists; disk ~83 %
  used / 187 G free — the reclaim target is void.
- **Finding** — `dcb_canvas` (3.0 G) is read by the starbreaker-ui live-IR
  harness (mod.rs:253) — the handoff's "nothing reads it" is wrong; deleting
  it silently disables the guard. Only stale data left = `graphify-out` old
  snapshots (15 dated dirs).
- **Action** — pruned `graphify-out` to the 3 newest snapshots + live
  `graph.json`; added a retention-policy note to `AGENTS.md §Building`
  (cargo-sweep any regrown debug tree, keep the release-deps ladder, keep
  `dcb_canvas`). `dcb_canvas` retention surfaced to the owner. (commit `fede2bbb7`)

### Addon: layout_key data_pointer drop — REJECTED

- **Observed** — proposed removing `data_pointer` from `layout_key`
  (`orchestration.py:746–758`) so distinct meshes sharing a slot layout reuse
  one cache entry (keeping it in `slot_mapping_cache`, :657).
- **Finding** — guard failed: the layout-cache hit path
  (`orchestration.py:763–776`) skips
  `_restore_generated_decal_host_variant_polygons` (`builders.py:2943`,
  mutates polygons per-mesh) and `_rebind_mesh_decal_for_host`
  (`builders.py:4849`, nearest-host spatial rebind reading vertex positions),
  both per-mesh and geometry-dependent, not part of the cache key. With
  `data_pointer` dropped, two distinct meshes could share an entry and the
  second mesh's per-mesh fixes would never run — a correctness regression, not
  a byte-identical win.
- **Action** — REJECTED, no code change. Would be dischargeable only by proving
  both per-mesh fixes are no-ops whenever the keyed inputs match (unlikely: the
  rebind reads live vertex positions). (no commit — ledger record)

### Release checklist: git history scrub (item 13, DO NOT run now)

- **Observed** — `.git` size-pack ~1.22 GiB; ~821 historical `ships/Data`
  blob paths (~1,155 MiB of once-committed, now-ignored export artifacts) ship
  with every public clone.
- **Finding** — a history rewrite removes them but rewrites all commit hashes —
  destructive, never mid-arc.
- **Action (PRE-PUBLIC-RELEASE ONLY, owner-scheduled)** — on a fresh clone run
  `git filter-repo --path ships --invert-paths`; verify the 4 legitimately
  tracked binaries survive (`material_templates.blend`, `pom_library.blend`,
  `screen_effects_library.blend`, the app icon); force-push once; re-clone
  everywhere. Pre-conditions: `.gitignore` covers `ships/`, `dcb_canvas`,
  `graphify-out`. Verify: `git count-objects -vH` size-pack shrunk ~1.15 GiB;
  `cargo build` + tests green on the rewritten clone. (no commit — checklist
  record)

### Spike: PNG fast-encode (item 9, report-only, not landed)

- **Observed** — texture-export PNG encode via `image` 0.25.10
  (`textures.rs::encode_png` :193 / `encode_png_rgba` :1098), both using
  `img.write_to(..., ImageFormat::Png)`. Hypothesis: default DEFLATE is slow;
  `PngEncoder::new_with_quality(CompressionType::Fast, FilterType::Adaptive)`
  would trade size for encode speed.
- **Finding** — **NO-OP on image 0.25.x.** In `image` 0.25.10 the default
  `write_to(Png)` path (`PngEncoder::new`) already uses
  `CompressionType::default()`, and `CompressionType`'s `#[default]` variant is
  `Fast` (codec doc: "The default setting is `Fast`"), with `FilterType`
  defaulting to `Adaptive`. The proposed change sets those exact same two
  values, so it is byte-for-byte identical to the current code. Measured on a
  Clipper texture export (`drak_clipper`, decomposed, lod 0, mip 0,
  `--materials all`, 788 PNGs): PNG-encode aggregate wall (rayon-summed via a
  temporary `AtomicU64` probe) 19.46s → 20.21s (within load noise; load ~2.4);
  total PNG size 2,442,424,970 → 2,442,424,970 bytes (**0 B, +0.00 %**);
  `diff -rq` base vs fast = only `.export_stamp.json` differs; decoded pixels
  byte-identical over 788 PNGs (0 mismatch, 0 missing) vs the `clipper_post1`
  oracle. There is no size-vs-speed tradeoff to make here — the current default
  IS the fast path.
- **Action** — SPIKE ONLY, reverted (`git checkout --` on
  `pipeline/textures.rs`, `pipeline/mod.rs`, `decomposed.rs`; probe removed);
  patch at `scratchpad/item9-png-fast.patch`. To actually cut encode time or
  size the owner would need a different lever (e.g. `Best` for smaller/slower,
  a non-`image` encoder, or fewer/smaller textures), not this switch. UI/holo
  PNGs remain excluded (freeze gate + gfx encoders). `cli/src/dds.rs`
  (standalone `dds` subcommand, off the export path) left untouched — optional
  follow-up only. (no commit — ledger record)

### mcp: tokio feature trim

- **Observed** — `mcp/Cargo.toml` pulled `tokio` with `features=["full"]`
  though the only runtime use is a single-task stdio server.
- **Finding** — `full` compiles the entire tokio surface (net, fs, time,
  multi-thread scheduler) that the stdio handshake never uses. The whole
  server is `#[tokio::main]` + `.serve(stdio).await` / `.waiting().await`.
- **Action** — `#[tokio::main(flavor = "current_thread")]` +
  `features=["rt","macros","io-std"]` (no extra feature needed — build was
  clean); rebuilt + redeployed; initialize handshake smoke green over stdio.
  Compile-time/dep-graph win only: release `-p starbreaker-mcp` rebuild
  45.33s → 28.29s (~17s, fewer tokio deps compiled). (commit `83a07950b`)
