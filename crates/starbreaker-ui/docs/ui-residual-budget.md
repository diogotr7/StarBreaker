# UI residual budget & capture provenance

> Satellite doc. The authoritative process is `crates/starbreaker-ui/docs/ui-workflow.md`;
> commands/tools/data are in `crates/starbreaker-ui/docs/ui-reference.md`; regression
> tiers, guards and the known-outlier mechanism are in
> `crates/starbreaker-ui/docs/ui-regression-policy.md`. This doc answers one question:
> **when is a render-vs-reference gap acceptable, and when is it a bug you must fix?**
> Classifications owner-approved 2026-07-08 (alignment plan T11).

A residual is any remaining difference between a rendered screen and its in-game
reference capture. Every residual is classified on two axes:

- **CAUSE** — is the difference in the **capture** (a property of how the reference
  screenshot was taken: bloom, skew, a live runtime state, a physical-screen
  characteristic) or in the **renderer** (our pipeline produces the wrong pixels)?
- **STATUS** — `permanent-acceptable` (the gap is intrinsic to the capture and will
  never close; do not chase it) or `must-fix` (a real renderer defect; fix now, or
  register it as a reference-anchored known-outlier IOU and fix later).

Capture-caused ⇒ permanent-acceptable. Renderer-caused ⇒ must-fix. There is no third
class (see "The retired interim class"). A residual whose cause is not yet proven
either way is **unclassified/pending-investigation** — it stays open; it is never
defaulted to "capture, acceptable".

## The one rule that governs the whole table

