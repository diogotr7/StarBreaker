#!/usr/bin/env bash
# perf_oracle.sh — the byte-identity correctness oracle for an optimisation pass.
#
# RESPONSIBILITY
#   An optimisation that changes the output is not an optimisation, it is a bug.
#   This runs the oracle the skill mandates after EVERY build —
#   `diff -rq <baseline> <new> | grep -v export_stamp` must be EMPTY — and, with
#   --determinism, the second half of the rule: export the SAME entity TWICE into
#   fresh dirs and diff those against each other, because parallelism + shared
#   maps + completion order can make output vary run-to-run (rule 5:
#   non-determinism is a bug, not noise).
#
# USAGE
#   perf_oracle.sh <baseline_dir> <new_dir> [--determinism <entity>]
#     <baseline_dir>  the kept baseline export (step 0 of the loop)
#     <new_dir>       the export from the binary under test
#     --determinism E export entity E twice into <new_dir>.det1/.det2 and diff
#
#   Final line is ORACLE: BYTE-IDENTICAL or ORACLE: DIVERGED (N paths); exit 1 on
#   divergence. SC_DATA_P4K is NOT exported — the P4K is auto-detected.
set -euo pipefail

# Verdict marker via EXIT trap so an early failure still ends in a parseable
# line (ledger 89: piped exit codes lie). _verdict is set once we have a real one.
_verdict=""
_marker() {
  local rc=$?
  [[ -n "$_verdict" ]] && echo "$_verdict" || echo "ORACLE: FAILED (rc=$rc)"
  exit "$rc"
}
trap _marker EXIT

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; }

case "${1:-}" in
  "" | -h | --help) usage; trap - EXIT; exit 0 ;;
esac

base="$1"; shift
[[ $# -ge 1 && "$1" != --* ]] || { echo "perf_oracle: <new_dir> is required (try --help)" >&2; exit 64; }
new="$1"; shift

det_entity=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --determinism) det_entity="${2:?--determinism needs an entity}"; shift 2 ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 64 ;;
  esac
done

for d in "$base" "$new"; do
  [[ -d "$d" ]] || { echo "perf_oracle: no such export dir: $d" >&2; exit 1; }
done

diverged=0

# --- 1. baseline vs new -----------------------------------------------------
echo "=== oracle: $base  vs  $new ==="
# export_stamp is expected to differ every export — it is the only exemption.
mapfile -t lines < <(diff -rq "$base" "$new" 2>&1 | grep -v export_stamp || true)
if [[ ${#lines[@]} -eq 0 ]]; then
  echo "  byte-identical (excl. export_stamp)"
else
  printf '  %s\n' "${lines[@]}"
  diverged=$((diverged + ${#lines[@]}))
fi

# --- 2. determinism: same binary, two fresh exports --------------------------
if [[ -n "$det_entity" ]]; then
  bin="$repo_root/target/release/starbreaker"
  [[ -x "$bin" ]] || { echo "perf_oracle: no release binary at $bin" >&2; exit 1; }
  echo
  echo "=== determinism: $det_entity exported twice ==="
  "$script_dir/perf_provenance.sh" "$bin"
  d1="${new}.det1"; d2="${new}.det2"
  for d in "$d1" "$d2"; do
    rm -rf "$d"
    env RUST_LOG=info "$bin" entity export "$det_entity" "$d" \
      --kind decomposed --lod 0 --mip 0 --materials all > "$d.log" 2>&1 \
      || { echo "perf_oracle: determinism export failed — see $d.log" >&2; tail -20 "$d.log" >&2; exit 1; }
  done
  mapfile -t dlines < <(diff -rq "$d1" "$d2" 2>&1 | grep -v export_stamp || true)
  if [[ ${#dlines[@]} -eq 0 ]]; then
    echo "  deterministic (two runs byte-identical)"
  else
    printf '  %s\n' "${dlines[@]}"
    diverged=$((diverged + ${#dlines[@]}))
  fi
fi

echo
if [[ $diverged -eq 0 ]]; then
  _verdict="ORACLE: BYTE-IDENTICAL"
else
  _verdict="ORACLE: DIVERGED ($diverged paths)"
  exit 1
fi
