# UI parity process + crate convergence — design (2026-07-04)

Two approved workstreams:

- **A — the matching process:** cut the token and wall-clock cost of a
  parity arc run by a lesser agent (Opus/Sonnet), preserving every
  guardrail and every non-negotiable rule.
- **B — the ui crate:** converge `starbreaker-ui` toward the in-game UI
  system faster by landing the four evidence-backed architecture items.

## Constraints (owner-set, non-negotiable)

1. **Everything is GATED until the owner says go.** Another agent is
   mid-arc in this repo; no skill, script, doc-restructure, or crate
   change lands until the explicit go. This spec (and the implementation
   plan) are the only artifacts this session produces.
2. All existing rules stay in force unchanged: no hard-coding (including
   game-data values), no name/ship/screen gating, IR as sole styling
   authority, TDD, audited freeze flow, 3000-line cap.
3. **Guardrail-preservation invariant for Workstream A:** no rule is
   deleted. Every removed prose warning maps to either (a) a relocated
   phase-reference file, or (b) a tool that now hard-fails on the same
   mistake. The mapping is recorded in the change that moves it.
4. Research/read-only sub-agents default to **Opus** (currently achieves
   the same result with fewer tokens than Sonnet 5). Builds/renders/tests
   stay sequential in the main agent (shared cargo target).
5. Direct commits on `feature/ui`; no new branches or worktrees.

## Workstream A — matching process

### A1. Tooling (built first; prose deletions depend on it)

1. **Machine-readable dossier** —
   `crates/starbreaker-ui/data/ui_screen_dossier_v1.json` (follows the
   existing `data/*_v1.json` registry convention, with a `.notes.md`
   provenance sidecar): one entry per screen with ship folder,
   reference stem, scene package, helper, LOD, compare preset, frozen
   tier, open-issues pointer. Consumed by `ui_render.sh`,
   `ui_arc_status.sh` (below), and agents. The prose dossier table in
   `ui-reference.md` §3 becomes a generated/validated view of this file
   (single source of truth; a guard fails on drift). This also delivers
   multi-ship support: `ui_render.sh`'s scene paths are currently
   hard-coded to the Clipper packages and its LOD fallback is a
   name-pattern heuristic over Clipper screens — both break on the next
   ship (Anvil Carrack) and are themselves the banned name-gating shape,
   in tooling.
2. **`ui_render.sh` upgrades** — dossier-driven scene/LOD resolution;
   unique timestamped output filenames plus a `latest` symlink (kills the
   viewer-cache trap class); keep the md5 print and binary-mtime print.
3. **`scripts/ui_arc_status.sh`** — one command per loop cycle:
   export-staleness check → render (via the upgraded wrapper) → compare
   (`ui_compare.py` with the dossier preset + `--stats`) → structured
   per-region numeric summary, with settled numbers looked up from the
   measurement bank and per-region change flags vs the previous cycle.
   Prints a single RESULT MARKER line (same pattern as `ui_check.sh`) so
   piped/backgrounded runs can't mask failures. The agent vision-reads
   only regions flagged changed/new instead of every crop every cycle.
4. **`ui_variant_styles` MCP tool** (on the StarBreaker MCP server,
   beside the existing style trio) — given helper + node pattern: dump
   the INSTANTIATED
   variant's authored entries (`defaultStyles`, `brandStyles`, canvas-root
   `embeddedStyles` including bare `Type(Text)` selectors) matching the
   node, alongside the effective IR `text_style`, with an
   APPLIED / NOT-APPLIED verdict per authored value. This is the
   four-times-re-derived font-blocker investigation as one call.
5. **Blocker-evidence validator** — a small template + checker
   (`scripts/ui_blocker_evidence.py`): a blocker claim is a file listing
   each search surface (DataCore record families, P4K, localization,
   derivable-from-decoded-mechanism check) with the exact queries run and
   their results; the checker rejects empty or missing surfaces. The
   skill's blocker gate then requires a validated evidence file instead
   of restating the proof bar in prose.

Process-tooling rule carried over: none of A1 may alter render behaviour
(ledger: process changes must not change pixels).

### A2. Skill restructure (progressive disclosure)

`SKILL.md` 552 → ~180–220 line core that stays in context all arc:
mission + compressed strict rules, the launch flow, the loop as a
numbered checklist of tool invocations (`ui_arc_status.sh` at its
centre), the gates table, operating posture in ~10 lines, sub-agent
policy (read-only research, Opus default), and explicit
"STOP — read `references/<phase>.md` now" lines at phase transitions
pointing at:

- `references/launch.md` — input-question protocol (SHIP → SCREEN →
  REFERENCE sequential, ≥2-options padding, scope & mode).
- `references/catalog.md` — build + self-verify (look-again,
  background/backplate) + user confirmation gate.
- `references/blockers.md` — the proof bar, the font drill
  (`ui_variant_styles` first), shared-mechanism rules (asset/icon/binding
  AND layout/render formula ⇒ `--full` after fresh export + sibling
  eyeball), instrument-resolved-geometry-not-hand-solve note.
