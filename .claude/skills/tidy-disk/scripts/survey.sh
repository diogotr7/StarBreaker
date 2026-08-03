#!/usr/bin/env bash
# tidy-disk survey → JSON on stdout. Read-only. Optional arg: session scratchpad dir.
set -euo pipefail
exec python3 "$(dirname "$0")/survey.py" "$@"
