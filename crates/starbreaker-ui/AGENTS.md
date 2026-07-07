# starbreaker-ui crate instructions

Scope: everything under `StarBreaker/crates/starbreaker-ui/`.

Read this after the repo-level `StarBreaker/AGENTS.md` and before planning or editing work in this crate.

## Required first reads

Before planning or editing in this crate, read these in order:

1. `StarBreaker/AGENTS.md`
2. `StarBreaker/.github/copilot-instructions.md`
3. `StarBreaker/crates/starbreaker-ui/AGENTS.md` (this file)
4. `StarBreaker/crates/starbreaker-ui/docs/ui-workflow.md` (the process) and
   `StarBreaker/crates/starbreaker-ui/docs/ui-reference.md` (commands/tools/data + per-screen
   dossier) for any UI parity/matching task. Read each doc's **"Read WHEN" index**
   (top of file) up front, then read full sections when the index or the parity
   skill's current phase directs — EXCEPT ui-workflow §1 (non-negotiable rules),
   which is always a full read before any fix.

Do not rely on stale chat context for UI matching behavior. Re-read
`crates/starbreaker-ui/docs/ui-workflow.md` when switching screens, after long detours, or when
visual fixes stop converging. To launch a per-screen parity pass, use
`crates/starbreaker-ui/docs/ui-matching-agent-prompt.md`.

## Core rules

- No hard-coding, no name-matching, no ship-specific branches, and no screen-specific branches in production code.
- No heuristic placement rules, blend factors, hand-tuned percentages, or magic offsets unless the surrounding rule is already structurally defined by source data and the new math is derived from that data.
- **No hard-coded game-data VALUES either** — palette colours
  (`RgbaColor { r: .., g: .., b: .. }` literals), font sizes, brand font
  lists, layout constants. This includes "fallback" palettes: an invented
  colour is hard-coding even when labelled non-authoritative. Resolve from
  DataCore/P4K, or use a provenance-noted extracted fixture (registry
  pattern) where tests need real values offline. The
  `rgba_colour_literals_are_not_hardcoded` test
  (tests/source_hardcoding_guards.rs) enforces
  this crate-wide — including test fixtures: a real palette value copied
  into a test is still a copy; load it from the extracted fixture instead.
  Genuinely arbitrary test colours must be neutral (pure black/white) or
  carry a `// hardcoding-guard: synthetic` annotation.
- **Self-correction is mandatory.** If you find hard-coding that predates
  your change: replace it with the data-derived source, or flag it
  explicitly (task/handoff entry) in the same change. Never copy or extend
  an existing hard-coded pattern because precedent exists — precedent of a
  banned pattern is a defect, not a licence. Extend the guards
  (`scripts/check_ui_hardcoding.sh`, tests/source_hardcoding_guards.rs) when you
  discover a category they miss.
- Fix the structural cause. Do not patch symptoms in one renderer path if the real issue is authored metadata, IR compilation, layout, or text measurement.
- IR is the source of truth for render values. The renderer must not override IR-provided font size, position, alignment, scale, margin, padding, text colour, stroke colour, icon tint, or visibility based on widget names, parent context, or screen-specific checks.
- If rendered output is wrong, correct the owning upstream stage (`bb_layout.rs` / `ui_ir.rs` / source data resolution) so IR values are correct before rendering.
- When a rendered position looks wrong, identify which abstraction owns it before editing:
	- `bb_layout.rs` owns authored layout rects.
	- `ui_ir.rs` owns which authored metadata is preserved into IR.
	- `ir_compose.rs` owns final draw-time rects and renderer-specific adjustments.
	- `text.rs` owns text metrics, baseline, and rendered glyph bounds.

## Known deviations: register a reference-anchored outlier, do NOT hack

When an element is *knowingly* a few px / one colour-role off the in-game
reference and the true fix is deferred (e.g. a deep, higher-risk layout change),
**do not** bake a compensating blend factor, magic offset, or `clamp`/fudge to
hit the value (that violates Core rules). Equally, **do not** strict-freeze the
wrong value — that enshrines the miss and later flags the genuine fix as a
regression.

Instead register the element as a **known-outlier override** in
`crates/starbreaker-ui/tests/fixtures/ui_ir/ui_known_outliers.json`
(`{ target, identity, field, frozen_value, reference_target, confidence, reason,
source }`; numbers only — no reference image enters the repo). The structural
comparator (`compare_snapshots_with_overrides`, `ui_snapshot/compare.inc`) then
treats that field one-sided, anchored on the measured reference:

- moving **toward** `reference_target` → passes + a `✅ IMPROVEMENT` note. **This
  means re-freeze, NOT revert** — the genuine fix is landing.
- moving **away** → fails as a regression.
- within `confidence` of the target → graduate: drop the entry, strict-freeze.

