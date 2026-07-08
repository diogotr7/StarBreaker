# arc_cost_log_v1.jsonl — provenance notes

Companion to `arc_cost_log_v1.jsonl` (JSONL carries no comments).

**What it is.** An append-only log of the *cost* of each UI screen-parity
arc — one JSON object per line, appended at retro time (the mandatory
closing step, `.claude/skills/starbreaker-ui-screen-parity/references/retro.md`).
It exists to make Workstream A's premise — *"cheaper arcs"* — falsifiable:
that justification was never measured, so "cheaper" was unverifiable. This
log is the measurement. Consumed by future scaling decisions (is the
per-arc cost actually trending down as the process/skill/crate work lands?).

**Source of the numbers.** The *executing agent's own honest count* for the
arc it just ran — not an estimate, not a target. Counted in the same lived
session as the arc (the retro runs in-session), so the agent has the real
figures in context.

**Append-only.** Never rewrite or reorder existing lines; only append. A
mistaken line is corrected by appending a follow-up line, not by editing
history (the log's value is that it is an untampered record).

## Field schema

One line per arc:

```json
{"date":"YYYY-MM-DD","screen_id":"str","ship":"str","mode":"semi|full","bootstrap_lines_read":0,"loop_cycles":0,"tool_calls_per_cycle_median":0,"wall_clock_min":0,"gates_fired":["str"],"froze":false,"notes":"str"}
```

| Field | Type | Meaning |
|---|---|---|
| `date` | `YYYY-MM-DD` string | arc completion date |
| `screen_id` | string | dossier `screen_id` (see `ui_screen_dossier_v1.json`) |
| `ship` | string | ship the arc targeted (e.g. `Carrack`) |
| `mode` | `"semi"` \| `"full"` | arc automation mode — semi-auto (stop-and-ask) vs fully-auto (keep-fixing-until-clean); the retro runs in BOTH, log the mode this arc actually ran in |
| `bootstrap_lines_read` | int | **total doc/skill lines read before the FIRST render of the arc** — the agent's own count of the doc/skill/reference lines it had to read to get to first render (the launch cost) |
| `loop_cycles` | int | number of render→compare→fix cycles run |
| `tool_calls_per_cycle_median` | int | median tool calls per loop cycle |
| `wall_clock_min` | int | wall-clock minutes for the arc |
| `gates_fired` | [string] | which guard/freeze gates fired (e.g. `["freeze","hardcoding-guard"]`) |
| `froze` | bool | did the arc reach a freeze |
| `notes` | string | one-line free-form (blockers, deferrals, anything skewing the numbers) |

## The comparison bar

The Workstream-A targets to compare each line against (from
`docs/superpowers/plans/2026-07-04-ui-parity-process-and-crate-plan.md`,
Task A10 Step 2):

- `bootstrap_lines_read` — target **~600** lines before first render.
- `tool_calls_per_cycle_median` — target **1–2** calls per loop cycle.

These are the plan's stated goals, not a floor; a line above them is a
signal, not a failure.

## Shape check

Validated with (empty file is valid):

```bash
python3 -c "import json;[json.loads(l) for l in open('crates/starbreaker-ui/data/arc_cost_log_v1.jsonl')]"
```

Runs clean (no output) when every line is well-formed JSON.

## Sunset

Keep. This is a permanent measurement series, not a pinned value awaiting a
derivation. It retires only if Workstream A's "cheaper arcs" premise is
abandoned.