**A capture-imperfection claim needs a DISTINGUISHING MEASUREMENT, never an eyeball.**
"It looks like bloom / it looks close / it's within tolerance" is not a classification —
it is a hypothesis until a measurement separates the capture explanation from the
renderer explanation. The SUB DECK heading was called "within tolerance" twice on a
width-match coincidence before a sibling cap-height ratio proved it ~1.35× too small
and genuinely renderer-caused (ledger 107). Closing an item as capture-caused /
within-tolerance carries the **same evidence bar as declaring a blocker**
(the parity skill's `references/catalog.md`): a cap-height ratio, connected-component
isolation, `scripts/ui_measure.py`, or the per-target `--full` percentages — not a look.

## How residuals interact with the tier budgets

The whole-image regression guard (`tests/manifest_visual_regression.rs`) fails when
more than a tier-dependent fraction of pixels differ **from the frozen baseline** — not
from the reference capture. Budgets: **platinum 0.5%**, **gold 1%**
(`crates/starbreaker-ui/docs/ui-regression-policy.md`). Three consequences this doc
turns into rules:

1. **Capture-caused residuals never touch the pixel budget.** The baseline was frozen
   *against the render*, so a render-vs-reference gap (bloom, skew, live state) is
   invisible to the budget by construction. The budget governs render-vs-baseline
   **drift over time**, not fidelity to the capture. So "it passes the 0.5%/1% budget"
   is *never* evidence that a capture-caused residual is acceptable — that judgement
   is made here, by measurement, not by the guard.

2. **A must-fix residual smaller than the budget is NOT protected by it.** A few-px
   element (a vanished nav arrow, a slightly-small heading) stays under the tier
   fraction and passes a GREEN `--full` (ledger 77/106/107). Sub-threshold
   element-loss is owned generically by the **element-presence guard**
   (`tests/element_presence_guard.rs`); anything below that (a size/position residual
   that is present but off) needs the sibling-eyeball step and, if deferred, a
   known-outlier entry. **Never read a GREEN budget as "no must-fix residual".**

3. **A deferred must-fix residual with a measurable field is tracked as a
   reference-anchored known-outlier**, not frozen as a strict baseline (freezing the
   miss would later flag the genuine fix as a regression). The comparator treats the
   field one-sided: moving **toward** the measured `reference_target` passes with a
   `✅ IMPROVEMENT` note (**re-freeze, don't revert**); moving away fails; reaching
   within `confidence` graduates to a strict freeze
   (`crates/starbreaker-ui/docs/ui-regression-policy.md` §Known-Outlier Overrides;
   register file `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_known_outliers.json`).

## Residual classes

Each row's distinguishing detection method is the measurement that separates the
class from its opposite-cause twin.

### Capture-caused → permanent-acceptable (owner-approved 2026-07-08)

| # | Class | Distinguishing detection (capture vs renderer) | Tier-budget interaction | Evidence |
|---|---|---|---|---|
| C1 | **Physical-screen emissive characteristics** — bloom / over-brightness (bright elements read whiter, blue channel lifts near bright areas), the brighter uniform panel vs the render's authored vignette, and the dark screen-edge bezel falloff. One class (owner 2026-07-08); **hue ratios are the faithful comparison.** | Photometric anchor: `scripts/ui_compare.py --stats` R-normalised ratios calibrated to a KNOWN anchor on the SAME capture (footer text = Base, pip slabs = Bright). A uniform luminance lift with **unchanged hue ratios** = the physical screen (capture); a **changed hue ratio** = a real colour bug (renderer). | Invisible to the budget (render-vs-baseline). | velocity-num note (c) — vignette vs emissive panel + bezel, "PROVEN capture characteristic"; master-mode residual (SCM/GUN at authored α0.64 vs ref bloomed white); compass minor ticks (authored alpha 0.2, "ref brighter = bloom"); LR-indicator ("within capture bloom"). |
| C2 | **Perspective skew** — off-straight-on capture, geometry sheared. | Store the four screen-corner pixel coords as `<ref>.corners.json`; `scripts/ui_compare.py` homography-rectifies. Judge POSITION on the rectified capture; **judge thin-feature COLOUR on the crisp original** (ledger 35). | Measurement-side; no budget effect. | ui-reference §3; workflow §10 rectification caveat. |
| C3 | **Mouse-hover / cursor artifacts** — spurious outlines/highlights the engine draws only under a live pointer (e.g. power-screen pip red/white outlines). | The authored node carries no such state at rest; the artifact is absent in a clean capture and has no IR source. | Invisible to budget. | workflow §4/§10 ("power-screen pip outlines are mouse-hover artifacts"). |
| C4 | **Capture resolution / rescale** — reference shot at a different pixel size (e.g. 1959×1513 vs 1600 render space). | Compare only after scaling the reference to the render width (`scripts/ui_compare.py` auto-scales); never compare raw-pixel counts across resolutions. | Measurement-side. | measurement-bank notes (per-capture scale factors). |
| C5 | **Live runtime state in the capture** — the screenshot shows a live value the static at-rest render cannot and should not reproduce: compass live heading (~132°, labels 100/120/140/160), radar live contacts + heading/range readout, self-status **seated** capture + SELECTED focus highlight, master-mode live weapon-group icon + "GUN" submode. **The principled at-rest state is canonical (owner 2026-07-08); divergence from a live-state capture is permanent-acceptable.** | The value is engine-runtime with NO static signal decodable from the canvas/records; the **principled at-rest value** (heading 0, unselected, powered-default mode) is the target. Prove absence of a static signal, don't assume it. | Invisible to budget (different STATE, not drift). | compass, radar, self, master-mode dossier rows (ui-reference §3). |
| C6 | **Physical-screen / Blender-shader-geometry characteristics** — CRT scanline overlay, bezel geometry, corner triangles/chamfers, and "aspect literalness". NOT part of the UI texture render. **General class, all ships (owner-confirmed 2026-07-08; first confirmed on the Carrack console 2026-07-04).** | Screen-mesh / Blender shader / cockpit geometry, not UI-render output; the UI crate is a texture source, not the final composited screen (workflow Purpose). | Out of the UI render's scope entirely. | Carrack console dossier row (owner-confirmed out of scope). |

### Renderer-caused → must-fix

| # | Class | Distinguishing detection | Status / tracking | Evidence |
|---|---|---|---|---|
| R1 | **Caption/heading line-box baseline offset** — heading text top renders ~5px HIGH of the reference. | `primary_text_top` band-profile vs the measured `reference_target`; the offset is stable across captures (not skew). | must-fix, **deferred** — registered known-outlier (`ui_target_a`, `frozen_value 73.0 → reference_target 78.0, confidence 2.0`). True fix = the caption/heading line-box baseline model. | `ui_known_outliers.json` (two header entries); A5 residual class. |
| R2 | **Fixed-heading font under-size** — a `Heading3` caption renders at nominal 28px where in-game is ~38px (~1.35× small). | SIBLING cap-height ratio (SUB DECK vs CALL ELEVATOR: render **0.636** vs ref **~1.10**); a width-match is meaningless for a FIXED (`autoScalingMethod:None`) heading. Button geometry matches in canvas space, so it is not aspect. | must-fix, **owner-deferred fix** (defect confirmed real, fix parked 2026-07-04). Carries a **hard-coding flag**: `compose/text_draw.rs:100-102` hard-codes `Heading1=>48, Heading3=>28` (AGENTS.md game-data-in-source ban) — verify whether the real brand Heading3 is failing to apply and falling back to 28 (the velocity/master-mode "authored-but-unapplied" pattern). Candidate fix = the compass height-driven-labels pattern (data-backed off field height). | ledger 107; Carrack console dossier row. |
| R3 | **Text-field vertical clip / line-box fit** — a second label line clips at the field's bottom edge. | Rendered glyph bottom exceeds the field's computed rect; reproduces in a fresh export (not a stale view). | must-fix, **open**. | Carrack console dossier row ("second label line clips … line-box/field fit"). |
| R4 | **Genuine blend-shaped colour drift at a composited/chiclet edge** (the former "lenient-until-B4" members). | Confirm the entry-driven colour TOKEN in IR, then discriminate the pixels by the **COUNT of near-pure-black pixels** (`max(r,g,b)<25`) in the isolated region, not a mean (anti-aliasing makes means unreliable); uniform-colour screens need connected-component isolation. | must-fix — root-cause and adjudicate via workflow §5 (the compositing GAP itself is CLOSED by B4; only a genuine remaining drift is a bug). Resolve colour from the entry-driven token, never a draw-time hack. | parity skill `references/blockers.md` "Colour compositing — linear-light LANDED"; ledger 107 measurement note. |

### Unclassified → pending-investigation

| # | Candidate | Status (owner 2026-07-08) |
|---|---|---|
| U1 | **Radar disc spoke prominence** — the spokes read softened. The disc is the REAL engine texture (`r_radarmapscreen_radial_gradients.dds`); the spokes are soft **in that source `.dds`**, so this is either capture-faithful (we render the softened source truthfully) or a renderer over-softening bug. | **pending-investigation** — the next radar arc measures the source `.dds` vs the render and classifies it properly. Until then it is neither accepted nor a defect. |

## The retired interim class

An earlier plan listed a third status, `lenient-until-B4`, for blend-shaped colour
residuals at composited edges judged leniently and parked as known-outliers pointing at
the linear-light arc. **That class no longer exists.** Plan B4 (renderer-wide
linear-light compositing) LANDED (`999d485ab`, all 15 gold/platinum baselines
re-frozen) and the interim lenient rule was RETIRED (`4f9ffc091`). Former members are
now classified by CAUSE like everything else: an over-bright composited edge is **C1
(permanent-acceptable)**; a genuine blend-shaped token/edge drift is **R4 (must-fix)**,
root-caused and adjudicated via workflow §5 — never registered as a lenient outlier.

## Capture provenance

The reference is imperfect and the classification above leans on knowing *how* each
capture was taken. Rules:

1. **Record provenance for every reference capture, when known: game build/version,
   capture date, and graphics settings.** Today's provenance carriers record capture
   PATH, PIXEL SIZE / scale factor, measurement METHOD and DATE — the measurement bank
   `crates/starbreaker-ui/tests/fixtures/ui_ir/reference_measurements_v1.json` + its
   `.notes.md` (measured 2026-06-12; per-capture resolutions noted, e.g.
   `Screen_Left_Lower_RTT.png` 1600×1200, `Screen_Right_Upper_RTT.png` 1959×1513), the
   `source` field of each `ui_known_outliers.json` entry, and the dossier reference
   column (`crates/starbreaker-ui/docs/ui-reference.md` §3). They do **not** yet record
   the game **build/version** or the in-game graphics settings a capture was shot
   under. Capturing that is the go-forward rule (new captures and new measurement-bank
   entries note it when known) so a future engine change can be told apart from a
   renderer regression.

2. **Known capture / measurement error classes** (consult before trusting any
   reference number):
   - *Bloom / emissive lift* brightens near bright elements and lifts the blue channel
     (class C1) — judge hue from ratios, not raw values.
   - *Skew* shears geometry (class C2) — rectify for position.
   - *Mouse-hover / cursor* artifacts add outlines/highlights not in the at-rest UI
     (class C3).
   - *Resolution mismatch* — scale the reference to render width before comparing
     (class C4).
   - *Live runtime state* — the capture shows a live heading/contact/focus/mode the
     static render should not reproduce (class C5).
   - *Rectification thin-feature colour smear* (ledger 35): the homography warp
     interpolates a ≤~4px feature (header bars, strokes, dotted separators) with its
     background, diluting the hue — a 2px Accent1 bar measured G/R 0.64 ≈ the Base
     anchor and was wrongly recorded "faithful". Rectify for POSITION; judge a thin
     feature's COLOUR on the CRISP ORIGINAL. `scripts/ui_measure.py` warns when
     `feature_width ≤ 4`.
   - *Same-hue band contamination* (ledger 68c): a fixed `--box` merges a digit band
     with a same-hue tick band below it, reporting a false cap-height ("25% cap,
     clipping" when it was 18.9% with a 27px gap). Use contiguous-band detection
     (`scripts/ui_measure.py --box` `bands`/`band_gaps`), not a single bbox.

3. **A capture-imperfection claim needs a distinguishing measurement, not an eyeball**
   (restated because it is the load-bearing rule). Judge hue from R-normalised ratios
   calibrated to a known same-capture anchor, cap-height from a sibling ratio, thin
   colour on the crisp original, thin-glyph colour by near-black pixel COUNT. If the
   measurement cannot separate the capture explanation from the renderer explanation,
   the residual is **unclassified/pending-investigation** — it stays open, it is not
   defaulted to "capture, acceptable".
