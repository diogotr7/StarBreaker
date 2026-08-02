# Performance review — ship extraction end-to-end (2026-08-02)

Five-surface multi-agent review (export hot path, UI extraction, memory/disk/infra,
Blender import addon, radical replatforming), grounded on `docs/optimisation-ledger.md`
and the 2026-07-18 pass. Review was read-only; top claims were verified against the
files by the orchestrator before inclusion. Reference workload: Carrack decomposed
lod0 mip0 materials-all — ~44 s blend total, 13.3 G peak RSS, 2417 files / 3.1 G
output (1334 PNGs = 2.99 GB); Clipper ~42–46 s.

## TL;DR

| # | Finding | Est. gain | Risk | Class |
|---|---|---|---|---|
| E1 | Mesh-free sidecar material view (mesh half of `build_decomposed_material_view` is dead in the writer) | 1.5–4 s | low | non-radical |
| U1 | Opaque-source blend = exact copy (ledger item-6 caveat OVERTURNED, verified exhaustively in f32) | 2–4 s of render | low | non-radical |
| U2 | Per-blit 256-entry tint LUT in `blit_atlas_image_tinted_with_mode` (3 powf/pixel → pure fn of one byte) | 1.5–3 s of render | low | non-radical |
| M1 | `png_cache` + roughness cache drain-on-consume | −3 G RSS (13.3→~10 G) | low | non-radical |
| M2 | Prewarms ignore `--skip-existing-assets` (live waste bug) | prewarm cost on every incremental run | low | non-radical |
| E2 | Dead metallicRoughness PNG encode per DDNA source on the status path | ~1 s + 0.5–1.5 G RSS | low | non-radical |
| E3 | Roughness prewarm enumerates the wrong material set (canonical + layers missing) | 1–3 s | low-med | non-radical |
| A1 | Addon: O(n²) `obj.children` tree walks (3 sites) | secs–min on import | low | non-radical |
| R1 | Streaming output flush (kill the 3.1 G in-RAM `BTreeMap`) + parallel write | −RSS to working set | med | RADICAL (4–6 pd) |
| R2 | `-C target-cpu` + fat LTO (no `.cargo/config.toml` exists; every `.round()` is a libm call) | unknown, possibly large | oracle-gated | RADICAL-ish spike |
| D1 | Targeted `target/release/deps` sweep (7.5 G, ladder is only 0.48 G) | 5–6 GB disk | low | housekeeping |

## Likely live bugs (verified)

- **B1 — HDR gobo export is dead.** `decomposed.rs:4020` calls
  `p4k.entry_case_insensitive(source_path)` on the RAW `light.projector_texture`
  string; `cmp_lower_against` does exact byte compare (no separator folding), so
  forward-slash DataCore paths never match backslashed P4K entries. Every sibling
  reader converts via `datacore_path_to_p4k` first. Corpus evidence: 1757
  projector refs, **0 `.exr`** in `carrack_post1`. The BC6H branch never fires;
  HDR gobos silently degrade to SDR PNG. One-line fix + gated test.
- **B2 — `material_sidecar_default_palettes` always empty.**
  `blend_assembly.rs:527` filters `manifest_files` for `.materials.json`, but
  sidecars are classified `MaterialSidecar` and never enter `manifest_files`
  (only `PackageManifest` does, `:1176-1181`). The `sidecar_palette_id` fallback
  at `scene_manifest_instances:600-624` is unreachable. Probe whether instances
  always carry explicit `palette_id`; then fix the plumbing or delete the fallback.
- **B3 (addon) — POM post-passes never run on the file-open path.**
  `_bake_pom_mesh_bitangent_signs` / `_apply_deferred_pom_background_bias` are
  called only from `import_scene` (`orchestration.py:576,579`); the load-post
  refresh rebuilds POM materials without either, and the Rust exporter writes no
  `starbreaker_bitangent_sign` attribute. Verify against a real file open.
- **B4 (addon) — height-bias failure retried forever** (`builders.py:292-341`):
  failure result never cached; every POM material re-does `images.load` + full
  decode. Negative-cache it.
- **B5 (addon) — material cache-key collision** (`materials.py:526-535`):
  `_reusable_material` accepts by name without comparing `PROP_MATERIAL_IDENTITY`
  to the incoming key, and ignores `ui_image_path` when `None` — two keys can
  share one datablock. Correctness, not perf.
