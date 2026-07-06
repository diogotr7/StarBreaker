# UI parity reference — commands, tools, data

The lookup half of the UI workflow (`crates/starbreaker-ui/docs/ui-workflow.md` is the process).
Every command here was executed during writing (2026-06-11). Paths marked
(ws) are workspace-specific (this machine); the repo-relative ones are
universal.

## 1. Build & test

```bash
cargo build                      # debug — fast, fine for ALL iteration incl. renders
cargo build --release -p starbreaker   # only for deploy / canonical exports

bash scripts/ui_check.sh         # TDD tier: examples compile + the WHOLE ui suite
                                 #   (lib + every integration target; the two
                                 #   export-coupled visual guards skipped)
bash scripts/ui_check.sh --full  # boundary tier: + those export-coupled visual
                                 #   guards, freeze+artifact validators, 3d lib,
                                 #   font harness
cargo test --workspace           # everything (slow)
```
**`--full` does NOT re-export.** The export-coupled visual guards compare
`ships/Data/UI/Generated/*.png`, which only refresh on a full export — run one
FIRST or `--full` fails the staleness guard (`STALE EXPORT: Generated PNGs
predate the current build`), which looks like breakage but is the P0.2 guard
working. Re-export with the canonical guard command (§2) before `--full`:
```bash
cargo build --release -p starbreaker && \
./target/release/starbreaker entity export drak_clipper "$HOME/projects/scorg_tools/ships" \
  --kind decomposed --lod 0 --mip 0 --materials all
```
`--lod 0` is REQUIRED (LOD1 culls the cockpit HUD screens, §5).

**Buffered output (ledger 65):** when `ui_check.sh` / `--full` / `entity export`
output is REDIRECTED to a file (e.g. run in the background), the battery flushes
only at process EXIT — the file stays 0 bytes for minutes. Don't read that silence
as a hang/stall: wait on the process exit (the background-task completion
notification, or an `until <exit-status>; do sleep N; done` loop), not on
incremental lines in the file.

Individual suites for targeted debugging:
`cargo test -p starbreaker-ui --lib [filter]`, `--test manifest_live_ir_guard`,
`--test line_count_guard`, `--test manifest_snapshot_regression`,
`--test manifest_visual_regression`. Hardcoding guard:
`bash scripts/check_ui_hardcoding.sh`.

**graphify code graph (§4b) stays fresh automatically.** A post-commit git hook
(`graphify hook install`, already installed) re-extracts changed CODE files and
rebuilds `graphify-out/graph.json` after every commit — so the code-navigation
graph tracks source as you commit. It is AST-only (no API cost) and ignores
doc/image changes; for those run `/graphify . --update` in an assistant. If you
need the graph current mid-arc before committing, `graphify update .` by hand.

## 2. Render & export

**Replay (iteration, ~1 min, debug binary is fine).** Prefer the wrapper —
it always rebuilds first and prints the binary mtime (a background shell's
cwd reset twice caused renders from a STALE binary that looked like a fix
had no effect):
```bash
bash scripts/ui_render.sh --helper Screen_Annunciator_L [--screen <screen_id>] \
  [--ir] [--lod 0|1] [--scene <scene.json>] [--out <dir>]
```
The scene comes from the screen dossier
(`crates/starbreaker-ui/data/ui_screen_dossier_v1.json`, §3's machine mirror):
`--scene` wins; else the helper — or `--screen <screen_id>`, the unique key for
the medical/door rows whose helper is the generic "usable screen" — resolves to
its `scene_package` + `lod`. So a non-Clipper ship (e.g. the Carrack lift-call
console) resolves its OWN scene instead of silently falling back to a Clipper
one, the old case-list bug. An unknown helper with no dossier row and no
`--scene` is a hard error naming the dossier. The power screen needs no
`--scene`: `ui_render.sh --helper Screen_Left_Lower_RTT --ir`. Each run writes a
fresh `/tmp/ui_render/<helper>/<UTC-stamp>/`; `/tmp/ui_render/<helper>/latest`
symlinks the most recent (consecutive renders never cache-collide on a fixed
path, so `--out` is rarely needed).
Raw form (when the wrapper's defaults don't fit):
```bash
./target/debug/starbreaker ui render \
  --scene "$HOME/projects/scorg_tools/ships/Packages/DRAK Clipper_LOD0_TEX0/scene.json" \
  --out-dir /tmp/ui_replay \
  [--helper Screen_Left_Lower_RTT]   # filter to one screen
  [--dump-ir-dir /tmp/ui_replay/ir]  # write composed IR JSON per helper
  [--mip 2]                          # smaller output
```
Replay renders every `ui_bindings` entry through live DataCore/P4K fetchers
and derives ship values from the scene's root entity — replay == export.
P4K location is auto-detected; do NOT set `SC_DATA_P4K` unless pointing at
non-default data, and never set `RAYON_NUM_THREADS=1` except when
benchmarking.

**Standalone canvas render** (no ship scene/export needed — ledger 101; born
on the Carrack lift-call arc for a screen with no Clipper-scene helper):
```bash
bash scripts/ui_render_canvas.sh --canvas <guid> --entity <EntityClassName> \
    [--kind physical|mfd|radar] [--helper <name>] [--out <dir>] [--ir]
```
Generates a minimal single-binding scene.json and replays it; prints the PNG
md5 (ledger 69). `--entity` picks the manufacturer/style + ship values (e.g.
`ANVL_Carrack`).

**Structural record / kit-sheet / tag queries** (parse-by-structure, never
line-grep a nested record — ledger 68/101/103):
```bash
python3 scripts/ui_canvas_query.py node <record.json> <node-name> [--raw]
python3 scripts/ui_canvas_query.py entries <record.json> [--filter SUBSTR]
python3 scripts/ui_canvas_query.py tag <uuid-or-name-substring>
```
Record paths resolve relative to `ships/dcb_canvas/libs/foundry/records`;
`entries` resolves condition tag UUIDs to names via the tag database (so a
kit sheet's `RootFilled…` conditions read as `Tag(text-element-instance)`,
not bare UUIDs).

**Full export (canonical PNGs under `ships/Data/UI/Generated/...`, ~50s
as of 2026-06-12 — cheap enough to re-export before ANY artifact
comparison; the `Generated` PNGs are only as fresh as the last export and
a stale comparison mis-adjudicated a real regression as "zero drift" on
2026-06-12):**
```bash
./target/release/starbreaker entity export drak_clipper \
  ~/projects/scorg_tools/ships --kind decomposed
```
Required before artifact freezes. The generated PNGs are written near the
END of the run — never diff/freeze until the process exits.

**Freeze tooling** (flows in `crates/starbreaker-ui/docs/ui-workflow.md` §7). The whole artifact
cycle (release build → export → stale-comparison cleanup → freeze → both
validators → full battery) is one command:
```bash
bash scripts/ui_freeze_cycle.sh --approver <name> --reason "..." [--skip-export]
```
Individual steps:
```bash
bash scripts/add_ui_regression_target.sh --id <id> --tier <gold|platinum> \
  --source-generated-png ships/Data/UI/Generated/ship/<mfr>/<Ship>/<file>.png
bash scripts/freeze_ui_snapshot_ir.sh --approver <name> --reason "..."
bash scripts/validate_ui_snapshot_freeze.sh
bash scripts/generate_ui_regression_artifacts.sh
bash scripts/freeze_ui_regression_artifacts.sh --approver <name> --reason "..."
bash scripts/validate_ui_regression_artifacts.sh --quick
bash scripts/validate_ui_regression_repo_only.sh   # hosted CI (no game data)
```

**Registering a NEW gold/platinum target** (ledger 44 — `clipper_power_master`
was the first new gold in a while and the ordering bit). `ui_freeze_cycle.sh`
alone is INSUFFICIENT for a new id: it freezes the image artifact but omits the
IR snapshot freeze, so its validator then hard-fails `snapshot freeze ids do not
match manifest ids`. Do, in order:
```bash
bash scripts/add_ui_regression_target.sh --id <id> --tier <gold|platinum> \
  --source-generated-png ships/Data/UI/Generated/ship/drak/Clipper/<canvas>.png
# bump the hard-coded count in tests/manifest_visual_regression.rs
#   (manifest_contains_expected_visual_targets: targets.len() == N) and add an
#   `any(id == "<id>")` assert.
bash scripts/freeze_ui_snapshot_ir.sh --approver owner --reason "…"   # reads a delta
bash scripts/ui_freeze_cycle.sh --approver owner --reason "…"          # image artifacts
bash scripts/validate_ui_snapshot_freeze.sh                            # expect "N target(s)"
```
The generated source PNG is `buildingblocks_canvas_<canvasname>.png` (e.g.
`…_mc_s_power_master.png`), written by the FULL `entity export`, not `ui render`.

## 3. Comparison & screen dossier

**The loop cycle is ONE command** (plan A3):
```bash
bash scripts/ui_arc_status.sh --screen <screen_id> [--no-render]
```
It renders the screen (or reuses the latest render with `--no-render`), compares
it to the dossier's reference over the dossier's preset, and prints one line per
region flagged `CHANGED` / `NEW` / `same` vs the previous cycle (state in
`/tmp/ui_arc_status/<screen_id>/{cur,prev}.json`). **Vision-read ONLY the crops
it lists for flagged regions** — not the whole screen every cycle. Under the
hood it runs `ui_compare.py … --json` (region stats) + `ui_region_summary.py`
(flags a region when any render mean channel moves > 1.0 or any ratio > 0.01);
bank measurements annotate their region where present. A screen with no preset
fails loudly (add one first). Marker: `ui_arc_status: OK (N regions, M changed)`.

The `ui_compare.py` form below is the underlying tool (and the way to crop
ad-hoc `--box` regions or a screen with no dossier preset):
```bash
python3 scripts/ui_compare.py <render.png> <reference.png> \
  --regions <preset> --out-dir /tmp/ui_compare [--stats] [--json <path>]
python3 scripts/ui_compare.py --regions list   # available presets
python3 scripts/ui_compare.py <render> <ref> --box x0,y0,x1,y1 [--box …]  # ad-hoc region(s), no preset
```
`--stats` prints per-region bright/dark pixel means + R-normalised ratios
for both images — the photometric review method. Judge HUE from the ratios,
never raw values: every reference capture has its own cast (G/B attenuate)
and bloom lifts B near bright elements. Before judging an unknown colour,
measure a known anchor on the SAME capture (footer text = Base, pip slabs =
Bright); a colour matches a palette slot when the RATIOS line up under that
cast. This method identified the linear-light compositing gap and the
MissionObjectives icon slot. The refined additive-haze form (measured_ratio
≈ true_ratio + haze_offset, solved from the anchor) is implemented in
`scripts/ui_measure.py` (`--anchor` / `--anchor-rgb`; model documented in
its docstring), which also measures glyph cap heights with
edge-contamination flagging — prefer it over ad-hoc pixel maths.
The reference is auto-scaled to the render width before cropping; READ the
emitted `cmp_*.png` files with vision. Reference screenshots are imperfect
(skew/bloom/capture artifacts; power-screen pip outlines are mouse-hover
artifacts) — compare structurally. Skew is correctable: store the capture's
four screen-corner pixel coordinates as `<reference>.corners.json`
(`{"tl":[x,y],"tr":..,"br":..,"bl":..}` — in GIMP, hover each screen bezel
corner and read the pointer coordinates off the status bar) and
`ui_compare.py` homography-rectifies the capture onto the render rectangle
automatically, printing "rectified via <file>" (or pass `--rectify
<corners.json>` explicitly). **Thin-feature COLOUR caveat (ledger 35):** the
warp interpolates a few-px-wide feature (a bar/stroke/dotted separator) with
whatever sits behind it, smearing its hue toward the background — a 2px Accent1
header bar measured G/R 0.64 ≈ Base on the *rectified* power reference and a real
colour bug was nearly closed as "faithful". Rectify for POSITION; judge a thin
feature's COLOUR on the CRISP ORIGINAL (`ui_measure.py` warns when
`feature_width ≤ 4`).

