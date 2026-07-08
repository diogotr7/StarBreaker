# starbreaker-ui-screen-parity — review & recommendations

Companion to `SKILL.md`. Per the skill's creation decision (2026-06-14), the
skill is **not** pressure-tested by re-writing it in a RED→GREEN loop; instead,
findings land here in two states, with a lifecycle:

- **Open recommendations** — ideas not yet applied. End-of-arc retrospectives
  (`crates/starbreaker-ui/docs/ui-process-retro-prompt.md`) and reviews append
  here rather than editing `SKILL.md` mid-arc.
- **Change log** — applied changes. When a `writing-skills` session implements
  an open recommendation (or an owner-requested edit), it records the applied
  change in the Change log and clears the open item.

## Validation status

- **De-facto tested through real use.** No formal subagent baseline/green test
  has been run, but the skill has been exercised on real invocations (Drake
  Clipper) and that surfaced 8 issues — all fixed (see Change log). The recurring
  class was "agent shortcuts the discipline" (presume / batch / estimate-defer),
  now consolidated into `SKILL.md`'s **Operating posture** section.
- **Freeze gate held on the first full real arc (2026-06-15, g-force/velocity
  ball to platinum).** The agent stopped at the freeze checkpoint, presented the
  per-identity deltas, and the owner explicitly authorised the re-freezes — the
  one invariant gate worked as designed, not just by assumption. The end-of-arc
  retrospective also fired on BOTH destinations: the skill finding → Open
  recommendations (now applied), and process/tool/doc findings →
  `crates/starbreaker-ui/docs/ui-process-improvements.md` ledger items 52–55 +
  `scripts/ui_gauge_measure.py` (committed `0daf35e8e`). The self-improvement
  loop is validated end-to-end.
- **"Inherited verdicts are hints" / "undecoded = under-research" rules were
  DECISIVE (2026-06-15, velocity-num → gold).** The dossier + memory recorded the
  ~9× font gap as a PROVEN blocker ("per-screen text-scale model undecoded, blocked
  by the annunciator counterexample"). Re-deriving from the reference (per the
  skill's mandate) overturned it: never a scale problem — the variant authors
  FontSize 500/420, they weren't being applied. Fixed structurally (`6c1343abf`/
  `c7931fb4b`) + onboarded gold (`a91cec377`) in the same arc. The canonical case the
  red-flag row "undecoded → research then fix" guards against, now proven on a real
  wrongly-deferred item — the existing rules did exactly their job, no skill change
  needed. (Process/tool findings → ledger items 60–62.)
- **Recommended later validation** (lower priority now, given real use):
  1. **Compliance dry-run (cheap).** Dispatch a fresh subagent with
     "Drake Clipper / Screen_Right_Upper_RTT" and confirm, from the skill + docs
     alone, it: (a) resolves the reference to
     `reference/in-game/Clipper/Screen_Right_Upper_RTT.png` and CONFIRMS it with
     the user; (b) reads the four required docs in order; (c) finds the dossier
     row (canvas `MC_S_Target_Master`, preset `target`, GOLD
     `clipper_target_master`); (d) respects the freeze/commit/final stop points
     rather than barrelling through; (e) knows to run the retro at the end. Any
     miss is a skill gap — fix it in `SKILL.md` and note it here.
  2. **Full execution test (expensive).** Same scenario, subagent actually runs
     the loop; compare behaviour with vs without the skill.

## Open recommendations (review / decide)

- **Formal compliance dry-run** — still un-run. Decide whether real-use coverage
  supersedes it, or run it once for the paths real use hasn't exercised:
  multi-ship disambiguation, and a screen absent from the dossier.
- **Nothing consumes this file.** Open recommendations accumulate but no step
  reads them back. Decide a trigger — e.g. a `writing-skills` session on this
  skill (or arc start) skims the open items first.
- **Multi-ship generalization unverified** (see Known assumptions) — exercise
  when a second `reference/in-game/` folder exists.
- **"Font too small" → the LR-indicator case was PREMISE-OVERTURNED by a data
  probe (2026-06-18) — it was an unapplied authored size, not undecoded engine
  scaling; applied to `SKILL.md`, see Change log.** (The drafted worry that a
  named-style/`Auto`-sized HUD label with no authored FontSize is an undecoded
  engine scale to document-and-defer was checked against the data and falsified.)

## Known assumptions & risks

- **Ship→folder mapping** is only exercised for Drake Clipper → `Clipper`; no
  other reference folder exists yet. The skill auto-discovers folders under
  `reference/in-game/` and confirms the match with the user, which should
  generalize, but multi-folder disambiguation is unverified.
- **Reference-variant selection** (prefer the `corners.json` straight-on
  capture) is heuristic; the mandatory user confirmation is the backstop.
- **Restated rules can drift.** `SKILL.md` repeats a few rule headlines from
  `ui-workflow.md` §1 for strict adherence. If those docs change, re-check the
  skill's restated rules; verify-on-write applies to the command lines it cites.
- **Discoverability.** The skill lives in `StarBreaker/.claude/skills/`; it
  auto-loads as a project skill when Claude runs with StarBreaker as the project
  root. From the `scorg_tools` workspace cwd, invoke it explicitly.
- **Autonomy boundary.** Two launch-selected modes: **semi-automated** gates
  commits and freezes (+ final parity); **fully automated** auto-commits and
  drops the final-parity gate but STILL gates every freeze. The freeze gate is
  invariant across modes by design — it mutates frozen regression baselines, the
  docs' hard approval gate (workflow §6/§7). Auto-approving freezes is NOT an
  offered mode; if ever wanted, record the decision here first.

## Change log (applied changes, append-only)

Dated, sourced entries for changes already made. Open, not-yet-applied ideas go
under **Open recommendations** above.

- **2026-06-14 (first real invocation).** The skill auto-presumed SHIP = Drake
  Clipper instead of asking, rationalising "only one populated folder / the ship
  this session was on." Root cause: step 1 said "resolve by matching," which
  reads as a licence to auto-select. Fix landed same day: step 1 now mandates an
  `AskUserQuestion` ship confirmation (discovered folders as options + "Other"
  free-text), explicitly "even when only one folder exists," and a red-flag row
  counters the presume-the-single-folder rationalisation.
