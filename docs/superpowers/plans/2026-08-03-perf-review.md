# Performance review — ship extraction end-to-end (2026-08-03)

Four-surface multi-agent review (UI prerender, export hot path, Blender addon
import, radical replatforming), grounded on the 2026-08-02 pass and its
handoff. Review was read-only; top findings per surface adversarially verified
against the files. Baseline (perf-profile binary, 3-run medians, quiet):
Carrack 33.41s / 9.12G, Clipper 26.36s / 5.54G. Phase attribution: Carrack
prerender_ui 14.53s (43% of wall), prewarm_textures 3.57s, ~4.7s startup;
Clipper interior_asset_resolve 5.4–5.7s (vs 2.3–3.0s on the larger Carrack —
explained by EP1 below). CPU utilisation ~49% of 16 cores.

## TL;DR

| # | Finding | Est. gain | Class |
|---|---|---|---|
| EP1 | Layer-submaterial textures bypass the parallel prewarm → decoded serially in the writer (THE Clipper anomaly) | −2–4s Clipper, −1–1.5s Carrack | non-radical |
| EP2 | 2026-08-02 parallel final write + all mem instrumentation landed on a dead branch (`--kind decomposed` never reaches it) | −0.2–0.5s + restores [mem] | bug |
| EP3 | P4K index build fully serial; 2 of 4 indexes unused by export (1.35M entries) | −1–1.5s every export | non-radical |
| U1 | U7 + 3 unreported siblings: full-canvas scratch (up to 8.3 MB) allocated per node/text draw | −1.5–4s of prerender_ui | non-radical (traps documented) |
| U2 | SWF library (incl. fonts_en.gfx glyph sheet) fetched+parsed 2× per render, 82× per Carrack export; ImportAssets pass double-parses every buffer | ~−2s CPU / sub-second wall | non-radical |
| U3 | Canvas fetcher memo per-binding + deep-clone per fetch (subsumes U8, which measured ~20× smaller than filed) | −0.5–1.5s | non-radical |
| U4 | U9 refined: SVG bytes re-read from P4K + re-parsed by usvg per node per render; AtlasLibrary per binding | −1–3s | non-radical (manufacturer-key trap) |
| U5 | U10: ir_compile deep-clones whole scene JSON → Arc<Value> raw | −0.3–0.8s | non-radical |
| EP4 | prewarm enumeration serial (285 .mtl parses) + MtlFile deep-clone per cache hit | −0.8–2.7s combined | non-radical |
| AL1 | Delete TrackingAllocator (3 atomics per alloc on 16 threads, feeding a debug logger + off-by-default cap); statm-based mem logging | deletion; unmeasured | non-radical |
| B1 | Batch multi-entity export (`names: Vec<String>`) — daemon + on-disk cache REJECTED | −2.7s/ship at scale | non-radical |
| A1 | Addon: POM chain re-appended from pom_library.blend per unique height image (~145 loads/Carrack cook) → session-append + tree.copy() | est −3–10s cook (gate on R8) | non-radical |
| A2 | R8 measurement recipe delivered (headless, 3-phase split, traps verified by probe) | baseline | housekeeping |
| A3 | R6 plan incl. un-flagged trap: MaterialRefreshSession has NO dirty-object filter — naive swap rebuilds 6295 objects | UX | non-radical |
| A4 | B3a: `_apply_deferred_pom_background_bias` never runs on file-open → Bias stuck at 0.5 fallback | correctness | bug |
| A5 | B3b: bitangent-sign bake IMPOSSIBLE on file-open (mesh data library-linked); root fix is exporter-side (dequant.rs already decodes the sign) | correctness + −0.5–2s import | RADICAL (re-freeze) |
| A6/A7 | Sidebar draw O(scene) walk + JSON re-parse per redraw; orphan-material queue never flushed on blocking path | interactivity / leak | non-radical / bug |
| U6 | Misc UI: P4kSwfFetcher linear scan → binary search; glyph→path memo; wrap-lines O(n²); dead SHA-256; TextRenderer per EditText; dead legacy compositor (~1200 lines + banned hard-coded table) | −0.3–1s + hygiene | non-radical / housekeeping |

## Radical axis — verdicts (web-verified 2026-08-03)

