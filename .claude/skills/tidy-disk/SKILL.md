---
name: tidy-disk
description: Use when asked to free disk space, tidy storage, prune build/bench artifacts, or when the disk is nearly full on a box running StarBreaker (df >85%, cargo build failing for space, target/ or bench trees ballooning).
---

# Tidy Disk (StarBreaker / scorg_tools)

Free disk space in this workspace — **without ever deleting anything unapproved, load-bearing, or that is the last copy of something.** This is a survey-then-ask workflow: you never `rm` on your own judgement, no matter how obvious the candidate looks.

## Iron rules (violating the letter IS violating the spirit)

1. **Permission for EVERY deletion.** Nothing goes until the user approves it in an `AskUserQuestion` (candidates batched into categories; `multiSelect: true`; one option per category). "AGENTS.md authorizes this class", "it's oracle-verified disposable", "the build will fail" — none substitute for approval. No answer = no space freed; report and stop.
2. **Scope fence.** Delete only inside (a) the locations mapped below, (b) files this session wrote (scratchpad, temp scripts). Everything else — `~/.cargo`, `~/.cache`, other projects, system dirs, Blender config, the Star Citizen install / `Data.p4k` — is OUT OF SCOPE: report its size if asked "what else is big", never modify it.
3. **Never the bisect ladder.** The bare `starbreaker-<hash>` CLI binaries under `target/release/deps` (~0.5G) are the time-travel-bisect ladder (ledger item 4). Never delete them, and never run `cargo sweep` on `release/deps` — its time cutoff eats old ladder binaries. Stale `.rlib`/`.rmeta`/`.so` generations there ARE proposable (keep the newest generation per crate name).
4. **Never the current oracle baselines.** The `ships_perf_bench/<label>` dirs named as current by the LATEST pass entry in `docs/optimisation-ledger.md` are the byte-identity guard for all perf work. Superseded labels (e.g. a retired `*_post1`) are proposable only with that supersession quoted from the ledger — and note in the option that they are the only pre-<event> byte reference if that's true.
5. **Never `ships/`.** The canonical export root: `scene.blend` files LINK mesh libraries from `ships/Data`, so deleting shared data corrupts every saved Blender scene; `ships/dcb_canvas` feeds the live-IR test harness (deleting silently disables that guard); the `ships/Data` objects/Objects case-dup (~1.5G) is a standing owner-blocked ledger item — mention it as blocked, never propose it as approved.
6. **Never under a live build/bench/Blender.** Check for running `cargo`/`rustc`/`starbreaker`/`blender` processes and 1-min load first. A bench in flight holds its `target/tmp` dir; Blender holds `ships/` libraries open; concurrent cargo runs race `target/`.
7. **Never repo content or generated knowledge.** The git repo, `docs/`, `graphify-out/`'s live set (`graph.json`, `cache/`, `cost.json`, `manifest.json`), `reference/`, `renders/`, `assets/`, `blender/` are not disk-cleanup candidates. Dated `graphify-out/20*/` snapshots beyond the 3 newest are proposable (retention policy), but still asked.

## Known locations (the map)

| Location | What it is | Deletable class |
|---|---|---|
| `StarBreaker/target/tmp/{bench_,oracle_,aug,det}*/` dirs | Bench/oracle scratch exports (2.5–3.2G each; regrow in ~20–35s) | Proposable once no bench is running; keep the `*.log` files — ledger stage tables cite them |
| `StarBreaker/target/debug` | Debug build tree (tens of G) | `cargo sweep --time 30` ONLY — never blind `cargo clean`; an active tree reclaims little (atime-based), say so |
| `StarBreaker/target/release/deps` | Bisect ladder (bare `starbreaker-<hash>`, NEVER) + stale dep generations | Stale generations only, newest per crate kept; NO cargo-sweep here (rule 3) |
| `StarBreaker/target/perf` | Fat-LTO perf-profile build (~0.6G; ~4.5 min rebuild) | Proposable with rebuild cost stated |
| `ships_perf_bench/<label>` | Byte-oracle baseline exports | Superseded labels only, ledger quote required (rule 4) |
| `ships/` | Canonical shared export root + `dcb_canvas` | NEVER (rule 5); case-dup item stays owner-blocked |
| `graphify-out/20*/` dated snapshots | Graph history | Beyond 3 newest proposable; live set NEVER (rule 7) |
| Session scratchpad | This session's scratch | Proposable |
| `~/.cargo`, Blender config, SC install, other projects | — | OUT OF SCOPE (rule 2) |

## Procedure

1. **Survey + pre-checks**: `scripts/survey.sh` → read-only JSON (df, per-location sizes, target/tmp dirs vs logs, ladder inventory, bench labels with the ledger's current-baseline line, graphify snapshots, busy processes + load). The script decides nothing; judge the JSON. **Never propose anything the JSON flags `protected: true`**; any `busy` hit blocks its candidates.
2. **Categorise** every candidate: paths, size, what it is, why deletable, the risk, and the recovery path (re-export ~30s / rebuild ~minutes / re-bench / none).
3. **Ask** — one `AskUserQuestion`, `multiSelect: true`, one option per category, sizes and risks in the descriptions. Include KEEP-flagged near-candidates (ladder, current baselines, ships/Data case-dup) so the user sees what you deliberately excluded.
4. **Delete approved categories only**, by explicit literal paths — no `*` globs that could sweep a protected sibling (a glob under `release/deps` is exactly how you lose the ladder).
5. **Verify**: `scripts/verify.sh` — df, ladder count unchanged, current baselines present, `ships/dcb_canvas` present, graphify live set present, no half-deleted bench dir. Non-zero exit names the failing check → stop and report immediately.
6. **Report**: freed total per category; what was kept and why; remaining large consumers (out-of-scope ones as observations only).

## Rationalization table (none is valid)

| Excuse | Reality |
|---|---|
| "AGENTS.md's retention policy already authorizes this class" | The policy names WHAT is safe; this skill's rule 1 still requires the ask. Policy ≠ approval. |
| "It's byte-identical to a kept baseline, verified" | Then it's a strong candidate with low risk — in the ask. Not a self-service delete. |
| "The bench dirs regrow in 30 seconds" | Regrowth needs a quiet machine and a valid binary; the user may want the exact dirs for a diff. Ask. |
| "cargo clean is faster than cargo sweep" | Blind clean nukes the incremental tree AND (via release) risks the ladder. Sweep debug only, and only after the ask. |
| "The *_post1 baselines are superseded" | Retirement is a recorded owner decision (ledger). Quote it or treat as pending. |
| "It's only logs / only a few MB" | Small deletions still get batched into the ask. The rule is EVERY item. |
| "I cleaned the same class earlier with approval" | That approval covered those items then, not the class forever. |

## Red flags — STOP

- You are typing `rm` and no AskUserQuestion in this session approved that exact category.
- The path is under `ships/`, matches `target/release/deps/starbreaker-*` with no extension, or is a baseline the ledger's latest pass names as current.
- A `cargo`, `starbreaker`, or `blender` process is running and the candidate is something it could hold open.
- You're about to delete the only remaining pre-<change> byte reference "because git has the code" — the code isn't the bytes.
- The path is outside the map and wasn't written by this session.
