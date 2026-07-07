# Clipper UI parity — open items (handoff)

> Slimmed 2026-06-13. The landed-work NARRATIVE (the per-step prose and the
> commit table) was removed — that knowledge now lives in the code,
> `crates/starbreaker-ui/docs/ui-fallback-register.md` (retired
> constants/fallbacks), `crates/starbreaker-ui/docs/ui-architecture-runbook.md`
> (engine models + the "Open architecture debt" section), and the git log.
> What remains is the **open Clipper parity work** plus a one-line provenance
> index so existing `catalog #N` / `§N` citations still resolve. Process +
> commands: `crates/starbreaker-ui/docs/ui-workflow.md` +
> `crates/starbreaker-ui/docs/ui-reference.md`; the per-screen dossier
> (`ui-reference.md` §3) maps each screen to its scene/canvas/preset. Fresh
> sessions: instantiate `crates/starbreaker-ui/docs/ui-matching-agent-prompt.md`
> with `SCREEN=Screen_Left_Lower_RTT`, `HANDOFF=` this file.

Reference images: `~/projects/scorg_tools/reference/in-game/Clipper/`
(`Screen_Left_Lower_RTT.png` power, `Screen_Right_Upper_RTT.png` target,
`screen_16x9_a-[medical1].png`, `mesh_end_screen_plane-[medical2].png`). The
red/white pip outlines in the power reference are mouse-hover artifacts —
ignore (owner's rule); captures are imperfect (skew/bloom) — compare
structurally (workflow §4).

## Diff catalog (power screen vs reference) — status

| # | Region | Difference | Status |
|---|---|---|---|
| 1 / 1b | Emissions | header collapse + emitted/ambient overlap | **FIXED** (SizeY behaviour + column zero-Auto text intrinsics/shrink) |
| 2 | Emissions | IR/EM/CS labels rendered `@LOC_PLACEHOLDER` | **FIXED** (clone FieldModifierLocalization → `_SynthLocalizedWidget_`) |
| 3 | OUTPUT card | title flow (icon→dots→title left-aligned) | **FIXED** (draw-metrics intrinsic measure, `pipeline/text_measure.rs`; measure == draw) |
| 4 | Battery card | OFFLINE text overflowed card | **FIXED** (draw-width box + shrink) |
| 5 | Cards/pips/gauges | icon/colour tints, pip brightness, "2" white | **MOSTLY LANDED**; open residuals below (P4, P13, IR/EM/CS) |
| 6 | Footer/scrollbar | faint track + backdrop band | **OPEN** (A7 class, below) |

## 2026-06-13 — user-symptom triage (Screen_Left_Lower_RTT)

Four reported symptoms, mapped to root cause (one landed, three already-open):

1. **Pip gauge "clipped right / container ~30px wider / slider wider"** —
   container width is **FAITHFUL**: `canvas_PowerSystems` is authored `0.7`
   Percent and its parent `base_PowerAssignmentListContainer` flexes Row with
   `axisJustification: "Center"`, so the 0.3+0.7 children centre with ~15px each
   side (the "30px"). The col-3 heat bar (authored right 1528) is fade-clipped by
   the `Scrollview` (overflow Clip, right 1499) — engine-authored list behaviour;
   its fill still renders to x≈1512. The reference's right extent measures to
   x≈1580, *past* the screen edge (1559) = capture bloom, not a real gauge. Do
   NOT widen/rejustify (would hack authored data). The "slider wider" half = **P7**,
   now **FAITHFUL**: the thumb formula (viewport/content × track) is already correct
   in `apply_scroll_thumb_rects`; 393px matches the DataCore-derived 7-column
   content; the reference 432 is bloom on the orange slider. No fix.
2. **Header side bars red + further out** — POSITION **LANDED** (SpaceBetween) +
   COLOUR **LANDED** (shared-tier `BackgroundColor` no longer overrides the bars'
   authored Accent on custom shapes) — both see P13. Minor ~30px edge-position
   residual open.
3. **Output-card dotted separator** + **4c battery dotted separator** — empty
   `BuildingBlocks_WidgetSeparator` hosts (power titles id=604/607, emissions
   Sep*) with `asset_layout` but no `custom_shape`/`asset_ref`/children — need the
   widget-standard expansion = **P3** (parked; ID-band + brand blockers).
4. **Battery card** — re-measured 2026-06-13 (rectified `Screen_Left_Lower_RTT_dark`,
   battery-column text-band centroids). 0/0 Δ−8px and BATTERY Δ−3.5px are within
   capture noise (so rectification is accurate AND those two are faithful); the
   OPPOSITE-sign OFFLINE Δ is therefore a real LOCAL deviation, not skew:
   - (a) **OFFLINE ~35px too low → FIXED** (`be1859d59`). Was render centre y≈841
     vs ref y≈806; cause: `base_ValuesContainer` (`0.9 Auto`) FILLED 0.9/(0.9+0.5)
     of the 220.8px column = 141.9px on one "0/0" line, leaving ~71px empty that
     pushed the OFFLINE box down. Fix: a column child authored non-zero `Auto`
     content-fits its TEXT (`engine_01.part` ~L1052), exactly like the 0.0 case —
     "Auto" = fit-to-content, the value is only the no-content fallback. ValuesContainer
     →93px; OFFLINE re-measured Δ−5.9 (uniform with 0/0 Δ−3.8, BATTERY Δ−4.3 = faithful).
     Full detail + why the paper analysis wrongly read it as blocked: **P14**.
   - (b) **BATTERY overflows the card ~21px** — title `text_BatteryTitle` is
     `Auto`-width (256.7px, no shrink) in an authored `Start` row, starting at
     x=274.4 IDENTICAL to OUTPUT's *fitting* title (icon box + separator slot are
     byte-identical between the two headers — same 79.6 square, same 0.1×0.6
     separator). The icon is **NOT stretched** (re-checked per owner report
     2026-06-13): painted aspect 1.38 ≈ ref 1.37 ≈ the SVG's natural ink ratio
     (`MFD-Icon-Battery.svg` glyph bbox 142.2/103.8 = 1.37); `Contain` fits it to
     the 79.6 box width (77.8×56.8) correctly — the icon does NOT shift the title.
     The overflow is purely "BATTERY" being the LONGEST title at the **P8** letter
     pitch (~6–8%; the longest word overflows first). Font size, icon, title start
     all faithful. P8 stays gated on a clean (un-bloomed) reference.
   - (c) = item 3.

