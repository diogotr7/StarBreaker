# Handoff — remaining items from the 2026-08-03 perf review

Parent report: `2026-08-03-perf-review.md`. Landed this session (commit on
`feature/ui`): EP1 layer-texture prewarm, EP2 dead-branch write fix + restored
[mem] instrumentation, AL1 TrackingAllocator deletion, EP3 partial (parallel
lowercase + par_sort lower index), EP4b Arc<MtlFile> memo, U2 SWF parse-once
across measure/compose, U6a SWF fetch binary search, U6b dead SHA-256 removal,
A4 (B3a POM bias on both refresh paths), A7 orphan flush on blocking path.
All Rust changes byte-oracle-verified vs `ships_perf_bench/{carrack,clipper}_aug02`
+ two-run determinism clean.

## Shared preamble (binding)

- Read `AGENTS.md` first: TDD, no hard-coded game-data values, no per-asset
  gates, `//!` headers, commit directly on `feature/ui`, NO trailers.
- Byte-identity: `scripts/perf_oracle.sh ../ships_perf_bench/<ship>_aug02
  <fresh export> [--determinism <entity>]` after every batch. Bench:
  `scripts/perf_bench.sh <entity> <label> --runs 3`, wait for 1-min load < 2.
- Entities: `anvl_carrack`, `drak_clipper`. Baselines and current medians in
  `docs/optimisation-ledger.md` (2026-08-03 entry).
- Full verified findings (evidence, corrections, traps) are in the review
  workflow output; the traps that matter are restated below.

## 1. U1 — pooled scratch surfaces in the UI compositor (biggest UI item)

Full-canvas scratch (1.5–8.3 MB) allocated per text draw / clip node:
`text/swf_draw.rs:187`, `ir_compose/engine_02.rs:163` and `:188`,
`swf_render/edit_text.rs:238`, plus siblings `swf_render/mod.rs:46,:72,:91`.
Est. −1.5–4s of Carrack's 14.5s prerender_ui. VERIFIER TRAPS (all mandatory):
- Do NOT ship a blunt `fill(Color::TRANSPARENT)` clear — on the arena path it
  is a wash and on the mmap path a regression (converts lazy zero-pages into a
  full-buffer write). Clear only the PREVIOUS call's dirty region, tracked
  with the pooled buffer. Clearing the CURRENT call's glyph-bounds union is
  NOT byte-identical (`fill_path` is unclipped; stale glyphs from a prior node
  can land inside the current readback `rect`).
- Oversized scratch is safe only where readback uses `pixmap.pixel()` /
  `get_pixel()`. `composite_clip_region_pixmap` (`ir_compose/clip_composite.rs:36`)
  indexes `src.data()` with the DESTINATION width as stride — size that
  scratch exactly or fix the stride first.
- Pool must be a thread-local CHECKOUT STACK, not a single slot:
  `with_node_clip`'s closure can nest into `draw_edit_text` → `draw_swf_font`.
- Keep the full-canvas coordinate space and Transform untouched (the
  bounds-sized `fill_primitives.rs:33` pattern changes f32 rounding →
  re-freeze). Oracle after.
- Measure first (cheap): `perf stat -e page-faults,minor-faults` on one
  binding render + an eprintln counter per alloc site — if faults are low the
  win is a plain memset saving, cut the estimate ~3×.

## 2. U3 — export-scoped canvas fetcher (subsumes U8)

`DatacoreCanvasFetcher::new` per binding (`ui_pipeline.rs:112`) throws away the
`by_guid` memo per render; `fetch_canvas_json`/`fetch_canvas_by_name` deep-clone
the whole record JSON per fetch (`canvas_fetcher.rs:131`). Hoist name index +
`RwLock<HashMap<CigGuid, Arc<Value>>>` to export scope; widen the shared path
into graph1/graph2 resolvers. Trap: `CanvasNameIndex` first-wins construction
order is load-bearing — preserve it exactly. U8 (index once per export) comes
free here; do not schedule it separately (measured ~20× smaller than filed).
Est. −0.5–1.5s.

## 3. U4 — SVG byte + raster caches (U9 refined)

