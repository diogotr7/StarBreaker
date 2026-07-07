# UI Architecture and Troubleshooting Runbook

> Satellite doc — the authoritative process is `crates/starbreaker-ui/docs/ui-workflow.md`; commands/tools/data live in `crates/starbreaker-ui/docs/ui-reference.md`.

## Architecture Summary

The UI pipeline is split into four stages:

1. Source resolution
- Resolve BuildingBlocks canvases, styles, bindings, and localization.
- Files: `bb_resolve/`, `bb_state_filter.rs`, `bb_bindings/`, `bb_style_engine.rs`
  (the single style-cascade application engine, P4) + `bb_brand_apply/`
  (its condition/modifier kernel).

2. Canonical IR compilation
- Compile deterministic `UiIrDocument` output with fidelity fields and provenance.
- File: `ui_ir.rs`.

3. Renderer consumption
- Render from IR only (no source-data probing in renderer).
- Files: `ir_compose.rs`, `hybrid_compose.rs`, `compose.rs` compatibility wrapper.

4. Regression/certification
- Structural snapshot extraction and comparison for representative families.
- File: `ui_snapshot.rs` and example `phase5_certification_dashboard.rs`.

## Why Ruffle/Flash playback is a dead end here (attempted twice, removed twice)

Recovered from the Copilot-era session logs (2026-05) after the question
resurfaced; recorded here so a third attempt starts from the evidence:

- The runtime SWFs (`Data/UI/BuildingBlocks/assets/SWF/BuildingBlocks_root.swf`
  ~113 KB, `Canvas.swf` ~13 KB) are a **draw surface, not authored screens**.
  `BuildingBlocks_root.swf` carries 127 exports — ALL `__Packages.*`
  ActionScript classes (`bhvr.*`, `gfx.*`, `caurina.*`); **shapes=0,
  bitmaps=0, fonts=0** (verified in commit `aa5cb043d`). Opening them in
  Ruffle shows an **empty stage**: the engine pushes draw commands in from
  the C++ BuildingBlocks runtime via ExternalInterface at run time.
- Playing them therefore requires faking the ENGINE side of that interface —
  i.e. reimplementing BuildingBlocks anyway, with a Flash VM in the middle.
  "The earlier Phase R / Ruffle plan was based on the wrong mental model";
  the project rule that followed was "**No Java, no Ruffle** — the earlier
  additions of these were removed."
- What DOES work and is already in the tree: Ruffle's **`swf` parser crate**
  (not the player) powers `starbreaker-swf`'s font extraction —
  `DefineFont2/3` → TTF, verified **bit-exact** (12 glyphs / 8 fonts, zero
  differing pixels) — and the hybrid path renders the static authored SWF
  content that exists (e.g. `TargetStatus.swf` `DefineEditText` HTML, which
  carries the authoritative typography: `$Furore`, size, letterSpacing).
- The still-open opportunity is **static ABC/ActionScript MINING** (read the
  `__Packages.*` bytecode for constants/formulas — the measured 44px
  content-view inset, the scrollbar slider math, text-scale rules), which is
  analysis, not playback.

### AVM1 mining results (plan P2, 2026-06-12 — `examples/swf_avm1_dump.rs`)

The mining ran: full pool/push dumps of every `DoInitAction` stream, plus
an `--ops <class-substring>` opcode trace for reading formulas. Findings:

- **Content-view inset (44/1192/676): NOT in any SWF.** All 127
  `BuildingBlocks_root.swf` classes dumped — no such pushes (the only 44s
  are a keycode-style table in `gfx.core.UIComponent`). The placement is
  computed on the C++ host side; `mfd_view.rs`'s constant stays a measured
  pin with this bound recorded.
- **Scrollbar thumb formula CONFIRMED:** `gfx.controls.ScrollIndicator.
  updateThumb` = `max(10, pageSize/max(1,(maxPos−minPos)+pageSize)×track)`
  — reduces to our `viewport/content × track` for pixel inputs. The BB
  scrollbar standard, however, binds its bar SizeX to the engine-pushed
  `_SizeRatio` component parameter, so the power P7 residual (ratio 0.402
  rendered vs 0.440 in-game) is a C++ input difference, not formula error.
- **Text sizing: AS2 applies sizes VERBATIM** (`bhvr.utils.
  TextFieldContainer` sets `TextFormat.size = _fontSize` with no scaling;
  no fontLib/textScale handling anywhere) — the `imageSizePercent`
  host-path division happens below the SWF layer, in engine rasterisation.
