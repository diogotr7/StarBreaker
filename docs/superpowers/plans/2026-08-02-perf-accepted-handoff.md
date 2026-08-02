# Handoff — accepted follow-ups from the 2026-08-02 perf review

Owner-accepted items NOT landed in the review session (each needs a Blender
session and/or a re-freeze cycle). Written for a fresh agent with zero context.
Parent report: `2026-08-02-perf-review.md`; landed work: optimisation-ledger
"Pass 2026-08-02" entry and commits `23c577334..64fe3fcd9` on `feature/ui`.

## Shared preamble (repo conventions — binding)

- Read `AGENTS.md` first: TDD (failing test before fix), no hard-coded
  game-data values, no per-asset name gates, `//!` headers on new Rust files,
  commit on `feature/ui` directly, NO commit trailers, addon guidance in
  `blender_addon/AGENTS.md`.
- Byte-identity discipline: perf changes verify with
  `scripts/perf_oracle.sh <baseline> <candidate>` (current oracle baselines:
  `ships_perf_bench/{carrack,clipper}_aug02`) + a two-run determinism diff.
  Anything that changes bytes needs an owner-approved re-freeze
  (`crates/starbreaker-ui/docs/ui-workflow.md`; UI freeze SHAs in
  `crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json`).
- Benchmarks: `scripts/perf_bench.sh <entity> <label> --runs 3`, machine load
  < 2 first. Current medians (3-run, quiet): Carrack 37.01s / 9.19G,
  Clipper 29.40s / 5.51G.
- The optimisation skill: `.claude/skills/starbreaker-optimisation/SKILL.md`.

Recommended order: 1 (R6) → 2 (B3, same Blender-session tooling) → 3 (R4) →
4 (R3). Items 3 and 4 both re-freeze — do them as separate commits with
separate freeze cycles so regressions are attributable.

## 1. R6 — wire up `MaterialRefreshSession` (file-open no longer freezes)

**Problem.** Opening a ship `scene.blend` triggers material cooking through
`ui.py:2045 _starbreaker_load_post` → one-shot timer `ui.py:1980` →
`refresh_materials_for_package_root` (`runtime/package_ops.py:297`) — one
synchronous unbudgeted loop that freezes Blender until the whole ship is done
(progress box repainted every 5 s, only the first dirty root refreshed).
Meanwhile `MaterialRefreshSession` (`package_ops.py:370`) — chunked, budgeted
(`step(budget_seconds)`), correctly cached, tested — has ZERO production
callers (`tests/test_auto_refresh_timer.py:119` injects a poisoned session to
assert the timer does NOT use it — that test must be updated deliberately).

**Change.** Replace the blocking call at `ui.py:~2016` with a repeating
`bpy.app.timers` callback driving `session.step(budget)`; return the
re-arm interval while work remains, `None` when done. Fix the known landmine
first: `MaterialRefreshSession._context_stack` (`package_ops.py:419-421`) is
opened in `__init__` and closed only in `_finish_objects` — an abandoned
session leaves viewports stuck on SOLID; scope `_temporary_object_mode` /
the context stack per-step or add an explicit `abort()` called on file
change/addon unload. Note the session-class palette-memo call at
`package_ops.py:466` is the CORRECT pattern (the shipping-path equivalent was
fixed 2026-08-02).

**Verify.** Addon suite green (`cd blender_addon && python3 -m unittest
discover -s tests -q`; 377+ tests). Then a REAL Blender open of
`ships/Packages/DRAK_Clipper_LOD0_TEX0/scene.blend` (blender MCP available):
UI stays responsive during cooking, final materials identical to the blocking
path (compare a node-graph dump or material count/name set before/after),
viewport shading restored on completion AND on interruption (open a second
file mid-cook).

**Risks.** Re-entrancy (second load-post while a session runs) — guard with a
single active-session slot keyed by package root; timer leaking after file
close — deregister in the load-pre handler.

## 2. B3 — POM post-passes never run on the file-open path

**Problem.** `_bake_pom_mesh_bitangent_signs` and
`_apply_deferred_pom_background_bias` run only from `import_scene`
(`runtime/importer/orchestration.py:576,579`). The load-post refresh path
rebuilds POM materials WITHOUT either. The Rust exporter writes no
`starbreaker_bitangent_sign` attribute (verified: no such string in
`crates/starbreaker-3d/src/`), so a natively-assembled scene.blend can carry
POM materials on meshes lacking the correction — mirrored-UV faces march the
wrong way; Bias stays at the 0.5 fallback.

