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

- **2026-07-08 (alignment plan T15 — ponytail reconciliation; applied via
  `superpowers:writing-skills` review).** `SKILL.md` strict rules gained ONE line — the
  **ponytail note** (laziest-that-works = smallest change still engine-faithful + data-derived;
  a hard-coded value / invented geometry / name-gate / heuristic is never "lazy" here, it trips
  the guards). This skill already names its superpowers sub-skills correctly
  (`writing-plans`/`executing-plans`/`subagent-driven-development`/`systematic-debugging`), so it
  did NOT get the process-skill/brainstorming line. `writing-skills` pass: reframe-not-prohibition
  form, bold lead, no `SKILL.md` structural change beyond the one bullet.

## Open recommendations (appended 2026-07-18, perf-handoff implementation run)
- Handoff docs written in a prior session carry environmental claims (disk usage, dep versions, "nothing reads X") that drift: this run found the 46-66G disk-reclaim premise void, `image`'s png link mis-versioned, and dcb_canvas actually read by a live test guard. Re-verify every environmental number in a handoff during research, before scoping work on it.
- Per-task adversarial review caught a gate-criterion misapplication (item-7 swf_load + swf_load_measure summation) that would have silently dropped a ~2s/export win. Keep gate-verdict tasks reviewable: the criterion and the numbers must appear together in the report.
