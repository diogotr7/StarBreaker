# UI parity workflow (authoritative)

## Purpose (the WHY — read before the how)

Produce near-pixel-perfect replicas of the in-game UI as **static images**, so
the exported Blender files look as close to the in-game environments as
possible. The renders are baked as textures onto screen meshes during
`entity export` — **this crate is a texture source for the Blender export, not
a UI runtime.** Interactivity and animation matter only insofar as they
determine a screen's correct static visual state; runtime behaviour is never an
end in itself. Everything below — the engine-faithful, generic *means* — serves
that end. When trading off effort, how a screen *looks in the exported scene*
outranks how it would behave live. Owner direction (2026-07): scale parity arcs
across many ships once the process is proven; a public release eventually; the
residual (acceptable-gap) bar being codified; fully-auto arcs on known ships.

THE process for UI-matching work in `starbreaker-ui`: getting a rendered
screen to match its in-game reference, engine-faithfully and generically.
Commands, tools, data locations, and the per-screen dossier live in the
companion **`crates/starbreaker-ui/docs/ui-reference.md`** — this doc is the *how to work*, that
doc is the *what to type*.

Starting fresh? Read this doc, then `crates/starbreaker-ui/docs/ui-reference.md`, then the current
arc's handoff doc (see the dossier's "open issues" column). The short
per-screen prompt template is
`crates/starbreaker-ui/docs/ui-matching-agent-prompt.md`.

## Read WHEN (index)

Read **§1 ALWAYS** (before any fix — non-negotiable, a full read). Then jump by need:

