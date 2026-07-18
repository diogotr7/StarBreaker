# `starbreaker-ui` — Performance Baseline (B0)

**Date:** 2026-06-08  
**Machine:** Linux (same machine as dev)  
**Build profile:** `release` (Rust optimised)  
**P4K build:** LIVE Data.p4k  
**Benchmark ship:** `DRAK_Clipper`  
**Run mode:** `RAYON_NUM_THREADS=1` (serial, for per-image isolation)  
**Command:**
```bash
SB_UI_TIMING=1 SC_DATA_P4K="..." RAYON_NUM_THREADS=1 RUST_LOG=info \
  starbreaker entity export "DRAK_Clipper" "ships" \
  --kind decomposed --lod 0 --mip 0 --materials all
```

> NOTE 2026-06-13: this is the dated **B0 baseline** — a serial
> (`RAYON_NUM_THREADS=1`) per-stage isolation run from 2026-06-08, captured
> *before* the optimisation plan below was actioned. It is kept as the
> before-snapshot, not a statement of current performance: full-export wall
> time is now ~48–50s (`crates/starbreaker-ui/docs/ui-reference.md` §2), so the serial 326 s /
> "5–7 min per image" figures no longer hold. Re-profile (plan step 5) before
> citing these stage percentages as current.

---

## X — Aggregate stage breakdown (all 43 bindings, serial)

| Stage | Total (s) | % of wall |
|---|---|---|
| `ir_compile` (`compile_ui_ir_from_scene_with_animation_sample`) | 303.9 | **93.2%** |
| `graph2` (`resolve_canvas_graph_with_loc_and_bound_view`) | 3.5 | 1.1% |
| `render` (rasterise + composite) | 3.4 | 1.0% |
| `swf_load` (`load_first_swf`) | 2.7 | 0.8% |
| `encode` (PNG encode) | 0.1 | 0.0% |
| `manifest` (`build_asset_reference_manifest`) | 0.1 | 0.0% |
| `graph1` (`CanvasWidgetTreeResolver::resolve`) | 0.0 | 0.0% |
| `fetch` / `style_load` | 0.0 | 0.0% |
| **Total serial** | **326.1** | 100% |

> Note: `compile` (314.4s) wraps `ir_compile` + setup; `ir_compile` is the stage timer
> for `compile_ui_ir_from_scene_with_animation_sample` alone. Percentages relative to
> total wall time (326.1s), not to `compile`.

---

## X — Per-binding totals (named bindings only)

| Binding | Kind | Total (s) |
|---|---|---|
| Screen_Left_Lower_RTT | mfd | ~78 (two renders: 77.5s + 79.1s) |
| Screen_Radar_RTT | radar | 67.1 |
| Screen_Left_Upper_RTT | mfd | 26.4 |
| Screen_Right_Upper_RTT | mfd | 19.5 |
| `?` (door/medical) | physical | 10.0 |
| `$slot_standing_screen` | physical | 5.3 |
| Screen_Annunciator_L | physical | 4.2 |
| Screen_Annunciator_R | physical | 4.2 |
| Screen_Left_Upper_RTT_Small | physical | 2.7 |
| Screen_Small_Radar2 | physical | 2.3 |
| cabinet_attach_loc (×2) | physical | ~1.8 |
| Screen_Small_Radar1 | physical | 1.5 |
| Countermeasures_Screen | physical | 1.4 |
| screen_flight_hud_right_upper | physical | 1.3 |
| screen_flight_hud_right | physical | 1.3 |
| Screen_Central_Compass | physical | 1.2 |
| `?` (radar small) | radar | 1.2 |
| screen_flight_hud_left_upper | physical | 1.2 |
| screen_flight_hud_left | physical | 0.8 |
| `?` (multiple fast ≤0.6s each) | physical | 0.4–0.6 |
| `?` slow single | physical | 3.7 |

**Max single-image time: ~79s.** Median: ~0.8s.  
**Total serial time for 43 bindings: 326.1s.**

---

## Key finding

