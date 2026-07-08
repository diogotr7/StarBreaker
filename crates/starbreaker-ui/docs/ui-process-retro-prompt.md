# UI process retrospective — end-of-arc prompt

The companion to `ui-matching-agent-prompt.md`: that one starts a parity
arc fresh; this one is pasted **at the end of a working session, to the
agent that did the work** — the retrospective runs on lived context (the
friction, dead ends, and retyped commands only that session knows). The
ledger of findings and their implementation state is
`crates/starbreaker-ui/docs/ui-process-improvements.md` — retros APPEND to it, they don't start
new documents.

## Instructions

Consider the work you have done in this session/arc: what worked, what
didn't, and how the process can be improved — updating tools, creating new
ones, and updating documentation with enough information to bootstrap the
process so the next session needs less research at the start.

First read crates/starbreaker-ui/docs/ui-process-improvements.md (the ledger — you will APPEND
to it, matching its Observed/Improvement/Action format as a self-contained retro
entry) and skim crates/starbreaker-ui/docs/ui-workflow.md + crates/starbreaker-ui/docs/ui-reference.md so proposals
amend the current process rather than reinvent it. Use this session's own
experience as the primary evidence; use the arc's git log, handoff, and
memory file to recall anything context compaction has blurred.

Sweep these categories against what actually happened:
1. Repeated manual work -> tooling. Anything you typed more than twice
   (ad-hoc crops, hand-rolled diffs, probe greps, command batteries)
   becomes or extends a script/example. Check scripts/ and the examples/
   diagnostics first — extend before creating.
2. Silent failure modes -> loud ones. Any harness/checker/guard that gave
   you a wrong-but-plausible answer (zero matches reported as data drift,
   format rot as MISSING) gets a distinct hard failure.
3. Documentation drift. Every doc claim you relied on that was wrong or
   stale gets fixed with verify-on-write (run the commands; repo-wide grep
   for references to anything renamed/deleted, in the same commit). Never
   leave parallel half-truths; the docs_reference_guard test must cover any
   new doc that cites files.
4. Bootstrap cost. List everything you had to RE-DERIVE this session that
   the docs should have carried (data locations, screen mappings,
   engine-model rules, don't-retry traps, probe names). Land each in
   crates/starbreaker-ui/docs/ui-reference.md (dossier rows, probe registry, glossary) or
   crates/starbreaker-ui/docs/ui-workflow.md §10 (pain points / don't-retry) so the next session
   starts warm.
5. Guard/freeze friction. Adjudications that took detours, deltas you
   audited by hand, outliers that should have been registered earlier —
   improve the flow or the docs that teach it.
6. Memory/handoff quality. Would the handoff you wrote (or inherited) have
   been enough to resume cold? What was missing after compaction? Fix the
   handoff expectations in crates/starbreaker-ui/docs/ui-workflow.md §9, not just this arc's file.
7. Slow tooling -> profiled speedup. A diagnostic/harness/check you WAITED ON
   repeatedly (seconds-to-minutes per run) is iteration tax, not a fixed cost —
   profile it, don't tolerate it. Do it in this order:
   (a) MEASURE before changing anything: count/size the inputs and time the
       phases (add a phase banner if one is missing) so you cut the real
       bottleneck, not the suspected one — a slow run is harness load far more
       often than a pipeline loop (ledger 42).
   (b) Prefer an EXISTING faster path first: an indexed MCP/DataCore tool beats
       a mirror-walking example; tool choice beats optimisation, and the §10
       guidance + reference tool registry should point there.
   (c) If the slow tool must stay, cut the dominant cost: scope the work to what
       is actually fetched (don't index the whole mirror), replace a full parse
       with targeted field extraction, MEMOISE repeated fetches mirroring the
       production fetcher (a hot record re-read+re-parsed per call is the usual
       hidden cost), and parallelize.
   (d) VERIFY by measurement AND that the output is unchanged — a speedup that
       drifts behaviour is a regression, not a win. Worked example: ledger 42
       (mfd_ir_dump ~94s -> ~5s: subtree index + parallel head-scan + memoising
       fetcher).

Then:
- APPEND the findings as new numbered items to
  crates/starbreaker-ui/docs/ui-process-improvements.md — self-contained retro
  entries (Observed/Improvement/Action, with the implementation inline). Do NOT
  start a new phased plan: the ledger abandoned that genre around item 52 (see
  its Current-truth index). For each recurring lesson, the DEFAULT action is to
  convert it into an ENFORCED gate/test/probe/tool — the way the recurring
  "inherited 'proven blocker' was under-research" lesson became the
  blocker-evidence gate (scripts/ui_blocker_evidence.py) + the `ui_variant_styles`
  probe (ledger 108 A4/A5), not another prose bullet. A prose-only bullet is the
  LAST resort and must state WHY a gate/test/probe/tool is not possible.
- IMPLEMENT the plan: quick tooling wins first, then docs, then automation;
  one commit per coherent item citing its ledger number; verify-on-write
  for every doc change; bash scripts/ui_check.sh green per commit. Process
  changes must not alter render behaviour — anything that would goes
  through the normal TDD/guard flow of crates/starbreaker-ui/docs/ui-workflow.md instead.
- Baseline-affecting actions (TSV/freeze re-captures) are APPROVAL-GATED:
  present the deltas and stop unless approval was given up front.
- Mark executed steps [done <date> <commit>] in the ledger; update the
  session memory pointers.

Acceptance (the bootstrap test): after implementing, dry-run the
per-screen prompt (ui-matching-agent-prompt.md) from the docs alone —
every command, path, and mapping a fresh agent needs must resolve without
leaving crates/starbreaker-ui/docs/ui-workflow.md + crates/starbreaker-ui/docs/ui-reference.md + the dossier. Any
excursion YOU needed during this retro is a doc bug: fix it before
closing.