- **§2** — locating which stage owns a wrong thing
- **§3–§4** — during the working loop (per-item fix + workstream-boundary review)
- **§5** — a guard tripped
- **§6–§7** — before ANY freeze / known-outlier
- **§8** — pinning or deriving a value
- **§9** — at a pause / writing a handoff
- **§10** — before retrying anything weird (the don't-retry list)

## 1. Non-negotiable rules

1. **Engine-faithful and generic.** No hard-coding, no name-matching, no
   ship-/screen-/manufacturer-specific branches in production code. No
   heuristic blend factors, hand-tuned percentages, or magic offsets unless
   derived from source data. Hard-coded game-data VALUES (palette
   `RgbaColor` literals, font sizes, brand font lists — fallbacks included)
   are hard-coding: derive from DataCore/P4K, or use a provenance-noted
   extracted fixture for offline tests (registry pattern; guard:
   `tests/source_hardcoding_guards.rs` + `scripts/check_ui_hardcoding.sh`). The rule
   is SELF-CORRECTING: encountering pre-existing hard-coding obliges you to
   replace it or flag it in the same change — never extend it because it is
   already there. If one asset misbehaves, find the structural property of
   its category and fix the rule for the whole category. Ask: *how does the
   engine handle this?* — mirror its model.
2. **TDD.** When a bug is found, write a failing test that reproduces it
   BEFORE changing code; verify it fails; fix; verify it passes.
3. **Frozen platinum/gold = regressions in source behaviour.** Never silence
   them by editing tests or baselines first. Baselines change only through
   the audited freeze flow (§7) with explicit approver + reason, or via a
   reference-anchored known-outlier (§6).
4. **IR is the sole styling authority.** The renderer consumes explicit IR
   fields; it never invents semantics from style tags, widget names, parent
   context, or palettes at draw time. If an effect isn't in IR yet, add it
   to `ui_ir` preprocessing/schema first, then consume it. Compose-time
   effects must be represented in IR/snapshot semantics so guards can see
   drift.
5. **3000-line cap** on every `.rs` under `src/` (enforced by
   `line_count_guard`). Split by responsibility well before the cap —
   target chunks of ~2500 lines or less; the cap is the hard stop, not
   the goal.
6. **Remove no-effect experiments immediately.** A patch that doesn't
   measurably change queried IR/draw values or rendered output is a failed
   hypothesis: revert it and record what was falsified before trying the
   next idea. Don't stack overlapping fallbacks — one structural rule per
   behaviour.
7. **Verify-on-write documentation.** Every command line that enters a doc
   is run once at writing time. Renaming/removing a CLI subcommand, script,
   probe, or doc file includes a repo-wide grep for references in the same
   commit.
8. **"Overrides" means freeze-system known-outliers only** — never
   hard-coded value overrides in code.

## 2. The architecture in one page

Pipeline stages and the bug classes they own (deep dive:
`crates/starbreaker-ui/docs/ui-architecture-runbook.md`):

| Stage | Files | Owns |
|---|---|---|
| Source resolution | `bb_resolve/` (Pass-1 canvas-ref entries, Pass-2 WidgetCanvas urls, widget-standard expansion, list/array materialisation, namespaces), `bb_state_filter.rs`, `bb_style_engine` (the single style-cascade application engine, P4) + `bb_brand_apply/` (its condition/modifier kernel), `bb_scene/` (parse, WidgetClone expansion) | which nodes exist, their authored fields, applied styles |
| Bindings | `bb_bindings/` (op-graph eval vs `DefaultValueRegistry`, bound geometry/state-tags/text) | values: text, numbers, booleans, bound SizeX/Y + AnchorX/Y, state tags |
| Layout | `bb_layout/` | rects: flex (order, grow/no-grow, shrink, intrinsic text), overlay anchors/pivots |
| IR compile | `ui_ir/` | the canonical `UiIrDocument`: which authored metadata survives, effective font sizes, clip rects, payload classification |
| Render | `ir_compose/` (+ `text/`, `bb_svg`) | final draw: glyphs, fills, tints, contain-fit, clipping |
| Regression | `ui_snapshot/`, manifest tests, freeze fixtures | structural + whole-image comparison vs frozen baselines |

Style cascade order (lowest first): canvas `style` record link < `sharedStyles`
< selected `brandStyles[]` < `embeddedStyles` < node `inlineStyles` (applied
last in every pass; an inline `FontSize` is marked `__InlineFontSize` and
outranks the brand-table standard in font resolution). Each tier is a
`bb_style_engine::StyleSheet` with a `Tier`; the full pass list (incl.
widget-standard module sheets, deferred late-state subtree passes, and the
text-format route gated on `Tier::Brand`) is `crates/starbreaker-ui/docs/ui-cascade-passes.md`.

Per-ship values: derived at export in
`starbreaker-3d/src/ui_pipeline/ship_values.rs` → `PipelineInputs::derived_values`;
the CLI replay derives from the scene's root entity so **replay == export**.
At-rest engine-pushed values that can't be derived yet are pinned in
`crates/starbreaker-ui/data/default_value_registry_v1.json` — every pin gets a
provenance entry in `default_value_registry_v1.notes.md`.

## 3. The working loop (per catalog item)

1. **Identify the owning stage before editing.** For style/tag questions run
   the MCP trio in order: `ui_canvas_style_inventory` (which authored
   container/entry) → `ui_scene_style_probe` (which entries matched the
   node) → `ui_ir_query` (what the renderer will consume). For geometry,
   `ui render --dump-ir-dir` + `examples/ui_stage_diff` (parse vs resolve)
   + the probes (reference doc §6). Read the authored canvas in the
   dcb_canvas mirror — it is grep-able JSON.
2. **One falsifiable hypothesis, one focused change**, with the failing test
   first (rule 2). Don't chain speculative layout changes between
   measurements.
3. **Validate:** `bash scripts/ui_check.sh` (every cycle), then re-render
   via the ~1-minute replay (`bash scripts/ui_render.sh --helper <name>` —
   it rebuilds first and prints the binary mtime, so a stale binary can't
   masquerade as "the fix had no effect") and compare with
   `scripts/ui_compare.py`. Run `ui_check.sh` UNPIPED and read its
   `ui_check: ALL GREEN` line — never a piped exit code (ledger 89). Each run
   writes a marker file (`.git/ui-check-marker`); a pre-commit gate then refuses
   any commit touching `crates/starbreaker-ui/`, `scripts/`, or `mcp/` unless the
   last marker is `ALL GREEN` and fresh (`SB_UI_MARKER_MAX_AGE`, default 1800s;
   `SB_SKIP_UI_GATE=1` bypasses for rebase/emergency). Install it once with
   `bash scripts/install_ui_precommit.sh`.
4. **Update the catalog** (fixed / still open / new finding) and the arc's
   memory/handoff notes for any non-trivial diagnosis — at discovery time,
   not at session end.
5. **Commit** per coherent fix; message cites the catalog item. Before each
   commit, the arc's memory/handoff state is current.
6. At a workstream boundary: `bash scripts/ui_check.sh --full`, re-export,
   full reference comparison, handoff/memory update.

**Waiting on a long-running command — its marker, never a sleep-loop.**
`ui_check.sh --full`, a full `entity export`, and `ui_arc_status.sh` / `ui_render.sh`
run for tens of seconds to minutes. Launch them via the harness background facility
and detect completion by the command's exact terminal line in its log —
`ui_check: ALL GREEN`, `ui_arc_status: OK (…)`, or `ui_render.sh`'s closing
`latest -> …` after its `png md5:` print. Never a `sleep` loop, never a turn-holding
foreground wait, and never a regex looser than the exact marker string (a loose match
fires on a progress line and reads an unfinished run as done; ledger 89).

User gives relative feedback ("move it ~20px up")? Treat it as a calibration
target: measure current values first, trace the mismatch to authored
metadata / layout math / IR loss / draw-time adjustment, fix structurally,
re-measure numerically, only then ask for visual confirmation.

## 4. The review phase (mandatory at every workstream boundary)

1. Re-render the screen (replay; full export only when refreshing canonical
   PNGs).
2. `python3 scripts/ui_compare.py <render> <reference> --regions <preset>`
   and READ each crop with vision. For any colour question add `--stats`
   (bright/dark means + R-normalised ratios per region): judge hue from
   ratios, calibrate the capture's cast from a known anchor on the SAME
   reference (footer text = Base, pip slabs = Bright) — method details in
   the reference doc §3. Pixel adjudications consult the **measurement
   bank FIRST** (`crates/starbreaker-ui/tests/fixtures/ui_ir/
   reference_measurements_v1.json` + its `.notes.md`): settled reference
   numbers are lookups, not re-measurements; new measurements use
   `scripts/ui_measure.py` and get appended to the bank with provenance.
3. Build/extend the numbered **diff catalog**: region | difference |
   severity | root-cause hypothesis | fix-or-defer decision. Deferrals are
   explicit entries, not omissions. Reference screenshots are imperfect
   (skew, bloom, resolution, capture artifacts — e.g. the power screen's
   red/white pip outlines are mouse-hover artifacts): compare structurally,
   not pixel-naively.
4. Order the fixes (structural/layout before styling; shared-root-cause
   items together).
5. Execute via §3; re-run the review until parity or only documented
   deferrals remain.

## 5. When a guard trips

The platinum/gold guards failing after your change is the system working.
Method (never name-gate your way out):

1. Read the failure: it names the target and `<node_id>:<node_type>` with
   field deltas.
2. Find that element in the freeze JSON
   (`crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json`)
   and its authored source in the dcb_canvas mirror.
3. Find the **structural discriminator** separating the frozen
   counterexample from your motivating case (sizing behaviour class, anchor
   range, absolute-vs-relative path, value range — never a name).
4. Scope the rule by that property and cite the counterexample in a code
   comment.
5. If instead the **baseline is wrong** (your change moves the render toward
   the in-game reference): verify visually against the reference, then
   re-freeze (§7) quoting the per-identity delta in the reason and commit
   message.
6. If the deviation is known but the true fix is deferred: register a
   known-outlier (§6) instead of freezing the miss.

**The empirical disable→adjudicate audit** (ledger item 22): to test whether
a suspect rule (hard-coded constant, name match, special pass) is
load-bearing, disable it and let the frozen pins adjudicate — one session
proved eight rules deletable and three load-bearing this way, far cheaper
than per-rule reference archaeology. Preconditions, both mandatory:
(1) **fresh export first** — the whole-image guard compares
`ships/Data/UI/Generated/*.png`, which only refresh on export; the P0.2
staleness guard (`.export_stamp.json`) now hard-fails stale comparisons,
but don't lean on it: re-export (~50s) before judging "zero drift";
(2) consult ALL THREE suites — lib tests, the live-IR guard, AND the
visual/snapshot suites — a rule can be invisible to two and pinned by the
third. Scope caveat: a clean pass proves "**no frozen pin references
this**" (five screens, one ship today), NOT "correct everywhere" — record
the deletion in the fallback register's retired table with that bound.

## 6. Known-outlier overrides (reference-anchored)

For elements knowingly off-reference where the true fix is deferred:
register in `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_known_outliers.json`
(`{target, identity, field, frozen_value, reference_target, confidence,
reason, source}`; numbers only — no reference image enters the repo). The
comparator treats the field one-sided: movement toward `reference_target`
passes with a `✅ IMPROVEMENT` note (**re-freeze, don't revert**); away
fails; within `confidence` → graduate to a strict freeze. Field-generic
(geometry, alpha, font_size, text tops, tint rgba/token). Gotcha: freeze
pipeline font metrics differ from full renders — anchor text-top targets as
`frozen_snapshot_value − measured_render_delta`. Full policy:
`crates/starbreaker-ui/docs/ui-regression-policy.md`.

## 7. Freezes, target onboarding, tier changes

Baselines are data-driven from
`crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json`.
Flows (commands in the reference doc §2/§7):

- **Onboard a target** (only after the output is explicitly approved as a
  standard): `add_ui_regression_target.sh` → `freeze_ui_snapshot_ir.sh
  --approver <name> --reason "..."` → re-export →
  `generate_ui_regression_artifacts.sh` → `freeze_ui_regression_artifacts.sh`
  → both validators → `ui_check.sh --full`.
- **The artifact cycle as one command**: `bash scripts/ui_freeze_cycle.sh
  --approver <name> --reason "..."` runs release build → export → stale
  `*-current.png` cleanup → artifact freeze → both validators →
  `ui_check.sh --full`. The IR snapshot freeze stays a separate, deliberate
  step: its printed delta must be read and accounted for.
- **Re-freeze after an intentional improvement**: same freeze command; the
  tool prints the per-identity delta — the reason and the commit message
  must account for every changed identity. AUDIT RULE: a re-freeze whose
  delta contains identities you can't explain is rejected.
- **Artifact freezes** source the EXPORTED PNGs: re-export first; the export
  writes those PNGs near the END of its run — never diff or freeze until
  the process has exited.
- **Tier changes**: never to silence drift; re-freeze with reason, run both
  validators + the full battery.
- No image binaries in freeze commits (`test-artifacts/ui/*.png` stays
  untracked).
- Every UI defect fix lands with a regression guard in the same change
  (category guidance: `crates/starbreaker-ui/docs/ui-regression-policy.md`). Pixel regression is
  the content-agnostic whole-image guard — do NOT add per-screen ROI pixel
  checks.

## 8. Derivation policy

- Ship-specific values (pips, temps, battery counts, signatures…) derive
  from DataCore in `ship_values.rs`, with unit tests asserting the
  reference-verified numbers.
- Engine-runtime values with no decoded formula yet are pinned
  (registry or derivation constants) with a TODO + provenance note —
  documented IOUs, not silent magic.
- New fallback logic of any kind requires an entry in
  `crates/starbreaker-ui/docs/ui-fallback-register.md` (trigger signal + retirement target).

## 9. Memory, handoffs, prompts

- Non-trivial diagnoses go into the arc's memory file **when made**.
- Every commit is preceded by a memory/handoff status update.
- An arc that pauses (or at any major milestone) maintains a repo handoff
  doc — pattern `docs/ui-<arc>-handoff.md` — with: landed commits, catalog
  status, next-step spec (ideally an `#[ignore]`d failing test), parked
  work with full diagnosis, approval-gated items. **Delete the handoff when
  the work lands** (grep its name repo-wide + the docs_reference_guard list
  in the same commit).
- **A handoff that claims binding-kind / canvas / aspect facts MUST verify
  them against the actual exported `scene.json` `ui_bindings`, not just
  DataCore records.** The step-3 hand-off was researched from records and
  asserted the square screens were `mfd`/4:3 reusing the aspect-tag path;
  the export showed them `physical` on `M_Physical_Screen` (16:9) with no
  mfd path, so the whole "reuse it" plan was wrong and cost a spike to
  overturn (ledger 48). The export is the ground truth — check it first.
- To launch fresh-context work on a screen, instantiate
  `crates/starbreaker-ui/docs/ui-matching-agent-prompt.md` with the screen
  name — the dossier (reference doc §3) supplies everything else.
- At the END of an arc — in the SAME session, before clearing context —
  paste the process retrospective prompt
  (`crates/starbreaker-ui/docs/ui-process-retro-prompt.md`): it runs on the
  session's lived experience (the friction only that session knows);
  findings append to the `crates/starbreaker-ui/docs/ui-process-improvements.md` ledger and get
  implemented, so each arc lowers the next one's bootstrap cost.

## 10. Known pain points & don't-retry list

- **State-bound visibility**: live-session elements are IsActive-gated
  chains false at static rest; visibility gating lives in
  `bb_state_filter`'s capture-tested heuristics. A generic IsActive-binding
  pass over the merged scene breaks medical platinum — don't retry.
- **Relative `urlPostfix` composition**: medical canvases author bindings
  pre-qualified and the platinum registry keys pin that resolution; only
  ABSOLUTE (leading-slash) postfixes compose. Composing relative ones needs
  a registry key migration first.
- **Overlay-default tint** ("enableColorOverlay + null colour → Base" for
  shapes/images): regressed target-screen chevrons. Entry-driven FillColor
  is the engine model. (The narrow WidgetIcon case is implemented and
  scoped.)
- **Column-wide intrinsic text sizing**: medical baselines pin the
  fill/auto_main placement for non-zero Auto hints in columns; only
  Auto == 0.0 (pure content hint) is in scope for column intrinsics.
- **Font reads TOO SMALL/BIG? read the INSTANTIATED variant's authored FontSize
  FIRST — before theorising about a per-screen scale model (ledger 60).** The
  velocity-num readouts rendered ~9× small and were deferred a whole arc as "no
  data-derived scale reaches 6× — per-screen text-scale model undecoded, blocked by
  the annunciator counterexample." That was UNDER-RESEARCH: never a scale problem.
  The DRAK velocity SCREEN variant authors FontSize **500/420** directly; they
  weren't being APPLIED (the no-brand-match `defaultStyles` fallback + the
  text-format `Type(Text)` gap). `SB_UI_FONT_DUMP` shows the EFFECTIVE size (52 =
  the Heading2 fallback), `BB_A3_STYLE_PROBE`/`BB_TEXT_FORMAT_PROBE` show which
  entries matched (none here), and the authored size is grep-able in the variant
  record (find the instantiated variant via the rendered node names — the master
  selects it by tag). A "no derivable scale" conclusion is valid ONLY after
  confirming the authored size is reaching the node.
- **Text-size calibration constants are RETIRED — do not reintroduce.**
  The 0.98 all-caps reduction (2026-06), then the `1.5` TTF draw/measure
  pair, the `0.84` SWF calibration, the `0.33` inline word-gap and the
  `-8.0` caption overlap (all P3, 2026-06-13) were removed for derived
  models: IR font size = design-em px (factor 1.0), inline pairs continue
  at glyph-advance, caption pairs stack by the line box. The data-backed
  font model owns sizing — a tuned scalar is a regression
  (`crates/starbreaker-ui/docs/ui-font-size-harness.md` guards it; run via `ui_check.sh --full`
  whenever text size could change). Engine-model detail:
  `crates/starbreaker-ui/docs/ui-architecture-runbook.md` §"engine models settled".
- **`.tif` in canvas JSON = `.dds` in P4K.**
- **Stale generated PNGs**: `ships/Data/UI/Generated/...` only refreshes on
  full export. Iterate via replay; export before freezing artifacts.
- **Thin-feature COLOUR on the rectified reference is wrong (ledger 35).** The
  homography warp (`ui_compare`/`ui_measure` against a `corners.json`-rectified
  capture) interpolates a ≤~4px feature — header bars, strokes, dotted
  separators — with its background, diluting the hue: a 2px Accent1 header bar
  measured G/R 0.64 ≈ the same-capture Base anchor, so the orange render was
  recorded "faithful" until the owner overturned it (the bars author Accent1/
  Accent2; the crisp original is unambiguously red). Rectify for POSITION;
  judge a thin feature's COLOUR on the CRISP ORIGINAL. `ui_measure.py` warns
  when `feature_width ≤ 4`.
- **Use the INDEXED tools for IR inspection; don't mistake slowness for a loop
  (ledger 42).** Fast paths: `ui_ir_query` (MCP — ad-hoc canvas pair → rects +
  tags + tokens, ~1s) and `starbreaker ui render --helper <name> --dump-ir-dir
  <dir>` (bound screen → `*.ir.json` with `computed_rect`, ~15s, *matches the
  actual render*). `mfd_ir_dump` (the no-P4K/no-MCP fallback) is now ~5s too
  (indexes only the UI subtrees + caches the parsed `TagDatabase`; it prints
  index/compile timing). It WAS ~94s when it parsed the whole 60k-file mirror —
  if you ever see sustained CPU/RSS, time the run to completion, don't read it as
  a layout loop and start reverting. `PercentOfY` content width does NOT cycle.
- **`ui_ir_query` DOES exercise the aspect-tag content-scaling (ledger 43,
  reversed).** The MCP canvas fetcher now indexes `BuildingBlocks_AspectRatioLibrary`,
  so `AspectRatioToTag_MFD` resolves and `apply_mfd_content_canvas_scaling` fires
  — `ui_ir_query` returns the scaled layout (power cards 438px, matching the
  export). It is the fastest way to verify MFD content-scaling / aspect changes.
  (Requires the redeployed MCP binary; restart the client after an MCP rebuild.)
- **Measure element width from the DRAW RECT, not dim pixels (ledger 45).**
  At-rest cards render at low alpha (the battery card is 0.2 "depleted"), so
  pixel-scanning the export PNG catches only a glyph's dense core (and the glyph
  moves when layout reflows). Read the laid-out rect from `ui_ir_query` /
  `--dump-ir-dir` (`computed_rect`), or `BB_DRAW_RECT_PROBE=<name|asset|1>` on a
  `ui render` for the actual raster size in the custom-shape draw path.
