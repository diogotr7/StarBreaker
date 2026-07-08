# UI Regression Policy

> Satellite doc — the authoritative process is `crates/starbreaker-ui/docs/ui-workflow.md`; commands/tools/data live in `crates/starbreaker-ui/docs/ui-reference.md`.

This policy defines required regression coverage for `starbreaker-ui` changes.

## Required for Every UI Defect Fix

When fixing a UI defect category, add or update at least one regression guard in the same change:

- Structural/layout drift: snapshot or layout assertion test.
- Typography drift: text style/spacing/font-selection assertion.
- Image/shape/tint drift: snapshot field or renderer metadata assertion.
- Binding/localization drift: binding-resolution assertion in IR compile path.

## Mandatory CI Checks

The following checks are required in CI for UI changes:

- `cargo test -p starbreaker-ui`
- `bash .github/scripts/check_ui_hardcoding.sh`
- `cargo run -p starbreaker-ui --example phase5_certification_dashboard`

## Pixel Regression: generic, not targeted

Rendered-output regressions are caught by a **content-agnostic, per-target
whole-image** comparison: `manifest_targets_whole_image_colour_regression_guard`
(`tests/manifest_visual_regression.rs`) iterates every target in the regression
manifest and fails when more than a tier-dependent fraction of pixels differ from
the frozen baseline beyond a small per-channel tolerance (platinum 0.5%, gold
1%). Adding a screen to the manifest extends coverage automatically.

Do **not** add focused/ROI/heuristic per-screen pixel checks (e.g. cyan-coverage
in a hand-picked title rectangle). Such tests only see the regions and colours
they were written for and miss everything else (a white→grey caption, a
white→blue button frame).

**The whole-image guard is bounded by its own tier budget.** It catches *large*
drift on any screen with no per-screen knowledge, but a change that touches fewer
pixels than the tolerated fraction stays under the threshold and is invisible to
it: a vanished nav arrow or a collapsed few-pixel element can disappear entirely
under a GREEN whole-image guard (ledger 77/106/107 — caught only by eye). That
**sub-threshold element-loss class** is owned — generically — by the
**element-presence guard** (`tests/element_presence_guard.rs`): for every frozen
target it requires every drawable, non-degenerately-sized node in that target's
own frozen IR snapshot to still be present, with a non-degenerate rect, in the
freshly compiled live IR. It is *derived from each target's snapshot*, so it is
generic in the same sense as the whole-image guard — adding a screen to the
freeze extends it automatically — **not** a hand-targeted ROI check: it authors
no per-screen element list and encodes no per-screen colours or coordinates.

Compose-only effects (e.g. a synthetic close-button tint) and runtime-bound text
(e.g. caption values) are invisible to the IR-level guards and rely on the pixel
guard. IR-level/tint semantics are still gated separately by
`manifest_live_ir_guard`.

## Contributor Guardrails

- New fallback logic requires:
  - entry in `crates/starbreaker-ui/docs/ui-fallback-register.md`,
  - explicit trigger signal,
  - retirement target.
- Hardcoded ship/manufacturer/screen/name/path behavior in production UI code is forbidden.
- Source-backed IR fields should be preferred over renderer-time inference.
- Any snapshot baseline update must be intentional and reviewed.

## Known-Outlier Overrides (reference-anchored)

Some elements are *knowingly* a few pixels (or one colour role, etc.) off the
in-game reference because the true fix is a deeper, higher-risk change not yet
landed. Freezing such a value as a strict baseline is harmful: it enshrines the
miss and later flags the genuine fix as a "regression". Instead, register the
element as a **known outlier** in
`crates/starbreaker-ui/tests/fixtures/ui_ir/ui_known_outliers.json`.

Each entry is `{ target, identity, field, frozen_value, reference_target,
confidence, reason, source }` and is **field-generic** — any captured snapshot
field (geometry `x/y/w/h`, `alpha`, `primary_text_top`/`secondary_text_top`,
`font_size`, a tint `*_rgba`, a tint token, ...) can be registered.

The comparator (`compare_snapshots_with_overrides`) then treats that field
**one-sided**, anchored on the measured in-game reference:

- moving **toward** `reference_target` (closer than `frozen_value`) → **passes**
  and emits a `✅ IMPROVEMENT …` note (surfaced by the live-IR guard / dashboard).
  Treat such a change as a genuine improvement: **re-freeze, do not revert it.**
- moving **away** from the target → **fails** as a regression.
- reaching within `confidence` of the target → the note suggests graduating:
  drop the outlier entry and strict-freeze at the reference.

Rules:

- The target is a **number** measured from the in-game reference (no reference
  image enters the repo; only the recorded value). Record provenance in `source`
  and the root cause in `reason`.
- `frozen_value` is validated against the live baseline; a stale entry fails
  loudly rather than silently masking drift.
- An override is an explicit, auditable IOU for a real fix — not a way to hide a
  hardcoded workaround. The hardcoding guard still applies to production code.

## Gold-standard targets and tier selection

- **`medical1` / `medical2` are PLATINUM gold-standard** outputs: deviations are
  presumed broken until proven to be an intentional, source-backed contract
  change with explicit owner approval. Never auto-write or auto-bless a medical
  baseline; baseline changes are explicit, manual, code-reviewed, and land in a
  dedicated commit that states why the drift is expected.
- **A medical render or live IR showing visible placeholder text, unresolved
  keys, or missing major elements is a PRODUCT regression signal — STOP.** That
  is not baseline drift; fix the product path first, do not touch the baseline.
- **Tier selection** when onboarding a target (the full onboarding flow is
  `ui-workflow.md` §7 + `ui-reference.md` §2/§7):
  - `platinum` — critical surfaces where drift must stay tight (medical, safety,
    high-trust readouts); pixel budget 0.5%.
  - `gold` — important but moderately variable surfaces where controlled drift is
    acceptable (e.g. `clipper_small_door`); pixel budget 1%.
  - The final tier is owner-approved.

## Review Checklist Addendum

Reviewers should verify:

- A regression test was added for the fixed defect category.
- Existing certified-family cases do not regress in the dashboard output.
- No undocumented fallback logic was introduced.
- Hardcoding guard remains green.
- A known-outlier `✅ IMPROVEMENT` note means **re-freeze, don't revert**; a new
  outlier entry is reference-anchored (measured target + provenance), not a
  cover for a hardcoded workaround.