- `references/freeze.md` — freeze / dry-freeze-first no-op check /
  known-outlier flows.
- `references/retro.md` — the 7 sweep categories + two destinations.

The ~30-row red-flag table is distributed to the phase file where each
row bites; the core keeps only the rows guarding the loop itself
(~8). The two open 2026-07-03 recommendation candidates are folded in
(shared layout-formula = shared mechanism; probe resolved geometry).
`recommendations.md` keeps its lifecycle and records this restructure in
its Change log.

### A3. Doc bootstrap chain

`ui-workflow.md` and `ui-reference.md` stay authoritative but each gains
a ~10-line "read WHEN" index at the top; the skill's required-reads
contract becomes "read both indexes, then sections as directed by the
phase you are in". Target: cold-start reading drops from ~2,100 lines to
roughly 600 before the first render.

### A4. Validation (gated)

1. **Compliance dry-run (cheap):** fresh Opus subagent, scenario "Drake
   Clipper / Screen_Right_Upper_RTT", must — from the restructured skill
   + docs alone — resolve the reference path, read the right sections at
   the right phases, find the dossier row, respect the gates, and know
   the retro is mandatory. No builds/renders. Any miss is a skill bug to
   fix before go.
2. **Execution test:** the next real arc — the not-yet-worked **Anvil
   Carrack screen** — run on Opus under the new skill; findings feed the
   existing retro loop. This arc also exercises the multi-ship dossier
   path end-to-end (new ship folder, new scene packages).

### A success criteria

- Bootstrap context before first render: ~2,100 → ~600 lines read.
- Loop cycle: 4–6 tool calls + all-crops vision reads → 1–2 tool calls +
  changed-region reads only.
- Zero guardrail regressions: hardcoding guards, freeze gates, blocker
  gates all still bind (dry-run + Carrack arc verify behaviourally).

## Workstream B — ui crate convergence

Sequencing principle: pixel-neutral refactors first while baselines are
stable; the everything-changes re-freeze happens once, near the end.

1. **One brand-context resolver** (pure refactor, target zero pixel
   change). Extract a single resolver — canvas style-link →
   `s_<mfr>_{hud|env}` by canvas family → sibling swap; identity matching
   only, no prefix scans over shared standards — and migrate the four
   existing selection paths (`resolve_brand_style` manufacturer-prefix
   scan, `collect_standard_text_styles` family mapping, body-background
   preferred chain, separator `hud`↔`env` swap) one call site at a time
   under the guards, with the disable→adjudicate audit + `--full` at each
   step. Lands before Carrack multiplies brand contexts.
2. **Binding/state fidelity** (small, individually-guarded behavioural
   fixes): per-host-type expansion **ID-band lanes** in
   `merge_child_scene` (stops frozen-identity theft; unblocks the parked
   separator-dots work); `bb_state_filter` **derived numeric at-rest
   values** scoped to component-local bare bindings (ledger 87/88);
   **text-format route at all cascade tiers** (ledger 96 covered
   Brand+Embedded; complete the tier model so the
   authored-but-unapplied class cannot recur).
3. **Flash/SWF hybrid completion** (additive): finish the approved phased
   plan — Furore font from the `fonts_en` SWF, MFD footer in frame
   chrome, 4:3 frame vs 16:9 content composition — generically per the
   hybrid architecture notes.
4. **Linear-light compositing** (last; a dedicated owner-gated arc):
   renderer-wide linear blending with no image-only carve-outs, one
   audited full re-freeze of every gold/platinum target with per-identity
   adjudication. Existing evidence: predicted linear blends land on the
   reference values the current sRGB blend misses.

Interim rule: until item 4 lands, colour calls in new arcs (Carrack) are
judged leniently where the residual is blend-shaped, and recorded as
known-outliers pointing at the linear-light arc rather than chased with
per-screen fixes.

### B success criteria

- Items 1–3: `ui_check.sh --full` green after fresh export at each
  boundary; freezes only where owner-approved; every fix carries its
  regression guard.
- Item 4: full re-freeze approved by the owner with per-identity deltas
  explained; post-arc reference comparisons move toward the captures on
  the screens the evidence predicted.

## Overall sequencing

Workstream A first (it makes every subsequent arc — including all of B —
cheaper), then B1 → B4. The Carrack arc doubles as A's execution test
and may interleave with B1/B2 at the owner's discretion.

## Risks & mitigations

- **A lesser agent skips a phase-reference read** → core has explicit
  STOP lines at transitions; the dry-run tests exactly this; the retro
  loop catches residuals.
- **dossier.json drifts from prose** → the §3 table becomes a
  generated/validated view; a guard fails on drift.
- **`ui_arc_status.sh` masks a failure** → RESULT MARKER pattern
  (mandated by the same rule as `ui_check.sh`), never bare exit codes.
- **B1 refactor moves pixels** → migrate one call site per commit under
  the guards; disable→adjudicate + `--full` per step; any drift is a stop.
- **Linear-light partial application** → forbidden by design (no
  carve-outs); it ships as one arc or not at all.
