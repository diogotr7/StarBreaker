# System Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute every recommendation of the 2026-07-07 system alignment review — hygiene (P0), prose-lessons-to-enforced-gates (P1), knowledge-store reconciliation (P2), and measurement/validation (P3) — so the whole toolchain pulls toward: near-pixel-perfect static UI replicas baked into Blender exports, engine-faithful and generic, scalable across many ships with auto arcs on known ships.

**Architecture:** Four phases matching the review's priorities. P0 touches only user-level config (memory dir, Claude settings) — safe while B1–B4 arcs run. P1 converts the six still-prose failure traps into mechanical gates (scripts, hooks, tests). P2 reconciles the governance/knowledge docs with reality and the owner's four direction decisions. P3 builds the measurement baseline (arc cost, frozen-target drift) that makes "scale across many ships" steerable. Spec: `docs/StarBreaker/2026-07-07-system-alignment-review.md` (workspace docs, next to this plan).

**Tech Stack:** bash + python3 (scripts, validators), Rust (`crates/starbreaker-ui` tests, guard tests), Claude skill markdown (`StarBreaker/.claude/skills/`), Claude settings JSON, git hooks.

## Global Constraints

- **REPO-QUIET GATE: no task that writes inside `StarBreaker/` executes until `git -C StarBreaker status --porcelain` is clean of other agents' work AND the owner confirms the B1–B4 arcs are done or paused.** P0 tasks (1–3) are exempt (they never touch the repo). Record the confirmation in the task's commit message ("repo-quiet confirmed by owner <date>").
- All sub-agents run on **Opus**. Read-only research may fan out in parallel; **anything running cargo / `ui_check.sh` / `ui_render.sh` / `entity export` runs sequentially in one executor at a time** (shared cargo target).
- `bash scripts/ui_check.sh` green before every StarBreaker commit — **read the RESULT MARKER (`ui_check: ALL GREEN`) from unpiped output, never a piped exit code** (ledger 89).
- TDD: every behavioural change (scripts with logic, hooks, Rust tests-as-guards) gets a failing self-test FIRST (`--self-test` scaffold pattern, exit 2 "not implemented" → implement → PASS). Validation scripts print a RESULT MARKER (`<tool>: OK …` / `<tool>: FAILED (exit N)`) via an EXIT trap.
- No hard-coding: no ship/screen/manufacturer branches, no magic offsets, **no hard-coded game-data values** anywhere including fixtures/fallbacks. Derived counts (e.g. expected PNG totals) come from scene/dossier data at run time.
- Verify-on-write: every command line added to a doc is run once at writing time; renames include a repo-wide reference grep in the same commit.
- Commits directly on `feature/ui`; no new branches/worktrees. One commit per task. Commit messages follow the repo style (`feat(ui-process): …`, `docs(ui): …`). **No Co-Authored-By / session trailers** (repo rule overrides harness default).
- Never name the maintainer — "the owner"; no `/home/<user>` paths in repo content (`$HOME`/`~` only).
- Nothing in this plan freezes or re-freezes a baseline. If any task's validation shows a frozen-target delta, STOP and present to the owner — do not adjudicate alone.
- Docs that change in `StarBreaker/` get one graphify doc sync (`/graphify . --update`) at the END of P2 (Task 16), not per-task.

## Task Dependency / Parallelism Map

| Task | Phase | Repo write? | Depends on | Parallel-safe with |
|---|---|---|---|---|
| 1 memory prune | P0 | no | — | 2, 3 |
| 2 global settings | P0 | no | owner approval step | 1, 3 |
| 3 project allowlist | P0 | no | — | 1, 2 |
| 4 marker pre-commit gate | P1 | yes | repo-quiet | — (sequential) |
| 5 vacuous-guard preconditions | P1 | yes | repo-quiet | — (sequential) |
| 6 element-presence guard | P1 | yes | repo-quiet | — (sequential) |
| 7 export smoke check | P1 | yes | repo-quiet | — (sequential) |
| 8 perf provenance gate | P1 | yes | repo-quiet | — (sequential) |
| 9 skill folds (blockers/waits) | P1 | yes | 4–8 landed | — |
| 10 goal paragraph | P2 | yes | repo-quiet | 12, 14 (different files; still commit sequentially) |
| 11 residual-budget doc | P2 | yes | owner content gate | — |
| 12 hard-coding discriminator | P2 | yes | repo-quiet | 10, 14 |
| 13 ledger index + retro-prompt | P2 | yes | repo-quiet | — (ledger is append-target; single writer) |
| 14 governance batch | P2 | yes | repo-quiet | 10, 12 |
| 15 skill mode-default + persona lines | P2 | yes | 9 (same files) | — |
| 16 graphify sync + ledger entry + plan copy | P2 | yes | 10–15 | — |
| 17 arc-cost instrumentation | P3 | yes | repo-quiet | — |
| 18 frozen-target drift audit | P3 | no (report only) | repo-quiet (uses toolchain) | — |
| 19 A10 Carrack arc | P3 | yes (arc) | 9, 17 | — |

Execution shape for the orchestrator: **P0 = up to 3 parallel sub-agents. P1/P2/P3 = one executor sub-agent at a time** (every task either builds/tests or writes shared docs), with read-only research fanned out inside a task where marked.

