# UI parity — per-screen agent prompt

Instantiate the variables and paste. All process knowledge lives in
`crates/starbreaker-ui/docs/ui-workflow.md` + `crates/starbreaker-ui/docs/ui-reference.md`;
keep this prompt short and let those docs do the arming. Companion:
`ui-process-retro-prompt.md` runs the post-arc retrospective. (All UI docs
live under `crates/starbreaker-ui/docs/`.)

## Template

```text
You are working in the StarBreaker repo, crate starbreaker-ui. Changes
land on main through pull requests (squash merge): work on a branch cut
from main, never commit to main directly.

Read, in order:
1. @StarBreaker/AGENTS.md
2. @StarBreaker/crates/starbreaker-ui/AGENTS.md
3. @StarBreaker/crates/starbreaker-ui/docs/ui-workflow.md      (the process — follow it exactly)
4. @StarBreaker/crates/starbreaker-ui/docs/ui-reference.md     (commands, tools, data; find SCREEN in §3 dossier)

SCREEN=<dossier row, e.g. Screen_Left_Lower_RTT>
REFERENCE=<optional reference-image path — overrides the dossier's, or
  supplies one for a screen not yet in the dossier; in that case ADD the
  screen's dossier row (crates/starbreaker-ui/docs/ui-reference.md §3) as part of the work>
HANDOFF=<optional, e.g. crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md — read it if set>
KNOWN_SYMPTOMS=<optional user-observed differences, else start from review>

Goal: get SCREEN's render closer to its reference image (REFERENCE if set,
else the dossier row's; the dossier row also gives scene, canvas, compare
preset, frozen tier, open issues).

Work: replay-render SCREEN; run scripts/ui_compare.py with the preset and
READ the crops; build/extend the numbered diff catalog (workflow §4); then
the TDD loop
from @StarBreaker/crates/starbreaker-ui/docs/ui-workflow.md §3 — failing test first, one structural fix at the
owning stage, scripts/ui_check.sh every cycle, re-render + compare, update
the catalog and the arc's handoff doc / dossier row. Guard trips → §5 adjudication
(structural discriminators, never names). Baselines/outliers only via §6/§7
with explicit approval. Do not stop until the catalog items are fixed,
explicitly deferred with reasons, or a concrete blocker is proven.
```

## Example (power screen)

`SCREEN=Screen_Left_Lower_RTT`,
`HANDOFF=crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md` — the dossier maps it to the
LOD0 scene, canvas `MC_S_Power_Master`, reference
`reference/in-game/Clipper/Screen_Left_Lower_RTT.png`, preset `power`.
