# LAUNCH — gather inputs + required reads

STOP here at phase 1. Every invocation starts COLD: ask SHIP, SCREEN, REFERENCE,
SCOPE fresh via `AskUserQuestion` — never inherit a choice from earlier in this
session or a prior arc, even when you already "know" the answer. Prior context is
a hint to surface, never a substitute for asking.

**Ask as separate, SEQUENTIAL questions — never batch dependent ones.** Each
answer determines the next question's options: the ship picks the screen list, and
**the chosen SCREEN determines which reference images are discovered.** Do NOT
pre-compute a later question's options from a presumed earlier answer — the classic
bug is offering the reference for a default screen instead of the one the user just
picked. Wait for each answer before building the next question. Only SCOPE & MODE
(independent of each other) may share one prompt.

**`AskUserQuestion` needs 2–4 explicit `options`; the auto-appended "Other" does
NOT count** — a one-option call fails with `InputValidationError: options
too_small, expected >=2`. When a list built from discovered files/folders has fewer
than two entries, ADD an explicit extra option (e.g. "A different ship — I'll name
it"); the user still gets free-text "Other" on top. This bites the SHIP question
today: `reference/in-game/` holds only one ship folder (`Clipper`), so a naive
"one option per folder" call is invalid. Never skip the question to dodge this — PAD.

1. **SHIP** — list the *folders* (not files — `ASOP.png` is a stray, skip it)
   under `~/projects/scorg_tools/reference/in-game/` (each folder = one ship's
   reference set, e.g. `Clipper` = Drake Clipper) as the options; pad to ≥2. Ask
   **even when only one folder exists** — a single populated folder is NOT a licence
   to auto-select, no matter what the session was previously working on. Resolves to
   `reference/in-game/<folder>/`.
2. **SCREEN** — ask fresh even if a screen was confirmed earlier. List the distinct
   file stems in the confirmed ship's folder (e.g. `Screen_Right_Upper_RTT`,
   `self_master`, `compass_master`); the auto "Other" covers a screen with no capture
   yet (adding its dossier row is then part of the work). Same ≥2 rule; if a folder
   holds more than the 4-option limit, show the full list in the message and offer
   the most relevant. The chosen SCREEN is the reference stem AND the dossier row;
   the render `--helper` comes from the dossier's **Helper/scene column**, usually —
   but NOT always — the same name (velocity-num: stem `ship_velocity_num_master`,
   helper `screen_flight_hud_left_upper`). When they differ the dossier is authoritative.
3. **REFERENCE — resolve, SHOW, then CONFIRM (hard stop).** ONLY after SCREEN is
   answered, list files matching that CHOSEN screen — never a presumed one. Prefer
   the straight-on capture carrying a `<name>.corners.json` sidecar over a legacy
   name (reference §3 — e.g. power uses `Screen_Left_Lower_RTT_dark.png`).
   **Read the chosen image and show it**, then confirm via `AskUserQuestion` ("Yes,
   use this" / "Pick a different file"; "Other" points at another path). Do NOT
   continue until confirmed.
4. **SCOPE & MODE** — two questions, one prompt:
   - **Scope:** "Full review of every region" vs "Specific issues I'll name" (the
     free-text captures observed symptoms; named issues SEED the catalog, the review
     still surfaces the rest).
   - **Mode:** "Semi-automated — gate commits and freezes" vs "Fully automated —
     commit automatically, gate only freezes." Freezing is gated in BOTH modes; the
     mode only changes whether commits and the final parity check pause.
   Then build/seed the catalog and **order it by priority (workflow §4:
   structural/layout before styling; shared-root-cause items together) and work it
   top-down — never pause to ask which item next.** A HANDOFF doc named in the
   dossier's open-issues column is read either way.

## Required reads (before any fix)

In order: `StarBreaker/AGENTS.md` → `crates/starbreaker-ui/AGENTS.md` →
`ui-workflow.md` (the process — read its "Read WHEN" index, then §1 rules in full)
→ `ui-reference.md` (its index, then find SCREEN in the §3 dossier: scene/LOD,
canvas, compare preset, frozen tier, open issues). SCREEN not in the dossier → add
its row (JSON `ui_screen_dossier_v1.json` + §3 table, `validate_ui_dossier.py`
green) as part of the work.

## Red flags — launch

| Thought | Reality |
|---|---|
| "I'll presume/reuse the ship, screen, or reference" | Every run starts COLD — ask SHIP, SCREEN, REFERENCE, SCOPE fresh via `AskUserQuestion` (even with one folder; never reuse a prior choice or skip the reference confirmation). |
| "Batch the screen + reference questions to save a round-trip" | They're dependent — the REFERENCE options come from the SCREEN answer. Ask sequentially; batching offers the reference for a presumed screen. |
