# One Brand-Context Resolver Implementation Plan (plan-B1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify the four brand/style selection paths in `starbreaker-ui` into ONE identity+family resolver in `bb_brand_style`, removing the manufacturer-prefix scan that cannot distinguish the `s_<mfr>_hud`/`s_<mfr>_env` sibling pair.

**Architecture:** One resolver picks a brand entry from a record's `brandStyles[]` by ORDERED IDENTITY candidates — `[canvas style-link, s_<mfr>_<class>, sibling, bare s_<mfr>, gen_*, s_default_*]` — where `<class>` (hud|env) is derived from the canvas family by a single shared classifier, and a per-element policy re-orders the sibling pair (separators prefer `env`). No `starts_with` prefix scans over shared standards. Migrate ONE call site per commit; each migration is verified **zero-pixel-change** (fresh export + `ui_check.sh --full`). The legacy `resolve_brand_style` prefix scan is deleted only after every site is migrated.

**Tech Stack:** Rust (`crates/starbreaker-ui`), the UI parity validation harness (`scripts/ui_check.sh`, `--full`), the decomposed exporter for fresh renders.

## Global Constraints

- **Zero pixel change is the bar.** Every migration targets byte-identical renders. Any drift is STOP-AND-DIAGNOSE, never a re-freeze-to-pass. A drift that proves the old scan was picking wrong is a latent-bug finding → adjudicate; a fix that moves a frozen baseline is a GATED owner re-freeze, OUT OF SCOPE for this pixel-neutral refactor (surface it, do not land it here).
- **TDD:** every migration writes a CHARACTERIZATION test capturing the CURRENT resolved brand FIRST (RED against the not-yet-built resolver, or GREEN pinning current behavior), then migrates, then the test + `ui_check.sh` stay green.
- **No hard-coded game-data VALUES** (palette/font/brand-list literals) in production or fixtures; synthetic test fixtures must be visibly synthetic.
- **`--full` does NOT re-export** — always fresh-export first (the decomposed exporter) before `ui_check.sh --full`; read the `ui_check: ALL GREEN` marker unpiped.
- Commit directly on `feature/ui`, one commit per task, no Co-authored-by trailer, never name the maintainer.
- Preserve ALL non-prefix-scan behavior: IC_* single-entry override, `gen_`/`s_default_` fallback, bare `s_<mfr>`, `defaultStyles` last-resort, the separator env-first policy + `orig` tail, and every activation GATE (texture-body-background, `mfd_frame`).

Research map (file:line for all four paths, per-site pixel-risk): `.superpowers/sdd/B1-research.md`.

---

### Task 1: Extract the shared hud/env classifier

**Files:**
- Modify: `crates/starbreaker-ui/src/bb_brand_style.rs` (add `BrandClass` + `brand_class_for_canvas`)
- Modify: `crates/starbreaker-ui/src/ui_ir/engine_02.rs:1295-1320` (`collect_standard_text_styles` manufacturer leg → call the classifier)
- Test: unit test in `bb_brand_style.rs`

**Interfaces:**
- Produces: `pub enum BrandClass { Hud, Env }` and `pub fn brand_class_for_canvas(canvas_name: Option<&str>) -> BrandClass`. Semantics: strip a leading `BuildingBlocks_Canvas.`, classify via existing `classify_canvas_family`, return `Hud` iff family ∈ {`Mfd`,`MfdRoot`} OR `is_cockpit_hud_canvas`, else `Env`. This is the EXACT current inline computation at `engine_02.rs:1300-1318`, lifted verbatim.

- [ ] **Step 1:** Write a unit test in `bb_brand_style.rs` asserting `brand_class_for_canvas` on representative names: `mc_*` / `m_*` → `Hud`; `hc_hud_*`/`h_hud_*`/`h_eng_*` → `Hud`; `ic_*` / `fms_*` / a plain env canvas → `Env`; a `BuildingBlocks_Canvas.`-prefixed name classifies on the stripped stem; `None` → `Env`. (Names only — no game-data values.)
- [ ] **Step 2:** Run `cargo test -p starbreaker-ui brand_class_for_canvas` → FAIL (fn absent).
- [ ] **Step 3:** Add `BrandClass` + `brand_class_for_canvas` (verbatim lift of the `engine_02.rs:1300-1318` logic). `//!`/doc the new items.
- [ ] **Step 4:** Replace the inline hud/env computation in `collect_standard_text_styles` with `let class = brand_class_for_canvas(canvas_name);` then `format!("s_{}_{}", mfr, class_str)` (add a `BrandClass::as_str` → `"hud"|"env"`). Behavior identical.
- [ ] **Step 5:** `cargo test -p starbreaker-ui` green; `bash scripts/ui_check.sh` → `ALL GREEN`. (No export needed — pure refactor of a computation, but if the compare suite covers a text-style screen it exercises the path.)
- [ ] **Step 6:** Fresh export + `ui_check.sh --full` → zero drift (path 2 is on the render path). Commit `refactor(ui-brand): extract brand_class_for_canvas classifier (B1 task 1)`.

