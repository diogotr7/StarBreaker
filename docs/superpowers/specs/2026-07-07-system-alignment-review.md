# System Alignment Review — 2026-07-07

Whole-system review of the scorg_tools / StarBreaker working environment:
what the project is ultimately trying to accomplish, which parts of the
agent-facing system (AGENTS/CLAUDE files, memory, skills, hooks, settings,
process docs) work against that goal, and what the 61 Claude sessions
(2026-06-08 → 2026-07-07) show about what worked and what didn't.

Method: 5 system audits + 10 chronological session-batch reviews + a merge
pass + an adversarial critique, all on Opus subagents (~2.7M tokens), plus
direct reads of the redesign spec/plan, ui-workflow.md, the retro ledger,
and git. Findings the critique judged weak are either dropped or marked.

---

## 1. The goal

**Owner's statement (2026-07-07):** "the overall goal of the UI crate is to
produce near pixel perfect replicas of the in-game UI as static images so
that the exported Blender files look as close to the in-game environments
as possible."

Characterization of the whole project, as evidenced by docs + sessions:

- **Workspace end state:** take Star Citizen assets out of the game and
  into Blender, engine-faithfully — geometry, materials, tints, lights,
  interiors, animations, and UI screens. The measure of success is that a
  Blender render of an exported ship/environment is visually
  indistinguishable from the in-game original.
- **The UI crate's role:** a *texture source* for `entity export`. Screens
  are baked as static PNGs onto screen meshes (54 PNGs for the Clipper).
  It is not a UI runtime; interactivity/animation matter only insofar as
  they determine the correct *static* visual state.
- **The means (non-negotiable ethos):** engine-faithful and generic — no
  per-ship/screen/manufacturer branches, no invented or hard-coded
  game-data values, everything derived from DataCore/P4K, fix the owning
  upstream stage, IR is the sole styling authority.
- **The second product:** the *process itself*. The ledger → skill →
  tooling loop (now 108 entries, the A1–A9 overhaul, the dossier,
  `ui_arc_status.sh`, `ui_variant_styles`) is deliberately building toward
  agents running parity arcs cheaply and autonomously, with the owner as
  the visual ground-truth arbiter at gates (catalog, freeze, blockers).

Status at review time: ~5 Clipper screens GOLD/PLATINUM + Carrack
lift-call/annunciator; Phase A landed; **B1–B4 all in progress**
(owner-confirmed); A10 (Carrack execution-test arc) not yet run.

Open goal questions only the owner can answer → §6.

## 2. The headline conclusion

The system is *directionally* healthy — the ethos is stated consistently
everywhere, and the trajectory across 61 sessions is a genuine maturation
from ad-hoc thrashing (blank MFDs shipped through 7 "green" mock-tested
phases in early June) to the most disciplined mode yet (gated
brainstorm→spec→plan→subagent execution in July). The single most
important finding is a **meta-pattern the ledger itself proves**:

> **Lessons stick only when they become enforced gates (a test, guard,
> probe, tool, or hook). Prose — §10 bullets, memory notes, skill
> red-flags — gets re-learned.** The font "proven blocker" recurred 4–5×
> after the prose fix (ledger 60→66→76→94→107); the stale-artifact trap
> recurred ~10× in evolving forms; the `| tail` exit-code mask recurred
> immediately after being documented (ledger 89). Every fix that stuck is
> mechanical: the freeze guard battery, TDD red-first, the byte-identical
> export gate, disable→adjudicate audits, `ui_variant_styles`.

The A1–A8 overhaul is the correct response to exactly this (it converted
the four worst prose lessons into tools/gates). It is, however, **unproven
until A10 runs** — and several recurring traps still live only as prose
(§4, P1).

## 3. What is working FOR the goal (keep, and lean harder on)

1. **The frozen GOLD/PLATINUM guard battery** (`ui_check.sh --full`,
   live-IR + whole-image + IR-snapshot) — caught essentially every
   over-broad shared-mechanism regression before shipping. Load-bearing.
