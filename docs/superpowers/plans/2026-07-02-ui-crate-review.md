# starbreaker-ui Crate Review — Findings & Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Land the safe, behaviour-identical improvements from the 2026-07-02 full-crate review; record the larger architectural recommendations with their risk notes for owner decision.

**Review method:** module-map + size profile + targeted source reads (`lib.rs`, `bb_resolve`/`ir_compose`/`bb_layout`/`ui_ir` split wrappers + part headers, `bb_style_engine.rs`, `bb_brand_apply/*`, `pipeline/mod.rs`, `bb_scene/*`, expansion/test code), grounded in the Carrack arc's lived-in-code experience. graphify god-nodes unavailable during review (tooling outage) — and would miss the `.part` core anyway (its documented blind spot).

## Findings

### F1 — `engine_parts/*.part` include-splice architecture (major, gated)
12 flat `.part` chunks of 2.0–3.0k lines (~31k total) spliced via `include!("engine.inc")` into four single-namespace stage modules (`bb_resolve`, `ui_ir`, `ir_compose`, `bb_layout`). Consequences, all already being paid: graphify cannot index the core (the skill carries TWO red-flag rows just to warn about it); no `//!` docs possible (AGENTS.md requires them); no enforced boundaries ("later depends on earlier" by comment only); rustfmt does not format non-`.rs` files; the chunks were already "consolidated" once (engine_01 header lists 7 former parts), i.e. split by line-count, not responsibility, contra AGENTS.md.
**Recommendation:** convert one stage (pilot: `bb_resolve`, the most-touched) to real submodules — `engine_NN.rs` + `use super::*;`, promoting cross-part private items to `pub(super)` compiler-error-driven. Pure code motion; verify with the full test suite + `ui_check.sh` marker + a before/after render md5 on a frozen screen. Roll to the other three stages only after the pilot proves cheap. GATE with the owner first — highest-churn files in the repo.

### F2 — component-standard dispatch is table-shaped but written as 5 scattered matches (medium)
Adding `ComponentGeneralButton` required edits at: hosts filter, template-path match, params match, icon/label forwarding gate, fallback-clear gate, `implicit_host_tag_name` — the exact "one gap repeated at three levels" that caused the arc's bug (ledger 103). **Recommendation:** a `ComponentStandardSpec { ty, template_path, implicit_tag, params: fn, forwards_label: bool }` registry consulted by all sites. Behaviour-identical refactor, unit-testable.

### F3 — three duplicated expansion-child stack walks (small)
`forward_host_icon_identity`, `forward_host_label_case`, `tag_expanded_instances` repeat the identical `children ≥ EXPANSION_ID_BASE` DFS. **Recommendation:** one `for_each_expanded_instance(scene, host_id, f: impl FnMut(&mut BbNode))` helper; the three become closures.

### F4 — four near-identical `modular_*_style_path` functions (small)
`linearprogress`/`buttonprimary`/`buttonsecondary`/`scrollbar` differ only in the filename suffix. **Recommendation:** `modular_kit_style_path(style_id, component: &str)` + thin wrappers or direct calls.

### F5 — stale-token hazard fixed for one arm only (small, correctness-adjacent)
`apply_color_field`'s Fill/Stroke/Background arm now drops a stale `<Field>Token` on literal application (`29c2c1fc3`); the `BorderColor*` arms still call `write_color_token_to_raw` (silent no-op on None) — same latent shadowing class. **Recommendation:** move the drop-on-None semantics INTO `write_color_token_to_raw` (one place, all arms), with a unit test mirroring the fill-token test.

### F6 — PngCache stringly-typed keys duplicated (small, cross-file)
`cached_load_keyed` (textures.rs) formats `"{path}@mip{mip}{disc}"`; `prewarm_decomposed_textures` (decomposed.rs) re-formats the same string by hand — the exact key-drift class behind the `@n` regression hunt. **Recommendation:** `pub(crate) fn png_cache_key(path, mip, discriminator) -> String` in textures.rs, used by both.

