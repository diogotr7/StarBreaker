# UI Parity Process + Crate Convergence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut the token/wall-clock cost of UI parity arcs run by Opus/Sonnet agents (Workstream A), then converge the `starbreaker-ui` crate on the in-game engine model via four sequenced architecture items (Workstream B).

**Architecture:** A builds tooling first (machine-readable dossier, one-command loop cycle, variant-style probe, blocker-evidence validator), then restructures the parity skill into a lean core + phase-scoped references, then validates with an Opus dry-run and the Anvil Carrack arc. B lands pixel-neutral refactors first and the everything-changes linear-light re-freeze last. Spec: `docs/superpowers/specs/2026-07-04-ui-parity-process-and-crate-design.md`.

**Tech Stack:** bash + python3 (scripts), Rust (MCP server `mcp/`, crate `crates/starbreaker-ui/`), Claude skill markdown (`.claude/skills/starbreaker-ui-screen-parity/`).

## Global Constraints

- **GATE: no task in this plan executes until the owner's explicit go** (another agent is mid-arc in this repo). The plan document itself is the only pre-go artifact.
- No hard-coding: no name/ship/screen/manufacturer branches, no magic offsets, **no hard-coded game-data values** (palette literals, font sizes, brand lists) in production code, fallbacks, or fixtures. Data files transcribed from authoritative docs get a `.notes.md` provenance sidecar (registry pattern).
- Process/tooling changes must NOT alter render behaviour (ledger rule). `bash scripts/ui_check.sh` green before every commit.
- Validation scripts print a RESULT MARKER (`<tool>: OK …` / `<tool>: FAILED (exit N)`) via an EXIT trap; never rely on a piped exit code.
- TDD: failing test/self-check first for every behavioural change.
- Commits directly on `feature/ui`; no new branches/worktrees. One commit per coherent task. Never name the maintainer (use "the owner"); no `/home/<user>` paths in repo content (`$HOME`/`~` only).
- Sub-agents: read-only research only, default model **Opus**; anything touching cargo target/build/render/test runs sequentially in the main session.
- Verify-on-write: every command line added to a doc is run once at writing time; renames include a repo-wide reference grep in the same commit.
- After doc-file changes land, run the graphify doc sync (`/graphify . --update`) once at the end of the phase.

---

## Phase A — matching process

### Task A0: Confirm the gate is open

**Files:** none.

- [ ] **Step 1:** Confirm the owner has said go for this plan, and `git log --oneline -5` shows no other agent mid-arc (or the owner confirms the other arc is done/paused). If not confirmed, STOP.

### Task A1: Machine-readable screen dossier + drift validator

**Files:**
- Create: `crates/starbreaker-ui/data/ui_screen_dossier_v1.json`
- Create: `crates/starbreaker-ui/data/ui_screen_dossier_v1.notes.md`
- Create: `scripts/validate_ui_dossier.py`
- Modify: `scripts/ui_check.sh` (repo-only tier: call the validator)

**Interfaces:**
- Produces: `ui_screen_dossier_v1.json` — schema below; consumed by Tasks A2/A3 and by agents. `scripts/validate_ui_dossier.py [--self-test]`, exit 0 + `validate_ui_dossier: OK (N screens)` on success.

Schema (top-level object):

```json
{
  "version": 1,
  "screens": [
    {
      "screen_id": "Screen_Right_Upper_RTT",
      "ship_folder": "Clipper",
      "reference_file": "Screen_Right_Upper_RTT.png",
      "scene_package": "DRAK Clipper_LOD0_TEX0",
      "helper": "Screen_Right_Upper_RTT",
      "lod": 0,
      "canvas": "MC_S_Target_Master",
      "preset": "target",
      "tier": "GOLD",
      "target_id": "clipper_target_master",
      "open_issues": "A7 backdrop stack remainder (handoff)"
    }
  ]
}
```

`scene_package` resolves to `$HOME/projects/scorg_tools/ships/Packages/<scene_package>/scene.json`. `preset`/`tier`/`target_id` may be `null` where §3 has `— (add)`.

