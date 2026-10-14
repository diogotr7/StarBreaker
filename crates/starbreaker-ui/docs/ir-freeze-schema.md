# IR Freeze Schema

Proposed: 2026-05-31. Status (2026-10): IR freeze implemented and in daily use
(M1–M3 landed). M4 was deliberately not taken: the image-artifact freeze stays as
the whole-image guard, which sees compose-only effects the IR snapshot cannot
(see §"Current Migration Status").
Scope: an IR-only, git-portable structural freeze alongside the image-artifact freeze.

## Goal

Define a freeze format that:

- Stores only IR/snapshot structural baseline data in git.
- Requires no image binaries in git.
- Produces deterministic, field-level regression failures (for example `font_size drift`) before any hash-level checks.
- Lets any developer run the same tests against local game data and local generated outputs.

## Non-Goals

- Storing PNG/JPG binaries in git.
- Using artifact image hash as the primary regression oracle.
- Hiding semantic drift behind broad hash mismatch errors.

## The image-artifact freeze (kept, now secondary)

The artifact freeze lock (`ui_regression_freeze.json`) stores image-oriented metadata:

- artifact path
- artifact hash
- width/height/channels

This detects drift but does not directly encode semantic UI invariants, and first-failure output is often not descriptive enough (for example hash mismatch rather than `font_size drift`).

## Implemented File

Path:

- `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json`

Schema version:

- Freeze file (top level): `schema_version: 1`.
- Each target's `baseline_snapshot.schema_version`: **2** (the per-element
  `UI_SNAPSHOT_SCHEMA_VERSION`, raised from 1 to add the visible text-band
  tops `primary_text_top` / `secondary_text_top`).

Top-level structure:

```json
{
  "schema_version": 1,
  "frozen_at": "2026-05-31T00:00:00Z",
  "approver": "<name>",
  "reason": "<why>",
  "signature": null,
  "manifest_path": "crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json",
  "delta": [
    {
      "target": "clipper_target_master",
      "identity": "40:widget_custom_shape",
      "field": "h",
      "old": 0.18,
      "new": 194.4
    }
  ],
  "targets": [
    {
      "id": "ui_target_a",
      "tier": "platinum",
      "category": "image",
      "baseline_snapshot": {
        "schema_version": 2,
        "canvas_guid": "...",
        "canvas_name": "...",
        "target_width": 1920,
        "target_height": 1080,
        "elements": []
      }
    }
  ]
}
```

`delta` (additive, optional in older files) is the SELF-AUDIT written by
`freeze_ui_snapshot_ir`: one entry per identity/field changed vs the previous
freeze (`<added>`/`<removed>` for new/vanished identities or targets). The
freeze tool prints it and refuses a no-op re-freeze without `--allow-empty`;
the `reason` and the freeze commit message must account for every entry
(`crates/starbreaker-ui/docs/ui-workflow.md` §7).

## Required Fields

Per target:

- `id` (must match manifest id)
- `tier` (`gold` or `platinum`; must match manifest)
- `category` (must match manifest)
- `baseline_snapshot` (canonical serialized `UiScreenSnapshot`)

Snapshot element fields should include at minimum:

- identity, category, draw order
- x/y/w/h, alpha
- text payload/case
- text font identity
- text font size (`text_font_size`)
- visible text-band tops (`primary_text_top` / `secondary_text_top`, schema v2)
- line spacing
- text/background/stroke/icon tint RGBA
- text/background/stroke/icon tint token semantics
- alignment/overflow/blend mode/asset identity

Schema-addition policy:

- When adding new snapshot semantics (for example `stroke_tint_token`, `text_tint_token`), add the fields to:
  - `UiSnapshotElement` schema
  - snapshot capture path
  - comparator checks
  - live-guard focused semantic views (only clear fields intentionally)
- Comparator policy for optional string fields remains migration-friendly:
  - baseline `Some` -> current `None` or different must fail
  - baseline `None` -> current `Some` is allowed during additive rollout
- For newly added fields that must become strict immediately on specific targets, add explicit required checks in `manifest_live_ir_guard` instead of forcing one-shot baseline churn.

## Validation Rules

1. Manifest/freeze parity:
- Target ids in manifest and freeze must be exactly 1:1.

2. Schema parity:
- Snapshot schema versions must match expected `UI_SNAPSHOT_SCHEMA_VERSION`.

