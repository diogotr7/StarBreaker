# FREEZE / COMMIT — the gated actions (phase 5)

STOP before ANY freeze. At each ACTIVE checkpoint, present the evidence, then ask
permission via `AskUserQuestion` (a concrete approve/decline choice). Never proceed on
a presumed "yes"; never perform the action before the answer.

## Baseline freeze / re-freeze — ALWAYS gated, both modes

§6/§7 are APPROVAL-GATED. Show the per-identity delta, then ask via `AskUserQuestion`
("Freeze these N identities?" → "Freeze" / "Don't freeze"). Never auto-approve, never
freeze a value you can't explain.

**First MEASURE that a re-freeze is even needed** — a change you *assume* "drifts a
baseline" may sit within the captured metric tolerance. Run `ui_check.sh --full` (after
a fresh export) and a dry freeze; if `--full` is green AND the artifact hashes are
unchanged, the re-freeze is a metadata-only no-op that just overwrites the prior
`reason`/`frozen_at` → REVERT the churn, don't bring it to the gate. Re-freeze only when
a guard actually FAILS or to deliberately pin an improvement the owner wants (ledger 85).

A wide change that moves many frozen baselines TOWARD the reference is the workflow §5
"baseline is wrong → re-freeze" path — present that delta at the freeze gate.

## Major-item blocker — ALWAYS gated, both modes

Before accepting that a MAJOR/dominant item (one that bounds achievable parity — a whole
region empty/wrong, a dominant element) is a PROVEN blocker, present the evidence trail
(a file that `scripts/ui_blocker_evidence.py` accepts — see `blockers.md`) and ask via
`AskUserQuestion` ("Accept as blocked?" → "Accept" / "Research further"). Giving up on a
major item is consequential — like a freeze; never self-certified. (Minor residuals
deferred with proof don't need this gate.)

## Git commit

- **Semi-automated:** show the diff/summary, ask "Commit this?" → "Commit" / "Not yet",
  commit only on yes.
- **Fully automated:** commit autonomously per coherent fix (message cites the catalog
  item), no question.

## Final parity

Driven by the closing re-review (`catalog.md`). Semi-automated: present that fresh
re-review and ask whether parity is acceptable or another pass is wanted. Fully
automated: the re-review keeps fixing until clean or the remainder is proven
deferred/blocked, then finishes (no gate).

## Guards are never silenced

Frozen platinum/gold guards are never silenced by editing tests/baselines; baselines
move only through the audited freeze flow or a §6 known-outlier.

## Red flags — freeze

| Thought | Reality |
|---|---|
| "Freeze it to pass the guard / I'll freeze without asking / fully-auto so auto-freeze" | Freezing a baseline is ALWAYS gated in BOTH modes (show the per-identity delta, ask via `AskUserQuestion`). Never auto-approve, and never freeze a wrong value to silence a guard — fix the cause or register a §6 outlier. |
| "My change surely drifts that baseline — I'll re-freeze it" | MEASURE before assuming. Run `--full` (fresh export) + a dry freeze first; if `--full` is green and the artifact hashes are unchanged, the re-freeze is a metadata-only no-op (only overwrites the prior `reason`) → revert the churn, don't gate it. Re-freeze only when a guard actually FAILS or to pin an improvement the owner wants (ledger 85). |
| "They'll obviously say yes, I'll just do it" | Ask anyway via `AskUserQuestion` at an active checkpoint. A presumed approval is not an approval. |