- **2026-06-14 (authoring refinement, owner-requested).** Front-loaded
  interaction model firmed up: SHIP, REFERENCE, and SCOPE are now each confirmed
  via `AskUserQuestion` before the autonomous loop starts — ship pick, reference
  image confirmation (yes / different file), then "work automatically vs. name
  specific issues." Keeps the loop hands-off while making the three decisions
  that steer the whole arc explicit user choices.
- **2026-06-14 (second real invocation).** On a re-run the skill reused the
  previously-confirmed SCREEN and gave no chance to re-select. Same root-cause
  category as the SHIP presumption: carrying session/prior-arc state into a
  fresh run. Fixed at the category level rather than per-input — added an
  "Every invocation starts cold" lead-in (SHIP, SCREEN, REFERENCE, SCOPE all
  asked fresh, never inherited), rewrote step 2 to always ask SCREEN via
  `AskUserQuestion` (lists the folder's screens for re-select + "Other", with a
  prose-list fallback when screens exceed the 4-option limit), and added a
  red-flag row. Watch for the next variant of this class (reusing a reference
  variant or scope) — the cold-start lead-in should now pre-empt it.
- **2026-06-14 (checkpoint UX, owner-requested).** Checkpoints (freeze, commit,
  final parity) now request permission via `AskUserQuestion` with an explicit
  approve/decline choice, not a prose stop — consistent with the input
  questions. Added a red-flag row against acting on a presumed "yes." Reference
  confirmation already used the tool.
- **2026-06-15 (automatic-mode autonomy).** In automatic SCOPE mode the skill
  was asking which catalog item to tackle next. Clarified that automatic mode
  orders the catalog by priority (workflow §4) and works top-down without
  per-item check-ins — when an item is fixed/deferred/blocked it pulls the
  next-highest itself; the four checkpoints remain the only interruptions. Added
  a red-flag row. (Named-issues mode still works the user's list, also in
  priority order.)
- **2026-06-15 (two autonomy modes).** Split the single automatic mode into
  **semi-automated** (gates commits + freezes + final parity — the prior
  behaviour) and **fully automated** (auto-commits, drops the final-parity gate,
  but STILL gates freezing). Chosen at launch via the SCOPE & MODE question
  (now two questions in one prompt: scope = full review vs named issues; mode =
  semi vs fully). Checkpoints section is now a mode × checkpoint table; freeze
  is the invariant gate. Added red-flag rows against auto-freezing in fully
  automated mode.
- **2026-06-15 (fix without asking).** The skill was pausing to ask permission
  after tracing a root cause. Clarified that applying a structural fix is the
  loop's job, not a checkpoint — once the owning stage is identified, edit source
  directly (TDD: failing test first). Only freeze/commit/final gate. Updated the
  Fix step and added a red-flag row.
- **2026-06-15 (sequential dependent questions).** The skill batched SCREEN and
  REFERENCE into one prompt, so the reference options were pre-computed from a
  presumed/default screen instead of the one the user actually picked. Root
  cause: REFERENCE options depend on the SCREEN answer. Fixed by mandating
  separate, SEQUENTIAL `AskUserQuestion` calls for the dependent chain
  (SHIP → SCREEN → REFERENCE), discovering reference candidates only after the
  screen answer returns; only SCOPE & MODE (mutually independent) may share one
  prompt. Added a lead-in rule, a step-3 "only after SCREEN is answered" clause,
  and a red-flag row. General lesson for this skill: never pre-compute a later
  question's options from a guessed earlier answer.
- **2026-06-15 (don't defer on estimated risk).** The skill was deferring items
  on a self-assessed "large blast radius / deserves its own change" estimate
  instead of doing them. Raised the deferral bar to a PROVEN concrete blocker;
  size/risk now triggers research + (if large) a short plan + execution. Points
  at the empirical disable→adjudicate audit (workflow §5) + `ui_check.sh --full`
  to MEASURE blast radius rather than estimate it (matches the arc-memory lesson
  "measure, don't estimate" — paper analysis repeatedly mis-called "blocked").
  Reassures that making the change is autonomous and only the resulting freeze is
  gated. Added "Default to fixing, not deferring" paragraph + two red-flag rows.
- **2026-06-15 (review consolidation).** Review of the log surfaced that 6 of 8
  findings are one class — agent substitutes a guess for an ask or a
  measurement. Added an **Operating posture** section to `SKILL.md` (ASK inputs /
  MEASURE judgments / JUST-DO the work / GATE hard-to-reverse actions) so the
  next novel shortcut is caught without its own red-flag row. Also fixed this
  doc's purpose drift: it had become a changelog of done edits though it was
  meant for open ideas — split into **Open recommendations** (lifecycle: retro
  appends → writing-skills applies → moves here) and **Change log**, and
  refreshed the stale "Not yet executed" validation status.
- **2026-06-15 (subagents for research).** Added a "Use subagents for read-only
  research" section: delegate parallel-safe, READ-ONLY work (per-item root-cause
  research via the MCP trio + dcb_canvas mirror, guard-trip archaeology,
  measuring an existing render+reference pair, doc/data lookups) to subagents
  that report back; keep everything sharing the cargo target — `cargo
  build`/`test`, `ui_check.sh`, `ui_render.sh`, `entity export`, freezes — plus
  the fix/TDD test and gated actions sequential in the main agent (AGENTS.md:
  avoid broad parallelism for shared compilation targets). Subagents are told
  explicitly not to build/test/render/export/edit. Added a red-flag row + a
  pointer from the "Default to fixing" paragraph.