- **Cockpit UI screen geometry needs `--lod 0`; LOD1 culls the small HUD
  screens (ledger 47).** Plain `entity export <ship>` defaults to LOD1, which
  drops g-force/velocity/countermeasures/etc. — anything keyed off their screen
  mesh (the per-screen render aspect) silently resolves to `None`/16:9, and the
  no-`--lod` export writes the `LOD1_TEX2` package so the `LOD0_TEX0` scene.json
  is left stale. Don't conclude "the geometry is unreachable" from a LOD1
  export: re-export `--lod 0`, use `SB_SCREEN_ASPECT_PROBE=1` to confirm, and
  read the *package's own* scene.json + `<helper>_TEX0.png`. Full data model:
  reference §5 *screen-mesh → render aspect*.
- **`cover_fit_recentre` / post-layout rect shifts are INVISIBLE to the TDD-tier
  guards — re-run `--full` (ledger 52).** The IR-snapshot freeze pipeline does not
  apply `cover_fit`, so a change to `cover_fit_recentre` (or any `bb_layout`
  post-layout translate) does NOT move the snapshot and `ui_check` stays green
  while the EXPORT render shifts. It also reaches every `cover_fit=true` screen —
  including the aspectOverrides door/annunciator (their `canvas==target`, so the
  shift must be gated to axes where the canvas OVERFLOWS the target, else their
  content moves). After touching it, re-export + `ui_check --full` (the whole-image
  guard is the only one that sees it).
