# BLOCKERS — default to fixing; the proof bar; the font drill; shared mechanisms

Read this whenever you are tempted to defer, call something a blocker, or when a
font/size/colour looks wrong.

## Default to fixing, not deferring

"Large blast radius," "touches many frozen nodes," "deserves its own deliberate
change," or any size/risk estimate is NOT a blocker — it is a signal to RESEARCH and
PLAN, then fix within this arc (fan the read-only research across subagents). But
"fixing" means rendering the REAL decoded asset — never inventing geometry or values
for an element the draw path can't natively produce (see the strict rules; ledger
78–83).

**Reproduce before you build a fix.** The owner-reported symptom must reproduce in a FRESH
export first — a non-reproducing symptom (common right after an owner-requested re-export) is a
STALE VIEW, not a bug: present the fresh-export evidence and confirm, don't fix a ghost. Full
rule + example: `references/catalog.md` (ledger 107).

De-risk a wide change EMPIRICALLY: run the disable→adjudicate audit (workflow §5) and
`bash scripts/ui_check.sh --full` (re-export FIRST — `--full` does not re-export;
ledger 56) so the frozen pins MEASURE the real blast radius instead of you estimating
it; find the one structural discriminator; for a genuinely large change, write a short
plan in the arc memory/handoff (affected identities, sequence, the single rule) and
execute it. **Making the change is autonomous — only the resulting baseline FREEZE is
gated** (a wide change that moves many frozen baselines TOWARD the reference is the §5
"baseline is wrong → re-freeze" path; present that delta at the freeze gate).

An inherited "+N regresses frozen X" verdict is itself an ESTIMATE — MEASURE it: if
those +N elements lay out OFF-SCREEN (0×0, no visual change), it's a gated re-freeze of
a CORRECT improvement, not a regression to dodge (the velocity-ball's `base_CPLD`/
`base_ESP` are 0×0 — ledger 98).

## The blocker proof bar (a MAJOR item is USER-gated)

Defer ONLY on a PROVEN concrete blocker — "proven" is a high bar. It means you searched
the WHOLE decodable surface (DataCore records, P4K assets, the localization/text tables
— not just the canvas JSON) AND showed the value isn't DERIVABLE from an already-decoded
mechanism (a screen's content sub-rect/zoom from the screen-mesh/aspect data; a unit
suffix from the engine's enum→localization table).

- **"Not in the canvas data" / "undecoded this arc" is UNDER-RESEARCH, not proof** — fan
  the search across read-only subagents and exhaust it first.
- **"It's engine C++ only / it's in the engine" is NEVER a valid basis** — you can't read
  the C++, so it can never demonstrate absence, and projection / spacing / interval / FOV
  configs are frequently authored in records or assets.
- **Search the record FAMILIES, not just the feature keyword** — a config value lives in a
  `*Params` / `*Global` / `*HudParams` struct (the compass tick range/major/subTicks was
  in `SVehicleHudParams.compassTape`, one `search_records("vehiclehud")` away, while
  `search_records("compass")` returned only UI canvases; ledger 66).
- **A blocker claim MUST carry a VERIFIABLE EVIDENCE TRAIL** — the exact record names and
  grep/probe patterns you ran and the empty results, not a summary assertion. Record it in
  an evidence file and validate it: `python3 scripts/ui_blocker_evidence.py <file.json>`
  must print `ui_blocker_evidence: OK` (all four surfaces — datacore_families, p4k_assets,
  localization, derivable_mechanisms — each carry a real {query, result}). Schema + a worked
  example: `crates/starbreaker-ui/docs/blocker-evidence-schema.md`. If you can't produce a
  file the validator accepts, you haven't proven it.
- **A MAJOR/dominant item is confirmed by the USER, both modes** — before accepting one as
  blocked, dispatch a dedicated "find-it-or-prove-absence" subagent, then surface the
  blocker + the validated evidence file to the user (gate). Never self-certified. (Minor
  residuals deferred with proof don't need the gate.) Before deferring or accepting ANY
  residual, classify it against `crates/starbreaker-ui/docs/ui-residual-budget.md`
  (capture-caused permanent-acceptable vs renderer-caused must-fix; a capture claim needs
  a distinguishing measurement).

## The wrong-stage / whack-a-mole trap

When scoping render-side to dodge a frozen regression just regresses the NEXT frozen
sibling (each narrower scope `auto` → `HC_HUD+auto` → tag-match reveals another), that is
NOT a proven blocker — it is the signal you are at the WRONG STAGE, symptom-scoping a fix
that belongs UPSTREAM.

## The font / size / colour drill — run `ui_variant_styles` FIRST

FOUR arcs (velocity-num, compass, master-mode, LR-indicator) looked like a size/colour
BLOCKER and each DISSOLVED once the INSTANTIATED variant's authored entries were read: the
value was authored in `defaultStyles`/`brandStyles`/`embeddedStyles` and simply was not
being APPLIED. So before surfacing ANY size/colour blocker:

1. **Run the `ui_variant_styles` MCP tool** on the canvas + node — it lists each matched
   node's AUTHORED entries per tier (defaultStyles / manufacturer brand / canvas-root
   embeddedStyles, INCLUDING bare `Type(Text)` selectors) with an `applied` verdict.
   `applied:false` on a matched entry is the authored-but-UNAPPLIED case — the value IS in
   the data, the cascade just isn't reaching the node.
2. When a master TILES per-manufacturer sub-canvases via `CanvasReferenceRecord`, the
   instantiated variant is the BRAND one (`drak_*_cutlass_*`, NOT `generic_*`). The
   canvas-root `embeddedStyles` `Type(Text)` selectors auto-apply to every matching node and
   are the easiest to miss — the LR-indicator's FontSize 100 lived ONLY there (initially
   mis-deferred as "undecoded engine HUD scaling", ledger 94).
