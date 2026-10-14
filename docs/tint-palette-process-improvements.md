# Tint palette process — improvements ledger

> **Append-only retrospective ledger** — the dated record of tint-palette
> process/tooling/doc improvements and the **append target for end-of-arc
> retrospectives** (run the closing step of the `starbreaker-tint-palette` skill). Each item:
> **Observed** (the incident that exposed it) / **Improvement** / **Action**.
> Append new items with the next number; never rewrite history. The live how-to is
> `docs/tint-palette-workflow.md`.

The seed items below are the lived failure modes of the New Babbage XL hangar
socpak palette arc (2026-06-20) — the baseline a fresh agent hits WITHOUT this
process. They are why the skill teaches "decode + parse, never assume."

---

## 1. Per-object palette index lived at the wrong field offset

**Observed:** the socpak per-object `tint_palette_index` was first decoded at
chunk offset **+170 (word1)**, which is actually flags — it produced
out-of-range values (8/15/16/19) that all fell back to the default grey palette,
so the hangar walls rendered grey instead of their brand blue. The owner caught
it by eye ("walls should be `gen_grey_darkblue_darkorange`, not grey"); parsing
the field then proved +170 was flags and **+172 (word2)** was the real index.

**Improvement:** never trust an assumed field offset — PARSE the decoded indices
for many objects and check they map to plausible, varied palettes (a field that
yields mostly-out-of-range or all-same values is the wrong field).

**Action:** workflow §3 pins the +172 offset and the parse-the-indices rule;
`included_objects.rs` has the regression test `parse_per_object_tint_palette_index`.

## 2. "Default" palette vs the location brand

**Observed:** ~3900 objects resolved to palette index 0 → the `default`
white/grey/black palette, when many should inherit the location's brand
(microTech) palette. Distinguishing "correctly default" from "should be brand"
required reading the per-object indices, not a global assumption.

**Improvement:** treat index 0 / `0xFFFF` / out-of-range as *no override → scene
default* and verify whether the scene default itself is the right brand (Mode A)
separately from whether specific objects override it (Mode B). Don't conflate.

**Action:** workflow §3/§4 separates Mode A (scene default/dominant) from Mode B
(per-object override).

## 3. Colour role slots are not positional

**Observed:** picking a palette role by array position gave grey where Base was
expected — `BB_ColorStyle` role indices diverge (Bright=6 grey ≠ Base=0).

**Improvement:** resolve a role by its enum index, never by order.

**Action:** workflow §2.

## 4. The decal stencil is part of the palette identity

**Observed:** a stray "GEMINI" logo (`gmni_logo_stencil`) appeared scene-wide —
diagnosed at first as a separate decal bug, but it was the WRONG *palette* being
applied (the decal stencil rides inside the `TintPalette`/`palettes.json` entry).

**Improvement:** when a manufacturer logo/decal is wrong, suspect the palette
assignment first, not a decal-specific path.

**Action:** workflow §2 states the decal is part of the palette identity.

## 5. Stale Blender scene masks every palette change

**Observed:** palette re-exports appeared to "do nothing" because the loaded
Blender scene was a stale import showing the old palette.

**Improvement:** ALWAYS fresh-import (`read_homefile` → import) before judging a
palette change; never re-tune on a stale scene.

**Action:** workflow §5 step 5 makes the fresh re-import mandatory before
verification.
