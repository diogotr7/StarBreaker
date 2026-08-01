#!/usr/bin/env bash
# One inner-loop cycle of the UI parity TDD workflow (the four commands every
# session retypes ~12x): build -> lib tests -> ui_check battery -> render.
#
# Usage:
#   bash scripts/ui_iterate.sh [--full] [--screen <screen_id>]
#                              [--render-args "<args for ui_render_canvas.sh>"]
#                              [--skip-render]
#
#   --full          run scripts/ui_check.sh --full (workstream-boundary tier)
#   --screen        final stage is scripts/ui_arc_status.sh --screen <id>
#                   (render + region diff) instead of a plain canvas render
#   --render-args   final stage is scripts/ui_render_canvas.sh <args>
#                   (e.g. --render-args "--canvas <guid> --entity <Class>")
#   --skip-render   stop after ui_check.sh
#
# Result MARKERS, never exit codes (ledger 89): each stage's log is grepped for
# its own success marker, so a piped/filtered run can never report a false
# green. The first stage whose marker is absent stops the cycle: the failing
# stage is named on the first line of the failure report, with the last 20 log
# lines. Logs: /tmp/ui_iterate/<stage>.log
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

FULL=0
SCREEN=""
RENDER_ARGS=""
SKIP_RENDER=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --full) FULL=1; shift ;;
        --screen) SCREEN="$2"; shift 2 ;;
        --render-args) RENDER_ARGS="$2"; shift 2 ;;
        --skip-render) SKIP_RENDER=1; shift ;;
        -h|--help) sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

LOG_DIR="/tmp/ui_iterate"
mkdir -p "$LOG_DIR"

# stage <name> <success-marker-regex> <command...>
stage() {
    local name="$1" marker="$2"
    shift 2
    local log="$LOG_DIR/$name.log"
    echo
    echo "==> $name: $*"
    # `|| true`: the exit code is deliberately ignored — the marker below is the
    # authoritative result. Piping through tee would hand us tee's status anyway.
    { "$@" 2>&1 | tee "$log"; } || true
    if ! grep -qE "$marker" "$log"; then
        echo
        echo "FAIL stage=$name"
        echo "  marker not found: /$marker/  (log: $log)"
        echo "  --- last 20 lines ---"
        tail -20 "$log"
        exit 1
    fi
}

# cargo prints "Finished `dev` profile ..." only on a successful build/test
# compile; "test result: ok." is cargo test's own summary line.
stage build 'Finished' cargo build -p starbreaker-ui
stage test '^test result: ok\.' cargo test -p starbreaker-ui --lib

CHECK_ARGS=()
[[ "$FULL" -eq 1 ]] && CHECK_ARGS=(--full)
stage check 'ui_check: ALL GREEN' bash scripts/ui_check.sh "${CHECK_ARGS[@]}"

if [[ "$SKIP_RENDER" -eq 1 ]]; then
    echo
    echo "==> render SKIPPED (--skip-render)"
elif [[ -n "$SCREEN" ]]; then
    stage arc_status 'ui_arc_status: OK' bash scripts/ui_arc_status.sh --screen "$SCREEN"
elif [[ -n "$RENDER_ARGS" ]]; then
    read -ra RA <<<"$RENDER_ARGS"
    stage render '^png md5:' bash scripts/ui_render_canvas.sh "${RA[@]}"
else
    echo
    echo "==> render SKIPPED (no --screen / --render-args given)"
fi

echo
echo "UI_ITERATE: ALL GREEN"
