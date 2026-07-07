# B3 Flash/SWF Hybrid — Defensive Overlay-Trigger Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Narrow the full-stage SWF overlay so it can only fire on a genuine full-Flash-stage host (a Flash node whose whole subtree is a non-painting placeholder), never on an ordinary `rendererType:"Flash"` primitive node that has real BB children — defusing the "blank + over-paint real BB content" landmine while staying pixel-neutral.

**Architecture:** The overlay dispatch (`hybrid_compose::render_ui_ir_with_swf_overlay`) currently treats **every** `is_flash_renderer == true` node as a full-stage SWF host: it removes the node's whole BB subtree and stamps the SWF stage over its rect. But per the runbook (ledger 63) `rendererType:"Flash"` is the *normal* primitive renderer for essentially every visible BB node, so `is_flash_renderer` is set on ~every drawable node (1261 nodes true in the Clipper IR dump). The fix adds one structural guard at the single overlay chokepoint: a Flash node is a full-stage host **only if no node in its subtree paints BB content** (candidate discriminator (a): the subtree is a pure, non-drawing placeholder — nothing to blank). Ordinary Flash primitives with real BB children have a drawable descendant → excluded → their BB is kept and no SWF is stamped. `is_flash_renderer`'s broad assignment is left unchanged (its meaning "this node uses the Flash primitive renderer" is correct); only the overlay *consumer* is narrowed.

**Tech Stack:** Rust, `starbreaker-ui` crate, `image`/`tiny_skia` raster compositing, `cargo test`.

## Global Constraints

- **TDD:** Write the failing test first, run it, watch it fail, then implement, then watch it pass. (`AGENTS.md` TDD rule.)
- **No hard-coded game-data / asset / ship / screen gating:** the discriminator is a **category rule** (a structural "does this subtree paint BB content" property), never a name/ship/screen/asset branch. (`AGENTS.md` "Never hard-code workarounds for specific assets.")
- **One commit per task, directly on `feature/ui`.** Never create a new branch/worktree. (Memory: stay-on-feature-ui-branch.)
- **Pixel-neutral:** the change must not alter any exported pixel. Validate with a fresh **release** rebuild + LOD0 export (`--lod 0 --mip 0 --materials all`) and `scripts/ui_check.sh --full` **ALL GREEN**. Neutrality is guaranteed by construction — see the empirical dormancy finding below — but the gate must still be run.
- **Do NOT run `graphify . --update`** — the post-commit hook re-extracts changed code automatically.
- **Every new Rust source line touched keeps its `//!` / `///` docs accurate** — update the `hybrid_compose.rs` module header and fn doc to describe the narrowed rule. (`AGENTS.md` module-doc rule.)

## Empirical grounding (read before starting — this is why the change is safe)

Read-only IR dumps (`starbreaker ui render --dump-ir-dir`, release binary) were taken over the fresh LOD0 exports of both target ships:

| Ship | Canvases | `renderer_hint` | Resolve a SWF | Take the overlay (`Swf`/`Hybrid`) |
| --- | --- | --- | --- | --- |
| DRAK Clipper | 27 | **all `bb`** | 2 (`Screen_Annunciator_L/R` → `AnnunciatorHalve1/2.swf`) | **0** |
| ANVL Carrack | 14 | **all `bb`** | 0 | **0** |

The overlay path (`render_ui_ir_with_swf_overlay`) **never dispatches** today: dispatch is gated on `renderer_hint ∈ {Swf, Hybrid}` (`pipeline/mod.rs:765-773`), which requires `has_selected_swf_source && has_custom_shape` (`ui_ir/engine_01.rs:684-688`). No current screen meets both. The two Clipper annunciators are the **closest miss**: they resolve a real SWF but stay `bb` because they have no `WidgetCustomShape` node. Therefore any narrowing of `flash_ids` is **pixel-neutral by construction** (it can only ever remove nodes from an overlay set that no exported screen currently reaches), and the fix is purely defensive hardening against a future where SWF resolution becomes more robust.

## File structure