- **A `--full` visual-guard failure on a screen your change can't reach = a STALE
  LOCAL artifact, not a regression (ledger 53).** `test-artifacts/ui/*.png` are
  untracked and refresh only on freeze, so they drift from the current render
  (e.g. a prior text-top change). If the failing screen's IR snapshot is unchanged
  AND it has no code path your change touched (no full-circle node, no cover-fit,
  `BB_SHRINK_PROBE` shows no relevant shrink), it's a stale artifact — re-freeze to
  sync (owner-gated, §7), don't chase a phantom root cause.
- **Do NOT broaden `ir_compose` `center_anchored_heading` to centre text (ledger
  59).** That rule centres a label anchored `anchorToParent` 0.5/0.5 inside a
  `0,0`-anchored field, gated to `label_style=="Heading1"`. Dropping the Heading1
  gate to centre another readout (e.g. the velocity-num HUD, Heading2) REGRESSES the
  medical-bed (`ui_target_a`) titles/descriptions — they share the identical anchor
  pattern but render LEFT (owner-caught, 1.5%). Centring is NOT purely anchor-driven;
  the Heading1 gate is load-bearing. Centre stacked readouts via `bb_layout` CARD
  cross-axis content-fit + `crossAxisJustification=Center` (scoped to centred
  columns, which no frozen screen has), NOT the text-draw rule.
