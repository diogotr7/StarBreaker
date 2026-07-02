---
name: starbreaker-optimisation
description: Use when making a StarBreaker export/pipeline/CLI operation faster or leaner — a slow `entity export`, an export that takes too long or uses too much memory, optimising decomposed/blend/UI/texture stages, or any "make X faster / profile this / why is this slow" request on the Rust pipeline. Triggers: slow export, profile, optimise, parallelise, `[timing]` stage breakdown, high RSS / OOM during export.
---

# StarBreaker Optimisation

## Overview

Make a StarBreaker pipeline operation measurably faster (or leaner) **without
changing its output**, by profiling first, optimising the proven bottleneck, and
re-profiling every change. This skill ORCHESTRATES a measurement-driven loop and
defers the actual change-making to the planning and implementation sub-skills; it
does not replace them.

**Core principle:** never optimise from a guess. MEASURE the bottleneck, RESEARCH
why it is slow, change ONE thing, then RE-PROFILE. A change with no measured gain —
or a regression — is REVERTED, not kept "because it's nicer". Complexity is only
earned by an observed speedup. **Output stays byte-identical** unless the user
explicitly approves a behaviour change; an export that is faster but wrong is a
failure, not an optimisation.

**The expensive lesson this skill exists to prevent:** parallelising a stage that
looked CPU-bound but was actually MEMORY-bound produced a fully byte-identical,
deterministic, OOM-safe rewrite that was **+26s SLOWER** and had to be thrown
away. Profile the parallelism efficiency of a representative slice BEFORE building
the whole parallel path (Measurement rules §2).

## Operating posture

Nearly every way this skill wastes effort is the same shortcut: trusting a guess
or a stale number instead of a fresh measurement on this machine. Default
behaviour by category:

- **Inputs / user-facing choices → ASK.** What to optimise (multi-select), and —
  always — permission before running a benchmark. Fresh `AskUserQuestion` each
  time; never presume the target or that "now" is a fine time to load the CPU.
- **Technical judgments → MEASURE.** Where the time goes, whether a stage is
  CPU- or memory-bound, whether a change helped: prove it with the `[timing]`
  breakdown, `/usr/bin/time -v`, and a re-profile — never estimate-then-assume.
  Apples-to-apples means the SAME machine, back-to-back, multiple runs; a number
  from a prior session or a doc is a hint, not a baseline.
- **The work itself → USE THE SUB-SKILLS.** Planning and implementation are
  `superpowers:writing-plans` and `superpowers:executing-plans` /
  `superpowers:subagent-driven-development`; debugging an unexpected result is
  `superpowers:systematic-debugging`. Don't hand-roll those phases.
- **Resource-heavy or hard-to-reverse actions → GATE.** A benchmark run (loads
  every core), a commit, and a push: stop and ask via `AskUserQuestion`. The
  user's other CPU/GPU work makes benchmark timing meaningless and may be
  disrupted — never benchmark unprompted.

Asking and measuring are always safe; presuming and reusing a stale timing are the
failure mode.

## Required reads (before any change)

In order: `StarBreaker/AGENTS.md` (no hard-coding, no game-data values, commit/naming
rules) → the target subsystem's `AGENTS.md` / docs (e.g. `docs/decomposed-export-contract.md`
for export shape; the relevant crate's `//!` headers) → the optimisation ledger
`docs/optimisation-ledger.md` (what's been tried, what's a known dead-end — e.g.
jemalloc is slower here, the parallel-sidecar regression). If a prior entry already
proved your idea a dead-end, pick another candidate.

