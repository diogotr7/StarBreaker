---
name: starbreaker-tint-palette
description: Use when getting StarBreaker tint palettes / paint to look right on an exported ship or socpak scene — e.g. "the hangar walls should be microTech blue not grey", "change the scene's main palette", "this object has the wrong manufacturer colours or decal/logo". Triggers: tint palette, paint scheme, manufacturer colours, wrong/grey/default palette, per-object tint, palettes.json, apply_palette, stray logo/decal.
---

# Tint Palette

## Overview

Get a StarBreaker exported scene's **tint palettes / paint** correct —
**engine-faithfully and generically**, for ships AND socpaks. This skill
ORCHESTRATES the authoritative process; it does not replace it. The how-to is
`docs/tint-palette-workflow.md`; the dated lessons are
`docs/tint-palette-process-improvements.md` (the ledger). Follow them exactly.

**Core principle:** wrong colour → fix where the palette is **decoded or
assigned** (the per-object index field, the index→palette resolve, the role-slot
map), never a draw-time tint nudge and **never a hard-coded colour value**. Every
colour is derived at run time from the DataCore `TintPaletteTree`. There is no
reference image — correctness is judged structurally (which palette role, which
brand, which decal stencil), confirmed by PARSING the data and looking at the
re-imported scene.

Two modes — decide which the request is FIRST (workflow §4):
- **Mode A — scene main palette:** change the dominant palette for the whole
  scene (Blender `Apply Palette`/`Apply Paint`/`Apply Livery`, or the export's
  default-palette resolution).
- **Mode B — match specific objects:** get individual objects' palettes right
  (the per-object `tint_palette_index → palette_id` decode/resolve — an exporter
  fix, verified by parsing the indices).

## Operating posture

Nearly every way this goes wrong is the same shortcut: substituting a guess for a
parse or a fresh import. Default behaviour by category:

- **Inputs / user-facing choices → ASK** via `AskUserQuestion`, fresh each run:
  the source (ship vs socpak) + entity/socpak name, Mode A vs B, and the target
  (which palette / which objects / what they should look like). Never presume
  from session state.
- **Technical judgments → PARSE / MEASURE.** Which palette an object has, whether
  index 0 means "correctly default" or "should be brand", which role slot is
  wrong: prove it by parsing `palettes.json` + the per-object `palette_id` + the
  `TintPaletteTree` record and iterating — never eyeball one object and assume the
  field (ledger 1). A colour claim must survive parsing the palette, not a pixel
  pick.
- **The work itself → JUST DO IT.** Applying the structural fix (TDD, failing
  test first) is the loop's job, not a checkpoint.
- **Hard-to-reverse / outward actions → GATE:** git commit, and any change that
  re-paints a frozen/blessed baseline export. Stop and ask via `AskUserQuestion`.

Asking and parsing are always safe; presuming and pixel-picking are the failure
mode.

## Gather inputs (in order, via `AskUserQuestion`)

Every run starts cold. `AskUserQuestion` needs ≥2 explicit options (the auto
"Other" does not count) — pad if a discovered list has one entry.

1. **Source + name** — ship (`entity export`) or socpak (`socpak export`), and
   the entity/socpak name (e.g. `hangar_xltop_001_newbab`, `drak_cutlass`).
2. **Mode** — "Scene main palette (Mode A)" vs "Specific objects (Mode B)";
   "Other" captures a mix. The chosen mode picks the workflow path (§4).
3. **Target** — what should it look like: a named palette to apply (Mode A), or
   which objects + their expected brand/role (Mode B). Free-text via "Other".

## Required reads (before any fix)

In order: `StarBreaker/AGENTS.md` → `blender_addon/AGENTS.md` (if touching the
addon) → `docs/tint-palette-workflow.md` (THE process) →
`docs/tint-palette-process-improvements.md` (the ledger — its seed items are the
traps you will otherwise re-hit).

## The loop (workflow §5)

Decode source → diagnose mode + owning stage → TDD structural fix → re-export →
**fresh** re-import → verify → closing re-check → retrospective. Per the
workflow:

- **Decode (parse, don't assume):** the `TintPaletteTree` record, the socpak
  `tint_palette_paths` + per-object `tint_palette_index` (offset **+172**), and
  the exported `palettes.json` + per-object `palette_id`.
- **Fix at the owning stage, generically:** failing test first; no colour
  literals, no name/asset/offset gates; must hold for ships AND socpaks.
- **Re-export** (`--kind decomposed --lod 0 --mip 0`), then **ALWAYS fresh
  re-import** in Blender (`read_homefile` → import) — a stale scene shows the old
  palette and masks the change (ledger 5).
- **Verify** by parsing `palettes.json`/`palette_id` AND looking at the
  re-imported objects (Blender MCP screenshot/viewport): right role colours +
  right decal stencil.
- **Self-correct hard-coding:** if you find an existing colour literal / name
  gate while working, replace or flag it in the SAME change (AGENTS).

## Checkpoints

| Checkpoint | Action |
|---|---|
| Inputs (source, mode, target) at launch | ask via `AskUserQuestion` |
| Git commit | gate — show diff, ask before committing |
| Re-painting a frozen/blessed baseline export | gate — ask before overwriting |

Never proceed on a presumed "yes."

## Strict rules (workflow §1 — non-negotiable)

- **No hard-coded colour VALUES** anywhere (source, tests, fallbacks). Derive
  from `TintPaletteTree`/`mtl::TintPalette` at run time.
- **No name/asset/offset gating** — find the structural property, fix the
  category.
- **Colour role slots are not positional** — resolve a role by its
  `BB_ColorStyle` enum index, never by order (memory
  [[bb-colorstyle-enum-slot-mapping]]).
- **The decal stencil is part of the palette identity** — a wrong logo is a wrong
  palette, not a decal bug.
- Fix the decode/assign stage, not a draw-time tint.
- **Ponytail note:** in this repo the laziest solution that works = the smallest
  change that is still engine-faithful and data-derived. A hard-coded value, invented
  geometry, name-gate, or heuristic is never 'the lazy solution' here — it trips the
  guards.
- **This skill IS the process skill for its arc** — its launch/catalog gates stand in
  for superpowers:brainstorming's design gate; superpowers planning/TDD/debugging
  skills are sub-tools invoked WITHIN the loop.

## Close-out (MANDATORY closing step)

**The arc is not done when the colours are right — it is done after the
close-out.** Invoke the **arc-closeout** skill (a todo from arc start): it runs
the sweep, the ledger append (`docs/tint-palette-process-improvements.md`, via
`scripts/ledger_append.py`), the `recommendations.md` split, the memory update,
and the bootstrap-test acceptance. Ledger entries use Observed/Improvement/Action.

The parse-don't-eyeball battery is scripted: `uv run python scripts/tint_audit.py
<export_dir>` dumps per-palette object counts, out-of-range/all-same index signals
(ledger 1), and no-override counts (ledger 2) — run it instead of hand-rolled
index parses.

## Red flags — STOP, you're rationalizing

| Thought | Reality |
|---|---|
| "Just hard-code this colour / fallback palette" | Banned, fixtures and fallbacks included. Derive from `TintPaletteTree`. |
| "This object's wrong, special-case it by name/offset" | Find the structural cause (index field, resolve rule, role slot) and fix the category. |
| "I'll eyeball one object and assume the field is right" | PARSE the indices for many objects; a field yielding out-of-range/all-same is the wrong field (ledger 1). |
| "Role 0 is Base, pick by position" | `BB_ColorStyle` slots diverge (Bright=6 ≠ Base=0). Resolve by enum index. |
| "Stray logo → fix the decal path" | The decal stencil is part of the palette identity; suspect the palette assignment (ledger 4). |
| "Re-export looks unchanged — my fix did nothing" | A stale Blender scene shows the OLD palette. Fresh `read_homefile`+import first (ledger 5). |
| "Index 0 everywhere is a bug" | 0/`0xFFFF`/out-of-range = no override → scene default. Mode A (default) vs Mode B (override) are separate questions (ledger 2). |
| "Colours look right, I'm done" | The arc ends after the retrospective (append to the ledger + implement). Track it as a todo from the start. |
| "I'll presume the source/mode/target" | Every run starts cold — ask via `AskUserQuestion`. |
