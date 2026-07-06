# Blocker-evidence schema (plan A5)

A parity **blocker** claim — "this element cannot be reproduced from the game
data, it needs runtime state / it's a capture artifact" — has a poor track
record. Proven blockers kept dissolving the moment someone searched the right
data family: compass live ticks (the projection was decoded
`SVehicleHudParams.compassTape`), master-mode font size and colour (an
UNAPPLIED variant `defaultStyles` entry), velocity-num size (a SCREEN variant's
FontSize 500 that wasn't reaching the node). See [[feedback-find-indata-sources-before-inventing]].

**Rule:** a blocker claim presented at the **major-item gate** MUST attach an
evidence file that `scripts/ui_blocker_evidence.py` accepts. The file forces you
to record that you actually queried each in-data surface before concluding
"blocked" — the check is that the search happened, not that it failed.

```bash
python3 scripts/ui_blocker_evidence.py <evidence.json>   # ui_blocker_evidence: OK ...
python3 scripts/ui_blocker_evidence.py --self-test
```

## Schema

```json
{
  "item": "<the element / symptom being adjudicated>",
  "surfaces": {
    "datacore_families":    [{"query": "<what you searched>", "result": "<what you found>"}],
    "p4k_assets":           [{"query": "...", "result": "..."}],
    "localization":         [{"query": "...", "result": "..."}],
    "derivable_mechanisms": [{"query": "...", "result": "..."}]
  },
  "conclusion": "blocked | found"
}
```

- All four surfaces are REQUIRED and each needs ≥ 1 `{query, result}` pair with
  non-empty strings. The four surfaces are the four ways a value that "looks
  blocked" has actually turned out to be reachable:
  - **datacore_families** — the DataCore record family the value could live in
    (e.g. `search_records("vehiclehud")` → `SVehicleHudParams`). This is the one
    that dissolved the compass blocker.
  - **p4k_assets** — a P4K asset (texture / xml / swf) that carries it
    (`p4k_search`, Primitive material → texture, `svgFill.svgPath`, styleTag
    `SvgPath`/`ImagePath`).
  - **localization** — a loc key / SIUnit / global.ini string.
  - **derivable_mechanisms** — a principled at-rest derivation (heading 0,
    velocity 0, `ui_variant_styles` authored-but-unapplied entry, screen-mesh
    aspect) rather than procedural invention.
- `conclusion` is `blocked` (all four surfaces exhausted, genuinely runtime /
  capture) or `found` (a surface yielded the value — the common outcome).

## Worked example — the compass ticks that were NOT blocked (ledger 66)

Validated by `scripts/ui_blocker_evidence.py` (exit 0):

```json
{
  "item": "compass live ticks / labels",
  "surfaces": {
    "datacore_families": [
      {"query": "search_records(\"vehiclehud\") -> SVehicleHudParams",
       "result": "FOUND SVehicleHudParams.VehicleHudDefault.compassTape (hudparams/vehiclehuddefault.xml): range=90deg window, mainTickIncrement=20deg labelled, subTicks=4 -> minor every 5deg; the tick projection IS decoded data"}
    ],
    "p4k_assets": [
      {"query": "p4k_search hudparams/vehiclehuddefault.xml",
       "result": "vehiclehuddefault.xml carries the compassTape config; no separate tick texture"}
    ],
    "localization": [
      {"query": "tick label strings",
       "result": "labels are numeric headings derived from the heading value, not loc keys"}
    ],
    "derivable_mechanisms": [
      {"query": "derive at-rest tick array in ship_values.rs from compassTape at heading 0",
       "result": "derive_compass_ticks + compass_tape_from_global: 19 ticks in [-45,45]deg; principled at-rest heading 0 (like velocity=0), all from compassTape"}
    ]
  },
  "conclusion": "found"
}
```

The `datacore_families` search is what turned a "proven blocker" into a
data-backed fix — which is exactly the discipline this file enforces before a
blocker is accepted at the gate.