---

## Phase P0 — Hygiene (no repo writes; start immediately)

### Task 1: Memory prune + rename

**Files:**
- Modify: `~/.claude/projects/<memory-dir>/memory/MEMORY.md`
- Rename: `.../memory/ui-parity-redesign-gated.md` → `.../memory/ui-parity-redesign-status.md`
- Create: `.../memory/archive/` (move pruned files here — archive, never delete)

**Interfaces:**
- Produces: a MEMORY.md index where every line still changes future-session behaviour; `[[ui-parity-redesign-status]]` as the new slug.

- [ ] **Step 1:** Read `MEMORY.md` and all 56 memory files. For each of these candidates, confirm the status claimed in the index is "landed/DONE/superseded" by checking the file body (they were verified once already in the review — re-verify only that no file has an OPEN item not captured elsewhere): `font-sizing-constants-load-bearing` (superseded by `ui-font-size-engine-model`), `ui-improvement-plan-executed`, `medical-tint-fix-plan`, `power-screen-parity-plan`, `flash-hybrid-rendering-plan` (recovered into plan B3 — confirm B3 owns it now), `widget-standard-expansion-landed`, `mfd-content-view-stage-subrect`, `mfd-aspect-tag-content-scaling`, `caps-reduction-removed`, `clipper-medical-bed-parity`, `medical2-item2-investigation`, `clipper-target-screen-parity`.
- [ ] **Step 2:** Before archiving each file, harvest any line that is still a live LESSON (not status) and confirm an equivalent exists in a surviving memory or repo doc (e.g. power-screen's "measure, don't estimate blocked" lives in the skill's blockers reference). If an orphan lesson is found, append it to the most related surviving memory instead of keeping the whole file.
- [ ] **Step 3:** `mkdir -p .../memory/archive && mv` the pruned files; delete their index lines from `MEMORY.md`.
- [ ] **Step 4:** `mv ui-parity-redesign-gated.md ui-parity-redesign-status.md`; update the `name:` frontmatter to `ui-parity-redesign-status`; update its MEMORY.md line and grep the memory dir for `[[ui-parity-redesign-gated]]` links — update each.
- [ ] **Step 5:** Sanity: `wc -l MEMORY.md` should drop by ~12 lines; every remaining index line names a file that exists (`for f in $(grep -o '([a-z0-9-]*\.md)' MEMORY.md | tr -d '()'); do test -f .../memory/$f || echo MISSING $f; done` → no output).

### Task 2: Global settings cleanup (owner-gated)

**Files:**
- Modify: `~/.claude/settings.json`

- [ ] **Step 1:** Prepare the change set as a diff and present it to the owner via AskUserQuestion (one question per decision):
  1. Delete 6 dead `permissions.allow` entries: the two `awk … 45253bd8-…/tool-results/…` lines, the `swf_inventory` cargo-run line, the `entity export drak_clipper` cargo-run line, the `mc_s_target_master.json` python one-liner, the `y=[89][0-9][0-9]` awk one-liner. (All either reference dead session paths or are covered by the broad `Bash(cargo run *)` / `Bash(python3 -c ' *)` prefixes.)
  2. Disable plugins: `frontend-design`, `playwright`, `security-guidance` (zero surface for a headless Rust renderer); optionally `code-simplifier` (overlaps ponytail-review).
  3. Ponytail level: `lite` (shorter injection, keeps anti-over-engineering signal) vs keep `full` + reconciling line in domain skills (Task 15 adds the line either way). Recommend `lite`. NOTE: first identify the actual level mechanism (ponytail state lives in its own global state file, e.g. `~/.claude/.ponytail-active`, NOT in settings.json) and confirm whether a per-project override exists — if not, tell the owner the change is GLOBAL before applying.
- [ ] **Step 2:** Apply exactly what the owner approves. JSON-validate: `python3 -c "import json;json.load(open('$HOME/.claude/settings.json'))"` → no error.
- [ ] **Step 3:** Note in the task report: plugin/permission changes take effect next session; do not restart anything mid-arc.

### Task 3: Project allowlist additions

**Files:**
- Modify: `~/projects/scorg_tools/.claude/settings.local.json`

- [ ] **Step 1:** Add to `permissions.allow`: `Bash(git add *)`, `Bash(git commit *)`, `Bash(git status*)`, `Bash(git log*)`, `Bash(git diff*)`, `Bash(bash scripts/ui_check.sh*)`, `Bash(bash scripts/ui_render.sh*)`, `Bash(bash scripts/ui_arc_status.sh*)`, `Bash(python3 scripts/*)`.
- [ ] **Step 2:** JSON-validate as in Task 2. Report which entries were already present (skip duplicates).

---

## Phase P1 — Prose lessons → enforced gates (repo-quiet required; sequential)

### Task 4: `ui_check` marker file + pre-commit gate

Kills the ledger-89 trap (piped exit code read as green) mechanically: committing renderer/script changes requires a fresh, genuinely-green `ui_check` marker.

**Files:**
- Modify: `StarBreaker/scripts/ui_check.sh` (EXIT trap block)
- Create: `StarBreaker/scripts/install_ui_precommit.sh`
- Create: `StarBreaker/scripts/hooks/pre-commit-ui-gate` (the hook body, tracked in-repo; installer symlinks/copies it)

**Interfaces:**
- Produces: marker file `.git/ui-check-marker`, format (3 lines): `ui_check: ALL GREEN` or `ui_check: FAILED (exit N)` / `head=<git rev-parse HEAD at run time>` / `epoch=<unix seconds>`. Consumed by the pre-commit hook and by Task 18's audit.

- [ ] **Step 1 (failing self-test):** Create `install_ui_precommit.sh` with a `--self-test` scaffold that exits 2 "not implemented". Run it → `install_ui_precommit: FAILED (exit 2)`.
- [ ] **Step 2:** Rework `ui_check.sh`'s EXIT trap so it constructs the marker for BOTH outcomes and writes the 3-line marker file to `"$(git rev-parse --git-dir)/ui-check-marker"` on every exit. NOTE the current shape (verified): the trap at ~line 31 prints the marker only on failure (`rc -ne 0`); the `ui_check: ALL GREEN` line is a plain `echo` at ~line 107 outside the trap — move marker construction into the trap (`rc==0` → `ALL GREEN`, else `FAILED (exit rc)`) without changing the printed console output. Never let marker-file write failure change the script's exit status.
- [ ] **Step 3:** Write `scripts/hooks/pre-commit-ui-gate`: if any STAGED path matches `^(crates/starbreaker-ui/|scripts/|mcp/)` then require the marker file to (a) exist, (b) first line == `ui_check: ALL GREEN`, (c) `epoch` newer than `SB_UI_MARKER_MAX_AGE` (default 1800s). On failure, print the reason + `run: bash scripts/ui_check.sh (unpiped)` and exit 1. `SB_SKIP_UI_GATE=1` bypasses with a loud warning (for rebase/emergency). Docs-only commits pass untouched.
- [ ] **Step 4:** Implement the installer: copies the hook to `.git/hooks/pre-commit`. Verified ground truth: `.git/hooks/` currently has only `post-commit` (graphify's — do not touch) and `post-checkout`; there is NO existing pre-commit, so do NOT build chaining logic — if an unexpected `pre-commit` exists at install time, abort with a message for the human. `--self-test`: in a throwaway `git init` dir, simulate (stale marker → block; green fresh marker → pass; docs-only staged → pass; FAILED marker → block). Run → `install_ui_precommit: OK (4 cases)`.
- [ ] **Step 5:** Install for real; run `bash scripts/ui_check.sh` unpiped → marker file appears, hook passes on a dummy docs change; `bash scripts/ui_check.sh` green.
- [ ] **Step 6:** Document in `crates/starbreaker-ui/docs/ui-workflow.md` §3 validate step (one line: the marker file + hook; installer command) — verify-on-write. Commit: `feat(ui-process): ui_check marker file + pre-commit gate (alignment plan T4)`.

### Task 5: Vacuous-guard preconditions (match-count > 0 everywhere)

Generalises ledger 3/61/105: a guard that finds zero targets must FAIL, not pass.

**Files:**
- Create: `StarBreaker/scripts/lib/guard_assert.sh` (`assert_nonzero_matches <count> <what>` → prints `guard_assert: FAILED (0 matches: <what>)` and exits 1 on zero)
- Modify: every guard script/test found by the inventory (Step 1)
- Test: extend the existing hardcoding-guard self-checks; add `scripts/lib/guard_assert.sh --self-test`

- [ ] **Step 1 (read-only research, may fan out):** Inventory all guards: `ls scripts/*.sh | xargs grep -ln "guard\|GREEN"` plus `grep -rn "guard" crates/starbreaker-ui/tests/ --include=*.rs -l` plus the three in-module hardcoding guards (already strengthened per ledger 105 — verify, don't re-do). For each: does it assert its target/file/pattern set is non-empty before checking? Produce a table: guard → target discovery mechanism → N>0 asserted? (yes/no).
- [ ] **Step 2 (failing self-test):** `guard_assert.sh --self-test` scaffold → FAILED; implement; → `guard_assert: OK`.
- [ ] **Step 3:** For each "no" row: bash guards source `scripts/lib/guard_assert.sh` and assert after target discovery; Rust guard tests add `assert!(!targets.is_empty(), "guard scanned zero targets — vacuous")` (adapt to each test's collection variable).
- [ ] **Step 4:** Prove one representative case per language fails when vacuous (temporarily point a copy at an empty dir / filter; observe FAIL; restore). `bash scripts/ui_check.sh` green. Commit: `feat(ui-process): non-empty-target preconditions on all guards (alignment plan T5)`.

### Task 6: Generic element-presence guard + regression-policy correction

Closes the sub-threshold blind spot (ledger 77/106) with a guard DERIVED from each frozen target's own IR snapshot — no per-screen hand-authored knowledge, so it honours the generic-not-targeted policy.

**Files:**
- Create: `StarBreaker/crates/starbreaker-ui/tests/element_presence_guard.rs`
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-regression-policy.md` (~line 30)
- Read first: `crates/starbreaker-ui/docs/ir-freeze-schema.md`, the existing IR-snapshot test that iterates frozen targets (find via `grep -rn "snapshot" crates/starbreaker-ui/tests/ -l`)

**Interfaces:**
- Consumes: frozen IR snapshot fixtures (schema per `ir-freeze-schema.md`), live IR compile for each frozen target (same loader the IR-snapshot guard uses — reuse it, do not duplicate).
- Produces: test `element_presence_all_frozen_targets` — for every frozen target, every node present in the frozen IR snapshot with a non-degenerate rect (width>0 && height>0) and a drawable payload must exist in the live IR with a non-degenerate rect. Missing node or degenerate rect → failure naming target + node path.

- [ ] **Step 1 (research):** Read the freeze schema + one frozen snapshot fixture; identify the stable node identifier (path/id) and rect fields, and define "drawable payload" / "non-degenerate rect" precisely against `ir-freeze-schema.md` (write the definitions into the test's module doc). The reference implementation to mirror is `manifest_live_ir_guard.rs`: use ITS frozen-target enumeration, ITS live-IR input source (`FsCanvasFetcher`/`load_canvas_index` over the canvas export dir), and ITS skip-path when game data / the export dir is absent — identical tier, identical skip behavior. Add T5's `assert!(!targets.is_empty(), "element-presence guard scanned zero frozen targets — vacuous")` so the guard can never pass vacuously on a fresh checkout.
- [ ] **Step 2 (failing test):** Write the test with a deliberate synthetic check first: assert a node name that does NOT exist fails the guard (verifies failure path), and run against live IR → observe the failure message shape. Then write the real assertion loop. Run: `cargo test -p starbreaker-ui element_presence -- --nocapture` → real run PASSES on current tree (no regression right now), synthetic case proves it CAN fail.
- [ ] **Step 3 (falsification check):** Temporarily drop one small element in a scratch branch of logic (e.g. filter out one node id in a local unsaved edit), rerun → guard FAILS naming it; revert. This is the ledger-77 class caught mechanically.
- [ ] **Step 4:** Fix `ui-regression-policy.md`: replace the "catches any rendered change on any screen" claim with: whole-image %-budget catches large drift; elements smaller than the tier budget are invisible to it; the element-presence guard (IR-derived, generic) owns that class. Reconcile the "generic, not targeted" section: derived-from-snapshot ≠ hand-targeted.
- [ ] **Step 5:** `bash scripts/ui_check.sh` green (guard wired into the battery alongside the other repo-tier Rust tests — confirm it runs there). Commit: `feat(ui): element-presence guard derived from frozen IR snapshots + policy correction (alignment plan T6)`.

### Task 7: Composed-export smoke check

The deliverable is baked PNGs in the decomposed export; today no gate checks them end-to-end through PRODUCTION fetchers (blank-MFD class).

**Files:**
- Create: `StarBreaker/scripts/ui_export_smoke.sh`
- Modify: `StarBreaker/scripts/ui_check.sh` (`--full` game-data tier only)

**Interfaces:**
- Produces: `bash scripts/ui_export_smoke.sh <entity>` → UI-only export to a temp dir, then asserts (a) PNG count == the count of UI render targets derived from the exported scene data itself (scene.json / its UI sidecar — research exact field; NEVER a hard-coded number), (b) every PNG has >1 distinct pixel value (uniform image = dead render; use python3 + PIL `len(im.getcolors(2))`), (c) marker `ui_export_smoke: OK (N screens)` / `FAILED`.

- [ ] **Step 1 (research):** `scripts/benchmark_ui_only_export.sh` already runs the exact `--ui-only-files --lod 0 --mip 0` export this task needs — lift/source its export invocation rather than authoring a fresh one; the ONLY new logic in `ui_export_smoke.sh` is PNG-count-vs-scene-data + non-blank. Find where the export writes screen PNGs + which scene artifact enumerates the expected screens (the UI pipeline inputs / `UiRenderKey` dedup list). Record exact paths/fields in the script header comment.
- [ ] **Step 2 (failing self-test):** `--self-test` with two tiny synthetic PNGs (one uniform, one 2-colour) asserting the blank-detector; scaffold exits 2 → FAILED marker.
- [ ] **Step 3:** Implement; run for real against `drak_clipper` (canonical export command from workspace AGENTS.md, UI-only, LOD0) → `ui_export_smoke: OK (54 screens)` (54 is the OBSERVED current Clipper count — the script derives it; cite it only in the commit message as the observed value).
- [ ] **Step 4:** Wire into `ui_check.sh --full` tier. Verified ground truth: `--full` does NOT re-export — it checks the export stamp (`ships/Data/UI/Generated/.export_stamp.json`) and warns to re-export first; the export is an external precondition. So the smoke check reads the same pre-existing export dir (`ships/Data/UI/Generated`) the visual guards read, guarded by the same stamp/skip path — do not trigger a second export inside `--full`. `bash scripts/ui_check.sh` green; `--full` run once (after a fresh export) → green. Commit: `feat(ui-process): composed-export smoke gate — production-path PNG presence/non-blank (alignment plan T7)`.

### Task 8: Perf baseline-provenance gate

**Files:**
- Create: `StarBreaker/scripts/perf_provenance.sh`
- Modify: `StarBreaker/.claude/skills/starbreaker-optimisation/SKILL.md`

**Interfaces:**
- Produces: `bash scripts/perf_provenance.sh <binary>` → prints `path`, `size`, `mtime`, `sha256`, current `git rev-parse HEAD`. Marker `perf_provenance: OK`. (A predates-newest-commit staleness warning is optional — add only if it fits in ~1 line; sha256+mtime+HEAD on both endpoints already catches the stale-baseline class.)

- [ ] **Step 1:** Failing `--self-test` (nonexistent binary → FAILED; real binary → OK with all fields) → implement → PASS.
- [ ] **Step 2:** Skill edit — add to the optimisation skill's measurement rules: *before ANY before/after timing claim, run `perf_provenance.sh` on BOTH endpoints' binaries and include both outputs; mismatched/unknown provenance invalidates the comparison* (this is the stale-42s-baseline lesson as a gate). Keep it ≤6 lines; place beside the existing "same binary" rule.
- [ ] **Step 3:** Verify-on-write (run the doc's command once). `ui_check.sh` green. Commit: `feat(process): perf baseline-provenance gate + optimisation skill rule (alignment plan T8)`.

### Task 9: Skill folds — blockers/measurement/waiting rules

Folds the ledger-107 candidates + wait-on-marker into the parity skill references, via the mandated skill-editing process.

**Files:**
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/references/blockers.md`
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/references/catalog.md`
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/SKILL.md` (only if a core line is needed; keep ≤220 lines)
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/recommendations.md` (Change log; clear applied Open items)
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-workflow.md` §3 (wait pattern, one block)

- [ ] **Step 1 (audit-then-add):** Read `recommendations.md` Open items AND the current `blockers.md` in full first — blast-radius guidance already exists there (verified at ~lines 8/17/105/140/142, plus a fresh-export note ~line 93). Then fold ONLY the genuinely missing pieces: (a) **fresh-export reproduction gate** — owner-reported symptom must reproduce in a FRESH export before building any fix; present fresh-export evidence and confirm with the owner otherwise → `blockers.md`; (b) blast-radius-by-change-class: add at most a one-line cross-reference if the A6/108 fold already covers it — do NOT restate; (c) **thin-glyph colour by near-black pixel COUNT, not mean** → `blockers.md` measurement list; (d) **fixed-size vs width-fit check before any "size is correct" claim + sibling cap-height ratio + owner-repeated visual complaint = ground truth, re-derive from scratch** → `catalog.md` (measurement/self-verify section). Also extend the evidence bar beyond "blocked": a **"done" or "within tolerance" verdict at the close/catalog gate needs the same class of evidence** (measurement vs the residual-budget classes, once T11 lands) — one line in `catalog.md`'s closing re-review, so the spec's §4A1 extension isn't dropped.
- [ ] **Step 2:** Add the **wait-on-marker rule** to `ui-workflow.md` §3 (one block): long-running commands (`ui_render.sh`, exports, `--full`) run via the harness background facility with completion detected by the command's RESULT MARKER in its log file — never sleep-loops, never turn-holding, never a regex looser than the exact marker string. Cross-reference from the skill core loop if a line fits within the cap.
- [ ] **Step 3:** Invoke `superpowers:writing-skills` to review the edited skill files; apply findings. Record applied items in `recommendations.md` Change log (dated entry) and clear them from Open.
- [ ] **Step 4:** Verify every referenced tool/file exists (`ui_blocker_evidence.py`, tint-semantics test name, etc.). `ui_check.sh` green. Commit: `docs(skill): fold ledger-107 lessons + wait-on-marker into parity skill (alignment plan T9)`.

---

## Phase P2 — Knowledge-store reconciliation (docs-only; single writer)

### Task 10: The goal's WHY, in the repo

**Files:**
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-workflow.md` (top, after the title)
- Modify: `StarBreaker/crates/starbreaker-ui/AGENTS.md` (top)

- [ ] **Step 1:** Add this paragraph (owner's words, 2026-07-07) to both files, adapted to context: *"Purpose: produce near-pixel-perfect replicas of the in-game UI as STATIC images so the exported Blender files look as close to the in-game environments as possible. The renders are baked as textures onto screen meshes during `entity export` — this crate is a texture source for the Blender export, not a UI runtime. Interactivity/animation matter only insofar as they determine a screen's correct static visual state. Direction (2026-07): scale arcs across many ships once the process is proven; public release eventually; residual bar codified in ui-residual-budget.md; fully-auto arcs on known ships."*
- [ ] **Step 2:** Grep both files for now-contradicted phrasing (e.g. anything implying runtime goals) — none expected; fix if found. Commit: `docs(ui): state the crate's purpose and owner direction at the top of workflow/AGENTS (alignment plan T10)`.

### Task 11: Residual-budget + capture-provenance doc (owner-gated content)

**Files:**
- Create: `StarBreaker/crates/starbreaker-ui/docs/ui-residual-budget.md`
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/references/catalog.md` + `references/blockers.md` (one pointer line each)

- [ ] **Step 1 (research):** Harvest every deferred/known-outlier residual class from: the known-outlier register (workflow §6), the dossier open-issues column (ui-reference §3), ledger items 35, 68c, 107 (bloom, 2px rectification smear, hover artifacts, CRT curvature, bezel/corner triangles, aspect skew), and the interim B4 lenient-colour rule. Also pull the tier whole-image budgets from `ui-regression-policy.md`.
- [ ] **Step 2:** Draft the doc: table of residual classes — class, cause (capture vs renderer), status (`permanent-acceptable` / `lenient-until-B4` / `must-fix`), detection method (which measurement distinguishes it), and the per-tier numeric budgets it interacts with. Plus a §capture-provenance: each reference capture records game version/date/settings when known; known capture error classes; the rule that a capture-imperfection claim needs a distinguishing measurement (not an eyeball).
- [ ] **Step 3 (GATE):** Present the draft classification table to the owner via AskUserQuestion (which classes are permanent-acceptable?). Apply answers.
- [ ] **Step 4:** Add pointer lines in catalog.md ("classify each catalog item against ui-residual-budget.md before calling it within-tolerance") and blockers.md. `ui_check.sh` green (docs_reference_guard). Commit: `docs(ui): residual-budget + capture-provenance doc, owner-approved classes (alignment plan T11)`.

### Task 12: Hard-coding discriminator codified

**Files:**
- Modify: `StarBreaker/AGENTS.md` (§Coding Practices hard-coding bullet)
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-fallback-register.md` (header + new row)

- [ ] **Step 1:** Add the discriminator to AGENTS.md (3–4 lines): *a numeric constant is a LICENSED host-side fallback only when (a) proven absent from DataCore/P4K/AVM1 by an exhausted search (blocker-evidence style), (b) registered in ui-fallback-register.md with owner, trigger signal, and sunset target. Anything plausibly data-sourced is a violation to fix or flag. The two treatments are the same rule at different evidence levels.*
- [ ] **Step 2:** Mirror the same wording in the fallback register's header; add a register row for the `compose/text_draw.rs:100-102` heading table (`Heading1=>48, Heading3=>28`) as **unlicensed — pending derivation** with trigger = the SUB DECK font arc (owner-deferred) — the code fix itself is OUT of this plan's scope (it belongs to that arc; ledger 107 owns the trail).
- [ ] **Step 3:** Repo-grep for other size-table literals in `compose/` (`grep -rn "=> *[0-9][0-9]" crates/starbreaker-ui/src/compose/ | grep -iv test`) — add any hits to the register the same way (flag, don't fix). Commit: `docs(ui): codify the host-constant vs game-data discriminator; register heading-table debt (alignment plan T12)`.

### Task 13: Ledger current-truth index + retro-prompt alignment

**Files:**
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-process-improvements.md` (head + 2 one-line insertions)
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-process-retro-prompt.md`

- [ ] **Step 1:** Insert at the ledger head (after the existing header) a "**Current-truth index**" (~20 lines): (a) overturned entries → superseding entry: 60→68/108, 66→76, 67d→68, 75→76, 94→96/97 (verify each by reading the entries; add any others found by `grep -n "OVERTURNED\|overturned" `); (b) numbering notes: two entries share №19 (Part C vs Part D — cite both line numbers); 99–100 unused — never renumber, numbers are provenance anchors; (c) genre note: Parts A–E are history + executable plans of their era; from ~52 the ledger is a retro journal — live architecture backlog lives in `ui-architecture-runbook.md`.
- [ ] **Step 2:** Insert one-line forward pointers at the top of entries 75 and 67d (mirroring the pointer style entry 94 already has): `> ⚠️ OVERTURNED — see item 76 (authored FontSize was unapplied, not a scale blocker).` etc.
- [ ] **Step 3:** Retro-prompt: replace the "EXTEND its phased plan" instruction with: append the retro entry; **default action for any recurring lesson = convert it into an enforced gate/test/probe/tool** (cite ledger 108 as the pattern); a prose bullet is the last resort and must say why a gate isn't possible. Keep the rest.
- [ ] **Step 4:** `ui_check.sh` green (ledger is scanned by docs guards — confirm no guard keys on entry numbering). Commit: `docs(ui): ledger current-truth index + overturn pointers; retro default = build a gate (alignment plan T13)`.

### Task 14: Governance batch

**Files:**
- Modify: `~/projects/scorg_tools/AGENTS.md` (workspace — NOT in git; no commit)
- Modify: `StarBreaker/AGENTS.md`
- Modify: `StarBreaker/crates/starbreaker-ui/AGENTS.md`
- Modify: `StarBreaker/blender_addon/AGENTS.md`

- [ ] **Step 1 — workspace AGENTS.md:** fix branch note to `feature/ui`; change SC_DATA_P4K wording to "auto-detected from the default install path; set `SC_DATA_P4K` only for non-default installs" (keep the canonical command but mark the env prefix optional); scope the todo.md sentence to "the phased plan for EXPORTER/BLENDER work; UI work plans live in `StarBreaker/docs/superpowers/plans/` + the UI ledger"; redraw the `ships/` layout tree from actual `ls ~/projects/scorg_tools/ships/Packages/` output (verify-on-write; remove the double-nested example from the diagram — keep the warning text).
- [ ] **Step 2 — StarBreaker/AGENTS.md:** delete the "Delegating Phases to Sub-Agents" section (~105 lines) and replace with ~10 lines: sequential cargo builds (shared target races); sub-agents default Opus, read-only research parallel-safe; self-review against repo conventions before reporting done; `cargo build` (debug) + `cargo test --workspace` as the completion check (NOT --release); no Co-Authored-By trailers. Then add to §Git: the no-trailer rule and "work happens directly on `feature/ui`; never create branches/worktrees unless the owner asks".
- [ ] **Step 3 — crates/starbreaker-ui/AGENTS.md:** remove `.github/copilot-instructions.md` from required-reads (leave the file itself; it serves Copilot) — replace with one line: "(.github/copilot-instructions.md duplicates this file for Copilot users — do not read both)".
- [ ] **Step 4 — blender_addon/AGENTS.md:** fix the TEX0/TEX2 garbled block (keep ONLY the "Always use TEX0 for validation" rule); keep one canonical scene-reset block and replace the other two with one-line cross-references; replace the hard test-count baseline with "the suite must pass with zero failures; skips are bpy-only tests" .
- [ ] **Step 5:** Repo-grep every renamed/removed phrase for dangling references (`grep -rn "starbreaker-exporter\|Delegating Phases" . --include=*.md`). `ui_check.sh` green. One commit for the three in-repo files: `docs: governance reconciliation — build rules, delegation section, required-reads, addon fixes (alignment plan T14)`.

### Task 15: Skill SCOPE&MODE computed default + persona reconciling lines

**Files:**
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/references/launch.md`
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/SKILL.md` (strict rules: +2 lines)
- Modify: `StarBreaker/.claude/skills/starbreaker-tint-palette/SKILL.md` (+2 lines)
- Modify: `StarBreaker/.claude/skills/starbreaker-optimisation/SKILL.md` (+1 line, it already names its superpowers sub-skills)

- [ ] **Step 1 — launch.md:** in the SCOPE&MODE question, add the computed recommendation: *"known ship/screen" = its dossier row is complete (preset, tier, target_id non-null) AND the manufacturer already has ≥1 frozen GOLD/PLATINUM screen → recommend fully-automated; otherwise (first-of-a-kind: new manufacturer, new widget family, no dossier row) → recommend semi-automated. Present the recommendation as the first AskUserQuestion option per owner direction 2026-07; the ASK itself never disappears.*
- [ ] **Step 2 — persona lines** (same wording in parity + tint SKILL.md strict rules; optimisation gets only the ponytail line): (a) *"Ponytail note: in this repo the laziest solution that works = the smallest change that is still engine-faithful and data-derived. A hard-coded value, invented geometry, name-gate, or heuristic is never 'the lazy solution' here — it trips the guards."* (b) *"This skill IS the process skill for its arc — its launch/catalog gates stand in for superpowers:brainstorming's design gate; superpowers planning/TDD/debugging skills are sub-tools invoked WITHIN the loop."*
- [ ] **Step 3:** Invoke `superpowers:writing-skills` for review; apply findings; confirm SKILL.md stays ≤220 lines (`wc -l`). Record in `recommendations.md` Change log. Commit: `docs(skill): dossier-computed mode default + persona reconciliation lines (alignment plan T15)`.

### Task 16: Graphify sync + ledger entry + plan into repo

**Files:**
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-process-improvements.md` (append entry)
- Create: `StarBreaker/docs/superpowers/plans/2026-07-07-system-alignment-plan.md` (copy of this file)
- Create: `StarBreaker/docs/superpowers/specs/2026-07-07-system-alignment-review.md` (copy of the review)

- [ ] **Step 1:** Copy this plan + the review doc into the repo paths above; strip nothing (they already follow the no-names/no-home-paths rules — grep both for `/home/` and the owner's name to verify; fix `$HOME`-ify any hits).
- [ ] **Step 2:** Append the next numbered ledger entry (Observed/Improvement/Action format): the alignment review + P0–P2 landings, citing commits of Tasks 4–15 and this plan's repo path.
- [ ] **Step 3:** Run `/graphify . --update` (doc sync — dispatches extraction subagents). Confirm it reports the changed docs ingested.
- [ ] **Step 4:** `ui_check.sh` green. Commit: `docs(ui): ledger — system alignment P0-P2 landed; plan+review archived in-repo (alignment plan T16)`.

---

## Phase P3 — Measurement & validation

### Task 17: Arc-cost instrumentation

Makes Workstream A's "cheaper arcs" measurable — mandatory now that the direction is scale-across-ships.

**Files:**
- Create: `StarBreaker/crates/starbreaker-ui/data/arc_cost_log_v1.jsonl`
- Create: `StarBreaker/crates/starbreaker-ui/data/arc_cost_log_v1.notes.md` (provenance sidecar per registry pattern)
- Modify: `StarBreaker/.claude/skills/starbreaker-ui-screen-parity/references/retro.md`

**Interfaces:**
- Produces: one JSON line per arc appended at retro time: `{"date":"YYYY-MM-DD","screen_id":str,"ship":str,"mode":"semi|full","bootstrap_lines_read":int,"loop_cycles":int,"tool_calls_per_cycle_median":int,"wall_clock_min":int,"gates_fired":[str],"froze":bool,"notes":str}`. Consumed by future scaling decisions (target from plan A: bootstrap ~600 lines, 1–2 tool calls/cycle).

- [ ] **Step 1:** Create the empty JSONL + notes sidecar (source: alignment plan T17; fields defined above; append-only; numbers are the executing agent's own count — bootstrap_lines_read = total doc/skill lines read before the first render).
- [ ] **Step 2:** Add to `references/retro.md` a mandatory retro step: append the arc's line to the log (show the exact JSON shape); the A-plan targets (~600 bootstrap lines, 1–2 calls/cycle) are the comparison bar.
- [ ] **Step 3:** Validate the shape: `python3 -c "import json;[json.loads(l) for l in open('crates/starbreaker-ui/data/arc_cost_log_v1.jsonl')]"` (empty file OK). `ui_check.sh` green. Commit: `feat(ui-process): arc-cost log + mandatory retro instrumentation (alignment plan T17)`.

### Task 18: Frozen-target drift audit (one-time, report-only)

Are the certified screens still faithful TO THEIR REFERENCES today (not just to their frozen baselines)?

**Files:**
- Create: `~/projects/scorg_tools/docs/StarBreaker/2026-07-XX-baseline-drift-audit.md` (workspace report; date at execution)

- [ ] **Step 1:** Fresh export of every ship with frozen targets (currently Clipper; derive the list from the dossier's tier column — `python3 -c` over `crates/starbreaker-ui/data/ui_screen_dossier_v1.json`, rows where tier is non-null). Canonical export command, LOD0/MIP0.
- [ ] **Step 2:** `bash scripts/ui_check.sh --full` unpiped → record the marker. Any non-green = STOP, report to owner before continuing.
- [ ] **Step 3:** For each tiered dossier row: `bash scripts/ui_arc_status.sh --screen <screen_id>` → capture the per-region table (means, ratios, bank targets). This compares render ↔ REFERENCE, which the freeze guards don't do.
- [ ] **Step 4:** Compile the report: per screen — regions within bank targets? any region drifted vs the measurement bank? classify each residual against `ui-residual-budget.md` (Task 11). NO freezes, NO fixes — findings only; owner adjudicates.
- [ ] **Step 5:** Deliver the report to the owner; file any confirmed must-fix drift as arc candidates in the dossier open-issues column (docs commit only if the owner confirms).

### Task 19: A10 — Carrack execution-test arc (pointer)

Run per the 2026-07-04 plan's Task A10, unchanged: NEW session on Opus, invoke `StarBreaker:starbreaker-ui-screen-parity`, expected SHIP=Carrack / SCREEN=the unworked screen — but ASK per the skill; adding the dossier row is part of the arc. The arc's mandatory retro now ALSO appends the first `arc_cost_log_v1.jsonl` line (Task 17) — that line is the Workstream-A measurement this whole phase exists for. Prerequisites: Tasks 4–9 landed (the gates it exercises), Task 17 landed. Do not run it from this plan; it is an arc, owned by the skill.

---

## Self-review record

- Spec coverage: review §5 P0 (3 items) → T1–T3; P1 (7 items) → T4 (marker), T5 (vacuous), T6 (presence+policy), T7 (smoke), T8 (provenance), T9 (reproduction gate + wait-on-marker; folded together since both are skill/doc folds); P2 (7 items incl. owner-direction additions) → T10 (goal), T11 (residual+capture QC merged per review §6.3), T12 (discriminator), T13 (ledger+retro-prompt), T14 (governance), T15 (mode default + persona lines), T16 (sync+archive); P3 (3 items) → T17–T19. Review §4D harness items → T2/T3/T15. Nothing in §5 is unowned.
- Deliberately out of scope: the `text_draw.rs` heading-table CODE fix (owner-deferred arc, registered as debt in T12); B1–B4 (already in flight); graphify-doc-sync Sonnet-vs-Opus (global skill — flagged to owner in the review, not actioned).
- Placeholder scan: no TBDs; every step names exact files/commands; code shown only where the shape is load-bearing (marker format, JSONL schema, hook logic, discriminator wording) per the owner's "don't write all the code" instruction — executors TDD the implementations.
- Consistency: marker file path `.git/ui-check-marker` used in T4 and T18; `ui-residual-budget.md` name used in T10/T11/T18; `arc_cost_log_v1.jsonl` fields match between T17 interface and T19; dossier field names (`preset`, `tier`, `target_id`) match the A1 schema.
- External review (fresh-eyes Opus, 2026-07-07): APPROVE-WITH-FIXES, 9 findings, all applied — `--full` does not re-export (T7 corrected); reuse `benchmark_ui_only_export.sh` (T7); T6 pinned to `manifest_live_ir_guard.rs` inputs/skip-path + non-vacuous assertion; T4 hook chaining dropped (no existing pre-commit — verified) and marker construction moved into the EXIT trap for both outcomes; ponytail level is global state, owner told before change (T2); T9 made audit-then-add to avoid duplicating existing blockers.md guidance and extended the evidence bar to "done"/"within tolerance" verdicts; T8's staleness heuristic made optional. Reviewer verified: all named repo paths exist, dossier fields present in all 18 rows, `--ui-only-files` exists, `text_draw.rs:100-102` heading table confirmed.