- **Modify** `crates/starbreaker-ui/src/hybrid_compose.rs` — narrow the `flash_ids` filter; add the `is_full_stage_swf_host` guard; update `//!` header + fn doc.
- **Modify** `crates/starbreaker-ui/src/ui_ir/engine_01.rs` — add a `UiIrNode::paints_bb_content(&self) -> bool` inherent method next to the struct (`is_flash_renderer` setter at `:1647` and struct field at `:177` are **left unchanged**).
- **Modify** `crates/starbreaker-ui/tests/swf_phase5_wiring.rs` — invert Test 4 into the characterization (landmine) test; keep Test 3 as the positive host test; add one host-with-inactive-child positive test.
- **Modify** `crates/starbreaker-ui/docs/ui-fallback-register.md` — record the decision + dormancy finding + narrowed contract (folded into the same commit; not a separate task).

---

### Task 1: Narrow the overlay trigger to genuine full-stage hosts

**Files:**
- Modify: `crates/starbreaker-ui/src/ui_ir/engine_01.rs` (add `UiIrNode::paints_bb_content`; struct is defined here, `is_flash_renderer` field at `:177`)
- Modify: `crates/starbreaker-ui/src/hybrid_compose.rs:36-41` (flash_ids filter), `:1-6` + `:21-29` (docs), plus new private fn near `collect_subtree_ids:80-97`
- Test: `crates/starbreaker-ui/tests/swf_phase5_wiring.rs:297-333` (Test 4, inverted) and `:263-293` (Test 3, kept)
- Doc: `crates/starbreaker-ui/docs/ui-fallback-register.md`

**Interfaces:**
- Consumes: existing `UiIrDocument`, `UiIrNode` (`ui_ir/engine_01.rs:177`), `collect_subtree_ids(document, roots) -> HashSet<u32>` (`hybrid_compose.rs:80`).
- Produces:
  - `UiIrNode::paints_bb_content(&self) -> bool` — `true` iff the node is active and carries any BB paint-bearing attribute.
  - `fn is_full_stage_swf_host(document: &UiIrDocument, root_id: u32) -> bool` (private in `hybrid_compose.rs`) — `true` iff no node in `root_id`'s subtree (root ∪ descendants) returns `paints_bb_content() == true`.

- [ ] **Step 1: Write the failing characterization test (invert Test 4).**

In `crates/starbreaker-ui/tests/swf_phase5_wiring.rs`, replace the existing Test 4 (`hybrid_render_suppresses_bb_subtree_of_flash_node`, lines 297-333) with the corrected-contract test. The document is unchanged (a Flash parent id=1 with a real red-fill BB child id=2); only the assertion flips — the BB child MUST now survive:

```rust
// ── Test 4: real BB children of a Flash node are NOT blanked (landmine defused) ──

#[test]
fn hybrid_render_keeps_bb_subtree_of_flash_node_with_real_children() {
    // A `rendererType:"Flash"` node with a real drawable BB child is an ORDINARY
    // primitive node, NOT a full-stage SWF host. The overlay must not remove its
    // subtree or stamp the SWF stage over it. The red child must render.
    let assets = SwfAssetLibrary::new(vec![
        b'F', b'W', b'S', 6, 21, 0, 0, 0,
        0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ])
    .expect("minimal swf");
    let style = black_style();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets, hologram_fetcher: None };
    let atlas = AtlasLibrary::new(&EmptyFetcher, None);

    // Flash parent (id=1), active child with red fill (id=2).
    let red_fill = Some([1.0f32, 0.0, 0.0, 1.0]);
    let document = make_document(
        100,
        100,
        Some("test.swf"),
        vec![
            minimal_node(1, None, vec![2], true, (0.0, 0.0, 100.0, 100.0), None),
            minimal_node(2, Some(1), vec![], false, (0.0, 0.0, 100.0, 100.0), red_fill),
        ],
    );

    let result = render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, &|_| None)
        .expect("render failed");

    let red_pixels = result.pixels().filter(|p| p[0] > 200 && p[1] < 50 && p[2] < 50).count();
    assert!(
        red_pixels > 0,
        "Flash node with a real BB child must keep its subtree — found {red_pixels} red pixels",
    );
}
```