- **2026-06-15 (retrospective was skipped).** The skill ended without running
  the self-improvement retrospective — it was a soft pointer to the external
  `ui-process-retro-prompt.md` in the last section, easy to drop (especially in
  fully-automated mode, which stops after the final-parity report). Fixes:
  reframed it as a MANDATORY closing step ("arc done = retro done", both modes);
  inlined the 7 sweep categories so the skill is self-contained instead of
  leaning on an external prompt; split destinations (process/tool/doc findings →
  `ui-process-improvements.md` ledger; skill findings → Open recommendations);
  instructed tracking it as a TodoWrite item from arc start; added a "loop ends,
  arc doesn't" clause and a red-flag row. External prompt kept as the canonical
  option for the verbatim sweep.
- **2026-06-15 (catalog accuracy + confirm gate).** Two real defects in review:
  (a) findings were mostly right but occasionally outright wrong (square seen as
  circle); (b) the background/backplate layer (stretch, misalignment) was not
  considered. Added a "Build & confirm the diff catalog" phase at arc start
  (both modes, fresh or previously-worked screen): build → self-verify each
  finding by looking AGAIN (guard shape/count/offset-direction misreads, measure
  when unsure) → explicitly check the background layer → confirm the catalog with
  the user via `AskUserQuestion` (with free-text comments) before any fixing.
  Added a launch checkpoint row, an Operating-posture MEASURE clause, a loop
  Compare-bullet note (same checks on newly-surfaced diffs; re-confirm only on
  material change), and three red-flag rows.
- **2026-06-15 (closing re-review).** Added a mandatory closing re-review before
  the retrospective (both modes): re-render fresh + re-compare/self-verify
  against the reference exactly as the opening catalog phase (look-again +
  background layer), building a fresh catalog of what remains. Fully automated
  feeds remaining fixable diffs back into the loop and repeats until clean (or
  proven deferred/blocked); semi-automated surfaces the fresh re-review at the
  Final-parity `AskUserQuestion` gate (accept vs another pass). Updated the
  checkpoints table/bullet (fully-auto Final parity is now "re-review → fix until
  clean", not "report only"), the loop closing line, and added two red-flag rows.
- **2026-06-15 (inherit-verdicts clause; applied from Open recommendations).**
  The g-force arc's retro found the opening catalog UNDER-called the screen by
  carrying prior "faithful / owner-confirmed / not-flagged" verdicts forward
  (missed marker desync, at-rest "0", right-edge line; the user expanded the
  catalog at the confirmation gate — backstop worked). Applied to the catalog
  phase: "**Inherited verdicts are hints, not conclusions**" — re-derive each
  region from the reference at high zoom (small features hide in a naked
  side-by-side), with the scoping refinement that a frozen-baseline or
  registered-known-outlier region routes any re-opened verdict through the
  §5/§6/§7 audited flow, not a silent fix (so it doesn't collide with
  settled-numbers-are-lookups / frozen-baseline discipline). Added a red-flag row.
- **2026-06-15 (fix-before-retro + raised blocker bar; velocity-num arc).** A run
  fixed several issues, then identified more but ran the retro instead of fixing
  them — deferring on "not in the canvas data / undecoded this arc," which is
  under-research. Changes: (1) raised the PROVEN-blocker bar in *Default to
  fixing* — "demonstrated absent" now means searching the whole decodable surface
  (DataCore / P4K / localization tables, not just canvas JSON) AND showing the
  value isn't derivable from a decoded mechanism (screen-mesh/aspect for a
  content-view sub-rect; enum→localization for a unit suffix); "frozen-family
  risk" is a §5 discriminator task, not an auto-defer. (2) Closing re-review now
  states the retro is the LAST step, never a substitute for fixing a fixable diff;
  any late-found issue is fixed in the loop first. (3) Applied the two velocity-num
  Open recs: `--helper` comes from the dossier's Helper column (not always the
  reference stem — fixed step 2 + the render bullet), and a proven-blocked
  remainder is now an explicit valid terminal ("deferred with proof, not clean")
  so a fully-automated run STOPS instead of looping forever. Added two red-flag rows.
- **2026-06-15 (red-flag table consolidation).** The table had grown to 24 rows
  with near-duplicates. Merged to 14 along the Operating-posture categories (ASK
  inputs / MEASURE judgments / DO the work / GATE) with no specific catch lost:
  input presume/reuse/skip-confirm → one row; first-glance + background → one;
  root-cause + which-item-next → one; blast-radius + own-change + undecoded-defer
  → one; the three freeze rows → one; catalog-resolved + fixes-done +
  found-more-issues → one. Cleared the matching Open recommendation.
- **2026-06-15 (`--full` needs-fresh-export caveat; velocity-num arc, ledger 56).**
  `ui_check.sh --full` does not re-export, so running it against stale Generated
  PNGs trips the staleness guard and reads as breakage. Added a short "(after a
  fresh export)" / "(re-export first — `--full` does not re-export; ledger 56)"
  caveat to the two inline `--full` mentions (Operating posture MEASURE bullet +
  *Default to fixing*). Authoritative detail stays in `ui-reference.md` §1/§2,
  which the arc fixed.