- **B6 (addon) — dead memo on the shipping path** (`package_ops.py:332`):
  `_material_refresh_palette_id` call omits the `default_palette_cache` arg, so
  the `:543-547` memo never engages. One-word fix.
- **B7 (UI) — two divergent `pixmap_to_rgba_image` unpremultiply implementations**
  (`compose/blit.rs:84` integer vs `ir_compose/engine_01.rs:2835` float) — can
  differ by 1 LSB; nothing broken today, unify deliberately with an oracle run.
- **B8 (addon) — `_purge_orphaned_file_backed_images`** (`validators.py:135`)
  removes ANY zero-user file-backed image, including user-loaded ones.

## Zero-risk / cheap wins (byte-identical, non-radical)

**Rust export (starbreaker-3d):**
1. **E1** `build_decomposed_material_view` (`decomposed.rs:622-871`) is called
   2–3× per asset; in `write_decomposed_export` only `sidecar_materials` +
   `sidecar_original_indices` are consumed — `write_mesh_asset` takes `_mesh` and
   writes an empty placeholder (verified `decomposed.rs:2471-2492`). Add a
   mesh-free `sidecar_material_view` for the writer sites (`:1149,:1352,:1683`,
   `blend_assembly.rs:962`); keep the full builder where `.mesh` is real. Both
   outputs are pure functions of `materials` alone (branch-by-branch argument in
   the agent report); unit-test equivalence over fixtures.
2. **E2** `load_roughness_texture_result` (`textures.rs:1277-1280`) encodes an
   RGBA metallicRoughness PNG per DDNA source that the decomposed path never
   reads (only `grayscale_png` is consumed at `decomposed.rs:5066`;
   `slot_texture_flavor` never returns `Roughness`). Skip it on the
   status/prewarm path; keep for the GLB path. ~1 s + 0.5–1.5 G RSS.
3. **E3** `prewarm_decomposed_roughness` (`decomposed.rs:4906-4929`) enumerates
   raw `materials.materials`, while the writer extracts from the CANONICAL
   source `.mtl` and also walks layer submaterials — both miss the prewarm and
   decode serially (ledger amendment-11 + one undocumented gap). Enumerate from
   the same `sidecar_materials` + layer walk. NEVER seed `ddna_status_cache`
   (D5 trap).
4. **E4** `blend_assembly.rs:1006` clones the whole `(mesh, mtl, nmc)` tuple to
   feed a view builder, then returns `loaded` anyway. Borrow instead.
5. **E5** `scene.json` (8.36 MB) parsed 4–5× in `blend_assembly`
   (`:1056,:1191,:1281,:1346`; mutation at `:1242-1268`). Parse once, rebuild
   after the one mutation. ~0.3–0.5 s.
6. **E6** `json!` macro round-trips pre-built `Value`s through `to_value`
   (`decomposed.rs:2949,2953-2959,2998,2852-2877`) — full deep rebuild instead of
   a move. Explicit `Map::insert`. ~0.3–0.8 s.
7. **E7** Parallel final write + parent-dir dedupe (`cli/src/entity.rs:646-656`,
   `write_decomposed_file:80-104`): 2417 files serial with `create_dir_all` per
   file. ~0.5–1.5 s.
8. **E8** Add a `preload_interior_meshes` timer — it currently hides inside
   `base_decomposed_export`'s phase timer (`blend_assembly.rs:913-1050`) and is
   likely the peak-RSS driver. Instrumentation only.

**Memory (starbreaker-3d):**
9. **M1** `png_cache` entries are consumed at most once (`texture_cache`
   short-circuit at `decomposed.rs:3614` precedes every consult — verified
   including `:3057/:3073` which route through `export_texture_asset`, and
   `generated_ui_texture_for_binding` takes `_png_cache` unused). Drain on
   consume (−2–2.5 G). Same one-shot property for `RoughnessCache` via
   `ddna_status_cache` (−0.9 G). Combined: 13.3 G → ~10 G, under the 12 G OOM line.
10. **M2** Neither prewarm receives `existing_asset_paths`
    (`blend_assembly.rs:978,:989`) — on `--skip-existing-assets` re-exports the
    full parallel decode is paid and discarded. Filter prewarm jobs with the same
    key the writer's skip check uses.