- **White / wrong colour where a BACKGROUND should be = an OVER-PAINTER, not a
  missing asset (ledger 63).** The compass rendered a white sheet over its dark
  vignette; the texture was drawing fine (full-screen raster, correct `.dds`) —
  an authored `CanvasProxyRoot` with `rendererType:"None"` + `background.enable`
  white was painting OVER it. A node with `rendererType:"None"` is a non-rendering
  proxy/group (engine paints nothing for it; only `"Flash"` nodes draw their
  background — `node_background_enabled` enforces this). Before theorising about
  texture load/fit, dump EVERY node's fill: `python3 scripts/ui_ir_query.py query
  <ir.json> '.*' --fields background_fill_colour,stroke_colour`. A fill that can't
  come from the suspect texture's value range (white over a ≤50-valued vignette)
  proves a later element overpaints it.
- **A namespace-less `arrayVariable` that is a MULTI-segment path is an engine-state
  reference, not a relative name (ledger 64).** `apply_array_variable_lists` skipped
  namespace-less relative `arrayVariable`s, so a top-level engine list
  (`FlightController/Compass/Ticks`) left its lone authored child — a per-entry CLONE
  template — rendering as a stray item. A path with `/` (`A/B/C`) is an absolute
  engine-state ref (the authored leading slash is inconsistent across the data);
  resolve it directly and pin its at-rest count in the registry (compass `=0` →
  template deactivates = faithful empty list). A BARE single-segment name (power outer
  `pipList`) is UI-local and still needs a namespace — the discriminator that keeps the
  power pip stacks byte-identical.
- **The whole-image guard MISSES few-px elements — after any asset/icon/binding change,
  eyeball ALL screens sharing the mechanism (ledger 77).** A binding-first `SvgPath` change
  let a chrome FillStyle TAG ref win over the preset asset, so the MFD footer pixel nav
  arrows (`‹`/`›` on power/target/self) vanished — but each arrow is only a few px, under the
  1% gold whole-image budget, so `ui_check --full` stayed GREEN; the owner caught it by eye.
  The pixel-diff guard is a coarse net (no few-px element-presence check). After touching
  `asset_ref` / `collect_node_asset_refs` / `resolve_field_text` / icon presets, render the
  OTHER screens that share the path and read them with vision — small chrome (nav arrows,
  separators, greebles) regresses silently. Specific rule: a bound `SvgPath`/`svgPath`
  overrides the authored/preset asset only when it resolves to a real PATH (`contains('/')`),
  never a tag ref.
- **A size/colour "frozen-family blocker" → re-read the INSTANTIATED variant's authored
  style entries and confirm they APPLY, before any render-side scaling/gating (ledger 76,
  reinforces 60).** master-mode's text size AND colour (declared proven frozen-family blockers
  in ledger 75) were ONE unapplied authored `defaultStyles` entry (bare `Type(Text)`, FontSize
  350 + white) — the fix was UPSTREAM (apply it), not the render-side autoFontSize/tag-gating
  that collided with frozen `auto` siblings. When a render-side scope keeps regressing the next
  frozen sibling (whack-a-mole), suspect the WRONG STAGE: parse the variant's `defaultStyles`/
  `brandStyles` JSON and verify via `ui_ir_query --fields text_style` / `BB_TEXT_FORMAT_PROBE`
  that the authored value reaches the node. Third inherited "blocker" overturned this way
  (velocity-num, compass ×2, master-mode).
- **A draw path can't produce an element natively (3D-RTT / Primitive / live render)? EXHAUST the
  in-data ASSET before inventing — the hologram precedent is NOT a licence to invent geometry
  (ledger 78).** The radar scope is a 3D-RTT `WidgetWindow`; the first attempt cited the SELF-STATUS
  hologram and INVENTED the disc (eyeballed ring/spoke counts + tilt — banned reference-calibrated
  magic numbers). The owner corrected it: the disc is a REAL texture
  (`r_radarmapscreen_radial_gradients.dds`, bound by the `Circle_Radial_Grid` Primitive node's
  `.mtl` TexSlot1). The distinction: the hologram rendered the REAL decoded MESH — only its CAMERA
  was owner-tuned. So for any unreproducible visual: `mtl_summary <primitiveMaterialPath>` → texture
  slots → `p4k_search` + `image_preview` the `.dds`; check `svgFill.svgPath`, styleTag
  `SvgPath`/`ImagePath` modifiers, SWF/Flash. Render the REAL asset; ONLY the engine runtime camera
  transform (absent at static rest) + an emissive-brightness match are legitimately owner-tuned.
- **Visual assets are PER-MANUFACTURER — read the cascade-applied material, not the authored generic
  (ledger 79).** A brand swaps a Primitive node's texture via a `PrimitiveMaterialPath` style
  modifier, which `bb_brand_apply::apply_string_field` writes to a TOP-LEVEL `node.raw["PrimitiveMaterialPath"]`
  (not the nested `primitiveSettings.primitiveMaterialPath`). Prefer the top-level value (the IR
  `primitive_material` field does) so Greycat/RSI/… get their variant and a no-override brand (DRAK)
  keeps the generic. An RTT-window 3D scene reuses the hologram fetcher pattern (`HologramFetcher`
  trait method default-`None` → `PipelineInputs`→`ComposeContext`→`ir_compose`, impl in
  `P4kHologramFetcher`); a new `UiIrNode` field for it should be `#[serde(skip)]` (IR-snapshot-safe)
  and expect ~12 literal sites to update (ledger 80).