---

### Task 2: The unified resolver — ordered identity candidates

**Files:**
- Modify: `crates/starbreaker-ui/src/bb_brand_style.rs` (add `BrandPolicy` + `brand_candidate_identifiers` + a record-level `resolve_brand_identity`)
- Test: unit tests in `bb_brand_style.rs` with a synthetic `brandStyles[]` fixture

**Interfaces:**
- Produces:
  - `pub enum BrandPolicy { Default, SeparatorEnv }`
  - `pub fn brand_candidate_identifiers(style_link: Option<&str>, manufacturer: Option<&str>, class: BrandClass, policy: BrandPolicy) -> Vec<String>` — the ordered, de-duplicated, lowercased candidate identifiers: `style_link` (if any) first; then for `Default` `[s_<mfr>_<class>, s_<mfr>_<sibling>]`, for `SeparatorEnv` `[s_<mfr>_env, s_<mfr>_hud]`; then bare `s_<mfr>`; then `gen_` and `s_default_` are matched by a separate prefix-allowed fallback (documented — those two ARE families, not a manufacturer scan). `orig` tail is appended only by the separator caller (kept caller-side to avoid polluting non-separator candidates).
  - `pub fn resolve_brand_identity<'a>(record_or_value: &'a Value, style_link: Option<&str>, manufacturer: Option<&str>, class: BrandClass, policy: BrandPolicy) -> Option<BrandStyle<'a>>` — walks `brand_candidate_identifiers` in order, returns the FIRST `brandStyles[]` entry whose `extract_record_name(brandIdentifier)` equals a candidate (identity, `eq_ignore_ascii_case`); then applies the IC_* single-entry override and the `gen_`/`s_default_` family fallback exactly as the legacy fn does. Returns the same `BrandStyle<'a>` struct.
- Consumes: `BrandClass`/`brand_class_for_canvas` (Task 1), `BrandStyle`, `extract_record_name`, `classify_canvas_family`.

- [ ] **Step 1:** Write unit tests over a SYNTHETIC `brandStyles[]` fixture (identifiers only, no real palette values): (a) style-link identity wins over family; (b) `Default`+`Hud` picks `s_syn_hud` when both siblings present; (c) `SeparatorEnv` picks `s_syn_env` when both present; (d) bare `s_syn` matches when no suffixed sibling exists (the `MC_S_Self_Master` bare-container case); (e) IC_* single-entry returns the sole entry; (f) `gen_`/`s_default_` fallback when no `s_<mfr>` entry.
- [ ] **Step 2:** `cargo test -p starbreaker-ui resolve_brand_identity` → FAIL.
- [ ] **Step 3:** Implement `brand_candidate_identifiers` + `resolve_brand_identity`. Reuse the legacy fn's IC_* and `gen_`/`s_default_` branches verbatim (move, don't rewrite).
- [ ] **Step 4:** Migrate `collect_standard_text_styles` (path 2) to pick its brand entry via `resolve_brand_identity` (style_link from the `canvas:` leg, else `manufacturer:` leg → class + `BrandPolicy::Default`). Keep the entries-merge + role-keyed map exactly.
- [ ] **Step 5:** `cargo test -p starbreaker-ui` green; `ui_check.sh` green.
- [ ] **Step 6:** Fresh export + `ui_check.sh --full` → zero drift. Commit `feat(ui-brand): unified resolve_brand_identity; migrate text-style path (B1 task 2)`.

---

### Task 3: Migrate the body-background chain (path 3)