| Axis | Verdict | Note |
|---|---|---|
| Allocator | **SWITCH(a) / DROP(b)** | (a) TrackingAllocator deletion = AL1. (b) mimalloc DROPPED — ledger dead-end #1: allocator swap measured −12% (98.8 vs 88.3s); pipeline is memory-bandwidth-, not allocator-bound. |
| PGO / BOLT | **SPIKE PGO** (0.5 pd) | cargo-pgo 0.3.0 (2026-01-25) current; honest expectation 4–8% on the perf binary; codegen-only ⇒ oracle expected clean (fat-LTO+target-cpu precedent). BOLT REJECT (experimental). Script-only, never default build. |
| Process model | **SWITCH to batch** | B1. Daemon + on-disk P4K/DCB cache REJECTED in writing (IPC/lifecycle/staleness for the same amortisation a 20-line loop gets). |
| --mip default | **OWNER DECISION** | `--mip 1` for iteration exports: ~−3–4s wall (sub-proportional — from_split eagerly reads sibling mips), package 3.2G→~1.1G, big RSS cut, UI SHA-freeze untouched (ui_pipeline.rs:177 discards mip). Needs baseline re-cut + perf_oracle.sh:82 fix. Keep mip 0 for parity/gold. |
| Blender side | **KEEP addon, R8 first** | R7 cooked-material cache already shipped (identity short-circuit materials.py:277-283). Compiled-extension REJECT (cost is RNA layer, not Python). Blender 5.2 LTS already installed. |
| UI raster | **KEEP tiny-skia 0.12** | Alive (0.12.0 2026-02-02, linebender), changelog additive. R3 RETARGET: resvg is at 0.48.1 (2026-08-02); our text never goes through resvg (tiny-skia paths + rusttype fallback) ⇒ zero perf payoff; upgrade = hygiene only (collapses dup tiny-skia 0.11/0.12) at the cost of a guaranteed full UI re-freeze. Recommend re-deciding the old R3 acceptance. |
| PNG→WebP, io_uring, zstd level | **REJECT** | Encode ~1.2s and lossless WebP slower; handle pool + page cache leave no I/O wait; zstd level owner-rejected. |

## Owner decisions needed

1. **--mip 1 as the iteration default** (biggest single lever; re-cut baselines).
2. **R3 resvg**: defer/close (recommended) vs do at 0.48.1 as a hygiene re-freeze.
3. **B2 palette fallback**: probe result = FIX not delete — the dead fallback is
   suppressing real palette assignments (207/216 palette-less Clipper interior
   placements have a sidecar DefaultPalette). Output-changing ⇒ re-freeze.
4. **B3b exporter bitangent-sign attribute** (changes .blend bytes; premise
   needs one live-Blender verification first).
5. **EP overlap** (rayon::join prerender_ui with prewarm block, −2–4s, medium
   plumbing risk) — implement or defer.
6. **PGO spike** — run it or not.
7. ships/Data case-dup (~1.5G) — still blocked on sign-off.

## Sequencing (this session, non-radical pre-approved)

1. Rust export batch: EP1, EP2, EP3, EP4, AL1, B1 → oracle + determinism + bench.
2. UI batch: U2, U3, U4(a), U5, U6 quick items, then U1 (scratch pool, exact
   previous-dirty-rect clearing, checkout stack; NOT blunt fill; fix
   composite_clip_region_pixmap stride before oversizing) → oracle + bench.
3. Addon batch: A4 (B3a), A7, A6, then A2 (R8 measurement, load<2), then A1
   (gated on R8 numbers), then A3 (R6 wiring incl. dirty filter + abort()).
4. R4 Luma8 (owner pre-accepted re-freeze): separate commit + decoded-pixel
   comparison + freeze cycle. Addon audit clean (Luma8-safe: SeparateColor
   Green == luma for r=g=b; validation guards become vacuous — add IHDR
   colortype assertion).
5. Re-bench both ships (quiet gate, 3 runs) → new medians + ledger entry.

Full verified findings with file:line evidence, corrections and traps:
workflow wf_30584ede-dc1 transcript (session scratchpad `findings_summary.txt`
holds the extract; key traps are restated in the ledger entry on landing).
