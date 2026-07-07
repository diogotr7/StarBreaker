# guard_assert.sh — shared non-empty-target precondition for shell guards
# (alignment plan T5, generalising retro-ledger items 3/61/105).
#
# A guard that finds ZERO targets after a rename/decompose passes VACUOUSLY —
# false assurance that is worse than no guard. Every shell guard that discovers
# its target set at runtime sources this file and calls, right after discovery:
#
#     source "$(dirname "$0")/lib/guard_assert.sh"
#     assert_nonzero_matches "$count" "what was being scanned"
#
# `assert_nonzero_matches <count> <what>` returns 0 when <count> is a positive
# integer; otherwise (0, empty, negative, or non-numeric — every "found nothing"
# shape) it prints `guard_assert: FAILED (0 matches: <what>)` to stderr and
# exits 1, aborting the sourcing guard under `set -e`.
#
# Run `bash scripts/lib/guard_assert.sh --self-test` to exercise it.

assert_nonzero_matches() {
  local count="${1:-}" what="${2:-<unspecified>}"
  if [[ "$count" =~ ^[0-9]+$ ]] && (( count > 0 )); then
    return 0
  fi
  echo "guard_assert: FAILED (0 matches: $what)" >&2
  exit 1
}

# --- RESULT MARKER (self-test only) -----------------------------------------
# Installed ONLY on direct execution, never on source, so sourcing a guard does
# not clobber that guard's own EXIT trap.
_guard_assert_marker() {
  local rc=$?
  if [[ $rc -eq 0 ]]; then
    echo "guard_assert: OK"
  else
    echo "guard_assert: FAILED (exit $rc)"
  fi
  exit "$rc"
}

_guard_assert_self_test() {
  local fails=0 out rc
  # zero -> loud FAILED with the exact contract message, exit 1
  out="$( (assert_nonzero_matches 0 "widget targets") 2>&1 )"; rc=$?
  if [[ $rc -eq 1 && "$out" == "guard_assert: FAILED (0 matches: widget targets)" ]]; then
    echo "  ok: zero count fails loudly with contract message"
  else
    echo "  FAIL: zero count (rc=$rc out='$out')" >&2; fails=1
  fi
  # positive -> pass silently, exit 0
  out="$( (assert_nonzero_matches 5 "widget targets") 2>&1 )"; rc=$?
  if [[ $rc -eq 0 && -z "$out" ]]; then
    echo "  ok: positive count passes silently"
  else
    echo "  FAIL: positive count (rc=$rc out='$out')" >&2; fails=1
  fi
  # empty / non-numeric / negative -> treated as "found nothing", exit 1
  for bogus in "" "abc" "-3"; do
    out="$( (assert_nonzero_matches "$bogus" "scan") 2>&1 )"; rc=$?
    if [[ $rc -eq 1 && "$out" == "guard_assert: FAILED (0 matches: scan)" ]]; then
      echo "  ok: non-positive count '$bogus' fails loudly"
    else
      echo "  FAIL: bogus count '$bogus' (rc=$rc out='$out')" >&2; fails=1
    fi
  done
  return "$fails"
}

# Only fire when EXECUTED directly (not when sourced): sourced, $0 is the caller.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  case "${1:-}" in
    --self-test)
      trap _guard_assert_marker EXIT
      echo "guard_assert self-test:"
      _guard_assert_self_test
      ;;
    *)
      echo "guard_assert.sh is a sourced library; run --self-test to exercise it" >&2
      exit 64
      ;;
  esac
fi