**UI render (starbreaker-ui) — all provably byte-identical:**
11. **U1** Opaque-source short-circuit in `blend_premul_linear` /
    `blend_straight_linear` (`colour.rs:72,:48`). Orchestrator-verified: the f32
    encode∘decode round-trip is identity for all 256 bytes (0 mismatches,
    rustc -O), so `if src[3]==255 { *dst = src; return; }` is bit-exact. Ledger
    item-6's "not provably identity" is hereby overturned with evidence. Add the
    exhaustive test in the existing style. Est. −2–4 s of the 12.08 s render.
12. **U2** `blit_atlas_image_tinted_with_mode` (`ir_compose/engine_01.rs:2778-2833`)
    computes 3 powf/pixel where the pre-alpha expression is a pure function of one
    u8 + loop-invariant tint. Hoist a per-blit `[f32;256]` LUT (768 powf total).
    Est. −1.5–3 s.
13. **U3** `strip_custom_shape_uniform_matte` (`engine_01.rs:1162-1199`) returns
    `img.clone()` on all no-op exits of a buffer the caller owns. Take by value.
14. **U4** `env::var` (process-global lock) per node under rayon at
    `engine_01.rs:475`, `swf_draw.rs:122`, `bb_layout/engine_02.rs:2070` →
    `OnceLock`, matching `font_telemetry_enabled`.
15. **U5** Reorder cheap predicates before the O(nodes) scan in
    `center_anchored_heading_textfield_text_rect` (`engine_01.rs:1927-1955`);
    id→node HashMap for the linear `find`-by-id sites (`:1381,:1979`,
    `hybrid_compose.rs:74`).
16. **U6** SWF glyph handling (`text/swf_draw.rs`): code→glyph map per
    `FontGlyphSet` (currently O(glyphs) linear scan per char), memoised
    `swf_glyph_to_path` per (font, index). Skip the incremental wrap-width
    variant (f32 order hazard).
17. **U7** `draw_swf_font` allocates a full-canvas 23 MB Pixmap per text node
    (`swf_draw.rs:185`), read back only over the clip rect — allocate at the
    integer clip rect. Oracle-check (integer-boundary clipping argument says
    byte-identical).
18. **U8** `CanvasNameIndex::build` runs per binding (`canvas_fetcher.rs:102`),
    before the stage timers (invisible in the table). Build once per export,
    share as `Arc`. Est. −0.3–2.5 s of prerender wall.
19. **U9** `AtlasLibrary` created per binding; SVG path bypasses its cache
    entirely (`bb_atlas/mod.rs:110,119`) — same icon fetched/inflated/parsed/
    rasterised per node per render. Add bytes + rasterised-SVG caches (key must
    include manufacturer for the MFD fallback), share per export. Est. −0.5–2 s.
20. **U10** `ir_compile` deep-clones the whole scene incl. every node's raw JSON
    (`ui_ir/engine_01.rs:639`); mutations touch only text nodes. `Arc<Value>` +
    `Arc::make_mut` (wide but mechanical). Est. −0.5–1.5 s.
21. **U-dead** Dead code: `UiAssetResolver::fetch_dds_as_rgba` (no callers);
    unreachable `postprocess/passes.rs` chain (`apply_postprocess: false`
    hardcoded, `ui_pipeline.rs:158`).

**Blender addon:**
22. **A1** `obj.children` is O(len(bpy.data.objects)) per access; tree walks at
    `orchestration.py:611-614`, `package_ops.py:1047`, `ui.py:1846` are O(n²).
    Build one parent→children dict per pass.
23. **A2** `resolve_path` memo on `PackageBundle` (`manifest.py:854-865`) —
    ~200–300 k stats per big ship.
24. **A3** glTF template import scans all objects twice per template
    (`orchestration.py:1335,1344`) — import into a scratch collection.
25. **A4** Gobo mean-luminance is a ~4 M-iteration Python pixel loop with a
    ~500 MB `pixels[:]` copy (`orchestration.py:1247-1253`) — numpy
    `foreach_get` is 50–100× (float accumulation order changes the Strength
    value negligibly — functional, not byte, contract).
26. **A5** Misc: submaterial JSON double-parse (`utils.py:96-104` — identity
    hash already compared), POM cache-hit full-tree re-walk (`groups.py:2185`),
    POM group append O(H²) rescans (`groups.py:2196-2236`), `batch_remove` in
    orphan purges (`validators.py:99-159`), per-redraw full-scene poll
    (`ui.py:1190`), template-time tangent bake double-covered
    (`orchestration.py:1306` — blocked on B3 verification).

## Storage / infra