`compile_ui_ir_from_scene_with_animation_sample` accounts for 93% of all render time.  
Within that function, the dominant cost is per-node `resolve_record`/`fetch_canvas_by_name`
calls — O(records) DataCore scans per style-tag, per node (driver #1 from §3 of the review).

`graph2` (the second full graph-resolution pass) is 1.1% — cheap but still wasteful.  
SWF loading (`load_first_swf`) is 0.8% — the 20×+ SWF re-parse problem (driver #3) is real but
secondary to the DataCore scan problem.

---

## Optimisation plan (ordered by measured impact)

1. **B2a** — memoising canvas fetcher: cache `guid/name → Value` per binding. Should cut `ir_compile` dramatically (every repeated scan becomes a hash lookup).
2. **B2b** — name/path O(1) index: build once, turn O(records) scans to O(1). Completes what B2a starts.
3. **B2c** — stop decompressing textures for diagnostics (currently <0.1% — low priority but cheap).
4. **B2d** — load localization once (currently <0.1% — low priority but cheap).
5. **Re-profile** after B2a+B2b to see updated split before tackling SWF/graph work.
---

## Post-zlib-rs re-profile (item 6 step 0) — 2026-07-18

**Date:** 2026-07-18
**Machine:** Linux (dev machine)
**Build profile:** release
**P4K build:** LIVE Data.p4k (auto-detected)
**Benchmark ship:** drak_clipper
**Run mode:** serial (`RAYON_NUM_THREADS=1`) for per-image isolation; plus one parallel run for production wall
**Command:**
```bash
SB_UI_TIMING=1 RAYON_NUM_THREADS=1 RUST_LOG=info \
  StarBreaker/target/release/starbreaker entity export drak_clipper <root> \
  --kind decomposed --lod 0 --mip 0 --materials all
```

Binding count fell 43 (B0, 2026-06-08) → 27 because the `UiRenderKey`
render-dedup (commit `476a9503e`, 2026-06-21, landed after B0) renders one
representative per distinct render key, collapsing duplicate screens to a
single render each.

### Aggregate stage breakdown (all 27 bindings, serial)

| Stage | Total (s) | % of wall |
|---|---|---|
| `render` | 14.151 | 52.5% |
| `ir_compile` | 4.976 | 18.5% |
| `graph2` | 1.763 | 6.5% |
| `swf_load` | 1.654 | 6.1% |
| `swf_load_measure` | 1.648 | 6.1% |
| `encode` | 0.249 | 0.9% |
| `manifest` | 0.001 | 0.0% |
| **Total serial wall** | **26.957** | 100% |

> Note: `compile` (10.805s) is a wrapper that itself contains `ir_compile` +
> `swf_load_measure`. `swf_load` (`pipeline/mod.rs:595`) and `swf_load_measure`
> (`:750`) are two SEPARATE real SWF parses of the same asset with no cross-call
> cache, so their cost SUMS (3.302s) — they are both paid, not one load counted
> twice (correcting the earlier reading). The wall (26.957s = sum of per-binding
> `total=`) decomposes non-overlappingly as `render` (14.151) + `compile`
> (10.805) + `graph2` (1.763) + `encode` (0.249) ≈ 26.97s. Post-zlib-rs the
> ir_compile collapse (303.9s → 4.98s) from the B2a/B2b memoisation has fully
> landed; `render` is now the dominant stage.

Parallel run: `interior_ui_bindings` = 0.00s (the decomposed-phase UI timer reads
0.00s in the parallel path — the per-binding work is timed inside the parallel
render, not this counter); full export wall = 40.98s (decomposed phase 27.06s).

### Per-binding (canonical render paths)

| Binding | Kind | total | swf_load | render | ir_compile |
|---|---|---|---|---|---|
| Screen_Left_Upper_RTT | mfd | 1.762 | 0.056 | 1.169 | 0.344 |
| Screen_Central_Compass | physical (HUD) | 0.587 | 0.056 | 0.195 | 0.091 |
| i_med_medicalbed_a | medical | N/A | N/A | N/A | N/A |

> Medical: the Clipper export has no medical binding (`i_med_*` / `kind=medical`
> absent); only `mfd`, `physical`, `radar` kinds are emitted. Compass is
> `kind=physical` (the closest HUD path). Per-binding `swf_load` is uniform
> (~0.056s) because SWF decompress+parse is a fixed per-binding cost.

### Item-7 GATE VERDICT
Rank the aggregate stages by serial time. **GATED-IN** if `swf_load`
(+ any stage-parse cost surfaced inside `render`/`swf_load_measure`) is in
the top 3 stages by total time; otherwise **GATED-OUT**.
Verdict: **GATED-IN** (reviewer-overturned, orchestrator-adjudicated).
`swf_load` (1.654s) and `swf_load_measure` (1.648s) are two SEPARATE real SWF
parses of the same asset (`pipeline/mod.rs:595` and `:750`, no cross-call
cache), so their cost SUMS to 3.302s — rank #3 by stage cost, above `graph2`
(1.763s). The prior GATED-OUT verdict wrongly treated the two timers as one load
counted twice; they are two loads, each paid. SWF parse-once (item 7, Task 3) is
therefore a top-3 lever and is IN SCOPE for this run.