- **`fonts_en.swf`: zero action tags** — a pure DefineFont3 container; no
  layout constants.
- **Per-screen content SWFs (`TargetStatus.swf` RSI/AEG 16-9 + DRAK
  Dragonfly bespoke): no layout constants.** They embed the same `bhvr.*`
  framework subset (66–83 classes); the only app-level numerics are
  time/angle math (60/360/3600) and `0xFFFFFF`. Their value remains the
  static `DefineEditText` typography (above), not bytecode.
- **CONFIRMED 2026-06-13** (plan P3.3, strings-scan of the live
  `StarCitizen.exe`): BuildingBlocks text rasterises via Terathon's **Slug**
  GPU text engine — `Terathon::Slug::FontHeader`/`AlbumHeader` via
  `CPaintManager`, `CSlugPipeline::{Execute,SubmitPolygon}`, `eVF_SLUG`,
  and `"[BuildingBlocks] ... incomplete SLUG format directive tag"`. Glyph
  rasterisation is Slug curve rendering, not GFx; the SWF files supply font
  DATA only. (The former `SWF_TEXT_RENDER_SIZE_CALIBRATION = 0.84` this
  lead was recorded for is retired — the inline-pair width it served now
  measures at draw size.)

## Reference: BB_ColorStyle colour roles

BuildingBlocks colour tokens (`BuildingBlocks_ColorStyle.color`, e.g. `Base`,
`Bright`, `Accent1`) are the `BB_ColorStyle` DataCore enum. The enum's integer
value is the **direct index into a brand style's `colorStyles` palette array**.
Authoritative order, dumped from `Data\Game2.dcb` via
`starbreaker_datacore::database::Database::{enum_defs, enum_options, resolve_string2}`:

| idx | role | idx | role | idx | role |
|----:|------|----:|------|----:|------|
| 0 | Base | 6 | Bright | 12 | ContactPositiveRep |
| 1 | Positive | 7 | Selected | 13 | ContactNegativeRep |
| 2 | Moderate | 8 | Disabled | 14 | ContactAgressive |
| 3 | Critical | 9 | Background | 15 | ContactUnknown |
| 4 | Accent1 | 10 | ContactNeutral | 16 | MissionObjectives |
| 5 | Accent2 | 11 | ContactParty | | |

Key facts:

- **The screen `background` is slot 9 (`Background`), not slot 8
  (`Disabled`).** The loader read slot 8 until 2026-06-13; adjudicated
  against two dark-room captures of the Clipper power MFD (plan P5.3 —
  dark-region ratio analysis, both captures favour slot 9; drak background
  is (38,27,10), not (20,13,5)). `StyleLoader::
  parse_buildingblocks_style_record` + the loader test pin slot 9.
- **`Bright` (6) is a muted role, distinct from `Base` (0).** Example: a
  `ComponentLabelCaptionPair` label uses `Heading3`→`Base` while its caption
  value uses `Heading6`→`Bright` (e.g. medical "MEDGELS" light blue label over a
  light-grey "200/200" value). Named text-style colours live in the standard
  textfield widget's per-brand `brandStyles[].entries[].modifiers[FillColor]`
  (`textfieldwidgetstandard.json`), parsed in `ui_ir` (`StandardTextStyle`,
  `brand_text_style_colour_token`).
- **The renderer's token→slot resolvers are role-aware, not pure enum-indexing.**
  `bb_brand_apply/colors.rs::color_style_slot_index` and
  `ir_compose` `resolve_colour_token` / `resolve_surface_colour_token` map the
  same token to different slots depending on whether it paints a *foreground*
  element (icon/text/glyph) or a *surface* (filled shape / chrome). The clearest
  case is `Accent1`: foreground → light slot 0, surface → darker slot 4 (the
  medical fingerprint). When extending the mapping, verify against an in-game
  reference per role; do not assume `slot == enum index`.

## Standard Validation Commands

- Full UI crate checks:
  - `cargo test -p starbreaker-ui`
- Hardcoding guard:
  - `bash .github/scripts/check_ui_hardcoding.sh`
- Family certification drift check:
  - `cargo run -p starbreaker-ui --example phase5_certification_dashboard`

## Fast Render Iteration: `ui render` replay

