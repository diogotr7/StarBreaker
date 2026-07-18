# Performance handoff implementation plan (2026-07-18) — MERGED

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox syntax. The binding design is `2026-07-18-perf-handoff-design.md` (same directory); this plan implements it — implementers do not redesign. On plan-vs-reality conflict: STOP and return the conflict to the orchestrator.

**Goal:** Land performance-handoff items 1–10+12 (item 11 deferred, 13 recorded) with byte/pixel/case-modulo oracles enforced per item.

**Structure:** Part A = Rust exporter (tasks A1–A7: items 1, 8-precheck, 10-probe, 8, 12.1+12.2, 2, 10-swap). Part B = UI crate (tasks B1–B3: item 6 profile, item 6 fast path, item 7 conditional). Part C = addon/ops/spikes (tasks C1–C7: items 3, 4, 5, 12.3-ledger, 13-note, 9-spike, 12.4).

**Global execution order (wall-clock aware; benchmark window closes ~07:00):**
A1 (zlib-rs + timing) → B1 (profile, gate verdict for B3) → A2 (item-8 pre-check) → A3 (item-10 probe) — these four serial, deadline-critical. Then B2 → [B3 if gated] → A4 → A5 → A6 (item 2, LAST byte-affecting exporter change; fresh baselines after) → A7 (if >5%) → C6 (item-9 spike, measure + revert). C1–C5 (addon/ops, no cargo) interleave any time no cargo build runs; C7 (mcp) after all other cargo work. Each phase ends with its review task.

## BINDING AMENDMENTS (orchestrator arbitrations of planner-flagged conflicts — these override the design/handoff wording where they conflict)