**Files:**
- Modify: `crates/starbreaker-ui/src/bb_resolve/engine_01.rs:280-332` (`apply_body_background_standard_styles`) + the `canvas_brand` pre-resolution at `1569-1571`
- Test: characterization test for the body-bg brand pick

**Interfaces:** Consumes `resolve_brand_identity` (Task 2). Keep the texture-authoring gate (`record_authors_texture_body_background` + `body_background_uses_texture`) and the `defaultStyles` last-resort UNCHANGED.

- [ ] **Step 1:** Characterization test: for a representative body-background canvas (a DRAK cockpit MFD that authors a texture body-background), assert the currently-resolved brand identifier (capture what `resolve_brand_style` returns today — the `(A)` identity path result). Pin it.
- [ ] **Step 2:** Run it → GREEN on current code (pins today's pick).
- [ ] **Step 3:** Replace the `resolve_brand_style(std, None, preferred).or_else(resolve_brand_style(std, mfr, None))` chain (lines 302-304) with a single `resolve_brand_identity(std, style_link, mfr, brand_class_for_canvas(canvas_name), BrandPolicy::Default)`; derive `style_link`/`class`/`canvas_name` at the call site (the canvas is in scope at 1557-1571). Delete the `canvas_brand` prefix-scan pre-resolution (1569-1571) — the resolver now derives the identity directly. Keep the `else defaultStyles` branch.
- [ ] **Step 4:** Characterization test still GREEN (same pick); `cargo test -p starbreaker-ui` green; `ui_check.sh` green.
- [ ] **Step 5:** Fresh export + `ui_check.sh --full` → zero drift. If a body-background screen drifts, STOP: the old `(B)` scan was picking a different sibling — diagnose, adjudicate (do NOT re-freeze here). Commit `refactor(ui-brand): body-background via resolve_brand_identity (B1 task 3)`.

---

### Task 4: Migrate the separator path (path 4)

**Files:**
- Modify: `crates/starbreaker-ui/src/ui_ir/engine_02.rs:357-483` (`separator_standard_style_from_source` / `separator_brand_candidate_slugs`)
- Test: characterization test for the separator brand pick on an MFD frame

**Interfaces:** Consumes `brand_candidate_identifiers` with `BrandPolicy::SeparatorEnv` (Task 2). Preserve: the `mfd_frame` gate, the `(direction,style)`→record-name table, the direct-slug-first ordering, and the `orig` tail (append caller-side after the candidate list).

- [ ] **Step 1:** Characterization test: for an MFD-frame vertical/horizontal separator on a DRAK canvas, assert the resolved brand slug is `s_drak_env` today (the env-first swap). Also assert a non-MFD (medical/door) separator resolves to nothing (byte-identical no-render). Pin both.
- [ ] **Step 2:** Run → GREEN on current code.
- [ ] **Step 3:** Replace `separator_brand_candidate_slugs`'s hand-built `[s_<mfr>_env, s_<mfr>_hud, s_<mfr>, orig]` with `brand_candidate_identifiers(None, Some(mfr), brand_class_for_canvas(..), BrandPolicy::SeparatorEnv)` then push `"orig"`. Keep the direct-slug-first `match_slugs` assembly and the `mfd_frame` gate. (Env-first is now the policy, not a hard-coded list.)
- [ ] **Step 4:** Characterization tests GREEN; `cargo test -p starbreaker-ui` green; `ui_check.sh` green.
- [ ] **Step 5:** Fresh export + `ui_check.sh --full` → zero drift (all MFD separators + a non-MFD screen eyeballed — shared-mechanism rule). Commit `refactor(ui-brand): separator swap via SeparatorEnv policy (B1 task 4)`.

---

### Tasks 5–N: Migrate the remaining `resolve_brand_style` call sites (path 1), one per commit

The legacy `resolve_brand_style` has ~10 call sites (research §Path 1 table). Migrate them lowest-risk first. EACH task is one call site and follows the SAME cycle — do not batch:

**Per-site cycle (the invariant):**
- [ ] **a.** Characterization test capturing the brand this site resolves TODAY for a representative in-repo canvas (identifier only).
- [ ] **b.** Run → GREEN (pins current pick).
- [ ] **c.** Replace the `resolve_brand_style(record, mfr, preferred)` call with `resolve_brand_identity(record, style_link, mfr, brand_class_for_canvas(canvas_name), BrandPolicy::Default)`, deriving `style_link`/`canvas_name` from the canvas already in scope (research names each site's `preferred_brand` source).
- [ ] **d.** Characterization test GREEN; `cargo test -p starbreaker-ui` green; `ui_check.sh` green.
- [ ] **e.** Fresh export + `ui_check.sh --full` → zero drift; else STOP-AND-DIAGNOSE. Commit `refactor(ui-brand): migrate <site> to resolve_brand_identity (B1 task N)`.

**Migration order (lowest-risk first — the two already-identity-first sites are the template):**
- [ ] **Task 5:** `bb_resolve/engine_01.rs:303-304` — already identity-first `.or_else(scan)`; collapses to one identity call. Lowest risk.
- [ ] **Task 6:** `bb_resolve/engine_01.rs:1570` — already prefers the canvas-selected identity over the scan.
- [ ] **Task 7:** `bb_resolve/engine_01.rs:1965` (`pick_active_entries`).
- [ ] **Task 8:** `bb_resolve/engine_01.rs:884` (brand-container cascade collect).
- [ ] **Task 9:** `bb_resolve/engine_01.rs:1031` (deferred/late-state cascade).
- [ ] **Task 10:** `bb_resolve/engine_01.rs:156` (`modular_style_identifier`) + `:1590` (`modular_style_id` fallback) — both `.map(id)` consumers; migrate together only if the same canvas context (else split).
- [ ] **Task 11:** `pipeline/style_projection.rs:20` (`project_canvas_style_entries`) — palette source + Tier::Brand sheet; verify chrome palette unchanged.
- [ ] **Task 12:** `mcp/src/ui_variant_styles.rs:69` — the A4 tool. Not on the render path (`--full` won't cover it); characterize via a `ui_variant_styles` MCP call before/after on the same canvas instead. Deploy (release + copy) after.

---

### Task N+1: Delete the legacy prefix scan

**Files:** Modify `crates/starbreaker-ui/src/bb_brand_style.rs` (remove `resolve_brand_style` + its prefix-scan helpers once unreferenced)

- [ ] **Step 1:** `grep -rn "resolve_brand_style" crates/ mcp/` → only the definition + tests remain. If any production site remains, it is un-migrated — go back.
- [ ] **Step 2:** Delete `resolve_brand_style` and the now-dead `brand_identifier_basename`/prefix helpers; move any still-needed helper (IC_*/gen_ logic) into `resolve_brand_identity` if not already.
- [ ] **Step 3:** `cargo test -p starbreaker-ui` green; `ui_check.sh` green.
- [ ] **Step 4:** Fresh export + `ui_check.sh --full` → zero drift. Update the runbook's "Open architecture debt" note (the prefix-scan hazard at `ui-architecture-runbook.md:404` is now RESOLVED). Commit `refactor(ui-brand): delete legacy prefix-scan resolver (B1 done)`.

---

## Self-Review

- **Spec coverage:** B1 Step-1 (research) → `.superpowers/sdd/B1-research.md`. B1 Step-2 (one resolver: style-link → `s_<mfr>_{hud|env}` by family → sibling swap, identity-only, no prefix scans) → Tasks 1-2 (classifier + resolver), Tasks 3-12 (migrate every call site), Task N+1 (delete the scan). B1 Step-3 (per-migration failing-characterization-test → migrate → ui_check → disable→adjudicate + fresh export + `--full`, zero pixel change) → the per-site cycle + every task's fresh-export `--full` step.
- **No placeholders — with a caveat the parent plan sanctions:** the resolver's exact BODY (Task 2 Step 3) and each migration's exact diff are written test-first AT EXECUTION because a pixel-neutral refactor's edits depend on the code state when reached (parent plan §Phase B: "their step-level code depends on the codebase state when reached, and pretending otherwise now would write placeholders"). The safety mechanism that replaces upfront exact-code is the CHARACTERIZATION-TEST-FIRST invariant: current behavior is pinned before each edit, so "minimal code" = "make the pinned test stay green." Interfaces (signatures, enum variants, candidate ordering) ARE fully specified above.
- **Type consistency:** `BrandClass`/`BrandPolicy`/`brand_class_for_canvas`/`brand_candidate_identifiers`/`resolve_brand_identity`/`BrandStyle` used identically across tasks. `class.as_str()` → `"hud"|"env"` defined in Task 1.
