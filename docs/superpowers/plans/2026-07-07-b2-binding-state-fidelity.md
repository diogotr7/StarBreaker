# B2 Binding/State Fidelity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the three guarded binding/state-fidelity fixes of parent-plan Task B2 — a second expansion ID band, a slash-path exclusion regression test, and a widened text-format route — each as its own commit with its own regression guard.

**Architecture:** Three independent one-commit deliverables in the `starbreaker-ui` crate, each starting with a failing test (TDD). Fixes 1 and 2 are pixel-neutral (byte-identical snapshots); fix 3 is the only one that deliberately moves pixels — it activates a real text drop at the StyleLink and Shared cascade tiers and its moved targets are adjudicated toward their references. Approach and file:line map are fixed by the completed research (`.superpowers/sdd/B2-research.md`); this plan is the task breakdown, not a redesign.

**Tech Stack:** Rust (`crates/starbreaker-ui/`), `cargo test -p starbreaker-ui`, `bash scripts/ui_check.sh [--full]`, the release CLI exporter (`target/release/starbreaker entity export`).

**Sequencing (parent plan line 329):** B2 runs after B1 (LANDED `0f612f73f`), before B3/B4; pixel-neutral fixes (1, 2) first, the pixel-moving fix (3) last within B2.

## Global Constraints

Copied verbatim from the parent plan (`docs/superpowers/plans/2026-07-04-ui-parity-process-and-crate-plan.md`, Global Constraints):

- **GATE: no task in this plan executes until the owner's explicit go** (another agent is mid-arc in this repo). The plan document itself is the only pre-go artifact.
- No hard-coding: no name/ship/screen/manufacturer branches, no magic offsets, **no hard-coded game-data values** (palette literals, font sizes, brand lists) in production code, fallbacks, or fixtures. Data files transcribed from authoritative docs get a `.notes.md` provenance sidecar (registry pattern).
- Process/tooling changes must NOT alter render behaviour (ledger rule). `bash scripts/ui_check.sh` green before every commit.
- Validation scripts print a RESULT MARKER (`<tool>: OK …` / `<tool>: FAILED (exit N)`) via an EXIT trap; never rely on a piped exit code.
- TDD: failing test/self-check first for every behavioural change.
- Commits directly on `feature/ui`; no new branches/worktrees. One commit per coherent task. Never name the maintainer (use "the owner"); no `/home/<user>` paths in repo content (`$HOME`/`~` only).
- Sub-agents: read-only research only, default model **Opus**; anything touching cargo target/build/render/test runs sequentially in the main session.
- Verify-on-write: every command line added to a doc is run once at writing time; renames include a repo-wide reference grep in the same commit.
- After doc-file changes land, run the graphify doc sync (`/graphify . --update`) once at the end of the phase.

B2-specific constraints (exact values from `.superpowers/sdd/B2-research.md`):

- **TDD is mandatory and observed:** write the failing test first, run it and watch it FAIL, implement, run it and watch it PASS. One commit per fix, committed directly on `feature/ui`, no `Co-authored-by` trailer, never name the maintainer.
- **No hard-coded game-data VALUES** (palette / font / brand literals) in production code OR fixtures. Fixtures invented for these tests must be visibly synthetic (arbitrary sizes like `100.0`, placeholder names like `existing`/`new_type`, synthetic tag ids like `tagT`) — never a copy of a real in-game value.
- **`--full` export protocol:** any `ui_check.sh --full` run in this plan MUST be preceded by `cargo build --release` and a fresh export using `--lod 0 --mip 0 --materials all`. LOD1 culls small HUD screens, producing spurious dimension drift in the snapshot guards — LOD0 with all mips/materials is the only valid `--full` input.
- **Pixel-neutral is the bar for fixes 1 and 2.** For fix 3, moved snapshot/pixel targets are EXPECTED where it fixes a real text drop; those moved targets are adjudicated TOWARD their in-game reference and re-frozen ONLY if in scope for this plan. A **regression** (movement away from reference) on any target is stop-and-diagnose — never re-freeze to pass.

---

## Task 1: Second expansion ID band (latent-stability)