Ladder: (a) memoise raw P4K asset bytes in `P4kAssetFetcher`
(`RwLock<HashMap<String, Option<Arc<Vec<u8>>>>>`, export-scoped) — trivially
byte-identical, fixes `fetch_raw`/`resolve`/`asset_exists` at once; (b) hoist
`AtlasLibrary` to export scope — TRAP: its cache key omits `manufacturer_id`
but `fetch_with_mfd_fallback` rewrites `/mfd/` paths with it
(`bb_atlas/mod.rs:130-166`); add the manufacturer to the key or you silently
cross-contaminate MFD assets; (c) memoise rasterised SVG keyed by
(path, w, h, fill bits); drop the double parse in `rasterize_svg_nine_slice`
(`bb_svg.rs:137` + `:146`, third at `:153`). Est. −1–3s.

## 4. U5 — `Arc<serde_json::Value>` for `BbNode.raw` (U10)

`ui_ir/engine_01.rs:639` clones the whole scene per render. `Arc` +
`Arc::make_mut` at the two mutation sites (padding patch :655-681,
`_EffectiveFontPx_` annotations). ~333 read sites compile unchanged via deref.
Do NOT delete the clone instead — the function reads pristine `scene` while
writing `layout_scene`. Est. −0.3–0.8s.

## 5. Remaining small UI items

- F9 glyph→path memo (`swf_draw.rs:77-80`); key on (font CharacterId, glyph
  index) — merged libraries collide on glyph index alone.
- F8 `swf_wrap_lines` running-accumulator (`swf_draw.rs:298-334`) — bit-exact
  because the fold replays the identical addition order; reset folds the bare
  word from 0.0.
- F11 thread the caller's `TextRenderer` into `draw_edit_text`
  (`edit_text.rs:239` re-parses two embedded TTFs per EditText); precompute
  `find_font_by_name`'s sorted order (preserve CharacterId-ascending — it is a
  documented determinism guard, `library.rs:135-148`).
- F2 remainder: export-scoped `Arc<SwfAssetLibrary>` memo keyed on the FULL
  resolved path list (never merge into a shared Arc), and fold ImportAssets
  collection into the existing parse (`swf_selection/loader.rs:60,:105` →
  `:177-183` re-decompresses every buffer today).
- F12 delete the dead legacy compositor: `compose/{draw_node,draw_primitives,
  raw_assets,text_draw}.rs`, dead half of `blit.rs`, `render_bb_scene*`,
  `render_canvas*` re-exports + tests. Keep `ComposeContext`/`ComposeTarget`/
  `encode_png`. Removes the duplicated hard-coded font-size table
  (`text_draw.rs:98-111`, banned category).
- EP-F5 prewarm enumeration two-phase (parse distinct .mtl paths in a rayon
  pass, then the cheap walk) — decomposed.rs:5069-5107.

## 6. R8 — measure the Blender import wall (do BEFORE A1/R6 ranking)

Recipe verified by probe (timers never fire under `blender -b`). Script sketch
in the review workflow output (addon-import surface, finding A2) with these
corrections: real paths are `~/projects/scorg_tools/ships/Packages/DRAK
Clipper_LOD0_TEX0/scene.blend` and `.../ANVL Carrack_LOD0_TEX0/scene.blend`
(spaces, ships/ is a SIBLING of StarBreaker/); use `--factory-startup` on ALL
phases; read VmRSS per phase (VmHWM is monotone); iterate roots and pick the
first with a non-empty dirty list; `only_unloaded=True` with `target_objects`
is a no-op filter; record results as a LOWER BOUND on GUI wall. 3 runs,
medians, load < 2, ledger rows next to the export medians.

## 7. A1 — POM library append per height image (gate on R8)