**Screen dossier** — one row per known screen (extend as screens are
worked). References live in
`~/projects/scorg_tools/reference/in-game/Clipper/` (ws); generated
PNGs in `ships/Data/UI/Generated/ship/drak/Clipper/` named
`buildingblocks_canvas_<canvas>.png`; scenes in
`~/projects/scorg_tools/ships/Packages/` (ws). **Reference variant:** when a
screen has several captures, prefer the straight-on one that carries a
`<name>.corners.json` sidecar (`ui_compare` auto-rectifies it) over the
dossier's legacy name — e.g. the power screen uses `Screen_Left_Lower_RTT_dark.png`
(rectifiable), NOT the un-cornered `Screen_Left_Lower_RTT.png`. (Then heed the
thin-feature colour caveat above: rectify for position, measure thin colour on
the original.)

| Screen | Helper / scene | Canvas | Reference image | Preset | Tier / target id | Open issues |
|---|---|---|---|---|---|---|
| Power MFD | `Screen_Left_Lower_RTT` / LOD0 scene | `MC_S_Power_Master` (via `M_MFD_Screen` frame) | `Screen_Left_Lower_RTT.png` | `power` | **GOLD `clipper_power_master`** (frozen 2026-06-14) | card width + battery icon DONE 2026-06-14 (data-driven AspectRatioToTag→"Content Canvas Scaling", `pipeline/aspect_tag.rs`; cards ~437, icon ~67px); open: P3 separator dots, P4 pip brightness, P13 header side bars, P7 slider width (plan P2.2b), P8 letter pitch (`crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md`) |
| Target MFD | `Screen_Right_Upper_RTT` / LOD0 scene | `MC_S_Target_Master` | `Screen_Right_Upper_RTT.png` | `target` | GOLD `clipper_target_master` | A7 backdrop stack remainder (handoff) |
| Medical bed | usable screen / LOD1 scene | `I_Med_MedicalBed_A` | `screen_16x9_a-[medical1].png` | — (add) | PLATINUM `ui_target_a` | handoff steps 10–13 |
| Medical end-of-bed | usable screen / LOD1 scene | `I_Med_MedicalEndOfBed_A` | `mesh_end_screen_plane-[medical2].png` | — (add) | PLATINUM `ui_target_b` | logo −12px check (handoff) |
| Small door | usable screen / LOD1 scene | `I_Door_Small_DRAK` | `Door-closed.png` | `door` | GOLD `clipper_small_door` (re-frozen 2026-06-14, 1920×1132) | render aspect 1.70 from the `rtt_screen` mesh (was the 16:9 canvas) |
| Annunciator L | `Screen_Annunciator_L` / LOD0 cockpit | `H_Eng_Annunciator_Master_Left` | `Screen_Annunciator_L.png` (resized to match) | `annunciator` | **PLATINUM `eng_annunciator_master_left`** (re-frozen 2026-06-14, 1920×344) | aspect 5.58 from the screen quad (was 4.44 SWF content-crop) |
| Annunciator R | `Screen_Annunciator_R` / LOD0 cockpit | `H_Eng_Annunciator_Master_Right` | (mirror of L — no separate capture) | `annunciator` | **PLATINUM `eng_annunciator_master_right`** (onboarded + frozen 2026-06-14, 1920×344) | aspect 5.58 (mirror of L) |
| G-force ball | `Screen_Small_Radar2` / LOD0 cockpit | `HC_HUD_Ship_G_Force_Ball_Master` | `g_force_ball_master.png` | `target` | **PLATINUM `clipper_g_force_ball_master`** (frozen 2026-06-15) | PARITY PASS 2026-06-15 (owner-approved platinum): diagram centred (cover-fit recentre clamp relaxed + scoped to overflowing axes only, so aspectOverrides door/annunciator are untouched), cross SQUARE (flex `shrinkProportion=0` now honoured → `card_BallArea` stays square, readouts overflow/crop), centre dot now a true CIRCLE (`rounded_rect_path` cubic arcs for full ellipses), cardinal+diagonal markers symmetric at the axis ends, at-rest LIMIT "0" hidden (`AccelerationLimiterRatio=1.0` registry pin → `base_LIMIT Text` IsActive false), right-edge readout cropped off-screen. Earlier history: aspect 1.0 square; blank-at-rest FIXED 2026-06-14 (`bb_state_filter` sole-root `Instantiated` exemption — class-wide, also unblanked velocity ball/countermeasures). Correct content = DRAK ADV ball (G1a generic-vs-drak angle FALSIFIED+reverted). C1/G1b square-gauge centring LANDED 2026-06-14 (owner-approved): `cover_fit` (uniform max-scale) + `cover_fit_recentre` (clamp-centre on the largest LOCALISED node = the ball-area) in `bb_layout`, gated to the `screen_aspect_w_over_h` `useRaw` branch — g-force (ball-left) + velocity (ball-right, mirrored) both centre & fill, readouts overflow/crop; ui_check GREEN, frozen screens untouched. C2 = bezel geometry (square RT correct, no change). C3 spoke colour cyan→orange LANDED 2026-06-15 (`bb_svg::parse_uniform_colorstyle` + `ir_compose::svg_colorstyle_fill_override`: resolve the `colorstyle:<role>`/`opacity:` SVG path-id convention → brand surface palette × opacity, uniform-role only; the literal `#6CB8C7` is an Illustrator placeholder). Generic across HUD glyph SVGs; 0.0000% diff on all frozen targets. C4 solid markers LANDED 2026-06-15: WidgetCircle `doFill`+`fillColor` was unhandled (`draw_widget_circle` only stroked); now `ui_ir` captures `circle_fill_colour_token` (doFill-gated) and `ir_compose` fills via the brand surface palette — the 2 solid diagonal dots render (frozen targets 0.0000% diff). The 4 cardinal markers are circular RINGS (correct; owner clarified the ref markers are circles). **C6 white centre dot LANDED 2026-06-15** (`ui_ir::is_untinted_overlay_render_shape`→white overlay-identity fill + `ir_compose::fill_rounded_rect_ts_with_mode`; shared with velocity V2b). **Cardinal-marker rotation+positioning LANDED 2026-06-15** via the velocity V4 fix (these `base_Cap*` markers are the same display-widget+SVG structure with z=90 rotation + flipV; `apply_node_rotation` + `flip_adjusted_contain_position` make them symmetric at the diagram edges — top 768/bottom 767, left 717/right 717). Background centred via V5. Residual: g-force diagram is ~63px left of screen centre (cover-fit ball-area centring) and 1436 vs 1536 wide (slightly squashed cross); cardinal markers are circles vs the reference's rounded squares (owner-confirmed circles) (see [[gforce-hud-blank-fix]]) |
| Velocity ball | `Screen_Small_Radar1` / LOD0 cockpit | `HC_HUD_Ship_Velocity_Ball_Master` | `velocity_ball_master.png` | `target` | **PLATINUM `clipper_velocity_ball_master`** (frozen 2026-06-15) | PARITY PASS 2026-06-15 (owner-approved platinum): inherits the g-force structural fixes (centring overflow-gate, `shrinkProportion=0` square ball, full-ellipse cubic circle dot) — centred square cross, circular centre dot, four chevron caps symmetric at the axis ends, matches the reference. Earlier history: aspect 1.0 (square) fixed 2026-06-14. V1 spokes (`base_Diagram` cross_line `Accent1` op 85/50) + chevrons (`base_Cap*` cross_cap `Critical` op 100/70) cyan→**orange** LANDED 2026-06-15: these HUD glyph SVGs are non-uniform colorstyle (mixed opacity / two roles), which `parse_uniform_colorstyle` rejects → `bb_svg::recolour_colorstyle_svg` recolours each path to its OWN id-encoded role colour at its OWN opacity (generic per-path; uniform SVGs stay on the single-overlay path so the frozen g-force bytes are untouched). Both DRAK roles resolve to the same orange (ref `(213,80,60)` bloomed ≈ render `(193,52,43)` crisp). V2 at-rest cream "L" vector (`base_CurrentLine*` Bright / `base_InputLine*` Base, `SizeY=|flightcontroller/linearvelocity/ratio/z|/2`) COLLAPSE LANDED 2026-06-15: `resolve_geometry_fields_into_scene`'s `value<=0 → keep authored` guard discarded the genuine at-rest 0; now `field_value_source_is_engine_variable` (path-sensitive: follows the taken boolean branch, treats div-by-zero as data-absent) lets an engine-`Variable`-grounded 0 collapse the bar, while the power-pip `1/MaxPipList` + widget-standard unwired `ParamInput` sizes stay placeholders. V4 cap-arrow rotation+positioning LANDED 2026-06-15: node `rotation_deg` (orientation.z+orientationOffset.z) captured in `ui_ir`, applied in `ir_compose::apply_node_rotation` (rotate asset around pivot, alpha-weighted bilinear) — left/right caps author z=90; AND `flip_adjusted_contain_position` pre-inverts contain pos on flipped axes so the whole-box flip lands content at its authored end (bottom/left caps were ~200px short). All 4 caps symmetric at the diagram edges. V5 green background fit+centre LANDED 2026-06-15: `bb_layout::cover_fit_full_bleed_to_viewport` snaps the full-canvas `WidgetImage` background to the visible viewport (was stretched to the 16:9 canvas + bright centre off-screen). V2b/C6 white centre dot LANDED 2026-06-15 (NOT deferred — owner directed): `is_untinted_overlay_render_shape` (background.enable+null AND svgFill.renderShape+overlay+null+empty-path) on a LEAF → white `background_fill_colour` (colour-overlay identity; zero frozen-target matches) + `fill_rounded_rect_ts_with_mode` renders corner_radius fills rounded (dot is a circle). All five fixes pass ui_check + visual guard. Velocity ball now matches the reference structurally |
| Countermeasures | `Countermeasures_Screen` / LOD0 cockpit | `HC_HUD_Ship_Countermeasures_Master` | `countermeasures_master.png` | — | **GOLD `clipper_countermeasures_master`** (frozen 2026-06-18) | aspect 1.0 (square) fixed 2026-06-14. **BLANK-AT-REST FIXED 2026-06-18** (`15c94c7bc` cover_fit_recentre on-canvas filter — ignored the off-canvas pivot.x=22 hold-circle; `f1da3030b` `derive_countermeasure_launchers` in ship_values + materialised-entry distribute): screen now shows 2 stacked panels with counts (noise top=5, decoy bottom=48). **FIT + COUNT + DOT LANDED 2026-06-18:** (1) FIT — the cockpit RTT screen renders the 16:9 useRaw canvas via non-uniform FILL/stretch, NOT uniform cover. Proof: cover over-zoomed content ×1.78 (count "5" 35% tall vs ref 22%; panels full-bleed) — ref panels are rounded plates w/ corners AND the ref "5" is vertically stretched (aspect 1.67 vs normal ~1.4). `bb_layout` UseRaw `cover_fit && mesh_aspect_fill` → fill (csx=sx, csy=sy): `PercentOfX` nodes (ball gauges) stay round, `Percent` nodes (panels) stretch — same fit serves balls+CM. `mesh_aspect_fill` = `cover_fit_mesh_aspect && binding_kind==physical` (set in `pipeline/mod.rs`, threaded through `ui_ir`): the RADAR (`binding_kind==radar`, 3D-RTT scope) keeps COVER (its disc projection + chrome were tuned for it; byte-identical render, gold UNTOUCHED). Recentre gated on `canvas_overflows` (cover only; fill has none). **Re-froze PLATINUM g-force + velocity ball** (owner-approved 2026-06-18; both improved TOWARD ref — g-force no longer 63px-left, velocity exact). (2) count RIGHT-aligned: right-anchored `_MaterialisedEntry_` content-fits its cross width so the column placement honours its authored `anchor.x=1.0`; the dot↔count gap (22×50=1100) only fits under FILL (cover's 1956 overflowed). (3) white DOT renders at panel-left (`card_CountermeasureFireHoldCirlce`, alpha 0.5, x≈16%; pivot.x=22 lands it left of the right-aligned entry). **ALL 4 CATALOG ISSUES LANDED 2026-06-18** (`2538c8743` + `e946eb73a`, ui_check ALL GREEN; matches ref — top dot+5, bottom dot+4). (#2) firing "0" HIDDEN: `bb_state_filter` now bridges at-rest numeric cold defaults into the IsActive eval (`resolve_static_number`/`eval_bool_from_number` + `numeric_variable_defaults`→`static_nums`, SCOPED to BARE component-local bindings so frozen screens stay byte-identical — the unscoped cut regressed clipper_self_master, ledger 88); `CurrentBurstSize`=0/`BurstSizeHoldRatio`=0.0 registered as universal cold defaults (ledger 87). (#4) decoy VALUE 4 not 48: the DRAK count widget binds engine `AmmoCount` (no DecoyCount widget, no UI arithmetic); `derive_countermeasure_launchers` now derives the display per FIRE-ACTION TYPE — Single→raw ammo (noise=5), Burst→ceil(ammo/`COUNTERMEASURE_CHARGE_SIZE`=12) (decoy 48→4). Charge size 12 is engine-runtime (exhausted: launcher weapon/fireaction shotCount=1, behr_flare+VehicleCounterMeasureFlares+TALN_Chaff ammo, resourcetypes, weapon controller, P4K, cross-ship survey 48/25/30 fits no stored divisor) → ONE universal derivation constant + IOU (owner-approved). (#1 dot + #3 alignment) RESOLVED BY #4 (ledger 90): the authored dot pivot.x=22 (~1100px) + content-fit entry are calibrated for a SINGLE-digit count; "48" pushed the decoy dot off-canvas (x≈−170) + the right edge in — "4" makes both panels identical (dot at x=313 both panels, counts right-aligned). NOT onboarded as a frozen target (no tier). Minor residual: dot at authored ~16% (faithful to the pivot). See [[ui-radar-screen-parity]] for the cover-fit/aspect family. |
| Compass | `Screen_Central_Compass` / LOD0 cockpit | `HC_HUD_Ship_Compass_Master` | `compass_master.png` | `compass` | **GOLD `clipper_compass_master`** (frozen 2026-06-16) | PARITY PASS 2026-06-15: was a WHITE sheet over the vignette. Item1 LANDED (`a47759308`): `CanvasProxyRoot` (`rendererType:"None"` non-render proxy) painted its authored white `background` — `node_background_enabled` now false for explicit `"None"` (only `"Flash"` nodes draw bg) → dark `DRAK_Background_compass` vignette shows. Item2 LANDED: `list_Ticks` arrayVariable `FlightController/Compass/Ticks` absent at rest → lone `base_Tick` CLONE template rendered as a stray left-edge tick; pinned count 0 (registry) + `apply_array_variable_lists` now resolves a namespace-less MULTI-segment engine path (bare names like power `pipList` still need a namespace) → template deactivates, canvas reflows so lubber line spans full height (matches ref 0.02..0.99). Lubber + vignette MATCH. Item3 live ticks/labels: FIRST mis-called a proven blocker, then RESOLVED on a deeper search — the projection config IS decoded data `SVehicleHudParams.VehicleHudDefault.compassTape` (`hudparams/vehiclehuddefault.xml`: `range=90`° window, `mainTickIncrement=20`° labelled, `subTicks=4` → minor every 5°; verified vs ref). DERIVED the at-rest tick array in `ship_values.rs` (`derive_compass_ticks`+`compass_tape_from_global`, `COMPASS_AT_REST_HEADING_DEG=0`): 19 ticks in [-45,45]°, registry paths `FlightController/Compass/Ticks`+`…/[000j]/{anchor,maintick,value}` (overrides the static `=0` fallback). Render shows 320/340/0/20/40 + major/minor ticks; labels differ from ref's 100/120/140/160 ONLY because the ref is a LIVE heading (~132°), unreproducible statically (heading 0 = principled at-rest, like velocity=0; NOT hard-coding — all from compassTape). Minor ticks at authored alpha 0.2 (faint; ref brighter=bloom). NOT onboarded (freeze-gated).  **FOLLOW-UP (owner feedback) 2026-06-15:** ticks clipped + font/colour wrong once populated. Ticks LANDED `4f352b429`: `bb_layout` lumped `coordinateMethod:"auto"` with `useRaw`; compass master UNIQUELY authors `auto` → now FILLS the target like `aspectOverrides*` (non-uniform), major 235px/minor 59px bottom-aligned (no frozen screen is `auto`). Font LANDED `ca5ec0b25`: height-driven labels (Percent height + `PercentOfY` width) size font to the field, not the Heading1 60 (~6%→15.5% cap); a broad Percent-height gate REGRESSED medical `ui_target_a` (live-IR guard caught) → `PercentOfY`-width discriminator. **ROUND 3 LANDED + ONBOARDED GOLD 2026-06-16 (`5ba8f4b7e`): the colour was NOT a blocker — 67(d)'s cardinal-tag theory was WRONG** (that tag colours only cardinal/intercardinal dirs, never the all-orange 100/120/140/160 the ref shows). Root cause: the compass is a cockpit HUD ship-component but `collect_standard_text_styles` classified it `env` (only `MC_*`/`M_*`→`hud`), so it used `s_drak_env` H1 = audimatmono-**bold**/**Bright** not `s_drak_hud` H1 = audimatmono-**regular**/**Accent2**. `is_cockpit_hud_canvas` (`HC_HUD_*`/`H_HUD_*`/`H_Eng_*`→`hud`, in `bb_brand_style.rs`) fixes weight (bold→regular) AND colour (white→Accent2) in one change. Font SIZE/clipping also fixed: a height-driven field size IS the glyph EM → used VERBATIM (`resolve_effective_font_size` returns it styled), NOT ÷imageSizePercent (the 0.75 plain-size boost over-sized to ~22% cap+clipping; verbatim ~19% with a tick gap, matching ref). g-force (`HC_HUD_*`) took the same HUD-typography correction on its off-screen at-rest readouts (nodes 28/29/39 bold→regular, 29 Bright→Accent2; re-frozen, visually unchanged). LESSON: read nested records by JSON structure, NOT line-range grep — a `sed`/`grep` window of the 2 MB `textfieldwidgetstandard.json` found the WRONG H1 entry and falsified the (correct) brand-class diagnosis; a 6-line `json.load` + iterate gave the truth (ledger 68). aspect 3.27 fixed 2026-06-14 |
| Radar | `Screen_Radar_RTT` / LOD0 cockpit | `MapDisplayMaster` | `Screen_Radar_RTT.png` (+ `mapdisplaymaster.png`) | — (whole-image) | — | **PARITY PASS 2026-06-17.** `MapDisplayMaster` is a MULTI-MODE map (StarMap/Interior/Galaxy/RTT + readouts) gated by `/MapNamespace` flags. **#1 mode gating** (`044e35d38`): registry pins select the cockpit-radar mode (`IsRTT/IsStarMapActive/IsRadar`=true, `IsInteriorMapActive/IsGalacticMapActive/IsVolumetric`=false, `IsMapActive`=true) → deactivates the interior-map over-paint (cream panel/"APARTMENT"/"ALREADY SELECTED") that filled the screen; **#3a** `StarMapData/ShowRadarLocked`=false hides the spurious readout padlock. **#3b** orange readout backplate suppressed: `RadarMagnification` is `rendererType:Primitive` w/ an **AR_HoloVolume** material → `node_background_enabled` now suppresses the flat fill of holo-volume Primitive cards (`52901ec10`; the brand `Card→null` suppressor is Greycat/RSI-only, no DRAK). **#2 radar scope BUILT data-faithfully** (`9e38f1501`/`d76270597`/`e530c0820`): the scope is a 3D-RTT `WidgetWindow` (`WindowContainer`, material `map_window.mtl`, camera FOV 20); reproduced like the hologram via `gfx::radar_plane::project_radar_disc` — loads the REAL disc texture (`r_radarmapscreen_radial_gradients.dds`, bound by the `Circle_Radial_Grid` node's `ui_r_radarmapscreen_radial_grid.mtl` TexSlot1), tilt-projects it (circle→ellipse), tints by brand Accent1, + the `Circle_Ripple` WidgetCircle ring + own-ship triangle. Wired via `HologramFetcher::fetch_radar_plane`→`ir_compose` (gated to the `map_window` WidgetWindow). **PER-MANUFACTURER**: IR `primitive_material` prefers the cascade-applied `PrimitiveMaterialPath` brand override (Greycat `ui_grin_…`/RSI `…_RSI`) over the authored generic. DATA-faithful: only the camera tilt (37°) + emissive brightness owner-tuned. `ui_check --full` GREEN, no frozen regression. DEFERRED (live, like compass heading): contact chevrons (live MapMarkers), heading/range "130° 0.7km" (`FlightController/Compass/Value` + `radarrangemeters`); residual: spoke prominence (in the gradients texture, softened). Handoff: `crates/starbreaker-ui/docs/ui-radar-parity-handoff.md`. aspect 1.23 fixed 2026-06-14 |
| Self MFD | `Screen_Left_Upper_RTT` / LOD0 cockpit | `MC_S_Self_Master` | `self_master.png` (seated capture — shows MORE than at-rest) | — | **GOLD `clipper_self_master`** (frozen 2026-06-16, text-category) | **PARITY + GOLD 2026-06-16 — origin-centred hologram framing (owner-directed).** The hull fills ~65% of its longest IMAGE axis via `min(hull-fit, box-fit)` in `mesh_holo` (so the shield cage NEVER clips on any hull aspect — verified Clipper/Gladius/Avenger), centred on the model ORIGIN (projects to (0,0) → image centre, so the body is centred even for an asymmetric hull). `ir_compose::draw_non_text_node` draws the `WidgetRuntimeImage` FULL-SCREEN (full width, image-top→footer-top = the node rect's own bottom edge), NOT the inset 2D content sub-rect. Shield box (`with_shield_panes`, data-driven): faces sized to the hull silhouette per face, placed at `hull_half/shieldDistance` (face spans `sd` of its box side → ~15% corner gap), PANES ROTATED 90° in-plane BEFORE the stretch so the curve bows OUTWARD (was flat trays), box centred on the origin & sized to the hull's max reach FROM the origin, then scaled 0.82 toward the origin (owner view-choice constant, like yaw/pitch/fit) so faces read smaller/hug the hull; faces at 0.30× hull alpha. Generic: NO ship/manufacturer/screen branches (hull + shield proxy + `shieldDistance` all DataCore). Onboarded GOLD as **text-category**: the composited 3D hologram is guarded by the WHOLE-IMAGE artifact; the standalone-canvas IR snapshot only reproduces the screen's text fields (the hologram needs the cockpit scene + fetcher the snapshot pipeline lacks). **OPEN follow-up — footer SELECTED highlight**: the cream-bg/dark-text style (`ScreenNameBackground_Selected`/`ScreenNameTextSelected`, gated by an ANCESTOR carrying tag `fef243b7` via the `SelectedName`/`UnSelectedName` entries in `mfd_g_header`/`gen_mc_s_header`) is engine runtime FOCUS state with NO static signal (no `selectedViewIndex` anywhere in the MFD canvas data; the self capture was focused, target's wasn't) — frozen UNSELECTED, matching target's at-rest state; reproducing it would need a per-screen focus pin for what is likely a capture artifact. **Earlier history (parts SUPERSEDED — the square box / 15% margin / `SPAN_FRAC` / fit 0.5 / 0.5 shield alpha below are replaced by the framing above): SHIP HOLOGRAM LANDED 2026-06-16** (not frozen). The reference `self_master.png` is a SEATED capture with extra weapon/gimbal/ammo content; the at-rest (standing) target is just the **own-vehicle hologram + 4 shield faces + SELF STATUS footer**. The central "Own Vehicle Hologram" is a `BuildingBlocks_WidgetRuntimeImage`/`rendererType:Primitive` 3D-scene render the 2D UI pipeline can't natively produce → reproduced HEADLESLY: new CPU mesh rasteriser `starbreaker-gfx::render_vehicle_hologram` (orthographic→**perspective** projection, painter's-order **shaded filled faces**, optional wireframe, neutral greyscale × data-driven tint) fed by `starbreaker-3d::ui_pipeline::hologram_fetcher::P4kHologramFetcher` (resolves the render scene's ROOT vehicle hull `.cga` via `SGeometryResourceParams`, decodes via **`parse_skin_positioned`** — generic per loaded ship, full hull). **`parse_skin_positioned` (lib.rs) BAKES the NMC scene-graph node transforms** into a world-space mesh (the same assembly `skin_to_glb`/the Tauri viewer applies): raw `parse_skin` leaves multi-part geometry (wings, both engine pods, sub-objects) DETACHED at the origin; `flatten_nmc_to_world` composes each node's `bone_to_world` down the parent hierarchy and transforms each submesh's INDEX-range vertices → the complete symmetric ship. Unit test `nmc_flatten_tests::flatten_bakes_node_and_parent_transforms`. Wired via a `HologramFetcher` trait on `PipelineInputs`→`ComposeContext`; `ir_compose::draw_non_text_node` composites it into the `WidgetRuntimeImage` rect (gated on the node's authored `background_fill_colour` = the per-mfr holo tint, so the untinted "old man vehicle" sibling is skipped). SELF-STATUS framing (owner-tuned): yaw 0° (nose AWAY / up the frame), pitch −30° (nose dips DOWN), perspective 1.5, fit 0.5, semi-transparent faces (face_alpha 0.25, no wireframe). **Visibility**: the player diagram (`base_Root`) gates `IsActive` on `/SeatDashboard/PowerState == 1`; registry pin changed `"OFFLINE"`(str)→`1`(int enum, OFFLINE=1) — power screen still shows "OFFLINE" (ui_check --full green, clipper_power_master unchanged). Guards: gfx `mesh_holo` unit tests + `ir_compose` `render_ui_ir_document_composites_vehicle_hologram`. **FOOTER TEXT FIXED**: two `SMFDView`s author `eView_SelfStatus` — pilot (`@ui_MFD_View_SelfStatus`) + turret variant (`@hud_turret_status`); `build_mfd_view_canvas_map` `or_insert` kept the turret one → "TURRET STATUS". `prefer_new_view_entry` now keeps the canonical `@ui_MFD_View_<Type>` record on collision → "SELF STATUS" (only the self slot collides; frozen power/target footers unchanged, ui_check --full GREEN). **OPEN follow-ups**: (#4) footer should be the inverted SELECTED style (bright-yellow bg `ScreenNameBackground_Selected` + black text `ScreenNameTextSelected`), gated by the `selected` style tag (`fb089da7`) applied via `BindingsTagFromBoolean` ← component-param focus relay (`selectedViewIndex==thisIndex`); needs scoped focus-state resolution for the primary MFD (a global enable regresses frozen power/target). **(#5) SHIELD FACES LANDED 2026-06-16** (`e97b0105d`): 4 shield faces forming a SQUARE box around the hull. `with_shield_panes` (lib.rs) wraps the hull with `UIHoloVehicle_Config.shieldProxyModel` (`shield_pane.cgf`, resolved data-driven from the DataCore record — no hard-coded path); `hologram_fetcher` passes the shield-triangle boundary + a 0.5 alpha scale so shields render at HALF the hull face alpha (`HologramParams.shield_index_start`/`shield_alpha_scale`, per-triangle in `render_vehicle_hologram`). **Pane-axis gotcha** (a warped/rotated first attempt): the proxy is flat in its **X-Y plane** (width X, height Y) with a shallow dome along **+Z** (its normal), NOT flat in Y-Z/normal +X — standing it up means width→tangent, height→world up, dome→outward, then rotating 0/90/180/270° about the vertical axis (identical faces, rear = front rotated 180°). Box = 15% margin off the larger horizontal dimension (owner spec, NOT the engine's `shieldDistance 0.85` which has different semantics), 2× hull height centred, `SPAN_FRAC 0.8` → corner gaps. `UIHoloVehicle_Config` also carries the authored holo camera (`cameraFOV 10`, `ownDefaultViewAngle 60`, `ownCameraDistanceScaler 1.5`) — a future data-driven-camera source vs the owner-tuned framing constants. Diagnostic: `cargo run -p starbreaker-3d --example holo_preview -- <cga> <out.png> <tilt> <yaw>` (needs `SC_DATA_P4K`). mfd 4:3 frame path |
| LR-indicator | `Screen_Left_Upper_RTT_Small` / LOD0 cockpit (needs `--lod 0` AND the wrapper picks LOD1 for the `_Small` stem — pass `--lod 0`) | `HC_HUD_Ship_LRInd_Master` | `lrind_master.png` | — | **GOLD `clipper_lrind_master`** (frozen 2026-06-19) | **PARITY PASS 2026-06-18→19** (`7f6d688e5`/`7f1edd52f`/`53b29b137`/`6cb31a350`/`2bbf9b214`/`64020f7cc`). Reference is the ship's INITIAL POWER-ON state; the screen is ONE tall portrait image, the apparent indicator cells are cockpit GEOMETRY BEZELS in front of it (not the UI render). The master tiles TWO half-width interchangeable `Set Canvas Left/Right` columns (Left=CPLD/ESP/LOCK, Right=ATMO[→VTOL]/GEAR/GSAF). **(1) UN-BLANK** (was 100% blank): `run_pass1_canvas_references` followed only ONE canvas-ref entry for un-namespaced masters → now follows both when the host slots are sub-full TILING (`host_is_subfull_tiling_slot`, excludes MC_S_Self_Master's full-size overlay modes); AND the Left slot's `Instantiated=(powerstate==1) AND seatdashboard/isactive` was gated off → `is_tiling_sibling_canvas_slot` keeps a gated sub-full-WIDTH disjoint-column slot alongside its sibling (scoped to disjoint horizontal columns: the medical bed's full-width stacked state screens + the radar X/NOT-X overlays are excluded) + pinned `/seatdashboard/isactive=true`. **(2) PORTRAIT aspect** 1.56→0.64: `ui_screen_aspect` was orientation-blind (long/short ≥1); `planar_aspect_w_over_h` now classifies the in-plane PCA axes against world-up (+Z) → portrait screens read <1 (LR-ind 0.64≈ref 0.68; landscape screens unchanged; the velocity/afterburner bars correctly 0.33). **(3) BLACK background** (was brand brown slot-9 (38,27,10)): the full-screen clear used the brand "Background"; cockpit-HUD canvases (`is_cockpit_hud_canvas`) now clear BLACK (the screen bezel) — gauges unchanged (own `Screen Background` nodes cover it), door/MFDs/medical keep the brown clear they composite over (an unscoped black clear regressed `clipper_small_door` 27%). **(4) RIGHT-column lit-state** (at-rest flight pins, reference-verified landed/powered: isvtol=false→VTOL unlit, islandinggeardown/isgsafety=true→GEAR/GSAF lit, isdecoupled/isrotationlocked=false, isesp=true; audit-confirmed byte-identical for the frozen g-force/velocity ball platinum). ui_check --full GREEN throughout. **LABEL FONT FIXED 2026-06-18** (`6cb31a350`+`2bbf9b214`; ledger 94 OVERTURNED → 96/97): labels were ~5× too small; the prior "undecoded engine HUD-label scaling, no data source" defer was WRONG on BOTH counts. (i) There IS an authored size — the Clipper instantiates the brand sub-canvas `DRAK_HC_HUD_Cutlass_LInd`/`_RInd` (via `CanvasReferenceRecord`, NOT `generic_*`), whose canvas-root `embeddedStyles` authors a bare `Type(Text)` → FontSize 100; it was dropped because the text-format route was `Tier::Brand`-only → now also runs at the EMBEDDED tier for an UNCONDITIONAL bare `Type(Text)` (`TextFormatRoute::BareTextOnly` + `entry_is_unconditional_bare_text_selector`; conditional embedded state/overrides like target `Bright Elements`/medbed stay excluded). (ii) The remaining ~3× is the PORTRAIT screen text scale — the LR-ind is the only portrait cockpit screen (landscape canvas filling a 0.64 mesh stretches geometry ~2.78× but a Fixed font is drawn at design px) → `ui_ir::portrait_font_screen_scale` (vertical fill axis for a fill-branch screen with `target_h>target_w`, 1.0 elsewhere) applied to BOTH the layout text measure (so the content-fit field grows, no clipping) and the IR font. Cap 1.2%→5.5% of screen (ref ~6.6%, residual within capture bloom — NO magic scalar). `ui_check --full` GREEN, every frozen baseline byte-identical (no re-freeze). NOTE: bb_layout fill-branch `canvas_scale` is INERT for the IR glyph render — `ir_compose::draw_text_node` sizes from the IR `font_size`. **CPLD/ESP LIT FIXED + ONBOARDED GOLD 2026-06-19** (`64020f7cc`): the left-column CPLD/ESP indicators (authored-false `DisplayWidget`s, `IsActive ← flightcontroller.isdecoupled`/`isesp`) are now lit at the at-rest power-on state (coupled+ESP-on); LOCK/VTOL stay correctly unlit. `forced_active_widgets_with_defaults` was `WidgetImage`-scoped (radar bg) because broadening to ALL DisplayWidgets regresses medical (`ui_target_a`, gated on `Bed/state.*`, spurious-true at rest); now ALSO activates a DisplayWidget whose `IsActive` is grounded in a `flightcontroller.*` variable (`isactive_grounded_in_flight_controller`) — medical (Bed/state) excluded, radar (WidgetImage) kept. The disable→adjudicate audit (ALL THREE suites) proved medical was the ONLY frozen target the broad scope touched. The dossier's "+2 velocity-ball" was REAL: the velocity-ball canvas carries its OWN `base_CPLD`/`base_ESP` (nodes 25/27) laid out **0×0/off-screen** → they activate too but render nothing (whole-image guard GREEN); re-froze PLATINUM `clipper_velocity_ball_master` (IR snapshot +2 active + 4 draw-order reindexes, owner-approved). Onboarded **GOLD `clipper_lrind_master`** (image category, the portrait bound render). Residual: the reference shows VTOL/LOCK lit too, but isvtol/isrotationlocked=false at rest = principled unlit. See [[mfd-square-aspect-uimaterial-rule]], [[gforce-hud-blank-fix]]. |
| Velocity num | `screen_flight_hud_left_upper` / LOD0 cockpit | `HC_HUD_Ship_Velocity_Num_Master` | `ship_velocity_num_master.png` | — | **GOLD `clipper_velocity_num_master`** (frozen 2026-06-15) | PARITY PASS 2026-06-15 (DRAK cutlass variant, two readouts stacked centred: "0m/s" / "0.0 G"). Was 100% BLANK at rest. LANDED: (1) at-rest content — both engine `NumberVariable`s (`flightController.forwardVelocity`, `flightcontroller.totalgforceamount`, +`ForwardBackSpeedGoal`) absent in static replay → placeholders ('-', 'G'); pinned =0 in the default-value registry (`1f25845d2`, AccelerationLimiterRatio precedent) → velocity "0", g-force "0.0 G" ✓. (2) layout — `base_CurrentVelocityNumber` is a flex Column(axisJustification/cross/item=Center) whose two `WidgetCard`s author non-zero Auto (64>1.0) on both axes; the flex sizer had no content-fit path for value>1.0 text-backed column children → they filled the canvas, overlapped, dumped glyphs top-left. Added main+cross-axis content-fit via `auto_text_intrinsic_main`, gated `axisJustification==Center` (no frozen screen has it — medical pins fill for non-zero column Auto, workflow §10) → cards shrink-wrap + Center stacks/centres them (`0cdf6d526`, visual+snapshot guards GREEN after `--lod 0` export). Colour Bright≈white faithful. (3) **velocity "m/s" unit LANDED** (`131c2f8b8`): `LocalizedSIUnitFromNumber` now renders `unitSuffix` — the SI unit symbol is the systematic localization key `text_ui_SIUnit_<suffix>` (`Speed`→"m/s", from global.ini at runtime, NO hard-coded string), and `forcedSIPrefix="Unit"` pins the base unit (no K/M/G); `unitSuffix="None"` adds nothing (g-force "G" comes from the later combine). Render now "0m/s" / "0.0 G". Frozen target master ALSO uses SIUnit Speed/Distance but its bindings are absent at rest (no target) → None → unchanged (visual+snapshot GREEN). **FONT ~9× LANDED 2026-06-15** (`6c1343abf`): the prior "52px Heading2, undecoded per-screen scale" diagnosis was WRONG. The DRAK velocity SCREEN variant (`drak_hc_hud_cutlass_velocity_num`, the `(Screen)` tag-79f1bb24 variant the Clipper instantiates) authors FontSize **500** ("0m/s") / **420** ("0.0 G") in BOTH its `defaultStyles` (FontSize + white StrokeColor) AND a duplicate `s_grey_hud` brandStyles block (FillColor Base). A drak ship matches NO brandStyles entry (only `s_grey_hud` declared → grey-HUD ships), so per the engine "no brand match → defaultStyles used at every level" rule the sizing comes from **defaultStyles**, applied at the BRAND tier as a no-brand-match fallback (`apply_canvas_style_cascade`) so the textfield text-format route reaches the readouts. The route fix: a WidgetTextField's implicit text-format CHILD is a `Text` node, so `Type(Text)` matches it INSIDE a `Parent(...)`-wrapped entry (velocity's `Type(Text)+Parent[(Not)Tag(fontnumber)]`); a bare/`Ancestor`-wrapped `Type(Text)` stays widget-route (MFD footer screen-name SAFE — it's `Ancestor`-wrapped). Render 20.9%/16.2% cap-h vs ref 20.4%/16.2% (size_ratio 1.01), centred. **COLOUR faithful**: text keeps its natural `Bright` (244,244,218 ≈ ref 232,236,218) — applying the `s_grey_hud` BRAND instead (the FALSIFIED first approach) forced FillColor Base → drak orange; defaultStyles has no FillColor so the natural white survives. **BOXES fixed** (`c7931fb4b`): defaultStyles' white StrokeColor landed on the node `stroke_colour` and `ir_compose` drew it as a box around each field — a WidgetTextField's StrokeColor is a GLYPH outline, not a box (`node_draws_rect_stroke` now excludes `widget_text_field`). **(c) background = PROVEN capture characteristic**: render faithfully reproduces the `DRAK_Background_indicators` dark-centre/bright-edge vignette; the ref's brighter uniform rounded panel + dark bezel is the physical emissive screen + rounded screen-mesh shape (same bound the g-force/velocity-ball platinum passes accepted); no brand-driven brightening exists for the drak no-brand-match path. **TRAP STILL VALID: broadening `ir_compose` `center_anchored_heading` (Heading1→any label_style) REGRESSES medical-bed title/desc — centre via card cross-fit, NOT the text-draw rule.** ui_check --full GREEN, no frozen-screen drift. ONBOARDED **GOLD `clipper_velocity_num_master`** 2026-06-15 (owner-approved, `a91cec377`; manifest 9→10; artifact from `--lod 0`). aspect 1.24 fixed 2026-06-14 |
| Carrack lift-call console | `console_liftcall_009` (ANVL Carrack interior transit peripheral; screen mesh `anvl_crk_console_liftcall_009`, material `carrack_interior:RTT_screens`) — no Clipper-scene helper; iterate via `ui render --scene <tiny scene.json>` with a hand-written `physical` binding (canvas `a2c5fae4-f018-4d05-8ab7-e4f17a4d8ae4`, 512×740, manufacturer anvl) or a full Carrack decomposed export | `OLD_TransitUIPanelExterior_ANVL` (per-instance `.soc` `EntityComponentUIBuildingBlocks` override — the exporter binds it via `InteriorMesh::ui_canvas_guid`, commit `a65b4927c`; class default is the generic `OLD_TransitUIPanelExterior`; the in-lift screen uses `OLD_TransitUIPanelInterior_ANVL` `4a8bbd52-…`) | `reference/in-game/Carrack/mesh_anvl_crk_console_liftcall_009.png` | — (whole-image; no preset yet) | — (not onboarded) | **ARC 2026-06-30→07-02.** Was 100% BLANK (no binding — transit peripherals author the canvas INLINE on the `.soc` component; exporter fix `a65b4927c`). Button chrome landed `29c2c1fc3`+`fd5147062`: primary `ComponentGeneralButton` widget-standard expansion (`buttongeneralcomponentstandard.json`), `Label` param injection (`ParamValue::Loc`), icon-identity forwarding (+parse-time `BbIcon` rebuild), `icon-/text-element-instance` tags (SHOWN elements only), `sk_uilo_a_buttonprimarystyles` module sheet, literal-colour stale-token drop, caseModifier forwarding. Renders: separators, yellow rules, bottom bar, corner image, green Filled backplate, double-caret (CENTRED — flex-column cross-axis fix `e4e5e5906`; colour black→Background `560c9ffe2`, see CHEVRON COLOUR FIXED below), dark "CALL / ELEVATOR" (2-line like ref). **SUB DECK heading RENDERS** (`51feaf23a`): `Label_ThisFloor` bound to the transit floor loc key resolved from the socpak `SCTransitDestination` metadata via nearest-within-radius association (`transitdisplay.panelLocation` pin; iterate standalone with `--transit-loc-key @vehicle_deck_sub_deck`). Button caret/label centring shares the `ComponentGeneralButton` icon-anchor mechanism fixed for the medbed close ✕ (`e4e5e5906`: host `iconProperties.anchorToParent` → template params + flex box-centre for no-cross-anchor items). OPEN: second label line clips at the field's bottom edge (line-box/field fit); chamfered corners (RootFilled authors `EnableTopLeft/BottomRightBorderChamfer` + `BorderBottomRightRadius 20`; per-corner radius+chamfer geometry landed `5e9f3f270`/ledger 106 — bottom-right chamfer now drawn). **CHEVRON COLOUR FIXED 2026-07-04 (`560c9ffe2`, ledger 107):** the double-caret rendered PURE BLACK because `sk_uilo_a` authors its icon `FillColor` as a `ColorSolid` pure black (editor placeholder; siblings use `ColorStyle(Background)`), which the cascade resolves to a token-less RGBA → SVG native black. A non-HUD `widget_icon` with a token-less pure-black `FillColor` and no resolved tint now inherits its SIBLING text field's colour token (`apply_placeholder_black_icon_inheritance`, `ui_ir/engine_01.rs`), so the caret matches CALL ELEVATOR (Background). `ui_check` ALL GREEN (element-level gold tint-semantics unchanged). **Standalone repro floor loc-key for the sub-deck panel is `@ui_interactor_carrack_garage` (→ "Sub Deck"), NOT `@vehicle_deck_sub_deck`.** `/ships/` re-exported UI-only@LOD0 (`--ui-only-files`). Owner 2026-07-04 confirmed the re-export also cleared a transient STALE-export "button-low / SUB DECK-missing" report (fresh export: all 17 console bindings floored, garage panel = SUB DECK + centred button). **OPEN — SUB DECK heading font too small (owner-flagged, DEFERRED by owner 2026-07-04 "leave the work here").** Confirmed real (I first wrongly called it within-tolerance): reference SUB DECK cap-h ≈ CALL ELEVATOR cap-h (ratio ~1.10) but the render draws 0.636 (nominal Heading3 28 vs Heading1 44). Button geometry matches in canvas space (~332×183) so it is NOT aspect; SUB DECK is ~1.35× too small (in-game effective ~38px vs my 28px). Root cause traced but not fixed: the caption is authored `captionProperties.style="Heading3"` with `autoScalingMethod:None` + no `autoFontSize`, so my pipeline renders nominal Heading3; in-game it renders ~38px ≈ its 64px field height (the compass "height-driven labels" pattern `ca5ec0b25` is the candidate fix, data-backed off the field height — NOT a scalar). **HARD-CODING FLAG (AGENTS.md self-correcting ban):** `crates/starbreaker-ui/src/compose/text_draw.rs:100-102` hard-codes a heading→size fallback table (`Heading1=>48, Heading3=>28`) — game-data values in source; must be derived from the brand/typography records (verify whether the real brand Heading3 size is failing to apply and falling back to 28, the velocity/master-mode pattern). Caption/heading sizing is a SHARED mechanism → any fix needs `ui_check --full` + sibling eyeball. REMAINING (owner-deferred): CRT scanline overlay + corner triangles/bezel + aspect literalness are Blender-shader/geometry / capture characteristics (out of scope, owner-confirmed) |
| Master-mode display | `screen_flight_hud_right_upper` / LOD0 cockpit | `HC_HUD_Ship_Master_Mode_Display_Master` (DRAK→`drak_hc_hud_cutlass_masterm_display` variant) | `master_mode_display_master.png` | — (whole-image) | **GOLD `clipper_master_mode_display_master`** (frozen 2026-06-17) | **PARITY + GOLD 2026-06-17 — the ARC-1 "PROVEN BLOCKERS" were OVERTURNED (ledger 76; owner-pushed).** Matches the reference: 4 white bullet icons + large white "SCM"/"GUN" + cream bar, no grey boxes. **Size+colour (ARC-1 blockers (a)+(b)) — ONE root cause, `b9350d1b9`:** the DRAK variant's `defaultStyles` "New Style" (bare `Type(Text)`) authors FontSize **350** + FillColor/StrokeColor **white**, but it was never applied (text fell back to widget-standard Heading1 60/Accent2). `condition_matches_text_format` routed `Type(Text)` to a WidgetTextField's text-format child only when `Parent`-wrapped; now also for BARE (discriminator `entry_has_parent || !entry_has_ancestor` — the MFD-footer `Ancestor` case stays excluded). No tag-gating → no compass conflict (ARC-1 blocker (b) reasoning moot). **Icon = white bullets + GUN submode, `18a92a2d9`:** pin `seatdashboard/currentmode=3` (Combat/weapons-on, the powered SCM default — owner-confirmed, overturns ARC-1 "live-state, no principled value") drives the `guns.svg` icon (`ParamInput4`) AND "GUN" (`@hud_op_Combat`). A bound `SvgPath` now overrides the authored placeholder when it's a real PATH (`contains('/')`) — NOT a chrome tag ref (that broke the MFD footer nav arrows, ledger 77). `guns.svg` is authored all-white; the MFD `MissionObjectives` generic-icon default is gated OFF for HUD canvases (`is_hud_canvas`) so the bullets keep native white. **No grey box:** `card_Icon`/`card_CurrentModeText` are `rendererType:Primitive` + literal `ColorSolid` white α60 → editor placeholder the engine ignores; `node_background_enabled` suppresses Primitive+ColorSolid (palette-role `ColorStyle` Primitive fills like the power `PipBox_Fill` still render). **Residual (capture, not fixed):** SCM/GUN render at the authored FillColor α0.64 (slightly grey); the ref's brighter white = emissive/bloom (same bound as velocity-num). **ARC-1 history (`2b32a1b2e`/`ca5d44904`/`1efbf87df`): D1 (`2b32a1b2e`):** the AEGS-Gladius ship-in-oval rendered OVER the DRAK layout — the interchangeable host authored `canvas`=aegs_gladius (editor placeholder) AND a brand-style `Set Canvas (Screen)` set drak_cutlass; both instantiated. `all_canvas_guard_entries` now collects canvas-ref URLs from ALL brand+default entries (not just `pick_active_entries`) so the Pass-2 mode-switch guard skips the non-active-brand placeholder (velocity-num latent — its host authors `canvas:""`). **D2 (`ca5d44904`):** the two mode-text `WidgetCard`s author `Auto` value EXACTLY 1.0 in a Center column → fell in the gap between the `(0,1.0)` fraction path and velocity-num's `>1.0` path → filled+overlapped (big grey box). Center-column content-fit gate broadened `>1.0`→`>=1.0`, SCOPED to `WidgetCard` (bare fields at 1.0 keep fill — the power gold `2 / 16` `564/565` is top-aligned-fill; live-IR caught the unscoped attempt). **D3a (`1efbf87df`):** pinned `controllablemanager/mastermode=2` → "SCM" (principled powered at-rest mode). **PROVEN BLOCKERS:** (a) text ~6× too small — drak_cutlass uniquely authors `coordinateMethod=auto` with fixed `Heading1`=60 (no autoFontSize/scale/Percent-height); the reference cap ~14% (icon-calibrated, the icon is 28% geom) needs ~340px. An "auto cards fill + autoFontSize" experiment hit ~20% but REGRESSED the frozen `clipper_target_master` (MC_S, also `auto`, 7.4% whole-image; ledger-52 invisible to the IR snapshot) — no clean discriminator; (b) "SCM" orange vs ref white — the standard `s_drak_hud` H1 sets `FillColor:Accent2` by-name (`brand_text_style_colour_token`/`_fill_rgba` ignore the entry's condition TAG `c6348321fe4c`). Gating colour on the tag (even scoped HC_HUD+auto) REGRESSES the frozen compass (its derived labels are HC_HUD+auto `Heading1` with non-H1 tags `21788313`/`97f6322f` that MUST stay Accent2-orange) — master-mode + compass are structurally identical but want opposite colours. **Live-state (unreproducible at rest, like compass live heading):** the bullets icon (`shape_Icon`←ParamInput4 weapon-group switch on `seatdashboard/currentmode`) and "GUN" submode (`text_SubMode`←currentmode switch: Flight/Combat/Turret/…, none is "GUN"). **Capture/known:** card backplates (`background.enable` SRGBA8 α60) render more prominent than the ref — the linear-light compositing gap (gated-open). aspect 1.24 fixed 2026-06-14 |
| Velocity / afterburner bars | `screen_flight_hud_left` / `_right` / LOD0 cockpit | `HC_HUD_Ship_Velocity_Bar_Master` / `…_Afterburner_Bar_Master` | (no straight-on capture yet) | — | — | aspect 2.99 fixed 2026-06-14 |