**Change (after verifying against a real open).** First REPRODUCE in a real
Blender session: open the Clipper scene.blend fresh (no prior import), find a
POM material (`docs/blender-pom-parallax-occlusion.md` is the runbook), check
whether the bitangent-sign attribute exists and whether Bias is the fallback.
If confirmed: run both passes from the refresh path too (they are idempotent —
the bake skips meshes that already carry the attribute; keep it that way), or
have the session (item 1) enqueue them per rebuilt POM material.

**Verify.** Data-level checks, not eyeballing (owner feedback memory): mesh
attribute presence + Bias node values before/after; `PomBiasSliceTests` and
the POM tests stay green. If item 1 landed first, verify on the session path.

**Risks.** Running the bake on every refresh is O(package) — gate it to
POM-material-bearing meshes only (the existing walk already filters).

## 3. R4 — channel-pack single-channel maps (Luma8) — RE-FREEZE

**Problem.** `pack_perceptual_roughness_as_grayscale_roughness_png`
(`crates/starbreaker-3d/src/pipeline/textures.rs:~1395`) expands 1-channel
roughness to RGBA8 (`R=G=B=v, A=255`); `encode_png`/`encode_png_rgba` only
accept `RgbaImage`. Carrack: 284 roughness PNGs = 944 MB; encode time is
proportional to raw input bytes (4× the data).

**Change.** Emit `Luma8` PNGs for the grayscale roughness path (and audit
other single-channel maps). Add a `Luma8` encode helper beside `encode_png`.
The sidecar already records `packed_texture_format: "roughness_grayscale"` and
`packed_channel_semantics` — update those fields' documented meaning in
`docs/decomposed-export-contract.md` in the same change.

**Blocker to clear FIRST:** verify the addon texture path handles 1-channel
PNGs: `blender_addon/.../importer/materials.py` node wiring (does anything
index channel 1/2/3 of the roughness image?), and the POM/height path if it
consumes roughness. Blender loads Luma8 PNGs fine but node sockets read
R=G=B automatically only when the image is single-channel — confirm the
template graphs (`docs/blender-material-template-authoring.md`).

**Verify.** Not byte-identical by design: run a decoded-PIXEL comparison
(grayscale values equal to the old R channel) over all roughness PNGs vs
`carrack_aug02`, `diff -rq` clean for everything else; re-freeze any UI
artifacts only if touched (they should NOT be — UI PNGs don't use this path);
re-bench (expect encode-time and ~600+ MB size reduction on the Carrack);
Blender import visual check via node-value dumps.

**Risks.** Addon or template assuming 4 channels — the audit above is the
guard; sidecar consumers parsing `packed_channel_semantics` — grep the addon
for it before changing the value.

## 4. R3 — resvg/usvg 0.42 → 0.47 — RE-FREEZE

**Why.** 0.46 added glyph-outline caching (SVG text raster is on the UI hot
path); 0.45 relicensed MPL-2.0 → Apache-2.0/MIT (cleaner for the public
release). tiny-skia 0.12 requirement already met.

**Change.** Bump `resvg`/`usvg` in `crates/starbreaker-ui/Cargo.toml`
(check for API breaks across 0.43-0.47 — the usvg tree API moved around).
Text rendering changes pixels → this is a re-freeze event: own commit,
`scripts/ui_freeze_cycle.sh` per `ui-workflow.md`, full measure sweep
(`scripts/ui_check.sh --full`) with before/after diffs reviewed toward-
reference (the known-outlier override pattern `9980b779d` if any element is
knowingly off). Re-bench the UI stage after.

**Risks.** Silent glyph-metric drift beyond the freeze targets — the
font-size harness (`crates/starbreaker-ui/docs/ui-font-size-harness.md`,
`SB_UI_FONT_DUMP` + `scripts/font_size_check.py`) must be run; any regression
vs the platinum/gold baselines blocks the bump.

## Also open (not owner-blocked, next perf pass)

- U7–U10 (clip-rect text scratch, per-export CanvasNameIndex, AtlasLibrary
  SVG cache, Arc<Value> scene clone): add the sub-render `AtomicU64` probes
  first (review §finding 13), re-rank, then implement the winners.
- B2 probe: `material_sidecar_default_palettes` is always empty
  (`blend_assembly.rs:527` filters `manifest_files`, which only ever holds
  `PackageManifest` kinds) — decide fix-or-delete for the unreachable
  `sidecar_palette_id` fallback.
- `ships/Data` objects/Objects case-dup cleanup (~1.5 G) — still blocked on
  owner sign-off (ledger item-2 record has the audit).
