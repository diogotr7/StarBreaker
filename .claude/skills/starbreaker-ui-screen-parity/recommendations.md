# starbreaker-ui-screen-parity — recommendations

Companion to `SKILL.md`. Per the skill's creation decision (2026-06-14) it is
validated by real use, not a RED→GREEN subagent loop. Findings live here in two
states:

- **Open** — skill improvements not yet applied. End-of-arc retrospectives (via
  `references/retro.md` → the arc-closeout skill) and reviews append here rather
  than editing `SKILL.md` mid-arc.
- **Change log** — one line per applied change. A `writing-skills` session that
  applies an open item moves it here. Full narratives live in git history (this
  file before the 2026-10-14 condense) and in the process ledger
  `crates/starbreaker-ui/docs/ui-process-improvements.md`.

## Validation status

- **Tested through real use** on Drake Clipper arcs (g-force/velocity ball →
  platinum, velocity-num → gold, compass, countermeasures, LR-indicator, master
  mode) and the Anvil Carrack console arcs. The freeze gate held on the first full
  arc (2026-06-15) and the retro fed both destinations (ledger 52–55).
- **The A8 compliance dry-run ran** (plan A1–A8, ledger 108, commit
  `369feeaba`): 7/7 acceptance items, and it generalized a stale "only one ship
  folder" claim.
- **Still open:** the full execution test, A10 (the next real Carrack arc under the
  restructured skill) — carried as open in ledger item 110; success bar in
  `docs/superpowers/specs/2026-07-04-ui-parity-process-and-crate-design.md`,
  "A success criteria".

## Open

1. **Nothing consumes this file.** Open items accumulate but no step reads them
   back. Decide a trigger — e.g. a `writing-skills` session on this skill (or arc
   start) skims the open items first.
2. **Canonical home for "verify a re-freeze is needed first"** (ledger 85). The
   skill's `references/freeze.md` has the dry-freeze / `--preflight` no-op check,
   but `crates/starbreaker-ui/docs/ui-workflow.md` §7 still lacks it as a backing
   rule.
3. **A10 execution test** (above) — run it, then record shortfalls here and in the
   ledger. Check `arc_cost_log_v1.jsonl` gets its line: it was still empty at the
   2026-10-14 docs audit (either no arc has run since the step was added, or the
   step is being skipped).
4. **Reference root is machine-specific.** The UI scripts hard-code
   `$HOME/projects/scorg_tools/...` (reference root, export root) and `/tmp`, the
   old Linux workspace layout; `references/launch.md` now tells the agent to ask.
   Parameterising the scripts is a code task.

## Known assumptions & risks

- **Reference-variant selection** (prefer the `corners.json` straight-on capture)
  is heuristic; the mandatory user confirmation is the backstop.
- **Restated rules can drift.** `SKILL.md` repeats rule headlines from
  `ui-workflow.md` §1; when those change, re-check the restatements
  (verify-on-write applies to cited commands).
- **Discoverability.** The skill auto-loads as a project skill when Claude runs
  with StarBreaker as the project root; from a parent workspace, invoke it
  explicitly.
- **Autonomy boundary.** Semi-automated gates commits, freezes and final parity;
  fully automated auto-commits and drops the final-parity gate but STILL gates
  every freeze (it mutates frozen baselines — workflow §6/§7). Auto-approving
  freezes is not an offered mode; record the decision here first if ever wanted.

## Change log (applied, one line each)

- 2026-06-14 — SHIP always asked via `AskUserQuestion`, even with one folder.
- 2026-06-14 — SHIP, REFERENCE and SCOPE confirmed up front (owner request).
- 2026-06-14 — "every invocation starts cold": SCREEN (and all inputs) re-asked.
- 2026-06-14 — checkpoints use `AskUserQuestion` approve/decline, not prose stops.
- 2026-06-15 — automatic mode works the catalog top-down without per-item check-ins.
- 2026-06-15 — split into semi-automated vs fully automated; freeze is the invariant gate.
- 2026-06-15 — applying a structural fix is the loop's job, not a checkpoint.
- 2026-06-15 — SHIP → SCREEN → REFERENCE asked sequentially, never batched.
- 2026-06-15 — deferral bar raised to a PROVEN blocker; blast radius is measured, not estimated.
- 2026-06-15 — Operating posture (ASK / MEASURE / JUST-DO / GATE) consolidated the shortcut class; this file split into Open + Change log.
- 2026-06-15 — read-only research delegated to subagents; builds/renders stay sequential.
- 2026-06-15 — retrospective made a MANDATORY closing step, tracked from arc start.
- 2026-06-15 — catalog phase: build → look again → background layer → user confirms.
- 2026-06-15 — mandatory closing re-review before the retro.
- 2026-06-15 — "inherited verdicts are hints, not conclusions" (catalog phase).
- 2026-06-15 — fix-before-retro; blocker search covers the whole decodable surface; `--helper` from the dossier.
- 2026-06-15 — red-flag table consolidated 24 → 14 rows.
- 2026-06-15 — `--full` needs a fresh export first (ledger 56).
- 2026-06-15 — major-item blocker gate (both modes) + verifiable evidence trail (ledger 66).
- 2026-06-15 — diagnose pre-gate, land the source fix only after the catalog gate.
- 2026-06-16 — verify data claims by parsing JSON / runtime probe; don't override a subagent cheaply (ledger 68).
- 2026-06-17 — unique filenames for user-facing renders (ledger 69); auto-canvas experiments judged with `--full`.
- 2026-06-17 — whack-a-mole scoping = WRONG STAGE, not a proven blocker (overturned ledger 75 via 76); sibling-screen eyeball (ledger 77).
- 2026-06-17 — reproduce from decoded data, never invent geometry (ledger 78–83).
- 2026-06-18 — read `ui_check`'s result marker, never a piped exit code (ledger 89; canonical in `ui-workflow.md` §3).
- 2026-06-18 — SHIP/SCREEN options padded to ≥2 (auto "Other" doesn't count).
- 2026-06-18 — measure whether a re-freeze is needed before gating it (ledger 85).
- 2026-06-18 — LR-indicator "undecoded scaling" defer overturned: re-read includes canvas-root `embeddedStyles` (ledger 94).
- 2026-06-19 — "font too small" has TWO non-blocker causes (unapplied size, per-screen render scale); off-screen "+N regresses" deltas (ledger 96–98). LR-indicator onboarded GOLD.
- 2026-07-02 — a landed sub-fix is not an arc boundary; perf side-questions go to the optimisation skill.
- 2026-07-07 — A6 restructure: `SKILL.md` → navigation core + `references/*.md`; layout/render formula = shared mechanism; instrument resolved geometry (ledger 106).
- 2026-07-08 — T9: thin-glyph colour by pixel count, fresh-export reproduction gate, wait-on-marker, interim linear-light leniency retired (B4 landed; ledger 107).
- 2026-07-08 — T15: dossier-computed SCOPE&MODE default; ponytail + process-skill lines.
- 2026-10-14 — docs audit: closed stale opens (A8 dry-run ran; multi-ship exercised by Carrack; LR-indicator font fixed; `engine_*.part` split done, ledger 104/105; hard-coded heading table tracked in `ui-fallback-register.md`). Memory steps → in-repo ledger; commits on the arc branch, never `main`; `FONTPROBE` → `SB_UI_FONT_DUMP`/`BB_TEXT_FORMAT_PROBE`; graphify optional; `uv run` for Python; Clipper target tier corrected to PLATINUM.