Per-screen render aspect (commit `cc67d79e2`, 2026-06-14): physical/radar
screens are sized to their cockpit screen-mesh aspect (the `RTT_Screen`
faces), so the square gauges, compass and annunciator no longer render
squashed. This needs the **LOD0** cockpit geometry (the small HUD screens
are culled in LOD1); the freeze scripts export `--lod 0`.

Scene split: **LOD0** (`DRAK Clipper_LOD0_TEX0/scene.json`) carries the whole
cockpit dashboard — the MFDs AND the HUD gauges + annunciator L/R (the small HUD
screens exist ONLY here; LOD1 culls them); **LOD1**
(`DRAK Clipper_LOD1_TEX2/scene.json`) carries the interior usables (medical,
door) — the font baseline and the medical/door frozen targets render at LOD1,
but the regression FREEZE exports `--lod 0` for every target (it reaches the
cockpit screens), so the dossier "scene" column is the *replay* LOD, not the
freeze source.

## 4. MCP tools (server `starbreakerMcp`)

Policy: MCP-first for data archaeology; CLI for renders/exports; LOCAL
PNGs/JSON are read directly with the Read tool (vision for images) — never
via MCP. Confirm data is loaded with `p4k_data_status`. **`canvas` arg = the
record GUID or name, NOT the dcb_canvas mirror file path** (a path returns
`canvas_not_found`); get the GUID from the mirror file's top-of-file
`_RecordId_` / `_RecordName_` (e.g. `BuildingBlocks_Canvas.GEN_MC_S_Emissions`).