1. **(A/F1)** `write_decomposed_export` has exactly one caller (`blend_assembly.rs:1028`); no glb empty-cache plumbing exists or is needed.
2. **(A/F2)** No in-memory `MappedP4k` constructor exists; the item-2 TDD determinism test is integration-gated on real P4K data. The implementer MUST run it explicitly (per the task's env-gated command) and show fail-before/pass-after output; bare `cargo test` silence does not satisfy TDD for this task.
3. **(A/F3)** Item 10 swap scope = **P4K decode only** (`starbreaker-p4k::zstd_decompress`). chf stays wholly on ruzstd (encode is a save-file CLI path outside all oracles; decode-only swap there is negative consolidation). Ledger records the partial-consolidation tradeoff for the owner.
4. **(A/F4)** A4 (item 8) lands before A5 (12.1+12.2) so the `write_decomposed_export` signature changes settle once.
5. **(B)** `parse_tags` returns the owned `SwfBuf` (decompress once); each construction runs `parse_swf(&buf)` once and all `*_from_tags(&parsed.tags)` in-scope. One decompress+parse per construction is the invariant; the literal `-> Vec<Tag>` helper in the design is not expressible (borrowed lifetimes) and is amended accordingly.
6. **(C)** Item-9 spike scope excludes gfx encode helpers (feed UI/holo/radar outputs — byte-frozen territory) and `cli/dds.rs` (standalone subcommand off the measured path); both noted as optional follow-ups in the report. Spike measures the two `pipeline/textures.rs` encoders only, then reverts.
7. **(C)** Oracle UI-PNG count is whatever the glob finds (26 in clipper_pre1), never a hardcoded 23.

## BINDING AMENDMENTS — round-1 adversarial review (verdict APPROVE-WITH-FIXES; fixes adopted verbatim, non-structural, so no second full review round)

8. **(review F1, MATERIAL)** The A2 (`textures.rs` `#[cfg(test)]` probe) and A3 (`archive.rs`/`lib.rs` zstd probe) instrumentation is reverted **immediately after its measurement is recorded** — before ANY later task commits. All implementation commits in every part use explicit-path `git add <files>`; `git commit -am` and `git add -A` are banned for this run.
9. **(review F2, MATERIAL)** A6 (item 2): the three existing casing unit tests — `material_sidecar_relative_path_uses_source_mtl_not_geometry_path` (decomposed.rs:7983), `material_sidecar_relative_path_encodes_mip_level` (:8002), and the `_normalizes_case` sibling (:8012) — assert lowercase literals and cannot supply the new `p4k` param. Convert them to the `SC_DATA_P4K`-gated integration form asserting P4K-authority casing, or delete where the new gated invariant test subsumes them. Do NOT treat them as case-independent.
10. **(review F3, minor)** C6 (item-9 spike): the revert step must name every touched file (textures.rs AND any decomposed.rs timing probe) — explicit `git checkout -- <both paths>` or a scoped stash; follow with `git status --short` expected empty.
11. **(review F4, minor)** A4 (item 8) ledger entry must note: layer submaterials resolved via `resolve_layer_submaterial` (decomposed.rs:2707) are not in the prewarm enumeration and still decode serially (byte-safe, partial speedup) — so the timing delta is read correctly.

---
# Plan — Part A: Rust exporter workstream (perf handoff 2026-07-18)

Workstream = binding decisions **D1, D2, D5, D6, D12** → handoff items **1, 8, 12.1, 12.2, 2, 10**.
Planner only; implementer executes. All anchors verified 2026-07-18 against live source.

## Global constraints (travel to every task)

- Commit directly on `feature/ui`. No branches/worktrees. No commit trailers. Never name the owner.
- Debug `cargo build` for iteration; `cargo build --release` only for benchmark/deploy binaries. **Builds are sequential** — never run two cargo builds against the shared `target/`.
- Byte-identical gate (items 1, 8, 12.1, 12.2, 10-swap): re-export into a fresh root, then
  `diff -rq <baseline> <new> | grep -v export_stamp` must be **empty**. Item 2 uses the **case-modulo oracle** below (bytes differ only by path casing).
- Every new `.rs` file starts with a `//!` header. No hard-coded game-data values. No `.unwrap_or`/`.max()` silencers. Root-cause fixes only.
- TDD for item 2 (failing test first, verify it fails, fix, verify it passes).
- Benchmarks: check `uptime` load < 2 first; benchmark window closes ~07:00. `RUST_LOG=info` emits `[timing]`. Do **not** set `RAYON_NUM_THREADS=1` except for the item-8 serial-vs-parallel measurement.
- Canonical export command (run from `/home/tom/projects/scorg_tools`; P4K auto-detected):
  ```
  StarBreaker/target/release/starbreaker entity export <entity> <root> --kind decomposed --lod 0 --mip 0 --materials all
  ```
  Entities: `drak_clipper`, `anvl_carrack`. Pre-change oracles already captured at
  `ships_perf_bench/clipper_pre1` and `ships_perf_bench/carrack_pre1`.
- After each landed item: append Observed/Finding/Action to `StarBreaker/docs/optimisation-ledger.md`. One commit per coherent item.
- Test commands: `cargo test --workspace`; targeted `cargo test -p starbreaker-3d --lib`.

## Task order & binding

| Task | Item | Phase | Order dependency |
|------|------|-------|------------------|
| 1 | 1 zlib-rs hoist | A | **First** (unblocks all later benchmarks) |
| 2 | 8 pre-check probe | A (window) | after Task 1 build; independent of 3 |
| 3 | 10 decode-time probe | A (window) | after Task 1 build; independent of 2 |
| 4 | 8 DDNA prewarm impl | C | **gated on Task 2 > 4×**; after Task 1 |
| 5 | 12.1 + 12.2 | C | after Task 1; independent of Task 4 |
| 6 | 2 case bug + cleanup | D | **LAST byte-identical item** — after Tasks 1, 4, 5 land |
| 7 | 10 zstd swap | E | **gated on Task 3 > 5%**; after Task 6 (verifies vs fresh baselines) |

Each phase ends with a review placeholder the orchestrator runs.

---

## TASK 1 — Item 1: flate2 → zlib-rs backend (Phase A, ORDER-BOUND FIRST)

**Files**
- `StarBreaker/Cargo.toml` (workspace root; `[workspace.dependencies]` at :35)
- `StarBreaker/crates/starbreaker-p4k/Cargo.toml:11`
- `StarBreaker/crates/starbreaker-ui/Cargo.toml:8`
- `StarBreaker/crates/starbreaker-gfx/Cargo.toml:9`
- `StarBreaker/crates/starbreaker-blend/Cargo.toml:7`

**Interfaces** — none (dependency feature flag only).

**Steps**
- [ ] Root `Cargo.toml` `[workspace.dependencies]` — add beside `parking_lot`:
  ```toml
  [workspace.dependencies]
  parking_lot = "0.12"
  flate2 = { version = "1", features = ["zlib-rs"] }
  ```
- [ ] In each of the four crate manifests, replace the bare `flate2 = "1"` line with:
  ```toml
  flate2 = { workspace = true }
  ```
  (precedent: `starbreaker-p4k/Cargo.toml:16` already uses `parking_lot = { workspace = true }`).
- [ ] `cargo build` (debug). Expected: clean build; `cargo tree -i flate2` shows the zlib-rs backend feature, one flate2 version.
- [ ] `cargo test --workspace`. Expected: green (no behaviour change; inflate is bit-identical by format).

**Benchmark + byte oracle** (run from `/home/tom/projects/scorg_tools`; confirm `uptime` load < 2 first)
- [ ] Release build: `cd StarBreaker && cargo build --release && cd ..`
- [ ] `RUST_LOG=info StarBreaker/target/release/starbreaker entity export drak_clipper ships_perf_bench/clipper_post_item1 --kind decomposed --lod 0 --mip 0 --materials all 2>clipper_item1.log`
- [ ] Byte oracle (must be empty):
  ```
  diff -rq ships_perf_bench/clipper_pre1 ships_perf_bench/clipper_post_item1 | grep -v export_stamp
  ```
- [ ] Record before/after `[timing][decomposed]` / `[timing][blend]` totals (pre-change: 48.0–49.0 s from `clipper_pre{1..3}.log`) into the ledger.

**Ledger** (`StarBreaker/docs/optimisation-ledger.md`)
- [ ] Observed: bare `flate2` → miniz_oxide on P4K + SWF deflate hot paths. Finding: zlib-rs ~2× faster, inflate bit-identical. Action: hoisted `flate2 { workspace=true, features=["zlib-rs"] }`; byte oracle clean; timing Δ = <before>→<after>.

**Commit**
- [ ] `git add -A && git commit -m "perf(deps): switch flate2 to zlib-rs backend via workspace dependency"`

**Review placeholder**
- [ ] `// ORCHESTRATOR REVIEW — Phase A gate (run after Tasks 1–3): code-review security+perf lens + D1 spec compliance`

---

## TASK 2 — Item 8 pre-check probe: roughness decode serial vs parallel (Phase A, benchmark window)

Independent measurement; **gates Task 4**. No production code changes (test-only, `#[ignore]`d probe).

**Files**
- `StarBreaker/crates/starbreaker-3d/src/pipeline/textures.rs` — append a `#[cfg(test)]` probe to the existing test module (near the roughness tests). `load_roughness_texture_result` (:1209) is `pub(crate)`, so the probe must live in-crate, not in `tests/`.

**Interface consumed**
```rust
pub(crate) fn load_roughness_texture_result(
    p4k: &MappedP4k, tif_path: &str, mip_level: u32,
) -> Result<RoughnessTextureLoad, RoughnessTextureLoadError>
```
`load_roughness_texture_result` strips a `.tif` suffix else uses the path as-is, so feeding a real `*_ddna.dds` entry name works directly (its BC4 alpha-mip siblings live in the same P4K).

**Steps**
- [ ] Add the probe (gated on `SC_DATA_P4K`; skips silently if unset):
  ```rust
  #[test]
  #[ignore = "manual probe: needs SC_DATA_P4K; run with --ignored --nocapture"]
  fn precheck_roughness_decode_parallel_speedup() {
      use rayon::prelude::*;
      use std::time::Instant;
      let Ok(p4k_path) = std::env::var("SC_DATA_P4K") else { return };
      let p4k = starbreaker_p4k::MappedP4k::open(&p4k_path).expect("open p4k");
      let sources: Vec<String> = p4k
          .entries()
          .iter()
          .filter(|e| e.name.to_ascii_lowercase().ends_with("_ddna.dds"))
          .take(50)
          .map(|e| e.name.clone())
          .collect();
      assert!(sources.len() >= 20, "need a representative DDNA slice, got {}", sources.len());
      let t = Instant::now();
      for s in &sources { let _ = load_roughness_texture_result(&p4k, s, 0); }
      let serial = t.elapsed();
      let t = Instant::now();
      let _: Vec<_> = sources.par_iter().map(|s| load_roughness_texture_result(&p4k, s, 0)).collect();
      let par = t.elapsed();
      eprintln!(
          "[precheck][item8] n={} serial={:?} par={:?} speedup={:.2}x",
          sources.len(), serial, par, serial.as_secs_f64() / par.as_secs_f64()
      );
  }
  ```
  (`rayon` is already a dependency of `starbreaker-3d` via the pipeline; if the test module lacks the import it is added locally as above.)
- [ ] Run: `cd StarBreaker && SC_DATA_P4K=<Data.p4k> cargo test -p starbreaker-3d --lib precheck_roughness_decode_parallel_speedup -- --ignored --nocapture`
- [ ] Expected: one `[precheck][item8] … speedup=N.NNx` line.

**Gate decision (records into ledger regardless)**
- [ ] If `speedup > 4×` → **Task 4 proceeds**. Ledger: Observed serial/par times, Finding: DDS decode CPU-bound, speedup N×, Action: proceed to prewarm.
- [ ] If `speedup ≤ 4×` → **STOP item 8**. Ledger: dead-end recorded with the numbers; Task 4 skipped. Delete the probe (or leave `#[ignore]`d — orchestrator's call).

**Review placeholder** — folded into Phase A gate (Task 1).

---

## TASK 3 — Item 10 Phase 1 probe: P4K zstd-decode share (Phase A, benchmark window)

Independent measurement; **gates Task 7**. Temporary instrumentation, reverted or kept per D12.

**Files**
- `StarBreaker/crates/starbreaker-p4k/src/archive.rs` — `zstd_decompress` (:796); add a static nanos accumulator + public getter.
- `StarBreaker/cli/src/entity.rs` — log the accumulated total once, right after the export call returns (single, revertible point; matches `[timing]` idiom).

**Interfaces produced**
```rust
// archive.rs
pub static ZSTD_DECODE_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn zstd_decode_nanos() -> u64 { ZSTD_DECODE_NANOS.load(std::sync::atomic::Ordering::Relaxed) }
```

**Steps**
- [ ] In `archive.rs`, add the static + getter near the top, and instrument `zstd_decompress`:
  ```rust
  fn zstd_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, P4kError> {
      let _probe = std::time::Instant::now();
      let cursor = std::io::Cursor::new(data);
      let mut decoder = ruzstd::decoding::StreamingDecoder::new(cursor)
          .map_err(|e| P4kError::Decompression(format!("zstd init: {e}")))?;
      let mut output = Vec::with_capacity(size_hint);
      decoder.read_to_end(&mut output)
          .map_err(|e| P4kError::Decompression(format!("zstd: {e}")))?;
      ZSTD_DECODE_NANOS.fetch_add(_probe.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
      Ok(output)
  }
  ```
- [ ] In `cli/src/entity.rs`, immediately after the export call completes, add:
  ```rust
  log::info!(
      "[timing][p4k] zstd_decode_total: {:.2}s",
      starbreaker_p4k::zstd_decode_nanos() as f64 / 1e9,
  );
  ```
  (Confirm `starbreaker-p4k` re-exports the getter from its crate root; add `pub use archive::zstd_decode_nanos;` in `lib.rs` if not already public.)
- [ ] Release build, then export the biggest ship (Carrack) with `RUST_LOG=info`:
  ```
  cd StarBreaker && cargo build --release && cd ..
  RUST_LOG=info StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_item10probe --kind decomposed --lod 0 --mip 0 --materials all 2>carrack_item10probe.log
  ```
- [ ] Compute share = `zstd_decode_total / [timing] TOTAL export`. Expected: one `[timing][p4k] zstd_decode_total` line.

**Gate decision**
- [ ] `share ≤ 5%` → **KEEP ruzstd**; Task 7 skipped. Ledger: Observed share N%, Finding: decode not a meaningful share (mmap-bound elsewhere), Action: KEEP.
- [ ] `share > 5%` → **Task 7 proceeds**. Ledger: Observed share N%, Action: swap planned.
- [ ] Revert the probe instrumentation before/independent of Task 7 (Task 7 re-adds nothing — the swap itself is the change). Keep the ledger numbers.

**Review placeholder** — folded into Phase A gate (Task 1).

---

## TASK 4 — Item 8: DDNA→roughness parallel pre-decode (Phase C, GATED on Task 2 > 4×)

**Files**
- `StarBreaker/crates/starbreaker-3d/src/pipeline/textures.rs` — add `RoughnessCache` type alias (beside `PngCache` at :18).
- `StarBreaker/crates/starbreaker-3d/src/decomposed.rs` — new prewarm producer; new `roughness_cache` param threaded from `write_decomposed_export` (:1016) to `export_ddna_roughness_asset_with_status` (:4896); consult at :4950.
- `StarBreaker/crates/starbreaker-3d/src/pipeline/blend_assembly.rs` — build the cache beside `prewarmed_png_cache` (:978) and pass at the `write_decomposed_export` call (:1028).

**Interfaces produced**
```rust
// textures.rs — next to `pub(crate) type PngCache = HashMap<String, Option<Vec<u8>>>;`
pub(crate) type RoughnessCache =
    std::collections::HashMap<String, Result<RoughnessTextureLoad, RoughnessTextureLoadError>>;

// decomposed.rs
pub(crate) fn prewarm_decomposed_roughness(
    p4k: &MappedP4k,
    assets: &[(MtlFile, String, String)],
    texture_mip: u32,
) -> RoughnessCache
```
`RoughnessTextureLoad` derives `Clone` (textures.rs:30); `RoughnessTextureLoadError` is `Clone+Copy` (:20) — so a cached `Result` clones cheaply on a hit.

**⚠ CRITICAL trap** (D5): consult the new cache **before** `load_roughness_texture_result` at :4950 only. **Never** prewarm `ddna_status_cache` — its early-return at :4941 precedes the `insert_binary_file` at :4974, so seeding it drops every roughness PNG. The prewarm holds the pure decode `Result`; the serial writer still does all `files`/`texture_cache`/`ddna_status_cache` inserts.

**Steps**
- [ ] Add `RoughnessCache` alias to `textures.rs` and re-export via the existing `pipeline` re-export path if `PngCache` is re-exported there.
- [ ] Add `prewarm_decomposed_roughness` to `decomposed.rs`. Enumerate DDNA sources from the SAME asset set the writer processes, using the existing enumerator `normal_gloss_ddna_source_paths(&SubMaterial)` (:4609) which returns RAW `binding.path` strings — matching the writer's `source_path` key semantics exactly:
  ```rust
  //! (function goes in decomposed.rs — no new file, no //! needed)
  pub(crate) fn prewarm_decomposed_roughness(
      p4k: &MappedP4k,
      assets: &[(MtlFile, String, String)],
      texture_mip: u32,
  ) -> RoughnessCache {
      use rayon::prelude::*;
      let mut sources: HashSet<String> = HashSet::new();
      for (materials, _material_path, _geometry_path) in assets {
          for material in &materials.materials {
              for src in normal_gloss_ddna_source_paths(material) {
                  sources.insert(src);
              }
          }
      }
      let jobs: Vec<String> = sources.into_iter().collect();
      jobs.par_iter()
          .map(|src| (src.clone(), crate::pipeline::load_roughness_texture_result(p4k, src, texture_mip)))
          .collect()
  }
  ```
  Note: a source that fails to enumerate here is not a correctness bug — the writer falls back to its own `load_roughness_texture_result` on a cache miss (identical to today), exactly as the `png_cache` prewarm already tolerates misses.
- [ ] Thread `roughness_cache: &RoughnessCache` alongside every `ddna_status_cache` occurrence. Grep map (`crates/starbreaker-3d/src/decomposed.rs`): param declarations at :2467, :2575, :4900; pass-through call args at :2505, :2589, :2711, :1179, :1235, :1379, :1697; cache created at :1109. Add `roughness_cache` as a read-only `&` param at each (it is prewarmed, consult-only). `write_decomposed_export` (:1016) gains a `roughness_cache: RoughnessCache` param after `png_cache` (:1024); bind `let roughness_cache = roughness_cache;` and pass `&roughness_cache` downstream.
- [ ] At `export_ddna_roughness_asset_with_status` (:4896), add the `roughness_cache: &RoughnessCache` param and replace the scrutinee at :4950:
  ```rust
  let result = match roughness_cache
      .get(source_path)
      .cloned()
      .unwrap_or_else(|| crate::pipeline::load_roughness_texture_result(p4k, source_path, texture_mip))
  {
      Ok(loaded) => { /* unchanged body */ }
      Err(err) => { /* unchanged body */ }
  };
  ```
  Leave the `ddna_status_cache` memo (:4941, :5032), `files` inserts, and `texture_cache` untouched.
- [ ] In `blend_assembly.rs`, after the `prewarmed_png_cache` block (:978–985), build the roughness cache from the same `prewarm_assets` vec and log timing:
  ```rust
  let prewarm_rough_start = Instant::now();
  let prewarmed_roughness_cache =
      crate::decomposed::prewarm_decomposed_roughness(p4k, &prewarm_assets, opts.texture_mip);
  log::info!(
      "[timing][blend] prewarm_roughness: {:.2}s ({} sources)",
      prewarm_rough_start.elapsed().as_secs_f32(),
      prewarmed_roughness_cache.len(),
  );
  ```
  Pass `prewarmed_roughness_cache` into the `write_decomposed_export` call (:1028) after `prewarmed_png_cache`.
- [ ] **Only one caller** of `write_decomposed_export` exists (`blend_assembly.rs:1028`); the glb/socpak/glb-assembly paths all route through `write_decomposed_export_blend`, which internally calls it once. No empty-cache call site is needed (see Planner flag F1). If a second direct caller is added later it passes `RoughnessCache::new()`.

**Unit test** (in `decomposed.rs` `#[cfg(test)]`, integration-gated on `SC_DATA_P4K`, mirroring the roughness tests near :5751/:5812)
- [ ] Assert a prewarmed source yields an identical `(TextureExportRef, TextureDerivationStatus)` AND the roughness PNG is present in `files`, versus the cold path:
  ```rust
  #[test]
  #[ignore = "needs SC_DATA_P4K"]
  fn prewarmed_roughness_matches_cold_and_emits_png() {
      let Ok(p) = std::env::var("SC_DATA_P4K") else { return };
      let p4k = MappedP4k::open(&p).unwrap();
      let src = p4k.entries().iter()
          .find(|e| e.name.to_ascii_lowercase().ends_with("_ddna.dds"))
          .map(|e| e.name.clone()).expect("a ddna source");
      // cold
      let mut f0 = OutputFiles::new(); let mut tc0 = HashMap::new(); let mut sc0 = HashMap::new();
      let empty = RoughnessCache::new();
      let cold = export_ddna_roughness_asset_with_status(&mut f0, &p4k, &mut tc0, &mut sc0, &empty, &src, 0, None);
      // prewarmed
      let warm_cache: RoughnessCache = std::iter::once((
          src.clone(), crate::pipeline::load_roughness_texture_result(&p4k, &src, 0),
      )).collect();
      let mut f1 = OutputFiles::new(); let mut tc1 = HashMap::new(); let mut sc1 = HashMap::new();
      let warm = export_ddna_roughness_asset_with_status(&mut f1, &p4k, &mut tc1, &mut sc1, &warm_cache, &src, 0, None);
      assert_eq!(cold.0, warm.0);
      assert_eq!(cold.1, warm.1);
      let rel = texture_relative_path(&p4k, &src, TextureFlavor::Roughness, 0);
      assert!(f1.contains_key(&rel), "roughness PNG must be inserted on the prewarmed path");
  }
  ```
  (Adjust helper names to the actual `OutputFiles`/`RoughnessCache` construction visible in the test module; `TextureExportRef`/`TextureDerivationStatus` must be `PartialEq` — confirm, else compare field-wise.)
- [ ] `cargo build && cargo test -p starbreaker-3d --lib` (green; gated test skips without env).

**Byte + RSS + determinism verification** (from `/home/tom/projects/scorg_tools`)
- [ ] `cd StarBreaker && cargo build --release && cd ..`
- [ ] Carrack + Clipper, two roots each (determinism twin):
  ```
  RUST_LOG=info StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_item8_a --kind decomposed --lod 0 --mip 0 --materials all 2>carrack_item8_a.log
  StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_item8_b --kind decomposed --lod 0 --mip 0 --materials all
  StarBreaker/target/release/starbreaker entity export drak_clipper ships_perf_bench/clipper_item8 --kind decomposed --lod 0 --mip 0 --materials all
  ```
- [ ] Byte oracle vs pre-change (empty):
  ```
  diff -rq ships_perf_bench/carrack_pre1 ships_perf_bench/carrack_item8_a | grep -v export_stamp
  diff -rq ships_perf_bench/clipper_pre1 ships_perf_bench/clipper_item8   | grep -v export_stamp
  ```
- [ ] Determinism (strictly empty):
  ```
  diff -rq ships_perf_bench/carrack_item8_a ships_perf_bench/carrack_item8_b | grep -v export_stamp
  ```
- [ ] Max-RSS not materially above baseline 9.1 G:
  ```
  /usr/bin/time -v StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_item8_rss --kind decomposed --lod 0 --mip 0 --materials all 2>carrack_item8_rss.log
  grep "Maximum resident" carrack_item8_rss.log
  ```
- [ ] Record child/interior `[timing][decomposed]` + `prewarm_roughness` deltas in ledger.

**Ledger + Commit**
- [ ] Ledger: Observed serial DDNA first-decode inside interior loop; Finding: prewarm par_iter cut it by N s at byte-identity; Action: `prewarm_decomposed_roughness` + consult-only cache; ddna_status_cache untouched (trap noted).
- [ ] `git commit -m "perf(3d): parallel pre-decode of DDNA roughness textures before decomposed writer"`

**Review placeholder**
- [ ] `// ORCHESTRATOR REVIEW — Phase C gate (after Tasks 4–5): code-review + D5 trap check (ddna_status_cache untouched) + byte/RSS/determinism oracles`

---

## TASK 5 — Item 12.1 + 12.2: per-CGF precompute + threaded mtl_cache (Phase C, one commit)

**Files**
- `StarBreaker/crates/starbreaker-3d/src/decomposed.rs` — placement loop (:1555), recomputations at :1585-1593 and :1755-1760; prewarm `mtl_cache` (:4814); writer `mtl_cache` (:1111).
- `StarBreaker/crates/starbreaker-3d/src/pipeline/blend_assembly.rs` — thread prewarm's returned `mtl_cache` into `write_decomposed_export`.

**Interfaces**
- 12.1: a `Vec<PrecomputedCgf>` indexed by `mesh_index`, built once before the container loop.
  ```rust
  struct PrecomputedCgf {
      normalized_cgf_path: String,
      normalized_material_path: Option<String>,
      cache_key: String,
  }
  ```
- 12.2: `prewarm_decomposed_textures` (:4806) return type changes
  `PngCache` → `(PngCache, HashMap<String, Option<MtlFile>>)` (its internal `mtl_cache` at :4814); `write_decomposed_export` seeds its `mtl_cache` (:1111) from it.

**Steps (12.1)**
- [ ] Before the `for (index, container) …` loop at :1555, precompute over `input.interiors.unique_cgfs` (`Vec<InteriorCgfEntry>`):
  ```rust
  let precomputed_cgfs: Vec<PrecomputedCgf> = input
      .interiors
      .unique_cgfs
      .iter()
      .map(|entry| {
          let normalized_cgf_path = normalize_source_path(p4k, &entry.cgf_path);
          let normalized_material_path = entry
              .material_path
              .as_deref()
              .map(|path| normalize_source_path(p4k, path));
          let cache_key = interior_asset_lookup_key(
              &normalized_cgf_path,
              normalized_material_path.as_deref(),
          );
          PrecomputedCgf { normalized_cgf_path, normalized_material_path, cache_key }
      })
      .collect();
  ```
- [ ] Replace the per-placement recomputation at :1585-1593 with a lookup:
  ```rust
  let precomp = &precomputed_cgfs[placement.mesh_index];
  let cache_key = precomp.cache_key.clone();
  // (normalized_cgf_path / normalized_material_path available as precomp.* )
  ```
- [ ] Replace the duplicated recomputation at :1755-1760 (the `InteriorPlacementRecord` fields) with `precomp.normalized_cgf_path.clone()` and `precomp.normalized_material_path.clone()`. Byte-identical: same `normalize_source_path` values, same first-seen casing (the `case_index` is unaffected — it only records what is written, and identical strings are written).

**Steps (12.2)**
- [ ] Change `prewarm_decomposed_textures` (:4806) to return `(PngCache, HashMap<String, Option<MtlFile>>)`, returning its local `mtl_cache` (:4814) as the second element (currently dropped). Keep the parallel decode exactly as-is.
- [ ] In `blend_assembly.rs`, destructure the return (`let (prewarmed_png_cache, prewarmed_mtl_cache) = …`) at :978, add it to the timing log, and pass `prewarmed_mtl_cache` into `write_decomposed_export`.
- [ ] `write_decomposed_export` gains a `mtl_cache: HashMap<String, Option<MtlFile>>` param; replace the `let mut mtl_cache: … = HashMap::new();` at :1111 with `let mut mtl_cache = mtl_cache;`. The single caller passes the prewarmed cache; if a second caller appears it passes `HashMap::new()`.
- [ ] If Task 4 already added params to `write_decomposed_export`, order params consistently (`png_cache`, `roughness_cache`, `mtl_cache`) — land Task 5 after Task 4 so signatures compose cleanly, or reconcile if landed independently.

**Verification** (byte-identical)
- [ ] `cargo build && cargo test -p starbreaker-3d --lib` green.
- [ ] `cd StarBreaker && cargo build --release && cd ..`, then Carrack + Clipper:
  ```
  RUST_LOG=info StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_item12 --kind decomposed --lod 0 --mip 0 --materials all 2>carrack_item12.log
  StarBreaker/target/release/starbreaker entity export drak_clipper ships_perf_bench/clipper_item12 --kind decomposed --lod 0 --mip 0 --materials all
  diff -rq ships_perf_bench/carrack_pre1 ships_perf_bench/carrack_item12 | grep -v export_stamp
  diff -rq ships_perf_bench/clipper_pre1 ships_perf_bench/clipper_item12 | grep -v export_stamp
  ```
  Both empty.
- [ ] Record `interior_asset_resolve` timing delta (pre-change ~24 k `normalize_source_path` calls on Carrack) in the ledger.

**Ledger + Commit**
- [ ] Ledger (two Observed/Finding/Action stanzas, one item): 12.1 precompute per-CGF normals; 12.2 thread prewarm mtl_cache.
- [ ] `git commit -m "perf(3d): precompute per-CGF normalized paths and reuse prewarm mtl cache"`

**Review placeholder** — folded into Phase C gate (Task 4).

---

## TASK 6 — Item 2: case-canonicalization bug + cleanup + fresh baselines (Phase D, LAST byte-identical item)

Lands **after** Tasks 1, 4, 5 so their byte oracles stayed valid. Uses the **case-modulo oracle**. TDD mandatory.

### Root cause (D2, binding)
Three divergent casing authorities in `decomposed.rs`:
1. `texture_relative_path` (:3895) → `normalize_source_path` (:3942) — **correct** (P4K-entry casing). Leave as the reference.
2. `mesh_asset_relative_path` (:3864) → `normalize_requested_source_path` (:3938, source-string casing). `let _ = p4k;` at :3879 is the fix seam.
3. `material_sidecar_relative_path` (:3885) → `normalize_material_source_for_manifest` (:3748) which **force-lowercases** after `Data/` (:3755).

Fix: unify both (2) and (3) on `normalize_source_path` (P4K-entry casing, deterministic, run-independent). `OutputFiles.case_index` stays as belt-and-braces.

### TDD (failing test first — integration-gated; no in-memory `MappedP4k` exists, see F2)
**Files** — `decomposed.rs` `#[cfg(test)]` module.
- [ ] Write the failing invariant test (gated on `SC_DATA_P4K`), before any fix:
  ```rust
  #[test]
  #[ignore = "needs SC_DATA_P4K"]
  fn path_builders_use_p4k_entry_casing_regardless_of_input_case() {
      let Ok(p) = std::env::var("SC_DATA_P4K") else { return };
      let p4k = MappedP4k::open(&p).unwrap();
      // pick a real .cgf and its .mtl with mixed-case P4K entry names
      let cgf = p4k.entries().iter()
          .find(|e| e.name.to_ascii_lowercase().ends_with(".cgf"))
          .map(|e| e.name.clone()).expect("a cgf");
      let canonical = normalize_source_path(&p4k, &cgf);            // reference (texture authority)
      let upper = cgf.to_ascii_uppercase();
      let lower = cgf.to_ascii_lowercase();
      // mesh asset builder must yield the SAME base casing for any input casing
      let m_up = mesh_asset_relative_path(&p4k, &upper, "n", 0, ExportFormat::Blend);
      let m_lo = mesh_asset_relative_path(&p4k, &lower, "n", 0, ExportFormat::Blend);
      assert_eq!(m_up, m_lo, "mesh asset path must be case-stable");
      assert!(m_up.to_lowercase().starts_with(&replace_extension(&canonical, "").to_lowercase()));
      // and it must match the texture authority's casing, not the input's
      assert!(m_up.starts_with(&replace_extension(&canonical, "")),
          "mesh asset must adopt P4K-entry casing ({canonical}), got {m_up}");
  }
  ```
  Add an analogous assertion for the material sidecar builder once its signature is settled (below).
- [ ] Run `SC_DATA_P4K=<Data.p4k> cargo test -p starbreaker-3d --lib path_builders_use_p4k_entry_casing -- --ignored --nocapture`; **confirm it FAILS** against unfixed code.

### Fix
- [ ] `mesh_asset_relative_path` (:3864): drop `let _ = p4k;` (:3879) and swap the builder at :3880:
  ```rust
  replace_extension(&normalize_source_path(p4k, geometry_path), extension)
  ```
- [ ] `normalize_material_source_for_manifest` (:3748): add a `p4k: &MappedP4k` first param and replace the force-lowercase body with the P4K-entry authority:
  ```rust
  fn normalize_material_source_for_manifest(p4k: &MappedP4k, path: &str) -> String {
      normalize_source_path(p4k, path)
  }
  ```
  Thread `p4k` to its callers (grep map, `decomposed.rs`): `.map(|path| normalize_material_source_for_manifest(&path))` at :1191, :1247, :1391, :1679, :1709 → add `p4k` (in scope at all five); `:3745` (`canonical_material_source_path`), `:3846` (`reusable_interior_asset_paths`, `p4k` in scope). `material_sidecar_relative_path` (:3885) internally calls it at :3886 — give `material_sidecar_relative_path` a `p4k: &MappedP4k` param and thread p4k to ITS callers at :2485, :3733, :3849 (all have `p4k`).
- [ ] **Reconcile existing casing unit tests** — these encode the OLD (buggy) invariant and must be updated as part of the TDD, not left green:
  - `material_sidecar_relative_path_normalizes_case` (:8012) — asserts lowercasing; delete or rewrite to the P4K-casing invariant (it now needs a `p4k`; convert to the gated integration form or drop, since the new invariant test above covers it).
  - `material_sidecar_relative_path_uses_source_mtl_not_geometry_path` (:7983), `_encodes_mip_level` (:8002-8004) — add the `p4k` arg; use a path whose casing is preserved by the fallback (they don't depend on casing). Confirm they still assert extension/mip/source-vs-geometry logic.
- [ ] Re-run the gated invariant test; **confirm it now PASSES**. `cargo test -p starbreaker-3d --lib` green.

### Case-modulo oracle verification (from `/home/tom/projects/scorg_tools`)
- [ ] `cd StarBreaker && cargo build --release && cd ..`
- [ ] Two roots (determinism twin):
  ```
  StarBreaker/target/release/starbreaker entity export drak_clipper ships_perf_bench/clipper_item2_a --kind decomposed --lod 0 --mip 0 --materials all
  StarBreaker/target/release/starbreaker entity export drak_clipper ships_perf_bench/clipper_item2_b --kind decomposed --lod 0 --mip 0 --materials all
  StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_item2_a --kind decomposed --lod 0 --mip 0 --materials all
  ```
- [ ] **Determinism (strictly empty)** — post-fix casing is run-independent:
  ```
  diff -rq ships_perf_bench/clipper_item2_a ships_perf_bench/clipper_item2_b | grep -v export_stamp
  ```
- [ ] **Case-modulo oracle** vs pre-change baseline (same path set modulo case + identical content; any non-case difference is a bug):
  ```
  uv run python - <<'PY'
  import os
  a="ships_perf_bench/clipper_pre1"; b="ships_perf_bench/clipper_item2_a"
  def idx(root):
      m={}
      for dp,_,fs in os.walk(root):
          for f in fs:
              rel=os.path.relpath(os.path.join(dp,f),root)
              if rel.endswith("export_stamp"): continue
              m[rel.lower()]=os.path.join(dp,f)
      return m
  A,B=idx(a),idx(b)
  miss=set(A)^set(B)
  assert not miss, ("path-set differs modulo case:", sorted(miss)[:20])
  bad=[k for k in A if open(A[k],'rb').read()!=open(B[k],'rb').read()]
  print("CASE-MODULO OK" if not bad else ("CONTENT MISMATCH:", bad[:20]))
  PY
  ```
- [ ] UI PNG bytes unchanged: `cargo test -p starbreaker-ui` freeze SHAs green.

### Cleanup (D2 §cleanup — after the fix lands)
Operates on the accumulating tree at `/home/tom/projects/scorg_tools/ships`.
- [ ] Verify `ships/Data/objects` is a strict byte-subset of `ships/Data/Objects` per shared subdir:
  ```
  diff -rq ships/Data/objects ships/Data/Objects   # expect only "Only in objects" absent; identical files
  ```
  then `rm -rf ships/Data/objects`.
- [ ] `materials` (26 M) is NOT a subset of `Materials` (14 M): **merge, don't blind-delete**. Any file present only in `ships/Data/materials` must move into the canonical `ships/Data/Materials` casing, then delete the lowercase tree:
  ```
  diff -rq ships/Data/materials ships/Data/Materials   # enumerate "Only in materials" files
  # move each unique lowercase-only file to canonical casing (manual/rsync --ignore-existing), then:
  rm -rf ships/Data/materials
  ```
- [ ] Confirm `scene.json` references now match on-disk casing after the fix (grep a sample of `mesh_asset`/`material_sidecar` paths against the surviving `Data/Objects`/`Data/Materials` casing).

### Fresh baselines
- [ ] After item 2 lands, capture new byte oracles for anything later (Task 7 verifies against these):
  ```
  StarBreaker/target/release/starbreaker entity export drak_clipper ships_perf_bench/clipper_post2 --kind decomposed --lod 0 --mip 0 --materials all
  StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_post2 --kind decomposed --lod 0 --mip 0 --materials all
  ```

**Ledger + Commit**
- [ ] Ledger: Observed dual-cased `Data/objects` + `Data/materials` trees; Finding: mesh/material builders diverged from the texture (P4K-entry) authority; Action: unified on `normalize_source_path`; cleanup reclaimed ~1.6 G; case-modulo + determinism oracles clean.
- [ ] `git commit -m "fix(3d): canonicalize mesh and material sidecar paths to P4K-entry casing"`

**Review placeholder**
- [ ] `// ORCHESTRATOR REVIEW — Phase D gate: code-review + D2 spec (three authorities unified) + case-modulo + determinism oracles + cleanup subset/merge check`

---

## TASK 7 — Item 10: P4K ruzstd → zstd swap (Phase E, GATED on Task 3 > 5%)

**Files**
- `StarBreaker/crates/starbreaker-p4k/src/archive.rs` — `zstd_decompress` (:796) decode swap; remove the Task-3 probe instrumentation.
- `StarBreaker/crates/starbreaker-p4k/Cargo.toml` — drop `ruzstd` (:10), add `zstd = "0.13"`.
- (See F3 for the chf verdict — chf is left on ruzstd.)

**Interfaces** — internal; `zstd_decompress` signature unchanged (bytes identical by format).

**Steps**
- [ ] Swap the P4K decode body:
  ```rust
  fn zstd_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, P4kError> {
      let mut out = Vec::with_capacity(size_hint);
      zstd::stream::copy_decode(std::io::Cursor::new(data), &mut out)
          .map_err(|e| P4kError::Decompression(format!("zstd: {e}")))?;
      Ok(out)
  }
  ```
  (`zstd::stream::copy_decode` uses the size_hint-preallocated `out`; or `zstd::stream::decode_all(data)` if simpler.)
- [ ] `Cargo.toml`: remove `ruzstd = "0.8"`, add `zstd = "0.13"` (already linked workspace-wide via `starbreaker-blend`/`mcp`, so no new toolchain).
- [ ] `cargo build && cargo test --workspace` green (p4k has `#[cfg(test)]` decode paths).

**Verification** (byte-identical vs FRESH post-item-2 baselines)
- [ ] `cd StarBreaker && cargo build --release && cd ..`
  ```
  RUST_LOG=info StarBreaker/target/release/starbreaker entity export anvl_carrack ships_perf_bench/carrack_item10 --kind decomposed --lod 0 --mip 0 --materials all 2>carrack_item10.log
  diff -rq ships_perf_bench/carrack_post2 ships_perf_bench/carrack_item10 | grep -v export_stamp
  ```
  Empty. Record `[timing]` TOTAL delta vs Task-3 probe run.

**Ledger + Commit**
- [ ] Ledger: Observed zstd-decode share N% (>5%); Finding: C-zstd faster, bytes identical by format; Action: swapped p4k decode to `zstd` crate, dropped `ruzstd` from starbreaker-p4k; chf kept on ruzstd (F3 tradeoff).
- [ ] `git commit -m "perf(p4k): decode zstd entries via the zstd crate, drop ruzstd dependency"`

**Review placeholder**
- [ ] `// ORCHESTRATOR REVIEW — Phase E gate: code-review + byte oracle vs post-item-2 baselines + D12 verdict recorded`

---

## Planner flags (for the orchestrator — do NOT resolve here)

- **F1 (D5 wording vs reality).** D5/research says "non-blend callers (pipeline/mod.rs:568 glb path) pass PngCache::new() and need the new empty arg too." Verified: `write_decomposed_export` (decomposed.rs) has **exactly one** caller — `blend_assembly.rs:1028`. The glb/socpak paths call `write_decomposed_export_blend` (blend_assembly.rs:826), which itself does the prewarm and calls `write_decomposed_export` once. So no separate empty-cache call site is required for Task 4 or Task 5; the plan reflects the single-caller reality. Flagging because the design text implies a second site.

- **F2 (D2 test infra decision).** No in-memory `MappedP4k` constructor exists — all fields private (owned.rs:17-25), only `open(path)` / `open_with_progress`. `archive.rs:475 build_archive_indexes(entries)` + the `#[cfg(test)] make_entry` helper could seed one, but exposing it requires a **public** (not `#[cfg(test)]`) constructor on `MappedP4k` visible to the downstream `starbreaker-3d` crate — new production API surface purely for a test. Per D2's stated fallback ("else integration-gated — planner decides"), **verdict: integration-gated** (`SC_DATA_P4K`, `#[ignore]`d), matching the repo's existing `phase_6b_integration_test`/`pipeline_integration_test` pattern. Consequence: the item-2 TDD test does not run in a bare `cargo test` — the orchestrator must run it explicitly with `SC_DATA_P4K` set to see it fail then pass. Alternative (add a `pub fn MappedP4k::from_entry_names`) is available if the owner wants an env-free unit test.

- **F3 (D12 chf encode verdict).** `to_chf` (ruzstd ENCODE, container.rs:82) is reachable ONLY via `json_to_chf` (chf/lib.rs:41,:53) → `cli/src/chf.rs:48`, a standalone `chf` CLI subcommand that writes game-consumed 4096-byte character save files. It is **not** in any decomposed-export byte oracle (clipper/carrack) nor in a byte-comparing regression test (the chf roundtrip tests `v8_roundtrip.rs`/`bulk_test.rs` only call `from_chf` DECODE). Different zstd implementations emit different frames, and no test proves the game accepts a `zstd`-crate frame → **keep ruzstd for chf encode**. Since chf must retain ruzstd for encode regardless, swapping chf's DECODE (:62) to `zstd` would ADD `zstd` to the chf crate while NOT removing `ruzstd` — negative consolidation, and chf decode is off the capital-ship export hot path (character files aren't read during `entity export`). **Recommendation: scope Task 7 to the P4K decode only; leave starbreaker-chf entirely on ruzstd.** This deviates from D12's literal "swap its DECODE (:62) too" — flagging for the orchestrator to confirm; the plan currently implements the recommendation and records the tradeoff in the ledger per D12's own "keep ruzstd for chf encode and record the partial-consolidation tradeoff" clause.

- **F4 (signature composition).** Tasks 4 and 5 both add params to `write_decomposed_export` and `export_ddna_roughness_asset_with_status` / prewarm. Land Task 4 before Task 5 (or reconcile) so the param order settles as `(… png_cache, roughness_cache, mtl_cache …)` once. Both are Phase C; sequential build discipline already forces ordering.
---
plan: Perf handoff — Workstream B (UI crate: D3 item 6 + D4 item 7)
branch: feature/ui
oracle: /home/tom/projects/scorg_tools/ships_perf_bench/clipper_pre1
depends_on: Phase A item 1 (flate2 → zlib-rs) already landed
---

# Plan — UI-crate perf workstream (items 6 & 7)

Workstream B of the 2026-07-18 perf handoff. Covers design decisions **D3**
(item 6: colour.rs opaque fast path + step-0 profile) and **D4** (item 7:
SWF parse-once, GATED on the profile). PLAN ONLY — implementer executes.

## Global constraints (apply to every task)

- Commit directly on `feature/ui`. No branches, no worktrees, no commit
  trailers, never name the owner.
- Builds are **sequential** (shared `target/`) — never run concurrent cargo.
  `cargo build` (debug = `[optimized+debuginfo]`) for iteration; `--release`
  only for the benchmark export.
- Benchmark rule: run `uptime` first and proceed only if 1-min load < 2; the
  benchmark window closes ~07:00.
- Every new `.rs` file starts with a `//!` header. No hard-coded game-data
  values. No `.unwrap_or` silencers / `.max()` floors. Root-cause only.
- `cargo test -p starbreaker-ui` must stay **green with the freeze SHAs
  UNCHANGED** after items 6 and 7 (both byte-identical by construction).
- **Byte-identity oracle** for items 6 & 7:
  `/home/tom/projects/scorg_tools/ships_perf_bench/clipper_pre1`. After a
  change, re-export the Clipper into a fresh scratch root and:
  `diff -rq ships_perf_bench/clipper_pre1 <newroot> | grep -v export_stamp`
  → empty; plus a `cmp` loop over the 26 generated UI PNGs.
- Canonical export (run from `/home/tom/projects/scorg_tools`; P4K
  auto-detected):
  `StarBreaker/target/release/starbreaker entity export drak_clipper <root> --kind decomposed --lod 0 --mip 0 --materials all`
- After each landed item append an Observed/Finding/Action entry to
  `StarBreaker/docs/optimisation-ledger.md`. One commit per item.
- TDD where a test surface exists (colour.rs has one; SWF has one).
- Never simplify away: the byte oracle, the two-run determinism check, the
  exhaustive colour boundary test, the CharacterId-sorted determinism fix.

## Task order

- **Task 1** — step-0 profile + gate verdict. **RUNS FIRST** (Phase A tail;
  needs post-zlib-rs numbers). Produces the written item-7 GATE verdict.
- **Task 2** — colour.rs fast path. Phase B. Byte-identical. **Independent of
  Task 1's verdict** (D3 pre-decided it lands); only ordered after Task 1 so
  the profile row is captured on the pre-fastpath build.
- **Task 3** — SWF parse-once (item 7). Phase B. **CONDITIONAL** on Task 1's
  verdict = GATED-IN. Fully specified regardless. Order-bound AFTER Task 2 (or
  independent of Task 2 content, but land Task 2 first so its byte-oracle run
  isn't entangled).

---

## Task 1 — Item 6 step-0 profile + item-7 GATE verdict (RUNS FIRST)

**Goal:** capture a fresh post-zlib-rs per-stage UI timing baseline on the
Clipper, append a dated block to the baseline doc, and write an explicit
GATE verdict deciding whether Task 3 (item 7) proceeds.

**Precondition:** item 1 (zlib-rs) is landed on `feature/ui` (Phase A). Confirm
with `grep -n 'zlib-rs' StarBreaker/Cargo.toml StarBreaker/crates/starbreaker-ui/Cargo.toml`.

**Baseline doc:** `/home/tom/projects/scorg_tools/docs/StarBreaker/starbreaker-ui-perf-baseline.md`
(EXISTS — append a new dated block, do not overwrite the B0 block).

Steps:

- [ ] Confirm the release binary is current: from `/home/tom/projects/scorg_tools`
      run `cargo build --release -p starbreaker --manifest-path StarBreaker/Cargo.toml`
      (sequential; no other cargo running). Expected: `Finished release`.
- [ ] `uptime` → confirm 1-min load < 2 and it is before ~07:00.
- [ ] Create scratch roots (throwaway — do NOT use the oracle dir):
      `mkdir -p /home/tom/projects/scorg_tools/ships_perf_bench/clipper_profile_serial /home/tom/projects/scorg_tools/ships_perf_bench/clipper_profile_par`
- [ ] **Serial per-image run** (per-image isolation), from `/home/tom/projects/scorg_tools`:
      ```bash
      SB_UI_TIMING=1 RAYON_NUM_THREADS=1 RUST_LOG=info \
        StarBreaker/target/release/starbreaker entity export drak_clipper \
        ships_perf_bench/clipper_profile_serial --kind decomposed --lod 0 --mip 0 --materials all \
        2>&1 | tee /tmp/claude-1000/-home-tom-projects-scorg-tools/87ac34cf-d805-406e-8cc7-31ca20c9f92f/scratchpad/clipper_ui_timing_serial.log
      ```
      Expected: ~40+ `[timing][ui]` blocks (one per binding), each with
      `fetch / resolve / swf_load / ir_compile / render / encode` sub-stage
      lines (plus `graph1 / graph2 / manifest / style_load / swf_load_measure`).
- [ ] **Parallel run** (production wall + decomposed phase), same command WITHOUT
      `RAYON_NUM_THREADS=1`, into `clipper_profile_par`, tee to
      `clipper_ui_timing_par.log`. Expected: one
      `[timing][decomposed] interior_ui_bindings=…s` line + the full export wall.
- [ ] Aggregate the serial `[timing][ui]` sub-stage numbers into a per-stage
      total table (sum each label across all bindings; compute % of serial wall).
- [ ] Pull the per-stage rows for the three canonical render paths:
      one MFD = `Screen_Left_Upper_RTT`, one HUD = `Screen_Central_Compass`,
      one medical = the `i_med_medicalbed_a` binding (grep the helper name in
      the log; if the medical helper name differs, use the `i_med_*` binding).
- [ ] Append this block to the baseline doc (mirror the existing B0 format):

      ```markdown
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

      ### Aggregate stage breakdown (all bindings, serial)

      | Stage | Total (s) | % of wall |
      |---|---|---|
      | `ir_compile` | … | … |
      | `render` | … | … |
      | `swf_load` | … | … |
      | `swf_load_measure` | … | … |
      | `graph2` | … | … |
      | `manifest` | … | … |
      | `encode` | … | … |
      | **Total serial wall** | … | 100% |

      Parallel run: `interior_ui_bindings` = …s; full export wall = …s.

      ### Per-binding (three render paths)

      | Binding | Kind | total | swf_load | render | ir_compile |
      |---|---|---|---|---|---|
      | Screen_Left_Upper_RTT | mfd | … | … | … | … |
      | Screen_Central_Compass | hud | … | … | … | … |
      | i_med_medicalbed_a | medical | … | … | … | … |

      ### Item-7 GATE VERDICT
      Rank the aggregate stages by serial time. **GATED-IN** if `swf_load`
      (+ any stage-parse cost surfaced inside `render`/`swf_load_measure`) is in
      the top 3 stages by total time; otherwise **GATED-OUT**.
      Verdict: **GATED-{IN|OUT}** — <one sentence citing the ranked numbers>.
      ```

- [ ] Copy the same GATE VERDICT sentence into a new ledger entry
      (`StarBreaker/docs/optimisation-ledger.md`): Observed (fresh per-stage
      table), Finding (swf_load rank post-zlib-rs), Action (Task 3 proceeds / is
      dropped). If GATED-OUT, Task 3 is NOT executed but stays in this plan.
- [ ] `rm -rf ships_perf_bench/clipper_profile_serial ships_perf_bench/clipper_profile_par`
      (scratch roots; keep the logs in scratchpad).
- [ ] **Commit** (docs only — this is not a code change; the profile block +
      ledger entry): `git add docs/StarBreaker/starbreaker-ui-perf-baseline.md docs/optimisation-ledger.md && git commit -m "ui perf: post-zlib-rs step-0 re-profile + item-7 gate verdict"`

**Note:** the `[timing][ui]` stage labels are emitted by
`pipeline/timing.rs::timed` (wrapped in `pipeline/mod.rs` / `ui_pipeline.rs`);
labels are stable (`fetch/resolve/swf_load/swf_load_measure/manifest/ir_compile/render/encode/graph1/graph2/style_load`).
No source change needed — instrumentation already exists (B0, commit `e2a2a2892`).

---

## Task 2 — Item 6 colour.rs opaque fast path (D3) — byte-identical

**File:** `crates/starbreaker-ui/src/colour.rs`.

**What:** in `blend_premul_linear` (:72) and `blend_premul_add_linear` (:100),
hoist two `bool`s (`src[3]==255`, `dst[3]==255`) and, per channel, substitute the
LUT decode `u8_to_linear(_)` for the `powf` un-premultiply decode on whichever
side is opaque. Bit-exact because when `a==255`, `sa/da == 1.0`, the divide is
identity, the `.clamp(0,1)` is a no-op (byte/255 ≤ 1), and
`srgb_channel_to_linear(v/255.0) == u8_to_linear(v)` for all `v` (proven by the
existing `lut_matches_powf_helper_for_all_bytes` test). Encode stays `powf`.
**No opaque copy short-circuit** (encode∘decode is not provably identity).

TDD note: this is behaviour-**preserving**, so the exhaustive test asserts
fast == slow (it passes on landing; it is a refactor guard, not a red test).
Keep verbatim copies of the CURRENT bodies as `#[cfg(test)]` reference fns.

Steps:

- [ ] Add the two `bool` hoists + per-side branch in `blend_premul_linear`:

      ```rust
      pub(crate) fn blend_premul_linear(dst: &mut [u8; 4], src: [u8; 4]) {
          let sa = src[3] as f32 / 255.0;
          if sa <= 0.0 {
              return;
          }
          let da = dst[3] as f32 / 255.0;
          let out_a = sa + da * (1.0 - sa);
          if out_a <= 0.0 {
              return;
          }
          let src_opaque = src[3] == 255;
          let dst_opaque = dst[3] == 255;
          for c in 0..3 {
              let s_lin = if src_opaque {
                  u8_to_linear(src[c])
              } else {
                  srgb_channel_to_linear(((src[c] as f32 / 255.0) / sa).clamp(0.0, 1.0))
              };
              let d_lin = if dst_opaque {
                  u8_to_linear(dst[c])
              } else if da > 0.0 {
                  srgb_channel_to_linear(((dst[c] as f32 / 255.0) / da).clamp(0.0, 1.0))
              } else {
                  0.0
              };
              let out_lin = (s_lin * sa + d_lin * da * (1.0 - sa)) / out_a;
              dst[c] = (linear_channel_to_srgb(out_lin.clamp(0.0, 1.0)) * out_a * 255.0)
                  .round()
                  .clamp(0.0, 255.0) as u8;
          }
          dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
      }
      ```

- [ ] Same treatment in `blend_premul_add_linear` (the `s_pl`/`d_pl` decodes).
      When opaque, drop the `* sa` / `* da` — the factor is exactly `1.0`
      (`255/255`) so `x * 1.0 == x` in IEEE754, bit-exact:

      ```rust
      pub(crate) fn blend_premul_add_linear(dst: &mut [u8; 4], src: [u8; 4]) {
          let sa = src[3] as f32 / 255.0;
          let da = dst[3] as f32 / 255.0;
          let out_a = (sa + da).min(1.0);
          if out_a <= 0.0 {
              return;
          }
          let src_opaque = src[3] == 255;
          let dst_opaque = dst[3] == 255;
          for c in 0..3 {
              let s_pl = if src_opaque {
                  u8_to_linear(src[c])
              } else if sa > 0.0 {
                  srgb_channel_to_linear(((src[c] as f32 / 255.0) / sa).clamp(0.0, 1.0)) * sa
              } else {
                  0.0
              };
              let d_pl = if dst_opaque {
                  u8_to_linear(dst[c])
              } else if da > 0.0 {
                  srgb_channel_to_linear(((dst[c] as f32 / 255.0) / da).clamp(0.0, 1.0)) * da
              } else {
                  0.0
              };
              let out_pl = (s_pl + d_pl).min(out_a);
              let out_straight = (out_pl / out_a).clamp(0.0, 1.0);
              dst[c] = (linear_channel_to_srgb(out_straight) * out_a * 255.0)
                  .round()
                  .clamp(0.0, 255.0) as u8;
          }
          dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
      }
      ```

- [ ] Add to the `#[cfg(test)] mod tests` in colour.rs: verbatim reference
      copies of the PRE-CHANGE bodies as `blend_premul_linear_slow` and
      `blend_premul_add_linear_slow` (all-`powf` decode path, no fast branch),
      then the exhaustive equivalence test:

      ```rust
      #[test]
      fn fast_path_matches_slow_over_all_bytes_and_boundaries() {
          // sa/da include 255 (fast branch) AND non-255 (slow branch) so the
          // refactor is proven a no-op EVERYWHERE, not just on the boundary.
          for v in 0u16..=255 {
              let v = v as u8;
              for &sa in &[255u8, 200, 128, 1, 0] {
                  for &da in &[255u8, 200, 128, 1, 0] {
                      let src = [v, 255 - v, v / 2, sa];
                      let base = [255 - v, v, 200u8.wrapping_sub(v), da];

                      let mut fast = base;
                      blend_premul_linear(&mut fast, src);
                      let mut slow = base;
                      blend_premul_linear_slow(&mut slow, src);
                      assert_eq!(fast, slow, "premul mismatch v={v} sa={sa} da={da}");

                      let mut fast_a = base;
                      blend_premul_add_linear(&mut fast_a, src);
                      let mut slow_a = base;
                      blend_premul_add_linear_slow(&mut slow_a, src);
                      assert_eq!(fast_a, slow_a, "premul_add mismatch v={v} sa={sa} da={da}");
                  }
              }
          }
      }
      ```

- [ ] Build + unit test: `cargo test -p starbreaker-ui colour`.
      Expected: new test + `lut_matches_powf_helper_for_all_bytes` +
      `blend_premul_*` tests all green.
- [ ] Full crate test: `cargo test -p starbreaker-ui`. Expected: green,
      **freeze SHAs unchanged** (manifest_visual_regression, regression_hashes,
      line_count_guard).
- [ ] Byte-oracle export (from `/home/tom/projects/scorg_tools`):
      ```bash
      cargo build --release -p starbreaker --manifest-path StarBreaker/Cargo.toml
      mkdir -p ships_perf_bench/clipper_item6
      StarBreaker/target/release/starbreaker entity export drak_clipper \
        ships_perf_bench/clipper_item6 --kind decomposed --lod 0 --mip 0 --materials all
      diff -rq ships_perf_bench/clipper_pre1 ships_perf_bench/clipper_item6 | grep -v export_stamp
      for f in ships_perf_bench/clipper_pre1/Data/UI/Generated/ship/drak/Clipper/*.png; do
        b=$(basename "$f"); cmp -s "$f" "ships_perf_bench/clipper_item6/Data/UI/Generated/ship/drak/Clipper/$b" || echo "CHANGED: $b";
      done
      ```
      Expected: `diff` prints nothing (only export_stamp filtered); zero
      `CHANGED:` lines. Any diff/CHANGED = bug, stop and fix.
- [ ] `rm -rf ships_perf_bench/clipper_item6`.
- [ ] Ledger entry (Observed: linear blend does 2 powf decodes/channel on
      opaque pixels; Finding: opaque decode == LUT bit-exact; Action: LUT
      fast path on both premul fns, byte-identical, exhaustive test).
- [ ] **Commit:** `git commit -am "ui colour: LUT fast path for opaque premul blends (byte-identical)"`

Marker: **order-bound after Task 1** (capture the profile row first); content
is independent of Task 1's verdict.

---

## Task 3 — Item 7 SWF parse-once (D4) — CONDITIONAL on Task 1 = GATED-IN

**Files:** `crates/starbreaker-ui/src/swf_assets/{extract.rs, stage.rs, library.rs}`.

**Execute only if Task 1's GATE VERDICT = GATED-IN.** If GATED-OUT, skip
implementation and record "item 7 gated out — see baseline verdict" in the
ledger. Plan is fully specified either way. One bisectable commit.

**Root problem:** `SwfAssetLibrary::new` runs 8 extractors, each a full
`decompress_swf` + `parse_swf` (via `with_parsed_swf!`); `stage_frame`/
`stage_size`/`stage_visual_bounds` re-parse `self.raw` per call (per flash
node); `merge_swf_bytes` parses 5 more times. Parse once; cache stage data;
drop `raw`.

### 3a. Lifetime reality — REPORTABLE deviation from the literal D4/B3a wording

`swf::parse_swf(&buf) -> ParseResult { tags: Vec<Tag<'a>> }` where `Tag<'a>`
**borrows** the decompressed `SwfBuf`. A helper `parse_tags(bytes) -> Vec<Tag>`
(as literally written in D4 / B3a) is **not expressible** — the tags cannot
outlive `buf` without a self-referential struct. Resolution (achieves the same
"one decompress+parse per construction" goal): `parse_tags` returns the owned
`SwfBuf`; each caller does `let buf = parse_tags(bytes)?; let parsed =
swf::parse_swf(&buf)?;` then runs all `*_from_tags(&parsed.tags)` within that
scope. This is the intended shape; flag it in the ledger, do not "fix" it by
converting Tags to a fully-owned model (large, unnecessary).

### 3b. `extract.rs` changes

- [ ] Add the single choke helper (with a test-only parse counter):

      ```rust
      #[cfg(test)]
      pub(crate) static SWF_PARSE_COUNT: std::sync::atomic::AtomicU64 =
          std::sync::atomic::AtomicU64::new(0);

      /// Decompress a SWF exactly once. Returns the owned buffer; the caller
      /// calls `swf::parse_swf(&buf)` to borrow its tags (which cannot cross this
      /// fn boundary — `Tag<'a>` borrows `buf`). Single choke point so tests can
      /// assert one decompress+parse per library construction.
      pub(crate) fn parse_tags(bytes: &[u8]) -> Result<swf::SwfBuf, UiError> {
          #[cfg(test)]
          SWF_PARSE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
          Ok(swf::decompress_swf(std::io::Cursor::new(bytes))?)
      }
      ```

- [ ] Delete the `with_parsed_swf!` macro. Convert each extractor to a
      `*_from_tags(tags: &[Tag]) -> …` fn holding the body currently inside the
      macro, and keep a thin `extract_*(bytes)` wrapper:
      ```rust
      pub fn extract_bitmaps(swf_bytes: &[u8]) -> Result<HashMap<CharacterId, RgbaImage>, UiError> {
          let buf = parse_tags(swf_bytes)?;
          let parsed = swf::parse_swf(&buf)?;
          extract_bitmaps_from_tags(&parsed.tags)
      }
      pub(crate) fn extract_bitmaps_from_tags(tags: &[Tag]) -> Result<HashMap<CharacterId, RgbaImage>, UiError> { /* current body */ }
      ```
      Convert all seven: `extract_bitmaps`, `extract_shapes`, `extract_fonts`,
      `extract_exported_symbols`, `extract_edit_text_records`,
      `extract_main_timeline_labels`, `extract_font_edit_text_metrics`.
- [ ] **Two-pass preservation** (the `*_from_tags` bodies iterate the SAME
      `tags` slice, so order is preserved automatically — do not reorder):
      - `extract_fonts_from_tags`: keep pass 1 (`DefineFont/Font2/Font4` build)
        then pass 2 (`DefineFontInfo` → `out.get_mut` mutate). Both over `tags`.
      - `extract_font_edit_text_metrics_from_tags`: keep pass 1
        (`ImportAssets` → `imported_font_symbols`) then pass 2
        (`DefineEditText`). Both over `tags`.
      - `extract_main_timeline_labels_from_tags`: keep the single-pass
        `ShowFrame` counter incrementing `current_frame`.

### 3c. `stage.rs` changes — cache all stage frames + size, no per-call re-parse

- [ ] Add `extract_all_sprite_first_frames_from_tags(tags: &[Tag]) ->
      HashMap<CharacterId, Vec<PlaceRecord>>` (body of the current
      `extract_all_sprite_first_frames` minus the decompress/parse). Keep the
      `extract_all_sprite_first_frames(bytes)` wrapper for the mod re-export.
- [ ] Add `extract_all_stage_frames_from_tags(tags: &[Tag]) ->
      (Vec<Vec<PlaceRecord>>, Vec<PlaceRecord>)` returning `(snapshots, tail)`.
      **Byte-exactly** reproduce `extract_stage_frame` semantics: walk `tags`
      maintaining `depth_map`; on each `ShowFrame` push a depth-sorted snapshot
      of the current `depth_map`; process `PlaceObject`/`RemoveObject` exactly
      as today (same `previous`/`Modify`/`Replace` logic, `Matrix::IDENTITY`
      fallback); `tail` = final depth-sorted `depth_map` after all tags.
      Then `stage_frame(N)` returns `snapshots[N]` if `N < snapshots.len()`
      else `tail`. (Verified equivalent to the current break-on-`==`/`>` loop:
      snapshot[N] = cumulative display list just before the (N+1)-th ShowFrame;
      N ≥ ShowFrame count returns the post-last-ShowFrame tail. Empty timeline →
      no snapshots → always `tail`.)
- [ ] Keep `extract_stage_frame(bytes, idx)`, `extract_stage_size(bytes)`,
      `extract_sprite_first_frame(bytes, id)` as-is (still `pub`, re-exported in
      `swf_assets/mod.rs:16`, used by examples/tests). They are no longer called
      from `library.rs`.

### 3d. `library.rs` changes — struct fields + parse-once `new` + `merge`

- [ ] Struct field changes on `SwfAssetLibrary`:
      - **Remove** `raw: Vec<u8>` (nothing reads it once stage data is cached;
        `merge_swf_bytes` takes EXTERNAL bytes; confirmed grep: `self.raw` used
        only by `stage_frame`/`stage_size`). **raw-drop decision: DROP.**
      - **Add** `stage_size: (f32, f32)`.
      - **Add** `stage_frames: Vec<Vec<PlaceRecord>>` (per-frame snapshots).
      - **Add** `stage_frames_tail: Vec<PlaceRecord>`.
- [ ] Rewrite `SwfAssetLibrary::new`:
      ```rust
      pub fn new(swf_bytes: Vec<u8>) -> Result<Self, UiError> {
          let content_hash = { /* Sha256 over &swf_bytes, unchanged */ };
          let buf = parse_tags(&swf_bytes)?;
          let parsed = swf::parse_swf(&buf)?;
          let tags = &parsed.tags;

          let bitmaps = extract_bitmaps_from_tags(tags)?;
          let shapes = extract_shapes_from_tags(tags)?;
          let fonts = extract_fonts_from_tags(tags)?;
          let exports = extract_exported_symbols_from_tags(tags)?;
          let edit_texts = extract_edit_text_records_from_tags(tags)?;
          let font_edit_text_metrics = extract_font_edit_text_metrics_from_tags(tags)?;
          let frame_labels = extract_main_timeline_labels_from_tags(tags)?;
          let sprite_first_frames = extract_all_sprite_first_frames_from_tags(tags);

          let stage_size = {
              let r = buf.header.stage_size();
              (
                  (r.x_max - r.x_min).to_pixels() as f32,
                  (r.y_max - r.y_min).to_pixels() as f32,
              )
          };
          let (stage_frames, stage_frames_tail) = extract_all_stage_frames_from_tags(tags);
          // buf + parsed dropped here; `raw` no longer stored.

          Ok(Self { content_hash, bitmaps, shapes, fonts, exports, edit_texts,
              font_edit_text_metrics, frame_labels, sprite_first_frames,
              stage_size, stage_frames, stage_frames_tail })
      }
      ```
      NOTE: `buf.header.stage_size()` returns the same `Rectangle` the old
      `extract_stage_size` read from `ParseResult.header`; verify identical `(w,h)`
      against the pre-change `extract_stage_size(&swf_bytes)` in the parse-count
      test below.
- [ ] Rewrite `merge_swf_bytes` to parse once (5 parses → 1):
      ```rust
      pub fn merge_swf_bytes(&mut self, swf_bytes: &[u8]) -> Result<(), UiError> {
          let buf = parse_tags(swf_bytes)?;
          let parsed = swf::parse_swf(&buf)?;
          let tags = &parsed.tags;
          self.bitmaps.extend(extract_bitmaps_from_tags(tags)?);
          self.shapes.extend(extract_shapes_from_tags(tags)?);
          self.fonts.extend(extract_fonts_from_tags(tags)?);
          for (symbol, metrics) in extract_font_edit_text_metrics_from_tags(tags)? {
              self.font_edit_text_metrics.entry(symbol).or_insert(metrics);
          }
          for (name, id) in extract_exported_symbols_from_tags(tags)? {
              self.exports.entry(name).or_insert(id);
          }
          Ok(())
      }
      ```
      (Merge still updates ONLY bitmaps/shapes/fonts/metrics/exports — does NOT
      touch stage_frames/frame_labels/edit_texts/sprite_first_frames. Preserve
      exactly.)
- [ ] Rewrite the stage accessors to read cached fields (no re-parse):
      ```rust
      pub fn stage_frame(&self, frame_index: u32) -> Vec<PlaceRecord> {
          self.stage_frames
              .get(frame_index as usize)
              .cloned()
              .unwrap_or_else(|| self.stage_frames_tail.clone())
      }
      pub fn stage_size(&self) -> (f32, f32) { self.stage_size }
      ```
      `stage_visual_bounds` is unchanged (it calls `self.stage_frame`).
- [ ] **Do NOT regress** `find_font_by_name` (CharacterId-sorted determinism
      fix, library.rs:107-108) — leave untouched.

### 3e. Parse-count test

- [ ] In `swf_assets/tests.rs` (the existing `library_content_hash_is_stable`
      at :210-216 builds two libraries from the same bytes — colocate here):
      ```rust
      #[test]
      fn library_construction_parses_once() {
          use crate::swf_assets::extract::SWF_PARSE_COUNT;
          use std::sync::atomic::Ordering;
          let bytes = make_minimal_swf();
          let before = SWF_PARSE_COUNT.load(Ordering::Relaxed);
          let lib = SwfAssetLibrary::new(bytes.clone()).expect("library");
          let after = SWF_PARSE_COUNT.load(Ordering::Relaxed);
          assert_eq!(after - before, 1, "SwfAssetLibrary::new must decompress+parse exactly once");
          // stage size matches the pre-refactor standalone extractor exactly.
          assert_eq!(lib.stage_size(), extract_stage_size(&bytes));
      }
      ```
      (Import path for `SWF_PARSE_COUNT` may need a `pub(crate) use` re-export in
      `swf_assets/mod.rs` — add if the test can't reach it. The counter is
      global/`Relaxed`, so the test measures a delta, not an absolute, to stay
      correct under parallel test execution.)

### 3f. Full B-VAL verification loop

- [ ] `cargo build -p starbreaker-ui -p starbreaker-3d` (debug). Expected: clean
      (watch for unused-import warnings from the deleted macro / dropped `raw`).
- [ ] `cargo test -p starbreaker-ui`. Expected: green, **freeze SHAs
      unchanged**. Specifically confirm: `swf_display_list`, `swf_state_selection`,
      `swf_rendering_fixtures`, `swf_phase5_wiring`, `swf_edittext_render`,
      `swf_color_transform`, `pipeline_mfd_frame`, `manifest_visual_regression`,
      `regression_hashes`, `line_count_guard`, and the new `library_construction_parses_once`.
- [ ] `cargo test -p starbreaker-3d`. Expected: green.
- [ ] Byte-oracle export (from `/home/tom/projects/scorg_tools`):
      ```bash
      cargo build --release -p starbreaker --manifest-path StarBreaker/Cargo.toml
      mkdir -p ships_perf_bench/clipper_item7
      StarBreaker/target/release/starbreaker entity export drak_clipper \
        ships_perf_bench/clipper_item7 --kind decomposed --lod 0 --mip 0 --materials all
      diff -rq ships_perf_bench/clipper_pre1 ships_perf_bench/clipper_item7 | grep -v export_stamp
      for f in ships_perf_bench/clipper_pre1/Data/UI/Generated/ship/drak/Clipper/*.png; do
        b=$(basename "$f"); cmp -s "$f" "ships_perf_bench/clipper_item7/Data/UI/Generated/ship/drak/Clipper/$b" || echo "CHANGED: $b";
      done
      ```
      Expected: `diff` empty (export_stamp filtered); zero `CHANGED:` lines.
- [ ] **Two-run determinism** (item 7 touches the SWF/font path): export a
      second time into `clipper_item7b`, then
      `diff -rq ships_perf_bench/clipper_item7 ships_perf_bench/clipper_item7b | grep -v export_stamp`
      → empty. (Guards against any re-introduced HashMap-order non-determinism.)
- [ ] Capture a fresh serial `SB_UI_TIMING=1 RAYON_NUM_THREADS=1` timing row and
      append it under the Task-1 baseline block (swf_load before/after).
- [ ] `rm -rf ships_perf_bench/clipper_item7 ships_perf_bench/clipper_item7b`.
- [ ] Ledger entry (Observed: SWF decompress+parse ~20×/image; Finding:
      parse-once + cached stage frames removes all re-parses, byte-identical +
      deterministic; Action: `parse_tags` choke + `*_from_tags` + cached
      `stage_*` fields, `raw` dropped; note the `parse_tags -> SwfBuf` lifetime
      deviation from the literal B3a wording).
- [ ] **Commit:** `git commit -am "ui swf: parse each SWF once, cache stage frames (byte-identical)"`

Marker: **conditional** (GATED-IN only) and **order-bound after Task 2**.

---

## Out of scope (this workstream)

- B2e "build library once per image" (D4: "only if it falls out naturally" —
  it does not; parse-once already removes the per-call cost). Do NOT restructure
  `render_for_binding_ir`/`compile_ir_for_binding` build sites (mod.rs:540/595/750).
- B3b single-graph-resolve, B4a/B4b caches, parallelism, code-size (`.part`
  scheme), tiny-skia dedup — all SKIP / other windows per the handoff.
- The three per-image `SwfAssetLibrary::new` build sites benefit for free from
  parse-once; leave the call sites unchanged.
# Plan — Part C: addon / ops / spikes workstream (perf handoff 2026-07-18)

Workstream = D7, D8, D9, D10, D11, D13, D14 (design doc
`StarBreaker/docs/superpowers/plans/2026-07-18-perf-handoff-design.md`).
This is Phase F of the implementation order. Planner-only; every step is a
bite-sized checkbox for the implementer.

## Global constraints (travel to every task)

- Commit directly on `feature/ui`; no branches/worktrees; no commit trailers;
  never name the owner.
- Addon tests (INDEPENDENT of cargo, do not touch `target/`):
  `cd StarBreaker/blender_addon && python3 -m unittest discover -s tests -q`
  (SYSTEM `python3`, NOT uv — 370 tests, ~0.4 s).
- Other Python one-liners/scripts: `uv run python` (item 9 pixel oracle only).
- **ORDER-BOUND** = touches cargo/`target/` → serialise behind the
  orchestrator's Rust phases (shared `target/`, no concurrent builds). Tagged
  per task below. Benchmarks need `uptime` load < 2 and the window before
  ~07:00.
- Ledger = `StarBreaker/docs/optimisation-ledger.md`, Observed/Finding/Action.
  One commit per coherent item.
- No hard-coded game-data values; root-cause fixes; every new `.rs` gets a
  `//!` header (none created here).

## Task independence map

| Task | Item | Cargo? | Independent? |
|------|------|--------|--------------|
| C1 resolve_path lazy index | 3 (D8) | no | INDEPENDENT |
| C2 POM bias slice | 4 (D9) | no | INDEPENDENT |
| C3 graphify prune + AGENTS.md note + dcb_canvas report | 5 (D10) | no | INDEPENDENT |
| C4 ledger: 12.3 rejection | 12.3 (D7) | no | INDEPENDENT |
| C5 ledger: item-13 scrub note | 13 (D14) | no | INDEPENDENT |
| C6 PNG fast-encode spike (measure, DO NOT LAND) | 9 (D11) | YES | ORDER-BOUND |
| C7 mcp tokio trim + redeploy | 12.4 (D13) | YES | ORDER-BOUND |

C1–C5 can run any time (no cargo). C6, C7 wait for a cargo slot.

---

## C1 — Item 3 / D8: `resolve_path` one-loop lazy path-index (INDEPENDENT, TDD)

File: `StarBreaker/blender_addon/starbreaker_addon/manifest.py`
(`PackageBundle.resolve_path`, currently :854–863). `_build_path_index`
(:880) memoizes `self._path_index` (default `None`, :797).

### Steps

- [ ] **Write the failing test first.** Append to
  `StarBreaker/blender_addon/tests/test_manifest.py` a new test method on
  `ManifestTests` (NOT `@_requires_argo_fixture` — must run unconditionally):

```python
    def test_direct_hit_does_not_build_path_index(self) -> None:
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Data" / "Objects").mkdir(parents=True)
            target = root / "Data" / "Objects" / "thing_LOD0.glb"
            target.write_bytes(b"glb")
            bundle = PackageBundle(
                export_root=root,
                scene_path=root / "Packages" / "X" / "scene.json",
                scene=None,
                palettes={},
                liveries={},
                paints={},
            )
            resolved = bundle.resolve_path("Data/Objects/thing_LOD0.glb")
            self.assertEqual(resolved, target)
            self.assertIsNone(bundle._path_index)
```

  (`PackageBundle` is a plain dataclass, manifest.py:788; `scene=None` is fine
  — `resolve_path` never touches `scene`.)

- [ ] **Verify it FAILS pre-fix** (current code calls `_build_path_index()`
  unconditionally → `_path_index` becomes `{}`, so `assertIsNone` fails):
  ```bash
  cd StarBreaker/blender_addon && python3 -m unittest -q \
    tests.test_manifest.ManifestTests.test_direct_hit_does_not_build_path_index
  ```
  Expected: `FAILED (failures=1)` with `AssertionError: {...} is not None`.

- [ ] **Apply the fix.** Replace `resolve_path` (manifest.py:854–863):

```python
    def resolve_path(self, relative_path: str | None) -> Path | None:
        path_index = None
        for candidate in _candidate_relative_paths(relative_path):
            direct = self.export_root / Path(candidate)
            if direct.exists():
                return direct
            if path_index is None:
                path_index = self._build_path_index()
            resolved = path_index.get(candidate.lower())
            if resolved is not None:
                return resolved
        return None
```

  Precedence is byte-for-byte the old order (per candidate: direct, then index;
  candidate 1 fully before candidate 2). The index builds only on the first
  direct miss; a first-candidate direct hit returns before any rglob, leaving
  `_path_index is None`.

- [ ] **Verify the test now PASSES + full suite green:**
  ```bash
  cd StarBreaker/blender_addon && python3 -m unittest discover -s tests -q
  ```
  Expected tail: `OK` (or `OK (skipped=N)`), 0 failures, 0 errors.

- [ ] **Ledger** — append under a new `### Addon: resolve_path lazy index`:
  > **Observed** — `PackageBundle.resolve_path` rglob'd the whole shared
  > `ships/` root (~9,150 files, grows per ship) on every call via an
  > unconditional `_build_path_index()`, though manifests store normalised
  > paths that almost always hit the direct `export_root/candidate` check
  > (bottleneck #1, `docs/blender-import-export-performance.md`, never applied).
  > **Finding** — the walk is pure waste on the direct-hit path; the index is
  > only needed as a case-insensitive fallback after all direct candidates miss.
  > **Action** — one-loop lazy init: `path_index=None`, per candidate try
  > `direct.exists()` then lazily build+consult the index on first miss;
  > precedence unchanged. Regression `test_direct_hit_does_not_build_path_index`
  > asserts `_path_index is None` after a direct hit. (commit `<hash>`)

- [ ] **Commit** (message):
  ```
  addon: resolve_path skips rglob index on direct hit

  Build the path index lazily on the first direct-candidate miss instead of
  unconditionally rglob'ing the shared ships/ root every call. Per-candidate
  direct-then-index precedence unchanged; regression asserts the index stays
  unbuilt on a direct hit.
  ```

---

## C2 — Item 4 / D9: POM bias `pixels[:]` → `pixels[0:4]` (INDEPENDENT, TDD)

File: `StarBreaker/blender_addon/starbreaker_addon/runtime/importer/builders.py`,
`_height_image_background_bias` (:292). Two whole-buffer copies to read one
pixel: `:326 bias = _luma(tmp.pixels[:])`, `:333 bias = _luma(image.pixels[:])`
(`_luma` reads `buf[0:3]` after a `len(buf) < 4` guard). `orchestration.py:1247`
is a legitimate full-image loop — DO NOT touch.

### Steps

- [ ] **Write the failing test first.** Append to
  `StarBreaker/blender_addon/tests/test_layers.py` (it already imports
  `builders` under a working bpy/mathutils/numpy stub, :47–68). Add the import
  and a test class:

```python
from starbreaker_addon.runtime.importer.builders import _height_image_background_bias


class PomBiasSliceTests(unittest.TestCase):
    class _SliceGuardImage:
        """pixels[:] raises; only pixels[0:4] is allowed — proves the fix
        reads four floats, not the whole buffer."""

        def __init__(self, head):
            self._head = list(head)  # four RGBA floats

        @property
        def pixels(self):
            return self

        def __getitem__(self, key):
            if key == slice(0, 4):
                return self._head
            raise AssertionError(
                f"POM bias must read pixels[0:4], not {key!r}"
            )

        def get(self, key):  # cache lookup at top of the fn
            return None

        def __setitem__(self, key, value):  # cache write at the end
            pass

    def test_fallback_reads_only_first_pixel(self) -> None:
        # No .filepath → the temp-load branch is skipped, exercising the
        # image.pixels fallback (builders.py:333).
        image = self._SliceGuardImage([0.2, 0.4, 0.6, 1.0])
        bias = _height_image_background_bias(image)
        self.assertIsNotNone(bias)
        self.assertAlmostEqual(
            bias, 0.299 * 0.2 + 0.587 * 0.4 + 0.114 * 0.6, places=6
        )
```

  Note: `getattr(image, "filepath", "") or ""` → `""` (falsy) → the
  `bpy.data.images.load` block is skipped entirely, so no bpy calls are made on
  this path; only `image.pixels[...]`, `image.get`, `image[...] =` are touched.

- [ ] **Verify it FAILS pre-fix** (`tmp.pixels[:]`/`image.pixels[:]` → the guard
  raises `AssertionError`; the fallback line `image.pixels[:]` triggers it):
  ```bash
  cd StarBreaker/blender_addon && python3 -m unittest -q \
    tests.test_layers.PomBiasSliceTests.test_fallback_reads_only_first_pixel
  ```
  Expected: `FAILED` with `AssertionError: POM bias must read pixels[0:4], not slice(None, None, None)`.

- [ ] **Apply the fix** in builders.py:
  - `:326` `bias = _luma(tmp.pixels[:])` → `bias = _luma(tmp.pixels[0:4])`
  - `:333` `bias = _luma(image.pixels[:])` → `bias = _luma(image.pixels[0:4])`

- [ ] **Verify PASS + full suite green:**
  ```bash
  cd StarBreaker/blender_addon && python3 -m unittest discover -s tests -q
  ```
  Expected: `OK` / 0 failures.

- [ ] **Ledger** — append `### Addon: POM bias reads one pixel not the atlas`:
  > **Observed** — `_height_image_background_bias` copied the entire height
  > image (`pixels[:]`, ~500 MB transient for 2048² RGBA, twice) to read the
  > top-left pixel; `_luma` only touches `buf[0:3]`. The import has OOM'd at
  > 12 GB+ (dead-end 2). **Finding** — `pixels[:]` materialises the whole
  > buffer; a bounded `pixels[0:4]` slice yields the same four floats.
  > **Action** — `pixels[:]` → `pixels[0:4]` at both sites (temp-load :326 and
  > fallback :333); `orchestration.py:1247` (a real full-image loop) untouched.
  > Behaviour-identical; `PomBiasSliceTests` guards the slice bound.
  > (commit `<hash>`)

- [ ] **Commit** (message):
  ```
  addon: POM bias reads pixels[0:4] instead of copying the whole image

  _height_image_background_bias only needs the top-left RGBA pixel; the
  pixels[:] full-buffer copy allocated ~500 MB per 2048^2 height image. Slice
  to pixels[0:4] at both the temp-load and fallback sites. Value-identical;
  orchestration.py's legitimate full-image loop is untouched.
  ```

C1 and C2 may share one addon-test run and be committed separately (one commit
per item). Sequence: land C1, then C2 (both edit different files).

---

## C3 — Item 5 / D10: disk retention (INDEPENDENT — pure fs + docs)

Handoff numbers are STALE. Verified 2026-07-18: `target/debug` does not exist;
disk ~83 %/187 G free; `ships/dcb_canvas` (3.0 G) IS read by
`crates/starbreaker-ui/tests/live_ir_harness/mod.rs:253` (skips gracefully if
absent) → DO NOT DELETE. Only real reclaim now = prune old `graphify-out`
snapshots (14 dated dirs `2026-06-16`…`2026-07-08`, ~13–17 M each).

### Steps

- [ ] **Confirm the current snapshot set** (sanity before deleting):
  ```bash
  cd /home/tom/projects/scorg_tools/StarBreaker/graphify-out && ls -d 20*/
  ```
  Expected: 14 dirs `2026-06-16/` … `2026-07-08/`. Live artifacts
  `graph.json`, `cache/`, `cost.json`, `manifest.json` do NOT match `20*/`.

- [ ] **Prune to the 3 newest dated snapshots + live graph.json:**
  ```bash
  cd /home/tom/projects/scorg_tools/StarBreaker/graphify-out && \
    ls -d 20*/ | sort | head -n -3 | xargs -r rm -rf
  ```
  (`sort` ascending; `head -n -3` drops the newest 3; ISO dir names sort
  chronologically.) Keeps `2026-07-06/ 2026-07-07/ 2026-07-08/`.

- [ ] **Verify:**
  ```bash
  cd /home/tom/projects/scorg_tools/StarBreaker/graphify-out && ls -d 20*/
  ```
  Expected: exactly `2026-07-06/ 2026-07-07/ 2026-07-08/`. Confirm
  `graph.json`, `cache/`, `cost.json`, `manifest.json` still present.

- [ ] **Add the retention note to `StarBreaker/AGENTS.md`.** Insert a new
  subsection immediately AFTER the `## Building` paragraph (which ends
  "…final binaries, CLI re-exports.") and BEFORE `## Coding Practices`:

```markdown
### Disk retention policy

`target/` and generated caches regrow; keep them bounded without a blind
`cargo clean` (the old hashed release binaries under `target/release/deps`
are the time-travel-bisect ladder — see `docs/optimisation-ledger.md` item 4):

- **Debug tree:** if a `target/debug` tree regrows, run `cargo sweep --time 30`
  from the repo root (install `cargo-sweep` first). Never blind-`clean`.
- **`release/deps` bisect ladder (~1.2 G):** leave it — it is the bisect
  mechanism recorded in the optimisation ledger.
- **`graphify-out/`:** keep the live `graph.json` (plus `cache/`, `cost.json`,
  `manifest.json`) and the 3 newest dated snapshot dirs; delete older ones:
  `ls -d 20*/ | sort | head -n -3 | xargs -r rm -rf`.
- **`ships/dcb_canvas`:** do NOT delete — the starbreaker-ui live-IR harness
  reads it (`crates/starbreaker-ui/tests/live_ir_harness/mod.rs`), skipping
  gracefully only if it is absent (deleting silently disables that guard).
```

- [ ] **Ledger** — append `### Ops: disk retention (item 5, re-scoped)`:
  > **Observed** — handoff premised a 46–66 G reclaim from `target/debug`
  > (68 G). Verified 2026-07-18: no `target/debug` tree exists; disk ~83 %
  > used / 187 G free — the reclaim target is void. **Finding** — `dcb_canvas`
  > (3.0 G) is read by the starbreaker-ui live-IR harness (mod.rs:253) — the
  > handoff's "nothing reads it" is wrong; deleting it silently disables the
  > guard. Only stale data left = `graphify-out` old snapshots (14 dated dirs).
  > **Action** — pruned `graphify-out` to the 3 newest snapshots + live
  > `graph.json`; added a retention-policy note to `AGENTS.md §Building`
  > (cargo-sweep any regrown debug tree, keep the release-deps ladder, keep
  > `dcb_canvas`). `dcb_canvas` retention surfaced to the owner. (commit `<hash>`)

- [ ] **Commit** (message):
  ```
  ops: prune stale graphify snapshots, document disk retention

  graphify-out kept 14 dated snapshots; trim to the newest 3 plus live
  graph.json. Add a retention policy to AGENTS.md: cargo-sweep any regrown
  debug tree, keep the release/deps bisect ladder, keep ships/dcb_canvas
  (read by the starbreaker-ui live-IR harness). The handoff's target/debug
  reclaim is void — no debug tree exists.
  ```

- [ ] **End-of-run report line (owner):** `ships/dcb_canvas` (3.0 G) was NOT
  deleted — it is a live test dependency of the starbreaker-ui live-IR harness
  (`tests/live_ir_harness/mod.rs:253`), contradicting the handoff. Owner to
  decide if the harness should point elsewhere before any future reclaim.

---

## C4 — Item 12.3 / D7: layout-cache `data_pointer` drop — REJECTED (INDEPENDENT, ledger only)

No code change (design D7 rejected the optimisation on a failed guard).

- [ ] **Ledger** — append `### Addon: layout_key data_pointer drop — REJECTED`:
  > **Observed** — proposed removing `data_pointer` from `layout_key`
  > (`orchestration.py:746–758`) so distinct meshes sharing a slot layout reuse
  > one cache entry (keeping it in `slot_mapping_cache`, :657). **Finding** —
  > guard failed: the layout-cache hit path (`orchestration.py:763–776`) skips
  > `_restore_generated_decal_host_variant_polygons` (`builders.py:2943`,
  > mutates polygons per-mesh) and `_rebind_mesh_decal_for_host`
  > (`builders.py:4849`, nearest-host spatial rebind reading vertex positions),
  > both per-mesh and geometry-dependent, not part of the cache key. With
  > `data_pointer` dropped, two distinct meshes could share an entry and the
  > second mesh's per-mesh fixes would never run — a correctness regression, not
  > a byte-identical win. **Action** — REJECTED, no code change. Would be
  > dischargeable only by proving both per-mesh fixes are no-ops whenever the
  > keyed inputs match (unlikely: the rebind reads live vertex positions).
  > (no commit — ledger record)

- [ ] **Commit** (ledger-only; may be batched with C5 in one "ledger notes"
  commit):
  ```
  ledger: record item 12.3 (layout_key data_pointer) rejection
  ```

---

## C5 — Item 13 / D14: git-history scrub note (INDEPENDENT, ledger only)

No action now (pre-public-release only). The repo has no release-checklist doc
(`scripts/build-release-*.sh` + `.github/workflows/release.yml` only), so per
D14 this lands as a ledger entry.

- [ ] **Ledger** — append `### Release checklist: git history scrub (item 13, DO NOT run now)`:
  > **Observed** — `.git` size-pack ~1.22 GiB; ~821 historical `ships/Data`
  > blob paths (~1,155 MiB of once-committed, now-ignored export artifacts) ship
  > with every public clone. **Finding** — a history rewrite removes them but
  > rewrites all commit hashes — destructive, never mid-arc. **Action (PRE-
  > PUBLIC-RELEASE ONLY, owner-scheduled):** on a fresh clone run
  > `git filter-repo --path ships --invert-paths`; verify the 4 legitimately
  > tracked binaries survive (`material_templates.blend`, `pom_library.blend`,
  > `screen_effects_library.blend`, the app icon); force-push once; re-clone
  > everywhere. Pre-conditions: `.gitignore` covers `ships/`, `dcb_canvas`,
  > `graphify-out`. Verify: `git count-objects -vH` size-pack shrunk ~1.15 GiB;
  > `cargo build` + tests green on the rewritten clone. (no commit — checklist
  > record)

- [ ] **Commit** (batch with C4):
  ```
  ledger: record git-history scrub procedure for the release checklist
  ```

- [ ] **End-of-run report:** restate the scrub procedure + pre-conditions
  (above) as one paragraph for the owner.

---

## C6 — Item 9 / D11: PNG fast-encode SPIKE (ORDER-BOUND, DOES NOT LAND)

Measure only; capture numbers + a patch file; revert the working tree. `image`
0.25.10 already links `png` 0.18.1 — no version bump. **UI PNGs EXCLUDED** (the
starbreaker-ui freeze gate hashes PNG bytes; UI/holo renders go through the gfx
encoders — leave `starbreaker-gfx/src/render.rs:138` and `resolver.rs:231`
UNTOUCHED, see Conflicts §1). Scope = texture-export encoders only.

Encoders to switch (both currently `img.write_to(...ImageFormat::Png)`):
- `crates/starbreaker-3d/src/pipeline/textures.rs::encode_png` (:185)
- `crates/starbreaker-3d/src/pipeline/textures.rs::encode_png_rgba` (:1090)

Oracle: `/home/tom/projects/scorg_tools/ships_perf_bench/clipper_pre1` (788
PNGs, pre-change decoded pixels).

### Steps

- [ ] **Confirm the cargo slot is free** (orchestrator scheduling): no other
  cargo build running; `uptime` load < 2 for the timing run.

- [ ] **Capture a clean pre-change texture-export baseline** into a fresh dir
  (so size/timing deltas are attributable), release build:
  ```bash
  cd /home/tom/projects/scorg_tools/StarBreaker
  RUST_LOG=info ./target/release/starbreaker entity export "drak_clipper" \
    /tmp/claude-1000/-home-tom-projects-scorg-tools/87ac34cf-d805-406e-8cc7-31ca20c9f92f/scratchpad/clipper_png_base \
    --kind decomposed --lod 0 --mip 0 --materials all 2>&1 | tee \
    /tmp/.../scratchpad/item9-base.log
  du -sb .../clipper_png_base --exclude='*.json' 2>/dev/null; \
    find .../clipper_png_base -name '*.png' | wc -l; \
    find .../clipper_png_base -name '*.png' -printf '%s\n' | awk '{s+=$1} END{print s" bytes PNG"}'
  ```
  Record total PNG byte size + PNG count + `[timing][decomposed]` totals.
  (If `target/release/starbreaker` is stale, the orchestrator rebuilds it in its
  slot first — do NOT `cargo build` concurrently.)

- [ ] **Add a temporary encode-wall probe** (spike-only, reverted after) to
  isolate encode time — an `AtomicU64` nanos accumulator around the two
  encoders, logged once at export end in the existing `[timing]` idiom. Check
  first whether `[timing][decomposed]` already breaks out a `png_encode`/
  texture stage (read the export-end timing block in
  `crates/starbreaker-3d/src/decomposed.rs`); if a texture/encode stage line
  already exists, reuse it and skip the probe.

- [ ] **Apply the fast-encode change** to both `textures.rs` encoders. Pattern
  (mirrors `starbreaker-gfx/src/render.rs:138` which already uses `PngEncoder`):

```rust
pub(super) fn encode_png(image: &image::RgbaImage) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut png_buf = Vec::new();
    PngEncoder::new_with_quality(
        std::io::Cursor::new(&mut png_buf),
        CompressionType::Fast,
        FilterType::Adaptive,
    )
    .write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )
    .ok()?;
    Some(png_buf)
}

pub(super) fn encode_png_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let img = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png_buf = Vec::new();
    PngEncoder::new_with_quality(
        std::io::Cursor::new(&mut png_buf),
        CompressionType::Fast,
        FilterType::Adaptive,
    )
    .write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )
    .ok()?;
    Some(png_buf)
}
```
  (Leave `cli/src/dds.rs` — its `img.save`/`dds.save_png` is the standalone
  manual `dds` subcommand, NOT the export pipeline; not on the measured path.
  Note in the report as an optional follow-up, do not switch it in the spike.)

- [ ] **Build (release) in the cargo slot:**
  ```bash
  cargo build --release -p starbreaker-3d
  ```

- [ ] **Re-export to a second fresh dir + capture deltas:**
  ```bash
  RUST_LOG=info ./target/release/starbreaker entity export "drak_clipper" \
    .../scratchpad/clipper_png_fast --kind decomposed --lod 0 --mip 0 \
    --materials all 2>&1 | tee .../scratchpad/item9-fast.log
  find .../clipper_png_fast -name '*.png' -printf '%s\n' | awk '{s+=$1} END{print s" bytes PNG"}'
  ```
  Record fast PNG total size, encode-wall delta, `[timing][decomposed]` total.

- [ ] **Pixel-identity oracle** — decode-and-compare every PNG in the fast
  export against the same relative path in `clipper_pre1` (Pillow is available
  via `uv run python`, confirmed). Script to write to scratchpad and run:

```python
# .../scratchpad/item9_pixel_identity.py
import sys
from pathlib import Path
from PIL import Image

fast = Path(sys.argv[1])          # clipper_png_fast root
oracle = Path("/home/tom/projects/scorg_tools/ships_perf_bench/clipper_pre1")
mismatch = missing = checked = 0
for p in fast.rglob("*.png"):
    rel = p.relative_to(fast)
    o = oracle / rel
    if not o.is_file():
        missing += 1
        continue
    a = Image.open(p).convert("RGBA").tobytes()
    b = Image.open(o).convert("RGBA").tobytes()
    checked += 1
    if a != b:
        mismatch += 1
        print("PIXEL DIFF:", rel)
print(f"checked={checked} mismatch={mismatch} missing_in_oracle={missing}")
sys.exit(1 if mismatch else 0)
```
  ```bash
  uv run python .../scratchpad/item9_pixel_identity.py .../scratchpad/clipper_png_fast
  ```
  Expected: `mismatch=0` (bytes differ by design, decoded pixels must be
  identical). Any `PIXEL DIFF` = the spike is unsound → report and stop.

- [ ] **Capture the patch, then revert the working tree** (spike does NOT land):
  ```bash
  cd /home/tom/projects/scorg_tools/StarBreaker
  git diff > .../scratchpad/item9-png-fast.patch
  git checkout -- crates/starbreaker-3d/src/pipeline/textures.rs
  # also revert the temporary timing probe if added
  cargo build --release -p starbreaker-3d   # restore the byte-identical binary in the slot
  ```

- [ ] **Ledger** — append `### Spike: PNG fast-encode (item 9, report-only, not landed)`:
  > **Observed** — texture-export PNG encode via `image` 0.25 default DEFLATE
  > on `textures.rs::encode_png`/`encode_png_rgba`. **Finding** —
  > `PngEncoder::new_with_quality(Fast, Adaptive)` on a Clipper texture export:
  > encode wall `<before>`→`<after>`, total PNG size `<before>`→`<after>`
  > (`+X %`), decoded pixels byte-identical over `<N>` PNGs (0 mismatch).
  > **Action** — SPIKE ONLY, reverted; patch at
  > `scratchpad/item9-png-fast.patch`. UI/holo PNGs excluded (freeze gate + gfx
  > encoders). Owner decides the size-vs-speed tradeoff. (no commit)

- [ ] **Report the numbers to the owner** (size delta % + encode speedup +
  pixel-identity pass) — this item does not land without an owner call.

---

## C7 — Item 12.4 / D13: mcp tokio trim + redeploy (ORDER-BOUND)

Files: `mcp/Cargo.toml:12`, `mcp/src/main.rs:16`. Only tokio usage is
`#[tokio::main]` (:16) + `.serve(...).await` / `.waiting().await` (:30–31) over
rmcp stdio. `#[tokio::main]` defaults to multi-thread → needs `rt-multi-thread`;
option (b) forces current-thread so `rt` suffices.

### Steps

- [ ] **Confirm the cargo slot is free** (release build follows).

- [ ] **Edit `mcp/Cargo.toml:12`:**
  ```toml
  tokio = { version = "1", features = ["rt", "macros", "io-std"] }
  ```

- [ ] **Edit `mcp/src/main.rs:16`:**
  ```rust
  #[tokio::main(flavor = "current_thread")]
  ```

- [ ] **Build (release):**
  ```bash
  cd /home/tom/projects/scorg_tools/StarBreaker
  cargo build --release -p starbreaker-mcp
  ```
  Expected: clean build. **Fallback:** if the compiler names a missing tokio
  feature (e.g. rmcp's stdio transport needs `io-util`), add exactly that one
  feature to the list and rebuild — do NOT revert to `["full"]`.

- [ ] **Redeploy** per AGENTS.md §MCP (Linux):
  ```bash
  pkill -x starbreaker-mcp || true
  cargo build --release -p starbreaker-mcp && \
    cp target/release/starbreaker-mcp mcp/starbreaker-mcp
  ```

- [ ] **Handshake smoke test** — feed a JSON-RPC `initialize` over stdio and
  expect a `result` (server responds before any data load, per main.rs:24–27):
  ```bash
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
    | timeout 15 ./mcp/starbreaker-mcp 2>/dev/null | head -1
  ```
  Expected: a single JSON line containing `"result"` and `"serverInfo"` /
  `"protocolVersion"`. (If it blocks with no output, the current-thread runtime
  starved a spawned task → add `io-util`/`rt-multi-thread` as the fallback names
  it, rebuild, retest.)

- [ ] **Restart the MCP client** to pick up the new binary (`.mcp.json` points
  at the deployed copy).

- [ ] **Ledger** — append `### mcp: tokio feature trim`:
  > **Observed** — `mcp/Cargo.toml` pulled `tokio` with `features=["full"]`
  > though the only runtime use is a single-task stdio server. **Finding** —
  > `full` compiles the entire tokio surface (net, fs, time, multi-thread
  > scheduler) that the stdio handshake never uses. **Action** —
  > `#[tokio::main(flavor = "current_thread")]` + `features=["rt","macros",
  > "io-std"]` (`+ <extra>` if the build named one); rebuilt + redeployed;
  > handshake smoke green. Compile-time/dep-graph win only. (commit `<hash>`)

- [ ] **Commit** (message):
  ```
  mcp: trim tokio to current-thread stdio feature set

  The MCP server is a single-task rmcp stdio server; tokio "full" compiled
  the whole runtime surface it never uses. Switch to
  #[tokio::main(flavor = "current_thread")] with features rt/macros/io-std.
  Rebuilt and redeployed; initialize handshake verified over stdio.
  ```

---

## Suggested execution sequence

1. C1 → C2 (addon, back-to-back; one test run each; two commits). INDEPENDENT.
2. C3 (fs prune + AGENTS.md + ledger). INDEPENDENT.
3. C4 + C5 (ledger notes; one commit). INDEPENDENT.
4. When a cargo slot opens (orchestrator): C7 (quick), then C6 (spike, needs the
   benchmark window < ~07:00 and load < 2).

C1–C5 need no cargo and can complete before any Rust phase.

---

## Conflicts / notes for the orchestrator (report, do not resolve)

1. **gfx encoders vs D11 scope.** D11 lists "gfx encode helpers if trivially
   shared" as candidates, but `starbreaker-gfx/src/render.rs:138` /
   `resolver.rs:231` encode UI screen / hologram / radar textures — i.e. UI-side
   renders. D11 also says **UI PNGs are EXCLUDED** (freeze gate hashes PNG
   bytes). These two clauses conflict. C6 EXCLUDES the gfx encoders to keep the
   spike clean and the freeze gate valid; only the `starbreaker-3d/pipeline`
   texture-export encoders are switched. Flagging in case the orchestrator wants
   gfx measured separately under an explicit re-freeze.

2. **`cli/src/dds.rs:290`.** D11/research name it, but `:290` (`img.save` for the
   grayscale alpha-mip) and `:295` (`dds.save_png`) are the standalone `dds` CLI
   subcommand, not the decomposed export path measured on a Clipper texture
   export. C6 leaves it out of the spike (noted as an optional follow-up). If the
   owner wants the CLI dump tool sped up too, that's a separate, non-measured
   change.

3. **Item 5 `dcb_canvas`.** D10 already reflects this, but restating for the
   end-of-run report: the handoff's delete-dcb_canvas instruction is unsafe (live
   test dependency). No reclaim beyond the graphify snapshot prune (~40–50 M) is
   available now; the 46–66 G target is void (no `target/debug`).

4. **Item 4 test placement.** The POM-bias test goes in `tests/test_layers.py`
   (already imports `builders` under a working stub set) rather than a new file —
   avoids duplicating the ~50-line bpy/mathutils/numpy stub preamble. If the
   orchestrator prefers a dedicated `tests/test_pom_bias.py`, copy the
   test_layers.py:1–57 preamble verbatim.
