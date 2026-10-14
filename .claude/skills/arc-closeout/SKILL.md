---
name: arc-closeout
description: Use when a StarBreaker work arc (UI parity, tint palette, optimisation pass, or any skill-driven arc) is reaching its end — when the last fix has landed, when about to declare complete, or when tempted to stop and ask "shall I continue?".
---

# Arc Close-out

The arc is not done when the fix lands — it is done after this close-out. Every
StarBreaker arc skill (ui-screen-parity, tint-palette, optimisation) delegates its
mandatory closing retrospective here. Track "arc close-out" as an item in the task
list (TodoWrite where available) from ARC START so it is never dropped; sessions
where it wasn't tracked are the sessions the owner had to chase it ("have you
specifically completed the retrospective?").

## The sweep (lived context, SAME session — fix each, don't just note it)

1. **Repeated manual work → tooling.** Anything typed >2× this arc (a measurement,
   a diff battery, a parse) becomes/extends a `scripts/` helper — extend before
   creating.
2. **Silent failures → loud.** Any decode/guard/probe that gave a wrong-but-plausible
   answer gets a distinct hard failure or a regression test.
3. **Doc drift.** Every doc claim relied on that was wrong/stale gets fixed with
   verify-on-write (run the cited command in the same commit).
4. **Dead-ends and levers → record** so they aren't re-attempted/re-derived
   (which ledger: see destinations).
5. **Bootstrap cost.** Anything re-derived this arc (a flag, an offset, a data
   location, a trap) lands in the owning workflow doc.

The calling skill's retro reference (e.g. ui-screen-parity `references/retro.md`)
extends this list from 6 onward; it does not renumber 1-5.

## Destinations (mechanics are scripted)

Knowledge stays **in-repo** — never in user-folder memory
(`~/.claude/projects/.../memory/`). The ledger + workflow doc ARE the handoff.

- **Process/tool/doc findings** → APPEND a numbered item to the arc's ledger via
  `uv run python scripts/ledger_append.py <ledger> --title "..." < body.md`
  (auto-detects each ledger's numbering/format):
  - UI parity → `crates/starbreaker-ui/docs/ui-process-improvements.md`
  - tint palette → `docs/tint-palette-process-improvements.md`
  - optimisation → `docs/optimisation-ledger.md`
  Then IMPLEMENT the tooling/instrumentation wins — one commit per coherent item.
- **Durable facts / arc outcome** (cumulative numbers, new dead-ends, the current
  state the next session starts from) → record them in the arc's ledger entry (and
  the owning workflow doc when they change the process), so the next session
  starts from the current state.
- **Improvements to the arc's SKILL itself** → append under **Open** in that
  skill's `recommendations.md`; never rewrite a SKILL.md mid-arc.
- **Docs changed and `graphify-out/` exists** (graphify is optional) → refresh the
  graph: `graphify update .` for code, `/graphify . --update` for doc changes.
  Skip when graphify isn't installed.

## Commits

Commit only under the same gate the calling skill uses (UI parity: per the
launch-chosen mode in its `references/freeze.md` — semi-automated asks, fully
automated commits; tint palette and optimisation: ask first).
Always commit **on the arc branch, never on `main`** — work lands on `main` via a
squash-merged PR from that branch in the main checkout.

## Acceptance (bootstrap test)

A fresh agent could run the next arc from the skill + workflow doc + ledger alone.
Any excursion this arc needed is a doc bug — fix it before closing.

## Red flags — STOP, you're rationalizing

| Thought | Reality |
|---|---|
| "The fixes landed — done" / "colours are right, I'm done" | The arc ends after the close-out, never at the last fix. |
| "I'll note the retro items for later" | Later never comes. Sweep items are FIXED in this session or explicitly ledgered with an owner-visible entry. |
| "Found more issues during the retro — stop and ask" | New fixable issues found during close-out go back into the arc loop; the close-out reruns after. |
| "Short on context — skip the retro" | Short on context = say so and hand state over via the ledger entry; that entry is exactly the handoff. |
| "The ledger format/number — I'll eyeball it" | `scripts/ledger_append.py` computes it. |
| "I'll save this to memory" | Knowledge stays in-repo: ledger or workflow doc, never user-folder memory. |
