# starbreaker-optimisation — recommendations

Append improvements to **THIS skill** here during a pass (do not rewrite
`SKILL.md` mid-pass). Process/tooling/profiling findings go to
`docs/optimisation-ledger.md` instead; this file is only for changes to how the
skill itself should guide the next pass.

Format per item: **Observed** (what friction/gap surfaced) → **Recommendation**
(the concrete SKILL.md edit) → **Status** (open / applied in commit `<sha>`).

## Open recommendations

_(none yet)_

## Applied

- **Skill created (2026-06-21).** Distilled from the Idris export optimisation arc:
  the O(depth) canonicalisation win (landed) and the parallel interior-sidecar
  rewrite (byte-identical + deterministic + memory-safe but **+26s slower**,
  reverted). That arc is the "watched it fail" grounding for the
  profile-parallelism-before-you-build rule and the keep-or-revert discipline.
  Initial flow + red-flags table seeded from it.

- **2026-07-02 (DDNA memo pass) — FOLDED into SKILL.md 2026-07-02 as Measurement
  rules 7–10.** Worked well end-to-end (bisect → localize →
  memoize → byte-identical oracle → ledger). Additions worth folding into SKILL.md:
  (1) "validate the baseline's PROVENANCE" as a Measurement rule — a fast prior run
  may be a STALE BINARY; `target/release/deps/starbreaker-<hash>` executables are a
  no-rebuild bisect ladder; (2) when a probe stays silent during a hot phase, the
  probe is at the WRONG LAYER (the DDNA decode bypassed `cached_load_keyed`, so
  `[tex-miss]` showed nothing) — instrument the phase boundary FIRST (per-item
  heartbeats), then descend; (3) gdb/perf can be locked down (ptrace_scope,
  perf_event_paranoid=4): `/proc/<pid>/task/*/stat` R/S counts distinguish
  serial-main-thread vs parallel phases for free; (4) tmpfs /tmp fills fast with
  multi-GB export outputs — budget bench dirs and clean between runs, or a later
  run fails mid-write with "Disk quota exceeded" and poisons the comparison.