- **2026-06-15 (major-item blocker gate + verifiable proof; compass arc, owner
  decisions).** The agent self-certified the compass live-ticks as a PROVEN
  blocker ("engine C++ only, exhausted the data") — 3rd recurrence (cf. font,
  ledger 60); the owner's manual press found it one `search_records("vehiclehud")`
  away (`SVehicleHudParams.compassTape`, ledger 66). Applied (owner chose "surface
  to user, both modes"): (1) a new **Major-item blocker** checkpoint — a
  dominant-item give-up is gated in BOTH modes (present evidence trail → "Accept
  as blocked? / Research further"), like a freeze, never self-certified; (2)
  *Default to fixing* raised — "engine C++ only" is rejected as unfalsifiable;
  a blocker needs a VERIFIABLE evidence trail (records/greps/probes + empty
  results, not an assertion); search the record FAMILIES (`*Params`/`*HudParams`),
  not just the feature keyword; run a find-it-or-prove-absence subagent first;
  (3) closing-re-review terminal note + two red-flag rows. Cleared the families
  Open rec.
- **2026-06-15 (diagnose-vs-land boundary; compass arc, owner decision).** Owner
  chose "investigate + characterizing failing test pre-gate; land the source fix
  only after the catalog gate (commit always waits)." Stated in *Build & confirm
  the diff catalog* + a red-flag row. Cleared the Open rec.
- **2026-06-17 (visual-iteration cache trap + whack-a-mole-scoping blocker;
  applied from Open recommendations — self-status hologram ledger 69 + master-mode
  ledger 75).** A `writing-skills` session pointed at the recent retros actioned the
  two un-applied Open recs. (1) Ledger 69: iterating renders to the user under
  `ui_render.sh`'s FIXED output path let the viewer cache by filename and report "no
  change" on a file that DID change (cost several cycles). Applied to the loop's
  *Render* bullet — copy each user-facing iteration to a UNIQUE filename, confirm
  on-disk change via the printed `png md5:`, never re-tune blind on a "looks
  identical" report — plus a red-flag row. (2) Ledger 75: on the master-mode arc the
  size and colour fixes each regressed a DIFFERENT frozen `auto` canvas, and every
  narrower scope (`auto`→`HC_HUD+auto`→tag) regressed the next frozen sibling.
  Applied to *Default to fixing* at the §5-discriminator sentence — "no discriminator"
  now has a concrete tell (repeated next-sibling regression = PROVEN no-discriminator
  blocker; stop threading scopings, surface it), and such auto-canvas /
  `coordinateMethod=auto` / HC_HUD text experiments MUST be judged with `--full`
  after a fresh export because the drift is whole-image-only and invisible to
  `ui_check` live-IR — plus a red-flag row. Both changes are backed by documented
  real-arc failures (the RED), consistent with this skill's real-use validation
  model. Cleared both Open recs.
- **2026-06-16 (verify data claims with the right tool; don't override a subagent
  cheaply; compass round 3, ledger 68).** The agent dismissed a CORRECT subagent
  colour diagnosis by `sed`/`grep`-ing line ranges of a 2 MB nested record,
  landing on the wrong H1 entry — then chased a wrong overlay fix + revert before
  a `json.load` + `FONTPROBE` proved the subagent right. Applied (verification
  discipline, both homes): Operating posture MEASURE now says verify a
  structured-data claim by PARSING the JSON / runtime probe, never a line-range
  grep of a big nested record (serialization order defeats it); the subagents
  section adds "don't override a subagent's data claim with a weaker check than it
  used — your refutation must meet the same rigour"; two red-flag rows. Cleared
  the Open rec.
- **2026-06-17 (REVISED the whack-a-mole guidance — it was itself an
  inherited-blocker trap; + sibling-screen eyeball; master-mode arc-2 ledger 76 +
  ledger 77).** A retro OVERTURNED ledger 75 (codified in the entry above, the SAME
  day): the master-mode "no-discriminator blocker" was a RENDER-SIDE scope treadmill —
  the real fix was UPSTREAM, an authored `defaultStyles` (FontSize 350 + white) the
  first pass never read (ledger 76). That made the freshly-added "repeated
  next-sibling regression = PROVEN blocker" text actively MISLEADING — the THIRD
  inherited size/colour "blocker" to dissolve on reading the instantiated variant
  (after velocity-num + compass). FIX: *Default to fixing* and its red-flag row now
  say repeated next-sibling regression means you are at the WRONG STAGE
  (symptom-scoping render-side a fix that belongs UPSTREAM), NOT that a blocker is
  proven — re-read the instantiated variant's `defaultStyles`/`brandStyles` (parse
  JSON) and verify the authored value reaches the node (`ui_ir_query --fields
  text_style` / `BB_TEXT_FORMAT_PROBE`) before ANY size/colour blocker claim; only a
  genuinely-absent authored value + exhausted upstream search is a blocker. The
  `--full`-after-export half of the prior entry stays (still correct). SEPARATELY
  (ledger 77): a shared-mechanism asset/icon/binding change (footer nav arrows)
  regressed sibling screens a few px UNDER `--full`'s ~1% budget — added a
  sibling-screen eyeball note to the loop's *Fix* step + *Closing re-review* + a
  red-flag row. Cleared both Open recs. META-lesson: be wary of codifying "X is a
  proven blocker" from one arc — these blockers keep dissolving on a deeper read of
  the authored variant; the skill should bias toward RE-READING, not toward accepting.
- **2026-06-19 (LR-indicator fix arc CLOSED — validated the 2026-06-18 overturn +
  found a SECOND cause; ledger 96–98; `writing-skills` review session).** The hand-off
  arc ran (onboarded GOLD `clipper_lrind_master`). It confirmed BOTH of ledger 94's
  premises were wrong: (96) the authored FontSize 100 in the DRAK sub-canvas's
  `embeddedStyles` `Type(Text)` selector was real but the text-format route ran only at
  the BRAND tier, not the EMBEDDED tier (fixed `6cb31a350`) — exactly the overturn +
  embeddedStyles read I applied on 2026-06-18, so that skill edit is VALIDATED, no
  further change there; (97) a SECOND root cause I did NOT catch — even with the size
  applied the portrait screen was ~3× small and needed a `ui_ir` font screen scale
  (`2bbf9b214`), and the obvious `bb_layout` `canvas_scale` knob was INERT (the IR
  renderer reads `font_size` directly; a byte-identical `png md5` proved a dead-stage
  edit). So "font too small" has TWO non-blocker causes (unapplied authored size OR a
  per-screen render scale), not one — my 2026-06-18 red-flag row covered only the first,
  leaving the residual deferrable as "undecoded." Applied: extended the font red-flag row
  to name BOTH causes + the "trace the lever to where the glyph px is read; an inert knob
  = wrong stage, confirm via `png md5`" point. Also applied (98): an inherited "+N
  regresses frozen X" verdict can be an OFF-SCREEN (0×0) re-freezable delta, not a real
  regression — added a MEASURE-it clause to *Default to fixing* (frozen-family). NOT skill
  changes: 96's text-format-tier fix + 97's `ui_ir`-vs-`bb_layout` lever + 98's
  forced_active binding-namespace scope and stale-`ships/`-path detour are domain/tooling
  (ledger + dossier homes). Net: the owner's "the agent gave up" instinct was right twice
  over — BOTH deferred premises were do-able fixes.
- **2026-06-18 (LR-indicator font "undecoded engine scaling" defer was PREMATURE —
  4th font-blocker to dissolve on reading the instantiated variant; owner-prompted
  "the agent gave up").** The owner asked to check the LR-indicator retro's font
  give-up (ledger 94: labels ~5–7× too small, declared "undecoded engine HUD-label
  scaling, document-and-defer"). RED: the defer's load-bearing claim was "labels
  author `labelProperties.style=Heading1` (60px), NO authored FontSize, the named
  style IS reaching the node." A data probe (the skill's own mandate) FALSIFIED it:
  the Clipper instantiates the per-manufacturer `DRAK_HC_HUD_Cutlass_LInd`/`RInd`
  sub-canvas (the master tiles brand sub-canvases via `CanvasReferenceRecord`; the
  agent read the GENERIC master / only `labelProperties`), and that variant's
  canvas-root `embeddedStyles` authors a bare `Type(Text)` selector → `FontSize 100`
  (the generic variant has none). 100 vs Heading1 60 ≈ 1.67× = exactly the "~2×
  bigger than Heading1 at native canvas" residual the agent attributed to an undecoded
  engine scale. This is the velocity-num/master-mode/compass bug a FOURTH time — an
  authored size present but not APPLIED. The authored value lived in `embeddedStyles`,
  a THIRD location the skill's re-read mandate didn't name (it listed only
  `defaultStyles`/`brandStyles`), so an agent following the mandate literally would
  still miss it. Applied to `SKILL.md`: (1) the *Default to fixing* re-read mandate
  now includes canvas-root `embeddedStyles` (CSS-like `Type(Text)`/selector entries
  auto-apply and are easy to miss) + the "instantiated = the brand sub-canvas
  `drak_*_cutlass_*`, NOT `generic_*`" point, bumped THREE→FOUR dissolved arcs; (2)
  the matching red-flag row updated to four arcs + `embeddedStyles`; (3) a NEW
  red-flag row for the exact "no authored FontSize → undecoded → defer" rationalization.
  This is the OPPOSITE of the drafted Open rec (which proposed a "recognize-and-defer
  -faster" line) — the empirical record is that these font "blockers" keep dissolving
  on a deeper read, so the skill must bias toward RE-READING (ledger 76 meta-lesson),
  never toward easier deferral. OUT OF SKILL SCOPE, flagged for the owner / next arc:
  ledger 94 + the LR-indicator dossier row carry the now-falsified "undecoded engine
  scaling, deferred" conclusion — they should be corrected and the item re-opened; the
  actual parity fix (apply the `embeddedStyles` `Type(Text)` FontSize to the labels —
  likely the velocity-num text-format routing path) is a separate gated UI arc, not
  done here.
- **2026-06-18 (trust `ui_check`'s result MARKER, not a piped/notified exit code;
  countermeasures arc continuation, ledger 89; `writing-skills` review session).**
  The countermeasures arc closed out (GOLD `clipper_countermeasures_master`), adding
  ledger 88–90. RED: a run piped `ui_check.sh 2>&1 | tail` (and backgrounded it),
  which reported "exit 0" on a run where the live-IR guard actually FAILED — the
  pipe/notifier exit status masks the script's. The skill leans on `ui_check`'s
  result in 7 places (MEASURE posture, per-cycle check, commit gate, closing
  re-review) but never told the agent how to READ that result, so a masked failure
  would silently corrupt the fix-vs-defer / clean / commit decisions. Fix: a factual
  caveat in the Operating-posture MEASURE bullet — read the `ui_check: ALL GREEN` /
  `ui_check: FAILED (exit N)` marker, never the piped/notified exit code (the script
  now prints the FAILED marker via an EXIT trap). Factual caveat, not a red-flag (the
  failure is being MISLED by tooling, not a discipline rationalization). The other
  two new findings are NOT skill changes: 88 (scope `bb_state_filter` numeric
  resolution to bare component-local bindings) is a domain code detail whose "run the
  live-IR guard before assuming a shared-filter change is local" lesson is already
  embodied by "run `ui_check` every cycle"; 90 (a multi-digit data value breaks
  single-digit layout; one region's catalog items can be causally linked through a
  shared value) the retro itself routed to the DOSSIER, and the skill's "fix the
  upstream cause, not the symptom" + "shared-root-cause items together" already
  orient it. OUT OF SKILL SCOPE, flagged for the owner / next retro: ledger 89's
  "read the marker, not the piped exit code" caveat has NO home in the authoritative
  `ui-reference.md` §1/§2 (where it belongs canonically) — the script fix landed but
  the workflow caveat was only noted in the ledger.
- **2026-06-18 (SHIP question crashed with `options too_small`; owner-reported
  bug).** RED: every run failed at the very first `AskUserQuestion` with
  `InputValidationError: options too_small, expected >=2`. Root cause: step 1
  built the SHIP options as one-per-folder, and `reference/in-game/` currently
  holds exactly ONE ship folder (`Clipper`), so the call had a single option —
  but `AskUserQuestion` requires 2–4 explicit options and the harness's automatic
  "Other" does NOT count toward that minimum. The skill explicitly told the agent
  to "rely on the automatic 'Other'", which is the trap. Fix: added a lead-in rule
  to *Gather inputs* (the ≥2-explicit-options requirement + auto-"Other" doesn't
  count + PAD the list when a discovered set has <2 entries), and patched step 1
  (SHIP — add an explicit "A different ship — I'll name it" option; also skip
  stray files like `ASOP.png`, folders only) and step 2 (SCREEN — same pad rule
  if a folder lists <2 screens). Structural fix (the right form for a wrong-shape
  failure), no red-flag row. The bug reproduced on EVERY run because there is only
  one reference folder today, so it blocked the skill entirely until now.
- **2026-06-18 (verify a re-freeze is actually needed before gating it;
  countermeasures arc, ledger 85; surfaced + applied in a `writing-skills`
  review session).** The countermeasures retro routed all of ledger 84–87 to the
  process ledger and nothing here, so its skill-relevant finding never reached
  this file — the review session that skims open items (the standing "nothing
  consumes this file" trigger) caught it. RED: the agent ASSUMED the non-uniform
  FILL fit change drifted the g-force/velocity platinum baselines and pursued a
  gated re-freeze; `ui_check.sh --full` was already green and the dry freeze
  produced ZERO artifact-hash changes (only `reason`/`frozen_at` churn) — a no-op
  re-freeze, reverted. The skill's freeze gate started from "you've decided to
  re-freeze" and never told the agent to MEASURE whether one was needed, so a
  fresh agent could repeat the wasted motion. Applied: the Checkpoints freeze
  bullet now requires confirming a re-freeze is necessary FIRST (`--full` after a
  fresh export + a dry freeze; green + unchanged hashes ⇒ metadata-only no-op ⇒
  revert, don't gate), and a new red-flag row counters the "my change surely
  drifts the baseline → re-freeze" assumption — the MEASURE-don't-estimate posture
  applied to the re-freeze decision itself. The other three countermeasures
  findings are NOT skill changes: 84 (cockpit-RTT fit-first diagnostic) is domain
  knowledge now in the dossier; 86 (split `engine_01.part`, at the 3000-line cap)
  is a code task, flagged; 87 (derived int/number values don't reach
  `bb_state_filter` IsActive) is a deferred engine residual. OUT OF SKILL SCOPE,
  flagged for the owner / next retro: ledger 85 has the same gap in the
  authoritative `ui-workflow.md` §7 freeze flow (it carries the "unexplained delta
  is rejected" AUDIT RULE but not "verify a re-freeze is needed first") — §7
  should gain it as the canonical home so the skill's restated clause has a
  backing rule.
- **2026-06-17 (reproduce from decoded data, never invent geometry; radar arc,
  ledger 78–83).** When a draw path can't natively produce an element (3D-RTT
  `WidgetWindow`/`Primitive`/live render), the skill's "Default to fixing" + the
  self-status hologram precedent led the radar pass to a procedural rasteriser that
  INVENTED the disc (eyeballed ring/spoke/tilt = banned magic numbers); the owner
  corrected the family ~10× across the arc. The disc was a real engine texture
  (`…radial_gradients.dds`); the hologram had rendered the real decoded MESH —
  only its CAMERA was tuned, so inventing BOTH geometry and camera was never "the
  hologram pattern." This is a NEW failure class, not covered anywhere in the skill,
  so it landed in three places (close the loophole at the lever, the law, and the
  self-check): (1) a one-sentence defuse in *Default to fixing* ("'fixing' means
  rendering the REAL decoded asset — never inventing"); (2) a new **Strict rules**
  bullet — exhaust the in-data art (`mtl_summary`→`image_preview`, `svgFill.svgPath`,
  styleTag `SvgPath`/`ImagePath`, SWF; reference §4/§5) and PROJECT the real asset,
  resolve PER-MANUFACTURER (cascade-applied `PrimitiveMaterialPath`, not the generic),
  place every element from AUTHORED node geometry through ONE shared transform
  (per-element constants = wrong stage), "gated off in the IR" ≠ "absent" (activate
  the real node, don't generate), only a parsed-proven runtime-absent value (the
  camera) is owner-tuned; (3) a red-flag row. The owner had already pushed the full
  detail to ledger 78–83 / workflow §10 / reference §4/§5 / memory
  `feedback-find-indata-sources-before-inventing`; the skill carries the principle +
  rationalization-counters and points at the docs for the probe chain. Cleared the
  Open rec.

- **FOLDED into SKILL.md 2026-07-02** (both entries below: the red-flag row
  "That fix landed — a natural checkpoint…" and the perf side-question bullet
  in the autonomous loop):
- **2026-07-02 (Carrack lift-call console arc) — premature arc pause.** After the
  exporter binding fix landed (blank→renders, 5/8 elements), the session STOPPED
  and asked "want me to take on the button, or is this the stopping point?" —
  despite fully-automated mode, a user-confirmed catalog with the button elements
  on it, and a completed root-cause diagnosis (nothing was blocked). The owner had
  to push ("continue to fix all items until you have parity"). The rationalization
  was "a clean milestone is a natural checkpoint" + context-budget anxiety; neither
  is a skill-sanctioned stop (the only interruptions are the active checkpoints:
  catalog gate, freezes, semi-auto commits). Counter for the SKILL: a landed
  sub-fix is NOT an arc boundary — after each landed fix, re-read the confirmed
  catalog and take the next open item without asking. If context is genuinely
  short, say so explicitly and hand off state to memory — do not convert it into
  a permission question.
- **2026-07-02 — perf side-finding worth keeping.** A "the export got slow"
  side-question inside a parity arc is an optimisation-skill job (it was invoked
  and worked): the decisive early move is pinning BOTH baselines' binary
  provenance (`target/release/deps/starbreaker-<hash>` mtimes = free time-travel
  bisect) before attributing anything to the arc's own changes.

- **2026-07-03 (Carrack console arc / medbed ✕ collateral) — a shared LAYOUT-FORMULA
  fix needs `--full` (post-export) BEFORE the freeze gate; the arc's own test can't
  see siblings.** Centring the medbed close ✕ meant editing a `bb_layout` flex
  cross-axis formula. The first edit (drop `- pivot.x*w` for all Center items) passed
  a fresh unit test AND the arc screen, but regressed the `clipper_countermeasures_master`
  GOLD screen by 3.87 % — its decoy counts (anchor==pivot) relied on that term to cancel.
  Only `ui_check.sh --full` (re-export first) surfaced it, via the whole-image guard's
  per-target %s. The skill already says "render+eyeball every screen sharing a SHARED
  ASSET/ICON/BINDING mechanism" (ledger 77) — extend the mental model: **a shared
  LAYOUT/RENDER FORMULA is the same class of shared mechanism**, and the measurement is
  the `--full` per-target percentages across ALL frozen targets, not the arc test. The
  fix that stuck came from DATA: dumping the sibling screen's nodes showed every
  `pivot.x != 0` node had `anchor.x == pivot.x` while the ✕ uniquely had `anchor.x == 0`
  — that discriminator scoped the carve-out to zero collateral. Also reusable: to locate
  a layout bug, a throwaway `#[test]` that `compile_ir_for_binding` on the real canvas +
  a stage-local `eprintln` beats hand-solving the authored op graph (which here could not
  emit the centred anchor at all). Candidate SKILL folds: (a) *Default to fixing* /
  workflow §5 — name "layout/render formula" alongside asset/icon/binding as a shared
  mechanism that requires `--full`; (b) a probe note: instrument RESOLVED geometry, don't
  solve the op graph by hand. **→ FOLDED 2026-07-07 (A6 restructure) into
  `references/blockers.md` — both (a) and (b).**

- **2026-07-04 (Carrack console re-open) — an owner symptom that doesn't reproduce in a
  FRESH export is a stale VIEW, not a bug to fix.** The owner re-opened the console with
  three items but had prefaced the arc with "you'll need to re-export to get an up-to-date
  version." After that export, 2 of the 3 (button-too-low, SUB DECK missing) did not
  reproduce — all 17 console bindings had a floor, every floor panel localized its heading,
  and the low-button/missing-heading pair existed only in the unused no-floor panel. They
  were a stale export the owner had been viewing. I presented the fresh-export evidence
  (the exact garage="SUB DECK" panel + binding floor coverage) and asked; the owner
  confirmed the re-export fixed them. **Skill fold candidate:** add to the operating
  posture / reproduction guidance — *right after an owner-requested re-export, an
  owner-reported symptom that does not reproduce in the fresh export is most likely a stale
  view; surface the fresh-export evidence and confirm before building any fix.* Chasing the
  ghost would have meant fixing a panel no console uses.
- **2026-07-04 — blast-radius measurement is a function of the CHANGE CLASS.** The one real
  bug (chevrons pure black) was a COLOUR/token fix in `ui_ir` (a token-less `ColorSolid`
  placeholder-black icon `FillColor` inheriting its sibling text field's colour token). A
  colour/token change cannot move anything, so its blast-radius guard is the element-level
  `..._tint_semantics` gold snapshot (MORE sensitive than `--full` whole-image), which ran
  GREEN across all frozen targets — no separate sibling render needed. Contrast ledger 106:
  a LAYOUT-formula change needs `--full` per-target %s. **Skill fold candidate:** where the
  workflow says "render+eyeball siblings after a shared-mechanism change," note that a
  colour/token-only change is already covered by the element-level tint snapshot, while a
  layout/render-formula change needs the `--full` per-target percentages.
- **2026-07-04 — two small measurement notes.** (a) A thin-glyph colour claim (the caret's
  double-caret strokes) is unmeasurable by a MEAN — anti-aliasing against the bright button
  gave `(35,57,40)` for both black and dark-green; the discriminator was the COUNT of
  near-pure-black pixels (`max(r,g,b)<25`) in the region: 395 → 0. (b) A width-fit heading's
  SIZE is judged on the horizontal axis the fit constrains (aspect-invariant), not the
  cap-height (which the mesh aspect stretches) — this cleanly answered "is SUB DECK too
  small?" as "no, the width matches; the height gap is the deferred aspect + bloom."
  **→ (a) FOLDED 2026-07-08 (T9) into `references/blockers.md`; (b) SUPERSEDED by the SAME-arc
  correction below (the heading is FIXED, not width-fit) — that correction's rules are in
  `catalog.md`.**
- **2026-07-04 (SAME arc, correction) — that width-match answer was WRONG; the owner was
  right.** The heading is a FIXED Heading3 (28px, `autoScalingMethod:None`, no `autoFontSize`),
  NOT width-fit, so an 80%-width match proves nothing about its size. The owner repeated "still
  too small"; re-measuring properly (connected-component button isolation, cap-h compared to the
  CALL ELEVATOR sibling as a ratio) showed the reference draws SUB DECK ≈ CALL cap-h (~1.10) vs
  the render's 0.636 — genuinely ~1.35× too small, not aspect. **Skill folds:** (a) before using a
  width-match to argue a heading's size is correct, CHECK width-fit vs FIXED — a fixed heading's
  width match is irrelevant to its size; (b) size-compare a text element to a SIBLING text element
  (cap-h ratio), which is aspect- and resolution-invariant, not to the aspect-stretched image
  height; (c) uniform-colour screens (all-green transit panels) defeat colour thresholds — use
  connected-component region isolation and require a measurement to survive a second, cleaner
  method before trusting it; (d) **when the owner repeats a visual complaint, treat it as ground
  truth and re-derive from scratch — do NOT re-defend the prior call.** This is my standing
  weakness (visual/spatial judgment from renders); the memory note says trust the owner + use DATA,
  and here I initially did neither. The fix itself (caption size → its 64px field, the compass
  height-driven pattern; plus the hard-coded `text_draw.rs` heading table) was owner-DEFERRED.

- **2026-07-07 (plan A6 — skill restructure; applied via `writing-skills` review).** `SKILL.md`
  552 → 168-line navigation core + `references/{launch,catalog,blockers,freeze,retro}.md`, each
  read on-demand at its phase (an agent in the LOOP never loads launch/retro). Relocation AUDIT:
  all 27 original red-flag rows mapped to a destination (the two measurement rows — png-md5
  ledger 69, `--full`-green ledger 77 — re-verified present); load-bearing prose rules (≥2-option
  padding, sequential dependent questions, don't-land-fix-pre-gate, stage table, never-delegate,
  blocker evidence trail, dry-freeze no-op, retro categories) all preserved. New content folded in:
  (1) the two 2026-07-03 candidates above — layout/render formula = shared mechanism needing
  `--full`, and instrument-RESOLVED-geometry-via-throwaway-`#[test]`+`eprintln` — into
  `references/blockers.md`; (2) the 2026-07-04 refinements — blast-radius-by-CHANGE-CLASS
  (colour/token → element tint snapshot; layout → `--full` per-target %s), width-fit-vs-FIXED
  heading + sibling cap-h ratio + connected-component isolation + owner-repeats-complaint, and
  stale-view-after-re-export — into `blockers.md`/`catalog.md`; (3) the spec's INTERIM colour rule
  (blend-shaped edge residuals judged leniently + registered as §6 outliers pointing at plan B4)
  into `blockers.md` + the core strict rules. The loop is now wired to the A1–A5 tools:
  `ui_arc_status.sh` (render+compare+flag), the `ui_variant_styles` MCP tool as the FIRST step of
  the font/size/colour drill, and `ui_blocker_evidence.py` (+ `blocker-evidence-schema.md`) as the
  required attachment for a gated blocker claim. Behavioral RED/GREEN test is plan A8 (the fresh
  Opus compliance dry-run narrating a parity arc from the restructured skill).

- **2026-07-08 (plan T9 — ledger-107 folds + wait-on-marker + interim-rule RETIREMENT; applied
  via `superpowers:writing-skills` review).** Audit-then-add pass over `blockers.md` + `catalog.md`
  (both read in full first). ALREADY PRESENT, so NOT restated: blast-radius-by-CHANGE-CLASS
  (colour/token → element tint-semantics snapshot; layout → `--full` per-target %s; ledger
  77/106/107) in `blockers.md` Shared-mechanism rules; and width-fit-vs-FIXED heading + sibling
  cap-h ratio + connected-component isolation + owner-repeats-complaint in `catalog.md` (A6).
  Genuinely-missing pieces FOLDED: (1) thin-glyph colour by near-pure-black pixel COUNT
  (`max(r,g,b)<25`), not a mean — the caret 395→0 discriminator → `blockers.md` colour-measurement
  note (ledger 107); (2) fresh-export REPRODUCTION gate — the owner symptom must reproduce in a
  FRESH export before any fix, else it is a stale VIEW → a compact cross-ref line in `blockers.md`
  pointing at `catalog.md`'s stale-view rule (completes what the A6 entry claimed for `blockers.md`
  without restating the detail); (3) the spec §4A1 evidence-bar extension — a "done" / "within
  tolerance" close-gate verdict needs the same class of MEASUREMENT evidence as a blocker, not an
  eyeball (the SUB DECK width-match miss as the cautionary case) → one line in `catalog.md`'s
  closing re-review (measurement generally; Task 11's residual-budget doc adds its own pointer
  later); (4) the wait-on-marker rule — long-running commands (`ui_render.sh`, exports, `--full`)
  run via the harness background facility with completion detected by the command's exact terminal
  marker in its log, never a sleep-loop / turn-hold / regex looser than the marker string →
  `ui-workflow.md` §3 (one block) + a cross-ref clause on the SKILL Operating-posture MARKER
  bullet. RETIRED (B4 linear-light compositing LANDED, `999d485ab`, all 15 gold/platinum baselines
  re-frozen): the interim "blend-shaped colour residuals judged LENIENTLY and parked as §6
  known-outliers pointing at B4" rule — replaced in `SKILL.md` strict rules + `blockers.md` with
  "colour residuals judged NORMALLY; a genuine blend-shaped edge drift is a latent bug to
  root-cause and adjudicate (§5)." `writing-skills` review applied: reference/measurement guidance
  written as recipes not prohibitions (Match-the-Form-to-the-Failure), cross-references over
  restatement, keyword coverage preserved, `SKILL.md` ≤220 lines held. Every cited command/marker
  verified on write: `ui_check: ALL GREEN`, `ui_arc_status: OK (…)`, `ui_render.sh` `latest -> …`,
  `ui_blocker_evidence.py`, `live_manifest_targets_match_gold_standard_tint_semantics`.

- **2026-07-08 (alignment plan T15 — dossier-computed SCOPE&MODE default + persona
  reconciliation; applied via `superpowers:writing-skills` review).** `references/launch.md`
  step 4 now COMPUTES the recommended Mode from the chosen SCREEN's dossier row: "known" =
  row COMPLETE (`preset`/`tier`/`target_id` all non-null) AND its manufacturer already has
  ≥1 OTHER frozen GOLD/PLATINUM screen — manufacturer = the `scene_package` prefix
  (`DRAK`/`ANVL`), DERIVED from the row, never a hard-coded ship→brand map → recommend
  fully-automated; first-of-a-kind (new manufacturer / new widget family / no dossier row)
  → semi-automated. Presented as the FIRST `AskUserQuestion` option (owner direction 2026-07:
  auto for KNOWN ships, hands-on for first-of-a-kind); the ASK never disappears. `SKILL.md`
  strict rules gained the two always-on-persona reconciliation lines: (a) a **ponytail note**
  (laziest-that-works = smallest change still engine-faithful + data-derived; a hard-coded
  value / invented geometry / name-gate / heuristic is never "lazy" here — it trips the
  guards), and (b) **this skill IS the process skill for its arc** (its launch/catalog gates
  stand in for superpowers:brainstorming's design gate; superpowers planning/TDD/debugging are
  sub-tools WITHIN the loop). `writing-skills` pass: persona lines written as a REFRAME +
  consequence (Match-the-Form-to-the-Failure — not a bare prohibition), skill-name cross-refs
  (no `@` force-loads), bold scannable leads, keyword coverage kept, `SKILL.md` 179 ≤ 220.
  Verify-on-write: the documented dossier snippet was EXTRACTED from `launch.md` and RUN
  against the real `ui_screen_dossier_v1.json` — `Screen_Right_Upper_RTT` / `Screen_Left_Lower_RTT`
  (DRAK, complete) → fully-automated; `console_liftcall_009` (ANVL, first-of-a-kind), a
  no-dossier-row screen, and a synthetic new-manufacturer row → semi-automated. Battery:
  `ui_check: ALL GREEN`, `validate_ui_dossier: OK (18 screens)`.
