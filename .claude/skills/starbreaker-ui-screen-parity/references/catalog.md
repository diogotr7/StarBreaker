# CATALOG — build & confirm (phase 2), and the closing re-review (phase 4)

Same procedure, run twice: once at arc start to build the catalog, once at the end
to re-review from scratch.

## Build & confirm the diff catalog (phase 2 — both modes)

Do this once at arc start, after the required reads, whether the screen is fresh or
worked before.

**Inherited verdicts are hints, not conclusions.** A prior arc's open issues, or a
dossier/memory/handoff label of "faithful / owner-confirmed / not flagged", SEEDS
this pass but never replaces it — such a label is a prior judgment that may have
under-scrutinized, or that the owner's view has since outgrown. Re-derive each
region's verdict from the reference at HIGH ZOOM (small features — circle vs
squircle, a few-dozen-px marker offset — hide in a naked side-by-side). For a region
under a frozen baseline or a registered known-outlier, route any re-opened verdict
through the §5/§6/§7 audited flow (adjudicate → re-freeze / known-outlier), not a
silent fix.

Steps:

1. **Render + compare → build the catalog.** `bash scripts/ui_arc_status.sh --screen
   <id>` renders, compares over the dossier preset, and flags CHANGED/NEW/same
   regions — vision-read the FLAGGED crops. Build the numbered diff catalog: region |
   difference | severity | root-cause hypothesis | fix-or-defer.
2. **Self-verify every finding — look AGAIN before trusting it.** Re-open the crops
   and re-check each item: is the difference real and correctly described? Findings
   are mostly right but occasionally outright wrong — guard specifically against
   misreading SHAPE (square vs circle vs ring), COUNT, presence/absence, and the
   DIRECTION of an offset. Drop or correct any finding that doesn't survive the
   second look; measure (`ui_measure.py`) when unsure rather than eyeball. Size-compare
   a text element to a SIBLING text element (cap-height ratio — aspect- and
   resolution-invariant), NOT to the aspect-stretched image height; and a FIXED
   heading's width match says nothing about its size (check width-fit vs fixed first).
   Uniform-colour screens defeat mean-based colour thresholds — use connected-component
   region isolation and require a measurement to survive a second, cleaner method.
3. **Check the BACKGROUND / backplate layer explicitly** — not just foreground
   widgets. Is the background image stretched, wrong-aspect, scaled, offset,
   mis-aligned, or cropped vs the reference? Add any background diffs; they are easy to
   miss and skew everything layered on top.
4. **Confirm with the USER via `AskUserQuestion`.** Present the self-verified catalog,
   ask to confirm or amend, invite free-text ("Other"). Do NOT start fixing until
   confirmed — this gate runs in BOTH modes.

**Diagnose freely, but don't LAND a fix pre-gate.** Investigating/root-causing —
including writing the characterizing FAILING TEST — is catalog-building and fine
before the gate. Do NOT land a source FIX until the catalog is confirmed (and the
commit always waits for the commit gate): the gate exists so the user can correct
WHAT's wrong before source changes accrue. After the gate, the loop applies fixes
directly (no per-fix permission).

**Right after an owner-requested re-export, an owner-reported symptom that does not
reproduce in the FRESH export is most likely a stale VIEW, not a bug** — surface the
fresh-export evidence (the exact panel/binding coverage) and confirm before building
any fix. Chasing the ghost fixes something nothing uses (ledger 107).

## Closing re-review (phase 4 — before the retrospective, both modes)

When the catalog is resolved, do NOT trust it is done — re-evaluate from scratch,
exactly as the build phase did:

1. **Re-render fresh** and **re-run compare + self-verify** against the reference:
   look AGAIN (shape/count/offset), re-check the BACKGROUND/backplate layer. Build a
   fresh catalog of what REMAINS — including anything the fixes introduced or missed.
   If the arc touched a SHARED asset/icon/binding/layout-render-formula mechanism, also
   re-render and EYEBALL the OTHER screens that share it (e.g. all MFD footers) — small
   chrome regresses below `--full`'s ~1% budget (ledger 77; see `blockers.md`).
2. **Fully automated:** feed any fixable difference back into the loop and FIX it;
   repeat re-render → re-review → fix until the screen is clean — every difference
   fixed or carrying a PROVEN deferral/blocker (size/risk is not a deferral; freeze
   stays gated). Do not finish while fixable diffs remain.
3. **Semi-automated:** this fresh re-review IS the Final-parity checkpoint — present
   it via `AskUserQuestion` ("parity acceptable" vs "another pass", with free-text);
   "another pass" resumes fixing, then re-reviews again.

**The retrospective is the LAST step, never a substitute for fixing.** Any issue the
closing re-review surfaces is fixed in the loop FIRST. Do NOT enter the retro to
"wrap up" with a fixable-but-unfixed diff open. The arc proceeds to the retro only
once the closing re-review is clean, OR every remaining diff carries an
exhausted-search proven blocker (see `blockers.md`).

**A "done" or "within tolerance" verdict needs evidence too — the same class as a blocker.**
Closing an item as clean or within tolerance requires a MEASUREMENT (a sibling cap-height ratio,
connected-component isolation, `ui_measure.py`, the per-target `--full` %s), not an eyeballed
"looks close": the SUB DECK "within tolerance" call was wrong twice on a width-match coincidence
before a proper cap-h ratio proved it ~1.35× too small (ledger 107). Judge a residual on data
before you accept it.

A proven-blocked remainder IS a valid terminal state — "deferred with proof, not
clean": some screens have intrinsic engine-mechanism limits that bound achievable
parity this arc. A MAJOR/dominant blocked item is surfaced for user confirmation
first (gates), never self-accepted; once confirmed, a fully-automated run STOPS there
— it has converged — it does not loop forever trying to fix the unfixable.

## Red flags — catalog

| Thought | Reality |
|---|---|
| "First glance: a square / shifted left / only the foreground's off" | Look AGAIN before cataloguing (shape/count/offset misreads are the wrong ones — measure if unsure) AND check the background/backplate layer (stretch/scale/aspect/align/crop — the commonly-missed layer that skews everything on top). |
| "Memory says this region is faithful/owner-confirmed — skip it" | Inherited verdicts are hints, not conclusions. Re-derive each region from the reference at high zoom; the owner's view evolves and earlier passes under-scrutinize. Frozen/outlier regions route through §5/§6/§7. |
| "The findings look right, start fixing" | First self-verify (look again + background), then confirm the catalog with the user via `AskUserQuestion`. Both modes, every arc. |
| "When the owner repeats a visual complaint I'll re-defend my prior call" | Treat an owner's repeated visual complaint as ground truth and re-derive from scratch with DATA — do NOT re-defend the prior call (visual/spatial judgment from renders is a known weakness). |
| "This render looks identical to the last — my change did nothing" | `ui_arc_status.sh`/`ui_render.sh` write a UNIQUE dir per run (so successive renders don't collide), but if you copy one to a STABLE name to show the user, the viewer caches by name and shows the OLD image — confirm an on-disk change via the printed `png md5:` line before concluding no-op (ledger 69). |
