# starbreaker-optimisation — recommendations

Append improvements to **THIS skill** under **Open** during a pass (do not rewrite
`SKILL.md` mid-pass). Process/tooling/profiling findings go to
`docs/optimisation-ledger.md` instead; this file is only for changes to how the
skill itself should guide the next pass. When an item is applied, move it to the
**Change log** as a one-line entry.

## Open

1. **Re-verify handoff environment claims** (2026-07-18, perf-handoff run).
   Handoff docs from a prior session carry environmental claims (disk usage, dep
   versions, "nothing reads X") that drift — that run found the 46-66G disk-reclaim
   premise void, `image`'s png link mis-versioned, and `dcb_canvas` read by a live
   test guard. Recommendation: a Step 4 bullet — re-verify every environmental
   number in a handoff during research, before scoping work on it.
2. **Keep gate verdicts reviewable** (2026-07-18). Per-task adversarial review
   caught a gate-criterion misapplication (item-7 `swf_load` + `swf_load_measure`
   summation) that would have dropped a ~2s/export win. Recommendation: a Step 7
   rule — a gate-verdict report shows the criterion and the numbers together.
3. **Port the perf scripts to Windows** (2026-10-14 docs audit). `perf_bench.sh` /
   `perf_provenance.sh` need Linux (`/usr/bin/time -v`, `/proc/loadavg`, `nproc`,
   GNU `stat -c`); the owner develops on Windows. A code task — once done, drop the
   SKILL's Platform note caveats.

## Change log

- **2026-06-21 — created** from the Idris export optimisation arc (O(depth)
  canonicalisation landed; parallel interior-sidecar rewrite +26s slower,
  reverted) — the grounding for profile-parallelism-first and keep-or-revert.
- **2026-07-02 — DDNA memo pass findings folded in** as Measurement rules 7–10
  (baseline provenance / bisect ladder, silent probe = wrong layer, locked-down
  profilers → `/proc` run-state, tmpfs budget).
- **2026-07-08 — ponytail note** added to strict rules (alignment plan T15,
  `writing-skills` review); no process-skill line, since the sub-skills were
  already named correctly.
- **2026-10-14 — docs audit:** Linux-only tooling labelled (Platform note,
  `.exe` binaries on Windows); file-size rule aligned to AGENTS.md (~500 split
  signal, 3000 = UI cap only); dead `idris-export-perf` memory pointer replaced by
  the ledger's `AEGS_Idris_P` workload section; graphify marked optional; commits
  on the pass branch, never `main`.