Decouple expansion-node identity from expansion ORDER so a host TYPE added AFTER today's set cannot renumber the frozen instance IDs of the existing types. Today's four expanding host types keep byte-identical allocations in the primary band `0xF000_0000`; only a future/new host type is routed to a reserved second band `0xF800_0000`. This is pure latent-stability / architecture-debt insurance — the concrete evidence is the one-time medical close-X theft (`4026531855 = 0xF000000F` "became a separator"; handoff `ui-clipper-parity-handoff.md:130-132`, runbook `ui-architecture-runbook.md:395-402`).

> **STALE-MOTIVATION FLAG:** the parent-plan text says fix 1 "Unblocks the parked separator-dots work." That motivation is STALE — separator-dots already LANDED (`af9ce13d3`) via the MFD-frame gate + brand-SVG route, with no expansion/ID-band involvement. Reframe this task as latent-stability: it prevents a FUTURE host-type addition from stealing frozen expansion IDs. Do NOT reintroduce or re-plumb separator-dots here.

**Files:**
- Modify: `crates/starbreaker-ui/src/bb_resolve/engine_01.rs` — `EXPANSION_ID_BASE` const region (`:2296-2300`) and `merge_child_scene` (`:2302-2358`, band-allocation block `:2328-2357`), plus its two intra-file canvas-merge call sites at `:1585` and `:1961`.
- Modify: `crates/starbreaker-ui/src/bb_resolve/engine_04.rs` — the expansion call site `expand_widget_standards` `:1900` (chooses the band per host type).
- Test: `crates/starbreaker-ui/src/bb_resolve/engine_04.rs` — new test in `mod tests_expansion` (module starts `:2293`; peers `button_expansion_forwards_host_icon_identity_to_icon_instance` `:2399`, `scrollbar_expansion_pairs_thumb_with_target_view` `:2657`; `use super::*` already brings `merge_child_scene`, `EXPANSION_ID_BASE`, `parse_bb_canvas`, `BbScene`, `BbNodeId`, `BbNodeType` into scope via `bb_resolve/mod.rs`'s `pub use engine_01::*`).

**Interfaces:**
- Consumes: `merge_child_scene(parent_scene: &mut BbScene, child_scene: BbScene, match_to: &str, host_parent_override: Option<BbNodeId>, band_base: Option<BbNodeId>)` — the fifth parameter changes from `reserve_id_band: bool` to `band_base: Option<BbNodeId>`. `None` = ordinary low-band canvas merge (was `false`); `Some(base)` = expansion, allocating within the band that starts at `base` (was `true`, implicitly `Some(EXPANSION_ID_BASE)`).
- Produces: `pub(crate) const EXPANSION_ID_BASE_SECOND: BbNodeId = 0xF800_0000;` (engine_01.rs) and `fn expansion_band_for_host_type(ty: &BbNodeType) -> BbNodeId` (engine_04.rs) mapping today's four types → `EXPANSION_ID_BASE`, anything else → `EXPANSION_ID_BASE_SECOND`.

- [ ] **Step 1: Write the failing test** in `mod tests_expansion` (engine_04.rs, alongside the peers named above):

```rust
    /// A host TYPE added AFTER today's set (WidgetIcon / general button /
    /// scrollbar) must NOT steal the primary expansion band from an existing
    /// host's already-frozen instance IDs. Characterizes the medical close-X
    /// theft (handoff 130-132 / runbook 395-402): before this fix a new host
    /// type sorting at a lower node id expanded first and consumed
    /// 0xF000_0000, shifting every downstream expansion ID. Latent-stability
    /// only — no production host type reaches the second band today.
    #[test]
    fn new_host_type_expansion_does_not_shift_existing_low_band_ids() {
        // One-node child scene standing in for a merged template instance.
        let child = |name: &str| {
            parse_bb_canvas(&serde_json::json!({
                "_RecordValue_": {
                    "_Type_": "BuildingBlocks_Canvas",
                    "size": {"x": 1.0, "y": 1.0},
                    "scene": [
                        {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": name}
                    ]
                }
            }))
            .expect("child canvas should parse")
        };
        let parent = || {
            parse_bb_canvas(&serde_json::json!({
                "_RecordValue_": {
                    "_Type_": "BuildingBlocks_Canvas",
                    "size": {"x": 100.0, "y": 100.0},
                    "scene": [
                        {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget", "name": "root"}
                    ]
                }
            }))
            .expect("parent canvas should parse")
        };
        let merged_id = |scene: &BbScene, name: &str| -> BbNodeId {
            scene.nodes.values().find(|n| n.name == name).expect("merged node present").id
        };

        // Baseline: an existing host type expands into the primary band.
        let mut base = parent();
        merge_child_scene(&mut base, child("existing"), "", None, Some(EXPANSION_ID_BASE));
        let existing_solo = merged_id(&base, "existing");
        assert_eq!(
            existing_solo, EXPANSION_ID_BASE,
            "an existing host type's instance takes the primary band base"
        );

        // Theft scenario: a NEW host type merges FIRST (sorts earlier), then the
        // existing type. The new type must land in the reserved SECOND band and
        // leave the existing type's primary-band id untouched.
        let mut with_new = parent();
        merge_child_scene(&mut with_new, child("new_type"), "", None, Some(EXPANSION_ID_BASE_SECOND));
        merge_child_scene(&mut with_new, child("existing"), "", None, Some(EXPANSION_ID_BASE));
        assert!(
            merged_id(&with_new, "new_type") >= EXPANSION_ID_BASE_SECOND,
            "the new host type lands in the reserved second band (0xF800_0000)"
        );
        assert_eq!(
            merged_id(&with_new, "existing"), existing_solo,
            "existing host type's instance id is invariant to the new type (no theft)"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p starbreaker-ui new_host_type_expansion_does_not_shift_existing_low_band_ids`
Expected: FAIL to COMPILE — `cannot find value EXPANSION_ID_BASE_SECOND in this scope` and `merge_child_scene` arity/type mismatch (`Option<BbNodeId>` passed where `bool` expected). This is the intended Rust TDD red: the test encodes behaviour the crate cannot yet build. (After the fix it becomes a live behavioural guard — the final `assert_eq!` would fail if anyone reverts to a single shared `next_high`.)

- [ ] **Step 3: Add the second-band constant** in `engine_01.rs` immediately after `EXPANSION_ID_BASE` (`:2300`):

```rust
/// Node IDs at or above this value belong to the SECOND expansion band,
/// reserved for host types added AFTER today's set (WidgetIcon / general
/// button / scrollbar). Routing new types here keeps their instance IDs from
/// renumbering the frozen primary-band allocations of the existing types
/// (latent-stability; runbook "Open architecture debt"). No production host
/// type maps here today — it is byte-identical for every frozen screen.
pub(crate) const EXPANSION_ID_BASE_SECOND: BbNodeId = 0xF800_0000;
```

- [ ] **Step 4: Change `merge_child_scene` to allocate within the requested band.** Replace the `reserve_id_band: bool` parameter (`:2307`) with `band_base: Option<BbNodeId>`, and replace the `next_high` computation (`:2336-2343`) and the per-node counter choice (`:2346`) so allocation continues within `[band_lo, band_hi)` of the requested band. `parent_scene.nodes` is a `BTreeMap<BbNodeId, _>`, so `.range(..)` gives per-band continuation:

```rust
pub(crate) fn merge_child_scene(
    parent_scene: &mut BbScene,
    child_scene: BbScene,
    match_to: &str,
    host_parent_override: Option<BbNodeId>,
    band_base: Option<BbNodeId>,
) {
    // ... (child_scene destructure + next_low unchanged) ...

    // Expansion merges reserve within a band; canvas merges (band_base None)
    // do not. Today's four host types share the primary band, a future host
    // type gets the second band, so the two never renumber each other.
    let reserve = band_base.is_some();
    let band_lo = band_base.unwrap_or(EXPANSION_ID_BASE);
    let band_hi = if band_lo < EXPANSION_ID_BASE_SECOND {
        EXPANSION_ID_BASE_SECOND
    } else {
        BbNodeId::MAX
    };
    let mut next_high: BbNodeId = parent_scene
        .nodes
        .range(band_lo..band_hi)
        .next_back()
        .map(|(&k, _)| k.wrapping_add(1))
        .unwrap_or(band_lo);
    let mut id_map: HashMap<BbNodeId, BbNodeId> = HashMap::with_capacity(child_nodes.len());
    for &orig_id in child_nodes.keys() {
        let counter = if reserve || orig_id >= EXPANSION_ID_BASE {
            &mut next_high
        } else {
            &mut next_low
        };
        while parent_scene.nodes.contains_key(counter) || id_map.values().any(|&v| v == *counter) {
            *counter = counter.wrapping_add(1);
        }
        id_map.insert(orig_id, *counter);
        *counter = counter.wrapping_add(1);
    }
    // ... (rest of the function unchanged) ...
}
```

Note: for today's single-band case (`band_base = Some(EXPANSION_ID_BASE)`, no second-band nodes present), `range(EXPANSION_ID_BASE..EXPANSION_ID_BASE_SECOND).next_back()` returns exactly what the old `keys().next_back().filter(k >= EXPANSION_ID_BASE)` returned — hence byte-identical. Update the `//!`/doc comment on the band-allocation block to describe the two-band scheme.

- [ ] **Step 5: Update the three call sites.**
  - `engine_01.rs:1585` and `engine_01.rs:1961` (canvas merges): change the last argument `false` → `None`.
  - `engine_04.rs:1900` (expansion): choose the band per host type. Add above `expand_widget_standards` (or near it):

```rust
/// Today's expanding host types share the primary expansion band; any host
/// type added LATER maps to the second band so it cannot renumber the frozen
/// instance IDs of the types above (latent-stability — see runbook "Open
/// architecture debt").
fn expansion_band_for_host_type(ty: &BbNodeType) -> BbNodeId {
    match ty {
        BbNodeType::WidgetIcon
        | BbNodeType::ComponentGeneralButton
        | BbNodeType::ComponentGeneralButtonSecondary => EXPANSION_ID_BASE,
        ty if is_scrollbar_component(ty) => EXPANSION_ID_BASE,
        _ => EXPANSION_ID_BASE_SECOND,
    }
}
```

  and change the call at `:1900`:

```rust
        let band_base = expansion_band_for_host_type(&host_ty);
        merge_child_scene(scene, child_scene, "", Some(host_id), Some(band_base));
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p starbreaker-ui new_host_type_expansion_does_not_shift_existing_low_band_ids`
Expected: PASS. Then run the existing expansion peers to confirm no regression: `cargo test -p starbreaker-ui tests_expansion` — all green (icon-identity, scrollbar-pairing, etc.).

- [ ] **Step 7: Prove byte-identical for today's host types.** `cargo build --release`, re-export the standard `--full` target (Clipper) with `--lod 0 --mip 0 --materials all`, then `bash scripts/ui_check.sh --full`. Freeze snapshots key by `<node_id>:<node_type>` (`ir-freeze-schema.md:58`), so ANY expansion-ID shift on power (buttons + scrollbars), MFD footers (button icons), medical (close-X icon + scrollbars), or Carrack lift-call console (primary buttons) trips the guard. Expected: `ui_check: ALL GREEN` with ZERO snapshot movement — the goal is that no current type's allocation moved.

- [ ] **Step 8: Commit**

```bash
git add crates/starbreaker-ui/src/bb_resolve/engine_01.rs crates/starbreaker-ui/src/bb_resolve/engine_04.rs
git commit -m "fix(ui): reserve a second expansion ID band so a new host type cannot steal frozen instance IDs (plan B2-1)

Latent-stability: today's four expanding host types keep byte-identical
primary-band (0xF000_0000) allocations; only a host type added later maps to
the reserved second band (0xF800_0000). Characterization test drives the
medical close-X theft case. Separator-dots (already landed af9ce13d3) is NOT
the motivation."
```

---

## Task 2: bb_state_filter slash-path exclusion regression test

The production discriminator already LANDED (`2538c8743`, ledger 87/88): `numeric_variable_defaults` at `bb_state_filter/mod.rs:937` excludes slash-prefixed absolute engine-state paths from static at-rest resolution (`if binding.is_empty() || binding.contains('/') || out.contains_key(binding) { continue; }`). Resolving those slash-paths regressed the `clipper_self_master` gold baseline (+12 elements). The deliverable here is the MISSING regression test that PINS that `binding.contains('/')` clause — no production change. Only the bare firing-state vars have positive tests today (`firing_overlay_hidden_when_burst_state_pinned_zero` `:811`, `firing_overlay_shown_when_bursting` `:842`); nothing exercises the slash-path negative case, so the load-bearing discriminator is guarded only by the slow `ui_check --full`.

**Files:**
- Test: `crates/starbreaker-ui/src/bb_state_filter/tests_b.rs` — new test beside `firing_overlay_hidden_when_burst_state_pinned_zero` (`:811`). Harness in scope: `make_record_value(static_vars, ops)` (`:25`), `instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(rv, &[], &HashMap::new(), Some(&defaults))` (mod.rs `:163`), `crate::defaults::DefaultValueRegistry::{new, insert_path}`, `crate::canvas::Value`.
- Modify: NONE in production (the guard is the product). Registry normalisation confirmed: `DefaultValueRegistry::normalise_path` trims a leading `/` and lowercases (`defaults/registry.rs:96`), so a registry entry keyed `/seatdashboard/powerstate` is found by `lookup_path` once the guard is bypassed — the negative case is genuinely reachable.

**Interfaces:**
- Consumes: the existing at-rest resolver path (`instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults` → `numeric_variable_defaults` → `BooleanFromInteger`/`BooleanField IsActive`). No new API.
- Produces: a regression test that FAILS iff the `binding.contains('/')` clause at `mod.rs:937` is removed.

- [ ] **Step 1: Write the test** in `tests_b.rs` (beside `:811`). Two `IsActive = (var > 1)` overlays: one gated on a BARE binding, one on a SLASH-PATH; the registry pins BOTH to 0. Under the guard the bare gate resolves (overlay 5 hides) and the slash-path gate stays on the heuristic (overlay 6 shown). Written inline (like the firing tests) because `boolean_field_op` uses `Instantiated`, and this case needs `IsActive`:

```rust
    /// The at-rest numeric resolver is COMPONENT-LOCAL: it resolves BARE
    /// component-relative bindings only. Absolute engine-state paths (slash-
    /// prefixed) are excluded and stay on the heuristic — resolving them
    /// regressed the clipper_self_master gold baseline (ledger 88). This pins
    /// the `binding.contains('/')` discriminator at mod.rs:937: it must FAIL
    /// if that clause is deleted.
    #[test]
    fn slash_path_numeric_gate_excluded_from_static_resolution() {
        let rv = make_record_value(
            vec![],
            vec![
                // Bare component-local var → resolves at rest.
                json!({"_Pointer_":"ptr:13","_Type_":"BuildingBlocks_BindingsIntegerVariable","binding":"CurrentBurstSize"}),
                json!({"_Pointer_":"ptr:19","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Greater","inputL":"_PointsTo_:ptr:13","value":1}),
                json!({"_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:5","field":"IsActive","input":"_PointsTo_:ptr:19"}),
                // Absolute engine-state path → MUST stay on the heuristic.
                json!({"_Pointer_":"ptr:14","_Type_":"BuildingBlocks_BindingsIntegerVariable","binding":"/seatdashboard/powerstate"}),
                json!({"_Pointer_":"ptr:21","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Greater","inputL":"_PointsTo_:ptr:14","value":1}),
                json!({"_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:6","field":"IsActive","input":"_PointsTo_:ptr:21"}),
            ],
        );
        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("CurrentBurstSize", crate::canvas::Value::Int(0));
        defaults.insert_path("/seatdashboard/powerstate", crate::canvas::Value::Int(0));
        let false_set = instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
            &rv,
            &[],
            &std::collections::HashMap::new(),
            Some(&defaults),
        );
        // Bare gate resolves 0 > 1 = false → overlay 5 hides (resolver active).
        assert!(
            false_set.contains(&5),
            "bare component-local gate resolves at rest (CurrentBurstSize=0 → 0>1 false → hidden)"
        );
        // Slash-path gate stays on the heuristic → overlay 6 shown (NOT hidden).
        // FAILS if mod.rs:937's `binding.contains('/')` clause is deleted: the
        // slash var would then resolve from the registry (0 > 1 = false) and
        // wrongly hide overlay 6, regressing the frozen at-rest visibility.
        assert!(
            !false_set.contains(&6),
            "slash-path gate must stay on the heuristic (component-local scoping is load-bearing)"
        );
    }
```

- [ ] **Step 2: Run the test to verify it passes against current (correct) code**

Run: `cargo test -p starbreaker-ui slash_path_numeric_gate_excluded_from_static_resolution`
Expected: PASS (the guard is already present, so both assertions hold).

- [ ] **Step 3: Watch it FAIL with the clause removed (the TDD proof this test is load-bearing).** Temporarily edit `bb_state_filter/mod.rs:937`, deleting only the `|| binding.contains('/')` sub-clause, so the line reads `if binding.is_empty() || out.contains_key(binding) { continue; }`. Run the same command.
Expected: FAIL on the second assertion (`slash-path gate must stay on the heuristic` — overlay 6 is now wrongly in the false set). If it does NOT flip, the registry key normalisation differs from what `lookup_path` receives — re-check `normalise_path` and key the slash entry so the negative case is reachable before proceeding.

- [ ] **Step 4: Restore the clause and re-verify PASS.** Revert the `mod.rs:937` edit (the production line must end this task byte-for-byte unchanged). Run: `cargo test -p starbreaker-ui slash_path_numeric_gate_excluded_from_static_resolution` → PASS. Then `bash scripts/ui_check.sh` → `ui_check: ALL GREEN` (test-only change; the live-IR manifest guard is the byte-identical proof).

- [ ] **Step 5: Commit** (test-only; no production diff):

```bash
git add crates/starbreaker-ui/src/bb_state_filter/tests_b.rs
git commit -m "test(ui): pin bb_state_filter slash-path exclusion discriminator (plan B2-2)

Regression guard for the component-local at-rest scoping landed in 2538c8743:
a bare binding resolves at rest, an absolute slash-path stays on the
heuristic. Fails iff mod.rs:937's binding.contains('/') clause is deleted."
```

---

## Task 3: Widen the text-format route to StyleLink + Shared tiers

The `Type(Text)` text-format route (a `Type(Text)` selector styling a `WidgetTextField`'s implicit text-format child — FontSize/FillColor) is tier-gated in ONE place, `bb_style_engine.rs:113-117`: `Full` at Brand, `BareTextOnly` at Embedded (ledger 96), `Off` everywhere else. Extend it to the remaining substantive tiers so an unconditional bare `Type(Text)` FontSize authored at StyleLink or Shared reaches the field — with the SAME discriminator as Embedded (conditional selectors stay Brand-only). This is the ONLY B2 fix expected to move pixels: it activates a real text drop that today only velocity-num/master-mode dodge via the targeted no-brand-match `defaultStyles`-at-Brand workaround (`bb_resolve/engine_01.rs:1150`).

Tier enumeration (`bb_style_engine.rs:24-41`; authoritative map `ui-cascade-passes.md` table `:28-39`):
- **Brand** → `Full` (unchanged): routes conditional overrides too (target `Bright Elements`, medbed `Textfield_BrightColor_Override`).
- **Embedded** → `BareTextOnly` (unchanged, ledger 96).
- **StyleLink** (pass 4, canvas `style` link applied only when no brand) → `BareTextOnly` (NEW). Also reached by root-canvas re-application (`pipeline/style_projection.rs:31-46` applies root `defaultStyles.entries` at `Tier::StyleLink`) — the concrete remaining-tier gap where a root bare `Type(Text)` FontSize was dropped.
- **Shared** (pass 5, `defaultStyles.sharedStyles` — `mfd_g_*`) → `BareTextOnly` (NEW).
- **StandardModule** (widget-standard module sheets `sk_<brand>_*`, passes 1/9 — element-instance markers/tags, not canvas-wide bare `Type(Text)`) → `Off` (N/A, enumerated).
- **Inline** (empty finishing pass, no entries) → `Off` (N/A, enumerated).

StyleLink/Shared are `BareTextOnly` (NOT blanket `Full`) because conditional overrides must remain Brand-only — the same `entry_is_unconditional_bare_text_selector` discriminator the Embedded tier uses (`bb_style_engine.rs:316-401` tests are the contract to mirror).

**Files:**
- Modify: `crates/starbreaker-ui/src/bb_style_engine.rs:113-117` (the `match sheet.tier` route selector) and the doc comment above it (`:107-112`).
- Test: `crates/starbreaker-ui/src/bb_style_engine.rs` `mod tests` (`:150`) — four new tests using the existing harness: `textfield_canvas()` (`:332`), `bare_text_fontsize(f64)` (`:346`), `conditional_text_fontsize(f64)` (`:360`), `label_fontsize(&BbScene) -> Option<f64>` (`:374`); mirror the Embedded pair at `:378`/`:388`.
- Modify: `crates/starbreaker-ui/docs/ui-cascade-passes.md` — the stale "Brand tier only" prose (section `:72-82`) and the table-row parenthetical `:35` ("the TEXT-FORMAT route lives ONLY here").

**Interfaces:**
- Consumes: `apply(scene, &[StyleSheet::uniform(tier, id, &palette, &entries)], None)`, `StyleSheet::uniform` (`:70`), `Tier::{StyleLink, Shared, Brand, Embedded}`, `crate::bb_brand_apply::TextFormatRoute::{Full, BareTextOnly, Off}`.
- Produces: no new public symbols — the `match sheet.tier` gains `Tier::StyleLink | Tier::Shared` on the `BareTextOnly` arm.

- [ ] **Step 1: Write the failing tests** in `mod tests` (bb_style_engine.rs, after the Embedded pair ~`:401`):

```rust
    #[test]
    fn stylelink_tier_routes_unconditional_bare_text_fontsize() {
        let palette = serde_json::json!({});
        let mut scene = parse_bb_canvas(&textfield_canvas()).expect("parses");
        let size = [bare_text_fontsize(100.0)];
        apply(&mut scene, &[StyleSheet::uniform(Tier::StyleLink, "linked_style", &palette, &size)], None);
        assert_eq!(label_fontsize(&scene), Some(100.0),
            "style-link bare Type(Text) FontSize must reach the field's text format");
    }

    #[test]
    fn stylelink_tier_excludes_conditional_text_override_but_brand_keeps_it() {
        let palette = serde_json::json!({});
        let cond = [conditional_text_fontsize(99.0)];
        let mut sl = parse_bb_canvas(&textfield_canvas()).expect("parses");
        apply(&mut sl, &[StyleSheet::uniform(Tier::StyleLink, "linked_style", &palette, &cond)], None);
        assert_eq!(label_fontsize(&sl), None,
            "a CONDITIONAL style-link text entry must NOT take the route (stays brand-only)");
    }

    #[test]
    fn shared_tier_routes_unconditional_bare_text_fontsize() {
        let palette = serde_json::json!({});
        let mut scene = parse_bb_canvas(&textfield_canvas()).expect("parses");
        let size = [bare_text_fontsize(100.0)];
        apply(&mut scene, &[StyleSheet::uniform(Tier::Shared, "mfd_g_content", &palette, &size)], None);
        assert_eq!(label_fontsize(&scene), Some(100.0),
            "shared-style bare Type(Text) FontSize must reach the field's text format");
    }

    #[test]
    fn shared_tier_excludes_conditional_text_override_but_brand_keeps_it() {
        let palette = serde_json::json!({});
        let cond = [conditional_text_fontsize(99.0)];
        let mut sh = parse_bb_canvas(&textfield_canvas()).expect("parses");
        apply(&mut sh, &[StyleSheet::uniform(Tier::Shared, "mfd_g_content", &palette, &cond)], None);
        assert_eq!(label_fontsize(&sh), None,
            "a CONDITIONAL shared text entry must NOT take the route (stays brand-only)");
    }
```

- [ ] **Step 2: Run the tests to verify the positives fail**

Run: `cargo test -p starbreaker-ui -- stylelink_tier shared_tier`
Expected: the two `*_routes_unconditional_bare_text_fontsize` tests FAIL (`assertion left == right failed: left: None, right: Some(100.0)` — route is `Off` at StyleLink/Shared today). The two `*_excludes_conditional_*` tests already PASS (route Off → `None == None`); they are the enduring negative guards.

- [ ] **Step 3: Widen the route** at `bb_style_engine.rs:113-117`:

```rust
    let text_format_route = match sheet.tier {
        Tier::Brand => crate::bb_brand_apply::TextFormatRoute::Full,
        // StyleLink (pass 4) and Shared (pass 5) join Embedded on the
        // bare-only route: an unconditional bare `Type(Text)` FontSize is a
        // canvas-wide text size and must reach the field, but conditional
        // selectors stay brand-tier-only (same discriminator as Embedded).
        Tier::StyleLink | Tier::Shared | Tier::Embedded => {
            crate::bb_brand_apply::TextFormatRoute::BareTextOnly
        }
        // StandardModule (element-instance markers/tags) and Inline (empty
        // finishing pass) carry no canvas-wide bare `Type(Text)` — N/A.
        Tier::StandardModule | Tier::Inline => crate::bb_brand_apply::TextFormatRoute::Off,
    };
```

Update the doc comment above (`:107-112`) to describe the bare-only route spanning StyleLink/Shared/Embedded, Full at Brand. Leave the `sheet.tier == Tier::Brand` (stamp) and `sheet.tier == Tier::Shared` (suppress-intrinsic-fill) bool arguments unchanged.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p starbreaker-ui -- stylelink_tier shared_tier`
Expected: all four PASS. Then the full engine suite for no regression: `cargo test -p starbreaker-ui bb_style_engine` — green (including the unchanged Embedded and brand-stamp tests).

- [ ] **Step 5: Fresh LOD0 export + `--full` sibling audit.** `cargo build --release`, re-export the standard `--full` target(s) with `--lod 0 --mip 0 --materials all`, then `bash scripts/ui_check.sh --full`. This is the single kernel for EVERY cascade pass on EVERY screen, so re-check (vision + snapshot deltas) the siblings that consume StyleLink/Shared styled text fields:
  - power / target / self MFD (consume `mfd_g_*` SHARED styles);
  - medical (shared `mfd_g_*` + `h_hud_g_controlhints_b`);
  - LR-indicator (the ledger-96 embedded case — must stay put);
  - velocity-num & master-mode — verify the general fix does NOT double-apply with the no-brand-match `defaultStyles`-at-Brand path (`bb_resolve/engine_01.rs:1150`).

  Adjudicate every moved target TOWARD its in-game reference. A target that moved toward reference is a DELIBERATE parity improvement (re-freeze only if in scope for this plan, via the audited freeze flow). A target that moved AWAY is a REGRESSION → stop and diagnose; do not re-freeze to pass.

- [ ] **Step 6: Update the stale cascade doc.** In `ui-cascade-passes.md`: rewrite the section heading + body at `:72-82` from "The text-format route (Brand tier only)" to reflect Full at Brand and bare-only at StyleLink/Shared/Embedded (StandardModule/Inline N/A); amend the table-row parenthetical at `:35` ("**the TEXT-FORMAT route lives ONLY here**") so it no longer claims Brand-exclusivity. Verify-on-write any command line touched (none expected here). Note: `/graphify . --update` for the doc change runs at phase end per the global constraint.

- [ ] **Step 7: Commit**

```bash
git add crates/starbreaker-ui/src/bb_style_engine.rs crates/starbreaker-ui/docs/ui-cascade-passes.md
git commit -m "fix(ui): route unconditional bare Type(Text) at StyleLink and Shared tiers (plan B2-3)

Generalizes the text-format route (Full at Brand, bare-only at Embedded) to
the remaining substantive tiers with the same conditional-stays-brand-only
discriminator; StandardModule/Inline N/A. Retires the velocity-num/master-mode
no-brand-match defaultStyles-at-Brand workaround's dependence. Moved targets
adjudicated toward reference. Updates the stale ui-cascade-passes prose."
```

---

## Self-Review record

- **Spec coverage:** parent-plan B2 Step 1 → Task 1 (second band `0xF800_0000`, medical close-X characterization; separator-dots motivation flagged STALE and reframed as latent-stability). Step 2 → Task 2 (component-local bare-only scoping; the `binding.contains('/')` discriminator gets its own test; production already correct per `2538c8743`). Step 3 → Task 3 (route at all remaining tiers, per-tier characterization Full-vs-BareTextOnly, `--full` after fresh export). Step 4 → each task is one commit with its regression guard; none exceeds one-commit scope.
- **Placeholder scan:** every code step shows the actual test/impl code; run commands carry expected FAIL then PASS; the only non-code guidance (Task 3 Step 5 sibling audit, Task 1 Step 7 snapshot check) is verification procedure, not deferred implementation.
- **Type consistency:** `merge_child_scene(..., band_base: Option<BbNodeId>)` used identically in Task 1's test and its three call-site updates; `EXPANSION_ID_BASE` / `EXPANSION_ID_BASE_SECOND` consistent; `TextFormatRoute::{Full, BareTextOnly, Off}`, `Tier::{Brand, StyleLink, Shared, Embedded, StandardModule, Inline}`, and harness fns (`textfield_canvas`, `bare_text_fontsize`, `conditional_text_fontsize`, `label_fontsize`) match their source definitions.
- **Pixel movement:** Tasks 1 and 2 are byte-identical (snapshot / live-IR guards prove it). Task 3 is the only pixel-mover — moved targets adjudicated toward reference, regressions stop-and-diagnose.
