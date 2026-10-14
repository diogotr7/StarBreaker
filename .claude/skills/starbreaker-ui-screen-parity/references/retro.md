# RETRO — self-improve every arc (phase 6, MANDATORY closing step)

**The arc is not done when the catalog is — it is done AFTER the retrospective.** Run
it in the SAME session (lived context), in BOTH modes, before declaring complete. It is
the task-list item (TodoWrite where available) added at arc start; do not close the arc
with it open.

**The sweep and destinations are the shared arc-closeout skill — invoke it.** Its
sweep categories 1-5 (repeated work → tooling; silent failures → loud; doc drift →
verify-on-write; dead-ends/levers → recorded; bootstrap cost → owning doc) plus these
UI-specific categories, numbered on from arc-closeout's list:

6. **Guard/freeze friction.** Detoured adjudications, hand-audited deltas, late-registered
   outliers → improve the flow or the doc that teaches it.
7. **Handoff quality.** Would the in-repo handoff doc resume cold? Fix the `ui-workflow.md`
   §9 expectations, not just this arc's file. (Durable facts go in the ledger, never
   user-folder memory.)
8. **Slow tooling → profiled speedup.** A tool you waited on repeatedly: MEASURE first
   (usually harness load, not a loop), prefer an existing faster path, then cut the
   dominant cost; VERIFY the output is unchanged.

UI-specific destination details: the ledger is
`crates/starbreaker-ui/docs/ui-process-improvements.md` (Observed/Improvement/Action;
append via `scripts/ledger_append.py`); bootstrap-cost items land in `ui-reference.md`
(dossier / probe registry / glossary) or `ui-workflow.md` §10; doc fixes get repo-wide
grep for renamed references (`docs_reference_guard` covers new file-citing docs);
`bash scripts/ui_check.sh` green per commit; process changes must not alter render
behaviour; baseline-affecting actions stay APPROVAL-GATED (the freeze gate).

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
- **Comparison bar** (targets from `docs/superpowers/specs/2026-07-04-ui-parity-process-and-crate-design.md`,
  "A success criteria"): `bootstrap_lines_read` ~**600**, `tool_calls_per_cycle_median` **1–2**.
  A line above the bar is a signal to feed the next process retro, not a failure.

Append it, then confirm the file still parses (empty file is valid):

```bash
uv run --no-project python -c "import json;[json.loads(l) for l in open('crates/starbreaker-ui/data/arc_cost_log_v1.jsonl')]"
```

Do not close the arc with this line unwritten (it is part of the same mandatory retro
task-list item). The log was still empty at the 2026-10-14 docs audit — if no earlier
line exists, yours is the first; that is not a reason to skip it.

Acceptance (bootstrap test): a fresh agent could run the next arc from `ui-workflow.md` +
`ui-reference.md` + the dossier alone. Any excursion you needed is a doc bug — fix it
before closing.

## Red flag — retro

| Thought | Reality |
|---|---|
| "Catalog's resolved / found more issues — run the retro" | The retro is the LAST step, never a substitute for fixing. Fix fixable diffs in the loop first, then the Closing re-review (re-render + re-compare like the start; fully-auto keeps fixing until clean or proven-blocked), THEN the retro. Not complete until the re-review is clean/proven-deferred AND the retro runs (track as a task-list item from arc start). |