**Style/IR investigation order** (run BEFORE editing style logic; if a
change has no effect in these, revert it):
1. `ui_canvas_style_inventory` — authored containers (embeddedStyles,
   defaultStyles, brandStyles[], inlineStyles) with condition/modifier
   summaries.
2. `ui_scene_style_probe` — resolved scene nodes with style tags, raw
   colour/tint fields, `__AppliedStyleEntries` (which entries actually
   matched).
3. `ui_ir_query` — compiled IR: computed/draw rects, text payloads/bounds,
   resolved tokens and colours (what the renderer consumes).

**Data tools:** `search_entities` (EntityClassDefinition by name),
`search_records` (all record types), `datacore_record` (full JSON by
GUID/name), `datacore_query` (property path, e.g.
`Components[SEntityComponentDefaultLoadoutParams]`), `entity_loadout`
(resolved tree), `p4k_list` / `p4k_read` (CryXML auto-decode) /
`p4k_search`, `image_preview` (vision on P4K DDS/PNG), `chunk_list` /
`chunk_read`, `ui_regression_registry` / `ui_regression_validate`.
Gotcha: canvas JSON says `.tif` → the P4K entry is `.dds`.

MCP server redeploy after changing it:
`pkill -f starbreaker-mcp || true && cargo build --release -p starbreaker-mcp
&& cp target/release/starbreaker-mcp mcp/starbreaker-mcp`, then restart the
client.