- **"Font too small" can be a per-screen RENDER scale (PORTRAIT stretch), not an authored size NOR an
  "undecoded engine" wall — and bb_layout `canvas_scale` is INERT for the IR glyph render (ledger 97).**
  The LR-indicator labels stayed ~3× small after the authored FontSize was applied: it is the only
  PORTRAIT cockpit screen (a 1920×1080 landscape `useRaw` canvas filling a 0.64 mesh → geometry stretches
  ~2.78× but a Fixed font is drawn at design px). TWO traps: (1) the obvious lever, the `bb_layout`
  fill-branch `canvas_scale` (commented "the Fixed/text scale"), does NOT reach the live render —
  `ir_compose::draw_text_node` sizes glyphs from the IR `font_size` directly, so a one-line change there
  renders BYTE-IDENTICAL (the printed `png md5` is how you catch it). The lever is `ui_ir`. (2) Scaling only
  the IR draw font CLIPS — a content-fit text field is sized for the unscaled font and cuts the enlarged
  glyphs, so the scale must ALSO apply to the layout text measurement (`annotate_effective_font_px`) so the
  field grows; measure == draw. The fix (`ui_ir::portrait_font_screen_scale`, vertical fill axis for a
  fill-branch screen with `target_h>target_w`, 1.0 elsewhere) is self-scoping — only the portrait screen
  changes, every frozen baseline byte-identical, no re-freeze. Keep `--full` to confirm the no-op on the
  square-mesh gauges (same canvas, aspect-1.0 target).
