---
name: starbreaker-ui-screen-parity
description: Use when getting a StarBreaker ship UI screen's render to match its in-game reference capture — e.g. "make Drake Clipper Screen_Right_Upper_RTT match its reference". Triggers: UI parity arc, screen render vs reference/in-game/<Ship>/<Screen>.png, closing visual gaps on cockpit/MFD/HUD/medical/door screens.
---
# UI Screen Parity

Get ONE screen's render as close to its in-game reference as the reference
allows — engine-faithfully, generically. This skill ORCHESTRATES the
authoritative process; it does not replace it: `ui-workflow.md` is how-to-work,
`ui-reference.md` is what-to-type + the per-screen dossier. Skim each doc's
section headings first, then read the relevant sections on demand. Fix the owning UPSTREAM
stage so the IR is correct; never name-gate, never hard-code a value, never
invent geometry. Reference captures are imperfect (bloom, skew, CRT, hover
artifacts, resolution) — match STRUCTURALLY, not pixel-naively. The target is
"as close as the reference allows," not pixel-identity.

Run autonomously through the loop; how many gates interrupt is chosen at launch
(SCOPE & MODE): **semi-automated** gates commits + freezes; **fully automated**
auto-commits and gates only freezes. **Freezing a baseline is ALWAYS gated.**

Worked example: SHIP `Drake Clipper`, SCREEN `Screen_Right_Upper_RTT` → folder
`Clipper`, reference `reference/in-game/Clipper/Screen_Right_Upper_RTT.png`;
dossier row → LOD0 scene, canvas `MC_S_Target_Master`, preset `target`, GOLD
`clipper_target_master`.

## Operating posture (the four defaults)

Nearly every way this goes wrong is one shortcut: a guess where an ASK or a
MEASUREMENT belonged.

- **Inputs / user-facing choices → ASK.** `AskUserQuestion`, FRESH every run,
  sequentially. Never inherit ship/screen/reference/scope from the session or a
  prior arc; never pre-compute one question's options from a guessed earlier
  answer.
- **Technical judgments → MEASURE.** Blast radius, "is this a blocker?", root
  cause, colour/position: prove it with the probes, `ui_arc_status.sh`,
  `ui_check.sh --full` (after a fresh export), the disable→adjudicate audit.
  Never estimate-then-defer.
- **The work (a TDD structural fix) → JUST DO IT.** No per-fix permission.
- **Hard-to-reverse → GATE.** Freeze (always); commit + final parity (semi).
- **Read result MARKERS** (`…: ALL GREEN` / `…: OK (N …)`), NEVER a piped or
  notified exit code — a `2>&1 | tail`/`| grep` or a backgrounded run reports the
  PIPE's/notifier's status (0), so a real guard failure reads "green" until you
  grep the marker or read it unpiped (ledger 89). Long-running commands
  (`ui_render.sh`, exports, `ui_check.sh --full`) WAIT on their exact marker in the
  log via the harness background facility — never a sleep-loop or turn-holding
  (ui-workflow §3).
- **Verify STRUCTURED-DATA claims by PARSING JSON + iterating the arrays** (or a
  runtime probe: `ui_variant_styles`, `BB_A3_STYLE_PROBE`, `SB_UI_FONT_DUMP`), NEVER
  a `sed`/`grep` line-window of a big nested record (serialization order lands you
  on the wrong entry; ledger 68). Visual findings must survive a SECOND look
  (shape/count/offset misreads) and include the background/backplate layer.

Asking and measuring are always safe; presuming and estimating are the failure mode.

## Phases

1. **LAUNCH — STOP: read `references/launch.md`.** SHIP → SCREEN → REFERENCE →
   SCOPE&MODE as sequential `AskUserQuestion`s (each answer feeds the next; the
   REFERENCE options come from the chosen SCREEN — never batch them). Then the
   required reads in order: `StarBreaker/AGENTS.md` → `crates/starbreaker-ui/AGENTS.md`
   → `ui-workflow.md` → `ui-reference.md` §3 dossier row (scene/LOD, canvas,
   preset, tier, open issues). SCREEN not in the dossier → adding its row (JSON +
   §3, `validate_ui_dossier.py` green) is part of the work.