## 4b. Code navigation (graphify)

A repo-wide AST + semantic knowledge graph (`graphify-out/graph.json`, built by
the `/graphify` skill; auto-rebuilt by a post-commit git hook — §1). Use it as
the relationship-aware alternative to blind `grep` for **source-code** discovery
— "where does X live, what calls/relates to Y, what's the path A→B" over the
`.rs`/`.py` tree — BEFORE grepping. Answers from `graph.json`, no LLM/API cost:

- `graphify query "<question>"` — BFS neighbourhood for a question (best for
  "what's involved in resolving the brand-style cascade?"); returns nodes with
  `file:line`.
- `graphify explain "<symbol>"` — one node plus its neighbours (callers/callees).
- `graphify path "A" "B"` / `graphify affected "X"` — shortest path / reverse
  deps (available, but spottier — see the blind spot below).
- MCP server `graphify-mcp` exposes the same as agent tools (graph search, node
  details, neighbours, shortest-path); the `/graphify <question>` skill is the
  conversational front-end.

**The former `.part` blind spot is CLOSED** (review F1, ledger 104,
2026-07-02): the UI engine core (~31k lines) now lives in real
`crates/starbreaker-ui/src/<stage>/engine_NN.rs` submodules and IS indexed —
`graphify query "expand_widget_standards"` resolves to
`bb_resolve/engine_04.rs` with its cross-module call graph. Residual caveats:
- An empty graphify result is still **not proof of absence** (the workflow's
  "absence is under-research" rule) — confirm with grep before concluding.