**Profiling is built in — read it, don't add print-debugging.** `RUST_LOG=info`
already emits `[timing][decomposed]` / `[timing][blend]` / per-stage lines for the
export pipeline; `SB_UI_TIMING=1` adds per-binding UI timing (but its logger mutex
inflates and serialises the numbers — don't trust its absolute parallelism). Read
those first.

## The optimisation loop

**Track the pass with TodoWrite from the start — include the mandatory closing
retrospective (below) as a todo so it is never dropped.** One pass = pick a
workload, baseline it, optimise one-or-more chosen items, re-baseline, retro, then
ask whether to repeat.

### 0. Pick the workload + the correctness oracle

Choose a representative, repeatable workload (e.g. `AEGS_Idris_P --kind decomposed
--lod 0` — a capital ship exercises every stage). Establish the **correctness
oracle** up front: a full export to a baseline dir you will `diff -rq` every later
build against (`diff -rq baseline new | grep -v export_stamp` must be EMPTY). No
oracle → no optimisation; you cannot tell a speedup from a corruption.

### 1. Instrument

Ensure the workload emits a stage breakdown (`RUST_LOG=info` for the export
pipeline). If a hot region has no timing, ADD a `[timing]` line at the stage
boundary (cheap `Instant::now()` / `elapsed()`), behind the existing log level —
not a scattered `eprintln`. Instrumentation must not change output.

### 2. Baseline (GATE: ask before benchmarking)

Ask the user via `AskUserQuestion` whether now is a good time to run benchmarks
(quiet machine). Then capture the baseline: **3+ runs**, clean release binary, no
mem-cap overhead, recording wall time, the `[timing]` stage breakdown, CPU%, and
max RSS:

```bash
cargo build --release -p starbreaker        # AGENTS.md: release only for perf/deploy
for r in 1 2 3; do
  /usr/bin/time -v env RUST_LOG=info ./target/release/starbreaker \
    entity export "<entity>" target/tmp/base$r --kind decomposed --lod 0 \
    > target/tmp/base$r.log 2>&1
  grep -aE "Elapsed \(wall|Percent of CPU|Maximum resident|\[timing\]" target/tmp/base$r.log
done
```

Keep one baseline export dir for the byte-identical oracle. Record the numbers in
the TodoWrite/scratch so later deltas compare against a written baseline, not a
memory.

### 3. Analyse

Read the stage breakdown. Find the DOMINANT stage(s) — optimise the biggest
contributor, not the most interesting one. A 9s stage that parallelises to 8s is a
worse target than a 30s stage with an O(n²) inside it. Note what each stage's cost
actually IS (decode? JSON build? mesh clone? file inserts?) — the memory says the
labels (e.g. "this stage = JSON build, NOT texture decode").

### 4. Research candidates (fan read-only work to subagents)

For each candidate bottleneck, research WHY it is slow and what bounds a fix:
- **CPU-bound vs memory-bound vs IO-bound?** Check CPU% during the stage. A stage
  that already saturates cores is CPU-bound (algorithmic win only); a long stage at
  low CPU% is serial or memory-bound. **Memory-bound work does NOT parallelise** —
  prove parallelism efficiency on a SLICE (Measurement rules §2) before proposing a
  parallel rewrite.
- **Algorithmic?** O(n²) scans, redundant work, re-decoding, rebuilt-per-item
  caches (the canonicalize O(files²) → O(depth) win; the UI render dedup 219→29).
- **Shared-state / ordering dependencies** that constrain parallelism or threaten
  byte-identity (path canonicalisation first-seen casing, collision naming order,
  a closure side-effect that another stage consumes).

Use `graphify` (`graphify query`/`explain`, `/graphify`) to find the owning code
relationship-aware, not blind grep. Delegate independent read-only research to
subagents (one per candidate); **never delegate builds/benchmarks** — they share
the cargo target and race, and benchmarks must be serial on a quiet machine.

### 5. Offer options (ASK: multi-select)

Present the researched candidates via `AskUserQuestion` with **`multiSelect: true`**.
For EACH option give: expected gain (from the profile, not a hope), the RISK
(byte-identity, memory, blast radius, complexity), and rough effort. Include a
recommended option first. The user picks one or more to pursue this pass.

### 6. Plan, then review (sub-skills)

For the chosen item(s), **use `superpowers:writing-plans`** to write the plan to
`docs/superpowers/plans/<date>-<topic>.md`, then review it (re-read against the
research; sanity-check the expected gain and the byte-identity story). Decompose
into the smallest independently-profilable changes.

### 7. Implement → re-profile → keep-or-revert (per change)

**Use `superpowers:executing-plans` / `superpowers:subagent-driven-development`** to
execute. After EACH discrete change:

1. **Verify correctness FIRST.** Rebuild, re-export, `diff -rq` vs the oracle
   (`grep -v export_stamp` empty). For anything touching parallelism, run the
   workload **twice** and diff the two outputs — non-determinism (a `.blend` set
   that varies run-to-run) is a bug, not noise. A risky run gets `--mem-cap <MB>`
   so a runaway aborts gracefully instead of OOM-killing the machine.
2. **Re-profile** (ask before the benchmark if the machine may be busy).
3. **Keep-or-revert:** a change with a measured gain AND identical output is kept.
   A change with **no observed gain, or a regression, is REVERTED** (`git
   checkout`/stash) — do not keep complexity that doesn't pay. If a better approach
   is visible, re-plan it; otherwise drop the item and move on. Never ship "it's
   cleaner / it's byte-identical so it's fine" without a speedup — byte-identical
   AND slower is still a revert.

If a change behaves unexpectedly (OOM, drift, slowdown), switch to
`superpowers:systematic-debugging` — and when 3+ fixes each reveal a new problem,
question the architecture (the parallel-sidecar arc hit OOM → layers → mesh_data →
casing → collision before the benchmark finally killed it).

### 8. Re-baseline + verify (before the retro)

Re-run the full baseline (3+ runs, same machine) on the final binary. Confirm the
**net** wall-time improvement is real (not contention — read CPU% and the stage
breakdown, not wall time alone) and the output is byte-identical. If the net is a
wash or a regression after reverting the bad parts, say so plainly — a pass that
proves an approach is a dead-end is a SUCCESS (record it so it isn't re-attempted),
not a failure to hide. Then run the retrospective.

### 9. Ask: repeat? (ASK)

Via `AskUserQuestion`, ask whether to run another pass against the NEW baseline
(the next-dominant stage) or stop here. Each repeat starts again at step 3
(analyse) on the fresh profile.

## Measurement rules (hard-won)

1. **Read the `[timing]` markers, not wall time alone.** Wall time hides which
   stage moved and is corrupted by machine contention. The stage breakdown + CPU% +
   max RSS are the real signal. `/usr/bin/time -v` gives wall, CPU%, and Maximum
   resident set size.
2. **Profile parallelism efficiency on a slice BEFORE building a full parallel
   path.** If a `par_iter` over the real work gives ~1× speedup and CPU stays well
   below `cores × 100%`, the work is memory-bandwidth-bound and threads won't help —
   STOP, don't build the parallel machinery. (The interior-sidecar prebuild hit
   ~1× at <600% on 16 threads; the whole rewrite was a regression.)
3. **Benchmark on a QUIET machine, multiple runs, apples-to-apples on THE SAME
   machine.** Ask before loading the cores. A prior-session or documented number is
   NOT a baseline — build the comparison binary and measure both back-to-back
   (stash the change, build, time; pop, build, time).
4. **`--mem-cap <MB>` is the safety guard** for runs that might blow memory (aborts
   at the cap instead of OOM-killing). But it adds allocation-tracking overhead — do
   correctness runs with it, but do TIMING runs without it.
5. **Determinism is part of correctness.** Parallelism + shared maps + completion
   order can make output vary run-to-run. Run the workload twice and diff.
6. **The allocator is rarely the answer.** jemalloc via `LD_PRELOAD` was measured
   SLOWER here — the pipeline is not allocator-bound. Don't reach for it.
7. **Validate the baseline's PROVENANCE.** A "fast prior run" may be a STALE
   BINARY from before the regressing commit — stat the binary mtime against
   `git log` before trusting any endpoint. Old hashed executables under
   `target/release/deps/starbreaker-<hash>` are a free no-rebuild time-travel
   bisect ladder.
8. **A silent probe means the WRONG LAYER, not "no cost".** Instrument the
   phase boundary first (per-item heartbeats), then descend — the DDNA
   regression bypassed `cached_load_keyed`, so a `[tex-miss]` probe there
   stayed silent through a 280s phase.
9. **Profilers may be locked down** (`ptrace_scope`, `perf_event_paranoid=4`):
   `/proc/<pid>/task/*/stat` run-state counts (1 R + N S = a serial
   main-thread phase) distinguish serial vs parallel phases for free.
10. **Budget /tmp (tmpfs) for bench outputs** — multi-GB export dirs fill it;
    a later run then dies mid-write ("Disk quota exceeded") and poisons the
    comparison. Clean bench dirs between runs.

## Strict rules

- **Byte-identical output is mandatory** unless the user explicitly approves a
  behaviour change (and then a NEW baseline is captured and the change is recorded
  as a behaviour change, not an optimisation). `diff -rq base new | grep -v
  export_stamp` empty, every build.
- **No hard-coded game-data values, no name/asset/ship gating** (AGENTS.md) — an
  optimisation that special-cases one asset is banned, even in a fast path.
- **Revert no-gain and worsening changes immediately;** record what was falsified.
  Don't add a cache, a parallel pass, or an index that the profile doesn't reward.
- **Don't commit or push unless the user asks.** Commit per coherent landed win;
  message follows repo convention (NO `Co-authored-by` trailer, no maintainer name —
  AGENTS.md). Branch first if on the default branch.
- **3000-line / small-file discipline; update `//!` headers** when a file's
  responsibility changes; verify-on-write any command you cite in a doc.

## Checkpoints

At each ACTIVE checkpoint present the evidence, then ask via `AskUserQuestion` (a
concrete approve/decline). Never proceed on a presumed "yes".

| Checkpoint | Action |
|---|---|
| **Run a benchmark** (machine may be busy) | **gate** — ask "OK to run benchmarks now?" each time |
| **Optimisation selection** (what to pursue) | **ask** — multi-select with gain/risk/effort |
| **Plan review** (before implementing) | review against research; proceed when sound |
| **Commit / push** | **gate** — show the diff + the measured delta |
| **Behaviour change** (output not byte-identical) | **gate** — never ship a non-identical output without explicit approval |
| **Repeat another pass** | **ask** — another pass vs stop |

## Self-improve every pass: the retrospective (MANDATORY closing step)

**The pass is not done when the speedup lands — it is done after the
retrospective.** Run it in the SAME session (lived context), before declaring
complete. It is the TodoWrite item added at the start; do not close with it open.

Sweep this pass's lived experience — for each, FIX it, don't just note it:

1. **Repeated manual work → tooling.** A profiling command battery, a
   diff+grep oracle, a stash/build/time comparison typed >2× becomes a
   `scripts/` helper.
2. **Missing/inflated instrumentation → fix it.** A stage with no `[timing]`
   line, or a probe whose own overhead skews the number (like `SB_UI_TIMING`),
   gets a real boundary timer.
3. **A dead-end proven → record it** so it isn't re-attempted (jemalloc slower;
   parallelising a memory-bound stage; an allocator/cache that didn't pay).
4. **A surprising win or root cause → record the lever** (the cost was JSON build
   not decode; O(files²) canonicalisation; render redundancy).
5. **Bootstrap cost.** Anything you re-derived (where the timing logs are, the
   mem-cap flag, the oracle command, the quiet-machine rule) lands in the ledger or
   this skill's reference.

Two destinations:

- **Profiling findings / dead-ends / levers / tooling →** APPEND numbered items to
  `docs/optimisation-ledger.md` (Observed / Finding / Action format) and IMPLEMENT
  the tooling/instrumentation wins; one commit per coherent item. Also update the
  relevant project memory (e.g. `idris-export-perf`) with the cumulative numbers and
  any new dead-end, so the next session starts from the current baseline.
- **Improvements to THIS skill →** append under **Open recommendations** in
  `recommendations.md` (next to this file); do not rewrite `SKILL.md` mid-pass.

Acceptance (bootstrap test): a fresh agent could run the next pass from this skill
+ `docs/optimisation-ledger.md` + the memory alone. Any excursion you needed is a
doc bug — fix it before closing.

## Red flags — STOP, you're rationalizing

| Thought | Reality |
|---|---|
| "This stage is slow, parallelise it" | First profile parallelism on a slice. ~1× speedup at low CPU% = memory-bound; threads won't help and the safety machinery will cost more than the win (the +26s sidecar regression). |
| "The docs / last session say it was Xs" | Not a baseline. Measure on THIS machine, back-to-back, build both binaries. Prior numbers are hints. |
| "Wall time dropped, it's faster" | Could be contention or noise. Read the `[timing]` stage breakdown + CPU% across 3+ runs; confirm the stage you touched actually moved. |
| "It compiles and a quick export looks right — ship it" | Byte-identical oracle (`diff -rq … grep -v export_stamp` empty) AND a twice-run determinism diff. Faster-but-wrong is not an optimisation. |
| "It's byte-identical and cleaner, keep it" | No measured gain = REVERT. Byte-identical AND slower is still a revert. Complexity is earned by a speedup, nothing else. |
| "Add a cache/index/parallel pass, it should help" | "Should" is a guess. Land it, re-profile; if the profile doesn't reward it, revert. |
| "Just run the benchmark real quick" | ASK first — the user's other CPU/GPU work makes the number meaningless and may be disrupted. Benchmarks are gated. |
| "Each fix reveals a new problem, push through" | 3+ fixes each surfacing a new failure = question the architecture (OOM→layers→mesh_data→casing→collision was the signal the whole approach was wrong). |
| "Try jemalloc / a fancier allocator" | Measured slower here; the pipeline isn't allocator-bound. The ledger records dead-ends — read it first. |
| "I'll plan and implement this inline" | Use `superpowers:writing-plans` then `superpowers:executing-plans`/`subagent-driven-development`; `systematic-debugging` when a change misbehaves. Don't hand-roll the phases. |
| "Output changed but it's arguably more correct" | That's a behaviour change, not an optimisation — GATE it with the user, capture a new baseline; never silently ship a non-identical output. |
| "Commit the win, push it" | Commit/push only when the user asks; show the diff + measured delta; repo commit convention (no `Co-authored-by`, no maintainer name). |