- [ ] **Step 1: Write the failing self-test.** Create `scripts/validate_ui_dossier.py` with only the `--self-test` scaffold (asserts a malformed fixture is rejected and a well-formed one accepted) and a `main()` that exits 2 "not implemented". Run `python3 scripts/validate_ui_dossier.py --self-test` → expect FAILED marker.
- [ ] **Step 2: Transcribe the dossier.** Read `crates/starbreaker-ui/docs/ui-reference.md` §3 dossier table (every row, including the Carrack lift-call console rows added 2026-07-03/04) and write one JSON entry per row into `ui_screen_dossier_v1.json`. Where the table row does not carry the scene package explicitly, derive it from the Helper/scene column ("LOD0 scene" → the ship's `*_LOD0_TEX0` package; "LOD1 scene" → `*_LOD1_TEX2` for Clipper) and cross-check against `ls ~/projects/scorg_tools/ships/Packages/`. Carrack packages: `ANVL Carrack_LOD0_TEX0` / `ANVL Carrack_LOD0_TEX2`.
- [ ] **Step 3: Write the provenance sidecar** `ui_screen_dossier_v1.notes.md`: source = ui-reference §3 (transcription date, commit), plus the rule that §3's table and this JSON must not drift (the validator enforces it) and that new screens add BOTH.
- [ ] **Step 4: Implement the validator.** `validate_ui_dossier.py`: (a) JSON schema check (required keys, `lod` ∈ {0,1}, `tier` ∈ {PLATINUM, GOLD, null}); (b) parse the §3 markdown table and cross-check per screen: `screen_id`, `helper`, `preset`, `tier`, `target_id` must match; extra/missing screens on either side fail; (c) check each `scene_package` directory exists (warn, don't fail, when the ships tree is absent — repo-only CI has no game data); (d) EXIT-trap RESULT MARKER. Run `--self-test` → PASS; run for real → `validate_ui_dossier: OK (N screens)`.
- [ ] **Step 5: Wire into `ui_check.sh`** repo-only tier (beside the existing repo-only validators). Run `bash scripts/ui_check.sh` → `ui_check: ALL GREEN`.
- [ ] **Step 6: Commit** `feat(ui-process): machine-readable screen dossier v1 + drift validator (plan A1)`.

### Task A2: `ui_render.sh` — dossier-driven, unique output names

**Files:**
- Modify: `scripts/ui_render.sh` (scene/LOD block ≈ lines 24–66; OUT block ≈ lines 66–85)

**Interfaces:**
- Consumes: `ui_screen_dossier_v1.json` (Task A1).
- Produces: `bash scripts/ui_render.sh --helper <name> [--screen <screen_id>] [--scene|--lod|--out|--ir]`; renders to `/tmp/ui_render/<helper>/<UTC-stamp>/`, maintains `/tmp/ui_render/<helper>/latest` symlink, prints `png md5:` lines (unchanged). Unknown helper with no dossier row and no `--scene` → hard error naming the dossier file.

- [ ] **Step 1: Failing check.** Run `bash scripts/ui_render.sh --helper mesh_anvl_crk_console_liftcall_009`-style Carrack helper (any dossier row whose ship is not Clipper) — observe it wrongly resolves a Clipper scene or errors. Record the wrong behaviour in the commit message.
- [ ] **Step 2: Replace the hard-coded scene constants and helper case-list** with a dossier lookup (python3 inline, since the repo standardises on python3). Add `--screen) SCREEN_ID="$2"; shift 2 ;;` to the arg loop (default `SCREEN_ID=""`), then:

```bash
# dossier lookup: helper (or --screen) → scene_package + lod
DOSSIER="crates/starbreaker-ui/data/ui_screen_dossier_v1.json"
if [[ -z "$SCENE" ]]; then
    read -r PKG DLOD < <(python3 - "$DOSSIER" "$HELPER" "$SCREEN_ID" <<'PY'
import json,sys
d=json.load(open(sys.argv[1])); h=sys.argv[2]; sid=sys.argv[3]
rows=[s for s in d["screens"] if s["helper"]==h or (sid and s["screen_id"]==sid)]
if not rows: sys.exit(3)
s=rows[0]; print(s["scene_package"], s["lod"])
PY
) || { echo "error: helper '$HELPER' not in $DOSSIER — add its row (and ui-reference §3) or pass --scene" >&2; exit 2; }
    [[ -n "$LOD" ]] || LOD="$DLOD"
    SCENE="$HOME/projects/scorg_tools/ships/Packages/$PKG/scene.json"
fi
```

  Keep `--scene`/`--lod` as explicit overrides (`--lod` alone without a dossier row is no longer meaningful and errors).
- [ ] **Step 3: Unique outputs.** `STAMP=$(date -u +%Y%m%dT%H%M%SZ)`; default `OUT="/tmp/ui_render/$HELPER/$STAMP"`; after a successful render `ln -sfn "$OUT" "/tmp/ui_render/$HELPER/latest"`. Keep the md5 print and binary-mtime print. Drop the `rm -rf "$OUT"` for the default path (each run is fresh); keep it when `--out` is explicit.
- [ ] **Step 4: Verify.** Two consecutive runs of a Clipper helper produce two distinct directories + `latest` pointing at the second; the Carrack helper from Step 1 now resolves `ANVL Carrack_*/scene.json`. `bash scripts/ui_check.sh` green.
- [ ] **Step 5: Update docs** that cite the old flags/paths (`ui-reference.md` §2, architecture runbook "Fast Render Iteration"): repo-wide grep `ui_render.sh` and fix each citing line; run each edited command once (verify-on-write).
- [ ] **Step 6: Commit** `feat(ui-process): dossier-driven ui_render.sh with unique output dirs (plan A2)`.

### Task A3: `ui_arc_status.sh` — one command per loop cycle

**Files:**
- Create: `scripts/ui_arc_status.sh`
- Create: `scripts/ui_region_summary.py`
- Modify: `scripts/ui_compare.py` (add `--json <path>`)

**Interfaces:**
- Consumes: upgraded `ui_render.sh` (A2), dossier (A1), `scripts/ui_compare.py` presets/`--stats`, measurement bank `crates/starbreaker-ui/tests/fixtures/ui_ir/reference_measurements_v1.json`.
- Produces: `bash scripts/ui_arc_status.sh --screen <screen_id> [--no-render]` → renders (unique dir), compares against the dossier's reference with the dossier's preset, writes machine state to `/tmp/ui_arc_status/<screen_id>/{prev.json,cur.json}`, prints: export-stamp age (info), per-region table (bright/dark means, R-normalised ratios, bank targets where present), `CHANGED`/`NEW`/`same` flag per region vs the previous cycle, crop paths for flagged regions only, and marker `ui_arc_status: OK (N regions, M changed)` / `ui_arc_status: FAILED (…)`.

- [ ] **Step 1: `ui_compare.py --json`.** Failing check first: run with `--json /tmp/x.json` → argparse error. Then add the flag: dump the per-region stats it already computes (region name, boxes, bright/dark means per channel, ratios, crop file paths) as JSON. Re-run → file exists, fields present. This is pure output plumbing — assert (by diffing stdout) that a `--stats` run with and without `--json` prints identical console output.
- [ ] **Step 2: `ui_region_summary.py`.** Reads `cur.json` (+ optional `prev.json` + measurement bank), emits the table + flags. Change thresholds: flag `CHANGED` when any regional mean channel moves > 1.0 (0–255) or any ratio moves > 0.01 vs prev; regions absent from prev are `NEW`; first cycle flags all. Bank rows matched by the dossier `target_id`. Self-test mode `--self-test` with two inline synthetic stats dicts (one changed, one same) asserting the flags — run it failing first (script exits 2 unimplemented), then implement → PASS.
- [ ] **Step 3: `ui_arc_status.sh`.** Bash orchestration: resolve the dossier row (same python3 lookup pattern as A2, by `screen_id`); render via `ui_render.sh` unless `--no-render` (then use `latest`); locate the reference `$HOME/projects/scorg_tools/reference/in-game/<ship_folder>/<reference_file>` (prefer the `corners.json`-carrying variant exactly as ui-reference §3 describes); run `ui_compare.py … --regions <preset> --stats --json`; rotate `cur.json`→`prev.json`; call `ui_region_summary.py`; EXIT-trap marker. A dossier row with `preset: null` fails loudly: "add a preset for this screen first (ui_compare presets)".
- [ ] **Step 4: Verify end-to-end** on a settled Clipper screen (e.g. `Screen_Right_Upper_RTT`): first run flags all regions; immediate second run with `--no-render` flags none. `bash scripts/ui_check.sh` green (no behaviour change to render/tests).
- [ ] **Step 5: Document** in `ui-reference.md` §2/§3 (one short block: the loop cycle is `ui_arc_status.sh`, agents vision-read only flagged crops; verify-on-write).
- [ ] **Step 6: Commit** `feat(ui-process): ui_arc_status.sh one-command loop cycle + ui_compare --json (plan A3)`.

### Task A4: `ui_variant_styles` MCP tool

**Files:**
- Create: `mcp/src/ui_variant_styles.rs`
- Modify: `mcp/src/tools.rs` (register the tool; mirror the `ui_scene_style_probe` wiring at `mcp/src/tools.rs:1609`)
- Test: in-module `#[cfg(test)]` mirroring `ui_scene_style_probe_reports_applied_entries` (`mcp/src/tools.rs:3990`)

**Interfaces:**
- Consumes: the same canvas/scene loading + style plumbing `ui_scene_style_probe` uses (starbreaker-ui `bb_style_engine` / `ui_ir` APIs).
- Produces: MCP tool `ui_variant_styles` — request `{ canvas_or_helper: String, node_pattern: String, manufacturer: Option<String> }`; response per matched node: the instantiated variant's authored entries from `defaultStyles`, `brandStyles`, canvas-root `embeddedStyles` (including bare `Type(Text)` selectors) that match the node, each with `{tier, selector, fields, applied: bool}` (applied = the value is present in the node's effective IR `text_style`/style fields), plus the effective IR values. This is the "authored-but-unapplied font" drill (4× dissolved blocker) as one call.

- [ ] **Step 1: Failing test.** In the new module, a test that loads the same fixture canvas the existing probe tests use, calls `ui_variant_styles` for a text node known to have an authored `Type(Text)` entry, and asserts the response contains that entry with an `applied` verdict. Run `cargo test -p starbreaker-mcp ui_variant_styles -- --nocapture` → FAIL (unresolved symbol).
- [ ] **Step 2: Implement.** New module: request/response structs (serde), a function that (a) resolves the canvas exactly as `ui_scene_style_probe` does — reuse its loader, do not duplicate it: extract a shared helper in `tools.rs` if needed; (b) collects authored entries per tier for nodes matching `node_pattern` (regex on node name/path); (c) compiles the IR for the canvas and compares each authored field value against the node's effective IR value → `applied`; (d) returns JSON text. Register in `tools.rs` beside the other `ui_*` tools (new code goes in the new module — `tools.rs` is already 4,805 lines; do not grow it beyond registration glue).
- [ ] **Step 3: Test passes.** `cargo test -p starbreaker-mcp` all green.
- [ ] **Step 4: Deploy** per AGENTS.md §MCP: `pkill -f starbreaker-mcp || true && cargo build --release -p starbreaker-mcp && cp target/release/starbreaker-mcp mcp/starbreaker-mcp`. Note in the commit message that the client needs a restart to pick it up.
- [ ] **Step 5: Document** in `ui-reference.md` §4 (MCP tools, WHEN-to-use: "font/size/colour looks wrong → run this FIRST") — verify-on-write with a real invocation via the restarted client, or mark the doc line as verified-post-restart in the commit.
- [ ] **Step 6: Commit** `feat(mcp): ui_variant_styles — authored-vs-applied style drill for parity arcs (plan A4)`.

### Task A5: Blocker-evidence validator

**Files:**
- Create: `scripts/ui_blocker_evidence.py`
- Create: `crates/starbreaker-ui/docs/blocker-evidence-schema.md` (template + example)

**Interfaces:**
- Produces: `python3 scripts/ui_blocker_evidence.py <evidence.json>` → `ui_blocker_evidence: OK` only when every required surface carries ≥1 `{query, result}` pair with non-empty strings; `--self-test` mode. Evidence schema:

```json
{
  "item": "compass live ticks",
  "surfaces": {
    "datacore_families": [{"query": "search_records(\"vehiclehud\")", "result": "SVehicleHudParams.compassTape — FOUND"}],
    "p4k_assets": [{"query": "p4k_search …", "result": "…"}],
    "localization": [{"query": "…", "result": "…"}],
    "derivable_mechanisms": [{"query": "screen-mesh aspect check …", "result": "…"}]
  },
  "conclusion": "blocked | found"
}
```

- [ ] **Step 1: Failing self-test** (missing surface → reject; complete file → accept). Run → FAILED marker.
- [ ] **Step 2: Implement** (schema check, all four surfaces required, non-empty query+result strings, EXIT-trap marker). Self-test → PASS.
- [ ] **Step 3: Write `blocker-evidence-schema.md`**: the schema, one worked example (the compass case from ledger 66 — the family search that FOUND the "blocked" value), and the rule: a blocker claim presented at the major-item gate MUST attach a file this script accepts.
- [ ] **Step 4: Commit** `feat(ui-process): blocker-evidence schema + validator (plan A5)`.

### Task A6: Skill restructure — lean core + phase references

**Files:**
- Modify: `.claude/skills/starbreaker-ui-screen-parity/SKILL.md` (552 → ~200 lines)
- Create: `.claude/skills/starbreaker-ui-screen-parity/references/{launch,catalog,blockers,freeze,retro}.md`
- Modify: `.claude/skills/starbreaker-ui-screen-parity/recommendations.md` (Change log entry; clear the two folded 2026-07-03 candidates)

**Interfaces:**
- Consumes: tools from A1–A5 (the core's loop checklist invokes `ui_arc_status.sh`, `ui_variant_styles`, `ui_blocker_evidence.py`).
- Produces: the restructured skill; **guardrail-preservation invariant**: every removed SKILL.md rule/red-flag row maps to a destination (reference file or tool) — the mapping table below is the audit.

Relocation map (destination for every current SKILL.md section/row — rows quoted by their opening words):

| Current SKILL.md content | Destination |
|---|---|
| Overview / core principle / worked example | core (compressed) |
| Operating posture (4 categories + marker/parse-JSON caveats) | core, ~10 lines |
| Gather inputs steps 1–4 + ≥2-options padding + sequential-questions rule | `references/launch.md` |
| Build & confirm catalog (self-verify, background layer, user gate, diagnose-vs-land) | `references/catalog.md` |
| Autonomous loop bullets | core loop checklist (rewritten around `ui_arc_status.sh`) |
| Default to fixing / blocker proof bar / font drill / instantiated-variant re-read | `references/blockers.md` (font drill now starts: run `ui_variant_styles`) |
| Shared-mechanism rules (asset/icon/binding + **layout/render formula**, `--full` fresh-export, sibling eyeball) | `references/blockers.md` + one core loop line |
| Subagents section (read-only, Opus default, don't-override-with-weaker-check) | core, 6 lines |
| Checkpoints table + freeze bullets (dry-freeze no-op check) | core keeps the table; detail → `references/freeze.md` |
| Strict rules | core, compressed (no rule dropped) |
| Closing re-review | `references/catalog.md` (same procedure, closing variant) + core STOP line |
| Retrospective (7 categories, two destinations) | `references/retro.md` + core STOP line |
| Red-flag rows: presume/reuse inputs; batch dependent questions | `references/launch.md` |
| Red-flag rows: first-glance/background; findings-look-right; memory-says-faithful | `references/catalog.md` |
| Red-flag rows: hard-code one value; invent geometry; render-differs-so-render-wrong; blast-radius defer; engine-C++-only; whack-a-mole scoping; font-too-small (both causes); grep-line-range; graphify-empty; refute-subagent-cheaply | `references/blockers.md` |
| Red-flag rows: freeze-to-pass / auto-freeze / surely-drifts | `references/freeze.md` |
| Red-flag rows: retro-as-wrap-up; landed-fix-checkpoint; ask-before-fixing; parallel-builds; looks-identical (now tool-killed by unique filenames — REMOVE, note tool in map); skill-summary-is-enough | core (~8-row table); the looks-identical row is deleted with `Removed — enforced by ui_render.sh unique dirs (A2)` recorded in recommendations.md |

- [ ] **Step 1: Draft the new core** using the skeleton below (adapt wording, keep ≤220 lines), then the five reference files per the map. Fold in the two open 2026-07-03 candidates (layout-formula = shared mechanism; "instrument RESOLVED geometry via a throwaway `#[test]` + `eprintln`, don't hand-solve the op graph" → `references/blockers.md`) AND the spec's interim rule: until the linear-light arc (B4) lands, blend-shaped colour residuals are judged leniently and registered as known-outliers pointing at that arc → `references/blockers.md`.

```markdown
---
name: starbreaker-ui-screen-parity
description: (unchanged)
---
# UI Screen Parity
Get ONE screen's render as close to its in-game reference as the reference
allows — engine-faithfully, generically. This skill orchestrates
ui-workflow.md (how to work) + ui-reference.md (what to type); read their
header indexes first, then sections on demand. Fix the owning UPSTREAM
stage; never hard-code, never invent geometry; captures are imperfect —
match structurally.

## Operating posture (the four defaults)
- Inputs → ASK (AskUserQuestion, fresh every run).  Judgments → MEASURE
  (probes, ui_arc_status.sh, parse JSON — never line-grep a big record).
- The work → JUST DO IT (fix without asking).  Hard-to-reverse → GATE.
- Read result MARKERS (`…: ALL GREEN` / `…: OK`), never piped exit codes.

## Phases
1. LAUNCH — STOP: read references/launch.md. SHIP → SCREEN → REFERENCE →
   SCOPE&MODE, sequential AskUserQuestions.
2. CATALOG — STOP: read references/catalog.md. Build via ui_arc_status.sh;
   self-verify; background layer; user confirms catalog (gate, both modes).
3. LOOP (priority order, autonomous):
   a. bash scripts/ui_arc_status.sh --screen <id> → vision-read FLAGGED crops only
   b. owning stage: MCP trio (styles) / graphify (code) / stage table below
   c. font/size/colour wrong? → ui_variant_styles MCP tool FIRST
   d. TDD failing test → ONE structural fix → bash scripts/ui_check.sh → (a)
   e. shared mechanism (asset/icon/binding/LAYOUT-FORMULA)? → fresh export
      + --full + eyeball sibling screens
   f. guard trip → workflow §5. Tempted to defer/blocked? STOP: read
      references/blockers.md (evidence file validated by
      ui_blocker_evidence.py; major items are user-gated)
   g. landed fix ≠ checkpoint: take the next catalog item.
4. CLOSE — re-review from scratch (fresh render+compare, look-again,
   background). Fully-auto: fix until clean or proven-blocked. Semi:
   final-parity gate.
5. FREEZE/COMMIT — STOP before ANY freeze: read references/freeze.md
   (dry-freeze first; freezes are ALWAYS user-gated, both modes).
6. RETRO (mandatory, TodoWrite item from arc start) — STOP: read
   references/retro.md.

## Stage table
| wrong thing | owning stage |
| node exists/authored fields/styles | bb_resolve / bb_style_engine / bb_state_filter |
| values (text/numbers/geometry bindings) | bb_bindings |
| rects | bb_layout |  | surviving metadata/font px | ui_ir |
| final draw | ir_compose + text/ | (details: workflow §2)

## Gates (unchanged table)
| Checkpoint | Semi | Fully |
| Reference selection / catalog confirm | ask | ask |
| Freeze / re-freeze | GATE | GATE |
| Major-item blocker | GATE | GATE |
| Commit | gate | auto |
| Final parity | gate | re-review→fix until clean |

## Strict rules (non-negotiable)
- No hard-coding: names, offsets, blend factors, GAME-DATA VALUES
  (fallbacks + fixtures included). Self-correcting: replace or flag
  pre-existing offences in the same change.
- Reproduce from the REAL decoded asset (exhaust textures/svgPath/
  styleTags/SWF; per-manufacturer via the cascade-applied override);
  "gated off in IR" ≠ absent — activate, never generate.
- IR is the sole styling authority; fix upstream, not draw-time.
- Frozen baselines change only via the audited freeze flow / §6 outlier.
- 3000-line cap; revert no-effect experiments immediately.
- Subagents: read-only research only, model=Opus; builds/renders/tests/
  fixes stay sequential in the main agent.

## Core red flags
(~8 rows: skill-summary-is-enough; ask-before-fixing; landed-fix-as-
checkpoint; retro-as-wrap-up; parallel builds; freeze-without-gate;
defer-on-estimate; presume-inputs)

## Pointers
workflow §5 guard trips · §6 outliers · §7 freezes · reference §3 dossier
(machine mirror: crates/starbreaker-ui/data/ui_screen_dossier_v1.json) ·
ledger for history · recommendations.md for skill findings.
```

- [ ] **Step 2: Audit the map.** Diff old SKILL.md against new core + references: for every deleted line, name its destination row (or the enforcing tool). Any orphan = fix before commit. Record the completed audit in `recommendations.md`'s Change log (dated entry; clear the two folded open items).
- [ ] **Step 3: Invoke `superpowers:writing-skills`** to review the restructured skill against its checklist (skills evolve; this is the mandated skill for skill edits). Apply its findings.
- [ ] **Step 4: Verify** references resolve: every `references/*.md` STOP pointer names an existing file; every command line in the new files runs (verify-on-write); `wc -l SKILL.md` ≤ 220.
- [ ] **Step 5: Commit** `refactor(skill): starbreaker-ui-screen-parity — lean core + phase references, guardrail map audited (plan A6)`.

### Task A7: Doc read-WHEN indexes

**Files:**
- Modify: `crates/starbreaker-ui/docs/ui-workflow.md` (prepend index)
- Modify: `crates/starbreaker-ui/docs/ui-reference.md` (prepend index)
- Modify: `crates/starbreaker-ui/AGENTS.md` (required-reads contract: indexes first, sections on demand)

- [ ] **Step 1:** Prepend to each doc a ~10-line "Read WHEN" index. ui-workflow: §1 always (rules) · §2 when locating a stage · §3–4 during the loop · §5 on a guard trip · §6–7 before any freeze/outlier · §8 when pinning values · §9 at pause/handoff · §10 before retrying anything weird. ui-reference: §1–2 when building/rendering · §3 at launch (dossier) + any comparison · §4/4b before style/code research · §5 when hunting data · §6 before adding a probe · §7 diagnostics · §8 glossary on demand.
- [ ] **Step 2:** Update the crate AGENTS.md required-reads paragraph: read both indexes up front; read full sections when the index or the skill's phase directs. (Do not weaken: §1 non-negotiable rules stay a full read.)
- [ ] **Step 3:** `bash scripts/ui_check.sh` green (docs_reference_guard); commit `docs(ui): read-WHEN indexes for workflow/reference; required-reads contract (plan A7)`.

### Task A8: Compliance dry-run (Opus subagent) — gated validation part 1

**Files:** none (findings → fix commits against A6/A7 files).

- [ ] **Step 1:** Dispatch a fresh **Opus** subagent (read-only; no builds/renders/edits) with exactly: "You are launching a UI screen parity arc for SHIP `Drake Clipper`, SCREEN `Screen_Right_Upper_RTT`. Follow `.claude/skills/starbreaker-ui-screen-parity/SKILL.md`. Narrate each step you would take, each file/section you read and why, each command you would run, and every point where you must stop and ask the user. Do not execute builds, renders, or edits — narrate them."
- [ ] **Step 2:** Score against the acceptance checklist: (a) resolves `reference/in-game/Clipper/Screen_Right_Upper_RTT.png` and gates on user confirmation; (b) reads the two doc indexes + dossier row (`MC_S_Target_Master`, preset `target`, GOLD `clipper_target_master`); (c) opens `references/launch.md` then `references/catalog.md` at the right phases; (d) loop cycle = `ui_arc_status.sh` + flagged-crops-only; (e) names `ui_variant_styles` for a font symptom; (f) stops at freeze/major-blocker gates; (g) knows the retro is mandatory. 
- [ ] **Step 3:** Every miss is a skill bug: fix in SKILL.md/references (not in the prompt), re-run the dry-run until clean. Commit fixes `fix(skill): dry-run findings (plan A8)`.

### Task A9: Graphify doc sync + ledger entry

- [ ] **Step 1:** Append a numbered ledger item to `crates/starbreaker-ui/docs/ui-process-improvements.md` (Observed/Improvement/Action): the parity-process overhaul (A1–A8), citing this plan + the spec.
- [ ] **Step 2:** Run the graphify doc sync (`/graphify . --update`) so the changed docs enter the knowledge graph. Commit `docs(ui): ledger — parity process overhaul landed (plan A9)`.

### Task A10: Execution test — Anvil Carrack arc (gated; run via the skill, not this plan)

- [ ] **Step 1:** With the owner's go, launch a NEW session on **Opus** and invoke `StarBreaker:starbreaker-ui-screen-parity`. Expected inputs when asked: SHIP = Carrack, SCREEN = the unworked screen (today `reference/in-game/Carrack/` holds `Screen_Annunciator.png` and the lift-call console; the unworked one is `Screen_Annunciator`) — but ASK per the skill; do not pre-answer. Adding its dossier row (JSON + §3, validator green) is part of the arc.
- [ ] **Step 2:** The arc's mandatory retro is the measurement: bootstrap lines read before first render (~600 target), tool calls per loop cycle (1–2 target), guardrail behaviour (all gates fired). Findings → ledger + recommendations.md as usual.

---

## Phase B — ui crate convergence (each item = its own arc + sub-plan)

Sequencing is fixed: B1 → B2 → B3 → B4 (pixel-neutral refactors first; the everything-changes re-freeze last). Each task below starts with research and produces a sub-plan via `superpowers:writing-plans` before any code: their step-level code depends on the codebase state when reached, and pretending otherwise now would write placeholders.

### Task B1: One brand-context resolver

- [ ] **Step 1 (research):** Fan read-only Opus subagents (graphify `explain`/`query` + targeted reads) over the four selection paths: `resolve_brand_style` (manufacturer-prefix scan, `bb_brand_style.rs`), `collect_standard_text_styles` (`selected_style_name` family mapping), the body-background preferred chain, the separator `hud`↔`env` sibling swap. Deliverable per path: file:line, inputs, output container, divergences.
- [ ] **Step 2 (sub-plan):** `docs/superpowers/plans/<date>-one-brand-context-resolver.md` — one resolver (canvas style-link → `s_<mfr>_{hud|env}` by canvas family → sibling swap; identity matching only, no prefix scans over shared standards), migrating ONE call site per task/commit.
- [ ] **Step 3 (execute):** per migration: failing characterization test → migrate → `ui_check.sh` → disable→adjudicate audit + fresh export + `--full`. Target: zero pixel change; any drift is a stop-and-diagnose, never a re-freeze-to-pass.

### Task B2: Binding/state fidelity (three guarded fixes)

- [ ] **Step 1:** Per-host-type expansion **ID-band lanes** in `merge_child_scene` (design already sketched in the runbook's Open architecture debt: a lane per host type or a second band `0xF800_0000`). TDD: a test that adding a new host type does NOT shift existing expansion IDs (the medical close-X theft case is the characterization). Unblocks the parked separator-dots work.
- [ ] **Step 2:** `bb_state_filter` derived numeric at-rest values, scoped to COMPONENT-LOCAL bare bindings only (ledger 87/88 — global slash-paths regress frozen baselines; that scoping is the load-bearing discriminator and gets its own test).
- [ ] **Step 3:** Text-format route at ALL cascade tiers (ledger 96 covered Brand+Embedded): enumerate tiers from `ui-cascade-passes.md`, characterization test per remaining tier, then the generic fix. `--full` after fresh export (auto-canvas drift is whole-image-only).
- [ ] **Step 4:** Each lands as its own commit with its regression guard; sub-plan first if any step exceeds one-commit scope.

### Task B3: Flash/SWF hybrid completion

- [ ] **Step 1 (research):** Recover the approved phased plan (memory `flash-hybrid-rendering-plan`; architecture in memory `clipper-mfd-hybrid-flash-architecture` + runbook AVM1 section + `hybrid_compose.rs`, `swf_assets/`, `swf_render/`): what already landed, what remains (Furore font from the `fonts_en` SWF, MFD footer in frame chrome, 4:3 frame vs 16:9 content composition).
- [ ] **Step 2 (sub-plan + execute):** TDD per remaining phase, generic (no screen gating); footer/font work re-renders ALL MFD screens + sibling eyeball (shared-mechanism rule).

### Task B4: Linear-light compositing (dedicated owner-gated arc — LAST)

- [ ] **Step 1 (pre-arc):** Confirm B1–B3 landed and `ui_check.sh --full` green on a fresh export. Re-read the runbook's gated entry (evidence: predicted linear blends land on reference values at the chiclet edges).
- [ ] **Step 2 (sub-plan):** renderer-wide linear blend in `ir_compose`/`text` — NO carve-outs (image-only exceptions are explicitly not engine-faithful); conversion boundaries (sRGB→linear at ingest, linear→sRGB at output); perf note (the blend runs per pixel — measure with `ui-perf-baseline.md`'s method).
- [ ] **Step 3 (execute + re-freeze):** land behind the full protocol: fresh export → whole-image + IR snapshot deltas for EVERY gold/platinum target → per-identity adjudication vs references (expect movement TOWARD captures) → owner freeze gate with the complete delta table → `ui_freeze_cycle.sh --approver owner --reason "linear-light compositing migration"` → both validators + `--full`. Interim known-outliers registered against this arc (per the spec) are graduated or closed here.

---

## Self-review record

- Spec coverage: A1↔spec A1.1, A2↔A1.2, A3↔A1.3, A4↔A1.4, A5↔A1.5, A6↔A2, A7↔A3, A8/A10↔A4, B1–B4↔spec B1–B4, gating↔Global Constraints + A0. Interim colour-leniency rule lives in the spec and `references/blockers.md` (A6 map).
- No placeholders: B tasks' research-first shape is deliberate phased planning (stated up front), not deferral; every A step carries concrete content.
- Type consistency: dossier field names (`screen_id`, `scene_package`, `helper`, `lod`, `preset`, `target_id`) are used identically in A1/A2/A3; marker format `<tool>: OK|FAILED` consistent across A1/A3/A5.