- [ ] **Step 2: Add the host-with-inactive-child positive test.**

Append to `crates/starbreaker-ui/tests/swf_phase5_wiring.rs`. This locks the `is_active` half of the predicate — an *inactive* (non-drawing) child does not disqualify a placeholder host, so the SWF still composites:

```rust
// ── Test 6: an inactive/non-drawing child does not disqualify a full-stage host ──

#[test]
fn hybrid_render_composites_swf_for_placeholder_host_with_inactive_child() {
    // SWF renders white "A" text; the Flash node's only descendant is INACTIVE
    // (paints nothing), so the subtree is still a pure placeholder → host → overlay.
    let swf_bytes = swf_helpers::make_edit_text_with_font_swf();
    let assets = SwfAssetLibrary::new(swf_bytes).expect("SwfAssetLibrary");
    let style = black_style();
    let defaults = DefaultValueRegistry::with_well_known_path_defaults();
    let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets, hologram_fetcher: None };
    let atlas = AtlasLibrary::new(&EmptyFetcher, None);

    // Flash parent (id=1); child id=2 has a red fill but is INACTIVE → paints nothing.
    let mut inactive_child = minimal_node(2, Some(1), vec![], false, (0.0, 0.0, 100.0, 100.0), Some([1.0, 0.0, 0.0, 1.0]));
    inactive_child.is_active = false;
    let document = make_document(
        100,
        100,
        Some("test.swf"),
        vec![
            minimal_node(1, None, vec![2], true, (0.0, 0.0, 100.0, 100.0), None),
            inactive_child,
        ],
    );

    let result = render_ui_ir_with_swf_overlay(&document, &ctx, &atlas, &|_| None)
        .expect("render failed");

    let non_black = result.pixels().filter(|p| p[0] > 10 || p[1] > 10 || p[2] > 10).count();
    assert!(non_black > 0, "placeholder host with an inactive child must still composite the SWF");
    let red_pixels = result.pixels().filter(|p| p[0] > 200 && p[1] < 50 && p[2] < 50).count();
    assert_eq!(red_pixels, 0, "inactive child must not paint");
}
```

- [ ] **Step 3: Run the two new tests to verify they fail.**

Run:
```bash
cargo test -p starbreaker-ui --test swf_phase5_wiring \
  hybrid_render_keeps_bb_subtree_of_flash_node_with_real_children \
  hybrid_render_composites_swf_for_placeholder_host_with_inactive_child -- --nocapture
```
Expected: **FAIL.** With the current code every `is_flash_renderer` node is treated as a host, so the red child in Test 4 is blanked (`red_pixels == 0`, assertion `> 0` fails), and the inactive-child host test may also mis-suppress. This proves the landmine is real and the tests are genuine regressions.

- [ ] **Step 4: Add `UiIrNode::paints_bb_content`.**

In `crates/starbreaker-ui/src/ui_ir/engine_01.rs`, add an inherent method to `UiIrNode` (place it in/adjacent to the existing `impl UiIrNode` block; if none exists, add one directly after the struct definition that ends near line 177). Use only fields confirmed present on `UiIrNode` (see the `minimal_node` constructor in the test file for the field list):

```rust
impl UiIrNode {
    /// True iff this node would paint real BB content (so removing it from the
    /// BB pass and stamping a SWF stage over it would blank something visible).
    ///
    /// Used to distinguish a genuine full-stage SWF-host placeholder (a Flash
    /// node whose whole subtree paints nothing) from an ordinary
    /// `rendererType:"Flash"` primitive node that draws real BB content.
    ///
    // ponytail: this OR-list is the drawable-attribute set as of the current
    // UiIrNode. Ceiling: a NEW paint-bearing field added to UiIrNode but not
    // listed here would read as "paints nothing" and could be blanked by the
    // overlay. Keep this list in sync when adding drawable node attributes.
    pub fn paints_bb_content(&self) -> bool {
        self.is_active
            && (self.background_fill_colour.is_some()
                || self.background_fill_colour_token.is_some()
                || self.circle_fill_colour_token.is_some()
                || self.segmented_fill.is_some()
                || self.polygon.is_some()
                || self.border.is_some()
                || self.stroke_colour.is_some()
                || self.stroke_colour_token.is_some()
                || self.separator_strip.is_some()
                || self.icon_preset.is_some()
                || self.text_payload.is_some()
                || self.secondary_text_payload.is_some()
                || self.meter_progress.is_some()
                || self.asset_ref.is_some()
                || self.custom_shape.is_some()
                || self.primitive_material.is_some())
    }
}
```