- A method reference (`.map(foo)`) will not match a `foo(` grep pattern.

**Scope:** code STRUCTURE only. graphify does not model game-data VALUES — it is
NOT a substitute for the §4 MCP data probes or the parse-JSON / runtime-probe
verification of style/brand/font records (a graph edge is never evidence for a
colour/size claim; ledger 68 still applies). If the graph looks stale mid-arc,
`graphify update .` refreshes the code side (AST-only, free).

## 5. Data locations

| What | Where |
|---|---|
| Decompiled record mirror (grep-able authored canvases/styles/tags — the workhorse) | `~/projects/scorg_tools/ships/dcb_canvas/libs/foundry/records/` (ws); UI under `ui/buildingblocks/...` |
| Decomposed export | `~/projects/scorg_tools/ships/Packages/<Ship>_LODn_TEXm/scene.json` (ws) |
| Generated screen PNGs | `ships/Data/UI/Generated/ship/<mfr>/<Ship>/buildingblocks_canvas_*.png` (ws, refreshed by full export only) |
| Reference screenshots | `~/projects/scorg_tools/reference/in-game/Clipper/` (ws) |
| Regression manifest | `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_manifest.json` |
| IR freeze (baselines) | `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json` (schema: `crates/starbreaker-ui/docs/ir-freeze-schema.md`) |
| Known outliers | `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_known_outliers.json` |
| Measurement bank (settled reference numbers — consult FIRST, workflow §4) | `crates/starbreaker-ui/tests/fixtures/ui_ir/reference_measurements_v1.json` + `.notes.md` |
| Font baseline | `crates/starbreaker-ui/tests/fixtures/font_size_baseline.tsv` (`crates/starbreaker-ui/docs/ui-font-size-harness.md`) |
| Default-value registry + provenance | `crates/starbreaker-ui/data/default_value_registry_v1.json` + `.notes.md` |
| Ship-value derivation | `crates/starbreaker-3d/src/ui_pipeline/ship_values.rs` (+ `ship_values/tests.rs`) |
| Pipeline stages | see `crates/starbreaker-ui/docs/ui-workflow.md` §2 table; engine code in `crates/starbreaker-ui/src/<stage>/engine_NN.rs` (real submodules since review F1) |
| Fallback register | `crates/starbreaker-ui/docs/ui-fallback-register.md` |