## Open items

### P3 — separator dots (power cards) — LANDED (`af9ce13d3`)

The OUTPUT/BATTERY card dotted dividers now render, matching the reference, with
the medical platinum byte-identical. Resolution (differs from the parked plan
below — no widget-standard EXPANSION/ID-band was needed):
- **No render-path expansion.** A `WidgetSeparator` already has IR/compose
  handling; the gap was the brand SVG + a render dispatch. `separator_standard_style_from_source`
  resolves the brand by manufacturer family (`manufacturer:drak` → `s_drak_env`)
  for the SVG, and `draw_node` routes an `asset_ref`-svg separator to the
  rasteriser.
- **Blocker B solved by the MFD-frame gate, NOT a typography resolver.** Colour
  matches the canvas's DIRECT slug (unchanged); the dotted SVG resolves ONLY when
  `design_text_scale > 1` (MFD frame). Physical interior screens (medical, door)
  match nothing → no separator → frozen platinum untouched. No AEGS leak because
  the medical footer isn't reached for the SVG.
- **The DRAK asset is ABSENT in this build.** `s_drak_env` overrides SvgPath to
  `DRAK_S42_…` which doesn't exist (only AEGS/BIOC/CRUS/… ship vectors). The
  engine falls back to the standard's DEFAULT non-brand SvgPath —
  `PU_MFD_Generic_…_V_Divider.svg` (a column of 8 dots). `separator_default_svg_path()`
  pulls that default and the resolver prefers it for MFD separators.
- **Round dots, not dashes:** Contain-fit + centred (`asset_layout` Contain, no
  `custom_shape` so `rasterize_svg_for_node` takes the aspect-preserved path).
Verified: `ui_check --full` ALL GREEN; 527 lib tests; 5 frozen targets
byte-identical; guard `compile_ir_mfd_separator_paints_default_divider_but_skips_physical_screens`.
The parked plan below is kept for history (the ID-band/typography blockers it
worried about never materialised under the MFD-gate approach).

### P3 (original parked plan — superseded, kept for history)

The dotted icon/title separators are `BuildingBlocks_WidgetSeparator` widgets
(direction Vertical, style Tertiary on the power cards) — a modularkit
standard family: 6 records `modularkit/standard/widgets/{vertical,horizontal}
separator{primary,secondary,tertiary}widgetstandard.json`, each a single
ComponentRoot WidgetCustomShape whose PER-BRAND container authors the visual
(drak env: `DRAK_S42_seperator_vertical_2.svg`, EnableColorOverlay=false,
nine-slice). A working implementation was built and REVERTED for two blockers
(both are now architecture debt in the runbook):