### F7 — doc drift: `bb_style_engine.rs` header (trivial)
Header says the text-format route "is gated on `Tier::Brand`" — stale since the LR-indicator arc added the Embedded-tier bare-`Type(Text)` route (ledger 96/97). Fix the sentence.

### F8 — expansion is single-level; element tags approximate nesting (recorded, no action now)
The engine nests instance widgets' own standards; `expand_widget_standards` runs once and the `icon-/text-element-instance` tag attachment reproduces the styling identity (ledger 103). A fixed-point expansion loop would be truer but re-shifts every button-bearing frozen IR baseline for zero pixel change. Not worth the churn until a screen NEEDS deeper nesting — record only.

### F9 — new MCP UI tool candidates (recorded for the MCP backlog)
(a) `ui_tag_lookup` (uuid↔name over the tag db) and (b) `ui_kit_sheet_entries` (entries + tag-resolved conditions) — trivial ports of `ui_canvas_query.py` into `mcp/src/tools.rs`. (c) `ui_render_canvas` MCP variant is NOT recommended: the deployed server binary goes stale against working-tree engine changes, which is precisely when renders matter — the script (builds current tree) is the right tool. Implementing (a)/(b) requires the MCP kill/redeploy cycle — schedule outside an active session.

## Tasks (safe subset — F3, F4, F5, F6, F7)

### Task 1: shared expansion traversal (F3)
- [ ] In `bb_resolve/engine_parts/engine_04.part`, add `fn for_each_expanded_instance(scene: &mut BbScene, host_id: BbNodeId, mut visit: impl FnMut(&mut crate::bb_scene::BbNode))` implementing the stack walk (children filtered `>= EXPANSION_ID_BASE`, recursing via child lists).
- [ ] Rewrite `forward_host_icon_identity`, `forward_host_label_case`, `tag_expanded_instances` bodies as calls with closures (capture preset/custom/case/tag first to satisfy borrows).
- [ ] `cargo test -p starbreaker-ui --lib` → ok; commit `refactor(ui): shared expansion-instance traversal (review F3)`.

### Task 2: single modular-kit path builder (F4)
- [ ] In `bb_resolve/engine_parts/engine_01.part`, add `fn modular_kit_style_path(style_identifier: &str, component: &str) -> Option<String>` (the shared s_→sk_ mapping + `format!(".../{0}/{0}_{component}styles.json")`).
- [ ] Replace the four functions' bodies with calls (keep their names/signatures — call sites unchanged).
- [ ] `cargo test -p starbreaker-ui --lib` → ok; commit `refactor(ui): one modular-kit sheet path builder (review F4)`.

### Task 3: token-drop semantics in one place (F5)
- [ ] Unit test in `bb_brand_apply/tests_colors.rs`: apply a tokened border colour, then a literal (token=None) via `apply_color_field`; assert `BorderColorTopToken` removed.
- [ ] Move drop-on-None into `write_color_token_to_raw` (remove the key when token is None); delete the now-redundant inline branch in the Fill arm.
- [ ] Full lib tests + `bash scripts/ui_check.sh` (read the marker) — behaviour identical on frozen targets; commit `fix(ui): literal colour application drops stale tokens for ALL colour fields (review F5)`.

### Task 4: shared PNG-cache key builder (F6)
- [ ] `pub(crate) fn png_cache_key(path: &str, mip: u32, discriminator: &str) -> String` in `starbreaker-3d/src/pipeline/textures.rs`; use in `cached_load_keyed` and `prewarm_decomposed_textures`.
- [ ] `cargo test -p starbreaker-3d --lib` → ok; commit `refactor(3d): shared png_cache key builder (review F6)`.

### Task 5: doc fixes (F7)
- [ ] Correct the `bb_style_engine.rs` header sentence (Brand + the Embedded bare-`Type(Text)` route, cite ledger 96/97).
- [ ] Commit `docs(ui): style-engine header reflects the Embedded text-format route (review F7)`.

### Task 6: record F1/F8/F9 for the owner
- [ ] Append the F1 pilot proposal + F8/F9 notes to the ledger as the review's numbered entry; the F1 conversion itself is OWNER-GATED (highest-churn files).
