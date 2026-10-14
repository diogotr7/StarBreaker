# starbreaker-ui

`starbreaker-ui` compiles Star Citizen UI data into deterministic, testable outputs.

## Overview

This crate provides a data-driven pipeline that:
- Parses and resolves BuildingBlocks canvas graphs.
- Resolves bindings, localization, default state, and brand/style modifiers.
- Computes layout and compiles canonical UI IR.
- Renders static outputs from canonical IR.
- Produces snapshot/metadata artifacts for regression validation.

Core constraints:
- No production hard-coded ship/canvas/asset name heuristics (crate
  `AGENTS.md` "Core rules").
- Deterministic output for equivalent inputs.
- A 3000-line cap on every `.rs` under `src/`, enforced by
  `tests/line_count_guard.rs`; split by responsibility well before it
  (target ~2500 lines).

The four stage engines (`bb_layout`, `bb_resolve`, `ir_compose`, `ui_ir`) are
directories of real `engine_NN.rs` submodules behind a `mod.rs` (converted
from `include!`-spliced `.part` files in ledger item 105).

## Key Entry Points

- `src/lib.rs`: crate exports and top-level module map.
- `src/pipeline/mod.rs`: binding-driven orchestration from sources to IR/image.
- `src/ui_ir/mod.rs`: canonical IR schema + compiler.
- `src/ir_compose/mod.rs`: canonical IR renderer.
- `src/ui_snapshot/mod.rs`: structural + metadata snapshot comparison.

## Quick Navigation

Render path (BB data -> image):
1. `pipeline::compile_ir_for_binding`
2. `ui_ir::compile_ui_ir_from_scene`
3. `ir_compose::render_ui_ir_document`
4. Optional post-process in `postprocess`

Defaults/state path:
1. `defaults::DefaultValueRegistry`
2. `bb_bindings` resolver lookups
3. `bb_state_filter` static visibility filtering
4. `bb_brand_apply` style/tag-driven modifiers

Regression path:
1. `ui_snapshot::snapshot_from_ui_ir`
2. `ui_snapshot::compare_snapshots`
3. `ui_regression_manifest::compare_manifest_targets_with_loader`
4. integration tests under `tests/manifest_*`

Each source file opens with a `//!` header describing its responsibility; read
those (or use graphify, `crates/starbreaker-ui/docs/ui-reference.md` §4b)
rather than a hand-kept file list. Each file in `tests/` is its own test
binary (`cargo test -p starbreaker-ui --test <file-stem>`); `examples/` holds
the diagnostics listed in `crates/starbreaker-ui/docs/ui-reference.md` §7.

## Regression Policy

- Visual and snapshot regression failures are product regressions until proven otherwise.
- First response must be root-cause investigation and code/data-flow fixes.
- Do not change baselines, thresholds, or regression tests as an initial workaround.
- Baseline/test updates are allowed only after root-cause fix is implemented and validated.

Full policy: `crates/starbreaker-ui/docs/ui-regression-policy.md`. The
working process for UI parity is `crates/starbreaker-ui/docs/ui-workflow.md`.
