#!/usr/bin/env bash
# perf_provenance.sh — perf comparison baseline-provenance gate (alignment plan T8).
#
# RESPONSIBILITY
#   Print the identity of a binary that is about to be used as ONE endpoint of a
#   before/after perf comparison, so a STALE binary can never masquerade as a
#   fresh baseline. Two separate incidents each cost sessions of wasted analysis:
#   a "42s baseline" measured on a binary that predated the change under test,
#   and a worktree "build" that finished in 0.21s and then timed a byte-identical
#   binary. Running this on BOTH endpoints and diffing the outputs catches that
#   whole class: an identical sha256 across endpoints means you timed the SAME
#   binary twice; a binary mtime older than HEAD's commit means it predates the
#   change. The optimisation skill (starbreaker-optimisation, Measurement rules)
#   GATES on this — run it on both endpoints before ANY timing claim, include
#   both outputs; mismatched or unknown provenance invalidates the comparison.
#
# USAGE
#   perf_provenance.sh <binary>     print path/size/mtime/sha256/head; marker OK
#   perf_provenance.sh --self-test  nonexistent binary -> FAILED; real -> OK
set -euo pipefail

# RESULT MARKER via EXIT trap — survives `| tail`/`| grep` (ledger 89): a pipe
# would otherwise report the filter's exit code and hide a real failure.
_perf_provenance_marker() {
  local rc=$?
  if [[ $rc -eq 0 ]]; then
    echo "perf_provenance: OK"
  else
    echo "perf_provenance: FAILED"
  fi
  exit "$rc"
}
trap _perf_provenance_marker EXIT

# print_provenance <binary> — the five identity fields, or fail loudly.
print_provenance() {
  local bin="$1"
  [[ -f "$bin" ]] || { echo "perf_provenance: no such binary: $bin" >&2; exit 1; }
  local mtime_epoch; mtime_epoch="$(stat -c %Y "$bin")"
  echo "path:   $bin"
  echo "size:   $(stat -c %s "$bin") bytes"
  echo "mtime:  $(date -d "@$mtime_epoch" '+%Y-%m-%dT%H:%M:%S%z')"
  echo "sha256: $(sha256sum "$bin" | cut -d' ' -f1)"
  echo "head:   $(git rev-parse HEAD 2>/dev/null || echo unknown)"
  # ponytail: staleness hint, one line — a binary older than HEAD's commit cannot
  # include HEAD's change (the stale-42s-baseline class). sha256/mtime/head is the
  # real gate; this is a nudge, not authority (git-less or shallow tree -> skipped).
  local head_ct; head_ct="$(git log -1 --format=%ct 2>/dev/null || echo 0)"
  (( mtime_epoch < head_ct )) && echo "  WARNING: binary mtime predates HEAD commit ($(date -d "@$head_ct" '+%Y-%m-%dT%H:%M:%S%z')) — may not include the change under test" >&2
  return 0
}

# --- self-test --------------------------------------------------------------
self_test() {
  echo "perf_provenance self-test:"
  local out rc

  # (a) nonexistent binary -> non-zero exit, and NOT a set of fields
  rc=0; out="$( (print_provenance "/no/such/binary-xyz") 2>&1 )" || rc=$?
  if [[ $rc -ne 0 && "$out" == *"no such binary"* ]]; then
    echo "  ok: nonexistent binary fails loudly"
  else
    echo "  FAIL: nonexistent binary not rejected (rc=$rc out='$out')" >&2; exit 2
  fi

  # (b) real binary -> exit 0 with ALL five identity fields present
  local tmp; tmp="$(mktemp)"; printf 'provenance-fixture\n' > "$tmp"
  trap 'rm -f "$tmp"' RETURN
  rc=0; out="$( (print_provenance "$tmp") 2>&1 )" || rc=$?
  local field ok=1
  for field in "path:" "size:" "mtime:" "sha256:" "head:"; do
    [[ "$out" == *"$field"* ]] || { echo "  FAIL: missing field '$field'" >&2; ok=0; }
  done
  if [[ $rc -eq 0 && $ok -eq 1 ]]; then
    echo "  ok: real binary prints all five fields"
  else
    echo "  FAIL: real binary (rc=$rc ok=$ok out='$out')" >&2; exit 2
  fi
}

# --- entrypoint -------------------------------------------------------------
case "${1:-}" in
  --self-test)
    self_test
    ;;
  "" | -h | --help)
    sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'
    trap - EXIT   # help runs no check: don't mint a false OK marker.
    exit 0
    ;;
  --*)
    echo "unknown option: $1 (try --help)" >&2; exit 64
    ;;
  *)
    print_provenance "$1"
    ;;
esac
