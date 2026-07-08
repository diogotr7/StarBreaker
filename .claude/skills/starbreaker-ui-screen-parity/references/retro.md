# RETRO — self-improve every arc (phase 6, MANDATORY closing step)

**The arc is not done when the catalog is — it is done AFTER the retrospective.** Run
it in the SAME session (lived context), in BOTH modes, before declaring complete. It is
the TodoWrite item added at arc start; do not close the arc with it open.

Sweep this session's lived experience (friction, dead ends, retyped commands) across
these categories — for each, FIX it, don't just note it:

1. **Repeated manual work → tooling.** Anything typed >2× (ad-hoc crops, probe greps,
   command batteries) extends a `scripts/`/`examples/` tool (extend before creating).
2. **Silent failures → loud.** Any harness/guard that gave a wrong-but-plausible answer
   gets a distinct hard failure.
3. **Doc drift.** Every doc claim you relied on that was wrong/stale gets fixed with
   verify-on-write (run the command; repo-wide grep for renamed/deleted references in
   the same commit; `docs_reference_guard` covers new file-citing docs).
4. **Bootstrap cost.** Everything you had to RE-DERIVE (data locations, screen mappings,
   engine rules, probe names, don't-retry traps) lands in `ui-reference.md` (dossier /
   probe registry / glossary) or `ui-workflow.md` §10.
5. **Guard/freeze friction.** Detoured adjudications, hand-audited deltas, late-registered
   outliers → improve the flow or the doc that teaches it.
6. **Memory/handoff quality.** Would the handoff resume cold? Fix the §9 expectations, not
   just this arc's file.
7. **Slow tooling → profiled speedup.** A tool you waited on repeatedly: MEASURE first
   (usually harness load, not a loop), prefer an existing faster path, then cut the
   dominant cost; VERIFY the output is unchanged.

Two destinations:

- **Process / tool / doc findings →** APPEND numbered items to
  `crates/starbreaker-ui/docs/ui-process-improvements.md` (the ledger; Observed/
  Improvement/Action format) and IMPLEMENT them — quick tooling wins first, then docs;
  one commit per coherent item citing its ledger number; `bash scripts/ui_check.sh` green
  per commit; process changes must not alter render behaviour. Baseline-affecting actions
  stay APPROVAL-GATED (the freeze gate). The external prompt `ui-process-retro-prompt.md`
  is the canonical version of this sweep — run it verbatim if you prefer; the categories
  above are its essence so the skill is self-contained either way.
- **Improvements to THIS skill →** append under **Open recommendations** in
  `recommendations.md` (next to this file); do not rewrite `SKILL.md` mid-arc.

## Log the arc's cost (MANDATORY — the Workstream-A measurement)

Workstream A's whole justification is *"cheaper arcs"* — unverifiable until measured.
So every arc appends ONE line to `crates/starbreaker-ui/data/arc_cost_log_v1.jsonl`
(append-only; provenance + full field table in its `.notes.md`). The numbers are YOUR
OWN honest count from this lived session — not estimates, not the targets below.

Exact shape (one line, all fields):

```json
{"date":"YYYY-MM-DD","screen_id":"...","ship":"...","mode":"semi|full","bootstrap_lines_read":0,"loop_cycles":0,"tool_calls_per_cycle_median":0,"wall_clock_min":0,"gates_fired":[],"froze":false,"notes":"..."}
```

- `bootstrap_lines_read` = **total doc/skill lines you read before the FIRST render of
  the arc** (the launch cost). `mode` = the automation mode this arc actually ran in
  (semi-auto stop-and-ask vs fully-auto keep-fixing). `gates_fired` = which guard/freeze
  gates tripped.
- **Comparison bar** (targets from `docs/superpowers/plans/2026-07-04-ui-parity-process-and-crate-plan.md`,
  Task A10 Step 2): `bootstrap_lines_read` ~**600**, `tool_calls_per_cycle_median` **1–2**.
  A line above the bar is a signal to feed the next process retro, not a failure.

Append it, then confirm the file still parses (empty file is valid):

```bash
python3 -c "import json;[json.loads(l) for l in open('crates/starbreaker-ui/data/arc_cost_log_v1.jsonl')]"
```

Do not close the arc with this line unwritten (it is part of the same mandatory retro
TodoWrite item).

Acceptance (bootstrap test): a fresh agent could run the next arc from `ui-workflow.md` +
`ui-reference.md` + the dossier alone. Any excursion you needed is a doc bug — fix it
before closing.

## Red flag — retro

| Thought | Reality |
|---|---|
| "Catalog's resolved / found more issues — run the retro" | The retro is the LAST step, never a substitute for fixing. Fix fixable diffs in the loop first, then the Closing re-review (re-render + re-compare like the start; fully-auto keeps fixing until clean or proven-blocked), THEN the retro. Not complete until the re-review is clean/proven-deferred AND the retro runs (track as a TodoWrite item from arc start). |