2. **TDD red-first at the owning stage** + temporary-revert to prove the
   guard catches the bug.
3. **Data-tracing over theorizing** — reading the INSTANTIATED variant's
   authored entries dissolved every false size/colour "blocker";
   structural discriminators (`shrinkProportion==0`, `rendererType:None`,
   sole-top-level-WidgetCanvas…) fixed classes generically.
4. **Disable→adjudicate blast-radius audits** — measured impact repeatedly
   overturned "too risky" estimates.
5. **The byte-identical export gate** for optimisation work (surfaced the
   HashMap nondeterminism; held output identical through a 16.5× speedup).
6. **The ledger→tooling conversion loop** — the self-improving premise
   works *when the retro output is a tool* (A1–A9 is the exemplar).
7. **Owner steering style** — numbered defect lists, hand measurements,
   supplied suspect commits/reference implementations, high autonomy with
   hard gates. Sessions converge fastest when the agent mirrors this:
   measure, don't estimate; fix forward; ask only at genuine gates.

## 4. What is working AGAINST the goal (ranked)

### A. Process-level (highest impact, evidenced by sessions + ledger)

**A1. Premature "blocked / done / within-tolerance / inherent" verdicts.**
Dominant waste source across the whole period; overturned nearly every
time by one more query or the owner's "keep digging". The A5
blocker-evidence gate + A4 `ui_variant_styles` are the countermeasure —
**validate via A10 before trusting**, and extend the same evidence-file
bar to "done" and "within tolerance" claims, not just "blocked".

**A2. Shared-mechanism edits regress frozen siblings; the whole-image
guard is structurally blind below the tier % budget.**
~9 recurrences (ledger 52/59/67c/75/77/88/98/106). Nav arrows vanished
under a GREEN guard and the owner caught it by eye (77); the pivot fix
regressed countermeasures 3.87% (106). Directly relevant risk to B1 (one
resolver touching every frozen screen) and B4 (everything changes).
→ Fix is a *policy reconciliation*, not just a new check:
`ui-regression-policy.md:30` falsely claims the guard "catches any
rendered change on any screen" AND bans the targeted checks that would
close the gap. Add a **generic, IR-derived element-presence/coverage
guard** (derived per frozen target from its own IR snapshot — no
per-screen hand-authored ROI, so it satisfies the generic-not-targeted
principle) and correct the policy text.