**Screen-mesh → render aspect** (the physical/radar screen-aspect mechanism,
`crates/starbreaker-3d/src/ui_pipeline/screen_aspect.rs`; landed `cc67d79e2`).
A render-to-texture UI screen is displayed on a mesh quad; its proportions ARE
the engine's render-target aspect, so the export derives each screen's aspect
from the geometry and sizes the `physical`/`radar` target to it (`mfd` keeps the
frame-canvas 4:3 path). Hard-won facts a fresh agent needs:

- **LOD0 is required.** Plain `entity export <ship>` defaults to **LOD1**, which
  CULLS the small cockpit HUD screens (g-force, velocity ball, countermeasures,
  …) — their aspect then resolves to `None` and they render 16:9. The cockpit
  UI lives at **LOD0**; export `--lod 0` (the freeze scripts already do). The
  `SB_SCREEN_ASPECT_PROBE` (§6) flags the empty-mesh/LOD case. Watch for STALE
  scene.json: the no-`--lod` export writes the `LOD1_TEX2` package, so the
  `LOD0_TEX0` scene.json + shared `Generated/*.png` can be stale.
- **Material id, not name.** The submesh `material_name` is EMPTY in the export;
  the UI render-target is identified by `material_id` → `MtlFile` name containing
  `RTT_Screen`/`RTT_Hud` (`RTT_Text_To_Decal` / `Glass_*_RTO` are excluded). The
  screen quad maps to its binding via `submesh.node_parent_index` →
  `nmc.nodes[i].name` == the binding `helper_name`.
- **PCA in-plane, not AABB.** Screens are TILTED in the cockpit, so an
  axis-aligned bbox collapses them (gave a false 1.96 for the 5.58 annunciator).
  Use principal-axis (PCA) extents of the `RTT_Screen` vertices. Curved screens
  (radar, the 81-vert MFDs) read the chord aspect (a few % low). Blender
  ground-truth (Clipper loaded): `pts=[mw@v.co]; U,S,Vt=svd(pts-mean);
  ext=sort((pts-mean)@Vt.T max-min)[::-1]; aspect=ext[0]/ext[1]` over the
  object's `RTT_Screen` faces.
- **The freeze sources from `--lod 0`** (`generate_ui_regression_artifacts.sh`),
  so it reaches the cockpit screens regardless of a dossier row's "scene"
  column (that column is the *replay* scene). Re-freezing one cockpit screen can
  surface others changed at LOD0 — inspect the artifact dims before freezing.