- **D1** `target/release/deps` = 7.5 G; the bisect ladder is 64 binaries /
  0.48 G; up to 30 stale generations per crate. Targeted sweep (keep ladder +
  newest generation) reclaims **5–6 GB**. AGENTS.md's "~1.2 G ladder: leave it"
  is 6× stale, and its `cargo sweep` remedy points at the wrong tree (debug is
  all fresh-atime; release is the stale one) — correct the note.
- `target/tmp` empty; `*_pre1` baselines already retired; `dcb_canvas` KEEP;
  `ships/Data` case-dup (~1.5 G) stays blocked on owner; git scrub stays
  pre-public-release.
- Export I/O clean: no fsync, one write per file, `MappedP4k` is pread not mmap
  (RSS is all heap → M1/R1 are the whole story).

## Radical axis — verdicts (replatforming agent, web-verified 2026-08-02)

| Axis | Verdict | Note |
|---|---|---|
| PNG encoding | KEEP (+ filter spike) | Already on fdeflate UltraFast — item-9's "no-op" explained. fpnge/fpng/mtpng/zune/spng all REJECTED (no crate / dormant / loses fdeflate / decode-only). Encode ≈1.2 s of 44 s wall. |
| DDS decode | KEEP | bcdec_rs is the ecosystem's fastest; no SIMD Rust BCn decoder exists; GPU decode REJECTED (no crate, readback-bound). |
| Rasterisation | KEEP tiny-skia 0.12 | Custom compositor is where the time lives, not tiny-skia. vello_cpu 0.1.0 is 4 days old, ARM-only benchmarks, no determinism guarantee — REJECTED for SHA-frozen output; skia-safe REJECTED (C++ build, <2×). SPIKE: resvg 0.42→0.47 (glyph caching + MPL→MIT/Apache; re-freeze event). |
| .blend writing | KEEP | glTF path exists and was rejected (drops custom properties, slower import); no production Rust USD writer (drops int64/uint/double/half anyway). Check Blender 5.2 opens generated files (0.5 pd). |
| Process model | **SWITCH — R1** | `OutputFiles: BTreeMap<String,Vec<u8>>` holds all 3.1 G before the serial write loop — the structural cause of 13.3 G RSS. Streaming flush (only `contains_key` is hot; `insert_unique_path`'s `files.get` dedup probe is the one trap) + par_iter write. 4–6 pd, byte-identical. Per-ship sharding REJECTED (multiplies footprint; `xargs -P` covers batching free). |
| Output format | KEEP PNG (+ packing spike R4) | DDS not Blender-loadable (BC7 gap), EXR bigger, zstd-raw unreadable, WebP-lossless deferred. Real lever: single-channel maps written as RGBA8 (284 roughness PNGs = 944 MB) → Luma8/Rgb8 cuts size AND encode time. Semantic change, needs owner + re-freeze. |
| Build/allocator | SPIKE ×3 — R2 | No `.cargo/config.toml`: baseline x86-64, every `.round()` is libm. `target-cpu=x86-64-v3` + fat LTO + cg-units=1: oracle-gated, non-portable binary → local profile only. mimalloc: after R1 only. jemalloc stays dead. |

## Suggested sequencing

1. Bug fixes B1 (gobo), B6 (one-word), B4 (negative cache); probe B2.
2. Rust quick wins, one oracle run per batch: U1+U2+U3+U4+U5, then E1+E4+E5+E6, then M1+M2+E2+E3, then E7+E8+U6.
3. Addon wins A1–A5 (functional-identical, test via existing addon tests).
4. D1 sweep + AGENTS.md correction.
5. Re-bench (quiet machine, load < 2, 3+ runs, perf_bench.sh) → new baseline.
6. Larger diffs re-ranked after a sub-render probe (U7–U10).
7. Radical items on owner decision: R1 streaming flush, R2 target-cpu/LTO spike,
   R3 resvg upgrade (re-freeze), R4 channel packing (re-freeze + addon), R5
   BLEND_ZSTD_LEVEL 3→1 (re-freeze), R6 wire up MaterialRefreshSession
   (UX change), R7 cooked-material cache arc, R8 measure Blender import wall
   first (1 pd — likely the real end-to-end bottleneck).

## Items needing the owner's decision

R1–R8 above, plus: ships/Data case-dup deletion (~1.5 G, still blocked), B3
verification scope (needs a real Blender file-open), B5 material-identity check
(may change which material wins on collision), whether `--mip 1` is acceptable
for any workflow (quarters the texture set for free).
