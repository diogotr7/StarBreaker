# ui_screen_dossier_v1.json — provenance

**Source:** transcribed from `crates/starbreaker-ui/docs/ui-reference.md` §3
"Comparison & screen dossier" (the markdown table, one row per known screen).

**Transcribed:** 2026-07-06, from the table as of commit `38863e78f`
(plan `docs/superpowers/plans/2026-07-04-ui-parity-process-and-crate-plan.md`,
Task A1). 18 screens.

## The no-drift rule (registry pattern)

The markdown table in ui-reference §3 and this JSON are two views of the same
data and **must not diverge**. `scripts/validate_ui_dossier.py` enforces it:
it re-derives `screen_id`, `helper`, `preset`, `tier`, `target_id` from the
markdown and fails if any screen's values differ, or if either side has a
screen the other lacks. The validator runs in the repo-only (default) tier of
`scripts/ui_check.sh`.

**When you add or change a screen, edit BOTH** the §3 table and this JSON in
the same change, then run `python3 scripts/validate_ui_dossier.py` (or
`bash scripts/ui_check.sh`) to confirm they still agree.

## Field-extraction contract

The validator derives the cross-checked fields from a §3 table row (cells
split on `|`: Screen | Helper/scene | Canvas | Reference | Preset |
Tier/target id | Open issues) using these rules — this JSON follows the same
rules so the two agree:

- **helper** — first `` `backticked` `` token in the Helper/scene cell; if the
  cell has none (e.g. "usable screen / LOD1 scene"), the text before the first
  `/`. Not required to be unique.
- **canvas** — first backticked token in the Canvas cell.
- **screen_id** — the helper token when the Helper/scene cell is backticked;
  otherwise the canvas name (so medical/door rows, whose helper cell is the
  generic "usable screen", key on their unique canvas). Unique across screens.
- **preset** — first backticked token in the Preset cell; `null` for `—` /
  `— (add)` / `— (whole-image)` cells (no backtick).
- **tier** — `PLATINUM` or `GOLD` if that word appears in the Tier/target id
  cell, else `null`.
- **target_id** — first backticked token in the Tier/target id cell, else `null`.

Fields NOT cross-checked (free-form, may summarise the table):
`reference_file`, `scene_package`, `lod`, `open_issues`. `open_issues` here is
a short summary of the table's (often very long) Open-issues prose.
`reference_file` is `null` where the table row has no capture ("mirror of L",
"no straight-on capture yet"). `scene_package` is derived from the LOD column
("LOD0 scene/cockpit" → `<ship>_LOD0_TEX0`; Clipper "LOD1 scene" →
`DRAK Clipper_LOD1_TEX2`; Carrack → `ANVL Carrack_LOD0_TEX0`) and cross-checked
against `~/projects/scorg_tools/ships/Packages/` — a missing package is a
WARNING only (repo-only CI and partially-exported trees have no game data).