Do NOT run a full entity export to iterate on UI rendering. Once a
decomposed export exists, replay every UI binding through the SAME live
DataCore + P4K fetchers in about a minute:

```bash
target/release/starbreaker ui render \
  --scene "<workspace>/ships/Packages/DRAK Clipper_LOD0_TEX0/scene.json" \
  --out-dir /tmp/ui_replay
```

This renders all bindings (43 on the Clipper) as
`<helper_name>_TEX0.png` / `<canvas_guid>_TEX0.png`. A full export is only
needed when the binding set itself changed or to refresh the canonical
`ships/Data/UI/Generated/...` PNGs for the regression artifact freeze.

Prefer the wrapper `bash scripts/ui_render.sh --helper <name>` (or `--screen
<screen_id>`) for single-screen iteration: it rebuilds first and prints the
binary mtime (stale-binary guard), resolves the scene from the screen dossier
(`crates/starbreaker-ui/data/ui_screen_dossier_v1.json` — so non-Clipper ships
resolve their own scene), and writes each run to a unique
`/tmp/ui_render/<helper>/<UTC-stamp>/` with a `latest` symlink.

**Trap:** the full export writes its Generated PNGs near the END of the
run. Diffing a PNG right after its mtime first changes can read the
PREVIOUS export's bytes. Wait for export completion before diffing.

For IR-structure questions (node rects, tags, text payloads) use the MCP
`ui_ir_query` tool — but remember the deployed MCP binary may predate
your working-tree changes; treat its layout numbers as stale and only
trust its structure/names, or rebuild+redeploy the MCP first.

## Reference: MFD frame geometry model

A ship MFD render (binding_kind `mfd`) composes through the hosting GFx
stage of `BuildingBlocks_root.swf` (1280×720). Established, verified
constants (all measured against Clipper in-game captures and
cross-checked between two screens):

- **Stage → render-target scale** is per-axis: 1600×1200 target →
  (×1.25, ×1.6667). Text pixel size = `FontSize × max(sx, sy)`
  (`pipeline/host_stage.rs`); geometry does NOT use this scale.
- **The bound content view is hosted in a stage sub-rect** inset 44
  stage-px from the left/right/bottom edges, flush top (runtime
  ActionScript behaviour of `BuildingBlocksView`, not authored in any
  record; constant in `mfd_view.rs`, applied by `apply_bound_mfd_view`
  sizing the landscape slot to Percent(1192/1280, 676/720), anchor/pivot
  (0.5, 0)). Frame chrome (header/footer) outside the slot stays
  full-bleed.
- The frame canvas chain is `M_MFD_Screen` (800×600, full-bleed slot) →
  `m_eng_mfdcontent` (content + 0.11-height footer overlay) → bound
  screen canvas (1920×1080 or 1600×900 content).

## Reference: at-rest binding semantics (static renders)

A static render substitutes for live engine data with these rules
(verified against in-game captures):

- An **unbound variable holds its type default** (bool false, int 0).
  `BindingsIntegerFromBoolean` etc. evaluate accordingly
  (`bb_bindings/eval_numeric.rs`).
- A boolean component parameter **wired to an unbound variable** takes
  the type default `false`, NOT the authored editor `defaultValue`
  (`bb_bindings/param_overrides.rs`). Unwired parameters keep the
  authored default. Integers are deliberately excluded — the footer's
  `bindingid == selectedmfd` unbound-sentinel guard
  (`is_unbound_integer_param`) depends on unresolved integer params.
- A boolean component parameter **named after an authored
  `staticVariables[]` entry** takes that variable's authored value
  (case-insensitive; `bb_state_filter/eval.rs`). Canvases author their
  at-rest state this way (e.g. the power screen's
  `engineeringoverride`/`PresetNotification = false` while the editor
  defaults are `true`).
- **Do NOT add a generic merged-scene IsActive-binding pass.** It was
  tried and reverted: reference captures are LIVE states, and e.g. the
  medical screens' session elements are IsActive-gated chains that are
  false at static rest but true in the captured state. Visibility
  gating lives in `bb_state_filter`'s capture-tested heuristics
  (`Instantiated`/`IsActive`/`Visible`/`Enabled` fields).

## Reference: widget-standard template expansion

