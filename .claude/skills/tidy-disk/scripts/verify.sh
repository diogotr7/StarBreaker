#!/usr/bin/env bash
# tidy-disk post-cleanup verification. Exits non-zero naming the first failing check.
set -uo pipefail
cd "$(dirname "$0")/../../../.." || exit 2   # -> StarBreaker repo root
ws="$(cd .. && pwd)"
fail() { echo "FAIL: $*" >&2; exit 1; }

echo "== df -h ."; df -h . || fail "df"

echo "== bisect ladder"
count=$(find target/release/deps -maxdepth 1 -regextype posix-extended \
  -regex '.*/starbreaker-[0-9a-f]{16}' 2>/dev/null | wc -l)
echo "  bare starbreaker-<hash> binaries: $count"
[ "$count" -gt 0 ] || fail "bisect ladder is EMPTY (target/release/deps/starbreaker-<hash>)"

echo "== current oracle baselines (from ledger's last brace-form mention)"
line=$(grep -oE 'ships_perf_bench/\{[^}]+\}_[A-Za-z0-9_]+' docs/optimisation-ledger.md | tail -1)
[ -n "$line" ] || fail "could not parse current baselines from docs/optimisation-ledger.md"
suffix="${line##*\}_}"
comps="${line#ships_perf_bench/\{}"; comps="${comps%%\}*}"
IFS=',' read -ra arr <<<"$comps"
for c in "${arr[@]}"; do
  d="$ws/ships_perf_bench/$(echo "$c" | xargs)_${suffix}"
  [ -d "$d" ] || fail "current baseline missing: $d"
  echo "  ok  $d"
done

echo "== ships/ guards"
[ -d "$ws/ships/dcb_canvas" ] || fail "ships/dcb_canvas missing (live-IR harness guard silently disabled)"
echo "  ok  ships/dcb_canvas present"

echo "== graphify live set"
for f in graphify-out/graph.json graphify-out/manifest.json; do
  [ -e "$f" ] || fail "graphify live set missing: $f"
done
echo "  ok"

echo "ALL CHECKS PASSED"
