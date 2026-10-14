# starbreaker-ui crate instructions

Scope: everything under `crates/starbreaker-ui/`. Read this after the repo-level
`AGENTS.md` and before planning or editing work in this crate.

## Purpose (why this crate exists)

Produce near-pixel-perfect replicas of the in-game UI as **static images**, so
the exported Blender files look as close to the in-game environments as
possible. The renders are baked as textures onto screen meshes during
`entity export` — **this crate is a texture source for the Blender export, not
a UI runtime.** Interactivity and animation matter only insofar as they
determine a screen's correct static visual state. The Core rules below are the
*means*; static-visual fidelity is the *end*. Owner direction (2026-07): scale
parity arcs across many ships once the process is proven; a public release
eventually; the residual (acceptable-gap) bar is codified in
`crates/starbreaker-ui/docs/ui-residual-budget.md`; fully-auto arcs on known ships.

## Required first reads

1. The repo-level `AGENTS.md`, then this file.
2. For any UI parity/matching task: `crates/starbreaker-ui/docs/ui-workflow.md` (the process) and
   `crates/starbreaker-ui/docs/ui-reference.md` (commands, tools, data, and the §3 per-screen
   dossier). Read each doc's **"Read WHEN" index** up front, then full sections
   as the index or the parity skill's current phase directs. ui-workflow §1
   (non-negotiable rules) is always a full read before any fix.

(`.github/copilot-instructions.md` duplicates this file for Copilot users — do
not read both.) Re-read `crates/starbreaker-ui/docs/ui-workflow.md` when switching screens, after long
detours, or when visual fixes stop converging. To launch a per-screen parity
pass, use `crates/starbreaker-ui/docs/ui-matching-agent-prompt.md`.

## Core rules

- No hard-coding, no name-matching, no ship-specific branches, and no screen-specific branches in production code.
- No heuristic placement rules, blend factors, hand-tuned percentages, or magic offsets unless the surrounding rule is already structurally defined by source data and the new math is derived from that data.
- **No hard-coded game-data VALUES either** — palette colours
  (`RgbaColor { r: .., g: .., b: .. }` literals), font sizes, brand font
  lists, layout constants. This includes "fallback" palettes: an invented
  colour is hard-coding even when labelled non-authoritative. Resolve from
  DataCore/P4K, or use a provenance-noted extracted fixture (registry
  pattern) where tests need real values offline. The
  `rgba_colour_literals_are_not_hardcoded` test
  (tests/source_hardcoding_guards.rs) enforces
  this crate-wide — including test fixtures: a real palette value copied
  into a test is still a copy; load it from the extracted fixture instead.
  Genuinely arbitrary test colours must be neutral (pure black/white) or
  carry a `// hardcoding-guard: synthetic` annotation.
- **Self-correction is mandatory.** If you find hard-coding that predates
  your change: replace it with the data-derived source, or flag it
  explicitly (`crates/starbreaker-ui/docs/ui-fallback-register.md` §"Flagged hard-coding", or a
  handoff entry) in the same change. Never copy or extend an existing
  hard-coded pattern because precedent exists — precedent of a banned
  pattern is a defect, not a licence. Extend the guards
  (`scripts/check_ui_hardcoding.sh`, tests/source_hardcoding_guards.rs) when you
  discover a category they miss.
- Fix the structural cause. Do not patch symptoms in one renderer path if the real issue is authored metadata, IR compilation, layout, or text measurement.
- IR is the source of truth for render values. The renderer must not override IR-provided font size, position, alignment, scale, margin, padding, text colour, stroke colour, icon tint, or visibility based on widget names, parent context, or screen-specific checks.
- If rendered output is wrong, correct the owning upstream stage so IR values
  are correct before rendering. Which stage owns what (`bb_resolve/`,
  `bb_state_filter/`, `bb_bindings/`, `bb_layout/`, `ui_ir/`, `ir_compose/`,
  `text/`) is the table in `crates/starbreaker-ui/docs/ui-workflow.md` §2.
- A knowingly-off element whose true fix is deferred is registered as a
  reference-anchored **known-outlier** (`crates/starbreaker-ui/docs/ui-workflow.md` §6), never
  baked in as a fudge factor and never strict-frozen at the wrong value.

## Working loop and checks

The loop (identify the owning stage, one falsifiable hypothesis, one focused
change with a failing test first, measure, then ask for visual confirmation)
is `crates/starbreaker-ui/docs/ui-workflow.md` §3; relative user feedback ("move it ~20px up") is a
calibration target, handled as §3 describes. Query and probe tools (MCP
`ui_variant_styles` / `ui_canvas_style_inventory` / `ui_scene_style_probe` /
`ui_ir_query`, `ui render --dump-ir-dir`, the `query_ui_layout` and
`ui_stage_diff` examples, the env probes) are in `crates/starbreaker-ui/docs/ui-reference.md`
§4, §6 and §7. Keep debug tooling generic: add a measurable output to the
shared query path, never screen-specific debug code.

Before claiming a fix or committing, run `bash scripts/ui_check.sh` unpiped
and read its `ui_check: ALL GREEN` line; the pre-commit gate
(`scripts/install_ui_precommit.sh`) refuses UI/script commits without a fresh
green marker. At a workstream boundary, re-export and run
`bash scripts/ui_check.sh --full`. Required regression coverage per defect
category is `crates/starbreaker-ui/docs/ui-regression-policy.md`.

## Guardrails for code review

Reject a change if it does any of the following:

- branches on `MedGel`, `ui_target_a`, `ui_target_b`, ship names, manufacturer names, or specific asset paths in production logic
- introduces unexplained percentages or offsets for placement
- adds debug output that only works for one screen instead of the generic query path
- fixes a draw-time symptom while leaving a clearly wrong upstream rect or missing metadata untouched

If the source data genuinely does not contain the needed signal, prove it with
the query tools and a blocker-evidence file (`crates/starbreaker-ui/docs/blocker-evidence-schema.md`)
before choosing the narrowest renderer-side fallback, and register that
fallback in `crates/starbreaker-ui/docs/ui-fallback-register.md`.