2. **CATALOG — STOP: read `references/catalog.md`.** Build the numbered diff
   catalog via `ui_arc_status.sh`; self-verify (look AGAIN + background layer);
   the USER confirms the catalog (gate, BOTH modes). Diagnose/root-cause freely
   (incl. the failing test) but do NOT land a source fix pre-gate.
3. **LOOP (priority order, autonomous — structural/layout before styling):**
   a. `bash scripts/ui_arc_status.sh --screen <id>` → vision-read the FLAGGED
      crops only (not the whole screen every cycle).
   b. Owning stage: the MCP style trio / graphify (code — never blind grep) /
      the stage table below.
   c. font/size/colour wrong? → run the **`ui_variant_styles`** MCP tool FIRST
      (the authored-but-unapplied drill), THEN `references/blockers.md`.
   d. TDD failing test → ONE structural fix at the owning stage →
      `bash scripts/ui_check.sh` → re-render (a).
   e. Shared mechanism (asset / icon / binding / **LAYOUT-RENDER FORMULA**)? →
      fresh export + `--full` + EYEBALL every sibling screen sharing it
      (`references/blockers.md`).
   f. Guard trip = the system working → adjudicate via workflow §5 (structural
      discriminator, never a name). Tempted to defer / "blocked"? → STOP: read
      `references/blockers.md` (attach an evidence file that
      `scripts/ui_blocker_evidence.py` accepts; MAJOR items are USER-gated).
   g. A landed fix is NOT a checkpoint — take the next catalog item yourself.
4. **CLOSE — re-review from scratch** (fresh render+compare, look-again,
   background) — the closing variant in `references/catalog.md`. Fully-auto: fix
   until clean or proven-blocked. Semi: this IS the final-parity gate.
5. **FREEZE / COMMIT — STOP before ANY freeze: read `references/freeze.md`**
   (dry-freeze no-op check FIRST; freezes are ALWAYS user-gated, both modes).
6. **RETRO (MANDATORY — a TodoWrite item from arc start) — STOP: read
   `references/retro.md`.** The arc is done AFTER the retro, never before.

## Stage table (which stage owns the wrong thing — workflow §2)

| wrong thing | owning stage |
|---|---|
| node exists / authored fields / styles matched | bb_resolve / bb_style_engine / bb_state_filter |
| values (text / numbers / geometry bindings) | bb_bindings |
| rects / layout | bb_layout |
| surviving metadata / font px | ui_ir |
| final draw | ir_compose + text/ |

## Subagents (read-only research only)

Fan READ-ONLY research where it can't conflict; keep anything touching the shared
cargo target single-threaded.

- **Delegate (parallel-safe, READ-ONLY):** per-item root-cause (dcb_canvas mirror
  + MCP trio → owning stage + discriminator), guard-trip archaeology, measurement/
  catalog from an EXISTING render+ref pair, doc / DataCore / P4K lookups. Model = **Opus**.
  Tell each explicitly: do NOT build/test/render/export/edit — query, measure, report.
- **Code discovery: graphify before blind grep** (`graphify query`/`explain`,
  `/graphify`, `graphify-mcp` — relationship-aware, `file:line`, no API cost;
  ui-reference §4b). It maps CODE structure only — never a game-data VALUE source
  (MCP trio / parse-JSON rule). An empty graphify result ≠ absence (grep to confirm).
- **Never delegate:** anything running cargo/`ui_check.sh`/`ui_render.sh`/
  `entity export`/a freeze (shared target races); the fix + its TDD test; gated
  actions. Don't override a subagent's data claim with a WEAKER check than it used (ledger 68).

## Gates (which apply depends on the mode)

