# UI font-size harness

> Satellite doc — the authoritative process is `crates/starbreaker-ui/docs/ui-workflow.md`; commands/tools/data live in `crates/starbreaker-ui/docs/ui-reference.md`.

A diagnostic tool for auditing the **final rendered size of every text element**
on the UI screens, and regression-checking it against a frozen baseline that is
within ~1–2% of the in-game reference screenshots.

Use it whenever you touch anything that can affect text size — the font-size
resolver (`ui_ir`), the SWF/TTF draw path (`ir_compose`, `text/`), font
records, brand styles, or layout scaling — to prove you did **not** regress the
platinum/gold screens.

## Why it exists

UI text size now comes from the **data-backed design-em model** (plan P3,
2026-06-13): the IR font size IS the design-em pixel size
(em = ascent + |descent|), and the SWF renderer maps em→raster via the font's
own `units_per_em`, so a 30px field measures ~30px tall at factor 1.0. The
earlier per-context tuned scalars (`STANDARD_TEXTFIELD_NOMINAL_FONT_SCALE = 0.72`,
`SWF_TEXT_RENDER_SIZE_CALIBRATION = 0.84`, the `1.5` TTF draw/measure pair, the
`0.33` inline word-gap, the `-8.0` caption overlap) were **retired** —
reintroducing any tuned size scalar is now a regression (workflow §10,
"Text-size calibration constants are RETIRED").
Engine-model detail: [`ui-architecture-runbook.md`](ui-architecture-runbook.md)
§"engine models settled".

But the per-element size still depends on the resolver, the font/glyph metrics,
brand-style FontSize resolution, the `autoFontSize` line-box fit-to-rect path,
and layout scaling — any of which can move rendered text off the platinum/gold
references. This harness makes the *visible* effect of any such change
measurable per element (the held invariant is the rendered glyph cap height)
instead of relying on eyeballing screenshots, so it is the guard that proves you
did **not** regress the platinum/gold screens.

## Components

| Piece | Location | Role |
|---|---|---|
| `SB_UI_FONT_DUMP` emitter | `crates/starbreaker-ui/src/text/swf_draw.rs` | When the env var is set, prints one `FONTDUMP` line per rendered text element. No effect otherwise. |
| dump context | `crates/starbreaker-ui/src/text/mod.rs` (`set_font_dump_ctx`), called from `crates/starbreaker-ui/src/ir_compose/engine_01.rs` | Keys each dump line to its source canvas + node. |
| baseline | `crates/starbreaker-ui/tests/fixtures/font_size_baseline.tsv` | Committed (1–2%-accurate) rendered sizes: 39 elements across the 4 canvases below. |
| checker | `scripts/font_size_check.py` | Diffs a new dump against the baseline; per-element PASS/FAIL; non-zero exit on drift. |

The harness adds **no behavioural change** — it emits nothing and costs nothing
unless `SB_UI_FONT_DUMP` is set.

### Dump line format

```
FONTDUMP \t canvas \t node \t font \t size_px \t visible_px \t units_per_em \t width_px \t text
```

- `size_px` — the size handed to the rasteriser (after IR + render-side scaling).
- `visible_px` — **the held invariant**: the rendered glyph cap height in image
  pixels (`glyph_bbox × size_px / units_per_em`, the max ink height over the
  lines). It folds in every render-side factor (em choice, glyph metrics), so
  matching it means the on-screen text is unchanged. A DASH/`-` placeholder
  measures only a few px here even at a large `size_px`.
- `units_per_em` — the font's own em units (e.g. 20480); a per-font constant, NOT
  a position — do not misread it as a draw coordinate. (The emitter's source
  comment calls this column `em` and omits `width_px`; the `eprintln!` format
  string is the truth.)
- `width_px` — widest rendered line in px (the value to compare against the
  reference text-block width).
- `text` — the first 28 characters of the drawn string.

## Regression targets

The baseline covers four of the 15 frozen targets in
`crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json`: the ones
whose bindings are in the Clipper LOD1 scene the harness replays. The cockpit
MFD and HUD targets (LOD0-only screens) are not in the font baseline; their
text is guarded by the IR snapshot and whole-image guards.

| Canvas | Tier | Screen | Elements |
|---|---|---|---|
| `I_Med_MedicalBed_A` | platinum | Medical bed (ui_target_a) | 15 |
| `I_Med_MedicalEndOfBed_A` | platinum | Medical end-of-bed (ui_target_b) | 6 |
| `I_Door_Small_DRAK` | gold | Small door (`CLOSED`) | 13 |
| `H_Eng_Annunciator_Master_Left` | platinum | Engineering annunciator | 5 |

## Usage

`bash scripts/ui_check.sh --full` runs these two steps for you (scene:
`UI_CHECK_SCENE`, default the Clipper LOD1 scene). By hand, from the repo root
(the P4K is auto-detected; don't set `SC_DATA_P4K`):

```bash
# 1. Capture a dump from the Clipper LOD1 interior scene (it carries the
#    medical/door/annunciator bindings; the LOD0 cockpit scene does not).
SB_UI_FONT_DUMP=1 cargo run -q -p starbreaker -- ui render \
    --scene "<export>/Packages/DRAK Clipper_LOD1_TEX2/scene.json" \
    --out-dir /tmp/fontcheck 2>&1 | grep '^FONTDUMP' > /tmp/font_dump.tsv

# 2. Check it against the frozen baseline (default tolerance 2.5%).
python3 scripts/font_size_check.py /tmp/font_dump.tsv
#   -> "baseline elements: 39  matched within 2.5%: 39 ... PASS"

# Optional: looser tolerance, or a different baseline file.
python3 scripts/font_size_check.py /tmp/font_dump.tsv 4
python3 scripts/font_size_check.py /tmp/font_dump.tsv 2.5 /path/to/other_baseline.tsv
```

A failing element prints its exact drift, e.g.:

```
  -28.6%  H_Eng_Annunciator_Master_Left Text Item 0   base=  71.74 new=  51.21  'PWR'
FAIL (15 drifted)
```

The script exits non-zero on any drift, so it can gate a change before commit.

## Re-baselining (after an intentional, approved size change)

Only do this when a size change is *deliberate* and verified against the in-game
references — never to silence a drift.

```bash
SB_UI_FONT_DUMP=1 cargo run -q -p starbreaker -- ui render \
    --scene "<export>/Packages/DRAK Clipper_LOD1_TEX2/scene.json" \
    --out-dir /tmp/fontcheck 2>&1 | grep '^FONTDUMP' \
  | grep -E 'I_Med_MedicalBed_A|I_Med_MedicalEndOfBed_A|I_Door_Small_DRAK|H_Eng_Annunciator_Master_Left' \
  | sed 's/BuildingBlocks_Canvas\.//' | sort \
  > crates/starbreaker-ui/tests/fixtures/font_size_baseline.tsv
```

This is the *visible-size* companion to the artifact/byte-hash freeze
(`crates/starbreaker-ui/docs/ui-workflow.md` §7): the hash freeze proves pixels are identical; this
proves text *sizes* are within tolerance even when other pixels legitimately
change.

## Notes / caveats

- `visible_px` is measured from the glyph bounding box, so a per-element key with
  unusually short text (one wide/narrow glyph) can read a percent or two off the
  style's nominal — keep the default tolerance at ≥2.5%.
- The dump only covers SWF-font text (`draw_swf_font`). TTF-fallback text is not
  emitted; on the Clipper every element resolves to an SWF font, so coverage is
  complete there.
- Keyed by `(canvas, node, text)`; if a binding renders the same node with
  several different strings (e.g. the three medical option cards) each string is
  a separate row.
