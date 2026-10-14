# starbreaker-tint-palette — recommendations

Companion to `SKILL.md`. End-of-arc retrospectives (via the arc-closeout skill) and
reviews append skill improvements under **Open** rather than editing `SKILL.md`
mid-arc; when a `writing-skills` session applies one, it moves to the **Change
log** as a one-line entry. Process/tool findings go to the ledger
(`docs/tint-palette-process-improvements.md`), not here.

## Validation status

Grounded in one real arc (the New Babbage XL hangar socpak palette arc,
2026-06-20; its failure modes are ledger items 1–5 and the SKILL's Operating
posture + Red flags), not subagent-tested. The next real invocation is the test —
ideally a SHIP, to exercise Mode A paint/livery.

## Open

1. **Ship Mode-A coverage is under-exercised.** The seed arc was a socpak (Mode B,
   per-object). On the first ship paint/livery arc, confirm the `Apply Paint` /
   `Apply Livery` operators + `liveries.json` are documented accurately in
   workflow §4; tighten if anything had to be re-derived.

## Change log

- **2026-06-20 — created** from the New Babbage hangar palette arc (workflow +
  ledger items 1–5 + `SKILL.md` + this file).
- **2026-07-08 — persona reconciliation** (alignment plan T15, `writing-skills`
  review): strict rules gained the ponytail note and the "this skill IS the process
  skill for its arc" line, verbatim with ui-screen-parity. Owner note: the shared
  "launch/catalog gates" wording is illustrative here (this skill's gates are
  input-gather + mode-decision).
- **Palette-dump tooling — DONE** (`9fbf3ded`): `scripts/tint_audit.py` is the
  parse-don't-eyeball battery, cited in `SKILL.md` Close-out.
- **2026-10-14 — docs audit:** dead memory wiki-link replaced by ledger 3;
  close-out records durable facts in the ledger (no user-folder memory); commits
  land on the arc branch, never `main`.