`groups.py:2205-2218`: ~≤145 `bpy.data.libraries.load(pom_library.blend)`
round-trips per Carrack cook (one per unique height image; the `[<ext>]` key
half is dead — extension is always REPEAT). Fix: append the pristine chain
once per session, then `tree.copy()` the 5 per-image groups + rewire; shared
helpers (tangent_space, Clamp) point at canonicals; keep the existing DFS +
rename. Traps: verify nested `.copy()` semantics (shallow shares sub-trees —
wrong for POM_disp's sampler); every library append also drags in an orphan
`argo_pom_displ.tif` image that nothing purges (the copy path removes this);
keep the pristine chain out of `_purge_orphaned_runtime_groups`' reach or add
bare-name cleanup; validate at DATA level (Bias/Scale/Layers/sampler image +
extension equality, chain count) — never by eyeballing renders.

## 8. A3 — R6 MaterialRefreshSession wiring (UX, owner-accepted)

Five-piece plan in the review output (addon surface, finding A3). The critical
un-flagged trap: the session has NO dirty-object filter (`package_ops.py:396-403`
takes ALL sidecar meshes) — thread `target_objects` through `__init__` FIRST or
the swap rebuilds 6295 Carrack objects instead of the dirty subset. Add
`abort()` closing `_context_stack` (idempotent), called from a NEW load_pre
handler (restoring `_temporary_object_mode` after a file swap touches a freed
datablock — never abort from load_post/timer); re-entrancy via the existing
token + a single module-level session slot (`_AUTO_MATERIAL_REFRESH_SESSION` —
the test namespace already stubs this name); step budget ~0.05s; keep the
manual refresh operator blocking. Tests to rewrite: `test_auto_refresh_timer.py`
poisoned-session test inverts. Verify in a REAL Blender session (UI responsive,
identical material name-set/count, shading restored on completion AND on
mid-cook file open). Note B3a bias + orphan flush now run in `_finish_objects`
— keep them on the session path.

## 9. A6 — sidebar draw memos

`ui.py` draw walks `children_recursive` (O(scene)) + re-parses glow JSON 4-8×
per redraw; memoise with the existing `(as_pointer(), len(bpy.data.objects))`
pattern from `ui.py:1188-1201`. ~30 lines + one stub test.

## 10. R4 — Luma8 roughness (owner-accepted re-freeze; separate commit)

Change ONLY `pack_perceptual_roughness_as_grayscale_roughness_png`
(`textures.rs:1447-1466`) to `image::GrayImage` + a Luma8 encode helper
(precedent: `cli/src/dds.rs:288`; `from_raw` needs an owned Vec). Verified
consumer surface: sole Rust consumer `decomposed.rs:5302`; sidecar semantics
stay true (r/g/b=roughness, a=1.0) so JSON is byte-identical; addon
`_metallic_roughness_green_socket` (SeparateColor→Green) is Luma8-safe
(Blender yields v,v,v); `validation.rs:1534-1604`/`:1048-1073` pass via
`.to_rgba8()` BUT the `non_grayscale`/`non_opaque_alpha` guards go vacuous —
add an IHDR colortype assertion to keep the invariant guarded. Gains: ~900 MiB
→ ~230-270 MiB disk+cache per Carrack (measured 3.4-4.1× on samples), −0.5-1s.
Verify: decoded-PIXEL equality vs `carrack_aug02` roughness PNGs (gray == old
R), `diff -rq` clean elsewhere, re-bench, update
`docs/decomposed-export-contract.md` in the same change. Then owner re-freeze
of the oracle baselines.

## 11. B1 — batch multi-entity export (scale prep)

`cli/src/entity.rs` Export takes one `name: String`. A variadic form must move
`output` off positional (breaks `export_ship.sh`/`perf_bench.sh`/
`perf_oracle.sh`/docs — update together) OR add an `export-batch` subcommand.
Hoist `(p4k, db)` over the loop. Verify: export ship B solo vs batched `A B` →
oracle on B. ~2.7s/ship saved at scale; zero single-ship gain. Check
process-lifetime caches are ship-independent (registry.rs:14, colour.rs:41 —
they are).

## 12. Owner-decision queue (from the report)

--mip 1 iteration default; R3 resvg 0.48.1 defer-or-do; B2 palette-fallback
FIX (re-freeze; evidence: 207/216 palette-less Clipper interior placements);
B3b exporter `starbreaker_bitangent_sign` attribute (re-freeze; verify the
linked-mesh premise in one live session first — also check
`_ensure_active_uv_layer` writes on linked meshes, `package_ops.py:588-604`);
EP-F7 prerender/prewarm overlap; PGO spike; EP-F10 prewarm byte budget;
ships/Data case-dup.