1. **bb_scene**: add `BbNodeType::WidgetSeparator` (+3 exhaustive matches:
   bb_layout `type_name_str`, ui_ir `node_type_name` "widget_separator",
   compose `draw_node` no-op host arm) and convert the 3 existing
   `Other("BuildingBlocks_WidgetSeparator")` matches in ui_ir engine_02.part;
   add `"Separator"` to `node_type_matches`.
2. **Expansion** (bb_resolve engine_04 `expand_widget_standards`): include
   WidgetSeparator hosts, template by direction/style, no params. **BLOCKER A
   → runbook "per-host-type ID-band lanes":** instances consume the shared
   `0xF000_0000` band counter and SHIFT frozen platinum instance ids (the
   medical close-button X 4026531855 became a separator).
3. **Brand application** (`apply_separator_standard_styles`, engine_01.part,
   subtree-scoped via `apply_scene_style_entries_in_subtree`): **BLOCKER B →
   runbook "one brand-context resolver" (RESOLVED — B1):** exact canvas-selected
   id + hud↔env sibling works for power (`s_drak_hud`→`s_drak_env`) and the
   medical bed, but the medical FOOTER selects `s_aegs_env` and matches the
   standard's `s_aegs_env` ⇒ AEGS divider leaks into platinum. Now resolved by
   the unified identity resolver (`resolve_brand_identity` +
   `brand_class_for_canvas` by canvas family; identity matching only, no prefix
   scan) — the legacy `resolve_brand_style` scan is deleted.
4. The overlay-icon default must respect a styled `EnableColorOverlay=false`
   (PascalCase raw override) so the separator SVGs are not
   MissionObjectives-tinted.
5. Test fixtures need the tag database served (expansion bails without it).

**2026-06-13 verification (owner asked to confirm medical stays dot-free).** The
power separators (`gen_mc_s_poweroutputinfo`, `separator_OUTPUT/BatteryTitle`) are
bare hosts: `styleTags=[]`, `isActive=true`, `direction=Vertical style=Tertiary`.
The medical components (`i_medicalcommon_components/*`: TopSeparator, BottomSeparator,
injury/clone/text dividers) are **byte-identical in form** — `styleTags=[]`,
`isActive=true` (mostly `Horizontal` Secondary/Tertiary). Every brand in each
standard carries an `SvgPath` modifier (`s_drak_env`→DRAK dots, `s_bioc`→BIOC,
`s_aegs_env`→AEGS …). So there is **no authored discriminator** between
"should render" (power) and "must not render" (medical): a type-based expansion
draws BOTH, changing the frozen medical platinum. The in-game medical bed DOES show
its own (horizontal) dividers, so the real rule is the **brand-context resolver**
(which separator renders in which canvas family) — Blocker B is load-bearing, not
optional. Until it's solved, the feature stays parked; a naive expansion is a
medical-platinum regression. CONFIRMED: cannot render the power dots in isolation
without the resolver.

### P14 — non-zero-Auto column model (4a OFFLINE) — RESOLVED (`be1859d59`)

A column child authored non-zero `Auto` (value v∈(0,1)) now **content-fits its
text** (`engine_01.part` ~L1052), exactly like the 0.0 pure-hint case: "Auto" =
fit-to-content, the value is only the no-content fallback fraction (`else` fill).
`base_ValuesContainer` 141.9→93px; OFFLINE Δ+34.7→−5.9 (uniform with 0/0 −3.8,
BATTERY −4.3 = all faithful). `ui_check --full` green, all 5 frozen baselines
unchanged, +2 TDD guards.

**Lesson — the empirical probe beat the paper analysis (owner was right to push).**
My first pass concluded "blocked, needs the engine spec" from this table:

| model | POWER OFFLINE (ref ~806) | MEDICAL HeaderTitleBase (frozen 78, content ~18) |
|---|---|---|
| fill `v×container` (old) | 842 (−35) | 78 |
| content-fit | **758 (paper) → actually 801** | **18 (paper) → actually UNCHANGED** |