`WidgetIcon` and `ComponentGeneralButtonSecondary` hosts expand the
engine's standard template canvases in `bb_resolve`
(`bb_resolve/engine_04.rs`, `expand_widget_standards`): synthetic component
params from the host's authored properties, implicit framework tags
("icon", "general-button-secondary") resolved from the tag database by
name, host icon identity forwarded onto the template's icon instance,
and the host's parse-time icon fallback cleared. Template nodes are
allocated in a **reserved node-ID band (`0xF000_0000+`)** so injecting
them at any depth never renumbers sibling canvas nodes (platinum
snapshots key elements by node ID). sk-brand chrome (`sk_<brand>_
buttonsecondarystyles`) supplies padding/borders/corner radii; a padded
parent's content box caps fixed-size children in layout (overlay and
flex no-grow paths).

## Reference: engine models settled by the 2026-06-12/13 arcs (ledger item 24)

Capture-derived models that previously lived only in code comments and
handoffs (the 2026-06-13 text-calibration arc, plan P3, added the em /
advance / line-box trio below; the cascade-unification arc, plan P4,
added the engine note at the end):

- **Padding × canvas geometry scale.** Authored TRBL padding scales with
  the canvas's geometry scale to the render target — the 800×600 MFD
  content canvas renders its authored paddings ×2 on the 1600×1200 RTT.
  Evidence: the power screen's pip-top/stride values, pinned by the
  frozen power pins after the content-scale fix; owning code in
  `bb_layout`.
- **The text-format style route + literal-match precedence.** A
  Parent-wrapped style entry on a **Brand-tier** sheet styles a
  textfield's TEXT FORMAT (FontSize/FillColor) rather than the widget;
  only a text-format-routed FontSize (`__EntryFontSize`) outranks the
  named-style table — a LITERAL widget match does not. Counterexample
  that pinned it: the medical header T3 (commit `07c821a83`). (P4.3
  replaced the original `s_*`-identifier-prefix trigger with the
  explicit `Tier::Brand` gate in `bb_style_engine`.)
- **Text sizing is the design-em model — NO calibration constant.** The
  IR font size IS the design-em pixel size (em = ascent + |descent|);
  the SWF renderer maps em→raster via the font's own `units_per_em`, and
  the TTF (DejaVu) fallback's rusttype `Scale` already normalises to the
  same span, so a 30px field measures ~30px tall with factor 1.0. Plan
  P3.2 retired the tuned `TEXT_RENDER_SIZE_CALIBRATION` /
  `LAYOUT_TEXT_MEASURE_CALIBRATION` = 1.5 pair (calibrated when DejaVu
  stood in for game fonts on live screens — a dead case: the shared
  `fonts_en` fontlib merges into every binding). Glyph rasterisation is
  **Terathon Slug**, confirmed from `StarCitizen.exe` strings (see the
  Ruffle section); the SWF carries font DATA only.
- **Inline nested-textfield continuation = advance at draw size.** A
  child textfield with pivot.x≈1 / anchor.x>1 / Center valign that shares
  its parent's style continues the parent's inline run: its origin is the
  parent's glyph-advance end measured at the parent's ACTUAL draw size,
  and the glyphs' own side bearings supply the visible gap (medical1
  "T3"→"M" ink gap ~3px, letter-gap scale — NOT a typeset word space).
  Plan P3.3/P3.4 retired the `SWF_TEXT_RENDER_SIZE_CALIBRATION = 0.84` +
  `INLINE_NESTED_TEXTFIELD_WORD_GAP = 0.33` pair.
- **Caption-pair line-box stack.** A `ComponentLabelCaptionPair` stacks
  label over value in a flex Column with `rowSpacing = 0`; the engine's
  line box IS the em box (line advance == font size), so the value's line
  top sits exactly one label-em below the label's — no overlap
  subtraction, no tuned spacing. Verified: medical1 MEDGELS→200/200
  top-to-top 29px = capture. Plan P3.4 retired
  `LABEL_CAPTION_PAIR_FLEX_ROW_SPACING = -8.0`. (Right-anchored pairs
  carry a registered top-padding pin compensating the med2 slot rect —
  see the fallback register.)
- **Host-path `imageSizePercent` division.** On the GFx-host (framed
  MFD) path, EVERY font-size class divides by the font record's
  `imageSizePercent` (0.75 → ×4/3) at draw; non-host canvases use styled
  sizes verbatim. Verified per element on both MFD captures (commit
  `15d1e3b99`); AVM1 corroboration: the AS2 framework applies
  `TextFormat.size` verbatim, so the compensation is engine-side
  (`apply_font_image_size_percent` in `ui_ir`, plan P2.2c).