3. **TWO non-blocker causes precede any "undecoded" defer, not one.** (a) *Unapplied
   authored size* (above). (b) *Per-screen RENDER scale:* even with the size applied, a
   portrait cockpit screen needs a `ui_ir` font screen scale — and the obvious `bb_layout`
   `canvas_scale` knob is INERT (the IR renderer reads `font_size` directly; a byte-identical
   render via the printed `png md5:` proves you edited a dead stage). Trace the lever to where
   the glyph px is actually READ (a throwaway `#[test]` that `compile_ir_for_binding` on the
   real canvas + a stage-local `eprintln` beats hand-solving the op graph), THEN judge any
   residual (ledger 94/96/97).

Only a genuinely-absent authored value AND an exhausted upstream search is a real blocker.
Still judge auto-canvas (`coordinateMethod=auto`)/HC_HUD experiments with `--full` after a
fresh export — that drift is WHOLE-IMAGE-only, invisible to `ui_check` live-IR.

## Shared-mechanism rules

After a change to a SHARED mechanism, `--full`'s ~1% whole-image budget can MISS a few-px
element regression on SIBLING screens that share it. The measurement scales with the CHANGE
CLASS:

- **Asset / icon / binding change** (SvgPath, icon preset, separator, footer chrome): render
  + EYEBALL every screen sharing it (e.g. all MFD footers), not just the arc's (ledger 77).
- **LAYOUT / RENDER FORMULA change** is the SAME class of shared mechanism (a `bb_layout`
  flex/aspect formula, an `ir_compose` transform): the measurement is the `--full` per-target
  PERCENTAGES across ALL frozen targets after a fresh export — the arc's own unit test can't
  see siblings (dropping a flex cross-axis `pivot.x*w` term passed a fresh test AND the arc
  screen but regressed `clipper_countermeasures_master` 3.87%; only `--full` caught it; the
  discriminator that scoped the carve-out to zero collateral came from DUMPING the sibling's
  nodes — every `pivot.x!=0` node had `anchor.x==pivot.x` while the ✕ uniquely had
  `anchor.x==0`; ledger 106).
- **Colour / token-only change** cannot move geometry, so it is already covered by the
  element-level `..._tint_semantics` gold snapshot (MORE sensitive than `--full` whole-image)
  — no separate sibling render needed (ledger 107).

Place every element from its AUTHORED node geometry through ONE shared transform — a
per-element tuned constant is the smell you're at the wrong stage. To locate a layout bug,
INSTRUMENT the RESOLVED geometry (throwaway `#[test]` + `eprintln`), don't hand-solve the
authored op graph.

## Perf side-questions

"The export got slow" is the `starbreaker-optimisation` skill's job — invoke it rather than
ad-hoc timing. Its first move: pin BOTH baselines' binary provenance
(`target/release/deps/starbreaker-<hash>` mtimes are a no-rebuild bisect ladder) before
attributing anything to this arc's changes.

## Colour compositing — linear-light LANDED (interim leniency RETIRED)

The renderer now composites in LINEAR light (plan B4 landed; all gold/platinum baselines
re-frozen). The former interim rule — judge blend-shaped colour residuals at composited edges
LENIENTLY and park them as §6 known-outliers pointing at B4 — is RETIRED. Colour residuals are
now judged NORMALLY: a genuine blend-shaped drift at a composited/chiclet edge is a latent bug to
root-cause and adjudicate (§5), not an outlier to register. Still resolve colour from the
entry-driven token, never a draw-time hack.