Present the evidence, then ask permission via `AskUserQuestion`; never proceed on a
presumed "yes," never act before the answer.

| Checkpoint | Semi-automated | Fully automated |
|---|---|---|
| Reference selection / catalog confirm | ask | ask |
| **Baseline freeze / re-freeze** | **GATE** | **GATE** |
| **Major-item blocker** | **GATE** | **GATE** |
| Git commit | gate | auto, no gate |
| Final parity | gate | re-review → fix until clean |

## Strict rules (workflow §1 — non-negotiable)

- **No hard-coding:** names, ship/screen/manufacturer branches, magic offsets/
  blend factors, **GAME-DATA VALUES** (palette / font-size / brand-list literals —
  fallbacks AND fixtures included). Self-correcting: replace or flag pre-existing
  offences in the SAME change; never extend because precedent exists.
- **Reproduce from the REAL decoded asset** (exhaust textures via `mtl_summary`→
  `image_preview`, `svgFill.svgPath`, styleTag `SvgPath`/`ImagePath`, SWF; resolve
  PER-MANUFACTURER via the cascade-applied `PrimitiveMaterialPath` override). "Gated
  off in the IR" ≠ absent — activate the real node, never generate a stand-in. A
  procedural rasteriser with eyeballed ring/spoke/tilt constants is the banned
  magic-number pattern (NOT "the hologram pattern," which rendered the real mesh;
  only its camera was tuned). Detail: `references/blockers.md`.
- IR is the sole styling authority; fix the owning upstream stage, not the draw-time symptom.
- Frozen platinum/gold baselines move ONLY via the audited freeze flow / §6 known-outlier.
- 3000-line cap; revert no-effect experiments immediately; verify-on-write every doc command.
- **Linear-light compositing LANDED** (plan B4; all baselines re-frozen): colour
  residuals are now judged NORMALLY, not leniently — a genuine blend-shaped drift at
  a composited edge is a latent bug to root-cause and adjudicate (§5), not a
  known-outlier to park (`references/blockers.md`).

## Core red flags — STOP, you're rationalizing

| Thought | Reality |
|---|---|
| "The skill summary is enough" | Read ui-workflow.md + ui-reference.md; this skill only orchestrates. |
| "Found the root cause / know the next item — ask first?" | Just do it. Applying a fix and pulling the next-priority item are the loop's work, not checkpoints. |
| "That fix landed — a natural checkpoint to ask whether to continue" | A landed sub-fix is NOT an arc boundary. Take the next open item (fully-auto never asks). Short on context? SAY so + hand state to memory/handoff — don't convert budget anxiety into a permission question. |
| "Catalog's resolved / found more issues — run the retro" | The retro is the LAST step, never a substitute for fixing. Fix fixable diffs first, then the closing re-review, THEN the retro (TodoWrite item from arc start). |
| "Spin up parallel agents to build/render faster" | Builds share the cargo target and race. Only READ-ONLY research parallelizes; builds/renders/tests/fixes/freezes stay sequential in the main agent. |
| "Freeze it to pass / fully-auto so auto-freeze / it surely drifts so re-freeze" | Freezing is ALWAYS gated, both modes — STOP: read references/freeze.md (dry-freeze no-op check first). |
| "Large blast radius / undecoded this arc — defer it" | Size/risk/"undecoded" is not a blocker — STOP: read references/blockers.md. Defer only on an exhausted-search PROVEN blocker (evidence file validated by ui_blocker_evidence.py). |
| "I'll presume/reuse the ship, screen, or reference" | Every run starts COLD — STOP: read references/launch.md; ask fresh via AskUserQuestion (even with one folder; never skip the reference confirmation). |

## Pointers

workflow §5 guard trips · §6 known-outliers · §7 freezes · reference §3 dossier
(machine mirror: `crates/starbreaker-ui/data/ui_screen_dossier_v1.json`) ·
`ui-process-improvements.md` ledger for history · `recommendations.md` for skill findings.