3. Semantic-first comparison:
- Compare local current snapshot to frozen baseline snapshot using generic comparator.
- Report field-level deltas first.

4. Hash-last backstop (optional):
- If hash checks are retained for diagnostics, run after semantic comparisons and never as first-failure output.

5. No git image payloads:
- Freeze file must not include binary content or artifact path/hash as required baseline fields.

## Comparator Order Requirement

Required failure ordering for all required regression runs:

1. Semantic/category presence checks.
2. Typography checks (`font_size`, `font_weight`, payload/case, font identity).
3. Color/tint/alpha checks.
4. Geometry/scale checks.
5. Optional hash/dimension diagnostics last.

This ensures first failure is descriptive and actionable.

## Migration Plan (historical; M4 not taken)

Phase M1:

- Add `font_size` to `UiSnapshotElement`.
- Ensure capture path persists `font_size` from IR.
- Add comparator rule for `font_size` using tier `font_size_relative` tolerance.

Phase M2:

- Introduce `ui_snapshot_freeze.json` generation script that writes baseline snapshots only.
- Keep existing artifact freeze in parallel for one transition window.

Phase M3:

- Update validator to enforce manifest <-> IR-freeze parity.
- Make semantic comparator output mandatory and first.

Phase M4:

- Remove artifact-hash freeze from required gating.
- Keep optional image diagnostics as non-primary debugging aids.

## Script Changes

New/updated scripts:

- New: `scripts/freeze_ui_snapshot_ir.sh`
  - Loads manifest targets.
  - Produces canonical baseline `UiScreenSnapshot` payload per target.
  - Writes `ui_snapshot_freeze.json`.

- New: `scripts/validate_ui_snapshot_freeze.sh`
  - Validates manifest/freeze id parity.
  - Rejects artifact-path and sha256 fields in the IR freeze payload.
  - Requires canonical `baseline_snapshot` payloads for every frozen target.

- Updated: `scripts/validate_ui_regression_artifacts.sh`
  - Add mode validating semantic snapshot drift against IR freeze.
  - Keep image checks optional and clearly secondary.

- Existing onboarding remains:
  - `scripts/add_ui_regression_target.sh`
  - `scripts/generate_ui_regression_artifacts.sh` (local debug assets, not freeze payload)

## CI/Command Contract

Required command path must always include all standard targets and semantic-first checks.

Suggested required sequence:

1. `cargo test -p starbreaker-ui --test manifest_snapshot_regression -- --nocapture`
2. `cargo test -p starbreaker-ui --test manifest_visual_regression -- --nocapture`
3. `cargo test -p starbreaker-ui --test manifest_live_ir_guard -- --nocapture`
4. `./scripts/validate_ui_snapshot_freeze.sh`
5. `./scripts/validate_ui_regression_artifacts.sh --quick`

If any optional hash/image diagnostics run, they must execute after semantic checks and must not replace semantic failures.

## Current Migration Status

- `ui_snapshot_freeze.json` now exists as a git-storable IR-only baseline file.
- Phase M1 landed: `text_font_size` is captured per element, and the per-snapshot
  schema is now **v2** (adds `primary_text_top` / `secondary_text_top` for the
  reference-anchored text-top known-outliers).
- `scripts/freeze_ui_snapshot_ir.sh` generates the file from manifest targets and local records.
- `scripts/validate_ui_snapshot_freeze.sh` enforces manifest/freeze parity and bans artifact-path/hash fields from the IR freeze payload (M3); it runs in `scripts/ui_check.sh --full` and `scripts/ui_freeze_cycle.sh`.
- M4 is NOT planned. The image artifact freeze (`ui_regression_freeze.json`) backs the whole-image guard `manifest_targets_whole_image_colour_regression_guard`, which catches compose-only and runtime-text drift the IR snapshot cannot see (`crates/starbreaker-ui/docs/ui-regression-policy.md` §"Pixel Regression: generic, not targeted"). Both freezes are required; the IR freeze gives the descriptive first failure.

## Acceptance Criteria

- Freeze baseline is IR-only and committed in git.
- No image binaries are committed for freeze updates.
- Font-size regressions produce explicit semantic failures.
- All standard targets (gold/platinum) are always included in required runs.
- Different developers with local game data can run and reproduce equivalent semantic regression results.
