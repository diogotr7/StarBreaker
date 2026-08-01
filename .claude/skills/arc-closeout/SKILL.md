---
name: arc-closeout
description: Use when a StarBreaker work arc (UI parity, tint palette, optimisation pass, or any skill-driven arc) is reaching its end — when the last fix has landed, when about to declare complete, or when tempted to stop and ask "shall I continue?".
---

# Arc Close-out

The arc is not done when the fix lands — it is done after this close-out. Every
StarBreaker arc skill (ui-screen-parity, tint-palette, optimisation) delegates its
mandatory closing retrospective here. Track "arc close-out" as a todo from ARC START
so it is never dropped; sessions where it wasn't a todo are the sessions the owner
had to chase it ("have you specifically completed the retrospective?").

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

## Destinations (mechanics are scripted)

- **Process/tool/doc findings** → APPEND a numbered item to the arc's ledger via
  `uv run python scripts/ledger_append.py <ledger> --title "..." < body.md`
  (auto-detects each ledger's numbering/format):
  - UI parity → `crates/starbreaker-ui/docs/ui-process-improvements.md`
  - tint palette → `docs/tint-palette-process-improvements.md`
  - optimisation → `docs/optimisation-ledger.md`
  Then IMPLEMENT the tooling/instrumentation wins — one commit per coherent item.
- **Improvements to the arc's SKILL itself** → append under **Open recommendations**
  in that skill's `recommendations.md`; never rewrite a SKILL.md mid-arc.
- **Project memory** → update the relevant memory file AND its `MEMORY.md` index
  line with the arc's outcome (cumulative numbers, new dead-ends), so the next
  session starts from the current state.
- **Docs changed and `graphify-out/` exists** → run graphify-doc-sync on the
  finalized docs before closing.

## Acceptance (bootstrap test)

A fresh agent could run the next arc from the skill + workflow doc + ledger + memory
alone. Any excursion this arc needed is a doc bug — fix it before closing.

## Red flags — STOP, you're rationalizing

| Thought | Reality |
|---|---|
| "The fixes landed — done" / "colours are right, I'm done" | The arc ends after the close-out, never at the last fix. |
| "I'll note the retro items for later" | Later never comes. Sweep items are FIXED in this session or explicitly ledgered with an owner-visible entry. |
| "Found more issues during the retro — stop and ask" | New fixable issues found during close-out go back into the arc loop; the close-out reruns after. |
| "Short on context — skip the retro" | Short on context = say so and hand state to memory/handoff; the ledger entry and memory update are exactly the handoff. |
| "The ledger format/number — I'll eyeball it" | `scripts/ledger_append.py` computes it. |
