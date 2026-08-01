#!/usr/bin/env bash
# Append one line to the SDD progress ledger (.superpowers/sdd/progress.md).
#
# 49 of these lines were hand-written by agents across one run, drifting in
# shape. This writes the established shape:
#
#   Task 7: complete (commit abc1234, review clean)
#   Task A4: implemented (commit 44dacd909)
#   Task A2: complete (no commit; probe reverted)
#
# Usage:
#   bash scripts/sdd_record.sh <task-num> <status> [sha] [note]
#
#   sha "" or "-" omits the commit clause. Set SDD_PROGRESS to append elsewhere
#   (used by --selftest).
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
LEDGER="${SDD_PROGRESS:-$REPO_ROOT/.superpowers/sdd/progress.md}"

if [ "${1:-}" = "--selftest" ]; then
    tmp="$(mktemp -d)"
    export SDD_PROGRESS="$tmp/sdd/progress.md"
    self="${BASH_SOURCE[0]}"
    [ "$(bash "$self" 7 complete abc1234 'review clean')" = "Task 7: complete (commit abc1234, review clean)" ]
    [ "$(bash "$self" A4 implemented 44dacd909)" = "Task A4: implemented (commit 44dacd909)" ]
    [ "$(bash "$self" A2 complete - 'probe reverted')" = "Task A2: complete (probe reverted)" ]
    [ "$(bash "$self" C6 complete)" = "Task C6: complete" ]
    [ "$(wc -l < "$SDD_PROGRESS")" -eq 4 ]
    rm -rf "$tmp"
    echo "selftest=OK"
    exit 0
fi

if [ $# -lt 2 ]; then
    echo "usage: sdd_record.sh <task-num> <status> [sha] [note]" >&2
    exit 2
fi

TASK="$1"
STATUS="$2"
SHA="${3:-}"
NOTE="${4:-}"
[ "$SHA" = "-" ] && SHA=""

case "$SHA:$NOTE" in
    :)      DETAIL="" ;;
    :*)     DETAIL=" ($NOTE)" ;;
    *:)     DETAIL=" (commit $SHA)" ;;
    *)      DETAIL=" (commit $SHA, $NOTE)" ;;
esac

LINE="Task $TASK: $STATUS$DETAIL"
mkdir -p "$(dirname -- "$LEDGER")"
printf '%s\n' "$LINE" >> "$LEDGER"
printf '%s\n' "$LINE"