- [ ] **Step 5: Narrow the overlay `flash_ids` filter and add the host guard.**

In `crates/starbreaker-ui/src/hybrid_compose.rs`, change the filter at lines 36-41 so only genuine full-stage hosts are collected:

```rust
    let flash_ids: Vec<u32> = document
        .nodes
        .iter()
        .filter(|n| n.is_flash_renderer && is_full_stage_swf_host(document, n.id))
        .map(|n| n.id)
        .collect();
```

Add the guard fn next to `collect_subtree_ids` (after line 97). It reuses the existing subtree walk and the new predicate:

```rust
/// A `rendererType:"Flash"` node is a genuine full-stage SWF host only when its
/// entire BB subtree (the node and all recursive children) paints nothing — a
/// pure placeholder whose visual is meant to come from the SWF stage. If any
/// node in the subtree paints real BB content, the Flash node is an ordinary
/// primitive renderer and must keep its BB subtree (the overlay must not fire on
/// it). Safe default for a populated subtree: NOT a host.
fn is_full_stage_swf_host(document: &UiIrDocument, root_id: u32) -> bool {
    let subtree = collect_subtree_ids(document, &[root_id]);
    !document
        .nodes
        .iter()
        .any(|n| subtree.contains(&n.id) && n.paints_bb_content())
}
```

- [ ] **Step 6: Update the module + fn documentation.**

In `crates/starbreaker-ui/src/hybrid_compose.rs`, replace the `//!` header (lines 1-6) and the `render_ui_ir_with_swf_overlay` doc (lines 21-29) so they describe the narrowed contract. New `//!` header:

```rust
//! Hybrid UI IR renderer that composes IR-backed BB content with SWF overlays.
//!
//! `render_ui_ir_with_swf_overlay` renders the IR document with Phase 5 Flash-wins
//! precedence, but ONLY for genuine full-stage SWF hosts: a `is_flash_renderer`
//! node whose entire BB subtree paints nothing (a pure placeholder — see
//! `is_full_stage_swf_host`). Such a node suppresses its (empty) BB subtree and
//! composites the SWF stage at its `computed_rect`. An ordinary
//! `rendererType:"Flash"` primitive node with real BB children keeps its BB
//! subtree and is never over-painted. Nodes without `is_flash_renderer` render
//! via the normal BB path unchanged.
```

Update the fn doc comment (lines 21-29) to match — change "For each node with `is_flash_renderer == true`" to "For each genuine full-stage SWF host (see `is_full_stage_swf_host`)".

- [ ] **Step 7: Run the overlay test module to verify the new + kept tests pass.**

Run:
```bash
cargo test -p starbreaker-ui --test swf_phase5_wiring -- --nocapture
```
Expected: **PASS** — all five original tests plus the two new ones. Specifically: `hybrid_render_keeps_bb_subtree_of_flash_node_with_real_children` now finds red pixels (`> 0`); `hybrid_render_composites_swf_for_flash_node` (Test 3, lone Flash node with no children → still a host) still composites; `hybrid_render_composites_swf_for_placeholder_host_with_inactive_child` composites the SWF with no red leak; `non_flash_node_still_renders_bb` unchanged.

- [ ] **Step 8: Run the full crate test suite (regression check).**

Run:
```bash
cargo test -p starbreaker-ui
```
Expected: **PASS** — no regressions. The module unit test in `hybrid_compose.rs` (`hybrid_rendering_does_not_require_separate_swf_assets_parameter`, empty node list) is unaffected (empty `flash_ids`, early return).

- [ ] **Step 9: Record the decision in the fallback register.**