- **The additive-haze photometric model.** Capture casts add a roughly
  constant offset to R-normalised channel ratios in a local region:
  `measured_ratio ≈ true_ratio + haze_offset`, solved from an anchor of
  known colour on the SAME capture (footer text = Base, pip slabs =
  Bright). Implemented in `scripts/ui_measure.py` (`--anchor`/
  `--anchor-rgb`; model documented in its docstring); settled values
  live in the measurement bank
  (`crates/starbreaker-ui/tests/fixtures/ui_ir/reference_measurements_v1.json`).
- **The style cascade applies through ONE engine.** Plan P4 unified
  every entry-application pass onto `bb_style_engine::apply` (a
  `StyleSheet` per container, tagged with its `Tier`); the legacy
  per-entry-point wrappers and the identifier-prefix sniff are deleted.
  The authoritative pass list + order is `crates/starbreaker-ui/docs/ui-cascade-passes.md`
  (unchanged by the migration — verified byte-identical on all frozen
  targets).
- **A shared-tier `BackgroundColor` does NOT override a custom shape's
  authored colour** (2026-06-13, ledger item 38). A generic shared sheet
  (`Tier::Shared`, the `mfd_g_*` records) is not the styling authority for a
  `WidgetCustomShape`'s intrinsic fill: when the shape already authors an
  enabled `background.color`, a shared-tier `BackgroundColor` modifier is
  skipped (`bb_brand_apply::shared_background_override_suppressed`, threaded
  via `apply_sheet`'s `sheet.tier == Tier::Shared`). Brand / embedded / inline
  tiers still override — a brand CAN restyle the shape. Motivating case: the
  `GEN_MC_S_Emissions` header side bars author `background.color` = Accent2
  (Sep1) / Accent1 (Sep2–4) = red, which the shared `mfd_g_emissions`
  "New Style" entry was recolouring to Base (orange); the in-game bars are red.
  Brand-index note for that canvas: `brandStyles[1]` = `s_drak_hud` is the
  Clipper's DRAK brand and authors NO separator colour — the Accent1/visibility
  separator entries live in `s_argo_hud`/`s_grin_hud` (other brands, never
  selected for the Clipper), so the authored node colour is the only red source.
- **`rendererType:"None"` nodes paint NOTHING (2026-06-15, ledger item 63).** A
  `BuildingBlocks_DisplayWidget` with `rendererType:"None"` is a non-rendering
  proxy/group: the engine draws no background/svg/fill for it even when
  `background.enable=true` with a colour. Only nodes with a primitive renderer
  (`rendererType:"Flash"` — solid-fill rects, text fields, images) draw their
  authored background. `node_background_enabled` (ui_ir `engine_01.part`) returns
  false for an EXPLICIT `"None"`; an absent rendererType keeps the normal rules.
  Motivating case: the compass `CanvasProxyRoot` (None) authored an enabled white
  background that painted an opaque sheet over the dark `DRAK_Background_compass`
  vignette until this gate landed (`a47759308`).

## Reference-capture measurement methodology

- Anchor the capture scale on **frame footer arrows** (x) and measure x
  and y mappings INDEPENDENTLY — do not assume the content maps
  isotropically or full-bleed.
- Prefer near-isotropic captures (check width/height against the
  physical 4:3 screen). The Clipper power capture (1468×1109) is
  trustworthy; the right-upper target capture (1959×1513) is ~4%
  y-ambiguous — use within-content ratios there.
- Derive content mappings from at least two authored-rect features
  (e.g. card box tops), then cross-check the second capture before
  concluding.
- Skewed captures are rectifiable: store the four screen-corner pixels
  as `<reference>.corners.json` and `ui_compare.py` warps the capture
  onto the render rectangle automatically (plan P1.3); prefer
  `scripts/ui_measure.py` over ad-hoc pixel maths and consult the
  measurement bank FIRST (workflow §4).

## Troubleshooting Flow

1. Confirm source provenance first
- Check selected style/SWF source and unresolved references in diagnostics output.

2. Reproduce with deterministic fixture path
- Use representative fixture canvases under `crates/starbreaker-ui/tests/fixtures/canvas/`.

3. Compare structural snapshots
- Run the certification dashboard and read its stdout table:
  - `cargo run -p starbreaker-ui --example phase5_certification_dashboard`
- For per-target structural drift, run the live IR guard / snapshot suite
  (`cargo test -p starbreaker-ui --test manifest_live_ir_guard` /
  `--test manifest_snapshot_regression`).

4. Classify fault domain
- Source resolution mismatch: investigate `bb_resolve`/bindings/style application.
- IR mismatch: investigate `ui_ir` compilation fields.
- Rendering mismatch: investigate `ir_compose` use of existing IR fields.

5. Add a regression test with the fix
- Add/update a focused unit/integration test in the touched subsystem.
- Re-run `cargo test -p starbreaker-ui`.

## Incident Checklist

- Is this a source-data issue or renderer misuse?
- Is the behavior represented in IR (`UiIrDocument`) correctly?
- Did certification snapshots drift for previously certified screens?
- Is there an undocumented fallback involved?
- Did the fix add a focused regression test?

## Open architecture debt (deferred; evidence recorded)

Cross-arc structural work proven necessary but deliberately not landed —
each is recorded with its evidence so the next attempt starts warm. (These
were items 16/17/18 of the retrospective ledger
`crates/starbreaker-ui/docs/ui-process-improvements.md`; they live here now
because they are architecture, not history. Screen-level parity residuals
that depend on these are in `crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md`.)

- **Per-host-type expansion ID-band lanes.** Adding a new expanding host
  type shifts the shared `EXPANSION_ID_BASE` allocation order in
  `merge_child_scene` and can STEAL a frozen platinum identity (adding
  `WidgetSeparator` turned the medical close-button X into a separator
  instance — the guards caught it pre-land). The design couples baseline
  identity to expansion ORDER. Fix: a band lane per host type (or a second
  band, e.g. `0xF800_0000`, for new types) in `merge_child_scene`. Blocks the
  parked separator-dots work (handoff "Open items").
- **One brand-context resolver (RESOLVED — B1, 2026-07-07).** Formerly four
  independent brand-container selection paths existed — `resolve_brand_style`'s
  manufacturer-prefix scan, `collect_standard_text_styles`' `selected_style_name`
  family mapping, the body-background preferred chain, and the separator
  `hud`↔`env` sibling swap — and the separator AEGS-divider leak came from one of
  them improvising over a shared standard. Now unified on ONE identity resolver
  in `bb_brand_style` (`resolve_brand_identity` + `brand_class_for_canvas` +
  `BrandPolicy`): canvas style-link → `s_<mfr>_{hud|env}` by canvas family →
  sibling swap; identity matching only, no prefix scans over shared standards.
  Every call site was migrated one per commit under the guards and the legacy
  `resolve_brand_style` prefix scan is deleted.
- **Renderer-wide linear-light compositing (LANDED — B4, 2026-07-07).** The
  engine composites in LINEAR light; the renderer previously blended in sRGB,
  so antialiased edges and translucent overlays rendered too dark. The whole
  compositor was migrated to blend in LINEAR (Strategy A: per-site composite
  into a transparent scratch, then premultiplied linear source-over into the
  u8-premultiplied-sRGB `Pixmap`/`RgbaImage` — storage stays sRGB u8, only the
  blend math moved). Coverage is renderer-wide with NO carve-outs: fills,
  strokes (via `ir_compose::fill_primitives` `fill_linear`/`stroke_linear`),
  tinted blits (linear texel loop), the two manual clip composites
  (`ir_compose::clip_composite`), and ttf+swf text antialiasing — all through
  `crate::colour`'s `blend_premul_linear`/`blend_straight_linear`/
  `blend_premul_add_linear`. The separate `swf_render` overlay compositor's two
  composite functions (`composite_rgba_over_pixmap`/`composite_pixmap_over_rgba`)
  were migrated too, so no sRGB island remains in the binary. The only
  intentionally-sRGB draw is the opaque
  full-pixmap background init (no blend). The earlier scoped
  `blit_white_mask_overlay_linear` glow carve-out was folded into the now-linear
  general blit and deleted (its white-texel result reproduced within ±2). All
  15 gold/platinum targets re-frozen; the geometry-IR snapshot stayed
  byte-identical (colour-only). The migration moved every target TOWARD its
  reference (brighter, none darker; the annunciator's reference-verified
  chiclet fills preserved within ±2, confirming the general blit reproduces the
  carve-out that was itself verified against reference (71,48,15)).