**A3. Stale artifacts/binaries/views produce phantom bugs and false
baselines.** ~10 recurrences in evolving forms (stale MCP IR, cached
same-filename renders, cross-worktree cargo no-ops, a stale perf baseline
costing ~2 sessions, owner's own stale export in ledger 107). Each closed
form was replaced by a new form → needs a *general* provenance rule:
pin both endpoints' binary/export provenance before any before/after or
"regressed" claim; owner-reported symptoms must reproduce on a FRESH
export before any fix is built (ledger 107 lesson — fold into
`references/blockers.md` as a gate, it is currently a candidate note).

**A4. The agent's visual/spatial judgment is the weakest faculty and it
keeps *defending* wrong visual verdicts against the owner's eye.**
(Furore "matches exactly" → wrong font at half size; SUB DECK "within
tolerance" → 1.35× too small.) Standing rule to promote into the skill
core: **an owner-repeated visual complaint is ground truth — re-derive
from scratch with a two-method measurement; never re-defend the prior
call.** Route thin-glyph/small-feature claims to the ledger's own derived
methods (pixel-count, connected components, sibling cap-height ratios).

**A5. Idle background-polling as a token sink** (30+ no-op "holding"
turns in one session; Monitor false-match on a loose regex). Never
durably fixed. Needs a blessed wait-on-marker primitive in the loop docs.

**A6. No composed-export smoke gate on the actual deliverable.** The
product is baked PNGs in the decomposed export, but guards test the
renderer against fixtures/mocks — blank MFDs once shipped through 7 green
phases. One end-to-end check (fresh export → N expected PNGs exist,
non-blank, through PRODUCTION fetchers) closes the gap between "tests
green" and "the Blender export looks right".

### B. Governance files (stale/contradictory instructions loaded every session)

*(verified against filesystem + git by the audit)*

1. ~~MEMORY.md "redesign GATED" index line~~ — **fixed this session**
   (was the most dangerous line in the stack; an agent trusting it would
   refuse legitimate work. The `-gated` slug still misleads — rename.)
2. `StarBreaker/AGENTS.md` "Delegating Phases" section (~105 lines):
   generic imported boilerplate that contradicts the file's own rules —
   mandates `cargo build --release` per phase vs "use debug, NOT
   --release" (lines 25–28); "≥5–6 unit tests" vs "tests track behaviour,
   not lines"; describes SQL/spreadsheet tracking nobody uses; omits the
   Opus-subagent directive. Cut to ~10 lines keeping the real lessons
   (sequential builds, self-review, Opus, no co-author trailer — promote
   the trailer rule to §Git).
3. Workspace `AGENTS.md`: names dead branch `starbreaker-exporter`
   (actual: `feature/ui` — the stay-on-branch rule lives only in memory;
   put it in the repo); claims `SC_DATA_P4K` is required (it
   auto-detects — three other sources say so); anoints `todo.md` (last
   touched 2026-06-01, phases out of order) as "live source of truth"
   while UI work actually plans in `docs/superpowers/plans/`; the layout
   diagram shows the double-nested anti-layout the same file forbids.
4. `crates/starbreaker-ui/AGENTS.md` requires reading
   `.github/copilot-instructions.md` (369 lines, ~90% duplicate of
   AGENTS.md) in every UI session — drop it or reduce it to a pointer.
5. `blender_addon/AGENTS.md`: garbled TEX0-vs-TEX2 self-contradiction;
   scene-reset rule stated 3×; test baseline 2× stale (says 165 tests,
   actual 370).
6. Memory index ~1/3 dead weight (superseded/DONE arc-status entries
   auto-loaded every session) — prune list in §5 P0.

### C. Knowledge-store hygiene (ledger + docs)

1. **Overturned-in-place ledger entries without forward pointers** —
   item 75 is falsified by 76 thirty lines later with no marker at 75; a
   grep-and-stop reader gets dead guidance. Add top-line pointers (like
   94 has) + a short "current truth" index at the head. (The critique
   downgraded "ledger too big" and "CI numbering check" as speculative /
   over-engineered — a lightweight index covers both; do note the
   duplicate item 19 and missing 99–100 when adding it.)
2. **Hard-coding ban applied contradictorily**: `ui-fallback-register.md`
   blesses HOST_CONTENT_INSET=44/annunciator 25px as "measured,
   proven-absent-from-data host-side constants" while ledger 107 flags
   the `text_draw.rs` Heading1=48/Heading3=28 table as an AGENTS.md
   violation — same class, opposite treatment. Codify the discriminator:
   *proven-absent-from-data host-side constant, registered in the
   fallback register with trigger + sunset = licensed; anything plausibly
   data-sourced = violation.* Apply it to the heading table (already a
   flagged TODO).
3. **The goal's "why" appears nowhere in the repo.** Every doc states the
   means (engine-faithful, generic) but not the end (baked textures so
   Blender exports match the game). One paragraph at the top of
   `ui-workflow.md` + the crate AGENTS.md prevents a whole class of
   mis-prioritization (e.g. an agent over-investing in runtime/interactive
   behavior, or under-valuing how screens look *in the exported scene*).
4. `ui-process-retro-prompt.md` still asks each arc to "extend the phased
   plan" — a genre the ledger abandoned at ~item 52. Align it, and make
   the retro's default action "convert the lesson into a gate/tool"
   (append-a-bullet = last resort).

### D. Harness (settings, hooks, plugins, skills)

1. **Missing allowlist entries cause prompt-stops in "fully-auto" arcs**:
   no `git` permissions at all, and none for `bash scripts/ui_check.sh`,
   `ui_render.sh`, `ui_arc_status.sh` — the exact per-cycle commands.
2. **Dead allowlist weight**: 6 hyper-specific one-offs (awk lines pinned
   to dead session paths, python one-liners already covered by broad
   prefixes) — delete.
3. **Ponytail** (always-on, level full, re-injected into EVERY subagent ≈
   1.5k tokens each → ~10–15k/arc): its "shortest diff / laziest solution"
   framing pulls against the owner's recorded "a longer route is not a
   problem if it does it a better way" and the no-shortcuts ethos. Its
   root-cause and anti-over-engineering content aligns well. Options:
   `lite` for this project, or keep `full` plus one reconciling line in
   the domain skills: *"here, the laziest solution that works = the
   smallest change that is still engine-faithful and data-derived; a
   hard-coded value or invented geometry is never it."*
4. **Superpowers**: `using-git-worktrees` / `finishing-a-development-branch`
   directly contradict the stay-on-feature/ui rule; the brainstorming
   HARD-GATE double-gates parity arcs whose launch/catalog gates ARE the
   design approval. Add a stand-in line to the three domain skills ("this
   skill IS the process skill for its arc; superpowers planning/TDD/
   debugging skills are sub-tools within the loop") — the optimisation
   skill already does this correctly; parity and tint do not.
5. **Zero-value plugins here**: frontend-design, playwright (+~30
   deferred tools), security-guidance; code-simplifier overlaps
   ponytail-review. Disable for this project.
6. **graphify-doc-sync mandates Sonnet extraction subagents**, contradicting
   the standing Opus-default preference — global skill, owner call.
7. Healthy: starbreakerMcp + graphify + blender MCP wiring, the graphify
   post-commit hook (verified rebuilding on today's commits), model/effort
   settings, skill symlinks (workspace → repo, no divergence).

## 5. Improvement plan

Ordered; respects that B1–B4 arcs are LIVE in the repo (no repo writes
from other sessions until they settle — P0 items below are all outside
the git repo or owner-approval actions).

**P0 — now, zero repo risk**
- [x] Fix MEMORY.md gate line + broken ledger link (done this session).
- [ ] Prune memory index dead weight (superseded/DONE entries:
  `font-sizing-constants-load-bearing`, `ui-improvement-plan-executed`,
  `medical-tint-fix-plan`, `power-screen-parity-plan`,
  `flash-hybrid-rendering-plan`, `widget-standard-expansion-landed`,
  `mfd-content-view-stage-subrect`, `mfd-aspect-tag-content-scaling`,
  `caps-reduction-removed`, medical bed/med2 status entries,
  `clipper-target-screen-parity` — archive, keep any line that still
  changes behavior). Rename `ui-parity-redesign-gated` →
  `ui-parity-redesign-status`.
- [ ] Settings: add `git add/commit/status/log/diff`, the three arc
  scripts to project allowlist; delete the 6 dead global entries;
  disable frontend-design / playwright / security-guidance (+ decide
  code-simplifier); decide ponytail level for this project.

**P1 — convert the still-prose traps into gates (queue behind live arcs;
each is small and most touch scripts/docs, not the renderer)**
- [ ] Pre-commit RESULT-MARKER check (greps `ui_check: ALL GREEN` from an
  unpiped run log; kills the ledger-89 trap mechanically).
- [ ] Shared guard precondition: every guard asserts `matched N>0`
  targets (kills the vacuous-guard class: ledger 3/61/105).
- [ ] Generic IR-derived element-presence guard per frozen target +
  correct `ui-regression-policy.md`'s false "catches any change" claim.
- [ ] Composed-export smoke test through PRODUCTION fetchers (fresh
  export → expected screen PNG count, none blank).
- [ ] Baseline-provenance gate for before/after perf comparisons (fold
  into starbreaker-optimisation skill).
- [ ] Fresh-export reproduction gate for owner-reported symptoms (fold
  ledger-107 candidate into `references/blockers.md`).
- [ ] Wait-on-marker primitive documented in the loop (kill idle polling).

**P2 — knowledge-store reconciliation (docs-only commits)**
- [ ] Goal paragraph (owner's words) atop `ui-workflow.md` + crate
  AGENTS.md.
- [ ] Residual-budget doc (owner decision §6.3): permanently-acceptable
  capture-imperfection classes vs must-fix, per tier; referenced by the
  skill's catalog/close phases so "within tolerance" has an objective bar.
- [ ] Skill SCOPE&MODE: compute the recommended mode from the dossier
  ("known ship" = row complete + manufacturer has a frozen screen →
  recommend fully-auto; first-of-a-kind → recommend semi) per §6.4 —
  keep the ASK, derive the default.
- [ ] Hard-coding discriminator codified in AGENTS.md + fallback
  register; heading-table TODO executed against it.
- [ ] Ledger: current-truth index at head; forward-pointers on overturned
  entries (75→76, 67d→68); retro-prompt updated (default action = gate,
  genre = retro journal).
- [ ] Governance fixes from §4B (root AGENTS.md branch/P4K/todo/layout;
  cut the Delegating-Phases section; ui-crate required-reads;
  blender_addon garble + stale baseline).

**P3 — measurement & validation**
- [ ] Run A10 (Carrack arc) WITH arc-cost instrumentation (bootstrap
  lines read, tool calls/cycle, tokens, wall-clock) → this is the missing
  baseline that makes Workstream A's "cheaper" measurable.
- [ ] One-time frozen-baseline drift audit: fresh export, render all
  GOLD/PLATINUM targets, compare against their frozen images — confirms
  the certified screens are still faithful today.
- [ ] Reference-capture QC note: capture provenance + known error budget
  (bloom, rectification smear, hover artifacts) — captures are ground
  truth but not perfect ground truth (ledger 35/68c).

## 6. Owner direction (interview answers, 2026-07-07)

1. **Breadth: SCALE.** After A10 validates the process, the point is
   coverage — cheap arcs across most/all flyable ships' screens.
   *Implications:* Workstream A cost-per-arc is the critical path; the
   arc-cost instrumentation (P3) is mandatory, not optional; the
   sibling-regression guard gap (§4 A2) grows linearly with frozen-target
   count, so the element-presence guard should land before mass scaling;
   ledger/memory leanness matters more (N ships × arcs of retro output).
2. **Audience: PUBLIC RELEASE eventually.** Packaging, docs, onboarding
   matter. *Implications:* the §4B governance staleness is release debt,
   not just agent friction; README/AGENTS accuracy and the no-names/no-home-paths
   rules are load-bearing; one flag to keep in view — distributing
   *outputs containing extracted game assets* has different ToS
   implications than distributing the tool itself.
3. **Residual bar: CODIFY.** New P2 item: a residual-budget doc defining
   permanently-acceptable capture-imperfection classes (CRT/bezel/bloom/
   aspect…) vs must-fix, per tier. Ends per-arc re-litigation and gives
   "within tolerance" claims an objective target. Complements the interim
   B4 lenient-colour rule (which graduates/closes at B4's re-freeze).
4. **Autonomy: AUTO FOR KNOWN SHIPS.** Fully-auto on process-proven
   ships/screen types; hands-on for first-of-a-kind (new manufacturer,
   new widget family). *Implication:* define "known" mechanically —
   e.g. dossier row complete (preset+tier+target_id) AND the manufacturer
   already has ≥1 frozen screen — and have the skill's SCOPE&MODE launch
   question present the computed recommendation as its default (keep the
   ASK, derive the default).

---

*Sources: 5 system audits + 10 session-batch reviews + merge + adversarial
critique (Opus subagents, workflow `wf_d4cf7369-1a4`), direct reads of the
2026-07-04 spec/plan, ui-workflow.md, ledger entries 100–108, git history,
and the owner's in-session goal statement. Session extracts:
condensed user/assistant text from all 61 transcripts.*
