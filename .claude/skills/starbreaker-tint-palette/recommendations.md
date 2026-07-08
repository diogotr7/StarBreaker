# starbreaker-tint-palette — review & recommendations

Companion to `SKILL.md`. Following the `starbreaker-ui-screen-parity` precedent, this skill
is **not** pressure-tested by re-writing it in a RED→GREEN subagent loop;
instead, findings land here in two states, with a lifecycle:

- **Open recommendations** — ideas not yet applied. End-of-arc retrospectives
  (the skill's closing step) and reviews append here rather than editing
  `SKILL.md` mid-arc.
- **Change log** — applied changes. When a `writing-skills` session implements an
  open recommendation (or an owner-requested edit), it records the applied change
  here and clears the open item.

## Validation status

- **Grounded in a real arc, not subagent-tested.** Modeled on `starbreaker-ui-screen-parity`
  (which is validated by real use, not a formal baseline/green run). `starbreaker-tint-palette`
  was authored from the lived New Babbage XL hangar socpak palette arc
  (2026-06-20), whose real failure modes are seeded as ledger items 1–5
  (`docs/tint-palette-process-improvements.md`) and as the **Operating posture** +
  **Red flags** in `SKILL.md`. The recurring class was "agent assumes instead of
  parsing / re-imports stale" — addressed by the ASK/PARSE/JUST-DO/GATE posture
  and the mandatory fresh re-import.
- **Next real invocation is the test.** The first time the skill drives a fresh
  palette arc (ideally a SHIP, to exercise Mode A paint/livery, since the seed arc
  was a socpak Mode B), capture any shortfall here and in the ledger.

## Open recommendations

1. **Ship Mode-A coverage is under-exercised.** The seed arc was a socpak (Mode B,
   per-object). Confirm on a ship paint/livery arc that the `Apply Paint` /
   `Apply Livery` operators + `liveries.json` are documented accurately in
   workflow §4; tighten if the first ship run needs anything re-derived.
2. **Palette-dump tooling.** If parsing `palettes.json` + per-object `palette_id`
   + the `TintPaletteTree` record gets typed >2× across arcs, add a
   `scripts/`/`examples/` palette-dump helper (per the retrospective's category 1)
   and cite it in workflow §6.

## Change log

- **2026-06-20 — created.** Authored from the New Babbage hangar palette arc.
  Workflow (`docs/tint-palette-workflow.md`) + ledger
  (`docs/tint-palette-process-improvements.md`, items 1–5) + `SKILL.md` +
  this file.

- **2026-07-08 (alignment plan T15 — persona reconciliation; applied via
  `superpowers:writing-skills` review).** `SKILL.md` strict rules gained the two always-on-persona
  reconciliation lines (identical wording to the `starbreaker-ui-screen-parity` skill): (a) a
  **ponytail note** — laziest-that-works = smallest change still engine-faithful + data-derived;
  a hard-coded value / invented geometry / name-gate / heuristic is never "lazy" here, it trips
  the guards; and (b) **this skill IS the process skill for its arc** — its own gates stand in
  for superpowers:brainstorming's design gate; superpowers planning/TDD/debugging are sub-tools
  WITHIN the loop. `writing-skills` pass: reframe-not-prohibition form, skill-name cross-refs
  (no `@`), bold leads. NOTE for owner review: line (b) uses the shared verbatim "launch/catalog
  gates" wording; this skill's gates are input-gather + mode-decision (no numbered "catalog"
  phase), so the phrase is illustrative-of-parity rather than literal here.