**Modular-kit component sheets** (the button/scrollbar/progress chrome —
ledger 103). A canvas's style link `S_<kit>` maps to
`styles/modularkitstyles/sk_<kit>/sk_<kit>_<component>styles.json`
(buttonprimary, buttonsecondary, linearprogressmeter, scrollbar — see
`modular_kit_style_path` in `bb_resolve/engine_01.rs`; the sheets
apply at `Tier::StandardModule` AFTER the canvas cascade, with the brand Style
record as their palette). The button sheets' state entries
(`RootFilled…ElementInstance`) select on the widget standards'
`icon-/text-element-instance` tags, which the widget-standard expansion
attaches to SHOWN instance widgets only (`bb_resolve/engine_04.rs` — a hidden element
would be revealed by the kit's base `IsActive=true` entry). Colour modifiers
are either `ColorStyle` palette roles or literal `ColorSolid` RGBA — a literal
application removes any stale `<Field>Token`, else the token shadows the
literal at draw time. Inspect a sheet with
`python3 scripts/ui_canvas_query.py entries <sheet.json>` (§2).

## 6. Probe registry

Env-gated diagnostics (zero cost unless set). Add new probes to this table
in the same commit that introduces them.

| Probe | Owner | Channel | Prints |
|---|---|---|---|
| `BB_A3_STYLE_PROBE=1` | `bb_brand_apply` | stderr (`eprintln`) | per node, per cascade pass: name, style tags, matched entry names + their key modifiers (`Name[BackgroundColor=Base]`, `[IsActive=false]`, `[SizeY=0.5]`) |
| `SB_UI_STYLE_PROVENANCE=1` | `bb_brand_apply` | IR field (with `--dump-ir-dir`) | adds `UiIrNode.style_provenance` = `{field: "pass/entry"}` recording which cascade entry WON each colour / `IsActive` / `Size*` / `Anchor*` field — only modifiers actually applied, so a gate-suppressed override is NOT credited (query: `ui_ir_query.py … --fields style_provenance`). None in normal compiles, so freezes/hashes are unaffected |
| `BB_A3_TEXT_PROBE=1` | `bb_bindings` (LocalizedFromBoolean) | log (`log::info` — needs `RUST_LOG=info`) | text branch selection + resolved values |
| `BB_SHRINK_PROBE=1` | `bb_layout` flex shrink | stderr (`eprintln`) | shrink scale + each child's name/type/main-axis sizing |
| `SB_UI_GEOM_PROBE=1` | `bb_bindings::resolve_geometry_fields_into_scene` | stderr (`eprintln`) | bound SizeX/SizeY input chains + resolved values per node |
| `SB_SHIP_VALUES_DUMP=1` | `starbreaker-3d` `ship_values` | stderr (`eprintln`) | every derived registry path = value at export/replay |
| `SB_UI_FONT_DUMP=1` | `text/swf_draw` | stderr (`eprintln`) | one `FONTDUMP` line per rendered text element (see harness doc) |
| `BB_TEXT_FORMAT_PROBE=1` | `bb_brand_apply` | stderr (`eprintln`) | per pass: `TFPROBE` = text-format-route entry applications (FontSize/FillColor on tagged textfields); `TFPROBE-NORMAL` = normal-route entries carrying FontSize (with modifiers + conditions) |
| `BB_DRAW_RECT_PROBE=<1\|filter>` | `ir_compose` custom-shape draw | stderr (`eprintln`) | per asset draw: node name, laid-out `rect`, actual `raster` WxH, asset path (`1` = all; else a name/asset substring filter). For element width from layout, not dim pixels (ledger 45) |
| `SB_SCREEN_ASPECT_PROBE=1` | `starbreaker-3d` `child_payload` (per-screen aspect populate) | stderr (`eprintln`) | `SCREEN_ASPECT helper=… kind=… mesh_verts=… aspect=…` per UI binding. Diagnoses the per-screen render aspect (see §5 *screen-mesh → render aspect*): `aspect=None` + `mesh_verts=0` = empty mesh (WRONG LOD — small HUD screens are culled in LOD1, re-export `--lod 0`); `None` + non-zero verts = no `RTT_Screen` submesh on the helper node. Runs on `entity export` (the populate step) |
| `SB_SCREEN_ASPECT_ORIENT_PROBE=1` | `starbreaker-3d` `screen_aspect::ui_screen_aspect` | stderr (`eprintln`) | `SCREEN_ORIENT node=… major_ax/minor_ax=… extents=… |major.z|/|minor.z|=… long/short=… w/h=…` per UI screen. Shows the in-plane PCA axes (world-space) + the world-up (+Z) classification that decides portrait vs landscape — diagnoses the ORIENTED `w/h` aspect (`<1` portrait, e.g. the LR-indicator 0.64; `≥1` landscape). Runs on `entity export` (the populate step), same as `SB_SCREEN_ASPECT_PROBE` |
| `ui render --dump-ir-dir <dir>` | CLI flag | files (`*.ir.json`) | composed `*.ir.json` per helper (nodes, rects, payloads, tints). FAST IR inspection of a bound screen (~15s, matches the render) — prefer over `mfd_ir_dump` |

Example: `BB_SHRINK_PROBE=1 ./target/debug/starbreaker ui render --scene
"<scene>" --out-dir /tmp/x --helper Screen_Left_Lower_RTT 2>&1 | grep PROBE`.

## 7. Diagnostics (`cargo run -p starbreaker-ui --example <name> --`, plus scripts)

| Example | Use |
|---|---|
| `python3 scripts/ui_ir_query.py query <ir.json> <regex> [--fields a.b,c]` | list IR nodes whose name or text matches the regex: id, parent, type, rect, is_active + dotted-path extras (input: `ui render --dump-ir-dir` output). **Over-painter probe (ledger 63):** `query <ir.json> '.*' --fields background_fill_colour,stroke_colour` flat-lists EVERY node's fill/stroke — the fast way to find which node paints a wrong colour over the background (the compass white sheet was `CanvasProxyRoot` fill `[1,1,1,1]`) |
| `python3 scripts/ui_ir_query.py tree <ir.json> <node_id>` | ancestor chain for one node with rect, authored_size, anchor/pivot, padding, margin |
| `python3 scripts/ui_ir_query.py children <ir.json> <node_id> [--depth N] [--fields a.b,c]` | descendant subtree (rect, `right`=x+w, is_active, non-Visible overflow) — the mirror of `tree`, for clip/overflow tracing |
| `python3 scripts/ui_measure.py <image> --box x0,y0,x1,y1 [--ir <ir.json> --node <id>] [--delta N] [--anchor … --anchor-rgb …]` | glyph-run cap heights (contamination-flagged) + colour ratios with `feature_width` (warns when ≤4px that a thin feature on a RECTIFIED capture has a smeared hue — measure colour on the ORIGINAL) + optional additive-haze correction (JSON to stdout) |
| `python3 scripts/ui_measure.py --text-bands <image> [--ref <reference>]` | text-SCREEN mode (no box): bright-text bbox + `centre_x_frac` (is it centred?) + per-line cap-height bands as % of image height; with `--ref` adds `size_ratio_render_over_ref` (the resolution-independent font-scale gap — velocity-num measured 0.11 = render ~9× too small). For diagnosing blank/mispositioned/mis-sized text readouts |
| `python3 scripts/ui_gauge_measure.py <render> [reference] [--montage out.png]` | circular HUD-gauge geometry (g-force/velocity ball, countermeasures, radar): centre-dot offset + circularity (circle vs squircle), cross-arm V/H symmetry, per-cardinal ring perp-offset + radius fraction (JSON), and a centre-aligned render\|reference montage. Use abs paths — a relative `ships/…` resolves to the STALE `StarBreaker/ships/` copy. Caveat: a cardinal window can catch an adjacent diagonal marker (the cross V/H band metric is robust) |
| `ui_stage_diff <canvas.json> [WxH] [--records-root <dir>] [--filter <substr>]` | parse-only vs full-resolve layout diff; flags first name-matched divergence (cracks "which stage broke the geometry") |
| `mfd_ir_dump <canvas-guid> <content-guid> [name-filter] [WxH]` | framed MFD IR dump from the LOCAL record mirror — the no-P4K/no-MCP fallback (~5s: indexes only the UI subtrees + caches the parsed TagDatabase; prints index/compile timing). When P4K/MCP is available prefer `ui_ir_query` (canvas pair) or `ui render --dump-ir-dir` (bound screen). NOTE: uses anim sample 0; `ui render` uses 50 (the render), so a few state-dependent rects differ |
| `query_ui_layout --canvas-guid <guid> --query <pattern>` | per-node layout/draw/text rects + drawn glyph bounds |
| `bb_layout_wireframe <fixture.json> <out.png> [--merge]` | wireframe overlay of layout rects |
| `phase5_certification_dashboard` | representative-family certification table (CI) |
| `freeze_ui_snapshot_ir` | driven by `scripts/freeze_ui_snapshot_ir.sh` |

## 8. Glossary

- **platinum / gold**: regression tiers (pixel-diff budget 0.5% / 1%;
  structural thresholds tighter for platinum). Targets live in the
  manifest; baselines in the IR freeze.
- **known-outlier**: reference-anchored one-sided override for a knowingly
  off element (workflow §6).
- **derivation vs pin**: per-ship value computed from DataCore at export vs
  an at-rest engine-pushed value recorded in the registry with provenance.
- **replay**: `ui render --scene` re-render of an existing export — same
  pipeline + derived values, ~6× faster than exporting.
- **dcb_canvas mirror**: local decompiled DataCore record tree (grep-able
  authored JSON).
- **widget-standard expansion**: bb_resolve instantiation of component
  templates (buttons, icons, scrollbars) from `*componentstandard` records.
- **param relay / slot broadcast**: parent→child ComponentParameter wiring;
  a slot name may have MULTIPLE live defs (all get injected) and plain
  un-namespaced canvas hops relay onward.
- **host-stage scale**: MFD frame path multiplies FontSize by
  max(target/stage) of `BuildingBlocks_root.swf` (1280×720 → ×1.667 at
  1600×1200); geometry does NOT scale.
- **BB_ColorStyle slot order**: authoritative enum index (Base=0 …
  Bright=6, Selected=7, Disabled=8…) — see `crates/starbreaker-ui/docs/ui-architecture-runbook.md`.
- **LOD0 / LOD1 scene**: cockpit MFDs vs interior usables (dossier §3).
- **runtime-image hologram**: a `BuildingBlocks_WidgetRuntimeImage` /
  `rendererType:"Primitive"` node (e.g. SELF-STATUS "Own Vehicle Hologram") is a
  live 3D-scene render the engine composites into the UI — NOT a UI asset (the
  compositor only handles `Flash`/`None` rendererTypes). Reproduced headlessly by
  a CPU mesh rasteriser (`starbreaker-gfx::mesh_holo::render_vehicle_hologram` —
  yaw/pitch + perspective, painter's-order shaded filled faces, greyscale × tint)
  fed by a `HologramFetcher` trait on `PipelineInputs`→`ComposeContext` (impl
  `starbreaker-3d::ui_pipeline::hologram_fetcher::P4kHologramFetcher`: resolves the
  render scene's ROOT vehicle hull `.cga` via `SGeometryResourceParams`,
  `parse_skin`→mesh). The tint is the node's authored `background_fill_colour` (the
  per-manufacturer holo colour). Reuse this fetcher-trait pattern for any
  engine-3D-into-UI node. Gotcha: `Mesh.model_min/max` (bbox print) misreports the
  axes — confirm orientation by rendering, e.g. the `holo_preview` example. Ledger 70.