Field-generic: any captured snapshot field works (geometry, `alpha`, `font_size`,
`primary_text_top`/`secondary_text_top`, tint `*_rgba`, tint tokens). The snapshot
(schema v2) records the rendered glyph cap-top/left, so overrides anchor on the
*visible* text position. **Gotcha:** the freeze pipeline's font metrics differ
from a full `ui render`, so absolute text-top values differ between them; the
container position and any layout-fix delta are font-independent, so a px delta
measured on the full render still applies in snapshot space (set
`reference_target = frozen_snapshot_value − measured_render_delta`). Full policy:
`crates/starbreaker-ui/docs/ui-regression-policy.md` § *Known-Outlier Overrides*.

## Required validation loop for visual work

For visual/layout tasks in this crate, work in this order:

1. Identify the owning rect or draw path with the query tools before editing.
2. Form one falsifiable local hypothesis about the bad position.
3. Make one focused change.
4. Run the narrowest relevant validation immediately.
5. Measure the new result numerically.
6. Only then ask the user for final visual confirmation.

Do not chain multiple speculative layout changes before remeasuring.

## Query and debug tools

### MCP Tools (preferred for UI diagnostics)

The StarBreaker MCP server provides three diagnostic tools that query **live DataCore
records** and **P4K archive** directly — no local JSON files or decomposed exports needed.
These replace the old file-system-based fetchers (`LocalUiRecordIndex`, `LocalUiStyleFetcher`).

| Tool | Purpose | Key return fields |
|------|---------|-------------------|
| `ui_ir_query` | Compile canvas to canonical IR | `computed_rect`, `draw_rect`, `style_tag_uuids`, `resolved_style_tags`, `text_payload`, `asset_ref`, `background_fill_colour`, `stroke_colour` |
| `ui_canvas_style_inventory` | List authored style containers | `containers[]` (embeddedStyles, defaultStyles, brandStyles), `entries[]` with conditions/modifiers |
| `ui_scene_style_probe` | Match scene nodes to styles | `nodes[]` with `style_tag_uuids`, `colour_fields`, `applied_style_entries[]` |
| `p4k_data_status` | Confirm P4K/DataCore loaded | `p4k_path`, `entries` count, `datacore_bytes` |

**Data source architecture:**
- `P4kCanvasFetcher` → DataCore `BuildingBlocks_Canvas` records (replaces `LocalUiRecordIndex`)
- `P4kStyleFetcher` → DataCore `BuildingBlocks_Style` records (replaces `LocalUiStyleFetcher`)
- `P4kSwfFetcher` → P4K read for SWF files (replaces `McpNullSwfFetcher`)
- `P4kAssetFetcher` → P4K read for DDS/SVG/PNG textures (replaces `McpNullAssetFetcher`)

Canvas records are queried by GUID or name substring. Brand styles are resolved from
`BuildingBlocks_Style` records via manufacturer identifier (e.g. "drak", "banu", "aegis").
The canvas JSON references `.tif` extensions but the actual files in P4K are `.dds`, whever you see `.tif` expect a `.dds` entry in the P4K.

### CLI query tool (fallback)

Prefer the generic query example over ad-hoc logging:

```bash
cargo run -p starbreaker-ui --example query_ui_layout -- \
	--canvas-guid <guid> --query <pattern>
```

The query tool is intended to be generic. Keep it generic when extending it.

Current debug outputs include:

- node `x/y/w/h` layout rect
- resolved `draw_rect`
- `parent_id`
- primary/secondary text rects
- primary/secondary text origins
- primary/secondary drawn glyph bounds
- progress-meter draw rect
- asset reference path when present
- custom-shape metadata when present

If a future investigation needs another measurable draw-time output, add it here generically rather than adding screen-specific debug code.

## Troubleshooting workflow for relative visual feedback

When the user gives relative movement feedback such as “move it up about 20px” or “needs slightly more gap”, use that as a calibration target for investigation, not as the final rule.

The workflow is:

1. Use `query_ui_layout` to measure the current layout rect, draw rect, text rects, and drawn bounds.
2. Compare the measured values with the user’s relative estimate.
3. Trace the mismatch back to authored metadata, layout math, IR loss, or draw-time adjustment.
4. Implement a structural fix that explains the requested movement.
5. Re-run the focused tests and `query_ui_layout` to confirm the measured movement matches the estimate.
6. Re-render the affected screen.
7. Return to the user only for final visual confirmation.

This lets iteration continue without repeatedly asking the user to re-check half-finished passes, while still keeping the final rule structural.

## Minimum checks before claiming a fix

- For `ir_compose.rs` work: run `cargo test -p starbreaker-ui ir_compose --lib`.
- For query/debug tool changes: run the example you changed against a real canvas.
- For any renderer/layout change touching visible output: regenerate the relevant render example and inspect it.
- Before commit, run `cargo test -p starbreaker-ui`.

## Guardrails for code review

Reject a change if it does any of the following:

- branches on `MedGel`, `ui_target_a`, `ui_target_b`, ship names, manufacturer names, or specific asset paths in production logic
- introduces unexplained percentages or offsets for placement
- adds debug output that only works for one screen instead of the generic query path
- fixes a draw-time symptom while leaving a clearly wrong upstream rect or missing metadata untouched

If the source data genuinely does not contain the needed signal, prove that first with the query/debug tools before choosing the narrowest renderer-side fallback.