In `crates/starbreaker-ui/docs/ui-fallback-register.md`, under "Active Fallbacks" (or a short "Notes" line adjacent to it), add a row/note capturing the B3 outcome (no invented values — this is a decision record):

```markdown
| `crates/starbreaker-ui/src/hybrid_compose.rs` (`is_full_stage_swf_host`) | Full-stage SWF overlay fires ONLY on a Flash node whose entire subtree paints no BB content (pure placeholder); ordinary `rendererType:"Flash"` primitives with real BB children keep their BB subtree | UI hybrid compose | Every `is_flash_renderer` node considered for the overlay | Overlay is empirically DORMANT — 2026-07-07 IR dumps: DRAK Clipper 27/27 canvases `renderer_hint:bb` (annunciators resolve a SWF but stay `bb`, no `WidgetCustomShape`), ANVL Carrack 14/14 `bb`; 0 screens take `Swf`/`Hybrid` | Defensive guard; the three named B3 hybrid gaps (Furore font, MFD footer, 4:3-vs-16:9) are already carried by the BB layer (see `.superpowers/sdd/B3-research.md`). Retire the overlay entirely only if BB is confirmed to carry every full-stage case. |
```

- [ ] **Step 10: Pixel-neutrality gate — fresh release LOD0 export + `ui_check.sh --full`.**

Rebuild release and re-export both target ships, then run the full UI battery:
```bash
cargo build --release -p starbreaker
SC_DATA_P4K="$HOME/Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k" \
  ./target/release/starbreaker entity export "DRAK Clipper" "$HOME/projects/scorg_tools/ships" \
  --kind decomposed --lod 0 --mip 0 --materials all
scripts/ui_check.sh --full
```
Expected: **ALL GREEN**, and the frozen Clipper/Carrack baselines (target + power footers, annunciators, HUD gauges) **byte-unchanged** — consistent with the dormancy finding (the overlay never fired, so no exported pixel can move). If any baseline changes, STOP: the neutrality assumption is violated and the discriminator needs review before committing.

- [ ] **Step 11: Commit (one commit, on `feature/ui`).**

```bash
git add crates/starbreaker-ui/src/hybrid_compose.rs \
        crates/starbreaker-ui/src/ui_ir/engine_01.rs \
        crates/starbreaker-ui/tests/swf_phase5_wiring.rs \
        crates/starbreaker-ui/docs/ui-fallback-register.md
git commit -m "B3: narrow SWF overlay to genuine full-stage hosts (defuse Flash-primitive blanking landmine)

is_flash_renderer is set on ~every visible BB node (rendererType:Flash = the
normal primitive renderer, runbook ledger 63), yet the overlay treated each as
a full-stage SWF host and removed its BB subtree. Guard the overlay so it only
fires on a Flash node whose whole subtree paints no BB content; ordinary Flash
primitives with real BB children keep their BB. Pixel-neutral: the overlay is
dormant on all current screens (Clipper 27/27 + Carrack 14/14 renderer_hint:bb)."
```

## Self-Review

- **Spec coverage:** Q1 (dormancy) → answered empirically, drives the pixel-neutrality argument (grounding section + Step 10). Q2 (structural discriminator) → candidate (a), `is_full_stage_swf_host` via `paints_bb_content` (Steps 4-5). Q3 (characterization test) → Step 1 (negative/landmine) + Step 2 (positive host) + kept Test 3 (positive), in `tests/swf_phase5_wiring.rs`. Q4 (pixel gate) → Global Constraints + Step 10. Category-rule-only, no name gating → predicate is a structural property; documented in Global Constraints.
- **Placeholder scan:** none — every step has concrete code/commands and expected output.
- **Type consistency:** `paints_bb_content(&self) -> bool` (defined Step 4) is called by `is_full_stage_swf_host` (Step 5), called by the `flash_ids` filter (Step 5). `collect_subtree_ids` reused with its existing `(&UiIrDocument, &[u32]) -> HashSet<u32>` signature. Field names in `paints_bb_content` match the `UiIrNode` fields in `minimal_node` (`swf_phase5_wiring.rs:52-110`).