**Measuring a thin-glyph / edge colour:** a MEAN is unreliable — anti-aliasing against a bright
background gives near-identical means for black vs a dark tint. Discriminate by the COUNT of
near-pure-black pixels (`max(r,g,b)<25`) in the isolated region (395→0 caught the chevron fix),
and confirm against the IR token. Uniform-colour screens defeat colour thresholds — use
connected-component region isolation and require the measurement to survive a second, cleaner
method (`references/catalog.md`; ledger 107).

## Red flags — blockers

| Thought | Reality |
|---|---|
| "Just hard-code this one value/offset" | Banned, even in fixtures/fallbacks. Find the structural cause. |
| "Can't render it natively / missing from the IR — I'll reproduce or generate the geometry" | That's the banned invent-magic-numbers pattern, not "the hologram pattern" (which rendered the REAL mesh; only the camera was tuned). Exhaust the in-data art first (`mtl_summary`→texture / `image_preview` / `svgFill.svgPath` / styleTag `SvgPath`/`ImagePath` / SWF) and render the REAL asset; a "missing" node is usually GATED-OFF not absent — activate the real node. Resolve PER-MANUFACTURER (`PrimitiveMaterialPath`, not the generic). Only a parsed-proven runtime-absent value (the camera) is owner-tuned (ledger 78–83). |
| "Render differs from ref, so the render is wrong" | Captures have bloom/skew/resolution/hover artifacts. Compare structurally. |
| "Large blast radius / deserves its own change / undecoded this arc — defer it" | Size/risk/"undecoded" is not a blocker — research then fix. MEASURE with disable→adjudicate + `--full`; for "missing data" search the whole surface (DataCore/P4K/localization) and check it isn't derivable; frozen-family risk = find the §5 discriminator. Defer only on an exhausted-search PROVEN blocker (evidence file). |
| "It's engine C++ only / I exhausted the data — blocked" | Show the trail via `ui_blocker_evidence.py` or it isn't proven: exact records/greps/probes + their empty results. Search the record FAMILIES (`*Params`/`*HudParams`), not the feature keyword (compass ticks were in `SVehicleHudParams`). "Engine C++ only" is unfalsifiable. A MAJOR-item blocker is user-confirmed, never self-certified. |
| "Each narrower scope regresses the next frozen sibling — no-discriminator blocker proven" | NO — that's the signal you're at the WRONG STAGE, symptom-scoping render-side a fix that belongs UPSTREAM. Four arcs "blocked" on size/colour, all dissolved on `ui_variant_styles` reading the INSTANTIATED variant's authored entries (value authored but NOT applied). |
| "Font's too small but there's no authored FontSize — undecoded engine scaling, defer" | TWO non-blocker causes first. (1) Unapplied authored size — run `ui_variant_styles`; re-read the INSTANTIATED brand sub-canvas (`drak_*_cutlass_*`) incl. canvas-root `embeddedStyles` `Type(Text)` (LR-indicator's FontSize 100 lived ONLY there). (2) Per-screen RENDER scale — a portrait screen needs a `ui_ir` font scale; the `bb_layout` `canvas_scale` knob is INERT (byte-identical render proves a dead stage). Apply BOTH, trace the lever, THEN judge (ledger 94/96/97). |
| "I'll grep the line range to check that data claim" | A `sed`/`grep` line-window of a big nested record lands on the wrong entry (serialization order, same-named/conditional entries). Parse the JSON + iterate, or run `ui_variant_styles`/`BB_A3_STYLE_PROBE`/`FONTPROBE`. |
| "I'll grep the source to find which stage/function owns this" | Query graphify first (`graphify query`/`explain`, `/graphify`, `graphify-mcp`) — relationship-aware, `file:line`, no API cost (ui-reference §4b). Code structure only — not a data-value source. |
| "graphify shows nothing / `No path` — so that code doesn't exist" | An empty graphify result is never proof of absence — grep to confirm (`.map(foo)` won't match `foo(`). The old `engine_*.part` blind spot is CLOSED (ledger 105): real `engine_NN.rs` submodules, fully indexed. |
| "My quick check refutes the subagent — move on" | Refuting a careful subagent finding needs the SAME rigour as the claim. If your refutation is the weaker read, IT'S the unreliable one — verify with parse/probe before acting (ledger 68). |
| "Root cause's obvious — land the fix before the catalog gate" | Pre-gate you investigate and write the characterizing failing test, but DON'T land a source fix until the catalog is confirmed. After the gate the loop fixes directly. |
| "`--full` is green, so nothing regressed" | `--full`'s ~1% whole-image budget misses a few-px element drop (e.g. footer nav arrows) on SIBLING screens. After an asset/icon/binding/layout-formula change, EYEBALL every screen sharing the mechanism, not just the arc's (ledger 77). A colour/token-only change is already covered by the element-level tint snapshot (ledger 107). |