Both paper cells were wrong: (1) I estimated the "0/0" content basis at ~71px (the
text-field height); the real measured text intrinsic is **93px**, which lands
OFFLINE at 801 ≈ ref 806, not the predicted 758. (2) I assumed medical's
`HeaderTitleBase` would shrink — but it is a **ROW child** (it has a vertical
separator sibling), so its h is CROSS-axis and the COLUMN `!is_row` branch never
touches it. The "78→18" §10 regression came from a BROADER earlier change; this
narrow one (only text-measurable non-zero Auto in `!is_row`) leaves every frozen
target byte-identical. Takeaway: when a model is "between" two analytic results,
run it behind the full guard rather than reasoning about content sizes — measure,
don't estimate.

### P4 — pip slab brightness

Pip slabs render saturated orange vs pale/washed in the reference. Likely the
pip fill colour role (Bright vs Base) combined with capture bloom — investigate
the IR tint token against a SAME-capture anchor (workflow §4 photometric
method) before changing a role; do not chase the bloom.

### P13 — header side bars (emissions Sep1/Sep4)

Two parts. **POSITION — LANDED 2026-06-13:** the bars sit at the emissions
header edges via authored `axisJustification: "SpaceBetween"` on
`base_EmissionsContainer` (GEN_MC_S_Emissions). `bb_layout` did not implement
the space-distributing modes — `SpaceBetween`/`SpaceAround`/`SpaceEvenly` fell
through the `main_offset` match to `_ => 0.0` (Start), left-packing the row, so
Sep4 sat at x≈1425 with ~104px of dead space on the right. Implemented the three
space modes in `bb_layout/engine_01.rs` (slack = avail−total_main,
shared as equal gaps; test `flex_row_space_between_spreads_children_to_container_edges`).
The IR/EM/CS groups now spread and Sep4 reaches the content-right edge (x≈1510,
inside the container's ~17px padding) — matches the reference. Clean across all
three suites (lib + IR-snapshot + whole-image visual after re-export); no frozen
target uses a slack-bearing space mode, so nothing shifted.

**COLOUR — LANDED 2026-06-13 (shared-tier background suppression).** The visible
edge bars Sep1 (x≈88) and Sep4 (x≈1510) are solid-fill `WidgetCustomShape`s that
AUTHOR `background.color` = **Accent2 (Sep1) / Accent1 (Sep2/3/4)** — i.e. red.
They rendered orange because the shared `mfd_g_emissions` **"New Style"** entry
(`ConditionParent`→`AnyOfTag{80c85b19(sep1),21cf313d(sep4)}`, `BackgroundColor=Base`)
overrode the authored accent → `Base`; Sep2/Sep3 (not matched) kept `Accent1`.
(Earlier mis-called "faithful" — the rectified reference smears the 2px bar into the
orange screen-grid, lifting G/R to 0.64≈Base; the CRISP original measures G/R≈0.52,
B/R 0.31≈Accent1, and the owner confirmed visually.) FIX: a **shared-tier**
(`Tier::Shared`) `BackgroundColor` modifier is suppressed when the target is a
`WidgetCustomShape` that already authors an enabled `background.color` — a generic
shared sheet is not the styling authority for a shape's intrinsic fill; brand /
embedded / inline tiers still override (`bb_brand_apply::shared_background_override_suppressed`,
threaded via `apply_sheet`'s `sheet.tier == Tier::Shared`; test
`shared_tier_background_color_keeps_custom_shape_authored_colour`). Rendered bars
now G/R 0.41–0.47 (red, matching). Validated clean across all three suites (lib +
IR-snapshot + whole-image visual after re-export) — no frozen target regressed
(the gate is scoped to custom-shapes, leaving card backgrounds like the MFD footer's
authored-Base→styled-Disabled untouched). This was NOT the brand-context resolver
and NOT the overlay default. RESIDUAL (minor, open): the bars sit ~30px inside the
reference edge position (content edge vs container edge) — likely capture bloom;
not chased. Sep2/Sep3 zero-size is faithful for DRAK.

### P7 — scrollbar slider width

Orange scrollbar slider measures ~393px vs reference ~432px. The driver is the
engine `_SizeRatio` input (CLIK thumb formula = viewport/content; plan P2.2b);
the slider-width math suggested ~0.7×padding-box but the card-width / scroll
position evidence is capture-skew-limited. Defer until the `_SizeRatio` source
is decoded.

### P8 — footer letter-spacing

Footer letter pitch is ~6% off (long-standing) — a global SWF `LetterSpacing`
model question, not a power-screen-local fix. Defer (documented).

### Close-button outset border (medical bed; target side-buttons in blast radius)

The frame node (`ComponentRoot`, authored 64×64, border 3px, radius 6) draws
its border OUTSET in-game (content-box: 64 + 2×3 = 70 visible; the reference
measures 68–70 at the aligned right edge); ours draws inset (64 visible).
**A w/h known-outlier is the WRONG vehicle** — outset borders are paint-only,
so the snapshot rect stays 64 and would never graduate. Implementation = outset
border drawing scoped to expanded-standard component roots (own
`canvas-proxy-root` tag); blast radius includes the GOLD target master's two
212×105.6 side buttons (3.33px borders) — measure those frames on the target
reference before landing, then artifact-adjudicate every target.

### IR/EM/CS abbreviation colour

In-game the IR/EM/CS abbreviations keep the H1 deep orange; ours get recoloured
to slot-0 Base by the shared `mfd_g_emissions` `Icon Color` entry (a normal
match). Evidence (incl. the footer `SelectedName` comment in `conditions.rs`)
suggests shared-record colour entries do NOT restyle textfield text — this
needs its own scoped rule (shared-tier colour entries skip text-format
restyling) plus a battery adjudication.

### A7 — backdrop band class

A faint footer track + backdrop band remain on the power footer and the target
status area. Partially resolved by the audited gold re-freeze (the
`clipper_target_master` `40:widget_custom_shape` SizeY 0.18 Percent → 194.4
band, alpha 0.1); the remaining A7 backdrop stack (faint track + band visible
pixels) is the open residual. Target dossier tracks it as "A7 backdrop stack
remainder".

### Linear-light compositing — LANDED (B4, 2026-07-07)

The glow (and every antialiased edge / translucent overlay) previously
rendered darker than the reference because the engine composites in LINEAR
light and the renderer blended in sRGB. The **renderer-wide** migration to
linear compositing has landed with NO carve-outs; the earlier scoped
white-mask path (`blit_white_mask_overlay_linear`) was folded into the
now-linear general blit and deleted. All 15 targets were re-frozen with the
geometry-IR snapshot byte-identical (colour-only). Full detail is in the
runbook's **architecture** section (the linear-light bullet).

### Minor — OFFLINE cap height

The battery OFFLINE glyph cap measures ~43px in the reference vs ~49–53
predicted (band-fit?) — unexplained; recorded in the measurement bank
(`crates/starbreaker-ui/tests/fixtures/ui_ir/reference_measurements_v1.notes.md`).

## Landed — provenance index

One line per landed step (full narrative in git; engine models in the runbook,
constant/fallback retirements in `ui-fallback-register.md`). Kept so the
`catalog #N` and `§N` citations elsewhere in the repo still resolve.

| Ref | What landed | Commit(s) |
|---|---|---|
| catalog #1/1b | emissions header collapse + emitted/ambient stacking | (Step 6 + flex shrink) |
| catalog #2 | IR/EM/CS labels via clone FieldModifierLocalization | (clone_expand) |
| catalog #3/#4 | draw-metrics intrinsic measure (measure == draw); OUTPUT/OFFLINE | (`pipeline/text_measure.rs`) |
| §5 | tint/defaultStyles cascade re-land; token-only fills | 79c87e8ea |
| §6 | medical: white X, Bioticorp logo −12px, position outliers registered | 4f8532f4e |
| §9 | annunciator rounds 2–3 + MFD body backplate | 81c93109e, f436b446f |
| §10 | annunciator round 4 — near-black bg + white-mask Base overlay | 16d595dbb |
| §11 | annunciator round 5 — WPN glow off + linear mask glow | f5040aeab |
| §12 | power review round 2 — P1 "2" cream, P2/P5 MissionObjectives icons | (P1 + next) |
| §13 | power review round 3 — per-axis Overflow clip; text-format route (open: OFFLINE cap 43) | a30761f20, 07c821a83 |
| §15 | text-format route landed + hard-coding remediation | 07c821a83, 4803d3c48 |
| §16 | MFD text size — host-path `imageSizePercent` division | 15d1e3b99 |
| P13-pos | header side bars reach edges — `bb_layout` SpaceBetween/Around/Evenly (were unimplemented → Start) | (2026-06-13) |
| P13-col | header side bars red — shared-tier `BackgroundColor` no longer overrides a custom shape's authored Accent (`shared_background_override_suppressed`) | (2026-06-13) |
